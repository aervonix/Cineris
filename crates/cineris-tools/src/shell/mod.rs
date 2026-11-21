//! The `shell` tool: one command, run in the platform's shell
//! (`docs/architecture/05-tools.md`, "`shell`").
//!
//! | Part | What it does |
//! |---|---|
//! | [`Shell`], [`ShellKind`], [`ShellSearch`] | Which shell there is, and how each kind is started, wrapped and quoted for. |
//! | [`ShellTool`] | The tool: it plans a call, runs the command through the [process runner](crate::process), and writes the result. |
//! | [`ChangeDetector`] | Which files the command changed. |
//! | [`CommandProfile`] | What the project knows about a command, for choosing which lines of a long output to keep. |
//!
//! # What a call declares
//!
//! - An `exec` effect, with the command exactly as the model wrote it and
//!   the directory it would run in.
//! - A `write` effect on the workspace root: a command may change anything
//!   there. And one on its working directory, when that is outside the
//!   workspace.
//!
//! A call is scheduled alone. The permission engine may mark a plan
//! read-only when it knows the command changes nothing, and such a call is
//! then run beside others and is not asked what it changed.
//!
//! # What a result holds
//!
//! The output, compressed and cut to the limits by the tool itself
//! ([`output::shape`](crate::output::shape)), then a line that says how the
//! command ended. When anything was cut, a notice names the artifact that
//! holds all of it. Front ends also get a payload: the exit code or
//! signal, the duration, the flags, and the changed files.

pub mod changes;
mod env;
mod kind;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cineris_base::child_env::Withheld;
use cineris_base::git::{AsItIs, Confinement};
use cineris_base::{AbsPath, CancelToken, ResolvedPath};
use cineris_protocol::item::ArtifactRef;
use cineris_protocol::lease::RuleAction;
use serde::Deserialize;
use serde_json::json;

pub use changes::{ChangeDetector, Changes, Incomplete};
pub use env::{ANALYSIS_REMOVED, DEFAULT_REMOVED};
pub use kind::{Shell, ShellError, ShellKind, ShellSearch, Unquotable, Unrunnable};

use crate::args;
use crate::blocking::{BlockingError, run_blocking};
use crate::context::{PlanCx, RunCx};
use crate::contract::{
    BoxFuture, Concurrency, ConcurrencyClass, Effect, Json, Plan, Tool, ToolOutput, ToolSpec,
};
use crate::output::{self, Compression, Keep, OutputLimits};
use crate::process::{
    CommandLine, Direct, Ending, Exit, Finished, LaunchCx, Launcher, Runner, Start, Stop,
};
use crate::result::{ToolError, ToolResult};
use crate::store::ContentStore;

/// The time limit of a command that gives none.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// The longest time limit a command may ask for, unless configuration says
/// otherwise.
pub const DEFAULT_MAX_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// What an artifact of a command's output is called.
const ARTIFACT_LABEL: &str = "full output of shell";

/// How much of a command a summary shows.
const SUMMARY_CHARS: usize = 80;

/// The environment of a program a session starts beside its shell's
/// commands, by the same rules as a command's: an extension's
/// `cineris.exec`, a hook command (`docs/steps/28/answered.md`, "The
/// processes"). It is exactly what a command started in `cwd` is given
/// (`docs/architecture/05-tools.md`, "`shell`", "Environment"): `inherited`
/// without the providers' key variables ([`DEFAULT_REMOVED`]), without
/// what `removed` names (the session's `shell.env_remove` and the keys of
/// the providers it is configured with) and without [`ANALYSIS_REMOVED`];
/// no pager, no colour, a terminal that can do nothing; git's file-system
/// monitor off; a `PATH` of absolute directories only; on Windows
/// `NoDefaultCurrentDirectoryInExePath`; and `PWD` naming `cwd`. Nothing
/// a caller passes can bring a removed variable back.
#[must_use]
pub fn command_env(
    inherited: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    removed: &[String],
    cwd: &std::ffi::OsStr,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    env::starting_in(
        env::build(inherited, removed, cfg!(windows)),
        cwd,
        cfg!(windows),
    )
}

/// The settings of the `shell` tool: configuration's `[shell]` table, less
/// the program and its arguments, which choose the [`Shell`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellConfig {
    /// The time limit of a command that gives none.
    pub timeout: Duration,
    /// The longest time limit a command may ask for.
    pub max_timeout: Duration,
    /// The environment variables a command is not given, besides
    /// [`DEFAULT_REMOVED`], which are never passed on. A name may end with
    /// `*`.
    pub env_remove: Vec<String>,
    /// Whether to look, before and after a command, for the files it
    /// changed.
    pub detect_changes: bool,
    /// Whether the project is trusted, and so whether that look may ask
    /// git ([`ChangeDetector::trusting_the_repository`]). Without trust it
    /// goes by modification times.
    pub repository_trusted: bool,
    /// Where a command that names no `cwd` runs: a directory relative to
    /// the workspace root, with `/` between names (the directory the
    /// person started in, since the follow-ups of step 23). `None`, or one
    /// that is no longer a directory: the root.
    pub start: Option<String>,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_timeout: DEFAULT_MAX_TIMEOUT,
            env_remove: Vec::new(),
            detect_changes: true,
            repository_trusted: false,
            start: None,
        }
    }
}

/// What of the `shell` tool's settings a reload of the configuration
/// changes while its session runs (step 24): whether the project is
/// trusted, so whether the look at what a command changed may ask git, and
/// the providers' key variables, which no command is given. Both take
/// effect for the next command.
///
/// The key variables are a [`Withheld`] the harness's own git holds too
/// ([`ShellLive::withheld`]), so that one reload changes what both
/// withhold and neither keeps an old copy (the harness's own processes'
/// fixes).
#[derive(Debug, Default)]
pub struct ShellLive {
    repository_trusted: std::sync::atomic::AtomicBool,
    key_variables: Arc<Withheld>,
}

impl ShellLive {
    /// The settings a session starts with.
    #[must_use]
    pub fn new(repository_trusted: bool, key_variables: Vec<String>) -> Self {
        Self::sharing(repository_trusted, Arc::new(Withheld::new(key_variables)))
    }

    /// The settings a session starts with, the key variables being a list
    /// the session's other readers hold too.
    #[must_use]
    pub fn sharing(repository_trusted: bool, key_variables: Arc<Withheld>) -> Self {
        Self {
            repository_trusted: std::sync::atomic::AtomicBool::new(repository_trusted),
            key_variables,
        }
    }

    /// The key variables as a value other readers can hold: what
    /// [`ShellLive::set_key_variables`] sets, they read.
    #[must_use]
    pub fn withheld(&self) -> Arc<Withheld> {
        Arc::clone(&self.key_variables)
    }

    /// Whether the project is trusted now.
    #[must_use]
    pub fn repository_trusted(&self) -> bool {
        self.repository_trusted
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// The project trusted, or no longer.
    pub fn set_repository_trusted(&self, trusted: bool) {
        self.repository_trusted
            .store(trusted, std::sync::atomic::Ordering::Release);
    }

    /// The providers' key variables now.
    #[must_use]
    pub fn key_variables(&self) -> Vec<String> {
        self.key_variables.get()
    }

    /// The providers' key variables, in place of those there were.
    pub fn set_key_variables(&self, variables: Vec<String>) {
        self.key_variables.set(variables);
    }
}

/// What the project knows about a command
/// (`docs/architecture/14-adaptation.md`). The project profile implements
/// it; a session without one keeps the beginning and the end of every long
/// output.
pub trait CommandProfile: Send + Sync + std::fmt::Debug {
    /// Whether `command` is one of the project's builds or test runs. The
    /// lines of its output that report a problem are then kept ahead of
    /// the routine ones.
    fn is_build_or_test(&self, command: &str) -> bool;
}

/// The `shell` tool.
#[derive(Debug)]
pub struct ShellTool {
    spec: ToolSpec,
    shell: Shell,
    config: ShellConfig,
    runner: Runner,
    launcher: Arc<dyn Launcher>,
    profile: Option<Arc<dyn CommandProfile>>,
    /// The environment commands inherit from, when it is not the
    /// process's own.
    inherited: Option<Vec<(std::ffi::OsString, std::ffi::OsString)>>,
    /// The detector of the workspace the tool last ran in, and whether it
    /// may ask git. It remembers, for one workspace, that git is not
    /// installed.
    detector: Mutex<Option<(AbsPath, bool, Arc<ChangeDetector>)>>,
    /// What a reload changes while the session runs.
    live: Option<Arc<ShellLive>>,
    /// Where the harness's own git runs when it looks for what a command
    /// changed.
    git: Arc<dyn Confinement>,
}

impl ShellTool {
    /// The tool for `shell`, with nothing between a command and the
    /// system: no sandbox, no guard, the runtime's timer.
    ///
    /// The description the model is given names the shell and its version,
    /// so the shell is found, and asked its version, before this is called
    /// ([`ShellSearch::find`], [`Shell::probe_version`]).
    #[must_use]
    pub fn new(shell: Shell, config: ShellConfig) -> Self {
        let spec = described(&shell, &config, None);
        Self {
            spec,
            shell,
            config,
            runner: Runner::new(),
            launcher: Arc::new(Direct),
            profile: None,
            inherited: None,
            detector: Mutex::new(None),
            git: Arc::new(AsItIs),
            live: None,
        }
    }

    /// Sets what a reload of the configuration changes while the session
    /// runs, in place of the trust in [`ShellConfig`] and no key variables
    /// beyond its `env_remove`.
    #[must_use]
    pub fn with_live(mut self, live: Arc<ShellLive>) -> Self {
        self.live = Some(live);
        self
    }

    /// Whether the look at what a command changed may ask git now.
    fn repository_trusted(&self) -> bool {
        self.live
            .as_ref()
            .map_or(self.config.repository_trusted, |live| {
                live.repository_trusted()
            })
    }

    /// The variables no command is given, besides [`DEFAULT_REMOVED`]: the
    /// configured ones and the providers' keys as they are now.
    fn removed(&self) -> Vec<String> {
        let mut removed = self.config.env_remove.clone();
        if let Some(live) = &self.live {
            removed.extend(live.key_variables());
        }
        removed
    }

    /// Sets the runner: its timer, its clock, its guard and its graces.
    #[must_use]
    pub fn with_runner(mut self, runner: Runner) -> Self {
        self.runner = runner;
        self
    }

    /// Sets what starts a command: the sandbox launcher, in place of
    /// [`Direct`].
    #[must_use]
    pub fn with_launcher(mut self, launcher: Arc<dyn Launcher>) -> Self {
        self.launcher = launcher;
        self
    }

    /// Sets where the harness's own git runs when it looks for what a
    /// command changed, in place of [`AsItIs`]: the session's sandbox,
    /// which confines it while commands are confined
    /// ([`ChangeDetector::with_git`]). It is not the launcher's call: a
    /// command the user let leave the sandbox is still looked at by a git
    /// inside it.
    #[must_use]
    pub fn with_git(mut self, git: Arc<dyn Confinement>) -> Self {
        self.git = git;
        self
    }

    /// Sets what the project knows about commands.
    #[must_use]
    pub fn with_profile(mut self, profile: Arc<dyn CommandProfile>) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Sets the environment commands inherit from, in place of the
    /// process's own. What is removed from it and added to it is the same
    /// either way (`docs/architecture/05-tools.md`, "`shell`",
    /// "Environment"). The harness leaves this alone; a test of what a
    /// command is given sets it, because a test cannot change its own
    /// process's environment.
    #[must_use]
    pub fn with_inherited_env(
        mut self,
        inherited: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    ) -> Self {
        let inherited: Vec<_> = inherited.into_iter().collect();
        self.spec = described(&self.shell, &self.config, Some(&inherited));
        self.inherited = Some(inherited);
        self
    }

    /// The shell commands run in.
    #[must_use]
    pub fn shell(&self) -> &Shell {
        &self.shell
    }

    fn detector(&self, root: &AbsPath) -> Arc<ChangeDetector> {
        let trusted = self.repository_trusted();
        let mut held = self.detector.lock().unwrap_or_else(PoisonError::into_inner);
        match &*held {
            Some((known, was, detector)) if known == root && *was == trusted => {
                Arc::clone(detector)
            }
            _ => {
                let detector = Arc::new(
                    ChangeDetector::new(root.to_native_string())
                        .trusting_the_repository(trusted)
                        .with_git(Arc::clone(&self.git)),
                );
                *held = Some((root.clone(), trusted, Arc::clone(&detector)));
                detector
            }
        }
    }
}

/// The tool's definition: fixed for as long as the tool exists.
/// The tool's description and arguments for commands that inherit
/// `inherited`, or the harness's own environment: whether it says that
/// `PATH` was replaced is read from the environment a command is built
/// with ([`env::path_replaced`]).
fn described(
    shell: &Shell,
    config: &ShellConfig,
    inherited: Option<&[(std::ffi::OsString, std::ffi::OsString)]>,
) -> ToolSpec {
    let own: Vec<_>;
    let inherited = match inherited {
        Some(inherited) => inherited,
        None => {
            own = std::env::vars_os().collect();
            &own
        }
    };
    let replaced = env::path_replaced(inherited, &config.env_remove, cfg!(windows));
    spec(shell, config, replaced)
}

/// The tool's description and arguments. `path_replaced`: the harness's
/// `PATH` names no absolute directory, so a command is given
/// [`env::NO_ABSOLUTE_PATH`] in its place, and the model is told why a
/// program it names is not found.
fn spec(shell: &Shell, config: &ShellConfig, path_replaced: bool) -> ToolSpec {
    let system = if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    };
    let syntax = match shell.kind() {
        ShellKind::Pwsh | ShellKind::WindowsPowerShell => {
            " Write PowerShell: `;` between commands, `$env:NAME` for an environment variable."
        }
        ShellKind::Cmd => {
            " Write one line of cmd.exe syntax: `&` or `&&` between commands, `%NAME%` for an \
             environment variable."
        }
        _ => "",
    };
    // Every command is given `NoDefaultCurrentDirectoryInExePath` on
    // Windows (`cineris_base::child_env::NO_CURRENT_DIRECTORY`), so a file of the directory a
    // command runs in is found only by its path, from a batch file too.
    let here = if cfg!(windows) {
        " A program or batch file in the directory the command runs in is found only by its \
         path (`.\\build.cmd`), not by its name alone; a batch file calls another of its own \
         folder as `call \"%~dp0other.cmd\"`."
    } else {
        ""
    };
    let default_ms = millis(config.timeout);
    let max_ms = millis(config.max_timeout).max(1);
    // Started at the root, the text is what it always was: the request's
    // prefix, and so its cache, is unchanged.
    let (runs_in, cwd_default) = match &config.start {
        None => (
            "the workspace root".to_owned(),
            "The directory to run in. Defaults to the workspace root.".to_owned(),
        ),
        Some(start) => {
            let start = cineris_base::visible(start);
            (
                format!("`{start}` (relative to the workspace root)"),
                format!("The directory to run in. Defaults to `{start}`."),
            )
        }
    };
    // Said only when it holds, so that the text, and the request's cache,
    // is what it always was otherwise.
    let path = if path_replaced {
        format!(
            " The harness was started with no absolute directory in `PATH`, so a command is \
             given `PATH={}` in its place: a program elsewhere must be run by its full \
             path.",
            env::NO_ABSOLUTE_PATH
        )
    } else {
        String::new()
    };
    ToolSpec::new(
        "shell",
        format!(
            "Run a command in {} on {system}, and get back what it printed (standard output and \
             standard error together, in order) and its exit code.{syntax} The command runs in \
             {runs_in} unless `cwd` names another directory. It has no terminal and no \
             input, so it cannot ask a question: pass the flags that make a tool run without \
             one. It is stopped when it runs longer than `timeout_ms`. Nothing is left running: \
             a process the command starts in the background is stopped when the command exits, \
             because background processes are not supported yet.{here}{path}",
            shell.name()
        ),
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The command line."
                },
                "cwd": {
                    "type": "string",
                    "minLength": 1,
                    "description": cwd_default
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": max_ms,
                    "description": format!(
                        "The time limit in milliseconds. {default_ms} unless given, and at most {max_ms}."
                    )
                },
                "sandbox": {
                    "type": "boolean",
                    "description": "Leave out, or true, to run in the sandbox. false runs the \
                                    command outside it, with the user's full rights, after the \
                                    user approves: only for a command the sandbox blocked."
                }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
    )
    .permission(RuleAction::Ask)
    .concurrency(ConcurrencyClass::Exclusive)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Deserialize)]
struct Arguments {
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    sandbox: Option<bool>,
}

/// A validated call.
#[derive(Debug, Clone)]
struct Call {
    command: String,
    line: CommandLine,
    cwd: ResolvedPath,
    timeout: Duration,
    /// Whether the command is started through the launcher: false when
    /// the model asked to leave the sandbox.
    confined: bool,
}

impl Tool for ShellTool {
    fn preview(&self, _plan: &Plan, _cx: &RunCx) -> Option<crate::Preview> {
        // It changes no file.
        None
    }

    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn plan(&self, args: &Json, cx: &PlanCx) -> Result<Plan, ToolError> {
        let arguments: Arguments = args::parse(&self.spec, args)?;
        // A command that would not reach the shell as it was written is
        // refused here, before anyone is asked to approve it.
        let line = self
            .shell
            .command_line(&arguments.command)
            .map_err(|refusal| ToolError::invalid(refusal.to_string()))?;
        let workspace = cx.paths.resolve(".")?;
        let cwd = match (&arguments.cwd, &self.config.start) {
            (Some(cwd), _) => cx.paths.resolve(cwd)?,
            // Where the session started, while it is a directory inside
            // the workspace; the root otherwise.
            (None, Some(start)) => cx
                .paths
                .resolve(start)
                .ok()
                .filter(is_directory)
                .unwrap_or_else(|| workspace.clone()),
            (None, None) => workspace.clone(),
        };
        if !is_directory(&cwd) {
            return Err(ToolError::invalid(format!(
                "`cwd` is not a directory: {}. Give a directory that exists, or leave `cwd` \
                 out to run in the workspace root.",
                cwd.shown()
            )));
        }
        let timeout = arguments
            .timeout_ms
            .map_or(self.config.timeout, Duration::from_millis)
            .min(self.config.max_timeout);

        let confined = arguments.sandbox != Some(false);
        let mut summary = format!("Run `{}`", summary_of(&arguments.command));
        if cwd != workspace {
            summary.push_str(&format!(" in {}", cwd.shown()));
        }
        if !confined {
            summary.push_str(" outside the sandbox");
        }
        let mut plan = Plan::new(
            summary,
            Call {
                command: arguments.command.clone(),
                line,
                cwd: cwd.clone(),
                timeout,
                confined,
            },
        )
        .with_effect(Effect::Exec {
            command: arguments.command,
            cwd: cwd.clone(),
        })
        .with_effect(Effect::Write(workspace))
        .with_concurrency(Concurrency::Exclusive);
        if !cwd.is_inside_workspace() {
            plan = plan.with_effect(Effect::Write(cwd));
        }
        if !confined {
            plan = plan.with_effect(Effect::Unconfined);
        }
        Ok(plan)
    }

    fn run<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        out: &'a mut dyn ToolOutput,
    ) -> BoxFuture<'a, ToolResult> {
        self.run_through(plan, cx, out, Arc::clone(&self.launcher))
    }
}

impl ShellTool {
    /// Runs a planned command as [`Tool::run`] does, started through
    /// `launcher` in place of the tool's own ([`ShellTool::with_launcher`]).
    /// For a caller that decided, once and before this, how the command is
    /// confined and so how it is looked at: the start then follows that
    /// decision, not whatever the session's level is at the moment the
    /// command reaches its launcher (the third security fix). A call the
    /// user let leave the sandbox is started as it is, whatever `launcher`
    /// is.
    pub fn run_through<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        out: &'a mut dyn ToolOutput,
        launcher: Arc<dyn Launcher>,
    ) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            // A plan the permission engine marked read-only is a command it
            // vouches changes nothing: there is nothing to look for.
            let looks = self.config.detect_changes && plan.concurrency != Concurrency::ReadOnly;
            let call = match plan.into_args::<Call>() {
                Ok(call) => call,
                Err(error) => return error.into_result(),
            };

            // A call that was cancelled while it waited its turn starts
            // nothing.
            if cx.cancel.is_cancelled() {
                return ToolResult::interrupted(
                    "`shell` was interrupted before the command started.",
                );
            }
            let detector = looks.then(|| self.detector(cx.paths.root()));
            let before = match &detector {
                Some(detector) => {
                    let detector = Arc::clone(detector);
                    match run_blocking(&cx.cancel, move |_| detector.before()).await {
                        Ok(before) => Some(before),
                        Err(BlockingError::Cancelled) => {
                            return ToolResult::interrupted(
                                "`shell` was interrupted before the command started.",
                            );
                        }
                        Err(BlockingError::Panicked(_)) => None,
                    }
                }
                None => None,
            };

            // A call the user let leave the sandbox is started as it is.
            let launcher: Arc<dyn Launcher> = if call.confined {
                launcher
            } else {
                Arc::new(Direct)
            };
            let launch =
                match launcher.launch(call.line, &LaunchCx::new(cx.paths.root(), call.cwd.abs())) {
                    Ok(launch) => launch,
                    Err(reason) => {
                        return ToolResult::error(format!("The command was not started: {reason}"));
                    }
                };
            // The plain spelling: a program is not given the form of
            // path that only file calls understand.
            let cwd: std::path::PathBuf = call.cwd.abs().to_native_string().into();
            let start = Start {
                launch,
                cwd: cwd.clone(),
                env: env::starting_in(
                    match &self.inherited {
                        Some(inherited) => {
                            env::build(inherited.iter().cloned(), &self.removed(), cfg!(windows))
                        }
                        None => env::build(std::env::vars_os(), &self.removed(), cfg!(windows)),
                    },
                    cwd.as_os_str(),
                    cfg!(windows),
                ),
                timeout: call.timeout,
                label: ARTIFACT_LABEL.to_owned(),
            };
            let finished = match self
                .runner
                .run_until(start, &cx.cancel, &cx.kill, Arc::clone(&cx.store), out)
                .await
            {
                Ok(finished) => finished,
                Err(error) => {
                    return ToolResult::error(format!(
                        "The command was not started: {error}. The shell is {}.",
                        self.shell.name()
                    ));
                }
            };

            // The command has run, whatever became of the call since: what
            // it changed is looked for, and its result is written, even if
            // the call was cancelled meanwhile.
            let free = CancelToken::new();
            let changes = match (detector, before) {
                (Some(detector), Some(before)) => {
                    run_blocking(&free, move |_| detector.after(&before))
                        .await
                        .ok()
                }
                _ => None,
            };

            let keep = match &self.profile {
                Some(profile) if profile.is_build_or_test(&call.command) => Keep::Important,
                _ => Keep::HeadAndTail,
            };
            let mut payload = payload(&self.shell, &finished, changes.as_ref());
            let incomplete = changes.as_ref().and_then(|changes| changes.incomplete);
            let limits = cx.limits;
            let store = Arc::clone(&cx.store);
            // What the launcher says its confinement blocked goes in the
            // payload too, for a caller that must tell a command the
            // sandbox stopped from one that failed (the verification
            // stage, step 19).
            let explained: Arc<Mutex<Option<String>>> = Arc::default();
            let seen = Arc::clone(&explained);
            let explain = move |exit: Option<Exit>, output: &str| {
                let line = launcher.explain(exit, output);
                seen.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone_from(&line);
                line
            };
            let fallback = finished.clone();
            let written = run_blocking(&free, move |_| {
                write_result(
                    &finished,
                    &limits,
                    keep,
                    store.as_ref(),
                    &explain,
                    incomplete,
                )
            })
            .await;
            let (text, artifact) = written.unwrap_or_else(|_| {
                // The store panicked. The status is still told.
                (status_line(&fallback), None)
            });
            if let Some(line) = explained
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                && let Some(fields) = payload.as_object_mut()
            {
                fields.insert("sandbox".to_owned(), Json::String(line));
            }
            let mut result = match fallback.ending {
                Ending::Interrupted => ToolResult::interrupted(text),
                Ending::Exited if fallback.exit == Some(Exit::Code(0)) => ToolResult::success(text),
                Ending::Exited | Ending::TimedOut => ToolResult::error(text),
            }
            .with_payload(payload);
            if let Some(artifact) = artifact {
                result = result.with_artifact(artifact);
            }
            result
        })
    }
}

/// Whether a resolved path is an existing directory.
#[allow(
    clippy::disallowed_methods,
    reason = "planning may ask the file system where a path leads, and is run off the core thread"
)]
fn is_directory(path: &ResolvedPath) -> bool {
    path.exists() && std::fs::metadata(path.to_fs_path()).is_ok_and(|metadata| metadata.is_dir())
}

/// The first line of a command, shortened, for a summary.
fn summary_of(command: &str) -> String {
    let mut lines = command
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let first = lines.next().unwrap_or("");
    let more = lines.next().is_some();
    let mut shown: String = first.chars().take(SUMMARY_CHARS).collect();
    if more || first.chars().count() > SUMMARY_CHARS {
        shown.push('…');
    }
    shown
}

/// The structured payload of a result, for front ends.
fn payload(shell: &Shell, finished: &Finished, changes: Option<&Changes>) -> Json {
    let (exit_code, signal) = match finished.exit {
        Some(Exit::Code(code)) => (Some(code), None),
        Some(Exit::Signal(signal)) => (None, Some(signal)),
        None => (None, None),
    };
    json!({
        "shell": shell.name(),
        "exit_code": exit_code,
        "signal": signal,
        "duration_ms": millis(finished.duration),
        "timed_out": finished.ending == Ending::TimedOut,
        "interrupted": finished.ending == Ending::Interrupted,
        "stopped_with": finished.stopped_with.map(|stage| match stage {
            Stop::Interrupt => "interrupt",
            Stop::Terminate => "terminate",
            Stop::Kill => "kill",
        }),
        "background_stopped": finished.left_running,
        "outlived": finished.outlived,
        "escaped": finished.escaped,
        "output_lost": finished.output_lost,
        "output_bytes": finished.output.total_bytes,
        "changes": changes.map(Changes::to_json),
    })
}

/// The model-visible text of a result, and the artifact that holds the
/// whole output when anything of it was cut. It blocks when it stores one.
fn write_result(
    finished: &Finished,
    limits: &OutputLimits,
    keep: Keep,
    store: &dyn ContentStore,
    explain: &dyn Fn(Option<Exit>, &str) -> Option<String>,
    incomplete: Option<Incomplete>,
) -> (String, Option<ArtifactRef>) {
    let kept = finished.output.text();
    let mut closing = vec![status_line(finished)];
    if finished.left_running {
        closing.push(
            "[The command left processes running, and they were stopped. Background processes \
             are not supported yet: a command must finish what it starts.]"
                .to_owned(),
        );
    }
    if finished.outlived {
        closing.push(
            "[A process the command started is still running and still holds the command's \
             output. It is outside the command's process tree, so it could not be stopped.]"
                .to_owned(),
        );
    }
    if finished.escaped {
        closing.push(
            "[A process the command started kept forking and could not all be killed within the \
             bound. Something of the command may still be running inside the sandbox, and may \
             change a protected path after this check — a git hook (`.git/hooks`), git's \
             configuration (`.git/config`), a CI file (`.github/workflows`), `.cineris` or another \
             watched name — which would run later, outside the sandbox. Look at those paths \
             before trusting the workspace. The turn stops here.]"
                .to_owned(),
        );
    }
    if finished.output_lost {
        closing.push(
            "[What the command printed could not be collected, so none of it is shown. The \
             command itself ran.]"
                .to_owned(),
        );
    }
    if let Some(line) = explain(finished.exit, &kept.text) {
        closing.push(line);
    }
    if let Some(why) = incomplete {
        closing.push(format!(
            "[The list of files the command changed is incomplete: {}. Undoing this turn may not \
             put back every file the command changed.]",
            why.as_str()
        ));
    }

    // The limits less the lines added here, and less the notice when the
    // buffer has already dropped the middle: the shaping then may find
    // nothing more to cut, and would leave the notice no room.
    let cut_by_buffer = kept.dropped_bytes > 0;
    let closing_bytes: usize = closing.iter().map(|line| line.len() + 1).sum();
    let notice_bytes = if cut_by_buffer { 640 } else { 0 };
    let inner = OutputLimits::new(
        limits
            .max_bytes()
            .saturating_sub(closing_bytes + notice_bytes),
        limits
            .max_lines()
            .saturating_sub(closing.len() + usize::from(cut_by_buffer)),
        limits.max_line_chars(),
    );
    let mut shaped = output::shape(&kept.text, &inner, keep, Compression::Structural);
    if cut_by_buffer {
        // What the buffer dropped is missing too, and the marker line that
        // stands for it is not a line of the output.
        let dropped = usize::try_from(kept.dropped_lines).unwrap_or(usize::MAX);
        if dropped > 0 {
            shaped.omitted_lines = shaped.omitted_lines.saturating_add(dropped);
            shaped.total_lines = shaped.total_lines.saturating_add(dropped).saturating_sub(1);
        } else {
            // One line longer than the buffer.
            shaped.clipped_lines += 1;
        }
    }

    let total = usize::try_from(finished.output.total_bytes).unwrap_or(usize::MAX);
    let stored = match (&finished.output.artifact, shaped.is_lossy()) {
        // Written as the output arrived.
        (Some(stored), _) => Some(stored.clone()),
        // The buffer held all of it and the limits did not.
        (None, true) => Some(
            store
                .put_artifact(
                    ARTIFACT_LABEL,
                    "text/plain",
                    finished.output.whole().unwrap_or(&[]),
                )
                .map_err(|error| error.to_string()),
        ),
        (None, false) => None,
    };
    let notice = stored
        .as_ref()
        .and_then(|stored| shaped.notice(total, stored.as_ref().map_err(String::as_str)));

    let mut text = shaped.text;
    if text.is_empty() && finished.output.total_bytes == 0 && !finished.output_lost {
        text.push_str("(no output)");
    }
    for line in notice.iter().chain(&closing) {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line);
    }
    (text, stored.and_then(Result::ok))
}

/// The line that says how a command ended.
fn status_line(finished: &Finished) -> String {
    let exit = match finished.exit {
        Some(Exit::Code(code)) => exit_code(code),
        Some(Exit::Signal(signal)) => match signal_name(signal) {
            Some(name) => format!("killed by signal {signal} ({name})"),
            None => format!("killed by signal {signal}"),
        },
        None => "exit status unknown".to_owned(),
    };
    let took = duration(finished.duration);
    match finished.ending {
        Ending::Exited => format!("[{exit}, after {took}]"),
        Ending::TimedOut => format!("[timed out after {took} and was stopped: {exit}]"),
        Ending::Interrupted => format!("[interrupted after {took} and was stopped: {exit}]"),
    }
}

/// An exit code as text. A code Windows gives for a crash is a large
/// number that people know by its hexadecimal form.
fn exit_code(code: i32) -> String {
    if (0..=255).contains(&code) {
        format!("exit code {code}")
    } else {
        format!(
            "exit code {code} (0x{:08X})",
            u32::from_ne_bytes(code.to_ne_bytes())
        )
    }
}

/// The names of the signals a command is most often ended by.
fn signal_name(signal: i32) -> Option<&'static str> {
    Some(match signal {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        _ => return None,
    })
}

/// A duration for people: `0.4 s`, `12.5 s`, `95 s`, `3 min 20 s`.
fn duration(duration: Duration) -> String {
    let tenths = duration.as_millis() / 100;
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{}.{} s", tenths / 10, tenths % 10)
    } else if seconds < 180 {
        format!("{seconds} s")
    } else {
        format!("{} min {} s", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Captured;
    use crate::store::MemoryStore;

    fn finished(output: &[u8], exit: Option<Exit>, ending: Ending) -> Finished {
        Finished {
            exit,
            ending,
            stopped_with: None,
            left_running: false,
            outlived: false,
            escaped: false,
            output_lost: false,
            duration: Duration::from_millis(1234),
            output: Captured {
                head: output.to_vec(),
                tail: Vec::new(),
                total_bytes: output.len() as u64,
                total_breaks: output.iter().filter(|byte| **byte == b'\n').count() as u64,
                artifact: None,
            },
        }
    }

    fn write(
        finished: &Finished,
        limits: &OutputLimits,
    ) -> (String, Option<ArtifactRef>, MemoryStore) {
        let store = MemoryStore::new();
        let (text, artifact) = write_result(
            finished,
            limits,
            Keep::HeadAndTail,
            &store,
            &|_, _| None,
            None,
        );
        (text, artifact, store)
    }

    /// What a reload changes reaches the next command (step 24): a key
    /// variable named since is taken from it, beside the configured ones,
    /// and the look at what it changes asks git only while the project is
    /// trusted, its detector made again when that changes.
    #[test]
    fn a_reload_reaches_the_next_command() {
        let live = Arc::new(ShellLive::new(true, vec!["OLD_KEY".to_owned()]));
        let tool = ShellTool::new(
            Shell::new("/bin/sh"),
            ShellConfig {
                env_remove: vec!["MINE".to_owned()],
                ..ShellConfig::default()
            },
        )
        .with_live(Arc::clone(&live));
        assert_eq!(tool.removed(), ["MINE", "OLD_KEY"]);
        assert!(tool.repository_trusted());
        live.set_key_variables(vec!["NEW_KEY".to_owned()]);
        live.set_repository_trusted(false);
        assert_eq!(tool.removed(), ["MINE", "NEW_KEY"]);
        assert!(!tool.repository_trusted());

        let root = AbsPath::parse(
            if cfg!(windows) { "C:\\work" } else { "/work" },
            cineris_base::PathStyle::NATIVE,
        )
        .unwrap();
        live.set_repository_trusted(true);
        let trusted = tool.detector(&root);
        assert!(
            Arc::ptr_eq(&trusted, &tool.detector(&root)),
            "made again for nothing"
        );
        live.set_repository_trusted(false);
        assert!(
            !Arc::ptr_eq(&trusted, &tool.detector(&root)),
            "the detector of a trusted project was kept"
        );
    }

    /// A harness with no absolute directory in its `PATH` gives commands
    /// one that finds nothing, and the model is told so, and to name a
    /// program by its full path; otherwise the text says nothing of it.
    #[test]
    fn the_model_is_told_when_path_is_replaced() {
        let told = |path: Option<&str>| {
            let inherited: Vec<(std::ffi::OsString, std::ffi::OsString)> = path
                .map(|path| ("PATH".into(), path.into()))
                .into_iter()
                .collect();
            ShellTool::new(Shell::new("bash"), ShellConfig::default())
                .with_inherited_env(inherited)
                .spec()
                .description
                .contains(&format!("`PATH={}`", env::NO_ABSOLUTE_PATH))
        };
        let windows = cfg!(windows);
        for path in [Some("."), Some(""), Some("bin:./x"), None] {
            assert_eq!(told(path), !windows, "{path:?}");
        }
        let absolute = if windows { "C:\\Windows" } else { ".:/usr/bin" };
        assert!(!told(Some(absolute)));
    }

    /// On Windows the model is told that a file of the directory a command
    /// runs in is found only by its path, and how a batch file calls
    /// another; elsewhere the text says nothing of it.
    #[test]
    fn the_model_is_told_how_a_file_where_the_command_runs_is_found() {
        let described = spec(&Shell::new("bash"), &ShellConfig::default(), false).description;
        assert_eq!(
            described.contains("call \"%~dp0other.cmd\""),
            cfg!(windows),
            "{described}"
        );
    }

    /// Started at the root, the tool's text is what it always was, to the
    /// byte (the request's prefix and its cache); started below it, the
    /// text and `cwd`'s say where a command runs (the follow-ups of step
    /// 23).
    #[test]
    fn the_tools_text_names_where_commands_start_only_below_the_root() {
        let at_root = spec(&Shell::new("bash"), &ShellConfig::default(), false);
        assert!(
            at_root.description.contains(
                "The command runs in the workspace root unless `cwd` names another directory."
            ),
            "{}",
            at_root.description
        );
        assert_eq!(
            at_root.parameters["properties"]["cwd"]["description"],
            "The directory to run in. Defaults to the workspace root."
        );
        let below = spec(
            &Shell::new("bash"),
            &ShellConfig {
                start: Some("src/deep".to_owned()),
                ..ShellConfig::default()
            },
            false,
        );
        assert!(
            below.description.contains(
                "The command runs in `src/deep` (relative to the workspace root) unless `cwd` \
                 names another directory."
            ),
            "{}",
            below.description
        );
        assert_eq!(
            below.parameters["properties"]["cwd"]["description"],
            "The directory to run in. Defaults to `src/deep`."
        );
        // Nothing else differs.
        assert_eq!(
            below.description.replace(
                "`src/deep` (relative to the workspace root)",
                "the workspace root"
            ),
            at_root.description
        );
    }

    #[test]
    fn a_result_ends_with_how_the_command_ended() {
        let limits = OutputLimits::default();
        let (text, artifact, _) = write(
            &finished(b"hello\nworld\n", Some(Exit::Code(0)), Ending::Exited),
            &limits,
        );
        assert_eq!(text, "hello\nworld\n[exit code 0, after 1.2 s]");
        assert_eq!(artifact, None);

        let (text, _, _) = write(&finished(b"", Some(Exit::Code(3)), Ending::Exited), &limits);
        assert_eq!(text, "(no output)\n[exit code 3, after 1.2 s]");

        let (text, _, _) = write(
            &finished(b"partial", Some(Exit::Signal(15)), Ending::TimedOut),
            &limits,
        );
        assert_eq!(
            text,
            "partial\n[timed out after 1.2 s and was stopped: killed by signal 15 (SIGTERM)]"
        );
        let (text, _, _) = write(
            &finished(b"partial", Some(Exit::Signal(2)), Ending::Interrupted),
            &limits,
        );
        assert_eq!(
            text,
            "partial\n[interrupted after 1.2 s and was stopped: killed by signal 2 (SIGINT)]"
        );
        let (text, _, _) = write(&finished(b"x", None, Ending::Exited), &limits);
        assert_eq!(text, "x\n[exit status unknown, after 1.2 s]");
        let (text, _, _) = write(
            &finished(b"x", Some(Exit::Signal(64)), Ending::Exited),
            &limits,
        );
        assert_eq!(text, "x\n[killed by signal 64, after 1.2 s]");
    }

    #[test]
    fn an_exit_code_of_a_crash_is_shown_in_both_forms() {
        assert_eq!(exit_code(0), "exit code 0");
        assert_eq!(exit_code(255), "exit code 255");
        assert_eq!(
            exit_code(i32::from_ne_bytes(0xC000_013A_u32.to_ne_bytes())),
            "exit code -1073741510 (0xC000013A)"
        );
        assert_eq!(exit_code(1000), "exit code 1000 (0x000003E8)");
    }

    #[test]
    fn durations_read_as_people_say_them() {
        assert_eq!(duration(Duration::from_millis(0)), "0.0 s");
        assert_eq!(duration(Duration::from_millis(449)), "0.4 s");
        assert_eq!(duration(Duration::from_millis(59_949)), "59.9 s");
        assert_eq!(duration(Duration::from_secs(95)), "95 s");
        assert_eq!(duration(Duration::from_secs(200)), "3 min 20 s");
    }

    #[test]
    fn what_was_left_running_is_said() {
        let mut ended = finished(b"started\n", Some(Exit::Code(0)), Ending::Exited);
        ended.left_running = true;
        let (text, _, _) = write(&ended, &OutputLimits::default());
        assert!(text.starts_with("started\n[exit code 0, after 1.2 s]\n[The command left"));
        assert!(
            text.contains("Background processes are not supported yet"),
            "{text}"
        );
        ended.left_running = false;
        ended.outlived = true;
        let (text, _, _) = write(&ended, &OutputLimits::default());
        assert!(text.contains("could not be stopped"), "{text}");
    }

    #[test]
    fn output_that_could_not_be_collected_is_not_called_no_output() {
        let mut ended = finished(b"", Some(Exit::Code(0)), Ending::Exited);
        ended.output_lost = true;
        let (text, artifact, _) = write(&ended, &OutputLimits::default());
        assert_eq!(
            text,
            "[exit code 0, after 1.2 s]\n[What the command printed could not be collected, so \
             none of it is shown. The command itself ran.]"
        );
        assert_eq!(artifact, None);
    }

    #[test]
    fn a_list_of_changes_that_is_incomplete_is_said_with_why() {
        let store = MemoryStore::new();
        let ended = finished(b"done\n", Some(Exit::Code(0)), Ending::Exited);
        let (text, _) = write_result(
            &ended,
            &OutputLimits::default(),
            Keep::HeadAndTail,
            &store,
            &|_, _| None,
            Some(Incomplete::GitFailedConfined),
        );
        assert!(
            text.ends_with(&format!(
                "[The list of files the command changed is incomplete: {}. Undoing this turn may \
                 not put back every file the command changed.]",
                Incomplete::GitFailedConfined.as_str()
            )),
            "{text}"
        );
        let (text, _) = write_result(
            &ended,
            &OutputLimits::default(),
            Keep::HeadAndTail,
            &store,
            &|_, _| None,
            None,
        );
        assert!(!text.contains("incomplete"), "{text}");
    }

    #[test]
    fn a_command_that_outran_its_kill_is_said_with_where_to_look() {
        let store = MemoryStore::new();
        let ended = Finished {
            escaped: true,
            ..finished(b"ok\n", Some(Exit::Code(0)), Ending::Exited)
        };
        let (text, _) = write_result(
            &ended,
            &OutputLimits::default(),
            Keep::HeadAndTail,
            &store,
            &|_, _| None,
            None,
        );
        // It warns, and names the watched paths to look at.
        assert!(
            text.contains("kept forking and could not all be killed"),
            "{text}"
        );
        assert!(
            text.contains(".git/hooks") && text.contains(".git/config"),
            "{text}"
        );
        assert!(text.contains("The turn stops here"), "{text}");
    }

    #[test]
    fn the_launcher_may_add_a_line_about_what_it_blocked() {
        let store = MemoryStore::new();
        let ended = finished(b"denied\n", Some(Exit::Code(1)), Ending::Exited);
        let (text, _) = write_result(
            &ended,
            &OutputLimits::default(),
            Keep::HeadAndTail,
            &store,
            &|exit, output| {
                assert_eq!(exit, Some(Exit::Code(1)));
                assert_eq!(output, "denied\n");
                Some("[The sandbox refused a write outside the workspace.]".to_owned())
            },
            None,
        );
        assert!(
            text.ends_with("after 1.2 s]\n[The sandbox refused a write outside the workspace.]")
        );
    }

    #[test]
    fn output_over_the_limits_is_cut_with_a_notice_and_an_artifact() {
        let limits = OutputLimits::new(4096, 40, 2000);
        let output: String = (1..=200)
            .map(|n| format!("building part {n} of the thing\n"))
            .collect();
        let (text, artifact, store) = write(
            &finished(output.as_bytes(), Some(Exit::Code(1)), Ending::Exited),
            &limits,
        );
        assert!(limits.fits(&text), "{text}");
        let artifact = artifact.expect("the whole output is kept");
        assert_eq!(
            store.artifact(&artifact.id).expect("stored"),
            output.as_bytes()
        );
        assert!(
            text.contains(&format!("is artifact {}", artifact.id)),
            "{text}"
        );
        assert!(text.ends_with("[exit code 1, after 1.2 s]"), "{text}");
        // The beginning and the end are both there.
        assert!(text.starts_with("building part 1 of the thing\n"), "{text}");
        assert!(text.contains("building part 200 of the thing\n"), "{text}");
    }

    #[test]
    fn what_the_buffer_dropped_is_counted_in_the_notice() {
        // As the runner leaves it when the output passed the buffer: the
        // two ends, the totals, and the artifact that was written.
        let store = MemoryStore::new();
        let reference = store
            .put_artifact(ARTIFACT_LABEL, "text/plain", b"(the whole output)")
            .expect("stored");
        let ended = Finished {
            output: Captured {
                head: b"first\nsecond\nthi".to_vec(),
                tail: b"enth\nlast but one\nlast\n".to_vec(),
                total_bytes: 50_000,
                total_breaks: 1_000,
                artifact: Some(Ok(reference.clone())),
            },
            ..finished(b"", Some(Exit::Code(0)), Ending::Exited)
        };
        let (text, artifact) = write_result(
            &ended,
            &OutputLimits::default(),
            Keep::HeadAndTail,
            &store,
            &|_, _| None,
            None,
        );
        assert_eq!(artifact, Some(reference));
        assert_eq!(
            text,
            "first\nsecond\n[… 996 lines not shown …]\nlast but one\nlast\n\
             [Output cut: 996 of 1,000 lines not shown. The whole output (48.8 KiB) is artifact \
             a_0001: pass that id as `artifact` to read or search.]\n\
             [exit code 0, after 1.2 s]"
        );

        // A store that could not keep it is said, and the result stands.
        let ended = Finished {
            output: Captured {
                artifact: Some(Err("the content store failed: disk full".to_owned())),
                ..ended.output.clone()
            },
            ..ended
        };
        let (text, artifact) = write_result(
            &ended,
            &OutputLimits::default(),
            Keep::HeadAndTail,
            &store,
            &|_, _| None,
            None,
        );
        assert_eq!(artifact, None);
        assert!(
            text.contains("could not be kept: the content store failed: disk full"),
            "{text}"
        );
    }

    #[test]
    fn a_summary_is_the_first_line_shortened() {
        assert_eq!(summary_of("cargo test"), "cargo test");
        assert_eq!(summary_of("  \n cargo build \nls"), "cargo build…");
        assert_eq!(summary_of(&"x".repeat(200)), format!("{}…", "x".repeat(80)));
    }
}
