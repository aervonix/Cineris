//! Tables: columns aligned with spaces and a dim rule under the header, or,
//! when that is too wide, stacked `header: value` rows
//! (`docs/architecture/09-tui.md`, "Elements").
//!
//! A table is committed when it ends, because its columns' widths depend on
//! every row; a long one in parts of [`super::TABLE_PART`] rows, each with
//! the widths and the layout decided for the first.

use cineris_term::{Line, Span, Style};

use super::Frame;
use super::block::{Align, split_row};
use super::inline;
use super::rows::piece_style;
use crate::layout::{cut, wrap};
use crate::theme::Token;
use crate::view::Look;

/// A table in progress.
#[derive(Debug, Clone)]
pub(crate) struct Table {
    header: Vec<String>,
    aligns: Vec<Align>,
    /// Rows not yet committed.
    rows: Vec<Vec<String>>,
    /// The layout, once the first part is committed.
    layout: Option<Layout>,
}

#[derive(Debug, Clone)]
enum Layout {
    /// Columns of these widths.
    Columns(Vec<usize>),
    /// `header: value`, one line a cell.
    Stacked,
}

/// A cell, parsed: its text and styles.
type Cell = Vec<Span>;

impl Table {
    pub(crate) fn new(header: &str, aligns: Vec<Align>) -> Self {
        Self {
            header: split_row(header)
                .into_iter()
                .map(|cell| cell.trim().to_owned())
                .collect(),
            aligns,
            rows: Vec::new(),
            layout: None,
        }
    }

    pub(crate) fn row(&mut self, line: &str) {
        let mut cells: Vec<String> = split_row(line)
            .into_iter()
            .map(|cell| cell.trim().to_owned())
            .collect();
        cells.resize(self.header.len(), String::new());
        self.rows.push(cells);
    }

    /// Rows waiting to be committed.
    pub(crate) fn waiting(&self) -> usize {
        self.rows.len()
    }

    /// The rows so far, committed as a part.
    pub(crate) fn part(&mut self, width: usize, prefix: usize, look: &Look) -> Vec<Line> {
        self.draw(width, prefix, look)
    }

    /// The rest of the table.
    pub(crate) fn finish(&mut self, width: usize, prefix: usize, look: &Look) -> Vec<Line> {
        self.draw(width, prefix, look)
    }

    fn draw(&mut self, width: usize, prefix: usize, look: &Look) -> Vec<Line> {
        let frame = Frame { width, look };
        let room = width.saturating_sub(prefix).max(1);
        let header: Vec<Cell> = self
            .header
            .iter()
            .map(|text| cell(text, frame, true))
            .collect();
        let rows: Vec<Vec<Cell>> = std::mem::take(&mut self.rows)
            .iter()
            .map(|row| row.iter().map(|text| cell(text, frame, false)).collect())
            .collect();
        let first = self.layout.is_none();
        let layout = self.layout.get_or_insert_with(|| {
            let mut widths: Vec<usize> = header.iter().map(cell_width).collect();
            for row in &rows {
                for (column, cell) in row.iter().enumerate() {
                    widths[column] = widths[column].max(cell_width(cell));
                }
            }
            let total = widths.iter().sum::<usize>() + 2 * widths.len().saturating_sub(1);
            if total <= room {
                Layout::Columns(widths)
            } else {
                Layout::Stacked
            }
        });
        let mut lines = Vec::new();
        match layout {
            Layout::Columns(widths) => {
                if first {
                    lines.push(columns(&header, widths, &self.aligns, look));
                    let total = widths.iter().sum::<usize>() + 2 * widths.len().saturating_sub(1);
                    lines.push(Line::styled(
                        look.glyphs.horizontal.repeat(total),
                        look.theme.style(Token::Border),
                    ));
                }
                for row in &rows {
                    lines.push(cut(
                        &columns(row, widths, &self.aligns, look),
                        room,
                        look.glyphs.ellipsis,
                    ));
                }
            }
            Layout::Stacked => {
                // A table with only a header: its names, a row each, so that
                // nothing it says is dropped.
                if first && rows.is_empty() {
                    for name in &header {
                        lines.extend(wrap(name, room));
                    }
                }
                for (index, row) in rows.iter().enumerate() {
                    if index > 0 || !first {
                        lines.push(Line::new());
                    }
                    for (name, value) in header.iter().zip(row) {
                        let mut spans: Vec<Span> = name.clone();
                        spans.push(Span::new(": ", look.theme.style(Token::Title)));
                        spans.extend(value.iter().cloned());
                        lines.extend(wrap(&spans, room));
                    }
                }
            }
        }
        lines
    }
}

/// A cell's text, parsed and styled: bold in the header.
fn cell(text: &str, frame: Frame<'_>, header: bool) -> Cell {
    let base = if header {
        frame.look.theme.style(Token::Title)
    } else {
        Style::PLAIN
    };
    let pieces = inline::parse(text, &[]).pieces;
    let mut cell = Vec::with_capacity(pieces.len());
    // The text of the link being drawn, to say whether it is the address.
    let mut shown = String::new();
    for (at, piece) in pieces.iter().enumerate() {
        let span = Span::new(
            piece.text.replace('\n', " "),
            piece_style(piece, base, frame, true),
        );
        let Some(url) = &piece.link else {
            cell.push(span);
            continue;
        };
        shown.push_str(span.text());
        cell.push(span.link(url.clone()));
        // Where the terminal has no links, or the address may not be one,
        // the address follows the link's text, as in a paragraph.
        let last = pieces
            .get(at + 1)
            .is_none_or(|next| next.link != piece.link);
        if last {
            if !(frame.look.hyperlinks && cineris_term::line::linkable(url)) && shown != *url {
                cell.push(Span::new(
                    format!(" ({url})"),
                    frame.look.theme.style(Token::Muted),
                ));
            }
            shown.clear();
        }
    }
    cell
}

fn cell_width(cell: &Cell) -> usize {
    cell.iter().map(Span::width).sum()
}

/// One row of aligned columns, two spaces apart, with no spaces at its
/// end.
fn columns(cells: &[Cell], widths: &[usize], aligns: &[Align], look: &Look) -> Line {
    let mut line: Vec<Span> = Vec::new();
    for (column, width) in widths.iter().enumerate() {
        if column > 0 {
            line.push(Span::plain("  "));
        }
        let cell = cells.get(column).cloned().unwrap_or_default();
        let shown = cut(&crate::layout::join(&cell), *width, look.glyphs.ellipsis);
        let pad = width.saturating_sub(shown.width());
        let (before, after) = match aligns.get(column).copied().unwrap_or(Align::Left) {
            Align::Left => (0, pad),
            Align::Right => (pad, 0),
            Align::Center => (pad / 2, pad - pad / 2),
        };
        line.push(Span::plain(" ".repeat(before)));
        line.extend(shown.spans().iter().cloned());
        line.push(Span::plain(" ".repeat(after)));
    }
    while line
        .last()
        .is_some_and(|span| span.style() == Style::PLAIN && span.text().trim().is_empty())
    {
        line.pop();
    }
    Line::from(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::Glyphs;
    use crate::theme::Theme;

    fn look() -> Look {
        Look::new(
            Theme::new(cineris_term::ColorDepth::TrueColor, None),
            &Glyphs::UNICODE,
        )
    }

    fn texts(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|line| line.text().into_owned()).collect()
    }

    #[test]
    fn columns_are_aligned_with_a_rule_under_the_header() {
        let look = look();
        let mut table = Table::new(
            "| name | ctx | price |",
            vec![Align::Left, Align::Right, Align::Center],
        );
        table.row("| opus | 200k | $5 |");
        table.row("| `haiku-5.5` | 1m | $1 |");
        let lines = table.finish(80, 2, &look);
        assert_eq!(
            texts(&lines),
            [
                "name        ctx  price",
                "──────────────────────",
                "opus       200k   $5",
                "haiku-5.5    1m   $1",
            ]
        );
        // The header is bold, inline code keeps its colour.
        assert!(lines[0].spans()[0].style().bold);
        assert!(
            lines[3]
                .spans()
                .iter()
                .any(|span| span.text() == "haiku-5.5"
                    && span.style().fg != cineris_term::Color::Default)
        );
    }

    #[test]
    fn a_table_too_wide_is_stacked() {
        let look = look();
        let mut table = Table::new("| key | value |", vec![Align::Left, Align::Left]);
        table.row("| model | a long description of the model that does not fit |");
        table.row("| speed | fast |");
        let lines = table.finish(30, 2, &look);
        assert_eq!(
            texts(&lines),
            [
                "key: model",
                "value: a long description of",
                "the model that does not fit",
                "",
                "key: speed",
                "value: fast",
            ]
        );
        assert!(lines.iter().all(|line| line.width() <= 28));
    }

    #[test]
    fn later_parts_keep_the_first_parts_widths() {
        let look = look();
        let mut table = Table::new("| a | b |", vec![Align::Left, Align::Left]);
        table.row("| 1 | 2 |");
        let first = table.part(80, 2, &look);
        assert_eq!(texts(&first), ["a  b", "────", "1  2"]);
        table.row("| wide | 3 |");
        let second = table.finish(80, 2, &look);
        assert_eq!(texts(&second), ["…  3"]);
    }
}
