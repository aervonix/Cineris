//! Earlier prompts (`docs/architecture/09-tui.md`, "History").
//!
//! The interface never reads or writes a file. Its caller hands it a
//! [`HistoryStore`] holding the project's earlier prompts, and is told of
//! each prompt sent ([`HistoryStore::push`]), to keep wherever it keeps
//! them. [`MemoryHistory`] is a store in memory, for tests and for a
//! session with nowhere to keep history.
//!
//! - **Stepping** ([`Steps`]): `Up` on the editor's first row goes to the
//!   prompt before, `Down` on its last row to the one after, and past the
//!   newest back to what was being typed. An entry edited and stepped away
//!   from keeps its edit until the prompt is sent.
//! - **Searching** ([`Search`]): `Ctrl+R` finds the newest prompt that
//!   holds the search, ignoring case, and again finds the one before it.

use std::collections::HashMap;
use std::collections::VecDeque;

use crate::editor::Fragment;

/// Where earlier prompts are kept. Index 0 is the oldest.
pub trait HistoryStore: std::fmt::Debug {
    /// How many prompts it holds.
    fn len(&self) -> usize;

    /// Whether it holds none.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The prompt at `index`, 0 the oldest.
    fn get(&self, index: usize) -> Option<&str>;

    /// Adds a prompt that was sent, as the newest.
    fn push(&mut self, prompt: &str);
}

/// A [`HistoryStore`] in memory, holding at most a number of prompts, and a
/// prompt the same as the newest only once.
#[derive(Debug, Clone)]
pub struct MemoryHistory {
    entries: VecDeque<String>,
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

    /// A store holding `prompts`, oldest first.
    #[must_use]
    pub fn from_prompts(prompts: impl IntoIterator<Item = String>) -> Self {
        let mut store = Self::new();
        for prompt in prompts {
            store.push(&prompt);
        }
        store
    }
}

impl Default for MemoryHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl HistoryStore for MemoryHistory {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn get(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(String::as_str)
    }

    fn push(&mut self, prompt: &str) {
        if prompt.trim().is_empty() || self.entries.back().is_some_and(|last| last == prompt) {
            return;
        }
        if self.entries.len() == self.limit {
            self.entries.pop_front();
        }
        self.entries.push_back(prompt.to_owned());
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
            .unwrap_or_else(|| Fragment::text(store.get(at).unwrap_or_default()))
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

fn matches(entry: Option<&str>, query: &str) -> bool {
    entry.is_some_and(|entry| query.is_empty() || entry.to_lowercase().contains(query))
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
        store.push("a");
        store.push("a");
        store.push("  ");
        assert_eq!(store.len(), 1);
        store.push("b");
        store.push("c");
        assert_eq!((store.get(0), store.get(1)), (Some("b"), Some("c")));
    }
}
