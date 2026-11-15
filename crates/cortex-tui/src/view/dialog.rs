//! The dialogs that take the input box's place: an approval, and leaving
//! the sandbox (`docs/architecture/09-tui.md`, "Dialogs and menus").
//!
//! ```text
//! ? Edit .github/workflows/ci.yml?
//!   CI configuration is protected, so a change to it always asks.
//!
//!      29         - uses: dtolnay/rust-toolchain@stable
//!      30 −         with: { components: rustfmt }
//!      30 +         with: { components: rustfmt, clippy }
//!
//!   y Allow once   s Allow this session   a Always allow here   n Deny   e Deny with a note
//! ```
//!
//! The question is bold after a `?` in the warning colour, with a dim line
//! for each reason it is asked. The detail is exactly what would happen,
//! wrapped and never shortened, a diff with its gutter; a diff taller than
//! the room it is given is clipped with a line that says so, and `Ctrl+O`
//! shows it whole. The answers are on one line, each key in the accent.
//! Everything from outside passes through `cortex_term::sanitize`.

use cortex_protocol::lease::Effect;
use cortex_protocol::methods::{ApprovalDetail, ApprovalRequest};
use cortex_term::{Line, Span, sanitize};

use super::round::diff_rows;
use super::{DiffLine, Look};
use crate::layout::{cut, wrap};
use crate::theme::Token;

/// One answer as the dialog lists it: its key, as the key map names it, and
/// its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The key: `y`, `esc`.
    pub key: String,
    /// What it does: `Allow once`.
    pub label: String,
}

/// What an approval dialog shows beside its request.
#[derive(Debug, Clone, Default)]
pub struct ApprovalView<'a> {
    /// The answers on offer, in order.
    pub answers: Vec<Answer>,
    /// A diff's lines, parsed from the request's detail.
    pub diff: Vec<DiffLine>,
    /// The most rows the detail may take; the rest is behind `Ctrl+O`.
    pub detail_rows: usize,
    /// The note being written, after "deny with a note".
    pub note: Option<&'a str>,
}

/// Whether the request is one to leave the sandbox: a command that would
/// run outside it.
#[must_use]
pub fn leaves_sandbox(request: &ApprovalRequest) -> bool {
    matches!(&request.detail, ApprovalDetail::Command(command) if command.outside_sandbox)
}

/// The approval dialog, `width` columns wide at most.
#[must_use]
pub fn approval(
    request: &ApprovalRequest,
    view: &ApprovalView<'_>,
    width: usize,
    look: &Look,
) -> Vec<Line> {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    let mut lines = Vec::new();
    let outside = leaves_sandbox(request);
    let question = if outside {
        "Run outside the sandbox?".to_owned()
    } else {
        format!("{}?", sanitize(&request.summary).replace(['\n', '\t'], " "))
    };
    let title = Line::from(Span::new(look.glyphs.waiting, theme.style(Token::Warning)))
        .with(Span::plain(" "))
        .with(Span::new(question, theme.style(Token::Title)));
    lines.push(cut(&title, width, look.glyphs.ellipsis));
    if !outside {
        for reason in &request.reasons {
            indented(&mut lines, &sanitize(reason), muted, 2, width);
        }
    }
    lines.push(Line::new());

    match &request.detail {
        ApprovalDetail::Command(command) => {
            let accent = theme.style(Token::AccentText);
            // Exactly what would run, line breaks and all.
            for line in sanitize(&command.command).split('\n') {
                indented(&mut lines, line, accent, 4, width);
            }
            indented(
                &mut lines,
                &format!("in {}", sanitize(&command.cwd)),
                muted,
                4,
                width,
            );
            if outside {
                lines.push(Line::new());
                indented(
                    &mut lines,
                    "It would run with your full rights: it can write anywhere you can, and use \
                     the network. This is asked every time.",
                    theme.style(Token::Warning),
                    2,
                    width,
                );
            }
        }
        ApprovalDetail::Diff(diff) => {
            if view.diff.is_empty() {
                indented(
                    &mut lines,
                    &sanitize(&diff.path),
                    theme.style(Token::Code),
                    4,
                    width,
                );
                let why = diff
                    .omitted
                    .as_deref()
                    .unwrap_or("The change has no lines to show.");
                indented(&mut lines, &sanitize(why), muted, 4, width);
            } else {
                let rows = diff_rows(&view.diff, width.saturating_sub(1), look);
                let room = view.detail_rows.max(1);
                let more = rows.len().saturating_sub(room);
                let shown = if more > 0 {
                    room.saturating_sub(1)
                } else {
                    rows.len()
                };
                for (row, fill) in rows.into_iter().take(shown) {
                    let mut line = Line::plain(" ");
                    for span in row.spans() {
                        line.push(span.clone());
                    }
                    let line = crate::layout::pad(line, width, fill);
                    lines.push(cut(&line, width, look.glyphs.ellipsis));
                }
                if more > 0 {
                    let hidden = more + 1;
                    indented(
                        &mut lines,
                        &format!(
                            "{} {hidden} more lines: ctrl+o shows it whole",
                            look.glyphs.ellipsis
                        ),
                        muted,
                        4,
                        width,
                    );
                }
            }
        }
        ApprovalDetail::Target(target) => {
            let what = match &target.effect {
                Effect::Read => "Read",
                Effect::Write => "Write",
                Effect::Network => "Connect to",
                _ => "Use",
            };
            indented(
                &mut lines,
                &format!("{what} {}", sanitize(&target.target)),
                theme.style(Token::Code),
                4,
                width,
            );
        }
        ApprovalDetail::Tool(tool) => {
            let text = serde_json::to_string_pretty(&tool.arguments).unwrap_or_default();
            for line in sanitize(&text).split('\n') {
                indented(&mut lines, line, theme.style(Token::Code), 4, width);
            }
        }
        _ => indented(
            &mut lines,
            "This build cannot show what the call would do.",
            muted,
            4,
            width,
        ),
    }

    // What "this session" and "here" would allow from then on, in full.
    if !request.adds.is_empty() && !outside {
        lines.push(Line::new());
        let rules: Vec<String> = request
            .adds
            .iter()
            .map(|rule| format!("{} {}", effect_word(&rule.effect), sanitize(&rule.pattern)))
            .collect();
        indented(
            &mut lines,
            &format!("Allowing it for longer allows: {}", rules.join(", ")),
            muted,
            2,
            width,
        );
    }

    lines.push(Line::new());
    match view.note {
        Some(note) => {
            let row = Line::plain("  ")
                .with(Span::new("Note for the model: ", theme.style(Token::Title)))
                .with(Span::plain(sanitize(note).replace(['\n', '\t'], " ")));
            lines.push(cut(&row, width, look.glyphs.ellipsis));
        }
        None => {
            let mut row = Line::plain("  ");
            for answer in &view.answers {
                row.push(Span::new(answer.key.clone(), theme.style(Token::Key)));
                row.push(Span::plain(format!(" {}   ", answer.label)));
            }
            lines.push(cut(&row, width, look.glyphs.ellipsis));
        }
    }
    lines
}

/// The rows of a dialog's detail, wrapped at `width` and set `indent`
/// columns in.
fn indented(
    lines: &mut Vec<Line>,
    text: &str,
    style: cortex_term::Style,
    indent: usize,
    width: usize,
) {
    let room = width.saturating_sub(indent).max(1);
    let pad = " ".repeat(indent);
    for row in wrap(&[Span::new(text, style)], room) {
        let mut line = Line::plain(pad.clone());
        for span in row.spans() {
            line.push(span.clone());
        }
        lines.push(line);
    }
}

fn effect_word(effect: &Effect) -> &'static str {
    match effect {
        Effect::Read => "reading",
        Effect::Write => "writing",
        Effect::Exec => "running",
        Effect::Network => "connecting to",
        _ => "using",
    }
}

#[cfg(test)]
mod tests;
