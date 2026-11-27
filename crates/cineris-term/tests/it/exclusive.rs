//! The terminal's exclusive mode across a stop and a continue (the signed
//! approvals: `Terminal::set_exclusive`, which the approvals' password
//! turns on while it is asked), on Linux: while it is on, a second open of
//! the terminal is refused (`EBUSY`), and it is on again after the program
//! was stopped (`SIGTSTP`, which the terminal's handler takes, turning it
//! off) and continued.
//!
//! The child is this test binary run again in a pseudo-terminal of its own:
//! a signal's action is the process's.

use cineris_testkit::role::Role;

/// This binary as a child that turns the exclusive mode on, says where its
/// terminal is, and says each time it is continued, until `q`.
const EXCLUSIVE: Role = Role::new("cinterm-exclusive=");

/// What the child says once its terminal is closed to others.
#[cfg(target_os = "linux")]
const ON: &str = "closed to others";

/// What the child says each time it was continued.
#[cfg(target_os = "linux")]
const RESUMED: &str = "continued";

/// Plays the child, if this process was started as it.
pub(crate) fn enter() {
    if EXCLUSIVE.options().is_none() {
        return;
    }
    #[cfg(target_os = "linux")]
    play();
    #[expect(clippy::exit, reason = "a child ends when it has played")]
    std::process::exit(0);
}

#[cfg(target_os = "linux")]
fn play() {
    use cineris_term::{Event, KeyCode, Overrides, Terminal};
    use cineris_testkit::role::write_terminal;

    let mut terminal = Terminal::enter(Overrides::default()).expect("enter the terminal");
    terminal.set_exclusive(true).expect("the exclusive mode");
    let path = std::fs::read_link("/proc/self/fd/0").expect("the terminal's path");
    write_terminal(&format!("{ON} at {}\r\n", path.display()));
    let mut resumed = 0;
    loop {
        match terminal.next(None) {
            Ok(Some(Event::Resumed)) => {
                resumed += 1;
                write_terminal(&format!("{RESUMED} {resumed}\r\n"));
            }
            Ok(Some(Event::Key(key))) if key.code == KeyCode::Char('q') => break,
            Ok(_) => {}
            Err(error) => panic!("the terminal: {error}"),
        }
    }
    drop(terminal);
}

/// Whether a second open of the terminal at `path` is refused as busy.
#[cfg(target_os = "linux")]
fn busy(path: &str) -> bool {
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    {
        Ok(_) => false,
        Err(error) => error.raw_os_error() == Some(libc::EBUSY),
    }
}

/// While the exclusive mode is on, the terminal cannot be opened again;
/// after a stop and a continue it is on again.
#[cfg(target_os = "linux")]
#[test]
fn sys_the_exclusive_mode_holds_and_comes_back_after_a_stop() {
    use std::time::Duration;

    const LIMIT: Duration = Duration::from_secs(30);
    // Root opens it all the same (`CAP_SYS_ADMIN`): there is nothing to see.
    let id = std::process::Command::new("id")
        .arg("-u")
        .output()
        .expect("id");
    if String::from_utf8_lossy(&id.stdout).trim() == "0" {
        return;
    }
    let mut session = cineris_testkit::pty::Session::start(
        &super::this_binary(),
        &EXCLUSIVE.arguments(""),
        80,
        24,
    )
    .answering();
    session.wait_for(LIMIT, "the exclusive mode", |t| t.shows(ON));
    let path = session
        .screen()
        .history()
        .into_iter()
        .find_map(|row| {
            row.split_once(" at ")
                .map(|(_, path)| path.trim().to_owned())
        })
        .expect("the terminal's path");
    assert!(busy(&path), "a second open of {path} was not refused");
    let pid = session.pid().to_string();
    for (n, signal) in [(1, "-TSTP"), (2, "-TSTP")] {
        for signal in [signal, "-CONT"] {
            let sent = std::process::Command::new("kill")
                .args([signal, pid.as_str()])
                .status()
                .expect("kill");
            assert!(sent.success(), "{signal} was not sent");
        }
        let said = format!("{RESUMED} {n}");
        session.wait_for(LIMIT, "the continue", |t| t.shows(&said));
        assert!(
            busy(&path),
            "a second open of {path} was not refused after continue {n}"
        );
    }
    session.send(b"q");
    let status = session.wait_exit(LIMIT);
    assert!(status.success(), "{status}");
    assert!(!busy(&path), "the terminal stayed closed once put back");
}
