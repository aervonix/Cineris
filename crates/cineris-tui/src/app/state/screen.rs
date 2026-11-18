//! The full-screen presenter's view of the session (since step 25;
//! `docs/architecture/09-tui.md`, "Full screen as the alternative";
//! `docs/steps/25/answered.md`, point 2): the kept transcript, the rows of
//! the block still open and the live rows, in a window that follows the end
//! until it is moved up, above the same panels and bottom the inline
//! presenter draws.
//!
//! A committed box is drawn as the inline presenter committed it, unless a
//! click opened or closed it here; the viewer (`Ctrl+O`) is the way to it
//! by keys, on the same alternate screen.

use std::collections::BTreeMap;
use std::time::Instant;

use cineris_term::{
    Button, Cursor, Key, KeyCode, Line, Mods, Mouse, MouseKind, Span, Style, Wheel,
};

use super::App;
use super::viewer::WHEEL_ROWS;
use crate::app::scroll::{Place, Scroll, Source, Window};
use crate::app::search::TextSearch;
use crate::keys::{Action, Context, Gesture};
use crate::theme::Token;
use crate::view::Placement;

/// A search of the transcript (`Ctrl+F`), while it is open.
#[derive(Debug, Default)]
struct Searching {
    /// What is typed.
    query: String,
    /// The worker's answer for the query: each kept block that holds it,
    /// and how many times, in order. `None` until it comes.
    found: Option<Vec<(usize, usize)>>,
    /// The current match, by its place among all the matches, oldest
    /// first.
    current: Option<usize>,
    /// Where the current match is drawn: its block, its row there, and
    /// which match of that row it is.
    at: Option<(usize, usize, usize)>,
    /// The box the search opened for the current match, and how it was
    /// drawn before (`None`: as it committed).
    opened: Option<(usize, Option<bool>)>,
}

/// The full-screen presenter's view: where the window is, the boxes opened
/// or closed here, and what the last frame drew, for the mouse.
#[derive(Debug, Default)]
pub(crate) struct ScreenView {
    /// The search, while it is open.
    search: Option<Searching>,
    /// The search's worker, started by the first search, and how many
    /// kept blocks it has been given.
    worker: Option<TextSearch>,
    fed: usize,
    /// The window over the transcript.
    scroll: Scroll,
    /// The boxes a click opened (`true`) or closed, by their place among
    /// the rounds; the others are drawn as they committed.
    open: BTreeMap<usize, bool>,
    /// Where each row of the transcript area the last frame drew came
    /// from: what a click finds. The area is the screen's top.
    shown: Vec<Place>,
    /// Where the left button went down, for a click.
    pressed: Option<(u16, u16)>,
    /// Where the panels' column begins, when it is shown: a click there
    /// is not the transcript's.
    column: Option<usize>,
    /// The width the open block's rows were last laid out at.
    width: usize,
}

impl ScreenView {
    /// Forgets the rows made: the look changed.
    pub(super) fn forget_rows(&mut self) {
        self.scroll.rows.clear();
    }

    /// Forgets one kept block's rows: it is to be drawn again.
    pub(super) fn forget_block(&mut self, block: usize) {
        self.scroll.rows.forget(block);
    }
}

/// The columns between the transcript and the panels' column.
const GAP: usize = 2;

/// The most characters a search's query holds: more typed or pasted is
/// left out (the review of step 25).
const QUERY_MOST: usize = 256;

/// The narrowest window, in columns, that has the panels beside the
/// transcript (the coordinator's answer of 2026-11-17).
pub(crate) const SIDE_FROM: usize = 140;

/// The panels' column for a window of `cols` columns: none below
/// [`SIDE_FROM`], 40 columns there, and one more for each two past it, up
/// to 56.
pub(crate) fn side_column(cols: usize) -> Option<usize> {
    (cols >= SIDE_FROM).then(|| (40 + (cols - SIDE_FROM) / 2).min(56))
}

/// Where the full screen's panels' column begins, if it is shown.
fn view_column(app: &App) -> Option<usize> {
    app.screen.as_ref().and_then(|view| view.column)
}

/// A block's rows in the full-screen view: a round as it committed, or as a
/// click or the search left it.
fn block_rows(
    app: &App,
    open: &BTreeMap<usize, bool>,
    searched: Option<usize>,
    block: usize,
    width: usize,
) -> Vec<Line> {
    let Some(kept) = app.kept.get(block) else {
        return Vec::new();
    };
    let round = app.kept.round_number(block);
    let placement = match round.and_then(|round| open.get(&round)) {
        Some(&open) => Placement::Screen {
            open,
            searched: open && round.is_some() && round == searched,
        },
        None => Placement::Committed,
    };
    kept.rows(placement, width, &app.look)
}

impl App {
    /// Whether the application draws with the full-screen presenter.
    #[must_use]
    pub fn full_screen(&self) -> bool {
        self.screen.is_some()
    }

    /// The columns the full screen's transcript may use: the window's, less
    /// the panels' column and its gap on a wide window.
    fn transcript_width(&self) -> usize {
        let full = self.width.max(1);
        side_column(full + 1).map_or(full, |column| full.saturating_sub(column + GAP).max(1))
    }

    /// Whether the viewer and the full-screen presenter report the mouse
    /// (`terminal.mouse`).
    #[must_use]
    pub fn mouse(&self) -> bool {
        self.settings.mouse && !self.viewing_to_copy()
    }

    /// Runs `act` on the full-screen view's window, with what it is drawn
    /// from at `width`: the kept blocks, then the rows of the block still
    /// open and the live rows.
    fn with_screen<T>(
        &self,
        view: &mut ScreenView,
        width: usize,
        live: &std::sync::Arc<Vec<Line>>,
        act: impl FnOnce(&mut Scroll, &mut Source<'_>) -> T,
    ) -> T {
        let mut scroll = std::mem::take(&mut view.scroll);
        let open = &view.open;
        let searched = view
            .search
            .as_ref()
            .and_then(|search| search.opened)
            .map(|(round, _)| round);
        let mut make = |block: usize| block_rows(self, open, searched, block, width);
        let mut source = Source {
            blocks: self.kept.len(),
            make: &mut make,
            live,
            width,
        };
        let result = act(&mut scroll, &mut source);
        view.scroll = scroll;
        result
    }

    /// The rows under the kept blocks: those committed of the block still
    /// open, then the live rows.
    pub(super) fn screen_live(&self, width: usize) -> std::sync::Arc<Vec<Line>> {
        let mut rows = self.transcript.open_rows().to_vec();
        rows.extend(self.transcript.live(width, &self.look, self.tick));
        std::sync::Arc::new(rows)
    }

    /// The full-screen presenter's frame for `now`: every row of the window,
    /// top to bottom (the transcript area, the panels' block, the bottom),
    /// and where the cursor goes, if anywhere.
    pub fn screen_frame(&mut self, now: Instant) -> (Vec<Line>, Option<Cursor>) {
        let full = self.width.max(1);
        // On a wide window the panels have a column of their own, and the
        // transcript the rest.
        let column = side_column(full + 1);
        let width = self.transcript_width();
        let Some(mut view) = self.screen.take() else {
            return (Vec::new(), None);
        };
        // The block still open was laid out at another width: again, at
        // this one (an answer resized while it streams).
        if view.width != width {
            if view.width != 0 {
                self.transcript.rewrap_open(width, &self.look);
            }
            view.width = width;
        }
        let (_, boxes) = self.prepare(now, width);
        for block in std::mem::take(&mut self.redraw_blocks) {
            view.forget_block(block);
        }
        self.feed_search(&mut view);
        let _ = self.take_found(&mut view, width);
        let live = self.screen_live(width);
        // The live rows the panels may leave the transcript, as inline: the
        // full screen's transcript area scrolls, so they count no further.
        let live_len = live.len().min(4);
        let mut bottom = self.bottom(now, full, &boxes, live_len, column);
        // The search's row in the hint row's place, the cursor after what
        // is typed.
        // Not over a dialog's own row: its keys are the dialog's.
        if let Some(search) = view.search.as_ref().filter(|_| self.dialogs.is_empty())
            && let Some(last) = bottom.lines.last_mut()
        {
            let (row, col) = self.search_row(search, full, view.worker.is_some());
            *last = row;
            bottom.cursor = Some((bottom.lines.len() - 1, col));
        }
        let rows = self.rows.max(1);
        // A dialog may fill the window: the transcript then has no row, and
        // the dialog's own last row, its keys, is still drawn (the review of
        // step 25).
        let height = rows.saturating_sub(bottom.block.len() + bottom.lines.len());
        let mut window = if height == 0 {
            Window::default()
        } else {
            self.with_screen(&mut view, width, &live, |scroll, source| {
                scroll.window(source, height)
            })
        };
        if let Some(search) = &view.search {
            self.mark_matches(search, &mut window);
        }
        if !window.at_end {
            self.more_below(&mut window, width);
        }
        view.shown = window.places;
        view.column = column.map(|_| width + GAP);
        let mut lines = window.rows;
        // The panels' column beside the transcript, from its first row.
        if column.is_some() {
            for (line, side) in lines.iter_mut().zip(bottom.column) {
                let pad = width.saturating_sub(line.width()) + GAP;
                line.push(Span::plain(" ".repeat(pad)));
                for span in side.spans() {
                    line.push(span.clone());
                }
            }
        }
        let top = lines.len() + bottom.block.len();
        lines.extend(bottom.block);
        lines.extend(bottom.lines);
        let cursor = bottom.cursor.map(|(row, col)| Cursor {
            row: top + row,
            col,
        });
        self.screen = Some(view);
        (lines, cursor)
    }

    /// The last row of a window that does not follow the end says there is
    /// more below, and how to go there.
    fn more_below(&self, window: &mut Window, width: usize) {
        let Some(last) = window.rows.last_mut() else {
            return;
        };
        let keys = self
            .input
            .keymap()
            .keys_of(Action::ScreenPageDown)
            .first()
            .map(|key| crate::keys::name(*key))
            .unwrap_or_else(|| "pgdn".to_owned());
        let text = format!(
            "  more below {} {keys} goes to the end",
            self.look.glyphs.separator
        );
        *last = crate::layout::cut(
            &Line::from(Span::new(text, self.look.theme.style(Token::Hint))),
            width,
            self.look.glyphs.ellipsis,
        );
        if let Some(place) = window.places.last_mut() {
            *place = Place::Empty;
        }
    }

    /// A key the full-screen view may take: the search's while one is
    /// open, or one of its context's bound to nothing where the input has
    /// the keyboard (since step 25). Says whether it took it.
    pub(super) fn screen_key(&mut self, key: Key) -> bool {
        if self.screen.is_none() || self.viewer.is_some() || !self.dialogs.is_empty() {
            return false;
        }
        if self
            .screen
            .as_ref()
            .is_some_and(|view| view.search.is_some())
        {
            self.search_key(key);
            return true;
        }
        let keymap = self.input.keymap();
        let Some(action) = keymap
            .lookup(&[Context::Screen], key)
            .filter(|action| action.context() == Context::Screen)
        else {
            return false;
        };
        // The editor's, a menu's or a focused panel's own keys come first.
        let contexts = self.input.contexts();
        if keymap
            .lookup(&contexts, key)
            .is_some_and(|other| other.context() != Context::Global)
        {
            return false;
        }
        self.screen_act(action, None);
        true
    }

    /// The mouse in the full-screen view: the wheel scrolls, and a click on
    /// a committed box's row opens or closes it, each through the key map's
    /// gestures.
    pub(super) fn screen_mouse(&mut self, mouse: Mouse) -> bool {
        let Some(view) = self.screen.as_mut() else {
            return false;
        };
        let gesture = match mouse.kind {
            MouseKind::Wheel(Wheel::Up) => Some(Gesture::WheelUp),
            MouseKind::Wheel(Wheel::Down) => Some(Gesture::WheelDown),
            MouseKind::Press(Button::Left) => {
                view.pressed = Some((mouse.col, mouse.row));
                None
            }
            MouseKind::Release(_) => view
                .pressed
                .take()
                .filter(|&pressed| pressed == (mouse.col, mouse.row))
                .map(|_| Gesture::Click),
            _ => None,
        };
        let Some(action) = gesture.and_then(|gesture| {
            self.input
                .keymap()
                .lookup_gesture(&[Context::Screen], gesture)
        }) else {
            return false;
        };
        // A click under a dialog does nothing: the dialog has the keys.
        if action == Action::ScreenToggle && !self.dialogs.is_empty() {
            return true;
        }
        // A click in the panels' column is not on the transcript.
        if action == Action::ScreenToggle
            && view_column(self).is_some_and(|column| usize::from(mouse.col) >= column)
        {
            return true;
        }
        self.screen_act(action, Some(usize::from(mouse.row)));
        true
    }

    /// Does `action` in the full-screen view; `row` is the screen row a
    /// click was on.
    fn screen_act(&mut self, action: Action, row: Option<usize>) {
        let width = self.transcript_width();
        let live = self.screen_live(width);
        let Some(mut view) = self.screen.take() else {
            return;
        };
        let page = view.scroll.height.max(1);
        match action {
            Action::ScreenPageUp => self.with_screen(&mut view, width, &live, |scroll, source| {
                scroll.up(source, page.saturating_sub(1).max(1));
            }),
            Action::ScreenPageDown => {
                self.with_screen(&mut view, width, &live, |scroll, source| {
                    scroll.down(source, page.saturating_sub(1).max(1));
                })
            }
            Action::ScreenLineUp => self.with_screen(&mut view, width, &live, |scroll, source| {
                scroll.up(source, 1);
            }),
            Action::ScreenLineDown => {
                self.with_screen(&mut view, width, &live, |scroll, source| {
                    scroll.down(source, 1);
                })
            }
            Action::ScreenScrollUp => {
                self.with_screen(&mut view, width, &live, |scroll, source| {
                    scroll.up(source, WHEEL_ROWS);
                })
            }
            Action::ScreenScrollDown => {
                self.with_screen(&mut view, width, &live, |scroll, source| {
                    scroll.down(source, WHEEL_ROWS);
                });
            }
            Action::ScreenSearch => {
                if view.worker.is_none() {
                    view.worker = TextSearch::start(self.waker.clone());
                    view.fed = 0;
                }
                view.search = Some(Searching::default());
            }
            Action::ScreenToggle => {
                let clicked =
                    row.and_then(|row| view.shown.get(row))
                        .and_then(|place| match place {
                            Place::Kept { block, .. } => {
                                self.kept.round_number(*block).map(|round| (*block, round))
                            }
                            _ => None,
                        });
                if let Some((block, round)) = clicked {
                    // Open if it showed no detail as committed; then each
                    // click turns it over.
                    let now_open = match view.open.get(&round) {
                        Some(&open) => !open,
                        None => !self.kept.get(block).is_some_and(|kept| {
                            kept.round().is_some_and(|round| {
                                crate::view::round::shows_detail(round, &self.look)
                            })
                        }),
                    };
                    view.open.insert(round, now_open);
                    view.scroll.rows.forget(block);
                }
            }
            _ => {}
        }
        self.screen = Some(view);
    }

    /// A paste while the search is open: its query's, on one line. Says
    /// whether the search took it.
    pub(super) fn screen_paste(&mut self, text: &str) -> bool {
        let Some(mut view) = self.screen.take() else {
            return false;
        };
        let taken = if let Some(search) = view.search.as_mut() {
            let room = QUERY_MOST.saturating_sub(search.query.chars().count());
            search.query.extend(
                text.chars()
                    .take(room)
                    .map(|c| if c.is_control() { ' ' } else { c }),
            );
            self.ask_search(&mut view);
            true
        } else {
            false
        };
        self.screen = Some(view);
        taken
    }

    /// Hands the search's worker the blocks kept since it was last given
    /// any.
    fn feed_search(&self, view: &mut ScreenView) {
        let Some(worker) = view.worker.as_ref() else {
            return;
        };
        if view.fed > self.kept.len() {
            worker.clear();
            view.fed = 0;
        }
        let blocks: Vec<(usize, String)> = (view.fed..self.kept.len())
            .filter_map(|index| self.kept.get(index).map(|block| (index, block.text())))
            .collect();
        view.fed = self.kept.len();
        worker.add(blocks);
    }

    /// Asks the worker for the query as it is now; the current match is
    /// chosen again when the answer comes.
    fn ask_search(&self, view: &mut ScreenView) {
        self.close_search_box(view);
        self.feed_search(view);
        let Some(search) = view.search.as_mut() else {
            return;
        };
        search.found = None;
        search.current = None;
        search.at = None;
        if let Some(worker) = view.worker.as_mut() {
            worker.ask(&search.query);
        }
    }

    /// Takes the worker's answer, if one came, and goes to the newest match.
    fn take_found(&self, view: &mut ScreenView, width: usize) -> bool {
        let Some(found) = view.worker.as_ref().and_then(TextSearch::answer) else {
            return false;
        };
        self.found(view, found, width);
        true
    }

    /// The ASCII glyphs from now on, for tests.
    #[cfg(test)]
    pub(crate) fn use_ascii_glyphs(&mut self) {
        self.look.glyphs = &crate::glyphs::Glyphs::ASCII;
        self.look_changed();
    }

    /// Whether the full screen's window follows the end, for tests.
    #[cfg(test)]
    pub(crate) fn screen_follows(&self) -> bool {
        self.screen
            .as_ref()
            .is_some_and(|view| view.scroll.following())
    }

    /// The search's query, for tests.
    #[cfg(test)]
    pub(crate) fn search_query(&self) -> Option<String> {
        self.screen
            .as_ref()
            .and_then(|view| view.search.as_ref())
            .map(|search| search.query.clone())
    }

    /// A key while the transcript is read from under a dialog (since step
    /// 25): the full screen's scrolling keys and the arrows move it, the
    /// dialog's transcript key or `Esc` goes back to the dialog, whose quiet
    /// starts again, and nothing else does anything: no key pressed while
    /// reading reaches the dialog. The global keys keep their meaning.
    pub(super) fn reading_key(&mut self, key: Key, at: Instant) -> bool {
        let keymap = self.input.keymap();
        if let Some(action @ (Action::Interrupt | Action::Redraw)) = keymap.lookup(&[], key) {
            let outcome = if action == Action::Interrupt {
                self.input.interrupt_hidden()
            } else {
                self.input.act(action)
            };
            self.outcome(outcome, at);
            return true;
        }
        let back = keymap.lookup(&[Context::Dialog], key) == Some(Action::DialogTranscript)
            || (key.code == KeyCode::Esc && key.mods.is_empty());
        if back {
            self.reading_transcript = None;
            // Back at the question: the quiet of step 23 starts again, so no
            // key pressed while reading, nor one just after, answers it.
            self.not_taken(at);
            return true;
        }
        let action = keymap
            .lookup(&[Context::Screen], key)
            .filter(|action| action.context() == Context::Screen)
            .or(match (key.code, key.mods.is_empty()) {
                (KeyCode::Up, true) => Some(Action::ScreenLineUp),
                (KeyCode::Down, true) => Some(Action::ScreenLineDown),
                _ => None,
            });
        if let Some(action) = action {
            self.screen_act(action, None);
        }
        true
    }

    /// Whether the full screen's search is open.
    pub(super) fn screen_searching(&self) -> bool {
        self.screen
            .as_ref()
            .is_some_and(|view| view.search.is_some())
    }

    /// Takes the search worker's answer, when it has come: the loop's drain
    /// asks, so that an answer that comes while nothing else happens is
    /// drawn (the review of step 25). Says whether one came.
    pub(super) fn take_search_answer(&mut self) -> bool {
        let width = self.transcript_width();
        let Some(mut view) = self.screen.take() else {
            return false;
        };
        let came = self.take_found(&mut view, width);
        self.screen = Some(view);
        came
    }

    /// Waits at most `limit` for the search worker's answer to the latest
    /// query, and takes it: for tests, which cannot wait for the loop's
    /// wake.
    #[cfg(test)]
    pub(crate) fn settle_search(&mut self, limit: std::time::Duration) -> bool {
        let width = self.transcript_width();
        let Some(mut view) = self.screen.take() else {
            return false;
        };
        // A frame, or the loop's drain, took it already.
        if view
            .search
            .as_ref()
            .is_some_and(|search| search.found.is_some())
        {
            self.screen = Some(view);
            return true;
        }
        let found = view.worker.as_ref().and_then(|worker| worker.wait(limit));
        let settled = found.is_some();
        if let Some(found) = found {
            self.found(&mut view, found, width);
        }
        self.screen = Some(view);
        settled
    }

    /// Waits at most `limit` for the search worker's answer and leaves it
    /// to be taken: for tests of what takes it.
    #[cfg(test)]
    pub(crate) fn search_ready(&self, limit: std::time::Duration) -> bool {
        self.screen
            .as_ref()
            .and_then(|view| view.worker.as_ref())
            .is_some_and(|worker| worker.wait_ready(limit))
    }

    /// The worker's answer to the latest query: the newest match is the
    /// current one.
    fn found(&self, view: &mut ScreenView, found: crate::app::search::Found, width: usize) {
        let Some(search) = view.search.as_mut() else {
            return;
        };
        let total: usize = found.blocks.iter().map(|(_, count)| count).sum();
        search.found = Some(found.blocks);
        search.current = total.checked_sub(1);
        self.reveal_match(view, width);
    }

    /// The search's keys, the history search's context's: older (`Up`,
    /// `Enter`, `Ctrl+F` again), newer (`Down`), erase, and `Esc` to end.
    pub(super) fn search_key(&mut self, key: Key) {
        let width = self.transcript_width();
        let Some(mut view) = self.screen.take() else {
            return;
        };
        let action = self.input.keymap().lookup(&[Context::Search], key);
        let again =
            self.input.keymap().lookup(&[Context::Screen], key) == Some(Action::ScreenSearch);
        match action {
            _ if again => self.step_match(&mut view, width, false),
            Some(Action::SearchOlder | Action::SearchAccept) => {
                self.step_match(&mut view, width, false);
            }
            Some(Action::SearchNewer) => self.step_match(&mut view, width, true),
            Some(Action::SearchErase) => {
                if let Some(search) = view.search.as_mut() {
                    search.query.pop();
                }
                self.ask_search(&mut view);
            }
            Some(Action::SearchCancel | Action::Interrupt) => {
                // Ended on a match inside a box it opened: the box stays
                // open, as if clicked.
                if let Some(search) = view.search.take()
                    && let Some((round, _)) = search.opened
                {
                    view.open.insert(round, true);
                    if let Some(block) = self.kept.round_block(round) {
                        view.scroll.rows.forget(block);
                    }
                }
            }
            _ => {
                if let KeyCode::Char(c) = key.code
                    && !key.mods.contains(Mods::CTRL)
                    && !key.mods.contains(Mods::ALT)
                {
                    let c = crate::keys::typed(key).unwrap_or(c);
                    if let Some(search) = view
                        .search
                        .as_mut()
                        .filter(|search| search.query.chars().count() < QUERY_MOST)
                    {
                        search.query.push(c);
                    }
                    self.ask_search(&mut view);
                }
            }
        }
        self.screen = Some(view);
    }

    /// Moves to the next match, newer or older, going round.
    fn step_match(&self, view: &mut ScreenView, width: usize, newer: bool) {
        let Some(search) = view.search.as_mut() else {
            return;
        };
        let total: usize = search
            .found
            .as_ref()
            .map_or(0, |found| found.iter().map(|(_, count)| count).sum());
        if total == 0 {
            return;
        }
        search.current = Some(match (search.current, newer) {
            (Some(current), true) => (current + 1) % total,
            (Some(current), false) => (current + total - 1) % total,
            (None, _) => total - 1,
        });
        self.close_search_box(view);
        self.reveal_match(view, width);
    }

    /// Closes the box the search opened for the match before, as it was.
    fn close_search_box(&self, view: &mut ScreenView) {
        let Some((round, before)) = view.search.as_mut().and_then(|search| search.opened.take())
        else {
            return;
        };
        match before {
            Some(before) => view.open.insert(round, before),
            None => view.open.remove(&round),
        };
        if let Some(block) = self.kept.round_block(round) {
            view.scroll.rows.forget(block);
        }
    }

    /// Brings the current match into view: its block, the row it is drawn
    /// on, opening a closed box whose detail holds it.
    fn reveal_match(&self, view: &mut ScreenView, width: usize) {
        // Under a dialog the window stays where the request left it, at what
        // led to the dialog: the search moves it once it can be seen again
        // (the second review of step 25).
        if !self.dialogs.is_empty() {
            return;
        }
        let Some(search) = view.search.as_ref() else {
            return;
        };
        let (Some(found), Some(current)) = (search.found.as_ref(), search.current) else {
            return;
        };
        // The block, and which of its matches.
        let mut left = current;
        let mut place = None;
        for &(block, count) in found {
            if left < count {
                place = Some((block, left));
                break;
            }
            left -= count;
        }
        let Some((block, nth)) = place else {
            return;
        };
        let query = search.query.to_lowercase();
        let find_row = |rows: &[Line]| {
            let mut seen = 0;
            for (row, line) in rows.iter().enumerate() {
                let here = crate::app::search::count(&line.text().to_lowercase(), &query);
                if nth < seen + here {
                    return Some((row, nth - seen));
                }
                seen += here;
            }
            None
        };
        let live = self.screen_live(width);
        let mut rows = self.with_screen(view, width, &live, |scroll, source| {
            scroll.rows.of(block, width, source.make)
        });
        let mut at = find_row(&rows);
        // Not drawn: a closed box's detail holds it. Open the box while the
        // match is current.
        if at.is_none()
            && let Some(round) = self.kept.round_number(block)
        {
            let before = view.open.get(&round).copied();
            view.open.insert(round, true);
            view.scroll.rows.forget(block);
            if let Some(search) = view.search.as_mut() {
                search.opened = Some((round, before));
            }
            rows = self.with_screen(view, width, &live, |scroll, source| {
                scroll.rows.of(block, width, source.make)
            });
            at = find_row(&rows);
        }
        // A match the markdown's marks split as drawn: its block's start.
        let (row, in_row) = at.unwrap_or((0, usize::MAX));
        if let Some(search) = view.search.as_mut() {
            search.at = Some((block, row, in_row));
        }
        self.with_screen(view, width, &live, |scroll, source| {
            scroll.reveal(source, block, row, row + 1);
            // The last row of a window that does not follow the end is
            // covered by the row that says there is more below: a match
            // revealed there goes up one.
            let window = scroll.window(source, scroll.height.max(1));
            if !window.at_end && window.places.last() == Some(&Place::Kept { block, row }) {
                scroll.down(source, 1);
            }
        });
    }

    /// The search's row: its key, what is typed, and where it is; and the
    /// column the cursor goes to, after what is typed.
    fn search_row(&self, search: &Searching, width: usize, working: bool) -> (Line, usize) {
        let key = self
            .input
            .keymap()
            .keys_of(Action::ScreenSearch)
            .first()
            .map_or_else(|| "search".to_owned(), |key| crate::keys::name(*key));
        let query = cineris_term::sanitize(&search.query).into_owned();
        let indent = 2;
        let col = indent
            + crate::editor::cluster::width(&key)
            + 1
            + crate::editor::cluster::width(&query);
        let total: usize = search
            .found
            .as_ref()
            .map_or(0, |found| found.iter().map(|(_, count)| count).sum());
        let place = match (&search.found, search.current) {
            _ if search.query.is_empty() => String::new(),
            // Its thread could not be started: there is no search.
            (None, _) if !working => "the search could not start".to_owned(),
            (None, _) => "searching…".to_owned(),
            (Some(_), None) => "no match".to_owned(),
            (Some(_), Some(current)) => format!("{} of {total}", current + 1),
        };
        let separator = self.look.glyphs.separator;
        let hint = self.look.theme.style(Token::Hint);
        let line = Line::from(Span::plain(" ".repeat(indent)))
            .with(Span::new(key, self.look.theme.style(Token::Key)))
            .with(Span::plain(" "))
            .with(Span::plain(query))
            .with(Span::plain("   "))
            .with(Span::new(
                format!(
                    "{place}{}up older {separator} down newer {separator} esc ends",
                    if place.is_empty() {
                        String::new()
                    } else {
                        format!(" {separator} ")
                    }
                ),
                hint,
            ));
        (
            crate::layout::cut(&line, width, self.look.glyphs.ellipsis),
            col.min(width),
        )
    }

    /// Draws every match on the window's rows reversed, and the current
    /// one bold as well.
    fn mark_matches(&self, search: &Searching, window: &mut Window) {
        if search.query.is_empty() {
            return;
        }
        for (line, place) in window.rows.iter_mut().zip(&window.places) {
            let current = match (place, search.at) {
                (Place::Kept { block, row }, Some((at_block, at_row, nth)))
                    if *block == at_block && *row == at_row =>
                {
                    Some(nth)
                }
                _ => None,
            };
            *line = marked(line, &search.query, current);
        }
    }

    /// Follows the end again: a prompt was sent, or a dialog opened.
    pub(super) fn screen_follow(&mut self) {
        if let Some(view) = self.screen.as_mut() {
            view.scroll.follow();
        }
    }
}

/// `line` with every match of `query` (ignoring case) drawn reversed, and
/// the `current`-th of them bold too. A character whose lower case is not
/// one character is compared as it is.
fn marked(line: &Line, query: &str, current: Option<usize>) -> Line {
    let lower = |c: char| {
        let mut chars = c.to_lowercase();
        match (chars.next(), chars.next()) {
            (Some(one), None) => one,
            _ => c,
        }
    };
    let text: Vec<char> = line.text().chars().map(lower).collect();
    let wanted: Vec<char> = query.chars().map(lower).collect();
    if wanted.is_empty() || text.len() < wanted.len() {
        return line.clone();
    }
    // The matches, by characters, not overlapping.
    let mut ranges = Vec::new();
    let mut at = 0;
    while at + wanted.len() <= text.len() {
        if text[at..at + wanted.len()] == wanted[..] {
            ranges.push(at..at + wanted.len());
            at += wanted.len();
        } else {
            at += 1;
        }
    }
    if ranges.is_empty() {
        return line.clone();
    }
    let style_at = |index: usize, style: Style| -> Style {
        match ranges.iter().position(|range| range.contains(&index)) {
            Some(nth) if Some(nth) == current => style.reverse().bold(),
            Some(_) => style.reverse(),
            None => style,
        }
    };
    let mut out = Line::new();
    let mut index = 0;
    for span in line.spans() {
        let mut piece = String::new();
        let mut piece_style = None;
        for c in span.text().chars() {
            let style = style_at(index, span.style());
            if piece_style.is_some_and(|before| before != style) {
                let mut part = Span::new(std::mem::take(&mut piece), piece_style.unwrap_or(style));
                if let Some(url) = span.url() {
                    part = part.link(url);
                }
                out.push(part);
            }
            piece_style = Some(style);
            piece.push(c);
            index += 1;
        }
        if let Some(style) = piece_style {
            let mut part = Span::new(piece, style);
            if let Some(url) = span.url() {
                part = part.link(url);
            }
            out.push(part);
        }
    }
    out
}
