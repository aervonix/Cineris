//! The session's side of finding its extensions: the task that finds,
//! judges and registers them when the session opens, and the look at a
//! turn's start that does it again when what they were found in changed
//! (the [parent module](super) says what each rule is).
//!
//! **The session's skills are found on the same task,** after its
//! extensions and before the first turn may start, and found again when
//! what they were found in changed: the user's three roots always, and the
//! project's three whether or not it is trusted (skills are text, as
//! instruction files are). Each place is stamped before it is looked at or
//! read, as an extension's are, following no link (`skills::find_seeing`).
//! What the skills and the loaded extensions give the system prompt is
//! settled into the session's [`PromptParts`] before the work is settled,
//! so that a turn that waits for it is built with them.
//!
//! **The folder of each of the user's own skills outside the workspace may
//! be read unasked:** it is given to the session as a rule of its own, an
//! allow for reading the folder and everything below it
//! ([`crate::policy::reading_below`]), added as "allow for this session"
//! adds one, with the source `session`, at each finding of the skills
//! ([`super::reads`] says which folders, and how the rules follow them),
//! **while `permissions.read_skill_folders` is on**, which only the user's
//! own configuration may do and which is off by default (the owner's
//! decision: a skill's folder may hold a `.env` its scripts use, and what
//! is read is sent to the provider).
//! The folder is where the skill really is, as `skills::find` judged it, so
//! a skill whose folder is a link out of its root, which is not found, has
//! none; a rule that holds reading back still wins, as it wins over any
//! allow.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use cineris_base::{
    CancelToken, ContentHash, PathResolver, SharedClock, SharedTimer, in_code, visible,
};
use cineris_config::StateDir;
use cineris_ext::cache::{Entry, RegistrationCache};
use cineris_ext::discover::{self, Discovered, LeftOut, Root, Seen};
use cineris_ext::folder::Snapshot;
use cineris_ext::skills;
use cineris_ext::source::Source;
use cineris_policy::{Definition, PinStore, ProjectTrust, Standing, TrustStore, UserPins};
use cineris_protocol::ProjectKey;
use cineris_protocol::methods::TrustResourceKind;
use cineris_tools::ToolRegistry;

use super::{
    BEFORE_AN_EXTENSION_IS_REGISTERED, BEFORE_THE_CACHE_IS_KEPT, Extensions, Listed, Looked,
    Pending, UserNames, in_project, sentence, with_paths_shown,
};
use crate::ext::prompt::PromptParts;
use crate::ext::watched::{MAX_PLACES_PER_ROOT, Watched};
use crate::ext::{ApprovedExtension, HostParts};
use crate::ext::{Gate, ScriptHooks, SeenHooks, SessionStart, Stand, way_out};
use cineris_ext::event::Event;

/// How the session is told something.
type Tell = Arc<dyn Fn(String) + Send + Sync>;

/// What a session finds its extensions with.
pub(crate) struct Finding {
    /// The session's path resolver: its workspace, and the home directory
    /// `~` stands for.
    pub(crate) resolver: Arc<PathResolver>,
    /// The configuration directory, as it was given: the user's folder of
    /// extensions is in it.
    pub(crate) config_dir: Option<PathBuf>,
    /// The home directory, which a path the user is shown is under.
    pub(crate) home: Option<PathBuf>,
    /// The state directory: the pins, the trust store and the cache.
    pub(crate) state: Option<StateDir>,
    /// The project's key, when the project is trusted (in the store, or for
    /// the run): its folder of extensions is looked in.
    pub(crate) project: Option<ProjectKey>,
    /// What the trust question found in the project's folder, when it
    /// looked: used in place of a second look, its places stamped as they
    /// were before the question read them.
    pub(crate) project_found: Option<Looked>,
    /// The project's extensions a yes the trust store could not keep
    /// approved, by the hashes shown: approved for this session.
    pub(crate) for_the_run: Vec<Definition>,
    /// The project is trusted for the run by the command line
    /// (`--trust-project`), which approves no extension.
    pub(crate) trust_project: bool,
    /// The names the user's configuration turns off
    /// (`[extensions.<name>] enabled = false`).
    pub(crate) disabled: Vec<String>,
    /// Where the approved extensions' tools go.
    pub(crate) registry: Arc<ToolRegistry>,
    /// What their calls are served with, and how the session is told.
    pub(crate) parts: Arc<HostParts>,
    /// The roots skills are looked for in (`skills::Root::standard`, as
    /// `compat.skills` says).
    pub(crate) skills: Vec<skills::Root>,
    /// Whether the user's own skills' folders may be read unasked
    /// (`permissions.read_skill_folders`, the user's alone, off by
    /// default). Off, the skills give the session no rule.
    pub(crate) read_skill_folders: bool,
    /// Where what the skills and the extensions give the system prompt is
    /// settled, for the loop to ask.
    pub(crate) prompt: Arc<PromptParts>,
    /// Where the hooks of the extensions loaded are put on their events.
    pub(crate) scripts: Arc<ScriptHooks>,
    /// The session the `session.start` hooks of its extensions are heard
    /// for, as it opened: set by the core once the session is open.
    pub(crate) session_start: Option<SessionStart>,
    /// Ends the work when the session or the core ends.
    pub(crate) cancel: CancelToken,
}

impl std::fmt::Debug for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Finding")
            .field("config_dir", &self.config_dir)
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
}

/// What a session keeps of its extensions from one turn to the next.
#[derive(Default)]
pub(crate) struct Shared {
    held: tokio::sync::Mutex<Option<Held>>,
    cancel: Mutex<Option<CancelToken>>,
    /// The session's timer and how it is told, for a wait that is said.
    waiting: Mutex<Option<(SharedTimer, Tell)>>,
    /// Set once the extensions are found and their `session.start` hooks
    /// are heard: what a wait that is said then waits for.
    hearing: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

/// An extension as the session holds it: where it comes from, its name,
/// and the hash it was loaded at.
type Key = (Source, String, ContentHash);

/// What the session found, and loaded.
struct Held {
    inputs: Arc<Inputs>,
    registry: Arc<ToolRegistry>,
    parts: Arc<HostParts>,
    /// What is loaded, by the hash it was loaded at.
    loaded: Vec<Key>,
    /// The hashes whose evaluation failed: not evaluated again until the
    /// folder changes, which gives it another hash.
    failed: Vec<ContentHash>,
    /// What the session was told of what was found: not said again.
    told: HashSet<String>,
    /// What it was found in, stamped.
    watched: Watched,
    /// What the skills were found in, stamped: looked at apart, so that a
    /// change to a skill does not have every extension read again.
    skills_watched: Watched,
    /// The prompt text of each extension loaded, by what it was loaded as.
    texts: Vec<(Key, String)>,
    /// Where sections 6 and 7 are settled.
    prompt: Arc<PromptParts>,
    /// Where the loaded extensions' hooks are put on their events.
    scripts: Arc<ScriptHooks>,
    /// What stands approved and could not be loaded or registered (its
    /// capabilities do not compile, the registry refused it, its first
    /// registration failed): the gates it holds while it stays so.
    unloaded: Vec<(Key, Gate)>,
    /// The session's rules for reading its skills' folders, changed on
    /// the blocking pool by the work that makes them.
    reads: Arc<Mutex<super::reads::SkillReads>>,
}

/// What the blocking look is given.
struct Inputs {
    resolver: Arc<PathResolver>,
    config_dir: Option<PathBuf>,
    home: Option<PathBuf>,
    state: Option<StateDir>,
    project: Option<ProjectKey>,
    for_the_run: Vec<Definition>,
    trust_project: bool,
    disabled: Vec<String>,
    skills: Vec<skills::Root>,
    read_skill_folders: bool,
    clock: SharedClock,
}

/// What the blocking look decided.
#[derive(Default)]
struct Decided {
    /// What to tell the session.
    told: Vec<String>,
    /// Approved, with what they registered from the cache, and the text of
    /// the prompt file the cache holds.
    approved: Vec<(ApprovedExtension, Option<String>)>,
    /// Approved, and missing from the cache: registered now, from the
    /// snapshot whose hash is the approved one.
    missing: Vec<(Listed, Arc<Snapshot>)>,
    /// The cache, when it was read: written and pruned after.
    cache: Option<RegistrationCache>,
    /// What everything was found in, stamped before it was read.
    watched: Watched,
    /// The extensions that stand approved at a hash they are not found at
    /// (changed, or unreadable as approved): their gating hooks are held on
    /// their events, failing, from the registration their pin names
    /// (`docs/steps/28/answered.md`, point 5).
    held: Vec<Gate>,
    /// Each extension found, by where it comes from and its name.
    found: Vec<(Source, String)>,
    /// The stores of approvals that could not be read, by the source whose
    /// approvals they hold, with the file as the user is shown it.
    unread: Vec<(Source, String)>,
    /// The sources whose store of approvals was read whole.
    judged: Vec<Source>,
    /// The extensions of those sources that stand approved, at any hash.
    standing: Vec<(Source, String)>,
    /// Each extension found and not loaded, by where it comes from and its
    /// name, with what the session is told of why: what a command of it is
    /// refused with.
    not_loaded: Vec<(Source, String, String)>,
}

/// The cache's part of the work, done after the session's extensions are
/// settled: what was registered written, and the cache pruned.
struct Kept {
    cache: RegistrationCache,
    written: Vec<(ContentHash, Entry)>,
    state: StateDir,
    clock: SharedClock,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts finding a session's extensions, on a task of its own: what the
/// session's first turn waits for.
pub(crate) fn start(pending: Pending) -> Arc<Extensions> {
    let Some(finding) = pending else {
        return Extensions::none();
    };
    let extensions = Arc::new(Extensions::default());
    *lock(&extensions.held.cancel) = Some(finding.cancel.clone());
    *lock(&extensions.held.waiting) = Some((
        Arc::clone(&finding.parts.timer),
        Arc::clone(&finding.parts.tell),
    ));
    let task = Arc::clone(&extensions);
    tokio::spawn(async move {
        let _settles = Settles(Arc::clone(&task));
        let cancel = finding.cancel.clone();
        let _ = cancel.race(first(&task, finding)).await;
    });
    extensions
}

/// Settles a session's extensions when it is dropped: when the work ends,
/// however it ends (done, cancelled, or a panic), so that a turn never
/// waits for work that is gone. Work that ended before it settled them
/// fails them: no turn starts as if they were found, since a gate one of
/// them holds would be skipped (a session that closes meanwhile starts no
/// turn anyway).
struct Settles(Arc<Extensions>);

/// Why a session's extensions failed when their finding ended before it was
/// done.
pub(crate) const FINDING_ENDED: &str = "the work that finds them ended before it was done";

impl Drop for Settles {
    fn drop(&mut self) {
        self.0.ready.fail(FINDING_ENDED.to_owned());
    }
}

/// The session's first finding: settled as soon as its tools are in the
/// registry, and the cache written and pruned after that, which no turn
/// waits for.
async fn first(extensions: &Extensions, finding: Finding) {
    let Finding {
        resolver,
        config_dir,
        home,
        state,
        project,
        project_found,
        for_the_run,
        trust_project,
        disabled,
        registry,
        parts,
        skills,
        read_skill_folders,
        prompt,
        scripts,
        session_start,
        cancel: _,
    } = finding;
    let inputs = Arc::new(Inputs {
        resolver,
        config_dir,
        home,
        state,
        project,
        for_the_run,
        trust_project,
        disabled,
        skills,
        read_skill_folders,
        clock: Arc::clone(&parts.clock),
    });
    let kept = {
        let mut slot = extensions.held.held.lock().await;
        let mut held = Held {
            inputs,
            registry,
            parts,
            loaded: Vec::new(),
            failed: Vec::new(),
            told: HashSet::new(),
            watched: Watched::default(),
            skills_watched: Watched::default(),
            texts: Vec::new(),
            prompt,
            scripts: Arc::clone(&scripts),
            unloaded: Vec::new(),
            reads: Arc::default(),
        };
        let kept = held.settle(project_found).await;
        held.find_skills().await;
        let tell = Arc::clone(&held.parts.tell);
        *slot = Some(held);
        (kept, tell)
    };
    let (kept, tell) = kept;
    let kept = match kept {
        Ok(kept) => kept,
        Err(why) => {
            // No turn starts: a gate one of them holds would be skipped.
            extensions.ready.fail(why);
            return;
        }
    };
    // The `session.start` hooks of the extensions it loaded as it opened:
    // the first turn waits for those that are waited for, as for a hook
    // command's (`docs/steps/28/answered.md`, point 6).
    if let Some(started) = &session_start {
        extensions
            .held
            .hearing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        scripts.session_started(started, &*tell).await;
    }
    extensions.ready.settle();
    if let Some(kept) = kept {
        cineris_base::hook::await_point!(BEFORE_THE_CACHE_IS_KEPT);
        let _ = blocking(move || kept.keep()).await;
    }
}

impl Shared {
    /// At a turn's start: when what the extensions were found in changed,
    /// they are found and judged again, what no longer stands approved as
    /// it was loaded is withdrawn, and what now stands approved is loaded.
    pub(crate) async fn recheck(&self) {
        let kept = {
            let mut slot = self.held.lock().await;
            let Some(held) = slot.as_mut() else {
                return;
            };
            let watched = std::mem::take(&mut held.watched);
            let skills = std::mem::take(&mut held.skills_watched);
            let Some((changed, watched, skills_changed, skills)) = blocking(move || {
                (
                    watched.changed(),
                    watched,
                    skills.changed().is_some(),
                    skills,
                )
            })
            .await
            else {
                return;
            };
            held.watched = watched;
            held.skills_watched = skills;
            if skills_changed {
                held.find_skills().await;
            }
            if changed.is_none() {
                return;
            }
            match held.settle(None).await {
                Ok(kept) => kept,
                Err(why) => {
                    // What is loaded, and what is held, stays as it was.
                    held.tell(format!(
                        "The session's extensions were not found again after a change: {why}. \
                         What was loaded, and what holds its gates, stays as it was."
                    ));
                    None
                }
            }
        };
        if let Some(kept) = kept {
            let _ = blocking(move || kept.keep()).await;
        }
    }

    /// Ends the session's finding, if it is still under way.
    pub(crate) fn stop(&self) {
        if let Some(cancel) = lock(&self.cancel).as_ref() {
            cancel.cancel();
        }
    }

    /// The session's timer and how it is told, once the finding started.
    pub(crate) fn waiting(&self) -> Option<(SharedTimer, Tell)> {
        lock(&self.waiting).clone()
    }

    /// What a turn that has waited long is told: of the finding, or, once
    /// the extensions are found, of their `session.start` hooks.
    pub(crate) fn waiting_said(&self) -> String {
        if self.hearing.load(std::sync::atomic::Ordering::SeqCst) {
            super::WAITING_FOR_HOOKS.to_owned()
        } else {
            super::WAITING.to_owned()
        }
    }
}

impl Held {
    /// Says `message`, unless the session was told it already.
    fn tell(&mut self, message: String) {
        if self.told.insert(message.clone()) {
            (self.parts.tell)(message);
        }
    }

    /// Finds and judges the extensions, withdraws what no longer stands
    /// approved as it was loaded, and loads what now stands approved: from
    /// the cache, or registered first. What is left of the cache's work.
    ///
    /// # Errors
    ///
    /// Why they could not be found: the work that finds them did not
    /// finish.
    async fn settle(&mut self, project_found: Option<Looked>) -> Result<Option<Kept>, String> {
        let inputs = Arc::clone(&self.inputs);
        let Decided {
            told,
            approved,
            missing,
            cache,
            watched,
            mut held,
            found,
            unread,
            judged,
            standing: still_approved,
            mut not_loaded,
        } = blocking(move || decide(&inputs, project_found))
            .await
            .ok_or_else(|| "the work that finds and judges them did not finish".to_owned())?;
        // What left the session for good (its pin, or its project's trust,
        // taken away) is forgotten: a store that cannot be read later gives
        // it no gate back (the third re-check of the full review).
        self.scripts.forget(|source, name| {
            judged.contains(&source)
                && !still_approved
                    .iter()
                    .any(|(approved, named)| *approved == source && named == name)
        });
        // While a store of approvals cannot be read, what the session loaded
        // of that source and is not found now (removed, or its folder left
        // out) keeps its gates, from what the session saw it register (the
        // re-check of the full review, M1').
        for (source, file) in &unread {
            for (name, hash) in self.scripts.loaded_of(*source) {
                if found.contains(&(*source, name.clone())) {
                    continue;
                }
                let was = match source {
                    Source::User => Unstood::PinsUnreadable(Vec::new(), file.clone()),
                    Source::Project => Unstood::TrustUnreadable(Vec::new(), file.clone()),
                };
                held.push(gate(
                    *source,
                    &name,
                    None,
                    hash,
                    &was,
                    None,
                    self.inputs.trust_project,
                ));
            }
        }
        self.watched = watched;
        for message in told {
            self.tell(message);
        }
        let wanted: Vec<Key> = approved
            .iter()
            .map(|(one, _)| (one.source, one.manifest.name.clone(), one.hash))
            .chain(
                missing
                    .iter()
                    .map(|(one, _)| (one.source, one.manifest.name.clone(), one.hash)),
            )
            .collect();
        // What stands approved and is not found as approved holds its gates,
        // failing, from the registration approved, beside what stands
        // approved and could not be loaded: taken before a withdrawn
        // extension's hooks leave, which they may be made from.
        self.unloaded.retain(|(key, _)| wanted.contains(key));
        let found_gates = held;
        self.hold_gates(&found_gates);
        // What is withdrawn leaves the prompt's snapshot first, its text and
        // its source together, then the registry: a text no later than its
        // tools.
        let gone: Vec<String> = self
            .loaded
            .iter()
            .filter(|key| !wanted.contains(key))
            .map(|key| key.1.clone())
            .collect();
        self.prompt.withdraw(&gone);
        for gone in self.loaded.iter().filter(|key| !wanted.contains(key)) {
            // Its hooks with it: an extension's hooks are on their events
            // only while it is loaded as it was approved.
            self.scripts.withdraw(&gone.1);
            self.registry.remove_source(&gone.1);
            (self.parts.tell)(format!(
                "The extension `{}` is withdrawn from this session: its tools are no longer \
                 offered.",
                in_code(&gone.1)
            ));
        }
        self.loaded.retain(|key| wanted.contains(key));
        self.texts.retain(|(key, _)| wanted.contains(key));
        let mut new: Vec<ApprovedExtension> = Vec::new();
        for (one, text) in approved {
            let key = (one.source, one.manifest.name.clone(), one.hash);
            if self.loaded.contains(&key) {
                continue;
            }
            if let Some(text) = text {
                self.texts.push((key, text));
            }
            new.push(one);
        }
        let mut written: Vec<(ContentHash, Entry)> = Vec::new();
        for (listed, snapshot) in missing {
            let hash = listed.hash;
            if self
                .loaded
                .contains(&(listed.source, listed.manifest.name.clone(), hash))
                // One whose evaluation failed is not evaluated again until
                // its folder changes.
                || self.failed.contains(&hash)
            {
                continue;
            }
            let name = in_code(&listed.manifest.name).into_owned();
            // A turn that waits says why.
            (self.parts.tell)(format!(
                "Registering the extension `{name}` (`{}`): what it registers is not in the \
                 registration cache, so it is evaluated once before the turn starts.",
                listed.shown
            ));
            cineris_base::hook::await_point!(BEFORE_AN_EXTENSION_IS_REGISTERED);
            match cineris_ext::engine::register(Arc::clone(&snapshot), self.parts.timer.as_ref())
                .await
            {
                Ok((manifest, registration)) => {
                    // The prompt's text as the runtime reads it: from the
                    // snapshot that was evaluated.
                    let prompt = manifest
                        .prompt
                        .as_deref()
                        .and_then(|file| snapshot.file(file))
                        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                    if let Some(text) = &prompt {
                        self.texts
                            .push(((listed.source, manifest.name.clone(), hash), text.clone()));
                    }
                    written.push((hash, Entry::new(&manifest, registration.clone(), prompt)));
                    new.push(ApprovedExtension {
                        source: listed.source,
                        folder: listed.folder,
                        hash,
                        manifest,
                        registration,
                    });
                }
                Err(error) => {
                    self.failed.push(hash);
                    // What a script threw, escaped; and said once.
                    let said = format!(
                        "The extension `{name}` (`{}`) could not be registered, so it is not \
                         loaded: {}",
                        listed.shown,
                        visible(&error.to_string())
                    );
                    not_loaded.push((listed.source, listed.manifest.name.clone(), said.clone()));
                    self.tell(said);
                    // What it registers is known nowhere: each gating event
                    // its manifest lists holds a failing gate (the
                    // maintainers' call).
                    let key = (listed.source, listed.manifest.name.clone(), hash);
                    // Known as approved, for a store that cannot be read
                    // later (the third re-check of the full review, L1).
                    self.scripts.note(
                        listed.source,
                        &listed.manifest.name,
                        hash,
                        SeenHooks::Events(listed.manifest.capabilities.hooks.clone()),
                    );
                    let held = gate(
                        listed.source,
                        &listed.manifest.name,
                        Some(&listed.shown),
                        hash,
                        &Unstood::Unregistered(listed.manifest.capabilities.hooks.clone()),
                        None,
                        self.inputs.trust_project,
                    );
                    self.unloaded.push((key, held));
                }
            }
        }
        let keys: Vec<Key> = new
            .iter()
            .map(|one| (one.source, one.manifest.name.clone(), one.hash))
            .collect();
        // What each registered: an extension that cannot be loaded holds its
        // gates from it.
        let known: Vec<(Key, Vec<cineris_ext::registration::HookDef>)> = new
            .iter()
            .map(|one| {
                (
                    (one.source, one.manifest.name.clone(), one.hash),
                    one.registration.hooks.clone(),
                )
            })
            .collect();
        for (name, message) in crate::ext::register(new, &self.parts, &self.registry, &self.scripts)
        {
            (self.parts.tell)(message.clone());
            if let Some((key, hooks)) = known.iter().find(|(key, _)| key.1 == name) {
                not_loaded.push((key.0, key.1.clone(), message.clone()));
                self.scripts
                    .note(key.0, &key.1, key.2, SeenHooks::Registered(hooks.clone()));
                let held = gate(
                    key.0,
                    &key.1,
                    None,
                    key.2,
                    &Unstood::Unloaded,
                    Some(hooks.clone()),
                    self.inputs.trust_project,
                );
                self.unloaded.push((key.clone(), held));
            }
        }
        self.loaded.extend(keys);
        self.hold_gates(&found_gates);
        // What a command of an extension not loaded is refused with: what the
        // session was told of it, or why its gates are held (one not found
        // where it was approved has no other word).
        for gate in found_gates
            .iter()
            .chain(self.unloaded.iter().map(|(_, gate)| gate))
        {
            if !not_loaded
                .iter()
                .any(|(source, name, _)| *source == gate.source && *name == gate.name)
            {
                not_loaded.push((gate.source, gate.name.clone(), sentence(&gate.why)));
            }
        }
        self.scripts.set_not_loaded(not_loaded);
        // Section 7: the text of each extension loaded, as the cache holds
        // it, and only of one the registry took, as the turn will find it:
        // one whose capabilities did not compile is not loaded.
        let sources: HashSet<String> = self
            .registry
            .sources()
            .into_iter()
            .map(|source| source.name)
            .collect();
        self.texts
            .retain(|(key, _)| self.loaded.contains(key) && sources.contains(&key.1));
        self.prompt.settle(
            self.texts
                .iter()
                .map(|((_, name, _), text)| cineris_agent::ExtensionPrompt {
                    name: name.clone(),
                    text: text.clone(),
                })
                .collect(),
        );
        let Some(state) = self.inputs.state.clone() else {
            return Ok(None);
        };
        Ok(cache.map(|cache| Kept {
            cache,
            written,
            state,
            clock: Arc::clone(&self.inputs.clock),
        }))
    }
}

impl Held {
    /// Holds the gates of `found` (what stands approved and is not found as
    /// approved) and of what stands approved and could not be loaded, and
    /// tells the user of each, once a session.
    fn hold_gates(&mut self, found: &[Gate]) {
        let gates: Vec<Gate> = found
            .iter()
            .cloned()
            .chain(self.unloaded.iter().map(|(_, gate)| gate.clone()))
            .collect();
        let notices: Vec<String> = gates.iter().map(|gate| gate.notice.clone()).collect();
        // Told once a session, in words, of each that holds a gate: the
        // model's refusal of each call it gates stays short (the
        // maintainers' call on the early review of the script hooks).
        for (index, events) in self.scripts.hold(gates) {
            let held = match events.as_slice() {
                [cineris_agent::HookEvent::PromptSubmit] => {
                    "Its gate on `prompt.submit` holds, refusing every prompt"
                }
                [cineris_agent::HookEvent::ToolBefore] => {
                    "Its gate on `tool.before` holds, refusing every call it is on"
                }
                _ => {
                    "Its gates on `prompt.submit` and `tool.before` hold, refusing every prompt, \
                     and every call they are on"
                }
            };
            let told = notices[index].replace(GATES_HELD, held);
            // Said once a session, as all the finding tells.
            self.tell(told);
        }
    }

    /// Finds the session's skills, each place stamped before it is read,
    /// and settles section 6's part of them. On the blocking pool.
    async fn find_skills(&mut self) {
        let inputs = Arc::clone(&self.inputs);
        let Some(((found, watched), home)) =
            blocking(move || (skills_found(&inputs), real_home(&inputs.resolver))).await
        else {
            return;
        };
        self.skills_watched = watched;
        self.prompt
            .set_skills(skills::render(&found, cineris_agent::SKILLS_CAP).text);
        // Off (the default), the skills give no rule: each read of their
        // files outside the workspace is asked about.
        if !self.inputs.read_skill_folders {
            return;
        }
        let workspace = self.inputs.resolver.root().to_fs_path();
        let folders = super::reads::readable(&found.skills, &workspace, home.as_deref());
        let reads = Arc::clone(&self.reads);
        let policy = Arc::clone(&self.parts.policy);
        // Off the core's thread: the change waits for a person's answer
        // being stored for the project, if one is. The rules it makes are
        // held by the same work, so none is left that the session does not
        // know of.
        let made = blocking(move || lock(&reads).follow(&policy, folders)).await;
        // The user is told once, when the first rules are made, in a notice
        // of the session's: the model is not told.
        if let Some(count) = made.flatten() {
            (self.parts.tell)(super::reads::told(count));
        }
    }
}

/// A root gives at most the names on the way to it, where it would be or
/// leads when it is not there, itself, and three places an entry it looks
/// at (`skills::find_seeing`): within what a root may stamp.
const _: () = assert!(5 + 3 * skills::MAX_ENTRIES <= MAX_PLACES_PER_ROOT);

/// Where the home directory really is, every link followed: what a skill's
/// folder is held against. `None` when there is none. Blocks.
fn real_home(resolver: &PathResolver) -> Option<PathBuf> {
    resolver
        .resolve("~")
        .ok()
        .filter(|home| home.exists())
        .map(|home| home.to_fs_path())
}

/// The session's skills, found in their roots, and what they were found in,
/// each place stamped before it was looked at or read. Blocks.
fn skills_found(inputs: &Inputs) -> (skills::Found, Watched) {
    let clock = &inputs.clock;
    let mut watched = Watched::default();
    let found = skills::find_seeing(&inputs.resolver, &inputs.skills, &mut |place| match place {
        // A name on the way by its kind alone: `~/.claude`, which another
        // tool writes in, is no change each time it does.
        skills::Seen::Way(way) => watched.note_kind(way.to_path_buf(), clock),
        skills::Seen::Place(place) => watched.note(place.to_path_buf(), clock),
    });
    (found, watched)
}

/// Runs `work` on the blocking pool. `None` when it could not finish.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    cineris_tools::run_blocking(&CancelToken::new(), move |_| work())
        .await
        .ok()
}

/// The user's folder of extensions, looked in, each place stamped before it
/// is read. Blocks.
fn in_users(inputs: &Inputs, config: &std::path::Path, watched: &mut Watched) -> Discovered {
    let clock = &inputs.clock;
    let mut left = crate::ext::watched::MAX_PLACES_PER_ROOT;
    let discovered = discover::discover_seeing(
        &[Root::user(config)],
        &inputs.resolver,
        &mut |seen| match seen {
            Seen::Root(_, real) => {
                watched.note(real.to_path_buf(), clock);
                true
            }
            Seen::Extension(_, folder) => watched.note_tree(folder, clock, &mut left),
        },
    );
    if !discovered
        .roots
        .iter()
        .any(|(source, _)| *source == Source::User)
    {
        // Not there, or not opened: where it would be, so that one made
        // later is a change.
        let root = Root::user(config).dir;
        let place = if root.is_absolute() {
            Some(root)
        } else {
            root.to_str().and_then(|text| {
                inputs
                    .resolver
                    .resolve(text)
                    .ok()
                    .map(|resolved| resolved.to_fs_path())
            })
        };
        if let Some(place) = place {
            watched.note(place, clock);
        }
    }
    for left in &discovered.left_out {
        watched.note(left.folder.clone(), clock);
    }
    discovered
}

/// Finds the extensions, decides how each stands, and takes what the
/// approved ones registered from the cache. Every place is stamped before
/// it is read. Blocks.
fn decide(inputs: &Inputs, project_found: Option<Looked>) -> Decided {
    let mut decided = Decided::default();
    let clock = &inputs.clock;
    // The stores of the pins first: an approval made, or taken back, in
    // another process is a change.
    if let Some(state) = &inputs.state {
        decided.watched.note(state.pins_file(), clock);
        decided.watched.note(state.trust_file(), clock);
    }
    let user = inputs
        .config_dir
        .as_ref()
        .map(|config| in_users(inputs, config, &mut decided.watched));
    // The project's, while it is trusted, or while the trust store cannot
    // be read, when its extensions hold their gates: what the trust
    // question read of it, or a look now.
    // Read once: a store that is not there is one failed look, and no
    // parse (the full review of the script hooks, L6).
    let (trust_read, trust_unreadable) = read_trust(inputs);
    let project: Option<(Vec<Listed>, Vec<LeftOut>, Option<Discovered>)> =
        inputs.project.as_ref().map(|_| match project_found {
            Some(looked) => {
                decided.watched.extend(looked.watched);
                (looked.found, looked.left_out, None)
            }
            None => {
                let (discovered, watched) = in_project(&inputs.resolver, clock);
                decided.watched.extend(watched);
                (
                    discovered.found.iter().map(Listed::of).collect(),
                    discovered.left_out.clone(),
                    Some(discovered),
                )
            }
        });
    // What was found and could not be read as it is: an extension's gates
    // are held when what was left out is one that stands approved.
    let mut unreadable: Vec<(Source, String, String)> = Vec::new();
    for left in user
        .iter()
        .flat_map(|user| &user.left_out)
        .chain(project.iter().flat_map(|(_, left, _)| left))
    {
        decided.told.push(sentence(&left.reason));
        if let Some(name) = left.folder.file_name().and_then(|name| name.to_str()) {
            unreadable.push((left.source, name.to_owned(), left.shown.clone()));
            decided
                .not_loaded
                .push((left.source, name.to_owned(), sentence(&left.reason)));
        }
    }
    let users = UserNames::of(user.as_ref());
    let mine: Vec<(String, String)> = user
        .iter()
        .flat_map(|user| &user.found)
        .map(|found| {
            (
                found.manifest.name.to_ascii_lowercase(),
                found.shown.clone(),
            )
        })
        .collect();
    // Each one found, with its bytes when they were read now.
    let mut candidates: Vec<(Listed, Option<Arc<Snapshot>>)> = user
        .iter()
        .flat_map(|user| &user.found)
        .map(|found| (Listed::of(found), Some(Arc::clone(&found.snapshot))))
        .collect();
    let mut project_read: Option<Discovered> = None;
    if let Some((found, _, read)) = project {
        let bytes = |listed: &Listed| {
            read.as_ref().and_then(|read| {
                read.found
                    .iter()
                    .find(|found| found.folder == listed.folder)
                    .map(|found| Arc::clone(&found.snapshot))
            })
        };
        candidates.extend(found.into_iter().map(|listed| {
            let snapshot = bytes(&listed);
            (listed, snapshot)
        }));
        project_read = read;
    }
    let mut pins: Option<Option<UserPins>> = None;
    let mut trust: Option<ProjectTrust> = Some(trust_read);
    let mut standing_approved = Vec::new();
    let mut changed: Vec<(Source, String, String)> = Vec::new();
    // Where no store can say what was approved (it cannot be read), each
    // found holds the gates its manifest lists, and is not loaded.
    let mut store_held: Vec<Gate> = Vec::new();
    // The user's found while their pins cannot be read: gated once the
    // registration cache is read (the third re-check of the full review, L2).
    let mut pins_unread: Vec<(String, String, ContentHash, Vec<Event>)> = Vec::new();
    let pins_file = inputs.state.as_ref().map(|state| {
        let file = state.pins_file();
        with_paths_shown(
            &file.display().to_string(),
            &[&file],
            inputs.home.as_deref(),
        )
    });
    let found_names: Vec<(Source, String)> = candidates
        .iter()
        .map(|(listed, _)| (listed.source, listed.manifest.name.clone()))
        .collect();
    for (listed, snapshot) in candidates {
        let name = in_code(&listed.manifest.name).into_owned();
        let definition = listed.definition();
        // What the session is told of why it is not loaded, kept for a
        // command of it too.
        let mut because = |told: &mut Vec<String>, said: String, tell: bool| {
            if tell {
                told.push(said.clone());
            }
            decided
                .not_loaded
                .push((listed.source, listed.manifest.name.clone(), said));
        };
        let standing = match listed.source {
            Source::User => {
                // Turned off by the user: not loaded, and nothing asked or
                // said, but to a command of it.
                if inputs.disabled.contains(&listed.manifest.name) {
                    because(
                        &mut decided.told,
                        format!(
                            "Your extension `{name}` (`{}`) is turned off in your \
                             configuration (`enabled = false`), so it is not loaded.",
                            listed.shown
                        ),
                        false,
                    );
                    continue;
                }
                let Some(pins) = pins
                    .get_or_insert_with(|| user_pins(inputs, &mut decided.told))
                    .as_ref()
                else {
                    because(
                        &mut decided.told,
                        format!(
                            "Your extension `{name}` (`{}`) is not loaded: the pin store, which \
                             holds what you approved of your own, could not be read (said \
                             above).",
                            listed.shown
                        ),
                        true,
                    );
                    pins_unread.push((
                        listed.manifest.name.clone(),
                        listed.shown.clone(),
                        listed.hash,
                        listed.manifest.capabilities.hooks.clone(),
                    ));
                    continue;
                };
                pins.standing(&definition)
            }
            Source::Project => {
                if users.takes(&listed.manifest.name) {
                    because(
                        &mut decided.told,
                        taken(&listed, &name, &mine, users.cut()),
                        true,
                    );
                    continue;
                }
                let standing = project_standing(
                    trust.get_or_insert_with(|| project_trust(inputs)),
                    &inputs.for_the_run,
                    &definition,
                );
                // The trust store cannot be read: nothing says what was
                // approved, but a yes of this session's own. Only what the
                // session loaded keeps its gates: no repository gates a
                // session by itself (the owner's answer on the re-check of
                // the full review, L-b; 2b-1's settled point 10).
                if standing != Standing::Approved
                    && let Some(file) = &trust_unreadable
                {
                    let out = way_out(
                        Source::Project,
                        &listed.manifest.name,
                        None,
                        Stand::StoreUnreadable(file),
                    );
                    because(&mut decided.told, trust_unread_notice(file, &out), true);
                    store_held.push(gate(
                        Source::Project,
                        &listed.manifest.name,
                        Some(&listed.shown),
                        listed.hash,
                        &Unstood::TrustUnreadable(Vec::new(), file.clone()),
                        None,
                        inputs.trust_project,
                    ));
                    continue;
                }
                standing
            }
        };
        if standing == Standing::Approved {
            standing_approved.push((listed, snapshot));
        } else {
            if standing == Standing::Changed {
                changed.push((
                    listed.source,
                    listed.manifest.name.clone(),
                    listed.shown.clone(),
                ));
            }
            let said = not_loaded(&listed, &name, standing, inputs.trust_project);
            because(&mut decided.told, said, true);
        }
    }
    // Each that stands approved at another hash than it is found at, or that
    // is found and cannot be read as it is: its gates are held, from the
    // registration its pin names. One the user turned off, one of a
    // project's whose name is the user's, and one no pin names any more
    // hold nothing: the user's word, or the approval taken back.
    decided.held.extend(store_held);
    decided.found = found_names.clone();
    let mut pending: Vec<(Source, String, String, ContentHash, Unstood)> = Vec::new();
    let mut seen: Vec<(Source, String)> = found_names;
    seen.extend(
        unreadable
            .iter()
            .map(|(source, name, _)| (*source, name.clone())),
    );
    for (source, name, shown, was) in changed
        .into_iter()
        .map(|(source, name, shown)| (source, name, shown, Unstood::Changed))
        .chain(
            unreadable
                .into_iter()
                .map(|(source, name, shown)| (source, name, shown, Unstood::Unreadable)),
        )
        .chain(not_found(
            inputs,
            &seen,
            &mut pins,
            &mut trust,
            &mut decided.told,
        ))
    {
        let released = match source {
            Source::User => inputs.disabled.contains(&name),
            Source::Project => users.takes(&name),
        };
        if released
            || pending
                .iter()
                .any(|held| held.0 == source && held.1 == name)
        {
            continue;
        }
        let pinned = match source {
            Source::User => pins
                .get_or_insert_with(|| user_pins(inputs, &mut decided.told))
                .as_ref()
                .and_then(|pins| pins.get(&TrustResourceKind::Extension, &name))
                .map(|pin| pin.hash),
            Source::Project => inputs
                .for_the_run
                .iter()
                .find(|run| run.kind == TrustResourceKind::Extension && run.name == name)
                .map(|run| run.hash)
                .or_else(|| {
                    trust
                        .get_or_insert_with(|| project_trust(inputs))
                        .pinned_of(&TrustResourceKind::Extension)
                        .into_iter()
                        .find(|(pinned, _)| *pinned == name)
                        .map(|(_, hash)| hash)
                }),
        };
        if let Some(hash) = pinned {
            pending.push((source, name, shown, hash, was));
        }
    }
    decided.unread = unread_stores(&pins, trust_unreadable.as_ref(), pins_file.as_ref());
    // What stands approved where its store was read whole: what left the
    // session for good is the rest.
    if let Some(Some(pins)) = &pins {
        decided.judged.push(Source::User);
        decided.standing.extend(
            pins.list()
                .iter()
                .filter(|pin| pin.kind == TrustResourceKind::Extension)
                .map(|pin| (Source::User, pin.name.clone())),
        );
    }
    if inputs.project.is_some()
        && trust_unreadable.is_none()
        && let Some(trust) = &trust
    {
        decided.judged.push(Source::Project);
        decided.standing.extend(
            trust
                .pinned_of(&TrustResourceKind::Extension)
                .into_iter()
                .map(|(name, _)| (Source::Project, name)),
        );
        decided.standing.extend(
            inputs
                .for_the_run
                .iter()
                .filter(|run| run.kind == TrustResourceKind::Extension)
                .map(|run| (Source::Project, run.name.clone())),
        );
    }
    if standing_approved.is_empty() && pending.is_empty() && pins_unread.is_empty() {
        return decided;
    }
    let loaded = inputs.state.as_ref().map(|state| {
        let cache = RegistrationCache::new(state.registration_cache_file())
            .with_clock(Arc::clone(&inputs.clock));
        let loaded = cache.load();
        (cache, loaded)
    });
    if let Some((cache, loaded)) = &loaded
        && let Some(why) = &loaded.set_aside
    {
        let file = cache.path().to_path_buf();
        let aside = cache.set_aside_path();
        decided.told.push(sentence(&with_paths_shown(
            why,
            &[&file, &aside],
            inputs.home.as_deref(),
        )));
    }
    for (listed, snapshot) in standing_approved {
        let entry = loaded
            .as_ref()
            .and_then(|(_, loaded)| loaded.cache.get(&listed.hash));
        // Keyed by the hash of the folder, an entry is taken only when it
        // fits the extension as it was read: its name, its `api`, hooks the
        // manifest allows and the prompt file's text. A misfit is missing,
        // and the extension is registered again (the cache's fixes).
        let against = snapshot.as_deref().unwrap_or(&listed.prompt);
        match entry {
            Some(entry) if entry.fits(&listed.manifest, against).is_ok() => {
                decided.approved.push((
                    ApprovedExtension {
                        source: listed.source,
                        folder: listed.folder,
                        hash: listed.hash,
                        manifest: listed.manifest,
                        registration: entry.registration.clone(),
                    },
                    entry.prompt.clone(),
                ));
            }
            _ => {
                // To register it its bytes are needed: what the question
                // listed is read again, and registered only at the hash it
                // was approved at.
                let snapshot = snapshot.or_else(|| {
                    let read =
                        project_read.get_or_insert_with(|| in_project(&inputs.resolver, clock).0);
                    read.found
                        .iter()
                        .find(|found| found.folder == listed.folder)
                        .map(|found| Arc::clone(&found.snapshot))
                });
                match snapshot {
                    Some(snapshot) if snapshot.hash() == listed.hash => {
                        decided.missing.push((listed, snapshot));
                    }
                    _ => {
                        let said = not_loaded(
                            &listed,
                            &in_code(&listed.manifest.name),
                            Standing::Changed,
                            inputs.trust_project,
                        );
                        decided.told.push(said.clone());
                        decided.not_loaded.push((
                            listed.source,
                            listed.manifest.name.clone(),
                            said,
                        ));
                        // Changed since the question read it at the hash
                        // approved: its gates are held as any changed one's.
                        pending.push((
                            listed.source,
                            listed.manifest.name.clone(),
                            listed.shown.clone(),
                            listed.hash,
                            Unstood::Changed,
                        ));
                    }
                }
            }
        }
    }
    for (source, name, shown, hash, was) in pending {
        // What it registered at the hash approved, as the cache keeps it;
        // none when the cache does not, and the session's own then serves.
        let hooks = loaded
            .as_ref()
            .and_then(|(_, loaded)| loaded.cache.get(&hash))
            .filter(|entry| entry.name == name)
            .map(|entry| entry.registration.hooks.clone());
        // One not found has no folder to show, nor to approve from.
        let shown = (!matches!(was, Unstood::Missing)).then_some(shown.as_str());
        decided.held.push(gate(
            source,
            &name,
            shown,
            hash,
            &was,
            hooks,
            inputs.trust_project,
        ));
    }
    // The user's found while their pins cannot be read: the registration
    // cache stands in for the store, pruned as it is to the hashes pinned
    // when it was last written, so that its entries of the name are approved
    // versions: their gating hooks, together; the manifest as found only
    // where it has none. The cache keys an entry by its hash, and names it,
    // with no source: an entry of a project's of the same name counts too,
    // which errs toward a gate.
    for (name, shown, hash, events) in pins_unread {
        let approved: Option<Vec<cineris_ext::registration::HookDef>> =
            loaded.as_ref().and_then(|(_, loaded)| {
                let mut entries = loaded.cache.iter().filter(|(_, entry)| entry.name == name);
                let first = entries.next()?;
                let mut hooks = first.1.registration.hooks.clone();
                for (_, entry) in entries {
                    hooks.extend(entry.registration.hooks.iter().cloned());
                }
                Some(hooks)
            });
        decided.held.push(gate(
            Source::User,
            &name,
            Some(&shown),
            hash,
            &Unstood::PinsUnreadable(events, pins_file.clone().unwrap_or_default()),
            approved,
            inputs.trust_project,
        ));
    }
    decided.cache = loaded.map(|(cache, _)| cache);
    decided
}

/// The stores of approvals the finding could not read: the user's pins,
/// when they were read and could not be, and the trust store.
fn unread_stores(
    pins: &Option<Option<UserPins>>,
    trust: Option<&String>,
    pins_file: Option<&String>,
) -> Vec<(Source, String)> {
    let mut unread = Vec::new();
    if matches!(pins, Some(None)) {
        unread.push((Source::User, pins_file.cloned().unwrap_or_default()));
    }
    if let Some(file) = trust {
        unread.push((Source::Project, file.clone()));
    }
    unread
}

/// Where a gate's notice says what it holds, once its events are known.
const GATES_HELD: &str = "{gates held}";

/// How an extension that stands approved does not stand as it was
/// approved: what decides the gates it holds.
enum Unstood {
    /// Found with another hash than its pin's. Where what it registered at
    /// that hash is known nowhere, it holds both gates: its manifest as
    /// found is the change's, and says nothing of what was approved.
    Changed,
    /// Found and not read as it is (a link in it, a file past its cap).
    Unreadable,
    /// Not found where it was approved.
    Missing,
    /// Approved and found as approved, and not loaded (said as it was
    /// found).
    Unloaded,
    /// Approved and found as approved, and its first registration failed
    /// (said as it did): the events its manifest lets it hook.
    Unregistered(Vec<Event>),
    /// Found while the store of the user's pins, `file` as shown, cannot be
    /// read: the events its manifest lets it hook.
    PinsUnreadable(Vec<Event>, String),
    /// A project's, found while the trust store, `file` as shown, cannot be
    /// read: the events its manifest lets it hook.
    TrustUnreadable(Vec<Event>, String),
}

/// The gate an extension that stands approved and does not stand as it was
/// approved holds, from `hooks`, what it registered at the hash approved,
/// when known: its failure for the model, short, and its notice for the
/// user, in words, with the way out ([`way_out`]).
fn gate(
    source: Source,
    name: &str,
    shown: Option<&str>,
    hash: ContentHash,
    was: &Unstood,
    hooks: Option<Vec<cineris_ext::registration::HookDef>>,
    trust_project: bool,
) -> Gate {
    let both = || vec![Event::PromptSubmit, Event::ToolBefore];
    let stand = match was {
        Unstood::Missing => Stand::Gone,
        Unstood::PinsUnreadable(_, file) | Unstood::TrustUnreadable(_, file) => {
            Stand::StoreUnreadable(file)
        }
        _ => Stand::Unapproved { trust_project },
    };
    let out = way_out(source, name, shown, stand);
    // What the model is told, short, and the user, with where it was said.
    let (what, told, unknown) = match was {
        Unstood::Changed => (
            "has changed since it was approved",
            "has changed since you approved it",
            both(),
        ),
        Unstood::Unreadable => (
            "cannot be read as it was approved",
            "cannot be read as you approved it (said above)",
            both(),
        ),
        Unstood::Missing => (
            "cannot be found where it was approved",
            "cannot be found where you approved it",
            both(),
        ),
        Unstood::Unloaded => (
            "could not be loaded",
            "could not be loaded (said above)",
            both(),
        ),
        Unstood::Unregistered(events) => (
            "could not be registered",
            "could not be registered (said above)",
            events.clone(),
        ),
        Unstood::PinsUnreadable(events, _) => (
            "cannot be judged: the pin store could not be read",
            "",
            events.clone(),
        ),
        Unstood::TrustUnreadable(events, _) => (
            "cannot be judged: the trust store could not be read",
            "",
            events.clone(),
        ),
    };
    let code = in_code(name);
    let whose = match source {
        Source::User => "Your extension",
        Source::Project => "This project's extension",
    };
    let at = shown
        .map(|shown| format!(" (`{shown}`)"))
        .unwrap_or_default();
    Gate {
        source,
        name: name.to_owned(),
        hash,
        why: format!("the extension `{code}`{at} {what}, so its gate refuses it: {out}"),
        // The stores' notices are one a session, whatever the extension:
        // the same words for each.
        notice: match was {
            Unstood::Missing => format!("{whose} `{code}` {told}. {GATES_HELD}: {out}."),
            Unstood::PinsUnreadable(_, file) => format!(
                "The pin store (`{file}`) could not be read, so your extensions cannot be judged, \
                 and are not loaded: each one this session loaded keeps the gating hooks it \
                 registered, and each other one found holds a gate on each gating event its \
                 manifest lists (`tool.before`, `prompt.submit`), refusing every call or prompt it \
                 is on. For each, {}.",
                way_out(source, name, None, stand)
            ),
            Unstood::TrustUnreadable(_, file) => trust_unread_notice(file, &out),
            _ => format!("{whose} `{code}`{at} {told}. {GATES_HELD}, until you {out}."),
        },
        hooks,
        unknown,
        by_name: matches!(
            was,
            Unstood::PinsUnreadable(..) | Unstood::TrustUnreadable(..)
        ),
    }
}

/// What the user is told, once, while the trust store, `file` as shown,
/// cannot be read: the project's extensions are not loaded, and those the
/// session had loaded keep their gates; `out` is the fix.
fn trust_unread_notice(file: &str, out: &str) -> String {
    format!(
        "The trust store (`{file}`) could not be read, so this project's extensions are not \
         loaded until it is fixed; those this session had loaded keep their gating hooks on \
         their events, failing. To fix it, {out}."
    )
}

/// The extensions a pin names that were found nowhere, the user's while
/// their folder of extensions is looked in, a trusted project's while it is:
/// approved, and removed or moved (never released: only the user's word,
/// the project's trust, or the pin taken away does that).
fn not_found(
    inputs: &Inputs,
    seen: &[(Source, String)],
    pins: &mut Option<Option<UserPins>>,
    trust: &mut Option<ProjectTrust>,
    told: &mut Vec<String>,
) -> Vec<(Source, String, String, Unstood)> {
    let mut missing = Vec::new();
    if inputs.config_dir.is_some()
        && let Some(pins) = pins.get_or_insert_with(|| user_pins(inputs, told)).as_ref()
    {
        for pin in pins.list() {
            if pin.kind == TrustResourceKind::Extension
                && !seen.contains(&(Source::User, pin.name.clone()))
            {
                // Not found, it has no folder to show.
                missing.push((
                    Source::User,
                    pin.name.clone(),
                    String::new(),
                    Unstood::Missing,
                ));
            }
        }
    }
    if inputs.project.is_some() {
        let pinned = trust
            .get_or_insert_with(|| project_trust(inputs))
            .pinned_of(&TrustResourceKind::Extension);
        for (name, _) in pinned {
            if !seen.contains(&(Source::Project, name.clone())) {
                missing.push((Source::Project, name, String::new(), Unstood::Missing));
            }
        }
    }
    missing
}

/// How a project's extension stands: approved for this session by a yes
/// the store could not keep, by the hash shown; otherwise as the project's
/// entry in the store pins it (approved for the session at another hash is
/// changed).
fn project_standing(
    trust: &ProjectTrust,
    for_the_run: &[Definition],
    definition: &Definition,
) -> Standing {
    if for_the_run.contains(definition) {
        return Standing::Approved;
    }
    match trust.standing(definition) {
        Standing::Approved => Standing::Approved,
        _ if for_the_run.iter().any(|run| run.name == definition.name) => Standing::Changed,
        other => other,
    }
}

/// The user's own pins. `None` when the store cannot be read: nothing is
/// taken as pinned, and the session is told why, with the file under `~`.
fn user_pins(inputs: &Inputs, told: &mut Vec<String>) -> Option<UserPins> {
    let Some(state) = &inputs.state else {
        return Some(UserPins::default());
    };
    let file = state.pins_file();
    match PinStore::new(&file, Arc::clone(&inputs.clock)).pins() {
        Ok(pins) => Some(pins),
        Err(error) => {
            told.push(sentence(&with_paths_shown(
                &error.to_string(),
                &[&file],
                inputs.home.as_deref(),
            )));
            None
        }
    }
}

/// The project's trust as the store holds it (none while the project is
/// not trusted), and where the store is, as the user is shown it, when it
/// cannot be read: every project's extensions are then gated by their
/// manifests. A store that cannot be read trusts nothing.
fn read_trust(inputs: &Inputs) -> (ProjectTrust, Option<String>) {
    let Some(state) = &inputs.state else {
        return (ProjectTrust::untrusted(), None);
    };
    let file = state.trust_file();
    let store = TrustStore::new(&file, Arc::clone(&inputs.clock));
    // A project not trusted as the session opened has none of its
    // extensions looked at, so its store is not read.
    let Some(key) = &inputs.project else {
        return (ProjectTrust::untrusted(), None);
    };
    match store.project(key) {
        Ok(trust) => (trust, None),
        Err(_) => (
            ProjectTrust::untrusted(),
            Some(with_paths_shown(
                &file.display().to_string(),
                &[&file],
                inputs.home.as_deref(),
            )),
        ),
    }
}

/// The project's trust as the store holds it. A store that cannot be read
/// trusts nothing: the session was told so when it was put together.
fn project_trust(inputs: &Inputs) -> ProjectTrust {
    match (&inputs.state, &inputs.project) {
        (Some(state), Some(key)) => TrustStore::new(state.trust_file(), Arc::clone(&inputs.clock))
            .project(key)
            .unwrap_or_default(),
        _ => ProjectTrust::untrusted(),
    }
}

/// What the session is told of a project's extension whose name a folder
/// of the user's takes.
fn taken(found: &Listed, name: &str, mine: &[(String, String)], cut: bool) -> String {
    let folded = found.manifest.name.to_ascii_lowercase();
    let shown = &found.shown;
    match mine.iter().find(|(user, _)| *user == folded) {
        None if cut => format!(
            "This project's extension `{name}` (`{shown}`) is not used: your own folder of \
             extensions could not be listed whole (said above), so any name may be yours."
        ),
        Some((_, theirs)) => format!(
            "This project's extension `{name}` (`{shown}`) is not used: your own extension \
             (`{theirs}`) has its name, and the name is yours."
        ),
        None => format!(
            "This project's extension `{name}` (`{shown}`) is not used: a folder of your own \
             extensions has its name, though it was left out (said above), and the name is \
             yours."
        ),
    }
}

/// What the session is told of an extension it does not load, and how to
/// approve it.
fn not_loaded(found: &Listed, name: &str, standing: Standing, trust_project: bool) -> String {
    let shown = &found.shown;
    // `--trust-project` puts no question: the command approves.
    let project_remedy = if trust_project {
        "`cineris permissions trust` shows it, to approve it."
    } else {
        "The project's trust question lists it, to approve it, when a session opens."
    };
    match (found.source, standing) {
        (Source::User, Standing::Changed) => format!(
            "Your extension `{name}` (`{shown}`) has changed since you approved it, so it is not \
             loaded. `cineris ext add {shown}` shows it as it is now, to approve it."
        ),
        (Source::User, _) => format!(
            "Your extension `{name}` (`{shown}`) is not loaded: you have not approved it. \
             `cineris ext add {shown}` shows it, to approve it."
        ),
        (Source::Project, Standing::Changed) => format!(
            "This project's extension `{name}` (`{shown}`) has changed since you approved it, so \
             it is not loaded. {project_remedy}"
        ),
        (Source::Project, _) => format!(
            "This project's extension `{name}` (`{shown}`) is not loaded: you have not approved \
             it. {project_remedy}"
        ),
    }
}

impl Kept {
    /// Writes what the session registered into the cache, then prunes the
    /// cache of the hashes no pin names. Blocks.
    fn keep(self) {
        for (hash, entry) in self.written {
            // Not kept, the extension is registered again by the next
            // session: it fails closed.
            if let Err(error) = self.cache.put(&hash, entry) {
                tracing::warn!(%error, "an extension's registration was not kept");
            }
        }
        // Read after the writes, so that a pin made meanwhile is kept.
        let Some(pinned) = pinned(&self.state, &self.clock) else {
            return;
        };
        if let Err(error) = self.cache.prune(|hash| pinned.contains(hash)) {
            tracing::warn!(%error, "the registration cache was not pruned");
        }
    }
}

/// Every hash a pin of an extension names: the user's own and every
/// trusted project's, whose cache this is too. `None` when either store
/// cannot be read: nothing is pruned then.
fn pinned(state: &StateDir, clock: &SharedClock) -> Option<Vec<ContentHash>> {
    let kind = TrustResourceKind::Extension;
    let mine = PinStore::new(state.pins_file(), Arc::clone(clock))
        .pins()
        .ok()?;
    let mut pinned: Vec<ContentHash> = mine
        .list()
        .iter()
        .filter(|pin| pin.kind.as_str() == kind.as_str())
        .map(|pin| pin.hash)
        .collect();
    pinned.extend(
        TrustStore::new(state.trust_file(), Arc::clone(clock))
            .pinned_hashes(&kind)
            .ok()?,
    );
    Some(pinned)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a session's finding costs with no extension, the stores of the
    /// pins and of the trust absent and present (the full review of the
    /// script hooks, L6): the whole of it, and the two reads the gates of an
    /// extension that is gone or unjudged added (the pins, the trust), each
    /// a failed look and no parse where its store is not there. A release
    /// build's medians of 400: `cargo xtask perf -p cineris-core`.
    #[test]
    #[ignore = "a measurement: `cargo xtask perf -p cineris-core`"]
    #[expect(
        clippy::disallowed_methods,
        reason = "a measurement makes its folders and stores on its own thread"
    )]
    fn perf_the_finding_with_no_extension() {
        let dir = std::env::temp_dir().join(format!("cineris-core-finding-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(dir.join("cfg")).unwrap();
        let state = StateDir::new(dir.join("state"));
        std::fs::create_dir_all(state.root()).unwrap();
        let resolver =
            Arc::new(PathResolver::new(&work, cineris_base::PathOptions::new()).unwrap());
        let key = resolver.root().project_key();
        let clock: SharedClock = Arc::new(cineris_base::SystemClock::new());
        let inputs = Inputs {
            resolver,
            config_dir: Some(dir.join("cfg")),
            home: Some(dir.clone()),
            state: Some(state.clone()),
            project: Some(key.clone()),
            for_the_run: Vec::new(),
            trust_project: false,
            disabled: Vec::new(),
            skills: Vec::new(),
            read_skill_folders: false,
            clock: Arc::clone(&clock),
        };
        let median = |run: &dyn Fn()| {
            let mut took: Vec<f64> = (0..400)
                .map(|_| {
                    let began = std::time::Instant::now();
                    run();
                    began.elapsed().as_secs_f64() * 1e6
                })
                .collect();
            took.sort_by(f64::total_cmp);
            took[took.len() / 2]
        };
        for case in ["absent", "present"] {
            if case == "present" {
                // A pin of the user's that names no extension: one that
                // names an extension not found would hold its gates.
                PinStore::new(state.pins_file(), Arc::clone(&clock))
                    .pin(&Definition::of(TrustResourceKind::Hook, "elsewhere", ["x"]))
                    .unwrap();
                TrustStore::new(state.trust_file(), Arc::clone(&clock))
                    .trust(&key, "~/work", &[], false)
                    .unwrap();
            }
            let whole = median(&|| {
                let decided = decide(&inputs, None);
                assert!(decided.approved.is_empty() && decided.held.is_empty());
            });
            let pins = median(&|| {
                let _ = user_pins(&inputs, &mut Vec::new());
            });
            let trust = median(&|| {
                let (_, unreadable) = read_trust(&inputs);
                assert!(unreadable.is_none());
            });
            for (what, value) in [("whole", whole), ("pins_read", pins), ("trust_read", trust)] {
                cineris_testkit::numbers::report(
                    &format!("finding.no_extension.stores_{case}.{what}"),
                    value,
                    "us",
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
