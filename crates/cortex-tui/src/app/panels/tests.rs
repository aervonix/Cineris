//! The panel mechanism: the layout and the live region's limit, the focus
//! kept on its item, what can be focused, and the sweep of every window
//! height and every set of panels through the application's frame.

use std::time::{Duration, Instant};

use cortex_protocol::event::{
    Actor, CheckOutcome, CheckReport, CheckSource, Delta, ItemCompleted, ItemDelta, ItemKind,
    ItemStarted, Notice, PlanUpdated, ToolCallDelta, TurnCompleted, TurnOutcome, TurnStarted,
    VerificationNotice, VerificationPhase,
};
use cortex_protocol::item::{PlanItem, PlanStatus, ToolOutcome, ToolResultItem};
use cortex_protocol::{CallId, Item, ItemId, PlanItemId};
use cortex_term::ColorDepth;

use super::*;
use crate::app::App;
use crate::focus::Target;
use crate::glyphs::Glyphs;
use crate::layout::cut;
use crate::testing::{self, DARK};
use crate::testkit::{app, drain, event, opened};
use crate::theme::Theme;

/// A panel of a set shape: `items` items of `rows` rows each, and
/// `collapsed` rows without the focus. It sees nothing and takes no key.
#[derive(Debug)]
pub(crate) struct StandIn {
    pub(crate) name: &'static str,
    pub(crate) items: usize,
    pub(crate) rows: usize,
    pub(crate) collapsed: usize,
}

impl Panel for StandIn {
    fn name(&self) -> &'static str {
        self.name
    }

    fn seen(&mut self, _seen: &Seen<'_>) -> Show {
        Show::Same
    }

    fn summon(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn collapsed(&self, draw: &Draw<'_>) -> Vec<Line> {
        (0..self.collapsed)
            .map(|row| {
                cut(
                    &Line::plain(format!("{} collapsed {row}", self.name)),
                    draw.width,
                    draw.look.glyphs.ellipsis,
                )
            })
            .collect()
    }

    fn expanded(&self, focused: usize, draw: &Draw<'_>) -> Vec<PanelRow> {
        let mut rows = vec![PanelRow::plain(Line::plain(format!("◇ {}", self.name)))];
        for item in 0..self.items {
            for row in 0..self.rows {
                let mark = if item == focused { ">" } else { " " };
                rows.push(PanelRow::of(
                    item,
                    cut(
                        &Line::plain(format!(" {mark}{} item {item} row {row}", self.name)),
                        draw.width,
                        draw.look.glyphs.ellipsis,
                    ),
                ));
            }
        }
        rows
    }

    fn items(&self) -> usize {
        self.items
    }

    fn item(&self, index: usize) -> Option<ItemKey> {
        (index < self.items).then(|| ItemKey {
            id: index.to_string(),
            content: String::new(),
        })
    }

    fn hint(&self, _focused: usize, _keymap: &KeyMap) -> Vec<String> {
        vec!["esc back to the input".to_owned()]
    }

    fn act(&mut self, _action: Action, _focused: usize, _session: Option<&SessionId>) -> Act {
        Act::Pass
    }
}

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

fn standin(name: &'static str, items: usize, rows: usize, collapsed: usize) -> Box<dyn Panel> {
    Box::new(StandIn {
        name,
        items,
        rows,
        collapsed,
    })
}

fn texts(lines: &[Line]) -> Vec<String> {
    lines.iter().map(|line| line.text().into_owned()).collect()
}

#[test]
fn the_focused_panel_scrolls_to_keep_its_row_with_a_dim_row_at_each_cut_end() {
    let look = look();
    let draw = Draw {
        width: 59,
        look: &look,
        tick: 0,
    };
    let mut panels = Panels::new();
    let tall = panels.register(standin("tall", 12, 1, 2));
    panels.set_open(&[tall]);
    panels.focused(tall);
    for _ in 0..6 {
        panels.moved(tall, true);
    }
    assert_eq!(panels.focus_of(tall), 6);
    let laid = panels.layout(6, Some(tall), &draw, &KeyMap::defaults());
    let rows = texts(&laid.lines);
    assert_eq!(rows.len(), 6, "{rows:#?}");
    assert_eq!(rows[0], "◇ tall");
    assert!(
        rows[1].starts_with("  … ") && rows[1].ends_with(" more"),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains(">tall item 6")),
        "{rows:#?}"
    );
    assert!(rows[5].starts_with("  … "), "{rows:#?}");
    // The hidden rows are counted: above and below, with what is shown,
    // the twelve items.
    let count = |row: &str| -> usize {
        row.trim_start_matches("  … ")
            .trim_end_matches(" more")
            .parse()
            .unwrap()
    };
    assert_eq!(count(&rows[1]) + 3 + count(&rows[5]), 12);
    assert_eq!(laid.focused, Some(tall));
    // At the end of its list, no row below.
    for _ in 0..10 {
        panels.moved(tall, true);
    }
    let rows = texts(
        &panels
            .layout(6, Some(tall), &draw, &KeyMap::defaults())
            .lines,
    );
    assert!(rows[5].contains(">tall item 11"), "{rows:#?}");
    // Room for all of it: no dim row.
    let rows = texts(
        &panels
            .layout(20, Some(tall), &draw, &KeyMap::defaults())
            .lines,
    );
    assert_eq!(rows.len(), 13);
    assert!(!rows.iter().any(|row| row.contains("more")));
}

#[test]
fn the_oldest_collapsed_panels_fold_into_one_row_and_with_no_room_nothing_is_drawn() {
    let look = look();
    let draw = Draw {
        width: 59,
        look: &look,
        tick: 0,
    };
    let mut panels = Panels::new();
    let first = panels.register(standin("first", 3, 1, 3));
    let second = panels.register(standin("second", 1, 1, 1));
    let third = panels.register(standin("third", 1, 1, 1));
    panels.set_open(&[first, second, third]);
    let keymap = KeyMap::defaults();
    let all = panels.layout(5, None, &draw, &keymap);
    assert_eq!(all.lines.len(), 5);
    assert_eq!(all.drawn, [first, second, third]);
    // One row short: the oldest folds.
    let folded = panels.layout(3, None, &draw, &keymap);
    assert_eq!(
        texts(&folded.lines),
        [
            "+1 panel · ctrl+p",
            "second collapsed 0",
            "third collapsed 0"
        ]
    );
    assert_eq!(folded.drawn, [second, third]);
    let one = panels.layout(1, None, &draw, &keymap);
    assert_eq!(texts(&one.lines), ["+3 panels · ctrl+p"]);
    assert!(one.drawn.is_empty());
    // The folded ones that fit focused are still reachable, and drawn
    // first when they have the focus.
    assert_eq!(folded.reachable, [first, second, third]);
    let focused = panels.layout(3, Some(first), &draw, &keymap);
    assert_eq!(focused.focused, Some(first));
    assert_eq!(texts(&focused.lines)[0], "+2 panels · ctrl+p");
    assert!(texts(&focused.lines)[1].starts_with("◇ first"));
    // No room: nothing, and nothing reachable.
    let none = panels.layout(0, Some(first), &draw, &keymap);
    assert_eq!(none, Laid::default());
}

#[test]
fn a_panel_that_would_not_be_drawn_focused_cannot_be_reached() {
    let look = look();
    let draw = Draw {
        width: 59,
        look: &look,
        tick: 0,
    };
    let mut panels = Panels::new();
    // Its focused item alone is three rows, under its first row.
    let tall = panels.register(standin("tall", 2, 3, 1));
    let small = panels.register(standin("small", 1, 1, 1));
    panels.set_open(&[tall, small]);
    let keymap = KeyMap::defaults();
    let laid = panels.layout(3, Some(tall), &draw, &keymap);
    assert_eq!(laid.reachable, [small], "four rows do not fit in three");
    assert_eq!(laid.focused, None, "drawn as if nothing had the focus");
    assert_eq!(
        texts(&laid.lines),
        ["tall collapsed 0", "small collapsed 0"]
    );
    let roomy = panels.layout(4, Some(tall), &draw, &keymap);
    assert_eq!(roomy.focused, Some(tall));
    assert_eq!(
        roomy.lines.len(),
        4,
        "the other folds away: no row is left for it"
    );
}

#[test]
fn the_window_keeps_the_wanted_rows_and_moves_as_little_as_it_can() {
    assert_eq!(window(5, 8, 2..3, 0), (0, 5, false), "all of it fits");
    // Ten rows in five: a dim row at each cut end.
    assert_eq!(window(10, 5, 0..1, 0), (0, 4, true));
    assert_eq!(window(10, 5, 4..5, 0), (2, 5, true));
    assert_eq!(window(10, 5, 9..10, 0), (6, 10, true));
    // From where it was, the nearest start that shows the wanted rows.
    assert_eq!(window(10, 5, 5..6, 3), (3, 6, true));
    // No room for a dim row beside the wanted ones: those alone.
    assert_eq!(window(10, 1, 4..5, 0), (4, 5, false));
    assert_eq!(window(10, 2, 3..6, 0), (3, 5, false));
    // Nothing wanted: from where it was.
    assert_eq!(window(10, 4, 0..0, 4), (4, 6, true));
}

// ------------------------------------------------------------- the sweep

/// A plan of four items, one active.
fn plan_event() -> Event {
    let item = |id: &str, status| PlanItem {
        id: PlanItemId::from(id),
        text: format!("the {id} step of the plan"),
        status,
    };
    Event::PlanUpdated(PlanUpdated {
        plan: cortex_protocol::Plan {
            items: vec![
                item("p1", PlanStatus::Done),
                item("p2", PlanStatus::Active),
                item("p3", PlanStatus::Pending),
                item("p4", PlanStatus::Pending),
            ],
            revision: 1,
        },
        by: Actor::Agent,
    })
}

/// A turn that edited two files and ran its checks, with a call still
/// running when `running`: its round's box is live.
fn turn_events(running: bool) -> Vec<Event> {
    let call = |id: &str, name: &str| {
        Event::ItemDelta(ItemDelta {
            item_id: ItemId::from("m1"),
            delta: Delta::ToolCall(ToolCallDelta {
                call_id: CallId::from(id),
                name: name.to_owned(),
            }),
        })
    };
    let edit = |id: &str, path: &str| {
        Event::ItemCompleted(ItemCompleted {
            item: Item::ToolResult(ToolResultItem {
                id: ItemId::from(format!("r_{id}").as_str()),
                call_id: CallId::from(id),
                tool: "edit".to_owned(),
                outcome: ToolOutcome::Success,
                content: Vec::new(),
                payload: Some(
                    serde_json::json!({"path": path, "kind": "edit", "added": 4, "removed": 1}),
                ),
                artifacts: Vec::new(),
                duration_ms: Some(20),
                created_at: None,
            }),
        })
    };
    let check = |command: &str| CheckReport {
        command: command.to_owned(),
        cwd: None,
        source: CheckSource::User,
        entry: None,
        outcome: Some(CheckOutcome::Passed),
        exit_code: Some(0),
        duration_ms: Some(1_200),
        note: None,
        artifact: None,
    };
    let mut events = vec![
        Event::TurnStarted(TurnStarted {}),
        Event::ItemStarted(ItemStarted {
            item_id: ItemId::from("m1"),
            kind: ItemKind::AssistantMessage,
            tool: None,
            call_id: None,
        }),
        call("c1", "edit"),
        call("c2", "edit"),
        call("c3", "shell"),
        edit("c1", "src/lex.rs"),
        edit("c2", "tests/lex.rs"),
        Event::Notice(Notice::Verification(VerificationNotice {
            message: "The checks passed.".to_owned(),
            phase: VerificationPhase::Finished,
            round: 1,
            rounds: 2,
            checks: vec![check("cargo test"), check("cargo clippy")],
            verdict: Some(cortex_protocol::event::Verification::Passed),
        })),
    ];
    if !running {
        events.push(Event::TurnCompleted(TurnCompleted {
            outcome: TurnOutcome::Completed,
            stop_reason: None,
            error: None,
            usage: Default::default(),
            verification: Some(cortex_protocol::event::Verification::Passed),
            duration_ms: Some(1_000),
        }));
    }
    events
}

/// An application with the four panels' state: the plan and the review
/// from the core's events, and two stand-ins of other shapes. Their ids,
/// in that order.
fn four_panels(now: Instant, running: bool) -> (App, [u64; 4]) {
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    for event_of_turn in turn_events(running) {
        link.give(event(event_of_turn));
    }
    link.give(event(plan_event()));
    drain(&mut app, now);
    let plan = app.panels_mut().find("plan").unwrap();
    let review = app.panels_mut().find("review").unwrap();
    assert!(app.panels_mut().is_open(plan) && app.panels_mut().is_open(review));
    let tall = app.panels_mut().register(standin("tall", 15, 1, 3));
    let tiny = app.panels_mut().register(standin("tiny", 2, 2, 1));
    (app, [plan, review, tall, tiny])
}

#[test]
fn the_live_region_keeps_its_limit_and_a_focused_panel_is_drawn_or_the_input_has_the_focus() {
    let now = Instant::now() + Duration::from_secs(1);
    let columns = 60;
    let width = columns - 1;
    for running in [false, true] {
        let (mut app, ids) = four_panels(now, running);
        assert_eq!(app.running(), running);
        for set in 0..16_u32 {
            let open: Vec<u64> = (0..4)
                .filter(|bit| set & (1 << bit) != 0)
                .map(|bit| ids[bit])
                .collect();
            for rows in 4..=60 {
                for focus in std::iter::once(None).chain(open.iter().copied().map(Some)) {
                    app.panels_mut().set_open(&open);
                    app.focus_panel(focus);
                    app.resize(width, rows);
                    let (_, live, _) = app.frame(now);
                    let context =
                        format!("running {running}, panels {open:?}, {rows} rows, focus {focus:?}");
                    // The input box (3 rows) and the hint row are the bottom.
                    let limit = (rows / 2).max(4);
                    assert!(live.len() <= limit, "{context}: {} rows", live.len());
                    let problems = testing::problems(&live, width);
                    assert!(problems.is_empty(), "{context}: {problems:?}");
                    match app.input().focus().focused() {
                        Target::Panel(id) => {
                            assert_eq!(app.panel_drawn(), Some(id), "{context}");
                            let look = app.look().clone();
                            let draw = Draw {
                                width,
                                look: &look,
                                tick: 0,
                            };
                            let wanted = texts(&app.panels_mut().focused_lines(id, &draw));
                            let shown = texts(&live);
                            assert!(
                                wanted.iter().all(|row| shown.contains(row)),
                                "{context}: {wanted:?} not in {shown:#?}"
                            );
                        }
                        Target::Input | Target::ToolBox(_) => {
                            assert_eq!(app.panel_drawn(), None, "{context}");
                        }
                    }
                }
            }
        }
        // With a menu open no panel is drawn, at any height.
        app.panels_mut().set_open(&ids);
        app.focus_panel(None);
        crate::app::tests::type_text(&mut app, "/", now);
        assert!(app.input().menu().is_some());
        for rows in 4..=60 {
            app.resize(width, rows);
            let (_, live, _) = app.frame(now);
            let shown = texts(&live).join("\n");
            assert!(!shown.contains("◇ Plan"), "{rows} rows:\n{shown}");
            assert!(!shown.contains("collapsed"), "{rows} rows:\n{shown}");
            let menu = crate::view::menu::menu(app.input(), app.look(), width)
                .map_or(1, |menu| menu.len());
            assert!(live.len() <= (rows / 2).max(3 + menu), "{rows} rows");
        }
    }
}

#[test]
fn the_block_is_set_apart_above_and_below_and_the_live_rows_keep_four() {
    let now = Instant::now() + Duration::from_secs(1);
    let (mut app, ids) = four_panels(now, true);
    app.panels_mut().set_open(&ids);
    app.resize(99, 40);
    let (_, live, _) = app.frame(now);
    let shown = texts(&live);
    // The live round's box and its blank row, the block, a blank row, then
    // the input box and the hint row.
    let first = shown
        .iter()
        .position(|row| row.starts_with("◇ Plan"))
        .expect("the plan");
    assert!(
        shown[0].starts_with('╭'),
        "the round's box first: {shown:#?}"
    );
    assert_eq!(shown[first - 1], "", "{shown:#?}");
    assert!(
        shown[shown.len() - 6].ends_with("tiny collapsed 0"),
        "{shown:#?}"
    );
    assert_eq!(shown[shown.len() - 5], "", "{shown:#?}");
    // Short of room, the panels fold before the transcript's live rows go
    // under four.
    app.resize(99, 24);
    let (_, live, _) = app.frame(now);
    let shown = texts(&live);
    let block = shown
        .iter()
        .position(|row| row.starts_with('+'))
        .expect("the folded row");
    assert!(block >= LIVE_ROWS, "{shown:#?}");
    // Idle, with nothing live above it, a blank row sets it apart from the
    // scrollback.
    let (mut idle, ids) = four_panels(now, false);
    idle.panels_mut().set_open(&ids[..1]);
    idle.resize(99, 30);
    let (_, live, _) = idle.frame(now);
    let shown = texts(&live);
    assert_eq!(shown[0], "", "{shown:#?}");
    assert!(shown[1].starts_with("◇ Plan"), "{shown:#?}");
}

/// The transcript's live rows that panels never take.
const LIVE_ROWS: usize = 4;
