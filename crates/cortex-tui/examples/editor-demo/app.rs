//! The editor demo: fake streamed text above the real input, for measuring
//! key latency with the editor, its key map and its menus in the loop
//! (`docs/performance-budgets.md`, the key-latency rows).
//!
//! It is the example `editor-demo` (`cargo run --release -p cortex-tui
//! --example editor-demo --features demo -- rate=2000 auto`), and the same
//! code runs as a child of `cortex-tui`'s own test binary in `cargo xtask
//! perf`.
//!
//! **The input box here is the demo's own**, until step 22's box and theme
//! are in: a rounded border and `◆`, drawn plainly. Everything under it,
//! the key map, the editor, the menu and its filter, is the interface's.
//!
//! # Options
//!
//! | Option | Meaning |
//! |---|---|
//! | `rate=N` | Tokens a second while streaming (400). |
//! | `turn-ms=N` | How long a turn streams (12000). |
//! | `auto` | Start a turn at once. |
//! | `quit-after-turn` | Quit when the turn ends. |
//! | `files=N` | Generate N workspace paths for `@` completion (0). |
//! | `stats=PATH` | Write the frame and key timings there on exit, as JSON. |
//!
//! # Keys
//!
//! The interface's default keys. `Enter` sends the prompt, which starts a
//! turn; `Ctrl+C` stops it, or clears the prompt; `Ctrl+D` on an empty
//! prompt quits.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a program of its own: it writes its statistics file and streams at a set pace"
)]
#![allow(
    unreachable_pub,
    reason = "the example's binary and the test binary both use this module"
)]

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cortex_term::{
    Cursor, Event, Frame, Inline, Line, Overrides, Scheduler, Span, Style, Terminal, Wake,
};
use cortex_tui::history::MemoryHistory;
use cortex_tui::input::{Input, Outcome};
use cortex_tui::keys::KeyMap;

/// The first word of the demo's status row: what the measurement looks for
/// to know a frame is complete.
pub const STATUS_MARK: &str = "editor-demo:";
/// What starts the input box's text row, after its border.
pub const PROMPT: &str = "\u{2502} \u{25C6} ";

const WORDS: &[&str] = &[
    "the",
    "stream",
    "carries",
    "plain",
    "words",
    "of",
    "varied",
    "length",
    "so",
    "that",
    "lines",
    "wrap",
    "at",
    "different",
    "places",
    "and",
    "commit",
    "as",
    "soon",
    "stable",
];

/// What the demo was asked to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Tokens a second.
    pub rate: u64,
    /// How long a turn streams.
    pub turn: Duration,
    /// Start a turn at once.
    pub auto: bool,
    /// Quit when the turn ends.
    pub quit_after_turn: bool,
    /// Workspace paths to generate for `@`.
    pub files: usize,
    /// Where to write the timings.
    pub stats: Option<String>,
}

impl Options {
    /// Reads `name=value` options.
    ///
    /// # Errors
    ///
    /// When an option is not known, or a number is not one.
    pub fn parse<'a>(arguments: impl IntoIterator<Item = &'a str>) -> Result<Self, String> {
        let mut options = Self {
            rate: 400,
            turn: Duration::from_secs(12),
            ..Self::default()
        };
        for argument in arguments {
            let (name, value) = argument.split_once('=').unwrap_or((argument, ""));
            let number = || {
                value
                    .parse::<u64>()
                    .map_err(|_| format!("`{name}` takes a number, not `{value}`"))
            };
            match name {
                "rate" => options.rate = number()?.max(1),
                "turn-ms" => options.turn = Duration::from_millis(number()?),
                "auto" => options.auto = true,
                "quit-after-turn" => options.quit_after_turn = true,
                "files" => options.files = usize::try_from(number()?).unwrap_or(0),
                "stats" => options.stats = Some(value.to_owned()),
                "" => {}
                _ => return Err(format!("unknown option `{name}`")),
            }
        }
        Ok(options)
    }
}

#[derive(Debug, Default)]
struct Stream {
    tokens: Mutex<Vec<u64>>,
    done: AtomicBool,
    stop: AtomicBool,
}

fn start_turn(stream: &Arc<Stream>, options: &Options, waker: cortex_term::Waker) {
    stream.done.store(false, Ordering::SeqCst);
    stream.stop.store(false, Ordering::SeqCst);
    let stream = Arc::clone(stream);
    let (rate, turn) = (options.rate, options.turn);
    std::thread::spawn(move || {
        let started = Instant::now();
        let total = u64::try_from(turn.as_millis()).unwrap_or(u64::MAX) * rate / 1000;
        let mut sent = 0;
        while sent < total && !stream.stop.load(Ordering::SeqCst) {
            let due =
                u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX) * rate / 1_000_000;
            let due = due.min(total);
            if due > sent {
                stream
                    .tokens
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend(sent..due);
                sent = due;
                waker.wake();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        stream.done.store(true, Ordering::SeqCst);
        waker.wake();
    });
}

/// The streamed text: lines to commit and the growing last line.
#[derive(Debug, Default)]
struct Text {
    commit: Vec<Line>,
    current: String,
    tokens: u64,
}

impl Text {
    fn push(&mut self, token: u64, width: usize) {
        self.tokens += 1;
        let word = WORDS[usize::try_from(token).unwrap_or(0) % WORDS.len()];
        if !self.current.is_empty() && self.current.len() + 1 + word.len() > width {
            self.commit
                .push(Line::plain(std::mem::take(&mut self.current)));
        }
        if !self.current.is_empty() {
            self.current.push(' ');
        }
        self.current.push_str(word);
    }

    fn finish(&mut self) {
        if !self.current.is_empty() {
            self.commit
                .push(Line::plain(std::mem::take(&mut self.current)));
        }
    }
}

/// Generated paths for `@` completion.
fn workspace(count: usize) -> Vec<String> {
    (0..count)
        .map(|n| {
            let a = WORDS[n % WORDS.len()];
            let b = WORDS[(n / WORDS.len()) % WORDS.len()];
            format!("src/{a}/{b}/file_{n}.rs")
        })
        .collect()
}

/// The live region: the streaming line, the input box, a menu's rows, the
/// status row; and where the cursor goes.
fn live(input: &Input, text: &Text, streaming: bool, width: usize) -> (Vec<Line>, Cursor) {
    let dim = Style::PLAIN.dim();
    let mut lines = vec![Line::plain(text.current.clone()), Line::new()];
    let inner = width.saturating_sub(2);
    lines.push(Line::styled(
        format!("\u{256D}{}\u{256E}", "\u{2500}".repeat(inner)),
        dim,
    ));
    let editor = input.editor();
    let layout = editor.layout(input.width());
    let top = lines.len();
    let shown = layout
        .rows
        .iter()
        .skip(input.scroll())
        .take(cortex_tui::input::MAX_ROWS);
    for (index, row) in shown.enumerate() {
        let lead = if index == 0 && input.scroll() == 0 {
            PROMPT
        } else {
            "\u{2502}   "
        };
        let body = cortex_term::sanitize(&editor.text()[row.range.clone()]).into_owned();
        let used = 4 + cortex_base::text::width(&body);
        let pad = width.saturating_sub(used + 1);
        lines.push(
            Line::new()
                .with(Span::plain(lead.to_owned()))
                .with(Span::plain(body))
                .with(Span::plain(" ".repeat(pad)))
                .with(Span::new("\u{2502}", dim)),
        );
    }
    lines.push(Line::styled(
        format!("\u{2570}{}\u{256F}", "\u{2500}".repeat(inner)),
        dim,
    ));
    if let Some(menu) = input.menu() {
        for at in menu.visible() {
            let mark = if menu.selected() == Some(at) {
                "  \u{25C6} "
            } else {
                "    "
            };
            lines.push(Line::plain(format!("{mark}{}", menu.rows[at].label)));
        }
    } else {
        let state = if streaming { "streaming" } else { "ready" };
        lines.push(Line::styled(
            format!("  {STATUS_MARK} {state}, {} tokens", text.tokens),
            dim,
        ));
    }
    let cursor = Cursor {
        row: top + layout.cursor_row - input.scroll(),
        col: 4 + layout.cursor_col,
    };
    (lines, cursor)
}

/// Runs the demo until it is told to quit.
///
/// # Errors
///
/// When the terminal cannot be used.
pub fn run(options: &Options) -> io::Result<()> {
    let mut terminal = Terminal::enter(Overrides::default())?;
    let (mut cols, rows) = terminal.size()?;
    let mut presenter = Inline::new(cols, rows);
    let mut scheduler = Scheduler::new();
    let mut input = Input::new(KeyMap::defaults(), Box::new(MemoryHistory::new()));
    input.set_width(usize::from(presenter.width()).saturating_sub(6));
    if options.files > 0 {
        input.set_files(workspace(options.files));
    }
    let stream = Arc::new(Stream::default());
    let mut text = Text::default();
    let mut streaming = false;
    let mut frames: Vec<(u128, usize, u128)> = Vec::new();
    let mut keys: Vec<u128> = Vec::new();
    let began = Instant::now();
    let mut key_read: Option<Instant> = None;

    if options.auto {
        streaming = true;
        input.set_running(true);
        start_turn(&stream, options, terminal.waker());
    }
    scheduler.input();
    loop {
        let tokens =
            std::mem::take(&mut *stream.tokens.lock().unwrap_or_else(PoisonError::into_inner));
        if !tokens.is_empty() {
            for token in tokens {
                text.push(token, usize::from(presenter.width()));
            }
            scheduler.stream();
        }
        if streaming
            && stream.done.load(Ordering::SeqCst)
            && stream.tokens.lock().map_or(true, |t| t.is_empty())
        {
            streaming = false;
            input.set_running(false);
            text.finish();
            scheduler.stream();
            if options.quit_after_turn {
                break;
            }
        }
        let now = Instant::now();
        let timeout = match scheduler.wake(now) {
            Wake::Now => {
                let started = Instant::now();
                let (lines, cursor) =
                    live(&input, &text, streaming, usize::from(presenter.width()));
                let commit = std::mem::take(&mut text.commit);
                let caps = *terminal.caps();
                let drawn = presenter.draw(
                    &Frame {
                        commit: &commit,
                        live: &lines,
                        cursor: Some(cursor),
                    },
                    &caps,
                    &mut terminal,
                )?;
                let finished = Instant::now();
                if let cortex_term::Drawn::Written(bytes) = drawn {
                    frames.push((
                        (finished - started).as_micros(),
                        bytes,
                        (finished - began).as_micros(),
                    ));
                }
                if let Some(read) = key_read.take() {
                    keys.push((finished - read).as_micros());
                }
                scheduler.drawn(now);
                continue;
            }
            Wake::At(at) => Some(at.saturating_duration_since(now)),
            Wake::Idle => None,
        };
        let Some(event) = terminal.next(timeout)? else {
            continue;
        };
        match &event {
            Event::Key(_) | Event::Paste(_) => {
                key_read = Some(Instant::now());
                match input.handle(&event) {
                    Outcome::Quit => break,
                    Outcome::Interrupt => stream.stop.store(true, Ordering::SeqCst),
                    Outcome::Redraw => presenter.redraw(),
                    Outcome::Submit { .. } if !streaming => {
                        streaming = true;
                        input.set_running(true);
                        start_turn(&stream, options, terminal.waker());
                    }
                    _ => {}
                }
                scheduler.input();
            }
            Event::Resize {
                cols: new_cols,
                rows,
            } => {
                cols = *new_cols;
                presenter.resize(cols, *rows);
                input.set_width(usize::from(presenter.width()).saturating_sub(6));
                scheduler.input();
            }
            Event::Resumed => {
                presenter.restart();
                scheduler.input();
            }
            Event::Focus(_) | Event::Reply(_) => {}
        }
    }
    let caps = *terminal.caps();
    presenter.finish(&caps, &mut terminal)?;
    terminal.leave()?;
    if let Some(path) = &options.stats {
        let frames: Vec<String> = frames
            .iter()
            .map(|(micros, bytes, at)| format!("[{micros},{bytes},{at}]"))
            .collect();
        let keys: Vec<String> = keys.iter().map(u128::to_string).collect();
        std::fs::write(
            path,
            format!(
                "{{\"frames\":[{}],\"keys\":[{}]}}\n",
                frames.join(","),
                keys.join(",")
            ),
        )?;
    }
    Ok(())
}
