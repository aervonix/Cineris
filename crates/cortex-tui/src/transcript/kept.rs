//! What is kept of the transcript once it has committed (since step 25):
//! the finished blocks themselves, not their lines, so that the viewer and
//! the full-screen presenter draw them again at any width
//! (`docs/architecture/09-tui.md`, "Rendering cost control": "rendered
//! lines are kept only around the viewport and rebuilt from items on
//! demand").
//!
//! A [`Kept`] is filled from [`Transcript::take_finished`](super::Transcript::take_finished),
//! in the order the blocks committed. Each [`Piece`] is drawn by the same
//! view functions the transcript committed it with, so at the width it
//! committed at it gives the same lines, the blank rows between blocks
//! included. An answer keeps its markdown and its code lines' colours
//! ([`Source`]), not its stream.
//!
//! **What a round keeps is bounded:** a finished command's output is kept to
//! its last [`OUTPUT_KEPT`] bytes, from a line's start, as a running
//! command's is (the core keeps it whole as an artifact).

use std::ops::Range;

use cortex_term::Line;

use super::{Block, draw};
use crate::markdown::{Flavor, Markdown, Source};
use crate::view::round::Detail;
use crate::view::{Look, Placement, Round};

/// The most of a finished command's output a kept round holds, from its
/// end: what a running command keeps (`app::items`), so the viewer shows
/// what it showed before step 25. Reading the whole of a cut output when its
/// box is opened is not built: `docs/outstanding/a-kept-box-holds-only-the-end-of-its-output.md`.
pub const OUTPUT_KEPT: usize = 64 * 1024;

/// A finished block, as kept.
#[derive(Debug, Clone)]
#[allow(
    clippy::large_enum_variant,
    reason = "a kept block is one entry of a list; boxing it would cost an allocation each for nothing"
)]
pub enum Piece {
    /// Any block but an answer, as it committed.
    Block(Block),
    /// An answer: its markdown and its code lines' colours.
    Answer(Source),
}

/// One finished block and its place in the flow.
#[derive(Debug, Clone)]
pub struct KeptBlock {
    /// The block.
    pub piece: Piece,
    /// Whether a blank row came before it.
    pub gap: bool,
    /// Whether a blank row came after it (the session's header card).
    pub after: bool,
}

impl KeptBlock {
    /// Its rows at `width`, a round drawn at `placement`: the blank rows
    /// before and after it included.
    #[must_use]
    pub fn rows(&self, placement: Placement, width: usize, look: &Look) -> Vec<Line> {
        let mut rows = Vec::new();
        if self.gap {
            rows.push(Line::new());
        }
        match &self.piece {
            Piece::Answer(source) => {
                rows.extend(Markdown::render_source(Flavor::Answer, source, width, look));
            }
            Piece::Block(Block::Round(round)) => {
                rows.extend(crate::view::round(round, placement, 0, width, look));
            }
            Piece::Block(block) => rows.extend(draw(block, Placement::Committed, 0, width, look)),
        }
        if self.after {
            rows.push(Line::new());
        }
        rows
    }

    /// The round it holds, if it is one.
    #[must_use]
    pub fn round(&self) -> Option<&Round> {
        match &self.piece {
            Piece::Block(Block::Round(round)) => Some(round),
            _ => None,
        }
    }

    /// Its text as it reads, for a search (since step 25): an answer's
    /// markdown, a round's calls and their whole kept detail, every other
    /// block's words.
    #[must_use]
    pub fn text(&self) -> String {
        match &self.piece {
            Piece::Answer(source) => source.text.clone(),
            Piece::Block(Block::Round(round)) => {
                let mut text = String::new();
                for call in &round.calls {
                    text.push_str(&call.verb);
                    text.push(' ');
                    match &call.target {
                        crate::view::round::Target::Path(target)
                        | crate::view::round::Target::Text(target) => text.push_str(target),
                        crate::view::round::Target::None => {}
                    }
                    text.push('\n');
                    match &call.detail {
                        Detail::None => {}
                        Detail::Output(output) => text.push_str(output),
                        Detail::Diff(lines) => {
                            for line in lines {
                                text.push_str(&line.text);
                                text.push('\n');
                            }
                        }
                    }
                    text.push('\n');
                }
                text
            }
            Piece::Block(Block::User(text) | Block::Notice(text)) => text.clone(),
            Piece::Block(Block::Reasoning(reasoning)) => reasoning
                .summary
                .clone()
                .unwrap_or_else(|| reasoning.text.clone()),
            Piece::Block(Block::Listing { title, lines }) => {
                let mut text = title.clone();
                for line in lines {
                    text.push('\n');
                    text.push_str(line);
                }
                text
            }
            Piece::Block(Block::Error { message, hint }) => {
                format!("{message}\n{}", hint.as_deref().unwrap_or_default())
            }
            Piece::Block(Block::Header(_) | Block::Summary { .. } | Block::Answer(_)) => {
                String::new()
            }
        }
    }

    /// The bytes of text it holds, roughly: what the tests of the bound
    /// count.
    #[must_use]
    pub fn bytes(&self) -> usize {
        match &self.piece {
            Piece::Answer(source) => source.bytes(),
            Piece::Block(block) => block_bytes(block),
        }
    }
}

/// A block finished by the transcript, before it is kept.
#[derive(Debug, Clone)]
pub struct Finished {
    /// The block.
    pub block: Block,
    /// Whether a blank row came before it.
    pub gap: bool,
    /// Whether a blank row came after it.
    pub after: bool,
}

/// The finished transcript. See the [module documentation](self).
#[derive(Debug, Clone, Default)]
pub struct Kept {
    blocks: Vec<KeptBlock>,
    /// Each round's place among the blocks, in order.
    rounds: Vec<usize>,
    bytes: usize,
}

impl Kept {
    /// An empty one.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps what the transcript finished, in order. An answer keeps its
    /// source, which it must have been made to keep
    /// ([`Markdown::keeping`]); one that kept none is kept as its text
    /// cannot be: an empty answer.
    pub fn extend(&mut self, finished: impl IntoIterator<Item = Finished>) {
        for Finished { block, gap, after } in finished {
            let piece = match block {
                Block::Answer(mut markdown) => {
                    Piece::Answer(markdown.take_source().unwrap_or_default())
                }
                Block::Round(mut round) => {
                    bound_output(&mut round);
                    Piece::Block(Block::Round(round))
                }
                block => Piece::Block(block),
            };
            if matches!(piece, Piece::Block(Block::Round(_))) {
                self.rounds.push(self.blocks.len());
            }
            let kept = KeptBlock { piece, gap, after };
            self.bytes += kept.bytes();
            self.blocks.push(kept);
        }
    }

    /// How many blocks it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The block at `index`.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&KeptBlock> {
        self.blocks.get(index)
    }

    /// The blocks, in order.
    pub fn iter(&self) -> impl Iterator<Item = &KeptBlock> {
        self.blocks.iter()
    }

    /// The blocks in `range`, in order.
    #[must_use]
    pub fn slice(&self, range: Range<usize>) -> &[KeptBlock] {
        let end = range.end.min(self.blocks.len());
        &self.blocks[range.start.min(end)..end]
    }

    /// How many rounds it holds.
    #[must_use]
    pub fn rounds(&self) -> usize {
        self.rounds.len()
    }

    /// The block index of round `n`, counted from 0.
    #[must_use]
    pub fn round_block(&self, n: usize) -> Option<usize> {
        self.rounds.get(n).copied()
    }

    /// Which round, counted from 0, the block at `index` is, if it is one.
    #[must_use]
    pub fn round_number(&self, index: usize) -> Option<usize> {
        self.rounds.binary_search(&index).ok()
    }

    /// The answer at `index`, to put colours that came late into.
    pub fn source_mut(&mut self, index: usize) -> Option<&mut Source> {
        match self.blocks.get_mut(index).map(|block| &mut block.piece) {
            Some(Piece::Answer(source)) => Some(source),
            _ => None,
        }
    }

    /// The bytes of text it holds, roughly.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Forgets everything: a session left, or a rewind that draws its
    /// history again.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Cuts each finished command's output to its last [`OUTPUT_KEPT`] bytes.
fn bound_output(round: &mut Round) {
    for call in &mut round.calls {
        if let Detail::Output(output) = &mut call.detail
            && output.len() > OUTPUT_KEPT
        {
            let mut from = output.len() - OUTPUT_KEPT;
            while !output.is_char_boundary(from) {
                from += 1;
            }
            // From the start of a line, so that the first one kept is whole.
            if let Some(newline) = output[from..].find('\n') {
                from += newline + 1;
            }
            output.replace_range(..from, "");
        }
    }
}

fn block_bytes(block: &Block) -> usize {
    match block {
        Block::Header(_) | Block::Summary { .. } => 256,
        Block::User(text) | Block::Notice(text) => text.len(),
        Block::Reasoning(reasoning) => {
            reasoning.text.len() + reasoning.summary.as_ref().map_or(0, String::len)
        }
        Block::Answer(_) => 0,
        Block::Round(round) => round
            .calls
            .iter()
            .map(|call| {
                call.verb.len()
                    + 64
                    + match &call.detail {
                        Detail::None => 0,
                        Detail::Output(output) => output.len(),
                        Detail::Diff(lines) => lines
                            .iter()
                            .map(|line| line.text.len() + 16 + line.emphasis.len() * 16)
                            .sum(),
                    }
            })
            .sum(),
        Block::Listing { title, lines } => {
            title.len() + lines.iter().map(String::len).sum::<usize>()
        }
        Block::Error { message, hint } => message.len() + hint.as_ref().map_or(0, String::len),
    }
}
