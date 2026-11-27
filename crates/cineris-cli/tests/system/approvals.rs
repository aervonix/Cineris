//! The approvals password at a terminal, end to end (the signed approvals
//! step): `cineris approvals setup`, `reset` and `status`, `ext add` and
//! `permissions trust` signing with the password typed at the prompt, the
//! application asking for it when a session opens with approvals to use,
//! and guard A over a session's commands. Each under a pseudo-terminal of
//! its own, through the shell role of `tui`, keys sent with `send_key`.
//!
//! **Argon2's cost.** A key made at the standard cost takes seconds in an
//! unoptimised build. Every test but the two that must write one (`setup`,
//! and `reset`, which always writes the standard cost) writes the person's
//! key file itself, at a test's cost ([`Machine::person`]).

use std::path::PathBuf;
use std::time::Duration;

use cineris_testkit::VTerm;
use cineris_testkit::pty::Session;
use cineris_testkit::role::{NOT_RESTORED, RESTORED};

use super::tui::{LIMIT, shows_ever};
use crate::{Machine, shown};

/// The person's password: what [`Machine::person`] sets up.
/// Words of its own, so that no eight characters of it in a row are in
/// anything the binary says.
pub const PASSWORD: &str = "Vq7 tidemork 91 Plover";

/// Another, which is not theirs.
const WRONG: &str = "Zr4 bellgrave 55 Mottle";

/// What the binary ended with, as the shell said, and every row it showed.
pub struct Ended {
    pub code: Option<i32>,
    pub screen: String,
    /// The bytes it wrote, as text, unparsed.
    pub raw: String,
}

impl Ended {
    pub fn expect_code(&self, code: i32) -> &Self {
        assert_eq!(self.code, Some(code), "{}", self.screen);
        self
    }
}

impl Machine {
    /// Sets the person's approvals up with [`PASSWORD`], as `cineris
    /// approvals setup` would, but at a test's cost: the key file written
    /// to the state directory.
    pub fn person(&self) {
        use cineris_core::signed::{Cost, KEY_FILE, KeyFile, Salt};
        let state = cineris_config::StateDir::from_dirs(&self.dirs()).unwrap();
        std::fs::create_dir_all(state.root()).unwrap();
        let (file, _) = KeyFile::make(
            PASSWORD.as_bytes(),
            Salt::from_bytes([7; Salt::LEN]),
            Cost::new(64, 1, 1).unwrap(),
            cineris_protocol::ids::Timestamp::new("2026-11-27T10:00:00Z"),
        )
        .unwrap();
        file.write(
            &state.root().join(KEY_FILE),
            &cineris_base::SystemClock::new(),
        )
        .unwrap();
    }

    /// The key file's place.
    pub fn key_file(&self) -> PathBuf {
        cineris_config::StateDir::from_dirs(&self.dirs())
            .unwrap()
            .root()
            .join(cineris_core::signed::KEY_FILE)
    }

    /// Runs the binary with `arguments` at a terminal of its own, answers
    /// each `[y/N]` it asks with the next of `yes`, and the password's
    /// prompt with `password`, once its field takes keys; then waits for
    /// it to end.
    pub fn signing(&self, arguments: &[&str], yes: &[&str], password: &str) -> Ended {
        let mut session = self.tui(arguments);
        for answer in yes {
            session.wait_for(LIMIT, "a question", |t| shows_ever(t, "[y/N]"));
            session.send(format!("{answer}\r").as_bytes());
        }
        type_password(&mut session, password);
        ended(&mut session)
    }
}

/// Opens the application in the machine's workspace, which has approvals
/// to use, types the person's password when it asks, waits until the
/// session's header names `model`, and quits: what a session opened with
/// the person's approvals runs on.
pub fn header_with_the_password(machine: &Machine, model: &str) {
    use cineris_testkit::glyphs::RowPattern;
    let mut session = machine.tui(&[]);
    session.wait_for(LIMIT, "the approvals' password", |t| {
        t.shows("to use this session's approvals")
    });
    type_password(&mut session, PASSWORD);
    session.wait_for(LIMIT, "the header's model", |t| {
        super::tui::GLYPHS.history_has_row(t, |g| {
            RowPattern::starting_with(g.vertical).containing(model)
        })
    });
    super::tui::quit(&mut session, &cineris_testkit::Phases::new("the header"));
}

impl Ended {
    /// The one JSON object of `--output json`, from the bytes the binary
    /// wrote: its last line that is one.
    pub fn json(&self) -> serde_json::Value {
        self.raw
            .lines()
            .rev()
            .find_map(|line| {
                let at = line.find("{\"")?;
                serde_json::from_str(line[at..].trim_end()).ok()
            })
            .unwrap_or_else(|| panic!("no JSON record:\n{}", self.screen))
    }

    /// As a run's streams, the screen for both: for what reads a [`Run`].
    pub fn as_run(&self) -> crate::Run {
        crate::Run {
            code: self.code,
            stdout: self.screen.clone(),
            stderr: self.screen.clone(),
        }
    }
}

/// Whether the prompt's field takes keys: its row of keys is drawn.
fn field_ready(t: &VTerm) -> bool {
    [
        "enter approves",
        "enter goes on",
        "enter keeps it",
        "enter opens them",
    ]
    .iter()
    .any(|keys| t.shows(keys))
}

/// Types `password` at the prompt once its field takes keys, and `Enter`
/// once it has every character.
pub fn type_password(session: &mut Session, password: &str) {
    session.wait_for(LIMIT, "the password's field", field_ready);
    session.send(password.as_bytes());
    let count = format!("{} characters, hidden", password.chars().count());
    session.wait_for(LIMIT, "the characters typed", |t| t.shows(&count));
    session.send(b"\r");
}

/// Waits for the binary to end, and for the shell's word on the terminal.
pub fn ended(session: &mut Session) -> Ended {
    session.wait_for(LIMIT, "the shell's verdict", |t| {
        (t.shows(RESTORED) || t.shows(NOT_RESTORED)) && t.shows(")")
    });
    let screen = session.screen().history().join("\n");
    assert!(
        session.screen().shows(RESTORED),
        "the terminal was not put back:\n{screen}"
    );
    let code = ["exit status: ", "exit code: "].iter().find_map(|said| {
        let at = screen.find(said)?;
        screen[at + said.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .ok()
    });
    let bytes: Vec<u8> = session
        .recording()
        .iter()
        .flat_map(|chunk| chunk.bytes.iter().copied())
        .collect();
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let _ = session.wait_exit(Duration::from_secs(30));
    Ended { code, screen, raw }
}

/// Whether `text` holds any eight characters of `password` in a row.
fn holds_any_of(text: &str, password: &str) -> Option<String> {
    let chars: Vec<char> = password.chars().collect();
    chars
        .windows(8)
        .map(|window| window.iter().collect::<String>())
        .find(|piece| text.contains(piece.as_str()))
}

/// `approvals setup` chooses the password at the terminal, typed twice and
/// never echoed: 11 characters are refused and 12 taken, a second typing
/// that differs asks again, and the key is written at the standard cost
/// (the one test that pays it for set-up). A second set-up is refused, and
/// with no terminal nothing is asked.
#[test]
fn sys_approvals_setup_chooses_a_password_twice_unechoed() {
    let mut machine = Machine::new("approvals_setup");
    machine.quick_dialogs();
    let short = "elevenchars";
    let chosen = "twelve chars";
    let mut session = machine.tui(&["approvals", "setup"]);
    session.wait_for(LIMIT, "the prompt", |t| t.shows("Choose a password"));
    type_password(&mut session, short);
    session.wait_for(LIMIT, "eleven refused", |t| t.shows("that one has fewer"));
    type_password(&mut session, chosen);
    session.wait_for(LIMIT, "the second typing", |t| {
        t.shows("Type the same password again")
    });
    type_password(&mut session, "twelve charz");
    session.wait_for(LIMIT, "a mismatch", |t| {
        t.shows("The two were not the same")
    });
    type_password(&mut session, chosen);
    type_password(&mut session, chosen);
    let setup = ended(&mut session);
    setup.expect_code(0);
    assert!(
        setup.screen.contains("Your approvals password is set (key"),
        "{}",
        setup.screen
    );
    for typed in [short, chosen] {
        assert_eq!(holds_any_of(&setup.raw, typed), None, "{typed} was written");
    }
    let file = std::fs::read_to_string(machine.key_file()).unwrap();
    assert!(file.contains("memory_kib = 65536"), "{file}");

    // A second set-up is refused, at a terminal, before anything is asked.
    let mut session = machine.tui(&["approvals", "setup"]);
    let again = ended(&mut session);
    again.expect_code(2);
    assert!(
        again
            .screen
            .contains("a password is set for your approvals already"),
        "{}",
        again.screen
    );
    // With no terminal, nothing is asked, and it says where to run it.
    let piped = machine.run(&["approvals", "setup"]);
    piped.expect_code(2);
    assert!(
        piped.stderr.contains("in your own terminal"),
        "{}",
        shown(&piped)
    );
}

/// What is typed before the prompt is drawn does not count, and is not
/// echoed: the terminal is taken before the core starts. The binary is held
/// just before its core (`cineris_base::hook::HOLD_CORE`), a password and
/// `Enter` typed then, and the core let go: the field is empty, and `Esc`
/// sets nothing. Unix only: Windows' console echoes nothing outside a read
/// in line mode.
#[cfg(unix)]
#[test]
fn sys_a_password_typed_ahead_of_the_prompt_does_not_count() {
    let mut machine = Machine::new("approvals_typed_ahead");
    machine.quick_dialogs();
    let hold = machine.home.join("hold");
    std::fs::create_dir_all(&hold).unwrap();
    machine.hold_the_core(&hold);
    let mut session = machine.tui(&["approvals", "setup"]);
    let deadline = std::time::Instant::now() + LIMIT;
    while !hold.join("held").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "not held before its core"
        );
        session.step(Duration::from_millis(20));
    }
    session.send(format!("{PASSWORD}\r").as_bytes());
    while session.step(Duration::from_millis(200)).is_some() {}
    std::fs::write(hold.join("go"), b"").unwrap();
    session.wait_for(LIMIT, "the field", field_ready);
    assert!(
        session.screen().shows("0 characters, hidden"),
        "{}",
        session.screen().dump()
    );
    session.send_key("escape");
    let ended = ended(&mut session);
    ended.expect_code(2);
    assert_eq!(holds_any_of(&ended.raw, PASSWORD), None, "it was echoed");
    assert!(!machine.key_file().exists(), "a password was set");
}

/// Starts the binary with `arguments` at a terminal, held just before its
/// core starts (`cineris_base::hook::HOLD_CORE`), types the person's
/// password and `Enter` then, reads what the terminal shows of it, and
/// lets the core go: the session, with whatever was echoed.
#[cfg(unix)]
fn typed_while_held(machine: &mut Machine, arguments: &[&str]) -> Session {
    let hold = machine.home.join("hold");
    std::fs::create_dir_all(&hold).unwrap();
    // What an earlier hold left: held already, and let go.
    for left in ["held", "go"] {
        let _ = std::fs::remove_file(hold.join(left));
    }
    machine.hold_the_core(&hold);
    let mut session = machine.tui(arguments);
    let deadline = std::time::Instant::now() + LIMIT;
    while !hold.join("held").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "not held before its core"
        );
        session.step(Duration::from_millis(20));
    }
    session.send(format!("{PASSWORD}\r").as_bytes());
    // Whatever the terminal echoes comes back at once.
    while session.step(Duration::from_millis(200)).is_some() {}
    std::fs::write(hold.join("go"), b"").unwrap();
    session
}

/// The bytes the session has shown so far, unparsed.
#[cfg(unix)]
fn raw_so_far(session: &Session) -> String {
    let bytes: Vec<u8> = session
        .recording()
        .iter()
        .flat_map(|chunk| chunk.bytes.iter().copied())
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `ext add` and a headless run at a terminal take the terminal before
/// their core starts (and before the key file is read): a password typed
/// then is never echoed, and does not answer the prompt, which then takes
/// the password typed at it. Unix only: Windows' console echoes nothing
/// outside a read in line mode.
#[cfg(all(unix, feature = "scripts"))]
#[test]
fn sys_a_password_typed_before_a_subcommand_or_a_run_asks_is_never_echoed() {
    let mut machine = Machine::new("approvals_typed_early");
    machine.quick_dialogs();
    machine.person();
    let path = tracker(&machine);
    let mut session = typed_while_held(&mut machine, &["ext", "add", &path]);
    assert_eq!(
        holds_any_of(&raw_so_far(&session), PASSWORD),
        None,
        "echoed while ext add's core started"
    );
    session.wait_for(LIMIT, "the field", field_ready);
    assert!(
        session.screen().shows("0 characters, hidden"),
        "{}",
        session.screen().dump()
    );
    type_password(&mut session, PASSWORD);
    let added = ended(&mut session);
    added.expect_code(0);
    assert_eq!(holds_any_of(&added.raw, PASSWORD), None);

    // The pin carries a signature: a headless run at a terminal asks.
    machine.reply_text("Ran.");
    let mut session = typed_while_held(&mut machine, &["-p", "hi"]);
    assert_eq!(
        holds_any_of(&raw_so_far(&session), PASSWORD),
        None,
        "echoed while the run's core started"
    );
    session.wait_for(LIMIT, "the field", field_ready);
    assert!(
        session.screen().shows("0 characters, hidden"),
        "{}",
        session.screen().dump()
    );
    type_password(&mut session, PASSWORD);
    let ran = ended(&mut session);
    ran.expect_code(0);
    assert!(ran.screen.contains("Ran."), "{}", ran.screen);
    assert_eq!(holds_any_of(&ran.raw, PASSWORD), None);

    // The application takes the terminal before its core starts too (the
    // full review of the signed approvals: it took it only after).
    let sent = machine.server.requests().len();
    let mut session = typed_while_held(&mut machine, &[]);
    assert_eq!(
        holds_any_of(&raw_so_far(&session), PASSWORD),
        None,
        "echoed while the application's core started"
    );
    session.wait_for(LIMIT, "the field", field_ready);
    assert!(
        session.screen().shows("0 characters, hidden"),
        "{}",
        session.screen().dump()
    );
    type_password(&mut session, PASSWORD);
    session.wait_for(LIMIT, "the session's model", |t| {
        shows_ever(t, crate::MODEL)
    });
    super::tui::quit(
        &mut session,
        &cineris_testkit::Phases::new("the application typed early"),
    );
    assert_eq!(
        machine.server.requests().len(),
        sent,
        "what was typed early was sent"
    );
}

/// What is typed while `permissions trust` holds the terminal, before its
/// question of the project's extensions, does not answer the question: a
/// `y` and `Enter` typed early, then `Enter` at the question, is no (the
/// full review of the signed approvals: the terminal's own line mode took
/// what was typed in raw mode as the start of the answer). Unix only, as
/// the tests of typing early above.
#[cfg(unix)]
#[test]
fn sys_a_yes_typed_before_the_question_does_not_answer_it() {
    let mut machine = Machine::new("approvals_yes_typed_early");
    machine.quick_dialogs();
    machine.person();
    probe_extension(&machine);
    let mut session = held_while_typing(&mut machine, &["permissions", "trust"], b"y\r");
    session.wait_for(LIMIT, "the question", |t| shows_ever(t, "[y/N]"));
    session.send(b"\r");
    type_password(&mut session, PASSWORD);
    let trusted = ended(&mut session);
    trusted.expect_code(0);
    assert!(
        trusted.screen.contains("left unapproved"),
        "{}",
        trusted.screen
    );
    let pinned = std::fs::read_to_string(machine.dirs().state.unwrap().join("trust.toml"))
        .unwrap_or_default();
    assert!(!pinned.contains("kind = \"extension\""), "{pinned}");
}

/// A project's extension, `probe`, which `permissions trust` asks about.
fn probe_extension(machine: &Machine) {
    let folder = machine.workspace.join(".cineris/extensions/probe");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"A test extension\"\nmain = \
         \"main.js\"\n",
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), "// nothing yet\n").unwrap();
}

/// Starts the binary with `arguments` at a terminal, held just before its
/// core starts, types `typed` then, and lets the core go.
#[cfg(unix)]
fn held_while_typing(machine: &mut Machine, arguments: &[&str], typed: &[u8]) -> Session {
    let hold = machine.home.join("hold");
    std::fs::create_dir_all(&hold).unwrap();
    for left in ["held", "go"] {
        let _ = std::fs::remove_file(hold.join(left));
    }
    machine.hold_the_core(&hold);
    let mut session = machine.tui(arguments);
    let deadline = std::time::Instant::now() + LIMIT;
    while !hold.join("held").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "not held before its core"
        );
        session.step(Duration::from_millis(20));
    }
    session.send(typed);
    while session.step(Duration::from_millis(200)).is_some() {}
    std::fs::write(hold.join("go"), b"").unwrap();
    session
}

/// An extension folder, `tracker`, under the machine's home: what `ext add`
/// is given.
fn tracker(machine: &Machine) -> String {
    let folder = machine.home.join("src").join("tracker");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"tracker\"\nversion = \"1\"\ndescription = \"Tracks.\"\nmain = \
         \"main.js\"\n",
    )
    .unwrap();
    std::fs::write(
        folder.join("main.js"),
        "cineris.tool({ name: 'look', description: 'Looks around.', input: { type: 'object' }, \
         async run() { return {}; } });\n",
    )
    .unwrap();
    folder.to_string_lossy().into_owned()
}

/// The hexadecimal digits after `before` in `text`.
fn hash_after(text: &str, before: &str) -> String {
    let at = text
        .find(before)
        .unwrap_or_else(|| panic!("{before}: {text}"));
    text[at + before.len()..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect()
}

fn pins(machine: &Machine) -> String {
    std::fs::read_to_string(machine.dirs().state.unwrap().join("pins.toml")).unwrap_or_default()
}

/// `ext add` at a terminal: once the extension is shown, the password is
/// the yes. One that is not the person's asks again and approves nothing;
/// theirs signs the pin. With no terminal it is refused, saying where to
/// run it, whatever hash is given.
#[cfg(feature = "scripts")]
#[test]
fn sys_ext_add_is_signed_with_the_password_at_a_terminal() {
    let mut machine = Machine::new("approvals_ext_add");
    machine.quick_dialogs();
    machine.person();
    let path = tracker(&machine);
    let piped = machine.run(&["ext", "add", &path]);
    piped.expect_code(2);
    let hash = hash_after(&piped.stdout, "(its hash is ");
    let hashed = machine.run(&["ext", "add", &path, "--hash", &hash]);
    hashed.expect_code(2);
    assert!(
        hashed.stderr.contains("in your own terminal")
            && hashed.stderr.contains("not an approval on its own"),
        "{}",
        shown(&hashed)
    );
    assert!(!pins(&machine).contains("tracker"), "{}", pins(&machine));

    let mut session = machine.tui(&["ext", "add", &path]);
    session.wait_for(LIMIT, "the prompt", |t| {
        shows_ever(t, "Type your approvals password to approve:")
    });
    type_password(&mut session, WRONG);
    session.wait_for(LIMIT, "asked again", |t| {
        shows_ever(t, "that is not the password of your approvals")
    });
    type_password(&mut session, PASSWORD);
    let added = ended(&mut session);
    added.expect_code(0);
    assert!(
        added.screen.contains("Approved `tracker`"),
        "{}",
        added.screen
    );
    assert!(
        pins(&machine).contains("tracker") && pins(&machine).contains("signature"),
        "{}",
        pins(&machine)
    );
    for typed in [PASSWORD, WRONG] {
        assert_eq!(holds_any_of(&added.raw, typed), None, "{typed} was written");
    }
}

/// `approvals reset` warns that every approval asks again, chooses a new
/// password twice, and says what it took the signature off. `status`
/// before and after reads what carries a signature, and checks nothing.
#[test]
fn sys_approvals_reset_and_status() {
    let mut machine = Machine::new("approvals_reset");
    machine.quick_dialogs();
    machine.person();
    machine.project_config("[model]\nmain = \"claude-opus-5-5\"\n");
    machine
        .signing(&["permissions", "trust"], &[], PASSWORD)
        .expect_code(0);
    let before = machine.run(&["approvals", "status"]);
    before.expect_code(0);
    assert!(
        before.stdout.contains("Approvals password: set, key")
            && before
                .stdout
                .contains("Projects' trust (trust.toml): 1 carrying a signature, not checked"),
        "{}",
        shown(&before)
    );

    let fresh = "a fresh password after a reset";
    let mut session = machine.tui(&["approvals", "reset"]);
    session.wait_for(LIMIT, "the warning", |t| {
        shows_ever(t, "Every approval you gave asks again")
    });
    type_password(&mut session, fresh);
    type_password(&mut session, fresh);
    let reset = ended(&mut session);
    reset.expect_code(0);
    assert!(
        reset.screen.contains("A new password is set")
            && reset
                .screen
                .contains("0 of your own pins and 1 projects' trust"),
        "{}",
        reset.screen
    );
    let after = machine.run(&["approvals", "status"]);
    after.expect_code(0);
    assert!(
        after
            .stdout
            .contains("Projects' trust (trust.toml): 0 carrying a signature, not checked; 1"),
        "{}",
        shown(&after)
    );
    // The old password no longer signs: it is asked again, and `Ctrl+C`
    // trusts nothing.
    let mut session = machine.tui(&["permissions", "trust"]);
    type_password(&mut session, PASSWORD);
    session.wait_for(LIMIT, "asked again", |t| {
        shows_ever(t, "that is not the password of your approvals")
    });
    session.wait_for(LIMIT, "the field", field_ready);
    session.send(b"\x03");
    let old = ended(&mut session);
    old.expect_code(2);
    assert!(
        old.screen.contains("the project was not trusted"),
        "{}",
        old.screen
    );
}

/// A session of a trusted project has approvals to use: the application
/// asks for the password after its first frame, and opens the session with
/// them (the project's model is used); `Esc` opens it without them (the
/// default model, and it says so), and nothing is asked about the project.
#[test]
fn sys_the_application_asks_for_the_password_when_a_session_has_approvals_to_use() {
    let mut machine = Machine::new("approvals_unlock");
    machine.quick_dialogs();
    machine.person();
    machine.project_config("[model]\nmain = \"claude-opus-5-5\"\n");
    machine
        .signing(&["permissions", "trust"], &[], PASSWORD)
        .expect_code(0);

    header_with_the_password(&machine, "claude-opus-5-5");

    let mut session = machine.tui(&[]);
    session.wait_for(LIMIT, "the question", |t| {
        t.shows("to use this session's approvals")
    });
    session.wait_for(LIMIT, "the field", field_ready);
    session.send_key("escape");
    session.wait_for(LIMIT, "opened without", |t| {
        shows_ever(t, "opened without your approvals")
    });
    session.wait_for(LIMIT, "the default model", |t| shows_ever(t, crate::MODEL));
    assert!(
        !session
            .screen()
            .history()
            .iter()
            .any(|row| row.contains("Trust this project?")),
        "asked about a project whose trust is there"
    );
    super::tui::quit(&mut session, &cineris_testkit::Phases::new("without"));
}

/// A headless run with standard output piped asks nothing: it uses no
/// approvals, and says so once on standard error, naming why; the project's
/// trust is not used (its model waits). At a terminal, the same run asks
/// for the password before its session opens, and uses it.
#[test]
fn sys_a_headless_run_with_no_terminal_asks_nothing_and_says_why() {
    let mut machine = Machine::new("approvals_headless");
    machine.quick_dialogs();
    machine.person();
    machine.project_config("[model]\nmain = \"claude-opus-5-5\"\n");
    machine
        .signing(&["permissions", "trust"], &[], PASSWORD)
        .expect_code(0);
    machine.reply_text("Piped.");
    let piped = machine.run(&["-p", "hi", "--output", "jsonl"]);
    piped.expect_code(0);
    let said = "no terminal to ask your approvals' password at: your extensions and this \
                project's trust are not used in this run";
    assert_eq!(piped.stderr.matches(said).count(), 1, "{}", shown(&piped));
    assert_eq!(crate::jsonl(&piped.stdout)[0]["model"], crate::MODEL);

    machine.reply_text("At a terminal.");
    let mut session = machine.tui(&["-p", "hi", "--output", "jsonl"]);
    session.wait_for(LIMIT, "the approvals' password", |t| {
        t.shows("to use this session's approvals")
    });
    type_password(&mut session, PASSWORD);
    let ended = ended(&mut session);
    ended.expect_code(0);
    assert!(
        ended.raw.contains("\"model\":\"claude-opus-5-5\""),
        "{}",
        ended.screen
    );
    assert!(!ended.screen.contains(said), "{}", ended.screen);

    // `Esc` at the password goes on without the approvals, and says so;
    // `Ctrl+C` ends the run as interrupted, its session never opened, and
    // nothing sent (the full review of the signed approvals: it went on
    // without them, as `Esc` does).
    let sent = machine.server.requests().len();
    machine.reply_text("Without.");
    let mut session = machine.tui(&["-p", "hi", "--output", "jsonl"]);
    session.wait_for(LIMIT, "the field", field_ready);
    session.send_key("escape");
    let without = crate::approvals::ended(&mut session);
    without.expect_code(0);
    assert!(
        without
            .raw
            .contains("this run goes on without your approvals"),
        "{}",
        without.screen
    );
    assert!(
        without
            .raw
            .contains(&format!("\"model\":\"{}\"", crate::MODEL)),
        "{}",
        without.screen
    );
    let sent = sent + 1;
    assert_eq!(machine.server.requests().len(), sent);
    let mut session = machine.tui(&["-p", "hi", "--output", "jsonl"]);
    session.wait_for(LIMIT, "the field", field_ready);
    session.send(b"\x03");
    let interrupted = crate::approvals::ended(&mut session);
    interrupted.expect_code(130);
    assert!(
        interrupted.raw.contains("\"outcome\":\"interrupted\""),
        "{}",
        interrupted.screen
    );
    assert!(
        !interrupted.raw.contains("\"type\":\"session\""),
        "a session opened: {}",
        interrupted.screen
    );
    assert_eq!(machine.server.requests().len(), sent, "a turn was sent");
}

/// No password is asked while a debugger is attached, at any prompt of a
/// subcommand or a headless run: `approvals setup` refuses before anything
/// is asked, `permissions trust` refuses at its password and trusts
/// nothing, and a headless run at a terminal goes on without the approvals,
/// saying why (the full review of the signed approvals). The debugger is
/// the test's file (`Machine::traced_while`), which the binary's
/// `harden::traced` takes for one in a build for tests.
#[test]
fn sys_no_password_is_asked_at_a_subcommand_or_a_run_while_a_debugger_is_attached() {
    let mut machine = Machine::new("approvals_traced");
    machine.quick_dialogs();
    let debugger = machine.home.join("debugger");
    machine.traced_while(&debugger);
    let said = "a debugger is attached to Cineris";
    std::fs::write(&debugger, b"").unwrap();

    // `approvals setup`, before anything else: it would otherwise say that
    // standard input is not a terminal.
    let setup = machine.run(&["approvals", "setup"]);
    setup.expect_code(2);
    assert!(setup.stderr.contains(said), "{}", shown(&setup));
    assert!(!machine.key_file().exists());

    // `permissions trust` at a terminal: no password asked, nothing trusted.
    machine.person();
    machine.project_config("[model]\nmain = \"claude-opus-5-5\"\n");
    let mut session = machine.tui(&["permissions", "trust"]);
    let refused = ended(&mut session);
    refused.expect_code(2);
    assert!(
        refused.raw.contains(said) && refused.raw.contains("the project was not trusted"),
        "{}",
        refused.screen
    );
    assert!(
        !refused.raw.contains("Type your approvals password"),
        "{}",
        refused.screen
    );
    let trust = std::fs::read_to_string(machine.dirs().state.unwrap().join("trust.toml"))
        .unwrap_or_default();
    assert!(!trust.contains("signature"), "{trust}");

    // A headless run at a terminal, with approvals to use: it goes on
    // without them, says why once, and asks nothing.
    std::fs::remove_file(&debugger).unwrap();
    machine
        .signing(&["permissions", "trust"], &[], PASSWORD)
        .expect_code(0);
    std::fs::write(&debugger, b"").unwrap();
    machine.reply_text("Ran.");
    let mut session = machine.tui(&["-p", "hi", "--output", "jsonl"]);
    let ran = ended(&mut session);
    ran.expect_code(0);
    assert_eq!(ran.raw.matches(said).count(), 1, "{}", ran.screen);
    assert!(
        !ran.raw.contains("to use this session's approvals"),
        "{}",
        ran.screen
    );
    assert!(
        ran.raw.contains(&format!("\"model\":\"{}\"", crate::MODEL)),
        "{}",
        ran.screen
    );
}

/// No password is asked in the application while a debugger is attached:
/// one attached before the session opens keeps the password's dialog from
/// opening, and the session opens without the approvals, saying why; one
/// attached while the dialog is open is refused by the call made with what
/// it read, at the open (`unlock`), at the trust question's yes (`sign`),
/// and at a yes that sets the password up (`set_up`, which then writes no
/// key file). The full review of the signed approvals: the application
/// checked only after its dialog had read the password.
#[test]
fn sys_the_application_asks_no_password_while_a_debugger_is_attached() {
    use super::tui::{TRUST_ANSWERS, quit, trust_asked};
    let mut machine = Machine::new("approvals_traced_app");
    machine.quick_dialogs();
    machine.person();
    machine.project_config("[model]\nmain = \"claude-opus-5-5\"\n");
    machine
        .signing(&["permissions", "trust"], &[], PASSWORD)
        .expect_code(0);
    let debugger = machine.home.join("debugger");
    machine.traced_while(&debugger);
    let phases = cineris_testkit::Phases::new("the application while a debugger is attached");

    // Attached before the session opens: no dialog.
    std::fs::write(&debugger, b"").unwrap();
    let mut session = machine.tui(&[]);
    session.wait_for(LIMIT, "opened without", |t| {
        shows_ever(t, "opened without your approvals")
    });
    assert!(
        shows_ever(session.screen(), "a debugger is attached"),
        "{}",
        session.screen().dump()
    );
    assert!(!shows_ever(
        session.screen(),
        "to use this session's approvals"
    ));
    quit(&mut session, &phases);

    // Attached while the dialog at the open is up: `unlock` refuses.
    std::fs::remove_file(&debugger).unwrap();
    let mut session = machine.tui(&[]);
    session.wait_for(LIMIT, "the field", field_ready);
    std::fs::write(&debugger, b"").unwrap();
    type_password(&mut session, PASSWORD);
    session.wait_for(LIMIT, "the open refused", |t| {
        shows_ever(t, "Your approvals were not opened: a debugger")
    });
    session.wait_for(LIMIT, "the default model", |t| shows_ever(t, crate::MODEL));
    quit(&mut session, &phases);

    // Attached while the trust question's password is asked: `sign`
    // refuses, and nothing is trusted.
    std::fs::remove_file(&debugger).unwrap();
    machine.run(&["permissions", "forget"]).expect_code(0);
    let mut session = machine.tui(&[]);
    trust_asked(&mut session);
    session.answer_dialog(LIMIT, TRUST_ANSWERS, b"y", |t| {
        t.shows("Type your approvals password to approve:")
    });
    session.wait_for(LIMIT, "the field", field_ready);
    std::fs::write(&debugger, b"").unwrap();
    type_password(&mut session, PASSWORD);
    session.wait_for(LIMIT, "the yes refused", |t| {
        shows_ever(t, "The project was not trusted: a debugger")
    });
    quit(&mut session, &phases);
    let trust = std::fs::read_to_string(machine.dirs().state.unwrap().join("trust.toml"))
        .unwrap_or_default();
    assert!(!trust.contains("signature"), "{trust}");

    // With no key set up, attached while the password is chosen: `set_up`
    // refuses, and no key file is written.
    std::fs::remove_file(&debugger).unwrap();
    std::fs::remove_file(machine.key_file()).unwrap();
    let mut session = machine.tui(&[]);
    trust_asked(&mut session);
    session.answer_dialog(LIMIT, TRUST_ANSWERS, b"y", |t| t.shows("Choose a password"));
    type_password(&mut session, PASSWORD);
    session.wait_for(LIMIT, "the second typing", |t| {
        t.shows("Type the same password again")
    });
    std::fs::write(&debugger, b"").unwrap();
    type_password(&mut session, PASSWORD);
    session.wait_for(LIMIT, "the yes refused", |t| {
        shows_ever(t, "The project was not trusted: a debugger")
    });
    quit(&mut session, &phases);
    assert!(!machine.key_file().exists(), "a key file was written");
}

/// Guard A over a session's commands: `cin approvals reset` (and setup) is
/// refused at every level, `unrestricted` here, and `cin approvals status`
/// runs. The model asks for each in turn; what it is told is in the next
/// request.
#[test]
fn sys_a_sessions_command_may_read_the_status_and_not_reset_the_password() {
    let machine = Machine::with_config(
        "approvals_guard_a",
        "[permissions]\nmode = \"unrestricted\"\n",
    );
    let binary = std::path::Path::new(crate::CINERIS).display().to_string();
    let line = |action: &str| {
        if cfg!(windows) {
            format!("& '{binary}' approvals {action}")
        } else {
            format!("'{binary}' approvals {action}")
        }
    };
    machine.reply_tool(
        "toolu_1",
        "shell",
        &serde_json::json!({"command": line("reset"), "timeout_ms": 60_000}),
    );
    machine.reply_tool(
        "toolu_2",
        "shell",
        &serde_json::json!({"command": line("status"), "timeout_ms": 60_000}),
    );
    machine.reply_text("Done.");
    let run = machine.run(&["-p", "look", "--output", "json"]);
    let requests = machine.server.requests();
    assert_eq!(requests.len(), 3, "{}", shown(&run));
    let told = |at: usize| String::from_utf8_lossy(&requests[at].body).into_owned();
    assert!(
        told(1).contains("Cineris refuses `cineris approvals reset`"),
        "{}",
        told(1)
    );
    assert!(
        told(2).contains("Approvals password: not set"),
        "{}",
        told(2)
    );
    assert!(!machine.key_file().exists());
}
