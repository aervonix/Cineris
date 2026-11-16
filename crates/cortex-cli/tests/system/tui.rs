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

use super::{CORTEX, LEFT_OUT, MODEL, Machine};

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
        self.tui_as(std::path::Path::new(CORTEX), arguments)
    }

    /// As [`tui`](Self::tui), the binary at `program`: under another name.
    pub fn tui_as(&self, program: &std::path::Path, arguments: &[&str]) -> Session {
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
                "program": program.to_string_lossy(),
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

/// Waits for the trust question's answers, so that a key answers it.
fn trust_asked(session: &mut Session) {
    wait_shows(session, "Trust this project?");
    wait_shows(session, "x Never for this project");
}

/// A project whose file sets a model is asked about it when its session
/// opens; after `Trust it` the session runs on the project's model.
#[test]
fn sys_a_project_is_trusted_from_its_dialog_and_its_model_is_used() {
    let phases = Phases::new("sys_a_project_is_trusted_from_its_dialog_and_its_model_is_used");
    let machine = Machine::new("tui_trust");
    machine.project_config("[model]\nmain = \"claude-opus-5-5\"\n");
    let mut session = machine.tui(&[]);
    first_frame_before_trust(&mut session, &phases);

    phases.mark("the trust question");
    trust_asked(&mut session);
    let shown = everything(&session);
    assert!(shown.contains("model.main"), "{shown}");
    assert!(shown.contains("claude-opus-5-5"), "{shown}");
    // The folder as a person reads it, never the canonical form.
    assert!(!shown.contains(r"\\?\"), "{shown}");
    session.send(b"y");
    // The header's box (`│`, or `|` in the ASCII glyphs a Windows console
    // gets) names the model the session runs on (the dialog
    // named it too, outside any box).
    session.wait_for(LIMIT, "the header with the project's model", |t| {
        let all = t.history();
        all.iter()
            .any(|row| row.trim_start().starts_with(['│', '|']) && row.contains("claude-opus-5-5"))
    });

    quit(&mut session, &phases);
}

/// The owner's decision of 2026-11-16: an untrusted project's first limit
/// waits for trust. The question lists it, and after `Trust it` it is in
/// force: a limit of nothing stops the turn at once.
#[test]
fn sys_a_projects_first_limit_is_listed_and_in_force_once_trusted() {
    let phases = Phases::new("sys_a_projects_first_limit_is_listed_and_in_force_once_trusted");
    let machine = Machine::new("tui_first_limit");
    machine.project_config("[limits.turn]\ntime = \"0s\"\n");
    machine.reply_text("Not sent.");
    let mut session = machine.tui(&[]);
    first_frame_before_trust(&mut session, &phases);

    phases.mark("the trust question");
    trust_asked(&mut session);
    let shown = everything(&session);
    assert!(shown.contains("limits.turn.time"), "{shown}");
    assert!(shown.contains("first limit"), "{shown}");
    session.send(b"y");
    session.wait_for(LIMIT, "the header", |t| t.shows("Cortex-Harness"));

    phases.mark("the turn");
    type_text(&mut session, "go\r");
    wait_shows(&mut session, "time limit");
    assert!(
        machine.server.requests().is_empty(),
        "a request was sent past a limit of nothing"
    );

    quit(&mut session, &phases);
}

/// The first frame, for a project that is asked about: its header comes
/// only once the question is answered. The question may be the first frame
/// seen: a pseudo-console draws the screen as it is when it next looks,
/// and the dialog takes the input box's place a moment after the first
/// frame ([`docs/conpty.md`](../../../../docs/conpty.md)).
fn first_frame_before_trust(session: &mut Session, phases: &Phases) {
    phases.mark("the binary starts");
    let starting = session.wait_for(LIMIT, "the shell to start the binary", |t| {
        t.shows(STARTING)
    });
    let drawn = session.wait_for(LIMIT, "the first frame", |t| {
        READY.iter().any(|ready| t.shows(ready)) || t.shows("Trust this project?")
    });
    let took = drawn - starting;
    assert!(
        took < FIRST_FRAME,
        "the first frame took {took:?}, more than {FIRST_FRAME:?}"
    );
}

/// The owner's test of "never for this project" (2026-11-15): a never
/// survives a restart, which neither asks nor trusts and says why;
/// `permissions list` names it, `permissions trust` clears it, and
/// `permissions forget` makes the next start ask again.
#[test]
fn sys_a_never_survives_a_restart_and_permissions_trust_clears_it() {
    let phases = Phases::new("sys_a_never_survives_a_restart_and_permissions_trust_clears_it");
    let machine = Machine::new("tui_never");
    machine.project_config("[model]\nmain = \"claude-opus-5-5\"\n");
    let header_with = |session: &mut Session, model: &str| {
        session.wait_for(LIMIT, "the header's model", |t| {
            let all = t.history();
            all.iter()
                .any(|row| row.trim_start().starts_with(['│', '|']) && row.contains(model))
        });
    };

    let mut session = machine.tui(&[]);
    first_frame_before_trust(&mut session, &phases);
    phases.mark("never");
    trust_asked(&mut session);
    session.send(b"x");
    header_with(&mut session, MODEL);
    quit(&mut session, &phases);

    phases.mark("a restart");
    let mut session = machine.tui(&[]);
    first_frame(&mut session, &phases);
    header_with(&mut session, MODEL);
    wait_shows(&mut session, "You said never to trust this project");
    assert!(
        !everything(&session).contains("Trust this project?"),
        "a project marked never was asked about"
    );
    quit(&mut session, &phases);

    phases.mark("the commands");
    let listed = machine.run(&["permissions", "list"]);
    listed.expect_code(0);
    assert!(listed.stdout.contains("never trusted"), "{}", listed.stdout);
    let trusted = machine.run(&["permissions", "trust"]);
    trusted.expect_code(0);
    assert!(
        trusted.stdout.contains("which you had said never to trust"),
        "{}",
        trusted.stdout
    );
    let mut session = machine.tui(&[]);
    first_frame(&mut session, &phases);
    header_with(&mut session, "claude-opus-5-5");
    quit(&mut session, &phases);

    let forgot = machine.run(&["permissions", "forget"]);
    forgot.expect_code(0);
    assert!(forgot.stdout.contains("asks again"), "{}", forgot.stdout);
    let listed = machine.run(&["permissions", "list"]);
    assert!(listed.stdout.contains("not trusted"), "{}", listed.stdout);
}

/// Types a slash command and sends it: the command menu takes the first
/// `Enter`, the second sends. (`Esc` to close the menu would come with the
/// `Enter` in one read, which is `Alt+Enter`, a new line.)
fn command(session: &mut Session, line: &str) {
    type_text(session, line);
    session.send(b"\r\r");
}

/// A session is resumed with its history drawn, and its next turn has the
/// whole conversation.
#[test]
fn sys_a_session_is_resumed_with_its_history_drawn() {
    let phases = Phases::new("sys_a_session_is_resumed_with_its_history_drawn");
    let machine = Machine::new("tui_resume");
    machine.reply_text("The first answer.");
    machine.reply_text("The second answer.");
    let mut session = machine.tui(&["the first question"]);
    first_frame(&mut session, &phases);
    wait_shows(&mut session, "The first answer.");
    quit(&mut session, &phases);

    phases.mark("resumed");
    let mut session = machine.tui(&["--continue"]);
    first_frame(&mut session, &phases);
    // Drawn from the session's history, on a screen that never showed it.
    wait_shows(&mut session, "the first question");
    wait_shows(&mut session, "The first answer.");
    type_text(&mut session, "the second question\r");
    wait_shows(&mut session, "The second answer.");
    let requests = machine.server.requests();
    assert_eq!(requests.len(), 2, "{requests:#?}");
    let second = body(&requests[1]);
    assert!(
        second.contains("the first question") && second.contains("The first answer."),
        "{second}"
    );
    quit(&mut session, &phases);
}

/// `/undo` puts back what the last turn changed. A file the person edited
/// since is asked about first, and "Put them back" overwrites it.
#[test]
fn sys_undo_asks_about_a_file_edited_since_and_puts_it_back() {
    let phases = Phases::new("sys_undo_asks_about_a_file_edited_since_and_puts_it_back");
    let machine = Machine::new("tui_undo");
    let notes = machine.workspace.join("notes.txt");
    machine.reply_tool(
        "toolu_1",
        "write",
        &json!({"path": "notes.txt", "content": "after\n"}),
    );
    machine.reply_text("Written.");
    let mut session = machine.tui(&["--allow", "write notes.txt", "change the notes"]);
    first_frame(&mut session, &phases);
    wait_shows(&mut session, "Written.");
    // The turn made the file.
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "after\n");

    phases.mark("undo");
    // The person changes the file after the turn.
    std::fs::write(&notes, "after, and mine\n").unwrap();
    command(&mut session, "/undo");
    wait_shows(&mut session, "notes.txt");
    wait_shows(&mut session, "1 Put them back");
    session.send(b"1");
    wait_shows(&mut session, "The last turn is undone");
    assert!(
        !notes.exists(),
        "the file the undone turn made is still there"
    );

    quit(&mut session, &phases);
}

/// Start to first frame of the real application, under both its names,
/// `CORTEX_PERF_RUNS` runs of each (30 unless given): from the moment the
/// shell starts the binary to the moment the first frame is on the
/// screen, read through the pseudo-terminal. The budget
/// (`docs/performance-budgets.md`) is 20 ms at the desk, 50 ms on a Linux
/// runner and 120 ms on a Windows one, in a release build. A measurement:
///
/// ```text
/// gh workflow run try --ref <branch> -f os=windows -f command="perf -p cortex-cli"
/// cargo test --release -p cortex-cli --test system perf_start -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, run in a release build: see its documentation"]
fn perf_start_to_first_frame_under_both_names() {
    let runs: usize = std::env::var("CORTEX_PERF_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    let machine = Machine::new("perf_first_frame");
    let alias = crate::names::install_alias(&machine);
    for (name, program) in [("cortex", std::path::PathBuf::from(CORTEX)), ("cx", alias)] {
        let mut times: Vec<f64> = Vec::with_capacity(runs);
        for _ in 0..runs {
            let mut session = machine.tui_as(&program, &[]);
            let starting = session.wait_for(LIMIT, "the shell to start the binary", |t| {
                t.shows(STARTING)
            });
            let drawn = session.wait_for(LIMIT, "the first frame", |t| {
                READY.iter().any(|ready| t.shows(ready))
            });
            times.push((drawn - starting).as_secs_f64() * 1000.0);
            session.send(b"\x04");
            session.wait_exit(LIMIT);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "start to first frame as {name:<6} {runs} runs: median {:>7.2} ms   p95 {:>7.2} ms   \
             best {:>7.2} ms",
            times[times.len() / 2],
            times[(times.len() * 95 / 100).min(times.len() - 1)],
            times[0]
        );
    }
}

/// The resident memory of the process that the role shell started for
/// `session`, in KiB: Linux only, read from `/proc`.
/// Whether the prompt's box shows `text` typed: after `◆`, or after `>`
/// where the console gets the ASCII glyphs (Windows).
fn typed(screen: &cortex_testkit::VTerm, text: &str) -> bool {
    screen.shows(&format!("◆ {text}")) || screen.shows(&format!("> {text}"))
}

/// A field of a process's `/proc` status: Linux only.
#[cfg(target_os = "linux")]
fn status_field(status: &str, name: &str) -> Option<String> {
    status
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .map(|value| value.trim().trim_end_matches("kB").trim().to_owned())
}

/// The status of the process the role shell started for `session`, the
/// binary: the process whose parent is the shell. Linux only.
#[cfg(target_os = "linux")]
fn child_status(session: &Session) -> Option<String> {
    let shell = session.pid().to_string();
    std::fs::read_dir("/proc")
        .ok()?
        .flatten()
        .find_map(|entry| {
            let status = std::fs::read_to_string(entry.path().join("status")).ok()?;
            (status_field(&status, "PPid:")? == shell).then_some(status)
        })
}

/// The resident memory of the binary, in KiB. Linux only.
#[cfg(target_os = "linux")]
fn resident_kib(session: &Session) -> Option<u64> {
    status_field(&child_status(session)?, "VmRSS:")?
        .parse()
        .ok()
}

/// Each of SIGTERM, SIGHUP, SIGINT and SIGQUIT ends the application
/// gracefully (`docs/steps/23/answered.md`, "Signals, a failed write and a
/// panic"): the terminal is put back at once, the running turn is stopped,
/// the core ends, and the process exits with the code the signal's own end
/// would have given.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_signal_ends_the_application_gracefully_with_its_code() {
    for (signal, code) in [("TERM", 143), ("HUP", 129), ("INT", 130), ("QUIT", 131)] {
        let machine = Machine::new(&format!("tui_signal_{signal}"));
        // A turn that runs until its request is dropped.
        let closed = Gate::new();
        machine.server.push(
            ServerReply::sse()
                .send(super::sse(&[
                    super::message_start(),
                    json!({"type": "content_block_start", "index": 0,
                        "content_block": {"type": "text", "text": ""}}),
                    json!({"type": "content_block_delta", "index": 0,
                        "delta": {"type": "text_delta", "text": "Working on it\n"}}),
                ]))
                .hang_until_closed(&closed),
        );
        let mut session = machine.tui(&["work"]);
        wait_shows(&mut session, "Working on it");
        let pid = child_status(&session)
            .and_then(|status| status_field(&status, "Pid:"))
            .expect("the binary's process");
        let sent = Command::new("kill")
            .args([format!("-{signal}"), pid])
            .status()
            .expect("kill");
        assert!(sent.success(), "SIG{signal} was not sent");
        session.wait_for(LIMIT, "the shell's verdict", |t| {
            t.shows(RESTORED) || t.shows(NOT_RESTORED)
        });
        let shown = everything(&session);
        assert!(
            shown.contains(&format!("{RESTORED} (exit status: {code})")),
            "SIG{signal}: {shown}"
        );
        assert!(closed.wait(LIMIT), "SIG{signal}: the turn was not stopped");
    }
}

#[expect(clippy::print_stdout, reason = "a measurement prints what it measured")]
fn percentiles(name: &str, mut times: Vec<f64>) {
    times.sort_by(f64::total_cmp);
    let at = |share: usize| times[(times.len() * share / 100).min(times.len() - 1)];
    println!(
        "{name:<34} {} samples: median {:>7.2} ms   p95 {:>7.2} ms   p99 {:>7.2} ms   most {:>7.2} ms",
        times.len(),
        at(50),
        at(95),
        at(99),
        times[times.len() - 1]
    );
}

/// The application idle with a new session: its resident memory (Linux),
/// and a key's time to its frame, `CORTEX_PERF_RUNS` keys (200 unless
/// given), a character typed and taken back by turns, each timed from its
/// write to the frame that shows it. Budgets: 25 MB resident; 2 ms at the
/// 99th percentile for a key (`docs/performance-budgets.md`). A
/// measurement, in a release build: see
/// [`perf_start_to_first_frame_under_both_names`].
#[test]
#[ignore = "a measurement, run in a release build: see its documentation"]
fn perf_idle_memory_and_a_keys_time_to_its_frame() {
    let keys: usize = std::env::var("CORTEX_PERF_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);
    let machine = Machine::new("perf_idle");
    let mut session = machine.tui(&[]);
    session.wait_for(LIMIT, "the first frame", |t| {
        READY.iter().any(|ready| t.shows(ready))
    });
    session.wait_for(LIMIT, "the session's header", |t| t.shows("Cortex-Harness"));
    #[cfg(target_os = "linux")]
    match resident_kib(&session) {
        Some(kib) => println!(
            "idle with a new session: {:.1} MB resident",
            kib as f64 / 1024.0
        ),
        None => println!("idle with a new session: the resident memory could not be read"),
    }
    #[cfg(not(target_os = "linux"))]
    println!("idle with a new session: resident memory is read on Linux only");
    let mut times = Vec::with_capacity(keys);
    for n in 0..keys {
        let started = Instant::now();
        if n % 2 == 0 {
            session.send(b"x");
            session.wait_for(LIMIT, "the key's echo", |t| typed(t, "x"));
        } else {
            session.send(b"\x7f");
            session.wait_for(LIMIT, "the key taken back", |t| {
                READY.iter().any(|ready| t.shows(ready))
            });
        }
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    percentiles("a key to its frame, idle", times);
    session.send(b"\x04");
    session.wait_exit(LIMIT);
}

/// A streamed answer's piece to the frame that shows it: the scripted
/// server holds each piece until the test lets it go, and the time is from
/// letting it go to the screen showing it (the budget: 20 ms at the 95th
/// percentile; the frame cap of 60 a second is most of it). Keys typed
/// while the turn runs are timed to their frames too. A measurement, in a
/// release build: see [`perf_start_to_first_frame_under_both_names`].
#[test]
#[ignore = "a measurement, run in a release build: see its documentation"]
fn perf_a_streamed_piece_and_a_key_while_streaming() {
    const PIECES: usize = 40;
    let machine = Machine::new("perf_stream");
    let gates: Vec<Gate> = (0..PIECES).map(|_| Gate::new()).collect();
    let mut reply = ServerReply::sse().send(super::sse(&[
        super::message_start(),
        json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
    ]));
    for (n, gate) in gates.iter().enumerate() {
        reply = reply.wait(gate).send(super::sse(&[json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": format!("piece{n:03} ")}
        })]));
    }
    reply = reply.send(super::sse(&[
        json!({"type": "content_block_stop", "index": 0}),
        super::message_delta("end_turn"),
        json!({"type": "message_stop"}),
    ]));
    machine.server.push(reply);
    let mut session = machine.tui(&["go"]);
    session.wait_for(LIMIT, "the first frame", |t| {
        READY.iter().any(|ready| t.shows(ready))
    });
    let mut pieces = Vec::with_capacity(PIECES);
    let mut keys = Vec::with_capacity(PIECES);
    for (n, gate) in gates.iter().enumerate() {
        let started = Instant::now();
        gate.open();
        let text = format!("piece{n:03}");
        session.wait_for(LIMIT, "the piece", |t| {
            t.history().iter().any(|row| row.contains(&text))
        });
        pieces.push(started.elapsed().as_secs_f64() * 1000.0);
        // A key while the answer streams: typed, then taken back.
        let started = Instant::now();
        session.send(b"q");
        session.wait_for(LIMIT, "the key's echo", |t| typed(t, "q"));
        keys.push(started.elapsed().as_secs_f64() * 1000.0);
        session.send(b"\x7f");
        session.wait_for(LIMIT, "the key taken back", |t| !typed(t, "q"));
    }
    percentiles("a streamed piece to its frame", pieces);
    percentiles("a key to its frame, streaming", keys);
    session.send(b"\x04");
    session.wait_exit(LIMIT);
}
