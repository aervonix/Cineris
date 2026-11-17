//! The full-screen presenter's view (step 25; `docs/steps/25/answered.md`,
//! point 2): the same rows as the inline screen, a window that follows the
//! end until it is moved up, a click and the wheel, and a resize that lays
//! everything out again.

use std::time::Instant;

use cortex_protocol::event::{
    Delta, Event, ItemCompleted, ItemDelta, ItemKind, ItemStarted, TextDelta, TurnCompleted,
    TurnOutcome, TurnStarted,
};
use cortex_protocol::item::{AssistantMessageItem, StopReason, UserMessageItem};
use cortex_protocol::message::Part;
use cortex_protocol::{CallId, Item, ItemId, TurnId};
use cortex_term::{Button, Event as TermEvent, Line, Mods, Mouse, MouseKind, Wheel};

use super::{key, text};
use crate::app::{Inbound, Presenter, Settings};
use crate::testkit::{Scripted, app_with_settings, drain, event, opened};

/// An app with the presenter given, its session open, in a window of
/// `width` usable columns and `rows` rows.
fn opened_app(
    presenter: Presenter,
    width: usize,
    rows: usize,
    now: Instant,
) -> (super::App, Scripted) {
    let settings = Settings {
        presenter,
        ..Settings::default()
    };
    let (mut app, link) = app_with_settings(now, settings);
    app.resize(width, rows);
    opened(&mut app, &link, now);
    (app, link)
}

fn in_turn(turn: u64, inbound: Inbound) -> Inbound {
    match inbound {
        Inbound::Event(mut event) => {
            event.turn_id = Some(TurnId::from(format!("t_{turn}").as_str()));
            Inbound::Event(event)
        }
        other => other,
    }
}

/// One turn's events, in the order they come: the prompt, the answer
/// streamed in `pieces`, a command with its output, and the turn's end.
/// Each inner list is what arrives between two frames.
fn turn(turn: u64, prompt: &str, pieces: &[&str], output: &str) -> Vec<Vec<Inbound>> {
    let answer = ItemId::from(format!("a{turn}").as_str());
    let whole: String = pieces.concat();
    let mut steps = vec![vec![
        event(Event::TurnStarted(TurnStarted {})),
        event(Event::ItemCompleted(ItemCompleted {
            item: Item::UserMessage(UserMessageItem {
                id: ItemId::from(format!("u{turn}").as_str()),
                parts: vec![Part::text(prompt)],
                delivery: None,
                created_at: None,
            }),
        })),
        event(Event::ItemStarted(ItemStarted {
            item_id: answer.clone(),
            kind: ItemKind::AssistantMessage,
            tool: None,
            call_id: None,
        })),
    ]];
    for piece in pieces {
        steps.push(vec![event(Event::ItemDelta(ItemDelta {
            item_id: answer.clone(),
            delta: Delta::Text(TextDelta {
                text: (*piece).to_owned(),
            }),
        }))]);
    }
    let call = CallId::from(format!("c{turn}").as_str());
    steps.push(vec![
        event(Event::ItemCompleted(ItemCompleted {
            item: Item::AssistantMessage(AssistantMessageItem {
                id: answer,
                parts: vec![
                    Part::text(&whole),
                    Part::ToolCall(cortex_protocol::message::ToolCallPart {
                        id: call.clone(),
                        name: "shell".to_owned(),
                        arguments: serde_json::json!({"command": "cargo test"}),
                        raw: None,
                    }),
                ],
                stop_reason: Some(StopReason::ToolCalls),
                raw_stop: None,
                stop_detail: None,
                truncated: false,
                origin: None,
                usage: None,
                error: None,
                created_at: None,
            }),
        })),
        event(Event::ItemCompleted(ItemCompleted {
            item: Item::ToolResult(cortex_protocol::item::ToolResultItem {
                id: ItemId::from(format!("r{turn}").as_str()),
                call_id: call,
                tool: "shell".to_owned(),
                outcome: cortex_protocol::item::ToolOutcome::Error,
                content: vec![Part::text(output)],
                payload: Some(serde_json::json!({"exit_code": 101, "output": output})),
                artifacts: Vec::new(),
                duration_ms: Some(3400),
                created_at: None,
            }),
        })),
        event(Event::TurnCompleted(TurnCompleted {
            outcome: TurnOutcome::Completed,
            stop_reason: Some(StopReason::End),
            error: None,
            usage: Default::default(),
            verification: None,
            duration_ms: Some(1200),
        })),
    ]);
    steps
        .into_iter()
        .map(|step| {
            step.into_iter()
                .map(|inbound| in_turn(turn, inbound))
                .collect()
        })
        .collect()
}

const ANSWER: &[&str] = &[
    "`scan_number` stops at the first ",
    "character that is not a digit, so `1_000` ends after the `1`. I'll read the ",
    "scanner and allow `_` between digits.\n\n```rust\nif c == '_' {\n    self.bump();\n",
    "    continue;\n}\n```\n\nThen the **test**: a separator must be followed ",
    "by a digit, so `1_` is an error.\n",
];

const OUTPUT: &str = "test lex::numbers::trailing_separator ... FAILED\n\
                      assertion failed: lex(\"1_\").is_err()\n\
                      test result: FAILED. 47 passed; 1 failed; 0 ignored";

/// Rows without the blank rows at their end.
fn trimmed(mut rows: Vec<Line>) -> Vec<Line> {
    while rows.last().is_some_and(|row| row.width() == 0) {
        rows.pop();
    }
    rows
}

/// The rows above the bottom of the inline screen (its scrollback, then its
/// live region less the bottom) and of the full screen, the bottom being
/// `bottom` rows.
fn transcripts(
    inline: &mut super::App,
    full: &mut super::App,
    scrollback: &mut Vec<Line>,
    now: Instant,
) -> (Vec<Line>, Vec<Line>) {
    let (commit, mut live, _) = inline.frame(now);
    scrollback.extend(commit);
    let (rows, _) = full.screen_frame(now);
    // The bottom is the same code in both: the input box and the hint row.
    let bottom = 4;
    assert_eq!(
        text(&rows[rows.len() - bottom..]),
        text(&live[live.len() - bottom..]),
        "the bottoms"
    );
    live.truncate(live.len() - bottom);
    let mut shown = scrollback.clone();
    shown.extend(live);
    (
        trimmed(shown),
        trimmed(rows[..rows.len() - bottom].to_vec()),
    )
}

/// The same session, frame by frame as it streams, draws the same rows
/// under both presenters, styles included: the inline screen's scrollback
/// and live rows, and the full screen's window from its first row (step
/// 25's "Done when"). At three widths.
#[test]
fn the_same_rows_are_drawn_under_both_presenters_frame_by_frame() {
    for width in [59, 79, 119] {
        let now = Instant::now();
        let (mut inline, inline_link) = opened_app(Presenter::Inline, width, 400, now);
        let (mut full, full_link) = opened_app(Presenter::Fullscreen, width, 400, now);
        let mut scrollback = Vec::new();
        for (n, prompt) in ["Fix scan_number.", "And the test."].iter().enumerate() {
            for step in turn(n as u64 + 1, prompt, ANSWER, OUTPUT) {
                for inbound in step {
                    inline_link.give(inbound.clone());
                    full_link.give(inbound);
                }
                drain(&mut inline, now);
                drain(&mut full, now);
                let (shown, drawn) = transcripts(&mut inline, &mut full, &mut scrollback, now);
                assert_eq!(text(&drawn), text(&shown), "at {width} columns");
                assert_eq!(drawn, shown, "the styles at {width} columns");
            }
        }
        assert!(full.full_screen() && !inline.full_screen());
    }
}

/// Lines of a transcript long enough to scroll: `count` turns.
fn long_session(presenter: Presenter, count: u64, now: Instant) -> (super::App, Scripted) {
    let (mut app, link) = opened_app(presenter, 79, 20, now);
    for n in 1..=count {
        for step in turn(n, &format!("Turn {n}."), ANSWER, OUTPUT) {
            for inbound in step {
                link.give(inbound);
            }
            drain(&mut app, now);
            if app.full_screen() {
                let _ = app.screen_frame(now);
            } else {
                let _ = app.frame(now);
            }
        }
    }
    (app, link)
}

fn rows_text(app: &mut super::App, now: Instant) -> Vec<String> {
    text(&app.screen_frame(now).0)
}

/// The window follows the end; `PageUp` stops it, and what arrives then
/// does not move it, while its last row says there is more below;
/// `PageDown` to the end, and a prompt sent, follow again.
#[test]
fn the_window_follows_the_end_until_it_is_moved_up() {
    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 4, now);
    let rows = rows_text(&mut app, now);
    assert_eq!(rows.len(), 20);
    assert!(
        rows.iter().any(|row| row.contains("test result: FAILED")),
        "{rows:#?}"
    );
    key(&mut app, "pageup", now);
    let held = rows_text(&mut app, now);
    assert_ne!(held, rows);
    let transcript = &held[..held.len() - 4];
    assert!(
        transcript
            .last()
            .is_some_and(|row| row.contains("more below")),
        "{held:#?}"
    );
    // A new turn arrives: the window does not move.
    for step in turn(5, "Turn 5.", ANSWER, OUTPUT) {
        for inbound in step {
            link.give(inbound);
        }
        drain(&mut app, now);
    }
    let after = rows_text(&mut app, now);
    assert_eq!(after[..after.len() - 5], held[..held.len() - 5]);
    for _ in 0..40 {
        key(&mut app, "pagedown", now);
    }
    let end = rows_text(&mut app, now);
    assert!(
        !end.iter().any(|row| row.contains("more below")),
        "{end:#?}"
    );
    // The end: the last turn's box closes the transcript area.
    assert!(end[15].starts_with('╰'), "{end:#?}");
    assert_ne!(end, after);
    // Up a row, then a prompt sent: the end again.
    key(&mut app, "shift+up", now);
    assert!(
        rows_text(&mut app, now)
            .iter()
            .any(|row| row.contains("more below"))
    );
    super::type_text(&mut app, "next", now);
    key(&mut app, "enter", now);
    assert!(
        !rows_text(&mut app, now)
            .iter()
            .any(|row| row.contains("more below"))
    );
}

fn click(app: &mut super::App, col: u16, row: u16, now: Instant) {
    for kind in [
        MouseKind::Press(Button::Left),
        MouseKind::Release(Some(Button::Left)),
    ] {
        app.terminal(
            &TermEvent::Mouse(Mouse {
                kind,
                col,
                row,
                mods: Mods::NONE,
            }),
            now,
        );
    }
}

fn wheel(app: &mut super::App, up: bool, now: Instant) {
    app.terminal(
        &TermEvent::Mouse(Mouse {
            kind: MouseKind::Wheel(if up { Wheel::Up } else { Wheel::Down }),
            col: 5,
            row: 5,
            mods: Mods::NONE,
        }),
        now,
    );
}

/// The wheel scrolls three rows a notch and stops following; a click on a
/// committed box opens or closes it, and anywhere else does nothing.
#[test]
fn the_wheel_scrolls_and_a_click_opens_or_closes_a_box() {
    let now = Instant::now();
    let (mut app, _link) = long_session(Presenter::Fullscreen, 2, now);
    let end = rows_text(&mut app, now);
    wheel(&mut app, true, now);
    let up = rows_text(&mut app, now);
    assert_eq!(up[3], end[0], "three rows down the screen: {up:#?}");
    wheel(&mut app, false, now);
    assert_eq!(rows_text(&mut app, now), end);
    // The failed command's box shows its output's tail, as it committed:
    // a click on its call's row closes it, a second opens it again.
    let call = end
        .iter()
        .position(|row| row.contains("Run cargo test"))
        .expect("the box's call row");
    assert!(end.iter().any(|row| row.contains("trailing_separator")));
    click(&mut app, 4, u16::try_from(call).unwrap(), now);
    let closed = rows_text(&mut app, now);
    assert!(
        !closed.iter().any(|row| row.contains("trailing_separator")),
        "{closed:#?}"
    );
    let call = closed
        .iter()
        .position(|row| row.contains("Run cargo test"))
        .expect("the box's call row");
    click(&mut app, 4, u16::try_from(call).unwrap(), now);
    assert!(
        rows_text(&mut app, now)
            .iter()
            .any(|row| row.contains("trailing_separator"))
    );
    // A click on the answer's text does nothing.
    let answer = rows_text(&mut app, now)
        .iter()
        .position(|row| row.contains("separator must be followed"))
        .unwrap();
    let before = rows_text(&mut app, now);
    click(&mut app, 4, u16::try_from(answer).unwrap(), now);
    assert_eq!(rows_text(&mut app, now), before);
    // A press and a release on different cells is a drag, not a click.
    app.terminal(
        &TermEvent::Mouse(Mouse {
            kind: MouseKind::Press(Button::Left),
            col: 4,
            row: u16::try_from(call).unwrap(),
            mods: Mods::NONE,
        }),
        now,
    );
    app.terminal(
        &TermEvent::Mouse(Mouse {
            kind: MouseKind::Release(Some(Button::Left)),
            col: 9,
            row: u16::try_from(call).unwrap(),
            mods: Mods::NONE,
        }),
        now,
    );
    assert_eq!(rows_text(&mut app, now), before);
}

/// An answer resized while it streams is laid out again at the new width
/// in the full screen: frame by frame after the resize, its rows are those
/// of a full screen that was that wide all along.
#[test]
fn an_answer_resized_while_it_streams_is_laid_out_again() {
    let now = Instant::now();
    let (mut resized, resized_link) = opened_app(Presenter::Fullscreen, 99, 200, now);
    let (mut narrow, narrow_link) = opened_app(Presenter::Fullscreen, 49, 200, now);
    let steps = turn(1, "Fix scan_number.", ANSWER, OUTPUT);
    let half = steps.len() / 2;
    for (index, step) in steps.into_iter().enumerate() {
        for inbound in step {
            resized_link.give(inbound.clone());
            narrow_link.give(inbound);
        }
        drain(&mut resized, now);
        drain(&mut narrow, now);
        if index == half {
            resized.resize(49, 200);
        }
        let (wide_rows, _) = resized.screen_frame(now);
        let (narrow_rows, _) = narrow.screen_frame(now);
        if index >= half {
            assert_eq!(
                text(&wide_rows),
                text(&narrow_rows),
                "after step {index}, the resize at {half}"
            );
        }
    }
}

/// A narrower window lays every row out again: none is wider than it, and
/// the answer's words reflow, its code keeping its lines.
#[test]
fn a_resize_lays_the_transcript_out_again() {
    let now = Instant::now();
    let (mut app, _link) = long_session(Presenter::Fullscreen, 2, now);
    app.resize(39, 60);
    let (rows, _) = app.screen_frame(now);
    assert_eq!(rows.len(), 60);
    assert!(
        rows.iter().all(|row| row.width() <= 39),
        "{:#?}",
        text(&rows)
    );
    let shown = text(&rows);
    assert!(
        shown.iter().any(|row| row.starts_with("  if c == '_' {")),
        "{shown:#?}"
    );
    // Back to wide: as it was.
    app.resize(79, 20);
    let (wide, _) = app.screen_frame(now);
    assert!(wide.iter().any(|row| row.width() > 39));
}

/// The inline screen is untouched by any of it: no click or wheel does
/// anything there, and the full screen's own keys are the editor's.
#[test]
fn the_inline_screen_takes_no_mouse_and_no_full_screen_keys() {
    let now = Instant::now();
    let (mut app, _link) = long_session(Presenter::Inline, 2, now);
    assert!(!app.full_screen());
    wheel(&mut app, true, now);
    click(&mut app, 4, 4, now);
    key(&mut app, "pageup", now);
    let (_, live, _) = app.frame(now);
    assert!(text(&live).iter().all(|row| !row.contains("more below")));
}

/// The viewer takes the wheel and a click: a click on a box's row selects
/// it and opens it whole, a second closes it.
#[test]
fn the_viewer_scrolls_with_the_wheel_and_a_click_opens_a_box() {
    let now = Instant::now();
    let (mut app, _link) = long_session(Presenter::Inline, 3, now);
    key(&mut app, "ctrl+o", now);
    assert!(app.viewing());
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(rows[29].ends_with("box 3 of 3"), "{rows:#?}");
    // The first box on the screen, by its call's row.
    let call = rows
        .iter()
        .position(|row| row.contains("Run cargo test"))
        .expect("a box on the screen");
    // The second box: not the one selected, the last.
    click(&mut app, 4, u16::try_from(call).unwrap(), now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows[29].ends_with("box 2 of 3"),
        "the clicked box is selected: {rows:#?}"
    );
    assert!(rows[29].contains("enter collapses"), "and open: {rows:#?}");
    let rows_before = rows;
    wheel(&mut app, true, now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert_ne!(rows, rows_before, "the wheel scrolled");
    // A click on the title row does nothing.
    click(&mut app, 4, 0, now);
    assert_eq!(text(&app.viewer_frame(30, 99, now)), rows);
}

/// How long a test waits for the search's worker; never reached when all
/// is well.
const BOUND: std::time::Duration = std::time::Duration::from_secs(8);

/// `Ctrl+F` searches the kept transcript off the key's path: the newest
/// match is brought into view and drawn reversed and bold, `Up` goes to the
/// older one and `Down` back, going round, the row in the hint row's place
/// says where the search is, and `Esc` ends it where it is.
#[test]
fn ctrl_f_searches_the_transcript_and_goes_between_matches() {
    let now = Instant::now();
    let (mut app, _link) = long_session(Presenter::Fullscreen, 3, now);
    key(&mut app, "ctrl+f", now);
    super::type_text(&mut app, "TURN", now);
    let rows = rows_text(&mut app, now);
    assert!(rows[19].starts_with("  ctrl+f TURN"), "{rows:#?}");
    assert!(app.settle_search(BOUND), "the worker answered");
    let (lines, cursor) = app.screen_frame(now);
    let rows = text(&lines);
    assert!(rows[19].contains("3 of 3"), "{rows:#?}");
    // The cursor is after what is typed.
    assert_eq!(
        cursor.map(|cursor| (cursor.row, cursor.col)),
        Some((19, 13))
    );
    // The newest match is shown, reversed and bold, the rest of its row as
    // it was.
    let row = rows
        .iter()
        .position(|row| row.contains("Turn 3."))
        .expect("turn 3");
    let spans = lines[row].spans();
    let marked = spans
        .iter()
        .find(|span| span.text() == "Turn")
        .expect("the match is a span of its own");
    let rest = spans
        .iter()
        .find(|span| span.text() == " 3.")
        .expect("the rest of the prompt");
    assert_eq!(marked.style(), rest.style().reverse().bold());
    key(&mut app, "up", now);
    let rows = rows_text(&mut app, now);
    assert!(rows[19].contains("2 of 3"), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("Turn 2.")), "{rows:#?}");
    key(&mut app, "down", now);
    assert!(rows_text(&mut app, now)[19].contains("3 of 3"));
    // Going round: newer than the newest is the oldest.
    key(&mut app, "down", now);
    let rows = rows_text(&mut app, now);
    assert!(rows[19].contains("1 of 3"), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("Turn 1.")), "{rows:#?}");
    key(&mut app, "esc", now);
    let rows = rows_text(&mut app, now);
    assert!(!rows[19].contains("ctrl+f"), "{rows:#?}");
    assert!(
        rows.iter().any(|row| row.contains("Turn 1.")),
        "the view stays: {rows:#?}"
    );
    // In text that is not bold, the current match is told apart from the
    // others: bold as well as reversed. It is shown, though its row is the
    // window's last, which the row saying there is more below covers.
    key(&mut app, "esc", now);
    key(&mut app, "ctrl+f", now);
    super::type_text(&mut app, "digit", now);
    assert!(app.settle_search(BOUND));
    let (lines, _) = app.screen_frame(now);
    // The transcript's rows: the search row holds the query too.
    let styles: Vec<_> = lines[..19]
        .iter()
        .flat_map(|line| line.spans().iter())
        .filter(|span| span.text() == "digit")
        .map(|span| span.style())
        .collect();
    assert!(styles.len() >= 3, "{:#?}", text(&lines));
    let apart = styles
        .iter()
        .filter(|style| styles.iter().filter(|other| other == style).count() == 1)
        .count();
    assert_eq!(apart, 1, "one match is drawn apart: {styles:#?}");
    key(&mut app, "esc", now);
    // Nothing found says so.
    key(&mut app, "ctrl+f", now);
    super::type_text(&mut app, "nowhere at all", now);
    assert!(app.settle_search(BOUND));
    assert!(rows_text(&mut app, now)[19].contains("no match"));
}

/// A match inside a closed box opens the box while it is the current
/// match, its border saying the search opened it; a new query closes it,
/// and ending the search on it leaves it open.
#[test]
fn a_match_inside_a_closed_box_opens_it_while_it_is_current() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Fullscreen, 79, 24, now);
    // A command that succeeded: its box starts closed.
    let mut steps = turn(1, "Build it.", &["Built.\n"], "compiled findme\nall fine");
    for step in &mut steps {
        for inbound in step.iter_mut() {
            if let Inbound::Event(event) = inbound
                && let Event::ItemCompleted(ItemCompleted {
                    item: Item::ToolResult(result),
                }) = &mut event.event
            {
                result.outcome = cortex_protocol::item::ToolOutcome::Success;
                result.payload = Some(serde_json::json!({"exit_code": 0}));
            }
        }
    }
    for step in steps {
        for inbound in step {
            link.give(inbound);
        }
        drain(&mut app, now);
        let _ = app.screen_frame(now);
    }
    let rows = rows_text(&mut app, now);
    assert!(
        !rows.iter().any(|row| row.contains("findme")),
        "closed: {rows:#?}"
    );
    key(&mut app, "ctrl+f", now);
    super::type_text(&mut app, "findme", now);
    assert!(app.settle_search(BOUND));
    let rows = rows_text(&mut app, now);
    assert!(
        rows.iter().any(|row| row.contains("compiled findme")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("opened by the search")),
        "{rows:#?}"
    );
    // A query that no longer matches closes it again.
    super::type_text(&mut app, "x", now);
    assert!(app.settle_search(BOUND));
    let rows = rows_text(&mut app, now);
    assert!(rows[23].contains("no match"), "{rows:#?}");
    assert!(
        !rows.iter().any(|row| row.contains("compiled findme")),
        "{rows:#?}"
    );
    key(&mut app, "backspace", now);
    assert!(app.settle_search(BOUND));
    key(&mut app, "esc", now);
    let rows = rows_text(&mut app, now);
    assert!(
        rows.iter().any(|row| row.contains("compiled findme")),
        "left open: {rows:#?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("opened by the search")),
        "{rows:#?}"
    );
}

/// From 140 columns the panels sit in a column of their own beside the
/// transcript, from the top: the plan there, collapsed, then whole with
/// the focus; the transcript keeps the rest, and no row is wider than the
/// window. Narrower, they are stacked above the input box as inline.
#[test]
fn panels_sit_beside_the_transcript_on_a_wide_window() {
    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 2, now);
    link.give(super::panels::plan_updated(super::panels::screen_plan(1)));
    drain(&mut app, now);
    app.resize(149, 30);
    let (lines, _) = app.screen_frame(now);
    let rows = text(&lines);
    assert!(lines.iter().all(|line| line.width() <= 149), "{rows:#?}");
    // The plan's first row, at the column's start: 45 columns at 150 (40,
    // and one for each two past 140), two from the transcript's 102.
    let plan = rows
        .iter()
        .position(|row| row.contains("Plan  1 of 4 done"))
        .expect("the plan");
    assert_eq!(plan, 0, "from the top: {rows:#?}");
    assert_eq!(
        rows[plan].chars().position(|c| c == '◇'),
        Some(104),
        "{rows:#?}"
    );
    // The transcript beside it, at its own width.
    assert!(
        rows.iter()
            .any(|row| row.starts_with('╭') && row.chars().nth(101) == Some('╮')),
        "{rows:#?}"
    );
    // Focused: drawn whole, every item.
    key(&mut app, "ctrl+p", now);
    let rows = rows_text(&mut app, now);
    for item in [
        "allow _ between digits",
        "add a test for 1_000",
        "run clippy",
    ] {
        assert!(
            rows.iter().any(|row| row.contains(item)),
            "{item}: {rows:#?}"
        );
    }
    key(&mut app, "esc", now);
    // At 139 columns: stacked above the input box, under the transcript.
    app.resize(138, 30);
    let rows = rows_text(&mut app, now);
    let plan = rows
        .iter()
        .position(|row| row.contains("Plan  1 of 4 done"))
        .expect("the plan");
    assert!(rows[plan].starts_with("◇"), "{rows:#?}");
    assert!(plan > 10, "above the bottom: {rows:#?}");
    assert_eq!(crate::app::state::screen::side_column(139), None);
    assert_eq!(crate::app::state::screen::side_column(140), Some(40));
    assert_eq!(crate::app::state::screen::side_column(200), Some(56));
}

/// Code whose colours were late is drawn plain as it commits, kept as late,
/// handed to the highlighter again, and drawn in its colours by the full
/// screen once they come (closes `highlighted-code-drawn-again`).
#[cfg(feature = "highlight")]
#[test]
fn code_committed_plain_is_coloured_once_its_colours_come() {
    use crate::highlight::Highlighter;
    use crate::markdown::Markdown;
    use crate::theme::Token;

    let now = Instant::now();
    let highlighter = Highlighter::new(|| {});
    // The grammars loaded first, then the worker held: the code's colours
    // are late.
    let look = crate::app::look(&Settings::default(), &crate::testkit::caps());
    let mut warm = Markdown::answer().with_highlighter(highlighter.clone());
    warm.push("```rust\nx\n```\n");
    let _ = warm.commit(79, &look);
    assert!(highlighter.settle(BOUND));
    highlighter.hold(true);
    let settings = Settings {
        presenter: Presenter::Fullscreen,
        ..Settings::default()
    };
    let (mut app, link) = crate::testkit::app_with_highlighter(now, settings, highlighter.clone());
    app.resize(79, 60);
    opened(&mut app, &link, now);
    let later = now + std::time::Duration::from_secs(5);
    for step in turn(1, "Fix it.", ANSWER, OUTPUT) {
        for inbound in step {
            link.give(inbound);
        }
        drain(&mut app, now);
        let _ = app.screen_frame(now);
    }
    // Past the wait: committed plain.
    let _ = app.screen_frame(later);
    let keyword = look.theme.style(Token::SyntaxKeyword);
    let coloured = |lines: &[Line]| {
        lines
            .iter()
            .flat_map(|line| line.spans().to_vec())
            .any(|span| span.text() == "continue" && span.style() == keyword)
    };
    let (lines, _) = app.screen_frame(later);
    assert!(
        lines.iter().any(|line| line.text().contains("continue")),
        "{:#?}",
        text(&lines)
    );
    assert!(!coloured(&lines), "plain while late");
    highlighter.hold(false);
    assert!(highlighter.settle(BOUND));
    let (lines, _) = app.screen_frame(later);
    assert!(
        coloured(&lines),
        "coloured once they came: {:#?}",
        text(&lines)
    );
}

/// A frame of the full-screen presenter after 200 turns (step 25,
/// `docs/performance-budgets.md`): the application's frame and `Full`'s
/// bytes for it, at the end and following, scrolled into the middle, and
/// while an answer streams (a piece taken in before each frame), in a
/// window of 120 by 40. Ignored; run in a release build:
///
/// ```text
/// cargo test --release -p cortex-tui --features test-kit --lib perf_a_frame_in_the_full_screen -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, in a release build"]
fn perf_a_frame_in_the_full_screen_presenter() {
    use cortex_term::{Full, Screen};

    let runs: usize = std::env::var("CORTEX_PERF_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000);
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Fullscreen, 119, 40, now);
    for n in 1..=200 {
        for step in turn(n, &format!("Turn {n}."), ANSWER, OUTPUT) {
            for inbound in step {
                link.give(inbound);
            }
            drain(&mut app, now);
        }
        let _ = app.screen_frame(now);
    }
    let caps = crate::testkit::caps();
    let mut full = Full::new(120, 40);
    let mut measure = |label: &str, app: &mut super::App, each: &mut dyn FnMut(&mut super::App)| {
        let mut times = Vec::with_capacity(runs);
        let mut bytes = Vec::with_capacity(runs);
        for _ in 0..runs {
            each(app);
            let began = Instant::now();
            let (lines, cursor) = app.screen_frame(now);
            let mut sink = Vec::new();
            full.draw(
                &Screen {
                    lines: &lines,
                    cursor,
                },
                &caps,
                &mut sink,
            )
            .expect("drawn");
            times.push(began.elapsed());
            bytes.push(sink.len());
        }
        times.sort();
        bytes.sort_unstable();
        let at = |share: f64| {
            let index = ((times.len() as f64 * share) as usize).min(times.len() - 1);
            times[index].as_secs_f64() * 1_000.0
        };
        let median = at(0.5);
        let p99 = at(0.99);
        println!(
            "{label}: {median:.3} ms median, {p99:.3} ms at the 99th, {} bytes median, {} at most",
            bytes[bytes.len() / 2],
            bytes[bytes.len() - 1]
        );
        cortex_testkit::numbers::report(&format!("full_screen.{label}.median"), median, "ms");
        cortex_testkit::numbers::report(&format!("full_screen.{label}.p99"), p99, "ms");
    };
    measure("idle_at_the_end", &mut app, &mut |_| {});
    for _ in 0..40 {
        key(&mut app, "pageup", now);
    }
    measure("idle_in_the_middle", &mut app, &mut |_| {});
    key(&mut app, "pagedown", now);
    for _ in 0..80 {
        key(&mut app, "pagedown", now);
    }
    // An answer streaming: a piece each frame, the window following it.
    let answer = cortex_protocol::ItemId::from("streaming");
    link.give(event(Event::ItemStarted(ItemStarted {
        item_id: answer.clone(),
        kind: ItemKind::AssistantMessage,
        tool: None,
        call_id: None,
    })));
    drain(&mut app, now);
    let mut n = 0;
    measure("streaming", &mut app, &mut |app| {
        n += 1;
        link.give(event(Event::ItemDelta(ItemDelta {
            item_id: answer.clone(),
            delta: Delta::Text(TextDelta {
                text: format!("word{n} and some more words to wrap, "),
            }),
        })));
        drain(app, now);
    });
}

/// What a played terminal reads, in turn.
type Steps = Vec<(cortex_term::Ready, Vec<u8>)>;

/// What the loop wrote, played into a virtual terminal: the modes it left.
fn modes_after(written: &[u8]) -> cortex_testkit::vterm::Modes {
    let mut term = cortex_testkit::VTerm::new(100, 30);
    term.feed(written);
    term.modes()
}

/// A played terminal of 100 by 30 with `steps`, and what is written to it.
fn played(
    steps: Steps,
    base: Instant,
) -> (super::Played, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
    let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let device = super::Played {
        steps: steps.into_iter().collect(),
        written: std::sync::Arc::clone(&written),
        waits: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        base,
        elapsed: std::time::Duration::ZERO,
    };
    (device, written)
}

fn bytes(written: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> Vec<u8> {
    written
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn holds(bytes: &[u8], part: &[u8]) -> bool {
    bytes.windows(part.len()).any(|window| window == part)
}

/// The full-screen presenter's loop draws on the alternate screen with the
/// mouse reported, and leaves both off when it quits, the core goes, or
/// the terminal fails under it; the mouse is never on with the main
/// screen.
#[test]
fn every_way_out_of_the_full_screen_leaves_the_mouse_and_the_alternate_screen_off() {
    use cortex_term::Ready;
    let ways: [(&str, Steps); 3] = [
        (
            "quit",
            vec![(Ready::Woken, Vec::new()), (Ready::Input, b"\x04".to_vec())],
        ),
        ("the core gone", vec![(Ready::Woken, Vec::new())]),
        // The script ends: the terminal fails under the loop.
        ("the terminal failed", Vec::new()),
    ];
    for (way, steps) in ways {
        let base = Instant::now();
        let settings = Settings {
            presenter: Presenter::Fullscreen,
            ..Settings::default()
        };
        let (mut app, link) = app_with_settings(base, settings);
        link.give(Inbound::Done(
            crate::app::Ticket(1),
            Ok(cortex_protocol::methods::OpResult::SessionNew(
                cortex_protocol::methods::SessionResult {
                    session: crate::testkit::session().build(),
                },
            )),
        ));
        if way == "the core gone" {
            link.give(Inbound::Gone(cortex_protocol::ErrorData::new(
                cortex_protocol::ErrorKind::Internal,
                "gone",
                "the core went away",
            )));
        }
        let (device, written) = played(steps, base);
        let mut terminal =
            cortex_term::Terminal::with_device(device, crate::testkit::caps()).expect("a terminal");
        let ended = crate::app::run(&mut terminal, &mut app);
        if way == "the terminal failed" {
            assert!(ended.is_err(), "{way}");
        } else {
            ended.expect("the loop ends");
        }
        // What the loop drew: on the alternate screen, the mouse with it.
        let during = bytes(&written);
        assert!(
            holds(&during, b"\x1b[?1049h\x1b[?1000h\x1b[?1006h"),
            "{way}"
        );
        // Put back as the program puts it back.
        terminal.leave().expect("left");
        let all = bytes(&written);
        let modes = modes_after(&all);
        assert!(!modes.alternate, "{way}: the alternate screen is left");
        assert!(!modes.mouse && !modes.mouse_sgr, "{way}: the mouse is off");
        // The mouse was never on while the main screen was shown: each
        // turning on follows the alternate screen's.
        let on = b"\x1b[?1000h";
        let mut from = 0;
        while let Some(at) = all[from..].windows(on.len()).position(|w| w == on) {
            let at = from + at;
            assert!(all[..at].ends_with(b"\x1b[?1049h"), "{way}: at {at}");
            from = at + on.len();
        }
    }
}

/// The viewer opened from the inline screen reports the mouse while it is
/// shown, and turns it off as it returns to the main screen.
#[test]
fn the_viewer_turns_the_mouse_off_as_it_returns_to_the_main_screen() {
    use cortex_term::Ready;
    let base = Instant::now();
    let (mut app, link) = crate::testkit::app(base);
    link.give(Inbound::Done(
        crate::app::Ticket(1),
        Ok(cortex_protocol::methods::OpResult::SessionNew(
            cortex_protocol::methods::SessionResult {
                session: crate::testkit::session().build(),
            },
        )),
    ));
    let (device, written) = played(
        vec![
            (Ready::Woken, Vec::new()),
            // Ctrl+O, then Esc on its own.
            (Ready::Input, b"\x0f".to_vec()),
            (Ready::Timeout, Vec::new()),
            (Ready::Input, b"\x1b".to_vec()),
            (Ready::Timeout, Vec::new()),
            (Ready::Timeout, Vec::new()),
            (Ready::Input, b"\x04".to_vec()),
        ],
        base,
    );
    let mut terminal =
        cortex_term::Terminal::with_device(device, crate::testkit::caps()).expect("a terminal");
    crate::app::run(&mut terminal, &mut app).expect("the loop ends");
    let all = bytes(&written);
    assert!(
        holds(&all, b"\x1b[?1049h\x1b[?1000h\x1b[?1006h"),
        "the viewer had the mouse"
    );
    assert!(
        holds(&all, b"\x1b[?1006l\x1b[?1000l\x1b[?1049l"),
        "and gave it back"
    );
    let modes = modes_after(&all);
    assert!(!modes.alternate && !modes.mouse && !modes.mouse_sgr);
}
