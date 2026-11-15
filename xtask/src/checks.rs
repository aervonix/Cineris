//! The check and test commands. The blocking CI jobs run `check` and
//! `ci test`, so the commands here are the definition of what CI runs
//! (`docs/testing-and-ci.md`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::Result;
use crate::mutate;
use crate::rules;
use crate::runs::{self, Source};
use crate::schema;
use crate::steps::{self, Step};
use crate::workspace;
use crate::{compare, tiers};

/// The tier 0 steps, in the order they run: cheapest first.
const CHECK_STEPS: [&str; 6] = ["fmt", "rules", "breaks", "generated", "clippy", "deny"];

fn check_step(name: &str) -> Option<Step> {
    Some(match name {
        "fmt" => Step::cargo("fmt", &["fmt", "--all", "--check"]),
        "rules" => Step::function("rules", rules),
        // Every break in `mutations/` still finds the code it changes
        // (docs/testing-and-ci.md, "Breaking the code on purpose"). It reads
        // a few files and runs nothing.
        "breaks" => Step::function("breaks", mutate::check_breaks_apply),
        // The checked-in protocol schema and TypeScript declarations are what
        // the types generate (docs/architecture/11-rpc-protocol.md).
        "generated" => Step::function("generated", schema::check),
        "clippy" => Step::cargo(
            "clippy",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        ),
        // Advisories are not here: a new advisory against an unchanged
        // lockfile must not block unrelated pull requests. They run nightly.
        "deny" => Step::cargo("deny", &["deny", "check", "licenses", "bans", "sources"]),
        _ => return None,
    })
}

/// `cargo xtask check [STEP...]`
pub(crate) fn check(arguments: &[String]) -> Result<()> {
    let names: Vec<&str> = if arguments.is_empty() {
        CHECK_STEPS.to_vec()
    } else {
        arguments.iter().map(String::as_str).collect()
    };
    let mut selected = Vec::new();
    for name in names {
        selected.push(check_step(name).ok_or_else(|| {
            format!(
                "unknown check `{name}`. The checks are: {}",
                CHECK_STEPS.join(", ")
            )
        })?);
    }
    steps::run("Static checks", selected)
}

/// `cargo xtask rules`
pub(crate) fn rules() -> Result<()> {
    let violations = rules::check_repository(&workspace::root())?;
    if violations.is_empty() {
        println!("The repository follows its rules.");
        return Ok(());
    }
    for violation in &violations {
        println!("{violation}\n");
    }
    Err(format!("{} repo-rule violation(s)", violations.len()))
}

/// `cargo xtask links`: the part of the repo rules that is about the docs,
/// by itself. It reads text files and asks cargo nothing, so it answers
/// before a page that was just edited is out of mind.
pub(crate) fn links() -> Result<()> {
    let root = workspace::root();
    let files = workspace::tracked_files(&root)?;
    let names = workspace::repository_files(&root)?;
    let violations = rules::check_docs(&root, &files, &names)?;
    if violations.is_empty() {
        println!(
            "Every link inside the repository leads somewhere, and the step records are in their places."
        );
        return Ok(());
    }
    for violation in &violations {
        println!("{violation}\n");
    }
    Err(format!(
        "{} broken link(s) or misplaced record(s)",
        violations.len()
    ))
}

/// `cargo xtask fmt`
pub(crate) fn fmt() -> Result<()> {
    steps::cargo(&["fmt", "--all"])
}

/// `cargo xtask test [ARGS...]`
///
/// The whole workspace, unless the arguments name packages: `test -p
/// cortex-agent` builds and runs that crate's tests and no other's.
///
/// `--dependents` beside `-p` adds every workspace crate whose tests can see
/// a change to the crates named ([`crate::dependents`]).
pub(crate) fn test(arguments: &[String]) -> Result<()> {
    require_nextest()?;
    let arguments = &crate::dependents::expand(arguments)?;
    let command = test_command(arguments);
    workspace::say_implied(&workspace::implied_features(&workspace::packages_named(
        arguments,
    )));
    steps::cargo(&command)
}

/// The arguments of `cargo` for `cargo xtask test <arguments>`.
fn test_command(arguments: &[String]) -> Vec<String> {
    let mut command = vec!["nextest".to_owned(), "run".to_owned()];
    if !selects_packages(arguments) {
        command.push("--workspace".to_owned());
    }
    command.extend(workspace::implied_features(&workspace::packages_named(
        arguments,
    )));
    command.extend(arguments.iter().cloned());
    command
}

/// Whether arguments for `cargo nextest run` already say which packages to
/// build and test. To cargo, `--workspace` beside `-p` means the whole
/// workspace, so it is added only when nothing was chosen.
pub(crate) fn selects_packages(arguments: &[String]) -> bool {
    arguments
        .iter()
        // What follows `--` goes to the test binaries.
        .take_while(|argument| *argument != "--")
        .any(|argument| {
            matches!(
                argument.as_str(),
                "-p" | "--package" | "--workspace" | "--all"
            ) || argument.starts_with("--package=")
                // `-pcortex-agent`. No other option of nextest starts `-p`.
                || (argument.starts_with("-p") && !argument.starts_with("--"))
        })
}

/// Fails with a pointer to the installer when cargo-nextest is missing,
/// instead of cargo's "no such command".
pub(crate) fn require_nextest() -> Result<()> {
    let found = Command::new(workspace::cargo())
        .args(["nextest", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if found {
        return Ok(());
    }
    Err(
        "cargo-nextest is not installed. See https://nexte.st. The version CI uses is in \
         docs/dependencies.md."
            .to_owned(),
    )
}

/// The blocking test job: tiers 1 to 3, in nextest's `ci` profile, built with
/// cargo's `ci` profile (third-party crates unoptimised, so that a cold cache
/// still fits the time budget). Building and running are separate steps so
/// that their times are reported separately, and so are the job's two runs
/// ([`ci_runs`]).
fn ci_test_steps() -> Vec<Step> {
    let mut build = ci_run(&[]);
    build.push("--no-run".to_owned());
    let mut all = vec![
        // The last run's report is not this run's. If the build fails, the
        // tier report then says there is nothing to read.
        Step::closure("build tests", move || {
            forget_reports(&workspace::root())?;
            steps::cargo(&build)
        }),
    ];
    all.extend(run_steps(ci_runs(&[], &[], true)));
    // Each test has its own limit in .config/nextest.toml. This checks
    // what the tests of each tier took together.
    all.push(Step::function("tier budgets", tiers::check_ci_report));
    all
}

/// The system tests, tier 3, as `.config/nextest.toml` picks them: by name.
pub(crate) const SYSTEM_TESTS: &str = "test(/(^|::)sys_/)";

/// The start of the line of `.config/nextest.toml` that gives the job's
/// second run, the system tests, a group of its own. A tree without it is
/// from before the job ran in two (`ab`'s base can be), and its job is one
/// run.
const SYSTEM_GROUP: &str = "system = { max-threads";

/// Whether the blocking test job of the tree at `root` is two runs, by its
/// own `.config/nextest.toml`.
pub(crate) fn splits(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".config").join("nextest.toml"))
        .is_ok_and(|text| text.lines().any(|line| line.starts_with(SYSTEM_GROUP)))
}

/// The blocking test job's runs, in order, for the packages named (empty:
/// the workspace), with `rest` (more arguments for nextest) given to each.
///
/// Split, by the owner's decision of 2026-11-15, tiers 1 and 2 run first,
/// and then the system tests alone, three at a time (the `ci` profile's
/// `system` group), so that each tier is measured under its own load and
/// not beside the others' (docs/testing-and-ci.md, "The tiers"). nextest
/// takes a test that matches any of its filtersets, so a filterset in
/// `rest` is joined to each run's own with `and`, not given beside it.
pub(crate) fn ci_runs(packages: &[String], rest: &[String], split: bool) -> Vec<Vec<String>> {
    if !split {
        let mut run = ci_run(packages);
        run.extend(rest.iter().cloned());
        return vec![run];
    }
    let (given, rest) = filtersets(rest);
    [format!("not {SYSTEM_TESTS}"), SYSTEM_TESTS.to_owned()]
        .into_iter()
        .map(|own| {
            let filter = match &given {
                Some(given) => format!("({given}) and ({own})"),
                None => own,
            };
            let mut run = ci_run(packages);
            // A crate may have no system test, or nothing but.
            run.extend(["--no-tests=pass".to_owned(), "-E".to_owned(), filter]);
            run.extend(rest.iter().cloned());
            run
        })
        .collect()
}

/// The filtersets among arguments for nextest, joined with `or` as nextest
/// joins them, and the other arguments.
fn filtersets(arguments: &[String]) -> (Option<String>, Vec<String>) {
    const NAMES: [&str; 3] = ["-E", "--filterset", "--filter-expr"];
    let mut found = Vec::new();
    let mut rest = Vec::new();
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            rest.push(word.clone());
            rest.extend(words.by_ref().cloned());
            break;
        }
        if NAMES.contains(&word.as_str()) {
            if let Some(value) = words.next() {
                found.push(value.clone());
            }
            continue;
        }
        let joined = NAMES
            .iter()
            .find_map(|name| word.strip_prefix(&format!("{name}=")));
        match joined {
            Some(value) => found.push(value.to_owned()),
            None => rest.push(word.clone()),
        }
    }
    let joined = match found.as_slice() {
        [] => None,
        [one] => Some(one.clone()),
        many => Some(
            many.iter()
                .map(|one| format!("({one})"))
                .collect::<Vec<_>>()
                .join(" or "),
        ),
    };
    (joined, rest)
}

/// The steps that run `runs` in this tree, and leave one report of them all
/// where one run would leave its own.
fn run_steps(runs: Vec<Vec<String>>) -> Vec<Step> {
    if runs.len() == 1 {
        return runs
            .into_iter()
            .map(|run| Step::cargo_owned("run tests", run))
            .collect();
    }
    let names = ["run tiers 1 and 2", "run the system tests"];
    let last = runs.len() - 1;
    runs.into_iter()
        .enumerate()
        .map(|(index, run)| {
            Step::closure(names[index.min(1)], move || {
                let watch = crate::stall::Watch::start();
                let ran = steps::cargo(&run);
                let root = workspace::root();
                if ran.is_err() {
                    say_if_stalled(&watch, &workspace::nextest_report(&root, "ci"));
                }
                if index == last {
                    bring_together(&root)?;
                } else {
                    set_aside(&root)?;
                }
                ran
            })
        })
        .collect()
}

/// After a run that failed: says so when its timeouts ended together, the
/// sign of a machine that stalled (`crate::stall`).
pub(crate) fn say_if_stalled(watch: &crate::stall::Watch, report: &Path) {
    let Ok(xml) = std::fs::read_to_string(report) else {
        return;
    };
    if let Some(stalled) = watch.explain(&crate::junit::failures(&xml)) {
        println!("{stalled}");
    }
}

/// Where the first run's report waits for the second's, beside it.
fn aside_path(root: &Path) -> PathBuf {
    workspace::nextest_report(root, "ci").with_file_name("junit-tiers-1-and-2.xml")
}

/// Removes the reports of an earlier run of the job in the tree at `root`.
pub(crate) fn forget_reports(root: &Path) -> Result<()> {
    workspace::forget_report(&workspace::nextest_report(root, "ci"))?;
    workspace::forget_report(&aside_path(root))
}

/// After a run of the job that is not its last: its report, moved out of
/// the next run's way.
pub(crate) fn set_aside(root: &Path) -> Result<()> {
    let (report, aside) = (workspace::nextest_report(root, "ci"), aside_path(root));
    workspace::forget_report(&aside)?;
    if report.is_file() {
        std::fs::rename(&report, &aside)
            .map_err(|error| format!("could not move {}: {error}", report.display()))?;
    }
    Ok(())
}

/// After the job's last run: the report set aside and the last run's, as
/// one report in the last's place.
pub(crate) fn bring_together(root: &Path) -> Result<()> {
    let (report, aside) = (workspace::nextest_report(root, "ci"), aside_path(root));
    if !aside.is_file() {
        return Ok(());
    }
    let read = |path: &Path| {
        std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))
    };
    let merged = if report.is_file() {
        crate::junit::merge(&read(&aside)?, &read(&report)?)?
    } else {
        read(&aside)?
    };
    std::fs::write(&report, merged)
        .map_err(|error| format!("could not write {}: {error}", report.display()))?;
    workspace::forget_report(&aside)
}

/// `cargo nextest run` as the blocking test job runs it, for the whole
/// workspace or for the packages named.
pub(crate) fn ci_run(packages: &[String]) -> Vec<String> {
    let mut run = vec!["nextest".to_owned(), "run".to_owned()];
    if packages.is_empty() {
        run.push("--workspace".to_owned());
    }
    for package in packages {
        run.extend(["-p".to_owned(), package.clone()]);
    }
    run.extend(workspace::implied_features(packages));
    run.extend(["--locked", "--cargo-profile", "ci", "--profile", "ci"].map(str::to_owned));
    run
}

/// `cargo xtask ci [lint|test]`, and `cargo xtask ci --tiers ...`
pub(crate) fn ci(arguments: &[String]) -> Result<()> {
    if arguments.first().is_some_and(|first| first == "--tiers") {
        return ci_tiers(&arguments[1..]);
    }
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["lint"] => check(&[]),
        ["test"] => {
            require_nextest()?;
            steps::run("Tests", ci_test_steps())
        }
        [] => {
            require_nextest()?;
            let mut all: Vec<Step> = CHECK_STEPS
                .iter()
                .filter_map(|name| check_step(name))
                .collect();
            all.extend(ci_test_steps());
            steps::run("Everything that blocks a pull request", all)
        }
        _ => Err(CI_USAGE.to_owned()),
    }
}

const CI_USAGE: &str = "usage: cargo xtask ci [lint|test]
       cargo xtask ci --tiers [-p CRATE]... [--top N] [-- NEXTEST ARGUMENT...]
       cargo xtask ci --tiers --from JUNIT.XML [--os linux|windows] [--top N]
       cargo xtask ci --tiers --compare NEW [--against OLD] [--os linux|windows] [--top N]
                              NEW and OLD: a JUnit report, a run's id, or `main` (OLD's default)";

/// How many of each tier's slowest tests `ci --tiers` lists when it is not
/// told.
const TIERS_TOP: usize = 10;

/// What `cargo xtask ci --tiers` was asked for.
#[derive(Debug, PartialEq)]
struct TiersRequest {
    /// The packages to build and test. Empty: the workspace.
    packages: Vec<String>,
    /// How many of each tier's slowest tests to list.
    top: usize,
    /// A report to read in place of running anything.
    from: Option<PathBuf>,
    /// The OS the report is from, which decides tier 3's budget.
    os: tiers::Os,
    /// A report to compare with `against`, in place of running anything.
    compare: Option<Source>,
    /// What `compare` is compared with: the main branch when not given.
    against: Option<Source>,
    /// What follows `--`: more arguments for nextest, a filter for example.
    rest: Vec<String>,
    /// Run at a runner's test threads, and estimate what a runner of each
    /// OS would report (`like_runner`).
    like_runner: bool,
    /// A `try` A/B run to measure the factors of `like_runner` from, with
    /// the desk's report `from`.
    factors_from: Option<Source>,
}

impl TiersRequest {
    fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Self {
            packages: Vec::new(),
            top: TIERS_TOP,
            from: None,
            os: tiers::Os::here(),
            compare: None,
            against: None,
            rest: Vec::new(),
            like_runner: false,
            factors_from: None,
        };
        let mut os_given = false;
        let mut rest = arguments.iter();
        while let Some(argument) = rest.next() {
            let mut value = |what: &str| {
                rest.next()
                    .ok_or_else(|| format!("`{argument}` needs {what}\n{CI_USAGE}"))
            };
            match argument.as_str() {
                "--" => {
                    request.rest = rest.cloned().collect();
                    break;
                }
                "-p" | "--package" => request.packages.push(value("a crate")?.clone()),
                "--top" => {
                    let text = value("a number")?;
                    request.top = text
                        .parse()
                        .map_err(|_| format!("`--top {text}` is not a number"))?;
                }
                "--from" => request.from = Some(PathBuf::from(value("a JUnit report")?)),
                "--like-runner" => request.like_runner = true,
                "--factors-from" => {
                    request.factors_from = Some(Source::parse(value("a try run's id")?));
                }
                "--compare" => {
                    request.compare = Some(Source::parse(value("a report, a run or `main`")?));
                }
                "--against" => {
                    request.against = Some(Source::parse(value("a report, a run or `main`")?));
                }
                "--os" => {
                    os_given = true;
                    request.os = match value("`linux` or `windows`")?.as_str() {
                        "linux" => tiers::Os::Linux,
                        "windows" => tiers::Os::Windows,
                        other => {
                            return Err(format!("`--os {other}`: say `linux` or `windows`"));
                        }
                    };
                }
                other => return Err(format!("unknown argument `{other}`\n{CI_USAGE}")),
            }
        }
        if request.against.is_some() && request.compare.is_none() {
            return Err("`--against` says what `--compare` is compared with".to_owned());
        }
        if request.compare.is_some() && request.from.is_some() {
            return Err("`--compare` and `--from` both read a report: give one".to_owned());
        }
        if request.compare.is_some() {
            request.against.get_or_insert(Source::Main);
        }
        let reads = request.from.is_some() || request.compare.is_some();
        if reads && !(request.packages.is_empty() && request.rest.is_empty()) {
            return Err(
                "`--from` and `--compare` read reports and run nothing, so `-p` and arguments \
                 for nextest have no meaning with them"
                    .to_owned(),
            );
        }
        if os_given && !reads {
            return Err(
                "`--os` says where a report given with `--from` was made. A run made here is \
                 measured against this machine's budgets."
                    .to_owned(),
            );
        }
        Ok(request)
    }
}

/// `cargo xtask ci --tiers`: what each tier's tests take together, against
/// its budget, and which tests that time is made of.
///
/// It builds and runs as the blocking test job does, in cargo's `ci` profile,
/// where third-party crates are not optimised. A desk's own profile
/// optimises them, and a test that spends its time in one (a JSON reader, a
/// regular-expression engine) is several times slower in CI than it looks at
/// a desk. With `--from` it runs nothing and reports on a JUnit report that
/// is already there: the one a CI run uploads, for example.
fn ci_tiers(arguments: &[String]) -> Result<()> {
    let request = TiersRequest::parse(arguments)?;
    let (top, os) = (request.top, request.os);
    if let Some(run) = &request.factors_from {
        let Some(desk) = &request.from else {
            return Err(
                "`--factors-from` measures against the desk's report: give it with \
                        `--from`"
                    .to_owned(),
            );
        };
        let Source::Run(run) = run else {
            return Err("`--factors-from` takes a try run's id".to_owned());
        };
        let desk = crate::junit::read(desk)?;
        let side = |os| -> Result<Vec<crate::junit::TestCase>> {
            let sides = crate::records::ab_reports(*run, os)?
                .ok_or_else(|| format!("run {run} kept no report for {os:?}"))?;
            // The head: the tree whose job this `xtask` runs, in its own
            // layout. An A/A run's two sides are one tree.
            Ok(crate::ab::medians(&sides.head))
        };
        print!(
            "{}",
            crate::like_runner::measure(
                &desk,
                &side(tiers::Os::Linux)?,
                &side(tiers::Os::Windows)?
            )
        );
        return Ok(());
    }
    if let Some(report) = request.from {
        let checked = tiers::check_report(&report, os, top);
        if request.like_runner {
            print!(
                "{}",
                crate::like_runner::estimate(&crate::junit::read(&report)?)
            );
        }
        return checked;
    }
    if let (Some(new), Some(old)) = (&request.compare, &request.against) {
        let (label, cases) = runs::report(new, os)?;
        let new = compare::Side { label, cases };
        let (label, cases) = runs::report(old, os)?;
        let old = compare::Side { label, cases };
        print!("{}", compare::compare(&new, &old, os, top));
        return Ok(());
    }
    require_nextest()?;
    let mut rest = request.rest.clone();
    let threads_given = rest
        .iter()
        .any(|word| word.starts_with("--test-threads") || word == "-j");
    if request.like_runner && !threads_given {
        rest.splice(
            0..0,
            [
                "--test-threads".to_owned(),
                crate::like_runner::RUNNER_THREADS.to_owned(),
            ],
        );
    }
    let runs = ci_runs(&request.packages, &rest, true);
    let like_runner = request.like_runner;
    let mut build = ci_run(&request.packages);
    build.push("--no-run".to_owned());
    let title = if request.packages.is_empty() && request.rest.is_empty() {
        "Test time by tier, in CI's profile".to_owned()
    } else {
        let mut chosen = request.packages.clone();
        chosen.extend(request.rest.iter().cloned());
        format!(
            "Test time by tier, in CI's profile: {} only (the budgets are the whole workspace's)",
            chosen.join(" ")
        )
    };
    let mut all = vec![Step::closure("build tests", move || {
        forget_reports(&workspace::root())?;
        steps::cargo(&build)
    })];
    all.extend(run_steps(runs));
    all.push(Step::closure("tier budgets", move || {
        let checked = tiers::check_report(&tiers::ci_report_path(), os, top);
        if like_runner {
            let cases = crate::junit::read(&tiers::ci_report_path())?;
            print!("{}", crate::like_runner::estimate(&cases));
        }
        checked
    }));
    steps::run(&title, all)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        CHECK_STEPS, TiersRequest, check, check_step, ci, ci_run, selects_packages, test_command,
    };
    use crate::runs::Source;
    use crate::tiers::Os;

    fn strings(arguments: &[&str]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect()
    }

    #[test]
    fn every_listed_check_exists() {
        for name in CHECK_STEPS {
            assert!(check_step(name).is_some(), "{name}");
        }
    }

    #[test]
    fn an_unknown_check_is_refused_before_anything_runs() {
        let error = check(&["clipy".to_owned()]).unwrap_err();
        assert!(error.contains("unknown check `clipy`"));
        assert!(error.contains(&CHECK_STEPS.join(", ")));
    }

    #[test]
    fn naming_a_package_keeps_the_rest_of_the_workspace_out() {
        for chosen in [
            &["-p", "cortex-agent"][..],
            &["--package", "cortex-agent"],
            &["--package=cortex-agent"],
            &["-pcortex-agent"],
            &["--profile", "fast", "-p", "cortex-agent", "denials"],
            &["--workspace", "--exclude", "cortex-cli"],
        ] {
            assert!(selects_packages(&strings(chosen)), "{chosen:?}");
        }
    }

    #[test]
    fn a_run_of_one_crate_is_not_a_run_of_the_workspace() {
        assert_eq!(
            super::test_command(&strings(&["-p", "cortex-agent", "denials::"])).join(" "),
            "nextest run -p cortex-agent denials::"
        );
        assert_eq!(
            super::test_command(&strings(&["--profile", "fast"])).join(" "),
            "nextest run --workspace --profile fast"
        );
        assert_eq!(
            super::test_command(&[]).join(" "),
            "nextest run --workspace"
        );
    }

    #[test]
    fn without_a_package_the_whole_workspace_is_tested() {
        for open in [
            &[][..],
            &["--profile", "fast"],
            &["denials::"],
            &["--exclude", "cortex-cli"],
            &["--no-fail-fast", "-E", "test(/sys_/)"],
            // Arguments for the test binaries are not nextest's.
            &["--", "-p"],
            &["--", "--package"],
        ] {
            assert!(!selects_packages(&strings(open)), "{open:?}");
        }
    }

    /// The `backends` trap: a build of cortex-sandbox alone that tests
    /// nothing it confines.
    #[test]
    fn the_sandbox_alone_is_built_with_its_backends_on_linux() {
        let test = test_command(&strings(&["-p", "cortex-sandbox", "confined::"])).join(" ");
        let ci = ci_run(&strings(&["cortex-sandbox"])).join(" ");
        let other = test_command(&strings(&["-p", "cortex-agent"])).join(" ");
        let spelled = test_command(&strings(&["--package=cortex-sandbox"])).join(" ");
        if cfg!(target_os = "linux") {
            assert_eq!(
                test,
                "nextest run --features cortex-sandbox/backends -p cortex-sandbox confined::"
            );
            assert!(ci.contains("--features cortex-sandbox/backends"), "{ci}");
            assert!(
                spelled.contains("--features cortex-sandbox/backends"),
                "{spelled}"
            );
        } else {
            assert!(!test.contains("--features"), "{test}");
        }
        assert!(!other.contains("--features"), "{other}");
        // The whole workspace turns it on through cortex-core.
        assert!(!test_command(&[]).join(" ").contains("--features"));
    }

    #[test]
    fn the_job_runs_tiers_one_and_two_and_then_the_system_tests() {
        let runs = super::ci_runs(&[], &[], true);
        let base = "nextest run --workspace --locked --cargo-profile ci --profile ci";
        assert_eq!(
            runs.iter().map(|run| run.join(" ")).collect::<Vec<_>>(),
            [
                format!("{base} --no-tests=pass -E not test(/(^|::)sys_/)"),
                format!("{base} --no-tests=pass -E test(/(^|::)sys_/)"),
            ]
        );
        // A tree from before the split: one run, as its own job had it.
        assert_eq!(
            super::ci_runs(
                &strings(&["cortex-llm"]),
                &strings(&["--test-threads", "4"]),
                false
            ),
            [strings(&[
                "nextest",
                "run",
                "-p",
                "cortex-llm",
                "--locked",
                "--cargo-profile",
                "ci",
                "--profile",
                "ci",
                "--test-threads",
                "4"
            ])]
        );
    }

    #[test]
    fn a_filterset_given_narrows_each_run_and_is_not_added_beside_it() {
        // nextest runs a test that matches any of its filtersets: given
        // beside a run's own, `-E 'test(x)'` would run every test of the
        // other tier too.
        let rest = strings(&[
            "-E",
            "package(cortex-tui)",
            "--filterset=test(a)",
            "--test-threads",
            "4",
            "--",
            "-E",
            "not mine",
        ]);
        let runs = super::ci_runs(&[], &rest, true);
        let tail = |run: &Vec<String>| run[8..].to_vec();
        assert_eq!(
            tail(&runs[0]),
            strings(&[
                "--no-tests=pass",
                "-E",
                "((package(cortex-tui)) or (test(a))) and (not test(/(^|::)sys_/))",
                "--test-threads",
                "4",
                "--",
                "-E",
                "not mine",
            ])
        );
        assert_eq!(
            tail(&runs[1])[2],
            "((package(cortex-tui)) or (test(a))) and (test(/(^|::)sys_/))"
        );
        assert_eq!(
            super::filtersets(&strings(&["--filter-expr", "a"]))
                .0
                .as_deref(),
            Some("a")
        );
        assert_eq!(super::filtersets(&strings(&["x"])), (None, strings(&["x"])));
    }

    #[test]
    fn this_trees_job_is_two_runs_and_a_tree_without_the_group_is_one() {
        assert!(super::splits(&crate::workspace::root()));
        let root = std::env::temp_dir().join(format!("xtask-splits-{}", std::process::id()));
        let config = root.join(".config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("nextest.toml"),
            "[test-groups]\nprocess = { max-threads = 2 }\n# system = { max-threads = 3 }\n",
        )
        .unwrap();
        let before = super::splits(&root);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(!before);
    }

    #[test]
    fn the_first_runs_report_is_set_aside_and_merged_with_the_last() {
        let root = std::env::temp_dir().join(format!("xtask-reports-{}", std::process::id()));
        let report = crate::workspace::nextest_report(&root, "ci");
        std::fs::create_dir_all(report.parent().unwrap()).unwrap();
        let one = |name: &str| {
            format!(
                "<testsuites name=\"nextest-run\" tests=\"1\">\n<testsuite name=\"a\"><testcase \
                 name=\"{name}\" classname=\"a\" time=\"1\"/></testsuite>\n</testsuites>\n"
            )
        };
        let names = |path: &std::path::Path| -> Vec<String> {
            crate::junit::read(path)
                .unwrap()
                .into_iter()
                .map(|case| case.name)
                .collect()
        };
        // A report a run before this one left, which must not be merged.
        std::fs::write(super::aside_path(&root), one("stale")).unwrap();
        super::forget_reports(&root).unwrap();
        let stale_left = super::aside_path(&root).exists();
        std::fs::write(&report, one("first")).unwrap();
        super::set_aside(&root).unwrap();
        let moved = !report.exists();
        std::fs::write(&report, one("second")).unwrap();
        super::bring_together(&root).unwrap();
        let both = names(&report);
        let aside_left = super::aside_path(&root).exists();
        // A last run that wrote nothing leaves the first's report alone.
        std::fs::write(&report, one("only")).unwrap();
        super::set_aside(&root).unwrap();
        super::bring_together(&root).unwrap();
        let alone = names(&report);
        std::fs::remove_dir_all(&root).unwrap();

        assert!(
            !stale_left,
            "the build forgets a report set aside by an earlier run"
        );
        assert!(moved);
        assert_eq!(both, ["first", "second"]);
        assert!(!aside_left);
        assert_eq!(alone, ["only"]);
    }

    #[test]
    fn the_ci_run_is_the_workspace_or_the_packages_named() {
        assert_eq!(
            ci_run(&[]).join(" "),
            "nextest run --workspace --locked --cargo-profile ci --profile ci"
        );
        assert_eq!(
            ci_run(&strings(&["cortex-agent", "cortex-tools"])).join(" "),
            "nextest run -p cortex-agent -p cortex-tools --locked --cargo-profile ci --profile ci"
        );
    }

    #[test]
    fn a_tiers_request_is_read() {
        assert_eq!(
            TiersRequest::parse(&[]).unwrap(),
            TiersRequest {
                packages: vec![],
                top: 10,
                from: None,
                os: Os::here(),
                compare: None,
                against: None,
                rest: vec![],
                like_runner: false,
                factors_from: None,
            }
        );
        assert_eq!(
            TiersRequest::parse(&strings(&[
                "-p",
                "cortex-llm",
                "--top",
                "3",
                "--",
                "replay::",
                "--top",
            ]))
            .unwrap(),
            TiersRequest {
                packages: strings(&["cortex-llm"]),
                top: 3,
                from: None,
                os: Os::here(),
                compare: None,
                against: None,
                // After `--` nothing is this command's, whatever it looks like.
                rest: strings(&["replay::", "--top"]),
                like_runner: false,
                factors_from: None,
            }
        );
        assert_eq!(
            TiersRequest::parse(&strings(&["--from", "junit.xml", "--os", "windows"])).unwrap(),
            TiersRequest {
                packages: vec![],
                top: 10,
                from: Some(PathBuf::from("junit.xml")),
                os: Os::Windows,
                compare: None,
                against: None,
                rest: vec![],
                like_runner: false,
                factors_from: None,
            }
        );
        let like = TiersRequest::parse(&strings(&["--like-runner", "-p", "cortex-llm"])).unwrap();
        assert!(like.like_runner);
        assert_eq!(like.packages, strings(&["cortex-llm"]));
        let measure = TiersRequest::parse(&strings(&[
            "--from",
            "desk.xml",
            "--factors-from",
            "43663690282",
        ]))
        .unwrap();
        assert_eq!(measure.factors_from, Some(Source::Run(43663690282)));
    }

    #[test]
    fn a_comparison_is_against_the_main_branch_unless_told() {
        let request =
            TiersRequest::parse(&strings(&["--compare", "123", "--os", "windows"])).unwrap();
        assert_eq!(request.compare, Some(Source::Run(123)));
        assert_eq!(request.against, Some(Source::Main));
        assert_eq!(request.os, Os::Windows);
        let request =
            TiersRequest::parse(&strings(&["--compare", "a.xml", "--against", "b.xml"])).unwrap();
        assert_eq!(request.compare, Some(Source::File("a.xml".into())));
        assert_eq!(request.against, Some(Source::File("b.xml".into())));
    }

    #[test]
    fn a_tiers_request_that_makes_no_sense_is_refused() {
        for (arguments, expected) in [
            (&["--top"][..], "needs a number"),
            (&["--top", "many"], "is not a number"),
            (&["--os", "macos", "--from", "x"], "`linux` or `windows`"),
            (&["--os", "windows"], "`--os` says where a report"),
            (&["--from", "x", "-p", "cortex-llm"], "run nothing"),
            (&["--from", "x", "--", "replay::"], "run nothing"),
            (&["--compare", "x", "-p", "cortex-llm"], "run nothing"),
            (
                &["--against", "main"],
                "says what `--compare` is compared with",
            ),
            (&["--compare", "1", "--from", "x"], "give one"),
            (&["--compare"], "needs a report, a run or `main`"),
            (&["--fast"], "unknown argument `--fast`"),
        ] {
            let error = TiersRequest::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn an_unknown_ci_argument_is_refused_with_the_usage() {
        let error = ci(&strings(&["tests"])).unwrap_err();
        assert!(error.contains("ci --tiers"), "{error}");
    }
}
