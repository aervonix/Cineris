//! Telling a machine that stalled from tests that hang.
//!
//! When a shared machine's disk stalls, every test that touches a file
//! waits with it, and a dozen tests of crates nobody changed run out of
//! time together. nextest reports each as a timeout, and nothing says that
//! the machine stopped: step 18 spent an hour on such a run, and
//! `mutate --jobs 4` lost baselines to it in wave 9.
//!
//! **The sign is when they were stopped.** A test that hangs by itself is
//! stopped at its own limit, counted from its own start. When the machine
//! stalls, the timers of the test runner are late too, and fire together
//! when it comes back: the tests are stopped within moments of each other,
//! whatever time each started. So [`find`] looks for [`AT_LEAST`] timeouts
//! whose ends (start plus time, from the JUnit report) lie within
//! [`WINDOW_MS`] of each other. One test's timeout is never a stall.
//!
//! **The evidence is the kernel's.** On Linux, `/proc/pressure/io` says
//! what share of the time tasks waited on I/O, over the last 10 and 60
//! seconds, and in all since boot. [`Watch`] reads it when a run starts
//! and again at its end, so the message gives the share of the run itself
//! that was spent stalled, beside the averages. Elsewhere, or where the
//! file cannot be read, the message says so.

use std::time::{Duration, Instant};

use crate::junit::Failure;

/// How close together, in milliseconds, the ends of the timeouts must lie.
pub(crate) const WINDOW_MS: i64 = 250;

/// How many timeouts so close together are taken for the machine. Two
/// tests started in the same moment and both hanging on one bug would
/// look like a stall; three such is rarer, and the message still shows the
/// kernel's pressure, which a bug does not move.
pub(crate) const AT_LEAST: usize = 3;

/// Timeouts that ended together.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stall {
    /// The tests, as `binary name`, in the order they were stopped.
    pub(crate) tests: Vec<String>,
    /// From the first end to the last, in milliseconds.
    pub(crate) spread_ms: i64,
}

/// The largest group of timeouts among `failures` whose ends lie within
/// [`WINDOW_MS`] of each other and whose starts do not, when it has
/// [`AT_LEAST`] of them.
///
/// Tests that started together and hang on one bug are stopped together
/// too, each at its start plus the same limit: nextest starts its first
/// tests within moments of each other, and a break that makes a shared
/// path hang (in `mutate`) ends them all in one breath. What a stall does
/// that a bug does not is stop tests that started at different times.
pub(crate) fn find(failures: &[Failure]) -> Option<Stall> {
    let mut ends: Vec<(i64, i64, String)> = failures
        .iter()
        .filter(|failure| failure.kind.contains("timeout"))
        .filter_map(|failure| {
            let start = instant_ms(&failure.started)?;
            // Rounded to the millisecond: nextest writes three decimals.
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a test's time in milliseconds fits an i64"
            )]
            let took = (failure.seconds * 1000.0).round() as i64;
            Some((
                start + took,
                start,
                format!("{} {}", failure.binary, failure.name),
            ))
        })
        .collect();
    ends.sort();
    let started_apart = |group: &[(i64, i64, String)]| {
        let starts = group.iter().map(|(_, start, _)| *start);
        let (earliest, latest) = (starts.clone().min(), starts.max());
        matches!((earliest, latest), (Some(earliest), Some(latest)) if latest - earliest > WINDOW_MS)
    };
    let mut best = (0, 0);
    let mut first = 0;
    for last in 0..ends.len() {
        while ends[last].0 - ends[first].0 > WINDOW_MS {
            first += 1;
        }
        if last + 1 - first > best.1 - best.0 && started_apart(&ends[first..=last]) {
            best = (first, last + 1);
        }
    }
    let group = &ends[best.0..best.1];
    if group.len() < AT_LEAST {
        return None;
    }
    Some(Stall {
        tests: group.iter().map(|(_, _, test)| test.clone()).collect(),
        spread_ms: group[group.len() - 1].0 - group[0].0,
    })
}

/// Milliseconds since 1970 of a time as nextest writes one:
/// `2026-11-15T01:27:48.020+00:00`, with any offset, or `Z`.
pub(crate) fn instant_ms(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let mut date = date.splitn(3, '-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let (clock, offset_minutes) = if let Some(clock) = time.strip_suffix('Z') {
        (clock, 0)
    } else {
        let at = time.rfind(['+', '-'])?;
        let (clock, offset) = time.split_at(at);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let (hours, minutes) = offset[1..].split_once(':')?;
        (
            clock,
            sign * (hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?),
        )
    };
    let mut parts = clock.splitn(3, ':');
    let hours: i64 = parts.next()?.parse().ok()?;
    let minutes: i64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "seconds within a minute, in milliseconds"
    )]
    let millis = (seconds * 1000.0).round() as i64;
    Some(((days * 24 + hours) * 60 + minutes - offset_minutes) * 60_000 + millis)
}

/// What `/proc/pressure/io` says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pressure {
    /// The share of the last 10 and 60 seconds, in percent, in which some
    /// task waited on I/O.
    pub(crate) some: (f64, f64),
    /// The same, for every task at once.
    pub(crate) full: (f64, f64),
    /// Microseconds since boot in which some task, and every task, waited.
    pub(crate) some_total: u64,
    pub(crate) full_total: u64,
}

/// Reads the `some` and `full` lines of `/proc/pressure/io`.
pub(crate) fn parse_pressure(text: &str) -> Option<Pressure> {
    let line = |kind: &str| -> Option<(f64, f64, u64)> {
        let line = text.lines().find(|line| line.starts_with(kind))?;
        let field = |name: &str| {
            line.split_whitespace()
                .find_map(|word| word.strip_prefix(name)?.strip_prefix('='))
        };
        Some((
            field("avg10")?.parse().ok()?,
            field("avg60")?.parse().ok()?,
            field("total")?.parse().ok()?,
        ))
    };
    let some = line("some ")?;
    // A kernel without the `full` line for I/O has none to give.
    let full = line("full ").unwrap_or((0.0, 0.0, 0));
    Some(Pressure {
        some: (some.0, some.1),
        full: (full.0, full.1),
        some_total: some.2,
        full_total: full.2,
    })
}

/// Where the kernel says it.
const PRESSURE_FILE: &str = "/proc/pressure/io";

/// The I/O pressure now, or why it cannot be read.
pub(crate) fn read_pressure() -> Result<Pressure, String> {
    if !cfg!(target_os = "linux") {
        return Err(format!(
            "{PRESSURE_FILE} is Linux's, and this is {}",
            std::env::consts::OS
        ));
    }
    let text = std::fs::read_to_string(PRESSURE_FILE)
        .map_err(|error| format!("{PRESSURE_FILE} could not be read: {error}"))?;
    parse_pressure(&text).ok_or_else(|| format!("{PRESSURE_FILE} was not in the form expected"))
}

/// A run being watched: the pressure when it started.
#[derive(Debug)]
pub(crate) struct Watch {
    before: Option<Pressure>,
    started: Instant,
}

impl Watch {
    /// Notes the pressure now.
    pub(crate) fn start() -> Self {
        Self {
            before: read_pressure().ok(),
            started: Instant::now(),
        }
    }

    /// What to say of a run whose report holds `failures`: nothing unless
    /// its timeouts ended together.
    pub(crate) fn explain(&self, failures: &[Failure]) -> Option<String> {
        let stall = find(failures)?;
        Some(explain(
            &stall,
            self.before.as_ref(),
            &read_pressure(),
            self.started.elapsed(),
        ))
    }
}

/// How many of a stall's tests are named.
const NAMED: usize = 6;

/// The message for a stall, with the pressure before and after a run of
/// `elapsed`.
pub(crate) fn explain(
    stall: &Stall,
    before: Option<&Pressure>,
    now: &Result<Pressure, String>,
    elapsed: Duration,
) -> String {
    let mut text = format!(
        "STALL: {} tests timed out within {} ms of each other: likely the machine, not the \
         tests.\n",
        stall.tests.len(),
        stall.spread_ms
    );
    for test in stall.tests.iter().take(NAMED) {
        text.push_str(&format!("  {test}\n"));
    }
    if stall.tests.len() > NAMED {
        text.push_str(&format!("  and {} more\n", stall.tests.len() - NAMED));
    }
    match now {
        Ok(now) => {
            text.push_str(&format!(
                "  I/O pressure ({PRESSURE_FILE}) now: some task waited {:.1}% of the last 10 s \
                 and {:.1}% of the last 60 s; every task at once {:.1}% and {:.1}%.\n",
                now.some.0, now.some.1, now.full.0, now.full.1
            ));
            if let Some(before) = before {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "microseconds of a run, shown to a tenth of a second"
                )]
                let seconds = |micros: u64| micros as f64 / 1e6;
                text.push_str(&format!(
                    "  During this run's {:.0} s, some task waited on I/O for {:.1} s, and every \
                     task at once for {:.1} s.\n",
                    elapsed.as_secs_f64(),
                    seconds(now.some_total.saturating_sub(before.some_total)),
                    seconds(now.full_total.saturating_sub(before.full_total)),
                ));
            }
        }
        Err(why) => text.push_str(&format!("  The I/O pressure could not be read: {why}.\n")),
    }
    text.push_str(
        "  Run them again, alone or with fewer jobs, before reading anything into these \
         timeouts.\n",
    );
    text
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Pressure, explain, find, instant_ms, parse_pressure};
    use crate::junit::Failure;

    fn timeout(name: &str, started: &str, seconds: f64) -> Failure {
        Failure {
            binary: "cx::it".to_owned(),
            name: name.to_owned(),
            repeat: None,
            text: String::new(),
            kind: "test timeout".to_owned(),
            stderr: String::new(),
            started: started.to_owned(),
            seconds,
        }
    }

    #[test]
    fn a_time_is_read_with_its_offset() {
        assert_eq!(instant_ms("1970-01-01T00:00:00.000+00:00"), Some(0));
        assert_eq!(instant_ms("1970-01-01T00:00:01.250Z"), Some(1250));
        assert_eq!(
            instant_ms("2026-11-15T01:27:48.020+00:00"),
            Some(1_794_706_068_020)
        );
        // The same moment, written in another zone.
        assert_eq!(
            instant_ms("2026-11-15T02:27:48.020+01:00"),
            instant_ms("2026-11-15T01:27:48.020+00:00")
        );
        assert_eq!(
            instant_ms("2026-11-14T20:27:48.020-05:00"),
            instant_ms("2026-11-15T01:27:48.020+00:00")
        );
        // A leap year's last day, and the turn of a century.
        assert_eq!(
            instant_ms("2024-12-31T00:00:00Z").map(|ms| ms / 86_400_000),
            Some(20_088)
        );
        assert_eq!(
            instant_ms("2000-03-01T00:00:00Z").map(|ms| ms / 86_400_000),
            Some(11_017)
        );
        assert_eq!(instant_ms("not a time"), None);
        assert_eq!(instant_ms(""), None);
    }

    #[test]
    fn timeouts_that_end_together_are_a_stall_whatever_time_each_started() {
        // Started across eight seconds, stopped within 200 ms.
        let failures = [
            timeout("a", "2026-11-15T10:00:00.000+00:00", 10.0),
            timeout("b", "2026-11-15T10:00:03.000+00:00", 7.1),
            timeout("c", "2026-11-15T10:00:08.000+00:00", 2.2),
            // A panic beside them is not a timeout.
            Failure {
                kind: "test failure with exit code 101".to_owned(),
                ..timeout("d", "2026-11-15T10:00:09.900+00:00", 0.2)
            },
        ];
        let stall = find(&failures).expect("a stall");
        assert_eq!(stall.tests, ["cx::it a", "cx::it b", "cx::it c"]);
        assert_eq!(stall.spread_ms, 200);
    }

    #[test]
    fn one_tests_timeout_is_not_a_stall_nor_are_timeouts_apart() {
        assert_eq!(
            find(&[timeout("a", "2026-11-15T10:00:00.000+00:00", 10.0)]),
            None
        );
        // Two together are not enough.
        assert_eq!(
            find(&[
                timeout("a", "2026-11-15T10:00:00.000+00:00", 10.0),
                timeout("b", "2026-11-15T10:00:00.100+00:00", 10.0),
            ]),
            None
        );
        // Three, a second apart each: tests that hang, each at its own
        // limit.
        assert_eq!(
            find(&[
                timeout("a", "2026-11-15T10:00:00.000+00:00", 10.0),
                timeout("b", "2026-11-15T10:00:01.000+00:00", 10.0),
                timeout("c", "2026-11-15T10:00:02.000+00:00", 10.0),
            ]),
            None
        );
        // Just outside the window.
        assert_eq!(
            find(&[
                timeout("a", "2026-11-15T10:00:00.000+00:00", 10.0),
                timeout("b", "2026-11-15T10:00:00.100+00:00", 10.0),
                timeout("c", "2026-11-15T10:00:00.251+00:00", 10.0),
            ]),
            None
        );
    }

    #[test]
    fn tests_that_started_together_and_hang_together_are_not_a_stall() {
        // nextest's first tests start within moments of each other; a bug
        // that hangs them all stops them at the same limit, together.
        let failures = [
            timeout("a", "2026-11-15T10:00:00.000+00:00", 10.0),
            timeout("b", "2026-11-15T10:00:00.004+00:00", 10.0),
            timeout("c", "2026-11-15T10:00:00.009+00:00", 10.0),
            timeout("d", "2026-11-15T10:00:00.200+00:00", 10.0),
        ];
        assert_eq!(find(&failures), None);
        // One more, started seconds before and stopped with them: the
        // machine's, and the group is named.
        let mut stalled = failures.to_vec();
        stalled.push(timeout("e", "2026-11-15T10:00:04.000+00:00", 6.1));
        assert_eq!(find(&stalled).map(|stall| stall.tests.len()), Some(5));
    }

    #[test]
    fn the_largest_group_is_the_one_named() {
        let failures = [
            timeout("early-1", "2026-11-15T09:59:58.000+00:00", 12.0),
            timeout("early-2", "2026-11-15T10:00:00.050+00:00", 10.0),
            timeout("early-3", "2026-11-15T10:00:00.090+00:00", 10.0),
            timeout("late-1", "2026-11-15T10:00:59.000+00:00", 11.0),
            timeout("late-2", "2026-11-15T10:01:00.010+00:00", 10.0),
            timeout("late-3", "2026-11-15T10:01:00.020+00:00", 10.0),
            timeout("late-4", "2026-11-15T10:01:00.249+00:00", 10.0),
        ];
        let stall = find(&failures).expect("a stall");
        assert_eq!(stall.tests.len(), 4);
        assert!(stall.tests[0].ends_with("late-1"), "{:?}", stall.tests);
        assert_eq!(stall.spread_ms, 249);
    }

    #[test]
    fn the_kernels_pressure_is_read() {
        let text = "some avg10=12.50 avg60=3.10 avg300=0.80 total=41300000\n\
                    full avg10=9.00 avg60=2.00 avg300=0.50 total=30200000\n";
        assert_eq!(
            parse_pressure(text),
            Some(Pressure {
                some: (12.5, 3.1),
                full: (9.0, 2.0),
                some_total: 41_300_000,
                full_total: 30_200_000,
            })
        );
        assert_eq!(parse_pressure("nothing here"), None);
    }

    #[test]
    fn the_message_names_the_tests_and_the_stall_time_of_the_run() {
        let stall = super::Stall {
            tests: (1..=8).map(|n| format!("cx::it t{n}")).collect(),
            spread_ms: 140,
        };
        let before = Pressure {
            some: (0.0, 0.0),
            full: (0.0, 0.0),
            some_total: 1_000_000,
            full_total: 500_000,
        };
        let now = Pressure {
            some: (40.0, 15.5),
            full: (30.0, 10.0),
            some_total: 42_300_000,
            full_total: 30_700_000,
        };
        let text = explain(&stall, Some(&before), &Ok(now), Duration::from_secs(312));
        assert!(
            text.starts_with(
                "STALL: 8 tests timed out within 140 ms of each other: likely the machine"
            ),
            "{text}"
        );
        assert!(text.contains("  cx::it t6\n  and 2 more\n"), "{text}");
        assert!(text.contains("40.0% of the last 10 s and 15.5%"), "{text}");
        assert!(
            text.contains("During this run's 312 s, some task waited on I/O for 41.3 s, and every task at once for 30.2 s."),
            "{text}"
        );
        let unread = explain(
            &stall,
            None,
            &Err("it is not Linux".to_owned()),
            Duration::ZERO,
        );
        assert!(
            unread.contains("The I/O pressure could not be read: it is not Linux."),
            "{unread}"
        );
    }
}
