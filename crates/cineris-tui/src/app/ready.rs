//! When something that takes keys of its own is ready to take them: the
//! dialog's rule (`docs/architecture/09-tui.md`, "The approval dialog";
//! `docs/steps/23/answered.md`), kept in one place so that the dialogs and
//! the panels (step 24) hold to the same one.
//!
//! A [`Readiness`] is three times and a flag:
//!
//! - **The grace:** it is ready no sooner than [`GRACE`] after it was first
//!   drawn ([`Readiness::drawn`]).
//! - **The quiet:** and no sooner than [`QUIET`] after the last input that
//!   it did not take ([`Readiness::typed`]): typing that runs on past its
//!   arrival never reaches it, however slow, and a person who stopped to
//!   read it presses one key.
//! - **Not before:** and no sooner than a time its owner names
//!   ([`Readiness::not_before`]): a dialog after one a key answered waits
//!   [`REPEAT`] after that key.
//! - **What it showed:** a dialog takes keys only once a frame has drawn
//!   its answers ([`Readiness::takes_keys`]); input it did not take hides
//!   them again. A panel, whose keys the hint row shows from its first
//!   frame, asks only whether it is ready ([`Readiness::ready`]).
//!
//! [`Readiness::hide`] starts it again as if just shown: after the viewer,
//! after the program was continued (`fg`), after the external editor, and
//! whenever its owner says what it showed is no longer what it shows.
//! [`Readiness::again`] starts only its grace again, keeping the quiet it
//! owes: a first-run step whose rows the worker's answer changed.
//!
//! All of these are comparisons of times: the one wake they need is the
//! moment it is ready, when its owner has something to draw then.

use std::time::{Duration, Instant};

/// How long after it is first drawn, and after the key before, a key still
/// goes to the editor.
pub(crate) const GRACE: Duration = Duration::from_millis(300);

/// How long with no input it did not take it waits before it takes keys
/// (the owner's "about 1 s", after the review of step 23). Even a slow
/// typist's gaps inside a sentence are shorter: 40 words a minute is a key
/// every 300 ms, 15 a minute one every 800 ms. And a second is short beside
/// the time it takes to read a dialog, so a person who stopped typing to
/// read it does not wait for it.
pub(crate) const QUIET: Duration = Duration::from_millis(1000);

/// How long after a key the same key again is taken for that key held
/// down, where the terminal does not say: terminals repeat a held key
/// after 250 ms to 1 s (X11's default is 660 ms, Windows' and macOS's
/// about 500 ms), then every 25 to 50 ms.
pub(crate) const REPEAT: Duration = Duration::from_millis(1000);

// The quiet covers a key held down where the terminal does not mark its
// repeats: each repeat holds it for longer than the gap to the next.
const _: () = assert!(QUIET.as_millis() >= REPEAT.as_millis());

/// When it is ready. See the [module documentation](self).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Readiness {
    /// When its first frame was written.
    shown: Option<Instant>,
    /// When input it did not take was last read: it is ready [`QUIET`]
    /// after.
    typed: Option<Instant>,
    /// The earliest it can be ready.
    not_before: Option<Instant>,
    /// A frame with its answers has been drawn, and nothing hid them since.
    answers_drawn: bool,
}

impl Readiness {
    /// When its first frame was written, if it has been.
    pub(crate) fn shown(&self) -> Option<Instant> {
        self.shown
    }

    /// A frame of it was written at `at`. Says whether it was the first,
    /// which starts the grace.
    pub(crate) fn drawn(&mut self, at: Instant) -> bool {
        if self.shown.is_some() {
            return false;
        }
        self.shown = Some(at);
        true
    }

    /// Input it did not take was read at `at`: it is ready only [`QUIET`]
    /// after, and answers it showed are hidden. Says whether answers were
    /// hidden, so that its owner draws them again.
    pub(crate) fn typed(&mut self, at: Instant) -> bool {
        self.typed = Some(self.typed.map_or(at, |typed| typed.max(at)));
        self.hide_answers()
    }

    /// It is ready no earlier than `at`.
    pub(crate) fn not_before(&mut self, at: Instant) {
        self.not_before = Some(self.not_before.map_or(at, |before| before.max(at)));
    }

    /// It starts again as if just shown: its grace begins with its next
    /// frame, and answers it showed are hidden until then. Says whether
    /// answers were hidden.
    pub(crate) fn hide(&mut self) -> bool {
        self.shown = None;
        self.typed = None;
        self.hide_answers()
    }

    /// Its grace starts again with its next frame, and answers it showed
    /// are hidden until then, but the quiet it owes input it did not take
    /// is kept: what it shows changed under it (the first run's rows, when
    /// the worker's answer comes; since the full review of step 25's third
    /// pull request). Says whether answers were hidden.
    pub(crate) fn again(&mut self) -> bool {
        self.shown = None;
        self.hide_answers()
    }

    /// The answers are hidden; whether they were shown.
    fn hide_answers(&mut self) -> bool {
        std::mem::take(&mut self.answers_drawn)
    }

    /// When it is ready: the end of its grace, [`QUIET`] after input it did
    /// not take, and no sooner than [`not_before`](Self::not_before) said.
    /// `None` until its first frame.
    pub(crate) fn ready_at(&self) -> Option<Instant> {
        let shown = self.shown?;
        let ready = (shown + GRACE).max(self.not_before.unwrap_or(shown));
        Some(self.typed.map_or(ready, |typed| ready.max(typed + QUIET)))
    }

    /// Whether it is ready at `at`.
    pub(crate) fn ready(&self, at: Instant) -> bool {
        self.ready_at().is_some_and(|ready| at >= ready)
    }

    /// Whether a frame has drawn its answers since they were last hidden.
    pub(crate) fn answers_drawn(&self) -> bool {
        self.answers_drawn
    }

    /// A frame drew its answers, or did not.
    pub(crate) fn set_answers_drawn(&mut self, drawn: bool) {
        self.answers_drawn = drawn;
    }

    /// Whether it takes keys at `at`: it is ready, and a frame has drawn
    /// its answers.
    pub(crate) fn takes_keys(&self, at: Instant) -> bool {
        self.answers_drawn && self.ready(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_ready_after_its_grace_the_quiet_and_the_time_named() {
        let now = Instant::now();
        let mut ready = Readiness::default();
        assert_eq!(ready.ready_at(), None, "not before its first frame");
        assert!(ready.drawn(now));
        assert!(!ready.drawn(now + GRACE), "the first frame only");
        assert_eq!(ready.ready_at(), Some(now + GRACE));
        assert!(!ready.ready(now + GRACE / 2));
        assert!(ready.ready(now + GRACE));
        // Typing it did not take holds it a quiet second after.
        ready.typed(now + GRACE);
        assert_eq!(ready.ready_at(), Some(now + GRACE + QUIET));
        // An earlier time named changes nothing; a later one holds it.
        ready.not_before(now);
        assert_eq!(ready.ready_at(), Some(now + GRACE + QUIET));
        ready.not_before(now + GRACE * 10);
        assert_eq!(ready.ready_at(), Some(now + GRACE * 10));
    }

    #[test]
    fn hiding_starts_it_again_and_input_hides_what_it_showed() {
        let now = Instant::now();
        let mut ready = Readiness::default();
        ready.drawn(now);
        ready.set_answers_drawn(true);
        assert!(ready.takes_keys(now + GRACE));
        assert!(ready.typed(now + GRACE), "the answers were hidden");
        assert!(!ready.takes_keys(now + GRACE + QUIET), "until drawn again");
        ready.set_answers_drawn(true);
        assert!(ready.takes_keys(now + GRACE + QUIET));
        assert!(ready.hide());
        assert_eq!(ready.shown(), None);
        assert!(!ready.ready(now + GRACE * 100), "until its next frame");
        ready.drawn(now + GRACE * 100);
        assert!(
            ready.ready(now + GRACE * 101),
            "its grace again, not its quiet"
        );
    }

    #[test]
    fn again_starts_its_grace_again_and_keeps_its_quiet() {
        let now = Instant::now();
        let mut ready = Readiness::default();
        ready.drawn(now);
        ready.set_answers_drawn(true);
        ready.typed(now + GRACE);
        ready.set_answers_drawn(true);
        assert!(ready.again(), "the answers were hidden");
        assert_eq!(ready.shown(), None);
        ready.drawn(now + GRACE * 2);
        assert_eq!(
            ready.ready_at(),
            Some(now + GRACE + QUIET),
            "the quiet is kept"
        );
        ready.again();
        ready.drawn(now + GRACE * 10);
        assert_eq!(ready.ready_at(), Some(now + GRACE * 11), "its grace again");
    }
}
