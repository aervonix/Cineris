//! Earlier prompts (`docs/architecture/09-tui.md`, "History").
//!
//! The interface never reads or writes a file. Its caller hands it a
//! [`HistoryStore`] holding the project's earlier prompts, and is told of
//! each prompt sent ([`HistoryStore::push`]), to keep wherever it keeps
//! them. [`MemoryHistory`] is a store in memory, for tests and for a
//! session with nowhere to keep history.
//!
//! - **A prompt is kept as it was written** (since the follow-ups of step
//!   23): a [`Fragment`], each paste that was a chip still a chip, so that
//!   recalling it gives back the chip and not its text spread out in the
//!   editor.
//! - **What is kept is bounded** ([`kept`]): a chip keeps at most
//!   [`CHIP_KEEP`] bytes of its text, and says so in its label when it was
//!   cut; a prompt's chips together keep at most [`CHIP_TEXT_KEEP`]; and the
//!   prompt's own text, outside its chips, at most [`TEXT_KEEP`], with a
//!   line saying how much was left out. A pasted log no longer makes the
//!   history's file hundreds of megabytes.
//! - **Stepping** ([`Steps`]): `Up` on the editor's first row goes to the
//!   prompt before, `Down` on its last row to the one after, and past the
//!   newest back to what was being typed. An entry edited and stepped away
//!   from keeps its edit until the prompt is sent.
//! - **Searching** ([`Search`]): `Ctrl+R` finds the newest prompt that
//!   holds the search, ignoring case, and again finds the one before it. It
//!   looks in what the prompt shows: its text, and each chip's label.

use std::collections::HashMap;
use std::collections::VecDeque;

use crate::editor::{Chip, Fragment};

/// The most bytes of a chip's text a prompt in history keeps. A chip whose
/// text is longer keeps its first `CHIP_KEEP` bytes (cut at a character's
/// edge), and its label says so.
pub const CHIP_KEEP: usize = 64 * 1024;

/// The most bytes of chip text one prompt in history keeps, its chips
/// together: a chip past it keeps its label and none of its text.
pub const CHIP_TEXT_KEEP: usize = 4 * CHIP_KEEP;

/// The most bytes of a prompt's own text, outside its chips, that history
/// keeps: text written in the external editor can be any length. What is
/// past it is left out, and a line at the end says how much.
pub const TEXT_KEEP: usize = 64 * 1024;

/// The room the line that says how much was left out takes past
/// [`TEXT_KEEP`]: a text within both is not cut, so that a prompt kept once
/// is kept again as it is.
const SAID_ROOM: usize = 128;

/// Where earlier prompts are kept. Index 0 is the oldest.
pub trait HistoryStore: std::fmt::Debug {
    /// How many prompts it holds.
    fn len(&self) -> usize;

    /// Whether it holds none.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The prompt at `index`, 0 the oldest, as it was written: its chips
    /// are chips.
    fn get(&self, index: usize) -> Option<&Fragment>;

    /// Adds a prompt that was sent, as the newest. The input hands it over
    /// as [`kept`] makes it.
    fn push(&mut self, prompt: &Fragment);
}

/// A prompt as history keeps it: within [`CHIP_KEEP`] for each chip,
/// [`CHIP_TEXT_KEEP`] for its chips together, and [`TEXT_KEEP`] for its own
/// text. A chip that was cut says so in its label (`[pasted 2,345 lines,
/// first 64 KiB kept]`), so that one recalled is not taken for the whole
/// paste; own text that was cut ends with a line saying how many
/// characters were left out. What was within the bounds is kept as it is.
#[must_use]
pub fn kept(prompt: &Fragment) -> Fragment {
    let whole = prompt.as_str();
    let (text, dropped) = match cut_at(whole, TEXT_KEEP + SAID_ROOM) {
        Some(_) => {
            let cut = cut_at(whole, TEXT_KEEP).unwrap_or(whole);
            (cut, whole[cut.len()..].chars().count())
        }
        None => (whole, 0),
    };
    // A chip whose mark was cut off goes with it.
    let marks = crate::editor::buffer::marks(text);
    let mut budget = CHIP_TEXT_KEEP;
    let chips = prompt.chips()[..marks]
        .iter()
        .map(|chip| {
            let room = CHIP_KEEP.min(budget);
            let content = chip.content();
            match cut_at(content, room) {
                None => {
                    budget -= content.len();
                    chip.clone()
                }
                Some(cut) => {
                    budget -= cut.len();
                    Chip::new(cut, cut_label(chip.label(), cut.is_empty()))
                }
            }
        })
        .collect();
    let mut text = text.to_owned();
    if dropped > 0 {
        text.push_str(&format!(
            "\n[{} more characters were not kept in history]",
            crate::paste::thousands(dropped)
        ));
    }
    Fragment::with_chips(text, chips).unwrap_or_default()
}

/// `text` cut to at most `limit` bytes at a character's edge, or `None`
/// when it fits.
fn cut_at(text: &str, limit: usize) -> Option<&str> {
    if text.len() <= limit {
        return None;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(&text[..end])
}

/// A chip's label once its text was cut: `[pasted 2,345 lines]` becomes
/// `[pasted 2,345 lines, first 64 KiB kept]`.
fn cut_label(label: &str, nothing: bool) -> String {
    let said = if nothing {
        "its text not kept".to_owned()
    } else {
        format!("first {} KiB kept", CHIP_KEEP / 1024)
    };
    match label.strip_suffix(']') {
        Some(open) => format!("{open}, {said}]"),
        None => format!("{label} [{said}]"),
    }
}

/// A [`HistoryStore`] in memory, holding at most a number of prompts, and a
/// prompt the same as the newest only once.
#[derive(Debug, Clone)]
pub struct MemoryHistory {
    entries: VecDeque<Fragment>,
    limit: usize,
}

impl MemoryHistory {
    /// The most prompts a store keeps unless told otherwise.
    pub const LIMIT: usize = 1_000;

    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limit(Self::LIMIT)
    }

    /// An empty store that keeps at most `limit` prompts, dropping the
    /// oldest.
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            limit: limit.max(1),
        }
    }

    /// A store holding `prompts`, oldest first, each plain text.
    #[must_use]
    pub fn from_prompts(prompts: impl IntoIterator<Item = String>) -> Self {
        let mut store = Self::new();
        for prompt in prompts {
            store.push(&Fragment::text(prompt));
        }
        store
    }
}

impl Default for MemoryHistory {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a prompt is nothing to keep: no chip, and its text blank.
#[must_use]
pub fn is_blank(prompt: &Fragment) -> bool {
    prompt.chips().is_empty() && prompt.as_str().trim().is_empty()
}

impl HistoryStore for MemoryHistory {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn get(&self, index: usize) -> Option<&Fragment> {
        self.entries.get(index)
    }

    fn push(&mut self, prompt: &Fragment) {
        if is_blank(prompt) || self.entries.back().is_some_and(|last| last == prompt) {
            return;
        }
        if self.entries.len() == self.limit {
            self.entries.pop_front();
        }
        self.entries.push_back(prompt.clone());
    }
}

/// Stepping through history from the editor.
#[derive(Debug, Default)]
pub struct Steps {
    /// The entry shown, if the editor is showing one.
    at: Option<usize>,
    /// What was being typed before the first step.
    draft: Option<Fragment>,
    /// Entries edited and stepped away from.
    edited: HashMap<usize, Fragment>,
}

impl Steps {
    /// The entry the editor is showing, if any.
    #[must_use]
    pub fn at(&self) -> Option<usize> {
        self.at
    }

    /// One step older, from `current` (what the editor holds now): the
    /// fragment to show, or `None` at the oldest.
    pub fn older(&mut self, store: &dyn HistoryStore, current: Fragment) -> Option<Fragment> {
        let target = match self.at {
            Some(0) => return None,
            Some(at) => at - 1,
            None => store.len().checked_sub(1)?,
        };
        self.leave(current);
        self.at = Some(target);
        Some(self.shown(store, target))
    }

    /// One step newer: the fragment to show, or `None` when the editor is
    /// not showing an entry. Past the newest, what was being typed.
    pub fn newer(&mut self, store: &dyn HistoryStore, current: Fragment) -> Option<Fragment> {
        let at = self.at?;
        self.leave(current);
        if at + 1 < store.len() {
            self.at = Some(at + 1);
            Some(self.shown(store, at + 1))
        } else {
            self.at = None;
            Some(self.draft.take().unwrap_or_default())
        }
    }

    /// Forgets the steps: the prompt was sent.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn leave(&mut self, current: Fragment) {
        match self.at {
            Some(at) => {
                self.edited.insert(at, current);
            }
            None => self.draft = Some(current),
        }
    }

    fn shown(&mut self, store: &dyn HistoryStore, at: usize) -> Fragment {
        self.edited
            .remove(&at)
            .unwrap_or_else(|| store.get(at).cloned().unwrap_or_default())
    }
}

/// A search through history.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Search {
    /// What is searched for.
    query: String,
    /// The entry found, if any.
    found: Option<usize>,
}

impl Search {
    /// A new search, for nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What is searched for.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The entry found, if any.
    #[must_use]
    pub fn found(&self) -> Option<usize> {
        self.found
    }

    /// Adds `c` to the search, and finds again from the newest.
    pub fn push(&mut self, store: &dyn HistoryStore, c: char) {
        self.query.push(c);
        self.find(store, store.len(), true);
    }

    /// Takes the last character off the search, and finds again from the
    /// newest.
    pub fn pop(&mut self, store: &dyn HistoryStore) {
        self.query.pop();
        self.find(store, store.len(), true);
    }

    /// The next older entry that matches. Stays on the match it had when
    /// there is none older.
    pub fn older(&mut self, store: &dyn HistoryStore) {
        let from = self.found.unwrap_or(store.len());
        self.find(store, from, false);
    }

    /// The next newer entry that matches.
    pub fn newer(&mut self, store: &dyn HistoryStore) {
        let Some(from) = self.found else {
            return;
        };
        let query = self.query.to_lowercase();
        if let Some(found) =
            (from + 1..store.len()).find(|&index| matches(store.get(index), &query))
        {
            self.found = Some(found);
        }
    }

    /// Looks back from just before `before`. `fresh` drops the old match
    /// when nothing is found.
    fn find(&mut self, store: &dyn HistoryStore, before: usize, fresh: bool) {
        let query = self.query.to_lowercase();
        let found = (0..before.min(store.len()))
            .rev()
            .find(|&index| matches(store.get(index), &query));
        if found.is_some() || fresh {
            self.found = found;
        }
    }
}

/// Whether an entry holds the query, in what it shows: its own text and
/// its chips' labels, not their text, which can be 64 KiB each.
fn matches(entry: Option<&Fragment>, query: &str) -> bool {
    entry.is_some_and(|entry| query.is_empty() || entry.shown().to_lowercase().contains(query))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> MemoryHistory {
        MemoryHistory::from_prompts(
            ["fix the lexer", "run clippy", "fix the parser"].map(String::from),
        )
    }

    #[test]
    fn stepping_back_and_forth_keeps_the_draft_and_edits() {
        let store = store();
        let mut steps = Steps::default();
        let shown = steps.older(&store, Fragment::text("draft")).unwrap();
        assert_eq!(shown.expanded(), "fix the parser");
        let shown = steps
            .older(&store, Fragment::text("fix the parser!"))
            .unwrap();
        assert_eq!(shown.expanded(), "run clippy");
        let shown = steps.newer(&store, Fragment::text("run clippy")).unwrap();
        assert_eq!(shown.expanded(), "fix the parser!");
        let shown = steps
            .newer(&store, Fragment::text("fix the parser!"))
            .unwrap();
        assert_eq!(shown.expanded(), "draft");
        assert_eq!(steps.newer(&store, Fragment::text("draft")), None);
    }

    #[test]
    fn the_oldest_entry_goes_no_further() {
        let store = store();
        let mut steps = Steps::default();
        for _ in 0..3 {
            assert!(steps.older(&store, Fragment::default()).is_some());
        }
        assert_eq!(steps.older(&store, Fragment::default()), None);
        assert_eq!(steps.at(), Some(0));
    }

    #[test]
    fn a_search_finds_newest_first_ignoring_case_and_can_find_nothing() {
        let store = store();
        let mut search = Search::new();
        for c in "FIX".chars() {
            search.push(&store, c);
        }
        assert_eq!(search.found(), Some(2));
        search.older(&store);
        assert_eq!(search.found(), Some(0));
        search.older(&store);
        assert_eq!(search.found(), Some(0), "stays on the oldest match");
        search.newer(&store);
        assert_eq!(search.found(), Some(2));
        search.push(&store, 'z');
        assert_eq!(search.found(), None);
        search.pop(&store);
        assert_eq!(search.found(), Some(2));
    }

    #[test]
    fn the_same_prompt_twice_is_kept_once() {
        let mut store = MemoryHistory::with_limit(2);
        store.push(&Fragment::text("a"));
        store.push(&Fragment::text("a"));
        store.push(&Fragment::text("  "));
        assert_eq!(store.len(), 1);
        store.push(&Fragment::text("b"));
        store.push(&Fragment::text("c"));
        let text = |index| store.get(index).map(Fragment::as_str);
        assert_eq!((text(0), text(1)), (Some("b"), Some("c")));
    }

    fn with_chips(text: &str, chips: &[(&str, &str)]) -> Fragment {
        Fragment::with_chips(
            text,
            chips
                .iter()
                .map(|(content, label)| Chip::new(*content, *label))
                .collect(),
        )
        .unwrap()
    }

    /// A prompt within the bounds is kept exactly; a chip stays a chip.
    #[test]
    fn a_prompt_within_the_bounds_is_kept_as_it_is() {
        let prompt = with_chips(
            "look at \u{FFFC} and \u{FFFC}",
            &[("a\nb\n", "[pasted 2 lines]"), ("\u{FFFC}", "\u{FFFC}")],
        );
        assert_eq!(kept(&prompt), prompt);
        let at_the_edge = with_chips("\u{FFFC}", &[(&"x".repeat(CHIP_KEEP), "[p]")]);
        assert_eq!(kept(&at_the_edge), at_the_edge);
    }

    /// A large paste is kept as its chip, its text cut to [`CHIP_KEEP`] at
    /// a character's edge, its label saying so; and recalled as a chip
    /// (the follow-ups of step 23: it was stored expanded, whole).
    #[test]
    fn a_large_paste_is_kept_as_its_chip_cut_to_the_cap() {
        let big = format!("x{}", "é".repeat(CHIP_KEEP));
        let prompt = with_chips("see \u{FFFC} please", &[(&big, "[pasted 3 lines]")]);
        let kept = kept(&prompt);
        assert_eq!(kept.as_str(), "see \u{FFFC} please");
        let chip = &kept.chips()[0];
        assert!(chip.content().len() <= CHIP_KEEP);
        assert!(chip.content().len() > CHIP_KEEP - 2, "cut at the cap");
        assert!(big.starts_with(chip.content()));
        assert_eq!(chip.label(), "[pasted 3 lines, first 64 KiB kept]");

        let mut store = MemoryHistory::new();
        store.push(&kept);
        let mut steps = Steps::default();
        let recalled = steps.older(&store, Fragment::default()).unwrap();
        assert_eq!(recalled.chips().len(), 1, "recalled as a chip");
        assert_eq!(recalled.shown(), "see [pasted 3 lines, first 64 KiB kept] please");
    }

    /// The chips of one prompt keep [`CHIP_TEXT_KEEP`] together: one past
    /// it keeps its label alone.
    #[test]
    fn the_chips_of_one_prompt_share_a_cap() {
        let big = "y".repeat(CHIP_KEEP * 2);
        let count = CHIP_TEXT_KEEP / CHIP_KEEP + 1;
        let chips: Vec<(&str, &str)> = (0..count).map(|_| (big.as_str(), "[pasted]")).collect();
        let prompt = with_chips(&"\u{FFFC}".repeat(count), &chips);
        let kept = kept(&prompt);
        let total: usize = kept.chips().iter().map(|chip| chip.content().len()).sum();
        assert_eq!(total, CHIP_TEXT_KEEP);
        let last = kept.chips().last().unwrap();
        assert_eq!(last.content(), "");
        assert_eq!(last.label(), "[pasted, its text not kept]");
    }

    /// The prompt's own text is cut at [`TEXT_KEEP`] with a line saying how
    /// much was left out, and a chip whose mark was cut off goes with it.
    #[test]
    fn a_long_text_is_cut_with_a_line_saying_how_much() {
        let text = format!("{}\u{FFFC}{}", "a".repeat(TEXT_KEEP), "b".repeat(1_500));
        let prompt = with_chips(&text, &[("c", "[c]")]);
        let kept = kept(&prompt);
        assert!(kept.chips().is_empty());
        assert_eq!(
            kept.as_str(),
            format!(
                "{}\n[1,501 more characters were not kept in history]",
                "a".repeat(TEXT_KEEP)
            )
        );
        // Kept again, it is as it was: a file read back, or a recalled
        // prompt sent again, loses nothing more.
        assert_eq!(super::kept(&kept), kept);
        let chip = with_chips("\u{FFFC}", &[(&"z".repeat(CHIP_KEEP * 2), "[pasted 9 lines]")]);
        let once = super::kept(&chip);
        assert_eq!(super::kept(&once), once);
    }

    /// A search looks in the chips' labels, not their text.
    #[test]
    fn a_search_looks_in_labels_not_in_a_chips_text() {
        let mut store = MemoryHistory::new();
        store.push(&with_chips("fix \u{FFFC}", &[("needle", "[pasted 20 lines]")]));
        let mut search = Search::new();
        for c in "needle".chars() {
            search.push(&store, c);
        }
        assert_eq!(search.found(), None);
        let mut search = Search::new();
        for c in "PASTED 20".chars() {
            search.push(&store, c);
        }
        assert_eq!(search.found(), Some(0));
    }
}
