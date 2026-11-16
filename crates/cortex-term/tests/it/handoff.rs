//! Signals handed to the program (`Terminal::hand_signals_over`, step 23),
//! on Unix: the restore erases the whole live region, the watchdog is set
//! when the first signal is handed over, a second signal ends the process
//! at once but a second SIGHUP is the same end, and the terminal going away
//! under a read is SIGHUP's end (the review of step 23), even when its
//! SIGHUP is being handled on another thread as the read finds the end.
//!
//! Each case runs in a child of its own in a pseudo-terminal, this test
//! binary run again: a signal's action is the process's.

#[cfg(unix)]
use std::time::Duration;

use cortex_testkit::role::Role;

/// This binary as a child that plays one case: `watchdog`, `second`,
/// `sighup-twice`, `eof` or `race`.
const HANDOFF: Role = Role::new("cxterm-handoff=");

/// The live region's rows the child draws, the cursor left on the last.
#[cfg(unix)]
const ROWS: [&str; 4] = [
    "region row one",
    "region row two",
    "region row three",
    "the last row",
];

/// What the child says once a signal was handed to it.
#[cfg(unix)]
const HANDED: &str = "handed over";

/// The variable that names the file a child with no terminal left says
/// what happened in.
#[cfg(unix)]
const REPORT: &str = "CXTERM_HANDOFF_REPORT";

/// Says `line` in the report file the test named.
#[cfg(unix)]
fn report(line: &str) {
    use std::io::Write as _;
    let path = std::env::var_os(REPORT).expect("the report's path");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("the report");
    writeln!(file, "{line}").expect("write the report");
}

/// Plays the case this process was started for, if it was.
pub(crate) fn enter() {
    let Some(case) = HANDOFF.options() else {
        return;
    };
    #[cfg(target_os = "linux")]
    if case == "race" {
        race::play();
    }
    #[cfg(unix)]
    play(&case);
    #[cfg(not(unix))]
    let _ = case;
    #[expect(clippy::exit, reason = "a child ends when its case is played")]
    std::process::exit(0);
}

#[cfg(unix)]
fn play(case: &str) {
    use cortex_term::{Frame, Inline, Line, Overrides, Terminal};
    use cortex_testkit::role::write_terminal;

    let mut terminal = Terminal::enter(Overrides::default()).expect("enter the terminal");
    terminal.hand_signals_over();
    let (cols, rows) = terminal.size().expect("the size");
    let mut presenter = Inline::new(cols, rows);
    let live: Vec<Line> = ROWS.iter().map(|row| Line::plain(*row)).collect();
    let caps = *terminal.caps();
    presenter
        .draw(
            &Frame {
                commit: &[],
                live: &live,
                cursor: Some(cortex_term::Cursor {
                    row: ROWS.len() - 1,
                    col: 0,
                }),
            },
            &caps,
            &mut terminal,
        )
        .expect("draw");
    // Waits for the signal, as the application's loop does.
    let code = loop {
        let read = terminal.next(None);
        // A signal handed over: what came after it (a reply to a query,
        // which a write then answers) may have failed, the terminal put back
        // by the handler, on another thread, a moment before it hands over.
        if read.is_err() {
            let deadline = std::time::Instant::now() + LIMIT;
            while terminal.ending().is_none() && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
        }
        if let Some(code) = terminal.ending() {
            break code;
        }
        if let Err(error) = read {
            // The terminal went away, and that was not taken as SIGHUP's end.
            let said = format!("a read failed: {error}");
            if case == "eof" {
                report(&said);
            } else {
                write_terminal(&format!("{said}\r\n"));
            }
            #[expect(clippy::exit, reason = "the child ends when it has said so")]
            std::process::exit(0);
        }
    };
    let left = cortex_term::pty::watchdog_left();
    if case == "eof" {
        // The terminal is gone: the report says it.
        report(&format!("{HANDED} {code}, the watchdog at {left} s"));
        drop(terminal);
        cortex_term::signal_ended();
        return;
    }
    // On a row of its own: replies to the queries that come once the
    // terminal is put back are echoed, and the text after them on their row
    // would wrap, its words split across two rows.
    write_terminal(&format!("\r\n{HANDED} {code}, the watchdog at {left} s\r\n"));
    match case {
        // A second SIGHUP, as the kernel and a shell send one after the
        // other when a terminal closes: the same end, which goes on.
        "sighup-twice" => {
            cortex_term::pty::raise_hangup().expect("raise SIGHUP");
            write_terminal("survived the second SIGHUP\r\n");
            drop(terminal);
            cortex_term::signal_ended();
        }
        // Ends itself in time: the watchdog is put off for good.
        "watchdog" => {
            drop(terminal);
            cortex_term::signal_ended();
        }
        // Never ends itself: a second signal must end it, at once.
        "second" => loop {
            std::thread::park();
        },
        other => panic!("no case `{other}`"),
    }
}

#[cfg(unix)]
fn start(case: &str) -> cortex_testkit::pty::Session {
    let mut session = cortex_testkit::pty::Session::start(
        &super::this_binary(),
        &HANDOFF.arguments(case),
        80,
        24,
    )
    .answering();
    session.wait_for(LIMIT, "the live region", |t| t.shows(ROWS[3]));
    session
}

#[cfg(unix)]
const LIMIT: Duration = Duration::from_secs(30);

#[cfg(unix)]
fn kill(session: &cortex_testkit::pty::Session, signal: &str) {
    let sent = std::process::Command::new("kill")
        .args([format!("-{signal}"), session.pid().to_string()])
        .status()
        .expect("kill");
    assert!(sent.success(), "SIG{signal} was not sent");
}

/// The first signal is handed over with the watchdog set (3 s), and the
/// restore erases every row of the live region, not only those from the
/// cursor's row down: the rows above it were left on the screen before.
#[cfg(unix)]
#[test]
fn sys_a_signal_handed_over_sets_the_watchdog_and_erases_the_whole_region() {
    let mut session = start("watchdog");
    kill(&session, "TERM");
    // What the child wrote last can still be on its way once it has ended.
    session.wait_for(LIMIT, "the hand-over", |t| t.shows(HANDED));
    let status = session.wait_exit(LIMIT);
    let screen = session.screen();
    let said = screen
        .history()
        .into_iter()
        .find(|row| row.contains(HANDED))
        .unwrap_or_else(|| panic!("the child never said: {status}\n{}", screen.dump()));
    assert!(
        // Set at 3 s by the hand-over; a loaded machine may take a second
        // or two to ask.
        ["3", "2", "1"]
            .iter()
            .any(|left| said.contains(&format!("handed over 143, the watchdog at {left} s"))),
        "{said}"
    );
    for row in ROWS {
        assert!(
            !screen.shows(row),
            "the restore left `{row}` of the live region:\n{}",
            screen.dump()
        );
    }
    assert!(status.success(), "{status}");
}

/// A second signal, while the program ends what the first asked, ends the
/// process at once by that signal's own action.
#[cfg(unix)]
#[test]
fn sys_a_second_signal_ends_the_process_at_once() {
    use std::os::unix::process::ExitStatusExt as _;

    let mut session = start("second");
    kill(&session, "TERM");
    session.wait_for(LIMIT, "the hand-over", |t| t.shows(HANDED));
    kill(&session, "TERM");
    let status = session.wait_exit(LIMIT);
    assert_eq!(
        status.signal(),
        Some(15),
        "the second SIGTERM did not end the process by its own action: {status}"
    );
}

/// A SIGHUP after a SIGHUP handed over is the same end, not a second
/// signal: the kernel sends one to the session's leader and its jobs, and a
/// shell resends one, when a terminal closes. The program goes on ending.
#[cfg(unix)]
#[test]
fn sys_a_second_sighup_is_the_same_end() {
    let mut session = start("sighup-twice");
    kill(&session, "HUP");
    session.wait_for(LIMIT, "the child to survive the second", |t| {
        t.shows("survived the second SIGHUP")
    });
    let status = session.wait_exit(LIMIT);
    let screen = session.screen();
    assert!(
        screen.shows("handed over 129") && screen.shows("survived the second SIGHUP"),
        "{status}\n{}",
        screen.dump()
    );
    assert!(status.success(), "{status}");
}

/// The terminal going away under a read, with no signal at all (the
/// program does not have it as its controlling terminal, so the kernel
/// sends it none): its end of input is SIGHUP's end, exit code 129, with
/// the watchdog set.
#[cfg(unix)]
#[test]
fn sys_a_terminal_that_goes_away_under_a_read_is_sighups_end() {
    use std::io::Read as _;

    let scratch = cortex_testkit::Scratch::outside("cortex-term-handoff", "eof");
    let report = scratch.path().join("report.txt");
    let pty = cortex_term::pty::Pty::spawn_without_control(
        &super::this_binary(),
        &HANDOFF.arguments("eof"),
        &[(REPORT.to_owned(), report.display().to_string())],
        80,
        24,
    )
    .expect("start the child");
    // Reads until the live region is drawn, then lets go of this end.
    let mut reader = pty.reader().expect("the output");
    let (seen, drawn) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buffer = [0u8; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            all.extend_from_slice(&buffer[..count]);
            if String::from_utf8_lossy(&all).contains(ROWS[3]) {
                break;
            }
        }
        let _ = seen.send(());
    });
    drawn.recv_timeout(LIMIT).expect("the live region");
    drop(pty);
    let deadline = std::time::Instant::now() + LIMIT;
    let said = loop {
        let text = std::fs::read_to_string(&report).unwrap_or_default();
        if !text.is_empty() || std::time::Instant::now() >= deadline {
            break text;
        }
        std::thread::yield_now();
    };
    assert!(
        // Set at 3 s by the hand-over; a loaded machine may take a second
        // or two to ask.
        ["3", "2", "1"]
            .iter()
            .any(|left| said.contains(&format!("handed over 129, the watchdog at {left} s"))),
        "{said}"
    );
}

/// The SIGHUP of a terminal that closes, handled on one thread while the
/// loop's read finds the terminal gone on another: the kernel sends the
/// hang-up's signal as the read ends, and hands it to whichever thread does
/// not block it. The read took the hand-over while the handler was putting
/// the terminal back, and the handler then found it taken and ended the
/// process by SIGHUP's default action, part-way through its end, before its
/// result was said (the hang-up test of `cortex-tui`, a run in a hundred
/// under load and on hosted runners).
///
/// The child holds the handler in its restore's write with a full pipe for
/// its output, with no timing: the read then finds the end, the loop ends,
/// and the pipe is let go. The process must live through its handler's end
/// and end with SIGHUP's code.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_sighup_handled_on_another_thread_as_the_read_finds_the_end_is_the_same_end() {
    use std::os::unix::process::ExitStatusExt as _;

    let scratch = cortex_testkit::Scratch::outside("cortex-term-handoff", "race");
    let report = scratch.path().join("report.txt");
    // Not its controlling terminal: the only SIGHUP is the child's own.
    let mut pty = cortex_term::pty::Pty::spawn_without_control(
        &super::this_binary(),
        &HANDOFF.arguments("race"),
        &[(REPORT.to_owned(), report.display().to_string())],
        80,
        24,
    )
    .expect("start the child");
    let deadline = std::time::Instant::now() + LIMIT;
    let status = loop {
        if let Some(status) = pty.try_wait().expect("the child's state") {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = pty.kill();
            break None;
        }
        std::thread::yield_now();
    };
    let said = std::fs::read_to_string(&report).unwrap_or_default();
    assert_ne!(
        status.and_then(|status| status.signal()),
        Some(libc::SIGHUP),
        "the SIGHUP that came as the read found the end killed the process: {said}"
    );
    assert!(
        said.contains("handed over 129, and the SIGHUP's handler returned"),
        "{status:?}: {said}"
    );
    assert!(
        status.is_some_and(|status| status.success()),
        "{status:?}: {said}"
    );
}

/// The child's side of the race: everything is the child's own, with no
/// timing.
#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "a child that makes its own pipes, sends a signal to one of its threads, and \
              reads where that thread waits"
)]
mod race {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    use cortex_term::{Overrides, Terminal};

    use super::{HANDED, LIMIT, report};

    /// The thread that takes the SIGHUP, by its id; 0 until it runs.
    static TAKER: AtomicI32 = AtomicI32::new(0);
    /// Set when the taker may end.
    static STOP: AtomicBool = AtomicBool::new(false);

    fn pipe() -> (libc::c_int, libc::c_int) {
        let mut fds = [0; 2];
        // SAFETY: `pipe` fills an array of two descriptors.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "a pipe");
        (fds[0], fds[1])
    }

    fn set_blocking(fd: libc::c_int, blocking: bool) {
        // SAFETY: the flags of a descriptor of this process's own.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            let flags = if blocking {
                flags & !libc::O_NONBLOCK
            } else {
                flags | libc::O_NONBLOCK
            };
            libc::fcntl(fd, libc::F_SETFL, flags);
        }
    }

    /// Writes to `fd` until not one byte more goes in.
    fn fill(fd: libc::c_int) {
        set_blocking(fd, false);
        let chunk = [b'x'; 4096];
        for size in [chunk.len(), 1] {
            // SAFETY: a local buffer and its length, at most.
            while unsafe { libc::write(fd, chunk.as_ptr().cast(), size) } > 0 {}
        }
        set_blocking(fd, true);
    }

    /// Reads all that `fd` holds.
    fn drain(fd: libc::c_int) {
        set_blocking(fd, false);
        let mut buffer = [0u8; 4096];
        // SAFETY: a local buffer and its length.
        while unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) } > 0 {}
    }

    /// The state of thread `tid` of this process (`R`, `S`, `D`...).
    fn state(tid: i32) -> char {
        std::fs::read_to_string(format!("/proc/self/task/{tid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit(')')
                    .next()
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|state| state.chars().next())
            })
            .unwrap_or('?')
    }

    pub(super) fn play() {
        use std::os::unix::thread::JoinHandleExt as _;

        let mut terminal = Terminal::enter(Overrides::default()).expect("enter the terminal");
        terminal.hand_signals_over();
        // The terminal's input becomes a pipe whose writing end this process
        // closes: the end of input a terminal that closes gives the read.
        let (input, input_end) = pipe();
        // Its output becomes a pipe that is full: the handler's restore
        // waits in its write until the pipe is read.
        let (output_read, output) = pipe();
        fill(output);
        // SAFETY: both descriptors are this process's own; standard input
        // and output become them.
        unsafe {
            libc::dup2(input, 0);
            libc::dup2(output, 1);
        }
        // A thread that never blocks SIGHUP and is never asleep, so that it
        // is asleep only in the handler's write.
        let taker = std::thread::spawn(|| {
            // SAFETY: `gettid` reads nothing.
            TAKER.store(unsafe { libc::gettid() }, Ordering::SeqCst);
            while !STOP.load(Ordering::SeqCst) {
                std::hint::spin_loop();
            }
        });
        let deadline = std::time::Instant::now() + LIMIT;
        while TAKER.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "the taker never ran");
            std::thread::yield_now();
        }
        let tid = TAKER.load(Ordering::SeqCst);
        // SAFETY: the thread lives until `STOP`; SIGHUP goes to it alone.
        let sent = unsafe { libc::pthread_kill(taker.as_pthread_t(), libc::SIGHUP) };
        assert_eq!(sent, 0, "SIGHUP to the taker");
        // Asleep: in the handler, putting the terminal back, before it
        // hands the signal over.
        while state(tid) != 'S' {
            assert!(
                std::time::Instant::now() < deadline,
                "the handler never waited in its write"
            );
            std::thread::yield_now();
        }
        // The terminal goes away under the loop's read, which takes the
        // hand-over as SIGHUP's end.
        // SAFETY: the pipe's writing end, which nothing else uses.
        unsafe { libc::close(input_end) };
        let code = loop {
            let read = terminal.next(Some(LIMIT));
            if let Some(code) = terminal.ending() {
                break code;
            }
            if let Err(error) = read {
                report(&format!("a read failed: {error}"));
                #[expect(clippy::exit, reason = "the child ends when it has said so")]
                std::process::exit(0);
            }
            assert!(std::time::Instant::now() < deadline, "no end was seen");
        };
        // The handler's write goes through, and the handler ends: with the
        // process, if it took the signal as a second one.
        drain(output_read);
        STOP.store(true, Ordering::SeqCst);
        taker.join().expect("the taker");
        report(&format!(
            "{HANDED} {code}, and the SIGHUP's handler returned"
        ));
        drop(terminal);
        cortex_term::signal_ended();
        #[expect(clippy::exit, reason = "the child ends when it has said so")]
        std::process::exit(0);
    }
}
