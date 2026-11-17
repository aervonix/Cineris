//! The project panel's rows (`docs/architecture/09-tui.md`, "Panels";
//! "The panels as built"; `docs/architecture/14-adaptation.md`, "Project
//! knowledge"): the entries the harness keeps of the project, in use and
//! waiting for the person's review.
//!
//! ```text
//! ◇ Project  2 wait for review · 3 in use
//!   ? command  test: cargo test -p parser                              2d
//!     runs  cargo test -p parser --all-features
//!     in crates/parser · may change files
//!     observed 3 times, learned in session s_1 on 2026-11-14
//!     waits for your review
//!     check  `Cargo.toml` exists, not run in this build
//!   ? convention  Errors are written as sentences.                     5h
//!   ✓ fact  The parser is hand-written.                                3d
//! ```
//!
//! Collapsed, the panel is its first row. Focused, each entry is a row
//! (its mark, kind, text cut with `…`, and age), and the focused entry's
//! detail follows it, **wrapped at the width and never cut**: what will
//! run, where, whether it only reads, where the entry came from, whether
//! it still asks and when it expires, its check, and the question when its
//! deletion is asked. The panel takes `y` only when that detail was drawn
//! whole (`app::panels::project`).
//!
//! Everything here came from outside (a text, a command, a folder, an
//! evidence, a check, a session's id, a problem) and is made one line and
//! visible before it is laid out.

use cortex_base::UtcTime;
use cortex_protocol::project::{
    EntryBasis, EntryCheckKind, EntryKind, EntryScope, EntrySourceKind, EntryStatus, ProjectEntry,
};
use cortex_term::{Line, Span};

use super::panel::{self, PanelRow};
use super::{Look, one_line};
use crate::layout::{concat, cut, spread, wrap};
use crate::theme::Token;

/// The project panel's state, as its view draws it.
#[derive(Debug, Clone, Copy)]
pub struct ProjectView<'a> {
    /// The entries shown, in order: those waiting first, then those in use.
    pub entries: &'a [ProjectEntry],
    /// The time now.
    pub now: UtcTime,
    /// The entry whose deletion is asked, by its place.
    pub asking: Option<usize>,
    /// Whether the entries are still being read (`project/list`).
    pub reading: bool,
    /// What is wrong with the project's files, a sentence each.
    pub problems: &'a [String],
    /// Whether the person trusts the project as it is: without that, an
    /// accepted command still asks before it runs.
    pub trusted: bool,
    /// The key that answers the question of a deletion (`focus.accept`),
    /// as the key map names it: none when no key is bound to it.
    pub yes: Option<&'a str>,
}

/// Whether the user accepted `entry`, or wrote it in their own store: for
/// a command, what lets it run without asking once the project is trusted.
#[must_use]
pub fn accepted(entry: &ProjectEntry) -> bool {
    entry.accepted_at.is_some()
        || (entry.scope == EntryScope::Private && entry.source.kind == EntrySourceKind::User)
}

/// Whether `entry` waits for this person's review: a proposal, or an entry
/// in use that they never accepted and did not write (one observed that
/// became active by itself, one of the repository's file).
#[must_use]
pub fn waits(entry: &ProjectEntry) -> bool {
    match &entry.status {
        EntryStatus::Proposed => true,
        EntryStatus::Active => {
            !accepted(entry)
                && (entry.basis == Some(EntryBasis::Observed) || entry.scope == EntryScope::Shared)
        }
        _ => false,
    }
}

/// Whether `y` would change `entry`: one waiting, turned off, expired, or
/// a command in use that the person has not accepted.
#[must_use]
pub fn acceptable(entry: &ProjectEntry, now: UtcTime) -> bool {
    waits(entry) || entry.status != EntryStatus::Active || !accepted(entry) || expired(entry, now)
}

/// Whether the core would promote `entry` to the repository's file: one of
/// the private store, in use and accepted (the core refuses the others as
/// `not_promotable`).
#[must_use]
pub fn promotable(entry: &ProjectEntry, now: UtcTime) -> bool {
    entry.scope == EntryScope::Private
        && entry.status == EntryStatus::Active
        && !expired(entry, now)
        && accepted(entry)
}

/// Whether an entry of the repository's file: the core changes its text
/// and deletes it in no store (`shared_entry_read_only`).
#[must_use]
pub fn shared(entry: &ProjectEntry) -> bool {
    entry.scope == EntryScope::Shared
}

/// When accepting `entry` at `now` has it expire: a learned entry is kept
/// its kind's lifetime from the acceptance (90 days for a command, 60 for a
/// convention or a fact, 30 for a flaky test, as `cortex-knowledge` keeps
/// them); any other keeps its own expiry, or none.
#[must_use]
pub fn kept_until(entry: &ProjectEntry, now: UtcTime) -> Option<UtcTime> {
    const DAY: u64 = 86_400;
    if entry.source.kind != EntrySourceKind::Learned || entry.scope == EntryScope::Shared {
        return entry
            .expires_at
            .as_ref()
            .and_then(|at| UtcTime::parse(at.as_str()).ok())
            .filter(|at| *at > now);
    }
    let days = match &entry.kind {
        EntryKind::Command => 90,
        EntryKind::Flaky => 30,
        EntryKind::Convention | EntryKind::Fact => 60,
        _ => return None,
    };
    now.checked_add(std::time::Duration::from_secs(days * DAY))
}

/// The day of `at`, as the protocol writes it: `2026-12-14`.
fn day_of(at: UtcTime) -> String {
    let stamp = at.to_timestamp();
    stamp
        .as_str()
        .get(..10)
        .unwrap_or(stamp.as_str())
        .to_owned()
}

/// Whether `entry` is past its expiry at `now`.
#[must_use]
pub fn expired(entry: &ProjectEntry, now: UtcTime) -> bool {
    entry
        .expires_at
        .as_ref()
        .and_then(|at| UtcTime::parse(at.as_str()).ok())
        .is_some_and(|at| at <= now)
}

/// The panel's first row: `◇ Project  2 wait for review · 3 in use`.
#[must_use]
pub fn project_header(view: &ProjectView<'_>, width: usize, look: &Look) -> Line {
    let theme = &look.theme;
    let waiting = view.entries.iter().filter(|entry| waits(entry)).count();
    let in_use = view.entries.len() - waiting;
    let mut parts = Vec::new();
    if view.reading {
        parts.push((format!("reading{}", look.glyphs.ellipsis), Token::Muted));
    } else {
        if waiting > 0 {
            let noun = if waiting == 1 {
                "entry waits"
            } else {
                "entries wait"
            };
            parts.push((format!("{waiting} {noun} for review"), Token::Warning));
        }
        if in_use > 0 || waiting == 0 {
            parts.push((format!("{in_use} in use"), Token::Muted));
        }
    }
    let mut rest = vec![Span::plain("  ")];
    for (at, (text, token)) in parts.into_iter().enumerate() {
        if at > 0 {
            rest.push(Span::new(
                format!(" {} ", look.glyphs.separator),
                theme.style(Token::Muted),
            ));
        }
        rest.push(Span::new(text, theme.style(token)));
    }
    panel::header("Project", &rest, width, look)
}

/// The panel collapsed: its first row.
#[must_use]
pub fn project_collapsed(view: &ProjectView<'_>, width: usize, look: &Look) -> Vec<Line> {
    vec![project_header(view, width, look)]
}

/// The panel drawn whole: its first row, then each entry's row, the
/// `focused` one drawn as the selection and followed by its detail, then
/// the problems of the project's files.
#[must_use]
pub fn project_rows(
    view: &ProjectView<'_>,
    focused: Option<usize>,
    width: usize,
    look: &Look,
) -> Vec<PanelRow> {
    let muted = look.theme.style(Token::Muted);
    let mut rows = vec![PanelRow::plain(project_header(view, width, look))];
    if !view.reading && view.entries.is_empty() {
        rows.push(PanelRow::plain(cut(
            &Line::plain("  ").with(Span::new(
                "Nothing is in use or waiting for this project.",
                muted,
            )),
            width,
            look.glyphs.ellipsis,
        )));
    }
    for (index, entry) in view.entries.iter().enumerate() {
        let focus = focused == Some(index);
        rows.push(PanelRow::of(
            index,
            entry_row(entry, focus, view.now, width, look),
        ));
        if focus {
            let asking = view.asking == Some(index);
            let asking = asking.then_some(view.yes);
            for line in detail(entry, asking, view.now, view.trusted, width, look) {
                rows.push(PanelRow::of(index, line));
            }
        }
    }
    for problem in view.problems {
        rows.push(PanelRow::plain(cut(
            &Line::plain("  ").with(Span::new(
                format!("problem: {}", one_line(problem)),
                look.theme.style(Token::Warning),
            )),
            width,
            look.glyphs.ellipsis,
        )));
    }
    rows
}

/// An entry's row: its mark, its kind, its text cut with `…`, and its age
/// at the row's end.
#[must_use]
pub fn entry_row(
    entry: &ProjectEntry,
    focused: bool,
    now: UtcTime,
    width: usize,
    look: &Look,
) -> Line {
    let theme = &look.theme;
    let glyphs = look.glyphs;
    let (mark, token) = if waits(entry) {
        (glyphs.waiting, Token::Warning)
    } else if entry.status == EntryStatus::Active && !expired(entry, now) {
        (glyphs.succeeded, Token::Success)
    } else {
        (glyphs.denied, Token::Muted)
    };
    let text = match (&entry.command, &entry.kind) {
        (Some(command), EntryKind::Command) => format!(
            "{}: {}",
            one_line(command.purpose.as_str()),
            one_line(&command.command)
        ),
        _ => one_line(&entry.text),
    };
    let text_token = if focused {
        Token::Selected
    } else {
        Token::Text
    };
    let left = Line::plain("  ")
        .with(Span::new(mark, theme.style(token)))
        .with(Span::plain(" "))
        .with(Span::new(
            one_line(entry.kind.as_str()),
            theme.style(Token::Muted),
        ))
        .with(Span::plain("  "))
        .with(Span::new(text, theme.style(text_token)));
    let age = entry
        .source
        .at
        .as_ref()
        .and_then(|at| age(at.as_str(), now));
    let Some(age) = age else {
        return cut(&left, width, glyphs.ellipsis);
    };
    spread(
        &left,
        &Line::styled(age, theme.style(Token::Muted)),
        width,
        2,
        glyphs.ellipsis,
    )
}

/// The focused entry's detail, four columns in, wrapped at the width and
/// never cut: what it runs, what for and where, whether it only reads,
/// where it came from, its standing (`trusted` when the project is trusted
/// as it is), what accepting it keeps it until, its check and its
/// evidence; and, when its deletion is asked (`asking`, with the key that
/// answers it), the question. Everything a
/// `y` acts on is here: the entry's row, which is cut, is not needed.
#[must_use]
pub fn detail(
    entry: &ProjectEntry,
    asking: Option<Option<&str>>,
    now: UtcTime,
    trusted: bool,
    width: usize,
    look: &Look,
) -> Vec<Line> {
    let theme = &look.theme;
    let text = theme.style(Token::Text);
    let muted = theme.style(Token::Muted);
    let mut paragraphs: Vec<Vec<Span>> = Vec::new();
    match &entry.command {
        Some(command) => {
            paragraphs.push(vec![
                Span::new("runs  ", muted),
                Span::new(one_line(&command.command), theme.style(Token::Code)),
            ]);
            let mut where_ = vec![format!("for {}", one_line(command.purpose.as_str()))];
            if let Some(cwd) = command.cwd.as_deref().filter(|cwd| !cwd.is_empty()) {
                where_.push(format!("in {}", one_line(cwd)));
            }
            where_.push(
                if command.read_only {
                    "only reads"
                } else {
                    "may change files"
                }
                .to_owned(),
            );
            paragraphs.push(vec![Span::new(
                where_.join(&format!(" {} ", look.glyphs.separator)),
                muted,
            )]);
            if !entry.text.is_empty() {
                paragraphs.push(vec![Span::new(one_line(&entry.text), text)]);
            }
        }
        None => paragraphs.push(vec![Span::new(one_line(&entry.text), text)]),
    }
    paragraphs.push(vec![Span::new(origin(entry), muted)]);
    let (standing, token) = standing(entry, now, trusted);
    paragraphs.push(vec![Span::new(standing, theme.style(token))]);
    if acceptable(entry, now)
        && let Some(until) = kept_until(entry, now)
    {
        paragraphs.push(vec![Span::new(
            format!("accepting keeps it until {}", day_of(until)),
            muted,
        )]);
    }
    if let Some(check) = &entry.check {
        let field = |field: &Option<String>| one_line(field.as_deref().unwrap_or(""));
        let shown = match &check.kind {
            EntryCheckKind::Exists => format!("`{}` exists", field(&check.path)),
            EntryCheckKind::Contains => format!(
                "`{}` contains `{}`",
                field(&check.path),
                field(&check.pattern)
            ),
            EntryCheckKind::Command => format!("`{}` succeeds", field(&check.command)),
            other => one_line(other.as_str()),
        };
        paragraphs.push(vec![
            Span::new("check  ", muted),
            Span::new(format!("{shown}, not run in this build"), muted),
        ]);
    }
    if let Some(evidence) = entry.evidence.as_deref().filter(|e| !e.is_empty()) {
        paragraphs.push(vec![
            Span::new("evidence  ", muted),
            Span::new(one_line(evidence), muted),
        ]);
    }
    // The question names the key that answers it, as the key map binds it.
    if let Some(yes) = asking {
        let question = match yes {
            Some(yes) => format!(
                "delete this entry? {} deletes {} any other key keeps it",
                one_line(yes),
                look.glyphs.separator
            ),
            None => "delete this entry? no key is bound to accept it: any key keeps it".to_owned(),
        };
        paragraphs.push(vec![Span::new(question, theme.style(Token::Warning))]);
    }
    // Four columns in, where the window has room for them.
    let indent = if width >= 12 { 4 } else { 0 };
    let room = (width - indent).max(1);
    let mut lines = Vec::new();
    for spans in paragraphs {
        for line in wrap(&spans, room) {
            lines.push(concat(&Line::plain(" ".repeat(indent)), &line));
        }
    }
    lines
}

/// Where `entry` came from, as `cortex project` says it.
fn origin(entry: &ProjectEntry) -> String {
    let learned = || {
        let mut words = String::new();
        if let Some(session) = &entry.source.session_id {
            words.push_str(&format!(" in session {}", one_line(session.as_str())));
        }
        if let Some(at) = &entry.source.at {
            let day = at.as_str().get(..10).unwrap_or(at.as_str());
            words.push_str(&format!(" on {}", one_line(day)));
        }
        words
    };
    match (&entry.scope, &entry.basis, &entry.source.kind) {
        (EntryScope::Shared, _, _) => "from the repository's file".to_owned(),
        (_, Some(EntryBasis::Observed), _) => {
            let seen = match entry.observations {
                Some(1) => "observed once".to_owned(),
                Some(count) => format!("observed {count} times"),
                None => "observed".to_owned(),
            };
            format!("{seen}, learned{}", learned())
        }
        (_, Some(EntryBasis::Proposed), _) => format!("proposed by the model{}", learned()),
        (_, _, EntrySourceKind::User) => "you wrote it".to_owned(),
        _ => format!("learned{}", learned()),
    }
}

/// The entry's standing, saying no more than the engine will do with it,
/// and the token to draw it in. An entry `y` would accept says no expiry:
/// accepting gives one of its own, said after.
fn standing(entry: &ProjectEntry, now: UtcTime, trusted: bool) -> (String, Token) {
    let day =
        |at: &cortex_protocol::Timestamp| one_line(at.as_str().get(..10).unwrap_or(at.as_str()));
    let until = entry
        .expires_at
        .as_ref()
        .filter(|_| !acceptable(entry, now))
        .map(|at| format!(", until {}", day(at)))
        .unwrap_or_default();
    let ended = || entry.expires_at.as_ref().map(day).unwrap_or_default();
    match &entry.status {
        EntryStatus::Proposed if expired(entry, now) => (
            format!(
                "waits for your review: not in use, and it expired on {}",
                ended()
            ),
            Token::Warning,
        ),
        EntryStatus::Proposed => (
            "waits for your review: not in use".to_owned(),
            Token::Warning,
        ),
        EntryStatus::Disabled => ("turned off".to_owned(), Token::Muted),
        EntryStatus::Active if expired(entry, now) => {
            (format!("expired on {}: not in use", ended()), Token::Muted)
        }
        EntryStatus::Active if entry.kind == EntryKind::Command && !accepted(entry) => (
            "in use, and still asks before it runs".to_owned(),
            Token::Warning,
        ),
        // Accepted, it runs without asking only in a project trusted as
        // it is.
        EntryStatus::Active if entry.kind == EntryKind::Command && !trusted => (
            format!(
                "in use and accepted, and still asks before it runs: this project is not \
                 trusted as it is{until}"
            ),
            Token::Warning,
        ),
        EntryStatus::Active if entry.kind == EntryKind::Command => (
            format!("in use and accepted: it runs without asking{until}"),
            Token::Muted,
        ),
        EntryStatus::Active => (format!("in use{until}"), Token::Muted),
        other => (one_line(other.as_str()), Token::Muted),
    }
}

/// How long ago `at` was, at `now`, in a few columns: `now`, `5m`, `3h`,
/// `2d`. Nothing for a time that cannot be read, or one after `now`.
#[must_use]
pub fn age(at: &str, now: UtcTime) -> Option<String> {
    let at = UtcTime::parse(at).ok()?;
    let secs = now.unix_secs().checked_sub(at.unix_secs())?;
    if secs < 0 {
        return None;
    }
    Some(match secs {
        0..60 => "now".to_owned(),
        60..3_600 => format!("{}m", secs / 60),
        3_600..86_400 => format!("{}h", secs / 3_600),
        _ => format!("{}d", secs / 86_400),
    })
}

#[cfg(test)]
pub(crate) mod tests;
