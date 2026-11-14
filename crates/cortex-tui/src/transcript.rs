//! The transcript in progress: blocks in order, each committed when it can
//! no longer change, and the rows still live (`docs/architecture/09-tui.md`,
//! "Inline by default").
//!
//! This is the state the core's events are mapped onto (step 23): a user
//! message, reasoning and an answer as they stream, tool rounds as their
//! calls start and end, notices, errors, the header and the turn summary.
//! Blocks commit in the order they were pushed: a block's lines never go
//! into the scrollback before those of a block above it. A blank row sets
//! each block apart, except one notice from the next; under compact density
//! only a turn is set apart.
//!
//! [`Transcript::commit`] gives the lines that became final since the last
//! call, and [`Transcript::live`] the rows of what is still open, drawn now:
//! the answer's growing tail, the two reasoning rows, the running round's
//! box.
//!
//! [`Transcript::push`] returns the block's [`BlockId`], by which
//! [`Transcript::get_mut`] reaches it while it is open: a round's later
//! results, after a notice or a steering message was pushed below it.

use std::collections::VecDeque;
use std::time::Duration;

use cortex_term::Line;

use crate::markdown::Markdown;
use crate::view::{self, Density, Header, Look, Placement, Round};

/// One block of the transcript.
#[derive(Debug, Clone)]
#[allow(
    clippy::large_enum_variant,
    reason = "the transcript holds only the few blocks still open; boxing an answer would cost an allocation for nothing"
)]
pub enum Block {
    /// The session header, once, at the start.
    Header(Header),
    /// Your message.
    User(String),
    /// Reasoning: streaming until it has finished.
    Reasoning(Reasoning),
    /// An answer, streaming until it is finished.
    Answer(Markdown),
    /// A round of tool calls: live while any call runs.
    Round(Round),
    /// A notice.
    Notice(String),
    /// An error, and what to do next.
    Error {
        /// What went wrong.
        message: String,
        /// What to do next.
        hint: Option<String>,
    },
    /// A turn's summary (shown with `tui.turn_summary`).
    Summary {
        /// How long the turn took.
        took: Duration,
        /// Its tokens.
        tokens: Option<u64>,
        /// Its cost, as shown.
        cost: Option<String>,
    },
}

/// Reasoning, as it streams and when it has finished.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reasoning {
    /// The words so far.
    pub text: String,
    /// How long it has run.
    pub elapsed: Duration,
    /// The provider's summary, if it sent one.
    pub summary: Option<String>,
    /// Whether it has ended (its item completed), with a summary or
    /// without one.
    pub finished: bool,
}

/// A block's place in the transcript, for updating it while it is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(u64);

/// The kinds of block, for the rules on blank rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Header,
    User,
    Notice,
    Other,
}

impl Block {
    fn kind(&self) -> Kind {
        match self {
            Self::Header(_) => Kind::Header,
            Self::User(_) => Kind::User,
            Self::Notice(_) => Kind::Notice,
            _ => Kind::Other,
        }
    }

    /// Whether the block can still change.
    fn is_open(&self) -> bool {
        match self {
            Self::Reasoning(reasoning) => !reasoning.finished,
            Self::Answer(answer) => !answer.is_done(),
            Self::Round(round) => round.is_running(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    id: BlockId,
    block: Block,
    /// Whether any of its lines (or the blank before it) has been
    /// committed.
    started: bool,
}

/// The transcript in progress. See the [module documentation](self).
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    /// Blocks not yet wholly committed, in order.
    queue: VecDeque<Entry>,
    /// The kind of the last block that committed a line.
    last: Option<Kind>,
    /// Whether the last line committed is blank.
    last_blank: bool,
    /// The caller's clock, given to each answer: it bounds how long a code
    /// line waits for its colours.
    now: Duration,
    /// The next block's id.
    next: u64,
}

impl Transcript {
    /// An empty transcript.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a block after the others, and gives its id.
    pub fn push(&mut self, block: Block) -> BlockId {
        let id = BlockId(self.next);
        self.next += 1;
        self.queue.push_back(Entry {
            id,
            block,
            started: false,
        });
        id
    }

    /// The block `id`, to stream into or update, while any of it is still
    /// to commit; `None` once it has all committed, when it can no longer
    /// change on the screen.
    pub fn get_mut(&mut self, id: BlockId) -> Option<&mut Block> {
        self.queue
            .iter_mut()
            .find(|entry| entry.id == id)
            .map(|entry| &mut entry.block)
    }

    /// The last block, to stream into or update: the answer, the reasoning
    /// or the round in progress.
    pub fn last_mut(&mut self) -> Option<&mut Block> {
        self.queue.back_mut().map(|entry| &mut entry.block)
    }

    /// The time on the caller's clock (`Clock::monotonic`), for the answers'
    /// code lines.
    pub fn set_now(&mut self, now: Duration) {
        self.now = now;
    }

    /// Whether nothing is waiting to be committed or drawn live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// The lines that became final since the last call, drawn at `width`.
    pub fn commit(&mut self, width: usize, look: &Look) -> Vec<Line> {
        let mut out = Vec::new();
        while let Some(entry) = self.queue.front_mut() {
            let kind = entry.block.kind();
            if !entry.started {
                // The blank row before a block goes in as soon as the block has
                // begun: what comes after it is certain.
                let gap = gap(self.last, kind, self.last_blank, look);
                if gap {
                    out.push(Line::new());
                    self.last_blank = true;
                }
                entry.started = true;
            }
            let lines = match &mut entry.block {
                Block::Answer(answer) => {
                    answer.set_now(self.now);
                    answer.commit(width, look)
                }
                block if block.is_open() => Vec::new(),
                block => draw(block, Placement::Committed, 0, width, look),
            };
            // Asked after the commit: an answer that has ended may still have
            // lines waiting for their colours.
            let open = entry.block.is_open();
            if !lines.is_empty() {
                self.last = Some(kind);
                self.last_blank = lines.last().is_some_and(|line| line.width() == 0);
                out.extend(lines);
            }
            if open {
                break;
            }
            if kind == Kind::Header {
                // The header card is set apart from whatever follows at once.
                out.push(Line::new());
                self.last_blank = true;
            }
            self.queue.pop_front();
        }
        out
    }

    /// The rows still live, drawn now: `tick` is the spinner's frame. When
    /// there are any, a blank row ends them, which sets them apart from the
    /// input box below (09-tui: "the tail, a blank row, the 3 rows of the
    /// input box, and the hint row"). Call [`commit`](Self::commit) first.
    #[must_use]
    pub fn live(&self, width: usize, look: &Look, tick: usize) -> Vec<Line> {
        let mut rows = Vec::new();
        let mut last = self.last;
        let mut last_blank = self.last_blank;
        for entry in &self.queue {
            let kind = entry.block.kind();
            if !entry.started && gap(last, kind, last_blank, look) {
                rows.push(Line::new());
            }
            let lines = match &entry.block {
                Block::Answer(answer) => answer.live(width, look),
                block => draw(block, Placement::Live, tick, width, look),
            };
            if !lines.is_empty() {
                last = Some(kind);
                last_blank = lines.last().is_some_and(|line| line.width() == 0);
                rows.extend(lines);
            }
        }
        if rows.last().is_some_and(|row| row.width() > 0) {
            rows.push(Line::new());
        }
        rows
    }
}

/// Whether a blank row comes before a block of `kind`, after one of `last`.
fn gap(last: Option<Kind>, kind: Kind, last_blank: bool, look: &Look) -> bool {
    let Some(last) = last else {
        return false;
    };
    if last_blank || (last == Kind::Notice && kind == Kind::Notice) {
        return false;
    }
    look.density == Density::Comfortable || kind == Kind::User
}

/// A finished or live block's lines.
fn draw(block: &Block, placement: Placement, tick: usize, width: usize, look: &Look) -> Vec<Line> {
    match block {
        Block::Header(header) => view::header(header, width, look),
        Block::User(text) => view::user(text, width, look),
        Block::Reasoning(reasoning) if reasoning.finished => view::reasoning(
            reasoning.summary.as_deref().unwrap_or_default(),
            reasoning.elapsed,
            width,
            look,
        ),
        Block::Reasoning(reasoning) => {
            view::reasoning_live(&reasoning.text, reasoning.elapsed, tick, width, look)
        }
        // An answer draws itself through its stream, in `commit` and `live`.
        Block::Answer(_) => Vec::new(),
        Block::Round(round) => view::round(round, placement, tick, width, look),
        Block::Notice(text) => view::notice(text, width, look),
        Block::Error { message, hint } => view::error(message, hint.as_deref(), width, look),
        Block::Summary { took, tokens, cost } => {
            view::turn_summary(*took, *tokens, cost.as_deref(), width, look)
        }
    }
}

#[cfg(test)]
mod tests;
