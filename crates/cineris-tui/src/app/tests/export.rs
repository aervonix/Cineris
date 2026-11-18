//! `/export` in the application (step 25): the session read for an export
//! through the link, page by page, masked by the core and then by the
//! client's pass, and the text given to the program's writer on a worker;
//! what is said after. A page the core does not say it masked writes
//! nothing.

use std::sync::{Arc, Mutex};

use cineris_protocol::methods::{SessionReadParams, SessionReadResult};

use super::*;
use crate::export::{ExportError, ExportFile, Format};

/// A token in a well-known format, made up.
const TOKEN: &str = "ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8";

/// What the writer was given, and what it answers.
type Written = Arc<Mutex<Vec<ExportFile>>>;

fn writer(answer: Result<String, ExportError>) -> (crate::export::Writer, Written) {
    let written: Written = Arc::default();
    let kept = Arc::clone(&written);
    let writer: crate::export::Writer = Arc::new(move |file: ExportFile| {
        kept.lock().unwrap().push(file);
        answer.clone()
    });
    (writer, written)
}

/// The `session/read`s sent, each with its ticket.
fn reads(link: &Scripted) -> Vec<(Ticket, SessionReadParams)> {
    link.take_sent()
        .into_iter()
        .filter_map(|(ticket, op)| match op {
            Op::SessionRead(params) => Some((ticket, params)),
            _ => None,
        })
        .collect()
}

fn user(id: &str, text: &str) -> Item {
    Item::UserMessage(UserMessageItem {
        id: ItemId::from(id),
        parts: vec![Part::text(text)],
        delivery: None,
        created_at: None,
    })
}

fn page(items: Vec<Item>, masked: u32, next: Option<&str>) -> SessionReadResult {
    SessionReadResult {
        items,
        incomplete: Vec::new(),
        next_cursor: next.map(str::to_owned),
        seq: 4,
        masked,
        redacted: true,
    }
}

/// `/export` reads every page of the session with `redacted`, through the
/// link; the client's pass masks what the core's left (a token in a
/// well-known format, here), and the writer is given the Markdown, the
/// session, and no path; what is said names the file and the count, the
/// core's and the client's together.
#[test]
fn export_reads_the_session_masked_and_hands_the_writer_its_text() {
    let now = Instant::now();
    let (mut app, link) = with_models(now);
    let (writer, written) = writer(Ok("/work/app/cineris-session.md".to_owned()));
    app.set_export_writer(writer);
    run_command(&mut app, "/export", now);
    let sent = reads(&link);
    assert_eq!(sent.len(), 1);
    let (first, params) = &sent[0];
    assert!(params.redacted, "read for an export");
    assert_eq!(params.cursor, None);
    link.give(Inbound::Done(
        *first,
        Ok(OpResult::SessionRead(page(
            vec![user("u1", "my key is [redacted]")],
            1,
            Some("c2"),
        ))),
    ));
    drain(&mut app, now);
    let sent = reads(&link);
    assert_eq!(sent.len(), 1, "the next page");
    let (second, params) = &sent[0];
    assert!(params.redacted);
    assert_eq!(params.cursor.as_deref(), Some("c2"));
    link.give(Inbound::Done(
        *second,
        Ok(OpResult::SessionRead(page(
            vec![user("u2", &format!("and the token {TOKEN}"))],
            0,
            None,
        ))),
    ));
    drain(&mut app, now);
    app.wait_for_export();
    let written = written.lock().unwrap().clone();
    assert_eq!(written.len(), 1);
    let file = &written[0];
    assert_eq!(file.path, None);
    assert_eq!(file.format, Format::Markdown);
    assert!(file.text.contains("my key is [redacted]"), "{}", file.text);
    assert!(
        file.text.contains("and the token [redacted]"),
        "{}",
        file.text
    );
    assert!(!file.text.contains(TOKEN), "{}", file.text);
    let said = notices(&mut app, now);
    assert!(
        said.contains(
            "Exported the session to /work/app/cineris-session.md. 2 values were masked."
        ),
        "{said}"
    );
}

/// `/export json` to a file that is there: the writer says so, and what is
/// said names it, and that nothing was written; a second `/export` while
/// one is under way is refused; a failed read says why.
#[test]
fn an_export_over_a_file_that_is_there_writes_nothing_and_says_so() {
    let now = Instant::now();
    let (mut app, link) = with_models(now);
    let (writer, written) = writer(Err(ExportError::Exists("notes.json".to_owned())));
    app.set_export_writer(writer);
    run_command(&mut app, "/export notes.json", now);
    run_command(&mut app, "/export", now);
    let sent = reads(&link);
    assert_eq!(sent.len(), 1, "one export at a time");
    assert!(notices(&mut app, now).contains("An export is being written already"));
    link.give(Inbound::Done(
        sent[0].0,
        Ok(OpResult::SessionRead(page(vec![user("u1", "hi")], 0, None))),
    ));
    drain(&mut app, now);
    app.wait_for_export();
    let file = written.lock().unwrap()[0].clone();
    assert_eq!(file.format, Format::Json);
    assert_eq!(file.path.as_deref(), Some("notes.json"));
    assert!(notices(&mut app, now).contains("notes.json is there already: nothing was written."));

    // A read the core refused: said, and the next export may start.
    run_command(&mut app, "/export", now);
    let sent = reads(&link);
    link.give(Inbound::Done(
        sent[0].0,
        Err(cineris_protocol::ErrorData::new(
            cineris_protocol::ErrorKind::Internal,
            "store_failed",
            "the log could not be read",
        )),
    ));
    drain(&mut app, now);
    assert!(notices(&mut app, now).contains("the log could not be read"));
    run_command(&mut app, "/export", now);
    assert_eq!(reads(&link).len(), 1, "a new export starts");
}

/// Where the program gives no writer, `/export` says it is not available
/// and reads nothing; a path that is neither Markdown nor JSON is refused
/// before anything is read.
#[test]
fn export_says_why_it_cannot_and_reads_nothing() {
    let now = Instant::now();
    let (mut app, link) = with_models(now);
    run_command(&mut app, "/export", now);
    assert!(reads(&link).is_empty());
    assert!(notices(&mut app, now).contains("Exporting is not available here."));
    let (writer, _written) = writer(Ok(String::new()));
    app.set_export_writer(writer);
    run_command(&mut app, "/export out.txt", now);
    assert!(reads(&link).is_empty());
    assert!(notices(&mut app, now).contains("ends in .md or .json"));
}

/// A core that does not know `redacted` (an older one) reads the items as
/// they are, and its result does not say it masked them: nothing is given
/// to the writer, no other page is asked for, and the person is told why
/// (the full review of step 25's second pull request).
#[test]
fn a_page_the_core_did_not_mask_writes_nothing_and_says_why() {
    let now = Instant::now();
    let (mut app, link) = with_models(now);
    let (writer, written) = writer(Ok("/work/app/cineris-session.md".to_owned()));
    app.set_export_writer(writer);
    run_command(&mut app, "/export", now);
    let sent = reads(&link);
    let mut unmasked = page(
        vec![user("u1", &format!("the token {TOKEN}"))],
        0,
        Some("c2"),
    );
    unmasked.redacted = false;
    link.give(Inbound::Done(
        sent[0].0,
        Ok(OpResult::SessionRead(unmasked)),
    ));
    drain(&mut app, now);
    assert!(reads(&link).is_empty(), "no other page is read");
    app.wait_for_export();
    assert!(
        written.lock().unwrap().is_empty(),
        "nothing was given to the writer"
    );
    assert!(
        notices(&mut app, now)
            .contains("The core did not mask the session, so nothing was written"),
    );
    // The next export may start.
    run_command(&mut app, "/export", now);
    assert_eq!(reads(&link).len(), 1);
}

/// A turn that runs with no item in progress (waiting on an approval,
/// between its rounds, running its checks) is still a turn that was
/// running: the export says so.
#[test]
fn an_export_while_a_turn_runs_with_nothing_in_progress_says_it_was_running() {
    let now = Instant::now();
    let (mut app, link) = with_models(now);
    let (writer, written) = writer(Ok("/work/app/cineris-session.md".to_owned()));
    app.set_export_writer(writer);
    link.give(event(Event::TurnStarted(TurnStarted {})));
    drain(&mut app, now);
    run_command(&mut app, "/export", now);
    let sent = reads(&link);
    link.give(Inbound::Done(
        sent[0].0,
        Ok(OpResult::SessionRead(page(vec![user("u1", "hi")], 0, None))),
    ));
    drain(&mut app, now);
    app.wait_for_export();
    let file = written.lock().unwrap()[0].clone();
    assert!(file.text.contains("A turn was running"), "{}", file.text);
}
