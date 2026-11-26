//! The first run (since step 25): each step through the scripted link and
//! a fake program, as the loop drives them. What each choice sends and
//! writes, that every step can be skipped, that nothing unpicked is
//! imported, that a key is never drawn, and that every listing it shows
//! holds hostile text.

use std::sync::Arc;
use std::time::{Duration, Instant};

use cineris_base::Secret;
use cineris_protocol::TurnId;
use cineris_protocol::event::{Event, TurnCompleted, TurnOutcome, TurnStarted};
use cineris_protocol::methods::{
    ConfigReloadResult, Disposition, Op, OpResult, ProjectDetectResult, SessionResult,
    TurnStartResult,
};
use cineris_protocol::project::{CommandPurpose, DetectedCommand};
use cineris_protocol::session::TrustStanding;

use super::{key, text, type_text};
use crate::app::setup::{
    Chosen, Found, Imported, KeyStore, McpFile, McpServer, Provider, Providers,
};
use crate::app::{Inbound, Open, Setup, Ticket};
use crate::testing::{TRICKY, problems};
use crate::testkit::{
    FakeSetup, Scripted, app_first_run, app_first_run_on, app_first_run_opening,
    app_first_run_prompted, approval, ask, drain, event, session,
};

const KEY: &str = "sk-ant-api03-Zq7XfirstrunKEYw9";

/// The model the program says it writes for `anthropic`.
const MODEL: &str = "claude-opus-5-5";

fn providers() -> Providers {
    Providers {
        providers: vec![
            Provider {
                name: "anthropic".to_owned(),
                variable: Some("ANTHROPIC_API_KEY".to_owned()),
                stored: false,
                model: Some(MODEL.to_owned()),
                model_shared: false,
            },
            Provider {
                name: "openai".to_owned(),
                variable: None,
                stored: false,
                model: None,
                model_shared: false,
            },
        ],
        store: KeyStore::File("~/.config/cineris/credentials.toml".to_owned()),
        sandbox: true,
    }
}

fn server(name: &str, shown: &str, refused: Option<&str>) -> McpServer {
    McpServer {
        name: name.to_owned(),
        shown: shown.to_owned(),
        hidden: false,
        env: Vec::new(),
        headers: Vec::new(),
        refused: refused.map(str::to_owned),
    }
}

fn fake() -> FakeSetup {
    let mut tracker = server("tracker", "https://tracker.example.com/mcp", None);
    tracker.headers = vec![(
        "Authorization".to_owned(),
        "TRACKER_AUTHORIZATION".to_owned(),
    )];
    let mut setup = FakeSetup {
        providers: providers(),
        found: Found {
            user_mcp: vec![
                McpFile {
                    path: "~/.claude.json".to_owned(),
                    from: "Claude Code".to_owned(),
                },
                McpFile {
                    path: "~/.cursor/mcp.json".to_owned(),
                    from: "Cursor".to_owned(),
                },
            ],
            project_mcp: vec![".mcp.json".to_owned()],
        },
        ..FakeSetup::default()
    };
    setup.servers.insert(
        "~/.claude.json".to_owned(),
        vec![
            server("docs", "npx -y @example/docs-mcp --root ~/src/docs", None),
            tracker,
            server(
                "github",
                "docker run -i --rm -e GITHUB_TOKEN=[redacted] ghcr.io/example/github-mcp:1.4",
                Some("holds a credential in its command"),
            ),
        ],
    );
    setup
}

/// A first run driven as the loop drives it.
struct Run {
    app: crate::app::App,
    link: Scripted,
    setup: FakeSetup,
    now: Instant,
    scrollback: Vec<String>,
    /// Every row drawn, committed or live, in every frame.
    drawn: Vec<String>,
}

impl Run {
    fn new(setup: FakeSetup) -> Self {
        Self::with(setup, 40, None)
    }

    /// A first run in a window of `rows`, the command line having given
    /// `prompt`.
    fn with(setup: FakeSetup, rows: usize, prompt: Option<&str>) -> Self {
        let now = Instant::now();
        let (app, link) = match prompt {
            Some(prompt) => app_first_run_prompted(now, &setup, 100, rows, prompt),
            None => app_first_run(now, &setup, 100, rows),
        };
        let mut run = Self {
            app,
            link,
            setup,
            now,
            scrollback: Vec::new(),
            drawn: Vec::new(),
        };
        run.frame();
        run.answer();
        run
    }

    /// A first run in a window of `columns` by `rows`.
    fn sized(setup: FakeSetup, columns: usize, rows: usize) -> Self {
        let now = Instant::now();
        let (app, link) = app_first_run(now, &setup, columns, rows);
        Self::begun(app, link, setup, now)
    }

    /// A first run that the command line asked to `open` a session of its
    /// own.
    fn opening(setup: FakeSetup, open: Open) -> Self {
        let now = Instant::now();
        let (app, link) = app_first_run_opening(now, &setup, 100, 40, open);
        Self::begun(app, link, setup, now)
    }

    /// Its first frame, and the program's first answer.
    fn begun(app: crate::app::App, link: Scripted, setup: FakeSetup, now: Instant) -> Self {
        let mut run = Self {
            app,
            link,
            setup,
            now,
            scrollback: Vec::new(),
            drawn: Vec::new(),
        };
        run.frame();
        run.answer();
        run
    }

    /// A first run on `program`, which keeps what it is asked in `setup`'s
    /// log, its first frame drawn and nothing of the program's taken.
    fn on(setup: FakeSetup, program: Arc<dyn Setup>) -> Self {
        let now = Instant::now();
        let (app, link) = app_first_run_on(now, program, 100, 40);
        let mut run = Self {
            app,
            link,
            setup,
            now,
            scrollback: Vec::new(),
            drawn: Vec::new(),
        };
        run.frame();
        run
    }

    /// A frame at the time now, drawn.
    fn frame(&mut self) -> Vec<String> {
        let (commit, live, _) = self.app.frame(self.now);
        assert_eq!(problems(&commit, 99), Vec::<String>::new());
        assert_eq!(problems(&live, 99), Vec::<String>::new());
        let commit = text(&commit);
        let live = text(&live);
        self.drawn.extend(commit.iter().cloned());
        self.drawn.extend(live.iter().cloned());
        self.scrollback.extend(commit);
        self.app.drawn(self.now);
        live
    }

    /// The step's first frame, then the time it takes keys.
    fn ready(&mut self) -> Vec<String> {
        self.now += Duration::from_secs(2);
        let _ = self.frame();
        self.now += Duration::from_millis(1500);
        let live = self.frame();
        assert_eq!(self.app.deadline(), None, "its keys are drawn");
        live
    }

    fn key(&mut self, binding: &str) {
        key(&mut self.app, binding, self.now);
        self.now += Duration::from_millis(50);
    }

    /// A key that moves the selection, and the frame the loop draws after
    /// it, as it draws one after every key: a key that accepts a row takes
    /// the row that frame drew.
    fn moves(&mut self, binding: &str) {
        self.key(binding);
        let _ = self.frame();
    }

    /// The worker's next answer, taken.
    fn answer(&mut self) {
        assert!(
            self.app.wait_for_setup(Duration::from_secs(10)),
            "the worker answered"
        );
    }

    /// The operation of `kind` the application sent, by its ticket.
    fn sent(&self, kind: fn(&Op) -> bool) -> Option<(Ticket, Op)> {
        self.link.take_sent().into_iter().find(|(_, op)| kind(op))
    }

    fn give(&mut self, ticket: Ticket, result: OpResult) {
        self.link.give(Inbound::Done(ticket, Ok(result)));
        drain(&mut self.app, self.now);
    }

    /// Answers the reload and the open, the session `trust`. Gives the
    /// configuration the session was opened with as its own.
    fn open(&mut self, trust: TrustStanding) -> serde_json::Map<String, serde_json::Value> {
        let (reload, _) = self
            .sent(|op| matches!(op, Op::ConfigReload(_)))
            .expect("the configuration read again");
        self.give(
            reload,
            OpResult::ConfigReload(ConfigReloadResult::default()),
        );
        self.opened(trust)
    }

    /// Answers the open, the session `trust`; the configuration it carried.
    fn opened(&mut self, trust: TrustStanding) -> serde_json::Map<String, serde_json::Value> {
        let (open, op) = self
            .sent(|op| matches!(op, Op::SessionNew(_)))
            .expect("the session opened once the file was written");
        let Op::SessionNew(params) = op else {
            unreachable!()
        };
        self.give(
            open,
            OpResult::SessionNew(SessionResult {
                session: session().trust(trust).build(),
            }),
        );
        params.config
    }

    fn written(&self) -> Vec<Chosen> {
        self.setup.log().written.clone()
    }

    /// Answers the project's detection the application sent with
    /// `commands`.
    fn detect(&mut self, commands: Vec<DetectedCommand>) {
        let (detect, _) = self
            .sent(|op| matches!(op, Op::ProjectDetect(_)))
            .expect("the project's commands asked for");
        self.give(
            detect,
            OpResult::ProjectDetect(ProjectDetectResult { commands }),
        );
    }

    /// The session's turn begins.
    fn turn_starts(&mut self) {
        self.link.give(event(Event::TurnStarted(TurnStarted {})));
        drain(&mut self.app, self.now);
    }

    /// The session's turn ends.
    fn turn_ends(&mut self) {
        self.link.give(event(Event::TurnCompleted(TurnCompleted {
            outcome: TurnOutcome::Completed,
            stop_reason: None,
            error: None,
            usage: Default::default(),
            verification: None,
            duration_ms: Some(10),
        })));
        drain(&mut self.app, self.now);
    }

    /// Whether the last frame `live` shows the commands' step.
    fn shows_commands(live: &[String]) -> bool {
        live.iter().any(|row| row == "◇ Commands  5 of 5")
    }
}

/// Two commands of a project's files.
fn two_commands() -> Vec<DetectedCommand> {
    vec![
        detected("build", "cargo build --workspace", None, "Cargo.toml"),
        detected("test", "cargo test --workspace", None, "Cargo.toml"),
    ]
}

/// To the imports' step, its file of servers read and its keys drawn
/// again after its rows came.
fn to_the_servers(run: &mut Run) {
    let _ = run.ready();
    run.key("s");
    run.answer();
    let _ = run.ready();
    run.moves("down");
    run.moves("down");
    run.key("space");
    run.answer();
    let _ = run.ready();
}

/// `s` at the provider (whose skip asks for what other tools left), at
/// the imports, at the skill folders and at the mode (whose skip writes the
/// file).
fn skip_three(run: &mut Run) {
    let _ = run.ready();
    run.key("s");
    run.answer();
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("s");
    run.answer();
}

/// `row` with each run of spaces made one, and none at its ends.
fn squash(row: &str) -> String {
    row.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn detected(name: &str, command: &str, runs: Option<&str>, source: &str) -> DetectedCommand {
    DetectedCommand {
        name: name.to_owned(),
        command: command.to_owned(),
        runs: runs.map(str::to_owned),
        source: source.to_owned(),
        purpose: CommandPurpose::Test,
    }
}

/// The whole first run, each step answered: a key from the environment,
/// one server picked, the mode, the session opened trusted, one command
/// kept. The file is written before the session opens, with what was
/// chosen; the command joins the profile as an entry the person wrote.
#[test]
fn the_first_run_reaches_a_session_made_with_what_was_chosen() {
    let mut run = Run::new(fake());
    let live = run.ready();
    assert!(
        live.iter().any(|row| row.contains("◆ Cineris 0.1.0")),
        "{live:#?}"
    );
    assert!(
        live.iter().any(|row| row == "◇ Provider  1 of 5"),
        "{live:#?}"
    );
    assert!(
        live.iter()
            .any(|row| squash(row) == "◆ anthropic its key from ANTHROPIC_API_KEY, as it is set"),
        "{live:#?}"
    );
    assert!(run.link.take_sent().is_empty(), "nothing is opened yet");
    run.key("enter");
    run.answer();
    let live = run.ready();
    assert!(
        live.iter().any(|row| row == "◇ Imports  2 of 5"),
        "{live:#?}"
    );
    // Down to the first file, read it, down to its first server, pick it.
    run.moves("down");
    run.moves("down");
    run.key("space");
    run.answer();
    assert_eq!(run.setup.log().read, ["~/.claude.json"]);
    // Its servers came: the step's grace starts again.
    let _ = run.ready();
    run.moves("down");
    run.key("space");
    // The refused server cannot be picked.
    run.moves("down");
    run.moves("down");
    run.key("space");
    let live = run.frame();
    assert!(live.iter().any(|row| row.contains("☑ docs")), "{live:#?}");
    run.key("enter");
    // The skill folders: no is preselected, and the question says plainly
    // what a yes sends (the owner's answer); here the person says yes.
    let live = run.ready();
    assert!(
        live.iter().any(|row| row == "◇ Skill folders  3 of 5"),
        "{live:#?}"
    );
    assert!(
        live.iter().any(|row| row.starts_with("  ◆ no")),
        "no is preselected: {live:#?}"
    );
    let asked = squash(&live.join(" "));
    assert!(
        asked.contains("secrets kept there included, is sent to the model's provider"),
        "{asked}"
    );
    run.moves("down");
    run.key("enter");
    let live = run.ready();
    assert!(live.iter().any(|row| row == "◇ Mode  4 of 5"), "{live:#?}");
    assert!(
        live.iter().any(|row| row.starts_with("  ◆ supervised")),
        "supervised is preselected: {live:#?}"
    );
    assert!(!live.iter().any(|row| row.contains("unrestricted")));
    run.key("enter");
    run.answer();
    assert_eq!(
        run.written(),
        [Chosen {
            provider: Some("anthropic".to_owned()),
            model: Some(MODEL.to_owned()),
            model_shared: false,
            imports: Some(Imported {
                instructions: false,
                skills: false,
                servers: vec![("~/.claude.json".to_owned(), "docs".to_owned())],
            }),
            mode: Some("supervised".to_owned()),
            read_skill_folders: true,
        }]
    );
    assert!(
        run.open(TrustStanding::Trusted).is_empty(),
        "the file written holds what was chosen"
    );
    let _ = run.frame();
    let (detect, _) = run
        .sent(|op| matches!(op, Op::ProjectDetect(_)))
        .expect("the project's commands asked for");
    run.give(
        detect,
        OpResult::ProjectDetect(ProjectDetectResult {
            commands: vec![
                detected("build", "cargo build --workspace", None, "Cargo.toml"),
                detected(
                    "test",
                    "npm run test",
                    Some("jest --coverage"),
                    "package.json",
                ),
            ],
        }),
    );
    let live = run.ready();
    assert!(
        live.iter().any(|row| row == "◇ Commands  5 of 5"),
        "{live:#?}"
    );
    assert!(
        live.iter()
            .any(|row| row.contains("☐ build") && row.contains("cargo build --workspace")),
        "{live:#?}"
    );
    assert!(
        live.iter().any(|row| row.trim() == "runs jest --coverage"),
        "{live:#?}"
    );
    assert!(
        !live.iter().any(|row| row.contains("Setting up")),
        "one card once the session is open: {live:#?}"
    );
    run.moves("down");
    run.key("space");
    run.key("enter");
    let (kept, update) = run
        .sent(|op| matches!(op, Op::ProjectUpdate(_)))
        .expect("the command kept");
    let Op::ProjectUpdate(update) = update else {
        unreachable!()
    };
    assert_eq!(update.add.len(), 1);
    let entry = &update.add[0];
    assert_eq!(
        entry
            .command
            .as_ref()
            .map(|command| command.command.as_str()),
        Some("npm run test")
    );
    assert!(
        entry.text.contains("found in package.json by the set-up"),
        "{}",
        entry.text
    );
    assert!(run.app.in_first_run(), "until the profile has it");
    run.give(
        kept,
        OpResult::ProjectUpdate(cineris_protocol::methods::ProjectEntriesResult {
            entries: Vec::new(),
            problems: Vec::new(),
        }),
    );
    let _ = run.frame();
    assert!(!run.app.in_first_run(), "it has ended");
    let said = squash(&run.scrollback.join(" "));
    for line in [
        "· Provider: anthropic, its key from ANTHROPIC_API_KEY.",
        "· Imports: the MCP server docs.",
        "· Skill folders: your own are read without asking, and what is read is sent to the \
         model's provider.",
        "· Mode: supervised. Written to ~/.config/cineris/config.toml.",
        "· Project: ~/src/parser trusted.",
        "· Commands: test kept in this project's profile.",
        "The model is anthropic's claude-opus-5-5 (model.main).",
        "~/.config/cineris/config.toml holds the settings, and a change there is kept.",
    ] {
        assert!(said.contains(line), "{line}: {said}");
    }
    assert!(said.contains("`cineris auth login <provider>`"), "{said}");
    assert!(
        said.contains("`/mode`") && said.contains("`/project`"),
        "{said}"
    );
    let cards = run
        .scrollback
        .iter()
        .filter(|row| row.contains("◆ Cineris"))
        .count();
    assert_eq!(cards, 1, "the set-up card is never committed: {said}");
}

/// `s` skips each step: nothing is stored, picked or kept, the file is
/// written with each step's comment, and the session opens all the same.
/// The skill folders' skip is no (the owner's answer).
#[test]
fn every_step_can_be_skipped() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    assert_eq!(run.written(), [Chosen::default()]);
    assert!(!run.written()[0].read_skill_folders);
    assert!(
        squash(&run.scrollback.join(" ")).contains(
            "· Skill folders: a read of your own is asked about. `cineris setup skill-folders` \
             changes it."
        ),
        "{:?}",
        run.scrollback
    );
    assert!(run.setup.log().stored.is_empty());
    assert!(run.setup.log().read.is_empty(), "no file was read");
    let _ = run.open(TrustStanding::Untrusted);
    let _ = run.frame();
    let (detect, _) = run.sent(|op| matches!(op, Op::ProjectDetect(_))).unwrap();
    run.give(
        detect,
        OpResult::ProjectDetect(ProjectDetectResult {
            commands: vec![detected("test", "cargo test", None, "Cargo.toml")],
        }),
    );
    let live = run.ready();
    assert!(
        live.iter()
            .any(|row| row.contains("not trusted, so a command still asks")),
        "{live:#?}"
    );
    run.key("s");
    assert!(run.sent(|op| matches!(op, Op::ProjectUpdate(_))).is_none());
    let _ = run.frame();
    assert!(!run.app.in_first_run());
    assert!(run.scrollback.join("\n").contains("· Commands: none kept."));
}

/// `x` skips the whole set-up: the file is written with the defaults, the
/// session opens, and no command is asked for.
#[test]
fn x_skips_the_whole_set_up() {
    let mut run = Run::new(fake());
    let _ = run.ready();
    run.key("x");
    run.answer();
    assert_eq!(run.written(), [Chosen::default()]);
    let _ = run.open(TrustStanding::Trusted);
    let _ = run.frame();
    let _ = run.frame();
    assert!(run.sent(|op| matches!(op, Op::ProjectDetect(_))).is_none());
    assert!(!run.app.in_first_run());
    let said = run.scrollback.join("\n");
    assert!(said.contains("· Set-up skipped."), "{said}");
    assert!(
        squash(&said).contains(
            "No provider was chosen, so there is no model yet: model.main names one once a key \
             is stored."
        ),
        "{said}"
    );
    assert!(
        squash(&said).contains("~/.config/cineris/config.toml holds the settings"),
        "{said}"
    );
}

/// A prompt typed at the first frame, and an `Enter` held from the step
/// before, change nothing and store nothing: a step takes no key before
/// its grace, nor until a quiet second after input it did not take.
#[test]
fn a_prompt_typed_at_the_first_frame_chooses_nothing() {
    let mut run = Run::new(fake());
    type_text(&mut run.app, "fix the build", run.now);
    key(&mut run.app, "enter", run.now);
    key(&mut run.app, "x", run.now);
    key(&mut run.app, "s", run.now + Duration::from_millis(900));
    let live = run.frame();
    assert!(
        live.iter().any(|row| row == "◇ Provider  1 of 5"),
        "{live:#?}"
    );
    assert!(run.written().is_empty() && run.setup.log().stored.is_empty());
    // The step answered, an `Enter` at once does not answer the next.
    let _ = run.ready();
    run.key("enter");
    run.answer();
    run.now += Duration::from_millis(100);
    let _ = run.frame();
    run.key("enter");
    let live = run.frame();
    assert!(
        live.iter().any(|row| row == "◇ Imports  2 of 5"),
        "{live:#?}"
    );
}

/// A step's keys are drawn at the end of its grace, when the loop is due to
/// draw, and it takes none before: a key before then holds it a quiet
/// second more, its keys hidden again.
#[test]
fn a_steps_keys_are_drawn_at_the_end_of_its_grace_and_taken_only_then() {
    let mut run = Run::new(fake());
    let live = run.frame();
    assert!(
        !live.iter().any(|row| row.contains("enter chooses")),
        "{live:#?}"
    );
    let due = run.app.deadline().expect("the step's keys are due");
    assert!(due > run.now, "not before its grace");
    run.now = due - Duration::from_millis(100);
    run.key("down");
    let later = run.app.deadline().expect("due again");
    assert!(
        later >= due + Duration::from_millis(900),
        "a quiet second after the key"
    );
    run.now = later;
    let live = run.frame();
    assert!(
        live.iter().any(|row| row.contains("enter chooses")),
        "{live:#?}"
    );
    assert_eq!(run.app.deadline(), None);
    run.key("enter");
    assert_eq!(
        squash(&run.scrollback.join(" ")),
        "",
        "the first row was chosen at the next frame, not the second: the down was dropped"
    );
    let _ = run.frame();
    assert!(
        squash(&run.scrollback.join(" ")).contains("its key from ANTHROPIC_API_KEY"),
        "{:#?}",
        run.scrollback
    );
}

/// A key typed or pasted is never drawn: no row of any frame holds any of
/// it. A paste with a line break, or one too long, is refused whole;
/// `Ctrl+U` empties it; `Enter` stores it in the store.
#[test]
fn a_key_typed_is_never_drawn_and_goes_only_to_the_store() {
    let mut setup = fake();
    setup.providers.providers[0].variable = None;
    let mut run = Run::new(setup);
    let live = run.ready();
    assert!(
        live.iter()
            .any(|row| row.starts_with("  ◆ anthropic") && row.ends_with("store a key")),
        "{live:#?}"
    );
    run.key("enter");
    let live = run.ready();
    assert!(
        live.iter()
            .any(|row| row.contains("Paste or type the key for anthropic")),
        "{live:#?}"
    );
    assert!(live.iter().any(|row| row.contains("0 characters, hidden")));
    type_text(&mut run.app, "junk", run.now);
    run.key("ctrl+u");
    run.app.terminal(
        &cineris_term::Event::Paste("half\nof it".to_owned()),
        run.now,
    );
    let live = run.frame();
    assert!(
        live.iter().any(|row| row.contains("line break")),
        "{live:#?}"
    );
    assert!(
        live.iter().any(|row| row.contains("0 characters, hidden")),
        "{live:#?}"
    );
    run.app
        .terminal(&cineris_term::Event::Paste(KEY[..10].to_owned()), run.now);
    type_text(&mut run.app, &KEY[10..], run.now);
    let live = run.frame();
    assert!(
        live.iter()
            .any(|row| row.contains(&format!("{} characters, hidden", KEY.len()))),
        "{live:#?}"
    );
    run.key("enter");
    run.answer();
    assert_eq!(
        run.setup.log().stored,
        [("anthropic".to_owned(), KEY.to_owned())]
    );
    let _ = run.ready();
    let all = run.drawn.join("\n");
    for piece in [
        &KEY[..6],
        &KEY[10..16],
        &KEY[KEY.len() - 6..],
        "junk",
        "half",
    ] {
        assert!(!all.contains(piece), "{piece} was drawn");
    }
    assert!(
        squash(&run.scrollback.join(" ")).contains(
            "· Provider: anthropic, its key stored in ~/.config/cineris/credentials.toml, \
             which only you can read."
        ),
        "{:#?}",
        run.scrollback
    );
}

/// A reply of the terminal's split after its ESC, as a pseudo-console
/// splits one on a busy machine, comes from the terminal layer as the
/// reply while the replies are due: none of it goes into the key, and its
/// ESC does not leave the step (a review of step 25; small fixes II).
#[test]
fn a_reply_split_after_its_escape_never_goes_into_the_key() {
    use cineris_term::Ready;
    let mut setup = fake();
    setup.providers.providers[0].variable = None;
    let mut run = Run::new(setup);
    let _ = run.ready();
    run.key("enter");
    let _ = run.ready();
    // The reply's ESC, then nothing for 600 ms, then its rest.
    let device = super::Played {
        steps: [
            (Ready::Input, b"\x1b".to_vec()),
            (Ready::Timeout, Vec::new()),
            (Ready::Input, b"[?0u".to_vec()),
        ]
        .into_iter()
        .collect(),
        written: Arc::new(std::sync::Mutex::new(Vec::new())),
        waits: Arc::new(std::sync::Mutex::new(Vec::new())),
        base: run.now,
        elapsed: Duration::ZERO,
    };
    let mut terminal =
        cineris_term::Terminal::with_device(device, crate::testkit::caps()).expect("the terminal");
    let mut events = Vec::new();
    for _ in 0..2 {
        let read = terminal.next(Some(Duration::from_millis(600)));
        events.extend(read.expect("a read"));
    }
    for event in &events {
        run.app.terminal(event, run.now);
    }
    assert_eq!(
        events,
        [cineris_term::Event::Reply(cineris_term::Reply::Keyboard(0))]
    );
    let live = run.frame();
    assert!(
        live.iter()
            .any(|row| row.contains("Paste or type the key for anthropic")),
        "{live:#?}"
    );
    assert!(
        live.iter().any(|row| row.contains("0 characters, hidden")),
        "{live:#?}"
    );
}

/// An `Esc` the terminal held, pressed in the key step's grace and read
/// once the step takes keys, does not go back: a step judges a key by when
/// it was pressed (the full review of small fixes II).
#[test]
fn an_escape_pressed_in_the_key_steps_grace_and_read_later_does_not_go_back() {
    let mut setup = fake();
    setup.providers.providers[0].variable = None;
    let mut run = Run::new(setup);
    let _ = run.ready();
    run.key("enter");
    run.now += Duration::from_secs(2);
    let _ = run.frame();
    let pressed = run.now + Duration::from_millis(100);
    run.now += Duration::from_millis(1500);
    let live = run.frame();
    assert!(
        live.iter().any(|row| row.contains("0 characters, hidden")),
        "{live:#?}"
    );
    let esc = cineris_term::Event::Key(crate::keys::parse("esc").expect("a key"));
    run.app.terminal_pressed(&esc, pressed, run.now);
    let live = run.frame();
    assert!(
        live.iter()
            .any(|row| row.contains("Paste or type the key for anthropic")),
        "{live:#?}"
    );
}

/// A store that refuses says why and keeps the step; `Esc` goes back to
/// the providers with nothing kept.
#[test]
fn a_store_that_refuses_keeps_the_step_and_esc_goes_back() {
    let mut setup = fake();
    setup.store_fails = Some("the disk is full".to_owned());
    let mut run = Run::new(setup);
    let _ = run.ready();
    run.moves("down");
    run.key("enter");
    let _ = run.ready();
    type_text(&mut run.app, KEY, run.now);
    run.key("enter");
    run.answer();
    let live = run.frame();
    assert!(
        live.iter()
            .any(|row| row.contains("could not be stored: the disk is full")),
        "{live:#?}"
    );
    run.key("esc");
    let live = run.ready();
    assert!(
        live.iter()
            .any(|row| row.contains("its key from ANTHROPIC_API_KEY"))
    );
    assert!(!run.drawn.join("\n").contains(&KEY[8..20]));
}

/// Nothing found is imported unpicked: a file read and left unpicked
/// brings in nothing, and the instructions and skills stay off.
#[test]
fn nothing_unpicked_is_imported() {
    let mut run = Run::new(fake());
    let _ = run.ready();
    run.key("s");
    run.answer();
    let live = run.ready();
    assert!(
        live.iter()
            .any(|row| row.contains("☐ AGENTS.md, CLAUDE.md")),
        "unpicked: {live:#?}"
    );
    assert!(
        live.iter().any(|row| row.contains("off unless picked")),
        "{live:#?}"
    );
    run.moves("down");
    run.moves("down");
    run.key("space");
    run.answer();
    let _ = run.ready();
    run.key("enter");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("enter");
    run.answer();
    assert_eq!(
        run.written()[0].imports,
        Some(Imported::default()),
        "a file read is not a pick"
    );
}

/// An MCP server's command is shown whole, wrapped and never cut, its
/// credential masked; its headers' variables are named.
#[test]
fn an_mcp_command_is_shown_whole_and_its_credential_masked() {
    let mut setup = fake();
    let long = format!(
        "npx -y @example/docs-mcp --root ~/src/docs {}",
        (1..=12)
            .map(|n| format!("--flag-{n}=value-{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    setup.servers.get_mut("~/.claude.json").unwrap()[0].shown = long.clone();
    let mut run = Run::new(setup);
    let _ = run.ready();
    run.key("s");
    run.answer();
    let _ = run.ready();
    run.moves("down");
    run.moves("down");
    run.key("space");
    run.answer();
    let live = run.frame();
    let joined = live.join(" ");
    for word in long.split(' ') {
        assert!(joined.contains(word), "{word} was cut: {live:#?}");
    }
    assert!(joined.contains("GITHUB_TOKEN=[redacted]"), "{live:#?}");
    assert!(joined.contains("holds a credential in its command: not imported"));
    assert!(joined.contains("its Authorization header comes from TRACKER_AUTHORIZATION"));
}

/// Every listing the first run shows holds hostile text drawn visible and
/// within the window: providers, files, servers and their commands, and
/// the commands found.
#[test]
fn every_listing_holds_hostile_text() {
    for (index, tricky) in TRICKY.iter().enumerate() {
        let hostile = format!("{tricky}x\u{202e}y{}", TRICKY[(index + 5) % TRICKY.len()]);
        let mut setup = fake();
        setup.providers.providers[0].name = hostile.clone();
        setup.providers.providers[0].variable = Some(hostile.clone());
        setup.found.user_mcp[0].path = hostile.clone();
        setup.found.user_mcp[0].from = hostile.clone();
        setup.found.project_mcp = vec![hostile.clone()];
        let mut server = server(&hostile, &hostile.repeat(5), Some(&hostile));
        server.env = vec![hostile.clone()];
        server.headers = vec![(hostile.clone(), hostile.clone())];
        setup.servers.insert(hostile.clone(), vec![server]);
        let mut run = Run::new(setup);
        let _ = run.ready();
        run.key("enter");
        run.answer();
        let _ = run.ready();
        run.moves("down");
        run.moves("down");
        run.key("space");
        run.answer();
        let _ = run.ready();
        run.key("enter");
        let _ = run.ready();
        run.key("s");
        let _ = run.ready();
        run.key("enter");
        run.answer();
        let _ = run.open(TrustStanding::Untrusted);
        let _ = run.frame();
        let (detect, _) = run.sent(|op| matches!(op, Op::ProjectDetect(_))).unwrap();
        run.give(
            detect,
            OpResult::ProjectDetect(ProjectDetectResult {
                commands: vec![detected(&hostile, &hostile, Some(&hostile), &hostile)],
            }),
        );
        let _ = run.ready();
        run.key("e");
        type_text(&mut run.app, &hostile, run.now);
        let _ = run.frame();
        // `Run::frame` checks every row of every frame.
    }
}

/// `autonomous` is offered only where the sandbox enforces, and
/// `unrestricted` never.
#[test]
fn autonomous_is_offered_only_where_the_sandbox_enforces() {
    for sandbox in [true, false] {
        let mut setup = fake();
        setup.providers.sandbox = sandbox;
        let mut run = Run::new(setup);
        let _ = run.ready();
        run.key("s");
        run.answer();
        let _ = run.ready();
        run.key("s");
        let _ = run.ready();
        run.key("s");
        let live = run.ready();
        let levels: Vec<String> = live
            .iter()
            .map(|row| squash(row))
            .filter_map(|row| {
                let row = row.trim_start_matches("◆ ");
                ["read-only", "supervised", "autonomous", "unrestricted"]
                    .iter()
                    .find(|level| row.starts_with(*level))
                    .map(|level| (*level).to_owned())
            })
            .collect();
        let mut expected = vec!["read-only", "supervised"];
        if sandbox {
            expected.push("autonomous");
        }
        assert_eq!(levels, expected, "{live:#?}");
        // The last offered, chosen.
        run.moves("down");
        run.key("enter");
        run.answer();
        assert_eq!(
            run.written()[0].mode.as_deref(),
            Some(*expected.last().unwrap())
        );
    }
}

/// `Ctrl+C` twice leaves the program during the first run, nothing
/// written.
#[test]
fn ctrl_c_twice_leaves_with_nothing_written() {
    let mut run = Run::new(fake());
    let _ = run.ready();
    run.key("ctrl+c");
    let live = run.frame();
    assert!(live.iter().any(|row| row.contains("ctrl+c")), "{live:#?}");
    run.key("ctrl+c");
    assert!(run.app.exit().is_some());
    assert!(run.written().is_empty());
    assert!(run.link.take_sent().is_empty(), "nothing opened");
}

/// After the project's commands come back empty the first run ends, said.
#[test]
fn a_project_with_no_commands_ends_the_first_run() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Untrusted);
    let _ = run.frame();
    let (detect, _) = run.sent(|op| matches!(op, Op::ProjectDetect(_))).unwrap();
    run.give(
        detect,
        OpResult::ProjectDetect(ProjectDetectResult {
            commands: Vec::new(),
        }),
    );
    let _ = run.frame();
    assert!(!run.app.in_first_run());
    let said = run.scrollback.join(
        "
",
    );
    assert!(
        said.contains("· Commands: none found in this project's files."),
        "{said}"
    );
    assert!(said.contains("· Set-up done."), "{said}");
}

// ------------------------------------------------ the full review of the
// third pull request: how keys reach the steps

/// The first start given a prompt (`cineris "fix the build"`): the prompt
/// goes at the open, and the project's commands are asked for, and their
/// step shown, only once its turn has ended; until then the input box is
/// the input box.
#[test]
fn the_commands_wait_for_the_turn_a_prompt_on_the_command_line_began() {
    let mut run = Run::with(fake(), 40, Some("fix the build"));
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Trusted);
    let _ = run.frame();
    let sent = run.link.take_sent();
    let (turn, _) = sent
        .iter()
        .find(|(_, op)| matches!(op, Op::TurnStart(_)))
        .expect("the prompt sent at the open");
    assert!(
        !sent
            .iter()
            .any(|(_, op)| matches!(op, Op::ProjectDetect(_))),
        "the commands are not asked for while its turn runs"
    );
    let turn = *turn;
    run.give(
        turn,
        OpResult::TurnStart(TurnStartResult {
            turn_id: TurnId::from("t_1"),
            disposition: Disposition::Started,
        }),
    );
    run.turn_starts();
    let live = run.ready();
    assert!(!Run::shows_commands(&live), "{live:#?}");
    assert!(run.sent(|op| matches!(op, Op::ProjectDetect(_))).is_none());
    run.turn_ends();
    let _ = run.frame();
    run.detect(two_commands());
    let live = run.ready();
    assert!(Run::shows_commands(&live), "{live:#?}");
}

/// The commands' step shown, a turn begins (another client's) and a
/// dialog of its comes: the step is not shown while either is, and no key
/// reaches it: `x` under the turn, `space` and `enter` under the dialog
/// go where they would without it. Once the session is idle again the
/// step is shown again, its grace begun anew, nothing picked.
#[test]
fn the_commands_step_takes_no_key_while_a_turn_or_a_dialog_is_shown() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Trusted);
    let _ = run.frame();
    run.detect(two_commands());
    let live = run.ready();
    assert!(Run::shows_commands(&live), "{live:#?}");
    run.turn_starts();
    let live = run.ready();
    assert!(!Run::shows_commands(&live), "{live:#?}");
    run.key("x");
    assert!(run.app.in_first_run(), "x reached the input box");
    ask(&mut run.app, &run.link, 7, approval("c_1"), run.now);
    let live = run.ready();
    assert!(
        live.iter().any(|row| row.contains("Allow once")),
        "the dialog, its answers drawn: {live:#?}"
    );
    run.key("space");
    run.key("enter");
    assert!(
        run.sent(|op| matches!(op, Op::ProjectUpdate(_))).is_none(),
        "no command kept"
    );
    assert!(run.app.in_first_run());
    // The turn's end closes its dialog; what the keys typed is taken out
    // of the input box.
    run.turn_ends();
    run.key("ctrl+u");
    let live = run.frame();
    assert!(Run::shows_commands(&live), "{live:#?}");
    assert!(
        !live.iter().any(|row| row.contains("enter keeps")),
        "its grace begun anew: {live:#?}"
    );
    let live = run.ready();
    assert_eq!(
        live.iter().filter(|row| row.contains('☐')).count(),
        2,
        "nothing picked: {live:#?}"
    );
}

/// Typing into the input box when the commands come: the step waits while
/// the input box holds text, and once it is empty holds a quiet second
/// after the last key, as a dialog would: past its grace, and within that
/// second, its keys are not drawn and `space` and `enter` keep nothing.
#[test]
fn typing_just_before_a_step_comes_holds_it_a_quiet_second() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Trusted);
    let _ = run.frame();
    let (detect, _) = run
        .sent(|op| matches!(op, Op::ProjectDetect(_)))
        .expect("the project's commands asked for");
    // Long after the mode's key, which holds what comes next no more.
    run.now += Duration::from_secs(5);
    let typed = run.now;
    type_text(&mut run.app, "a", typed);
    run.give(
        detect,
        OpResult::ProjectDetect(ProjectDetectResult {
            commands: two_commands(),
        }),
    );
    let live = run.frame();
    assert!(
        !Run::shows_commands(&live),
        "not over what is being typed: {live:#?}"
    );
    // The last key leaves the input box empty.
    key(&mut run.app, "backspace", typed);
    run.now = typed + Duration::from_millis(10);
    let live = run.frame();
    assert!(Run::shows_commands(&live), "{live:#?}");
    run.now = typed + Duration::from_millis(600);
    let live = run.frame();
    assert!(
        !live.iter().any(|row| row.contains("space picks")),
        "a quiet second after the typing: {live:#?}"
    );
    run.key("space");
    run.key("enter");
    assert!(run.sent(|op| matches!(op, Op::ProjectUpdate(_))).is_none());
    let live = run.ready();
    assert!(!live.iter().any(|row| row.contains('☑')), "{live:#?}");
}

/// The key that answered a dialog just before the commands' step is shown
/// holds the step a key's repeat window, as it would hold the next
/// dialog: past its grace, and within that window, `space` and `enter`
/// keep nothing.
#[test]
fn the_key_that_answered_a_dialog_holds_the_step_after_it() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Trusted);
    // A question comes (another client's turn's, say), and the commands
    // found come while it is up: the step waits under it.
    ask(&mut run.app, &run.link, 7, approval("c_1"), run.now);
    run.detect(two_commands());
    let live = run.ready();
    assert!(!Run::shows_commands(&live), "{live:#?}");
    assert!(
        live.iter().any(|row| row.contains("Allow once")),
        "{live:#?}"
    );
    let answered = run.now;
    run.key("y");
    assert_eq!(run.link.script().answers.len(), 1, "the dialog answered");
    let live = run.frame();
    assert!(Run::shows_commands(&live), "{live:#?}");
    run.now = answered + Duration::from_millis(600);
    let live = run.frame();
    assert!(
        !live.iter().any(|row| row.contains("space picks")),
        "a key's repeat window after the answer: {live:#?}"
    );
    run.key("space");
    run.key("enter");
    assert!(run.sent(|op| matches!(op, Op::ProjectUpdate(_))).is_none());
    let live = run.ready();
    assert!(!live.iter().any(|row| row.contains('☑')), "{live:#?}");
}

/// The other way: the key that answered the mode holds the session's trust
/// question that comes just after it a key's repeat window, as a dialog's
/// answer would, so that the `enter` held down, or a `y` typed on, does
/// not answer it past its grace.
#[test]
fn the_key_that_answered_a_step_holds_the_dialog_after_it() {
    let mut run = Run::new(fake());
    let _ = run.ready();
    run.key("s");
    run.answer();
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    let chosen = run.now;
    run.key("enter");
    run.answer();
    let (reload, _) = run
        .sent(|op| matches!(op, Op::ConfigReload(_)))
        .expect("the configuration read again");
    run.give(
        reload,
        OpResult::ConfigReload(ConfigReloadResult::default()),
    );
    // The session's own trust question, as it opens.
    let shown = chosen + Duration::from_millis(100);
    ask(
        &mut run.app,
        &run.link,
        9,
        super::trust_request(Vec::new()),
        shown,
    );
    let past_its_grace = shown + Duration::from_millis(350);
    let _ = run.app.frame(past_its_grace);
    run.app.drawn(past_its_grace);
    key(
        &mut run.app,
        "y",
        past_its_grace + Duration::from_millis(50),
    );
    assert!(
        run.link.script().answers.is_empty(),
        "a key's repeat window after the mode's"
    );
}

/// A file's servers that come while the imports' step is ready start its
/// grace again: a key read as they are drawn picks nothing it had not
/// shown.
#[test]
fn rows_that_come_under_a_ready_step_start_its_grace_again() {
    let mut run = Run::new(fake());
    let _ = run.ready();
    run.key("s");
    run.answer();
    let _ = run.ready();
    run.moves("down");
    run.moves("down");
    run.key("space");
    run.answer();
    let live = run.frame();
    assert!(live.iter().any(|row| row.contains("docs")), "{live:#?}");
    assert!(
        !live.iter().any(|row| row.contains("enter keeps")),
        "its keys wait for its grace: {live:#?}"
    );
    run.moves("down");
    run.key("space");
    let live = run.ready();
    assert!(live.iter().any(|row| row.contains("☐ docs")), "{live:#?}");
    assert!(!live.iter().any(|row| row.contains('☑')), "{live:#?}");
}

/// [`FakeSetup`], its providers and what other tools left held until the
/// test opens their gates: a step is ready, reading, when they come.
struct Held {
    setup: FakeSetup,
    providers: cineris_testkit::Gate,
    found: cineris_testkit::Gate,
}

impl Setup for Held {
    fn providers(&self) -> Result<Providers, String> {
        assert!(self.providers.wait(Duration::from_secs(10)), "let go");
        self.setup.providers()
    }

    fn store_key(&self, provider: &str, key: Secret) -> Result<(), String> {
        self.setup.store_key(provider, key)
    }

    fn discover(&self) -> Found {
        assert!(self.found.wait(Duration::from_secs(10)), "let go");
        self.setup.discover()
    }

    fn read_servers(&self, path: &str) -> Result<Vec<McpServer>, String> {
        self.setup.read_servers(path)
    }

    fn write_config(&self, chosen: &Chosen) -> Result<String, crate::app::setup::NotWritten> {
        self.setup.write_config(chosen)
    }
}

/// The providers, and what other tools left, that come while their step
/// is ready, reading, start its grace again: a key read as they are drawn
/// chooses nothing and moves nothing.
#[test]
fn the_providers_and_what_was_found_start_the_grace_again() {
    let setup = fake();
    let held = Held {
        setup: setup.clone(),
        providers: cineris_testkit::Gate::new(),
        found: cineris_testkit::Gate::new(),
    };
    let (providers, found) = (held.providers.clone(), held.found.clone());
    let mut run = Run::on(setup, Arc::new(held));
    let live = run.ready();
    assert!(
        live.iter().any(|row| row.contains("s skips this step")),
        "ready, reading: {live:#?}"
    );
    providers.open();
    run.answer();
    let live = run.frame();
    assert!(
        live.iter().any(|row| row.contains("ANTHROPIC_API_KEY")),
        "{live:#?}"
    );
    assert!(
        !live.iter().any(|row| row.contains("enter chooses")),
        "its keys wait for its grace: {live:#?}"
    );
    run.key("enter");
    let live = run.frame();
    assert!(run.scrollback.is_empty(), "nothing chosen: {live:#?}");
    assert!(
        live.iter().any(|row| row == "◇ Provider  1 of 5"),
        "{live:#?}"
    );
    let _ = run.ready();
    run.key("enter");
    let live = run.ready();
    assert!(
        live.iter().any(|row| row == "◇ Imports  2 of 5"),
        "{live:#?}"
    );
    found.open();
    run.answer();
    let live = run.frame();
    assert!(
        live.iter().any(|row| row.contains("~/.claude.json")),
        "{live:#?}"
    );
    assert!(
        !live.iter().any(|row| row.contains("enter keeps")),
        "its keys wait for its grace: {live:#?}"
    );
    run.moves("down");
    let live = run.ready();
    assert!(
        live.iter().any(|row| row.starts_with("  ◆ ☐ AGENTS.md")),
        "the selection did not move: {live:#?}"
    );
}

/// The imports' selection is the row it is: the first file's servers,
/// which come after the person moved on to the second file, are inserted
/// above it, and `space` still reads the second file, not the first
/// server that came.
#[test]
fn a_files_answer_does_not_move_the_selection_onto_its_servers() {
    let mut setup = fake();
    setup.servers.insert(
        "~/.cursor/mcp.json".to_owned(),
        vec![server("search", "npx -y @example/search-mcp", None)],
    );
    let mut run = Run::new(setup);
    let _ = run.ready();
    run.key("s");
    run.answer();
    let _ = run.ready();
    run.moves("down");
    run.moves("down");
    run.key("space");
    // To the second file before the first's answer comes.
    run.moves("down");
    run.answer();
    let _ = run.ready();
    run.key("space");
    run.answer();
    assert_eq!(
        run.setup.log().read,
        ["~/.claude.json", "~/.cursor/mcp.json"]
    );
    let live = run.ready();
    assert!(live.iter().any(|row| row.contains("search")), "{live:#?}");
    assert!(!live.iter().any(|row| row.contains('☑')), "{live:#?}");
}

/// In a window of 24 rows, a file of eight servers: every frame fits the
/// window and keeps the step's title; the selected row is drawn whole as
/// it moves down the list, a dim row saying what is above and below; and
/// the last server, picked, is the one written.
#[test]
fn in_a_short_window_the_step_keeps_its_title_and_the_selected_row_whole() {
    let mut setup = fake();
    let command = |n: usize| {
        format!("npx -y @example/server-{n}-mcp --root ~/src/project-{n} --port 70{n:02} --verbose")
    };
    setup.servers.insert(
        "~/.claude.json".to_owned(),
        (1..=8)
            .map(|n| server(&format!("server-{n}"), &command(n), None))
            .collect(),
    );
    let mut run = Run::with(setup, 24, None);
    to_the_servers(&mut run);
    for n in 1..=8 {
        run.moves("down");
        let live = run.frame();
        assert!(live.len() <= 24, "{} rows: {live:#?}", live.len());
        assert!(
            live.iter().any(|row| row == "◇ Imports  2 of 5"),
            "{live:#?}"
        );
        let selected = live
            .iter()
            .position(|row| row.starts_with("  ◆"))
            .expect("the selected row is drawn");
        assert!(
            live[selected].contains(&format!("server-{n} ")),
            "{live:#?}"
        );
        assert!(
            squash(&live[selected..].join(" ")).contains(&command(n)),
            "server-{n}'s command whole: {live:#?}"
        );
        if n == 1 {
            assert!(
                live.iter().any(|row| row.contains("more below")),
                "{live:#?}"
            );
        }
        if n == 8 {
            assert!(
                live.iter().any(|row| row.contains("more above")),
                "{live:#?}"
            );
        }
    }
    run.key("space");
    run.key("enter");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("enter");
    run.answer();
    assert_eq!(
        run.written()[0]
            .imports
            .as_ref()
            .map(|imported| imported.servers.clone()),
        Some(vec![("~/.claude.json".to_owned(), "server-8".to_owned())])
    );
}

/// A server whose command is taller than the window is cut, the hint row
/// says a larger window shows it whole, and `space` does not pick it:
/// what is accepted has been drawn whole.
#[test]
fn a_row_the_window_cannot_show_whole_is_not_accepted() {
    let mut setup = fake();
    setup.servers.get_mut("~/.claude.json").unwrap()[0].shown = (1..=60)
        .map(|n| format!("--flag-{n}=value-{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut run = Run::with(setup, 16, None);
    to_the_servers(&mut run);
    run.moves("down");
    let live = run.frame();
    assert!(live.len() <= 16, "{} rows: {live:#?}", live.len());
    assert!(
        live.iter()
            .any(|row| row.contains("a larger window shows this row whole")),
        "{live:#?}"
    );
    run.key("space");
    let live = run.frame();
    assert!(!live.iter().any(|row| row.contains('☑')), "{live:#?}");
}

// ------------------------------------------------ the full review of the
// third pull request: what is written, what opens, and the rest

/// To the commands' step, two commands found, its keys drawn.
fn commands_step(run: &mut Run) -> Vec<String> {
    skip_three(run);
    let _ = run.open(TrustStanding::Trusted);
    let _ = run.frame();
    run.detect(two_commands());
    run.ready()
}

/// A file that could not be written: the line says truthfully that
/// nothing was saved, the configuration is not read again, and this run's
/// sessions, `/new`'s too, are opened with what was chosen as their own
/// settings: the instructions as picked, the mode, the model and its
/// provider (High 1).
#[test]
fn a_file_that_could_not_be_written_opens_the_session_with_what_was_chosen() {
    let mut setup = fake();
    setup.write_fails = Some("the disk is full".to_owned());
    let mut run = Run::new(setup);
    let _ = run.ready();
    run.key("enter");
    run.answer();
    let _ = run.ready();
    run.key("space");
    run.key("enter");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.moves("up");
    run.key("enter");
    run.answer();
    // The model alone: no other provider has one of its id (the review of
    // the fixes).
    let chosen = serde_json::json!({
        "compat": { "instructions": true, "skills": false },
        "permissions": { "mode": "read-only", "read_skill_folders": false },
        "model": { "main": MODEL },
    });
    let sent = run.link.take_sent();
    assert!(
        !sent.iter().any(|(_, op)| matches!(op, Op::ConfigReload(_))),
        "nothing to read again"
    );
    let (open, op) = sent
        .into_iter()
        .find(|(_, op)| matches!(op, Op::SessionNew(_)))
        .expect("the session opened all the same");
    let Op::SessionNew(params) = op else {
        unreachable!()
    };
    assert_eq!(serde_json::Value::Object(params.config), chosen);
    run.give(
        open,
        OpResult::SessionNew(SessionResult {
            session: session().trust(TrustStanding::Trusted).build(),
        }),
    );
    let _ = run.frame();
    run.detect(Vec::new());
    let _ = run.frame();
    assert!(!run.app.in_first_run());
    let said = squash(&run.scrollback.join(" "));
    for line in [
        "· Mode: read-only, for this run. Nothing was saved: the disk is full. This run's \
         sessions are opened with what was chosen, and the set-up runs again at the next start.",
        "The model is anthropic's claude-opus-5-5 (model.main).",
        "Nothing was saved, so the set-up runs again at the next start.",
    ] {
        assert!(said.contains(line), "{line}: {said}");
    }
    assert!(!said.contains("holds the settings"), "{said}");
    // `/new` opens its session with them too.
    type_text(&mut run.app, "/new", run.now);
    run.key("enter");
    run.key("enter");
    let (_, op) = run
        .sent(|op| matches!(op, Op::SessionNew(_)))
        .expect("a new session");
    let Op::SessionNew(params) = op else {
        unreachable!()
    };
    assert_eq!(serde_json::Value::Object(params.config), chosen);
}

/// A reload that fails, or finds the files wrong, leaves the file written
/// out of force: the line says so, and the session is opened with what was
/// chosen; a reload that goes well opens it with nothing of its own (High
/// 1).
#[test]
fn a_configuration_that_could_not_be_read_again_opens_the_session_with_what_was_chosen() {
    let failures = [
        Ok(ConfigReloadResult {
            errors: vec!["config.toml: an unknown key".to_owned()],
            ..ConfigReloadResult::default()
        }),
        Err(cineris_protocol::ErrorData::new(
            cineris_protocol::ErrorKind::Config,
            "invalid_configuration",
            "config.toml: an unknown key",
        )),
    ];
    for failure in failures {
        let mut run = Run::new(fake());
        skip_three(&mut run);
        let (reload, _) = run
            .sent(|op| matches!(op, Op::ConfigReload(_)))
            .expect("read again");
        run.link
            .give(Inbound::Done(reload, failure.map(OpResult::ConfigReload)));
        drain(&mut run.app, run.now);
        let config = run.opened(TrustStanding::Trusted);
        assert_eq!(
            serde_json::Value::Object(config),
            serde_json::json!({
                "compat": { "instructions": false, "skills": false },
                "permissions": { "read_skill_folders": false }
            })
        );
        let _ = run.frame();
        let said = squash(&run.scrollback.join(" "));
        assert!(
            said.contains(
                "· ~/.config/cineris/config.toml was written, but the configuration could not be \
                 read again: config.toml: an unknown key. This run's sessions are opened with \
                 what was chosen."
            ),
            "{said}"
        );
    }
    let mut run = Run::new(fake());
    skip_three(&mut run);
    assert!(
        run.open(TrustStanding::Trusted).is_empty(),
        "the file holds them"
    );
}

/// A session that does not open ends the first run, with a line that says
/// what was saved (High 2: it waited for good).
#[test]
fn a_session_that_does_not_open_ends_the_first_run_truthfully() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    let (reload, _) = run
        .sent(|op| matches!(op, Op::ConfigReload(_)))
        .expect("read again");
    run.give(
        reload,
        OpResult::ConfigReload(ConfigReloadResult::default()),
    );
    let (open, _) = run
        .sent(|op| matches!(op, Op::SessionNew(_)))
        .expect("opened");
    run.link.give(Inbound::Done(
        open,
        Err(cineris_protocol::ErrorData::new(
            cineris_protocol::ErrorKind::Usage,
            "no_workspace",
            "The workspace is gone.",
        )),
    ));
    drain(&mut run.app, run.now);
    let _ = run.frame();
    assert!(!run.app.in_first_run(), "it ended");
    let said = squash(&run.scrollback.join(" "));
    assert!(
        said.contains(
            "· Set-up ended: the session could not be opened. ~/.config/cineris/config.toml \
             holds the settings, and a change there is kept. /new tries again."
        ),
        "{said}"
    );
}

/// `x` at the mode keeps what the imports step answered: only the steps
/// not reached take their defaults (Medium 8).
#[test]
fn x_at_the_mode_keeps_the_imports_answered() {
    let mut run = Run::new(fake());
    let _ = run.ready();
    run.key("s");
    run.answer();
    let _ = run.ready();
    run.key("space");
    run.key("enter");
    let _ = run.ready();
    run.key("x");
    run.answer();
    assert_eq!(
        run.written(),
        [Chosen {
            imports: Some(Imported {
                instructions: true,
                ..Imported::default()
            }),
            ..Chosen::default()
        }]
    );
}

/// A server whose command shows a value masked by its name, which is
/// imported as it is, says so on its row (Medium 7).
#[test]
fn a_server_imported_with_a_value_unseen_says_so() {
    let mut setup = fake();
    let docs = &mut setup.servers.get_mut("~/.claude.json").unwrap()[0];
    docs.shown = "npx -y @example/docs-mcp --api-key [redacted]".to_owned();
    docs.hidden = true;
    let mut run = Run::new(setup);
    to_the_servers(&mut run);
    let live = squash(&run.frame().join(" "));
    assert!(
        live.contains("a value shown [redacted] is imported as it is in that file, unseen here"),
        "{live}"
    );
}

/// Two servers of one name, from two files, are not both picked: one name
/// is one server in the user's file, which the step says.
#[test]
fn a_second_server_of_a_name_picked_is_not_picked() {
    let mut setup = fake();
    setup.servers.insert(
        "~/.cursor/mcp.json".to_owned(),
        vec![server("docs", "npx -y @other/docs-mcp", None)],
    );
    let mut run = Run::new(setup);
    to_the_servers(&mut run);
    run.moves("down");
    run.key("space");
    for _ in 0..3 {
        run.moves("down");
    }
    run.key("space");
    run.answer();
    let _ = run.ready();
    run.moves("down");
    run.key("space");
    let live = squash(&run.frame().join(" "));
    assert!(
        live.contains(
            "A server named docs is picked already, from ~/.claude.json: one name is one server."
        ),
        "{live}"
    );
    run.key("enter");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("enter");
    run.answer();
    assert_eq!(
        run.written()[0]
            .imports
            .as_ref()
            .map(|imported| imported.servers.clone()),
        Some(vec![("~/.claude.json".to_owned(), "docs".to_owned())])
    );
}

/// Typing into a command's edit field is typing: a key held down and a
/// paste go in (a paste's line break at its end left off), a paste with a
/// character that cannot be shown is refused whole and said.
#[test]
fn typing_into_a_commands_edit_is_typing() {
    let mut run = Run::new(fake());
    let _ = commands_step(&mut run);
    run.key("e");
    run.app.terminal(
        &cineris_term::Event::Repeat(cineris_term::Key::plain(cineris_term::KeyCode::Char('x'))),
        run.now,
    );
    run.app
        .terminal(&cineris_term::Event::Paste(" --all\n".to_owned()), run.now);
    run.app
        .terminal(&cineris_term::Event::Paste("a\u{7}b".to_owned()), run.now);
    let live = squash(&run.frame().join(" "));
    assert!(live.contains("cargo build --workspacex --all_"), "{live}");
    assert!(
        live.contains("That paste holds a line break or a character that cannot be shown"),
        "{live}"
    );
}

/// The edit's `Enter` keeps a changed command and picks it; one that
/// changed nothing picks nothing; and a second `Enter` straight after does
/// not keep the commands: the step waits a key's repeat window (the early
/// review of the fixes).
#[test]
fn an_edit_kept_picks_only_a_change_and_holds_the_next_enter() {
    let mut run = Run::new(fake());
    let _ = commands_step(&mut run);
    run.key("e");
    run.key("enter");
    let live = run.ready();
    assert_eq!(
        live.iter().filter(|row| row.contains('☐')).count(),
        2,
        "an edit that changed nothing picks nothing: {live:#?}"
    );
    run.key("e");
    type_text(&mut run.app, " --release", run.now);
    run.key("enter");
    // A frame between: only the repeat window holds the second.
    let _ = run.frame();
    run.key("enter");
    assert!(
        run.sent(|op| matches!(op, Op::ProjectUpdate(_) | Op::TurnStart(_)))
            .is_none(),
        "a second enter keeps nothing"
    );
    let live = run.ready();
    assert!(
        live.iter()
            .any(|row| row.contains("☑ build") && row.contains("cargo build --workspace --release")),
        "{live:#?}"
    );
    run.key("enter");
    let (_, update) = run
        .sent(|op| matches!(op, Op::ProjectUpdate(_)))
        .expect("kept");
    let Op::ProjectUpdate(update) = update else {
        unreachable!()
    };
    assert_eq!(
        update.add[0]
            .command
            .as_ref()
            .map(|command| command.command.as_str()),
        Some("cargo build --workspace --release")
    );
}

/// Keys the last step does not take yet go to the input box, as at a
/// dialog: what they type there hides the step until it is sent or
/// emptied (the early review of the fixes).
#[test]
fn keys_before_the_last_step_is_ready_go_to_the_input_box() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Trusted);
    let _ = run.frame();
    run.detect(two_commands());
    let live = run.frame();
    assert!(Run::shows_commands(&live), "{live:#?}");
    type_text(&mut run.app, "hi", run.now);
    assert_eq!(run.app.input().editor().text(), "hi");
    let live = run.frame();
    assert!(!Run::shows_commands(&live), "{live:#?}");
}

/// A command being edited when a turn begins: the edit is dropped and
/// said, and the `Enter` that was to keep it does not send the input box's
/// text as a prompt (the early review of the fixes).
#[test]
fn an_edit_cut_by_a_turn_sends_nothing() {
    let mut run = Run::new(fake());
    let _ = commands_step(&mut run);
    run.key("e");
    type_text(&mut run.app, "ab", run.now);
    run.turn_starts();
    type_text(&mut run.app, "c", run.now);
    run.key("enter");
    assert!(
        run.sent(|op| matches!(op, Op::TurnStart(_))).is_none(),
        "nothing sent"
    );
    let _ = run.frame();
    assert!(
        squash(&run.scrollback.join(" "))
            .contains("The commands' step gave way while a command was being edited"),
        "{:#?}",
        run.scrollback
    );
}

/// Continued after a stop (`fg`), a step starts its grace again: the
/// `Enter` that ended `fg` chooses nothing (the early review of the fixes).
#[test]
fn a_step_starts_its_grace_again_after_fg() {
    let mut run = Run::new(fake());
    let _ = run.ready();
    run.key("s");
    run.answer();
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.app.terminal(&cineris_term::Event::Resumed, run.now);
    run.key("enter");
    run.answer();
    assert!(run.written().is_empty(), "nothing chosen");
    let live = run.frame();
    assert!(live.iter().any(|row| row == "◇ Mode  4 of 5"), "{live:#?}");
}

/// After one `Ctrl+C` the hint row says what a second does, not the
/// step's keys, and the step takes no key until they are drawn again (the
/// early review of the fixes).
#[test]
fn a_key_after_one_ctrl_c_answers_nothing() {
    let mut run = Run::new(fake());
    let _ = run.ready();
    run.key("ctrl+c");
    let _ = run.frame();
    run.key("enter");
    let live = run.frame();
    assert!(run.scrollback.is_empty(), "nothing chosen: {live:#?}");
    assert!(
        live.iter().any(|row| row == "◇ Provider  1 of 5"),
        "{live:#?}"
    );
}

/// A row is accepted only as the last frame drew it: a move and an accept
/// read before the next frame accept nothing (the early review of the
/// fixes).
#[test]
fn a_key_after_a_move_and_before_a_frame_accepts_nothing() {
    let mut setup = fake();
    setup.servers.get_mut("~/.claude.json").unwrap()[0].shown = (1..=60)
        .map(|n| format!("--flag-{n}=value-{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut run = Run::with(setup, 16, None);
    to_the_servers(&mut run);
    // From the file's row, drawn whole, to the server too tall to be.
    run.key("down");
    run.key("space");
    let live = run.frame();
    assert!(!live.iter().any(|row| row.contains('☑')), "{live:#?}");
}

/// Every accepting key holds to "drawn whole": a command too tall for the
/// window is neither picked nor kept from its edit, and a provider or a
/// mode in a window too small for its row is not chosen (the early review
/// of the fixes).
#[test]
fn no_accepting_key_takes_a_row_not_drawn_whole() {
    let long = (1..=120)
        .map(|n| format!("step-{n}"))
        .collect::<Vec<_>>()
        .join(" && ");
    let mut run = Run::with(fake(), 16, None);
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Trusted);
    let _ = run.frame();
    run.detect(vec![
        detected("build", "npm run build", Some(&long), "package.json"),
        detected("test", "cargo test", None, "Cargo.toml"),
    ]);
    let _ = run.ready();
    run.key("space");
    run.key("e");
    type_text(&mut run.app, "x", run.now);
    run.key("enter");
    let live = squash(&run.ready().join(" "));
    assert!(!live.contains('☑'), "{live}");
    assert!(
        live.contains("enter keeps the edit"),
        "still editing: {live}"
    );
    // A provider and a mode in a window of four rows.
    let mut run = Run::with(fake(), 4, None);
    let _ = run.ready();
    run.key("enter");
    let _ = run.frame();
    assert!(run.scrollback.is_empty(), "no provider chosen");
    run.key("s");
    run.answer();
    // Past the imports and the skill folders, to the mode.
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    let written = run.written().len();
    assert!(
        squash(&run.ready().join(" ")).contains("Mode"),
        "at the mode: {}",
        squash(&run.ready().join(" "))
    );
    run.key("enter");
    run.answer();
    assert_eq!(run.written().len(), written, "no mode chosen");
}

/// What is typed into a command's edit holds a dialog that comes just
/// after, as typing into the input box would (the early review of the
/// fixes: `first_run_untaken`).
#[test]
fn typing_into_an_edit_holds_the_dialog_after_it() {
    let mut run = Run::new(fake());
    let _ = commands_step(&mut run);
    run.key("e");
    run.now += Duration::from_secs(5);
    let typed = run.now;
    type_text(&mut run.app, "x", typed);
    run.turn_starts();
    ask(
        &mut run.app,
        &run.link,
        7,
        approval("c_1"),
        typed + Duration::from_millis(10),
    );
    let _ = run.app.frame(typed + Duration::from_millis(450));
    run.app.drawn(typed + Duration::from_millis(450));
    key(&mut run.app, "y", typed + Duration::from_millis(500));
    assert!(
        run.link.script().answers.is_empty(),
        "a quiet second after the typing"
    );
}

/// The last step waits while the history search is open, and while a
/// prompt held at the open is on its way though no turn runs (the early
/// review of the fixes).
#[test]
fn the_last_step_waits_for_the_history_search_and_a_prompt_on_its_way() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Trusted);
    let _ = run.frame();
    let (detect, _) = run
        .sent(|op| matches!(op, Op::ProjectDetect(_)))
        .expect("asked for");
    run.key("ctrl+r");
    run.give(
        detect,
        OpResult::ProjectDetect(ProjectDetectResult {
            commands: two_commands(),
        }),
    );
    let live = run.frame();
    assert!(!Run::shows_commands(&live), "{live:#?}");
    run.key("esc");
    let live = run.frame();
    assert!(Run::shows_commands(&live), "{live:#?}");

    // The command line's prompt steers a turn the open says runs; that
    // turn ends before the prompt's answer comes.
    let mut run = Run::with(fake(), 40, Some("and the tests"));
    skip_three(&mut run);
    let (reload, _) = run
        .sent(|op| matches!(op, Op::ConfigReload(_)))
        .expect("read again");
    run.give(
        reload,
        OpResult::ConfigReload(ConfigReloadResult::default()),
    );
    let (open, _) = run
        .sent(|op| matches!(op, Op::SessionNew(_)))
        .expect("opened");
    run.give(
        open,
        OpResult::SessionNew(SessionResult {
            session: session().running("t_1").build(),
        }),
    );
    assert!(
        run.sent(|op| matches!(op, Op::TurnStart(_))).is_some(),
        "the prompt on its way"
    );
    run.turn_ends();
    let _ = run.frame();
    assert!(
        run.sent(|op| matches!(op, Op::ProjectDetect(_))).is_none(),
        "not while the prompt is on its way"
    );
}

/// With the full screen's search open the last step is not asked for: a
/// frame has the view out while it asks (the early review of the fixes).
#[test]
fn the_last_step_waits_for_the_full_screens_search() {
    let setup = fake();
    let now = Instant::now();
    let link = Scripted::default();
    let start = crate::app::Start {
        version: "0.1.0".to_owned(),
        folder: "~/src/parser".to_owned(),
        cwd: None,
        workspace: crate::testkit::WORKSPACE.to_owned(),
        open: crate::app::Open::New {
            workspace: crate::testkit::WORKSPACE.to_owned(),
            settings: cineris_protocol::session::SessionSettings::default(),
            ephemeral: false,
        },
        prompt: None,
        settings: crate::app::Settings {
            presenter: crate::app::Presenter::Fullscreen,
            ..crate::app::Settings::default()
        },
        keymap: crate::keys::KeyMap::defaults(),
        notices: Vec::new(),
        history: Box::new(crate::history::MemoryHistory::new()),
        highlighter: crate::highlight::Highlighter::off(),
        restyle: None,
        export: None,
        setup: Some(Arc::new(setup.clone())),
    };
    let mut app = crate::app::App::new(
        Box::new(link.clone()),
        start,
        &crate::testkit::caps(),
        99,
        40,
        now,
    );
    let mut now = now;
    let frame = |app: &mut crate::app::App, at: Instant| {
        let _ = app.screen_frame(at);
        app.drawn(at);
    };
    let wait = |app: &mut crate::app::App| assert!(app.wait_for_setup(Duration::from_secs(10)));
    frame(&mut app, now);
    wait(&mut app);
    for _ in 0..4 {
        now += Duration::from_secs(2);
        frame(&mut app, now);
        now += Duration::from_millis(1500);
        frame(&mut app, now);
        key(&mut app, "s", now);
        wait(&mut app);
    }
    let reload = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, Op::ConfigReload(_)))
        .expect("read again")
        .0;
    link.give(Inbound::Done(
        reload,
        Ok(OpResult::ConfigReload(ConfigReloadResult::default())),
    ));
    drain(&mut app, now);
    key(&mut app, "ctrl+f", now);
    let open = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, Op::SessionNew(_)))
        .expect("opened")
        .0;
    link.give(Inbound::Done(
        open,
        Ok(OpResult::SessionNew(SessionResult {
            session: session().trust(TrustStanding::Trusted).build(),
        })),
    ));
    drain(&mut app, now);
    frame(&mut app, now);
    frame(&mut app, now);
    assert!(
        !link
            .take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::ProjectDetect(_))),
        "not while the search is open"
    );
}

/// A store that panics ends `storing…` with why, and the step goes on
/// (a panicking call of the program's).
#[test]
fn a_store_that_panics_ends_storing() {
    let mut setup = fake();
    setup.providers.providers[0].variable = None;
    setup.store_panics = true;
    let mut run = Run::new(setup);
    let _ = run.ready();
    run.key("enter");
    let _ = run.ready();
    type_text(&mut run.app, KEY, run.now);
    run.key("enter");
    run.answer();
    let live = squash(&run.frame().join(" "));
    assert!(
        live.contains("The key could not be stored: it failed inside the program"),
        "{live}"
    );
    assert!(!live.contains("storing"), "{live}");
}

/// A worker that cannot start is said, and this run's sessions read
/// nothing of other tools'.
#[test]
fn a_first_run_that_cannot_start_is_said() {
    crate::app::setup::REFUSE_THREAD.with(|refuse| refuse.set(true));
    let mut run = Run::new(fake());
    assert!(!run.app.in_first_run());
    let config = run.opened(TrustStanding::Trusted);
    assert_eq!(
        serde_json::Value::Object(config),
        serde_json::json!({
            "compat": { "instructions": false, "skills": false },
            "permissions": { "read_skill_folders": false }
        })
    );
    let _ = run.frame();
    assert!(
        squash(&run.scrollback.join(" ")).contains(
            "The set-up could not start (no thread could be made): this run's sessions read \
             nothing of other tools'"
        ),
        "{:#?}",
        run.scrollback
    );
}

// ------------------------------------------------ the review of the fixes
// of the third pull request (Opus)

/// The run's own configuration names the provider beside its model when
/// another provider has a model of that id, as the file does, and only
/// then (M-A, settled by the maintainers).
#[test]
fn a_model_another_provider_has_too_names_its_provider_in_the_runs_configuration() {
    let mut setup = fake();
    setup.providers.providers[0].model_shared = true;
    setup.write_fails = Some("the disk is full".to_owned());
    let mut run = Run::new(setup);
    let _ = run.ready();
    run.key("enter");
    run.answer();
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("s");
    run.answer();
    let config = run.opened(TrustStanding::Trusted);
    assert_eq!(
        config["model"],
        serde_json::json!({ "main": MODEL, "roles": { "main": { "provider": "anthropic" } } })
    );
}

/// A `/new` whose open fails once the first run's own session is open does
/// not end the first run, nor say that its session could not be opened:
/// the first run waits on, and its last step comes with the next session
/// (L-1).
#[test]
fn an_open_that_fails_after_the_first_runs_own_does_not_end_it() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Trusted);
    // At `Opened`, before a frame asks for the project's commands.
    type_text(&mut run.app, "/new", run.now);
    run.key("enter");
    run.key("enter");
    let (open, _) = run
        .sent(|op| matches!(op, Op::SessionNew(_)))
        .expect("a new session");
    run.link.give(Inbound::Done(
        open,
        Err(cineris_protocol::ErrorData::new(
            cineris_protocol::ErrorKind::Usage,
            "no_workspace",
            "The workspace is gone.",
        )),
    ));
    drain(&mut run.app, run.now);
    let _ = run.frame();
    assert!(run.app.in_first_run(), "it waits on for a session");
    let said = squash(&run.scrollback.join(" "));
    assert!(!said.contains("Set-up ended"), "{said}");
    type_text(&mut run.app, "/new", run.now);
    run.key("enter");
    run.key("enter");
    let _ = run.opened(TrustStanding::Trusted);
    let _ = run.frame();
    run.detect(two_commands());
    let live = run.ready();
    assert!(Run::shows_commands(&live), "{live:#?}");
}

/// A first run asked to resume, continue or fork a session, whose choices
/// are not in force from a file (not written, or its worker never started),
/// opens a new session with them in its place and says so; with the file
/// in force it resumes as asked (L-2; the fork since the short review of
/// the fixes).
#[test]
fn a_first_run_asked_to_resume_opens_a_new_session_when_its_choices_are_not_in_force() {
    let resume = || Open::Resume(cineris_protocol::SessionId::from("s_old"));
    let continuing = || Open::Continue {
        workspace: crate::testkit::WORKSPACE.to_owned(),
        settings: cineris_protocol::session::SessionSettings::default(),
        ephemeral: false,
    };
    let instead = "This is a new session, not the one asked for";
    let fork = || Open::Fork(cineris_protocol::SessionId::from("s_old"));
    for open in [resume(), continuing(), fork()] {
        let mut setup = fake();
        setup.write_fails = Some("the disk is full".to_owned());
        let mut run = Run::opening(setup, open);
        skip_three(&mut run);
        let sent = run.link.take_sent();
        assert!(
            !sent.iter().any(|(_, op)| matches!(
                op,
                Op::SessionOpen(_) | Op::SessionList(_) | Op::SessionFork(_)
            )),
            "{sent:?}"
        );
        let Some((_, Op::SessionNew(params))) = sent
            .into_iter()
            .find(|(_, op)| matches!(op, Op::SessionNew(_)))
        else {
            panic!("no new session");
        };
        assert_eq!(
            serde_json::Value::Object(params.config),
            serde_json::json!({
                "compat": { "instructions": false, "skills": false },
                "permissions": { "read_skill_folders": false }
            })
        );
        let _ = run.frame();
        assert!(
            squash(&run.scrollback.join(" ")).contains(instead),
            "{:#?}",
            run.scrollback
        );
    }
    // The file in force: resumed as asked.
    let mut run = Run::opening(fake(), resume());
    skip_three(&mut run);
    let (reload, _) = run
        .sent(|op| matches!(op, Op::ConfigReload(_)))
        .expect("read again");
    run.give(
        reload,
        OpResult::ConfigReload(ConfigReloadResult::default()),
    );
    assert!(
        run.sent(|op| matches!(op, Op::SessionOpen(_))).is_some(),
        "resumed"
    );
    // The worker that never started: nothing of other tools' is read, in a
    // new session.
    crate::app::setup::REFUSE_THREAD.with(|refuse| refuse.set(true));
    let mut run = Run::opening(fake(), resume());
    assert!(!run.app.in_first_run());
    let config = run.opened(TrustStanding::Trusted);
    assert_eq!(
        serde_json::Value::Object(config),
        serde_json::json!({
            "compat": { "instructions": false, "skills": false },
            "permissions": { "read_skill_folders": false }
        })
    );
    let _ = run.frame();
    assert!(
        squash(&run.scrollback.join(" ")).contains(instead),
        "{:#?}",
        run.scrollback
    );
}

/// A file that appears at the configuration's name before it is written is
/// left as it is, read now, and said so: the session opens once it is read,
/// not beside the reload, and what was chosen applies over it for the run,
/// the provider named beside the model so that the file's own role
/// provider does not hold it; not that nothing was saved and the set-up
/// runs again (L-3; the short review of the fixes: it was said to be in
/// force only from the next start; the final review: the open's order, and
/// the found file's provider).
#[test]
fn a_file_found_at_the_name_is_read_now_and_what_was_chosen_applies_over_it() {
    let mut setup = fake();
    setup.file_appears = true;
    let mut run = Run::new(setup);
    // The provider from the environment, with its model; the imports and
    // the mode skipped.
    let _ = run.ready();
    run.key("enter");
    run.answer();
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("s");
    let _ = run.ready();
    run.key("s");
    run.answer();
    let sent = run.link.take_sent();
    let reload = sent
        .iter()
        .find(|(_, op)| matches!(op, Op::ConfigReload(_)))
        .map(|(ticket, _)| *ticket)
        .expect("the file found is read now");
    assert!(
        !sent.iter().any(|(_, op)| matches!(op, Op::SessionNew(_))),
        "the session opens once the file is read, not beside the reload"
    );
    run.give(
        reload,
        OpResult::ConfigReload(ConfigReloadResult::default()),
    );
    let config = run.opened(TrustStanding::Trusted);
    assert_eq!(
        serde_json::Value::Object(config),
        serde_json::json!({
            "compat": { "instructions": false, "skills": false },
            "model": { "main": MODEL, "roles": { "main": { "provider": "anthropic" } } },
            "permissions": { "read_skill_folders": false },
        }),
        "what was chosen applies over it, its provider named"
    );
    let _ = run.frame();
    run.detect(Vec::new());
    let _ = run.frame();
    assert!(!run.app.in_first_run());
    let said = squash(&run.scrollback.join(" "));
    for line in [
        "· Mode: supervised, for this run. ~/.config/cineris/config.toml was made meanwhile, not \
         by the set-up, and was left as it is: it is read now, and what was chosen applies over \
         it for this run.",
        "Nothing of the set-up's was saved: ~/.config/cineris/config.toml, made meanwhile, is the \
         configuration now, what was chosen applying over it for this run, and the set-up does \
         not run again.",
    ] {
        assert!(said.contains(line), "{line}: {said}");
    }
    assert!(!said.contains("runs again at the next start"), "{said}");
}

/// A file found at the name that cannot be read is said by its path, as
/// not read, and the last line does not say it is in force: this run's
/// sessions carry what was chosen (the final review).
#[test]
fn a_file_found_that_cannot_be_read_is_said_so() {
    let mut setup = fake();
    setup.file_appears = true;
    let mut run = Run::new(setup);
    skip_three(&mut run);
    let (reload, _) = run
        .sent(|op| matches!(op, Op::ConfigReload(_)))
        .expect("the file found is read now");
    run.link.give(Inbound::Done(
        reload,
        Ok(OpResult::ConfigReload(ConfigReloadResult {
            errors: vec!["config.toml: an unknown key".to_owned()],
            ..ConfigReloadResult::default()
        })),
    ));
    drain(&mut run.app, run.now);
    let config = run.opened(TrustStanding::Trusted);
    assert_eq!(
        serde_json::Value::Object(config),
        serde_json::json!({
            "compat": { "instructions": false, "skills": false },
            "permissions": { "read_skill_folders": false }
        })
    );
    let _ = run.frame();
    run.detect(Vec::new());
    let _ = run.frame();
    let said = squash(&run.scrollback.join(" "));
    for line in [
        "· ~/.config/cineris/config.toml could not be read: config.toml: an unknown key. This \
         run's sessions are opened with what was chosen.",
        "Nothing of the set-up's was saved: ~/.config/cineris/config.toml, made meanwhile, could \
         not be read, so this run's sessions carry what was chosen; the next start reads it again, \
         and the set-up does not run again.",
    ] {
        assert!(said.contains(line), "{line}: {said}");
    }
    assert!(!said.contains("is the configuration now"), "{said}");
}

/// "Drawn whole" holds by width as by height (L-4): in a window 8 columns
/// wide, where the step fits by height but a provider's detail begins past
/// the edge, no provider is chosen; at 11 columns, where each row wraps
/// whole (its label a character a line, since labels wrap), one is.
#[test]
fn a_row_cut_by_width_is_not_accepted_where_the_step_fits_by_height() {
    let mut run = Run::sized(fake(), 8, 400);
    let _ = run.ready();
    run.key("enter");
    let _ = run.frame();
    assert!(run.scrollback.is_empty(), "no provider chosen");
    let mut run = Run::sized(fake(), 11, 400);
    let _ = run.ready();
    run.key("enter");
    run.answer();
    let _ = run.frame();
    assert!(!run.scrollback.is_empty(), "the provider chosen");
}

/// A label longer than its column wraps in it, and its row is drawn whole
/// and accepted at 80 columns: Claude Desktop's file on Windows is read,
/// and a server named in 60 characters is picked (the maintainers' answer
/// to the review of the fixes: such a row could not be accepted on a small
/// terminal).
#[test]
fn a_long_label_wraps_and_is_accepted_at_80_columns() {
    let path = "~\\AppData\\Roaming\\Claude\\claude_desktop_config.json";
    let name = "a-server-named-in-sixty-characters-which-is-longer-than-room";
    let mut setup = fake();
    setup.found.user_mcp = vec![McpFile {
        path: path.to_owned(),
        from: "Claude Desktop".to_owned(),
    }];
    setup.servers.insert(
        path.to_owned(),
        vec![server(name, "npx -y @example/long-mcp", None)],
    );
    let mut run = Run::sized(setup, 81, 60);
    to_the_servers(&mut run);
    assert_eq!(run.setup.log().read, [path], "the file's row was accepted");
    let live = run.frame();
    assert!(
        live.join("\n")
            .contains("~\\AppData\\Roaming\\Claude\\claude_desktop"),
        "{live:#?}"
    );
    run.moves("down");
    run.key("space");
    let live = run.frame();
    let at = live
        .iter()
        .position(|row| row.starts_with("  ◆"))
        .expect("the selected row");
    assert!(
        live[at].contains(&format!("☑ {}", &name[..40])),
        "picked, its name whole: {live:#?}"
    );
    assert!(live[at + 1].contains(&name[50..]), "{live:#?}");
    assert!(
        !live
            .iter()
            .any(|row| row.contains("a larger window shows this row whole")),
        "{live:#?}"
    );
}

/// At the last step a key held down goes where its press went (L-5): the
/// repeats of a `space` the step took type nothing into the input box, and
/// those of a key it did not take go on to it.
#[test]
fn a_held_key_the_last_step_took_types_nothing_into_the_input_box() {
    let repeat = |run: &mut Run, character: char| {
        for _ in 0..3 {
            run.app.terminal(
                &cineris_term::Event::Repeat(cineris_term::Key::plain(
                    cineris_term::KeyCode::Char(character),
                )),
                run.now,
            );
            run.now += Duration::from_millis(30);
        }
    };
    let mut run = Run::new(fake());
    let _ = commands_step(&mut run);
    run.key("space");
    repeat(&mut run, ' ');
    assert_eq!(run.app.input().editor().text(), "");
    let live = run.frame();
    assert!(Run::shows_commands(&live), "{live:#?}");
    assert!(
        live.iter().any(|row| row.contains("☑ build")),
        "the press picked: {live:#?}"
    );
    // A key the step did not take yet: its repeats go where it went.
    let mut run = Run::new(fake());
    skip_three(&mut run);
    let _ = run.open(TrustStanding::Trusted);
    let _ = run.frame();
    run.detect(two_commands());
    let _ = run.frame();
    run.key("a");
    repeat(&mut run, 'a');
    assert_eq!(run.app.input().editor().text(), "aaaa");
}

/// A key that ended the last step, held down, types nothing into the input
/// box the step gives back: its repeats, marked or not, go where its press
/// went (the short review of the fixes, Low 3: a held `s` typed `sss`).
#[test]
fn a_held_key_that_ends_the_last_step_types_nothing() {
    let mut run = Run::new(fake());
    let _ = commands_step(&mut run);
    run.key("s");
    assert!(!run.app.in_first_run(), "`s` ended it");
    for _ in 0..3 {
        run.app.terminal(
            &cineris_term::Event::Repeat(cineris_term::Key::plain(cineris_term::KeyCode::Char(
                's',
            ))),
            run.now,
        );
        run.now += Duration::from_millis(30);
    }
    // A repeat the terminal does not mark: the same key within the repeat
    // window.
    run.key("s");
    assert_eq!(run.app.input().editor().text(), "");
    // A press after the window is typing.
    run.now += Duration::from_secs(1);
    run.key("s");
    assert_eq!(run.app.input().editor().text(), "s");
}
