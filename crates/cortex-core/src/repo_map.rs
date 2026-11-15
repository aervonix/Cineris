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
//! - **Nothing is held:** the map is read from the cache when asked for, an
//!   outline made from its file, and both dropped.
//! - **A pass that panics** answers who waits with no map, and lets the next
//!   request start another. An idle session goes on passing while each
//!   pass makes progress, a few passes at most for each idle slice.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

    /// Passes until nobody waits, answering who does, and while the session
    /// is idle and the last pass left work and made progress.
    fn work(&self) {
        /// How many passes an idle slice carries on with at most, so that a
        /// workspace that never finishes costs a bounded slice.
        const IDLE_PASSES: usize = 8;
        let mut indexed = None;
        let mut idle_passes = 0;
        loop {
            let give_way =
                || self.turn.load(Ordering::SeqCst) && lock(&self.state).waiting.is_empty();
            // A pass that panics (an extractor's bug, a gate's) must not
            // leave the index running for ever and who waits unanswered.
            let passed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.index.pass(&self.cancel, &give_way)
            }));
            let Ok(report) = passed else {
                tracing::warn!("a pass of the repository index failed: no map from it");
                let mut state = lock(&self.state);
                state.waiting.clear();
                state.unfinished = false;
                state.running = false;
                return;
            };
            let gave_way = report.partial.contains(&cortex_index::Partial::Stopped)
                && !self.cancel.is_cancelled();
            // A pass that gave way made no progress, and stands for none
            // either: the idle slice after it is the next one's to use.
            let progressed = gave_way || indexed.is_none_or(|before| report.indexed > before);
            indexed = Some(report.indexed);
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
                let recent =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.recent.recent()))
                        .unwrap_or_default();
                for (touched, answer) in waiting {
                    let request = MapRequest {
                        touched,
                        recent: recent.clone(),
                        tokens: self.tokens,
                    };
                    let map = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.index.map(&request)
                    }))
                    .unwrap_or(None);
                    answer.send(map);
                }
            }
            let mut state = lock(&self.state);
            if !state.waiting.is_empty() {
                continue;
            }
            // An idle slice the turn's end may have looked for too early:
            // carry on while the last pass made progress.
            let idle = !self.turn.load(Ordering::SeqCst) && !self.cancel.is_cancelled();
            if state.unfinished && idle && progressed && idle_passes < IDLE_PASSES {
                idle_passes += 1;
                continue;
            }
            state.running = false;
            return;
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

/// The least time a pass is given to read, whatever the configuration says:
/// less would make an idle session pass over the workspace again for every
/// few files (the second review of step 18).
pub(crate) const LEAST_PASS_TIME: Duration = Duration::from_millis(250);

/// The index's limits from the configuration's `[index]` caps.
pub(crate) fn limits(
    max_files: u64,
    max_file_bytes: u64,
    pass_time: Duration,
) -> cortex_index::Limits {
    cortex_index::Limits {
        max_files: usize::try_from(max_files).unwrap_or(usize::MAX),
        max_file_bytes,
        pass_time: pass_time.max(LEAST_PASS_TIME),
        ..cortex_index::Limits::default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use cortex_base::{FakeClock, PathOptions, PathResolver};
    use cortex_index::{Limits, NoRecentFiles};
    use cortex_testkit::{Scratch, block_on};

    use super::*;

    /// A time limit below the least is raised to it; one above is kept.
    #[test]
    fn the_pass_time_is_never_below_the_least() {
        assert_eq!(limits(10, 10, Duration::ZERO).pass_time, LEAST_PASS_TIME);
        assert_eq!(
            limits(10, 10, Duration::from_millis(1)).pass_time,
            LEAST_PASS_TIME
        );
        assert_eq!(
            limits(10, 10, Duration::from_secs(2)).pass_time,
            Duration::from_secs(2)
        );
        assert_eq!(limits(7, 9, Duration::from_secs(2)).max_files, 7);
        assert_eq!(limits(7, 9, Duration::from_secs(2)).max_file_bytes, 9);
    }

    /// An idle session carries on with unfinished work a few passes at
    /// most, however little each pass may read (the second review of step
    /// 18: a time limit of nothing would pass over the workspace again for
    /// every file).
    #[test]
    fn an_idle_slice_carries_on_a_bounded_number_of_passes() {
        let scratch = Scratch::outside("cortex-core-unit", "an_idle_slice_is_bounded");
        for n in 0..50 {
            scratch.file(format!("ws/f{n:02}.rs"), format!("pub fn f{n}() {{}}\n"));
        }
        let root = PathResolver::new(&scratch.dir("ws"), PathOptions::new())
            .expect("a workspace")
            .root()
            .clone();
        // Each pass lists every file, and asks about this one once.
        let looks = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&looks);
        let gate: Arc<dyn cortex_index::Gate> = Arc::new(move |path: &AbsPath| {
            if path.as_str().ends_with("/f49.rs") {
                counted.fetch_add(1, Ordering::SeqCst);
            }
            true
        });
        let limits = Limits {
            pass_time: std::time::Duration::ZERO,
            ..Limits::default()
        };
        let index = Index::new(
            root,
            scratch.join("index"),
            limits,
            gate,
            // Far after the files were written: each is settled, so a pass
            // that read it keeps it.
            Arc::new(FakeClock::starting_at("2999-01-01T00:00:00Z").expect("a time")),
        )
        .with_threads(1);
        let repo = RepoIndex::new(index, 1500, Arc::new(NoRecentFiles), CancelToken::new());
        // The worker, on this thread: it returns when it stops.
        lock(&repo.shared.state).running = true;
        repo.shared.work();
        let passes = looks.load(Ordering::SeqCst);
        assert_eq!(passes, 9, "one pass and eight more in the idle slice");
        assert!(
            lock(&repo.shared.state).unfinished,
            "the rest is the next slice's"
        );
        assert!(!lock(&repo.shared.state).running);
    }

    /// A clock that ends the session's turn the first time it is read
    /// after being armed: the moment just after a pass's listing.
    #[derive(Debug)]
    struct EndsTheTurn {
        inner: FakeClock,
        armed: Arc<AtomicBool>,
        shared: Arc<std::sync::OnceLock<std::sync::Weak<Shared>>>,
    }

    impl cortex_base::Clock for EndsTheTurn {
        fn now(&self) -> cortex_base::UtcTime {
            self.inner.now()
        }

        fn monotonic(&self) -> Duration {
            if self.armed.swap(false, Ordering::SeqCst)
                && let Some(shared) = self.shared.get().and_then(std::sync::Weak::upgrade)
            {
                shared.turn.store(false, Ordering::SeqCst);
            }
            self.inner.monotonic()
        }

        fn sleep(&self, duration: Duration) {
            self.inner.sleep(duration);
        }
    }

    /// A pass that gave way to a turn which ended before the worker looked
    /// is carried on: the turn's end saw the pass still running and left
    /// the idle slice to it (the second review of step 18).
    #[test]
    fn a_pass_that_gave_way_to_a_turn_already_over_is_carried_on() {
        let scratch = Scratch::outside("cortex-core-unit", "a_pass_that_gave_way_is_carried_on");
        for n in 0..50 {
            scratch.file(format!("ws/f{n:02}.rs"), format!("pub fn f{n}() {{}}\n"));
        }
        let root = PathResolver::new(&scratch.dir("ws"), PathOptions::new())
            .expect("a workspace")
            .root()
            .clone();
        let shared = Arc::new(std::sync::OnceLock::<std::sync::Weak<Shared>>::new());
        let armed = Arc::new(AtomicBool::new(false));
        // The last file closes each whole listing. The first look of the
        // second pass begins a turn, so the pass gives way at its next
        // entry, and arms the clock, which ends the turn right after.
        let looks = Arc::new(AtomicUsize::new(0));
        let begun = AtomicBool::new(false);
        let gate: Arc<dyn cortex_index::Gate> = {
            let (looks, armed, shared) =
                (Arc::clone(&looks), Arc::clone(&armed), Arc::clone(&shared));
            Arc::new(move |path: &AbsPath| {
                if path.as_str().ends_with("/f49.rs") {
                    looks.fetch_add(1, Ordering::SeqCst);
                } else if looks.load(Ordering::SeqCst) == 1 && !begun.swap(true, Ordering::SeqCst) {
                    let shared = shared
                        .get()
                        .and_then(std::sync::Weak::upgrade)
                        .expect("the index");
                    shared.turn.store(true, Ordering::SeqCst);
                    armed.store(true, Ordering::SeqCst);
                }
                true
            })
        };
        let clock = EndsTheTurn {
            inner: FakeClock::starting_at("2999-01-01T00:00:00Z").expect("a time"),
            armed,
            shared: Arc::clone(&shared),
        };
        let limits = Limits {
            pass_time: Duration::ZERO,
            ..Limits::default()
        };
        let index =
            Index::new(root, scratch.join("index"), limits, gate, Arc::new(clock)).with_threads(1);
        let repo = RepoIndex::new(index, 1500, Arc::new(NoRecentFiles), CancelToken::new());
        shared.set(Arc::downgrade(&repo.shared)).expect("set once");
        lock(&repo.shared.state).running = true;
        repo.shared.work();
        // The first pass, the one that gave way (which never reached the
        // last file), and seven more of the idle slice's eight.
        assert_eq!(
            looks.load(Ordering::SeqCst),
            8,
            "the work went on after the turn"
        );
        assert!(!lock(&repo.shared.state).running);
    }

    /// A pass that panics answers who waits with no map, and leaves the
    /// index able to start another (the review of step 18).
    #[test]
    fn a_pass_that_panics_answers_who_waits_and_lets_the_next_one_start() {
        let scratch = Scratch::outside("cortex-core-unit", "a_pass_that_panics");
        scratch.file("ws/src/lib.rs", "pub fn hello() {}\n");
        let root = PathResolver::new(&scratch.dir("ws"), PathOptions::new())
            .expect("a workspace")
            .root()
            .clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let gate: Arc<dyn cortex_index::Gate> = Arc::new(move |_: &AbsPath| {
            assert!(
                counted.fetch_add(1, Ordering::SeqCst) > 0,
                "the first look panics"
            );
            true
        });
        let index = Index::new(
            root,
            scratch.join("index"),
            Limits::default(),
            gate,
            Arc::new(FakeClock::default()),
        );
        let repo = RepoIndex::new(index, 1500, Arc::new(NoRecentFiles), CancelToken::new());
        let mut first = repo.request(Vec::new());
        assert_eq!(
            block_on(first.wait()),
            None,
            "who waited is told there is no map"
        );
        let mut second = repo.request(Vec::new());
        let map = block_on(second.wait()).expect("the next request starts a pass that works");
        assert!(map.contains("src/lib.rs"), "{map}");
    }
}
