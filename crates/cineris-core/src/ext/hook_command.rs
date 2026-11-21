//! A hook command's start (`docs/steps/28/answered.md`, "The processes",
//! the hook command's column, and point 6).
//!
//! A hook command is started as an extension's program is, through
//! `super::exec::run`: the same program lookup (`PATH`'s absolute
//! directories outside the workspace and the write roots; on Windows
//! `.exe`, `.cmd`, `.bat` a directory at a time), the same Windows quoting
//! and `cmd.exe` line for a script, the same environment (a shell
//! command's), the same launcher of its own where the sandbox enforces,
//! the same looks at the protected paths before and after a start that has
//! a write root, with their stop, and the same runner, whose time is on the
//! session's clock and which kills the process tree when it is up. What
//! differs:
//!
//! - **Its arguments**: the entry's, each placeholder filled as a whole
//!   argument (`cineris_ext::hook::CommandHook::expand`), `{path}` with
//!   each file the call's plan reads or writes **within the workspace**,
//!   judged by place, resolved, in the system's plain form: a file outside
//!   it is left out, and the hook is told of none (the early review of the
//!   hook commands: a user's hook is not handed a file the model chose
//!   outside the workspace before the call's own question is answered).
//!   An entry that uses `{path}` does not run for a call that names no
//!   file within it.
//! - **Its decision**: every start is decided by the permission engine as
//!   an `exec` of the expanded line in the workspace (and `network` beside
//!   it with `network = true`), and never asks: [`hook_may_run`].
//! - **Its confinement**: the session's reads, and as its only write root
//!   the session's temporary folder, and no network, unless the entry says
//!   `write = true` (the workspace too, but never at `read-only`) or
//!   `network = true`. **Starts with a write root run one at a time,** as
//!   they share one baseline of the look at the protected paths.
//! - **Its input and answer**: the event's JSON on standard input, at most
//!   1 MiB; standard output read up to 1 MiB, the answer at most 64 KiB,
//!   one JSON value with no key twice at any depth
//!   (`cineris_ext::hook::answer::value`), which the loop reads by its
//!   event's contract.
//! - **Its writes** are not recorded in the round's checkpoint, and the
//!   model's table of files is not told of them.
//!
//! A hook that is refused, would need a question, does not start, runs
//! past its time, exits with anything but 0, writes more than 1 MiB, or
//! answers outside the contract has failed ([`HookOutcome::Failed`]).

use std::sync::{Arc, Mutex};

use cineris_agent::{Decision, PlannedCall};
use cineris_base::{CancelToken, PathResolver, in_code, visible};
use cineris_ext::event::Event;
use cineris_ext::hook::answer::MAX_OUTPUT;
use cineris_ext::hook::{CommandHook, Expansion};
use cineris_ext::host::{EXEC_MAX_STDIN, ExecOutput, ExecRequest, HostError};
use cineris_ext::source::Source;
use cineris_protocol::ids::{CallId, ItemId};
use cineris_protocol::lease::RuleAction;
use cineris_protocol::{SessionId, TurnId};
use cineris_tools::{ContentStore, Effect, FileTable, Json, Plan, RunCx, ToolSpec};
use serde_json::json;

use super::exec::{self, Decided, ToDecide};
use super::host::CallHost;
use super::tools::HostParts;
use crate::policy::EnginePolicy;

/// The longest part of a hook's standard error a failure quotes.
const STDERR_SHOWN: usize = 200;

/// Why the permission engine's decision does not let a hook command start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotRun {
    /// A rule refuses it.
    Denied(String),
    /// The turn is stopped.
    Stopped(String),
    /// The engine would ask, and a hook never puts a question.
    WouldAsk(String),
}

/// Whether a hook command's start goes ahead, by the permission engine's
/// `decision` of it (the owner's answer of 2026-11-21, "The owner's
/// answers"), and never by a question:
///
/// | The engine | The user's own hook | A project's hook |
/// |---|---|---|
/// | allow | runs | runs |
/// | allow as read-only | runs, unless the engine's reading may not be the program's (`read_only_asks`), when it is as an ask | the same |
/// | ask, a managed `ask` rule among the reasons (`managed_ask`) | does not run | does not run |
/// | ask | runs: it is the user's word | does not run |
/// | deny | does not run | does not run |
/// | stop | does not run, the turn stopped | the same |
///
/// # Errors
///
/// Why it does not run.
pub(crate) fn hook_may_run(
    decision: &Decision,
    managed_ask: bool,
    read_only_asks: bool,
    source: Source,
) -> Result<(), NotRun> {
    let asks = match decision {
        Decision::Allow => false,
        Decision::AllowReadOnly => read_only_asks,
        Decision::Ask => true,
        Decision::Deny { reason } => {
            return Err(NotRun::Denied(
                reason
                    .clone()
                    .unwrap_or_else(|| "a permission rule refuses it".to_owned()),
            ));
        }
        Decision::Stop { reason } => return Err(NotRun::Stopped(reason.clone())),
        _ => {
            return Err(NotRun::Denied(
                "the permission engine's answer is not one this build knows".to_owned(),
            ));
        }
    };
    if !asks {
        return Ok(());
    }
    if managed_ask {
        return Err(NotRun::WouldAsk(
            "the permission engine would ask before it runs, by a rule of the managed \
             configuration, which an administrator wrote for a person to decide each time, and \
             a hook never asks"
                .to_owned(),
        ));
    }
    match source {
        Source::User => Ok(()),
        Source::Project => Err(NotRun::WouldAsk(
            "the permission engine would ask before it runs, and a project's hook never asks: \
             allow its line with a permission rule"
                .to_owned(),
        )),
    }
}

/// One event a hook is run for.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HookEvent<'a> {
    /// The event's JSON, given on standard input.
    pub(crate) input: &'a Json,
    /// For a tool event, the tool the call is matched by: for a
    /// `tool_call`, the `source/tool` it names.
    pub(crate) tool: Option<&'a str>,
    /// For a tool event, the call's plan's effects: each `read` and
    /// `write` effect's file fills `{path}`.
    pub(crate) effects: &'a [Effect],
}

/// Where in the session a hook runs.
#[derive(Debug, Clone)]
pub(crate) struct HookCx {
    /// The session.
    pub(crate) session: SessionId,
    /// The turn the event is of: a stop it is under refuses the start, and
    /// a protected path the hook changes stops it.
    pub(crate) turn: TurnId,
    /// Stops the hook: its run is killed.
    pub(crate) cancel: CancelToken,
    /// Stops it at once, with no stages.
    pub(crate) kill: CancelToken,
    /// The session's path resolver.
    pub(crate) paths: Arc<PathResolver>,
}

/// What became of a hook command's run.
#[derive(Debug, Clone, PartialEq)]
pub enum HookOutcome {
    /// A hook that is waited for ran and answered this JSON value, with no
    /// key twice at any depth: the loop reads it by the event's contract.
    Answered(Json),
    /// A hook that is waited for ran and answered nothing; or it does not
    /// run for this call, as it uses `{path}` and the call names no file.
    Nothing,
    /// An observer ran: its answer is not read.
    Observed,
    /// It failed, and why: refused, not started, past its time, an exit
    /// that is not 0, too much output, or an answer outside the contract.
    /// On a gating event it counts as `deny`.
    Failed(String),
    /// The look at the protected paths around it stopped the turn, with
    /// what the user is told.
    Stopped(String),
}

/// A session's hook commands: those it runs, the entries it refused, and
/// what runs them, as the loop's points (`super::hook_points`) and the
/// session's own (`session.start`, `session.end`) reach them.
#[derive(Debug)]
pub(crate) struct SessionHooks {
    /// The user's own, then the project's approved ones.
    pub(crate) hooks: Vec<CommandHook>,
    /// The entries that do not run because they were refused, or changed
    /// since they were approved: each is on its event all the same, as a
    /// hook that fails ([`super::hooks::Refused`]).
    pub(crate) refused: Vec<super::hooks::Refused>,
    /// What runs them.
    pub(crate) commands: Arc<HookCommands>,
    /// The session's path resolver.
    pub(crate) paths: Arc<PathResolver>,
    /// The `session.start` hooks' run, which the first turn waits for.
    pub(crate) starting: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// The JSON a hook of the session's own events is given: the event, the
/// session and the workspace.
fn session_input(event: Event, session: &SessionId, paths: &PathResolver) -> Json {
    json!({
        "event": event.as_str(),
        "session_id": session.as_str(),
        "workspace": paths.root().to_native_string(),
    })
}

/// What the model is told of a `session.start` hook's `context`: under a
/// line that names the hook, every line quoted, what acts unseen escaped.
fn session_notice(hook: &str, context: &str) -> String {
    let shown = cineris_base::visible_keeping(context, &['\n']);
    let quoted: Vec<String> = shown
        .split('\n')
        .map(|line| {
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect();
    format!(
        "The hook `{}` adds this as the session opens. It is the hook's text, not the \
         user's:\n\n{}",
        in_code(hook),
        quoted.join("\n")
    )
}

impl SessionHooks {
    /// The hooks on `event` of the session's own (`session.start`,
    /// `session.end`), each run on a task of its own, together, with its
    /// cancellation: the runs, and whether each is waited for.
    fn runs_of(
        &self,
        event: Event,
        session: &SessionId,
    ) -> Vec<(
        String,
        bool,
        CancelToken,
        tokio::task::JoinHandle<HookOutcome>,
    )> {
        let input = session_input(event, session, &self.paths);
        self.hooks
            .iter()
            .filter(|hook| hook.event == event)
            .map(|hook| {
                let cancel = CancelToken::new();
                let cx = HookCx {
                    session: session.clone(),
                    // No turn of its own: a stop its look makes reaches the
                    // next prompt (`HookCommands::stopped`).
                    turn: TurnId::new("t_0"),
                    cancel: cancel.clone(),
                    kill: cancel.clone(),
                    paths: Arc::clone(&self.paths),
                };
                let commands = Arc::clone(&self.commands);
                let (hook, input) = (hook.clone(), input.clone());
                let name = hook.name.clone();
                let observe = hook.observe;
                let task = tokio::spawn(async move {
                    let event = HookEvent {
                        input: &input,
                        tool: None,
                        effects: &[],
                    };
                    commands.run(&hook, &event, &cx).await
                });
                (name, !observe, cancel, task)
            })
            .collect()
    }

    /// Starts the session's `session.start` hooks (`docs/steps/28/answered.md`,
    /// point 6): each one's `context` becomes a notice before the first
    /// prompt, with `notice`; a failure is told to the user and adds
    /// nothing. The first turn waits for those that are waited for, up to
    /// their time ([`Self::started`]); an observer runs to its own.
    pub(crate) async fn start(
        &self,
        session: &SessionId,
        notice: Arc<dyn Fn(String) + Send + Sync>,
    ) {
        let runs = self.runs_of(Event::SessionStart, session);
        if runs.is_empty() {
            return;
        }
        let tell = Arc::clone(&self.commands.parts.tell);
        let waiting = tokio::spawn(async move {
            for (name, waited, _cancel, task) in runs {
                if !waited {
                    // Never waited for, its answer not read: a failure is
                    // logged.
                    drop(task);
                    continue;
                }
                let said = match task.await {
                    Ok(HookOutcome::Answered(answer)) => {
                        cineris_ext::hook::Answer::of(Event::SessionStart, Some(answer)).map_err(
                            |why| {
                                format!("The hook `{}` failed: its answer {why}.", in_code(&name))
                            },
                        )
                    }
                    Ok(HookOutcome::Nothing | HookOutcome::Observed) => {
                        Ok(cineris_ext::hook::Answer::default())
                    }
                    Ok(HookOutcome::Failed(why) | HookOutcome::Stopped(why)) => Err(why),
                    Err(_) => Err(format!(
                        "The hook `{}` failed: its run ended before it could say how.",
                        in_code(&name)
                    )),
                };
                match said {
                    Ok(answer) => {
                        if let Some(context) = answer.context.filter(|text| !text.is_empty()) {
                            notice(session_notice(&name, &context));
                        }
                    }
                    Err(why) => tell(visible(&why).into_owned()),
                }
            }
        });
        *self.starting.lock().await = Some(waiting);
    }

    /// Waits for the `session.start` hooks that are waited for: what the
    /// first turn does before it starts. At once after that.
    pub(crate) async fn started(&self) {
        let waiting = self.starting.lock().await.take();
        if let Some(waiting) = waiting {
            let _ = waiting.await;
        }
    }

    /// Runs the session's `session.end` hooks as it closes: observed, their
    /// answers not read, given up to [`SESSION_END_TIME`] on `timer`, after
    /// which each is stopped, its process tree with it, and its look made.
    pub(crate) async fn end(&self, session: &SessionId, timer: &cineris_base::SharedTimer) {
        let runs = self.runs_of(Event::SessionEnd, session);
        if runs.is_empty() {
            return;
        }
        let mut over = timer.sleep(SESSION_END_TIME);
        for (name, _, cancel, mut task) in runs {
            let ended = std::future::poll_fn(|context| {
                if let std::task::Poll::Ready(ended) = std::pin::Pin::new(&mut task).poll(context) {
                    return std::task::Poll::Ready(Some(ended));
                }
                if over.as_mut().poll(context).is_ready() {
                    return std::task::Poll::Ready(None);
                }
                std::task::Poll::Pending
            })
            .await;
            let ended = match ended {
                Some(ended) => ended,
                None => {
                    // Its time is up: stopped, and its look made.
                    cancel.cancel();
                    task.await
                }
            };
            if let Ok(HookOutcome::Failed(why) | HookOutcome::Stopped(why)) = ended {
                tracing::warn!(hook = %name, %why, "a `session.end` hook failed");
            }
        }
    }
}

/// The most a session's `session.end` hooks are given while it closes.
pub(crate) const SESSION_END_TIME: std::time::Duration = std::time::Duration::from_secs(5);

/// What a session runs its hook commands with: its extensions' host parts,
/// whose `exec` is the shell's runner, sandbox and environment.
pub(crate) struct HookCommands {
    parts: Arc<HostParts>,
    content: Arc<dyn ContentStore>,
    /// One start with a write root at a time: they share one baseline of
    /// the look at the protected paths, which two at once would each take
    /// the other's change for its own.
    writing: tokio::sync::Mutex<()>,
    /// A stop a hook's look made, for the next prompt: the user is told at
    /// once, and the next prompt is refused with it, whether or not a turn
    /// was running when it was made (the early review's M2).
    stop: Mutex<Option<String>>,
}

impl std::fmt::Debug for HookCommands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookCommands").finish_non_exhaustive()
    }
}

impl HookCommands {
    /// The hook commands of a session with `parts` and the store of
    /// versions and artifacts `content`.
    pub(crate) fn new(parts: Arc<HostParts>, content: Arc<dyn ContentStore>) -> Self {
        Self {
            parts,
            content,
            writing: tokio::sync::Mutex::new(()),
            stop: Mutex::new(None),
        }
    }

    /// The session's policy, whose lease names the protected paths.
    pub(crate) fn policy(&self) -> &Arc<EnginePolicy> {
        &self.parts.policy
    }

    /// Whether a hook's look stopped a turn and the next prompt is to be
    /// refused for it.
    pub(crate) fn stop_pending(&self) -> bool {
        self.stop
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// The stop left for the next prompt, taken: it refuses one prompt.
    pub(crate) fn take_stop(&self) -> Option<String> {
        self.stop
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// What a hook's look found that stopped a turn, `note`: the user is
    /// told at once, and the next prompt is refused with it. On any event,
    /// observer or not, so that a stop made outside a running turn (an
    /// observer that ends late, `turn.end`, `session.start`) is not lost on
    /// a turn that is over or has not begun.
    pub(crate) fn stopped(&self, hook: &str, note: &str) {
        (self.parts.tell)(format!(
            "The hook `{}` was stopped by the look at the protected paths: {note}",
            in_code(hook)
        ));
        *self
            .stop
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(format!(
            "The hook `{}` stopped a turn: {note} The prompt was not sent, so that you can look \
             first. Send it again to go on.",
            in_code(hook)
        ));
    }

    /// Runs `hook` for `event`, in the session and turn its context names.
    pub(crate) async fn run(
        &self,
        hook: &CommandHook,
        event: &HookEvent<'_>,
        cx: &HookCx,
    ) -> HookOutcome {
        let name = in_code(&hook.name).into_owned();
        let failed = |why: String| HookOutcome::Failed(format!("The hook `{name}` failed: {why}."));
        // `{path}`: each file the call reads or writes, once, by place, in
        // the system's plain form (never `\\?\C:\…`).
        let mut keys = Vec::new();
        let mut paths = Vec::new();
        for effect in event.effects {
            if let Effect::Read(path) | Effect::Write(path) = effect
                && path.is_inside_workspace()
                && !keys.contains(&path.abs().key().to_owned())
            {
                keys.push(path.abs().key().to_owned());
                paths.push(path.abs().to_native_string());
            }
        }
        let workspace = cx.paths.root().to_native_string();
        let args = match hook.expand(&Expansion {
            tool: event.tool,
            paths: &paths,
            workspace: &workspace,
        }) {
            Ok(Some(args)) => args,
            // It uses `{path}`, and the call names no file: it does not run.
            Ok(None) => return HookOutcome::Nothing,
            Err(why) => return failed(why),
        };
        let input = match serde_json::to_vec(event.input) {
            Ok(input) if input.len() <= EXEC_MAX_STDIN => input,
            Ok(input) => {
                return failed(format!(
                    "the event is {} bytes, and a hook is given at most {} MiB",
                    input.len(),
                    EXEC_MAX_STDIN / (1024 * 1024)
                ));
            }
            Err(error) => return failed(format!("the event could not be written: {error}")),
        };
        let request = ExecRequest {
            program: hook.program.clone(),
            args,
            cwd: None,
            stdin: Some(input),
            timeout: hook.timeout,
            // Its decision and the looks before it are within its time.
            call_left: hook.timeout,
            network: hook.network,
            // Never at `read-only`, where nothing writes the workspace.
            write_roots: if hook.write
                && *self.parts.policy.engine().level() != cineris_protocol::lease::Level::ReadOnly
            {
                vec![cx.paths.root().clone()]
            } else {
                Vec::new()
            },
        };
        // One start with a write root at a time: the wait is within the
        // hook's own time, which began when it was asked.
        let _writing = if request.write_roots.is_empty() {
            None
        } else {
            Some(self.writing.lock().await)
        };
        let run_cx = RunCx::new(
            Arc::clone(&cx.paths),
            Arc::clone(&self.content),
            Arc::new(FileTable::new()),
        )
        .with_cancel(cx.cancel.clone())
        .with_kill(cx.kill.clone())
        .with_turn(turn_number(&cx.turn));
        let files = Arc::new(FileTable::new());
        let full_name = format!("hook.{}", hook.name);
        let progress = Mutex::new(Vec::new());
        // The extension's call it is not: a host of no extension, whose
        // only use is the start's own context (its run, its turn, the
        // session's policy, sandbox and front ends).
        let host = CallHost {
            parts: &self.parts,
            cx: &run_cx,
            files: &files,
            full_name: &full_name,
            extension: "",
            source: hook.source,
            progress: &progress,
        };
        let policy = Arc::clone(&self.parts.policy);
        let decide = |start: ToDecide<'_>| decide(&policy, hook, cx, start);
        let ran = exec::run(&self.parts.exec, request, &host, &Decided::Unasked(&decide)).await;
        match ran {
            Ok(output) => outcome(hook, &output).unwrap_or_else(failed),
            Err(HostError::Stopped(note)) => {
                self.stopped(&hook.name, &note);
                HookOutcome::Stopped(note)
            }
            Err(HostError::Denied(why) | HostError::Failed(why) | HostError::NotHere(why)) => {
                failed(why.trim_end_matches('.').to_owned())
            }
            Err(other) => failed(other.to_string()),
        }
    }
}

/// One event to run a session's hook command for, as the tests of a hook
/// command's start give it until the loop's points call hooks (the step's
/// next part replaces them by turns).
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct HookRun {
    /// The event's JSON, given on standard input.
    pub input: Json,
    /// For a tool event, the tool the call is matched by.
    pub tool: Option<String>,
    /// The files the call reads, as the model named them: resolved by the
    /// session's resolver, they fill `{path}`.
    pub paths: Vec<String>,
    /// The turn the event is of: `t_1` when not given.
    pub turn: Option<String>,
}

impl crate::Core {
    /// Runs the hook command `name` of the session `session` for one
    /// event, as the loop's points will, and says how it ended: for the
    /// tests of a hook command's start, until the loop calls hooks.
    ///
    /// # Errors
    ///
    /// When the session is not open, it runs no hook of that name, or a
    /// path cannot be resolved.
    #[doc(hidden)]
    pub async fn run_hook_command(
        &self,
        session: &SessionId,
        name: &str,
        run: HookRun,
    ) -> Result<HookOutcome, crate::OpError> {
        let live = self.live(session)?;
        let hooks = Arc::clone(&live.command_hooks);
        let usage = |message: String| {
            Box::new(cineris_protocol::error::ErrorData::new(
                cineris_protocol::error::ErrorKind::Usage,
                "no_such_hook",
                message,
            ))
        };
        let hook = hooks
            .hooks
            .iter()
            .find(|hook| hook.name == name)
            .ok_or_else(|| usage(format!("The session runs no hook `{name}`.")))?;
        let effects = run
            .paths
            .iter()
            .map(|path| {
                hooks
                    .paths
                    .resolve(path)
                    .map(Effect::Read)
                    .map_err(|error| usage(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let cx = HookCx {
            session: session.clone(),
            turn: TurnId::new(run.turn.as_deref().unwrap_or("t_1")),
            cancel: self.inner.options.cancel.child(),
            kill: CancelToken::new(),
            paths: Arc::clone(&hooks.paths),
        };
        let event = HookEvent {
            input: &run.input,
            tool: run.tool.as_deref(),
            effects: &effects,
        };
        Ok(hooks.commands.run(hook, &event, &cx).await)
    }
}

/// The number of turn `turn` (`t_7` is 7): 0 when it has none.
fn turn_number(turn: &TurnId) -> u32 {
    cineris_base::IdCounter::new("t_")
        .number_of(turn.as_str())
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or(0)
}

/// The engine's decision of a hook command's start, as an `exec` of its
/// line in the workspace under the tool name `hook.<name>`, with the
/// built-in tools' default; then [`hook_may_run`].
fn decide(
    policy: &EnginePolicy,
    hook: &CommandHook,
    cx: &HookCx,
    start: ToDecide<'_>,
) -> Result<(), HostError> {
    let spec = ToolSpec::new(format!("hook.{}", hook.name), "", json!({"type": "object"}))
        .permission(RuleAction::Allow);
    let mut plan = Plan::new(
        format!(
            "hook `{}`: run `{}`",
            in_code(&hook.name),
            in_code(start.line)
        ),
        (),
    );
    plan.effects.extend(start.effects);
    let arguments = json!({"command": start.line, "cwd": start.cwd.data()});
    let call_id = CallId::new(format!("hook_{}", hook.name));
    let item = ItemId::new(format!("hook_{}_{}", hook.name, cx.turn.as_str()));
    let planned = PlannedCall {
        session: &cx.session,
        turn: &cx.turn,
        call_id: &call_id,
        item: &item,
        spec: &spec,
        arguments: &arguments,
        plan: &plan,
        preview: None,
        hooks: None,
    };
    // One answer of the engine, of which everything is read: whether the
    // turn is stopped, then the outcome, and of a question whether an `ask`
    // rule of the managed configuration is among what asks, which holds
    // the user's own hook back too, as the user's own check (step 19). No
    // second asking could answer otherwise (the early review's L4).
    let (decision, managed) = match policy.stopped_turn(turn_number(&cx.turn)) {
        Some(reason) => (Decision::Stop { reason }, false),
        None => match policy.outcome(&planned) {
            cineris_policy::Outcome::Allow { read_only: true } => (Decision::AllowReadOnly, false),
            cineris_policy::Outcome::Allow { read_only: false } => (Decision::Allow, false),
            cineris_policy::Outcome::Ask(question) => (Decision::Ask, question.managed),
            cineris_policy::Outcome::Deny { reason } => (
                Decision::Deny {
                    reason: Some(reason),
                },
                false,
            ),
        },
    };
    hook_may_run(&decision, managed, start.read_only_asks, hook.source).map_err(|not| match not {
        NotRun::Stopped(reason) => HostError::Stopped(reason),
        NotRun::Denied(reason) => HostError::Denied(format!(
            "it was not started: a permission rule refuses it: {}",
            visible(&reason)
        )),
        NotRun::WouldAsk(why) => {
            HostError::Denied(format!("it was not started: {why}{}", needs_network(hook)))
        }
    })
}

/// What a hook's start, refused because the engine would ask, says it
/// needs: for a project's hook that may reach the network, an `allow`
/// rule of `network` too.
fn needs_network(hook: &CommandHook) -> &'static str {
    if hook.network {
        " and, as it may reach the network, a rule that allows `network` too"
    } else {
        ""
    }
}

/// What a hook's run comes to: an observer's is not read; a waited-for
/// hook's answer is read strictly once it ended by itself with 0 and wrote
/// no more than [`MAX_OUTPUT`].
fn outcome(hook: &CommandHook, output: &ExecOutput) -> Result<HookOutcome, String> {
    if output.timed_out {
        return Err(format!(
            "it ran past its {} ms and was stopped, its processes with it",
            hook.timeout.as_millis()
        ));
    }
    match output.code {
        Some(0) => {}
        Some(code) => {
            return Err(format!(
                "it exited with {code}{}",
                said(&output.stderr, output.stderr_cut)
            ));
        }
        None => {
            return Err(format!(
                "it was ended by the system{}",
                said(&output.stderr, output.stderr_cut)
            ));
        }
    }
    if hook.observe {
        return Ok(HookOutcome::Observed);
    }
    if output.stdout_cut || output.stdout.len() > MAX_OUTPUT {
        return Err(format!(
            "it wrote more than {} MiB to its standard output",
            MAX_OUTPUT / (1024 * 1024)
        ));
    }
    match cineris_ext::hook::answer::value(&output.stdout) {
        Ok(Some(answer)) => Ok(HookOutcome::Answered(answer)),
        Ok(None) => Ok(HookOutcome::Nothing),
        Err(why) => Err(format!("its answer {why}")),
    }
}

/// What a failure quotes of a hook's standard error: its first line, cut,
/// every character that acts unseen escaped. Nothing when it wrote none.
fn said(stderr: &[u8], cut: bool) -> String {
    let text = String::from_utf8_lossy(stderr);
    let Some(first) = text.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return String::new();
    };
    let shown = cineris_base::text::truncate_bytes(first, STDERR_SHOWN);
    let more = if shown.len() < text.trim_end().len() || cut {
        "…"
    } else {
        ""
    };
    format!(", saying: {}{more}", visible(shown))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cineris_ext::hook::command::Entry;

    use super::*;

    /// `hook_may_run` across the engine's answers, for the user's own hook
    /// and a project's.
    #[test]
    fn a_hooks_start_follows_the_engine_and_a_project_never_passes_a_question() {
        use Source::{Project, User};
        let allow = Decision::Allow;
        let read_only = Decision::AllowReadOnly;
        let ask = Decision::Ask;
        let deny = Decision::Deny {
            reason: Some("a rule".to_owned()),
        };
        let stop = Decision::Stop {
            reason: "stopped".to_owned(),
        };
        // (decision, managed ask, read-only may not be the program's, source) → runs
        let table = [
            (&allow, false, false, User, true),
            (&allow, false, false, Project, true),
            (&read_only, false, false, User, true),
            (&read_only, false, false, Project, true),
            (&read_only, false, true, User, true),
            (&read_only, false, true, Project, false),
            (&ask, false, false, User, true),
            (&ask, false, false, Project, false),
            (&ask, true, false, User, false),
            (&ask, true, false, Project, false),
            (&deny, false, false, User, false),
            (&deny, false, false, Project, false),
            (&stop, false, false, User, false),
            (&stop, false, false, Project, false),
        ];
        for (decision, managed, read_only_asks, source, runs) in table {
            let decided = hook_may_run(decision, managed, read_only_asks, source);
            assert_eq!(
                decided.is_ok(),
                runs,
                "{decision:?} managed {managed} read-only-asks {read_only_asks} {source:?}: \
                 {decided:?}"
            );
        }
        assert_eq!(
            hook_may_run(&deny, false, false, User),
            Err(NotRun::Denied("a rule".to_owned()))
        );
        assert_eq!(
            hook_may_run(&stop, false, false, User),
            Err(NotRun::Stopped("stopped".to_owned()))
        );
        let Err(NotRun::WouldAsk(managed)) = hook_may_run(&ask, true, false, User) else {
            panic!("a managed ask holds the user's hook back");
        };
        assert!(managed.contains("managed configuration"), "{managed}");
        let Err(NotRun::WouldAsk(project)) = hook_may_run(&ask, false, false, Project) else {
            panic!("a project's hook never passes a question");
        };
        assert!(project.contains("a project's hook never asks"), "{project}");
    }

    fn hook(observe: bool) -> CommandHook {
        CommandHook::compile(
            "gate",
            Source::User,
            &Entry {
                event: Some("tool.before".to_owned()),
                command: vec!["gate".to_owned()],
                observe,
                timeout: Some(Duration::from_secs(5)),
                ..Entry::default()
            },
        )
        .unwrap()
    }

    fn ran(code: Option<i32>, stdout: &str, stderr: &str) -> ExecOutput {
        ExecOutput {
            code,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
            ..ExecOutput::default()
        }
    }

    /// A run that ended with 0 is read strictly; any other end, a late one,
    /// or output past its cap, is a failure, quoting the start of what the
    /// hook said on standard error with nothing unseen.
    #[test]
    fn a_runs_end_and_output_decide_its_outcome() {
        let gate = hook(false);
        let answered = outcome(
            &gate,
            &ran(Some(0), r#"{"decision":"deny","reason":"no"}"#, ""),
        );
        let Ok(HookOutcome::Answered(answer)) = answered else {
            panic!("{answered:?}");
        };
        assert_eq!(answer["decision"], "deny");
        assert_eq!(
            outcome(&gate, &ran(Some(0), " \n", "")),
            Ok(HookOutcome::Nothing)
        );
        // A key given twice is refused here: the loop is handed one value.
        let strict = outcome(
            &gate,
            &ran(Some(0), r#"{"decision":"deny","decision":"allow"}"#, ""),
        )
        .unwrap_err();
        assert!(
            strict.starts_with("its answer is not one JSON value"),
            "{strict}"
        );
        assert!(
            strict.contains("the key `decision` is given twice"),
            "{strict}"
        );
        let exited =
            outcome(&gate, &ran(Some(2), "", "\n  bad \u{1b}[31m thing\nmore")).unwrap_err();
        assert_eq!(exited, "it exited with 2, saying: bad \\u{1b}[31m thing…");
        let killed = outcome(&gate, &ran(None, "", "")).unwrap_err();
        assert_eq!(killed, "it was ended by the system");
        let late = outcome(
            &gate,
            &ExecOutput {
                timed_out: true,
                ..ran(None, "", "")
            },
        )
        .unwrap_err();
        assert!(late.contains("ran past its 5000 ms"), "{late}");
        let flooded = outcome(
            &gate,
            &ExecOutput {
                stdout_cut: true,
                ..ran(Some(0), "{}", "")
            },
        )
        .unwrap_err();
        assert!(flooded.contains("more than 1 MiB"), "{flooded}");
        // An observer's answer is not read, whatever it is.
        assert_eq!(
            outcome(&hook(true), &ran(Some(0), "not json", "")),
            Ok(HookOutcome::Observed)
        );
        assert!(outcome(&hook(true), &ran(Some(1), "", "")).is_err());
    }

    #[test]
    fn a_turns_number_is_read_from_its_id() {
        assert_eq!(turn_number(&TurnId::new("t_7")), 7);
        assert_eq!(turn_number(&TurnId::new("none")), 0);
    }
}
