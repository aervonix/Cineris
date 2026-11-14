//! Inline pieces laid out in rows of a width, with the way back from a place
//! in a row to the source it was drawn from.

use std::ops::Range;

use cortex_base::text::wrap_ranges;
use cortex_term::{Line, Span, Style};

use super::inline::Piece;
use super::{Flavor, Frame};
use crate::theme::Token;

/// One piece as laid out: where it is in the shown text.
#[derive(Debug, Clone)]
struct Placed {
    piece: Piece,
    /// Where its text is in [`Laid::text`].
    at: Range<usize>,
    /// Text the layout added (a link's address, where the terminal has no
    /// links), which no source byte stands behind.
    added: bool,
}

/// A paragraph's pieces laid out at a width.
#[derive(Debug, Clone)]
pub(crate) struct Laid {
    text: String,
    placed: Vec<Placed>,
    /// Each row's bytes of `text`.
    pub(crate) rows: Vec<Range<usize>>,
}

impl Laid {
    pub(crate) fn new(pieces: Vec<Piece>, width: usize, frame: Frame<'_>) -> Self {
        let mut text = String::new();
        let mut placed: Vec<Placed> = Vec::with_capacity(pieces.len());
        let count = pieces.len();
        let mut pieces = pieces.into_iter().peekable();
        let mut index = 0;
        while let Some(piece) = pieces.next() {
            index += 1;
            let start = text.len();
            text.push_str(&piece.text);
            let link = piece.link.clone();
            let end_of_link = link.is_some() && pieces.peek().is_none_or(|next| next.link != link);
            let src_end = piece.src.end;
            placed.push(Placed {
                at: start..text.len(),
                piece,
                added: false,
            });
            // Where the terminal has no links, the address follows the
            // link's text, unless the text is the address.
            if end_of_link
                && !frame.look.hyperlinks
                && let Some(url) = link
            {
                let shown_text: String = placed
                    .iter()
                    .rev()
                    .take_while(|p| p.piece.link.as_deref() == Some(url.as_str()) && !p.added)
                    .map(|p| p.piece.text.as_str())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                if shown_text != url {
                    let start = text.len();
                    text.push_str(" (");
                    text.push_str(&url);
                    text.push(')');
                    placed.push(Placed {
                        at: start..text.len(),
                        piece: Piece {
                            text: text[start..].to_owned(),
                            marks: super::inline::Marks::default(),
                            link: None,
                            src: src_end..src_end,
                            linear: false,
                            depth: 0,
                        },
                        added: true,
                    });
                }
            }
        }
        debug_assert_eq!(index, count);
        let rows = wrap_ranges(&text, width);
        Self { text, placed, rows }
    }

    /// The piece that holds the shown byte `at`.
    fn piece_at(&self, at: usize) -> Option<&Placed> {
        let index = self.placed.partition_point(|placed| placed.at.end <= at);
        self.placed
            .get(index)
            .filter(|placed| placed.at.contains(&at))
    }

    /// The source byte just after the shown byte `at - 1`: where the source
    /// stands behind the end of what is shown before `at`.
    fn source_after(&self, at: usize) -> usize {
        if at == 0 {
            return 0;
        }
        match self.piece_at(at - 1) {
            Some(placed) if placed.piece.linear => placed.piece.src.start + (at - placed.at.start),
            Some(placed) => placed.piece.src.end,
            None => self.placed.last().map_or(0, |placed| placed.piece.src.end),
        }
    }

    /// The source byte where the first word of row `row` ends, or the row
    /// does, if the word goes on past it: a word too long for a row is
    /// broken where the row is full, so no mark after the row can move the
    /// break before it.
    pub(crate) fn first_word_end(&self, row: usize) -> usize {
        let Range {
            start,
            end: row_end,
        } = self.rows[row].clone();
        let end = self.text[start..]
            .char_indices()
            .find(|&(_, c)| c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{2007}' | '\u{202f}'))
            .map_or(self.text.len(), |(offset, _)| start + offset);
        self.source_after(end.min(row_end))
    }

    /// Where the source may be cut at the break before row `row`: nothing
    /// spans it, so the text after it parses alone as it parsed here.
    /// `None` when something does. The caller checks that no mark before
    /// the break is still open.
    pub(crate) fn clean_break(&self, row: usize) -> Option<usize> {
        if row == 0 || row >= self.rows.len() {
            return None;
        }
        let gap = self.rows[row - 1].end..self.rows[row].start;
        let plain = |placed: &Placed| {
            placed.piece.linear
                && placed.piece.depth == 0
                && placed.piece.link.is_none()
                && !placed.added
        };
        if gap.is_empty() {
            // A break inside a run of text (wide characters, or a word too
            // long for a row). Everything before it is closed or literal, so
            // only what follows matters: plain text starting with a character
            // that can neither open nor close anything parses alone as it
            // parsed here, whatever came before it.
            let after = self.piece_at(gap.start)?;
            let special = |c: Option<char>| {
                c.is_none_or(|c| {
                    matches!(
                        c,
                        '*' | '_' | '`' | '[' | ']' | '!' | '<' | '>' | '&' | '\\'
                    )
                })
            };
            if !plain(after) || special(self.text[gap.start..].chars().next()) {
                return None;
            }
            return Some(after.piece.src.start + (gap.start - after.at.start));
        }
        let mut at = gap.start;
        while at < gap.end {
            let placed = self.piece_at(at)?;
            let hard_break = placed.piece.text == "\n" && placed.piece.depth == 0 && !placed.added;
            if !(plain(placed) || hard_break) {
                return None;
            }
            at = placed.at.end.min(gap.end);
        }
        Some(self.source_after(gap.end))
    }

    /// Where the source may be cut at the break before row `row` when no
    /// clean break has come for too long. Without `inside`, only where the
    /// row starts outside every construct: nothing spans the break, though a
    /// mark just after it may read its neighbour differently than it did
    /// here. With `inside`, also within one, whose marks the text after the
    /// break then shows as they are. `None` where the row's start stands for
    /// no one source byte.
    pub(crate) fn forced_break(&self, row: usize, inside: bool) -> Option<usize> {
        if row == 0 || row >= self.rows.len() {
            return None;
        }
        let start = self.rows[row].start;
        let after = self.piece_at(start)?;
        if after.added || (!inside && (after.piece.depth != 0 || after.piece.link.is_some())) {
            return None;
        }
        if after.piece.linear {
            Some(after.piece.src.start + (start - after.at.start))
        } else if start == after.at.start {
            Some(after.piece.src.start)
        } else {
            None
        }
    }

    /// Lays the rows out again: `fixed` as they are, and the text from the
    /// shown byte `resume` on at `width`. For a paragraph some of whose rows
    /// were committed at another width: they keep the breaks they were
    /// drawn with, and what follows them is wrapped at the width now.
    pub(crate) fn refit(&mut self, fixed: &[Range<usize>], resume: usize, width: usize) {
        let resume = resume.min(self.text.len());
        let mut rows = fixed.to_vec();
        if resume < self.text.len() {
            rows.extend(
                wrap_ranges(&self.text[resume..], width)
                    .into_iter()
                    .map(|row| row.start + resume..row.end + resume),
            );
        }
        self.rows = rows;
    }

    /// Where row `row` starts in the shown text, or its end past the last.
    pub(crate) fn row_start(&self, row: usize) -> usize {
        self.rows.get(row).map_or(self.text.len(), |row| row.start)
    }

    /// Row `row`, drawn: answer text, or a reasoning summary's.
    pub(crate) fn line(&self, row: usize, frame: Frame<'_>, flavor: Flavor) -> Line {
        let base = match flavor {
            Flavor::Answer => Style::PLAIN,
            Flavor::Reasoning => frame.look.theme.style(Token::Reasoning),
        };
        self.draw(row, frame, base, flavor == Flavor::Answer)
    }

    /// Row `row`, drawn over `base`, as a heading is.
    pub(crate) fn styled(&self, row: usize, frame: Frame<'_>, base: Style) -> Line {
        self.draw(row, frame, base, true)
    }

    fn draw(&self, row: usize, frame: Frame<'_>, base: Style, code_colour: bool) -> Line {
        let range = self.rows[row].clone();
        let mut line = Line::new();
        let first = self
            .placed
            .partition_point(|placed| placed.at.end <= range.start);
        for placed in &self.placed[first..] {
            if placed.at.start >= range.end {
                break;
            }
            let from = placed.at.start.max(range.start);
            let to = placed.at.end.min(range.end);
            if from >= to {
                continue;
            }
            let text = &self.text[from..to];
            let style = if placed.added {
                frame.look.theme.style(Token::Muted)
            } else {
                piece_style(&placed.piece, base, frame, code_colour)
            };
            let mut span = Span::new(text, style);
            if let Some(url) = &placed.piece.link {
                span = span.link(url.clone());
            }
            line.push(span);
        }
        line
    }
}

/// The style of a piece of inline text over `base`.
pub(crate) fn piece_style(
    piece: &Piece,
    base: Style,
    frame: Frame<'_>,
    code_colour: bool,
) -> Style {
    let theme = &frame.look.theme;
    let mut style = if piece.marks.code && code_colour {
        let code = theme.style(Token::Code);
        Style {
            fg: code.fg,
            bold: code.bold || base.bold,
            ..base
        }
    } else {
        base
    };
    if piece.marks.bold {
        style = style.bold();
    }
    if piece.marks.italic {
        style = style.italic();
    }
    if piece.link.is_some() {
        let link = theme.style(Token::Link);
        style.underline |= link.underline;
        if link.fg != cortex_term::Color::Default {
            style.fg = link.fg;
        }
    }
    style
}
