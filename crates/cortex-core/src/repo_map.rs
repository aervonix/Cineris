//! The repository index of a session: `cortex-index` behind the loop's
//! [`MapSource`] and the tools' [`Outliner`]
//! (`docs/architecture/15-efficiency.md`, "Repository map").
//!
//! - **Nothing on the start-up path.** Making one ([`RepoIndex::new`])
//!   touches no file. The first pass is started when the session's loop is
//!   running ([`RepoIndex::warm`]), on the blocking pool.
//! - **One pass at a time,** on the blocking pool. A map asked for while one
//!   runs is answered when it ends; one asked for when none runs starts
//!   one.
//! - **A pass gives way to a turn.** While a turn runs and nobody waits for
//!   a map, a pass stops at its next file. One that stopped, or ran out of
//!   its time, is carried on when the turn ends: the session's idle slices.
//!   A pass that gave way while a map was asked for is run again for it.
//! - **It stops with the session:** a pass looks at the session's token
//!   between files.
//! - **Nothing is held:** the map and each outline are read from the cache
//!   when asked for, and dropped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cortex_agent::{MapAnswer, MapPending, MapSource};
use cortex_base::{AbsPath, CancelToken};
use cortex_index::{Index, MapRequest, OutlineError, RecentFiles};
use cortex_tools::{Outlined, Outliner};

/// A session's repository index.
#[derive(Clone)]
pub(crate) struct RepoIndex {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for RepoIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoIndex")
            .field("index", &self.shared.index)
            .finish_non_exhaustive()
    }
}

struct Shared {
    index: Index,
    /// The map's budget, in tokens.
    tokens: usize,
    recent: Arc<dyn RecentFiles>,
    /// The session's token: a pass stops when it fires.
    cancel: CancelToken,
    /// A turn is running.
    turn: AtomicBool,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// A pass is running, or about to.
    running: bool,
    /// Maps asked for and not yet answered.
    waiting: Vec<(Vec<String>, MapAnswer)>,
    /// The last pass stopped or ran out of time: there is more to do.
    unfinished: bool,
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl RepoIndex {
    /// The index of a session. Touches no file.
    pub(crate) fn new(
        index: Index,
        tokens: usize,
        recent: Arc<dyn RecentFiles>,
        cancel: CancelToken,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                index,
                tokens,
                recent,
                cancel,
                turn: AtomicBool::new(false),
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// Starts a pass in the background, unless one is running: the first,
    /// once the session's loop runs.
    pub(crate) fn warm(&self) {
        let start = {
            let mut state = lock(&self.shared.state);
            !std::mem::replace(&mut state.running, true)
        };
        if start {
            Arc::clone(&self.shared).spawn();
        }
    }
}

impl Shared {
    fn spawn(self: Arc<Self>) {
        if self.cancel.is_cancelled() {
            // The session has ended: who waits is told there is no map.
            let mut state = lock(&self.state);
            state.running = false;
            state.waiting.clear();
            return;
        }
        // On the blocking pool; with no runtime (a test of the core's
        // parts), on a thread of its own.
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                drop(runtime.spawn_blocking(move || self.work()));
            }
            Err(_) => {
                drop(std::thread::spawn(move || self.work()));
            }
        }
    }

    /// Passes until nobody waits, answering who does.
    fn work(&self) {
        loop {
            let give_way =
                || self.turn.load(Ordering::SeqCst) && lock(&self.state).waiting.is_empty();
            let report = self.index.pass(&self.cancel, &give_way);
            let gave_way = report.partial.contains(&cortex_index::Partial::Stopped)
                && !self.cancel.is_cancelled();
            let waiting = {
                let mut state = lock(&self.state);
                state.unfinished = report.unfinished();
                if !state.waiting.is_empty() && gave_way {
                    // It gave way before a map was asked for: once more,
                    // now for the map.
                    continue;
                }
                std::mem::take(&mut state.waiting)
            };
            if !waiting.is_empty() {
                let recent = self.recent.recent();
                for (touched, answer) in waiting {
                    let request = MapRequest {
                        touched,
                        recent: recent.clone(),
                        tokens: self.tokens,
                    };
                    answer.send(self.index.map(&request));
                }
            }
            let mut state = lock(&self.state);
            if state.waiting.is_empty() {
                state.running = false;
                return;
            }
        }
    }
}

impl MapSource for RepoIndex {
    fn request(&self, touched: Vec<String>) -> MapPending {
        let (answer, pending) = MapPending::channel();
        let start = {
            let mut state = lock(&self.shared.state);
            state.waiting.push((touched, answer));
            !std::mem::replace(&mut state.running, true)
        };
        if start {
            Arc::clone(&self.shared).spawn();
        }
        pending
    }

    fn turn_started(&self) {
        self.shared.turn.store(true, Ordering::SeqCst);
    }

    fn turn_ended(&self) {
        self.shared.turn.store(false, Ordering::SeqCst);
        // An idle slice: carry on with what the last pass left.
        if lock(&self.shared.state).unfinished {
            self.warm();
        }
    }
}

impl Outliner for RepoIndex {
    fn outline(&self, path: &AbsPath) -> Option<String> {
        let found = self.shared.index.outline_of(path).ok()?;
        (!found.outline.entries.is_empty()).then(|| found.outline.render())
    }

    fn describe(&self, path: &AbsPath) -> Result<Outlined, String> {
        match self.shared.index.outline_of(path) {
            Ok(found) => {
                let mut outlined = Outlined::new(
                    found.outline.render(),
                    found
                        .outline
                        .language
                        .map(|language| language.name().to_owned()),
                    found.outline.entries.len(),
                )
                .with_size(found.size);
                if let Some(covered) = found.covered {
                    outlined = outlined.covering(covered);
                }
                Ok(outlined)
            }
            Err(OutlineError::Binary) => Err("it is a binary file".to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }
}
