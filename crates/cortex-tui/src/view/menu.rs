//! The menus (`docs/architecture/09-tui.md`, "Dialogs and menus"): file
//! completion, the command menu and the pickers, and the history search.
//! They open below the input box, in place of the hint row.
//!
//! ```text
//!     opus-5.5      anthropic   200k  $5 · $25 /M    fast  current
//!   ◆ sonnet-5.5    anthropic   200k  $3 · $15 /M
//!     haiku-5.5     anthropic   200k  $1 · $5 /M
//! ```
//!
//! At most [`ROWS`](crate::complete::ROWS) rows, those in view. The
//! selected row is marked with `◆` in the accent and its label drawn as
//! the selection; every row's note is muted. Labels are padded to the
//! widest in view, so the notes line up. A picker's toggle row comes after
//! a blank line: its key, its state, and its note (`tab - fast mode  on  ·
//! …`).

use cortex_term::{Line, Span};

use crate::complete::{Empty, Kind, Menu};
use crate::input::Input;
use crate::keys::{self, Action};
use crate::layout::{cut, pad};
use crate::theme::Token;
use crate::view::Look;

/// Columns before a row's mark.
const INDENT: usize = 2;
/// Columns between a label and its note.
const GAP: usize = 2;

/// The menu or the history search that takes the hint row's place, if one
/// is open: its lines, each at most `width` columns.
#[must_use]
pub fn menu(input: &Input, look: &Look, width: usize) -> Option<Vec<Line>> {
    if let Some(search) = input.search() {
        return Some(vec![search_line(input, search.query(), look, width)]);
    }
    let menu = input.menu()?;
    let mut lines = Vec::new();
    if matches!(menu.kind, Kind::Picker(_)) && !menu.filter.is_empty() {
        lines.push(fit(
            Line::from(Span::plain(" ".repeat(INDENT + 2))).with(Span::new(
                super::one_line(&menu.filter),
                look.theme.style(Token::Hint),
            )),
            width,
            look,
        ));
    }
    if menu.rows.is_empty() {
        lines.push(fit(
            Line::from(Span::plain(" ".repeat(INDENT + 2))).with(Span::new(
                empty_text(menu, look),
                look.theme.style(Token::Hint),
            )),
            width,
            look,
        ));
    } else {
        rows(menu, look, width, &mut lines);
    }
    if let Some(toggle) = &menu.toggle {
        // The key that toggles it, if one is bound.
        let key = input
            .keymap()
            .keys_of(Action::ToggleFast)
            .first()
            .map_or_else(String::new, |key| format!("{} - ", keys::name(*key)));
        let (state, state_style) = if toggle.on {
            ("on", Token::AccentText)
        } else {
            ("off", Token::Hint)
        };
        let mut line = Line::from(Span::plain(" ".repeat(INDENT + 2)))
            .with(Span::new(
                format!("{key}{}", super::one_line(&toggle.label)),
                look.theme.style(Token::Key),
            ))
            .with(Span::plain(" ".repeat(GAP)))
            .with(Span::new(state, look.theme.style(state_style)));
        if !toggle.note.is_empty() {
            line.push(Span::plain(" ".repeat(GAP)));
            line.push(Span::new(
                super::one_line(&toggle.note),
                look.theme.style(Token::Hint),
            ));
        }
        lines.push(Line::new());
        lines.push(fit(line, width, look));
    }
    Some(lines)
}

fn rows(menu: &Menu, look: &Look, width: usize, lines: &mut Vec<Line>) {
    let visible = menu.visible();
    let labels: Vec<String> = visible
        .clone()
        .map(|at| super::one_line(&menu.rows[at].label))
        .collect();
    // The label column: the widest label in view, leaving the notes at
    // least a little room.
    let widest = labels
        .iter()
        .map(|label| cortex_base::text::width(label))
        .max()
        .unwrap_or(0);
    let column = widest.min(width.saturating_sub(INDENT + 2 + GAP + 8).max(1));
    for (label, at) in labels.into_iter().zip(visible) {
        let row = &menu.rows[at];
        let selected = menu.selected() == Some(at);
        let mut line = Line::from(Span::plain(" ".repeat(INDENT)));
        if selected {
            line.push(Span::new(look.glyphs.user, look.theme.style(Token::Accent)));
            line.push(Span::plain(" "));
        } else {
            line.push(Span::plain("  "));
        }
        let label_style = if selected {
            look.theme.style(Token::Selected)
        } else {
            look.theme.style(Token::Text)
        };
        let label = cut(
            &Line::styled(label, label_style),
            column,
            look.glyphs.ellipsis,
        );
        let label = if row.detail.is_empty() {
            label
        } else {
            pad(label, column, cortex_term::Style::PLAIN)
        };
        for span in label.spans() {
            line.push(span.clone());
        }
        if !row.detail.is_empty() {
            line.push(Span::plain(" ".repeat(GAP)));
            line.push(Span::new(
                super::one_line(&row.detail),
                look.theme.style(Token::Hint),
            ));
        }
        lines.push(fit(line, width, look));
    }
}

/// What a menu with no row says.
fn empty_text(menu: &Menu, look: &Look) -> String {
    match (menu.empty, &menu.kind) {
        (Empty::Listing, _) => format!("listing the workspace's files{}", look.glyphs.ellipsis),
        (Empty::Unlisted, _) => "the workspace's files could not be listed".to_owned(),
        (Empty::NoMatch, Kind::Files { .. }) => "no file matches".to_owned(),
        (Empty::NoMatch, Kind::Commands) => "no command matches".to_owned(),
        (Empty::NoMatch, Kind::Picker(_)) => "nothing matches".to_owned(),
    }
}

/// The history search: what is searched for, and the prompt found.
fn search_line(input: &Input, query: &str, look: &Look, width: usize) -> Line {
    // The key that searches, if one is bound; the search was opened by a
    // command otherwise.
    let key = input
        .keymap()
        .keys_of(Action::HistorySearch)
        .first()
        .map_or_else(|| "search".to_owned(), |key| keys::name(*key));
    let mut line = Line::from(Span::plain(" ".repeat(INDENT)))
        .with(Span::new(key, look.theme.style(Token::Key)))
        .with(Span::plain(" "))
        .with(Span::plain(super::one_line(query)));
    line.push(Span::plain(" ".repeat(GAP)));
    match input.search_found() {
        Some(found) => {
            let found = found.shown();
            let first = found.lines().next().unwrap_or_default();
            let more = if found.contains('\n') {
                look.glyphs.ellipsis
            } else {
                ""
            };
            line.push(Span::new(
                format!("{}{more}", super::one_line(first)),
                look.theme.style(Token::Hint),
            ));
        }
        None if query.is_empty() => {}
        None => line.push(Span::new("no match", look.theme.style(Token::Hint))),
    }
    fit(line, width, look)
}

fn fit(line: Line, width: usize, look: &Look) -> Line {
    cut(&line, width, look.glyphs.ellipsis)
}

#[cfg(test)]
mod tests;
