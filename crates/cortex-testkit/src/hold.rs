//! A hold on a point of library code: the test's side of
//! `cortex_base::hook` (`docs/testing-and-ci.md`, "A point the library
//! stops at").
//!
//! Library code marks a place with `cortex_base::hook::point("…")`. A test
//! that needs the library to stop there (between the read and the write,
//! between the new content and the rename) holds the point, waits until a
//! thread of the library has stopped at it, does what it came to do, and
//! lets the thread go:
//!
//! ```ignore
//! let hold = Hold::new(cortex_base::atomic::BEFORE_THE_RENAME);
//! let writer = std::thread::spawn(move || write_atomic(…));
//! hold.reach();                       // the writer is stopped at the point
//! // … read the file while the write is half done …
//! hold.let_one_go();
//! writer.join().unwrap().unwrap();
//! ```
//!
//! This is the base crate's [`Hold`](cortex_base::hook::Hold) with what only
//! the test kit can add: **every wait is named in the timeout report**
//! ([`watch`](crate::watch)), the hold by where the test made it, and the
//! library's own stopped threads by their point; and [`reach`](Hold::reach),
//! which fails the test with a message that says which points the library
//! did reach when the one named never came (a misspelled name shows up
//! there). Everything else (`release`, `let_one_go`, `arrivals`, `stopped`,
//! `timed_out`) is the base crate's, reached through `Deref`.
//!
//! A point that an async task reaches is marked with
//! `cortex_base::hook::await_point!(…)`, and a test whose library runs
//! on the test's own thread (the core's tests run everything on one
//! current-thread runtime) cannot wait for it with [`reach`](Hold::reach):
//! that blocks the only thread that could get there. It polls the
//! operation to the point instead, with
//! [`drive_until_reached`](Hold::drive_until_reached):
//!
//! ```ignore
//! let hold = Hold::new(cortex_core::BEFORE_THE_NEW_LEVEL_IS_IN_FORCE);
//! let mut level = std::pin::pin!(front.op(Op::SessionSet(…)));
//! hold.drive_until_reached(level.as_mut()).await; // the change is pending there
//! // … do what must come in between, on the same thread …
//! hold.release();
//! level.await?;
//! ```
//!
//! The points are in the build only when the `test-hooks` feature of
//! `cortex-base` is on, which this crate turns on: a crate that has the test
//! kit as a dev-dependency has them in its test builds, and in no other.

use std::future::Future;
use std::ops::Deref;
use std::panic::Location;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

pub use cortex_base::hook::{Stopped, seen, stopped};

/// How long [`Hold::reach`] waits for the library to arrive. A bound only:
/// the library arrives at once or never, and it is never waited for in real
/// time on the way to a passing test.
const REACH: Duration = Duration::from_secs(5);

/// A test's hold on a point of library code. See the [module](self).
#[must_use = "a hold that is dropped at once stops nothing for long"]
#[derive(Debug)]
pub struct Hold {
    inner: cortex_base::hook::Hold,
    made_at: &'static Location<'static>,
}

impl Deref for Hold {
    type Target = cortex_base::hook::Hold;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Hold {
    /// Holds `point`, a name the library exports as a constant. A thread
    /// stops there for at most
    /// [`DEFAULT_LIMIT`](cortex_base::hook::Hold::DEFAULT_LIMIT) unless the
    /// test lets it go first.
    #[track_caller]
    pub fn new(point: &str) -> Self {
        Self {
            made_at: Location::caller(),
            inner: cortex_base::hook::Hold::new(point),
        }
    }

    /// Lets the first `arrivals` arrivals through, and holds the ones after:
    /// the n-th arrival is the one that stops.
    pub fn skipping(self, arrivals: usize) -> Self {
        Self {
            inner: self.inner.skipping(arrivals),
            made_at: self.made_at,
        }
    }

    /// "the hold of the point `…` made at <file>:<line>:<column>".
    #[must_use]
    pub fn name(&self) -> String {
        format!(
            "the hold of the point `{}` made at {}",
            self.inner.point(),
            self.made_at
        )
    }

    /// Waits until a thread of the library has stopped at the point, for at
    /// most `limit`. Whether one has.
    #[must_use]
    pub fn wait_reached(&self, limit: Duration) -> bool {
        let _doing = crate::watch::doing(format!(
            "waits for the library to reach {}, up to {limit:?}",
            self.name()
        ));
        self.inner.wait_reached(limit)
    }

    /// Waits until `stops` arrivals have stopped here, for at most `limit`.
    /// Whether they have.
    #[must_use]
    pub fn wait_reached_times(&self, stops: usize, limit: Duration) -> bool {
        let _doing = crate::watch::doing(format!(
            "waits for the library to reach {} {stops} time(s), up to {limit:?}",
            self.name()
        ));
        self.inner.wait_reached_times(stops, limit)
    }

    /// Waits until exactly `stopped` threads are stopped here, for at most
    /// `limit`: until a thread that was let go has gone on. Whether that is
    /// so.
    #[must_use]
    pub fn wait_stopped(&self, stopped: usize, limit: Duration) -> bool {
        let _doing = crate::watch::doing(format!(
            "waits for {stopped} thread(s) to be stopped at {}, up to {limit:?}",
            self.name()
        ));
        self.inner.wait_stopped(stopped, limit)
    }

    /// Waits until a thread of the library has stopped at the point, and
    /// fails the test, with what the library did reach, when none does.
    ///
    /// # Panics
    ///
    /// When no thread arrives within a few seconds, which is a bound and not
    /// a wait: the library arrives at once or never.
    pub fn reach(&self) {
        self.reach_times(1);
    }

    /// [`reach`](Self::reach) for the `stops`-th stop.
    ///
    /// # Panics
    ///
    /// As [`reach`](Self::reach).
    pub fn reach_times(&self, stops: usize) {
        self.reach_within(stops, REACH);
    }

    fn reach_within(&self, stops: usize, limit: Duration) {
        assert!(
            self.wait_reached_times(stops, limit),
            "{}",
            self.not_reached(stops, limit)
        );
    }

    /// Polls `operation`, a future of the library that the test drives on
    /// its own thread, until it has stopped at the point (an awaited point,
    /// `cortex_base::hook::await_point!`), and leaves it pending there for
    /// the test to go on with: [`reach`](Self::reach) for a library that
    /// runs on the test's own runtime, where a blocking wait would stop the
    /// only thread that could get there. Between polls the runtime runs its
    /// other tasks.
    ///
    /// # Panics
    ///
    /// When the operation finishes without stopping there, and when no stop
    /// comes within a few seconds (a bound, not a wait), with what the
    /// library did reach.
    pub async fn drive_until_reached<F: Future>(&self, operation: Pin<&mut F>) {
        self.drive_within(1, operation, REACH).await;
    }

    /// [`drive_until_reached`](Self::drive_until_reached) for the
    /// `stops`-th stop.
    ///
    /// # Panics
    ///
    /// As [`drive_until_reached`](Self::drive_until_reached).
    pub async fn drive_until_reached_times<F: Future>(&self, stops: usize, operation: Pin<&mut F>) {
        self.drive_within(stops, operation, REACH).await;
    }

    async fn drive_within<F: Future>(
        &self,
        stops: usize,
        mut operation: Pin<&mut F>,
        limit: Duration,
    ) {
        let _doing = crate::watch::doing(format!(
            "drives the library to {} {stops} time(s), up to {limit:?}",
            self.name()
        ));
        let mut reached = std::pin::pin!(self.inner.wait_reached_times_async(stops, limit));
        let reached = std::future::poll_fn(|cx| {
            // The operation first: a stop it makes in this poll is seen
            // by the look after it.
            if operation.as_mut().poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            reached.as_mut().poll(cx).map(Some)
        })
        .await;
        match reached {
            Some(true) => {}
            Some(false) => panic!("{}", self.not_reached(stops, limit)),
            None => panic!(
                "the operation finished without stopping at the point `{}` {stops} time(s): {} \
                 arrival(s) (the ones skipped included), {} let go by the bound. The points the \
                 library did reach while a hold was alive: {:?}.",
                self.inner.point(),
                self.inner.arrivals(),
                self.inner.timed_out(),
                seen(),
            ),
        }
    }

    /// What a test whose point never came is told.
    fn not_reached(&self, stops: usize, limit: Duration) -> String {
        format!(
            "no thread of the library stopped at the point `{}` {stops} time(s) within {limit:?}: \
             {} arrival(s) so far (the ones skipped included), {} stopped, {} let go by the bound.\n\
             The points the library did reach while a hold was alive: {:?}.\n\
             A point is named by a constant of the module that marks it; a literal that differs \
             by a character is a point nobody reaches.",
            self.inner.point(),
            self.inner.arrivals(),
            self.inner.stopped(),
            self.inner.timed_out(),
            seen(),
        )
    }
}

#[cfg(test)]
mod tests {
    use cortex_base::hook::{point, point_async};

    use super::*;

    const AT: &str = "testkit hold tests: the point";

    #[test]
    fn a_hold_stops_a_thread_of_the_library_until_the_test_lets_it_go() {
        let hold = Hold::new(AT);
        let past = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread = {
            let past = std::sync::Arc::clone(&past);
            std::thread::spawn(move || {
                point(AT);
                past.store(true, std::sync::atomic::Ordering::SeqCst);
            })
        };
        hold.reach();
        assert!(!past.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(hold.stopped(), 1, "through Deref, the base crate's own");
        hold.let_one_go();
        thread.join().unwrap();
        assert!(past.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(hold.timed_out(), 0);
    }

    #[test]
    fn a_hold_is_known_by_where_it_was_made() {
        let (line, hold) = (line!(), Hold::new("testkit hold tests: named"));
        let name = hold.name();
        assert!(name.contains("testkit hold tests: named"), "{name}");
        assert!(name.contains("hold.rs"), "{name}");
        assert!(name.contains(&format!(":{line}:")), "{name}");
    }

    #[test]
    fn the_arrival_that_stops_is_chosen_by_skipping() {
        let hold = Hold::new("testkit hold tests: nth").skipping(1);
        point("testkit hold tests: nth");
        assert_eq!(hold.stopped(), 0, "the first passed");
        let second = std::thread::spawn(|| point("testkit hold tests: nth"));
        hold.reach();
        hold.release();
        second.join().unwrap();
    }

    #[test]
    fn a_point_nobody_reaches_fails_with_the_points_that_were_reached() {
        let hold = Hold::new("testkit hold tests: misspelled");
        point("testkit hold tests: a neighbour");
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hold.reach_within(1, Duration::ZERO);
        }))
        .expect_err("nobody reached the point");
        let message = failure
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(
            message.contains("testkit hold tests: misspelled"),
            "{message}"
        );
        assert!(message.contains("0 arrival(s)"), "{message}");
        assert!(
            message.contains("testkit hold tests: a neighbour"),
            "{message}"
        );
    }

    #[test]
    fn the_report_of_a_test_that_runs_out_of_time_names_a_thread_stopped_at_a_point() {
        const STOPPED: &str = "testkit hold tests: reported";
        let hold = Hold::new(STOPPED);
        let thread = std::thread::Builder::new()
            .name("a library thread".to_owned())
            .spawn(|| point(STOPPED))
            .unwrap();
        hold.reach();
        let waits = crate::watch::waits();
        assert!(
            waits.contains("a library thread")
                && waits.contains("stopped at the library point")
                && waits.contains(STOPPED),
            "{waits}"
        );
        hold.release();
        thread.join().unwrap();
        let waits = crate::watch::waits();
        assert!(!waits.contains(STOPPED), "{waits}");
    }

    /// Runs `future` on this thread to its end, waking by the thread's
    /// park. A test kit with no runtime of its own needs no more.
    fn block_on<F: Future>(future: F) -> F::Output {
        struct Unpark(std::thread::Thread);
        impl std::task::Wake for Unpark {
            fn wake(self: std::sync::Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(std::sync::Arc::new(Unpark(std::thread::current())));
        let mut context = std::task::Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        let deadline = std::time::Instant::now() + REACH;
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "the future was never woken");
            std::thread::park_timeout(left);
        }
    }

    fn panic_message(failure: &(dyn std::any::Any + Send)) -> String {
        failure
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                failure
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
            })
            .unwrap_or_default()
    }

    #[test]
    fn an_operation_on_the_tests_own_thread_is_driven_to_an_awaited_point_and_left_there() {
        const AWAITED: &str = "testkit hold tests: awaited";
        let hold = Hold::new(AWAITED);
        let steps = std::sync::Mutex::new(Vec::new());
        let operation = async {
            steps.lock().unwrap().push("before");
            // As library code marks it.
            cortex_base::hook::await_point!(AWAITED);
            steps.lock().unwrap().push("after");
            7
        };
        let answer = block_on(async {
            let mut operation = std::pin::pin!(operation);
            hold.drive_until_reached(operation.as_mut()).await;
            assert_eq!(*steps.lock().unwrap(), ["before"], "stopped at the point");
            assert_eq!(hold.stopped(), 1);
            hold.release();
            operation.await
        });
        assert_eq!(answer, 7);
        assert_eq!(*steps.lock().unwrap(), ["before", "after"]);
        assert_eq!(hold.timed_out(), 0, "the release let it go, not the bound");
    }

    #[test]
    fn the_second_stop_is_driven_to_after_the_first_is_let_go() {
        const AWAITED: &str = "testkit hold tests: awaited twice";
        let hold = Hold::new(AWAITED);
        let operation = async {
            point_async(AWAITED).await;
            point_async(AWAITED).await;
        };
        block_on(async {
            let mut operation = std::pin::pin!(operation);
            hold.drive_until_reached(operation.as_mut()).await;
            hold.let_one_go();
            hold.drive_until_reached_times(2, operation.as_mut()).await;
            assert_eq!((hold.arrivals(), hold.stopped()), (2, 1));
            hold.release();
            operation.await;
        });
        assert_eq!(hold.timed_out(), 0);
    }

    #[test]
    fn an_operation_that_ends_without_stopping_fails_the_drive_and_says_so() {
        let hold = Hold::new("testkit hold tests: never awaited");
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            block_on(async {
                let mut operation = std::pin::pin!(async {
                    point_async("testkit hold tests: awaited instead").await;
                });
                hold.drive_until_reached(operation.as_mut()).await;
            });
        }))
        .expect_err("the operation never stopped");
        let message = panic_message(failure.as_ref());
        assert!(message.contains("finished without stopping"), "{message}");
        assert!(
            message.contains("testkit hold tests: never awaited")
                && message.contains("testkit hold tests: awaited instead"),
            "{message}"
        );
    }

    #[test]
    fn a_drive_that_reaches_nothing_within_its_bound_fails_with_what_was_reached() {
        let hold = Hold::new("testkit hold tests: not within the bound");
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            block_on(async {
                let mut operation = std::pin::pin!(std::future::pending::<()>());
                hold.drive_within(1, operation.as_mut(), Duration::ZERO)
                    .await;
            });
        }))
        .expect_err("nothing stopped");
        let message = panic_message(failure.as_ref());
        assert!(
            message.contains("no thread of the library stopped"),
            "{message}"
        );
        assert!(message.contains("0 arrival(s)"), "{message}");
    }
}
