//! The session the reference screens show (`docs/architecture/tui-look/`):
//! turn 3 in a Rust workspace, where the agent fixes digit separators in a
//! lexer, runs the tests, sees one fail, and streams the fix while the user
//! types a steering message. Each screen is composed from the real views as
//! the interactive application will compose them: this step's, and step
//! 21's input box and menus. The rows other steps draw are marked as not
//! built yet, with the step that builds them.

use std::time::Duration;

use cortex_term::{ColorDepth, Event, Key, KeyCode, Line};
use cortex_tui::complete::{Menu, Row as Choice, Toggle};
use cortex_tui::glyphs::Glyphs;
use cortex_tui::highlight::Highlighter;
use cortex_tui::history::MemoryHistory;
use cortex_tui::input::Input;
use cortex_tui::keys::KeyMap;
use cortex_tui::markdown::Markdown;
use cortex_tui::theme::{Rgb, Theme};
use cortex_tui::transcript::{Block, Reasoning, Transcript};
use cortex_tui::view::{
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
pub(crate) const SPECS: [Spec; 17] = [
    spec("0"),
    spec("1"),
    spec("1b"),
    spec("1c"),
    spec("1d"),
    spec("2a"),
    spec("2b"),
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

    fn not_built(&mut self, count: usize, what: &'static str) {
        self.rows
            .extend(std::iter::repeat_n(Row::NotBuilt(what), count));
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

    /// The input box empty, where the reference shows what step 21's box
    /// does not say: its borders, and its row not built.
    fn input_unsaid(&mut self) {
        let lines = view::input::input(&prompt("", self.width), self.look, self.width).lines;
        let [top, _, bottom] = <[Line; 3]>::try_from(lines).expect("an empty box is three rows");
        self.rows.push(Row::Built(top));
        self.not_built(1, UNSAID);
        self.rows.push(Row::Built(bottom));
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

/// What the steering message says.
const STEER: &str = "also run clippy when that's done";

const UNSAID: &str =
    "the input box's placeholder while a turn runs, or with the focus elsewhere (not built)";
const DIALOG: &str = "dialog (step 23)";
const PANELS: &str = "panels (step 24)";
const VIEWER: &str = "viewer's title, rules and keys (step 23)";

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
            screen.input_unsaid();
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
            screen.input_unsaid();
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
            screen.not_built(if spec.id == "2a" { 11 } else { 10 }, DIALOG);
            screen.hint(&hint(
                Hint::Keys(vec![
                    "esc denies".to_owned(),
                    "ctrl+o shows it whole".to_owned(),
                ]),
                true,
            ));
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
            screen.not_built(6, PANELS);
            screen.not_built(1, PANELS);
            screen.input_unsaid();
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
            screen.not_built(2, VIEWER);
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
                cortex_tui::markdown::Flavor::Answer,
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
                cortex_tui::markdown::Flavor::Answer,
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
            screen.not_built(2, VIEWER);
            screen.live_from = 0;
        }
        other => panic!("no screen {other}"),
    }
    Screen {
        rows: screen.rows,
        live_from: screen.live_from,
    }
}
