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
//!   skipped;
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

Prints what a `try` run found, from its artifacts: for each OS the run had, an
A/B run's tables (tiers, each round, the crates and tests that moved, the rounds
left out, the tests it skipped), a `test` run's tiers against the budgets, or a
stress run's summary; then each failed test with the first lines of its message,
and for a job that failed with no failed test in a report, the last lines of its
log. It exits with an error naming what failed when the run did.
--top N is the tests a tier lists (8 when not given), --tail N the lines of a
log (30).";

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
}

impl Request {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut run = None;
        let mut request = Self {
            run: 0,
            os: None,
            top: TOP,
            tail: TAIL,
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
/// (`CORTEX_TEST_FIRST_SEED=<n> cargo nextest …`); 0 when it shows none.
pub(crate) fn first_seed_of(log: &str) -> u64 {
    log.find("CORTEX_TEST_FIRST_SEED=")
        .and_then(|at| {
            let digits: String = log[at + "CORTEX_TEST_FIRST_SEED=".len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        })
        .unwrap_or(0)
}

/// How many lines a block of skipped tests is read for at most.
const SKIPPED_LINES: usize = 40;

/// The tests an A/B run skipped, as its log lists them: a run compares
/// the tests that passed on one side at least, and names the ones it
/// leaves out. What it writes is not fixed here, so this reads a block: a
/// line that says `skipped` and names both sides, and the indented lines
/// after it. Nothing when the log has no such line, and a log that has one
/// shows it whatever it is called.
pub(crate) fn skipped_block(lines: &[String]) -> Option<String> {
    let at = lines.iter().position(|line| {
        let lower = line.to_lowercase();
        lower.contains("skipped") && lower.contains("both sides")
    })?;
    let mut block = vec![lines[at].trim_end().to_owned()];
    for line in lines[at + 1..].iter().take(SKIPPED_LINES) {
        if line.trim().is_empty() || !line.starts_with(char::is_whitespace) {
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
}

/// Which of a `try` artifact's files is which.
struct Kept {
    log: Option<PathBuf>,
    ci: Option<PathBuf>,
    stress: Option<PathBuf>,
    perf: Option<PathBuf>,
    ab: Vec<PathBuf>,
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
        ab: files
            .iter()
            .filter(|file| {
                file.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| runs::ab_report_name(name).is_some())
            })
            .cloned()
            .collect(),
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
        for ((round, side), path) in by_report {
            failures_in(
                &read_text(path)?,
                &format!("{side} round {round}"),
                &mut failed,
            );
        }
    }
    if let Some(path) = &kept.ci {
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
    if let Some(path) = &kept.perf {
        reported = true;
        let xml = read_text(path)?;
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
        failures_in(&xml, "the performance tests", &mut failed);
    }
    if !reported && directory.is_some() {
        let _ = writeln!(
            result.text,
            "The artifact holds no JUnit report: the command did not run tests, or the job ended \
             before it wrote one.\n"
        );
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
    let status = parse_status(&runs::gh(&[
        "run",
        "view",
        &id,
        "--json",
        "workflowName,status,conclusion,headSha,jobs",
    ])?)?;
    let jobs = chosen_jobs(&status, request.os.as_deref())?;
    if status.status != "completed" {
        return Err(format!(
            "run {run} is {}, not finished: `ci-wait --runs {run}` waits for it",
            if status.status.is_empty() {
                "not completed"
            } else {
                &status.status
            }
        ));
    }
    println!(
        "try run {run} at {}: {}\n",
        status.sha,
        status.conclusion.as_deref().unwrap_or("no conclusion")
    );
    let mut problems = Vec::new();
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
    }
    if problems.is_empty() && status.conclusion.as_deref().is_none_or(|c| c == "success") {
        println!("run {run}: nothing failed");
        return Ok(());
    }
    if problems.is_empty() {
        problems.push(format!(
            "the run ended as {}",
            status.conclusion.as_deref().unwrap_or("no conclusion")
        ));
    }
    Err(format!("try run {run} failed: {}", problems.join("; ")))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Job, Request, Status, against_of, chosen_jobs, first_seed_of, job_result, os_of,
        parse_status, skipped_block,
    };
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
                tail: 9
            }
        );
        assert_eq!(request().top, 8);
        assert_eq!(request().tail, 30);
        for (arguments, expected) in [
            (&[][..], "usage: cargo xtask try-result"),
            (&["x"], "is not a run's id"),
            (&["1", "--os", "macos"], "say `linux`, `windows`"),
            (&["1", "--top"], "needs a number"),
            (&["1", "--top", "many"], "is not a number"),
            (&["1", "--fast"], "`--fast`?"),
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
        let log = "$ CORTEX_TEST_FIRST_SEED=40 cargo nextest run --stress-count 9\n";
        assert_eq!(first_seed_of(log), 40);
        assert_eq!(first_seed_of("no seed"), 0);
    }

    #[test]
    fn skipped_tests_are_read_wherever_the_log_names_them_and_nowhere_else() {
        let lines = strings(&[
            "Summary [  1.2s] 100 tests run: 98 passed, 2 skipped",
            "Tests skipped, failed on both sides:",
            "  cortex-base text::a",
            "  cortex-llm::it replay::b",
            "",
            "round 1 ended",
        ]);
        assert_eq!(
            skipped_block(&lines).as_deref(),
            Some(
                "Tests skipped, failed on both sides:\n  cortex-base text::a\n  cortex-llm::it replay::b"
            )
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
        // Round 2's head failed a test: it is left out, and named.
        let failing = BRANCH.replace(
            "<testcase name=\"replay::quick\" classname=\"cortex-llm::it\" time=\"0.520\"/>",
            "<testcase name=\"replay::quick\" classname=\"cortex-llm::it\" time=\"0.510\"><failure message=\"boom\" type=\"test failure with exit code 101\">assertion failed: left == right</failure></testcase>",
        );
        assert_ne!(failing, BRANCH, "the fixture changed under the test");
        keep(&dir, "work/repo/target/ab/head-round-2.xml", &failing);
        keep(
            &dir,
            "tmp/try-output.log",
            "== building the base (origin/main) ==\nTests skipped, failed on both sides:\n  cortex-base text::a\n\nnext\n",
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
        assert!(text.contains("the median of 2 runs each"), "{text}");
        assert!(text.contains("| Tier | The PR | `origin/main` |"), "{text}");
        assert!(
            text.contains("round 2: head failed: cortex-llm::it replay::quick"),
            "{text}"
        );
        assert!(
            text.contains("What moved, the PR against origin/main"),
            "{text}"
        );
        assert!(text.contains("cortex-llm::it  replay::every_cut"), "{text}");
        assert!(
            text.contains("Tests skipped, failed on both sides:\n  cortex-base text::a"),
            "{text}"
        );
        assert!(
            text.contains("FAILED cortex-llm::it replay::quick  (head round 2)\n  | assertion failed: left == right\n"),
            "{text}"
        );
        assert!(!text.contains("The log's last lines"), "{text}");
        assert_eq!(
            result.problems,
            [
                "`try-windows` ended as failure",
                "`try-windows`: 1 test(s) failed (cortex-llm::it replay::quick)"
            ]
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
            text.contains("FAILED cortex-agent::it loop_::stops_at_the_refused_call"),
            "{text}"
        );
        assert!(
            text.contains("FAILED cortex-agent::it loop_::never_ends"),
            "{text}"
        );
        assert!(text.contains("| timed out (test timeout)"), "{text}");
        assert!(!text.contains("passes_the_second_time  ("), "{text}");
        assert!(
            !text.contains("FAILED cortex-cli::system timing_sensitive"),
            "{text}"
        );
        assert_eq!(
            result.problems,
            [
                "`try-linux` ended as failure",
                "`try-linux`: 2 test(s) failed (cortex-agent::it loop_::stops_at_the_refused_call, cortex-agent::it loop_::never_ends)"
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
            "$ CORTEX_TEST_FIRST_SEED=10 cargo nextest run\n",
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
}
