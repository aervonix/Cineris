//! Where a turn's time goes (`docs/architecture/03-agent-loop.md`, "What is
//! recorded for adaptation").
//!
//! A turn's wall time is cut into spans of one kind each, by what the turn
//! was waiting for at that moment: a model (connecting, the first byte,
//! streaming, a retry's wait, a summary), tools, the user's answer, the
//! project's checks, or nothing but the harness's own work. Every moment is
//! in exactly one, so the parts add up to the whole.
//!
//! | The loop is | The moment is |
//! |---|---|
//! | Sending a request, until the provider answers | `connect` |
//! | Answered, until the response's first event | `first_byte` |
//! | Taking the response, tools started early included | `streaming` |
//! | Waiting to send a failed request again | `retry`, then `connect` once the wait is over |
//! | Compacting, waiting for the summary | `summary` |
//! | In a tool round, with a call running | `tools`, shared by the calls running |
//! | In a tool round, with none running and a question open | `approval` |
//! | Verifying | `verification` |
//! | Anything else | `harness` |
//!
//! The clock is the loop's monotonic clock: in tests, the fake one. Keeping
//! it costs a subtraction and an addition at each change of state.

use std::collections::BTreeMap;
use std::time::Duration;

use cortex_protocol::usage::{ModelTime, TimeTotals};

/// What a moment of a turn was spent on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spend {
    Harness,
    Connect,
    FirstByte,
    Streaming,
    Retry,
    Summary,
    Tools,
    Approval,
    Verification,
}

const KINDS: usize = 9;

impl Spend {
    const fn index(self) -> usize {
        match self {
            Self::Harness => 0,
            Self::Connect => 1,
            Self::FirstByte => 2,
            Self::Streaming => 3,
            Self::Retry => 4,
            Self::Summary => 5,
            Self::Tools => 6,
            Self::Approval => 7,
            Self::Verification => 8,
        }
    }
}

/// The parts of a turn's time so far, and what the turn is spending now.
#[derive(Debug, Clone)]
pub(crate) struct TurnClock {
    started: Duration,
    current: Spend,
    since: Duration,
    /// While waiting to send again: when the wait ends. After it the time
    /// is the next attempt's connection.
    retry_until: Option<Duration>,
    totals: [Duration; KINDS],
    /// The tools running now, which share the tools' time.
    running: Vec<String>,
    /// Each tool's share of the tools' time.
    shares: BTreeMap<String, Duration>,
}

/// A turn's time, cut into its parts, in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnTimes {
    /// The parts, which add up to `wall_ms`.
    pub totals: TimeTotals,
    /// `tools_ms` by tool, in name order; adds up to it.
    pub by_tool: Vec<(String, u64)>,
}

impl TurnClock {
    pub(crate) fn new(now: Duration) -> Self {
        Self {
            started: now,
            current: Spend::Harness,
            since: now,
            retry_until: None,
            totals: [Duration::ZERO; KINDS],
            running: Vec::new(),
            shares: BTreeMap::new(),
        }
    }

    /// Adds the time since the last change to what was being spent.
    fn close(&mut self, now: Duration) {
        let spent = now.saturating_sub(self.since);
        match self.current {
            Spend::Retry => {
                let waited = self
                    .retry_until
                    .map_or(spent, |until| until.saturating_sub(self.since).min(spent));
                self.totals[Spend::Retry.index()] += waited;
                self.totals[Spend::Connect.index()] += spent - waited;
            }
            Spend::Tools if !self.running.is_empty() => {
                self.totals[Spend::Tools.index()] += spent;
                // An equal share each, the nanoseconds left over to the
                // first, so that the shares add up to the whole.
                let count = u32::try_from(self.running.len()).unwrap_or(u32::MAX);
                let each = spent / count;
                let rest = spent - each * count;
                for (at, tool) in self.running.iter().enumerate() {
                    let share = if at == 0 { each + rest } else { each };
                    *self.shares.entry(tool.clone()).or_default() += share;
                }
            }
            other => self.totals[other.index()] += spent,
        }
        self.since = now.max(self.since);
    }

    /// From `now` the turn spends its time on `spend`.
    pub(crate) fn enter(&mut self, now: Duration, spend: Spend) {
        self.close(now);
        self.current = spend;
        self.retry_until = None;
    }

    /// From `now` the turn waits `wait` to send a request again.
    pub(crate) fn retry(&mut self, now: Duration, wait: Duration) {
        self.enter(now, Spend::Retry);
        self.retry_until = Some(now + wait);
    }

    /// In a tool round, at `now`: these calls are running, and a question
    /// is open or not.
    pub(crate) fn round(&mut self, now: Duration, running: Vec<String>, asking: bool) {
        self.close(now);
        self.current = if !running.is_empty() {
            Spend::Tools
        } else if asking {
            Spend::Approval
        } else {
            Spend::Harness
        };
        self.retry_until = None;
        self.running = running;
    }

    /// What the model's part has come to by `now`: for one request's own
    /// time, taken before and after it.
    pub(crate) fn model_time(&mut self, now: Duration) -> [Duration; 4] {
        self.close(now);
        [
            self.totals[Spend::Connect.index()],
            self.totals[Spend::FirstByte.index()],
            self.totals[Spend::Streaming.index()],
            self.totals[Spend::Retry.index()],
        ]
    }

    /// The turn ended at `now`: its parts, in milliseconds that add up to
    /// its wall time in milliseconds.
    pub(crate) fn finish(&mut self, now: Duration) -> TurnTimes {
        self.close(now);
        // Each part's milliseconds are where it ends less where it begins,
        // laid end to end and each end rounded down: so they add up to the
        // whole rounded down, which is the turn's own `duration_ms`.
        let order = [
            Spend::Connect,
            Spend::FirstByte,
            Spend::Streaming,
            Spend::Retry,
            Spend::Summary,
            Spend::Tools,
            Spend::Approval,
            Spend::Verification,
            Spend::Harness,
        ];
        let mut at = Duration::ZERO;
        let mut ms = [0u64; KINDS];
        let mut tools_from = Duration::ZERO;
        for spend in order {
            let begin = at;
            at += self.totals[spend.index()];
            if spend == Spend::Tools {
                tools_from = begin;
            }
            ms[spend.index()] = millis(at) - millis(begin);
        }
        let mut by_tool = Vec::with_capacity(self.shares.len());
        let mut at = tools_from;
        for (tool, share) in &self.shares {
            let begin = at;
            at += *share;
            by_tool.push((tool.clone(), millis(at) - millis(begin)));
        }
        let wall = now.saturating_sub(self.started);
        let totals = TimeTotals {
            turns: 1,
            wall_ms: millis(wall),
            model: ModelTime {
                connect_ms: ms[Spend::Connect.index()],
                first_byte_ms: ms[Spend::FirstByte.index()],
                streaming_ms: ms[Spend::Streaming.index()],
                retry_ms: ms[Spend::Retry.index()],
                summary_ms: ms[Spend::Summary.index()],
            },
            tools_ms: ms[Spend::Tools.index()],
            approval_ms: ms[Spend::Approval.index()],
            verification_ms: ms[Spend::Verification.index()],
            harness_ms: ms[Spend::Harness.index()],
        };
        TurnTimes { totals, by_tool }
    }
}

/// Whole milliseconds, rounded down.
pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn every_moment_is_in_one_part_and_the_parts_add_up() {
        let mut clock = TurnClock::new(ms(1000));
        clock.enter(ms(1003), Spend::Connect);
        clock.enter(ms(1100), Spend::FirstByte);
        clock.enter(ms(1350), Spend::Streaming);
        // A retry of 200 ms, then 40 ms to connect again.
        clock.retry(ms(1500), ms(200));
        clock.enter(ms(1740), Spend::Streaming);
        clock.enter(ms(1800), Spend::Harness);
        // Two tools side by side, then one alone, then a question.
        clock.round(
            ms(1801),
            vec!["read".to_owned(), "search".to_owned()],
            false,
        );
        clock.round(ms(1901), vec!["read".to_owned()], true);
        clock.round(ms(1951), Vec::new(), true);
        clock.round(ms(2951), Vec::new(), false);
        clock.enter(ms(2952), Spend::Verification);
        clock.enter(ms(3000), Spend::Harness);
        let times = clock.finish(ms(3010));
        let totals = &times.totals;
        assert_eq!(totals.wall_ms, 2010);
        assert_eq!(totals.parts_ms(), totals.wall_ms);
        assert_eq!(totals.model.connect_ms, 97 + 40);
        assert_eq!(totals.model.first_byte_ms, 250);
        assert_eq!(totals.model.streaming_ms, 150 + 60);
        assert_eq!(totals.model.retry_ms, 200);
        assert_eq!(totals.tools_ms, 150);
        assert_eq!(totals.approval_ms, 1000);
        assert_eq!(totals.verification_ms, 48);
        assert_eq!(
            times.by_tool,
            [("read".to_owned(), 100), ("search".to_owned(), 50)]
        );
    }

    /// What the loop's spans cost when no subscriber listens: a span made
    /// and its future polled, and a change of what the turn's time is spent
    /// on. Run by hand: `cargo test --release -p cortex-agent --lib
    /// perf_spans -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand"]
    fn perf_spans_with_no_subscriber() {
        use std::future::Future as _;
        use tracing::Instrument as _;
        let waker = std::task::Waker::noop();
        let mut context = std::task::Context::from_waker(waker);
        let rounds = 1_000_000u32;
        let started = std::time::Instant::now();
        for n in 0..rounds {
            let span = tracing::info_span!("tool", tool = %"read", call = %n);
            let mut future = std::pin::pin!(std::future::ready(n).instrument(span));
            let _ = future.as_mut().poll(&mut context);
        }
        let spanned = started.elapsed();
        let started = std::time::Instant::now();
        for n in 0..rounds {
            let mut future = std::pin::pin!(std::future::ready(n));
            let _ = future.as_mut().poll(&mut context);
        }
        let bare = started.elapsed();
        println!(
            "a span made and its future polled, with no subscriber: {:?} each ({:?} without the span)",
            spanned / rounds,
            bare / rounds
        );
        let mut clock = TurnClock::new(Duration::ZERO);
        let started = std::time::Instant::now();
        for n in 0..rounds {
            clock.enter(Duration::from_micros(u64::from(n)), Spend::Streaming);
        }
        println!(
            "a change of what the turn's time is spent on: {:?} each",
            started.elapsed() / rounds
        );
    }

    #[test]
    fn milliseconds_add_up_however_the_parts_fall() {
        // Parts of a fraction of a millisecond each still add up to the
        // wall time rounded down.
        let mut clock = TurnClock::new(Duration::ZERO);
        let mut now = Duration::ZERO;
        let kinds = [
            Spend::Connect,
            Spend::Streaming,
            Spend::Harness,
            Spend::Verification,
        ];
        for step in 0..997u32 {
            now += Duration::from_micros(333 + u64::from(step % 7));
            clock.enter(now, kinds[step as usize % kinds.len()]);
            if step % 5 == 0 {
                clock.round(
                    now,
                    vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
                    false,
                );
            }
        }
        let times = clock.finish(now);
        assert_eq!(times.totals.parts_ms(), times.totals.wall_ms);
        let shares: u64 = times.by_tool.iter().map(|(_, ms)| ms).sum();
        assert_eq!(shares, times.totals.tools_ms);
    }
}
