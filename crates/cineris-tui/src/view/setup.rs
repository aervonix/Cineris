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
//!
//! **A step fits the window** ([`fitted`], since the full review of the
//! third pull request): its rows are a window around the selected one,
//! which is drawn whole, with a dim row for those above and below; the
//! note, the blank and the sentence give way, in that order, only when the
//! selected row would not fit beside them. A selected row taller than the
//! window is cut, and [`Fitted::whole`] says so: the application then
//! takes no key that would accept it.

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

/// A step's lines fitted to a window, and whether its selected row is in
/// them whole (since the full review of the third pull request).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fitted {
    /// Its lines: at most the rows it was given.
    pub lines: Vec<Line>,
    /// Whether the selected row is drawn whole, every line of its detail:
    /// a key that accepts it is taken only then. True when no row is
    /// selected.
    pub whole: bool,
}

/// A step's lines, each at most `width` columns.
#[must_use]
pub fn step(step: &Step, width: usize, look: &Look) -> Vec<Line> {
    fitted(step, width, look, usize::MAX).lines
}

/// A step's lines in at most `room` rows, each at most `width` columns
/// (since the full review of the third pull request). All of them when
/// they fit. When they do not, the rows are a window around the selected
/// one, drawn whole, with a dim row in the place of those above it and of
/// those below (`… 3 more below`); the title stays, and the note, the
/// blank above the rows and the sentence go, in that order, only when the
/// selected row would not fit beside them. A selected row taller than the
/// room is cut, and said not whole.
#[must_use]
pub fn fitted(step: &Step, width: usize, look: &Look, room: usize) -> Fitted {
    // A step reads as prose does: at the prose width, on a wide window.
    let width = width.min(look.prose_width.max(40));
    let theme = &look.theme;
    let title = cut(
        &Line::from(Span::new(look.glyphs.answer, theme.style(Token::Accent)))
            .with(Span::plain(" "))
            .with(Span::new(one_line(&step.title), theme.style(Token::Title)))
            .with(Span::new(
                format!("  {} of {STEPS}", step.number),
                theme.style(Token::Muted),
            )),
        width,
        look.glyphs.ellipsis,
    );
    let sentence = paragraph(&step.sentence, INDENT, width, theme.style(Token::Text));
    let rows = row_lines(step, width, look);
    // A window too narrow for a row's label column and its detail (eleven
    // columns or fewer) gives lines wider than it: each is cut to the
    // width, and that row is not whole (the early review of the third pull
    // request's fixes).
    let rows_whole: Vec<bool> = rows
        .iter()
        .map(|lines| lines.iter().all(|line| line.width() <= width))
        .collect();
    let rows: Vec<Vec<Line>> = rows
        .into_iter()
        .map(|lines| {
            lines
                .into_iter()
                .map(|line| {
                    if line.width() > width {
                        cut(&line, width, look.glyphs.ellipsis)
                    } else {
                        line
                    }
                })
                .collect()
        })
        .collect();
    let selected_fits = step
        .selected
        .and_then(|at| rows_whole.get(at).copied())
        .unwrap_or(true);
    let note = step.note.as_ref().map_or_else(Vec::new, |note| {
        let mut lines = vec![Line::new()];
        lines.extend(paragraph(note, INDENT, width, theme.style(Token::Warning)));
        lines
    });
    let gap = usize::from(!rows.is_empty());
    let all: usize = rows.iter().map(Vec::len).sum();
    if 1 + sentence.len() + gap + all + note.len() <= room {
        let mut lines = vec![title];
        lines.extend(sentence);
        if gap == 1 {
            lines.push(Line::new());
        }
        lines.extend(rows.into_iter().flatten());
        lines.extend(note);
        return Fitted {
            lines,
            whole: selected_fits,
        };
    }
    let selected = step.selected.filter(|at| *at < rows.len());
    let at = selected.unwrap_or(0);
    // The least the rows need: the selected row whole, and the dim rows
    // for what is above and below it.
    let need =
        rows.get(at).map_or(0, Vec::len) + usize::from(at > 0) + usize::from(at + 1 < rows.len());
    // What stays beside the rows, the most first.
    let (with_sentence, with_gap, with_note) = [
        (true, true, true),
        (true, true, false),
        (true, false, false),
        (false, false, false),
    ]
    .into_iter()
    .find(|(with_sentence, with_gap, with_note)| {
        1 + if *with_sentence { sentence.len() } else { 0 }
            + if *with_gap { gap } else { 0 }
            + if *with_note { note.len() } else { 0 }
            + need
            <= room
    })
    .unwrap_or((false, false, false));
    let mut lines = vec![title];
    if with_sentence {
        lines.extend(sentence);
    }
    if with_gap && gap == 1 {
        lines.push(Line::new());
    }
    let list = room.saturating_sub(lines.len() + if with_note { note.len() } else { 0 });
    let (shown, whole) = window(step, &rows, at, list, width, look);
    lines.extend(shown);
    if with_note {
        lines.extend(note);
    }
    lines.truncate(room);
    Fitted {
        lines,
        whole: (whole && selected_fits) || selected.is_none(),
    }
}

/// Columns before a row's label: the selection mark's.
fn selection_width(look: &Look) -> usize {
    cineris_base::text::width(look.glyphs.user) + 1
}

/// The lines of each of a step's rows.
fn row_lines(step: &Step, width: usize, look: &Look) -> Vec<Vec<Line>> {
    let theme = &look.theme;
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
    let selection = selection_width(look);
    let mut all = Vec::with_capacity(step.rows.len());
    for (index, row) in step.rows.iter().enumerate() {
        let mut lines = Vec::new();
        if row.heading {
            let line = Line::plain(" ".repeat(INDENT + selection))
                .with(Span::new(one_line(&row.label), theme.style(Token::Muted)));
            lines.push(cut(&line, width, look.glyphs.ellipsis));
            all.push(lines);
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
        all.push(lines);
    }
    all
}

/// The rows that fit in `room` lines around row `at`, with a dim row in
/// the place of those above them and of those below, and whether row `at`
/// is drawn whole. They grow from `at` a row at a time, above and below in
/// turn, while the next fits.
fn window(
    step: &Step,
    rows: &[Vec<Line>],
    at: usize,
    room: usize,
    width: usize,
    look: &Look,
) -> (Vec<Line>, bool) {
    let count = rows.len();
    if count == 0 {
        return (Vec::new(), true);
    }
    let cost = |from: usize, to: usize| {
        rows[from..to].iter().map(Vec::len).sum::<usize>()
            + usize::from(from > 0)
            + usize::from(to < count)
    };
    let (mut from, mut to) = (at, at + 1);
    let whole = cost(from, to) <= room;
    if whole {
        let (mut up, mut down) = (true, true);
        while up || down {
            up = up && from > 0 && cost(from - 1, to) <= room;
            if up {
                from -= 1;
            }
            down = down && to < count && cost(from, to + 1) <= room;
            if down {
                to += 1;
            }
        }
    }
    let more = |range: std::ops::Range<usize>, side: &str| {
        let hidden = step.rows[range].iter().filter(|row| !row.heading).count();
        let text = if hidden == 0 {
            format!("{} more {side}", look.glyphs.ellipsis)
        } else {
            format!("{} {hidden} more {side}", look.glyphs.ellipsis)
        };
        cut(
            &Line::plain(" ".repeat(INDENT + selection_width(look)))
                .with(Span::new(text, look.theme.style(Token::Muted))),
            width,
            look.glyphs.ellipsis,
        )
    };
    let marks = usize::from(from > 0) + usize::from(to < count);
    let mut lines = Vec::new();
    if from > 0 {
        lines.push(more(0..from, "above"));
    }
    if whole {
        lines.extend(rows[from..to].iter().flatten().cloned());
    } else {
        lines.extend(rows[at].iter().take(room.saturating_sub(marks)).cloned());
    }
    if to < count {
        lines.push(more(to..count, "below"));
    }
    (lines, whole)
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

    /// Ten servers, the seventh selected, and a note.
    fn ten(detail: &str) -> Step {
        Step {
            title: "Imports".to_owned(),
            number: 2,
            sentence: "Pick what to bring in.".to_owned(),
            rows: (0..10)
                .map(|n| {
                    let detail = if n == 6 {
                        detail.to_owned()
                    } else {
                        format!("npx -y server-{n}")
                    };
                    Row::new(0, Mark::Picked(false), format!("server-{n}"), vec![detail])
                })
                .collect(),
            selected: Some(6),
            note: Some("A note under the rows.".to_owned()),
        }
    }

    /// A window too narrow for a row's label column and its detail cuts
    /// the row to the width, and says it is not whole (the early review of
    /// the third pull request's fixes: the detail ran past the edge).
    #[test]
    fn a_window_too_narrow_for_a_rows_detail_is_not_whole() {
        let look = look();
        let shown = ten("npx -y server-6");
        // The detail's column is nine in: the mark, the label cut to one
        // column, the gap.
        for width in [6, 9] {
            let drawn = fitted(&shown, width, &look, usize::MAX);
            assert!(!drawn.whole, "{width}");
            assert_eq!(problems(&drawn.lines, width), Vec::<String>::new());
        }
        assert!(fitted(&shown, 60, &look, usize::MAX).whole);
    }

    /// A step taller than its room keeps its title, its sentence, its note
    /// and its selected row, the rows around it in a window that says what
    /// is above and below; with less room the note goes, then the blank,
    /// then the sentence, before the selected row is cut, which is said
    /// (the full review of the third pull request).
    #[test]
    fn a_step_taller_than_its_room_is_a_window_around_its_selected_row() {
        let look = look();
        let shown = ten("npx -y server-6");
        // The title, the sentence, a blank, ten rows, a blank and the note.
        let full = fitted(&shown, 60, &look, 15);
        assert!(full.whole);
        assert_eq!(full.lines.len(), 15);
        assert!(!text(&full.lines).iter().any(|row| row.contains("more")));

        let one_short = fitted(&shown, 60, &look, 14);
        let rows = text(&one_short.lines);
        assert!(one_short.whole);
        assert_eq!(rows.len(), 14, "{rows:#?}");
        assert_eq!(rows[0], "◇ Imports  2 of 4");
        assert_eq!(rows[1], "  Pick what to bring in.");
        assert!(rows.contains(&"    … 2 more above".to_owned()), "{rows:#?}");
        assert!(rows.iter().any(|row| row.starts_with("  ◆ ☐ server-6")));
        assert_eq!(rows.last().unwrap(), "  A note under the rows.");
        assert_eq!(problems(&one_short.lines, 60), Vec::<String>::new());

        let small = fitted(&shown, 60, &look, 8);
        let rows = text(&small.lines);
        assert!(small.whole);
        assert_eq!(rows.len(), 8, "{rows:#?}");
        assert!(
            rows.iter().any(|row| row.contains("more above")),
            "{rows:#?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("more below")),
            "{rows:#?}"
        );
        assert!(rows.iter().any(|row| row.starts_with("  ◆ ☐ server-6")));

        // A selected row of nine lines: the note, the blank and the
        // sentence go, in that order, before it is cut.
        let tall = ten(&(0..30)
            .map(|n| format!("--flag-{n}"))
            .collect::<Vec<_>>()
            .join(" "));
        let tall_rows = text(&fitted(&tall, 60, &look, usize::MAX).lines);
        let height = tall_rows
            .iter()
            .skip_while(|row| !row.starts_with("  ◆"))
            .take_while(|row| !row.contains("server-7"))
            .count();
        assert!(height >= 5, "{tall_rows:#?}");
        let with = |room: usize| text(&fitted(&tall, 60, &look, room).lines);
        // Title, sentence, blank, two marks, the row, a blank and the note.
        let rows = with(height + 7);
        assert!(rows.iter().any(|row| row.contains("A note")), "{rows:#?}");
        let rows = with(height + 5);
        assert!(!rows.iter().any(|row| row.contains("A note")), "{rows:#?}");
        assert!(rows.contains(&"  Pick what to bring in.".to_owned()));
        assert!(rows.contains(&String::new()), "the blank stays: {rows:#?}");
        let rows = with(height + 4);
        assert!(!rows.contains(&String::new()), "the blank goes: {rows:#?}");
        assert!(rows.contains(&"  Pick what to bring in.".to_owned()));
        let rows = with(height + 3);
        assert!(
            !rows.iter().any(|row| row.contains("Pick what")),
            "{rows:#?}"
        );
        assert!(fitted(&tall, 60, &look, height + 3).whole);
        let cut = fitted(&tall, 60, &look, height + 2);
        assert!(!cut.whole, "the selected row is cut, and said not whole");
        assert_eq!(text(&cut.lines)[0], "◇ Imports  2 of 4");
        assert!(cut.lines.len() <= height + 2);
    }
}
