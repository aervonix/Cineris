//! The viewer's frame (`docs/architecture/09-tui.md`, "The viewer"): a
//! title row and a rule above the transcript, a rule and a row of keys
//! below it (since step 23). The transcript between them is drawn by the
//! other views, its tool boxes with [`Placement::Viewer`].
//!
//! [`Placement::Viewer`]: super::Placement::Viewer

use cineris_term::{Line, Span, sanitize};

use super::Look;
use crate::layout::cut;
use crate::theme::Token;

/// What the title row says beside `Viewer`.
#[derive(Debug, Clone, Default)]
pub struct ViewerTitle {
    /// The session's folder, as a person reads it (`~/src/parser`).
    pub folder: String,
    /// When the session began, in the local time (`14:02`).
    pub began: Option<String>,
    /// The first and the last turn the view holds.
    pub turns: Option<(u64, u64)>,
}

/// What the row of keys under the transcript says.
#[derive(Debug, Clone)]
pub enum ViewerKeys {
    /// The transcript has tool boxes: the selected one, counted from one,
    /// of `count`, and whether it is open.
    Boxes {
        /// The selected box, counted from one.
        selected: usize,
        /// How many there are.
        count: usize,
        /// Whether the selected box is open.
        open: bool,
    },
    /// A reasoning or a compaction is selected (since step 25): `Enter`
    /// shows it whole; `label` says which.
    Summary {
        /// `reasoning` or `summary`.
        label: &'static str,
        /// Where the read of a compaction's summary is, once `Enter` asked
        /// for it (since the review of step 25's second pull request).
        read: Option<SummaryRead>,
    },
    /// No tool box: the view only scrolls.
    Lines,
    /// A search being typed after `/`.
    Search {
        /// What has been typed.
        query: String,
        /// Whether the last search found nothing.
        missed: bool,
    },
}

/// Where the read of a compaction's summary is (since the review of step
/// 25's second pull request).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryRead {
    /// Asked for, its answer not come.
    Reading,
    /// It failed, or the session had no such summary.
    Failed,
}

/// The title row and the rule under it, `width` columns.
#[must_use]
pub fn title(title: &ViewerTitle, width: usize, look: &Look) -> Vec<Line> {
    let muted = look.theme.style(Token::Muted);
    let separator = look.glyphs.separator;
    let mut said = vec![sanitize(&title.folder).into_owned()];
    if let Some(began) = &title.began {
        said.push(format!("session of {began}"));
    }
    match title.turns {
        Some((first, last)) if first == last => said.push(format!("turn {first}")),
        Some((first, last)) if last == first + 1 => said.push(format!("turns {first} and {last}")),
        Some((first, last)) => said.push(format!("turns {first} to {last}")),
        None => {}
    }
    let left = Line::from(Span::new(" Viewer", look.theme.style(Token::Title))).with(Span::new(
        format!("   {}", said.join(&format!(" {separator} "))),
        muted,
    ));
    vec![
        both_ends(
            left,
            Span::new("esc returns to the session ", muted),
            width,
            look,
        ),
        rule(width, look),
    ]
}

/// The rule above the row of keys, and the row, `width` columns.
#[must_use]
pub fn keys(keys: &ViewerKeys, width: usize, look: &Look) -> Vec<Line> {
    let muted = look.theme.style(Token::Muted);
    let separator = look.glyphs.separator;
    let join = |parts: &[&str]| format!(" {}", parts.join(&format!(" {separator} ")));
    let (left, right) = match keys {
        ViewerKeys::Boxes {
            selected,
            count,
            open,
        } => (
            join(&[
                "up/down moves between boxes",
                if *open {
                    "enter collapses"
                } else {
                    "enter expands"
                },
                "/ searches",
                "esc returns",
            ]),
            format!("box {selected} of {count} "),
        ),
        // The read's state first, where a narrow row keeps it.
        ViewerKeys::Summary { label, read } => (
            match read {
                None => join(&[
                    "up/down moves between boxes",
                    "enter shows it whole",
                    "/ searches",
                    "esc returns",
                ]),
                Some(SummaryRead::Reading) => join(&[
                    &format!("reading the {label}{}", look.glyphs.ellipsis),
                    "up/down moves between boxes",
                    "esc returns",
                ]),
                Some(SummaryRead::Failed) => join(&[
                    &format!("the {label} could not be read"),
                    "enter tries again",
                    "esc returns",
                ]),
            },
            format!("{label} "),
        ),
        ViewerKeys::Lines => (
            join(&["up/down scrolls", "/ searches", "esc returns"]),
            String::new(),
        ),
        ViewerKeys::Search { query, missed } => (
            format!(" / {}", sanitize(query).replace(['\n', '\t'], " ")),
            if *missed {
                "no match · enter finds · esc stops ".to_owned()
            } else {
                "enter finds · esc stops ".to_owned()
            },
        ),
    };
    vec![
        rule(width, look),
        both_ends(
            Line::from(Span::new(left, muted)),
            Span::new(right, muted),
            width,
            look,
        ),
    ]
}

/// A rule across `width` columns.
fn rule(width: usize, look: &Look) -> Line {
    Line::styled(
        look.glyphs.horizontal.repeat(width),
        look.theme.style(Token::Muted),
    )
}

/// `left`, then `right` against the right edge; `left` is cut when both do
/// not fit, and `right` left out when it alone does not.
fn both_ends(left: Line, right: Span, width: usize, look: &Look) -> Line {
    let room = width.saturating_sub(right.width());
    if left.width() + 1 > room {
        return cut(&left, width, look.glyphs.ellipsis);
    }
    let mut line = left;
    let gap = room - line.width();
    line.push(Span::plain(" ".repeat(gap)));
    line.push(right);
    line
}
