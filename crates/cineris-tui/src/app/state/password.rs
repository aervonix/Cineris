//! The approvals password in the application (since the signed approvals;
//! `docs/architecture/09-tui.md`, "The approvals password"): a dialog in
//! the input box's place over a [`PasswordPrompt`], which the application
//! opens with [`App::ask_password`], and which hands the password to the
//! handler it was opened with.
//!
//! - **Keys go to it alone.** While it is open, every key, held key and
//!   paste is its own or nobody's: never the input box's, its history's,
//!   the draft's, the viewer's, a panel's, the first run's or another
//!   dialog's. A resize, a resumption, the focus and the mouse go on as
//!   they would. The viewer, which hides the inline screen, is closed when
//!   it opens.
//! - **The dialogs' rule** ([`Readiness`]): its field is drawn, and takes
//!   keys, no sooner than 300 ms after its first frame, a quiet second
//!   after the last input it did not take (the typing just before it came,
//!   the keys already queued when it opened, which it drops, and typing
//!   that runs on into it), and the repeat window after the key that
//!   answered what came before it; and only while every row fits the
//!   window. `Ctrl+C` answers it with nothing whenever it comes.
//! - **One at a time:** a password asked while one is open waits behind
//!   it.
//! - **The handler** is called once the dialog has gone, with the
//!   password, or `None` for `Esc` or `Ctrl+C`. A dialog still open when
//!   the application ends is dropped with it, its buffers wiped and its
//!   handler not called.
//! - **The terminal is closed to other programs** while one is open: the
//!   loop's draw asks for it ([`App::asking_password`]).

use std::time::Instant;

use cineris_base::Password;
use cineris_term::{Event as TermEvent, Line};

use super::{App, LastKey};
use crate::app::ready::{REPEAT, Readiness, scaled};
use crate::secret::password::is_ctrl_c;
use crate::secret::{PasswordAsk, PasswordPrompt, Prompted};
use crate::view::Hint;

/// What is done with the password once the dialog has gone.
type Handler = Box<dyn FnOnce(&mut App, Option<Password>) + Send>;

/// One password asked. See the [module documentation](self).
pub(super) struct PasswordDialog {
    prompt: PasswordPrompt,
    ready: Readiness,
    handler: Option<Handler>,
    /// The last frame drew its field ready to type into.
    field: bool,
    /// The last frame was due to draw its field and could not fit its
    /// rows: no frame is due for it until the window changes.
    small: bool,
}

impl std::fmt::Debug for PasswordDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordDialog")
            .field("prompt", &self.prompt)
            .field("ready", &self.ready)
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl App {
    /// Asks the person for the approvals password: a dialog in the input
    /// box's place says what `ask` asks, and once the person has answered
    /// it, and it has gone, `handler` is given the password, or `None` for
    /// `Esc` or `Ctrl+C`. While it is open, keys and pastes go to it alone.
    /// See `app::state::password`.
    pub fn ask_password(
        &mut self,
        ask: PasswordAsk,
        handler: impl FnOnce(&mut App, Option<Password>) + Send + 'static,
    ) {
        // The viewer hides the inline screen: closed, so that the dialog is
        // seen.
        if self.viewer.take().is_some() {
            self.panels.restart();
        }
        self.passwords.push_back(PasswordDialog {
            prompt: PasswordPrompt::new(ask),
            ready: Readiness::default(),
            handler: Some(Box::new(handler)),
            field: false,
            small: false,
        });
    }

    /// Whether a password dialog is open: the loop closes the terminal to
    /// other programs while one is.
    #[must_use]
    pub fn asking_password(&self) -> bool {
        !self.passwords.is_empty()
    }

    /// A terminal event while a password dialog is open: `None` when none
    /// is, or the event is not input, and it goes where it would otherwise.
    pub(super) fn password_event(&mut self, event: &TermEvent, at: Instant) -> Option<bool> {
        // Whether it was ready for the key is judged when the key was
        // pressed; the input it holds counts from `at`.
        let pressed = self.judged(at);
        let dialog = self.passwords.front_mut()?;
        let takes = dialog.ready.takes_keys(pressed);
        let prompted = match event {
            TermEvent::Key(key) if is_ctrl_c(*key) => dialog.prompt.cancel(),
            TermEvent::Key(key) if takes => dialog.prompt.key(*key),
            TermEvent::Repeat(key) if takes => dialog.prompt.repeat(*key),
            TermEvent::Paste(text) if takes => dialog.prompt.paste(text),
            // Before it takes keys: dropped, and it waits for the typing to
            // stop.
            TermEvent::Key(_) | TermEvent::Repeat(_) | TermEvent::Paste(_) => {
                dialog.ready.typed(at);
                dialog.field = false;
                Prompted::Nothing
            }
            // Continued after a stop: what was typed meanwhile was the
            // shell's, and it starts as if just shown.
            TermEvent::Resumed => {
                dialog.ready.hide();
                dialog.field = false;
                return None;
            }
            TermEvent::Resize { .. } => {
                dialog.small = false;
                return None;
            }
            _ => return None,
        };
        if let Prompted::Done(answer) = prompted {
            self.password_done(answer, at);
        }
        Some(true)
    }

    /// The front dialog was answered at `at`: it goes, and its handler is
    /// given `answer`.
    fn password_done(&mut self, answer: Option<Password>, at: Instant) {
        let Some(mut dialog) = self.passwords.pop_front() else {
            return;
        };
        // A key a dialog took: what comes next waits its repeat window.
        self.taken = Some(at);
        self.last_key = Some(LastKey { typing: false });
        self.input.disarm_quit();
        // What it hid starts again as if just shown.
        if let Some(next) = self.dialogs.front_mut() {
            next.hide();
        }
        self.first_run_resumed();
        self.panels.restart();
        let handler = dialog.handler.take();
        drop(dialog);
        if let Some(handler) = handler {
            handler(self, answer);
        }
    }

    /// A frame was written at `at`: the front dialog's first starts its
    /// grace, held by what was typed just before it and by the key that
    /// answered what came before it, as a dialog's is.
    pub(super) fn password_drawn(&mut self, at: Instant) {
        let (untaken, taken) = (self.untaken, self.taken);
        let Some(dialog) = self.passwords.front_mut() else {
            return;
        };
        if dialog.ready.drawn(at) {
            if let Some(untaken) = untaken {
                dialog.ready.typed(untaken);
            }
            if let Some(taken) = taken {
                dialog.ready.not_before(taken + scaled(REPEAT));
            }
        }
        dialog.ready.set_answers_drawn(dialog.field);
    }

    /// When a frame is due for the front dialog's field: the moment it is
    /// ready, until a frame has drawn it.
    pub(super) fn password_deadline(&self) -> Option<Instant> {
        let dialog = self.passwords.front()?;
        if dialog.field || dialog.small {
            return None;
        }
        dialog.ready.ready_at()
    }

    /// The front dialog's rows at `now`, `width` columns wide, with a blank
    /// row and the hint row under them while the window has room: `None`
    /// when none is open.
    pub(super) fn password_bottom(&mut self, now: Instant, width: usize) -> Option<Vec<Line>> {
        let chrome = self.rows >= 6;
        let rows = if chrome {
            self.rows - 2
        } else {
            self.rows.max(1)
        };
        let dialog = self.passwords.front_mut()?;
        let shown = dialog.ready.ready(now);
        let drawn = dialog.prompt.rows(width, rows, shown, &self.look);
        dialog.field = drawn.field;
        dialog.small = shown && !drawn.field;
        let keys = dialog.prompt.keys(drawn.field);
        let mut lines = drawn.lines;
        if chrome {
            lines.push(Line::new());
            lines.push(self.hint_row(Hint::Keys(keys), now));
        }
        Some(lines)
    }
}

#[cfg(test)]
mod tests;
