//! The dialogs' key rule after the review of step 23 (`super::super::dialog`,
//! "Only keys meant for it answer it"): typing, a paste, a held key and the
//! terminal's own repeats never answer a dialog; one key does once the
//! person has stopped; the answers are drawn only when they can be pressed;
//! two dialogs at once; a note cut off by its request's end; `Ctrl+C` over a
//! dialog; and the viewer's paste and scroll.

use super::*;
use crate::app::dialog::{GRACE, QUIET, REPEAT};

/// `n` milliseconds after `now`.
fn ms(now: Instant, n: u64) -> Instant {
    now + Duration::from_millis(n)
}

/// One key read at `at`, and the frame the loop draws for it.
fn press(app: &mut App, binding: &str, at: Instant) {
    key(app, binding, at);
    let _ = app.frame(at);
    app.drawn(at);
}

/// Typing `text` from `from`, a key every `every` milliseconds, each with
/// its frame. Returns when the last key was read.
fn typing(app: &mut App, text: &str, from: Instant, every: u64) -> Instant {
    let mut at = from;
    for c in text.chars() {
        at += Duration::from_millis(every);
        app.terminal(
            &TermEvent::Key(cortex_term::Key::plain(cortex_term::KeyCode::Char(c))),
            at,
        );
        let _ = app.frame(at);
        app.drawn(at);
    }
    at
}

/// The loop's frame at the dialog's moment, as it is woken for it: the
/// moment the dialog is ready, or `None` when nothing is due.
fn wake(app: &mut App) -> Option<Instant> {
    let due = app.deadline()?;
    let _ = app.frame(due);
    app.drawn(due);
    Some(due)
}

/// Whether the frame at `at` shows the approval's answers row.
fn answers_shown(app: &mut App, at: Instant) -> bool {
    let (_, live, _) = app.frame(at);
    text(&live).iter().any(|line| line.contains("Allow once"))
}

/// A session open with a turn running, and an approval whose first frame
/// was drawn at `at`.
fn approval_up(at: Instant) -> (App, Scripted) {
    let (mut app, link) = app(at);
    opened(&mut app, &link, at);
    type_text(&mut app, "hi", at);
    key(&mut app, "enter", at);
    link.take_sent();
    ask(&mut app, &link, 7, approval("c_7"), at);
    (app, link)
}

#[test]
fn in_the_ordinary_case_one_key_answers_once_the_grace_has_passed() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    assert!(
        !answers_shown(&mut app, ms(now, 10)),
        "answers in the grace"
    );
    assert_eq!(app.deadline(), Some(now + GRACE));
    let ready = wake(&mut app).expect("a frame at the grace's end");
    assert!(answers_shown(&mut app, ready));
    press(&mut app, "y", ready + Duration::from_millis(500));
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// The review's trigger: "try a smaller fix" at a key every 350 ms, slower
/// than the 300 ms pause, typed on through an approval's arrival.
#[test]
fn steady_typing_slower_than_the_pause_never_answers() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let last = typing(&mut app, "try a smaller fix", now, 350);
    // The loop's wake for the dialog, all along: nothing is ready.
    assert!(
        app.deadline().is_some_and(|due| due >= last + QUIET),
        "the dialog was ready while the person typed"
    );
    assert!(answers(&link).is_empty(), "typing answered the dialog");
    // All of it reached the editor, where it was going.
    assert_eq!(app.input().editor().expanded(), "try a smaller fix");
    assert!(!answers_shown(&mut app, last));
}

/// The review's trigger: typing with an `e` in it would open "deny with a
/// note", and the `Enter` meant to send it would deny with the rest.
#[test]
fn steady_typing_with_an_e_never_becomes_a_deny_note() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let last = typing(&mut app, "please check it", now, 350);
    press(&mut app, "enter", last + Duration::from_millis(350));
    assert!(answers(&link).is_empty(), "typing answered the dialog");
}

/// Typing stops: one second with no input later the answers are drawn, and
/// one key answers. A key just before the second is still typing.
#[test]
fn once_the_typing_stops_for_a_second_one_key_answers() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let last = typing(&mut app, "go on", now, 150);
    assert_eq!(app.deadline(), Some(last + QUIET));
    assert!(!answers_shown(
        &mut app,
        last + QUIET - Duration::from_millis(1)
    ));
    // A key a moment before the quiet is over is typing: it goes on to the
    // editor and the quiet starts again.
    let early = last + QUIET - Duration::from_millis(50);
    press(&mut app, "y", early);
    assert!(answers(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), "go ony");
    let ready = wake(&mut app).expect("a frame when the quiet is over");
    assert_eq!(ready, early + QUIET);
    assert!(answers_shown(&mut app, ready));
    press(&mut app, "s", ready + Duration::from_millis(400));
    assert_eq!(answers(&link), [ApprovalChoice::AllowSession]);
}

/// Answers drawn, then a paste or a key that goes to the editor: the
/// answers are hidden again, never shown while the dialog would drop a key.
#[test]
fn answers_shown_are_hidden_again_by_input_that_reaches_the_editor() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    assert!(answers_shown(&mut app, ready));
    // Keys read together are typing: they go to the editor.
    let at = ready + Duration::from_millis(500);
    app.burst(at);
    press(&mut app, "x", at);
    assert!(!answers_shown(&mut app, at), "answers shown over typing");
    assert!(answers(&link).is_empty());
    let ready = wake(&mut app).expect("the answers again");
    assert_eq!(ready, at + QUIET);
    assert!(answers_shown(&mut app, ready));
}

/// The review's trigger: a key held down from inside the grace. Its first
/// repeat, 500 ms after the press, comes once the answers would be drawn.
#[test]
fn a_held_key_whose_first_repeat_comes_after_the_grace_never_answers() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let pressed = ms(now, 50);
    press(&mut app, "a", pressed);
    let mut at = pressed + Duration::from_millis(500);
    for _ in 0..20 {
        press(&mut app, "a", at);
        let _ = wake(&mut app);
        at += Duration::from_millis(33);
    }
    assert!(answers(&link).is_empty(), "a held key answered");
}

/// A key held from before the dialog came, whose first repeat comes after
/// its answers are drawn: the same key as the one before, typing, within
/// the repeat window. Released and pressed again after it, it answers.
#[test]
fn a_key_held_from_before_the_dialog_never_answers_and_a_press_after_does() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "hi", now);
    key(&mut app, "enter", now);
    // `a` pressed and held; the approval comes 100 ms later.
    press(&mut app, "a", ms(now, 1000));
    ask(&mut app, &link, 7, outside("c_7"), ms(now, 1100));
    let ready = wake(&mut app).expect("the answers' frame");
    assert_eq!(ready, ms(now, 1100) + GRACE);
    // The first repeat, 500 ms after the press, and more.
    let mut at = ms(now, 1500);
    for _ in 0..10 {
        press(&mut app, "a", at);
        at += Duration::from_millis(33);
    }
    assert!(answers(&link).is_empty(), "a held key answered");
    // Released; pressed again after a pause longer than a repeat's delay.
    press(&mut app, "a", at + REPEAT);
    assert_eq!(answers(&link), [ApprovalChoice::AllowProject]);
}

/// A repeat the terminal marks (the enhanced protocol's event types, win32
/// input) answers nothing, and goes nowhere once the dialog takes keys.
#[test]
fn a_repeat_the_terminal_marks_never_answers() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    let y = parse("y").unwrap();
    for n in 0..5 {
        let at = ready + Duration::from_millis(400 + 33 * n);
        app.terminal(&TermEvent::Repeat(y), at);
    }
    assert!(answers(&link).is_empty(), "a repeat answered");
    assert_eq!(app.input().editor().expanded(), "", "a repeat was typed");
    assert!(answers_shown(&mut app, ready + Duration::from_millis(600)));
}

/// The review's trigger: typing, an approval, a paste, and a key 100 ms
/// after the paste.
#[test]
fn a_key_soon_after_a_paste_does_not_answer() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    let pasted = ready + Duration::from_millis(500);
    app.terminal(&TermEvent::Paste("bug in parser.rs ".to_owned()), pasted);
    let _ = app.frame(pasted);
    assert!(
        !answers_shown(&mut app, pasted),
        "answers shown over a paste"
    );
    press(&mut app, "a", pasted + Duration::from_millis(100));
    assert!(answers(&link).is_empty(), "a key after a paste answered");
    // The paste reached the editor, and so did the key.
    assert_eq!(app.input().editor().expanded(), "bug in parser.rs a");
}

/// A key 200 ms after the dialog's first frame is still in its grace: it
/// goes to the editor (300 ms is the grace, and nothing shorter).
#[test]
fn a_key_inside_the_grace_goes_to_the_editor() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    // A frame inside the grace, as a spinner draws them: still no answers.
    let _ = app.frame(ms(now, 200));
    app.drawn(ms(now, 200));
    press(&mut app, "y", ms(now, 200));
    assert!(answers(&link).is_empty());
    assert_eq!(app.input().editor().expanded(), "y");
}

/// A paste while a dialog is up (no note being written) is not lost: it
/// goes to the editor.
#[test]
fn a_paste_while_a_dialog_is_up_reaches_the_editor() {
    let now = Instant::now();
    let (mut app, _link) = approval_up(now);
    app.terminal(&TermEvent::Paste("kept".to_owned()), ms(now, 50));
    assert_eq!(app.input().editor().expanded(), "kept");
}

/// After the program is continued (`fg`), a dialog starts its grace again:
/// a key typed ahead at the shell's prompt does not answer it.
#[test]
fn after_fg_the_dialog_starts_its_grace_again() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    let resumed = ready + Duration::from_secs(5);
    app.terminal(&TermEvent::Resumed, resumed);
    assert!(!answers_shown(&mut app, resumed));
    press(&mut app, "y", resumed);
    assert!(answers(&link).is_empty(), "a key at once after fg answered");
    let ready = wake(&mut app).expect("the answers again");
    assert!(ready >= resumed + GRACE);
}

/// After the external editor gives the terminal back, likewise.
#[test]
fn after_the_external_editor_the_dialog_starts_its_grace_again() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    app.edited(Ok(crate::external::Edited::Unchanged));
    let back = ready + Duration::from_secs(5);
    let _ = app.frame(back);
    app.drawn(back);
    press(&mut app, "y", back + Duration::from_millis(10));
    assert!(
        answers(&link).is_empty(),
        "a key at once after the editor answered"
    );
    assert!(app.deadline().is_some_and(|due| due > back));
}

/// Two dialogs at once: a key answers the one shown, the first; a
/// resolution of another request leaves it shown and answerable.
#[test]
fn with_two_dialogs_a_key_answers_the_one_shown_and_another_ones_end_leaves_it() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    link.give(Inbound::Request(RequestId::Number(8), approval("c_8")));
    drain(&mut app, now);
    // Request 8 is settled elsewhere, and so is one this client never had.
    for settled in [8, 99] {
        link.give(event(Event::RequestResolved(RequestResolved {
            request_id: RequestId::Number(settled),
            resolution: Resolution::Answered,
        })));
    }
    drain(&mut app, now);
    let ready = wake(&mut app).expect("the answers' frame");
    assert!(answers_shown(&mut app, ready), "the dialog shown went");
    press(&mut app, "y", ready + Duration::from_millis(400));
    let given = std::mem::take(&mut link.script().answers);
    assert_eq!(given.len(), 1, "{given:?}");
    assert_eq!(given[0].0, RequestId::Number(7));
}

/// Two dialogs, each answered in turn: the second waits for its own grace.
#[test]
fn the_next_dialog_answers_with_its_own_key_after_its_own_grace() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    link.give(Inbound::Request(RequestId::Number(8), approval("c_8")));
    drain(&mut app, now);
    let ready = wake(&mut app).expect("the first's answers");
    press(&mut app, "y", ready + Duration::from_millis(400));
    let second = wake(&mut app).expect("the second's answers");
    press(&mut app, "n", second + Duration::from_millis(400));
    let given: Vec<RequestId> = std::mem::take(&mut link.script().answers)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(given, [RequestId::Number(7), RequestId::Number(8)]);
}

/// The review's trigger: a deny note being written when its turn ends. The
/// note comes into the editor as a draft, and the `Enter` meant for it does
/// not send it to the model.
#[test]
fn a_note_cut_off_by_its_turns_end_is_a_draft_and_not_sent() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    press(&mut app, "e", ready + Duration::from_millis(400));
    let mut at = typing(&mut app, "use make", ready + Duration::from_millis(400), 80);
    link.give(event(Event::TurnCompleted(TurnCompleted {
        outcome: TurnOutcome::Interrupted,
        stop_reason: None,
        error: None,
        usage: cortex_protocol::usage::UsageTotals::default(),
        verification: None,
        duration_ms: None,
    })));
    drain(&mut app, at);
    // The frame after it says where the note went.
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, at, &mut scrollback);
    assert!(
        scrollback
            .iter()
            .any(|line| line.contains("note is in the input")),
        "{scrollback:#?}"
    );
    at = typing(&mut app, " instead", at, 80);
    press(&mut app, "enter", at + Duration::from_millis(80));
    assert!(
        turn_start(&link.take_sent()).is_empty(),
        "the end of a note was sent as a prompt"
    );
    assert!(answers(&link).is_empty());
    assert!(
        app.input().editor().expanded().contains("use make instead"),
        "the note is not in the editor: {:?}",
        app.input().editor().expanded()
    );
    // After a pause, `Enter` sends what the editor holds, as ever.
    press(&mut app, "enter", at + QUIET + Duration::from_millis(100));
    assert_eq!(turn_start(&link.take_sent()).len(), 1);
}

/// The same when another client answers the request (`request.resolved`).
#[test]
fn a_note_cut_off_by_another_clients_answer_is_a_draft() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    press(&mut app, "e", ready + Duration::from_millis(400));
    let at = typing(&mut app, "no", ready + Duration::from_millis(400), 80);
    link.give(event(Event::RequestResolved(RequestResolved {
        request_id: RequestId::Number(7),
        resolution: Resolution::Answered,
    })));
    drain(&mut app, at);
    press(&mut app, "enter", at + Duration::from_millis(80));
    assert!(turn_start(&link.take_sent()).is_empty());
    assert_eq!(app.input().editor().expanded(), "no");
}

/// `Ctrl+C` over a dialog with no turn running (the trust question, the
/// undo's question) leaves the text the dialog hides, and says that a
/// second quits.
#[test]
fn ctrl_c_over_a_dialog_with_no_turn_keeps_the_hidden_text_and_says_so() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "draft", now);
    ask(&mut app, &link, 7, approval("c_7"), now);
    press(&mut app, "ctrl+c", ms(now, 50));
    assert_eq!(app.input().editor().expanded(), "draft");
    let (_, live, _) = app.frame(ms(now, 50));
    let shown = text(&live).join("\n");
    assert!(shown.contains("again"), "the quit is not said:\n{shown}");
    press(&mut app, "ctrl+c", ms(now, 100));
    assert!(matches!(app.exit(), Some(Exit::Quit)));
}

/// A paste while the viewer is open goes to its search, never to the
/// editor it hides.
#[test]
fn a_paste_in_the_viewer_goes_to_its_search_and_not_the_editor() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    press(&mut app, "ctrl+o", now);
    assert!(app.viewing());
    app.terminal(&TermEvent::Paste("lost".to_owned()), ms(now, 10));
    press(&mut app, "/", ms(now, 20));
    app.terminal(&TermEvent::Paste("needle\nhay".to_owned()), ms(now, 30));
    assert_eq!(app.input().editor().expanded(), "");
    let lines = app.viewer_frame(40, 98, ms(now, 40));
    let shown = text(&lines).join("\n");
    assert!(shown.contains("needle hay"), "{shown}");
}

/// The trust question as the session opens, its first frame drawn at `at`.
fn trust_up(at: Instant) -> (App, Scripted) {
    let (mut app, link) = app(at);
    let _ = app.frame(at);
    app.drawn(at);
    link.take_sent();
    ask(&mut app, &link, 9, trust_request(project_resources()), at);
    (app, link)
}

/// The responses given so far, taken.
fn responses(link: &Scripted) -> Vec<ServerResponse> {
    std::mem::take(&mut link.script().answers)
        .into_iter()
        .map(|(_, response)| response)
        .collect()
}

/// The same rule holds for the trust question: typing at the start, with a
/// `y` and an `x` in it, neither trusts nor refuses the project.
#[test]
fn typing_through_the_trust_question_neither_trusts_nor_refuses() {
    let now = Instant::now();
    let (mut app, link) = trust_up(now);
    let last = typing(&mut app, "fix the yaml, then exit", now, 350);
    assert!(
        responses(&link).is_empty(),
        "typing answered the trust question"
    );
    let ready = wake(&mut app).expect("the answers' frame");
    assert_eq!(ready, last + QUIET);
    press(&mut app, "n", ready + GRACE);
    assert_eq!(
        responses(&link),
        [ServerResponse::Trust(
            cortex_protocol::methods::TrustResponse::default()
        )]
    );
}

/// The review's trigger: the trust question's answers drawn, a prompt
/// pasted, and `y` 100 ms after the paste.
#[test]
fn a_key_soon_after_a_paste_does_not_trust_the_project() {
    let now = Instant::now();
    let (mut app, link) = trust_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    let pasted = ready + Duration::from_millis(50);
    app.terminal(
        &TermEvent::Paste("yesterday's build broke: ".to_owned()),
        pasted,
    );
    // Past the pause since the paste, inside the quiet after it.
    press(&mut app, "y", pasted + Duration::from_millis(400));
    assert!(responses(&link).is_empty(), "a key after a paste trusted");
}

/// A question's digits too: typing "2 files" through the undo's question
/// does not pick its second choice; a digit once the typing stopped does.
#[test]
fn typing_through_a_question_does_not_pick_a_choice() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    ask(
        &mut app,
        &link,
        11,
        ServerRequest::Question(cortex_protocol::methods::QuestionRequest {
            session_id: SessionId::from(SESSION),
            turn_id: None,
            question: "Files changed since the turn. Put them back?".to_owned(),
            choices: vec!["Keep them".to_owned(), "Put them back".to_owned()],
            extension: None,
        }),
        now,
    );
    let last = typing(&mut app, "2 files", now, 350);
    assert!(responses(&link).is_empty(), "typing picked a choice");
    let ready = wake(&mut app).expect("the answers' frame");
    assert_eq!(ready, last + QUIET);
    press(&mut app, "1", ready + GRACE);
    assert_eq!(
        responses(&link),
        [ServerResponse::Question(
            cortex_protocol::methods::QuestionResponse {
                answer: Some("Keep them".to_owned())
            }
        )]
    );
}

/// `End` then `PageDown` (or `j`) in the viewer does not overflow.
#[test]
fn end_then_page_down_in_the_viewer_stays_at_the_end() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    press(&mut app, "ctrl+o", now);
    for binding in ["end", "pagedown", "end", "j", "down"] {
        key(&mut app, binding, ms(now, 10));
    }
    let _ = app.viewer_frame(40, 98, ms(now, 20));
}
