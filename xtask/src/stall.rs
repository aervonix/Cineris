//! Telling a machine that stalled from tests that hang.
//!
//! When a shared machine's disk stalls, every test that touches a file
//! waits with it, and a dozen tests of crates nobody changed run out of
//! time together. nextest reports each as a timeout, and nothing says that
//! the machine stopped: step 18 spent an hour on such a run, and
//! `mutate --jobs 4` lost baselines to it in wave 9.
//!
//! **The sign is when they were stopped.** A test that hangs, by itself or
//! on a bug it shares with others, is stopped on time, at its own limit.
//! When the machine stalls, the timers of the test runner are late too, and
//! fire together when it comes back: the tests are stopped late, past their
//! limits, and within moments of each other. So [`find`] looks for
//! [`AT_LEAST`] timeouts stopped late ([`stopped_late`]) whose ends (start
//! plus time, from the JUnit report) lie within [`WINDOW_MS`] of each other.
//! One test's timeout is never a stall.
//!
//! **The evidence is the kernel's.** On Linux, `/proc/pressure/io` says
//! what share of the time tasks waited on I/O, over the last 10 and 60
//! seconds, and in all since boot. [`Watch`] reads it when a run starts
//! and again at its end, so the message gives the share of the run itself
//! that was spent stalled, beside the averages. Elsewhere, or where the
//! file cannot be read, the message says so.
//!
//! **A run's stall is a number, and two commands act on it.** [`Observed`]
//! is what the pressure said of one run: the share of its time in which
//! every task waited on I/O. `cargo xtask ci` runs a cluster of timeouts
//! again once when that share is [`STALLED_SHARE`] or more ([`rerun`]), and
//! its tier check does not judge a run at a desk whose share is that high
//! ([`judge`]). Both decisions are pure functions of the pressures and the
//! JUnit report, so that tests give them a recorded pressure and never a
//! real stall; [`PRESSURE_FILE_VARIABLE`] names a file to read in place of
//! the kernel's, for the same reason.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::junit::Failure;

pub(crate) mod rerun;

/// The share of a run's time, in percent, in which every task waited on
/// I/O (`full` of `/proc/pressure/io`), from which the run is taken to have
/// stalled: `cargo xtask ci` runs a cluster of timeouts again, and does not
/// hold the tiers to their budgets at a desk.
///
/// It is `mutate`'s figure for a busy disk (`full avg60` of 10%, from which
/// it starts with one job): a run that spent a tenth of its time with every
/// task waiting on the disk has lost a tenth of its time to it, which is
/// 2 s of a tier's 20 s. It was chosen, not fitted to quiet runs, of which
/// nobody has a distribution: the stalled runs of wave 11 read 54% and 66%,
/// and a machine at rest reads 0%. A lower figure would take every busy
/// afternoon of the desktop for a stall and run tests again for nothing; a
/// higher one would call a run that lost a fifth of its time judged.
pub(crate) const STALLED_SHARE: f64 = 10.0;

/// How close together, in milliseconds, the ends of the timeouts must lie.
pub(crate) const WINDOW_MS: i64 = 250;

/// How many timeouts so close together are taken for the machine.
pub(crate) const AT_LEAST: usize = 3;

/// The limits nextest stops a test at, in milliseconds, in the profiles of
/// `.config/nextest.toml`: tiers 1 and 2, tier 3, the nightly `properties`
/// run and the `perf` measurements. A test checks them against the file.
pub(crate) const LIMITS_MS: [i64; 4] = [10_000, 60_000, 300_000, 1_800_000];

/// Timeouts that ended together.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stall {
    /// The tests, as `binary name`, in the order they were stopped.
    pub(crate) tests: Vec<String>,
    /// From the first end to the last, in milliseconds.
    pub(crate) spread_ms: i64,
}

/// Whether a test that ran `took_ms` was stopped late: not within
/// [`WINDOW_MS`] after one of nextest's limits.
///
/// A test that hangs by itself, or on a bug it shares with others, is
/// stopped on time, at its own limit: a break that hangs a crate's tests
/// stops its system tests at 60 s and a later wave of its other tests at
/// 10 s, all in one moment, and none of them late. When the machine stalls,
/// the test runner's own timers are late, and fire when it comes back: the
/// tests it stops then have run past their limits, each by its own amount.
pub(crate) fn stopped_late(took_ms: i64) -> bool {
    !LIMITS_MS
        .iter()
        .any(|limit| took_ms >= *limit && took_ms - limit <= WINDOW_MS)
}

/// The largest group of timeouts among `failures` that were stopped late
/// ([`stopped_late`]) and whose ends lie within [`WINDOW_MS`] of each
/// other, when it has [`AT_LEAST`] of them.
pub(crate) fn find(failures: &[Failure]) -> Option<Stall> {
    find_cases(failures).map(|(stall, _)| stall)
}

/// [`find`], with the places in `failures` of the cases it found.
pub(crate) fn find_cases(failures: &[Failure]) -> Option<(Stall, Vec<usize>)> {
    let mut ends: Vec<(i64, String, usize)> = failures
        .iter()
        .enumerate()
        .filter(|(_, failure)| failure.kind.contains("timeout"))
        .filter_map(|(place, failure)| {
            let start = instant_ms(&failure.started)?;
            // Rounded to the millisecond: nextest writes three decimals.
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a test's time in milliseconds fits an i64"
            )]
            let took = (failure.seconds * 1000.0).round() as i64;
            stopped_late(took).then(|| {
                (
                    start + took,
                    format!("{} {}", failure.binary, failure.name),
                    place,
                )
            })
        })
        .collect();
    ends.sort();
    let mut best = (0, 0);
    let mut first = 0;
    for last in 0..ends.len() {
        while ends[last].0 - ends[first].0 > WINDOW_MS {
            first += 1;
        }
        if last + 1 - first > best.1 - best.0 {
            best = (first, last + 1);
        }
    }
    let group = &ends[best.0..best.1];
    if group.len() < AT_LEAST {
        return None;
    }
    let stall = Stall {
        tests: group.iter().map(|(_, test, _)| test.clone()).collect(),
        spread_ms: group[group.len() - 1].0 - group[0].0,
    };
    Some((stall, group.iter().map(|(_, _, place)| *place).collect()))
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

/// The environment variable that names a file to read the pressure from in
/// place of the kernel's, on any system: how the tests of the commands that
/// act on a stall, and a person trying one by hand, give it a pressure that
/// was recorded or made up and never a real stall. The file is in the
/// kernel's own form (`some avg10=… avg60=… avg300=… total=…`, and `full`).
pub(crate) const PRESSURE_FILE_VARIABLE: &str = "CINERIS_PRESSURE_FILE";

/// The I/O pressure now, or why it cannot be read: from the file that
/// [`PRESSURE_FILE_VARIABLE`] names, when it is set and not empty, else the
/// kernel's.
pub(crate) fn read_pressure() -> Result<Pressure, String> {
    read_pressure_with(std::env::var_os(PRESSURE_FILE_VARIABLE))
}

/// [`read_pressure`], given what [`PRESSURE_FILE_VARIABLE`] is set to.
fn read_pressure_with(file: Option<std::ffi::OsString>) -> Result<Pressure, String> {
    match file.filter(|file| !file.is_empty()) {
        Some(file) => read_pressure_from(Path::new(&file)),
        None => read_kernel_pressure(),
    }
}

/// The pressure a file says, in the kernel's form.
pub(crate) fn read_pressure_from(file: &Path) -> Result<Pressure, String> {
    let shown = file.display();
    let text = std::fs::read_to_string(file).map_err(|error| {
        format!("{shown} ({PRESSURE_FILE_VARIABLE}) could not be read: {error}")
    })?;
    parse_pressure(&text).ok_or_else(|| format!("{shown} was not in the form expected"))
}

fn read_kernel_pressure() -> Result<Pressure, String> {
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
    before: Result<Pressure, String>,
    started: Instant,
}

impl Watch {
    /// Notes the pressure now.
    pub(crate) fn start() -> Self {
        Self {
            before: read_pressure(),
            started: Instant::now(),
        }
    }

    /// What the pressure said of the run since [`Watch::start`], or why it
    /// cannot say: it could not be read at the start or cannot be now.
    pub(crate) fn observe(&self) -> Result<Observed, String> {
        let before = self.before.clone()?;
        Ok(Observed {
            before,
            after: read_pressure()?,
            elapsed: self.started.elapsed(),
        })
    }

    /// What to say of a run whose report holds `failures`: nothing unless
    /// its timeouts ended together.
    pub(crate) fn explain(&self, failures: &[Failure]) -> Option<String> {
        let stall = find(failures)?;
        Some(explain(
            &stall,
            self.before.as_ref().ok(),
            &read_pressure(),
            self.started.elapsed(),
        ))
    }
}

/// What the pressure said of one run: its values when the run started and
/// when it ended, and the time between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Observed {
    pub(crate) before: Pressure,
    pub(crate) after: Pressure,
    pub(crate) elapsed: Duration,
}

impl Observed {
    /// The seconds in which every task waited on I/O during the run.
    pub(crate) fn stalled_seconds(&self) -> f64 {
        micros_to_seconds(self.after.full_total.saturating_sub(self.before.full_total))
    }

    /// The share of the run, in percent, in which every task waited on I/O.
    /// Nothing for a run that took no time.
    pub(crate) fn share(&self) -> f64 {
        percent(self.stalled_seconds(), self.elapsed.as_secs_f64())
    }

    /// What of the run the tier check needs ([`Record`]).
    pub(crate) fn record(&self) -> Record {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a run's milliseconds fit a u64"
        )]
        let elapsed_ms = self.elapsed.as_millis() as u64;
        Record {
            elapsed_ms,
            full_us: self.after.full_total.saturating_sub(self.before.full_total),
            tiers: EVERY_TIER,
        }
    }
}

/// The tiers a test run measured, as the bits of [`Record::tiers`]: the
/// job's first run (tiers 1 and 2), its second (tier 3), or a run of all.
pub(crate) const UNIT_AND_INTEGRATION: u8 = 0b011;
/// The job's second run: the system tests.
pub(crate) const SYSTEM: u8 = 0b100;
/// A run of every tier.
pub(crate) const EVERY_TIER: u8 = 0b111;

/// The bit of `tier` in [`Record::tiers`].
pub(crate) fn tier_bit(tier: crate::tiers::Tier) -> u8 {
    match tier {
        crate::tiers::Tier::Unit => 0b001,
        crate::tiers::Tier::Integration => 0b010,
        crate::tiers::Tier::System => 0b100,
    }
}

/// `tiers` as a record writes it: their numbers, `1,2`.
fn tiers_text(tiers: u8) -> String {
    crate::tiers::Tier::ALL
        .iter()
        .enumerate()
        .filter(|(_, tier)| tiers & tier_bit(**tier) != 0)
        .map(|(index, _)| (index + 1).to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The tiers of a record's `tiers=1,2`; `None` for anything else.
fn parse_tiers(text: &str) -> Option<u8> {
    let mut tiers = 0;
    for number in text.split(',') {
        tiers |= match number {
            "1" => 0b001,
            "2" => 0b010,
            "3" => 0b100,
            _ => return None,
        };
    }
    (tiers != 0).then_some(tiers)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "microseconds of a run, in seconds to a tenth"
)]
fn micros_to_seconds(micros: u64) -> f64 {
    micros as f64 / 1e6
}

/// `stalled` seconds as a percentage of `run` seconds: 0 for no time.
fn percent(stalled: f64, run: f64) -> f64 {
    if run > 0.0 {
        100.0 * stalled / run
    } else {
        0.0
    }
}

/// How long a run was and how long every task waited on I/O in it: one
/// line of the file the test runs of `cargo xtask ci` leave for its tier
/// check, which is another step and reads it ([`judge`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) elapsed_ms: u64,
    pub(crate) full_us: u64,
    /// The tiers the run measured ([`tier_bit`]): a stall excuses theirs
    /// alone.
    pub(crate) tiers: u8,
}

/// Where the records of a CI run in the tree at `root` are kept: beside its
/// report.
pub(crate) fn record_path(root: &Path) -> PathBuf {
    crate::workspace::nextest_report(root, "ci").with_file_name("pressure.txt")
}

/// Adds a run's record to the file at `path`.
pub(crate) fn append_record(path: &Path, record: &Record) -> crate::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    writeln!(
        file,
        "elapsed_ms={} full_us={} tiers={}",
        record.elapsed_ms,
        record.full_us,
        tiers_text(record.tiers)
    )
    .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// The records in a file's text. A line that is not one is left out, and
/// so are its tiers' excuse: a run with no record is judged. A line with no
/// `tiers` is of every tier.
pub(crate) fn parse_records(text: &str) -> Vec<Record> {
    text.lines()
        .filter_map(|line| {
            let word = |name: &str| {
                line.split_whitespace()
                    .find_map(|word| word.strip_prefix(name)?.strip_prefix('='))
            };
            let field = |name: &str| word(name)?.parse().ok();
            let tiers = match word("tiers") {
                None => EVERY_TIER,
                Some(text) => parse_tiers(text)?,
            };
            Some(Record {
                elapsed_ms: field("elapsed_ms")?,
                full_us: field("full_us")?,
                tiers,
            })
        })
        .collect()
}

/// The records a CI run left, none when there is no file.
pub(crate) fn read_records(path: &Path) -> Vec<Record> {
    std::fs::read_to_string(path)
        .map(|text| parse_records(&text))
        .unwrap_or_default()
}

/// What the stall of a run keeps its tiers from being judged by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stalled {
    /// The share of the runs' time, in percent, in which every task waited.
    pub(crate) share: f64,
    /// That time, in seconds.
    pub(crate) stalled_seconds: f64,
    /// The runs' time, in seconds.
    pub(crate) run_seconds: f64,
    /// The tiers those runs measured ([`tier_bit`]): the ones not judged.
    pub(crate) tiers: u8,
}

impl Stalled {
    /// Whether `tier` is still held to its budget: it was not measured by a
    /// run that stalled.
    pub(crate) fn judges(&self, tier: crate::tiers::Tier) -> bool {
        self.tiers & tier_bit(tier) == 0
    }

    /// What the tier check says in place of the verdict of the tiers it
    /// does not judge.
    pub(crate) fn message(&self) -> String {
        let tiers = tiers_text(self.tiers).replace(',', ", ");
        format!(
            "not judged: the machine stalled. Every task waited on I/O for {:.1} s of the test \
             runs' {:.0} s ({:.0}%, from {PRESSURE_FILE}; a run is taken to have stalled at \
             {STALLED_SHARE:.0}%), so the times of the tiers they ran ({tiers}) are the disk's \
             as much as the tests', and those tiers are not held to their budgets; any other \
             is. Run the tests again when the machine is quiet. CI's runners are always judged.",
            self.stalled_seconds, self.run_seconds, self.share
        )
    }
}

/// Whether the tiers of a run can be held to their budgets, given the
/// records of its test runs and where it ran: the stall when they cannot.
///
/// On CI (`in_actions`: `GITHUB_ACTIONS` is set) the tiers are always
/// judged, whatever the pressure: the budgets are the pull request's, a
/// hosted runner's disk is its own, and a verdict that moved with a reading
/// nobody can check there would be a loophole. Elsewhere, a run that
/// stalled for [`STALLED_SHARE`] or more of its time does not have the
/// tiers it ran judged, and one with no record (the pressure could not be
/// read, or nothing wrote one) is. The records of one set of tiers (a run
/// and its re-run) are one run; a stall in the system tests' run leaves
/// tiers 1 and 2, from another run, judged.
pub(crate) fn judge(in_actions: bool, records: &[Record]) -> Option<Stalled> {
    if in_actions {
        return None;
    }
    let mut runs: Vec<(u8, u64, u64)> = Vec::new();
    for record in records {
        match runs.iter_mut().find(|run| run.0 == record.tiers) {
            Some(run) => {
                run.1 += record.elapsed_ms;
                run.2 += record.full_us;
            }
            None => runs.push((record.tiers, record.elapsed_ms, record.full_us)),
        }
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "milliseconds of a run, in seconds to a tenth"
    )]
    let stalled = |(_, elapsed_ms, full_us): &(u8, u64, u64)| {
        percent(micros_to_seconds(*full_us), *elapsed_ms as f64 / 1e3) >= STALLED_SHARE
    };
    let stalled: Vec<&(u8, u64, u64)> = runs.iter().filter(|run| stalled(run)).collect();
    let tiers = stalled.iter().fold(0, |tiers, run| tiers | run.0);
    let elapsed_ms: u64 = stalled.iter().map(|run| run.1).sum();
    let full_us: u64 = stalled.iter().map(|run| run.2).sum();
    #[expect(
        clippy::cast_precision_loss,
        reason = "milliseconds of a run, in seconds to a tenth"
    )]
    let run_seconds = elapsed_ms as f64 / 1e3;
    let stalled_seconds = micros_to_seconds(full_us);
    let share = percent(stalled_seconds, run_seconds);
    (tiers != 0).then_some(Stalled {
        share,
        stalled_seconds,
        run_seconds,
        tiers,
    })
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

    use super::{EVERY_TIER, SYSTEM, UNIT_AND_INTEGRATION};
    use super::{
        Observed, Pressure, Record, Stalled, append_record, explain, find, find_cases, instant_ms,
        judge, parse_pressure, read_pressure_from, read_pressure_with, read_records,
    };
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
    fn timeouts_stopped_late_and_together_are_a_stall_whatever_time_each_started() {
        // Started across four seconds, stopped late and within 200 ms.
        let failures = [
            timeout("a", "2026-11-15T10:00:00.000+00:00", 14.0),
            timeout("b", "2026-11-15T10:00:02.000+00:00", 12.1),
            timeout("c", "2026-11-15T10:00:03.900+00:00", 10.3),
            // A panic beside them is not a timeout.
            Failure {
                kind: "test failure with exit code 101".to_owned(),
                ..timeout("d", "2026-11-15T10:00:13.900+00:00", 0.2)
            },
        ];
        let stall = find(&failures).expect("a stall");
        assert_eq!(stall.tests, ["cx::it a", "cx::it b", "cx::it c"]);
        assert_eq!(stall.spread_ms, 200);
    }

    #[test]
    fn tests_stopped_at_their_own_limits_are_not_a_stall_whatever_the_limits() {
        // A break that hangs a crate's tests: two system tests stopped at
        // their 60 s, three others started 50 s later stopped at their
        // 10 s, all in one moment, and every one on time.
        let on_time = [
            timeout("sys_a", "2026-11-15T10:00:00.000+00:00", 60.02),
            timeout("sys_b", "2026-11-15T10:00:00.003+00:00", 60.01),
            timeout("c", "2026-11-15T10:00:50.010+00:00", 10.01),
            timeout("d", "2026-11-15T10:00:50.020+00:00", 10.0),
            timeout("e", "2026-11-15T10:00:50.020+00:00", 10.02),
        ];
        assert_eq!(find(&on_time), None);
        // The same tests stopped late, together: the machine's.
        let late = [
            timeout("sys_a", "2026-11-15T10:00:00.000+00:00", 61.5),
            timeout("sys_b", "2026-11-15T10:00:00.003+00:00", 61.5),
            timeout("c", "2026-11-15T10:00:50.000+00:00", 11.5),
            timeout("d", "2026-11-15T10:00:49.000+00:00", 12.5),
            timeout("e", "2026-11-15T10:00:48.000+00:00", 13.5),
        ];
        assert_eq!(find(&late).map(|stall| stall.tests.len()), Some(5));
    }

    #[test]
    fn a_test_is_late_only_past_the_window_after_a_limit() {
        use super::stopped_late;
        assert!(!stopped_late(10_000));
        assert!(!stopped_late(10_250));
        assert!(stopped_late(10_251));
        assert!(!stopped_late(60_061), "nextest's own 61 ms past the limit");
        assert!(stopped_late(61_500));
        assert!(!stopped_late(300_010));
        assert!(!stopped_late(1_800_100));
        // Short of every limit: a timeout nextest's limits do not explain.
        assert!(stopped_late(4_000));
    }

    #[test]
    fn the_limits_are_those_nextest_is_given() {
        let configuration = include_str!("../../.config/nextest.toml");
        let mut found = Vec::new();
        for line in configuration.lines() {
            let Some(rest) = line.trim().strip_prefix("slow-timeout = { period = \"") else {
                continue;
            };
            let (period, rest) = rest.split_once("s\", terminate-after = ").expect("a limit");
            let count = rest.trim_end_matches(" }");
            let ms = period.parse::<i64>().unwrap() * 1000 * count.parse::<i64>().unwrap();
            found.push(ms);
        }
        assert!(!found.is_empty());
        for ms in found {
            assert!(
                super::LIMITS_MS.contains(&ms),
                "nextest stops a test at {ms} ms, which LIMITS_MS does not hold"
            );
        }
    }

    #[test]
    fn one_tests_timeout_is_not_a_stall_nor_are_timeouts_apart() {
        assert_eq!(
            find(&[timeout("a", "2026-11-15T10:00:00.000+00:00", 11.0)]),
            None
        );
        // Two together are not enough.
        assert_eq!(
            find(&[
                timeout("a", "2026-11-15T10:00:00.000+00:00", 11.0),
                timeout("b", "2026-11-15T10:00:00.100+00:00", 11.0),
            ]),
            None
        );
        // Three, a second apart each.
        assert_eq!(
            find(&[
                timeout("a", "2026-11-15T10:00:00.000+00:00", 11.0),
                timeout("b", "2026-11-15T10:00:01.000+00:00", 11.0),
                timeout("c", "2026-11-15T10:00:02.000+00:00", 11.0),
            ]),
            None
        );
        // Just outside the window.
        assert_eq!(
            find(&[
                timeout("a", "2026-11-15T10:00:00.000+00:00", 11.0),
                timeout("b", "2026-11-15T10:00:00.100+00:00", 11.0),
                timeout("c", "2026-11-15T10:00:00.251+00:00", 11.0),
            ]),
            None
        );
    }

    #[test]
    fn the_largest_group_is_the_one_named() {
        let failures = [
            timeout("early-1", "2026-11-15T09:59:58.000+00:00", 12.0),
            timeout("early-2", "2026-11-15T09:59:59.050+00:00", 11.0),
            timeout("early-3", "2026-11-15T09:59:59.090+00:00", 11.0),
            timeout("late-1", "2026-11-15T10:00:59.000+00:00", 11.0),
            timeout("late-2", "2026-11-15T10:00:58.010+00:00", 12.0),
            timeout("late-3", "2026-11-15T10:00:58.020+00:00", 12.0),
            timeout("late-4", "2026-11-15T10:00:58.249+00:00", 12.0),
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

    fn observed(before_total: u64, after_total: u64, elapsed: Duration) -> Observed {
        let pressure = |full_total| Pressure {
            some: (0.0, 0.0),
            full: (0.0, 0.0),
            some_total: full_total,
            full_total,
        };
        Observed {
            before: pressure(before_total),
            after: pressure(after_total),
            elapsed,
        }
    }

    #[test]
    fn a_runs_stall_is_the_share_of_its_own_time_every_task_waited() {
        // 30.2 s of 312 s.
        let run = observed(500_000, 30_700_000, Duration::from_secs(312));
        assert!((run.stalled_seconds() - 30.2).abs() < 1e-9);
        assert!((run.share() - 9.679_487).abs() < 1e-5, "{}", run.share());
        assert_eq!(
            run.record(),
            Record {
                elapsed_ms: 312_000,
                full_us: 30_200_000,
                tiers: EVERY_TIER,
            }
        );
        // A run that took no time stalled for none of it, and counters that
        // went back (a reboot) are no stall.
        assert_eq!(observed(0, 5, Duration::ZERO).share(), 0.0);
        assert_eq!(observed(9, 1, Duration::from_secs(10)).share(), 0.0);
    }

    #[test]
    fn a_run_that_stalled_for_a_tenth_of_its_time_is_not_judged_away_from_ci() {
        let record = |full_s: u64, elapsed_s: u64| Record {
            elapsed_ms: elapsed_s * 1000,
            full_us: full_s * 1_000_000,
            tiers: EVERY_TIER,
        };
        // Judged under the threshold, not judged at it, and over.
        assert_eq!(judge(false, &[record(9, 100)]), None);
        let at = judge(false, &[record(10, 100)]).expect("not judged at a tenth");
        assert!((at.share - 10.0).abs() < 1e-9);
        assert!(judge(false, &[record(60, 100)]).is_some());
        // Judged on CI whatever the pressure.
        assert_eq!(judge(true, &[record(90, 100)]), None);
        // Judged when there is nothing to read, or a run with no time.
        assert_eq!(judge(false, &[]), None);
        assert_eq!(judge(false, &[record(5, 0)]), None);
        // Two runs are one: 8 s of 40 and 14 s of 60 is 22 s of 100.
        let both = judge(false, &[record(8, 40), record(14, 60)]).expect("22%");
        assert!((both.share - 22.0).abs() < 1e-9);
        assert!((both.stalled_seconds - 22.0).abs() < 1e-9);
        assert!((both.run_seconds - 100.0).abs() < 1e-9);
        assert_eq!(both.tiers, EVERY_TIER);
    }

    #[test]
    fn a_stall_excuses_the_tiers_of_the_run_that_stalled_and_no_other() {
        let run = |full_s: u64, elapsed_s: u64, tiers: u8| Record {
            elapsed_ms: elapsed_s * 1000,
            full_us: full_s * 1_000_000,
            tiers,
        };
        use crate::tiers::Tier;
        // Tiers 1 and 2 quiet for 200 s, the system tests stalled 35 s of
        // 100: together 11.7%, but only tier 3 is excused.
        let stalled = judge(
            false,
            &[run(0, 200, UNIT_AND_INTEGRATION), run(35, 100, SYSTEM)],
        )
        .expect("the system tests' run stalled");
        assert_eq!(stalled.tiers, SYSTEM);
        assert!(stalled.judges(Tier::Unit) && stalled.judges(Tier::Integration));
        assert!(!stalled.judges(Tier::System));
        assert!((stalled.share - 35.0).abs() < 1e-9);
        assert!(
            stalled.message().contains("the tiers they ran (3)"),
            "{}",
            stalled.message()
        );
        // A run and its re-run are one run: 6 s of 50 and 6 s of 50 is 12%.
        let rerun = judge(
            false,
            &[
                run(6, 50, UNIT_AND_INTEGRATION),
                run(6, 50, UNIT_AND_INTEGRATION),
            ],
        )
        .expect("12% of the run and its re-run");
        assert_eq!(rerun.tiers, UNIT_AND_INTEGRATION);
        // Neither run stalled: everything judged.
        assert_eq!(
            judge(
                false,
                &[run(9, 100, UNIT_AND_INTEGRATION), run(9, 100, SYSTEM)]
            ),
            None
        );
    }

    #[test]
    fn the_message_of_a_run_that_is_not_judged_has_the_numbers() {
        let message = Stalled {
            share: 34.4,
            stalled_seconds: 41.3,
            run_seconds: 120.0,
            tiers: EVERY_TIER,
        }
        .message();
        assert!(
            message.starts_with(
                "not judged: the machine stalled. Every task waited on I/O for 41.3 s of the \
                 test runs' 120 s (34%"
            ),
            "{message}"
        );
        assert!(message.contains("stalled at 10%"), "{message}");
        assert!(
            message.contains("CI's runners are always judged"),
            "{message}"
        );
    }

    #[test]
    fn a_runs_record_is_kept_in_a_file_and_read_back_and_a_missing_file_is_none() {
        let folder = std::env::temp_dir().join(format!("xtask-records-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("pressure.txt");
        let first = Record {
            elapsed_ms: 1500,
            full_us: 250,
            tiers: UNIT_AND_INTEGRATION,
        };
        let second = Record {
            elapsed_ms: 99,
            full_us: 0,
            tiers: SYSTEM,
        };
        let missing = read_records(&path);
        append_record(&path, &first).unwrap();
        append_record(&path, &second).unwrap();
        let both = read_records(&path);
        std::fs::write(
            &path,
            "garbage\nelapsed_ms=1 full_us=2\nelapsed_ms=x full_us=2\nelapsed_ms=1 full_us=2 \
             tiers=4\n",
        )
        .unwrap();
        let tolerant = read_records(&path);
        std::fs::remove_dir_all(&folder).unwrap();
        assert!(missing.is_empty());
        assert_eq!(both, [first, second]);
        assert_eq!(
            tolerant,
            [Record {
                elapsed_ms: 1,
                full_us: 2,
                tiers: EVERY_TIER,
            }]
        );
    }

    #[test]
    fn a_pressure_can_be_read_from_the_file_a_variable_names() {
        let folder = std::env::temp_dir().join(format!("xtask-psi-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("io");
        std::fs::write(
            &path,
            "some avg10=12.50 avg60=3.10 avg300=0.80 total=41300000\n\
             full avg10=9.00 avg60=2.00 avg300=0.50 total=30200000\n",
        )
        .unwrap();
        let read = read_pressure_with(Some(path.clone().into_os_string()));
        let missing = read_pressure_from(&folder.join("nothing"));
        std::fs::write(&path, "nonsense").unwrap();
        let wrong = read_pressure_from(&path);
        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(read.unwrap().full_total, 30_200_000);
        assert!(missing.unwrap_err().contains("CINERIS_PRESSURE_FILE"));
        assert!(wrong.unwrap_err().contains("not in the form expected"));
    }

    #[test]
    fn the_cluster_is_found_with_the_places_of_its_cases() {
        let failures = [
            timeout("early", "2026-11-15T09:00:00.000+00:00", 11.0),
            timeout("a", "2026-11-15T10:00:00.000+00:00", 14.0),
            timeout("b", "2026-11-15T10:00:02.000+00:00", 12.1),
            timeout("c", "2026-11-15T10:00:03.900+00:00", 10.3),
        ];
        let (stall, places) = find_cases(&failures).expect("a stall");
        assert_eq!(places, [1, 2, 3]);
        assert_eq!(Some(stall), find(&failures));
    }
}
