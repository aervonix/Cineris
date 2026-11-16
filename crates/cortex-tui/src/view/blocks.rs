//! The transcript's small blocks (`docs/architecture/09-tui.md`,
//! "Elements"): your message, a notice, an error, reasoning while it streams
//! and when it has finished, and the turn summary.
//!
//! Each is led by its glyph, its text two columns in, wrapped at the prose
//! width. Text from the model, a tool or the core is sanitised here.

use std::time::Duration;

use cortex_base::text::{Keep, expand_tabs, truncate_with};
use cortex_term::{Line, Span, sanitize};

use super::{Density, Look, Reasoning, duration, one_line};
use crate::layout::{cut, wrap};
use crate::markdown::{Flavor, Markdown};
use crate::theme::Token;

/// Text after a glyph: the first row led by `glyph`, the others two
/// columns in, wrapped at the prose width.
fn led(glyph: Span, text: &[Span], width: usize, look: &Look) -> Vec<Line> {
    let prose = look.prose_width.min(width);
    let room = prose.saturating_sub(2).max(1);
    let indent = " ".repeat(cortex_base::text::width(glyph.text()).max(1) + 1);
    wrap(text, room)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let lead = if index == 0 {
                Line::from(glyph.clone()).with(Span::plain(" "))
            } else {
                Line::plain(indent.clone())
            };
            let mut line = lead;
            for span in row.spans() {
                line.push(span.clone());
            }
            cut(&line, width, look.glyphs.ellipsis)
        })
        .collect()
}

/// Content made safe and laid out on rows: control characters made
/// visible, tabs expanded.
fn clean(text: &str) -> String {
    expand_tabs(&sanitize(text), 4).into_owned()
}

/// Your message: `◆` in the accent, then the text in bold.
#[must_use]
pub fn user(text: &str, width: usize, look: &Look) -> Vec<Line> {
    let theme = &look.theme;
    led(
        Span::new(look.glyphs.user, theme.style(Token::User)),
        &[Span::new(clean(text), theme.style(Token::Title))],
        width,
        look,
    )
}

/// A notice: `·`, then one dim sentence.
#[must_use]
pub fn notice(text: &str, width: usize, look: &Look) -> Vec<Line> {
    let muted = look.theme.style(Token::Muted);
    led(
        Span::new(look.glyphs.notice, muted),
        &[Span::new(clean(text).replace('\n', " "), muted)],
        width,
        look,
    )
}

/// What a command shows: `·` and its title, then each of its lines as it
/// is, two columns in, dim, cut to the width rather than wrapped, so that
/// what lines up still does.
#[must_use]
pub fn listing(title: &str, lines: &[String], width: usize, look: &Look) -> Vec<Line> {
    let muted = look.theme.style(Token::Muted);
    let mut out = notice(title, width, look);
    for line in lines {
        let row = Line::plain("  ").with(Span::new(clean(line).replace('\n', " "), muted));
        out.push(crate::layout::cut(&row, width, look.glyphs.ellipsis));
    }
    out
}

/// An error: `✗` and a sentence in the error colour, then a dim sentence on
/// what to do next, two columns in.
#[must_use]
pub fn error(message: &str, hint: Option<&str>, width: usize, look: &Look) -> Vec<Line> {
    let theme = &look.theme;
    let error = theme.style(Token::Error);
    let mut lines = led(
        Span::new(look.glyphs.failed, error),
        &[Span::new(clean(message), error)],
        width,
        look,
    );
    if let Some(hint) = hint {
        let room = look.prose_width.min(width).saturating_sub(2).max(1);
        for row in wrap(&[Span::new(clean(hint), theme.style(Token::Muted))], room) {
            let mut line = Line::plain("  ");
            for span in row.spans() {
                line.push(span.clone());
            }
            lines.push(cut(&line, width, look.glyphs.ellipsis));
        }
    }
    lines
}

/// Reasoning while it streams: the spinner, `Reasoning` and the time so
/// far; then its latest words, in dim italic, on one row.
#[must_use]
pub fn reasoning_live(
    text: &str,
    elapsed: Duration,
    tick: usize,
    width: usize,
    look: &Look,
) -> Vec<Line> {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    let first = Line::from(Span::new(look.spinner(tick), theme.style(Token::Accent)))
        .with(Span::plain(" "))
        .with(Span::new("Reasoning", muted))
        .with(Span::new(format!("  {}", duration(elapsed)), muted));
    let mut lines = vec![cut(&first, width, look.glyphs.ellipsis)];
    let words = clean(text).split_whitespace().collect::<Vec<_>>().join(" ");
    if !words.is_empty() {
        let room = look.prose_width.min(width).saturating_sub(2);
        let latest = truncate_with(&words, room, look.glyphs.ellipsis, Keep::End);
        let line = Line::plain("  ").with(Span::new(latest, theme.style(Token::Reasoning)));
        lines.push(cut(&line, width, look.glyphs.ellipsis));
    }
    lines
}

/// Finished reasoning: `∴ Reasoned for 6s`, dim, then the provider's
/// summary in dim italic, two columns in. With `tui.reasoning =
/// "duration"`, or compact density, the first line alone.
#[must_use]
pub fn reasoning(summary: &str, took: Duration, width: usize, look: &Look) -> Vec<Line> {
    let muted = look.theme.style(Token::Muted);
    let first = Line::from(Span::new(look.glyphs.reasoning, muted)).with(Span::new(
        format!(" Reasoned for {}", duration(took)),
        muted,
    ));
    let mut lines = vec![cut(&first, width, look.glyphs.ellipsis)];
    if look.reasoning == Reasoning::Summary && look.density == Density::Comfortable {
        lines.extend(Markdown::render(Flavor::Reasoning, summary, width, look));
    }
    lines
}

/// What a turn cost: its duration, its tokens and its cost, dim, two
/// columns in. Shown only with `tui.turn_summary`.
#[must_use]
pub fn turn_summary(
    took: Duration,
    tokens: Option<u64>,
    cost: Option<&str>,
    width: usize,
    look: &Look,
) -> Vec<Line> {
    if !look.turn_summary {
        return Vec::new();
    }
    let mut parts = vec![duration(took)];
    if let Some(tokens) = tokens {
        parts.push(format!("{} tokens", count(tokens)));
    }
    if let Some(cost) = cost {
        parts.push(one_line(cost));
    }
    let text = parts.join(&format!(" {} ", look.glyphs.separator));
    let line = Line::plain("  ").with(Span::new(text, look.theme.style(Token::Muted)));
    vec![cut(&line, width, look.glyphs.ellipsis)]
}

/// A count as people read it: `412`, `12.4k`, `1.2M`.
#[must_use]
pub fn count(n: u64) -> String {
    // Shown to one decimal place, which f64 holds exactly enough.
    let value = n as f64;
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", value / 1_000.0)
    } else {
        format!("{:.1}M", value / 1_000_000.0)
    }
}

/// A blank row.
#[must_use]
pub fn blank() -> Line {
    Line::new()
}

#[cfg(test)]
mod tests;
