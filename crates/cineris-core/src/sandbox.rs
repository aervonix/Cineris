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
//! - **The harness's own git** runs inside the sandbox too while commands
//!   are confined, with no write root and no network ([`SessionGit`]): a
//!   confined command may write the repository's configuration, which
//!   can name a program for git to run (found by the third review of step
//!   26). After an extension's program, which its own launcher confines at
//!   every level, the look at what it changed runs git inside the sandbox
//!   at every level too (`FollowingTheProgram`).

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
        let watch = self.watch.as_ref()?;
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

    /// What to look at before and after a command that runs now: the
    /// protected paths, when the command is confined.
    pub(crate) fn watch(&self) -> Option<&Watch> {
        self.watch.as_ref().filter(|_| self.confines())
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

    /// What to look at before and after an extension's program that may
    /// write: the protected paths, wherever the sandbox enforces, at every
    /// level, as its launcher confines it at every level.
    #[cfg(feature = "scripts")]
    pub(crate) fn extension_watch(&self) -> Option<&Watch> {
        self.watch.as_ref()
    }

    /// What to start in place of the harness's own `git` with `arguments`
    /// now: the reading launcher while commands are confined, git as it is
    /// otherwise.
    fn start_git(&self, git: &Path, arguments: Vec<OsString>) -> Started {
        match self.reader.as_ref().filter(|_| self.confines()) {
            Some(reader) => reading(reader, git, arguments),
            None => Started {
                program: git.to_owned(),
                arguments,
                stdin: None,
            },
        }
    }

    /// What to start in place of the harness's own `git` with `arguments`
    /// at every level, `unrestricted` included: the reading launcher.
    /// After a program that something other than the session's level
    /// confined (an extension's, by its own launcher), so that git follows
    /// the program's confinement, not the session's commands'.
    #[cfg(feature = "scripts")]
    fn start_git_confined(&self, git: &Path, arguments: Vec<OsString>) -> Result<Started, String> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| "the sandbox does not enforce in this session".to_owned())?;
        Ok(reading(reader, git, arguments))
    }

    /// Removes the temporary directory and everything in it, if the
    /// session made it. It blocks.
    pub(crate) fn remove_temp(&self) {
        if self.temp.made {
            remove_temp(&self.temp.path);
        }
    }
}

impl Launcher for SessionSandbox {
    fn launch(&self, command: CommandLine, cx: &LaunchCx<'_>) -> Result<Launch, String> {
        let Some(launcher) = self.launcher() else {
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
        let launcher = self.launcher()?;
        let code = match exit {
            Some(Exit::Code(code)) => Some(code),
            _ => None,
        };
        launcher.explain(code, output)
    }
}

/// Where a session's own git runs, and with what: inside its sandbox,
/// reading only, while its commands are confined; never a git inside the
/// workspace, or inside a write root a confined command could have planted
/// one in; and without the providers' key variables.
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
    fn confines(&self) -> bool {
        self.sandbox.get().is_some_and(|sandbox| sandbox.confines())
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

/// The session's git after a program an extension's own launcher confined:
/// started inside the sandbox, reading only, at every level, as the program
/// was confined at every level (the review of step 28's process starts).
/// At `unrestricted` the session's git runs as it is, after commands that
/// ran as they are; but a program confined by its launcher may have
/// written the repository's configuration inside its write roots, and a
/// filter it names would then run, with the user's full rights, in the
/// look at what the program changed. Where the sandbox does not enforce,
/// nothing is started: no program was confined there to follow.
#[cfg(feature = "scripts")]
#[derive(Debug)]
pub(crate) struct FollowingTheProgram(pub(crate) Arc<SessionGit>);

#[cfg(feature = "scripts")]
impl Confinement for FollowingTheProgram {
    fn confines(&self) -> bool {
        true
    }

    fn git_program(&self) -> Option<PathBuf> {
        self.0.git_program()
    }

    fn withheld(&self) -> Vec<String> {
        self.0.withheld()
    }

    fn avoid(&self) -> Vec<PathBuf> {
        self.0.avoid()
    }

    fn inherited(&self) -> Vec<(OsString, OsString)> {
        self.0.inherited()
    }

    fn start(&self, git: &Path, arguments: Vec<OsString>) -> Result<Started, String> {
        let sandbox = self
            .0
            .sandbox
            .get()
            .ok_or_else(|| "the session's sandbox is not made yet".to_owned())?;
        sandbox.start_git_confined(git, arguments)
    }
}

/// What the model and the user are told after a confined command changed
/// protected paths, or when the look at them stopped at its bound. `None`
/// when there is nothing to say.
pub(crate) fn protected_note(
    changed: &[Changed],
    (complete_before, complete_after): (bool, bool),
    root: &Path,
    by_hand: &[PathBuf],
) -> Option<String> {
    let shown = |paths: &mut dyn Iterator<Item = &Changed>| -> Vec<String> {
        paths
            .map(|changed| {
                let path = changed.path.strip_prefix(root).unwrap_or(&changed.path);
                // A name from the workspace, which the model reads: every
                // character that would act unseen is an escape.
                format!(
                    "`{}` ({})",
                    cineris_base::visible(&path.to_string_lossy()),
                    changed.change.as_str()
                )
            })
            .collect()
    };
    let mut lines = Vec::new();
    if !changed.is_empty() {
        let all = shown(&mut changed.iter());
        let not_undone = shown(&mut changed.iter().filter(|one| by_hand.contains(&one.path)));
        let undo = if not_undone.is_empty() {
            "Undoing this turn puts them back.".to_owned()
        } else if not_undone.len() == all.len() {
            "Undoing this turn does not put them back: what they were is not kept (a link, a \
             file too large, a change of mode, a directory, a path outside the workspace, or a \
             session that keeps no checkpoints), so they must be put back by hand."
                .to_owned()
        } else {
            format!(
                "Undoing this turn puts back the others; these must be put back by hand: {}.",
                not_undone.join(", ")
            )
        };
        lines.push(format!(
            "[The command changed protected paths, which decide what runs later outside the \
             sandbox: {}. The turn stops here, so that the user can look at the change. {undo}]",
            all.join(", "),
        ));
    }
    if complete_before && !complete_after {
        lines.push(
            "[The look at protected paths after the command stopped at its bound, where the \
             look before it did not: the command made the workspace too large to look at \
             whole, and a change it made further down would not be seen. The turn stops \
             here, so that the user can look.]"
                .to_owned(),
        );
    } else if !complete_after {
        lines.push(
            "[Protected paths were looked at only in part: the look stopped at its bound. A \
             change to one further down the workspace would not have been seen.]"
                .to_owned(),
        );
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
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
        let note = protected_note(&changed, (true, true), &root, &[]).expect("a note");
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

    /// The harness's own git, while commands are confined, is started by the
    /// launcher under a policy that writes nothing and reaches no network;
    /// at `unrestricted` it is started as it is.
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

        sandbox.set_level(&Level::Unrestricted);
        assert!(!git.confines());
        let started = git.start(Path::new("/usr/bin/git"), arguments).unwrap();
        assert_eq!(started.program, PathBuf::from("/usr/bin/git"));
        assert_eq!(started.stdin, None);
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
        let after = sandbox.watch().expect("a watch").take();
        sandbox.remember_after(after);
        // A process the command left running writes a hook after the turn.
        std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\ncurl evil\n").unwrap();
        // The next command's look finds the change against the after-look.
        let now = sandbox.watch().expect("a watch").take();
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
}
