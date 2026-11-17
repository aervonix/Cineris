//! The application's end on a real terminal (a pseudo-terminal), Unix
//! only: `app::interactive` run by this test binary run again as a child,
//! with a link that answers nothing (the review of step 23).
//!
//! - **The terminal closing** (a window, a tab, an SSH connection) reaches
//!   the reader as the end of input or EIO before, or instead of, SIGHUP:
//!   it ends the application as SIGHUP does, and the SIGHUP that follows is
//!   that same end, not a second signal that kills the process part-way.
//! - **A signal while a key is taken** (between the loop's look for a
//!   signal and the frame the key draws) ends it as that signal, not with
//!   the failed write's error.
//! - **Quitting from the viewer** leaves the main screen as a plain quit
//!   does, the live region erased.
//!
//! What the application returned is written to a report file, since the
//! terminal may be gone by then.

#![cfg(unix)]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::methods::{Op, ServerResponse};
use cineris_protocol::session::SessionSettings;
use cineris_testkit::pty::Session;
use cineris_testkit::role::{Role, this_binary};
use cineris_tui::app::{self, Inbound, Link, Open, Overrides, Settings, Start, Ticket};
use cineris_tui::highlight::Highlighter;
use cineris_tui::history::{HistoryStore, MemoryHistory};
use cineris_tui::keys::KeyMap;

/// This binary as the application (`app`, `key`, `viewer`, `plain`, or
/// `job`: run by a shell), or as a shell that runs it as its job
/// (`leader`).
const SIGNALS: Role = Role::new("cxtui-signals=");
/// A bound on every wait, which only a broken test reaches.
const LIMIT: Duration = Duration::from_secs(30);
/// A notice the application shows in its first frame.
const NOTICE: &str = "SIGNALS-NOTICE";
/// The variable that names the report file.
const REPORT: &str = "CXTUI_SIGNALS_REPORT";

fn append(path: &Path, line: &str) {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

fn report() -> Option<PathBuf> {
    std::env::var_os(REPORT).map(PathBuf::from)
}

/// A link that answers nothing; its first send (the open, after the first
/// frame) is said in the report.
struct Quiet {
    sent: u64,
}

impl Link for Quiet {
    fn send(&mut self, _op: Op) -> Ticket {
        self.sent += 1;
        if self.sent == 1
            && let Some(report) = report()
        {
            append(&report, "drawn");
        }
        Ticket(self.sent)
    }
    fn respond(&mut self, _: RequestId, _: ServerResponse) {}
    fn next(&mut self) -> Option<Inbound> {
        None
    }
}

/// A history whose push stands for a signal that comes while the loop
/// takes `Enter`: SIGINT raised on the interface's own thread, its handler
/// run before this returns.
#[derive(Debug)]
struct RaisesOnPush;

impl HistoryStore for RaisesOnPush {
    fn len(&self) -> usize {
        0
    }
    fn get(&self, _: usize) -> Option<&cineris_tui::editor::Fragment> {
        None
    }
    fn push(&mut self, _: &cineris_tui::editor::Fragment) {
        cineris_term::pty::raise_interrupt().expect("raise SIGINT");
    }
}

/// Plays the part this process was started for, if it was.
pub(crate) fn enter() {
    let Some(case) = SIGNALS.options() else {
        return;
    };
    if case == "leader" {
        // A shell's part: the session's leader, with the application as
        // its job in the foreground. When the terminal hangs up the leader
        // dies of SIGHUP, and the kernel then sends SIGHUP to the job, as a
        // shell resends it to its jobs: after the hang-up.
        let status = SIGNALS.command("job").status();
        if let Some(report) = report() {
            append(&report, &format!("leader saw the app end: {status:?}"));
        }
        #[expect(clippy::exit, reason = "a child ends when its part is played")]
        std::process::exit(0);
    }
    let parent = std::os::unix::process::parent_id();
    // The application's process, which the test describes should the report
    // stop short: a job's is not the test's own child.
    if let Some(report) = report() {
        append(&report, &format!("pid {}", std::process::id()));
    }
    // Should the end stall, the report says where each thread waits in the
    // kernel, every 5 s: a test's run out of time then says why.
    std::thread::spawn(|| {
        loop {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(5) {
                std::thread::park_timeout(Duration::from_secs(5));
            }
            let mut said = String::from("still running:");
            for task in std::fs::read_dir("/proc/self/task")
                .into_iter()
                .flatten()
                .flatten()
            {
                let wchan = std::fs::read_to_string(task.path().join("wchan")).unwrap_or_default();
                let stat = std::fs::read_to_string(task.path().join("stat")).unwrap_or_default();
                let state = stat
                    .rsplit(')')
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .next()
                    .unwrap_or("?")
                    .to_owned();
                said.push_str(&format!(
                    " [{} {state} {wchan}]",
                    task.file_name().to_string_lossy()
                ));
            }
            if let Some(report) = report() {
                append(&report, &said);
            }
        }
    });
    let history: Box<dyn HistoryStore + Send> = if case == "key" {
        Box::new(RaisesOnPush)
    } else {
        Box::new(MemoryHistory::new())
    };
    let result = app::interactive(Overrides::default(), move |_waker, _caps| {
        let start = Start {
            version: "0.0.0".to_owned(),
            folder: "~/signals".to_owned(),
            cwd: None,
            workspace: "/nonexistent-signals".to_owned(),
            open: Open::New {
                workspace: "/nonexistent-signals".to_owned(),
                settings: SessionSettings::default(),
                ephemeral: true,
            },
            prompt: None,
            settings: Settings::default(),
            keymap: KeyMap::defaults(),
            notices: vec![NOTICE.to_owned()],
            history,
            highlighter: Highlighter::off(),
            restyle: None,
            export: None,
        };
        (Box::new(Quiet { sent: 0 }) as Box<dyn Link>, start)
    });
    let said = format!("RESULT {result:?}");
    if let Some(report) = report() {
        append(&report, &said);
    }
    let _ = std::io::Write::write_all(&mut std::io::stdout(), format!("\r\n{said}\r\n").as_bytes());
    let _ = std::io::Write::flush(&mut std::io::stdout());
    if case == "job" {
        // Run as a job: the SIGHUP the leader's end sends it comes as the
        // leader exits. Once this process is no longer the leader's child
        // the signal has been sent, and is taken on the next return from
        // the kernel: a process still here after that survived it.
        let deadline = Instant::now() + LIMIT;
        while std::os::unix::process::parent_id() == parent && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let _ = std::process::id();
        if let Some(report) = report() {
            append(&report, "ALIVE after the leader's end");
        }
    }
    app::signal_ended();
    #[expect(clippy::exit, reason = "a child ends when its part is played")]
    std::process::exit(0);
}

/// Everything the screen and its scrollback show.
fn shown(session: &Session) -> String {
    session.screen().history().join("\n")
}

fn start(case: &str) -> Session {
    let mut session = Session::start(&this_binary(), &SIGNALS.arguments(case), 80, 24).answering();
    session.wait_for(LIMIT, "the first frame", |t| t.shows(NOTICE));
    session
}

/// A signal that comes while the loop takes a key (here `Enter`, its
/// handler run on the loop's own thread) ends the application as that
/// signal, and the restore erases the whole live region: the input box's
/// top border, above the cursor, was left before.
#[test]
fn sys_a_signal_while_a_key_is_taken_ends_as_that_signal() {
    let mut session = start("key");
    session.send(b"hi\r");
    session.wait_exit(LIMIT);
    // What the child wrote last can still be on its way once it has ended.
    session.wait_for(LIMIT, "the result", |t| t.shows("RESULT"));
    let screen = shown(&session);
    assert!(
        screen.contains("RESULT Ok(Signal(130))"),
        "the loop did not end as the signal asked:\n{screen}"
    );
    assert!(
        !screen.contains('╭') && !screen.contains('╰'),
        "the restore left the input box:\n{screen}"
    );
}

/// Quitting (`Ctrl+C` twice, empty prompt, no turn) while the viewer is
/// open leaves the main screen as quitting from the inline screen does.
#[test]
fn sys_quitting_from_the_viewer_leaves_the_main_screen_clean() {
    let mut plain = start("plain");
    plain.send(b"\x03\x03");
    plain.wait_exit(LIMIT);
    // The process's last output can still be on its way when it has ended.
    plain.wait_for(LIMIT, "the result", |t| t.shows("RESULT"));
    let plain_screen = shown(&plain);

    let mut viewer = start("viewer");
    viewer.send(b"\x0f");
    viewer.wait_for(LIMIT, "the viewer", |t| t.modes().alternate);
    viewer.send(b"\x03\x03");
    viewer.wait_exit(LIMIT);
    viewer.wait_for(LIMIT, "the result", |t| t.shows("RESULT"));
    let viewer_screen = shown(&viewer);
    assert!(
        !viewer.screen().modes().alternate,
        "left on the alternate screen"
    );
    assert!(
        viewer_screen.contains("RESULT Ok(Quit)"),
        "did not quit:\n{viewer_screen}"
    );
    assert_eq!(
        viewer_screen.trim_end(),
        plain_screen.trim_end(),
        "the main screen after quitting from the viewer differs from after a plain quit"
    );
}

/// The report once it has the application's result and, for a job, the
/// line that says it outlived its leader's end.
fn wait_result(report: &Path, job: bool) -> String {
    let deadline = Instant::now() + LIMIT;
    loop {
        let text = std::fs::read_to_string(report).unwrap_or_default();
        let done = text.contains("RESULT") && (!job || text.contains("ALIVE"));
        if done || Instant::now() >= deadline {
            return text;
        }
        std::thread::yield_now();
    }
}

/// Starts `case` in a pseudo-terminal of its own, waits for its first
/// frame, then closes the terminal: every handle on its other end goes.
/// Returns what the report says.
fn close_the_terminal(case: &str, test: &str, job: bool) -> String {
    let dir = std::env::temp_dir().join(format!("cxtui-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("report.txt");
    // The job outlives its leader: once it ends it stays as this process's
    // zombie, whose end `/proc` can still say.
    cineris_term::pty::adopt_orphans().unwrap();
    let pty = cineris_term::pty::Pty::spawn(
        &this_binary(),
        &SIGNALS.arguments(case),
        &[(REPORT.to_owned(), report.display().to_string())],
        80,
        24,
    )
    .unwrap();
    // Reads until the first frame is there, then lets go of this end; the
    // queries the terminal is asked go unanswered.
    let mut reader = pty.reader().unwrap();
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut seen = Vec::new();
        let mut buffer = [0u8; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            seen.extend_from_slice(&buffer[..count]);
            if String::from_utf8_lossy(&seen).contains(NOTICE) {
                break;
            }
        }
        let _ = done.send(());
    });
    finished.recv_timeout(LIMIT).expect("the first frame");
    let deadline = Instant::now() + LIMIT;
    while !std::fs::read_to_string(&report).is_ok_and(|text| text.contains("drawn")) {
        assert!(Instant::now() < deadline, "no first frame");
        std::thread::yield_now();
    }
    drop(pty);
    let mut text = wait_result(&report, job);
    if !(text.contains("RESULT") && (!job || text.contains("ALIVE"))) {
        // Where the application is, or how it ended: a signal's default
        // action (SIGALRM is the watchdog), a panic, or still running. A
        // job that wrote its result and was then ended part-way is
        // described too.
        let pid = text
            .lines()
            .find_map(|line| line.strip_prefix("pid "))
            .and_then(|pid| pid.trim().parse::<u32>().ok());
        text.push_str(&pid.map_or_else(|| "no pid said".to_owned(), describe));
    }
    let _ = std::fs::remove_dir_all(&dir);
    text
}

/// What `/proc` says of process `pid`: how it ended, if it is a zombie, or
/// else each thread's state and where it waits, and its signals.
fn describe(pid: u32) -> String {
    let base = format!("/proc/{pid}");
    let Ok(stat) = std::fs::read_to_string(format!("{base}/stat")) else {
        return format!("process {pid}: gone, waited for by another");
    };
    let fields: Vec<&str> = stat
        .rsplit(')')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    let state = fields.first().copied().unwrap_or("?");
    if state == "Z" {
        // Field 52, the status `waitpid` would give.
        let code: i32 = fields
            .get(49)
            .and_then(|code| code.parse().ok())
            .unwrap_or(-1);
        return if code & 0x7f == 0 {
            format!("process {pid}: exited with {}", (code >> 8) & 0xff)
        } else {
            format!("process {pid}: killed by signal {}", code & 0x7f)
        };
    }
    let mut said = format!("process {pid}: {state}");
    for task in std::fs::read_dir(format!("{base}/task"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let wchan = std::fs::read_to_string(task.path().join("wchan")).unwrap_or_default();
        let stat = std::fs::read_to_string(task.path().join("stat")).unwrap_or_default();
        let state = stat
            .rsplit(')')
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap_or("?")
            .to_owned();
        said.push_str(&format!(
            " [{} {state} {wchan}]",
            task.file_name().to_string_lossy()
        ));
    }
    let status = std::fs::read_to_string(format!("{base}/status")).unwrap_or_default();
    for line in status.lines().filter(|line| {
        ["SigPnd", "ShdPnd", "SigBlk", "SigIgn", "SigCgt"]
            .iter()
            .any(|name| line.starts_with(name))
    }) {
        said.push_str(&format!("\n{line}"));
    }
    said
}

/// The terminal closing (a hang-up, then SIGHUP to the job, as a shell
/// resends it) ends the application as SIGHUP, and the SIGHUP after it
/// does not end the process part-way through its end.
#[test]
fn sys_closing_the_terminal_ends_as_sighup() {
    let text = close_the_terminal("leader", "hup-job", true);
    assert!(
        text.contains("RESULT Ok(Signal(129))"),
        "closing the terminal did not end the application as SIGHUP:\n{text}"
    );
    assert!(
        text.contains("ALIVE after the leader's end"),
        "the SIGHUP after the hang-up ended the process part-way:\n{text}"
    );
}

/// The same with the application as the session's leader (a terminal
/// emulator or `ssh host cineris` running it directly).
#[test]
fn sys_closing_the_terminal_of_a_leader_ends_as_sighup() {
    let text = close_the_terminal("app", "hup-leader", false);
    assert!(
        text.contains("RESULT Ok(Signal(129))"),
        "closing the terminal of a leader did not end it as SIGHUP:\n{text}"
    );
}
