//! The editor's text in rows of a given width.
//!
//! Unlike [`cortex_base::text::wrap`], which drops the white space it breaks
//! at, every byte of the text is in exactly one row here: the cursor can be
//! anywhere, and every place it can be has a row and a column. A row breaks
//! after the last white space that fits, or between clusters when a word is
//! wider than the row. A line break ends a row and belongs to none.
//!
//! **The cursor's column.** A row holds at most `width` columns. The cursor
//! after the last cluster of a full row that ends a line is at column
//! `width`: the caller keeps one column beside the text for it (the input
//! box's padding), so it never reaches the column the box's edge is in. A
//! cursor at the end of a row that wrapped is at the start of the next row.

use std::ops::Range;

use super::buffer::Buffer;
use super::cluster::{self, CHIP, Class};
use cortex_base::text;

/// One row of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The bytes of the text it shows.
    pub range: Range<usize>,
    /// Whether a line break or the end of the text ends it, rather than
    /// wrapping.
    pub ends_line: bool,
}

/// The rows of a text at a width, and where the cursor is in them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The rows, in order. Never empty: an empty text is one empty row.
    pub rows: Vec<Row>,
    /// The cursor's row.
    pub cursor_row: usize,
    /// The cursor's column in its row.
    pub cursor_col: usize,
}

/// The width of the cluster at `at` in `buffer`: a chip's label is cut to
/// the row, and is drawn cut to the same width.
pub(crate) fn cluster_width(buffer: &Buffer, at: usize, cluster: &str, width: usize) -> usize {
    if cluster.starts_with(CHIP) {
        let label = buffer
            .chip_at(at)
            .map_or(1, |chip| text::width(chip.label()));
        label.clamp(1, width.max(1))
    } else {
        cluster::width(cluster)
    }
}

/// Lays `buffer` out in rows of `width` columns, with the cursor at byte
/// `cursor`.
pub(crate) fn layout(buffer: &Buffer, width: usize, cursor: usize) -> Layout {
    let width = width.max(1);
    let text = buffer.text();
    let mut rows = Vec::new();
    let mut line_start = 0;
    loop {
        let line_end = cluster::line_end(text, line_start);
        wrap_line(buffer, line_start..line_end, width, &mut rows);
        if line_end == text.len() {
            break;
        }
        line_start = line_end + 1;
    }
    let cursor_row = rows
        .iter()
        .position(|row| row.range.contains(&cursor) || (row.ends_line && row.range.end == cursor))
        .unwrap_or(rows.len() - 1);
    let start = rows[cursor_row].range.start;
    let cursor_col = columns(buffer, start..cursor.max(start), width);
    Layout {
        rows,
        cursor_row,
        cursor_col,
    }
}

/// The columns the bytes `range` take.
pub(crate) fn columns(buffer: &Buffer, range: Range<usize>, width: usize) -> usize {
    let text = buffer.text();
    cluster::clusters(&text[range.clone()])
        .map(|(offset, c)| cluster_width(buffer, range.start + offset, c, width))
        .sum()
}

fn wrap_line(buffer: &Buffer, line: Range<usize>, width: usize, rows: &mut Vec<Row>) {
    let text = &buffer.text()[line.clone()];
    let mut row_start = line.start;
    let mut col = 0;
    // After the last white space in the row: where it may break, and the
    // column there.
    let mut breakable: Option<(usize, usize)> = None;
    for (offset, c) in cluster::clusters(text) {
        let at = line.start + offset;
        let w = cluster_width(buffer, at, c, width);
        if col + w > width && col > 0 {
            match breakable {
                Some((after, at_col)) if after > row_start && after <= at => {
                    rows.push(Row {
                        range: row_start..after,
                        ends_line: false,
                    });
                    row_start = after;
                    col -= at_col;
                }
                _ => {
                    rows.push(Row {
                        range: row_start..at,
                        ends_line: false,
                    });
                    row_start = at;
                    col = 0;
                }
            }
            breakable = None;
        }
        col += w;
        if cluster::class(c) == Class::Space {
            breakable = Some((at + c.len(), col));
        }
    }
    rows.push(Row {
        range: row_start..line.end,
        ends_line: true,
    });
}

/// The byte in `row` nearest column `goal` without passing it, on a cluster
/// boundary. A row that wraps never gives its end: that place is the start
/// of the next row.
pub(crate) fn offset_at(buffer: &Buffer, row: &Row, goal: usize, width: usize) -> usize {
    let text = buffer.text();
    let mut col = 0;
    let mut best = row.range.start;
    for (offset, c) in cluster::clusters(&text[row.range.clone()]) {
        let at = row.range.start + offset;
        let w = cluster_width(buffer, at, c, width);
        if col + w > goal {
            return at;
        }
        col += w;
        best = at + c.len();
    }
    if !row.ends_line && best == row.range.end {
        return cluster::prev(text, best).max(row.range.start);
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::buffer::Fragment;

    fn buffer(text: &str) -> Buffer {
        let mut buffer = Buffer::default();
        buffer.replace(0..0, Fragment::text(text));
        buffer
    }

    fn rows(text: &str, width: usize) -> Vec<String> {
        let b = buffer(text);
        layout(&b, width, 0)
            .rows
            .iter()
            .map(|row| text[row.range.clone()].to_owned())
            .collect()
    }

    #[test]
    fn rows_break_after_white_space_and_keep_every_byte() {
        assert_eq!(rows("fix the lexer now", 8), ["fix the ", "lexer ", "now"]);
        assert_eq!(rows("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(rows("a\n\nb", 4), ["a", "", "b"]);
        assert_eq!(rows("", 4), [""]);
    }

    #[test]
    fn a_wide_character_never_straddles_a_row() {
        assert_eq!(rows("ab中文", 3), ["ab", "中", "文"]);
    }

    #[test]
    fn the_cursor_after_a_full_row_that_ends_a_line_is_in_the_padding_column() {
        let b = buffer("abcd");
        let at_end = layout(&b, 4, 4);
        assert_eq!((at_end.cursor_row, at_end.cursor_col), (0, 4));
        let b = buffer("abcdef");
        // At the wrap: the start of the next row, never column 4 of the
        // first.
        let wrapped = layout(&b, 4, 4);
        assert_eq!((wrapped.cursor_row, wrapped.cursor_col), (1, 0));
    }
}
