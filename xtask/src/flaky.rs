//! The known flaky tests, and running one again (since the tenth tooling
//! step).
//!
//! A test that fails now and then for a reason not yet fixed is an open
//! item, and its file names it: `- Flaky: <binary> <test>`
//! ([`crate::outstanding`]). The list is the items' own, so it cannot drift
//! from them: settling the item, or taking the line out, takes the test off
//! the list.
//!
//! `stress`, `ci` (away from CI) and the run of `mutate` with nothing broken
//! read it. When a listed test fails they name it with its item, run it
//! again **once**, and go on if that run passes; every such run is printed
//! and counted. A listed test that fails its second run is a failure, and a
//! test that is not listed is never run again. CI's blocking jobs run
//! nothing again: they name a listed test that failed, and fail as before.

use std::path::Path;

use crate::Result;
use crate::junit::{self, Failure};
use crate::outstanding;
use crate::stall::rerun::{self, Attempt, Test, Verdict};

/// A test the open items name as flaky, and the item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Listed {
    pub(crate) test: Test,
    /// The item's file name, without `.md`.
    pub(crate) item: String,
}

impl Listed {
    /// Where the item is, from the repository's root.
    pub(crate) fn file(&self) -> String {
        format!("{}{}.md", outstanding::FOLDER, self.item)
    }
}

/// The flaky tests the open items of the tree at `root` name. An item that
/// cannot be read names none, and is said in the warning: a list that is
/// short runs fewer tests again, never more.
pub(crate) fn listed(root: &Path) -> (Vec<Listed>, Option<String>) {
    let folder = root.join(outstanding::FOLDER);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return (
            Vec::new(),
            Some(format!(
                "could not read {}: no test is taken for a known flake",
                outstanding::FOLDER
            )),
        );
    };
    let mut texts = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Ok(text) = std::fs::read_to_string(entry.path()) {
            texts.push((format!("{}{name}", outstanding::FOLDER), text));
        }
    }
    let crates = outstanding::crates_in(root);
    let (items, problems) = outstanding::read(
        texts
            .iter()
            .map(|(file, text)| (file.as_str(), text.as_str())),
        &crates,
    );
    let warning = (!problems.is_empty()).then(|| {
        let files: Vec<&str> = problems.iter().map(|(file, _)| file.as_str()).collect();
        format!(
            "{} open item(s) could not be read, and any flaky test they name is not taken for \
             one: {} (`cargo xtask links` says why)",
            files.len(),
            files.join(", ")
        )
    });
    (from_items(&items), warning)
}

/// The flaky tests `items` name, in the items' order.
pub(crate) fn from_items(items: &[outstanding::Item]) -> Vec<Listed> {
    items
        .iter()
        .flat_map(|item| {
            item.flaky.iter().map(|flake| Listed {
                test: Test {
                    binary: flake.binary.clone(),
                    name: flake.test.clone(),
                },
                item: item.name.clone(),
            })
        })
        .collect()
}

/// The entry of `listed` for a test, by its binary and its name exactly.
pub(crate) fn find<'a>(listed: &'a [Listed], binary: &str, name: &str) -> Option<&'a Listed> {
    listed
        .iter()
        .find(|one| one.test.binary == binary && one.test.name == name)
}

/// Whether this runs under GitHub Actions, where nothing is run again.
pub(crate) fn in_ci() -> bool {
    std::env::var_os("GITHUB_ACTIONS").is_some_and(|value| value == "true")
}

/// The line that names a listed test that failed.
pub(crate) fn named(listed: &Listed) -> String {
    format!(
        "{} {} failed, and is a known flake: {}",
        listed.test.binary,
        listed.test.name,
        listed.file()
    )
}

/// The failures of `failed` that `listed` names, each once, in the order
/// they failed, and how many failures are of other tests.
pub(crate) fn split<'a>(failed: &[Failure], listed: &'a [Listed]) -> (Vec<&'a Listed>, usize) {
    let mut flakes: Vec<&Listed> = Vec::new();
    let mut others = 0;
    for failure in failed {
        match find(listed, &failure.binary, &failure.name) {
            Some(one) => {
                if !flakes.contains(&one) {
                    flakes.push(one);
                }
            }
            None => others += 1,
        }
    }
    (flakes, others)
}

/// How a run of tests ended after its listed flakes were seen to.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) result: Result<()>,
    /// For a summary page, when a listed test failed.
    pub(crate) summary: Option<String>,
}

/// After a run of `args` that failed with `error`, its report at `report`:
/// the listed tests among its failures named, and, unless `rerun` is
/// false (in CI), run again once with `exec`. The run then passes only
/// when its report, with each such test's case replaced by the second
/// run's, holds no failed case, the second run's command succeeded, and
/// every test run again passed. Nothing else is run again.
pub(crate) fn after_failure(
    args: &[String],
    report: &Path,
    error: String,
    listed: &[Listed],
    rerun: bool,
    exec: &mut dyn FnMut(&[String]) -> Attempt,
    say: &mut dyn FnMut(&str),
) -> Outcome {
    let failed_as_it_was = |error: String| Outcome {
        result: Err(error),
        summary: None,
    };
    let Ok(first_report) = std::fs::read_to_string(report) else {
        return failed_as_it_was(error);
    };
    let (flakes, others) = split(&junit::failed(&first_report), listed);
    if flakes.is_empty() {
        return failed_as_it_was(error);
    }
    for flake in &flakes {
        say(&named(flake));
    }
    if !rerun {
        say(&format!(
            "Not run again: CI's blocking jobs run nothing again. {} known flake(s) failed{}.",
            flakes.len(),
            if others > 0 {
                format!(", and {others} other test(s)")
            } else {
                String::new()
            }
        ));
        return Outcome {
            result: Err(error),
            summary: Some(summary(&flakes, None, others, false)),
        };
    }
    let tests: Vec<Test> = flakes.iter().map(|flake| flake.test.clone()).collect();
    say(&format!(
        "Running the {} known flake(s) again, once. A flake that fails again, and every other \
         failure ({others}), still fails this step.",
        tests.len()
    ));
    let second = exec(&rerun::with_filter(args, &rerun::filter(&tests)));
    let put_back = |why: &str| {
        let _ = std::fs::write(report, &first_report);
        Outcome {
            result: Err(format!("{error}; {why}")),
            summary: Some(summary(&flakes, None, others, true)),
        }
    };
    let Ok(second_report) = std::fs::read_to_string(report) else {
        say("The run of the known flakes left no report.");
        return put_back("the run of the known flakes left no report");
    };
    let pairs: Vec<(String, String)> = tests
        .iter()
        .map(|test| (test.binary.clone(), test.name.clone()))
        .collect();
    let combined = junit::replace_cases(&first_report, &second_report, &pairs);
    if let Err(why) = std::fs::write(report, &combined) {
        let why = format!("could not write {}: {why}", report.display());
        say(&why);
        return put_back(&why);
    }
    let verdicts = rerun::verdicts(&tests, &second_report);
    for (flake, verdict) in flakes.iter().zip(&verdicts) {
        say(&format!(
            "  {} {} {}",
            flake.test.binary,
            flake.test.name,
            says(*verdict)
        ));
    }
    let not_passed = verdicts
        .iter()
        .filter(|verdict| **verdict != Verdict::Passed)
        .count();
    let left = junit::failed(&combined).len();
    let mut problems = Vec::new();
    if not_passed > 0 {
        problems.push(format!(
            "{not_passed} of the {} known flake(s) failed again",
            tests.len()
        ));
    }
    let other_failures = left.saturating_sub(not_passed);
    if other_failures > 0 {
        problems.push(format!(
            "{other_failures} other test(s) failed, which are not run again"
        ));
    }
    if let Err(why) = &second.result
        && problems.is_empty()
    {
        problems.push(format!("the run of the known flakes failed: {why}"));
    }
    let summary = Some(summary(&flakes, Some(&verdicts), others, true));
    if problems.is_empty() {
        say(&format!(
            "The {} known flake(s) passed when run again, and nothing else failed: the step \
             passes. Their times in the report are the second run's.",
            tests.len()
        ));
        return Outcome {
            result: Ok(()),
            summary,
        };
    }
    say(&format!("The step fails: {}.", problems.join("; ")));
    Outcome {
        result: Err(format!("{error}; {}", problems.join("; "))),
        summary,
    }
}

/// How a known flake's second run went, in words.
pub(crate) fn says(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Passed => "passed when run again",
        Verdict::TimedOut => "TIMED OUT AGAIN",
        Verdict::Failed => "FAILED AGAIN",
        Verdict::Missing => "is NOT IN THE REPORT of its second run",
    }
}

/// What a summary page says of the known flakes that failed.
fn summary(flakes: &[&Listed], verdicts: Option<&[Verdict]>, others: usize, rerun: bool) -> String {
    let mut text = String::from("### Known flakes that failed\n\n");
    if !rerun {
        text.push_str("Not run again: CI's blocking jobs run nothing again.\n\n");
    }
    text.push_str("| Test | Item | Second run |\n|---|---|---|\n");
    for (index, flake) in flakes.iter().enumerate() {
        let second = match verdicts.and_then(|verdicts| verdicts.get(index)) {
            Some(verdict) => says(*verdict),
            None if rerun => "none: the second run failed before it",
            None => "not run",
        };
        text.push_str(&format!(
            "| `{} {}` | `{}` | {second} |\n",
            flake.test.binary,
            flake.test.name,
            flake.file()
        ));
    }
    if others > 0 {
        text.push_str(&format!(
            "\n{others} other test(s) failed, which are not run again.\n"
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::{Listed, Outcome, after_failure, split};
    use crate::junit::{self, Failure};
    use crate::stall::rerun::{Attempt, Test};

    fn listed() -> Vec<Listed> {
        vec![
            Listed {
                test: Test {
                    binary: "cx::it".to_owned(),
                    name: "trust::sys_flaky".to_owned(),
                },
                item: "a-flaky-trust-test".to_owned(),
            },
            Listed {
                test: Test {
                    binary: "cx".to_owned(),
                    name: "memory::peak".to_owned(),
                },
                item: "a-memory-peak".to_owned(),
            },
        ]
    }

    fn failure(binary: &str, name: &str) -> Failure {
        Failure {
            binary: binary.to_owned(),
            name: name.to_owned(),
            repeat: None,
            text: "panicked".to_owned(),
            kind: "test failure".to_owned(),
            stderr: String::new(),
            started: "2026-11-16T10:00:00.000+00:00".to_owned(),
            seconds: 0.5,
        }
    }

    /// A known flake whose test was renamed or removed would be run again
    /// by nobody: each one the open items name is a function of its crate.
    #[test]
    fn every_known_flake_names_a_test_its_crate_has() {
        let root = crate::workspace::root();
        let (listed, warning) = super::listed(&root);
        assert_eq!(warning, None);
        assert!(!listed.is_empty(), "the open items name no flaky test");
        for one in &listed {
            let package = one.test.binary.split("::").next().unwrap();
            let function = one.test.name.rsplit("::").next().unwrap();
            let folder = if package == "xtask" {
                root.join("xtask")
            } else {
                root.join("crates").join(package)
            };
            let mut stack = vec![folder];
            let mut found = false;
            while let Some(folder) = stack.pop() {
                for entry in std::fs::read_dir(&folder).unwrap().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|ending| ending == "rs") {
                        let text = std::fs::read_to_string(&path).unwrap_or_default();
                        found |= text.contains(&format!("fn {function}("));
                    }
                }
            }
            assert!(
                found,
                "{} names {}, and its crate has no `fn {function}`",
                one.file(),
                one.test.name
            );
        }
    }

    #[test]
    fn a_failure_is_a_known_flake_only_by_its_binary_and_its_name_exactly() {
        let list = listed();
        let failed = [
            failure("cx::it", "trust::sys_flaky"),
            // The same name in another binary, a name that ends the same,
            // and one that begins the same are other tests.
            failure("cx", "trust::sys_flaky"),
            failure("cx::it", "other::trust::sys_flaky"),
            failure("cx::it", "trust::sys_flaky_too"),
            // Twice, as a stress run's report has it: one flake.
            failure("cx::it", "trust::sys_flaky"),
        ];
        let (flakes, others) = split(&failed, &list);
        assert_eq!(flakes, [&list[0]]);
        assert_eq!(others, 3);
    }

    fn case(binary: &str, name: &str, failure: bool) -> String {
        let inner = if failure {
            "<failure type=\"test failure\"/>"
        } else {
            ""
        };
        format!(
            "    <testsuite name=\"{binary}\" tests=\"1\" failures=\"{}\">\n        <testcase \
             name=\"{name}\" classname=\"{binary}\" timestamp=\"2026-11-16T10:00:00.000+00:00\" \
             time=\"0.5\">{inner}</testcase>\n    </testsuite>\n",
            usize::from(failure)
        )
    }

    fn report(cases: &[(&str, &str, bool)]) -> String {
        let failures = cases.iter().filter(|(_, _, failed)| *failed).count();
        let mut text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"nextest-run\" \
             tests=\"{}\" failures=\"{failures}\" errors=\"0\" time=\"1.0\">\n",
            cases.len()
        );
        for (binary, name, failed) in cases {
            text.push_str(&case(binary, name, *failed));
        }
        text.push_str("</testsuites>\n");
        text
    }

    struct Ran {
        outcome: Outcome,
        commands: Vec<String>,
        said: String,
        left: String,
    }

    /// A run that failed and wrote `first`, then `after_failure` with a
    /// second run that writes `second` (`None`: no report) and ends as
    /// `second_ends`.
    fn ran(
        label: &str,
        first: &str,
        second: Option<&str>,
        second_ends: Result<(), String>,
        rerun: bool,
    ) -> Ran {
        let folder: PathBuf =
            std::env::temp_dir().join(format!("xtask-flaky-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let report = folder.join("junit.xml");
        std::fs::write(&report, first).unwrap();
        let commands = RefCell::new(Vec::new());
        let said = RefCell::new(String::new());
        let mut exec = |args: &[String]| {
            commands.borrow_mut().push(args.join(" "));
            match second {
                Some(second) => std::fs::write(&report, second).unwrap(),
                None => std::fs::remove_file(&report).unwrap(),
            }
            Attempt {
                result: second_ends.clone(),
                observed: Err("not read here".to_owned()),
            }
        };
        let args: Vec<String> = ["nextest", "run", "-E", "not test(sys_)"]
            .map(str::to_owned)
            .to_vec();
        let outcome = after_failure(
            &args,
            &report,
            "`cargo nextest run` failed".to_owned(),
            &listed(),
            rerun,
            &mut exec,
            &mut |line| {
                said.borrow_mut().push_str(line);
                said.borrow_mut().push('\n');
            },
        );
        let left = std::fs::read_to_string(&report).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&folder);
        Ran {
            outcome,
            commands: commands.into_inner(),
            said: said.into_inner(),
            left,
        }
    }

    const FLAKE: (&str, &str, bool) = ("cx::it", "trust::sys_flaky", true);
    const FLAKE_PASSES: (&str, &str, bool) = ("cx::it", "trust::sys_flaky", false);
    const FINE: (&str, &str, bool) = ("cx", "fine", false);

    #[test]
    fn a_known_flake_that_passes_its_second_run_lets_the_run_pass() {
        let ran = ran(
            "passes",
            &report(&[FLAKE, FINE]),
            Some(&report(&[FLAKE_PASSES])),
            Ok(()),
            true,
        );
        assert_eq!(ran.outcome.result, Ok(()), "{}", ran.said);
        // Only the flake is run again, by its binary and its name, in place
        // of the run's own filter.
        assert_eq!(
            ran.commands,
            ["nextest run -E (binary_id(=cx::it) and test(=trust::sys_flaky))"]
        );
        assert!(
            ran.said.contains(
                "cx::it trust::sys_flaky failed, and is a known flake: \
                 docs/outstanding/a-flaky-trust-test.md"
            ),
            "{}",
            ran.said
        );
        assert!(
            ran.said
                .contains("Running the 1 known flake(s) again, once")
        );
        assert!(
            ran.said
                .contains("cx::it trust::sys_flaky passed when run again")
        );
        // The report has the second run's case and no failure; the summary
        // counts the flake.
        assert!(junit::failed(&ran.left).is_empty(), "{}", ran.left);
        assert_eq!(junit::parse(&ran.left).len(), 2);
        let summary = ran.outcome.summary.unwrap();
        assert!(
            summary
                .contains("| `cx::it trust::sys_flaky` | `docs/outstanding/a-flaky-trust-test.md`")
                && summary.contains("passed when run again"),
            "{summary}"
        );
    }

    #[test]
    fn a_known_flake_that_fails_again_fails_the_run() {
        let ran = ran(
            "again",
            &report(&[FLAKE, FINE]),
            Some(&report(&[FLAKE])),
            Err("`cargo nextest run` failed".to_owned()),
            true,
        );
        let error = ran.outcome.result.unwrap_err();
        assert!(
            error.contains("1 of the 1 known flake(s) failed again"),
            "{error}"
        );
        assert!(ran.said.contains("FAILED AGAIN"), "{}", ran.said);
        assert_eq!(junit::failed(&ran.left).len(), 1);
    }

    #[test]
    fn a_test_that_is_not_listed_is_never_run_again() {
        // Alone: nothing is run, nothing is said.
        let ran_alone = ran(
            "alone",
            &report(&[("cx", "other", true), FINE]),
            Some(&report(&[])),
            Ok(()),
            true,
        );
        assert!(ran_alone.outcome.result.is_err());
        assert!(ran_alone.commands.is_empty(), "{:?}", ran_alone.commands);
        assert!(ran_alone.said.is_empty(), "{}", ran_alone.said);
        assert!(ran_alone.outcome.summary.is_none());

        // Beside a flake: the flake alone is run again, and the other
        // failure still fails the run, though the flake passed.
        let ran = ran(
            "beside",
            &report(&[FLAKE, ("cx", "other", true)]),
            Some(&report(&[FLAKE_PASSES])),
            Ok(()),
            true,
        );
        assert_eq!(ran.commands.len(), 1);
        assert!(!ran.commands[0].contains("other"), "{:?}", ran.commands);
        let error = ran.outcome.result.unwrap_err();
        assert!(
            error.contains("1 other test(s) failed, which are not run again"),
            "{error}"
        );
        assert_eq!(junit::failed(&ran.left).len(), 1);
    }

    #[test]
    fn in_ci_a_known_flake_is_named_and_nothing_is_run_again() {
        let ran = ran(
            "ci",
            &report(&[FLAKE, FINE]),
            Some(&report(&[FLAKE_PASSES])),
            Ok(()),
            false,
        );
        assert!(ran.outcome.result.is_err());
        assert!(ran.commands.is_empty(), "{:?}", ran.commands);
        assert!(ran.said.contains("is a known flake"), "{}", ran.said);
        assert!(
            ran.said.contains("CI's blocking jobs run nothing again"),
            "{}",
            ran.said
        );
        assert_eq!(junit::failed(&ran.left).len(), 1, "the report is as it was");
        assert!(ran.outcome.summary.unwrap().contains("| not run |"));
    }

    #[test]
    fn a_second_run_that_is_missing_or_whose_command_failed_fails_the_run() {
        // No report: the first is put back.
        let first = report(&[FLAKE, FINE]);
        let ran_gone = ran("gone", &first, None, Ok(()), true);
        let error = ran_gone.outcome.result.unwrap_err();
        assert!(error.contains("left no report"), "{error}");
        assert_eq!(ran_gone.left, first);

        // The flake missing from the second run's report.
        let ran_missing = ran("missing", &first, Some(&report(&[FINE])), Ok(()), true);
        assert!(ran_missing.outcome.result.is_err());
        assert!(
            ran_missing.said.contains("NOT IN THE REPORT"),
            "{}",
            ran_missing.said
        );

        // The command failed though its report says the flake passed.
        let ran_failed = ran(
            "failed",
            &first,
            Some(&report(&[FLAKE_PASSES])),
            Err("nextest: a setup script failed".to_owned()),
            true,
        );
        let error = ran_failed.outcome.result.unwrap_err();
        assert!(
            error.contains("the run of the known flakes failed"),
            "{error}"
        );
    }
}
