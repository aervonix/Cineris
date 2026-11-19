//! What a hosted runner would report for the tests run at a desk
//! (`docs/testing-and-ci.md`, "What a runner would say, from a desk").
//!
//! `cargo xtask ci --tiers --like-runner` runs the tests as the blocking job
//! does, in CI's profile and at a runner's four test threads, and then
//! scales each crate's time in each tier by a factor measured for that crate
//! and tier on each OS: the same tree run at a desk and on both runners.
//! The factors differ because the work does: code of our own that is
//! unoptimised in CI's build, generics of an optimised crate included, is
//! two to four times as slow on a runner; work that waits on the disk can be
//! slower at the desk than on a Linux runner; and Windows-only work (a
//! PowerShell test, a file's access list) is not done at a Linux desk at
//! all, so its crate's factor is large. A crate the table does not know
//! takes its tier's factor.
//!
//! Hosted runners differ in speed, and the factors are of one runner of
//! each OS. The estimate is given for that runner and for a slow one, at a
//! measured ratio between two runners of each OS.
//!
//! `cargo xtask ci --tiers --from DESK.xml --factors-from TRY-RUN` measures
//! the table again: `DESK.xml` is the desk's report of a run with
//! `--like-runner` (or `-- --test-threads 4`), and `TRY-RUN` a `try` run
//! of the same tree with `-f against=…` (an A/A run, or any A/B, whose head
//! side, this tree, is read), whose rounds give each test's median on each
//! runner. It prints the table to paste here.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::junit::TestCase;
use crate::starts::{Counts, Kind, Starts};
use crate::tiers::{self, Os, Tier};

/// The test threads of a hosted runner: both OSes report 4 cores.
pub(crate) const RUNNER_THREADS: &str = "4";

/// Each crate's factor in each tier, Linux and Windows: a runner's time
/// above the start of a test's process over the desk's, the same tree on
/// both, the job split into its two runs, with Windows' system tests two at
/// a time. Measured on 2026-11-15 in the sixth tooling step, from the
/// desk's run of the step's branch at `5e2866b` at four test threads
/// (`rx --exclusive`, though the desk's disk stalled 54% of the run, which
/// makes its file-heavy crates slow and their factors low) against the
/// branch's side of `try` run 43718003767 (an A/B against `main` at
/// `c1e9ea3`, five rounds, start 4 ms on Linux and 6 ms on Windows). Only a
/// crate and tier with at least 0.05 s at the desk is listed. The fifth
/// tooling step's table, of three system tests at a time, read Windows'
/// system tier 84 s where runners read 39 to 71.
const FACTORS: &[(&str, Tier, f64, f64)] = &[
    ("cineris-agent", Tier::Integration, 1.56, 3.04),
    ("cineris-base", Tier::Unit, 1.73, 3.62),
    ("cineris-base", Tier::Integration, 0.75, 2.39),
    ("cineris-cli", Tier::System, 0.96, 2.84),
    ("cineris-config", Tier::Integration, 0.22, 0.78),
    ("cineris-core", Tier::Unit, 0.52, 1.41),
    ("cineris-core", Tier::Integration, 0.67, 1.28),
    ("cineris-core", Tier::System, 1.17, 5.39),
    ("cineris-index", Tier::Unit, 1.67, 2.45),
    ("cineris-index", Tier::Integration, 1.76, 2.83),
    ("cineris-llm", Tier::Unit, 1.42, 3.28),
    ("cineris-llm", Tier::Integration, 1.45, 2.17),
    ("cineris-llm", Tier::System, 1.68, 0.32),
    ("cineris-policy", Tier::Unit, 1.34, 2.22),
    ("cineris-policy", Tier::Integration, 0.80, 1.57),
    ("cineris-policy", Tier::System, 1.12, 4.15),
    ("cineris-sandbox", Tier::System, 4.89, 3.33),
    ("cineris-session", Tier::Unit, 1.03, 2.35),
    ("cineris-session", Tier::Integration, 0.80, 1.51),
    ("cineris-session", Tier::System, 5.09, 26.69),
    ("cineris-stats", Tier::Unit, 1.56, 2.44),
    ("cineris-term", Tier::Unit, 1.52, 2.01),
    ("cineris-term", Tier::Integration, 1.50, 1.99),
    ("cineris-term", Tier::System, 1.18, 4.72),
    ("cineris-tools", Tier::Unit, 0.99, 2.43),
    ("cineris-tools", Tier::Integration, 1.64, 4.35),
    ("cineris-tools", Tier::System, 2.01, 31.55),
    ("cineris-tui", Tier::Unit, 1.57, 2.03),
    ("cineris-tui", Tier::Integration, 1.57, 2.10),
    ("cineris-tui", Tier::System, 3.35, 12.58),
    ("xtask", Tier::Unit, 1.54, 4.43),
    ("xtask", Tier::System, 1.33, 4.85),
];

/// Each tier's factor, Linux and Windows, for a crate the table does not
/// know: the tier's whole time on the runner over the desk's, in the same
/// measurement.
const TIER_FACTORS: [(Tier, f64, f64); 3] = [
    (Tier::Unit, 1.43, 2.61),
    (Tier::Integration, 0.96, 1.80),
    (Tier::System, 1.42, 6.44),
];

/// How much slower than the measured runner a slow one was, Linux and
/// Windows: tier 2 of the same tree in its pull request's CI run
/// (43691685559, start 4 and 10 ms: 14.0 and 30.2 s), over the measured
/// runner's (9.7 and 24.8 s).
const SLOW_RUNNER: (f64, f64) = (1.44, 1.22);

fn factor(krate: &str, tier: Tier, os: Os) -> f64 {
    let pick = |linux: f64, windows: f64| match os {
        Os::Linux => linux,
        Os::Windows => windows,
    };
    FACTORS
        .iter()
        .find(|(name, of, ..)| *name == krate && *of == tier)
        .map(|(_, _, linux, windows)| pick(*linux, *windows))
        .or_else(|| {
            TIER_FACTORS
                .iter()
                .find(|(of, ..)| *of == tier)
                .map(|(_, linux, windows)| pick(*linux, *windows))
        })
        .unwrap_or(1.0)
}

/// Each crate's time above the start in each tier.
fn by_crate_and_tier(cases: &[TestCase]) -> BTreeMap<(String, Tier), f64> {
    let (start, _) = tiers::start_of(cases);
    let mut sums: BTreeMap<(String, Tier), f64> = BTreeMap::new();
    for case in cases {
        let key = (
            tiers::crate_of(&case.binary).to_owned(),
            Tier::of(&case.binary, &case.name),
        );
        *sums.entry(key).or_default() += (case.seconds - start).max(0.0);
    }
    sums
}

/// What a runner of each OS would report for the desk's `cases`: each
/// tier's estimate on the measured runner and on a slow one, against the
/// budget, and the crates that make the most of it.
pub(crate) fn estimate(cases: &[TestCase]) -> String {
    let desk = by_crate_and_tier(cases);
    let mut text = String::from(
        "\nWhat a hosted runner would report, estimated from this desk's run (each crate's time \
         in each tier times a factor measured for it, docs/testing-and-ci.md):\n",
    );
    for os in [Os::Linux, Os::Windows] {
        let slow = match os {
            Os::Linux => SLOW_RUNNER.0,
            Os::Windows => SLOW_RUNNER.1,
        };
        let _ = writeln!(
            text,
            "\n  {}            desk   runner   a slow runner   budget",
            match os {
                Os::Linux => "Linux  ",
                Os::Windows => "Windows",
            }
        );
        for tier in Tier::ALL {
            let mut contributions: Vec<(String, f64)> = desk
                .iter()
                .filter(|((_, of), _)| *of == tier)
                .map(|((krate, _), seconds)| (krate.clone(), seconds * factor(krate, tier, os)))
                .collect();
            let at_desk: f64 = desk
                .iter()
                .filter(|((_, of), _)| *of == tier)
                .map(|(_, seconds)| seconds)
                .sum();
            let estimated: f64 = contributions.iter().map(|(_, seconds)| seconds).sum();
            let budget = tier.budget(os);
            let warn = if estimated * slow > budget {
                "  over on a slow runner"
            } else {
                ""
            };
            let _ = writeln!(
                text,
                "  {:<16} {at_desk:>6.1} s {estimated:>6.1} s {:>12.1} s   {budget:>4.0} s{warn}",
                tier.name(),
                estimated * slow
            );
            contributions.sort_by(|a, b| b.1.total_cmp(&a.1));
            let most: Vec<String> = contributions
                .iter()
                .take(3)
                .filter(|(_, seconds)| *seconds >= 0.05)
                .map(|(krate, seconds)| format!("{krate} {seconds:.1} s"))
                .collect();
            if !most.is_empty() {
                let _ = writeln!(text, "      most: {}", most.join(", "));
            }
        }
    }
    text.push_str(
        "\n  The runner columns are an estimate, and say most about a change to crates the \
         factors know: a runner's speed varies, and Windows-only work is not seen at a Linux \
         desk. Measure a change on the runner with `try -f against=main`.\n",
    );
    text
}

/// What one start of each kind costs, in seconds: at a Linux desk, and on
/// the measured Windows runner. A desk's start is charged on Windows as the
/// kind it stands for there ([`on_windows`]).
///
/// The Windows figures are fitted to `try` run 43892837490 (a 9 ms runner;
/// the ninth tooling step's tree, its system tests in two lanes and every
/// test's starts written down): each system test's time above the start of
/// its process, against its starts of each kind and its crate, by least
/// squares over its 176 tests. On `main` before that step, with two system
/// tests at a time anywhere, a PowerShell 7 start fitted at 0.44 s and
/// Windows PowerShell at 0.31 s (`try` 43883759862). The desk's are fitted
/// the same way to the desk's run of the same tree (`rx --exclusive`).
const START_COSTS: [(Kind, f64, f64); 8] = [
    (Kind::Pwsh, 0.050, 0.331),
    (Kind::WindowsPowerShell, 0.050, 0.276),
    (Kind::Posix, 0.002, 0.276),
    (Kind::Cmd, 0.002, 0.026),
    (Kind::Git, 0.002, 0.037),
    (Kind::Binary, 0.004, 0.010),
    (Kind::Other, 0.004, 0.151),
    // The harness asks bash its version at a Linux desk, and asks the
    // tests' shell on Windows, Windows PowerShell, nothing.
    (Kind::Probe, 0.002, 0.0),
];

/// The kind a start at the desk stands for on Windows: a POSIX shell is the
/// tests' shell there, Windows PowerShell (the shell of every test that is
/// not about which shell runs its command). The others are themselves.
fn on_windows(kind: Kind) -> Kind {
    match kind {
        Kind::Posix => Kind::WindowsPowerShell,
        other => other,
    }
}

fn cost(kind: Kind, os: Os) -> f64 {
    START_COSTS
        .iter()
        .find(|(of, ..)| *of == kind)
        .map_or(0.0, |(_, desk, windows)| match os {
            Os::Linux => *desk,
            Os::Windows => *windows,
        })
}

/// What `counts`' starts cost at a desk.
fn at_desk(counts: &Counts) -> f64 {
    Kind::ALL
        .iter()
        .map(|kind| counts.of(*kind) as f64 * cost(*kind, Os::Linux))
        .sum()
}

/// What `counts`' starts, made at a desk, would cost on the Windows runner.
fn on_the_runner(counts: &Counts) -> f64 {
    Kind::ALL
        .iter()
        .map(|kind| counts.of(*kind) as f64 * cost(on_windows(*kind), Os::Windows))
        .sum()
}

/// Each crate's factor for the work of its tests that is not starting a
/// process, on Windows: the runner's time above the start of a test's
/// process, less what its starts cost there, over the desk's, less what
/// they cost at the desk. A test that runs only on the runner (a
/// PowerShell test) is counted whole on the runner's side, so that its
/// crate's factor carries it. Measured with `--factors-from` and
/// `--starts-from`, as [`FACTORS`] is; a crate the table does not know
/// takes [`FACTORS`]' factor.
///
/// Measured in the ninth tooling step for the system tier only, where the
/// starts are: the desk's run of the step's tree (`rx --exclusive`; its
/// integration tier stalled on the desk's disk, and is not read) against a
/// 9 ms runner's run of the same tree with its starts written down (`try`
/// 43892837490). Tiers 1 and 2 start no process, and their factors are
/// [`FACTORS`]'. `cineris-tools`' and `xtask`'s are large because
/// Windows-only tests (each shell's own) and the starts no mark covers (the
/// owner guard's tests' own git) run only on the runner.
const WORK_FACTORS: &[(&str, Tier, f64)] = &[
    ("cineris-cli", Tier::System, 0.53),
    ("cineris-core", Tier::System, 0.00),
    ("cineris-llm", Tier::System, 0.53),
    ("cineris-policy", Tier::System, 6.46),
    ("cineris-sandbox", Tier::System, 0.00),
    ("cineris-term", Tier::System, 0.00),
    ("cineris-tools", Tier::System, 17.37),
    ("xtask", Tier::System, 13.09),
];

/// A crate's work factor: [`WORK_FACTORS`]' where it has one, [`FACTORS`]'
/// otherwise, and never under 1: a hosted runner is not quicker than the
/// desk at a test's work, and a factor of 0 (a crate whose Linux-only tests
/// start more than its Windows run costs) would hide every change to its
/// tests' work from a comparison.
const LEAST_WORK_FACTOR: f64 = 1.0;

fn work_factor(krate: &str, tier: Tier) -> f64 {
    WORK_FACTORS
        .iter()
        .find(|(name, of, _)| *name == krate && *of == tier)
        .map_or_else(|| factor(krate, tier, Os::Windows), |(.., value)| *value)
        .max(LEAST_WORK_FACTOR)
}

/// One test's time on the Windows runner, estimated from its time at the
/// desk above `start` and the processes it started: its work (its time less
/// what its starts cost at the desk), scaled by its crate's work factor,
/// and each start at what the runner charges for it.
pub(crate) fn windows_seconds(case: &TestCase, start: f64, counts: &Counts) -> f64 {
    let krate = tiers::crate_of(&case.binary);
    let tier = Tier::of(&case.binary, &case.name);
    let above = (case.seconds - start).max(0.0);
    let work = (above - at_desk(counts)).max(0.0);
    work * work_factor(krate, tier) + on_the_runner(counts)
}

/// Each tier's estimate on the Windows runner, from a desk's run and the
/// starts its tests wrote down; and each test's, for a comparison.
pub(crate) struct WindowsEstimate {
    pub(crate) tiers: [f64; 3],
    /// What the tier's starts make of it.
    pub(crate) starts: [f64; 3],
    /// The tier's starts, by kind.
    pub(crate) counts: [Counts; 3],
    pub(crate) tests: BTreeMap<(String, String), f64>,
}

pub(crate) fn windows_estimate(cases: &[TestCase], starts: &Starts) -> WindowsEstimate {
    let (start, _) = tiers::start_of(cases);
    let mut estimate = WindowsEstimate {
        tiers: [0.0; 3],
        starts: [0.0; 3],
        counts: Default::default(),
        tests: BTreeMap::new(),
    };
    for case in cases {
        let counts = starts.of(&case.binary, &case.name);
        let seconds = windows_seconds(case, start, &counts);
        let index = Tier::of(&case.binary, &case.name).index();
        estimate.tiers[index] += seconds;
        estimate.starts[index] += on_the_runner(&counts);
        estimate.counts[index].add(&counts);
        estimate
            .tests
            .insert((case.binary.clone(), case.name.clone()), seconds);
    }
    estimate
}

/// The starts a desk's run made, by tier, and what they would cost on the
/// Windows runner: printed after [`estimate`] when the run wrote them down.
pub(crate) fn starts_text(cases: &[TestCase], starts: &Starts) -> String {
    let estimate = windows_estimate(cases, starts);
    let mut text = String::from(
        "\n  Processes the tests started, and what the Windows runner would charge for them \
         (a POSIX shell at the desk is Windows PowerShell there):\n",
    );
    for tier in Tier::ALL {
        let index = tier.index();
        let counts = &estimate.counts[index];
        let _ = writeln!(
            text,
            "  {:<16} {:>4} starts {:>6.1} s   on Windows {:>6.1} s   {}",
            tier.name(),
            counts.total(),
            estimate.starts[index],
            estimate.tiers[index],
            counts.brief()
        );
    }
    text
}

/// How many seconds of a tier's estimated Windows time a change may add
/// before the comparison warns: under the noise of two runs of `main` on
/// one runner (2 to 3 s of tier 3 between rounds), and well under what
/// step 17 found too late (11.7 s).
pub(crate) const WARN_SECONDS: f64 = 3.0;

/// This tree's estimate against a base's, both run at the desk with their
/// starts written down: each tier's estimate on the Windows runner on each
/// side, the change and the share of its budget; the tests that moved most;
/// and a warning for each tier that grew by more than [`WARN_SECONDS`].
/// Returns the text and whether it warned.
pub(crate) fn compare(
    head: (&[TestCase], &Starts),
    base: (&[TestCase], &Starts),
    label: &str,
    top: usize,
) -> (String, bool) {
    // A test that failed on either side is no measurement on either: a
    // timeout reads as its limit's length, a panic as however far it got.
    // It is left out of both, and named.
    let failed: std::collections::BTreeSet<(String, String)> = head
        .0
        .iter()
        .chain(base.0)
        .filter(|case| case.failed)
        .map(|case| (case.binary.clone(), case.name.clone()))
        .collect();
    let kept = |cases: &[TestCase]| -> Vec<TestCase> {
        cases
            .iter()
            .filter(|case| !failed.contains(&(case.binary.clone(), case.name.clone())))
            .cloned()
            .collect()
    };
    let new = windows_estimate(&kept(head.0), head.1);
    let old = windows_estimate(&kept(base.0), base.1);
    let mut text = format!(
        "\nThe Windows runner's tiers, estimated for this tree and for {label}, both run at this \
         desk:\n\n  tier              {label:>12}   this tree   change   budget   starts, then and now\n"
    );
    let mut warned = false;
    for tier in Tier::ALL {
        let index = tier.index();
        let change = new.tiers[index] - old.tiers[index];
        let budget = tier.budget(Os::Windows);
        let warn = change > WARN_SECONDS;
        warned |= warn;
        let _ = writeln!(
            text,
            "  {:<16} {:>10.1} s {:>9.1} s {:>+7.1} s   {budget:>4.0} s   {} -> {}{}",
            tier.name(),
            old.tiers[index],
            new.tiers[index],
            change,
            old.counts[index].total(),
            new.counts[index].total(),
            if warn {
                format!(
                    "\n      warning: {change:.1} s more of Windows' {} tier, estimated ({:.0}% of its \
                     budget): measure it with `try -f os=windows -f command=test -f against=main` \
                     before the pull request",
                    tier.name(),
                    change / budget * 100.0
                )
            } else {
                String::new()
            }
        );
    }
    let mut moved: Vec<(&(String, String), f64)> = new
        .tests
        .iter()
        .map(|(key, seconds)| (key, seconds - old.tests.get(key).copied().unwrap_or(0.0)))
        .chain(
            old.tests
                .iter()
                .filter(|(key, _)| !new.tests.contains_key(*key))
                .map(|(key, seconds)| (key, -seconds)),
        )
        .collect();
    moved.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
    let moved: Vec<_> = moved
        .into_iter()
        .filter(|(_, change)| change.abs() >= 0.05)
        .take(top)
        .collect();
    if !moved.is_empty() {
        text.push_str("\n  The tests whose estimate moved most, with their starts here:\n");
        for ((binary, name), change) in moved {
            let counts = head.1.of(binary, name);
            let _ = writeln!(
                text,
                "  {change:>+7.2} s  {binary}  {name}{}",
                if counts.total() > 0 {
                    format!("  [{}]", counts.brief())
                } else {
                    String::new()
                }
            );
        }
    }
    if !failed.is_empty() {
        let _ = writeln!(
            text,
            "\n  {} test(s) failed on one side or both, and are left out of both estimates:",
            failed.len()
        );
        for (binary, name) in &failed {
            let _ = writeln!(text, "    {binary}  {name}");
        }
    }
    if !head.1.is_empty() && base.1.is_empty() {
        text.push_str(
            "\n  The base wrote down no starts: its tree is from before they were counted, so \
             every start here reads as new. Compare with a base that counts them.\n",
        );
    }
    (text, warned)
}

/// The directory a tree's run writes its starts to: in its own build
/// directory, emptied before each run.
pub(crate) fn starts_dir(tree: &Path) -> PathBuf {
    tree.join("target").join("like-runner").join("starts")
}

/// The base given with `--base`, checked as `ab` checks its own: a checkout
/// of the workspace, outside this one (inside it, this tree's
/// `.cargo/config.toml` would apply to its build too).
pub(crate) fn base_tree(base: &Path) -> Result<PathBuf> {
    let base = crate::workspace::normalised(
        &std::path::absolute(base).map_err(|error| format!("{}: {error}", base.display()))?,
    );
    if !base.join("Cargo.toml").is_file() {
        return Err(format!(
            "{} is not a checkout of the workspace",
            base.display()
        ));
    }
    crate::ab::refuse_base(&crate::workspace::root(), &base)?;
    Ok(base)
}

/// The base's tests, built and run as the blocking job runs them, in its
/// own build directory, with their starts written down. A failing test is
/// said and the base's estimate is still made: what is compared is the
/// time the tests took.
pub(crate) fn run_base(base: &Path, packages: &[String], rest: &[String]) -> Result<()> {
    let starts = starts_dir(base);
    crate::workspace::forget_dir(&starts)?;
    crate::checks::forget_reports(base)?;
    let mut build = crate::checks::ci_run(packages);
    build.push("--no-run".to_owned());
    let status = |mut command: std::process::Command, what: &str| {
        command
            .status()
            .map_err(|error| format!("could not run cargo in {}: {error}", base.display()))
            .map(|status| {
                if !status.success() {
                    println!("{what} in {} ended {status}", base.display());
                }
                status.success()
            })
    };
    if !status(crate::ab::nextest(base, &build), "the build")? {
        return Err(format!(
            "the base's tests did not build in {}",
            base.display()
        ));
    }
    let runs = crate::checks::ci_runs(packages, rest, crate::checks::splits(base));
    let last = runs.len() - 1;
    for (index, run) in runs.iter().enumerate() {
        let mut command = crate::ab::nextest(base, run);
        command.env(crate::starts::VARIABLE, &starts);
        status(command, "a run of the base's tests")?;
        if index == last {
            crate::checks::bring_together(base)?;
        } else {
            crate::checks::set_aside(base)?;
        }
    }
    Ok(())
}

/// A table to paste in place of [`WORK_FACTORS`], measured so that the
/// estimate of the desk's run gives the runner's figure for each crate and
/// tier: the runner's time above the start of a test's process, less what
/// the desk's tests' starts would be charged there, over the desk's time
/// less what its starts cost at the desk. What the runner ran and the desk
/// did not (each Windows shell's own tests), and what the desk ran and the
/// runner did not (bash's and sh's), are carried by the factor, so the
/// table is of the two systems' tests as they are; a crate whose desk
/// starts would cost more than all its runner time is given 0.
pub(crate) fn measure_work(
    desk: &[TestCase],
    desk_starts: &Starts,
    windows: &[TestCase],
) -> String {
    let (desk_start, _) = tiers::start_of(desk);
    let (runner_start, _) = tiers::start_of(windows);
    let key = |case: &TestCase| {
        (
            tiers::crate_of(&case.binary).to_owned(),
            Tier::of(&case.binary, &case.name),
        )
    };
    // Each crate and tier: the desk's work, and what its starts would be
    // charged on the runner.
    let mut at_desk_work: BTreeMap<(String, Tier), f64> = BTreeMap::new();
    let mut charged: BTreeMap<(String, Tier), f64> = BTreeMap::new();
    for case in desk {
        let counts = desk_starts.of(&case.binary, &case.name);
        let above = (case.seconds - desk_start).max(0.0);
        *at_desk_work.entry(key(case)).or_default() += (above - at_desk(&counts)).max(0.0);
        *charged.entry(key(case)).or_default() += on_the_runner(&counts);
    }
    let mut on_runner: BTreeMap<(String, Tier), f64> = BTreeMap::new();
    for case in windows {
        *on_runner.entry(key(case)).or_default() += (case.seconds - runner_start).max(0.0);
    }
    let mut text = String::from("const WORK_FACTORS: &[(&str, Tier, f64)] = &[\n");
    for ((krate, tier), seconds) in &at_desk_work {
        if *seconds < 0.05 {
            continue;
        }
        let k = (krate.clone(), *tier);
        let runner = on_runner.get(&k).copied().unwrap_or(0.0);
        let starts = charged.get(&k).copied().unwrap_or(0.0);
        let _ = writeln!(
            text,
            "    (\"{krate}\", Tier::{:?}, {:.2}),",
            tier,
            (runner - starts).max(0.0) / seconds
        );
    }
    text.push_str("];\n");
    text
}

/// A `try` run's Windows report: a plain run's (`-f command=test`), or an
/// A/B's head, the median of its rounds. What [`measure_work`] reads of a
/// runner.
pub(crate) fn runner_report(run: u64) -> Result<Vec<TestCase>> {
    let name = crate::runs::try_artifact_name(Os::Windows);
    let directory = crate::runs::artifact_dir(run, name)?
        .ok_or_else(|| format!("run {run} kept no `{name}`"))?;
    let files = crate::runs::files_under(&directory)?;
    match files
        .iter()
        .find(|path| path.ends_with("nextest/ci/junit.xml"))
    {
        Some(report) => crate::junit::read(report),
        None => {
            let sides = crate::records::ab_reports(run, Os::Windows)?
                .ok_or_else(|| format!("run {run} kept no `{name}`"))?;
            Ok(crate::ab::medians(&sides.head))
        }
    }
}

/// [`compare`] of this tree's run with the base's, as [`run_base`] left it.
pub(crate) fn compare_with_base(
    cases: &[TestCase],
    starts: &Starts,
    base: &Path,
    top: usize,
) -> Result<(String, bool)> {
    let base_cases = crate::junit::read(&crate::workspace::nextest_report(base, "ci"))?;
    let base_starts = Starts::read(&starts_dir(base))?;
    let label = base.file_name().map_or_else(
        || "the base".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    Ok(compare(
        (cases, starts),
        (&base_cases, &base_starts),
        &label,
        top,
    ))
}

/// A table of factors to paste in place of [`FACTORS`] and
/// [`TIER_FACTORS`]: the runners' medians over the desk's, by crate and
/// tier, from `desk` and each OS's rounds.
pub(crate) fn measure(desk: &[TestCase], linux: &[TestCase], windows: &[TestCase]) -> String {
    let desk = by_crate_and_tier(desk);
    let linux = by_crate_and_tier(linux);
    let windows = by_crate_and_tier(windows);
    let tier_name = |tier: Tier| match tier {
        Tier::Unit => "Tier::Unit",
        Tier::Integration => "Tier::Integration",
        Tier::System => "Tier::System",
    };
    let mut text = String::from("const FACTORS: &[(&str, Tier, f64, f64)] = &[\n");
    for ((krate, tier), seconds) in &desk {
        if *seconds < 0.05 {
            continue;
        }
        let key = (krate.clone(), *tier);
        let _ = writeln!(
            text,
            "    (\"{krate}\", {}, {:.2}, {:.2}),",
            tier_name(*tier),
            linux.get(&key).copied().unwrap_or(0.0) / seconds,
            windows.get(&key).copied().unwrap_or(0.0) / seconds
        );
    }
    text.push_str("];\n\nconst TIER_FACTORS: [(Tier, f64, f64); 3] = [\n");
    for tier in Tier::ALL {
        let sum = |sums: &BTreeMap<(String, Tier), f64>| {
            sums.iter()
                .filter(|((_, of), _)| *of == tier)
                .map(|(_, seconds)| seconds)
                .sum::<f64>()
        };
        let at_desk = sum(&desk).max(f64::EPSILON);
        let _ = writeln!(
            text,
            "    ({}, {:.2}, {:.2}),",
            tier_name(tier),
            sum(&linux) / at_desk,
            sum(&windows) / at_desk
        );
    }
    text.push_str("];\n");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::junit::tests::case;

    /// A desk's report: a hundred quick tests (the start cost, 2 ms), and
    /// one second above it for a known crate in tier 2 and for an unknown
    /// one in tier 1.
    fn desk() -> Vec<TestCase> {
        let mut cases: Vec<TestCase> = (0..100)
            .map(|index| case("cineris-base", &format!("t::quick_{index}"), 0.002))
            .collect();
        cases.push(case("cineris-llm::it", "decode::a", 1.002));
        cases.push(case("cineris-new", "t::b", 1.002));
        cases
    }

    #[test]
    fn a_crate_is_scaled_by_its_own_factor_and_an_unknown_one_by_its_tiers() {
        assert_eq!(factor("cineris-llm", Tier::Integration, Os::Windows), 2.17);
        assert_eq!(factor("cineris-new", Tier::Unit, Os::Windows), 2.61);
        assert_eq!(factor("cineris-new", Tier::Unit, Os::Linux), 1.43);

        let text = estimate(&desk());

        // Windows: 1 s of cineris-llm's tier 2 is 2.2 s on the runner and
        // 2.6 s on a slow one; 1 s of an unknown crate's tier 1 is 2.6 s.
        assert!(
            text.contains("  2, integration      1.0 s    2.2 s          2.6 s     40 s\n"),
            "{text}"
        );
        assert!(text.contains("      most: cineris-llm 2.2 s\n"), "{text}");
        assert!(
            text.contains("  1, unit             1.0 s    2.6 s          3.2 s     20 s\n"),
            "{text}"
        );
        // Linux's slow runner is 1.44 times the measured one.
        assert!(
            text.contains("  2, integration      1.0 s    1.4 s          2.1 s     40 s\n"),
            "{text}"
        );
    }

    #[test]
    fn a_tier_a_slow_runner_would_put_over_its_budget_is_named() {
        let mut cases = desk();
        cases.push(case("cineris-tools::it", "shell::sys_x", 2.502));

        let text = estimate(&cases);

        // 2.5 s of cineris-tools' system tests is 78.9 s on the Windows
        // runner, and 96 s on a slow one.
        assert!(text.contains("78.9 s"), "{text}");
        assert!(text.contains("90 s  over on a slow runner"), "{text}");
    }

    fn starts(lines: &[(&str, &str, &str, usize)]) -> Starts {
        let mut text = String::new();
        for (binary, name, program, count) in lines {
            for _ in 0..*count {
                text.push_str(&format!("{binary}\t{name}\t{program}\n"));
            }
        }
        Starts::parse(&text)
    }

    #[test]
    fn a_tests_starts_are_charged_at_the_runners_price_and_its_work_by_its_crate() {
        // 0.104 s at the desk above a 2 ms start: 0.1 s of it the work, and
        // two bash starts at 2 ms each.
        // `cineris-policy`'s system tests, whose work factor is well above 1.
        let case = case("cineris-policy::it", "trust::sys_a", 0.106);
        assert!(work_factor("cineris-policy", Tier::System) > 2.0);
        let counts = starts(&[("cineris-policy::it", "trust::sys_a", "bash", 2)])
            .of("cineris-policy::it", "trust::sys_a");
        let seconds = windows_seconds(&case, 0.002, &counts);
        // The work is the time less the two starts' desk cost.
        let work = (0.104 - 2.0 * cost(Kind::Posix, Os::Linux))
            * work_factor("cineris-policy", Tier::System);
        let shells = 2.0 * cost(Kind::WindowsPowerShell, Os::Windows);
        assert!((seconds - (work + shells)).abs() < 1e-9, "{seconds}");
        // A test that started nothing is all work.
        let quiet = windows_seconds(&case, 0.002, &Counts::default());
        assert!(
            (quiet - 0.104 * work_factor("cineris-policy", Tier::System)).abs() < 1e-9,
            "{quiet}"
        );
        // A version probe at the desk costs nothing on Windows, where the
        // tests' shell is asked nothing.
        let probed = starts(&[("cineris-policy::it", "trust::sys_a", "probe>bash", 3)])
            .of("cineris-policy::it", "trust::sys_a");
        let seconds = windows_seconds(&case, 0.002, &probed);
        assert!(
            (seconds
                - (0.104 - 3.0 * cost(Kind::Probe, Os::Linux))
                    * work_factor("cineris-policy", Tier::System))
            .abs()
                < 1e-9,
            "{seconds}"
        );
        // Starts that cost more at the desk than the test took leave no
        // work, and no negative time.
        let short = TestCase {
            seconds: 0.003,
            ..case.clone()
        };
        let many = starts(&[("cineris-policy::it", "trust::sys_a", "git", 50)])
            .of("cineris-policy::it", "trust::sys_a");
        let seconds = windows_seconds(&short, 0.002, &many);
        assert!(
            (seconds - 50.0 * cost(Kind::Git, Os::Windows)).abs() < 1e-9,
            "{seconds}"
        );
    }

    /// A desk's report of a tree: the start cost, and one system test that
    /// started `shells` shells.
    fn tree(shells: usize) -> (Vec<TestCase>, Starts) {
        let mut cases = desk();
        // Its work is 10 ms whatever it starts, each bash 2 ms more.
        cases.push(case(
            "cineris-core::it",
            "knowledge::sys_x",
            0.012 + shells as f64 * 0.002,
        ));
        let starts = starts(&[("cineris-core::it", "knowledge::sys_x", "bash", shells)]);
        (cases, starts)
    }

    #[test]
    fn a_change_that_adds_shell_starts_is_warned_of_with_the_test_that_made_them() {
        let (base, base_starts) = tree(2);
        let (head, head_starts) = tree(14);
        let (text, warned) = compare((&head, &head_starts), (&base, &base_starts), "main", 5);
        // Twelve more shells at Windows PowerShell's price.
        let more = 12.0 * cost(Kind::WindowsPowerShell, Os::Windows);
        assert!(more > WARN_SECONDS);
        assert!(warned, "{text}");
        assert!(
            text.contains(&format!(
                "warning: {more:.1} s more of Windows' 3, system tier"
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "{more:+.2} s  cineris-core::it  knowledge::sys_x  [posix shell 14]"
            )),
            "{text}"
        );
        // The same tree against itself warns of nothing.
        let (text, warned) = compare((&head, &head_starts), (&head, &head_starts), "main", 5);
        assert!(!warned, "{text}");
        assert!(!text.contains("warning"), "{text}");
    }

    #[test]
    fn a_crate_whose_factor_was_measured_at_0_still_shows_its_work() {
        assert_eq!(work_factor("cineris-core", Tier::System), LEAST_WORK_FACTOR);
        let case = case("cineris-core::it", "trust::sys_a", 1.002);
        assert!((windows_seconds(&case, 0.002, &Counts::default()) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_change_under_the_warning_is_shown_and_not_warned_of() {
        let (base, base_starts) = tree(2);
        let (head, head_starts) = tree(10);
        let (text, warned) = compare((&head, &head_starts), (&base, &base_starts), "main", 5);
        // Eight more shells: under the warning's 3 s.
        let more = 8.0 * cost(Kind::WindowsPowerShell, Os::Windows);
        assert!(more < WARN_SECONDS);
        assert!(!warned, "{text}");
        assert!(text.contains(&format!("{more:+.2} s")), "{text}");
    }

    #[test]
    fn a_test_that_failed_on_either_side_is_left_out_of_both_estimates() {
        let (base, base_starts) = tree(2);
        let (mut head, head_starts) = tree(14);
        // The head's test timed out: it reads as its limit, and its shells
        // as twelve more. Left out of both, nothing grew.
        for case in &mut head {
            if case.name == "knowledge::sys_x" {
                case.failed = true;
                case.seconds = 60.0;
            }
        }
        let (text, warned) = compare((&head, &head_starts), (&base, &base_starts), "main", 5);
        assert!(!warned, "{text}");
        assert!(
            text.contains("1 test(s) failed on one side or both"),
            "{text}"
        );
        assert!(
            text.contains("cineris-core::it  knowledge::sys_x"),
            "{text}"
        );
    }

    #[test]
    fn a_base_that_counted_no_starts_is_said_to_be_one() {
        let (base, _) = tree(2);
        let (head, head_starts) = tree(2);
        let (text, _) = compare(
            (&head, &head_starts),
            (&base, &Starts::default()),
            "main",
            5,
        );
        assert!(text.contains("The base wrote down no starts"), "{text}");
    }

    #[test]
    fn the_work_factors_give_the_runners_figure_for_the_desks_run() {
        // At the desk: 0.1 s of work and two bash starts at 2 ms.
        let mut desk = desk();
        desk.push(case("cineris-core::it", "knowledge::sys_x", 0.106));
        let desk_starts = starts(&[("cineris-core::it", "knowledge::sys_x", "bash", 2)]);
        // On the runner the same test took 2 s, and a test only the runner
        // has 0.5 s.
        let mut windows = desk.clone();
        for case in &mut windows {
            if case.name == "knowledge::sys_x" {
                case.seconds = 0.002 + 2.0;
            }
        }
        windows.push(case(
            "cineris-core::it",
            "knowledge::sys_windows_only",
            0.502,
        ));
        let table = measure_work(&desk, &desk_starts, &windows);
        let factor = (2.5 - 2.0 * cost(Kind::WindowsPowerShell, Os::Windows)) / 0.1;
        assert!(
            table.contains(&format!(
                "    (\"cineris-core\", Tier::System, {factor:.2}),"
            )),
            "{table}"
        );
        // By construction, the desk's run is estimated at the runner's time.
        let counts = desk_starts.of("cineris-core::it", "knowledge::sys_x");
        let estimated = 0.1 * factor + on_the_runner(&counts);
        assert!((estimated - 2.5).abs() < 1e-9, "{estimated}");
    }

    #[test]
    fn the_table_is_measured_from_a_desk_and_the_runners() {
        let mut linux = desk();
        let mut windows = desk();
        for (cases, factor) in [(&mut linux, 2.0), (&mut windows, 5.0)] {
            for case in cases.iter_mut().filter(|case| case.seconds > 1.0) {
                case.seconds = 0.002 + factor * 1.0;
            }
        }

        let table = measure(&desk(), &linux, &windows);

        assert!(
            table.contains("    (\"cineris-llm\", Tier::Integration, 2.00, 5.00),\n"),
            "{table}"
        );
        assert!(
            table.contains("    (\"cineris-new\", Tier::Unit, 2.00, 5.00),\n"),
            "{table}"
        );
        assert!(table.contains("    (Tier::Unit, 2.00, 5.00),\n"), "{table}");
        // A crate under 0.05 s at the desk is not listed.
        assert!(!table.contains("(\"cineris-base\""), "{table}");
    }
}
