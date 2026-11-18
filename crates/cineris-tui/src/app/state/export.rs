//! `/export` in the application (since step 25): the session read for an
//! export, page by page, through the `Link` (`session/read` with
//! `redacted`, a read beside the lane, so `cineris attach` exports the same
//! way), each page refused unless the core says it masked it; then, on a
//! thread of its own, the client's pass, the file's text
//! ([`crate::export`]) and the program's writer, which wakes the loop when
//! it is done.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use cineris_protocol::methods::{Op, SessionReadParams, SessionReadResult};
use cineris_protocol::{Item, SessionId};

use super::{App, HISTORY_PAGE, Pending};
use crate::export::{Export, ExportError, ExportFile, Format};

/// The writer's answer, and how many values are masked in what it was
/// given.
type Written = (Result<String, ExportError>, usize);

/// An export under way.
#[derive(Debug)]
pub(super) struct Exporting {
    /// The session read.
    session: SessionId,
    /// The format asked for.
    format: Format,
    /// The path the person gave.
    path: Option<String>,
    /// The items read so far, masked by the core.
    items: Vec<Item>,
    /// A turn was running while the session was read: what it had not
    /// finished was left out.
    running: bool,
    /// The worker's answer, once the items are with it.
    writing: Option<Receiver<Written>>,
}

impl App {
    /// `/export [md | json | <path>]`.
    pub(super) fn export(&mut self, argument: &str) {
        if self.writer.is_none() {
            self.say("Exporting is not available here.");
            return;
        }
        let Some(session) = self.session.as_ref().map(|state| state.session_id.clone()) else {
            self.say("There is no session to export yet.");
            return;
        };
        if self.exporting.is_some() {
            self.say("An export is being written already: wait for it to finish.");
            return;
        }
        let (format, path) = match Format::from_argument(argument) {
            Ok(asked) => asked,
            Err(sentence) => {
                self.say(&sentence);
                return;
            }
        };
        self.exporting = Some(Exporting {
            session: session.clone(),
            format,
            path,
            items: Vec::new(),
            running: false,
            writing: None,
        });
        self.read_for_export(session, None);
    }

    /// Asks for a page of the session, masked for an export.
    fn read_for_export(&mut self, session_id: SessionId, cursor: Option<String>) {
        let ticket = self.link.send(Op::SessionRead(SessionReadParams {
            session_id: session_id.clone(),
            item_id: None,
            limit: Some(HISTORY_PAGE),
            cursor,
            redacted: true,
        }));
        self.pending.insert(ticket, Pending::Export(session_id));
    }

    /// A page read for the export: refused when the core does not say it
    /// masked it; else the next is asked for, or, at the last, the items
    /// go to a worker, which makes the text and gives it to the writer.
    pub(super) fn export_read(&mut self, session: &SessionId, page: SessionReadResult) {
        // A turn waiting on an answer, between its rounds or running its
        // checks has no item in progress, and is running all the same.
        let turn_runs = self.turn_runs;
        let Some(exporting) = self
            .exporting
            .as_mut()
            .filter(|exporting| exporting.session == *session && exporting.writing.is_none())
        else {
            return;
        };
        if !page.redacted {
            // A core that does not know `redacted` read the items as they
            // are: provider blocks, and credentials only the core can find.
            self.exporting = None;
            self.say(
                "The core did not mask the session, so nothing was written: it may be older \
                 than this client.",
            );
            return;
        }
        exporting.items.extend(page.items);
        exporting.running |= turn_runs || !page.incomplete.is_empty();
        if let Some(cursor) = page.next_cursor {
            self.read_for_export(session.clone(), Some(cursor));
            return;
        }
        let title = self.session.as_ref().and_then(|state| state.title.clone());
        let items = std::mem::take(&mut exporting.items);
        let (format, path, running) = (exporting.format, exporting.path.clone(), exporting.running);
        let id = session.as_str().to_owned();
        let Some(writer) = self.writer.clone() else {
            self.exporting = None;
            return;
        };
        let (send, answer) = mpsc::channel();
        let waker = self.waker.clone();
        let spawned = std::thread::Builder::new()
            .name("cineris-export".to_owned())
            .spawn(move || {
                // The client's pass and the text are made here: a long
                // session's are not the interface's to wait on.
                let export = Export::new(&id, title.as_deref(), items, running);
                let (text, masked) = export.text(format);
                let file = ExportFile {
                    path,
                    session: id,
                    format,
                    text,
                };
                let _ = send.send((writer(file), masked));
                if let Some(waker) = waker {
                    waker.wake();
                }
            });
        match spawned {
            Ok(_) => exporting.writing = Some(answer),
            Err(error) => {
                self.exporting = None;
                self.say(&format!("The export could not be started: {error}."));
            }
        }
    }

    /// The read for the export failed: it ends, and why is said.
    pub(super) fn export_unread(&mut self, session: &SessionId) {
        if self
            .exporting
            .as_ref()
            .is_some_and(|exporting| exporting.session == *session)
        {
            self.exporting = None;
        }
    }

    /// Takes the writer's answer, when it has come: where the file went
    /// and how many values are masked in it, or why nothing was written.
    pub(super) fn take_export(&mut self) {
        self.settle_export(false);
    }

    /// Takes the writer's answer, waiting for it with `wait`. Says whether
    /// there was one to take.
    fn settle_export(&mut self, wait: bool) -> bool {
        let Some(answer) = self
            .exporting
            .as_ref()
            .and_then(|exporting| exporting.writing.as_ref())
        else {
            return false;
        };
        let answer = if wait {
            answer.recv().map_err(|_| TryRecvError::Disconnected)
        } else {
            answer.try_recv()
        };
        let sentence = match answer {
            Err(TryRecvError::Empty) => return false,
            Ok((Ok(path), masked)) => format!(
                "Exported the session to {}. {}.",
                crate::export::readable(&path),
                crate::export::masked_sentence(masked)
            ),
            Ok((Err(ExportError::Exists(path)), _)) => format!(
                "{} is there already: nothing was written. Name another file.",
                crate::export::readable(&path)
            ),
            Ok((Err(ExportError::Failed(why)), _)) => format!(
                "The export could not be written, and nothing of it was kept: {}",
                crate::export::readable(&why)
            ),
            Err(TryRecvError::Disconnected) => {
                "The export could not be written: its worker ended.".to_owned()
            }
        };
        self.exporting = None;
        self.say(&sentence);
        true
    }

    /// Gives the application a writer of `/export`'s files, for tests.
    #[cfg(test)]
    pub(crate) fn set_export_writer(&mut self, writer: crate::export::Writer) {
        self.writer = Some(writer);
    }

    /// Waits for the worker's answer and takes it, for tests: the worker
    /// always answers, or ends, so the wait has an end without a clock.
    /// Says whether an export was being written.
    #[cfg(test)]
    pub(crate) fn wait_for_export(&mut self) -> bool {
        self.settle_export(true)
    }
}
