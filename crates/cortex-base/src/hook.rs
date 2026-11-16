//! Points in library code where a test can make the library stop
//! (`docs/testing-and-ci.md`, "A point the library stops at").
//!
//! A race test needs the library to stop at a precise place: between the
//! read and the write, between the new content being written and the
//! rename. Nothing from outside can make it stop there, so such tests used
//! to rely on timing, on a thread that was held by hand, on a FIFO or on a
//! loop that tries until the race happens. Library code marks the place:
//!
//! ```ignore
//! hook::point("write_atomic: the content is written, before the rename");
//! ```
//!
//! and a test holds it:
//!
//! ```ignore
//! let hold = Hold::new("write_atomic: the content is written, before the rename");
//! let writer = std::thread::spawn(|| write_atomic(…));
//! assert!(hold.wait_reached(LIMIT)); // the writer is stopped there
//! // … look at the file, change it …
//! hold.release();
//! writer.join().unwrap();
//! ```
//!
//! # A point on an async task: [`await_point!`]
//!
//! [`point`] stops the *thread*. On a task of an async runtime that is the
//! runtime's thread, and on a single-threaded runtime (the one a test of
//! the core drives everything on) nothing else runs until the hold lets it
//! go: not the test, which would let it go, and not the work the test
//! wants to do while the library is stopped. A place reached by a task
//! marks itself with [`await_point!`] instead, in an async fn or block:
//!
//! ```ignore
//! cortex_base::hook::await_point!(BEFORE_THE_NEW_LEVEL_IS_IN_FORCE);
//! ```
//!
//! With the feature it awaits `point_async(name)`. Held, the task is
//! pending there and the runtime goes on with its other tasks; the hold's
//! `release` or `let_one_go` wakes it, and so does its bound. A [`Hold`]
//! holds either kind of point, and counts, lists and lets go a stopped
//! task as it does a stopped thread. The test's side has an awaitable wait
//! too (`Hold::wait_reached_times_async`), for a test that drives the
//! library on its own thread.
//!
//! # With the feature off, a point is nothing
//!
//! [`point`] is an empty `const fn` marked `#[inline(always)]`. A build
//! without the `test-hooks` feature (every build of the product: the
//! released binary, `cargo build -p cortex-cli`) has no code for it, no
//! symbol, no data, not even the load of a flag. [`await_point!`] is a
//! macro and not a function because an await is never nothing: an await
//! of even a future that is always ready adds a state to the async fn's
//! state machine, and keeps what lives across it in the future, which
//! grows (`docs/testing-and-ci.md`, "A point the library stops at"). So
//! without the feature the macro expands to no await at all, only a check
//! that the name is a `&'static str`. [`Hold`] and everything else
//! test-side do not exist there. Only a dev-dependency turns the feature on, and `cargo
//! xtask rules` fails when anything else does (`docs/testing-and-ci.md`,
//! "A seeded order of events").
//!
//! # With the feature on and nothing held, a point is one load
//!
//! A test build has the feature on, for every test and for the binaries the
//! system tests start (features are unified across a build of the whole
//! workspace's tests). A point that no [`Hold`] names passes at once: it
//! loads one counter, the number of holds alive in the process, and
//! returns when it is zero. That is the case in the product's code under
//! every test that holds nothing, and in every system test. An awaited
//! point does the same at its first poll, and is ready.
//!
//! # What a [`Hold`] does
//!
//! - **It names a point and stops every arrival there.** A thread that
//!   arrives waits until the test lets it go, or until the hold's bound
//!   (5 s unless [`Hold::with_limit`] says otherwise) has passed. After
//!   the bound the thread goes on and the hold remembers it
//!   ([`timed_out`](Hold::timed_out)): a mistake in a test is a failed
//!   test, never a hung one. Every wait of the test's side takes a bound
//!   too. A task that arrives at an awaited point is pending under the
//!   same rules, and a task dropped while it is stopped is no longer
//!   counted as stopped.
//! - **The n-th arrival** is held with [`skipping`](Hold::skipping): the
//!   first `n` arrivals pass and the next ones stop.
//! - **Letting go.** [`release`](Hold::release) lets every thread stopped
//!   go, and every later arrival pass. [`let_one_go`](Hold::let_one_go)
//!   lets one go (one of those stopped, in no order, or the next to
//!   arrive) and keeps
//!   holding the others. Dropping the hold releases it, so a test that
//!   fails never leaves a thread stopped.
//! - **Two tests in one process.** `cargo test` runs tests as threads of
//!   one process, and a hold is the process's. Two holds of one point do
//!   not live together: the second waits, within its bound, for the first
//!   to be dropped (and fails with a message that says so, not a hang).
//!   But any thread of the process that arrives at a held point stops,
//!   whichever test it belongs to, because the library cannot know. nextest
//!   gives each test a process of its own, which is how the workspace runs
//!   its tests, and there a hold is the test's alone. A test that holds a
//!   point is written for that.
//! - **A wrong name is found.** The points that threads reached while any
//!   hold was alive are remembered ([`seen`]), and the test kit's wait says
//!   which they were when the point it named never came.
//! - **The timeout report names it** (`cortex_testkit::watch`): the points
//!   at which a thread is stopped now, and for how long ([`stopped`]).

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test support behind the `test-hooks` feature: a blocking wait with a bound"
)]

/// Marks a place in library code that a test can make the thread stop at.
/// Without the `test-hooks` feature this is nothing at all.
///
/// The name says where the point is and what has happened by then, as a
/// sentence a failure can print: `"write_atomic: the content is written,
/// before the rename"`. The module that has the point exports the name as a
/// constant, so that a test cannot misspell it.
#[cfg(not(feature = "test-hooks"))]
#[inline(always)]
pub const fn point(_name: &'static str) {}

/// Marks the start of a process: called by every place in the workspace
/// that starts one, with the program it starts, just before it does
/// (`docs/testing-and-ci.md`, "What a runner would say, from a desk").
/// Without the `test-hooks` feature this is nothing at all.
///
/// A start is what a hosted Windows runner charges most for (a PowerShell
/// start is a third of a second there), and what a desk's run cannot see:
/// with [`STARTS`] naming a directory, under nextest, each start is written
/// down for the test that made it, and `cargo xtask ci --tiers
/// --like-runner` counts them.
#[cfg(not(feature = "test-hooks"))]
#[inline(always)]
pub const fn started(_program: &std::ffi::OsStr) {}

/// Marks the sandbox's `launcher` being made to become `program` (an
/// `exec` in the launcher, not a new process): the launcher's own start,
/// marked as `launcher`'s when it is started, is counted as `program`'s.
/// Marked by the harness when it wraps the command, since the launcher's
/// process is given no environment of the test's. Without the `test-hooks`
/// feature this is nothing at all.
#[cfg(not(feature = "test-hooks"))]
#[inline(always)]
pub const fn launched(_launcher: &std::ffi::OsStr, _program: &std::ffi::OsStr) {}

/// What a line of [`launched`] starts with, before the program's name.
pub const LAUNCHED: &str = "launcher>";

/// Marks a start of `program` to ask it its version (`--version`), apart
/// from the starts that run a command: on Linux the harness asks bash its
/// version on the side, where on Windows the tests' shell is asked nothing,
/// so a desk's estimate must not charge one for it. Without the
/// `test-hooks` feature this is nothing at all.
#[cfg(not(feature = "test-hooks"))]
#[inline(always)]
pub const fn probed(_program: &std::ffi::OsStr) {}

/// What a line of [`probed`] starts with, before the program's name.
pub const PROBED: &str = "probe>";

/// The variable that names the directory [`started`] writes to. Unset or
/// empty: nothing is written.
pub const STARTS: &str = "CORTEX_TESTKIT_STARTS";

#[cfg(feature = "test-hooks")]
pub use starts::{launched, probed, program_name, started};

/// Process starts, written down for the test that made them.
#[cfg(feature = "test-hooks")]
mod starts {
    use std::ffi::OsStr;
    use std::io::Write as _;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    use crate::sync::lock;

    /// Where this process writes, and the test it writes for: read once.
    struct Record {
        file: Mutex<std::fs::File>,
        /// `binary-id\ttest-name\t`, the start of every line.
        test: String,
    }

    static RECORD: OnceLock<Option<Record>> = OnceLock::new();

    /// Writes one line, `<binary id>\t<test name>\t<program>`, to a file of
    /// this process's own in the directory [`super::STARTS`] names, when it
    /// names one and the process runs under nextest (which says which test
    /// it is, in `NEXTEST_BINARY_ID` and `NEXTEST_TEST_NAME`, and gives the
    /// variables to every process the test starts). Nothing is written,
    /// and nothing fails, otherwise: a start is never refused for this.
    pub fn started(program: &OsStr) {
        write("", program);
    }

    /// As [`started`], for `launcher` made to become `program`: the line's
    /// program is `launcher>LAUNCHER>PROGRAM`.
    pub fn launched(launcher: &OsStr, program: &OsStr) {
        let prefix = format!("{}{}>", super::LAUNCHED, program_name(launcher));
        write(&prefix, program);
    }

    /// As [`started`], for a start that asks `program` its version: the
    /// line's program is `probe>NAME`.
    pub fn probed(program: &OsStr) {
        write(super::PROBED, program);
    }

    fn write(prefix: &str, program: &OsStr) {
        let Some(record) = RECORD.get_or_init(open) else {
            return;
        };
        let line = format!("{}{prefix}{}\n", record.test, program_name(program));
        let _ = lock(&record.file).write_all(line.as_bytes());
    }

    /// The name a start is counted by: the program's file name without its
    /// extension, in lower case (`C:\…\pwsh.exe` is `pwsh`), with no tab or
    /// line break.
    #[must_use]
    pub fn program_name(program: &OsStr) -> String {
        let path = Path::new(program);
        let stem = path.file_stem().unwrap_or(program).to_string_lossy();
        stem.to_lowercase().replace(['\t', '\n', '\r'], " ")
    }

    fn open() -> Option<Record> {
        let directory = std::env::var_os(super::STARTS).filter(|value| !value.is_empty())?;
        let binary = std::env::var("NEXTEST_BINARY_ID").ok()?;
        let test = std::env::var("NEXTEST_TEST_NAME").ok()?;
        std::fs::create_dir_all(&directory).ok()?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(Path::new(&directory).join(format!("{}.starts", std::process::id())))
            .ok()?;
        let clean = |text: &str| text.replace(['\t', '\n', '\r'], " ");
        Some(Record {
            file: Mutex::new(file),
            test: format!("{}\t{}\t", clean(&binary), clean(&test)),
        })
    }
}

/// The macro behind [`await_point!`], without the `test-hooks` feature: no
/// await, nothing but the name's type checked.
#[cfg(not(feature = "test-hooks"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __hook_await_point {
    ($name:expr) => {{
        let _: &'static str = $name;
    }};
}

/// The macro behind [`await_point!`], with the `test-hooks` feature: an
/// await of `point_async`.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
#[macro_export]
macro_rules! __hook_await_point {
    ($name:expr) => {
        $crate::hook::point_async($name).await
    };
}

/// Marks a place that an async task reaches, where a test can make the
/// *task* wait without stopping its thread: [`point`] for code on a
/// runtime's thread. Written as a statement in an async fn or block:
/// `cortex_base::hook::await_point!(NAME);`. With the `test-hooks`
/// feature it is `point_async(NAME).await`; without it, nothing at all,
/// not even an await (see the [module](self)).
pub use crate::__hook_await_point as await_point;

#[cfg(feature = "test-hooks")]
pub use enabled::{Hold, Stopped, point, point_async, seen, stopped};

#[cfg(feature = "test-hooks")]
mod enabled {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};

    use crate::sync::lock;

    /// How many holds are alive in this process. A point that sees none
    /// does nothing else.
    static LIVE: AtomicUsize = AtomicUsize::new(0);

    /// The number the next stop is known by.
    static NEXT_STOP: AtomicU64 = AtomicU64::new(0);

    static REGISTRY: Registry = Registry {
        state: Mutex::new(RegistryState {
            holds: Vec::new(),
            seen: Vec::new(),
            waiting: 0,
        }),
        freed: Condvar::new(),
    };

    struct Registry {
        state: Mutex<RegistryState>,
        /// Signalled when a hold is dropped, for a hold that waits for its
        /// point's name.
        freed: Condvar,
    }

    struct RegistryState {
        holds: Vec<Arc<Shared>>,
        /// The points reached while a hold was alive, each once.
        seen: Vec<&'static str>,
        /// How many holds are waiting for another hold of their point to
        /// end, for the test of that wait.
        waiting: usize,
    }

    /// Marks a place in library code that a test can make the thread stop
    /// at. With nothing held it is one load and a return.
    #[inline]
    pub fn point(name: &'static str) {
        if LIVE.load(Ordering::Acquire) == 0 {
            return;
        }
        arrive(name);
    }

    #[cold]
    #[inline(never)]
    fn arrive(name: &'static str) {
        if let Some(hold) = held(name) {
            hold.arrive();
        }
    }

    /// The hold of `name`, if one is alive; the point is remembered as
    /// reached either way.
    fn held(name: &'static str) -> Option<Arc<Shared>> {
        let mut registry = lock(&REGISTRY.state);
        if !registry.seen.contains(&name) {
            registry.seen.push(name);
        }
        registry
            .holds
            .iter()
            .find(|hold| hold.point == name)
            .cloned()
    }

    /// Marks a place that an async task reaches: held, the task is pending
    /// there, and its thread goes on with other tasks, until the hold lets
    /// it go or its bound passes. With nothing held the first poll is one
    /// load and `Ready`.
    pub fn point_async(name: &'static str) -> impl Future<Output = ()> + Send + 'static {
        AtPoint {
            name,
            stop: None,
            done: false,
        }
    }

    /// The future of [`point_async`].
    struct AtPoint {
        name: &'static str,
        /// The stop, once the task arrived at a held point and stopped.
        stop: Option<TaskStop>,
        done: bool,
    }

    struct TaskStop {
        hold: Arc<Shared>,
        /// The number its record in the hold is known by.
        number: u64,
        deadline: Instant,
        alarm: Arc<Alarm>,
    }

    impl Future for AtPoint {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.done {
                return Poll::Ready(());
            }
            if self.stop.is_none() {
                if LIVE.load(Ordering::Acquire) == 0 {
                    self.done = true;
                    return Poll::Ready(());
                }
                let Some(hold) = held(self.name) else {
                    self.done = true;
                    return Poll::Ready(());
                };
                let thread = std::thread::current();
                let who = format!(
                    "a task polled on {}",
                    thread.name().unwrap_or("an unnamed thread")
                );
                let mut state = lock(&hold.state);
                let Some(number) = Shared::stop(&mut state, who) else {
                    drop(state);
                    self.done = true;
                    return Poll::Ready(());
                };
                drop(state);
                hold.changed.notify_all();
                let deadline = Instant::now() + hold.limit;
                self.stop = Some(TaskStop {
                    hold,
                    number,
                    deadline,
                    alarm: Arc::new(Alarm::default()),
                });
            }
            let Some(stop) = &self.stop else {
                return Poll::Ready(());
            };
            let mut state = lock(&stop.hold.state);
            if Shared::may_go(&mut state) {
                // Let go by the test.
            } else if Instant::now() >= stop.deadline {
                state.timed_out += 1;
            } else {
                drop(state);
                let number = stop.number;
                stop.alarm.arm(
                    &stop.hold,
                    stop.deadline,
                    cx.waker(),
                    move |state: &State| {
                        state.released
                            || state.permits > 0
                            || !state.waiting.iter().any(|waiting| waiting.number == number)
                    },
                );
                return Poll::Pending;
            }
            Shared::leave(&mut state, stop.number);
            drop(state);
            stop.hold.changed.notify_all();
            self.stop = None;
            self.done = true;
            Poll::Ready(())
        }
    }

    impl Drop for AtPoint {
        fn drop(&mut self) {
            // A task dropped while it is stopped is no longer stopped.
            if let Some(stop) = self.stop.take() {
                Shared::leave(&mut lock(&stop.hold.state), stop.number);
                stop.alarm.disarm(&stop.hold);
            }
        }
    }

    /// What wakes a pending future of this module: a thread of its own
    /// that waits on the hold's condition variable until what the future
    /// waits for is so, or its bound passes, and wakes the task. A thread
    /// and not a timer, because this crate has no runtime: the bound is a
    /// bound, and a passing test never waits for it.
    #[derive(Default)]
    struct Alarm {
        /// The task to wake, the latest poll's.
        waker: Mutex<Option<Waker>>,
        /// A thread is waiting for this future now.
        armed: AtomicBool,
        /// The future is gone: the thread waits no longer.
        gone: AtomicBool,
    }

    impl Alarm {
        fn arm(
            self: &Arc<Self>,
            hold: &Arc<Shared>,
            deadline: Instant,
            waker: &Waker,
            woken_when: impl Fn(&State) -> bool + Send + 'static,
        ) {
            {
                let mut kept = lock(&self.waker);
                match kept.as_ref() {
                    Some(kept_waker) if kept_waker.will_wake(waker) => {}
                    _ => *kept = Some(waker.clone()),
                }
            }
            if self.armed.swap(true, Ordering::AcqRel) {
                return;
            }
            let (alarm, hold) = (Arc::clone(self), Arc::clone(hold));
            let started = std::thread::Builder::new()
                .name("cortex-base hook alarm".to_owned())
                .spawn(move || {
                    let left = deadline.saturating_duration_since(Instant::now());
                    let state = lock(&hold.state);
                    let state = hold
                        .changed
                        .wait_timeout_while(state, left, |state| {
                            !alarm.gone.load(Ordering::Acquire) && !woken_when(state)
                        })
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .0;
                    drop(state);
                    // Disarmed before the wake, so that a poll that is
                    // still pending after it arms another.
                    alarm.armed.store(false, Ordering::Release);
                    if let Some(waker) = lock(&alarm.waker).take() {
                        waker.wake();
                    }
                });
            if started.is_err() {
                // No thread to wait with: poll again at once, and wait by
                // polling.
                self.armed.store(false, Ordering::Release);
                waker.wake_by_ref();
            }
        }

        /// Ends the thread's wait, for a future that is dropped. Under the
        /// hold's lock, so that the thread cannot miss it between its look
        /// and its wait.
        fn disarm(&self, hold: &Shared) {
            let state = lock(&hold.state);
            self.gone.store(true, Ordering::Release);
            drop(state);
            hold.changed.notify_all();
        }
    }

    /// The points that threads reached while any hold was alive, in the
    /// order they first were, since the last hold before them was dropped.
    /// For the message of a test whose point never came: a misspelled name
    /// shows up here as its near neighbour.
    #[must_use]
    pub fn seen() -> Vec<&'static str> {
        lock(&REGISTRY.state).seen.clone()
    }

    /// A thread, or a task, that is stopped at a point now.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Stopped {
        /// The point.
        pub point: String,
        /// The thread's name, or a stand-in for one with none; for a task
        /// at an awaited point, "a task polled on" the thread that first
        /// polled it.
        pub thread: String,
        /// How long it has been stopped.
        pub for_how_long: Duration,
    }

    /// Every thread stopped at a point of any hold, longest first. For the
    /// test kit's report of a test that ran out of time.
    #[must_use]
    pub fn stopped() -> Vec<Stopped> {
        let holds = lock(&REGISTRY.state).holds.clone();
        let mut all: Vec<Stopped> = holds
            .iter()
            .flat_map(|hold| {
                lock(&hold.state)
                    .waiting
                    .iter()
                    .map(|waiting| Stopped {
                        point: hold.point.clone(),
                        thread: waiting.name.clone(),
                        for_how_long: waiting.since.elapsed(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        all.sort_by_key(|stopped| std::cmp::Reverse(stopped.for_how_long));
        all
    }

    struct Shared {
        point: String,
        /// How long a thread stays stopped before it goes on by itself.
        limit: Duration,
        state: Mutex<State>,
        changed: Condvar,
    }

    #[derive(Default)]
    struct State {
        /// Every arrival, the ones let through included.
        arrivals: usize,
        /// The arrivals that pass before the first stop.
        skip: usize,
        /// The arrivals that stopped.
        stops: usize,
        /// The stopped threads and tasks, longest first.
        waiting: Vec<Waiting>,
        /// Threads that may go on, one each.
        permits: usize,
        /// Everything goes on, now and from here on.
        released: bool,
        /// Threads that went on because the bound ran out.
        timed_out: usize,
    }

    struct Waiting {
        number: u64,
        name: String,
        since: Instant,
    }

    impl Shared {
        /// Counts an arrival, and whether it stops: the number its record
        /// is known by when it does.
        fn stop(state: &mut State, who: String) -> Option<u64> {
            state.arrivals += 1;
            if state.released || state.arrivals <= state.skip {
                return None;
            }
            state.stops += 1;
            let number = NEXT_STOP.fetch_add(1, Ordering::Relaxed);
            state.waiting.push(Waiting {
                number,
                name: who,
                since: Instant::now(),
            });
            Some(number)
        }

        /// Whether a stopped arrival may go on now, taking the permit that
        /// lets it when there is one.
        fn may_go(state: &mut State) -> bool {
            if state.released {
                return true;
            }
            if state.permits > 0 {
                state.permits -= 1;
                return true;
            }
            false
        }

        /// The record of a stop that ended.
        fn leave(state: &mut State, number: u64) {
            if let Some(index) = state
                .waiting
                .iter()
                .position(|waiting| waiting.number == number)
            {
                state.waiting.remove(index);
            }
        }

        fn arrive(&self) {
            let mut state = lock(&self.state);
            let thread = std::thread::current();
            let who = thread.name().unwrap_or("an unnamed thread").to_owned();
            let Some(number) = Self::stop(&mut state, who) else {
                return;
            };
            self.changed.notify_all();
            let (mut state, _) = self
                .changed
                .wait_timeout_while(state, self.limit, |state| {
                    !state.released && state.permits == 0
                })
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !Self::may_go(&mut state) {
                state.timed_out += 1;
            }
            Self::leave(&mut state, number);
            drop(state);
            self.changed.notify_all();
        }
    }

    /// A test's hold on a point of library code: every thread that arrives
    /// there stops until the test lets it go. See the [module](self).
    #[must_use = "a hold that is dropped at once stops nothing for long"]
    pub struct Hold {
        shared: Arc<Shared>,
    }

    impl std::fmt::Debug for Hold {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let state = lock(&self.shared.state);
            f.debug_struct("Hold")
                .field("point", &self.shared.point)
                .field("arrivals", &state.arrivals)
                .field("stopped", &state.waiting.len())
                .field("released", &state.released)
                .finish()
        }
    }

    impl Hold {
        /// The default bound on how long a thread stays stopped and on how
        /// long a hold waits for another hold of the same point to end.
        pub const DEFAULT_LIMIT: Duration = Duration::from_secs(5);

        /// Holds `point`, with the default bound.
        ///
        /// # Panics
        ///
        /// When another hold of the same point is alive and does not end
        /// within the bound: two tests of one process holding one point.
        pub fn new(point: &str) -> Self {
            Self::with_limit(point, Self::DEFAULT_LIMIT)
        }

        /// Holds `point`; a thread stopped there goes on by itself after
        /// `limit`, and so does the wait for another hold of the point.
        ///
        /// # Panics
        ///
        /// As [`new`](Self::new).
        pub fn with_limit(point: &str, limit: Duration) -> Self {
            let deadline = Instant::now() + limit;
            let mut registry = lock(&REGISTRY.state);
            while registry.holds.iter().any(|hold| hold.point == point) {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !left.is_zero(),
                    "another hold of the point `{point}` is still alive after {limit:?}: two \
                     tests of one process hold one point, which `cargo test` allows and nextest \
                     (a process for each test) does not meet"
                );
                registry.waiting += 1;
                registry = REGISTRY
                    .freed
                    .wait_timeout(registry, left)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
                registry.waiting -= 1;
            }
            if registry.holds.is_empty() {
                registry.seen.clear();
            }
            let shared = Arc::new(Shared {
                point: point.to_owned(),
                limit,
                state: Mutex::new(State::default()),
                changed: Condvar::new(),
            });
            registry.holds.push(Arc::clone(&shared));
            LIVE.fetch_add(1, Ordering::Release);
            Self { shared }
        }

        /// Lets the first `arrivals` arrivals through, and stops the ones
        /// after: the n-th arrival is held. Call it before the library
        /// can arrive.
        pub fn skipping(self, arrivals: usize) -> Self {
            lock(&self.shared.state).skip = arrivals;
            self
        }

        /// Waits until a thread of the library has stopped at the point,
        /// for at most `limit`. Whether it has.
        #[must_use]
        pub fn wait_reached(&self, limit: Duration) -> bool {
            self.wait_reached_times(1, limit)
        }

        /// Waits until `stops` arrivals have stopped here (together, or
        /// one after another, the ones let go included), for at most
        /// `limit`. Whether they have.
        #[must_use]
        pub fn wait_reached_times(&self, stops: usize, limit: Duration) -> bool {
            let state = lock(&self.shared.state);
            let (state, _) = self
                .shared
                .changed
                .wait_timeout_while(state, limit, |state| state.stops < stops)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.stops >= stops
        }

        /// [`wait_reached_times`](Self::wait_reached_times) as a future,
        /// for a test whose library runs on the test's own thread: it is
        /// pending, and lets the runtime run the library, until `stops`
        /// arrivals have stopped here or `limit` has passed. Whether they
        /// have.
        pub fn wait_reached_times_async(
            &self,
            stops: usize,
            limit: Duration,
        ) -> impl Future<Output = bool> + Send + 'static {
            Reached {
                hold: Arc::clone(&self.shared),
                stops,
                deadline: Instant::now() + limit,
                alarm: Arc::new(Alarm::default()),
            }
        }

        /// Lets every thread stopped here go on, and every later arrival
        /// pass.
        pub fn release(&self) {
            lock(&self.shared.state).released = true;
            self.shared.changed.notify_all();
        }

        /// Lets one thread go on: one of those stopped now, whichever wakes
        /// first (in no order), or the next to arrive. The others stay.
        pub fn let_one_go(&self) {
            lock(&self.shared.state).permits += 1;
            self.shared.changed.notify_all();
        }

        /// Waits until exactly `stopped` threads are stopped here, for at
        /// most `limit`: after [`release`](Self::release) or
        /// [`let_one_go`](Self::let_one_go), until the thread has gone on
        /// (`0`, or one fewer). Whether that is so.
        #[must_use]
        pub fn wait_stopped(&self, stopped: usize, limit: Duration) -> bool {
            let state = lock(&self.shared.state);
            let (state, _) = self
                .shared
                .changed
                .wait_timeout_while(state, limit, |state| state.waiting.len() != stopped)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.waiting.len() == stopped
        }

        /// The point.
        #[must_use]
        pub fn point(&self) -> &str {
            &self.shared.point
        }

        /// How many threads have arrived at the point since the hold was
        /// made, the ones let through included.
        #[must_use]
        pub fn arrivals(&self) -> usize {
            lock(&self.shared.state).arrivals
        }

        /// How many threads are stopped at the point now.
        #[must_use]
        pub fn stopped(&self) -> usize {
            lock(&self.shared.state).waiting.len()
        }

        /// How many threads went on because the bound ran out, and not
        /// because the test let them go. A test that expects none asserts
        /// it.
        #[must_use]
        pub fn timed_out(&self) -> usize {
            lock(&self.shared.state).timed_out
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            self.release();
            let mut registry = lock(&REGISTRY.state);
            registry
                .holds
                .retain(|hold| !Arc::ptr_eq(hold, &self.shared));
            LIVE.fetch_sub(1, Ordering::Release);
            drop(registry);
            REGISTRY.freed.notify_all();
        }
    }

    /// The future of [`Hold::wait_reached_times_async`].
    struct Reached {
        hold: Arc<Shared>,
        stops: usize,
        deadline: Instant,
        alarm: Arc<Alarm>,
    }

    impl Future for Reached {
        type Output = bool;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<bool> {
            let stops = self.stops;
            if lock(&self.hold.state).stops >= stops {
                return Poll::Ready(true);
            }
            if Instant::now() >= self.deadline {
                return Poll::Ready(false);
            }
            self.alarm.arm(
                &self.hold,
                self.deadline,
                cx.waker(),
                move |state: &State| state.stops >= stops,
            );
            Poll::Pending
        }
    }

    impl Drop for Reached {
        fn drop(&mut self) {
            self.alarm.disarm(&self.hold);
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::atomic::AtomicBool;
        use std::time::Duration;

        use super::*;

        const LIMIT: Duration = Duration::from_secs(5);

        // One test at a time holds a point of this module's tests, so that
        // `cargo test`'s threads do not meet each other's holds: each test
        // names a point of its own.

        #[test]
        fn a_point_nobody_holds_passes_at_once() {
            point("hook tests: nobody holds this");
            let hold = Hold::new("hook tests: another point");
            point("hook tests: nobody holds this");
            assert_eq!(hold.arrivals(), 0);
            assert_eq!(hold.stopped(), 0);
        }

        #[test]
        fn a_thread_stops_at_a_held_point_until_the_hold_releases_it() {
            const AT: &str = "hook tests: stops until released";
            let hold = Hold::new(AT);
            let past = Arc::new(AtomicBool::new(false));
            let thread = {
                let past = Arc::clone(&past);
                std::thread::Builder::new()
                    .name("the stopped one".to_owned())
                    .spawn(move || {
                        point(AT);
                        past.store(true, Ordering::SeqCst);
                    })
                    .unwrap()
            };
            assert!(hold.wait_reached(LIMIT), "the thread never arrived");
            assert!(!past.load(Ordering::SeqCst), "it stopped at the point");
            assert_eq!(hold.stopped(), 1);
            let listed = stopped();
            assert!(
                listed
                    .iter()
                    .any(|each| each.point == AT && each.thread == "the stopped one"),
                "{listed:?}"
            );
            hold.release();
            thread.join().unwrap();
            assert!(past.load(Ordering::SeqCst), "it went on after the release");
            assert_eq!(hold.timed_out(), 0);
            assert_eq!(hold.arrivals(), 1);
            // After a release the point passes at once.
            point(AT);
            assert_eq!(hold.arrivals(), 2);
        }

        #[test]
        fn the_bound_lets_a_thread_go_on_and_says_so() {
            const AT: &str = "hook tests: the bound";
            let hold = Hold::with_limit(AT, Duration::ZERO);
            point(AT);
            assert_eq!(hold.timed_out(), 1, "nobody let it go");
            assert_eq!(hold.stopped(), 0);
            assert!(!hold.wait_reached_times(2, Duration::ZERO));
        }

        #[test]
        fn a_wait_for_a_point_nothing_reaches_ends_at_its_bound() {
            let hold = Hold::new("hook tests: never reached");
            assert!(!hold.wait_reached(Duration::ZERO));
            assert!(hold.wait_stopped(0, Duration::ZERO));
            assert!(
                !hold.wait_stopped(1, Duration::ZERO),
                "nobody is stopped, and the wait says so at its bound"
            );
        }

        #[test]
        fn the_nth_arrival_is_the_one_that_stops() {
            const AT: &str = "hook tests: the third";
            let hold = Hold::new(AT).skipping(2);
            point(AT);
            point(AT);
            assert_eq!(hold.arrivals(), 2);
            assert_eq!(hold.stopped(), 0, "the first two passed");
            let third = std::thread::spawn(|| point(AT));
            assert!(hold.wait_reached(LIMIT));
            assert_eq!(hold.stopped(), 1);
            hold.release();
            third.join().unwrap();
            assert_eq!(hold.timed_out(), 0, "the release let it go, not the bound");
        }

        #[test]
        fn one_thread_is_let_go_and_the_others_stay() {
            const AT: &str = "hook tests: one at a time";
            let hold = Hold::new(AT);
            let threads: Vec<_> = (0..2).map(|_| std::thread::spawn(|| point(AT))).collect();
            assert!(hold.wait_reached_times(2, LIMIT));
            assert_eq!(hold.stopped(), 2);
            hold.let_one_go();
            // One goes and one stays, whichever it was.
            assert!(hold.wait_stopped(1, LIMIT));
            assert_eq!(hold.timed_out(), 0);
            hold.let_one_go();
            hold.release();
            for thread in threads {
                thread.join().unwrap();
            }
            assert_eq!(hold.timed_out(), 0);
        }

        #[test]
        fn a_second_hold_of_a_point_waits_for_the_first_to_end() {
            const AT: &str = "hook tests: two holds";
            let first = Hold::new(AT);
            let made = Arc::new(AtomicBool::new(false));
            let second = std::thread::spawn({
                let made = Arc::clone(&made);
                move || {
                    // A bound far past the test's own limit: only the end of
                    // the first hold can let it go in time.
                    let hold = Hold::with_limit(AT, Duration::from_secs(600));
                    made.store(true, Ordering::Release);
                    hold.arrivals()
                }
            });
            // Until the second is seen waiting, with the first alive.
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while lock(&REGISTRY.state).waiting == 0 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the second hold never waited"
                );
                std::thread::yield_now();
            }
            assert!(
                !made.load(Ordering::Acquire),
                "the second hold was made while the first was alive"
            );
            // Ending the first wakes it.
            drop(first);
            assert_eq!(second.join().unwrap(), 0);
            assert!(made.load(Ordering::Acquire));
        }

        #[test]
        #[should_panic(expected = "another hold of the point")]
        fn a_second_hold_that_waits_too_long_fails_and_does_not_hang() {
            const AT: &str = "hook tests: two holds, no end";
            let _first = Hold::new(AT);
            let _second = Hold::with_limit(AT, Duration::ZERO);
        }

        #[test]
        fn a_dropped_hold_releases_what_it_stopped() {
            const AT: &str = "hook tests: dropped";
            let hold = Hold::new(AT);
            let thread = std::thread::spawn(|| point(AT));
            assert!(hold.wait_reached(LIMIT));
            let shared = Arc::clone(&hold.shared);
            drop(hold);
            thread.join().unwrap();
            assert_eq!(
                lock(&shared.state).timed_out,
                0,
                "the drop let it go, not the bound"
            );
            point(AT);
        }

        #[test]
        fn the_points_reached_while_a_hold_lives_are_remembered() {
            let hold = Hold::new("hook tests: the held one");
            point("hook tests: a neighbour");
            assert!(seen().contains(&"hook tests: a neighbour"));
            drop(hold);
        }

        // ------------------------------------------------ an awaited point

        /// A waker that remembers it was woken, and lets a test wait for
        /// that with a bound.
        #[derive(Default)]
        struct Woken {
            woken: Mutex<usize>,
            changed: Condvar,
        }

        impl std::task::Wake for Woken {
            fn wake(self: Arc<Self>) {
                self.wake_by_ref();
            }

            fn wake_by_ref(self: &Arc<Self>) {
                *lock(&self.woken) += 1;
                self.changed.notify_all();
            }
        }

        impl Woken {
            /// Waits until the task has been woken since the last wait, for
            /// at most `limit`. Whether it was.
            fn wait(&self, limit: Duration) -> bool {
                let woken = lock(&self.woken);
                let (mut woken, _) = self
                    .changed
                    .wait_timeout_while(woken, limit, |woken| *woken == 0)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let was = *woken > 0;
                *woken = 0;
                was
            }
        }

        /// A future and the waker its polls are given.
        struct Task<F> {
            future: Pin<Box<F>>,
            woken: Arc<Woken>,
        }

        impl<F: Future> Task<F> {
            fn new(future: F) -> Self {
                Self {
                    future: Box::pin(future),
                    woken: Arc::new(Woken::default()),
                }
            }

            fn poll(&mut self) -> Poll<F::Output> {
                let waker = Waker::from(Arc::clone(&self.woken));
                self.future.as_mut().poll(&mut Context::from_waker(&waker))
            }
        }

        #[test]
        fn an_awaited_point_nobody_holds_is_ready_at_its_first_poll() {
            assert!(
                Task::new(point_async("hook tests: nobody awaits this"))
                    .poll()
                    .is_ready()
            );
            let hold = Hold::new("hook tests: held, and awaited by nobody");
            assert!(
                Task::new(point_async("hook tests: nobody awaits this"))
                    .poll()
                    .is_ready()
            );
            assert_eq!(hold.arrivals(), 0);
            assert!(
                seen().contains(&"hook tests: nobody awaits this"),
                "an awaited point is remembered as reached as any is"
            );
        }

        #[test]
        fn a_task_at_a_held_point_is_pending_and_its_thread_goes_on_until_the_release() {
            const AT: &str = "hook tests: a task stops until released";
            let hold = Hold::new(AT);
            let mut task = Task::new(point_async(AT));
            // Pending, on this thread, which goes on: it is the test.
            assert!(task.poll().is_pending());
            assert!(hold.wait_reached(Duration::ZERO), "the stop is counted");
            assert_eq!(hold.stopped(), 1);
            let here = std::thread::current().name().unwrap_or_default().to_owned();
            assert!(
                stopped().iter().any(|each| each.point == AT
                    && each.thread.contains("a task polled on")
                    && each.thread.contains(&here)),
                "{:?}",
                stopped()
            );
            // Polled again while held, it stays.
            assert!(task.poll().is_pending());
            assert_eq!(hold.arrivals(), 1, "one arrival, polled twice");
            hold.release();
            assert!(task.woken.wait(LIMIT), "the release wakes the task");
            assert!(task.poll().is_ready());
            assert_eq!(hold.stopped(), 0);
            assert_eq!(hold.timed_out(), 0, "the release let it go, not the bound");
            // After a release the point is ready at once.
            assert!(Task::new(point_async(AT)).poll().is_ready());
            assert_eq!(hold.arrivals(), 2);
        }

        #[test]
        fn one_task_is_let_go_and_the_other_stays() {
            const AT: &str = "hook tests: one task at a time";
            let hold = Hold::new(AT);
            let mut tasks = [Task::new(point_async(AT)), Task::new(point_async(AT))];
            for task in &mut tasks {
                assert!(task.poll().is_pending());
            }
            assert_eq!(hold.stopped(), 2);
            hold.let_one_go();
            // Whichever is woken first and polled takes the one permit.
            assert!(
                tasks.iter().any(|task| task.woken.wait(LIMIT)),
                "the permit wakes a task"
            );
            let gone: Vec<bool> = tasks
                .iter_mut()
                .map(|task| task.poll().is_ready())
                .collect();
            assert_eq!(gone.iter().filter(|gone| **gone).count(), 1, "{gone:?}");
            assert_eq!(hold.stopped(), 1);
            let staying = gone.iter().position(|gone| !gone).unwrap_or_default();
            hold.let_one_go();
            assert!(tasks[staying].woken.wait(LIMIT));
            assert!(tasks[staying].poll().is_ready());
            assert_eq!(hold.stopped(), 0);
            assert_eq!(hold.timed_out(), 0);
        }

        /// A task woken for a permit that a thread took first is pending
        /// again, and the next letting go wakes it again.
        #[test]
        fn a_task_woken_for_a_permit_another_took_is_woken_again() {
            const AT: &str = "hook tests: a permit taken by another";
            let hold = Hold::new(AT);
            let mut task = Task::new(point_async(AT));
            assert!(task.poll().is_pending());
            hold.let_one_go();
            assert!(task.woken.wait(LIMIT), "the permit wakes the task");
            // A thread arrives before the task is polled, and takes the
            // permit as it arrives.
            std::thread::spawn(|| point(AT)).join().unwrap();
            assert!(task.poll().is_pending(), "no permit is left for it");
            hold.release();
            assert!(task.woken.wait(LIMIT), "the release wakes it again");
            assert!(task.poll().is_ready());
            assert_eq!(hold.timed_out(), 0);
        }

        #[test]
        fn the_bound_lets_a_task_go_on_and_says_so() {
            const AT: &str = "hook tests: a task's bound";
            let hold = Hold::with_limit(AT, Duration::ZERO);
            assert!(Task::new(point_async(AT)).poll().is_ready());
            assert_eq!(hold.timed_out(), 1, "nobody let it go");
            assert_eq!(hold.stopped(), 0);
        }

        #[test]
        fn the_nth_arrival_of_a_task_is_the_one_that_stops() {
            const AT: &str = "hook tests: a task's second";
            let hold = Hold::new(AT).skipping(1);
            assert!(Task::new(point_async(AT)).poll().is_ready());
            let mut second = Task::new(point_async(AT));
            assert!(second.poll().is_pending());
            assert_eq!((hold.arrivals(), hold.stopped()), (2, 1));
            drop(hold);
            assert!(second.woken.wait(LIMIT), "dropping the hold releases it");
            assert!(second.poll().is_ready());
        }

        #[test]
        fn a_task_dropped_while_it_is_stopped_is_stopped_no_longer() {
            const AT: &str = "hook tests: a dropped task";
            let hold = Hold::new(AT);
            let mut task = Task::new(point_async(AT));
            assert!(task.poll().is_pending());
            assert_eq!(hold.stopped(), 1);
            drop(task);
            assert_eq!(hold.stopped(), 0);
            assert!(hold.wait_stopped(0, Duration::ZERO));
            assert_eq!(hold.timed_out(), 0);
        }

        #[test]
        fn the_awaited_wait_for_a_stop_is_ready_when_a_thread_stops() {
            const AT: &str = "hook tests: an awaited wait";
            let hold = Hold::new(AT);
            let mut reached = Task::new(hold.wait_reached_times_async(1, LIMIT));
            assert!(reached.poll().is_pending(), "nobody arrived yet");
            let thread = std::thread::spawn(|| point(AT));
            assert!(reached.woken.wait(LIMIT), "the stop wakes the wait");
            assert_eq!(reached.poll(), Poll::Ready(true));
            // One that has happened already is ready at once.
            assert_eq!(
                Task::new(hold.wait_reached_times_async(1, LIMIT)).poll(),
                Poll::Ready(true)
            );
            // And one that does not happen ends at its bound.
            assert_eq!(
                Task::new(hold.wait_reached_times_async(2, Duration::ZERO)).poll(),
                Poll::Ready(false)
            );
            hold.release();
            thread.join().unwrap();
            assert_eq!(hold.timed_out(), 0);
        }
    }
}
