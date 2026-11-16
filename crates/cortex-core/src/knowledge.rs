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
//! # One place holds a session's learned rules
//!
//! [`Knowledge`] keeps the rules it last gave the engine. A change of
//! level makes a new engine, which is given them again from here
//! ([`Knowledge::reapply`]); nothing else holds them, so a command the
//! user turned off cannot come back with a new engine.
//!
//! # Only this project's store
//!
//! A session's `Knowledge` is made for the session's own workspace, and
//! `remember`, the observer and the prompt reach no other. `project/*`
//! names a project by a session the client has open, or by a workspace
//! root, which only a `user` client may do.

#![allow(
    clippy::disallowed_methods,
    reason = "the two stamps looked at before a command's decision are two stat calls; \
              everything else here that blocks runs on the blocking pool"
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
    snapshot: Mutex<Option<Snapshot>>,
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
    tokens: u32,
    learn: bool,
    rounds: Option<Arc<crate::checkpoints::Rounds>>,
    shared_file: Option<cortex_base::AbsPath>,
    /// What this session's commands have shown so far.
    watch: Mutex<Watch>,
    /// The learned rules the engine was last given: the one place they are
    /// held.
    learned: Mutex<Vec<Rule>>,
    /// What the user was told already, so that each thing is said once.
    told: Mutex<BTreeSet<String>>,
}

impl std::fmt::Debug for Knowledge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Knowledge")
            .field("session", &self.session)
            .field("project", &self.key)
            .finish_non_exhaustive()
    }
}

/// An entry's command as a person is shown it.
fn shown(entry: &Entry) -> String {
    match &entry.command {
        Some(command) => format!("`{}`", visible(&command.run)),
        None => format!("\"{}\"", visible(&entry.text)),
    }
}

impl Knowledge {
    pub(crate) fn new(parts: Parts) -> Self {
        Self {
            project: parts.project,
            key: parts.key,
            shared: parts.shared,
            session: parts.session,
            policy: parts.policy,
            hooks: parts.hooks,
            syntax: parts.syntax,
            filter: parts.filter,
            tokens: parts.tokens,
            learn: parts.learn,
            rounds: parts.rounds,
            shared_file: parts.shared_file,
            watch: Mutex::new(Watch::new()),
            learned: Mutex::new(Vec::new()),
            told: Mutex::new(BTreeSet::new()),
        }
    }

    /// Tells the user something, once in this session.
    fn tell_once(&self, message: String) {
        if lock(&self.told).insert(message.clone()) {
            self.hooks.tell(message);
        }
    }

    fn finds_secret(&self, text: &str) -> bool {
        self.filter
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finds(text)
    }

    /// The snapshot every session of the project last read, if any has.
    fn peek(&self) -> Option<Snapshot> {
        lock(&self.shared.snapshot).clone()
    }

    /// The project's knowledge as it is now: read again when either file
    /// changed since the last read, with every entry past its expiry
    /// marked. Blocks.
    fn refresh(&self) -> Snapshot {
        let held = self.peek();
        let mut snapshot = match held {
            Some(held) if self.project.is_current(&held) => held,
            held => self.project.load(held.as_ref()),
        };
        if snapshot.has_lapsed(self.project.now()) {
            match self.project.lapse(Some(&snapshot)) {
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
                    snapshot = lapsed.snapshot;
                }
                // The entry is out of use by the clock already; the mark
                // waits for a store that can be written.
                Err(error) => self.tell_once(format!(
                    "An expired entry of the project profile could not be marked: {error}"
                )),
            }
        }
        self.took(&snapshot);
        snapshot
    }

    /// Takes `snapshot` as the project's knowledge now: every session of
    /// the project in this process sees it, and this session's engine is
    /// given the rules that follow from it.
    pub(crate) fn took(&self, snapshot: &Snapshot) {
        *lock(&self.shared.snapshot) = Some(snapshot.clone());
        for problem in snapshot.problems() {
            self.tell_once(problem.clone());
        }
        self.give_rules(snapshot);
    }

    /// Gives the engine one exact allow rule for each command of
    /// `snapshot` that the user accepted, in place of the ones it had.
    fn give_rules(&self, snapshot: &Snapshot) {
        let now = self.project.now();
        let rules: Vec<Rule> = snapshot
            .runs_unasked(now)
            .filter_map(|entry| entry.command.as_ref())
            .filter_map(|command| exact_command_rule(&command.run, self.syntax))
            .collect();
        let mut learned = lock(&self.learned);
        if *learned == rules {
            return;
        }
        let engine = self.policy.engine();
        match engine.set_learned(rules.clone()) {
            Ok(()) => *learned = rules,
            Err(errors) => {
                // No learned rule is in force then.
                learned.clear();
                self.tell_once(format!(
                    "The project's accepted commands could not be given to the permission \
                     engine, so each asks before it runs: {}",
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
        }
        let waiting = engine
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
    }

    /// Gives the engine in force the learned rules again: after a change
    /// of level made a new one.
    pub(crate) fn reapply(&self) {
        let rules = lock(&self.learned).clone();
        if let Err(errors) = self.policy.engine().set_learned(rules) {
            lock(&self.learned).clear();
            tracing::warn!(
                ?errors,
                "the learned rules could not be given to the new engine"
            );
        }
    }

    /// Before a command is decided: when either of the project's files
    /// changed since they were read, every learned rule is dropped at
    /// once, and given again from the files at the next turn's start.
    /// Another process's `cortex project disable` then takes a command off
    /// the safe list before its next run, not at the next turn.
    pub(crate) fn before_command(&self) {
        let mut learned = lock(&self.learned);
        if learned.is_empty() {
            return;
        }
        let current = self
            .peek()
            .is_some_and(|snapshot| self.project.is_current(&snapshot));
        if !current {
            learned.clear();
            if let Err(errors) = self.policy.engine().set_learned(Vec::new()) {
                tracing::warn!(?errors, "the learned rules could not be taken back");
            }
        }
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
    pub(crate) fn observed(self: &Arc<Self>, run: &Run) {
        // The watch sees every run, for what changed between two of them.
        let seen = lock(&self.watch).see(run, Some(&self.session));
        if !self.learn || seen.is_empty() || self.finds_secret(&run.command) {
            return;
        }
        let now = self.project.now();
        let held = self.peek();
        for learnt in seen {
            // A line the engine could not write as an exact rule is not
            // learned: it could never run unasked, and is not one command.
            if exact_command_rule(&learnt.line, self.syntax).is_none() {
                continue;
            }
            if held.as_ref().is_some_and(|held| !held.wants(&learnt, now)) {
                continue;
            }
            // A flip is told from what the harness saw change, and a
            // session kept in memory does not see what its file tools
            // write.
            if learnt.kind == Kind::Flaky && self.rounds.is_none() {
                continue;
            }
            let this = Arc::clone(self);
            // Not waited for: the command's result goes back to the model
            // now, and the store is written beside it.
            drop(tokio::task::spawn_blocking(move || this.learn(&learnt)));
        }
    }

    /// Stores one thing the observer saw. Blocks.
    fn learn(&self, learnt: &Learnt) {
        let held = self.peek();
        match self.project.learn(learnt, held.as_ref()) {
            Ok((outcome, updated)) => {
                self.took(&updated.snapshot);
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
                 {error}"
            )),
        }
    }

    /// Every entry, for `project/list`. Blocks.
    fn list(&self) -> ProjectEntriesResult {
        let snapshot = self.refresh();
        ProjectEntriesResult {
            entries: snapshot.listing(),
            problems: snapshot.problems().to_vec(),
        }
    }
}

impl cortex_agent::ProfileSource for Knowledge {
    fn section(&self) -> String {
        let snapshot = self.refresh();
        let section = cortex_knowledge::render::render_with(
            &snapshot,
            self.project.now(),
            self.tokens,
            &|line| !self.finds_secret(line),
        );
        if section.left_out > 0 {
            self.tell_once(format!(
                "The project profile is larger than its budget of {} tokens: {} of its entries \
                 are left out of the prompt. `cortex project list` shows them all.",
                self.tokens, section.left_out
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
        if self.finds_secret(&proposal.text) || self.finds_secret(&proposal.evidence) {
            return Err(ProposalError::new(
                "The entry holds what looks like a credential, so it was not stored. Say it \
                 without the credential.",
            ));
        }
        let check = proposal
            .check
            .as_ref()
            .map(Check::from_wire)
            .transpose()
            .map_err(ProposalError::new)?;
        let held = self.peek();
        let stored = self.project.propose(
            Proposed {
                kind,
                text: proposal.text,
                evidence: proposal.evidence,
                check,
                session: Some(self.session.clone()),
            },
            held.as_ref(),
        );
        match stored {
            Ok((outcome, updated)) => {
                self.took(&updated.snapshot);
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
            "invalid_request",
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
    // What the user writes must not put a credential into every later
    // prompt, or into the repository.
    let filter = Arc::clone(&core.inner.providers.filter);
    let promoted = params
        .changes
        .iter()
        .any(|change| change.action == EntryAction::Promote);
    let updated = Core::blocking(move || {
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
        for new in &params.add {
            if !matches!(new.kind, EntryKind::Unknown(_))
                && (finds(&new.text)
                    || new
                        .command
                        .as_ref()
                        .is_some_and(|command| finds(&command.command)))
            {
                return Err(credential("An entry to add"));
            }
        }
        for change in &params.changes {
            if change.text.as_deref().is_some_and(finds) {
                return Err(credential("An entry's new text"));
            }
        }
        if promoted {
            let snapshot = project.load(held.as_ref());
            for change in &params.changes {
                if change.action == EntryAction::Promote
                    && let Some(entry) = snapshot.find(&change.id)
                    && (finds(&entry.text)
                        || entry
                            .command
                            .as_ref()
                            .is_some_and(|command| finds(&command.run)))
                {
                    return Err(credential("An entry to promote"));
                }
            }
        }
        project
            .update(&params.changes, &params.add, held.as_ref())
            .map_err(|failure| refused(&failure))
    })
    .await?;

    // Every session of the project in this process takes what follows at
    // once: an accepted command stops asking, and one turned off asks.
    let sessions: Vec<Arc<crate::core::Live>> = core.sessions_of(&key);
    for live in &sessions {
        if let Some(knowledge) = &live.knowledge {
            knowledge.took(&updated.snapshot);
            knowledge.updated(&updated.changed, &updated.removed);
            if promoted {
                knowledge.touched_shared_file();
            }
        }
    }
    Ok(ProjectEntriesResult {
        entries: updated.changed.iter().map(Entry::to_wire).collect(),
        problems: updated.snapshot.problems().to_vec(),
    })
}
