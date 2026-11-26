//! Signals, the guard of process trees, and a run with no console
//! (`docs/architecture/10-headless.md`, "Signals"; `02-runtime.md`,
//! "Cancellation").
//!
//! The binary is started as a process tree of its own
//! (`cineris_sandbox::tree::Tree`), which is what lets a test send it what a
//! terminal would: SIGINT and SIGTERM to its process group on Linux, a
//! Ctrl-Break event on Windows. Ctrl-C itself cannot be sent to one process
//! group on Windows, so there the first signal is Ctrl-Break, whose code is
//! 143.

use std::fs::File;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use cineris_sandbox::tree::{Exit, Tree};
use cineris_testkit::children::{ChildSpec, Roll};
use cineris_testkit::{Gate, ServerReply};
use serde_json::json;

use crate::{CHILD_ENTRY, Machine, Run};

/// A bound on a mistake, never waited out by a test that passes.
const LIMIT: Duration = Duration::from_secs(30);

/// The binary, running as a tree of its own, with its streams in files.
struct Running {
    tree: Tree,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Running {
    fn start(machine: &Machine, arguments: &[&str]) -> Self {
        let stdout = machine.home.join("stdout.txt");
        let stderr = machine.home.join("stderr.txt");
        let mut command = machine.command(arguments);
        command
            .stdout(Stdio::from(File::create(&stdout).unwrap()))
            .stderr(Stdio::from(File::create(&stderr).unwrap()));
        let tree = Tree::spawn(command, None).expect("the binary starts");
        Self {
            tree,
            stdout,
            stderr,
        }
    }

    /// Waits for the binary to exit, and reads what it wrote.
    fn finish(self) -> Run {
        let exit = self.tree.wait().unwrap();
        self.read(exit)
    }

    /// Reads what the binary wrote, once it has exited as `exit` says.
    fn read(self, exit: Exit) -> Run {
        self.tree.end();
        Run {
            code: match exit {
                Exit::Code(code) => Some(code),
                Exit::Signal(_) => None,
            },
            stdout: std::fs::read_to_string(&self.stdout).unwrap(),
            stderr: std::fs::read_to_string(&self.stderr).unwrap(),
        }
    }
}

/// [`Running`], with a thread that waits for the binary to exit and opens
/// `exited` when it has: a test bounds its wait for the exit, and sees an
/// exit that comes while it waits for something else.
struct Watched {
    running: Running,
    exited: Gate,
    waiter: std::thread::JoinHandle<Exit>,
}

impl Watched {
    fn start(machine: &Machine, arguments: &[&str]) -> Self {
        let running = Running::start(machine, arguments);
        let exited = Gate::new();
        let waiter = {
            let (tree, exited) = (running.tree.clone(), exited.clone());
            std::thread::spawn(move || {
                let exit = tree.wait().unwrap();
                exited.open();
                exit
            })
        };
        Self {
            running,
            exited,
            waiter,
        }
    }

    /// Waits up to [`LIMIT`] for `done`, and no longer once the binary has
    /// exited. Whether `done` came.
    fn until(&self, what: &str, done: impl Fn() -> bool) -> bool {
        let _doing = cineris_testkit::watch::doing(format!("waits for {what}, up to {LIMIT:?}"));
        let began = Instant::now();
        loop {
            if done() {
                return true;
            }
            if self.exited.is_open() || began.elapsed() >= LIMIT {
                return false;
            }
            std::thread::yield_now();
        }
    }

    /// What the binary left, and whether it exited by itself within
    /// `limit`: when it had not, it was killed.
    fn end_within(self, limit: Duration) -> (Run, bool) {
        let by_itself = self.exited.wait(limit);
        if !by_itself {
            let _ = self.running.tree.kill();
        }
        let exit = self.waiter.join().unwrap();
        (self.running.read(exit), by_itself)
    }
}

/// A response that has begun and goes no further until the client hangs
/// up: the run is waiting on the model when the signal comes. `begun` opens
/// when the server has sent all it will send, so a signal sent after it
/// finds the server holding the connection, never still writing to it.
fn held_response(machine: &Machine, begun: &Gate, closed: &Gate) {
    let started = format!(
        "event: message_start\ndata: {}\n\n",
        json!({"type": "message_start", "message": {"model": crate::MODEL, "id": "msg_held",
            "type": "message", "role": "assistant", "content": [], "stop_reason": null,
            "usage": {"input_tokens": 5, "output_tokens": 1}}})
    );
    machine.server.push(
        ServerReply::sse()
            .send(started)
            .mark(begun)
            .hang_until_closed(closed),
    );
}

fn interrupted_by(signal: impl FnOnce(&Tree), code: i32, test: &str) {
    let machine = Machine::new(test);
    let (begun, closed) = (Gate::new(), Gate::new());
    held_response(&machine, &begun, &closed);
    let running = Running::start(&machine, &["-p", "wait for me", "--output", "json"]);
    assert!(begun.wait(LIMIT), "the response began");
    signal(&running.tree);
    let run = running.finish();
    let record = run.expect_code(code).json();
    assert_eq!(record["outcome"], "interrupted");
    // Dropping the response is how the model is told: the connection went.
    assert!(closed.wait(LIMIT), "the request was dropped");
}

#[cfg(unix)]
#[test]
fn sys_ctrl_c_interrupts_the_turn_and_exits_130() {
    interrupted_by(|tree| tree.interrupt().unwrap(), 130, "sys_sigint");
}

#[cfg(unix)]
#[test]
fn sys_sigterm_interrupts_the_turn_and_exits_143() {
    interrupted_by(|tree| tree.terminate().unwrap(), 143, "sys_sigterm");
}

#[cfg(windows)]
#[test]
fn sys_ctrl_break_interrupts_the_turn_and_exits_143() {
    interrupted_by(|tree| tree.interrupt().unwrap(), 143, "sys_ctrl_break");
}

/// The project's extension `probe`, approved with the project as
/// `permissions trust --with-extensions` approves it: its command `spin`
/// notes in the project's values that it runs, then runs until it is
/// stopped. The file the note is kept in, which is there once it runs.
fn spinning_command(machine: &Machine) -> PathBuf {
    let folder = machine.workspace.join(".cineris/extensions/probe");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"A test extension\"\nmain = \
         \"main.js\"\n[capabilities]\n",
    )
    .unwrap();
    std::fs::write(
        folder.join("main.js"),
        "cineris.command({ name: 'spin', description: 'Runs until it is stopped', async run() {\n\
         \x20 await cineris.state.set('started', true, { scope: 'project' });\n\
         \x20 for (;;) {}\n\
         } });\n",
    )
    .unwrap();
    machine
        .run(&["permissions", "trust", "--with-extensions"])
        .expect_code(0);
    // `projects/<project key>/extensions/project-probe.json` in the state
    // directory (`cineris_core::ext::state`).
    let key = cineris_base::PathResolver::new(&machine.workspace, cineris_base::PathOptions::new())
        .unwrap()
        .root()
        .project_key();
    cineris_config::StateDir::from_dirs(&machine.dirs())
        .unwrap()
        .project_dir(&key)
        .unwrap()
        .join("extensions")
        .join("project-probe.json")
}

/// A signal while an extension's command runs, inside `turn/start`, stops
/// the command through its own token: the run ends interrupted at once,
/// with the signal's code, and nothing was sent to the model. The command
/// would run for its whole time otherwise (two minutes, the manifest's
/// default), and the wait for the exit is far shorter than that.
fn command_stopped_by(signal: impl FnOnce(&Tree), code: i32, test: &str) {
    let machine = Machine::new(test);
    let values = spinning_command(&machine);
    let running = Watched::start(&machine, &["-p", "/probe:spin", "--output", "json"]);
    if !running.until("the command to run", || values.exists()) {
        let (run, by_itself) = running.end_within(Duration::ZERO);
        panic!(
            "the command did not run (the binary {}):\n{}",
            if by_itself {
                "exited first"
            } else {
                "was killed"
            },
            crate::shown(&run)
        );
    }
    let kept: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&values).unwrap()).unwrap();
    assert_eq!(kept["values"]["started"], true, "{kept}");

    signal(&running.running.tree);
    let (run, by_itself) = running.end_within(LIMIT);
    assert!(
        by_itself,
        "the binary was still running {LIMIT:?} after the signal, and was killed:\n{}",
        crate::shown(&run)
    );
    // Standard error names the command and whose it is before it runs.
    assert!(
        run.stderr
            .contains("Sending `/probe:spin`, the project's extension `probe`'s command."),
        "{}",
        run.stderr
    );
    let record = run.expect_code(code).json();
    assert_eq!(record["outcome"], "interrupted", "{record}");
    assert!(
        machine.server.requests().is_empty(),
        "the model was asked: {record}"
    );
}

#[cfg(unix)]
#[test]
fn sys_ctrl_c_stops_a_running_extension_command() {
    command_stopped_by(|tree| tree.interrupt().unwrap(), 130, "sys_sigint_command");
}

#[cfg(windows)]
#[test]
fn sys_ctrl_break_stops_a_running_extension_command() {
    command_stopped_by(
        |tree| tree.interrupt().unwrap(),
        143,
        "sys_ctrl_break_command",
    );
}

/// The command a shell runs to start `spec`: the test binary, quoted for
/// the shell the harness finds (bash or sh on Linux, PowerShell on
/// Windows).
pub(crate) fn shell_line(spec: &ChildSpec) -> String {
    let (program, arguments) = spec.arguments(CHILD_ENTRY);
    let quoted = format!("'{}'", program.replace('\'', "''"));
    let call = if cfg!(windows) {
        format!("& {quoted}")
    } else {
        quoted.replace("''", "'\\''")
    };
    format!("{call} {}", arguments.join(" "))
}

/// A run whose model starts `spec` with the shell tool and then waits.
fn commanding(machine: &Machine, spec: &ChildSpec) {
    machine.reply_tool(
        "toolu_1",
        "shell",
        &json!({"command": shell_line(spec), "timeout_ms": 600_000}),
    );
    machine.reply_text("Done.");
}

/// The second Ctrl-C is `turn/interrupt` with `now`: a command that is
/// being stopped in stages is killed at once, without the rest of its
/// graces. Here the command's grandchild ignores SIGINT and SIGTERM, so
/// only the kill ends it, and the graces would take two seconds and more.
#[cfg(unix)]
#[test]
fn sys_a_second_ctrl_c_kills_running_commands_at_once() {
    let machine = Machine::new("sys_second_ctrl_c");
    let roll = Roll::new();
    let spec =
        ChildSpec::new(&roll, "parent").with_child(ChildSpec::new(&roll, "grandchild").stubborn());
    commanding(&machine, &spec);
    let running = Running::start(
        &machine,
        &[
            "-p",
            "run it",
            "--permission-mode",
            "unrestricted",
            "--output",
            "json",
        ],
    );
    let [parent, grandchild]: [_; 2] = roll
        .all(&["parent", "grandchild"], LIMIT)
        .try_into()
        .unwrap();

    running.tree.interrupt().unwrap();
    // The polite interrupt ends the command, and not its grandchild.
    assert!(parent.gone_within(LIMIT));
    assert!(grandchild.is_alive());

    running.tree.interrupt().unwrap();
    assert!(
        grandchild.gone_within(Duration::from_millis(1500)),
        "the second Ctrl-C killed what was left at once, not after the graces"
    );
    // The run went on to its end and wrote its record: the second Ctrl-C
    // did not end the process.
    let record = running.finish().expect_code(130).json();
    assert_eq!(record["outcome"], "interrupted");
    assert_eq!(record["tool_calls"], 1);
}

/// A harness that is killed outright leaves no command behind: the guard
/// kills the command's tree on Linux, and the Job Object does on Windows.
#[test]
fn sys_a_killed_harness_leaves_no_command_running() {
    let machine = Machine::new("sys_guard");
    let roll = Roll::new();
    let spec =
        ChildSpec::new(&roll, "parent").with_child(ChildSpec::new(&roll, "grandchild").stubborn());
    commanding(&machine, &spec);
    // Started as a plain process, not a tree: killing it must not kill
    // anything else by itself.
    let mut harness = machine
        .command(&["-p", "run it", "--permission-mode", "unrestricted"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let [parent, grandchild]: [_; 2] = roll
        .all(&["parent", "grandchild"], LIMIT)
        .try_into()
        .unwrap();

    harness.kill().unwrap();
    let _ = harness.wait();
    assert!(
        parent.gone_within(LIMIT),
        "the command died with the harness"
    );
    assert!(
        grandchild.gone_within(LIMIT),
        "the command's own child died with the harness"
    );
}

/// On Windows a program may be started with no console at all, by a
/// service or a scheduler. A run then has no console events to watch and
/// starts its commands with none: it still runs to its end.
#[cfg(windows)]
#[test]
fn sys_a_run_with_no_console_completes() {
    use std::os::windows::process::CommandExt as _;
    /// `DETACHED_PROCESS`: the new process has no console.
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    // The shell the binary picks itself, which starts with no console here.
    let machine = Machine::with_product_shell("sys_no_console");
    machine.reply_tool(
        "toolu_1",
        "shell",
        &json!({"command": "echo \"no console\""}),
    );
    machine.reply_text("Done.");
    let output = machine
        .command(&[
            "-p",
            "run it",
            "--permission-mode",
            "unrestricted",
            "--output",
            "json",
        ])
        .creation_flags(DETACHED_PROCESS)
        .output()
        .unwrap();
    let run = Run::of(output);
    let record: serde_json::Value = run.expect_code(0).json();
    assert_eq!(record["result"], "Done.");
    assert_eq!(record["tool_calls"], 1);
}
