//! A window over the kept transcript and the rows still live (since step
//! 25): what the full-screen presenter and the viewer show of a session,
//! scrolled (`docs/architecture/09-tui.md`, "Full screen as the
//! alternative"; `docs/steps/25/answered.md`, point 2).
//!
//! **The rows are made only around the window.** Each kept block's rows at
//! the width are made when the window first reaches them, kept while the
//! block is near the window, and dropped after; a change of width drops
//! them all. A frame costs the rows near the screen, not the session's
//! length.
//!
//! **Where the window is** is the block its top row is in and the row inside
//! it, or [`Anchor::End`]: following the end, which it does from the start.
//! Held by its block, the window does not move when something changes above
//! it or below it; a block that is drawn again with another height keeps the
//! window on its own row as near as it can.

use std::collections::HashMap;
use std::rc::Rc;

use cortex_term::Line;

/// Where the window's top is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Anchor {
    /// Following the end: the last row is the window's last.
    #[default]
    End,
    /// The top row is row `row` of block `block`; the block one past the
    /// kept ones is the live rows.
    At {
        /// The block.
        block: usize,
        /// The row inside it.
        row: usize,
    },
}

/// Where a row of the window comes from: a kept block, or the live rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// Row `row` of kept block `block`.
    Kept {
        /// The block.
        block: usize,
        /// The row inside it.
        row: usize,
    },
    /// Row `row` of the live rows.
    Live(usize),
    /// Below everything: the window is taller than what there is.
    Empty,
}

/// What a frame shows of the transcript.
#[derive(Debug, Clone, Default)]
pub(crate) struct Window {
    /// The rows, as many as the window is tall.
    pub(crate) rows: Vec<Line>,
    /// Where each row comes from.
    pub(crate) places: Vec<Place>,
    /// Whether the window shows the last row: it follows the end.
    pub(crate) at_end: bool,
}

/// The rows made for the kept blocks near the window, at one width.
#[derive(Debug, Default)]
pub(crate) struct Rows {
    width: usize,
    made: HashMap<usize, Rc<Vec<Line>>>,
}

impl Rows {
    /// Block `block`'s rows at `width`, made by `make` unless they were
    /// already. A width other than the last drops every row made before.
    pub(crate) fn of(
        &mut self,
        block: usize,
        width: usize,
        make: &mut dyn FnMut(usize) -> Vec<Line>,
    ) -> Rc<Vec<Line>> {
        if width != self.width {
            self.made.clear();
            self.width = width;
        }
        Rc::clone(
            self.made
                .entry(block)
                .or_insert_with(|| Rc::new(make(block))),
        )
    }

    /// Forgets block `block`'s rows: it is to be drawn otherwise (a box
    /// opened, colours that came late).
    pub(crate) fn forget(&mut self, block: usize) {
        self.made.remove(&block);
    }

    /// Forgets every block's rows: the look changed, or the kept transcript
    /// was made again.
    pub(crate) fn clear(&mut self) {
        self.made.clear();
    }

    /// Keeps only the rows of the blocks in `near`.
    pub(crate) fn keep(&mut self, near: std::ops::RangeInclusive<usize>) {
        self.made.retain(|block, _| near.contains(block));
    }

    /// How many blocks' rows are kept: what the test of the bound counts.
    pub(crate) fn len(&self) -> usize {
        self.made.len()
    }
}

/// A scrolled view of the kept transcript. See the [module
/// documentation](self).
#[derive(Debug, Default)]
pub(crate) struct Scroll {
    /// Where the window is.
    pub(crate) anchor: Anchor,
    /// The kept blocks' rows near the window.
    pub(crate) rows: Rows,
    /// The top of the last window drawn, as a block and a row: where a move
    /// from the end starts.
    top: Option<(usize, usize)>,
    /// How many rows the last window had: a page.
    pub(crate) height: usize,
}

/// What the window is drawn from: how many blocks are kept, how to make
/// one's rows, the live rows, and the width.
pub(crate) struct Source<'a> {
    /// How many blocks are kept.
    pub(crate) blocks: usize,
    /// Makes a kept block's rows at the width.
    pub(crate) make: &'a mut dyn FnMut(usize) -> Vec<Line>,
    /// The rows still live, drawn under the kept blocks.
    pub(crate) live: &'a [Line],
    /// The columns a row may use.
    pub(crate) width: usize,
}

impl Scroll {
    /// Whether the window follows the end.
    pub(crate) fn following(&self) -> bool {
        self.anchor == Anchor::End
    }

    /// Follows the end again.
    pub(crate) fn follow(&mut self) {
        self.anchor = Anchor::End;
    }

    /// A block's rows, from the cache.
    fn block(&mut self, source: &mut Source<'_>, block: usize) -> Rc<Vec<Line>> {
        if block >= source.blocks {
            return Rc::new(source.live.to_vec());
        }
        self.rows.of(block, source.width, source.make)
    }

    /// The window of `height` rows for this frame.
    pub(crate) fn window(&mut self, source: &mut Source<'_>, height: usize) -> Window {
        self.height = height;
        let height = height.max(1);
        let window = match self.anchor {
            Anchor::End => self.from_end(source, height),
            Anchor::At { block, row } => {
                let window = self.from(source, block, row, height);
                if window.at_end {
                    // Moved down to the end: following it again.
                    self.anchor = Anchor::End;
                    self.from_end(source, height)
                } else {
                    window
                }
            }
        };
        // Only the rows of the blocks near the window stay made: two
        // windows' worth of blocks either side, at least a few.
        let shown: Vec<usize> = window
            .places
            .iter()
            .filter_map(|place| match place {
                Place::Kept { block, .. } => Some(*block),
                _ => None,
            })
            .collect();
        let (first, last) = match (shown.first(), shown.last()) {
            (Some(first), Some(last)) => (*first, *last),
            _ => (source.blocks.saturating_sub(1), source.blocks.saturating_sub(1)),
        };
        let spread = (last - first + 1) * 2 + 8;
        self.rows
            .keep(first.saturating_sub(spread)..=last.saturating_add(spread));
        window
    }

    /// The window that ends with the last row.
    fn from_end(&mut self, source: &mut Source<'_>, height: usize) -> Window {
        let mut pieces: Vec<(usize, Rc<Vec<Line>>)> = Vec::new();
        let mut have = 0;
        let mut block = source.blocks + 1;
        while have < height && block > 0 {
            block -= 1;
            let rows = self.block(source, block);
            have += rows.len();
            pieces.push((block, rows));
        }
        pieces.reverse();
        let skip = have.saturating_sub(height);
        let mut window = gather(&pieces, skip, height, source.blocks);
        window.at_end = true;
        self.top = window.places.iter().find_map(|place| match place {
            Place::Kept { block, row } => Some((*block, *row)),
            Place::Live(row) => Some((source.blocks, *row)),
            Place::Empty => None,
        });
        window
    }

    /// The window whose top is row `row` of block `block`.
    fn from(&mut self, source: &mut Source<'_>, block: usize, row: usize, height: usize) -> Window {
        let block = block.min(source.blocks);
        let mut pieces: Vec<(usize, Rc<Vec<Line>>)> = Vec::new();
        let first = self.block(source, block);
        let row = row.min(first.len().saturating_sub(1));
        let mut have = first.len().saturating_sub(row);
        pieces.push((block, first));
        let mut next = block + 1;
        while have < height && next <= source.blocks {
            let rows = self.block(source, next);
            have += rows.len();
            pieces.push((next, rows));
            next += 1;
        }
        let mut window = gather(&pieces, row, height, source.blocks);
        // The end is in view when nothing is left below the window's last
        // row.
        window.at_end = next > source.blocks && have <= height;
        self.top = Some((block, row));
        window
    }

    /// Moves the window up `rows` rows, which stops it following the end.
    pub(crate) fn up(&mut self, source: &mut Source<'_>, rows: usize) {
        let Some((mut block, row)) = self.top_now(source) else {
            return;
        };
        let mut row = row;
        let mut left = rows;
        while left > 0 {
            if row >= left {
                row -= left;
                left = 0;
            } else {
                left -= row;
                if block == 0 {
                    row = 0;
                    break;
                }
                block -= 1;
                row = self.block(source, block).len();
                if row == 0 {
                    continue;
                }
            }
        }
        // A row past the end of its block is the next block's first.
        let len = self.block(source, block).len();
        if row >= len && block < source.blocks {
            block += 1;
            row = 0;
        }
        self.anchor = Anchor::At { block, row };
    }

    /// Moves the window down `rows` rows; reaching the end follows it again.
    pub(crate) fn down(&mut self, source: &mut Source<'_>, rows: usize) {
        if self.following() {
            return;
        }
        let Some((mut block, mut row)) = self.top_now(source) else {
            return;
        };
        row += rows;
        loop {
            let len = self.block(source, block).len();
            if row < len || block >= source.blocks {
                break;
            }
            row -= len;
            block += 1;
        }
        self.anchor = Anchor::At { block, row };
        // Whether that reached the end is found when the window is made.
        let height = self.height.max(1);
        if self.from(source, block, row, height).at_end {
            self.anchor = Anchor::End;
        }
    }

    /// Moves the window to the first row.
    pub(crate) fn top(&mut self) {
        self.anchor = Anchor::At { block: 0, row: 0 };
    }

    /// Moves the window so that row `row` of block `block` is in it, at its
    /// top if it was not: what a search's match and a selected box do.
    pub(crate) fn show(&mut self, source: &mut Source<'_>, block: usize, row: usize) {
        let height = self.height.max(1);
        let window = self.window(source, height);
        let shown = window
            .places
            .iter()
            .any(|place| *place == Place::Kept { block, row });
        if !shown {
            self.anchor = Anchor::At { block, row };
        }
    }

    /// The window's top now: the anchor's, or where the end's window began.
    fn top_now(&mut self, source: &mut Source<'_>) -> Option<(usize, usize)> {
        match self.anchor {
            Anchor::At { block, row } => Some((block, row)),
            Anchor::End => {
                let height = self.height.max(1);
                self.from_end(source, height);
                self.top
            }
        }
    }
}

/// The window's rows from `pieces`, skipping `skip` rows of the first, up
/// to `height`, padded with blank rows.
fn gather(pieces: &[(usize, Rc<Vec<Line>>)], skip: usize, height: usize, blocks: usize) -> Window {
    let mut window = Window::default();
    let mut skip = skip;
    'pieces: for (block, rows) in pieces {
        for (row, line) in rows.iter().enumerate() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            if window.rows.len() == height {
                break 'pieces;
            }
            window.rows.push(line.clone());
            window.places.push(if *block >= blocks {
                Place::Live(row)
            } else {
                Place::Kept { block: *block, row }
            });
        }
    }
    while window.rows.len() < height {
        window.rows.push(Line::new());
        window.places.push(Place::Empty);
    }
    window
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Blocks of the given heights, each row saying where it is.
    fn blocks(heights: &[usize]) -> Vec<Vec<Line>> {
        heights
            .iter()
            .enumerate()
            .map(|(block, &height)| {
                (0..height)
                    .map(|row| Line::plain(format!("{block}.{row}")))
                    .collect()
            })
            .collect()
    }

    fn texts(window: &Window) -> Vec<String> {
        window
            .rows
            .iter()
            .map(|line| line.text().into_owned())
            .collect()
    }

    struct Rig {
        blocks: Vec<Vec<Line>>,
        live: Vec<Line>,
        made: usize,
        scroll: Scroll,
    }

    impl Rig {
        fn new(heights: &[usize], live: usize) -> Self {
            Self {
                blocks: blocks(heights),
                live: (0..live).map(|row| Line::plain(format!("live.{row}"))).collect(),
                made: 0,
                scroll: Scroll::default(),
            }
        }

        fn with<T>(&mut self, act: impl FnOnce(&mut Scroll, &mut Source<'_>) -> T) -> T {
            let blocks = &self.blocks;
            let made = &mut self.made;
            let mut make = |block: usize| {
                *made += 1;
                blocks[block].clone()
            };
            let mut source = Source {
                blocks: blocks.len(),
                make: &mut make,
                live: &self.live,
                width: 80,
            };
            act(&mut self.scroll, &mut source)
        }

        fn window(&mut self, height: usize) -> Vec<String> {
            texts(&self.with(|scroll, source| scroll.window(source, height)))
        }
    }

    #[test]
    fn it_follows_the_end_from_the_start_and_as_rows_are_added() {
        let mut rig = Rig::new(&[3, 2, 4], 1);
        assert_eq!(rig.window(4), ["2.1", "2.2", "2.3", "live.0"]);
        rig.live.push(Line::plain("live.1"));
        assert_eq!(rig.window(4), ["2.2", "2.3", "live.0", "live.1"]);
        assert!(rig.scroll.following());
    }

    #[test]
    fn a_move_up_stops_following_and_what_is_added_below_does_not_move_it() {
        let mut rig = Rig::new(&[3, 2, 4], 1);
        rig.window(4);
        rig.with(|scroll, source| scroll.up(source, 3));
        assert!(!rig.scroll.following());
        let held = rig.window(4);
        assert_eq!(held, ["1.0", "1.1", "2.0", "2.1"]);
        rig.live.push(Line::plain("live.1"));
        assert_eq!(rig.window(4), held);
        // Down past the end follows again.
        rig.with(|scroll, source| scroll.down(source, 100));
        assert!(rig.scroll.following());
        assert_eq!(rig.window(4), ["2.2", "2.3", "live.0", "live.1"]);
    }

    #[test]
    fn up_stops_at_the_first_row_and_a_short_transcript_is_padded() {
        let mut rig = Rig::new(&[2, 1], 0);
        rig.window(6);
        rig.with(|scroll, source| scroll.up(source, 50));
        assert_eq!(rig.window(6), ["0.0", "0.1", "1.0", "", "", ""]);
        let mut rig = Rig::new(&[3, 3, 3], 0);
        rig.window(2);
        rig.with(|scroll, source| scroll.up(source, 50));
        assert_eq!(rig.window(2), ["0.0", "0.1"]);
    }

    #[test]
    fn a_window_makes_only_the_rows_of_the_blocks_near_it() {
        let heights = vec![5; 1_000];
        let mut rig = Rig::new(&heights, 2);
        rig.window(10);
        // Two or three blocks are shown; those near them are made, not the
        // thousand.
        assert!(rig.made <= 4, "{} blocks made", rig.made);
        rig.with(|scroll, _| scroll.top());
        assert_eq!(rig.window(10)[0], "0.0");
        assert!(rig.scroll.rows.len() <= 30, "{}", rig.scroll.rows.len());
        // The window moved away from the end: the blocks there were let go.
        assert!(rig.made <= 10, "{} blocks made", rig.made);
    }

    #[test]
    fn a_shown_row_does_not_move_the_window_and_one_not_shown_brings_it_to_the_top() {
        let mut rig = Rig::new(&[3, 3, 3, 3], 0);
        rig.window(4);
        rig.with(|scroll, source| scroll.show(source, 3, 1));
        assert!(rig.scroll.following());
        rig.with(|scroll, source| scroll.show(source, 1, 2));
        assert_eq!(rig.window(4), ["1.2", "2.0", "2.1", "2.2"]);
    }
}
