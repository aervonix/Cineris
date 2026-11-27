//! The binary closes its own process to the user's other programs, first
//! thing in `main` (`cineris_sandbox::harden`; the owner's call 8 in the
//! signed approvals' work): on Linux it is not dumpable, so the test, its
//! parent and a process of the same user, cannot read its environment or
//! its program's link, nor open its memory, whatever Yama says; its
//! `stat` and `cmdline`, which the checks of a session's commands read,
//! stay readable. What a session's commands are, and what a nested run
//! reads, is `nested`'s.

use std::os::unix::fs::MetadataExt as _;
use std::process::Stdio;
use std::time::Duration;

use crate::Machine;

#[test]
fn sys_the_binary_closes_its_own_process() {
    let machine = Machine::new("sys_the_binary_closes_its_own_process");
    let mut command = machine.command(&["serve", "--stdio"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("the binary starts");
    let pid = child.id();
    // Root's once the process is not dumpable: what `/proc` shows of a
    // process that closed itself.
    cineris_testkit::children::until(
        Duration::from_secs(10),
        "the binary to close its own process",
        || std::fs::metadata(format!("/proc/{pid}/mem")).is_ok_and(|mem| mem.uid() == 0),
    );
    let environ = std::fs::read(format!("/proc/{pid}/environ"));
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"));
    let mem = std::fs::File::open(format!("/proc/{pid}/mem"));
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"));
    drop(child.stdin.take());
    child.wait().unwrap();
    for (what, error) in [
        ("environ", environ.err()),
        ("exe", exe.err()),
        ("mem", mem.err()),
    ] {
        assert_eq!(
            error.map(|error| error.kind()),
            Some(std::io::ErrorKind::PermissionDenied),
            "{what}"
        );
    }
    assert!(stat.unwrap().starts_with(&format!("{pid} (cineris)")));
    assert!(cmdline.unwrap().ends_with(b"serve\0--stdio\0"));
}
