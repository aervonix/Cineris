//! The project's knowledge in a session
//! (`docs/architecture/14-adaptation.md`, "Project knowledge: profile and
//! memory").
//!
//! `cortex-knowledge` keeps the two files and knows nothing of sessions,
//! tools or the permission engine. This module is where a session meets
//! it, and fills four seams from it:
//!
//! | Seam | Whose | Filled with |
//! |---|---|---|
//! | [`cortex_agent::ProfileSource`] | The loop's: section 5 of the system prompt | The profile as it is now, rendered, with the files read again when they changed. |
//! | [`cortex_tools::Proposals`] | `remember`'s | A proposal stored as proposed in the project's own store. |
//! | [`cortex_tools::CommandProfile`] | `shell`'s | Whether a command is one of the project's builds, test runs or checks. |
//! | The engine's learned rules | `cortex-policy`'s | One exact allow rule for each command the user accepted. |
//!
//! And it is where the command observer is fed: each finished `shell` call
//! is handed to [`Knowledge::observed`].
//!
//! # Nothing is read until the first turn
//!
//! Making a session's [`Knowledge`] touches no file. The first read is
//! when the loop asks for section 5, at the start of the session's first
//! turn, on the blocking pool; the engine is given the learned rules
//! there, before the turn's first request.
//!
//! # A turn's start and a command's result wait for no lock
//!
//! A read waits for nothing: it takes no lock for the store, and tries
//! the lock without waiting when it has the good copy to bring up to
//! date. What writes the store for the session itself (what the observer
//! saw, the mark on an entry that expired) is done beside the turn, on
//! the blocking pool ([`Knowledge::beside`]): a process that holds the
//! lock and has stopped cannot hold up a turn's start or a command's
//! result. `project/list` waits for that work, so a listing after a turn
//! holds what the turn's commands showed. A `remember` call writes the
//! store itself, and waits for the lock as any change does.
//!
//! # A session's parts do not hold each other
//!
//! The permission policy and the round's checkpoints call back into the
//! session's knowledge, which holds both: the callbacks hold it weakly, or
//! a closed session would never be let go (found by the independent
//! review).
//!
//! # One place holds a session's learned rules
//!
//! [`Knowledge`] keeps the rules it last gave the engine, with the
//! snapshot they were made from. A change of level makes a new engine,
//! which is given them from here as it is put in force
//! ([`Knowledge::put_in_force`], under this lock and then the policy's); nothing
//! else holds them, so a command the user turned off cannot come back
//! with a new engine. They are changed, and handed to an engine, only
//! under their own lock: two changes at once leave the engine with the
//! later one's rules, and this module knowing which those are. And they
//! are made only from a reading of the files that is still the files'
//! while that lock is held: a reading that another change has overtaken
//! is read again there, so it cannot put back what the change took away.
//!
//! Before each command is decided the session asks whether the two files
//! are still as **its own rules' snapshot** read them, and whether the
//! first of the entries behind the rules has expired. Not the snapshot
//! the project's sessions share: another session of the project may have
//! read the files again since, and this session's engine was not given
//! that reading's rules. When either is so, the files are read again
//! there and then, and the engine given the rules that follow: a command
//! turned off asks, and the others go on running unasked, though another
//! session or process wrote the store in the middle of this one's turn.
//!
//! # Only this project's store
//!
//! A session's `Knowledge` is made for the session's own workspace, and
//! `remember`, the observer and the prompt reach no other. `project/*`
//! names a project by a session the client has open, or by a workspace
//! root, which only a `user` client may do.

#![allow(
    clippy::disallowed_methods,
    reason = "before a command's decision the two files' stamps are looked at (two stat \
              calls), and the files read again when one changed (a bounded read that waits \
              for nothing); everything else here that blocks runs on the blocking pool"
)]

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use cortex_base::{SecretFilter, visible};
use cortex_knowledge::{
    Check, Entry, Kind, Learnt, Outcome, Project, Proposed, Purpose, Run, Snapshot, StoreError,
    UpdateError, Watch, classify, error_code, normal_line,
};
use cortex_policy::{ShellSyntax, exact_command_rule};
use cortex_protocol::ErrorKind;
use cortex_protocol::event::{Event, ProjectUpdated};
use cortex_protocol::ids::{EntryId, ProjectKey, SessionId};
use cortex_protocol::lease::Rule;
use cortex_protocol::methods::{
    ClientRole, ProjectEntriesResult, ProjectListParams, ProjectUpdateParams,
};
use cortex_protocol::project::{EntryAction, EntryKind};
use cortex_tools::shell::CommandProfile;
use cortex_tools::{Proposal, ProposalError, ProposalOutcome, Proposals};

use crate::clients::ClientId;
use crate::core::{Core, OpError, error};
use crate::policy::EnginePolicy;
use crate::store::Hooks;

/// What every session of one project in this process shares: the two
/// files as they were last read.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    snapshot: Mutex<Option<Arc<Snapshot>>>,
}

/// The learned rules a session's engine has, and what they were made from.
#[derive(Default)]
struct Given {
    /// The rules the engine was last given: the one place they are held.
    rules: Vec<Rule>,
    /// The snapshot they were made from. The look before a command asks
    /// whether the files are still as this one read them.
    from: Option<Arc<Snapshot>>,
    /// When the first of the entries behind the rules expires.
    until: Option<cortex_base::UtcTime>,
}

/// Hands the rules of `given` to `set`, **while holding them**: a change
/// of the rules that comes at the same moment waits, and then gives the
/// same engine its own. Handed over outside the lock, the older list could
/// arrive last, and a command just turned off would be back on an engine
/// that this module believes has no rule. A list that is refused leaves no
/// rule held.
fn regive<E>(given: &Mutex<Given>, set: impl FnOnce(Vec<Rule>) -> Result<(), E>) -> Result<(), E> {
    let mut given = lock(given);
    let rules = given.rules.clone();
    let outcome = set(rules);
    if outcome.is_err() {
        given.rules.clear();
    }
    outcome
}

/// Whether a text of `check` holds what `finds` takes for a credential.
fn check_holds(check: &Check, finds: &dyn Fn(&str) -> bool) -> bool {
    match check {
        Check::Exists { path } => finds(path),
        Check::Contains { path, pattern } => finds(path) || finds(pattern),
        Check::Command { command } => finds(command),
    }
}

/// Whether a text of `entry` that is kept in a file holds what `finds`
/// takes for a credential: its text, its evidence, its command line and
/// directory, and its check.
fn entry_holds(entry: &Entry, finds: &dyn Fn(&str) -> bool) -> bool {
    finds(&entry.text)
        || entry.evidence.as_deref().is_some_and(finds)
        || entry
            .command
            .as_ref()
            .is_some_and(|command| finds(&command.run) || command.cwd.as_deref().is_some_and(finds))
        || entry
            .check
            .as_ref()
            .is_some_and(|check| check_holds(check, finds))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a session's knowledge is made from.
pub(crate) struct Parts {
    pub(crate) project: Project,
    pub(crate) key: ProjectKey,
    pub(crate) shared: Arc<Shared>,
    pub(crate) session: SessionId,
    pub(crate) policy: Arc<EnginePolicy>,
    pub(crate) hooks: Arc<Hooks>,
    pub(crate) syntax: ShellSyntax,
    pub(crate) filter: Arc<RwLock<SecretFilter>>,
    /// `project.profile_tokens`.
    pub(crate) tokens: u32,
    /// `project.learn`.
    pub(crate) learn: bool,
    /// The session's checkpoints, which know what the harness's own tools
    /// changed. `None` for a session kept in memory.
    pub(crate) rounds: Option<Arc<crate::checkpoints::Rounds>>,
    /// The repository's shared file, as a path of the workspace.
    pub(crate) shared_file: Option<cortex_base::AbsPath>,
    /// A test's probe: called each time the project's files are looked at.
    pub(crate) on_read: Option<crate::repo_map::Probe>,
}

/// One session's view of its project's knowledge.
pub(crate) struct Knowledge {
    project: Project,
    pub(crate) key: ProjectKey,
    shared: Arc<Shared>,
    session: SessionId,
    policy: Arc<EnginePolicy>,
    hooks: Arc<Hooks>,
    syntax: ShellSyntax,
    filter: Arc<RwLock<SecretFilter>>,
    /// `project.profile_tokens`: a reload changes it as the session's next
    /// epoch begins (step 24).
    tokens: std::sync::atomic::AtomicU32,
    learn: bool,
    rounds: Option<Arc<crate::checkpoints::Rounds>>,
    shared_file: Option<cortex_base::AbsPath>,
    on_read: Option<crate::repo_map::Probe>,
    /// What this session's commands have shown so far.
    watch: Mutex<Watch>,
    /// The learned rules the engine was last given, and what they were
    /// made from: the one place they are held.
    learned: Mutex<Given>,
    /// What the user was told already, so that each thing is said once.
    told: Mutex<BTreeSet<String>>,
    /// This knowledge, for the work it starts beside a turn.
    me: std::sync::Weak<Knowledge>,
    /// The work started beside a turn that may still be running.
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// Whether the mark on expired entries is being written.
    marking: std::sync::atomic::AtomicBool,
}

/// The most things a session says once: past it, nothing new is said.
const MAX_TOLD: usize = 256;

/// How often the files are read again for one hand-over of rules, when
/// they change under it. Past it, no learned rule is in force until the
/// next hand-over.
const MAX_RELOADS: usize = 3;

/// The engine has the learned rules that follow from a reading of the
/// files (a point a test can stop at: [`cortex_base::hook`]).
pub const AFTER_THE_RULES: &str = "knowledge: the engine has its learned rules";

/// Lets the next mark of expired entries be made, when it is dropped.
struct Marked(std::sync::Weak<Knowledge>);

impl Drop for Marked {
    fn drop(&mut self) {
        if let Some(knowledge) = self.0.upgrade() {
            knowledge
                .marking
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

impl std::fmt::Debug for Knowledge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Knowledge")
            .field("session", &self.session)
            .field("project", &self.key)
            .finish_non_exhaustive()
    }
}

/// Whether `learnt` is of an entry of the private store that is in use:
/// one a check's run may confirm.
fn confirms(snapshot: &Snapshot, learnt: &Learnt, now: cortex_base::UtcTime) -> bool {
    snapshot.entries().iter().any(|entry| {
        entry.scope == cortex_knowledge::Scope::Private
            && entry.kind == learnt.kind
            && entry.in_use(now)
            && entry
                .command
                .as_ref()
                .is_some_and(|command| command.run == learnt.line && command.cwd == learnt.cwd)
    })
}

/// An entry's command as a person is shown it.
fn shown(entry: &Entry) -> String {
    match &entry.command {
        Some(command) => format!("`{}`", visible(&command.run)),
        None => format!("\"{}\"", visible(&entry.text)),
    }
}

impl Knowledge {
    pub(crate) fn new(parts: Parts) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            tasks: Mutex::new(Vec::new()),
            marking: std::sync::atomic::AtomicBool::new(false),
            project: parts.project,
            key: parts.key,
            shared: parts.shared,
            session: parts.session,
            policy: parts.policy,
            hooks: parts.hooks,
            syntax: parts.syntax,
            filter: parts.filter,
            tokens: std::sync::atomic::AtomicU32::new(parts.tokens),
            learn: parts.learn,
            rounds: parts.rounds,
            shared_file: parts.shared_file,
            on_read: parts.on_read,
            watch: Mutex::new(Watch::new()),
            learned: Mutex::new(Given::default()),
            told: Mutex::new(BTreeSet::new()),
        })
    }

    /// Tells the user something, once in this session.
    fn tell_once(&self, message: String) {
        let mut told = lock(&self.told);
        if told.len() < MAX_TOLD && told.insert(message.clone()) {
            drop(told);
            self.hooks.tell(message);
        }
    }

    /// Does `work` beside the turn, on the blocking pool, and returns at
    /// once: work that writes the store, and so waits for its lock. With
    /// no runtime to do it on, it is done here.
    fn beside(&self, work: impl FnOnce(&Self) + Send + 'static) {
        let Some(this) = self.me.upgrade() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            work(&this);
            return;
        };
        let task = runtime.spawn_blocking(move || work(&this));
        let mut tasks = lock(&self.tasks);
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }

    /// Waits for the work started beside a turn: what the session's
    /// commands showed is in the store once this returns.
    pub(crate) async fn settled(&self) {
        loop {
            let tasks = std::mem::take(&mut *lock(&self.tasks));
            if tasks.is_empty() {
                return;
            }
            for task in tasks {
                let _ = task.await;
            }
        }
    }

    fn finds_secret(&self, text: &str) -> bool {
        self.filter
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finds(text)
    }

    /// The snapshot every session of the project last read, if any has.
    fn peek(&self) -> Option<Arc<Snapshot>> {
        lock(&self.shared.snapshot).clone()
    }

    /// The project's knowledge as it is now: read again when either file
    /// changed since the last read. Reads, and takes no lock.
    ///
    /// An entry past its expiry is out of use by the clock already,
    /// wherever a snapshot is asked what is in use. The mark on it is a
    /// write, which waits for the store's lock: it is made beside the
    /// turn, or here when `wait` says the caller is not on a turn's path.
    fn refresh(&self, wait: bool) -> Arc<Snapshot> {
        if let Some(probe) = &self.on_read {
            probe();
        }
        let held = self.peek();
        let snapshot = match held {
            Some(held) if self.project.is_current(&held) => held,
            held => Arc::new(self.project.load(held.as_deref())),
        };
        let snapshot = self.took(&snapshot);
        if snapshot.has_lapsed(self.project.now()) {
            if wait {
                self.mark_lapsed();
                return self.peek().unwrap_or(snapshot);
            }
            if !self.marking.swap(true, std::sync::atomic::Ordering::SeqCst) {
                // However the mark ends, and if it never starts, the next
                // one may be made.
                let done = Marked(self.me.clone());
                self.beside(move |this| {
                    let _done = done;
                    this.mark_lapsed();
                });
            }
        }
        snapshot
    }

    /// Marks every entry past its expiry, and tells the user of each.
    /// Blocks, and waits for the store's lock.
    fn mark_lapsed(&self) {
        let held = self.peek();
        match self.project.lapse(held.as_deref()) {
            Ok(lapsed) => {
                for entry in &lapsed.changed {
                    self.hooks.tell(format!(
                        "The project profile's entry {} ({}) expired, and is no longer used. \
                         `cortex project accept {}` brings it back.",
                        entry.id,
                        shown(entry),
                        entry.id
                    ));
                }
                self.updated(&lapsed.changed, &[]);
                self.took(&Arc::new(lapsed.snapshot));
            }
            // The entry is out of use by the clock already; the mark
            // waits for a store that can be written.
            Err(error) => self.tell_once(format!(
                "An expired entry of the project profile could not be marked: {}",
                visible(&error.to_string())
            )),
        }
    }

    /// Takes `snapshot` as the project's knowledge now: every session of
    /// the project in this process sees it, and this session's engine is
    /// given the rules that follow from it. Answers with the snapshot the
    /// rules were made from, which is a later reading of the files when
    /// `snapshot` was no longer theirs ([`Knowledge::give_rules`]).
    pub(crate) fn took(&self, snapshot: &Arc<Snapshot>) -> Arc<Snapshot> {
        let snapshot = self.give_rules(snapshot);
        for problem in snapshot.problems() {
            self.tell_once(problem.clone());
        }
        snapshot
    }

    /// Gives the engine one exact allow rule for each command the user
    /// accepted, in place of the ones it had.
    ///
    /// **The rules are made from the files as they are while the rules
    /// are held.** A snapshot reaches here from several threads (a turn's
    /// start, what the observer stored beside a turn, a change the user
    /// made), and one that was read before another's change can arrive
    /// after it: handed to the engine as it is, it would put back the rule
    /// of a command the user has just turned off, between a command's look
    /// at the files and its decision (found by the full review of the
    /// second pull request). So a snapshot that is not the files' any more
    /// is not used: the files are read again, here, under the rules' own
    /// lock, and the rules made from that reading. Files that go on
    /// changing under it leave no learned rule in force.
    fn give_rules(&self, snapshot: &Arc<Snapshot>) -> Arc<Snapshot> {
        let mut given = lock(&self.learned);
        let mut snapshot = Arc::clone(snapshot);
        let mut current = self.project.is_current(&snapshot);
        let mut reloads = 0;
        while !current && reloads < MAX_RELOADS {
            snapshot = Arc::new(self.project.load(Some(&snapshot)));
            current = self.project.is_current(&snapshot);
            reloads += 1;
        }
        let now = self.project.now();
        let mut rules = Vec::new();
        let mut until = None;
        if current {
            for entry in snapshot.runs_unasked(now) {
                let Some(command) = &entry.command else {
                    continue;
                };
                let Some(rule) = exact_command_rule(&command.run, self.syntax) else {
                    continue;
                };
                rules.push(rule);
                until = [until, entry.expires].into_iter().flatten().min();
            }
        }
        // The project's sessions share the reading the rules stand on.
        *lock(&self.shared.snapshot) = Some(Arc::clone(&snapshot));
        given.from = Some(Arc::clone(&snapshot));
        given.until = until;
        if given.rules != rules {
            match self.policy.set_learned(rules.clone()) {
                Ok(()) => given.rules = rules,
                Err(errors) => {
                    // No learned rule is in force then.
                    given.rules.clear();
                    self.tell_once(format!(
                        "The project's accepted commands could not be given to the permission \
                         engine, so each asks before it runs: {}",
                        visible(
                            &errors
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("; ")
                        )
                    ));
                }
            }
        }
        drop(given);
        // After the hand-over, and outside this module's lock and the
        // policy's.
        cortex_base::hook::point(AFTER_THE_RULES);
        let waiting = self
            .policy
            .engine()
            .withheld()
            .iter()
            .filter(|rule| rule.source == cortex_policy::RuleSource::Learned)
            .count();
        if waiting > 0 {
            self.tell_once(format!(
                "{waiting} accepted command(s) of the project profile wait for the project's \
                 trust before they run without asking: `cortex permissions trust`."
            ));
        }
        snapshot
    }

    /// Puts `engine` in force for a change of level, with the learned
    /// rules: read under this module's lock, and handed to the engine with
    /// the swap under the policy's (the re-decide step of step 12). The
    /// order is this lock, then the policy's, everywhere. So the new engine
    /// never runs without them, and a command turned off while the level
    /// changed is off in it.
    pub(crate) fn put_in_force(&self, engine: cortex_policy::Engine) {
        let given = regive(&self.learned, |rules| {
            self.policy.replace_with(engine, rules)
        });
        if let Err(errors) = given {
            tracing::warn!(
                ?errors,
                "the learned rules could not be given to the new engine"
            );
        }
    }

    /// Before a command is decided: when either of the project's files
    /// changed since this session's rules were made from them, or the
    /// first of the entries behind the rules has expired, the rules are
    /// made again there and then, from the files as they are. Another
    /// process's `cortex project disable` then takes a command off the
    /// safe list before its next decision, not at the next turn, and the
    /// commands still accepted go on running unasked.
    ///
    /// The files are compared with the snapshot **these rules were made
    /// from**, not with the one the project's sessions share: another
    /// session may have read the files again since, and only its own
    /// engine was given what it read (found by the independent read of the
    /// trust path).
    pub(crate) fn before_command(&self) {
        let (from, expired) = {
            let given = lock(&self.learned);
            // With no learned rule in force there is nothing to take
            // back; what was accepted elsewhere since comes at the next
            // turn's start.
            if given.rules.is_empty() {
                return;
            }
            let expired = given.until.is_some_and(|until| until <= self.project.now());
            (given.from.clone(), expired)
        };
        if let Some(probe) = &self.on_read {
            probe();
        }
        let current = from
            .as_ref()
            .is_some_and(|from| self.project.is_current(from));
        if current && !expired {
            return;
        }
        // The rules are made again before this command is decided: from
        // the files as they are now when they changed (a read that waits
        // for nothing), or from the same snapshot at the time it is now
        // when an entry expired. A command turned off asks; the others
        // still run unasked.
        let snapshot = match from {
            Some(from) if current => from,
            from => Arc::new(self.project.load(from.as_deref())),
        };
        self.took(&snapshot);
    }

    /// Tells front ends which entries changed.
    fn updated(&self, changed: &[Entry], removed: &[EntryId]) {
        if changed.is_empty() && removed.is_empty() {
            return;
        }
        self.hooks.emit(Event::ProjectUpdated(ProjectUpdated {
            entries: changed.iter().map(Entry::to_wire).collect(),
            removed: removed.to_vec(),
        }));
    }

    /// A `promote` wrote the repository's shared file, which is a
    /// protected path the sandbox looks at around a confined command: the
    /// write is the harness's own, and must not stop a running turn as a
    /// change made from outside.
    fn touched_shared_file(&self) {
        if let (Some(rounds), Some(file)) = (&self.rounds, &self.shared_file) {
            rounds.touch(file);
        }
    }

    /// A file tool wrote a file: a test that failed before may pass now
    /// for that.
    pub(crate) fn file_changed(&self) {
        lock(&self.watch).changed();
    }

    /// A `shell` call finished. What it showed is counted off the turn's
    /// path, on the blocking pool; this returns at once.
    pub(crate) fn observed(&self, run: &Run) {
        self.observe(run, false);
    }

    /// One of the project's checks finished (step 19): the harness's run,
    /// not the model's or the user's. It counts towards a flip, and it
    /// confirms an entry this store already has in use, so that an
    /// accepted command does not lapse while it runs every turn. It never
    /// makes an entry: the observer learns from the commands a session
    /// chose to run.
    pub(crate) fn observed_check(&self, run: &Run) {
        self.observe(run, true);
    }

    /// The project's knowledge as it is now, and the time by its clock:
    /// read again when either file changed since the last read. Blocks,
    /// and takes no lock (step 19's checks, which look again just before
    /// each one runs).
    pub(crate) fn current(&self) -> (Arc<Snapshot>, cortex_base::UtcTime) {
        (self.refresh(false), self.project.now())
    }

    fn observe(&self, run: &Run, harness: bool) {
        // The watch sees every run, for what changed between two of them.
        let seen = lock(&self.watch).see(run, Some(&self.session));
        if !self.learn
            || seen.is_empty()
            || self.finds_secret(&run.command)
            || run.cwd.as_deref().is_some_and(|cwd| self.finds_secret(cwd))
        {
            return;
        }
        let now = self.project.now();
        let held = self.peek();
        let wanted: Vec<Learnt> = seen
            .into_iter()
            .filter(|learnt| {
                // A line the engine could not write as an exact rule is
                // not learned: it could never run unasked, and is not one
                // command.
                exact_command_rule(&learnt.line, self.syntax).is_some()
                    && held.as_ref().is_none_or(|held| held.wants(learnt, now))
                    // A flip is told from what the harness saw change, and
                    // a session kept in memory does not see what its file
                    // tools write.
                    && (learnt.kind != Kind::Flaky || self.rounds.is_some())
                    // The harness's own run only confirms what is in use.
                    && (!harness || held.as_ref().is_some_and(|held| confirms(held, learnt, now)))
            })
            .collect();
        if wanted.is_empty() {
            return;
        }
        // Not waited for: the command's result goes back to the model
        // now, and the store is written beside it, one observation after
        // the other.
        self.beside(move |this| {
            for learnt in &wanted {
                this.learn(learnt);
            }
        });
    }

    /// Stores one thing the observer saw. Blocks.
    fn learn(&self, learnt: &Learnt) {
        let held = self.peek();
        match self.project.learn(learnt, held.as_deref()) {
            Ok((outcome, updated)) => {
                self.took(&Arc::new(updated.snapshot));
                self.updated(&updated.changed, &updated.removed);
                let Some(entry) = updated.changed.first() else {
                    return;
                };
                match (outcome, entry.kind) {
                    (Outcome::Activated, Kind::Flaky) => self.hooks.tell(format!(
                        "Seen twice in this project: the test command {} failed and then passed \
                         with nothing changed. It is in the project profile as a flaky test now. \
                         `cortex project disable {}` takes it out.",
                        shown(entry),
                        entry.id
                    )),
                    (Outcome::Activated, _) => self.hooks.tell(format!(
                        "Learned: {} is this project's {} command (it ran and succeeded twice). \
                         It is in the project profile now, and still asks before it runs. \
                         `cortex project accept {}` lets it run without asking; \
                         `cortex project disable {}` takes it out.",
                        shown(entry),
                        entry
                            .command
                            .as_ref()
                            .map_or(Purpose::Run, |command| command.purpose)
                            .as_str(),
                        entry.id,
                        entry.id
                    )),
                    (Outcome::Revived, _) => self.hooks.tell(format!(
                        "The project profile's entry {} ({}) had expired and was seen to hold \
                         again: it is back in use, and asks before it runs until it is accepted \
                         again.",
                        entry.id,
                        shown(entry)
                    )),
                    _ => {}
                }
            }
            Err(error) => self.tell_once(format!(
                "What this session's commands showed could not be kept in the project profile: \
                 {}",
                visible(&error.to_string())
            )),
        }
    }

    /// Every entry, for `project/list`. Blocks, for the store's lock too
    /// when an entry has expired and is to be marked.
    fn list(&self) -> ProjectEntriesResult {
        let snapshot = self.refresh(true);
        ProjectEntriesResult {
            entries: snapshot.listing(),
            problems: snapshot.problems().to_vec(),
        }
    }
}

impl Knowledge {
    /// The profile's budget in tokens now.
    pub(crate) fn tokens(&self) -> u32 {
        self.tokens.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The profile's budget from now on: a reload's `project.profile_tokens`
    /// as the session's next epoch begins, which builds its prompt again.
    pub(crate) fn set_tokens(&self, tokens: u32) {
        self.tokens
            .store(tokens, std::sync::atomic::Ordering::Relaxed);
    }
}

impl cortex_agent::ProfileSource for Knowledge {
    fn section(&self) -> String {
        let snapshot = self.refresh(false);
        let section = cortex_knowledge::render::render_with(
            &snapshot,
            self.project.now(),
            self.tokens(),
            &|line| !self.finds_secret(line),
        );
        if section.left_out > 0 {
            self.tell_once(format!(
                "The project profile is larger than its budget of {} tokens: {} of its entries \
                 are left out of the prompt. `cortex project list` shows them all.",
                self.tokens(),
                section.left_out
            ));
        }
        if section.withheld > 0 {
            self.tell_once(format!(
                "{} entries of the project profile hold what looks like a credential, and are \
                 left out of the prompt.",
                section.withheld
            ));
        }
        section.text
    }
}

impl Proposals for Knowledge {
    fn propose(&self, proposal: Proposal) -> Result<ProposalOutcome, ProposalError> {
        let kind = Kind::from_wire(&proposal.kind)
            .filter(|kind| matches!(kind, Kind::Fact | Kind::Convention))
            .ok_or_else(|| ProposalError::new("A proposal is a fact or a convention."))?;
        let check = proposal
            .check
            .as_ref()
            .map(Check::from_wire)
            .transpose()
            .map_err(ProposalError::new)?;
        // The check too: it is kept in the store, and goes into the
        // repository's file if the entry is promoted.
        if self.finds_secret(&proposal.text)
            || self.finds_secret(&proposal.evidence)
            || check
                .as_ref()
                .is_some_and(|check| check_holds(check, &|text| self.finds_secret(text)))
        {
            return Err(ProposalError::new(
                "The entry holds what looks like a credential, so it was not stored. Say it \
                 without the credential.",
            ));
        }
        let held = self.peek();
        let stored = self.project.propose(
            Proposed {
                kind,
                text: proposal.text,
                evidence: proposal.evidence,
                check,
                session: Some(self.session.clone()),
            },
            held.as_deref(),
        );
        match stored {
            Ok((outcome, updated)) => {
                self.took(&Arc::new(updated.snapshot));
                self.updated(&updated.changed, &updated.removed);
                Ok(match outcome {
                    cortex_knowledge::ProposalOutcome::Stored => ProposalOutcome::Stored,
                    cortex_knowledge::ProposalOutcome::AlreadyKnown => {
                        ProposalOutcome::AlreadyKnown
                    }
                })
            }
            Err(error) => Err(ProposalError::new(match &error {
                StoreError::Invalid(_) => format!(
                    "The project's memory cannot be written until its file is put right, so \
                     the proposal was not stored: {error}"
                ),
                _ => format!("The proposal was not stored: {error}"),
            })),
        }
    }
}

impl CommandProfile for Knowledge {
    fn is_build_or_test(&self, command: &str) -> bool {
        let Some(line) = normal_line(command) else {
            return false;
        };
        if classify(&line).is_some() {
            return true;
        }
        let now = self.project.now();
        self.peek().is_some_and(|snapshot| {
            snapshot.in_use(now).any(|entry| {
                entry.kind == Kind::Command
                    && entry.command.as_ref().is_some_and(|command| {
                        command.run == line
                            && matches!(
                                command.purpose,
                                Purpose::Build | Purpose::Test | Purpose::Check
                            )
                    })
            })
        })
    }
}

// ------------------------------------------------------------ the protocol

/// The error a refused change is answered with.
fn refused(failure: &UpdateError) -> OpError {
    let kind = match failure {
        StoreError::Refused(_) | StoreError::Full(_) | StoreError::Invalid(_) => ErrorKind::Usage,
        StoreError::Io { .. } | StoreError::Write(_) | StoreError::Unreadable(_) => {
            ErrorKind::Internal
        }
    };
    error(kind, error_code(failure), failure.to_string())
}

/// Which project an operation is about.
enum Target {
    /// A session's, with its knowledge.
    Session(Arc<Knowledge>),
    /// A workspace's, named by a `user` client.
    Workspace(Project, ProjectKey),
}

fn is_user(core: &Core, client: ClientId) -> bool {
    core.inner.clients.info(client).map(|info| info.role) == Some(ClientRole::User)
}

/// The project a `project/*` operation names: by a session the client has
/// open, or by a workspace root, which only a `user` client may do. Never
/// both, never neither.
fn target(
    core: &Core,
    client: ClientId,
    session: Option<&SessionId>,
    workspace: Option<&str>,
) -> Result<Target, OpError> {
    match (session, workspace) {
        (Some(session), None) => {
            let live = core.open_for(client, session)?;
            live.knowledge
                .clone()
                .map(Target::Session)
                .ok_or_else(no_state)
        }
        (None, Some(workspace)) => {
            if !is_user(core, client) {
                return Err(error(
                    ErrorKind::Usage,
                    cortex_protocol::error::code::ROLE_NOT_ALLOWED,
                    "Only a `user` client may name a project by its workspace. Name it by a \
                     session that is open on this connection.",
                ));
            }
            let (project, key) = at(core, Path::new(workspace))?;
            Ok(Target::Workspace(project, key))
        }
        _ => Err(error(
            ErrorKind::Usage,
            "invalid_params",
            "Name the project by exactly one of `session_id` and `workspace`.",
        )),
    }
}

fn no_state() -> OpError {
    error(
        ErrorKind::Usage,
        "not_available",
        "There is no state directory, so no project profile is kept.",
    )
}

/// The knowledge of the project whose workspace root is `workspace`, as a
/// session there would have it.
pub(crate) fn at(core: &Core, workspace: &Path) -> Result<(Project, ProjectKey), OpError> {
    let root = crate::assemble::workspace_root(workspace).ok_or_else(|| {
        error(
            ErrorKind::Usage,
            "invalid_workspace",
            format!("`{}` cannot be a workspace.", workspace.display()),
        )
    })?;
    let key = root.project_key();
    let file = core
        .state_dir()
        .and_then(|state| state.knowledge_file(&key))
        .ok_or_else(no_state)?;
    let clock = Arc::clone(&core.inner.options.clock);
    Ok((Project::new(file, root.to_fs_path(), clock), key))
}

/// `project/list`.
pub(crate) async fn list(
    core: &Core,
    client: ClientId,
    params: &ProjectListParams,
) -> Result<ProjectEntriesResult, OpError> {
    let target = target(
        core,
        client,
        params.session_id.as_ref(),
        params.workspace.as_deref(),
    )?;
    // What the session's commands showed is in the store first.
    if let Target::Session(knowledge) = &target {
        knowledge.settled().await;
    }
    Core::blocking(move || {
        Ok(match target {
            Target::Session(knowledge) => knowledge.list(),
            Target::Workspace(project, _) => {
                let snapshot = project.load(None);
                ProjectEntriesResult {
                    entries: snapshot.listing(),
                    problems: snapshot.problems().to_vec(),
                }
            }
        })
    })
    .await
}

/// Makes a user's changes to a project's entries: the operation's, and
/// the command line's. What the user writes must not put a credential
/// into every later prompt, or into the repository, so each text a file
/// would keep is looked at first; then the change is all or nothing.
/// Blocks.
pub(crate) fn apply(
    project: &Project,
    filter: &RwLock<SecretFilter>,
    held: Option<&Snapshot>,
    changes: &[cortex_protocol::project::EntryChange],
    add: &[cortex_protocol::project::NewEntry],
) -> Result<cortex_knowledge::Updated, OpError> {
    let finds = |text: &str| {
        filter
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finds(text)
    };
    let credential = |what: &str| {
        error(
            ErrorKind::Usage,
            "invalid_entry",
            format!(
                "{what} holds what looks like a credential, so nothing was changed. Say it \
                 without the credential."
            ),
        )
    };
    for new in add {
        if !matches!(new.kind, EntryKind::Unknown(_))
            && (finds(&new.text)
                || new.command.as_ref().is_some_and(|command| {
                    finds(&command.command) || command.cwd.as_deref().is_some_and(finds)
                })
                || new.check.as_ref().is_some_and(|check| {
                    [&check.path, &check.pattern, &check.command]
                        .into_iter()
                        .flatten()
                        .any(|text| finds(text))
                }))
        {
            return Err(credential("An entry to add"));
        }
    }
    for change in changes {
        if change.text.as_deref().is_some_and(finds) {
            return Err(credential("An entry's new text"));
        }
    }
    if changes
        .iter()
        .any(|change| change.action == EntryAction::Promote)
    {
        // Everything of the entry that the repository's file would hold:
        // its evidence and its check with its text.
        let snapshot = project.load(held);
        for change in changes {
            if change.action == EntryAction::Promote
                && let Some(entry) = snapshot.find(&change.id)
                && entry_holds(entry, &finds)
            {
                return Err(credential("An entry to promote"));
            }
        }
    }
    project
        .update(changes, add, held)
        .map_err(|failure| refused(&failure))
}

/// Every entry of a project, for the command line: with what has expired
/// marked first, since nothing here is on a turn's path. Blocks.
pub(crate) fn listing(project: &Project) -> ProjectEntriesResult {
    let mut snapshot = project.load(None);
    if snapshot.has_lapsed(project.now())
        && let Ok(lapsed) = project.lapse(Some(&snapshot))
    {
        snapshot = lapsed.snapshot;
    }
    ProjectEntriesResult {
        entries: snapshot.listing(),
        problems: snapshot.problems().to_vec(),
    }
}

/// `project/update`: a `user` client's alone.
pub(crate) async fn update(
    core: &Core,
    client: ClientId,
    params: ProjectUpdateParams,
) -> Result<ProjectEntriesResult, OpError> {
    if !is_user(core, client) {
        return Err(error(
            ErrorKind::Usage,
            cortex_protocol::error::code::ROLE_NOT_ALLOWED,
            "Only a `user` client may change the project profile.",
        ));
    }
    let target = target(
        core,
        client,
        params.session_id.as_ref(),
        params.workspace.as_deref(),
    )?;
    let (project, key, held) = match &target {
        Target::Session(knowledge) => (
            knowledge.project.clone(),
            knowledge.key.clone(),
            knowledge.peek(),
        ),
        Target::Workspace(project, key) => (project.clone(), key.clone(), None),
    };
    let filter = Arc::clone(&core.inner.providers.filter);
    let promoted = params
        .changes
        .iter()
        .any(|change| change.action == EntryAction::Promote);
    let updated = Core::blocking(move || {
        apply(
            &project,
            &filter,
            held.as_deref(),
            &params.changes,
            &params.add,
        )
    })
    .await?;

    // Every session of the project in this process takes what follows at
    // once: an accepted command stops asking, and one turned off asks.
    let sessions: Vec<Arc<crate::core::Live>> = core.sessions_of(&key);
    let snapshot = Arc::new(updated.snapshot);
    let result = ProjectEntriesResult {
        entries: updated.changed.iter().map(Entry::to_wire).collect(),
        problems: snapshot.problems().to_vec(),
    };
    // On the blocking pool: handing a session its rules looks at the
    // files, and reads them again when another change came in between.
    Core::blocking(move || {
        for live in &sessions {
            if let Some(knowledge) = &live.knowledge {
                knowledge.took(&snapshot);
                knowledge.updated(&updated.changed, &updated.removed);
                if promoted {
                    knowledge.touched_shared_file();
                }
            }
        }
        Ok(())
    })
    .await?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a test fails by panicking")]

    use super::*;

    fn rule(line: &str) -> Rule {
        exact_command_rule(line, ShellSyntax::Posix).unwrap()
    }

    /// A change of level gives the new engine the learned rules while it
    /// holds them. Handed over after the lock was let go, the list of a
    /// moment ago could arrive after a `disable`'s empty one, and the
    /// command would run unasked on an engine believed to have no rule
    /// (found by the independent read of the trust path).
    #[test]
    fn the_rules_are_handed_to_a_new_engine_while_they_are_held() {
        let given = Mutex::new(Given {
            rules: vec![rule("cargo test")],
            from: None,
            until: None,
        });
        let handed = regive(&given, |rules| {
            assert_eq!(rules, [rule("cargo test")]);
            assert!(
                given.try_lock().is_err(),
                "another change of the rules could come between the copy and the hand-over"
            );
            Ok::<(), ()>(())
        });
        assert_eq!(handed, Ok(()));
        assert_eq!(lock(&given).rules, [rule("cargo test")]);

        // A list the engine refuses leaves no rule held: none is in force.
        assert_eq!(regive(&given, |_| Err(())), Err(()));
        assert_eq!(lock(&given).rules, Vec::<Rule>::new());
    }

    #[test]
    fn every_text_of_an_entry_that_a_file_keeps_is_looked_at_for_a_credential() {
        let finds = |text: &str| text.contains("SECRET");
        let entry = |change: &dyn Fn(&mut Entry)| {
            let mut entry = Entry::written(
                EntryId::new("k_1"),
                cortex_knowledge::Scope::Private,
                Kind::Command,
                "Runs the tests.",
            );
            entry.command = Some(cortex_knowledge::Command {
                purpose: Purpose::Test,
                run: "cargo test".to_owned(),
                cwd: Some("crates/app".to_owned()),
                usual_ms: None,
                read_only: false,
            });
            change(&mut entry);
            entry
        };
        assert!(!entry_holds(&entry(&|_| {}), &finds));
        let with: [&dyn Fn(&mut Entry); 8] = [
            &|entry| entry.text = "SECRET".to_owned(),
            &|entry| entry.evidence = Some("SECRET".to_owned()),
            &|entry| entry.command.as_mut().unwrap().run = "SECRET".to_owned(),
            &|entry| entry.command.as_mut().unwrap().cwd = Some("SECRET".to_owned()),
            &|entry| {
                entry.check = Some(Check::Exists {
                    path: "SECRET".to_owned(),
                });
            },
            &|entry| {
                entry.check = Some(Check::Contains {
                    path: "SECRET".to_owned(),
                    pattern: "x".to_owned(),
                });
            },
            &|entry| {
                entry.check = Some(Check::Contains {
                    path: "x".to_owned(),
                    pattern: "SECRET".to_owned(),
                });
            },
            &|entry| {
                entry.check = Some(Check::Command {
                    command: "curl -H SECRET".to_owned(),
                });
            },
        ];
        for (index, change) in with.into_iter().enumerate() {
            assert!(entry_holds(&entry(change), &finds), "case {index}");
        }
    }
}
