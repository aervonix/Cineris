//! The session the reference screens show (`docs/architecture/tui-look/`):
//! turn 3 in a Rust workspace, where the agent fixes digit separators in a
//! lexer, runs the tests, sees one fail, and streams the fix while the user
//! types a steering message. Each screen is composed from the real views as
//! the interactive application will compose them: step 22's views, step
//! 21's input box and menus, and step 23's placeholder and viewer frame.
//! A dialog is the application's own: [`App`] is given the request through
//! a scripted core, in a window of the screen's size, and its live region
//! is taken from the frame it draws once the dialog takes keys (the review
//! of step 23). The plan panel's rows are its view's (step 24), with the
//! blank row the panels' block ends in. The rows other steps draw (the
//! usage panel's limit line) are marked as not built yet, with the step
//! that builds them.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cineris_protocol::item::{Plan, PlanItem, PlanStatus};
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::lease::Effect;
use cineris_protocol::methods::{
    AddedRule, ApprovalChoice, ApprovalDetail, ApprovalRequest, CommandDetail, DiffDetail,
    QuestionRequest, SessionResult, TrustRequest, TrustResource, TrustResourceKind,
};
use cineris_protocol::session::{RoleState, SandboxStatus, SessionSettings, SessionState};
use cineris_protocol::{
    CallId, Level, ModelId, ModelRole, Op, OpResult, PlanItemId, ProviderId, ServerRequest,
    ServerResponse, SessionId, Speed as Pace, Timestamp, TurnId,
};
use cineris_term::{ColorDepth, Event, Key, KeyCode, Line};
use cineris_tui::app::{App, GlyphSet, Inbound, Link, Open, Settings, Start, Ticket};
use cineris_tui::complete::{Menu, Row as Choice, Toggle};
use cineris_tui::glyphs::Glyphs;
use cineris_tui::highlight::Highlighter;
use cineris_tui::history::MemoryHistory;
use cineris_tui::input::Input;
use cineris_tui::keys::KeyMap;
use cineris_tui::markdown::Markdown;
use cineris_tui::theme::{Rgb, Theme};
use cineris_tui::transcript::{Block, Reasoning, Transcript};
use cineris_tui::view::viewer::{ViewerKeys, ViewerTitle};
use cineris_tui::view::{
    self, Call, CallState, Change, Detail, DiffLine, Header, Hint, HintRow, Look, Measure,
    Placement, Round, RunningKeys, Speed, Target,
};

/// A row of a screen: drawn by this step's views, or by a step not yet
/// built.
#[derive(Debug, Clone)]
pub(crate) enum Row {
    Built(Line),
    NotBuilt(&'static str),
}

/// One reference screen, composed.
#[derive(Debug, Clone)]
pub(crate) struct Screen {
    pub(crate) rows: Vec<Row>,
    /// Where the live region starts.
    pub(crate) live_from: usize,
}

/// A screen's name, the window's width, its colour depth and glyph set.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Spec {
    pub(crate) id: &'static str,
    pub(crate) columns: usize,
    pub(crate) depth: ColorDepth,
    pub(crate) ascii: bool,
}

/// Every reference screen.
pub(crate) const SPECS: [Spec; 19] = [
    spec("0"),
    spec("1"),
    spec("1b"),
    spec("1c"),
    spec("1d"),
    spec("2a"),
    spec("2b"),
    spec("2c"),
    spec("2d"),
    spec("3a"),
    spec("3b"),
    spec("4"),
    spec("5"),
    Spec {
        id: "6",
        columns: 60,
        depth: ColorDepth::TrueColor,
        ascii: false,
    },
    Spec {
        id: "7a",
        columns: 100,
        depth: ColorDepth::Ansi16,
        ascii: false,
    },
    Spec {
        id: "7b",
        columns: 100,
        depth: ColorDepth::Mono,
        ascii: false,
    },
    Spec {
        id: "7c",
        columns: 100,
        depth: ColorDepth::Ansi16,
        ascii: true,
    },
    spec("8a"),
    spec("8b"),
];

const fn spec(id: &'static str) -> Spec {
    Spec {
        id,
        columns: 100,
        depth: ColorDepth::TrueColor,
        ascii: false,
    }
}

/// The two backgrounds the screens are drawn on: Windows Terminal's
/// Campbell and One Half Light.
pub(crate) const BACKGROUNDS: [(&str, Rgb); 2] =
    [("dark", (0x0c, 0x0c, 0x0c)), ("light", (0xfa, 0xfa, 0xfa))];

// ---- the session

const PREVIOUS: &str = "All 46 tests pass, and `cargo fmt` is clean.";
const PROMPT: &str = "1_000 lexes as 1. Fix scan_number so digit separators work, and add a test.";
const THOUGHT: &str = "The loop breaks on any non-digit, so `_` ends the literal. Check whether a trailing `_` is meant to be an error first.";
const THINKING: &str = "…so the loop breaks on any non-digit and `_` ends the literal; before changing it, check whether a trailing";
const FIRST: &str = "`scan_number` stops at the first character that is not a digit, so `1_000` ends after the `1`. \
                     I'll read the scanner and allow `_` between digits.";
const SECOND: &str = "One test fails: `1_` must be an error, but the scanner now stops before the `_` and returns `1`. \
                      A separator has to be followed by a digit, so the other case becomes an error:";
const CODE: [&str; 2] = [
    "if c == '_' {",
    "    if !self.peek_next().is_some_and(|d| d.is_ascii_digit()) {",
];
const STREAMING: &str = "        return Err(self.error(\"digit separator mu";
const CODE_REST: [&str; 5] = [
    "        return Err(self.error(\"digit separator must be followed by a digit\"));",
    "    }",
    "    self.bump();",
    "    continue;",
    "}",
];
const TEST_OUTPUT: &str = "test lex::numbers::separators ... ok\n\
                           test lex::numbers::trailing_separator ... FAILED\n\
                           assertion failed: lex(\"1_\").is_err()\n\
                           test result: FAILED. 47 passed; 1 failed; 0 ignored; finished in 0.02s\n";
const DONE: &str = "Done. `1_000` lexes as one integer, `1_` is an error with its own message, and the new test covers both.";

fn look(spec: Spec, background: Rgb) -> Look {
    let glyphs = if spec.ascii {
        &Glyphs::ASCII
    } else {
        &Glyphs::UNICODE
    };
    Look::new(Theme::new(spec.depth, Some(background)), glyphs)
}

fn answer(text: &str) -> Block {
    let mut stream = Markdown::answer();
    stream.push(text);
    stream.finish();
    Block::Answer(stream)
}

fn diff() -> Vec<DiffLine> {
    let line = |number, change, text: &str| DiffLine {
        number,
        change,
        text: text.to_owned(),
        emphasis: Vec::new(),
    };
    vec![
        line(141, Change::Same, "    while let Some(c) = self.peek() {"),
        line(
            142,
            Change::Removed,
            "        if !c.is_ascii_digit() { break; }",
        ),
        line(
            142,
            Change::Added,
            "        if c == '_' && self.peek_next().is_some_and(|d| d.is_ascii_digit()) {",
        ),
        line(143, Change::Added, "            self.bump();"),
        line(144, Change::Added, "            continue;"),
        line(145, Change::Added, "        }"),
        line(
            146,
            Change::Added,
            "        if !c.is_ascii_digit() { break; }",
        ),
        line(147, Change::Same, "        self.bump();"),
    ]
}

fn call(
    verb: &str,
    target: Target,
    state: CallState,
    measure: Measure,
    took: Option<u64>,
    detail: Detail,
) -> Call {
    Call {
        verb: verb.to_owned(),
        target,
        state,
        measure,
        took: took.map(Duration::from_millis),
        detail,
    }
}

fn read() -> Call {
    call(
        "Read",
        Target::Path("crates/parser/src/lex.rs".to_owned()),
        CallState::Succeeded,
        Measure::Lines {
            first: 120,
            last: 178,
        },
        None,
        Detail::None,
    )
}

fn edit() -> Call {
    call(
        "Edit",
        Target::Path("crates/parser/src/lex.rs".to_owned()),
        CallState::Succeeded,
        Measure::Changes {
            added: 5,
            removed: 1,
        },
        None,
        Detail::Diff(diff()),
    )
}

fn tests_failed() -> Call {
    call(
        "Run",
        Target::Text("cargo test -p parser".to_owned()),
        CallState::Failed,
        Measure::Note("exit 101".to_owned()),
        Some(3_400),
        Detail::Output(TEST_OUTPUT.to_owned()),
    )
}

fn tests_running() -> Call {
    call(
        "Run",
        Target::Text("cargo test -p parser".to_owned()),
        CallState::Running(Duration::from_millis(1_200)),
        Measure::None,
        None,
        Detail::Output(
            "   Compiling parser v0.3.0 (crates/parser)\n    Finished `test` profile [unoptimized] in 0.94s\n     Running unittests src/lib.rs\n"
                .to_owned(),
        ),
    )
}

fn round(calls: Vec<Call>) -> Round {
    Round { calls, open: None }
}

fn header() -> Header {
    Header {
        version: "0.1.0".to_owned(),
        model: "opus-5.5".to_owned(),
        effort: Some("high".to_owned()),
        folder: "~/src/parser".to_owned(),
        branch: Some("main".to_owned()),
        profile: Some("Knows 4 commands and 3 conventions here. /project shows them.".to_owned()),
    }
}

fn hint(left: Hint, fast: bool) -> HintRow {
    HintRow {
        left,
        model: "opus-5.5".to_owned(),
        effort: Some("high".to_owned()),
        speed: fast.then(|| Speed {
            name: "fast".to_owned(),
            cooldown_until: None,
        }),
        context: None,
        budget: None,
        limit: None,
        access: None,
        extras: Vec::new(),
    }
}

fn running(seconds: u64, tick: usize) -> Hint {
    Hint::Running {
        elapsed: Duration::from_secs(seconds),
        tick,
        keys: RunningKeys::Steer,
    }
}

/// The longest the screens wait for the highlighting worker: never reached
/// when all is well.
const SETTLE: Duration = Duration::from_secs(8);

/// One highlighter for every screen, as the application has one for the
/// process: the grammars load once.
fn highlighter() -> Highlighter {
    static SHARED: std::sync::OnceLock<Highlighter> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| Highlighter::new(|| {})).clone()
}

/// The turn so far, from the previous answer to the answer streaming its
/// code block.
fn turn(transcript: &mut Transcript, through: &str, highlighter: &Highlighter) {
    transcript.push(answer(PREVIOUS));
    transcript.push(Block::User(PROMPT.to_owned()));
    if through == "prompt" {
        return;
    }
    transcript.push(Block::Reasoning(Reasoning {
        text: String::new(),
        elapsed: Duration::from_secs(6),
        summary: Some(THOUGHT.to_owned()),
        finished: true,
    }));
    transcript.push(answer(FIRST));
    if through == "first" {
        return;
    }
    transcript.push(Block::Round(round(vec![read(), edit(), tests_failed()])));
    if through == "round" {
        return;
    }
    let mut stream = Markdown::answer().with_highlighter(highlighter.clone());
    stream.push(&format!(
        "{SECOND}\n\n```rust\n{}\n{STREAMING}",
        CODE.join("\n")
    ));
    transcript.push(Block::Answer(stream));
}

/// Builds a screen: the transcript committed and live, then the rows of
/// the live region below it.
struct Compose<'a> {
    rows: Vec<Row>,
    live_from: usize,
    width: usize,
    look: &'a Look,
    highlighter: Highlighter,
}

impl Compose<'_> {
    /// The transcript committed, once its code lines have their colours, then
    /// its live rows.
    fn transcript(&mut self, transcript: &mut Transcript, tick: usize) {
        let mut committed = transcript.commit(self.width, self.look);
        assert!(
            self.highlighter.settle(SETTLE),
            "the highlighter answered in time"
        );
        committed.extend(transcript.commit(self.width, self.look));
        self.rows.extend(committed.into_iter().map(Row::Built));
        self.live_from = self.rows.len();
        let live = transcript.live(self.width, self.look, tick);
        self.rows.extend(live.into_iter().map(Row::Built));
    }

    fn hint(&mut self, row: &HintRow) {
        self.rows
            .push(Row::Built(view::hint_row(row, self.width, self.look)));
    }

    fn lines(&mut self, lines: Vec<Line>) {
        self.rows.extend(lines.into_iter().map(Row::Built));
    }

    /// Step 21's input box, holding what `input` holds.
    fn input(&mut self, input: &Input) {
        self.lines(view::input::input(input, self.look, self.width).lines);
    }

    /// The input box empty while a turn runs: its placeholder says typing
    /// steers the turn, or, with the focus on a box (`Ctrl+P`), it holds
    /// its mark alone.
    fn input_running(&mut self, focus_elsewhere: bool) {
        let mut input = prompt("", self.width);
        input.set_running(true);
        if focus_elsewhere {
            input.focus_mut().update(&[1], &[]);
            input.focus_mut().advance();
        }
        self.input(&input);
    }

    /// The application's live region for `request`, asked in `session` (or
    /// while the session is being opened), in a window `height` rows high:
    /// the dialog, its blank row and its hint row, as the frame shows them
    /// once the dialog takes keys. The screen is that height in all.
    fn dialog(
        &mut self,
        spec: Spec,
        background: Rgb,
        height: usize,
        session: Option<SessionState>,
        request: ServerRequest,
        running: Option<Duration>,
    ) {
        self.lines(app_dialog(
            spec, background, height, session, request, running,
        ));
        assert_eq!(
            self.rows.len(),
            height,
            "screen {} is as high as the window the application drew it in",
            spec.id
        );
    }

    /// Step 21's menu, open in `input`, in the hint row's place.
    fn menu(&mut self, input: &Input) {
        let lines = view::menu::menu(input, self.look, self.width).expect("a menu is open");
        self.lines(lines);
    }
}

/// An input that `text` was typed into, a key at a time, its text wrapping
/// for a box `width` wide.
fn prompt(text: &str, width: usize) -> Input {
    let mut input = Input::new(KeyMap::defaults(), Box::new(MemoryHistory::new()));
    input.set_width(view::input::text_width(width));
    for c in text.chars() {
        input.handle(&Event::Key(Key::plain(KeyCode::Char(c))));
    }
    input
}

/// The `/model` picker of screen 3a, with the second row chosen. Its notes
/// are laid out in columns by whoever opens it: here, with gaps of three.
fn model_picker() -> Menu {
    let rows = [
        (
            "opus-5.5",
            " anthropic   200k  $5 · $25 /M    fast  current",
        ),
        ("sonnet-5.5", " anthropic   200k  $3 · $15 /M"),
        ("haiku-5.5", " anthropic   200k  $1 · $5 /M"),
        ("fable-5.2", " anthropic   200k  $15 · $75 /M   fast"),
        ("deepseek-v4", " deepseek    128k  $0.27 · $1.10 /M"),
        ("qwen3-coder", " openrouter  256k  $0.40 · $1.60 /M"),
    ];
    let mut menu = Menu::picker(
        "model",
        rows.iter()
            .map(|(label, note)| Choice::new(*label, *note))
            .collect(),
        0,
        None,
    );
    menu.select(1);
    menu
}

/// The `/effort` picker of screen 3b, with its fast-mode row.
fn effort_picker() -> Menu {
    let rows = [
        ("low", "a quick look, least reasoning"),
        ("medium", "balanced"),
        ("high", "careful work; the default for this model   current"),
        ("max", "the most reasoning the model allows"),
    ];
    Menu::picker(
        "effort",
        rows.iter()
            .map(|(label, note)| Choice::new(*label, *note))
            .collect(),
        2,
        Some(Toggle {
            label: "fast mode".to_owned(),
            on: true,
            note: "· about 2.5x the output speed, at 2x the price".to_owned(),
        }),
    )
}

/// The request of screen 2a: an edit of a protected file, with its reason,
/// which may be allowed for longer, and the rule that would add.
fn ci_edit() -> ApprovalRequest {
    let mut request = request(
        "edit",
        "Edit .github/workflows/ci.yml",
        ApprovalDetail::Diff(DiffDetail {
            path: ".github/workflows/ci.yml".to_owned(),
            diff: [
                "@@ -29,3 +29,4 @@",
                "       - uses: dtolnay/rust-toolchain@stable",
                "-        with: { components: rustfmt }",
                "+        with: { components: rustfmt, clippy }",
                "       - run: cargo test --workspace",
                "+      - run: cargo clippy --workspace -- -D warnings",
                "",
            ]
            .join("\n"),
            omitted: None,
        }),
        vec!["CI configuration is protected, so a change to it always asks.".to_owned()],
    );
    // What "this session" and "here" would add, as the core offers it for a
    // protected file: writing that one file, named by its whole path.
    request.adds = vec![AddedRule {
        effect: Effect::Write,
        pattern: "/work/parser/.github/workflows/ci.yml".to_owned(),
    }];
    request
}

/// The request of screen 2b: a command that needs the network, to run
/// outside the sandbox. It is asked with every answer the core offers a
/// command; leaving the sandbox offers none that lasts.
fn publish_outside() -> ApprovalRequest {
    let mut request = request(
        "shell",
        "Run cargo publish --dry-run -p parser",
        ApprovalDetail::Command(CommandDetail {
            command: "cargo publish --dry-run -p parser".to_owned(),
            cwd: "crates/parser".to_owned(),
            outside_sandbox: true,
        }),
        Vec::new(),
    );
    request.adds = vec![AddedRule {
        effect: Effect::Exec,
        pattern: "cargo publish *".to_owned(),
    }];
    request
}

fn request(
    tool: &str,
    summary: &str,
    detail: ApprovalDetail,
    reasons: Vec<String>,
) -> ApprovalRequest {
    ApprovalRequest {
        session_id: SessionId::from(SESSION),
        turn_id: TurnId::from("t_3"),
        call_id: CallId::from("c_1"),
        tool: tool.to_owned(),
        summary: summary.to_owned(),
        detail,
        choices: vec![
            ApprovalChoice::AllowOnce,
            ApprovalChoice::AllowSession,
            ApprovalChoice::AllowProject,
            ApprovalChoice::Deny,
        ],
        reasons,
        adds: Vec::new(),
    }
}

/// The trust question of screen 2c, as the core asks it when a session of
/// this workspace opens: its repository, a rule of its own, one of a
/// profile it defines, and a setting held back until it is trusted.
fn trust_question() -> TrustRequest {
    let resource = |kind, name: &str, detail: &str| TrustResource {
        kind,
        name: name.to_owned(),
        detail: detail.to_owned(),
        changed: false,
    };
    TrustRequest {
        session_id: SessionId::from(SESSION),
        trusted: false,
        workspace: "~/src/parser".to_owned(),
        resources: vec![
            resource(
                TrustResourceKind::Repository,
                "git",
                "Git may read this repository's own configuration, which can name programs for \
                 git to run: it is how the harness sees what a command changed, and reads the \
                 repository's state.",
            ),
            resource(
                TrustResourceKind::Rules,
                "permissions.rules",
                "allow write .github/**",
            ),
            resource(
                TrustResourceKind::Rules,
                "profiles.ci.permissions.rules",
                "allow exec make deploy",
            ),
            resource(
                TrustResourceKind::Settings,
                "model.main",
                "model.main = \"claude-haiku-5-5\" (it can raise what you spend)",
            ),
        ],
    }
}

/// The question of screen 2d: the undo's, as the core asks it when a file
/// the turn changed was changed again since.
fn undo_question() -> QuestionRequest {
    QuestionRequest {
        session_id: SessionId::from(SESSION),
        turn_id: None,
        question: "This file was changed after the turn that changed it: \
                   `crates/parser/src/lex.rs`. Put it back as it was before that turn, losing \
                   those changes?"
            .to_owned(),
        choices: vec![
            "Put them back".to_owned(),
            "Keep them as they are".to_owned(),
        ],
        extension: None,
    }
}

const SESSION: &str = "s_screens";

/// The height of screen 2c's window: its dialog, the line of what each
/// answer does (since the follow-ups of step 23), the blank row and the
/// hint row, with no room to spare.
const TRUST_HEIGHT: usize = 15;

/// The height of screen 2d's window.
const UNDO_HEIGHT: usize = 8;

/// The session the screens show, as the core gives it to the application:
/// `claude-opus-5-5` at high effort and fast, its commands confined.
fn session() -> SessionState {
    let mut roles = BTreeMap::new();
    roles.insert(
        ModelRole::Main,
        RoleState {
            provider: ProviderId::from("anthropic"),
            model: ModelId::from("claude-opus-5-5"),
            effort: Some("high".to_owned()),
            effort_in_effect: Some("high".to_owned()),
            speed: Pace::Fast,
            speed_in_effect: Pace::Fast,
            speed_cooldown_until: None,
        },
    );
    SessionState {
        session_id: SessionId::from(SESSION),
        workspace: "/work/parser".to_owned(),
        title: None,
        created_at: Timestamp::from("2026-11-14T14:02:00Z"),
        parent: None,
        level: Level::Supervised,
        lease_id: None,
        sandbox: SandboxStatus::Enforcing,
        speed: Pace::Fast,
        roles,
        seq: 0,
        running_turn: None,
        queue_length: 0,
        write_roots: vec!["/work/parser".to_owned()],
        trust: None,
    }
}

/// What the scripted core has been sent, and what it gives next.
#[derive(Debug, Default)]
struct Script {
    next: u64,
    sent: Vec<(Ticket, Op)>,
    inbound: VecDeque<Inbound>,
}

/// The core, scripted: the application's link to it records what is sent
/// and gives what the screen queued.
#[derive(Debug, Clone, Default)]
struct Core(Arc<Mutex<Script>>);

impl Core {
    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Link for Core {
    fn send(&mut self, op: Op) -> Ticket {
        let mut script = self.script();
        script.next += 1;
        let ticket = Ticket(script.next);
        script.sent.push((ticket, op));
        ticket
    }

    fn respond(&mut self, _: RequestId, _: ServerResponse) {}

    fn next(&mut self) -> Option<Inbound> {
        self.script().inbound.pop_front()
    }
}

/// The live region [`App`] draws for `request` in a window of `spec`'s
/// width and `height` rows, on a terminal of `spec`'s depth and glyphs
/// whose background is `background`: the session is opened as `session`
/// (or, with `None`, is still being opened, as when the trust question is
/// asked), the request comes, and the frame is the one the loop draws when
/// the dialog is ready for keys, its answers shown.
fn app_dialog(
    spec: Spec,
    background: Rgb,
    height: usize,
    session: Option<SessionState>,
    request: ServerRequest,
    running: Option<Duration>,
) -> Vec<Line> {
    let core = Core::default();
    let start = Start {
        version: "0.1.0".to_owned(),
        folder: "~/src/parser".to_owned(),
        cwd: None,
        workspace: "/work/parser".to_owned(),
        open: Open::New {
            workspace: "/work/parser".to_owned(),
            settings: SessionSettings::default(),
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
    };
    let mut caps = super::capabilities(spec);
    caps.background = Some(background);
    let began = Instant::now();
    let mut app = App::new(
        Box::new(core.clone()),
        start,
        &caps,
        spec.columns - 1,
        height,
        began,
    );
    let drain = |app: &mut App| while app.drain(began, 64).more {};
    // The first frame opens the session.
    let _ = app.frame(began);
    app.drawn(began);
    if let Some(session) = session {
        let opened = core
            .script()
            .sent
            .iter()
            .find(|(_, op)| matches!(op, Op::SessionNew(_)))
            .map(|(ticket, _)| *ticket)
            .expect("the session is opened after the first frame");
        core.script().inbound.push_back(Inbound::Done(
            opened,
            Ok(OpResult::SessionNew(SessionResult { session })),
        ));
        drain(&mut app);
    }
    // A question asked in the middle of a turn: the turn began when the
    // session did, and the question comes `running` into it, so that the
    // hint row shows the spinner and the turn's time before the dialog's
    // keys, as the application draws it (the owner's word of 2026-11-16).
    let asked = match running {
        Some(into) => {
            core.script()
                .inbound
                .push_back(Inbound::Event(cineris_protocol::SessionEvent {
                    session_id: SessionId::from(SESSION),
                    seq: 1,
                    turn_id: Some(TurnId::from("t_3")),
                    event: cineris_protocol::event::Event::TurnStarted(
                        cineris_protocol::event::TurnStarted {},
                    ),
                }));
            drain(&mut app);
            began + into
        }
        None => began,
    };
    core.script()
        .inbound
        .push_back(Inbound::Request(RequestId::from(1), request));
    while app.drain(asked, 64).more {}
    // The dialog's first frame, its answers not shown yet; then the frame
    // the loop draws when it is ready for keys.
    let _ = app.frame(asked);
    app.drawn(asked);
    let ready = app.deadline().expect("the dialog's answers are due");
    let (_, live, _) = app.frame(ready);
    app.drawn(ready);
    assert_eq!(
        app.deadline(),
        None,
        "the frame showed the dialog's answers"
    );
    live
}

/// The viewer's title in screens 8a and 8b.
fn viewer_title() -> ViewerTitle {
    ViewerTitle {
        folder: "~/src/parser".to_owned(),
        began: Some("14:02".to_owned()),
        turns: Some((2, 3)),
    }
}

/// What the steering message says.
const STEER: &str = "also run clippy when that's done";

/// Screen 4's usage panel, opened by itself: the line of a limit near, the
/// fast tier's output at 18% of its limit, which resets at 15:00, seen a
/// minute before 14:30 (UTC, the zone the screen is said in).
fn screen_usage(width: usize, look: &Look) -> Vec<Line> {
    use cineris_base::{UtcTime, Zone};
    use cineris_protocol::usage::{
        LimitMeasure, LimitSource, LimitWindow, ProviderLimits, SessionUsage, UsageRange,
    };
    let session = SessionUsage {
        limits: vec![ProviderLimits {
            provider: ProviderId::from("anthropic"),
            windows: vec![LimitWindow {
                measure: LimitMeasure::OutputTokens,
                speed: Some(Pace::Fast),
                limit: Some(100_000),
                remaining: Some(18_000),
                resets_at: Some(Timestamp::from("2026-11-16T15:00:00Z")),
            }],
            seen_at: Timestamp::from("2026-11-16T14:29:00Z"),
            source: LimitSource::Headers,
            balances: Vec::new(),
            available: None,
            next_epoch: None,
        }],
        ..SessionUsage::default()
    };
    let zone = Zone::utc();
    let state = view::usage::UsageView {
        session: Some(&session),
        report: None,
        reading: false,
        range: &UsageRange::Today,
        group: &[],
        refreshed: &[],
        said: &[],
        now: UtcTime::parse("2026-11-16T14:30:00Z").expect("a time"),
        zone: &zone,
    };
    view::usage::usage_collapsed(&state, false, width, look)
}

/// Screen 4's plan: one item done, the one being worked on, two to do.
fn screen_plan() -> Plan {
    let item = |id: &str, text: &str, status| PlanItem {
        id: PlanItemId::from(id),
        text: text.to_owned(),
        status,
    };
    Plan {
        items: vec![
            item("p1", "allow _ between digits", PlanStatus::Done),
            item("p2", "reject a trailing _", PlanStatus::Active),
            item("p3", "add a test for 1_000", PlanStatus::Pending),
            item("p4", "run clippy", PlanStatus::Pending),
        ],
        revision: 1,
    }
}

/// The screen `spec` on `background`.
pub(crate) fn compose(spec: Spec, background: Rgb) -> Screen {
    let look = look(spec, background);
    let width = spec.columns - 1;
    let highlighter = highlighter();
    let mut screen = Compose {
        rows: Vec::new(),
        live_from: 0,
        width,
        look: &look,
        highlighter: highlighter.clone(),
    };
    let mut transcript = Transcript::new();
    match spec.id {
        "0" => {
            transcript.push(Block::Header(header()));
            screen.transcript(&mut transcript, 0);
            screen.input(&prompt("", width));
            screen.hint(&hint(Hint::FirstSessions, false));
        }
        "1" | "6" | "7a" | "7b" | "7c" => {
            turn(&mut transcript, "all", &highlighter);
            screen.transcript(&mut transcript, 0);
            screen.input(&prompt(STEER, width));
            screen.hint(&hint(running(14, 0), true));
        }
        "1b" => {
            turn(&mut transcript, "prompt", &highlighter);
            transcript.push(Block::Reasoning(Reasoning {
                text: THINKING.to_owned(),
                elapsed: Duration::from_secs(3),
                summary: None,
                finished: false,
            }));
            screen.transcript(&mut transcript, 3);
            screen.input_running(false);
            screen.hint(&hint(running(3, 3), true));
        }
        "1c" | "1d" => {
            turn(&mut transcript, "first", &highlighter);
            let mut live = round(vec![read(), edit(), tests_running()]);
            if spec.id == "1d" {
                live.open = Some(false);
            }
            transcript.push(Block::Round(live));
            screen.transcript(&mut transcript, 2);
            screen.input_running(false);
            screen.hint(&hint(running(9, 2), true));
        }
        "2a" | "2b" => {
            let call = if spec.id == "2a" {
                call(
                    "Run",
                    Target::Text("cargo clippy -p parser -- -D warnings".to_owned()),
                    CallState::Succeeded,
                    Measure::Note("no warnings".to_owned()),
                    Some(8_100),
                    Detail::Output(
                        "    Checking parser v0.3.0 (crates/parser)\n    Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.06s\n"
                            .to_owned(),
                    ),
                )
            } else {
                call(
                    "Run",
                    Target::Text("cargo publish --dry-run -p parser".to_owned()),
                    CallState::Failed,
                    Measure::Note("blocked: network".to_owned()),
                    Some(400),
                    Detail::Output(
                        "error: failed to query the crates.io index: network unreachable\n(sandbox.network is \"off\": confined commands have no network)\n"
                            .to_owned(),
                    ),
                )
            };
            transcript.push(Block::Round(round(vec![call])));
            transcript.push(answer(if spec.id == "2a" {
                "Clippy is clean. I'll add it to CI so it stays that way; that file is protected, so it needs your approval."
            } else {
                "The dry run has to reach crates.io, and the sandbox has no network. I'll ask to run it outside the sandbox."
            }));
            screen.transcript(&mut transcript, 0);
            let (height, request) = if spec.id == "2a" {
                (20, ci_edit())
            } else {
                (19, publish_outside())
            };
            // Asked in the middle of the turn, its time running.
            let into = Duration::from_secs(if spec.id == "2a" { 12 } else { 7 });
            screen.dialog(
                spec,
                background,
                height,
                Some(session()),
                ServerRequest::Approval(request),
                Some(into),
            );
        }
        // The trust question, asked as the session opens, before anything
        // of it is shown: the window is as high as the dialog and its
        // chrome, so that the dialog fits it exactly.
        "2c" => screen.dialog(
            spec,
            background,
            TRUST_HEIGHT,
            None,
            ServerRequest::Trust(trust_question()),
            None,
        ),
        // The undo's question, after the turn's answer.
        "2d" => {
            transcript.push(answer(DONE));
            screen.transcript(&mut transcript, 0);
            screen.dialog(
                spec,
                background,
                UNDO_HEIGHT,
                Some(session()),
                ServerRequest::Question(undo_question()),
                None,
            );
        }
        "3a" | "3b" => {
            transcript.push(answer(DONE));
            screen.transcript(&mut transcript, 0);
            let (typed, picker) = if spec.id == "3a" {
                ("/model", model_picker())
            } else {
                ("/effort", effort_picker())
            };
            let mut input = prompt(typed, width);
            input.open_picker(picker);
            screen.input(&input);
            screen.menu(&input);
        }
        "4" => {
            turn(&mut transcript, "round", &highlighter);
            transcript.push(Block::Round(round(vec![call(
                "Run",
                Target::Text("cargo clippy -p parser -- -D warnings".to_owned()),
                CallState::Running(Duration::from_millis(6_200)),
                Measure::None,
                None,
                Detail::Output(
                    "    Checking unicode-ident v1.0.13\n    Checking proc-macro2 v1.0.89\n    Checking parser v0.3.0 (crates/parser)\n    Building [=======>          ] 41/97: syn\n"
                        .to_owned(),
                ),
            )])));
            screen.transcript(&mut transcript, 0);
            // The plan, focused on the item being worked on, then the usage
            // panel's line, and the blank row that ends the block.
            screen.lines(
                view::plan::plan_rows(&screen_plan(), Some(1), width, &look)
                    .into_iter()
                    .map(|row| row.line)
                    .collect(),
            );
            screen.lines(screen_usage(width, &look));
            screen.lines(vec![Line::new()]);
            screen.input_running(true);
            screen.hint(&hint(
                Hint::Running {
                    elapsed: Duration::from_secs(14),
                    tick: 0,
                    keys: RunningKeys::Of(
                        ["space skips", "e edits", "a adds", "esc back to the input"]
                            .map(str::to_owned)
                            .to_vec(),
                    ),
                },
                true,
            ));
        }
        "5" => {
            for block in [
                Block::Notice("AGENTS.md changed; the new text applies from the next request.".to_owned()),
                Block::User("Now make the same change in the JSON lexer.".to_owned()),
                Block::Notice(
                    "Context compacted in the background: 148.2k → 23.6k tokens (a 3.1k summary, the last 6 turns kept). ctrl+o shows the summary."
                        .to_owned(),
                ),
                Block::Notice("The sandbox stopped a command writing ~/.cargo/credentials.toml.".to_owned()),
                Block::Notice(
                    "Fast mode was refused (429), so this request runs at standard speed; fast resumes after 14:32."
                        .to_owned(),
                ),
                Block::Notice("The provider is overloaded (529). Retrying in 4s, attempt 2 of 5.".to_owned()),
                Block::Error {
                    message: "The request failed after 5 attempts: the connection to the provider was reset."
                        .to_owned(),
                    hint: Some("Press enter to try the turn again, or pick another model with /model.".to_owned()),
                },
            ] {
                transcript.push(block);
            }
            screen.transcript(&mut transcript, 0);
            screen.input(&prompt("", width));
            screen.hint(&hint(
                Hint::Cost {
                    turn: "$0.03".to_owned(),
                    session: "$0.61".to_owned(),
                },
                false,
            ));
        }
        "8a" | "8b" => {
            let open = spec.id == "8b";
            screen.lines(view::viewer::title(&viewer_title(), width, &look));
            let blank = || vec![Line::new()];
            screen.lines(view::user(
                "Run the tests and the formatter before I start on the lexer.",
                width,
                &look,
            ));
            screen.lines(blank());
            let earlier = round(vec![
                call(
                    "Run",
                    Target::Text("cargo test --workspace".to_owned()),
                    CallState::Succeeded,
                    Measure::Note("46 passed".to_owned()),
                    Some(5_100),
                    Detail::Output(
                        "test result: ok. 46 passed; 0 failed; 0 ignored; finished in 0.31s\n"
                            .to_owned(),
                    ),
                ),
                call(
                    "Run",
                    Target::Text("cargo fmt --check".to_owned()),
                    CallState::Succeeded,
                    Measure::Note("clean".to_owned()),
                    Some(400),
                    Detail::None,
                ),
            ]);
            screen.lines(view::round(
                &earlier,
                Placement::Viewer {
                    selected: false,
                    open: false,
                },
                0,
                width,
                &look,
            ));
            screen.lines(blank());
            screen.lines(Markdown::render(
                cineris_tui::markdown::Flavor::Answer,
                PREVIOUS,
                width,
                &look,
            ));
            screen.lines(blank());
            screen.lines(view::user(PROMPT, width, &look));
            screen.lines(blank());
            screen.lines(view::reasoning(
                THOUGHT,
                Duration::from_secs(6),
                width,
                &look,
            ));
            screen.lines(blank());
            screen.lines(Markdown::render(
                cineris_tui::markdown::Flavor::Answer,
                FIRST,
                width,
                &look,
            ));
            screen.lines(blank());
            screen.lines(view::round(
                &round(vec![read(), edit(), tests_failed()]),
                Placement::Viewer {
                    selected: true,
                    open,
                },
                0,
                width,
                &look,
            ));
            screen.lines(blank());
            let code: Vec<&str> = CODE.iter().chain(&CODE_REST).copied().collect();
            let mut whole = Transcript::new();
            let mut stream = Markdown::answer().with_highlighter(highlighter.clone());
            stream.push(&format!("{SECOND}\n\n```rust\n{}\n```\n", code.join("\n")));
            stream.finish();
            whole.push(Block::Answer(stream));
            let mut lines = whole.commit(width, &look);
            assert!(
                highlighter.settle(SETTLE),
                "the highlighter answered in time"
            );
            lines.extend(whole.commit(width, &look));
            screen.lines(lines);
            screen.lines(view::viewer::keys(
                &ViewerKeys::Boxes {
                    selected: 2,
                    count: 2,
                    open,
                },
                width,
                &look,
            ));
            screen.live_from = 0;
        }
        other => panic!("no screen {other}"),
    }
    Screen {
        rows: screen.rows,
        live_from: screen.live_from,
    }
}
