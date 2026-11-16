//! Interactive mode: `cortex` with no `-p` opens the terminal UI on a
//! session (`docs/architecture/09-tui.md`; step 23).
//!
//! **Threads.** The main thread is the interface's: it enters the terminal
//! and runs `cortex_tui::app`'s loop, and never waits on the core. The core
//! runs on a thread of its own, on a current-thread Tokio runtime with its
//! timer and its I/O driver on, as headless mode's does. Between them is
//! the in-process [`Link`]:
//!
//! - **Operations** go over an unbounded channel to one task on the core
//!   thread, which carries out those that change the session in the order
//!   they were sent, and the reads (`session/list`, `session/read`,
//!   `models/list`, `config/get`, …) beside them, each on a task of its own.
//!   Each result is queued for the interface, and the interface woken.
//! - **Events and requests** are read by the interface straight from its
//!   client's `EventQueue`, which wakes it when it stops being empty: the
//!   queue's merging of deltas and its `item.resync` work for an interface
//!   that is behind.
//! - **Answers** to requests are `Core::respond`, on the interface's thread:
//!   it takes a lock held only to change a table, and sends on a channel.
//!
//! **Start-up,** in the order 15-efficiency's "Start-up" asks: the
//! configuration of the user's own layers (for `[terminal]`, `[tui]`,
//! `[keys]` and `log.level`); the log; the core, given that configuration,
//! started on its thread and waited for, so that an invalid catalog stops
//! the program before the terminal is touched; the terminal; the first
//! frame; and only then the session, which the application opens itself.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use cortex_base::log::{self, FileOptions, Filter, LogHandle, LogOptions};
use cortex_base::{CancelToken, SecretFilter, SystemClock};
use cortex_config::schema::{
    ColorChoice, DensityChoice, GlyphChoice, KeySettings, ReasoningShown, TerminalSettings,
    ToolOutputChoice, TuiSettings,
};
use cortex_config::{Dirs, Env, LoadOptions, Loaded, StateDir};
use cortex_core::{ClientId, ClientInfo, Core, CoreOptions, EventQueue, Outgoing, TokioTimer};
use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::methods::{Op, ServerResponse};
use cortex_protocol::session::SessionSettings;
use cortex_protocol::{ErrorData, ErrorKind, SessionId};
use cortex_tui::app::{
    self, ColorDepth, Exit, Inbound, Link, Open, Overrides, Start, Ticket, Waker,
};
use cortex_tui::highlight::Highlighter;
use cortex_tui::history::MemoryHistory;
use cortex_tui::keys::KeyMap;
use cortex_tui::view::{Density, Reasoning, ToolOutput};

use crate::args::{Choice, Format, Run};
use crate::exit;

/// How long the end of the program waits for the core thread to finish:
/// the core's own shutdown waits at most 2 s for the statistics' writer.
const CORE_END: Duration = Duration::from_secs(5);

/// How long the end waits for the operation the lane was running once the
/// interface is gone and what it was asked is withdrawn: one stuck on its
/// disk does not hold the rest of the end.
const LANE_END: Duration = Duration::from_secs(1);

/// How long the core's end waits after a signal: the watchdog ends the
/// process 3 s after the signal came.
const SIGNAL_END: Duration = Duration::from_secs(2);

/// Runs the interactive application. Returns the exit code.
pub(crate) fn run(run: Run, name: &'static str, _started: Instant) -> u8 {
    if let Some(flag) = headless_only(&run) {
        eprintln!(
            "{name}: {flag} is for a run with -p; the interactive application does not take it."
        );
        return exit::USAGE;
    }
    let env = Env::from_process();
    let workspace = match std::env::current_dir() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("{name}: the working directory could not be read: {error}");
            return exit::USAGE;
        }
    };

    // The user's configuration: the interface's own settings come from it,
    // and the core is given it rather than reading it again.
    let mut load = LoadOptions::new(env.clone());
    load.profile.clone_from(&run.common.profile);
    load.overrides = crate::headless::overrides(&run);
    let loaded = match cortex_config::load(&load) {
        Ok(loaded) => loaded,
        Err(failure) => {
            eprintln!("{name}: {}", crate::visible(&failure.render(&env)));
            return exit::USAGE;
        }
    };
    let log = start_log(&run, &env, &loaded);
    let logs = StateDir::from_dirs(&Dirs::from_env(&env)).map(|state| state.logs_dir());
    install_panic_hook(logs.clone());

    // A rule the command line gives that cannot be read stops the program,
    // as it does a headless run: a deny that was mistyped must not be lost.
    let options = match core_options(&run, &env, &loaded, log.as_ref()) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{name}: {}", crate::visible(&message));
            return exit::USAGE;
        }
    };
    // The core, on its thread, waited for.
    let core = match CoreThread::start(options) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("{name}: {}", crate::visible(&error.message));
            return exit::for_outcome(
                &exit::outcome_for_kind(&error.kind),
                Some(&error.kind),
                None,
            );
        }
    };

    let mut notices = Vec::new();
    let settings = tui_settings(&loaded.settings.tui, &mut notices);
    let (keymap, errors) = keymap(&loaded.settings.keys);
    notices.extend(errors);
    let overrides = overrides(&loaded.settings.terminal);
    let folder = shown_folder(&workspace, env.home().as_deref());
    let open = open(&run, &workspace);
    let root = workspace.to_string_lossy().into_owned();
    let prompt = run.prompt.clone();
    let link = core.link();
    let history = history_for(&env, &workspace);

    // A panic of the interface's own unwinds to here: the terminal has been
    // put back by then (the terminal layer's hook, and its drop), and the
    // core is still ended in order and the report's pointer printed.
    let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        app::interactive(overrides, move |waker, _caps| {
            link.slot.set(waker.clone());
            let wake = waker.clone();
            let start = Start {
                version: crate::VERSION.to_owned(),
                folder,
                workspace: root,
                open,
                prompt,
                settings,
                keymap,
                notices,
                history,
                highlighter: Highlighter::new(move || wake.wake()),
            };
            (Box::new(link) as Box<dyn Link>, start)
        })
    }));
    // After a signal the watchdog gives 3 s in all: the core has 2.
    let signalled = matches!(&ended, Ok(Ok(Exit::Signal(_))));
    core.end(if signalled { SIGNAL_END } else { CORE_END });
    if let Some(log) = &log {
        log.flush();
    }
    let (code, said) = ending(
        ended,
        CRASH.get().map(PathBuf::as_path),
        name,
        cortex_tui_terminal,
    );
    if let Some(said) = said {
        eprintln!("{said}");
    }
    // A signal's watchdog on Windows waits for this.
    cortex_tui::app::signal_ended();
    code
}

/// The exit code of the interface's end, and the line to print for it. A
/// panic of the interface (`ended` is `Err`) is exit code 1 with a pointer
/// to its report, when one was written; `terminal` says whether standard
/// input and output are a terminal, asked only when the terminal failed.
fn ending(
    ended: std::thread::Result<io::Result<Exit>>,
    crash: Option<&Path>,
    name: &str,
    terminal: impl FnOnce() -> bool,
) -> (u8, Option<String>) {
    let ended = match ended {
        Ok(ended) => ended,
        Err(_) => {
            let said = match crash {
                Some(report) => format!(
                    "{name}: the interface failed; the report is in {}",
                    crate::visible(&report.display().to_string())
                ),
                None => format!("{name}: the interface failed; the log file has the details"),
            };
            return (exit::INTERNAL, Some(said));
        }
    };
    match ended {
        Ok(Exit::Quit) => (exit::COMPLETED, None),
        // Ended by a signal, gracefully: the code the signal's own end
        // would have given.
        Ok(Exit::Signal(code)) => (code, None),
        Ok(Exit::Gone(error)) => (
            exit::INTERNAL,
            Some(format!("{name}: {}", crate::visible(&error.message))),
        ),
        Err(error) if error.kind() == io::ErrorKind::Unsupported || !terminal() => (
            exit::USAGE,
            Some(format!(
                "{name}: the interactive application needs a terminal ({error}). Run a prompt \
                 headless with `{name} -p \"…\"`."
            )),
        ),
        Err(error) => (
            exit::INTERNAL,
            Some(format!("{name}: the terminal failed: {error}")),
        ),
    }
}

/// The editor's history: the state directory's, for this project, read on
/// a thread of its own; in memory when there is no state directory.
fn history_for(env: &Env, workspace: &Path) -> Box<dyn cortex_tui::history::HistoryStore + Send> {
    let state = StateDir::from_dirs(&Dirs::from_env(env));
    let project = cortex_base::PathResolver::new(workspace, cortex_base::PathOptions::new())
        .map(|resolver| resolver.root().project_key().into_string());
    match (state, project) {
        (Some(state), Ok(project)) => Box::new(crate::history::FileHistory::open(
            state.history_file(),
            project,
        )),
        _ => Box::new(MemoryHistory::new()),
    }
}

/// Whether standard input and output are a terminal: the interface needs
/// both.
fn cortex_tui_terminal() -> bool {
    use std::io::IsTerminal as _;
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// The first flag given that only a headless run takes.
fn headless_only(run: &Run) -> Option<&'static str> {
    if run.format != Format::Text {
        Some("--output")
    } else if run.include_deltas {
        Some("--include-deltas")
    } else if run.fail_on_denial {
        Some("--fail-on-denial")
    } else if run.quiet {
        Some("--quiet")
    } else {
        None
    }
}

/// The session the application opens.
fn open(run: &Run, workspace: &Path) -> Open {
    let workspace = workspace.to_string_lossy().into_owned();
    match &run.choice {
        Choice::New => Open::New {
            workspace,
            settings: SessionSettings::default(),
            ephemeral: run.no_session,
        },
        Choice::Continue => Open::Continue {
            workspace,
            settings: SessionSettings::default(),
            ephemeral: run.no_session,
        },
        Choice::Resume(id) => Open::Resume(SessionId::from(id.as_str())),
        Choice::Fork(id) => Open::Fork(SessionId::from(id.as_str())),
    }
}

/// The workspace as the person writes it: `~/src/app` under the home
/// directory.
fn shown_folder(workspace: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Some(rest) = cortex_base::strip_location_prefix(workspace, home)
    {
        let rest = rest.to_string_lossy().replace('\\', "/");
        return if rest.is_empty() {
            "~".to_owned()
        } else {
            format!("~/{rest}")
        };
    }
    workspace.to_string_lossy().into_owned()
}

/// The core's options: as headless mode's, but for a person at a terminal.
fn core_options(
    run: &Run,
    env: &Env,
    loaded: &Loaded,
    log: Option<&Arc<LogHandle>>,
) -> Result<CoreOptions, String> {
    let mut options = CoreOptions::new(env.clone(), Arc::new(TokioTimer));
    options.profile.clone_from(&run.common.profile);
    options.overrides = crate::headless::overrides(run);
    options.loaded = Some(loaded.clone());
    for (texts, action) in [
        (&run.allow, cortex_protocol::lease::RuleAction::Allow),
        (&run.deny, cortex_protocol::lease::RuleAction::Deny),
    ] {
        for text in texts {
            options.rules.push(cortex_core::parse_rule(text, action)?);
        }
    }
    options.trust_project = run.trust_project;
    options.system_append.clone_from(&run.system_append);
    // A person at a terminal can stop the turn from a dialog, so a later
    // call with side effects waits for an open question.
    options.answers_may_stop = true;
    // The first request of an epoch waits up to 150 ms for the repository
    // map (`CoreOptions::map_wait`'s default for the interactive app).
    let guard = std::env::current_exe()
        .ok()
        .map(|exe| (exe, vec![OsString::from(crate::args::GUARD)]));
    options.launcher = guard
        .as_ref()
        .map(|(exe, _)| (exe.clone(), vec![OsString::from(crate::args::SANDBOX)]));
    options.guard = guard;
    options.cancel = CancelToken::new();
    if let Some(log) = log {
        let log = Arc::clone(log);
        options.key_found = Some(Arc::new(move |secret: &cortex_base::Secret| {
            log.add_secret(secret);
        }));
    }
    Ok(options)
}

/// The `[tui]` settings in the application's terms; a theme it does not
/// know is said.
fn tui_settings(tui: &TuiSettings, notices: &mut Vec<String>) -> app::Settings {
    let theme = match tui.theme.as_str() {
        "auto" => app::ThemeChoice::Auto,
        "dark" => app::ThemeChoice::Dark,
        "light" => app::ThemeChoice::Light,
        other => {
            notices.push(format!(
                "There is no theme `{}`: the themes are auto, dark and light. The automatic \
                 one is used.",
                cortex_base::visible(other)
            ));
            app::ThemeChoice::Auto
        }
    };
    app::Settings {
        prose_width: usize::try_from(tui.prose_width).unwrap_or(80),
        tool_detail: usize::try_from(tui.tool_detail).unwrap_or(12),
        tool_output: match tui.tool_output {
            ToolOutputChoice::Auto => ToolOutput::Auto,
            ToolOutputChoice::Always => ToolOutput::Always,
            ToolOutputChoice::Never => ToolOutput::Never,
        },
        reasoning: match tui.reasoning {
            ReasoningShown::Summary => Reasoning::Summary,
            ReasoningShown::Duration => Reasoning::Duration,
        },
        density: match tui.density {
            DensityChoice::Comfortable => Density::Comfortable,
            DensityChoice::Compact => Density::Compact,
        },
        turn_summary: tui.turn_summary,
        reduced_motion: tui.reduced_motion,
        glyphs: match tui.glyphs {
            GlyphChoice::Auto => app::GlyphSet::Auto,
            GlyphChoice::Unicode => app::GlyphSet::Unicode,
            GlyphChoice::Ascii => app::GlyphSet::Ascii,
        },
        theme,
    }
}

/// The key map from `[keys]`, and each entry it could not use, as a notice.
fn keymap(keys: &KeySettings) -> (KeyMap, Vec<String>) {
    let mut table: Vec<(String, Vec<String>)> = Vec::new();
    for (context, bindings) in [
        ("editor", &keys.editor),
        ("menu", &keys.menu),
        ("picker", &keys.picker),
        ("toggle", &keys.toggle),
        ("search", &keys.search),
        ("focus", &keys.focus),
        ("dialog", &keys.dialog),
        ("global", &keys.global),
    ] {
        for (action, keys) in &bindings.0 {
            table.push((format!("{context}.{action}"), keys.clone()));
        }
    }
    let (keymap, errors) = KeyMap::with_table(
        table
            .iter()
            .map(|(name, keys)| (name.as_str(), keys.as_slice())),
    );
    let notices = errors
        .into_iter()
        .map(|error| {
            format!(
                "{}; that action keeps its own keys.",
                cortex_base::visible(&error.to_string())
            )
        })
        .collect();
    (keymap, notices)
}

/// The `[terminal]` overrides.
fn overrides(terminal: &TerminalSettings) -> Overrides {
    Overrides {
        sync: terminal.sync,
        keyboard: terminal.keyboard,
        color: terminal.color.map(|color| match color {
            ColorChoice::None => ColorDepth::Mono,
            ColorChoice::Ansi16 => ColorDepth::Ansi16,
            ColorChoice::Indexed256 => ColorDepth::Indexed256,
            ColorChoice::TrueColor => ColorDepth::TrueColor,
        }),
        hyperlinks: terminal.hyperlinks,
        unicode: terminal.unicode,
    }
}

/// Starts the log: the file alone, since standard error is the terminal.
/// Its level is `--log-level`, then `CORTEX_LOG`, then `log.level`.
fn start_log(run: &Run, env: &Env, loaded: &Loaded) -> Option<Arc<LogHandle>> {
    let clock = Arc::new(SystemClock::new());
    let mut options = LogOptions::new(clock);
    let level = run
        .log_level
        .clone()
        .or_else(|| env.get("CORTEX_LOG").map(str::to_owned))
        .or_else(|| loaded.settings.log.level.clone());
    if let Some(level) = level
        && let Ok(filter) = Filter::parse(&level)
    {
        options.filter = filter;
    }
    options.file = StateDir::from_dirs(&Dirs::from_env(env))
        .map(|state| FileOptions::in_dir(state.logs_dir()));
    options.stderr = None;
    options.secrets = Arc::new(SecretFilter::new());
    let (subscriber, handle) = log::start(options).ok()?;
    tracing::subscriber::set_global_default(subscriber).ok()?;
    Some(Arc::new(handle))
}

// ------------------------------------------------------------------ panics

/// The thread the interface runs on: a panic there puts the terminal back
/// (the terminal layer's hook) and ends the program.
static INTERFACE: OnceLock<std::thread::ThreadId> = OnceLock::new();
/// Where the report of the interface's panic was written.
static CRASH: OnceLock<PathBuf> = OnceLock::new();

/// The panic hook of interactive mode (`docs/architecture/02-runtime.md`,
/// "Panics"): a report with a backtrace in the log directory, and the
/// panic in the log. Nothing is written to the terminal from here: the
/// interface's panic has put it back by the time this runs (the terminal
/// layer's hook runs first, then this one), and its one-line pointer is
/// printed when the program ends; any other thread's panic, which the core
/// may have caught and gone on from, leaves the terminal the interface's,
/// so the default hook's message would land in the middle of a frame.
fn install_panic_hook(logs: Option<PathBuf>) {
    let _ = INTERFACE.set(std::thread::current().id());
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed").to_owned();
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(thread = %name, panic = %info, "a thread panicked");
        if INTERFACE.get() != Some(&thread.id()) {
            return;
        }
        let Some(logs) = &logs else {
            return;
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_millis())
            .unwrap_or(0);
        let path = logs.join(format!("crash-{stamp}.txt"));
        let written = cortex_base::private::create_dir_all(logs)
            .and_then(|()| cortex_base::private::create_new(&path))
            .and_then(|mut file| {
                writeln!(
                    file,
                    "cortex {} panicked on its {name} thread:\n{info}\n\n{backtrace}",
                    crate::VERSION
                )
            });
        if written.is_ok() {
            let _ = CRASH.set(path);
        }
    }));
}

// --------------------------------------------------------------- the core

/// Where the link's waker goes once the terminal has one: what arrives
/// before then is drained on the interface's first wake.
#[derive(Clone, Default)]
struct Slot(Arc<Mutex<Option<Waker>>>);

impl Slot {
    fn set(&self, waker: Waker) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    fn wake(&self) {
        let waker = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// The results waiting for the interface.
type Done = Arc<Mutex<VecDeque<Inbound>>>;

fn push(done: &Done, inbound: Inbound) {
    done.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push_back(inbound);
}

/// The core's thread.
struct CoreThread {
    core: Core,
    client: ClientId,
    queue: Arc<EventQueue>,
    ops: Option<tokio::sync::mpsc::UnboundedSender<(Ticket, Op)>>,
    done: Done,
    slot: Slot,
    ended: std::sync::mpsc::Receiver<()>,
}

/// Says the core is gone when the core thread ends any way but the one it
/// was told to: its link reports it, and the interface ends.
struct GoneGuard {
    done: Done,
    slot: Slot,
    normal: Arc<AtomicBool>,
}

impl Drop for GoneGuard {
    fn drop(&mut self) {
        if !self.normal.load(Ordering::SeqCst) {
            push(
                &self.done,
                Inbound::Gone(ErrorData::new(
                    ErrorKind::Internal,
                    "core_gone",
                    "The core stopped; the log file has the details.",
                )),
            );
            self.slot.wake();
        }
    }
}

/// Where an operation runs on the core thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// On a task of its own, beside the lane.
    Beside,
    /// In the lane, in the order sent; `write` when it changes a session.
    Lane { write: bool },
}

/// How the core thread orders what it is sent. What changes a session runs
/// in the lane, one at a time in the order sent. A read runs beside it,
/// unless a write sent before it is not done yet: then it goes into the
/// lane after that write, so that a read sees every write sent before it
/// (`/permissions`' list after a revoke, `/status` after `/model`), as when
/// the loop ran each write itself (the review of step 23's second pull
/// request).
#[derive(Debug, Clone, Default)]
struct Routes {
    /// The writes routed and not yet done.
    in_flight: Arc<AtomicUsize>,
}

impl Routes {
    /// Where `op` runs.
    fn route(&self, op: &Op) -> Route {
        if !reads(op) {
            self.in_flight.fetch_add(1, Ordering::SeqCst);
            return Route::Lane { write: true };
        }
        if self.in_flight.load(Ordering::SeqCst) > 0 {
            Route::Lane { write: false }
        } else {
            Route::Beside
        }
    }

    /// A write the lane ran is done.
    fn written(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Whether an operation only reads, and so may run beside the ordered ones.
fn reads(op: &Op) -> bool {
    matches!(
        op,
        Op::SessionList(_)
            | Op::SessionRead(_)
            | Op::WorkspaceFiles(_)
            | Op::ModelsList(_)
            | Op::ConfigGet(_)
            | Op::PermissionsList(_)
            | Op::PlanGet(_)
            | Op::UsageGet(_)
            | Op::UsageReport(_)
            | Op::McpStatus(_)
            | Op::ExtensionsStatus(_)
    )
}

impl CoreThread {
    /// Starts the core on a thread of its own and waits until it has
    /// started, or failed to.
    fn start(options: CoreOptions) -> Result<Self, Box<ErrorData>> {
        let (started, starting) = std::sync::mpsc::channel();
        let (ops, mut lane) = tokio::sync::mpsc::unbounded_channel::<(Ticket, Op)>();
        let (end, ended) = std::sync::mpsc::channel();
        let done: Done = Arc::new(Mutex::new(VecDeque::new()));
        let slot = Slot::default();
        let (thread_done, thread_slot) = (Arc::clone(&done), slot.clone());
        let spawned = std::thread::Builder::new()
            .name("cortex-core".to_owned())
            .spawn(move || {
                let normal = Arc::new(AtomicBool::new(false));
                let _guard = GoneGuard {
                    done: Arc::clone(&thread_done),
                    slot: thread_slot.clone(),
                    normal: Arc::clone(&normal),
                };
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        normal.store(true, Ordering::SeqCst);
                        let _ = started.send(Err(Box::new(ErrorData::new(
                            ErrorKind::Internal,
                            "no_runtime",
                            format!("the core thread's runtime did not start: {error}"),
                        ))));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let core = match Core::start(options) {
                        Ok(core) => core,
                        Err(error) => {
                            normal.store(true, Ordering::SeqCst);
                            let _ = started.send(Err(error));
                            return;
                        }
                    };
                    let client = core.connect(ClientInfo::user());
                    let _ = started.send(Ok((core.clone(), client.id, Arc::clone(&client.queue))));
                    // The lane: what changes a session runs on a task of
                    // its own, one at a time in the order sent, so that the
                    // end below is never behind it.
                    let (writes, mut written) =
                        tokio::sync::mpsc::unbounded_channel::<(Ticket, Op, bool)>();
                    let routes = Routes::default();
                    let lane_task = {
                        let core = core.clone();
                        let (done, wake) = (Arc::clone(&thread_done), thread_slot.clone());
                        let routes = routes.clone();
                        tokio::spawn(async move {
                            while let Some((ticket, op, write)) = written.recv().await {
                                let result = core.handle(client.id, op).await.map_err(|e| *e);
                                if write {
                                    routes.written();
                                }
                                push(&done, Inbound::Done(ticket, result));
                                wake.wake();
                            }
                        })
                    };
                    while let Some((ticket, op)) = lane.recv().await {
                        let route = routes.route(&op);
                        if let Route::Lane { write } = route {
                            let _ = writes.send((ticket, op, write));
                        } else {
                            let core = core.clone();
                            let (done, wake) = (Arc::clone(&thread_done), thread_slot.clone());
                            tokio::spawn(async move {
                                let result = core.handle(client.id, op).await.map_err(|e| *e);
                                push(&done, Inbound::Done(ticket, result));
                                wake.wake();
                            });
                        }
                    }
                    // The interface is gone, outside the lane: what it was
                    // asked and never answered is withdrawn, which ends an
                    // operation waiting on the answer (the trust question of
                    // an open, the undo's question), and the sessions it
                    // had open are closed.
                    drop(writes);
                    core.disconnect(client.id).await;
                    // Then the lane's last operation, for so long, and
                    // everything else in order.
                    let _ = tokio::time::timeout(LANE_END, lane_task).await;
                    core.shutdown().await;
                    normal.store(true, Ordering::SeqCst);
                });
                let _ = end.send(());
            });
        if let Err(error) = spawned {
            return Err(Box::new(ErrorData::new(
                ErrorKind::Internal,
                "no_thread",
                format!("the core thread could not be started: {error}"),
            )));
        }
        let (core, client, queue) = match starting.recv() {
            Ok(Ok(started)) => started,
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(Box::new(ErrorData::new(
                    ErrorKind::Internal,
                    "core_panicked",
                    "the core thread failed as it started; the log file has the details",
                )));
            }
        };
        let wake = slot.clone();
        queue.set_wake(Arc::new(move || wake.wake()));
        Ok(Self {
            core,
            client,
            queue,
            ops: Some(ops),
            done,
            slot,
            ended,
        })
    }

    fn link(&self) -> InProcess {
        InProcess {
            core: self.core.clone(),
            client: self.client,
            queue: Arc::clone(&self.queue),
            ops: self.ops.clone(),
            done: Arc::clone(&self.done),
            slot: self.slot.clone(),
            next: 0,
            gone_said: false,
        }
    }

    /// Ends the core: no more operations, then its shutdown, waited for at
    /// most [`CORE_END`].
    fn end(mut self, within: Duration) {
        self.ops = None;
        let _ = self.ended.recv_timeout(within);
    }
}

/// The in-process link. See the [module documentation](self).
struct InProcess {
    core: Core,
    client: ClientId,
    queue: Arc<EventQueue>,
    ops: Option<tokio::sync::mpsc::UnboundedSender<(Ticket, Op)>>,
    done: Done,
    slot: Slot,
    next: u64,
    /// Whether the end of the connection was told.
    gone_said: bool,
}

impl Link for InProcess {
    fn send(&mut self, op: Op) -> Ticket {
        self.next += 1;
        let ticket = Ticket(self.next);
        let sent = self
            .ops
            .as_ref()
            .is_some_and(|ops| ops.send((ticket, op)).is_ok());
        if !sent {
            push(
                &self.done,
                Inbound::Done(
                    ticket,
                    Err(ErrorData::new(
                        ErrorKind::Internal,
                        "core_gone",
                        "The core has stopped.",
                    )),
                ),
            );
            self.slot.wake();
        }
        ticket
    }

    fn respond(&mut self, request: RequestId, response: ServerResponse) {
        if let Err(error) = self.core.respond(self.client, &request, response) {
            // Settled meanwhile: the core says how in an event.
            tracing::debug!(code = %error.code, "an answer was not taken");
        }
    }

    fn next(&mut self) -> Option<Inbound> {
        if let Some(done) = self
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        {
            return Some(done);
        }
        let next = self.queue.try_next();
        inbound(next, self.queue.is_closed(), &mut self.gone_said)
    }
}

/// What the link gives for what the client's queue gave: an event, a
/// request, or, once the queue is closed and empty, the end of the
/// connection, said once.
fn inbound(next: Option<Outgoing>, closed: bool, gone_said: &mut bool) -> Option<Inbound> {
    match next {
        Some(Outgoing::Event(event)) => Some(Inbound::Event(event)),
        Some(Outgoing::Request { id, request }) => Some(Inbound::Request(id, request)),
        // The core disconnected this client: nothing more will come.
        None if closed && !*gone_said => {
            *gone_said = true;
            Some(Inbound::Gone(ErrorData::new(
                ErrorKind::Internal,
                "client_closed",
                "The core closed this interface's connection.",
            )))
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic of the interface is exit code 1, with the report's place;
    /// the core's end is not skipped for it (`run` ends the core first).
    #[test]
    fn a_panic_of_the_interface_is_exit_1_and_points_at_its_report() {
        let panicked: std::thread::Result<io::Result<Exit>> = Err(Box::new("boom"));
        let report = Path::new("logs").join("crash-1.txt");
        let (code, said) = ending(panicked, Some(&report), "cortex", || true);
        assert_eq!(code, exit::INTERNAL);
        let said = said.unwrap_or_default();
        assert!(said.contains("the interface failed"), "{said}");
        assert!(said.contains("crash-1.txt"), "{said}");
        let (code, said) = ending(Err(Box::new("boom")), None, "cortex", || true);
        assert_eq!(code, exit::INTERNAL);
        assert!(said.unwrap_or_default().contains("the log file"));
        assert_eq!(
            ending(Ok(Ok(Exit::Quit)), None, "cortex", || true),
            (exit::COMPLETED, None)
        );
        let (code, _) = ending(
            Ok(Err(io::Error::other("no terminal"))),
            None,
            "cortex",
            || false,
        );
        assert_eq!(code, exit::USAGE);
    }

    /// A queue the core closed is the end of the connection, said once,
    /// after what was in it.
    /// A read runs beside the lane while no write is under way, and after
    /// the writes sent before it while one is: a list after a revoke sees
    /// the revoke.
    #[test]
    fn a_read_after_a_write_waits_for_it_and_one_alone_runs_beside() {
        use cortex_protocol::methods::{PermissionsRevokeParams, SessionRef};
        let session = cortex_protocol::SessionId::from("s_1");
        let routes = Routes::default();
        let list = || {
            Op::PermissionsList(SessionRef {
                session_id: session.clone(),
            })
        };
        assert_eq!(routes.route(&list()), Route::Beside);
        let revoke = Op::PermissionsRevoke(PermissionsRevokeParams {
            session_id: session.clone(),
            ids: vec![cortex_protocol::ApprovalId::from("a_1")],
        });
        assert_eq!(routes.route(&revoke), Route::Lane { write: true });
        assert_eq!(routes.route(&list()), Route::Lane { write: false });
        routes.written();
        assert_eq!(routes.route(&list()), Route::Beside);
    }

    #[test]
    fn a_closed_queue_is_the_end_of_the_connection_said_once() {
        let mut said = false;
        assert!(inbound(None, false, &mut said).is_none());
        assert!(matches!(
            inbound(None, true, &mut said),
            Some(Inbound::Gone(error)) if error.code == "client_closed"
        ));
        assert!(inbound(None, true, &mut said).is_none(), "said once");
    }

    #[test]
    fn the_folder_is_shown_from_the_home_directory() {
        let home = Path::new(if cfg!(windows) {
            "D:\\people\\u"
        } else {
            "/srv/people/u"
        });
        let inside = home.join("src").join("app");
        assert_eq!(shown_folder(&inside, Some(home)), "~/src/app");
        assert_eq!(shown_folder(home, Some(home)), "~");
        let outside = Path::new(if cfg!(windows) {
            "D:\\work"
        } else {
            "/srv/work"
        });
        assert_eq!(shown_folder(outside, Some(home)), outside.to_string_lossy());
    }

    #[test]
    fn a_headless_flag_is_refused_by_name() {
        let mut run = Run {
            include_deltas: true,
            ..Run::default()
        };
        assert_eq!(headless_only(&run), Some("--include-deltas"));
        run.include_deltas = false;
        run.fail_on_denial = true;
        assert_eq!(headless_only(&run), Some("--fail-on-denial"));
        run.fail_on_denial = false;
        assert_eq!(headless_only(&Run::default()), None);
    }

    #[test]
    fn each_keys_table_reaches_the_key_map_and_a_bad_entry_is_said() {
        let mut keys = KeySettings::default();
        keys.dialog
            .0
            .insert("allow_once".to_owned(), vec!["o".to_owned()]);
        keys.editor
            .0
            .insert("no_such_action".to_owned(), vec!["ctrl+q".to_owned()]);
        let (map, notices) = keymap(&keys);
        assert_eq!(
            map.lookup(
                &[cortex_tui::keys::Context::Dialog],
                cortex_tui::keys::parse("o").unwrap()
            ),
            Some(cortex_tui::keys::Action::AllowOnce)
        );
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("editor.no_such_action"), "{notices:?}");
    }

    #[test]
    fn a_theme_that_is_not_known_is_said_and_the_automatic_one_used() {
        let tui = TuiSettings {
            theme: "dusk".to_owned(),
            ..TuiSettings::default()
        };
        let mut notices = Vec::new();
        let settings = tui_settings(&tui, &mut notices);
        assert_eq!(settings.theme, app::ThemeChoice::Auto);
        assert_eq!(notices.len(), 1);
    }
}
