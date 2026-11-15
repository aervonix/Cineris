//! A step's CI records from run ids: the tables of `docs/steps/<step>/ci.md`
//! in the form the records use, written into the file, and the pull
//! request's CI section (`docs/testing-and-ci.md`, "A CI run's test times").
//!
//! - `cargo xtask ci-times RUN --against main --pr N --write FILE` writes the
//!   section "On the pull request": which pull request and which `main`,
//!   each job's time, and each tier's time above the start of its processes
//!   against its budget, a column for each run and OS, labelled as the
//!   records label them ("Main, `sha`", "The PR at `sha`").
//! - `cargo xtask ci-times --ab RUN --write FILE` writes the section "Both
//!   trees on one runner" from a `try` run with `-f against=…`: each OS's
//!   tiers on both sides, the median of the rounds, with the change.
//! - Either prints the pull request's CI section, ready to paste.
//!
//! A section the file already has is replaced, up to the next heading of
//! its level; one it has not is added at the end. Nothing else in the file
//! is touched.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::junit::{self, TestCase};
use crate::runs::{self, Measured, RunInfo};
use crate::tiers::{self, Os, Tier};

/// The heading of the pull request's section of a `ci.md`.
pub(crate) const PR_SECTION: &str = "On the pull request";

/// The heading of the A/B's section.
pub(crate) const AB_SECTION: &str = "Both trees on one runner";

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// `1525` as the records write it: `1,525`.
fn thousands(number: usize) -> String {
    let digits = number.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Whether a run is `main`'s own.
fn is_main(info: &RunInfo) -> bool {
    info.branch == "main"
}

/// A run's column in the jobs' table.
fn job_label(info: &RunInfo) -> String {
    if is_main(info) {
        format!("Main, `{}`", short(&info.sha))
    } else {
        format!("The PR at `{}`", short(&info.sha))
    }
}

/// A run's name in the tiers' table, after the OS.
fn tier_label(info: &RunInfo) -> String {
    if is_main(info) {
        "main".to_owned()
    } else {
        format!("`{}`", short(&info.sha))
    }
}

fn budget_text(tier: Tier) -> String {
    match tier {
        Tier::System => "60 s on Linux, 90 s on Windows".to_owned(),
        _ => format!("{:.0} s", tier.budget(Os::Linux)),
    }
}

/// The section "On the pull request": a line saying which pull request and
/// which `main` (when they are known), then the jobs' table and the tiers'
/// table, in the records' form.
pub(crate) fn pr_section(runs: &[Measured], pr: Option<u64>) -> String {
    let mut text = String::new();
    let main = runs.iter().find(|measured| is_main(&measured.info));
    match (pr, main) {
        (Some(pr), Some(main)) => {
            let sha = short(&main.info.sha);
            let _ = writeln!(
                text,
                "Pull request {pr}, on `main` at `{sha}`. \"Main\" is main's own run at `{sha}` \
                 (run {}).",
                main.info.id
            );
        }
        (Some(pr), None) => {
            let _ = writeln!(text, "Pull request {pr}.");
        }
        (None, _) => {}
    }
    for measured in runs.iter().filter(|measured| !is_main(&measured.info)) {
        let _ = writeln!(
            text,
            "The PR's run is {}, at `{}`.",
            measured.info.id,
            short(&measured.info.sha)
        );
    }
    text.push('\n');

    let _ = write!(text, "| Job |");
    for measured in runs {
        let _ = write!(text, " {} |", job_label(&measured.info));
    }
    let _ = writeln!(text, "\n|---|{}", "---|".repeat(runs.len()));
    let mut names: Vec<&str> = Vec::new();
    for measured in runs {
        for (name, _) in &measured.info.jobs {
            if !names.contains(&name.as_str()) {
                names.push(name);
            }
        }
    }
    names.sort_by_key(|name| (name.contains("windows"), *name));
    for name in names {
        let _ = write!(text, "| `{name}` |");
        for measured in runs {
            let took = measured
                .info
                .jobs
                .iter()
                .find(|(job, _)| job == name)
                .and_then(|(_, took)| *took);
            match took {
                Some(seconds) => {
                    let _ = write!(text, " {seconds} s |");
                }
                None => text.push_str(" — |"),
            }
        }
        text.push('\n');
    }

    let mut columns: Vec<(Os, &RunInfo, &Vec<TestCase>)> = Vec::new();
    for os in [Os::Linux, Os::Windows] {
        for measured in runs {
            let cases = match os {
                Os::Linux => measured.linux.as_ref(),
                Os::Windows => measured.windows.as_ref(),
            };
            if let Some(cases) = cases {
                columns.push((os, &measured.info, cases));
            }
        }
    }
    if columns.is_empty() {
        text.push_str("\nNo test job of these runs kept a JUnit report.\n");
        return text;
    }
    let starts: Vec<f64> = columns
        .iter()
        .map(|(_, _, cases)| tiers::start_of(cases).0)
        .collect();
    text.push_str("\n| Time above process start, against the budget |");
    for ((os, info, _), start) in columns.iter().zip(&starts) {
        let _ = write!(
            text,
            " {}, {} ({:.0} ms) |",
            os_name(*os),
            tier_label(info),
            1000.0 * start
        );
    }
    let _ = writeln!(text, "\n|---|{}", "---|".repeat(columns.len()));
    for tier in Tier::ALL {
        let (number, name) = tier.name().split_once(", ").unwrap_or(("", tier.name()));
        let _ = write!(text, "| Tier {number}, {name} ({}) |", budget_text(tier));
        for ((_, _, cases), start) in columns.iter().zip(&starts) {
            let _ = write!(
                text,
                " {:.1} s |",
                tiers::total(cases, tier, *start).above_start
            );
        }
        text.push('\n');
    }
    text
}

fn os_name(os: Os) -> &'static str {
    match os {
        Os::Linux => "Linux",
        Os::Windows => "Windows",
    }
}

/// One OS's side of an A/B run: each round's reports, base and head.
#[derive(Debug, Default)]
pub(crate) struct Sides {
    pub(crate) base: Vec<Vec<TestCase>>,
    pub(crate) head: Vec<Vec<TestCase>>,
}

/// What an A/B of one OS came to: each tier's time above the start, and
/// its count of tests, on each side (the median of the rounds), and the
/// start cost both were held to.
pub(crate) struct Compared {
    pub(crate) os: Os,
    pub(crate) rounds: usize,
    pub(crate) start: f64,
    /// For each tier, (head, head's tests, base, base's tests).
    pub(crate) tiers: Vec<(Tier, f64, usize, f64, usize)>,
}

/// Compares the sides of one OS as `cargo xtask ab` does: each test's
/// median over the rounds, and one start cost for both, from both sides'
/// tests together.
pub(crate) fn compare(os: Os, sides: &Sides) -> Compared {
    let head = crate::ab::medians(&sides.head);
    let base = crate::ab::medians(&sides.base);
    let both: Vec<TestCase> = head.iter().chain(&base).cloned().collect();
    let (start, _) = tiers::start_of(&both);
    let tiers = Tier::ALL
        .iter()
        .map(|tier| {
            let new = tiers::total(&head, *tier, start);
            let old = tiers::total(&base, *tier, start);
            (
                *tier,
                new.above_start,
                new.count,
                old.above_start,
                old.count,
            )
        })
        .collect();
    Compared {
        os,
        rounds: sides.head.len().min(sides.base.len()),
        start,
        tiers,
    }
}

fn signed(change: f64) -> String {
    if change >= 0.05 {
        format!("+{change:.1} s")
    } else if change <= -0.05 {
        format!("−{:.1} s", -change)
    } else {
        "+0.0 s".to_owned()
    }
}

fn budget_of(tier: Tier, os: Os) -> String {
    format!("{:.0} s", tier.budget(os))
}

/// The section "Both trees on one runner": a table for each OS.
pub(crate) fn ab_section(run: u64, against: &str, compared: &[Compared]) -> String {
    let mut text = String::new();
    for one in compared {
        let _ = writeln!(
            text,
            "{}: `try` with `-f against={against}` (run {run}), the median of {} runs each, on one \
             runner at {:.1} ms a process start.\n",
            os_name(one.os),
            one.rounds,
            one.start * 1000.0
        );
        let _ = writeln!(text, "| Tier | This branch | Main | Change | Budget |");
        let _ = writeln!(text, "|---|---|---|---|---|");
        for (tier, new, new_count, old, old_count) in &one.tiers {
            let _ = writeln!(
                text,
                "| {} | {new:.1} s ({} tests) | {old:.1} s ({}) | {} | {} |",
                tier.name(),
                thousands(*new_count),
                thousands(*old_count),
                signed(new - old),
                budget_of(*tier, one.os)
            );
        }
        text.push('\n');
    }
    text
}

/// The pull request's CI section, from an A/B: each tier's change and its
/// share of the budget, by OS, Windows first as the records give it.
pub(crate) fn pr_ab_section(run: u64, compared: &[Compared]) -> String {
    let mut ordered: Vec<&Compared> = compared.iter().collect();
    ordered.sort_by_key(|one| one.os != Os::Windows);
    let rounds = compared.iter().map(|one| one.rounds).min().unwrap_or(0);
    let mut text = format!(
        "## CI time (both trees on one runner, the median of {rounds} runs each; try run {run})\n\n| Tier |"
    );
    for one in &ordered {
        let _ = write!(text, " {} |", os_name(one.os));
    }
    let _ = writeln!(text, "\n|---|{}", "---|".repeat(ordered.len()));
    for tier in Tier::ALL {
        let _ = write!(text, "| {} |", tier.name());
        for one in &ordered {
            if let Some((_, new, _, old, _)) = one.tiers.iter().find(|row| row.0 == tier) {
                let _ = write!(
                    text,
                    " {} ({new:.1} s of {:.0}) |",
                    signed(new - old),
                    tier.budget(one.os)
                );
            }
        }
        text.push('\n');
    }
    text
}

/// `text` with the section `## heading` replaced by `body`, up to the next
/// heading of the same level or higher, or added at the end.
pub(crate) fn with_section(text: &str, heading: &str, body: &str) -> String {
    let line = format!("## {heading}");
    let body = body.trim_end();
    let section = format!("{line}\n\n{body}\n");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(start) = lines.iter().position(|each| each.trim_end() == line) else {
        let mut out = text.trim_end().to_owned();
        out.push_str("\n\n");
        out.push_str(&section);
        return out;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|each| each.starts_with("## ") || each.starts_with("# "))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let mut out: String = lines[..start].concat();
    out.push_str(&section);
    if end < lines.len() {
        out.push('\n');
        out.push_str(&lines[end..].concat());
    }
    out
}

/// Writes `body` as the section `heading` of `file`.
fn write_section(file: &Path, heading: &str, body: &str) -> Result<()> {
    let text = std::fs::read_to_string(file)
        .map_err(|error| format!("could not read {}: {error}", file.display()))?;
    std::fs::write(file, with_section(&text, heading, body))
        .map_err(|error| format!("could not write {}: {error}", file.display()))?;
    println!("wrote the section \"{heading}\" of {}", file.display());
    Ok(())
}

/// Every A/B report a `try` run kept for `os`: `<side>-round-<n>.xml`,
/// found anywhere under its artifact, which keeps the runner's paths.
fn ab_reports(run: u64, os: Os) -> Result<Option<Sides>> {
    let name = match os {
        Os::Linux => "try-linux",
        Os::Windows => "try-windows",
    };
    let Some(directory) = runs::artifact_dir(run, name)? else {
        return Ok(None);
    };
    let mut found: Vec<(String, u32, PathBuf)> = Vec::new();
    let mut stack = vec![directory];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .map_err(|error| format!("could not list {}: {error}", dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let file = entry.file_name().to_string_lossy().into_owned();
            if let Some((side, round)) = file
                .strip_suffix(".xml")
                .and_then(|stem| stem.split_once("-round-"))
                && let Ok(round) = round.parse()
                && (side == "base" || side == "head")
            {
                found.push((side.to_owned(), round, path));
            }
        }
    }
    if found.is_empty() {
        return Err(format!(
            "run {run}'s `{name}` holds no A/B reports: was it a `try` run with `-f against=…`?"
        ));
    }
    found.sort_by_key(|(side, round, _)| (side.clone(), *round));
    let mut sides = Sides::default();
    for (side, _, path) in found {
        let cases = junit::read(&path)?;
        if side == "base" {
            sides.base.push(cases);
        } else {
            sides.head.push(cases);
        }
    }
    Ok(Some(sides))
}

/// `cargo xtask ci-times --ab RUN [--against REF] [--write FILE]`.
pub(crate) fn ab_times(run: u64, against: &str, write: Option<&Path>) -> Result<()> {
    let mut compared = Vec::new();
    for os in [Os::Windows, Os::Linux] {
        if let Some(sides) = ab_reports(run, os)? {
            compared.push(compare(os, &sides));
        }
    }
    if compared.is_empty() {
        return Err(format!(
            "run {run} kept no `try-linux` or `try-windows` artifact"
        ));
    }
    let section = ab_section(run, against, &compared);
    println!("## {AB_SECTION}\n\n{section}");
    println!("{}", pr_ab_section(run, &compared));
    if let Some(file) = write {
        write_section(file, AB_SECTION, &section)?;
    }
    Ok(())
}

/// What `ci-times RUN …` prints and, with `--write`, writes.
pub(crate) fn pr_times(runs: &[Measured], pr: Option<u64>, write: Option<&Path>) -> Result<()> {
    let section = pr_section(runs, pr);
    println!("## {PR_SECTION}\n\n{section}");
    if let Some(file) = write {
        write_section(file, PR_SECTION, &section)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::junit::tests::case;

    fn info(id: u64, sha: &str, branch: &str) -> RunInfo {
        RunInfo {
            id,
            sha: sha.to_owned(),
            branch: branch.to_owned(),
            event: "push".to_owned(),
            date: "2026-11-15".to_owned(),
            jobs: vec![
                ("test-windows".to_owned(), Some(190)),
                ("lint-linux".to_owned(), Some(44)),
                ("test-linux".to_owned(), None),
            ],
        }
    }

    /// A run's report: a hundred quick unit tests (the start cost, 4 ms),
    /// one slow unit test, one integration test and one system test.
    fn cases(unit: f64, integration: f64, system: f64) -> Vec<TestCase> {
        let mut cases: Vec<TestCase> = (0..100)
            .map(|index| case("cortex-base", &format!("t::quick_{index}"), 0.004))
            .collect();
        cases.push(case("cortex-base", "t::slow", unit));
        cases.push(case("cortex-core::it", "a::b", integration));
        cases.push(case("cortex-cli::system", "sys_c", system));
        cases
    }

    #[test]
    fn the_pull_requests_section_is_in_the_records_form() {
        let runs = [
            Measured {
                info: info(1, "8c818f3aaaa", "main"),
                linux: Some(cases(1.004, 2.004, 3.004)),
                windows: None,
            },
            Measured {
                info: info(2, "99f3307bbbb", "worktree-x"),
                linux: Some(cases(1.504, 2.004, 3.004)),
                windows: Some(cases(2.004, 4.004, 6.004)),
            },
        ];

        let text = pr_section(&runs, Some(48));

        assert!(
            text.starts_with(
                "Pull request 48, on `main` at `8c818f3`. \"Main\" is main's own run at \
                 `8c818f3` (run 1).\nThe PR's run is 2, at `99f3307`.\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("| Job | Main, `8c818f3` | The PR at `99f3307` |\n|---|---|---|\n"),
            "{text}"
        );
        // Linux's jobs first, a job that did not end as a dash.
        assert!(
            text.contains("| `lint-linux` | 44 s | 44 s |\n| `test-linux` | — | — |\n| `test-windows` | 190 s | 190 s |\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "| Time above process start, against the budget | Linux, main (4 ms) | Linux, \
                 `99f3307` (4 ms) | Windows, `99f3307` (4 ms) |"
            ),
            "{text}"
        );
        assert!(
            text.contains("| Tier 1, unit (20 s) | 1.0 s | 1.5 s | 2.0 s |"),
            "{text}"
        );
        assert!(
            text.contains(
                "| Tier 3, system (60 s on Linux, 90 s on Windows) | 3.0 s | 3.0 s | 6.0 s |"
            ),
            "{text}"
        );
    }

    #[test]
    fn an_ab_is_each_tiers_median_on_both_sides_with_the_change() {
        let sides = Sides {
            base: vec![
                cases(1.004, 2.004, 3.004),
                cases(9.004, 2.004, 3.004),
                cases(1.004, 2.004, 3.004),
            ],
            head: vec![
                cases(1.504, 1.004, 3.004),
                cases(1.504, 1.004, 3.004),
                cases(0.004, 1.004, 3.004),
            ],
        };

        let compared = compare(Os::Windows, &sides);
        let section = ab_section(43, "main", std::slice::from_ref(&compared));
        let pr = pr_ab_section(43, &[compared]);

        assert!(
            section.starts_with(
                "Windows: `try` with `-f against=main` (run 43), the median of 3 runs each, on \
                 one runner at 4.0 ms a process start.\n"
            ),
            "{section}"
        );
        // The median, not the mean: base's 9 s round is passed over.
        assert!(
            section.contains("| 1, unit | 1.5 s (101 tests) | 1.0 s (101) | +0.5 s | 20 s |"),
            "{section}"
        );
        assert!(
            section.contains("| 2, integration | 1.0 s (1 tests) | 2.0 s (1) | −1.0 s | 40 s |"),
            "{section}"
        );
        assert!(
            section.contains("| 3, system | 3.0 s (1 tests) | 3.0 s (1) | +0.0 s | 90 s |"),
            "{section}"
        );
        assert!(
            pr.starts_with(
                "## CI time (both trees on one runner, the median of 3 runs each; try run 43)\n"
            ),
            "{pr}"
        );
        assert!(
            pr.contains("| 2, integration | −1.0 s (1.0 s of 40) |"),
            "{pr}"
        );
    }

    #[test]
    fn a_section_is_replaced_in_place_or_added_at_the_end() {
        let page = "# After step 9: x\n\nIntro.\n\n## On the pull request\n\nOld table.\n\nMore old.\n\n## Windows\n\nKept.\n";

        let replaced = with_section(page, PR_SECTION, "New table.\n");

        assert_eq!(
            replaced,
            "# After step 9: x\n\nIntro.\n\n## On the pull request\n\nNew table.\n\n## Windows\n\nKept.\n"
        );
        // Written again, it is the same page.
        assert_eq!(with_section(&replaced, PR_SECTION, "New table."), replaced);
        // A page without the section gets it at its end.
        assert_eq!(
            with_section("# After step 9: x\n\nIntro.\n", AB_SECTION, "A/B."),
            "# After step 9: x\n\nIntro.\n\n## Both trees on one runner\n\nA/B.\n"
        );
        // The last section of a page runs to its end.
        assert_eq!(
            with_section("# T\n\n## On the pull request\n\nold\n", PR_SECTION, "new"),
            "# T\n\n## On the pull request\n\nnew\n"
        );
    }

    #[test]
    fn numbers_are_written_with_thousands() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1525), "1,525");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
