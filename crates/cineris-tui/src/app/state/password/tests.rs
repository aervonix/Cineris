//! The password dialog driven as the loop drives it: its keys are its own,
//! the keys queued when it opens and the typing that runs on into it are
//! dropped, and nothing of the password reaches a frame, the input box or
//! the link.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cineris_protocol::methods::Op;
use cineris_term::{Key, KeyCode, Mods};

use super::*;
use crate::app::ready::{GRACE, QUIET};
use crate::secret::Purpose;
use crate::testkit::{Scripted, app, opened};

/// What each handler was given, as text: `None` for nothing.
type Answers = Arc<Mutex<Vec<Option<String>>>>;

/// `n` milliseconds after `at`.
fn ms(at: Instant, n: u64) -> Instant {
    at + Duration::from_millis(n)
}

/// An application with its session open, at `now`.
fn open(now: Instant) -> (App, Scripted) {
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    (app, link)
}

/// Asks for the password for `purpose`; what the handler is given goes
/// into the answers.
fn ask(app: &mut App, purpose: Purpose) -> Answers {
    let answers = Answers::default();
    let kept = Arc::clone(&answers);
    app.ask_password(
        PasswordAsk::new(purpose, vec!["the extension `fmt-helper`".to_owned()]),
        move |_, password| {
            kept.lock().unwrap().push(
                password
                    .map(|password| String::from_utf8(password.expose_bytes().to_vec()).unwrap()),
            );
        },
    );
    answers
}

/// Every frame's rows, committed and live, as text.
#[derive(Default)]
struct Frames(Vec<String>);

impl Frames {
    /// A frame at `at`, drawn as the loop draws it; its live rows.
    fn draw(&mut self, app: &mut App, at: Instant) -> String {
        let (commit, live, _) = app.frame(at);
        app.drawn(at);
        let live: Vec<String> = live.iter().map(|line| line.text().into_owned()).collect();
        self.0
            .extend(commit.iter().map(|line| line.text().into_owned()));
        self.0.extend(live.iter().cloned());
        live.join("\n")
    }

    /// The loop's wake for the dialog's field, if one is due by `until`.
    fn wake(&mut self, app: &mut App, until: Instant) -> Option<String> {
        let due = app.deadline().filter(|due| *due <= until)?;
        Some(self.draw(app, due))
    }

    /// Whether any frame held `text`.
    fn held(&self, text: &str) -> bool {
        self.0.iter().any(|line| line.contains(text))
    }
}

fn char_key(character: char) -> TermEvent {
    TermEvent::Key(Key::plain(KeyCode::Char(character)))
}

/// Types `text` at `at`, a key a character, each with its frame.
fn type_at(app: &mut App, frames: &mut Frames, text: &str, at: Instant) {
    for character in text.chars() {
        app.terminal_pressed(&char_key(character), at, at);
        frames.draw(app, at);
    }
}

fn press(app: &mut App, frames: &mut Frames, code: KeyCode, mods: Mods, at: Instant) {
    app.terminal_pressed(&TermEvent::Key(Key { code, mods }), at, at);
    frames.draw(app, at);
}

/// The keys queued when it opens and the typing that runs on into it are
/// dropped; once it shows its field, the keys are its own and never the
/// input box's; `Enter` hands the password to the handler and the dialog
/// goes. No frame ever holds a character of it, and nothing is sent.
#[test]
fn keys_and_pastes_go_to_it_alone_and_never_to_the_input_box() {
    let now = Instant::now();
    let (mut app, link) = open(now);
    let mut frames = Frames::default();
    type_at(&mut app, &mut frames, "draft", now);
    let opened_at = ms(now, 5_000);
    let answers = ask(&mut app, Purpose::Sign);
    assert!(app.asking_password());
    // Queued when it opened, before its first frame.
    type_at(&mut app, &mut frames, "QUEUEDQ", opened_at);
    let first = frames.draw(&mut app, opened_at);
    assert!(first.contains("to approve:"), "{first}");
    assert!(first.contains("fmt-helper"), "{first}");
    assert!(
        !first.contains("characters, hidden"),
        "not before it is ready: {first}"
    );
    // Typing that runs on into it, and a paste.
    type_at(&mut app, &mut frames, "RUNONR", ms(opened_at, 100));
    app.terminal_pressed(
        &TermEvent::Paste("PASTEDP".to_owned()),
        ms(opened_at, 400),
        ms(opened_at, 400),
    );
    assert_eq!(frames.wake(&mut app, ms(opened_at, 400 + 999)), None);
    let ready = frames
        .wake(&mut app, ms(opened_at, 400 + QUIET.as_millis() as u64))
        .expect("a frame when the field is due");
    assert!(ready.contains("0 characters, hidden"), "{ready}");
    assert!(ready.contains("enter approves"), "{ready}");
    let typed_at = ms(opened_at, 2_000);
    type_at(&mut app, &mut frames, "s3cretPW", typed_at);
    app.terminal_pressed(&TermEvent::Paste("-ZZ\n".to_owned()), typed_at, typed_at);
    let typed = frames.draw(&mut app, typed_at);
    assert!(typed.contains("11 characters, hidden"), "{typed}");
    press(&mut app, &mut frames, KeyCode::Enter, Mods::NONE, typed_at);
    assert_eq!(*answers.lock().unwrap(), [Some("s3cretPW-ZZ".to_owned())]);
    assert!(!app.asking_password());
    assert_eq!(app.input().editor().expanded(), "draft");
    for text in ["QUEUEDQ", "RUNONR", "PASTEDP", "s3cretPW", "-ZZ"] {
        assert!(!frames.held(text), "{text} was drawn");
    }
    assert!(
        !link
            .take_sent()
            .iter()
            .any(|(_, op)| matches!(op, Op::TurnStart(_))),
        "nothing was sent"
    );
    let after = frames.draw(&mut app, ms(typed_at, 10));
    assert!(!after.contains("approve"), "the dialog is gone: {after}");
}

/// `Esc` and `Ctrl+C` give the handler nothing; `Ctrl+C` does before the
/// field is shown too, and it neither interrupts nor arms the quit.
#[test]
fn esc_and_ctrl_c_give_the_handler_nothing() {
    for (way, early) in [("esc", false), ("ctrl+c", false), ("early ctrl+c", true)] {
        let now = Instant::now();
        let (mut app, link) = open(now);
        let mut frames = Frames::default();
        let answers = ask(&mut app, Purpose::Unlock);
        frames.draw(&mut app, now);
        let at = if early {
            ms(now, 10)
        } else {
            frames
                .wake(&mut app, ms(now, 5_000))
                .expect("the field is due");
            let at = ms(now, 2_000);
            type_at(&mut app, &mut frames, "half", at);
            at
        };
        let (code, mods) = if way == "esc" {
            (KeyCode::Esc, Mods::NONE)
        } else {
            (KeyCode::Char('c'), Mods::CTRL)
        };
        press(&mut app, &mut frames, code, mods, at);
        assert_eq!(*answers.lock().unwrap(), [None], "{way}");
        assert!(!app.asking_password(), "{way}");
        assert!(!app.input().quit_armed(), "{way}");
        assert!(app.exit().is_none(), "{way}");
        assert!(
            !link
                .take_sent()
                .iter()
                .any(|(_, op)| matches!(op, Op::TurnStart(_) | Op::TurnInterrupt(_))),
            "{way}"
        );
    }
}

/// A password chosen in the dialog: typed twice, a mismatch says so and
/// asks again, and the handler is given it only once both typings agree.
#[test]
fn a_password_is_chosen_in_the_dialog_by_typing_it_twice() {
    let now = Instant::now();
    let (mut app, _link) = open(now);
    let mut frames = Frames::default();
    let answers = ask(&mut app, Purpose::Choose);
    frames.draw(&mut app, now);
    frames
        .wake(&mut app, ms(now, 5_000))
        .expect("the field is due");
    let at = ms(now, 2_000);
    for (typed, says) in [
        ("abcdefghijkl", "Type the same password again."),
        ("abcdefghijkX", "were not the same"),
        ("abcdefghijkX", "Type the same password again."),
    ] {
        type_at(&mut app, &mut frames, typed, at);
        press(&mut app, &mut frames, KeyCode::Enter, Mods::NONE, at);
        let shown = frames.draw(&mut app, at);
        assert!(shown.contains(says), "{typed}: {shown}");
        assert!(answers.lock().unwrap().is_empty());
    }
    type_at(&mut app, &mut frames, "abcdefghijkX", at);
    press(&mut app, &mut frames, KeyCode::Enter, Mods::NONE, at);
    assert_eq!(*answers.lock().unwrap(), [Some("abcdefghijkX".to_owned())]);
}

/// A second password asked waits behind the first; the key that answered
/// the first does not reach the second, which waits its own grace.
#[test]
fn a_second_password_asked_waits_behind_the_first() {
    let now = Instant::now();
    let (mut app, _link) = open(now);
    let mut frames = Frames::default();
    let first = ask(&mut app, Purpose::Sign);
    let second = ask(&mut app, Purpose::Unlock);
    frames.draw(&mut app, now);
    frames
        .wake(&mut app, ms(now, 5_000))
        .expect("the field is due");
    let at = ms(now, 2_000);
    type_at(&mut app, &mut frames, "first-pw", at);
    press(&mut app, &mut frames, KeyCode::Enter, Mods::NONE, at);
    assert_eq!(*first.lock().unwrap(), [Some("first-pw".to_owned())]);
    assert!(app.asking_password());
    let shown = frames.draw(&mut app, at);
    assert!(shown.contains("this session's approvals"), "{shown}");
    assert!(!shown.contains("characters, hidden"), "{shown}");
    // Typed at once after the key that answered the first: dropped.
    type_at(&mut app, &mut frames, "x", ms(at, GRACE.as_millis() as u64));
    let due = app.deadline().expect("the field is due");
    assert!(due >= ms(at, 1_000), "the repeat window after the answer");
    frames.draw(&mut app, due);
    type_at(&mut app, &mut frames, "second", ms(due, 10));
    press(
        &mut app,
        &mut frames,
        KeyCode::Enter,
        Mods::NONE,
        ms(due, 10),
    );
    assert_eq!(*second.lock().unwrap(), [Some("second".to_owned())]);
}
