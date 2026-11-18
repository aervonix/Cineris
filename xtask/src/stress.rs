//! `cargo xtask stress`: run tests many times over, each time under another
//! seed, and report the ones that fail on some
//! (`docs/testing-and-ci.md`, "A seeded order of events").
//!
//! A test that passes once may depend on an order of events that the run
//! happened to have. Two kinds of test are worth repeating:
//!
//! - tests whose rig seeds the order (`cineris-agent`'s): repeat `n` runs
//!   with seed `first + n`, so each repeat tries another order, and a
//!   failure names the seed that brings it back;
//! - tests that start processes or cross threads, where the machine decides
//!   the order: repeating is all there is, and the seed means nothing.
//!
//! The repeating is nextest's (`--stress-count`). This sets the first seed,
//! reads which repeat of which test failed from the JUnit report, and says
//! how to run a failure again.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::process::Command;

use crate::Result;
use crate::checks;
use crate::flaky::{self, Listed};
use crate::junit::{self, TestCase};
use crate::load::Squeeze;
use crate::stall;
use crate::stall::rerun::{self, Verdict};
use crate::workspace;

const USAGE: &str = "\
usage: cargo xtask stress -p CRATE [-p CRATE]... [--count N] [--first-seed S]
                          [--load N] [--cores K] [NEXTEST ARGUMENT]...

Runs the crates' tests N times over (100 when not given). Repeat n runs with
seed S + n (S is 0 when not given). Anything else goes to `cargo nextest run`:
a filter, `-E`, `--run-ignored`.

--cores K pins the run to K cores and --load N runs N busy loops beside it,
on the same cores: a race that needs the machine to be slow at the wrong
moment shows far more often. The tests are built first, unpinned.

A test that an open item names as flaky (`- Flaky:` in docs/outstanding/)
and that failed is named with its item and run once more, under the seed it
first failed with; if that passes, it is not a failure of the run. Every
such run is printed and counted. --strict runs nothing again: every failure
counts (for a session that is fixing a known flake).

A test that failed is run again in the same order with
    CINERIS_TEST_SEED=<seed> cargo xtask test -p <crate> -E 'test(=<name>)'";

/// The variable a seeded test rig reads the first seed from
/// (`cineris_testkit::seed::FIRST_SEED`).
const FIRST_SEED: &str = "CINERIS_TEST_FIRST_SEED";

/// The variable that fixes one seed (`cineris_testkit::seed::SEED`).
const SEED: &str = "CINERIS_TEST_SEED";

/// How many times each test is run when the count is not given.
const COUNT: u32 = 100;

/// What `cargo xtask stress` was asked for.
#[derive(Debug, PartialEq)]
struct Request {
    packages: Vec<String>,
    count: u32,
    first_seed: u64,
    /// The cores to pin the run to, and the busy loops beside it.
    squeeze: Squeeze,
    /// Arguments for nextest.
    rest: Vec<String>,
    /// `--strict`: a known flake is not run again.
    strict: bool,
}

impl Request {
    fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Self {
            packages: Vec::new(),
            count: COUNT,
            first_seed: 0,
            squeeze: Squeeze::default(),
            rest: Vec::new(),
            strict: false,
        };
        let mut arguments = arguments.iter();
        while let Some(argument) = arguments.next() {
            if request.squeeze.take(argument, &mut arguments)? {
                continue;
            }
            let mut value = |what: &str| {
                arguments
                    .next()
                    .ok_or_else(|| format!("`{argument}` needs {what}\n\n{USAGE}"))
            };
            match argument.as_str() {
                // What follows is for the test binaries, whatever it looks like.
                "--" => {
                    request.rest.push(argument.clone());
                    request.rest.extend(arguments.cloned());
                    break;
                }
                "-p" | "--package" => request.packages.push(value("a crate")?.clone()),
                joined if joined.starts_with("--package=") => {
                    request
                        .packages
                        .push(joined["--package=".len()..].to_owned());
                }
                joined if joined.starts_with("-p") && !joined.starts_with("--") => {
                    request.packages.push(joined[2..].to_owned());
                }
                "--count" => {
                    let text = value("a number")?;
                    request.count = text
                        .parse()
                        .ok()
                        .filter(|count| *count > 0)
                        .ok_or_else(|| format!("`--count {text}` is not a number above 0"))?;
                }
                "--first-seed" => {
                    let text = value("a number")?;
                    request.first_seed = text
                        .parse()
                        .map_err(|_| format!("`--first-seed {text}` is not a whole number"))?;
                }
                "--strict" => request.strict = true,
                "--help" | "-h" => return Err(USAGE.to_owned()),
                "--stress-count" | "--stress-duration" | "--profile" | "-P" => {
                    return Err(format!(
                        "`{argument}` is set by this command. Use `--count`.\n\n{USAGE}"
                    ));
                }
                _ => request.rest.push(argument.clone()),
            }
        }
        if request.packages.is_empty() {
            return Err(format!(
                "say which crates to stress with `-p`. The whole workspace a hundred times \
                 over is not a question worth its time.\n\n{USAGE}"
            ));
        }
        Ok(request)
    }

    /// The arguments of `cargo` for this request.
    /// The build of the tests the run will use, with nothing run: the
    /// command's arguments with `--no-run` before what goes to nextest.
    fn build_command(&self) -> Vec<String> {
        let mut command: Vec<String> = ["nextest", "run", "--profile", "stress", "--no-run"]
            .map(str::to_owned)
            .to_vec();
        for package in &self.packages {
            command.extend(["-p".to_owned(), package.clone()]);
        }
        command.extend(crate::workspace::implied_features(&self.packages));
        command.extend(self.rest.iter().take_while(|word| *word != "--").cloned());
        command
    }

    fn command(&self) -> Vec<String> {
        let mut command: Vec<String> = ["nextest", "run", "--profile", "stress", "--stress-count"]
            .map(str::to_owned)
            .to_vec();
        command.push(self.count.to_string());
        for package in &self.packages {
            command.extend(["-p".to_owned(), package.clone()]);
        }
        command.extend(crate::workspace::implied_features(&self.packages));
        command.extend(self.rest.iter().cloned());
        command
    }

    /// One run of `tests`, in place of the request's own filters, with the
    /// rest of what the request gives nextest and no retry
    /// ([`flaky::once`]).
    fn once_command(&self, tests: &[rerun::Test]) -> Vec<String> {
        let mut command: Vec<String> = ["nextest", "run", "--profile", "stress"]
            .map(str::to_owned)
            .to_vec();
        for package in &self.packages {
            command.extend(["-p".to_owned(), package.clone()]);
        }
        command.extend(crate::workspace::implied_features(&self.packages));
        command.extend(self.rest.iter().cloned());
        flaky::once(&command, tests)
    }
}

impl Request {
    /// cargo, as this request starts it: with the first seed, and without a
    /// seed that is fixed. One seed for every repeat would make every repeat
    /// the same run, and the run would say that nothing fails.
    fn cargo(&self, root: &std::path::Path) -> Command {
        let mut cargo = Command::new(workspace::cargo());
        cargo
            .args(self.command())
            .env(FIRST_SEED, self.first_seed.to_string())
            .env_remove(SEED)
            .current_dir(root);
        cargo
    }
}

/// Whether `cargo xtask stress` would take `arguments`, which it reads and
/// runs nothing: how `try` refuses what it cannot pass on before it builds.
pub(crate) fn check_arguments(arguments: &[String]) -> Result<()> {
    Request::parse(arguments).map(|_| ())
}

/// `cargo xtask stress`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    checks::require_nextest()?;
    let root = workspace::root();
    let report = workspace::nextest_report(&root, "stress");
    // A report left by the run before must not be read as this run's.
    workspace::forget_report(&report)?;

    // Built before the squeeze, so that two pinned cores are spent on the
    // tests and not on the compiler.
    if request.squeeze != Squeeze::default() {
        let mut build = Command::new(workspace::cargo());
        build.args(request.build_command()).current_dir(&root);
        let built = build
            .status()
            .map_err(|error| format!("could not run cargo nextest: {error}"))?;
        if !built.success() {
            return Err("the tests did not build".to_owned());
        }
    }
    let _load = request.squeeze.start()?;
    let watch = stall::Watch::start();
    println!(
        "$ {FIRST_SEED}={} cargo {}",
        request.first_seed,
        request.command().join(" ")
    );
    let status = request
        .cargo(&root)
        .status()
        .map_err(|error| format!("could not run cargo nextest: {error}"))?;

    let cases = junit::read(&report).map_err(|error| {
        format!("the stress run left no report to read ({error}). Did it build?")
    })?;
    // Read again for the failures' text: `read` has checked it is there.
    let xml = std::fs::read_to_string(&report).unwrap_or_default();
    let shown = report
        .strip_prefix(&root)
        .unwrap_or(&report)
        .display()
        .to_string();
    let failures = junit::failures(&xml);
    let summary = summarise(&cases, &failures, &shown, request.count, request.first_seed);
    print!("{}", summary.text);
    if let Some(stalled) = watch.explain(&failures) {
        print!("\n{stalled}");
    }
    let excused = if summary.failing > 0 {
        let (listed, warning) = flaky::listed(&root);
        if let Some(warning) = warning {
            println!("\nwarning: {warning}");
        }
        let mut run_once = |arguments: &[String], seed: u64| -> Result<bool> {
            Command::new(workspace::cargo())
                .args(arguments)
                .env(SEED, seed.to_string())
                .env_remove(FIRST_SEED)
                .current_dir(&root)
                .status()
                .map(|status| status.success())
                .map_err(|error| format!("could not run cargo nextest: {error}"))
        };
        known_flakes(
            &request,
            &listed,
            status.code(),
            (&report, &xml),
            &cases,
            &mut run_once,
            &mut |line| println!("{line}"),
        )?
    } else {
        0
    };
    conclude(&summary, status.success(), request.count, excused)
}

/// The known flakes among the tests that failed on some repeat of `cases`,
/// each with the seed of the first repeat it failed on, in the order of
/// their names.
fn flakes_to_run(cases: &[TestCase], listed: &[Listed], first_seed: u64) -> Vec<(Listed, u64)> {
    let mut first: BTreeMap<(&str, &str), u32> = BTreeMap::new();
    for case in cases.iter().filter(|case| case.failed || case.retried) {
        let repeat = case.repeat.unwrap_or(0);
        let entry = first
            .entry((case.binary.as_str(), case.name.as_str()))
            .or_insert(repeat);
        *entry = (*entry).min(repeat);
    }
    first
        .into_iter()
        .filter_map(|((binary, name), repeat)| {
            flaky::find(listed, binary, name)
                .map(|one| (one.clone(), first_seed.wrapping_add(u64::from(repeat))))
        })
        .collect()
}

/// The known flakes that failed on some repeat, named, and each run once
/// more under the seed it first failed with (`run_once` runs the arguments
/// with the seed, and says whether nextest succeeded): how many passed.
/// Nothing is run with `--strict`, nor when the stress run's nextest ended
/// in another way than with failed tests (`code`), which its report's
/// failures would not explain. The report at `report` is left as the
/// stress run wrote it (`xml`).
fn known_flakes(
    request: &Request,
    listed: &[Listed],
    code: Option<i32>,
    (report, xml): (&std::path::Path, &str),
    cases: &[TestCase],
    run_once: &mut dyn FnMut(&[String], u64) -> Result<bool>,
    say: &mut dyn FnMut(&str),
) -> Result<usize> {
    let flakes = flakes_to_run(cases, listed, request.first_seed);
    if flakes.is_empty() {
        return Ok(0);
    }
    say("");
    for (flake, _) in &flakes {
        say(&flaky::named(flake));
    }
    if request.strict {
        say("--strict: not run again, so each counts as a failure of this run.");
        return Ok(0);
    }
    if let Some(refusal) = flaky::refusal_by_codes(&[code]) {
        say(&format!(
            "Not run again: {refusal}. Each counts as a failure of this run."
        ));
        return Ok(0);
    }
    let mut passed = 0;
    let ran = (|| -> Result<()> {
        for (flake, seed) in &flakes {
            let arguments = request.once_command(std::slice::from_ref(&flake.test));
            say(&format!(
                "Running {} {} once more, under seed {seed}, the first it failed with:\n$ {SEED}={seed} \
             cargo {}",
                flake.test.binary,
                flake.test.name,
                arguments.join(" ")
            ));
            // A report the run before left must not be read as this run's.
            workspace::forget_report(report)?;
            let succeeded = run_once(&arguments, *seed)?;
            let verdict = std::fs::read_to_string(report).map_or(Verdict::Missing, |second| {
                rerun::verdicts(std::slice::from_ref(&flake.test), &second)[0]
            });
            let ok = succeeded && verdict == Verdict::Passed;
            say(&format!(
                "  {} {} {}",
                flake.test.binary,
                flake.test.name,
                if ok || verdict != Verdict::Passed {
                    flaky::says(verdict)
                } else {
                    "passed, but nextest failed: counted as a failure"
                }
            ));
            passed += usize::from(ok);
        }
        Ok(())
    })();
    // The report the summary pointed at is the stress run's, whatever
    // happened to the runs once more.
    std::fs::write(report, xml)
        .map_err(|error| format!("could not write {} back: {error}", report.display()))?;
    ran?;
    say(&format!(
        "{passed} of {} known flake(s) passed when run once more, and are not counted as \
         failures of this run; a known flake that failed again is.",
        flakes.len()
    ));
    Ok(passed)
}

/// Whether the run found nothing: no test failed on any repeat, nextest
/// agrees, and every repeat that was asked for is in the report.
/// `excused` of the failing tests are known flakes that passed when run
/// once more: they are not failures, and nextest's failure is theirs.
fn conclude(summary: &Summary, nextest_passed: bool, count: u32, excused: usize) -> Result<()> {
    let failing = summary.failing.saturating_sub(excused);
    if failing > 0 {
        let besides = if excused > 0 {
            format!(", besides {excused} known flake(s) that passed when run once more")
        } else {
            String::new()
        };
        return Err(format!("{failing} test(s) failed on some repeat{besides}"));
    }
    if !nextest_passed && summary.failing == 0 {
        return Err(
            "nextest failed, and its report names no failed test. See its output above.".to_owned(),
        );
    }
    if summary.repeats_seen != count {
        return Err(format!(
            "the report has {} repeat(s) where {count} were asked for",
            summary.repeats_seen
        ));
    }
    Ok(())
}

/// What a stress run's kept report says, as the run printed it, and the
/// number of tests that failed on some repeat. `try-result` prints it for a
/// `try` run: the repeats are those the report holds, and `first_seed` is
/// the log's.
pub(crate) fn report_summary(xml: &str, shown: &str, first_seed: u64) -> (String, usize) {
    let cases = junit::parse(xml);
    let repeats = cases
        .iter()
        .map(|case| case.repeat.unwrap_or(0) + 1)
        .max()
        .unwrap_or(1);
    let summary = summarise(&cases, &junit::failures(xml), shown, repeats, first_seed);
    (summary.text, summary.failing)
}

/// What a stress run came to.
struct Summary {
    text: String,
    /// The tests that failed at least once.
    failing: usize,
    /// How many repeats the report holds.
    repeats_seen: u32,
}

/// How many seeds of a failing test are listed.
const SEEDS_SHOWN: usize = 12;

/// How the test kit's timeout report begins (`cineris_testkit::watch`).
const WATCH_REPORT: &str = "== cineris-testkit: this test has run";

/// How many lines of a failure's text are shown.
pub(crate) const LINES_SHOWN: usize = 6;

/// How many characters of one line are shown.
const LINE_WIDTH: usize = 160;

/// How many other failures of one test, with another text, are named.
const OTHERS_SHOWN: usize = 2;

/// A failure's text as it is shown: its lines without the empty ones and
/// the note about backtraces, the panic's thread named no more (the test
/// is), each line cut to [`LINE_WIDTH`]; a timeout says so and shows what
/// the test wrote to standard error, where the test kit's timeout report
/// is. The count is of every line there was.
pub(crate) fn failure_lines(failure: &junit::Failure) -> (Vec<String>, usize) {
    let mut lines: Vec<String> = Vec::new();
    let timed_out = failure.kind.contains("timeout");
    let source = if timed_out {
        lines.push(format!("timed out ({})", failure.kind));
        // The test kit's timeout report comes late, after whatever else
        // the test wrote: it starts there when it is there.
        match failure.stderr.find(WATCH_REPORT) {
            Some(at) => &failure.stderr[at..],
            None => &failure.stderr,
        }
    } else if failure.text.trim().is_empty() {
        lines.push(if failure.kind.is_empty() {
            "failed, and the report keeps no text for it".to_owned()
        } else {
            format!("{}, and the report keeps no text for it", failure.kind)
        });
        &failure.stderr
    } else {
        &failure.text
    };
    for line in source.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.starts_with("note: run with `RUST_BACKTRACE") {
            continue;
        }
        // `thread 'a::b' (1234) panicked at src/x.rs:1:2:`
        let line = match line.strip_prefix("thread '") {
            Some(rest) => match rest.find(" panicked at ") {
                Some(at) => &rest[at + 1..],
                None => line,
            },
            None => line,
        };
        lines.push(cut(line));
    }
    let all = lines.len();
    lines.truncate(LINES_SHOWN);
    (lines, all)
}

/// `line`, or its first [`LINE_WIDTH`] characters and an ellipsis.
fn cut(line: &str) -> String {
    match line.char_indices().nth(LINE_WIDTH) {
        Some((at, _)) => format!("{} …", &line[..at]),
        None => line.to_owned(),
    }
}

/// What tells two failures' texts apart: their lines with the digits
/// taken out, which differ from one repeat to the next (a seed, a process
/// id, a time) when the failure is the same.
fn likeness(lines: &[String]) -> String {
    lines
        .iter()
        .flat_map(|line| line.chars().filter(|c| !c.is_ascii_digit()))
        .collect()
}

/// The text shown under a failing test: its first failure's lines, then
/// the first line of up to [`OTHERS_SHOWN`] failures whose text is not
/// like it, and where the rest is.
fn explain(failures: &[&junit::Failure], first_seed: u64, report: &str) -> String {
    let mut text = String::new();
    let mut ordered = failures.to_vec();
    ordered.sort_by_key(|failure| failure.repeat.unwrap_or(0));
    let Some(first) = ordered.first() else {
        let _ = writeln!(
            text,
            "  The report keeps no text for its failures: {report}"
        );
        return text;
    };
    let seed_of =
        |failure: &junit::Failure| first_seed.wrapping_add(u64::from(failure.repeat.unwrap_or(0)));
    let (lines, all) = failure_lines(first);
    let _ = writeln!(text, "  failure, under seed {}:", seed_of(first));
    for line in &lines {
        let _ = writeln!(text, "    | {line}");
    }
    if all > lines.len() {
        let _ = writeln!(text, "    | … and {} more line(s)", all - lines.len());
    }
    let mut seen = vec![likeness(&lines)];
    let mut unlike = 0;
    for failure in &ordered[1..] {
        let (lines, _) = failure_lines(failure);
        let key = likeness(&lines);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        unlike += 1;
        if unlike <= OTHERS_SHOWN {
            let _ = writeln!(
                text,
                "  another failure, under seed {}: {}",
                seed_of(failure),
                lines.first().map_or("", String::as_str)
            );
        }
    }
    if unlike > OTHERS_SHOWN {
        let _ = writeln!(
            text,
            "  and {} more failure(s) with other texts",
            unlike - OTHERS_SHOWN
        );
    }
    let _ = writeln!(text, "  all of it: {report}");
    text
}

/// Reads a stress run's report: for each test that failed, on how many of
/// its repeats, with which seeds, what its failure said, and how to run one
/// of them again.
fn summarise(
    cases: &[TestCase],
    failures: &[junit::Failure],
    report: &str,
    count: u32,
    first_seed: u64,
) -> Summary {
    // For each test: how often it ran, and the repeats it failed on.
    let mut tests: BTreeMap<(String, String), (u32, Vec<u32>)> = BTreeMap::new();
    let mut repeats_seen = 0;
    for case in cases {
        // A report of a run that was not a stress run has one repeat.
        let repeat = case.repeat.unwrap_or(0);
        repeats_seen = repeats_seen.max(repeat + 1);
        let entry = tests
            .entry((case.binary.clone(), case.name.clone()))
            .or_default();
        entry.0 += 1;
        // A test that passed only when it was tried again failed this
        // repeat: nextest gives some tests a retry, and a test that needs
        // it some of the time is what this command is looking for.
        if case.failed || case.retried {
            entry.1.push(repeat);
        }
    }

    let mut text = String::new();
    let last_seed = first_seed.wrapping_add(u64::from(count.saturating_sub(1)));
    let _ = writeln!(
        text,
        "\n{} test(s), {count} time(s) each, with seeds {first_seed} to {last_seed}.",
        tests.len()
    );
    let failing: Vec<_> = tests
        .iter()
        .filter(|(_, (_, failed))| !failed.is_empty())
        .collect();
    if failing.is_empty() {
        let _ = writeln!(text, "Every test passed every time.");
    }
    for ((binary, name), (ran, failed)) in &failing {
        let mut failed = failed.clone();
        failed.sort_unstable();
        let seeds: Vec<String> = failed
            .iter()
            .take(SEEDS_SHOWN)
            .map(|repeat| first_seed.wrapping_add(u64::from(*repeat)).to_string())
            .collect();
        let more = failed.len().saturating_sub(SEEDS_SHOWN);
        let _ = writeln!(
            text,
            "\nFAILED {} of {ran} times: {binary} {name}",
            failed.len()
        );
        if failed.len() == usize::try_from(*ran).unwrap_or(usize::MAX) {
            let _ = writeln!(
                text,
                "  It fails every time: the order of events is not what it depends on."
            );
        }
        let _ = writeln!(
            text,
            "  seeds: {}{}",
            seeds.join(", "),
            if more > 0 {
                format!(", and {more} more")
            } else {
                String::new()
            }
        );
        let own: Vec<&junit::Failure> = failures
            .iter()
            .filter(|failure| &failure.binary == binary && &failure.name == name)
            .collect();
        text.push_str(&explain(&own, first_seed, report));
        // nextest's binary id is `<package>` or `<package>::<target>`.
        let package = binary.split("::").next().unwrap_or(binary);
        let seed = first_seed.wrapping_add(u64::from(failed.first().copied().unwrap_or(0)));
        let _ = writeln!(
            text,
            "  again: {SEED}={seed} cargo xtask test -p {package} -E 'test(={name})'"
        );
    }
    Summary {
        text,
        failing: failing.len(),
        repeats_seen,
    }
}

#[cfg(test)]
mod tests {
    use super::{Request, Summary, summarise};
    use crate::junit;
    use crate::load::Squeeze;

    /// Where the tests say the report is.
    const REPORT_PATH: &str = "target/nextest/stress/junit.xml";

    fn summary(xml: &str, count: u32, first_seed: u64) -> Summary {
        summarise(
            &junit::parse(xml),
            &junit::failures(xml),
            REPORT_PATH,
            count,
            first_seed,
        )
    }

    #[test]
    fn a_squeeze_is_this_commands_and_not_nextests() {
        let request = Request::parse(&strings(&[
            "-p",
            "cineris-testkit",
            "--load",
            "6",
            "--cores",
            "2",
            "server::",
        ]))
        .unwrap();
        assert_eq!(
            request.squeeze,
            Squeeze {
                load: 6,
                cores: Some(2)
            }
        );
        assert_eq!(
            request.command().join(" "),
            "nextest run --profile stress --stress-count 100 -p cineris-testkit server::"
        );
        // The build before the squeeze runs nothing, whatever goes after `--`.
        let request = Request::parse(&strings(&[
            "-p",
            "cineris-testkit",
            "--cores",
            "2",
            "server::",
            "--",
            "--exact",
            "a",
        ]))
        .unwrap();
        assert_eq!(
            request.build_command().join(" "),
            "nextest run --profile stress --no-run -p cineris-testkit server::"
        );
    }

    fn strings(arguments: &[&str]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect()
    }

    #[test]
    fn a_request_is_read_and_what_it_does_not_know_goes_to_nextest() {
        let request = Request::parse(&strings(&[
            "-p",
            "cineris-agent",
            "--count",
            "300",
            "-p",
            "cineris-tools",
            "--first-seed",
            "1000",
            "denials::",
            "--run-ignored",
            "only",
        ]))
        .unwrap();

        assert_eq!(
            request,
            Request {
                packages: strings(&["cineris-agent", "cineris-tools"]),
                count: 300,
                first_seed: 1000,
                squeeze: Squeeze::default(),
                rest: strings(&["denials::", "--run-ignored", "only"]),
                strict: false,
            }
        );
        assert_eq!(
            request.command().join(" "),
            "nextest run --profile stress --stress-count 300 -p cineris-agent -p cineris-tools \
             denials:: --run-ignored only"
        );
    }

    #[test]
    fn a_package_is_read_in_every_spelling_and_nothing_after_the_dashes_is() {
        let request = Request::parse(&strings(&[
            "--package=cineris-agent",
            "-pcineris-tools",
            "--",
            "-p",
            "--count",
        ]))
        .unwrap();

        assert_eq!(
            request.packages,
            strings(&["cineris-agent", "cineris-tools"])
        );
        assert_eq!(request.count, 100);
        assert_eq!(request.rest, strings(&["--", "-p", "--count"]));
    }

    #[test]
    fn the_run_is_given_the_first_seed_and_no_seed_that_is_fixed() {
        let request =
            Request::parse(&strings(&["-p", "cineris-agent", "--first-seed", "1000"])).unwrap();
        let cargo = request.cargo(std::path::Path::new("."));
        let variable = |name: &str| {
            cargo
                .get_envs()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
        };

        assert_eq!(
            variable("CINERIS_TEST_FIRST_SEED"),
            Some(Some("1000".to_owned()))
        );
        // Removed, whatever the caller's environment holds.
        assert_eq!(variable("CINERIS_TEST_SEED"), Some(None));
    }

    #[test]
    fn a_hundred_times_from_seed_zero_when_not_told() {
        let request = Request::parse(&strings(&["-p", "cineris-agent"])).unwrap();

        assert_eq!((request.count, request.first_seed), (100, 0));
    }

    #[test]
    fn a_request_that_makes_no_sense_is_refused() {
        for (arguments, expected) in [
            (&[][..], "say which crates to stress"),
            (&["--count", "5"], "say which crates to stress"),
            (&["-p"], "`-p` needs a crate"),
            (&["-p", "a", "--count", "0"], "is not a number above 0"),
            (&["-p", "a", "--count", "lots"], "is not a number above 0"),
            (&["-p", "a", "--first-seed", "-3"], "is not a whole number"),
            (
                &["-p", "a", "--stress-count", "5"],
                "is set by this command",
            ),
            (&["-p", "a", "--profile", "ci"], "is set by this command"),
        ] {
            let error = Request::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    /// Four repeats of three tests. One passes every time, one fails on
    /// repeats 1 and 3, one fails on all four.
    fn report() -> String {
        let mut xml = String::from("<testsuites name=\"nextest-run\" tests=\"12\">\n");
        for repeat in 0..4 {
            let failure = "<failure type=\"test failure with exit code 101\">no</failure>";
            let odd = if repeat % 2 == 1 { failure } else { "" };
            xml.push_str(&format!(
                "<testsuite name=\"cineris-agent::it@stress-{repeat}\" tests=\"3\">
    <testcase name=\"rounds::steady\" classname=\"cineris-agent::it\" time=\"0.01\"></testcase>
    <testcase name=\"denials::needs_one_order\" classname=\"cineris-agent::it\" time=\"0.01\">{odd}</testcase>
    <testcase name=\"denials::broken\" classname=\"cineris-agent::it\" time=\"0.01\">{failure}</testcase>
</testsuite>\n"
            ));
        }
        xml.push_str("</testsuites>\n");
        xml
    }

    #[test]
    fn a_test_that_fails_on_some_repeats_is_reported_with_the_seeds_that_fail_it() {
        let summary = summary(&report(), 4, 1000);

        assert_eq!(summary.failing, 2);
        assert_eq!(summary.repeats_seen, 4);
        assert_eq!(
            summary.text,
            "
3 test(s), 4 time(s) each, with seeds 1000 to 1003.

FAILED 4 of 4 times: cineris-agent::it denials::broken
  It fails every time: the order of events is not what it depends on.
  seeds: 1000, 1001, 1002, 1003
  failure, under seed 1000:
    | no
  all of it: target/nextest/stress/junit.xml
  again: CINERIS_TEST_SEED=1000 cargo xtask test -p cineris-agent -E 'test(=denials::broken)'

FAILED 2 of 4 times: cineris-agent::it denials::needs_one_order
  seeds: 1001, 1003
  failure, under seed 1001:
    | no
  all of it: target/nextest/stress/junit.xml
  again: CINERIS_TEST_SEED=1001 cargo xtask test -p cineris-agent -E 'test(=denials::needs_one_order)'
"
        );
    }

    #[test]
    fn a_run_in_which_nothing_failed_says_so() {
        let report = report().replace(
            "<failure type=\"test failure with exit code 101\">no</failure>",
            "",
        );

        let summary = summary(&report, 4, 0);

        assert_eq!(summary.failing, 0);
        assert_eq!(
            summary.text,
            "\n3 test(s), 4 time(s) each, with seeds 0 to 3.\nEvery test passed every time.\n"
        );
    }

    #[test]
    fn a_test_that_passed_only_on_its_retry_failed_that_repeat() {
        // Repeat 1 failed and then passed when nextest tried it again.
        let report = "<testsuite name=\"a@stress-0\"><testcase name=\"timing_sensitive::t\" \
                      classname=\"a\" time=\"0.1\"/></testsuite><testsuite \
                      name=\"a@stress-1\"><testcase name=\"timing_sensitive::t\" \
                      classname=\"a\" time=\"0.1\"><flakyFailure>no</flakyFailure></testcase>\
                      </testsuite>";

        let summary = summary(report, 2, 0);

        assert_eq!(summary.failing, 1);
        assert!(
            summary
                .text
                .contains("FAILED 1 of 2 times: a timing_sensitive::t\n  seeds: 1\n"),
            "{}",
            summary.text
        );
    }

    #[test]
    fn a_run_found_nothing_only_if_nextest_agrees_and_every_repeat_is_there() {
        let clean = summary(
            &report().replace(
                "<failure type=\"test failure with exit code 101\">no</failure>",
                "",
            ),
            4,
            0,
        );
        let failing = summary(&report(), 4, 0);

        assert_eq!(super::conclude(&clean, true, 4, 0), Ok(()));
        let error = super::conclude(&failing, false, 4, 0).unwrap_err();
        assert!(error.contains("2 test(s) failed on some repeat"), "{error}");
        // nextest failed and no test is named: a build that broke, say.
        let error = super::conclude(&clean, false, 4, 0).unwrap_err();
        assert!(error.contains("names no failed test"), "{error}");
        // The run was cut short.
        let error = super::conclude(&clean, true, 5, 0).unwrap_err();
        assert!(
            error.contains("4 repeat(s) where 5 were asked for"),
            "{error}"
        );
    }

    #[test]
    fn known_flakes_that_passed_once_more_are_not_failures_and_nothing_else_is_excused() {
        let failing = summary(&report(), 4, 0);
        assert_eq!(failing.failing, 2);
        // Both failing tests were known flakes that passed: nextest's
        // failure is theirs.
        assert_eq!(super::conclude(&failing, false, 4, 2), Ok(()));
        // One was: the other still fails the run, and the message says both.
        let error = super::conclude(&failing, false, 4, 1).unwrap_err();
        assert!(
            error.contains(
                "1 test(s) failed on some repeat, besides 1 known flake(s) that passed when run \
                 once more"
            ),
            "{error}"
        );
    }

    fn listed(binary: &str, name: &str) -> crate::flaky::Listed {
        crate::flaky::Listed {
            test: crate::stall::rerun::Test {
                binary: binary.to_owned(),
                name: name.to_owned(),
            },
            item: "an-item".to_owned(),
        }
    }

    #[test]
    fn a_known_flake_is_run_once_more_under_the_first_seed_it_failed_with() {
        let case = |name: &str, repeat: u32, failed: bool, retried: bool| crate::junit::TestCase {
            binary: "cx::it".to_owned(),
            name: name.to_owned(),
            seconds: 0.1,
            failed,
            retried,
            repeat: Some(repeat),
        };
        let cases = [
            case("flaky", 3, false, false),
            case("flaky", 7, true, false),
            case("flaky", 5, true, false),
            // Failed only with a retry: a failure for stress.
            case("retried", 2, false, true),
            // Failed, and not listed: never run again.
            case("other", 1, true, false),
            // Listed, and passed every time.
            case("fine", 0, false, false),
        ];
        let list = [
            listed("cx::it", "flaky"),
            listed("cx::it", "retried"),
            listed("cx::it", "fine"),
            listed("cin", "other"),
        ];
        let chosen: Vec<(String, u64)> = super::flakes_to_run(&cases, &list, 1000)
            .into_iter()
            .map(|(one, seed)| (one.test.name, seed))
            .collect();
        assert_eq!(
            chosen,
            [("flaky".to_owned(), 1005), ("retried".to_owned(), 1002)]
        );
    }

    #[test]
    fn the_run_once_more_is_of_the_flake_alone_with_the_requests_other_arguments() {
        let request = Request::parse(&strings(&[
            "-p",
            "cineris-agent",
            "--count",
            "300",
            "-E",
            "test(/denials/)",
            "--run-ignored",
            "all",
            "--strict",
        ]))
        .unwrap();
        assert!(request.strict);
        let test = crate::stall::rerun::Test {
            binary: "cineris-agent::it".to_owned(),
            name: "a::b".to_owned(),
        };
        assert_eq!(
            request.once_command(&[test]).join(" "),
            "nextest run --profile stress -p cineris-agent --run-ignored all -E \
             (binary_id(=cineris-agent::it) and test(=a::b)) --retries 0"
        );
    }

    /// `known_flakes` on the cases of `report()` (two tests of `a`, each
    /// failing once) with `listed` known, a first run that ended with
    /// `code`, and a run once more that writes `second` and ends as
    /// `succeeds`: how many it excused, what it ran and what it said, and
    /// the report it left.
    fn flakes_with(
        label: &str,
        listed_names: &[&str],
        code: Option<i32>,
        strict: bool,
        second: &str,
        succeeds: bool,
    ) -> (usize, Vec<String>, String, String) {
        let folder =
            std::env::temp_dir().join(format!("xtask-stress-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let report_path = folder.join("junit.xml");
        let xml = report();
        std::fs::write(&report_path, &xml).unwrap();
        let mut words = vec!["-p", "cineris-agent"];
        if strict {
            words.push("--strict");
        }
        let request = Request::parse(&strings(&words)).unwrap();
        let listed: Vec<crate::flaky::Listed> = listed_names
            .iter()
            .map(|name| listed("cineris-agent::it", name))
            .collect();
        let mut ran = Vec::new();
        let mut said = String::new();
        let excused = super::known_flakes(
            &request,
            &listed,
            code,
            (&report_path, &xml),
            &junit::parse(&xml),
            &mut |arguments, seed| {
                ran.push(format!("{seed}: {}", arguments.join(" ")));
                std::fs::write(&report_path, second).unwrap();
                Ok(succeeds)
            },
            &mut |line| {
                said.push_str(line);
                said.push('\n');
            },
        )
        .unwrap();
        let left = std::fs::read_to_string(&report_path).unwrap();
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(left, xml, "the stress run's report is put back");
        (excused, ran, said, left)
    }

    fn one_case(name: &str, failed: bool) -> String {
        let failure = if failed {
            "<failure>again</failure>"
        } else {
            ""
        };
        format!(
            "<testsuites><testsuite name=\"cineris-agent::it\"><testcase name=\"{name}\" classname=\"cineris-agent::it\" \
             time=\"0.1\">{failure}</testcase></testsuite></testsuites>"
        )
    }

    #[test]
    fn known_flakes_are_run_once_more_counted_and_never_beside_another_kind_of_failure() {
        let names: Vec<String> = junit::parse(&report())
            .iter()
            .filter(|case| case.failed)
            .map(|case| case.name.clone())
            .collect();
        let first = names[0].as_str();
        // Listed, passes once more: excused, run under its seed, said.
        let (excused, ran, said, _) = flakes_with(
            "passes",
            &[first],
            Some(100),
            false,
            &one_case(first, false),
            true,
        );
        assert_eq!(excused, 1, "{said}");
        assert_eq!(ran.len(), 1, "{ran:?}");
        assert!(ran[0].contains(&format!("test(={first})")) && ran[0].contains("--retries 0"));
        assert!(said.contains("passed when run again"), "{said}");
        // It fails again; or nextest fails though its report says it passed.
        let (excused, _, said, _) = flakes_with(
            "again",
            &[first],
            Some(100),
            false,
            &one_case(first, true),
            false,
        );
        assert_eq!(excused, 0, "{said}");
        let (excused, _, said, _) = flakes_with(
            "status",
            &[first],
            Some(100),
            false,
            &one_case(first, false),
            false,
        );
        assert_eq!(excused, 0, "{said}");
        assert!(said.contains("but nextest failed"), "{said}");
        // --strict, or a stress run that ended other than by failed tests:
        // nothing is run.
        for (label, code, strict) in [("strict", Some(100), true), ("code", Some(104), false)] {
            let (excused, ran, said, _) =
                flakes_with(label, &[first], code, strict, &one_case(first, false), true);
            assert_eq!((excused, ran.len()), (0, 0), "{label}: {said}");
            assert!(said.contains("counts as a failure"), "{label}: {said}");
        }
        // Not listed: nothing at all.
        let (excused, ran, said, _) = flakes_with(
            "unlisted",
            &[],
            Some(100),
            false,
            &one_case(first, false),
            true,
        );
        assert_eq!((excused, ran.len(), said.as_str()), (0, 0, ""));
    }

    #[test]
    fn a_long_list_of_seeds_is_cut_and_counted() {
        let mut xml = String::new();
        for repeat in 0..20 {
            xml.push_str(&format!(
                "<testsuite name=\"a@stress-{repeat}\"><testcase name=\"t\" classname=\"a\" \
                 time=\"0.1\"><failure/></testcase></testsuite>"
            ));
        }

        let summary = summary(&xml, 20, 0);

        assert!(
            summary
                .text
                .contains("seeds: 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, and 8 more\n"),
            "{}",
            summary.text
        );
        assert!(
            summary
                .text
                .contains("again: CINERIS_TEST_SEED=0 cargo xtask test -p a -E 'test(=t)'"),
            "{}",
            summary.text
        );
    }

    /// A report nextest 0.9 wrote for three repeats of a crate made for
    /// it: `tests::flaky` fails on the even repeats with a message of nine
    /// lines, `tests::panics_plain` on every repeat with one of a line
    /// that holds `<`, `&` and quotes.
    const REAL: &str = include_str!("../fixtures/stress/junit.xml");

    #[test]
    fn each_failure_is_shown_by_the_start_of_its_message_with_where_the_rest_is() {
        let summary = summary(REAL, 3, 40);

        assert_eq!(summary.failing, 2);
        let flaky = summary
            .text
            .split("\nFAILED ")
            .find(|part| part.contains("jx tests::flaky"))
            .unwrap();
        assert_eq!(
            flaky,
            "2 of 3 times: jx tests::flaky
  seeds: 40, 42
  failure, under seed 40:
    | panicked at src/lib.rs:11:9:
    | assertion `left == right` failed: the repeat was even: 0
    | with a second line
    | and a third
    | and a fourth
    | and a fifth
    | … and 4 more line(s)
  all of it: target/nextest/stress/junit.xml
  again: CINERIS_TEST_SEED=40 cargo xtask test -p jx -E 'test(=tests::flaky)'
"
        );
        // The failure of repeat 2 differs only in its numbers: it is not
        // named again.
        assert!(!flaky.contains("another failure"), "{flaky}");
        assert!(
            summary.text.contains(
                "  failure, under seed 40:\n    | panicked at src/lib.rs:19:9:\n    | plain \
                 panic with <xml> & \"quotes\"\n  all of it:"
            ),
            "{}",
            summary.text
        );
    }

    #[test]
    fn a_timeout_shows_what_the_test_wrote_and_a_failure_of_another_kind_is_named() {
        let xml = "<testsuite name=\"a@stress-0\"><testcase name=\"t\" classname=\"a\" \
                   time=\"10\"><failure type=\"test timeout\"/><system-err>test seed 0\nnoise \
                   1\nnoise 2\nnoise 3\nnoise 4\nnoise 5\nnoise 6\n\n== cineris-testkit: this \
                   test has run 8.0 s of the 10 s it is given (x). What it is doing:\nwaits, and \
                   for how long:\n     8.0 s  thread `main`: the gate made at a.rs:4:2\n\
                   </system-err></testcase></testsuite><testsuite name=\"a@stress-1\">\
                   <testcase name=\"t\" classname=\"a\" time=\"0.1\"><failure \
                   message=\"thread &apos;t&apos; (9) panicked at a.rs:9:1\" type=\"test \
                   failure with exit code 101\">thread &apos;t&apos; (9) panicked at \
                   a.rs:9:1:\nleft and right differ</failure></testcase></testsuite>\
                   <testsuite name=\"a@stress-2\"><testcase name=\"t\" classname=\"a\" \
                   time=\"0.1\"><failure type=\"test failure with exit code 101\">thread \
                   &apos;t&apos; (12) panicked at a.rs:9:1:\nleft and right differ</failure>\
                   </testcase></testsuite>";

        let summary = summary(xml, 3, 0);

        assert!(
            summary.text.contains(
                "  failure, under seed 0:\n    | timed out (test timeout)\n    | == cineris-testkit: \
                 this test has run 8.0 s of the 10 s it is given (x). What it is doing:\n    | \
                 waits, and for how long:\n    |      8.0 s  thread `main`: the gate made at \
                 a.rs:4:2\n  another failure, under seed 1: panicked at a.rs:9:1:\n  all of it:"
            ),
            "{}",
            summary.text
        );
    }

    #[test]
    fn a_long_line_is_cut_and_many_other_texts_are_counted() {
        let mut xml = String::new();
        for repeat in 0..5 {
            let word = ["alpha", "beta", "gamma", "delta", "epsilon"][repeat];
            xml.push_str(&format!(
                "<testsuite name=\"a@stress-{repeat}\"><testcase name=\"t\" classname=\"a\" \
                 time=\"0.1\"><failure type=\"x\">{word} {}</failure></testcase></testsuite>",
                "y".repeat(300)
            ));
        }

        let summary = summary(&xml, 5, 0);

        let first = format!("    | alpha {} …\n", "y".repeat(154));
        assert!(summary.text.contains(&first), "{}", summary.text);
        assert!(
            summary
                .text
                .contains("  another failure, under seed 1: beta yyy"),
            "{}",
            summary.text
        );
        assert!(
            summary
                .text
                .contains("  another failure, under seed 2: gamma"),
            "{}",
            summary.text
        );
        assert!(
            summary
                .text
                .contains("  and 2 more failure(s) with other texts\n"),
            "{}",
            summary.text
        );
    }

    #[test]
    fn a_retry_that_passed_shows_the_failure_it_needed_it_for() {
        let xml = "<testsuite name=\"a@stress-0\"><testcase name=\"timing_sensitive::t\" \
                   classname=\"a\" time=\"0.1\"><flakyFailure type=\"test failure\">the \
                   first try lost a race</flakyFailure></testcase></testsuite>";

        let summary = summary(xml, 1, 0);

        assert_eq!(summary.failing, 1);
        assert!(
            summary.text.contains("    | the first try lost a race\n"),
            "{}",
            summary.text
        );
    }
}
