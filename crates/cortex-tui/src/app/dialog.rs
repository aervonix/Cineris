//! A dialog in the input box's place: the request it answers, its keys, and
//! the guard that keeps typing meant for the editor from answering it
//! (`docs/steps/23/answered.md`, point 2).
//!
//! **Only keys meant for it answer it.** A dialog can appear while the
//! person is typing: an approval while a steering message is being
//! written. A key read before the dialog's first frame was written goes on
//! to the editor, where it was going. **When the person was typing as it
//! appeared** (a key read less than [`GRACE`] before its first frame), keys
//! read less than [`GRACE`] after that frame go to the editor too: they
//! were typed before the person could see it. Otherwise the dialog takes
//! keys at once. A paste never answers a dialog. All of these are
//! comparisons of times, so no timer runs.

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

/// How long after a dialog is first drawn a key still goes to the editor.
pub(crate) const GRACE: Duration = Duration::from_millis(300);

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

/// One dialog. See the [module documentation](self).
#[derive(Debug, Clone)]
pub(crate) struct Dialog {
    pub(crate) id: RequestId,
    pub(crate) asks: Asks,
    /// When its first frame was written.
    pub(crate) shown: Option<Instant>,
    /// Whether the person was typing as it appeared: then it takes keys
    /// only from [`GRACE`] after its first frame.
    pub(crate) typing: bool,
    /// The note being written, after "deny with a note".
    note: Option<String>,
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
    /// Show the request's detail whole.
    ShowWhole,
    /// Nothing.
    Nothing,
}

impl Dialog {
    pub(crate) fn approval(id: RequestId, request: ApprovalRequest) -> Self {
        let diff = match &request.detail {
            ApprovalDetail::Diff(diff) => tools::diff_lines(&diff.diff),
            _ => Vec::new(),
        };
        Self {
            id,
            asks: Asks::Approval { request, diff },
            shown: None,
            typing: false,
            note: None,
        }
    }

    /// The call the dialog is about, if it is about one.
    pub(crate) fn call(&self) -> Option<&cortex_protocol::CallId> {
        match &self.asks {
            Asks::Approval { request, .. } => Some(&request.call_id),
        }
    }

    /// Whether a key read at `at` may answer the dialog.
    fn takes_keys_at(&self, at: Instant) -> bool {
        self.shown
            .is_some_and(|shown| at >= shown && (!self.typing || at >= shown + GRACE))
    }

    /// Its first frame was written at `at`; the last key before it was read
    /// at `last_key`.
    pub(crate) fn drawn(&mut self, at: Instant, last_key: Option<Instant>) {
        if self.shown.is_none() {
            self.shown = Some(at);
            self.typing = last_key.is_some_and(|key| at.saturating_duration_since(key) < GRACE);
        }
    }

    /// A key, read at `at`.
    pub(crate) fn key(&mut self, key: Key, at: Instant, keymap: &KeyMap) -> Keyed {
        if !self.takes_keys_at(at) {
            return Keyed::Pass;
        }
        if let Some(note) = &mut self.note {
            return match keymap.lookup(&[Context::Editor], key) {
                Some(Action::Submit) => {
                    let note = std::mem::take(note);
                    self.note = None;
                    Keyed::Answer(ServerResponse::Approval(ApprovalResponse {
                        choice: ApprovalChoice::Deny,
                        note: (!note.trim().is_empty()).then_some(note),
                    }))
                }
                Some(Action::Stop) => {
                    self.note = None;
                    Keyed::Changed
                }
                Some(Action::Backspace) => {
                    note.pop();
                    Keyed::Changed
                }
                Some(Action::Interrupt) => Keyed::Global(Action::Interrupt),
                _ => match typed(key) {
                    Some(c) => {
                        note.push(c);
                        Keyed::Changed
                    }
                    None => Keyed::Nothing,
                },
            };
        }
        let Some(action) = keymap.lookup(&[Context::Dialog], key) else {
            return Keyed::Nothing;
        };
        let offered = |choice: ApprovalChoice| match &self.asks {
            Asks::Approval { request, .. } => request.choices.contains(&choice),
        };
        let answer = |choice: ApprovalChoice| {
            Keyed::Answer(ServerResponse::Approval(ApprovalResponse {
                choice,
                note: None,
            }))
        };
        match action {
            Action::AllowOnce if offered(ApprovalChoice::AllowOnce) => {
                answer(ApprovalChoice::AllowOnce)
            }
            Action::AllowSession if offered(ApprovalChoice::AllowSession) => {
                answer(ApprovalChoice::AllowSession)
            }
            Action::AllowProject if offered(ApprovalChoice::AllowProject) => {
                answer(ApprovalChoice::AllowProject)
            }
            // Denying is always on offer: it is what no answer is.
            Action::Deny => answer(ApprovalChoice::Deny),
            Action::DenyWithNote => {
                self.note = Some(String::new());
                Keyed::Changed
            }
            Action::ShowWhole => Keyed::ShowWhole,
            Action::Interrupt | Action::Redraw | Action::Suspend | Action::ToggleRound => {
                Keyed::Global(action)
            }
            _ => Keyed::Nothing,
        }
    }

    /// The keys the hint row shows while the dialog is open.
    pub(crate) fn hint_keys(&self, keymap: &KeyMap) -> Vec<String> {
        let key = |action: Action, fallback: &str| {
            keymap
                .keys_of(action)
                .into_iter()
                .map(name)
                .find(|text| text == "esc")
                .or_else(|| keymap.keys_of(action).into_iter().next().map(name))
                .unwrap_or_else(|| fallback.to_owned())
        };
        if self.note.is_some() {
            return vec![
                "enter denies with the note".to_owned(),
                "esc goes back".to_owned(),
            ];
        }
        vec![
            format!("{} denies", key(Action::Deny, "esc")),
            format!("{} shows it whole", key(Action::ShowWhole, "ctrl+o")),
        ]
    }

    /// The dialog's rows, `width` columns wide, its detail in at most
    /// `detail_rows` rows.
    pub(crate) fn view(
        &self,
        keymap: &KeyMap,
        detail_rows: usize,
        width: usize,
        look: &Look,
    ) -> Vec<Line> {
        match &self.asks {
            Asks::Approval { request, diff } => {
                let key = |action: Action| {
                    keymap
                        .keys_of(action)
                        .into_iter()
                        .map(name)
                        .find(|text| text.chars().count() == 1)
                        .or_else(|| keymap.keys_of(action).into_iter().next().map(name))
                        .unwrap_or_default()
                };
                let outside = dialog::leaves_sandbox(request);
                let mut answers = Vec::new();
                for choice in &request.choices {
                    let (action, label) = match choice {
                        ApprovalChoice::AllowOnce if outside => {
                            (Action::AllowOnce, "Run once outside the sandbox")
                        }
                        ApprovalChoice::AllowOnce => (Action::AllowOnce, "Allow once"),
                        ApprovalChoice::AllowSession => {
                            (Action::AllowSession, "Allow this session")
                        }
                        ApprovalChoice::AllowProject => (Action::AllowProject, "Always allow here"),
                        ApprovalChoice::Deny => (Action::Deny, "Deny"),
                        _ => continue,
                    };
                    answers.push(Answer {
                        key: key(action),
                        label: label.to_owned(),
                    });
                }
                if !outside && request.choices.contains(&ApprovalChoice::Deny) {
                    answers.push(Answer {
                        key: key(Action::DenyWithNote),
                        label: "Deny with a note".to_owned(),
                    });
                }
                dialog::approval(
                    request,
                    &ApprovalView {
                        answers,
                        diff: diff.clone(),
                        detail_rows,
                        note: self.note.as_deref(),
                    },
                    width,
                    look,
                )
            }
        }
    }
}

#[cfg(test)]
mod tests;
