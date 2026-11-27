//! What the application holds of each turn's reasoning, in each presenter,
//! counted by a counting allocator (since the fixes of the review of step
//! 25, which found every reasoning held twice and without a bound: a copy
//! of its text as its summary, both kept). The allocator counts the whole
//! process, which nextest gives each test to itself.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;

use cineris_protocol::event::{
    Delta, Event, ItemCompleted, ItemDelta, ItemKind, ItemStarted, TextDelta, TurnCompleted,
    TurnOutcome, TurnStarted,
};
use cineris_protocol::item::{AssistantMessageItem, StopReason, UserMessageItem};
use cineris_protocol::message::Part;
use cineris_protocol::{Item, ItemId, TurnId};
use cineris_tui::app::{App, Inbound, Presenter, Settings};
use cineris_tui::testkit::{Scripted, app_with_settings, drain, event, opened};
use cineris_tui::transcript::kept::REASONING_KEPT;

/// The system's allocator, counting the bytes it has given out and not had
/// back.
struct Counting;

/// The bytes allocated and not freed.
static LIVE: AtomicIsize = AtomicIsize::new(0);

#[allow(
    clippy::cast_possible_wrap,
    reason = "an allocation is at most isize::MAX bytes"
)]
fn signed(bytes: usize) -> isize {
    bytes as isize
}

#[allow(
    unsafe_code,
    reason = "a counting global allocator, for the measurement"
)]
// SAFETY: every call goes to the system allocator as it came; the counter
// is only added to and taken from.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is the system allocator's.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(signed(layout.size()), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract is the system allocator's.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(signed(layout.size()), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's contract is the system allocator's.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_add(signed(new_size) - signed(layout.size()), Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn in_turn(turn: u64, inbound: Inbound) -> Inbound {
    match inbound {
        Inbound::Event(mut event) => {
            event.turn_id = Some(TurnId::from(format!("t_{turn}").as_str()));
            Inbound::Event(event)
        }
        other => other,
    }
}

/// A turn whose one response reasons `reasoning`, streamed in pieces of
/// 4 KiB, then answers `answer`; a frame after each event.
fn turn(app: &mut App, link: &Scripted, n: u64, reasoning: &str, answer: &str, now: Instant) {
    let id = ItemId::from(format!("a{n}").as_str());
    let mut steps = vec![
        event(Event::TurnStarted(TurnStarted {})),
        event(Event::ItemCompleted(ItemCompleted {
            item: Item::UserMessage(UserMessageItem {
                id: ItemId::from(format!("u{n}").as_str()),
                parts: vec![Part::text(format!("Turn {n}."))],
                delivery: None,
                command: None,
                created_at: None,
            }),
        })),
        event(Event::ItemStarted(ItemStarted {
            item_id: id.clone(),
            kind: ItemKind::AssistantMessage,
            tool: None,
            call_id: None,
        })),
    ];
    for piece in reasoning.as_bytes().chunks(4096) {
        steps.push(event(Event::ItemDelta(ItemDelta {
            item_id: id.clone(),
            delta: Delta::Reasoning(TextDelta {
                text: String::from_utf8_lossy(piece).into_owned(),
            }),
        })));
    }
    steps.push(event(Event::ItemDelta(ItemDelta {
        item_id: id.clone(),
        delta: Delta::Text(TextDelta {
            text: answer.to_owned(),
        }),
    })));
    steps.push(event(Event::ItemCompleted(ItemCompleted {
        item: Item::AssistantMessage(AssistantMessageItem {
            id,
            parts: vec![Part::text(answer)],
            stop_reason: Some(StopReason::End),
            raw_stop: None,
            stop_detail: None,
            truncated: false,
            origin: None,
            usage: None,
            error: None,
            created_at: None,
        }),
    })));
    steps.push(event(Event::TurnCompleted(TurnCompleted {
        outcome: TurnOutcome::Completed,
        stop_reason: Some(StopReason::End),
        error: None,
        usage: Default::default(),
        verification: None,
        duration_ms: Some(900),
    })));
    for inbound in steps {
        link.give(in_turn(n, inbound));
        drain(app, now);
        if app.full_screen() {
            let _ = app.screen_frame(now);
        } else {
            let _ = app.frame(now);
        }
    }
}

/// The heap the application holds after `turns` turns more than it held
/// after the first `turns`, each turn reasoning `reasoning` bytes (ASCII,
/// in lines). Both are read in one session, each after a change of width,
/// which drops every row the full screen made: it makes again only those
/// of the window at the end, the same after any number of turns, so they
/// drop out of the difference.
fn held_by_later_turns(presenter: Presenter, turns: u64, reasoning: usize) -> isize {
    let now = Instant::now();
    let settings = Settings {
        presenter,
        ..Settings::default()
    };
    let (mut app, link) = app_with_settings(now, settings);
    app.resize(99, 40);
    opened(&mut app, &link, now);
    let words = "the scanner reads each digit and an underscore between two of them\n";
    let reasoning: String = words.repeat(reasoning / words.len() + 1)[..reasoning].to_owned();
    let answer = "Paragraph: the scanner now treats an underscore as a separator.\n\n".repeat(12);
    let held_at = |app: &mut App, width: usize| {
        app.resize(width, 40);
        if app.full_screen() {
            let _ = app.screen_frame(now);
        } else {
            let _ = app.frame(now);
        }
        LIVE.load(Ordering::Relaxed)
    };
    for n in 1..=turns {
        turn(&mut app, &link, n, &reasoning, &answer, now);
    }
    let first = held_at(&mut app, 98);
    for n in turns + 1..=turns * 2 {
        turn(&mut app, &link, n, &reasoning, &answer, now);
    }
    let later = held_at(&mut app, 97);
    drop(app);
    later - first
}

/// What each further turn's reasoning of `bytes` adds to what each
/// presenter holds: what the turns from the 5th to the 8th hold more than
/// the first 4, with reasoning and without, a turn. What a presenter holds
/// whatever the session's length (the rows it has made near its window) is
/// in both, and drops out.
fn per_turn(bytes: usize) -> Vec<(Presenter, isize)> {
    const TURNS: u64 = 4;
    [Presenter::Inline, Presenter::Fullscreen]
        .into_iter()
        .map(|presenter| {
            let more = |reasoning| held_by_later_turns(presenter, TURNS, reasoning);
            let per_turn = (more(bytes) - more(0)) / isize::try_from(TURNS).unwrap_or(1);
            println!(
                "{presenter:?}: {bytes} bytes of reasoning a turn hold {per_turn} bytes a turn \
                 more ({:.2} times the reasoning)",
                per_turn as f64 / bytes as f64
            );
            (presenter, per_turn)
        })
        .collect()
}

/// A reasoning shorter than a kept block's bound is held once, in each
/// presenter: its words moved from the stream to the finished block, not
/// copied (on `main` before the fix, twice).
#[test]
fn a_finished_reasoning_is_held_once_in_each_presenter() {
    let bytes = REASONING_KEPT * 3 / 4;
    for (presenter, per_turn) in per_turn(bytes) {
        assert!(
            per_turn < signed(bytes) * 3 / 2,
            "{presenter:?}: {per_turn} bytes held a turn for {bytes} bytes of reasoning"
        );
    }
}

/// A reasoning longer than a kept block's bound is held to the bound, in
/// each presenter, however long it is: the viewer reads the rest from its
/// item when it shows it whole.
#[test]
fn a_long_reasoning_is_held_to_its_bound_in_each_presenter() {
    let bytes = REASONING_KEPT * 2;
    for (presenter, per_turn) in per_turn(bytes) {
        assert!(
            per_turn < signed(REASONING_KEPT) * 3 / 2,
            "{presenter:?}: {per_turn} bytes held a turn for {bytes} bytes of reasoning, \
             where {REASONING_KEPT} are kept"
        );
    }
}
