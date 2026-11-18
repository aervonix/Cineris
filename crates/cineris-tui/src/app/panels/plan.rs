//! The plan panel (`docs/architecture/09-tui.md`, "Panels" and "Editing the
//! plan"; screen 4): the plan the model keeps with its `plan` tool, which
//! the person can see and change.
//!
//! **Opens** when the plan has items, from `plan.updated` and from
//! `plan/get` when a stored session is opened, without the focus; `/plan`
//! opens it with the focus. **Closes** when the plan is empty, or when no
//! item is left to do (each is done or skipped) and the turn has ended:
//! while a turn runs, a plan all done stays.
//!
//! **Its keys,** each one `plan/update` with one edit, in the session open:
//! `space` skips the focused item, `e` rewords it and `a` adds one after it
//! (both in the prompt's own editor), `d` deletes it, `alt+up` and
//! `alt+down` move it. The plan the result gives replaces the one shown.
//! Restoring a skipped item needs `PlanEdit::Restore`, which the protocol
//! does not have yet: `space` on a skipped item does nothing, and the hint
//! row does not offer it.

use cineris_protocol::SessionId;
use cineris_protocol::event::Event;
use cineris_protocol::item::{Plan, PlanStatus};
use cineris_protocol::methods::{Op, OpResult, PlanUpdateParams, SessionRef};
use cineris_protocol::project::{PlanAdd, PlanEdit, PlanItemRef, PlanReorder, PlanReword};
use cineris_protocol::session::SessionState;
use cineris_term::Line;

use super::{Act, Draw, ItemEditFor, ItemKey, Panel, Seen, Show};
use crate::keys::{Action, KeyMap, name};
use crate::view::panel::PanelRow;
use crate::view::plan as views;

/// What `/plan` says when there is no plan to show.
pub(crate) const NO_PLAN: &str = "There is no plan yet: the model makes one with its plan tool.";

/// What `Enter` does in an item edit of the plan's, as the hint row says.
const SAVES: &str = "saves the item";

/// The most characters an item's text holds: the core refuses more
/// (`cineris-tools`'s `MAX_PLAN_TEXT`, which this crate may not name).
pub(crate) const ITEM_TEXT: usize = 400;

/// The plan panel. See the [module documentation](self).
#[derive(Debug, Default)]
pub(crate) struct PlanPanel {
    plan: Plan,
    /// Whether a turn runs: a plan all done stays open until it ends.
    running: bool,
}

impl PlanPanel {
    /// Takes `plan` when it is not older than the one shown, and says
    /// whether the panel opens or closes for it.
    fn take(&mut self, plan: &Plan) -> Show {
        if plan.revision < self.plan.revision {
            return Show::Same;
        }
        self.plan = plan.clone();
        self.show()
    }

    /// Open while it has items and something is left to do, or a turn
    /// runs.
    fn show(&self) -> Show {
        if self.plan.items.is_empty() || (self.finished() && !self.running) {
            Show::Close
        } else {
            Show::Open
        }
    }

    /// No item is left to do: each is done or skipped.
    fn finished(&self) -> bool {
        self.plan
            .items
            .iter()
            .all(|item| matches!(item.status, PlanStatus::Done | PlanStatus::Skipped))
    }

    fn update(session: &SessionId, edit: PlanEdit) -> Act {
        Act::Send(Op::PlanUpdate(PlanUpdateParams {
            session_id: session.clone(),
            edits: vec![edit],
        }))
    }

    /// The order with the item at `from` moved to `to`.
    fn reorder(&self, from: usize, to: usize, session: &SessionId) -> Act {
        let mut order: Vec<_> = self.plan.items.iter().map(|item| item.id.clone()).collect();
        let moved = order.remove(from);
        order.insert(to, moved);
        Self::update(session, PlanEdit::Reorder(PlanReorder { order }))
    }
}

impl Panel for PlanPanel {
    fn name(&self) -> &'static str {
        "plan"
    }

    fn seen(&mut self, seen: &Seen<'_>) -> Show {
        match seen {
            Seen::Event { event, running, .. } => {
                self.running = *running;
                match event {
                    Event::PlanUpdated(updated) => self.take(&updated.plan),
                    // The turn has ended: a plan with nothing left to do
                    // goes.
                    Event::TurnCompleted(_) if self.finished() => Show::Close,
                    _ => Show::Same,
                }
            }
            Seen::Prompt => Show::Same,
            Seen::Left => {
                self.plan = Plan::default();
                self.running = false;
                Show::Close
            }
        }
    }

    fn result(&mut self, result: &OpResult) -> Show {
        match result {
            OpResult::PlanGet(result) | OpResult::PlanUpdate(result) => self.take(&result.plan),
            _ => Show::Same,
        }
    }

    fn when_opened(&mut self, state: &SessionState, stored: bool) -> Option<Op> {
        // A new session has no plan yet; a stored one may.
        stored.then(|| {
            Op::PlanGet(SessionRef {
                session_id: state.session_id.clone(),
            })
        })
    }

    fn summon(&mut self) -> Result<(), String> {
        if self.plan.items.is_empty() {
            Err(NO_PLAN.to_owned())
        } else {
            Ok(())
        }
    }

    fn collapsed(&self, draw: &Draw<'_>) -> Vec<Line> {
        views::plan_collapsed(&self.plan, draw.width, draw.look)
    }

    fn expanded(&self, focused: usize, draw: &Draw<'_>) -> Vec<PanelRow> {
        views::plan_rows(&self.plan, Some(focused), draw.width, draw.look)
    }

    fn items(&self) -> usize {
        self.plan.items.len()
    }

    fn least_rows(&self, focused: usize, _draw: &Draw<'_>) -> usize {
        // Its first row, and the focused item's one.
        1 + usize::from(focused < self.plan.items.len())
    }

    fn item(&self, index: usize) -> Option<ItemKey> {
        let item = self.plan.items.get(index)?;
        Some(ItemKey {
            id: item.id.as_str().to_owned(),
            content: format!("{}\n{}", item.status.as_str(), item.text),
        })
    }

    fn first_focus(&self) -> usize {
        views::current(&self.plan)
            .and_then(|current| {
                self.plan
                    .items
                    .iter()
                    .position(|item| item.id == current.id)
            })
            .unwrap_or(0)
    }

    fn hint(&self, focused: usize, keymap: &KeyMap) -> Vec<String> {
        let key = |action: Action| keymap.keys_of(action).into_iter().next().map(name);
        let mut keys = Vec::new();
        // `space` skips an item, and restores one that was skipped; a done
        // item it leaves as it is.
        let status = self.plan.items.get(focused).map(|item| &item.status);
        if let Some(skip) = key(Action::FocusSkip) {
            match status {
                Some(PlanStatus::Skipped) => keys.push(format!("{skip} restores")),
                Some(PlanStatus::Done) | None => {}
                Some(_) => keys.push(format!("{skip} skips")),
            }
        }
        if let Some(edit) = key(Action::FocusEdit) {
            keys.push(format!("{edit} edits"));
        }
        if let Some(add) = key(Action::FocusAdd) {
            keys.push(format!("{add} adds"));
        }
        if let Some(back) = key(Action::FocusReturn) {
            keys.push(format!("{back} back to the input"));
        }
        keys
    }

    fn act(&mut self, action: Action, focused: usize, session: Option<&SessionId>) -> Act {
        let Some(item) = self.plan.items.get(focused) else {
            return Act::Pass;
        };
        let Some(session) = session else {
            return Act::Pass;
        };
        let id = item.id.clone();
        match action {
            Action::FocusSkip => match item.status {
                PlanStatus::Done => Act::Taken,
                // Restored to what it was before it was skipped; the model
                // is told as it is of a skip.
                PlanStatus::Skipped => Self::update(session, PlanEdit::Restore(PlanItemRef { id })),
                _ => Self::update(session, PlanEdit::Skip(PlanItemRef { id })),
            },
            Action::FocusEdit => Act::Edit {
                text: item.text.clone(),
                hint: SAVES,
                edit: ItemEditFor {
                    action,
                    item: id.as_str().to_owned(),
                },
            },
            Action::FocusAdd => Act::Edit {
                text: String::new(),
                hint: SAVES,
                edit: ItemEditFor {
                    action,
                    item: id.as_str().to_owned(),
                },
            },
            Action::FocusDelete => Self::update(session, PlanEdit::Remove(PlanItemRef { id })),
            Action::FocusMoveUp if focused > 0 => self.reorder(focused, focused - 1, session),
            Action::FocusMoveDown if focused + 1 < self.plan.items.len() => {
                self.reorder(focused, focused + 1, session)
            }
            // At an end, a move has nowhere to go.
            Action::FocusMoveUp | Action::FocusMoveDown => Act::Taken,
            _ => Act::Pass,
        }
    }

    fn edited(&mut self, edit: &ItemEditFor, text: &str, session: Option<&SessionId>) -> Act {
        // An item is one line: the line breaks, tabs and other control
        // characters a paste or the external editor brought become spaces,
        // which the core takes (it refuses an item's text with any).
        let text: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let text = text.trim();
        let Some(session) = session else {
            return Act::Taken;
        };
        let Some(item) = self
            .plan
            .items
            .iter()
            .find(|item| item.id.as_str() == edit.item)
        else {
            return Act::Say(
                "The item is no longer in the plan, so the edit was not saved.".to_owned(),
            );
        };
        if text.is_empty() {
            return Act::Taken;
        }
        // Longer than an item holds, as a paste can make it: cut, and said,
        // rather than refused by the core with the text lost.
        let cut = text.chars().count() > ITEM_TEXT;
        let text: String = text.chars().take(ITEM_TEXT).collect();
        let text = text.trim_end();
        let act = match edit.action {
            Action::FocusEdit if text == item.text => Act::Taken,
            Action::FocusEdit => Self::update(
                session,
                PlanEdit::Reword(PlanReword {
                    id: item.id.clone(),
                    text: text.to_owned(),
                }),
            ),
            Action::FocusAdd => Self::update(
                session,
                PlanEdit::Add(PlanAdd {
                    text: text.to_owned(),
                    after: Some(item.id.clone()),
                }),
            ),
            _ => Act::Taken,
        };
        match act {
            Act::Send(op) if cut => Act::SendAndSay(
                op,
                format!(
                    "The item was cut to its first {ITEM_TEXT} characters, the most an item of \
                     the plan holds."
                ),
            ),
            act => act,
        }
    }
}
