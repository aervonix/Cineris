//! The first run (since step 25): each step through the scripted link and
//! a fake program, as the loop drives them. What each choice sends and
//! writes, that every step can be skipped, that nothing unpicked is
//! imported, that a key is never drawn, and that every listing it shows
//! holds hostile text.

use std::time::{Duration, Instant};

use cineris_protocol::methods::{
    ConfigReloadResult, Op, OpResult, ProjectDetectResult, SessionResult,
};
use cineris_protocol::project::{CommandPurpose, DetectedCommand};
use cineris_protocol::session::TrustStanding;

use super::{key, text, type_text};
use crate::app::setup::{
    Chosen, Found, Imported, KeyStore, McpFile, McpServer, Provider, Providers,
};
use crate::app::{Inbound, Ticket};
use crate::testing::{TRICKY, problems};
use crate::testkit::{FakeSetup, Scripted, app_first_run, drain, session};

const KEY: &str = "sk-ant-api03-Zq7XfirstrunKEYw9";

fn providers() -> Providers {
    Providers {
        providers: vec![
            Provider {
                name: "anthropic".to_owned(),
                variable: Some("ANTHROPIC_API_KEY".to_owned()),
                stored: false,
            },
            Provider {
                name: "openai".to_owned(),
                variable: None,
                stored: false,
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
        let now = Instant::now();
        let (app, link) = app_first_run(now, &setup, 100, 40);
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

    /// Answers the reload and the open, the session `trust`.
    fn open(&mut self, trust: TrustStanding) {
        let (reload, _) = self
            .sent(|op| matches!(op, Op::ConfigReload(_)))
            .expect("the configuration read again");
        self.give(
            reload,
            OpResult::ConfigReload(ConfigReloadResult::default()),
        );
        let (open, _) = self
            .sent(|op| matches!(op, Op::SessionNew(_)))
            .expect("the session opened once the file was written");
        self.give(
            open,
            OpResult::SessionNew(SessionResult {
                session: session().trust(trust).build(),
            }),
        );
    }

    fn written(&self) -> Vec<Chosen> {
        self.setup.log().written.clone()
    }
}

/// `s` at the provider (whose skip asks for what other tools left), at
/// the imports and at the mode (whose skip writes the file).
fn skip_three(run: &mut Run) {
    let _ = run.ready();
    run.key("s");
    run.answer();
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
        live.iter().any(|row| row == "◇ Provider  1 of 4"),
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
        live.iter().any(|row| row == "◇ Imports  2 of 4"),
        "{live:#?}"
    );
    // Down to the first file, read it, down to its first server, pick it.
    run.key("down");
    run.key("down");
    run.key("space");
    run.answer();
    assert_eq!(run.setup.log().read, ["~/.claude.json"]);
    run.key("down");
    run.key("space");
    // The refused server cannot be picked.
    run.key("down");
    run.key("down");
    run.key("space");
    let live = run.frame();
    assert!(live.iter().any(|row| row.contains("☑ docs")), "{live:#?}");
    run.key("enter");
    let live = run.ready();
    assert!(live.iter().any(|row| row == "◇ Mode  3 of 4"), "{live:#?}");
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
            imports: Some(Imported {
                instructions: false,
                skills: false,
                servers: vec![("~/.claude.json".to_owned(), "docs".to_owned())],
            }),
            mode: Some("supervised".to_owned()),
        }]
    );
    run.open(TrustStanding::Trusted);
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
        live.iter().any(|row| row == "◇ Commands  4 of 4"),
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
    run.key("down");
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
        "· Mode: supervised. Written to ~/.config/cineris/config.toml.",
        "· Project: ~/src/parser trusted.",
        "· Commands: test kept in this project's profile.",
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
#[test]
fn every_step_can_be_skipped() {
    let mut run = Run::new(fake());
    skip_three(&mut run);
    assert_eq!(run.written(), [Chosen::default()]);
    assert!(run.setup.log().stored.is_empty());
    assert!(run.setup.log().read.is_empty(), "no file was read");
    run.open(TrustStanding::Untrusted);
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
    run.open(TrustStanding::Trusted);
    let _ = run.frame();
    let _ = run.frame();
    assert!(run.sent(|op| matches!(op, Op::ProjectDetect(_))).is_none());
    assert!(!run.app.in_first_run());
    let said = run.scrollback.join("\n");
    assert!(
        said.contains("· Set-up skipped. Each step can be done again"),
        "{said}"
    );
    assert!(
        said.contains("~/.config/cineris/config.toml holds"),
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
        live.iter().any(|row| row == "◇ Provider  1 of 4"),
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
        live.iter().any(|row| row == "◇ Imports  2 of 4"),
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

/// A store that refuses says why and keeps the step; `Esc` goes back to
/// the providers with nothing kept.
#[test]
fn a_store_that_refuses_keeps_the_step_and_esc_goes_back() {
    let mut setup = fake();
    setup.store_fails = Some("the disk is full".to_owned());
    let mut run = Run::new(setup);
    let _ = run.ready();
    run.key("down");
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
    run.key("down");
    run.key("down");
    run.key("space");
    run.answer();
    run.key("enter");
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
    run.key("down");
    run.key("down");
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
        run.key("down");
        run.key("down");
        run.key("space");
        run.answer();
        let _ = run.frame();
        run.key("enter");
        let _ = run.ready();
        run.key("enter");
        run.answer();
        run.open(TrustStanding::Untrusted);
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
        run.key("down");
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
    run.open(TrustStanding::Untrusted);
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
