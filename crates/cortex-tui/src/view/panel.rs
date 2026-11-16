//! What every panel draws alike (`docs/architecture/09-tui.md`, "Panels";
//! step 24): a row tagged with the item it belongs to, the dim row that
//! says how many rows a cut list hides, the dim row that stands for the
//! panels folded away, and a panel's first row.
//!
//! A panel is drawn as one block under the transcript's live rows and
//! above the input box. Each panel's own rows are drawn by its view
//! ([`super::plan`], [`super::review`]); the application lays the panels
//! out in the room the live region leaves them.

use cortex_term::{Line, Span};

use super::{Look, one_line};
use crate::layout::cut;
use crate::theme::Token;

/// One row of a panel drawn whole, and the item it belongs to: `None` for a
/// row that belongs to none (its first row, a line that says where the
/// checks are), the item's place counted from 0 otherwise. An item may have
/// more than one row; they come one after another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelRow {
    /// The row.
    pub line: Line,
    /// The item it belongs to.
    pub item: Option<usize>,
}

impl PanelRow {
    /// A row of no item.
    #[must_use]
    pub fn plain(line: Line) -> Self {
        Self { line, item: None }
    }

    /// A row of the item `item`.
    #[must_use]
    pub fn of(item: usize, line: Line) -> Self {
        Self {
            line,
            item: Some(item),
        }
    }
}

/// A panel's first row: `◇`, its title in bold, and what it says of itself
/// muted after two spaces (`◇ Plan  1 of 4 done`, screen 4). The title and
/// the rest are the caller's words; text from outside in them is made one
/// line and visible.
#[must_use]
pub fn header(title: &str, rest: &[Span], width: usize, look: &Look) -> Line {
    let theme = &look.theme;
    let mut line = Line::from(Span::new(look.glyphs.answer, theme.style(Token::Text)))
        .with(Span::plain(" "))
        .with(Span::new(one_line(title), theme.style(Token::Title)));
    for span in rest {
        line.push(span.clone());
    }
    cut(&line, width, look.glyphs.ellipsis)
}

/// The dim row in the place of the rows a focused panel's list cuts at one
/// end: `… 3 more`, two columns in.
#[must_use]
pub fn more(hidden: usize, width: usize, look: &Look) -> Line {
    let line = Line::plain("  ").with(Span::new(
        format!("{} {hidden} more", look.glyphs.ellipsis),
        look.theme.style(Token::Muted),
    ));
    cut(&line, width, look.glyphs.ellipsis)
}

/// The dim row in the place of the oldest panels folded away when there is
/// no room for them: `+2 panels · ctrl+p`, `key` being the key that moves
/// the focus to them, if one is bound.
#[must_use]
pub fn folded(count: usize, key: Option<&str>, width: usize, look: &Look) -> Line {
    let mut text = if count == 1 {
        "+1 panel".to_owned()
    } else {
        format!("+{count} panels")
    };
    if let Some(key) = key {
        text.push_str(&format!(" {} {}", look.glyphs.separator, one_line(key)));
    }
    cut(
        &Line::styled(text, look.theme.style(Token::Muted)),
        width,
        look.glyphs.ellipsis,
    )
}

#[cfg(test)]
mod tests;
