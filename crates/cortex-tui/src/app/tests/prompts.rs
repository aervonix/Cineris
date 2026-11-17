//! Prompts at a turn's edges (step 27, `docs/steps/27/answered.md`, point
//! 5): the prompts held while the session opened go one at a time, so that
//! their order holds; the first steers a turn the open says is running; and
//! a prompt refused `turn_running` is sent again once, queued.

use cortex_protocol::error::code;
use cortex_protocol::item::Delivery as ItemDelivery;
use cortex_protocol::session::SessionState;

use super::*;

/// A prompt sent: its ticket, its text and its delivery.
type Prompt = (Ticket, String, Option<ItemDelivery>);

/// The prompts sent since the last look, taken with everything else sent.
fn prompts(link: &Scripted) -> Vec<Prompt> {
    link.take_sent()
        .into_iter()
        .filter_map(|(ticket, op)| match op {
            Op::TurnStart(params) => {
                let text: Vec<String> = params
                    .input
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                    .collect();
                Some((ticket, text.join("\n"), params.delivery))
            }
            _ => None,
        })
        .collect()
}

/// The one prompt sent since the last look.
fn one_prompt(link: &Scripted) -> Prompt {
    let mut sent = prompts(link);
    assert_eq!(sent.len(), 1, "not one prompt: {sent:#?}");
    sent.remove(0)
}

fn accepted(link: &Scripted, ticket: Ticket, disposition: Disposition) {
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::TurnStart(TurnStartResult {
            turn_id: TurnId::from("t_2"),
            disposition,
        })),
    ));
}

fn refused(link: &Scripted, ticket: Ticket, code: &str) {
    link.give(Inbound::Done(
        ticket,
        Err(ErrorData::new(ErrorKind::Usage, code, "Refused.")),
    ));
}

/// Every line committed so far and the live region now, as one text.
fn shown(app: &mut App, now: Instant) -> String {
    let mut all = Vec::new();
    let live = screen(app, now, &mut all);
    all.extend(live);
    all.join("\n")
}

/// `first` given on the command line, and `typed` sent while the session
/// opens: the application and its link, the open not answered yet.
fn holding(now: Instant, first: &str, typed: &[&str]) -> (App, Scripted) {
    let (mut app, link) = app_with(now, Some(first));
    for text in typed {
        type_text(&mut app, text, now);
        key(&mut app, "enter", now);
    }
    (app, link)
}

/// Opens the stored session `s_a` (`--resume`, as `cortex attach` does) and
/// answers the open with `state`; returns the ticket of its history's read.
fn attached(app: &mut App, link: &Scripted, state: SessionState, now: Instant) -> Ticket {
    let _ = app.frame(now);
    app.drawn(now);
    let open = link
        .script()
        .sent
        .iter()
        .find(|(_, op)| matches!(op, Op::SessionOpen(_)))
        .map(|(ticket, _)| *ticket)
        .expect("the open");
    link.give(Inbound::Done(
        open,
        Ok(OpResult::SessionOpen(SessionResult { session: state })),
    ));
    drain(app, now);
    link.script()
        .sent
        .iter()
        .find(|(_, op)| matches!(op, Op::SessionRead(_)))
        .map(|(ticket, _)| *ticket)
        .expect("the history's read")
}

// ---------------------------------------- (1) held prompts, one at a time

/// Each held prompt goes once the one before it was answered: the first as
/// a prompt, the rest queued behind it, each once and in order.
#[test]
fn held_prompts_go_one_at_a_time_each_once_the_one_before_is_answered() {
    let now = Instant::now();
    let (mut app, link) = holding(now, "first", &["second", "third"]);
    opened(&mut app, &link, now);
    let (ticket, text, delivery) = one_prompt(&link);
    assert_eq!((text.as_str(), delivery), ("first", None));
    accepted(&link, ticket, Disposition::Started);
    drain(&mut app, now);
    let (ticket, text, delivery) = one_prompt(&link);
    assert_eq!(
        (text.as_str(), delivery),
        ("second", Some(ItemDelivery::Queue))
    );
    accepted(&link, ticket, Disposition::Queued);
    drain(&mut app, now);
    let (ticket, text, delivery) = one_prompt(&link);
    assert_eq!(
        (text.as_str(), delivery),
        ("third", Some(ItemDelivery::Queue))
    );
    accepted(&link, ticket, Disposition::Queued);
    drain(&mut app, now);
    assert!(prompts(&link).is_empty(), "each is sent once");
    assert_eq!(app.input().editor().expanded(), "", "nothing comes back");
    assert!(app.running());
}

/// The first is refused (`history_changing`): it comes back with those
/// behind it, in the order they were written, and none of them is sent.
/// Sent at once, the second, queued with no turn running, started a turn
/// of its own, and the first came back behind it.
#[test]
fn a_refused_held_prompt_gives_back_those_behind_it_in_order() {
    let now = Instant::now();
    let (mut app, link) = holding(now, "first", &["second", "third"]);
    opened(&mut app, &link, now);
    let (ticket, text, _) = one_prompt(&link);
    assert_eq!(text, "first");
    refused(&link, ticket, code::HISTORY_CHANGING);
    drain(&mut app, now);
    assert!(prompts(&link).is_empty(), "nothing behind it is sent");
    assert_eq!(app.input().editor().expanded(), "first\nsecond\nthird");
    assert!(!app.running(), "no turn was started");
    assert!(shown(&mut app, now).contains("send it again"));
}

/// A prompt sent once the session is open, while the held ones are still on
/// their way, follows them: it neither overtakes them nor comes back before
/// them.
#[test]
fn a_prompt_sent_while_held_ones_are_on_their_way_follows_them() {
    let now = Instant::now();
    let (mut app, link) = holding(now, "first", &["second"]);
    opened(&mut app, &link, now);
    let (first, _, _) = one_prompt(&link);
    type_text(&mut app, "third", now);
    key(&mut app, "enter", now);
    assert!(prompts(&link).is_empty(), "it waits for those before it");
    accepted(&link, first, Disposition::Started);
    drain(&mut app, now);
    let (second, text, _) = one_prompt(&link);
    assert_eq!(text, "second");
    refused(&link, second, code::HISTORY_CHANGING);
    drain(&mut app, now);
    assert!(prompts(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), "second\nthird");
}

/// Stopping while held prompts are on their way: the one sent is the
/// interrupt's, and those not sent come back once it is answered, rather
/// than start a turn after the stop.
#[test]
fn stopping_while_held_prompts_are_on_their_way_gives_back_those_not_sent() {
    let now = Instant::now();
    let (mut app, link) = holding(now, "first", &["second", "third"]);
    opened(&mut app, &link, now);
    let (first, _, _) = one_prompt(&link);
    key(&mut app, "esc", now);
    assert!(
        link.script()
            .sent
            .iter()
            .any(|(_, op)| matches!(op, Op::TurnInterrupt(_))),
        "the interrupt is sent"
    );
    accepted(&link, first, Disposition::Started);
    drain(&mut app, now);
    assert!(prompts(&link).is_empty(), "nothing more is sent");
    assert_eq!(app.input().editor().expanded(), "second\nthird");
}

// ----------------------------------- (3) an open onto a running turn

/// `session/open` says a turn of another client's runs (`cortex attach`
/// meets it every time): the held prompt steers it, as `Enter` steers a
/// running turn. With no delivery, the core refused it `turn_running`.
#[test]
fn the_first_held_prompt_steers_a_turn_the_open_says_is_running() {
    let now = Instant::now();
    let (mut app, link) = app_opening(
        now,
        Some("and the docs"),
        Open::Resume(SessionId::from("s_a")),
    );
    let _ = attached(
        &mut app,
        &link,
        session().id("s_a").seq(5).running("t_4").build(),
        now,
    );
    let (_, text, delivery) = one_prompt(&link);
    assert_eq!(
        (text.as_str(), delivery),
        ("and the docs", Some(ItemDelivery::Steer))
    );
}

/// After the first held prompt steered the running turn, the next is queued
/// behind that turn once the steer was answered.
#[test]
fn after_a_held_steer_the_next_held_prompt_is_queued() {
    let now = Instant::now();
    let (mut app, link) = app_opening(
        now,
        Some("and the docs"),
        Open::Resume(SessionId::from("s_a")),
    );
    type_text(&mut app, "then the tests", now);
    key(&mut app, "enter", now);
    let _ = attached(
        &mut app,
        &link,
        session().id("s_a").seq(5).running("t_4").build(),
        now,
    );
    let (ticket, text, _) = one_prompt(&link);
    assert_eq!(text, "and the docs");
    accepted(&link, ticket, Disposition::Steering);
    drain(&mut app, now);
    let (_, text, delivery) = one_prompt(&link);
    assert_eq!(
        (text.as_str(), delivery),
        ("then the tests", Some(ItemDelivery::Queue))
    );
}

/// An open that says no turn runs sends the first held prompt as a prompt.
#[test]
fn the_first_held_prompt_is_a_prompt_when_the_open_says_no_turn_runs() {
    let now = Instant::now();
    let (mut app, link) = app_opening(
        now,
        Some("and the docs"),
        Open::Resume(SessionId::from("s_a")),
    );
    let _ = attached(&mut app, &link, session().id("s_a").seq(5).build(), now);
    let (_, text, delivery) = one_prompt(&link);
    assert_eq!((text.as_str(), delivery), ("and the docs", None));
}

/// While held prompts are on their way the session is not left: `/new`
/// waits, though the turn the open found has ended, so that what follows
/// is never sent to the next session or given back in the wrong order.
#[test]
fn the_session_is_not_left_while_held_prompts_are_on_their_way() {
    let now = Instant::now();
    let (mut app, link) = app_opening(
        now,
        Some("and the docs"),
        Open::Resume(SessionId::from("s_a")),
    );
    type_text(&mut app, "then the tests", now);
    key(&mut app, "enter", now);
    let read = attached(
        &mut app,
        &link,
        session().id("s_a").seq(5).running("t_4").build(),
        now,
    );
    let (ticket, _, _) = one_prompt(&link);
    link.give(Inbound::Done(
        read,
        Ok(OpResult::SessionRead(
            cortex_protocol::methods::SessionReadResult {
                items: Vec::new(),
                incomplete: Vec::new(),
                next_cursor: None,
                seq: 5,
            },
        )),
    ));
    link.give(Inbound::Event(SessionEvent {
        session_id: SessionId::from("s_a"),
        seq: 6,
        turn_id: Some(TurnId::from("t_4")),
        event: Event::TurnCompleted(TurnCompleted {
            outcome: TurnOutcome::Completed,
            stop_reason: Some(StopReason::End),
            error: None,
            usage: Default::default(),
            verification: None,
            duration_ms: Some(1200),
        }),
    }));
    drain(&mut app, now);
    type_text(&mut app, "/new", now);
    key(&mut app, "enter", now);
    key(&mut app, "enter", now);
    assert!(
        !link
            .script()
            .sent
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionClose(_) | Op::SessionNew(_))),
        "the session was left"
    );
    accepted(&link, ticket, Disposition::Started);
    drain(&mut app, now);
    let sent = link.take_sent();
    let next: Vec<_> = sent
        .iter()
        .filter_map(|(_, op)| match op {
            Op::TurnStart(params) => Some(params.session_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(next, [SessionId::from("s_a")], "{sent:#?}");
}
