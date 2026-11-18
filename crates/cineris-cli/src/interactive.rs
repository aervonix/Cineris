//! Interactive mode: `cineris` with no `-p` opens the terminal UI on a
//! session (`docs/architecture/09-tui.md`; step 23).
//!
//! **Threads.** The main thread is the interface's: it enters the terminal
//! and runs `cineris_tui::app`'s loop, and never waits on the core. The core
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use cineris_base::log::{self, FileOptions, Filter, LogHandle, LogOptions};
use cineris_base::{CancelToken, SecretFilter, SystemClock};
use cineris_config::schema::{
    BackspaceChoice, ColorChoice, DensityChoice, GlyphChoice, KeySettings, PresenterChoice,
    ReasoningShown, TerminalSettings, ToolOutputChoice, TuiSettings,
};
use cineris_config::{Dirs, Env, LoadOptions, Loaded, StateDir};
use cineris_core::{ClientId, ClientInfo, Core, CoreOptions, EventQueue, Outgoing, TokioTimer};
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::methods::{Op, ServerResponse};
use cineris_protocol::session::SessionSettings;
use cineris_protocol::{ErrorData, ErrorKind, SessionId};
use cineris_rpc::lane::Dispatcher;
use cineris_rpc::order::{Next, Order};
use cineris_tui::app::{
    self, ColorDepth, Exit, Inbound, Link, Open, Overrides, Start, Ticket, Waker,
};
use cineris_tui::highlight::Highlighter;
use cineris_tui::history::MemoryHistory;
use cineris_tui::keys::KeyMap;
use cineris_tui::view::{Density, Reasoning, ToolOutput};

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
/// process 3 s after the signal came, and before this the history's end has
/// waited at most 1 s for its last prompts, so the core has 1.5 s and the
/// rest is left for the log (the review of step 23's second pull request).
const SIGNAL_END: Duration = Duration::from_millis(1500);

/// How long the end waits for the log to reach its file: what is left of
/// the watchdog's 3 s after the history and the core (the review of step
/// 23: a log writer stuck on its disk held a quit for ever).
const LOG_END: Duration = Duration::from_millis(300);

/// How often the core's end looks for a signal that came meanwhile.
const END_LOOK: Duration = Duration::from_millis(50);

/// When the core's end stops waiting: `within` after it `began`, or
/// [`SIGNAL_END`] after a signal that came while it waited, whichever is
/// first.
fn end_deadline(began: Instant, within: Duration, signalled: Option<Instant>) -> Instant {
    let deadline = began + within;
    signalled.map_or(deadline, |at| deadline.min(at + SIGNAL_END))
}

/// Runs the interactive application. Returns the exit code.
pub(crate) fn run(run: Run, name: &'static str, _started: Instant) -> u8 {
    if let Some(flag) = headless_only(&run) {
        eprintln!(
            "{name}: {flag} is for a run with -p; the interactive application does not take it."
        );
        return exit::USAGE;
    }
    let env = Env::from_process();
    // The workspace root, found by walking up; the working directory below
    // it is where commands start.
    let here = match crate::workspace::Here::now() {
        Ok(here) => here,
        Err(error) => {
            eprintln!("{name}: the working directory could not be read: {error}");
            return exit::USAGE;
        }
    };
    let workspace = here.root.clone();

    // The user's configuration: the interface's own settings come from it,
    // and the core is given it rather than reading it again.
    let mut load = LoadOptions::new(env.clone());
    load.profile.clone_from(&run.common.profile);
    load.overrides = crate::headless::overrides(&run);
    let loaded = match cineris_config::load(&load) {
        Ok(loaded) => loaded,
        Err(failure) => {
            eprintln!("{name}: {}", crate::visible(&failure.render(&env)));
            return exit::USAGE;
        }
    };
    // One filter for the log and the crash report: what the log masks, the
    // report masks too.
    let secrets = Arc::new(SecretFilter::new());
    let log = start_log(&run, &env, &loaded, &secrets);
    let logs = StateDir::from_dirs(&Dirs::from_env(&env)).map(|state| state.logs_dir());
    install_panic_hook(logs.clone(), secrets);

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
    let settings = app_settings(&loaded.settings, &mut notices);
    let (keymap, errors) = keymap(&loaded.settings.keys);
    notices.extend(errors);
    notices.extend(here.passed_over_said());
    let overrides = overrides(&loaded.settings.terminal);
    let folder = shown_folder(&workspace, env.home().as_deref());
    let open = open(&run, &workspace);
    let root = workspace.to_string_lossy().into_owned();
    let prompt = run.prompt.clone();
    let link = core.link();
    let history = history_for(&env, &workspace);
    // `/export` writes in the folder the person started in.
    let started_in = here.cwd.clone().unwrap_or_else(|| workspace.clone());

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
                cwd: here
                    .cwd
                    .as_ref()
                    .and_then(|cwd| cwd.to_str().map(str::to_owned)),
                open,
                prompt,
                settings,
                keymap,
                notices,
                history,
                highlighter: Highlighter::new(move || wake.wake()),
                restyle: Some(Box::new(restyle)),
                export: Some(crate::export::writer(started_in)),
            };
            (Box::new(link) as Box<dyn Link>, start)
        })
    }));
    // After a signal the watchdog gives 3 s in all (`SIGNAL_END`); one
    // that comes while the core ends cuts its wait short alike.
    let signalled = matches!(&ended, Ok(Ok(Exit::Signal(_))));
    core.end(if signalled { SIGNAL_END } else { CORE_END });
    if let Some(log) = log {
        log.finish_within(LOG_END);
    }
    // A signal that came after the interface's loop ended is the code the
    // program ends with: the end it asked for is the one just done.
    let ended = match (ended, app::signalled()) {
        (Ok(Ok(Exit::Quit | Exit::Gone(_))), Some(code)) => Ok(Ok(Exit::Signal(code))),
        (ended, _) => ended,
    };
    let (code, said) = ending(
        ended,
        CRASH.get().map(PathBuf::as_path),
        name,
        cineris_tui_terminal,
    );
    if let Some(said) = said {
        eprintln!("{said}");
    }
    // A signal's watchdog on Windows waits for this.
    cineris_tui::app::signal_ended();
    code
}

/// The exit code of the interface's end, and the line to print for it. A
/// panic of the interface (`ended` is `Err`) is exit code 1 with a pointer
/// to its report, when one was written; `terminal` says whether standard
/// input and output are a terminal, asked only when the terminal failed.
pub(crate) fn ending(
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
pub(crate) fn history_for(
    env: &Env,
    workspace: &Path,
) -> Box<dyn cineris_tui::history::HistoryStore + Send> {
    let state = StateDir::from_dirs(&Dirs::from_env(env));
    let project = cineris_base::PathResolver::new(workspace, cineris_base::PathOptions::new())
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
pub(crate) fn cineris_tui_terminal() -> bool {
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
pub(crate) fn shown_folder(workspace: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Some(rest) = cineris_base::strip_location_prefix(workspace, home)
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
pub(crate) fn core_options(
    run: &Run,
    env: &Env,
    loaded: &Loaded,
    log: Option<&Arc<LogHandle>>,
) -> Result<CoreOptions, String> {
    let mut options = CoreOptions::new(env.clone(), Arc::new(TokioTimer));
    options.profile.clone_from(&run.common.profile);
    options.overrides = crate::headless::overrides(run);
    options.loaded = Some(loaded.clone());
    // A reload of `log.level` changes the log's level, unless the command
    // line or `CINERIS_LOG` chose it, which outrank the file (step 24).
    if run.log_level.is_none() && env.get("CINERIS_LOG").is_none() {
        options.log = log.cloned();
    }
    for (texts, action) in [
        (&run.allow, cineris_protocol::lease::RuleAction::Allow),
        (&run.deny, cineris_protocol::lease::RuleAction::Deny),
    ] {
        for text in texts {
            options.rules.push(cineris_core::parse_rule(text, action)?);
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
        options.key_found = Some(Arc::new(move |secret: &cineris_base::Secret| {
            log.add_secret(secret);
        }));
    }
    Ok(options)
}

/// The interface's settings from the configuration: `[tui]`, and the
/// mouse from `[terminal]` (since step 25). Both interfaces, the in-process
/// one and `cineris attach`, take theirs from here, so neither can miss a
/// key the other reads (the review of step 25 found `attach` without the
/// mouse).
pub(crate) fn app_settings(
    settings: &cineris_config::schema::Settings,
    notices: &mut Vec<String>,
) -> app::Settings {
    let mut app = tui_settings(&settings.tui, notices);
    app.mouse = settings.terminal.mouse.unwrap_or(true);
    app
}

/// The `[tui]` settings in the application's terms; a theme it does not
/// know is said.
pub(crate) fn tui_settings(tui: &TuiSettings, notices: &mut Vec<String>) -> app::Settings {
    let theme = match tui.theme.as_str() {
        "auto" => app::ThemeChoice::Auto,
        "dark" => app::ThemeChoice::Dark,
        "light" => app::ThemeChoice::Light,
        other => {
            notices.push(format!(
                "There is no theme `{}`: the themes are auto, dark and light. The automatic \
                 one is used.",
                cineris_base::visible(other)
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
        presenter: match tui.presenter {
            PresenterChoice::Inline => app::Presenter::Inline,
            PresenterChoice::Fullscreen => app::Presenter::Fullscreen,
        },
        // `[terminal]`'s, set by the caller.
        mouse: true,
    }
}

/// The look and the key map read again from `config/get`'s answer for the
/// `tui` and `keys` sections, after a reload applied a key of them (step
/// 24): built as at the start, what cannot be used said again. A section
/// that does not read leaves both as they were, and says why.
pub(crate) fn restyle(
    config: &serde_json::Map<String, serde_json::Value>,
) -> Result<app::Restyled, String> {
    let unread = |error: String| {
        format!(
            "The look and the keys stay as they were: the configuration could not be read again \
             ({}).",
            cineris_base::visible(&error)
        )
    };
    let tui = cineris_config::schema::read_section::<TuiSettings>(config, "tui").map_err(unread)?;
    let keys =
        cineris_config::schema::read_section::<KeySettings>(config, "keys").map_err(unread)?;
    let mut look_notices = Vec::new();
    let settings = tui_settings(&tui, &mut look_notices);
    let (keymap, key_notices) = keymap(&keys);
    Ok(app::Restyled {
        settings,
        keymap,
        look_notices,
        key_notices,
    })
}

/// Every table of `[keys]`, by its name: each must be here, or its
/// bindings are dropped with no word (the review of step 25 found
/// `viewer` and `screen` missing).
fn key_tables(keys: &KeySettings) -> [(&'static str, &cineris_config::schema::Bindings); 11] {
    [
        ("editor", &keys.editor),
        ("menu", &keys.menu),
        ("picker", &keys.picker),
        ("toggle", &keys.toggle),
        ("fork", &keys.fork),
        ("search", &keys.search),
        ("focus", &keys.focus),
        ("dialog", &keys.dialog),
        ("viewer", &keys.viewer),
        ("screen", &keys.screen),
        ("global", &keys.global),
    ]
}

/// The key map from `[keys]`, and each entry it could not use, as a notice.
pub(crate) fn keymap(keys: &KeySettings) -> (KeyMap, Vec<String>) {
    let mut table: Vec<(String, Vec<String>)> = Vec::new();
    for (context, bindings) in key_tables(keys) {
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
                cineris_base::visible(&error.to_string())
            )
        })
        .collect();
    (keymap, notices)
}

/// The `[terminal]` overrides.
pub(crate) fn overrides(terminal: &TerminalSettings) -> Overrides {
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
        backspace_ctrl_h: match terminal.backspace {
            BackspaceChoice::Auto => None,
            BackspaceChoice::Del => Some(false),
            BackspaceChoice::CtrlH => Some(true),
        },
        bracketed_paste: terminal.bracketed_paste,
        paste_wait: Some(terminal.paste_wait),
        clipboard: terminal.clipboard,
    }
}

/// Starts the log: the file alone, since standard error is the terminal.
/// Its level is `--log-level`, then `CINERIS_LOG`, then `log.level`.
pub(crate) fn start_log(
    run: &Run,
    env: &Env,
    loaded: &Loaded,
    secrets: &Arc<SecretFilter>,
) -> Option<Arc<LogHandle>> {
    let clock = Arc::new(SystemClock::new());
    let mut options = LogOptions::new(clock);
    let level = run
        .log_level
        .clone()
        .or_else(|| env.get("CINERIS_LOG").map(str::to_owned))
        .or_else(|| loaded.settings.log.level.clone());
    if let Some(level) = level
        && let Ok(filter) = Filter::parse(&level)
    {
        options.filter = filter;
    }
    options.file = StateDir::from_dirs(&Dirs::from_env(env))
        .map(|state| FileOptions::in_dir(state.logs_dir()));
    options.stderr = None;
    options.secrets = Arc::clone(secrets);
    let (subscriber, handle) = log::start(options).ok()?;
    tracing::subscriber::set_global_default(subscriber).ok()?;
    Some(Arc::new(handle))
}

// ------------------------------------------------------------------ panics

/// The thread the interface runs on: a panic there puts the terminal back
/// (the terminal layer's hook) and ends the program.
static INTERFACE: OnceLock<std::thread::ThreadId> = OnceLock::new();
/// Where the report of the interface's panic was written.
pub(crate) static CRASH: OnceLock<PathBuf> = OnceLock::new();

/// The panic hook of interactive mode (`docs/architecture/02-runtime.md`,
/// "Panics"): a report with a backtrace in the log directory, and the
/// panic in the log. Nothing is written to the terminal from here: the
/// interface's panic has put it back by the time this runs (the terminal
/// layer's hook runs first, then this one), and its one-line pointer is
/// printed when the program ends; any other thread's panic, which the core
/// may have caught and gone on from, leaves the terminal the interface's,
/// so the default hook's message would land in the middle of a frame.
pub(crate) fn install_panic_hook(logs: Option<PathBuf>, secrets: Arc<SecretFilter>) {
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
        let written = cineris_base::private::create_dir_all(logs)
            .and_then(|()| cineris_base::private::create_new(&path))
            .and_then(|mut file| {
                write!(file, "{}", crash_report(&name, info, &backtrace, &secrets))
            });
        if written.is_ok() {
            let _ = CRASH.set(path);
        }
    }));
}

/// What the crash report says: the panic's message through the secret
/// filter the log uses, since a panic can quote the text it was handling (a
/// string slice's panic quotes up to 256 bytes of it, which may be a prompt
/// or a key), and the backtrace (the review of step 23).
fn crash_report(
    thread: &str,
    info: &dyn std::fmt::Display,
    backtrace: &dyn std::fmt::Display,
    secrets: &SecretFilter,
) -> String {
    format!(
        "cineris {} panicked on its {thread} thread:\n{}\n\n{backtrace}\n",
        crate::VERSION,
        secrets.redact(&info.to_string())
    )
}

// --------------------------------------------------------------- the core

/// Where the link's waker goes once the terminal has one: what arrives
/// before then is drained on the interface's first wake.
#[derive(Clone, Default)]
pub(crate) struct Slot(Arc<Mutex<Option<Waker>>>);

impl Slot {
    pub(crate) fn set(&self, waker: Waker) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    pub(crate) fn wake(&self) {
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
pub(crate) type Done = Arc<Mutex<VecDeque<Inbound>>>;

pub(crate) fn push(done: &Done, inbound: Inbound) {
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
            .name("cineris-core".to_owned())
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
                    // its own, one at a time in the order sent, and reads
                    // beside it (`cineris_rpc::lane`, which an RPC
                    // connection runs too), so that the end below is never
                    // behind it.
                    let mut dispatcher = {
                        let core = core.clone();
                        let id = client.id;
                        let run: cineris_rpc::lane::Run = Arc::new(move |op| {
                            let core = core.clone();
                            Box::pin(async move { core.handle(id, op).await })
                        });
                        let (done, wake) = (Arc::clone(&thread_done), thread_slot.clone());
                        Dispatcher::start(
                            run,
                            Arc::new(move |ticket: Ticket, result| {
                                push(&done, Inbound::Done(ticket, result));
                                wake.wake();
                            }),
                        )
                    };
                    while let Some((ticket, op)) = lane.recv().await {
                        dispatcher.send(ticket, op);
                    }
                    // The interface is gone, outside the lane: what it was
                    // asked and never answered is withdrawn, which ends an
                    // operation waiting on the answer (the trust question of
                    // an open, the undo's question), and the sessions it
                    // had open are closed.
                    let lane_task = dispatcher.close();
                    core.disconnect(client.id).await;
                    // Then the lane's last operation, for so long, and
                    // everything else in order.
                    if let Some(lane_task) = lane_task {
                        let _ = tokio::time::timeout(LANE_END, lane_task).await;
                    }
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
            order: Order::default(),
            gone_said: false,
        }
    }

    /// Ends the core: no more operations, then its shutdown, waited for at
    /// most `within`, and at most [`SIGNAL_END`] after a signal that comes
    /// meanwhile, which the watchdog gives 3 s from its coming (the review
    /// of step 23).
    fn end(mut self, within: Duration) {
        self.ops = None;
        let began = Instant::now();
        let mut signalled = None;
        loop {
            if signalled.is_none() && app::signalled().is_some() {
                signalled = Some(Instant::now());
            }
            let deadline = end_deadline(began, within, signalled);
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            // Looked at again every so often for a signal: the wait itself
            // cannot be woken by one.
            match self.ended.recv_timeout(left.min(END_LOOK)) {
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
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
    /// The events and requests still to be given before the result that is
    /// ready: see [`take`].
    order: Order,
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
        take(
            &self.done,
            &self.queue,
            &mut self.order,
            &mut self.gone_said,
            core_closed,
        )
    }
}

/// What the link gives next, in [`Order`]: a result that is ready only
/// after the events and requests that were queued when it was first seen
/// (the review of step 23's fixes), so that a history read's result never
/// overtakes the events it holds; and once the queue is closed and empty,
/// the end of the connection, said once.
pub(crate) fn take(
    done: &Done,
    queue: &EventQueue,
    order: &mut Order,
    gone_said: &mut bool,
    gone: fn() -> ErrorData,
) -> Option<Inbound> {
    match order.next(queue, done) {
        Some(Next::Result(result)) => Some(result),
        Some(Next::Core(next)) => inbound(Some(next), false, gone_said, gone),
        None => inbound(None, queue.is_closed(), gone_said, gone),
    }
}

/// What the in-process interface is told when the core closes its
/// connection.
fn core_closed() -> ErrorData {
    ErrorData::new(
        ErrorKind::Internal,
        "client_closed",
        "The core closed this interface's connection.",
    )
}

/// What the link gives for what the client's queue gave: an event, a
/// request, or, once the queue is closed and empty, the end of the
/// connection, said once.
fn inbound(
    next: Option<Outgoing>,
    closed: bool,
    gone_said: &mut bool,
    gone: fn() -> ErrorData,
) -> Option<Inbound> {
    match next {
        Some(Outgoing::Event(event)) => Some(Inbound::Event(event)),
        Some(Outgoing::Request { id, request }) => Some(Inbound::Request(id, request)),
        // The core disconnected this client: nothing more will come.
        None if closed && !*gone_said => {
            *gone_said = true;
            Some(Inbound::Gone(gone()))
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core is given its whole time after a quit, and at most
    /// `SIGNAL_END` from a signal: the one the loop ended for, or one that
    /// came while the core ended, whose watchdog gives 3 s from its coming
    /// (the review of step 23).
    #[test]
    fn the_cores_end_is_cut_short_by_a_signal() {
        let began = Instant::now();
        assert_eq!(end_deadline(began, CORE_END, None), began + CORE_END);
        let later = began + Duration::from_secs(1);
        assert_eq!(
            end_deadline(began, CORE_END, Some(later)),
            later + SIGNAL_END
        );
        // A signal near the end of the wait never lengthens it.
        let late = began + Duration::from_millis(4900);
        assert_eq!(end_deadline(began, CORE_END, Some(late)), began + CORE_END);
        assert!(
            SIGNAL_END + LOG_END < Duration::from_millis(2000),
            "the history's 1 s, the core's and the log's wait must fit the watchdog's 3 s"
        );
    }

    /// A panic of the interface is exit code 1, with the report's place;
    /// the core's end is not skipped for it (`run` ends the core first).
    #[test]
    fn a_panic_of_the_interface_is_exit_1_and_points_at_its_report() {
        let panicked: std::thread::Result<io::Result<Exit>> = Err(Box::new("boom"));
        let report = Path::new("logs").join("crash-1.txt");
        let (code, said) = ending(panicked, Some(&report), "cineris", || true);
        assert_eq!(code, exit::INTERNAL);
        let said = said.unwrap_or_default();
        assert!(said.contains("the interface failed"), "{said}");
        assert!(said.contains("crash-1.txt"), "{said}");
        let (code, said) = ending(Err(Box::new("boom")), None, "cineris", || true);
        assert_eq!(code, exit::INTERNAL);
        assert!(said.unwrap_or_default().contains("the log file"));
        assert_eq!(
            ending(Ok(Ok(Exit::Quit)), None, "cineris", || true),
            (exit::COMPLETED, None)
        );
        let (code, _) = ending(
            Ok(Err(io::Error::other("no terminal"))),
            None,
            "cineris",
            || false,
        );
        assert_eq!(code, exit::USAGE);
    }

    /// The crash report masks what the log masks: a panic that quotes a
    /// key does not put it in a file whose place is printed to be shared
    /// (the review of step 23).
    #[test]
    fn the_crash_report_masks_a_key_the_panic_quotes() {
        let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        let report = crash_report(
            "main",
            &format!("byte index 9 is not a char boundary; it is inside 'x' of `{key}`"),
            &"the backtrace",
            &SecretFilter::new(),
        );
        assert!(!report.contains(key), "{report}");
        assert!(report.contains("not a char boundary"), "{report}");
        assert!(report.contains("the backtrace"), "{report}");
    }

    fn event(seq: u64, event: cineris_protocol::event::Event) -> cineris_protocol::SessionEvent {
        cineris_protocol::SessionEvent {
            session_id: SessionId::from("s_a"),
            seq,
            turn_id: None,
            event,
        }
    }

    fn seq_of(inbound: Option<Inbound>) -> Option<u64> {
        match inbound {
            Some(Inbound::Event(event)) => Some(event.seq),
            Some(Inbound::Done(Ticket(ticket), _)) => Some(1000 + ticket),
            _ => None,
        }
    }

    fn read_result() -> Inbound {
        Inbound::Done(
            Ticket(7),
            Ok(cineris_protocol::methods::OpResult::SessionRead(
                cineris_protocol::methods::SessionReadResult {
                    items: Vec::new(),
                    incomplete: Vec::new(),
                    next_cursor: None,
                    seq: 9,
                    masked: 0,
                    redacted: false,
                },
            )),
        )
    }

    /// A turn begun after the open, at a call when the read is taken: its
    /// `turn.started` and items were queued before the read's result, and
    /// come before it, so the application holds them while it reads and
    /// takes the turn as running (the review of step 23's fixes found the
    /// result given first: the call cut off, and its result in a second
    /// box).
    #[test]
    fn a_result_comes_after_the_events_queued_ahead_of_it() {
        use cineris_protocol::event::{Event, TurnStarted};
        let queue = EventQueue::new(64, true);
        let done: Done = Arc::new(Mutex::new(VecDeque::new()));
        let (mut ahead, mut said) = (Order::default(), false);
        queue.push(event(6, Event::TurnStarted(TurnStarted {})));
        queue.push(event(7, Event::TurnStarted(TurnStarted {})));
        queue.push(event(9, Event::TurnStarted(TurnStarted {})));
        push(&done, read_result());
        let mut order = Vec::new();
        while let Some(seq) = seq_of(take(&done, &queue, &mut ahead, &mut said, core_closed)) {
            order.push(seq);
        }
        assert_eq!(order, [6, 7, 9, 1007]);
    }

    /// Only what was queued when the result was first seen comes before
    /// it: events that keep coming do not hold it back.
    #[test]
    fn events_that_keep_coming_do_not_hold_a_result_back() {
        use cineris_protocol::event::{Event, TurnStarted};
        let queue = EventQueue::new(64, true);
        let done: Done = Arc::new(Mutex::new(VecDeque::new()));
        let (mut ahead, mut said) = (Order::default(), false);
        queue.push(event(1, Event::TurnStarted(TurnStarted {})));
        queue.push(event(2, Event::TurnStarted(TurnStarted {})));
        push(&done, read_result());
        let mut order = Vec::new();
        for seq in 3..8 {
            order.push(seq_of(take(
                &done,
                &queue,
                &mut ahead,
                &mut said,
                core_closed,
            )));
            queue.push(event(seq, Event::TurnStarted(TurnStarted {})));
        }
        assert_eq!(
            order,
            [Some(1), Some(2), Some(1007), Some(3), Some(4)],
            "the result after the two queued before it"
        );
    }

    /// A queue the core closed is the end of the connection, said once,
    /// after what was in it.
    #[test]
    fn a_closed_queue_is_the_end_of_the_connection_said_once() {
        let mut said = false;
        assert!(inbound(None, false, &mut said, core_closed).is_none());
        assert!(matches!(
            inbound(None, true, &mut said, core_closed),
            Some(Inbound::Gone(error)) if error.code == "client_closed"
        ));
        assert!(
            inbound(None, true, &mut said, core_closed).is_none(),
            "said once"
        );
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
                &[cineris_tui::keys::Context::Dialog],
                cineris_tui::keys::parse("o").unwrap()
            ),
            Some(cineris_tui::keys::Action::AllowOnce)
        );
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("editor.no_such_action"), "{notices:?}");
    }

    /// Every table `[keys]` declares is read into the key map: the list of
    /// tables holds each field of the section, and a binding in each of
    /// the full screen's and the viewer's is taken (the review of step 25).
    #[test]
    fn every_table_of_keys_is_read() {
        let mut declared: Vec<String> = cineris_config::schema::keys()
            .into_iter()
            .filter_map(|key| key.path.strip_prefix("keys.").map(str::to_owned))
            .collect();
        declared.sort();
        let mut read: Vec<String> = key_tables(&KeySettings::default())
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect();
        read.sort();
        assert_eq!(read, declared);

        let mut keys = KeySettings::default();
        keys.screen
            .0
            .insert("search".to_owned(), vec!["ctrl+s".to_owned()]);
        keys.viewer.0.insert("top".to_owned(), vec!["t".to_owned()]);
        let (map, notices) = keymap(&keys);
        assert!(notices.is_empty(), "{notices:?}");
        let lookup = |context, key| map.lookup(&[context], cineris_tui::keys::parse(key).unwrap());
        assert_eq!(
            lookup(cineris_tui::keys::Context::Screen, "ctrl+s"),
            Some(cineris_tui::keys::Action::ScreenSearch)
        );
        assert_eq!(
            lookup(cineris_tui::keys::Context::Viewer, "t"),
            Some(cineris_tui::keys::Action::ViewerTop)
        );
    }

    /// The quirks' settings of `[terminal]` reach the terminal layer's
    /// overrides: `backspace` as whether byte 0x08 is Backspace (`auto`
    /// left to the terminal's erase character), and `bracketed_paste`,
    /// `paste_wait` and `clipboard` as they are (step 25; the review of its
    /// second pull request).
    #[test]
    fn the_terminals_quirks_reach_the_overrides() {
        let defaults = overrides(&TerminalSettings::default());
        assert_eq!(defaults.backspace_ctrl_h, None);
        assert_eq!(defaults.bracketed_paste, None);
        assert_eq!(defaults.paste_wait, Some(Duration::from_secs(2)));
        assert_eq!(defaults.clipboard, None);
        for (backspace, ctrl_h, told) in [
            (BackspaceChoice::Del, Some(false), false),
            (BackspaceChoice::CtrlH, Some(true), true),
        ] {
            let settings = TerminalSettings {
                backspace,
                bracketed_paste: Some(told),
                paste_wait: Duration::from_millis(500),
                clipboard: Some(told),
                ..TerminalSettings::default()
            };
            let overrides = overrides(&settings);
            assert_eq!(overrides.backspace_ctrl_h, ctrl_h);
            assert_eq!(overrides.bracketed_paste, Some(told));
            assert_eq!(overrides.paste_wait, Some(Duration::from_millis(500)));
            assert_eq!(overrides.clipboard, Some(told));
        }
    }

    /// The interface's settings take the mouse from `[terminal]`, on unless
    /// set off, and the presenter from `[tui]`: both interfaces build theirs
    /// with `app_settings` (step 25).
    #[test]
    fn the_interfaces_settings_take_the_mouse_and_the_presenter() {
        let mut settings = cineris_config::schema::Settings::default();
        let mut notices = Vec::new();
        assert!(app_settings(&settings, &mut notices).mouse);
        settings.terminal.mouse = Some(false);
        settings.tui.presenter = PresenterChoice::Fullscreen;
        let app = app_settings(&settings, &mut notices);
        assert!(!app.mouse);
        assert_eq!(app.presenter, app::Presenter::Fullscreen);
        assert!(notices.is_empty(), "{notices:?}");
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

    /// The look and the key map are built again from `config/get`'s answer
    /// as at the start, a theme not known said; a section that does not
    /// read is an error, which leaves the application's as they were (step
    /// 24's hot reload).
    #[test]
    fn the_look_and_the_keys_are_read_again_from_the_cores_answer() {
        let mut answer = serde_json::Map::new();
        answer.insert("tui.theme".to_owned(), "light".into());
        answer.insert("tui.density".to_owned(), "compact".into());
        answer.insert(
            "keys.editor".to_owned(),
            serde_json::json!({"submit": ["ctrl+j"], "nothing": ["f1"]}),
        );
        let restyled = restyle(&answer).unwrap();
        assert_eq!(restyled.settings.theme, app::ThemeChoice::Light);
        assert_eq!(restyled.settings.density, Density::Compact);
        let submit = restyled.keymap.keys_of(cineris_tui::keys::Action::Submit);
        assert!(
            submit.contains(&cineris_tui::keys::parse("ctrl+j").unwrap()),
            "{submit:?}"
        );
        assert_eq!(restyled.key_notices.len(), 1, "{:?}", restyled.key_notices);

        answer.insert("tui.theme".to_owned(), "dusk".into());
        let restyled = restyle(&answer).unwrap();
        assert_eq!(restyled.settings.theme, app::ThemeChoice::Auto);
        assert!(
            restyled
                .look_notices
                .iter()
                .any(|notice| notice.contains("dusk")),
            "{:?}",
            restyled.look_notices
        );

        answer.insert("tui.prose_width".to_owned(), "wide".into());
        let error = restyle(&answer).unwrap_err();
        assert!(error.contains("stay as they were"), "{error}");
    }
}
