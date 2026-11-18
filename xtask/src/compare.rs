//! Two runs' tier times side by side: `cargo xtask ci --tiers --compare A
//! --against B` (`docs/testing-and-ci.md`, "A CI run's test times").
//!
//! A tier near its budget used to be cut by guesswork: nothing said whether
//! a branch had made it slower than the main branch, or which crate and
//! which tests the time had gone to. This reads two JUnit reports, holds
//! each against its own start cost (a slow runner starts every test later,
//! and that is not the branch's doing), and prints each tier on both sides,
//! each crate's change, and the tests that moved most.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::junit::TestCase;
use crate::tiers::{self, Os, Tier};

/// One side of a comparison: a report and what to call it.
pub(crate) struct Side {
    /// `run 1234 (main at abc1234)`, or a file's path.
    pub(crate) label: String,
    pub(crate) cases: Vec<TestCase>,
}

/// How many crates the comparison lists by name.
const CRATES_SHOWN: usize = 12;

/// A change smaller than this, in seconds, is no change: the report shows
/// milliseconds, and a time less its start cost is not exact in binary.
const SAME: f64 = 0.0005;

/// `seconds` rounded to a tenth, with no minus sign on a zero: a change
/// of a few millionths of a second is shown as `+0.0`, not `-0.0`.
fn tenths(seconds: f64) -> f64 {
    let rounded = (seconds * 10.0).round() / 10.0;
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// What makes a comparison of `new` with `old` no measurement of the
/// tests: a test that failed on either side (a timeout reads as a test of
/// its limit's length), and a tier with tests on one side and none on the
/// other (a run that never happened reads as a tier that got faster). One
/// line each; empty when the two are comparable.
pub(crate) fn problems(new: &Side, old: &Side) -> Vec<String> {
    let mut problems = Vec::new();
    for side in [new, old] {
        let failed: Vec<String> = side
            .cases
            .iter()
            .filter(|case| case.failed)
            .map(TestCase::id)
            .collect();
        if !failed.is_empty() {
            let shown: Vec<&str> = failed.iter().take(5).map(String::as_str).collect();
            problems.push(format!(
                "{}: {} test(s) failed ({}{}): their times are not the tests' work",
                side.label,
                failed.len(),
                shown.join(", "),
                if failed.len() > shown.len() {
                    ", …"
                } else {
                    ""
                }
            ));
        }
    }
    for tier in Tier::ALL {
        let count = |side: &Side| {
            side.cases
                .iter()
                .filter(|case| Tier::of(&case.binary, &case.name) == tier)
                .count()
        };
        let (on_new, on_old) = (count(new), count(old));
        for (side, here, there) in [(new, on_new, on_old), (old, on_old, on_new)] {
            if here == 0 && there > 0 {
                problems.push(format!(
                    "{}: tier {} has no test, where the other side has {there}: its run did not \
                     happen, or its report is of part of the job",
                    side.label,
                    tier.name()
                ));
            }
        }
    }
    problems
}

/// The comparison of `new` with `old`, for the terminal: each tier above the
/// start on both sides, each crate's change, and each tier's `top` tests
/// that moved most.
pub(crate) fn compare(new: &Side, old: &Side, os: Os, top: usize) -> String {
    compare_with(new, old, os, top, Start::EachSide)
}

/// Whose start cost is taken off each side's tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Start {
    /// Each side's own: two runs on two runners, which start a process at
    /// two speeds.
    EachSide,
    /// One for both, from both sides' tests together: two trees run in turn
    /// on one machine (`cargo xtask ab`). Each side's own would differ by
    /// the noise of a millisecond, which over a thousand tests is a second.
    Shared,
}

/// [`compare`], with the start cost taken as `start` says.
pub(crate) fn compare_with(new: &Side, old: &Side, os: Os, top: usize, start: Start) -> String {
    let (new_start, new_sample) = tiers::start_of(&new.cases);
    let (old_start, old_sample) = tiers::start_of(&old.cases);
    let (new_start, old_start, start_line) = match start {
        Start::EachSide => (
            new_start,
            old_start,
            format!(
                "process start: new {:.0} ms a test, old {:.0} ms (each side's 1st percentile of \
                 {new_sample} and {old_sample} unit and integration tests), taken off each \
                 side's tests",
                1000.0 * new_start,
                1000.0 * old_start,
            ),
        ),
        Start::Shared => {
            let both: Vec<_> = new.cases.iter().chain(&old.cases).cloned().collect();
            let (shared, sample) = tiers::start_of(&both);
            (
                shared,
                shared,
                format!(
                    "process start: {:.1} ms a test for both sides, which ran on one machine (the \
                     1st percentile of their {sample} unit and integration tests together)",
                    1000.0 * shared
                ),
            )
        }
    };
    let mut text = format!(
        "  new: {}\n  old: {}\n  {start_line}\n",
        new.label, old.label,
    );
    if new_sample.min(old_sample) < 500 {
        text.push_str(
            "  One side has fewer than 500 unit and integration tests: its start cost may be more \
             than the start, and its time above the start too low.\n",
        );
    }
    let _ = writeln!(
        text,
        "\n  {:<15}  {:>18}  {:>18}  {:>9}  {:>6}",
        "tier", "new above start", "old above start", "change", "budget"
    );
    for tier in Tier::ALL {
        let now = tiers::total(&new.cases, tier, new_start);
        let before = tiers::total(&old.cases, tier, old_start);
        let _ = writeln!(
            text,
            "  {:<15}  {:>9.1} s ({:>4})  {:>9.1} s ({:>4})  {:>+7.1} s  {:>4.0} s",
            tier.name(),
            now.above_start,
            now.count,
            before.above_start,
            before.count,
            tenths(now.above_start - before.above_start),
            tier.budget(os),
        );
    }

    text.push_str(&crates(new, old, new_start, old_start));

    for tier in Tier::ALL {
        let moved = moved(new, old, new_start, old_start, tier);
        if moved.is_empty() || top == 0 {
            continue;
        }
        let _ = writeln!(
            text,
            "\n  moved most in tier {} (time above start, old -> new)",
            tier.name()
        );
        for change in moved.iter().take(top) {
            text.push_str(&moved_line(change));
        }
    }
    text
}

/// Each crate's change, tier by tier, largest change first.
fn crates(new: &Side, old: &Side, new_start: f64, old_start: f64) -> String {
    let mut rows: BTreeMap<String, ([f64; 3], [f64; 3])> = BTreeMap::new();
    for (name, seconds) in tiers::by_crate(&new.cases, new_start) {
        rows.entry(name).or_default().0 = seconds;
    }
    for (name, seconds) in tiers::by_crate(&old.cases, old_start) {
        rows.entry(name).or_default().1 = seconds;
    }
    let mut rows: Vec<(String, [f64; 3])> = rows
        .into_iter()
        .map(|(name, (now, before))| {
            (
                name,
                [now[0] - before[0], now[1] - before[1], now[2] - before[2]],
            )
        })
        .collect();
    let size = |change: &[f64; 3]| change.iter().map(|each| each.abs()).sum::<f64>();
    rows.sort_by(|(a_name, a), (b_name, b)| {
        size(b).total_cmp(&size(a)).then_with(|| a_name.cmp(b_name))
    });
    let mut text =
        String::from("\n  change by crate, above start      unit  integration  system\n");
    for (name, change) in rows.iter().take(CRATES_SHOWN) {
        let _ = writeln!(
            text,
            "    {name:<28} {:>+7.1}  {:>+11.1}  {:>+6.1}",
            tenths(change[0]),
            tenths(change[1]),
            tenths(change[2])
        );
    }
    if rows.len() > CRATES_SHOWN {
        let _ = writeln!(
            text,
            "    and {} more crates, each changed less",
            rows.len() - CRATES_SHOWN
        );
    }
    text
}

/// A crate that moved less than this in a tier, in seconds, is not listed:
/// its change is below the tenth of a second the tables round to.
const CRATE_MOVED: f64 = 0.05;

/// How many crates of a tier the detail lists by name.
const CRATES_MOVED_SHOWN: usize = 10;

/// A test's line in a list of those that moved: its change, its binary and
/// name, and what it was on each side.
fn moved_line(change: &Moved) -> String {
    let what = match (change.old, change.new) {
        (Some(old), Some(new)) => format!("{old:.3} -> {new:.3} s"),
        (None, Some(_)) => "new test".to_owned(),
        (Some(_), None) => "gone".to_owned(),
        (None, None) => String::new(),
    };
    format!(
        "    {:>+8.3} s  {}  {}  ({what})\n",
        change.delta, change.binary, change.name
    )
}

/// What moved between two sides measured on one machine (so with one
/// `start` cost for both), tier by tier: the crates that moved, each with
/// its time on both sides above the start and the change, and the `top`
/// tests that moved most with theirs. `names` are what the sides are called,
/// new first. A tier in which nothing moved is left out.
pub(crate) fn detail(
    new: &Side,
    old: &Side,
    start: f64,
    top: usize,
    names: (&str, &str),
) -> String {
    let crates_of = |side: &Side| -> BTreeMap<String, [f64; 3]> {
        tiers::by_crate(&side.cases, start).into_iter().collect()
    };
    let (now, before) = (crates_of(new), crates_of(old));
    let mut text = String::new();
    for tier in Tier::ALL {
        let mut crates: Vec<(&String, f64, f64)> = now
            .keys()
            .chain(before.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|name| {
                let at = |all: &BTreeMap<String, [f64; 3]>| {
                    all.get(name).map_or(0.0, |times| times[tier.index()])
                };
                (name, at(&before), at(&now))
            })
            .filter(|(_, old, new)| (new - old).abs() >= CRATE_MOVED)
            .collect();
        crates.sort_by(|a, b| {
            (b.2 - b.1)
                .abs()
                .total_cmp(&(a.2 - a.1).abs())
                .then_with(|| a.0.cmp(b.0))
        });
        let tests = moved(new, old, start, start, tier);
        if crates.is_empty() && (tests.is_empty() || top == 0) {
            continue;
        }
        let _ = writeln!(text, "\n  tier {} (time above start)", tier.name());
        if !crates.is_empty() {
            let _ = writeln!(
                text,
                "    crates that moved, in seconds ({} -> {}, and the change)",
                names.1, names.0
            );
            for (name, old, new) in crates.iter().take(CRATES_MOVED_SHOWN) {
                let _ = writeln!(
                    text,
                    "    {name:<28}{old:>7.2} s -> {new:>7.2} s  {:>+7.2} s",
                    new - old
                );
            }
            if crates.len() > CRATES_MOVED_SHOWN {
                let _ = writeln!(
                    text,
                    "    and {} more crates, each moved less",
                    crates.len() - CRATES_MOVED_SHOWN
                );
            }
        }
        if !tests.is_empty() && top > 0 {
            let _ = writeln!(
                text,
                "    tests that moved most, in seconds ({} -> {})",
                names.1, names.0
            );
            for change in tests.iter().take(top) {
                text.push_str(&moved_line(change));
            }
        }
    }
    if text.is_empty() {
        text.push_str("\n  nothing moved by a tenth of a second in any tier or crate\n");
    }
    text
}

/// A test's time on each side, above that side's start.
#[derive(Debug, PartialEq)]
struct Moved {
    binary: String,
    name: String,
    old: Option<f64>,
    new: Option<f64>,
    delta: f64,
}

/// A tier's tests with their change, largest change first, either way. A
/// test on one side only counts its whole time. A test that a stress run
/// repeated is counted once, by its slowest repeat.
fn moved(new: &Side, old: &Side, new_start: f64, old_start: f64, tier: Tier) -> Vec<Moved> {
    let times = |cases: &[TestCase], start: f64| {
        let mut times: BTreeMap<(String, String), f64> = BTreeMap::new();
        for case in cases {
            if Tier::of(&case.binary, &case.name) != tier {
                continue;
            }
            let above = (case.seconds - start).max(0.0);
            let time = times
                .entry((case.binary.clone(), case.name.clone()))
                .or_insert(above);
            *time = time.max(above);
        }
        times
    };
    let now = times(&new.cases, new_start);
    let before = times(&old.cases, old_start);
    let mut keys: Vec<&(String, String)> = now.keys().chain(before.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut moved: Vec<Moved> = keys
        .into_iter()
        .map(|key| {
            let (new, old) = (now.get(key).copied(), before.get(key).copied());
            Moved {
                binary: key.0.clone(),
                name: key.1.clone(),
                old,
                new,
                delta: new.unwrap_or(0.0) - old.unwrap_or(0.0),
            }
        })
        .filter(|change| change.delta.abs() >= SAME)
        .collect();
    moved.sort_by(|a, b| {
        b.delta
            .abs()
            .total_cmp(&a.delta.abs())
            .then_with(|| a.binary.cmp(&b.binary))
            .then_with(|| a.name.cmp(&b.name))
    });
    moved
}

#[cfg(test)]
mod shared_start_tests {
    use super::{Side, Start, compare_with};
    use crate::junit::TestCase;
    use crate::tiers::Os;

    fn side(label: &str, start: f64, work: f64) -> Side {
        Side {
            label: label.to_owned(),
            cases: (0..600)
                .map(|n| TestCase {
                    binary: "cineris-base".to_owned(),
                    name: format!("t{n}"),
                    seconds: start + if n % 2 == 0 { work } else { 0.0 },
                    failed: false,
                    retried: false,
                    repeat: None,
                })
                .collect(),
        }
    }

    /// One machine, one start: a millisecond of noise in one side's
    /// quickest tests is not taken off all its others.
    #[test]
    fn two_trees_on_one_machine_share_one_start_cost() {
        let old = side("old", 0.005, 0.010);
        let new = side("new", 0.006, 0.009);
        let shared = compare_with(&new, &old, Os::Linux, 0, Start::Shared);
        assert!(
            shared.contains("for both sides, which ran on one machine"),
            "{shared}"
        );
        let each = compare_with(&new, &old, Os::Linux, 0, Start::EachSide);
        assert!(each.contains("each side's 1st percentile"), "{each}");
        // The same tests, the same work: each side's own start sees no
        // change, one start sees the 0.3 s the new side's tests took more.
        assert!(each.contains("-0.3 s"), "{each}");
        assert!(shared.contains("+0.3 s"), "{shared}");
    }
}

#[cfg(test)]
mod tests {
    use super::{Moved, Side, compare, detail, moved};
    use crate::junit;
    use crate::tiers::{Os, Tier};

    #[test]
    fn a_side_with_a_failed_test_or_a_tier_missing_is_named() {
        use super::problems;
        use crate::junit::TestCase;
        let case = |binary: &str, name: &str, seconds: f64, failed: bool| TestCase {
            binary: binary.to_owned(),
            name: name.to_owned(),
            seconds,
            failed,
            retried: false,
            repeat: None,
        };
        let whole = |failed: bool| {
            vec![
                case("cineris-base", "t::unit", 0.010, false),
                case("cineris-base::it", "t::integration", 0.020, failed),
                case("cineris-cli::system", "sys_run", 0.300, false),
            ]
        };
        let side = |label: &str, cases: Vec<TestCase>| Side {
            label: label.to_owned(),
            cases,
        };
        assert_eq!(
            problems(&side("new", whole(false)), &side("old", whole(false))),
            Vec::<String>::new()
        );
        let failed = problems(&side("new", whole(true)), &side("old", whole(false)));
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(
            failed[0].starts_with("new: 1 test(s) failed (cineris-base::it t::integration)"),
            "{failed:?}"
        );
        // The system run of the new side never happened.
        let mut short = whole(false);
        short.pop();
        let missing = problems(&side("new", short), &side("old", whole(false)));
        assert_eq!(missing.len(), 1, "{missing:?}");
        assert!(
            missing[0].starts_with("new: tier 3, system has no test, where the other side has 1"),
            "{missing:?}"
        );
    }

    /// Two reports of one made-up workspace: the branch made one
    /// integration test slower, added a unit test and removed another, and
    /// ran on a runner that starts a test 10 ms later.
    const MAIN: &str = include_str!("../fixtures/tiers/compare-main.xml");
    const BRANCH: &str = include_str!("../fixtures/tiers/compare-branch.xml");

    fn sides() -> (Side, Side) {
        (
            Side {
                label: "branch".to_owned(),
                cases: junit::parse(BRANCH),
            },
            Side {
                label: "main".to_owned(),
                cases: junit::parse(MAIN),
            },
        )
    }

    #[test]
    fn the_comparison_of_two_fixed_reports_is_exactly_this() {
        let (branch, main) = sides();
        assert_eq!(
            compare(&branch, &main, Os::Linux, 2),
            "  new: branch
  old: main
  process start: new 20 ms a test, old 10 ms (each side's 1st percentile of 7 and 7 unit and \
integration tests), taken off each side's tests
  One side has fewer than 500 unit and integration tests: its start cost may be more than the \
start, and its time above the start too low.

  tier                new above start     old above start     change  budget
  1, unit                1.2 s (   5)        1.0 s (   5)     +0.2 s    20 s
  2, integration         3.5 s (   2)        1.5 s (   2)     +2.0 s    40 s
  3, system              2.0 s (   1)        2.0 s (   1)     +0.0 s    60 s

  change by crate, above start      unit  integration  system
    cineris-llm                      +0.0         +2.0    +0.0
    cineris-base                     +0.2         +0.0    +0.0
    cineris-cli                      +0.0         +0.0    +0.0

  moved most in tier 1, unit (time above start, old -> new)
      +0.600 s  cineris-base  text::tests::new_one  (new test)
      -0.400 s  cineris-base  text::tests::old_one  (gone)

  moved most in tier 2, integration (time above start, old -> new)
      +2.000 s  cineris-llm::it  replay::every_cut  (1.000 -> 3.000 s)
"
        );
    }

    #[test]
    fn the_detail_of_two_fixed_reports_is_exactly_this() {
        let (branch, main) = sides();
        assert_eq!(
            detail(&branch, &main, 0.010, 5, ("PR", "main")),
            "
  tier 1, unit (time above start)
    crates that moved, in seconds (main -> PR, and the change)
    cineris-base                    1.00 s ->    1.25 s    +0.25 s
    tests that moved most, in seconds (main -> PR)
      +0.610 s  cineris-base  text::tests::new_one  (new test)
      -0.400 s  cineris-base  text::tests::old_one  (gone)
      +0.010 s  cineris-base  text::tests::b  (0.200 -> 0.210 s)
      +0.010 s  cineris-base  text::tests::c  (0.400 -> 0.410 s)
      +0.010 s  cineris-base  text::tests::a  (0.000 -> 0.010 s)

  tier 2, integration (time above start)
    crates that moved, in seconds (main -> PR, and the change)
    cineris-llm                     1.50 s ->    3.52 s    +2.02 s
    tests that moved most, in seconds (main -> PR)
      +2.010 s  cineris-llm::it  replay::every_cut  (1.000 -> 3.010 s)
      +0.010 s  cineris-llm::it  replay::quick  (0.500 -> 0.510 s)

  tier 3, system (time above start)
    tests that moved most, in seconds (main -> PR)
      +0.010 s  cineris-cli::system  sys_version  (2.000 -> 2.010 s)
"
        );
        // Only the largest tests are listed, and none at all with 0.
        let one = detail(&branch, &main, 0.010, 1, ("PR", "main"));
        assert!(one.contains("new_one") && !one.contains("old_one"), "{one}");
        let none = detail(&branch, &main, 0.010, 0, ("PR", "main"));
        assert!(
            none.contains("crates that moved") && !none.contains("tests that moved"),
            "{none}"
        );
        // A tree compared with itself has moved nowhere.
        let same = detail(&main, &main, 0.010, 5, ("PR", "main"));
        assert!(
            same.contains("nothing moved by a tenth of a second"),
            "{same}"
        );
    }

    #[test]
    fn the_crates_that_moved_come_largest_first_and_the_rest_are_counted() {
        use crate::junit::TestCase;
        // Twelve crates, each with one unit test: crate `n` took `n` tenths
        // of a second more on the new side. The first moved too little to
        // count.
        let side = |label: &str, extra: &dyn Fn(usize) -> f64| Side {
            label: label.to_owned(),
            cases: (0..12)
                .map(|n| TestCase {
                    binary: format!("crate-{n:02}"),
                    name: "t".to_owned(),
                    seconds: 0.010 + 1.0 + extra(n),
                    failed: false,
                    retried: false,
                    repeat: None,
                })
                .collect(),
        };
        let old = side("old", &|_| 0.0);
        let new = side("new", &|n| n as f64 * 0.1 + 0.01);
        let text = detail(&new, &old, 0.010, 0, ("PR", "main"));
        let listed: Vec<&str> = text
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("crate-"))
            .map(|line| &line[..2])
            .collect();
        // Largest change first; crate-00 (+0.01 s) did not move enough,
        // and crate-01 is past the ten that are named.
        assert_eq!(
            listed,
            ["11", "10", "09", "08", "07", "06", "05", "04", "03", "02"],
            "{text}"
        );
        assert!(
            text.contains("and 1 more crates, each moved less"),
            "{text}"
        );
        assert!(!text.contains("crate-00"), "{text}");
    }

    #[test]
    fn a_slower_runner_alone_moves_nothing() {
        // The same tests, each 10 ms slower to start: nothing moved.
        let (branch, main) = sides();
        let changes = moved(&branch, &main, 0.020, 0.010, Tier::System);
        assert_eq!(changes, [], "the system test took 10 ms longer to start");
        let integration = moved(&branch, &main, 0.020, 0.010, Tier::Integration);
        // The quick integration test moved only by the start.
        assert_eq!(integration.len(), 1, "{integration:?}");
        let Moved {
            binary,
            name,
            old,
            new,
            delta,
        } = &integration[0];
        assert_eq!(
            (binary.as_str(), name.as_str()),
            ("cineris-llm::it", "replay::every_cut")
        );
        assert!((old.unwrap() - 1.0).abs() < 1e-9 && (new.unwrap() - 3.0).abs() < 1e-9);
        assert!((delta - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_report_compared_with_itself_has_moved_nowhere() {
        let main = Side {
            label: "a".to_owned(),
            cases: junit::parse(MAIN),
        };
        let again = Side {
            label: "b".to_owned(),
            cases: junit::parse(MAIN),
        };
        let text = compare(&again, &main, Os::Windows, 5);
        assert!(!text.contains("moved most"), "{text}");
        assert!(
            text.contains("+0.0 s    90 s"),
            "the Windows budget: {text}"
        );
    }

    #[test]
    fn two_real_runs_of_one_tree_compare_by_crate() {
        // The fast and the slow Windows runner of one tree: the unit tier
        // reads alike above the start (docs/testing-and-ci.md).
        let fast = Side {
            label: "fast".to_owned(),
            cases: junit::parse(include_str!("../fixtures/tiers/windows-fast-runner.xml")),
        };
        let slow = Side {
            label: "slow".to_owned(),
            cases: junit::parse(include_str!("../fixtures/tiers/windows-slow-runner.xml")),
        };
        let text = compare(&slow, &fast, Os::Windows, 3);
        let unit = text.lines().find(|line| line.contains("1, unit")).unwrap();
        let change: f64 = unit
            .split_whitespace()
            .rev()
            .nth(3)
            .and_then(|change| change.parse().ok())
            .unwrap_or_else(|| panic!("{unit}"));
        assert!(change.abs() < 1.0, "{text}");
        assert!(text.contains("change by crate"), "{text}");
        assert!(text.matches("moved most in tier").count() == 3, "{text}");
    }
}
