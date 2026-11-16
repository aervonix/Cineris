//! The input box (`docs/architecture/09-tui.md`, "The live region"): a
//! rounded box with a dim border, holding `◆` and the prompt's text. It
//! grows to [`MAX_ROWS`] rows and then scrolls to keep the cursor in view;
//! empty, it shows a dim placeholder.
//!
//! ```text
//! ╭──────────────────────────────────────────╮
//! │ ◆ also run clippy when that's done       │
//! ╰──────────────────────────────────────────╯
//! ```
//!
//! The text's rows are the editor's own ([`Editor::layout`]) at
//! [`text_width`]: the box's inside less the `◆` and its space. The cursor
//! after a full row that ends a line sits in the box's padding column,
//! never on its edge. A chip is drawn as its label, muted; a tab as four
//! spaces; any other control character as the renderer's visible symbol,
//! and what acts unseen as its escape, in the columns the editor counted
//! for it.

use cortex_term::{Cursor, Line, Span, Style};

use crate::editor::Editor;
use crate::editor::cluster::{self, CHIP, TAB_WIDTH};
use crate::editor::layout::chip_width;
use crate::input::{Input, MAX_ROWS};
use crate::layout::cut;
use crate::rounded::RoundedBox;
use crate::theme::Token;
use crate::view::Look;

/// What the empty input box says.
pub const PLACEHOLDER: &str = "Describe a task, or ask a question";

/// What the empty input box says while a turn runs (screens 1b to 1d).
pub const RUNNING_PLACEHOLDER: &str = "Type to steer this turn";

/// The input box's lines, and where the cursor is in them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBox {
    /// The box: its top border, a line for each row of text in view, and its
    /// bottom border.
    pub lines: Vec<Line>,
    /// The cursor: its row counted from the box's first line, and its
    /// column.
    pub cursor: Cursor,
}

/// The columns the prompt's text wraps at in a box `width` columns wide:
/// the box's inside less the `◆` and its space. What the caller gives
/// [`Input::set_width`].
#[must_use]
pub fn text_width(width: usize) -> usize {
    RoundedBox::new(width, &crate::glyphs::Glyphs::UNICODE, Style::PLAIN)
        .inner()
        .saturating_sub(MARK_WIDTH)
        .max(1)
}

/// The `◆` and the space after it.
const MARK_WIDTH: usize = 2;

/// The input box for `input`, `width` columns wide.
#[must_use]
pub fn input(input: &Input, look: &Look, width: usize) -> InputBox {
    let frame = RoundedBox::new(width, look.glyphs, look.theme.style(Token::Border));
    let wrap = text_width(width);
    let editor = input.editor();
    let mark = Span::new(
        format!("{} ", look.glyphs.user),
        look.theme.style(Token::User),
    );
    let mut lines = vec![frame.top()];
    if editor.is_empty() {
        // While a box or a panel has the focus the box holds its mark alone
        // (screen 4): typing would not reach it.
        let row = if input.focus().on_input() {
            let placeholder = if input.running() {
                RUNNING_PLACEHOLDER
            } else {
                PLACEHOLDER
            };
            Line::from(mark).with(Span::new(placeholder, look.theme.style(Token::Hint)))
        } else {
            Line::from(mark)
        };
        lines.push(frame.row(&row, Style::PLAIN));
        lines.push(frame.bottom(None));
        return InputBox {
            lines,
            cursor: Cursor {
                row: 1,
                col: inside(RoundedBox::TEXT_COLUMN + MARK_WIDTH, width),
            },
        };
    }
    let layout = editor.layout(wrap);
    let scroll = in_view(input.scroll(), layout.cursor_row, layout.rows.len());
    for (index, row) in layout.rows.iter().enumerate().skip(scroll).take(MAX_ROWS) {
        let lead = if index == 0 {
            mark.clone()
        } else {
            Span::plain(" ".repeat(MARK_WIDTH))
        };
        let mut line = Line::from(lead);
        for span in text_spans(editor, row.range.clone(), wrap, look) {
            line.push(span);
        }
        lines.push(frame.row(&line, Style::PLAIN));
    }
    lines.push(frame.bottom(None));
    InputBox {
        lines,
        cursor: Cursor {
            row: 1 + layout.cursor_row - scroll,
            col: inside(
                RoundedBox::TEXT_COLUMN + MARK_WIDTH + layout.cursor_col,
                width,
            ),
        },
    }
}

/// `col` kept inside the box: in one too narrow for its text, never on the
/// right border or past it.
fn inside(col: usize, width: usize) -> usize {
    col.min(width.saturating_sub(2))
}

/// The first row in view: the input's scroll, moved if it would leave the
/// cursor out of view (the caller gave the input another width).
fn in_view(scroll: usize, cursor_row: usize, rows: usize) -> usize {
    let most = rows.saturating_sub(MAX_ROWS);
    let scroll = scroll.min(most);
    if cursor_row < scroll {
        cursor_row
    } else if cursor_row >= scroll + MAX_ROWS {
        cursor_row + 1 - MAX_ROWS
    } else {
        scroll
    }
}

/// The spans of the text in `range`: chips as their labels, tabs as
/// spaces, the rest made safe to show.
fn text_spans(
    editor: &Editor,
    range: std::ops::Range<usize>,
    wrap: usize,
    look: &Look,
) -> Vec<Span> {
    let text = editor.text();
    let mut spans = Vec::new();
    // Each cluster is made safe to show alone, as the editor counts its
    // columns (`cluster::width`): what acts unseen takes its escape's.
    let mut plain = String::new();
    for (offset, piece) in cluster::clusters(&text[range.clone()]) {
        let at = range.start + offset;
        if piece.starts_with(CHIP) {
            if !plain.is_empty() {
                spans.push(Span::plain(std::mem::take(&mut plain)));
            }
            let label = editor
                .chip_at(at)
                .map_or(CHIP.to_string(), |chip| chip.label().to_owned());
            let label = cortex_term::sanitize(&label).into_owned();
            let columns = chip_width(&label, wrap);
            let style = look.theme.style(Token::Hint);
            let drawn = cut(&Line::styled(label, style), columns, look.glyphs.ellipsis);
            spans.extend(drawn.spans().iter().cloned());
        } else if piece == "\t" {
            plain.push_str(&" ".repeat(TAB_WIDTH));
        } else {
            plain.push_str(&cluster::drawn(piece));
        }
    }
    if !plain.is_empty() {
        spans.push(Span::plain(plain));
    }
    spans
}

#[cfg(test)]
mod tests;
