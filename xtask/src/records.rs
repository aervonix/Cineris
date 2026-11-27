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
//! is touched. A file that is not there yet, or is empty, is made with the
//! heading its place in `docs/steps/` gives it ([`new_page_heading`]), so
//! that the first write of a step's times needs no page made by hand.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::compare::{self, Side};
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

/// The jobs of the `ci` workflow that block no merge, by the start of their
/// names: the pull request's binary size (`pr-size-linux`,
/// `pr-size-windows`) and the TypeScript client of the protocol
/// (`ts-client`), which start only after the required jobs. A test reads
/// `.github/workflows/ci.yml` and fails on a job that is in neither this
/// list nor `BLOCKING`.
pub(crate) const NON_BLOCKING: [&str; 2] = ["pr-size", "ts-client"];

/// The `ci` workflow's jobs that a merge waits for, by their ids in the
/// workflow (`lint` is `lint-linux` and `lint-windows`; `test` is
/// `test-linux`; `build-windows` builds Windows' tests and archives them,
/// and `test-windows` is `test-windows-tiers` and `test-windows-system`,
/// which run them from the archive): what the test of the workflow checks
/// against, since every job not in [`NON_BLOCKING`] is taken to block.
#[cfg(test)]
const BLOCKING: [&str; 4] = ["lint", "test", "build-windows", "test-windows"];

/// Whether a job of a `ci` run blocks a merge. A job of no known kind is
/// taken as one that does, so that it is never left out of sight.
pub(crate) fn blocks(job: &str) -> bool {
    !NON_BLOCKING.iter().any(|kind| {
        job.strip_prefix(kind)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
    })
}

/// A job's place among an OS's jobs in the jobs' table, in the order they
/// run: the lint, the build of the tests where it is a job of its own
/// (`build-windows`), the tests (tiers 1 and 2 before the system tests,
/// where they are two jobs).
fn place(job: &str) -> u8 {
    if job.starts_with("lint") {
        0
    } else if job.starts_with("build") {
        1
    } else if job.ends_with("-system") {
        3
    } else {
        2
    }
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
    // The jobs that block a merge first, Linux's before Windows'; then the
    // ones that block nothing, marked, so that their minutes are never read
    // as the five-minute jobs'.
    names.sort_by_key(|name| (!blocks(name), name.contains("windows"), place(name), *name));
    for name in names {
        if blocks(name) {
            let _ = write!(text, "| `{name}` |");
        } else {
            let _ = write!(text, "| `{name}` (blocks nothing) |");
        }
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
    for ((os, info, cases), start) in columns.iter().zip(&starts) {
        // A split run's system tests carry their own runner's start, which
        // their tier is held above: said beside the other.
        let own = cases
            .iter()
            .find_map(|case| case.start)
            .map_or_else(String::new, |own| {
                format!("; tier 3 {:.0} ms", 1000.0 * own)
            });
        let _ = write!(
            text,
            " {}, {} ({:.0} ms{own}) |",
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
    /// The number of each round kept (counted from 1), in the order of
    /// `base` and `head`. Empty: they are rounds 1, 2, 3, ….
    pub(crate) numbers: Vec<usize>,
    /// The rounds left out of `base` and `head`, one line each: a side's run
    /// broke, a test failed on one side only (or not the same tests on
    /// both), or a side kept no report. `ab` leaves a round out of its
    /// comparison for the same reasons.
    pub(crate) left_out: Vec<String>,
    /// The tests that failed on both sides of a round, and nothing else
    /// did: as `ab` does, the round is compared, and they are left out of
    /// every round's medians on both sides.
    pub(crate) skipped: Vec<String>,
    /// The tests that failed on one side of a round whose runs failed by
    /// tests alone: as `ab` does since the ninth tooling step, the round is
    /// compared and they are left out of every round, and the run is a
    /// failure all the same. Also in `skipped`.
    pub(crate) one_sided: Vec<String>,
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
    /// The rounds that were compared, counted from 1.
    pub(crate) numbers: Vec<usize>,
    /// Each side's rounds, and each side's test by test median: what the
    /// tiers are the totals of, kept for the detail and the rounds' table.
    pub(crate) base_rounds: Vec<Vec<TestCase>>,
    pub(crate) head_rounds: Vec<Vec<TestCase>>,
    pub(crate) base: Vec<TestCase>,
    pub(crate) head: Vec<TestCase>,
    pub(crate) left_out: Vec<String>,
    /// The tests left out of every round: they failed on both sides.
    pub(crate) skipped: Vec<String>,
    /// The tests left out of every round that failed on one side only: a
    /// failure of the run.
    pub(crate) one_sided: Vec<String>,
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
    sides.numbers.truncate(rounds);
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
    let rounds = sides.head.len().min(sides.base.len());
    Compared {
        os,
        rounds,
        start,
        tiers,
        numbers: if sides.numbers.len() == rounds {
            sides.numbers.clone()
        } else {
            (1..=rounds).collect()
        },
        base_rounds: sides.base.clone(),
        head_rounds: sides.head.clone(),
        base,
        head,
        left_out: sides.left_out.clone(),
        skipped: sides.skipped.clone(),
        one_sided: sides.one_sided.clone(),
    }
}

/// Each round's tier times above the start, and the tiers' times, as the
/// A/B run printed them (`ab::rounds_table`): the spread of one side is the
/// noise a difference has to stand out of.
pub(crate) fn rounds_of(one: &Compared) -> String {
    crate::ab::rounds_table(&crate::ab::Compared {
        numbers: one.numbers.clone(),
        base: one.base_rounds.clone(),
        head: one.head_rounds.clone(),
    })
}

/// What moved in one OS's A/B, below its tier table: the crates and the
/// tests that moved most, each tier, `top` tests of each (0: none, and no
/// detail at all).
pub(crate) fn detail_of(one: &Compared, against: &str, top: usize) -> String {
    if top == 0 {
        return String::new();
    }
    let side = |label: &str, cases: &[TestCase]| Side {
        label: label.to_owned(),
        cases: cases.to_vec(),
    };
    let head = side("the PR", &one.head);
    let base = side(against, &one.base);
    compare::detail(&head, &base, one.start, top, ("PR", against))
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

/// Where `--write` puts the tables, and what it calls the section.
///
/// Without either flag the section is the one the command has always
/// written: `## On the pull request` for a run's tables, one `## <OS>, both
/// trees on one runner` for each OS of an A/B, each found by its heading
/// between the plain marks.
///
/// - `--section NAME` makes the tables **one section named NAME**, found
///   by its own marks, which carry the name. A step that writes several
///   (the pull request's, then the review's fixes') names each, and writing
///   one again replaces only that one. Its heading, when it is added, is
///   `## NAME` (an A/B's two OSes are `###` headings inside it).
/// - `--heading TEXT` is the heading a section is added with, when it
///   should not be its name: a dated one, as an older step's page has
///   (`After the review: the fixes' pull request (#56), 2026-11-15`). Alone,
///   it is the name too.
///
/// A section is added at the end of the page, never inside another; once it
/// is there, the marks find it wherever it has been moved to, and its
/// heading may be edited. A page's section of that heading that the tool
/// did not write (an older page's, by hand, with no marks) is never
/// written over: it is refused, and the tables go under a heading of their
/// own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Placing {
    pub(crate) section: Option<String>,
    pub(crate) heading: Option<String>,
}

impl Placing {
    /// Whether either flag was given.
    pub(crate) fn is_given(&self) -> bool {
        self.section.is_some() || self.heading.is_some()
    }

    /// Refuses a name or a heading that could not be written back and
    /// found: empty, with a line break, or with what would end a mark.
    pub(crate) fn check(&self) -> Result<()> {
        if let Some(section) = &self.section {
            Self::text_ok("--section", section)?;
            if section.contains('"') {
                return Err("`--section` may not hold a double quote: the marks quote it".into());
            }
        }
        if let Some(heading) = &self.heading {
            Self::text_ok("--heading", heading)?;
            if heading.starts_with('#') {
                return Err(
                    "`--heading` is the heading's text: ci-times writes the `## ` before it".into(),
                );
            }
        }
        Ok(())
    }

    fn text_ok(flag: &str, text: &str) -> Result<()> {
        if text.trim() != text || text.is_empty() {
            return Err(format!(
                "`{flag}` is empty or has white space at either end: `{text}`"
            ));
        }
        if text.contains(['\n', '\r']) || text.contains("-->") {
            return Err(format!(
                "`{flag}` is one line and may not hold `-->`: `{text}`"
            ));
        }
        Ok(())
    }

    /// The section's name and its heading, `default` being the heading the
    /// command writes under when it is told neither.
    fn resolve(&self, default: &str) -> (Option<String>, String) {
        match (&self.section, &self.heading) {
            (Some(name), heading) => (
                Some(name.clone()),
                heading.clone().unwrap_or_else(|| name.clone()),
            ),
            (None, Some(heading)) => (Some(heading.clone()), heading.clone()),
            (None, None) => (None, default.to_owned()),
        }
    }
}

/// What opens and closes a named section's text.
fn named_marks(name: &str) -> (String, String) {
    (
        format!(
            "<!-- written by cargo xtask ci-times, section \"{name}\": it replaces this up to the \
             end mark -->"
        ),
        format!("<!-- end of what cargo xtask ci-times wrote, section \"{name}\" -->"),
    )
}

/// The lines of `lines` that are `mark`, outside a fenced block.
fn mark_lines(lines: &[&str], mark: &str) -> Vec<usize> {
    let mut fenced = false;
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced && line.trim_end() == mark {
            found.push(index);
        }
    }
    found
}

/// `text` with `body` as what `ci-times` wrote in the section called `name`:
/// found by its marks wherever they are, and replaced between them with
/// everything else kept. A section that has no marks of that name is added
/// at the end of the page under `## heading`, unless the page has a heading
/// of that text already: that one was written by hand, or under the plain
/// marks, and is refused, as it would lose what it says. Marks that are not
/// one pair, in order, are refused too.
pub(crate) fn with_named_section(
    text: &str,
    name: &str,
    heading: &str,
    body: &str,
) -> Result<String> {
    let (open_mark, close_mark) = named_marks(name);
    let block = format!("{open_mark}\n{}\n\n{close_mark}\n", body.trim_end());
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let opens = mark_lines(&lines, &open_mark);
    let closes = mark_lines(&lines, &close_mark);
    match (opens.as_slice(), closes.as_slice()) {
        ([open], [close]) if open < close => {
            let mut out: String = lines[..*open].concat();
            out.push_str(&block);
            out.push_str(&lines[close + 1..].concat());
            Ok(out)
        }
        ([_], [_]) => Err(format!("the section \"{name}\" has its marks out of order")),
        ([], []) => {
            let line = format!("## {heading}");
            let mut fenced = false;
            let there = lines
                .iter()
                .any(|each| is_heading(each, &mut fenced) && each.trim_end() == line);
            if there {
                return Err(format!(
                    "the heading \"{heading}\" is already there, and ci-times did not write it \
                     as the section \"{name}\": it would put a second under it, or write over \
                     what it says. Give the section a heading of its own with `--heading`"
                ));
            }
            let mut out = text.trim_end().to_owned();
            out.push_str(&format!("\n\n{line}\n\n{block}"));
            Ok(out)
        }
        _ => Err(format!(
            "the section \"{name}\" has {} opening and {} closing marks: one of each is meant. \
             Take the extra ones out",
            opens.len(),
            closes.len()
        )),
    }
}

/// A section to write: its tables, and where they go.
pub(crate) struct Section {
    /// `Some`: a section found by its own marks. `None`: the plain section
    /// under `heading`.
    pub(crate) name: Option<String>,
    pub(crate) heading: String,
    pub(crate) body: String,
}

/// The step a folder of `docs/steps/` is the record of, as its headings
/// name it: `07` is `step 7`, `13a` is `step 13a`; a folder named in words
/// (`tooling-12`) is named by its name.
fn step_named(folder: &str) -> String {
    let numbered = folder.len() >= 2
        && folder.len() <= 3
        && folder.is_ascii()
        && folder[..2].chars().all(|c| c.is_ascii_digit())
        && folder[2..].chars().all(|c| c.is_ascii_lowercase());
    if numbered && let Ok(number) = folder[..2].parse::<u32>() {
        format!("step {number}{}", &folder[2..])
    } else {
        folder.to_owned()
    }
}

/// The heading a page `--write` makes starts with, from where it is, in the
/// form the step records' rule asks of it: `# After step 24: its CI times`
/// for a step's `ci.md`, `# Step 24, after review: its CI times` for a part's
/// record (`docs/steps/24/<group>/after-review.md`), and `# CI times` for
/// any other page. Its words after the colon are for the step to make its
/// own.
pub(crate) fn new_page_heading(file: &Path) -> String {
    let parts: Vec<String> = file
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let steps = parts
        .windows(2)
        .rposition(|pair| pair[0] == "docs" && pair[1] == "steps");
    let rest = steps.map_or(&[][..], |at| &parts[at + 2..]);
    match rest {
        [folder, page] if page == "ci.md" => {
            format!("# After {}: its CI times", step_named(folder))
        }
        [folder, _, part] if part.ends_with(".md") => {
            let who = step_named(folder);
            let mut letters = who.chars();
            let who: String = letters
                .next()
                .map(|first| first.to_uppercase().chain(letters).collect())
                .unwrap_or_default();
            let words = part.trim_end_matches(".md").replace('-', " ");
            format!("# {who}, {words}: its CI times")
        }
        _ => "# CI times".to_owned(),
    }
}

/// Writes each section into `file`, all of them or, when one is refused,
/// none. A file that is not there is made, its folder with it, and a file
/// that is empty is given its heading ([`new_page_heading`]) first.
fn write_sections(file: &Path, sections: &[Section]) -> Result<()> {
    let (mut text, made) = match std::fs::read_to_string(file) {
        Ok(text) if text.trim().is_empty() => (format!("{}\n", new_page_heading(file)), true),
        Ok(text) => (text, false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (format!("{}\n", new_page_heading(file)), true)
        }
        Err(error) => return Err(format!("could not read {}: {error}", file.display())),
    };
    for section in sections {
        text = match &section.name {
            None => with_section(&text, &section.heading, &section.body),
            Some(name) => with_named_section(&text, name, &section.heading, &section.body),
        }
        .map_err(|error| format!("{}: {error}. Nothing was written.", file.display()))?;
    }
    if let Some(folder) = file
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("could not make {}: {error}", folder.display()))?;
    }
    std::fs::write(file, text)
        .map_err(|error| format!("could not write {}: {error}", file.display()))?;
    if made {
        println!(
            "made {} with the heading `{}`: make its words after the colon the step's own",
            file.display(),
            new_page_heading(file)
        );
    }
    for section in sections {
        println!(
            "wrote the section \"{}\" of {}",
            section.name.as_ref().unwrap_or(&section.heading),
            file.display()
        );
    }
    Ok(())
}

/// What an A/B run's artifact for one OS held: its reports by side and
/// round, read as `ab` reads them. A round is kept when both sides' reports
/// are there and nothing went wrong but tests failing: those tests are left
/// out of every round, and named in `skipped` (both sides failed them) or in
/// `one_sided` (one side did, a failure of the run). It is left out, and
/// `left_out` says why, when a side's run broke (its report kept as
/// `<side>-round-<n>-broke.xml`), or when a side kept no report or one that
/// lists nothing.
pub(crate) fn sides_of(run: u64, artifact: &str, files: &[PathBuf]) -> Result<Sides> {
    let mut rounds: BTreeMap<usize, (Option<&PathBuf>, Option<&PathBuf>)> = BTreeMap::new();
    let mut broke: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for path in files {
        let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some((side, round)) = runs::ab_broke_name(file) {
            broke.entry(round).or_default().push(side);
            rounds.entry(round).or_default();
            continue;
        }
        if let Some((side, round)) = runs::ab_report_name(file) {
            let entry = rounds.entry(round).or_default();
            if side == "base" {
                entry.0 = Some(path);
            } else {
                entry.1 = Some(path);
            }
        }
    }
    if rounds.is_empty() {
        return Err(format!(
            "run {run}'s `{artifact}` holds no A/B reports: was it a `try` run with `-f \
             against=…`?"
        ));
    }
    let read = |path: &PathBuf| -> Result<Vec<TestCase>> {
        let xml = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        Ok(junit::parse(&xml))
    };
    let mut sides = Sides::default();
    let mut skipped: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut one_sided: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (round, (base, head)) in rounds {
        if let Some(which) = broke.get(&round) {
            sides.left_out.push(format!(
                "round {round}: the {} run broke (not by a test failing), so its report may be \
                 of part of the job",
                which.join(" and the ")
            ));
            continue;
        }
        let (Some(base), Some(head)) = (base, head) else {
            let missing = match (base, head) {
                (None, None) => "both sides",
                (None, Some(_)) => "base",
                _ => "head",
            };
            sides
                .left_out
                .push(format!("round {round}: {missing} kept no report"));
            continue;
        };
        let (base, head) = (read(base)?, read(head)?);
        let failed_ids = |cases: &[TestCase]| -> std::collections::BTreeSet<String> {
            cases
                .iter()
                .filter(|case| case.failed)
                .map(TestCase::id)
                .collect()
        };
        let (base_failed, head_failed) = (failed_ids(&base), failed_ids(&head));
        // Tests failed, and nothing else went wrong (a run that broke is
        // named above): compared, without them, as `ab` compares it. One
        // that failed on one side only is a failure of the run.
        if base.is_empty() || head.is_empty() {
            sides
                .left_out
                .push(format!("round {round}: a report lists no tests"));
        } else {
            for id in base_failed.symmetric_difference(&head_failed) {
                one_sided.insert(id.clone());
            }
            skipped.extend(base_failed);
            skipped.extend(head_failed);
            sides.base.push(base);
            sides.head.push(head);
            sides.numbers.push(round);
        }
    }
    // A test that failed on both sides is left out of every round, as `ab`
    // leaves it out of its medians.
    for cases in sides.base.iter_mut().chain(sides.head.iter_mut()) {
        cases.retain(|case| !skipped.contains(&case.id()));
    }
    sides.skipped = skipped
        .into_iter()
        .filter(|id| !one_sided.contains(id))
        .collect();
    sides.one_sided = one_sided.into_iter().collect();
    Ok(sides)
}

/// Every A/B report a `try` run kept for `os`: `<side>-round-<n>.xml`,
/// found anywhere under its artifact, which keeps the runner's paths.
pub(crate) fn ab_reports(run: u64, os: Os) -> Result<Option<Sides>> {
    let name = runs::try_artifact_name(os);
    let Some(directory) = runs::artifact_dir(run, name)? else {
        return Ok(None);
    };
    sides_of(run, name, &runs::files_under(&directory)?).map(Some)
}

/// What `ci-times --ab` and `try-result` print below one OS's tier table of
/// an A/B: the rounds that were left out and why, and what moved by crate
/// and by test (`top` tests a tier; 0 prints neither the detail nor the
/// rounds' table).
pub(crate) fn ab_detail_text(one: &Compared, against: &str, top: usize) -> String {
    let mut text = String::new();
    if top > 0 {
        let _ = writeln!(text, "{}", rounds_of(one));
    }
    if !one.left_out.is_empty() {
        let _ = writeln!(
            text,
            "Rounds left out of the comparison, with the other side's of the same round:"
        );
        for line in &one.left_out {
            let _ = writeln!(text, "  {line}");
        }
        text.push('\n');
    }
    if !one.skipped.is_empty() {
        let _ = writeln!(
            text,
            "Tests left out of every round, on both sides: they failed on both sides of a round."
        );
        for id in &one.skipped {
            let _ = writeln!(text, "  {id}");
        }
        text.push('\n');
    }
    if !one.one_sided.is_empty() {
        let _ = writeln!(
            text,
            "TESTS THAT FAILED ON ONE SIDE ONLY, left out of every round on both sides, and a \
             failure of the run: the comparison is of the other tests."
        );
        for id in &one.one_sided {
            let _ = writeln!(text, "  {id}");
        }
        text.push('\n');
    }
    if top > 0 {
        let _ = write!(
            text,
            "What moved, the PR against {against}, each side's median of {} runs:\n{}",
            one.rounds,
            detail_of(one, against, top)
        );
    }
    text
}

/// The OSes an A/B left out, each with the note that says why.
pub(crate) type LeftOut = Vec<(Os, String)>;

/// The A/B of each OS a `try` run has, as far as any round was finished by
/// both sides. An OS with none is named in the returned notes, with why.
pub(crate) fn ab_compared(run: u64) -> Result<(Vec<Compared>, LeftOut)> {
    let mut compared = Vec::new();
    let mut notes = Vec::new();
    for os in [Os::Windows, Os::Linux] {
        if let Some(sides) = ab_reports(run, os)? {
            let left_out = sides.left_out.clone();
            match complete_rounds(sides) {
                Some(sides) => compared.push(compare(os, &sides)),
                None => notes.push((
                    os,
                    format!(
                        "run {run}: {} has no round that both sides finished (a test that \
                         fails ends an A/B round), so it is left out. {}",
                        os_name(os),
                        left_out.join("; ")
                    ),
                )),
            }
        }
    }
    Ok((compared, notes))
}

/// The sections `--write` puts an A/B's tables in: one for each OS, or,
/// when the placing names one, the OSes together as `###` parts of it. An
/// OS the run had and left out (`notes`: no round both sides finished) is
/// written too, as its note, so that the page says it was left out.
fn ab_sections(
    run: u64,
    against: &str,
    compared: &[Compared],
    notes: &[(Os, String)],
    placing: &Placing,
) -> Vec<Section> {
    let mut parts: Vec<(Os, String, String)> = compared
        .iter()
        .map(|one| (one.os, ab_heading(one.os), ab_section(run, against, one)))
        .chain(
            notes
                .iter()
                .map(|(os, note)| (*os, ab_heading(*os), format!("{note}\n"))),
        )
        .collect();
    // Windows first, then Linux, as the run's OSes are read.
    parts.sort_by_key(|(os, _, _)| *os != Os::Windows);
    let (name, heading) = placing.resolve("");
    match name {
        None => parts
            .into_iter()
            .map(|(_, heading, body)| Section {
                name: None,
                heading,
                body,
            })
            .collect(),
        Some(name) => {
            let body = parts
                .iter()
                .map(|(_, heading, body)| format!("### {heading}\n\n{}", body.trim_end()))
                .collect::<Vec<_>>()
                .join("\n\n");
            vec![Section {
                name: Some(name),
                heading,
                body,
            }]
        }
    }
}

/// `cargo xtask ci-times --ab RUN [--against REF] [--write FILE]
/// [--section NAME] [--heading TEXT] [--top N]`.
pub(crate) fn ab_times(
    run: u64,
    against: &str,
    write: Option<&Path>,
    placing: &Placing,
    top: usize,
) -> Result<()> {
    placing.check()?;
    if placing.is_given() && write.is_none() {
        return Err("`--section` and `--heading` say where `--write` puts the tables".to_owned());
    }
    let (compared, notes) = ab_compared(run)?;
    for (_, note) in &notes {
        println!("{note}");
    }
    if compared.is_empty() {
        return Err(format!(
            "run {run} has no A/B round that both sides finished, on any OS"
        ));
    }
    let sections = ab_sections(run, against, &compared, &notes, placing);
    for one in &compared {
        println!(
            "## {}\n\n{}",
            ab_heading(one.os),
            ab_section(run, against, one)
        );
    }
    println!("{}", pr_ab_section(run, &compared));
    for one in &compared {
        println!(
            "## {}, what moved\n\n{}",
            os_name(one.os),
            ab_detail_text(one, against, top)
        );
    }
    if let Some(file) = write {
        write_sections(file, &sections)?;
    }
    Ok(())
}

/// What `ci-times RUN …` prints and, with `--write`, writes.
pub(crate) fn pr_times(
    runs: &[Measured],
    pr: Option<u64>,
    write: Option<&Path>,
    placing: &Placing,
) -> Result<()> {
    placing.check()?;
    if placing.is_given() && write.is_none() {
        return Err("`--section` and `--heading` say where `--write` puts the tables".to_owned());
    }
    let section = pr_section(runs, pr);
    println!("## {PR_SECTION}\n\n{section}");
    if let Some(file) = write {
        let (name, heading) = placing.resolve(PR_SECTION);
        write_sections(
            file,
            &[Section {
                name,
                heading,
                body: section,
            }],
        )?;
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
                ("pr-size-linux".to_owned(), Some(240)),
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
            .map(|index| case("cineris-base", &format!("t::quick_{index}"), 0.004))
            .collect();
        cases.push(case("cineris-base", "t::slow", unit));
        cases.push(case("cineris-core::it", "a::b", integration));
        cases.push(case("cineris-cli::system", "sys_c", system));
        cases
    }

    #[test]
    fn a_job_blocks_unless_it_is_of_a_kind_that_blocks_nothing() {
        assert!(!blocks("pr-size-linux"));
        assert!(!blocks("pr-size-windows"));
        assert!(!blocks("pr-size"));
        assert!(blocks("pr-sizes"));
        assert!(!blocks("ts-client"));
        assert!(blocks("lint-linux"));
        assert!(blocks("test-windows"));
        assert!(blocks("build-windows"));
        assert!(blocks("test-windows-tiers"));
        assert!(blocks("test-windows-system"));
        assert!(blocks("a-job-not-known-yet"));
    }

    #[test]
    fn a_split_runs_windows_jobs_are_in_the_order_they_run() {
        let mut run = info(3, "aaaaaaa1111", "worktree-x");
        run.jobs = [
            "test-windows-system",
            "pr-size-windows",
            "test-linux",
            "test-windows-tiers",
            "build-windows",
            "lint-windows",
            "lint-linux",
        ]
        .iter()
        .map(|name| ((*name).to_owned(), Some(60)))
        .collect();
        let text = pr_section(
            &[Measured {
                info: run,
                linux: None,
                windows: None,
            }],
            None,
        );
        let rows: Vec<&str> = text
            .lines()
            .filter_map(|line| line.strip_prefix("| `")?.split('`').next())
            .collect();
        assert_eq!(
            rows,
            [
                "lint-linux",
                "test-linux",
                "lint-windows",
                "build-windows",
                "test-windows-tiers",
                "test-windows-system",
                "pr-size-windows"
            ],
            "{text}"
        );
    }

    /// Every job of `ci.yml` is in one of the two lists, so that a job
    /// added to the workflow is placed in `ci-times`' table on purpose.
    #[test]
    fn every_job_of_the_ci_workflow_is_known_as_blocking_or_not() {
        let workflow =
            std::fs::read_to_string(crate::workspace::root().join(".github/workflows/ci.yml"))
                .unwrap();
        let after_jobs = workflow
            .split_once("\njobs:\n")
            .map(|(_, rest)| rest)
            .unwrap();
        let ids: Vec<&str> = after_jobs
            .lines()
            .filter_map(|line| line.strip_prefix("  ")?.strip_suffix(':'))
            .filter(|id| !id.starts_with([' ', '#']))
            .collect();
        assert!(ids.contains(&"pr-size"), "{ids:?}");
        assert!(ids.contains(&"ts-client"), "{ids:?}");
        for id in ids {
            assert!(
                BLOCKING.contains(&id) || NON_BLOCKING.contains(&id),
                "the `ci` workflow's job `{id}` is in neither records::BLOCKING nor \
                 records::NON_BLOCKING"
            );
        }
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
        // Linux's jobs first, a job that did not end as a dash, and the job
        // that blocks nothing last, marked.
        assert!(
            text.contains(
                "| `lint-linux` | 44 s | 44 s |\n| `test-linux` | — | — |\n| `test-windows` | 190 \
                 s | 190 s |\n| `pr-size-linux` (blocks nothing) | 240 s | 240 s |\n"
            ),
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
            text.contains(
                "| Tier 1, unit (20 s on Linux, 25 s on Windows) | 1.0 s | 1.5 s | 2.0 s |"
            ),
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
            ..Sides::default()
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
            section.contains("| 1, unit | 1.5 s (101 tests) | 1.0 s (101) | +0.5 s | 25 s |"),
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
            ..Sides::default()
        };
        assert!(complete_rounds(ended).is_none());
        // It ended in the third: two rounds are compared.
        let partly = Sides {
            base: vec![round(), round(), round()],
            head: vec![round(), round()],
            ..Sides::default()
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

    /// A page of an older step: no marks, a hand-written section of a
    /// dated heading, and a table in it.
    const OLD_PAGE: &str = "# After step 18: the map\n\n## On the pull request\n\nBy hand.\n\n## After the independent review: the fixes' pull request (#56)\n\nMeasured on 2026-11-15.\n\n| Job | Main |\n|---|---|\n| `lint-linux` | 50 s |\n";

    fn named(page: &str, name: &str, heading: &str, body: &str) -> String {
        with_named_section(page, name, heading, body).unwrap()
    }

    #[test]
    fn a_named_section_is_added_at_the_end_with_marks_that_carry_its_name() {
        let written = named(OLD_PAGE, "fixes", "The fixes, 2026-11-16", "Table one.\n");
        let (open, close) = named_marks("fixes");
        assert_eq!(
            written,
            format!("{OLD_PAGE}\n## The fixes, 2026-11-16\n\n{open}\nTable one.\n\n{close}\n")
        );
        // The older sections are as they were, byte for byte.
        assert!(written.starts_with(OLD_PAGE), "{written}");
        assert!(open.contains("section \"fixes\""), "{open}");
        // A page that ends without a line break gets the same.
        assert_eq!(
            named(
                OLD_PAGE.trim_end(),
                "fixes",
                "The fixes, 2026-11-16",
                "Table one.\n"
            ),
            written
        );
    }

    #[test]
    fn writing_a_named_section_again_replaces_only_it_wherever_it_has_gone() {
        let first = named(OLD_PAGE, "pr", "On the pull request, again", "PR one.");
        let both = named(&first, "fixes", "The fixes", "Fixes one.");
        // Each is replaced alone; the other, the notes and the older
        // sections stay.
        let noted = both.replace(
            "\n## The fixes\n",
            "\nWhat the first runs found.\n\n## The fixes\n",
        );
        let again = named(&noted, "pr", "ignored: the section is there", "PR two.");
        assert_eq!(again, noted.replace("PR one.", "PR two."));
        let again = named(&again, "fixes", "ignored", "Fixes two.");
        assert_eq!(
            again,
            noted
                .replace("PR one.", "PR two.")
                .replace("Fixes one.", "Fixes two.")
        );
        // Written twice, it is the same page.
        assert_eq!(named(&again, "fixes", "ignored", "Fixes two."), again);
        // The heading may be edited by hand, and the section moved: the
        // marks are what finds it.
        let edited = again.replace("## The fixes", "## The fixes' pull request (#56)");
        let replaced = named(&edited, "fixes", "The fixes", "Fixes three.");
        assert_eq!(replaced, edited.replace("Fixes two.", "Fixes three."));
        assert_eq!(replaced.matches("The fixes").count(), 1, "{replaced}");
    }

    #[test]
    fn a_named_section_never_writes_over_what_it_did_not_write() {
        // The older page's hand-written heading, with no marks.
        let error = with_named_section(
            OLD_PAGE,
            "fixes",
            "After the independent review: the fixes' pull request (#56)",
            "x",
        )
        .unwrap_err();
        assert!(
            error.contains("is already there, and ci-times did not write it"),
            "{error}"
        );
        assert!(error.contains("--heading"), "{error}");
        // A heading the plain marks wrote is not a named section's.
        let plain = with_section("# T\n", PR_SECTION, "plain").unwrap();
        assert!(with_named_section(&plain, PR_SECTION, PR_SECTION, "x").is_err());
        // A heading inside a fenced block is not one, and nor is a mark.
        let (open, close) = named_marks("n");
        let fenced = format!("# T\n\n```text\n## Fixes\n{open}\n```\n");
        let written = named(&fenced, "n", "Fixes", "body");
        assert!(written.starts_with(&fenced), "{written}");
        assert_eq!(written.matches(&open).count(), 2, "{written}");
        assert!(
            written.ends_with(&format!("{open}\nbody\n\n{close}\n")),
            "{written}"
        );
    }

    #[test]
    fn marks_that_are_not_one_pair_in_order_are_refused_and_nothing_is_guessed() {
        let (open, close) = named_marks("n");
        for (page, expected) in [
            (format!("# T\n\n{open}\nx\n"), "1 opening and 0 closing"),
            (format!("# T\n\n{close}\n"), "0 opening and 1 closing"),
            (
                format!("# T\n\n{open}\n{open}\nx\n{close}\n"),
                "2 opening and 1 closing",
            ),
            (format!("# T\n\n{close}\nx\n{open}\n"), "out of order"),
        ] {
            let error = with_named_section(&page, "n", "N", "y").unwrap_err();
            assert!(error.contains(expected), "{page:?}: {error}");
        }
    }

    #[test]
    fn a_placing_decides_the_name_and_the_heading_and_refuses_text_it_could_not_find_again() {
        let placing = |section: Option<&str>, heading: Option<&str>| Placing {
            section: section.map(str::to_owned),
            heading: heading.map(str::to_owned),
        };
        assert_eq!(
            placing(None, None).resolve("Default"),
            (None, "Default".to_owned())
        );
        assert_eq!(
            placing(Some("fixes"), None).resolve("Default"),
            (Some("fixes".to_owned()), "fixes".to_owned())
        );
        assert_eq!(
            placing(Some("fixes"), Some("The fixes, 2026-11-16")).resolve("Default"),
            (Some("fixes".to_owned()), "The fixes, 2026-11-16".to_owned())
        );
        // A heading alone is the name too: an older page's way in.
        assert_eq!(
            placing(None, Some("Dated")).resolve("Default"),
            (Some("Dated".to_owned()), "Dated".to_owned())
        );
        assert!(!placing(None, None).is_given() && placing(None, Some("x")).is_given());
        for (given, expected) in [
            (placing(Some(""), None), "empty"),
            (placing(Some(" x"), None), "white space"),
            (placing(Some("a\nb"), None), "one line"),
            (placing(Some("a --> b"), None), "one line"),
            (placing(Some("a\"b"), None), "double quote"),
            (placing(None, Some("## x")), "the `## ` before it"),
            (placing(None, Some("x\n")), "white space"),
        ] {
            let error = given.check().unwrap_err();
            assert!(error.contains(expected), "{given:?}: {error}");
        }
        assert!(
            placing(Some("The review's fixes"), Some("A: b (#56)"))
                .check()
                .is_ok()
        );
    }

    #[test]
    fn a_new_pages_heading_is_the_one_its_place_asks_for() {
        for (path, heading) in [
            ("docs/steps/24/ci.md", "# After step 24: its CI times"),
            ("docs/steps/13a/ci.md", "# After step 13a: its CI times"),
            (
                "docs/steps/tooling-12/ci.md",
                "# After tooling-12: its CI times",
            ),
            (
                "docs/steps/24/after-review/pr2-trust.md",
                "# Step 24, pr2 trust: its CI times",
            ),
            (
                "docs/steps/tooling-12/parts/b-rebase-and-git.md",
                "# Tooling-12, b rebase and git: its CI times",
            ),
            ("notes/times.md", "# CI times"),
            ("docs/steps/24/answered.md", "# CI times"),
        ] {
            let mut file = std::path::PathBuf::new();
            for part in path.split('/') {
                file.push(part);
            }
            assert_eq!(new_page_heading(&file), heading, "{path}");
        }
    }

    /// A step's first write of its times: the page and its folder are not
    /// there, and are made, with a heading the records' rule takes and the
    /// section. An empty page gets its heading too; a page with no such
    /// section gets the section, and keeps what it had.
    #[test]
    fn a_write_makes_a_missing_page_and_adds_a_missing_section() {
        let root = std::env::temp_dir().join(format!("xtask-ci-times-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let file = root.join("docs").join("steps").join("31").join("ci.md");
        let plain = |body: &str| Section {
            name: None,
            heading: PR_SECTION.to_owned(),
            body: body.to_owned(),
        };
        write_sections(&file, &[plain("The tables.")]).unwrap();
        let made = std::fs::read_to_string(&file).unwrap();
        assert!(
            made.starts_with("# After step 31: its CI times\n\n## On the pull request\n\n"),
            "{made}"
        );
        assert!(made.contains("The tables."), "{made}");

        std::fs::write(&file, "\n").unwrap();
        write_sections(&file, &[plain("Again.")]).unwrap();
        let titled = std::fs::read_to_string(&file).unwrap();
        assert!(
            titled.starts_with("# After step 31: its CI times\n"),
            "{titled}"
        );

        let page = "# After step 31: the thing\n\nWhat it built.\n";
        std::fs::write(&file, page).unwrap();
        write_sections(&file, &[plain("Third.")]).unwrap();
        let added = std::fs::read_to_string(&file).unwrap();
        assert!(added.starts_with(page.trim_end()), "{added}");
        assert!(
            added.contains("## On the pull request\n\n<!-- written by cargo xtask ci-times"),
            "{added}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_write_into_an_older_page_adds_its_sections_and_changes_nothing_else() {
        let file = std::env::temp_dir().join(format!("xtask-ci-times-{}.md", std::process::id()));
        std::fs::write(&file, OLD_PAGE).unwrap();
        let sections = |body: &str| {
            vec![Section {
                name: Some("fixes".to_owned()),
                heading: "The fixes".to_owned(),
                body: body.to_owned(),
            }]
        };
        write_sections(&file, &sections("One.")).unwrap();
        let first = std::fs::read_to_string(&file).unwrap();
        assert!(first.starts_with(OLD_PAGE), "{first}");
        write_sections(&file, &sections("Two.")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            first.replace("One.", "Two.")
        );
        // All or none: the second of two that is refused leaves the file.
        let before = std::fs::read_to_string(&file).unwrap();
        let both = vec![
            Section {
                name: Some("other".to_owned()),
                heading: "Other".to_owned(),
                body: "x".to_owned(),
            },
            Section {
                name: Some("by-hand".to_owned()),
                heading: "On the pull request".to_owned(),
                body: "y".to_owned(),
            },
        ];
        let error = write_sections(&file, &both).unwrap_err();
        assert!(error.contains("Nothing was written"), "{error}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
        let _ = std::fs::remove_file(file);
    }

    fn compared_of(os: Os, rounds: usize) -> Compared {
        let sides = Sides {
            base: (0..rounds).map(|_| cases(1.004, 2.004, 3.004)).collect(),
            head: (0..rounds).map(|_| cases(1.504, 1.004, 3.004)).collect(),
            numbers: (1..=rounds).collect(),
            left_out: vec!["round 9: head failed: a b".to_owned()],
            skipped: Vec::new(),
            one_sided: Vec::new(),
        };
        compare(os, &sides)
    }

    #[test]
    fn an_abs_sections_are_one_for_each_os_or_one_named_with_a_part_for_each() {
        let compared = [compared_of(Os::Windows, 3), compared_of(Os::Linux, 3)];
        // As ever: a section a heading, between the plain marks.
        let plain = ab_sections(43, "main", &compared, &[], &Placing::default());
        let headings: Vec<&str> = plain.iter().map(|each| each.heading.as_str()).collect();
        assert_eq!(
            headings,
            [
                "Windows, both trees on one runner",
                "Linux, both trees on one runner"
            ]
        );
        assert!(plain.iter().all(|each| each.name.is_none()));
        // Named: one section, the OSes as parts of it.
        let placing = Placing {
            section: Some("fixes".to_owned()),
            heading: Some("The fixes' A/B".to_owned()),
        };
        let named = ab_sections(43, "main", &compared, &[], &placing);
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].name.as_deref(), Some("fixes"));
        assert_eq!(named[0].heading, "The fixes' A/B");
        let body = &named[0].body;
        assert!(
            body.starts_with(
                "### Windows, both trees on one runner\n\n`try` with `-f against=main`"
            ),
            "{body}"
        );
        assert!(
            body.contains("\n\n### Linux, both trees on one runner\n\n"),
            "{body}"
        );
    }

    #[test]
    fn an_os_left_out_of_an_ab_is_written_as_its_note() {
        let compared = [compared_of(Os::Linux, 3)];
        let notes = [(
            Os::Windows,
            "run 43: Windows has no round that both sides finished".to_owned(),
        )];
        let plain = ab_sections(43, "main", &compared, &notes, &Placing::default());
        assert_eq!(plain.len(), 2);
        assert_eq!(plain[0].heading, "Windows, both trees on one runner");
        assert_eq!(
            plain[0].body,
            "run 43: Windows has no round that both sides finished\n"
        );
        assert_eq!(plain[1].heading, "Linux, both trees on one runner");
        let placing = Placing {
            section: Some("fixes".to_owned()),
            heading: Some("The fixes' A/B".to_owned()),
        };
        let named = ab_sections(43, "main", &compared, &notes, &placing);
        assert!(
            named[0].body.starts_with(
                "### Windows, both trees on one runner\n\nrun 43: Windows has no round that \
                 both sides finished\n\n### Linux"
            ),
            "{}",
            named[0].body
        );
    }

    #[test]
    fn what_moved_in_an_ab_is_each_tiers_crates_and_tests_with_the_rounds_left_out() {
        let one = compared_of(Os::Windows, 3);
        let text = ab_detail_text(&one, "main", 3);
        assert!(
            text.contains("Each round, each tier's time above the start"),
            "{text}"
        );
        assert!(
            text.contains("Rounds left out of the comparison, with the other side's of the same round:\n  round 9: head failed: a b\n"),
            "{text}"
        );
        assert!(
            text.contains("What moved, the PR against main, each side's median of 3 runs:"),
            "{text}"
        );
        // Unit: cineris-base's slow test moved 0.5 s; integration: 1 s faster.
        assert!(text.contains("tier 1, unit (time above start)"), "{text}");
        assert!(
            text.contains("cineris-base  t::slow  (1.000 -> 1.500 s)"),
            "{text}"
        );
        assert!(
            text.contains("cineris-core::it  a::b  (2.000 -> 1.000 s)"),
            "{text}"
        );
        // System tests did not move: no tier 3 part.
        assert!(!text.contains("tier 3"), "{text}");
        // `--top 0` is no detail and no rounds' table, and keeps the list
        // of rounds left out.
        let none = ab_detail_text(&one, "main", 0);
        assert!(none.contains("round 9"), "{none}");
        assert!(
            !none.contains("What moved") && !none.contains("Each round"),
            "{none}"
        );
    }

    /// A folder of the test's own, emptied.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-records-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn report_with(failure: bool) -> String {
        let failed = if failure {
            "<failure message=\"boom\" type=\"test failure with exit code 101\">no</failure>"
        } else {
            ""
        };
        format!(
            "<testsuites><testsuite name=\"cineris-base\"><testcase name=\"t::a\" classname=\"cineris-base\" time=\"0.010\">{failed}</testcase></testsuite></testsuites>"
        )
    }

    #[test]
    fn a_failed_round_or_a_lone_report_is_left_out_with_its_reason_and_the_rest_keep_their_numbers()
    {
        let dir = scratch("sides");
        let mut files = Vec::new();
        let mut keep = |name: &str, failure: bool| {
            let path = dir.join("runner").join("target").join("ab").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, report_with(failure)).unwrap();
            files.push(path);
        };
        for round in 1..=5 {
            keep(&format!("base-round-{round}.xml"), round == 3);
            // Round 4's head report never came.
            if round != 4 {
                keep(&format!("head-round-{round}.xml"), round == 1);
            }
        }
        // Not a round's report.
        std::fs::write(dir.join("try-output.log"), "log").unwrap();
        files.push(dir.join("try-output.log"));

        let sides = sides_of(7, "try-windows", &files).unwrap();

        // Rounds 1 and 3 failed only by a test, on one side: compared
        // without it, as `ab` compares them, and the test named as a
        // failure. Round 4 kept no report on one side. The rest keep their
        // numbers.
        assert_eq!(sides.numbers, [1, 2, 3, 5]);
        assert_eq!((sides.base.len(), sides.head.len()), (4, 4));
        assert_eq!(sides.left_out, ["round 4: head kept no report"]);
        assert_eq!(sides.one_sided, ["cineris-base t::a"]);
        assert!(sides.skipped.is_empty(), "{:?}", sides.skipped);
        for cases in sides.base.iter().chain(&sides.head) {
            assert!(
                !cases.iter().any(|case| case.id() == "cineris-base t::a"),
                "the failed test is out of every round"
            );
        }
        let none = sides_of(7, "try-windows", &files[..0]).unwrap_err();
        assert!(none.contains("holds no A/B reports"), "{none}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A report of `t::a` (failed or not) and `t::b`, which passed.
    fn report_of_two(a_failed: bool) -> String {
        let failed = if a_failed {
            "<failure message=\"boom\" type=\"test failure with exit code 101\">no</failure>"
        } else {
            ""
        };
        format!(
            "<testsuites><testsuite name=\"cineris-base\"><testcase name=\"t::a\" classname=\"cineris-base\" time=\"0.010\">{failed}</testcase><testcase name=\"t::b\" classname=\"cineris-base\" time=\"0.020\"/></testsuite></testsuites>"
        )
    }

    #[test]
    fn a_test_that_failed_on_both_sides_is_left_out_of_every_round_and_the_rounds_compared() {
        let dir = scratch("both-sides");
        let mut files = Vec::new();
        let keep = |files: &mut Vec<PathBuf>, name: &str, text: String| {
            let path = dir.join("target").join("ab").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            files.push(path);
        };
        // `t::a` fails on both sides in rounds 1 and 2, and passes in 3.
        for round in 1..=3 {
            keep(
                &mut files,
                &format!("base-round-{round}.xml"),
                report_of_two(round < 3),
            );
            keep(
                &mut files,
                &format!("head-round-{round}.xml"),
                report_of_two(round < 3),
            );
        }
        let sides = sides_of(7, "try-windows", &files).unwrap();
        assert_eq!(sides.numbers, [1, 2, 3], "every round is compared");
        assert_eq!(sides.left_out, Vec::<String>::new());
        assert_eq!(sides.skipped, ["cineris-base t::a"]);
        for cases in sides.base.iter().chain(&sides.head) {
            let ids: Vec<String> = cases.iter().map(TestCase::id).collect();
            assert_eq!(ids, ["cineris-base t::b"], "t::a is out of every round");
        }

        assert!(sides.one_sided.is_empty());

        // Failed on one side only in another round: that round is compared
        // too, t::a is out of every round, and it is named as a test that
        // failed on one side, no longer as one both sides failed.
        keep(&mut files, "base-round-4.xml", report_of_two(true));
        keep(&mut files, "head-round-4.xml", report_of_two(false));
        let sides = sides_of(7, "try-windows", &files).unwrap();
        assert_eq!(sides.numbers, [1, 2, 3, 4]);
        assert_eq!(sides.left_out, Vec::<String>::new());
        assert_eq!(sides.one_sided, ["cineris-base t::a"]);
        assert!(sides.skipped.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_round_whose_run_broke_is_left_out_whatever_its_report_holds() {
        let dir = scratch("broke");
        let mut files = Vec::new();
        let mut keep = |name: &str| {
            let path = dir.join("target").join("ab").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            // A report with no failed test: the run broke after tiers 1
            // and 2 (its system tests never ran).
            std::fs::write(&path, report_with(false)).unwrap();
            files.push(path);
        };
        for round in 1..=2 {
            keep(&format!("base-round-{round}.xml"));
        }
        keep("head-round-1.xml");
        keep(&crate::ab::kept_report_name(false, 2, true));
        let sides = sides_of(7, "try-windows", &files).unwrap();
        assert_eq!(sides.numbers, [1]);
        assert_eq!(sides.left_out.len(), 1, "{:?}", sides.left_out);
        assert!(
            sides.left_out[0].starts_with("round 2: the head run broke"),
            "{:?}",
            sides.left_out
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_rounds_that_were_compared_keep_their_numbers_in_the_rounds_table() {
        let sides = Sides {
            base: vec![cases(1.004, 2.004, 3.004), cases(1.004, 2.004, 3.004)],
            head: vec![cases(1.504, 2.004, 3.004), cases(1.504, 2.004, 3.004)],
            numbers: vec![1, 4],
            left_out: vec![],
            skipped: Vec::new(),
            one_sided: Vec::new(),
        };
        let one = compare(Os::Linux, &sides);
        assert_eq!(one.numbers, [1, 4]);
        let table = rounds_of(&one);
        let rounds: Vec<&str> = table
            .lines()
            .filter(|line| line.contains("base") || line.contains("head"))
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        assert!(rounds.contains(&"4") && !rounds.contains(&"2"), "{table}");
    }

    /// A split run's tier 3 is held above its own job's start, and the
    /// column says both.
    #[test]
    fn a_split_runs_column_says_tier_3s_own_start_and_holds_it_above_it() {
        let mut windows = cases(2.004, 4.004, 6.004);
        for case in windows
            .iter_mut()
            .filter(|case| case.name.starts_with("sys_"))
        {
            case.start = Some(1.004);
        }
        let text = pr_section(
            &[Measured {
                info: info(4, "bbbbbbb2222", "worktree-x"),
                linux: None,
                windows: Some(windows),
            }],
            None,
        );
        assert!(
            text.contains("Windows, `bbbbbbb` (4 ms; tier 3 1004 ms) |"),
            "{text}"
        );
        // 6.004 s above 1.004 s, not above 4 ms.
        assert!(
            text.contains("| Tier 3, system (60 s on Linux, 90 s on Windows) | 5.0 s |"),
            "{text}"
        );
    }
}
