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

use crate::junit::TestCase;
use crate::tiers::{self, Os, Tier};

/// The test threads of a hosted runner: both OSes report 4 cores.
pub(crate) const RUNNER_THREADS: &str = "4";

/// Each crate's factor in each tier, Linux and Windows: a runner's time
/// above the start of a test's process over the desk's, the same tree on
/// both, the job split into its two runs. Measured on 2026-11-15 in the
/// fifth tooling step, from the desk's run of the step's branch at
/// `ef5009d` at four test threads (quiet, `rx --exclusive`) against the
/// branch's side of `try` run 43691692866 (an A/B against `main` at
/// `8a894c4`, five rounds, start 3 ms on Linux and 10 ms on Windows). Only
/// a crate and tier with at least 0.05 s at the desk is listed. The first
/// table, of `main` at `9ddbf67` in one run, read Windows' tier 2 of the
/// split job 51% high.
const FACTORS: &[(&str, Tier, f64, f64)] = &[
    ("cortex-agent", Tier::Integration, 2.25, 5.50),
    ("cortex-base", Tier::Unit, 2.74, 5.39),
    ("cortex-base", Tier::Integration, 0.13, 0.57),
    ("cortex-cli", Tier::System, 0.58, 3.73),
    ("cortex-config", Tier::Integration, 0.07, 0.38),
    ("cortex-core", Tier::Unit, 0.49, 1.76),
    ("cortex-core", Tier::Integration, 0.69, 1.93),
    ("cortex-core", Tier::System, 1.15, 12.38),
    ("cortex-llm", Tier::Unit, 3.72, 6.34),
    ("cortex-llm", Tier::Integration, 1.73, 3.19),
    ("cortex-llm", Tier::System, 1.90, 0.72),
    ("cortex-policy", Tier::Unit, 1.96, 3.98),
    ("cortex-policy", Tier::Integration, 0.44, 1.06),
    ("cortex-policy", Tier::System, 0.92, 7.24),
    ("cortex-sandbox", Tier::System, 3.51, 7.41),
    ("cortex-session", Tier::Unit, 2.38, 4.50),
    ("cortex-session", Tier::Integration, 0.46, 1.26),
    ("cortex-session", Tier::System, 5.10, 49.93),
    ("cortex-term", Tier::Unit, 2.36, 2.97),
    ("cortex-term", Tier::Integration, 2.12, 3.44),
    ("cortex-term", Tier::System, 1.22, 5.63),
    ("cortex-tools", Tier::Unit, 3.47, 6.52),
    ("cortex-tools", Tier::Integration, 1.04, 4.26),
    ("cortex-tools", Tier::System, 2.32, 83.87),
    ("cortex-tui", Tier::Unit, 2.17, 3.46),
    ("cortex-tui", Tier::Integration, 1.96, 3.43),
    ("cortex-tui", Tier::System, 3.40, 16.23),
    ("xtask", Tier::Unit, 3.54, 8.53),
    ("xtask", Tier::System, 1.41, 9.91),
];

/// Each tier's factor, Linux and Windows, for a crate the table does not
/// know: the tier's whole time on the runner over the desk's, in the same
/// measurement.
const TIER_FACTORS: [(Tier, f64, f64); 3] = [
    (Tier::Unit, 2.41, 4.40),
    (Tier::Integration, 0.69, 1.74),
    (Tier::System, 1.16, 12.36),
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
            .map(|index| case("cortex-base", &format!("t::quick_{index}"), 0.002))
            .collect();
        cases.push(case("cortex-llm::it", "decode::a", 1.002));
        cases.push(case("cortex-new", "t::b", 1.002));
        cases
    }

    #[test]
    fn a_crate_is_scaled_by_its_own_factor_and_an_unknown_one_by_its_tiers() {
        assert_eq!(factor("cortex-llm", Tier::Integration, Os::Windows), 3.19);
        assert_eq!(factor("cortex-new", Tier::Unit, Os::Windows), 4.40);
        assert_eq!(factor("cortex-new", Tier::Unit, Os::Linux), 2.41);

        let text = estimate(&desk());

        // Windows: 1 s of cortex-llm's tier 2 is 3.2 s on the runner and
        // 3.9 s on a slow one; 1 s of an unknown crate's tier 1 is 4.4 s.
        assert!(
            text.contains("  2, integration      1.0 s    3.2 s          3.9 s     40 s\n"),
            "{text}"
        );
        assert!(text.contains("      most: cortex-llm 3.2 s\n"), "{text}");
        assert!(
            text.contains("  1, unit             1.0 s    4.4 s          5.4 s     20 s\n"),
            "{text}"
        );
        // Linux's slow runner is 1.44 times the measured one.
        assert!(
            text.contains("  2, integration      1.0 s    1.7 s          2.5 s     40 s\n"),
            "{text}"
        );
    }

    #[test]
    fn a_tier_a_slow_runner_would_put_over_its_budget_is_named() {
        let mut cases = desk();
        cases.push(case("cortex-tools::it", "shell::sys_x", 2.002));

        let text = estimate(&cases);

        // 2 s of cortex-tools' system tests is 168 s on the Windows runner.
        assert!(text.contains("167.7 s"), "{text}");
        assert!(text.contains("90 s  over on a slow runner"), "{text}");
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
            table.contains("    (\"cortex-llm\", Tier::Integration, 2.00, 5.00),\n"),
            "{table}"
        );
        assert!(
            table.contains("    (\"cortex-new\", Tier::Unit, 2.00, 5.00),\n"),
            "{table}"
        );
        assert!(table.contains("    (Tier::Unit, 2.00, 5.00),\n"), "{table}");
        // A crate under 0.05 s at the desk is not listed.
        assert!(!table.contains("(\"cortex-base\""), "{table}");
    }
}
