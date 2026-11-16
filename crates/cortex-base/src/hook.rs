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
//! # With the feature off, a point is nothing
//!
//! [`point`] is an empty `const fn` marked `#[inline(always)]`. A build
//! without the `test-hooks` feature (every build of the product: the
//! released binary, `cargo build -p cortex-cli`) has no code for it, no
//! symbol, no data, not even the load of a flag. [`Hold`] and everything
//! else test-side do not exist there. Only a dev-dependency turns the
//! feature on, and `cargo xtask rules` fails when anything else does
//! (`docs/testing-and-ci.md`, "A seeded order of events").
//!
//! # With the feature on and nothing held, a point is one load
//!
//! A test build has the feature on, for every test and for the binaries the
//! system tests start (features are unified across a build of the whole
//! workspace's tests). A point that no [`Hold`] names passes at once: it
//! loads one counter, the number of holds alive in the process, and
//! returns when it is zero. That is the case in the product's code under
//! every test that holds nothing, and in every system test.
//!
//! # What a [`Hold`] does
//!
//! - **It names a point and stops every arrival there.** A thread that
//!   arrives waits until the test lets it go, or until the hold's bound
//!   (5 s unless [`Hold::with_limit`] says otherwise) has passed. After
//!   the bound the thread goes on and the hold remembers it
//!   ([`timed_out`](Hold::timed_out)): a mistake in a test is a failed
//!   test, never a hung one. Every wait of the test's side takes a bound
//!   too.
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
pub use enabled::{Hold, Stopped, point, seen, stopped};
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

#[cfg(feature = "test-hooks")]
mod enabled {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread::ThreadId;
    use std::time::{Duration, Instant};

    use crate::sync::lock;

    /// How many holds are alive in this process. A point that sees none
    /// does nothing else.
    static LIVE: AtomicUsize = AtomicUsize::new(0);

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
        let hold = {
            let mut registry = lock(&REGISTRY.state);
            if !registry.seen.contains(&name) {
                registry.seen.push(name);
            }
            registry
                .holds
                .iter()
                .find(|hold| hold.point == name)
                .cloned()
        };
        if let Some(hold) = hold {
            hold.arrive();
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

    /// A thread that is stopped at a point now.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Stopped {
        /// The point.
        pub point: String,
        /// The thread's name, or a stand-in for one with none.
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
        /// The stopped threads, longest first.
        waiting: Vec<Waiting>,
        /// Threads that may go on, one each.
        permits: usize,
        /// Everything goes on, now and from here on.
        released: bool,
        /// Threads that went on because the bound ran out.
        timed_out: usize,
    }

    struct Waiting {
        id: ThreadId,
        name: String,
        since: Instant,
    }

    impl Shared {
        fn arrive(&self) {
            let mut state = lock(&self.state);
            state.arrivals += 1;
            if state.released || state.arrivals <= state.skip {
                return;
            }
            state.stops += 1;
            let thread = std::thread::current();
            state.waiting.push(Waiting {
                id: thread.id(),
                name: thread.name().unwrap_or("an unnamed thread").to_owned(),
                since: Instant::now(),
            });
            self.changed.notify_all();
            let (mut state, _) = self
                .changed
                .wait_timeout_while(state, self.limit, |state| {
                    !state.released && state.permits == 0
                })
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !state.released {
                if state.permits > 0 {
                    state.permits -= 1;
                } else {
                    state.timed_out += 1;
                }
            }
            let id = thread.id();
            if let Some(index) = state.waiting.iter().position(|waiting| waiting.id == id) {
                state.waiting.remove(index);
            }
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
    }
}
