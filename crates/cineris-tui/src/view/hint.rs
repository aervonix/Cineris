//! The hint row (`docs/architecture/09-tui.md`, "The live region"): one dim
//! row under the input box, in place of a status line. Its left side holds
//! the keys that matter now, and its right side the status that matters now.
//!
//! Both sides sit two columns in from the edges, in line with the input
//! box's text. When the row is too narrow, the left side is cut, then the
//! right side's segments are dropped from its right; under 50 columns the
//! row shows the model and the context only.

use std::time::Duration;

use cineris_term::{Line, Span};

use super::{Look, duration, one_line};
use crate::layout::{cut, spread};
use crate::theme::Token;

/// What the hint row shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintRow {
    /// Its left side.
    pub left: Hint,
    /// The model's short name: always shown.
    pub model: String,
    /// The reasoning level, after the model.
    pub effort: Option<String>,
    /// The speed, when it is not `standard` (decision 0027).
    pub speed: Option<Speed>,
    /// How full the context window is, in percent: shown from 50.
    pub context: Option<u8>,
    /// The spend against a budget, when one is set (`$0.42 of $5`).
    pub budget: Option<String>,
    /// What is left of a provider's limit, when under a fifth
    /// (`18% left, resets 15:00`).
    pub limit: Option<String>,
    /// Access that is not the default: `sandbox off`, `unrestricted`.
    pub access: Option<String>,
    /// Anything else that matters now: `2 queued`, a rate-limit wait.
    pub extras: Vec<String>,
}

/// The hint row's left side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// Nothing.
    Empty,
    /// A turn is running: the spinner and its time, then the keys that
    /// matter.
    Running {
        /// The turn's time so far.
        elapsed: Duration,
        /// The spinner's frame.
        tick: usize,
        /// The keys after them.
        keys: RunningKeys,
    },
    /// The keys of a dialog, a menu or a focused panel.
    Keys(Vec<String>),
    /// After a turn, for a while: what it cost and what the session has.
    Cost {
        /// The turn's cost.
        turn: String,
        /// The session's.
        session: String,
    },
    /// In the first sessions: where to start.
    FirstSessions,
    /// `Ctrl+C` was pressed idle with nothing typed: a second press quits
    /// (step 21's `Outcome::QuitArmed`), and any other key disarms it.
    QuitArmed,
}

/// The keys shown while a turn runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunningKeys {
    /// `Enter` steers the turn with the pending message.
    Steer,
    /// `Tab` has made the pending message a queued one.
    Queue,
    /// A dialog's, a menu's or a focused panel's keys.
    Of(Vec<String>),
}

/// A speed other than `standard`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speed {
    /// Its name: `fast`, or a provider's own (`flex`, `priority`).
    pub name: String,
    /// When a cooldown after a fallback ends, while one runs (`14:32`):
    /// requests go at `standard` until then.
    pub cooldown_until: Option<String>,
}

/// Under this many columns the row shows the model and the context only.
pub const NARROW: usize = 50;

/// The hint row, `width` columns wide at most.
#[must_use]
pub fn hint_row(row: &HintRow, width: usize, look: &Look) -> Line {
    let theme = &look.theme;
    let hint = theme.style(Token::Hint);
    let sep = format!(" {} ", look.glyphs.separator);
    let segments = right_segments(row, width < NARROW, look);
    let left = if width < NARROW {
        Line::new()
    } else {
        left_side(&row.left, &sep, look)
    };
    let indent = Line::plain("  ");
    // The right side's segments are dropped from its right until it leaves
    // the left side at least this much room, or only the model is left.
    let left_room = left.width().min(24);
    let mut count = segments.len();
    let right = loop {
        let mut right = Line::new();
        for (index, segment) in segments[..count].iter().enumerate() {
            if index > 0 {
                right.push(Span::new(sep.clone(), hint));
            }
            for span in segment.spans() {
                right.push(span.clone());
            }
        }
        right.push(Span::plain("  "));
        if count <= 1 || 2 + left_room + 2 + right.width() <= width {
            break right;
        }
        count -= 1;
    };
    let mut line = indent;
    let left = cut(&left, width.saturating_sub(2), look.glyphs.ellipsis);
    let body = spread(
        &left,
        &right,
        width.saturating_sub(2),
        2,
        look.glyphs.ellipsis,
    );
    for span in body.spans() {
        line.push(span.clone());
    }
    cut(&line, width, look.glyphs.ellipsis)
}

fn left_side(left: &Hint, sep: &str, look: &Look) -> Line {
    let theme = &look.theme;
    let hint = theme.style(Token::Hint);
    match left {
        Hint::Empty => Line::new(),
        Hint::Running {
            elapsed,
            tick,
            keys,
        } => {
            let keys = match keys {
                RunningKeys::Steer => [
                    "esc stops",
                    "enter steers this turn",
                    "tab queues it instead",
                ]
                .join(sep),
                RunningKeys::Queue => [
                    "esc stops",
                    "enter queues it",
                    "tab steers this turn instead",
                ]
                .join(sep),
                RunningKeys::Of(keys) => keys
                    .iter()
                    .map(|key| one_line(key))
                    .collect::<Vec<_>>()
                    .join(sep),
            };
            Line::from(Span::new(look.spinner(*tick), theme.style(Token::Accent)))
                .with(Span::new(format!(" {}  ", duration(*elapsed)), hint))
                .with(Span::new(keys, hint))
        }
        Hint::Keys(keys) => {
            let keys: Vec<String> = keys.iter().map(|key| one_line(key)).collect();
            Line::styled(keys.join(sep), hint)
        }
        Hint::Cost { turn, session } => Line::styled(
            format!(
                "this turn {}, session {}",
                one_line(turn),
                one_line(session)
            ),
            hint,
        ),
        Hint::FirstSessions => Line::styled(
            ["/ commands", "@ files", "alt+enter new line"].join(sep),
            hint,
        ),
        Hint::QuitArmed => Line::styled("ctrl+c again quits", hint),
    }
}

/// The right side's segments, in the order they are dropped from the end.
fn right_segments(row: &HintRow, narrow: bool, look: &Look) -> Vec<Line> {
    let theme = &look.theme;
    let hint = theme.style(Token::Hint);
    let mut model = one_line(&row.model);
    if let Some(effort) = &row.effort {
        model.push(' ');
        model.push_str(&one_line(effort));
    }
    let mut segments = vec![Line::styled(model, hint)];
    let context = row.context.filter(|percent| *percent >= 50).map(|percent| {
        let token = if percent >= 90 {
            Token::Error
        } else if percent >= 70 {
            Token::Warning
        } else {
            Token::Hint
        };
        Line::styled(format!("{percent}% context"), theme.style(token))
    });
    if narrow {
        segments.extend(context);
        return segments;
    }
    if let Some(speed) = &row.speed {
        let name = one_line(&speed.name);
        segments.push(match &speed.cooldown_until {
            Some(until) => Line::styled(format!("{name} after {}", one_line(until)), hint),
            None => Line::styled(name, theme.style(Token::AccentText)),
        });
    }
    segments.extend(context);
    if let Some(budget) = &row.budget {
        segments.push(Line::styled(one_line(budget), hint));
    }
    if let Some(limit) = &row.limit {
        segments.push(Line::styled(one_line(limit), theme.style(Token::Warning)));
    }
    if let Some(access) = &row.access {
        segments.push(Line::styled(one_line(access), theme.style(Token::Warning)));
    }
    for extra in &row.extras {
        segments.push(Line::styled(one_line(extra), hint));
    }
    segments
}

#[cfg(test)]
mod tests;
