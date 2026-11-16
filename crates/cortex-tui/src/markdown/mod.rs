//! Markdown, streamed: what a model writes, laid out as it arrives, with
//! each line committed the moment it can no longer change
//! (`docs/architecture/09-tui.md`, "Rendering approach" and "The commit
//! rule").
//!
//! [`Markdown`] takes text as it streams ([`Markdown::push`]) and gives back
//! the lines that are final ([`Markdown::commit`]), and the rows still live
//! ([`Markdown::live`]).
//!
//! # The commit rule
//!
//! - **Blocks are decided by whole lines.** A line is read when it ends, and
//!   it says whether the block before it goes on: a blank line, or a line
//!   that starts another block, ends it. A finished block's lines are
//!   committed, and its text is dropped: it is never parsed again.
//! - **A code line** is committed when it ends. **A heading or a rule** when
//!   its line ends. **A table** when it ends, or in parts of
//!   [`TABLE_PART`] rows with the widths known so far.
//! - **A paragraph's row** is committed when the next row has begun and
//!   nothing can change before the end of that row's first word (or of the
//!   row, if the word goes on past it): no mark that text still to come
//!   could close (emphasis, a backtick run, a bracket), and no construct
//!   with such a mark inside it. A row with a mark still open is held for
//!   two more rows; when a third has begun, it is committed as it is, and
//!   its open marks, with those inside a construct it holds, stay literal
//!   from then on.
//! - **Rows are decided at the end of each word,** from the text up to it,
//!   and inside a run of text with no white space every 64 bytes,
//!   where the byte after the cut cannot change what ends there. The
//!   decisions therefore depend on the text alone and not on how it was
//!   split into pieces: the same text gives the same lines, token by token
//!   or all at once.
//! - **Text is sanitised as it is read,** a line whole or, given to a
//!   paragraph before its line ends, in pieces, each as it reads after the
//!   last character of the piece before (`cortex_term::sanitize::sanitize_after`):
//!   an emoji's variation selector that arrives in the next piece is kept,
//!   as in the whole line, and is not drawn as `\u{fe0f}` (since the review
//!   of step 23).
//!
//! Inline markup is parsed for the paragraph in progress only, from the
//! first row not yet committed: the text of committed rows is dropped at a
//! break no construct spans, after which the text parses alone as it did.
//! Where none comes, the text kept is bounded (1 KB, then 4 KB): a break
//! is forced, and each byte is parsed a bounded number of times.

mod block;
pub(crate) mod inline;
mod rows;
mod table;

use std::collections::VecDeque;
use std::ops::Range;
use std::time::Duration;

use cortex_base::text::expand_tabs;
use cortex_term::sanitize::sanitize_after;
use cortex_term::{Line, Span, Style, sanitize};

use crate::highlight::{BlockId, Highlighter, Outcome, WAIT};
use crate::layout::cut;
use crate::theme::Token;
use crate::view::{Density, Look};

use self::block::{classify, closes_fence, dedent, delimiter_row, split_row, surely_text};
use self::rows::Laid;
use self::table::Table;

/// The most table rows held in the live region; a longer table is
/// committed in parts of this many rows, with the column widths known when
/// the first part is.
pub const TABLE_PART: usize = 16;

/// The committed text a paragraph keeps for parsing while it waits for a
/// clean break before one is forced where a row starts outside every
/// construct; past it, decisions are also spaced [`LONG_RUN`] bytes apart
/// (09-tui, "The commit rule").
const KEPT: usize = 1024;

/// The committed text past which a break is forced where any row starts.
const LIMIT: usize = 4096;

/// The most of the line still arriving that the live region draws.
const LIVE_LINE: usize = 16 * 1024;

/// A run of text with no white space gets a decision every this many bytes,
/// so that a very long word (a hash, a blob) does not keep its rows live.
const LONG_RUN: usize = 64;

/// What a stream of markdown is, which sets how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// An answer: `◇` before its first row, text at full brightness.
    Answer,
    /// A reasoning summary: no mark, every row muted and italic.
    Reasoning,
}

/// A markdown stream. See the [module documentation](self).
#[derive(Debug, Clone)]
pub struct Markdown {
    flavor: Flavor,
    /// Whether a paragraph's rows commit before it ends. Always, except in
    /// the tests that compare a stream with the whole document.
    early: bool,
    /// Text pushed and not yet read: the end of the last complete line read
    /// is its start.
    pending: String,
    /// Bytes at the start of `pending` known to hold no line break.
    scanned: usize,
    /// Bytes of the line at the start of `pending` already given to the
    /// open paragraph, when that line was certainly its text.
    taken: Option<usize>,
    /// The last character of the line at the start of `pending` given to
    /// the open paragraph and dropped: the rest of the line is sanitised as
    /// it reads after it, as the whole line would be.
    before: Option<char>,
    finished: bool,
    containers: Vec<Container>,
    leaf: Option<Leaf>,
    /// A line starting with `|`, waiting for the next to say whether it is a
    /// table's header.
    candidate: Option<Candidate>,
    /// Whether a row has been committed, and so whether the next leaf is
    /// set apart by a blank row.
    started: bool,
    /// Whether a leaf has opened since the last row: its first row is set
    /// apart from what came before.
    fresh: bool,
    blank_seen: bool,
    last_in_list: bool,
    out: Vec<Line>,
    /// Rows that are final but wait behind a code line still waiting for its
    /// colours, so that rows commit in order.
    queue: VecDeque<Pending>,
    highlighter: Highlighter,
    /// The caller's clock, which bounds how long a code line waits.
    now: Duration,
    /// Code blocks whose lines are still waiting: how many, and whether the
    /// block has ended (the worker is told when both are done).
    blocks: Vec<(BlockId, usize, bool)>,
    /// A copy drawn for the live region: it reads the highlighter's results
    /// without taking them, and gives it no work.
    scratch: bool,
    /// How many times the commit rule forced something: a row committed
    /// while a mark in it was still open, or kept text cut where no break
    /// was clean.
    forced: usize,
    /// Bytes given to the inline parser, for the test that nothing finished
    /// is parsed again.
    parsed: usize,
    /// The text kept before a break is forced ([`KEPT`] and [`LIMIT`]): the
    /// tests of the bounds give smaller ones, to test them on less text.
    bounds: (usize, usize),
}

#[derive(Debug, Clone)]
enum Container {
    Quote,
    Item {
        /// Columns the item's content is indented by.
        content: usize,
        /// The marker as shown: the bullet, or the number as written.
        marker: String,
        /// Whether a row of the item has been committed, which shows the
        /// marker.
        shown: bool,
    },
}

#[derive(Debug, Clone)]
enum Leaf {
    Paragraph(Paragraph),
    Code(Code),
    Table(Table),
}

#[derive(Debug, Clone)]
struct Code {
    /// The fence's character, length and indentation, for a fenced block.
    fence: Option<(u8, usize, usize)>,
    /// The block, as the highlighter knows it, and its next line's number.
    block: Option<BlockId>,
    next: usize,
}

/// A row ready to commit, or a code line waiting for its colours.
#[derive(Debug, Clone)]
enum Pending {
    /// A row: the marks before it, and its content.
    Row { prefix: Line, content: Line },
    Code {
        /// The marks before it, already decided.
        prefix: Line,
        text: String,
        room: usize,
        width: usize,
        block: BlockId,
        index: usize,
        /// When it ended.
        since: Duration,
    },
}

#[derive(Debug, Clone)]
struct Candidate {
    line: String,
    /// The containers matched by the line, so that a lazy line is put back.
    matched: usize,
}

/// The paragraph in progress: the text of its rows not yet dropped.
#[derive(Debug, Clone, Default)]
struct Paragraph {
    src: String,
    /// Bytes of `src` taken as text whatever they would open.
    frozen: Vec<usize>,
    /// Rows of `src` already committed.
    committed: usize,
    /// Bytes of `src` whose word ends have been looked at.
    evaluated: usize,
    /// Where in `src` the last decision was made.
    last: usize,
    /// Where a decision inside a run of text with no white space becomes
    /// due: [`LONG_RUN`] bytes into the run, or after the last such one.
    due: usize,
    /// The width its rows are laid out at.
    width: usize,
    /// Rows committed at another width, kept as they were drawn, and where
    /// in the shown text the rows after them start (09-tui, "The commit
    /// rule"): until their text is dropped.
    fixed: Option<(Vec<Range<usize>>, usize)>,
}

impl Paragraph {
    /// Drops the text before `split`, which committed rows hold.
    fn drop_front(&mut self, split: usize) {
        self.src.drain(..split);
        self.frozen.retain(|&at| at >= split);
        for at in &mut self.frozen {
            *at -= split;
        }
        self.committed = 0;
        self.fixed = None;
    }

    /// `pieces` of its text laid out at `width`, its rows committed at
    /// another width kept as they were.
    fn lay_out(&self, pieces: Vec<inline::Piece>, width: usize, frame: Frame<'_>) -> Laid {
        let mut laid = Laid::new(pieces, width, frame);
        if let Some((rows, resume)) = &self.fixed {
            laid.refit(rows, *resume, width);
        }
        laid
    }
}

/// The size the stream is laid out at, and how it looks.
#[derive(Debug, Clone, Copy)]
struct Frame<'a> {
    width: usize,
    look: &'a Look,
}

impl Frame<'_> {
    fn prose(&self) -> usize {
        self.look.prose_width.min(self.width)
    }
}

impl Markdown {
    /// An answer, empty.
    #[must_use]
    pub fn answer() -> Self {
        Self::new(Flavor::Answer)
    }

    /// A reasoning summary, empty.
    #[must_use]
    pub fn reasoning() -> Self {
        Self::new(Flavor::Reasoning)
    }

    /// An empty stream of `flavor`.
    #[must_use]
    pub fn new(flavor: Flavor) -> Self {
        Self {
            flavor,
            early: true,
            pending: String::new(),
            scanned: 0,
            taken: None,
            before: None,
            finished: false,
            containers: Vec::new(),
            leaf: None,
            candidate: None,
            started: false,
            fresh: true,
            blank_seen: false,
            last_in_list: false,
            out: Vec::new(),
            queue: VecDeque::new(),
            highlighter: Highlighter::off(),
            now: Duration::ZERO,
            blocks: Vec::new(),
            scratch: false,
            forced: 0,
            parsed: 0,
            bounds: (KEPT, LIMIT),
        }
    }

    /// The whole of `text`, drawn at `width` with `look`: what a stream of it
    /// commits.
    #[must_use]
    pub fn render(flavor: Flavor, text: &str, width: usize, look: &Look) -> Vec<Line> {
        let mut stream = Self::new(flavor);
        stream.push(text);
        stream.finish();
        stream.commit(width, look)
    }

    /// Adds text as it arrived. It is sanitised as it is read.
    pub fn push(&mut self, text: &str) {
        if !self.finished {
            self.pending.push_str(text);
        }
    }

    /// The stream has ended: everything still open is finished, and an
    /// unclosed fence ends with the text.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Whether the stream has ended.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The lines that became final since the last call, laid out at `width`
    /// (the columns a line may use) with `look`.
    pub fn commit(&mut self, width: usize, look: &Look) -> Vec<Line> {
        let frame = Frame {
            width: width.max(1),
            look,
        };
        self.read(frame);
        self.flush(frame);
        std::mem::take(&mut self.out)
    }

    /// The rows not yet committed, as they look now: the lines a commit would
    /// give if the stream ended here. Call [`commit`](Self::commit) first.
    #[must_use]
    pub fn live(&self, width: usize, look: &Look) -> Vec<Line> {
        // The line still arriving is copied up to 16 KB: no more of a very
        // long one can be on screen, and a copy of all of it each frame would
        // cost a frame its length.
        let mut cut = self.pending.len().min(LIVE_LINE);
        while !self.pending.is_char_boundary(cut) {
            cut -= 1;
        }
        let mut scratch = Self {
            flavor: self.flavor,
            early: self.early,
            pending: self.pending[..cut].to_owned(),
            scanned: 0,
            taken: self.taken.map(|taken| taken.min(cut)),
            before: self.before,
            finished: true,
            containers: self.containers.clone(),
            leaf: self.leaf.clone(),
            candidate: self.candidate.clone(),
            started: self.started,
            fresh: self.fresh,
            blank_seen: self.blank_seen,
            last_in_list: self.last_in_list,
            out: self.out.clone(),
            queue: self.queue.clone(),
            highlighter: self.highlighter.clone(),
            now: self.now,
            blocks: self.blocks.clone(),
            scratch: true,
            forced: self.forced,
            parsed: self.parsed,
            bounds: self.bounds,
        };
        scratch.commit(width, look)
    }

    /// Colours code blocks with `highlighter` (an answer's only).
    #[must_use]
    pub fn with_highlighter(mut self, highlighter: Highlighter) -> Self {
        self.highlighter = highlighter;
        self
    }

    /// The time on the caller's clock, which bounds how long a code line
    /// waits for its colours ([`WAIT`]).
    pub fn set_now(&mut self, now: Duration) {
        self.now = now;
    }

    /// Whether everything has been committed: the stream has ended and no
    /// line is waiting.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.finished && self.queue.is_empty() && self.out.is_empty()
    }

    /// How many times the commit rule forced something: a row committed
    /// while a mark in it was still open, or a paragraph's kept text cut
    /// where no break was clean. While it is 0, the lines are those of the
    /// text laid out whole.
    #[must_use]
    pub fn forced(&self) -> usize {
        self.forced
    }

    #[cfg(test)]
    pub(crate) fn whole(mut self) -> Self {
        self.early = false;
        self
    }

    /// The stream with other bounds on the text kept than [`KEPT`] and
    /// [`LIMIT`].
    #[cfg(test)]
    pub(crate) fn with_bounds(mut self, kept: usize, limit: usize) -> Self {
        self.bounds = (kept, limit);
        self
    }

    #[cfg(test)]
    pub(crate) fn parsed_bytes(&self) -> usize {
        self.parsed
    }

    /// The text kept for parsing: the paragraph in progress's, and the line
    /// still arriving. What a frame's live rows parse.
    #[cfg(test)]
    pub(crate) fn kept_bytes(&self) -> usize {
        let paragraph = match &self.leaf {
            Some(Leaf::Paragraph(paragraph)) => paragraph.src.len(),
            _ => 0,
        };
        paragraph + self.pending.len()
    }

    // ---- reading lines

    fn read(&mut self, frame: Frame<'_>) {
        // Each line is read where it is, and the text read is dropped once:
        // draining the buffer line by line would move the rest of a large
        // document once a line.
        let pending = std::mem::take(&mut self.pending);
        let mut start = 0;
        let mut search = self.scanned.min(pending.len());
        while let Some(found) = pending[search..].find('\n') {
            let end = search + found;
            let raw = &pending[start..end];
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            match self.taken.take() {
                Some(taken) => {
                    let rest = self.rest_of_line(raw, taken);
                    self.paragraph_text(&rest, frame);
                }
                None => {
                    let line = sanitize(raw).into_owned();
                    self.line(&line, frame);
                }
            }
            start = end + 1;
            search = start;
        }
        self.pending = if start == 0 {
            pending
        } else {
            pending[start..].to_owned()
        };
        self.scanned = self.pending.len();
        if self.finished {
            if !self.pending.is_empty() {
                let raw = std::mem::take(&mut self.pending);
                let raw = raw.strip_suffix('\r').unwrap_or(&raw);
                match self.taken.take() {
                    Some(taken) => {
                        let rest = self.rest_of_line(raw, taken);
                        self.paragraph_text(&rest, frame);
                    }
                    None => {
                        let line = sanitize(raw).into_owned();
                        self.line(&line, frame);
                    }
                }
            }
            self.close_candidate(frame);
            self.close_leaf(frame);
            self.containers.clear();
            return;
        }
        self.partial(frame);
    }

    /// The rest of a line whose first `taken` bytes the open paragraph has,
    /// sanitised as it reads after them: an emoji's selector that came after
    /// its emoji is kept, as in the whole line. The line is over.
    fn rest_of_line(&mut self, raw: &str, taken: usize) -> String {
        let taken = taken.min(raw.len());
        let before = raw[..taken].chars().next_back().or(self.before);
        self.before = None;
        sanitize_after(before, &raw[taken..]).into_owned()
    }

    /// Gives the open paragraph what has arrived of a line that is
    /// certainly its text, so that its words are decided as they end.
    fn partial(&mut self, frame: Frame<'_>) {
        // A carriage return may be the first half of a line break.
        let usable = self
            .pending
            .strip_suffix('\r')
            .unwrap_or(&self.pending)
            .len();
        match self.taken {
            Some(taken) => {
                if usable > taken {
                    let before = self.pending[..taken].chars().next_back().or(self.before);
                    let text = sanitize_after(before, &self.pending[taken..usable]).into_owned();
                    self.before = self.pending[..usable].chars().next_back();
                    self.taken = Some(usable);
                    self.paragraph_text(&text, frame);
                }
            }
            None => {
                // A line that may be a table's delimiter row waits for its end.
                if self.candidate.is_some() {
                    return;
                }
                let partial = &self.pending[..usable];
                let (matched, rest) = self.match_containers(partial);
                let all = matched == self.containers.len();
                // Inside a fenced code block a line is code, not text.
                if all && matches!(self.leaf, Some(Leaf::Code(Code { fence: Some(_), .. }))) {
                    return;
                }
                if !surely_text(rest) {
                    return;
                }
                let start = partial.len() - rest.trim_start_matches([' ', '\t']).len();
                // The line's start is sanitised with it when it ends whole:
                // its text reads after the marks before it.
                let before = self.pending[..start].chars().next_back();
                let text = sanitize_after(before, &self.pending[start..usable]).into_owned();
                self.before = self.pending[..usable].chars().next_back();
                self.taken = Some(usable);
                // What `line` does with a line of text, decided from its start.
                let paragraph = matches!(self.leaf, Some(Leaf::Paragraph(_)));
                if !all && !paragraph {
                    self.close_leaf(frame);
                    self.containers.truncate(matched);
                }
                self.paragraph_line(&text, frame);
            }
        }
        // What was given to the paragraph is not read again: only the rest of
        // the line is kept, so a long line costs a frame only its new bytes.
        if let Some(taken) = self.taken.filter(|&taken| taken > 0) {
            self.pending.drain(..taken);
            self.scanned = self.scanned.saturating_sub(taken);
            self.taken = Some(0);
        }
    }

    /// How many open containers `line` continues, and the rest of it.
    fn match_containers<'l>(&self, line: &'l str) -> (usize, &'l str) {
        let mut rest = line;
        let mut matched = 0;
        for container in &self.containers {
            match container {
                Container::Quote => match classify(rest) {
                    block::Line::Quote { rest: skip } => rest = &rest[skip..],
                    _ => break,
                },
                Container::Item { content, .. } => {
                    if rest.trim().is_empty() {
                        rest = "";
                    } else if let Some(inner) = dedent(rest, *content) {
                        rest = inner;
                    } else {
                        break;
                    }
                }
            }
            matched += 1;
        }
        (matched, rest)
    }

    /// One whole line, sanitised and without its line break.
    fn line(&mut self, line: &str, frame: Frame<'_>) {
        let (matched, rest) = self.match_containers(line);
        let all = matched == self.containers.len();

        // Inside a fenced code block, a line is code until the fence closes.
        if let Some(Leaf::Code(Code {
            fence: Some((ch, len, indent)),
            ..
        })) = &self.leaf
            && all
        {
            let (ch, len, indent) = (*ch, *len, *indent);
            if closes_fence(rest, ch, len) {
                self.close_leaf(frame);
            } else {
                let code = dedent(rest, indent).unwrap_or_else(|| rest.trim_start_matches(' '));
                self.code_line(code, frame);
            }
            return;
        }

        // A header candidate: this line says whether it is a table.
        if let Some(candidate) = self.candidate.take() {
            let header = split_row(&candidate.line).len();
            match delimiter_row(rest).filter(|aligns| all && aligns.len() == header) {
                Some(aligns) => {
                    self.close_leaf(frame);
                    self.fresh = true;
                    self.leaf = Some(Leaf::Table(Table::new(&candidate.line, aligns)));
                    return;
                }
                None => {
                    self.text_line(&candidate.line, candidate.matched, frame);
                }
            }
        }

        let kind = classify(rest);

        // A lazy line: the open paragraph goes on, its containers with it.
        if !all
            && matches!(self.leaf, Some(Leaf::Paragraph(_)))
            && matches!(
                kind,
                block::Line::Text | block::Line::Indented | block::Line::Pipe
            )
        {
            self.paragraph_line(rest.trim_start_matches([' ', '\t']), frame);
            return;
        }
        if !all {
            self.close_leaf(frame);
            self.containers.truncate(matched);
        }
        self.leaf_line(rest, frame);
    }

    /// A line after its containers: new containers it opens, then its leaf.
    fn leaf_line(&mut self, mut rest: &str, frame: Frame<'_>) {
        loop {
            match classify(rest) {
                block::Line::Quote { rest: skip } => {
                    self.close_leaf(frame);
                    self.containers.push(Container::Quote);
                    rest = &rest[skip..];
                }
                block::Line::Item {
                    marker,
                    ordered,
                    content,
                } => {
                    self.close_leaf(frame);
                    let shown = if ordered {
                        marker
                    } else {
                        frame.look.glyphs.bullet.to_owned()
                    };
                    self.containers.push(Container::Item {
                        content,
                        marker: shown,
                        shown: false,
                    });
                    rest = rest.get(content..).unwrap_or("");
                }
                _ => break,
            }
        }
        match classify(rest) {
            block::Line::Blank => {
                if matches!(self.leaf, Some(Leaf::Paragraph(_) | Leaf::Table(_)))
                    || matches!(self.leaf, Some(Leaf::Code(Code { fence: None, .. })))
                {
                    self.close_leaf(frame);
                }
                if self.started {
                    self.blank_seen = true;
                }
            }
            block::Line::Fence { ch, len, info } => {
                self.close_leaf(frame);
                self.fresh = true;
                let indent = block::indentation(rest);
                let block = if self.flavor == Flavor::Answer && !self.scratch {
                    self.highlighter.open(&info)
                } else {
                    None
                };
                if let Some(block) = block {
                    self.blocks.push((block, 0, false));
                }
                self.leaf = Some(Leaf::Code(Code {
                    fence: Some((ch, len, indent)),
                    block,
                    next: 0,
                }));
            }
            block::Line::Heading { text, .. } => {
                self.close_leaf(frame);
                self.heading(&text, frame);
            }
            block::Line::Rule => {
                self.close_leaf(frame);
                self.rule(frame);
            }
            block::Line::Pipe => {
                if let Some(Leaf::Table(table)) = &mut self.leaf {
                    table.row(rest);
                    if table.waiting() >= TABLE_PART {
                        self.table_part(frame);
                    }
                } else {
                    self.candidate = Some(Candidate {
                        line: rest.to_owned(),
                        matched: self.containers.len(),
                    });
                }
            }
            block::Line::Indented => {
                if matches!(self.leaf, Some(Leaf::Paragraph(_))) {
                    self.paragraph_line(rest.trim_start_matches([' ', '\t']), frame);
                } else {
                    if !matches!(self.leaf, Some(Leaf::Code(Code { fence: None, .. }))) {
                        self.close_leaf(frame);
                        self.fresh = true;
                        self.leaf = Some(Leaf::Code(Code {
                            fence: None,
                            block: None,
                            next: 0,
                        }));
                    }
                    let code = dedent(rest, 4).unwrap_or(rest);
                    self.code_line(code, frame);
                }
            }
            block::Line::Text => {
                if !matches!(self.leaf, Some(Leaf::Paragraph(_))) {
                    self.close_leaf(frame);
                }
                self.paragraph_line(rest.trim_start_matches([' ', '\t']), frame);
            }
            block::Line::Quote { .. } | block::Line::Item { .. } => {}
        }
    }

    /// A header candidate that was not one: text, with the containers it was
    /// written in.
    fn text_line(&mut self, line: &str, matched: usize, frame: Frame<'_>) {
        if matched < self.containers.len() && !matches!(self.leaf, Some(Leaf::Paragraph(_))) {
            self.close_leaf(frame);
            self.containers.truncate(matched);
        }
        if !matches!(self.leaf, Some(Leaf::Paragraph(_))) {
            self.close_leaf(frame);
        }
        self.paragraph_line(line.trim_start_matches([' ', '\t']), frame);
    }

    fn close_candidate(&mut self, frame: Frame<'_>) {
        if let Some(candidate) = self.candidate.take() {
            self.text_line(&candidate.line, candidate.matched, frame);
        }
    }

    // ---- leaves

    /// Finishes the open leaf: its rows are committed.
    fn close_leaf(&mut self, frame: Frame<'_>) {
        match self.leaf.take() {
            Some(Leaf::Paragraph(mut paragraph)) => {
                self.evaluate(&mut paragraph, frame);
                // White space at a paragraph's end is not shown.
                let end = paragraph.src.trim_end().len();
                paragraph.src.truncate(end);
                let parsed = inline::parse(&paragraph.src, &paragraph.frozen);
                self.parsed += paragraph.src.len();
                let laid = paragraph.lay_out(parsed.pieces, self.content_width(frame), frame);
                for row in paragraph.committed..laid.rows.len() {
                    let line = laid.line(row, frame, self.flavor);
                    self.emit(line, frame);
                }
            }
            Some(Leaf::Table(mut table)) => {
                let rows = table.finish(frame.width, self.prefix_width(), frame.look);
                for row in rows {
                    self.emit(row, frame);
                }
            }
            Some(Leaf::Code(Code {
                block: Some(block), ..
            })) => {
                if let Some(entry) = self.blocks.iter_mut().find(|(id, _, _)| *id == block) {
                    entry.2 = true;
                }
                self.release_blocks();
            }
            Some(Leaf::Code(_)) | None => {}
        }
    }

    fn table_part(&mut self, frame: Frame<'_>) {
        let width = self.prefix_width();
        if let Some(Leaf::Table(table)) = &mut self.leaf {
            let rows = table.part(frame.width, width, frame.look);
            for row in rows {
                self.emit(row, frame);
            }
        }
    }

    /// A new line of the paragraph (opening one if none is open).
    fn paragraph_line(&mut self, text: &str, frame: Frame<'_>) {
        let mut paragraph = match self.leaf.take() {
            Some(Leaf::Paragraph(paragraph)) => paragraph,
            other => {
                self.leaf = other;
                self.close_leaf(frame);
                self.fresh = true;
                Paragraph {
                    due: LONG_RUN,
                    ..Paragraph::default()
                }
            }
        };
        if !paragraph.src.is_empty() || paragraph.committed > 0 || paragraph.evaluated > 0 {
            paragraph.src.push('\n');
        }
        paragraph.src.push_str(text);
        self.evaluate(&mut paragraph, frame);
        self.leaf = Some(Leaf::Paragraph(paragraph));
    }

    /// More of the paragraph's current line.
    fn paragraph_text(&mut self, text: &str, frame: Frame<'_>) {
        if let Some(Leaf::Paragraph(mut paragraph)) = self.leaf.take() {
            paragraph.src.push_str(text);
            self.evaluate(&mut paragraph, frame);
            self.leaf = Some(Leaf::Paragraph(paragraph));
        } else {
            self.paragraph_line(text, frame);
        }
    }

    /// Looks at each word end not yet looked at, in order, and commits the
    /// rows that became final there.
    fn evaluate(&mut self, paragraph: &mut Paragraph, frame: Frame<'_>) {
        if !self.early {
            paragraph.evaluated = paragraph.src.len();
            return;
        }
        self.rewidth(paragraph, frame);
        let white = |byte: u8| matches!(byte, b' ' | b'\n' | b'\t');
        // Every byte is looked at once, in order, and what is kept between
        // calls is in `paragraph`: the decisions are the same however the
        // text arrived.
        let mut at = paragraph.evaluated.max(1);
        while at < paragraph.src.len() {
            let bytes = paragraph.src.as_bytes();
            let space = white(bytes[at]);
            let decide = if space {
                paragraph.due = at + 1 + LONG_RUN;
                !white(bytes[at - 1])
            } else {
                at >= paragraph.due && settled_inside_word(&paragraph.src, at, &paragraph.frozen)
            };
            // With much text kept, each decision parses much: they are spaced.
            let decide = decide && (at <= self.bounds.0 || at >= paragraph.last + LONG_RUN);
            if decide {
                if !space {
                    paragraph.due = at + LONG_RUN;
                }
                let dropped = self.decide(paragraph, at, frame);
                at -= dropped;
                paragraph.due -= dropped;
                paragraph.last = at;
            }
            at += 1;
        }
        paragraph.evaluated = paragraph.src.len();
    }

    /// The decisions at `end` of the paragraph's text: a word's end, or a
    /// place inside a long run of text. Returns how many bytes of the text
    /// were dropped.
    fn decide(&mut self, paragraph: &mut Paragraph, end: usize, frame: Frame<'_>) -> usize {
        // One parse serves every row committed here. Freezing an open mark
        // changes nothing it shows (an open mark is shown as text, and no
        // closer took it), and after a clean cut the text parses alone as it
        // parsed here, so its rows are this layout's from the cut on.
        let continues = paragraph
            .src
            .as_bytes()
            .get(end)
            .is_some_and(|&byte| !matches!(byte, b' ' | b'\n' | b'\t'));
        let mut parsed = inline::parse_cut(&paragraph.src[..end], &paragraph.frozen, continues);
        self.parsed += end;
        let laid = paragraph.lay_out(
            std::mem::take(&mut parsed.pieces),
            self.content_width(frame),
            frame,
        );
        // Where the text is cut: its source byte, and the rows before it.
        let mut cut = (0, 0);
        loop {
            let next = paragraph.committed;
            if laid.rows.len() < next + 2 {
                break;
            }
            let first_word_end = laid.first_word_end(next + 1);
            let held = parsed
                .unstable_with(&paragraph.frozen)
                .is_some_and(|at| at < first_word_end);
            if held {
                if laid.rows.len() < next + 4 {
                    break;
                }
                // Held for two more rows: committed as it is, and its marks
                // stay literal, with those inside a construct it holds.
                paragraph.frozen.extend(parsed.settling(first_word_end));
                paragraph.frozen.sort_unstable();
                paragraph.frozen.dedup();
                self.forced += 1;
            }
            let line = laid.line(next, frame, self.flavor);
            self.emit(line, frame);
            paragraph.committed += 1;
            // Drop the committed rows' text where nothing spans the break.
            // Nothing before it is open: a row commits only when it is
            // settled, or with its marks frozen.
            let row = paragraph.committed;
            let settled = |split: usize| {
                parsed
                    .opens
                    .iter()
                    .all(|&open| open >= split || paragraph.frozen.binary_search(&open).is_ok())
            };
            if let Some(split) = laid.clean_break(row).filter(|&split| settled(split)) {
                cut = (split, row);
                continue;
            }
            // A paragraph with no clean break for long (a long word of marks
            // or entities, or text whose rows all start inside a construct)
            // keeps a bounded amount of committed text: past `KEPT` bytes it
            // is cut where a row starts outside every construct, and past
            // `LIMIT` where any row starts. The text after a forced cut is
            // parsed alone, which may differ from this parse: nothing more is
            // committed from this one.
            let kept = |split: usize| split.saturating_sub(cut.0);
            let (kept_most, limit) = self.bounds;
            let forced = laid
                .forced_break(row, false)
                .filter(|&split| kept(split) > kept_most && settled(split))
                .or_else(|| {
                    laid.forced_break(row, true)
                        .filter(|&split| kept(split) > limit && settled(split))
                });
            if let Some(split) = forced {
                self.forced += 1;
                cut = (split, row);
                break;
            }
        }
        let (split, rows) = cut;
        if split > 0 {
            let committed = paragraph.committed - rows;
            paragraph.drop_front(split);
            paragraph.committed = committed;
        }
        split
    }

    /// A paragraph some of whose rows were committed at another width (the
    /// window was resized): those rows keep the breaks they were drawn with,
    /// and the text after them is wrapped at this width, so that no text is
    /// lost or shown twice and nothing is parsed differently.
    fn rewidth(&mut self, paragraph: &mut Paragraph, frame: Frame<'_>) {
        let width = self.content_width(frame);
        let old = std::mem::replace(&mut paragraph.width, width);
        if old == 0 || old == width || paragraph.committed == 0 {
            return;
        }
        let parsed = inline::parse(&paragraph.src, &paragraph.frozen);
        self.parsed += paragraph.src.len();
        let laid = paragraph.lay_out(parsed.pieces, old, frame);
        let committed = paragraph.committed.min(laid.rows.len());
        paragraph.fixed = Some((laid.rows[..committed].to_vec(), laid.row_start(committed)));
    }

    fn content_width(&self, frame: Frame<'_>) -> usize {
        frame.prose().saturating_sub(self.prefix_width()).max(1)
    }

    fn lay_out(&mut self, src: &str, frozen: &[usize], frame: Frame<'_>) -> Laid {
        let parsed = inline::parse(src, frozen);
        self.parsed += src.len();
        Laid::new(parsed.pieces, self.content_width(frame), frame)
    }

    fn heading(&mut self, text: &str, frame: Frame<'_>) {
        self.fresh = true;
        let laid = self.lay_out(text, &[], frame);
        for row in 0..laid.rows.len() {
            let line = laid.styled(row, frame, frame.look.theme.style(Token::Heading));
            self.emit(line, frame);
        }
    }

    fn rule(&mut self, frame: Frame<'_>) {
        self.fresh = true;
        let width = self.content_width(frame);
        let line = Line::styled(
            frame.look.glyphs.horizontal.repeat(width),
            frame.look.theme.style(Token::Border),
        );
        self.emit(line, frame);
    }

    fn code_line(&mut self, code: &str, frame: Frame<'_>) {
        let text = expand_tabs(code, 4).into_owned();
        let base = match self.flavor {
            Flavor::Answer => Style::PLAIN,
            Flavor::Reasoning => frame.look.theme.style(Token::Reasoning),
        };
        let room = frame.width.saturating_sub(self.prefix_width());
        let block = match &mut self.leaf {
            Some(Leaf::Code(code)) if !self.scratch => code.block.map(|block| {
                let index = code.next;
                code.next += 1;
                (block, index)
            }),
            _ => None,
        };
        let Some((block, index)) = block else {
            let line = cut(&Line::styled(text, base), room, frame.look.glyphs.ellipsis);
            self.emit(line, frame);
            return;
        };
        // The worker colours it; the line commits when it has, or when it has
        // waited long enough, plain.
        self.highlighter.line(block, index, &text);
        if let Some(entry) = self.blocks.iter_mut().find(|(id, _, _)| *id == block) {
            entry.1 += 1;
        }
        let prefix = self.start_row(frame);
        self.queue.push_back(Pending::Code {
            prefix,
            text,
            room,
            width: frame.width,
            block,
            index,
            since: self.now,
        });
    }

    /// Commits what is ready at the queue's front, in order.
    fn flush(&mut self, frame: Frame<'_>) {
        while let Some(front) = self.queue.front() {
            let lines = match front {
                Pending::Row { prefix, content } => fitted(prefix, content, frame),
                Pending::Code {
                    prefix,
                    text,
                    room,
                    width,
                    block,
                    index,
                    since,
                } => {
                    let outcome = if self.scratch {
                        self.highlighter.peek(*block, *index)
                    } else {
                        self.highlighter.take(*block, *index)
                    };
                    let colours = match outcome {
                        Outcome::Coloured(colours) => colours,
                        Outcome::Plain => Vec::new(),
                        Outcome::Waiting
                            if self.scratch || self.now.saturating_sub(*since) >= WAIT =>
                        {
                            Vec::new()
                        }
                        Outcome::Waiting => break,
                    };
                    let mut line = prefix.clone();
                    let code = coloured(text, &colours, frame);
                    for span in cut(&code, *room, frame.look.glyphs.ellipsis).spans() {
                        line.push(span.clone());
                    }
                    let (block, width) = (*block, *width);
                    if let Some(entry) = self.blocks.iter_mut().find(|(id, _, _)| *id == block) {
                        entry.1 = entry.1.saturating_sub(1);
                    }
                    // No wider than when it was queued, nor than the window
                    // now, which may have shrunk while it waited.
                    let width = width.min(frame.width);
                    if line.width() > width {
                        line = cut(&line, width, frame.look.glyphs.ellipsis);
                    }
                    vec![line]
                }
            };
            self.queue.pop_front();
            self.out.extend(lines);
        }
        self.release_blocks();
    }

    /// Tells the highlighter of the blocks that have ended and whose lines
    /// have all committed.
    fn release_blocks(&mut self) {
        if self.scratch {
            return;
        }
        let highlighter = &self.highlighter;
        self.blocks.retain(|&(block, waiting, ended)| {
            let done = ended && waiting == 0;
            if done {
                highlighter.close(block);
            }
            !done
        });
    }

    // ---- rows

    /// The columns of the marks before each row: the answer's mark or its
    /// indent, then each container's.
    fn prefix_width(&self) -> usize {
        2 + self
            .containers
            .iter()
            .map(|container| match container {
                Container::Quote => 2,
                Container::Item { marker, .. } => cortex_base::text::width(marker) + 1,
            })
            .sum::<usize>()
    }

    /// Commits a row of the open leaf, with its marks before it, and a blank
    /// row before the leaf's first.
    fn emit(&mut self, content: Line, frame: Frame<'_>) {
        let prefix = self.start_row(frame);
        if self.queue.is_empty() {
            self.out.extend(fitted(&prefix, &content, frame));
        } else {
            self.queue.push_back(Pending::Row { prefix, content });
        }
    }

    /// A finished row: committed, or queued behind a code line still waiting.
    fn deliver(&mut self, line: Line) {
        if self.queue.is_empty() {
            self.out.push(line);
        } else {
            self.queue.push_back(Pending::Row {
                prefix: Line::new(),
                content: line,
            });
        }
    }

    /// The marks before a new row of the open leaf, and the blank row before
    /// the leaf's first, which is delivered at once.
    fn start_row(&mut self, frame: Frame<'_>) -> Line {
        let look = frame.look;
        let in_list = self
            .containers
            .iter()
            .any(|c| matches!(c, Container::Item { .. }));
        if self.started
            && self.fresh
            && look.density == Density::Comfortable
            && (self.blank_seen || !(self.last_in_list && in_list))
        {
            self.deliver(Line::new());
        }
        let mut line = Line::new();
        let outer = if self.started || self.flavor == Flavor::Reasoning {
            Span::plain("  ")
        } else {
            Span::new(
                format!("{} ", look.glyphs.answer),
                look.theme.style(Token::Answer),
            )
        };
        line.push(outer);
        for container in &mut self.containers {
            match container {
                Container::Quote => {
                    line.push(Span::new(
                        format!("{} ", look.glyphs.vertical),
                        look.theme.style(Token::Border),
                    ));
                }
                Container::Item { marker, shown, .. } => {
                    let width = cortex_base::text::width(marker) + 1;
                    if *shown {
                        line.push(Span::plain(" ".repeat(width)));
                    } else {
                        line.push(Span::plain(format!("{marker} ")));
                        *shown = true;
                    }
                }
            }
        }
        self.started = true;
        self.fresh = false;
        self.blank_seen = false;
        self.last_in_list = in_list;
        line
    }
}

/// A code line in its colours, over the plain style.
fn coloured(text: &str, colours: &[(std::ops::Range<usize>, Token)], frame: Frame<'_>) -> Line {
    let mut line = Line::new();
    let mut at = 0;
    for (range, token) in colours {
        let (start, end) = (range.start.min(text.len()), range.end.min(text.len()));
        if start < at || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        line.push(Span::plain(&text[at..start]));
        line.push(Span::new(&text[start..end], frame.look.theme.style(*token)));
        at = end;
    }
    line.push(Span::plain(&text[at..]));
    line
}

#[cfg(test)]
mod tests;

/// Whether a decision may be made at `at`, inside a run of text with no
/// white space: the text before it parses as it will with what follows. The
/// parse of a cut text takes its end as white space, which is right at a
/// word's end, and here wherever the byte after the cut cannot change what
/// ends there: `!` unless `[` follows, `]` unless `(` does, `<` unless a
/// letter does, `&` unless a letter, a digit or `#` does, backslashes in
/// pairs, and a stretch of `*`, `_` and backticks once it is longer than
/// marks can be ([`inline::MARK_RUN`]; one more, in case an escape takes its
/// first). `frozen` is sorted.
fn settled_inside_word(src: &str, at: usize, frozen: &[usize]) -> bool {
    if !src.is_char_boundary(at) {
        return false;
    }
    let before = &src[..at];
    let Some(last) = before.chars().next_back() else {
        return false;
    };
    let next = src[at..].chars().next();
    let is_frozen = |at: usize| frozen.binary_search(&at).is_ok();
    // The run of `c` that ends at the cut, up to a frozen byte when `stop`
    // (a frozen backtick or backslash ends a run, a frozen `*` does not).
    let run = |c: u8, stop: bool| {
        before
            .bytes()
            .enumerate()
            .rev()
            .take_while(|&(index, byte)| byte == c && !(stop && is_frozen(index)))
            .count()
    };
    match last {
        '*' | '_' | '`' => {
            before
                .bytes()
                .rev()
                .take_while(|&byte| inline::is_mark(byte))
                .count()
                > inline::MARK_RUN + 1
        }
        '\\' => {
            !is_frozen(at - 1) && run(b'\\', true) % 2 == 0 && run(b'\\', false) == run(b'\\', true)
        }
        '!' | ']' | '<' | '&' if is_frozen(at - 1) => true,
        '!' => next != Some('['),
        ']' => next != Some('('),
        '<' => !next.is_some_and(|c| c.is_ascii_alphabetic()),
        '&' => !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '#'),
        _ => true,
    }
}

/// A row's marks and content as lines of the window now. A row laid out for
/// the window is one line; a row queued behind a code line while the window
/// shrank is wrapped to it, its later lines indented as far as its marks, so
/// that nothing it holds is dropped. A window too narrow for the marks cuts
/// the row.
fn fitted(prefix: &Line, content: &Line, frame: Frame<'_>) -> Vec<Line> {
    let mut line = prefix.clone();
    for span in content.spans() {
        line.push(span.clone());
    }
    if line.width() <= frame.width {
        return vec![line];
    }
    let indent = prefix.width();
    let room = frame.width.saturating_sub(indent);
    if room < 2 || content.width() <= room {
        return vec![cut(&line, frame.width, "")];
    }
    crate::layout::wrap(content.spans(), room)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let mut line = if index == 0 {
                prefix.clone()
            } else {
                Line::plain(" ".repeat(indent))
            };
            for span in row.spans() {
                line.push(span.clone());
            }
            line
        })
        .collect()
}
