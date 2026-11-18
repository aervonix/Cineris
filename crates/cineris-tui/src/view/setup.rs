//! The first run's views (since step 25; `docs/architecture/09-tui.md`,
//! "The first run"; the screens F1a to F4 in `docs/architecture/tui-look/`).
//!
//! ```text
//! ╭──────────────────────────────────────────────────────────────╮
//! │ ◆ Cineris 0.1.0                                       │
//! │                                                              │
//! │   Setting up, in four short steps. Each one can be skipped,  │
//! │   and x skips them all.                                      │
//! ╰──────────────────────────────────────────────────────────────╯
//!
//! ◇ Mode  3 of 4
//!   How much the agent does before it asks you. /mode changes it at any time.
//!
//!     read-only      reads, and runs only commands known to read; asks before
//!                    any other, and never writes
//!   ◆ supervised     reads and edits files in this project without asking; …
//! ```
//!
//! The set-up card ([`card`]) stays in the live region while the steps
//! before the session run, and is never committed. Each step ([`step`])
//! takes the input box's place, as a dialog does: its title and count, a
//! sentence, and its rows, each a mark, a label and what it means, wrapped
//! and never cut. Every label and every text from outside (a provider's
//! name, a file's path, a server's command, a command found in a project's
//! files) passes through `cineris_term::sanitize`.

use cineris_term::{Line, Span, Style};

use super::{Look, one_line};
use crate::layout::{cut, wrap};
use crate::rounded::RoundedBox;
use crate::theme::Token;

/// The widest the set-up card is: as the session's header.
pub const CARD_WIDTH: usize = super::header::HEADER_WIDTH;

/// The steps, in all.
pub const STEPS: usize = 4;

/// The set-up card: the program and its version, and what is about to
/// happen.
#[must_use]
pub fn card(version: &str, width: usize, look: &Look) -> Vec<Line> {
    let theme = &look.theme;
    let frame = RoundedBox::new(
        width.min(CARD_WIDTH),
        look.glyphs,
        theme.style(Token::Border),
    );
    let title = Line::from(Span::new(look.glyphs.user, theme.style(Token::User)))
        .with(Span::plain(" "))
        .with(Span::new("Cineris", theme.style(Token::Title)))
        .with(Span::new(
            format!(" {}", one_line(version)),
            theme.style(Token::Muted),
        ));
    let mut rows = vec![(title, Style::PLAIN), (Line::new(), Style::PLAIN)];
    let about = "Setting up, in four short steps. Each one can be skipped, and x skips them all.";
    for line in wrap(
        &[Span::plain(about)],
        frame.inner().saturating_sub(2).max(1),
    ) {
        rows.push((Line::plain("  ").with_line(&line), Style::PLAIN));
    }
    frame.draw(&rows, None)
}

/// A row's mark before its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// No mark: a choice of one (a provider, a mode).
    None,
    /// A box, picked or not.
    Picked(bool),
    /// A row that opens, open or not.
    Folded(bool),
    /// The mark's room left blank: a row that cannot be picked.
    Blank,
}

/// One row of a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// How far in it is: 0, or 1 under a row that opens.
    pub depth: usize,
    /// Its mark.
    pub mark: Mark,
    /// Its label: a name, a path, a level.
    pub label: String,
    /// What it means, each a paragraph wrapped after the label, never cut.
    pub detail: Vec<String>,
    /// A heading of the rows under it, which is never selected.
    pub heading: bool,
}

impl Row {
    /// A row with a label and a detail.
    #[must_use]
    pub fn new(depth: usize, mark: Mark, label: impl Into<String>, detail: Vec<String>) -> Self {
        Self {
            depth,
            mark,
            label: label.into(),
            detail,
            heading: false,
        }
    }

    /// A heading.
    #[must_use]
    pub fn heading(label: impl Into<String>) -> Self {
        Self {
            depth: 0,
            mark: Mark::None,
            label: label.into(),
            detail: Vec::new(),
            heading: true,
        }
    }
}

/// What a step shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Step {
    /// Its name: `Provider`.
    pub title: String,
    /// Which step it is, from 1.
    pub number: usize,
    /// What it asks, and what a choice does.
    pub sentence: String,
    /// Its rows.
    pub rows: Vec<Row>,
    /// The row selected, by its index in `rows`.
    pub selected: Option<usize>,
    /// A sentence under the rows: `reading…`, why something was refused.
    pub note: Option<String>,
}

/// Columns before a row's selection mark.
const INDENT: usize = 2;
/// Columns between a label and its detail.
const GAP: usize = 2;
/// The fewest columns a detail wraps at: past it, the label is cut.
const DETAIL_MIN: usize = 24;

/// A step's lines, each at most `width` columns.
#[must_use]
pub fn step(step: &Step, width: usize, look: &Look) -> Vec<Line> {
    // A step reads as prose does: at the prose width, on a wide window.
    let width = width.min(look.prose_width.max(40));
    let theme = &look.theme;
    let mut lines = vec![cut(
        &Line::from(Span::new(look.glyphs.answer, theme.style(Token::Accent)))
            .with(Span::plain(" "))
            .with(Span::new(one_line(&step.title), theme.style(Token::Title)))
            .with(Span::new(
                format!("  {} of {STEPS}", step.number),
                theme.style(Token::Muted),
            )),
        width,
        look.glyphs.ellipsis,
    )];
    lines.extend(paragraph(
        &step.sentence,
        INDENT,
        width,
        theme.style(Token::Text),
    ));
    if !step.rows.is_empty() {
        lines.push(Line::new());
    }
    let mark_width = [
        look.glyphs.plan_todo,
        look.glyphs.plan_done,
        look.glyphs.folded,
        look.glyphs.unfolded,
    ]
    .iter()
    .map(|glyph| cineris_base::text::width(glyph))
    .max()
    .unwrap_or(1)
        + 1;
    let selection = cineris_base::text::width(look.glyphs.user) + 1;
    for (index, row) in step.rows.iter().enumerate() {
        if row.heading {
            let line = Line::plain(" ".repeat(INDENT + selection))
                .with(Span::new(one_line(&row.label), theme.style(Token::Muted)));
            lines.push(cut(&line, width, look.glyphs.ellipsis));
            continue;
        }
        let prefix = |row: &Row| {
            INDENT
                + selection
                + row.depth * 2
                + if row.mark == Mark::None {
                    0
                } else {
                    mark_width
                }
        };
        // The label's column: the widest label of the rows of this depth
        // and kind, leaving the detail its least.
        let widest = step
            .rows
            .iter()
            .filter(|other| !other.heading && other.depth == row.depth)
            .map(|other| cineris_base::text::width(&one_line(&other.label)))
            .max()
            .unwrap_or(0);
        let start = prefix(row);
        let room = width.saturating_sub(start + GAP + DETAIL_MIN).max(1);
        let column = widest.min(room);
        let selected = step.selected == Some(index);
        let mut line = Line::plain(" ".repeat(INDENT));
        if selected {
            line.push(Span::new(look.glyphs.user, theme.style(Token::Accent)));
            line.push(Span::plain(" "));
        } else {
            line.push(Span::plain(" ".repeat(selection)));
        }
        line.push(Span::plain(" ".repeat(row.depth * 2)));
        let glyph = match row.mark {
            Mark::None => None,
            Mark::Picked(false) => Some((look.glyphs.plan_todo, Token::Muted)),
            Mark::Picked(true) => Some((look.glyphs.plan_done, Token::Accent)),
            Mark::Folded(false) => Some((look.glyphs.folded, Token::Muted)),
            Mark::Folded(true) => Some((look.glyphs.unfolded, Token::Muted)),
            Mark::Blank => Some(("", Token::Muted)),
        };
        if let Some((glyph, token)) = glyph {
            line.push(Span::new(glyph, theme.style(token)));
            line.push(Span::plain(
                " ".repeat(mark_width - cineris_base::text::width(glyph)),
            ));
        }
        let label_style = if selected {
            theme.style(Token::Selected)
        } else {
            theme.style(Token::Text)
        };
        let label = cut(
            &Line::styled(one_line(&row.label), label_style),
            column,
            look.glyphs.ellipsis,
        );
        let label_width = label.width();
        line = line.with_line(&label);
        let detail_column = start + column + GAP;
        let detail_width = width.saturating_sub(detail_column).max(1);
        let mut details = row
            .detail
            .iter()
            .flat_map(|text| wrap(&[Span::plain(one_line(text))], detail_width));
        match details.next() {
            Some(first) => {
                line.push(Span::plain(" ".repeat(column - label_width + GAP)));
                for span in first.spans() {
                    line.push(Span::new(span.text().to_owned(), theme.style(Token::Muted)));
                }
                lines.push(line);
            }
            None => lines.push(line),
        }
        for rest in details {
            let mut more = Line::plain(" ".repeat(detail_column));
            for span in rest.spans() {
                more.push(Span::new(span.text().to_owned(), theme.style(Token::Muted)));
            }
            lines.push(more);
        }
    }
    if let Some(note) = &step.note {
        lines.push(Line::new());
        lines.extend(paragraph(note, INDENT, width, theme.style(Token::Warning)));
    }
    lines
}

/// `text` wrapped `indent` columns in, every line in `style`.
fn paragraph(text: &str, indent: usize, width: usize, style: Style) -> Vec<Line> {
    if text.is_empty() {
        return Vec::new();
    }
    wrap(
        &[Span::new(one_line(text), style)],
        width.saturating_sub(indent).max(1),
    )
    .into_iter()
    .map(|line| Line::plain(" ".repeat(indent)).with_line(&line))
    .collect()
}

/// A line followed by another's spans.
trait WithLine {
    fn with_line(self, other: &Line) -> Self;
}

impl WithLine for Line {
    fn with_line(mut self, other: &Line) -> Self {
        for span in other.spans() {
            self.push(span.clone());
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TRICKY, problems};

    fn look() -> Look {
        Look::new(
            crate::theme::Theme::new(cineris_term::ColorDepth::TrueColor, None),
            &crate::glyphs::Glyphs::UNICODE,
        )
    }

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.text().trim_end().to_owned())
            .collect()
    }

    #[test]
    fn a_rows_detail_wraps_at_its_column_and_is_never_cut() {
        let command = "docker run -i --rm -e GITHUB_TOKEN=[redacted] ghcr.io/example/github-mcp:1.4 --and-more-arguments";
        let shown = step(
            &Step {
                title: "Imports".to_owned(),
                number: 2,
                sentence: "Pick what to bring in.".to_owned(),
                rows: vec![
                    Row::heading("MCP servers"),
                    Row::new(
                        1,
                        Mark::Picked(true),
                        "docs",
                        vec!["npx -y docs".to_owned()],
                    ),
                    Row::new(
                        1,
                        Mark::Blank,
                        "github",
                        vec![
                            command.to_owned(),
                            "holds a credential: not imported".to_owned(),
                        ],
                    ),
                ],
                selected: Some(1),
                note: None,
            },
            60,
            &look(),
        );
        let rows = text(&shown);
        assert_eq!(rows[0], "◇ Imports  2 of 4");
        assert_eq!(rows[3], "    MCP servers");
        assert_eq!(rows[4], "  ◆   ☑ docs    npx -y docs");
        assert!(
            rows[5].starts_with("        github  docker run"),
            "{rows:#?}"
        );
        let joined: String = rows[5..]
            .iter()
            .map(|row| row.trim())
            .collect::<Vec<_>>()
            .join(" ");
        for word in command.split(' ') {
            assert!(joined.contains(word), "{word} was cut: {rows:#?}");
        }
        assert_eq!(problems(&shown, 60), Vec::<String>::new());
    }

    #[test]
    fn hostile_labels_and_details_are_drawn_visible_and_fit() {
        for (label, detail) in TRICKY.iter().zip(TRICKY.iter().rev()) {
            let lines = step(
                &Step {
                    title: (*label).to_owned(),
                    number: 1,
                    sentence: format!("about {detail}"),
                    rows: vec![
                        Row::new(0, Mark::None, *label, vec![(*detail).repeat(9)]),
                        Row::new(1, Mark::Folded(false), *detail, vec![(*label).to_owned()]),
                        Row::heading(*label),
                    ],
                    selected: Some(0),
                    note: Some((*label).to_owned()),
                },
                40,
                &look(),
            );
            assert_eq!(problems(&lines, 40), Vec::<String>::new(), "{label:?}");
        }
        let card = card("0.1.0\u{1b}[2J\u{202e}", 30, &look());
        assert_eq!(problems(&card, 30), Vec::<String>::new());
    }

    #[test]
    fn the_card_says_what_is_about_to_happen() {
        let rows = text(&card("0.1.0", 99, &look()));
        assert_eq!(rows.len(), 6);
        assert!(rows[1].contains("◆ Cineris 0.1.0"));
        assert!(
            rows[3].contains("Setting up, in four short steps."),
            "{rows:#?}"
        );
        assert!(rows[4].contains("x skips them all."), "{rows:#?}");
    }
}
