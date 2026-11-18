//! Opening, reading and leaving sessions: a history read while a turn
//! streams, a switch while an open or a read is under way, and results that
//! come back for a session left meanwhile.

use cineris_protocol::item::{ToolOutcome, ToolResultItem};
use cineris_protocol::message::ToolCallPart;
use cineris_protocol::methods::{ConfigGetResult, SessionListResult, SessionReadResult};

use super::*;

fn state_of(id: &str, model: &str) -> cineris_protocol::session::SessionState {
    session().id(id).main_model(model).seq(5).build()
}

fn ev(session: &str, seq: u64, event: Event) -> Inbound {
    Inbound::Event(SessionEvent {
        session_id: SessionId::from(session),
        seq,
        turn_id: Some(TurnId::from("t_2")),
        event,
    })
}

fn user(id: &str, text: &str) -> Item {
    Item::UserMessage(UserMessageItem {
        id: ItemId::from(id),
        parts: vec![Part::text(text)],
        delivery: None,
        created_at: None,
    })
}

fn answer(id: &str, parts: Vec<Part>) -> Item {
    Item::AssistantMessage(AssistantMessageItem {
        id: ItemId::from(id),
        parts,
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

fn delta(id: &str, text: &str) -> Event {
    Event::ItemDelta(ItemDelta {
        item_id: ItemId::from(id),
        delta: Delta::Text(TextDelta {
            text: text.to_owned(),
        }),
    })
}

fn answer_started(id: &str) -> Event {
    Event::ItemStarted(ItemStarted {
        item_id: ItemId::from(id),
        kind: ItemKind::AssistantMessage,
        tool: None,
        call_id: None,
    })
}

fn ticket(sent: &[(Ticket, Op)], f: impl Fn(&Op) -> bool) -> Ticket {
    sent.iter()
        .find(|(_, op)| f(op))
        .map(|(t, _)| *t)
        .unwrap_or_else(|| panic!("not sent: {sent:#?}"))
}

/// Every line committed so far and the live region now.
fn everything(app: &mut App, now: Instant) -> Vec<String> {
    let mut all = Vec::new();
    let live = screen(app, now, &mut all);
    all.extend(live);
    all
}

/// Opens the stored session `id` (`--resume`) and returns the ticket of
/// its history's read, with everything sent.
fn resumed(
    app: &mut App,
    link: &Scripted,
    id: &str,
    model: &str,
    now: Instant,
) -> (Ticket, Vec<(Ticket, Op)>) {
    let _ = app.frame(now);
    app.drawn(now);
    let open = ticket(&link.take_sent(), |op| matches!(op, Op::SessionOpen(_)));
    link.give(Inbound::Done(
        open,
        Ok(OpResult::SessionOpen(SessionResult {
            session: state_of(id, model),
        })),
    ));
    drain(app, now);
    let sent = link.take_sent();
    (ticket(&sent, |op| matches!(op, Op::SessionRead(_))), sent)
}

fn summary(id: &str) -> SessionSummary {
    SessionSummary {
        session_id: SessionId::from(id),
        workspace: "/work/app".to_owned(),
        title: Some(id.to_owned()),
        created_at: Timestamp::from("2026-11-15T09:00:00Z"),
        updated_at: Timestamp::from("2026-11-15T09:30:00Z"),
        parent: None,
        model: None,
        turns: 1,
    }
}

/// `/sessions`, its list of `a` and `b`, and `b` picked.
fn pick_second_session(app: &mut App, link: &Scripted, now: Instant) {
    run_command(app, "/sessions", now);
    let list = ticket(&link.take_sent(), |op| matches!(op, Op::SessionList(_)));
    link.give(Inbound::Done(
        list,
        Ok(OpResult::SessionList(SessionListResult {
            sessions: vec![summary("s_a"), summary("s_b")],
            next_cursor: None,
        })),
    ));
    drain(app, now);
    key(app, "down", now);
    key(app, "enter", now);
}

fn read_result(items: Vec<Item>, incomplete: &[&str], seq: u64) -> OpResult {
    OpResult::SessionRead(SessionReadResult {
        items,
        incomplete: incomplete.iter().map(|id| ItemId::from(*id)).collect(),
        next_cursor: None,
        seq,
    })
}

fn shell_call(call: &str) -> Part {
    Part::ToolCall(ToolCallPart {
        id: CallId::from(call),
        name: "shell".to_owned(),
        arguments: serde_json::json!({"command": "make check"}),
        raw: None,
    })
}

fn shell_result(id: &str, call: &str, outcome: ToolOutcome) -> Item {
    Item::ToolResult(ToolResultItem {
        id: ItemId::from(id),
        call_id: CallId::from(call),
        tool: "shell".to_owned(),
        outcome,
        content: vec![Part::text("ok")],
        payload: Some(serde_json::json!({"exit_code": 0})),
        artifacts: Vec::new(),
        duration_ms: Some(1000),
        created_at: None,
    })
}

/// The lines of the boxes that ran `make check`.
fn runs(all: &[String]) -> Vec<&String> {
    all.iter().filter(|line| line.contains("Run")).collect()
}

// ------------------------------------------ a history read while a turn runs

/// `cineris --resume ID "go on"`: the held prompt is sent as the session
/// opens, beside the history's read, and the answer is mid-stream when the
/// read is taken. 11-rpc: "an item in progress comes back with the content
/// it has so far".
#[test]
fn an_answer_in_progress_at_the_read_is_drawn_once_and_whole() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, Some("go on"), Open::Resume(SessionId::from("s_a")));
    let (read, sent) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    assert!(sent.iter().any(|(_, op)| matches!(op, Op::TurnStart(_))));
    // The turn's events while the read is under way: held.
    link.give(ev("s_a", 6, Event::TurnStarted(TurnStarted {})));
    link.give(ev(
        "s_a",
        7,
        Event::ItemCompleted(ItemCompleted {
            item: user("i_5", "go on"),
        }),
    ));
    link.give(ev("s_a", 8, answer_started("i_6")));
    link.give(ev("s_a", 9, delta("i_6", "Hello ")));
    // The read was taken at 9; this came after it, before its result.
    link.give(ev("s_a", 10, delta("i_6", "world.")));
    drain(&mut app, now);
    link.give(Inbound::Done(
        read,
        Ok(read_result(
            vec![
                user("i_1", "earlier"),
                answer("i_2", vec![Part::text("Earlier answer.\n")]),
                user("i_5", "go on"),
                answer("i_6", vec![Part::text("Hello ")]),
            ],
            &["i_6"],
            9,
        )),
    ));
    drain(&mut app, now);
    // What came after the read goes on from it, live.
    let mut all = Vec::new();
    let live = screen(&mut app, now, &mut all);
    assert!(
        all.iter()
            .chain(&live)
            .any(|line| line.contains("Hello world.")),
        "{all:#?}\n{live:#?}"
    );
    link.give(ev(
        "s_a",
        11,
        Event::ItemCompleted(ItemCompleted {
            item: answer("i_6", vec![Part::text("Hello world.")]),
        }),
    ));
    drain(&mut app, now);
    let live = screen(&mut app, now, &mut all);
    all.extend(live);
    let joined = all.join("\n");
    assert!(
        joined.contains("Hello world.")
            && !joined.contains("world.world.")
            && !joined.contains("Hello Hello"),
        "{all:#?}"
    );
    assert_eq!(
        all.iter().filter(|line| line.contains("go on")).count(),
        1,
        "the prompt is drawn once: {all:#?}"
    );
}

/// The same race with a command running: its call is in the history's
/// last answer, its result in progress.
#[test]
fn a_command_running_at_the_read_ends_in_one_box_that_succeeded() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, Some("go on"), Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    link.give(Inbound::Done(
        read,
        Ok(read_result(
            vec![
                user("i_5", "go on"),
                answer("i_6", vec![shell_call("c_1")]),
                shell_result("i_7", "c_1", ToolOutcome::from_wire("running")),
            ],
            &["i_7"],
            12,
        )),
    ));
    drain(&mut app, now);
    link.give(ev(
        "s_a",
        13,
        Event::ItemCompleted(ItemCompleted {
            item: shell_result("i_7", "c_1", ToolOutcome::Success),
        }),
    ));
    drain(&mut app, now);
    let all = everything(&mut app, now);
    let runs = runs(&all);
    assert_eq!(runs.len(), 1, "{all:#?}");
    assert!(
        runs[0].contains("make check") && runs[0].contains('\u{2713}'),
        "{all:#?}"
    );
}

/// The turn began before the read, as its held `turn.started` says, though
/// the open said none ran: the call the history holds with no result yet is
/// not cut off, and its result ends it in its box.
#[test]
fn a_call_of_a_turn_that_began_before_the_read_is_not_cut_off() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, Some("go on"), Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    link.give(ev("s_a", 6, Event::TurnStarted(TurnStarted {})));
    drain(&mut app, now);
    link.give(Inbound::Done(
        read,
        Ok(read_result(
            vec![user("i_5", "go on"), answer("i_6", vec![shell_call("c_1")])],
            &[],
            9,
        )),
    ));
    drain(&mut app, now);
    link.give(ev(
        "s_a",
        10,
        Event::ItemStarted(ItemStarted {
            item_id: ItemId::from("i_7"),
            kind: ItemKind::ToolResult,
            tool: Some("shell".to_owned()),
            call_id: Some(CallId::from("c_1")),
        }),
    ));
    link.give(ev(
        "s_a",
        11,
        Event::ItemCompleted(ItemCompleted {
            item: shell_result("i_7", "c_1", ToolOutcome::Success),
        }),
    ));
    drain(&mut app, now);
    let all = everything(&mut app, now);
    let runs = runs(&all);
    assert_eq!(runs.len(), 1, "{all:#?}");
    assert!(runs[0].contains('\u{2713}'), "{all:#?}");
}

/// Attached to a turn another client started: the read holds its answer in
/// progress, and `Enter` steers that turn rather than send a prompt the
/// core refuses (`turn_running`).
#[test]
fn enter_steers_a_turn_found_running_at_the_read() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    link.give(Inbound::Done(
        read,
        Ok(read_result(
            vec![
                user("i_5", "go on"),
                answer("i_6", vec![Part::text("Working on ")]),
            ],
            &["i_6"],
            9,
        )),
    ));
    drain(&mut app, now);
    assert!(app.running(), "a turn runs");
    let _ = link.take_sent();
    type_text(&mut app, "and the docs", now);
    key(&mut app, "enter", now);
    let sent = link.take_sent();
    let starts = turn_start(&sent);
    assert_eq!(starts.len(), 1, "{sent:#?}");
    assert_eq!(
        starts[0].delivery,
        Some(cineris_protocol::item::Delivery::Steer)
    );
}

/// A response whose deltas come with no `item.started` before them may have
/// begun before this client saw it: none of it is drawn until it completes,
/// and then it is drawn whole, once.
#[test]
fn a_response_whose_start_was_never_seen_is_drawn_whole_once() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    link.give(ev(SESSION, 1, delta("i_6", "world.")));
    link.give(ev(
        SESSION,
        2,
        Event::ItemCompleted(ItemCompleted {
            item: answer("i_6", vec![Part::text("Hello world.")]),
        }),
    ));
    drain(&mut app, now);
    let joined = everything(&mut app, now).join("\n");
    assert!(
        joined.contains("Hello world.") && !joined.contains("world.world."),
        "{joined}"
    );
}

// ------------------------------------------- leaving a session part-way

/// While A's long history is read, the person picks B: the switch waits
/// until A is drawn, so that A's page is never drawn under B's header, nor
/// A's cursor sent for B.
#[test]
fn a_switch_while_a_history_is_read_is_refused_and_the_read_goes_on() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let (read_a, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    pick_second_session(&mut app, &link, now);
    let sent = link.take_sent();
    assert!(
        !sent
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionOpen(_) | Op::SessionClose(_))),
        "{sent:#?}"
    );
    link.give(Inbound::Done(
        read_a,
        Ok(OpResult::SessionRead(SessionReadResult {
            items: vec![user("i_1", "asked in session A")],
            incomplete: Vec::new(),
            next_cursor: Some("500".to_owned()),
            seq: 40,
        })),
    ));
    drain(&mut app, now);
    let sent = link.take_sent();
    assert!(
        sent.iter().any(|(_, op)| matches!(
            op,
            Op::SessionRead(p) if p.session_id == SessionId::from("s_a")
                && p.cursor.as_deref() == Some("500")
        )),
        "the next page is A's: {sent:#?}"
    );
    let all = everything(&mut app, now);
    assert!(
        all.iter()
            .any(|line| line.contains("/sessions can switch once")),
        "{all:#?}"
    );
    assert!(
        !all.iter().any(|line| line.contains("claude-sonnet-5-5")),
        "{all:#?}"
    );
}

/// `--resume A` on a long session: before A's open returns, the person
/// picks B in `/sessions`. The switch waits for A, which would otherwise be
/// opened and never closed.
#[test]
fn a_switch_while_a_session_opens_is_refused() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let _ = app.frame(now);
    app.drawn(now);
    let open_a = ticket(&link.take_sent(), |op| matches!(op, Op::SessionOpen(_)));
    pick_second_session(&mut app, &link, now);
    let sent = link.take_sent();
    assert!(
        !sent.iter().any(|(_, op)| matches!(
            op,
            Op::SessionOpen(o) if o.session_id == SessionId::from("s_b")
        )),
        "{sent:#?}"
    );
    link.give(Inbound::Done(
        open_a,
        Ok(OpResult::SessionOpen(SessionResult {
            session: state_of("s_a", "claude-opus-5-5"),
        })),
    ));
    drain(&mut app, now);
    assert_eq!(
        app.session().map(|s| s.session_id.clone()),
        Some(SessionId::from("s_a"))
    );
}

/// `/new` while the history is read waits for it, as it does for the open.
#[test]
fn new_while_a_history_is_read_says_so() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let _ = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    run_command(&mut app, "/new", now);
    let sent = link.take_sent();
    assert!(
        !sent
            .iter()
            .any(|(_, op)| matches!(op, Op::SessionNew(_) | Op::SessionClose(_))),
        "{sent:#?}"
    );
    assert!(notices(&mut app, now).contains("/new can start another once it is"));
}

/// Opens `s_a` with its history drawn, sends `command` (and `then`'s keys,
/// for a picker), and switches to `s_b`, whose open is still under way.
/// Returns the command's ticket.
fn left_while_waiting(
    app: &mut App,
    link: &Scripted,
    (command, then): (&str, &[&str]),
    now: Instant,
    is_it: impl Fn(&Op) -> bool,
) -> Ticket {
    let (read, _) = resumed(app, link, "s_a", "claude-opus-5-5", now);
    link.give(Inbound::Done(read, Ok(read_result(Vec::new(), &[], 5))));
    drain(app, now);
    let _ = link.take_sent();
    run_command(app, command, now);
    for binding in then {
        key(app, binding, now);
    }
    let asked = ticket(&link.take_sent(), is_it);
    pick_second_session(app, link, now);
    let sent = link.take_sent();
    assert!(
        sent.iter().any(|(_, op)| matches!(
            op,
            Op::SessionOpen(o) if o.session_id == SessionId::from("s_b")
        )),
        "{sent:#?}"
    );
    asked
}

/// `/undo` of A, whose result comes once the person has left A for B: A,
/// closed, is not made the session again, so a prompt waits for B.
#[test]
fn an_undo_of_the_session_left_does_not_make_it_the_session_again() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let undo = left_while_waiting(&mut app, &link, ("/undo", &[]), now, |op| {
        matches!(op, Op::SessionUndo(_))
    });
    link.give(Inbound::Done(
        undo,
        Ok(OpResult::SessionUndo(SessionResult {
            session: state_of("s_a", "claude-opus-5-5"),
        })),
    ));
    drain(&mut app, now);
    assert!(app.session().is_none(), "{:?}", app.session());
    type_text(&mut app, "next", now);
    key(&mut app, "enter", now);
    assert!(turn_start(&link.take_sent()).is_empty(), "held for B");
}

/// A change of A's settings that comes back after the switch to B: the
/// same.
#[test]
fn a_setting_of_the_session_left_does_not_make_it_the_session_again() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let set = left_while_waiting(&mut app, &link, ("/mode", &["down", "enter"]), now, |op| {
        matches!(op, Op::SessionSet(_))
    });
    link.give(Inbound::Done(
        set,
        Ok(OpResult::SessionSet(SessionResult {
            session: state_of("s_a", "claude-opus-5-5"),
        })),
    ));
    drain(&mut app, now);
    assert!(app.session().is_none(), "{:?}", app.session());
}

/// `/status` of A that comes back once B is open is not shown as B's.
#[test]
fn a_status_of_the_session_left_is_not_shown_for_the_next() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let status = left_while_waiting(&mut app, &link, ("/status", &[]), now, |op| {
        matches!(op, Op::ConfigGet(_))
    });
    link.give(Inbound::Done(
        status,
        Ok(OpResult::ConfigGet(ConfigGetResult {
            config: serde_json::Map::new(),
            origins: Default::default(),
            ..Default::default()
        })),
    ));
    drain(&mut app, now);
    assert!(!notices(&mut app, now).contains("Status:"));
}

/// `/new` whose open fails leaves no session open: what the session it
/// closed still sends is not drawn, but what it says is.
#[test]
fn with_no_session_open_only_notices_of_the_one_closed_are_shown() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    run_command(&mut app, "/new", now);
    let new = ticket(&link.take_sent(), |op| matches!(op, Op::SessionNew(_)));
    link.give(Inbound::Done(
        new,
        Err(ErrorData::new(
            ErrorKind::Config,
            "invalid_configuration",
            "The project's configuration is not valid.",
        )),
    ));
    drain(&mut app, now);
    link.give(ev(
        SESSION,
        9,
        Event::ItemCompleted(ItemCompleted {
            item: answer("i_9", vec![Part::text("late from the closed one")]),
        }),
    ));
    link.give(Inbound::Event(SessionEvent {
        session_id: SessionId::from(SESSION),
        seq: 10,
        turn_id: None,
        event: Event::Notice(cineris_protocol::event::Notice::Info(
            cineris_protocol::event::InfoNotice {
                message: "said by the closed one".to_owned(),
            },
        )),
    }));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert!(!all.contains("late from the closed one"), "{all}");
    assert!(all.contains("said by the closed one"), "{all}");
}

// ------------------------------------------------- prompts given back

/// A refused prompt comes back after what the editor holds, and a paste
/// there stays a chip, not its whole text.
#[test]
fn a_prompt_given_back_keeps_the_editors_chips() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    type_text(&mut app, "first", now);
    key(&mut app, "enter", now);
    let prompt = ticket(&link.take_sent(), |op| matches!(op, Op::TurnStart(_)));
    let long = "a pasted line\n".repeat(40);
    app.terminal(&TermEvent::Paste(long.clone()), now);
    assert_eq!(app.input().editor().chips().len(), 1);
    link.give(Inbound::Done(
        prompt,
        Err(ErrorData::new(
            ErrorKind::Usage,
            "history_changing",
            "The session's history is being changed.",
        )),
    ));
    drain(&mut app, now);
    let editor = app.input().editor();
    assert_eq!(editor.chips().len(), 1, "{:?}", editor.text());
    assert_eq!(editor.expanded(), format!("{long}\nfirst"));
}

// -------------------------------------- the command line's slash command

/// `cineris "/compact"` compacts, as typing it does: it is not sent to the
/// model as a prompt, and no turn is shown running.
#[test]
fn a_command_lines_prompt_that_is_a_command_is_done_as_one() {
    let now = Instant::now();
    let (mut app, link) = app_with(now, Some("/compact keep the plan"));
    opened(&mut app, &link, now);
    let sent = link.take_sent();
    assert!(turn_start(&sent).is_empty(), "{sent:#?}");
    assert!(
        sent.iter().any(|(_, op)| matches!(
            op,
            Op::SessionCompact(c) if c.focus.as_deref() == Some("keep the plan")
        )),
        "{sent:#?}"
    );
    assert!(!app.running());
}

/// One this build does not have yet is said, and given back, as typed.
#[test]
fn a_command_lines_prompt_that_is_a_command_not_built_yet_is_given_back() {
    let now = Instant::now();
    let (mut app, link) = app_with(now, Some("/rewind 3"));
    opened(&mut app, &link, now);
    assert!(turn_start(&link.take_sent()).is_empty());
    assert_eq!(app.input().editor().expanded(), "/rewind 3");
    assert!(notices(&mut app, now).contains("/rewind is not in this build yet"));
    assert!(!app.running());
}

// --------------------------- the review of the fixes after step 23's review

/// The binary's link gives an operation's result before the events queued
/// ahead of it: events the read already holds can be taken after the read
/// has ended, and are not drawn a second time.
#[test]
fn events_the_read_holds_that_come_after_its_result_are_not_drawn_twice() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, Some("go on"), Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    // Taken while the read was under way: held.
    link.give(ev("s_a", 6, Event::TurnStarted(TurnStarted {})));
    drain(&mut app, now);
    // The read's result, taken before what was queued ahead of it.
    link.give(Inbound::Done(
        read,
        Ok(read_result(
            vec![
                user("i_5", "go on"),
                answer("i_6", vec![Part::text("Hello ")]),
            ],
            &["i_6"],
            9,
        )),
    ));
    // Queued before the read was taken (up to its `seq`), taken after its
    // result.
    link.give(ev(
        "s_a",
        7,
        Event::ItemCompleted(ItemCompleted {
            item: user("i_5", "go on"),
        }),
    ));
    link.give(ev("s_a", 8, answer_started("i_6")));
    link.give(ev("s_a", 9, delta("i_6", "Hello ")));
    // After the read.
    link.give(ev("s_a", 10, delta("i_6", "world.")));
    link.give(ev(
        "s_a",
        11,
        Event::ItemCompleted(ItemCompleted {
            item: answer("i_6", vec![Part::text("Hello world.")]),
        }),
    ));
    drain(&mut app, now);
    let shown = everything(&mut app, now).join("\n");
    assert!(
        shown.contains("Hello world.") && !shown.contains("Hello Hello"),
        "{shown}"
    );
    assert_eq!(shown.matches("go on").count(), 1, "{shown}");
}

/// Opens and reads `s_a`, switches to `s_b`, gives A's late
/// `session.updated` while B opens when `late_update`, and opens B, whose
/// open says another client's turn runs, with a read that holds no item in
/// progress.
fn switched_to_a_running_session(late_update: bool) -> App {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    link.give(Inbound::Done(
        read,
        Ok(read_result(vec![user("i_1", "hi")], &[], 5)),
    ));
    drain(&mut app, now);
    let _ = link.take_sent();
    pick_second_session(&mut app, &link, now);
    let open_b = ticket(
        &link.take_sent(),
        |op| matches!(op, Op::SessionOpen(o) if o.session_id == SessionId::from("s_b")),
    );
    if late_update {
        // A's title, set after its last turn.
        let mut a_titled = state_of("s_a", "claude-opus-5-5");
        a_titled.title = Some("Fix the build".to_owned());
        link.give(ev(
            "s_a",
            3,
            Event::SessionUpdated(cineris_protocol::event::SessionUpdated { session: a_titled }),
        ));
        drain(&mut app, now);
    }
    let mut b = state_of("s_b", "claude-opus-5-5");
    b.seq = 9;
    b.running_turn = Some(TurnId::from("t_4"));
    link.give(Inbound::Done(
        open_b,
        Ok(OpResult::SessionOpen(SessionResult { session: b })),
    ));
    drain(&mut app, now);
    let read_b = ticket(&link.take_sent(), |op| matches!(op, Op::SessionRead(_)));
    link.give(Inbound::Done(
        read_b,
        Ok(read_result(vec![user("i_1", "go on")], &[], 9)),
    ));
    drain(&mut app, now);
    app
}

/// A late `session.updated` of the session left, held while the next one
/// opens, does not decide whether the next one's turn runs.
#[test]
fn a_late_update_of_the_session_left_does_not_decide_the_next_ones_turn() {
    let app = switched_to_a_running_session(true);
    assert!(app.running(), "B's turn, which its open said runs");
}

/// The control: with no late update, B's turn is taken as running.
#[test]
fn a_turn_the_open_says_runs_is_taken_as_running_at_the_read() {
    let app = switched_to_a_running_session(false);
    assert!(app.running(), "B's turn, which its open said runs");
}

/// An `/undo` of A that fails once the person has left A for B is not said
/// in B.
#[test]
fn an_error_of_the_session_left_is_not_shown_in_the_next() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let undo = left_while_waiting(&mut app, &link, ("/undo", &[]), now, |op| {
        matches!(op, Op::SessionUndo(_))
    });
    link.give(Inbound::Done(
        undo,
        Err(ErrorData::new(
            ErrorKind::Usage,
            "nothing_to_undo",
            "Session A has no turn to undo.",
        )),
    ));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert!(!all.contains("no turn to undo"), "{all}");
}

/// A history whose second page fails: what the first page drew is not
/// drawn again from the events held meanwhile, and the item it drew in
/// progress says a turn runs.
#[test]
fn a_history_whose_page_fails_part_way_draws_nothing_twice() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    link.give(ev(
        "s_a",
        8,
        Event::ItemCompleted(ItemCompleted {
            item: answer("i_2", vec![Part::text("Answered on page one.\n")]),
        }),
    ));
    drain(&mut app, now);
    link.give(Inbound::Done(
        read,
        Ok(OpResult::SessionRead(SessionReadResult {
            items: vec![
                user("i_1", "first page"),
                answer("i_2", vec![Part::text("Answered on page one.\n")]),
                answer("i_3", vec![Part::text("Working ")]),
            ],
            incomplete: vec![ItemId::from("i_3")],
            next_cursor: Some("500".to_owned()),
            seq: 9,
        })),
    ));
    drain(&mut app, now);
    let next = ticket(&link.take_sent(), |op| matches!(op, Op::SessionRead(_)));
    link.give(Inbound::Done(
        next,
        Err(ErrorData::new(
            ErrorKind::Internal,
            "store_failed",
            "The session's log could not be read.",
        )),
    ));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert_eq!(all.matches("Answered on page one.").count(), 1, "{all}");
    assert!(all.contains("could not be read"), "{all}");
    assert!(app.running(), "the item in progress says a turn runs");
}

// ------------------------ the follow-up review of the fixes after step 23

/// A history whose second page fails: what the pages before it did not
/// hold may be on the missing one, so its events are shown, and a call with
/// no result there is not taken for cut off: its result ends it in its box.
#[test]
fn a_history_whose_page_fails_part_way_shows_what_it_does_not_hold_and_cuts_nothing_off() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    link.give(ev(
        "s_a",
        8,
        Event::ItemCompleted(ItemCompleted {
            item: answer("i_9", vec![Part::text("Said on the missing page.\n")]),
        }),
    ));
    drain(&mut app, now);
    link.give(Inbound::Done(
        read,
        Ok(OpResult::SessionRead(SessionReadResult {
            items: vec![
                user("i_1", "check it"),
                answer("i_2", vec![shell_call("c_1")]),
            ],
            incomplete: Vec::new(),
            next_cursor: Some("500".to_owned()),
            seq: 9,
        })),
    ));
    drain(&mut app, now);
    let next = ticket(&link.take_sent(), |op| matches!(op, Op::SessionRead(_)));
    link.give(Inbound::Done(
        next,
        Err(ErrorData::new(
            ErrorKind::Internal,
            "store_failed",
            "The session's log could not be read.",
        )),
    ));
    drain(&mut app, now);
    link.give(ev(
        "s_a",
        10,
        Event::ItemCompleted(ItemCompleted {
            item: shell_result("i_3", "c_1", ToolOutcome::Success),
        }),
    ));
    drain(&mut app, now);
    let all = everything(&mut app, now);
    let joined = all.join("\n");
    assert!(joined.contains("Said on the missing page."), "{joined}");
    let runs = runs(&all);
    assert_eq!(runs.len(), 1, "{all:#?}");
    assert!(runs[0].contains('\u{2713}'), "{all:#?}");
}

/// A turn begun after the open is at a call when the read is taken; its
/// `turn.started` and items come before the read's result, as the binary's
/// link gives them (the follow-up review of step 23's fixes): the turn is
/// taken as running, and the call's result ends it in one box.
#[test]
fn a_call_of_a_turn_begun_after_the_open_ends_in_one_box() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    let call = answer("i_2", vec![shell_call("c_1")]);
    link.give(ev("s_a", 6, Event::TurnStarted(TurnStarted {})));
    link.give(ev(
        "s_a",
        7,
        Event::ItemCompleted(ItemCompleted {
            item: user("i_1", "check it"),
        }),
    ));
    link.give(ev(
        "s_a",
        9,
        Event::ItemCompleted(ItemCompleted { item: call.clone() }),
    ));
    link.give(Inbound::Done(
        read,
        Ok(read_result(vec![user("i_1", "check it"), call], &[], 10)),
    ));
    link.give(ev(
        "s_a",
        11,
        Event::ItemStarted(ItemStarted {
            item_id: ItemId::from("i_3"),
            kind: ItemKind::ToolResult,
            tool: Some("shell".to_owned()),
            call_id: Some(CallId::from("c_1")),
        }),
    ));
    link.give(ev(
        "s_a",
        12,
        Event::ItemCompleted(ItemCompleted {
            item: shell_result("i_3", "c_1", ToolOutcome::Success),
        }),
    ));
    drain(&mut app, now);
    assert!(app.running());
    let all = everything(&mut app, now);
    let runs = runs(&all);
    assert_eq!(runs.len(), 1, "{all:#?}");
    assert!(runs[0].contains('\u{2713}'), "{all:#?}");
}

/// A listing of A's files that comes back once the person has left A for B
/// is not taken for B: B's first `@` asks for B's own.
#[test]
fn a_file_listing_of_the_session_left_is_not_taken_for_the_next() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    link.give(Inbound::Done(read, Ok(read_result(Vec::new(), &[], 5))));
    drain(&mut app, now);
    let _ = link.take_sent();
    type_text(&mut app, "@", now);
    let _ = app.frame(now);
    let listing = ticket(&link.take_sent(), |op| matches!(op, Op::WorkspaceFiles(_)));
    key(&mut app, "backspace", now);
    pick_second_session(&mut app, &link, now);
    let open_b = ticket(
        &link.take_sent(),
        |op| matches!(op, Op::SessionOpen(o) if o.session_id == SessionId::from("s_b")),
    );
    link.give(Inbound::Done(
        listing,
        Ok(OpResult::WorkspaceFiles(
            cineris_protocol::methods::WorkspaceFilesResult {
                paths: vec!["of_a.rs".to_owned()],
                truncated: false,
            },
        )),
    ));
    drain(&mut app, now);
    link.give(Inbound::Done(
        open_b,
        Ok(OpResult::SessionOpen(SessionResult {
            session: state_of("s_b", "claude-opus-5-5"),
        })),
    ));
    drain(&mut app, now);
    let read_b = ticket(&link.take_sent(), |op| matches!(op, Op::SessionRead(_)));
    link.give(Inbound::Done(read_b, Ok(read_result(Vec::new(), &[], 5))));
    drain(&mut app, now);
    type_text(&mut app, "@", now);
    let _ = app.frame(now);
    let sent = link.take_sent();
    assert!(
        sent.iter().any(|(_, op)| matches!(
            op,
            Op::WorkspaceFiles(p) if p.session_id == SessionId::from("s_b")
        )),
        "{sent:#?}"
    );
}

/// A long history's read keeps the ids of the last page's worth of items,
/// not every item it has drawn.
#[test]
fn a_long_historys_read_keeps_only_the_last_pages_worth_of_ids() {
    let now = Instant::now();
    let (mut app, link) = app_opening(now, None, Open::Resume(SessionId::from("s_a")));
    let (read, _) = resumed(&mut app, &link, "s_a", "claude-opus-5-5", now);
    let items: Vec<Item> = (0..700).map(|n| user(&format!("i_{n}"), "asked")).collect();
    link.give(Inbound::Done(
        read,
        Ok(OpResult::SessionRead(SessionReadResult {
            items,
            incomplete: Vec::new(),
            next_cursor: Some("700".to_owned()),
            seq: 9,
        })),
    ));
    drain(&mut app, now);
    assert_eq!(app.history_kept(), 500, "a page of history is 500 items");
}
