//! What the harness starts on Windows has a console of its own, never the
//! harness's (the signed approvals' part "commands without the harness's
//! console"): a session's command, the harness's own git and what a
//! repository's configuration has git run, and the ordinary programs a
//! command runs still work there.
//!
//! The program that says what console it has is this test binary run
//! again, as [`REPORT`]; the test is the harness, on the console it was
//! started on.

use std::process::{Command, Stdio};

use cineris_sandbox::tree::console_processes;

use crate::ENTRY;
use crate::bench::{self, Bench, default_shell, invoke};

/// The argument that makes this binary say what console it has.
pub(crate) const REPORT: &str = "cin-console";

/// The body of [`REPORT`]: two lines on standard output, then the process
/// ends.
pub(crate) fn report() -> ! {
    println!("{REPORT} pid {}", std::process::id());
    match console_processes() {
        Ok(ids) => {
            let ids: Vec<String> = ids.iter().map(u32::to_string).collect();
            println!("{REPORT} console {}", ids.join(" "));
        }
        Err(error) => println!("{REPORT} no-console {error}"),
    }
    #[expect(clippy::exit, reason = "a child process ends when it has answered")]
    std::process::exit(0);
}

/// The processes on the console of the program that reported in `text`:
/// none when it said it had no console.
fn reported_console(text: &str) -> Vec<u32> {
    let said = |what: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(&format!("{REPORT} {what}")))
            .map(str::trim)
    };
    let pid: u32 = said("pid ")
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| panic!("no report: {text:?}"));
    match said("console ") {
        Some(ids) => {
            let ids: Vec<u32> = ids
                .split_whitespace()
                .map(|id| id.parse().expect("a process id"))
                .collect();
            assert!(ids.contains(&pid), "{ids:?} without {pid}");
            ids
        }
        None => Vec::new(),
    }
}

/// The test's own console, which nothing the harness starts may share.
fn own_console() {
    let ids = console_processes().expect("the test runs on a console, as a harness in a terminal");
    assert!(ids.contains(&std::process::id()), "{ids:?}");
}

/// The arguments that make this binary [`REPORT`].
fn report_arguments() -> Vec<String> {
    ["--exact", ENTRY, "--nocapture", REPORT]
        .map(str::to_owned)
        .to_vec()
}

fn this_binary() -> String {
    std::env::current_exe()
        .expect("the test binary has a path")
        .display()
        .to_string()
}

/// Through the session's command path: the shell tool, its runner and its
/// process tree. `cmd.exe` starts a program on its own console, so the
/// program's console is the command's.
#[test]
fn sys_a_shell_command_does_not_share_the_harnesss_console() {
    own_console();
    let Some(shell) = bench::shell("cmd") else {
        return;
    };
    let bench = Bench::new("console_shell_command", shell);
    let ran = bench.command(&invoke(bench.shell(), &this_binary(), &report_arguments()));
    assert_eq!(
        ran.payload("exit_code"),
        serde_json::json!(0),
        "{}",
        ran.text()
    );
    let console = reported_console(&ran.text());
    assert!(
        !console.is_empty(),
        "a command has a console: {}",
        ran.text()
    );
    assert!(
        !console.contains(&std::process::id()),
        "the command is on the harness's console: {console:?}"
    );
}

/// The ordinary programs a command runs still work on a console of the
/// command's own: git, cargo, `cmd.exe`, PowerShell's own commands, and a
/// `.cmd` shim of the kind npm writes. One PowerShell runs them all.
#[test]
fn sys_ordinary_commands_work_without_the_harnesss_console() {
    let bench = Bench::new("console_ordinary_commands", default_shell());
    let shim = bench.ws.root().join("shim.cmd");
    let node =
        cineris_base::program::find(&cineris_base::program::path_directories(), "node", true);
    // A shim as npm writes one runs node; with no node here, another
    // console program stands in.
    let (body, said) = match &node {
        Some(_) => ("@node -e \"console.log('shim-' + 'ran')\"\r\n", "shim-ran"),
        None => ("@cmd /d /c echo shim-ran\r\n", "shim-ran"),
    };
    std::fs::write(&shim, body).expect("write the shim");
    let quoted = bench
        .shell()
        .quote(&shim.display().to_string())
        .expect("a path");
    let command = format!(
        "git --version; cargo --version; cmd /d /c echo plain-cmd; \
         'here: ' + (Get-Location).Path; & {quoted}"
    );
    let ran = bench.command(&command);
    let text = ran.text();
    assert_eq!(ran.payload("exit_code"), serde_json::json!(0), "{text}");
    assert!(text.contains("git version"), "{text}");
    assert!(text.contains("cargo "), "{text}");
    assert!(text.contains("plain-cmd"), "{text}");
    // The workspace, in whatever spelling the resolver keeps.
    assert!(
        text.lines().any(|line| {
            let line = line.trim().to_ascii_lowercase();
            line.starts_with("here: ") && line.ends_with("console_ordinary_commands\\ws")
        }),
        "{text}"
    );
    assert!(text.contains(said), "the shim ran nothing: {text}");
}

/// The harness's own git, and what a repository's configuration has git
/// run (here an alias, which git runs through its own `sh`): the program
/// it starts is on git's console, which is not the harness's. A git
/// started as the harness started it before shows that the path through
/// `sh` passes the console on, so that the check is not empty.
#[test]
fn sys_the_harnesss_git_does_not_share_its_console() {
    own_console();
    let Some(git) = cineris_base::program::git() else {
        assert!(std::env::var_os("CI").is_none(), "CI has git");
        return;
    };
    let dir = std::env::temp_dir();
    // Forward slashes, which git's `sh` reads as Windows' own.
    let program = this_binary().replace('\\', "/");
    let alias = format!(
        "alias.cin-console=!'{program}' {}",
        report_arguments().join(" ")
    );
    let arguments = ["-c", alias.as_str(), "cin-console"];

    let shared = Command::new(git)
        .args(arguments)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("git starts");
    let console = reported_console(&String::from_utf8_lossy(&shared.stdout));
    assert!(
        console.contains(&std::process::id()),
        "a git that shares the console passes it on: {console:?}"
    );

    let mut child = cineris_base::git::start(&cineris_base::git::AsItIs, &dir, &arguments)
        .expect("the harness's git starts");
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take().expect("a pipe"), &mut out)
        .expect("read git's output");
    assert!(child.wait().expect("wait").success(), "{out}");
    let console = reported_console(&out);
    assert!(!console.is_empty(), "{out}");
    assert!(
        !console.contains(&std::process::id()),
        "the harness's git shares its console: {console:?}"
    );
}
