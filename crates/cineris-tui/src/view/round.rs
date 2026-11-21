//! A tool round in its box (`docs/architecture/09-tui.md`, "Tool rounds"):
//! one line per call, and each call's detail (a diff, or the output's last
//! lines) under it when the box is open.
//!
//! The box is drawn the same way wherever it is; [`Placement`] says where,
//! which decides only its bottom border's label and how much detail it may
//! show. While the round runs its box is in the live region; when its last
//! call ends, it is committed as it is ([`Round::is_running`]).

use std::time::Duration;

use cineris_base::text::expand_tabs;
use cineris_term::{Line, Span, Style, sanitize};

use super::{Look, ToolOutput, one_line, precise as duration};
use crate::layout::cut;
use crate::rounded::RoundedBox;
use crate::theme::Token;

/// One round of tool calls: the calls the model asked for together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Round {
    /// The calls, in the order the model gave them.
    pub calls: Vec<Call>,
    /// `Ctrl+T` while the round runs: `Some(true)` opens every detail,
    /// `Some(false)` closes them all, `None` leaves each to `tui.tool_output`.
    pub open: Option<bool>,
}

/// One tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// The tool as an imperative (`Read`, `Edit`, `Run`, `Search`), or an
    /// extension's tool by its own name.
    pub verb: String,
    /// What it acts on.
    pub target: Target,
    /// Where it is.
    pub state: CallState,
    /// What it measured, after the target.
    pub measure: Measure,
    /// How long it ran, once it has ended: after the measure.
    pub took: Option<Duration>,
    /// What can be shown under it.
    pub detail: Detail,
    /// The hook that put its arguments in place of the model's, when one
    /// did and the call was put to the user with them (since step 28): the
    /// target is then the hook's.
    pub hook: Option<String>,
}

/// What a call acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Nothing named.
    None,
    /// A path: shown by its file name, the whole path being in the viewer.
    Path(String),
    /// Text shown as it is: a command, a search.
    Text(String),
}

/// Where a call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// Running for this long; the spinner's frame is the round's tick.
    Running(Duration),
    /// Waiting for the user's approval.
    Waiting,
    /// It ended well.
    Succeeded,
    /// It ended badly.
    Failed,
    /// The user or a rule refused it.
    Denied,
    /// It was stopped.
    Interrupted,
}

/// What a call measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Measure {
    /// Nothing.
    None,
    /// A range of lines read: `lines 120–178`.
    Lines {
        /// The first line.
        first: u64,
        /// The last line.
        last: u64,
    },
    /// An edit's counts: `+5 −1`.
    Changes {
        /// Lines added.
        added: u64,
        /// Lines removed.
        removed: u64,
    },
    /// Anything else, in words: `59 lines`, `no warnings`, `exit 101`.
    Note(String),
}

/// What can be shown under a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    /// Nothing.
    None,
    /// The lines an edit changed, with context.
    Diff(Vec<DiffLine>),
    /// What a command printed, as it arrived.
    Output(String),
}

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Its number in the file: the old file's for a removed line, the new
    /// one's otherwise.
    pub number: u64,
    /// Whether it was kept, added or removed.
    pub change: Change,
    /// Its text.
    pub text: String,
    /// The bytes of `text` that changed, drawn bold.
    pub emphasis: Vec<std::ops::Range<usize>>,
}

/// What happened to a diff's line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Kept: context.
    Same,
    /// Added.
    Added,
    /// Removed.
    Removed,
}

/// Where a box is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// In the live region, while the round runs: `Ctrl+T` opens or closes
    /// it, and an open box shows at most `tui.tool_detail` rows of detail.
    Live,
    /// In the scrollback, as it was when the round ended.
    Committed,
    /// In the viewer, where every box starts closed and an open one shows
    /// all its detail.
    Viewer {
        /// The box is the viewer's selection.
        selected: bool,
        /// The user opened it.
        open: bool,
    },
    /// In the full-screen presenter, a committed box a click or a search
    /// turned over (since step 25): open with all its detail, as in the
    /// viewer, or closed.
    Screen {
        /// It is open.
        open: bool,
        /// The search opened it, for the match it shows: its border says
        /// so.
        searched: bool,
    },
}

impl Round {
    /// Whether any call is still running or waiting: until none is, the box
    /// stays live.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.calls
            .iter()
            .any(|call| matches!(call.state, CallState::Running(_) | CallState::Waiting))
    }
}

/// How many of a failure's last lines are shown, and of a running command's.
const FAILURE_TAIL: usize = 3;
const RUNNING_TAIL: usize = 4;

/// The round's box, `width` columns wide. `tick` is the spinner's.
#[must_use]
pub fn round(
    round: &Round,
    placement: Placement,
    tick: usize,
    width: usize,
    look: &Look,
) -> Vec<Line> {
    let theme = &look.theme;
    let border = match placement {
        Placement::Viewer { selected: true, .. } => theme.style(Token::BorderSelected),
        _ => theme.style(Token::Border),
    };
    let rounded = RoundedBox::new(width, look.glyphs, border);
    let room = rounded.inner();
    // Each row with its fill, and whether it is detail.
    let mut rows: Vec<(Line, Style, bool)> = Vec::new();
    let mut shown = false;
    let mut hidden = false;
    for call in &round.calls {
        rows.push((call_line(call, tick, room, look), Style::PLAIN, false));
        let detail = detail_rows(call, placement, room, look);
        if detail.is_empty() {
            continue;
        }
        if is_open(round, call, placement, look) {
            rows.extend(detail.into_iter().map(|(line, fill)| (line, fill, true)));
            shown = true;
        } else {
            hidden = true;
        }
    }
    // An open box in the live region shows at most `tui.tool_detail` rows of
    // detail; the rest is in the viewer.
    if placement == Placement::Live {
        clip_detail(&mut rows, look);
    }
    let rows: Vec<(Line, Style)> = rows
        .into_iter()
        .map(|(line, fill, _)| (line, fill))
        .collect();
    let label = match placement {
        Placement::Live if shown => Some("ctrl+t collapses"),
        Placement::Live if hidden => Some("ctrl+t expands"),
        Placement::Committed if hidden => Some("ctrl+o expands"),
        Placement::Viewer { selected: true, .. } if shown => Some("enter collapses"),
        Placement::Viewer { selected: true, .. } if hidden => Some("enter expands"),
        Placement::Screen { searched: true, .. } => Some("opened by the search"),
        Placement::Screen { .. } if hidden => Some("ctrl+o expands"),
        _ => None,
    }
    .map(|label| Line::styled(label, theme.style(Token::Hint)));
    rounded.draw(&rows, label.as_ref())
}

/// Whether a live box shows any call's detail now: what `Ctrl+T` turns
/// the other way (step 23).
#[must_use]
pub fn shows_detail(round: &Round, look: &Look) -> bool {
    round
        .calls
        .iter()
        .any(|call| call.detail != Detail::None && is_open(round, call, Placement::Live, look))
}

/// Whether a call's detail is open in this box.
fn is_open(round: &Round, call: &Call, placement: Placement, look: &Look) -> bool {
    if let Placement::Viewer { open, .. } | Placement::Screen { open, .. } = placement {
        return open;
    }
    if let Some(open) = round.open {
        return open;
    }
    match look.tool_output {
        ToolOutput::Always => true,
        ToolOutput::Never => false,
        ToolOutput::Auto => match (&call.detail, call.state) {
            (Detail::Output(_), CallState::Failed) => true,
            (Detail::Diff(lines), _) => lines.len() <= look.tool_detail,
            _ => false,
        },
    }
}

/// The open calls' detail, cut to `tui.tool_detail` rows in all: the first
/// rows are kept, and the last row kept says how many more there are.
fn clip_detail(rows: &mut Vec<(Line, Style, bool)>, look: &Look) {
    let detail = rows.iter().filter(|(_, _, is_detail)| *is_detail).count();
    if detail <= look.tool_detail {
        return;
    }
    let keep = look.tool_detail.saturating_sub(1);
    let mut kept = Vec::with_capacity(rows.len());
    let mut seen = 0;
    for (line, fill, is_detail) in rows.drain(..) {
        if !is_detail {
            kept.push((line, fill, false));
        } else if seen < keep {
            seen += 1;
            kept.push((line, fill, true));
        }
    }
    if look.tool_detail > 0 {
        let more = Line::styled(
            format!("  {} {} more lines", look.glyphs.ellipsis, detail - keep),
            look.theme.style(Token::Muted),
        );
        // The marker goes after the last detail kept.
        let at = kept
            .iter()
            .rposition(|(_, _, is_detail)| *is_detail)
            .map_or(kept.len(), |at| at + 1);
        kept.insert(at, (more, Style::PLAIN, true));
    }
    *rows = kept;
}

/// A call's line: its mark, the imperative and target in bold, and its
/// measure.
fn call_line(call: &Call, tick: usize, room: usize, look: &Look) -> Line {
    let theme = &look.theme;
    let g = look.glyphs;
    let (mark, mark_style) = match call.state {
        CallState::Running(_) => (look.spinner(tick), theme.style(Token::Accent)),
        CallState::Waiting => (g.waiting, theme.style(Token::Warning)),
        CallState::Succeeded => (g.succeeded, theme.style(Token::Success)),
        CallState::Failed => (g.failed, theme.style(Token::Error)),
        CallState::Denied => (g.denied, theme.style(Token::Muted)),
        CallState::Interrupted => (g.interrupted, theme.style(Token::Muted)),
    };
    let mut title = one_line(&call.verb);
    match &call.target {
        Target::None => {}
        Target::Path(path) => {
            title.push(' ');
            title.push_str(&one_line(file_name(path)));
        }
        Target::Text(text) => {
            title.push(' ');
            title.push_str(&one_line(text));
        }
    }
    let mut line = Line::from(Span::new(mark, mark_style))
        .with(Span::plain(" "))
        .with(Span::new(title, theme.style(Token::Title)));
    let measure = measure(call, look);
    if measure.width() > 0 {
        line.push(Span::plain("  "));
        for span in measure.spans() {
            line.push(span.clone());
        }
    }
    // It ran with a hook's arguments, which the user allowed.
    if let Some(hook) = &call.hook {
        line.push(Span::plain("  "));
        line.push(Span::new(
            format!("arguments from the hook {}", one_line(hook)),
            theme.style(Token::Warning),
        ));
    }
    cut(&line, room, g.ellipsis)
}

/// A call's measure: what it counted, then how long it ran.
fn measure(call: &Call, look: &Look) -> Line {
    let theme = &look.theme;
    let g = look.glyphs;
    let failed = call.state == CallState::Failed;
    let words = if failed {
        theme.style(Token::Error)
    } else {
        theme.style(Token::Muted)
    };
    let mut line = Line::new();
    match &call.measure {
        Measure::None => {}
        Measure::Lines { first, last } => {
            line.push(Span::new(format!("lines {first}{}{last}", g.range), words));
        }
        Measure::Changes { added, removed } => {
            line.push(Span::new(format!("+{added}"), theme.style(Token::Success)));
            line.push(Span::plain(" "));
            line.push(Span::new(
                format!("{}{removed}", g.minus),
                theme.style(Token::Error),
            ));
        }
        Measure::Note(note) => line.push(Span::new(one_line(note), words)),
    }
    let time = match call.state {
        CallState::Running(elapsed) => Some(elapsed),
        _ => call.took,
    };
    if call.state == CallState::Waiting && line.width() == 0 {
        line.push(Span::new("approval needed", theme.style(Token::Warning)));
    }
    if let Some(time) = time {
        if line.width() > 0 {
            line.push(Span::new(format!(" {} ", g.separator), words));
        }
        line.push(Span::new(duration(time), words));
    }
    line
}

/// The rows of a call's detail, each with the style that fills it to the
/// box's edge.
fn detail_rows(call: &Call, placement: Placement, room: usize, look: &Look) -> Vec<(Line, Style)> {
    match &call.detail {
        Detail::None => Vec::new(),
        Detail::Diff(lines) => diff_rows(lines, room, look),
        Detail::Output(text) => {
            let lines = output_lines(text);
            let keep = match (placement, call.state) {
                (Placement::Viewer { .. } | Placement::Screen { .. }, _) => lines.len(),
                (_, CallState::Running(_)) => RUNNING_TAIL,
                _ => FAILURE_TAIL,
            };
            let muted = look.theme.style(Token::Muted);
            lines[lines.len().saturating_sub(keep)..]
                .iter()
                .map(|line| {
                    (
                        Line::from(Span::plain("  ")).with(Span::new(line.clone(), muted)),
                        Style::PLAIN,
                    )
                })
                .collect()
        }
    }
}

/// A diff's rows: the line number and the sign in a gutter beside the code,
/// added and removed lines tinted to the box's edge.
pub(crate) fn diff_rows(lines: &[DiffLine], room: usize, look: &Look) -> Vec<(Line, Style)> {
    diff_rows_cut(lines, room, look).0
}

/// [`diff_rows`], and whether any row lost its end to `room`.
pub(crate) fn diff_rows_cut(
    lines: &[DiffLine],
    room: usize,
    look: &Look,
) -> (Vec<(Line, Style)>, bool) {
    let mut cut_any = false;
    let theme = &look.theme;
    let digits = lines
        .iter()
        .map(|line| line.number.to_string().len())
        .max()
        .unwrap_or(1)
        .max(4);
    let mut rows = Vec::with_capacity(lines.len());
    for line in lines {
        let text = expand_tabs(&sanitize(&line.text), 4).into_owned();
        let number = format!("{:>digits$}", line.number);
        let (row, fill) = match line.change {
            Change::Same => {
                let context = theme.style(Token::DiffContext);
                let row = Line::from(Span::plain("  "))
                    .with(Span::new(number, context))
                    .with(Span::plain("   "))
                    .with(Span::new(text, context));
                (row, Style::PLAIN)
            }
            Change::Added | Change::Removed => {
                let added = line.change == Change::Added;
                let (number_style, sign_style, body) = if added {
                    (Token::DiffAddNumber, Token::DiffAddSign, Token::DiffAddLine)
                } else {
                    (Token::DiffDelNumber, Token::DiffDelSign, Token::DiffDelLine)
                };
                let body = theme.style(body);
                let sign = if added { "+" } else { look.glyphs.minus };
                let mut row = Line::from(Span::plain("  "))
                    .with(Span::new(format!("{number} "), theme.style(number_style)))
                    .with(Span::new(format!("{sign} "), theme.style(sign_style)));
                for span in emphasised(
                    &line.text,
                    &text,
                    &line.emphasis,
                    body,
                    theme.style(Token::DiffEmph),
                ) {
                    row.push(span);
                }
                (row, body)
            }
        };
        cut_any |= row.width() > room;
        rows.push((cut(&row, room, look.glyphs.ellipsis), fill));
    }
    (rows, cut_any)
}

/// `shown` (the line's text, sanitised and with its tabs expanded) with the
/// changed bytes of the original `text` in bold. Where sanitising or tabs
/// moved the bytes, the emphasis is left out rather than put in the wrong
/// place.
fn emphasised(
    text: &str,
    shown: &str,
    emphasis: &[std::ops::Range<usize>],
    body: Style,
    emph: Style,
) -> Vec<Span> {
    let bold = Style {
        bold: body.bold || emph.bold,
        ..body
    };
    if text != shown || emphasis.is_empty() {
        return vec![Span::new(shown, body)];
    }
    let mut spans = Vec::new();
    let mut at = 0;
    for range in emphasis {
        let (start, end) = (range.start.min(text.len()), range.end.min(text.len()));
        if start < at || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        spans.push(Span::new(&text[at..start], body));
        spans.push(Span::new(&text[start..end], bold));
        at = end;
    }
    spans.push(Span::new(&text[at..], body));
    spans
}

/// A command's output as lines to show: sanitised, a carriage return taking
/// the line back to its start as a terminal would, tabs expanded, and a last
/// empty line left out.
pub(crate) fn output_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let line = line.rsplit('\r').next().unwrap_or(line);
            expand_tabs(&sanitize(line), 8).into_owned()
        })
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// A text shown whole (a reasoning, a compaction's summary) as lines to
/// show: sanitised, tabs expanded, a last empty line left out, and a
/// carriage return shown as its symbol rather than taking the line back as
/// a command's output does, so that nothing of it is hidden (since the
/// review of step 25's second pull request). The `\r` of a `\r\n` is the
/// line break's.
pub(crate) fn whole_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            expand_tabs(&sanitize(line), 8).into_owned()
        })
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// A path's file name: what follows its last `/` or `\`.
fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if name.is_empty() { path } else { name }
}

#[cfg(test)]
mod tests;
