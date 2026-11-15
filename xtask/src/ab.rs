//! `cargo xtask ab`: two trees' tests on one machine, run in turn, and
//! their tiers compared (`docs/testing-and-ci.md`, "Two trees on one
//! runner").
//!
//! Two CI runs land on two runners, and hosted Windows runners differ in
//! speed by a factor of two: a change's cost to a tier cannot be told apart
//! from the runner it drew. This builds the tests of both trees once each,
//! each in a build directory of its own, then runs them in turn on the same
//! machine, `count` rounds of one run each, the first in the order base,
//! head and the next head, base, so that whatever drifts during the job
//! (a warmer disk, a busier neighbour) falls on both alike. Each test's time
//! is the median of its runs, and the two sides are compared as
//! `ci --tiers --compare` compares two CI runs. Each round's tier totals are
//! printed too: their spread on one side is the noise a difference has to
//! stand out of.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Result;
use crate::compare::{self, Side};
use crate::junit::{self, TestCase};
use crate::tiers::{self, Os, Tier};
use crate::workspace;

const USAGE: &str = "\
usage: cargo xtask ab --base DIR [--label NAME] [--count N] [--top N] [--keep-going]
                      [-p CRATE]... [-- NEXTEST ARGUMENT...]

Builds the tests of this tree (the head) and of the tree at DIR (the base)
once each, as the blocking test job does, each in a build directory of its
own; runs them in turn on this machine, N rounds (5 when not given) in
alternating order; and compares each test's median time, by tier, crate and
test, as `ci --tiers --compare` does. NAME is what the base is called in the
report (its directory when not given). `cargo xtask try` runs it on a hosted
runner with the input `against`.

A test that fails ends the run, after both builds, unless --keep-going is
given: then the round is recorded as failed for that side, the run goes on to
the next round, and the comparison is of the rounds both sides finished. The
rounds that failed, and the tests that failed in them, are listed after it,
and the command still fails.";

/// Rounds when not told: enough for a median, few enough for a Windows job.
const ROUNDS: usize = 5;

/// The most rounds a run may ask for.
const MOST_ROUNDS: usize = 50;

/// What `cargo xtask ab` was asked for.
#[derive(Debug, PartialEq)]
pub(crate) struct Request {
    pub(crate) base: PathBuf,
    pub(crate) label: Option<String>,
    pub(crate) rounds: usize,
    pub(crate) top: usize,
    /// A round with a failing test is recorded, not the end of the run.
    pub(crate) keep_going: bool,
    pub(crate) packages: Vec<String>,
    /// For nextest, after `--`.
    pub(crate) rest: Vec<String>,
}

impl Request {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut base = None;
        let mut request = Request {
            base: PathBuf::new(),
            label: None,
            rounds: ROUNDS,
            top: 10,
            keep_going: false,
            packages: Vec::new(),
            rest: Vec::new(),
        };
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            let mut value = |what: &str| {
                words
                    .next()
                    .ok_or_else(|| format!("`{word}` needs {what}\n\n{USAGE}"))
            };
            match word.as_str() {
                "--" => {
                    request.rest = words.cloned().collect();
                    break;
                }
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
                "--top" => {
                    let text = value("a number")?;
                    request.top = text
                        .parse()
                        .map_err(|_| format!("`--top {text}` is not a number"))?;
                }
                "--keep-going" => request.keep_going = true,
                "-p" | "--package" => request.packages.push(value("a crate")?.clone()),
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
            }
        }
        request.base = base.ok_or_else(|| format!("say which tree is the base\n\n{USAGE}"))?;
        Ok(request)
    }
}

/// Which side runs first in round `round`, counted from 0: base, head,
/// then head, base, and so on.
pub(crate) fn order(round: usize) -> [Which; 2] {
    if round.is_multiple_of(2) {
        [Which::Base, Which::Head]
    } else {
        [Which::Head, Which::Base]
    }
}

/// One of the two trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Which {
    Base,
    Head,
}

/// Each test's median time over the runs of one side. A test that is not in
/// every run keeps the median of the runs it is in.
pub(crate) fn medians(runs: &[Vec<TestCase>]) -> Vec<TestCase> {
    let mut times: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();
    for run in runs {
        for case in run {
            times
                .entry((case.binary.clone(), case.name.clone()))
                .or_default()
                .push(case.seconds);
        }
    }
    times
        .into_iter()
        .map(|((binary, name), mut seconds)| {
            seconds.sort_by(f64::total_cmp);
            let middle = seconds.len() / 2;
            let median = if seconds.len() % 2 == 1 {
                seconds[middle]
            } else {
                f64::midpoint(seconds[middle - 1], seconds[middle])
            };
            TestCase {
                binary,
                name,
                seconds: median,
                failed: false,
                retried: false,
                repeat: None,
            }
        })
        .collect()
}

/// One side's run of one round: it finished, with its report, or a test
/// failed in it (or the run could not be made).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Attempt {
    Finished(Vec<TestCase>),
    Failed(Failure),
}

/// What a failed run left: the tests that failed, as `binary name`, from
/// whatever report it wrote, and what the run said.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Failure {
    pub(crate) tests: Vec<String>,
    pub(crate) error: String,
}

/// A failed run's failure, from its report when it wrote one.
pub(crate) fn failure_of(report: Option<&[TestCase]>, error: String) -> Failure {
    Failure {
        tests: report
            .unwrap_or_default()
            .iter()
            .filter(|case| case.failed)
            .map(TestCase::id)
            .collect(),
        error,
    }
}

/// Both sides of one round.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Round {
    pub(crate) base: Attempt,
    pub(crate) head: Attempt,
}

/// The rounds both sides finished, which are all that is compared: their
/// numbers (counted from 1), and each side's reports. A round in which either
/// side failed is in none of them.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Compared {
    pub(crate) numbers: Vec<usize>,
    pub(crate) base: Vec<Vec<TestCase>>,
    pub(crate) head: Vec<Vec<TestCase>>,
}

/// The rounds in which both sides finished.
pub(crate) fn finished_by_both(rounds: &[Round]) -> Compared {
    let mut compared = Compared::default();
    for (index, round) in rounds.iter().enumerate() {
        if let (Attempt::Finished(base), Attempt::Finished(head)) = (&round.base, &round.head) {
            compared.numbers.push(index + 1);
            compared.base.push(base.clone());
            compared.head.push(head.clone());
        }
    }
    compared
}

/// The rounds in which a side failed, and the tests that failed: empty when
/// none did.
pub(crate) fn failed_rounds(rounds: &[Round], label: &str) -> String {
    let mut text = String::new();
    for (index, round) in rounds.iter().enumerate() {
        let sides = [
            (format!("base ({label})"), &round.base),
            ("head".to_owned(), &round.head),
        ];
        for (side, attempt) in sides {
            let Attempt::Failed(failure) = attempt else {
                continue;
            };
            let _ = write!(text, "  round {}, {side}: ", index + 1);
            if failure.tests.is_empty() {
                let _ = writeln!(text, "no failing test in a report: {}", failure.error);
            } else {
                let _ = writeln!(text, "{} failed", failure.tests.join(", "));
            }
        }
    }
    if text.is_empty() {
        return text;
    }
    format!(
        "Rounds in which a run failed, left out of the comparison along with the other side's \
         run of the same round:\n{text}"
    )
}

/// Each round's tier times above the start, for both sides: the spread of
/// one side is the noise.
pub(crate) fn rounds_table(compared: &Compared) -> String {
    let mut text = String::from(
        "Each round, each tier's time above the start of its processes, in seconds:\n\n\
         round  side   start      unit  integration     system\n",
    );
    for (number, (base, head)) in compared
        .numbers
        .iter()
        .zip(compared.base.iter().zip(&compared.head))
    {
        for (side, cases) in [("base", base), ("head", head)] {
            let (start, _) = tiers::start_of(cases);
            let [unit, integration, system] =
                Tier::ALL.map(|tier| tiers::total(cases, tier, start).above_start);
            let _ = writeln!(
                text,
                "{:>5}  {side}  {:>5.0} ms  {unit:>8.1}  {integration:>11.1}  {system:>9.1}",
                number,
                start * 1000.0
            );
        }
    }
    text
}

/// `path` with its `.` and `..` taken out by reading it, not the file
/// system: on Linux `std::path::absolute` keeps them, so `../ab-base` from
/// the head would otherwise read as inside it.
pub(crate) fn normalised(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Refuses a base that is the head, or inside it, where the head's
/// `.cargo/config.toml` would apply to its build too. Both absolute.
pub(crate) fn refuse_base(head: &Path, base: &Path) -> Result<()> {
    let (head, base) = (normalised(head), normalised(base));
    if base == head {
        return Err("the base is this tree: give another checkout".to_owned());
    }
    if base.starts_with(&head) {
        return Err(format!(
            "{} is inside this tree, whose .cargo/config.toml would apply to its build too: \
             put it beside this tree",
            base.display()
        ));
    }
    Ok(())
}

/// The trees' build directories: each its own, whatever the caller's
/// environment says, so that building one never rebuilds the other.
fn target_of(tree: &Path) -> PathBuf {
    tree.join("target")
}

/// `cargo nextest run` in `tree` with `arguments`: the build of the
/// blocking test job's tests, or one of its runs.
fn nextest(tree: &Path, arguments: &[String]) -> Command {
    let mut command = Command::new(workspace::cargo());
    command
        .current_dir(tree)
        .env("CARGO_TARGET_DIR", target_of(tree))
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .args(arguments);
    command
}

/// The blocking test job's runs in `tree`, as that tree's own job runs
/// them: a base from before the job ran in two is run in one
/// (`checks::splits`), so that an A/B of the change compares the two jobs.
/// One report of them all is left where one run leaves its own.
fn run_job(tree: &Path, request: &Request) -> Result<()> {
    let split = crate::checks::splits(tree);
    let runs = crate::checks::ci_runs(&request.packages, &request.rest, split);
    crate::checks::forget_reports(tree)?;
    let last = runs.len() - 1;
    for (index, run) in runs.iter().enumerate() {
        let ran = run_in(tree, nextest(tree, run), "the tests");
        // A run that failed ends the job's, and what the earlier runs
        // reported is brought together with its own for whoever reads it.
        if index == last || ran.is_err() {
            crate::checks::bring_together(tree)?;
        } else {
            crate::checks::set_aside(tree)?;
        }
        ran?;
    }
    Ok(())
}

fn run_in(tree: &Path, mut command: Command, what: &str) -> Result<()> {
    let status = command
        .status()
        .map_err(|error| format!("could not run cargo in {}: {error}", tree.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed in {} ({status})", tree.display()))
    }
}

/// `cargo xtask ab …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    crate::checks::require_nextest()?;
    let head = workspace::root();
    // Without its dots: the path is the build directory's too, and is
    // compiled into the tests (`CARGO_TARGET_TMPDIR`), where a Unix socket
    // under `…/repo/../ab-base/target/tmp/…` ran past the 108 bytes of its
    // address on a hosted runner.
    let base = normalised(
        &std::path::absolute(&request.base)
            .map_err(|error| format!("{}: {error}", request.base.display()))?,
    );
    if !base.join("Cargo.toml").is_file() {
        return Err(format!(
            "{} is not a checkout of the workspace",
            base.display()
        ));
    }
    refuse_base(&head, &base)?;
    let label = request
        .label
        .clone()
        .unwrap_or_else(|| base.display().to_string());
    let tree = |which: Which| if which == Which::Base { &base } else { &head };

    let mut build = crate::checks::ci_run(&request.packages);
    build.push("--no-run".to_owned());
    for which in [Which::Base, Which::Head] {
        println!("== building the {} ==", name(which, &label));
        run_in(tree(which), nextest(tree(which), &build), "the build")?;
    }
    let mut rounds: Vec<Round> = Vec::new();
    for round in 0..request.rounds {
        let mut attempts: [Option<Attempt>; 2] = [None, None];
        for which in order(round) {
            println!(
                "== round {} of {}: the {} ==",
                round + 1,
                request.rounds,
                name(which, &label)
            );
            let at = tree(which);
            let report = workspace::nextest_report(at, "ci");
            let ran = run_job(at, &request);
            if let Err(error) = &ran {
                if !request.keep_going {
                    return Err(error.clone());
                }
                println!(
                    "== the {} failed: {error}; carrying on ==",
                    name(which, &label)
                );
            }
            // What the run reported, if it got as far as a report.
            let cases = junit::read(&report);
            // Each round's report, kept where the head's run keeps its own
            // (the `try` workflow uploads the folder).
            let kept = head.join("target").join("ab").join(format!(
                "{}-round-{}.xml",
                if which == Which::Base { "base" } else { "head" },
                round + 1
            ));
            if report.is_file() {
                std::fs::create_dir_all(kept.parent().unwrap_or(&head))
                    .and_then(|()| std::fs::copy(&report, &kept))
                    .map_err(|error| format!("could not keep {}: {error}", kept.display()))?;
            }
            attempts[which as usize] = Some(attempt_of(ran, cases)?);
        }
        let [Some(base), Some(head)] = attempts else {
            return Err("a round ran only one side".to_owned());
        };
        rounds.push(Round { base, head });
    }
    let compared = finished_by_both(&rounds);
    let failed = failed_rounds(&rounds, &label);
    if !failed.is_empty() {
        println!("\n{failed}");
    }
    let (base_runs, head_runs) = (&compared.base, &compared.head);
    if base_runs.is_empty() {
        crate::steps::append_job_summary(&format!("### Two trees on one runner\n\n{failed}\n"));
        return Err(format!(
            "no round finished on both sides: all {} had a failing run",
            rounds.len()
        ));
    }
    println!("\n{}", rounds_table(&compared));
    let base_side = Side {
        label: format!("base, {label} (the median of {} runs)", base_runs.len()),
        cases: medians(base_runs),
    };
    let head_side = Side {
        label: format!("head, this tree (the median of {} runs)", head_runs.len()),
        cases: medians(head_runs),
    };
    let table = compare::compare_with(
        &head_side,
        &base_side,
        Os::here(),
        request.top,
        compare::Start::Shared,
    );
    println!("{table}");
    crate::steps::append_job_summary(&format!(
        "### Two trees on one runner\n\n```text\n{}\n{table}\n{failed}```\n",
        rounds_table(&compared)
    ));
    conclusion(rounds.len(), compared.numbers.len())
}

/// What the run ends as, after its comparison is printed: the measurement is
/// kept, and a failing test is still a failure.
pub(crate) fn conclusion(rounds: usize, compared: usize) -> Result<()> {
    if compared < rounds {
        Err(format!(
            "{} of {rounds} rounds had a failing run, and are left out of the comparison above",
            rounds - compared
        ))
    } else {
        Ok(())
    }
}

/// One side's run of a round: `ran` is how the tests ended, and `report`
/// the report it left, if it left one. A run that failed is a failed
/// attempt, whatever its report says; one that passed and left no report
/// is an error.
pub(crate) fn attempt_of(ran: Result<()>, report: Result<Vec<TestCase>>) -> Result<Attempt> {
    match ran {
        Ok(()) => report.map(Attempt::Finished),
        Err(error) => Ok(Attempt::Failed(failure_of(report.ok().as_deref(), error))),
    }
}

fn name(which: Which, label: &str) -> String {
    match which {
        Which::Base => format!("base ({label})"),
        Which::Head => "head (this tree)".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Attempt, Compared, Failure, Request, Round, Which, attempt_of, conclusion, failed_rounds,
        failure_of, finished_by_both, medians, order, rounds_table,
    };
    use crate::junit::TestCase;

    fn case(binary: &str, name: &str, seconds: f64) -> TestCase {
        TestCase {
            binary: binary.to_owned(),
            name: name.to_owned(),
            seconds,
            failed: false,
            retried: false,
            repeat: None,
        }
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_request_is_read_and_what_makes_no_sense_is_refused() {
        assert_eq!(
            Request::parse(&strings(&[
                "--base",
                "../main",
                "--count",
                "8",
                "-p",
                "cortex-base",
                "--label",
                "main",
                "--",
                "-E",
                "test(/x/)"
            ]))
            .unwrap(),
            Request {
                base: "../main".into(),
                label: Some("main".to_owned()),
                rounds: 8,
                top: 10,
                keep_going: false,
                packages: strings(&["cortex-base"]),
                rest: strings(&["-E", "test(/x/)"]),
            }
        );
        assert_eq!(
            Request::parse(&strings(&["--base", "b"])).unwrap().rounds,
            5
        );
        for (words, expected) in [
            (&["--count", "3"][..], "say which tree is the base"),
            (&["--base", "b", "--count", "1"], "from 2 to 50"),
            (&["--base", "b", "--count", "51"], "from 2 to 50"),
            (&["--base"], "needs a directory"),
            (&["--base", "b", "--os", "linux"], "unknown argument"),
        ] {
            let error = Request::parse(&strings(words)).unwrap_err();
            assert!(error.contains(expected), "{words:?}: {error}");
        }
    }

    /// `../ab-base` from the head is beside it, on every system: on Linux
    /// `std::path::absolute` keeps the `..`, and a comparison of the
    /// components read it as inside.
    #[test]
    fn a_base_beside_the_head_is_taken_and_one_inside_it_refused() {
        let head = std::path::absolute("work/repo").unwrap();
        let beside = std::path::absolute("work/repo/../ab-base").unwrap();
        assert_eq!(super::refuse_base(&head, &beside), Ok(()));
        let inside = std::path::absolute("work/repo/./ab-base").unwrap();
        let error = super::refuse_base(&head, &inside).unwrap_err();
        assert!(error.contains("is inside this tree"), "{error}");
        let same = std::path::absolute("work/repo/x/..").unwrap();
        let error = super::refuse_base(&head, &same).unwrap_err();
        assert!(error.contains("the base is this tree"), "{error}");
    }

    #[test]
    fn the_order_alternates_so_drift_falls_on_both_sides() {
        assert_eq!(order(0), [Which::Base, Which::Head]);
        assert_eq!(order(1), [Which::Head, Which::Base]);
        assert_eq!(order(2), [Which::Base, Which::Head]);
    }

    #[test]
    fn each_test_takes_its_median_and_one_slow_run_moves_nothing() {
        let runs = vec![
            vec![case("a", "t", 0.010), case("a", "u", 0.5)],
            vec![case("a", "t", 0.012), case("a", "u", 9.0)],
            vec![case("a", "t", 0.011), case("a", "u", 0.6)],
            vec![case("a", "t", 0.013)],
        ];
        let medians = medians(&runs);
        assert_eq!(medians.len(), 2);
        assert!((medians[0].seconds - 0.0115).abs() < 1e-9, "{medians:?}");
        // The run of 9 s is one of three: the median is the middle one.
        assert!((medians[1].seconds - 0.6).abs() < 1e-9, "{medians:?}");
    }

    #[test]
    fn each_round_is_a_line_for_each_side() {
        let compared = Compared {
            numbers: vec![1, 3],
            base: vec![vec![case("cortex-base", "t", 0.5)]; 2],
            head: vec![vec![case("cortex-base", "t", 0.75)]; 2],
        };
        let table = rounds_table(&compared);
        let lines: Vec<&str> = table
            .lines()
            .filter(|line| line.contains(" base ") || line.contains(" head "))
            .collect();
        assert_eq!(lines.len(), 4, "{table}");
        assert!(lines[0].trim_start().starts_with("1  base"), "{table}");
        // A round left out keeps the numbers of the others.
        assert!(lines[3].trim_start().starts_with("3  head"), "{table}");
    }

    fn finished(seconds: f64) -> Attempt {
        Attempt::Finished(vec![case("cortex-base", "t", seconds)])
    }

    fn failed(tests: &[&str]) -> Attempt {
        Attempt::Failed(Failure {
            tests: strings(tests),
            error: "the tests failed".to_owned(),
        })
    }

    /// Four rounds: the head fails the second, the base fails the third.
    fn rounds() -> Vec<Round> {
        vec![
            Round {
                base: finished(0.5),
                head: finished(0.6),
            },
            Round {
                base: finished(0.5),
                head: failed(&["cortex-base t"]),
            },
            Round {
                base: failed(&[]),
                head: finished(0.6),
            },
            Round {
                base: finished(0.7),
                head: finished(0.8),
            },
        ]
    }

    #[test]
    fn only_the_rounds_both_sides_finished_are_compared() {
        let compared = finished_by_both(&rounds());
        assert_eq!(compared.numbers, [1, 4]);
        // The round the base finished and the head failed is not in the
        // base's medians either.
        assert_eq!(compared.base.len(), 2);
        assert!((compared.base[1][0].seconds - 0.7).abs() < 1e-9);
        assert!((compared.head[0][0].seconds - 0.6).abs() < 1e-9);
        let none = finished_by_both(&rounds()[1..3]);
        assert_eq!(none, Compared::default());
    }

    #[test]
    fn the_rounds_that_failed_are_listed_with_their_tests_and_a_clean_run_has_no_list() {
        let text = failed_rounds(&rounds(), "main");
        assert!(
            text.contains("round 2, head: cortex-base t failed"),
            "{text}"
        );
        assert!(
            text.contains("round 3, base (main): no failing test in a report: the tests failed"),
            "{text}"
        );
        assert!(
            !text.contains("round 1") && !text.contains("round 4"),
            "{text}"
        );
        assert_eq!(failed_rounds(&rounds()[..1], "main"), "");
    }

    #[test]
    fn a_failed_run_names_the_tests_its_report_says_failed() {
        let mut bad = case("cortex-agent::it", "turn::stops", 1.0);
        bad.failed = true;
        let report = vec![case("cortex-base", "t", 0.1), bad];
        assert_eq!(
            failure_of(Some(&report), "the tests failed".to_owned()),
            Failure {
                tests: strings(&["cortex-agent::it turn::stops"]),
                error: "the tests failed".to_owned(),
            }
        );
        // No report: nothing to name, and the error is what is left.
        assert!(failure_of(None, "x".to_owned()).tests.is_empty());
    }

    #[test]
    fn a_run_that_failed_is_a_failed_attempt_whatever_its_report_says() {
        let mut bad = case("cortex-base", "t", 1.0);
        bad.failed = true;
        let report = || Ok(vec![bad.clone()]);
        let attempt = attempt_of(Err("the tests failed".to_owned()), report()).unwrap();
        assert_eq!(attempt, failed(&["cortex-base t"]));
        // No report at all.
        let attempt = attempt_of(Err("the tests failed".to_owned()), Err("none".to_owned()));
        assert_eq!(attempt, Ok(failed(&[])));
        // A run that passed is finished, and one that left no report is an
        // error, not a failed attempt: nothing says it failed.
        assert!(matches!(
            attempt_of(Ok(()), report()),
            Ok(Attempt::Finished(_))
        ));
        assert_eq!(
            attempt_of(Ok(()), Err("none".to_owned())),
            Err("none".to_owned())
        );
    }

    #[test]
    fn a_run_with_a_round_left_out_ends_as_a_failure_and_one_without_does_not() {
        assert_eq!(conclusion(5, 5), Ok(()));
        let error = conclusion(5, 3).unwrap_err();
        assert!(error.contains("2 of 5 rounds"), "{error}");
        assert!(conclusion(2, 0).is_err());
    }

    #[test]
    fn a_request_may_ask_to_keep_going() {
        let request = Request::parse(&strings(&["--base", "b", "--keep-going"])).unwrap();
        assert!(request.keep_going);
        assert!(
            !Request::parse(&strings(&["--base", "b"]))
                .unwrap()
                .keep_going
        );
    }
}
