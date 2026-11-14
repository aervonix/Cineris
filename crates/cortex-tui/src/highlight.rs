//! Syntax highlighting for code blocks, off the interface's thread
//! (decision 0015; `docs/architecture/09-tui.md`, "Rendering cost
//! control").
//!
//! A [`Highlighter`] is a handle to a worker thread of its own, started the
//! first time a code block asks for it. The worker loads the grammars, keeps
//! each block's parse state, and highlights the block's lines in order; the
//! interface hands it lines and picks up what it has finished, and never
//! waits for it:
//!
//! - **A line waits for its colours for at most [`WAIT`],** counted on the
//!   clock the caller gives, whether the grammars are still loading or the
//!   worker is busy; after that it is committed plain. A frame never waits:
//!   a line still waiting is drawn plain in the live region. Until the
//!   grammars are loaded, code is therefore drawn plain, and the colours
//!   apply to the lines committed after they are.
//! - **A pathological line costs nothing on the interface's thread:** it is
//!   highlighted on the worker, a line over [`LONGEST`] bytes is never
//!   highlighted, and every regular expression is bounded in its
//!   backtracking by its engine. A line the worker gets stuck on runs out its
//!   wait and is drawn plain, and so do the lines queued behind it until the
//!   worker comes back.
//!
//! The colours are the theme's syntax tokens, picked by the grammar's scope
//! names, so highlighting follows the terminal's background and depth.
//!
//! Without the `highlight` feature a highlighter is always off, and costs
//! nothing.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use crate::theme::Token;

/// The longest a completed line waits for its colours before it is
/// committed plain.
pub const WAIT: Duration = Duration::from_millis(50);

/// The longest line, in bytes, that is highlighted.
pub const LONGEST: usize = 1_000;

/// A line's colours: byte ranges of its text and the token each is drawn in.
pub type Colours = Vec<(Range<usize>, Token)>;

/// A handle to the highlighting worker. Cloning it shares the worker.
#[derive(Clone, Default)]
pub struct Highlighter {
    shared: Option<Arc<Shared>>,
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Highlighter")
            .field("on", &self.shared.is_some())
            .finish_non_exhaustive()
    }
}

/// How far the worker is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Highlighting is off: the feature is not built in, or not wanted.
    Off,
    /// Nothing has asked for it yet.
    Idle,
    /// The worker is loading the grammars.
    Loading,
    /// The worker is ready.
    Ready,
    /// The grammars could not be loaded, or the worker could not start.
    Failed,
}

/// A code block known to the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(u64);

/// What became of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not done yet.
    Waiting,
    /// Its colours.
    Coloured(Colours),
    /// Drawn plain: the language is not known, the line is too long, or
    /// highlighting is off or failed.
    Plain,
}

#[cfg_attr(
    not(feature = "highlight"),
    allow(dead_code, reason = "without the feature no worker wakes anyone")
)]
struct Shared {
    state: Mutex<State>,
    work: Condvar,
    waker: Box<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct State {
    stage: Option<Stage>,
    next: u64,
    jobs: VecDeque<Job>,
    done: HashMap<(BlockId, usize), Outcome>,
    /// Whether the worker is in the middle of a job.
    busy: bool,
    /// The thread the worker runs on, for the tests.
    worker: Option<std::thread::ThreadId>,
    /// While set, the worker takes no job: for the test that a frame never
    /// waits for it.
    #[cfg(all(test, feature = "highlight"))]
    held: bool,
}

#[cfg_attr(
    not(feature = "highlight"),
    allow(dead_code, reason = "without the feature no worker reads the jobs")
)]
enum Job {
    Open(BlockId, String),
    Line(BlockId, usize, String),
    Close(BlockId),
}

impl Highlighter {
    /// A highlighter that never colours anything.
    #[must_use]
    pub fn off() -> Self {
        Self { shared: None }
    }

    /// A highlighter whose worker starts on first use. `waker` is called on
    /// the worker's thread each time it has finished something, so that the
    /// interface's loop can draw a frame. Without the `highlight` feature,
    /// the same as [`off`](Self::off).
    #[must_use]
    pub fn new(waker: impl Fn() + Send + Sync + 'static) -> Self {
        if cfg!(not(feature = "highlight")) {
            return Self::off();
        }
        Self {
            shared: Some(Arc::new(Shared {
                state: Mutex::new(State::default()),
                work: Condvar::new(),
                waker: Box::new(waker),
            })),
        }
    }

    /// How far the worker is.
    #[must_use]
    pub fn stage(&self) -> Stage {
        match &self.shared {
            None => Stage::Off,
            Some(shared) => lock(shared).stage.unwrap_or(Stage::Idle),
        }
    }

    /// A new code block in `language` (a fence's info string): starts the
    /// worker if it is not running. `None` when highlighting is off or has
    /// failed.
    pub fn open(&self, language: &str) -> Option<BlockId> {
        let shared = self.shared.as_ref()?;
        let mut state = lock(shared);
        match state.stage {
            Some(Stage::Failed | Stage::Off) => return None,
            None => {
                state.stage = Some(Stage::Loading);
                if !spawn(shared) {
                    state.stage = Some(Stage::Failed);
                    return None;
                }
            }
            Some(_) => {}
        }
        let id = BlockId(state.next);
        state.next += 1;
        state.jobs.push_back(Job::Open(id, language.to_owned()));
        shared.work.notify_all();
        Some(id)
    }

    /// The `index`th line of a block, for the worker to colour. A line too
    /// long is drawn plain at once.
    pub fn line(&self, block: BlockId, index: usize, text: &str) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = lock(shared);
        if text.len() > LONGEST {
            state.done.insert((block, index), Outcome::Plain);
            // The worker still reads past it, so later lines keep their state:
            // as text of no colour.
            state
                .jobs
                .push_back(Job::Line(block, usize::MAX, String::new()));
        } else {
            state
                .jobs
                .push_back(Job::Line(block, index, text.to_owned()));
        }
        shared.work.notify_all();
    }

    /// The block has ended: the worker forgets it.
    pub fn close(&self, block: BlockId) {
        if let Some(shared) = &self.shared {
            lock(shared).jobs.push_back(Job::Close(block));
            shared.work.notify_all();
        }
    }

    /// What became of a line, left for [`take`](Self::take): for drawing the
    /// live region, which must not take a result its commit will need.
    #[must_use]
    pub fn peek(&self, block: BlockId, index: usize) -> Outcome {
        let Some(shared) = &self.shared else {
            return Outcome::Plain;
        };
        let state = lock(shared);
        match state.done.get(&(block, index)) {
            Some(outcome) => outcome.clone(),
            None if matches!(state.stage, Some(Stage::Failed)) => Outcome::Plain,
            None => Outcome::Waiting,
        }
    }

    /// What became of a line, taken: a second call says `Waiting`.
    pub fn take(&self, block: BlockId, index: usize) -> Outcome {
        let Some(shared) = &self.shared else {
            return Outcome::Plain;
        };
        let mut state = lock(shared);
        match state.done.remove(&(block, index)) {
            Some(outcome) => outcome,
            None if matches!(state.stage, Some(Stage::Failed)) => Outcome::Plain,
            None => Outcome::Waiting,
        }
    }

    /// Waits, up to `limit`, until the worker has nothing left to do. For
    /// tests and for drawing a finished document whole; never on the
    /// interface's thread while it draws.
    pub fn settle(&self, limit: Duration) -> bool {
        let Some(shared) = &self.shared else {
            return true;
        };
        let state = lock(shared);
        let (state, timeout) = shared
            .work
            .wait_timeout_while(state, limit, |state| {
                !state.jobs.is_empty() || state.busy || matches!(state.stage, Some(Stage::Loading))
            })
            .unwrap_or_else(PoisonError::into_inner);
        drop(state);
        !timeout.timed_out()
    }

    /// The thread the worker runs on, once it has started.
    #[must_use]
    pub fn worker_thread(&self) -> Option<std::thread::ThreadId> {
        self.shared.as_ref().and_then(|shared| lock(shared).worker)
    }

    /// Stops the worker taking jobs, or lets it go on: for the test that a
    /// frame never waits for it.
    #[cfg(all(test, feature = "highlight"))]
    pub(crate) fn hold(&self, held: bool) {
        if let Some(shared) = &self.shared {
            lock(shared).held = held;
            shared.work.notify_all();
        }
    }
}

fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(feature = "highlight")]
fn spawn(shared: &Arc<Shared>) -> bool {
    let shared = Arc::clone(shared);
    std::thread::Builder::new()
        .name("cortex-highlight".to_owned())
        .spawn(move || engine::run(&shared))
        .is_ok()
}

#[cfg(not(feature = "highlight"))]
fn spawn(_shared: &Arc<Shared>) -> bool {
    false
}

#[cfg(feature = "highlight")]
mod engine {
    //! The worker: syntect's grammars and parser, on its own thread.

    use std::collections::HashMap;

    use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

    use super::{Colours, Job, Outcome, Shared, Stage, lock};
    use crate::theme::Token;

    /// A block's state: where its parse is.
    struct Block {
        parse: ParseState,
        scopes: ScopeStack,
    }

    pub(super) fn run(shared: &Shared) {
        lock(shared).worker = Some(std::thread::current().id());
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let roles = Roles::new();
        {
            let mut state = lock(shared);
            state.stage = Some(if roles.is_some() {
                Stage::Ready
            } else {
                Stage::Failed
            });
        }
        shared.work.notify_all();
        (shared.waker)();
        let Some(roles) = roles else {
            return;
        };
        let mut blocks: HashMap<super::BlockId, Option<Block>> = HashMap::new();
        loop {
            let job = {
                let mut state = lock(shared);
                loop {
                    #[cfg(test)]
                    let held = state.held;
                    #[cfg(not(test))]
                    let held = false;
                    if !held && let Some(job) = state.jobs.pop_front() {
                        state.busy = true;
                        break job;
                    }
                    state.busy = false;
                    shared.work.notify_all();
                    state = shared
                        .work
                        .wait(state)
                        .unwrap_or_else(super::PoisonError::into_inner);
                }
            };
            match job {
                Job::Open(id, language) => {
                    let syntax = syntaxes
                        .find_syntax_by_token(&language)
                        .filter(|_| !language.is_empty());
                    blocks.insert(
                        id,
                        syntax.map(|syntax| Block {
                            parse: ParseState::new(syntax),
                            scopes: ScopeStack::new(),
                        }),
                    );
                }
                Job::Line(id, index, text) => {
                    let outcome = match blocks.get_mut(&id) {
                        Some(Some(block)) => colour(block, &text, &syntaxes, &roles),
                        _ => Outcome::Plain,
                    };
                    if index != usize::MAX {
                        lock(shared).done.insert((id, index), outcome);
                        (shared.waker)();
                    }
                }
                Job::Close(id) => {
                    blocks.remove(&id);
                    // Lines whose wait ran out before their colours came.
                    lock(shared).done.retain(|(block, _), _| *block != id);
                }
            }
        }
    }

    /// One line's colours, the block's state carried on to the next line.
    fn colour(block: &mut Block, text: &str, syntaxes: &SyntaxSet, roles: &Roles) -> Outcome {
        let line = format!("{text}\n");
        let Ok(ops) = block.parse.parse_line(&line, syntaxes) else {
            return Outcome::Plain;
        };
        let mut colours: Colours = Vec::new();
        let mut at = 0;
        let mut push = |end: usize, stack: &ScopeStack, colours: &mut Colours| {
            let end = end.min(text.len());
            if end > at {
                if let Some(token) = roles.of(stack) {
                    match colours.last_mut() {
                        Some((range, last)) if *last == token && range.end == at => range.end = end,
                        _ => colours.push((at..end, token)),
                    }
                }
                at = end;
            }
        };
        for (offset, op) in ops {
            push(offset, &block.scopes, &mut colours);
            if block.scopes.apply(&op).is_err() {
                return Outcome::Plain;
            }
        }
        push(text.len(), &block.scopes, &mut colours);
        Outcome::Coloured(colours)
    }

    /// The scope names a token is picked by, innermost scope first.
    struct Roles {
        table: Vec<(Scope, Option<Token>)>,
    }

    impl Roles {
        fn new() -> Option<Self> {
            let names: [(&str, Option<Token>); 14] = [
                ("comment", Some(Token::SyntaxComment)),
                ("string", Some(Token::SyntaxString)),
                ("constant.character", Some(Token::SyntaxString)),
                ("constant.numeric", Some(Token::SyntaxNumber)),
                ("constant.language", Some(Token::SyntaxNumber)),
                // Operators are not keywords.
                ("keyword.operator", None),
                ("keyword", Some(Token::SyntaxKeyword)),
                ("storage.type", Some(Token::SyntaxKeyword)),
                ("storage.modifier", Some(Token::SyntaxKeyword)),
                ("entity.name.type", Some(Token::SyntaxType)),
                ("entity.name.class", Some(Token::SyntaxType)),
                ("support.type", Some(Token::SyntaxType)),
                ("support.class", Some(Token::SyntaxType)),
                ("variable.language", Some(Token::SyntaxType)),
            ];
            let mut table = Vec::with_capacity(names.len());
            for (name, token) in names {
                table.push((Scope::new(name).ok()?, token));
            }
            Some(Self { table })
        }

        /// The token of the innermost scope that names one.
        fn of(&self, stack: &ScopeStack) -> Option<Token> {
            for scope in stack.as_slice().iter().rev() {
                for (prefix, token) in &self.table {
                    if prefix.is_prefix_of(*scope) {
                        return *token;
                    }
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests;
