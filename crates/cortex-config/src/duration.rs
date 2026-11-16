//! Lengths of time as they are written in a configuration file: `"20s"`,
//! `"10m"`, `"1h30m"`, `"500ms"`.
//!
//! A duration is a string of one or more whole numbers, each followed by a
//! unit, with no spaces: `ms`, `s`, `m`, `h` or `d`. A bare number is
//! refused, so that nobody has to remember whether a key counts seconds or
//! milliseconds.

use std::time::Duration;

/// The units, largest first, with their length in milliseconds.
const UNITS: [(&str, u64); 5] = [
    ("d", 86_400_000),
    ("h", 3_600_000),
    ("m", 60_000),
    ("s", 1_000),
    ("ms", 1),
];

/// Reads a duration such as `"90s"` or `"1h30m"`.
///
/// # Errors
///
/// With a sentence that says what is expected, when the text is not a
/// duration or is too long to count in milliseconds.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    const EXPECTED: &str = "expected a length of time such as \"20s\", \"10m\" or \"1h30m\" \
                            (units: ms, s, m, h, d)";
    let mut rest = text;
    let mut total: u64 = 0;
    if rest.is_empty() {
        return Err(EXPECTED.to_owned());
    }
    while !rest.is_empty() {
        let digits = rest
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(rest.len());
        let (number, after) = rest.split_at(digits);
        let letters = after
            .find(|character: char| !character.is_ascii_lowercase())
            .unwrap_or(after.len());
        let (unit, after) = after.split_at(letters);
        let scale = UNITS
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, scale)| *scale);
        let (Ok(number), Some(scale)) = (number.parse::<u64>(), scale) else {
            return Err(EXPECTED.to_owned());
        };
        total = number
            .checked_mul(scale)
            .and_then(|part| total.checked_add(part))
            .ok_or_else(|| "the length of time is too long".to_owned())?;
        rest = after;
    }
    Ok(Duration::from_millis(total))
}

/// A bound of a range as a `settings!` declaration writes it, in the unit
/// the range is kept in: a length of time (`100ms`, `1m`, `1h30m`) in
/// milliseconds, any other number as it is written (`1_000`). `timed` says
/// which the setting is.
///
/// It is evaluated when the crate is built, so a bound that is neither, a
/// length of time written as a bare number, or a number written with a
/// unit, is an error of the build and never of a session.
///
/// # Panics
///
/// On such a bound, at compile time where it is called in a constant.
#[must_use]
pub(crate) const fn range_bound(text: &str, timed: bool) -> u64 {
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut total: u64 = 0;
    assert!(!bytes.is_empty(), "a range's bound is empty");
    while at < bytes.len() {
        let mut number: u64 = 0;
        let mut digits = 0;
        while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'_') {
            if bytes[at] != b'_' {
                let Some(tens) = number.checked_mul(10) else {
                    panic!("a range's bound is too large");
                };
                let Some(next) = tens.checked_add((bytes[at] - b'0') as u64) else {
                    panic!("a range's bound is too large");
                };
                number = next;
                digits += 1;
            }
            at += 1;
        }
        assert!(
            digits > 0,
            "a range's bound is a whole number, with a unit for a length of time"
        );
        let unit = at;
        while at < bytes.len() && bytes[at].is_ascii_lowercase() {
            at += 1;
        }
        if !timed {
            assert!(
                unit == at && at == bytes.len(),
                "the range of a setting that is not a length of time is written with no unit"
            );
            return number;
        }
        let scale = match unit_scale(bytes, unit, at) {
            Some(scale) => scale,
            None if unit == at => panic!(
                "the range of a length of time is written with its units, as the reference \
                 writes it: `100ms..=1m`"
            ),
            None => panic!("a range's bound has a unit that is not one of ms, s, m, h and d"),
        };
        let Some(part) = number.checked_mul(scale) else {
            panic!("a range's bound is too long a time");
        };
        let Some(sum) = total.checked_add(part) else {
            panic!("a range's bound is too long a time");
        };
        total = sum;
    }
    total
}

/// The length in milliseconds of the unit `bytes[start..end]`, as [`UNITS`]
/// has it, or `None` when it is not one of them.
const fn unit_scale(bytes: &[u8], start: usize, end: usize) -> Option<u64> {
    let mut index = 0;
    while index < UNITS.len() {
        let (name, scale) = UNITS[index];
        let name = name.as_bytes();
        if name.len() == end - start {
            let mut same = true;
            let mut offset = 0;
            while offset < name.len() {
                if name[offset] != bytes[start + offset] {
                    same = false;
                }
                offset += 1;
            }
            if same {
                return Some(scale);
            }
        }
        index += 1;
    }
    None
}

/// Writes a duration the way [`parse_duration`] reads it, with the largest
/// units that fit: 90 seconds is `1m30s`. Anything below a millisecond is
/// dropped.
#[must_use]
pub fn format_duration(duration: Duration) -> String {
    let mut left = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    if left == 0 {
        return "0s".to_owned();
    }
    let mut text = String::new();
    for (unit, scale) in UNITS {
        let count = left / scale;
        if count > 0 {
            text.push_str(&count.to_string());
            text.push_str(unit);
            left -= count * scale;
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_bound_is_read_in_its_settings_unit() {
        assert_eq!(range_bound("100ms", true), 100);
        assert_eq!(range_bound("0s", true), 0);
        assert_eq!(range_bound("2m", true), 120_000);
        assert_eq!(range_bound("1h30m", true), 5_400_000);
        assert_eq!(range_bound("1d", true), 86_400_000);
        assert_eq!(range_bound("60_000", false), 60_000);
        assert_eq!(
            range_bound("1_000_000_000_000_000", false),
            1_000_000_000_000_000
        );
        // Every bound as `parse_duration` reads the same text.
        for text in ["100ms", "1s", "30s", "1m", "5m", "1h", "1h30m", "1d"] {
            let millis = parse_duration(text).unwrap().as_millis();
            assert_eq!(u128::from(range_bound(text, true)), millis, "{text}");
        }
    }

    #[test]
    fn a_range_bound_in_the_wrong_unit_is_refused() {
        for (text, timed) in [
            ("100", true),
            ("100ms", false),
            ("1x", true),
            ("ms", true),
            ("", true),
            ("1m ", true),
            ("99999999999999999999", false),
        ] {
            let refused = std::panic::catch_unwind(|| range_bound(text, timed));
            assert!(refused.is_err(), "`{text}` (timed: {timed}) was taken");
        }
    }

    #[test]
    fn durations_read_and_write_back() {
        for (text, millis, canonical) in [
            ("0s", 0, "0s"),
            ("0ms", 0, "0s"),
            ("500ms", 500, "500ms"),
            ("20s", 20_000, "20s"),
            ("90s", 90_000, "1m30s"),
            ("10m", 600_000, "10m"),
            ("2m", 120_000, "2m"),
            ("1h30m", 5_400_000, "1h30m"),
            ("2h", 7_200_000, "2h"),
            ("1d", 86_400_000, "1d"),
            ("1d2h3m4s5ms", 93_784_005, "1d2h3m4s5ms"),
            ("1500ms", 1_500, "1s500ms"),
            ("36h", 129_600_000, "1d12h"),
        ] {
            let duration = parse_duration(text).unwrap();
            assert_eq!(duration, Duration::from_millis(millis), "{text}");
            assert_eq!(format_duration(duration), canonical, "{text}");
            assert_eq!(parse_duration(canonical).unwrap(), duration);
        }
    }

    #[test]
    fn what_is_not_a_duration_is_refused() {
        for text in [
            "",
            "20",
            "s",
            "20 s",
            "20S",
            "1.5s",
            "-5s",
            "20sec",
            "ten minutes",
            "5m ",
            " 5m",
            "5mm",
            "1h30",
            "+1s",
        ] {
            let error = parse_duration(text).unwrap_err();
            assert!(error.contains("length of time"), "{text}: {error}");
        }
        let error = parse_duration("99999999999999999999d").unwrap_err();
        assert!(error.contains("length of time"), "{error}");
        assert!(
            parse_duration("999999999999999999d")
                .unwrap_err()
                .contains("too long")
        );
    }

    #[test]
    fn a_length_too_long_to_count_is_written_as_the_longest_one() {
        let text = format_duration(Duration::MAX);
        assert_eq!(
            parse_duration(&text).unwrap(),
            Duration::from_millis(u64::MAX)
        );
    }
}
