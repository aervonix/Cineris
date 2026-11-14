//! The session header (`docs/architecture/09-tui.md`, "Elements"): at the
//! start of a session, once, a small box with a dim rounded border holding
//! `◆ Cortex-Harness` and the version; the model, effort, folder and branch;
//! and one dim line on what the project profile holds.

use cortex_term::{Line, Span, Style};

use super::{Look, one_line};
use crate::rounded::RoundedBox;
use crate::theme::Token;

/// What the header shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    /// The program's version.
    pub version: String,
    /// The model's short name.
    pub model: String,
    /// The reasoning level, when the model has one.
    pub effort: Option<String>,
    /// The folder, as the user would write it (`~/src/parser`).
    pub folder: String,
    /// The git branch, when there is one.
    pub branch: Option<String>,
    /// One sentence on what the project profile holds.
    pub profile: Option<String>,
}

/// The widest the header's box is.
pub const HEADER_WIDTH: usize = 64;

/// The header's box, at most [`HEADER_WIDTH`] columns wide.
#[must_use]
pub fn header(header: &Header, width: usize, look: &Look) -> Vec<Line> {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    let title = Line::from(Span::new(look.glyphs.user, theme.style(Token::User)))
        .with(Span::plain(" "))
        .with(Span::new("Cortex-Harness", theme.style(Token::Title)))
        .with(Span::new(format!(" {}", one_line(&header.version)), muted));
    let mut model = one_line(&header.model);
    if let Some(effort) = &header.effort {
        model.push(' ');
        model.push_str(&one_line(effort));
    }
    let mut place = one_line(&header.folder);
    if let Some(branch) = &header.branch {
        place.push_str(" on ");
        place.push_str(&one_line(branch));
    }
    let about = Line::plain(format!("  {model}  {}  {place}", look.glyphs.separator));
    let mut rows = vec![
        (title, Style::PLAIN),
        (Line::new(), Style::PLAIN),
        (about, Style::PLAIN),
    ];
    if let Some(profile) = &header.profile {
        rows.push((
            Line::plain("  ").with(Span::new(one_line(profile), muted)),
            Style::PLAIN,
        ));
    }
    RoundedBox::new(
        width.min(HEADER_WIDTH),
        look.glyphs,
        theme.style(Token::Border),
    )
    .draw(&rows, None)
}
