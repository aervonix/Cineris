//! `cargo xtask perf-ab`: two trees' ignored `perf_` tests on one machine, run
//! in turn, and their numbers compared (`docs/testing-and-ci.md`, "Two trees'
//! performance tests on one runner").
//!
//! A number measured on a hosted runner today and the number measured on
//! another runner last week are not comparable: runners differ in speed by a
//! factor of two. `ab` takes the runner out of a comparison of tests' times;
//! this does the same for a measurement's own numbers. It builds the
//! performance tests of both trees once each, then runs them in turn on this
//! machine, `--count` rounds, the first round base then head and the next
//! head then base, so that what drifts during the job (a warmer disk, a
//! busier neighbour) falls on both alike. Each run's output is read for the
//! lines `perf: NAME VALUE UNIT` that a test prints with
//! `cortex_testkit::numbers::report`; a measurement's figure on a side is the
//! median over the rounds, and the report shows the range beside it, which is
//! the noise a difference has to stand out of.
//!
//! A run that fails ends the command, with its output printed: a performance
//! test fails by an assertion about what it measured or by a corpus that is
//! not there, and either is not something to average away.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{Value, json};

use crate::Result;
use crate::ab::{self, Which};
use crate::workspace;

const USAGE: &str = "\
usage: cargo xtask perf-ab --base DIR [--label NAME] [--count N]
                           [--cargo-profile release|dist] -p CRATE... [NAME]...

Builds the ignored `perf_` tests of the crates in this tree (the head) and in
the tree at DIR (the base), once each, as `cargo xtask perf -p CRATE` builds
them; runs them in turn on this machine, N rounds (3 when not given, 2 to 20)
in alternating order, one test at a time with their output shown; and compares
the measurements they print as lines `perf: NAME VALUE UNIT`
(`cortex_testkit::numbers::report`): each side's median over the rounds and its
range, the difference and whether the two sides' ranges are apart. A NAME
narrows the tests, as it does for `perf -p`. NAME is what the base is called in
the report (its directory when not given). `cargo xtask try` runs this on a
hosted runner with the input `against`, for `perf -p CRATE`.

The pinned corpus of real code must be in place (`cargo xtask corpus fetch`;
`try` does it first). A run that fails ends the command.";

/// Rounds when not told: a performance test takes long, and three give a
/// median and a range.
const ROUNDS: usize = 3;

/// The most rounds a run may ask for.
const MOST_ROUNDS: usize = 20;

/// What `cargo xtask perf-ab` was asked for.
#[derive(Debug, PartialEq)]
pub(crate) struct Request {
    pub(crate) base: PathBuf,
    pub(crate) label: Option<String>,
    pub(crate) rounds: usize,
    /// `release` or `dist`.
    pub(crate) profile: String,
    pub(crate) packages: Vec<String>,
    /// Test names that narrow the tests.
    pub(crate) names: Vec<String>,
}

impl Request {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut base = None;
        let mut request = Request {
            base: PathBuf::new(),
            label: None,
            rounds: ROUNDS,
            profile: "release".to_owned(),
            packages: Vec::new(),
            names: Vec::new(),
        };
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            let mut value = |what: &str| {
                words
                    .next()
                    .ok_or_else(|| format!("`{word}` needs {what}\n\n{USAGE}"))
            };
            match word.as_str() {
                "--base" => base = Some(PathBuf::from(value("a directory")?)),
                "--label" => request.label = Some(value("a name")?.clone()),
                "--count" => {
                    let text = value("a number")?;
                    request.rounds = text
                        .parse()
                        .ok()
                        .filter(|rounds| (2..=MOST_ROUNDS).contains(rounds))
                        .ok_or_else(|| {
                            format!("`--count {text}`: a number of rounds from 2 to {MOST_ROUNDS}")
                        })?;
                }
                "--cargo-profile" => {
                    let profile = value("release or dist")?;
                    if !["release", "dist"].contains(&profile.as_str()) {
                        return Err(format!(
                            "`--cargo-profile {profile}`: release or dist (a measurement is of an optimised build)"
                        ));
                    }
                    request.profile.clone_from(profile);
                }
                "-p" | "--package" => request.packages.push(value("a crate")?.clone()),
                "--help" | "-h" => return Err(USAGE.to_owned()),
                option if option.starts_with('-') => {
                    return Err(format!("unknown argument `{option}`\n\n{USAGE}"));
                }
                name => request.names.push(name.to_owned()),
            }
        }
        request.base = base.ok_or_else(|| format!("say which tree is the base\n\n{USAGE}"))?;
        if request.packages.is_empty() {
            return Err(format!(
                "name the crates whose `perf_` tests to compare, with -p\n\n{USAGE}"
            ));
        }
        Ok(request)
    }
}

/// Whether `cargo xtask perf-ab` would take `arguments`, which it reads and
/// runs nothing: how `try` refuses what it cannot pass on before it builds.
pub(crate) fn check_arguments(arguments: &[String]) -> Result<()> {
    Request::parse(arguments).map(|_| ())
}

/// The arguments of `cargo nextest run` for the crates' performance tests: the
/// build alone (`no_run`), or a run of them, one at a time, their output shown.
fn nextest_arguments(request: &Request, no_run: bool) -> Vec<String> {
    let mut command: Vec<String> = ["nextest", "run", "--locked", "--cargo-profile"]
        .map(str::to_owned)
        .to_vec();
    command.push(request.profile.clone());
    command.extend(["--profile", "perf"].map(str::to_owned));
    for package in &request.packages {
        command.extend(["-p".to_owned(), package.clone()]);
    }
    command.extend(workspace::implied_features(&request.packages));
    command.extend(["--run-ignored", "only", "-E", crate::try_run::PERF_TESTS].map(str::to_owned));
    if no_run {
        command.push("--no-run".to_owned());
    } else {
        command.extend(["--no-capture", "--test-threads", "1"].map(str::to_owned));
        command.extend(request.names.iter().cloned());
    }
    command
}

/// One measurement's figures in one run of one side, by name and unit: a name
/// that a run prints twice is the median of its values.
type Numbers = BTreeMap<(String, String), f64>;

/// The measurements in a run's output (`perf: NAME VALUE UNIT`, as
/// `cortex_testkit::numbers` writes them: the same rules, in a copy of their
/// own because `xtask` does not build the test kit).
pub(crate) fn numbers_in(output: &str) -> Numbers {
    let mut found: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();
    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("perf: ") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let (Some(name), Some(value), Some(unit), None) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            continue;
        };
        let Some(value) = value.parse::<f64>().ok().filter(|value| value.is_finite()) else {
            continue;
        };
        found
            .entry((name.to_owned(), unit.to_owned()))
            .or_default()
            .push(value);
    }
    found
        .into_iter()
        .map(|(key, values)| (key, median(values)))
        .collect()
}

/// The median of `values`, which are not empty.
pub(crate) fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values[middle]
    } else {
        f64::midpoint(values[middle - 1], values[middle])
    }
}

/// One measurement's figure in each round, on each side. A round in which a
/// side did not print it has no figure there.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Series {
    pub(crate) name: String,
    pub(crate) unit: String,
    pub(crate) base: Vec<f64>,
    pub(crate) head: Vec<f64>,
}

/// What a run found.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Results {
    /// What the base is called.
    pub(crate) label: String,
    pub(crate) profile: String,
    pub(crate) packages: Vec<String>,
    pub(crate) rounds: usize,
    pub(crate) series: Vec<Series>,
}

/// Puts the rounds' numbers together: `rounds[n]` is what the base and the
/// head printed in round `n`.
pub(crate) fn collect(rounds: &[(Numbers, Numbers)]) -> Vec<Series> {
    let mut by_key: BTreeMap<(String, String), Series> = BTreeMap::new();
    for (base, head) in rounds {
        for (numbers, side) in [(base, Which::Base), (head, Which::Head)] {
            for ((name, unit), value) in numbers {
                let series = by_key
                    .entry((name.clone(), unit.clone()))
                    .or_insert_with(|| Series {
                        name: name.clone(),
                        unit: unit.clone(),
                        base: Vec::new(),
                        head: Vec::new(),
                    });
                match side {
                    Which::Base => series.base.push(*value),
                    Which::Head => series.head.push(*value),
                }
            }
        }
    }
    by_key.into_values().collect()
}

/// Runs the rounds: `run(side)` runs one side's tests and returns what they
/// printed. The order alternates ([`ab::order`]); the first run that fails
/// ends it.
pub(crate) fn drive(
    rounds: usize,
    mut run: impl FnMut(Which, usize) -> Result<String>,
) -> Result<Vec<(Numbers, Numbers)>> {
    let mut all = Vec::new();
    for round in 0..rounds {
        let (mut base, mut head) = (Numbers::new(), Numbers::new());
        for which in ab::order(round) {
            let output = run(which, round)?;
            let numbers = numbers_in(&output);
            match which {
                Which::Base => base = numbers,
                Which::Head => head = numbers,
            }
        }
        all.push((base, head));
    }
    Ok(all)
}

/// A side's figures for one measurement: median, least and most.
fn spread(values: &[f64]) -> Option<(f64, f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let least = values.iter().copied().fold(f64::INFINITY, f64::min);
    let most = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some((median(values.to_vec()), least, most))
}

/// A figure with the digits that mean something at its size.
fn figure(value: f64) -> String {
    let size = value.abs();
    if size >= 1000.0 {
        format!("{value:.0}")
    } else if size >= 100.0 {
        format!("{value:.1}")
    } else if size >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

/// A side's cell: `median (least to most)`, or `-` when it printed none.
fn side_cell(values: &[f64]) -> String {
    match spread(values) {
        None => "-".to_owned(),
        Some((middle, _, _)) if values.len() == 1 => format!("{} (1 run)", figure(middle)),
        Some((middle, least, most)) => {
            format!("{} ({} to {})", figure(middle), figure(least), figure(most))
        }
    }
}

/// How a measurement moved, in words: the change of the median, and whether
/// the two sides' ranges are apart. With ranges that overlap, a difference
/// is not told from noise by this run (the rounds are few).
fn verdict(series: &Series) -> (String, &'static str) {
    let (Some((base, base_least, base_most)), Some((head, head_least, head_most))) =
        (spread(&series.base), spread(&series.head))
    else {
        let only = if series.base.is_empty() {
            "only on the head"
        } else {
            "only on the base"
        };
        return ("-".to_owned(), only);
    };
    let change = head - base;
    let percent = if base == 0.0 {
        String::new()
    } else {
        format!(" ({:+.1}%)", change / base * 100.0)
    };
    let apart = head_least > base_most || head_most < base_least;
    (
        format!(
            "{}{}{percent}",
            if change < 0.0 { '-' } else { '+' },
            figure(change.abs())
        ),
        if apart { "apart" } else { "within noise" },
    )
}

/// The report: a table of every measurement, then how to read it.
pub(crate) fn render(results: &Results) -> String {
    let mut rows: Vec<[String; 6]> = vec![[
        "measurement".to_owned(),
        "unit".to_owned(),
        format!("base ({})", results.label),
        "head (this tree)".to_owned(),
        "head - base".to_owned(),
        "ranges".to_owned(),
    ]];
    for series in &results.series {
        let (change, ranges) = verdict(series);
        rows.push([
            series.name.clone(),
            series.unit.clone(),
            side_cell(&series.base),
            side_cell(&series.head),
            change,
            ranges.to_owned(),
        ]);
    }
    let widths: Vec<usize> = (0..6)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut text = format!(
        "The `perf_` tests of {} on one machine, the head against {}: {} rounds, `{}` build.\n\n",
        results.packages.join(", "),
        results.label,
        results.rounds,
        results.profile
    );
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        let _ = writeln!(text, "{}", cells.join("  ").trim_end());
    }
    if results.series.is_empty() {
        text.push_str(
            "\nNo test printed a line `perf: NAME VALUE UNIT`, so nothing is compared: \
             `cortex_testkit::numbers::report` prints one.\n",
        );
    }
    text.push_str(
        "\nEach side's figure is the median of its rounds, with the least and the most beside it. \
         `apart` means the two sides' ranges do not overlap: the difference is more than this \
         run's own spread. `within noise` means they overlap, and this run cannot tell the \
         difference from the machine's. A figure that is better when lower (ms, bytes) or higher \
         (MB/s) is for the reader to say: nothing here judges.\n",
    );
    text
}

/// The results as the JSON kept for `try-result`.
pub(crate) fn to_json(results: &Results) -> Value {
    json!({
        "label": results.label,
        "profile": results.profile,
        "packages": results.packages,
        "rounds": results.rounds,
        "measurements": results.series.iter().map(|series| json!({
            "name": series.name,
            "unit": series.unit,
            "base": series.base,
            "head": series.head,
        })).collect::<Vec<_>>(),
    })
}

/// The results from the JSON [`to_json`] wrote.
pub(crate) fn from_json(text: &str) -> Result<Results> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("a perf-ab record: {error}"))?;
    let string = |value: &Value, key: &str| -> Result<String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("a perf-ab record with no `{key}`"))
    };
    let numbers = |value: &Value, key: &str| -> Result<Vec<f64>> {
        value
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("a perf-ab record with no `{key}`"))?
            .iter()
            .map(|number| {
                number
                    .as_f64()
                    .ok_or_else(|| format!("a perf-ab record: `{key}` holds a non-number"))
            })
            .collect()
    };
    let series = value
        .get("measurements")
        .and_then(Value::as_array)
        .ok_or("a perf-ab record with no `measurements`")?
        .iter()
        .map(|each| {
            Ok(Series {
                name: string(each, "name")?,
                unit: string(each, "unit")?,
                base: numbers(each, "base")?,
                head: numbers(each, "head")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Results {
        label: string(&value, "label")?,
        profile: string(&value, "profile")?,
        packages: value
            .get("packages")
            .and_then(Value::as_array)
            .map(|packages| {
                packages
                    .iter()
                    .filter_map(|package| package.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        rounds: value
            .get("rounds")
            .and_then(Value::as_u64)
            .and_then(|rounds| usize::try_from(rounds).ok())
            .ok_or("a perf-ab record with no `rounds`")?,
        series,
    })
}

/// Where the results are kept, for the workflow to upload: under the head's
/// `target`, whatever the caller's build directory.
pub(crate) fn record_path() -> PathBuf {
    workspace::root()
        .join("target")
        .join("perf-ab")
        .join("results.json")
}

/// The name `try-result` looks for in a run's artifact.
pub(crate) const RECORD_PATH_NAMES: [&str; 2] = ["perf-ab", "results.json"];

/// One run of `cargo` in `tree`: its standard output is shown as it comes and
/// is returned.
fn run_in(tree: &Path, arguments: &[String], what: &str) -> Result<String> {
    let mut child = ab::nextest(tree, arguments)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run cargo in {}: {error}", tree.display()))?;
    let mut output = String::new();
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let line = line.map_err(|error| format!("could not read the run's output: {error}"))?;
            println!("{line}");
            output.push_str(&line);
            output.push('\n');
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("could not wait for cargo: {error}"))?;
    if status.success() {
        Ok(output)
    } else {
        Err(format!("{what} failed in {} ({status})", tree.display()))
    }
}

/// `cargo xtask perf-ab …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    crate::checks::require_nextest()?;
    let head = workspace::root();
    let base = ab::normalised(
        &std::path::absolute(&request.base)
            .map_err(|error| format!("{}: {error}", request.base.display()))?,
    );
    if !base.join("Cargo.toml").is_file() {
        return Err(format!(
            "{} is not a checkout of the workspace",
            base.display()
        ));
    }
    ab::refuse_base(&head, &base)?;
    let label = request
        .label
        .clone()
        .unwrap_or_else(|| base.display().to_string());
    let tree = |which: Which| if which == Which::Base { &base } else { &head };
    let name = |which: Which| match which {
        Which::Base => format!("base ({label})"),
        Which::Head => "head (this tree)".to_owned(),
    };

    let build = nextest_arguments(&request, true);
    for which in [Which::Base, Which::Head] {
        println!("== building the {} ==", name(which));
        run_in(tree(which), &build, "the build")?;
    }
    let run_arguments = nextest_arguments(&request, false);
    let rounds = drive(request.rounds, |which, round| {
        println!(
            "== round {} of {}: the {} ==",
            round + 1,
            request.rounds,
            name(which)
        );
        run_in(tree(which), &run_arguments, "the performance tests")
    })?;
    let results = Results {
        label,
        profile: request.profile.clone(),
        packages: request.packages.clone(),
        rounds: request.rounds,
        series: collect(&rounds),
    };
    let table = render(&results);
    println!("\n{table}");
    crate::steps::append_job_summary(&format!(
        "### Performance tests of two trees on one runner\n\n```text\n{table}```\n"
    ));
    let path = record_path();
    std::fs::create_dir_all(path.parent().unwrap_or(&head))
        .and_then(|()| std::fs::write(&path, format!("{:#}\n", to_json(&results))))
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Numbers, Request, Results, Series, collect, drive, from_json, median, nextest_arguments,
        numbers_in, render, to_json, verdict,
    };
    use crate::ab::Which;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn key(name: &str, unit: &str) -> (String, String) {
        (name.to_owned(), unit.to_owned())
    }

    #[test]
    fn a_request_is_read_and_what_makes_no_sense_is_refused() {
        assert_eq!(
            Request::parse(&strings(&[
                "--base",
                "../ab-base",
                "--label",
                "main",
                "--count",
                "5",
                "--cargo-profile",
                "dist",
                "-p",
                "cortex-index",
                "-p",
                "cortex-tools",
                "warm",
            ]))
            .unwrap(),
            Request {
                base: "../ab-base".into(),
                label: Some("main".to_owned()),
                rounds: 5,
                profile: "dist".to_owned(),
                packages: strings(&["cortex-index", "cortex-tools"]),
                names: strings(&["warm"]),
            }
        );
        let plain = Request::parse(&strings(&["--base", "b", "-p", "c"])).unwrap();
        assert_eq!((plain.rounds, plain.profile.as_str()), (3, "release"));
        for (words, expected) in [
            (&["-p", "c"][..], "say which tree is the base"),
            (&["--base", "b"], "name the crates"),
            (&["--base", "b", "-p", "c", "--count", "1"], "from 2 to 20"),
            (&["--base", "b", "-p", "c", "--count", "21"], "from 2 to 20"),
            (
                &["--base", "b", "-p", "c", "--cargo-profile", "ci"],
                "release or dist",
            ),
            (
                &["--base", "b", "-p", "c", "--run-ignored"],
                "unknown argument",
            ),
            (&["--base", "b", "-p"], "needs a crate"),
        ] {
            let error = Request::parse(&strings(words)).unwrap_err();
            assert!(error.contains(expected), "{words:?}: {error}");
        }
    }

    #[test]
    fn the_build_and_the_run_are_the_crates_ignored_perf_tests_one_at_a_time() {
        let request = Request::parse(&strings(&[
            "--base",
            "b",
            "-p",
            "cortex-index",
            "warm",
            "cold",
        ]))
        .unwrap();
        assert_eq!(
            nextest_arguments(&request, true).join(" "),
            "nextest run --locked --cargo-profile release --profile perf -p cortex-index \
             --run-ignored only -E test(/(^|::)perf_/) --no-run"
        );
        assert_eq!(
            nextest_arguments(&request, false).join(" "),
            "nextest run --locked --cargo-profile release --profile perf -p cortex-index \
             --run-ignored only -E test(/(^|::)perf_/) --no-capture --test-threads 1 warm cold"
        );
    }

    #[test]
    fn the_numbers_a_run_printed_are_read_and_a_repeated_name_is_its_median() {
        let output = "\
running 1 test
perf: index.warm 31 ms\r
a table for a person: 1 2 3
perf: index.warm 29.5 ms
perf: index.warm 28.9 ms
perf: index.bytes 1000 bytes
perf: broken
perf: index.warm 1 ms extra
perf: index.warm x ms
perf: index.nan NaN ms
perf: index.inf inf ms
perf: index.rate 50.5 MB/s
";
        let numbers = numbers_in(output);
        assert_eq!(numbers.len(), 3, "{numbers:?}");
        assert_eq!(numbers[&key("index.warm", "ms")], 29.5);
        assert_eq!(numbers[&key("index.bytes", "bytes")], 1000.0);
        assert_eq!(numbers[&key("index.rate", "MB/s")], 50.5);
        // The test kit prints the same lines.
        assert_eq!(
            numbers_in("perf: a.b 2.5 us\n")[&key("a.b", "us")],
            2.5,
            "the line `cortex_testkit::numbers::line(\"a.b\", 2.5, \"us\")` makes"
        );
    }

    #[test]
    fn the_median_of_an_even_count_is_between_the_middle_two() {
        assert_eq!(median(vec![10.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(median(vec![3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(vec![7.0]), 7.0);
    }

    fn numbers(values: &[(&str, f64)]) -> Numbers {
        values
            .iter()
            .map(|(name, value)| (key(name, "ms"), *value))
            .collect()
    }

    #[test]
    fn the_sides_run_in_turn_and_the_numbers_are_kept_for_each_round() {
        let mut order = Vec::new();
        let rounds = drive(3, |which, round| {
            order.push((round, which));
            Ok(format!(
                "perf: x {} ms\n",
                if which == Which::Base { 10 } else { 20 } + round
            ))
        })
        .unwrap();
        assert_eq!(
            order,
            [
                (0, Which::Base),
                (0, Which::Head),
                (1, Which::Head),
                (1, Which::Base),
                (2, Which::Base),
                (2, Which::Head),
            ]
        );
        let series = collect(&rounds);
        assert_eq!(
            series,
            [Series {
                name: "x".to_owned(),
                unit: "ms".to_owned(),
                base: vec![10.0, 11.0, 12.0],
                head: vec![20.0, 21.0, 22.0],
            }]
        );
    }

    #[test]
    fn a_run_that_fails_ends_it_and_a_measurement_on_one_side_is_said() {
        let mut runs = 0;
        let error = drive(3, |_, _| {
            runs += 1;
            if runs == 3 {
                Err("the performance tests failed in ../ab-base".to_owned())
            } else {
                Ok(String::new())
            }
        })
        .unwrap_err();
        assert_eq!(runs, 3);
        assert!(error.contains("failed in ../ab-base"), "{error}");

        let series = collect(&[
            (
                numbers(&[("both", 1.0)]),
                numbers(&[("both", 1.0), ("new", 2.0)]),
            ),
            (
                numbers(&[("both", 1.0), ("old", 5.0)]),
                numbers(&[("both", 1.0)]),
            ),
        ]);
        let only: Vec<(&str, &str)> = series
            .iter()
            .map(|series| (series.name.as_str(), verdict(series).1))
            .collect();
        assert_eq!(
            only,
            [
                ("both", "within noise"),
                ("new", "only on the head"),
                ("old", "only on the base"),
            ]
        );
    }

    #[test]
    fn a_difference_is_apart_only_where_the_ranges_do_not_overlap() {
        let series = |base: &[f64], head: &[f64]| Series {
            name: "x".to_owned(),
            unit: "ms".to_owned(),
            base: base.to_vec(),
            head: head.to_vec(),
        };
        assert_eq!(
            verdict(&series(&[10.0, 10.2, 10.4], &[12.0, 12.2, 12.4])),
            ("+2.00 (+19.6%)".to_owned(), "apart")
        );
        // Faster by more than the spread is apart too, and signed.
        assert_eq!(
            verdict(&series(&[10.0, 10.2, 10.4], &[8.0, 8.1, 8.2])),
            ("-2.10 (-20.6%)".to_owned(), "apart")
        );
        // A head whose range reaches into the base's is not told from noise.
        assert_eq!(
            verdict(&series(&[10.0, 10.2, 11.0], &[10.9, 12.0, 12.4])).1,
            "within noise"
        );
        // One run a side: ranges of a point, and equal points overlap.
        assert_eq!(verdict(&series(&[3.0], &[3.0])).1, "within noise");
        // A base of nothing has no percentage.
        assert_eq!(
            verdict(&series(&[0.0, 0.0], &[1.0, 1.0])),
            ("+1.00".to_owned(), "apart")
        );
    }

    fn results() -> Results {
        Results {
            label: "main".to_owned(),
            profile: "release".to_owned(),
            packages: strings(&["cortex-index"]),
            rounds: 3,
            series: collect(&[
                (
                    numbers(&[("index.warm", 28.0)]),
                    numbers(&[("index.warm", 31.0)]),
                ),
                (
                    numbers(&[("index.warm", 29.0)]),
                    numbers(&[("index.warm", 32.5)]),
                ),
                (
                    numbers(&[("index.warm", 28.5)]),
                    numbers(&[("index.warm", 31.5)]),
                ),
            ]),
        }
    }

    #[test]
    fn the_report_gives_both_sides_the_difference_and_how_to_read_it() {
        let text = render(&results());
        assert!(
            text.starts_with(
                "The `perf_` tests of cortex-index on one machine, the head against main: 3 rounds, `release` build."
            ),
            "{text}"
        );
        let row = text
            .lines()
            .find(|line| line.starts_with("index.warm"))
            .expect("a row");
        assert!(row.contains("28.50 (28.00 to 29.00)"), "{row}");
        assert!(row.contains("31.50 (31.00 to 32.50)"), "{row}");
        assert!(row.contains("+3.00 (+10.5%)"), "{row}");
        assert!(row.ends_with("apart"), "{row}");
        assert!(text.contains("nothing here judges"), "{text}");
        let empty = render(&Results {
            series: Vec::new(),
            ..results()
        });
        assert!(
            empty.contains("No test printed a line `perf: NAME VALUE UNIT`"),
            "{empty}"
        );
    }

    #[test]
    fn the_record_reads_back_what_was_written() {
        let one = results();
        let json = format!("{:#}", to_json(&one));
        assert_eq!(from_json(&json).unwrap(), one);
        assert!(from_json("{\"label\": \"x\"}").is_err());
        assert!(from_json("not json").is_err());
    }
}
