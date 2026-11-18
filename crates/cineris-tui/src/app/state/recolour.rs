//! Code highlighted again (since step 25; `docs/steps/25/answered.md`,
//! point 2, "Re-wrapping"; closes `highlighted-code-drawn-again`): a kept
//! answer keeps the colours its code was committed with, which are byte
//! ranges of each source line and so hold at any width. A line committed
//! plain because its colours were late is recorded as late; each code block
//! that holds one is handed to the highlighter again, whole, when its
//! answer is kept, and the colours that come are put into the kept answer,
//! whose rows are then drawn again. Nothing waits for the worker: until its
//! answer comes the line is drawn plain, and the worker's waker brings the
//! next frame.

use crate::highlight::{BlockId, Outcome};
use crate::markdown::Markdown;

use super::App;

/// A code block of a kept answer handed to the highlighter again.
#[derive(Debug, Clone, Copy)]
pub(super) struct Recolouring {
    /// The answer, by its place among the kept blocks.
    block: usize,
    /// The code block, by its place in the answer.
    ordinal: usize,
    /// The block as the highlighter knows it.
    id: BlockId,
    /// How many lines it has.
    lines: usize,
}

impl Recolouring {
    /// The code block as the highlighter knows it.
    pub(super) fn id(&self) -> BlockId {
        self.id
    }
}

impl App {
    /// Hands the highlighter, whole, each code block with a late line of the
    /// answers kept from `from` on.
    pub(super) fn ask_colours(&mut self, from: usize) {
        let highlighter = self.mapper.highlighter().clone();
        for block in from..self.kept.len() {
            let Some(source) = self.kept.source_mut(block) else {
                continue;
            };
            let late: Vec<usize> = source.late().collect();
            if late.is_empty() {
                continue;
            }
            let texts = Markdown::code_lines(source);
            for ordinal in late {
                let lines = texts.get(ordinal).cloned().unwrap_or_default();
                let language = source.code[ordinal].language.clone();
                match highlighter.open(&language) {
                    Some(id) => {
                        for (index, line) in lines.iter().enumerate() {
                            highlighter.line(id, index, line);
                        }
                        self.recolouring.push(Recolouring {
                            block,
                            ordinal,
                            id,
                            lines: lines.len(),
                        });
                    }
                    // No colours to be had: the late lines are plain for
                    // good, and not asked for again.
                    None => {
                        for line in &mut source.code[ordinal].lines {
                            line.get_or_insert_with(Vec::new);
                        }
                    }
                }
            }
        }
    }

    /// Takes the colours the highlighter has finished for the blocks handed
    /// to it again, puts them into the kept answers, and has their rows
    /// drawn again. Says whether any came.
    pub(super) fn take_colours(&mut self) -> bool {
        if self.recolouring.is_empty() {
            return false;
        }
        let highlighter = self.mapper.highlighter().clone();
        let mut came = false;
        let mut left = Vec::new();
        for recolouring in std::mem::take(&mut self.recolouring) {
            let done = (0..recolouring.lines)
                .all(|index| highlighter.peek(recolouring.id, index) != Outcome::Waiting);
            if !done {
                left.push(recolouring);
                continue;
            }
            let Some(source) = self.kept.source_mut(recolouring.block) else {
                highlighter.close(recolouring.id);
                continue;
            };
            if let Some(code) = source.code.get_mut(recolouring.ordinal) {
                for index in 0..recolouring.lines {
                    let colours = match highlighter.take(recolouring.id, index) {
                        Outcome::Coloured(colours) => colours,
                        Outcome::Plain | Outcome::Waiting => Vec::new(),
                    };
                    if let Some(line) = code.lines.get_mut(index) {
                        *line = Some(colours);
                    }
                }
            }
            highlighter.close(recolouring.id);
            self.forget_block_rows(recolouring.block);
            came = true;
        }
        self.recolouring = left;
        came
    }
}
