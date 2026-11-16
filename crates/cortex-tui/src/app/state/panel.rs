//! The panels in the application (step 24; `docs/architecture/09-tui.md`,
//! "The panels as built"): what they see, the block they are drawn in, and what their
//! keys do. The panels themselves are `app::panels`.
//!
//! **A panel's keys** come here while it has the focus:
//!
//! - **Moving** (`Up`, `Down`, `Ctrl+P`), `Enter` and `Esc` act at once.
//! - **A key that changes something** (every one of
//!   [`PANEL_KEYS`](crate::keys::PANEL_KEYS) but `focus.close`) is taken
//!   only when the panel is ready: the dialog's rule, its grace starting
//!   again after the viewer, `fg`, the external editor, a dialog that hid
//!   the panel, a change of the focused row from outside, and a new key
//!   map. Keys read together, a paste and a key held down never act: a
//!   marked repeat, or the same key again within the repeat window of one
//!   the panel took, goes where its press went (nowhere).
//! - **The typing rule:** a key that types and is not taken (the panel has
//!   no use for it, or is not ready) goes to the editor, and the focus with
//!   it; no keystroke is lost, and none is taken by surprise.
//!
//! **Operations** a panel sends go with the session's id in `Pending`: a
//! result for a session left is dropped, and an error is said with the
//! core's sentence.

use std::time::Instant;

use cortex_protocol::SessionId;
use cortex_protocol::event::{Event, Verification};
use cortex_protocol::item::Item;
use cortex_protocol::methods::OpResult;
use cortex_term::{Event as TermEvent, Key, Line};

use super::super::panels::{Act, Draw, Panels, Seen, plan::PlanPanel, review::ReviewPanel};
use super::super::ready::REPEAT;
use super::{App, LastKey, Pending};
use crate::focus::Target;
use crate::input::Outcome;
use crate::keys::{Action, KeyMap, typed};
use crate::view::Look;

/// A window of this many rows or fewer has no room for a panel.
pub(crate) const LEAST_ROWS: usize = 8;

/// The transcript's live rows a panel never takes: what streams never
/// vanishes under panels.
pub(crate) const LIVE_KEPT: usize = 4;

/// The panels of this build, each registered once: the plan's and the
/// review's.
pub(super) fn registered() -> Panels {
    let mut panels = Panels::new();
    panels.register(Box::new(PlanPanel::default()));
    panels.register(Box::new(ReviewPanel::default()));
    panels
}

/// The turn summary's last part (`$0.42`), with the turn's verdict after
/// it (`$0.42 · checks passed`) when its checks ran: the summary's view
/// draws the cost's text as it is given.
pub(super) fn with_verdict(
    cost: Option<String>,
    verdict: Option<&Verification>,
    look: &Look,
) -> Option<String> {
    let verdict = match verdict? {
        Verification::Passed => "checks passed",
        Verification::Failed => "checks failed",
        Verification::Incomplete => "checks incomplete",
        // Skipped is every turn that changed nothing: not worth a word.
        _ => return cost,
    };
    Some(match cost {
        Some(cost) => format!("{cost} {} {verdict}", look.glyphs.separator),
        None => verdict.to_owned(),
    })
}

impl App {
    /// The panels, for tests that add one of their own.
    #[cfg(test)]
    pub(crate) fn panels_mut(&mut self) -> &mut Panels {
        &mut self.panels
    }

    /// The panels, for tests that read them.
    #[cfg(test)]
    pub(crate) fn panels(&self) -> &Panels {
        &self.panels
    }

    /// The panel drawn with the focus in the last frame.
    #[cfg(test)]
    pub(crate) fn panel_drawn(&self) -> Option<u64> {
        self.panel_drawn
    }

    /// The panel `id` takes the focus, as `Ctrl+P` gives it; `None` gives
    /// it to the input.
    #[cfg(test)]
    pub(crate) fn focus_panel(&mut self, id: Option<u64>) {
        match id {
            Some(id) => {
                self.input.focus_mut().focus(Target::Panel(id));
                self.panels.focused(id);
            }
            None => self.input.focus_mut().to_input(),
        }
    }

    /// The look, for tests that draw a panel as the application does.
    #[cfg(test)]
    pub(crate) fn look(&self) -> &Look {
        &self.look
    }

    /// Replaces the key map (a reload of `[keys]`, step 24's third pull
    /// request): the input looks the next key up in it, and an open
    /// dialog's and every panel's readiness start again, since what the
    /// person saw offered was the old map's.
    pub fn set_keymap(&mut self, keymap: KeyMap) {
        self.input.set_keymap(keymap);
        if let Some(dialog) = self.dialogs.front_mut() {
            dialog.hide();
        }
        self.panels.restart();
    }

    /// Every panel sees `seen`. A focused panel that closed gives the
    /// focus back to the input.
    pub(super) fn panels_see(&mut self, seen: &Seen<'_>) {
        let closed = self.panels.seen(seen);
        self.closed_panels(&closed);
    }

    fn closed_panels(&mut self, closed: &[u64]) {
        if let Target::Panel(id) = self.input.focus().focused()
            && closed.contains(&id)
        {
            self.input.focus_mut().to_input();
        }
    }

    /// An event of the session open here, after the application took it.
    pub(super) fn panels_event(&mut self, event: &Event) {
        // A tool result's box in the viewer, while its turn runs.
        let round = match event {
            Event::ItemCompleted(completed) => match &completed.item {
                Item::ToolResult(result) => self.mapper.round_of(&result.call_id),
                _ => None,
            },
            _ => None,
        };
        let running = self.running.is_some();
        self.panels_see(&Seen::Event {
            event,
            round,
            running,
        });
    }

    /// What the panels ask of the core when `session` is open here.
    pub(super) fn panels_ask(&mut self, session: &SessionId, stored: bool) {
        for (panel, op) in self.panels.when_opened(session, stored) {
            let ticket = self.link.send(op);
            self.pending.insert(
                ticket,
                Pending::Panel {
                    session: session.clone(),
                    panel,
                },
            );
        }
    }

    /// The result of what `panel` sent, for the session still open.
    pub(super) fn panel_result(&mut self, panel: u64, result: &OpResult) {
        if self.panels.result(panel, result) {
            self.closed_panels(&[panel]);
        }
    }

    /// The session was left (or another is opened): the panels forget it,
    /// and an item edit of one of them ends, the draft back.
    pub(super) fn left_session(&mut self) {
        if self.item_edit.take().is_some() {
            self.input.cancel_item_edit();
        }
        self.panels_see(&Seen::Left);
    }

    /// The keys the hint row shows for an item being edited, or for the
    /// panel drawn with the focus.
    pub(super) fn panel_hint(&self) -> Option<Vec<String>> {
        if let Some(keys) = self.input.item_edit() {
            return Some(keys);
        }
        match self.input.focus().focused() {
            Target::Panel(id) if self.panel_drawn == Some(id) => {
                Some(self.panels.hint(id, self.input.keymap()))
            }
            _ => None,
        }
    }

    /// The panels' block for a live region that leaves them `room` rows
    /// under `live` rows of the transcript, `menu` when a menu or the
    /// history search is open; the focus ring is given the live `boxes`
    /// and the panels that can be drawn focused. The block ends in a blank
    /// row, and begins with one when no live row is above it to set it
    /// apart from the scrollback.
    pub(super) fn panel_block(
        &mut self,
        room: usize,
        live: usize,
        menu: bool,
        width: usize,
        boxes: &[u64],
    ) -> Vec<Line> {
        let mut block = Vec::new();
        let mut reachable = Vec::new();
        // No room at all: a small window, or a menu open.
        if !menu && self.rows > LEAST_ROWS {
            let spacer = live == 0 && !self.transcript.ends_blank();
            let room = room.saturating_sub(live.min(LIVE_KEPT) + 1 + usize::from(spacer));
            // The focused panel is drawn whole, and so is the one whose
            // item is being edited in the input box, the item shown.
            let whole = match self.input.focus().focused() {
                Target::Panel(id) => Some(id),
                Target::Input | Target::ToolBox(_) => self.item_edit.as_ref().map(|(id, _)| *id),
            };
            let draw = Draw {
                width,
                look: &self.look,
                tick: self.tick,
            };
            let laid = self.panels.layout(room, whole, &draw, self.input.keymap());
            if !laid.lines.is_empty() {
                if spacer {
                    block.push(Line::new());
                }
                block.extend(laid.lines);
                block.push(Line::new());
            }
            reachable = laid.reachable;
            self.panel_drawn = laid.focused;
        }
        self.input.focus_mut().update(boxes, &reachable);
        if self.panel_drawn.map(Target::Panel) != Some(self.input.focus().focused()) {
            self.panel_drawn = None;
        }
        // A panel a command asked for that this window cannot draw focused
        // took the focus for nothing: what happened is said.
        if let Some(id) = self.summoned.take()
            && !reachable.contains(&id)
        {
            let name = self.panels.name(id).unwrap_or("panel");
            self.say(&format!(
                "There is no room for the {name} panel in this window: make it taller."
            ));
        }
        block
    }

    /// A frame was written at `at`: the panel it drew focused, on its
    /// first frame since its readiness started again, is held by the input
    /// no panel took just before, as a dialog is.
    pub(super) fn panel_drawn_at(&mut self, at: Instant) {
        if let Some(id) = self.panel_drawn
            && self.panels.drawn(id, at)
            && let Some(untaken) = self.panel_untaken
        {
            self.panels.typed(untaken);
        }
    }

    /// `/plan` and the like: the panel named `name` opens with the focus,
    /// or the panel says why it cannot.
    pub(super) fn summon_panel(&mut self, name: &str) {
        let Some(id) = self.panels.find(name) else {
            return;
        };
        match self.panels.summon(id) {
            Ok(()) => {
                self.input.focus_mut().focus(Target::Panel(id));
                self.panels.focused(id);
                self.summoned = Some(id);
            }
            Err(sentence) => self.say(&sentence),
        }
    }

    /// Whether `key`, read at `at` (`repeat` when the terminal marked it
    /// one), is a key held down after a panel took it: the same key again
    /// within the repeat window. It goes where its press went, nowhere,
    /// whatever has the focus now: a held `x` that closed the review types
    /// nothing into the prompt.
    pub(super) fn panel_holds(&mut self, key: Key, at: Instant, repeat: bool) -> bool {
        let Some((held, last)) = self.panel_held else {
            return false;
        };
        if held != key || !(repeat || at.saturating_duration_since(last) < REPEAT) {
            return false;
        }
        self.panel_held = Some((key, at));
        self.last_key = Some(LastKey { typing: false });
        true
    }

    /// A key while the panel `id` has the focus, read at `at`; `repeat`
    /// when the terminal marked it a key held down. A key held down after
    /// one the panel took never reaches here ([`panel_holds`]).
    ///
    /// [`panel_holds`]: Self::panel_holds
    pub(super) fn panel_key(&mut self, id: u64, key: Key, at: Instant, repeat: bool) -> bool {
        // Any key a dialog did not take holds one that comes next.
        self.untaken = Some(at);
        self.panel_held = None;
        if !self.panels.is_open(id) {
            self.input.focus_mut().to_input();
            self.last_key = Some(LastKey { typing: true });
            self.panel_untaken = Some(at);
            return self.edit_with(key, at);
        }
        // Moving, `Enter` and `Esc` are the panel's, at once; a global key
        // is the application's; a key that types goes to the editor, and
        // the focus with it. The input gives a panel's own keys back as
        // `Outcome::Panel`, for the panel to take or pass on.
        let moving = matches!(
            self.input.keymap().lookup(&self.input.contexts(), key),
            Some(
                Action::FocusUp
                    | Action::FocusDown
                    | Action::FocusNext
                    | Action::FocusReturn
                    | Action::FocusActivate
            )
        );
        let outcome = self.input.handle(&TermEvent::Key(key));
        let Outcome::Panel(_, action) = outcome else {
            if !moving && typed(key).is_some() {
                self.panel_untaken = Some(at);
                self.panels.typed(at);
            }
            self.last_key = Some(LastKey { typing: !moving });
            self.outcome(outcome, at);
            return true;
        };
        let ready = action == Action::FocusClose || self.panels.ready(id, at);
        if repeat || !ready {
            return self.panel_passed(key, at);
        }
        let session = self.session.as_ref().map(|state| state.session_id.clone());
        let act = self.panels.act(id, action, session.as_ref());
        if act == Act::Pass {
            return self.panel_passed(key, at);
        }
        // While a change of the panel's is on its way, what it shows may be
        // out of date, and a change computed from it may be wrong (a second
        // reorder from the order before the first): the key is taken, and
        // nothing is sent.
        let act = match act {
            Act::Send(_) if self.panels.waiting(id) => Act::Taken,
            act => act,
        };
        self.panel_held = Some((key, at));
        self.taken = Some(at);
        self.last_key = Some(LastKey { typing: false });
        self.input.disarm_quit();
        self.panel_act(id, act);
        true
    }

    /// A panel's key it did not take: one that types goes to the editor,
    /// and the focus with it, as input no panel took.
    fn panel_passed(&mut self, key: Key, at: Instant) -> bool {
        if typed(key).is_some() {
            self.panel_untaken = Some(at);
            self.panels.typed(at);
            self.last_key = Some(LastKey { typing: true });
        }
        let outcome = self.input.pass_to_editor(key);
        self.outcome(outcome, at);
        true
    }

    /// What a panel's key, `Enter` or item edit came to.
    fn panel_act(&mut self, id: u64, act: Act) {
        match act {
            Act::Pass | Act::Taken => {}
            Act::Send(op) => {
                let Some(session) = self.session.as_ref().map(|state| state.session_id.clone())
                else {
                    return;
                };
                let ticket = self.link.send(op);
                self.pending
                    .insert(ticket, Pending::Panel { session, panel: id });
                self.panels.sent(id);
            }
            Act::Edit { text, hint, edit } => {
                self.item_edit = Some((id, edit));
                self.input.begin_item_edit(&text, hint);
            }
            Act::Close => {
                if self.panels.close(id) {
                    self.closed_panels(&[id]);
                }
            }
            Act::View(box_) => self.open_viewer(box_),
            Act::Say(sentence) => self.say(&sentence),
        }
    }

    /// What the input gave that is about a panel, or the focus.
    pub(super) fn panel_outcome(&mut self, outcome: Outcome) {
        match outcome {
            // A panel took the focus: its readiness starts with its next
            // frame, on the item it names.
            Outcome::Focused(Target::Panel(id)) => self.panels.focused(id),
            Outcome::Move(Target::Panel(id), down) => self.panels.moved(id, down),
            Outcome::Activate(Target::Panel(id)) => {
                let act = self.panels.activate(id);
                self.panel_act(id, act);
            }
            Outcome::ItemEdited(text) => {
                if let Some((id, edit)) = self.item_edit.take() {
                    let session = self.session.as_ref().map(|state| state.session_id.clone());
                    let act = self.panels.edited(id, &edit, &text, session.as_ref());
                    self.panel_act(id, act);
                    self.refocus(id);
                }
            }
            Outcome::ItemEditCancelled => {
                if let Some((id, _)) = self.item_edit.take() {
                    self.refocus(id);
                }
            }
            _ => {}
        }
    }

    /// After an item edit, the focus goes back to its panel, if it is still
    /// open, on the item it was on.
    fn refocus(&mut self, id: u64) {
        if self.panels.is_open(id) {
            self.input.focus_mut().focus(Target::Panel(id));
            self.panels.refocused(id);
        }
    }
}
