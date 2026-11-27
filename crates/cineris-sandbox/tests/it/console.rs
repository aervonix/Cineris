//! A command has a console of its own, never the harness's (Windows; the
//! signed approvals' part "commands without the harness's console"): what
//! the person types into the harness's console is not a command's to read,
//! and a question the harness asks there is not a command's to answer.
//!
//! The command is this test binary run again, as [`REPORT`]: it says which
//! processes share its console and whether it could open the console's
//! input. The test is the harness, with the console it was started on.

use std::io;
use std::process::{Command, Stdio};

use cineris_sandbox::tree::{Exit, Output, ReadOutcome, Tree, console_processes};
use cineris_testkit::children::{ChildSpec, Roll};

use crate::{ENTRY, LIMIT, guard};

/// The argument that makes this binary say what console it has.
pub(crate) const REPORT: &str = "cin-console";

/// The code a process ends with when a Ctrl-Break event ends it
/// (`STATUS_CONTROL_C_EXIT`), as [`Exit::Code`] shows it.
const ENDED_BY_THE_EVENT: i32 = i32::from_ne_bytes(0xC000_013A_u32.to_ne_bytes());

/// The body of [`REPORT`]: three lines on standard output, then the
/// process ends.
pub(crate) fn report() -> ! {
    let console = console_processes();
    let input = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONIN$");
    println!("{REPORT} pid {}", std::process::id());
    match console {
        Ok(ids) => {
            let ids: Vec<String> = ids.iter().map(u32::to_string).collect();
            println!("{REPORT} console {}", ids.join(" "));
        }
        Err(error) => println!("{REPORT} no-console {error}"),
    }
    match input {
        Ok(_) => println!("{REPORT} input opened"),
        Err(error) => println!("{REPORT} input {error}"),
    }
    #[expect(clippy::exit, reason = "a child process ends when it has answered")]
    std::process::exit(0);
}

/// What a command said of its console.
#[derive(Debug)]
struct Report {
    pid: u32,
    /// The processes on its console, or why it has none.
    console: Result<Vec<u32>, String>,
    /// Whether it could open its console's input, or why not.
    input: Result<(), String>,
}

fn read_report(text: &str) -> Report {
    let said = |what: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{REPORT} {what}")))
            .map(str::trim)
    };
    let pid = said("pid ")
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| panic!("no report: {text:?}"));
    let console = match (said("console "), said("no-console ")) {
        (Some(ids), _) => Ok(ids
            .split_whitespace()
            .map(|id| id.parse().expect("a process id"))
            .collect()),
        (None, Some(why)) => Err(why.to_owned()),
        (None, None) => panic!("no console line: {text:?}"),
    };
    let input = match said("input ") {
        Some("opened") => Ok(()),
        Some(why) => Err(why.to_owned()),
        None => panic!("no input line: {text:?}"),
    };
    Report {
        pid,
        console,
        input,
    }
}

/// Reads an output pipe to its end.
fn read_all(output: &mut Output) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        match output.read(&mut buffer).expect("read the output") {
            ReadOutcome::Bytes(count) => bytes.extend_from_slice(&buffer[..count]),
            ReadOutcome::Closed => break,
            ReadOutcome::Stopped => panic!("nothing stopped the read"),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The test's own console, which no command may share: a test runs on one,
/// as a harness started in a terminal does.
fn own_console() -> Vec<u32> {
    let ids = console_processes().expect("the test runs on a console, as a harness in a terminal");
    assert!(ids.contains(&std::process::id()), "{ids:?}");
    ids
}

#[test]
fn sys_a_command_has_a_console_of_its_own_and_not_the_harnesss() {
    own_console();
    let mut command = Command::new(std::env::current_exe().expect("the test binary has a path"));
    command
        .args(["--exact", ENTRY, "--nocapture", REPORT])
        .stdin(Stdio::null());
    let (mut output, _stop) = Output::attach(&mut command).expect("make the pipe");
    let tree = Tree::spawn(command, None).expect("start the command");
    let text = read_all(&mut output);
    assert!(tree.wait().expect("wait").success(), "{text}");
    tree.end();

    let report = read_report(&text);
    assert_eq!(report.pid, tree.id());
    let console = report
        .console
        .expect("a console program the tree starts has a console");
    assert!(console.contains(&report.pid), "{console:?}");
    assert!(
        !console.contains(&std::process::id()),
        "the command is on the harness's console: {console:?}"
    );
    // It can open a console's input, and it is its own console's, which
    // nobody types into. Its standard output is still the pipe: the
    // report came through it.
    report
        .input
        .expect("the command opens its own console's input");
}

/// Without the guard's program there is nothing to send the event from
/// inside the command's console, and the tree says so rather than send it
/// from the harness's: there it would reach nothing of the command's,
/// report success, and leave the command's id in the harness's console's
/// list (microsoft/terminal issue 335).
#[test]
fn sys_without_a_guard_the_interrupt_is_refused_and_the_harnesss_console_is_left_alone() {
    let before = own_console();
    let roll = Roll::new();
    let command = ChildSpec::new(&roll, "only").command(ENTRY);
    let tree = Tree::spawn(command, None).expect("start the command");
    let only = roll.next(LIMIT);

    let error = tree
        .interrupt()
        .expect_err("no interrupt without the guard's program");
    assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{error}");
    assert!(only.is_alive(), "nothing was sent");
    let after = console_processes().expect("the test still has its console");
    assert!(
        !after.contains(&tree.id()),
        "the command's id was put on the harness's console: {before:?}, then {after:?}"
    );

    tree.end();
    assert!(only.gone_within(LIMIT));
}

/// With the guard's program, the event is sent from inside the command's
/// console, and ends a command that listens with the event's own code, not
/// the kill's.
#[test]
fn sys_the_interrupt_is_sent_from_inside_the_commands_own_console() {
    let roll = Roll::new();
    let command = ChildSpec::new(&roll, "only").command(ENTRY);
    let tree = Tree::spawn(command, Some(&guard())).expect("start the command");
    let only = roll.next(LIMIT);

    tree.interrupt()
        .expect("the guard's program sends the event");
    assert!(only.gone_within(LIMIT), "the command did not get the event");
    assert_eq!(tree.wait().expect("wait"), Exit::Code(ENDED_BY_THE_EVENT));
    let after = console_processes().expect("the test still has its console");
    assert!(!after.contains(&tree.id()), "{after:?}");
    tree.end();
}
