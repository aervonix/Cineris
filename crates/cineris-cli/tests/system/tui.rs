//! The interactive application end to end: the real binary under a
//! pseudo-terminal (ConPTY on Windows), with the scripted model, typed at
//! as a person types (`docs/testing-and-ci.md`, "Testing the TUI").
//!
//! A pseudo-terminal's program takes its environment from the test process
//! on Windows (`docs/conpty.md`), and the binary must not see the
//! developer's own set-up. So each test starts this test binary in a role
//! of its own, a small shell (`cineris_testkit::role::ModeShell`), which
//! reads the machine's variables, the arguments and the workspace from a
//! file, runs the binary with them, and says afterwards whether the
//! terminal's modes are the ones it found: the restoration check of every
//! test.
//!
//! Each test names its phases (`cineris_testkit::Phases`), so that a slow
//! run on a hosted runner says where its time went.

use std::process::Command;
use std::time::{Duration, Instant};

use cineris_testkit::VTerm;
use cineris_testkit::children::{ChildSpec, Roll};
use cineris_testkit::exit::{Exit, Verdict};
use cineris_testkit::glyphs::{GlyphSets, RowPattern};
use cineris_testkit::pty::Session;
use cineris_testkit::role::{ModeShell, NOT_RESTORED, RESTORED, Role, this_binary};
use cineris_testkit::{Gate, Phases, ServerReply};
use cineris_tui::glyphs::Glyphs;
use serde_json::{Value, json};

use super::{CINERIS, LEFT_OUT, MODEL, Machine};

/// The glyph sets the interface draws in: a test spells a glyph once, as the
/// product's tables have it, and the screen may show either set (a Windows
/// console gets the ASCII one).
const GLYPHS: GlyphSets<'static, Glyphs> = GlyphSets::new(&[Glyphs::UNICODE, Glyphs::ASCII]);

/// The role of the shell the binary runs under: its options are the path
/// of the file that says how.
pub const SHELL: Role = Role::new("cincli-tui=");

/// A bound on every wait, which only a broken test reaches.
const LIMIT: Duration = Duration::from_secs(30);

/// The coarse check on the first frame, from the shell's start of the
/// binary: five times the runner's budget for the interface's first frame
/// (`docs/performance-budgets.md`), for a binary built without
/// optimisation. The budget itself is measured by `cargo xtask startup`.
///
/// The binary has been started once before (`--version`, by the shell), as
/// the budget's runs have: a runner's first start of an executable it has
/// never run (its scan by the system's antivirus, its pages read from disk)
/// is the runner's cost, not the interface's, and took a cold Windows
/// runner's first frame to 790 ms (try run 43883744404).
const FIRST_FRAME: Duration = if cfg!(windows) {
    Duration::from_millis(600)
} else {
    Duration::from_millis(250)
};

/// The header's title: the name and the version. Never the name alone, which
/// a path on the screen can hold (a copy of the repository is named after it,
/// and the trust question shows the workspace's path), and which would let a
/// wait for the header pass before there is one.
const HEADER: &str = concat!("Cineris ", env!("CARGO_PKG_VERSION"));

/// The coarse check on a key's echo in the real binary, from the key's
/// write to the frame that shows it: far over the budget of 2 ms at the
/// 99th percentile (`docs/performance-budgets.md`), for a binary built
/// without optimisation on a loaded runner, and under ConPTY's own 16 ms or
/// so on Windows; an interface thread that waits on the disk or the core
/// passes it by seconds.
const KEY_ECHO: Duration = if cfg!(windows) {
    Duration::from_millis(500)
} else {
    Duration::from_millis(250)
};

/// What the shell says just before it starts the binary.
const STARTING: &str = "starting cineris";

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
    let program = spec["program"].as_str().expect("a program");
    // Started once first, to no terminal: what is measured is a start of a
    // binary the system has run before ([`FIRST_FRAME`]).
    let warmed = Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("start the binary once");
    assert!(warmed.success(), "`--version` failed: {warmed}");
    let mut command = Command::new(program);
    command.current_dir(spec["cwd"].as_str().expect("a directory"));
    for argument in spec["args"].as_array().expect("arguments") {
        command.arg(argument.as_str().expect("an argument"));
    }
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        // The test kit's record of process starts goes on to the binary,
        // whose own starts are the test's.
        if (name.starts_with("CINERIS_") && name != cineris_base::hook::STARTS)
            || LEFT_OUT.contains(&name.as_ref())
        {
            command.env_remove(name.as_ref());
        }
    }
    for (name, value) in spec["env"].as_object().expect("variables") {
        command.env(name, value.as_str().expect("a value"));
    }
    cineris_base::hook::started(command.get_program());
    // It lives through a hang-up on Unix, to say how the binary ended after
    // the console was closed under it.
    ModeShell::new()
        .surviving_hangup()
        .announcing(STARTING)
        .run(command);
}

impl Machine {
    /// The binary with `arguments`, in a pseudo-terminal of 100 by 30, in
    /// the workspace with the machine's environment.
    pub fn tui(&self, arguments: &[&str]) -> Session {
        self.tui_as(std::path::Path::new(CINERIS), arguments)
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
    // The full screen takes the alternate screen at once: the shell's line
    // may come in the same read, under it, or (through ConPTY, which paints
    // on its own clock) never be painted at all, the switch marking the
    // start instead. Then the time to the first frame below is from the
    // switch, near nothing, and checks little: the start-up budget is
    // `cargo xtask startup`'s, and the inline tests keep this check.
    let starting = session.wait_for(LIMIT, "the shell to start the binary", |t| {
        t.shows(STARTING) || t.shows_under_the_alternate(STARTING) || t.modes().alternate
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
    session.wait_for(LIMIT, "the session's header", |t| t.shows(HEADER));
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

/// Types `word` into the empty prompt a key at a time, each when the last
/// has shown, and checks that each showed within [`KEY_ECHO`]. No space in
/// it: a row is padded with spaces, so a space typed would seem shown
/// before it was.
fn type_timed(session: &mut Session, word: &str) {
    assert!(!word.contains(' '), "{word:?}");
    let mut worst = Duration::ZERO;
    let mut shown_so_far = String::new();
    for c in word.chars() {
        shown_so_far.push(c);
        let sent = session.send(c.to_string().as_bytes());
        let shown = session.wait_for(LIMIT, "the key's echo", |t| typed(t, &shown_so_far));
        worst = worst.max(shown.saturating_duration_since(sent));
    }
    assert!(
        worst < KEY_ECHO,
        "a key took {worst:?} to show, more than {KEY_ECHO:?}"
    );
}

/// Everything the screen and its scrollback show.
fn everything(session: &Session) -> String {
    session.screen().history().join("\n")
}

fn body(request: &cineris_testkit::server::ReceivedRequest) -> String {
    String::from_utf8_lossy(&request.body).into_owned()
}

/// Waits until the turn has ended and the input box is idle again. Its
/// last answer is drawn before it ends, and a command that needs no turn
/// to be running (`/undo`) is refused until then: "A turn is running".
fn turn_ended(session: &mut Session) {
    session.wait_for(LIMIT, "the turn's end", |t| t.shows(READY[0]));
}

fn wait_shows(session: &mut Session, what: &str) {
    session.wait_for(LIMIT, what, |t| shows_ever(t, what));
}

/// Whether the screen or its scrollback holds `what`.
fn shows_ever(t: &VTerm, what: &str) -> bool {
    t.history().iter().any(|row| row.contains(what))
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

    phases.mark("the keys");
    type_timed(&mut session, "why");
    phases.mark("the turn");
    type_text(&mut session, " does the build fail?\r");
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

/// The full-screen presenter in the real binary (step 25): `--fullscreen`
/// draws on the alternate screen with the mouse reported (on Unix, in a
/// terminal without the win32 input mode; on Windows the mouse is not
/// asked for, and `PageUp` moves the view), runs a turn, scrolls up with
/// the wheel sent as the terminal sends it, re-wraps on a resize,
/// and quitting leaves the terminal as it was: the main screen as the shell
/// left it, the mouse and the alternate screen off, and the modes the
/// shell had.
#[test]
fn sys_the_full_screen_runs_a_turn_scrolls_and_leaves_the_terminal_as_it_was() {
    let phases =
        Phases::new("sys_the_full_screen_runs_a_turn_scrolls_and_leaves_the_terminal_as_it_was");
    let machine = Machine::new("tui_full_screen");
    let answer: String = (1..=60)
        .map(|n| format!("Line {n} of a long answer.\n\n"))
        .collect();
    // A file written first: its box holds the diff a click closes.
    machine.reply_tool(
        "toolu_1",
        "write",
        &json!({"path": "notes.txt", "content": "kept in the box\n"}),
    );
    machine.reply_text(&answer);
    // On Unix, a terminal like xterm, which does not know the win32
    // input mode: the one where the mouse is asked for by its sequences.
    let mut session = machine.tui(&["--fullscreen"]);
    if cfg!(unix) {
        session = session.without_win32_input();
    }
    first_frame(&mut session, &phases);
    if cfg!(unix) {
        // Through ConPTY the modes are the console's, not these.
        let modes = session.screen().modes();
        assert!(modes.alternate, "on the alternate screen");
        assert!(modes.mouse && modes.mouse_sgr, "the mouse is reported");
    } else {
        // Nothing asks for the mouse through a pseudo-console, in any form.
        let written: Vec<u8> = session
            .recording()
            .iter()
            .flat_map(|chunk| chunk.bytes.iter().copied())
            .collect();
        let set = cineris_testkit::vterm::modes_set(&written);
        assert!(
            !set.iter()
                .any(|mode| cineris_testkit::vterm::MOUSE_MODES.contains(mode)),
            "the mouse was asked for: every mode set was {set:?}"
        );
    }

    phases.mark("the turn");
    type_text(&mut session, "tell me a long story\r");
    session.wait_for(LIMIT, "the answer's end", |t| {
        t.shows("Line 60 of a long answer.")
    });

    phases.mark("the wheel");
    // Three notches of the wheel up, over the transcript. Under a
    // pseudo-console the terminal is not asked for the mouse (cineris-term's
    // `Capabilities::vt_mouse`): there `PageUp` moves the view, sent as
    // Windows Terminal sends it.
    if cfg!(windows) {
        session.send_key("pageup");
    } else {
        session.send(b"\x1b[<64;10;5M\x1b[<64;10;5M\x1b[<64;10;5M");
    }
    session.wait_for(LIMIT, "the view moved up", |t| t.shows("more below"));
    assert!(!session.screen().shows("Line 60 of a long answer."));

    phases.mark("the resize");
    session.resize(60, 30);
    session.wait_for(LIMIT, "drawn again at the new width", |t| {
        t.screen()
            .iter()
            .all(|row| row.trim_end().chars().count() <= 59)
            && t.shows("more below")
    });

    phases.mark("the click");
    // Up to the top, where the write's box is. The box's row is read only
    // once every `PageUp` has been taken: through ConPTY each comes as a
    // record of its own with frames between, and a row read from a frame
    // before the top would be moved by the next (the second review of step
    // 25). At the top the header card's model row is the window's fourth.
    for _ in 0..12 {
        session.send_key("pageup");
    }
    session.wait_for(LIMIT, "the top, and the write's box", |t| {
        t.screen()
            .get(3)
            .is_some_and(|row| row.contains(crate::MODEL))
            && t.shows("notes.txt")
    });
    // A file's diff is shown open; a click on the box's row closes it.
    assert!(session.screen().shows("kept in the box"), "open at first");
    let row = session
        .screen()
        .screen()
        .iter()
        .position(|row| row.contains("notes.txt"))
        .expect("the box's row");
    // A click, where the terminal reports the mouse: not under a
    // pseudo-console, where a kept box is opened from the viewer.
    if cfg!(unix) {
        session.send(format!("\x1b[<0;5;{0}M\x1b[<0;5;{0}m", row + 1).as_bytes());
        session.wait_for(LIMIT, "the box closed", |t| !t.shows("kept in the box"));
    }

    phases.mark("back to the end");
    for _ in 0..24 {
        session.send_key("pagedown");
    }
    session.wait_for(LIMIT, "the end again", |t| {
        t.shows("Line 60 of a long answer.") && !t.shows("more below")
    });

    quit(&mut session, &phases);
    let screen = session.screen();
    if cfg!(unix) {
        let modes = screen.modes();
        assert!(!modes.alternate, "the alternate screen was left");
        assert!(!modes.mouse && !modes.mouse_sgr, "the mouse was turned off");
    }
    // The main screen holds nothing of the session: it is as the shell
    // left it.
    assert!(
        !screen
            .history()
            .iter()
            .any(|row| row.contains("long answer")),
        "{}",
        screen.dump()
    );
}

#[test]
fn sys_an_approval_is_asked_in_a_dialog_and_answered_with_a_key() {
    let phases = Phases::new("sys_an_approval_is_asked_in_a_dialog_and_answered_with_a_key");
    let machine = Machine::new("tui_approval");
    // The project's configuration directory is protected: a write there
    // asks at every level below `unrestricted`.
    let target = ".cineris/notes.md";
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
    session.answer_dialog(LIMIT, "Allow once", b"y", |t| shows_ever(t, "Noted."));
    assert_eq!(
        std::fs::read_to_string(machine.workspace.join(target)).unwrap(),
        "remember the feature\n"
    );

    quit(&mut session, &phases);
}

#[test]
fn sys_a_turn_is_interrupted_mid_stream_and_mid_tool() {
    let phases = Phases::new("sys_a_turn_is_interrupted_mid_stream_and_mid_tool");
    // The shell the binary picks itself, where every other test of this
    // binary has Windows PowerShell: this one keeps the product's own pick
    // covered, from a key to a command it stops.
    let machine = Machine::with_product_shell("tui_interrupt");
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

/// The trust question's row of answers, which it draws once it takes keys.
const TRUST_ANSWERS: &str = "x Never for this project";

/// Waits for the trust question's answers, so that a key answers it.
fn trust_asked(session: &mut Session) {
    wait_shows(session, "Trust this project?");
    wait_shows(session, TRUST_ANSWERS);
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
    // The header's box names the model the session runs on (the dialog
    // named it too, outside any box).
    session.answer_dialog(LIMIT, TRUST_ANSWERS, b"y", |t| {
        GLYPHS.history_has_row(t, |g| {
            RowPattern::starting_with(g.vertical).containing("claude-opus-5-5")
        })
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
    session.answer_dialog(LIMIT, TRUST_ANSWERS, b"y", |t| t.shows(HEADER));

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
            GLYPHS.history_has_row(t, |g| {
                RowPattern::starting_with(g.vertical).containing(model)
            })
        });
    };

    let mut session = machine.tui(&[]);
    first_frame_before_trust(&mut session, &phases);
    phases.mark("never");
    trust_asked(&mut session);
    session.answer_dialog(LIMIT, TRUST_ANSWERS, b"x", |t| shows_ever(t, MODEL));
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
    turn_ended(&mut session);
    // The turn made the file.
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "after\n");

    phases.mark("undo");
    // The person changes the file after the turn.
    std::fs::write(&notes, "after, and mine\n").unwrap();
    command(&mut session, "/undo");
    // The question names the file (the turn's tool line named it before,
    // so the screen is read once the question is up).
    session.wait_for(LIMIT, "the undo's question", |t| t.shows("1 Put them back"));
    let screen = session.screen().screen().join("\n");
    assert!(screen.contains("notes.txt"), "{screen}");
    session.answer_dialog(LIMIT, "1 Put them back", b"1", |t| {
        shows_ever(t, "The last turn is undone")
    });
    assert!(
        !notes.exists(),
        "the file the undone turn made is still there"
    );

    quit(&mut session, &phases);
}

/// How a test ends the application while a question holds the core's lane.
/// `/quit` is not among them: a dialog that takes keys types nothing into
/// the editor, so it cannot be given while one is open.
#[derive(Debug, Clone, Copy)]
enum Ending {
    /// `Ctrl+C`, which arms the quit, then `Ctrl+C` again.
    TwoInterrupts,
    /// A signal by its name, to the binary alone (Linux): SIGTERM, and
    /// SIGHUP, which a terminal that closes sends.
    #[cfg(target_os = "linux")]
    Signal(&'static str),
}

impl Ending {
    /// What the platform can send.
    fn all() -> Vec<Self> {
        vec![
            Self::TwoInterrupts,
            #[cfg(target_os = "linux")]
            Self::Signal("TERM"),
            #[cfg(target_os = "linux")]
            Self::Signal("HUP"),
        ]
    }

    /// The exit code the binary ends with.
    fn code(self) -> i32 {
        match self {
            Self::TwoInterrupts => 0,
            #[cfg(target_os = "linux")]
            Self::Signal("TERM") => 143,
            #[cfg(target_os = "linux")]
            Self::Signal(_) => 129,
        }
    }
}

/// How long ending may take with a question open in the lane: well inside
/// the bound of the core's own shutdown (2 s), and under the 1 s the end
/// gives the lane's last operation, so that an end that waited for it
/// fails. It takes a few milliseconds at the desk. Before step 23 moved the
/// end out of the lane, a question nobody would answer held the core's
/// thread until the binary gave up on it (5 s, or 2 s after a signal).
const ENDING: Duration = Duration::from_secs(1);

/// Ends the application in `session` by `ending`, while a question waits
/// in the core's lane, and checks that it ends within [`ENDING`] with the
/// terminal put back and the code it should have.
fn ends_while_the_lane_waits(session: &mut Session, ending: Ending, waiting: &str) {
    let started = Instant::now();
    match ending {
        Ending::TwoInterrupts => {
            session.send(b"\x03");
            session.send(b"\x03");
        }
        #[cfg(target_os = "linux")]
        Ending::Signal(name) => {
            let pid = child_status(session)
                .and_then(|status| status_field(&status, "Pid:"))
                .expect("the binary's process");
            let sent = Command::new("kill")
                .args([format!("-{name}"), pid])
                .status()
                .expect("kill");
            assert!(sent.success(), "SIG{name} was not sent");
        }
    }
    session.wait_for(LIMIT, "the whole verdict", |t| {
        (t.shows(RESTORED) && t.shows(")")) || t.shows("after:")
    });
    let took = started.elapsed();
    let shown = everything(session);
    assert!(
        took < ENDING,
        "{ending:?} while {waiting}: the application took {took:?} to end"
    );
    assert_eq!(
        Verdict::parse(&shown),
        Some(Verdict {
            restored: true,
            exit: Exit::Code(ending.code())
        }),
        "{ending:?} while {waiting}: {shown}"
    );
    let modes = session.screen().modes();
    assert!(
        modes.cursor_visible,
        "{ending:?} while {waiting}: the cursor is hidden"
    );
    assert!(
        !modes.bracketed_paste,
        "{ending:?} while {waiting}: bracketed paste was left on"
    );
}

/// The trust question holds the session's open in the core's lane when
/// the application starts: each way out ends it at once all the same.
#[test]
fn sys_the_application_ends_at_once_while_the_trust_question_waits() {
    let machine = Machine::new("tui_lane_trust");
    machine.project_config("[model]\nmain = \"claude-opus-5-5\"\n");
    for ending in Ending::all() {
        let mut session = machine.tui(&[]);
        trust_asked(&mut session);
        ends_while_the_lane_waits(
            &mut session,
            ending,
            "the trust question waits at the start",
        );
    }
}

/// The trust question holds a stored session's open (`--continue`), which
/// goes through `session/open` rather than `session/new`.
#[test]
fn sys_the_application_ends_at_once_while_a_resumed_sessions_trust_question_waits() {
    let phases = Phases::new(
        "sys_the_application_ends_at_once_while_a_resumed_sessions_trust_question_waits",
    );
    let machine = Machine::new("tui_lane_resume");
    machine.project_config("[model]\nmain = \"claude-opus-5-5\"\n");
    machine.reply_text("Stored.");
    // A session of the untrusted project, stored: not now, then a turn.
    let mut session = machine.tui(&["remember this"]);
    trust_asked(&mut session);
    session.answer_dialog(LIMIT, TRUST_ANSWERS, b"n", |t| shows_ever(t, "Stored."));
    quit(&mut session, &phases);
    for ending in Ending::all() {
        let mut session = machine.tui(&["--continue"]);
        trust_asked(&mut session);
        ends_while_the_lane_waits(
            &mut session,
            ending,
            "the trust question waits on a resumed session",
        );
    }
}

/// The undo's question about a file edited since the turn holds
/// `session/undo` in the core's lane.
#[test]
fn sys_the_application_ends_at_once_while_the_undos_question_waits() {
    for ending in Ending::all() {
        let machine = Machine::new(&format!("tui_lane_undo_{}", ending.code()));
        let notes = machine.workspace.join("notes.txt");
        machine.reply_tool(
            "toolu_1",
            "write",
            &json!({"path": "notes.txt", "content": "after\n"}),
        );
        machine.reply_text("Written.");
        let mut session = machine.tui(&["--allow", "write notes.txt", "change the notes"]);
        wait_shows(&mut session, "Written.");
        turn_ended(&mut session);
        std::fs::write(&notes, "after, and mine\n").unwrap();
        command(&mut session, "/undo");
        wait_shows(&mut session, "1 Put them back");
        ends_while_the_lane_waits(&mut session, ending, "the undo's question waits");
        assert_eq!(
            std::fs::read_to_string(&notes).unwrap(),
            "after, and mine\n",
            "{ending:?}: a question left unanswered undid the turn"
        );
    }
}

/// Start to first frame of the real application, under both its names,
/// `CINERIS_PERF_RUNS` runs of each (30 unless given): from the moment the
/// shell starts the binary to the moment the first frame is on the
/// screen, read through the pseudo-terminal. The budget
/// (`docs/performance-budgets.md`) is 20 ms at the desk, 50 ms on a Linux
/// runner and 120 ms on a Windows one, in a release build. A measurement:
///
/// ```text
/// gh workflow run try --ref <branch> -f os=windows -f command="perf -p cineris-cli"
/// cargo test --release -p cineris-cli --test system perf_start -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, run in a release build: see its documentation"]
fn perf_start_to_first_frame_under_both_names() {
    let runs: usize = std::env::var("CINERIS_PERF_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    let machine = Machine::new("perf_first_frame");
    let alias = crate::names::install_alias(&machine);
    for (name, program) in [
        ("cineris", std::path::PathBuf::from(CINERIS)),
        ("cin", alias),
    ] {
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

/// Whether the prompt's box shows `text` typed, after the user's glyph.
fn typed(screen: &cineris_testkit::VTerm, text: &str) -> bool {
    GLYPHS.shows(screen, |g| format!("{} {text}", g.user))
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

/// Whether a turn's record is in the statistics on disk: the last thing
/// the core's end writes, after the session's own end. Linux only, as the
/// signal test that asks.
#[cfg(target_os = "linux")]
fn turn_recorded(machine: &Machine) -> bool {
    let Some(state) = machine.dirs().state else {
        return false;
    };
    std::fs::read_dir(state.join("stats"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .any(|text| text.contains("\"kind\":\"turn\""))
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
        assert_eq!(
            Verdict::parse(&everything(&session)),
            Some(Verdict {
                restored: true,
                exit: Exit::Code(code)
            }),
            "SIG{signal}"
        );
        assert!(closed.wait(LIMIT), "SIG{signal}: the turn was not stopped");
        // The session's last records reached the disk: the core ended, in
        // order, before the process did (the review of step 23).
        assert!(
            turn_recorded(&machine),
            "SIG{signal}: the stopped turn's statistics never reached the disk"
        );
    }
    // SIGABRT is not handed over: the terminal is put back, and the abort
    // ends the process as it asks, by its default action.
    let machine = Machine::new("tui_signal_ABRT");
    let mut session = machine.tui(&[]);
    first_frame(&mut session, &Phases::new("abort"));
    let pid = child_status(&session)
        .and_then(|status| status_field(&status, "Pid:"))
        .expect("the binary's process");
    // No core dump: the default action would write one wherever dumps are
    // on (a desk's systemd-coredump), costing time and disk each run.
    let _ = Command::new("prlimit")
        .args(["--core=0", "--pid", pid.as_str()])
        .status();
    let sent = Command::new("kill")
        .args(["-ABRT".to_owned(), pid])
        .status()
        .expect("kill");
    assert!(sent.success(), "SIGABRT was not sent");
    session.wait_for(LIMIT, "the shell's verdict", |t| {
        (t.shows(RESTORED) || t.shows(NOT_RESTORED)) && t.shows(")")
    });
    assert_eq!(
        Verdict::parse(&everything(&session)),
        Some(Verdict {
            restored: true,
            exit: Exit::Signal(6)
        }),
        "SIGABRT"
    );
}

/// Waits until the spinner has turned from its first frame: the hint row, the
/// last row of a frame, shows the second. The console is then closed just
/// after a whole frame, and the next is a tick away (100 ms), so that the
/// signal does not land while one is written: it would end the application
/// with 1 and "the terminal failed: the terminal has been put back", not
/// with 129 (the eighth tooling step's report says so), which these tests
/// are not about. A turn that has begun has drawn its call's box by then.
fn turned(session: &mut Session) {
    session.wait_for(LIMIT, "the spinner turning", |t| {
        GLYPHS.history_has_row(t, |g| {
            RowPattern::starting_with(g.spinner[1]).containing("esc stops")
        })
    });
}

/// How long the application has to end once its console is closed under it:
/// under the watchdog (3 s) that ends a process that does not, so that an
/// end the watchdog made fails. It takes a few milliseconds at the desk.
const CLOSING: Duration = Duration::from_secs(2);

/// Closes the console under the application in `session`, and checks that
/// it ends in order (`docs/steps/23/answered.md`, "Signals, a failed write
/// and a panic": the console's close is SIGHUP's twin): within [`CLOSING`],
/// `stopped` having happened by then, and on Unix with the terminal put
/// back and the code of SIGHUP's end, 129. On Windows the shell goes with
/// the console, so nothing is said on the screen: the test reads what the
/// application leaves outside it, which is `stopped`.
fn closes_in_order(session: &mut Session, what: &str, stopped: impl FnOnce() -> bool) {
    let started = session.close_console();
    assert!(stopped(), "{what}: not stopped when the console closed");
    let took = started.elapsed();
    assert!(
        took < CLOSING,
        "{what}: the application took {took:?} to stop after its console closed"
    );
    #[cfg(unix)]
    {
        session.wait_for(LIMIT, "the shell's verdict", |t| {
            Verdict::parse(&t.history().join("\n")).is_some()
        });
        assert_eq!(
            Verdict::parse(&everything(session)),
            Some(Verdict {
                restored: true,
                exit: Exit::Code(129)
            }),
            "{what}: {}",
            everything(session)
        );
    }
    #[cfg(windows)]
    session.wait_exit(LIMIT);
}

/// The console closing while a turn streams, which is a closed tab: the
/// turn's request is dropped and the application ends, as for SIGHUP
/// (`sys_a_signal_ends_the_application_gracefully_with_its_code`, which
/// Linux alone can send).
#[test]
fn sys_the_console_closing_ends_the_application_and_stops_its_turn() {
    let machine = Machine::new("tui_console_stream");
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
    turned(&mut session);
    closes_in_order(&mut session, "mid-stream", || closed.wait(LIMIT));
}

/// The same while a tool's command runs: it is stopped with the turn, not
/// left behind.
#[test]
fn sys_the_console_closing_stops_a_running_command() {
    let machine = Machine::new("tui_console_tool");
    let roll = Roll::new();
    let command = ChildSpec::new(&roll, "command");
    machine.reply_tool(
        "toolu_1",
        "shell",
        &json!({"command": super::signals::shell_line(&command), "timeout_ms": 600_000}),
    );
    let mut session = machine.tui(&["--permission-mode", "unrestricted", "run it"]);
    let [running]: [_; 1] = roll
        .all(&["command"], LIMIT)
        .try_into()
        .unwrap_or_else(|_| panic!("the command did not start"));
    turned(&mut session);
    closes_in_order(&mut session, "mid-tool", || running.gone_within(LIMIT));
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
/// and a key's time to its frame, `CINERIS_PERF_RUNS` keys (200 unless
/// given), a character typed and taken back by turns, each timed from its
/// write to the frame that shows it. Budgets: 25 MB resident; 2 ms at the
/// 99th percentile for a key (`docs/performance-budgets.md`). A
/// measurement, in a release build: see
/// [`perf_start_to_first_frame_under_both_names`].
#[test]
#[ignore = "a measurement, run in a release build: see its documentation"]
fn perf_idle_memory_and_a_keys_time_to_its_frame() {
    let keys: usize = std::env::var("CINERIS_PERF_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);
    let machine = Machine::new("perf_idle");
    let mut session = machine.tui(&[]);
    session.wait_for(LIMIT, "the first frame", |t| {
        READY.iter().any(|ready| t.shows(ready))
    });
    session.wait_for(LIMIT, "the session's header", |t| t.shows(HEADER));
    settle(&mut session);
    report(&format!("idle with a new session: {}", memory(&session)));
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

/// A streamed answer's piece to the frame that shows it, apart from the cap
/// of 60 frames a second: the scripted server holds each piece until the
/// test lets it go, and the test lets it go only once the screen has been
/// still for longer than a frame's interval, so that the frame for it is
/// due at once (`cineris-term`'s scheduler). The time is from letting it go
/// to the screen showing it, and, beside it, the time beyond the frame that
/// was due: the delta path's own (the budget: 20 ms at the 95th percentile
/// for the whole). Before the fixes of the review of step 23 (finding 12b)
/// each piece was let go about a millisecond after the last frame, so the
/// figure was the cap's 16.7 ms and hid up to that much of a regression. A
/// measurement, in a release build: see
/// [`perf_start_to_first_frame_under_both_names`].
#[test]
#[ignore = "a measurement, run in a release build: see its documentation"]
fn perf_a_streamed_piece_to_its_frame() {
    const PIECES: usize = 40;
    // Longer than a frame's interval (16.7 ms): the stream's next frame is
    // due at once.
    const STILL: Duration = Duration::from_millis(20);
    let machine = Machine::new("perf_stream_piece");
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
    let mut whole = Vec::with_capacity(PIECES);
    let mut beyond = Vec::with_capacity(PIECES);
    for (n, gate) in gates.iter().enumerate() {
        // The screen still for `STILL`: no frame in it, the spinner's
        // included.
        while session.step(STILL).is_some() {}
        let last = session.recording().last().map(|chunk| chunk.at);
        let opened = Instant::now();
        gate.open();
        let text = format!("piece{n:03}");
        let shown = session.wait_for(LIMIT, "the piece", |t| {
            t.history().iter().any(|row| row.contains(&text))
        });
        // The frame before the piece's own, which may be one the spinner
        // drew after the piece was let go: the piece's frame was due one
        // interval after it, or at once.
        let before = session
            .recording()
            .iter()
            .rev()
            .map(|chunk| chunk.at)
            .find(|at| *at + Duration::from_micros(500) < shown)
            .or(last);
        let due = before.map_or(opened, |before| opened.max(before + FRAME));
        whole.push(millis(shown.saturating_duration_since(opened)));
        beyond.push(millis(shown.saturating_duration_since(due)));
    }
    percentiles("a streamed piece to its frame", whole);
    percentiles("  beyond the frame that was due", beyond);
    session.wait_for(LIMIT, "the turn's end", |t| t.shows(READY[0]));
    session.send(b"\x04");
    session.wait_exit(LIMIT);
}

/// A key's time to its frame while an answer streams at 2,000 tokens a
/// second: the scripted server sends a word every half a millisecond, let
/// go on a timer by a thread of the test's, while the test types
/// `CINERIS_PERF_RUNS` keys (200 unless given), one every 23 ms (no multiple
/// of a frame's interval, so that they land all through it), a character
/// typed and taken back by turns, each timed from its write to the frame
/// that shows it (the budget: 4 ms at the 99th percentile). Each key is typed while the
/// stream runs: the test fails if the stream ended first. Before the fixes
/// of the review of step 23 (finding 12a) each key was typed between two
/// pieces, while the server was idle. A measurement, in a release build:
/// see [`perf_start_to_first_frame_under_both_names`].
#[test]
#[ignore = "a measurement, run in a release build: see its documentation"]
fn perf_a_key_while_an_answer_streams() {
    // 2,000 tokens a second, a word each.
    const EVERY: Duration = Duration::from_micros(500);
    const WORDS: usize = 40_000;
    const KEY_EVERY: Duration = Duration::from_millis(23);
    let keys: usize = std::env::var("CINERIS_PERF_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);
    let machine = Machine::new("perf_stream_keys");
    let gates: Vec<Gate> = (0..WORDS).map(|_| Gate::new()).collect();
    let mut reply = ServerReply::sse().send(super::sse(&[
        super::message_start(),
        json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
    ]));
    for (n, gate) in gates.iter().enumerate() {
        // A paragraph of forty words, as an answer has.
        let word = if n % 40 == 39 {
            format!("w{n:05}\n\n")
        } else {
            format!("w{n:05} ")
        };
        reply = reply.wait(gate).send(super::sse(&[json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": word}
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
    let pacer = Pacer::start(gates, EVERY);
    session.wait_for(LIMIT, "the stream", |t| {
        t.history().iter().any(|row| row.contains("w00000"))
    });
    let mut times = Vec::with_capacity(keys);
    let began = Instant::now();
    for n in 0..keys {
        // The output read until the key's time comes.
        let at = began + KEY_EVERY * u32::try_from(n).unwrap_or(u32::MAX);
        loop {
            let left = at.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let _ = session.step(left);
        }
        let (bytes, what): (&[u8], &str) = if n % 2 == 0 {
            (b"q", "the key's echo")
        } else {
            (b"\x7f", "the key taken back")
        };
        let sent = session.send(bytes);
        let shown = session.wait_for(LIMIT, what, |t| typed(t, "q") == (n % 2 == 0));
        times.push(millis(shown.saturating_duration_since(sent)));
    }
    let typing = began.elapsed();
    let (released, ended) = pacer.stop();
    assert!(
        !ended,
        "the stream ended before the last key: give it more than {WORDS} words"
    );
    report(&format!(
        "streamed while the keys were typed: {released} words in {:.1} s, {:.0} a second",
        typing.as_secs_f64(),
        released as f64 / typing.as_secs_f64()
    ));
    percentiles("a key to its frame, streaming", times);
    session.wait_for(LIMIT, "the turn's end", |t| t.shows(READY[0]));
    session.send(b"\x04");
    session.wait_exit(LIMIT);
}

/// The shortest time between two frames the application draws for what
/// streams in: 60 a second (`cineris_term::schedule::STREAM_INTERVAL`).
const FRAME: Duration = Duration::from_micros(16_667);

#[expect(clippy::print_stdout, reason = "a measurement prints what it measured")]
fn report(line: &str) {
    println!("{line}");
}

/// Milliseconds, as a measurement prints them.
fn millis(time: Duration) -> f64 {
    time.as_secs_f64() * 1000.0
}

/// A thread that opens gates one after another on a timer: a stream let go
/// at a rate, for a measurement. It waits by the clock, parked: the only
/// real waiting in these tests, where the time is what is measured.
struct Pacer {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::JoinHandle<(usize, bool)>,
}

impl Pacer {
    /// Opens `gates` in order, the `n`th `every * n` after the start; those
    /// whose time has come are opened together when the system's timer
    /// wakes the thread late (Windows).
    fn start(gates: Vec<Gate>, every: Duration) -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = std::sync::Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let began = Instant::now();
            let mut next = 0;
            while next < gates.len() {
                if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    // The rest at once, so that the answer ends.
                    for gate in &gates[next..] {
                        gate.open();
                    }
                    return (next, false);
                }
                let now = Instant::now();
                let due = began + every * u32::try_from(next).unwrap_or(u32::MAX);
                if now < due {
                    std::thread::park_timeout(due - now);
                    continue;
                }
                while next < gates.len()
                    && began + every * u32::try_from(next).unwrap_or(u32::MAX) <= now
                {
                    gates[next].open();
                    next += 1;
                }
            }
            (next, true)
        });
        Self { stop, thread }
    }

    /// Stops it, opening the gates left at once: how many it had opened on
    /// its timer, and whether that was all of them.
    fn stop(self) -> (usize, bool) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.thread.thread().unpark();
        self.thread.join().expect("the pacer")
    }
}

/// What the binary uses: its resident memory and its peak, in MB, and the
/// CPU time it has used, in the system's units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Usage {
    resident: f64,
    peak: f64,
    cpu: u64,
}

/// What the binary the role shell started for `session` uses: `VmRSS`,
/// `VmHWM` and the CPU ticks from `/proc`.
#[cfg(target_os = "linux")]
fn usage(session: &Session) -> Option<Usage> {
    let status = child_status(session)?;
    let pid = status_field(&status, "Pid:")?;
    let mb = |name: &str| {
        status_field(&status, name)
            .and_then(|kib| kib.parse::<f64>().ok())
            .map(|kib| kib / 1024.0)
    };
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // After the name, which is in brackets and may hold spaces: the state
    // is the first field, user time the twelfth and system time the
    // thirteenth.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let cpu = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    Some(Usage {
        resident: mb("VmRSS:")?,
        peak: mb("VmHWM:")?,
        cpu,
    })
}

/// What the binary the role shell started for `session` uses: its working
/// set and peak working set, and its CPU time in milliseconds, asked of
/// PowerShell (about a third of a second a question).
#[cfg(windows)]
fn usage(session: &Session) -> Option<Usage> {
    let script = format!(
        "$c = Get-CimInstance Win32_Process -Filter 'ParentProcessId={}' | \
         Where-Object {{ $_.Name -like 'c*.exe' }} | Select-Object -First 1; \
         $p = Get-Process -Id $c.ProcessId; \
         \"$($p.WorkingSet64) $($p.PeakWorkingSet64) $([long]$p.TotalProcessorTime.TotalMilliseconds)\"",
        session.pid()
    );
    let output = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.split_whitespace();
    let mut mb = || {
        fields
            .next()?
            .parse::<f64>()
            .ok()
            .map(|bytes| bytes / 1024.0 / 1024.0)
    };
    let (resident, peak) = (mb()?, mb()?);
    let cpu = text.split_whitespace().nth(2)?.parse().ok()?;
    Some(Usage {
        resident,
        peak,
        cpu,
    })
}

#[cfg(not(any(target_os = "linux", windows)))]
fn usage(_: &Session) -> Option<Usage> {
    None
}

/// Waits until the binary has used no CPU for a second: its session's
/// first index pass made, and whatever else the open started. Its output
/// is read meanwhile. Where its use cannot be read, it does not wait.
fn settle(session: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(120);
    let Some(mut cpu) = usage(session).map(|usage| usage.cpu) else {
        return;
    };
    let mut since = Instant::now();
    while since.elapsed() < Duration::from_secs(1) {
        assert!(Instant::now() < deadline, "the binary never settled");
        // Output, or a quarter of a second without any.
        let _ = session.step(Duration::from_millis(250));
        let now = usage(session).map_or(cpu, |usage| usage.cpu);
        if now != cpu {
            cpu = now;
            since = Instant::now();
        }
    }
}

/// The binary's resident memory and its peak, as a measurement prints them.
fn memory(session: &Session) -> String {
    usage(session).map_or_else(
        || "resident memory not read on this system".to_owned(),
        |usage| {
            format!(
                "{:.1} MB resident, {:.1} MB at the peak",
                usage.resident, usage.peak
            )
        },
    )
}

/// Fills `workspace` with `count` files of real Rust code, the corpus's
/// `.rs` files in name order (taken again in turn when it has fewer), a
/// hundred to a directory: hard links where the two are on one volume,
/// copies otherwise. Returns how many were links.
fn real_code(workspace: &std::path::Path, count: usize) -> usize {
    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, want: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            if out.len() >= want {
                return;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                rust_files(&entry.path(), out, want);
            } else if kind.is_file() && entry.path().extension().is_some_and(|e| e == "rs") {
                out.push(entry.path());
            }
        }
    }
    let corpus = cineris_testkit::corpus::real_code();
    let mut found = Vec::new();
    rust_files(&corpus, &mut found, count);
    assert!(!found.is_empty(), "no .rs file in {}", corpus.display());
    let mut linked = 0;
    for n in 0..count {
        let dir = n / 100;
        let to = workspace.join(format!("d{:03}/s{dir:04}/f{n}.rs", dir / 10));
        std::fs::create_dir_all(to.parent().expect("a parent")).expect("a directory");
        let from = &found[n % found.len()];
        if std::fs::hard_link(from, &to).is_ok() {
            linked += 1;
        } else {
            std::fs::copy(from, &to).expect("a copy");
        }
    }
    linked
}

/// The application idle with a new session in a workspace of real code,
/// for each count of files `CINERIS_PERF_FILES` lists (`20000,100000`
/// unless given, of the pinned corpus's files: `cargo xtask corpus fetch`):
/// its resident memory and its peak once it has used no CPU for a second,
/// the session's first index pass made (`index.map` is on); then the same
/// after the first `@`, which lists the workspace's files for completion.
/// The budget (`docs/performance-budgets.md`, "Memory"): 25 MB resident at
/// idle, in an empty workspace and in one of 20,000 files.
/// `CINERIS_PERF_CONFIG` adds lines to the user's configuration, to see what
/// holds the memory (`index.map = false`). On Linux the memory is `/proc`'s
/// `VmRSS` and `VmHWM`; on Windows the working set and its peak, asked of
/// PowerShell. A measurement, in a release build: see
/// [`perf_start_to_first_frame_under_both_names`].
#[test]
#[ignore = "a measurement, run in a release build: see its documentation"]
fn perf_idle_memory_in_a_workspace_of_real_code() {
    let counts: Vec<usize> = std::env::var("CINERIS_PERF_FILES")
        .unwrap_or_else(|_| "20000,100000".to_owned())
        .split(',')
        .filter_map(|count| count.trim().parse().ok())
        .collect();
    // Settings of the user's beside the machine's, to see what holds the
    // memory (`index.map = false`, say).
    let extra = std::env::var("CINERIS_PERF_CONFIG").unwrap_or_default();
    if !extra.is_empty() {
        report(&format!("with {extra:?}"));
    }
    for count in counts {
        let machine = Machine::with_config(&format!("perf_idle_{count}"), &extra);
        let made = Instant::now();
        let linked = real_code(&machine.workspace, count);
        report(&format!(
            "a workspace of {count} files of real code ({linked} hard links), made in {:.1} s",
            made.elapsed().as_secs_f64()
        ));
        let mut session = machine.tui(&[]);
        session.wait_for(LIMIT, "the first frame", |t| {
            READY.iter().any(|ready| t.shows(ready))
        });
        session.wait_for(LIMIT, "the session's header", |t| t.shows(HEADER));
        settle(&mut session);
        report(&format!("idle, {count} files: {}", memory(&session)));
        session.send(b"@");
        session.wait_for(LIMIT, "the workspace's files in the menu", |t| {
            t.shows(".rs") && typed(t, "@")
        });
        settle(&mut session);
        report(&format!(
            "after the first `@`, {count} files: {}",
            memory(&session)
        ));
        session.send(b"\x7f");
        session.wait_for(LIMIT, "the `@` taken back", |t| t.shows(READY[0]));
        session.send(b"\x04");
        session.wait_exit(LIMIT);
    }
}

/// The memory of a 200-turn session in each presenter (step 25's "Done
/// when"; `docs/performance-budgets.md`, "Memory": 40 MB inline, 80 MB full
/// screen): a workspace of 20,000 files of the real-code corpus, and 200
/// turns each of a read, an edit with a diff and a command that prints 200
/// lines, then an answer of about 40 lines with a code block; in the full
/// screen, the transcript then scrolled to its top and back, resized twice
/// and searched once. The binary's resident memory and its peak are read
/// once it has settled. Ignored; on the shipped musl `dist` build:
///
/// ```text
/// cargo test --profile dist --target x86_64-unknown-linux-musl -p cineris-cli --test system perf_two_hundred_turns -- --ignored --nocapture
/// ```
///
/// `CINERIS_PERF_TURNS` sets the turns, `CINERIS_PERF_FILES` the files.
#[test]
#[ignore = "a measurement, in a release build"]
fn perf_two_hundred_turns_in_each_presenter() {
    let turns: usize = std::env::var("CINERIS_PERF_TURNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);
    let files: usize = std::env::var("CINERIS_PERF_FILES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20_000);
    let answer: String = {
        let mut text = String::from("Here is what changed, and why.\n\n");
        for n in 1..=12 {
            text.push_str(&format!(
                "Paragraph {n}: the scanner now treats an underscore between two digits \
                 as a separator, and anything else as the end of the number.\n\n"
            ));
        }
        text.push_str("```rust\n");
        for n in 1..=16 {
            text.push_str(&format!("let value_{n} = scan(\"1_{n:03}\")?;\n"));
        }
        text.push_str("```\n\nThe tests pass.\n");
        text
    };
    let print = if cfg!(windows) {
        "1..200 | ForEach-Object { \"line $_ of the build's output, as a compiler prints it\" }"
    } else {
        "for i in $(seq 1 200); do echo \"line $i of the build's output, as a compiler prints it\"; done"
    };
    for presenter in ["inline", "fullscreen"] {
        let machine = Machine::new(&format!("perf_turns_{presenter}"));
        let made = Instant::now();
        let linked = real_code(&machine.workspace, files);
        report(&format!(
            "{presenter}: a workspace of {files} files of real code ({linked} hard links), made in {:.1} s",
            made.elapsed().as_secs_f64()
        ));
        std::fs::write(machine.workspace.join("count.txt"), "count 0\n").unwrap();
        for n in 1..=turns {
            let calls = [
                ("read", json!({"path": "count.txt"})),
                (
                    "edit",
                    json!({"path": "count.txt", "old_string": format!("count {}", n - 1),
                        "new_string": format!("count {n}")}),
                ),
                ("shell", json!({"command": print})),
            ];
            let mut events = vec![super::message_start()];
            for (index, (tool, input)) in calls.iter().enumerate() {
                events.push(json!({"type": "content_block_start", "index": index,
                    "content_block": {"type": "tool_use", "id": format!("toolu_{n}_{index}"),
                        "name": tool, "input": {}}}));
                events.push(json!({"type": "content_block_delta", "index": index,
                    "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}));
                events.push(json!({"type": "content_block_stop", "index": index}));
            }
            events.push(super::message_delta("tool_use"));
            events.push(json!({"type": "message_stop"}));
            machine
                .server
                .push(ServerReply::sse().send(super::sse(&events)));
            machine.reply_text(&format!("{answer}Turn {n} done.\n"));
        }
        let mut arguments = vec!["--permission-mode", "unrestricted"];
        if presenter == "fullscreen" {
            arguments.push("--fullscreen");
        }
        let mut session = machine.tui(&arguments);
        session.wait_for(LIMIT, "the first frame", |t| {
            READY.iter().any(|ready| t.shows(ready))
        });
        session.wait_for(LIMIT, "the session's header", |t| t.shows(HEADER));
        settle(&mut session);
        report(&format!(
            "{presenter}: idle before the turns, {}",
            memory(&session)
        ));
        let began = Instant::now();
        for n in 1..=turns {
            session.send(format!("turn {n}\r").as_bytes());
            let done = format!("Turn {n} done.");
            session.wait_for(Duration::from_secs(60), "the turn's answer", |t| {
                shows_ever(t, &done)
            });
            turn_ended(&mut session);
        }
        report(&format!(
            "{presenter}: {turns} turns in {:.1} s",
            began.elapsed().as_secs_f64()
        ));
        settle(&mut session);
        report(&format!(
            "{presenter}: after {turns} turns, {}",
            memory(&session)
        ));
        if presenter == "fullscreen" {
            // To the top and back, two resizes, and a search.
            for _ in 0..(turns * 4) {
                session.send(b"\x1b[5~");
            }
            session.wait_for(LIMIT, "the top", |t| t.shows(HEADER));
            for _ in 0..(turns * 4) {
                session.send(b"\x1b[6~");
            }
            session.resize(70, 30);
            session.resize(100, 30);
            session.send(b"\x06turn 1\r");
            settle(&mut session);
            session.send(b"\x1b");
            settle(&mut session);
            report(&format!(
                "{presenter}: scrolled, resized and searched, {}",
                memory(&session)
            ));
        }
        session.send(b"\x04");
        session.wait_exit(LIMIT);
    }
}

/// The look and a key binding changed in the user's file while the
/// application runs take effect after the next key: its frame asks the
/// core to look at the files, and the reload's `tui.*` and `keys.*` keys
/// are read again (step 24's hot reload). A file broken after that leaves
/// both as they were, and `/reload` says why, once.
#[test]
fn sys_a_look_and_a_binding_changed_in_the_file_reach_the_next_key() {
    let phases = Phases::new("sys_a_look_and_a_binding_changed_in_the_file_reach_the_next_key");
    let machine = Machine::new("tui_hot_reload");
    machine.reply_text("Sent with the new key.");
    let mut session = machine.tui(&[]);
    first_frame(&mut session, &phases);
    // The glyphs at the start, and the input box's border as the other
    // set draws it: the header card drawn at the start keeps its own.
    let unicode = session.screen().shows("╭");
    let other = if unicode {
        "+------"
    } else {
        "╭──────"
    };

    phases.mark("the file changed");
    let glyphs = if unicode { "ascii" } else { "unicode" };
    machine.user_config(&format!(
        "tui.glyphs = \"{glyphs}\"\nkeys.editor.submit = [\"ctrl+s\"]\n"
    ));
    // The first key's frame asks for the look.
    type_text(&mut session, "hello");
    session.wait_for(LIMIT, "the look read again", |t| {
        shows_ever(t, "in force now") && t.shows(other)
    });
    // Ctrl+S is submit now: a key with no escape before it, which ConPTY
    // passes on as it is (docs/conpty.md: a sequence after an escape is
    // misread there).
    session.send(b"\x13");
    wait_shows(&mut session, "Sent with the new key.");
    turn_ended(&mut session);

    phases.mark("the file broken");
    machine.user_config("tui.glyphs = [\n");
    // `/reload`, its row of the command menu taken with tab, and sent with
    // Ctrl+S.
    type_text(&mut session, "/reload");
    session.wait_for(LIMIT, "the command menu", |t| {
        t.shows("Read the configuration files again")
    });
    session.send(b"\t");
    session.wait_for(LIMIT, "the menu closed", |t| {
        !t.shows("Read the configuration files again")
    });
    session.send(b"\x13");
    session.wait_for(LIMIT, "the error said", |t| shows_ever(t, "config.toml"));
    assert!(
        session.screen().shows(other),
        "the look changed with a broken file:\n{}",
        session.screen().dump()
    );

    quit(&mut session, &phases);
}

// ------------------------------------------------------------------ attach

/// A server for `attach` to reach: the binary's `serve --socket`, at a
/// place of the test's own (on Unix a short path, since a socket's path has
/// at most 107 bytes; on Windows a pipe's name), with the machine's
/// environment, ended when dropped.
struct SocketServer {
    child: std::process::Child,
    socket: String,
    /// The socket's folder, removed at the end (Unix).
    folder: Option<std::path::PathBuf>,
    /// Held so that a line the server writes to it later still goes
    /// somewhere.
    _errors: std::io::BufReader<std::process::ChildStderr>,
}

impl Drop for SocketServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(folder) = &self.folder {
            let _ = std::fs::remove_dir_all(folder);
        }
    }
}

impl Machine {
    /// `serve --socket` on this machine, once it says where it serves.
    fn socket_server(&self, name: &str) -> SocketServer {
        use std::io::BufRead as _;
        #[cfg(unix)]
        let (socket, folder) = {
            use std::os::unix::fs::PermissionsExt as _;
            let folder =
                std::path::PathBuf::from(format!("/tmp/cin-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
            (
                folder.join("rpc.sock").to_string_lossy().into_owned(),
                Some(folder),
            )
        };
        #[cfg(windows)]
        let (socket, folder) = (format!("cineris-test-{name}-{}", std::process::id()), None);
        let mut command = self.command(&["serve", "--socket", &socket]);
        command.stdout(std::process::Stdio::null());
        let mut child = command.spawn().expect("the server starts");
        let mut errors = std::io::BufReader::new(child.stderr.take().expect("its standard error"));
        let mut said = String::new();
        errors
            .read_line(&mut said)
            .expect("the server says where it serves");
        assert!(said.contains("serving at"), "{said}");
        SocketServer {
            child,
            socket,
            folder,
            _errors: errors,
        }
    }
}

/// `attach` runs the application on a server's session: a prompt from its
/// command line is answered, and the session goes on when `attach` ends, so
/// that `attach --continue` opens it again with its history drawn
/// (`docs/steps/27/answered.md`, point 5).
#[test]
fn sys_attach_runs_a_turn_on_the_server_and_its_session_outlives_it() {
    let phases = Phases::new("sys_attach_runs_a_turn_on_the_server_and_its_session_outlives_it");
    let machine = Machine::new("tui_attach");
    machine.reply_text("Hello over the socket.");
    let server = machine.socket_server("attach");
    let mut session = machine.tui(&["attach", "--socket", &server.socket, "say hello"]);
    first_frame(&mut session, &phases);
    phases.mark("the turn");
    wait_shows(&mut session, "Hello over the socket.");
    turn_ended(&mut session);
    quit(&mut session, &phases);

    phases.mark("attached again");
    let mut again = machine.tui(&["attach", "--socket", &server.socket, "--continue"]);
    first_frame(&mut again, &phases);
    wait_shows(&mut again, "Hello over the socket.");
    quit(&mut again, &phases);
}

/// Over `attach` as in process, a key typed as an approval's dialog comes
/// up does not answer it: the dialog's grace and quiet are the
/// application's (`app::dialog`), whichever link it drives. Only a key
/// pressed once its answers are drawn does.
#[test]
fn sys_typing_through_an_approval_over_attach_does_not_answer_it() {
    let phases = Phases::new("sys_typing_through_an_approval_over_attach_does_not_answer_it");
    let machine = Machine::new("tui_attach_approval");
    let target = ".cineris/notes.md";
    machine.reply_tool(
        "toolu_1",
        "write",
        &json!({"path": target, "content": "remember the feature\n"}),
    );
    machine.reply_text("Noted.");
    let server = machine.socket_server("approval");
    let mut session = machine.tui(&["attach", "--socket", &server.socket, "note the feature"]);
    first_frame(&mut session, &phases);

    phases.mark("the dialog");
    session.wait_for(LIMIT, "the dialog", |t| shows_ever(t, "notes.md"));
    // As a person typing on: at the dialog's first sight, or just before it.
    type_text(&mut session, "y");
    wait_shows(&mut session, "Allow once");
    assert!(
        !machine.workspace.join(target).exists(),
        "a key typed through the dialog answered it"
    );
    session.answer_dialog(LIMIT, "Allow once", b"y", |t| shows_ever(t, "Noted."));
    assert_eq!(
        std::fs::read_to_string(machine.workspace.join(target)).unwrap(),
        "remember the feature\n"
    );
    // The key typed through went to the editor, if anywhere: taken out,
    // so that `Ctrl+D` on an empty prompt quits.
    session.send(b"\x7f");
    session.wait_for(LIMIT, "the editor empty", |t| t.shows(READY[0]));
    quit(&mut session, &phases);
    drop(server);
}

/// When the server goes, `attach` ends: it says the server's connection
/// has ended, and the terminal is put back.
#[test]
fn sys_attach_ends_when_the_server_goes() {
    let phases = Phases::new("sys_attach_ends_when_the_server_goes");
    let machine = Machine::new("tui_attach_gone");
    let server = machine.socket_server("gone");
    let mut session = machine.tui(&["attach", "--socket", &server.socket]);
    first_frame(&mut session, &phases);
    phases.mark("the server ends");
    drop(server);
    session.wait_for(LIMIT, "attach to say the server has gone", |t| {
        shows_ever(t, "connection has ended")
    });
    session.wait_for(LIMIT, "the shell's verdict", |t| {
        t.shows(RESTORED) || t.shows(NOT_RESTORED)
    });
    assert!(
        session.screen().shows(RESTORED),
        "the modes were not put back:\n{}",
        session.screen().dump()
    );
}

/// The text of a row of the prompt's box: its borders and its marker off.
fn in_the_box(row: &str) -> &str {
    row.trim_end()
        .trim_end_matches(['│', '|'])
        .trim_end()
        .trim_start_matches(['│', '|'])
        .trim_start()
        .trim_start_matches('◆')
        .trim_start()
}

/// Waits for `condition` after the keys `names` were sent, the program's
/// output from the piece `from` on: a key that did not arrive as itself
/// fails with the bytes each was sent as and what the program wrote since,
/// kept in memory and printed only then (step 25's design: a key that does
/// not arrive says what came of it; the review of its second pull request).
fn wait_after_keys(
    session: &mut Session,
    from: usize,
    names: &[&str],
    mut condition: impl FnMut(&VTerm) -> bool,
) {
    let deadline = Instant::now() + LIMIT;
    while !condition(session.screen()) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || (session.step(left).is_none() && session.ended()) {
            let since: Vec<u8> = session.recording()[from.min(session.recording().len())..]
                .iter()
                .flat_map(|chunk| chunk.bytes.iter().copied())
                .collect();
            let shown = &since[since.len().saturating_sub(4096)..];
            let sent: Vec<String> = names
                .iter()
                .map(|name| {
                    let bytes =
                        cineris_testkit::pty::key_bytes(name, cfg!(windows)).unwrap_or_default();
                    format!("{name} as {:?}", String::from_utf8_lossy(&bytes))
                })
                .collect();
            panic!(
                "waited {LIMIT:?} for {names:?} to reach the application as themselves; sent: \
                 {}; the program wrote since (its last 4 KiB): {:?}; the screen:\n{}",
                sent.join(", "),
                String::from_utf8_lossy(shown),
                session.screen().dump()
            );
        }
    }
}

/// F1 to F12 and the arrows reach the application as themselves, through
/// ConPTY on Windows as win32 input records, as Windows Terminal sends them
/// (step 25, `function-keys-through-conpty`): each function key, bound to
/// the editor's new line, ends the line typed before it, and the arrows move
/// in the prompt.
#[test]
fn sys_function_keys_and_the_arrows_reach_the_application() {
    let phases = Phases::new("sys_function_keys_and_the_arrows_reach_the_application");
    let machine = Machine::new("tui_function_keys");
    let keys: Vec<String> = (1..=12).map(|n| format!("\"f{n}\"")).collect();
    machine.user_config(&format!("keys.editor.new_line = [{}]\n", keys.join(", ")));
    let mut session = machine.tui(&[]);
    first_frame(&mut session, &phases);

    phases.mark("the function keys");
    for n in 1..=12 {
        type_text(&mut session, &format!("a{n}"));
        let from = session.recording().len();
        session.send_key(&format!("f{n}"));
        type_text(&mut session, &format!("b{n}"));
        let (before, after) = (format!("a{n}"), format!("b{n}"));
        wait_after_keys(&mut session, from, &[&format!("f{n}")], |t| {
            let rows = t.screen();
            rows.iter().any(|row| in_the_box(row).ends_with(&before))
                && rows.iter().any(|row| in_the_box(row).starts_with(&after))
        });
    }

    phases.mark("the arrows");
    let from = session.recording().len();
    session.send_key("f1");
    type_text(&mut session, "pq");
    session.send_key("left");
    type_text(&mut session, "M");
    session.send_key("right");
    type_text(&mut session, "N");
    session.send_key("up");
    type_text(&mut session, "U");
    session.send_key("down");
    type_text(&mut session, "D");
    let arrows = ["f1", "left", "right", "up", "down"];
    wait_after_keys(&mut session, from, &arrows, |t| {
        let rows = t.screen();
        rows.iter().any(|row| in_the_box(row).ends_with("b12U"))
            && rows.iter().any(|row| in_the_box(row) == "pMqND")
    });
    // Ctrl+C empties the prompt, so that Ctrl+D quits.
    session.send(b"\x03");
    wait_shows(&mut session, "Describe a task");
    quit(&mut session, &phases);
}
