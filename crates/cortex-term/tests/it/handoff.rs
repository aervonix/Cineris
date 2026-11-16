//! Signals handed to the program (`Terminal::hand_signals_over`, step 23),
//! on Unix: the restore erases the whole live region, the watchdog is set
//! when the first signal is handed over, a second signal ends the process
//! at once, and the terminal hanging up under a read is SIGHUP's end (the
//! review of step 23).
//!
//! Each case runs in a child of its own in a pseudo-terminal, this test
//! binary run again: a signal's action is the process's.

#[cfg(unix)]
use std::time::Duration;

use cortex_testkit::role::Role;

/// This binary as a child that plays one case: `watchdog` or `second`.
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

/// Plays the case this process was started for, if it was.
pub(crate) fn enter() {
    let Some(case) = HANDOFF.options() else {
        return;
    };
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
        let _ = terminal.next(None);
        if let Some(code) = terminal.ending() {
            break code;
        }
    };
    let left = cortex_term::pty::watchdog_left();
    write_terminal(&format!("{HANDED} {code}, the watchdog at {left} s\r\n"));
    match case {
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
    let status = session.wait_exit(LIMIT);
    let screen = session.screen();
    let said = screen
        .history()
        .into_iter()
        .find(|row| row.contains(HANDED))
        .unwrap_or_else(|| panic!("the child never said: {status}\n{}", screen.dump()));
    assert!(
        said.contains("handed over 143, the watchdog at 3 s")
            || said.contains("handed over 143, the watchdog at 2 s"),
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
