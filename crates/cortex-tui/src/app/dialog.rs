//! A dialog in the input box's place: the request it answers, its keys, and
//! the guard that keeps typing meant for the editor from answering it
//! (`docs/steps/23/answered.md`, "The design, before building", point 2).
//!
//! **Only keys meant for it answer it.** A dialog can appear while the
//! person is typing: an approval while a steering message is being
//! written. So a dialog takes no key until [`GRACE`] after its first frame
//! was written, and until a frame with its answers has been drawn, which
//! is from then: what can be pressed is what was shown. After that, a key
//! still answers only after a pause: one read less than [`GRACE`] after the
//! key before it carries on the typing, whatever the dialog shows. While it
//! does not take keys, what types a character goes on to the editor, where
//! it was going, and the dialog's own other keys (`Esc`) do nothing, so
//! that a key meant to deny never interrupts the turn instead. Once it
//! takes keys, a key it has no use for does nothing. A paste never answers
//! a dialog. All of these are comparisons of times; the one wake the grace
//! needs is its end, to draw the answers ([`Dialog::deadline`]).
//!
//! **Only what was seen can be pressed, and allowed for longer.** Only an
//! answer drawn whole answers (and the deny key, which the hint row shows).
//! A request to leave the sandbox, and one any of whose rows did not fit,
//! offer no lasting answer: a rule that would last is never added for
//! something the person could not read in full. Keys that come in one read
//! were typed together, and none of them answers ([`super::App::burst`]).
//!
//! **It fits the window.** The question and the answers wrap, and are never
//! cut. The line of what a lasting answer would allow gives way first, then
//! the detail, cut with a line that says how much more there is. In a window
//! too small even for that, the rows between the question and the answers
//! are dropped; in one too small for those two, no answer is taken until it
//! grows.
//!
//! **A note is the note's.** Once "deny with a note" is chosen, keys and a
//! paste go to the note, and its row shows the note's end.

use std::time::{Duration, Instant};

use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalRequest, ApprovalResponse, ServerResponse,
};
use cortex_term::{Key, Line};

use super::tools;
use crate::keys::{Action, Context, KeyMap, name, typed};
use crate::view::dialog::{self, Answer, ApprovalView};
use crate::view::{DiffLine, Look};

/// How long after a dialog is first drawn, and after the key before, a key
/// still goes to the editor.
pub(crate) const GRACE: Duration = Duration::from_millis(300);

/// The fewest rows a dialog's detail is given.
const LEAST_DETAIL: usize = 2;

/// What a dialog asks.
#[derive(Debug, Clone)]
pub(crate) enum Asks {
    /// May this tool call run?
    Approval {
        request: ApprovalRequest,
        /// The diff's lines, parsed once.
        diff: Vec<DiffLine>,
    },
}

/// What a dialog's rows were drawn for: the same again draws the same.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Drawn {
    rows: usize,
    width: usize,
    ready: bool,
    note: Option<String>,
}

/// One dialog. See the [module documentation](self).
#[derive(Debug, Clone)]
pub(crate) struct Dialog {
    pub(crate) id: RequestId,
    pub(crate) asks: Asks,
    /// When its first frame was written.
    pub(crate) shown: Option<Instant>,
    /// A frame with its answers has been drawn.
    answers_drawn: bool,
    /// It asks to leave the sandbox of a session whose commands are
    /// confined.
    leaving: bool,
    /// The answers drawn whole the last time they were drawn: only their
    /// keys answer (and the deny key, which the hint row shows).
    offered: Vec<Action>,
    /// The window was too small for its question and its answers the last
    /// time they were to be drawn: no answer is taken until it is not.
    unreadable: bool,
    /// The note being written, after "deny with a note".
    note: Option<String>,
    /// Its rows as last drawn, and what for.
    cache: Option<(Drawn, Vec<Line>)>,
}

/// What a key did to a dialog.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Keyed {
    /// It was not the dialog's: the editor takes it.
    Pass,
    /// A global action: the application does it.
    Global(Action),
    /// The dialog changed: draw it again.
    Changed,
    /// The dialog is answered.
    Answer(ServerResponse),
    /// Nothing.
    Nothing,
}

impl Dialog {
    /// An approval, in a session whose commands are `confined` or not.
    pub(crate) fn approval(id: RequestId, request: ApprovalRequest, confined: bool) -> Self {
        let diff = match &request.detail {
            ApprovalDetail::Diff(diff) => tools::diff_lines(&diff.diff),
            _ => Vec::new(),
        };
        Self {
            id,
            leaving: confined && dialog::outside_sandbox(&request),
            asks: Asks::Approval { request, diff },
            shown: None,
            answers_drawn: false,
            offered: Vec::new(),
            unreadable: false,
            note: None,
            cache: None,
        }
    }

    /// The call the dialog is about, if it is about one.
    pub(crate) fn call(&self) -> Option<&cortex_protocol::CallId> {
        match &self.asks {
            Asks::Approval { request, .. } => Some(&request.call_id),
        }
    }

    /// Its first frame was written at `at`.
    pub(crate) fn drawn(&mut self, at: Instant) {
        if self.shown.is_none() {
            self.shown = Some(at);
        }
    }

    /// The look changed: the rows are drawn again.
    pub(crate) fn restyled(&mut self) {
        self.cache = None;
    }

    /// When a frame is due for the dialog's answers: the end of its grace,
    /// until a frame with them has been drawn. The time may have passed: a
    /// frame is then due at once.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        if self.answers_drawn || self.unreadable {
            return None;
        }
        self.shown.map(|shown| shown + GRACE)
    }

    /// Whether a key read at `at` may answer the dialog, the key before it
    /// having been read at `previous`: once its answers have been drawn,
    /// which is [`GRACE`] after its first frame, and after a pause.
    fn takes_keys_at(&self, at: Instant, previous: Option<Instant>) -> bool {
        self.answers_drawn
            && previous.is_none_or(|previous| at.saturating_duration_since(previous) >= GRACE)
    }

    /// A key, read at `at`, the key before it at `previous`.
    pub(crate) fn key(
        &mut self,
        key: Key,
        at: Instant,
        previous: Option<Instant>,
        keymap: &KeyMap,
    ) -> Keyed {
        // A global action is the application's whenever it comes: `Ctrl+C`
        // interrupts the turn, which withdraws the question.
        if let Some(
            action @ (Action::Interrupt | Action::Redraw | Action::Suspend | Action::ToggleRound),
        ) = keymap.lookup(&[], key)
        {
            return Keyed::Global(action);
        }
        // The note was asked for with a key the dialog took: what is typed
        // after it is the note's, at any pace.
        if let Some(note) = &mut self.note {
            return match keymap.lookup(&[Context::Editor], key) {
                Some(Action::Submit) => {
                    let note = std::mem::take(note);
                    self.note = None;
                    self.cache = None;
                    Keyed::Answer(ServerResponse::Approval(ApprovalResponse {
                        choice: ApprovalChoice::Deny,
                        note: (!note.trim().is_empty()).then_some(note),
                    }))
                }
                Some(Action::Stop) => {
                    self.note = None;
                    self.cache = None;
                    Keyed::Changed
                }
                Some(Action::Backspace) => {
                    note.pop();
                    self.cache = None;
                    Keyed::Changed
                }
                _ => match typed(key) {
                    Some(c) => {
                        note.push(c);
                        self.cache = None;
                        Keyed::Changed
                    }
                    None => Keyed::Nothing,
                },
            };
        }
        let action = keymap.lookup(&[Context::Dialog], key);
        if !self.takes_keys_at(at, previous) {
            // Typing goes on where it was going; the dialog's own keys that
            // type nothing (`Esc`) do nothing yet.
            return if typed(key).is_none() && action.is_some_and(dialogs) {
                Keyed::Nothing
            } else {
                Keyed::Pass
            };
        }
        let Some(action) = action else {
            return Keyed::Nothing;
        };
        let answer = |choice: ApprovalChoice| {
            Keyed::Answer(ServerResponse::Approval(ApprovalResponse {
                choice,
                note: None,
            }))
        };
        // Only what was drawn whole can be pressed.
        let drawn = |action: Action| self.offered.contains(&action);
        match action {
            Action::AllowOnce if drawn(Action::AllowOnce) => answer(ApprovalChoice::AllowOnce),
            Action::AllowSession if drawn(Action::AllowSession) => {
                answer(ApprovalChoice::AllowSession)
            }
            Action::AllowProject if drawn(Action::AllowProject) => {
                answer(ApprovalChoice::AllowProject)
            }
            // Denying is always on offer: it is what no answer is, and the
            // hint row shows its key.
            Action::Deny => answer(ApprovalChoice::Deny),
            Action::DenyWithNote if drawn(Action::DenyWithNote) => {
                self.note = Some(String::new());
                self.cache = None;
                Keyed::Changed
            }
            _ => Keyed::Nothing,
        }
    }

    /// A paste: the note's while one is being written, and then it says
    /// so; otherwise never the dialog's.
    pub(crate) fn paste(&mut self, text: &str) -> bool {
        let Some(note) = &mut self.note else {
            return false;
        };
        note.push_str(text);
        self.cache = None;
        true
    }

    /// The keys the hint row shows while the dialog is open.
    pub(crate) fn hint_keys(&self, keymap: &KeyMap) -> Vec<String> {
        if self.note.is_some() {
            return vec![
                "enter denies with the note".to_owned(),
                "esc goes back".to_owned(),
            ];
        }
        let deny = keymap
            .keys_of(Action::Deny)
            .into_iter()
            .map(name)
            .find(|text| text == "esc")
            .or_else(|| keymap.keys_of(Action::Deny).into_iter().next().map(name))
            .unwrap_or_else(|| "esc".to_owned());
        vec![format!("{deny} denies")]
    }

    /// The dialog's rows at `now`, at most `rows` of them and `width`
    /// columns wide. Its answers are drawn once they can be pressed.
    pub(crate) fn view(
        &mut self,
        keymap: &KeyMap,
        rows: usize,
        width: usize,
        look: &Look,
        now: Instant,
    ) -> Vec<Line> {
        let ready = self.shown.is_some_and(|shown| now >= shown + GRACE);
        let drawn_for = Drawn {
            rows,
            width,
            ready,
            note: self.note.clone(),
        };
        if let Some((key, lines)) = &self.cache
            && *key == drawn_for
        {
            return lines.clone();
        }
        let (lines, readable, offered) = self.draw(keymap, rows, width, look, ready);
        if ready {
            self.answers_drawn = readable;
            self.unreadable = !readable;
            self.offered = offered;
        }
        self.cache = Some((drawn_for, lines.clone()));
        lines
    }

    /// Draws the dialog to fit `rows`. Says whether its question and its
    /// answers could both be shown, and which answers were drawn whole.
    fn draw(
        &self,
        keymap: &KeyMap,
        rows: usize,
        width: usize,
        look: &Look,
        ready: bool,
    ) -> (Vec<Line>, bool, Vec<Action>) {
        let Asks::Approval { request, diff } = &self.asks;
        // Rows are kept for every answer there could be; fewer take no more.
        let every = self.answers(keymap, false);
        let mut view = ApprovalView {
            answers: every.iter().map(|(_, answer)| answer.clone()).collect(),
            answers_hidden: !ready,
            diff,
            detail_rows: rows.max(LEAST_DETAIL),
            note: self.note.as_deref(),
            leaving: self.leaving,
            withheld: false,
        };
        // The note of what a lasting answer would allow gives way first, so
        // that the detail stays whole (no lasting answer is offered then),
        // and then the detail.
        let mut fitted = dialog::approval(request, &view, width, look);
        loop {
            let over = fitted.lines.len().saturating_sub(rows);
            if over == 0 {
                break;
            }
            let room = fitted.detail.min(view.detail_rows);
            if !view.withheld && !request.adds.is_empty() && !self.leaving {
                view.withheld = true;
            } else if room > LEAST_DETAIL {
                view.detail_rows = room.saturating_sub(over).max(LEAST_DETAIL);
            } else {
                break;
            }
            fitted = dialog::approval(request, &view, width, look);
        }
        let fits = fitted.lines.len() <= rows;
        // A lasting answer is offered only for what was seen whole, itself
        // included.
        let lasting_cut = every
            .iter()
            .zip(&fitted.whole)
            .any(|((action, _), whole)| !whole && lasting(*action));
        let clipped = fitted.clipped || view.withheld || !fits || lasting_cut;
        let mut drawn = fitted;
        let mut offered = Vec::new();
        if ready {
            let answers = self.answers(keymap, clipped);
            view.answers = answers.iter().map(|(_, answer)| answer.clone()).collect();
            view.answers_hidden = false;
            drawn = dialog::approval(request, &view, width, look);
            offered = answers
                .iter()
                .zip(&drawn.whole)
                .filter(|(_, whole)| **whole)
                .map(|((action, _), _)| *action)
                .collect();
        }
        let (question, answer_rows) = (drawn.question, drawn.answer_rows);
        let mut lines = drawn.lines;
        let mut readable = true;
        if lines.len() > rows {
            // Still too tall: its question, and its last rows, which hold
            // the answers. In a window too small for both, nothing it shows
            // can be pressed.
            readable = rows >= question + answer_rows;
            let head = question.min(rows);
            let tail = lines.split_off(lines.len() - (rows - head));
            lines.truncate(head);
            lines.extend(tail);
        }
        (lines, readable, offered)
    }

    /// The answers on offer, each with its action and its key as the key
    /// map names it.
    fn answers(&self, keymap: &KeyMap, clipped: bool) -> Vec<(Action, Answer)> {
        let key = |action: Action| {
            keymap
                .keys_of(action)
                .into_iter()
                .map(name)
                .find(|text| text.chars().count() == 1)
                .or_else(|| keymap.keys_of(action).into_iter().next().map(name))
                .unwrap_or_default()
        };
        let Asks::Approval { request, .. } = &self.asks;
        let lasting = !(self.leaving || clipped);
        let mut answers = Vec::new();
        for choice in &request.choices {
            let (action, label) = match choice {
                ApprovalChoice::AllowOnce if self.leaving => {
                    (Action::AllowOnce, "Run once outside the sandbox")
                }
                ApprovalChoice::AllowOnce => (Action::AllowOnce, "Allow once"),
                ApprovalChoice::AllowSession if lasting => {
                    (Action::AllowSession, "Allow this session")
                }
                ApprovalChoice::AllowProject if lasting => {
                    (Action::AllowProject, "Always allow here")
                }
                ApprovalChoice::Deny => (Action::Deny, "Deny"),
                _ => continue,
            };
            answers.push((
                action,
                Answer {
                    key: key(action),
                    label: label.to_owned(),
                },
            ));
        }
        if !self.leaving && request.choices.contains(&ApprovalChoice::Deny) {
            answers.push((
                Action::DenyWithNote,
                Answer {
                    key: key(Action::DenyWithNote),
                    label: "Deny with a note".to_owned(),
                },
            ));
        }
        answers
    }
}

/// Whether `action` would add a rule that lasts.
fn lasting(action: Action) -> bool {
    matches!(action, Action::AllowSession | Action::AllowProject)
}

/// Whether `action` is one of a dialog's own, not a global one.
fn dialogs(action: Action) -> bool {
    matches!(
        action,
        Action::AllowOnce
            | Action::AllowSession
            | Action::AllowProject
            | Action::Deny
            | Action::DenyWithNote
            | Action::ShowWhole
    )
}

#[cfg(test)]
mod tests;
