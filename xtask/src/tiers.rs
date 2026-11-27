//! The test-time budget of each tier, checked after a CI test run from the
//! JUnit report nextest writes (`docs/testing-and-ci.md`, "Test tiers").
//!
//! A single test is held to its limit by nextest itself (`.config/nextest.toml`
//! stops a test that runs too long). This is the other half: the time all the
//! tests of a tier take together, and which tests that time is made of.
//!
//! **A budget is held against the time above the start of the tests'
//! processes.** nextest runs every test in a process of its own, and on a
//! hosted Windows runner starting one costs 6 to 12 ms, by the runner's
//! speed. For a thousand unit tests that is 6 to 12 s of a 20 s budget (25 s
//! since 2026-11-20) before any test has done anything, so the raw total said
//! more about the runner a run drew than about the tests. Each run measures
//! its own start cost ([`start_cost`]) and takes it off every test; the
//! report prints both totals, and the cost it used.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::junit::{self, TestCase};
use crate::stall;
use crate::steps;
use crate::workspace;

/// How many of each tier's slowest tests a CI run lists.
const SLOWEST_IN_CI: usize = 5;

/// A test tier that blocks pull requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tier {
    Unit,
    Integration,
    System,
}

/// Which of a run's unit and integration test times is taken as the cost of
/// starting one test's process: the time that this many in a hundred of
/// them were quicker than.
///
/// The quickest tests of a run do next to nothing, so their time is the
/// start of a process. Not the very quickest: one test that happens to be
/// a millisecond quicker than the rest moves the whole floor, and with it a
/// thousand tests' worth of time. In one job that ran one tree twice with
/// each of two settings, the unit tier's figure moved by 1.1 s between the
/// two runs of one setting with the minimum as the floor, where the 1st
/// percentile and the raw totals did not move at all. System tests are not
/// looked at: they start other processes of their own.
const START_PERCENTILE: usize = 1;

/// Below this many unit and integration tests, the report says that its
/// start cost may be more than the start: in a run of a few crates, or of
/// tests chosen by a filter, the quickest may all do real work.
const SMALL_SAMPLE: usize = 500;

/// What starting one test's process cost in this run, in seconds: the
/// [`START_PERCENTILE`]th percentile of its unit and integration tests'
/// times. A run with none of those has no floor to measure: 0. The report
/// asks [`start_of`], which says how many tests it was measured on too.
#[cfg(test)]
fn start_cost(cases: &[TestCase]) -> f64 {
    start_of(cases).0
}

/// The start cost, and how many unit and integration tests it was
/// measured on.
pub(crate) fn start_of(cases: &[TestCase]) -> (f64, usize) {
    let mut times: Vec<f64> = cases
        .iter()
        .filter(|case| Tier::of(&case.binary, &case.name) != Tier::System)
        .map(|case| case.seconds)
        .collect();
    if times.is_empty() {
        return (0.0, 0);
    }
    times.sort_by(f64::total_cmp);
    (times[times.len() * START_PERCENTILE / 100], times.len())
}

/// The OS a report is from. Tier 3 has a larger budget on Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Os {
    Linux,
    Windows,
}

impl Os {
    /// The OS this command is running on.
    pub(crate) fn here() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

impl Tier {
    pub(crate) const ALL: [Tier; 3] = [Tier::Unit, Tier::Integration, Tier::System];

    /// The tier's place in [`Tier::ALL`].
    pub(crate) fn index(self) -> usize {
        match self {
            Tier::Unit => 0,
            Tier::Integration => 1,
            Tier::System => 2,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Tier::Unit => "1, unit",
            Tier::Integration => "2, integration",
            Tier::System => "3, system",
        }
    }

    /// The budget for the tier's total test time, in seconds: the sum of the
    /// times of its tests above the start of their processes, not the wall
    /// time of the run.
    pub(crate) fn budget(self, os: Os) -> f64 {
        match (self, os) {
            // The owner's decision of 2026-11-20, taken in the small fixes:
            // 680 unit tests since wave 12 put an 8 ms Windows runner at 20
            // to 21 s (docs/steps/ci-budgets/answered.md, "Left for the
            // owner"; docs/steps/small-fixes/answered.md).
            (Tier::Unit, Os::Windows) => 25.0,
            (Tier::Unit, Os::Linux) => 20.0,
            (Tier::Integration, _) => 40.0,
            (Tier::System, Os::Windows) => 90.0,
            (Tier::System, Os::Linux) => 60.0,
        }
    }

    /// Where a test lives, and its name, decide its tier.
    pub(crate) fn of(binary: &str, test: &str) -> Tier {
        if test.starts_with("sys_") || test.contains("::sys_") {
            Tier::System
        } else if binary.ends_with("::it") {
            Tier::Integration
        } else {
            Tier::Unit
        }
    }
}

/// Where a run in nextest's `ci` profile leaves its report.
pub(crate) fn ci_report_path() -> PathBuf {
    workspace::nextest_report(&workspace::root(), "ci")
}

/// The step that follows the test run in `cargo xtask ci test`.
///
/// **It does not judge a run that stalled.** The test runs leave what the
/// pressure said of them (`stall::record_path`), and when every task waited
/// on I/O for [`stall::STALLED_SHARE`] of the runs' time or more, away
/// from CI, the tiers' times are the disk's as much as the tests', and the
/// step says `not judged: the machine stalled` with the numbers and neither
/// passes nor fails a budget. On CI (`GITHUB_ACTIONS` set) the budgets are
/// held as they always were, whatever the pressure ([`stall::judge`]).
pub(crate) fn check_ci_report() -> Result<()> {
    check_ci_report_in(
        &workspace::root(),
        std::env::var_os("GITHUB_ACTIONS").is_some(),
        true,
    )
}

/// [`check_ci_report`] for the tree at `root`, run on CI or not, with the
/// job's summary page written or not.
fn check_ci_report_in(root: &Path, in_actions: bool, summary: bool) -> Result<()> {
    let records = stall::read_records(&stall::record_path(root));
    check_report_with(
        &workspace::nextest_report(root, "ci"),
        Os::here(),
        SLOWEST_IN_CI,
        stall::judge(in_actions, &records),
        summary,
        None,
    )
}

/// One part of the blocking test job, where the job runs as two: on
/// Windows a job of tiers 1 and 2 and a job of the system tests, each run
/// from the build job's archive of the tests (`cargo xtask ci test PART
/// --from-archive DIR`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// Tiers 1 and 2.
    Tiers,
    /// Tier 3, after the start probe ([`start_probe_path`]).
    System,
}

impl Part {
    /// `tiers` or `system`, as `ci test` takes them.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "tiers" => Some(Part::Tiers),
            "system" => Some(Part::System),
            _ => None,
        }
    }

    /// The tiers the part's report holds, each of them and no other.
    pub(crate) fn tiers(self) -> &'static [Tier] {
        match self {
            Part::Tiers => &[Tier::Unit, Tier::Integration],
            Part::System => &[Tier::System],
        }
    }
}

/// Where the system part keeps the report of its start probe: the unit
/// tests it runs first, on its own runner, to measure what the start of a
/// test's process costs there. The system tests' report has no unit or
/// integration test to measure it from, and a start measured on the other
/// part's runner is not this runner's (hosted runners differ by a factor
/// of two). It is beside the job's report, and is not part of it.
pub(crate) fn start_probe_path(root: &Path) -> PathBuf {
    workspace::nextest_report(root, "ci").with_file_name(START_PROBE_FILE)
}

/// The name of the start probe's report, beside the job's own: in the
/// system tests' job, and in its artifact, `junit-windows-system`.
pub(crate) const START_PROBE_FILE: &str = "start-probe.xml";

/// The step that follows a part's test run in `cargo xtask ci test PART
/// --from-archive DIR`: [`check_ci_report`] for the tiers of `part` alone.
/// The report must hold tests of each of the part's tiers and of no other,
/// and the system part's tier is held to its budget above the start its
/// probe measured on the same runner. A probe that failed or ran nothing
/// fails the check: it never only moves the figure.
pub(crate) fn check_ci_part(part: Part) -> Result<()> {
    check_ci_part_in(
        &workspace::root(),
        std::env::var_os("GITHUB_ACTIONS").is_some(),
        true,
        part,
    )
}

/// [`check_ci_part`] for the tree at `root`, run on CI or not, with the
/// job's summary page written or not.
fn check_ci_part_in(root: &Path, in_actions: bool, summary: bool, part: Part) -> Result<()> {
    let records = stall::read_records(&stall::record_path(root));
    check_report_with(
        &workspace::nextest_report(root, "ci"),
        Os::here(),
        SLOWEST_IN_CI,
        stall::judge(in_actions, &records),
        summary,
        Some((part, &start_probe_path(root))),
    )
}

/// The start probe's cases, read from `report`, refused when the probe
/// ran nothing or too few tests to measure a start on ([`SMALL_SAMPLE`]),
/// when one of its tests failed, or when it holds anything but unit tests.
pub(crate) fn read_probe(report: &Path) -> Result<Vec<TestCase>> {
    let cases = junit::read(report).map_err(|error| format!("the start probe: {error}"))?;
    if let Some(case) = cases.iter().find(|case| case.failed) {
        return Err(format!(
            "the start probe's test {} failed: the start of a test's process is not measured \
             from a run that failed",
            case.id()
        ));
    }
    if let Some(case) = cases
        .iter()
        .find(|case| Tier::of(&case.binary, &case.name) != Tier::Unit)
    {
        return Err(format!(
            "the start probe ran {}, which is not a unit test: the probe is unit tests only",
            case.id()
        ));
    }
    if cases.len() < SMALL_SAMPLE {
        return Err(format!(
            "the start probe ran {} tests, fewer than the {SMALL_SAMPLE} a start is measured on: \
             choose more for it (`checks::START_PROBE`), rather than hold tier 3 above a figure \
             of too few",
            cases.len()
        ));
    }
    Ok(cases)
}

/// A part's report holds tests of each of its tiers, and of no other: a
/// filter that chose the wrong tests, or none of a tier, is refused, and is
/// never judged as a tier that took no time.
fn check_part_holds(cases: &[TestCase], part: Part) -> Result<()> {
    if let Some(case) = cases
        .iter()
        .find(|case| !part.tiers().contains(&Tier::of(&case.binary, &case.name)))
    {
        return Err(format!(
            "the report of the job's {} holds {}, of tier {}, which is not one of its tiers",
            part_name(part),
            case.id(),
            Tier::of(&case.binary, &case.name).name()
        ));
    }
    for tier in part.tiers() {
        if !cases
            .iter()
            .any(|case| Tier::of(&case.binary, &case.name) == *tier)
        {
            return Err(format!(
                "the report of the job's {} has no test of tier {}: it did not run",
                part_name(part),
                tier.name()
            ));
        }
    }
    Ok(())
}

fn part_name(part: Part) -> &'static str {
    match part {
        Part::Tiers => "tiers 1 and 2",
        Part::System => "system tests",
    }
}

/// What one tier check covers: the tiers it shows and judges, and the
/// start of a test's process that is taken off each test.
struct Scope {
    tiers: &'static [Tier],
    start: f64,
    /// How many tests the start was measured on.
    sample: usize,
    /// The start was measured by the start probe, not on the report's own
    /// unit and integration tests.
    probe: bool,
    /// The start some of the report's cases carry of their own runner (a
    /// split run's system tests, made one report with tiers 1 and 2:
    /// [`TestCase::start`]), which they are held above in place of `start`.
    own: Option<f64>,
}

impl Scope {
    /// Every tier, above the start the report's own tests measured.
    fn whole(cases: &[TestCase]) -> Self {
        let (start, sample) = start_of(cases);
        Self {
            tiers: &Tier::ALL,
            start,
            sample,
            probe: false,
            own: cases.iter().find_map(|case| case.start),
        }
    }

    /// `part`'s tiers, above the start its report measured or, for the
    /// system tests, the start the probe at `probe` measured.
    fn of_part(cases: &[TestCase], part: Part, probe: &Path) -> Result<Self> {
        check_part_holds(cases, part)?;
        let (start, sample, probe) = match part {
            Part::Tiers => {
                let (start, sample) = start_of(cases);
                (start, sample, false)
            }
            Part::System => {
                let (start, sample) = start_of(&read_probe(probe)?);
                (start, sample, true)
            }
        };
        Ok(Self {
            tiers: part.tiers(),
            start,
            sample,
            probe,
            own: None,
        })
    }

    fn start_line(&self) -> String {
        let mut line = start_line_of(self.start, self.sample, self.probe);
        if let Some(own) = self.own {
            line.push_str(&format!(
                "; the system tests, run in a job of their own, above their own runner's start, \
                 {:.0} ms a test (its start probe's)",
                1000.0 * own
            ));
        }
        line
    }
}

/// Prints the tier totals of a report and each tier's `slowest` slowest
/// tests, and fails when a tier is far over its budget. A report of
/// system tests alone with a start probe's beside it ([`START_PROBE_FILE`])
/// is a split job's system tests, and is read as that part, above the
/// probe's start.
pub(crate) fn check_report(report: &Path, os: Os, slowest: usize) -> Result<()> {
    let probe = report.with_file_name(START_PROBE_FILE);
    let system_alone = junit::read(report).is_ok_and(|cases| {
        cases
            .iter()
            .all(|case| Tier::of(&case.binary, &case.name) == Tier::System)
    });
    let part = (system_alone && probe.is_file()).then_some((Part::System, probe.as_path()));
    check_report_with(report, os, slowest, None, true, part)
}

/// [`check_report`], or, for a run that `stalled`, the same tables with no
/// verdict: no mark on a tier, no warning, no failure, and a line that
/// says why. With `part`, the report is that part's of the job, and the
/// path its start probe's report ([`check_ci_part`]).
fn check_report_with(
    report: &Path,
    os: Os,
    slowest: usize,
    stalled: Option<stall::Stalled>,
    summary: bool,
    part: Option<(Part, &Path)>,
) -> Result<()> {
    let cases = junit::read(report)?;
    let scope = match part {
        None => Scope::whole(&cases),
        Some((part, probe)) => Scope::of_part(&cases, part, probe)?,
    };
    let judged = |tier: Tier| stalled.is_none_or(|stalled| stalled.judges(tier));
    let (table, over) = summarise_in(&cases, os, judged, &scope);
    if let Some(stalled) = &stalled {
        println!("{}", stalled.message());
    }
    print!("{table}");
    print!("{}", crates_text(&cases, CRATES_SHOWN, scope.start));
    print!("{}", slowest_tests(&cases, slowest));
    if summary {
        steps::append_job_summary(&format!(
            "### Test time by tier\n\n{}{}\n{}\n{}\n",
            stalled.map_or_else(String::new, |stalled| format!("{}\n\n", stalled.message())),
            markdown_in(&cases, os, &scope),
            crates_markdown(&cases, scope.start),
            slowest_markdown(&cases, slowest)
        ));
    }
    // The tiers a stalled run measured have no verdict (`over` leaves them
    // out); the others are judged as ever.

    let names = |wanted: Verdict| -> Vec<&str> {
        let tiers = over.iter().filter(|(_, verdict)| *verdict == wanted);
        tiers.map(|(tier, _)| tier.name()).collect()
    };
    let in_actions = std::env::var_os("GITHUB_ACTIONS").is_some();
    const ADVICE: &str = "Make the tests faster or move the slow ones to a non-blocking tier \
                          (docs/testing-and-ci.md). Budgets are not loosened quietly.";

    let (start, sample) = (scope.start, scope.sample);
    if !over.is_empty() {
        println!("{}", runner_line(os, start, sample));
    }
    if let Some(message) = start_warning(os, start, sample) {
        if in_actions {
            println!("::warning title=Test start cost high::{message}");
        } else {
            println!("warning: {message}");
        }
    }
    let warned = names(Verdict::OverBudget);
    if !warned.is_empty() {
        let message = format!(
            "over its time budget: tier {}. {ADVICE}",
            warned.join(", tier ")
        );
        // An annotation, so that it shows on the pull request and not only in the log.
        if in_actions {
            println!("::warning title=Test tier over budget::{message}");
        } else {
            println!("warning: {message}");
        }
    }
    let failed = names(Verdict::Failing);
    if failed.is_empty() {
        return Ok(());
    }
    let message = format!(
        "more than {FAIL_AT} times over its time budget: tier {}. {ADVICE}",
        failed.join(", tier ")
    );
    if in_actions {
        println!("::error title=Test tier far over budget::{message}");
    }
    Err(message)
}

/// What [`check_report`] prints, as text: the tiers against `os`'s budgets
/// (a tier over one marked), each tier by crate, and each tier's `slowest`
/// tests, and for a tier over its budget what the run's start cost says of
/// its runner. It writes nothing and judges nothing: `try-result` shows a
/// run's report with it, whatever the verdict.
pub(crate) fn report_text(cases: &[TestCase], os: Os, slowest: usize) -> String {
    let (mut text, over) = summarise(cases, os);
    text.push_str(&crates_text(cases, CRATES_SHOWN, start_of(cases).0));
    text.push_str(&slowest_tests(cases, slowest));
    if !over.is_empty() {
        let (start, sample) = start_of(cases);
        let _ = writeln!(text, "{}", runner_line(os, start, sample));
    }
    text
}

/// What a run's start cost says about its runner, for a run whose tiers
/// are over budget: hosted runners differ in speed by a factor of two, and
/// a slow one reads every tier slow, the unit tier too, with every test as
/// it was. The figures are what the fifth tooling step's runs read
/// (docs/steps/tooling-5/ci.md).
pub(crate) fn runner_line(os: Os, start: f64, sample: usize) -> String {
    let ms = (1000.0 * start).round();
    if sample < SMALL_SAMPLE {
        return format!(
            "runner: a start of {ms:.0} ms a test, measured on {sample} tests: too few to tell the \
             runner's speed. The whole workspace's run tells it."
        );
    }
    let (usual, slow_from, how_slow) = match os {
        // Linux: 3 to 4 ms a test on most runners; a slow one read the
        // integration tier 1.8 times a usual one's.
        Os::Linux => ("3 to 4 ms", 5.0, "1.8 times"),
        // Windows: 6 to 12 ms. At 10 ms the system tier read 77 to 90 s;
        // at 12 ms, 138.8 s, every tier about twice the 10 ms runner's.
        Os::Windows => ("6 to 10 ms", 11.0, "twice"),
    };
    let speed = if ms >= slow_from {
        format!(
            "a slow runner: tiers likely about {how_slow} what a usual runner ({usual} a test) \
             reads, with the same tests"
        )
    } else {
        format!("a usual runner ({usual} a test): the time is likely the tests'")
    };
    format!(
        "runner: {ms:.0} ms runner, {speed}. Compare a change only on runners of the same start, \
         or with both trees on one runner (`try -f against=main`)."
    )
}

/// The start of a test's process above which a run's report warns: a cost
/// every test process shares (a larger binary that loads more slowly, a
/// global initialiser, a heavier harness) rises with the start cost, which
/// is taken off every test, so the budgets do not see it. Hosted runners
/// start a test in 3 to 5 ms on Linux and 6 to 12 ms on Windows
/// (docs/steps/tooling-5/ci.md); these are above the slowest of each.
pub(crate) fn start_ceiling(os: Os) -> f64 {
    match os {
        Os::Linux => 0.008,
        Os::Windows => 0.015,
    }
}

/// The warning for a run whose start cost is above [`start_ceiling`], or
/// `None`. A run of too few tests to measure its start says nothing: its
/// quickest tests may do real work.
pub(crate) fn start_warning(os: Os, start: f64, sample: usize) -> Option<String> {
    let ceiling = start_ceiling(os);
    if sample < SMALL_SAMPLE || start <= ceiling {
        return None;
    }
    Some(format!(
        "the start of a test's process, {:.0} ms, is above {:.0} ms, more than the slowest runner's: a cost every test process shares (a larger test binary, a global initialiser, a heavier harness) is in it, and is taken off every test unseen by the budgets. Compare the start of `main`'s last run, and look for what every test now pays.",
        1000.0 * start,
        1000.0 * ceiling,
    ))
}

/// A tier fails the job only when it is this many times over its budget.
/// Hosted runners vary by tens of percent from one run to the next, more on
/// Windows, so a failure at exactly the budget would turn a tier that is
/// near its limit into red builds at random. Between the budget and this
/// factor, the job passes with a warning on the pull request.
const FAIL_AT: f64 = 1.5;

/// What a tier's total says about it, when it is over budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    /// Over the budget: a warning.
    OverBudget,
    /// Over [`FAIL_AT`] times the budget: a failure.
    Failing,
}

fn verdict(seconds: f64, budget: f64) -> Option<Verdict> {
    if seconds > FAIL_AT * budget {
        Some(Verdict::Failing)
    } else if seconds > budget {
        Some(Verdict::OverBudget)
    } else {
        None
    }
}

/// What a tier's tests took together.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Totals {
    pub(crate) count: usize,
    /// The sum of the tests' times.
    pub(crate) seconds: f64,
    /// The sum of what each test took above the start cost, a test that
    /// took less counting as nothing. This is what the budget is held to.
    pub(crate) above_start: f64,
}

pub(crate) fn total(cases: &[TestCase], tier: Tier, start: f64) -> Totals {
    let of_tier = cases
        .iter()
        .filter(|case| Tier::of(&case.binary, &case.name) == tier);
    of_tier.fold(
        Totals {
            count: 0,
            seconds: 0.0,
            above_start: 0.0,
        },
        |totals, case| Totals {
            count: totals.count + 1,
            seconds: totals.seconds + case.seconds,
            above_start: totals.above_start + case.above(start),
        },
    )
}

/// The line that says what the start of a test's process was taken to
/// cost, and from what: the run's own unit and integration tests, or the
/// system part's start probe when `probe`.
fn start_line_of(start: f64, sample: usize, probe: bool) -> String {
    let from = if probe {
        format!(
            "the {START_PERCENTILE}st percentile of the {sample} unit tests' times of the start \
             probe, run first on this runner to measure it"
        )
    } else {
        format!(
            "the {START_PERCENTILE}st percentile of the run's {sample} unit and integration tests' times"
        )
    };
    let mut line = format!(
        "process start: {:.0} ms a test ({from}), taken off every test before a tier is held to \
         its budget",
        1000.0 * start,
    );
    if sample < SMALL_SAMPLE {
        line.push_str(&format!(
            ". It was measured on fewer than {SMALL_SAMPLE} tests: in a run of a few crates, or \
             of tests a filter chose, the quickest may do more than start, and the figures \
             above the start are then too low"
        ));
    }
    line
}

/// The table for the terminal, and the tiers that are over budget.
fn summarise(cases: &[TestCase], os: Os) -> (String, Vec<(Tier, Verdict)>) {
    summarise_marked(cases, os, |_| true)
}

/// [`summarise`], with a mark on a tier that is over its budget only when
/// `marked`: a run that is not judged has none.
fn summarise_marked(
    cases: &[TestCase],
    os: Os,
    marked: impl Fn(Tier) -> bool,
) -> (String, Vec<(Tier, Verdict)>) {
    summarise_in(cases, os, marked, &Scope::whole(cases))
}

/// [`summarise_marked`], of the tiers of `scope` alone, above its start.
fn summarise_in(
    cases: &[TestCase],
    os: Os,
    marked: impl Fn(Tier) -> bool,
    scope: &Scope,
) -> (String, Vec<(Tier, Verdict)>) {
    let start = scope.start;
    let mut table = format!(
        "  {}\n  tier             tests   time    above start  budget  used\n",
        scope.start_line()
    );
    let mut over = Vec::new();
    for &tier in scope.tiers {
        let Totals {
            count,
            seconds,
            above_start,
        } = total(cases, tier, start);
        let budget = tier.budget(os);
        let used = 100.0 * above_start / budget;
        let verdict = verdict(above_start, budget).filter(|_| marked(tier));
        let mark = match verdict {
            None => "",
            Some(Verdict::OverBudget) => "  over budget (warning)",
            Some(Verdict::Failing) => "  FAR OVER BUDGET",
        };
        let _ = writeln!(
            table,
            "  {:<15}  {count:>5}  {seconds:>5.1} s  {above_start:>9.1} s  {budget:>4.0} s  {used:>3.0}%{mark}",
            tier.name(),
        );
        if let Some(verdict) = verdict {
            over.push((tier, verdict));
        }
    }
    (table, over)
}

#[cfg(test)]
fn markdown(cases: &[TestCase], os: Os) -> String {
    markdown_in(cases, os, &Scope::whole(cases))
}

/// The tiers' table of the job's summary page, of the tiers of `scope`
/// alone, above its start.
fn markdown_in(cases: &[TestCase], os: Os, scope: &Scope) -> String {
    let start = scope.start;
    let mut table = format!(
        "The {}.\n\n| Tier | Tests | Time | Above start | Budget | Used |\n\
         |---|---:|---:|---:|---:|---:|\n",
        scope
            .start_line()
            .replacen("process start: ", "start of a test's process: ", 1)
    );
    for &tier in scope.tiers {
        let Totals {
            count,
            seconds,
            above_start,
        } = total(cases, tier, start);
        let budget = tier.budget(os);
        let _ = writeln!(
            table,
            "| {} | {count} | {seconds:.1} s | {above_start:.1} s | {budget:.0} s | {:.0}% |",
            tier.name(),
            100.0 * above_start / budget,
        );
    }
    table
}

/// How many crates the terminal report lists by name; the rest are one
/// line. The job summary lists them all.
const CRATES_SHOWN: usize = 12;

/// The crate a test binary belongs to: `cineris-agent` for
/// `cineris-agent::it`, `cineris-cli` for `cineris-cli::bin/cineris`.
pub(crate) fn crate_of(binary: &str) -> &str {
    binary.split("::").next().unwrap_or(binary)
}

/// Each crate's time in each tier, above the start of the tests'
/// processes, in the order of [`Tier::ALL`]. The crates come largest total
/// first, and crates with the same total in name order.
pub(crate) fn by_crate(cases: &[TestCase], start: f64) -> Vec<(String, [f64; 3])> {
    let mut crates: Vec<(String, [f64; 3])> = Vec::new();
    for case in cases {
        let name = crate_of(&case.binary);
        let index = match crates.iter().position(|(crate_name, _)| crate_name == name) {
            Some(index) => index,
            None => {
                crates.push((name.to_owned(), [0.0; 3]));
                crates.len() - 1
            }
        };
        crates[index].1[Tier::of(&case.binary, &case.name).index()] += case.above(start);
    }
    crates.sort_by(|(a_name, a), (b_name, b)| {
        let (a_sum, b_sum): (f64, f64) = (a.iter().sum(), b.iter().sum());
        b_sum.total_cmp(&a_sum).then_with(|| a_name.cmp(b_name))
    });
    crates
}

/// Each tier's time above the start, split by crate: where to look when a
/// tier grows. The first `shown` crates by name, the rest summed in one
/// line.
fn crates_text(cases: &[TestCase], shown: usize, start: f64) -> String {
    let crates = by_crate(cases, start);
    let mut text = String::from("\n  by crate, above start           unit  integration  system\n");
    for (name, seconds) in crates.iter().take(shown) {
        let _ = writeln!(
            text,
            "    {name:<28} {:>6.1}  {:>11.1}  {:>6.1}",
            seconds[0], seconds[1], seconds[2]
        );
    }
    if crates.len() > shown {
        let mut rest = [0.0; 3];
        for (_, seconds) in &crates[shown..] {
            for (sum, each) in rest.iter_mut().zip(seconds) {
                *sum += each;
            }
        }
        let _ = writeln!(
            text,
            "    {:<28} {:>6.1}  {:>11.1}  {:>6.1}",
            format!("{} more crates", crates.len() - shown),
            rest[0],
            rest[1],
            rest[2]
        );
    }
    text
}

fn crates_markdown(cases: &[TestCase], start: f64) -> String {
    let mut table = String::from(
        "Each tier's time above the start, by crate:\n\n\
         | Crate | Unit | Integration | System |\n|---|---:|---:|---:|\n",
    );
    for (name, seconds) in by_crate(cases, start) {
        let _ = writeln!(
            table,
            "| `{name}` | {:.1} s | {:.1} s | {:.1} s |",
            seconds[0], seconds[1], seconds[2]
        );
    }
    table
}

/// The `count` slowest tests of a tier, slowest first. Tests that took the
/// same time are in name order, so the list is the same for the same report.
fn slowest(cases: &[TestCase], tier: Tier, count: usize) -> Vec<&TestCase> {
    let mut of_tier: Vec<&TestCase> = cases
        .iter()
        .filter(|case| Tier::of(&case.binary, &case.name) == tier)
        .collect();
    of_tier.sort_by(|a, b| {
        b.seconds
            .total_cmp(&a.seconds)
            .then_with(|| a.binary.cmp(&b.binary))
            .then_with(|| a.name.cmp(&b.name))
    });
    of_tier.truncate(count);
    of_tier
}

/// Each tier's slowest tests, for the terminal and the CI log. A CI log
/// shows no time for a test under two seconds, and a tier's total is made
/// of hundreds of such tests: this is where its time is.
fn slowest_tests(cases: &[TestCase], count: usize) -> String {
    let mut lines = String::new();
    if count == 0 {
        return lines;
    }
    for tier in Tier::ALL {
        let tests = slowest(cases, tier, count);
        if tests.is_empty() {
            continue;
        }
        let seconds = total(cases, tier, 0.0).seconds;
        let shown: f64 = tests.iter().map(|case| case.seconds).sum();
        let _ = writeln!(
            lines,
            "\n  slowest in tier {} ({} tests, {:.0}% of the tier's time)",
            tier.name(),
            tests.len(),
            if seconds > 0.0 {
                100.0 * shown / seconds
            } else {
                0.0
            },
        );
        for case in tests {
            let _ = writeln!(
                lines,
                "    {:>7.3} s  {}  {}",
                case.seconds, case.binary, case.name
            );
        }
    }
    lines
}

fn slowest_markdown(cases: &[TestCase], count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    let mut table = format!(
        "The {count} slowest tests of each tier:\n\n| Tier | Seconds | Test |\n|---|---:|---|\n"
    );
    for tier in Tier::ALL {
        for case in slowest(cases, tier, count) {
            let _ = writeln!(
                table,
                "| {} | {:.3} | `{}` `{}` |",
                tier.name(),
                case.seconds,
                case.binary,
                case.name
            );
        }
    }
    table
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_start_above_the_ceiling_warns_and_one_below_does_not() {
        assert!(start_warning(Os::Linux, 0.009, 1800).is_some());
        assert_eq!(start_warning(Os::Linux, 0.008, 1800), None);
        assert_eq!(start_warning(Os::Linux, 0.004, 1800), None);
        let windows = start_warning(Os::Windows, 0.016, 1800).unwrap();
        assert!(windows.contains("16 ms, is above 15 ms"), "{windows}");
        assert_eq!(start_warning(Os::Windows, 0.015, 1800), None);
        assert_eq!(start_warning(Os::Windows, 0.012, 1800), None);
        // Too few tests to know the start: nothing said.
        assert_eq!(start_warning(Os::Windows, 0.050, SMALL_SAMPLE - 1), None);
        assert!(start_warning(Os::Windows, 0.050, SMALL_SAMPLE).is_some());
    }

    #[test]
    fn an_over_budget_run_says_what_its_runner_was() {
        use super::{Os, runner_line};
        let slow = runner_line(Os::Windows, 0.012, 1900);
        assert!(
            slow.starts_with("runner: 12 ms runner, a slow runner: tiers likely about twice"),
            "{slow}"
        );
        let usual = runner_line(Os::Windows, 0.0098, 1900);
        assert!(
            usual.starts_with("runner: 10 ms runner, a usual runner (6 to 10 ms a test)"),
            "{usual}"
        );
        assert!(
            runner_line(Os::Linux, 0.005, 1900)
                .contains("a slow runner: tiers likely about 1.8 times"),
        );
        assert!(runner_line(Os::Linux, 0.003, 1900).contains("a usual runner"));
        // Too few tests to measure a runner by.
        assert!(runner_line(Os::Windows, 0.012, 40).contains("too few to tell"));
    }

    use super::{
        Os, SMALL_SAMPLE, Tier, Totals, Verdict, by_crate, crate_of, crates_markdown, crates_text,
        markdown, slowest_markdown, slowest_tests, start_cost, start_warning, summarise, total,
        verdict,
    };
    use crate::junit::tests::case;
    use crate::junit::{self, TestCase};

    /// A report kept in the repository: thirteen tests across the three
    /// tiers, two of them failures, with times chosen so that every line of
    /// the report below can be checked by hand. Its quickest unit or
    /// integration test took 0.2 s, which is therefore its start cost.
    const REPORT: &str = include_str!("../fixtures/tiers/junit.xml");

    /// Two real reports of one tree, from the Windows job on 2026-11-13:
    /// one from a fast runner and one from a slow one. Only each test's
    /// binary, tier and time are kept.
    const FAST_RUNNER: &str = include_str!("../fixtures/tiers/windows-fast-runner.xml");
    const SLOW_RUNNER: &str = include_str!("../fixtures/tiers/windows-slow-runner.xml");

    #[test]
    fn where_a_test_lives_and_its_name_decide_its_tier() {
        assert_eq!(Tier::of("cineris-base", "text::tests::wraps"), Tier::Unit);
        assert_eq!(
            Tier::of("cineris-cli::bin/cineris", "tests::help"),
            Tier::Unit
        );
        assert_eq!(
            Tier::of("cineris-agent::it", "loop_::retries"),
            Tier::Integration
        );
        assert_eq!(Tier::of("cineris-cli::system", "sys_version"), Tier::System);
        // A test that starts a process is a system test wherever it lives.
        assert_eq!(
            Tier::of("xtask::bin/xtask", "rules::tests::sys_repository"),
            Tier::System
        );
        assert_eq!(
            Tier::of("cineris-agent::it", "tools::sys_shell_is_cancelled"),
            Tier::System
        );
        // The prefix counts only at the start of a path segment.
        assert_eq!(
            Tier::of("cineris-base", "tests::parses_sys_calls"),
            Tier::Unit
        );
    }

    #[test]
    fn the_report_of_a_fixed_run_is_exactly_this() {
        let cases = junit::parse(REPORT);

        let (table, over) = summarise(&cases, Os::Linux);
        let report = format!("{table}{}", slowest_tests(&cases, 3));

        assert_eq!(over, []);
        assert_eq!(
            report,
            "  process start: 200 ms a test (the 1st percentile of the run's 9 unit and \
             integration tests' times), taken off every test before a tier is held to its budget. \
             It was measured on fewer than 500 tests: in a run of a few crates, or of tests a \
             filter chose, the quickest may do more than start, and the figures above the start \
             are then too low
  tier             tests   time    above start  budget  used
  1, unit              5    6.0 s        5.0 s    20 s   25%
  2, integration       4   10.0 s        9.2 s    40 s   23%
  3, system            4   15.0 s       14.2 s    60 s   24%

  slowest in tier 1, unit (3 tests, 92% of the tier's time)
      3.000 s  cineris-policy  rules::tests::a_deny_beats_an_allow_in_any_order
      1.500 s  cineris-base  framing::tests::every_split_into_three
      1.000 s  cineris-base  text::tests::wraps_at_the_width

  slowest in tier 2, integration (3 tests, 98% of the tier's time)
      6.250 s  cineris-llm::it  anthropic::replay::every_fixture_cut_in_two
      2.500 s  cineris-agent::it  denials::a_refused_call_stops_the_turn
      1.050 s  cineris-agent::it  rounds::two_reads_run_together

  slowest in tier 3, system (3 tests, 97% of the tier's time)
     10.000 s  cineris-policy::it  differential::sys_bash_agrees
      2.500 s  cineris-tools::it  shell::sys_the_exit_status_is_the_commands
      2.000 s  xtask::bin/xtask  rules::tests::sys_the_repository_follows_its_own_rules
"
        );
    }

    #[test]
    fn the_summary_page_has_the_same_numbers() {
        let cases = junit::parse(REPORT);

        assert_eq!(
            markdown(&cases, Os::Windows),
            "The start of a test's process: 200 ms a test (the 1st percentile of the run's 9 unit \
             and integration tests' times), taken off every test before a tier is held to its \
             budget. It was measured on fewer than 500 tests: in a run of a few crates, or of \
             tests a filter chose, the quickest may do more than start, and the figures above the \
             start are then too low.

| Tier | Tests | Time | Above start | Budget | Used |
|---|---:|---:|---:|---:|---:|
| 1, unit | 5 | 6.0 s | 5.0 s | 25 s | 20% |
| 2, integration | 4 | 10.0 s | 9.2 s | 40 s | 23% |
| 3, system | 4 | 15.0 s | 14.2 s | 90 s | 16% |
"
        );
        assert_eq!(
            slowest_markdown(&cases, 1),
            "The 1 slowest tests of each tier:

| Tier | Seconds | Test |
|---|---:|---|
| 1, unit | 3.000 | `cineris-policy` `rules::tests::a_deny_beats_an_allow_in_any_order` |
| 2, integration | 6.250 | `cineris-llm::it` `anthropic::replay::every_fixture_cut_in_two` |
| 3, system | 10.000 | `cineris-policy::it` `differential::sys_bash_agrees` |
"
        );
    }

    #[test]
    fn a_tier_is_split_by_crate_above_the_start() {
        assert_eq!(crate_of("cineris-agent::it"), "cineris-agent");
        assert_eq!(crate_of("cineris-cli::bin/cineris"), "cineris-cli");
        assert_eq!(crate_of("cineris-base"), "cineris-base");

        let cases = junit::parse(REPORT);
        // The start cost of the fixed report is 0.2 s.
        let crates = by_crate(&cases, 0.2);
        let rounded: Vec<(&str, [i64; 3])> = crates
            .iter()
            .map(|(name, seconds)| {
                (
                    name.as_str(),
                    seconds.map(|each| (each * 100.0).round() as i64),
                )
            })
            .collect();
        assert_eq!(
            rounded,
            [
                ("cineris-policy", [280, 0, 980]),
                ("cineris-llm", [0, 605, 0]),
                ("cineris-agent", [0, 315, 0]),
                ("cineris-tools", [0, 0, 260]),
                ("cineris-base", [220, 0, 0]),
                ("xtask", [0, 0, 180]),
            ]
        );
        // Every second above the start is in one crate's row, in its tier.
        for tier in Tier::ALL {
            let split: f64 = crates
                .iter()
                .map(|(_, seconds)| seconds[tier.index()])
                .sum();
            assert!(
                (split - total(&cases, tier, 0.2).above_start).abs() < 1e-9,
                "{tier:?}"
            );
        }
    }

    #[test]
    fn the_crates_past_the_first_few_are_one_line() {
        let cases = junit::parse(REPORT);
        let text = crates_text(&cases, 2, super::start_of(&cases).0);
        assert!(text.contains("cineris-policy"), "{text}");
        assert!(text.contains("cineris-llm"), "{text}");
        assert!(!text.contains("cineris-agent"), "{text}");
        // agent 3.15, tools 2.6, base 2.2, xtask 1.8.
        assert!(text.contains("4 more crates"), "{text}");
        let rest = text
            .lines()
            .find(|line| line.contains("more crates"))
            .unwrap();
        assert!(
            rest.trim_start().starts_with("4 more crates") && rest.ends_with("4.4"),
            "{text}"
        );
        let markdown = crates_markdown(&cases, super::start_of(&cases).0);
        assert!(
            markdown.contains("| `xtask` | 0.0 s | 0.0 s | 1.8 s |"),
            "{markdown}"
        );
    }

    #[test]
    fn tests_that_took_the_same_time_are_listed_in_name_order() {
        let cases = [
            case("cineris-base", "tests::b", 0.5),
            case("cineris-base", "tests::a", 0.5),
            case("cineris-agent", "tests::z", 0.5),
        ];

        let lines = slowest_tests(&cases, 2);

        let z = lines.find("cineris-agent  tests::z").unwrap();
        let a = lines.find("cineris-base  tests::a").unwrap();
        assert!(z < a, "{lines}");
        assert!(!lines.contains("tests::b"), "{lines}");
    }

    #[test]
    fn asking_for_no_slowest_tests_prints_none() {
        let cases = junit::parse(REPORT);

        assert_eq!(slowest_tests(&cases, 0), "");
        assert_eq!(slowest_markdown(&cases, 0), "");
    }

    /// Five unit tests of `seconds` each, and one quick integration test.
    fn unit_tests_taking(seconds: f64) -> Vec<TestCase> {
        let mut cases: Vec<TestCase> = (0..5)
            .map(|i| case("cineris-base", &format!("tests::slow_{i}"), seconds))
            .collect();
        cases.push(case("cineris-agent::it", "loop_::quick", 0.1));
        cases
    }

    #[test]
    fn a_tier_over_its_budget_is_a_warning() {
        // 22.0 s above the start cost of 0.1 s, against a budget of 20 s.
        let (table, over) = summarise(&unit_tests_taking(4.5), Os::Linux);

        assert_eq!(over, [(Tier::Unit, Verdict::OverBudget)]);
        assert!(table.contains("over budget (warning)"), "{table}");
    }

    #[test]
    fn a_tier_far_over_its_budget_fails() {
        // 32.0 s above the start cost: more than one and a half times the
        // budget of 20 s.
        let (table, over) = summarise(&unit_tests_taking(6.5), Os::Linux);

        assert_eq!(over, [(Tier::Unit, Verdict::Failing)]);
        assert!(table.contains("FAR OVER BUDGET"), "{table}");
    }

    #[test]
    fn the_system_tier_has_more_room_on_windows() {
        // 75 s of system tests: over 60 s, inside 90 s.
        let cases: Vec<TestCase> = (0..5)
            .map(|i| case("cineris-cli::system", &format!("sys_slow_{i}"), 15.0))
            .collect();

        assert_eq!(
            summarise(&cases, Os::Linux).1,
            [(Tier::System, Verdict::OverBudget)]
        );
        assert_eq!(summarise(&cases, Os::Windows).1, []);
    }

    #[test]
    fn the_start_cost_is_what_one_in_a_hundred_unit_and_integration_tests_beat() {
        // Two hundred tests from 10 ms to 209 ms: two of them are quicker
        // than 12 ms.
        let mut cases: Vec<TestCase> = (0..200)
            .map(|i| {
                case(
                    "cineris-base",
                    &format!("tests::t{i}"),
                    0.010 + 0.001 * f64::from(i),
                )
            })
            .collect();
        assert!((start_cost(&cases) - 0.012).abs() < 1e-9);
        // A system test is not looked at, however quick: it starts other
        // processes, and is no measure of starting one.
        cases.push(case("cineris-tools::it", "shell::sys_quick", 0.001));
        assert!((start_cost(&cases) - 0.012).abs() < 1e-9);
        // With fewer than a hundred, the quickest.
        let few = [
            case("cineris-base", "tests::a", 0.030),
            case("cineris-agent::it", "rounds::b", 0.020),
        ];
        assert!((start_cost(&few) - 0.020).abs() < 1e-9);
        // With none to measure, nothing is taken off.
        let system_only = [case("cineris-cli::system", "sys_version", 0.5)];
        assert_eq!(start_cost(&system_only), 0.0);
        assert_eq!(start_cost(&[]), 0.0);
    }

    #[test]
    fn a_test_quicker_than_the_start_cost_counts_as_nothing() {
        let cases = [
            case("cineris-base", "tests::a", 0.010),
            case("cineris-base", "tests::b", 0.008),
            case("cineris-base", "tests::c", 0.510),
        ];
        let totals = total(&cases, Tier::Unit, 0.010);
        assert_eq!(totals.count, 3);
        assert!((totals.seconds - 0.528).abs() < 1e-9);
        assert!((totals.above_start - 0.5).abs() < 1e-9, "{totals:?}");
    }

    #[test]
    fn a_slow_runner_alone_does_not_put_a_tier_over_its_budget() {
        // Two thousand unit tests that do nothing, each 12 ms of starting
        // a process on a slow runner: 24 s, near Windows' budget of 25 s as
        // a raw total, and nothing above the start.
        let cases: Vec<TestCase> = (0..2000)
            .map(|i| case("cineris-base", &format!("tests::t{i}"), 0.012))
            .collect();
        let (table, over) = summarise(&cases, Os::Windows);
        assert_eq!(over, [], "{table}");
        assert!(table.contains("24.0 s        0.0 s"), "{table}");
        // Two thousand tests are enough to measure the start on.
        assert!(!table.contains("fewer than"), "{table}");
        // One test that takes 26 s puts it over, as it should.
        let mut slow = cases;
        slow.push(case("cineris-base", "tests::slow", 26.0));
        assert_eq!(
            summarise(&slow, Os::Windows).1,
            [(Tier::Unit, Verdict::OverBudget)]
        );
    }

    /// Each tier's raw total and its total above the start cost.
    fn tiers_of(report: &str) -> (f64, Vec<Totals>) {
        let cases = junit::parse(report);
        let start = start_cost(&cases);
        let totals = Tier::ALL
            .iter()
            .map(|tier| total(&cases, *tier, start))
            .collect();
        (start, totals)
    }

    /// How far apart two figures are, as a share of the smaller.
    fn apart(a: f64, b: f64) -> f64 {
        (a - b).abs() / a.min(b)
    }

    #[test]
    fn a_fast_and_a_slow_windows_runner_read_alike_above_the_start() {
        let (fast_start, fast) = tiers_of(FAST_RUNNER);
        let (slow_start, slow) = tiers_of(SLOW_RUNNER);
        // The two runners: 6 ms and 11 ms to start a test.
        assert!((fast_start - 0.006).abs() < 1e-9, "{fast_start}");
        assert!((slow_start - 0.011).abs() < 1e-9, "{slow_start}");
        let (unit, integration, system) = (0, 1, 2);
        assert_eq!(fast[unit].count, slow[unit].count);

        // The unit tier: 17.0 s and 22.7 s raw, a third apart, which is
        // the runner and not the tests. Above the start, 11.0 s and 11.6 s:
        // within the run-to-run noise of one runner and one setting (0.5 s
        // in the one job that measured it).
        assert!(apart(fast[unit].seconds, slow[unit].seconds) > 0.30);
        assert!(
            apart(fast[unit].above_start, slow[unit].above_start) < 0.10,
            "{fast:?} {slow:?}"
        );

        // The integration tier: 28.2 s and 40.6 s raw, 23.1 s and 31.3 s
        // above the start. The start explains part of the gap, and the
        // rest is the slow runner being slower at the tests' work, which
        // the budget is right to see: its tests do much more than start.
        assert!(
            apart(fast[integration].above_start, slow[integration].above_start)
                < apart(fast[integration].seconds, slow[integration].seconds)
        );
        assert!(slow[integration].above_start < 40.0);

        // The system tier is unchanged in substance: its tests take
        // seconds, of which the start is a sliver.
        assert!(slow[system].seconds - slow[system].above_start < 1.0);
    }

    #[test]
    fn the_line_between_a_warning_and_a_failure_is_one_and_a_half_budgets() {
        assert_eq!(verdict(20.0, 20.0), None);
        assert_eq!(verdict(20.1, 20.0), Some(Verdict::OverBudget));
        assert_eq!(verdict(30.0, 20.0), Some(Verdict::OverBudget));
        assert_eq!(verdict(30.1, 20.0), Some(Verdict::Failing));
    }

    use super::{check_ci_report_in, summarise_marked};
    use crate::{stall, workspace};

    /// A report of `cases`, as much of one as the tier check reads.
    fn report_of(cases: &[TestCase]) -> String {
        let mut text = String::from("<testsuites name=\"nextest-run\">\n");
        for case in cases {
            text.push_str(&format!(
                "<testsuite name=\"{binary}\"><testcase name=\"{name}\" classname=\"{binary}\" \
                 time=\"{}\"/></testsuite>\n",
                case.seconds,
                binary = case.binary,
                name = case.name
            ));
        }
        text.push_str("</testsuites>\n");
        text
    }

    /// The tree of a CI run whose unit tier is far over its budget on every
    /// system (39.5 s above the start: past one and a half times 20 s on
    /// Linux and 25 s on Windows, judged on the system the test runs on),
    /// and whose test runs left `record`
    /// (a line of `stall::Record`'s form) when it is given.
    fn tree_with(label: &str, record: Option<&str>) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("xtask-tiers-{label}-{}", std::process::id()));
        let report = workspace::nextest_report(&root, "ci");
        std::fs::create_dir_all(report.parent().unwrap()).unwrap();
        std::fs::write(&report, report_of(&unit_tests_taking(8.0))).unwrap();
        if let Some(record) = record {
            std::fs::write(stall::record_path(&root), record).unwrap();
        }
        root
    }

    #[test]
    fn a_stalled_run_is_not_judged_at_a_desk_and_is_on_ci_whatever_the_pressure() {
        // 90 s of every task waiting, in test runs of 100 s.
        let stalled = "elapsed_ms=100000 full_us=90000000\n";
        // Not judged at a desk: far over its budget, and neither fails.
        let root = tree_with("desk-stalled", Some(stalled));
        let desk = check_ci_report_in(&root, false, false);
        // Judged on CI whatever the pressure: the same run fails.
        let actions = check_ci_report_in(&root, true, false);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(desk, Ok(()), "a stalled run was judged at a desk");
        assert!(
            actions.is_err_and(|error| error.contains("times over its time budget")),
            "CI judges a run whatever the pressure"
        );
    }

    #[test]
    fn a_stall_of_the_system_tests_run_leaves_the_unit_tier_judged() {
        // The unit tier is far over its budget in a quiet run of tiers 1 and
        // 2; the system tests' run stalled. Together 35 s of 300 (11.7%).
        let root = tree_with(
            "other-run-stalled",
            Some(
                "elapsed_ms=200000 full_us=0 tiers=1,2\nelapsed_ms=100000 full_us=35000000 \
                 tiers=3\n",
            ),
        );
        let desk = check_ci_report_in(&root, false, false);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            desk.is_err_and(|error| error.contains("times over its time budget")),
            "a stall of the system tests excused the unit tier"
        );
    }

    #[test]
    fn a_run_whose_disk_did_not_stall_is_judged_at_a_desk_as_it_always_was() {
        // 5 s of 100: under the threshold. And a run with no record at all,
        // where the pressure could not be read.
        for (label, record) in [
            ("quiet", Some("elapsed_ms=100000 full_us=5000000\n")),
            ("unread", None),
            ("short", Some("elapsed_ms=100000 full_us=9900000\n")),
        ] {
            let root = tree_with(label, record);
            let checked = check_ci_report_in(&root, false, false);
            std::fs::remove_dir_all(&root).unwrap();
            assert!(
                checked.is_err_and(|error| error.contains("times over its time budget")),
                "{label}"
            );
        }
    }

    #[test]
    fn a_run_that_is_not_judged_has_no_mark_on_any_tier() {
        let cases = unit_tests_taking(6.5);
        let (marked, over) = summarise_marked(&cases, Os::Linux, |_| true);
        assert!(marked.contains("FAR OVER BUDGET"));
        assert_eq!(over.len(), 1);
        let (plain, over) = summarise_marked(&cases, Os::Linux, |_| false);
        assert!(!plain.contains("OVER BUDGET") && !plain.contains("over budget"));
        assert!(over.is_empty());
        // The numbers are the same.
        assert_eq!(
            marked.replace("  FAR OVER BUDGET", ""),
            plain,
            "only the marks differ"
        );
    }

    use super::{Part, Scope, check_ci_part_in, markdown_in, start_probe_path, summarise_in};

    /// `count` system tests of `seconds` each.
    fn system_tests(count: usize, seconds: f64) -> Vec<TestCase> {
        (0..count)
            .map(|i| case("cineris-cli::system", &format!("t::sys_{i}"), seconds))
            .collect()
    }

    /// `count` unit tests of `seconds` each: a start probe's.
    fn probe_tests(count: usize, seconds: f64) -> Vec<TestCase> {
        (0..count)
            .map(|i| case("cineris-base", &format!("t::quick_{i}"), seconds))
            .collect()
    }

    /// The tree of a part's run: its report and, when given, its probe's
    /// report (as text: a probe's failure needs a `<failure>`).
    fn part_tree(label: &str, cases: &[TestCase], probe: Option<&str>) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("xtask-tiers-part-{label}-{}", std::process::id()));
        let report = workspace::nextest_report(&root, "ci");
        std::fs::create_dir_all(report.parent().unwrap()).unwrap();
        std::fs::write(&report, report_of(cases)).unwrap();
        if let Some(probe) = probe {
            std::fs::write(start_probe_path(&root), probe).unwrap();
        }
        root
    }

    #[test]
    fn the_system_part_is_held_to_its_budget_above_the_start_its_probe_measured() {
        // 1,000 system tests of 0.1 s: 100 s raw, 50 s above a start of
        // 50 ms. Over tier 3's budget raw on every OS, and under it on
        // Windows above the probe's start.
        let cases = system_tests(1000, 0.1);
        let root = part_tree(
            "probe-start",
            &cases,
            Some(&report_of(&probe_tests(600, 0.05))),
        );
        let scope = Scope::of_part(&cases, Part::System, &start_probe_path(&root));
        std::fs::remove_dir_all(&root).unwrap();
        let scope = scope.unwrap();
        assert!((scope.start - 0.05).abs() < 1e-9, "{}", scope.start);
        assert_eq!((scope.sample, scope.probe), (600, true));
        let (table, over) = summarise_in(&cases, Os::Windows, |_| true, &scope);
        assert!(table.contains("50.0 s"), "{table}");
        assert!(over.is_empty(), "{table}");
        // Only tier 3, and a start said to be the probe's.
        assert!(
            !table.contains("1, unit") && !table.contains("2, integration"),
            "{table}"
        );
        assert!(table.contains("of the start probe"), "{table}");
        let markdown = markdown_in(&cases, Os::Windows, &scope);
        assert!(markdown.contains("| 3, system | 1000 |"), "{markdown}");
        assert!(!markdown.contains("| 1, unit"), "{markdown}");
    }

    #[test]
    fn a_probe_that_failed_ran_nothing_or_is_missing_fails_the_system_part() {
        let cases = system_tests(10, 1.0);
        let failed = "<testsuites name=\"nextest-run\">\n<testsuite name=\"cineris-base\">\
                      <testcase name=\"t::quick\" classname=\"cineris-base\" time=\"0.01\">\
                      <failure message=\"no\">no</failure></testcase></testsuite>\n</testsuites>\n";
        let empty = "<testsuites name=\"nextest-run\">\n</testsuites>\n";
        let integration = report_of(&[case("cineris-agent::it", "a::b", 0.01)]);
        for (label, probe, said) in [
            ("failed", Some(failed), "failed"),
            ("empty", Some(empty), "lists no tests"),
            ("integration", Some(integration.as_str()), "not a unit test"),
            ("missing", None, "the start probe"),
        ] {
            let root = part_tree(label, &cases, probe);
            let checked = check_ci_part_in(&root, true, false, Part::System);
            std::fs::remove_dir_all(&root).unwrap();
            assert!(
                checked.as_ref().is_err_and(|error| error.contains(said)),
                "{label}: {checked:?}"
            );
        }
    }

    #[test]
    fn a_parts_report_must_hold_each_of_its_tiers_and_no_other() {
        let probe = report_of(&probe_tests(600, 0.01));
        let tiers_and_system = {
            let mut cases = unit_tests_taking(0.1);
            cases.push(case("cineris-cli::system", "t::sys_a", 1.0));
            cases
        };
        let no_integration = probe_tests(10, 0.01);
        for (label, cases, part, said) in [
            (
                "system-in-tiers",
                &tiers_and_system,
                Part::Tiers,
                "not one of its tiers",
            ),
            (
                "unit-in-system",
                &tiers_and_system,
                Part::System,
                "not one of its tiers",
            ),
            (
                "no-integration",
                &no_integration,
                Part::Tiers,
                "no test of tier 2",
            ),
        ] {
            let root = part_tree(label, cases, Some(&probe));
            let checked = check_ci_part_in(&root, true, false, part);
            std::fs::remove_dir_all(&root).unwrap();
            assert!(
                checked.as_ref().is_err_and(|error| error.contains(said)),
                "{label}: {checked:?}"
            );
        }
    }

    #[test]
    fn a_part_far_over_its_budget_fails_and_one_within_it_passes() {
        // 100 system tests of 2 s: about 199 s above the probe's start,
        // past one and a half times tier 3's budget on every OS.
        let probe = report_of(&probe_tests(600, 0.01));
        let root = part_tree("system-over", &system_tests(100, 2.0), Some(&probe));
        let over = check_ci_part_in(&root, true, false, Part::System);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            over.as_ref()
                .is_err_and(|error| error.contains("tier 3, system")),
            "{over:?}"
        );
        let root = part_tree("system-within", &system_tests(10, 1.0), Some(&probe));
        let within = check_ci_part_in(&root, true, false, Part::System);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(within, Ok(()));
        // Tiers 1 and 2 need no probe: the unit tier is far over its budget.
        let root = part_tree("tiers-over", &unit_tests_taking(8.0), None);
        let tiers = check_ci_part_in(&root, true, false, Part::Tiers);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            tiers
                .as_ref()
                .is_err_and(|error| error.contains("tier 1, unit")),
            "{tiers:?}"
        );
    }

    #[test]
    fn a_part_is_named_as_ci_test_takes_it() {
        assert_eq!(Part::parse("tiers"), Some(Part::Tiers));
        assert_eq!(Part::parse("system"), Some(Part::System));
        assert_eq!(Part::parse("all"), None);
        assert_eq!(Part::Tiers.tiers(), [Tier::Unit, Tier::Integration]);
        assert_eq!(Part::System.tiers(), [Tier::System]);
    }

    #[test]
    fn a_probe_of_too_few_tests_fails_the_system_part() {
        let probe = report_of(&probe_tests(SMALL_SAMPLE - 1, 0.01));
        let root = part_tree("few", &system_tests(10, 1.0), Some(&probe));
        let checked = check_ci_part_in(&root, true, false, Part::System);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            checked
                .as_ref()
                .is_err_and(|error| error.contains("fewer than the 500")),
            "{checked:?}"
        );
    }

    /// The report a split run's parts make together: tier 3 above the
    /// start its own cases carry, the others above the report's, and the
    /// start line says both.
    #[test]
    fn a_merged_split_reports_system_tests_are_held_above_their_own_start() {
        let mut cases = probe_tests(600, 0.01);
        cases.push(case("cineris-agent::it", "a::b", 0.5));
        let mut system = system_tests(100, 0.5);
        for case in &mut system {
            case.start = Some(0.1);
        }
        cases.extend(system);
        let scope = Scope::whole(&cases);
        let (table, _) = summarise_in(&cases, Os::Windows, |_| true, &scope);
        // 100 tests of 0.5 s above 0.1 s each: 40 s, not 49.
        assert!(table.contains("    40.0 s"), "{table}");
        assert!(
            table.contains("above their own runner's start, 100 ms"),
            "{table}"
        );
        assert!(table.contains("2, integration"), "{table}");
    }

    /// `ci --tiers --from` a system tests' report with its probe's beside
    /// it reads it as that part, above the probe's start.
    #[test]
    fn a_report_with_a_probe_beside_it_is_read_as_the_system_part() {
        let probe = report_of(&probe_tests(600, 0.4));
        let root = part_tree("from", &system_tests(100, 1.5), Some(&probe));
        // 100 tests of 1.5 s: 150 s raw, past one and a half times
        // Windows' 90 s (a failure), and 110 s above the probe's 0.4 s,
        // over the budget and under the failure (a warning).
        let read = super::check_report(&workspace::nextest_report(&root, "ci"), Os::Windows, 1);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(read, Ok(()));
    }

    /// A whole run's report is read whole even with a start probe's left
    /// beside it: only a report of system tests alone is the system part.
    #[test]
    fn a_whole_report_beside_a_stale_probe_is_read_whole() {
        let probe = report_of(&probe_tests(600, 0.01));
        let root = part_tree("stale-probe", &unit_tests_taking(8.0), Some(&probe));
        let read = super::check_report(&workspace::nextest_report(&root, "ci"), Os::Linux, 1);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            read.as_ref()
                .is_err_and(|error| error.contains("times over its time budget")),
            "{read:?}"
        );
    }
}
