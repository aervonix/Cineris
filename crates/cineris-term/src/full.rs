//! The full-screen presenter (since step 25): the whole screen, row by row,
//! on the alternate screen (`docs/architecture/09-tui.md`, "Full screen as
//! the alternative").
//!
//! A [`Screen`] is every row of the window, top to bottom, and where the
//! cursor goes. [`Full::draw`] builds the frame in one buffer and hands it
//! to a [`Sink`] in **one** call:
//!
//! 1. begin a synchronized update, and hide the cursor;
//! 2. for each row that differs from the last frame (every row after a
//!    resize, a redraw or a resumption): move to it, write it, and erase the
//!    rest of it;
//! 3. put the cursor where the screen says and show it, or leave it hidden;
//! 4. end the synchronized update.
//!
//! Nothing is cleared and then drawn again: each row is written over in
//! place, and erasing is only ever of the part of a row after its text. A
//! row is placed by its number, which the alternate screen allows: nothing
//! scrolls there, and the presenter owns every row of it.
//!
//! It says nothing to the emergency restore about rows above the cursor
//! ([`Sink::region`]): the main screen holds none of its rows, and the
//! inline presenter's region, if the viewer was opened over it, is what the
//! restore must still erase there once it has left the alternate screen.

use std::io;

use crate::caps::Capabilities;
use crate::inline::{Cursor, Drawn, SYNC_BEGIN, SYNC_END, Sink};
use crate::line::Line;
use crate::render::{self, Target};

const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
/// Erase from the cursor to the end of its row.
const ERASE_LINE: &[u8] = b"\x1b[K";

/// What one full-screen frame shows.
#[derive(Debug, Clone, Copy, Default)]
pub struct Screen<'a> {
    /// The rows, from the top of the window. Rows past the window's height
    /// are not drawn; rows missing at the bottom are blank.
    pub lines: &'a [Line],
    /// Where the cursor is shown, its row counted from the top of the
    /// window; `None` hides it.
    pub cursor: Option<Cursor>,
}

/// The full-screen presenter. See the [module documentation](self).
#[derive(Debug)]
pub struct Full {
    cols: usize,
    rows: usize,
    /// Each row as it is on screen, rendered.
    screen: Vec<Vec<u8>>,
    /// Where the cursor was shown, if it was.
    cursor: Option<Cursor>,
    /// What the renderer was told of the terminal for the rows on screen.
    target: Option<Target>,
    /// Whether the rows on screen can no longer be trusted (nothing drawn
    /// yet, a resize, a redraw asked for): the next frame writes them all.
    stale: bool,
    buffer: Vec<u8>,
    scratch: Vec<Vec<u8>>,
}

impl Full {
    /// A presenter for a window of `cols` by `rows`.
    #[must_use]
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols: usize::from(cols.max(1)),
            rows: usize::from(rows.max(1)),
            screen: Vec::new(),
            cursor: None,
            target: None,
            stale: true,
            buffer: Vec::with_capacity(8192),
            scratch: Vec::new(),
        }
    }

    /// The window's size changed: the next frame writes every row.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (cols, rows) = (usize::from(cols.max(1)), usize::from(rows.max(1)));
        if (cols, rows) != (self.cols, self.rows) {
            self.cols = cols;
            self.rows = rows;
            self.stale = true;
        }
    }

    /// The next frame writes every row, as after `Ctrl+L`, after the
    /// alternate screen was entered again (a resumption, the external
    /// editor's hand-back), or after something else drew on it.
    pub fn redraw(&mut self) {
        self.stale = true;
    }

    /// The window's width, in columns.
    #[must_use]
    pub fn cols(&self) -> u16 {
        u16::try_from(self.cols).unwrap_or(u16::MAX)
    }

    /// The window's height, in rows.
    #[must_use]
    pub fn rows(&self) -> u16 {
        u16::try_from(self.rows).unwrap_or(u16::MAX)
    }

    /// The columns a row may use: all but the last, as for the inline
    /// presenter ([`Inline::width`](crate::Inline::width)).
    #[must_use]
    pub fn width(&self) -> u16 {
        u16::try_from(self.line_width()).unwrap_or(u16::MAX)
    }

    fn line_width(&self) -> usize {
        self.cols.saturating_sub(1).max(1)
    }

    /// Draws `screen` into `sink`, in one call, unless it would change
    /// nothing. Each row is cut to the width a row may use.
    ///
    /// # Errors
    ///
    /// When the sink cannot be written to. The presenter then assumes the
    /// frame did not arrive, and writes every row next time.
    pub fn draw(
        &mut self,
        screen: &Screen<'_>,
        caps: &Capabilities,
        sink: &mut impl Sink,
    ) -> io::Result<Drawn> {
        let target = Target::of(caps);
        let width = self.line_width();
        let mut rows = std::mem::take(&mut self.scratch);
        rows.clear();
        for index in 0..self.rows {
            let mut bytes = Vec::new();
            if let Some(line) = screen.lines.get(index) {
                render::line(&mut bytes, line, width, target);
            }
            rows.push(bytes);
        }
        let cursor = screen
            .cursor
            .filter(|cursor| cursor.row < self.rows)
            .map(|cursor| Cursor {
                row: cursor.row,
                col: cursor.col.min(width),
            });

        let full = self.stale || self.target != Some(target) || self.screen.len() != rows.len();
        if !full && rows == self.screen && cursor == self.cursor {
            self.scratch = rows;
            return Ok(Drawn::Skipped);
        }

        let mut out = std::mem::take(&mut self.buffer);
        out.clear();
        if caps.sync {
            out.extend_from_slice(SYNC_BEGIN);
        }
        out.extend_from_slice(HIDE_CURSOR);
        for (index, row) in rows.iter().enumerate() {
            if !full && self.screen.get(index) == Some(row) {
                continue;
            }
            place(&mut out, index, 0);
            out.extend_from_slice(row);
            out.extend_from_slice(ERASE_LINE);
        }
        if let Some(cursor) = cursor {
            place(&mut out, cursor.row, cursor.col);
            out.extend_from_slice(SHOW_CURSOR);
        }
        if caps.sync {
            out.extend_from_slice(SYNC_END);
        }

        let written = sink.write_frame(&out);
        let length = out.len();
        self.buffer = out;
        if let Err(error) = written {
            self.scratch = rows;
            self.stale = true;
            return Err(error);
        }
        self.scratch = std::mem::replace(&mut self.screen, rows);
        self.cursor = cursor;
        self.target = Some(target);
        self.stale = false;
        Ok(Drawn::Written(length))
    }
}

/// Moves the cursor to `row` and `col`, both counted from 0.
fn place(out: &mut Vec<u8>, row: usize, col: usize) {
    out.extend_from_slice(b"\x1b[");
    out.extend_from_slice((row + 1).to_string().as_bytes());
    if col > 0 {
        out.push(b';');
        out.extend_from_slice((col + 1).to_string().as_bytes());
    }
    out.push(b'H');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::{Overrides, Platform};

    fn caps() -> Capabilities {
        Capabilities::detect(|_| None, Platform::Unix, Overrides::default())
    }

    fn lines(texts: &[&str]) -> Vec<Line> {
        texts.iter().map(|text| Line::plain(*text)).collect()
    }

    fn draw(full: &mut Full, texts: &[&str], cursor: Option<Cursor>) -> (Drawn, String) {
        let lines = lines(texts);
        let mut sink = Vec::new();
        let drawn = full
            .draw(
                &Screen {
                    lines: &lines,
                    cursor,
                },
                &caps(),
                &mut sink,
            )
            .unwrap();
        (drawn, String::from_utf8(sink).unwrap())
    }

    #[test]
    fn the_first_frame_writes_every_row_and_the_next_only_what_changed() {
        let mut full = Full::new(20, 3);
        let (_, first) = draw(&mut full, &["one", "two"], None);
        assert!(first.contains("\x1b[1Hone\x1b[K"), "{first:?}");
        assert!(first.contains("\x1b[2Htwo\x1b[K"), "{first:?}");
        // The third row, blank, is erased too: nothing of an old screen
        // stays.
        assert!(first.contains("\x1b[3H\x1b[K"), "{first:?}");
        let (_, second) = draw(&mut full, &["one", "TWO"], None);
        assert!(!second.contains("one"), "{second:?}");
        assert!(second.contains("\x1b[2HTWO\x1b[K"), "{second:?}");
        assert!(!second.contains("\x1b[3H"), "{second:?}");
    }

    #[test]
    fn a_frame_that_changes_nothing_is_skipped() {
        let mut full = Full::new(20, 3);
        draw(&mut full, &["one"], None);
        assert_eq!(draw(&mut full, &["one"], None).0, Drawn::Skipped);
        // The cursor moved: that is a change.
        let cursor = Some(Cursor { row: 0, col: 2 });
        let (drawn, bytes) = draw(&mut full, &["one"], cursor);
        assert!(matches!(drawn, Drawn::Written(_)));
        assert!(bytes.contains("\x1b[1;3H\x1b[?25h"), "{bytes:?}");
        assert!(!bytes.contains("one"), "{bytes:?}");
    }

    #[test]
    fn nothing_is_cleared_and_nothing_scrolls() {
        let mut full = Full::new(20, 4);
        for texts in [
            &["a", "b", "c", "d"][..],
            &["x"][..],
            &["a", "b", "c", "d", "e"][..],
        ] {
            let (_, bytes) = draw(&mut full, texts, None);
            assert!(
                !bytes.contains("\x1b[2J") && !bytes.contains("\x1b[J"),
                "{bytes:?}"
            );
            // No line break: every row is placed by its number, so nothing
            // can scroll, and a fifth line is not drawn at all.
            assert!(!bytes.contains('\n') && !bytes.contains('e'), "{bytes:?}");
        }
    }

    #[test]
    fn a_resize_or_a_redraw_writes_every_row_again() {
        let mut full = Full::new(20, 2);
        draw(&mut full, &["one", "two"], None);
        full.redraw();
        let (_, bytes) = draw(&mut full, &["one", "two"], None);
        assert!(bytes.contains("one") && bytes.contains("two"), "{bytes:?}");
        full.resize(30, 2);
        let (_, bytes) = draw(&mut full, &["one", "two"], None);
        assert!(bytes.contains("one") && bytes.contains("two"), "{bytes:?}");
    }

    #[test]
    fn a_row_is_cut_before_the_last_column_and_the_cursor_kept_inside() {
        let mut full = Full::new(5, 2);
        let cursor = Some(Cursor { row: 1, col: 40 });
        let (_, bytes) = draw(&mut full, &["abcdefgh"], cursor);
        assert!(bytes.contains("\x1b[1Habcd"), "{bytes:?}");
        assert!(!bytes.contains("abcde"), "{bytes:?}");
        assert!(bytes.contains("\x1b[2;5H\x1b[?25h"), "{bytes:?}");
        // A cursor below the window is not shown.
        let (_, bytes) = draw(&mut full, &["abcdefgh"], Some(Cursor { row: 2, col: 0 }));
        assert!(!bytes.contains("\x1b[?25h"), "{bytes:?}");
    }

    #[test]
    fn a_failed_write_is_drawn_whole_next_time() {
        struct Failing;
        impl Sink for Failing {
            fn write_frame(&mut self, _: &[u8]) -> io::Result<()> {
                Err(io::Error::other("gone"))
            }
        }
        let mut full = Full::new(20, 2);
        draw(&mut full, &["one", "two"], None);
        let lines = lines(&["one", "TWO"]);
        let screen = Screen {
            lines: &lines,
            cursor: None,
        };
        assert!(full.draw(&screen, &caps(), &mut Failing).is_err());
        let (_, bytes) = draw(&mut full, &["one", "TWO"], None);
        assert!(bytes.contains("one") && bytes.contains("TWO"), "{bytes:?}");
    }
}
