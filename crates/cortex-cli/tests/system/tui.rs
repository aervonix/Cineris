//! The interactive application end to end: the real binary under a
//! pseudo-terminal (ConPTY on Windows), with the scripted model, typed at
//! as a person types (`docs/testing-and-ci.md`, "Testing the TUI").
//!
//! A pseudo-terminal's program takes its environment from the test process
//! on Windows (`docs/conpty.md`), and the binary must not see the
//! developer's own set-up. So each test starts this test binary in a role
//! of its own, a small shell (`cortex_testkit::role::ModeShell`), which
//! reads the machine's variables, the arguments and the workspace from a
//! file, runs the binary with them, and says afterwards whether the
//! terminal's modes are the ones it found: the restoration check of every
//! test.
//!
//! Each test names its phases (`cortex_testkit::Phases`), so that a slow
//! run on a hosted runner says where its time went.

use std::process::Command;
use std::time::{Duration, Instant};

use cortex_testkit::children::{ChildSpec, Roll};
use cortex_testkit::pty::Session;
use cortex_testkit::role::{ModeShell, NOT_RESTORED, RESTORED, Role, this_binary};
use cortex_testkit::{Gate, Phases, ServerReply};
use serde_json::{Value, json};

use super::{CORTEX, LEFT_OUT, Machine};

/// The role of the shell the binary runs under: its options are the path
/// of the file that says how.
pub const SHELL: Role = Role::new("cxcli-tui=");

/// A bound on every wait, which only a broken test reaches.
const LIMIT: Duration = Duration::from_secs(30);

/// The coarse check on the first frame, from the shell's start of the
/// binary: five times the runner's budget for the interface's first frame
/// (`docs/performance-budgets.md`), for a binary built without
/// optimisation. The budget itself is measured by `cargo xtask startup`.
const FIRST_FRAME: Duration = if cfg!(windows) {
    Duration::from_millis(600)
} else {
    Duration::from_millis(250)
};

/// What the shell says just before it starts the binary.
const STARTING: &str = "starting cortex";

/// What the empty input box says, once the first frame is drawn: idle,
/// or with a turn on its way (a prompt given on the command line).
const READY: [&str; 2] = ["Describe a task", "Type to steer this turn"];

/// In the shell's role: runs the binary as the file says, and reports on
/// the terminal. Returns when this process is not in that role.
pub fn child() {
    let Some(spec) = SHELL.options() else {
        return;
    };
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(&spec).expect("the spec")).expect("JSON");
    let mut command = Command::new(spec["program"].as_str().expect("a program"));
    command.current_dir(spec["cwd"].as_str().expect("a directory"));
    for argument in spec["args"].as_array().expect("arguments") {
        command.arg(argument.as_str().expect("an argument"));
    }
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if name.starts_with("CORTEX_") || LEFT_OUT.contains(&name.as_ref()) {
            command.env_remove(name.as_ref());
        }
    }
    for (name, value) in spec["env"].as_object().expect("variables") {
        command.env(name, value.as_str().expect("a value"));
    }
    ModeShell::new().announcing(STARTING).run(command);
}

impl Machine {
    /// The binary with `arguments`, in a pseudo-terminal of 100 by 30, in
    /// the workspace with the machine's environment.
    pub fn tui(&self, arguments: &[&str]) -> Session {
        let spec = self.home.join(format!(
            "tui-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|time| time.as_nanos())
                .unwrap_or(0)
        ));
        let env: serde_json::Map<String, Value> = self
            .vars()
            .iter()
            .map(|(name, value)| (name.clone(), json!(value.to_string_lossy())))
            .collect();
        std::fs::write(
            &spec,
            json!({
                "program": CORTEX,
                "cwd": self.workspace.to_string_lossy(),
                "args": arguments,
                "env": env,
            })
            .to_string(),
        )
        .expect("write the spec");
        Session::start(&this_binary(), &SHELL.arguments(&path(&spec)), 100, 30).answering()
    }
}

fn path(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Waits for the first frame, and checks it came in time.
fn first_frame(session: &mut Session, phases: &Phases) {
    phases.mark("the binary starts");
    let starting = session.wait_for(LIMIT, "the shell to start the binary", |t| {
        t.shows(STARTING)
    });
    let drawn = session.wait_for(LIMIT, "the first frame", |t| {
        READY.iter().any(|ready| t.shows(ready))
    });
    let took = drawn - starting;
    assert!(
        took < FIRST_FRAME,
        "the first frame took {took:?}, more than {FIRST_FRAME:?}"
    );
    // The session opens after it: its header follows.
    phases.mark("the session opens");
    session.wait_for(LIMIT, "the session's header", |t| t.shows("Cortex-Harness"));
}

/// Quits with `Ctrl+D` on the empty prompt, and checks that the terminal's
/// modes are as they were.
fn quit(session: &mut Session, phases: &Phases) {
    phases.mark("the binary ends");
    session.send(b"\x04");
    session.wait_for(LIMIT, "the shell's verdict", |t| {
        t.shows(RESTORED) || t.shows(NOT_RESTORED)
    });
    session.wait_for(LIMIT, "the whole verdict", |t| {
        (t.shows(RESTORED) && t.shows(")")) || t.shows("after:")
    });
    let screen = session.screen();
    assert!(
        screen.shows(RESTORED),
        "the modes were not put back:\n{}",
        screen.dump()
    );
    let modes = screen.modes();
    assert!(modes.cursor_visible, "the cursor is hidden");
    assert!(!modes.sync, "a synchronized update was left open");
    assert!(!modes.bracketed_paste, "bracketed paste was left on");
}

/// Types `text` as keys.
fn type_text(session: &mut Session, text: &str) {
    session.send(text.as_bytes());
}

/// Everything the screen and its scrollback show.
fn everything(session: &Session) -> String {
    session.screen().history().join("\n")
}

fn body(request: &cortex_testkit::server::ReceivedRequest) -> String {
    String::from_utf8_lossy(&request.body).into_owned()
}

fn wait_shows(session: &mut Session, what: &str) {
    session.wait_for(LIMIT, what, |t| {
        let all = t.history();
        all.iter().any(|row| row.contains(what))
    });
}

#[test]
fn sys_a_turn_with_tools_runs_in_the_interface_and_the_terminal_is_put_back() {
    let phases =
        Phases::new("sys_a_turn_with_tools_runs_in_the_interface_and_the_terminal_is_put_back");
    let machine = Machine::new("tui_turn");
    std::fs::write(
        machine.workspace.join("notes.txt"),
        "the build needs the json feature\n",
    )
    .unwrap();
    machine.reply_tool("toolu_1", "read", &json!({"path": "notes.txt"}));
    machine.reply_text("The build needs the json feature turned on.");
    let mut session = machine.tui(&[]);
    first_frame(&mut session, &phases);

    phases.mark("the turn");
    type_text(&mut session, "why does the build fail?\r");
    wait_shows(&mut session, "The build needs the json feature turned on.");
    let shown = everything(&session);
    assert!(shown.contains("why does the build fail?"), "{shown}");
    assert!(shown.contains("Read notes.txt"), "{shown}");
    // The model was sent the file it read.
    let requests = machine.server.requests();
    assert_eq!(requests.len(), 2, "{requests:#?}");
    assert!(body(&requests[1]).contains("the build needs the json feature"));

    quit(&mut session, &phases);
}

#[test]
fn sys_an_approval_is_asked_in_a_dialog_and_answered_with_a_key() {
    let phases = Phases::new("sys_an_approval_is_asked_in_a_dialog_and_answered_with_a_key");
    let machine = Machine::new("tui_approval");
    // The project's configuration directory is protected: a write there
    // asks at every level below `unrestricted`.
    let target = ".cortex/notes.md";
    machine.reply_tool(
        "toolu_1",
        "write",
        &json!({"path": target, "content": "remember the feature\n"}),
    );
    machine.reply_text("Noted.");
    // The prompt is on the command line. The dialog's answers are drawn at
    // the end of its grace, when they can be pressed.
    let mut session = machine.tui(&["note that the build needs the feature"]);
    first_frame(&mut session, &phases);

    phases.mark("the dialog");
    wait_shows(&mut session, "Allow once");
    let shown = everything(&session);
    assert!(shown.contains("notes.md"), "{shown}");
    assert!(
        !machine.workspace.join(target).exists(),
        "it ran before the answer"
    );
    session.send(b"y");
    wait_shows(&mut session, "Noted.");
    assert_eq!(
        std::fs::read_to_string(machine.workspace.join(target)).unwrap(),
        "remember the feature\n"
    );

    quit(&mut session, &phases);
}

#[test]
fn sys_a_turn_is_interrupted_mid_stream_and_mid_tool() {
    let phases = Phases::new("sys_a_turn_is_interrupted_mid_stream_and_mid_tool");
    let machine = Machine::new("tui_interrupt");
    // Mid-stream: part of an answer, then nothing until the client hangs up.
    let closed = Gate::new();
    machine.server.push(
        ServerReply::sse()
            .send(super::sse(&[
                super::message_start(),
                json!({"type": "content_block_start", "index": 0,
                    "content_block": {"type": "text", "text": ""}}),
                json!({"type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": "The first half of the answer\n"}}),
            ]))
            .hang_until_closed(&closed),
    );
    // Mid-tool: a command that says when it is up, and then waits until it
    // is stopped (the test kit's child, which the roll sees start and end).
    let roll = Roll::new();
    let command = ChildSpec::new(&roll, "command");
    machine.reply_tool(
        "toolu_1",
        "shell",
        &json!({"command": super::signals::shell_line(&command), "timeout_ms": 600_000}),
    );
    let mut session = machine.tui(&["--permission-mode", "unrestricted"]);
    first_frame(&mut session, &phases);

    phases.mark("interrupted mid-stream");
    type_text(&mut session, "explain\r");
    wait_shows(&mut session, "The first half of the answer");
    session.send(b"\x1b");
    assert!(closed.wait(LIMIT), "the request was not dropped");
    wait_shows(&mut session, "Interrupted.");
    // What had arrived stays.
    assert!(everything(&session).contains("The first half of the answer"));

    phases.mark("interrupted mid-tool");
    type_text(&mut session, "wait a minute\r");
    // The command is up: it is stopped mid-tool, not before it ran.
    let [running]: [_; 1] = roll
        .all(&["command"], LIMIT)
        .try_into()
        .unwrap_or_else(|_| panic!("the command did not start"));
    let began = Instant::now();
    session.send(b"\x1b");
    assert!(running.gone_within(LIMIT), "the command was not stopped");
    session.wait_for(LIMIT, "the second interruption", |t| {
        let all = t.history();
        all.iter()
            .filter(|row| row.contains("Interrupted."))
            .count()
            >= 2
    });
    assert!(
        began.elapsed() < Duration::from_secs(20),
        "the command was not stopped: {:?}",
        began.elapsed()
    );

    quit(&mut session, &phases);
}

#[test]
fn sys_a_steer_reaches_the_running_turn_and_a_queued_prompt_runs_after_it() {
    let phases =
        Phases::new("sys_a_steer_reaches_the_running_turn_and_a_queued_prompt_runs_after_it");
    let machine = Machine::new("tui_steer");
    std::fs::write(machine.workspace.join("notes.txt"), "notes\n").unwrap();
    // The first response waits until the steer and the queued prompt are in.
    let typed = Gate::new();
    machine
        .server
        .push(ServerReply::sse().hold_until(&typed).send(super::sse(&[
            super::message_start(),
            json!({"type": "content_block_start", "index": 0,
                    "content_block": {"type": "tool_use", "id": "toolu_1", "name": "read",
                        "input": {}}}),
            json!({"type": "content_block_delta", "index": 0,
                    "delta": {"type": "input_json_delta",
                        "partial_json": json!({"path": "notes.txt"}).to_string()}}),
            json!({"type": "content_block_stop", "index": 0}),
            super::message_delta("tool_use"),
            json!({"type": "message_stop"}),
        ])));
    machine.reply_text("Read them both.");
    machine.reply_text("Here is the summary.");
    let mut session = machine.tui(&[]);
    first_frame(&mut session, &phases);

    phases.mark("the turn runs");
    type_text(&mut session, "look at the notes\r");
    wait_shows(&mut session, "esc stops");
    // Steering: Enter while the turn runs.
    type_text(&mut session, "and the readme\r");
    // Queueing: Tab switches the pending message to "queue".
    type_text(&mut session, "then summarise\t\r");
    wait_shows(&mut session, "1 queued");
    typed.open();

    phases.mark("the steer and the queued turn");
    wait_shows(&mut session, "Here is the summary.");
    let requests = machine.server.requests();
    assert_eq!(requests.len(), 3, "{requests:#?}");
    assert!(
        body(&requests[1]).contains("and the readme"),
        "the steer did not reach the turn's next request"
    );
    assert!(
        !body(&requests[1]).contains("then summarise"),
        "the queued prompt came early"
    );
    assert!(body(&requests[2]).contains("then summarise"));

    quit(&mut session, &phases);
}
