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
//! The points are in the build only when the `test-hooks` feature of
//! `cortex-base` is on, which this crate turns on: a crate that has the test
//! kit as a dev-dependency has them in its test builds, and in no other.

use std::ops::Deref;
use std::panic::Location;
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
        );
    }
}

#[cfg(test)]
mod tests {
    use cortex_base::hook::point;

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
}
