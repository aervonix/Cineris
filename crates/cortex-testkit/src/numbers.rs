//! A measurement's numbers in a line a program can read (`docs/testing-and-ci.md`,
//! "Two trees' performance tests on one runner").
//!
//! A `perf_` test prints a table for a person, and a table is not something
//! `cargo xtask perf-ab` can compare between two trees. A test that wants its
//! numbers compared also prints each one it means to be held to with
//! [`report`]:
//!
//! ```text
//! cortex_testkit::numbers::report("index.warm_pass_one_changed", 28.9, "ms");
//! ```
//!
//! which prints the line
//!
//! ```text
//! perf: index.warm_pass_one_changed 28.9 ms
//! ```
//!
//! - **The name** is one word of letters, digits and `._-`, which says what is
//!   measured and starts with the subject (`search.literal_real_code`): names
//!   are compared across a run's tests and crates, so one must not be used for
//!   two things. A name that a run prints twice counts as the median of its
//!   values.
//! - **The value** is a finite number, written as Rust writes an `f64`.
//! - **The unit** is one word (`ms`, `us`, `MB/s`, `bytes`). It is only shown,
//!   and tells the reader which way is better: the comparison does not judge.
//! - **The line stands alone.** Anything else a test prints is for a person
//!   and is not read.
//!
//! The parser is [`parse`]; `xtask` has the same rules in its own copy
//! (`xtask/src/perf_ab.rs`, which does not depend on this crate), and a test
//! on each side holds the same lines.

use std::fmt::Write as _;

/// What a line begins with.
pub const PREFIX: &str = "perf: ";

/// The line [`report`] prints for a measurement, or the reason it cannot be
/// one: a name that is not one word of the characters allowed, a value that
/// is not finite, a unit that is not one word.
///
/// # Errors
///
/// When the name, the value or the unit does not follow the rules above.
pub fn line(name: &str, value: f64, unit: &str) -> Result<String, String> {
    let word = |text: &str| !text.is_empty() && !text.chars().any(char::is_whitespace);
    if !word(name)
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err(format!(
            "`{name}` is not a measurement's name: one word of letters, digits and `._-`"
        ));
    }
    if !value.is_finite() {
        return Err(format!("`{name}` is {value}: not a finite number"));
    }
    if !word(unit) {
        return Err(format!("`{unit}` is not a unit: one word, such as `ms`"));
    }
    // Six decimals, and no zeros after them: a time in microseconds, taken
    // as seconds, has no more than that, and `18.429197000000002` is noise.
    let shown = format!("{value:.6}");
    let shown = shown.trim_end_matches('0').trim_end_matches('.');
    let mut text = String::from(PREFIX);
    let _ = write!(text, "{name} {shown} {unit}");
    Ok(text)
}

/// Prints a measurement as a line `xtask perf-ab` reads. Panics, which fails
/// the test, when the name, the value or the unit does not follow the rules:
/// a number that would be left out of the comparison in silence is a bug of
/// the test.
#[allow(
    clippy::print_stdout,
    reason = "the line is the point of the module, and the runner keeps standard output"
)]
pub fn report(name: &str, value: f64, unit: &str) {
    match line(name, value, unit) {
        Ok(text) => println!("{text}"),
        Err(reason) => panic!("{reason}"),
    }
}

/// The measurements in a run's output: each line that [`report`] printed, as
/// its name, value and unit, in the order printed. A line that begins with
/// the prefix and is not one (a word missing, a value that is not a number)
/// is left out, as any other line is.
#[must_use]
pub fn parse(output: &str) -> Vec<(String, f64, String)> {
    output
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix(PREFIX)?;
            let mut words = rest.split_whitespace();
            let (name, value, unit) = (words.next()?, words.next()?, words.next()?);
            let value: f64 = value.parse().ok().filter(|value: &f64| value.is_finite())?;
            words
                .next()
                .is_none()
                .then(|| (name.to_owned(), value, unit.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{line, parse};

    #[test]
    fn a_measurement_is_a_line_that_reads_back() {
        let text = line("index.warm_pass", 28.9, "ms").unwrap();
        assert_eq!(text, "perf: index.warm_pass 28.9 ms");
        assert_eq!(
            parse(&format!("noise\n{text}\r\nmore noise")),
            [("index.warm_pass".to_owned(), 28.9, "ms".to_owned())]
        );
        assert_eq!(
            line("llm.decode", 112.0, "MB/s").unwrap(),
            "perf: llm.decode 112 MB/s"
        );
        // No figures that are only the float's.
        assert_eq!(
            line("a", 18.429_197_000_000_002, "ms").unwrap(),
            "perf: a 18.429197 ms"
        );
        assert_eq!(line("a", 0.0, "ms").unwrap(), "perf: a 0 ms");
    }

    #[test]
    fn what_is_not_a_measurement_is_refused_and_a_line_that_is_not_one_is_left_out() {
        for (name, value, unit) in [
            ("two words", 1.0, "ms"),
            ("", 1.0, "ms"),
            ("a/b", 1.0, "ms"),
            ("a", f64::NAN, "ms"),
            ("a", f64::INFINITY, "ms"),
            ("a", 1.0, "milli seconds"),
            ("a", 1.0, ""),
        ] {
            assert!(line(name, value, unit).is_err(), "{name} {value} {unit}");
        }
        let output = "perf: a\nperf: a x ms\nperf: a 1 ms extra\nperf: a NaN ms\n perf: b 2 s\nxperf: c 1 s\n";
        assert_eq!(parse(output), [("b".to_owned(), 2.0, "s".to_owned())]);
    }
}
