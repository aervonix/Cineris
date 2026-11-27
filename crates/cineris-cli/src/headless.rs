//! Headless mode: a prompt run to completion, with a printer for a front
//! end (`docs/architecture/10-headless.md`).
//!
//! Three threads:
//!
//! - **The core thread** runs a current-thread Tokio runtime with its timer
//!   and its I/O driver on: the provider layer's and the process runner's
//!   waits need the first, and the HTTP client's sockets the second.
//!   It starts the core, chooses the session, starts the turn, reads the
//!   client's queue and hands each line to the writer.
//! - **The writer thread** writes standard output, a line at a time, and
//!   flushes each. A reader that is slow holds the writer up, never the
//!   core: the core's queue merges and drops deltas for a client that is
//!   behind, and the channel to the writer is bounded.
//! - **The main thread** watches for signals ([`crate::signals`]) until the
//!   core thread has finished.
//!
//! Standard output carries the result and nothing else. Diagnostics go to
//! standard error, through the log's stderr sink and the notices.
//!
//! **The person's approvals** (the signed approvals step): with standard
//! input and standard output both terminals, the run takes the terminal
//! before anything slow, so that a password typed early is not echoed;
//! once its core has started, and when something carries a signature to
//! use, it asks for the approvals' password there before it opens the
//! session (`crate::approvals::open_at_start`). The main thread watches
//! for signals only once the terminal is put back, so that the terminal's
//! own handlers never outlive it. With no terminal, nothing is asked: the
//! run uses no approvals, and says so once when it had some to use.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use cineris_base::log::{self, FileOptions, Filter, LogHandle, LogOptions};
use cineris_base::{CancelToken, SecretFilter, SystemClock};
use cineris_config::{Dirs, Env, Override, StateDir};
use cineris_core::{ClientInfo, Core, CoreOptions, OpError, Outgoing, TokioTimer, parse_rule};
use cineris_protocol::event::{Event, TurnOutcome};
use cineris_protocol::headless::{HeadlessResult, PermissionDenial};
use cineris_protocol::lease::RuleAction;
use cineris_protocol::message::Part;
use cineris_protocol::methods::{
    CommandCall, Op, OpResult, SessionForkParams, SessionListParams, SessionNewParams, SessionRef,
    TurnInterruptParams, TurnStartParams,
};
use cineris_protocol::model::ModelRole;
use cineris_protocol::session::{SessionSettings, SessionState};
use cineris_protocol::{ErrorData, ErrorKind, ModelId, SessionId};
use tokio::sync::mpsc;
use tracing::metadata::LevelFilter;

use crate::args::{Choice, Format, Run};
use crate::exit::{self, Signal};
use crate::report::{self, Collector};
use crate::signals::{self, Interrupt};

/// The most standard input that is read as part of the prompt.
const STDIN_LIMIT: u64 = 32 * 1024 * 1024;
/// Lines that may wait for the writer before the core thread waits.
const WRITER_QUEUE: usize = 64;

/// What the core thread needs from the command line.
struct Plan {
    run: Run,
    prompt: Prompt,
    name: &'static str,
    workspace: PathBuf,
    /// The working directory, when it is below the root: where commands
    /// start.
    cwd: Option<PathBuf>,
    started: Instant,
    guard: Option<(PathBuf, Vec<OsString>)>,
    log: Option<Arc<LogHandle>>,
    env: Env,
    /// Whether it runs under a session's command, and its mark, as they
    /// are being asked: the core thread waits for them, and sets the two
    /// below, before it starts the core.
    standing: Standing,
    /// Run under a session's command: its level is at most `read-only`.
    capped: bool,
    /// Could not mark its commands as a session's: what it says of that,
    /// and its level is at most `supervised`.
    unmarked: Option<String>,
    /// The terminal taken for the approvals' password, at a terminal, and
    /// what tells the main thread that it is put back.
    asking: Option<Asking>,
}

/// The terminal a headless run took for the approvals' password, before
/// anything slow. Dropped, it is put back, and the main thread told.
struct Asking {
    terminal: cineris_tui::secret::PasswordTerminal,
    left: tokio::sync::oneshot::Sender<()>,
}

/// What a headless run's standing is: whether it runs under a session's
/// command and is held to `read-only` ([`crate::nested_run`]), and what its
/// mark of its sessions' commands said when it failed
/// ([`crate::mark_sessions`]); or the exit code of a run refused under a
/// session's command, which has said why.
type Asked = Result<(bool, Option<String>), u8>;

/// A headless run's standing, asked on a thread of its own from the run's
/// start while the rest of the start goes on (the prompt, the workspace,
/// the writer and the core thread with its runtime), and waited for before
/// anything that depends on it: the core, which reads the configuration,
/// holds the session at its level and starts processes, is started only
/// once it is known (since step 28's third pull request: on Windows the
/// check lists every process of the system, and cost a start about a
/// millisecond once it was made cheaper; asked in line it was 3 ms).
struct Standing {
    thread: Option<std::thread::JoinHandle<Asked>>,
    known: Option<Asked>,
}

impl Standing {
    /// Starts asking. Where no thread can be started, it is asked here.
    fn ask(name: &'static str, run: &Run) -> Self {
        let asked = run.clone();
        match std::thread::Builder::new()
            .name("cineris-standing".to_owned())
            .spawn(move || standing(name, &asked))
        {
            Ok(thread) => Self {
                thread: Some(thread),
                known: None,
            },
            Err(_) => Self {
                thread: None,
                known: Some(standing(name, run)),
            },
        }
    }

    /// The standing, waited for. A thread that failed leaves it unknown,
    /// and the run does not go on as if it were not under a session's
    /// command: it ends, saying so.
    fn wait(&mut self, name: &str) -> Asked {
        if let Some(thread) = self.thread.take() {
            self.known = Some(thread.join().unwrap_or_else(|_| {
                eprintln!(
                    "{name}: whether this run is under a session's command could not be told, \
                     so it does not run"
                );
                Err(exit::INTERNAL)
            }));
        }
        self.known.clone().unwrap_or(Err(exit::INTERNAL))
    }
}

/// Whether the run is under a session's command, checked before it is
/// marked: a harness's own mark is a job that holds it, which the check
/// would take for a session's.
fn standing(name: &str, run: &Run) -> Asked {
    let capped = crate::nested_run(name, run)?;
    Ok((capped, crate::mark_sessions()))
}

/// Runs a headless prompt. Returns the exit code.
pub(crate) fn run(run: Run, name: &'static str, started: Instant) -> u8 {
    let env = Env::from_process();
    let log = start_log(&run, &env, name);
    let standing = std::cell::RefCell::new(Standing::ask(name, &run));
    // A run refused under a session's command says that, and nothing else:
    // what would end it before its core starts waits for its standing.
    let refused = || standing.borrow_mut().wait(name).err();
    let format = run.format;
    let fail = |kind: ErrorKind, code: &str, message: String| {
        if let Some(code) = refused() {
            return code;
        }
        let error = ErrorData::new(kind, code, message);
        finish_early(format, run.quiet, name, &error, started)
    };
    let prompt = match prompt(run.prompt.as_deref()) {
        Ok(Some(prompt)) => prompt,
        Ok(None) => {
            return fail(
                ErrorKind::Usage,
                "no_prompt",
                format!(
                    "no prompt: give it after -p, or on standard input (`{name} -p \"…\"`, or \
                     `… | {name} -p`)"
                ),
            );
        }
        Err(error) => {
            return fail(
                ErrorKind::Usage,
                "unreadable_prompt",
                format!("standard input could not be read: {error}"),
            );
        }
    };
    // The workspace root, found by walking up; the working directory below
    // it is where commands start.
    let here = match crate::workspace::Here::now() {
        Ok(here) => here,
        Err(error) => {
            return fail(
                ErrorKind::Usage,
                "no_working_directory",
                format!("the working directory could not be read: {error}"),
            );
        }
    };
    let passed_over = here.passed_over_said();
    if !passed_over.is_empty()
        && let Some(code) = refused()
    {
        return code;
    }
    for said in passed_over {
        eprintln!("{name}: {}", crate::visible(&said));
    }
    let crate::workspace::Here {
        root: workspace,
        cwd,
        ..
    } = here;
    // The harness's own executable, under the hidden subcommand: the guard
    // of process trees. Without a path to itself, commands run unguarded.
    let guard = std::env::current_exe()
        .ok()
        .map(|exe| (exe, vec![OsString::from(crate::args::GUARD)]));

    let (lines, written) = mpsc::channel::<String>(WRITER_QUEUE);
    let (writer_done, writer_finished) = tokio::sync::oneshot::channel::<()>();
    let writer = std::thread::Builder::new()
        .name("cineris-writer".to_owned())
        .spawn(move || {
            write_lines(written);
            let _ = writer_done.send(());
        });
    let Ok(writer) = writer else {
        return fail(
            ErrorKind::Internal,
            "no_thread",
            "the writer thread could not be started".to_owned(),
        );
    };
    let (interrupts, interrupted) = mpsc::unbounded_channel::<Interrupt>();
    let (done, finished) = tokio::sync::oneshot::channel::<u8>();
    let handle = log.clone();
    let quiet = run.quiet;
    // At a terminal, the terminal for the approvals' password, taken before
    // anything slow: from here a password typed early is not echoed. Its
    // signals are left to this run's own watch, which starts once it is
    // put back.
    let (asking, left) = match taken_for_the_password(&env) {
        Some(terminal) => {
            let (left, put_back) = tokio::sync::oneshot::channel::<()>();
            (Some(Asking { terminal, left }), Some(put_back))
        }
        None => (None, None),
    };
    let plan = Plan {
        standing: standing.into_inner(),
        // Set from the standing before the core starts.
        capped: true,
        unmarked: None,
        asking,
        run,
        prompt,
        name,
        workspace,
        cwd,
        started,
        guard,
        log,
        env,
    };
    let core_thread = std::thread::Builder::new()
        .name("cineris-core".to_owned())
        .spawn(move || {
            let code = core_thread(plan, &lines, interrupted);
            drop(lines);
            let _ = done.send(code);
            code
        });
    let Ok(core_thread) = core_thread else {
        let error = ErrorData::new(
            ErrorKind::Internal,
            "no_thread",
            "the core thread could not be started",
        );
        return finish_early(format, quiet, name, &error, started);
    };

    // The main thread watches for signals until the run is over: the core
    // thread is done and the writer has written everything. Once the core
    // thread is done there is nothing left to interrupt, so a signal then
    // ends the process at once (a reader that takes nothing more would
    // otherwise hold it).
    let core_done = Arc::new(AtomicBool::new(false));
    let watched = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|runtime| {
            let core_done = Arc::clone(&core_done);
            runtime.block_on(async move {
                // Watched once the terminal taken for the password is put
                // back: the terminal's handlers would restore the ones
                // there when it was taken, over this watch's.
                if let Some(put_back) = left {
                    let _ = put_back.await;
                }
                let watch = signals::watch(interrupts, Arc::clone(&core_done));
                race_done(finished, writer_finished, &core_done, watch).await;
            });
        });
    if let Err(error) = watched {
        tracing::warn!(%error, "signals are not watched: their runtime did not start");
    }
    let code = core_thread.join();
    let _ = writer.join();
    if let Some(handle) = handle {
        handle.flush();
    }
    match code {
        Ok(code) => code,
        Err(_) => {
            // A panic on the core thread: still a record, and the code that
            // says the harness itself failed.
            let error = ErrorData::new(
                ErrorKind::Internal,
                "core_panicked",
                "the core thread failed; the log file has the details",
            );
            finish_early(format, quiet, name, &error, started)
        }
    }
}

/// Waits for the core thread and then the writer to be done, watching for
/// signals meanwhile, and marks the core done when it is. Watching may end
/// first (no signals can be watched): the wait goes on.
async fn race_done(
    mut finished: tokio::sync::oneshot::Receiver<u8>,
    mut writer_finished: tokio::sync::oneshot::Receiver<()>,
    core_done: &AtomicBool,
    watch: impl std::future::Future<Output = ()>,
) {
    let mut watch = std::pin::pin!(watch);
    let mut watching = true;
    let (mut core, mut writer) = (false, false);
    std::future::poll_fn(|context| {
        if !core && std::pin::Pin::new(&mut finished).poll(context).is_ready() {
            core = true;
            core_done.store(true, Ordering::SeqCst);
        }
        if !writer
            && std::pin::Pin::new(&mut writer_finished)
                .poll(context)
                .is_ready()
        {
            writer = true;
        }
        if core && writer {
            return std::task::Poll::Ready(());
        }
        if watching && watch.as_mut().poll(context).is_ready() {
            watching = false;
        }
        std::task::Poll::Pending
    })
    .await;
}

/// The core thread: a runtime, the core, and the run. Returns the exit code.
fn core_thread(
    mut plan: Plan,
    lines: &mpsc::Sender<String>,
    interrupted: mpsc::UnboundedReceiver<Interrupt>,
) -> u8 {
    // The timer for the provider layer's and the process runner's waits,
    // and the I/O driver for the HTTP client's sockets.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "the core thread's runtime did not start");
            return exit::INTERNAL;
        }
    };
    // Its standing, before its core starts: a run refused under a
    // session's command ends here, having read none of its configuration;
    // one under a session's command is held to `read-only` from its core's
    // start; and its sessions' commands are marked before any process
    // starts, which only the core starts.
    match plan.standing.wait(plan.name) {
        Ok((capped, unmarked)) => {
            plan.capped = capped;
            plan.unmarked = unmarked;
        }
        Err(code) => return code,
    }
    runtime.block_on(drive(plan, lines, interrupted))
}

/// What a run in progress has to hand.
struct Front<'a> {
    plan: &'a Plan,
    lines: &'a mpsc::Sender<String>,
    interrupted: mpsc::UnboundedReceiver<Interrupt>,
    /// The first signal, which decides the exit code of an interrupted run.
    signal: Option<Signal>,
    /// Whether to print a line for each tool call on standard error.
    progress: bool,
    /// The turn a signal interrupts, once it has been asked for.
    target: Option<(Core, cineris_core::ClientId, SessionId)>,
}

impl Front<'_> {
    /// Hands a line to the writer. While the writer has no room (a reader
    /// that is slow, or stopped), signals are still taken: each interrupts
    /// the turn as it would anywhere else.
    async fn write(&mut self, line: String) {
        if line.is_empty() {
            return;
        }
        let mut send = std::pin::pin!(self.lines.send(line));
        loop {
            let mut signals_open = true;
            let next = std::future::poll_fn(|context| {
                if let std::task::Poll::Ready(sent) = send.as_mut().poll(context) {
                    // A writer that has gone (standard output closed) takes
                    // nothing more, and the run goes on to its end.
                    let _ = sent;
                    return std::task::Poll::Ready(None);
                }
                if signals_open {
                    match self.interrupted.poll_recv(context) {
                        std::task::Poll::Ready(Some(interrupt)) => {
                            return std::task::Poll::Ready(Some(interrupt));
                        }
                        std::task::Poll::Ready(None) => signals_open = false,
                        std::task::Poll::Pending => {}
                    }
                }
                std::task::Poll::Pending
            })
            .await;
            match next {
                None => return,
                Some(interrupt) => self.interrupt(interrupt).await,
            }
        }
    }

    /// Interrupts the turn, or, before there is one, remembers the signal
    /// so that none is started.
    async fn interrupt(&mut self, interrupt: Interrupt) {
        self.signal.get_or_insert(interrupt.signal);
        if let Some((core, client, session)) = &self.target {
            let _ = core
                .handle(
                    *client,
                    Op::TurnInterrupt(TurnInterruptParams {
                        session_id: session.clone(),
                        now: interrupt.now,
                    }),
                )
                .await;
        }
    }

    fn say(&self, text: &str) {
        if !self.plan.run.quiet {
            eprintln!("{}: {}", self.plan.name, crate::visible(text));
        }
    }

    /// Takes the signals that have arrived. Says whether there was one.
    fn signalled(&mut self) -> bool {
        while let Ok(interrupt) = self.interrupted.try_recv() {
            self.signal.get_or_insert(interrupt.signal);
        }
        self.signal.is_some()
    }
}

async fn drive(
    mut plan: Plan,
    lines: &mpsc::Sender<String>,
    interrupted: mpsc::UnboundedReceiver<Interrupt>,
) -> u8 {
    let asking = plan.asking.take();
    let run = &plan.run;
    let mut front = Front {
        plan: &plan,
        lines,
        interrupted,
        signal: None,
        progress: run.format == Format::Text && !run.quiet && io::stderr().is_terminal(),
        target: None,
    };
    let core = match start_core(&plan) {
        Ok(core) => core,
        Err(error) => {
            drop(asking);
            return failed(&mut front, *error).await;
        }
    };
    // The person's approvals, before the session opens: what a session
    // checks its stores with is the key it had when it opened. `Ctrl+C` at
    // their password ends the run as interrupted, before its session opens.
    if !approvals(&core, &front, asking) {
        front.signal.get_or_insert(Signal::Interrupt);
        let code = failed(
            &mut front,
            ErrorData::new(
                ErrorKind::Interrupted,
                "interrupted",
                "the run was interrupted at the approvals' password, before its session opened",
            ),
        )
        .await;
        core.shutdown().await;
        return code;
    }
    let code = in_core(&core, &mut front).await;
    core.shutdown().await;
    code
}

/// Takes the terminal for the approvals' password, when standard input and
/// standard output are both terminals: `None` otherwise, or when it cannot
/// be taken (the run then asks nothing). The configuration is read for its
/// `[terminal]` settings, as `auth login` reads it.
fn taken_for_the_password(env: &Env) -> Option<cineris_tui::secret::PasswordTerminal> {
    if !crate::approvals::at_terminal() {
        return None;
    }
    // With no key file there are no approvals to use: the terminal is left
    // as it is, and the run costs nothing more. Only whether it is there is
    // asked; it is read once the terminal is taken.
    let key_file = StateDir::from_dirs(&Dirs::from_env(env))
        .map(|state| state.root().join(cineris_core::signed::KEY_FILE))?;
    if std::fs::symlink_metadata(&key_file).is_err() {
        return None;
    }
    let overrides = cineris_config::load(&cineris_config::LoadOptions::new(env.clone()))
        .map(|loaded| crate::interactive::overrides(&loaded.settings.terminal))
        .unwrap_or_default();
    cineris_tui::secret::PasswordTerminal::enter_leaving_signals(overrides).ok()
}

/// Opens the person's approvals in a run's core when it has some to use and
/// took the terminal to ask for them; says on standard error, once, when it
/// goes on without them. A run under a session's command has no person,
/// and asks nothing. The terminal is put back, and the main thread told,
/// before this returns. `false` when the person pressed `Ctrl+C` at the
/// password: the run ends as interrupted.
fn approvals(core: &Core, front: &Front<'_>, asking: Option<Asking>) -> bool {
    if front.plan.capped {
        drop(asking);
        return true;
    }
    let (terminal, left) = match asking {
        Some(Asking { terminal, left }) => (Some(terminal), Some(left)),
        None => (None, None),
    };
    let project = core.project(&front.plan.workspace).ok();
    let at_start = crate::approvals::open_at_start(core, project.as_ref(), terminal);
    if let Some(left) = left {
        let _ = left.send(());
    }
    match at_start {
        crate::approvals::AtStart::GoesOn(said) => {
            if let Some(said) = said {
                front.say(&said);
            }
            true
        }
        crate::approvals::AtStart::Interrupted => false,
    }
}

/// Everything after the core started: the session, the turn and the
/// result.
async fn in_core(core: &Core, front: &mut Front<'_>) -> u8 {
    let run = &front.plan.run;
    let mut info = ClientInfo::headless();
    info.capabilities.deltas = run.include_deltas && run.format == Format::Jsonl;
    let client = core.connect(info);

    let state = match session(core, client.id, front.plan).await {
        Ok(state) => state,
        Err(error) => return failed(front, *error).await,
    };
    let session = state.session_id.clone();
    if run.format == Format::Jsonl {
        let model = state
            .roles
            .get(&ModelRole::Main)
            .map_or_else(|| ModelId::from(""), |role| role.model.clone());
        front
            .write(report::start_line(
                session.clone(),
                model,
                state.workspace.clone(),
            ))
            .await;
    }
    let mut collector = Collector::new(session.clone());

    // A signal that came while the session was being prepared stops the
    // run before anything is sent.
    if front.signalled() {
        let error = interrupted_error();
        return finish(
            core,
            front,
            &collector,
            Some((TurnOutcome::Interrupted, error)),
        )
        .await;
    }
    // Words after `-p` that are `/name …`, `name` one of the session's
    // custom commands, are sent as that command, as the interface sends it:
    // the core expands it before any hook, and puts what standard input gave
    // after it (step 28). Anything else is text.
    let command = match command_in(core, client.id, &session, &front.plan.prompt).await {
        Some((command, said)) => {
            front.say(&said);
            Some(command)
        }
        None => None,
    };
    let running_command = command.is_some();
    let mut start = std::pin::pin!(core.handle(
        client.id,
        Op::TurnStart(front.plan.prompt.start(session.clone(), command)),
    ));
    // An extension's command runs in the start itself: a signal meanwhile
    // interrupts it, through the core, and the run ends interrupted, with
    // nothing sent (step 28). Signals are still taken while the start ends:
    // a second one kills the command's programs at once (`now`), as it
    // would a turn's.
    let started = if running_command {
        match first_or_signal(start.as_mut(), &mut front.interrupted).await {
            Ok(started) => started,
            Err(interrupt) => {
                front.signal.get_or_insert(interrupt.signal);
                let mut now = interrupt.now;
                loop {
                    let _ = core
                        .handle(
                            client.id,
                            Op::TurnInterrupt(TurnInterruptParams {
                                session_id: session.clone(),
                                now,
                            }),
                        )
                        .await;
                    if first_or_signal(start.as_mut(), &mut front.interrupted)
                        .await
                        .is_ok()
                    {
                        break;
                    }
                    now = true;
                }
                let error = interrupted_error();
                return finish(
                    core,
                    front,
                    &collector,
                    Some((TurnOutcome::Interrupted, error)),
                )
                .await;
            }
        }
    } else {
        start.await
    };
    if let Err(error) = started {
        let outcome = exit::outcome_for_kind(&error.kind);
        return finish(core, front, &collector, Some((outcome, *error))).await;
    }
    front.target = Some((core.clone(), client.id, session.clone()));

    loop {
        let next = next_or_signal(&client.queue, &mut front.interrupted).await;
        match next {
            Next::Signal(interrupt) => front.interrupt(interrupt).await,
            Next::Outgoing(Outgoing::Event(event)) => {
                let ended = collector.observe(&event);
                if front.plan.run.format == Format::Jsonl {
                    front.write(report::event_line(&event)).await;
                }
                tell(front, &event.event);
                if ended {
                    break;
                }
            }
            // Headless mode answers nothing, and declared so: no request
            // is sent to it.
            Next::Outgoing(Outgoing::Request { .. }) => {}
            Next::Closed => {
                let error = ErrorData::new(
                    ErrorKind::Internal,
                    "client_closed",
                    "the core closed this run's connection before the turn ended",
                );
                return finish(core, front, &collector, Some((TurnOutcome::Failed, error))).await;
            }
        }
    }
    let code = finish(core, front, &collector, None).await;
    let _ = core
        .handle(
            client.id,
            Op::SessionClose(SessionRef {
                session_id: session,
            }),
        )
        .await;
    code
}

/// The custom command `prompt`'s words name ([`Prompt::command`]), when
/// `name` is one of the session's (`commands/list`), used or not, or an
/// extension's (`extension:command`) that neither list has, so that the core
/// refuses one it does not use, or does not have, with its reason: what
/// follows the name is its arguments. With it, the line that says on
/// standard error whose it is and its file ([`sending`]). When the list
/// cannot be had, a `/name …` is sent as a command all the same, for the
/// core to judge, rather than as text. `None` for any other prompt, which
/// is sent as text.
async fn command_in(
    core: &Core,
    client: cineris_core::ClientId,
    session: &SessionId,
    prompt: &Prompt,
) -> Option<(CommandCall, String)> {
    let (name, args) = prompt.command()?;
    let listed = core
        .handle(
            client,
            Op::CommandsList(cineris_protocol::methods::CommandsListParams {
                session_id: session.clone(),
            }),
        )
        .await;
    let listed = match listed {
        Ok(OpResult::CommandsList(listed)) => Some(listed),
        _ => None,
    };
    let said = sending(name, listed.as_ref())?;
    let call = CommandCall {
        name: name.to_owned(),
        args: args.trim().to_owned(),
    };
    Some((call, said))
}

/// What headless mode says on standard error as it sends `/name` as a
/// command, given the session's `commands/list` (`None` when it could not be
/// had): whose the command is and its file; that it is one not used; or, for
/// an extension's (a name with a `:`) that neither list has, that the session
/// does not list it: its extension may not be loaded, and the core says why,
/// rather than the name going to the model as text. `None` when `/name` is
/// none of these: the prompt is sent as text.
fn sending(
    name: &str,
    listed: Option<&cineris_protocol::methods::CommandsListResult>,
) -> Option<String> {
    let shown = cineris_base::in_code(&format!("/{name}")).into_owned();
    let Some(listed) = listed else {
        return Some(format!(
            "Sending `{shown}` as a command: the session's commands could not be listed."
        ));
    };
    let whose = |source: &cineris_protocol::item::CommandSource, extension: Option<&String>| {
        let owner = match source {
            cineris_protocol::item::CommandSource::User => "your".to_owned(),
            cineris_protocol::item::CommandSource::Project => "the project's".to_owned(),
            other => format!("{}'s", other.as_str()),
        };
        match extension {
            Some(extension) => format!(
                "{owner} extension `{}`'s command",
                cineris_base::in_code(extension)
            ),
            None => format!("{owner} command"),
        }
    };
    if let Some(command) = listed.commands.iter().find(|command| command.name == name) {
        let file = command
            .path
            .as_deref()
            .map(|path| format!(" ({})", cineris_base::in_code(path)))
            .unwrap_or_default();
        return Some(format!(
            "Sending `{shown}`, {}{file}.",
            whose(&command.source, command.extension.as_ref())
        ));
    }
    if listed
        .not_used
        .iter()
        .any(|command| !command.name.is_empty() && command.name == name)
    {
        return Some(format!("Sending `{shown}`, a command that is not used."));
    }
    name.contains(':').then(|| {
        format!(
            "Sending `{shown}` as an extension's command, which this session does not list: its \
             extension may not be loaded."
        )
    })
}

/// Writes the result in the run's format, says what went wrong on standard
/// error, and returns the exit code that agrees with the outcome.
async fn finish(
    core: &Core,
    front: &mut Front<'_>,
    collector: &Collector,
    failure: Option<(TurnOutcome, ErrorData)>,
) -> u8 {
    let denials = denials(core, collector);
    let mut result = collector.result(failure, denials, elapsed(front.plan.started));
    report::held_to_its_denial(&mut result, front.plan.run.fail_on_denial);
    let code = exit::for_outcome(
        &result.outcome,
        result.error.as_ref().map(|error| &error.kind),
        front.signal,
    );
    write_result(front, result).await;
    code
}

fn denials(core: &Core, collector: &Collector) -> Vec<PermissionDenial> {
    let Some(session) = collector.session() else {
        return Vec::new();
    };
    core.permission_denials(session)
        .into_iter()
        .map(|denial| PermissionDenial {
            turn_id: denial.turn_id,
            call_id: denial.call_id,
            tool: denial.tool,
            summary: denial.summary,
            reason: denial.reason,
        })
        .collect()
}

async fn write_result(front: &mut Front<'_>, mut result: HeadlessResult) {
    if front.plan.capped {
        front.say(&format!(
            "{} (`docs/outstanding/{}.md`)",
            crate::CAPPED,
            crate::CAPPED_ITEM
        ));
        result.capped = Some(cineris_protocol::headless::HeadlessCap {
            level: cineris_protocol::lease::Level::ReadOnly,
            reason: crate::CAPPED.to_owned(),
            item: Some(crate::CAPPED_ITEM.to_owned()),
        });
    } else if let Some(unmarked) = front.plan.unmarked.clone() {
        front.say(&unmarked);
        result.capped = Some(cineris_protocol::headless::HeadlessCap {
            level: crate::UNMARKED_LEVEL,
            reason: unmarked,
            item: None,
        });
    }
    if let Some(error) = &result.error {
        front.say(&format!(
            "{} ({}: {})",
            error.message, error.kind, error.code
        ));
    }
    for denial in &result.permission_denials {
        front.say(&format!("denied: {} ({})", denial.summary, denial.reason));
    }
    let line = match front.plan.run.format {
        Format::Text => report::text_output(&result),
        Format::Json | Format::Jsonl => report::result_line(result),
    };
    front.write(line).await;
}

/// A run that failed before it had a session.
async fn failed(front: &mut Front<'_>, error: ErrorData) -> u8 {
    let collector = Collector::default();
    let outcome = exit::outcome_for_kind(&error.kind);
    let result = collector.result(
        Some((outcome, error)),
        Vec::new(),
        elapsed(front.plan.started),
    );
    let code = exit::for_outcome(
        &result.outcome,
        result.error.as_ref().map(|error| &error.kind),
        front.signal,
    );
    write_result(front, result).await;
    code
}

/// A run that failed before its threads were started: its record is
/// written from the main thread.
fn finish_early(
    format: Format,
    quiet: bool,
    name: &str,
    error: &ErrorData,
    started: Instant,
) -> u8 {
    if !quiet {
        eprintln!("{name}: {}", crate::visible(&error.message));
    }
    let outcome = exit::outcome_for_kind(&error.kind);
    let result =
        Collector::default().result(Some((outcome, error.clone())), Vec::new(), elapsed(started));
    let code = exit::for_outcome(&result.outcome, Some(&error.kind), None);
    if format != Format::Text {
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(report::result_line(result).as_bytes());
        let _ = stdout.flush();
    }
    code
}

/// What to tell the person on standard error as an event passes: the
/// notices, and in `text` format a line for each tool call when standard
/// error is a terminal.
fn tell(front: &Front<'_>, event: &Event) {
    match event {
        Event::Notice(notice) => {
            if let Some(message) = notice.message() {
                front.say(message);
            }
        }
        Event::ItemStarted(started) if front.progress => {
            if let Some(tool) = &started.tool {
                front.say(&format!("· {tool}"));
            }
        }
        _ => {}
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "one value at a time, moved straight out: a box would be an allocation per event"
)]
enum Next {
    Outgoing(Outgoing),
    Signal(Interrupt),
    Closed,
}

/// The next thing for the client, or a signal, whichever comes first.
/// What `operation` ends with, or the signal that comes first. The
/// operation is left as it is on a signal, for the caller to finish.
async fn first_or_signal<T>(
    mut operation: std::pin::Pin<&mut impl Future<Output = T>>,
    interrupted: &mut mpsc::UnboundedReceiver<Interrupt>,
) -> Result<T, Interrupt> {
    let mut signals_open = true;
    std::future::poll_fn(|context| {
        if let std::task::Poll::Ready(done) = operation.as_mut().poll(context) {
            return std::task::Poll::Ready(Ok(done));
        }
        if signals_open {
            match interrupted.poll_recv(context) {
                std::task::Poll::Ready(Some(interrupt)) => {
                    return std::task::Poll::Ready(Err(interrupt));
                }
                std::task::Poll::Ready(None) => signals_open = false,
                std::task::Poll::Pending => {}
            }
        }
        std::task::Poll::Pending
    })
    .await
}

async fn next_or_signal(
    queue: &cineris_core::EventQueue,
    interrupted: &mut mpsc::UnboundedReceiver<Interrupt>,
) -> Next {
    let mut outgoing = std::pin::pin!(queue.next());
    let mut signals_open = true;
    std::future::poll_fn(|context| {
        if signals_open {
            match interrupted.poll_recv(context) {
                std::task::Poll::Ready(Some(interrupt)) => {
                    return std::task::Poll::Ready(Next::Signal(interrupt));
                }
                std::task::Poll::Ready(None) => signals_open = false,
                std::task::Poll::Pending => {}
            }
        }
        outgoing
            .as_mut()
            .poll(context)
            .map(|next| next.map_or(Next::Closed, Next::Outgoing))
    })
    .await
}

/// Starts the core with what the command line says.
fn start_core(plan: &Plan) -> Result<Core, OpError> {
    let run = &plan.run;
    let mut options = CoreOptions::new(plan.env.clone(), Arc::new(TokioTimer));
    options.own_commands = Some(crate::nested::own_commands());
    if plan.capped {
        options.level_at_most = Some(cineris_protocol::lease::Level::ReadOnly);
        options.level_at_most_why = Some(crate::CAPPED_WHY.to_owned());
    } else if plan.unmarked.is_some() {
        options.level_at_most = Some(crate::UNMARKED_LEVEL);
        options.level_at_most_why = Some(crate::UNMARKED_WHY.to_owned());
    }
    options.profile.clone_from(&run.common.profile);
    options.overrides = overrides(run);
    for (texts, action) in [
        (&run.allow, RuleAction::Allow),
        (&run.deny, RuleAction::Deny),
    ] {
        for text in texts {
            let rule = parse_rule(text, action).map_err(|message| {
                Box::new(ErrorData::new(ErrorKind::Usage, "invalid_rule", message))
            })?;
            options.rules.push(rule);
        }
    }
    options.trust_project = run.trust_project;
    options.fail_on_denial = run.fail_on_denial;
    // A headless run's first request does not wait for the repository map
    // (the coordinator's decision in step 18): the map follows as a notice
    // at the first boundary where it is ready. `index.map_wait` sets a wait.
    options.map_wait = std::time::Duration::ZERO;
    options.system_append.clone_from(&run.system_append);
    options.guard.clone_from(&plan.guard);
    // The same executable, under the other hidden subcommand: the sandbox
    // launcher every command is started through where the sandbox
    // enforces.
    options.launcher = plan
        .guard
        .as_ref()
        .map(|(exe, _)| (exe.clone(), vec![OsString::from(crate::args::SANDBOX)]));
    options.cancel = CancelToken::new();
    if let Some(log) = &plan.log {
        let log = log.clone();
        options.key_found = Some(Arc::new(move |secret: &cineris_base::Secret| {
            log.add_secret(secret);
        }));
    }
    // Held here by the tests of a password typed before the prompt
    // (`cineris_base::hook::HOLD_CORE`).
    cineris_base::hook::hold_core();
    Core::start(options)
}

/// The settings the command line gives, in order.
pub(crate) fn overrides(run: &Run) -> Vec<Override> {
    run.common
        .settings
        .iter()
        .map(|(key, value, flag)| Override {
            key: key.clone(),
            value: value.clone(),
            flag: flag.clone(),
        })
        .collect()
}

/// Opens the session the run is in.
async fn session(
    core: &Core,
    client: cineris_core::ClientId,
    plan: &Plan,
) -> Result<SessionState, OpError> {
    let workspace = plan.workspace.to_string_lossy().into_owned();
    let open = |session_id: String| {
        Op::SessionOpen(SessionRef {
            session_id: SessionId::from(session_id),
        })
    };
    let op = match &plan.run.choice {
        Choice::New => Op::SessionNew(SessionNewParams {
            cwd: plan
                .cwd
                .as_ref()
                .and_then(|cwd| cwd.to_str().map(str::to_owned)),
            workspace,
            settings: SessionSettings::default(),
            title: None,
            config: serde_json::Map::new(),
            ephemeral: plan.run.no_session,
        }),
        Choice::Resume(id) => open(id.clone()),
        Choice::Fork(id) => {
            // A session is forked from its log, so it is opened first.
            core.handle(client, open(id.clone())).await?;
            Op::SessionFork(SessionForkParams {
                session_id: SessionId::from(id.as_str()),
                at: None,
                before: None,
            })
        }
        Choice::Continue => {
            let listed = core
                .handle(
                    client,
                    Op::SessionList(SessionListParams {
                        workspace: Some(workspace),
                        limit: Some(1),
                        cursor: None,
                    }),
                )
                .await?;
            let latest = match listed {
                OpResult::SessionList(list) => list.sessions.into_iter().next(),
                _ => None,
            };
            let Some(latest) = latest else {
                return Err(Box::new(ErrorData::new(
                    ErrorKind::Usage,
                    "no_session",
                    "there is no session to continue in this workspace",
                )));
            };
            open(latest.session_id.as_str().to_owned())
        }
    };
    match core.handle(client, op).await? {
        OpResult::SessionNew(result) | OpResult::SessionOpen(result) => Ok(result.session),
        OpResult::SessionFork(result) => Ok(result.session),
        _ => Err(Box::new(ErrorData::new(
            ErrorKind::Internal,
            "unexpected_result",
            "the core answered with the result of another operation",
        ))),
    }
}

/// The prompt: standard input when it is not a terminal, and the words
/// given after `-p`. `None` when there is neither.
fn prompt(words: Option<&str>) -> io::Result<Option<Prompt>> {
    let stdin = io::stdin();
    let mut piped = if stdin.is_terminal() {
        String::new()
    } else {
        // Bytes that are not UTF-8 (a diff of a file in another encoding)
        // are read as U+FFFD rather than failing the run.
        let mut bytes = Vec::new();
        stdin.lock().take(STDIN_LIMIT).read_to_end(&mut bytes)?;
        String::from_utf8_lossy(&bytes).into_owned()
    };
    piped.truncate(piped.trim_end().len());
    let prompt = Prompt {
        piped,
        words: words.map(str::trim).unwrap_or_default().to_owned(),
    };
    Ok((!prompt.piped.is_empty() || !prompt.words.is_empty()).then_some(prompt))
}

/// A run's prompt, as it came: what standard input gave, and the words
/// given after `-p`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Prompt {
    /// Standard input, its end trimmed; empty when it is a terminal. Text
    /// for the model, a log or a diff, and never a command, whatever its
    /// first line is.
    piped: String,
    /// The words given after `-p`, trimmed: the prompt the person wrote,
    /// the only place a command is taken from.
    words: String,
}

impl Prompt {
    /// The prompt as text: standard input, then the words, with a blank
    /// line between.
    fn text(&self) -> String {
        match (self.piped.is_empty(), self.words.is_empty()) {
            (true, _) => self.words.clone(),
            (false, true) => self.piped.clone(),
            (false, false) => format!("{}\n\n{}", self.piped, self.words),
        }
    }

    /// The custom command the words name, `/name …`, and its arguments:
    /// what follows the name. Standard input is never read for one: a log
    /// that begins `/usr/bin/ld: cannot find -lfoo`, or an issue's body that
    /// begins `/tracker:deploy prod`, is text for the model. Nor is a name
    /// that holds a `/` one, as the interface reads it: a path, which no
    /// command's name has.
    fn command(&self) -> Option<(&str, &str)> {
        let named = self.words.strip_prefix('/')?;
        let (name, args) = named.split_once(char::is_whitespace).unwrap_or((named, ""));
        (!name.is_empty() && !name.contains('/')).then_some((name, args))
    }

    /// What `turn/start` is sent: the prompt as text, or, with a command,
    /// the command and what standard input gave, which the core puts after
    /// the command's expanded text, as one prompt that `prompt.submit`
    /// sees (step 28).
    fn start(&self, session: SessionId, command: Option<CommandCall>) -> TurnStartParams {
        let input = match (&command, self.piped.is_empty()) {
            (None, _) => vec![Part::text(self.text())],
            (Some(_), true) => Vec::new(),
            (Some(_), false) => vec![Part::text(self.piped.clone())],
        };
        TurnStartParams {
            session_id: session,
            input,
            delivery: None,
            command,
        }
    }
}

/// Starts the log: the file in the state directory's `logs/`, and the
/// stderr sink at `warn` (`info` with `--verbose`, off with `--quiet`).
/// A log that cannot start is said once, and the run goes on without it.
pub(crate) fn start_log(run: &Run, env: &Env, name: &str) -> Option<Arc<LogHandle>> {
    let clock = Arc::new(SystemClock::new());
    let mut options = LogOptions::new(clock);
    let level = run
        .log_level
        .clone()
        .or_else(|| env.get("CINERIS_LOG").map(str::to_owned));
    if let Some(level) = level {
        match Filter::parse(&level) {
            Ok(filter) => options.filter = filter,
            Err(error) if !run.quiet => {
                eprintln!("{name}: the log level `{level}` is not understood: {error}");
            }
            Err(_) => {}
        }
    }
    options.file = StateDir::from_dirs(&Dirs::from_env(env))
        .map(|state| FileOptions::in_dir(state.logs_dir()));
    if !run.quiet {
        let level = if run.verbose {
            LevelFilter::INFO
        } else {
            LevelFilter::WARN
        };
        options.stderr = Some((level, Box::new(io::stderr())));
    }
    options.secrets = Arc::new(SecretFilter::new());
    let (subscriber, handle) = match log::start(options) {
        Ok(started) => started,
        Err(error) => {
            if !run.quiet {
                eprintln!("{name}: the log file could not be opened: {error}");
            }
            return None;
        }
    };
    if tracing::subscriber::set_global_default(subscriber).is_err() {
        return None;
    }
    Some(Arc::new(handle))
}

/// Writes each line to standard output and flushes it. Stops writing (and
/// keeps taking lines, so the core never waits on it) once standard output
/// is closed.
fn write_lines(mut lines: mpsc::Receiver<String>) {
    let stdout = io::stdout();
    let mut open = true;
    while let Some(line) = lines.blocking_recv() {
        if !open {
            continue;
        }
        let mut out = stdout.lock();
        if out
            .write_all(line.as_bytes())
            .and_then(|()| out.flush())
            .is_err()
        {
            open = false;
        }
    }
}

fn interrupted_error() -> ErrorData {
    ErrorData::new(
        ErrorKind::Interrupted,
        "interrupted",
        "the run was interrupted before its prompt was sent",
    )
}

fn elapsed(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use cineris_protocol::item::CommandSource;
    use cineris_protocol::methods::{CommandInfo, CommandsListResult};

    use super::*;

    /// A `/name` is sent as a command when the session lists it, or when it
    /// is an extension's (`extension:command`) that neither list has: an
    /// extension that is not loaded is in neither, and the core says why.
    /// Any other `/name` the session does not have is text.
    #[test]
    fn an_extensions_command_no_list_has_is_sent_as_a_command_and_any_other_is_text() {
        let listed = CommandsListResult {
            commands: vec![CommandInfo {
                name: "review".to_owned(),
                source: CommandSource::User,
                extension: None,
                description: String::new(),
                arguments: Vec::new(),
                path: Some("~/.config/cineris/commands/review.md".to_owned()),
            }],
            not_used: Vec::new(),
        };
        let said = sending("review", Some(&listed)).unwrap_or_default();
        assert!(said.contains("your command"), "{said}");
        let said = sending("tracker:deploy", Some(&listed)).unwrap_or_default();
        assert!(said.contains("does not list"), "{said}");
        assert_eq!(sending("deploy", Some(&listed)), None);
        assert!(sending("deploy", None).is_some());
    }

    fn piped(piped: &str, words: &str) -> Prompt {
        Prompt {
            piped: piped.to_owned(),
            words: words.to_owned(),
        }
    }

    /// What standard input gives is never a command, whatever its first
    /// line: `gh issue view 12 | cineris -p "triage"`, whose body begins
    /// `/tracker:deploy prod`, sends the issue to the model as text, and
    /// runs no extension's script with it as its arguments. The words after
    /// `-p` are read for one with or without it.
    #[test]
    fn a_piped_prompts_first_line_is_never_a_command() {
        let issue = "/tracker:deploy prod\n\nThe deploy fails on a cold cache.";
        assert_eq!(piped(issue, "triage").command(), None);
        assert_eq!(piped(issue, "").command(), None);
        assert_eq!(piped("/review", "").command(), None);
        assert_eq!(
            piped(issue, "/review security").command(),
            Some(("review", "security"))
        );
        assert_eq!(piped("", "/review").command(), Some(("review", "")));
        let start = piped(issue, "triage").start(SessionId::new("s_1"), None);
        assert_eq!(
            (start.input, start.command),
            (vec![Part::text(format!("{issue}\n\ntriage"))], None)
        );
    }

    /// A name that holds a `/` is a path, never a command: `cc main.c 2>&1
    /// | cineris -p "why does this fail?"`, whose log begins with the
    /// linker's `/usr/bin/ld: cannot find -lfoo`, is text for the model, and
    /// so is that line given after `-p` itself, where its name would have
    /// the form of an extension's command (`usr/bin/ld:`).
    #[test]
    fn the_linkers_line_is_never_a_command() {
        let log = "/usr/bin/ld: cannot find -lfoo\ncollect2: error: ld returned 1 exit status";
        assert_eq!(piped("", log).command(), None);
        assert_eq!(piped("", "/usr/bin/ld: cannot find -lfoo").command(), None);
        assert_eq!(piped("", "/./review").command(), None);
        assert_eq!(piped(log, "why does this fail?").command(), None);
        assert_eq!(piped(log, "").command(), None);
    }

    /// `git diff | cineris -p "/review"` sends `/review` as the command,
    /// and the diff in `input` beside it, for the core to put after the
    /// command's expanded text as one prompt; with nothing piped, `input`
    /// is empty, as `turn/start` takes a command alone.
    #[test]
    fn a_command_with_piped_input_sends_both() {
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\n+fn added() {}";
        let prompt = piped(diff, "/review");
        let (name, args) = prompt.command().expect("`/review` is a command");
        let call = CommandCall {
            name: name.to_owned(),
            args: args.to_owned(),
        };
        let start = prompt.start(SessionId::new("s_1"), Some(call.clone()));
        assert_eq!(
            (start.input, start.command),
            (vec![Part::text(diff)], Some(call.clone()))
        );
        let start = piped("", "/review").start(SessionId::new("s_1"), Some(call.clone()));
        assert_eq!((start.input, start.command), (Vec::new(), Some(call)));
    }
}
