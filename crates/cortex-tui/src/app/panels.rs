//! The panels (`docs/architecture/09-tui.md`, "Panels";
//! "The panels as built"): bounded blocks in the live region
//! that appear when they have something to show and go when they do not.
//!
//! **A panel kind is one type and one registration.** Each kind is a type
//! that implements [`Panel`]: it keeps its own state, sees what happens
//! ([`Seen`]) and says whether that opens or closes it ([`Show`]), draws
//! itself collapsed and focused, and says what each of its keys does
//! ([`Act`]). [`Panels::register`] adds it, once, when the application is
//! made. Everything else is the mechanism's, here, and no kind repeats it:
//!
//! - **The open panels, in the order they opened,** drawn as one block
//!   under the transcript's live rows and above the input box
//!   ([`Panels::layout`]). Only the focused panel is drawn whole; the
//!   others are collapsed.
//! - **The live region's limit.** The block gets the room the application
//!   gives it. When the panels need more, the focused one scrolls its list
//!   to keep its focused row in view, with a dim `… N more` row at a cut
//!   end, and the oldest collapsed panels fold into one dim row, `+2 panels
//!   · ctrl+p`. With no room, nothing is drawn.
//! - **The focus.** What [`Panels::layout`] says can be drawn focused is
//!   what the application gives the focus ring, so a panel that would not
//!   be drawn cannot hold the focus. Inside the focused panel, the focused
//!   item is kept by its id when the list changes under it.
//! - **The readiness.** Every key of a panel's that changes something is
//!   taken only when the panel is ready (the dialog's rule,
//!   [`super::ready::Readiness`]): 300 ms after its focused row was first
//!   drawn and a second after input it did not take. Its grace starts
//!   again when the focused row's content changes from outside, and when
//!   the application says so ([`Panels::restart`]).
//!
//! **What the application does with a panel's keys** is in `state`: the
//! readiness and the held-key rule, the typing rule (a key that types and
//! is not taken goes to the editor, and the focus with it), the operations
//! sent with the session's id, and their results routed back
//! ([`Panel::result`]).

pub(crate) mod plan;
pub(crate) mod project;
pub(crate) mod review;
pub(crate) mod usage;

use std::time::Instant;

use cortex_protocol::event::Event;
use cortex_protocol::methods::{Op, OpResult};
use cortex_protocol::session::SessionState;
use cortex_protocol::{ErrorData, SessionId};
use cortex_term::Line;

use super::ready::Readiness;
use crate::keys::{Action, KeyMap, name};
use crate::view::Look;
use crate::view::panel::{self as views, PanelRow};

/// How a panel is drawn this frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Draw<'a> {
    /// The columns a row may use: one less than the window's.
    pub(crate) width: usize,
    /// The theme, the glyphs and the `tui` settings.
    pub(crate) look: &'a Look,
    /// The spinner's frame, for what runs.
    pub(crate) tick: usize,
}

/// Something that happened, which every panel sees.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Seen<'a> {
    /// An event of the session open here, after the application took it.
    Event {
        /// The event.
        event: &'a Event,
        /// For a tool result: the viewer's box that holds its call, counted
        /// from 1, as `/view n` counts.
        round: Option<u64>,
        /// Whether a turn runs, now that the event was taken.
        running: bool,
    },
    /// This client sent a prompt that starts a turn.
    Prompt,
    /// The session was left: another is open now, or none. What a panel
    /// showed of it is gone.
    Left,
}

/// What a panel wants after it saw something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Show {
    /// As it is: open or closed.
    Same,
    /// Open (after the others, if it was closed), without the focus: a
    /// panel that opens by itself never takes it.
    Open,
    /// Closed.
    Close,
}

/// The focused item of a panel, as the mechanism follows it: its id, kept
/// across changes of the list, and its content, whose change from outside
/// starts the panel's readiness again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemKey {
    /// What names the item across changes (a plan item's id, an entry's).
    pub(crate) id: String,
    /// What a person reads of it, or anything that changes when that does.
    pub(crate) content: String,
}

/// What an item edit is for, handed back with its text
/// ([`Panel::edited`]): the action that began it, and the item it is of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemEditFor {
    /// The key's action: `focus.edit` (reword), `focus.add` (add after).
    pub(crate) action: Action,
    /// The item's id ([`ItemKey::id`]).
    pub(crate) item: String,
}

/// What a panel's key did, for the application to finish.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Act {
    /// The panel has no use for the key here: one that types goes on to
    /// the editor, and the focus with it.
    Pass,
    /// Taken, and nothing for the application to do (the panel changed
    /// itself, or the key does nothing on this item).
    Taken,
    /// Taken: send this operation, whose result comes back to
    /// [`Panel::result`] while its session is the one open, and whose
    /// error is told with the core's sentence.
    Send(Op),
    /// Taken: send this operation, and say this, as a notice (what an item
    /// edit's text lost to fit).
    SendAndSay(Op, String),
    /// Taken: edit `text` in the prompt's own editor (`Input::begin_item_edit`),
    /// the hint row saying `enter {hint}`; the text is handed to
    /// [`Panel::edited`] with `edit`.
    Edit {
        /// The text to start from: the item's, or nothing for a new one.
        text: String,
        /// What `Enter` does: `saves the item`.
        hint: &'static str,
        /// What the edit is for.
        edit: ItemEditFor,
    },
    /// Taken: the panel closes, and the focus goes back to the input.
    Close,
    /// Taken: the viewer opens, at box `n` opened when there is one (as
    /// `/view n` does), or at the latest.
    View(Option<usize>),
    /// Taken: the viewer opens on this stored artifact of the session (a
    /// check's whole output), read with `session/artifact`; the label is
    /// its title.
    Artifact {
        /// The artifact.
        id: cortex_protocol::ArtifactId,
        /// What it holds, in a few words.
        label: String,
    },
    /// Taken: say this, as a notice.
    Say(String),
}

/// One kind of panel. See the [module documentation](self).
///
/// For the author of a new kind: implement this, register it in
/// `App::new` with [`Panels::register`], and add a test that it opens and
/// closes on its conditions. The layout, the focus, the readiness, the
/// typing rule, the item edit and the routing of results are the
/// mechanism's: a kind only says what it shows and what its keys do.
pub(crate) trait Panel: std::fmt::Debug + Send {
    /// Its name, by which a command finds it (`plan`, `review`).
    fn name(&self) -> &'static str;

    /// It saw `seen`: what that does to its state, and whether it opens or
    /// closes. Every panel sees everything, open or not.
    fn seen(&mut self, seen: &Seen<'_>) -> Show;

    /// The result of an operation it sent ([`Act::Send`]) or asked for when
    /// the session opened ([`Panel::when_opened`]), while that session is
    /// still the one open.
    fn result(&mut self, result: &OpResult) -> Show {
        let _ = result;
        Show::Same
    }

    /// What it sent or asked for was refused, with `error`, while that
    /// session is still the one open: the application says the error too.
    fn failed(&mut self, error: &ErrorData) -> Show {
        let _ = error;
        Show::Same
    }

    /// A session is open here, as `state` says it: what to ask the core
    /// for, `stored` when it was opened from the store (resumed, switched
    /// to, forked), so that what it already holds can be shown
    /// (`plan/get`). The result comes to [`Panel::result`].
    fn when_opened(&mut self, state: &SessionState, stored: bool) -> Option<Op> {
        let _ = (state, stored);
        None
    }

    /// A command asked for it (`/plan`): `Ok` opens it with the focus, and
    /// the sentence in `Err` is said instead when it has nothing to show.
    fn summon(&mut self) -> Result<(), String> {
        Err("This panel opens by itself, when it has something to show.".to_owned())
    }

    /// What to ask the core for when a command opened it, in `session`,
    /// the one open: the results come to [`Panel::result`].
    fn summoned(&self, session: Option<&SessionId>) -> Vec<Op> {
        let _ = session;
        Vec::new()
    }

    /// It took the focus: what a key of its asked and was not answered
    /// (a question in a row) is forgotten.
    fn focus_taken(&mut self) {}

    /// The focus moved to `focused` inside it (`Up`, `Down`).
    fn moved(&mut self, focused: usize) {
        let _ = focused;
    }

    /// It was drawn focused on `focused`, whose rows were all drawn when
    /// `whole`: a key that acts on what the person was shown asks this.
    /// `true` when that is something new to act on (an item's detail drawn
    /// whole for the first time): its readiness starts again with this
    /// frame.
    fn shown_whole(&mut self, focused: usize, whole: bool) -> bool {
        let _ = (focused, whole);
        false
    }

    /// Its collapsed rows, one or two: how it is drawn without the focus.
    fn collapsed(&self, draw: &Draw<'_>) -> Vec<Line>;

    /// Its rows drawn whole, with `focused` drawn as the selection: first
    /// the rows of no item that are always shown (its first row), then the
    /// rest, each tagged with its item. The mechanism keeps the first rows
    /// and scrolls the rest.
    fn expanded(&self, focused: usize, draw: &Draw<'_>) -> Vec<PanelRow>;

    /// How many items can have the focus.
    fn items(&self) -> usize;

    /// The item at `index`, by its id and its content.
    fn item(&self, index: usize) -> Option<ItemKey>;

    /// The item the focus starts on when the panel takes it.
    fn first_focus(&self) -> usize {
        0
    }

    /// The fewest rows it is drawn in with the focus on `focused`: the
    /// first rows of [`expanded`](Self::expanded), those of no item, and
    /// the rows of `focused`. Asked of every open panel each frame, to know
    /// which can be focused in the room there is: a kind whose rows are
    /// costly to draw says it without drawing them, and must say what
    /// `expanded` would draw. A kind whose focused item may be drawn cut
    /// (the project panel's entry, its detail after it) may count its
    /// item's first row alone: then what it does on that item is for what
    /// was drawn whole ([`Panel::shown_whole`]).
    fn least_rows(&self, focused: usize, draw: &Draw<'_>) -> usize {
        let rows = self.expanded(focused, draw);
        let head = rows.iter().take_while(|row| row.item.is_none()).count();
        head + rows.iter().filter(|row| row.item == Some(focused)).count()
    }

    /// The keys the hint row shows while it has the focus, on `focused`,
    /// each with its key as `keymap` names it (`space skips`).
    fn hint(&self, focused: usize, keymap: &KeyMap) -> Vec<String>;

    /// One of its keys ([`crate::keys::PANEL_KEYS`]) on `focused`, when the
    /// panel is ready (the mechanism holds back the ones that change
    /// something until then), in `session`, the one open, if any.
    fn act(&mut self, action: Action, focused: usize, session: Option<&SessionId>) -> Act;

    /// `Enter` on `focused`: it changes nothing, and is taken at once.
    fn activate(&mut self, focused: usize) -> Act {
        let _ = focused;
        Act::Taken
    }

    /// The item edit it began ([`Act::Edit`]) ended with `text`.
    fn edited(&mut self, edit: &ItemEditFor, text: &str, session: Option<&SessionId>) -> Act {
        let _ = (edit, text, session);
        Act::Taken
    }
}

/// One registered panel and what the mechanism keeps of it.
#[derive(Debug)]
struct Slot {
    panel: Box<dyn Panel>,
    /// The focused item's place.
    focus: usize,
    /// The focused item's id, to keep the focus on it when the list
    /// changes.
    focus_id: Option<String>,
    /// The focused item as last drawn: a change of it from outside starts
    /// the readiness again.
    seen: Option<ItemKey>,
    /// The first row of its list in view, while it is cut.
    scroll: usize,
    ready: Readiness,
}

/// What [`Panels::layout`] drew.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Laid {
    /// The block's rows, without the blank row after it.
    pub(crate) lines: Vec<Line>,
    /// The panels drawn, in order.
    pub(crate) drawn: Vec<u64>,
    /// The panels that can have the focus: those that would be drawn
    /// focused in this room, in the order drawn. What the focus ring is
    /// given.
    pub(crate) reachable: Vec<u64>,
    /// The focused panel, when it was drawn with its focused row.
    pub(crate) focused: Option<u64>,
}

/// The panels. See the [module documentation](self).
#[derive(Debug, Default)]
pub(crate) struct Panels {
    slots: Vec<Slot>,
    /// The open ones, by id, oldest first.
    open: Vec<u64>,
}

impl Panels {
    /// No panel.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds a kind of panel, closed. Its id, which the focus ring knows it
    /// by (`Target::Panel`).
    pub(crate) fn register(&mut self, panel: Box<dyn Panel>) -> u64 {
        self.slots.push(Slot {
            panel,
            focus: 0,
            focus_id: None,
            seen: None,
            scroll: 0,
            ready: Readiness::default(),
        });
        (self.slots.len() - 1) as u64
    }

    fn slot(&self, id: u64) -> Option<&Slot> {
        self.slots.get(usize::try_from(id).ok()?)
    }

    fn slot_mut(&mut self, id: u64) -> Option<&mut Slot> {
        self.slots.get_mut(usize::try_from(id).ok()?)
    }

    /// The panel named `name`.
    pub(crate) fn find(&self, name: &str) -> Option<u64> {
        self.slots
            .iter()
            .position(|slot| slot.panel.name() == name)
            .map(|at| at as u64)
    }

    /// Whether `id` is open.
    pub(crate) fn is_open(&self, id: u64) -> bool {
        self.open.contains(&id)
    }

    /// `id`'s name, as a command finds it.
    pub(crate) fn name(&self, id: u64) -> Option<&'static str> {
        self.slot(id).map(|slot| slot.panel.name())
    }

    /// Every panel sees `seen`; each opens or closes as it says. The panels
    /// that closed.
    pub(crate) fn seen(&mut self, seen: &Seen<'_>) -> Vec<u64> {
        let mut closed = Vec::new();
        for at in 0..self.slots.len() {
            let show = self.slots[at].panel.seen(seen);
            if self.apply(at as u64, show) {
                closed.push(at as u64);
            }
            self.observe(at as u64);
        }
        closed
    }

    /// The result of what `id` sent or asked for. Whether it closed.
    pub(crate) fn result(&mut self, id: u64, result: &OpResult) -> bool {
        let Some(slot) = self.slot_mut(id) else {
            return false;
        };
        let show = slot.panel.result(result);
        let closed = self.apply(id, show);
        self.observe(id);
        closed
    }

    /// What `id` sent or asked for was refused. Whether it closed.
    pub(crate) fn failed(&mut self, id: u64, error: &ErrorData) -> bool {
        let Some(slot) = self.slot_mut(id) else {
            return false;
        };
        let show = slot.panel.failed(error);
        let closed = self.apply(id, show);
        self.observe(id);
        closed
    }

    /// The fewest rows `id` is drawn in when it takes the focus, on the
    /// item its focus starts on.
    pub(crate) fn rows_to_focus(&self, id: u64, draw: &Draw<'_>) -> usize {
        self.least(id, false, draw)
    }

    /// After a change, an open `id`'s focus follows its item, and the
    /// readiness starts again if the focused item is not the one last seen
    /// there: a key read before the next frame acts on nothing it was not
    /// shown.
    fn observe(&mut self, id: u64) {
        if self.is_open(id) {
            self.follow(id);
        }
    }

    /// Opens or closes `id` as `show` says. Whether it closed.
    fn apply(&mut self, id: u64, show: Show) -> bool {
        match show {
            Show::Same => false,
            Show::Open => {
                if !self.open.contains(&id) {
                    self.open.push(id);
                }
                self.follow(id);
                false
            }
            Show::Close => self.close(id),
        }
    }

    /// Closes `id`. Whether it was open.
    pub(crate) fn close(&mut self, id: u64) -> bool {
        let was = self.open.contains(&id);
        self.open.retain(|open| *open != id);
        was
    }

    /// What every panel asks of the core when a session is open here, as
    /// `state` says it.
    pub(crate) fn when_opened(&mut self, state: &SessionState, stored: bool) -> Vec<(u64, Op)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(at, slot)| Some((at as u64, slot.panel.when_opened(state, stored)?)))
            .collect()
    }

    /// A command asked for `id`, in `session`: opened, with what it asks of
    /// the core, or the sentence that says why not.
    pub(crate) fn summon(
        &mut self,
        id: u64,
        session: Option<&SessionId>,
    ) -> Result<Vec<Op>, String> {
        let slot = self.slot_mut(id).ok_or_else(String::new)?;
        slot.panel.summon()?;
        let asks = slot.panel.summoned(session);
        self.apply(id, Show::Open);
        Ok(asks)
    }

    /// `id` took the focus (`Ctrl+P`, a command): its focus starts on the
    /// item it names, and its readiness starts again with its next frame.
    pub(crate) fn focused(&mut self, id: u64) {
        let Some(slot) = self.slot_mut(id) else {
            return;
        };
        slot.panel.focus_taken();
        slot.focus = slot.panel.first_focus();
        slot.focus_id = slot.panel.item(slot.focus).map(|key| key.id);
        slot.seen = slot.panel.item(slot.focus);
        slot.scroll = 0;
        slot.ready.hide();
    }

    /// `id` takes the focus back after an item edit: its focused item
    /// stays, and its readiness starts again.
    pub(crate) fn refocused(&mut self, id: u64) {
        if let Some(slot) = self.slot_mut(id) {
            slot.ready.hide();
        }
    }

    /// `Up` (`down` false) or `Down` in `id`: the focus moves an item,
    /// stopping at either end. Moving is not a change from outside: the
    /// readiness goes on.
    pub(crate) fn moved(&mut self, id: u64, down: bool) {
        let Some(slot) = self.slot_mut(id) else {
            return;
        };
        let items = slot.panel.items();
        if items == 0 {
            return;
        }
        slot.focus = if down {
            (slot.focus + 1).min(items - 1)
        } else {
            slot.focus.saturating_sub(1)
        };
        slot.focus_id = slot.panel.item(slot.focus).map(|key| key.id);
        slot.panel.moved(slot.focus);
        slot.seen = slot.panel.item(slot.focus);
    }

    /// Keeps `id`'s focus on the item it was on, by its id, when the list
    /// changed; on the nearest place when that item is gone. When what has
    /// the focus is not what was seen there (its item went, or changed),
    /// the readiness starts again, at once: a key read before the next
    /// frame acts on nothing the person was not shown.
    fn follow(&mut self, id: u64) {
        let Some(slot) = self.slot_mut(id) else {
            return;
        };
        let items = slot.panel.items();
        if let Some(wanted) = &slot.focus_id
            && slot
                .panel
                .item(slot.focus)
                .is_none_or(|key| &key.id != wanted)
            && let Some(found) =
                (0..items).find(|at| slot.panel.item(*at).is_some_and(|key| &key.id == wanted))
        {
            slot.focus = found;
        }
        slot.focus = slot.focus.min(items.saturating_sub(1));
        let now = slot.panel.item(slot.focus);
        slot.focus_id = now.as_ref().map(|key| key.id.clone());
        // What has the focus changed under it, from outside: whatever the
        // person was about to press was meant for what was there.
        if now != slot.seen {
            slot.ready.hide();
            slot.seen = now;
        }
    }

    /// Whether `id` takes a key that changes something at `at`.
    pub(crate) fn ready(&self, id: u64, at: Instant) -> bool {
        self.slot(id).is_some_and(|slot| slot.ready.ready(at))
    }

    /// `id` was drawn focused in a frame written at `at`. Says whether it
    /// was its first since its readiness started again: the caller then
    /// gives it the input it did not take ([`typed`](Self::typed)).
    pub(crate) fn drawn(&mut self, id: u64, at: Instant) -> bool {
        self.slot_mut(id).is_some_and(|slot| slot.ready.drawn(at))
    }

    /// Input no panel took was read at `at` (typing, keys read together, a
    /// paste): a panel is ready only a quiet second after it.
    pub(crate) fn typed(&mut self, at: Instant) {
        for slot in &mut self.slots {
            slot.ready.typed(at);
        }
    }

    /// Every panel's readiness starts again with its next frame: after the
    /// viewer, after `fg`, after the external editor, after a dialog that
    /// hid them, after the key map was replaced.
    pub(crate) fn restart(&mut self) {
        for slot in &mut self.slots {
            slot.ready.hide();
        }
    }

    /// One of `id`'s keys, on its focused item. A key that changed what
    /// the focused item shows (a question asked in its row) starts the
    /// readiness again, at once: the next key acts on what is shown next.
    pub(crate) fn act(&mut self, id: u64, action: Action, session: Option<&SessionId>) -> Act {
        let Some(slot) = self.slot_mut(id) else {
            return Act::Pass;
        };
        let focus = slot.focus;
        let act = slot.panel.act(action, focus, session);
        self.observe(id);
        act
    }

    /// `Enter` on `id`'s focused item.
    pub(crate) fn activate(&mut self, id: u64) -> Act {
        let Some(slot) = self.slot_mut(id) else {
            return Act::Taken;
        };
        let focus = slot.focus;
        slot.panel.activate(focus)
    }

    /// An item edit of `id`'s ended with `text`.
    pub(crate) fn edited(
        &mut self,
        id: u64,
        edit: &ItemEditFor,
        text: &str,
        session: Option<&SessionId>,
    ) -> Act {
        match self.slot_mut(id) {
            Some(slot) => slot.panel.edited(edit, text, session),
            None => Act::Taken,
        }
    }

    /// The keys the hint row shows for `id`, focused.
    pub(crate) fn hint(&self, id: u64, keymap: &KeyMap) -> Vec<String> {
        self.slot(id)
            .map(|slot| slot.panel.hint(slot.focus, keymap))
            .unwrap_or_default()
    }

    /// The open panels laid out in `room` rows, `focused` drawn whole and
    /// the others collapsed, with the live region's limit (see the [module
    /// documentation](self)); `keymap` names the key on the folded row.
    pub(crate) fn layout(
        &mut self,
        room: usize,
        focused: Option<u64>,
        draw: &Draw<'_>,
        keymap: &KeyMap,
    ) -> Laid {
        let mut laid = Laid::default();
        if room == 0 || self.open.is_empty() {
            return laid;
        }
        for id in self.open.clone() {
            self.follow(id);
        }
        // What each open panel needs at the least to be drawn focused.
        let least: Vec<(u64, usize)> = self
            .open
            .iter()
            .map(|id| (*id, self.least(*id, focused == Some(*id), draw)))
            .collect();
        laid.reachable = least
            .iter()
            .filter(|(_, rows)| *rows <= room)
            .map(|(id, _)| *id)
            .collect();
        let focused = focused.filter(|id| laid.reachable.contains(id));
        let others: Vec<(u64, Vec<Line>)> = self
            .open
            .iter()
            .filter(|id| Some(**id) != focused)
            .filter_map(|id| Some((*id, self.slot(*id)?.panel.collapsed(draw))))
            .collect();
        let need = focused.map_or(0, |id| {
            least
                .iter()
                .find(|(at, _)| *at == id)
                .map_or(0, |(_, rows)| *rows)
        });
        // The oldest collapsed panels fold first, until what is left fits.
        let fitted = (0..=others.len()).find_map(|folded| {
            let rows = usize::from(folded > 0)
                + others[folded..]
                    .iter()
                    .map(|(_, lines)| lines.len())
                    .sum::<usize>();
            (rows + need <= room).then_some((folded, rows))
        });
        // With no room for even the folded row beside the focused panel,
        // the focused panel alone.
        let (folded, others_rows, shown_others) = match fitted {
            Some((folded, rows)) => (folded, rows, true),
            None => (others.len(), 0, false),
        };
        let mut focused_lines = Vec::new();
        if let Some(id) = focused {
            focused_lines = self.scrolled(id, room - others_rows, draw);
            laid.focused = Some(id);
        }
        if shown_others && folded > 0 {
            // The key is named only when it reaches a panel folded there.
            let reached = others[..folded]
                .iter()
                .any(|(id, _)| laid.reachable.contains(id));
            let key = keymap
                .keys_of(Action::FocusNext)
                .into_iter()
                .next()
                .filter(|_| reached)
                .map(name);
            laid.lines
                .push(views::folded(folded, key.as_deref(), draw.width, draw.look));
        }
        for id in self.open.clone() {
            if Some(id) == focused {
                laid.lines.append(&mut focused_lines);
                laid.drawn.push(id);
            } else if shown_others
                && let Some(place) = others.iter().position(|(other, _)| *other == id)
                && place >= folded
            {
                laid.lines.extend(others[place].1.iter().cloned());
                laid.drawn.push(id);
            }
        }
        // `reachable` is in the order the panels opened, which is the order
        // they are drawn in: the ring goes through them so, the folded ones
        // (drawn when focused) first, where their row is.
        laid
    }

    /// The fewest rows `id` is drawn focused in: its first rows and the
    /// rows of the item the focus is on, or, for a panel that does not have
    /// it, of the item it would start on.
    fn least(&self, id: u64, focused: bool, draw: &Draw<'_>) -> usize {
        let Some(slot) = self.slot(id) else {
            return usize::MAX;
        };
        let focus = if focused {
            slot.focus
        } else {
            slot.panel.first_focus()
        };
        slot.panel.least_rows(focus, draw)
    }

    /// `id` drawn focused in at most `room` rows: its first rows, then as
    /// much of the rest as fits around its focused item, a dim `… N more`
    /// row in the place of what is cut at either end. (A change of the
    /// focused item from outside started its readiness again when it was
    /// seen: [`follow`](Self::follow).)
    fn scrolled(&mut self, id: u64, room: usize, draw: &Draw<'_>) -> Vec<Line> {
        let Some(slot) = self.slot_mut(id) else {
            return Vec::new();
        };
        let rows = slot.panel.expanded(slot.focus, draw);
        let head = rows.iter().take_while(|row| row.item.is_none()).count();
        let (top, body) = rows.split_at(head);
        let mut lines: Vec<Line> = top.iter().map(|row| row.line.clone()).collect();
        let n = room.saturating_sub(head);
        let focus = slot.focus;
        let first = body.iter().position(|row| row.item == Some(focus));
        let last = body.iter().rposition(|row| row.item == Some(focus));
        let wanted = match (first, last) {
            (Some(first), Some(last)) => first..last + 1,
            _ => 0..0,
        };
        let (start, end, marked) = window(body.len(), n, wanted.clone(), slot.scroll);
        slot.scroll = start;
        let whole = !wanted.is_empty() && start <= wanted.start && wanted.end <= end;
        if slot.panel.shown_whole(focus, whole) {
            // Something new to act on, shown with this frame: its grace
            // starts from it.
            slot.ready.hide();
        }
        if marked && start > 0 {
            lines.push(views::more(start, draw.width, draw.look));
        }
        lines.extend(body[start..end].iter().map(|row| row.line.clone()));
        if marked && end < body.len() {
            lines.push(views::more(body.len() - end, draw.width, draw.look));
        }
        lines
    }
}

/// Which of `len` rows to show in `room` rows so that `wanted` is in view:
/// `(start, end, marked)`. When `marked`, a row of `room` is kept at each
/// cut end for its `… N more` row; the start is the one nearest `scroll`,
/// the last start, that shows `wanted` so. When no start does, `wanted`'s
/// first rows alone, unmarked: what is wanted comes before saying what is
/// not shown.
fn window(
    len: usize,
    room: usize,
    wanted: std::ops::Range<usize>,
    scroll: usize,
) -> (usize, usize, bool) {
    if len <= room {
        return (0, len, false);
    }
    let shown = |start: usize| {
        let left = room.saturating_sub(usize::from(start > 0));
        if start + left >= len {
            (start, len)
        } else {
            (start, start + left.saturating_sub(1))
        }
    };
    let fits = |(start, end): (usize, usize)| {
        wanted.is_empty() || (start <= wanted.start && wanted.end <= end)
    };
    (0..len)
        .map(shown)
        .filter(|&(start, end)| end > start)
        .filter(|&span| fits(span))
        .min_by_key(|&(start, _)| start.abs_diff(scroll))
        .map_or(
            (wanted.start, (wanted.start + room).min(len), false),
            |(start, end)| (start, end, true),
        )
}

/// What tests set and read of the panels directly.
#[cfg(test)]
impl Panels {
    /// Exactly `ids` open, in that order, whatever each would say.
    pub(crate) fn set_open(&mut self, ids: &[u64]) {
        self.open = ids.to_vec();
        for id in ids {
            self.follow(*id);
        }
    }

    /// The focused item's place in `id`.
    pub(crate) fn focus_of(&self, id: u64) -> usize {
        self.slot(id).map_or(0, |slot| slot.focus)
    }

    /// The rows of `id`'s focused item, drawn as its focused panel draws
    /// them.
    pub(crate) fn focused_lines(&self, id: u64, draw: &Draw<'_>) -> Vec<Line> {
        let Some(slot) = self.slot(id) else {
            return Vec::new();
        };
        slot.panel
            .expanded(slot.focus, draw)
            .into_iter()
            .filter(|row| row.item == Some(slot.focus))
            .map(|row| row.line)
            .collect()
    }
}

#[cfg(test)]
mod tests;
