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
//! for each reason it is asked. The detail is exactly what would happen, a
//! command wrapped and never shortened, a diff with its gutter; detail of
//! any kind taller than the room it is given is clipped with a line that
//! says how much more there is, and the drawing says it was clipped, so
//! that the caller offers no lasting answer for what was not seen. The
//! answers are on one line, each key in the accent. Everything from
//! outside passes through `cineris_term::sanitize`.

use cineris_protocol::lease::Effect;
use cineris_protocol::methods::{
    ApprovalDetail, ApprovalRequest, QuestionRequest, RewindFile, RewindFileStatus, RewindTarget,
    RewindWhyNot, SessionRewindPlanResult, TrustRequest, TrustResourceKind,
};
use cineris_term::{Line, Span, Style, sanitize};

use super::round::diff_rows_cut;
use super::{DiffLine, Look};
use crate::layout::{cut, pad, wrap};
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
    /// The answers on offer, in order, wrapped onto as many rows as they
    /// need.
    pub answers: Vec<Answer>,
    /// The answers cannot be pressed yet: their rows are left blank, as
    /// many as they would take.
    pub answers_hidden: bool,
    /// A diff's lines, parsed from the request's detail.
    pub diff: &'a [DiffLine],
    /// The most rows the detail may take; the rest is said, not shown.
    pub detail_rows: usize,
    /// The note being written, after "deny with a note".
    pub note: Option<&'a str>,
    /// It is a request to leave the sandbox: a command that would run
    /// outside it in a session whose commands are confined.
    pub leaving: bool,
    /// No lasting answer is offered (the dialog did not fit the window), so
    /// what one would allow is not said.
    pub withheld: bool,
}

/// The dialog's rows, and what of it did not fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawn {
    /// The rows.
    pub lines: Vec<Line>,
    /// Some of the detail did not fit: rows of it, or the end of a diff's
    /// line.
    pub clipped: bool,
    /// How many of the rows are the question's: the first.
    pub question: usize,
    /// How many of the rows are the detail's, the line that says it was
    /// clipped included.
    pub detail: usize,
    /// How many of the rows are the answers': the last.
    pub answer_rows: usize,
    /// For each answer, whether it was drawn whole.
    pub whole: Vec<bool>,
}

/// Whether the request is for a command that would run outside the
/// sandbox. In a session whose commands are confined, that is a request to
/// leave it; in one whose sandbox is off, every command runs so.
#[must_use]
pub fn outside_sandbox(request: &ApprovalRequest) -> bool {
    matches!(&request.detail, ApprovalDetail::Command(command) if command.outside_sandbox)
}

/// The approval dialog, `width` columns wide at most.
#[must_use]
pub fn approval(
    request: &ApprovalRequest,
    view: &ApprovalView<'_>,
    width: usize,
    look: &Look,
) -> Drawn {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    // Sentences wrap at the prose width, as the transcript's do; what would
    // run, a path and a diff take the whole width.
    let prose = look.prose_width.min(width);
    let mut lines = Vec::new();
    let question = if view.leaving {
        "Run outside the sandbox?".to_owned()
    } else {
        format!("{}?", sanitize(&request.summary).replace(['\n', '\t'], " "))
    };
    // The question is wrapped, never cut: it says what is asked.
    for (index, row) in wrap(
        &[Span::new(question, theme.style(Token::Title))],
        width.saturating_sub(2).max(1),
    )
    .into_iter()
    .enumerate()
    {
        let mut line = if index == 0 {
            Line::from(Span::new(look.glyphs.waiting, theme.style(Token::Warning)))
                .with(Span::plain(" "))
        } else {
            Line::plain("  ")
        };
        for span in row.spans() {
            line.push(span.clone());
        }
        lines.push(line);
    }
    let question_rows = lines.len();
    if !view.leaving {
        for reason in &request.reasons {
            lines.extend(indented(&sanitize(reason), muted, 2, prose));
        }
    }
    lines.push(Line::new());

    let mut detail: Vec<Line> = Vec::new();
    let mut after: Vec<Line> = Vec::new();
    // A diff's line wider than the window loses its end.
    let mut sideways = false;
    match &request.detail {
        ApprovalDetail::Command(command) => {
            let accent = theme.style(Token::AccentText);
            // Exactly what would run, line breaks and all.
            for line in sanitize(&command.command).split('\n') {
                detail.extend(indented(line, accent, 4, width));
            }
            after.extend(indented(
                &format!("in {}", sanitize(&command.cwd)),
                muted,
                4,
                width,
            ));
            if view.leaving {
                after.push(Line::new());
                after.extend(indented(
                    "It would run with your full rights: it can write anywhere you can, and use \
                     the network. This is asked every time.",
                    theme.style(Token::Warning),
                    2,
                    prose,
                ));
            }
        }
        ApprovalDetail::Diff(diff) => {
            if view.diff.is_empty() {
                detail.extend(indented(
                    &sanitize(&diff.path),
                    theme.style(Token::Code),
                    4,
                    width,
                ));
                let why = diff
                    .omitted
                    .as_deref()
                    .unwrap_or("The change has no lines to show.");
                detail.extend(indented(&sanitize(why), muted, 4, prose));
            } else {
                let (rows, cut_any) = diff_rows_cut(view.diff, width.saturating_sub(1), look);
                sideways = cut_any;
                for (row, fill) in rows {
                    let mut line = Line::plain(" ");
                    for span in row.spans() {
                        line.push(span.clone());
                    }
                    let line = pad(line, width, fill);
                    detail.push(cut(&line, width, look.glyphs.ellipsis));
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
            detail.extend(indented(
                &format!("{what} {}", sanitize(&target.target)),
                theme.style(Token::Code),
                4,
                width,
            ));
        }
        ApprovalDetail::Tool(tool) => {
            let text = serde_json::to_string_pretty(&tool.arguments).unwrap_or_default();
            for line in sanitize(&text).split('\n') {
                detail.extend(indented(line, theme.style(Token::Code), 4, width));
            }
        }
        _ => detail.extend(indented(
            "This build cannot show what the call would do.",
            muted,
            4,
            width,
        )),
    }
    // Clipped to its room: the first rows, and one that says how many more.
    let room = view.detail_rows.max(2);
    let mut clipped = sideways;
    if detail.len() > room {
        clipped = true;
        let hidden = detail.len() - (room - 1);
        detail.truncate(room - 1);
        detail.extend(indented(
            &format!(
                "{} {hidden} more lines are not shown, so only this call can be allowed",
                look.glyphs.ellipsis
            ),
            muted,
            4,
            prose,
        ));
    }
    let detail_rows = detail.len();
    lines.extend(detail);
    lines.extend(after);

    // What "this session" and "here" would allow from then on, in full.
    if !request.adds.is_empty() && !view.leaving && !clipped && !view.withheld {
        lines.push(Line::new());
        let rules: Vec<String> = request
            .adds
            .iter()
            .map(|rule| format!("{} {}", effect_word(&rule.effect), sanitize(&rule.pattern)))
            .collect();
        lines.extend(indented(
            &format!("Allowing it for longer allows: {}", rules.join(", ")),
            muted,
            2,
            prose,
        ));
    }

    lines.push(Line::new());
    let (answer_lines, whole) = match view.note {
        Some(note) => (
            vec![note_row("Note for the model: ", note, width, look)],
            Vec::new(),
        ),
        None => answer_rows(&view.answers, view.answers_hidden, width, look),
    };
    let answer_rows = answer_lines.len();
    lines.extend(answer_lines);
    Drawn {
        lines,
        clipped,
        question: question_rows,
        detail: detail_rows,
        answer_rows,
        whole,
    }
}

/// The answers, wrapped between answers onto as many rows as they need,
/// and for each whether it was drawn whole. Hidden, they are blank rows.
fn answer_rows(
    answers: &[Answer],
    hidden: bool,
    width: usize,
    look: &Look,
) -> (Vec<Line>, Vec<bool>) {
    const GAP: &str = "   ";
    let key_style = look.theme.style(Token::Key);
    let mut rows: Vec<Line> = Vec::new();
    let mut whole = Vec::with_capacity(answers.len());
    let mut row = Line::plain("  ");
    let mut items = 0;
    for answer in answers {
        let item = Line::from(Span::new(answer.key.clone(), key_style))
            .with(Span::plain(format!(" {}", answer.label)));
        let gap = if items == 0 { 0 } else { GAP.len() };
        if items > 0 && row.width() + gap + item.width() > width {
            rows.push(std::mem::replace(&mut row, Line::plain("  ")));
            items = 0;
        }
        if items > 0 {
            row.push(Span::plain(GAP));
        }
        whole.push(row.width() + item.width() <= width);
        for span in item.spans() {
            row.push(span.clone());
        }
        items += 1;
    }
    rows.push(row);
    let rows = rows
        .into_iter()
        .map(|row| {
            if hidden {
                Line::new()
            } else {
                cut(&row, width, look.glyphs.ellipsis)
            }
        })
        .collect();
    (rows, whole)
}

/// The note being written after `label`, its end shown when it is longer
/// than the row.
fn note_row(label: &str, note: &str, width: usize, look: &Look) -> Line {
    let text = sanitize(note).replace(['\n', '\t'], " ");
    let room = width.saturating_sub(2 + label.len()).max(1);
    let rows = wrap(&[Span::plain(text)], room.saturating_sub(2).max(1));
    let mut row = Line::plain("  ").with(Span::new(label, look.theme.style(Token::Title)));
    if rows.len() > 1 {
        row.push(Span::plain(format!("{} ", look.glyphs.ellipsis)));
    }
    if let Some(last) = rows.last() {
        for span in last.spans() {
            row.push(span.clone());
        }
    }
    cut(&row, width, look.glyphs.ellipsis)
}

/// What the trust question shows beside its request.
#[derive(Debug, Clone, Default)]
pub struct TrustView {
    /// The answers on offer, in order.
    pub answers: Vec<Answer>,
    /// The answers cannot be pressed yet: their rows are left blank.
    pub answers_hidden: bool,
    /// The most rows the list of what trust turns on may take.
    pub detail_rows: usize,
    /// The line of what each answer does is left out, in a window too
    /// small for it and the list whole.
    pub unsaid: bool,
}

/// The third answer to a trust question: never, for a project not
/// trusted; for one trusted already, keep trusting it without what is new
/// in it, which never would have taken away (the follow-ups of step 23).
#[must_use]
pub fn third_trust_answer(request: &TrustRequest) -> &'static str {
    if !request.trusted {
        return "Never for this project";
    }
    let git = request
        .resources
        .iter()
        .any(|resource| resource.kind == TrustResourceKind::Repository);
    let rules = request.resources.iter().any(|resource| resource.changed);
    match (git, rules) {
        (true, true) => "Keep trusting it, without these",
        (true, false) => "Keep trusting it, without git",
        _ => "Keep trusting it, without the new rules",
    }
}

/// What each answer to a trust question does, said under the answers: of
/// "Trust it" only when it is offered (a list that does not fit offers it
/// not).
fn trust_answers_said(request: &TrustRequest, offered: bool) -> String {
    let trust = if offered {
        "Trust it turns these on; "
    } else {
        ""
    };
    let rest = if request.trusted {
        "not now asks again at the next start; keep trusting it leaves them off and asks no \
         more until they change."
    } else {
        "not now asks again at the next start; never asks no more, until `cineris permissions \
         trust`."
    };
    let said = format!("{trust}{rest}");
    let mut first = said.chars();
    first
        .next()
        .map(|letter| letter.to_uppercase().chain(first).collect())
        .unwrap_or_default()
}

/// The trust question, `width` columns wide at most (reference screen 2c,
/// `docs/architecture/tui-look/2c.txt`: the question, the folder, then
/// each thing trust turns on in full, the answers and what each does).
///
/// ```text
/// ? Trust this project?
///   ~/src/parser
///   Trusting it turns on what its files ask for:
///     git      Git may read this repository's own configuration, …
///     rule     allow write .github/**
///     rule     allow exec make deploy (profile ci)
///     setting  model.main = "claude-haiku-5-5" (it can raise what you spend)
///
///   y Trust it   n Not now   x Never for this project
///   Trust it turns these on, and asks again if they change. Not now …
/// ```
///
/// A project trusted already is asked about what is new in it ("Trust
/// what is new in this project?"), and its third answer keeps the trust it
/// had ([`third_trust_answer`]).
#[must_use]
pub fn trust(request: &TrustRequest, view: &TrustView, width: usize, look: &Look) -> Drawn {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    let prose = look.prose_width.min(width);
    let mut lines = title(
        if request.trusted {
            "Trust what is new in this project?"
        } else {
            "Trust this project?"
        },
        width,
        look,
    );
    let question = lines.len();
    lines.extend(indented(
        &sanitize(&request.workspace),
        theme.style(Token::Code),
        2,
        width,
    ));
    lines.extend(indented(
        if request.trusted {
            "You trust this project. Trusting what is new in it turns on:"
        } else {
            "Trusting it turns on what its files ask for:"
        },
        muted,
        2,
        prose,
    ));
    const LABEL: usize = 9;
    let mut detail: Vec<Line> = Vec::new();
    let mut changed = false;
    for resource in &request.resources {
        changed |= resource.changed;
        let detail_text = sanitize(&resource.detail).into_owned();
        let (label, text) = match &resource.kind {
            TrustResourceKind::Repository => ("git", detail_text),
            TrustResourceKind::Rules => {
                let profile = resource
                    .name
                    .strip_prefix("profiles.")
                    .and_then(|rest| rest.strip_suffix(".permissions.rules"));
                (
                    "rule",
                    match profile {
                        Some(profile) => format!("{detail_text} (profile {})", sanitize(profile)),
                        None => detail_text,
                    },
                )
            }
            TrustResourceKind::Settings => ("setting", detail_text),
            TrustResourceKind::Hook => ("hook", detail_text),
            TrustResourceKind::Extension => ("extension", detail_text),
            TrustResourceKind::McpServer => ("mcp", detail_text),
            TrustResourceKind::Commands => ("commands", detail_text),
            _ => ("other", detail_text),
        };
        let rows = wrap(
            &[Span::new(text, theme.style(Token::AccentText))],
            width.saturating_sub(4 + LABEL).max(1),
        );
        for (index, row) in rows.into_iter().enumerate() {
            let mut line = Line::plain("    ");
            if index == 0 {
                line.push(Span::new(format!("{label:<LABEL$}"), muted));
            } else {
                line.push(Span::plain(" ".repeat(LABEL)));
            }
            for span in row.spans() {
                line.push(span.clone());
            }
            detail.push(line);
        }
    }
    let room = view.detail_rows.max(2);
    let mut clipped = false;
    if detail.len() > room {
        clipped = true;
        let hidden = detail.len() - (room - 1);
        detail.truncate(room - 1);
        detail.extend(indented(
            &format!(
                "{} {hidden} more lines are not shown, so it cannot be trusted from here: \
                 `cineris permissions trust` lists them all",
                look.glyphs.ellipsis
            ),
            muted,
            4,
            prose,
        ));
    }
    let detail_rows = detail.len();
    lines.extend(detail);
    if changed {
        lines.extend(indented(
            "Its rules have changed since you trusted it.",
            theme.style(Token::Warning),
            2,
            prose,
        ));
    }
    lines.push(Line::new());
    let (answer_lines, whole) = answer_rows(&view.answers, view.answers_hidden, width, look);
    let mut answer_lines = answer_lines;
    if !view.unsaid {
        answer_lines.extend(indented(
            &trust_answers_said(request, !clipped),
            muted,
            2,
            prose,
        ));
    }
    let answer_rows = answer_lines.len();
    lines.extend(answer_lines);
    Drawn {
        lines,
        clipped,
        question,
        detail: detail_rows,
        answer_rows,
        whole,
    }
}

/// What a question of the core's shows beside its request.
#[derive(Debug, Clone, Default)]
pub struct QuestionView<'a> {
    /// The answers on offer, in order: its choices, by their digits.
    pub answers: Vec<Answer>,
    /// The answers cannot be pressed yet: their rows are left blank.
    pub answers_hidden: bool,
    /// The most rows the question may take below its first.
    pub detail_rows: usize,
    /// The answer being written, to a question with no choices.
    pub note: Option<&'a str>,
}

/// A question of the core's (an undo's, about files edited since: reference
/// screen 2d), `width` columns wide at most: the question, then its choices
/// by their digits.
#[must_use]
pub fn question(
    request: &QuestionRequest,
    view: &QuestionView<'_>,
    width: usize,
    look: &Look,
) -> Drawn {
    let mut lines = title(&request.question, width, look);
    let question = lines.len();
    lines.push(Line::new());
    let (answer_lines, whole) = match view.note {
        Some(note) => (
            vec![note_row("Your answer: ", note, width, look)],
            Vec::new(),
        ),
        None => answer_rows(&view.answers, view.answers_hidden, width, look),
    };
    let answer_rows = answer_lines.len();
    lines.extend(answer_lines);
    Drawn {
        lines,
        clipped: false,
        question,
        detail: 0,
        answer_rows,
        whole,
    }
}

/// The rewind's question (since step 25; `docs/steps/25/answered.md`,
/// point 4): the point, its prompt, and the core's own plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindAsk {
    /// The turn rewound to before, counted from one along the active
    /// branch.
    pub turn: u64,
    /// The prompt of `turn`, whose first line is shown.
    pub prompt: String,
    /// What the core would do (`session/rewind_plan`).
    pub plan: SessionRewindPlanResult,
}

/// What the rewind's question shows beside its plan.
#[derive(Debug, Clone, Default)]
pub struct RewindView {
    /// The answers on offer, by their digits.
    pub answers: Vec<Answer>,
    /// The answers cannot be pressed yet: their rows are left blank.
    pub answers_hidden: bool,
    /// The most rows the plan's files may take; the rest is said, not
    /// shown.
    pub detail_rows: usize,
}

/// The rewind's question, `width` columns wide at most: the point, the
/// prompt's first line, how many turns go, each file of the plan with what
/// happens to it, and the answers by their digits.
///
/// ```text
/// ? Rewind to before turn 12?
///   fix the lexer so digit separators…
///   The plan undoes 3 turns. They changed these files:
///     src/lexer.rs  goes back to what it was before
///     src/new.rs  is removed: it did not exist then
///     notes.txt  was changed since: you are asked before it is put back
///
///   1 Rewind it and put back the files listed
///   2 Rewind the conversation only; the files stay as they are
/// ```
///
/// A file list taller than its room is cut, with a line that says so, and
/// the drawing says it was clipped: the caller then offers `1` only once
/// the plan has been seen whole in the viewer.
#[must_use]
pub fn rewind(ask: &RewindAsk, view: &RewindView, width: usize, look: &Look) -> Drawn {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    let prose = look.prose_width.min(width);
    let mut lines = title(&format!("Rewind to before turn {}?", ask.turn), width, look);
    let question = lines.len();
    // The prompt's first line, cut at the row: it says which point this is.
    let first = ask.prompt.lines().next().unwrap_or_default();
    let more = ask.prompt.trim_end().contains('\n');
    let prompt = format!(
        "{}{}",
        sanitize(first).replace('\t', " "),
        if more { look.glyphs.ellipsis } else { "" }
    );
    lines.push(cut(
        &Line::plain("  ").with(Span::new(prompt, theme.style(Token::AccentText))),
        width,
        look.glyphs.ellipsis,
    ));
    // The core's own count, which its mark holds: a turn run since the
    // points were read is in it.
    let undone = ask.plan.turns.len();
    let turns = if undone == 1 {
        "The plan undoes one turn.".to_owned()
    } else {
        format!("The plan undoes {undone} turns.")
    };
    let files = match (ask.plan.files.is_empty(), undone == 1) {
        (true, true) => " It changed no file the harness recorded.",
        (true, false) => " They changed no file the harness recorded.",
        (false, true) => " It changed these files:",
        (false, false) => " They changed these files:",
    };
    lines.extend(indented(&format!("{turns}{files}"), muted, 2, prose));
    if ask.plan.unreadable {
        lines.extend(indented(
            "A checkpoint of those turns cannot be read, so no file is put back.",
            theme.style(Token::Warning),
            2,
            prose,
        ));
    }
    let mut detail: Vec<Line> = Vec::new();
    for file in &ask.plan.files {
        // The name from outside, on one line, wrapped and never cut; then
        // what happens to it.
        let name = sanitize(&file.path).replace(['\n', '\t'], " ");
        let rows = wrap(
            &[
                Span::new(name, theme.style(Token::Code)),
                Span::plain("  "),
                Span::new(what_happens(file), muted),
            ],
            width.saturating_sub(4).max(1),
        );
        for row in rows {
            let mut line = Line::plain("    ");
            for span in row.spans() {
                line.push(span.clone());
            }
            detail.push(line);
        }
    }
    let room = view.detail_rows.max(2);
    let mut clipped = false;
    if detail.len() > room {
        clipped = true;
        let hidden = detail.len() - (room - 1);
        detail.truncate(room - 1);
        detail.extend(indented(
            &format!(
                "{} {hidden} more lines are not shown, so the files are not put back from here \
                 until the plan is seen whole",
                look.glyphs.ellipsis
            ),
            muted,
            4,
            prose,
        ));
    }
    let detail_rows = detail.len();
    lines.extend(detail);
    lines.push(Line::new());
    let (answer_lines, whole) = answer_rows(&view.answers, view.answers_hidden, width, look);
    let answer_rows = answer_lines.len();
    lines.extend(answer_lines);
    Drawn {
        lines,
        clipped,
        question,
        detail: detail_rows,
        answer_rows,
        whole,
    }
}

/// What a rewind does to one file of its plan, in words.
fn what_happens(file: &RewindFile) -> String {
    let why = |why: Option<&RewindWhyNot>| match why {
        Some(RewindWhyNot::NotRecorded) => "what it held before was not recorded",
        Some(RewindWhyNot::TooLarge) => "its earlier content was too large to keep",
        Some(RewindWhyNot::Missing) => "its earlier content is no longer kept",
        Some(RewindWhyNot::Unreadable) => "a checkpoint of the turns cannot be read",
        Some(RewindWhyNot::ThroughALink) => "it is now reached through a link",
        _ => "it cannot be put back",
    };
    match (&file.status, &file.to) {
        (RewindFileStatus::Cannot, _) | (_, RewindTarget::NotRecorded) => {
            format!("stays as it is: {}", why(file.why_not.as_ref()))
        }
        (RewindFileStatus::Already, _) => "is already as it was".to_owned(),
        (RewindFileStatus::EditedSince, RewindTarget::Removed) => {
            "was changed since, and would be removed: you are asked first".to_owned()
        }
        (RewindFileStatus::EditedSince, _) => {
            "was changed since: you are asked before it is put back".to_owned()
        }
        (RewindFileStatus::AsLeft, RewindTarget::Removed) => {
            "is removed: it did not exist then".to_owned()
        }
        (RewindFileStatus::AsLeft, _) => "goes back to what it was before".to_owned(),
        _ => "cannot be told: this build does not know what the plan says".to_owned(),
    }
}

/// A dialog's question after its `?`, wrapped and never cut.
fn title(text: &str, width: usize, look: &Look) -> Vec<Line> {
    let theme = &look.theme;
    let text = sanitize(text).replace(['\n', '\t'], " ");
    wrap(
        &[Span::new(text, theme.style(Token::Title))],
        width.saturating_sub(2).max(1),
    )
    .into_iter()
    .enumerate()
    .map(|(index, row)| {
        let mut line = if index == 0 {
            Line::from(Span::new(look.glyphs.waiting, theme.style(Token::Warning)))
                .with(Span::plain(" "))
        } else {
            Line::plain("  ")
        };
        for span in row.spans() {
            line.push(span.clone());
        }
        line
    })
    .collect()
}

/// The rows of a dialog's detail, wrapped at `width` and set `indent`
/// columns in.
fn indented(text: &str, style: Style, indent: usize, width: usize) -> Vec<Line> {
    let room = width.saturating_sub(indent).max(1);
    let pad = " ".repeat(indent);
    wrap(&[Span::new(text, style)], room)
        .into_iter()
        .map(|row| {
            let mut line = Line::plain(pad.clone());
            for span in row.spans() {
                line.push(span.clone());
            }
            line
        })
        .collect()
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
