//! The views' share of a frame, measured: a 30-row live region of real
//! views (a streaming answer's tail, a running round's open box, the input
//! box's rows and the hint row) built and drawn through the inline presenter
//! into a buffer, frame after frame, while the answer streams; and what the
//! highlighter costs the first time it is used.
//!
//! Ignored in every ordinary run. `cargo xtask perf` runs it optimised,
//! after the terminal layer's demo, and prints the table. The budget is
//! `docs/performance-budgets.md`'s "Frame cost with a 30-line live region":
//! 2 ms at the 99th percentile.

use std::time::{Duration, Instant};

use cineris_term::{Capabilities, ColorDepth, Frame, Inline, Line, Overrides, Platform};
use cineris_tui::glyphs::Glyphs;
use cineris_tui::highlight::Highlighter;
use cineris_tui::markdown::Markdown;
use cineris_tui::theme::Theme;
use cineris_tui::transcript::{Block, Transcript};
use cineris_tui::view::{
    self, Call, CallState, Change, Detail, DiffLine, Hint, HintRow, Look, Measure, Round,
    RunningKeys, Target,
};

const ROWS: usize = 30;

fn percentile(sorted: &[Duration], share: f64) -> Duration {
    // The index of the share, rounded down, of a non-empty list.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "an index into a list of a few thousand"
    )]
    let at = ((sorted.len() - 1) as f64 * share) as usize;
    sorted[at]
}

fn running_round(lines: usize) -> Round {
    let diff = (0..lines)
        .map(|n| DiffLine {
            number: 100 + n as u64,
            change: if n % 3 == 0 {
                Change::Added
            } else {
                Change::Same
            },
            text: format!("    let value_{n} = compute(input, {n}) + offset; // a line of code"),
            emphasis: Vec::new(),
        })
        .collect();
    Round {
        calls: vec![
            Call {
                verb: "Edit".to_owned(),
                target: Target::Path("crates/parser/src/lex.rs".to_owned()),
                state: CallState::Succeeded,
                measure: Measure::Changes {
                    added: 9,
                    removed: 2,
                },
                took: None,
                detail: Detail::Diff(diff),
                hook: None,
            },
            Call {
                verb: "Run".to_owned(),
                target: Target::Text("cargo test -p parser".to_owned()),
                state: CallState::Running(Duration::from_secs(1)),
                measure: Measure::None,
                took: None,
                detail: Detail::None,
                hook: None,
            },
        ],
        open: Some(true),
    }
}

#[test]
#[ignore = "a measurement: run by `cargo xtask perf`, optimised"]
fn perf_frames_of_the_views_with_a_30_row_live_region() {
    let mut look = Look::new(
        Theme::new(ColorDepth::TrueColor, Some((12, 12, 12))),
        &Glyphs::UNICODE,
    );
    look.tool_detail = 40;
    let caps = Capabilities::detect(
        |_| None,
        Platform::Unix,
        Overrides {
            color: Some(ColorDepth::TrueColor),
            sync: Some(true),
            ..Overrides::default()
        },
    );
    let width = 119;
    let mut transcript = Transcript::new();
    let mut answer = Markdown::answer();
    answer.push("Here is the change, and why it works:\n\n```rust\n");
    transcript.push(Block::Answer(answer));
    let mut presenter = Inline::new(120, 60);
    let mut times = Vec::new();
    let mut bytes = Vec::new();
    let tokens =
        "let separator = self.peek_next().is_some_and(|d| d.is_ascii_digit()); ".repeat(200);
    let input: Vec<Line> = (0..3)
        .map(|_| Line::plain("│ ◆ also run clippy when that's done"))
        .collect();
    for (tick, token) in tokens.split_inclusive(' ').enumerate() {
        let started = Instant::now();
        if let Some(Block::Answer(answer)) = transcript.last_mut() {
            answer.push(token);
            if tick % 12 == 11 {
                answer.push("\n");
            }
        }
        let commit = transcript.commit(width, &look);
        let mut live = transcript.live(width, &look, tick);
        live.extend(view::round(
            &running_round(ROWS),
            view::Placement::Live,
            tick,
            width,
            &look,
        ));
        live.truncate(ROWS - 4);
        while live.len() < ROWS - 4 {
            live.push(Line::new());
        }
        live.extend(input.iter().cloned());
        live.push(view::hint_row(
            &HintRow {
                left: Hint::Running {
                    elapsed: Duration::from_secs(14),
                    tick,
                    keys: RunningKeys::Steer,
                },
                model: "opus-5.5".to_owned(),
                effort: Some("high".to_owned()),
                speed: None,
                context: Some(62),
                budget: None,
                limit: None,
                access: None,
                extras: Vec::new(),
            },
            width,
            &look,
        ));
        assert_eq!(live.len(), ROWS);
        let mut sink = Vec::new();
        presenter
            .draw(
                &Frame {
                    commit: &commit,
                    live: &live,
                    cursor: None,
                },
                &caps,
                &mut sink,
            )
            .unwrap();
        times.push(started.elapsed());
        bytes.push(sink.len());
    }
    times.sort_unstable();
    bytes.sort_unstable();
    println!(
        "frames of the views, a 30-row live region, {} frames:",
        times.len()
    );
    println!(
        "  build and draw: median {:.3} ms, 99th {:.3} ms, largest {:.3} ms (budget: 2 ms at the 99th)",
        percentile(&times, 0.5).as_secs_f64() * 1e3,
        percentile(&times, 0.99).as_secs_f64() * 1e3,
        times.last().unwrap().as_secs_f64() * 1e3
    );
    println!(
        "  bytes a frame: median {}, largest {}",
        bytes[bytes.len() / 2],
        bytes.last().unwrap()
    );
    assert!(
        percentile(&times, 0.99) <= Duration::from_millis(2),
        "over the frame budget"
    );

    // The highlighter's first use: from the first block to 200 short lines
    // coloured, all on its own thread.
    let highlighter = Highlighter::new(|| {});
    let started = Instant::now();
    let block = highlighter.open("rust").unwrap();
    for n in 0..200 {
        highlighter.line(block, n, &format!("    if c == '_' && self.peek_next().is_some_and(|d| d.is_ascii_digit()) {{ return {n}; }}"));
    }
    assert!(highlighter.settle(Duration::from_secs(30)));
    let cold = started.elapsed();
    let started = Instant::now();
    for n in 200..400 {
        highlighter.line(
            block,
            n,
            &format!("    let value_{n} = compute(input, {n}) + offset; // a line of code"),
        );
    }
    assert!(highlighter.settle(Duration::from_secs(30)));
    let warm = started.elapsed();
    println!(
        "highlighting on its own thread: 200 lines from cold {:.1} ms (grammars loaded and patterns compiled on first use), 200 more {:.1} ms",
        cold.as_secs_f64() * 1e3,
        warm.as_secs_f64() * 1e3
    );
}
