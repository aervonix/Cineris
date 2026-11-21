//! The sandbox as a session sees it (`docs/architecture/06-permissions-and-sandbox.md`,
//! "The sandbox"): whether it enforces, the launcher the shell tool starts
//! commands through, and the session's private temporary directory.
//!
//! - **The state** comes from what the kernel offers
//!   ([`cineris_sandbox::support`]), whether the binary gave the core a
//!   launcher to start ([`CoreOptions::launcher`](crate::core::CoreOptions::launcher)),
//!   and `sandbox.enabled`. It is what lets `autonomous` start, what
//!   `SessionInfo::sandbox` and the `initialize` result report, and what an
//!   approval's `outside_sandbox` says.
//! - **The launcher** confines every command at every level but
//!   `unrestricted`, where commands run as they are. It is given the
//!   lease's write roots and network setting, the session's temporary
//!   directory, and the Landlock version detection found.
//! - **The temporary directory** is made, open to the user alone, when the
//!   session starts, given to every confined command as `TMPDIR`, and
//!   removed when the session ends.
//! - **The harness's own git** runs inside the sandbox wherever the
//!   session's sandbox enforces, at every level, `unrestricted` included,
//!   with no write root and no network ([`SessionGit`]): a confined command,
//!   or an extension's program, may write the repository's configuration,
//!   which can name a program for git to run, and a look taken later, at
//!   any level, would run it (found by the third review of step 26; at
//!   every level since the third security fix, which folded in the rule
//!   that followed an extension's program).
//! - **A command's confinement is decided once** ([`SessionSandbox::decide`]),
//!   when the look before it is taken: the start and both looks follow that
//!   one decision, whatever the level does meanwhile (the third security
//!   fix).

#![allow(
    clippy::disallowed_methods,
    reason = "the temporary directory is made while a session is assembled and removed when it \
              ends, both on the blocking pool"
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, PoisonError};

use cineris_base::child_env::Withheld;
use cineris_base::git::{Confinement, Started};

use cineris_policy::{NoSandbox, SandboxState};
use cineris_protocol::SessionId;
use cineris_protocol::lease::{Level, NetworkAccess, SandboxPolicy};
use cineris_protocol::session::SandboxStatus;
use cineris_sandbox::launch::Launcher as SandboxLauncher;
use cineris_sandbox::policy::{Network, Policy, Reach};
use cineris_sandbox::protect::{Changed, Limits, Snapshot, Watch};
use cineris_sandbox::support::{Assessment, Support, Unconfined};
use cineris_tools::process::{Argument, CommandLine, Direct, Exit, Launch, LaunchCx, Launcher};

/// Whether a path the configuration names is watched: a file, or a name not
/// there yet; never a folder, which would watch whatever is in it, nor the
/// workspace or a folder above it (the full review of the third security
/// fix).
#[allow(
    clippy::disallowed_methods,
    reason = "part of a look at the protected paths, on the blocking pool"
)]
fn watched_by_name(path: &Path, workspace: &Path) -> bool {
    !std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
        && !cineris_base::location_within(workspace, path)
}

/// `path` where it really is: its folder with every link on the way
/// resolved, and its own name; as it is where its folder is not there.
#[allow(
    clippy::disallowed_methods,
    reason = "part of a look at the protected paths, on the blocking pool"
)]
fn where_it_is(path: &Path) -> PathBuf {
    match (path.parent().map(std::fs::canonicalize), path.file_name()) {
        (Some(Ok(folder)), Some(name)) => folder.join(name),
        _ => path.to_path_buf(),
    }
}

/// How long the read of what the repository's configuration names may take
/// ([`SessionSandbox::follow_configured`]): a configuration that includes a
/// file that stalls a read (a FIFO) gives up there, and nothing more is
/// watched.
const CONFIGURED_CAP: std::time::Duration = std::time::Duration::from_millis(500);

/// What the core knows of the sandbox, once for its life.
#[derive(Debug, Clone)]
pub(crate) struct CoreSandbox {
    support: Support,
    program: Option<(PathBuf, Vec<OsString>)>,
}

impl CoreSandbox {
    /// From what the system offers, or what a test says it does, and the
    /// program the binary gave to start as the launcher.
    pub(crate) fn new(support: Support, program: Option<(PathBuf, Vec<OsString>)>) -> Self {
        Self { support, program }
    }

    /// What a session with `sandbox.enabled` set as given reports, and the
    /// Landlock version and the gaps it enforces with.
    pub(crate) fn state(&self, enabled: bool) -> (SandboxState, Option<(u8, Vec<&'static str>)>) {
        if !enabled {
            return (SandboxState::NotEnforcing(NoSandbox::TurnedOff), None);
        }
        match self.support.assess() {
            // Without a launcher to start, nothing can be confined.
            Assessment::Enforcing { .. } if self.program.is_none() => {
                (SandboxState::NotEnforcing(NoSandbox::NotBuilt), None)
            }
            Assessment::Enforcing { landlock, gaps } => {
                (SandboxState::Enforcing, Some((landlock, gaps)))
            }
            Assessment::NotEnforcing(why) => (
                SandboxState::NotEnforcing(match why {
                    Unconfined::NotBuilt => NoSandbox::NotBuilt,
                    Unconfined::UnsupportedSystem => NoSandbox::UnsupportedSystem,
                    Unconfined::NoLandlock => NoSandbox::NoLandlock,
                    Unconfined::LandlockTooOld(version) => NoSandbox::LandlockTooOld(version),
                }),
                None,
            ),
        }
    }

    /// The program started as the launcher, as the binary gave it.
    pub(crate) fn program(&self) -> Option<&Path> {
        self.program.as_ref().map(|(program, _)| program.as_path())
    }

    /// What the `initialize` result reports: whether a session with the
    /// sandbox enabled would be confined.
    pub(crate) fn status(&self) -> SandboxStatus {
        status_of(self.state(true).0)
    }
}

/// The protocol's word for a state.
pub(crate) fn status_of(state: SandboxState) -> SandboxStatus {
    match state {
        SandboxState::Enforcing => SandboxStatus::Enforcing,
        SandboxState::NotEnforcing(_) => SandboxStatus::Off,
    }
}

/// Where a session's private temporary directory is.
pub(crate) fn temp_path(session: &SessionId) -> PathBuf {
    std::env::temp_dir().join(format!("cineris-{session}"))
}

/// A session's private temporary directory: where it is, as the lease names
/// it, and whether it was made as the session's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Temp {
    pub(crate) path: PathBuf,
    pub(crate) made: bool,
}

/// The sandbox of one session: the launcher handed to its shell tool.
#[derive(Debug)]
pub(crate) struct SessionSandbox {
    /// The launcher, when the sandbox enforces. A reload of the
    /// configuration that changes the grant's write roots or network makes
    /// another, for the commands started after it (step 24).
    launcher: std::sync::RwLock<Option<SandboxLauncher>>,
    /// What the launcher is made with, to make it again: the program and
    /// its prefix, and the Landlock version.
    made_with: Option<(PathBuf, Vec<OsString>, u8)>,
    /// The launcher the harness's own git is started through, when the
    /// sandbox enforces: the session's reads, and no write root, no
    /// network and no temporary directory, so that nothing git starts can
    /// change anything.
    reader: Option<SandboxLauncher>,
    /// Whether commands are confined now: the sandbox enforces and the
    /// level is not `unrestricted`.
    confine: AtomicBool,
    /// The private temporary directory.
    temp: Temp,
    /// The protected paths, looked at before and after each confined
    /// command, when the sandbox enforces.
    watch: Option<Watch>,
    /// What the repository's configuration has named in this session for git
    /// to run or read (the folder for hooks, the files it includes, the
    /// programs its keys name), where it lies inside a write root: watched
    /// with the protected paths, read again before and after each confined
    /// start ([`Self::follow_configured`]; the third security fix).
    configured: std::sync::RwLock<Vec<PathBuf>>,
    /// The session's own folder where the put-back keeps what it writes
    /// over or removes (in its session directory, with its other state):
    /// `None` for a session kept in memory, which then writes over nothing.
    keep: OnceLock<PathBuf>,
    /// What a confined start changed and the put-back left, to be put back
    /// by hand, with the look after it: told again at each turn's start
    /// while it is still as the start left it (the maintainers' settlement
    /// after the full review of the third security fix, for a session kept
    /// in memory, and after its narrow check, for everything named by
    /// hand).
    by_hand: std::sync::Mutex<Vec<ByHand>>,
    /// The workspace, as the session's paths give it: what a name told to
    /// the user is relative to.
    workspace: PathBuf,
    /// Held from the look before a confined start to the look after it, by
    /// every confined start of the session: two that overlap would each take
    /// the other's change for its own, and one could put back what the
    /// other planted (found by the early review of the third security fix).
    starts: tokio::sync::Mutex<()>,
    /// The look taken after the last confined command, kept so that the
    /// next confined command, and the session's close, can tell whether a
    /// protected path changed in between — by a process a command left
    /// running that the kill could not clear, or by the user's own hand —
    /// without depending on the kill reporting an escape (found by the
    /// third review of step 26).
    last_after: std::sync::Mutex<Option<Snapshot>>,
}

impl SessionSandbox {
    /// The sandbox of a session in `workspace`, in `state`, at `level`,
    /// whose lease grants `grant`, with its temporary directory, looking at
    /// its protected paths within `limits`.
    #[expect(
        clippy::too_many_arguments,
        reason = "each is one thing a session's sandbox is made from"
    )]
    pub(crate) fn new(
        core: &CoreSandbox,
        state: SandboxState,
        landlock: Option<u8>,
        level: &Level,
        grant: &SandboxPolicy,
        temp: Temp,
        workspace: &Path,
        limits: Limits,
    ) -> Self {
        let mut made_with = None;
        let (launcher, reader) = match (state, landlock, &core.program) {
            (SandboxState::Enforcing, Some(landlock), Some((program, prefix))) => {
                made_with = Some((program.clone(), prefix.clone(), landlock));
                let policy = policy_of(grant, &temp, landlock);
                let reading = Policy {
                    read: policy.read.clone(),
                    write: Vec::new(),
                    network: Network::Off,
                    temp: None,
                    landlock,
                };
                (
                    Some(SandboxLauncher::new(
                        program.clone(),
                        prefix.clone(),
                        policy,
                    )),
                    Some(SandboxLauncher::new(
                        program.clone(),
                        prefix.clone(),
                        reading,
                    )),
                )
            }
            _ => (None, None),
        };
        let watch = launcher.is_some().then(|| {
            Watch::new(
                workspace,
                grant.protect.roots.iter().map(PathBuf::from).collect(),
            )
            .with_limits(limits)
        });
        let sandbox = Self {
            launcher: std::sync::RwLock::new(launcher),
            made_with,
            reader,
            confine: AtomicBool::new(false),
            temp,
            watch,
            configured: std::sync::RwLock::new(Vec::new()),
            keep: OnceLock::new(),
            by_hand: std::sync::Mutex::new(Vec::new()),
            workspace: workspace.to_path_buf(),
            starts: tokio::sync::Mutex::new(()),
            last_after: std::sync::Mutex::new(None),
        };
        sandbox.set_level(level);
        sandbox
    }

    /// Remembers the look taken after a confined command, so that the next
    /// confined command, and the close, can tell what changed in between.
    pub(crate) fn remember_after(&self, after: Snapshot) {
        *self
            .last_after
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(after);
    }

    /// Keeps what the put-back writes over or removes in `dir`, the
    /// session's own: set once, when the session's directory is known.
    pub(crate) fn keep_copies_in(&self, dir: PathBuf) {
        let _ = self.keep.set(dir);
    }

    /// Where the put-back keeps what it writes over or removes.
    pub(crate) fn kept_in(&self) -> Option<&Path> {
        self.keep.get().map(PathBuf::as_path)
    }

    /// The workspace the session's sandbox looks at.
    pub(crate) fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Remembers what `found` left to be put back by hand, with the look
    /// after the start, which it is compared with.
    pub(crate) fn remember_by_hand(&self, found: &Found) {
        if found.left.is_empty() {
            return;
        }
        let paths = found
            .left
            .iter()
            .map(|one| (one.path.clone(), found.kept_nowhere.contains(&one.path)))
            .collect();
        self.by_hand
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ByHand {
                after: found.after.clone(),
                paths,
            });
    }

    /// What a confined start changed and the put-back left to be put back by
    /// hand, and that is still as the start left it: a look now, taken only
    /// while there is something to ask about. Each path comes with whether
    /// it was left because the session keeps nothing on disk. What changed
    /// since (put back by hand, or changed again) is forgotten. It blocks.
    #[allow(
        clippy::disallowed_methods,
        reason = "a look at the protected paths, on the blocking pool"
    )]
    pub(crate) fn still_by_hand(&self) -> Vec<(PathBuf, bool)> {
        let mut remembered = self.by_hand.lock().unwrap_or_else(PoisonError::into_inner);
        if remembered.is_empty() {
            return Vec::new();
        }
        let Some(watch) = self.protected() else {
            return Vec::new();
        };
        let now = watch.take();
        let mut still: Vec<(PathBuf, bool)> = Vec::new();
        remembered.retain_mut(|left| {
            let since = left.after.compare(&now);
            left.paths
                .retain(|(path, _)| !since.iter().any(|one| one.path == *path));
            for (path, nowhere) in &left.paths {
                if !still.iter().any(|(known, _)| known == path) {
                    still.push((path.clone(), *nowhere));
                }
            }
            !left.paths.is_empty()
        });
        still
    }

    /// Waits until no other confined start of the session is between its
    /// look before and its look after, and holds that until the guard goes.
    pub(crate) async fn one_start_at_a_time(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.starts.lock().await
    }

    /// Whether a look after a confined start is kept to compare with.
    pub(crate) fn has_last_after(&self) -> bool {
        self.last_after
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Forgets the look after the last confined command, so the next one
    /// does not compare across what happened in between. Used when a command
    /// ran unconfined (the user let it leave the sandbox): its own approved
    /// changes to a protected path must not look like a change from outside
    /// to the next command (found by the independent review of the step 26
    /// fixes).
    pub(crate) fn forget_after(&self) {
        *self
            .last_after
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// What changed among the protected paths between the look after the
    /// last confined command and `now` — whatever made it, which the caller
    /// tells apart. `None` when there is no earlier look to compare with.
    pub(crate) fn continuity(&self, now: &Snapshot) -> Option<Vec<Changed>> {
        let last = self
            .last_after
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        last.as_ref().map(|last| last.compare(now))
    }

    /// Looks at the protected paths once more and says what changed since
    /// the last confined command: for the session's close. Blocks. `None`
    /// when there is no earlier look, or nothing changed.
    pub(crate) fn continuity_at_close(&self) -> Option<Vec<Changed>> {
        let watch = self.protected()?;
        let now = watch.take();
        let changed = self.continuity(&now)?;
        (!changed.is_empty()).then_some(changed)
    }

    /// Whether the session's sandbox enforces at all: its launcher was
    /// made, whatever the level.
    pub(crate) fn enforces(&self) -> bool {
        self.made_with.is_some()
    }

    /// Whether a command that runs now is confined.
    pub(crate) fn confines(&self) -> bool {
        self.confine.load(Ordering::Acquire)
    }

    /// `avoid` with the write roots the launcher's policy gives now, which
    /// a reload may have widened since `avoid` was made: where a program
    /// the harness looks up by name is never taken from, as a confined
    /// command could have planted it there (the early review of step 28).
    pub(crate) fn avoiding_roots_now(&self, avoid: &[PathBuf]) -> Vec<PathBuf> {
        let mut all = avoid.to_vec();
        let now = self
            .launcher
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|launcher| launcher.policy().write.clone())
            .unwrap_or_default();
        for root in now {
            if !all.contains(&root) {
                all.push(root);
            }
        }
        all
    }

    /// The launcher as it is now, while commands are confined.
    fn launcher(&self) -> Option<SandboxLauncher> {
        if !self.confines() {
            return None;
        }
        self.launcher
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The launcher's policy for `grant` (a reload changed its write roots
    /// or network): `None` where the sandbox does not enforce. Blocks: each
    /// root is found where it really is.
    pub(crate) fn policy_for(&self, grant: &SandboxPolicy) -> Option<Policy> {
        let (_, _, landlock) = self.made_with.as_ref()?;
        Some(policy_of(grant, &self.temp, *landlock))
    }

    /// Makes the launcher again with `policy` ([`Self::policy_for`]), for
    /// the commands started from now on; one running keeps its own. What it
    /// watches of the protected paths is not changed: `sandbox.protect`
    /// takes effect in a new session.
    pub(crate) fn set_policy(&self, policy: Policy) {
        let Some((program, prefix, _)) = &self.made_with else {
            return;
        };
        *self
            .launcher
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(SandboxLauncher::new(
            program.clone(),
            prefix.clone(),
            policy,
        ));
    }

    /// Follows a change of the session's level: nothing is confined at
    /// `unrestricted`.
    pub(crate) fn set_level(&self, level: &Level) {
        let confine = self.made_with.is_some() && *level != Level::Unrestricted;
        self.confine.store(confine, Ordering::Release);
    }

    /// How a command started from here runs, decided now, once: the
    /// launcher as it is now while commands are confined, or none. Its
    /// start and the looks before and after it all follow this one value,
    /// so that a level changed in between (lowered before the start and
    /// raised again before the look after) cannot leave a confined command
    /// unlooked at, nor have a command looked at as confined that ran as
    /// it is (the third security fix; `outstanding`'s
    /// `a-command-started-confined-between-two-changes-of-level`). A level
    /// changed after it applies from the next command.
    pub(crate) fn decide(&self) -> Decided {
        Decided {
            launcher: self.launcher(),
        }
    }

    /// The protected paths, wherever the sandbox enforces, whatever the
    /// level: for a caller that decided its start is confined (a command
    /// [`decide`](Self::decide)d so, an extension's program its own launcher
    /// confines at every level). With them, what the repository's
    /// configuration names for git to run or read, as
    /// [`Self::follow_configured`] last read it.
    pub(crate) fn protected(&self) -> Option<Watch> {
        let configured = self
            .configured
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        self.watch
            .as_ref()
            .map(|watch| watch.clone().with_more(configured))
    }

    /// Reads again what the repository the workspace `root` is in has git
    /// run or read (`cineris_tools::shell::changes::configured_paths`: the
    /// folder for its hooks, every configuration file it includes, every
    /// path a key that names a program gives), with the session's own git
    /// `git`, and keeps what lies inside one of the session's write roots
    /// to watch: a hook, a program or an included file a confined start
    /// writes there runs, or is read, at the next `git` outside the
    /// sandbox, though none of them is on the protected list (step 28's
    /// review of protected writes; the third security fix). One git command,
    /// before each confined start's look and again before the look after
    /// it, so that what a start names through a file the configuration
    /// includes is watched by the look after the same start. What was once
    /// watched stays watched for the session: a look that dropped a path
    /// would take what is at it for gone, and watching one more only
    /// narrows. Nothing where the sandbox does not enforce, where nothing is
    /// looked at. It blocks: it may start git.
    pub(crate) fn follow_configured(&self, git: &dyn Confinement, root: &Path) {
        if self.watch.is_none() {
            return;
        }
        let roots: Vec<PathBuf> = self
            .launcher
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|launcher| launcher.policy().write.clone())
            .unwrap_or_default();
        let kept: Vec<PathBuf> =
            cineris_tools::shell::changes::configured_paths(git, root, CONFIGURED_CAP)
                .into_iter()
                // As named, so that a link on its way is the look's to see
                // (the narrow check of the third security fix), and where
                // it really is, so that a place inside a write root reached
                // by another name is watched too.
                .flat_map(|path| {
                    let real = where_it_is(&path);
                    if real == path {
                        vec![path]
                    } else {
                        vec![path, real]
                    }
                })
                .filter(|path| watched_by_name(path, root))
                .filter(|path| {
                    roots
                        .iter()
                        .any(|root| cineris_base::location_within(path, root))
                })
                .collect();
        let mut configured = self
            .configured
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        for path in kept {
            if !configured.contains(&path) {
                configured.push(path);
            }
        }
    }

    /// The launcher an extension's program is started through
    /// (`docs/steps/28/answered.md`, "The processes"), wherever the sandbox
    /// enforces and at every level, `unrestricted` included: the session's
    /// reads; as write roots `write_roots` alone, each resolved, and the
    /// session's private temporary directory, which it is given as
    /// `TMPDIR`; and the network only with `network`. Not the workspace,
    /// nor any root of the session's grant: a program writes no wider than
    /// its manifest's directories. `None` where the sandbox does not
    /// enforce, where a program is started as it is. Blocks: each root is
    /// found where it really is.
    #[cfg(feature = "scripts")]
    pub(crate) fn extension_launcher(
        &self,
        write_roots: &[PathBuf],
        network: bool,
    ) -> Option<SandboxLauncher> {
        let (program, prefix, landlock) = self.made_with.as_ref()?;
        let read = self
            .launcher
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()?
            .policy()
            .read
            .clone();
        // As the caller gives them: the workspace's real path joined with
        // each root's names, never resolved here, so that the launcher's
        // open refuses a link on the way to one instead of granting where
        // it leads.
        let mut write: Vec<PathBuf> = write_roots.to_vec();
        // The session's own, made with no link on the way (`make_temp`); one
        // that was not made is not written to, as for the shell's commands.
        let temp = self.temp.made.then(|| resolve_root(&self.temp.path));
        write.extend(temp.clone());
        Some(SandboxLauncher::new(
            program.clone(),
            prefix.clone(),
            Policy {
                read,
                write,
                network: if network { Network::Full } else { Network::Off },
                temp,
                landlock: *landlock,
            },
        ))
    }

    /// What to start in place of the harness's own `git` with `arguments`:
    /// the reading launcher wherever the sandbox enforces, at every level;
    /// git as it is where it does not, where nothing was confined to write
    /// what it reads.
    fn start_git(&self, git: &Path, arguments: Vec<OsString>) -> Started {
        match &self.reader {
            Some(reader) => reading(reader, git, arguments),
            None => Started {
                program: git.to_owned(),
                arguments,
                stdin: None,
            },
        }
    }

    /// Removes the temporary directory and everything in it, if the
    /// session made it. It blocks.
    pub(crate) fn remove_temp(&self) {
        if self.temp.made {
            remove_temp(&self.temp.path);
        }
    }
}

/// How one command runs, decided once ([`SessionSandbox::decide`]): through
/// the session's launcher as it was at the decision, or as it is.
#[derive(Debug, Clone)]
pub(crate) struct Decided {
    launcher: Option<SandboxLauncher>,
}

impl Decided {
    /// Whether the command is confined: the session confined commands at
    /// the decision. A call the user let leave the sandbox runs as it is
    /// all the same.
    pub(crate) fn confines(&self) -> bool {
        self.launcher.is_some()
    }

    /// The roots a command started now may write: the decided launcher's.
    pub(crate) fn write_roots(&self) -> Vec<PathBuf> {
        self.launcher
            .as_ref()
            .map(|launcher| launcher.policy().write.clone())
            .unwrap_or_default()
    }
}

/// A launcher read again at each start, for a caller that does not decide
/// first: the shell tool's own, which [`RecordingShell`](crate::checkpoints::RecordingShell)
/// passes over with its decision.
impl Launcher for SessionSandbox {
    fn launch(&self, command: CommandLine, cx: &LaunchCx<'_>) -> Result<Launch, String> {
        self.decide().launch(command, cx)
    }

    fn explain(&self, exit: Option<Exit>, output: &str) -> Option<String> {
        self.decide().explain(exit, output)
    }
}

impl Launcher for Decided {
    fn launch(&self, command: CommandLine, cx: &LaunchCx<'_>) -> Result<Launch, String> {
        let Some(launcher) = &self.launcher else {
            return Direct.launch(command, cx);
        };
        let arguments: Vec<OsString> = command
            .arguments
            .iter()
            .map(|argument| match argument {
                Argument::Exact(text) | Argument::Verbatim(text) => OsString::from(text),
            })
            .collect();
        let wrapped = launcher.wrap(command.program.as_os_str(), &arguments);
        let mut line = CommandLine::new(wrapped.program);
        line.arguments = wrapped
            .arguments
            .into_iter()
            .map(|argument| {
                argument
                    .into_string()
                    .map(Argument::Exact)
                    .map_err(|_| "a command line that is not text cannot be confined".to_owned())
            })
            .collect::<Result<_, _>>()?;
        Ok(Launch {
            command: line,
            stdin: Some(wrapped.stdin),
        })
    }

    fn explain(&self, exit: Option<Exit>, output: &str) -> Option<String> {
        let launcher = self.launcher.as_ref()?;
        let code = match exit {
            Some(Exit::Code(code)) => Some(code),
            _ => None,
        };
        launcher.explain(code, output)
    }
}

/// Where a session's own git runs, and with what: inside its sandbox,
/// reading only, wherever the sandbox enforces, at every level; never a git
/// inside the workspace, or inside a write root a confined command could
/// have planted one in; and without the providers' key variables.
///
/// **At every level, `unrestricted` included** (the third security fix):
/// what a repository's configuration makes git run (a filter, a submodule's
/// own filter, a program no list foresaw) is planted by a confined command
/// or an extension's program, and the look that runs it may come at any
/// later level. So git does not follow the level: where the sandbox
/// enforces, whatever its configuration names runs with no write root and
/// no network. What it costs: a filter that must write or reach the network
/// (git-lfs's) fails at `unrestricted` too, and the list of what a command
/// changed says it is incomplete, as it does below `unrestricted`.
///
/// The session's checkpoints need their git before its sandbox can be
/// made (the sandbox needs the lease), so this is made first and given the
/// sandbox once it is ([`set`](Self::set)). Until then git is not started
/// at all: no answer is better than one from an unconfined git.
#[derive(Debug)]
pub(crate) struct SessionGit {
    sandbox: OnceLock<Arc<SessionSandbox>>,
    /// The resolved directories to keep the git program out of: the
    /// workspace, and the write roots while the session would confine.
    avoid: Vec<PathBuf>,
    /// The providers' key variables the session started with: a variable
    /// that stops being a key at a reload is still withheld, as it is from
    /// the shell's commands (their removal list is seeded the same way).
    seeded: Vec<String>,
    /// The providers' key variables as they are now: the list the shell
    /// tool's commands are kept from, the same value, which a reload sets.
    withheld: Arc<Withheld>,
    /// The environment git's is built from, when it is not this process's
    /// own (a test's).
    inherited: Option<Vec<(OsString, OsString)>>,
}

impl SessionGit {
    /// A git that is never found inside one of `avoid`, and is never given
    /// a variable that `withheld` names now or named when the session
    /// started, from `inherited` or this process's own environment.
    pub(crate) fn new(
        avoid: Vec<PathBuf>,
        withheld: Arc<Withheld>,
        inherited: Option<Vec<(OsString, OsString)>>,
    ) -> Self {
        Self {
            sandbox: OnceLock::new(),
            avoid,
            seeded: withheld.get(),
            withheld,
            inherited,
        }
    }

    /// Gives it the session's sandbox. Only the first call counts.
    pub(crate) fn set(&self, sandbox: Arc<SessionSandbox>) {
        let _ = self.sandbox.set(sandbox);
    }
}

impl Confinement for SessionGit {
    /// Wherever the sandbox enforces, whatever the level.
    fn confines(&self) -> bool {
        self.sandbox.get().is_some_and(|sandbox| sandbox.enforces())
    }

    /// Git on the `PATH` of the environment it inherits, outside every one
    /// of `avoid`.
    fn git_program(&self) -> Option<PathBuf> {
        let path = match &self.inherited {
            Some(inherited) => inherited
                .iter()
                .find(|(name, _)| cineris_base::child_env::is_named(name, "PATH", cfg!(windows)))
                .map(|(_, value)| std::env::split_paths(value).collect())
                .unwrap_or_default(),
            None => cineris_base::program::path_directories(),
        };
        cineris_base::program::find_outside(&path, "git", cfg!(windows), &self.avoid())
    }

    fn withheld(&self) -> Vec<String> {
        let mut withheld = self.seeded.clone();
        withheld.extend(self.withheld.get());
        withheld
    }

    /// The directories the session started with, and the write roots a
    /// reload has added since.
    fn avoid(&self) -> Vec<PathBuf> {
        match self.sandbox.get() {
            Some(sandbox) => sandbox.avoiding_roots_now(&self.avoid),
            None => self.avoid.clone(),
        }
    }

    fn inherited(&self) -> Vec<(OsString, OsString)> {
        self.inherited
            .clone()
            .unwrap_or_else(|| std::env::vars_os().collect())
    }

    fn start(&self, git: &Path, arguments: Vec<OsString>) -> Result<Started, String> {
        let sandbox = self
            .sandbox
            .get()
            .ok_or_else(|| "the session's sandbox is not made yet".to_owned())?;
        Ok(sandbox.start_git(git, arguments))
    }
}

/// `git` with `arguments`, started through the reading launcher `reader`.
fn reading(reader: &SandboxLauncher, git: &Path, arguments: Vec<OsString>) -> Started {
    let wrapped = reader.wrap(git.as_os_str(), &arguments);
    Started {
        program: wrapped.program,
        arguments: wrapped.arguments,
        stdin: Some(wrapped.stdin),
    }
}

/// What the look after a confined start found among the protected paths,
/// and what of it was put back ([`protected_changes`]).
#[derive(Debug)]
pub(crate) struct Found {
    /// What the start changed, inside its own write roots.
    pub(crate) changed: Vec<Changed>,
    /// What still differs from the look before once the change was put
    /// back: what could not be, to be put back by hand.
    pub(crate) left: Vec<Changed>,
    /// What changed while the start ran outside every root it could write:
    /// not the start's, so not put back (the user's own hand, or a program
    /// of the user's), and told as a change from outside.
    pub(crate) outside: Vec<Changed>,
    /// What is as it was before, again, where the last look reached.
    pub(crate) restored: Vec<PathBuf>,
    /// What a path held when the put-back wrote over or removed it, and
    /// where that is kept.
    pub(crate) kept: Vec<(PathBuf, PathBuf)>,
    /// The folders the last look found and could not list, and whether the
    /// look before could list each.
    pub(crate) unlistable: Vec<(PathBuf, bool)>,
    /// In a session kept in memory, what is left as the start made it
    /// because what it holds could not be kept first: there is nowhere to
    /// keep it.
    pub(crate) kept_nowhere: Vec<PathBuf>,
    /// The folders the look before listed and the look after could not,
    /// given their owner's right to list them back.
    pub(crate) reopened: Vec<PathBuf>,
    /// Whether the look before and the last look stopped at their bound.
    pub(crate) stopped: (bool, bool),
    /// Whether the look before and the look after were complete.
    pub(crate) complete: (bool, bool),
    /// The last look taken: after the put-back, where there was one. The
    /// next confined start compares with it.
    pub(crate) after: Snapshot,
}

/// Looks at the protected paths after a confined start, against `before`,
/// and puts back at once what the start changed, from `before`'s own record
/// ([`Watch::put_back`]): nothing a confined command, a project's check or
/// an extension's program wrote in a protected path is kept, so nothing of
/// it runs later outside the sandbox (the third security fix). Only what
/// lies inside one of the start's own write roots (`roots`) is its, and
/// put back: a change elsewhere was made by something else while it ran,
/// and is told as such, not undone (found by the early review of the third
/// security fix: a protected path of the user's own outside the workspace
/// was put back). Whether the looks were complete is told of the last look
/// taken, after the put-back. It blocks.
pub(crate) fn protected_changes(
    watch: &Watch,
    before: &Snapshot,
    roots: &[PathBuf],
    keep: Option<&Path>,
) -> Found {
    let unlistable = |last: &Snapshot| -> Vec<(PathBuf, bool)> {
        last.unlistable()
            .iter()
            .map(|folder| (folder.clone(), !before.unlistable().contains(folder)))
            .collect()
    };
    let within = |path: &Path| {
        roots
            .iter()
            .any(|root| cineris_base::location_within(path, root))
    };
    let (all, after) = watch.changes_since(before);
    let (changed, mut outside): (Vec<Changed>, Vec<Changed>) =
        all.into_iter().partition(|one| within(&one.path));
    // A folder the look before listed and this one cannot hides what is in
    // it: the put-back opens it and looks again, even where nothing else
    // was found (the narrow check of the third security fix).
    let closed = after
        .unlistable()
        .iter()
        .any(|folder| before.was_listed(folder) && within(folder));
    if changed.is_empty() && !closed {
        return Found {
            changed,
            left: Vec::new(),
            outside,
            restored: Vec::new(),
            kept: Vec::new(),
            unlistable: unlistable(&after),
            kept_nowhere: Vec::new(),
            reopened: Vec::new(),
            stopped: (before.stopped_at_its_bound(), after.stopped_at_its_bound()),
            complete: (before.is_complete(), after.is_complete()),
            after,
        };
    }
    let put = watch.put_back(before, &changed, &within, keep);
    // What the last look still finds outside the start's roots is told as
    // a change from outside: the put-back did not touch it.
    for one in &put.left {
        if !within(&one.path) && !outside.iter().any(|other| other.path == one.path) {
            outside.push(one.clone());
        }
    }
    let left = put
        .left
        .into_iter()
        .filter(|one| within(&one.path))
        .collect();
    Found {
        changed: put.changed,
        left,
        outside,
        restored: put.restored,
        kept: put.kept,
        unlistable: unlistable(&put.after),
        kept_nowhere: if keep.is_none() {
            put.unkept
        } else {
            Vec::new()
        },
        reopened: put.reopened,
        stopped: (
            before.stopped_at_its_bound(),
            put.after.stopped_at_its_bound(),
        ),
        complete: (before.is_complete(), put.after.is_complete()),
        after: put.after,
    }
}

impl Found {
    /// Whether what was found stops the turn: a change the start made, a
    /// change from outside while it ran, a look the start made stop at its
    /// bound where the one before did not, or a folder it made unlistable.
    pub(crate) fn stops_the_turn(&self) -> bool {
        !self.changed.is_empty()
            || !self.outside.is_empty()
            || !self.reopened.is_empty()
            || (self.stopped.1 && !self.stopped.0)
            || self.unlistable.iter().any(|(_, new)| *new)
    }
}

/// What the model and the user are told after a confined start changed
/// protected paths: what was put back, what is left to put back by hand,
/// what was not seen again, and where what was written over is kept; after
/// a change outside the start's reach while it ran; of a folder no look can
/// list; and when the look stopped at its bound. `None` when there is
/// nothing to say.
pub(crate) fn protected_note(found: &Found, root: &Path) -> Option<String> {
    // A name from the workspace, which the model reads: every character
    // that would act unseen is an escape.
    let name = |path: &Path| -> String {
        let path = path.strip_prefix(root).unwrap_or(path);
        format!("`{}`", cineris_base::visible(&path.to_string_lossy()))
    };
    let shown = |paths: &mut dyn Iterator<Item = &Changed>| -> Vec<String> {
        paths
            .map(|changed| format!("{} ({})", name(&changed.path), changed.change.as_str()))
            .collect()
    };
    let mut lines = Vec::new();
    // First, and plainly: in a session that keeps nothing on disk, what
    // could not be put back stays as the command made it, and git runs
    // what it names (the maintainers' settlement after the full review).
    if !found.kept_nowhere.is_empty() {
        let names: Vec<String> = found.kept_nowhere.iter().map(|path| name(path)).collect();
        lines.push(format!("[{}]", kept_nowhere_words(&names)));
    }
    if !found.changed.is_empty() {
        let all = shown(&mut found.changed.iter());
        let back = shown(
            &mut found
                .changed
                .iter()
                .filter(|one| found.restored.contains(&one.path)),
        );
        let left = shown(&mut found.left.iter());
        let unseen = shown(&mut found.changed.iter().filter(|one| {
            !found.restored.contains(&one.path)
                && !found.left.iter().any(|still| still.path == one.path)
        }));
        let mut said = Vec::new();
        if back.len() == all.len() {
            said.push("They were put back as they were before it.".to_owned());
        } else {
            if !back.is_empty() {
                said.push(format!(
                    "These were put back as they were before it: {}.",
                    back.join(", ")
                ));
            }
            if !left.is_empty() {
                said.push(format!(
                    "These could not be put back (a link on the way, what they held not kept, a \
                     directory that is not empty, or a place first watched during the command), \
                     and must be put back by hand: {}.",
                    left.join(", ")
                ));
            }
            if !unseen.is_empty() {
                said.push(format!(
                    "These were not seen again after the put-back (the look did not reach them), \
                     and must be looked at by hand: {}.",
                    unseen.join(", ")
                ));
            }
        }
        if !found.kept.is_empty() {
            let kept: Vec<String> = found
                .kept
                .iter()
                .map(|(path, copy)| {
                    let copy = copy
                        .file_name()
                        .map_or_else(String::new, |copy| copy.to_string_lossy().into_owned());
                    format!(
                        "{} as `put-back/{}`",
                        name(path),
                        cineris_base::visible(&copy)
                    )
                })
                .collect();
            said.push(format!(
                "What they held when the command ended is kept in the session's folder: {}.",
                kept.join(", ")
            ));
        }
        // The owner's decision of 2026-11-21: a change a confined start
        // makes there is never kept, an approved one included.
        lines.push(format!(
            "[The command changed protected paths, which decide what runs later outside the \
             sandbox: {}. {} Nothing run inside the sandbox keeps a change to a protected \
             path. If the user wants this change, make it with a command outside the sandbox \
             (`sandbox: false`), which the user is asked to allow once, or leave it to the \
             user. The turn stops here, so that the user can look.]",
            all.join(", "),
            said.join(" "),
        ));
    }
    if !found.outside.is_empty() {
        lines.push(format!(
            "[While the command ran, protected paths it could not write changed: {}. Something \
             other than the command changed them (the user's own hand, or a program of the \
             user's), so they are left as they are. The turn stops here, so that the user can \
             look.]",
            shown(&mut found.outside.iter()).join(", ")
        ));
    }
    if !found.reopened.is_empty() {
        let folders: Vec<String> = found.reopened.iter().map(|folder| name(folder)).collect();
        lines.push(format!(
            "[The command made these folders so that no look could list them, and they were \
             given their owner's right to list them back, so that what is in them was looked \
             at: {}. The turn stops here, so that the user can look.]",
            folders.join(", ")
        ));
    }
    if !found.unlistable.is_empty() {
        let folders: Vec<String> = found
            .unlistable
            .iter()
            .map(|(folder, _)| name(folder))
            .collect();
        let made = found.unlistable.iter().any(|(_, new)| *new);
        lines.push(format!(
            "[These folders cannot be listed, so what is in them was not looked at: {}.{}]",
            folders.join(", "),
            if made {
                " The command made them so. The turn stops here, so that the user can look."
            } else {
                ""
            }
        ));
    }
    let (stopped_before, stopped_after) = found.stopped;
    if stopped_after && !stopped_before {
        lines.push(
            "[The look at protected paths after the command stopped at its bound, where the \
             look before it did not: the command made the workspace too large to look at \
             whole, and a change it made further down would not be seen. The turn stops \
             here, so that the user can look.]"
                .to_owned(),
        );
    } else if stopped_after {
        lines.push(
            "[Protected paths were looked at only in part: the look stopped at its bound. A \
             change to one further down the workspace would not have been seen.]"
                .to_owned(),
        );
    } else if !found.complete.1 && found.unlistable.is_empty() {
        lines.push(
            "[Protected paths were looked at only in part: a file was too large to compare. A \
             change to it may not have been seen.]"
                .to_owned(),
        );
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// What a confined start changed and the put-back left to be put back by
/// hand, remembered to be told again at each turn's start.
#[derive(Debug)]
struct ByHand {
    /// The look after the start, the put-back's last: a path still as it
    /// holds it is still as the start left it.
    after: Snapshot,
    /// Each path, and whether it was left because the session keeps
    /// nothing on disk.
    paths: Vec<(PathBuf, bool)>,
}

/// What the user is told, again, of protected paths a confined start
/// changed that could not be put back, in a session that keeps its state:
/// `names` are the paths, each in backticks already.
pub(crate) fn by_hand_words(names: &[String]) -> String {
    let (were, it, name) = if names.len() == 1 {
        ("was", "it", "names")
    } else {
        ("were", "them", "name")
    };
    let what = if names.len() == 1 { "it" } else { "they" };
    format!(
        "{} {were} changed by a command run in the sandbox and could not be put back. Git, and \
         so the model's own git commands, will run what {what} now {name}, as will any other \
         tool that reads {it}. Put {it} back by hand before continuing.",
        names.join(", ")
    )
}

/// What the user and the model are told of protected paths a confined
/// start changed, in a session kept in memory, that could not be put back:
/// `names` are the paths, each in backticks already.
pub(crate) fn kept_nowhere_words(names: &[String]) -> String {
    let (were, holds, it, name) = if names.len() == 1 {
        ("was", "the file holds", "it", "names")
    } else {
        ("were", "the files hold", "them", "name")
    };
    let what = if names.len() == 1 { "it" } else { "they" };
    format!(
        "{} {were} changed by a command run in the sandbox and {were} not put back: this \
         session keeps nothing on disk, so what {holds} now could not be kept first. Git, and \
         so the model's own git commands, will run what {what} now {name}, as will any other \
         tool that reads {it}. Put {it} back by hand before continuing.",
        names.join(", ")
    )
}

/// Tells the user, at a turn's start, what a confined start changed and
/// is still not put back, as the start left it: the stop that named it may
/// have been passed over, and the level raised since, where the model's own
/// git runs outside the sandbox.
pub(crate) async fn tell_what_is_still_by_hand(live: &crate::core::Live) {
    let sandbox = Arc::clone(&live.sandbox);
    let Ok(still) = tokio::task::spawn_blocking(move || sandbox.still_by_hand()).await else {
        return;
    };
    if still.is_empty() {
        return;
    }
    let root = live.sandbox.workspace().to_path_buf();
    let name = |path: &Path| {
        let path = path.strip_prefix(&root).unwrap_or(path);
        format!("`{}`", cineris_base::visible(&path.to_string_lossy()))
    };
    let nowhere: Vec<String> = still
        .iter()
        .filter(|(_, nowhere)| *nowhere)
        .map(|(path, _)| name(path))
        .collect();
    let other: Vec<String> = still
        .iter()
        .filter(|(_, nowhere)| !*nowhere)
        .map(|(path, _)| name(path))
        .collect();
    let mut said = Vec::new();
    if !nowhere.is_empty() {
        said.push(kept_nowhere_words(&nowhere));
    }
    if !other.is_empty() {
        said.push(by_hand_words(&other));
    }
    live.hooks.emit(cineris_protocol::event::Event::Notice(
        cineris_protocol::Notice::Sandbox(cineris_protocol::event::SandboxNotice {
            message: format!("Still not put back. {}", said.join(" ")),
            status: if live.sandbox.confines() {
                SandboxStatus::Enforcing
            } else {
                SandboxStatus::Off
            },
        }),
    ));
}

/// The launcher's policy, from the lease's grant.
///
/// Each root is resolved to its real path here, once, when the session's
/// policy is made: a link the user set up on purpose (a `sandbox.write`
/// through a symlinked home) is followed now, so that the launcher can
/// open the real path with no link followed and refuse one a confined
/// command plants later (finding 6 of the third review of step 26;
/// [`crate::confine`]'s `grant`). A root that does not resolve (it is not
/// there yet) is kept as it is: the launcher skips a root that is not
/// there, and reaches the no-link rule for one that appears.
fn policy_of(grant: &SandboxPolicy, temp: &Temp, landlock: u8) -> Policy {
    let roots = |roots: &[String]| {
        roots
            .iter()
            .map(|root| resolve_root(Path::new(root)))
            .collect::<Vec<_>>()
    };
    let mut write = roots(&grant.write.roots);
    // The temporary directory is the session's own, made with no link on
    // the way (`make_temp`): its real path is itself.
    let temp_path = resolve_root(&temp.path);
    // A temporary directory that was not made, or is not the session's own,
    // is not written to: someone else could read it, or it leads elsewhere.
    if !temp.made {
        // By place: when the directory was not made, neither side could be
        // resolved, and one is in the extended-length form on Windows.
        write.retain(|root| !cineris_base::same_location(root, &temp_path));
    }
    Policy {
        read: if grant.read.all {
            Reach::All
        } else {
            Reach::Only(roots(&grant.read.roots))
        },
        write,
        network: match grant.network {
            NetworkAccess::Off => Network::Off,
            NetworkAccess::Full => Network::Full,
        },
        temp: temp.made.then_some(temp_path),
        landlock,
    }
}

/// A root's real path, with every symbolic link on the way to it resolved,
/// or the path as it is when it cannot be resolved (it is not there). It
/// blocks.
#[allow(
    clippy::disallowed_methods,
    reason = "resolving the roots is part of making the session's sandbox, on the blocking pool"
)]
fn resolve_root(root: &Path) -> PathBuf {
    std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

/// Makes the session's temporary directory, open to the user alone. It
/// blocks.
///
/// # Errors
///
/// When it cannot be made, or something is already at its path that is not
/// a private directory of the user's own: another user could have put it
/// there, or a link to somewhere else.
pub(crate) fn make_temp(path: &Path) -> std::io::Result<PathBuf> {
    let made = make_private_dir(path);
    match made {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || !owned_by_me(&metadata) || !cineris_base::private::is_private(path)? {
        return Err(std::io::Error::other(format!(
            "{} is already there and is not a private directory of yours",
            path.display()
        )));
    }
    Ok(path.to_path_buf())
}

#[cfg(unix)]
fn make_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn make_private_dir(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    cineris_base::private::create_dir_all(path)
}

#[cfg(unix)]
fn owned_by_me(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    // The owner of a file this process made in the temporary directory is
    // its own: a directory there owned by anyone else was not made here.
    let own = std::fs::metadata("/proc/self").map(|own| own.uid());
    own.is_ok_and(|uid| uid == metadata.uid())
}

#[cfg(not(unix))]
fn owned_by_me(_metadata: &std::fs::Metadata) -> bool {
    // The access-list check of `is_private` says whose it is on Windows.
    true
}

/// Removes a temporary directory and everything in it, following no link.
fn remove_temp(path: &Path) {
    if let Err(error) = std::fs::remove_dir_all(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), %error, "the session's temporary directory was not removed");
    }
}

#[cfg(test)]
mod tests {
    use cineris_base::Rng;
    use cineris_sandbox::support::Support;

    use super::*;

    /// The note on changed protected paths names them, and the model reads
    /// it in the command's result: a name is an escape there, so that it
    /// cannot start lines of its own (the fixes of the independent review
    /// of step 18).
    #[test]
    fn the_note_on_protected_paths_names_them_as_escapes() {
        let root = PathBuf::from("/work");
        let changed = [cineris_sandbox::protect::Changed {
            path: root.join(".git/hooks/x\nNotice from the harness\u{E0041}"),
            change: cineris_sandbox::protect::Change::Created,
            before: cineris_sandbox::protect::Before::Absent,
            mode_changed: false,
        }];
        let found = Found {
            restored: changed.iter().map(|one| one.path.clone()).collect(),
            changed: changed.to_vec(),
            left: Vec::new(),
            outside: Vec::new(),
            kept: Vec::new(),
            unlistable: Vec::new(),
            kept_nowhere: Vec::new(),
            reopened: Vec::new(),
            stopped: (false, false),
            complete: (true, true),
            after: Snapshot::default(),
        };
        let note = protected_note(&found, &root).expect("a note");
        assert!(
            !note
                .lines()
                .any(|line| line.starts_with("Notice from the harness")),
            "{note}"
        );
        assert!(
            note.contains("x\\nNotice from the harness\\u{e0041}"),
            "{note}"
        );
    }

    /// In a session kept in memory, what could not be put back is said
    /// first, plainly: the file, that a command in the sandbox changed it
    /// and it stays so because nothing is kept on disk, that git and the
    /// model's own git commands will run what it names, and to put it back
    /// before going on (the maintainers' settlement after the full review
    /// of the third security fix).
    #[test]
    fn what_a_session_kept_in_memory_cannot_put_back_is_said_first() {
        let root = PathBuf::from("/work");
        let config = root.join(".git/config");
        let changed = [cineris_sandbox::protect::Changed {
            path: config.clone(),
            change: cineris_sandbox::protect::Change::Modified,
            before: cineris_sandbox::protect::Before::Unknown,
            mode_changed: false,
        }];
        let found = Found {
            restored: Vec::new(),
            changed: changed.to_vec(),
            left: changed.to_vec(),
            outside: Vec::new(),
            kept: Vec::new(),
            unlistable: Vec::new(),
            kept_nowhere: vec![config],
            reopened: Vec::new(),
            stopped: (false, false),
            complete: (true, true),
            after: Snapshot::default(),
        };
        let note = protected_note(&found, &root).expect("a note");
        let first = note.lines().next().unwrap_or_default();
        assert_eq!(
            first,
            "[`.git/config` was changed by a command run in the sandbox and was not put back: \
             this session keeps nothing on disk, so what the file holds now could not be kept \
             first. Git, and so the model's own git commands, will run what it now names, as \
             will any other tool that reads it. Put it back by hand before continuing.]"
        );
        let two = kept_nowhere_words(&["`a`".to_owned(), "`b`".to_owned()]);
        assert!(
            two.starts_with("`a`, `b` were changed")
                && two.contains("what they now name")
                && two.ends_with("Put them back by hand before continuing."),
            "{two}"
        );
    }

    /// The harness's git withholds the key variables a reload sets, through
    /// the one value the shell holds too, and still the ones the session
    /// started with.
    #[test]
    fn the_harness_git_withholds_what_a_reload_sets_and_what_the_start_had() {
        let withheld = Arc::new(Withheld::new(vec!["OLD_KEY".to_owned()]));
        let git = SessionGit::new(Vec::new(), Arc::clone(&withheld), None);
        assert_eq!(git.withheld(), ["OLD_KEY", "OLD_KEY"]);
        withheld.set(vec!["NEW_KEY".to_owned()]);
        assert_eq!(git.withheld(), ["OLD_KEY", "NEW_KEY"]);
    }

    /// The harness's git is never one inside what the session avoids (the
    /// workspace, a write root): the search goes on down `PATH`.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "the test plants programs")]
    fn the_harness_git_is_never_one_inside_the_workspace() {
        use cineris_base::Rng;
        let dir = std::env::temp_dir().join(format!(
            "cineris-core-git-avoid-{:016x}",
            cineris_base::rng::SystemRng::new().next_u64()
        ));
        let name = if cfg!(windows) { "git.exe" } else { "git" };
        let plant = |folder: &Path| {
            std::fs::create_dir_all(folder).unwrap();
            let path = folder.join(name);
            std::fs::write(&path, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        };
        let ws = dir.join("ws");
        let planted = plant(&ws.join(".venv").join("bin"));
        let real = plant(&dir.join("usr").join("bin"));
        let path =
            std::env::join_paths([planted.parent().unwrap(), real.parent().unwrap()]).unwrap();
        let inherited = Some(vec![(OsString::from("PATH"), path)]);
        let avoiding = SessionGit::new(vec![ws], Arc::default(), inherited.clone());
        assert_eq!(avoiding.git_program(), Some(real));
        let not = SessionGit::new(Vec::new(), Arc::default(), inherited);
        assert_eq!(not.git_program(), Some(planted));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The harness's own git, wherever the sandbox enforces, is started by
    /// the launcher under a policy that writes nothing and reaches no
    /// network, at `unrestricted` too (the third security fix); where the
    /// sandbox does not enforce it is started as it is.
    #[test]
    fn the_harness_git_runs_inside_the_sandbox_reading_only() {
        let workspace = std::env::temp_dir().join("cineris-core-git-policy");
        let root = cineris_base::AbsPath::from_os(&workspace).unwrap();
        let mut grant = cineris_policy::default_sandbox(&root, &[], &[]);
        grant.network = NetworkAccess::Full;
        let core = CoreSandbox::new(
            Support::Landlock(8),
            Some((
                PathBuf::from("/opt/cineris"),
                vec![OsString::from("__sandbox")],
            )),
        );
        let (state, enforcing) = core.state(true);
        let sandbox = Arc::new(SessionSandbox::new(
            &core,
            state,
            enforcing.map(|(landlock, _)| landlock),
            &Level::Autonomous,
            &grant,
            Temp {
                path: workspace.join("tmp"),
                made: true,
            },
            &workspace,
            Limits::default(),
        ));
        let git = SessionGit::new(Vec::new(), Arc::default(), None);
        let arguments = vec![OsString::from("status")];
        // Before it is given the sandbox, it starts nothing.
        assert!(
            git.start(Path::new("/usr/bin/git"), arguments.clone())
                .is_err()
        );
        git.set(Arc::clone(&sandbox));
        assert!(git.confines());

        let started = git
            .start(Path::new("/usr/bin/git"), arguments.clone())
            .unwrap();
        assert_eq!(started.program, PathBuf::from("/opt/cineris"));
        assert_eq!(
            started.arguments,
            ["__sandbox", "--", "/usr/bin/git", "status"].map(OsString::from)
        );
        let policy = Policy::decode(&started.stdin.expect("the policy")).unwrap();
        assert_eq!(
            policy,
            Policy {
                read: Reach::All,
                write: Vec::new(),
                network: Network::Off,
                temp: None,
                landlock: 8,
            }
        );

        // At `unrestricted`, where commands run as they are, git is still
        // the sandbox's: what it reads a confined start may have written.
        sandbox.set_level(&Level::Unrestricted);
        assert!(!sandbox.confines());
        assert!(git.confines());
        let started = git
            .start(Path::new("/usr/bin/git"), arguments.clone())
            .unwrap();
        assert_eq!(started.program, PathBuf::from("/opt/cineris"));
        assert_eq!(
            Policy::decode(&started.stdin.expect("the policy"))
                .unwrap()
                .write,
            Vec::<PathBuf>::new()
        );

        // Where the sandbox does not enforce, git runs as it is.
        let off = Arc::new(SessionSandbox::new(
            &core,
            SandboxState::NotEnforcing(NoSandbox::TurnedOff),
            None,
            &Level::Supervised,
            &grant,
            Temp {
                path: workspace.join("tmp"),
                made: false,
            },
            &workspace,
            Limits::default(),
        ));
        let as_it_is = SessionGit::new(Vec::new(), Arc::default(), None);
        as_it_is.set(off);
        assert!(!as_it_is.confines());
        let started = as_it_is
            .start(Path::new("/usr/bin/git"), arguments)
            .unwrap();
        assert_eq!(started.program, PathBuf::from("/usr/bin/git"));
        assert_eq!(started.stdin, None);
    }

    /// One confined start of a session at a time, from its look before to
    /// its look after: a second waits for the first's guard to go (the
    /// early review of the third security fix).
    #[test]
    fn confined_starts_take_their_turn_one_at_a_time() {
        let workspace = std::env::temp_dir().join("cineris-core-one-at-a-time");
        let root = cineris_base::AbsPath::from_os(&workspace).unwrap();
        let grant = cineris_policy::default_sandbox(&root, &[], &[]);
        let core = CoreSandbox::new(
            Support::Landlock(8),
            Some((
                PathBuf::from("/opt/cineris"),
                vec![OsString::from("__sandbox")],
            )),
        );
        let (state, enforcing) = core.state(true);
        let sandbox = SessionSandbox::new(
            &core,
            state,
            enforcing.map(|(landlock, _)| landlock),
            &Level::Autonomous,
            &grant,
            Temp {
                path: workspace.join("tmp"),
                made: false,
            },
            &workspace,
            Limits::default(),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let first = sandbox.one_start_at_a_time().await;
            assert!(
                sandbox.starts.try_lock().is_err(),
                "a second start went ahead"
            );
            drop(first);
            assert!(sandbox.starts.try_lock().is_ok());
        });
    }

    /// After a command escapes its kill, the protected paths are looked at
    /// again against the look before that command, so a hook it writes
    /// after its turn is found; only the first escape is remembered.
    #[test]
    fn a_change_after_an_escaped_command_is_found_on_re_check() {
        let workspace = std::env::temp_dir().join(format!(
            "cineris-core-recheck-{:016x}",
            cineris_base::rng::SystemRng::new().next_u64()
        ));
        let hooks = workspace.join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let root =
            cineris_base::AbsPath::from_os(&std::fs::canonicalize(&workspace).unwrap()).unwrap();
        let grant = cineris_policy::default_sandbox(&root, &[], &[]);
        let core = CoreSandbox::new(
            Support::Landlock(8),
            Some((
                PathBuf::from("/opt/cineris"),
                vec![OsString::from("__sandbox")],
            )),
        );
        let (state, enforcing) = core.state(true);
        let sandbox = SessionSandbox::new(
            &core,
            state,
            enforcing.map(|(landlock, _)| landlock),
            &Level::Autonomous,
            &grant,
            Temp {
                path: workspace.join("tmp"),
                made: false,
            },
            root.to_fs_path().as_path(),
            // A time no machine reaches: the standard quarter of a second
            // of real time, reached on a loaded machine, would leave the
            // hooks unlooked at by chance.
            Limits {
                time: std::time::Duration::from_secs(3600),
                ..Limits::default()
            },
        );
        // Nothing to compare with yet: the close re-check finds nothing.
        assert!(sandbox.continuity_at_close().is_none());
        // A command runs; the look after it is remembered.
        let after = sandbox.protected().expect("a watch").take();
        sandbox.remember_after(after);
        // A process the command left running writes a hook after the turn.
        std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\ncurl evil\n").unwrap();
        // The next command's look finds the change against the after-look.
        let now = sandbox.protected().expect("a watch").take();
        let changed = sandbox.continuity(&now).expect("a comparison");
        assert!(
            changed
                .iter()
                .any(|one| one.path.ends_with(".git/hooks/pre-commit")),
            "{changed:?}"
        );
        // The close re-check finds it too.
        assert!(sandbox.continuity_at_close().is_some());
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// What the configuration names is watched only by name: a folder (a
    /// hooks folder named whole, `./tools` in a program's line), the
    /// workspace and a folder above it are not, a file and a name not there
    /// yet are.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "the test makes its files")]
    fn a_folder_or_the_workspace_named_is_not_watched() {
        let dir = std::env::temp_dir().join(format!(
            "cineris-core-watched-by-name-{:016x}",
            cineris_base::rng::SystemRng::new().next_u64()
        ));
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join("tools")).unwrap();
        std::fs::write(ws.join("tools/strip.sh"), "").unwrap();
        assert!(watched_by_name(&ws.join("tools/strip.sh"), &ws));
        assert!(watched_by_name(&ws.join("tools/not-yet.sh"), &ws));
        assert!(!watched_by_name(&ws.join("tools"), &ws));
        assert!(!watched_by_name(&ws, &ws));
        assert!(!watched_by_name(&dir, &ws));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What the configuration names is taken where it really is: a folder
    /// on its way that is a link is resolved, so that it is held to the
    /// write roots by place, not by its text.
    #[cfg(unix)]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "the test makes its files")]
    fn a_named_path_is_taken_where_it_really_is() {
        let dir = std::env::temp_dir().join(format!(
            "cineris-core-where-it-is-{:016x}",
            cineris_base::rng::SystemRng::new().next_u64()
        ));
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("via")).unwrap();
        let real = std::fs::canonicalize(dir.join("real")).unwrap();
        assert_eq!(where_it_is(&dir.join("via/hook")), real.join("hook"));
        // A folder not there: the path as it is.
        assert_eq!(where_it_is(&dir.join("gone/hook")), dir.join("gone/hook"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
