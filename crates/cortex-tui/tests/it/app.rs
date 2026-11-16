//! The application through its public test kit (`cortex_tui::testkit`), as
//! another crate's tests see it, and keys at a person's pace from the test
//! kit's `cadence`: a dialog that comes while someone types.

use std::time::{Duration, Instant};

use cortex_protocol::methods::{ApprovalChoice, ApprovalResponse, ServerResponse};
use cortex_protocol::session::SandboxStatus;
use cortex_term::Event;
use cortex_testkit::cadence::{Cadence, Stroke};
use cortex_tui::app::App;
use cortex_tui::keys::parse;
use cortex_tui::testkit::{app, approval, ask, opened, opened_with, session, shown_dialog};

/// Input as the interface's loop takes it: the frame due for the dialog
/// before each read, a key read with more input as one of a burst, and a
/// frame after each read.
fn feed(app: &mut App, strokes: &[Stroke]) {
    for stroke in strokes {
        while let Some(due) = app.deadline().filter(|due| *due <= stroke.at) {
            let _ = app.frame(due);
            app.drawn(due);
            if app.deadline() == Some(due) {
                break;
            }
        }
        if stroke.queued && matches!(stroke.event, Event::Key(_)) {
            app.burst(stroke.at);
        }
        app.terminal(&stroke.event, stroke.at);
        if !stroke.queued {
            let _ = app.frame(stroke.at);
            app.drawn(stroke.at);
        }
    }
}

fn allowed_once(response: &ServerResponse) -> bool {
    matches!(
        response,
        ServerResponse::Approval(ApprovalResponse {
            choice: ApprovalChoice::AllowOnce,
            ..
        })
    )
}

/// An approval comes while a message is being typed, at a key every 350 ms:
/// none of it answers, all of it reaches the editor, and once the typing
/// has stopped long enough for the answers to be drawn, one key answers.
#[test]
fn typing_through_an_approval_reaches_the_editor_and_one_key_after_answers() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let asked = now + Duration::from_secs(5);
    ask(&mut app, &link, 7, approval("c_7"), asked);
    let typed = Cadence::starting(asked).typing("try a smaller fix", Duration::from_millis(350));
    let last = typed.now();
    feed(&mut app, &typed.strokes());
    assert!(link.take_answers().is_empty(), "typing answered the dialog");
    assert_eq!(app.input().editor().expanded(), "try a smaller fix");

    let ready = app.deadline().expect("the answers are due");
    assert!(ready > last, "the answers were due while the person typed");
    feed(
        &mut app,
        &Cadence::starting(ready)
            .wait(Duration::from_millis(400))
            .key(parse("y").unwrap())
            .strokes(),
    );
    let answers = link.take_answers();
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert!(allowed_once(&answers[0].1), "{answers:?}");
}

/// The kit's dialog is answered at the time it returns, and the session the
/// test made is the application's.
#[test]
fn the_kit_opens_the_session_it_was_given_and_shows_a_dialog_one_key_answers() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    let confined = session().sandbox(SandboxStatus::Enforcing).build();
    opened_with(&mut app, &link, now, confined);
    assert_eq!(
        app.session().map(|state| state.sandbox.clone()),
        Some(SandboxStatus::Enforcing)
    );
    let at = shown_dialog(&mut app, &link, 1, now);
    app.terminal(&Event::Key(parse("y").unwrap()), at);
    let answers = link.take_answers();
    assert!(
        answers.iter().any(|(_, response)| allowed_once(response)),
        "{answers:?}"
    );
}
