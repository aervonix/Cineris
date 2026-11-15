//! Something the start-up path must not touch, watched
//! (`docs/performance-budgets.md`, "Start-up": nothing on it reads the
//! repository index or the statistics).
//!
//! The part that must wait (the repository index, the statistics) is given
//! a hook that calls [`Untouched::touch`]. The test holds it while the
//! session opens:
//!
//! - **a touch on the path that opens the session** waits there, so the
//!   session does not open until the hold's limit has passed, and that
//!   touch is counted as [`early`](Untouched::early);
//! - **a touch after it** (the background work starting) waits until the
//!   test lets go with [`release`](Untouched::release), and is not early.
//!
//! So a test opens the session, lets go, waits for the first touch, and
//! asserts that none was early. Nothing is asserted about a gate another
//! thread opens (`docs/testing-and-ci.md`, "No real waiting in tests"):
//! the limit only turns a regression into a failure instead of a hang.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::Gate;

/// How long a touch waits for the test to let go before it goes on and
/// counts as early: under the per-test limit, so the test fails on its
/// assertion.
pub const HOLD: Duration = Duration::from_secs(4);

/// A watched part of the harness. See the module's description.
#[derive(Debug, Default)]
pub struct Untouched {
    released: Gate,
    touched: Gate,
    touches: AtomicUsize,
    early: AtomicUsize,
}

impl Untouched {
    /// A watch that holds every touch until [`release`](Self::release).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What the watched part calls each time it is used. Waits until the
    /// test lets go, or [`HOLD`] has passed.
    pub fn touch(&self) {
        self.touches.fetch_add(1, Ordering::SeqCst);
        self.touched.open();
        if !self.released.wait(HOLD) {
            self.early.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Lets every touch through, the waiting ones and every later one.
    pub fn release(&self) {
        self.released.open();
    }

    /// Waits up to `limit` for the first touch. Whether it came.
    #[must_use]
    pub fn wait_touched(&self, limit: Duration) -> bool {
        self.touched.wait(limit)
    }

    /// How many touches there have been.
    #[must_use]
    pub fn touches(&self) -> usize {
        self.touches.load(Ordering::SeqCst)
    }

    /// Touches that went on only because the hold's limit passed: they came
    /// before the test let go, on the path the test held.
    #[must_use]
    pub fn early(&self) -> usize {
        self.early.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_touch_after_the_release_is_not_early() {
        let watch = std::sync::Arc::new(Untouched::new());
        let toucher = {
            let watch = std::sync::Arc::clone(&watch);
            std::thread::spawn(move || watch.touch())
        };
        assert!(watch.wait_touched(Duration::from_secs(5)));
        watch.release();
        toucher.join().unwrap();
        assert_eq!((watch.touches(), watch.early()), (1, 0));
        watch.touch();
        assert_eq!((watch.touches(), watch.early()), (2, 0));
    }
}
