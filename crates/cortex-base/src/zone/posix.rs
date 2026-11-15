//! A POSIX time-zone rule, as `TZ` holds one and as a zone file ends with
//! one: `std offset [dst [offset] [,start[/time],end[/time]]]`.

use crate::time::{days_from_civil, days_in_month, is_leap_year};

const SECS_PER_DAY: i64 = 86_400;

/// A day of the year a change of offset falls on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Day {
    /// `Jn`: the day of the year from 1 to 365, 29 February never counted.
    Julian(u16),
    /// `n`: the day of the year from 0 to 365, 29 February counted.
    Ordinal(u16),
    /// `Mm.w.d`: day `d` (0 is Sunday) of week `w` (1 to 5, 5 the last) of
    /// month `m`.
    Month { month: u8, week: u8, weekday: u8 },
}

/// When a change happens: a day, and a time of that day in the local time
/// in force before the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Change {
    day: Day,
    secs: i64,
}

/// A rule with its daylight-saving part, if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PosixRule {
    /// Standard time, in seconds east of UTC.
    std_offset: i32,
    /// Daylight time, and when it starts and ends.
    dst: Option<(i32, Change, Change)>,
}

impl PosixRule {
    /// Reads a rule. `None` when it is not one this reader understands.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut rest = text.as_bytes();
        name(&mut rest)?;
        let std_offset = -offset(&mut rest)?;
        if rest.is_empty() {
            return Some(Self {
                std_offset,
                dst: None,
            });
        }
        name(&mut rest)?;
        let dst_offset = if rest.first().is_some_and(|byte| *byte != b',') {
            -offset(&mut rest)?
        } else {
            std_offset + 3600
        };
        let (start, end) = if rest.is_empty() {
            // No dates: the rule of the United States, which is what the C
            // library takes then.
            (
                Change {
                    day: Day::Month {
                        month: 3,
                        week: 2,
                        weekday: 0,
                    },
                    secs: 7200,
                },
                Change {
                    day: Day::Month {
                        month: 11,
                        week: 1,
                        weekday: 0,
                    },
                    secs: 7200,
                },
            )
        } else {
            expect(&mut rest, b',')?;
            let start = change(&mut rest)?;
            expect(&mut rest, b',')?;
            let end = change(&mut rest)?;
            (start, end)
        };
        if !rest.is_empty() {
            return None;
        }
        Some(Self {
            std_offset,
            dst: Some((dst_offset, start, end)),
        })
    }

    /// The offset at `secs` since the Unix epoch, in seconds east of UTC.
    #[must_use]
    pub fn offset_at(&self, secs: i64) -> i32 {
        let Some((dst_offset, start, end)) = self.dst else {
            return self.std_offset;
        };
        let local = secs + i64::from(self.std_offset);
        let (year, _, _) = crate::time::civil_from_days(local.div_euclid(SECS_PER_DAY));
        // The changes of this year, as moments in UTC. The start is read in
        // standard time, the end in daylight time.
        let start_utc =
            day_of(year, start.day) * SECS_PER_DAY + start.secs - i64::from(self.std_offset);
        let end_utc = day_of(year, end.day) * SECS_PER_DAY + end.secs - i64::from(dst_offset);
        let in_dst = if start_utc < end_utc {
            start_utc <= secs && secs < end_utc
        } else {
            // The southern hemisphere: daylight time spans the new year.
            !(end_utc <= secs && secs < start_utc)
        };
        if in_dst { dst_offset } else { self.std_offset }
    }
}

/// The days from 1970-01-01 to a change's day in `year`.
fn day_of(year: i64, day: Day) -> i64 {
    let first = days_from_civil(year, 1, 1);
    match day {
        Day::Julian(n) => {
            let n = i64::from(n);
            let leap_after = is_leap_year(year) && n >= 60;
            first + n - 1 + i64::from(leap_after)
        }
        Day::Ordinal(n) => first + i64::from(n),
        Day::Month {
            month,
            week,
            weekday,
        } => {
            let month_first = days_from_civil(year, i64::from(month), 1);
            // 1970-01-01 was a Thursday: 4 counted from Sunday.
            let first_weekday = (month_first + 4).rem_euclid(7);
            let mut day =
                1 + (i64::from(weekday) - first_weekday).rem_euclid(7) + (i64::from(week) - 1) * 7;
            let last = days_in_month(year, i64::from(month));
            while day > last {
                day -= 7;
            }
            month_first + day - 1
        }
    }
}

fn expect(rest: &mut &[u8], byte: u8) -> Option<()> {
    let (first, tail) = rest.split_first()?;
    (*first == byte).then(|| *rest = tail)
}

/// A zone's abbreviation: three letters or more, or anything in `<…>`.
fn name(rest: &mut &[u8]) -> Option<()> {
    if rest.first() == Some(&b'<') {
        let close = rest.iter().position(|byte| *byte == b'>')?;
        *rest = &rest[close + 1..];
        return Some(());
    }
    let length = rest
        .iter()
        .take_while(|byte| byte.is_ascii_alphabetic())
        .count();
    if length < 3 {
        return None;
    }
    *rest = &rest[length..];
    Some(())
}

/// `[+-]hh[:mm[:ss]]`, in seconds, as written: positive is west of UTC.
fn offset(rest: &mut &[u8]) -> Option<i32> {
    let secs = time(rest)?;
    i32::try_from(secs).ok()
}

/// `[+-]h[h][:mm[:ss]]`, in seconds.
fn time(rest: &mut &[u8]) -> Option<i64> {
    let sign = match rest.first() {
        Some(b'-') => {
            *rest = &rest[1..];
            -1
        }
        Some(b'+') => {
            *rest = &rest[1..];
            1
        }
        _ => 1,
    };
    let hours = number(rest, 3)?;
    if hours > 167 {
        return None;
    }
    let mut secs = hours * 3600;
    for scale in [60, 1] {
        if rest.first() != Some(&b':') {
            break;
        }
        *rest = &rest[1..];
        let part = number(rest, 2)?;
        if part > 59 {
            return None;
        }
        secs += part * scale;
    }
    Some(sign * secs)
}

/// Up to `digits` decimal digits, at least one.
fn number(rest: &mut &[u8], digits: usize) -> Option<i64> {
    let length = rest
        .iter()
        .take(digits)
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if length == 0 {
        return None;
    }
    let mut value = 0i64;
    for byte in &rest[..length] {
        value = value * 10 + i64::from(byte - b'0');
    }
    *rest = &rest[length..];
    Some(value)
}

/// `date[/time]`.
fn change(rest: &mut &[u8]) -> Option<Change> {
    let day = match rest.first()? {
        b'J' => {
            *rest = &rest[1..];
            let n = u16::try_from(number(rest, 3)?).ok()?;
            (1..=365).contains(&n).then_some(Day::Julian(n))?
        }
        b'M' => {
            *rest = &rest[1..];
            let month = u8::try_from(number(rest, 2)?).ok()?;
            expect(rest, b'.')?;
            let week = u8::try_from(number(rest, 1)?).ok()?;
            expect(rest, b'.')?;
            let weekday = u8::try_from(number(rest, 1)?).ok()?;
            ((1..=12).contains(&month) && (1..=5).contains(&week) && weekday <= 6).then_some(
                Day::Month {
                    month,
                    week,
                    weekday,
                },
            )?
        }
        _ => {
            let n = u16::try_from(number(rest, 3)?).ok()?;
            (n <= 365).then_some(Day::Ordinal(n))?
        }
    };
    let secs = if rest.first() == Some(&b'/') {
        *rest = &rest[1..];
        time(rest)?
    } else {
        7200
    };
    Some(Change { day, secs })
}
