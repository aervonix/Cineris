//! Local time: the date a moment fell on where the user is
//! (`docs/architecture/18-usage.md`, "Where our own numbers come from").
//!
//! The usage view counts by the local calendar day, week and month, so a
//! request is recorded with the local day it fell on, and a range is a run
//! of local days. This module answers the two questions that needs: what
//! the local date is at a moment ([`Zone::date_of`]), and when a local date
//! began ([`Zone::start_of`]).
//!
//! **Where the zone comes from** ([`Zone::system`]). On Linux, the `TZ`
//! variable as the C library reads it (a path, a name under
//! `/usr/share/zoneinfo`, or a rule such as `CET-1CEST,M3.5.0,M10.5.0/3`),
//! and without it `/etc/localtime`: a compiled zone file (TZif), read here,
//! with the rule at its end for the years after its last transition. On
//! Windows, the zone the system is set to, through its own API, which knows
//! each year's rules. Anything that cannot be read is UTC, and says so.
//!
//! **No file is read until asked.** [`Zone::system`] reads, and it blocks:
//! the statistics' writer thread calls it the first time it records, and a
//! command calls it when it reports. Nothing on the start-up path does.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "reading the zone file blocks; its callers are off the core thread"
)]

mod posix;
mod tzif;
#[cfg(windows)]
mod windows;

use std::fmt;

use crate::time::{UtcTime, civil_from_days, days_from_civil, days_in_month};

pub use posix::PosixRule;
pub use tzif::Transitions;

const SECS_PER_DAY: i64 = 86_400;

/// A date of the Gregorian calendar, with no time and no zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate {
    year: i32,
    month: u8,
    day: u8,
}

impl LocalDate {
    /// A date, when it is a real one between the years 0000 and 9999.
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        if !(0..=9999).contains(&year) || !(1..=12).contains(&month) || day == 0 {
            return None;
        }
        if i64::from(day) > days_in_month(i64::from(year), i64::from(month)) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// The date `days` after 1970-01-01.
    #[must_use]
    pub fn from_days(days: i64) -> Self {
        let (year, month, day) = civil_from_days(days);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a month is 1 to 12 and a day 1 to 31; the year is held to 0 to 9999 by its callers"
        )]
        Self {
            year: year.clamp(0, 9999) as i32,
            month: month as u8,
            day: day as u8,
        }
    }

    /// Days from 1970-01-01 to this date.
    #[must_use]
    pub fn days(self) -> i64 {
        days_from_civil(
            i64::from(self.year),
            i64::from(self.month),
            i64::from(self.day),
        )
    }

    /// The year.
    #[must_use]
    pub const fn year(self) -> i32 {
        self.year
    }

    /// The month, 1 to 12.
    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }

    /// The day of the month, from 1.
    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }

    /// The day of the week, counted from Monday as 0 to Sunday as 6.
    #[must_use]
    pub fn weekday(self) -> u8 {
        // 1970-01-01 was a Thursday, which is 3 counted from Monday.
        #[expect(clippy::cast_possible_truncation, reason = "a remainder of 7")]
        let weekday = (self.days() + 3).rem_euclid(7) as u8;
        weekday
    }

    /// The date `days` later (or earlier, for a negative number).
    #[must_use]
    pub fn add_days(self, days: i64) -> Self {
        Self::from_days(self.days() + days)
    }

    /// The first day of this date's month.
    #[must_use]
    pub const fn first_of_month(self) -> Self {
        Self {
            day: 1,
            ..self
        }
    }

    /// The month as `YYYY-MM`.
    #[must_use]
    pub fn month_text(self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }

    /// Reads `YYYY-MM-DD`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let number = |range: std::ops::Range<usize>| -> Option<u32> {
            let part = text.get(range)?;
            part.bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| part.parse().ok())?
        };
        let year = i32::try_from(number(0..4)?).ok()?;
        let month = u8::try_from(number(5..7)?).ok()?;
        let day = u8::try_from(number(8..10)?).ok()?;
        Self::new(year, month, day)
    }
}

impl fmt::Display for LocalDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A time zone: the offset from UTC at each moment.
#[derive(Debug, Clone)]
pub struct Zone {
    rule: Rule,
    /// How the zone is named to the user.
    name: String,
}

#[derive(Debug, Clone)]
enum Rule {
    /// The same offset always, in seconds east of UTC.
    Fixed(i32),
    /// A POSIX rule.
    Posix(PosixRule),
    /// A compiled zone file.
    Tzif(Transitions),
    /// The zone Windows is set to.
    #[cfg(windows)]
    Windows(windows::SystemZone),
}

impl Zone {
    /// UTC.
    #[must_use]
    pub fn utc() -> Self {
        Self {
            rule: Rule::Fixed(0),
            name: "UTC".to_owned(),
        }
    }

    /// The same offset always, in seconds east of UTC: `3600` is one hour
    /// ahead of it.
    #[must_use]
    pub fn fixed(offset_secs: i32) -> Self {
        let sign = if offset_secs < 0 { '-' } else { '+' };
        let abs = offset_secs.unsigned_abs();
        Self {
            rule: Rule::Fixed(offset_secs),
            name: format!("UTC{sign}{:02}:{:02}", abs / 3600, abs % 3600 / 60),
        }
    }

    /// A POSIX rule, as the `TZ` variable holds one: `EST5EDT,M3.2.0,M11.1.0`.
    #[must_use]
    pub fn posix(rule: &str) -> Option<Self> {
        Some(Self {
            rule: Rule::Posix(PosixRule::parse(rule)?),
            name: rule.to_owned(),
        })
    }

    /// A compiled zone file's bytes (RFC 8536), named `name`.
    #[must_use]
    pub fn from_tzif(bytes: &[u8], name: &str) -> Option<Self> {
        Some(Self {
            rule: Rule::Tzif(Transitions::parse(bytes)?),
            name: name.to_owned(),
        })
    }

    /// The zone this machine is set to, and UTC when it cannot be read.
    /// It reads a file on Linux, so it blocks.
    #[must_use]
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            windows::SystemZone::read().map_or_else(Self::utc, |zone| Self {
                rule: Rule::Windows(zone),
                name: "the system's zone".to_owned(),
            })
        }
        #[cfg(not(windows))]
        {
            unix_zone(std::env::var_os("TZ").as_deref()).unwrap_or_else(Self::utc)
        }
    }

    /// How the zone is named to the user.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The offset from UTC at `time`, in seconds east of UTC.
    #[must_use]
    pub fn offset_at(&self, time: UtcTime) -> i32 {
        let secs = time.unix_secs();
        match &self.rule {
            Rule::Fixed(offset) => *offset,
            Rule::Posix(rule) => rule.offset_at(secs),
            Rule::Tzif(transitions) => transitions.offset_at(secs),
            #[cfg(windows)]
            Rule::Windows(zone) => zone.offset_at(secs),
        }
    }

    /// The local date at `time`.
    #[must_use]
    pub fn date_of(&self, time: UtcTime) -> LocalDate {
        let local = time.unix_secs() + i64::from(self.offset_at(time));
        LocalDate::from_days(local.div_euclid(SECS_PER_DAY))
    }

    /// The moment `date` began here: its local midnight. Where midnight
    /// does not exist (a change of offset at midnight), the first moment of
    /// the date that does.
    #[must_use]
    pub fn start_of(&self, date: LocalDate) -> UtcTime {
        let midnight = date.days() * SECS_PER_DAY;
        // The offset in force at the moment sought is one of the offsets
        // either side of it. Try the one a day before, then the one after.
        let mut best = None;
        for probe in [midnight - SECS_PER_DAY, midnight, midnight + SECS_PER_DAY] {
            let probe_time = UtcTime::from_unix(probe, 0).unwrap_or(UtcTime::UNIX_EPOCH);
            let utc = midnight - i64::from(self.offset_at(probe_time));
            let at = UtcTime::from_unix(utc, 0).unwrap_or(UtcTime::UNIX_EPOCH);
            if self.date_of(at) == date {
                best = Some(best.map_or(at, |earlier: UtcTime| earlier.min(at)));
            }
        }
        best.unwrap_or_else(|| {
            UtcTime::from_unix(midnight - i64::from(self.offset_at(UtcTime::UNIX_EPOCH)), 0)
                .unwrap_or(UtcTime::UNIX_EPOCH)
        })
    }
}

/// The zone `TZ` names on a Unix system, or `/etc/localtime` without it.
/// `None` when nothing can be read.
#[cfg(not(windows))]
fn unix_zone(tz: Option<&std::ffi::OsStr>) -> Option<Zone> {
    use std::path::{Path, PathBuf};

    const ZONEINFO: &str = "/usr/share/zoneinfo";
    let read = |path: &Path, name: &str| -> Option<Zone> {
        let bytes = read_small(path)?;
        Zone::from_tzif(&bytes, name)
    };
    let Some(tz) = tz else {
        return read(Path::new("/etc/localtime"), "the system's zone");
    };
    let tz = tz.to_str()?;
    if tz.is_empty() {
        return Some(Zone::utc());
    }
    let named = tz.strip_prefix(':').unwrap_or(tz);
    if named.starts_with('/') {
        return read(Path::new(named), named);
    }
    // A name under the zone directory, never one that leaves it.
    let safe = !named.split('/').any(|part| part.is_empty() || part == "..");
    if safe && let Some(zone) = read(&PathBuf::from(ZONEINFO).join(named), named) {
        return Some(zone);
    }
    Zone::posix(tz)
}

/// A zone file's bytes, when it is a regular file of a sensible size.
#[cfg(not(windows))]
fn read_small(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    const LIMIT: u64 = 1024 * 1024;
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(LIMIT).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests;
