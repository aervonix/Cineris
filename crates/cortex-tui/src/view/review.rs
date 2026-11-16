//! The review panel's rows (`docs/architecture/09-tui.md`, "Panels";
//! `docs/steps/24/answered.md`, points 2 and 4b): what a turn changed, and
//! what the project's checks said of it.
//!
//! ```text
//! ◇ Changed 3 files  +12 −4 · checks failed
//!   src/lex.rs  +5 −1
//!   …/numbers/separators.rs  +6 −3
//!   tests/lex.rs  +1 −0
//!   ✓ cargo test -p parser  crates/parser  exit 0 · 3.4s
//!   ✗ cargo clippy -- -D warnings  exit 101 · 8.1s  2 warnings
//!   checks failed · round 1 of 2
//!   commands ran too: /view shows them
//! ```
//!
//! Collapsed, the panel is its first row alone. Focused, each file is a row
//! (its path shortened from its start with `…`, so that its counts stay),
//! then the checks: while a round runs, `checks running · round 1 of 2` and
//! each check by its command; when it ended, each check's line (its mark,
//! command, folder, measure and note) and the verdict; when they were only
//! offered (`verify.mode = "ask"`), `checks not run: /verify runs them`.
//! When a command ran in the turn, a last row says the viewer shows it:
//! which files a command changed is not something the protocol says.
//!
//! Everything here came from outside (a path, a command, a folder, a note)
//! and is made one line and visible before it is laid out.

use std::time::Duration;

use cortex_base::text::{graphemes, width as text_width};
use cortex_protocol::event::{
    CheckOutcome, CheckReport, Verification, VerificationNotice, VerificationPhase,
};
use cortex_term::{Line, Span};

use super::panel::{self, PanelRow};
use super::{Look, one_line, precise};
use crate::layout::cut;
use crate::theme::Token;

/// What a turn changed, and what its checks said: the review panel's state,
/// as its view draws it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Review {
    /// The files the file tools changed, in the order first changed.
    pub files: Vec<ChangedFile>,
    /// The last verification notice of the turn: a round starting, a round
    /// ended, or the checks offered.
    pub checks: Option<VerificationNotice>,
    /// The turn's verdict, from its end (`turn.completed`).
    pub verdict: Option<Verification>,
    /// A command ran and succeeded in the turn: it may have changed files
    /// too.
    pub commands: bool,
}

/// One file a turn changed, with its counts where they are known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// The path, as the tool gave it.
    pub path: String,
    /// Lines added, when counted.
    pub added: Option<u64>,
    /// Lines removed, when counted.
    pub removed: Option<u64>,
}

impl Review {
    /// The rows that can have the focus: each file, then each check.
    #[must_use]
    pub fn items(&self) -> usize {
        self.files.len() + self.checks.as_ref().map_or(0, |notice| notice.checks.len())
    }
}

/// The review panel's first row: `◇ Changed 3 files  +12 −4 · checks
/// passed`, or the checks alone when no file tool changed a file.
#[must_use]
pub fn review_header(review: &Review, width: usize, look: &Look) -> Line {
    let theme = &look.theme;
    let mut rest = Vec::new();
    let status = status(review, look);
    let title = if review.files.is_empty() {
        // The checks alone: their words are the title.
        let (text, _) = status
            .clone()
            .unwrap_or_else(|| ("Changes".to_owned(), Token::Muted));
        capitalised(&text)
    } else {
        let count = review.files.len();
        let noun = if count == 1 { "file" } else { "files" };
        let (added, removed) = totals(review);
        if added.is_some() || removed.is_some() {
            rest.push(Span::plain("  "));
            rest.extend(counts(added, removed, look));
        }
        if let Some((text, token)) = status {
            rest.push(Span::new(
                format!(" {} ", look.glyphs.separator),
                theme.style(Token::Muted),
            ));
            rest.push(Span::new(text, theme.style(token)));
        }
        format!("Changed {count} {noun}")
    };
    panel::header(&title, &rest, width, look)
}

/// The panel drawn whole, as its focused panel shows it, the `focused`
/// item drawn as the selection, `tick` the spinner's frame.
#[must_use]
pub fn review_rows(
    review: &Review,
    focused: Option<usize>,
    width: usize,
    look: &Look,
    tick: usize,
) -> Vec<PanelRow> {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    let mut rows = vec![PanelRow::plain(review_header(review, width, look))];
    for (index, file) in review.files.iter().enumerate() {
        rows.push(PanelRow::of(
            index,
            file_row(file, focused == Some(index), width, look),
        ));
    }
    let first_check = review.files.len();
    if let Some(notice) = &review.checks {
        let round = round_words(notice, look);
        match &notice.phase {
            VerificationPhase::Started => {
                rows.push(PanelRow::plain(indented(
                    format!("checks running{round}"),
                    muted,
                    width,
                    look,
                )));
            }
            VerificationPhase::Offered => {
                rows.push(PanelRow::plain(indented(
                    "checks not run: /verify runs them".to_owned(),
                    muted,
                    width,
                    look,
                )));
            }
            _ => {}
        }
        for (index, check) in notice.checks.iter().enumerate() {
            let item = first_check + index;
            rows.push(PanelRow::of(
                item,
                check_row(
                    check,
                    &notice.phase,
                    focused == Some(item),
                    width,
                    look,
                    tick,
                ),
            ));
        }
        if notice.phase == VerificationPhase::Finished
            && let Some((text, token)) =
                verdict_words(review.verdict.as_ref().or(notice.verdict.as_ref()))
        {
            rows.push(PanelRow::plain(cut(
                &Line::plain("  ")
                    .with(Span::new(text, theme.style(token)))
                    .with(Span::new(round, muted)),
                width,
                look.glyphs.ellipsis,
            )));
        }
    } else if let Some((text, token)) = verdict_words(review.verdict.as_ref()) {
        rows.push(PanelRow::plain(indented(
            text,
            theme.style(token),
            width,
            look,
        )));
    }
    if review.commands {
        rows.push(PanelRow::plain(indented(
            "commands ran too: /view shows them".to_owned(),
            muted,
            width,
            look,
        )));
    }
    rows
}

/// The panel collapsed: its first row.
#[must_use]
pub fn review_collapsed(review: &Review, width: usize, look: &Look) -> Vec<Line> {
    vec![review_header(review, width, look)]
}

/// A file's row: its path, shortened from its start so that the counts
/// after it stay, and the counts.
#[must_use]
pub fn file_row(file: &ChangedFile, focused: bool, width: usize, look: &Look) -> Line {
    let theme = &look.theme;
    let mut counted = Line::new();
    if file.added.is_some() || file.removed.is_some() {
        counted.push(Span::plain("  "));
        for span in counts(file.added, file.removed, look) {
            counted.push(span);
        }
    }
    let token = if focused {
        Token::Selected
    } else {
        Token::Text
    };
    let room = width.saturating_sub(2 + counted.width());
    let path = one_line(&file.path);
    let path = shortened(&path, room, look.glyphs.ellipsis);
    let mut line = Line::plain("  ").with(Span::new(path, theme.style(token)));
    for span in counted.spans() {
        line.push(span.clone());
    }
    cut(&line, width, look.glyphs.ellipsis)
}

/// A check's row: its mark, its command, its folder, how it ended and its
/// note.
#[must_use]
pub fn check_row(
    check: &CheckReport,
    phase: &VerificationPhase,
    focused: bool,
    width: usize,
    look: &Look,
    tick: usize,
) -> Line {
    let theme = &look.theme;
    let glyphs = look.glyphs;
    let muted = theme.style(Token::Muted);
    let (mark, mark_token) = match (&check.outcome, phase) {
        (Some(CheckOutcome::Passed), _) => (glyphs.succeeded, Token::Success),
        (Some(CheckOutcome::Failed), _) => (glyphs.failed, Token::Error),
        (Some(CheckOutcome::TimedOut), _) => (glyphs.interrupted, Token::Warning),
        (Some(CheckOutcome::Flaky), _) => (glyphs.waiting, Token::Warning),
        (Some(CheckOutcome::Blocked), _) => (glyphs.denied, Token::Warning),
        (Some(_), _) => (glyphs.denied, Token::Muted),
        (None, VerificationPhase::Started) => (look.spinner(tick), Token::Accent),
        (None, _) => (glyphs.notice, Token::Muted),
    };
    let command = if focused {
        Token::Selected
    } else {
        Token::Title
    };
    let mut line = Line::plain("  ")
        .with(Span::new(mark, theme.style(mark_token)))
        .with(Span::plain(" "))
        .with(Span::new(one_line(&check.command), theme.style(command)));
    if let Some(cwd) = check.cwd.as_deref().filter(|cwd| !cwd.is_empty()) {
        line.push(Span::new(format!("  {}", one_line(cwd)), muted));
    }
    let failed = matches!(
        check.outcome,
        Some(CheckOutcome::Failed | CheckOutcome::TimedOut)
    );
    let measure_style = if failed {
        theme.style(Token::Error)
    } else {
        muted
    };
    let mut measure = Vec::new();
    if check.outcome == Some(CheckOutcome::TimedOut) {
        measure.push("timed out".to_owned());
    } else if let Some(code) = check.exit_code.filter(|code| failed || *code != 0) {
        measure.push(format!("exit {code}"));
    }
    if let Some(ms) = check.duration_ms {
        measure.push(precise(Duration::from_millis(ms)));
    }
    if !measure.is_empty() {
        line.push(Span::plain("  "));
        line.push(Span::new(
            measure.join(&format!(" {} ", glyphs.separator)),
            measure_style,
        ));
    }
    if let Some(note) = check.note.as_deref().filter(|note| !note.is_empty()) {
        line.push(Span::new(format!("  {}", one_line(note)), muted));
    }
    cut(&line, width, glyphs.ellipsis)
}

/// What the checks say, in words, with the token to draw it in: the verdict
/// once the turn or a round has one, else a round running, or the checks
/// offered. A turn whose checks were offered has the verdict `skipped`:
/// what was offered says more.
fn status(review: &Review, look: &Look) -> Option<(String, Token)> {
    let skipped = review.verdict == Some(Verification::Skipped);
    if !skipped && let Some(words) = verdict_words(review.verdict.as_ref()) {
        return Some(words);
    }
    let Some(notice) = review.checks.as_ref() else {
        return verdict_words(review.verdict.as_ref());
    };
    match &notice.phase {
        VerificationPhase::Started => Some((
            format!("checks running{}", round_words(notice, look)),
            Token::Muted,
        )),
        VerificationPhase::Offered => {
            Some(("checks not run: /verify runs them".to_owned(), Token::Muted))
        }
        _ => verdict_words(review.verdict.as_ref().or(notice.verdict.as_ref()))
            .or_else(|| Some(("checks finished".to_owned(), Token::Muted))),
    }
}

/// A verdict in words, and its token.
fn verdict_words(verdict: Option<&Verification>) -> Option<(String, Token)> {
    Some(match verdict? {
        Verification::Passed => ("checks passed".to_owned(), Token::Success),
        Verification::Failed => ("checks failed".to_owned(), Token::Error),
        Verification::Incomplete => ("checks incomplete".to_owned(), Token::Warning),
        Verification::Skipped => ("no checks ran".to_owned(), Token::Muted),
        other => (format!("checks {}", one_line(other.as_str())), Token::Muted),
    })
}

/// ` · round 1 of 2`, when the turn has more than one round.
fn round_words(notice: &VerificationNotice, look: &Look) -> String {
    if notice.rounds > 1 && notice.round > 0 {
        format!(
            " {} round {} of {}",
            look.glyphs.separator, notice.round, notice.rounds
        )
    } else {
        String::new()
    }
}

/// The counts known of every file, summed.
fn totals(review: &Review) -> (Option<u64>, Option<u64>) {
    let sum = |pick: fn(&ChangedFile) -> Option<u64>| {
        review
            .files
            .iter()
            .filter_map(pick)
            .reduce(u64::saturating_add)
    };
    (sum(|file| file.added), sum(|file| file.removed))
}

/// `+12 −4` in the diff's colours, each part there when known.
fn counts(added: Option<u64>, removed: Option<u64>, look: &Look) -> Vec<Span> {
    let theme = &look.theme;
    let mut spans = Vec::new();
    if let Some(added) = added {
        spans.push(Span::new(format!("+{added}"), theme.style(Token::Success)));
    }
    if let Some(removed) = removed {
        if !spans.is_empty() {
            spans.push(Span::plain(" "));
        }
        spans.push(Span::new(
            format!("{}{removed}", look.glyphs.minus),
            theme.style(Token::Error),
        ));
    }
    spans
}

/// `text` two columns in, in `style`, cut to the width.
fn indented(text: String, style: cortex_term::Style, width: usize, look: &Look) -> Line {
    cut(
        &Line::plain("  ").with(Span::new(text, style)),
        width,
        look.glyphs.ellipsis,
    )
}

/// `text` with its first letter in capitals.
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// `path` in at most `room` columns: whole when it fits, else its end after
/// `ellipsis`, cut between grapheme clusters, so that the file's name
/// stays.
fn shortened(path: &str, room: usize, ellipsis: &str) -> String {
    if text_width(path) <= room {
        return path.to_owned();
    }
    let mark = text_width(ellipsis);
    if room <= mark {
        // No room for a part of it: what fits of the mark, cut later.
        return ellipsis.to_owned();
    }
    let clusters: Vec<(usize, &str, usize)> = graphemes(path).collect();
    let mut used = 0;
    let mut start = path.len();
    for (offset, _, columns) in clusters.iter().rev() {
        if used + columns > room - mark {
            break;
        }
        used += columns;
        start = *offset;
    }
    format!("{ellipsis}{}", &path[start..])
}

#[cfg(test)]
mod tests;
