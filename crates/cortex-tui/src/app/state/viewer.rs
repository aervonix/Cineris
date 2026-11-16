//! The viewer (`docs/architecture/09-tui.md`, "The viewer"): the session on
//! the alternate screen, its tool boxes closed and the latest selected.
//! `Up` and `Down` move between boxes, `Enter` opens or closes the selected
//! one with all its detail, `PageUp` and `PageDown` scroll, `/` searches,
//! and `Esc` returns to the inline screen as it was.
//!
//! The transcript it shows is what was committed (each finished round kept
//! as a round, drawn again here as a box that can be selected and opened)
//! and what is still live, drawn as it is.

use std::collections::BTreeSet;
use std::time::Instant;

use cortex_base::{UtcTime, Zone};
use cortex_protocol::jsonrpc::RequestId;
use cortex_term::{Key, KeyCode, Line, Mods};

use super::App;
use crate::keys::Action;
use crate::transcript::Committed;
use crate::view::viewer::{self, ViewerKeys, ViewerTitle};
use crate::view::{self as views, Placement};

/// The rows the frame takes beside the transcript: the title and its rule,
/// the rule and the keys.
const FRAME_ROWS: usize = 4;

/// The viewer while it is open.
#[derive(Debug, Clone, Default)]
pub(super) struct Viewer {
    /// The first row of the transcript shown.
    top: usize,
    /// The selected box, by its place among the boxes.
    selected: Option<usize>,
    /// The boxes opened.
    open: BTreeSet<usize>,
    /// Whether the next frame brings the selected box into view.
    follow: bool,
    /// The search: what is typed after `/`, while it is typed.
    typing: Option<String>,
    /// The last search, which `n` finds again, and whether it found
    /// nothing.
    search: Option<(String, bool)>,
    /// How many rows the body had at the last frame, for the page keys.
    body: usize,
    /// Where the last frame's search should look from, and what it found.
    find: Option<usize>,
    /// It shows this request whole (`Ctrl+O` in its dialog), not the
    /// transcript, while its dialog is the one open.
    whole: Option<RequestId>,
}

/// The rows of the transcript in the viewer, and where each box's rows are.
struct Drawn {
    rows: Vec<Line>,
    /// Each box's first row and its end.
    boxes: Vec<(usize, usize)>,
}

impl App {
    /// Opens the viewer at the latest tool box, or at the box `n`, counted
    /// from one, opened (`/view n`).
    pub(super) fn open_viewer(&mut self, n: Option<usize>) {
        let boxes = self.boxes_kept();
        let selected = match n {
            Some(n) if (1..=boxes).contains(&n) => Some(n - 1),
            _ => boxes.checked_sub(1),
        };
        let mut open = BTreeSet::new();
        if let (Some(n), Some(selected)) = (n, selected)
            && n == selected + 1
        {
            open.insert(selected);
        }
        if self.zone.is_none() {
            // One small file, read once, when the viewer first opens.
            self.zone = Some(Zone::system());
        }
        self.viewer = Some(Viewer {
            selected,
            open,
            follow: true,
            ..Viewer::default()
        });
    }

    /// Opens the viewer on the open dialog's request, drawn whole: `Esc`
    /// returns to the dialog, still waiting for its answer.
    pub(super) fn open_whole(&mut self) {
        if self.zone.is_none() {
            self.zone = Some(Zone::system());
        }
        self.viewer = Some(Viewer {
            whole: self.dialogs.front().map(|dialog| dialog.id.clone()),
            ..Viewer::default()
        });
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

    /// How many boxes the committed transcript holds.
    fn boxes_kept(&self) -> usize {
        self.kept
            .iter()
            .filter(|part| matches!(part, Committed::Round(..)))
            .count()
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
        let drawn = self.viewer_rows(&state, width);
        let body = rows.saturating_sub(FRAME_ROWS).max(1);
        state.body = body;
        if let Some(from) = state.find.take() {
            // A search looks from the row after `from`, and goes round.
            let query = state
                .search
                .as_ref()
                .map(|(query, _)| query.to_lowercase())
                .unwrap_or_default();
            let count = drawn.rows.len();
            let found = (1..=count)
                .map(|step| (from + step) % count.max(1))
                .find(|&row| text_of(&drawn.rows[row]).to_lowercase().contains(&query));
            if let Some((_, missed)) = state.search.as_mut() {
                *missed = found.is_none();
            }
            if let Some(row) = found {
                state.top = row;
                state.follow = false;
            }
        }
        if state.follow
            && let Some((start, end)) = state
                .selected
                .and_then(|box_| drawn.boxes.get(box_).copied())
        {
            // The selected box in view: its top, if it is taller than the
            // body.
            if start < state.top || end - start > body {
                state.top = start;
            } else if end > state.top + body {
                state.top = end - body;
            }
            state.follow = false;
        }
        state.top = state.top.min(drawn.rows.len().saturating_sub(body));

        let mut lines = viewer::title(&self.viewer_title(), width, &self.look);
        let end = (state.top + body).min(drawn.rows.len());
        lines.extend(drawn.rows[state.top..end].iter().cloned());
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
            _ => match state.selected {
                Some(selected) => ViewerKeys::Boxes {
                    selected: selected + 1,
                    count: drawn.boxes.len(),
                    open: state.open.contains(&selected),
                },
                None => ViewerKeys::Lines,
            },
        };
        lines.extend(viewer::keys(&keys, width, &self.look));
        self.viewer = Some(state);
        lines
    }

    /// The transcript as the viewer draws it.
    fn viewer_rows(&self, state: &Viewer, width: usize) -> Drawn {
        let mut rows = Vec::new();
        let mut boxes = Vec::new();
        if let Some(id) = &state.whole {
            if let Some(dialog) = self.dialogs.front().filter(|dialog| &dialog.id == id) {
                rows = dialog.whole(width, &self.look);
            }
            return Drawn { rows, boxes };
        }
        for part in &self.kept {
            match part {
                Committed::Lines(lines) => rows.extend(lines.iter().cloned()),
                Committed::Round(round, _) => {
                    let place = boxes.len();
                    let start = rows.len();
                    rows.extend(views::round(
                        round,
                        Placement::Viewer {
                            selected: state.selected == Some(place),
                            open: state.open.contains(&place),
                        },
                        0,
                        width,
                        &self.look,
                    ));
                    boxes.push((start, rows.len()));
                }
            }
        }
        rows.extend(self.transcript.live(width, &self.look, self.tick));
        Drawn { rows, boxes }
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

    /// A key while the viewer is open. Global keys keep their meaning.
    pub(super) fn viewer_key(&mut self, key: Key, at: Instant) -> bool {
        if let Some(action @ (Action::Interrupt | Action::Redraw)) =
            self.input.keymap().lookup(&[], key)
        {
            let outcome = self.input.act(action);
            self.outcome(outcome, at);
            return true;
        }
        let boxes = self.boxes_kept();
        let Some(state) = self.viewer.as_mut() else {
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
                        state.find = Some(state.top);
                    }
                }
                KeyCode::Backspace => {
                    query.pop();
                }
                KeyCode::Char(c) if !key.mods.contains(Mods::CTRL) => query.push(c),
                _ => {}
            }
            return true;
        }
        let page = state.body.max(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.viewer = None,
            KeyCode::Char('o') if key.mods.contains(Mods::CTRL) => self.viewer = None,
            KeyCode::Up | KeyCode::Char('k') => match state.selected {
                Some(selected) => {
                    state.selected = Some(selected.saturating_sub(1));
                    state.follow = true;
                }
                None => state.top = state.top.saturating_sub(1),
            },
            KeyCode::Down | KeyCode::Char('j') => match state.selected {
                Some(selected) => {
                    state.selected = Some((selected + 1).min(boxes.saturating_sub(1)));
                    state.follow = true;
                }
                None => state.top += 1,
            },
            KeyCode::Enter => {
                if let Some(selected) = state.selected {
                    if !state.open.remove(&selected) {
                        state.open.insert(selected);
                    }
                    state.follow = true;
                }
            }
            KeyCode::PageUp => state.top = state.top.saturating_sub(page),
            KeyCode::PageDown => state.top += page,
            KeyCode::Home | KeyCode::Char('g') => state.top = 0,
            KeyCode::End | KeyCode::Char('G') => state.top = usize::MAX,
            KeyCode::Char('/') => {
                state.typing = Some(String::new());
                state.search = None;
            }
            KeyCode::Char('n') if state.search.is_some() => state.find = Some(state.top),
            _ => {}
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
