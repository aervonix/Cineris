//! The person's approvals in the application, with a fake [`Approvals`]:
//! the password asked for when a session with approvals to use opens, and
//! at the trust question's yes; and the password going nowhere but the
//! fake's call.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cineris_protocol::ProjectKey;
use cineris_protocol::SessionId;
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::methods::{
    ApprovalProposal, Op, ServerRequest, ServerResponse, TrustRequest, TrustResponse,
};
use cineris_term::{Event as TermEvent, Key, KeyCode, Line};

use super::*;
use crate::app::dialog;
use crate::testkit::{SESSION, Scripted, app_with_approvals, drain, opened};

/// A bound on a wait for a call's thread, which only a broken test reaches.
const BOUND: Duration = Duration::from_secs(10);

/// The person's password, as the fake knows it.
const PASSWORD: &str = "correct horse battery";

/// What the fake signs with: a signature it can be told by.
const SIGNATURE: &str = "5ab1";

/// Why the fake says no password may be asked once it is traced.
const DEBUGGER: &str = "a debugger is attached";

/// What the fake was asked, each password as its text.
#[derive(Debug, Default)]
struct Log {
    unlocked: Vec<String>,
    set_up: Vec<String>,
    signed: Vec<(String, ApprovalProposal)>,
}

/// A fake of the program's approvals: one password, which a set-up
/// replaces.
struct Fake {
    to_use: bool,
    password: Mutex<Option<String>>,
    open: Mutex<bool>,
    log: Mutex<Log>,
    /// Why no password may be asked, while one may not (a debugger
    /// attached).
    refused: Mutex<Option<String>>,
}

impl Fake {
    fn new(to_use: bool, set_up: bool) -> Arc<Self> {
        Arc::new(Self {
            to_use,
            password: Mutex::new(set_up.then(|| PASSWORD.to_owned())),
            open: Mutex::new(false),
            log: Mutex::new(Log::default()),
            refused: Mutex::new(None),
        })
    }

    /// From now on, no password may be asked, for the reason a debugger
    /// attached gives.
    fn traced(&self) {
        *self.refused.lock().unwrap() = Some(DEBUGGER.to_owned());
    }

    fn is(&self, typed: &str) -> Result<(), Refused> {
        if self.password.lock().unwrap().as_deref() == Some(typed) {
            *self.open.lock().unwrap() = true;
            Ok(())
        } else {
            Err(Refused::NotThePassword(
                "that is not the password of your approvals".to_owned(),
            ))
        }
    }
}

fn text_of(password: &Password) -> String {
    String::from_utf8(password.expose_bytes().to_vec()).unwrap()
}

impl Approvals for Fake {
    fn to_use(&self) -> bool {
        self.to_use
    }

    fn why_not_asked(&self) -> Option<String> {
        self.refused.lock().unwrap().clone()
    }

    fn state(&self) -> ApprovalsState {
        if *self.open.lock().unwrap() {
            ApprovalsState::Open
        } else if self.password.lock().unwrap().is_none() {
            ApprovalsState::NotSetUp
        } else {
            ApprovalsState::Locked
        }
    }

    fn unlock(&self, password: &Password) -> Result<(), Refused> {
        let typed = text_of(password);
        self.log.lock().unwrap().unlocked.push(typed.clone());
        self.is(&typed)
    }

    fn set_up(&self, password: &Password) -> Result<(), Refused> {
        let typed = text_of(password);
        self.log.lock().unwrap().set_up.push(typed.clone());
        *self.password.lock().unwrap() = Some(typed);
        *self.open.lock().unwrap() = true;
        Ok(())
    }

    fn sign(&self, password: &Password, proposal: &ApprovalProposal) -> Result<String, Refused> {
        let typed = text_of(password);
        self.log
            .lock()
            .unwrap()
            .signed
            .push((typed.clone(), proposal.clone()));
        self.is(&typed).map(|()| SIGNATURE.repeat(32))
    }
}

/// Every frame's rows, as text, and what the frames held.
#[derive(Default)]
struct Frames(Vec<String>);

impl Frames {
    /// A frame at `at`, drawn as the loop draws it: its live rows.
    fn draw(&mut self, app: &mut App, at: Instant) -> String {
        let (commit, live, _) = app.frame(at);
        app.drawn(at);
        let text = |lines: &[Line]| -> Vec<String> {
            lines.iter().map(|line| line.text().into_owned()).collect()
        };
        self.0.extend(text(&commit));
        let live = text(&live);
        self.0.extend(live.iter().cloned());
        live.join("\n")
    }

    /// A frame at `after`, which shows the password's dialog, and the frame
    /// drawn when its field is due: the time that was, and its live rows.
    fn field(&mut self, app: &mut App, after: Instant) -> (Instant, String) {
        self.draw(app, after);
        let due = app.deadline().expect("the field is due").max(after);
        (due, self.draw(app, due))
    }

    fn held(&self, text: &str) -> bool {
        self.0.iter().any(|row| row.contains(text))
    }
}

/// Types `text` and `Enter` into the password's field at `at`.
fn type_password(app: &mut App, frames: &mut Frames, text: &str, at: Instant) {
    for character in text.chars() {
        app.terminal_pressed(
            &TermEvent::Key(Key::plain(KeyCode::Char(character))),
            at,
            at,
        );
    }
    frames.draw(app, at);
    app.terminal_pressed(&TermEvent::Key(Key::plain(KeyCode::Enter)), at, at);
    frames.draw(app, at);
}

fn press(app: &mut App, code: KeyCode, at: Instant) {
    app.terminal_pressed(&TermEvent::Key(Key::plain(code)), at, at);
}

fn opens(link: &Scripted) -> usize {
    link.take_sent()
        .iter()
        .filter(|(_, op)| matches!(op, Op::SessionNew(_)))
        .count()
}

fn ms(at: Instant, n: u64) -> Instant {
    at + Duration::from_millis(n)
}

/// When the session has approvals to use, the password is asked for after
/// the first frame, and the session opens only once it is answered: a
/// password that is not the person's asks again, saying so; theirs opens
/// the approvals, then the session.
#[test]
fn a_session_with_approvals_to_use_opens_after_the_password() {
    let now = Instant::now();
    let fake = Fake::new(true, true);
    let (mut app, link) = app_with_approvals(now, fake.clone());
    let mut frames = Frames::default();
    let first = frames.draw(&mut app, now);
    assert!(
        !first.contains("approvals password"),
        "not in the way of the first frame"
    );
    assert!(app.asking_password());
    assert_eq!(opens(&link), 0, "the open waits for the password");
    let (at, shown) = frames.field(&mut app, ms(now, 1));
    assert!(shown.contains("to use this session's approvals"), "{shown}");
    assert!(shown.contains("esc goes on without them"), "{shown}");
    type_password(&mut app, &mut frames, "a wrong password!", at);
    assert!(app.wait_for_approvals(BOUND));
    assert!(app.asking_password(), "asked again");
    assert_eq!(opens(&link), 0);
    let (at, shown) = frames.field(&mut app, ms(at, 1_000));
    assert!(shown.contains("that is not the password"), "{shown}");
    type_password(&mut app, &mut frames, PASSWORD, at);
    assert!(app.wait_for_approvals(BOUND));
    assert!(!app.asking_password());
    assert_eq!(opens(&link), 1, "the session opens once they are open");
    assert_eq!(
        fake.log.lock().unwrap().unlocked,
        ["a wrong password!", PASSWORD]
    );
    for typed in ["a wrong password!", PASSWORD] {
        assert!(!frames.held(typed), "{typed} was drawn");
    }
}

/// `Esc` at the password opens the session without the approvals, and
/// says so; nothing is unlocked.
#[test]
fn esc_opens_the_session_without_the_approvals() {
    let now = Instant::now();
    let fake = Fake::new(true, true);
    let (mut app, link) = app_with_approvals(now, fake.clone());
    let mut frames = Frames::default();
    frames.draw(&mut app, now);
    let (at, _) = frames.field(&mut app, ms(now, 1));
    press(&mut app, KeyCode::Esc, at);
    frames.draw(&mut app, at);
    assert!(!app.asking_password());
    assert_eq!(opens(&link), 1);
    assert!(frames.held("opened without your approvals"));
    assert!(fake.log.lock().unwrap().unlocked.is_empty());
}

/// With no approvals to use, nothing is asked: the session opens after
/// the first frame, as it always has.
#[test]
fn with_nothing_to_use_the_session_opens_unasked() {
    let now = Instant::now();
    let (mut app, link) = app_with_approvals(now, Fake::new(false, true));
    let mut frames = Frames::default();
    frames.draw(&mut app, now);
    assert!(!app.asking_password());
    assert_eq!(opens(&link), 1);
}

/// A trust question that proposes what a yes writes.
fn proposing(summary: &[&str]) -> ServerRequest {
    ServerRequest::Trust(TrustRequest {
        session_id: SessionId::from(SESSION),
        workspace: "~/src/app".to_owned(),
        resources: Vec::new(),
        trusted: false,
        proposal: Some(ApprovalProposal::Project {
            key: ProjectKey::new("app-1234"),
            revision: 1,
            root: "~/src/app".to_owned(),
            git_trusted: false,
            pins: Vec::new(),
        }),
        summary: summary.iter().map(|line| (*line).to_owned()).collect(),
    })
}

/// An application with its session open and `request` asked under the id
/// 40, its answers drawn: the time a key answers it.
fn asked(fake: &Arc<Fake>, request: ServerRequest, now: Instant) -> (App, Scripted, Instant) {
    let (mut app, link) = app_with_approvals(now, fake.clone());
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    link.give(crate::app::Inbound::Request(RequestId::Number(40), request));
    drain(&mut app, now);
    let _ = app.frame(now);
    app.drawn(now);
    let at = now + dialog::GRACE * 2;
    let _ = app.frame(at);
    app.drawn(at);
    (app, link, at)
}

/// The trust question's yes asks for the password with what the question
/// says a yes changes, signs the question's proposal with it, and answers
/// with the signature. The password goes to the call and nowhere else: no
/// frame, the input box, its history, nor anything sent holds it.
#[test]
fn the_trust_questions_yes_signs_with_the_password_and_nothing_else_holds_it() {
    let now = Instant::now();
    let fake = Fake::new(false, true);
    let (mut app, link, at) = asked(&fake, proposing(&["Trust ~/src/app and its rules"]), now);
    let mut frames = Frames::default();
    press(&mut app, KeyCode::Char('y'), at);
    assert!(
        link.take_answers().is_empty(),
        "not answered before it is signed"
    );
    assert!(app.asking_password());
    let (at, shown) = frames.field(&mut app, at);
    assert!(shown.contains("to approve:"), "{shown}");
    assert!(shown.contains("Trust ~/src/app and its rules"), "{shown}");
    type_password(&mut app, &mut frames, PASSWORD, at);
    assert!(app.wait_for_approvals(BOUND));
    let answers = link.take_answers();
    assert_eq!(
        answers,
        [(
            RequestId::Number(40),
            ServerResponse::Trust(TrustResponse {
                trusted: true,
                signature: Some(SIGNATURE.repeat(32)),
                ..TrustResponse::default()
            })
        )]
    );
    let log = fake.log.lock().unwrap();
    assert_eq!(log.signed.len(), 1);
    assert_eq!(log.signed[0].0, PASSWORD);
    assert!(matches!(log.signed[0].1, ApprovalProposal::Project { .. }));
    assert!(!frames.held(PASSWORD), "a frame held the password");
    assert!(app.input().editor().is_empty(), "the input box holds it");
    assert!(
        !format!("{:?}", link.take_sent()).contains(PASSWORD),
        "something sent holds it"
    );
    assert!(!format!("{answers:?}").contains(PASSWORD));
    // Its history: an `Up` in the empty input box brings back nothing of
    // it.
    press(&mut app, KeyCode::Up, ms(at, 2_000));
    assert!(!app.input().editor().expanded().contains(PASSWORD));
}

/// At the yes, a password that is not the person's asks again, saying so;
/// `Esc` then is not now.
#[test]
fn a_wrong_password_at_the_yes_asks_again_and_esc_is_not_now() {
    let now = Instant::now();
    let fake = Fake::new(false, true);
    let (mut app, link, at) = asked(&fake, proposing(&["Trust ~/src/app"]), now);
    let mut frames = Frames::default();
    press(&mut app, KeyCode::Char('y'), at);
    let (at, _) = frames.field(&mut app, at);
    type_password(&mut app, &mut frames, "not the one at all", at);
    assert!(app.wait_for_approvals(BOUND));
    assert!(link.take_answers().is_empty());
    assert!(app.asking_password(), "asked again");
    let (at, shown) = frames.field(&mut app, ms(at, 1_000));
    assert!(shown.contains("that is not the password"), "{shown}");
    press(&mut app, KeyCode::Esc, at);
    assert_eq!(
        link.take_answers(),
        [(
            RequestId::Number(40),
            ServerResponse::Trust(TrustResponse::default())
        )]
    );
}

/// With no key set up, the yes first sets one up: a password chosen, typed
/// twice, then the proposal signed with it.
#[test]
fn with_no_key_the_yes_sets_one_up_first() {
    let now = Instant::now();
    let fake = Fake::new(false, false);
    let (mut app, link, at) = asked(&fake, proposing(&["Trust ~/src/app"]), now);
    let mut frames = Frames::default();
    press(&mut app, KeyCode::Char('y'), at);
    let (at, shown) = frames.field(&mut app, at);
    assert!(shown.contains("Choose a password"), "{shown}");
    assert!(
        shown.contains("No password is set for your approvals yet"),
        "{shown}"
    );
    type_password(&mut app, &mut frames, "a password chosen now", at);
    let at = ms(at, 2_000);
    frames.draw(&mut app, at);
    type_password(&mut app, &mut frames, "a password chosen now", at);
    assert!(app.wait_for_approvals(BOUND));
    let log = fake.log.lock().unwrap();
    assert_eq!(log.set_up, ["a password chosen now"]);
    assert_eq!(log.signed.len(), 1);
    assert!(matches!(
        link.take_answers().as_slice(),
        [(
            _,
            ServerResponse::Trust(TrustResponse {
                trusted: true,
                signature: Some(_),
                ..
            })
        )]
    ));
}

/// With no approvals given (`cineris attach`), a yes that must be signed
/// is not now, said; a question that proposes nothing is answered as it
/// always was.
#[test]
fn with_nothing_to_sign_with_a_yes_that_must_be_signed_is_not_now() {
    let now = Instant::now();
    let (mut app, link) = crate::testkit::app(now);
    opened(&mut app, &link, now);
    link.give(crate::app::Inbound::Request(
        RequestId::Number(41),
        proposing(&["Trust ~/src/app"]),
    ));
    drain(&mut app, now);
    let _ = app.frame(now);
    app.drawn(now);
    let at = now + dialog::GRACE * 2;
    let _ = app.frame(at);
    app.drawn(at);
    press(&mut app, KeyCode::Char('y'), at);
    assert!(!app.asking_password());
    assert_eq!(
        link.take_answers(),
        [(
            RequestId::Number(41),
            ServerResponse::Trust(TrustResponse::default())
        )]
    );
    let rows: Vec<String> = app
        .frame(at)
        .0
        .iter()
        .map(|line| line.text().into_owned())
        .collect();
    assert!(
        rows.iter().any(|row| row.contains("cannot ask for")),
        "{rows:#?}"
    );
}

/// With a debugger attached before the session opens, no password is
/// asked: the dialog never opens, the session opens without the approvals,
/// and why is said (the full review of the signed approvals: the program
/// checked only once the dialog had read the password).
#[test]
fn a_debugger_attached_before_the_open_asks_nothing_and_opens_without() {
    let now = Instant::now();
    let fake = Fake::new(true, true);
    fake.traced();
    let (mut app, link) = app_with_approvals(now, fake.clone());
    let mut frames = Frames::default();
    frames.draw(&mut app, now);
    frames.draw(&mut app, ms(now, 1));
    assert!(!app.asking_password(), "a password was asked");
    assert_eq!(opens(&link), 1, "the session opens without them");
    assert!(frames.held("a debugger is attached"));
    assert!(frames.held("opened without your approvals"));
    assert!(fake.log.lock().unwrap().unlocked.is_empty());
}

/// With a debugger attached before the trust question's yes, no password
/// is asked, set up or signed: the yes is not now, and why is said.
#[test]
fn a_debugger_attached_before_the_yes_asks_nothing_and_is_not_now() {
    for set_up in [true, false] {
        let now = Instant::now();
        let fake = Fake::new(false, set_up);
        let (mut app, link, at) = asked(&fake, proposing(&["Trust ~/src/app"]), now);
        fake.traced();
        let mut frames = Frames::default();
        press(&mut app, KeyCode::Char('y'), at);
        frames.draw(&mut app, at);
        assert!(!app.asking_password(), "a password was asked ({set_up})");
        assert_eq!(
            link.take_answers(),
            [(
                RequestId::Number(40),
                ServerResponse::Trust(TrustResponse::default())
            )]
        );
        assert!(frames.held("a debugger is attached"), "{set_up}");
        let log = fake.log.lock().unwrap();
        assert!(log.signed.is_empty() && log.set_up.is_empty());
    }
}

/// The trust question's yes shows every line of the request's summary
/// under the password's sentence, the lines that say what a yes keeps of
/// what the entry held (a pin carried as held, not signed by the person's
/// key) among them, each drawn whole (the full review of the signed
/// approvals: a change signed shows all of what it does).
#[test]
fn the_yes_shows_every_line_of_the_summary_held_carries_among_them() {
    let now = Instant::now();
    let fake = Fake::new(false, true);
    let summary = [
        "trust the project at ~/src/app",
        "approve the rules `permissions.rules` (1a2b3c4d)",
        "keep the extension `tracker` (5e6f7a8b) as held: not signed by your key, it holds its \
         gates and is asked about again",
        "keep the hook `lint` (9c0d1e2f) as held: not signed by your key, it holds its gates and \
         is asked about again",
    ];
    let (mut app, _link, at) = asked(&fake, proposing(&summary), now);
    let mut frames = Frames::default();
    press(&mut app, KeyCode::Char('y'), at);
    let (_, shown) = frames.field(&mut app, at);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    for line in summary {
        assert!(flat.contains(line), "{line}: {shown}");
    }
}
