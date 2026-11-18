//! The dialogs' key rule after the review of step 23 (`super::super::dialog`,
//! "Only keys meant for it answer it"): typing, a paste, a held key and the
//! terminal's own repeats never answer a dialog; one key does once the
//! person has stopped; the answers are drawn only when they can be pressed;
//! two dialogs at once; a note cut off by its request's end; `Ctrl+C` over a
//! dialog; and the viewer's paste and scroll.

use cortex_testkit::cadence::{AutoRepeat, Cadence, Stroke};

use super::*;
use crate::app::dialog::{GRACE, QUIET, REPEAT};

/// `n` milliseconds after `now`.
fn ms(now: Instant, n: u64) -> Instant {
    now + Duration::from_millis(n)
}

/// The loop's own wakes up to `until`: when the dialog's moment has come
/// by then, a frame at that moment, as `run_with` draws it.
fn wake_until(app: &mut App, until: Instant) {
    while let Some(due) = app.deadline() {
        if due > until {
            break;
        }
        let _ = app.frame(due);
        app.drawn(due);
        if app.deadline() == Some(due) {
            break;
        }
    }
}

/// One key read at `at`, after the loop's wakes before it, and the frame
/// the loop draws for it.
fn press(app: &mut App, binding: &str, at: Instant) {
    wake_until(app, at);
    key(app, binding, at);
    let _ = app.frame(at);
    app.drawn(at);
}

/// Typing `text` from `from`, a key every `every` milliseconds, each with
/// its frame. Returns when the last key was read.
fn typing(app: &mut App, text: &str, from: Instant, every: u64) -> Instant {
    feed(
        app,
        &Cadence::starting(from)
            .typing(text, Duration::from_millis(every))
            .strokes(),
    )
}

/// Input as the loop takes it: its wakes before each read, a key read with
/// more input as one of a burst (`App::burst`), and a frame after each
/// read. Returns when the last was read.
fn feed(app: &mut App, strokes: &[Stroke]) -> Instant {
    for stroke in strokes {
        wake_until(app, stroke.at);
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

/// A session open with a turn running (its prompt typed long before), and
/// an approval whose first frame was drawn at `at`.
fn approval_up(at: Instant) -> (App, Scripted) {
    let long_before = at - Duration::from_secs(5);
    let (mut app, link) = app(long_before);
    opened(&mut app, &link, long_before);
    type_text(&mut app, "hi", long_before);
    key(&mut app, "enter", long_before);
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

/// A key the dialog offers, held from inside the grace, at each system's
/// default auto-repeat, marked as a repeat or not: never an answer, and
/// all of it typing.
#[test]
fn a_held_key_never_answers_at_either_systems_auto_repeat() {
    let now = Instant::now();
    let y = parse("y").unwrap();
    for repeat in [AutoRepeat::X11, AutoRepeat::WINDOWS] {
        let (mut app, link) = approval_up(now);
        let strokes = Cadence::starting(ms(now, 50)).hold(y, repeat, 40).strokes();
        let last = feed(&mut app, &strokes);
        assert!(answers(&link).is_empty(), "{repeat:?}: a held key answered");
        assert_eq!(
            app.input().editor().expanded(),
            "y".repeat(41),
            "{repeat:?}"
        );
        // A second after it is let go, one press answers.
        let ready = wake(&mut app).expect("the answers' frame");
        assert_eq!(ready, last + QUIET, "{repeat:?}");
        press(&mut app, "y", ready + Duration::from_millis(400));
        assert_eq!(answers(&link), [ApprovalChoice::AllowOnce], "{repeat:?}");
    }
}

/// Keys that arrive in one read (a paste where the terminal has no
/// bracketed paste, or typing a stall held back) are typing, and none of
/// them answers, though the dialog takes keys; one key read alone after the
/// quiet does.
#[test]
fn keys_read_together_never_answer_and_one_read_alone_does() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    let read = feed(
        &mut app,
        &Cadence::starting(ready)
            .wait(Duration::from_millis(500))
            .together("yes")
            .strokes(),
    );
    assert!(answers(&link).is_empty(), "keys read together answered");
    assert_eq!(app.input().editor().expanded(), "yes");
    let again = wake(&mut app).expect("the answers again");
    assert_eq!(again, read + QUIET);
    feed(
        &mut app,
        &Cadence::starting(again)
            .wait(Duration::from_millis(400))
            .key(parse("y").unwrap())
            .strokes(),
    );
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
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
    // The press just before it counts as typing at it: a quiet second.
    let ready = wake(&mut app).expect("the answers' frame");
    assert_eq!(ready, ms(now, 1000) + QUIET);
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
/// input) answers nothing.
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
    // A key held down whose press no dialog took is typing: its repeats go
    // on to the editor, and the answers wait a quiet second after the last.
    assert_eq!(app.input().editor().expanded(), "yyyyy");
    assert!(!answers_shown(&mut app, ready + Duration::from_millis(600)));
    let again = wake(&mut app).expect("the answers again");
    assert_eq!(again, ready + Duration::from_millis(400 + 33 * 4) + QUIET);
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

/// A paste after a note was cut off carries the typing on as a key does:
/// an `Enter` soon after it does not send, though the cut was long before.
#[test]
fn a_paste_after_a_cut_note_holds_the_enter_too() {
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
    let pasted = at + QUIET - Duration::from_millis(100);
    app.terminal(&TermEvent::Paste(", use make".to_owned()), pasted);
    press(&mut app, "enter", pasted + Duration::from_millis(300));
    assert!(
        turn_start(&link.take_sent()).is_empty(),
        "an Enter just after a paste sent the cut note"
    );
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

/// A question's digits too: typing that began before the undo's question
/// and goes on through it, "2 files" among it, does not pick its second
/// choice; a digit once the typing stopped does.
#[test]
fn typing_through_a_question_does_not_pick_a_choice() {
    let now = Instant::now();
    let (mut app, link) = app(now - Duration::from_secs(5));
    opened(&mut app, &link, now - Duration::from_secs(5));
    let before = typing(&mut app, "keep ", now - Duration::from_millis(1750), 350);
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
        before + Duration::from_millis(100),
    );
    let last = typing(&mut app, "2 files", before, 350);
    assert!(responses(&link).is_empty(), "typing picked a choice");
    assert_eq!(app.input().editor().expanded(), "keep 2 files");
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

// The independent review of these fixes, with the loop's wakes the first
// tests left out.

/// The review's own trigger through the loop: typing at a key every 350
/// ms, the approval's first frame between two keys. The loop wakes at the
/// grace's end; the keys after it are still typing.
#[test]
fn steady_typing_through_the_loops_wake_never_answers() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    typing(&mut app, "try a smaller fix", now, 350);
    assert!(answers(&link).is_empty(), "steady typing answered");
    assert_eq!(app.input().editor().expanded(), "try a smaller fix");
}

/// A slow typist (a key every 500 ms), the approval 100 ms after a key:
/// the typing before it counts as typing at it.
#[test]
fn typing_just_before_the_dialog_counts_as_typing() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now - Duration::from_secs(10));
    // The first dialog answered long ago; the typing starts afresh.
    let _ = wake(&mut app);
    press(&mut app, "n", now - Duration::from_secs(8));
    assert_eq!(answers(&link), [ApprovalChoice::Deny]);
    let last = typing(&mut app, "please ", now - Duration::from_secs(4), 500);
    ask(
        &mut app,
        &link,
        8,
        approval("c_8"),
        last + Duration::from_millis(100),
    );
    typing(&mut app, "say yes", last, 500);
    assert!(
        answers(&link).is_empty(),
        "typing before the dialog answered it"
    );
    assert_eq!(app.input().editor().expanded(), "please say yes");
}

/// Typing into the full screen's search is typing, as into the editor
/// (the review of step 25): a dialog that comes in the middle of it, typed
/// slowly, waits for it to pause, and the search row is not drawn over the
/// dialog's own row.
#[test]
fn typing_into_the_full_screen_search_does_not_answer_a_dialog() {
    let now = Instant::now();
    let long_before = now - Duration::from_secs(10);
    let settings = Settings {
        presenter: Presenter::Fullscreen,
        ..Settings::default()
    };
    let (mut app, link) = crate::testkit::app_with_settings(long_before, settings);
    opened(&mut app, &link, long_before);
    type_text(&mut app, "hi", long_before);
    key(&mut app, "enter", long_before);
    link.take_sent();
    press(&mut app, "ctrl+f", now - Duration::from_secs(4));
    // `t`, then the dialog, then `y`, which is an answer of its own.
    let last = typing(&mut app, "t", now - Duration::from_secs(1), 450);
    ask(
        &mut app,
        &link,
        7,
        approval("c_7"),
        last + Duration::from_millis(100),
    );
    let last = typing(&mut app, "ype", last + Duration::from_millis(450), 450);
    assert!(
        answers(&link).is_empty(),
        "typing into the search answered the dialog"
    );
    // The keys the dialog passed went on to the search, hidden under it,
    // and `Enter`, the search's older match, sent nothing: never the hidden
    // editor, whose `Enter` would send (the second review of step 25).
    press(&mut app, "enter", last + Duration::from_millis(300));
    assert!(answers(&link).is_empty(), "Enter answered the dialog");
    assert_eq!(app.search_query().as_deref(), Some("type"));
    assert_eq!(app.input().editor().expanded(), "");
    assert!(
        !link
            .take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::TurnStart(_))),
        "a prompt was sent"
    );
    let (lines, _) = app.screen_frame(now + Duration::from_secs(20));
    let rows = text(&lines);
    assert!(
        !rows.last().is_some_and(|row| row.contains("ctrl+f")),
        "the search row over the dialog's: {rows:#?}"
    );
}

/// A paste into the full screen's search is typing too: a dialog that comes
/// just after it waits for a pause before `y` answers it (the review of step
/// 25).
#[test]
fn a_paste_into_the_full_screen_search_holds_a_dialog_that_comes() {
    let now = Instant::now();
    let long_before = now - Duration::from_secs(10);
    let settings = Settings {
        presenter: Presenter::Fullscreen,
        ..Settings::default()
    };
    let (mut app, link) = crate::testkit::app_with_settings(long_before, settings);
    opened(&mut app, &link, long_before);
    type_text(&mut app, "hi", long_before);
    key(&mut app, "enter", long_before);
    link.take_sent();
    press(&mut app, "ctrl+f", now - Duration::from_secs(4));
    let pasted = now - Duration::from_secs(1);
    app.terminal(&TermEvent::Paste("lexer".to_owned()), pasted);
    ask(
        &mut app,
        &link,
        7,
        approval("c_7"),
        pasted + Duration::from_millis(100),
    );
    press(&mut app, "y", pasted + Duration::from_millis(450));
    assert!(
        answers(&link).is_empty(),
        "a key just after the paste answered the dialog"
    );
}

/// A dialog that fills a short window in the full screen leaves the
/// transcript no row, and its own last row, its keys, is drawn: the frame
/// is the window's height, no more (the review of step 25).
#[test]
fn a_dialog_that_fills_the_full_screen_keeps_its_keys_row() {
    let now = Instant::now();
    let settings = Settings {
        presenter: Presenter::Fullscreen,
        ..Settings::default()
    };
    let (mut app, link) = crate::testkit::app_with_settings(now, settings);
    opened(&mut app, &link, now);
    app.resize(79, 8);
    ask(&mut app, &link, 7, approval("c_7"), now);
    let (lines, _) = app.screen_frame(now + Duration::from_secs(2));
    let rows = text(&lines);
    assert_eq!(rows.len(), 8, "{rows:#?}");
    assert!(
        rows.last().is_some_and(|row| row.contains("esc denies")),
        "{rows:#?}"
    );
}

/// The mouse answers no dialog, so it is not typing: the wheel turned over
/// the full screen while a dialog's answers are shown leaves them shown,
/// and the next key answers (the review of step 25).
#[test]
fn the_wheel_does_not_hide_a_dialogs_answers() {
    let now = Instant::now();
    let settings = Settings {
        presenter: Presenter::Fullscreen,
        ..Settings::default()
    };
    let (mut app, link) = crate::testkit::app_with_settings(now, settings);
    opened(&mut app, &link, now);
    ask(&mut app, &link, 7, approval("c_7"), now);
    let ready = wake(&mut app).expect("the answers' frame");
    assert!(answers_shown(&mut app, ready));
    let turned = ready + Duration::from_millis(100);
    app.terminal(
        &TermEvent::Mouse(cortex_term::Mouse {
            kind: cortex_term::MouseKind::Wheel(cortex_term::Wheel::Up),
            col: 5,
            row: 5,
            mods: cortex_term::Mods::NONE,
        }),
        turned,
    );
    assert!(answers_shown(&mut app, turned), "the wheel hid the answers");
    press(&mut app, "y", turned + Duration::from_millis(100));
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// A gesture bound to closing the viewer closes it as its key does: a
/// dialog waiting under the viewer starts its grace again, and its answers
/// are not drawn the moment the viewer is gone (the second review of step
/// 25).
#[test]
fn a_gesture_that_closes_the_viewer_restarts_the_dialogs_grace() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let wheel = vec!["wheel_down".to_owned()];
    let (keymap, errors) = crate::keys::KeyMap::with_table([("viewer.close", wheel.as_slice())]);
    assert!(errors.is_empty(), "{errors:?}");
    // A new map hides the dialog: its first frame again.
    app.set_keymap(keymap);
    let _ = app.frame(now);
    app.drawn(now);
    let ready = wake(&mut app).expect("the answers' frame");
    assert!(answers_shown(&mut app, ready));
    press(&mut app, "ctrl+o", ready + Duration::from_millis(100));
    assert!(app.viewing(), "the request shown whole");
    let closed = ready + Duration::from_millis(600);
    app.terminal(
        &TermEvent::Mouse(cortex_term::Mouse {
            kind: cortex_term::MouseKind::Wheel(cortex_term::Wheel::Down),
            col: 5,
            row: 5,
            mods: cortex_term::Mods::NONE,
        }),
        closed,
    );
    assert!(!app.viewing(), "the gesture closed the viewer");
    assert!(
        !answers_shown(&mut app, closed),
        "the answers drawn the moment the viewer closed"
    );
    app.drawn(closed);
    let again = wake(&mut app).expect("the answers again");
    assert!(again >= closed + GRACE);
    assert!(answers_shown(&mut app, again));
    assert!(answers(&link).is_empty());
}

/// Two approvals at once; `y` answers the first and is held a little long:
/// its repeats never answer the second, which was not seen yet.
#[test]
fn a_key_held_after_answering_one_dialog_never_answers_the_next() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    link.give(Inbound::Request(RequestId::Number(8), approval("c_8")));
    drain(&mut app, now);
    let held = Cadence::starting(ms(now, 1500))
        .hold(
            parse("y").unwrap(),
            AutoRepeat::new(Duration::from_millis(500), Duration::from_millis(33)),
            10,
        )
        .strokes();
    let (pressed, repeats) = held.split_first().unwrap();
    feed(&mut app, std::slice::from_ref(pressed));
    assert_eq!(answers(&link).len(), 1, "the first press answers");
    feed(&mut app, repeats);
    assert!(
        answers(&link).is_empty(),
        "a held key answered the next dialog"
    );
    // Once a second has passed with nothing typed, one key answers it.
    let ready = wake(&mut app).expect("the second's answers");
    press(&mut app, "n", ready + Duration::from_millis(400));
    assert_eq!(answers(&link), [ApprovalChoice::Deny]);
}

/// The answers row is drawn only when a key on it is taken: typing that
/// ended in `y` just before the dialog, then `y` pressed whenever the row
/// shows it, answers at the first press the row offers.
#[test]
fn a_key_the_row_offers_is_taken() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now - Duration::from_secs(10));
    let _ = wake(&mut app);
    press(&mut app, "n", now - Duration::from_secs(8));
    answers(&link);
    let last = typing(&mut app, "try", now - Duration::from_secs(1), 120);
    ask(
        &mut app,
        &link,
        8,
        approval("c_8"),
        last + Duration::from_millis(100),
    );
    let mut at = last + Duration::from_millis(900);
    for _ in 0..4 {
        wake_until(&mut app, at);
        let offered = answers_shown(&mut app, at);
        press(&mut app, "y", at);
        let given = answers(&link);
        if offered {
            assert_eq!(given, [ApprovalChoice::AllowOnce], "the row offered y");
            return;
        }
        assert!(
            given.is_empty(),
            "y answered while the row did not offer it"
        );
        at += Duration::from_millis(1100);
    }
    panic!("the row never offered y");
}

/// `Ctrl+O` shows the request whole, `Esc` closes the viewer, and `Esc`
/// again is to deny: taken when the row offers it, never dropped then.
#[test]
fn esc_after_closing_the_viewer_denies_when_the_row_offers_it() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    press(&mut app, "ctrl+o", ms(now, 1500));
    assert!(app.viewing(), "ctrl+o opens the viewer");
    let closed = ms(now, 4500);
    press(&mut app, "esc", closed);
    assert!(!app.viewing());
    // An `Esc` before the row is back is typing too: the next waits a
    // quiet second after it.
    for wait in [600, 1700] {
        let at = closed + Duration::from_millis(wait);
        wake_until(&mut app, at);
        let offered = answers_shown(&mut app, at);
        press(&mut app, "esc", at);
        let given = answers(&link);
        assert_eq!(
            !given.is_empty(),
            offered,
            "at +{wait} ms: offered {offered}"
        );
        if offered {
            assert_eq!(given, [ApprovalChoice::Deny]);
            return;
        }
    }
    panic!("the row never offered esc");
}

/// `Ctrl+C` over a question with no turn running arms the quit; a key the
/// question takes disarms it.
#[test]
fn answering_the_dialog_disarms_the_quit() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    ask(
        &mut app,
        &link,
        5,
        ServerRequest::Question(cortex_protocol::methods::QuestionRequest {
            session_id: SessionId::from(SESSION),
            turn_id: None,
            question: "Overwrite the files edited since?".to_owned(),
            choices: vec!["yes".to_owned(), "no".to_owned()],
            extension: None,
        }),
        ms(now, 1000),
    );
    press(&mut app, "ctrl+c", ms(now, 2500));
    assert!(app.input().quit_armed());
    press(&mut app, "esc", ms(now, 4000));
    assert_eq!(responses(&link).len(), 1, "esc answers");
    assert!(!app.input().quit_armed(), "the quit is still armed");
}

/// A key the ready dialog has no use for that types a character goes on to
/// the editor, as typing resumed after a pause: kept, and the answers are
/// hidden again for a quiet second.
#[test]
fn a_typing_key_the_dialog_has_no_use_for_goes_to_the_editor() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    let ready = wake(&mut app).expect("the answers' frame");
    let last = typing(&mut app, "try", ready + Duration::from_millis(100), 400);
    assert!(answers(&link).is_empty(), "resumed typing answered");
    assert_eq!(app.input().editor().expanded(), "try");
    assert!(!answers_shown(&mut app, last));
    // A key that types nothing and answers nothing does nothing.
    let ready = wake(&mut app).expect("the answers again");
    press(&mut app, "ctrl+b", ready + Duration::from_millis(400));
    assert!(answers_shown(&mut app, ready + Duration::from_millis(400)));
}

// The follow-up review of these fixes: the row offers only what is taken,
// and reading, answering one dialog after another, and coming back from the
// viewer each need one key.

/// A global key (`Ctrl+T` looks at the round) and then the answer: the row
/// offered it, and it is taken.
#[test]
fn a_key_the_row_offers_soon_after_a_global_key_is_taken() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    press(&mut app, "ctrl+t", ms(now, 2000));
    assert!(answers_shown(&mut app, ms(now, 2200)), "the row offers y");
    press(&mut app, "y", ms(now, 2200));
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// Two approvals answered one after the other: an answer key pressed
/// before the second's row is drawn is not typed into the hidden editor.
#[test]
fn an_answer_key_before_the_next_dialogs_row_is_not_typed() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    link.give(Inbound::Request(RequestId::Number(8), approval("c_8")));
    drain(&mut app, now);
    press(&mut app, "y", ms(now, 1000));
    assert_eq!(answers(&link).len(), 1);
    press(&mut app, "y", ms(now, 1700));
    assert!(answers(&link).is_empty(), "the second answered unseen");
    assert_eq!(
        app.input().editor().expanded(),
        "",
        "an answer key was typed"
    );
    let ready = wake(&mut app).expect("the second's answers");
    press(&mut app, "y", ready + Duration::from_millis(400));
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}

/// `Ctrl+O`, the request read whole, `Esc`, then `y` once the dialog's
/// grace after its return has passed: one key answers.
#[test]
fn ctrl_o_then_esc_then_one_key_answers() {
    let now = Instant::now();
    let (mut app, link) = approval_up(now);
    press(&mut app, "ctrl+o", ms(now, 1000));
    assert!(app.viewing());
    press(&mut app, "pagedown", ms(now, 3000));
    press(&mut app, "esc", ms(now, 6000));
    assert!(!app.viewing());
    assert!(answers_shown(&mut app, ms(now, 6500)), "the row is back");
    press(&mut app, "y", ms(now, 6500));
    assert_eq!(answers(&link), [ApprovalChoice::AllowOnce]);
}
