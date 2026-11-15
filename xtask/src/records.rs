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
pub(crate) const AB_SECTION: &str = "both trees on one runner";

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
    let (linux, windows) = (tier.budget(Os::Linux), tier.budget(Os::Windows));
    if (linux - windows).abs() < f64::EPSILON {
        format!("{linux:.0} s")
    } else {
        format!("{linux:.0} s on Linux, {windows:.0} s on Windows")
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

/// The rounds both sides finished, or none when there is no such round. A
/// run that a failing test ended has fewer rounds on one side: medians of
/// what one side ran alone would compare nothing.
pub(crate) fn complete_rounds(mut sides: Sides) -> Option<Sides> {
    let rounds = sides.base.len().min(sides.head.len());
    if rounds == 0 {
        return None;
    }
    sides.base.truncate(rounds);
    sides.head.truncate(rounds);
    Some(sides)
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

/// The heading of one OS's A/B section, as the records write it.
pub(crate) fn ab_heading(os: Os) -> String {
    format!("{}, {}", os_name(os), AB_SECTION)
}

/// What `against` is called in an A/B's table: `Main` for `main`, the ref
/// itself otherwise.
fn against_label(against: &str) -> String {
    if against == "main" {
        "Main".to_owned()
    } else {
        format!("`{against}`")
    }
}

/// One OS's section "<OS>, both trees on one runner": which run and runner,
/// and the table of each tier on both sides.
pub(crate) fn ab_section(run: u64, against: &str, one: &Compared) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "`try` with `-f against={against}` (run {run}), the median of {} runs each, on one \
         runner at {:.1} ms a process start:\n",
        one.rounds,
        one.start * 1000.0
    );
    let _ = writeln!(
        text,
        "| Tier | The PR | {} | Change | Budget |",
        against_label(against)
    );
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

/// What opens and closes the text `ci-times` writes in a section: writing
/// again replaces what is between them, and nothing else.
const OPEN_MARK: &str =
    "<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->";
const CLOSE_MARK: &str = "<!-- end of what cargo xtask ci-times wrote -->";

/// Whether `line` is a heading of level one or two, outside a fenced block
/// (`fenced` says whether the line is inside one, and is kept up to date).
fn is_heading(line: &str, fenced: &mut bool) -> bool {
    if line.trim_start().starts_with("```") {
        *fenced = !*fenced;
        return false;
    }
    !*fenced && (line.starts_with("## ") || line.starts_with("# "))
}

/// `text` with `body` as what `ci-times` wrote in the section `## heading`:
/// a section the page does not have is added at the end, with the body
/// between the marks; in one it has, the text between the marks is
/// replaced, and everything else, the notes after the end mark included, is
/// kept. A section that has no marks was written by hand, and is refused:
/// it would lose what it says.
pub(crate) fn with_section(text: &str, heading: &str, body: &str) -> Result<String> {
    let line = format!("## {heading}");
    let block = format!("{OPEN_MARK}\n{}\n\n{CLOSE_MARK}\n", body.trim_end());
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut fenced = false;
    let mut start = None;
    for (index, each) in lines.iter().enumerate() {
        if is_heading(each, &mut fenced) && each.trim_end() == line {
            start = Some(index);
            break;
        }
    }
    let Some(start) = start else {
        let mut out = text.trim_end().to_owned();
        out.push_str(&format!("\n\n{line}\n\n{block}"));
        return Ok(out);
    };
    let mut fenced = false;
    let end = lines[start + 1..]
        .iter()
        .position(|each| is_heading(each, &mut fenced))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let within = &lines[start + 1..end];
    let open = within.iter().position(|each| each.trim_end() == OPEN_MARK);
    let close = within.iter().position(|each| each.trim_end() == CLOSE_MARK);
    let (Some(open), Some(close)) = (open, close) else {
        return Err(format!(
            "the section \"{heading}\" is already there, and ci-times did not write it: it would \
             replace what it says. Move it, rename its heading, or take it out, then write again"
        ));
    };
    if close < open {
        return Err(format!(
            "the section \"{heading}\" has its marks out of order"
        ));
    }
    let mut out: String = lines[..start + 1 + open].concat();
    out.push_str(&block);
    out.push_str(&lines[start + 2 + close..].concat());
    Ok(out)
}

/// Writes each `(heading, body)` as a section of `file`, all of them or,
/// when one is refused, none.
fn write_sections(file: &Path, sections: &[(String, String)]) -> Result<()> {
    let mut text = std::fs::read_to_string(file)
        .map_err(|error| format!("could not read {}: {error}", file.display()))?;
    for (heading, body) in sections {
        text = with_section(&text, heading, body)
            .map_err(|error| format!("{}: {error}. Nothing was written.", file.display()))?;
    }
    std::fs::write(file, text)
        .map_err(|error| format!("could not write {}: {error}", file.display()))?;
    for (heading, _) in sections {
        println!("wrote the section \"{heading}\" of {}", file.display());
    }
    Ok(())
}

/// Every A/B report a `try` run kept for `os`: `<side>-round-<n>.xml`,
/// found anywhere under its artifact, which keeps the runner's paths.
pub(crate) fn ab_reports(run: u64, os: Os) -> Result<Option<Sides>> {
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
            match complete_rounds(sides) {
                Some(sides) => compared.push(compare(os, &sides)),
                None => println!(
                    "run {run}: {} has no round that both sides finished (a test that fails \
                     ends an A/B run), so it is left out",
                    os_name(os)
                ),
            }
        }
    }
    if compared.is_empty() {
        return Err(format!(
            "run {run} has no A/B round that both sides finished, on any OS"
        ));
    }
    let sections: Vec<(String, String)> = compared
        .iter()
        .map(|one| (ab_heading(one.os), ab_section(run, against, one)))
        .collect();
    for (heading, section) in &sections {
        println!("## {heading}\n\n{section}");
    }
    println!("{}", pr_ab_section(run, &compared));
    if let Some(file) = write {
        write_sections(file, &sections)?;
    }
    Ok(())
}

/// What `ci-times RUN …` prints and, with `--write`, writes.
pub(crate) fn pr_times(runs: &[Measured], pr: Option<u64>, write: Option<&Path>) -> Result<()> {
    let section = pr_section(runs, pr);
    println!("## {PR_SECTION}\n\n{section}");
    if let Some(file) = write {
        write_sections(file, &[(PR_SECTION.to_owned(), section)])?;
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
        let section = ab_section(43, "main", &compared);
        let pr = pr_ab_section(43, std::slice::from_ref(&compared));

        assert_eq!(ab_heading(Os::Windows), "Windows, both trees on one runner");
        assert!(
            section.starts_with(
                "`try` with `-f against=main` (run 43), the median of 3 runs each, on one runner \
                 at 4.0 ms a process start:\n\n| Tier | The PR | Main | Change | Budget |\n"
            ),
            "{section}"
        );
        // Another ref is named as it is.
        assert!(
            ab_section(43, "step-21", &compared).contains("| Tier | The PR | `step-21` | Change |"),
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
    fn a_section_is_written_between_its_marks_and_the_rest_of_the_page_is_kept() {
        let page = "# After step 9: x\n\nIntro.\n\n## Windows\n\nKept.\n";

        // A page without the section gets it at its end, between the marks.
        let written = with_section(page, PR_SECTION, "Table one.\n").unwrap();
        assert_eq!(
            written,
            format!(
                "# After step 9: x\n\nIntro.\n\n## Windows\n\nKept.\n\n## On the pull \
                 request\n\n{OPEN_MARK}\nTable one.\n\n{CLOSE_MARK}\n"
            )
        );
        // A note written after the end mark, and the sections after, are
        // kept when it is written again; written twice, it is the same page.
        let noted =
            format!("{written}\nWhat the first runs found.\n\n## Other runs\n\nAlso kept.\n");
        let again = with_section(&noted, PR_SECTION, "Table two.").unwrap();
        assert_eq!(again, noted.replace("Table one.", "Table two."));
        assert_eq!(
            with_section(&again, PR_SECTION, "Table two.").unwrap(),
            again
        );
        // A section written by hand is refused, not lost.
        let by_hand = "# T\n\n## On the pull request\n\nNotes by hand.\n";
        let error = with_section(by_hand, PR_SECTION, "new").unwrap_err();
        assert!(error.contains("ci-times did not write it"), "{error}");
        // A heading inside a fenced block is not one.
        let fenced = format!(
            "# T\n\n```text\n## On the pull request\n```\n\n## On the pull request\n\n{OPEN_MARK}\n\
             old\n{CLOSE_MARK}\n\n```text\n## not a heading\n```\n"
        );
        let replaced = with_section(&fenced, PR_SECTION, "new").unwrap();
        assert!(
            replaced.contains("```text\n## not a heading\n```\n"),
            "{replaced}"
        );
        assert!(
            replaced.starts_with("# T\n\n```text\n## On the pull request\n```\n"),
            "{replaced}"
        );
        assert!(
            replaced.contains("\nnew\n") && !replaced.contains("\nold\n"),
            "{replaced}"
        );
        // The fenced heading and the real one: nothing was added.
        assert_eq!(
            replaced.matches("## On the pull request").count(),
            2,
            "{replaced}"
        );
    }

    #[test]
    fn only_the_rounds_both_sides_finished_are_compared() {
        let round = || cases(1.004, 2.004, 3.004);
        // A failing test ended the run in the head's first round.
        let ended = Sides {
            base: vec![round()],
            head: vec![],
        };
        assert!(complete_rounds(ended).is_none());
        // It ended in the third: two rounds are compared.
        let partly = Sides {
            base: vec![round(), round(), round()],
            head: vec![round(), round()],
        };
        let kept = complete_rounds(partly).unwrap();
        assert_eq!((kept.base.len(), kept.head.len()), (2, 2));
        assert_eq!(compare(Os::Linux, &kept).rounds, 2);
    }

    #[test]
    fn numbers_are_written_with_thousands() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1525), "1,525");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
