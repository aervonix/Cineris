//! Keys as a person types them, as data: each terminal event with the time
//! it was read, for a test to feed an interface that tells typing from a
//! paste, and a key pressed from a key held down, by time
//! (`docs/testing-and-ci.md`, "Testing the TUI").
//!
//! A [`Cadence`] is a timeline built from its start: typing at a steady
//! pace, a pause, one key, keys that arrive in one read, a paste, and a key
//! held down with the terminal's auto-repeat. [`Cadence::strokes`] gives the
//! events in order, each a [`Stroke`] with its time and whether more input
//! was read with it. Nothing waits: the times are the test's, and the test
//! hands each event to the interface with its time, as its loop would.
//!
//! ```ignore
//! let strokes = Cadence::starting(now)
//!     .typing("go on", Duration::from_millis(150))
//!     .wait(Duration::from_secs(2))
//!     .hold(Key::plain(KeyCode::Char('y')), AutoRepeat::X11, 20)
//!     .strokes();
//! for stroke in &strokes {
//!     if stroke.queued && matches!(stroke.event, Event::Key(_)) {
//!         app.burst(stroke.at);
//!     }
//!     app.terminal(&stroke.event, stroke.at);
//! }
//! ```
//!
//! **Time.** The cadence keeps the time of its last input, [`Cadence::now`]
//! (its start before any). [`Cadence::typing`] puts its first key one gap
//! after it, so that two runs of typing follow on at the same pace;
//! [`Cadence::key`], [`Cadence::together`], [`Cadence::paste`] and the press
//! of [`Cadence::hold`] come at it, and [`Cadence::wait`] moves it on. Two
//! events at one time were read together.
//!
//! There is no pseudo-terminal form. A program in a pseudo-terminal reads
//! the time from its own clock, so keys typed into one at a pace need the
//! test to wait between them in real time, which no test does
//! (`AGENTS.md`, rule 12). Typing at a pace is a test of the application
//! with the times given, as here.

use std::time::{Duration, Instant};

use cineris_term::{Event, Key, KeyCode};

/// One terminal event, and the time it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stroke {
    /// When it was read.
    pub at: Instant,
    /// What was read.
    pub event: Event,
    /// More input was read with it: the next stroke is at the same time.
    /// What the terminal's `queued()` says as the interface's loop takes
    /// the event, and why that loop tells the application a key was one of
    /// a burst (`cineris_tui::app::App::burst`).
    pub queued: bool,
}

/// How a terminal repeats a key held down: the press, then after `delay` a
/// repeat every `every`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoRepeat {
    /// From the press to the first repeat.
    pub delay: Duration,
    /// Between one repeat and the next.
    pub every: Duration,
    /// Whether the terminal marks the repeats as repeats
    /// ([`Event::Repeat`]): the enhanced keyboard protocol does for the keys
    /// that type no text, and Windows Terminal's win32 input mode for all.
    /// Unmarked, a repeat is the same [`Event::Key`] again.
    pub marked: bool,
}

impl AutoRepeat {
    /// X11's default: the first repeat after 660 ms, then 25 a second,
    /// unmarked.
    pub const X11: Self = Self::new(Duration::from_millis(660), Duration::from_millis(40));

    /// Windows' default (a keyboard delay of 1 and a speed of 31): the
    /// first repeat after 500 ms, then about 30 a second, marked as win32
    /// input mode marks them.
    pub const WINDOWS: Self =
        Self::new(Duration::from_millis(500), Duration::from_millis(33)).marked();

    /// Repeats that come `delay` after the press, then every `every`,
    /// unmarked.
    #[must_use]
    pub const fn new(delay: Duration, every: Duration) -> Self {
        Self {
            delay,
            every,
            marked: false,
        }
    }

    /// The same, the terminal marking each repeat.
    #[must_use]
    pub const fn marked(self) -> Self {
        Self {
            marked: true,
            ..self
        }
    }
}

/// A timeline of terminal input, from a start. See the
/// [module documentation](self).
#[derive(Debug, Clone)]
pub struct Cadence {
    now: Instant,
    events: Vec<(Instant, Event)>,
}

impl Cadence {
    /// A timeline that starts at `start`, with nothing on it yet.
    #[must_use]
    pub fn starting(start: Instant) -> Self {
        Self {
            now: start,
            events: Vec::new(),
        }
    }

    /// The time of the last input, or the start before any (moved on by
    /// [`wait`](Self::wait)).
    #[must_use]
    pub fn now(&self) -> Instant {
        self.now
    }

    /// `text` typed a character at a time, a key every `every`, the first
    /// `every` after [`now`](Self::now). A character is a plain key of
    /// itself, a line break included.
    #[must_use]
    pub fn typing(mut self, text: &str, every: Duration) -> Self {
        for c in text.chars() {
            self.now += every;
            self.push(Event::Key(Key::plain(KeyCode::Char(c))));
        }
        self
    }

    /// Nothing for `pause`.
    #[must_use]
    pub fn wait(mut self, pause: Duration) -> Self {
        self.now += pause;
        self
    }

    /// One key, at [`now`](Self::now): read with the input before it if no
    /// [`wait`](Self::wait) came between.
    #[must_use]
    pub fn key(mut self, key: Key) -> Self {
        self.push(Event::Key(key));
        self
    }

    /// `text` as keys read together, at [`now`](Self::now): what a terminal
    /// with no bracketed paste sends for a paste, and keys a stall held
    /// back.
    #[must_use]
    pub fn together(mut self, text: &str) -> Self {
        for c in text.chars() {
            self.push(Event::Key(Key::plain(KeyCode::Char(c))));
        }
        self
    }

    /// `text` pasted with bracketed paste, at [`now`](Self::now).
    #[must_use]
    pub fn paste(mut self, text: &str) -> Self {
        self.push(Event::Paste(text.to_owned()));
        self
    }

    /// Any event, at [`now`](Self::now).
    #[must_use]
    pub fn event(mut self, event: Event) -> Self {
        self.push(event);
        self
    }

    /// `key` pressed at [`now`](Self::now) and held down for `repeats`
    /// repeats: the first `repeat.delay` after the press, then one every
    /// `repeat.every`. [`now`](Self::now) is the last repeat after.
    #[must_use]
    pub fn hold(mut self, key: Key, repeat: AutoRepeat, repeats: usize) -> Self {
        self.push(Event::Key(key));
        for n in 0..repeats {
            self.now += if n == 0 { repeat.delay } else { repeat.every };
            self.push(if repeat.marked {
                Event::Repeat(key)
            } else {
                Event::Key(key)
            });
        }
        self
    }

    /// The events, in the order they were read, each with its time and
    /// whether more input was read with it.
    #[must_use]
    pub fn strokes(self) -> Vec<Stroke> {
        let times: Vec<Instant> = self.events.iter().map(|(at, _)| *at).collect();
        self.events
            .into_iter()
            .enumerate()
            .map(|(n, (at, event))| Stroke {
                at,
                event,
                queued: times.get(n + 1) == Some(&at),
            })
            .collect()
    }

    fn push(&mut self, event: Event) {
        self.events.push((self.now, event));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn char_key(c: char) -> Key {
        Key::plain(KeyCode::Char(c))
    }

    /// Each stroke's time from the start, in milliseconds, and its event.
    fn timeline(start: Instant, strokes: &[Stroke]) -> Vec<(u128, Event)> {
        strokes
            .iter()
            .map(|stroke| ((stroke.at - start).as_millis(), stroke.event.clone()))
            .collect()
    }

    #[test]
    fn typing_puts_a_key_every_gap_starting_one_gap_on_and_runs_follow_on() {
        let start = Instant::now();
        let cadence = Cadence::starting(start).typing("ab", ms(80));
        assert_eq!(cadence.now(), start + ms(160));
        let strokes = cadence.typing("c", ms(80)).strokes();
        assert_eq!(
            timeline(start, &strokes),
            [
                (80, Event::Key(char_key('a'))),
                (160, Event::Key(char_key('b'))),
                (240, Event::Key(char_key('c'))),
            ]
        );
        assert!(strokes.iter().all(|stroke| !stroke.queued), "typed apart");
    }

    #[test]
    fn a_key_comes_at_now_and_a_wait_moves_now_on() {
        let start = Instant::now();
        let cadence = Cadence::starting(start)
            .key(char_key('x'))
            .wait(ms(500))
            .key(char_key('y'));
        assert_eq!(cadence.now(), start + ms(500));
        assert_eq!(
            timeline(start, &cadence.strokes()),
            [
                (0, Event::Key(char_key('x'))),
                (500, Event::Key(char_key('y'))),
            ]
        );
    }

    #[test]
    fn keys_read_together_are_queued_but_the_last() {
        let start = Instant::now();
        let strokes = Cadence::starting(start)
            .typing("a", ms(100))
            .wait(ms(100))
            .together("yes")
            .wait(ms(1))
            .paste("p")
            .strokes();
        let queued: Vec<bool> = strokes.iter().map(|stroke| stroke.queued).collect();
        // `a` alone; `y`, `e` and `s` in one read; the paste alone.
        assert_eq!(queued, [false, true, true, false, false]);
        assert_eq!(
            timeline(start, &strokes),
            [
                (100, Event::Key(char_key('a'))),
                (200, Event::Key(char_key('y'))),
                (200, Event::Key(char_key('e'))),
                (200, Event::Key(char_key('s'))),
                (201, Event::Paste("p".to_owned())),
            ]
        );
    }

    #[test]
    fn a_held_key_is_its_press_then_repeats_after_the_delay_at_the_rate() {
        let start = Instant::now();
        let a = char_key('a');
        let cadence = Cadence::starting(start).hold(a, AutoRepeat::new(ms(500), ms(33)), 3);
        assert_eq!(cadence.now(), start + ms(566));
        let strokes = cadence.strokes();
        assert_eq!(
            timeline(start, &strokes),
            [
                (0, Event::Key(a)),
                (500, Event::Key(a)),
                (533, Event::Key(a)),
                (566, Event::Key(a)),
            ]
        );
        assert!(strokes.iter().all(|stroke| !stroke.queued));
        // No repeats: the press alone.
        let pressed = Cadence::starting(start)
            .hold(a, AutoRepeat::X11, 0)
            .strokes();
        assert_eq!(timeline(start, &pressed), [(0, Event::Key(a))]);
    }

    #[test]
    fn a_terminal_that_marks_repeats_marks_all_but_the_press() {
        let start = Instant::now();
        let esc = Key::plain(KeyCode::Esc);
        let strokes = Cadence::starting(start)
            .hold(esc, AutoRepeat::new(ms(300), ms(50)).marked(), 2)
            .strokes();
        assert_eq!(
            timeline(start, &strokes),
            [
                (0, Event::Key(esc)),
                (300, Event::Repeat(esc)),
                (350, Event::Repeat(esc)),
            ]
        );
    }

    #[test]
    fn the_systems_defaults_are_what_they_say() {
        assert_eq!(
            AutoRepeat::X11,
            AutoRepeat {
                delay: ms(660),
                every: ms(40),
                marked: false
            }
        );
        assert_eq!(
            AutoRepeat::WINDOWS,
            AutoRepeat {
                delay: ms(500),
                every: ms(33),
                marked: true
            }
        );
    }

    #[test]
    fn any_event_comes_at_now() {
        let start = Instant::now();
        let strokes = Cadence::starting(start)
            .wait(ms(7))
            .event(Event::Focus(false))
            .strokes();
        assert_eq!(timeline(start, &strokes), [(7, Event::Focus(false))]);
    }
}
