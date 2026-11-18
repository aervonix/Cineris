//! Where a measurement test's time goes, in named phases, always
//! (`docs/testing-and-ci.md`, "Where a measurement's time goes").
//!
//! A `perf_` test measures something, and what it measured is rarely where
//! the time went: a tree is built, a cache is warmed, a pass is run fifteen
//! times, and the number a budget is held to is one of those. [`Phases`]
//! names each stretch of the test and prints a breakdown when the test is
//! done, so that the first run, on any machine, shows which stretch it was.
//! Unlike [`profile`](crate::profile), which is opt-in and counts the test
//! kit's own waits, it is always on: it costs two clock readings and a push
//! to a list for each phase, and a test that uses it is a measurement.
//!
//! ```text
//! let phases = Phases::new("perf_a_cold_pass");
//! let tree = phases.measure("set-up", || generate(files));
//! {
//!     let _pass = phases.phase("cold pass");
//!     index.pass();
//! }
//! phases.mark("warm passes"); // ends the phase `mark` began last
//! for _ in 0..15 { warm_pass(); }
//! phases.mark("tear-down");
//! drop(tree);
//! // The breakdown is printed when `phases` is dropped, or by `report()`.
//! ```
//!
//! ```text
//! phases of `perf_a_cold_pass`: 1,873.4 ms in all
//!   phase                         took   share  times
//!   set-up                    312.0 ms   16.7%
//!   cold pass               1,201.5 ms   64.1%
//!   warm passes               341.2 ms   18.2%
//!   tear-down                  15.9 ms    0.8%
//!   outside every phase         2.8 ms    0.1%
//! ```
//!
//! - **Phases of one name are added up**, in the order the names first
//!   appeared, with how many times each began when more than once.
//! - **[`phase`](Phases::phase)** is a guard: the phase is the guard's life.
//!   **[`mark`](Phases::mark)** is a boundary: the phase it began lasts until
//!   the next `mark`, [`end`](Phases::end), or the report. The two can be
//!   mixed; a `mark` phase does not stop a guard's phase from counting, so
//!   a phase inside a phase is counted twice and the shares add up to more
//!   than the whole. Phases are for stretches that follow one another.
//! - **The breakdown is printed when the `Phases` is dropped,** unless
//!   [`report`](Phases::report) printed it already, so that a test which
//!   fails half-way still says where its time had gone. It goes to standard
//!   output, which nextest shows for a failing test and, with
//!   `--no-capture`, for a passing one.
//! - **With `CINERIS_TESTKIT_PROFILE=1`** each phase is also a phase of the
//!   test's [`profile`](crate::profile), beside the waits of the test kit.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cineris_base::{Clock, SystemClock};

use crate::profile;

/// What has been measured so far.
#[derive(Debug, Default)]
struct Book {
    /// Each phase by name, in the order the names first appeared, with the
    /// time it has taken and how many times it began.
    phases: Vec<(String, Duration, u32)>,
    /// The phase `mark` began, when it has not ended.
    marked: Option<(String, Duration, profile::Mark)>,
    printed: bool,
}

/// The named phases of one test.
#[derive(Debug)]
pub struct Phases {
    test: String,
    clock: Arc<dyn Clock>,
    started: Duration,
    book: Mutex<Book>,
}

/// A phase that ends when this is dropped ([`Phases::phase`]).
#[derive(Debug)]
#[must_use = "the phase ends when this is dropped"]
pub struct Phase<'a> {
    phases: &'a Phases,
    name: String,
    started: Duration,
    _profile: profile::Mark,
}

impl Phases {
    /// Starts the clock for the test called `test`.
    #[must_use]
    pub fn new(test: &str) -> Self {
        Self::with_clock(test, Arc::new(SystemClock::new()))
    }

    /// The same, on another clock: a fake one, to test the breakdown
    /// without waiting.
    #[must_use]
    pub fn with_clock(test: &str, clock: Arc<dyn Clock>) -> Self {
        let started = clock.monotonic();
        Self {
            test: test.to_owned(),
            clock,
            started,
            book: Mutex::new(Book::default()),
        }
    }

    /// A phase called `name`, until the guard is dropped.
    pub fn phase(&self, name: &str) -> Phase<'_> {
        Phase {
            phases: self,
            name: name.to_owned(),
            started: self.clock.monotonic(),
            _profile: profile::phase(name),
        }
    }

    /// Runs `work` as a phase called `name`, and returns what it returns.
    pub fn measure<T>(&self, name: &str, work: impl FnOnce() -> T) -> T {
        let _phase = self.phase(name);
        work()
    }

    /// Ends the phase the last `mark` began, and begins another called
    /// `name`: it lasts until the next `mark`, [`end`](Self::end) or the
    /// report.
    pub fn mark(&self, name: &str) {
        let now = self.clock.monotonic();
        let mut book = self.lock();
        Self::close(&mut book, now);
        book.marked = Some((name.to_owned(), now, profile::phase(name)));
    }

    /// Ends the phase the last `mark` began, and begins none.
    pub fn end(&self) {
        let now = self.clock.monotonic();
        Self::close(&mut self.lock(), now);
    }

    /// The breakdown as text. It ends the phase `mark` began, and is
    /// printed with it.
    pub fn report(&self) -> String {
        let now = self.clock.monotonic();
        let mut book = self.lock();
        Self::close(&mut book, now);
        book.printed = true;
        let text = self.render(&book, now);
        drop(book);
        show(&text);
        text
    }

    /// The time the phase called `name` has taken so far, over all its
    /// times: for a test that holds a phase to a bound of its own.
    #[must_use]
    pub fn took(&self, name: &str) -> Option<Duration> {
        self.lock()
            .phases
            .iter()
            .find(|(phase, _, _)| phase == name)
            .map(|(_, took, _)| *took)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn add(&self, name: &str, took: Duration) {
        Self::record(&mut self.lock(), name, took);
    }

    fn record(book: &mut Book, name: &str, took: Duration) {
        match book.phases.iter_mut().find(|(phase, _, _)| phase == name) {
            Some((_, total, times)) => {
                *total += took;
                *times += 1;
            }
            None => book.phases.push((name.to_owned(), took, 1)),
        }
    }

    fn close(book: &mut Book, now: Duration) {
        if let Some((name, started, mark)) = book.marked.take() {
            drop(mark);
            Self::record(book, &name, now.saturating_sub(started));
        }
    }

    fn render(&self, book: &Book, now: Duration) -> String {
        let total = now.saturating_sub(self.started);
        let spent: Duration = book.phases.iter().map(|(_, took, _)| *took).sum();
        let outside = total.saturating_sub(spent);
        let share = |took: Duration| {
            if total.is_zero() {
                0.0
            } else {
                took.as_secs_f64() / total.as_secs_f64() * 100.0
            }
        };
        let width = book
            .phases
            .iter()
            .map(|(name, _, _)| name.chars().count())
            .max()
            .unwrap_or(0)
            .max("outside every phase".len());
        let mut text = format!(
            "phases of `{}`: {} in all\n  {:<width$}  {:>12}  {:>6}  times",
            self.test,
            ms(total),
            "phase",
            "took",
            "share",
        );
        for (name, took, times) in &book.phases {
            let times = if *times > 1 {
                times.to_string()
            } else {
                String::new()
            };
            let row = format!(
                "  {name:<width$}  {:>12}  {:>5.1}%  {times}",
                ms(*took),
                share(*took),
            );
            text.push('\n');
            text.push_str(row.trim_end());
        }
        let _ = write!(
            text,
            "\n  {:<width$}  {:>12}  {:>5.1}%",
            "outside every phase",
            ms(outside),
            share(outside),
        );
        text
    }
}

impl Drop for Phases {
    fn drop(&mut self) {
        let now = self.clock.monotonic();
        let mut book = self.lock();
        Self::close(&mut book, now);
        if !book.printed {
            book.printed = true;
            let text = self.render(&book, now);
            drop(book);
            show(&text);
        }
    }
}

impl Drop for Phase<'_> {
    fn drop(&mut self) {
        let took = self.phases.clock.monotonic().saturating_sub(self.started);
        self.phases.add(&self.name, took);
    }
}

/// The breakdown, to the output the test runner keeps for a failed test and
/// shows for a passing one with `--no-capture`.
#[allow(
    clippy::print_stdout,
    reason = "the breakdown is the point of the module, and the runner keeps standard output"
)]
fn show(text: &str) {
    println!("{text}");
}

/// Milliseconds, with a comma between thousands: `1,873.4 ms`.
fn ms(duration: Duration) -> String {
    let text = format!("{:.1}", duration.as_secs_f64() * 1000.0);
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, "0"));
    let mut grouped = String::new();
    for (at, digit) in whole.chars().enumerate() {
        if at > 0 && (whole.len() - at).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{grouped}.{fraction} ms")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cineris_base::FakeClock;

    fn phases(clock: &FakeClock) -> Phases {
        Phases::with_clock("perf_x", Arc::new(clock.clone()))
    }

    fn wait(clock: &FakeClock, millis: u64) {
        clock.advance(Duration::from_millis(millis));
    }

    #[test]
    fn each_phase_takes_what_the_clock_moved_and_the_rest_is_outside_every_phase() {
        let clock = FakeClock::default();
        let phases = phases(&clock);
        phases.measure("set-up", || wait(&clock, 300));
        wait(&clock, 50);
        {
            let _cold = phases.phase("cold pass");
            wait(&clock, 600);
        }
        let report = phases.report();
        assert!(
            report.starts_with("phases of `perf_x`: 950.0 ms in all"),
            "{report}"
        );
        let line = |name: &str| report.lines().find(|line| line.contains(name)).expect(name);
        assert!(
            line("set-up").contains("300.0 ms") && line("set-up").contains("31.6%"),
            "{report}"
        );
        assert!(
            line("cold pass").contains("600.0 ms") && line("cold pass").contains("63.2%"),
            "{report}"
        );
        assert!(
            line("outside every phase").contains("50.0 ms")
                && line("outside every phase").contains("5.3%"),
            "{report}"
        );
        assert_eq!(phases.took("cold pass"), Some(Duration::from_millis(600)));
        assert_eq!(phases.took("missing"), None);
    }

    #[test]
    fn phases_of_one_name_add_up_in_the_order_the_names_first_appeared() {
        let clock = FakeClock::default();
        let phases = phases(&clock);
        for _ in 0..3 {
            phases.measure("warm pass", || wait(&clock, 10));
            phases.measure("map", || wait(&clock, 5));
        }
        phases.measure("set-up", || wait(&clock, 1));
        let report = phases.report();
        let names: Vec<&str> = report
            .lines()
            .skip(2)
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        assert_eq!(names, ["warm", "map", "set-up", "outside"], "{report}");
        let warm = report
            .lines()
            .find(|line| line.contains("warm pass"))
            .unwrap();
        assert!(
            warm.contains("30.0 ms") && warm.trim_end().ends_with('3'),
            "{warm}"
        );
        let once = report.lines().find(|line| line.contains("set-up")).unwrap();
        assert!(
            once.trim_end().ends_with('%'),
            "a phase that ran once has no count: {once}"
        );
    }

    #[test]
    fn a_mark_lasts_until_the_next_mark_the_end_or_the_report() {
        let clock = FakeClock::default();
        let phases = phases(&clock);
        phases.mark("a");
        wait(&clock, 100);
        phases.mark("b");
        wait(&clock, 200);
        phases.end();
        wait(&clock, 40);
        phases.mark("c");
        wait(&clock, 7);
        let report = phases.report();
        assert_eq!(phases.took("a"), Some(Duration::from_millis(100)));
        assert_eq!(phases.took("b"), Some(Duration::from_millis(200)));
        assert_eq!(phases.took("c"), Some(Duration::from_millis(7)), "{report}");
        let line = report
            .lines()
            .find(|line| line.contains("outside"))
            .unwrap();
        assert!(line.contains("40.0 ms"), "{report}");
    }

    #[test]
    fn the_phases_and_what_is_outside_them_add_up_to_the_total() {
        let clock = FakeClock::default();
        let phases = phases(&clock);
        wait(&clock, 13);
        phases.measure("one", || wait(&clock, 29));
        phases.mark("two");
        wait(&clock, 31);
        wait(&clock, 11);
        let report = phases.report();
        let shares: f64 = report
            .lines()
            .skip(2)
            .map(|line| {
                line.split_whitespace()
                    .find_map(|word| word.strip_suffix('%'))
                    .expect("a share")
                    .parse::<f64>()
                    .unwrap()
            })
            .sum();
        assert!((shares - 100.0).abs() < 0.3, "{shares}: {report}");
        assert!(
            report.starts_with("phases of `perf_x`: 84.0 ms in all"),
            "{report}"
        );
    }

    #[test]
    fn a_report_is_printed_once_and_the_drop_does_not_repeat_it() {
        let clock = FakeClock::default();
        let phases = phases(&clock);
        phases.measure("x", || wait(&clock, 1));
        let _ = phases.report();
        assert!(phases.lock().printed);
        // Dropping prints nothing more: `printed` is what says so.
        drop(phases);
    }

    #[test]
    fn a_whole_with_nothing_in_it_has_no_share() {
        let clock = FakeClock::default();
        let phases = phases(&clock);
        let report = phases.report();
        assert!(
            report.contains("0.0%") && !report.contains("NaN"),
            "{report}"
        );
    }

    #[test]
    fn times_are_written_with_thousands_grouped() {
        assert_eq!(ms(Duration::from_micros(1_873_400)), "1,873.4 ms");
        assert_eq!(ms(Duration::from_micros(999_900)), "999.9 ms");
        assert_eq!(ms(Duration::from_secs(1_234)), "1,234,000.0 ms");
        assert_eq!(ms(Duration::ZERO), "0.0 ms");
    }
}
