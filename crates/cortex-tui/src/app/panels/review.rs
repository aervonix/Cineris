//! The review panel (`docs/architecture/09-tui.md`, "Panels";
//! "The panels as built"): what the last turn
//! changed, and what the project's checks said of it.
//!
//! **What it gathers,** turn by turn from `turn.started`: each file an
//! `edit` or `write` changed (from the tool's result: its path and its
//! counts, the box in the viewer that holds the call), whether a command
//! ran and succeeded, the verification notices (a round starting, a round
//! ended, the checks offered), and the verdict `turn.completed` gives.
//!
//! **Opens** when the turn's checks start, end or are offered (they run at
//! the end of a turn that changed files), and when the turn ends having
//! changed files; without the focus. **Closes** when the next prompt is
//! sent (this client's, or any turn that starts), and with `x`.
//!
//! **`Enter` on a file** opens the viewer at the box that holds its last
//! call, as `/view n` does.

use cortex_protocol::SessionId;
use cortex_protocol::event::{Event, Notice};
use cortex_protocol::item::{Item, ToolOutcome, ToolResultItem};
use cortex_term::Line;
use serde_json::Value;

use super::{Act, Draw, ItemKey, Panel, Seen, Show};
use crate::keys::{Action, KeyMap, name};
use crate::view::panel::PanelRow;
use crate::view::review::{self as views, ChangedFile, Review};

/// The review panel. See the [module documentation](self).
#[derive(Debug, Default)]
pub(crate) struct ReviewPanel {
    /// The turn gathered so far.
    turn: Review,
    /// Each file's box in the viewer, counted from 1.
    rounds: Vec<Option<u64>>,
}

impl ReviewPanel {
    fn reset(&mut self) {
        self.turn = Review::default();
        self.rounds.clear();
    }

    /// A tool's result: a file changed, or a command that ran.
    fn tool_result(&mut self, result: &ToolResultItem, round: Option<u64>) {
        if result.outcome != ToolOutcome::Success {
            return;
        }
        let payload = result.payload.as_ref().unwrap_or(&Value::Null);
        match result.tool.as_str() {
            "shell" => self.turn.commands = true,
            "edit" | "write" => {
                if payload.get("kind").and_then(Value::as_str) == Some("unchanged") {
                    return;
                }
                let Some(path) = payload.get("path").and_then(Value::as_str) else {
                    return;
                };
                let added = payload.get("added").and_then(Value::as_u64);
                let removed = payload.get("removed").and_then(Value::as_u64);
                let sum = |had: Option<u64>, more: Option<u64>| match (had, more) {
                    (Some(had), Some(more)) => Some(had.saturating_add(more)),
                    (had, more) => had.or(more),
                };
                match self.turn.files.iter().position(|file| file.path == path) {
                    Some(at) => {
                        let file = &mut self.turn.files[at];
                        file.added = sum(file.added, added);
                        file.removed = sum(file.removed, removed);
                        // Its last call is the one the viewer opens at.
                        if round.is_some() {
                            self.rounds[at] = round;
                        }
                    }
                    None => {
                        self.turn.files.push(ChangedFile {
                            path: path.to_owned(),
                            added,
                            removed,
                        });
                        self.rounds.push(round);
                    }
                }
            }
            _ => {}
        }
    }

    /// Something to review: a file changed, or checks seen.
    fn has_any(&self) -> bool {
        !self.turn.files.is_empty() || self.turn.checks.is_some()
    }
}

impl Panel for ReviewPanel {
    fn name(&self) -> &'static str {
        "review"
    }

    fn seen(&mut self, seen: &Seen<'_>) -> Show {
        match seen {
            Seen::Event { event, round, .. } => match event {
                // The next prompt's turn: the last one's review goes.
                Event::TurnStarted(_) => {
                    self.reset();
                    Show::Close
                }
                Event::ItemCompleted(completed) => {
                    if let Item::ToolResult(result) = &completed.item {
                        self.tool_result(result, *round);
                    }
                    Show::Same
                }
                Event::Notice(Notice::Verification(notice)) => {
                    self.turn.checks = Some(notice.clone());
                    Show::Open
                }
                Event::TurnCompleted(completed) => {
                    self.turn.verdict.clone_from(&completed.verification);
                    if self.has_any() {
                        Show::Open
                    } else {
                        Show::Same
                    }
                }
                _ => Show::Same,
            },
            Seen::Prompt => Show::Close,
            Seen::Left => {
                self.reset();
                Show::Close
            }
        }
    }

    fn collapsed(&self, draw: &Draw<'_>) -> Vec<Line> {
        views::review_collapsed(&self.turn, draw.width, draw.look)
    }

    fn expanded(&self, focused: usize, draw: &Draw<'_>) -> Vec<PanelRow> {
        views::review_rows(&self.turn, Some(focused), draw.width, draw.look, draw.tick)
    }

    fn items(&self) -> usize {
        self.turn.items()
    }

    fn item(&self, index: usize) -> Option<ItemKey> {
        if let Some(file) = self.turn.files.get(index) {
            return Some(ItemKey {
                id: format!("file\n{}", file.path),
                content: format!("{:?} {:?}", file.added, file.removed),
            });
        }
        let notice = self.turn.checks.as_ref()?;
        let at = index - self.turn.files.len();
        let check = notice.checks.get(at)?;
        Some(ItemKey {
            id: format!("check\n{at}\n{}", check.command),
            content: format!("{:?} {:?}", notice.phase, check),
        })
    }

    fn hint(&self, focused: usize, keymap: &KeyMap) -> Vec<String> {
        let key = |action: Action| keymap.keys_of(action).into_iter().next().map(name);
        let mut keys = Vec::new();
        // PR 3: `Enter` on a check opens its output (`CheckReport.artifact`)
        // in the viewer.
        if focused < self.turn.files.len()
            && let Some(open) = key(Action::FocusActivate)
        {
            keys.push(format!("{open} opens it in the viewer"));
        }
        if let Some(close) = key(Action::FocusClose) {
            keys.push(format!("{close} closes"));
        }
        if let Some(back) = key(Action::FocusReturn) {
            keys.push(format!("{back} back to the input"));
        }
        keys
    }

    fn act(&mut self, action: Action, _focused: usize, _session: Option<&SessionId>) -> Act {
        match action {
            Action::FocusClose => Act::Close,
            _ => Act::Pass,
        }
    }

    fn activate(&mut self, focused: usize) -> Act {
        match self.rounds.get(focused) {
            Some(round) => Act::View(round.and_then(|round| usize::try_from(round).ok())),
            // PR 3: a check's output in the viewer.
            None => Act::Taken,
        }
    }
}
