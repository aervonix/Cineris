//! The plan and review panels in the application (step 24): when each
//! opens and closes, what each key sends, the readiness rule they share
//! with the dialogs, the typing rule, and the turn's verdict.

use cortex_protocol::PlanItemId;
use cortex_protocol::event::{
    Actor, CheckOutcome, CheckReport, CheckSource, Notice, PlanUpdated, Verification,
    VerificationNotice, VerificationPhase,
};
use cortex_protocol::item::{Plan, PlanItem, PlanStatus, ToolOutcome, ToolResultItem};
use cortex_protocol::methods::{PlanResult, SessionRef};
use cortex_protocol::project::PlanEdit;
use cortex_testkit::cadence::{AutoRepeat, Cadence, Stroke};

use super::*;
use crate::app::ready::{GRACE, QUIET, REPEAT};
use crate::focus::Target;

/// `n` milliseconds after `now`.
fn ms(now: Instant, n: u64) -> Instant {
    now + Duration::from_millis(n)
}

fn plan_item(id: &str, text: &str, status: PlanStatus) -> PlanItem {
    PlanItem {
        id: PlanItemId::from(id),
        text: text.to_owned(),
        status,
    }
}

/// Screen 4's plan, at `revision`.
fn screen_plan(revision: u64) -> Plan {
    Plan {
        items: vec![
            plan_item("p1", "allow _ between digits", PlanStatus::Done),
            plan_item("p2", "reject a trailing _", PlanStatus::Active),
            plan_item("p3", "add a test for 1_000", PlanStatus::Pending),
            plan_item("p4", "run clippy", PlanStatus::Pending),
        ],
        revision,
    }
}

fn plan_updated(plan: Plan) -> Inbound {
    event(Event::PlanUpdated(PlanUpdated {
        plan,
        by: Actor::Agent,
    }))
}

fn turn_completed(verification: Option<Verification>) -> Inbound {
    event(Event::TurnCompleted(TurnCompleted {
        outcome: TurnOutcome::Completed,
        stop_reason: Some(StopReason::End),
        error: None,
        usage: Default::default(),
        verification,
        duration_ms: Some(2_000),
    }))
}

/// A session open long before `at`, with screen 4's plan given, and a
/// frame drawn.
fn with_plan(at: Instant) -> (App, Scripted) {
    let long_before = at - Duration::from_secs(10);
    let (mut app, link) = app(long_before);
    opened(&mut app, &link, long_before);
    link.give(plan_updated(screen_plan(1)));
    drain(&mut app, long_before);
    let _ = app.frame(long_before);
    app.drawn(long_before);
    link.take_sent();
    (app, link)
}

/// One key read at `at`, and the frame the loop draws for it.
fn press(app: &mut App, binding: &str, at: Instant) {
    key(app, binding, at);
    let _ = app.frame(at);
    app.drawn(at);
}

/// Input as the loop takes it: a key read with more input as one of a
/// burst, and a frame after each read.
fn feed(app: &mut App, strokes: &[Stroke]) -> Instant {
    for stroke in strokes {
        if stroke.queued && matches!(stroke.event, TermEvent::Key(_)) {
            app.burst(stroke.at);
        }
        app.terminal(&stroke.event, stroke.at);
        if !stroke.queued {
            let _ = app.frame(stroke.at);
            app.drawn(stroke.at);
        }
    }
    strokes.last().expect("input to feed").at
}

/// `/plan` typed long before `at` and its `Enter` at `at`, the frame drawn
/// then: the plan has the focus. The time it is ready.
fn plan_focused(app: &mut App, at: Instant) -> Instant {
    type_text(app, "/plan", at - Duration::from_secs(5));
    key(app, "esc", at - Duration::from_secs(5));
    press(app, "enter", at);
    assert_eq!(app.input().focus().focused(), Target::Panel(plan_id(app)));
    at + QUIET
}

fn plan_id(app: &App) -> u64 {
    app.panels().find("plan").expect("the plan panel")
}

fn review_id(app: &App) -> u64 {
    app.panels().find("review").expect("the review panel")
}

/// The plan's edits sent so far, taken.
fn edits(link: &Scripted) -> Vec<PlanEdit> {
    link.take_sent()
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::PlanUpdate(params) => {
                assert_eq!(params.session_id, SessionId::from(SESSION));
                Some(params.edits)
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// The live region of the frame drawn at `at`, as the loop draws it.
fn live_text(app: &mut App, at: Instant) -> Vec<String> {
    let (_, live, _) = app.frame(at);
    app.drawn(at);
    text(&live)
}

#[test]
fn the_plan_opens_by_itself_collapsed_without_the_focus() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, _link) = with_plan(now);
    let live = live_text(&mut app, now);
    let at = live
        .iter()
        .position(|row| row == "◇ Plan  1 of 4 done")
        .unwrap_or_else(|| panic!("{live:#?}"));
    assert_eq!(live[at + 1], "  ◧ reject a trailing _");
    assert_eq!(live[at + 2], "", "a blank row after the block");
    assert!(app.input().focus().on_input());
    assert!(
        live.iter().any(|row| row.contains("Describe a task")),
        "{live:#?}"
    );
}

#[test]
fn the_plan_stays_all_done_while_the_turn_runs_and_goes_when_it_ends() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    let plan = plan_id(&app);
    link.give(event(Event::TurnStarted(TurnStarted {})));
    let mut done = screen_plan(2);
    for item in &mut done.items {
        item.status = PlanStatus::Done;
    }
    // One skipped counts as nothing left to do.
    done.items[3].status = PlanStatus::Skipped;
    link.give(plan_updated(done.clone()));
    drain(&mut app, now);
    assert!(app.panels().is_open(plan), "all done, the turn running");
    link.give(turn_completed(None));
    drain(&mut app, now);
    assert!(!app.panels().is_open(plan), "all done, the turn ended");
    // A plan with something to do opens it again; an empty one closes it.
    link.give(plan_updated(screen_plan(3)));
    drain(&mut app, now);
    assert!(app.panels().is_open(plan));
    link.give(plan_updated(Plan {
        items: Vec::new(),
        revision: 4,
    }));
    drain(&mut app, now);
    assert!(!app.panels().is_open(plan));
    // All done with no turn running: it does not open.
    done.revision = 5;
    link.give(plan_updated(done));
    drain(&mut app, now);
    assert!(!app.panels().is_open(plan));
}

#[test]
fn an_older_plan_never_replaces_a_newer_one() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    let mut newer = screen_plan(5);
    newer.items[0].text = "the newer first step".to_owned();
    link.give(plan_updated(newer));
    link.give(plan_updated(screen_plan(4)));
    drain(&mut app, now);
    plan_focused(&mut app, now);
    let live = live_text(&mut app, now).join("\n");
    assert!(live.contains("the newer first step"), "{live}");
}

#[test]
fn slash_plan_focuses_the_plan_and_with_no_plan_says_the_model_makes_one() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    run_command(&mut app, "/plan", now);
    assert!(app.input().focus().on_input());
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, now, &mut scrollback);
    assert!(
        scrollback.iter().any(
            |row| row.contains("There is no plan yet: the model makes one with its plan tool.")
        ),
        "{scrollback:#?}"
    );
    assert!(
        !link
            .take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::TurnStart(_))),
        "a command is never sent to the model"
    );
    // With a plan: focused, drawn whole, the hint row its keys.
    let (mut app, _link) = with_plan(now);
    plan_focused(&mut app, now);
    let live = live_text(&mut app, now);
    let at = live
        .iter()
        .position(|row| row == "◇ Plan  1 of 4 done")
        .unwrap();
    assert_eq!(
        live[at + 1..at + 5],
        [
            "  ☑ allow _ between digits",
            "  ◧ reject a trailing _",
            "  ☐ add a test for 1_000",
            "  ☐ run clippy",
        ]
    );
    let hint = live.last().unwrap();
    assert!(
        hint.starts_with("  space skips · e edits · a adds · esc back to the input"),
        "{hint}"
    );
}

#[test]
fn each_plan_key_sends_its_edit_to_the_session() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    let id = |id: &str| PlanItemId::from(id);
    // The focus starts on the active item.
    press(&mut app, "space", ready);
    assert_eq!(
        edits(&link),
        [PlanEdit::Skip(cortex_protocol::project::PlanItemRef {
            id: id("p2")
        })]
    );
    press(&mut app, "down", ms(ready, 2_000));
    press(&mut app, "d", ms(ready, 4_000));
    assert_eq!(
        edits(&link),
        [PlanEdit::Remove(cortex_protocol::project::PlanItemRef {
            id: id("p3")
        })]
    );
    press(&mut app, "alt+up", ms(ready, 6_000));
    assert_eq!(
        edits(&link),
        [PlanEdit::Reorder(cortex_protocol::project::PlanReorder {
            order: vec![id("p1"), id("p3"), id("p2"), id("p4")],
        })]
    );
    press(&mut app, "alt+down", ms(ready, 8_000));
    assert_eq!(
        edits(&link),
        [PlanEdit::Reorder(cortex_protocol::project::PlanReorder {
            order: vec![id("p1"), id("p2"), id("p4"), id("p3")],
        })]
    );
    // At the end, a move down has nowhere to go.
    press(&mut app, "down", ms(ready, 9_000));
    press(&mut app, "alt+down", ms(ready, 11_000));
    assert!(edits(&link).is_empty());
}

#[test]
fn rewording_and_adding_use_the_prompts_editor_and_send_no_prompt() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    press(&mut app, "e", ready);
    assert_eq!(app.input().editor().expanded(), "reject a trailing _");
    let live = live_text(&mut app, ready);
    assert!(
        live.last()
            .unwrap()
            .starts_with("  enter saves the item · esc cancels"),
        "{live:#?}"
    );
    type_text(&mut app, " early", ready);
    press(&mut app, "enter", ms(ready, 100));
    assert_eq!(
        edits(&link),
        [PlanEdit::Reword(cortex_protocol::project::PlanReword {
            id: PlanItemId::from("p2"),
            text: "reject a trailing _ early".to_owned(),
        })]
    );
    assert!(
        !link
            .script()
            .sent
            .iter()
            .any(|(_, op)| matches!(op, Op::TurnStart(_))),
        "no prompt"
    );
    // The focus is back on the plan, ready again after its grace.
    assert_eq!(app.input().focus().focused(), Target::Panel(plan_id(&app)));
    let again = ms(ready, 100) + QUIET;
    press(&mut app, "a", again);
    assert_eq!(app.input().editor().expanded(), "");
    type_text(&mut app, "test 1_", again);
    press(&mut app, "enter", ms(again, 50));
    assert_eq!(
        edits(&link),
        [PlanEdit::Add(cortex_protocol::project::PlanAdd {
            text: "test 1_".to_owned(),
            after: Some(PlanItemId::from("p2")),
        })]
    );
    // `Esc` cancels: nothing is sent, and nothing went to the history.
    let later = ms(again, 50) + QUIET;
    press(&mut app, "e", later);
    press(&mut app, "esc", ms(later, 10));
    assert!(edits(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), "");
    app.focus_panel(None);
    press(&mut app, "up", ms(later, 20));
    assert_eq!(
        app.input().editor().expanded(),
        "/plan",
        "the history holds the command, not the items"
    );
}

#[test]
fn the_focus_stays_on_its_item_when_the_plan_changes_under_it() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    let plan = plan_id(&app);
    press(&mut app, "down", ready);
    assert_eq!(app.panels().focus_of(plan), 2, "on p3");
    // The first item goes: p3 is the second now, and keeps the focus, its
    // content as it was, so the panel stays ready.
    let mut fewer = screen_plan(2);
    fewer.items.remove(0);
    link.give(plan_updated(fewer));
    drain(&mut app, ready);
    let _ = live_text(&mut app, ready);
    assert_eq!(app.panels().focus_of(plan), 1);
    press(&mut app, "d", ms(ready, 10));
    assert_eq!(
        edits(&link),
        [PlanEdit::Remove(cortex_protocol::project::PlanItemRef {
            id: PlanItemId::from("p3")
        })]
    );
    // The focused item itself goes: the focus is on the nearest, which the
    // person has not seen there, and the grace starts again.
    let mut gone = screen_plan(3);
    gone.items.retain(|item| item.id.as_str() != "p3");
    link.give(plan_updated(gone));
    drain(&mut app, ms(ready, 20));
    let _ = live_text(&mut app, ms(ready, 20));
    assert_eq!(app.panels().focus_of(plan), 1, "on p2 now");
    // Another key than the last (the same one so soon is that key held).
    press(&mut app, "space", ms(ready, 30));
    assert!(edits(&link).is_empty(), "not ready: typed");
    assert_eq!(app.input().editor().expanded(), " ");
}

#[test]
fn space_on_a_done_or_skipped_item_does_nothing_and_is_not_offered() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    press(&mut app, "up", ready);
    let live = live_text(&mut app, ready);
    let hint = live.last().unwrap();
    assert!(hint.starts_with("  e edits · a adds"), "{hint}");
    press(&mut app, "space", ms(ready, 10));
    assert!(edits(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), "", "not typed either");
    assert_eq!(app.input().focus().focused(), Target::Panel(plan_id(&app)));
}

#[test]
fn an_edits_result_replaces_the_plan_and_one_for_a_session_left_is_dropped() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    press(&mut app, "space", ready);
    let sent = link.take_sent();
    let (ticket, _) = sent
        .iter()
        .find(|(_, op)| matches!(op, Op::PlanUpdate(_)))
        .unwrap();
    let mut skipped = screen_plan(2);
    skipped.items[1].status = PlanStatus::Skipped;
    link.give(Inbound::Done(
        *ticket,
        Ok(OpResult::PlanUpdate(PlanResult {
            plan: skipped.clone(),
        })),
    ));
    drain(&mut app, ready);
    let live = live_text(&mut app, ready).join("\n");
    assert!(live.contains("⊘ reject a trailing _"), "{live}");
    // An error is said with the core's sentence.
    press(&mut app, "d", ms(ready, 2_000));
    let (ticket, _) = link.take_sent().into_iter().next().unwrap();
    link.give(Inbound::Done(
        ticket,
        Err(ErrorData::new(
            ErrorKind::Usage,
            "unknown_item",
            "There is no item p2.",
        )),
    ));
    drain(&mut app, ready);
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, ms(ready, 2_000), &mut scrollback);
    assert!(
        scrollback
            .iter()
            .any(|row| row.contains("There is no item p2.")),
        "{scrollback:#?}"
    );
    // A result for a session left meanwhile is dropped.
    press(&mut app, "d", ms(ready, 4_000));
    let (ticket, _) = link.take_sent().into_iter().next().unwrap();
    app.focus_panel(None);
    run_command(&mut app, "/new", ms(ready, 4_100));
    assert!(
        !app.panels().is_open(plan_id(&app)),
        "left with its session"
    );
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::PlanUpdate(PlanResult { plan: skipped })),
    ));
    drain(&mut app, ms(ready, 4_200));
    assert!(!app.panels().is_open(plan_id(&app)));
}

#[test]
fn a_stored_session_asks_for_its_plan_and_a_new_one_does_not() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut resumed, link) = app_opening(now, None, Open::Resume(SessionId::from(SESSION)));
    let app = &mut resumed;
    let _ = app.frame(now);
    app.drawn(now);
    let sent = link.take_sent();
    let (open, _) = sent
        .iter()
        .find(|(_, op)| matches!(op, Op::SessionOpen(_)))
        .unwrap();
    link.give(Inbound::Done(
        *open,
        Ok(OpResult::SessionOpen(SessionResult {
            session: session().build(),
        })),
    ));
    drain(app, now);
    let sent = link.take_sent();
    let (ask, _) = sent
        .iter()
        .find(|(_, op)| {
            matches!(op, Op::PlanGet(SessionRef { session_id }) if session_id.as_str() == SESSION)
        })
        .unwrap_or_else(|| panic!("plan/get: {sent:#?}"));
    link.give(Inbound::Done(
        *ask,
        Ok(OpResult::PlanGet(PlanResult {
            plan: screen_plan(7),
        })),
    ));
    drain(app, now);
    assert!(app.panels().is_open(plan_id(app)));
    // A new one has no plan to ask for.
    let (mut fresh, link) = crate::testkit::app(now);
    opened(&mut fresh, &link, now);
    assert!(
        !link
            .take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::PlanGet(_)))
    );
}

#[test]
fn typing_that_runs_on_past_slash_plans_enter_never_skips_an_item() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    // `/plan`, its `Enter`, and typing that goes on at a person's pace,
    // spaces and the panel's letters among it.
    let strokes = Cadence::starting(now)
        .typing("/plan", Duration::from_millis(120))
        .key(cortex_term::Key::plain(cortex_term::KeyCode::Esc))
        .wait(Duration::from_millis(120))
        .key(cortex_term::Key::plain(cortex_term::KeyCode::Enter))
        .typing(" add a dep and run it", Duration::from_millis(120))
        .strokes();
    feed(&mut app, &strokes);
    assert!(
        edits(&link).is_empty(),
        "nothing skipped, edited or deleted"
    );
    assert_eq!(app.input().editor().expanded(), " add a dep and run it");
    assert!(
        app.input().focus().on_input(),
        "the focus came back with it"
    );
    // At a slow typist's pace too.
    let (mut app, link) = with_plan(now);
    let enter = plan_focused(&mut app, now);
    let strokes = Cadence::starting(enter - QUIET)
        .typing(" ada", Duration::from_millis(800))
        .strokes();
    feed(&mut app, &strokes);
    assert!(edits(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), " ada");
}

#[test]
fn a_held_key_keys_read_together_and_a_paste_never_act() {
    let now = Instant::now() + Duration::from_secs(20);
    let space = cortex_term::Key::plain(cortex_term::KeyCode::Char(' '));
    let d = cortex_term::Key::plain(cortex_term::KeyCode::Char('d'));
    // A key held down, its repeats marked: one press, one edit.
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    feed(
        &mut app,
        &Cadence::starting(ready)
            .hold(d, AutoRepeat::WINDOWS, 20)
            .strokes(),
    );
    assert_eq!(edits(&link).len(), 1, "the press, not its repeats");
    assert_eq!(app.input().editor().expanded(), "", "nor typed");
    // Unmarked, as X11 sends them: the same key again within the repeat
    // window goes where its press went.
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    feed(
        &mut app,
        &Cadence::starting(ready)
            .hold(d, AutoRepeat::X11, 20)
            .strokes(),
    );
    assert_eq!(edits(&link).len(), 1);
    assert_eq!(app.input().editor().expanded(), "");
    // Released and pressed again after the window: it acts.
    press(&mut app, "d", ms(ready, 660 + 40 * 19) + REPEAT);
    assert_eq!(edits(&link).len(), 1);
    // Keys read together: none acts.
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    feed(&mut app, &Cadence::starting(ready).together(" d").strokes());
    assert!(edits(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), " d");
    // A paste: to the editor, never an edit.
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    app.terminal(&TermEvent::Paste(" d".to_owned()), ready);
    assert!(edits(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), " d");
    // A marked repeat of a key that went nowhere yet is not taken either.
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    app.terminal(&TermEvent::Repeat(space), ready);
    assert!(edits(&link).is_empty());
}

#[test]
fn a_panel_key_the_ready_plan_has_no_use_for_goes_to_the_editor() {
    let now = Instant::now() + Duration::from_secs(20);
    for (n, binding) in ["y", "n", "p", "r", "g", "x"].into_iter().enumerate() {
        let (mut app, link) = with_plan(now);
        let ready = plan_focused(&mut app, now);
        press(&mut app, binding, ready);
        assert!(edits(&link).is_empty(), "{binding}");
        assert_eq!(app.input().editor().expanded(), binding, "{n}: typed");
        assert!(
            app.input().focus().on_input(),
            "{binding}: the focus came back"
        );
    }
    // `tab` types nothing: it does nothing, and the focus stays.
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    press(&mut app, "tab", ready);
    assert!(edits(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), "");
    assert_eq!(app.input().focus().focused(), Target::Panel(plan_id(&app)));
}

#[test]
fn moving_and_esc_act_at_once_and_a_key_before_the_grace_is_typed() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    plan_focused(&mut app, now);
    let plan = plan_id(&app);
    press(&mut app, "down", ms(now, 10));
    assert_eq!(app.panels().focus_of(plan), 2, "moved at once");
    press(&mut app, "esc", ms(now, 20));
    assert!(app.input().focus().on_input(), "esc at once");
    // Back with `Ctrl+P`, which is not typing: ready after the grace alone.
    press(&mut app, "ctrl+p", ms(now, 5_000));
    assert_eq!(app.input().focus().focused(), Target::Panel(plan));
    press(&mut app, "e", ms(now, 5_000) + GRACE / 2);
    assert_eq!(app.input().editor().expanded(), "e", "in its grace: typed");
    assert!(app.input().focus().on_input());
    press(&mut app, "ctrl+p", ms(now, 9_000));
    press(&mut app, "space", ms(now, 9_000) + GRACE);
    assert_eq!(edits(&link).len(), 1, "after its grace: taken");
}

/// The plan focused and ready at the time returned, after `restart` did
/// what starts its grace again at `at`: a key in the grace after it is
/// typed, and one after the grace acts.
fn grace_restarts(restart: impl Fn(&mut App, &Scripted, Instant)) {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    let at = ms(ready, 5_000);
    restart(&mut app, &link, at);
    let _ = app.frame(at);
    app.drawn(at);
    if !matches!(app.input().focus().focused(), Target::Panel(_)) {
        // A dialog took the focus: it comes back with `Ctrl+P`.
        app.focus_panel(Some(plan_id(&app)));
        let _ = app.frame(at);
        app.drawn(at);
    }
    press(&mut app, "space", at + GRACE / 2);
    assert!(edits(&link).is_empty(), "taken in the grace after it");
    assert_eq!(app.input().editor().expanded(), " ");
    app.focus_panel(Some(plan_id(&app)));
    let back = at + GRACE;
    let _ = app.frame(back);
    app.drawn(back);
    // Typing just before holds it a quiet second.
    press(&mut app, "space", back + GRACE);
    assert!(edits(&link).is_empty());
    app.focus_panel(Some(plan_id(&app)));
    let later = back + GRACE + QUIET;
    let _ = app.frame(later);
    app.drawn(later);
    press(&mut app, "space", later + GRACE);
    assert_eq!(edits(&link).len(), 1);
}

#[test]
fn the_grace_starts_again_after_the_viewer() {
    grace_restarts(|app, _, at| {
        press(app, "ctrl+o", at - Duration::from_secs(2));
        assert!(app.viewing());
        key(app, "esc", at);
        assert!(!app.viewing());
    });
}

#[test]
fn the_grace_starts_again_after_resume() {
    grace_restarts(|app, _, at| {
        app.terminal(&TermEvent::Resumed, at);
    });
}

#[test]
fn the_grace_starts_again_after_the_external_editor() {
    grace_restarts(|app, _, _| {
        app.edited(Ok(crate::external::Edited::Unchanged));
    });
}

#[test]
fn the_grace_starts_again_after_a_dialog() {
    grace_restarts(|app, link, at| {
        ask(app, link, 9, approval("c_9"), at - Duration::from_secs(3));
        assert!(app.input().focus().on_input(), "a dialog takes the focus");
        let _ = app.frame(at - Duration::from_secs(3));
        let (_, live, _) = app.frame(at - Duration::from_secs(3));
        assert!(
            !text(&live).iter().any(|row| row.starts_with("◇ Plan")),
            "no panel while a dialog is up"
        );
        link.give(event(Event::RequestResolved(RequestResolved {
            request_id: RequestId::Number(9),
            resolution: Resolution::Answered,
        })));
        drain(app, at);
    });
}

#[test]
fn the_grace_starts_again_when_the_focused_row_changes() {
    grace_restarts(|app, link, at| {
        let mut changed = screen_plan(9);
        changed.items[1].text = "reject a trailing _ and a leading one".to_owned();
        link.give(plan_updated(changed));
        drain(app, at);
    });
}

#[test]
fn the_grace_starts_again_with_a_new_key_map() {
    grace_restarts(|app, _, _| {
        app.set_keymap(crate::keys::KeyMap::defaults());
    });
}

/// A dialog that comes while a panel has the focus takes it at once, not
/// at its first frame: a key read in between is typing for the hidden
/// editor, as any key in a dialog's grace, never a key of the panel's.
#[test]
fn a_key_between_a_dialog_and_its_frame_is_typed_not_lost() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_plan(now);
    let ready = plan_focused(&mut app, now);
    link.give(Inbound::Request(RequestId::Number(4), approval("c_4")));
    drain(&mut app, ready);
    assert!(app.input().focus().on_input(), "before any frame");
    key(&mut app, "d", ms(ready, 5));
    assert!(edits(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), "d", "typed, not lost");
}

#[test]
fn a_letter_with_a_box_or_a_panel_focused_reaches_the_editor() {
    let now = Instant::now() + Duration::from_secs(20);
    // A live box: a call still running.
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    for inbound in [
        event(Event::TurnStarted(TurnStarted {})),
        event(Event::ItemStarted(ItemStarted {
            item_id: ItemId::from("m1"),
            kind: ItemKind::AssistantMessage,
            tool: None,
            call_id: None,
        })),
        event(Event::ItemDelta(ItemDelta {
            item_id: ItemId::from("m1"),
            delta: Delta::ToolCall(cortex_protocol::event::ToolCallDelta {
                call_id: CallId::from("c1"),
                name: "shell".to_owned(),
            }),
        })),
    ] {
        link.give(inbound);
    }
    drain(&mut app, now);
    let _ = app.frame(now);
    press(&mut app, "ctrl+p", now);
    assert!(matches!(app.input().focus().focused(), Target::ToolBox(_)));
    // A letter no key of the focus's, then one of a panel's own.
    feed(
        &mut app,
        &Cadence::starting(now)
            .typing("ok, yes", Duration::from_millis(60))
            .strokes(),
    );
    assert_eq!(app.input().editor().expanded(), "ok, yes");
    assert!(app.input().focus().on_input());
    // A panel: its keys and others alike, none lost or reordered.
    let (mut app, link) = with_plan(now);
    plan_focused(&mut app, now);
    feed(
        &mut app,
        &Cadence::starting(now)
            .typing("dear ed, a quick note", Duration::from_millis(60))
            .strokes(),
    );
    assert_eq!(app.input().editor().expanded(), "dear ed, a quick note");
    assert!(edits(&link).is_empty());
}

// ---------------------------------------------------------- the review

fn edit_result(call: &str, path: &str, added: u64, removed: u64) -> Inbound {
    event(Event::ItemCompleted(ItemCompleted {
        item: Item::ToolResult(ToolResultItem {
            id: ItemId::from(format!("r_{call}").as_str()),
            call_id: CallId::from(call),
            tool: "edit".to_owned(),
            outcome: ToolOutcome::Success,
            content: Vec::new(),
            payload: Some(serde_json::json!({
                "path": path, "kind": "edit", "added": added, "removed": removed
            })),
            artifacts: Vec::new(),
            duration_ms: Some(10),
            created_at: None,
        }),
    }))
}

/// An edit that failed: it changed nothing.
fn failed_edit(call: &str, path: &str) -> Inbound {
    event(Event::ItemCompleted(ItemCompleted {
        item: Item::ToolResult(ToolResultItem {
            id: ItemId::from(format!("r_{call}").as_str()),
            call_id: CallId::from(call),
            tool: "edit".to_owned(),
            outcome: ToolOutcome::Error,
            content: vec![Part::text("The text to replace was not found.")],
            payload: Some(serde_json::json!({"path": path, "added": 9, "removed": 9})),
            artifacts: Vec::new(),
            duration_ms: Some(10),
            created_at: None,
        }),
    }))
}

fn shell_result(call: &str) -> Inbound {
    event(Event::ItemCompleted(ItemCompleted {
        item: Item::ToolResult(ToolResultItem {
            id: ItemId::from(format!("r_{call}").as_str()),
            call_id: CallId::from(call),
            tool: "shell".to_owned(),
            outcome: ToolOutcome::Success,
            content: vec![Part::text("ok")],
            payload: Some(serde_json::json!({"exit_code": 0})),
            artifacts: Vec::new(),
            duration_ms: Some(10),
            created_at: None,
        }),
    }))
}

/// A response asking for `calls`, each `(id, tool)`, completed: its round.
fn asked(response: &str, calls: &[(&str, &str)]) -> Inbound {
    event(Event::ItemCompleted(ItemCompleted {
        item: Item::AssistantMessage(AssistantMessageItem {
            id: ItemId::from(response),
            parts: calls
                .iter()
                .map(|(id, tool)| {
                    Part::ToolCall(cortex_protocol::message::ToolCallPart {
                        id: CallId::from(*id),
                        name: (*tool).to_owned(),
                        arguments: serde_json::json!({"path": "x", "command": "make"}),
                        raw: None,
                    })
                })
                .collect(),
            stop_reason: Some(StopReason::ToolCalls),
            raw_stop: None,
            stop_detail: None,
            truncated: false,
            origin: None,
            usage: None,
            error: None,
            created_at: None,
        }),
    }))
}

fn checks(phase: VerificationPhase, verdict: Option<Verification>) -> Inbound {
    let check = |command: &str, outcome| CheckReport {
        command: command.to_owned(),
        cwd: None,
        source: CheckSource::User,
        entry: None,
        outcome,
        exit_code: None,
        duration_ms: None,
        note: None,
        artifact: None,
    };
    let finished = phase == VerificationPhase::Finished;
    event(Event::Notice(Notice::Verification(VerificationNotice {
        message: "The checks.".to_owned(),
        phase,
        round: 1,
        rounds: 2,
        checks: vec![
            check("cargo test", finished.then_some(CheckOutcome::Passed)),
            check("cargo clippy", finished.then_some(CheckOutcome::Failed)),
        ],
        verdict,
    })))
}

/// A session with a turn that edited two files in two rounds and ran a
/// command, given and taken; the turn still running.
fn turn_with_changes(now: Instant) -> (App, Scripted) {
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    for inbound in [
        event(Event::TurnStarted(TurnStarted {})),
        asked("m1", &[("c1", "edit")]),
        edit_result("c1", "src/lex.rs", 5, 1),
        asked("m2", &[("c2", "edit"), ("c3", "shell")]),
        edit_result("c2", "tests/lex.rs", 1, 0),
        shell_result("c3"),
        asked("m3", &[("c4", "edit"), ("c5", "edit")]),
        edit_result("c4", "src/lex.rs", 2, 2),
        failed_edit("c5", "src/broken.rs"),
    ] {
        link.give(inbound);
    }
    drain(&mut app, now);
    (app, link)
}

#[test]
fn the_review_opens_when_the_turn_ends_having_changed_files_and_goes_with_the_next_prompt() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = turn_with_changes(now);
    let review = review_id(&app);
    assert!(!app.panels().is_open(review), "not while the turn runs");
    link.give(turn_completed(None));
    drain(&mut app, now);
    assert!(app.panels().is_open(review));
    assert!(app.input().focus().on_input(), "it never takes the focus");
    let live = live_text(&mut app, now);
    assert!(
        live.iter().any(|row| row == "◇ Changed 2 files  +8 −3"),
        "{live:#?}"
    );
    // Focused: one file a row, then that commands ran.
    press(&mut app, "ctrl+p", now);
    let live = live_text(&mut app, now);
    let at = live
        .iter()
        .position(|row| row.starts_with("◇ Changed"))
        .unwrap();
    assert_eq!(
        live[at + 1..at + 4],
        [
            "  src/lex.rs  +7 −3",
            "  tests/lex.rs  +1 −0",
            "  commands ran too: /view shows them",
        ]
    );
    assert!(
        live.last()
            .unwrap()
            .starts_with("  enter opens it in the viewer · x closes · esc back to the input"),
        "{live:#?}"
    );
    // The next prompt: it goes.
    app.focus_panel(None);
    type_text(&mut app, "next", now);
    key(&mut app, "enter", now);
    assert!(!app.panels().is_open(review));
}

#[test]
fn a_turn_that_changed_nothing_opens_no_review_and_a_new_turn_closes_one() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let review = review_id(&app);
    for inbound in [
        event(Event::TurnStarted(TurnStarted {})),
        asked("m1", &[("c1", "shell")]),
        shell_result("c1"),
        turn_completed(Some(Verification::Skipped)),
    ] {
        link.give(inbound);
    }
    drain(&mut app, now);
    assert!(!app.panels().is_open(review));
    let (mut app, link) = turn_with_changes(now);
    link.give(turn_completed(None));
    drain(&mut app, now);
    assert!(app.panels().is_open(review));
    // Another client's turn, or a queued prompt's.
    link.give(event(Event::TurnStarted(TurnStarted {})));
    drain(&mut app, now);
    assert!(!app.panels().is_open(review));
}

#[test]
fn the_review_shows_the_checks_as_they_come() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = turn_with_changes(now);
    let review = review_id(&app);
    link.give(checks(VerificationPhase::Started, None));
    drain(&mut app, now);
    assert!(app.panels().is_open(review), "the checks run: it opens");
    let live = live_text(&mut app, now);
    assert!(
        live.iter()
            .any(|row| row == "◇ Changed 2 files  +8 −3 · checks running · round 1 of 2"),
        "{live:#?}"
    );
    link.give(checks(
        VerificationPhase::Finished,
        Some(Verification::Failed),
    ));
    link.give(turn_completed(Some(Verification::Failed)));
    drain(&mut app, now);
    press(&mut app, "ctrl+p", now);
    let live = live_text(&mut app, now);
    let all = live.join("\n");
    assert!(
        all.contains("◇ Changed 2 files  +8 −3 · checks failed"),
        "{all}"
    );
    assert!(all.contains("  ✓ cargo test"), "{all}");
    assert!(all.contains("  ✗ cargo clippy"), "{all}");
    assert!(all.contains("  checks failed · round 1 of 2"), "{all}");
    // Offered, with `verify.mode = "ask"`.
    let (mut app, link) = turn_with_changes(now);
    link.give(checks(VerificationPhase::Offered, None));
    drain(&mut app, now);
    let live = live_text(&mut app, now).join("\n");
    assert!(
        live.contains("◇ Changed 2 files  +8 −3 · checks not run: /verify runs them"),
        "{live}"
    );
}

#[test]
fn x_closes_the_review_and_enter_opens_the_viewer_at_the_files_box() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = turn_with_changes(now);
    link.give(turn_completed(None));
    drain(&mut app, now);
    let review = review_id(&app);
    let _ = app.frame(now);
    // `src/lex.rs` was last changed in the third round.
    press(&mut app, "ctrl+p", now);
    press(&mut app, "enter", ms(now, 10));
    assert!(app.viewing(), "enter acts at once");
    let lines = app.viewer_frame(40, 99, ms(now, 10));
    let keys = text(&lines).join("\n");
    assert!(keys.contains("box 3 of 3"), "{keys}");
    key(&mut app, "esc", ms(now, 20));
    press(&mut app, "down", ms(now, 30));
    press(&mut app, "enter", ms(now, 40));
    let lines = app.viewer_frame(40, 99, ms(now, 40));
    assert!(text(&lines).join("\n").contains("box 2 of 3"));
    key(&mut app, "esc", ms(now, 50));
    // `x` closes it, at once, and the focus goes back to the input.
    press(&mut app, "x", ms(now, 60));
    assert!(!app.panels().is_open(review));
    assert!(app.input().focus().on_input());
    assert_eq!(app.input().editor().expanded(), "", "not typed");
}

#[test]
fn the_turn_summary_says_the_checks_verdict() {
    let now = Instant::now() + Duration::from_secs(20);
    let link = Scripted::default();
    let start = Start {
        version: "0.1.0".to_owned(),
        folder: "~/src/app".to_owned(),
        workspace: WORKSPACE.to_owned(),
        open: Open::New {
            workspace: WORKSPACE.to_owned(),
            settings: SessionSettings::default(),
            ephemeral: false,
        },
        prompt: None,
        settings: Settings {
            turn_summary: true,
            ..Settings::default()
        },
        keymap: crate::keys::KeyMap::defaults(),
        notices: Vec::new(),
        history: Box::new(crate::history::MemoryHistory::new()),
        highlighter: crate::highlight::Highlighter::off(),
    };
    let mut app = App::new(Box::new(link.clone()), start, &caps(), 99, 40, now);
    opened(&mut app, &link, now);
    link.give(event(Event::TurnStarted(TurnStarted {})));
    link.give(turn_completed(Some(Verification::Passed)));
    link.give(event(Event::TurnStarted(TurnStarted {})));
    link.give(turn_completed(Some(Verification::Skipped)));
    drain(&mut app, now);
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, now, &mut scrollback);
    let summaries: Vec<&String> = scrollback
        .iter()
        .filter(|row| row.starts_with("  2s"))
        .collect();
    assert_eq!(summaries.len(), 2, "{scrollback:#?}");
    assert!(summaries[0].ends_with("· checks passed"), "{summaries:?}");
    assert!(!summaries[1].contains("checks"), "skipped says nothing");
}

#[test]
fn hostile_plan_items_and_paths_are_drawn_visible_in_the_frame() {
    let now = Instant::now() + Duration::from_secs(20);
    let hostile = "\u{1b}[2J\u{202e}gone\u{200b}\u{9b}31m\u{e0041}";
    let (mut app, link) = with_plan(now);
    let mut plan = screen_plan(3);
    for item in &mut plan.items {
        item.text = format!("{hostile}{}", item.text);
    }
    link.give(plan_updated(plan));
    for inbound in [
        event(Event::TurnStarted(TurnStarted {})),
        asked("m1", &[("c1", "edit")]),
        edit_result("c1", hostile, 1, 1),
        turn_completed(None),
    ] {
        link.give(inbound);
    }
    drain(&mut app, now);
    for focus in [None, Some(plan_id(&app)), Some(review_id(&app))] {
        app.focus_panel(focus);
        for columns in [20, 60, 120] {
            app.resize(columns - 1, 40);
            let (_, live, _) = app.frame(now);
            let problems = crate::testing::problems(&live, columns - 1);
            assert!(problems.is_empty(), "{focus:?} at {columns}: {problems:#?}");
        }
    }
}
