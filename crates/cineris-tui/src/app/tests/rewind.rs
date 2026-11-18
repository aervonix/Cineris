//! `/rewind`, its question, fork, and the transcript read again (since step
//! 25; `docs/steps/25/answered.md`, point 4): the whole of it through the
//! scripted link, which is what `cineris attach`'s link is held to.

use cineris_protocol::item::Delivery as ItemDelivery;
use cineris_protocol::methods::{
    RewindFile, RewindFileStatus, RewindTarget, RewindWhyNot, SessionListResult, SessionReadResult,
    SessionRewindParams, SessionRewindPlanResult,
};

use super::*;
use crate::app::Presenter;
use crate::testing::{TRICKY, problems};

fn ms(now: Instant, n: u64) -> Instant {
    now + Duration::from_millis(n)
}

fn user(id: &str, text: &str) -> Item {
    Item::UserMessage(UserMessageItem {
        id: ItemId::from(id),
        parts: vec![Part::text(text)],
        delivery: None,
        created_at: Some(Timestamp::from("2026-11-15T14:32:10Z")),
    })
}

fn answer(id: &str, text: &str) -> Item {
    Item::AssistantMessage(AssistantMessageItem {
        id: ItemId::from(id),
        parts: vec![Part::text(text)],
        stop_reason: Some(StopReason::End),
        raw_stop: None,
        stop_detail: None,
        truncated: false,
        origin: None,
        usage: None,
        error: None,
        created_at: None,
    })
}

/// Three turns: `first prompt`, `fix the lexer` (two lines), `third
/// prompt`, each answered.
fn history() -> Vec<Item> {
    vec![
        user("i_1", "first prompt"),
        answer("i_2", "First answer."),
        user("i_3", "fix the lexer\nso digit separators work"),
        answer("i_4", "Second answer."),
        user("i_5", "third prompt"),
        answer("i_6", "Third answer."),
    ]
}

fn page(items: Vec<Item>, next: Option<&str>) -> OpResult {
    OpResult::SessionRead(SessionReadResult {
        items,
        incomplete: Vec::new(),
        next_cursor: next.map(str::to_owned),
        seq: 9,
        masked: 0,
        redacted: false,
    })
}

/// The plan of a rewind with `files`.
fn plan(files: Vec<RewindFile>) -> SessionRewindPlanResult {
    SessionRewindPlanResult {
        turns: vec![TurnId::from("t_2"), TurnId::from("t_3")],
        files,
        unreadable: false,
        mark: "aaaa-bbbb".to_owned(),
    }
}

fn file(path: &str, to: RewindTarget, status: RewindFileStatus) -> RewindFile {
    RewindFile {
        path: path.to_owned(),
        to,
        status,
        why_not: None,
    }
}

/// Two files: one to go back, one to be removed.
fn two_files() -> Vec<RewindFile> {
    vec![
        file(
            "src/lexer.rs",
            RewindTarget::Content,
            RewindFileStatus::AsLeft,
        ),
        file(
            "src/new.rs",
            RewindTarget::Removed,
            RewindFileStatus::AsLeft,
        ),
    ]
}

/// The ticket of the one operation sent that `f` picks, and that
/// operation; every other sent is dropped.
fn sent_one<T>(link: &Scripted, f: impl Fn(&Op) -> Option<T>) -> (Ticket, T) {
    let sent = link.take_sent();
    sent.iter()
        .find_map(|(ticket, op)| f(op).map(|found| (*ticket, found)))
        .unwrap_or_else(|| panic!("not sent: {sent:#?}"))
}

fn give(link: &Scripted, ticket: Ticket, result: Result<OpResult, ErrorData>) {
    link.give(Inbound::Done(ticket, result));
}

/// An application with a session open whose history the events drew.
fn with_history(now: Instant) -> (App, Scripted) {
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    for item in history() {
        link.give(event(Event::ItemCompleted(ItemCompleted { item })));
    }
    drain(&mut app, now);
    let _ = app.frame(now);
    let _ = link.take_sent();
    (app, link)
}

/// `/rewind`, its read answered with `items` in one page: the picker is
/// open.
fn rewind_picker(app: &mut App, link: &Scripted, items: Vec<Item>, at: Instant) {
    run_command(app, "/rewind", at);
    let (ticket, read) = sent_one(link, |op| match op {
        Op::SessionRead(read) => Some(read.clone()),
        _ => None,
    });
    assert_eq!(read.item_id, None, "the branch, not one item");
    give(link, ticket, Ok(page(items, None)));
    drain(app, at);
}

/// The picker's rows, as labels.
fn rows(app: &App) -> Vec<String> {
    app.input()
        .menu()
        .map(|menu| menu.rows.iter().map(|row| row.label.clone()).collect())
        .unwrap_or_default()
}

/// Turn 2 chosen in the picker (its second row, newest first), its prompt
/// read and its plan answered with `files`; the question drawn until it is
/// ready. Returns the time it takes keys.
fn asked(app: &mut App, link: &Scripted, files: Vec<RewindFile>, at: Instant) -> Instant {
    key(app, "down", at);
    key(app, "enter", at);
    let sent = link.take_sent();
    let prompt = sent
        .iter()
        .find(|(_, op)| matches!(op, Op::SessionRead(read) if read.item_id.is_some()))
        .map(|(ticket, _)| *ticket)
        .expect("the prompt read whole");
    let (planned, point) = sent
        .iter()
        .find_map(|(ticket, op)| match op {
            Op::SessionRewindPlan(params) => {
                Some((*ticket, (params.to.clone(), params.before.clone())))
            }
            _ => None,
        })
        .expect("the plan asked for");
    assert_eq!(
        point,
        (None, Some(ItemId::from("i_3"))),
        "just before turn 2's prompt"
    );
    give(link, prompt, Ok(page(vec![history()[2].clone()], None)));
    give(link, planned, Ok(OpResult::SessionRewindPlan(plan(files))));
    drain(app, at);
    let _ = app.frame(at);
    app.drawn(at);
    let ready = app.deadline().expect("the answers' moment");
    let _ = app.frame(ready);
    app.drawn(ready);
    ready
}

/// The `session/rewind` sent, if one was.
fn rewind_sent(link: &Scripted) -> Option<(Ticket, SessionRewindParams)> {
    link.take_sent()
        .into_iter()
        .find_map(|(ticket, op)| match op {
            Op::SessionRewind(params) => Some((ticket, params)),
            _ => None,
        })
}

// ------------------------------------------------------------ the picker

#[test]
fn rewind_lists_the_earlier_prompts_newest_first_and_not_the_first() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    app.set_zone(cineris_base::Zone::utc());
    rewind_picker(&mut app, &link, history(), now);
    assert_eq!(
        rows(&app),
        [
            "turn 3  14:32  third prompt",
            "turn 2  14:32  fix the lexer so digit separators work",
        ]
    );
    let (_, live, _) = app.frame(now);
    let shown = text(&live).join("\n");
    assert!(shown.contains("tab - fork from here"), "{shown}");
}

/// A long history is read page by page, and its points are counted along
/// it; a steering message begins no turn.
#[test]
fn the_points_are_read_page_by_page_and_steering_is_no_point() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    run_command(&mut app, "/rewind", now);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionRead(_)).then_some(()));
    let mut items = history();
    let later = items.split_off(3);
    let steer = UserMessageItem {
        id: ItemId::from("i_2s"),
        parts: vec![Part::text("and keep it short")],
        delivery: Some(ItemDelivery::Steer),
        created_at: None,
    };
    items.insert(2, Item::UserMessage(steer));
    give(&link, ticket, Ok(page(items, Some("c_2"))));
    drain(&mut app, now);
    let (ticket, read) = sent_one(&link, |op| match op {
        Op::SessionRead(read) => Some(read.clone()),
        _ => None,
    });
    assert_eq!(read.cursor.as_deref(), Some("c_2"));
    give(&link, ticket, Ok(page(later, None)));
    drain(&mut app, now);
    let rows = rows(&app);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows[1].starts_with("turn 2 "), "{rows:?}");
    assert!(rows[0].starts_with("turn 3 "), "{rows:?}");
}

#[test]
fn rewind_while_a_turn_runs_says_to_wait_and_reads_nothing() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    link.give(event(Event::TurnStarted(TurnStarted {})));
    drain(&mut app, now);
    run_command(&mut app, "/rewind", now);
    assert!(
        !link
            .take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionRead(_))),
        "read while a turn runs"
    );
    assert!(notices(&mut app, now).contains("wait for it to end, or stop it"));
}

/// Going back before the first prompt is `/new`, whatever the harness
/// put before it.
#[test]
fn a_session_with_only_one_prompt_has_no_point() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    let mut items = vec![Item::Notice(cineris_protocol::item::NoticeItem {
        id: ItemId::from("i_0"),
        source: cineris_protocol::item::NoticeSource::Map,
        text: "The repository's map.".to_owned(),
        created_at: None,
    })];
    items.extend(history()[..2].to_vec());
    rewind_picker(&mut app, &link, items, now);
    assert!(app.input().menu().is_none());
    assert!(notices(&mut app, now).contains("no earlier prompt to go back to"));
}

// ---------------------------------------------------------- the question

#[test]
fn one_rewinds_and_puts_the_files_back_by_the_plan_shown() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    let (_, live, _) = app.frame(ready);
    let shown = text(&live).join("\n");
    for words in [
        "Rewind to before turn 2?",
        // The prompt's first line, and that there is more.
        "fix the lexer…",
        "The plan undoes 2 turns. They changed these files:",
        "src/lexer.rs  goes back to what it was before",
        "src/new.rs  is removed: it did not exist then",
        "1 Rewind it and put back the files listed",
        "2 Rewind the conversation only",
    ] {
        assert!(shown.contains(words), "{words:?} is not shown:\n{shown}");
    }
    key(&mut app, "1", ready);
    let (_, params) = rewind_sent(&link).expect("the rewind");
    assert_eq!(
        (params.to, params.before),
        (None, Some(ItemId::from("i_3"))),
        "just before turn 2's prompt"
    );
    assert!(params.restore_files);
    assert_eq!(params.plan.as_deref(), Some("aaaa-bbbb"));
}

#[test]
fn two_rewinds_the_conversation_only() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    key(&mut app, "2", ready);
    let (_, params) = rewind_sent(&link).expect("the rewind");
    assert!(!params.restore_files);
    assert_eq!(params.plan.as_deref(), Some("aaaa-bbbb"));
}

#[test]
fn esc_cancels_and_sends_nothing() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    key(&mut app, "esc", ready);
    assert_eq!(rewind_sent(&link), None);
    let (_, live, _) = app.frame(ms(ready, 10));
    assert!(
        !text(&live).join("\n").contains("Rewind to before"),
        "the question stayed"
    );
    // A later `/rewind` is not held up by the one cancelled.
    run_command(&mut app, "/rewind", ms(ready, 20));
    assert!(
        link.take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionRead(_)))
    );
}

/// No answer is preselected: `Enter` chooses nothing, and the question
/// stays.
#[test]
fn enter_chooses_nothing() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    key(&mut app, "enter", ready);
    assert_eq!(rewind_sent(&link), None);
    assert!(turn_start(&link.take_sent()).is_empty(), "sent as a prompt");
    key(&mut app, "2", ms(ready, 1));
    assert!(rewind_sent(&link).is_some(), "the question went away");
}

/// The dialogs' rule of keys: an answer typed before the question is
/// ready answers nothing.
#[test]
fn a_digit_before_the_question_is_ready_answers_nothing() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    key(&mut app, "down", now);
    key(&mut app, "enter", now);
    for (ticket, op) in link.take_sent() {
        match op {
            Op::SessionRewindPlan(_) => give(
                &link,
                ticket,
                Ok(OpResult::SessionRewindPlan(plan(two_files()))),
            ),
            Op::SessionRead(_) => give(&link, ticket, Ok(page(vec![history()[2].clone()], None))),
            _ => {}
        }
    }
    let shown = ms(now, 5000);
    drain(&mut app, shown);
    let _ = app.frame(shown);
    app.drawn(shown);
    key(&mut app, "1", ms(shown, 50));
    assert_eq!(rewind_sent(&link), None, "answered in its grace");
    assert_eq!(
        app.input().editor().expanded(),
        "",
        "typed into the editor under the question"
    );
    key(&mut app, "esc", ms(shown, 60));
    let (_, live, _) = app.frame(ms(shown, 70));
    assert!(
        text(&live).join("\n").contains("Rewind to before turn 2?"),
        "esc in its grace closed the question"
    );
}

/// The question says the turns the core's plan undoes, which its mark
/// holds: a turn run since the points were read is counted (the review of
/// this part).
#[test]
fn the_question_counts_the_turns_of_the_plan_not_of_the_picker() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    key(&mut app, "down", now);
    key(&mut app, "enter", now);
    let mut moved = plan(two_files());
    moved.turns.push(TurnId::from("t_4"));
    for (ticket, op) in link.take_sent() {
        match op {
            Op::SessionRewindPlan(_) => {
                give(
                    &link,
                    ticket,
                    Ok(OpResult::SessionRewindPlan(moved.clone())),
                );
            }
            Op::SessionRead(_) => give(&link, ticket, Ok(page(vec![history()[2].clone()], None))),
            _ => {}
        }
    }
    drain(&mut app, now);
    let (_, live, _) = app.frame(now);
    let shown = text(&live).join("\n");
    assert!(shown.contains("The plan undoes 3 turns."), "{shown}");
}

/// A turn that ends under the question (another client's) closes it:
/// nothing is rewound, it is said, and `/rewind` asks afresh.
#[test]
fn a_turn_ending_under_the_question_closes_it_and_rewinds_nothing() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    link.give(event(Event::TurnStarted(TurnStarted {})));
    link.give(event(Event::TurnCompleted(TurnCompleted {
        outcome: TurnOutcome::Completed,
        stop_reason: Some(StopReason::End),
        error: None,
        usage: Default::default(),
        verification: None,
        duration_ms: Some(1200),
    })));
    drain(&mut app, ready);
    let mut scrollback = Vec::new();
    let live = screen(&mut app, ms(ready, 10), &mut scrollback).join("\n");
    assert!(!live.contains("Rewind to before"), "the question stayed");
    assert_eq!(rewind_sent(&link), None, "rewound after the turn");
    let words = [scrollback.join(" "), live]
        .concat()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(words.contains("so nothing was rewound"), "{words}");
    run_command(&mut app, "/rewind", ms(ready, 30));
    assert!(
        link.take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionRead(_))),
        "the rewind closed from outside held /rewind up"
    );
}

/// While the transcript is read again, `/new`, a switch, a second `/undo`
/// and `/rewind` wait and say so: a prompt held for the read is this
/// session's.
#[test]
fn new_a_switch_and_another_undo_wait_for_the_read_again() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    run_command(&mut app, "/undo", now);
    // A second undo while the first is on its way.
    run_command(&mut app, "/undo", now);
    let sent = link.take_sent();
    let undos: Vec<Ticket> = sent
        .iter()
        .filter(|(_, op)| matches!(op, Op::SessionUndo(_)))
        .map(|(ticket, _)| *ticket)
        .collect();
    assert_eq!(undos.len(), 1, "{sent:#?}");
    give(
        &link,
        undos[0],
        Ok(OpResult::SessionUndo(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, now);
    let _ = link.take_sent();
    run_command(&mut app, "/new", now);
    run_command(&mut app, "/rewind", now);
    let sent = link.take_sent();
    assert!(
        !sent.iter().any(|(_, op)| matches!(
            op,
            Op::SessionNew(_) | Op::SessionClose(_) | Op::SessionRead(_)
        )),
        "{sent:#?}"
    );
    let all = notices(&mut app, now);
    assert!(
        all.contains("/new can start another session once it is done"),
        "{all}"
    );
    assert!(
        all.contains("/rewind can go back again once it is done"),
        "{all}"
    );
}

/// An answer of another kind than the plan ends the rewind as an error
/// does: nothing waits for it after (the review of this part).
#[test]
fn an_answer_of_another_kind_ends_the_rewind() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    key(&mut app, "enter", now);
    let (ticket, _) = sent_one(&link, |op| {
        matches!(op, Op::SessionRewindPlan(_)).then_some(())
    });
    give(
        &link,
        ticket,
        Ok(OpResult::SessionClose(cineris_protocol::methods::Empty {})),
    );
    drain(&mut app, now);
    run_command(&mut app, "/rewind", now);
    assert!(
        link.take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionRead(read) if read.item_id.is_none())),
        "the rewind still waited"
    );
}

/// `1` puts files back, so it is offered only for a plan seen whole: in a
/// window too small for the files, until the viewer has shown every row.
#[test]
fn one_is_refused_until_every_file_of_the_plan_was_drawn_whole() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let many: Vec<RewindFile> = (0..80)
        .map(|n| {
            file(
                &format!("src/file_{n}.rs"),
                RewindTarget::Content,
                RewindFileStatus::AsLeft,
            )
        })
        .collect();
    let ready = asked(&mut app, &link, many, now);
    let (_, live, _) = app.frame(ready);
    let shown = text(&live).join("\n");
    assert!(shown.contains("more lines are not shown"), "{shown}");
    assert!(!shown.contains("1 Rewind it"), "{shown}");
    assert!(shown.contains("2 Rewind the conversation only"), "{shown}");
    key(&mut app, "1", ready);
    assert_eq!(rewind_sent(&link), None, "files put back unseen");

    // A part of the plan in the viewer is not the whole of it.
    let at = ms(ready, 2000);
    key(&mut app, "ctrl+o", at);
    assert!(app.viewing());
    let first = text(&app.viewer_frame(40, 98, at)).join("\n");
    assert!(first.contains("src/file_0.rs"), "{first}");
    key(&mut app, "esc", ms(at, 1));
    let back = ms(at, 2);
    let _ = app.frame(back);
    app.drawn(back);
    let ready = app.deadline().expect("ready again");
    let (_, live, _) = app.frame(ready);
    app.drawn(ready);
    assert!(
        !text(&live).join("\n").contains("1 Rewind it"),
        "offered after a part was seen"
    );

    // The plan whole in the viewer, every row of it drawn.
    let at = ms(ready, 2000);
    key(&mut app, "ctrl+o", at);
    assert!(app.viewing());
    let _ = app.viewer_frame(40, 98, at);
    for step in 0..6 {
        key(&mut app, "pagedown", ms(at, step));
        let _ = app.viewer_frame(40, 98, ms(at, step));
    }
    key(&mut app, "esc", ms(at, 10));
    assert!(!app.viewing());
    let back = ms(at, 20);
    let _ = app.frame(back);
    app.drawn(back);
    let ready = app.deadline().expect("ready again");
    let (_, live, _) = app.frame(ready);
    app.drawn(ready);
    assert!(
        text(&live).join("\n").contains("1 Rewind it"),
        "not offered once seen whole"
    );
    key(&mut app, "1", ready);
    let (_, params) = rewind_sent(&link).expect("the rewind");
    assert!(params.restore_files);
}

/// A plan that moved between the question and the answer is the core's to
/// refuse: the client sends what was chosen, with the plan's mark, and
/// says the refusal; nothing goes back into the editor.
#[test]
fn a_plan_the_core_refuses_as_moved_is_said() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    key(&mut app, "1", ready);
    let (ticket, params) = rewind_sent(&link).expect("the rewind");
    assert!(params.restore_files);
    assert_eq!(params.plan.as_deref(), Some("aaaa-bbbb"));
    give(
        &link,
        ticket,
        Err(ErrorData::new(
            ErrorKind::Usage,
            cineris_protocol::error::code::PLAN_CHANGED,
            "The rewind's plan changed after it was shown.",
        )),
    );
    drain(&mut app, ready);
    let all = notices(&mut app, ready);
    assert!(all.contains("plan changed after it was shown"), "{all}");
    assert!(all.contains("/rewind"), "no way on is said: {all}");
    assert_eq!(app.input().editor().expanded(), "");
    assert!(
        !link
            .take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionRead(_))),
        "a refused rewind read the transcript again"
    );
}

// ------------------------------------------------------------- after it

/// The prompt goes back into the editor after the draft; inline a notice
/// says the scrollback above it no longer counts; the viewer draws the
/// branch read again, the turns undone gone.
#[test]
fn after_a_rewind_the_prompt_comes_back_and_the_viewer_draws_the_branch_again() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    // A draft written while the question waited (typing at a dialog goes
    // to the editor), then a pause, then the answer.
    type_text(&mut app, "draft", ready);
    let later = ms(ready, 1500);
    let _ = app.frame(later);
    app.drawn(later);
    let at = app.deadline().map_or(later, |due| due.max(later));
    let _ = app.frame(at);
    app.drawn(at);
    key(&mut app, "2", at);
    let (ticket, _) = rewind_sent(&link).expect("the rewind");
    give(
        &link,
        ticket,
        Ok(OpResult::SessionRewind(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, at);
    assert_eq!(
        app.input().editor().expanded(),
        "draft\nfix the lexer\nso digit separators work"
    );
    let all = notices(&mut app, at);
    let words = all.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        words.contains(
            "Rewound to before turn 2. The conversation goes on from there; what is above \
             this line is no longer part of it."
        ),
        "{all}"
    );
    let (ticket, read) = sent_one(&link, |op| match op {
        Op::SessionRead(read) => Some(read.clone()),
        _ => None,
    });
    assert_eq!(read.item_id, None);
    give(&link, ticket, Ok(page(history()[..2].to_vec(), None)));
    drain(&mut app, at);
    let _ = app.frame(at);
    key(&mut app, "ctrl+o", at);
    let viewed = text(&app.viewer_frame(60, 98, at)).join("\n");
    assert!(viewed.contains("first prompt"), "{viewed}");
    assert!(viewed.contains("First answer."), "{viewed}");
    assert!(
        !viewed.contains("Second answer."),
        "an undone turn: {viewed}"
    );
    assert!(!viewed.contains("third prompt"), "an undone turn: {viewed}");
    assert!(
        viewed.contains("Rewound to before turn 2. The conversation goes on from there."),
        "{viewed}"
    );
    assert!(!viewed.contains("above this line"), "{viewed}");
}

/// The full screen draws the branch read again too.
#[test]
fn the_full_screen_draws_the_branch_read_again_after_a_rewind() {
    let now = Instant::now();
    let settings = Settings {
        presenter: Presenter::Fullscreen,
        ..Settings::default()
    };
    let (mut app, link) = crate::testkit::app_with_settings(now, settings);
    opened(&mut app, &link, now);
    for item in history() {
        link.give(event(Event::ItemCompleted(ItemCompleted { item })));
    }
    drain(&mut app, now);
    let _ = app.screen_frame(now);
    let _ = link.take_sent();
    rewind_picker(&mut app, &link, history(), now);
    let _ = app.screen_frame(now);
    app.drawn(now);
    let ready = asked(&mut app, &link, two_files(), now);
    key(&mut app, "1", ready);
    let (ticket, _) = rewind_sent(&link).expect("the rewind");
    give(
        &link,
        ticket,
        Ok(OpResult::SessionRewind(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, ready);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionRead(_)).then_some(()));
    give(&link, ticket, Ok(page(history()[..2].to_vec(), None)));
    drain(&mut app, ready);
    let (lines, _) = app.screen_frame(ready);
    let shown = text(&lines).join("\n");
    assert!(shown.contains("First answer."), "{shown}");
    assert!(!shown.contains("Third answer."), "{shown}");
    assert!(
        shown.contains("Rewound to before turn 2. The conversation goes on from there."),
        "{shown}"
    );
}

/// A prompt sent while the transcript is read again waits for the read,
/// then goes as a new turn.
#[test]
fn a_prompt_sent_while_the_branch_is_read_again_waits_for_it() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    run_command(&mut app, "/undo", now);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionUndo(_)).then_some(()));
    give(
        &link,
        ticket,
        Ok(OpResult::SessionUndo(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, now);
    let (read, _) = sent_one(&link, |op| matches!(op, Op::SessionRead(_)).then_some(()));
    type_text(&mut app, "next", now);
    key(&mut app, "enter", now);
    assert!(
        turn_start(&link.take_sent()).is_empty(),
        "sent before the read"
    );
    give(&link, read, Ok(page(history()[..4].to_vec(), None)));
    drain(&mut app, now);
    let sent = link.take_sent();
    let prompts = turn_start(&sent);
    assert_eq!(prompts.len(), 1, "{sent:#?}");
    assert_eq!(prompts[0].delivery, None, "a new turn");
    let _ = app.frame(now);
    key(&mut app, "ctrl+o", now);
    let viewed = text(&app.viewer_frame(60, 98, now)).join("\n");
    assert!(viewed.contains("Second answer."), "{viewed}");
    assert!(!viewed.contains("Third answer."), "{viewed}");
}

// ------------------------------------------------------------------ fork

#[test]
fn tab_in_the_rewind_picker_forks_from_before_the_point() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    key(&mut app, "down", now);
    key(&mut app, "tab", now);
    let (ticket, fork) = sent_one(&link, |op| match op {
        Op::SessionFork(fork) => Some(fork.clone()),
        _ => None,
    });
    assert_eq!(fork.session_id, SessionId::from(SESSION));
    assert_eq!(
        (fork.at, fork.before),
        (None, Some(ItemId::from("i_3"))),
        "just before turn 2's prompt"
    );
    give(
        &link,
        ticket,
        Ok(OpResult::SessionFork(SessionResult {
            session: session().id("s_fork").build(),
        })),
    );
    drain(&mut app, now);
    let sent = link.take_sent();
    assert!(
        sent.iter().any(
            |(_, op)| matches!(op, Op::SessionClose(close) if close.session_id.as_str() == SESSION)
        ),
        "the session forked is left: {sent:#?}"
    );
    assert!(
        sent.iter().any(
            |(_, op)| matches!(op, Op::SessionRead(read) if read.session_id.as_str() == "s_fork")
        ),
        "the fork's history is drawn: {sent:#?}"
    );
    assert_eq!(
        app.session()
            .map(|state| state.session_id.as_str().to_owned()),
        Some("s_fork".to_owned())
    );
    assert!(notices(&mut app, now).contains("Forked from before turn 2"));
}

/// A fork that fails leaves the session as it was, open here.
#[test]
fn a_fork_that_fails_keeps_the_session() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    key(&mut app, "tab", now);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionFork(_)).then_some(()));
    give(
        &link,
        ticket,
        Err(ErrorData::new(
            ErrorKind::Usage,
            "session_not_stored",
            "A session with no log cannot be forked.",
        )),
    );
    drain(&mut app, now);
    assert_eq!(
        app.session()
            .map(|state| state.session_id.as_str().to_owned()),
        Some(SESSION.to_owned())
    );
    assert!(notices(&mut app, now).contains("cannot be forked"));
    // Not held as if opening: a prompt goes.
    type_text(&mut app, "go on", now);
    key(&mut app, "enter", now);
    assert_eq!(turn_start(&link.take_sent()).len(), 1);
}

fn summary(id: &str, title: Option<&str>, parent: Option<&str>) -> SessionSummary {
    SessionSummary {
        session_id: SessionId::from(id),
        workspace: WORKSPACE.to_owned(),
        title: title.map(str::to_owned),
        created_at: Timestamp::from("2026-11-15T09:00:00Z"),
        updated_at: Timestamp::from("2026-11-15T09:30:00Z"),
        parent: parent.map(SessionId::from),
        model: None,
        turns: 1,
    }
}

fn sessions_picker(app: &mut App, link: &Scripted, sessions: Vec<SessionSummary>, at: Instant) {
    run_command(app, "/sessions", at);
    let (ticket, _) = sent_one(link, |op| matches!(op, Op::SessionList(_)).then_some(()));
    give(
        link,
        ticket,
        Ok(OpResult::SessionList(SessionListResult {
            sessions,
            next_cursor: None,
        })),
    );
    drain(app, at);
}

#[test]
fn the_sessions_list_says_whose_fork_a_session_is() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    sessions_picker(
        &mut app,
        &link,
        vec![
            summary("s_child", Some("try it the other way"), Some(SESSION)),
            summary(
                SESSION,
                Some("fix the lexer so digit separators work"),
                None,
            ),
            summary("s_orphan", None, Some("s_4f2a99887766")),
        ],
        now,
    );
    let details: Vec<String> = app
        .input()
        .menu()
        .map(|menu| menu.rows.iter().map(|row| row.detail.clone()).collect())
        .unwrap_or_default();
    assert!(
        details[0].ends_with("fork of \"fix the lexer so digit…\""),
        "{details:?}"
    );
    assert!(!details[1].contains("fork of"), "{details:?}");
    assert!(details[2].ends_with("fork of s_4f2a…"), "{details:?}");
    let (_, live, _) = app.frame(now);
    assert!(text(&live).join("\n").contains("tab - fork it"));
}

#[test]
fn tab_in_sessions_forks_the_open_session_at_its_end() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    sessions_picker(&mut app, &link, vec![summary(SESSION, None, None)], now);
    key(&mut app, "tab", now);
    let (_, fork) = sent_one(&link, |op| match op {
        Op::SessionFork(fork) => Some(fork.clone()),
        _ => None,
    });
    assert_eq!(fork.session_id, SessionId::from(SESSION));
    assert_eq!((fork.at, fork.before), (None, None), "at its end");
}

/// Another session is opened to be forked, then the fork opened in this
/// one's place.
#[test]
fn tab_in_sessions_forks_another_session_at_its_end() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    sessions_picker(
        &mut app,
        &link,
        vec![summary(SESSION, None, None), summary("s_other", None, None)],
        now,
    );
    key(&mut app, "down", now);
    key(&mut app, "tab", now);
    let sent = link.take_sent();
    assert!(
        sent.iter().any(
            |(_, op)| matches!(op, Op::SessionClose(close) if close.session_id.as_str() == SESSION)
        ),
        "{sent:#?}"
    );
    let open = sent
        .iter()
        .find_map(|(ticket, op)| match op {
            Op::SessionOpen(open) if open.session_id.as_str() == "s_other" => Some(*ticket),
            _ => None,
        })
        .expect("the other opened to be forked");
    give(
        &link,
        open,
        Ok(OpResult::SessionOpen(SessionResult {
            session: session().id("s_other").build(),
        })),
    );
    drain(&mut app, now);
    let (_, fork) = sent_one(&link, |op| match op {
        Op::SessionFork(fork) => Some(fork.clone()),
        _ => None,
    });
    assert_eq!(fork.session_id, SessionId::from("s_other"));
    assert_eq!((fork.at, fork.before), (None, None));
}

/// A fork of another session from `/sessions` closes the one open here
/// first, as a switch does: one that fails leaves no session open, says
/// why, and closes the other, opened for it (the review of step 25's
/// second pull request found the guide said the session stayed open).
#[test]
fn a_failed_fork_of_another_session_leaves_none_open_and_says_why() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    sessions_picker(
        &mut app,
        &link,
        vec![summary(SESSION, None, None), summary("s_other", None, None)],
        now,
    );
    key(&mut app, "down", now);
    key(&mut app, "tab", now);
    let (open, _) = sent_one(&link, |op| {
        matches!(op, Op::SessionOpen(open) if open.session_id.as_str() == "s_other").then_some(())
    });
    give(
        &link,
        open,
        Ok(OpResult::SessionOpen(SessionResult {
            session: session().id("s_other").build(),
        })),
    );
    drain(&mut app, now);
    let (fork, _) = sent_one(&link, |op| matches!(op, Op::SessionFork(_)).then_some(()));
    give(
        &link,
        fork,
        Err(ErrorData::new(
            ErrorKind::Usage,
            "session_not_stored",
            "A session with no log cannot be forked.",
        )),
    );
    drain(&mut app, now);
    assert_eq!(app.session(), None, "no session is open here");
    let sent = link.take_sent();
    assert!(
        sent.iter().any(
            |(_, op)| matches!(op, Op::SessionClose(close) if close.session_id.as_str() == "s_other")
        ),
        "the other, opened for the fork, is closed: {sent:#?}"
    );
    assert!(notices(&mut app, now).contains("cannot be forked"));
}

/// Prompts typed while a fork from `/rewind` is on its way were meant for
/// the fork: when it fails they come back into the editor, and do not run
/// at the end of the session that stays (the review of step 25's second
/// pull request).
#[test]
fn prompts_held_through_a_failed_fork_come_back_into_the_editor() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    key(&mut app, "tab", now);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionFork(_)).then_some(()));
    type_text(&mut app, "typed ahead", now);
    key(&mut app, "enter", now);
    assert!(
        turn_start(&link.take_sent()).is_empty(),
        "sent before the fork"
    );
    give(
        &link,
        ticket,
        Err(ErrorData::new(
            ErrorKind::Usage,
            "session_not_stored",
            "A session with no log cannot be forked.",
        )),
    );
    drain(&mut app, now);
    assert!(
        turn_start(&link.take_sent()).is_empty(),
        "sent to the session that stays"
    );
    assert_eq!(app.input().editor().expanded(), "typed ahead");
    assert!(notices(&mut app, now).contains("cannot be forked"));
}

/// While a rewind is under way (its plan asked for), neither a switch nor
/// a fork from `/sessions` leaves: what comes back is this session's.
#[test]
fn a_switch_and_a_fork_wait_for_a_rewind_under_way() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    key(&mut app, "enter", now);
    let _ = link.take_sent();
    // A switch.
    sessions_picker(
        &mut app,
        &link,
        vec![summary(SESSION, None, None), summary("s_other", None, None)],
        now,
    );
    key(&mut app, "down", now);
    key(&mut app, "enter", now);
    // A fork of this one.
    sessions_picker(&mut app, &link, vec![summary(SESSION, None, None)], now);
    key(&mut app, "tab", now);
    let sent = link.take_sent();
    assert!(
        !sent.iter().any(|(_, op)| matches!(
            op,
            Op::SessionClose(_) | Op::SessionOpen(_) | Op::SessionFork(_)
        )),
        "{sent:#?}"
    );
    let all = notices(&mut app, now);
    assert!(
        all.contains("A rewind is under way: /sessions can switch once it is done."),
        "{all}"
    );
    assert!(
        all.contains("The session is being read: a fork can be made once it is."),
        "{all}"
    );
}

/// Keys and a paste that come before the question is ready answer nothing,
/// and reach nothing: they would wait in the hidden editor and come back in
/// front of the prompt the rewind gives back, for an `Enter` to send. They
/// hold the question's quiet, as typing does (the review of step 25's
/// second pull request).
#[test]
fn what_is_typed_before_the_question_is_ready_reaches_nothing() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    key(&mut app, "down", now);
    key(&mut app, "enter", now);
    for (ticket, op) in link.take_sent() {
        match op {
            Op::SessionRewindPlan(_) => give(
                &link,
                ticket,
                Ok(OpResult::SessionRewindPlan(plan(two_files()))),
            ),
            Op::SessionRead(_) => give(&link, ticket, Ok(page(vec![history()[2].clone()], None))),
            _ => {}
        }
    }
    drain(&mut app, now);
    let _ = app.frame(now);
    app.drawn(now);
    let first_due = app.deadline().expect("the answers' moment");
    let typed = ms(now, 100);
    type_text(&mut app, "12x", typed);
    app.terminal(&TermEvent::Paste("pasted".to_owned()), typed);
    assert_eq!(
        app.input().editor().expanded(),
        "",
        "typed under the question"
    );
    key(&mut app, "enter", typed);
    let sent = link.take_sent();
    assert!(
        !sent
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionRewind(_))),
        "answered before it was ready: {sent:#?}"
    );
    assert!(turn_start(&sent).is_empty(), "sent as a prompt: {sent:#?}");
    assert_eq!(app.input().editor().expanded(), "");
    let _ = app.frame(typed);
    app.drawn(typed);
    let ready = app.deadline().expect("the answers' moment");
    assert!(ready > first_due, "the quiet was not held");
    let _ = app.frame(ready);
    app.drawn(ready);
    key(&mut app, "2", ready);
    let (ticket, _) = rewind_sent(&link).expect("the rewind");
    give(
        &link,
        ticket,
        Ok(OpResult::SessionRewind(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, ready);
    assert_eq!(
        app.input().editor().expanded(),
        "fix the lexer\nso digit separators work",
        "the prompt alone comes back"
    );
}

/// Each awaits its own kind of answer: a plan answered to the rewind ends
/// the rewind as an error does, and nothing waits for it (the review of
/// step 25's second pull request found it left `/rewind`, `/new` and the
/// rest refused for good).
#[test]
fn a_plan_given_as_the_rewinds_answer_ends_the_rewind() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    key(&mut app, "2", ready);
    let (ticket, _) = rewind_sent(&link).expect("the rewind");
    give(
        &link,
        ticket,
        Ok(OpResult::SessionRewindPlan(plan(two_files()))),
    );
    drain(&mut app, ready);
    run_command(&mut app, "/rewind", ready);
    assert!(
        link.take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionRead(read) if read.item_id.is_none())),
        "the rewind still waited"
    );
}

/// A prompt sent after the answer waits for the rewind and for the read
/// after it, so that its events are not drawn and then read again (the
/// review of step 25's second pull request); a rewind refused gives it
/// back, as it was meant for the conversation rewound.
#[test]
fn a_prompt_sent_after_the_answer_waits_for_the_rewind() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    key(&mut app, "2", ready);
    let (ticket, _) = rewind_sent(&link).expect("the rewind");
    type_text(&mut app, "next", ready);
    key(&mut app, "enter", ready);
    assert!(
        turn_start(&link.take_sent()).is_empty(),
        "sent under the rewind"
    );
    give(
        &link,
        ticket,
        Ok(OpResult::SessionRewind(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, ready);
    let (read, _) = sent_one(&link, |op| matches!(op, Op::SessionRead(_)).then_some(()));
    give(&link, read, Ok(page(history()[..2].to_vec(), None)));
    drain(&mut app, ready);
    let sent = link.take_sent();
    let prompts = turn_start(&sent);
    assert_eq!(prompts.len(), 1, "{sent:#?}");
    assert_eq!(prompts[0].input, vec![Part::text("next")]);

    // Refused: the prompt comes back, and nothing is sent.
    let (mut app, link) = with_history(now);
    rewind_picker(&mut app, &link, history(), now);
    let ready = asked(&mut app, &link, two_files(), now);
    key(&mut app, "2", ready);
    let (ticket, _) = rewind_sent(&link).expect("the rewind");
    type_text(&mut app, "next", ready);
    key(&mut app, "enter", ready);
    give(
        &link,
        ticket,
        Err(ErrorData::new(
            ErrorKind::Usage,
            cineris_protocol::error::code::PLAN_CHANGED,
            "The rewind's plan changed after it was shown.",
        )),
    );
    drain(&mut app, ready);
    assert!(
        turn_start(&link.take_sent()).is_empty(),
        "sent after a refusal"
    );
    assert_eq!(app.input().editor().expanded(), "next");
}

/// `Ctrl+C` while prompts wait for the transcript read again gives them
/// back and says so: they had not been sent (the review of step 25's
/// second pull request found them sent when the read ended).
#[test]
fn ctrl_c_gives_back_the_prompts_waiting_for_the_read_again() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    run_command(&mut app, "/undo", now);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionUndo(_)).then_some(()));
    give(
        &link,
        ticket,
        Ok(OpResult::SessionUndo(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, now);
    let (read, _) = sent_one(&link, |op| matches!(op, Op::SessionRead(_)).then_some(()));
    type_text(&mut app, "next", now);
    key(&mut app, "enter", now);
    key(&mut app, "ctrl+c", now);
    assert_eq!(app.input().editor().expanded(), "next");
    assert!(
        notices(&mut app, now).contains("waited for the rewind were not sent"),
        "not said"
    );
    give(&link, read, Ok(page(history()[..4].to_vec(), None)));
    drain(&mut app, now);
    let sent = link.take_sent();
    assert!(turn_start(&sent).is_empty(), "sent after all: {sent:#?}");
    assert!(
        !sent
            .iter()
            .any(|(_, op)| matches!(op, Op::TurnInterrupt(_))),
        "nothing ran to interrupt: {sent:#?}"
    );
}

/// The transcript's read again failing is said, and the prompts that
/// waited for it go: the history did change.
#[test]
fn a_read_again_that_fails_is_said_and_the_prompts_waiting_go() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    run_command(&mut app, "/undo", now);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionUndo(_)).then_some(()));
    give(
        &link,
        ticket,
        Ok(OpResult::SessionUndo(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, now);
    let (read, _) = sent_one(&link, |op| matches!(op, Op::SessionRead(_)).then_some(()));
    type_text(&mut app, "next", now);
    key(&mut app, "enter", now);
    give(
        &link,
        read,
        Err(ErrorData::new(
            ErrorKind::Internal,
            "store_failed",
            "The session's log could not be read.",
        )),
    );
    drain(&mut app, now);
    let sent = link.take_sent();
    let prompts = turn_start(&sent);
    assert_eq!(prompts.len(), 1, "{sent:#?}");
    assert_eq!(prompts[0].delivery, None, "a new turn");
    assert!(notices(&mut app, now).contains("could not be read"));
}

/// An item whose events came while the branch is read again is drawn from
/// them, and not again from the read; one the read says is still in
/// progress is left to its events.
#[test]
fn what_came_while_the_branch_was_read_is_drawn_once() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    run_command(&mut app, "/undo", now);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionUndo(_)).then_some(()));
    give(
        &link,
        ticket,
        Ok(OpResult::SessionUndo(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, now);
    let (read, _) = sent_one(&link, |op| matches!(op, Op::SessionRead(_)).then_some(()));
    // Another client's turn, whose prompt and answer come meanwhile.
    link.give(event(Event::ItemCompleted(ItemCompleted {
        item: user("i_7", "elsewhere prompt"),
    })));
    link.give(event(Event::ItemCompleted(ItemCompleted {
        item: answer("i_8", "Elsewhere answer."),
    })));
    drain(&mut app, now);
    let mut items = history()[..4].to_vec();
    items.push(user("i_7", "elsewhere prompt"));
    items.push(answer("i_8", "Elsewhere answer."));
    items.push(answer("i_9", "Still streaming."));
    give(
        &link,
        read,
        Ok(OpResult::SessionRead(SessionReadResult {
            items,
            incomplete: vec![ItemId::from("i_9")],
            next_cursor: None,
            seq: 9,
            masked: 0,
            redacted: false,
        })),
    );
    drain(&mut app, now);
    let _ = app.frame(now);
    key(&mut app, "ctrl+o", now);
    let viewed = text(&app.viewer_frame(80, 98, now)).join("\n");
    assert_eq!(viewed.matches("Elsewhere answer.").count(), 1, "{viewed}");
    assert_eq!(viewed.matches("elsewhere prompt").count(), 1, "{viewed}");
    assert!(!viewed.contains("Still streaming."), "{viewed}");
    assert!(viewed.contains("Second answer."), "{viewed}");
}

/// `/copy` after an undo or a rewind copies the branch's last answer, not
/// the one undone (the review of step 25's second pull request).
#[test]
fn copy_after_an_undo_copies_the_branchs_last_answer() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    assert_eq!(app.last_answer().trim(), "Third answer.");
    run_command(&mut app, "/undo", now);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionUndo(_)).then_some(()));
    give(
        &link,
        ticket,
        Ok(OpResult::SessionUndo(SessionResult {
            session: session().build(),
        })),
    );
    drain(&mut app, now);
    let (read, _) = sent_one(&link, |op| matches!(op, Op::SessionRead(_)).then_some(()));
    give(&link, read, Ok(page(history()[..4].to_vec(), None)));
    drain(&mut app, now);
    assert_eq!(app.last_answer().trim(), "Second answer.");
}

/// The points' read failing is said, opens nothing, and leaves `/rewind`
/// free to ask again.
#[test]
fn a_points_read_that_fails_is_said_and_holds_nothing_up() {
    let now = Instant::now();
    let (mut app, link) = with_history(now);
    run_command(&mut app, "/rewind", now);
    let (ticket, _) = sent_one(&link, |op| matches!(op, Op::SessionRead(_)).then_some(()));
    give(
        &link,
        ticket,
        Err(ErrorData::new(
            ErrorKind::Internal,
            "store_failed",
            "The session's log could not be read.",
        )),
    );
    drain(&mut app, now);
    assert!(rows(&app).is_empty(), "a picker opened");
    assert!(notices(&mut app, now).contains("could not be read"));
    run_command(&mut app, "/rewind", now);
    assert!(
        link.take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionRead(_))),
        "the failed read held /rewind up"
    );
}

/// The prompt read whole failing, or giving another item than the point's,
/// leaves the rewind to go on: the editor gets nothing, and that is said
/// after the rewind.
#[test]
fn a_prompt_that_cannot_be_read_is_not_given_back_and_that_is_said() {
    let now = Instant::now();
    for prompt in [
        Err(ErrorData::new(
            ErrorKind::Internal,
            "store_failed",
            "The session's log could not be read.",
        )),
        // Another prompt than the point's.
        Ok(page(vec![history()[4].clone()], None)),
    ] {
        let (mut app, link) = with_history(now);
        rewind_picker(&mut app, &link, history(), now);
        key(&mut app, "down", now);
        key(&mut app, "enter", now);
        for (ticket, op) in link.take_sent() {
            match op {
                Op::SessionRewindPlan(_) => give(
                    &link,
                    ticket,
                    Ok(OpResult::SessionRewindPlan(plan(two_files()))),
                ),
                Op::SessionRead(_) => give(&link, ticket, prompt.clone()),
                _ => {}
            }
        }
        drain(&mut app, now);
        let _ = app.frame(now);
        app.drawn(now);
        let ready = app.deadline().expect("the answers' moment");
        let _ = app.frame(ready);
        app.drawn(ready);
        key(&mut app, "2", ready);
        let (ticket, _) = rewind_sent(&link).expect("the rewind");
        give(
            &link,
            ticket,
            Ok(OpResult::SessionRewind(SessionResult {
                session: session().build(),
            })),
        );
        drain(&mut app, ready);
        assert_eq!(app.input().editor().expanded(), "", "{prompt:?}");
        let words = notices(&mut app, ready)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            words.contains("The prompt of that turn could not be read"),
            "{prompt:?}: {words}"
        );
    }
}

// --------------------------------------------------------- hostile text

/// Text from outside in every new listing is sanitised and cut: the
/// picker's rows, the plan's file names, and a fork's note.
#[test]
fn hostile_text_in_the_rows_the_plan_and_a_forks_note_is_made_safe() {
    let now = Instant::now();
    let hostile = TRICKY.concat();
    let (mut app, link) = with_history(now);
    let mut items = history();
    items[2] = user("i_3", &format!("{hostile}\u{202e}evil\u{2066}{hostile}"));
    rewind_picker(&mut app, &link, items, now);
    for columns in [40, 60, 99] {
        app.resize(columns - 1, 40);
        let (_, live, _) = app.frame(now);
        let found = problems(&live, columns - 1);
        assert!(found.is_empty(), "{columns}: {found:?}");
    }
    let row = rows(&app).join("\n");
    assert!(row.contains("evil"), "the text is kept, made safe: {row}");
    app.resize(98, 40);
    let files = vec![
        file(&hostile, RewindTarget::Content, RewindFileStatus::AsLeft),
        RewindFile {
            path: "a\u{1b}]52;c;aGk=\u{7}\nb\u{202e}c".to_owned(),
            to: RewindTarget::NotRecorded,
            status: RewindFileStatus::Cannot,
            why_not: Some(RewindWhyNot::NotRecorded),
        },
    ];
    let ready = asked(&mut app, &link, files, now);
    for columns in [40, 60, 99] {
        app.resize(columns - 1, 40);
        let (_, live, _) = app.frame(ready);
        let found = problems(&live, columns - 1);
        assert!(found.is_empty(), "{columns}: {found:?}");
        let shown = text(&live).join("\n");
        assert!(
            shown.contains("\\u{202e}"),
            "{columns}: not shown escaped:\n{shown}"
        );
    }
    // The plan whole in the viewer.
    key(&mut app, "ctrl+o", ready);
    assert!(app.viewing());
    for columns in [40, 60, 99] {
        let lines = app.viewer_frame(40, columns - 1, ready);
        let found = problems(&lines, columns - 1);
        assert!(found.is_empty(), "the viewer at {columns}: {found:?}");
    }

    let (mut app, link) = with_history(now);
    sessions_picker(
        &mut app,
        &link,
        vec![
            summary("s_child", Some(&hostile), Some("s_parent")),
            summary("s_parent", Some(&format!("{hostile}\u{202e}")), None),
            summary("s_lost", None, Some(&format!("s_\u{1b}[2J{hostile}"))),
        ],
        now,
    );
    for columns in [40, 60, 99] {
        app.resize(columns - 1, 40);
        let (_, live, _) = app.frame(now);
        let found = problems(&live, columns - 1);
        assert!(found.is_empty(), "{columns}: {found:?}");
    }
}
