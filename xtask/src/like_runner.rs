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
//! of the same tree with `-f against=…` (an A/A run, or any A/B, whose base
//! side is read), whose rounds give each test's median on each runner. It
//! prints the table to paste here.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::junit::TestCase;
use crate::tiers::{self, Os, Tier};

/// The test threads of a hosted runner: both OSes report 4 cores.
pub(crate) const RUNNER_THREADS: &str = "4";

/// Each crate's factor in each tier, Linux and Windows: a runner's time
/// above the start of a test's process over the desk's, the same tree on
/// both. Measured on 2026-11-15 in the fifth tooling step, from the desk's
/// run of `main` at `9ddbf67` at four test threads against `try` run
/// 43663690282 (main against itself, three rounds, start 3 ms on Linux and
/// 8 ms on Windows). Only a crate and tier with at least 0.05 s at the desk
/// is listed.
const FACTORS: &[(&str, Tier, f64, f64)] = &[
    ("cortex-agent", Tier::Integration, 2.23, 4.55),
    ("cortex-base", Tier::Unit, 2.42, 4.34),
    ("cortex-base", Tier::Integration, 1.27, 3.12),
    ("cortex-cli", Tier::System, 1.28, 5.60),
    ("cortex-config", Tier::Integration, 0.25, 1.13),
    ("cortex-core", Tier::Unit, 0.66, 2.62),
    ("cortex-core", Tier::Integration, 0.73, 2.19),
    ("cortex-core", Tier::System, 1.15, 10.18),
    ("cortex-llm", Tier::Unit, 2.76, 11.19),
    ("cortex-llm", Tier::Integration, 1.63, 4.17),
    ("cortex-llm", Tier::System, 2.13, 0.55),
    ("cortex-policy", Tier::Unit, 1.60, 3.17),
    ("cortex-policy", Tier::Integration, 0.92, 2.09),
    ("cortex-policy", Tier::System, 1.47, 9.73),
    ("cortex-sandbox", Tier::System, 4.82, 6.16),
    ("cortex-session", Tier::Unit, 1.38, 2.87),
    ("cortex-session", Tier::Integration, 1.06, 2.00),
    ("cortex-session", Tier::System, 5.63, 41.54),
    ("cortex-term", Tier::Unit, 2.56, 2.75),
    ("cortex-term", Tier::Integration, 2.12, 2.84),
    ("cortex-term", Tier::System, 1.47, 5.59),
    ("cortex-tools", Tier::Unit, 3.23, 5.79),
    ("cortex-tools", Tier::Integration, 2.54, 8.29),
    ("cortex-tools", Tier::System, 4.12, 65.54),
    ("cortex-tui", Tier::Unit, 2.10, 3.60),
    ("cortex-tui", Tier::Integration, 1.71, 3.04),
    ("cortex-tui", Tier::System, 4.73, 13.73),
    ("xtask", Tier::Unit, 3.90, 7.23),
    ("xtask", Tier::System, 1.39, 6.54),
];

/// Each tier's factor, Linux and Windows, for a crate the table does not
/// know: the tier's whole time on the runner over the desk's, in the same
/// measurement.
const TIER_FACTORS: [(Tier, f64, f64); 3] = [
    (Tier::Unit, 2.27, 4.33),
    (Tier::Integration, 1.14, 2.85),
    (Tier::System, 1.76, 12.23),
];

/// How much slower than the measured runner a slow one was, Linux and
/// Windows: tier 2 of `main` at `9ddbf67` in its own CI run (43639354819,
/// start 4 and 10 ms), over the measured runner's.
const SLOW_RUNNER: (f64, f64) = (1.77, 1.41);

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
        assert_eq!(factor("cortex-llm", Tier::Integration, Os::Windows), 4.17);
        assert_eq!(factor("cortex-new", Tier::Unit, Os::Windows), 4.33);
        assert_eq!(factor("cortex-new", Tier::Unit, Os::Linux), 2.27);

        let text = estimate(&desk());

        // Windows: 1 s of cortex-llm's tier 2 is 4.2 s on the runner and
        // 5.9 s on a slow one; 1 s of an unknown crate's tier 1 is 4.3 s.
        assert!(
            text.contains("  2, integration      1.0 s    4.2 s          5.9 s     40 s\n"),
            "{text}"
        );
        assert!(text.contains("      most: cortex-llm 4.2 s\n"), "{text}");
        assert!(
            text.contains("  1, unit             1.0 s    4.3 s          6.1 s     20 s\n"),
            "{text}"
        );
        // Linux's slow runner is 1.77 times the measured one.
        assert!(
            text.contains("  2, integration      1.0 s    1.6 s          2.9 s     40 s\n"),
            "{text}"
        );
    }

    #[test]
    fn a_tier_a_slow_runner_would_put_over_its_budget_is_named() {
        let mut cases = desk();
        cases.push(case("cortex-tools::it", "shell::sys_x", 2.002));

        let text = estimate(&cases);

        // 2 s of cortex-tools' system tests is 131 s on the Windows runner.
        assert!(text.contains("131.1 s"), "{text}");
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
