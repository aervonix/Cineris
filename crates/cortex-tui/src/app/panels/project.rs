//! The project panel (`docs/architecture/09-tui.md`, "Panels";
//! `docs/architecture/14-adaptation.md`, "Project knowledge"): the entries
//! the harness keeps of the project, for the person to review.
//!
//! **Opens** when entries wait for this person's review (a proposal, an
//! observed entry that became active by itself, an entry of the
//! repository's file not accepted here), from `project/list` when a session
//! is opened and from `project.updated`, collapsed and without the focus;
//! `/project` opens it with the focus, with every entry in use and waiting.
//! **Closes** when nothing waits and `/project` did not open it, and with
//! `x`.
//!
//! **Its keys,** each one `project/update` with one change, in the session
//! open: `y` accepts, `e` edits the text (in the prompt's own editor), `n`
//! turns the entry off, `d` deletes it, `p` promotes it to the repository's
//! file. The entries the result gives replace those shown; a deleted entry
//! goes when `project.updated` says it was removed, which the core sends
//! every session of the project, this one included. A key is offered, and
//! taken, only where the core would do it: no edit or delete of an entry of
//! the repository's file, and no promote of one not in use and accepted.
//!
//! - **`y` is taken only when the focused entry's whole detail was drawn**
//!   in the frame on screen: what a person accepts is what they were shown
//!   (a command whole, what for and where, and what accepting keeps it
//!   until). One whose detail did not fit offers no `y`, and the hint row
//!   says to make the window taller. The detail drawn whole for a new entry
//!   starts the readiness again: the grace is for what was just shown.
//! - **`d` asks first, in the entry's row**: `y` then deletes, and any
//!   other key keeps it, a move of the focus included. The question changes
//!   the focused row, so the panel's readiness starts again: its `y` is
//!   held to the same rule.
//! - **`p` while a turn of this session runs** is refused by the core (no
//!   promote writes the repository's file under a confined command): the
//!   hint row says so beside the key, and the refusal is said with the
//!   core's sentence.

use cortex_base::SharedClock;
use cortex_protocol::SessionId;
use cortex_protocol::event::Event;
use cortex_protocol::ids::EntryId;
use cortex_protocol::methods::{Op, OpResult, ProjectListParams, ProjectUpdateParams};
use cortex_protocol::project::{EntryAction, EntryChange, EntryStatus, ProjectEntry};
use cortex_protocol::session::{SessionState, TrustStanding};
use cortex_term::Line;

use super::{Act, Draw, ItemEditFor, ItemKey, Panel, Seen, Show};
use crate::keys::{Action, KeyMap, name};
use crate::view::panel::PanelRow;
use crate::view::project::{self as views, ProjectView, acceptable, promotable, shared, waits};

/// What `Enter` does in an item edit of the panel's, as the hint row says.
const SAVES: &str = "saves the entry";

/// What `y` says when the focused entry's detail did not fit.
pub(crate) const TALLER: &str =
    "The entry's whole detail is not on screen: make the window taller to accept it.";

/// The project panel. See the [module documentation](self).
#[derive(Debug)]
pub(crate) struct ProjectPanel {
    /// Every entry, as the last list and the changes since give them.
    entries: Vec<ProjectEntry>,
    /// Those shown, in order: waiting for review first, then in use.
    shown: Vec<ProjectEntry>,
    /// What is wrong with the project's files.
    problems: Vec<String>,
    /// Whether the entries have been listed for the session open.
    listed: bool,
    /// Whether `/project` opened it: it stays open with nothing waiting.
    summoned: bool,
    /// Whether this session's turn runs.
    running: bool,
    /// Whether the person trusts the project as it is.
    trusted: bool,
    /// The entry whose deletion is asked.
    asking: Option<EntryId>,
    /// The entry whose detail the last frame drew whole, with the focus.
    whole: Option<EntryId>,
    clock: SharedClock,
}

impl ProjectPanel {
    /// The panel, its times read from `clock`.
    pub(crate) fn new(clock: SharedClock) -> Self {
        Self {
            entries: Vec::new(),
            shown: Vec::new(),
            problems: Vec::new(),
            listed: false,
            summoned: false,
            running: false,
            trusted: false,
            asking: None,
            whole: None,
            clock,
        }
    }

    /// Puts the entries shown in order again: those waiting for review,
    /// then the rest in use.
    fn order(&mut self) {
        let in_use = |entry: &&ProjectEntry| {
            matches!(entry.status, EntryStatus::Proposed | EntryStatus::Active)
        };
        let waiting = self.entries.iter().filter(in_use).filter(|e| waits(e));
        let rest = self.entries.iter().filter(in_use).filter(|e| !waits(e));
        self.shown = waiting.chain(rest).cloned().collect();
    }

    /// Open while something waits, or `/project` opened it.
    fn show(&self) -> Show {
        if self.summoned || self.entries.iter().any(waits) {
            Show::Open
        } else {
            Show::Close
        }
    }

    /// Takes the entries `changed` as they are now, and drops `removed`.
    fn merge(&mut self, changed: &[ProjectEntry], removed: &[EntryId]) {
        self.entries.retain(|entry| !removed.contains(&entry.id));
        for entry in changed {
            match self.entries.iter_mut().find(|had| had.id == entry.id) {
                Some(had) => had.clone_from(entry),
                None => self.entries.push(entry.clone()),
            }
        }
        self.order();
    }

    fn view(&self) -> ProjectView<'_> {
        let asking = self
            .asking
            .as_ref()
            .and_then(|id| self.shown.iter().position(|entry| &entry.id == id));
        ProjectView {
            entries: &self.shown,
            now: self.clock.now(),
            asking,
            reading: !self.listed,
            problems: &self.problems,
            trusted: self.trusted,
        }
    }

    /// One change to the entry `id`, in `session`.
    fn change(session: &SessionId, id: &EntryId, action: EntryAction, text: Option<String>) -> Act {
        Act::Send(Op::ProjectUpdate(ProjectUpdateParams {
            session_id: Some(session.clone()),
            workspace: None,
            changes: vec![EntryChange {
                id: id.clone(),
                action,
                text,
            }],
            add: Vec::new(),
        }))
    }

    /// The list, for `session`.
    fn list(session: &SessionId) -> Op {
        Op::ProjectList(ProjectListParams {
            session_id: Some(session.clone()),
            workspace: None,
        })
    }

    /// Whether a project with `trust` is trusted as it is: what lets an
    /// accepted command run without asking.
    fn trusted(trust: Option<&TrustStanding>) -> bool {
        matches!(
            trust,
            Some(TrustStanding::Trusted | TrustStanding::ForTheRun)
        )
    }
}

impl Panel for ProjectPanel {
    fn name(&self) -> &'static str {
        "project"
    }

    fn seen(&mut self, seen: &Seen<'_>) -> Show {
        match seen {
            Seen::Event { event, running, .. } => {
                self.running = *running;
                match event {
                    Event::ProjectUpdated(updated) => {
                        self.merge(&updated.entries, &updated.removed);
                        self.show()
                    }
                    Event::SessionUpdated(updated) => {
                        self.trusted = Self::trusted(updated.session.trust.as_ref());
                        Show::Same
                    }
                    _ => Show::Same,
                }
            }
            // This client's prompt starts a turn: promote waits for its end.
            Seen::Prompt => {
                self.running = true;
                Show::Same
            }
            Seen::Left => {
                let clock = self.clock.clone();
                *self = Self::new(clock);
                Show::Close
            }
        }
    }

    fn result(&mut self, result: &OpResult) -> Show {
        match result {
            OpResult::ProjectList(list) => {
                self.entries.clone_from(&list.entries);
                self.problems.clone_from(&list.problems);
                self.listed = true;
                self.order();
                self.show()
            }
            OpResult::ProjectUpdate(update) => {
                self.merge(&update.entries, &[]);
                self.problems.clone_from(&update.problems);
                self.show()
            }
            _ => Show::Same,
        }
    }

    fn when_opened(&mut self, state: &SessionState, _stored: bool) -> Option<Op> {
        self.trusted = Self::trusted(state.trust.as_ref());
        self.running = state.running_turn.is_some();
        // Entries may wait from earlier sessions, a new one's included.
        Some(Self::list(&state.session_id))
    }

    fn summon(&mut self) -> Result<(), String> {
        self.summoned = true;
        Ok(())
    }

    fn summoned(&self, session: Option<&SessionId>) -> Vec<Op> {
        session.map(Self::list).into_iter().collect()
    }

    fn collapsed(&self, draw: &Draw<'_>) -> Vec<Line> {
        views::project_collapsed(&self.view(), draw.width, draw.look)
    }

    fn expanded(&self, focused: usize, draw: &Draw<'_>) -> Vec<PanelRow> {
        views::project_rows(&self.view(), Some(focused), draw.width, draw.look)
    }

    fn items(&self) -> usize {
        self.shown.len()
    }

    fn least_rows(&self, focused: usize, _draw: &Draw<'_>) -> usize {
        // Its first row, and the focused entry's own row: its detail may be
        // cut, and then it offers no `y` (the trait allows this). Or the
        // row that says there is none.
        if self.shown.is_empty() {
            1 + usize::from(self.listed)
        } else {
            1 + usize::from(focused < self.shown.len())
        }
    }

    fn item(&self, index: usize) -> Option<ItemKey> {
        let entry = self.shown.get(index)?;
        Some(ItemKey {
            id: entry.id.as_str().to_owned(),
            // The whole entry, and the question when it is asked: a change
            // of either starts the readiness again.
            content: format!("{entry:?}\n{}", self.asking.as_ref() == Some(&entry.id)),
        })
    }

    fn focus_taken(&mut self) {
        self.asking = None;
    }

    fn moved(&mut self, focused: usize) {
        // The question is the entry's it was asked on: the focus leaving it
        // keeps the entry.
        if self.asking.as_ref() != self.shown.get(focused).map(|entry| &entry.id) {
            self.asking = None;
        }
    }

    fn shown_whole(&mut self, focused: usize, whole: bool) -> bool {
        let now = self
            .shown
            .get(focused)
            .filter(|_| whole)
            .map(|entry| entry.id.clone());
        let new = now.is_some() && now != self.whole;
        self.whole = now;
        new
    }

    fn hint(&self, focused: usize, keymap: &KeyMap) -> Vec<String> {
        let key = |action: Action| keymap.keys_of(action).into_iter().next().map(name);
        let mut keys = Vec::new();
        let now = self.clock.now();
        if let Some(entry) = self.shown.get(focused) {
            if self.asking.as_ref() == Some(&entry.id) {
                if let Some(yes) = key(Action::FocusAccept) {
                    keys.push(format!("{yes} deletes"));
                }
                keys.push("any other key keeps it".to_owned());
                return keys;
            }
            if acceptable(entry, now) {
                match key(Action::FocusAccept) {
                    Some(yes) if self.whole.as_ref() == Some(&entry.id) => {
                        keys.push(format!("{yes} accepts"));
                    }
                    Some(_) => keys.push("make the window taller to accept".to_owned()),
                    None => {}
                }
            }
            if !shared(entry)
                && let Some(edit) = key(Action::FocusEdit)
            {
                keys.push(format!("{edit} edits"));
            }
            if entry.status == EntryStatus::Active
                && let Some(off) = key(Action::FocusDisable)
            {
                keys.push(format!("{off} turns off"));
            }
            // Refused by the core while a turn of the project runs.
            if promotable(entry, now)
                && let Some(promote) = key(Action::FocusPromote)
            {
                if self.running {
                    keys.push(format!("{promote} promotes after this turn"));
                } else {
                    keys.push(format!("{promote} promotes"));
                }
            }
            if !shared(entry)
                && let Some(delete) = key(Action::FocusDelete)
            {
                keys.push(format!("{delete} deletes"));
            }
        }
        if let Some(close) = key(Action::FocusClose) {
            keys.push(format!("{close} closes"));
        }
        if let Some(back) = key(Action::FocusReturn) {
            keys.push(format!("{back} back to the input"));
        }
        keys
    }

    fn act(&mut self, action: Action, focused: usize, session: Option<&SessionId>) -> Act {
        if action == Action::FocusClose {
            self.asking = None;
            self.summoned = false;
            return Act::Close;
        }
        let Some(entry) = self.shown.get(focused) else {
            return Act::Pass;
        };
        let Some(session) = session else {
            return Act::Pass;
        };
        let id = entry.id.clone();
        let now = self.clock.now();
        // A deletion asked of this entry: `y` deletes, any other key keeps
        // the entry. A question of another entry is forgotten, and the key
        // does what it does.
        match self.asking.take() {
            Some(asked) if asked == id && action == Action::FocusAccept => {
                return Self::change(session, &id, EntryAction::Delete, None);
            }
            Some(asked) if asked == id => return Act::Taken,
            _ => {}
        }
        match action {
            Action::FocusAccept if !acceptable(entry, now) => Act::Taken,
            // What is accepted is what was shown, whole.
            Action::FocusAccept if self.whole.as_ref() != Some(&id) => Act::Say(TALLER.to_owned()),
            Action::FocusAccept => Self::change(session, &id, EntryAction::Accept, None),
            // The repository's file is changed there, not here.
            Action::FocusEdit | Action::FocusDelete if shared(entry) => Act::Taken,
            Action::FocusEdit => Act::Edit {
                text: entry.text.clone(),
                hint: SAVES,
                edit: ItemEditFor {
                    action,
                    item: id.as_str().to_owned(),
                },
            },
            Action::FocusDisable if entry.status == EntryStatus::Active => {
                Self::change(session, &id, EntryAction::Disable, None)
            }
            Action::FocusDelete => {
                self.asking = Some(id);
                Act::Taken
            }
            Action::FocusPromote if promotable(entry, now) => {
                Self::change(session, &id, EntryAction::Promote, None)
            }
            Action::FocusDisable | Action::FocusPromote => Act::Taken,
            _ => Act::Pass,
        }
    }

    fn edited(&mut self, edit: &ItemEditFor, text: &str, session: Option<&SessionId>) -> Act {
        // An entry's text is one line, as the plan's items are.
        let text: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let text = text.trim();
        let Some(session) = session else {
            return Act::Taken;
        };
        let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.id.as_str() == edit.item)
        else {
            return Act::Say("The entry is no longer kept, so the edit was not saved.".to_owned());
        };
        if text.is_empty() || text == entry.text {
            return Act::Taken;
        }
        Self::change(session, &entry.id, EntryAction::Edit, Some(text.to_owned()))
    }
}
