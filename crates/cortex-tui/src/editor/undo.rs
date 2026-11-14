//! Undo and redo.
//!
//! Each step of undo is a [`Group`]: the changes one action made, with the
//! cursor before and after. Typing is grouped into words: characters typed
//! one after another join the group they follow until a word begins after
//! something that was not a word, so `fix the bug` is undone as `bug`,
//! `the `, `fix `. Deleting a character at a time is grouped the same way.
//! Anything else (a paste, a yank, a completion, a kill, a line break, a
//! move of the cursor) ends the group, and is a group of its own.

use super::buffer::{Buffer, Fragment};

/// The most undo steps kept. The oldest go first.
const LIMIT: usize = 1_000;

/// One replacement, as made: at `at`, `removed` went out and `inserted`
/// came in.
#[derive(Debug, Clone)]
pub(crate) struct Change {
    pub(crate) at: usize,
    pub(crate) removed: Fragment,
    pub(crate) inserted: Fragment,
}

/// What an open group is still taking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Open {
    /// Typed characters, the last of them a word character or not.
    Typing { word: bool },
    /// Characters deleted backwards one at a time.
    Erasing { word: bool },
    /// Characters deleted forwards one at a time.
    Deleting { word: bool },
}

/// The changes of one step of undo.
#[derive(Debug, Clone)]
pub(crate) struct Group {
    pub(crate) changes: Vec<Change>,
    pub(crate) before: usize,
    pub(crate) after: usize,
}

#[derive(Debug, Default)]
pub(crate) struct History {
    done: Vec<Group>,
    undone: Vec<Group>,
    /// The group that typing or deleting may still add to.
    open: Option<Open>,
}

impl History {
    /// Ends the open group: the next change starts a new one.
    pub(crate) fn close(&mut self) {
        self.open = None;
    }

    pub(crate) fn open(&self) -> Option<Open> {
        self.open
    }

    /// Records `change`, made with the cursor at `before` and leaving it at
    /// `after`. With `join`, it joins the open group, which then takes
    /// `open`; otherwise it is a group of its own, open as `open` says.
    pub(crate) fn record(
        &mut self,
        change: Change,
        before: usize,
        after: usize,
        join: bool,
        open: Option<Open>,
    ) {
        self.undone.clear();
        match self.done.last_mut() {
            Some(group) if join && self.open.is_some() => {
                merge(&mut group.changes, change);
                group.after = after;
            }
            _ => {
                if self.done.len() == LIMIT {
                    self.done.remove(0);
                }
                self.done.push(Group {
                    changes: vec![change],
                    before,
                    after,
                });
            }
        }
        self.open = open;
    }

    /// Undoes the last group in `buffer`: the cursor it leaves, if there was
    /// one.
    pub(crate) fn undo(&mut self, buffer: &mut Buffer) -> Option<usize> {
        self.open = None;
        let group = self.done.pop()?;
        for change in group.changes.iter().rev() {
            let end = change.at + change.inserted.text.len();
            buffer.replace(change.at..end, change.removed.clone());
        }
        let cursor = group.before;
        self.undone.push(group);
        Some(cursor)
    }

    /// Makes the last undone group again.
    pub(crate) fn redo(&mut self, buffer: &mut Buffer) -> Option<usize> {
        self.open = None;
        let group = self.undone.pop()?;
        for change in &group.changes {
            let end = change.at + change.removed.text.len();
            buffer.replace(change.at..end, change.inserted.clone());
        }
        let cursor = group.after;
        self.done.push(group);
        Some(cursor)
    }

    /// Forgets everything.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Adds `change` to `changes`, into the last one when it continues it: text
/// typed at its end, or a character deleted just before or after it.
fn merge(changes: &mut Vec<Change>, change: Change) {
    if let Some(last) = changes.last_mut() {
        let last_end = last.at + last.inserted.text.len();
        if change.removed.is_empty() && last.removed.is_empty() && change.at == last_end {
            last.inserted.push(change.inserted);
            return;
        }
        if change.inserted.is_empty() && last.inserted.is_empty() {
            if change.at + change.removed.text.len() == last.at {
                // Backspace: the new piece is before the last.
                last.at = change.at;
                last.removed.prepend(change.removed);
                return;
            }
            if change.at == last.at {
                // Delete: the new piece was after the last.
                last.removed.push(change.removed);
                return;
            }
        }
    }
    changes.push(change);
}
