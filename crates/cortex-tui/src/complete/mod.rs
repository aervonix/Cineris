//! Completion menus and pickers (`docs/architecture/09-tui.md`, "Dialogs and
//! menus", "Files", "Commands").
//!
//! - **`@` files.** Typing `@` at the start of the prompt or after white
//!   space opens file completion over the workspace's paths, which the
//!   caller lists with the ignore rules and hands over as [`Candidates`].
//!   Accepting puts `@path` in place of what was typed.
//! - **`/` commands.** `/` at the start of the prompt opens the command
//!   menu, over the [`Command`]s the caller hands over.
//! - **Pickers** (`/model`, `/effort`, sessions) are menus whose rows the
//!   caller makes ([`Menu::picker`]). The `/effort` picker has a toggle row
//!   for fast mode.
//!
//! A menu shows at most [`ROWS`] rows, filtered as the user types, and
//! scrolls to keep the selected row in view.

pub mod fuzzy;

use std::ops::Range;

pub use fuzzy::{Candidates, Filter, Hit};

/// The most rows a menu shows.
pub const ROWS: usize = 8;

/// The most matches kept for a menu to scroll through.
pub const KEPT: usize = 64;

/// A slash command, for the command menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// Its name, without the `/`.
    pub name: String,
    /// What it does, in a few words.
    pub description: String,
}

/// What the text at the cursor asks to complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// A file: `@` at byte `start`, and what follows it up to the cursor.
    File {
        /// Where the `@` is.
        start: usize,
        /// What was typed after it.
        query: String,
    },
    /// A command: `/` at the start of the prompt, and what follows it up to
    /// the cursor.
    Command {
        /// What was typed after the `/`.
        query: String,
    },
}

impl Trigger {
    /// The bytes an accepted row replaces.
    #[must_use]
    pub fn range(&self, cursor: usize) -> Range<usize> {
        match self {
            Self::File { start, .. } => *start..cursor,
            Self::Command { .. } => 0..cursor,
        }
    }
}

/// What, if anything, the text before `cursor` asks to complete.
#[must_use]
pub fn trigger(text: &str, cursor: usize) -> Option<Trigger> {
    let before = &text[..cursor];
    // The word the cursor ends: back to white space or the start.
    let word_start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(at, c)| at + c.len_utf8());
    let word = &before[word_start..];
    if let Some(query) = word.strip_prefix('/')
        && word_start == 0
    {
        return Some(Trigger::Command {
            query: query.to_owned(),
        });
    }
    if let Some(query) = word.strip_prefix('@') {
        return Some(Trigger::File {
            start: word_start,
            query: query.to_owned(),
        });
    }
    None
}

/// What accepting a file row puts in the prompt: `@path`, with the path in
/// quotes when it holds white space, and a space after.
#[must_use]
pub fn file_token(path: &str) -> String {
    if path.chars().any(char::is_whitespace) {
        format!("@\"{path}\" ")
    } else {
        format!("@{path} ")
    }
}

/// One row of a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// What the row is: a path, a command's name, a model.
    pub label: String,
    /// A dim note after it.
    pub detail: String,
    /// The bytes of `label` the filter matched, for drawing in bold.
    pub matched: Vec<Range<usize>>,
}

impl Row {
    /// A row with a label and a note.
    #[must_use]
    pub fn new(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            matched: Vec::new(),
        }
    }
}

/// A picker's toggle row: `tab - fast mode  on`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toggle {
    /// What it turns on and off: `fast mode`.
    pub label: String,
    /// Whether it is on.
    pub on: bool,
    /// A dim note after its state.
    pub note: String,
}

/// What a menu is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// File completion for the `@` at this byte.
    Files {
        /// Where the `@` is.
        start: usize,
    },
    /// The command menu.
    Commands,
    /// A picker the caller opened, by the name it gave it.
    Picker(String),
}

/// What a menu shows when it has no row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Empty {
    /// Nothing matches what was typed.
    NoMatch,
    /// The caller has not handed over the workspace's paths yet.
    Listing,
}

/// A menu: its rows, which is selected, and which are in view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    /// What it is for.
    pub kind: Kind,
    /// The rows that match, best first.
    pub rows: Vec<Row>,
    /// What to say when there is no row.
    pub empty: Empty,
    /// A toggle row under the rows, in a picker that has one.
    pub toggle: Option<Toggle>,
    /// What is typed into a picker to filter it.
    pub filter: String,
    /// The row the caller marked as current (`current` in `/model`).
    pub current: Option<usize>,
    selected: usize,
    top: usize,
    /// A picker's rows before filtering, and which of them each shown row
    /// is.
    all: Vec<Row>,
    shown: Vec<usize>,
}

impl Menu {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            rows: Vec::new(),
            empty: Empty::NoMatch,
            toggle: None,
            filter: String::new(),
            current: None,
            selected: 0,
            top: 0,
            all: Vec::new(),
            shown: Vec::new(),
        }
    }

    /// A completion menu with these rows, the first selected.
    #[must_use]
    pub fn completion(kind: Kind, rows: Vec<Row>, empty: Empty) -> Self {
        let mut menu = Self::new(kind);
        menu.rows = rows;
        menu.empty = empty;
        menu
    }

    /// A picker named `name` over `rows`, with `selected` selected, and a
    /// toggle row if it has one.
    #[must_use]
    pub fn picker(
        name: impl Into<String>,
        rows: Vec<Row>,
        selected: usize,
        toggle: Option<Toggle>,
    ) -> Self {
        let mut menu = Self::new(Kind::Picker(name.into()));
        menu.shown = (0..rows.len()).collect();
        menu.all = rows.clone();
        menu.rows = rows;
        menu.toggle = toggle;
        menu.current = Some(selected);
        menu.select(selected);
        menu
    }

    /// The selected row's index in [`rows`](Self::rows), if there is one.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        (self.selected < self.rows.len()).then_some(self.selected)
    }

    /// In a picker, the index of the selected row among the rows it was
    /// opened with.
    #[must_use]
    pub fn picked(&self) -> Option<usize> {
        self.selected().and_then(|at| self.shown.get(at).copied())
    }

    /// The rows in view: their indices.
    #[must_use]
    pub fn visible(&self) -> Range<usize> {
        self.top..(self.top + ROWS).min(self.rows.len())
    }

    /// Selects row `at`, scrolling it into view.
    pub fn select(&mut self, at: usize) {
        self.selected = at.min(self.rows.len().saturating_sub(1));
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + ROWS {
            self.top = self.selected + 1 - ROWS;
        }
    }

    /// The row above; from the first, the last.
    pub fn up(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let at = self.selected.checked_sub(1).unwrap_or(self.rows.len() - 1);
        self.select(at);
    }

    /// The row below; from the last, the first.
    pub fn down(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let at = if self.selected + 1 >= self.rows.len() {
            0
        } else {
            self.selected + 1
        };
        self.select(at);
    }

    /// New rows for a completion menu, as the filter changed: the best
    /// match is selected.
    pub fn refill(&mut self, rows: Vec<Row>, empty: Empty) {
        self.rows = rows;
        self.empty = empty;
        self.top = 0;
        self.select(0);
    }

    /// A picker's rows filtered by what is typed into it.
    pub(crate) fn refilter(&mut self) {
        let mut filter = Filter::new();
        let candidates = Candidates::new(self.all.iter().map(|row| row.label.clone()));
        let hits = filter.run(&candidates, &self.filter, self.all.len());
        let mut hits: Vec<usize> = hits.into_iter().map(|hit| hit.index).collect();
        if self.filter.is_empty() {
            hits.sort_unstable();
        }
        self.rows = hits
            .iter()
            .map(|&index| {
                let mut row = self.all[index].clone();
                row.matched = fuzzy::positions(&row.label, &self.filter);
                row
            })
            .collect();
        self.shown = hits;
        self.top = 0;
        let current = self
            .current
            .and_then(|c| self.shown.iter().position(|&s| s == c));
        self.select(current.filter(|_| self.filter.is_empty()).unwrap_or(0));
    }
}

/// The rows for `query` over `candidates`: a file menu's, or a command
/// menu's with each command's description.
pub(crate) fn rows(
    filter: &mut Filter,
    candidates: &Candidates,
    query: &str,
    detail: impl Fn(usize) -> String,
) -> Vec<Row> {
    filter
        .run(candidates, query, KEPT)
        .into_iter()
        .filter_map(|hit| {
            let label = candidates.get(hit.index)?.to_owned();
            let matched = fuzzy::positions(&label, query);
            Some(Row {
                detail: detail(hit.index),
                label,
                matched,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_after_white_space_or_at_the_start_opens_files() {
        assert_eq!(
            trigger("read @src/le", 12),
            Some(Trigger::File {
                start: 5,
                query: "src/le".into()
            })
        );
        assert_eq!(
            trigger("@", 1),
            Some(Trigger::File {
                start: 0,
                query: String::new()
            })
        );
        assert_eq!(trigger("mail me@home", 12), None);
        assert_eq!(trigger("read @src now", 13), None);
    }

    #[test]
    fn slash_only_at_the_start_opens_commands() {
        assert_eq!(
            trigger("/mo", 3),
            Some(Trigger::Command { query: "mo".into() })
        );
        assert_eq!(trigger("use a/b", 7), None);
        assert_eq!(trigger("/model x", 8), None);
    }

    #[test]
    fn a_menu_scrolls_to_keep_the_selection_in_view_and_wraps() {
        let rows: Vec<Row> = (0..12).map(|n| Row::new(format!("r{n}"), "")).collect();
        let mut menu = Menu::completion(Kind::Commands, rows, Empty::NoMatch);
        assert_eq!(menu.visible(), 0..8);
        for _ in 0..9 {
            menu.down();
        }
        assert_eq!(menu.selected(), Some(9));
        assert_eq!(menu.visible(), 2..10);
        menu.up();
        menu.up();
        menu.up();
        assert_eq!(menu.visible(), 2..10);
        menu.select(0);
        menu.up();
        assert_eq!(menu.selected(), Some(11));
        assert_eq!(menu.visible(), 4..12);
    }

    #[test]
    fn a_picker_filters_and_remembers_which_row_is_which() {
        let rows = ["low", "medium", "high", "max"].map(|l| Row::new(l, ""));
        let mut menu = Menu::picker("effort", rows.to_vec(), 2, None);
        assert_eq!(menu.picked(), Some(2));
        menu.filter.push('m');
        menu.refilter();
        let labels: Vec<&str> = menu.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels.len(), 2);
        menu.select(labels.iter().position(|l| *l == "max").unwrap());
        assert_eq!(menu.picked(), Some(3));
        menu.filter.clear();
        menu.refilter();
        assert_eq!(menu.picked(), Some(2), "back on the current row");
    }
}
