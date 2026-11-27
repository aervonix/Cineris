//! The full-screen presenter's view (step 25; `docs/steps/25/answered.md`,
//! point 2): the same rows as the inline screen, a window that follows the
//! end until it is moved up, a click and the wheel, and a resize that lays
//! everything out again.

use std::time::Instant;

use cineris_protocol::event::{
    Delta, Event, ItemCompleted, ItemDelta, ItemKind, ItemStarted, TextDelta, TurnCompleted,
    TurnOutcome, TurnStarted,
};
use cineris_protocol::item::{AssistantMessageItem, StopReason, UserMessageItem};
use cineris_protocol::message::Part;
use cineris_protocol::{CallId, Item, ItemId, TurnId};
use cineris_term::{Button, Event as TermEvent, Line, Mods, Mouse, MouseKind, Wheel};

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
                command: None,
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
                    Part::ToolCall(cineris_protocol::message::ToolCallPart {
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
            item: Item::ToolResult(cineris_protocol::item::ToolResultItem {
                id: ItemId::from(format!("r{turn}").as_str()),
                call_id: call,
                tool: "shell".to_owned(),
                outcome: cineris_protocol::item::ToolOutcome::Error,
                content: vec![Part::text(output)],
                payload: Some(serde_json::json!({"exit_code": 101, "output": output})),
                artifacts: Vec::new(),
                duration_ms: Some(3400),
                created_at: None,
                replaced: None,
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

/// The search's answer that comes while nothing else happens is drawn:
/// the loop's drain takes it and says there is a change, so the row does not
/// stay at `searching…` until the next key (the review of step 25).
#[test]
fn a_search_answer_that_comes_while_idle_schedules_a_frame() {
    let now = Instant::now();
    let (mut app, _link) = long_session(Presenter::Fullscreen, 3, now);
    key(&mut app, "ctrl+f", now);
    super::type_text(&mut app, "TURN", now);
    assert!(app.search_ready(BOUND), "the worker answered");
    assert!(app.drain(now, 64).changed, "the answer is a change to draw");
    let rows = rows_text(&mut app, now);
    assert!(rows[19].contains("3 of 3"), "{rows:#?}");
}

/// The viewer opened while an answer streams shows what the answer has
/// committed to the scrollback, then its live rows: nothing in between is
/// missing (the review of step 25).
#[test]
fn the_viewer_opened_mid_stream_shows_what_the_answer_committed() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 79, 20, now);
    let steps = turn(1, "Turn 1.", ANSWER, OUTPUT);
    // The answer's first four pieces: its first paragraph and its code are
    // committed, its last paragraph is live.
    let mut committed = Vec::new();
    for step in &steps[..5] {
        for inbound in step {
            link.give(inbound.clone());
        }
        drain(&mut app, now);
        let (commit, _, _) = app.frame(now);
        committed.extend(text(&commit));
        app.drawn(now);
    }
    assert!(
        committed
            .iter()
            .any(|row| row.contains("scan_number stops at the first")),
        "{committed:#?}"
    );
    key(&mut app, "ctrl+o", now);
    assert!(app.viewing());
    let rows = text(&app.viewer_frame(40, 79, now));
    for wanted in [
        "scan_number stops at the first",
        "self.bump();",
        "Then the test",
    ] {
        assert!(
            rows.iter().any(|row| row.contains(wanted)),
            "{wanted:?} missing: {rows:#?}"
        );
    }
}

/// `End`, then `PageDown`, `j` and `Down` in the viewer: the window stays
/// at the end, its rows those `End` showed (step 23's overflow, held since
/// step 25 by the window itself).
#[test]
fn end_then_page_down_in_the_viewer_stays_at_the_end() {
    let now = Instant::now();
    let (mut app, _link) = long_session(Presenter::Inline, 3, now);
    key(&mut app, "ctrl+o", now);
    key(&mut app, "home", now);
    key(&mut app, "end", now);
    let at_end = text(&app.viewer_frame(30, 99, now));
    assert!(at_end[29].ends_with("box 3 of 3"), "{at_end:#?}");
    for binding in ["pagedown", "end", "j", "down", "pagedown"] {
        key(&mut app, binding, now);
    }
    assert_eq!(text(&app.viewer_frame(30, 99, now)), at_end);
}

/// A dialog that comes while the search is open: what is pasted then goes
/// to the search, and its answer does not move the window, which stays at
/// the end the request followed, at what led to the dialog (the second
/// review of step 25).
#[test]
fn the_search_does_not_move_the_window_under_a_dialog() {
    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 3, now);
    key(&mut app, "ctrl+f", now);
    link.give(Inbound::Request(
        cineris_protocol::jsonrpc::RequestId::Number(7),
        crate::testkit::approval("c_7"),
    ));
    drain(&mut app, now);
    let _ = rows_text(&mut app, now);
    assert!(app.screen_follows(), "the request follows the end");
    app.terminal(&TermEvent::Paste("Turn".to_owned()), now);
    assert!(app.search_ready(BOUND));
    let _ = app.drain(now, 64);
    let _ = rows_text(&mut app, now);
    assert_eq!(app.search_query().as_deref(), Some("Turn"));
    assert!(
        app.screen_follows(),
        "the search moved the window under the dialog"
    );
}

/// A search's query holds at most 256 characters, however much is typed
/// or pasted; with the ASCII glyphs, the row that says there is more below
/// uses theirs (the review of step 25).
#[test]
fn the_query_is_bounded_and_the_more_below_row_follows_the_glyphs() {
    let now = Instant::now();
    let (mut app, _link) = long_session(Presenter::Fullscreen, 3, now);
    key(&mut app, "ctrl+f", now);
    app.terminal(&TermEvent::Paste("x".repeat(1_000)), now);
    super::type_text(&mut app, "yz", now);
    assert_eq!(
        app.search_query().map(|query| query.chars().count()),
        Some(256)
    );
    key(&mut app, "esc", now);
    app.use_ascii_glyphs();
    key(&mut app, "pageup", now);
    let rows = rows_text(&mut app, now);
    assert!(
        rows.iter().any(|row| row.contains("more below | ")),
        "{rows:#?}"
    );
}

/// A turn that reasons, in three lines, then answers; then a compaction.
fn reasoned_and_compacted(app: &mut super::App, link: &Scripted, now: Instant) {
    reasoned_and_compacted_with(
        app,
        link,
        "First, the scanner.\nThen the \u{202e}parser.\nLast, the tests.",
        now,
    );
}

/// A turn that reasons `reasoning`, then answers; then a compaction. Gives
/// the rows the frames committed.
fn reasoned_and_compacted_with(
    app: &mut super::App,
    link: &Scripted,
    reasoning: &str,
    now: Instant,
) -> Vec<String> {
    let answer = ItemId::from("a1");
    let steps = [
        event(Event::TurnStarted(TurnStarted {})),
        event(Event::ItemStarted(ItemStarted {
            item_id: answer.clone(),
            kind: ItemKind::AssistantMessage,
            tool: None,
            call_id: None,
        })),
        event(Event::ItemDelta(ItemDelta {
            item_id: answer.clone(),
            delta: Delta::Reasoning(TextDelta {
                text: reasoning.to_owned(),
            }),
        })),
        event(Event::ItemDelta(ItemDelta {
            item_id: answer.clone(),
            delta: Delta::Text(TextDelta {
                text: "Done.\n".to_owned(),
            }),
        })),
        event(Event::ItemCompleted(ItemCompleted {
            item: Item::AssistantMessage(AssistantMessageItem {
                id: answer,
                parts: vec![Part::text("Done.\n")],
                stop_reason: Some(StopReason::End),
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
            item: Item::Summary(cineris_protocol::item::SummaryItem {
                id: ItemId::from("sum1"),
                text: "(the summary, as the event has it)".to_owned(),
                covers_through: None,
                usage: None,
                created_at: None,
            }),
        })),
        event(Event::TurnCompleted(TurnCompleted {
            outcome: TurnOutcome::Completed,
            stop_reason: Some(StopReason::End),
            error: None,
            usage: Default::default(),
            verification: None,
            duration_ms: Some(900),
        })),
    ];
    let mut committed = Vec::new();
    for inbound in steps {
        link.give(inbound);
        drain(app, now);
        let (commit, _, _) = app.frame(now);
        committed.extend(text(&commit));
    }
    committed
}

/// A reasoning of `lines` lines, each naming its number, the last
/// `The last line of a long reasoning.`
fn long_reasoning(lines: usize) -> String {
    let mut text = String::new();
    for n in 1..lines {
        text.push_str(&format!(
            "Line {n}: the scanner reads each digit, and an underscore between two.\n"
        ));
    }
    text.push_str("The last line of a long reasoning.");
    text
}

/// A reasoning longer than a kept block holds (since the fixes of the
/// review of step 25, which found every reasoning kept twice and without a
/// bound): the inline presenter commits it whole, as before; the viewer
/// draws it to its bound, with a row that says it was cut; and `Enter` reads
/// it whole from its item, of the session it is of, by `session/read`, and
/// shows it.
#[test]
fn a_reasoning_kept_only_in_part_is_read_whole_from_its_item() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 99, 30, now);
    let reasoning = long_reasoning(400);
    assert!(reasoning.len() > 3 * crate::transcript::kept::REASONING_KEPT);
    let committed = reasoned_and_compacted_with(&mut app, &link, &reasoning, now);
    // Inline, what is drawn is not changed: all of it.
    for line in [
        "Line 1: the scanner",
        "Line 399: the scanner",
        "The last line of a long reasoning.",
    ] {
        assert!(
            committed.iter().any(|row| row.contains(line)),
            "{line:?} not committed: {committed:#?}"
        );
    }
    assert!(!committed.iter().any(|row| row.contains("cut here")));
    link.take_sent();
    key(&mut app, "ctrl+o", now);
    key(&mut app, "up", now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(rows[29].trim_end().ends_with("reasoning"), "{rows:#?}");
    key(&mut app, "home", now);
    // The transcript's view from its top, a page at a time: the kept start,
    // then the row that says it was cut, and none of what was not kept.
    let mut seen = Vec::new();
    for _ in 0..12 {
        seen.extend(text(&app.viewer_frame(30, 99, now))[2..27].to_vec());
        key(&mut app, "pagedown", now);
    }
    assert!(
        seen.iter().any(|row| row.contains("Line 1: the scanner")),
        "{seen:#?}"
    );
    assert!(
        seen.iter()
            .any(|row| row.contains("cut here: enter on it in the viewer shows it whole")),
        "{seen:#?}"
    );
    assert!(
        !seen
            .iter()
            .any(|row| row.contains("The last line of a long reasoning.")),
        "{seen:#?}"
    );
    key(&mut app, "enter", now);
    let read = link
        .take_sent()
        .into_iter()
        .find_map(|(ticket, op)| match op {
            cineris_protocol::methods::Op::SessionRead(params) => Some((ticket, params)),
            _ => None,
        })
        .expect("the reasoning read");
    assert_eq!(read.1.item_id, Some(ItemId::from("a1")));
    assert_eq!(read.1.session_id.as_str(), crate::testkit::SESSION);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(rows[29].contains("reading the reasoning"), "{rows:#?}");
    link.give(Inbound::Done(
        read.0,
        Ok(cineris_protocol::methods::OpResult::SessionRead(
            cineris_protocol::methods::SessionReadResult {
                items: vec![Item::AssistantMessage(AssistantMessageItem {
                    id: ItemId::from("a1"),
                    parts: vec![
                        Part::Reasoning(cineris_protocol::message::ReasoningPart {
                            text: reasoning.clone(),
                            raw: None,
                        }),
                        Part::text("Done.\n"),
                    ],
                    stop_reason: Some(StopReason::End),
                    raw_stop: None,
                    stop_detail: None,
                    truncated: false,
                    origin: None,
                    usage: None,
                    error: None,
                    created_at: None,
                })],
                incomplete: Vec::new(),
                next_cursor: None,
                seq: 9,
                masked: 0,
                redacted: false,
            },
        )),
    ));
    drain(&mut app, now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows.iter().any(|row| row.contains("The reasoning, whole")),
        "{rows:#?}"
    );
    key(&mut app, "end", now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows.iter()
            .any(|row| row.contains("The last line of a long reasoning.")),
        "{rows:#?}"
    );
}

/// A compaction's summary, and a reasoning kept only in part, are read
/// from the session their block is of, not from whichever is open (the
/// review of step 25, finding 18: item ids repeat across sessions, so after
/// `/new` the read named the new session's item of the same id and showed
/// it). A session left is not open on this connection, where `session/read`
/// refuses: nothing is asked, and the keys row says it could not be read.
#[test]
fn a_summary_is_read_from_the_session_its_block_is_of() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 99, 30, now);
    reasoned_and_compacted_with(&mut app, &link, &long_reasoning(400), now);
    link.take_sent();
    super::type_text(&mut app, "/new", now);
    key(&mut app, "esc", now);
    key(&mut app, "enter", now);
    let (ticket, _) = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, cineris_protocol::methods::Op::SessionNew(_)))
        .expect("/new opens a session");
    link.give(Inbound::Done(
        ticket,
        Ok(cineris_protocol::methods::OpResult::SessionNew(
            cineris_protocol::methods::SessionResult {
                session: crate::testkit::session().id("s_other").build(),
            },
        )),
    ));
    drain(&mut app, now);
    let _ = app.frame(now);
    link.take_sent();
    key(&mut app, "ctrl+o", now);
    // The compaction, then the reasoning: neither is read from `s_other`.
    for (stop, label) in [
        ("summary", "the summary could not be read"),
        ("reasoning", "the reasoning could not be read"),
    ] {
        let rows = text(&app.viewer_frame(30, 99, now));
        assert!(rows[29].trim_end().ends_with(stop), "{rows:#?}");
        key(&mut app, "enter", now);
        let sent = link.take_sent();
        assert!(
            !sent
                .iter()
                .any(|(_, op)| matches!(op, cineris_protocol::methods::Op::SessionRead(_))),
            "{stop}: {sent:#?}"
        );
        let rows = text(&app.viewer_frame(30, 99, now));
        assert!(rows[29].contains(label), "{rows:#?}");
        key(&mut app, "up", now);
    }
}

/// The viewer moves between a reasoning and a compaction as between boxes
/// (step 25): `Enter` on the reasoning shows its whole text, hostile text
/// made safe, and `Esc` goes back to the transcript with it still
/// selected; `Enter` on the compaction reads its summary item through the
/// link and shows it.
#[test]
fn the_viewer_shows_a_reasoning_and_a_compactions_summary_whole() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 99, 30, now);
    reasoned_and_compacted(&mut app, &link, now);
    link.take_sent();
    key(&mut app, "ctrl+o", now);
    assert!(app.viewing());
    // The last stop, the compaction, is selected.
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(rows[29].trim_end().ends_with("summary"), "{rows:#?}");
    key(&mut app, "up", now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(rows[29].trim_end().ends_with("reasoning"), "{rows:#?}");
    assert!(rows[29].contains("enter shows it whole"), "{rows:#?}");
    key(&mut app, "enter", now);
    let rows = text(&app.viewer_frame(30, 99, now));
    for line in [
        "First, the scanner.",
        "Last, the tests.",
        "\\u{202e}parser.",
    ] {
        assert!(
            rows.iter().any(|row| row.contains(line)),
            "{line:?}: {rows:#?}"
        );
    }
    assert!(
        crate::testing::unseen(&rows.concat()).is_empty(),
        "{rows:#?}"
    );
    key(&mut app, "esc", now);
    assert!(app.viewing(), "back to the transcript, not out");
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(rows[29].trim_end().ends_with("reasoning"), "{rows:#?}");
    // The compaction: its summary read by its id.
    key(&mut app, "down", now);
    key(&mut app, "enter", now);
    let read = link
        .take_sent()
        .into_iter()
        .find_map(|(ticket, op)| match op {
            cineris_protocol::methods::Op::SessionRead(params) => Some((ticket, params)),
            _ => None,
        })
        .expect("the summary read");
    assert_eq!(read.1.item_id, Some(ItemId::from("sum1")));
    link.give(Inbound::Done(
        read.0,
        Ok(cineris_protocol::methods::OpResult::SessionRead(
            cineris_protocol::methods::SessionReadResult {
                items: vec![Item::Summary(cineris_protocol::item::SummaryItem {
                    id: ItemId::from("sum1"),
                    text: "We fixed the lexer; the parser is next.".to_owned(),
                    covers_through: None,
                    usage: None,
                    created_at: None,
                })],
                incomplete: Vec::new(),
                next_cursor: None,
                seq: 9,
                masked: 0,
                redacted: false,
            },
        )),
    ));
    drain(&mut app, now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows.iter()
            .any(|row| row.contains("We fixed the lexer; the parser is next.")),
        "{rows:#?}"
    );
    key(&mut app, "esc", now);
    key(&mut app, "esc", now);
    assert!(!app.viewing());
}

/// The viewer brings the selected stop into view by its place among the
/// stops, not among the rounds (found reviewing step 25's rewind part):
/// opened on a long transcript that starts with a reasoning and a
/// compaction, it shows the last round; moved up to the reasoning, it shows
/// the reasoning.
#[test]
fn the_viewer_reveals_the_selected_stop_after_a_reasoning_and_a_compaction() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 99, 20, now);
    reasoned_and_compacted(&mut app, &link, now);
    for n in 2..=4 {
        for step in turn(n, &format!("Turn {n}."), ANSWER, OUTPUT) {
            for inbound in step {
                link.give(inbound);
            }
            drain(&mut app, now);
            let _ = app.frame(now);
        }
    }
    // The reasoning's own row, not the keys row's word for it.
    let shows_reasoning = |rows: &[String]| {
        rows[..rows.len() - 1]
            .iter()
            .any(|row| row.contains("∴ Reasoned for"))
    };
    key(&mut app, "ctrl+o", now);
    let rows = text(&app.viewer_frame(20, 99, now));
    assert!(
        !shows_reasoning(&rows),
        "opened away from the last round: {rows:#?}"
    );
    for _ in 0..4 {
        key(&mut app, "up", now);
        let _ = app.viewer_frame(20, 99, now);
    }
    let rows = text(&app.viewer_frame(20, 99, now));
    assert!(rows[19].trim_end().ends_with("reasoning"), "{rows:#?}");
    assert!(
        shows_reasoning(&rows),
        "the reasoning selected out of view: {rows:#?}"
    );
}

/// The summary read the viewer sent: its ticket.
fn summary_read(link: &Scripted) -> crate::app::Ticket {
    link.take_sent()
        .into_iter()
        .find_map(|(ticket, op)| match op {
            cineris_protocol::methods::Op::SessionRead(params)
                if params.item_id == Some(ItemId::from("sum1")) =>
            {
                Some(ticket)
            }
            _ => None,
        })
        .expect("the summary read")
}

/// A page that holds the summary `text`, or none.
fn summary_page(text: Option<&str>) -> cineris_protocol::methods::OpResult {
    cineris_protocol::methods::OpResult::SessionRead(cineris_protocol::methods::SessionReadResult {
        items: text
            .map(|text| {
                Item::Summary(cineris_protocol::item::SummaryItem {
                    id: ItemId::from("sum1"),
                    text: text.to_owned(),
                    covers_through: None,
                    usage: None,
                    created_at: None,
                })
            })
            .into_iter()
            .collect(),
        incomplete: Vec::new(),
        next_cursor: None,
        seq: 9,
        masked: 0,
        redacted: false,
    })
}

/// A compaction's summary read lands only on the view that asked, at the
/// stop it asked at: the viewer closed and opened again on `/copy`'s text
/// keeps that text, and the mouse off over it, when the late answer comes.
/// While the read is under way the keys row says so, and a read that finds
/// no summary, or fails, says that (the review of step 25's second pull
/// request).
#[test]
fn a_late_summary_lands_only_where_it_was_asked_for() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 99, 30, now);
    reasoned_and_compacted(&mut app, &link, now);
    link.take_sent();
    key(&mut app, "ctrl+o", now);
    key(&mut app, "enter", now);
    let late = summary_read(&link);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(rows[29].contains("reading the summary…"), "{rows:#?}");
    key(&mut app, "esc", now);
    assert!(!app.viewing());
    let mut caps = crate::testkit::caps();
    caps.clipboard = false;
    app.set_caps(&caps);
    super::run_command(&mut app, "/copy", now);
    assert!(app.viewing() && !app.mouse(), "the answer shown to copy");
    link.give(Inbound::Done(
        late,
        Ok(summary_page(Some("The late summary."))),
    ));
    drain(&mut app, now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        !rows.iter().any(|row| row.contains("The late summary.")),
        "the late answer was put over the text to copy: {rows:#?}"
    );
    assert!(!app.mouse(), "the mouse came back over the text to copy");
    key(&mut app, "esc", now);

    // A read that finds no summary, and one that fails: said.
    key(&mut app, "ctrl+o", now);
    key(&mut app, "enter", now);
    let empty = summary_read(&link);
    link.give(Inbound::Done(empty, Ok(summary_page(None))));
    drain(&mut app, now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows[29].contains("the summary could not be read"),
        "{rows:#?}"
    );
    key(&mut app, "enter", now);
    let failed = summary_read(&link);
    link.give(Inbound::Done(
        failed,
        Err(cineris_protocol::ErrorData::new(
            cineris_protocol::ErrorKind::Internal,
            "store_failed",
            "the log could not be read",
        )),
    ));
    drain(&mut app, now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows[29].contains("the summary could not be read"),
        "{rows:#?}"
    );
    // Moved to the reasoning before the answer came: not shown there.
    key(&mut app, "enter", now);
    let moved = summary_read(&link);
    key(&mut app, "up", now);
    link.give(Inbound::Done(
        moved,
        Ok(summary_page(Some("The late summary."))),
    ));
    drain(&mut app, now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        !rows.iter().any(|row| row.contains("The late summary.")),
        "{rows:#?}"
    );
    assert!(rows[29].trim_end().ends_with("reasoning"), "{rows:#?}");
}

/// A compaction's summary asked for, then a reasoning shown whole before
/// the read answered: back from the reasoning, the compaction does not say
/// `reading the summary…` for good, whether the answer came before `Esc` or
/// after it, and `Enter` there asks again (the review of the fixes of step
/// 25's second pull request: the read's answer went to the view on top,
/// and the view behind it kept waiting).
#[test]
fn a_summary_read_left_behind_a_reasoning_is_not_waited_for() {
    let now = Instant::now();
    for answered_on_top in [true, false] {
        let (mut app, link) = opened_app(Presenter::Inline, 99, 30, now);
        reasoned_and_compacted(&mut app, &link, now);
        link.take_sent();
        key(&mut app, "ctrl+o", now);
        key(&mut app, "enter", now);
        let late = summary_read(&link);
        key(&mut app, "up", now);
        key(&mut app, "enter", now);
        let answer = || {
            link.give(Inbound::Done(
                late,
                Ok(summary_page(Some("The late summary."))),
            ));
        };
        if answered_on_top {
            answer();
            drain(&mut app, now);
        }
        key(&mut app, "esc", now);
        if !answered_on_top {
            answer();
            drain(&mut app, now);
        }
        key(&mut app, "down", now);
        let rows = text(&app.viewer_frame(30, 99, now));
        assert!(
            !rows[29].contains("reading the summary…"),
            "{answered_on_top}: {rows:#?}"
        );
        assert!(rows[29].contains("summary"), "{answered_on_top}: {rows:#?}");
        key(&mut app, "enter", now);
        let again = summary_read(&link);
        link.give(Inbound::Done(
            again,
            Ok(summary_page(Some("The summary asked again."))),
        ));
        drain(&mut app, now);
        let rows = text(&app.viewer_frame(30, 99, now));
        assert!(
            rows.iter()
                .any(|row| row.contains("The summary asked again.")),
            "{answered_on_top}: {rows:#?}"
        );
    }
}

/// The look changed while a summary is shown: the transcript's view behind
/// it draws its rows again too, so that `Esc` shows it in the new look (the
/// review of step 25's second pull request).
#[test]
fn a_look_changed_under_a_summary_reaches_the_transcript_behind_it() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 99, 30, now);
    reasoned_and_compacted(&mut app, &link, now);
    key(&mut app, "ctrl+o", now);
    key(&mut app, "up", now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows.iter().any(|row| row.contains("∴ Reasoned for")),
        "{rows:#?}"
    );
    key(&mut app, "enter", now);
    let _ = app.viewer_frame(30, 99, now);
    app.use_ascii_glyphs();
    key(&mut app, "esc", now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows.iter().any(|row| row.contains("~ Reasoned for")),
        "{rows:#?}"
    );
    assert!(!rows.iter().any(|row| row.contains('∴')), "{rows:#?}");
}

/// A reasoning and a compaction's summary shown whole hide nothing behind a
/// bare carriage return: it is shown as its symbol, where a command's
/// output takes the line back to its start (the review of step 25's second
/// pull request).
#[test]
fn a_summary_shown_whole_shows_a_carriage_return() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 99, 30, now);
    reasoned_and_compacted_with(&mut app, &link, "Plan: do X\rdo nothing.\r\nDone.", now);
    link.take_sent();
    key(&mut app, "ctrl+o", now);
    key(&mut app, "up", now);
    key(&mut app, "enter", now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows.iter()
            .any(|row| row.contains("Plan: do X␍do nothing.")),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row == "Done."), "{rows:#?}");
    key(&mut app, "esc", now);
    key(&mut app, "down", now);
    key(&mut app, "enter", now);
    let read = summary_read(&link);
    link.give(Inbound::Done(
        read,
        Ok(summary_page(Some("Go on with X\rfrom here."))),
    ));
    drain(&mut app, now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows.iter()
            .any(|row| row.contains("Go on with X␍from here.")),
        "{rows:#?}"
    );
}

/// A click on a reasoning's row selects it and shows it whole, as `Enter`
/// does; on a compaction's, it reads its summary (the review of step 25's
/// second pull request).
#[test]
fn a_click_on_a_reasoning_or_a_compaction_shows_it_whole() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 99, 30, now);
    reasoned_and_compacted(&mut app, &link, now);
    link.take_sent();
    key(&mut app, "ctrl+o", now);
    let rows = text(&app.viewer_frame(30, 99, now));
    let reasoning = rows
        .iter()
        .position(|row| row.contains("∴ Reasoned for"))
        .unwrap_or_else(|| panic!("no reasoning: {rows:#?}"));
    click(&mut app, 2, u16::try_from(reasoning).unwrap(), now);
    let rows = text(&app.viewer_frame(30, 99, now));
    assert!(
        rows.iter().any(|row| row.contains("Last, the tests.")),
        "{rows:#?}"
    );
    key(&mut app, "esc", now);
    let rows = text(&app.viewer_frame(30, 99, now));
    let compaction = rows
        .iter()
        .position(|row| row.contains("The conversation was summarised"))
        .unwrap_or_else(|| panic!("no compaction: {rows:#?}"));
    click(&mut app, 2, u16::try_from(compaction).unwrap(), now);
    let _ = summary_read(&link);
}

/// `/view n` opens box n, counted among the boxes, with a reasoning and a
/// compaction before it: that box, open, not the stop of that number (the
/// review of step 25's second pull request).
#[test]
fn view_n_opens_the_nth_box_after_a_reasoning_and_a_compaction() {
    let now = Instant::now();
    let (mut app, link) = opened_app(Presenter::Inline, 99, 40, now);
    reasoned_and_compacted(&mut app, &link, now);
    for n in 2..=3 {
        for step in turn(n, &format!("Turn {n}."), ANSWER, OUTPUT) {
            for inbound in step {
                link.give(inbound);
            }
            drain(&mut app, now);
            let _ = app.frame(now);
        }
    }
    super::run_command(&mut app, "/view 1", now);
    let rows = text(&app.viewer_frame(40, 99, now));
    let keys = rows[39].trim_end();
    assert!(keys.ends_with("box 1 of 2"), "{rows:#?}");
    assert!(keys.contains("enter collapses"), "opened: {rows:#?}");
}

/// The approvals' answers sent so far.
fn approvals(link: &Scripted) -> usize {
    std::mem::take(&mut link.script().answers).len()
}

/// The loop's wake for the dialog: a frame at its moment. Returns it.
fn dialog_wake(app: &mut super::App) -> Instant {
    let due = app.deadline().expect("the dialog's moment");
    let _ = app.frame(due);
    app.drawn(due);
    due
}

/// In the full screen, `Tab` moves from a dialog to the transcript and back
/// while the dialog stays up (since step 25): its row offers it; while
/// reading, the dialog is its first line, the row says how to scroll and go
/// back, the keys scroll the transcript, and none of them, the dialog's own
/// answers included, answers it; back at the dialog its quiet starts again,
/// so a key just after does not answer either, and one after the quiet
/// does.
#[test]
fn tab_reads_the_transcript_under_a_dialog_and_nothing_read_answers_it() {
    use crate::app::dialog::QUIET;
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 3, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    let rows = text(&app.screen_frame(ready).0);
    assert!(
        rows.last()
            .is_some_and(|row| row.contains("tab reads above")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("Allow once")),
        "{rows:#?}"
    );
    let at = |ms: u64| ready + Duration::from_millis(ms);
    key(&mut app, "tab", at(100));
    let rows = text(&app.screen_frame(at(100)).0);
    assert!(
        rows.last()
            .is_some_and(|row| row.contains("reading the transcript")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("Run cargo build")),
        "{rows:#?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("Allow once")),
        "{rows:#?}"
    );
    assert!(app.screen_follows());
    key(&mut app, "pageup", at(200));
    let _ = app.screen_frame(at(200));
    assert!(!app.screen_follows(), "the transcript scrolled");
    // The dialog's own keys, read while reading, answer nothing.
    for (n, binding) in ["y", "s", "a", "n", "e", "enter", "1"]
        .into_iter()
        .enumerate()
    {
        key(&mut app, binding, at(300 + 10 * n as u64));
    }
    assert_eq!(approvals(&link), 0, "a key read while reading answered");
    // Back, and its quiet again: its next moment is a second on.
    let back = at(400);
    key(&mut app, "tab", back);
    assert_eq!(app.deadline(), Some(back + QUIET));
    let rows = text(&app.screen_frame(back).0);
    assert!(
        !rows.last().is_some_and(|row| row.contains("goes back")),
        "{rows:#?}"
    );
    // Just back, before it is ready: keys go nowhere, the hidden editor
    // included, whose `Enter` would send (the focused review of step 25).
    key(&mut app, "y", back + Duration::from_millis(200));
    key(&mut app, "enter", back + Duration::from_millis(300));
    assert_eq!(approvals(&link), 0, "a key just after going back answered");
    assert_eq!(app.input().editor().expanded(), "");
    assert!(
        !link
            .take_sent()
            .iter()
            .any(|(_, op)| matches!(op, cineris_protocol::methods::Op::TurnStart(_))),
        "Enter just after going back sent the editor's text"
    );
    let again = dialog_wake(&mut app);
    assert!(again >= back + QUIET, "{:?}", again.duration_since(back));
    key(&mut app, "y", again + Duration::from_millis(50));
    assert_eq!(approvals(&link), 1, "a key after the quiet answers");
}

/// A `Tab` the terminal held, pressed in the dialog's grace and read once
/// it takes keys, does not start reading the transcript: it is judged when
/// it was pressed (the full review of small fixes II).
#[test]
fn a_tab_pressed_in_the_grace_and_read_after_it_does_not_start_reading() {
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 2, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    let read = ready + Duration::from_millis(50);
    let tab = TermEvent::Key(crate::keys::parse("tab").expect("a key"));
    app.terminal_pressed(&tab, asked + Duration::from_millis(50), read);
    let rows = text(&app.screen_frame(read).0);
    assert!(
        !rows.iter().any(|row| row.contains("tab goes back")),
        "{rows:#?}"
    );
}

/// `Tab` before the dialog takes keys is not reading: it goes where it was
/// going, and the dialog is answered as usual once ready (the focused review
/// of step 25).
#[test]
fn tab_before_the_dialog_is_ready_does_not_start_reading() {
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 2, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    key(&mut app, "tab", asked + Duration::from_millis(50));
    let ready = dialog_wake(&mut app);
    let rows = text(&app.screen_frame(ready).0);
    assert!(
        rows.iter().any(|row| row.contains("Allow once")),
        "{rows:#?}"
    );
    key(&mut app, "y", ready + Duration::from_millis(50));
    assert_eq!(approvals(&link), 1, "a Tab in the grace started reading");
}

/// While reading, at 80 columns with a turn running, the row's first words
/// are the way back; `Ctrl+F` opens no search under the dialog, a paste goes
/// nowhere, a held `Tab` does not toggle, and a request that comes keeps
/// the reader's place (the focused review of step 25).
#[test]
fn reading_keeps_its_way_back_shown_and_takes_only_the_scrolling_keys() {
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 3, now);
    app.resize(80, 20);
    link.give(event(Event::TurnStarted(TurnStarted {})));
    drain(&mut app, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    let at = |ms: u64| ready + Duration::from_millis(ms);
    key(&mut app, "tab", at(100));
    let (lines, _) = app.screen_frame(at(100));
    let rows = text(&lines);
    let reading_row = rows
        .iter()
        .rev()
        .find(|row| row.contains("goes back"))
        .unwrap_or_else(|| panic!("no way back shown: {rows:#?}"));
    assert!(reading_row.contains("tab goes back"), "{rows:#?}");
    key(&mut app, "pageup", at(200));
    let _ = app.screen_frame(at(200));
    assert!(!app.screen_follows());
    key(&mut app, "ctrl+f", at(300));
    assert_eq!(app.search_query(), None, "a search opened under the dialog");
    app.terminal(&TermEvent::Paste("pasted".to_owned()), at(400));
    assert_eq!(app.input().editor().expanded(), "");
    app.terminal(
        &TermEvent::Repeat(crate::keys::parse("tab").unwrap()),
        at(500),
    );
    let rows = text(&app.screen_frame(at(500)).0);
    assert!(
        rows.iter().any(|row| row.contains("tab goes back")),
        "a held Tab went back: {rows:#?}"
    );
    link.give(Inbound::Request(
        cineris_protocol::jsonrpc::RequestId::Number(8),
        crate::testkit::approval("c_8"),
    ));
    drain(&mut app, at(600));
    let _ = app.screen_frame(at(600));
    assert!(!app.screen_follows(), "a request moved the reader's place");
}

/// The interrupt the turn has, as sent: how many.
fn interrupts(link: &Scripted) -> usize {
    link.take_sent()
        .iter()
        .filter(|(_, op)| matches!(op, cineris_protocol::methods::Op::TurnInterrupt(_)))
        .count()
}

/// `Ctrl+C` and `Ctrl+L` keep their meaning while reading and just back
/// from it, and hold nothing: just back, before the dialog is ready again,
/// `Ctrl+C` interrupts the turn, and the dialog's moment stays where going
/// back put it (the review of step 25's second pull request).
#[test]
fn ctrl_c_and_ctrl_l_act_while_reading_and_just_back() {
    use crate::app::Effect;
    use crate::app::dialog::QUIET;
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 3, now);
    link.give(event(Event::TurnStarted(TurnStarted {})));
    drain(&mut app, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    let at = |ms: u64| ready + Duration::from_millis(ms);
    key(&mut app, "tab", at(100));
    app.take_effects();
    key(&mut app, "ctrl+l", at(150));
    assert_eq!(app.take_effects(), [Effect::Redraw], "Ctrl+L while reading");
    let _ = interrupts(&link);
    key(&mut app, "tab", at(200));
    let due = app.deadline();
    assert_eq!(due, Some(at(200) + QUIET));
    key(&mut app, "ctrl+l", at(300));
    assert_eq!(app.take_effects(), [Effect::Redraw], "Ctrl+L just back");
    key(&mut app, "ctrl+c", at(400));
    assert_eq!(interrupts(&link), 1, "Ctrl+C just back was swallowed");
    assert_eq!(
        app.deadline(),
        due,
        "a global key just back held the dialog"
    );
    assert_eq!(approvals(&link), 0);
}

/// A window that shrank while reading leaves the dialog too small to take
/// keys once back: the keyboard is not caught there. `Tab` reads the
/// transcript again, and once the window grows the dialog takes its keys
/// (the review of step 25's second pull request).
#[test]
fn back_at_a_dialog_the_window_is_too_small_for_tab_reads_again() {
    use crate::app::dialog::QUIET;
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 3, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    let at = |ms: u64| ready + Duration::from_millis(ms);
    key(&mut app, "tab", at(100));
    // One row: no room for the question and its answers together.
    app.resize(79, 1);
    let _ = app.screen_frame(at(100));
    key(&mut app, "esc", at(200));
    // Its moment, in a window too small for its answers.
    let later = at(200) + QUIET * 2;
    let _ = app.screen_frame(later);
    app.drawn(later);
    assert_eq!(app.deadline(), None, "no moment: the window is too small");
    // What is typed goes nowhere: the dialog takes no key, and the hidden
    // editor none either.
    key(&mut app, "y", later + Duration::from_millis(20));
    key(&mut app, "enter", later + Duration::from_millis(30));
    assert_eq!(approvals(&link), 0);
    assert_eq!(app.input().editor().expanded(), "");
    key(&mut app, "tab", later + Duration::from_millis(50));
    let rows = text(&app.screen_frame(later + Duration::from_millis(50)).0);
    assert!(
        rows.iter().any(|row| row.contains("goes back")),
        "Tab was swallowed: {rows:#?}"
    );
    // Grown again, back, and its quiet: then its keys are its own.
    app.resize(79, 20);
    let back = later + Duration::from_millis(100);
    key(&mut app, "tab", back);
    let _ = app.screen_frame(back);
    let again = dialog_wake(&mut app);
    assert!(again >= back + QUIET, "{:?}", again.duration_since(back));
    key(&mut app, "y", again + Duration::from_millis(50));
    assert_eq!(approvals(&link), 1);
}

/// Reading ends with its dialog however it goes: resolved elsewhere, or
/// withdrawn by the turn's end. The next dialog then follows the end, as
/// any does, wherever the reader had scrolled to (the review of step 25's
/// second pull request).
#[test]
fn a_dialog_read_under_that_ends_elsewhere_leaves_the_next_following_the_end() {
    use std::time::Duration;

    for resolved in [true, false] {
        let now = Instant::now();
        let (mut app, link) = long_session(Presenter::Fullscreen, 3, now);
        link.give(event(Event::TurnStarted(TurnStarted {})));
        drain(&mut app, now);
        let asked = now + Duration::from_secs(5);
        crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
        let ready = dialog_wake(&mut app);
        let at = |ms: u64| ready + Duration::from_millis(ms);
        key(&mut app, "tab", at(100));
        key(&mut app, "pageup", at(200));
        let _ = app.screen_frame(at(200));
        assert!(!app.screen_follows());
        if resolved {
            link.give(event(Event::RequestResolved(
                cineris_protocol::event::RequestResolved {
                    request_id: cineris_protocol::jsonrpc::RequestId::Number(7),
                    resolution: cineris_protocol::event::Resolution::Answered,
                },
            )));
        } else {
            link.give(event(Event::TurnCompleted(TurnCompleted {
                outcome: TurnOutcome::Interrupted,
                stop_reason: None,
                error: None,
                usage: Default::default(),
                verification: None,
                duration_ms: Some(900),
            })));
            link.give(event(Event::TurnStarted(TurnStarted {})));
        }
        drain(&mut app, at(300));
        let _ = app.screen_frame(at(300));
        crate::testkit::ask(&mut app, &link, 8, crate::testkit::approval("c_8"), at(400));
        assert!(
            app.screen_follows(),
            "resolved elsewhere {resolved}: the next dialog left the reader's place"
        );
    }
}

/// `tab reads above` is offered in the frame that first draws the answers:
/// a dialog with nothing moving gets no frame after it (the review of step
/// 25's second pull request).
#[test]
fn tab_reads_above_is_offered_in_the_frame_that_draws_the_answers() {
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 2, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let due = app.deadline().expect("the dialog's moment");
    let rows = text(&app.screen_frame(due).0);
    assert!(
        rows.iter().any(|row| row.contains("Allow once")),
        "{rows:#?}"
    );
    assert!(
        rows.last()
            .is_some_and(|row| row.contains("tab reads above")),
        "{rows:#?}"
    );
}

/// The wheel while reading scrolls the transcript and answers nothing;
/// `Tab` while a note is being written is not reading, and is not offered
/// then (the review of step 25's second pull request).
#[test]
fn the_wheel_scrolls_while_reading_and_a_note_being_written_keeps_tab() {
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 3, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    let at = |ms: u64| ready + Duration::from_millis(ms);
    key(&mut app, "tab", at(100));
    let _ = app.screen_frame(at(100));
    assert!(app.screen_follows());
    app.terminal(
        &TermEvent::Mouse(Mouse {
            kind: MouseKind::Wheel(Wheel::Up),
            col: 5,
            row: 5,
            mods: Mods::NONE,
        }),
        at(200),
    );
    let rows = text(&app.screen_frame(at(200)).0);
    assert!(!app.screen_follows(), "the wheel did not scroll");
    assert!(
        rows.iter().any(|row| row.contains("goes back")),
        "{rows:#?}"
    );
    assert_eq!(approvals(&link), 0);

    // A note: `Tab` is not reading, and the row does not offer it.
    let (mut app, link) = long_session(Presenter::Fullscreen, 2, now);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    let at = |ms: u64| ready + Duration::from_millis(ms);
    key(&mut app, "e", at(100));
    let rows = text(&app.screen_frame(at(100)).0);
    assert!(
        !rows.iter().any(|row| row.contains("reads above")),
        "{rows:#?}"
    );
    key(&mut app, "tab", at(200));
    let rows = text(&app.screen_frame(at(200)).0);
    assert!(
        !rows.iter().any(|row| row.contains("tab goes back")),
        "Tab started reading over a note: {rows:#?}"
    );
    super::type_text(&mut app, "no", at(300));
    key(&mut app, "enter", at(400));
    assert_eq!(approvals(&link), 1, "the note was not sent");
}

/// Inline, `Tab` under a dialog starts no reading: the dialog answers as
/// before.
#[test]
fn inline_tab_under_a_dialog_starts_no_reading() {
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Inline, 2, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    key(&mut app, "tab", ready + Duration::from_millis(50));
    key(&mut app, "y", ready + Duration::from_millis(100));
    assert_eq!(approvals(&link), 1);
}

/// `Esc` while reading goes back to the dialog too, and does not deny it;
/// the inline screen offers no reading, which has the terminal's own
/// scrollback (since step 25).
#[test]
fn esc_while_reading_goes_back_and_the_inline_screen_offers_no_reading() {
    use std::time::Duration;

    let now = Instant::now();
    let (mut app, link) = long_session(Presenter::Fullscreen, 2, now);
    let asked = now + Duration::from_secs(5);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    key(&mut app, "tab", ready + Duration::from_millis(100));
    key(&mut app, "esc", ready + Duration::from_millis(200));
    assert_eq!(approvals(&link), 0, "esc while reading denied the call");
    let rows = text(&app.screen_frame(ready + Duration::from_millis(200)).0);
    assert!(
        rows.last().is_some_and(|row| row.contains("esc denies"))
            && !rows.iter().any(|row| row.contains("goes back")),
        "back at the dialog: {rows:#?}"
    );

    // Reading ends with its dialog however it goes: one resolved elsewhere,
    // and the next opens as a dialog, not read.
    let ready = dialog_wake(&mut app);
    key(&mut app, "tab", ready + Duration::from_millis(50));
    let rows = text(&app.screen_frame(ready + Duration::from_millis(50)).0);
    assert!(
        rows.iter().any(|row| row.contains("goes back")),
        "{rows:#?}"
    );
    link.give(event(Event::RequestResolved(
        cineris_protocol::event::RequestResolved {
            request_id: cineris_protocol::jsonrpc::RequestId::Number(7),
            resolution: cineris_protocol::event::Resolution::Answered,
        },
    )));
    drain(&mut app, ready + Duration::from_millis(100));
    crate::testkit::ask(
        &mut app,
        &link,
        8,
        crate::testkit::approval("c_8"),
        ready + Duration::from_millis(200),
    );
    let rows = text(&app.screen_frame(ready + Duration::from_millis(200)).0);
    assert!(
        !rows.iter().any(|row| row.contains("goes back")),
        "the next dialog opened read: {rows:#?}"
    );
    let _ = approvals(&link);
    // Well past its grace and the quiet of the last key read: its answers
    // drawn, and its own keys its own.
    let later = ready + Duration::from_millis(200) + crate::app::dialog::QUIET * 2;
    let _ = app.frame(later);
    app.drawn(later);
    key(&mut app, "y", later + Duration::from_millis(50));
    assert_eq!(
        approvals(&link),
        1,
        "the next dialog's keys were read as reading"
    );

    let (mut app, link) = long_session(Presenter::Inline, 2, now);
    crate::testkit::ask(&mut app, &link, 7, crate::testkit::approval("c_7"), asked);
    let ready = dialog_wake(&mut app);
    let (_, live, _) = app.frame(ready);
    assert!(
        !text(&live).iter().any(|row| row.contains("reads above")),
        "{:#?}",
        text(&live)
    );
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
                result.outcome = cineris_protocol::item::ToolOutcome::Success;
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
/// screen once they come (closes `highlighted-code-drawn-again`): taken by
/// the loop's drain, with the full screen's view in its place.
#[cfg(feature = "highlight")]
#[test]
fn code_committed_plain_is_coloured_once_its_colours_come() {
    coloured_once_they_come(true);
}

/// The same, the colours taken by the frame itself, while the full
/// screen's view is out to make it: the block is drawn again in that frame
/// (the break `the-full-screen-keeps-the-plain-rows` went uncaught when
/// only the drain's way was tested; found in step 25's third pull request).
#[cfg(feature = "highlight")]
#[test]
fn code_coloured_while_a_frame_is_made_is_drawn_in_that_frame() {
    coloured_once_they_come(false);
}

/// Late colours coming, taken by the loop's drain (`by_drain`) or by the
/// next frame.
#[cfg(feature = "highlight")]
fn coloured_once_they_come(by_drain: bool) {
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
    // Their coming is a change the loop draws, with nothing else happening
    // (the review of step 25); or the frame takes them itself.
    if by_drain {
        assert!(app.drain(later, 64).changed, "the colours schedule a frame");
    }
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
/// cargo test --release -p cineris-tui --features test-kit --lib perf_a_frame_in_the_full_screen -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, in a release build"]
fn perf_a_frame_in_the_full_screen_presenter() {
    use cineris_term::{Full, Screen};

    let runs: usize = std::env::var("CINERIS_PERF_RUNS")
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
        cineris_testkit::numbers::report(&format!("full_screen.{label}.median"), median, "ms");
        cineris_testkit::numbers::report(&format!("full_screen.{label}.p99"), p99, "ms");
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
    let answer = cineris_protocol::ItemId::from("streaming");
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
    // The same answer, 2,500 paragraphs long and still open: the rows it
    // has committed are copied for each frame (the review of step 25).
    for k in 0..2_500 {
        link.give(event(Event::ItemDelta(ItemDelta {
            item_id: answer.clone(),
            delta: Delta::Text(TextDelta {
                text: format!("\n\nParagraph {k} of a long answer."),
            }),
        })));
    }
    drain(&mut app, now);
    let _ = app.screen_frame(now);
    let mut n = 0;
    measure("streaming_a_long_answer", &mut app, &mut |app| {
        n += 1;
        link.give(event(Event::ItemDelta(ItemDelta {
            item_id: answer.clone(),
            delta: Delta::Text(TextDelta {
                text: format!(" word{n}"),
            }),
        })));
        drain(app, now);
    });
}

/// What a played terminal reads, in turn.
type Steps = Vec<(cineris_term::Ready, Vec<u8>)>;

/// What the loop wrote, played into a virtual terminal: the modes it left.
fn modes_after(written: &[u8]) -> cineris_testkit::vterm::Modes {
    let mut term = cineris_testkit::VTerm::new(100, 30);
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
/// mouse reported, and leaves both off when it quits, the core goes, the
/// terminal fails under it, or the program is suspended (by a key bound
/// to it, after which it takes the screen again); the mouse is never on
/// with the main screen.
#[test]
fn every_way_out_of_the_full_screen_leaves_the_mouse_and_the_alternate_screen_off() {
    use cineris_term::Ready;
    let ways: [(&str, Steps); 4] = [
        (
            "quit",
            vec![(Ready::Woken, Vec::new()), (Ready::Input, b"\x04".to_vec())],
        ),
        (
            "suspended",
            vec![
                (Ready::Woken, Vec::new()),
                // F9, bound to `global.suspend` for this way.
                (Ready::Input, b"\x1b[20~".to_vec()),
                // Continued (`fg`), and a frame's time passes.
                (Ready::Resumed, Vec::new()),
                (Ready::Timeout, Vec::new()),
                (Ready::Input, b"\x04".to_vec()),
            ],
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
        if way == "suspended" {
            let f9 = vec!["f9".to_owned()];
            let (keymap, errors) =
                crate::keys::KeyMap::with_table([("global.suspend", f9.as_slice())]);
            assert!(errors.is_empty(), "{errors:?}");
            app.set_keymap(keymap);
        }
        link.give(Inbound::Done(
            crate::app::Ticket(1),
            Ok(cineris_protocol::methods::OpResult::SessionNew(
                cineris_protocol::methods::SessionResult {
                    session: crate::testkit::session().build(),
                },
            )),
        ));
        if way == "the core gone" {
            link.give(Inbound::Gone(cineris_protocol::ErrorData::new(
                cineris_protocol::ErrorKind::Internal,
                "gone",
                "the core went away",
            )));
        }
        let (device, written) = played(steps, base);
        let mut terminal = cineris_term::Terminal::with_device(device, crate::testkit::caps())
            .expect("a terminal");
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
        if way == "suspended" {
            // On, then off with the mouse first, then the stop, then on
            // again once continued.
            let on: &[u8] = b"\x1b[?1049h\x1b[?1000h\x1b[?1006h";
            let off: &[u8] = b"\x1b[?1006l\x1b[?1000l\x1b[?1049l";
            let at = |part: &[u8], from: usize| {
                during[from..]
                    .windows(part.len())
                    .position(|window| window == part)
                    .map(|found| found + from)
            };
            let first_on = at(on, 0).expect("on");
            let first_off = at(off, first_on).expect("off before the stop");
            let stopped = at(super::SUSPENDED, first_off).expect("the stop");
            assert!(at(on, stopped).is_some(), "on again after the stop");
        }
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
    use cineris_term::Ready;
    let base = Instant::now();
    let (mut app, link) = crate::testkit::app(base);
    link.give(Inbound::Done(
        crate::app::Ticket(1),
        Ok(cineris_protocol::methods::OpResult::SessionNew(
            cineris_protocol::methods::SessionResult {
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
        cineris_term::Terminal::with_device(device, crate::testkit::caps()).expect("a terminal");
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

/// The keyboard protocol's flags, which the terminal's reply turns on, are
/// left pushed on neither screen of a terminal that keeps a stack per
/// screen (kitty, ghostty, the test kit's `VTerm`): by the full screen,
/// whose first frame takes the alternate screen before the reply is read,
/// when it quits and when the terminal fails under it; and by the viewer
/// opened from the inline screen after the reply (the fixes of step 25's
/// review, finding 10: the full screen left them on the alternate screen,
/// where the next program that took it got keys it never asked for).
#[test]
fn the_full_screen_and_the_viewer_leave_no_keyboard_flags_on_either_screen() {
    use cineris_term::Ready;
    let reply = b"\x1b[?0u\x1b[?62;22c".to_vec();
    let ways: [(&str, Presenter, Steps); 3] = [
        (
            "the full screen quits",
            Presenter::Fullscreen,
            vec![
                (Ready::Woken, Vec::new()),
                (Ready::Input, reply.clone()),
                (Ready::Input, b"\x04".to_vec()),
            ],
        ),
        // The script ends: the terminal fails under the loop.
        (
            "the terminal fails under the full screen",
            Presenter::Fullscreen,
            vec![(Ready::Woken, Vec::new()), (Ready::Input, reply.clone())],
        ),
        (
            "the viewer opened and closed",
            Presenter::Inline,
            vec![
                (Ready::Woken, Vec::new()),
                (Ready::Input, reply.clone()),
                // Ctrl+O, then Esc on its own.
                (Ready::Input, b"\x0f".to_vec()),
                (Ready::Timeout, Vec::new()),
                (Ready::Input, b"\x1b".to_vec()),
                (Ready::Timeout, Vec::new()),
                (Ready::Timeout, Vec::new()),
                (Ready::Input, b"\x04".to_vec()),
            ],
        ),
    ];
    for (way, presenter, steps) in ways {
        let base = Instant::now();
        let settings = Settings {
            presenter,
            ..Settings::default()
        };
        let (mut app, link) = app_with_settings(base, settings);
        link.give(Inbound::Done(
            crate::app::Ticket(1),
            Ok(cineris_protocol::methods::OpResult::SessionNew(
                cineris_protocol::methods::SessionResult {
                    session: crate::testkit::session().build(),
                },
            )),
        ));
        let (device, written) = played(steps, base);
        let mut terminal = cineris_term::Terminal::with_device(device, crate::testkit::caps())
            .expect("a terminal");
        let ended = crate::app::run(&mut terminal, &mut app);
        assert_eq!(ended.is_err(), way.contains("fails"), "{way}: {ended:?}");
        terminal.leave().expect("left");
        let all = bytes(&written);
        assert!(holds(&all, b"\x1b[>3u"), "{way}: the flags were pushed");
        assert!(holds(&all, b"\x1b[?1049h"), "{way}: the alternate screen");
        let mut term = cineris_testkit::VTerm::new(100, 30);
        // The shell's own entry, pushed before the program: it is still
        // there (the second round of the fixes of step 25's review).
        term.feed(b"\x1b[>1u");
        term.feed(&all);
        assert!(
            term.main_keyboard() == [1] && term.alternate_keyboard().is_empty(),
            "{way}: the main screen's flags {:?} (the shell's is [1]), the alternate screen's {:?}",
            term.main_keyboard(),
            term.alternate_keyboard()
        );
        assert!(!term.modes().alternate, "{way}");
    }
}
