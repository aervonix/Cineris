//! `/export` in the application (since step 25): the session read for an
//! export, page by page, through the `Link` (`session/read` with
//! `redacted`, a read beside the lane, so `cineris attach` exports the same
//! way); then the client's pass and the file's text ([`crate::export`]);
//! then the program's writer, on a thread of its own, which wakes the loop
//! when it is done.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use cineris_protocol::methods::{Op, SessionReadParams, SessionReadResult};
use cineris_protocol::{Item, SessionId};

use super::{App, HISTORY_PAGE, Pending};
use crate::export::{Export, ExportError, ExportFile, Format};

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
    /// How many values the core said are masked in them.
    masked: u32,
    /// A turn was running: what it had not finished was left out.
    running: bool,
    /// The writer's answer, once the text is with it, and how many values
    /// are masked in what it writes.
    writing: Option<(Receiver<Result<String, ExportError>>, usize)>,
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
            masked: 0,
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

    /// A page read for the export: the next is asked for, or, at the last,
    /// the text is made and given to the writer.
    pub(super) fn export_read(&mut self, session: &SessionId, page: SessionReadResult) {
        let Some(exporting) = self
            .exporting
            .as_mut()
            .filter(|exporting| exporting.session == *session && exporting.writing.is_none())
        else {
            return;
        };
        exporting.items.extend(page.items);
        exporting.masked = exporting.masked.saturating_add(page.masked);
        exporting.running |= !page.incomplete.is_empty();
        if let Some(cursor) = page.next_cursor {
            self.read_for_export(session.clone(), Some(cursor));
            return;
        }
        let title = self.session.as_ref().and_then(|state| state.title.clone());
        let items = std::mem::take(&mut exporting.items);
        let export = Export::new(
            session.as_str(),
            title.as_deref(),
            items,
            exporting.masked,
            exporting.running,
        );
        let file = ExportFile {
            path: exporting.path.clone(),
            session: session.as_str().to_owned(),
            format: exporting.format,
            text: export.text(exporting.format),
        };
        let Some(writer) = self.writer.clone() else {
            self.exporting = None;
            return;
        };
        let (send, answer) = mpsc::channel();
        let waker = self.waker.clone();
        let spawned = std::thread::Builder::new()
            .name("cineris-export".to_owned())
            .spawn(move || {
                let _ = send.send(writer(file));
                if let Some(waker) = waker {
                    waker.wake();
                }
            });
        match spawned {
            Ok(_) => exporting.writing = Some((answer, export.masked)),
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
        let Some((answer, masked)) = self
            .exporting
            .as_ref()
            .and_then(|exporting| exporting.writing.as_ref())
        else {
            return;
        };
        let masked = *masked;
        let sentence = match answer.try_recv() {
            Err(TryRecvError::Empty) => return,
            Ok(Ok(path)) => format!(
                "Exported the session to {}. {}.",
                crate::export::readable(&path),
                crate::export::masked_sentence(masked)
            ),
            Ok(Err(ExportError::Exists(path))) => format!(
                "{} is there already: nothing was written. Name another file.",
                crate::export::readable(&path)
            ),
            Ok(Err(ExportError::Failed(why))) => format!(
                "The export could not be written, and nothing of it was kept: {}",
                crate::export::readable(&why)
            ),
            Err(TryRecvError::Disconnected) => {
                "The export could not be written: its worker ended.".to_owned()
            }
        };
        self.exporting = None;
        self.say(&sentence);
    }

    /// Gives the application a writer of `/export`'s files, for tests.
    #[cfg(test)]
    pub(crate) fn set_export_writer(&mut self, writer: crate::export::Writer) {
        self.writer = Some(writer);
    }

    /// Waits for the writer, for tests.
    #[cfg(test)]
    pub(crate) fn wait_for_export(&mut self, within: std::time::Duration) -> bool {
        let until = std::time::Instant::now() + within;
        while std::time::Instant::now() < until {
            let waiting = self
                .exporting
                .as_ref()
                .is_some_and(|exporting| exporting.writing.is_some());
            if !waiting {
                return true;
            }
            self.take_export();
            std::thread::yield_now();
        }
        false
    }
}
