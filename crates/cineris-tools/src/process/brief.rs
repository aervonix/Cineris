//! A short program run for its answer: a shell asked for its version
//! ([`run`]), `git` asked what has changed ([`answer`], for a git
//! `cineris_base::git::start` started with its own environment).
//!
//! This is not how a model's command runs. It is for the harness's own
//! questions to programs it chose itself: no process tree, no artifact, no
//! staging, and a limit on how long its answer may take and how large it
//! may be. A program that runs over either is killed, and the answer is
//! that there is none.
//!
//! It blocks, and is called off the core thread.

#![allow(
    clippy::disallowed_methods,
    reason = "runs on the blocking pool, and waits for a program with a limit"
)]

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// The most a brief program may print before its answer is given up.
const MOST_BYTES: u64 = 8 * 1024 * 1024;

/// Why a brief program gave no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unanswered {
    /// The program could not be started: it is not there.
    NotStarted,
    /// It ran and did not exit with success.
    Failed,
    /// It printed more than an answer may be.
    TooMuch,
    /// It had not answered when the limit passed, and was killed.
    TooSlow,
}

/// Runs `program` with `arguments` and returns what it wrote to its
/// standard output, if it exited with success within `limit`.
///
/// Its standard error is thrown away and its input is closed. `env` is its
/// whole environment: nothing is inherited, so that a program the harness
/// starts for itself is given only what its caller built for it (no
/// provider key, a `PATH` of absolute directories), never the harness's
/// own by default (the harness's own processes' fixes).
pub(crate) fn run(
    program: impl AsRef<OsStr>,
    arguments: &[&str],
    cwd: Option<&Path>,
    env: &[(OsString, OsString)],
    limit: Duration,
) -> Result<Vec<u8>, Unanswered> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .env_clear()
        .envs(env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    // The program is the session's shell, which runs the model's commands:
    // never on the harness's console.
    cineris_base::program::without_console(&mut command);
    let child = command.spawn().map_err(|_| Unanswered::NotStarted)?;
    answer(child, limit)
}

/// What a program already started wrote to its standard output, which
/// must be a pipe, if it exited with success within `limit`: for one its
/// caller started its own way (the harness's git, which
/// `cineris_base::git::start` starts). A program that runs over the limit,
/// or prints too much, is killed.
pub(crate) fn answer(mut child: Child, limit: Duration) -> Result<Vec<u8>, Unanswered> {
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Unanswered::Failed);
    };
    // The read is on a thread of its own, so that this one can stop
    // waiting for it: a pipe has no time limit of its own.
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("cineris-brief-output".to_owned())
        .spawn(move || {
            let mut bytes = Vec::new();
            let read = stdout.take(MOST_BYTES + 1).read_to_end(&mut bytes);
            let _ = sender.send(read.map(|_| bytes));
        });
    if reader.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Unanswered::Failed);
    }
    let answer = match receiver.recv_timeout(limit) {
        Ok(Ok(bytes)) if bytes.len() as u64 <= MOST_BYTES => Ok(bytes),
        Ok(Ok(_)) => Err(Unanswered::TooMuch),
        Ok(Err(_)) => Err(Unanswered::Failed),
        Err(_) => Err(Unanswered::TooSlow),
    };
    if answer.is_err() {
        let _ = child.kill();
    }
    // The output has ended, or the program was killed. A program exits a
    // moment after it closes its output, and this waits for that moment.
    let status = child.wait().map_err(|_| Unanswered::Failed)?;
    match answer {
        Ok(_) if !status.success() => Err(Unanswered::Failed),
        answer => answer,
    }
}

/// On Windows the program asked has a console of its own, not the
/// harness's (the signed approvals' part "commands without the harness's
/// console"). The program is this test binary run again.
#[cfg(all(test, windows))]
mod tests {
    use std::time::Duration;

    use super::run;

    /// The argument that makes this test binary say which processes share
    /// its console.
    const REPORT: &str = "cin-console";

    /// The test by which this binary is run again as [`REPORT`].
    const ENTRY: &str = "process::brief::tests::console_entry";

    /// What the test runner runs as an ordinary test, which passes at once,
    /// and what the test below runs again as the program it asks.
    #[test]
    fn console_entry() {
        if std::env::args().any(|argument| argument == REPORT) {
            let ids = cineris_sandbox::tree::console_processes()
                .map(|ids| ids.iter().map(u32::to_string).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            println!("{REPORT} {ids}");
            #[expect(clippy::exit, reason = "a child process ends when it has answered")]
            std::process::exit(0);
        }
    }

    #[test]
    fn sys_a_brief_program_does_not_share_the_harnesss_console() {
        let own = cineris_sandbox::tree::console_processes()
            .expect("the test runs on a console, as a harness in a terminal");
        assert!(own.contains(&std::process::id()), "{own:?}");
        let env: Vec<_> = std::env::vars_os().collect();
        let program = std::env::current_exe().expect("the test binary has a path");
        let said = run(
            &program,
            &["--exact", ENTRY, "--nocapture", REPORT],
            None,
            &env,
            Duration::from_secs(30),
        )
        .expect("the program answers");
        let text = String::from_utf8_lossy(&said);
        let ids: Vec<u32> = text
            .lines()
            .find_map(|line| line.strip_prefix(REPORT))
            .unwrap_or_else(|| panic!("no report: {text:?}"))
            .split_whitespace()
            .map(|id| id.parse().expect("a process id"))
            .collect();
        assert!(!ids.is_empty(), "the program has a console: {text:?}");
        assert!(
            !ids.contains(&std::process::id()),
            "the program shares the harness's console: {ids:?}"
        );
    }
}
