//! The first run's screens, F1a to F4 (since step 25;
//! `docs/steps/25/design/screens.md`): the application's own frames,
//! driven through its steps by a scripted core and the test kit's program
//! ([`FakeSetup`]), in a window of the screen's width. A screen's rows are
//! what the frames committed to the scrollback, then the live region of
//! the frame drawn once the step shows its keys.
//!
//! F1a is Linux's wording, the key kept in the owner-only file; F1b is
//! Windows', in Windows Credential Manager.

use std::sync::Arc;
use std::time::{Duration, Instant};

use cineris_protocol::methods::{ConfigReloadResult, ProjectDetectResult, SessionResult};
use cineris_protocol::project::{CommandPurpose, DetectedCommand};
use cineris_protocol::session::{SessionState, TrustStanding};
use cineris_protocol::{Op, OpResult};
use cineris_term::{Event, Key, KeyCode};
use cineris_tui::app::setup::{Found, KeyStore, McpFile, McpServer, Provider, Providers};
use cineris_tui::app::{App, GlyphSet, Inbound, Open, Settings, Start};
use cineris_tui::highlight::Highlighter;
use cineris_tui::history::MemoryHistory;
use cineris_tui::keys::KeyMap;
use cineris_tui::testkit::{FakeSetup, Scripted};
use cineris_tui::theme::Rgb;

use super::scenario::{Row, Screen, Spec};

/// The program the screens' first run talks to.
fn setup(windows: bool) -> FakeSetup {
    let names = ["anthropic", "openai", "openrouter", "deepseek", "venice"];
    let mut setup = FakeSetup {
        providers: Providers {
            providers: names
                .iter()
                .map(|name| Provider {
                    name: (*name).to_owned(),
                    variable: (*name == "anthropic" && !windows)
                        .then(|| "ANTHROPIC_API_KEY".to_owned()),
                    stored: false,
                    model: (*name == "anthropic").then(|| "claude-sonnet-5-5".to_owned()),
                    model_shared: false,
                })
                .collect(),
            store: if windows {
                KeyStore::CredentialManager
            } else {
                KeyStore::File("~/.config/cineris/credentials.toml".to_owned())
            },
            sandbox: true,
        },
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
            project_mcp: vec![".mcp.json".to_owned(), ".vscode/mcp.json".to_owned()],
        },
        ..FakeSetup::default()
    };
    setup.servers.insert(
        "~/.claude.json".to_owned(),
        vec![
            McpServer {
                name: "docs".to_owned(),
                shown: "npx -y @example/docs-mcp --root ~/src/docs".to_owned(),
                hidden: false,
                env: Vec::new(),
                headers: Vec::new(),
                refused: None,
            },
            McpServer {
                name: "tracker".to_owned(),
                shown: "https://tracker.example.com/mcp".to_owned(),
                hidden: false,
                env: Vec::new(),
                headers: vec![("Authorization".to_owned(), "TRACKER_AUTH".to_owned())],
                refused: None,
            },
            McpServer {
                name: "github".to_owned(),
                shown:
                    "docker run -i --rm -e GITHUB_TOKEN=[redacted] ghcr.io/example/github-mcp:1.4"
                        .to_owned(),
                hidden: false,
                env: Vec::new(),
                headers: Vec::new(),
                refused: Some("holds a credential in its command".to_owned()),
            },
        ],
    );
    setup
}

/// The first run as a screen shows it.
struct Drive {
    app: App,
    link: Scripted,
    now: Instant,
    rows: Vec<Row>,
}

impl Drive {
    fn new(spec: Spec, background: Rgb, height: usize, setup: &FakeSetup) -> Self {
        let link = Scripted::default();
        let start = Start {
            version: "0.1.0".to_owned(),
            folder: "~/src/parser".to_owned(),
            cwd: None,
            workspace: "/work/parser".to_owned(),
            open: Open::New {
                workspace: "/work/parser".to_owned(),
                settings: cineris_protocol::session::SessionSettings::default(),
                ephemeral: false,
            },
            prompt: None,
            settings: Settings {
                glyphs: if spec.ascii {
                    GlyphSet::Ascii
                } else {
                    GlyphSet::Unicode
                },
                ..Settings::default()
            },
            keymap: KeyMap::defaults(),
            notices: Vec::new(),
            history: Box::new(MemoryHistory::new()),
            highlighter: Highlighter::off(),
            restyle: None,
            export: None,
            setup: Some(Arc::new(setup.clone())),
        };
        let mut caps = super::capabilities(spec);
        caps.background = Some(background);
        let now = Instant::now();
        let app = App::new(
            Box::new(link.clone()),
            start,
            &caps,
            spec.columns - 1,
            height,
            now,
        );
        let mut drive = Self {
            app,
            link,
            now,
            rows: Vec::new(),
        };
        drive.frame();
        drive.answer();
        drive
    }

    /// A frame, its committed lines kept as the scrollback's; its live
    /// region given back.
    fn frame(&mut self) -> Vec<cineris_term::Line> {
        let (commit, live, _) = self.app.frame(self.now);
        self.rows.extend(commit.into_iter().map(Row::Built));
        self.app.drawn(self.now);
        live
    }

    /// The step's first frame, then the frame with its keys.
    fn ready(&mut self) -> Vec<cineris_term::Line> {
        self.now += Duration::from_secs(2);
        let _ = self.frame();
        self.now += Duration::from_millis(1500);
        self.frame()
    }

    fn key(&mut self, code: KeyCode) {
        self.app.terminal(&Event::Key(Key::plain(code)), self.now);
        self.now += Duration::from_millis(50);
        // The frame the loop draws after a key: a key that accepts a row
        // takes the row the last frame drew whole.
        if matches!(code, KeyCode::Up | KeyCode::Down) {
            let _ = self.frame();
        }
    }

    fn answer(&mut self) {
        assert!(
            self.app.wait_for_setup(Duration::from_secs(10)),
            "the program answered"
        );
    }

    /// Answers the operation of `kind` the application sent with `result`.
    fn give(&mut self, kind: fn(&Op) -> bool, result: OpResult) {
        let (ticket, _) = self
            .link
            .take_sent()
            .into_iter()
            .find(|(_, op)| kind(op))
            .expect("the operation was sent");
        self.link.give(Inbound::Done(ticket, Ok(result)));
        while self.app.drain(self.now, 64).more {}
    }

    /// The screen: the scrollback, then `live`.
    fn screen(mut self, live: Vec<cineris_term::Line>) -> Screen {
        let live_from = self.rows.len();
        self.rows.extend(live.into_iter().map(Row::Built));
        Screen {
            rows: self.rows,
            live_from,
        }
    }
}

fn command(name: &str, line: &str, purpose: CommandPurpose) -> DetectedCommand {
    DetectedCommand {
        name: name.to_owned(),
        command: line.to_owned(),
        runs: None,
        source: "Cargo.toml".to_owned(),
        purpose,
    }
}

/// Screen `spec` (F1a to F4), on `background`, of the session `opened`.
pub(super) fn compose(spec: Spec, background: Rgb, opened: SessionState) -> Screen {
    let windows = spec.id == "f1b";
    let setup = setup(windows);
    let mut drive = Drive::new(spec, background, 40, &setup);
    let live = drive.ready();
    if spec.id == "f1a" {
        return drive.screen(live);
    }
    if windows {
        // The first row: a key for anthropic, typed.
        drive.key(KeyCode::Enter);
        let _ = drive.ready();
        let key: String = std::iter::repeat_n("Zq7Xw9", 18).collect();
        drive.app.terminal(&Event::Paste(key), drive.now);
        let live = drive.frame();
        return drive.screen(live);
    }
    // Its key from the environment.
    drive.key(KeyCode::Enter);
    drive.answer();
    let live = drive.ready();
    if spec.id == "f2a" {
        return drive.screen(live);
    }
    // The instructions picked; the first file read; its first server
    // picked.
    drive.key(KeyCode::Char(' '));
    drive.key(KeyCode::Down);
    drive.key(KeyCode::Down);
    drive.key(KeyCode::Char(' '));
    drive.answer();
    // Its servers came: the step's grace starts again.
    let _ = drive.ready();
    drive.key(KeyCode::Down);
    drive.key(KeyCode::Char(' '));
    let live = drive.frame();
    if spec.id == "f2b" {
        return drive.screen(live);
    }
    drive.key(KeyCode::Enter);
    // The skill folders' question, answered with its default, no (since
    // step 28's third pull request).
    let _ = drive.ready();
    drive.key(KeyCode::Enter);
    let live = drive.ready();
    if spec.id == "f3" {
        return drive.screen(live);
    }
    // The mode kept, the file written and read again, the session opened
    // and trusted, and its commands found.
    drive.key(KeyCode::Enter);
    drive.answer();
    drive.give(
        |op| matches!(op, Op::ConfigReload(_)),
        OpResult::ConfigReload(ConfigReloadResult::default()),
    );
    let mut opened = opened;
    opened.trust = Some(TrustStanding::Trusted);
    drive.give(
        |op| matches!(op, Op::SessionNew(_)),
        OpResult::SessionNew(SessionResult { session: opened }),
    );
    let _ = drive.frame();
    drive.give(
        |op| matches!(op, Op::ProjectDetect(_)),
        OpResult::ProjectDetect(ProjectDetectResult {
            commands: vec![
                command("build", "cargo build --workspace", CommandPurpose::Build),
                command("test", "cargo test --workspace", CommandPurpose::Test),
                command(
                    "check",
                    "cargo clippy --workspace --all-targets -- -D warnings",
                    CommandPurpose::Check,
                ),
                command(
                    "format",
                    "cargo fmt --all -- --check",
                    CommandPurpose::Check,
                ),
            ],
        }),
    );
    let live = drive.ready();
    drive.screen(live)
}
