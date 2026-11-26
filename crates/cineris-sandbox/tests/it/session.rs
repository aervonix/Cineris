//! Whether a process is under a session's command, on Windows, by the real
//! lookups (`cineris_sandbox::session`): the system's list of processes,
//! each one's start, and a harness's job found from outside it. A harness
//! found on the line of parents alone is tested in the crate's own tests
//! (`session::sys::tests`), whose test switch makes the listing give
//! nothing, and which no built binary honours.

use cineris_sandbox::session::{Under, mark, under_a_session};

/// The roles this binary is run again in by
/// `sys_a_service_hosts_start_after_a_harness_began_is_under_a_session`: a
/// harness, a service's host that opens a terminal, the terminal, and an
/// approving subcommand that asks where it stands.
const HARNESS: &str = "cin-session-harness";
const HOST: &str = "cin-session-host";
const TERMINAL: &str = "cin-session-terminal";
const PROBE: &str = "cin-session-probe";

/// The test that runs a role.
const ENTRY: &str = "session::the_roles_this_binary_is_run_again_in";

/// This binary, or a link to it, run again in `role`, with `rest` after
/// `--`.
fn in_role(
    program: &std::path::Path,
    role: &str,
    rest: &[&std::path::Path],
) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command
        .args(["--exact", ENTRY, "--nocapture", role, "--"])
        .args(rest);
    command
}

/// Reads `out` up to the line `said`, past what the test runner prints.
fn read_up_to(out: &mut impl std::io::BufRead, said: &str) {
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(
            out.read_line(&mut line).unwrap(),
            0,
            "it ended before saying {said}"
        );
        if line.trim_end() == said {
            return;
        }
    }
}

/// What the test runner runs as an ordinary test, which passes at once,
/// and what a test runs again in one of the roles above.
#[test]
fn the_roles_this_binary_is_run_again_in() {
    use std::io::{BufRead as _, Read as _, Write as _};
    let arguments: Vec<String> = std::env::args().collect();
    let has = |role: &str| arguments.iter().any(|argument| argument == role);
    let after = arguments
        .iter()
        .position(|argument| argument == "--")
        .map_or(&[][..], |at| &arguments[at + 1..]);
    let code = if has(HARNESS) {
        mark().unwrap();
        println!("marked");
        std::io::stdout().flush().unwrap();
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        0
    } else if has(HOST) {
        // It opens the terminal, the program after `--`, which starts the
        // probe, the next: and ends as they do.
        let terminal = std::path::Path::new(&after[0]);
        let probe = std::path::Path::new(&after[1]);
        let status = in_role(terminal, TERMINAL, &[probe]).status().unwrap();
        status.code().unwrap_or(99)
    } else if has(TERMINAL) {
        println!("ready");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).unwrap();
        let probe = std::path::Path::new(&after[0]);
        let status = in_role(probe, PROBE, &[]).status().unwrap();
        status.code().unwrap_or(99)
    } else if has(PROBE) {
        match under_a_session() {
            Under::Nothing => 10,
            Under::Session(how) => {
                println!("under: {how}");
                11
            }
            Under::Unknown(what) => {
                println!("not known: {what}");
                12
            }
        }
    } else {
        return;
    };
    #[expect(clippy::exit, reason = "a role ends with what it found")]
    std::process::exit(code);
}

/// The real lookups of a service's start: a program named as the task
/// host (a link to this binary) opens a terminal named `cmd.exe` after a
/// harness of this user began, in another process and its own job, and an
/// approving subcommand run in that terminal is under a session's command:
/// the names come from the system's list of processes, the times from each
/// process, and the harness from its job, found by a process outside it.
/// The same terminal opened before every harness is the person's own, but
/// that is shown by `a_services_start_counts_only_after_a_running_harness_began`
/// alone: while the tests run, other tests' harnesses (the CLI's) may have
/// begun before it.
#[test]
fn sys_a_service_hosts_start_after_a_harness_began_is_under_a_session() {
    use std::io::Write as _;
    use std::process::Stdio;
    let scratch = cineris_testkit::scratch::Scratch::outside(
        "cineris-sandbox-session",
        "sys_a_service_hosts_start_after_a_harness_began_is_under_a_session",
    );
    let own = std::env::current_exe().unwrap();
    let link = |name: &str| {
        let path = scratch.join(name);
        if std::fs::hard_link(&own, &path).is_err() {
            std::fs::copy(&own, &path).unwrap();
        }
        path
    };
    let host = link("taskhostw.exe");
    let terminal = link("cmd.exe");
    let mut harness = in_role(&own, HARNESS, &[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    read_up_to(
        &mut std::io::BufReader::new(harness.stdout.take().unwrap()),
        "marked",
    );
    let mut opened = in_role(&host, HOST, &[&terminal, &own])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = std::io::BufReader::new(opened.stdout.take().unwrap());
    read_up_to(&mut out, "ready");
    opened.stdin.take().unwrap().write_all(b"go\n").unwrap();
    let mut said = String::new();
    let _ = std::io::Read::read_to_string(&mut out, &mut said);
    let code = opened.wait().unwrap().code();
    drop(harness.stdin.take());
    harness.wait().unwrap();
    assert_eq!(code, Some(11), "{said}");
}
