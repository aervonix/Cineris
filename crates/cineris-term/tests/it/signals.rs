//! The interrupt hand-off of step 21 (`cineris_term::ignore_interrupts`),
//! with signals delivered deterministically, on Unix.
//!
//! While a guard lives, `Ctrl+C` is for a child and passes over this
//! process; when it ends, a signal still pending is taken (the drain); and
//! for a second after, the terminal's own handler passes an interrupt over
//! (the grace), for one another thread took from the queue as the guard was
//! ending. Step 21 reasoned about all three; nothing tested them, because
//! the race they close needs the kernel to hand a signal to a thread at the
//! wrong moment. Here the moment is made: `cineris_term::pty::
//! interrupt_pending_here` leaves an interrupt pending on the thread, as the
//! kernel leaves one it has not yet handed over, and `raise_interrupt` runs
//! its handler at a known point.
//!
//! Each case runs in a child of its own, since a signal's action is the
//! process's, and each has a control: the same signals with no guard, which
//! must end the child, so that a case that passes shows the guard did it.
//!
//! On Windows there is no pending interrupt to drain: a console control
//! event runs its handler on a thread the system starts for it, which the
//! terminal layer's handler answers as handled while a guard lives.

#[cfg(unix)]
use std::time::Duration;

use cineris_testkit::role::{Role, write_terminal};

/// This binary as a child that plays one case: `drain`, `undrained`,
/// `guarded`, `grace` or `no-grace`.
const SIGNALS: Role = Role::new("cxterm-signals=");

/// What a child prints when it is still alive at the end of its case.
const SURVIVED: &str = "the interrupt passed over";

/// Plays the case this process was started for, if it was.
pub(crate) fn enter() {
    let Some(case) = SIGNALS.options() else {
        return;
    };
    #[cfg(unix)]
    play(&case);
    #[cfg(not(unix))]
    let _ = case;
    write_terminal(&format!("{SURVIVED}\r\n"));
    #[expect(clippy::exit, reason = "a child ends when its case is played")]
    std::process::exit(0);
}

#[cfg(unix)]
fn play(case: &str) {
    use cineris_term::pty::{interrupt_pending_here, raise_interrupt};
    use cineris_term::{Overrides, Terminal};

    match case {
        // An interrupt pending on the thread that ends the guard: the
        // guard takes it as it ends, and unblocking it delivers nothing.
        "drain" => {
            let guard = cineris_term::ignore_interrupts().expect("the guard");
            let pending = interrupt_pending_here().expect("an interrupt pending here");
            assert!(pending.still_pending(), "the interrupt is pending");
            drop(guard);
            assert!(
                !pending.still_pending(),
                "the guard took the pending interrupt"
            );
            drop(pending);
        }
        // The control: unblocked with no guard, it ends the process.
        "undrained" => {
            let pending = interrupt_pending_here().expect("an interrupt pending here");
            drop(pending);
        }
        // In the terminal, an interrupt while the guard lives is passed
        // over; and so is one just after it ended (the grace).
        // The terminal entered while a guard lives, as it is when an editor
        // hands it back: its own handler passes an interrupt over while the
        // guard lives.
        "guarded" => {
            let guard = cineris_term::ignore_interrupts().expect("the guard");
            let terminal = Terminal::enter(Overrides::default()).expect("enter the terminal");
            raise_interrupt().expect("raise the interrupt");
            drop(guard);
            terminal.leave().expect("leave the terminal");
        }
        // In the terminal, an interrupt just after the guard ended is passed
        // over (the grace).
        "grace" => {
            let terminal = Terminal::enter(Overrides::default()).expect("enter the terminal");
            let guard = cineris_term::ignore_interrupts().expect("the guard");
            drop(guard);
            raise_interrupt().expect("raise the interrupt");
            terminal.leave().expect("leave the terminal");
        }
        // The control: in the terminal with no guard, it ends the process,
        // the terminal put back.
        "no-grace" => {
            let _terminal = Terminal::enter(Overrides::default()).expect("enter the terminal");
            raise_interrupt().expect("raise the interrupt");
        }
        other => panic!("no case `{other}`"),
    }
}

/// Runs `case` in a pseudo-terminal, and returns its exit status and
/// whether it said it survived. A case expected to survive is waited for
/// until it says so: the program can end before its last output is read.
#[cfg(unix)]
fn run(case: &str, survives: bool) -> (std::process::ExitStatus, bool) {
    use cineris_testkit::pty::Session;

    let limit = Duration::from_secs(30);
    let mut session =
        Session::start(&super::this_binary(), &SIGNALS.arguments(case), 80, 24).answering();
    if survives {
        session.wait_for(limit, "the case to say it survived", |t| t.shows(SURVIVED));
    }
    let status = session.wait_exit(limit);
    let survived = session.screen().shows(SURVIVED);
    (status, survived)
}

#[cfg(unix)]
#[test]
fn sys_an_interrupt_pending_as_the_guard_ends_is_taken_by_it() {
    use std::os::unix::process::ExitStatusExt as _;

    let (status, survived) = run("drain", true);
    assert!(survived && status.success(), "drain: {status}");
    // Without the guard the same pending interrupt ends the child: the case
    // above is not passing for want of a signal.
    let (status, survived) = run("undrained", false);
    assert!(!survived, "undrained survived: {status}");
    assert_eq!(status.signal(), Some(libc::SIGINT), "undrained: {status}");
}

#[cfg(unix)]
#[test]
fn sys_an_interrupt_while_the_guard_lives_or_just_after_is_passed_over() {
    use std::os::unix::process::ExitStatusExt as _;

    for case in ["guarded", "grace"] {
        let (status, survived) = run(case, true);
        assert!(survived && status.success(), "{case}: {status}");
    }
    // In the terminal with no guard, the interrupt ends the child.
    let (status, survived) = run("no-grace", false);
    assert!(!survived, "no-grace survived: {status}");
    assert_eq!(status.signal(), Some(libc::SIGINT), "no-grace: {status}");
}
