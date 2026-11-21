//! `cineris.exec`: a program an extension asks the host to run
//! (`docs/steps/28/answered.md`, "The processes").
//!
//! `cineris_ext::host::serve` has already checked the program against the
//! manifest and bounded the request. Here, in this order:
//!
//! 1. **The program is found** ([`crate::programs::find`]): a bare name in
//!    `PATH`'s absolute directories outside the workspace and every
//!    writable root (on Windows `.exe`, `.cmd`, `.bat` a directory at a
//!    time, and nothing else of `PATHEXT`); an absolute path as named,
//!    refused inside a writable root while commands are confined.
//! 2. **The line the permission engine reads** is the program as named and
//!    each argument quoted in the session shell's syntax
//!    ([`crate::programs::engine_line`]); `cwd` is resolved as the shell
//!    tool resolves its own, and must be inside the workspace.
//! 3. **The call is decided** as an `exec` of that line (and a `network`
//!    to any host when the manifest has `exec_network`), beside the
//!    extension tool's `ext` effect ([`CallHost::decide`]): a question
//!    offers only "allow once" and "deny", and shows how the program's
//!    input begins, which no rule judges. The decision may wait on a
//!    person, and is given up when the call is cancelled or its time is
//!    up; the program is then given only what is left of the call's time
//!    when it starts (`ExecRequest::call_left`).
//! 4. **It is started** through a launcher of the extension's own where the
//!    sandbox enforces, at every level, `unrestricted` included
//!    ([`SessionSandbox::extension_launcher`]): the session's reads, the
//!    manifest's write roots and the session's private temporary directory,
//!    and the network only with `exec_network`. Where the sandbox does not
//!    enforce (Windows, no Landlock, the sandbox turned off) it is started
//!    as it is, as the shell's commands are. Its input reaches it either
//!    way: after the launcher's policy, or through the runner's pipe.
//! 5. **The protected paths are looked at** before and after a start that
//!    has a write root, where the sandbox enforces, as around a confined
//!    shell command (step 26): a change from outside since the last look
//!    refuses the start, and a change the program made stops the call;
//!    either stops the turn, and the script's call with it. Both looks are
//!    made whatever became of the run: ended by the call's cancellation or
//!    its time, or by a start that failed after the process was made.
//!    What the program changed in the workspace is looked at with the
//!    session's git, inside the sandbox wherever it enforces, at every
//!    level ([`SessionGit`]).
//! 6. **Its environment** is a shell command's
//!    ([`cineris_tools::shell::command_env`]): no provider key, no
//!    `shell.env_remove` variable, an absolute `PATH`; `TMPDIR` the
//!    session's private temporary directory where the launcher confines it.
//! 7. **On Windows a `.cmd` or `.bat`** is started through
//!    `%SystemRoot%\System32\cmd.exe /d /e:on /v:off /s /c` and one line
//!    of its own ([`crate::programs::batch_line`]).
//! 8. **It runs** through the session's process runner, its standard
//!    output and standard error read apart, each kept to
//!    [`EXEC_OUTPUT_CAP`] with a flag when cut.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use cineris_base::CancelToken;
use cineris_base::child_env::Withheld;
use cineris_ext::host::{EXEC_OUTPUT_CAP, ExecOutput, ExecRequest, HostError};
use cineris_sandbox::protect::{Changed, Snapshot};
use cineris_tools::process::{Argument, CommandLine, Ending, Exit, Launch, Runner, Start};
use cineris_tools::shell::ChangeDetector;
use cineris_tools::{Effect, NetworkTarget, Shell, ShellKind, ShellLive, run_blocking};
use serde_json::json;

use super::host::{CallHost, HostQuestion};
use crate::checkpoints::{Rounds, changed_since_last_look, note_changed};
use crate::programs::{self, Search};
use crate::sandbox::{SessionGit, SessionSandbox, protected_changes, protected_note};

/// What a session runs an extension's programs with.
#[derive(Debug)]
pub(crate) struct ExecParts {
    /// The session's sandbox: whether it enforces, the launcher's reads and
    /// temporary directory, and the protected paths.
    pub(crate) sandbox: Arc<SessionSandbox>,
    /// The session's process runner: the shell tool's, with its guard.
    pub(crate) runner: Runner,
    /// The session's shell, whose syntax the line the permission engine
    /// reads is written in. `None` when there is none.
    pub(crate) shell: Option<Shell>,
    /// The environment programs inherit from, when it is not this
    /// process's own (a test's).
    pub(crate) inherited: Option<Vec<(OsString, OsString)>>,
    /// The variables no program is given besides the providers' known
    /// keys: the session's `shell.env_remove` and the keys of the
    /// providers it was started with.
    pub(crate) env_remove: Vec<String>,
    /// The providers' key variables as a reload leaves them: the list the
    /// shell's commands are kept from.
    pub(crate) withheld: Arc<Withheld>,
    /// Where a program found by name is never taken from: the workspace,
    /// and the session's write roots while it would confine. Resolved.
    pub(crate) avoid: Vec<PathBuf>,
    /// The session's checkpoints, which record a protected path a program
    /// changed so that undo puts it back. `None` for a session kept in
    /// memory.
    pub(crate) rounds: Option<Arc<Rounds>>,
    /// Whether write roots hold the harness's own files, judged as the
    /// session's own roots are.
    pub(crate) harness_files: HarnessFiles,
    /// The shell's settings a reload changes: whether the look at what a
    /// program changed may ask git.
    pub(crate) live: Arc<ShellLive>,
    /// How that look's git is run: the session's, inside the sandbox
    /// wherever it enforces, at every level, as the program's own launcher
    /// confines it there at every level.
    pub(crate) git: Arc<SessionGit>,
}

/// Whether write roots hold the harness's own files: its configuration,
/// its state and data, or the launcher's program
/// (`assemble::holds_harness_files`).
#[derive(Clone)]
pub(crate) struct HarnessFiles(pub(crate) Arc<HoldsHarnessFiles>);

/// The judgement [`HarnessFiles`] holds, of write roots each in its plain
/// form ([`cineris_base::path::AbsPath::to_native_string`]): on Windows
/// `C:\…`, never the `\\?\C:\…` a file-system path is, which a root's
/// judgement cannot read. A root it cannot read counts as holding the
/// harness's files.
pub(crate) type HoldsHarnessFiles = dyn Fn(&[String]) -> bool + Send + Sync;

impl std::fmt::Debug for HarnessFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HarnessFiles")
    }
}

/// Runs `request` for the call `host` serves.
pub(crate) async fn run(
    parts: &ExecParts,
    request: ExecRequest,
    host: &CallHost<'_>,
) -> Result<ExecOutput, HostError> {
    let cx = host.cx;
    // The call's time is reckoned from here: `call_left` is what was left
    // of it when the script asked for the program.
    let arrived = host.parts.clock.monotonic();
    // A turn that a confined command stopped starts nothing more: the
    // decision below refuses every call of it (`EnginePolicy::decide`).
    let named = request.program.as_str().to_owned();
    let inherited = parts
        .inherited
        .clone()
        .unwrap_or_else(|| std::env::vars_os().collect());
    // The write roots as the launcher is given them: the workspace's real
    // path (the resolver's root is resolved) joined with each root's own
    // names, never resolved. A link on the way to one, there now or made
    // later, is then refused by the launcher's open rather than followed to
    // wherever it leads (the early review of step 28: `build -> ~` granted
    // writing the home folder).
    let workspace = cx.paths.root();
    let write_roots: Vec<PathBuf> = request
        .write_roots
        .iter()
        .map(|root| {
            let names = root.strip_prefix(workspace).ok_or_else(|| {
                HostError::Failed(format!(
                    "a write root is outside the workspace: {}",
                    root.to_native_string()
                ))
            })?;
            let mut path = workspace.to_fs_path();
            path.extend(names.names());
            Ok(path)
        })
        .collect::<Result<_, HostError>>()?;
    // Judged in their plain form: the file system's form of a Windows path
    // (`\\?\C:\…`) is one the judgement cannot read.
    let plain_roots: Vec<String> = request
        .write_roots
        .iter()
        .map(cineris_base::path::AbsPath::to_native_string)
        .collect();
    if (parts.harness_files.0)(&plain_roots) {
        return Err(HostError::Failed(
            "the extension's write roots hold the harness's own files (its configuration, its \
             state or its launcher): no program of it is run"
                .to_owned(),
        ));
    }

    // Where no program is taken from, by the search below or by the
    // program's own lookups: the workspace, the session's write roots as
    // the sandbox holds them now, and the extension's own.
    let avoid = {
        let mut avoid = parts.sandbox.avoiding_roots_now(&parts.avoid);
        avoid.extend(write_roots.iter().cloned());
        avoid
    };

    // 1. The program, and on Windows the `cmd.exe` a script needs.
    let found = {
        let program = request.program.clone();
        let path = path_of(&inherited);
        let avoid = avoid.clone();
        let confined_roots = if parts.sandbox.confines() {
            avoid.clone()
        } else {
            Vec::new()
        };
        let cmd_from = inherited.clone();
        blocking(cx, move || {
            let search = Search {
                path: &path,
                avoid: &avoid,
                confined_roots: &confined_roots,
                windows: cfg!(windows),
            };
            let found = programs::find(&program, &search)?;
            let cmd = (cfg!(windows) && cineris_base::program::is_batch_script(&found))
                .then(|| programs::system_cmd(&cmd_from, &avoid))
                .flatten();
            Ok::<_, String>((found, cmd))
        })
        .await?
        .map_err(HostError::Failed)?
    };
    let (found, cmd) = found;

    // 2. The line the engine reads, and where it runs.
    let fallback;
    let shell = match &parts.shell {
        Some(shell) => shell,
        None => {
            fallback = if cfg!(windows) {
                Shell::of_kind(ShellKind::Cmd, "cmd.exe")
            } else {
                Shell::of_kind(ShellKind::Posix, "sh")
            };
            &fallback
        }
    };
    let line = programs::engine_line(shell, &named, &request.args).map_err(HostError::Failed)?;
    // What is started, made now so that a batch script's line that cannot
    // be written (a character `cmd.exe` cannot carry) is refused before
    // anyone is asked about it.
    let command = programs::command_line(&found, &request.args, cfg!(windows), cmd.as_deref())
        .map_err(HostError::Failed)?;
    let cwd = {
        let paths = Arc::clone(&cx.paths);
        let asked = request.cwd.clone().unwrap_or_else(|| ".".to_owned());
        blocking(cx, move || {
            let cwd = paths.resolve(&asked).map_err(|error| error.to_string())?;
            if !cwd.is_inside_workspace() {
                return Err(format!(
                    "`cwd` is outside the workspace: {}. A program runs inside it.",
                    cwd.shown()
                ));
            }
            if !is_directory(&cwd.to_fs_path()) {
                return Err(format!("`cwd` is not a directory: {}", cwd.shown()));
            }
            Ok(cwd)
        })
        .await?
        .map_err(HostError::Failed)?
    };

    // 4. Through what it is started: made before the decision, which it
    // has no part in, so that the question says whether the program is
    // confined by it, whatever the session's commands are.
    let launcher = {
        let sandbox = Arc::clone(&parts.sandbox);
        let roots = write_roots.clone();
        let network = request.network;
        blocking(cx, move || sandbox.extension_launcher(&roots, network)).await?
    };

    // 3. The decision.
    let mut effects = vec![Effect::Exec {
        command: line.clone(),
        cwd: cwd.clone(),
    }];
    if request.network {
        effects.push(Effect::Network(NetworkTarget::Any));
    }
    let decision = host.decide(
        // The line as code it cannot end (the full review of step 28).
        format!("{}: run `{}`", host.full_name, cineris_base::in_code(&line)),
        json!({
            "program": named,
            "args": request.args,
            "cwd": cwd.data(),
        }),
        effects,
        None,
        HostQuestion {
            confined: Some(launcher.is_some()),
            // In PowerShell and `cmd` the line's first word may be the
            // shell's own command or alias (`write` is `Write-Output`),
            // not the program that runs; in a POSIX shell it is the
            // program, found as the shell finds it, except on Windows,
            // where Windows' lookup finds it (`sort` is
            // `System32\sort.exe`, not the shell's).
            read_only_asks: cfg!(windows)
                || !matches!(shell.kind(), ShellKind::Bash | ShellKind::Posix),
            // The engine judges the line, not the input: the question
            // shows its start.
            extra: request.stdin.as_deref().map(input_reason),
        },
    );
    // The decision is given up when the call is cancelled or its time is
    // up, and an open question with it: the engine follows a program's
    // host call to its end, and a question nobody answers would hold the
    // call, and the turn, for ever (the review of step 28's fixes).
    // Its time is reckoned from when the program was asked for, the steps
    // before the decision included, and a call already over or cancelled
    // is put no question at all (the review of step 28's fixes: one could
    // be put and withdrawn at once).
    let left = request
        .call_left
        .saturating_sub(host.parts.clock.monotonic().saturating_sub(arrived));
    let decided = if left.is_zero() || cx.cancel.is_cancelled() {
        None
    } else {
        let mut decision = std::pin::pin!(decision);
        let mut cancelled = std::pin::pin!(cx.cancel.cancelled());
        let mut over = host.parts.timer.sleep(left);
        std::future::poll_fn(|context| {
            if cancelled.as_mut().poll(context).is_ready() || over.as_mut().poll(context).is_ready()
            {
                return std::task::Poll::Ready(None);
            }
            if let std::task::Poll::Ready(decided) = decision.as_mut().poll(context) {
                return std::task::Poll::Ready(Some(decided));
            }
            std::task::Poll::Pending
        })
        .await
    };
    let not_started = || {
        HostError::Failed(format!(
            "`{named}` was not started: the call ended while it was being decided"
        ))
    };
    decided.ok_or_else(not_started)??;
    let watch = launcher
        .as_ref()
        .filter(|_| !write_roots.is_empty())
        .and_then(|_| parts.sandbox.protected());
    let launch = match &launcher {
        Some(launcher) => {
            let arguments = command
                .arguments
                .iter()
                .map(|argument| match argument {
                    Argument::Exact(text) => Ok(OsString::from(text)),
                    // Only a Windows line (a batch script's for `cmd.exe`,
                    // or a program's quoted as the C runtime reads it),
                    // where nothing is confined.
                    Argument::Verbatim(_) => Err(HostError::Failed(
                        "a command line written for Windows cannot be confined".to_owned(),
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let wrapped = match &request.stdin {
                Some(input) => launcher
                    .wrap_with_input(command.program.as_os_str(), &arguments, input)
                    .map_err(|error| HostError::Failed(error.to_string()))?,
                None => launcher.wrap(command.program.as_os_str(), &arguments),
            };
            let mut line = CommandLine::new(wrapped.program);
            line.arguments = wrapped
                .arguments
                .into_iter()
                .map(|argument| {
                    argument.into_string().map(Argument::Exact).map_err(|_| {
                        HostError::Failed(
                            "a command line that is not text cannot be confined".to_owned(),
                        )
                    })
                })
                .collect::<Result<_, _>>()?;
            Launch {
                command: line,
                stdin: Some(wrapped.stdin),
            }
        }
        // Started as it is, its input on the runner's pipe.
        None => Launch {
            command,
            stdin: request.stdin.clone(),
        },
    };

    // 5. The look before it, with the folder the repository's
    // configuration names for its hooks read first.
    let before = match &watch {
        Some(_) => {
            let (sandbox, git) = (Arc::clone(&parts.sandbox), Arc::clone(&parts.git));
            let root = cx.paths.root().to_fs_path();
            blocking(cx, move || {
                sandbox.follow_hooks(&*git, &root);
                sandbox.protected().map(|watch| watch.take())
            })
            .await?
        }
        None => None,
    };
    if let Some(before) = &before
        && parts.sandbox.confines()
        && let Some(note) = changed_since_last_look(
            &parts.sandbox,
            parts.rounds.as_ref(),
            &host.parts.policy,
            before,
            cx.turn,
            &cx.paths.root().to_fs_path(),
        )
        .await
    {
        // The turn is stopped: the script's call ends here, whatever it
        // would do with a refusal.
        return Err(HostError::Stopped(format!(
            "`{named}` was not started. {note}"
        )));
    }

    // 7. What it changes in the workspace, looked at as a shell command's
    // is: a program that may write it (one with a write root, or one
    // nothing confines) has what it changed put in the round's checkpoint,
    // so that undo puts it back, and the model's table of files told. The
    // look's git is the session's, inside the sandbox wherever it enforces,
    // at every level: a program its launcher confined may have written the
    // repository's configuration in its write roots (the review of the
    // extensions' process starts; every look since the third security fix).
    let detector = (!write_roots.is_empty() || launcher.is_none()).then(|| {
        let git = Arc::clone(&parts.git) as Arc<dyn cineris_base::git::Confinement>;
        Arc::new(
            ChangeDetector::new(cx.paths.root().to_native_string())
                .trusting_the_repository(parts.live.repository_trusted())
                .with_git(git),
        )
    });
    let changes_before = match &detector {
        Some(detector) => {
            let detector = Arc::clone(detector);
            Some(blocking(cx, move || detector.before()).await?)
        }
        None => None,
    };

    // 6. Its environment, and 8. the run.
    let mut removed = parts.env_remove.clone();
    removed.extend(parts.withheld.get());
    let directory: PathBuf = cwd.abs().to_native_string().into();
    let mut env = cineris_tools::shell::command_env(inherited, &removed, directory.as_os_str());
    // The program's own lookups (`#!/usr/bin/env node`, a shim's `node`)
    // find nothing where a program could have been planted, as the
    // harness's git's do (the early review of step 28).
    cineris_base::child_env::keep_out_of(&mut env, &avoid, cfg!(windows));
    // The decision may have waited on a person, and the looks before the
    // run taken their time: the program is given what is left of the
    // call's time just before it starts, no more, and nothing starts for a
    // call that is over.
    let spent = host.parts.clock.monotonic().saturating_sub(arrived);
    let left = request.call_left.saturating_sub(spent);
    if left.is_zero() || cx.cancel.is_cancelled() {
        return Err(not_started());
    }
    let start = Start {
        launch,
        env,
        cwd: directory,
        timeout: request.timeout.min(left),
        label: String::new(),
    };
    // Whatever becomes of the run, everything after it is done: a run the
    // call's cancellation or its time ended, and one whose start failed
    // after the process was made (a reader or the waiter not started), may
    // have changed things. The engine polls a host call to its end after
    // the call's cancellation or deadline, so that the look and the record
    // are made then too.
    let ran = parts
        .runner
        .run_apart(start, &cx.cancel, &cx.kill, EXEC_OUTPUT_CAP)
        .await;
    let escaped = ran.as_ref().is_ok_and(|finished| finished.escaped);

    // 5. The look after it, then 7.: a protected path's exact record comes
    // first, as only the first record of a file in a round counts.
    let looked = match before {
        Some(before) => look_after(parts, host, before, escaped).await,
        None => Ok(()),
    };
    if let (Some(detector), Some(before)) = (detector, changes_before) {
        record_changes(parts, cx, detector, before).await;
    }
    looked?;
    let finished =
        ran.map_err(|error| HostError::Failed(format!("`{named}` could not be started: {error}")))?;
    if finished.ending == Ending::Interrupted {
        return Err(HostError::Failed(format!(
            "`{named}` was stopped: the call was interrupted"
        )));
    }
    Ok(ExecOutput {
        code: match finished.exit {
            Some(Exit::Code(code)) => Some(code),
            _ => None,
        },
        stdout_cut: finished.stdout.is_cut(),
        stderr_cut: finished.stderr.is_cut(),
        stdout: finished.stdout.bytes,
        stderr: finished.stderr.bytes,
        timed_out: finished.ending == Ending::TimedOut,
    })
}

/// Records what a program changed in the workspace: in the round's
/// checkpoint, so that undo puts it back, and in the model's table of
/// files. Whatever became of the call: the program ran.
async fn record_changes(
    parts: &ExecParts,
    cx: &cineris_tools::RunCx,
    detector: Arc<ChangeDetector>,
    before: cineris_tools::shell::changes::Before,
) {
    let free = CancelToken::new();
    let Ok(changes) = run_blocking(&free, move |_| detector.after(&before).to_json()).await else {
        return;
    };
    if let Some(rounds) = &parts.rounds {
        let rounds = Arc::clone(rounds);
        let recorded = changes.clone();
        let _ = run_blocking(&free, move |_| rounds.record_changes(&recorded)).await;
    }
    note_changed(&changes, cx);
}

/// The look at the protected paths after a program that had a write root:
/// what the program changed put back at once (the third security fix), the
/// last look kept for the next confined start's comparison, what could not
/// be put back recorded in the round's checkpoint, and the turn stopped,
/// with the script's call stopped naming the change, when a protected path
/// changed or the look stopped short where the one before did not. A
/// program that left a process the kill could not clear stops the turn
/// too: it may still write one.
async fn look_after(
    parts: &ExecParts,
    host: &CallHost<'_>,
    before: Snapshot,
    escaped: bool,
) -> Result<(), HostError> {
    let cx = host.cx;
    // Not cancelled: the program ran, and what it changed is looked at
    // whatever became of the call since. The folder for hooks is read again
    // first: the program may have named another through a file the
    // configuration includes.
    let free = CancelToken::new();
    let (sandbox, git) = (Arc::clone(&parts.sandbox), Arc::clone(&parts.git));
    let root = cx.paths.root().to_fs_path();
    let found = run_blocking(&free, move |_| {
        sandbox.follow_hooks(&*git, &root);
        sandbox
            .protected()
            .map(|watch| protected_changes(&watch, &before))
    })
    .await
    .map_err(|error| HostError::Failed(format!("the protected paths were not looked at: {error}")))?
    .ok_or_else(|| HostError::Failed("the protected paths were not looked at".to_owned()))?;
    parts.sandbox.remember_after(found.after.clone());
    if let Some(rounds) = &parts.rounds {
        rounds.clear_touched();
        if !found.left.is_empty() {
            let rounds = Arc::clone(rounds);
            let left: Vec<Changed> = found.left.clone();
            let _ = run_blocking(&free, move |_| rounds.record_protected(&left)).await;
        }
    }
    let (changed, complete) = (&found.changed, found.complete);
    let root = cx.paths.root().to_fs_path();
    if let Some(note) = protected_note(changed, &found.left, complete, &root) {
        if !changed.is_empty() || (complete.0 && !complete.1) {
            // The turn stops, and the script's call with it: a script that
            // caught a refusal would go on as if nothing had changed.
            host.parts.policy.stop_turn(cx.turn, note.clone());
            return Err(HostError::Stopped(note));
        }
        // A workspace already past the bound before the program: told,
        // and the call goes on.
        (host.parts.tell)(note);
    }
    if escaped {
        let note = "[A process the extension's program started kept forking and could not all \
                    be killed. It may still be running inside the sandbox and may change a \
                    protected path after this check. The turn stops here, and the protected \
                    paths are looked at again before the next command and at the session's \
                    close.]";
        host.parts.policy.stop_turn(cx.turn, note.to_owned());
        return Err(HostError::Stopped(note.to_owned()));
    }
    Ok(())
}

/// The most lines of a program's input a question shows.
const INPUT_SHOWN_LINES: usize = 3;

/// The most bytes of a program's input a question shows.
const INPUT_SHOWN_BYTES: usize = 200;

/// The reason a question about a program given `input` adds: how long the
/// input is and how it begins, at most [`INPUT_SHOWN_LINES`] lines and
/// [`INPUT_SHOWN_BYTES`] bytes, every character that acts unseen escaped.
/// The engine judges the program's line, its arguments; the input is the
/// script's to give, and no rule reads it.
fn input_reason(input: &[u8]) -> String {
    let text = String::from_utf8_lossy(input);
    let lines: String = text.split_inclusive('\n').take(INPUT_SHOWN_LINES).collect();
    let mut end = lines.len().min(INPUT_SHOWN_BYTES);
    while !lines.is_char_boundary(end) {
        end -= 1;
    }
    let start = &lines[..end];
    let more = if start.len() < text.len() { "…" } else { "" };
    format!(
        "The program is given {} bytes of input, which no permission rule judges. It begins: \
         {}{more}",
        input.len(),
        cineris_base::visible(start)
    )
}

/// The directories of the `PATH` in `inherited`, in order: the first of
/// that name, in any case on Windows.
fn path_of(inherited: &[(OsString, OsString)]) -> Vec<PathBuf> {
    inherited
        .iter()
        .find(|(name, _)| cineris_base::child_env::is_named(name, "PATH", cfg!(windows)))
        .map(|(_, value)| std::env::split_paths(value).collect())
        .unwrap_or_default()
}

/// Whether `path` is a directory. Blocks: called on the blocking pool.
#[allow(
    clippy::disallowed_methods,
    reason = "looking at the file system blocks; it runs on the blocking pool"
)]
fn is_directory(path: &std::path::Path) -> bool {
    path.is_dir()
}

/// Runs `work` on the blocking pool, stopped by the call's cancellation.
async fn blocking<T: Send + 'static>(
    cx: &cineris_tools::RunCx,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, HostError> {
    run_blocking(&cx.cancel, move |_| work())
        .await
        .map_err(|error| HostError::Failed(error.to_string()))
}
