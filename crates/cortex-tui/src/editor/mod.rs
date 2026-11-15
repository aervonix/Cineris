//! The prompt's text editor (`docs/architecture/09-tui.md`, "The editor").
//!
//! - **Text.** UTF-8, and every position is a grapheme cluster boundary:
//!   moving and deleting go a whole cluster at a time, and an edit that
//!   joins two clusters moves the cursor out of the one it made
//!   (`cluster`).
//! - **Chips.** A paste too long to show is one [`Chip`], drawn as its
//!   label and sent as what it holds (`buffer`).
//! - **Undo.** Typing is grouped into words; a paste, a yank, a kill, a
//!   completion and a line break are each a step of their own (`undo`).
//! - **Kill and yank.** One slot. Kills one after another add to it, so
//!   `Ctrl+W Ctrl+W Ctrl+Y` gives both words back.
//! - **Rows.** The text wraps at a width the caller gives, and the cursor
//!   moves up and down by row (`layout`).
//!
//! The editor knows nothing of keys: the key map turns keys into
//! [`Action`](crate::keys::Action)s, and the input calls the methods here.

pub(crate) mod buffer;
pub(crate) mod cluster;
pub(crate) mod layout;
pub(crate) mod undo;

pub use buffer::{Chip, Fragment};
pub use layout::{Layout, Row};

use buffer::Buffer;
use cluster::{CHIP, Class};
use undo::{Change, Open};

/// Which way a kill went, so that kills one after another join in the slot
/// in the order the text had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kill {
    Back,
    Forward,
}

/// The prompt's text, its cursor, and its undo history.
#[derive(Debug, Default)]
pub struct Editor {
    buffer: Buffer,
    /// A byte offset, always on a cluster boundary.
    cursor: usize,
    undo: undo::History,
    /// The kill-and-yank slot.
    slot: Option<Fragment>,
    /// The last action was a kill, this way.
    killing: Option<Kill>,
    /// The column that up and down aim for, kept while they repeat.
    goal: Option<usize>,
}

impl Editor {
    /// An empty editor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The text, with each chip as one U+FFFC.
    #[must_use]
    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// The chips, in the order of their marks in [`text`](Self::text).
    #[must_use]
    pub fn chips(&self) -> &[Chip] {
        self.buffer.chips()
    }

    /// The chip whose mark is at byte `at` of [`text`](Self::text).
    #[must_use]
    pub fn chip_at(&self, at: usize) -> Option<&Chip> {
        if at >= self.text().len() || !self.text().is_char_boundary(at) {
            return None;
        }
        self.buffer.chip_at(at)
    }

    /// The cursor, as a byte offset into [`text`](Self::text).
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether there is no text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.text().is_empty()
    }

    /// The text with every chip expanded: what sending it sends.
    #[must_use]
    pub fn expanded(&self) -> String {
        self.buffer.expanded()
    }

    /// The text and its chips.
    #[must_use]
    pub fn fragment(&self) -> Fragment {
        self.buffer.fragment(0..self.buffer.text().len())
    }

    /// What the kill-and-yank slot holds.
    #[must_use]
    pub fn slot(&self) -> Option<&Fragment> {
        self.slot.as_ref()
    }

    /// The rows at `width` columns and where the cursor is in them.
    #[must_use]
    pub fn layout(&self, width: usize) -> Layout {
        layout::layout(&self.buffer, width, self.cursor)
    }

    // ------------------------------------------------------------ editing

    /// Types one character at the cursor. Characters typed one after
    /// another are undone a word at a time.
    pub fn type_char(&mut self, c: char) {
        let mut text = [0; 4];
        self.type_str(c.encode_utf8(&mut text));
    }

    /// Types `text` at the cursor, as typing: it joins the word being
    /// typed. A line break in it is kept; `\r` is taken as a line break.
    pub fn type_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let fragment = Fragment::text(normalize(text));
        let at = self.cursor;
        let previous = self.undo.open();
        let (change, before) = self.change(at..at, fragment);
        let word = cluster::class_before(self.text(), self.cursor) == Some(Class::Word);
        // A word that starts after something that was not one starts a
        // new step of undo.
        let join = match previous {
            Some(Open::Typing { word: last_word }) => !(word && !last_word),
            _ => false,
        };
        self.undo.record(
            change,
            before,
            self.cursor,
            join,
            Some(Open::Typing { word }),
        );
    }

    /// Inserts `fragment` at the cursor as one step of undo: a paste, a
    /// yank, a completion.
    pub fn insert(&mut self, fragment: Fragment) {
        if fragment.is_empty() {
            return;
        }
        let at = self.cursor;
        self.apply(at..at, fragment);
    }

    /// Replaces `range` with `fragment` as one step of undo, and leaves the
    /// cursor after it. For a completion, which replaces what was typed.
    pub fn replace(&mut self, range: std::ops::Range<usize>, fragment: Fragment) {
        let range = self.boundaries(range);
        self.apply(range, fragment);
    }

    /// Starts a new line at the cursor.
    pub fn new_line(&mut self) {
        let at = self.cursor;
        self.apply(at..at, Fragment::text("\n"));
    }

    /// Replaces the whole text, as one step of undo: an entry from history,
    /// or what the external editor gave back. The cursor goes to the end.
    pub fn set(&mut self, fragment: Fragment) {
        let all = 0..self.buffer.text().len();
        self.apply(all, fragment);
    }

    /// Removes all the text, as one step of undo.
    pub fn clear(&mut self) {
        self.set(Fragment::default());
    }

    /// Takes the text out, for sending: the editor is left empty, with no
    /// history of undo. The kill slot is kept.
    pub fn take(&mut self) -> Fragment {
        let fragment = self.fragment();
        self.buffer = Buffer::default();
        self.cursor = 0;
        self.undo.clear();
        self.killing = None;
        self.goal = None;
        fragment
    }

    /// Deletes the cluster before the cursor. Deletes one after another
    /// are undone a word at a time.
    pub fn backspace(&mut self) -> bool {
        let at = self.cursor;
        if at == 0 {
            return false;
        }
        let from = cluster::prev(self.text(), at);
        let word = cluster::class(&self.text()[from..at]) == Class::Word;
        let join = match self.undo.open() {
            Some(Open::Erasing { word: last_word }) => !(last_word && !word),
            _ => false,
        };
        let (change, before) = self.change(from..at, Fragment::default());
        self.undo.record(
            change,
            before,
            self.cursor,
            join,
            Some(Open::Erasing { word }),
        );
        true
    }

    /// Deletes the cluster after the cursor.
    pub fn delete(&mut self) -> bool {
        let at = self.cursor;
        if at >= self.text().len() {
            return false;
        }
        let to = cluster::next(self.text(), at);
        let word = cluster::class(&self.text()[at..to]) == Class::Word;
        let join = match self.undo.open() {
            Some(Open::Deleting { word: last_word }) => !(word && !last_word),
            _ => false,
        };
        let (change, before) = self.change(at..to, Fragment::default());
        self.undo.record(
            change,
            before,
            self.cursor,
            join,
            Some(Open::Deleting { word }),
        );
        true
    }

    /// Deletes the word before the cursor into the slot.
    pub fn kill_word_back(&mut self) -> bool {
        let from = cluster::word_start(self.text(), self.cursor);
        self.kill(from..self.cursor, Kill::Back)
    }

    /// Deletes the word after the cursor into the slot.
    pub fn kill_word_forward(&mut self) -> bool {
        let to = cluster::word_end(self.text(), self.cursor);
        self.kill(self.cursor..to, Kill::Forward)
    }

    /// Deletes to the end of the line into the slot; at its end, the line
    /// break.
    pub fn kill_to_line_end(&mut self) -> bool {
        let end = cluster::line_end(self.text(), self.cursor);
        let to = if end == self.cursor {
            cluster::next(self.text(), end)
        } else {
            end
        };
        self.kill(self.cursor..to, Kill::Forward)
    }

    /// Deletes to the start of the line into the slot; at its start, the
    /// line break before it.
    pub fn kill_to_line_start(&mut self) -> bool {
        let start = cluster::line_start(self.text(), self.cursor);
        let from = if start == self.cursor {
            cluster::prev(self.text(), start)
        } else {
            start
        };
        self.kill(from..self.cursor, Kill::Back)
    }

    /// Inserts what the slot holds.
    pub fn yank(&mut self) -> bool {
        let Some(fragment) = self.slot.clone() else {
            return false;
        };
        self.insert(fragment);
        true
    }

    /// Replaces the chip beside the cursor with its text: the one before
    /// it, or else the one after it.
    pub fn expand_chip(&mut self) -> bool {
        let at = self.cursor;
        let before = cluster::prev(self.text(), at);
        let range = if at > 0 && self.text()[before..at].starts_with(CHIP) {
            before..at
        } else if self.text()[at..].starts_with(CHIP) {
            at..at + CHIP.len_utf8()
        } else {
            return false;
        };
        let Some(chip) = self.buffer.chip_at(range.start) else {
            return false;
        };
        let content = Fragment::text(chip.content());
        let after = range.start == before;
        self.apply(range.clone(), content);
        if !after {
            self.cursor = range.start;
        }
        true
    }

    /// Undoes the last step.
    pub fn undo(&mut self) -> bool {
        self.settle();
        match self.undo.undo(&mut self.buffer) {
            Some(cursor) => {
                self.cursor = cluster::snap(self.buffer.text(), cursor);
                true
            }
            None => false,
        }
    }

    /// Makes the last undone step again.
    pub fn redo(&mut self) -> bool {
        self.settle();
        match self.undo.redo(&mut self.buffer) {
            Some(cursor) => {
                self.cursor = cluster::snap(self.buffer.text(), cursor);
                true
            }
            None => false,
        }
    }

    // ------------------------------------------------------------- moving

    /// One cluster left.
    pub fn left(&mut self) -> bool {
        let to = cluster::prev(self.text(), self.cursor);
        self.move_to(to)
    }

    /// One cluster right.
    pub fn right(&mut self) -> bool {
        let to = cluster::next(self.text(), self.cursor);
        self.move_to(to)
    }

    /// To the start of the word before the cursor.
    pub fn word_left(&mut self) -> bool {
        let to = cluster::word_start(self.text(), self.cursor);
        self.move_to(to)
    }

    /// To the end of the word after the cursor.
    pub fn word_right(&mut self) -> bool {
        let to = cluster::word_end(self.text(), self.cursor);
        self.move_to(to)
    }

    /// To the start of the line.
    pub fn line_start(&mut self) -> bool {
        let to = cluster::line_start(self.text(), self.cursor);
        self.move_to(to)
    }

    /// To the end of the line.
    pub fn line_end(&mut self) -> bool {
        let to = cluster::line_end(self.text(), self.cursor);
        self.move_to(to)
    }

    /// To the start of the text.
    pub fn text_start(&mut self) -> bool {
        self.move_to(0)
    }

    /// To the end of the text.
    pub fn text_end(&mut self) -> bool {
        let to = self.text().len();
        self.move_to(to)
    }

    /// Up one row at `width` columns. `false` on the first row, where the
    /// caller steps into history instead.
    pub fn up(&mut self, width: usize) -> bool {
        self.vertical(width, false)
    }

    /// Down one row. `false` on the last row.
    pub fn down(&mut self, width: usize) -> bool {
        self.vertical(width, true)
    }

    /// Whether the cursor is on the first row at `width`.
    #[must_use]
    pub fn on_first_row(&self, width: usize) -> bool {
        self.layout(width).cursor_row == 0
    }

    /// Whether the cursor is on the last row at `width`.
    #[must_use]
    pub fn on_last_row(&self, width: usize) -> bool {
        let layout = self.layout(width);
        layout.cursor_row + 1 == layout.rows.len()
    }

    /// Puts the cursor at byte `at`, snapped to a cluster boundary: a byte
    /// inside a character or past the end is taken as the nearest boundary
    /// after it.
    pub fn set_cursor(&mut self, at: usize) {
        self.settle();
        self.goal = None;
        let at = char_boundary(self.text(), at);
        self.cursor = cluster::snap(self.text(), at);
    }

    fn vertical(&mut self, width: usize, down: bool) -> bool {
        let width = width.max(1);
        let layout = self.layout(width);
        let target = if down {
            layout.cursor_row + 1
        } else {
            match layout.cursor_row.checked_sub(1) {
                Some(row) => row,
                None => return false,
            }
        };
        let Some(row) = layout.rows.get(target) else {
            return false;
        };
        let goal = *self.goal.get_or_insert(layout.cursor_col);
        let to = layout::offset_at(&self.buffer, row, goal, width);
        self.undo.close();
        self.killing = None;
        self.cursor = to;
        true
    }

    fn move_to(&mut self, to: usize) -> bool {
        self.settle();
        self.goal = None;
        let moved = to != self.cursor;
        self.cursor = to;
        moved
    }

    // ---------------------------------------------------------- internals

    /// Ends what typing, deleting or killing had open.
    fn settle(&mut self) {
        self.undo.close();
        self.killing = None;
    }

    fn kill(&mut self, range: std::ops::Range<usize>, way: Kill) -> bool {
        let range = self.boundaries(range);
        if range.is_empty() {
            return false;
        }
        // Kills one after another join, whichever way each went.
        let continuing = self.killing.is_some();
        let removed = self.buffer.fragment(range.clone());
        self.apply(range, Fragment::default());
        match (continuing, self.slot.as_mut(), way) {
            (true, Some(slot), Kill::Back) => slot.prepend(removed),
            (true, Some(slot), Kill::Forward) => slot.push(removed),
            _ => self.slot = Some(removed),
        }
        self.killing = Some(way);
        true
    }

    /// `range` with both ends on cluster boundaries.
    fn boundaries(&self, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
        let text = self.text();
        let start = cluster::snap(text, char_boundary(text, range.start));
        let end = cluster::snap(text, char_boundary(text, range.end)).max(start);
        start..end
    }

    /// Makes one change as a step of undo of its own.
    fn apply(&mut self, range: std::ops::Range<usize>, fragment: Fragment) {
        self.undo.close();
        let (change, before) = self.change(range, fragment);
        self.undo.record(change, before, self.cursor, false, None);
    }

    /// Makes one change, puts the cursor after what came in, and returns
    /// the change with the cursor it was made at, for the caller to record.
    fn change(&mut self, range: std::ops::Range<usize>, fragment: Fragment) -> (Change, usize) {
        let before = self.cursor;
        let at = range.start;
        let inserted = fragment.clone();
        let length = fragment.text.len();
        let removed = self.buffer.replace(range, fragment);
        self.cursor = cluster::snap(self.buffer.text(), at + length);
        self.killing = None;
        self.goal = None;
        (
            Change {
                at,
                removed,
                inserted,
            },
            before,
        )
    }
}

/// `at` within `text` and on a character boundary: the first one at or
/// after it.
fn char_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

/// Line breaks as the editor holds them: `\r\n` and a lone `\r` become
/// `\n`.
pub(crate) fn normalize(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}
