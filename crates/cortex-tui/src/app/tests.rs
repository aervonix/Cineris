//! The application driven as the loop drives it, with a scripted link in
//! the core's place: what each key, event and result does, and what the
//! frames show.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cortex_protocol::event::{
    Delta, Event, ItemCompleted, ItemDelta, ItemKind, ItemStarted, RequestResolved, Resolution,
    TextDelta, TurnCompleted, TurnOutcome, TurnStarted,
};
use cortex_protocol::item::{AssistantMessageItem, StopReason, UserMessageItem};
use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::lease::Level;
use cortex_protocol::message::Part;
use cortex_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalRequest, ApprovalResponse, CommandDetail, Disposition,
    Op, OpResult, ServerRequest, ServerResponse, SessionResult, TurnInterruptResult,
    TurnStartResult,
};
use cortex_protocol::model::ModelRole;
use cortex_protocol::session::{RoleState, SandboxStatus, SessionSettings, SessionState};
use cortex_protocol::{
    CallId, ErrorData, ErrorKind, Item, ItemId, ModelId, ProviderId, SessionEvent, SessionId,
    Speed, Timestamp, TurnId,
};
use cortex_term::{Capabilities, Event as TermEvent, Line, Overrides, Platform};

use super::*;
use crate::highlight::Highlighter;
use crate::history::MemoryHistory;
use crate::keys::{KeyMap, parse};

/// What the scripted link has seen, and what it will give.
#[derive(Debug, Default)]
pub(crate) struct Script {
    pub(crate) sent: Vec<(Ticket, Op)>,
    pub(crate) answers: Vec<(RequestId, ServerResponse)>,
    pub(crate) inbound: std::collections::VecDeque<Inbound>,
    next: u64,
}

/// A link that records what is sent and gives what the test queued.
#[derive(Debug, Clone, Default)]
pub(crate) struct Scripted(pub(crate) Arc<Mutex<Script>>);

impl Scripted {
    pub(crate) fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues something from the core.
    pub(crate) fn give(&self, inbound: Inbound) {
        self.script().inbound.push_back(inbound);
    }

    /// The operations sent so far, taken.
    pub(crate) fn take_sent(&self) -> Vec<(Ticket, Op)> {
        std::mem::take(&mut self.script().sent)
    }
}

impl Link for Scripted {
    fn send(&mut self, op: Op) -> Ticket {
        let mut script = self.script();
        script.next += 1;
        let ticket = Ticket(script.next);
        script.sent.push((ticket, op));
        ticket
    }

    fn respond(&mut self, request: RequestId, response: ServerResponse) {
        self.script().answers.push((request, response));
    }

    fn next(&mut self) -> Option<Inbound> {
        self.script().inbound.pop_front()
    }
}

/// A modern terminal, as the tests draw for.
pub(crate) fn caps() -> Capabilities {
    Capabilities::detect(
        |name| match name {
            "TERM" => Some("xterm-256color".to_owned()),
            "COLORTERM" => Some("truecolor".to_owned()),
            "LANG" => Some("C.UTF-8".to_owned()),
            _ => None,
        },
        Platform::Unix,
        Overrides::default(),
    )
}

pub(crate) const SESSION: &str = "s_test";

/// An application with a new session to open, and its link.
pub(crate) fn app(now: Instant) -> (App, Scripted) {
    app_with(now, None)
}

pub(crate) fn app_with(now: Instant, prompt: Option<&str>) -> (App, Scripted) {
    let open = Open::New {
        workspace: "/work/app".to_owned(),
        settings: SessionSettings::default(),
        ephemeral: false,
    };
    app_opening(now, prompt, open)
}

/// An application that opens `open`, and its link.
pub(crate) fn app_opening(now: Instant, prompt: Option<&str>, open: Open) -> (App, Scripted) {
    let link = Scripted::default();
    let start = Start {
        version: "0.1.0".to_owned(),
        folder: "~/src/app".to_owned(),
        workspace: "/work/app".to_owned(),
        open,
        prompt: prompt.map(str::to_owned),
        settings: Settings::default(),
        keymap: KeyMap::defaults(),
        notices: Vec::new(),
        history: Box::new(MemoryHistory::new()),
        highlighter: Highlighter::off(),
    };
    let app = App::new(Box::new(link.clone()), start, &caps(), 99, 40, now);
    (app, link)
}

pub(crate) fn state() -> SessionState {
    let mut roles = std::collections::BTreeMap::new();
    roles.insert(
        ModelRole::Main,
        RoleState {
            provider: ProviderId::from("anthropic"),
            model: ModelId::from("claude-opus-5-5"),
            effort: Some("high".to_owned()),
            effort_in_effect: Some("high".to_owned()),
            speed: Speed::Standard,
            speed_in_effect: Speed::Standard,
            speed_cooldown_until: None,
        },
    );
    SessionState {
        session_id: SessionId::from(SESSION),
        workspace: "/work/app".to_owned(),
        title: None,
        created_at: Timestamp::from("2026-11-15T10:00:00Z"),
        parent: None,
        level: Level::Supervised,
        lease_id: None,
        sandbox: SandboxStatus::Off,
        speed: Speed::Standard,
        roles,
        seq: 0,
        running_turn: None,
        queue_length: 0,
        write_roots: vec!["/work/app".to_owned()],
    }
}

pub(crate) fn event(event: Event) -> Inbound {
    Inbound::Event(SessionEvent {
        session_id: SessionId::from(SESSION),
        seq: 0,
        turn_id: Some(TurnId::from("t_1")),
        event,
    })
}

/// Draws the first frame and answers the open: the app has its session.
pub(crate) fn opened(app: &mut App, link: &Scripted, now: Instant) {
    opened_with(app, link, now, state());
}

/// As [`opened`], the session being `session`.
pub(crate) fn opened_with(app: &mut App, link: &Scripted, now: Instant, session: SessionState) {
    let _ = app.frame(now);
    app.drawn(now);
    let sent = link.take_sent();
    let (ticket, _) = sent
        .iter()
        .find(|(_, op)| matches!(op, Op::SessionNew(_)))
        .expect("the session is opened after the first frame");
    link.give(Inbound::Done(
        *ticket,
        Ok(OpResult::SessionNew(SessionResult { session })),
    ));
    drain(app, now);
}

pub(crate) fn drain(app: &mut App, now: Instant) {
    while app.drain(now, BATCH).more {}
}

pub(crate) fn key(app: &mut App, binding: &str, at: Instant) {
    app.terminal(&TermEvent::Key(parse(binding).expect("a key")), at);
}

pub(crate) fn type_text(app: &mut App, text: &str, at: Instant) {
    for c in text.chars() {
        app.terminal(
            &TermEvent::Key(cortex_term::Key::plain(cortex_term::KeyCode::Char(c))),
            at,
        );
    }
}

pub(crate) fn text(lines: &[Line]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line.text().trim_end().to_owned())
        .collect()
}

/// Every line the frames committed, and the live region now.
pub(crate) fn screen(app: &mut App, now: Instant, scrollback: &mut Vec<String>) -> Vec<String> {
    let (commit, live, _) = app.frame(now);
    scrollback.extend(text(&commit));
    text(&live)
}

fn turn_start(sent: &[(Ticket, Op)]) -> Vec<&cortex_protocol::methods::TurnStartParams> {
    sent.iter()
        .filter_map(|(_, op)| match op {
            Op::TurnStart(params) => Some(params),
            _ => None,
        })
        .collect()
}

/// A command that would run outside the sandbox, and may be allowed for
/// longer.
fn outside(call: &str) -> ServerRequest {
    let ServerRequest::Approval(mut request) = approval(call) else {
        unreachable!("an approval");
    };
    request.detail = ApprovalDetail::Command(CommandDetail {
        command: "cargo publish --dry-run".to_owned(),
        cwd: "/work/app".to_owned(),
        outside_sandbox: true,
    });
    request.choices.push(ApprovalChoice::AllowProject);
    ServerRequest::Approval(request)
}

/// The answers given so far, taken.
fn answers(link: &Scripted) -> Vec<ApprovalChoice> {
    std::mem::take(&mut link.script().answers)
        .into_iter()
        .map(|(_, response)| match response {
            ServerResponse::Approval(approval) => approval.choice,
            other => panic!("not an approval's answer: {other:?}"),
        })
        .collect()
}

/// Gives the application a request, and draws its dialog's first frame at
/// `now`.
fn ask(app: &mut App, link: &Scripted, id: i64, request: ServerRequest, now: Instant) {
    link.give(Inbound::Request(RequestId::Number(id), request));
    drain(app, now);
    let _ = app.frame(now);
    app.drawn(now);
}

fn approval(call: &str) -> ServerRequest {
    ServerRequest::Approval(ApprovalRequest {
        session_id: SessionId::from(SESSION),
        turn_id: TurnId::from("t_1"),
        call_id: CallId::from(call),
        tool: "shell".to_owned(),
        summary: "Run cargo build".to_owned(),
        detail: ApprovalDetail::Command(CommandDetail {
            command: "cargo build".to_owned(),
            cwd: "/work/app".to_owned(),
            outside_sandbox: false,
        }),
        choices: vec![
            ApprovalChoice::AllowOnce,
            ApprovalChoice::AllowSession,
            ApprovalChoice::Deny,
        ],
        reasons: vec!["`cargo build` is not on the safe list.".to_owned()],
        adds: Vec::new(),
    })
}

#[test]
fn the_session_is_opened_after_the_first_frame_and_its_header_comes_first() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    assert!(
        link.take_sent().is_empty(),
        "nothing before the first frame"
    );
    let _ = app.frame(now);
    app.drawn(now);
    let sent = link.take_sent();
    let open = sent
        .iter()
        .find(|(_, op)| matches!(op, Op::SessionNew(_)))
        .map(|(ticket, _)| *ticket)
        .expect("the open");
    // A notice of the assembly arrives before the open's result: it is held,
    // so that the header is the first block.
    link.give(event(Event::Notice(cortex_protocol::event::Notice::Info(
        cortex_protocol::event::InfoNotice {
            message: "This project is not trusted.".to_owned(),
        },
    ))));
    drain(&mut app, now);
    link.give(Inbound::Done(
        open,
        Ok(OpResult::SessionNew(SessionResult { session: state() })),
    ));
    drain(&mut app, now);
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, now, &mut scrollback);
    let header = scrollback
        .iter()
        .position(|line| line.contains("Cortex-Harness"))
        .expect("the header");
    let notice = scrollback
        .iter()
        .position(|line| line.contains("not trusted"))
        .expect("the notice");
    assert!(header < notice, "{scrollback:#?}");
}

#[test]
fn a_prompt_before_the_session_is_open_is_held_and_sent_once_it_is() {
    let now = Instant::now();
    let (mut app, link) = app_with(now, Some("from the command line"));
    assert!(app.running(), "a prompt given is a turn on its way");
    type_text(&mut app, "and this", now);
    key(&mut app, "enter", now);
    assert!(turn_start(&link.script().sent).is_empty());
    opened(&mut app, &link, now);
    let sent = link.take_sent();
    let prompts = turn_start(&sent);
    assert_eq!(prompts.len(), 2, "{sent:#?}");
    assert_eq!(prompts[0].input, vec![Part::text("from the command line")]);
    assert_eq!(prompts[0].delivery, None);
    // The second came while the first was on its way: it waits for it.
    assert_eq!(prompts[1].input, vec![Part::text("and this")]);
    assert_eq!(
        prompts[1].delivery,
        Some(cortex_protocol::item::Delivery::Queue)
    );
}

#[test]
fn enter_sends_a_prompt_and_then_steers_and_tab_queues() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "fix the build", now);
    key(&mut app, "enter", now);
    assert!(app.running(), "running from the moment it is sent");
    type_text(&mut app, "also clippy", now);
    key(&mut app, "enter", now);
    type_text(&mut app, "then docs", now);
    key(&mut app, "tab", now);
    key(&mut app, "enter", now);
    let sent = link.take_sent();
    let prompts = turn_start(&sent);
    let deliveries: Vec<_> = prompts.iter().map(|p| p.delivery.clone()).collect();
    assert_eq!(
        deliveries,
        [
            None,
            Some(cortex_protocol::item::Delivery::Steer),
            Some(cortex_protocol::item::Delivery::Queue),
        ]
    );
}

#[test]
fn esc_right_after_enter_interrupts_and_a_second_within_two_seconds_stops_now() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "go", now);
    key(&mut app, "enter", now);
    key(&mut app, "esc", now);
    key(&mut app, "ctrl+c", now + Duration::from_millis(1500));
    key(&mut app, "esc", now + Duration::from_millis(5000));
    let interrupts: Vec<bool> = link
        .take_sent()
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::TurnInterrupt(params) => Some(params.now),
            _ => None,
        })
        .collect();
    assert_eq!(interrupts, [false, true, false]);
}

#[test]
fn a_refused_prompt_comes_back_into_the_editor_and_says_why() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "keep me", now);
    key(&mut app, "enter", now);
    let (ticket, _) = link.take_sent().pop().expect("the prompt");
    link.give(Inbound::Done(
        ticket,
        Err(ErrorData::new(
            ErrorKind::Usage,
            "history_changing",
            "The session's history is being changed.",
        )),
    ));
    drain(&mut app, now);
    assert!(!app.running());
    assert_eq!(app.input().editor().expanded(), "keep me");
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, now, &mut scrollback);
    assert!(
        scrollback.iter().any(|line| line.contains("send it again")),
        "{scrollback:#?}"
    );
}

#[test]
fn steering_that_was_never_delivered_comes_back_into_the_editor() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "go", now);
    key(&mut app, "enter", now);
    key(&mut app, "esc", now);
    let (ticket, _) = link.take_sent().pop().expect("the interrupt");
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::TurnInterrupt(TurnInterruptResult {
            interrupted: true,
            undelivered: vec![vec![Part::text("the steer")]],
        })),
    ));
    drain(&mut app, now);
    assert_eq!(app.input().editor().expanded(), "the steer");
}

#[test]
fn a_dialog_that_appears_while_the_person_types_takes_keys_only_after_a_grace() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    link.give(Inbound::Request(RequestId::Number(7), approval("c_1")));
    drain(&mut app, now);
    // Typed before the dialog was drawn: the editor's.
    type_text(&mut app, "y", now);
    let _ = app.frame(now);
    app.drawn(now);
    // Typed in the grace after it: still the editor's.
    type_text(&mut app, "yes", now + Duration::from_millis(100));
    assert!(
        link.script().answers.is_empty(),
        "typing answered the dialog"
    );
    assert_eq!(app.input().editor().expanded(), "yyes");
    // The answers are drawn at the end of the grace; a paste never answers.
    let _ = app.frame(now + dialog::GRACE * 2);
    app.terminal(&TermEvent::Paste("y".to_owned()), now + dialog::GRACE * 2);
    assert!(link.script().answers.is_empty());
    // A key after the grace answers.
    key(&mut app, "y", now + dialog::GRACE * 2);
    let answers = std::mem::take(&mut link.script().answers);
    assert_eq!(
        answers,
        [(
            RequestId::Number(7),
            ServerResponse::Approval(ApprovalResponse {
                choice: ApprovalChoice::AllowOnce,
                note: None,
            })
        )]
    );
}

/// Whether or not anyone was typing, a dialog takes no key in its grace,
/// and shows its answers only when they can be pressed: the loop is woken
/// for that.
#[test]
fn a_dialog_takes_keys_and_shows_its_answers_only_after_its_grace() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    link.give(Inbound::Request(RequestId::Number(8), approval("c_1")));
    drain(&mut app, now);
    assert_eq!(app.deadline(), None, "no grace before it is drawn");
    let rows = text(&app.frame(now).1);
    assert!(
        rows.iter().any(|row| row.contains("Run cargo build?")),
        "{rows:#?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("Allow once")),
        "{rows:#?}"
    );
    app.drawn(now);
    assert_eq!(app.deadline(), Some(now + dialog::GRACE));
    key(&mut app, "y", now + dialog::GRACE / 2);
    assert!(answers(&link).is_empty(), "a key in the grace answered");
    let end = now + dialog::GRACE;
    let rows = text(&app.frame(end).1);
    assert!(
        rows.iter().any(|row| row.contains("y Allow once")),
        "{rows:#?}"
    );
    assert_eq!(app.deadline(), None, "nothing more to wake for");
    // After a pause since the key before, a key answers.
    key(&mut app, "y", end + dialog::GRACE);
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

#[test]
fn typing_that_goes_on_past_the_grace_never_answers_a_dialog() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    ask(&mut app, &link, 9, approval("c_1"), now);
    // A key every 100 ms for a second, each frame drawn: the grace ends,
    // and the answers are drawn, in the middle of it.
    let mut at = now;
    for _ in 0..10 {
        at += Duration::from_millis(100);
        key(&mut app, "y", at);
        let _ = app.frame(at);
    }
    assert!(answers(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), "yyyyyyyyyy");
    key(&mut app, "n", at + dialog::GRACE);
    assert_eq!(answers(&link), [ApprovalChoice::Deny]);
}

/// In a session whose commands are confined, a command outside the sandbox
/// is a request to leave it: once, or not at all.
#[test]
fn leaving_a_confined_sessions_sandbox_offers_only_once_and_deny() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    let mut confined = state();
    confined.sandbox = SandboxStatus::Enforcing;
    opened_with(&mut app, &link, now, confined);
    ask(&mut app, &link, 10, outside("c_1"), now);
    let at = now + dialog::GRACE;
    let rows = text(&app.frame(at).1);
    assert!(
        rows.iter()
            .any(|row| row.contains("Run outside the sandbox?")),
        "{rows:#?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("y Run once outside the sandbox")),
        "{rows:#?}"
    );
    assert!(
        !rows
            .iter()
            .any(|row| row.contains("Allow this session") || row.contains("Deny with a note")),
        "{rows:#?}"
    );
    key(&mut app, "s", at);
    key(&mut app, "a", at + dialog::GRACE);
    key(&mut app, "e", at + dialog::GRACE * 2);
    assert!(answers(&link).is_empty());
    key(&mut app, "y", at + dialog::GRACE * 3);
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// With the sandbox off every command runs outside it: that is asked as
/// any other call is.
#[test]
fn a_command_in_a_session_with_the_sandbox_off_may_be_allowed_for_longer() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    ask(&mut app, &link, 11, outside("c_1"), now);
    let at = now + dialog::GRACE;
    let rows = text(&app.frame(at).1);
    assert!(
        rows.iter().any(|row| row.contains("Run cargo build?")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("s Allow this session")),
        "{rows:#?}"
    );
    key(&mut app, "s", at);
    assert_eq!(answers(&link), [ApprovalChoice::AllowSession]);
}

/// A dialog taller than the window gives way in its detail: its question
/// stays at the top, the rest is said, and only this call can be allowed.
#[test]
fn a_dialog_taller_than_the_window_keeps_its_question_and_offers_only_this_call() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(99, 20);
    let ServerRequest::Approval(mut request) = approval("c_1") else {
        unreachable!("an approval");
    };
    let script: Vec<String> = (1..=80).map(|line| format!("echo {line}")).collect();
    // Reasons enough that the dialog drawn at its first size is too tall.
    request.reasons.extend(
        [
            "It runs a script.",
            "The script is long.",
            "It was not seen before.",
        ]
        .map(str::to_owned),
    );
    request.detail = ApprovalDetail::Command(CommandDetail {
        command: script.join("\n"),
        cwd: "/work/app".to_owned(),
        outside_sandbox: false,
    });
    ask(&mut app, &link, 12, ServerRequest::Approval(request), now);
    let at = now + dialog::GRACE;
    let rows = text(&app.frame(at).1);
    assert!(rows.len() <= 20, "{} rows:\n{rows:#?}", rows.len());
    assert!(
        rows.iter().any(|row| row.contains("? Run cargo build?")),
        "{rows:#?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("more lines are not shown")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("y Allow once")),
        "{rows:#?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("Allow this session")),
        "{rows:#?}"
    );
    key(&mut app, "s", at);
    assert!(answers(&link).is_empty());
    key(&mut app, "y", at + dialog::GRACE);
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// Shows a dialog, and draws the frame with its answers as the loop does at
/// the end of its grace: the time a key may answer it.
fn shown_dialog(app: &mut App, link: &Scripted, id: i64, now: Instant) -> Instant {
    link.give(Inbound::Request(RequestId::Number(id), approval("c_1")));
    drain(app, now);
    let _ = app.frame(now);
    app.drawn(now);
    let at = now + dialog::GRACE * 2;
    let _ = app.frame(at);
    at
}

#[test]
fn esc_denies_and_e_denies_with_a_note() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let at = shown_dialog(&mut app, &link, 1, now);
    key(&mut app, "esc", at);
    let at = shown_dialog(&mut app, &link, 2, at);
    key(&mut app, "e", at);
    type_text(&mut app, "use make", at);
    key(&mut app, "enter", at);
    let answers = std::mem::take(&mut link.script().answers);
    assert_eq!(
        answers,
        [
            (
                RequestId::Number(1),
                ServerResponse::Approval(ApprovalResponse {
                    choice: ApprovalChoice::Deny,
                    note: None,
                })
            ),
            (
                RequestId::Number(2),
                ServerResponse::Approval(ApprovalResponse {
                    choice: ApprovalChoice::Deny,
                    note: Some("use make".to_owned()),
                })
            ),
        ]
    );
}

#[test]
fn a_choice_not_on_offer_is_not_taken() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let at = shown_dialog(&mut app, &link, 3, now);
    // The request offers no "always allow here".
    key(&mut app, "a", at);
    assert!(link.script().answers.is_empty());
    key(&mut app, "s", at + dialog::GRACE);
    assert_eq!(link.script().answers.len(), 1);
}

#[test]
fn a_request_settled_elsewhere_closes_its_dialog() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let at = shown_dialog(&mut app, &link, 4, now);
    let (_, live, _) = app.frame(at);
    assert!(
        text(&live)
            .iter()
            .any(|line| line.contains("Run cargo build?"))
    );
    link.give(event(Event::RequestResolved(RequestResolved {
        request_id: RequestId::Number(4),
        resolution: Resolution::Cancelled,
    })));
    drain(&mut app, at);
    let (_, live, _) = app.frame(at);
    assert!(
        !text(&live)
            .iter()
            .any(|line| line.contains("Run cargo build?")),
        "{:#?}",
        text(&live)
    );
    // The dialog's keys are gone with it: `y` is typing again.
    key(&mut app, "y", at);
    assert!(link.script().answers.is_empty());
}

#[test]
fn a_turn_shows_the_message_the_answer_and_its_end() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    for inbound in [
        event(Event::TurnStarted(TurnStarted {})),
        event(Event::ItemCompleted(ItemCompleted {
            item: Item::UserMessage(UserMessageItem {
                id: ItemId::from("i_1"),
                parts: vec![Part::text("why does it fail?")],
                delivery: None,
                created_at: None,
            }),
        })),
        event(Event::ItemStarted(ItemStarted {
            item_id: ItemId::from("i_2"),
            kind: ItemKind::AssistantMessage,
            tool: None,
            call_id: None,
        })),
        event(Event::ItemDelta(ItemDelta {
            item_id: ItemId::from("i_2"),
            delta: Delta::Text(TextDelta {
                text: "It fails because the feature is off.\n".to_owned(),
            }),
        })),
        event(Event::ItemCompleted(ItemCompleted {
            item: Item::AssistantMessage(AssistantMessageItem {
                id: ItemId::from("i_2"),
                parts: vec![Part::text("It fails because the feature is off.\n")],
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
        event(Event::TurnCompleted(TurnCompleted {
            outcome: TurnOutcome::Completed,
            stop_reason: Some(StopReason::End),
            error: None,
            usage: Default::default(),
            verification: None,
            duration_ms: Some(1200),
        })),
    ] {
        link.give(inbound);
    }
    drain(&mut app, now);
    assert!(!app.running());
    let mut scrollback = Vec::new();
    let live = screen(&mut app, now, &mut scrollback);
    let all = scrollback.join("\n");
    assert!(all.contains("why does it fail?"), "{all}");
    assert!(
        all.contains("It fails because the feature is off."),
        "{all}"
    );
    assert!(
        live.iter().any(|line| line.contains("Describe a task")),
        "{live:#?}"
    );
    assert_eq!(app.last_answer(), "It fails because the feature is off.\n");
}

#[test]
fn a_failed_turn_says_what_went_wrong_and_what_to_do() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    link.give(event(Event::TurnStarted(TurnStarted {})));
    link.give(event(Event::TurnCompleted(TurnCompleted {
        outcome: TurnOutcome::Failed,
        stop_reason: None,
        error: Some(ErrorData::new(
            ErrorKind::Auth,
            "missing_key",
            "No key for anthropic.",
        )),
        usage: Default::default(),
        verification: None,
        duration_ms: None,
    })));
    drain(&mut app, now);
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, now, &mut scrollback);
    let all = scrollback.join("\n");
    assert!(all.contains("No key for anthropic."), "{all}");
    assert!(all.contains("cortex auth login"), "{all}");
}

#[test]
fn the_hint_row_follows_the_turn() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let (_, live, _) = app.frame(now);
    let hint = text(&live).last().cloned().unwrap_or_default();
    assert!(hint.contains("opus-5.5 high"), "{hint}");
    type_text(&mut app, "go", now);
    key(&mut app, "enter", now);
    let (_, live, _) = app.frame(now + Duration::from_secs(14));
    let hint = text(&live).last().cloned().unwrap_or_default();
    assert!(hint.contains("14s"), "{hint}");
    assert!(hint.contains("esc stops"), "{hint}");
    // The box says what typing does now.
    assert!(
        text(&live)
            .iter()
            .any(|line| line.contains("Type to steer this turn")),
        "{live:#?}"
    );
}

#[test]
fn quitting_with_ctrl_d_or_quit_or_twice_ctrl_c() {
    let now = Instant::now();
    for keys in [&["ctrl+d"][..], &["ctrl+c", "ctrl+c"]] {
        let (mut app, link) = app(now);
        opened(&mut app, &link, now);
        for binding in keys {
            assert!(app.exit().is_none());
            key(&mut app, binding, now);
        }
        assert_eq!(app.exit(), Some(&Exit::Quit), "{keys:?}");
    }
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "/quit", now);
    // `Enter` takes the menu's row first, then sends.
    key(&mut app, "enter", now);
    key(&mut app, "enter", now);
    assert_eq!(app.exit(), Some(&Exit::Quit));
    assert!(
        turn_start(&link.take_sent()).is_empty(),
        "/quit is no prompt"
    );
}

#[test]
fn the_core_going_away_ends_the_application() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let gone = ErrorData::new(ErrorKind::Internal, "core_gone", "The core stopped.");
    link.give(Inbound::Gone(gone.clone()));
    drain(&mut app, now);
    assert_eq!(app.exit(), Some(&Exit::Gone(gone)));
}

#[test]
fn a_prompt_that_starts_with_a_slash_and_is_no_command_is_a_prompt() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "/usr/bin is missing", now);
    // The command menu opened on `/`; it is closed first.
    key(&mut app, "esc", now);
    key(&mut app, "enter", now);
    let sent = link.take_sent();
    let prompts = turn_start(&sent);
    assert_eq!(prompts.len(), 1, "{sent:#?}");
    assert_eq!(prompts[0].input, vec![Part::text("/usr/bin is missing")]);
}

#[test]
fn compact_sends_its_focus() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "/compact keep the lexer", now);
    key(&mut app, "esc", now);
    key(&mut app, "enter", now);
    let sent = link.take_sent();
    let focus = sent.iter().find_map(|(_, op)| match op {
        Op::SessionCompact(params) => Some(params.focus.clone()),
        _ => None,
    });
    assert_eq!(focus, Some(Some("keep the lexer".to_owned())));
}

#[test]
fn the_live_region_is_at_most_half_the_window() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(99, 20);
    link.give(event(Event::ItemStarted(ItemStarted {
        item_id: ItemId::from("i_2"),
        kind: ItemKind::AssistantMessage,
        tool: None,
        call_id: None,
    })));
    // A table holds its rows live until it ends: a tall live region.
    let mut table = String::from("| a | b |\n|---|---|\n");
    for row in 0..30 {
        table.push_str(&format!("| {row} | x |\n"));
    }
    link.give(event(Event::ItemDelta(ItemDelta {
        item_id: ItemId::from("i_2"),
        delta: Delta::Text(TextDelta { text: table }),
    })));
    drain(&mut app, now);
    let (_, live, _) = app.frame(now);
    assert!(live.len() <= 10, "{} rows:\n{:#?}", live.len(), text(&live));
}

#[test]
fn turn_start_results_are_quiet_and_a_disposition_counts_as_a_turn() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "go", now);
    key(&mut app, "enter", now);
    let (ticket, _) = link.take_sent().pop().expect("the prompt");
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::TurnStart(TurnStartResult {
            turn_id: TurnId::from("t_1"),
            disposition: Disposition::Started,
        })),
    ));
    drain(&mut app, now);
    assert!(app.running());
}

#[test]
fn prompts_held_for_a_session_that_does_not_open_come_back_with_why() {
    let now = Instant::now();
    let (mut app, link) = app_with(now, Some("from the command line"));
    type_text(&mut app, "and this", now);
    key(&mut app, "enter", now);
    let _ = app.frame(now);
    app.drawn(now);
    let (ticket, _) = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, Op::SessionNew(_)))
        .expect("the open");
    link.give(Inbound::Done(
        ticket,
        Err(ErrorData::new(
            ErrorKind::Config,
            "invalid_configuration",
            "The project's configuration is not valid.",
        )),
    ));
    drain(&mut app, now);
    assert!(!app.running(), "no spinner for a turn that never starts");
    assert_eq!(
        app.input().editor().expanded(),
        "from the command line\nand this"
    );
    // With no session and none on its way, a prompt is given back.
    key(&mut app, "enter", now);
    assert!(turn_start(&link.take_sent()).is_empty());
    assert_eq!(
        app.input().editor().expanded(),
        "from the command line\nand this"
    );
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, now, &mut scrollback);
    let all = scrollback.join("\n");
    assert!(all.contains("configuration is not valid"), "{all}");
    assert!(all.contains("/new starts one"), "{all}");
}

#[test]
fn stopping_before_the_session_is_open_gives_the_held_prompts_back() {
    let now = Instant::now();
    let (mut app, link) = app_with(now, Some("from the command line"));
    key(&mut app, "esc", now);
    assert!(!app.running());
    assert_eq!(app.input().editor().expanded(), "from the command line");
    opened(&mut app, &link, now);
    assert!(turn_start(&link.take_sent()).is_empty(), "nothing is sent");
}

#[test]
fn new_closes_the_session_and_opens_another_in_the_workspace() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    type_text(&mut app, "/new", now);
    key(&mut app, "enter", now);
    key(&mut app, "enter", now);
    let sent = link.take_sent();
    assert!(
        sent.iter().any(|(_, op)| matches!(
            op,
            Op::SessionClose(close) if close.session_id == SessionId::from(SESSION)
        )),
        "{sent:#?}"
    );
    let (ticket, workspace) = sent
        .iter()
        .find_map(|(ticket, op)| match op {
            Op::SessionNew(params) => Some((*ticket, params.workspace.clone())),
            _ => None,
        })
        .expect("a new session");
    assert_eq!(workspace, "/work/app");
    let mut next = state();
    next.session_id = SessionId::from("s_next");
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::SessionNew(SessionResult { session: next })),
    ));
    drain(&mut app, now);
    assert_eq!(
        app.session().map(|session| session.session_id.clone()),
        Some(SessionId::from("s_next"))
    );
    // Not while a turn runs.
    type_text(&mut app, "go", now);
    key(&mut app, "enter", now);
    let _ = link.take_sent();
    type_text(&mut app, "/new", now);
    key(&mut app, "enter", now);
    key(&mut app, "enter", now);
    let sent = link.take_sent();
    assert!(
        !sent.iter().any(|(_, op)| matches!(op, Op::SessionNew(_))),
        "{sent:#?}"
    );
}

/// A session is forked from its log: it is opened, forked, and closed once
/// the fork is open.
#[test]
fn a_fork_opens_its_session_forks_it_and_closes_the_original() {
    let now = Instant::now();
    let original = SessionId::from("s_old");
    let (mut app, link) = app_opening(now, None, Open::Fork(original.clone()));
    let _ = app.frame(now);
    app.drawn(now);
    let sent = link.take_sent();
    let opened_first = sent
        .iter()
        .find_map(|(ticket, op)| match op {
            Op::SessionOpen(open) if open.session_id == original => Some(*ticket),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the original is opened first: {sent:#?}"));
    let mut old = state();
    old.session_id = original.clone();
    link.give(Inbound::Done(
        opened_first,
        Ok(OpResult::SessionOpen(SessionResult { session: old })),
    ));
    drain(&mut app, now);
    assert!(app.session().is_none(), "the original is not the session");
    let sent = link.take_sent();
    let fork = sent
        .iter()
        .find_map(|(ticket, op)| match op {
            Op::SessionFork(fork) if fork.session_id == original => Some(*ticket),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the fork follows: {sent:#?}"));
    link.give(Inbound::Done(
        fork,
        Ok(OpResult::SessionFork(SessionResult { session: state() })),
    ));
    drain(&mut app, now);
    assert_eq!(
        app.session().map(|session| session.session_id.clone()),
        Some(SessionId::from(SESSION))
    );
    let sent = link.take_sent();
    assert!(
        sent.iter().any(|(_, op)| matches!(
            op,
            Op::SessionClose(close) if close.session_id == original
        )),
        "{sent:#?}"
    );
}

#[test]
fn a_continue_whose_list_fails_says_why() {
    let now = Instant::now();
    let open = Open::Continue {
        workspace: "/work/app".to_owned(),
        settings: SessionSettings::default(),
        ephemeral: false,
    };
    let (mut app, link) = app_opening(now, Some("go on"), open);
    let _ = app.frame(now);
    app.drawn(now);
    let (ticket, _) = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, Op::SessionList(_)))
        .expect("the list");
    link.give(Inbound::Done(
        ticket,
        Err(ErrorData::new(
            ErrorKind::Internal,
            "store_unreadable",
            "The session store cannot be read.",
        )),
    ));
    drain(&mut app, now);
    assert!(!app.running());
    assert_eq!(app.input().editor().expanded(), "go on");
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, now, &mut scrollback);
    let all = scrollback.join("\n");
    assert!(all.contains("session store cannot be read"), "{all}");
}

/// A command of 09-tui that this build does not have is said to be
/// missing: it is no prompt for the model.
#[test]
fn a_command_not_in_this_build_is_said_and_not_sent() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    type_text(&mut app, "/model opus", now);
    key(&mut app, "esc", now);
    key(&mut app, "enter", now);
    assert!(turn_start(&link.take_sent()).is_empty());
    assert_eq!(app.input().editor().expanded(), "/model opus");
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, now, &mut scrollback);
    assert!(
        scrollback
            .iter()
            .any(|line| line.contains("/model is not in this build yet")),
        "{scrollback:#?}"
    );
}

/// Each wait of a [`Played`] terminal: its timeout, and how many bytes had
/// been written before it.
type Waits = Arc<Mutex<Vec<(Option<Duration>, usize)>>>;

/// A terminal for the loop's tests: it plays back steps on a clock of its
/// own, which a wait that times out moves on by the wait's length, so that
/// the loop's waits are tested without waiting.
struct Played {
    steps: std::collections::VecDeque<(cortex_term::Ready, Vec<u8>)>,
    written: Arc<Mutex<Vec<u8>>>,
    waits: Waits,
    base: Instant,
    elapsed: Duration,
}

impl cortex_term::Device for Played {
    fn size(&self) -> std::io::Result<(u16, u16)> {
        Ok((100, 30))
    }
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(())
    }
    fn wait(
        &mut self,
        timeout: Option<Duration>,
        into: &mut Vec<u8>,
    ) -> std::io::Result<cortex_term::Ready> {
        let written = self
            .written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((timeout, written));
        let Some((ready, bytes)) = self.steps.pop_front() else {
            return Err(std::io::Error::other("the script has ended"));
        };
        // A wait with no limit that "times out" stands for a pause of a
        // second: the person stopped typing.
        self.elapsed += match ready {
            cortex_term::Ready::Timeout => timeout.unwrap_or(Duration::from_secs(1)),
            _ => Duration::from_millis(1),
        };
        into.extend_from_slice(&bytes);
        Ok(ready)
    }
    fn waker(&self) -> cortex_term::Waker {
        cortex_term::Waker::new(|| {})
    }
    fn suspend(&mut self) {}
    fn leave(&mut self) {}
    fn now(&self) -> Instant {
        self.base + self.elapsed
    }
}

/// The loop itself: woken by the core, it draws a dialog without its
/// answers, sleeps exactly to the end of the grace, draws them, waits with
/// no timer while idle, and stops reading once the application ends.
#[test]
fn the_loop_wakes_for_a_dialogs_answers_and_sleeps_while_idle() {
    use cortex_term::Ready;
    let base = Instant::now();
    let (mut app, link) = app(base);
    // The open is the first thing sent: its result, then a request.
    link.give(Inbound::Done(
        Ticket(1),
        Ok(OpResult::SessionNew(SessionResult { session: state() })),
    ));
    link.give(Inbound::Request(RequestId::Number(7), approval("c_1")));
    let written = Arc::new(Mutex::new(Vec::new()));
    let waits = Arc::new(Mutex::new(Vec::new()));
    let device = Played {
        steps: [
            // The core's waker.
            (Ready::Woken, Vec::new()),
            // The grace's end.
            (Ready::Timeout, Vec::new()),
            (Ready::Input, b"y".to_vec()),
            (Ready::Timeout, Vec::new()),
            // Ctrl+D quits; what follows it is never read.
            (Ready::Input, b"\x04".to_vec()),
            (Ready::Input, b"left for the shell".to_vec()),
        ]
        .into_iter()
        .collect(),
        written: Arc::clone(&written),
        waits: Arc::clone(&waits),
        base,
        elapsed: Duration::ZERO,
    };
    let mut terminal = cortex_term::Terminal::with_device(device, caps()).expect("the terminal");
    run(&mut terminal, &mut app).expect("the loop ends");
    assert_eq!(app.exit(), Some(&Exit::Quit));
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);

    let written = written
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let waits = waits.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let before = |index: usize| String::from_utf8_lossy(&written[..waits[index].1]).into_owned();
    assert_eq!(waits.len(), 5, "the last input is not read: {waits:?}");
    // Before anything arrived: idle, no timer.
    assert_eq!(waits[0].0, None, "{waits:?}");
    // The dialog drawn without its answers, and a wait to the grace's end.
    assert_eq!(waits[1].0, Some(dialog::GRACE), "{waits:?}");
    assert!(before(1).contains("Run cargo build?"));
    assert!(!before(1).contains("Allow once"));
    // Its answers drawn then, and an idle wait for the key.
    assert!(before(2).contains("Allow once"), "{}", before(2));
    assert_eq!(waits[2].0, None, "{waits:?}");
    assert_eq!(waits[4].0, None, "{waits:?}");
}

/// The grace is over, but the frame with the answers was not drawn yet: a
/// key cannot answer what was not shown.
#[test]
fn a_key_before_the_answers_are_drawn_never_answers() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    ask(&mut app, &link, 13, approval("c_1"), now);
    // A frame in the grace, as the spinner draws them: still no answers.
    let _ = app.frame(now + dialog::GRACE / 2);
    let late = now + dialog::GRACE * 2;
    assert_eq!(app.deadline(), Some(now + dialog::GRACE), "a frame is due");
    key(&mut app, "y", late);
    assert!(answers(&link).is_empty());
    let _ = app.frame(late);
    assert_eq!(app.deadline(), None);
    key(&mut app, "y", late + dialog::GRACE);
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// `Esc` pressed in a dialog's grace was meant for the dialog: it does not
/// stop the turn, and it does not deny either.
#[test]
fn esc_in_a_dialogs_grace_neither_denies_nor_stops_the_turn() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "go", now);
    key(&mut app, "enter", now);
    let _ = link.take_sent();
    ask(&mut app, &link, 14, approval("c_1"), now);
    key(&mut app, "esc", now + dialog::GRACE / 3);
    let sent = link.take_sent();
    assert!(
        !sent
            .iter()
            .any(|(_, op)| matches!(op, Op::TurnInterrupt(_))),
        "{sent:#?}"
    );
    assert!(answers(&link).is_empty());
    assert!(app.running());
}

/// The case the review drew: 80 by 24, two reasons, a note of what a
/// lasting answer allows, and a command of 13 lines. The note gives way, the
/// command is shown whole, and only this call can be allowed.
#[test]
fn a_dialog_one_row_too_tall_drops_the_lasting_note_and_keeps_its_question() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(80, 24);
    let ServerRequest::Approval(mut request) = approval("c_1") else {
        unreachable!("an approval");
    };
    request
        .reasons
        .push("It writes outside the project's own files.".to_owned());
    request.adds = vec![cortex_protocol::methods::AddedRule {
        effect: cortex_protocol::lease::Effect::Exec,
        pattern: format!("cargo build --release --target {} *", "x".repeat(40)),
    }];
    let script: Vec<String> = (1..=13).map(|line| format!("echo {line}")).collect();
    request.detail = ApprovalDetail::Command(CommandDetail {
        command: script.join("\n"),
        cwd: "/work/app".to_owned(),
        outside_sandbox: false,
    });
    ask(&mut app, &link, 15, ServerRequest::Approval(request), now);
    let at = now + dialog::GRACE;
    let rows = text(&app.frame(at).1);
    assert!(rows.len() <= 24, "{} rows:\n{rows:#?}", rows.len());
    assert!(rows[0].contains("? Run cargo build?"), "{rows:#?}");
    for line in 1..=13 {
        assert!(
            rows.contains(&format!("    echo {line}")),
            "echo {line}: {rows:#?}"
        );
    }
    assert!(
        !rows
            .iter()
            .any(|row| row.contains("Allowing it for longer")),
        "{rows:#?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("Allow this session")),
        "{rows:#?}"
    );
    key(&mut app, "s", at);
    assert!(answers(&link).is_empty());
}

/// In a window too small for any detail, the question and the answers are
/// what is kept, and only this call can be allowed.
#[test]
fn in_a_tiny_window_a_dialog_keeps_its_question_and_its_answers() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(60, 8);
    ask(&mut app, &link, 16, approval("c_1"), now);
    let at = now + dialog::GRACE;
    let rows = text(&app.frame(at).1);
    assert!(rows.len() <= 8, "{} rows:\n{rows:#?}", rows.len());
    assert!(rows[0].contains("? Run cargo build?"), "{rows:#?}");
    assert!(
        rows.iter().any(|row| row.contains("y Allow once")),
        "{rows:#?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("Allow this session")),
        "{rows:#?}"
    );
    key(&mut app, "s", at);
    assert!(answers(&link).is_empty());
    key(&mut app, "y", at + dialog::GRACE);
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// `/new` opens its session as the command line asked for the first: in
/// memory only after `--no-session`, with the same settings.
#[test]
fn new_keeps_what_the_command_line_asked_for() {
    let now = Instant::now();
    let settings = SessionSettings {
        speed: Some(Speed::Fast),
        ..SessionSettings::default()
    };
    let open = Open::New {
        workspace: "/work/app".to_owned(),
        settings: settings.clone(),
        ephemeral: true,
    };
    let (mut app, link) = app_opening(now, None, open);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    type_text(&mut app, "/new", now);
    key(&mut app, "enter", now);
    key(&mut app, "enter", now);
    let sent = link.take_sent();
    let params = sent
        .iter()
        .find_map(|(_, op)| match op {
            Op::SessionNew(params) => Some(params.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a new session: {sent:#?}"));
    assert!(params.ephemeral, "{params:#?}");
    assert_eq!(params.settings, settings);
}

#[test]
fn new_while_a_session_is_being_opened_says_so() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    for _ in 0..2 {
        type_text(&mut app, "/new", now);
        key(&mut app, "enter", now);
        key(&mut app, "enter", now);
    }
    let sent = link.take_sent();
    let opens = sent
        .iter()
        .filter(|(_, op)| matches!(op, Op::SessionNew(_)))
        .count();
    assert_eq!(opens, 1, "{sent:#?}");
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, now, &mut scrollback);
    assert!(
        scrollback
            .iter()
            .any(|line| line.contains("A session is being opened")),
        "{scrollback:#?}"
    );
}

/// A fork that fails after its original was opened closes the original,
/// says why, and takes nothing of the original's for its own session.
#[test]
fn a_fork_that_fails_closes_its_original_and_takes_nothing_of_it() {
    let now = Instant::now();
    let original = SessionId::from("s_old");
    let (mut app, link) = app_opening(now, Some("go"), Open::Fork(original.clone()));
    let _ = app.frame(now);
    app.drawn(now);
    let (ticket, _) = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, Op::SessionOpen(_)))
        .expect("the original's open");
    let mut old = state();
    old.session_id = original.clone();
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::SessionOpen(SessionResult {
            session: old.clone(),
        })),
    ));
    drain(&mut app, now);
    // The original's own events, while the fork is on its way: an answer
    // streaming in it is none of the fork's.
    let of_original = |seq: u64, event: Event| {
        Inbound::Event(SessionEvent {
            session_id: original.clone(),
            seq,
            turn_id: Some(TurnId::from("t_old")),
            event,
        })
    };
    link.give(of_original(
        1,
        Event::ItemStarted(ItemStarted {
            item_id: ItemId::from("i_old"),
            kind: ItemKind::AssistantMessage,
            tool: None,
            call_id: None,
        }),
    ));
    link.give(of_original(
        2,
        Event::ItemDelta(ItemDelta {
            item_id: ItemId::from("i_old"),
            delta: Delta::Text(TextDelta {
                text: "from the original\n\n".to_owned(),
            }),
        }),
    ));
    drain(&mut app, now);
    let (fork, _) = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, Op::SessionFork(_)))
        .expect("the fork");
    link.give(Inbound::Done(
        fork,
        Err(ErrorData::new(
            ErrorKind::Usage,
            "session_locked",
            "The session is open elsewhere.",
        )),
    ));
    drain(&mut app, now);
    assert!(app.session().is_none(), "{:?}", app.session());
    assert!(!app.running());
    assert_eq!(app.input().editor().expanded(), "go");
    let sent = link.take_sent();
    assert!(
        sent.iter().any(|(_, op)| matches!(
            op,
            Op::SessionClose(close) if close.session_id == original
        )),
        "{sent:#?}"
    );
    // A late update of the original is not taken for a session of this
    // client's.
    link.give(of_original(
        3,
        Event::SessionUpdated(cortex_protocol::event::SessionUpdated { session: old }),
    ));
    drain(&mut app, now);
    assert!(app.session().is_none(), "{:?}", app.session());
    let mut scrollback = Vec::new();
    let live = screen(&mut app, now, &mut scrollback);
    assert!(
        scrollback
            .iter()
            .any(|line| line.contains("open elsewhere")),
        "{scrollback:#?}"
    );
    let all = [scrollback, live].concat().join("\n");
    assert!(!all.contains("from the original"), "{all}");
}

/// An approval that offers every answer there is.
fn every_answer(call: &str) -> ServerRequest {
    let ServerRequest::Approval(mut request) = approval(call) else {
        unreachable!("an approval");
    };
    request.choices.push(ApprovalChoice::AllowProject);
    ServerRequest::Approval(request)
}

/// The rows of the frame at `at`, and that none is wider than `width`.
fn rows_within(app: &mut App, at: Instant, width: usize) -> Vec<String> {
    let (_, live, _) = app.frame(at);
    for line in &live {
        assert!(
            line.width() <= width,
            "{} > {width}: {:?}",
            line.width(),
            line.text()
        );
    }
    text(&live)
}

/// In a narrow window the answers wrap onto rows of their own, each drawn
/// whole, and each can be pressed.
#[test]
fn in_a_narrow_window_the_answers_wrap_and_each_can_be_pressed() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(39, 30);
    ask(&mut app, &link, 20, every_answer("c_1"), now);
    let at = now + dialog::GRACE;
    let rows = rows_within(&mut app, at, 39);
    for answer in [
        "y Allow once",
        "s Allow this session",
        "a Always allow here",
        "n Deny",
        "e Deny with a note",
    ] {
        assert!(
            rows.iter().any(|row| row.contains(answer)),
            "{answer}: {rows:#?}"
        );
    }
    key(&mut app, "a", at);
    assert_eq!(answers(&link), [ApprovalChoice::AllowProject]);
}

/// Narrower still, an answer that cannot be drawn whole cannot be pressed,
/// and no lasting answer is offered.
#[test]
fn an_answer_not_drawn_whole_cannot_be_pressed() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(16, 30);
    ask(&mut app, &link, 21, every_answer("c_1"), now);
    let at = now + dialog::GRACE;
    let rows = rows_within(&mut app, at, 16);
    assert!(
        rows.iter().any(|row| row.contains("y Allow once")),
        "{rows:#?}"
    );
    key(&mut app, "s", at);
    key(&mut app, "a", at + dialog::GRACE);
    key(&mut app, "e", at + dialog::GRACE * 2);
    assert!(answers(&link).is_empty());
    key(&mut app, "y", at + dialog::GRACE * 3);
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// The question is wrapped, never cut, however narrow the window.
#[test]
fn a_long_question_wraps_and_is_never_cut() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(30, 30);
    let ServerRequest::Approval(mut request) = approval("c_1") else {
        unreachable!("an approval");
    };
    request.summary = "Edit crates/cortex-tui/src/app/dialog/tests.rs".to_owned();
    ask(&mut app, &link, 22, ServerRequest::Approval(request), now);
    let rows = rows_within(&mut app, now + dialog::GRACE, 30);
    assert!(rows[0].starts_with("? Edit"), "{rows:#?}");
    assert!(
        rows.iter().any(|row| row.contains("tests.rs?")),
        "{rows:#?}"
    );
    assert!(!rows[0].contains('…'), "{rows:#?}");
}

/// A detail that fits is shown whole, and a lasting answer offered: the
/// detail gives way only to what does not fit.
#[test]
fn a_detail_that_fits_is_shown_whole_with_every_answer() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(80, 24);
    let ServerRequest::Approval(mut request) = approval("c_1") else {
        unreachable!("an approval");
    };
    request.reasons.clear();
    let script: Vec<String> = (1..=16).map(|line| format!("echo {line}")).collect();
    request.detail = ApprovalDetail::Command(CommandDetail {
        command: script.join("\n"),
        cwd: "/work/app".to_owned(),
        outside_sandbox: false,
    });
    ask(&mut app, &link, 23, ServerRequest::Approval(request), now);
    let at = now + dialog::GRACE;
    let rows = rows_within(&mut app, at, 80);
    assert!(rows.len() <= 24, "{} rows:\n{rows:#?}", rows.len());
    for line in 1..=16 {
        assert!(
            rows.contains(&format!("    echo {line}")),
            "echo {line}: {rows:#?}"
        );
    }
    assert!(
        rows.iter().any(|row| row.contains("s Allow this session")),
        "{rows:#?}"
    );
    key(&mut app, "s", at);
    assert_eq!(answers(&link), [ApprovalChoice::AllowSession]);
}

/// A window too small for the question and the answers together takes no
/// answer, and nothing keeps the loop busy; a larger one shows them again.
#[test]
fn a_window_too_small_for_the_question_and_its_answers_takes_none() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(60, 1);
    ask(&mut app, &link, 24, approval("c_1"), now);
    let at = now + dialog::GRACE;
    let rows = rows_within(&mut app, at, 60);
    assert!(rows.len() <= 1, "{rows:#?}");
    assert_eq!(app.deadline(), None, "no wake for answers that cannot show");
    key(&mut app, "y", at);
    assert!(answers(&link).is_empty());
    app.resize(60, 24);
    let later = at + dialog::GRACE;
    let rows = rows_within(&mut app, later, 60);
    assert!(
        rows.iter().any(|row| row.contains("y Allow once")),
        "{rows:#?}"
    );
    key(&mut app, "y", later + dialog::GRACE);
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// While a note is written, a paste is the note's: it is sent with the
/// denial, and nothing is left in the editor.
#[test]
fn a_paste_while_a_note_is_written_is_the_notes() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let at = shown_dialog(&mut app, &link, 25, now);
    key(&mut app, "e", at);
    app.terminal(&TermEvent::Paste("use trash, not rm".to_owned()), at);
    key(&mut app, "enter", at);
    let given = std::mem::take(&mut link.script().answers);
    assert_eq!(
        given,
        [(
            RequestId::Number(25),
            ServerResponse::Approval(ApprovalResponse {
                choice: ApprovalChoice::Deny,
                note: Some("use trash, not rm".to_owned()),
            })
        )]
    );
    assert_eq!(app.input().editor().expanded(), "");
}

/// A long note shows its end, where the typing is.
#[test]
fn a_long_note_shows_its_end() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    app.resize(50, 30);
    let at = shown_dialog(&mut app, &link, 26, now);
    key(&mut app, "e", at);
    type_text(
        &mut app,
        "please use the trash command and never rm the build folder",
        at,
    );
    let rows = rows_within(&mut app, at, 50);
    let note = rows
        .iter()
        .find(|row| row.contains("Note for the model"))
        .cloned()
        .unwrap_or_default();
    assert!(note.ends_with("folder"), "{note}");
}

/// `Esc` read less than the grace after the key before it was typing that
/// carried on: it neither denies nor stops the turn. After a pause it
/// denies.
#[test]
fn esc_without_a_pause_after_the_answers_are_drawn_does_nothing() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "go", now);
    key(&mut app, "enter", now);
    let _ = link.take_sent();
    let at = shown_dialog(&mut app, &link, 27, now + dialog::GRACE * 4);
    key(&mut app, "x", at);
    key(&mut app, "esc", at + dialog::GRACE / 3);
    let sent = link.take_sent();
    assert!(
        !sent
            .iter()
            .any(|(_, op)| matches!(op, Op::TurnInterrupt(_))),
        "{sent:#?}"
    );
    assert!(answers(&link).is_empty());
    key(&mut app, "esc", at + dialog::GRACE * 2);
    assert_eq!(answers(&link), [ApprovalChoice::Deny]);
}

/// The look changed (a reply to a query of the terminal's): the dialog is
/// drawn again in it, not taken from what was drawn before.
#[test]
fn a_dialog_is_drawn_again_when_the_look_changes() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let at = shown_dialog(&mut app, &link, 28, now);
    let before = app.frame(at).1;
    let mono = Capabilities::detect(|_| None, Platform::Unix, Overrides::default());
    app.set_caps(&mono);
    let after = app.frame(at).1;
    assert_eq!(text(&before), text(&after));
    assert_ne!(before, after, "the rows kept the old look");
}

/// Keys that come in one read were typed together: none of them answers,
/// even the first after a long quiet. A key after a pause does.
#[test]
fn keys_read_together_never_answer_a_dialog() {
    use cortex_term::Ready;
    let base = Instant::now();
    let (mut app, link) = app(base);
    link.give(Inbound::Done(
        Ticket(1),
        Ok(OpResult::SessionNew(SessionResult { session: state() })),
    ));
    link.give(Inbound::Request(RequestId::Number(7), approval("c_1")));
    let waits = Arc::new(Mutex::new(Vec::new()));
    let device = Played {
        steps: [
            (Ready::Woken, Vec::new()),
            (Ready::Timeout, Vec::new()),
            // "sp", in one read: typing.
            (Ready::Input, b"sp".to_vec()),
            (Ready::Timeout, Vec::new()),
            // A pause, then a key alone.
            (Ready::Timeout, Vec::new()),
            (Ready::Input, b"y".to_vec()),
            (Ready::Timeout, Vec::new()),
        ]
        .into_iter()
        .collect(),
        written: Arc::new(Mutex::new(Vec::new())),
        waits: Arc::clone(&waits),
        base,
        elapsed: Duration::ZERO,
    };
    let mut terminal = cortex_term::Terminal::with_device(device, caps()).expect("the terminal");
    // The script's end ends the loop: the editor holds "sp", so Ctrl+D
    // would delete, not quit.
    let ended = run(&mut terminal, &mut app);
    assert!(
        matches!(&ended, Err(error) if error.to_string() == "the script has ended"),
        "{ended:?}"
    );
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
    assert_eq!(app.input().editor().expanded(), "sp");
}

fn resource(
    kind: cortex_protocol::methods::TrustResourceKind,
    name: &str,
    detail: &str,
    changed: bool,
) -> cortex_protocol::methods::TrustResource {
    cortex_protocol::methods::TrustResource {
        kind,
        name: name.to_owned(),
        detail: detail.to_owned(),
        changed,
    }
}

fn trust_request(resources: Vec<cortex_protocol::methods::TrustResource>) -> ServerRequest {
    ServerRequest::Trust(cortex_protocol::methods::TrustRequest {
        session_id: SessionId::from(SESSION),
        workspace: "~/src/app".to_owned(),
        resources,
    })
}

fn project_resources() -> Vec<cortex_protocol::methods::TrustResource> {
    use cortex_protocol::methods::TrustResourceKind as Kind;
    vec![
        resource(
            Kind::Repository,
            "git",
            "Git may read this repository's own configuration.",
            false,
        ),
        resource(
            Kind::Rules,
            "permissions.rules",
            "allow write .github/**",
            false,
        ),
        resource(
            Kind::Rules,
            "profiles.ci.permissions.rules",
            "allow exec make deploy",
            false,
        ),
        resource(
            Kind::Settings,
            "limits.turn.time",
            "limits.turn.time = \"0s\" (a first limit, where you have none: it could stop every turn)",
            false,
        ),
    ]
}

/// The trust question, asked while the session is being opened: it lists
/// what trust turns on, each rule with its profile, and takes the owner's
/// three answers by their keys, `Esc` being not now.
#[test]
fn the_trust_question_lists_what_trust_turns_on_and_takes_three_answers() {
    use cortex_protocol::methods::TrustResponse;
    let trusted = TrustResponse {
        trusted: true,
        never: false,
    };
    let never = TrustResponse {
        trusted: false,
        never: true,
    };
    for (binding, expected) in [
        ("y", trusted),
        ("n", TrustResponse::default()),
        ("x", never),
        ("esc", TrustResponse::default()),
    ] {
        let now = Instant::now();
        let (mut app, link) = app(now);
        let _ = app.frame(now);
        app.drawn(now);
        ask(&mut app, &link, 30, trust_request(project_resources()), now);
        let at = now + dialog::GRACE;
        let rows = text(&app.frame(at).1);
        for shown in [
            "? Trust this project?",
            "~/src/app",
            "git",
            "rule     allow write .github/**",
            "allow exec make deploy (profile ci)",
            "setting  limits.turn.time = \"0s\"",
            "y Trust it",
            "n Not now",
            "x Never for this project",
            "esc is not now",
        ] {
            assert!(
                rows.iter().any(|row| row.contains(shown)),
                "{shown}: {rows:#?}"
            );
        }
        key(&mut app, binding, at);
        let given = std::mem::take(&mut link.script().answers);
        assert_eq!(
            given,
            [(RequestId::Number(30), ServerResponse::Trust(expected))],
            "{binding}"
        );
    }
}

/// A list of what trust turns on that does not fit is cut, and the project
/// cannot be trusted from there: not now and never stay.
#[test]
fn a_trust_question_that_does_not_fit_cannot_be_trusted_from_there() {
    use cortex_protocol::methods::TrustResourceKind as Kind;
    let now = Instant::now();
    let (mut app, link) = app(now);
    app.resize(80, 14);
    let _ = app.frame(now);
    app.drawn(now);
    let rules: Vec<_> = (1..=30)
        .map(|n| {
            resource(
                Kind::Rules,
                "permissions.rules",
                &format!("allow exec tool{n} *"),
                false,
            )
        })
        .collect();
    ask(&mut app, &link, 31, trust_request(rules), now);
    let at = now + dialog::GRACE;
    let rows = text(&app.frame(at).1);
    assert!(rows.len() <= 14, "{rows:#?}");
    assert!(rows[0].contains("Trust this project?"), "{rows:#?}");
    assert!(
        rows.iter()
            .any(|row| row.contains("cannot be trusted from here")),
        "{rows:#?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("Trust it")),
        "{rows:#?}"
    );
    key(&mut app, "y", at);
    assert!(link.script().answers.is_empty());
    key(&mut app, "x", at + dialog::GRACE);
    assert_eq!(
        std::mem::take(&mut link.script().answers),
        [(
            RequestId::Number(31),
            ServerResponse::Trust(cortex_protocol::methods::TrustResponse {
                trusted: false,
                never: true,
            })
        )]
    );
}

/// A question of the core's, an undo's about files edited since: its
/// choices by their digits, `Esc` no answer.
#[test]
fn a_question_is_answered_by_the_digit_of_its_choice_or_left_unanswered() {
    use cortex_protocol::methods::{QuestionRequest, QuestionResponse};
    let request = || {
        ServerRequest::Question(QuestionRequest {
            session_id: SessionId::from(SESSION),
            turn_id: None,
            question: "This file was changed after the turn that changed it: `a.rs`. Put it back?"
                .to_owned(),
            choices: vec![
                "Put them back".to_owned(),
                "Keep them as they are".to_owned(),
            ],
            extension: None,
        })
    };
    for (binding, answer) in [
        ("2", Some("Keep them as they are".to_owned())),
        ("1", Some("Put them back".to_owned())),
        ("esc", None),
    ] {
        let now = Instant::now();
        let (mut app, link) = app(now);
        opened(&mut app, &link, now);
        ask(&mut app, &link, 32, request(), now);
        let at = now + dialog::GRACE;
        let rows = text(&app.frame(at).1);
        assert!(
            rows.iter().any(|row| row.contains("Put it back?")),
            "{rows:#?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("1 Put them back   2 Keep them as they are")),
            "{rows:#?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("esc leaves it unanswered")),
            "{rows:#?}"
        );
        // A digit with no choice does nothing.
        key(&mut app, "9", at);
        assert!(link.script().answers.is_empty());
        key(&mut app, binding, at + dialog::GRACE);
        assert_eq!(
            std::mem::take(&mut link.script().answers),
            [(
                RequestId::Number(32),
                ServerResponse::Question(QuestionResponse { answer })
            )],
            "{binding}"
        );
    }
}

/// A question with no choices is answered in words.
#[test]
fn a_question_with_no_choices_is_answered_in_words() {
    use cortex_protocol::methods::{QuestionRequest, QuestionResponse};
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    ask(
        &mut app,
        &link,
        33,
        ServerRequest::Question(QuestionRequest {
            session_id: SessionId::from(SESSION),
            turn_id: None,
            question: "Which colour?".to_owned(),
            choices: Vec::new(),
            extension: None,
        }),
        now,
    );
    let at = now + dialog::GRACE;
    let rows = text(&app.frame(at).1);
    assert!(rows.iter().any(|row| row.contains("y Answer")), "{rows:#?}");
    key(&mut app, "y", at);
    type_text(&mut app, "blue", at);
    app.terminal(&TermEvent::Paste(" and green".to_owned()), at);
    key(&mut app, "enter", at);
    assert_eq!(
        std::mem::take(&mut link.script().answers),
        [(
            RequestId::Number(33),
            ServerResponse::Question(QuestionResponse {
                answer: Some("blue and green".to_owned())
            })
        )]
    );
}
