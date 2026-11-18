//! The check and test commands. The blocking CI jobs run `check` and
//! `ci test`, so the commands here are the definition of what CI runs
//! (`docs/testing-and-ci.md`).

use std::fmt::Write as _;
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
/// cineris-agent` builds and runs that crate's tests and no other's.
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
                // `-pcineris-agent`. No other option of nextest starts `-p`.
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
    // The job's own runs run a cluster of timeouts again once when the disk
    // stalled during the run (`crate::stall::rerun`).
    all.extend(run_steps(ci_runs(&[], &[], true), true));
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
pub(crate) fn filtersets(arguments: &[String]) -> (Option<String>, Vec<String>) {
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
        // `-E=X`, `--filterset=X`, and the short form joined, `-EX`.
        let joined = NAMES
            .iter()
            .find_map(|name| word.strip_prefix(&format!("{name}=")))
            .or_else(|| {
                word.strip_prefix("-E")
                    .filter(|value| !value.is_empty() && !value.starts_with('='))
            });
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
///
/// With `rerun`, a run that fails with a cluster of timeouts while the disk
/// stalled runs the cluster's tests again, once, and what the pressure said
/// of each run is kept for the tier check (`crate::stall::rerun`). Without
/// it a failed run only says that it looks like a stall: `ci --tiers` is a
/// measurement, and a run of it that stalled is run again by a person.
fn run_steps(runs: Vec<Vec<String>>, rerun: bool) -> Vec<Step> {
    run_steps_with(runs, rerun, Vec::new())
}

/// [`run_steps`], with `variables` set for each run that is not run again
/// after a stall: `ci --tiers --like-runner`'s record of process starts.
fn run_steps_with(
    runs: Vec<Vec<String>>,
    rerun: bool,
    variables: Vec<(&'static str, String)>,
) -> Vec<Step> {
    if runs.len() == 1 && !rerun && variables.is_empty() {
        return runs
            .into_iter()
            .map(|run| Step::cargo_owned("run tests", run))
            .collect();
    }
    let names = ["run tiers 1 and 2", "run the system tests"];
    let last = runs.len() - 1;
    let single = runs.len() == 1;
    runs.into_iter()
        .enumerate()
        .map(|(index, run)| {
            let name = if single {
                "run tests"
            } else {
                names[index.min(1)]
            };
            let variables = variables.clone();
            Step::closure(name, move || {
                let root = workspace::root();
                let variables: Vec<(&str, &str)> = variables
                    .iter()
                    .map(|(name, value)| (*name, value.as_str()))
                    .collect();
                let tiers = if single {
                    crate::stall::EVERY_TIER
                } else if index == 0 {
                    crate::stall::UNIT_AND_INTEGRATION
                } else {
                    crate::stall::SYSTEM
                };
                let ran = if rerun {
                    run_with_rerun(&run, &root, tiers)
                } else {
                    let watch = crate::stall::Watch::start();
                    let ran = steps::cargo_with(&run, &variables);
                    if ran.is_err() {
                        say_if_stalled(&watch, &workspace::nextest_report(&root, "ci"));
                    }
                    ran
                };
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

/// A run of the job's tests in the tree at `root`, run again when
/// `crate::stall::rerun` says so. What the pressure said of each run is
/// added to the records the tier check reads, and a re-run is on the job's
/// summary page.
fn run_with_rerun(run: &[String], root: &Path, tiers: u8) -> Result<()> {
    use crate::stall::rerun;
    // How each run ended, by nextest's exit status: a known flake is run
    // again only when every run so far failed by its tests failing.
    let codes = std::cell::RefCell::new(Vec::new());
    let mut exec = |arguments: &[String]| {
        let watch = crate::stall::Watch::start();
        let (result, code) = steps::cargo_code(arguments);
        codes.borrow_mut().push(code);
        rerun::Attempt {
            result,
            observed: watch.observe(),
        }
    };
    let outcome = rerun::run(
        run,
        &workspace::nextest_report(root, "ci"),
        &mut exec,
        &mut |line| println!("{line}"),
    );
    let records = crate::stall::record_path(root);
    for record in &outcome.records {
        // A record is a convenience of the tier check: failing to write one
        // leaves the tiers judged, as they were without the pressure.
        let record = crate::stall::Record { tiers, ..*record };
        if let Err(error) = crate::stall::append_record(&records, &record) {
            println!("warning: {error}");
        }
    }
    if let Some(summary) = &outcome.summary {
        steps::append_job_summary(&format!("{summary}\n"));
    }
    let Err(error) = outcome.result else {
        return Ok(());
    };
    // A known flake among what still failed is named, and, away from CI,
    // run again once (`crate::flaky`).
    let (listed, warning) = crate::flaky::listed(root);
    if let Some(warning) = warning {
        println!("warning: {warning}");
    }
    let refusal = crate::flaky::refusal_in_ci(std::env::var_os("GITHUB_ACTIONS").as_deref())
        .or_else(|| crate::flaky::refusal_by_codes(&codes.borrow()));
    let flakes = crate::flaky::after_failure(
        run,
        &workspace::nextest_report(root, "ci"),
        crate::flaky::Before {
            error,
            refusal,
            already: &outcome.ran_again,
        },
        &listed,
        &mut exec,
        &mut |line| println!("{line}"),
    );
    if let Some(summary) = &flakes.summary {
        steps::append_job_summary(&format!("{summary}\n"));
    }
    flakes.result
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
    workspace::forget_report(&aside_path(root))?;
    // What the pressure said of the runs that wrote them.
    workspace::forget_report(&crate::stall::record_path(root))
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

/// Whether `cargo xtask ci` would take `arguments`, which it reads and runs
/// nothing: how `try` refuses what it cannot pass on before it builds.
pub(crate) fn check_ci_arguments(arguments: &[String]) -> Result<()> {
    if arguments.first().is_some_and(|first| first == "--tiers") {
        return TiersRequest::parse(&arguments[1..]).map(|_| ());
    }
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        [] | ["lint" | "test"] => Ok(()),
        other => Err(format!(
            "`ci {}` is not a form of `ci`: `ci`, `ci lint`, `ci test` or `ci --tiers …`",
            other.join(" ")
        )),
    }
}

const CI_USAGE: &str = "usage: cargo xtask ci [lint|test]
       cargo xtask ci --tiers [-p CRATE]... [--top N] [-- NEXTEST ARGUMENT...]
       cargo xtask ci --tiers --like-runner [--base DIR] [-p CRATE]... [--top N]
                              DIR: another checkout of the workspace (`main`'s), run and
                              estimated too, and compared with this one
       cargo xtask ci --tiers --from JUNIT.XML [--os linux|windows] [--top N]
       cargo xtask ci --tiers --compare NEW [--against OLD] [--os linux|windows|both] [--top N]
                              NEW and OLD: a JUnit report, a run's id (of the `ci` workflow, or a
                              `try` run of `test`), or `main` (OLD's default)";

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
    /// `--os both`: with `--compare`, each OS's comparison in turn, Linux's
    /// then Windows', from runs that kept both reports.
    both: bool,
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
    /// The starts the desk's run of `from` wrote down: a directory.
    starts_from: Option<PathBuf>,
    /// With `like_runner`: a tree to run as well, at this desk, and to
    /// compare this one's estimate with.
    base: Option<PathBuf>,
}

impl TiersRequest {
    /// The OSes a comparison is of: both with `--os both`, else the one.
    fn oses(&self) -> Vec<tiers::Os> {
        if self.both {
            vec![tiers::Os::Linux, tiers::Os::Windows]
        } else {
            vec![self.os]
        }
    }

    fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Self {
            packages: Vec::new(),
            top: TIERS_TOP,
            from: None,
            os: tiers::Os::here(),
            both: false,
            compare: None,
            against: None,
            rest: Vec::new(),
            like_runner: false,
            factors_from: None,
            starts_from: None,
            base: None,
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
                "--starts-from" => {
                    request.starts_from = Some(PathBuf::from(value("a directory of starts")?));
                }
                "--base" => request.base = Some(PathBuf::from(value("a directory")?)),
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
                        "both" => {
                            request.both = true;
                            tiers::Os::here()
                        }
                        other => {
                            return Err(format!(
                                "`--os {other}`: say `linux` or `windows`, or `both` with \
                                 `--compare`"
                            ));
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
        if request.both {
            if request.compare.is_none() {
                return Err(
                    "`--os both` compares two runs' reports of each OS: it goes with `--compare`"
                        .to_owned(),
                );
            }
            for source in [&request.compare, &request.against].into_iter().flatten() {
                if matches!(source, Source::File(_)) {
                    return Err(
                        "`--os both` reads both OSes' reports of a run: a report on disk is of \
                         one OS, so name a run or `main`, or compare the reports one `--os` at \
                         a time"
                            .to_owned(),
                    );
                }
            }
        }
        if request.base.is_some() && (!request.like_runner || reads) {
            return Err(
                "`--base` is a tree to run beside this one and compare with: it goes with \
                 `--like-runner`, and not with `--from` or `--compare`"
                    .to_owned(),
            );
        }
        if request.starts_from.is_some() && !request.like_runner && request.factors_from.is_none() {
            return Err(
                "`--starts-from` is read by `--like-runner` or `--factors-from`: alone it \
                 would change nothing"
                    .to_owned(),
            );
        }
        if request.starts_from.is_some() && request.from.is_none() {
            return Err(
                "`--starts-from` names the starts a report given with `--from` wrote down"
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

/// `new` compared with `old` for each OS of `oses` in turn, each against its
/// own OS's budgets. A source is chosen once for all the OSes (`main` is
/// one run, which kept every report asked for).
fn compare_reports(new: &Source, old: &Source, oses: &[tiers::Os], top: usize) -> Result<()> {
    let (new, old) = (runs::resolve(new, oses)?, runs::resolve(old, oses)?);
    let results: Vec<(tiers::Os, Result<String>)> = oses
        .iter()
        .map(|os| {
            let compared = (|| {
                let (label, cases) = runs::report_of(&new, *os)?;
                let new = compare::Side { label, cases };
                let (label, cases) = runs::report_of(&old, *os)?;
                let old = compare::Side { label, cases };
                let table = compare::compare(&new, &old, *os, top);
                let problems = compare::problems(&new, &old);
                if problems.is_empty() {
                    Ok(table)
                } else {
                    // The table is printed all the same, and the command
                    // fails: a side that failed or is missing a tier is no
                    // measurement.
                    print!("{table}");
                    Err(format!(
                        "not a comparison of the tests' times: {}",
                        problems.join("; ")
                    ))
                }
            })();
            (*os, compared)
        })
        .collect();
    let (text, outcome) = join_comparisons(results);
    print!("{text}");
    outcome
}

/// What each OS's comparison printed, under a heading when there are
/// several, and how the command ends. One OS whose report cannot be read
/// does not hide another's comparison: its reason is printed in its
/// place, and the command fails after the last. With one OS the error is
/// the error.
fn join_comparisons(results: Vec<(tiers::Os, Result<String>)>) -> (String, Result<()>) {
    let several = results.len() > 1;
    let mut text = String::new();
    let mut failed = Vec::new();
    for (index, (os, compared)) in results.into_iter().enumerate() {
        if several {
            let _ = writeln!(text, "{}== {os:?} ==\n", if index > 0 { "\n" } else { "" });
        }
        match compared {
            Ok(table) => text.push_str(&table),
            Err(error) if !several => return (text, Err(error)),
            Err(error) => {
                let _ = writeln!(text, "  not compared: {error}");
                failed.push(format!("{os:?}: {error}"));
            }
        }
    }
    let outcome = if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    };
    (text, outcome)
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
        if let Some(directory) = &request.starts_from {
            // The work factors, starts taken out: a runner's plain run with
            // its starts written down, of the tree the desk ran.
            let desk_starts = crate::starts::Starts::read(directory)?;
            let windows = crate::like_runner::runner_report(*run)?;
            print!(
                "{}",
                crate::like_runner::measure_work(&desk, &desk_starts, &windows)
            );
            return Ok(());
        }
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
            let cases = crate::junit::read(&report)?;
            print!("{}", crate::like_runner::estimate(&cases));
            if let Some(directory) = &request.starts_from {
                let starts = crate::starts::Starts::read(directory)?;
                print!("{}", crate::like_runner::starts_text(&cases, &starts));
            }
        }
        return checked;
    }
    if let (Some(new), Some(old)) = (&request.compare, &request.against) {
        return compare_reports(new, old, &request.oses(), top);
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
    // With `--like-runner` each test's process starts are written down, in
    // a directory emptied first.
    let starts = workspace::root()
        .join("target")
        .join("like-runner")
        .join("starts");
    let variables = if like_runner {
        vec![(
            crate::starts::VARIABLE,
            starts.to_string_lossy().into_owned(),
        )]
    } else {
        Vec::new()
    };
    let base = match &request.base {
        Some(base) => Some(crate::like_runner::base_tree(base)?),
        None => None,
    };
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
    let starts_to_clear = starts.clone();
    let mut all = vec![Step::closure("build tests", move || {
        forget_reports(&workspace::root())?;
        if like_runner {
            workspace::forget_dir(&starts_to_clear)?;
        }
        steps::cargo(&build)
    })];
    all.extend(run_steps_with(runs, false, variables));
    if let Some(base) = &base {
        let (base, packages, rest) = (base.clone(), request.packages.clone(), rest.clone());
        all.push(Step::closure("the base's tests", move || {
            crate::like_runner::run_base(&base, &packages, &rest)
        }));
    }
    all.push(Step::closure("tier budgets", move || {
        let checked = tiers::check_report(&tiers::ci_report_path(), os, top);
        if like_runner {
            let cases = crate::junit::read(&tiers::ci_report_path())?;
            let here = crate::starts::Starts::read(&starts)?;
            print!("{}", crate::like_runner::estimate(&cases));
            print!("{}", crate::like_runner::starts_text(&cases, &here));
            if let Some(base) = &base {
                let (text, warned) =
                    crate::like_runner::compare_with_base(&cases, &here, base, top)?;
                print!("{text}");
                if warned && std::env::var_os("GITHUB_ACTIONS").is_some() {
                    println!("::warning::the estimate of a Windows tier grew past its warning");
                }
            }
        }
        checked
    }));
    steps::run(&title, all)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        CHECK_STEPS, TiersRequest, check, check_step, ci, ci_run, join_comparisons,
        selects_packages, test_command,
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
            &["-p", "cineris-agent"][..],
            &["--package", "cineris-agent"],
            &["--package=cineris-agent"],
            &["-pcineris-agent"],
            &["--profile", "fast", "-p", "cineris-agent", "denials"],
            &["--workspace", "--exclude", "cineris-cli"],
        ] {
            assert!(selects_packages(&strings(chosen)), "{chosen:?}");
        }
    }

    #[test]
    fn a_run_of_one_crate_is_not_a_run_of_the_workspace() {
        assert_eq!(
            super::test_command(&strings(&["-p", "cineris-agent", "denials::"])).join(" "),
            "nextest run -p cineris-agent denials::"
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
            &["--exclude", "cineris-cli"],
            &["--no-fail-fast", "-E", "test(/sys_/)"],
            // Arguments for the test binaries are not nextest's.
            &["--", "-p"],
            &["--", "--package"],
        ] {
            assert!(!selects_packages(&strings(open)), "{open:?}");
        }
    }

    /// The `backends` trap: a build of cineris-sandbox alone that tests
    /// nothing it confines.
    #[test]
    fn the_sandbox_alone_is_built_with_its_backends_on_linux() {
        let test = test_command(&strings(&["-p", "cineris-sandbox", "confined::"])).join(" ");
        let ci = ci_run(&strings(&["cineris-sandbox"])).join(" ");
        let other = test_command(&strings(&["-p", "cineris-agent"])).join(" ");
        let spelled = test_command(&strings(&["--package=cineris-sandbox"])).join(" ");
        if cfg!(target_os = "linux") {
            assert_eq!(
                test,
                "nextest run --features cineris-sandbox/backends -p cineris-sandbox confined::"
            );
            assert!(ci.contains("--features cineris-sandbox/backends"), "{ci}");
            assert!(
                spelled.contains("--features cineris-sandbox/backends"),
                "{spelled}"
            );
        } else {
            assert!(!test.contains("--features"), "{test}");
        }
        assert!(!other.contains("--features"), "{other}");
        // The whole workspace turns it on through cineris-core.
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
                &strings(&["cineris-llm"]),
                &strings(&["--test-threads", "4"]),
                false
            ),
            [strings(&[
                "nextest",
                "run",
                "-p",
                "cineris-llm",
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
            "package(cineris-tui)",
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
                "((package(cineris-tui)) or (test(a))) and (not test(/(^|::)sys_/))",
                "--test-threads",
                "4",
                "--",
                "-E",
                "not mine",
            ])
        );
        assert_eq!(
            tail(&runs[1])[2],
            "((package(cineris-tui)) or (test(a))) and (test(/(^|::)sys_/))"
        );
        assert_eq!(
            super::filtersets(&strings(&["--filter-expr", "a"]))
                .0
                .as_deref(),
            Some("a")
        );
        assert_eq!(super::filtersets(&strings(&["x"])), (None, strings(&["x"])));
        // The short form joined to its value, as nextest takes it: a filter
        // left beside one that names a test exactly would run more.
        assert_eq!(
            super::filtersets(&strings(&["-Etest(/x/)", "-E=b", "y"])),
            (Some("(test(/x/)) or (b)".to_owned()), strings(&["y"]))
        );
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
    fn what_the_pressure_said_of_an_earlier_run_is_forgotten_with_its_reports() {
        let root = std::env::temp_dir().join(format!("xtask-records-gone-{}", std::process::id()));
        let records = crate::stall::record_path(&root);
        std::fs::create_dir_all(records.parent().unwrap()).unwrap();
        std::fs::write(&records, "elapsed_ms=100000 full_us=90000000\n").unwrap();
        let before = crate::stall::read_records(&records).len();
        super::forget_reports(&root).unwrap();
        let after = crate::stall::read_records(&records).len();
        std::fs::remove_dir_all(&root).unwrap();
        // A stale record would keep the next run's tiers from being judged.
        assert_eq!((before, after), (1, 0));
    }

    #[test]
    fn the_ci_run_is_the_workspace_or_the_packages_named() {
        assert_eq!(
            ci_run(&[]).join(" "),
            "nextest run --workspace --locked --cargo-profile ci --profile ci"
        );
        assert_eq!(
            ci_run(&strings(&["cineris-agent", "cineris-tools"])).join(" "),
            "nextest run -p cineris-agent -p cineris-tools --locked --cargo-profile ci --profile ci"
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
                both: false,
                compare: None,
                against: None,
                rest: vec![],
                like_runner: false,
                factors_from: None,
                starts_from: None,
                base: None,
            }
        );
        assert_eq!(
            TiersRequest::parse(&strings(&[
                "-p",
                "cineris-llm",
                "--top",
                "3",
                "--",
                "replay::",
                "--top",
            ]))
            .unwrap(),
            TiersRequest {
                packages: strings(&["cineris-llm"]),
                top: 3,
                from: None,
                os: Os::here(),
                both: false,
                compare: None,
                against: None,
                // After `--` nothing is this command's, whatever it looks like.
                rest: strings(&["replay::", "--top"]),
                like_runner: false,
                factors_from: None,
                starts_from: None,
                base: None,
            }
        );
        assert_eq!(
            TiersRequest::parse(&strings(&["--from", "junit.xml", "--os", "windows"])).unwrap(),
            TiersRequest {
                packages: vec![],
                top: 10,
                from: Some(PathBuf::from("junit.xml")),
                os: Os::Windows,
                both: false,
                compare: None,
                against: None,
                rest: vec![],
                like_runner: false,
                factors_from: None,
                starts_from: None,
                base: None,
            }
        );
        let like = TiersRequest::parse(&strings(&["--like-runner", "-p", "cineris-llm"])).unwrap();
        assert!(like.like_runner);
        // A base goes with the estimate, and the starts with what reads them.
        let based = TiersRequest::parse(&strings(&["--like-runner", "--base", "../b"])).unwrap();
        assert_eq!(based.base, Some(PathBuf::from("../b")));
        for (words, said) in [
            (&["--base", "../b"][..], "goes with `--like-runner`"),
            (
                &["--like-runner", "--base", "../b", "--from", "d.xml"],
                "not with `--from`",
            ),
            (&["--from", "d.xml", "--starts-from", "s"], "alone it"),
            (&["--like-runner", "--starts-from", "s"], "with `--from`"),
        ] {
            let error = TiersRequest::parse(&strings(words)).unwrap_err();
            assert!(error.contains(said), "{words:?}: {error}");
        }
        assert_eq!(like.packages, strings(&["cineris-llm"]));
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
    fn each_oses_comparison_is_printed_in_turn_and_one_failing_hides_no_other() {
        // One OS: the table as it is, and its error is the error.
        let (text, outcome) = join_comparisons(vec![(Os::Linux, Ok("table\n".to_owned()))]);
        assert_eq!((text.as_str(), outcome), ("table\n", Ok(())));
        let (text, outcome) = join_comparisons(vec![(Os::Windows, Err("no report".to_owned()))]);
        assert_eq!((text.as_str(), outcome), ("", Err("no report".to_owned())));
        // Both: each under its name, Linux first.
        let (text, outcome) = join_comparisons(vec![
            (Os::Linux, Ok("linux table\n".to_owned())),
            (Os::Windows, Ok("windows table\n".to_owned())),
        ]);
        assert_eq!(
            text,
            "== Linux ==\n\nlinux table\n\n== Windows ==\n\nwindows table\n"
        );
        assert_eq!(outcome, Ok(()));
        // Windows' report missing: Linux's table is there, Windows says why,
        // and the command fails naming it.
        let (text, outcome) = join_comparisons(vec![
            (Os::Linux, Ok("linux table\n".to_owned())),
            (Os::Windows, Err("run 5 kept no `junit-windows`".to_owned())),
        ]);
        assert!(text.contains("linux table"), "{text}");
        assert!(
            text.contains("== Windows ==\n\n  not compared: run 5 kept no `junit-windows`"),
            "{text}"
        );
        let error = outcome.unwrap_err();
        assert!(error.starts_with("Windows: run 5 kept no"), "{error}");
    }

    #[test]
    fn both_oses_are_compared_from_runs_and_never_from_one_file() {
        let request = TiersRequest::parse(&strings(&["--compare", "123", "--os", "both"])).unwrap();
        assert!(request.both);
        assert_eq!(request.oses(), [Os::Linux, Os::Windows]);
        assert_eq!(request.against, Some(Source::Main));
        let request = TiersRequest::parse(&strings(&[
            "--compare",
            "123",
            "--against",
            "456",
            "--os",
            "windows",
        ]))
        .unwrap();
        assert!(!request.both);
        assert_eq!(request.os, Os::Windows);
        assert_eq!(request.oses(), [Os::Windows]);
        for (arguments, expected) in [
            (
                &["--os", "both", "--from", "x.xml"][..],
                "goes with `--compare`",
            ),
            (
                &["--compare", "x.xml", "--os", "both"],
                "name a run or `main`",
            ),
            (
                &["--compare", "1", "--against", "b.xml", "--os", "both"],
                "name a run or `main`",
            ),
        ] {
            let error = TiersRequest::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn a_tiers_request_that_makes_no_sense_is_refused() {
        for (arguments, expected) in [
            (&["--top"][..], "needs a number"),
            (&["--top", "many"], "is not a number"),
            (&["--os", "macos", "--from", "x"], "`linux` or `windows`"),
            (&["--os", "windows"], "`--os` says where a report"),
            (&["--from", "x", "-p", "cineris-llm"], "run nothing"),
            (&["--from", "x", "--", "replay::"], "run nothing"),
            (&["--compare", "x", "-p", "cineris-llm"], "run nothing"),
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
