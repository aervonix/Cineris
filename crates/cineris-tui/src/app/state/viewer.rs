//! The viewer (`docs/architecture/09-tui.md`, "The viewer"): the session on
//! the alternate screen, its tool boxes closed and the latest selected.
//! `Up` and `Down` move between boxes, `Enter` opens or closes the selected
//! one with all its detail, `PageUp` and `PageDown` scroll, `/` searches,
//! and `Esc` returns to the inline screen as it was.
//!
//! The transcript it shows is the kept one (each finished block, drawn
//! again at the window's width, each round as a box that can be selected
//! and opened) and what is still live, drawn as it is. Since step 25 it is
//! drawn through the same scroll window as the full-screen presenter
//! (`app::scroll`): only the blocks near what is shown are drawn, and a
//! resize draws them again at the new width.

use std::collections::BTreeSet;
use std::time::Instant;

use cineris_base::{UtcTime, Zone};
use cineris_term::{Button, Key, KeyCode, Line, Mods, Mouse, MouseKind, Wheel};

use super::App;
use crate::app::Ticket;
use crate::app::scroll::{Anchor, Place, Scroll, Source};
use crate::keys::{Action, Context, Gesture};
use crate::transcript::kept::Summary;
use crate::view::Placement;
use crate::view::viewer::{self, SummaryRead, ViewerKeys, ViewerTitle};

/// The rows the frame takes beside the transcript: the title and its rule,
/// the rule and the keys.
const FRAME_ROWS: usize = 4;

/// How many rows a notch of the wheel scrolls (since step 25).
pub(crate) const WHEEL_ROWS: usize = 3;

/// The viewer while it is open.
#[derive(Debug, Clone, Default)]
pub(super) struct Viewer {
    /// The window over the transcript.
    scroll: Scroll,
    /// The selected box, by its place among the rounds.
    selected: Option<usize>,
    /// The boxes opened.
    open: BTreeSet<usize>,
    /// Whether the next frame brings the selected box into view.
    reveal: bool,
    /// The search: what is typed after `/`, while it is typed.
    typing: Option<String>,
    /// The last search, which `n` finds again, and whether it found
    /// nothing.
    search: Option<(String, bool)>,
    /// A search is due at the next frame, from the window's top.
    find: bool,
    /// It shows this request whole (`Ctrl+O` in its dialog), not the
    /// transcript, while its dialog is the one open.
    whole: Option<crate::app::dialog::DialogId>,
    /// It shows this text (a check's whole output, step 24), not the
    /// transcript: its label, then its lines, as a command's output is
    /// shown (sanitised, a carriage return taking the line back, tabs
    /// expanded).
    text: Option<(String, Vec<String>)>,
    /// The text's rows at the width they were last made for: made again
    /// only when the width changes, not at every frame or scroll.
    text_rows: Option<(usize, Vec<Line>)>,
    /// The width of the last frame, for the keys that move the window.
    width: usize,
    /// Where each row of the transcript the last frame drew came from, and
    /// the screen row of the first: what a click finds.
    shown: Vec<Place>,
    body_top: usize,
    /// Where the left button went down, for a click: a release on the same
    /// cell.
    pressed: Option<(u16, u16)>,
    /// It shows text to select and copy by hand: the mouse is not reported,
    /// so a plain drag is the terminal's own selection (since step 25).
    to_copy: bool,
    /// The transcript's view a summary was opened from: `Esc` goes back to
    /// it, not out of the viewer (since step 25).
    behind: Option<Box<Viewer>>,
    /// A compaction's summary item to read and show (since step 25).
    wants: Option<String>,
    /// The summary asked for, by the stop it was asked at: its read's
    /// ticket, or that it failed. An answer is shown only on this view and
    /// at that stop (since the review of step 25's second pull request).
    asked: Option<(usize, Asked)>,
    /// The request shown whole: the width its rows were counted at, and
    /// which of them a frame has shown (since step 25).
    whole_seen: (usize, Vec<bool>),
}

/// Where a compaction's summary asked for is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    /// Its read went with this ticket.
    Reading(Ticket),
    /// The read failed, or found no summary.
    Failed,
}

impl Viewer {
    /// Forgets the rows made: the look changed. The transcript's view
    /// behind a summary forgets its own too (since the review of step 25's
    /// second pull request).
    pub(super) fn forget_rows(&mut self) {
        self.scroll.rows.clear();
        if let Some(behind) = self.behind.as_mut() {
            behind.forget_rows();
        }
    }

    /// Forgets one kept block's rows: it is to be drawn again, here and in
    /// the view behind a summary.
    pub(super) fn forget_block(&mut self, block: usize) {
        self.scroll.rows.forget(block);
        if let Some(behind) = self.behind.as_mut() {
            behind.forget_block(block);
        }
    }

    /// Whether a search is being typed.
    pub(super) fn searching(&self) -> bool {
        self.typing.is_some()
    }

    /// The kept transcript was replaced, read again after a rewind or an
    /// undo (since step 25): its rows are made again, no box is open, and
    /// the selection is kept among the `stops` there are now.
    pub(super) fn kept_replaced(&mut self, stops: usize) {
        self.scroll.rows.clear();
        self.open.clear();
        self.selected = self
            .selected
            .map(|selected| selected.min(stops.saturating_sub(1)))
            .filter(|_| stops > 0);
        self.reveal = true;
        if let Some(behind) = self.behind.as_mut() {
            behind.kept_replaced(stops);
        }
    }

    /// The rows of the request shown whole that a frame showed: once every
    /// one of them has been, it was seen whole (since step 25).
    fn saw(&mut self, places: &[Place], rows: usize, width: usize) -> bool {
        if self.whole_seen.0 != width || self.whole_seen.1.len() != rows {
            self.whole_seen = (width, vec![false; rows]);
        }
        for place in places {
            if let Place::Live(row) = place
                && let Some(seen) = self.whole_seen.1.get_mut(*row)
            {
                *seen = true;
            }
        }
        self.whole_seen.1.iter().all(|seen| *seen)
    }
}

/// The rows of a block, by its place among the kept ones, as the viewer
/// draws them: a round as a box, selected or open as the viewer has it.
fn block_rows(app: &App, state: &Viewer, block: usize, width: usize) -> Vec<Line> {
    let Some(kept) = app.kept.get(block) else {
        return Vec::new();
    };
    let stop = app.kept.stop_number(block);
    let selected = stop.is_some() && state.selected == stop;
    let placement = match (app.kept.round_number(block), stop) {
        (Some(_), Some(stop)) => Placement::Viewer {
            selected,
            open: state.open.contains(&stop),
        },
        _ => Placement::Committed,
    };
    let mut rows = kept.rows(placement, width, &app.look);
    // A reasoning or a compaction selected (since step 25): its first row
    // is drawn as a selected box's border is.
    if selected
        && app.kept.round_number(block).is_none()
        && let Some(first) = rows.iter_mut().find(|row| row.width() > 0)
    {
        let style = app.look.theme.style(crate::theme::Token::BorderSelected);
        *first = Line::from(cineris_term::Span::new(first.text().into_owned(), style));
    }
    rows
}

impl App {
    /// Opens the viewer at the latest tool box, or at the box `n`, counted
    /// from one, opened (`/view n`).
    pub(super) fn open_viewer(&mut self, n: Option<usize>) {
        // `n` counts boxes (rounds); the viewer selects among its stops
        // (since step 25: rounds, reasoning, compactions).
        let asked = n
            .and_then(|n| n.checked_sub(1))
            .and_then(|round| self.kept.round_block(round))
            .and_then(|block| self.kept.stop_number(block));
        let selected = asked.or_else(|| self.boxes_kept().checked_sub(1));
        let mut open = BTreeSet::new();
        if let Some(asked) = asked {
            open.insert(asked);
        }
        if self.zone.is_none() {
            // One small file, read once, when the viewer first opens.
            self.zone = Some(Zone::system());
        }
        let mut scroll = Scroll::default();
        // The viewer opens at the transcript's start, with the selected box
        // brought into view.
        scroll.go_to_start();
        self.viewer = Some(Viewer {
            scroll,
            selected,
            open,
            reveal: true,
            ..Viewer::default()
        });
    }

    /// Opens the viewer on the open dialog's request, drawn whole: `Esc`
    /// returns to the dialog, still waiting for its answer.
    pub(super) fn open_whole(&mut self) {
        if self.zone.is_none() {
            self.zone = Some(Zone::system());
        }
        let mut scroll = Scroll::default();
        scroll.go_to_start();
        self.viewer = Some(Viewer {
            scroll,
            whole: self.dialogs.front().map(|dialog| dialog.id.clone()),
            ..Viewer::default()
        });
    }

    /// Opens the viewer on a stored artifact's text, under its label: `Esc`
    /// returns to the inline screen. What was cut to the core's limit is
    /// said at its end.
    pub(super) fn open_text(
        &mut self,
        label: &str,
        artifact: &cineris_protocol::methods::SessionArtifactResult,
    ) {
        if self.zone.is_none() {
            self.zone = Some(Zone::system());
        }
        let mut lines = crate::view::round::output_lines(&artifact.text);
        if artifact.cut {
            lines.push(String::new());
            lines.push(format!(
                "(cut: {} of its {} bytes are shown)",
                artifact.text.len(),
                artifact.bytes
            ));
        }
        let mut scroll = Scroll::default();
        scroll.go_to_start();
        self.viewer = Some(Viewer {
            scroll,
            text: Some((label.to_owned(), lines)),
            ..Viewer::default()
        });
    }

    /// Opens the viewer on `text`, for copying by hand where the terminal
    /// is thought not to take the clipboard sequence (since step 25): the
    /// mouse is not reported while it is open.
    pub(super) fn open_to_copy(&mut self, text: &str) {
        let artifact = cineris_protocol::methods::SessionArtifactResult {
            text: text.to_owned(),
            bytes: text.len() as u64,
            cut: false,
        };
        self.open_text("The last answer, to select and copy", &artifact);
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.to_copy = true;
        }
    }

    /// Whether the viewer shows text to copy by hand, with the mouse not
    /// reported.
    pub(super) fn viewing_to_copy(&self) -> bool {
        self.viewer.as_ref().is_some_and(|viewer| viewer.to_copy)
    }

    /// Whether the viewer is open: the loop then draws [`viewer_frame`]
    /// instead of the inline screen. A request shown whole closes it when
    /// its dialog is no longer the one open (answered elsewhere).
    ///
    /// [`viewer_frame`]: Self::viewer_frame
    #[must_use]
    pub fn viewing(&self) -> bool {
        self.viewer.as_ref().is_some_and(|viewer| {
            viewer
                .whole
                .as_ref()
                .is_none_or(|id| self.dialogs.front().is_some_and(|dialog| &dialog.id == id))
        })
    }

    /// Forgets a viewer that is no longer shown.
    pub(super) fn drop_stale_viewer(&mut self) {
        if self.viewer.is_some() && !self.viewing() {
            self.viewer = None;
        }
    }

    /// How many stops the committed transcript holds: its boxes, and
    /// since step 25 its reasoning and compactions.
    fn boxes_kept(&self) -> usize {
        self.kept.stops()
    }

    /// What the viewer shows that is not a kept block: the request whole,
    /// the text, or the open answer's committed rows and the live rows. With a request or a text,
    /// no kept block is shown.
    fn viewer_rest(&self, state: &Viewer, width: usize) -> (usize, std::sync::Arc<Vec<Line>>) {
        if let Some(id) = &state.whole {
            let rows = self
                .dialogs
                .front()
                .filter(|dialog| &dialog.id == id)
                .map(|dialog| dialog.whole(width, &self.look))
                .unwrap_or_default();
            return (0, std::sync::Arc::new(rows));
        }
        if state.text.is_some() {
            let rows = state
                .text_rows
                .as_ref()
                .filter(|(made_for, _)| *made_for == width)
                .map(|(_, rows)| rows.clone())
                .unwrap_or_default();
            return (0, std::sync::Arc::new(rows));
        }
        // The answer still open: the rows it has committed, then its live
        // ones, as the scrollback and the live region hold them (the review
        // of step 25).
        (self.kept.len(), self.screen_live(width))
    }

    /// Runs `act` on the viewer's window, with what it is drawn from at
    /// `width`.
    fn with_window<T>(
        &self,
        state: &mut Viewer,
        width: usize,
        act: impl FnOnce(&mut Scroll, &mut Source<'_>) -> T,
    ) -> T {
        let (blocks, live) = self.viewer_rest(state, width);
        let mut scroll = std::mem::take(&mut state.scroll);
        let shown = &*state;
        let mut make = |block: usize| block_rows(self, shown, block, width);
        let mut source = Source {
            blocks,
            make: &mut make,
            live: &live,
            width,
        };
        let result = act(&mut scroll, &mut source);
        state.scroll = scroll;
        result
    }

    /// The viewer's screen, `rows` rows of `width` columns: the title and
    /// its rule, the transcript from where it is scrolled to, and the rule
    /// and the keys.
    pub fn viewer_frame(&mut self, rows: usize, width: usize, now: Instant) -> Vec<Line> {
        let since = now.saturating_duration_since(self.began).as_millis();
        self.tick = usize::try_from(since / super::super::SPIN.as_millis()).unwrap_or(0);
        let Some(mut state) = self.viewer.take() else {
            return Vec::new();
        };
        let width = width.max(1);
        state.width = width;
        // A text's rows, made once for the width (`output_lines` has made
        // it safe to show): a scroll draws them, it does not make them.
        if let Some((label, lines)) = &state.text
            && state
                .text_rows
                .as_ref()
                .is_none_or(|(made_for, _)| *made_for != width)
        {
            let mut rows = crate::layout::wrap(
                &[cineris_term::Span::new(
                    cineris_term::sanitize(label).into_owned(),
                    self.look.theme.style(crate::theme::Token::Muted),
                )],
                width,
            );
            rows.push(Line::new());
            for line in lines {
                rows.extend(crate::layout::wrap(
                    &[cineris_term::Span::plain(line.clone())],
                    width,
                ));
            }
            state.text_rows = Some((width, rows));
        }
        let body = rows.saturating_sub(FRAME_ROWS).max(1);
        state.scroll.height = body;
        if std::mem::take(&mut state.find) {
            self.viewer_search(&mut state, width);
        }
        if std::mem::take(&mut state.reveal)
            && let Some(block) = state.selected.and_then(|stop| self.kept.stop_block(stop))
        {
            let rows = block_rows(self, &state, block, width);
            let start = usize::from(self.kept.get(block).is_some_and(|kept| kept.gap));
            self.with_window(&mut state, width, |scroll, source| {
                scroll.reveal(source, block, start, rows.len());
            });
        }
        let window = self.with_window(&mut state, width, |scroll, source| {
            scroll.window(source, body)
        });
        // A request shown whole whose every row a frame has shown was seen
        // whole: what its dialog offers only then is offered (since step
        // 25, the rewind's `1`).
        if let Some(id) = state.whole.clone()
            && let Some(dialog) = self.dialogs.front_mut().filter(|dialog| dialog.id == id)
        {
            let rows = dialog.whole(width, &self.look).len();
            if state.saw(&window.places, rows, width) {
                dialog.seen_whole();
            }
        }

        let mut lines = viewer::title(&self.viewer_title(), width, &self.look);
        state.body_top = lines.len();
        state.shown = window.places;
        lines.extend(window.rows);
        while lines.len() < body + 2 {
            lines.push(Line::new());
        }
        let keys = match (&state.typing, &state.search) {
            (Some(query), _) => ViewerKeys::Search {
                query: query.clone(),
                missed: false,
            },
            (None, Some((query, true))) => ViewerKeys::Search {
                query: query.clone(),
                missed: true,
            },
            _ => match state
                .selected
                .filter(|_| state.whole.is_none() && state.text.is_none())
            {
                Some(selected) => {
                    let block = self.kept.stop_block(selected);
                    match block.and_then(|block| self.kept.round_number(block)) {
                        Some(round) => ViewerKeys::Boxes {
                            selected: round + 1,
                            count: self.kept.rounds(),
                            open: state.open.contains(&selected),
                        },
                        None => ViewerKeys::Summary {
                            label: match block.and_then(|block| self.kept.summary(block)) {
                                Some(Summary::Compaction(_)) => "summary",
                                _ => "reasoning",
                            },
                            read: state.asked.filter(|(stop, _)| *stop == selected).map(
                                |(_, asked)| match asked {
                                    Asked::Reading(_) => SummaryRead::Reading,
                                    Asked::Failed => SummaryRead::Failed,
                                },
                            ),
                        },
                    }
                }
                None => ViewerKeys::Lines,
            },
        };
        lines.extend(viewer::keys(&keys, width, &self.look));
        self.viewer = Some(state);
        lines
    }

    /// Looks for the last search from the row after the window's top, going
    /// round, and moves the window's top to the row found.
    fn viewer_search(&mut self, state: &mut Viewer, width: usize) {
        let query = state
            .search
            .as_ref()
            .map(|(query, _)| query.to_lowercase())
            .unwrap_or_default();
        let found = self.with_window(state, width, |scroll, source| {
            let top = scroll.top_of(source).unwrap_or((0, 0));
            let count = source.blocks + 1;
            let first = top.0.min(count - 1);
            for step in 0..=count {
                let block = (first + step) % count;
                let rows = if block >= source.blocks {
                    std::sync::Arc::clone(source.live)
                } else {
                    scroll.rows.of(block, source.width, source.make)
                };
                let range = match step {
                    0 => top.1 + 1..rows.len(),
                    _ if step == count => 0..(top.1 + 1).min(rows.len()),
                    _ => 0..rows.len(),
                };
                for row in range {
                    if text_of(&rows[row]).to_lowercase().contains(&query) {
                        return Some((block, row));
                    }
                }
            }
            None
        });
        if let Some((_, missed)) = state.search.as_mut() {
            *missed = found.is_none();
        }
        if let Some((block, row)) = found {
            state.scroll.anchor = Anchor::At { block, row };
        }
    }

    /// What the title row says: the folder, when the session began in the
    /// local time, and the turns the view holds.
    fn viewer_title(&self) -> ViewerTitle {
        let began = self.session.as_ref().and_then(|session| {
            let time = UtcTime::parse(session.created_at.as_str()).ok()?;
            let offset = self.zone.as_ref().map_or(0, |zone| zone.offset_at(time));
            let local = time.unix_secs() + i64::from(offset);
            let minutes = local.div_euclid(60);
            Some(format!(
                "{:02}:{:02}",
                minutes.div_euclid(60).rem_euclid(24),
                minutes.rem_euclid(60)
            ))
        });
        ViewerTitle {
            folder: self.folder.clone(),
            began,
            turns: self.turns,
        }
    }

    /// A box's selection or opening changed: its rows are drawn again.
    fn forget_box(&self, state: &mut Viewer, stop: Option<usize>) {
        if let Some(block) = stop.and_then(|stop| self.kept.stop_block(stop)) {
            state.scroll.rows.forget(block);
        }
    }

    /// A key while the viewer is open. Global keys keep their meaning.
    pub(super) fn viewer_key(&mut self, key: Key, at: Instant) -> bool {
        if let Some(action @ (Action::Interrupt | Action::Redraw)) =
            self.input.keymap().lookup(&[], key)
        {
            // The editor is hidden: `Ctrl+C` leaves its text alone.
            let outcome = if action == Action::Interrupt {
                self.input.interrupt_hidden()
            } else {
                self.input.act(action)
            };
            self.outcome(outcome, at);
            return true;
        }
        let boxes = self.boxes_kept();
        let Some(mut state) = self.viewer.take() else {
            return false;
        };
        if let Some(query) = state.typing.as_mut() {
            match key.code {
                KeyCode::Esc => state.typing = None,
                KeyCode::Enter => {
                    let query = std::mem::take(query);
                    state.typing = None;
                    if !query.is_empty() {
                        state.search = Some((query, false));
                        state.find = true;
                    }
                }
                KeyCode::Backspace => {
                    query.pop();
                }
                KeyCode::Char(c) if !key.mods.contains(Mods::CTRL) => query.push(c),
                _ => {}
            }
            self.viewer = Some(state);
            return true;
        }
        // The viewer's keys are its context's in the key map (since step
        // 25); a key bound to none of them does nothing here.
        let action = self.input.keymap().lookup(&[Context::Viewer], key);
        let close = self.viewer_act(&mut state, action, boxes);
        self.read_summary(&mut state);
        if !close {
            self.viewer = Some(state);
        }
        true
    }

    /// Shows `text` in the viewer under `label`, the transcript's view kept
    /// behind it for `Esc` (since step 25). A summary's read the view behind
    /// was waiting for is given up: its answer goes to the view on top
    /// only, so the view behind would say `reading the summary…` for good
    /// (the review of the fixes of step 25's second pull request); `Enter`
    /// there asks again.
    fn show_summary(state: &mut Viewer, label: &str, text: &str) {
        let lines = crate::view::round::whole_lines(text);
        let mut behind = std::mem::take(state);
        behind.asked = None;
        let mut scroll = Scroll::default();
        scroll.go_to_start();
        *state = Viewer {
            scroll,
            text: Some((label.to_owned(), lines)),
            behind: Some(Box::new(behind)),
            ..Viewer::default()
        };
    }

    /// Asks for the compaction's summary the viewer wants, through the link
    /// (`session/read` with the item's id, since step 25); the keys row
    /// says it is being read.
    fn read_summary(&mut self, state: &mut Viewer) {
        let Some(item) = state.wants.take() else {
            return;
        };
        let Some(stop) = state.selected else {
            return;
        };
        let Some(session) = self.session.as_ref().map(|state| state.session_id.clone()) else {
            state.asked = Some((stop, Asked::Failed));
            return;
        };
        let ticket = self.link.send(cineris_protocol::methods::Op::SessionRead(
            cineris_protocol::methods::SessionReadParams {
                session_id: session.clone(),
                item_id: Some(cineris_protocol::ItemId::from(item.as_str())),
                limit: None,
                cursor: None,
                redacted: false,
            },
        ));
        self.pending
            .insert(ticket, super::Pending::Summary(session));
        state.asked = Some((stop, Asked::Reading(ticket)));
    }

    /// A compaction's summary read, with `ticket`: shown in the viewer, the
    /// transcript's view behind it, only on the view that asked and while
    /// it is still at the stop it asked at. The viewer closed and opened
    /// again on something else (a request whole, `/copy`'s text, another
    /// summary), or moved to another stop, drops it (since the review of
    /// step 25's second pull request). A read that holds no summary is said
    /// in the keys row.
    pub(super) fn summary_read(
        &mut self,
        ticket: Ticket,
        page: &cineris_protocol::methods::SessionReadResult,
    ) {
        let Some(state) = self.viewer.as_mut() else {
            return;
        };
        let Some((stop, Asked::Reading(asked))) = state.asked else {
            return;
        };
        if asked != ticket {
            return;
        }
        state.asked = None;
        if state.selected != Some(stop) || state.text.is_some() || state.whole.is_some() {
            return;
        }
        let text = page.items.iter().find_map(|item| match item {
            cineris_protocol::Item::Summary(summary) => Some(summary.text.clone()),
            _ => None,
        });
        match text {
            Some(text) => {
                Self::show_summary(state, "The summary the conversation goes on from", &text);
            }
            None => state.asked = Some((stop, Asked::Failed)),
        }
    }

    /// A compaction's summary could not be read, with `ticket`: the keys
    /// row of the view that asked says so (since the review of step 25's
    /// second pull request).
    pub(super) fn summary_unread(&mut self, ticket: Ticket) {
        if let Some(state) = self.viewer.as_mut()
            && let Some((stop, Asked::Reading(asked))) = state.asked
            && asked == ticket
        {
            state.asked = Some((stop, Asked::Failed));
        }
    }

    /// Does `action` in the viewer; says whether it closes it.
    fn viewer_act(&self, state: &mut Viewer, action: Option<Action>, boxes: usize) -> bool {
        let width = state.width.max(1);
        let page = state.scroll.height.max(1);
        match action {
            // From a summary, back to the transcript it was opened from.
            Some(Action::ViewerClose) => match state.behind.take() {
                Some(behind) => *state = *behind,
                None => return true,
            },
            Some(Action::ViewerUp) => match state.selected {
                Some(selected) => {
                    let next = selected.saturating_sub(1);
                    self.select(state, next);
                }
                None => self.with_window(state, width, |scroll, source| {
                    scroll.up(source, 1);
                }),
            },
            Some(Action::ViewerDown) => match state.selected {
                Some(selected) => {
                    let next = (selected + 1).min(boxes.saturating_sub(1));
                    self.select(state, next);
                }
                None => self.with_window(state, width, |scroll, source| {
                    scroll.down(source, 1);
                }),
            },
            Some(Action::ViewerToggle) => {
                let summary = state
                    .selected
                    .and_then(|selected| self.kept.stop_block(selected))
                    .and_then(|block| self.kept.summary(block));
                match summary {
                    Some(Summary::Reasoning(text)) => {
                        Self::show_summary(state, "The reasoning, whole", &text);
                    }
                    Some(Summary::Compaction(item)) => state.wants = Some(item),
                    None => {
                        if let Some(selected) = state.selected {
                            if !state.open.remove(&selected) {
                                state.open.insert(selected);
                            }
                            self.forget_box(state, Some(selected));
                            state.reveal = true;
                        }
                    }
                }
            }
            Some(Action::ViewerPageUp) => self.with_window(state, width, |scroll, source| {
                scroll.up(source, page);
            }),
            Some(Action::ViewerPageDown) => self.with_window(state, width, |scroll, source| {
                scroll.down(source, page);
            }),
            Some(Action::ViewerScrollUp) => self.with_window(state, width, |scroll, source| {
                scroll.up(source, WHEEL_ROWS);
            }),
            Some(Action::ViewerScrollDown) => self.with_window(state, width, |scroll, source| {
                scroll.down(source, WHEEL_ROWS);
            }),
            Some(Action::ViewerTop) => state.scroll.go_to_start(),
            // To the end: the window follows it, as the last page.
            Some(Action::ViewerBottom) => state.scroll.follow(),
            Some(Action::ViewerSearch) => {
                state.typing = Some(String::new());
                state.search = None;
            }
            Some(Action::ViewerNext) if state.search.is_some() => state.find = true,
            _ => {}
        }
        false
    }

    /// The mouse while the viewer is open (since step 25): the wheel
    /// scrolls, and a click on a box's row selects it and opens or closes
    /// it, each through the key map's gestures. What a row the last frame
    /// drew shows is what a click finds.
    pub(super) fn viewer_mouse(&mut self, mouse: Mouse) -> bool {
        let boxes = self.boxes_kept();
        let Some(mut state) = self.viewer.take() else {
            return false;
        };
        let keymap = self.input.keymap();
        let gesture = match mouse.kind {
            MouseKind::Wheel(Wheel::Up) => Some(Gesture::WheelUp),
            MouseKind::Wheel(Wheel::Down) => Some(Gesture::WheelDown),
            MouseKind::Press(Button::Left) => {
                state.pressed = Some((mouse.col, mouse.row));
                None
            }
            MouseKind::Release(_) => state
                .pressed
                .take()
                .filter(|&pressed| pressed == (mouse.col, mouse.row))
                .map(|_| Gesture::Click),
            _ => None,
        };
        let action = gesture.and_then(|gesture| keymap.lookup_gesture(&[Context::Viewer], gesture));
        let mut close = false;
        if action == Some(Action::ViewerToggle) {
            // The box clicked, if a box's row was: selected, then opened or
            // closed as `Enter` does.
            let row = usize::from(mouse.row);
            let round = row
                .checked_sub(state.body_top)
                .and_then(|row| state.shown.get(row))
                .and_then(|place| match place {
                    Place::Kept { block, .. } => self.kept.stop_number(*block),
                    _ => None,
                });
            if let Some(round) = round.filter(|_| state.whole.is_none() && state.text.is_none()) {
                self.select(&mut state, round);
                close = self.viewer_act(&mut state, action, boxes);
            }
        } else {
            close = self.viewer_act(&mut state, action, boxes);
        }
        self.read_summary(&mut state);
        if !close {
            self.viewer = Some(state);
        }
        true
    }

    /// Selects box `round`, drawing the one selected before and it again,
    /// and brings it into view.
    fn select(&self, state: &mut Viewer, round: usize) {
        let before = state.selected.replace(round);
        if before != Some(round) {
            self.forget_box(state, before);
            self.forget_box(state, Some(round));
        }
        state.reveal = true;
    }

    /// A paste while the viewer is open: its search's, while one is being
    /// typed, on one line; otherwise nothing, and never the hidden editor's
    /// (the review of step 23).
    pub(super) fn viewer_paste(&mut self, text: &str) -> bool {
        if let Some(query) = self.viewer.as_mut().and_then(|state| state.typing.as_mut()) {
            query.extend(text.chars().map(|c| if c.is_control() { ' ' } else { c }));
        }
        true
    }

    /// A turn the view holds, by its number.
    pub(super) fn note_turn(&mut self, number: u64) {
        self.turns = Some(match self.turns {
            Some((first, last)) => (first.min(number), last.max(number)),
            None => (number, number),
        });
    }
}

/// A row's text, for a search.
fn text_of(line: &Line) -> String {
    line.spans().iter().map(|span| span.text()).collect()
}
