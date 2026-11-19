//! `cargo xtask try-result RUN`: what a `try` run found, in one step, from
//! its artifacts (`docs/testing-and-ci.md`, "Running an experiment on the
//! runners").
//!
//! A `try` run keeps, for each OS it ran on, the artifact `try-<os>`: the
//! run's log, and whichever JUnit reports its command wrote (the tests'
//! in the `ci` profile, a stress run's, the performance tests', and each
//! round of an A/B run, base and head). Reading what the run found took a
//! download, a search for the paths the runner kept and a comparison by
//! hand, for each OS. This downloads the artifacts once (into
//! `target/xtask-runs/<run>/`, as the other commands do) and prints, for
//! each OS:
//!
//! - an A/B run's tables: each tier on both sides, each round, the crates
//!   and the tests that moved, the rounds left out and any tests the run
//!   skipped, and each side's wall time in each round;
//! - a `test` run's report of tiers against that OS's budgets;
//! - a stress run's summary, as the run printed it;
//! - every failed test with the first lines of its message; and for a job
//!   that failed with no failed test in a report, the log's last lines.
//!
//! It ends as a failure, naming what failed, when the run did.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::Result;
use crate::ci_log;
use crate::junit::{self, TestCase};
use crate::records;
use crate::runs;
use crate::stress;
use crate::tiers::{self, Os};

const USAGE: &str =
    "usage: cargo xtask try-result RUN [--os linux|windows|linux-22.04] [--top N] [--tail N]
                               [--queued-warn MIN]

Prints what a `try` run found, from its artifacts: for each OS the run had, an
A/B run's tables (tiers, each round, the crates and tests that moved, the rounds
left out, the tests it skipped, each side's wall time in each round), a `test`
run's tiers against the budgets, or a
stress run's summary; then each failed test with the first lines of its message,
and for a job that failed with no failed test in a report, the last lines of its
log. It exits with an error naming what failed when the run did.
--top N is the tests a tier lists (8 when not given), --tail N the lines of a
log (30).

A run that has not finished is not read. When it has sat queued (or in
progress with no job started) for longer than --queued-warn MIN minutes (10
when not given; --queued-limit is the same), it says so: GitHub has not given
it a runner.";

/// How many tests of a tier an A/B's detail and a report's list show.
const TOP: usize = 8;

/// How many lines of a log are shown when a job failed with no report.
const TAIL: usize = 30;

/// What `try-result` was asked for.
#[derive(Debug, PartialEq)]
pub(crate) struct Request {
    pub(crate) run: u64,
    /// Which runner to read: `linux`, `windows` or `linux-22.04`.
    pub(crate) os: Option<String>,
    pub(crate) top: usize,
    pub(crate) tail: usize,
    /// Minutes a run may sit with no job started before it is named.
    pub(crate) queued_warn: u64,
}

impl Request {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut run = None;
        let mut request = Self {
            run: 0,
            os: None,
            top: TOP,
            tail: TAIL,
            queued_warn: runs::QUEUED_WARN_MINUTES,
        };
        let mut rest = arguments.iter();
        while let Some(argument) = rest.next() {
            let mut value = |what: &str| {
                rest.next()
                    .ok_or_else(|| format!("`{argument}` needs {what}\n{USAGE}"))
            };
            let number = |text: &str, flag: &str| {
                text.parse::<usize>()
                    .map_err(|_| format!("`{flag} {text}` is not a number\n{USAGE}"))
            };
            match argument.as_str() {
                "--os" => {
                    let name = value("a runner")?;
                    if !["linux", "windows", "linux-22.04"].contains(&name.as_str()) {
                        return Err(format!(
                            "`--os {name}`: say `linux`, `windows` or `linux-22.04`\n{USAGE}"
                        ));
                    }
                    request.os = Some(name.clone());
                }
                "--top" => request.top = number(value("a number")?, "--top")?,
                "--tail" => request.tail = number(value("a number")?, "--tail")?,
                "--queued-warn" | "--queued-limit" => {
                    request.queued_warn =
                        runs::queued_warn_minutes(argument, value("a number of minutes")?)?;
                }
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other if run.is_none() && !other.starts_with('-') => {
                    run = Some(
                        other
                            .parse::<u64>()
                            .map_err(|_| format!("`{other}` is not a run's id\n{USAGE}"))?,
                    );
                }
                other => return Err(format!("`{other}`?\n{USAGE}")),
            }
        }
        request.run = run.ok_or_else(|| USAGE.to_owned())?;
        Ok(request)
    }
}

/// One job of a run, as `gh run view --json jobs` lists it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Job {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) conclusion: Option<String>,
}

/// What `gh run view RUN --json workflowName,status,conclusion,headSha,jobs`
/// says.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Status {
    pub(crate) workflow: String,
    pub(crate) status: String,
    pub(crate) conclusion: Option<String>,
    pub(crate) sha: String,
    pub(crate) jobs: Vec<Job>,
}

pub(crate) fn parse_status(json: &str) -> Result<Status> {
    let value: Value =
        serde_json::from_str(json).map_err(|error| format!("gh run view: {error}"))?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let jobs = value
        .get("jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|job| {
            Some(Job {
                id: job.get("databaseId")?.as_u64()?,
                name: job.get("name")?.as_str()?.to_owned(),
                conclusion: job
                    .get("conclusion")
                    .and_then(Value::as_str)
                    .filter(|conclusion| !conclusion.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect();
    Ok(Status {
        workflow: text("workflowName").unwrap_or_default(),
        status: text("status").unwrap_or_default(),
        conclusion: text("conclusion").filter(|conclusion| !conclusion.is_empty()),
        sha: text("headSha")
            .unwrap_or_default()
            .chars()
            .take(7)
            .collect(),
        jobs,
    })
}

/// Whether every job of the run has ended, with a conclusion: a run whose
/// own status still says otherwise is finished all the same. A run with no
/// job listed has not ended.
pub(crate) fn every_job_ended(status: &Status) -> bool {
    !status.jobs.is_empty() && status.jobs.iter().all(|job| job.conclusion.is_some())
}

/// The jobs of a `try` run the request names: `try-<runner>`, all of them
/// or the one `--os` says. A run of another workflow, or one with no job
/// of that runner, is an error that says what it has.
pub(crate) fn chosen_jobs<'a>(status: &'a Status, os: Option<&str>) -> Result<Vec<&'a Job>> {
    let runner = |job: &Job| job.name.strip_prefix("try-").map(str::to_owned);
    if !status.workflow.is_empty() && status.workflow != "try" {
        return Err(format!(
            "that run is of the `{}` workflow: `try-result` reads runs of `try`. \
             `ci-times` and `ci-log` read the others",
            status.workflow
        ));
    }
    let tried: Vec<&Job> = status
        .jobs
        .iter()
        .filter(|job| runner(job).is_some())
        .collect();
    if tried.is_empty() {
        return Err(format!(
            "that run has no `try-<runner>` job; its jobs are {}",
            names(&status.jobs)
        ));
    }
    let chosen: Vec<&Job> = tried
        .iter()
        .copied()
        .filter(|job| os.is_none_or(|os| runner(job).as_deref() == Some(os)))
        .collect();
    if chosen.is_empty() {
        return Err(format!(
            "that run has no job for `--os {}`; its jobs are {}",
            os.unwrap_or_default(),
            names(&status.jobs)
        ));
    }
    Ok(chosen)
}

fn names(jobs: &[Job]) -> String {
    jobs.iter()
        .map(|job| format!("`{}`", job.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The ref an A/B run compared with, as its log names it: `ab` prints
/// `== building the base (REF) ==`.
pub(crate) fn against_of(log: &str) -> Option<String> {
    log.lines().find_map(|line| {
        let rest = line.split("== building the base (").nth(1)?;
        rest.split(") ==").next().map(str::to_owned)
    })
}

/// The first seed of a stress run, from the command line its log shows
/// (`CINERIS_TEST_FIRST_SEED=<n> cargo nextest …`); 0 when it shows none.
pub(crate) fn first_seed_of(log: &str) -> u64 {
    log.find("CINERIS_TEST_FIRST_SEED=")
        .and_then(|at| {
            let digits: String = log[at + "CINERIS_TEST_FIRST_SEED=".len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        })
        .unwrap_or(0)
}

/// How many lines a block of skipped tests is read for at most.
const SKIPPED_LINES: usize = 40;

/// The tests an A/B run left out because they failed, as `ab` lists them
/// ([`crate::ab::skipped_text`]): the line that says how many `TEST(S)
/// FAILED ON BOTH SIDES`, or `ON ONE SIDE ONLY`, the indented lines after
/// it, and the other kind's block when it follows. Nothing when the log has
/// no such line. `ab` prints the blocks before its table and again after
/// it; the first are taken.
pub(crate) fn skipped_block(lines: &[String]) -> Option<String> {
    let heading = |line: &str| {
        let lower = line.to_lowercase();
        lower.contains("test(s) failed on both sides")
            || lower.contains("test(s) failed on one side only")
    };
    let at = lines.iter().position(|line| heading(line))?;
    let mut block = vec![lines[at].trim_end().to_owned()];
    for line in lines[at + 1..].iter().take(SKIPPED_LINES) {
        if line.trim().is_empty() || !(line.starts_with(char::is_whitespace) || heading(line)) {
            break;
        }
        block.push(line.trim_end().to_owned());
    }
    Some(block.join("\n"))
}

/// A failed test of a report, with where it failed.
#[derive(Debug, PartialEq)]
struct Failed {
    id: String,
    lines: Vec<String>,
    more: usize,
    /// `base round 2`, `head round 3`, or the report's own name.
    wheres: Vec<String>,
}

/// The tests that failed in the report `xml`, each with the first lines of
/// its message. A test that passed on its retry is not failed.
fn failures_in(xml: &str, place: &str, found: &mut Vec<Failed>) {
    let cases = junit::parse(xml);
    for failure in junit::failures(xml) {
        let failed = cases.iter().any(|case: &TestCase| {
            case.failed
                && case.binary == failure.binary
                && case.name == failure.name
                && case.repeat == failure.repeat
        });
        if !failed {
            continue;
        }
        let (lines, all) = stress::failure_lines(&failure);
        let shown = lines.len().min(stress::LINES_SHOWN);
        let id = format!("{} {}", failure.binary, failure.name);
        let more = all.saturating_sub(shown);
        let lines: Vec<String> = lines.into_iter().take(stress::LINES_SHOWN).collect();
        match found
            .iter_mut()
            .find(|each| each.id == id && each.lines == lines)
        {
            Some(each) => {
                if !each.wheres.iter().any(|at| at == place) {
                    each.wheres.push(place.to_owned());
                }
            }
            None => found.push(Failed {
                id,
                lines,
                more,
                wheres: vec![place.to_owned()],
            }),
        }
    }
}

fn failed_text(failed: &[Failed]) -> String {
    let mut text = String::new();
    for each in failed {
        let _ = writeln!(text, "FAILED {}  ({})", each.id, each.wheres.join("; "));
        for line in &each.lines {
            let _ = writeln!(text, "  | {line}");
        }
        if each.more > 0 {
            let _ = writeln!(text, "  | … and {} more line(s)", each.more);
        }
    }
    text
}

/// The last `tail` lines of `lines`, with a note of what was left out.
fn tail_text(lines: &[String], tail: usize) -> String {
    let skipped = lines.len().saturating_sub(tail);
    let mut text = String::new();
    if skipped > 0 {
        let _ = writeln!(text, "(… {skipped} earlier lines left out)");
    }
    for line in &lines[skipped..] {
        let _ = writeln!(text, "{line}");
    }
    text
}

/// What one job found.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct JobResult {
    pub(crate) text: String,
    /// What failed, one line each, for the error the command ends with.
    pub(crate) problems: Vec<String>,
    /// The command the job ran, as its log says (`$ cargo xtask …`).
    pub(crate) command: Option<String>,
}

/// The command a `try` job ran, from its log: `try` prints `$ cargo xtask
/// …` before it runs it (a dispatch's own line is not in a job's log).
pub(crate) fn command_of(lines: &[String]) -> Option<String> {
    lines.iter().find_map(|line| {
        line.trim()
            .strip_prefix("$ cargo xtask ")
            .map(|command| command.trim().to_owned())
    })
}

/// What a `size --against-main` job's log says when it could not read
/// `main`'s size (`  main     not read: …`): the reason. `None` for any other
/// command, or when it read it.
pub(crate) fn unread_main(command: &str, lines: &[String]) -> Option<String> {
    if !command
        .split_whitespace()
        .any(|word| word == "--against-main")
    {
        return None;
    }
    lines.iter().find_map(|line| {
        line.trim_start()
            .strip_prefix("main")?
            .trim_start()
            .strip_prefix("not read:")
            .map(|why| why.trim().to_owned())
    })
}

/// One line for each job a run had: its name, how it ended, and the
/// command it ran; what the run came to, read together, below each job's
/// own sections.
pub(crate) fn summary_text(run: u64, jobs: &[(String, String, Option<String>)]) -> String {
    let width = jobs.iter().map(|(name, ..)| name.len()).max().unwrap_or(0);
    let ended_width = jobs
        .iter()
        .map(|(_, ended, _)| ended.len())
        .max()
        .unwrap_or(0);
    let mut text = format!("==== run {run}, its {} jobs together ====\n\n", jobs.len());
    for (name, ended, command) in jobs {
        let _ = writeln!(
            text,
            "{name:<width$}  {ended:<ended_width$}  {}",
            command.as_deref().map_or_else(
                || "(the command is not in a log)".to_owned(),
                |command| format!("cargo xtask {command}")
            )
        );
    }
    text
}

/// Which of a `try` artifact's files is which.
struct Kept {
    log: Option<PathBuf>,
    ci: Option<PathBuf>,
    stress: Option<PathBuf>,
    perf: Option<PathBuf>,
    /// A performance A/B's results (`perf-ab/results.json`).
    perf_ab: Option<PathBuf>,
    ab: Vec<PathBuf>,
    /// An A/B's wall time of each side in each round (`ab/wall-times.json`).
    walls: Option<PathBuf>,
    /// A job A/B's record of each phase (`ab/job-ab.json`).
    job_ab: Option<PathBuf>,
}

fn sort_files(files: &[PathBuf]) -> Kept {
    let report = |profile: &str| {
        files
            .iter()
            .find(|file| runs::ends_with_names(file, &["nextest", profile, "junit.xml"]))
            .cloned()
    };
    Kept {
        log: files
            .iter()
            .find(|file| {
                file.file_name()
                    .is_some_and(|name| name == "try-output.log")
            })
            .cloned(),
        ci: report("ci"),
        stress: report("stress"),
        perf: report("perf"),
        perf_ab: files
            .iter()
            .find(|file| runs::ends_with_names(file, &crate::perf_ab::RECORD_PATH_NAMES))
            .cloned(),
        ab: files
            .iter()
            .filter(|file| {
                file.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| runs::ab_report_name(name).is_some())
            })
            .cloned()
            .collect(),
        walls: files
            .iter()
            .find(|file| runs::ends_with_names(file, &["ab", crate::ab::WALL_TIMES_FILE]))
            .cloned(),
        job_ab: files
            .iter()
            .find(|file| runs::ends_with_names(file, &["ab", crate::job_ab::RECORD_FILE]))
            .cloned(),
    }
}

fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))
}

/// What one job of a `try` run found: `directory` is its downloaded
/// artifact (none when the job kept none), `job_log` fetches the job's own
/// log from GitHub, for a job that kept no log of its own. `os` decides the
/// budgets a `test` run's tiers are held to.
pub(crate) fn job_result(
    run: u64,
    job: &Job,
    os: Os,
    directory: Option<&Path>,
    job_log: &dyn Fn() -> Result<String>,
    request: &Request,
) -> Result<JobResult> {
    let mut result = JobResult::default();
    let name = &job.name;
    let ended = job.conclusion.as_deref().unwrap_or("not ended");
    if ended != "success" {
        result.problems.push(format!("`{name}` ended as {ended}"));
    }
    let _ = writeln!(result.text, "==== {name} (job {}, {ended}) ====\n", job.id);

    let files = match directory {
        Some(directory) => runs::files_under(directory)?,
        None => Vec::new(),
    };
    let kept = sort_files(&files);
    let log_lines: Option<Vec<String>> = match &kept.log {
        Some(path) => Some(ci_log::log_lines(&read_text(path)?)),
        None => None,
    };
    let log_text = log_lines.as_deref().map(|lines| lines.join("\n"));
    result.command = log_lines.as_deref().and_then(command_of);
    // A size checked against main's that could not read main's: `size`
    // says so and passes, and the check it was asked for did not happen.
    if let (Some(command), Some(lines)) = (&result.command, log_lines.as_deref())
        && let Some(said) = unread_main(command, lines)
    {
        result.problems.push(format!(
            "`{name}`: main's size was not read, so the comparison with it asked for did not \
             happen ({said})"
        ));
    }
    if directory.is_none() {
        let _ = writeln!(
            result.text,
            "The job kept no artifact `{name}`: it ended before it uploaded one.\n"
        );
    }

    let mut failed: Vec<Failed> = Vec::new();
    let mut reported = false;
    if !kept.ab.is_empty() {
        reported = true;
        let sides = records::sides_of(run, name, &files)?;
        let left_out = sides.left_out.clone();
        // Tests that failed on both sides of a round, and nothing else did:
        // `ab` compared the round without them, and they are named in its
        // block below, not as failures of the run.
        let skipped = sides.skipped.clone();
        match records::complete_rounds(sides) {
            Some(sides) => {
                let one = records::compare(os, &sides);
                let against = log_text
                    .as_deref()
                    .and_then(against_of)
                    .unwrap_or_else(|| "base".to_owned());
                let _ = writeln!(
                    result.text,
                    "A/B, the PR against {against}:\n\n{}",
                    records::ab_section(run, &against, &one)
                );
                let _ = writeln!(
                    result.text,
                    "{}",
                    records::ab_detail_text(&one, &against, request.top)
                );
            }
            None => {
                let _ = writeln!(
                    result.text,
                    "No round was finished by both sides, so nothing is compared. {}\n",
                    left_out.join("; ")
                );
            }
        }
        if let Some(block) = log_lines.as_deref().and_then(skipped_block) {
            let _ = writeln!(result.text, "{block}\n");
        }
        // Kept since the tenth tooling step: an older run has none.
        if let Some(path) = &kept.walls {
            let _ = writeln!(result.text, "{}", walls_text(&read_text(path)?));
        }
        let mut by_report: BTreeMap<(usize, String), &PathBuf> = BTreeMap::new();
        for path in &kept.ab {
            if let Some((side, round)) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(runs::ab_report_name)
            {
                by_report.insert((round, side.to_owned()), path);
            }
        }
        let mut in_rounds: Vec<Failed> = Vec::new();
        for ((round, side), path) in by_report {
            failures_in(
                &read_text(path)?,
                &format!("{side} round {round}"),
                &mut in_rounds,
            );
        }
        in_rounds.retain(|one| !skipped.contains(&one.id));
        failed.extend(in_rounds);
    }
    if let Some(path) = &kept.job_ab {
        reported = true;
        let _ = writeln!(
            result.text,
            "{}",
            crate::job_ab::render(&crate::job_ab::from_json(&read_text(path)?)?)
        );
        for ((round, side), path) in crate::job_ab::kept_reports(&files) {
            failures_in(
                &read_text(&path)?,
                &format!("{side} job round {round}"),
                &mut failed,
            );
        }
    }
    // A job A/B leaves the head's last round's report where a test run
    // leaves its own: its rounds' reports are read above, and that one is not
    // the run's tiers.
    if let Some(path) = kept.ci.as_ref().filter(|_| kept.job_ab.is_none()) {
        reported = true;
        let cases = junit::read(path)?;
        let _ = writeln!(
            result.text,
            "Tiers, against {os:?}'s budgets:\n\n{}",
            tiers::report_text(&cases, os, request.top)
        );
        failures_in(&read_text(path)?, "the tests", &mut failed);
    }
    if let Some(path) = &kept.stress {
        reported = true;
        let xml = read_text(path)?;
        let seed = log_text.as_deref().map_or(0, first_seed_of);
        let (summary, failing) =
            stress::report_summary(&xml, "the artifact's nextest/stress/junit.xml", seed);
        let _ = writeln!(result.text, "Stress:\n{summary}");
        if failing > 0 {
            result
                .problems
                .push(format!("`{name}`: {failing} test(s) failed on some repeat"));
        }
    }
    if let Some(path) = &kept.perf_ab {
        reported = true;
        let results = crate::perf_ab::from_json(&read_text(path)?)?;
        let _ = writeln!(result.text, "{}", crate::perf_ab::render(&results));
        let _ = writeln!(
            result.text,
            "Each test's own tables are in the log: `cargo xtask ci-log {run} --grep 'perf_'`.\n"
        );
    }
    if let Some(path) = &kept.perf {
        reported = true;
        let xml = read_text(path)?;
        // A performance A/B's report is of the last run only, the head's.
        if kept.perf_ab.is_none() {
            let mut cases = junit::parse(&xml);
            cases.sort_by_key(TestCase::id);
            let _ = writeln!(result.text, "Performance tests ({} ran):", cases.len());
            for case in &cases {
                let _ = writeln!(
                    result.text,
                    "  {:>8.3} s  {}{}",
                    case.seconds,
                    case.id(),
                    if case.failed { "  FAILED" } else { "" }
                );
            }
            let _ = writeln!(
                result.text,
                "Their own tables are in the log: `cargo xtask ci-log {run} --grep 'perf_'`.\n"
            );
        }
        failures_in(&xml, "the performance tests", &mut failed);
    }
    if !reported && directory.is_some() {
        let _ = writeln!(
            result.text,
            "The artifact holds no JUnit report: the command did not run tests, or the job ended \
             before it wrote one.\n"
        );
        // What a command that runs no tests found (a `size` check, a
        // start-up measurement) is at the end of its log. A job that failed
        // has its log's end below already.
        if ended == "success"
            && let Some(lines) = log_lines.as_deref()
        {
            let _ = writeln!(
                result.text,
                "The log's last lines:\n\n{}",
                tail_text(lines, request.tail)
            );
        }
    }

    if !failed.is_empty() {
        let _ = writeln!(result.text, "{}", failed_text(&failed));
        let ids: Vec<&str> = failed.iter().take(3).map(|each| each.id.as_str()).collect();
        result.problems.push(format!(
            "`{name}`: {} test(s) failed ({}{})",
            failed.len(),
            ids.join(", "),
            if failed.len() > 3 { ", …" } else { "" }
        ));
    }
    // A job that failed and left no failed test to read: the log says why.
    let stress_failed = result
        .problems
        .iter()
        .any(|problem| problem.contains("failed on some repeat"));
    if ended != "success" && failed.is_empty() && !stress_failed {
        let lines = match log_lines {
            Some(lines) => lines,
            None => ci_log::log_lines(&job_log().map_err(|error| {
                format!("`{name}` failed with no log of its own kept, and GitHub's could not be read: {error}")
            })?),
        };
        let _ = writeln!(
            result.text,
            "No failed test in a report. The log's last lines:\n\n{}",
            tail_text(&lines, request.tail)
        );
    }
    Ok(result)
}

/// The table of each side's wall time by round, from the file `ab` kept; a
/// note in its place when the file cannot be read, which is no reason to
/// lose the rest of what the run found.
fn walls_text(kept: &str) -> String {
    match crate::ab::walls_from_json(kept) {
        Ok((label, walls)) => crate::ab::walls_table(&label, &walls),
        Err(error) => format!("(each side's wall time is not shown: {error})\n"),
    }
}

/// The budgets' OS of a `try` job named `try-<runner>`.
fn os_of(name: &str) -> Os {
    if name.contains("windows") {
        Os::Windows
    } else {
        Os::Linux
    }
}

/// `cargo xtask try-result RUN [--os …] [--top N] [--tail N]`.
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    let run = request.run;
    let id = run.to_string();
    let viewed = runs::gh(&[
        "run",
        "view",
        &id,
        "--json",
        "workflowName,status,conclusion,headSha,createdAt,jobs",
    ])?;
    let status = parse_status(&viewed)?;
    let jobs = chosen_jobs(&status, request.os.as_deref())?;
    if status.status != "completed" && every_job_ended(&status) {
        println!(
            "run {run} is {}, but every one of its jobs has ended: read as finished (GitHub \
             can be slow to mark a run complete)",
            if status.status.is_empty() {
                "not marked complete"
            } else {
                &status.status
            }
        );
    } else if status.status != "completed" {
        return Err(unfinished_text(
            run,
            &status.status,
            runs::queued_note(&viewed, run, runs::now_seconds(), request.queued_warn)?,
        ));
    }
    println!(
        "try run {run} at {}: {}\n",
        status.sha,
        status.conclusion.as_deref().unwrap_or("no conclusion")
    );
    let ended = conclusion_read(&status, &jobs, request.os.is_some());
    let mut problems = Vec::new();
    let mut together = Vec::new();
    let several = jobs.len() > 1;
    for job in jobs {
        let directory = runs::artifact_dir(run, &job.name)?;
        let job_id = job.id;
        let fetch = move || {
            runs::gh(&[
                "api",
                &format!("repos/{{owner}}/{{repo}}/actions/jobs/{job_id}/logs"),
            ])
        };
        let result = job_result(
            run,
            job,
            os_of(&job.name),
            directory.as_deref(),
            &fetch,
            &request,
        )?;
        print!("{}", result.text);
        problems.extend(result.problems);
        together.push((
            job.name.clone(),
            job.conclusion
                .clone()
                .unwrap_or_else(|| "not ended".to_owned()),
            result.command,
        ));
    }
    if several {
        print!("{}", summary_text(run, &together));
        println!();
    }
    if problems.is_empty() && ended.is_none() {
        println!("run {run}: nothing failed");
        return Ok(());
    }
    if problems.is_empty()
        && let Some(ended) = ended
    {
        problems.push(ended);
    }
    Err(format!("try run {run} failed: {}", problems.join("; ")))
}

/// What is said of run `run`, which has not finished: its status, how to
/// wait for it, and `queued`, the line that says it has sat queued with no
/// jobs past the limit, when it has.
pub(crate) fn unfinished_text(run: u64, status: &str, queued: Option<String>) -> String {
    let mut text = format!(
        "run {run} is {}, not finished: `gh run watch {run}` waits for it",
        if status.is_empty() {
            "not completed"
        } else {
            status
        }
    );
    if let Some(queued) = queued {
        text.push('\n');
        text.push_str(&queued);
    }
    text
}

/// How what was read ended, when not well: the run's conclusion, or with
/// `--os` the conclusion of the jobs it chose (a run with a failed Windows
/// job is not a failure of its Linux one). `None` when it ended well.
fn conclusion_read(status: &Status, jobs: &[&Job], one_os: bool) -> Option<String> {
    if one_os {
        let failed: Vec<String> = jobs
            .iter()
            .filter(|job| job.conclusion.as_deref() != Some("success"))
            .map(|job| {
                format!(
                    "`{}` ended as {}",
                    job.name,
                    job.conclusion.as_deref().unwrap_or("no conclusion")
                )
            })
            .collect();
        return (!failed.is_empty()).then(|| failed.join("; "));
    }
    match status.conclusion.as_deref() {
        None | Some("success") => None,
        Some(other) => Some(format!("the run ended as {other}")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Job, Request, Status, against_of, chosen_jobs, conclusion_read, every_job_ended,
        first_seed_of, job_result, os_of, parse_status, skipped_block,
    };

    #[test]
    fn one_os_is_judged_by_its_own_job_and_the_whole_run_by_its_conclusion() {
        let status = parse_status(STATUS).unwrap();
        // The run failed, by its Windows job.
        let linux = chosen_jobs(&status, Some("linux")).unwrap();
        assert_eq!(conclusion_read(&status, &linux, true), None);
        let windows = chosen_jobs(&status, Some("windows")).unwrap();
        assert_eq!(
            conclusion_read(&status, &windows, true).as_deref(),
            Some("`try-windows` ended as failure")
        );
        let both = chosen_jobs(&status, None).unwrap();
        assert_eq!(
            conclusion_read(&status, &both, false).as_deref(),
            Some("the run ended as failure")
        );
    }
    use crate::tiers::Os;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// A folder of the test's own, emptied.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("xtask-try-result-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes `text` at `relative` under `root`, as an artifact keeps the
    /// runner's paths.
    fn keep(root: &std::path::Path, relative: &str, text: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    const MAIN: &str = include_str!("../fixtures/tiers/compare-main.xml");
    const BRANCH: &str = include_str!("../fixtures/tiers/compare-branch.xml");

    fn job(name: &str, conclusion: &str) -> Job {
        Job {
            id: 7,
            name: name.to_owned(),
            conclusion: Some(conclusion.to_owned()),
        }
    }

    fn request() -> Request {
        Request::parse(&strings(&["5"])).unwrap()
    }

    /// What `gh run view --json …` prints for a `try` run on both runners.
    const STATUS: &str = r#"{"workflowName":"try","status":"completed","conclusion":"failure","headSha":"0123456789abcdef","jobs":[
        {"databaseId":11,"name":"try-linux","conclusion":"success"},
        {"databaseId":12,"name":"try-windows","conclusion":"failure"}]}"#;

    #[test]
    fn a_run_whose_jobs_have_all_ended_is_finished_whatever_its_status_says() {
        let lagging = parse_status(
            r#"{"workflowName":"try","status":"in_progress","conclusion":"","headSha":"0123456789abcdef","jobs":[
                {"databaseId":1,"name":"try-linux","conclusion":"success"},
                {"databaseId":2,"name":"try-windows","conclusion":"failure"}]}"#,
        )
        .unwrap();
        assert!(every_job_ended(&lagging));
        let running = parse_status(
            r#"{"workflowName":"try","status":"in_progress","conclusion":"","headSha":"0123456789abcdef","jobs":[
                {"databaseId":1,"name":"try-linux","conclusion":"success"},
                {"databaseId":2,"name":"try-windows","conclusion":""}]}"#,
        )
        .unwrap();
        assert!(!every_job_ended(&running), "a job is still running");
        let queued = parse_status(
            r#"{"workflowName":"try","status":"queued","conclusion":"","headSha":"0","jobs":[]}"#,
        )
        .unwrap();
        assert!(!every_job_ended(&queued), "no job yet");
    }

    #[test]
    fn a_wall_times_file_that_cannot_be_read_is_a_note_and_not_a_failure() {
        let text = super::walls_text("{ not json");
        assert!(
            text.starts_with("(each side's wall time is not shown: the wall times are not JSON"),
            "{text}"
        );
        let whole = crate::ab::walls_json(
            "main",
            &[crate::ab::Wall {
                round: 1,
                seconds: [1.0, 2.0],
                passed: [true, true],
            }],
        );
        assert!(super::walls_text(&whole).starts_with("Each round's wall time"));
        // The rest of the job is still read.
        let dir = scratch("ab-walls");
        for (side, xml) in [("base", MAIN), ("head", BRANCH)] {
            keep(&dir, &format!("w/target/ab/{side}-round-1.xml"), xml);
        }
        keep(&dir, "w/target/ab/wall-times.json", "{ broken");
        let result = job_result(
            5,
            &job("try-linux", "success"),
            Os::Linux,
            Some(&dir),
            &|| Err("not asked".to_owned()),
            &request(),
        )
        .unwrap();
        assert!(
            result.text.contains("wall time is not shown"),
            "{}",
            result.text
        );
        assert!(
            result.text.contains("the median of 1 runs"),
            "{}",
            result.text
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unfinished_run_says_so_and_names_a_run_that_sat_queued_too_long() {
        let queued = crate::runs::sat_queued_text(77, "try", 14);
        let text = super::unfinished_text(77, "queued", Some(queued.clone()));
        assert_eq!(
            text,
            format!("run 77 is queued, not finished: `gh run watch 77` waits for it\n{queued}")
        );
        let text = super::unfinished_text(77, "", None);
        assert_eq!(
            text,
            "run 77 is not completed, not finished: `gh run watch 77` waits for it"
        );
    }

    #[test]
    fn a_request_is_read_and_a_wrong_one_refused() {
        assert_eq!(
            Request::parse(&strings(&[
                "77", "--os", "windows", "--top", "3", "--tail", "9"
            ]))
            .unwrap(),
            Request {
                run: 77,
                os: Some("windows".to_owned()),
                top: 3,
                tail: 9,
                queued_warn: 10,
            }
        );
        assert_eq!(request().top, 8);
        assert_eq!(request().tail, 30);
        assert_eq!(
            Request::parse(&strings(&["1", "--queued-limit", "25"]))
                .unwrap()
                .queued_warn,
            25
        );
        for (arguments, expected) in [
            (&[][..], "usage: cargo xtask try-result"),
            (&["x"], "is not a run's id"),
            (&["1", "--os", "macos"], "say `linux`, `windows`"),
            (&["1", "--top"], "needs a number"),
            (&["1", "--top", "many"], "is not a number"),
            (&["1", "--fast"], "`--fast`?"),
            (&["1", "--queued-warn", "0"], "`--queued-warn 0`"),
        ] {
            let error = Request::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn the_jobs_of_a_try_run_are_chosen_by_runner() {
        let status = parse_status(STATUS).unwrap();
        assert_eq!(status.sha, "0123456");
        assert_eq!(status.conclusion.as_deref(), Some("failure"));
        // `linux` is the Ubuntu 24 runner alone, not `linux-22.04`.
        let both_linuxes = parse_status(
            r#"{"workflowName":"try","jobs":[{"databaseId":1,"name":"try-linux"},{"databaseId":2,"name":"try-linux-22.04"}]}"#,
        )
        .unwrap();
        let only = chosen_jobs(&both_linuxes, Some("linux")).unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].name, "try-linux");
        let all = chosen_jobs(&status, None).unwrap();
        assert_eq!(all.len(), 2);
        let windows = chosen_jobs(&status, Some("windows")).unwrap();
        assert_eq!(windows[0].name, "try-windows");
        // The Ubuntu 22.04 runner is not the `linux` one.
        let error = chosen_jobs(&status, Some("linux-22.04")).unwrap_err();
        assert!(error.contains("`try-linux`, `try-windows`"), "{error}");
        // A run of another workflow is not read.
        let other = parse_status(&STATUS.replace("\"try\"", "\"ci\"")).unwrap();
        assert!(
            chosen_jobs(&other, None)
                .unwrap_err()
                .contains("`ci` workflow")
        );
        let no_try =
            parse_status(r#"{"workflowName":"try","jobs":[{"databaseId":1,"name":"lint"}]}"#)
                .unwrap();
        assert!(
            chosen_jobs(&no_try, None)
                .unwrap_err()
                .contains("no `try-<runner>` job")
        );
        let _: Status = status;
    }

    #[test]
    fn the_log_says_what_an_ab_compared_with_and_a_stress_run_began_at() {
        let log = "== building the base (main) ==\n== round 1 of 5: the head (this tree) ==\n";
        assert_eq!(against_of(log).as_deref(), Some("main"));
        assert_eq!(against_of("nothing"), None);
        let log = "$ CINERIS_TEST_FIRST_SEED=40 cargo nextest run --stress-count 9\n";
        assert_eq!(first_seed_of(log), 40);
        assert_eq!(first_seed_of("no seed"), 0);
    }

    #[test]
    fn skipped_tests_are_read_wherever_the_log_names_them_and_nowhere_else() {
        // The block as `ab` writes it.
        let said = crate::ab::skipped_text(&[
            crate::ab::Skipped {
                id: "cineris-base text::a".to_owned(),
                rounds: vec![1, 3],
                base: "boom".to_owned(),
                head: "boom".to_owned(),
                on: crate::ab::FailedOn::Both,
            },
            crate::ab::Skipped {
                id: "cineris-llm::it replay::b".to_owned(),
                rounds: vec![2],
                base: "left".to_owned(),
                head: "right".to_owned(),
                on: crate::ab::FailedOn::Both,
            },
            crate::ab::Skipped {
                id: "cineris-tools::it shell::c".to_owned(),
                rounds: vec![1, 2],
                base: String::new(),
                head: "a Windows-only bug".to_owned(),
                on: crate::ab::FailedOn::OneSide,
            },
        ]);
        let mut lines = strings(&["Summary [  1.2s] 100 tests run: 98 passed, 2 skipped"]);
        lines.extend(said.lines().map(str::to_owned));
        lines.extend(strings(&["", "round 1 ended"]));
        assert_eq!(
            skipped_block(&lines).as_deref(),
            Some(said.trim_end()),
            "the whole of ab's block, and nothing after it"
        );
        // A nextest summary's `skipped` is not the list.
        assert_eq!(skipped_block(&lines[..1]), None);
    }

    #[test]
    fn an_ab_job_prints_its_tables_its_rounds_and_the_tests_that_failed() {
        let dir = scratch("ab");
        // The artifact keeps the runner's paths: the reports are deep.
        for round in [1, 2, 3] {
            keep(
                &dir,
                &format!("work/repo/target/ab/base-round-{round}.xml"),
                MAIN,
            );
            keep(
                &dir,
                &format!("work/repo/target/ab/head-round-{round}.xml"),
                BRANCH,
            );
        }
        // Round 2's head failed a test: the round is compared without it,
        // and it is named as a failure on one side.
        let failing = BRANCH.replace(
            "<testcase name=\"replay::quick\" classname=\"cineris-llm::it\" time=\"0.520\"/>",
            "<testcase name=\"replay::quick\" classname=\"cineris-llm::it\" time=\"0.510\"><failure message=\"boom\" type=\"test failure with exit code 101\">assertion failed: left == right</failure></testcase>",
        );
        assert_ne!(failing, BRANCH, "the fixture changed under the test");
        keep(&dir, "work/repo/target/ab/head-round-2.xml", &failing);
        keep(
            &dir,
            "tmp/try-output.log",
            "== building the base (origin/main) ==\n1 TEST(S) FAILED ON BOTH SIDES, and are left out:\n  cineris-base text::a (round 1)\n\nnext\n",
        );
        let wall = |round: usize, base: f64, head: f64| crate::ab::Wall {
            round,
            seconds: [base, head],
            passed: [true, round != 2],
        };
        keep(
            &dir,
            "work/repo/target/ab/wall-times.json",
            &crate::ab::walls_json(
                "origin/main",
                &[
                    wall(1, 300.0, 310.0),
                    wall(2, 280.0, 200.0),
                    wall(3, 290.0, 305.0),
                ],
            ),
        );

        let result = job_result(
            5,
            &job("try-windows", "failure"),
            Os::Windows,
            Some(&dir),
            &|| Err("the job log is not asked for".to_owned()),
            &request(),
        )
        .unwrap();

        let text = &result.text;
        assert!(text.contains("A/B, the PR against origin/main:"), "{text}");
        assert!(text.contains("the median of 3 runs each"), "{text}");
        assert!(text.contains("| Tier | The PR | `origin/main` |"), "{text}");
        assert!(
            text.contains("TESTS THAT FAILED ON ONE SIDE ONLY")
                && text.contains("  cineris-llm::it replay::quick\n"),
            "{text}"
        );
        assert!(
            text.contains("What moved, the PR against origin/main"),
            "{text}"
        );
        assert!(
            text.contains("cineris-llm::it  replay::every_cut"),
            "{text}"
        );
        assert!(
            text.contains(
                "1 TEST(S) FAILED ON BOTH SIDES, and are left out:\n  cineris-base text::a (round 1)"
            ),
            "{text}"
        );
        assert!(
            text.contains("FAILED cineris-llm::it replay::quick  (head round 2)\n  | assertion failed: left == right\n"),
            "{text}"
        );
        assert!(!text.contains("The log's last lines"), "{text}");
        // Each side's wall time, round by round, from the file `ab` kept.
        assert!(
            text.contains("Each round's wall time") && text.contains("base (origin/main)"),
            "{text}"
        );
        let median = text
            .lines()
            .find(|line| line.starts_with("median"))
            .unwrap_or_else(|| panic!("{text}"));
        assert_eq!(
            median.split_whitespace().collect::<Vec<_>>(),
            ["median", "290.0", "307.5"],
            "{text}"
        );
        assert_eq!(
            result.problems,
            [
                "`try-windows` ended as failure",
                "`try-windows`: 1 test(s) failed (cineris-llm::it replay::quick)"
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_test_that_failed_on_both_sides_of_a_job_that_succeeded_is_no_failure_of_the_run() {
        let dir = scratch("ab-both");
        // `replay::quick` fails on both sides, in both rounds.
        let failing = |xml: &str, time: &str| {
            let failed = xml.replace(
                &format!(
                    "<testcase name=\"replay::quick\" classname=\"cineris-llm::it\" time=\"{time}\"/>"
                ),
                &format!(
                    "<testcase name=\"replay::quick\" classname=\"cineris-llm::it\" time=\"{time}\"><failure message=\"boom\" type=\"test failure with exit code 101\">assertion failed</failure></testcase>"
                ),
            );
            assert_ne!(failed, xml, "the fixture changed under the test");
            failed
        };
        let (base, head) = (failing(MAIN, "0.510"), failing(BRANCH, "0.520"));
        for round in [1, 2] {
            keep(
                &dir,
                &format!("work/repo/target/ab/base-round-{round}.xml"),
                &base,
            );
            keep(
                &dir,
                &format!("work/repo/target/ab/head-round-{round}.xml"),
                &head,
            );
        }
        let result = job_result(
            5,
            &job("try-windows", "success"),
            Os::Windows,
            Some(&dir),
            &|| Err("the job log is not asked for".to_owned()),
            &request(),
        )
        .unwrap();
        // The rounds are compared, as `ab` compared them, and the run, which
        // succeeded, is not called failed for a test `ab` left out.
        assert_eq!(result.problems, Vec::<String>::new(), "{}", result.text);
        assert!(
            result.text.contains("the median of 2 runs each"),
            "{}",
            result.text
        );
        assert!(
            !result.text.contains("FAILED cineris-llm::it replay::quick"),
            "{}",
            result.text
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_job_that_failed_with_no_report_shows_the_end_of_its_log() {
        let dir = scratch("log");
        let log: String = (1..=50).map(|n| format!("line {n}\n")).collect();
        keep(&dir, "tmp/try-output.log", &log);
        let result = job_result(
            5,
            &job("try-linux", "failure"),
            Os::Linux,
            Some(&dir),
            &|| Err("not asked".to_owned()),
            &Request::parse(&strings(&["5", "--tail", "3"])).unwrap(),
        )
        .unwrap();
        assert!(
            result.text.contains("The artifact holds no JUnit report"),
            "{}",
            result.text
        );
        assert!(
            result
                .text
                .contains("(… 47 earlier lines left out)\nline 48\nline 49\nline 50\n"),
            "{}",
            result.text
        );
        assert_eq!(result.problems, ["`try-linux` ended as failure"]);

        // A job that kept no artifact at all: GitHub's own log is read.
        let result = job_result(
            5,
            &job("try-linux", "cancelled"),
            Os::Linux,
            None,
            &|| Ok("2026-11-16T10:00:00.0000000Z step one\n2026-11-16T10:00:01.0000000Z step two\n".to_owned()),
            &request(),
        )
        .unwrap();
        assert!(
            result.text.contains("kept no artifact `try-linux`"),
            "{}",
            result.text
        );
        assert!(result.text.contains("step two"), "{}", result.text);
        assert_eq!(result.problems, ["`try-linux` ended as cancelled"]);
        // And when GitHub's cannot be read either, that is the error.
        let error = job_result(
            5,
            &job("try-linux", "failure"),
            Os::Linux,
            None,
            &|| Err("404".to_owned()),
            &request(),
        )
        .unwrap_err();
        assert!(error.contains("GitHub's could not be read: 404"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_job_that_ran_no_tests_and_passed_shows_the_end_of_its_log_and_its_command() {
        let dir = scratch("size");
        let mut log = String::from("$ cargo xtask size --check --against-main\n");
        log.extend((1..=40).map(|n| format!("size line {n}\n")));
        keep(&dir, "tmp/try-output.log", &log);
        let result = job_result(
            5,
            &job("try-linux", "success"),
            Os::Linux,
            Some(&dir),
            &|| Err("not asked".to_owned()),
            &Request::parse(&strings(&["5", "--tail", "2"])).unwrap(),
        )
        .unwrap();
        assert!(
            result
                .text
                .contains("The log's last lines:\n\n(… 39 earlier lines left out)\nsize line 39\nsize line 40\n"),
            "{}",
            result.text
        );
        assert!(result.problems.is_empty(), "{:?}", result.problems);
        assert_eq!(
            result.command.as_deref(),
            Some("size --check --against-main")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_size_against_main_that_read_no_main_is_a_problem_of_the_run() {
        let dir = scratch("size-main");
        keep(
            &dir,
            "tmp/try-output.log",
            "$ cargo xtask size --check --against-main --target x86_64-unknown-linux-musl\n\
             x86_64-unknown-linux-musl, `dist` profile\n  binary   15006120 bytes\n  \
             main     not read: no run of main kept size-x86_64-unknown-linux-musl\n",
        );
        let result = job_result(
            5,
            &job("try-linux", "success"),
            Os::Linux,
            Some(&dir),
            &|| Err("not asked".to_owned()),
            &request(),
        )
        .unwrap();
        assert_eq!(
            result.problems,
            [
                "`try-linux`: main's size was not read, so the comparison with it asked for did \
              not happen (no run of main kept size-x86_64-unknown-linux-musl)"
            ]
        );
        let lines = strings(&["  main     15006120 bytes (run 1): +0 bytes"]);
        assert_eq!(super::unread_main("size --against-main", &lines), None);
        let lines = strings(&["  main     not read: gone"]);
        assert_eq!(
            super::unread_main("size --check", &lines),
            None,
            "not asked for"
        );
        assert_eq!(
            super::unread_main("size --check --against-main", &lines).as_deref(),
            Some("gone")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_run_of_several_jobs_ends_with_each_jobs_end_and_command_together() {
        let lines = super::summary_text(
            9,
            &[
                (
                    "try-windows".to_owned(),
                    "success".to_owned(),
                    Some("ab --base ../ab-base --label main".to_owned()),
                ),
                (
                    "try-linux-22.04".to_owned(),
                    "failure".to_owned(),
                    Some("ci --tiers --top 20".to_owned()),
                ),
                ("try-linux".to_owned(), "cancelled".to_owned(), None),
            ],
        );
        assert_eq!(
            lines,
            "==== run 9, its 3 jobs together ====\n\n\
             try-windows      success    cargo xtask ab --base ../ab-base --label main\n\
             try-linux-22.04  failure    cargo xtask ci --tiers --top 20\n\
             try-linux        cancelled  (the command is not in a log)\n"
        );
        assert_eq!(
            super::command_of(&strings(&[
                "against main is 0123 (resolved at dispatch)",
                "  $ cargo xtask test -p x  ",
                "$ cargo xtask later",
            ])),
            Some("test -p x".to_owned()),
            "the first, trimmed"
        );
        assert_eq!(super::command_of(&strings(&["$ cargo nextest run"])), None);
    }

    #[test]
    fn a_job_ab_prints_each_phases_medians_and_the_tests_that_failed() {
        use crate::ab::Which;
        use crate::job_ab::{Phase, Record, SideRound, to_json};
        let dir = scratch("job-ab");
        let side = |round, which, build: f64, passed: bool| SideRound {
            round,
            which,
            phases: vec![
                Phase {
                    name: "build".to_owned(),
                    seconds: build,
                    passed: true,
                },
                Phase {
                    name: "tests".to_owned(),
                    seconds: 20.0,
                    passed,
                },
            ],
            error: (!passed).then(|| "tests failed".to_owned()),
        };
        let record = Record {
            label: "main".to_owned(),
            rounds: 2,
            sides: vec![
                side(1, Which::Base, 100.0, true),
                side(1, Which::Head, 80.0, false),
            ],
            stopped: None,
        };
        keep(&dir, "work/repo/target/ab/job-ab.json", &to_json(&record));
        let failing = BRANCH.replace(
            "<testcase name=\"replay::quick\" classname=\"cineris-llm::it\" time=\"0.520\"/>",
            "<testcase name=\"replay::quick\" classname=\"cineris-llm::it\" time=\"0.510\"><failure message=\"boom\" type=\"test failure with exit code 101\">assertion failed</failure></testcase>",
        );
        assert_ne!(failing, BRANCH, "the fixture changed under the test");
        keep(&dir, "work/repo/target/ab/head-job-1.xml", &failing);
        // The head's last round's report, where a test run leaves its own.
        keep(&dir, "work/repo/target/nextest/ci/junit.xml", &failing);
        let result = job_result(
            5,
            &job("try-windows", "failure"),
            Os::Windows,
            Some(&dir),
            &|| Err("not asked".to_owned()),
            &request(),
        )
        .unwrap();
        assert!(
            result
                .text
                .contains("A whole job two ways, the head against main"),
            "{}",
            result.text
        );
        assert!(result.text.contains("-20.0"), "{}", result.text);
        assert!(
            result.text.contains("(head job round 1)"),
            "{}",
            result.text
        );
        assert!(
            !result.text.contains("Tiers, against"),
            "the head's last round is not the run's tiers: {}",
            result.text
        );
        assert!(
            !result.text.contains("the tests)"),
            "its failure is named once, by its round: {}",
            result.text
        );
        assert!(
            !result.text.contains("The artifact holds no JUnit report"),
            "{}",
            result.text
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_test_run_prints_its_tiers_against_the_budgets_of_its_os_and_a_clean_job_nothing_else() {
        let dir = scratch("test");
        keep(&dir, "work/repo/target/nextest/ci/junit.xml", MAIN);
        let result = job_result(
            5,
            &job("try-windows", "success"),
            Os::Windows,
            Some(&dir),
            &|| Err("not asked".to_owned()),
            &request(),
        )
        .unwrap();
        assert!(
            result.text.contains("Tiers, against Windows's budgets"),
            "{}",
            result.text
        );
        assert!(result.text.contains("90 s"), "{}", result.text);
        assert!(result.problems.is_empty(), "{:?}", result.problems);
        assert!(!result.text.contains("FAILED"), "{}", result.text);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_runner_is_held_to_its_oses_budgets_and_a_retried_test_is_not_a_failure() {
        assert_eq!(os_of("try-windows"), Os::Windows);
        assert_eq!(os_of("try-linux"), Os::Linux);
        assert_eq!(os_of("try-linux-22.04"), Os::Linux);

        // The report of the junit module's own tests: a failure, a timeout
        // and a test that passed on its retry.
        let dir = scratch("retry");
        keep(
            &dir,
            "w/target/nextest/ci/junit.xml",
            crate::junit::tests::REPORT,
        );
        let result = job_result(
            5,
            &job("try-linux", "failure"),
            Os::Linux,
            Some(&dir),
            &|| Err("not asked".to_owned()),
            &request(),
        )
        .unwrap();
        let text = &result.text;
        assert!(
            text.contains("FAILED cineris-agent::it loop_::stops_at_the_refused_call"),
            "{text}"
        );
        assert!(
            text.contains("FAILED cineris-agent::it loop_::never_ends"),
            "{text}"
        );
        assert!(text.contains("| timed out (test timeout)"), "{text}");
        assert!(!text.contains("passes_the_second_time  ("), "{text}");
        assert!(
            !text.contains("FAILED cineris-cli::system timing_sensitive"),
            "{text}"
        );
        assert_eq!(
            result.problems,
            [
                "`try-linux` ended as failure",
                "`try-linux`: 2 test(s) failed (cineris-agent::it loop_::stops_at_the_refused_call, cineris-agent::it loop_::never_ends)"
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stress_run_prints_its_summary_and_fails_on_a_test_that_failed() {
        let dir = scratch("stress");
        keep(
            &dir,
            "work/repo/target/nextest/stress/junit.xml",
            include_str!("../fixtures/stress/junit.xml"),
        );
        keep(
            &dir,
            "tmp/try-output.log",
            "$ CINERIS_TEST_FIRST_SEED=10 cargo nextest run\n",
        );
        let result = job_result(
            5,
            &job("try-linux", "failure"),
            Os::Linux,
            Some(&dir),
            &|| Err("not asked".to_owned()),
            &request(),
        )
        .unwrap();
        assert!(result.text.contains("Stress:\n"), "{}", result.text);
        assert!(result.text.contains("FAILED"), "{}", result.text);
        assert!(result.text.contains("seeds: 10"), "{}", result.text);
        assert!(
            result
                .problems
                .iter()
                .any(|p| p.contains("failed on some repeat")),
            "{:?}",
            result.problems
        );
        // The stress summary carries the failures: no second list of them.
        assert!(
            !result.text.contains("The log's last lines"),
            "{}",
            result.text
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A performance A/B run's artifact: the comparison of its measurements
    /// is printed, and the report of the last run alone is not listed as if
    /// it were the run's.
    #[test]
    fn a_performance_ab_run_prints_both_sides_numbers_and_the_difference() {
        let dir = scratch("perf-ab");
        keep(
            &dir,
            "work/repo/target/perf-ab/results.json",
            r#"{"label":"main","profile":"release","packages":["cineris-index"],"rounds":3,
                "measurements":[{"name":"index.warm","unit":"ms",
                "base":[28.0,29.0,28.5],"head":[31.0,32.5,31.5]}]}"#,
        );
        keep(
            &dir,
            "work/repo/target/nextest/perf/junit.xml",
            "<testsuites><testsuite name=\"cineris-index::it\">\
             <testcase name=\"perf::perf_warm\" classname=\"cineris-index::it\" time=\"4.5\"/>\
             </testsuite></testsuites>",
        );
        let result = job_result(
            5,
            &job("try-linux", "success"),
            Os::Linux,
            Some(&dir),
            &|| Err("not asked".to_owned()),
            &request(),
        )
        .unwrap();
        let row = result
            .text
            .lines()
            .find(|line| line.starts_with("index.warm"))
            .unwrap_or_else(|| panic!("{}", result.text));
        assert!(row.contains("28.50 (28.00 to 29.00)"), "{row}");
        assert!(row.contains("31.50 (31.00 to 32.50)"), "{row}");
        assert!(row.contains("+3.00 (+10.5%)"), "{row}");
        assert!(result.text.contains("against main"), "{}", result.text);
        assert!(
            !result.text.contains("Performance tests ("),
            "the last run's times are not the A/B's: {}",
            result.text
        );
        assert!(result.problems.is_empty(), "{:?}", result.problems);
        let _ = std::fs::remove_dir_all(dir);
    }
}
