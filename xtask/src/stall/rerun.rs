//! Running a cluster of timeouts again, once, when the disk stalled.
//!
//! In wave 11 six sessions of seven lost time to a stalled disk at the
//! owner's desk: a dozen tests of crates nobody had changed ran out of
//! time together, and a person had to read the output, see that it was the
//! machine, and run the tests again by hand. `cargo xtask ci` now does the
//! one thing a person would do, and says so, under rules that never let a
//! failure through.
//!
//! **It runs tests again only when all three hold:**
//!
//! 1. the run failed;
//! 2. its failures hold a cluster that [`super::find`] finds: three or more
//!    timeouts, each stopped late, ending within 250 ms of each other. A lone
//!    timeout, or timeouts at their own limits, or apart, is never one;
//! 3. the pressure says the disk stalled during the run: every task waited
//!    on I/O for [`STALLED_SHARE`] of the run's time or more
//!    ([`super::Observed::share`]). Where the pressure cannot be read
//!    (Windows, a kernel without it, a hosted runner whose kernel has none)
//!    a cluster is a failure and nothing is run again.
//!
//! **What it runs again** is the cluster's tests and no others, once, by
//! each test's binary and name, exactly ([`filter`]). The other timeouts of
//! the run, and every failure of any other kind, are not run again and
//! still fail the step: a stall that made one cluster late says nothing of
//! a test that panicked.
//!
//! **What it leaves in the report** is the run's own report with each of
//! those tests' cases replaced by the case the re-run wrote
//! ([`crate::junit::replace_cases`]), so that the tier check and `ci-times`
//! read one report, with the times the tests really took, and the counts of
//! failures made again. The step passes only when, after that, the report
//! holds no failed case at all, the re-run's command succeeded, and every
//! test that was run again is in the re-run's report and passed. A test
//! that times out again, fails, or is not in the re-run's report fails the
//! step, and so does everything else that failed.
//!
//! The decisions are functions of the failures, the pressure and the
//! reports: the tests of them give a pressure and reports, and run nothing.

use std::path::Path;
use std::time::Duration;

use super::{Observed, Record, STALLED_SHARE, Stall, explain, find_cases};
use crate::Result;
use crate::junit::{self, Failure};

/// A test, by its binary and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Test {
    pub(crate) binary: String,
    pub(crate) name: String,
}

impl Test {
    fn id(&self) -> String {
        format!("{} {}", self.binary, self.name)
    }
}

/// Tests to run again, and why.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Plan {
    /// The cluster's tests, in the order they were stopped.
    pub(crate) tests: Vec<Test>,
    /// The cluster, for the message.
    pub(crate) stall: Stall,
    /// The pressure over the run.
    pub(crate) observed: Observed,
}

/// Why a cluster of timeouts is not run again.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Declined {
    /// The pressure could not be read.
    Unreadable(String),
    /// It could, and the disk had not stalled: the share of the run, in
    /// percent, in which every task waited on I/O.
    Quiet(f64),
    /// A test's name has something in it that a filter cannot be trusted
    /// to name exactly.
    Unnamable(String),
}

impl Declined {
    fn message(&self) -> String {
        match self {
            Self::Unreadable(why) => format!(
                "the I/O pressure could not be read ({why}), so there is no evidence that the \
                 disk stalled"
            ),
            Self::Quiet(share) => format!(
                "the disk did not stall during the run: every task waited on I/O for {share:.1}% \
                 of its time, and a run is taken to have stalled at {STALLED_SHARE:.0}%"
            ),
            Self::Unnamable(test) => format!(
                "`{test}` has characters in its name that a filter cannot be trusted to match \
                 exactly"
            ),
        }
    }
}

/// What to do about a failed run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Decision {
    /// No cluster of timeouts: a failure, or a lone timeout, is a failure.
    Nothing,
    /// A cluster, and the disk stalled: run it again.
    Rerun(Plan),
    /// A cluster that is not run again.
    Declined { stall: Stall, why: Declined },
}

/// Characters a test's binary or name may have for a filter to name it
/// exactly: a Rust path, and the `/` and `.` of nextest's binary ids.
fn nameable(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '/' | '.' | '-'))
}

/// What to do about a run that failed with `failed` (its cases with a
/// `<failure>` or an `<error>`: [`junit::failed`]), given what the
/// pressure said of it.
pub(crate) fn decide(failed: &[Failure], observed: &Result<Observed>) -> Decision {
    let Some((stall, places)) = find_cases(failed) else {
        return Decision::Nothing;
    };
    let declined = |why| Decision::Declined {
        stall: stall.clone(),
        why,
    };
    let observed = match observed {
        Ok(observed) => observed,
        Err(why) => return declined(Declined::Unreadable(why.clone())),
    };
    if observed.share() < STALLED_SHARE {
        return declined(Declined::Quiet(observed.share()));
    }
    let tests: Vec<Test> = places
        .iter()
        .map(|place| Test {
            binary: failed[*place].binary.clone(),
            name: failed[*place].name.clone(),
        })
        .collect();
    if let Some(test) = tests
        .iter()
        .find(|test| !nameable(&test.binary) || !nameable(&test.name))
    {
        return declined(Declined::Unnamable(test.id()));
    }
    Decision::Rerun(Plan {
        tests,
        stall,
        observed: *observed,
    })
}

/// The nextest filterset that chooses exactly `tests`, each by its binary
/// and its name.
pub(crate) fn filter(tests: &[Test]) -> String {
    tests
        .iter()
        .map(|test| format!("(binary_id(={}) and test(={}))", test.binary, test.name))
        .collect::<Vec<_>>()
        .join(" or ")
}

/// The arguments of a run, with its filtersets taken out and `filterset`
/// given in their place, before any argument for the test binaries.
pub(crate) fn with_filter(run: &[String], filterset: &str) -> Vec<String> {
    let (_, mut rest) = crate::checks::filtersets(run);
    let at = rest
        .iter()
        .position(|argument| argument == "--")
        .unwrap_or(rest.len());
    rest.splice(at..at, ["-E".to_owned(), filterset.to_owned()]);
    rest
}

/// How a test that was run again did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Passed,
    /// It ran out of time again.
    TimedOut,
    /// It failed in another way.
    Failed,
    /// Its case is not in the re-run's report.
    Missing,
}

impl Verdict {
    fn says(self) -> &'static str {
        match self {
            Self::Passed => "passed on the re-run",
            Self::TimedOut => "TIMED OUT AGAIN on the re-run",
            Self::Failed => "FAILED on the re-run",
            Self::Missing => "is NOT IN THE REPORT of the re-run",
        }
    }
}

/// How each of `tests` did, by the re-run's report.
pub(crate) fn verdicts(tests: &[Test], rerun_report: &str) -> Vec<Verdict> {
    let cases = junit::parse(rerun_report);
    let failures = junit::failed(rerun_report);
    tests
        .iter()
        .map(|test| {
            let found = cases
                .iter()
                .find(|case| case.binary == test.binary && case.name == test.name);
            match found {
                None => Verdict::Missing,
                Some(case) if !case.failed => Verdict::Passed,
                Some(_) => {
                    let timed_out = failures.iter().any(|failure| {
                        failure.binary == test.binary
                            && failure.name == test.name
                            && failure.kind.contains("timeout")
                    });
                    if timed_out {
                        Verdict::TimedOut
                    } else {
                        Verdict::Failed
                    }
                }
            }
        })
        .collect()
}

/// A run of `cargo nextest`, and what the pressure said of it.
pub(crate) struct Attempt {
    pub(crate) result: Result<()>,
    pub(crate) observed: Result<Observed>,
}

/// How a run of tests that may be run again ended.
#[derive(Debug)]
pub(crate) struct Outcome {
    /// Whether the step passes.
    pub(crate) result: Result<()>,
    /// What the pressure said of each run, for the tier check.
    pub(crate) records: Vec<Record>,
    /// For the job's summary page, when tests were run again.
    pub(crate) summary: Option<String>,
    /// The tests that were run again: a known flake among them is not run
    /// a third time (`crate::flaky`).
    pub(crate) ran_again: Vec<Test>,
}

/// Runs `args` with `exec`, and when it fails and [`decide`] says so, runs
/// its cluster of timeouts again, once. `report` is where the runs write
/// their JUnit report; `say` is where the commands' own messages go.
pub(crate) fn run(
    args: &[String],
    report: &Path,
    exec: &mut dyn FnMut(&[String]) -> Attempt,
    say: &mut dyn FnMut(&str),
) -> Outcome {
    let first = exec(args);
    let records: Vec<Record> = first.observed.iter().map(Observed::record).collect();
    let Err(first_error) = first.result else {
        return Outcome {
            result: Ok(()),
            records,
            summary: None,
            ran_again: Vec::new(),
        };
    };
    let failed_as_it_was = |records, error: String| Outcome {
        result: Err(error),
        records,
        summary: None,
        ran_again: Vec::new(),
    };
    let Ok(first_report) = std::fs::read_to_string(report) else {
        return failed_as_it_was(records, first_error);
    };
    let decision = decide(&junit::failed(&first_report), &first.observed);
    let (stall, why) = match decision {
        Decision::Nothing => return failed_as_it_was(records, first_error),
        Decision::Declined { stall, why } => (stall, why),
        Decision::Rerun(plan) => {
            return rerun(
                args,
                report,
                (first_error, &first_report, records),
                &plan,
                exec,
                say,
            );
        }
    };
    say(&say_stall(&stall, &first.observed));
    say(&format!("Not running them again: {}.", why.message()));
    failed_as_it_was(records, first_error)
}

/// The message of the stall, with what is known of the pressure.
fn say_stall(stall: &Stall, observed: &Result<Observed>) -> String {
    match observed {
        Ok(observed) => explain(
            stall,
            Some(&observed.before),
            &Ok(observed.after),
            observed.elapsed,
        ),
        Err(why) => explain(stall, None, &Err(why.clone()), Duration::ZERO),
    }
}

/// The second half of [`run`]: the plan's tests run again, and the reports
/// put together.
fn rerun(
    args: &[String],
    report: &Path,
    (first_error, first_report, mut records): (String, &str, Vec<Record>),
    plan: &Plan,
    exec: &mut dyn FnMut(&[String]) -> Attempt,
    say: &mut dyn FnMut(&str),
) -> Outcome {
    say(&say_stall(&plan.stall, &Ok(plan.observed)));
    say(&format!(
        "Running the {} tests that timed out together again, once: the disk stalled during the \
         run (every task waited on I/O for {:.1} s of its {:.0} s, {:.1}%; a run is taken to have \
         stalled at {STALLED_SHARE:.0}%). A test that times out again, and every other failure, \
         still fails this step.",
        plan.tests.len(),
        plan.observed.stalled_seconds(),
        plan.observed.elapsed.as_secs_f64(),
        plan.observed.share(),
    ));
    let second = exec(&with_filter(args, &filter(&plan.tests)));
    records.extend(second.observed.iter().map(Observed::record));
    let put_back = |why: String| {
        // The first report is the one the tier check and `ci-times` read.
        let _ = std::fs::write(report, first_report);
        Outcome {
            result: Err(format!("{first_error}; {why}")),
            records: Vec::new(),
            summary: None,
            ran_again: plan.tests.clone(),
        }
    };
    let Ok(second_report) = std::fs::read_to_string(report) else {
        say("The re-run left no report.");
        return put_back("the re-run left no report".to_owned());
    };
    let pairs: Vec<(String, String)> = plan
        .tests
        .iter()
        .map(|test| (test.binary.clone(), test.name.clone()))
        .collect();
    let combined = junit::replace_cases(first_report, &second_report, &pairs);
    if let Err(error) = std::fs::write(report, &combined) {
        let why = format!("could not write {}: {error}", report.display());
        say(&why);
        return put_back(why);
    }
    let verdicts = verdicts(&plan.tests, &second_report);
    for (test, verdict) in plan.tests.iter().zip(&verdicts) {
        say(&format!("  {} {}", test.id(), verdict.says()));
    }
    let others = junit::failed(&combined);
    let mut problems = Vec::new();
    let not_passed = verdicts
        .iter()
        .filter(|verdict| **verdict != Verdict::Passed)
        .count();
    if not_passed > 0 {
        problems.push(format!(
            "{not_passed} of the {} tests did not pass on the re-run",
            plan.tests.len()
        ));
    }
    let other_failures = others.len().saturating_sub(not_passed);
    if other_failures > 0 {
        problems.push(format!(
            "{other_failures} other test(s) failed, which are not run again"
        ));
    }
    // The command failed with nothing in the report to say why: a test that
    // failed is already a problem above.
    if let Err(error) = &second.result
        && not_passed == 0
        && other_failures == 0
    {
        problems.push(format!("the re-run's own command failed: {error}"));
    }
    let summary = Some(summary(plan, &verdicts, &problems));
    if problems.is_empty() {
        say(&format!(
            "All {} tests passed on the re-run, and nothing else failed: the step passes. Their \
             times in the report are the re-run's.",
            plan.tests.len()
        ));
        return Outcome {
            result: Ok(()),
            records,
            summary,
            ran_again: plan.tests.clone(),
        };
    }
    say(&format!("The step fails: {}.", problems.join("; ")));
    Outcome {
        result: Err(format!("{first_error}; {}", problems.join("; "))),
        records,
        summary,
        ran_again: plan.tests.clone(),
    }
}

/// What the job's summary page says of a re-run.
fn summary(plan: &Plan, verdicts: &[Verdict], problems: &[String]) -> String {
    let mut text = format!(
        "### Tests run again after a stall\n\n{} tests timed out together, and the disk had \
         stalled for {:.1}% of the run ({:.1} s of {:.0} s), so they were run again once.\n\n",
        plan.tests.len(),
        plan.observed.share(),
        plan.observed.stalled_seconds(),
        plan.observed.elapsed.as_secs_f64(),
    );
    for (test, verdict) in plan.tests.iter().zip(verdicts) {
        text.push_str(&format!("- `{}` {}\n", test.id(), verdict.says()));
    }
    if problems.is_empty() {
        text.push_str("\nNothing else failed.\n");
    } else {
        text.push_str(&format!("\nThe step fails: {}.\n", problems.join("; ")));
    }
    text
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{
        Attempt, Decision, Declined, Test, Verdict, decide, filter, run, verdicts, with_filter,
    };
    use crate::junit::{self, Failure};
    use crate::stall::{Observed, Pressure};

    fn pressure(full_total: u64) -> Pressure {
        Pressure {
            some: (0.0, 0.0),
            full: (0.0, 0.0),
            some_total: full_total,
            full_total,
        }
    }

    /// What the pressure said of a run of `elapsed` seconds in which every
    /// task waited on I/O for `stalled` seconds.
    fn observed(stalled: f64, elapsed: u64) -> Result<Observed, String> {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a few seconds, in microseconds"
        )]
        let micros = (stalled * 1e6) as u64;
        Ok(Observed {
            before: pressure(5_000_000),
            after: pressure(5_000_000 + micros),
            elapsed: Duration::from_secs(elapsed),
        })
    }

    fn timeout(binary: &str, name: &str, started: &str, seconds: f64) -> Failure {
        Failure {
            binary: binary.to_owned(),
            name: name.to_owned(),
            repeat: None,
            text: String::new(),
            kind: "test timeout".to_owned(),
            stderr: String::new(),
            started: started.to_owned(),
            seconds,
        }
    }

    /// Three timeouts stopped late within 200 ms of each other, in two
    /// binaries.
    fn cluster() -> Vec<Failure> {
        vec![
            timeout("cx::it", "a", "2026-11-15T10:00:00.000+00:00", 14.0),
            timeout("cx", "b::c", "2026-11-15T10:00:02.000+00:00", 12.1),
            timeout("cx::it", "d", "2026-11-15T10:00:03.900+00:00", 10.3),
        ]
    }

    fn tests_of(decision: &Decision) -> Vec<String> {
        match decision {
            Decision::Rerun(plan) => plan.tests.iter().map(Test::id).collect(),
            other => panic!("not a re-run: {other:?}"),
        }
    }

    #[test]
    fn a_cluster_with_a_stall_is_run_again() {
        // 30 s of 100: 30%.
        let decision = decide(&cluster(), &observed(30.0, 100));
        assert_eq!(tests_of(&decision), ["cx::it a", "cx b::c", "cx::it d"]);
    }

    #[test]
    fn a_cluster_without_a_stall_is_not_run_again() {
        // 5 s of 100: 5%, below the threshold.
        let decision = decide(&cluster(), &observed(5.0, 100));
        let Decision::Declined { stall, why } = decision else {
            panic!("a cluster on a quiet disk was run again");
        };
        assert_eq!(stall.tests.len(), 3);
        assert_eq!(why, Declined::Quiet(5.0));
        assert!(why.message().contains("did not stall"), "{}", why.message());
    }

    #[test]
    fn the_threshold_is_a_tenth_of_the_runs_time_and_not_a_hair_under() {
        assert!(matches!(
            decide(&cluster(), &observed(10.0, 100)),
            Decision::Rerun(_)
        ));
        assert!(matches!(
            decide(&cluster(), &observed(9.9, 100)),
            Decision::Declined {
                why: Declined::Quiet(_),
                ..
            }
        ));
    }

    #[test]
    fn a_cluster_is_not_run_again_where_the_pressure_cannot_be_read() {
        let decision = decide(&cluster(), &Err("it is not Linux".to_owned()));
        let Decision::Declined { why, .. } = decision else {
            panic!("a cluster with no evidence was run again");
        };
        assert_eq!(why, Declined::Unreadable("it is not Linux".to_owned()));
        assert!(why.message().contains("no evidence"));
    }

    #[test]
    fn a_lone_timeout_is_never_run_again_whatever_the_pressure() {
        let lone = [timeout(
            "cx::it",
            "a",
            "2026-11-15T10:00:00.000+00:00",
            14.0,
        )];
        assert_eq!(decide(&lone, &observed(90.0, 100)), Decision::Nothing);
        // Two together are not a cluster either.
        let two = &cluster()[..2];
        assert_eq!(decide(two, &observed(90.0, 100)), Decision::Nothing);
        // Nor are timeouts apart, or stopped on time.
        let apart = [
            timeout("cx::it", "a", "2026-11-15T10:00:00.000+00:00", 14.0),
            timeout("cx::it", "b", "2026-11-15T10:00:10.000+00:00", 14.0),
            timeout("cx::it", "c", "2026-11-15T10:00:20.000+00:00", 14.0),
        ];
        assert_eq!(decide(&apart, &observed(90.0, 100)), Decision::Nothing);
        let on_time = [
            timeout("cx::it", "a", "2026-11-15T10:00:00.000+00:00", 10.01),
            timeout("cx::it", "b", "2026-11-15T10:00:00.005+00:00", 10.0),
            timeout("cx::it", "c", "2026-11-15T10:00:00.010+00:00", 10.0),
        ];
        assert_eq!(decide(&on_time, &observed(90.0, 100)), Decision::Nothing);
        assert_eq!(decide(&[], &observed(90.0, 100)), Decision::Nothing);
    }

    #[test]
    fn only_the_clusters_tests_are_run_again_and_no_failure_of_another_kind() {
        let mut failures = cluster();
        failures.push(Failure {
            kind: "test failure with exit code 101".to_owned(),
            ..timeout("cx::it", "panics", "2026-11-15T10:00:03.900+00:00", 14.0)
        });
        // A timeout far from the cluster, stopped late: not run again.
        failures.push(timeout(
            "cx::it",
            "lone",
            "2026-11-15T10:05:00.000+00:00",
            12.0,
        ));
        let decision = decide(&failures, &observed(30.0, 100));
        assert_eq!(tests_of(&decision), ["cx::it a", "cx b::c", "cx::it d"]);
    }

    #[test]
    fn a_test_whose_name_a_filter_cannot_name_exactly_is_not_run_again() {
        let mut failures = cluster();
        failures[1].name = "odd)name".to_owned();
        let decision = decide(&failures, &observed(30.0, 100));
        let Decision::Declined { why, .. } = decision else {
            panic!("a test with a `)` in its name was run again");
        };
        assert_eq!(why, Declined::Unnamable("cx odd)name".to_owned()));
    }

    #[test]
    fn the_filter_names_each_test_by_its_binary_and_name_exactly() {
        let tests = [
            Test {
                binary: "cx::it".to_owned(),
                name: "a::b".to_owned(),
            },
            Test {
                binary: "xtask::bin/xtask".to_owned(),
                name: "c".to_owned(),
            },
        ];
        assert_eq!(
            filter(&tests),
            "(binary_id(=cx::it) and test(=a::b)) or (binary_id(=xtask::bin/xtask) and test(=c))"
        );
    }

    #[test]
    fn the_filter_takes_the_place_of_the_runs_own_and_comes_before_the_tests_arguments() {
        let run: Vec<String> = [
            "nextest",
            "run",
            "--workspace",
            "--no-tests=pass",
            "-E",
            "not test(/sys_/)",
            "--",
            "--nocapture",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(
            with_filter(&run, "test(=x)").join(" "),
            "nextest run --workspace --no-tests=pass -E test(=x) -- --nocapture"
        );
        assert_eq!(
            with_filter(&run[..6], "test(=x)").join(" "),
            "nextest run --workspace --no-tests=pass -E test(=x)"
        );
    }

    // The runs, with reports written for the purpose in a folder of their
    // own and a command that writes them.

    fn case(binary: &str, name: &str, timestamp: &str, seconds: f64, failure: &str) -> String {
        let inner = if failure.is_empty() {
            String::new()
        } else {
            format!("\n            <failure type=\"{failure}\"/>\n        ")
        };
        format!(
            "        <testcase name=\"{name}\" classname=\"{binary}\" timestamp=\"{timestamp}\" \
             time=\"{seconds}\">{inner}</testcase>\n"
        )
    }

    /// A report of `cases`, in a suite of one binary each.
    fn report(cases: &[(&str, String, usize)]) -> String {
        let failures: usize = cases.iter().map(|(_, _, failing)| failing).sum();
        let mut text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"nextest-run\" \
             tests=\"{}\" failures=\"{failures}\" errors=\"0\" time=\"1.0\">\n",
            cases.len()
        );
        for (binary, body, failing) in cases {
            text.push_str(&format!(
                "    <testsuite name=\"{binary}\" tests=\"1\" disabled=\"0\" errors=\"0\" \
                 failures=\"{failing}\">\n{body}    </testsuite>\n"
            ));
        }
        text.push_str("</testsuites>\n");
        text
    }

    /// What the first run wrote: the cluster of three timeouts (of `cx::it`
    /// and `cx`), a test that passed, and `extra` suites.
    fn first_report(extra: &[(&str, String, usize)]) -> String {
        let mut cases = vec![
            (
                "cx::it",
                case(
                    "cx::it",
                    "a",
                    "2026-11-15T10:00:00.000+00:00",
                    14.0,
                    "test timeout",
                ),
                1,
            ),
            (
                "cx",
                case(
                    "cx",
                    "b::c",
                    "2026-11-15T10:00:02.000+00:00",
                    12.1,
                    "test timeout",
                ),
                1,
            ),
            (
                "cx::it2",
                case(
                    "cx::it2",
                    "d",
                    "2026-11-15T10:00:03.900+00:00",
                    10.3,
                    "test timeout",
                ),
                1,
            ),
            (
                "cx::fine",
                case("cx::fine", "ok", "2026-11-15T10:00:00.000+00:00", 0.25, ""),
                0,
            ),
        ];
        cases.extend(extra.iter().map(|(a, b, c)| (*a, b.clone(), *c)));
        report(&cases)
    }

    /// What the re-run writes for the three tests: each took a moment, and
    /// `failing` (a name and the kind of failure) did not pass.
    fn rerun_report(failing: &[(&str, &str)], leave_out: &[&str]) -> String {
        let all = [("cx::it", "a"), ("cx", "b::c"), ("cx::it2", "d")];
        let cases: Vec<(&str, String, usize)> = all
            .iter()
            .filter(|(_, name)| !leave_out.contains(name))
            .map(|(binary, name)| {
                let failure = failing
                    .iter()
                    .find(|(failing_name, _)| failing_name == name)
                    .map_or("", |(_, kind)| *kind);
                (
                    *binary,
                    case(binary, name, "2026-11-15T10:01:00.000+00:00", 0.5, failure),
                    usize::from(!failure.is_empty()),
                )
            })
            .collect();
        report(&cases)
    }

    struct Scene {
        folder: PathBuf,
        report: PathBuf,
    }

    impl Scene {
        fn new(label: &str) -> Self {
            let folder =
                std::env::temp_dir().join(format!("xtask-rerun-{label}-{}", std::process::id()));
            std::fs::create_dir_all(&folder).unwrap();
            let report = folder.join("junit.xml");
            Self { folder, report }
        }
    }

    impl Drop for Scene {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }

    /// What `run` did: its outcome, the commands it ran, the messages it
    /// said and the report it left.
    struct Ran {
        outcome: super::Outcome,
        commands: Vec<String>,
        said: String,
        left: String,
    }

    /// Runs with a first run that writes `first` and fails, whose pressure
    /// is `first_pressure`, and a second that writes `second` and ends as
    /// `second_ends`.
    fn ran(
        label: &str,
        first: &str,
        first_pressure: Result<Observed, String>,
        second: &str,
        second_ends: Result<(), String>,
    ) -> Ran {
        let scene = Scene::new(label);
        let commands = RefCell::new(Vec::new());
        let count = Cell::new(0);
        let said = RefCell::new(String::new());
        let mut exec = |args: &[String]| {
            commands.borrow_mut().push(args.join(" "));
            count.set(count.get() + 1);
            if count.get() == 1 {
                std::fs::write(&scene.report, first).unwrap();
                Attempt {
                    result: Err("`cargo nextest run` failed".to_owned()),
                    observed: first_pressure.clone(),
                }
            } else {
                std::fs::write(&scene.report, second).unwrap();
                Attempt {
                    result: second_ends.clone(),
                    observed: observed(0.0, 2),
                }
            }
        };
        let args: Vec<String> = ["nextest", "run", "--no-tests=pass", "-E", "not test(sys_)"]
            .map(str::to_owned)
            .to_vec();
        let outcome = run(&args, &scene.report, &mut exec, &mut |line| {
            said.borrow_mut().push_str(line);
            said.borrow_mut().push('\n');
        });
        let left = std::fs::read_to_string(&scene.report).unwrap();
        Ran {
            outcome,
            commands: commands.into_inner(),
            said: said.into_inner(),
            left,
        }
    }

    #[test]
    fn a_rerun_that_passes_replaces_the_timeouts_and_the_step_passes() {
        let ran = ran(
            "passes",
            &first_report(&[]),
            observed(30.0, 100),
            &rerun_report(&[], &[]),
            Ok(()),
        );
        assert_eq!(ran.outcome.result, Ok(()), "{}", ran.said);
        assert_eq!(ran.commands.len(), 2);
        assert_eq!(
            ran.commands[1],
            "nextest run --no-tests=pass -E (binary_id(=cx::it) and test(=a)) or \
             (binary_id(=cx) and test(=b::c)) or (binary_id(=cx::it2) and test(=d))"
        );
        // The report the tier check reads has no failure left, the tests'
        // times are the re-run's, and the other test's case is as it was.
        assert!(junit::failed(&ran.left).is_empty(), "{}", ran.left);
        let cases = junit::parse(&ran.left);
        assert_eq!(cases.len(), 4);
        let seconds: Vec<f64> = cases.iter().map(|case| case.seconds).collect();
        assert_eq!(seconds, [0.5, 0.5, 0.5, 0.25]);
        assert!(
            ran.left
                .contains("<testsuites name=\"nextest-run\" tests=\"4\" failures=\"0\"")
        );
        assert!(!ran.left.contains("failures=\"1\""), "{}", ran.left);
        assert!(
            ran.said
                .contains("Running the 3 tests that timed out together again")
        );
        assert!(
            ran.said.contains("cx::it a passed on the re-run"),
            "{}",
            ran.said
        );
        assert!(ran.said.contains("the step passes"), "{}", ran.said);
        // Both runs are recorded for the tier check, and the summary page
        // says what happened.
        assert_eq!(ran.outcome.records.len(), 2);
        assert!(ran.outcome.summary.unwrap().contains("run again once"));
    }

    #[test]
    fn a_timeout_that_recurs_on_the_rerun_fails_the_step() {
        let ran = ran(
            "recurs",
            &first_report(&[]),
            observed(30.0, 100),
            &rerun_report(&[("b::c", "test timeout")], &[]),
            Err("`cargo nextest run` failed".to_owned()),
        );
        let error = ran.outcome.result.unwrap_err();
        assert!(
            error.contains("1 of the 3 tests did not pass on the re-run"),
            "{error}"
        );
        assert!(ran.said.contains("cx b::c TIMED OUT AGAIN"), "{}", ran.said);
        // The report keeps that failure, and only that one.
        let failed: Vec<String> = junit::failed(&ran.left)
            .into_iter()
            .map(|failure| failure.name)
            .collect();
        assert_eq!(failed, ["b::c"]);
        assert!(ran.left.contains("failures=\"1\""));
    }

    #[test]
    fn a_failure_of_another_kind_beside_a_rerun_that_passes_still_fails_the_step() {
        // A test that panicked, in a suite of its own, beside the cluster.
        let panic = (
            "cx::panics",
            "        <testcase name=\"p\" classname=\"cx::panics\" time=\"0.1\">\n            \
             <failure type=\"test failure with exit code 101\">no</failure>\n        \
             </testcase>\n"
                .to_owned(),
            1,
        );
        let ran = ran(
            "panics",
            &first_report(&[panic]),
            observed(30.0, 100),
            &rerun_report(&[], &[]),
            Ok(()),
        );
        let error = ran.outcome.result.unwrap_err();
        assert!(error.contains("1 other test(s) failed"), "{error}");
        // It was not run again, and is still in the report.
        assert_eq!(ran.commands.len(), 2);
        assert!(!ran.commands[1].contains("test(=p)"));
        let failed: Vec<String> = junit::failed(&ran.left)
            .into_iter()
            .map(|failure| failure.name)
            .collect();
        assert_eq!(failed, ["p"]);
    }

    #[test]
    fn a_test_that_is_not_in_the_reruns_report_fails_the_step() {
        let ran = ran(
            "missing",
            &first_report(&[]),
            observed(30.0, 100),
            &rerun_report(&[], &["d"]),
            Ok(()),
        );
        let error = ran.outcome.result.unwrap_err();
        assert!(error.contains("1 of the 3 tests did not pass"), "{error}");
        assert!(
            ran.said.contains("cx::it2 d is NOT IN THE REPORT"),
            "{}",
            ran.said
        );
        // The report keeps the timeout it had.
        let failed: Vec<String> = junit::failed(&ran.left)
            .into_iter()
            .map(|failure| failure.name)
            .collect();
        assert_eq!(failed, ["d"]);
    }

    #[test]
    fn a_rerun_whose_command_failed_fails_the_step_whatever_its_report_says() {
        let ran = ran(
            "command",
            &first_report(&[]),
            observed(30.0, 100),
            &rerun_report(&[], &[]),
            Err("`cargo nextest run` failed".to_owned()),
        );
        let error = ran.outcome.result.unwrap_err();
        assert!(error.contains("the re-run's own command failed"), "{error}");
    }

    #[test]
    fn a_cluster_on_a_quiet_disk_runs_once_and_fails() {
        let ran = ran(
            "quiet",
            &first_report(&[]),
            observed(1.0, 100),
            &rerun_report(&[], &[]),
            Ok(()),
        );
        assert_eq!(ran.commands.len(), 1);
        assert!(ran.outcome.result.is_err());
        assert!(
            ran.said
                .contains("Not running them again: the disk did not stall"),
            "{}",
            ran.said
        );
        // The report is the run's own.
        assert_eq!(ran.left, first_report(&[]));
    }

    #[test]
    fn a_cluster_with_no_pressure_to_read_runs_once_and_fails() {
        let ran = ran(
            "unread",
            &first_report(&[]),
            Err("it is not Linux".to_owned()),
            &rerun_report(&[], &[]),
            Ok(()),
        );
        assert_eq!(ran.commands.len(), 1);
        assert!(ran.outcome.result.is_err());
        assert!(ran.outcome.records.is_empty());
        assert!(ran.said.contains("could not be read"), "{}", ran.said);
    }

    #[test]
    fn a_lone_timeout_on_a_stalled_disk_runs_once_and_fails() {
        let lone = report(&[(
            "cx::it",
            case(
                "cx::it",
                "a",
                "2026-11-15T10:00:00.000+00:00",
                14.0,
                "test timeout",
            ),
            1,
        )]);
        let ran = ran(
            "lone",
            &lone,
            observed(90.0, 100),
            &rerun_report(&[], &[]),
            Ok(()),
        );
        assert_eq!(ran.commands.len(), 1);
        assert!(ran.outcome.result.is_err());
        assert_eq!(ran.left, lone);
    }

    #[test]
    fn a_run_that_passes_is_not_run_again_and_a_report_that_is_not_there_is_a_failure() {
        let scene = Scene::new("clean");
        let mut count = 0;
        let outcome = run(
            &["nextest".to_owned()],
            &scene.report,
            &mut |_| {
                count += 1;
                Attempt {
                    result: Ok(()),
                    observed: observed(50.0, 100),
                }
            },
            &mut |_| {},
        );
        assert_eq!((outcome.result, count), (Ok(()), 1));
        // A failed run that wrote no report: its error stands.
        let outcome = run(
            &["nextest".to_owned()],
            &scene.report,
            &mut |_| Attempt {
                result: Err("the build failed".to_owned()),
                observed: observed(50.0, 100),
            },
            &mut |_| {},
        );
        assert_eq!(outcome.result, Err("the build failed".to_owned()));
    }

    #[test]
    fn tests_that_passed_on_their_retry_are_no_cluster_to_run_again() {
        // Three timeouts that ended together, each a try that a retry then
        // passed, and a test that failed: the run failed for that one.
        let flaky = |name: &str, started: &str, seconds: f64| {
            format!(
                "        <testcase name=\"{name}\" classname=\"cx::it\" timestamp=\"{started}\" \
                 time=\"{seconds}\">\n            <flakyFailure type=\"test timeout\"/>\n        \
                 </testcase>\n"
            )
        };
        let body = format!(
            "{}{}{}{}",
            flaky("a", "2026-11-15T10:00:00.000+00:00", 14.0),
            flaky("b", "2026-11-15T10:00:02.000+00:00", 12.1),
            flaky("c", "2026-11-15T10:00:03.900+00:00", 10.3),
            case(
                "cx::it",
                "p",
                "2026-11-15T10:00:00.000+00:00",
                0.1,
                "test failure with exit code 101"
            ),
        );
        let first = report(&[("cx::it", body, 1)]);
        let ran = ran(
            "flaky",
            &first,
            observed(30.0, 100),
            &rerun_report(&[], &[]),
            Ok(()),
        );
        assert_eq!(ran.commands.len(), 1, "{}", ran.said);
        assert!(ran.outcome.result.is_err());
        assert_eq!(ran.left, first);
    }

    #[test]
    fn each_test_that_was_run_again_is_judged_by_the_reruns_own_report() {
        let tests = [
            Test {
                binary: "cx::it".to_owned(),
                name: "a".to_owned(),
            },
            Test {
                binary: "cx".to_owned(),
                name: "b::c".to_owned(),
            },
            Test {
                binary: "cx::it2".to_owned(),
                name: "d".to_owned(),
            },
            Test {
                binary: "cx".to_owned(),
                name: "gone".to_owned(),
            },
        ];
        let report = rerun_report(&[("a", "test timeout"), ("b::c", "test failure")], &[]);
        assert_eq!(
            verdicts(&tests, &report),
            [
                Verdict::TimedOut,
                Verdict::Failed,
                Verdict::Passed,
                Verdict::Missing
            ]
        );
    }
}
