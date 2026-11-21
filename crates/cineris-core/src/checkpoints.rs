//! A tool round's checkpoint, as the tools fill it
//! (`docs/architecture/07-sessions-and-context.md`, "Snapshots and undo";
//! `docs/architecture/05-tools.md`, "Checkpoints").
//!
//! Undo of a turn puts back every file the turn's tools changed. The tools
//! are in `cineris-tools` and the checkpoints in `cineris-session`, and
//! neither can see the other, so this module is where they meet:
//!
//! - **The editing tools** (`write`, `edit`) hand over what a file held
//!   before they change it and what they left, through `cineris-tools`'
//!   [`Checkpoints`] seam, which [`Rounds`] implements over the round's
//!   `RoundCheckpoint`. The bytes a tool read are the bytes that are kept:
//!   nothing is read a second time.
//! - **The shell tool** cannot say beforehand what a command will change.
//!   It looks afterwards, and puts what it saw in its result's payload,
//!   `changes`. [`RecordingShell`] wraps the tool and turns that payload
//!   into the same round's checkpoint as soon as the command is over: a
//!   file the command created did not exist; a file whose earlier content
//!   git still has is stored from that content; any other changed file is
//!   recorded with its earlier state unknown, which an undo reports and
//!   never removes or overwrites.
//!
//! Both are done by the time the loop records the round's results, so the
//! session store takes the round ([`Rounds::take`]) and writes its
//! `checkpoint` entry before the first of them: a crash between the two
//! leaves a checkpoint for calls whose results a resume then repairs, never
//! results for changes nobody recorded.
//!
//! **Hashing.** A file the shell changed is hashed whole when the round is
//! taken, to record what the round left. That is about 0.4 s a gibibyte,
//! and nothing caps it: a command that rewrites a file of several
//! gibibytes makes the round's end that much slower. Left as it is, and
//! said in the step's report.

#![allow(
    clippy::disallowed_types,
    reason = "checkpoint calls block, and are made on the blocking pool"
)]

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cineris_base::{AbsPath, ContentHash};
use cineris_protocol::Part;
use cineris_sandbox::protect::{Before as Was, Changed, Snapshot};
use cineris_session::entry::FileVersion;
use cineris_session::{Before, Body, DiskStore, Git, RoundCheckpoint, SessionError};
use cineris_tools::{
    BoxFuture, CheckpointError, Checkpoints, Effect, FileSnapshot, Json, Plan, PlanCx, Preview,
    RunCx, ShellTool, Tool, ToolError, ToolOutput, ToolResult, ToolSpec,
};

use crate::policy::EnginePolicy;
use crate::sandbox::{SessionGit, SessionSandbox, protected_changes, protected_note};

/// The checkpoint of the tool round that is running, shared by the tools
/// that fill it and the session store that writes it.
pub(crate) struct Rounds {
    store: Arc<DiskStore>,
    root: AbsPath,
    limit: u64,
    git: Arc<dyn Git>,
    round: Mutex<RoundCheckpoint>,
    /// The paths the harness's own tools have changed since the last look
    /// after a confined command, by [`AbsPath::key`]: so that a change the
    /// continuity check finds that one of them made is not reported as a
    /// change made from outside (found by the third review of step 26). It
    /// spans rounds — an `edit` in one turn and a confined command in the
    /// next — and is cleared when the next after-look is taken.
    touched: Mutex<std::collections::BTreeSet<String>>,
    /// Called when one of the harness's tools changes a file: the command
    /// observer counts it (`crate::knowledge`).
    on_change: std::sync::OnceLock<Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for Rounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rounds")
            .field("root", &self.root)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl Rounds {
    /// The rounds of a session whose workspace is at `root`, keeping the
    /// content of files up to `limit` bytes in `store`.
    pub(crate) fn new(store: Arc<DiskStore>, root: AbsPath, limit: u64, git: Arc<dyn Git>) -> Self {
        Self {
            round: Mutex::new(RoundCheckpoint::with_limit(root.clone(), limit)),
            touched: Mutex::new(std::collections::BTreeSet::new()),
            on_change: std::sync::OnceLock::new(),
            store,
            root,
            limit,
            git,
        }
    }

    fn lock(&self) -> MutexGuard<'_, RoundCheckpoint> {
        // A round is changed one whole record at a time.
        self.round.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes that one of the harness's tools changed `file`, so the
    /// continuity check does not report it as a change from outside.
    fn note_touched(&self, file: &AbsPath) {
        self.touched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(file.key().to_owned());
        if let Some(changed) = self.on_change.get() {
            changed();
        }
    }

    /// Notes a change the harness made to `file` outside a tool: the
    /// project's shared profile, written by a `promote`.
    pub(crate) fn touch(&self, file: &AbsPath) {
        self.note_touched(file);
    }

    /// Sets what is called when a tool changes a file. Once.
    pub(crate) fn on_change(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        let _ = self.on_change.set(hook);
    }

    /// The paths the harness's tools have changed since the last after-look.
    pub(crate) fn touched(&self) -> std::collections::BTreeSet<String> {
        self.touched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Forgets the touched paths: called when the look after a confined
    /// command is taken, which already holds their new state.
    pub(crate) fn clear_touched(&self) {
        self.touched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// The largest file whose content is kept.
    pub(crate) fn limit(&self) -> u64 {
        self.limit
    }

    /// Ends the round: its `checkpoint` entry, if a file changed, and a
    /// fresh round for what comes next. Blocks: it hashes the files whose
    /// result no tool recorded.
    ///
    /// # Errors
    ///
    /// When such a file cannot be read. The round is gone all the same:
    /// its files would fail the next round's entry too.
    pub(crate) fn take(&self) -> Result<Option<Body>, SessionError> {
        let round = std::mem::replace(
            &mut *self.lock(),
            RoundCheckpoint::with_limit(self.root.clone(), self.limit),
        );
        if round.is_empty() {
            return Ok(None);
        }
        round.finish()
    }

    /// Records what a shell command changed, from the `changes` of its
    /// result's payload. Blocks: it asks git for the earlier content of
    /// the files git still has.
    pub(crate) fn record_changes(&self, changes: &Json) {
        let list = |name: &str| -> Vec<&str> {
            changes
                .get(name)
                .and_then(Json::as_array)
                .map(|paths| paths.iter().filter_map(Json::as_str).collect())
                .unwrap_or_default()
        };
        let commit = changes.get("base_commit").and_then(Json::as_str);
        let restorable = list("restorable");
        let created = list("created");

        let mut seen: Vec<(AbsPath, Before)> = Vec::new();
        for path in &created {
            if let Ok(file) = self.root.join(path) {
                seen.push((file, Before::Absent));
            }
        }
        for path in list("modified").into_iter().chain(list("deleted")) {
            let Ok(file) = self.root.join(path) else {
                continue;
            };
            if created.contains(&path) {
                continue;
            }
            // Only what git can still give back, and only from the commit
            // the tool named, is known. Everything else that changed had
            // already been changed before the command, or git was not
            // asked.
            let before = match commit {
                Some(commit) if restorable.contains(&path) => self
                    .earlier_content(commit, path)
                    .map_or(Before::Unknown, Before::Version),
                _ => Before::Unknown,
            };
            seen.push((file, before));
        }
        let mut round = self.lock();
        for (file, before) in seen {
            round.seen_changed(&file, before);
        }
    }

    /// Records protected paths a command changed, with what they held
    /// before it, so that undoing the round puts them back. It comes before
    /// [`record_changes`](Self::record_changes): only the first record of
    /// a file in a round counts, and this one is exact. Blocks: it stores
    /// the content.
    ///
    /// Returns the paths undo will not put back: outside the workspace (a
    /// `sandbox.protect` entry), whose earlier state was not kept (a link,
    /// a file too large, a change of mode), or a directory.
    pub(crate) fn record_protected(&self, changed: &[Changed]) -> Vec<std::path::PathBuf> {
        let mut by_hand = Vec::new();
        for changed in changed {
            // By place: a protected path from the configuration is written
            // plainly, and the root's file-system form is extended-length on
            // Windows.
            let relative =
                cineris_base::strip_location_prefix(&changed.path, &self.root.to_fs_path());
            let Some(file) = relative
                .as_deref()
                .and_then(|relative| relative.to_str())
                .and_then(|relative| self.root.join(relative).ok())
            else {
                by_hand.push(changed.path.clone());
                continue;
            };
            let recorded = match &changed.before {
                Was::Absent => self.before(&file, None).is_ok(),
                Was::File(content) => self.before(&file, Some(content)).is_ok(),
                Was::Unknown => {
                    self.lock().seen_changed(&file, Before::Unknown);
                    false
                }
                Was::Directory => false,
            };
            // Undo leaves a path reached through a link, and cannot put
            // back a mode.
            if !recorded || changed.mode_changed || reached_through_a_link(&self.root, &file) {
                by_hand.push(changed.path.clone());
            }
        }
        by_hand
    }

    /// The content of `path` at `commit`, stored. `None` when git cannot
    /// give it, or it is not content the store can keep a record of.
    fn earlier_content(&self, commit: &str, path: &str) -> Option<FileVersion> {
        // A commit and a path are data here, never options: a name that
        // begins with a dash is not one git would take as a flag after
        // `--`, and a commit is a hash the tool read from git itself.
        if !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) || commit.is_empty() {
            return None;
        }
        // `./` makes the path relative to the workspace, wherever the
        // workspace is in its repository. `cat-file` gives the bytes as
        // they are stored, through no filter of the repository's.
        let object = format!("{commit}:./{path}");
        let limit = usize::try_from(self.limit).unwrap_or(usize::MAX);
        let output = self
            .git
            .run(
                &self.root.to_fs_path(),
                &["cat-file", "blob", &object],
                limit.saturating_add(1),
            )
            .ok()?;
        if !output.success {
            return None;
        }
        if output.truncated || output.stdout.len() as u64 > self.limit {
            // Too large to keep: there is no content to put back, and its
            // hash is not known either.
            return None;
        }
        self.store
            .put_stream(&mut output.stdout.as_slice(), self.limit)
            .ok()
    }
}

impl Checkpoints for Rounds {
    fn before(&self, file: &AbsPath, content: Option<&[u8]>) -> Result<(), CheckpointError> {
        let before = match content {
            None => Before::Absent,
            // The bytes the tool read are what is kept: no second read of
            // the file, and nothing between the two for it to become.
            Some(content) => {
                let bytes = content.len() as u64;
                let kept = bytes <= self.limit;
                let hash = if kept {
                    self.store
                        .put_blob(content)
                        .map_err(|error| CheckpointError::new(error.to_string()))?
                } else {
                    ContentHash::of(content)
                };
                Before::Version(FileVersion { hash, bytes, kept })
            }
        };
        // Only the first record of a file in a round counts.
        self.note_touched(file);
        self.lock().record_before(file, before);
        Ok(())
    }

    fn after(&self, file: &AbsPath, left: Option<&FileSnapshot>) {
        // `None`: there is no file, which is what a round records for one
        // it removed, or could not create.
        self.note_touched(file);
        self.lock().after(
            file,
            left.map(|left| FileVersion {
                hash: left.hash,
                bytes: left.size,
                kept: true,
            }),
        );
    }
}

/// Whether a directory between `root` and `file`, or `file` itself, is a
/// link now: undo will not put such a file back
/// (`cineris_session`'s `through_a_link`).
#[allow(
    clippy::disallowed_methods,
    reason = "called while the round is recorded, on the blocking pool"
)]
fn reached_through_a_link(root: &AbsPath, file: &AbsPath) -> bool {
    let top = root.to_fs_path();
    let target = file.to_fs_path();
    let Ok(below) = target.strip_prefix(&top) else {
        return false;
    };
    let mut walked = top.clone();
    below.components().any(|part| {
        walked.push(part);
        std::fs::symlink_metadata(&walked).is_ok_and(|metadata| metadata.file_type().is_symlink())
    })
}

/// Any tool of a session, held to a stop a command made: once a confined
/// command of the turn changed protected paths, no call of the turn that
/// changes anything runs, even one decided or approved before the stop
/// (found by the reviews of step 26). A call that only reads still runs.
pub(crate) struct StopsWithTurn {
    tool: Arc<dyn Tool>,
    policy: Arc<EnginePolicy>,
}

impl StopsWithTurn {
    pub(crate) fn new(tool: Arc<dyn Tool>, policy: Arc<EnginePolicy>) -> Self {
        Self { tool, policy }
    }
}

impl Tool for StopsWithTurn {
    fn spec(&self) -> &ToolSpec {
        self.tool.spec()
    }

    fn plan(&self, args: &Json, cx: &PlanCx) -> Result<Plan, ToolError> {
        self.tool.plan(args, cx)
    }

    fn run<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        out: &'a mut dyn ToolOutput,
    ) -> BoxFuture<'a, ToolResult> {
        if plan.has_side_effects()
            && let Some(reason) = self.policy.stopped_turn(cx.turn)
        {
            return Box::pin(async move {
                ToolResult::denied(format!(
                    "The call was not run: the turn was stopped. {reason}"
                ))
            });
        }
        self.tool.run(plan, cx, out)
    }

    // A question about a change to a file shows the change: the wrapped
    // tool's preview, which a wrapper must not lose.
    fn preview(&self, plan: &Plan, cx: &RunCx) -> Option<Preview> {
        self.tool.preview(plan, cx)
    }
}

/// A command's confinement is decided and the look at protected paths
/// before it taken, and it is not started: a test changes the level here,
/// which must not change how the command runs (the third security fix). An
/// awaited point ([`cineris_base::hook::await_point!`]), on the turn's task.
pub const BEFORE_THE_COMMAND_STARTS: &str = "shell: a command's confinement is decided and its look before is taken, and it has not started";

/// A command has run, and the look at protected paths after it is not
/// taken: a test changes the level here, which must not change how the
/// command is looked at. An awaited point, on the turn's task.
pub const BEFORE_THE_LOOK_AFTER: &str =
    "shell: a command has run, and the look at protected paths after it is not taken";

/// This command stopped its turn (it changed a protected path, made the
/// look stop short, or left a process the kill could not clear), and its
/// result has not gone back to the round: another call of the turn starting
/// now must find the turn stopped. An awaited point, on the turn's task;
/// reached only by a command that stopped its turn.
pub const AFTER_THE_TURN_IS_STOPPED: &str =
    "shell: a command has stopped its turn, and its result has not gone back to the round";

/// The shell tool, with what each command changed recorded in the round's
/// checkpoint when the command is over, and the protected paths looked at
/// around each confined command (`docs/architecture/06-permissions-and-sandbox.md`,
/// "Protected paths").
#[derive(Debug)]
pub(crate) struct RecordingShell {
    tool: ShellTool,
    /// The session's checkpoints. `None` for a session kept in memory.
    rounds: Option<Arc<Rounds>>,
    sandbox: Arc<SessionSandbox>,
    /// The session's own git, which reads the folder the repository runs
    /// its hooks from, to look at it with the protected paths.
    git: Arc<SessionGit>,
    policy: Arc<EnginePolicy>,
    /// The project's knowledge, whose observer is told how each command
    /// ended. `None` for a session with no state directory.
    knowledge: Option<Arc<crate::knowledge::Knowledge>>,
}

impl RecordingShell {
    pub(crate) fn new(
        tool: ShellTool,
        rounds: Option<Arc<Rounds>>,
        (sandbox, git): (Arc<SessionSandbox>, Arc<SessionGit>),
        policy: Arc<EnginePolicy>,
        knowledge: Option<Arc<crate::knowledge::Knowledge>>,
    ) -> Self {
        Self {
            tool,
            rounds,
            sandbox,
            git,
            policy,
            knowledge,
        }
    }
}

/// What the command observer is told of a finished command: how it ended,
/// by the payload of its result, and whether it changed a file or could
/// not say.
fn observed_run(
    command: String,
    cwd: Option<Option<String>>,
    payload: Option<&Json>,
    changes: Option<&Json>,
    confined: bool,
    read_only: bool,
) -> cineris_knowledge::Run {
    let flag = |key: &str| {
        payload
            .and_then(|payload| payload.get(key))
            .and_then(Json::as_bool)
            .unwrap_or(false)
    };
    let exit = payload
        .and_then(|payload| payload.get("exit_code"))
        .and_then(Json::as_i64);
    // A command that was stopped, or left something running, says nothing
    // about itself.
    let cut_short = [
        "timed_out",
        "interrupted",
        "escaped",
        "outlived",
        "background_stopped",
    ]
    .iter()
    .any(|key| flag(key));
    let ending = match exit {
        _ if cut_short => cineris_knowledge::Ending::Other,
        Some(0) => cineris_knowledge::Ending::Succeeded,
        Some(_) => cineris_knowledge::Ending::Failed,
        None => cineris_knowledge::Ending::Other,
    };
    // No account of its changes is an account that it may have changed
    // something: detection off, a call planned as read-only, a look that
    // did not finish.
    let changed = changes.is_none_or(|changes| {
        !changes
            .get("complete")
            .and_then(Json::as_bool)
            .unwrap_or(false)
            || ["modified", "created", "deleted"].iter().any(|key| {
                changes
                    .get(key)
                    .and_then(Json::as_array)
                    .is_some_and(|paths| !paths.is_empty())
            })
    });
    cineris_knowledge::Run {
        command,
        outside: cwd.is_none(),
        cwd: cwd.flatten(),
        ending,
        duration_ms: payload
            .and_then(|payload| payload.get("duration_ms"))
            .and_then(Json::as_u64)
            .unwrap_or(0),
        changed,
        confined,
        read_only,
    }
}

impl Tool for RecordingShell {
    fn spec(&self) -> &ToolSpec {
        self.tool.spec()
    }

    fn plan(&self, args: &Json, cx: &PlanCx) -> Result<Plan, ToolError> {
        self.tool.plan(args, cx)
    }

    fn preview(&self, plan: &Plan, cx: &RunCx) -> Option<Preview> {
        self.tool.preview(plan, cx)
    }

    fn run<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        out: &'a mut dyn ToolOutput,
    ) -> BoxFuture<'a, ToolResult> {
        self.run_by(plan, cx, out, By::Model)
    }
}

/// Who chose to run a command: what the command observer learns from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum By {
    /// The model, through a `shell` call (or the user, as the model's
    /// call they approved).
    Model,
    /// The harness: one of the project's checks (step 19). It confirms
    /// what the profile holds and teaches it nothing new.
    Harness,
}

/// The session's shell as the project's checks run it (step 19): every
/// confined command's looks and records, and the command observer told
/// that the run was the harness's. It is what the verifier hands to
/// `cineris_tools::execute`, so that a check's result passes the same output
/// policy as a call's.
#[derive(Debug)]
pub(crate) struct AsCheck<'a>(pub(crate) &'a RecordingShell);

impl Tool for AsCheck<'_> {
    fn spec(&self) -> &ToolSpec {
        self.0.spec()
    }

    fn plan(&self, args: &Json, cx: &PlanCx) -> Result<Plan, ToolError> {
        self.0.plan(args, cx)
    }

    fn preview(&self, plan: &Plan, cx: &RunCx) -> Option<Preview> {
        self.0.preview(plan, cx)
    }

    fn run<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        out: &'a mut dyn ToolOutput,
    ) -> BoxFuture<'a, ToolResult> {
        self.0.run_by(plan, cx, out, By::Harness)
    }
}

impl RecordingShell {
    /// Runs a planned command, as [`Tool::run`] does for the model's call,
    /// or for the harness's own check.
    pub(crate) fn run_by<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        out: &'a mut dyn ToolOutput,
        by: By,
    ) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            // A turn a command of it stopped runs no more of its calls:
            // [`StopsWithTurn`], around every tool, sees to that.
            //
            // Every confined command has its protected paths looked at
            // before and after it, one the engine marked as only reading
            // too: the program it runs may do otherwise. One the user let
            // leave the sandbox is not looked at.
            let unconfined = plan.effects.contains(&Effect::Unconfined);
            // What the command observer is told afterwards: the line, and
            // the directory it runs in, relative to the workspace (`None`
            // for a run outside it, which is counted for what it changed
            // and teaches nothing).
            let observed = self.knowledge.as_ref().and_then(|_| {
                plan.effects.iter().find_map(|effect| match effect {
                    Effect::Exec { command, cwd } => Some((
                        command.clone(),
                        cwd.relative().map(|relative| {
                            let names: Vec<&str> = relative.names().collect();
                            (!names.is_empty()).then(|| names.join("/"))
                        }),
                    )),
                    _ => None,
                })
            });
            let read_only = matches!(plan.concurrency, cineris_tools::Concurrency::ReadOnly);
            // How the command runs is decided once, here, before the look:
            // its start, the looks before and after it, what is put back
            // and what the observer is told all follow this one decision,
            // whatever the level does meanwhile. A level lowered before the
            // start and raised again before the look after can no longer
            // leave a confined command unlooked at (the third security
            // fix). A level changed after this applies from the next
            // command.
            let decided = self.sandbox.decide();
            let session_confines = decided.confines();
            let confined = session_confines && !unconfined;
            let roots = decided.write_roots();
            // Looked at only where the command is confined, with the folder
            // the repository's configuration names for its hooks read first.
            let looks = confined && self.sandbox.protected().is_some();
            let root = cx.paths.root().to_fs_path();
            // One confined start of the session at a time, from its look
            // before to its look after: two that overlapped would each take
            // the other's change for its own (the early review of the third
            // security fix).
            let one_at_a_time = if looks {
                Some(self.sandbox.one_start_at_a_time().await)
            } else {
                None
            };
            // A command that is not confined, after a confined start whose
            // look after is kept (an extension's program, which its launcher
            // confines at every level, before a command at `unrestricted`):
            // what changed since that look is found before the command runs,
            // as before a confined one, since a process the start left
            // running may have written a protected path after it (the early
            // review of the third security fix). The look is then forgotten
            // when the command has run.
            if !looks && self.sandbox.enforces() && self.sandbox.has_last_after() {
                let (sandbox, git, root) = (
                    Arc::clone(&self.sandbox),
                    Arc::clone(&self.git),
                    root.clone(),
                );
                let guard = self.sandbox.one_start_at_a_time().await;
                let now = tokio::task::spawn_blocking(move || {
                    sandbox.follow_configured(&*git, &root);
                    sandbox.protected().map(|watch| watch.take())
                })
                .await
                .ok()
                .flatten();
                drop(guard);
                if let Some(now) = &now
                    && let Some(stop) = self.continuity(now, cx).await
                {
                    return stop;
                }
            }
            let before = if looks {
                let (sandbox, git, root) = (
                    Arc::clone(&self.sandbox),
                    Arc::clone(&self.git),
                    root.clone(),
                );
                tokio::task::spawn_blocking(move || {
                    sandbox.follow_configured(&*git, &root);
                    sandbox.protected().map(|watch| watch.take())
                })
                .await
                .ok()
                .flatten()
            } else {
                None
            };
            // Before the command runs, the look before it is compared with
            // the look after the last confined command: a protected path
            // that changed in between, and that none of the harness's own
            // tools changed, was changed from outside — a process an earlier
            // command left running, or the user's own hand — and stops the
            // turn, whether or not the kill reported an escape (found by the
            // third review of step 26).
            if let Some(before) = &before
                && let Some(stop) = self.continuity(before, cx).await
            {
                return stop;
            }
            cineris_base::hook::await_point!(BEFORE_THE_COMMAND_STARTS);
            // The command itself runs on this thread, the core's: the
            // kernel's death signal is tied to the thread that started it.
            // It starts through the launcher decided above, not the one the
            // level gives when it gets there.
            let launcher: Arc<dyn cineris_tools::process::Launcher> = Arc::new(decided);
            let mut result = self.tool.run_through(plan, cx, out, launcher).await;
            cineris_base::hook::await_point!(BEFORE_THE_LOOK_AFTER);
            // What the command changed among the protected paths is put
            // back at once, from the look before's own record: a confined
            // command never keeps a change there (the third security fix).
            // The folder for hooks is read again first: a command may have
            // named another through a file the configuration includes.
            let found = match before {
                Some(before) => {
                    let (sandbox, git) = (Arc::clone(&self.sandbox), Arc::clone(&self.git));
                    tokio::task::spawn_blocking(move || {
                        sandbox.follow_configured(&*git, &root);
                        let keep = sandbox.kept_in().map(std::path::Path::to_path_buf);
                        sandbox.protected().map(|watch| {
                            let found = protected_changes(&watch, &before, &roots, keep.as_deref());
                            sandbox.remember_kept_nowhere(&before, &found);
                            found
                        })
                    })
                    .await
                    .ok()
                    .flatten()
                }
                None => None,
            };
            drop(one_at_a_time);
            // The look after this command is kept for the next command's
            // and the close's continuity check, and the harness-touched set
            // is forgotten: the after-look already holds their new state.
            if let Some(found) = &found {
                self.sandbox.remember_after(found.after.clone());
                if let Some(rounds) = &self.rounds {
                    rounds.clear_touched();
                }
            } else if !looks && self.sandbox.enforces() {
                // A command that ran as it is (one the user let leave the
                // sandbox, or any at `unrestricted`) is not looked at, so its
                // own changes to a protected path are not recorded. The next
                // confined start's continuity check must not compare across
                // it and report those as a change from outside, so the
                // baseline is dropped and the next start begins fresh (found
                // by the independent review of the fixes of step 26; at
                // `unrestricted` too since the third security fix).
                self.sandbox.forget_after();
                if let Some(rounds) = &self.rounds {
                    rounds.clear_touched();
                }
            }
            let changes = result
                .payload()
                .and_then(|payload| payload.get("changes"))
                .filter(|changes| !changes.is_null())
                .cloned();
            if let Some(changes) = &changes {
                note_changed(changes, cx);
            }
            if let (Some(knowledge), Some((command, cwd))) = (&self.knowledge, observed) {
                let run = observed_run(
                    command,
                    cwd,
                    result.payload(),
                    changes.as_ref(),
                    confined,
                    read_only,
                );
                match by {
                    By::Model => knowledge.observed(&run),
                    By::Harness => knowledge.observed_check(&run),
                }
            }
            if let Some(rounds) = &self.rounds {
                let rounds = Arc::clone(rounds);
                // What is left of the change, which was not put back, is
                // recorded for undo where it can be; what was put back is
                // as it was, and has nothing to undo.
                let left = found.as_ref().map(|found| found.left.clone());
                // Not cancelled: the command ran, and what it changed is
                // recorded whether or not the turn goes on.
                let recorded = tokio::task::spawn_blocking(move || {
                    if let Some(left) = left {
                        let _ = rounds.record_protected(&left);
                    }
                    if let Some(changes) = changes {
                        rounds.record_changes(&changes);
                    }
                })
                .await;
                if recorded.is_err() {
                    tracing::warn!("what a shell command changed could not be recorded");
                }
            }
            // Whether this command stopped the turn, for the point below.
            let mut stopped = false;
            if let Some(found) = found {
                let root = cx.paths.root().to_fs_path();
                if let Some(note) = protected_note(&found, &root) {
                    result = result.with_part(Part::text(note.clone()));
                    // A change, a look the command itself made stop short,
                    // or a folder it made unlistable stops the turn; a
                    // workspace already past the bound only warns. The
                    // result is marked as the stop, so that the round ends
                    // at this call, even when it was the last of its
                    // response.
                    if found.stops_the_turn() {
                        self.policy.stop_turn(cx.turn, note.clone());
                        result = result.stopping_turn(note);
                        stopped = true;
                    }
                }
            }
            // A command that left a process the kill could not clear may
            // still be writing a protected path after the look: the turn
            // stops, so that nothing the model does next builds on a look
            // that may already be stale (found by the third review of
            // step 26).
            let escaped = result
                .payload()
                .and_then(|payload| payload.get("escaped"))
                .and_then(Json::as_bool)
                .unwrap_or(false);
            if escaped && session_confines {
                let note = "[A process the command started kept forking and could not all be \
                            killed. It may still be running inside the sandbox and may change a \
                            protected path after this check — a git hook (`.git/hooks`), git's \
                            configuration (`.git/config`), a CI file (`.github/workflows`), \
                            `.cineris` or another watched name. The turn stops here, and the \
                            protected paths are looked at again before the next command and at \
                            the session's close.]";
                self.policy.stop_turn(cx.turn, note.to_owned());
                result = result.stopping_turn(note);
                stopped = true;
            }
            if stopped {
                cineris_base::hook::await_point!(AFTER_THE_TURN_IS_STOPPED);
            }
            result
        })
    }
}

/// Tells the table of seen files which files a command changed, for what a
/// compaction carries across (`docs/architecture/07-sessions-and-context.md`,
/// "Compaction").
pub(crate) fn note_changed(changes: &Json, cx: &RunCx) {
    let root = cx.paths.root();
    for list in ["created", "modified", "deleted"] {
        let paths = changes.get(list).and_then(Json::as_array);
        for path in paths.into_iter().flatten().filter_map(Json::as_str) {
            if let Ok(file) = root.join(path) {
                cx.files.note_changed(&file);
            }
        }
    }
}

impl RecordingShell {
    /// Compares the look before this command with the look after the last
    /// confined command, and if a protected path changed in between that
    /// none of the harness's own tools changed, records it, stops the turn,
    /// and returns the result that says so — so this command does not run on
    /// a workspace something outside the harness has changed. `None` when
    /// nothing changed from outside.
    async fn continuity(&self, before: &Snapshot, cx: &RunCx) -> Option<ToolResult> {
        let note = changed_since_last_look(
            &self.sandbox,
            self.rounds.as_ref(),
            &self.policy,
            before,
            cx.turn,
            &cx.paths.root().to_fs_path(),
        )
        .await?;
        Some(ToolResult::denied(format!("The command was not run. {note}")).stopping_turn(note))
    }
}

/// Compares `before`, the look before a confined start, with the look after
/// the last one, and if a protected path changed in between that none of
/// the harness's own tools changed (a process an earlier command left
/// running, or the user's own hand), records it in the round's checkpoint,
/// stops turn `turn`, and returns what the model and the user are told.
/// `None` when nothing changed from outside. The shell's commands and an
/// extension's programs (step 28) start only after it.
pub(crate) async fn changed_since_last_look(
    sandbox: &SessionSandbox,
    rounds: Option<&Arc<Rounds>>,
    policy: &EnginePolicy,
    before: &Snapshot,
    turn: u32,
    root: &std::path::Path,
) -> Option<String> {
    let changed = sandbox.continuity(before)?;
    // What the harness's own tools changed since the last after-look is
    // not a change from outside.
    let touched = rounds.map(|rounds| rounds.touched());
    let changed: Vec<Changed> = changed
        .into_iter()
        .filter(|one| {
            touched.as_ref().is_none_or(|touched| {
                AbsPath::from_os(&one.path)
                    .ok()
                    .is_none_or(|path| !touched.contains(path.key()))
            })
        })
        .collect();
    if changed.is_empty() {
        return None;
    }
    let by_hand = match rounds {
        Some(rounds) => {
            let rounds = Arc::clone(rounds);
            let changed = changed.clone();
            tokio::task::spawn_blocking(move || rounds.record_protected(&changed))
                .await
                .unwrap_or_default()
        }
        None => changed.iter().map(|one| one.path.clone()).collect(),
    };
    let note = continuity_note(&changed, root, &by_hand);
    policy.stop_turn(turn, note.clone());
    Some(note)
}

/// What the model and the user are told when a protected path changed
/// between two confined commands, from outside the harness.
fn continuity_note(
    changed: &[Changed],
    root: &std::path::Path,
    by_hand: &[std::path::PathBuf],
) -> String {
    let shown: Vec<String> = changed
        .iter()
        .map(|one| {
            let path = one.path.strip_prefix(root).unwrap_or(&one.path);
            // A name from the workspace, which the model reads.
            format!(
                "`{}` ({})",
                cineris_base::visible(&path.to_string_lossy()),
                one.change.as_str()
            )
        })
        .collect();
    let undo = if by_hand.len() == changed.len() {
        "Undoing this turn does not put them back: what they were is not kept, so they must be \
         put back by hand."
    } else {
        "Undoing this turn puts them back."
    };
    format!(
        "[A protected path changed since the last command, and none of the harness's own tools \
         changed it: a process an earlier command left running inside the sandbox, or a change \
         made by hand between turns. These decide what runs later outside the sandbox: {}. The \
         turn stops here, so that the user can look. {undo}]",
        shown.join(", ")
    )
}

#[cfg(test)]
mod tests;
