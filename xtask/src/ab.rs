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

use std::collections::{BTreeMap, BTreeSet};
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
the next round (and, within a side's job, on to its system tests after a
failure in tiers 1 and 2), and the comparison is of the rounds both sides
finished. The rounds that failed, and the tests that failed in them, are
listed after it, and the command still fails.

With --keep-going, a test that failed on both sides in a round, and nothing
else going wrong in it, is not a failure of the round: it is left out of both
sides' medians and of the comparison (in every round, not only the one it
failed in), the other tests are compared as usual, and each such test is
named, with its rounds and the first line of each side's failure. A test that
failed on one side only, or a run that broke some other way, is a failure as
above. When only tests that failed on both sides went wrong, the command
succeeds.";

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

/// A test that failed, and the first line of what its failure said.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FailedTest {
    /// `binary name`, as [`TestCase::id`] writes it.
    pub(crate) id: String,
    pub(crate) message: String,
}

/// What a failed run left: the tests that failed, from whatever report it
/// wrote, with the whole report, and what the run said.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Failure {
    pub(crate) tests: Vec<FailedTest>,
    pub(crate) error: String,
    /// Every test the report lists, the failed ones included. Empty when the
    /// run wrote no report.
    pub(crate) cases: Vec<TestCase>,
    /// Every run of the job that failed did so because tests failed in it
    /// (nextest's exit code for that, and a failed test in its report), and
    /// for no other reason: not a build that broke, not a run that listed
    /// nothing. Only then do the failed tests say all that went wrong.
    pub(crate) by_tests: bool,
}

/// What a run of the job left of its report: every test, and the failed
/// ones with their messages.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Report {
    pub(crate) cases: Vec<TestCase>,
    pub(crate) failed: Vec<FailedTest>,
}

impl Report {
    /// Reads the report at `path`. A report with no test in it is an error,
    /// as for [`junit::read`]: a run that listed nothing checked nothing.
    fn read(path: &Path) -> Result<Self> {
        let xml = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let report = Self::of(&xml);
        if report.cases.is_empty() {
            return Err(format!("{} lists no tests", path.display()));
        }
        Ok(report)
    }

    /// The report written as `xml`.
    pub(crate) fn of(xml: &str) -> Self {
        let cases = junit::parse(xml);
        let said = junit::failures(xml);
        let failed = cases
            .iter()
            .filter(|case| case.failed)
            .map(|case| {
                let message = said
                    .iter()
                    .find(|one| {
                        one.binary == case.binary
                            && one.name == case.name
                            && one.repeat == case.repeat
                    })
                    .map(|one| {
                        let line = first_line(&one.text);
                        if line.is_empty() {
                            one.kind.clone()
                        } else {
                            line
                        }
                    })
                    .unwrap_or_default();
                FailedTest {
                    id: case.id(),
                    message,
                }
            })
            .collect();
        Self { cases, failed }
    }
}

/// The first line of `text` that says something, trimmed.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// A failed run's failure, from its report when it wrote one. `by_tests`:
/// see [`Failure::by_tests`].
pub(crate) fn failure_of(report: Option<Report>, error: String, by_tests: bool) -> Failure {
    let Some(report) = report else {
        return Failure {
            tests: Vec::new(),
            error,
            cases: Vec::new(),
            by_tests: false,
        };
    };
    // A report that names no failed test explains nothing.
    let explained = by_tests && !report.failed.is_empty();
    Failure {
        tests: report.failed,
        error,
        cases: report.cases,
        by_tests: explained,
    }
}

/// nextest's exit code for a run in which a test failed (`cargo nextest run`
/// exits 100 then, and another code when the build broke, no test was run or
/// the run could not be made).
pub(crate) const TEST_RUN_FAILED: i32 = 100;

/// How one run of the job ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RunEnd {
    Passed,
    /// Tests failed in it, and nothing else went wrong: the exit code says
    /// so, and its report names a failed test.
    TestsFailed(String),
    /// It failed otherwise, or in a way nothing says was the tests': a build
    /// that broke, a run that could not be made, a failure no report names.
    Broke(String),
}

/// How a run ended: whether it succeeded, its exit code if it had one,
/// whether the report it left names a failed test, and what to say of it.
pub(crate) fn end_of(
    success: bool,
    code: Option<i32>,
    a_test_failed: bool,
    said: String,
) -> RunEnd {
    if success {
        RunEnd::Passed
    } else if code == Some(TEST_RUN_FAILED) && a_test_failed {
        RunEnd::TestsFailed(said)
    } else {
        RunEnd::Broke(said)
    }
}

/// How a side's job ended, the runs of it together.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JobEnd {
    /// What went wrong, if anything did.
    pub(crate) error: Option<String>,
    /// Every failed run failed by its tests ([`Failure::by_tests`]).
    pub(crate) by_tests: bool,
}

/// Runs the job's `runs` in turn. Without `keep_going` the first run that
/// fails ends the job, as it always did. With it every run is made, so that
/// a failing test of tiers 1 and 2 does not leave the system tests
/// unmeasured: the round's two runs are each a run the comparison needs.
/// `after(index, last)` is called after each run, `last` when it is the job's
/// last (the reports are then brought together, otherwise set aside).
pub(crate) fn drive_job(
    runs: usize,
    keep_going: bool,
    mut run: impl FnMut(usize) -> RunEnd,
    mut after: impl FnMut(usize, bool) -> Result<()>,
) -> JobEnd {
    let mut errors = Vec::new();
    let mut by_tests = true;
    for index in 0..runs {
        let end = run(index);
        let failed = end != RunEnd::Passed;
        let stop = failed && !keep_going;
        if let Err(error) = after(index, index + 1 == runs || stop) {
            errors.push(error);
            by_tests = false;
        }
        match end {
            RunEnd::Passed => {}
            RunEnd::TestsFailed(said) => errors.push(said),
            RunEnd::Broke(said) => {
                errors.push(said);
                by_tests = false;
            }
        }
        if stop {
            break;
        }
    }
    JobEnd {
        error: (!errors.is_empty()).then(|| errors.join("; ")),
        by_tests,
    }
}

/// Both sides of one round.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Round {
    pub(crate) base: Attempt,
    pub(crate) head: Attempt,
}

/// The rounds that are compared: their numbers (counted from 1), and each
/// side's reports. A round in which a side failed is in none of them, unless
/// the tests that failed are the same on both sides and nothing else went
/// wrong ([`failed_on_both`]): it is compared without them.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Compared {
    pub(crate) numbers: Vec<usize>,
    pub(crate) base: Vec<Vec<TestCase>>,
    pub(crate) head: Vec<Vec<TestCase>>,
}

/// A test that failed on both sides of a round, and so is left out of the
/// comparison altogether.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Skipped {
    pub(crate) id: String,
    /// The rounds it failed on both sides in, counted from 1.
    pub(crate) rounds: Vec<usize>,
    /// What each side's failure said, in the first of those rounds.
    pub(crate) base: String,
    pub(crate) head: String,
}

/// The tests that failed on both sides of `round`, if that is all that went
/// wrong in it: both sides' runs failed, each only by tests failing, and the
/// two sides name the same failed tests. A test that failed on one side only
/// (the other passed it, did not run it, or does not have it) is a failure
/// of the round, and so is a run that broke in any other way or wrote no
/// report: nothing may let a comparison pass when a side failed or a test
/// went missing.
fn failed_on_both(round: &Round) -> Option<Vec<(&FailedTest, &FailedTest)>> {
    let (Attempt::Failed(base), Attempt::Failed(head)) = (&round.base, &round.head) else {
        return None;
    };
    if !base.by_tests || !head.by_tests || base.cases.is_empty() || head.cases.is_empty() {
        return None;
    }
    fn ids(failure: &Failure) -> BTreeSet<&str> {
        failure.tests.iter().map(|test| test.id.as_str()).collect()
    }
    if base.tests.is_empty() || ids(base) != ids(head) {
        return None;
    }
    base.tests
        .iter()
        .map(|one| {
            head.tests
                .iter()
                .find(|other| other.id == one.id)
                .map(|other| (one, other))
        })
        .collect()
}

/// The rounds that are compared, and the tests left out of them. A round in
/// which both sides finished is compared. So is one in which both sides ran
/// and failed only the same tests, without those tests. Such a test is left
/// out of every round of both sides, not only the one it failed in: what it
/// took in a round it passed is no more a measurement than what it took in
/// one it failed.
pub(crate) fn comparable(rounds: &[Round]) -> (Compared, Vec<Skipped>) {
    let mut skipped: BTreeMap<String, Skipped> = BTreeMap::new();
    for (index, round) in rounds.iter().enumerate() {
        for (base, head) in failed_on_both(round).unwrap_or_default() {
            skipped
                .entry(base.id.clone())
                .or_insert_with(|| Skipped {
                    id: base.id.clone(),
                    rounds: Vec::new(),
                    base: base.message.clone(),
                    head: head.message.clone(),
                })
                .rounds
                .push(index + 1);
        }
    }
    let mut compared = Compared::default();
    for (index, round) in rounds.iter().enumerate() {
        let (base, head) = match (&round.base, &round.head) {
            (Attempt::Finished(base), Attempt::Finished(head)) => (base, head),
            (Attempt::Failed(base), Attempt::Failed(head)) if failed_on_both(round).is_some() => {
                (&base.cases, &head.cases)
            }
            _ => continue,
        };
        let without = |cases: &Vec<TestCase>| {
            cases
                .iter()
                .filter(|case| !skipped.contains_key(&case.id()))
                .cloned()
                .collect::<Vec<_>>()
        };
        compared.numbers.push(index + 1);
        compared.base.push(without(base));
        compared.head.push(without(head));
    }
    (compared, skipped.into_values().collect())
}

/// What to say of the tests left out: each, the rounds it failed on both
/// sides in, and the first line of what each side's failure said. Empty when
/// none was.
pub(crate) fn skipped_text(skipped: &[Skipped]) -> String {
    if skipped.is_empty() {
        return String::new();
    }
    let mut text = format!(
        "{} TEST(S) FAILED ON BOTH SIDES, and are left out of both sides' medians and of the \
         comparison (a test that failed on one side only is a failure of its round, below):\n",
        skipped.len()
    );
    fn message(said: &str) -> &str {
        if said.is_empty() {
            "(no message)"
        } else {
            said
        }
    }
    for one in skipped {
        let rounds: Vec<String> = one.rounds.iter().map(ToString::to_string).collect();
        let _ = writeln!(
            text,
            "  {} (round {})\n      base: {}\n      head: {}",
            one.id,
            rounds.join(", "),
            message(&one.base),
            message(&one.head)
        );
    }
    text
}

/// The rounds in which a side failed and that are not compared, and the
/// tests that failed: empty when none did. A round whose failures were the
/// same tests on both sides is not here: it is compared, and its tests are
/// in [`skipped_text`].
pub(crate) fn failed_rounds(rounds: &[Round], label: &str) -> String {
    let mut text = String::new();
    for (index, round) in rounds.iter().enumerate() {
        if failed_on_both(round).is_some() {
            continue;
        }
        let sides = [
            (format!("base ({label})"), &round.base),
            ("head".to_owned(), &round.head),
        ];
        for (side, attempt) in sides {
            let Attempt::Failed(failure) = attempt else {
                continue;
            };
            let _ = write!(text, "  round {}, {side}: ", index + 1);
            let tests: Vec<&str> = failure.tests.iter().map(|test| test.id.as_str()).collect();
            if tests.is_empty() {
                let _ = writeln!(text, "no failing test in a report: {}", failure.error);
            } else if failure.by_tests {
                let _ = writeln!(text, "{} failed", tests.join(", "));
            } else {
                let _ = writeln!(
                    text,
                    "{} failed, and the run failed otherwise too: {}",
                    tests.join(", "),
                    failure.error
                );
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
/// One report of them all is left where one run leaves its own. With
/// `--keep-going` every run is made even when an earlier one failed (a test
/// of tiers 1 and 2 that fails must not leave the system tests unmeasured).
fn run_job(tree: &Path, request: &Request) -> JobEnd {
    let split = crate::checks::splits(tree);
    let runs = crate::checks::ci_runs(&request.packages, &request.rest, split);
    if let Err(error) = crate::checks::forget_reports(tree) {
        return JobEnd {
            error: Some(error),
            by_tests: false,
        };
    }
    let report = workspace::nextest_report(tree, "ci");
    drive_job(
        runs.len(),
        request.keep_going,
        |index| run_in(tree, nextest(tree, &runs[index]), "the tests", &report),
        // A run that failed ends the job's, and what the earlier runs
        // reported is brought together with its own for whoever reads it.
        |_, last| {
            if last {
                crate::checks::bring_together(tree)
            } else {
                crate::checks::set_aside(tree)
            }
        },
    )
}

/// One run of `command` in `tree`, and how it ended. `report` is where the
/// run leaves its report: a run that failed is the tests' failing only when
/// nextest says so (its exit code) and the report names a failed test.
fn run_in(tree: &Path, mut command: Command, what: &str, report: &Path) -> RunEnd {
    let status = match command.status() {
        Ok(status) => status,
        Err(error) => {
            return RunEnd::Broke(format!(
                "could not run cargo in {}: {error}",
                tree.display()
            ));
        }
    };
    let a_test_failed = !status.success()
        && std::fs::read_to_string(report).is_ok_and(|xml| !Report::of(&xml).failed.is_empty());
    end_of(
        status.success(),
        status.code(),
        a_test_failed,
        format!("{what} failed in {} ({status})", tree.display()),
    )
}

/// A build, which has no report: it passes or it breaks.
fn build_in(tree: &Path, command: Command) -> Result<()> {
    match run_in(tree, command, "the build", Path::new("")) {
        RunEnd::Passed => Ok(()),
        RunEnd::TestsFailed(said) | RunEnd::Broke(said) => Err(said),
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
        build_in(tree(which), nextest(tree(which), &build))?;
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
            let job = run_job(at, &request);
            if let Some(error) = &job.error {
                if !request.keep_going {
                    return Err(error.clone());
                }
                println!(
                    "== the {} failed: {error}; carrying on ==",
                    name(which, &label)
                );
            }
            // What the run reported, if it got as far as a report.
            let read = Report::read(&report);
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
            attempts[which as usize] = Some(attempt_of(job, read)?);
        }
        let [Some(base), Some(head)] = attempts else {
            return Err("a round ran only one side".to_owned());
        };
        rounds.push(Round { base, head });
    }
    let (compared, skipped) = comparable(&rounds);
    let failed = failed_rounds(&rounds, &label);
    let left_out = skipped_text(&skipped);
    if !failed.is_empty() {
        println!("\n{failed}");
    }
    if !left_out.is_empty() {
        println!("\n{left_out}");
    }
    let (base_runs, head_runs) = (&compared.base, &compared.head);
    if base_runs.is_empty() {
        crate::steps::append_job_summary(&format!(
            "### Two trees on one runner\n\n```text\n{left_out}{failed}```\n"
        ));
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
    if !skipped.is_empty() {
        // Again at the end, where the log is read from: the comparison is
        // of the other tests, and a reader must not take it for all of them.
        println!("{left_out}");
    }
    crate::steps::append_job_summary(&format!(
        "### Two trees on one runner\n\n```text\n{left_out}{}\n{table}\n{failed}```\n",
        rounds_table(&compared)
    ));
    conclusion(rounds.len(), compared.numbers.len())
}

/// What the run ends as, after its comparison is printed: the measurement is
/// kept, and a failing test is still a failure. `compared` counts the rounds
/// compared, those in which the same tests failed on both sides included:
/// those tests are left out and named, and are not a failure of the run.
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

/// One side's run of a round: `job` is how the tests ended, and `report`
/// the report it left, if it left one. A run that failed is a failed
/// attempt, whatever its report says; one that passed and left no report
/// is an error.
pub(crate) fn attempt_of(job: JobEnd, report: Result<Report>) -> Result<Attempt> {
    match job.error {
        None => report.map(|report| Attempt::Finished(report.cases)),
        Some(error) => Ok(Attempt::Failed(failure_of(
            report.ok(),
            error,
            job.by_tests,
        ))),
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
        Attempt, Compared, FailedTest, Failure, JobEnd, Report, Request, Round, RunEnd, Skipped,
        TEST_RUN_FAILED, Which, attempt_of, comparable, conclusion, drive_job, end_of,
        failed_rounds, failure_of, medians, order, rounds_table, skipped_text,
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

    /// A failed attempt of the old kind: a test failed in a run whose report
    /// listed that one test.
    fn failed(tests: &[&str]) -> Attempt {
        Attempt::Failed(Failure {
            tests: tests
                .iter()
                .map(|id| FailedTest {
                    id: (*id).to_owned(),
                    message: "boom".to_owned(),
                })
                .collect(),
            error: "the tests failed".to_owned(),
            cases: Vec::new(),
            by_tests: false,
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
        let (compared, skipped) = comparable(&rounds());
        assert_eq!(compared.numbers, [1, 4]);
        assert!(skipped.is_empty());
        // The round the base finished and the head failed is not in the
        // base's medians either.
        assert_eq!(compared.base.len(), 2);
        assert!((compared.base[1][0].seconds - 0.7).abs() < 1e-9);
        assert!((compared.head[0][0].seconds - 0.6).abs() < 1e-9);
        let (none, _) = comparable(&rounds()[1..3]);
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

    /// A report as nextest writes it, for the tests `(name, failed)` of one
    /// binary: a failed test has a failure whose text is `message` and a
    /// line more.
    fn xml(tests: &[(&str, Option<&str>)]) -> String {
        let mut text = String::from("<testsuites name=\"nextest-run\">\n<testsuite name=\"b\">\n");
        for (name, failure) in tests {
            match failure {
                None => text.push_str(&format!(
                    "<testcase name=\"{name}\" classname=\"b\" time=\"0.1\"/>\n"
                )),
                Some(message) => text.push_str(&format!(
                    "<testcase name=\"{name}\" classname=\"b\" time=\"0.1\">\n\
                     <failure type=\"test failure with exit code 101\">{message}\nsecond line</failure>\n\
                     </testcase>\n"
                )),
            }
        }
        text.push_str("</testsuite>\n</testsuites>\n");
        text
    }

    /// A side's failed attempt from a report of its tests: failed in a run
    /// that ended by its tests failing, or in one that broke otherwise.
    fn tried(tests: &[(&str, Option<&str>)], by_tests: bool) -> Attempt {
        Attempt::Failed(failure_of(
            Some(Report::of(&xml(tests))),
            "the tests failed".to_owned(),
            by_tests,
        ))
    }

    fn passed_report(names: &[&str]) -> Attempt {
        let tests: Vec<(&str, Option<&str>)> = names.iter().map(|name| (*name, None)).collect();
        Attempt::Finished(Report::of(&xml(&tests)).cases)
    }

    #[test]
    fn a_report_names_each_failed_test_with_the_first_line_of_its_message() {
        let report = Report::of(crate::junit::tests::REPORT);
        let failed: Vec<(&str, &str)> = report
            .failed
            .iter()
            .map(|test| (test.id.as_str(), test.message.as_str()))
            .collect();
        assert_eq!(
            failed,
            [
                (
                    "cortex-agent::it loop_::stops_at_the_refused_call",
                    "assertion `left == right` failed"
                ),
                // A timeout has no text: its kind is what is left.
                ("cortex-agent::it loop_::never_ends", "test timeout"),
            ]
        );
        // A test that passed on its retry did not fail.
        assert_eq!(report.cases.len(), 7);
    }

    #[test]
    fn a_failed_run_names_the_tests_its_report_says_failed() {
        let report = Report::of(&xml(&[("ok", None), ("bad", Some("it broke"))]));
        let failure = failure_of(Some(report), "the tests failed".to_owned(), true);
        assert_eq!(
            failure.tests,
            [FailedTest {
                id: "b bad".to_owned(),
                message: "it broke".to_owned()
            }]
        );
        assert_eq!(failure.cases.len(), 2);
        assert!(failure.by_tests);
        // No report: nothing to name, and nothing says the tests were all.
        let none = failure_of(None, "x".to_owned(), true);
        assert!(none.tests.is_empty() && none.cases.is_empty() && !none.by_tests);
        // A report that names no failed test explains nothing either.
        let clean = failure_of(Some(Report::of(&xml(&[("ok", None)]))), "x".to_owned(), true);
        assert!(!clean.by_tests);
    }

    fn job(error: Option<&str>, by_tests: bool) -> JobEnd {
        JobEnd {
            error: error.map(str::to_owned),
            by_tests,
        }
    }

    #[test]
    fn a_run_that_failed_is_a_failed_attempt_whatever_its_report_says() {
        let report = || Ok(Report::of(&xml(&[("bad", Some("it broke"))])));
        let attempt = attempt_of(job(Some("the tests failed"), true), report()).unwrap();
        assert!(matches!(&attempt, Attempt::Failed(failure) if failure.by_tests));
        // No report at all.
        let attempt = attempt_of(
            job(Some("the tests failed"), true),
            Err("none".to_owned()),
        );
        assert_eq!(attempt, Ok(failed(&[])));
        // A run that passed is finished, and one that left no report is an
        // error, not a failed attempt: nothing says it failed.
        assert!(matches!(
            attempt_of(job(None, true), report()),
            Ok(Attempt::Finished(_))
        ));
        assert_eq!(
            attempt_of(job(None, true), Err("none".to_owned())),
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

    // What a test that failed on both sides does to a round.

    #[test]
    fn a_test_that_failed_on_both_sides_is_left_out_and_the_others_are_compared() {
        let both = || Round {
            base: tried(&[("slow", None), ("flaky", Some("left 1 right 2"))], true),
            head: tried(&[("slow", None), ("flaky", Some("left 3 right 4"))], true),
        };
        let rounds = vec![
            Round {
                base: passed_report(&["slow", "flaky"]),
                head: passed_report(&["slow", "flaky"]),
            },
            both(),
            Round {
                base: passed_report(&["slow", "flaky"]),
                head: passed_report(&["slow", "flaky"]),
            },
            both(),
        ];
        let (compared, skipped) = comparable(&rounds);
        // Every round is compared, the test in none of them: not in the
        // rounds it failed in, and not in those it passed in.
        assert_eq!(compared.numbers, [1, 2, 3, 4]);
        for cases in compared.base.iter().chain(&compared.head) {
            let names: Vec<&str> = cases.iter().map(|case| case.name.as_str()).collect();
            assert_eq!(names, ["slow"]);
        }
        assert_eq!(
            skipped,
            [Skipped {
                id: "b flaky".to_owned(),
                rounds: vec![2, 4],
                base: "left 1 right 2".to_owned(),
                head: "left 3 right 4".to_owned(),
            }]
        );
        // Not a failure of the run, and not in the list of failed rounds.
        assert_eq!(failed_rounds(&rounds, "main"), "");
        assert_eq!(conclusion(rounds.len(), compared.numbers.len()), Ok(()));
        let text = skipped_text(&skipped);
        assert!(text.contains("1 TEST(S) FAILED ON BOTH SIDES"), "{text}");
        assert!(text.contains("b flaky (round 2, 4)"), "{text}");
        assert!(text.contains("base: left 1 right 2"), "{text}");
        assert!(text.contains("head: left 3 right 4"), "{text}");
        assert_eq!(skipped_text(&[]), "");
    }

    #[test]
    fn a_test_that_failed_on_one_side_only_is_still_a_failed_round() {
        let rounds = vec![
            // The head passed what the base failed.
            Round {
                base: tried(&[("a", None), ("t", Some("x"))], true),
                head: passed_report(&["a", "t"]),
            },
            // The base passed what the head failed.
            Round {
                base: passed_report(&["a", "t"]),
                head: tried(&[("a", None), ("t", Some("x"))], true),
            },
            // The head does not have the test the base failed: missing is
            // not failed.
            Round {
                base: tried(&[("a", None), ("t", Some("x"))], true),
                head: passed_report(&["a"]),
            },
            // The head failed a test the base did not have.
            Round {
                base: passed_report(&["a"]),
                head: tried(&[("a", None), ("t", Some("x"))], true),
            },
            // Two failed on the base, one of them on the head.
            Round {
                base: tried(&[("t", Some("x")), ("u", Some("y"))], true),
                head: tried(&[("t", Some("x")), ("u", None)], true),
            },
        ];
        for (index, round) in rounds.iter().enumerate() {
            let (compared, skipped) = comparable(std::slice::from_ref(round));
            assert_eq!(compared, Compared::default(), "round {}", index + 1);
            assert!(skipped.is_empty(), "round {}", index + 1);
            assert!(
                !failed_rounds(std::slice::from_ref(round), "main").is_empty(),
                "round {}",
                index + 1
            );
        }
        let (compared, skipped) = comparable(&rounds);
        assert_eq!(compared, Compared::default());
        assert!(skipped.is_empty());
        let error = conclusion(rounds.len(), compared.numbers.len()).unwrap_err();
        assert!(error.contains("5 of 5 rounds"), "{error}");
    }

    #[test]
    fn a_failure_that_the_tests_do_not_explain_is_not_a_test_failing_on_both_sides() {
        let same = |by_tests_head: bool| Round {
            base: tried(&[("a", None), ("t", Some("x"))], true),
            head: tried(&[("a", None), ("t", Some("x"))], by_tests_head),
        };
        // The control: both sides' runs ended by the test failing.
        let (compared, skipped) = comparable(&[same(true)]);
        assert_eq!((compared.numbers.len(), skipped.len()), (1, 1));
        // The head's other run broke too (the system tests' build, say).
        let (compared, skipped) = comparable(&[same(false)]);
        assert_eq!(compared, Compared::default());
        assert!(skipped.is_empty());
        let text = failed_rounds(&[same(false)], "main");
        assert!(
            text.contains("round 1, head: b t failed, and the run failed otherwise too"),
            "{text}"
        );
        // A side that wrote no report, and one whose report lists nothing it
        // failed.
        let silent = Round {
            base: tried(&[("t", Some("x"))], true),
            head: Attempt::Failed(failure_of(None, "no report".to_owned(), true)),
        };
        assert_eq!(comparable(&[silent]).0, Compared::default());
        let clean = Round {
            base: tried(&[("t", Some("x"))], true),
            head: tried(&[("t", None)], true),
        };
        assert_eq!(comparable(&[clean]).0, Compared::default());
    }

    // The two runs of a side's job.

    #[test]
    fn a_failure_in_the_first_run_does_not_stop_the_second_with_keep_going() {
        let mut ran = Vec::new();
        let mut after = Vec::new();
        let end = drive_job(
            2,
            true,
            |index| {
                ran.push(index);
                if index == 0 {
                    RunEnd::TestsFailed("tiers 1 and 2 failed".to_owned())
                } else {
                    RunEnd::Passed
                }
            },
            |index, last| {
                after.push((index, last));
                Ok(())
            },
        );
        assert_eq!(ran, [0, 1]);
        // The first run's report is set aside, the second's brought together.
        assert_eq!(after, [(0, false), (1, true)]);
        assert_eq!(end, job(Some("tiers 1 and 2 failed"), true));
        // Failing in both runs says both.
        let end = drive_job(
            2,
            true,
            |index| RunEnd::TestsFailed(format!("run {index} failed")),
            |_, _| Ok(()),
        );
        assert_eq!(end, job(Some("run 0 failed; run 1 failed"), true));
    }

    #[test]
    fn without_keep_going_the_first_failing_run_ends_the_job() {
        let mut ran = Vec::new();
        let mut after = Vec::new();
        let end = drive_job(
            2,
            false,
            |index| {
                ran.push(index);
                RunEnd::TestsFailed("failed".to_owned())
            },
            |index, last| {
                after.push((index, last));
                Ok(())
            },
        );
        assert_eq!(ran, [0]);
        // Its report is brought together with the earlier ones' all the same.
        assert_eq!(after, [(0, true)]);
        assert_eq!(end.error.as_deref(), Some("failed"));
    }

    #[test]
    fn a_run_that_broke_or_a_report_that_could_not_be_put_together_is_not_the_tests() {
        let end = drive_job(
            2,
            true,
            |index| {
                if index == 0 {
                    RunEnd::TestsFailed("tests".to_owned())
                } else {
                    RunEnd::Broke("the build broke".to_owned())
                }
            },
            |_, _| Ok(()),
        );
        assert_eq!(end, job(Some("tests; the build broke"), false));
        let end = drive_job(
            1,
            true,
            |_| RunEnd::TestsFailed("tests".to_owned()),
            |_, _| Err("could not move".to_owned()),
        );
        assert_eq!(end, job(Some("could not move; tests"), false));
        // Passing says nothing was wrong.
        let end = drive_job(2, true, |_| RunEnd::Passed, |_, _| Ok(()));
        assert_eq!(end.error, None);
    }

    #[test]
    fn a_run_is_the_tests_failing_only_by_nextests_code_and_a_failed_test_in_its_report() {
        let said = || "failed".to_owned();
        assert_eq!(end_of(true, Some(0), false, said()), RunEnd::Passed);
        assert_eq!(
            end_of(false, Some(TEST_RUN_FAILED), true, said()),
            RunEnd::TestsFailed(said())
        );
        // The code says tests failed but the report names none (it is of
        // another run, or none): nothing shows which.
        assert_eq!(
            end_of(false, Some(TEST_RUN_FAILED), false, said()),
            RunEnd::Broke(said())
        );
        // A failed test in the report and another code: the build broke
        // after an earlier run's report, or nextest itself failed.
        assert_eq!(
            end_of(false, Some(101), true, said()),
            RunEnd::Broke(said())
        );
        // Killed by a signal: no code.
        assert_eq!(end_of(false, None, true, said()), RunEnd::Broke(said()));
    }
}
