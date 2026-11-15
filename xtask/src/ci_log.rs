//! `cargo xtask ci-log`: a CI job's log, read through GitHub's jobs-logs
//! API (`docs/testing-and-ci.md`, "A CI run's test times").
//!
//! `gh run view --log-failed` prints nothing for the nightly workflow, and
//! its other forms download a whole run's logs as an archive. This asks for
//! one job's log, which GitHub serves as text, takes the escape codes out of
//! it, and prints all of it, the end of it, or only the lines that match a
//! pattern with a little around them. Nothing here writes to GitHub.
//!
//! The repository is `gh`'s own: `{owner}/{repo}` of the directory it is run
//! in, or `GH_REPO` when that is set, which is how it works through `rx`,
//! whose copy of the tree has no `.git`.

use std::fmt::Write as _;

use serde_json::Value;

use crate::Result;
use crate::pattern::Pattern;

const USAGE: &str = "\
usage: cargo xtask ci-log RUN|JOB [--job NAME] [--all] [--grep RE] [--context N] [--tail N]

Prints a CI job's log, read through gh, with its escape codes taken out.

A RUN id lists the run's jobs and prints the log of each one that failed
(failed, timed out): the log of the jobs whose names contain NAME with
--job, which may be given again, or of every job with --all. A JOB id prints
that job's log. Which an id is, is told by asking GitHub: a run's jobs first,
and a job's log when GitHub knows no such run.

--grep RE prints only the lines that match RE, each with its line number and
N lines (--context, 2 when not given) before and after it, as grep does.
RE is a small regular-expression dialect: literals, `.`, [classes], \\d \\w
\\s, ( ), |, * + ? {n,m}, ^ and $, and `(?i)` first for a match that ignores
case. --tail N prints the last N lines (of what --grep chose, when given).

Through `rx`, whose copy of the tree has no .git, name the repository:
`rx -- env GH_REPO=OWNER/REPO cargo xtask ci-log RUN`.";

/// How much of a job's log `--grep` shows around a line when not told.
const CONTEXT: usize = 2;

/// Which of a run's jobs to print.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    /// The jobs that failed.
    Failed,
    /// Every job.
    All,
    /// The jobs whose names contain one of these, in any case.
    Named(Vec<String>),
}

/// What `cargo xtask ci-log` was asked for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub(crate) id: u64,
    pub(crate) choice: Choice,
    pub(crate) grep: Option<String>,
    pub(crate) context: usize,
    pub(crate) tail: Option<usize>,
}

impl Request {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut id = None;
        let mut names = Vec::new();
        let mut all = false;
        let mut request = Request {
            id: 0,
            choice: Choice::Failed,
            grep: None,
            context: CONTEXT,
            tail: None,
        };
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            let mut value = |what: &str| {
                words
                    .next()
                    .ok_or_else(|| format!("`{word}` needs {what}\n\n{USAGE}"))
            };
            let number = |flag: &str, text: &str| {
                text.parse::<usize>()
                    .map_err(|_| format!("`{flag} {text}` is not a number"))
            };
            match word.as_str() {
                "--job" => names.push(value("a job's name")?.clone()),
                "--all" => all = true,
                "--grep" => request.grep = Some(value("a pattern")?.clone()),
                "--context" => request.context = number("--context", value("a number")?)?,
                "--tail" => request.tail = Some(number("--tail", value("a number")?)?),
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other if id.is_none() && !other.starts_with('-') => {
                    id = Some(
                        other
                            .parse::<u64>()
                            .map_err(|_| format!("`{other}` is not a run's or a job's id"))?,
                    );
                }
                other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
            }
        }
        request.id = id.ok_or_else(|| format!("say which run or job\n\n{USAGE}"))?;
        if all && !names.is_empty() {
            return Err("`--all` and `--job` choose differently: give one".to_owned());
        }
        request.choice = if all {
            Choice::All
        } else if names.is_empty() {
            Choice::Failed
        } else {
            Choice::Named(names)
        };
        if let Some(grep) = &request.grep {
            // A pattern that cannot be read is an error before GitHub is asked.
            Pattern::new(grep)?;
        }
        Ok(request)
    }
}

/// One job of a run, as GitHub lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Job {
    pub(crate) id: u64,
    pub(crate) name: String,
    /// `success`, `failure`, `cancelled`, `skipped`, `timed_out`; none while
    /// the job has not ended.
    pub(crate) conclusion: Option<String>,
}

impl Job {
    /// The job's log is worth reading as a failure's.
    fn failed(&self) -> bool {
        matches!(self.conclusion.as_deref(), Some("failure" | "timed_out"))
    }
}

/// The jobs of `GET /repos/{owner}/{repo}/actions/runs/{id}/jobs`.
pub(crate) fn parse_jobs(json: &str) -> Result<Vec<Job>> {
    let value: Value = serde_json::from_str(json).map_err(|error| format!("the jobs: {error}"))?;
    let jobs = value
        .get("jobs")
        .and_then(Value::as_array)
        .ok_or("the answer has no `jobs`")?;
    Ok(jobs
        .iter()
        .filter_map(|job| {
            Some(Job {
                id: job.get("id")?.as_u64()?,
                name: job.get("name")?.as_str()?.to_owned(),
                conclusion: job
                    .get("conclusion")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect())
}

/// The jobs `choice` names, in the order GitHub listed them. A choice that
/// finds nothing says what there was.
pub(crate) fn choose<'a>(jobs: &'a [Job], choice: &Choice) -> Result<Vec<&'a Job>> {
    let chosen: Vec<&Job> = match choice {
        Choice::All => jobs.iter().collect(),
        Choice::Failed => jobs.iter().filter(|job| job.failed()).collect(),
        Choice::Named(names) => jobs
            .iter()
            .filter(|job| {
                let name = job.name.to_lowercase();
                names
                    .iter()
                    .any(|wanted| name.contains(&wanted.to_lowercase()))
            })
            .collect(),
    };
    if chosen.is_empty() {
        return Err(match choice {
            Choice::Failed => format!(
                "no job of the run failed; its jobs are\n{}\nname one with --job, or take every \
                 job with --all",
                listing(jobs)
            ),
            Choice::Named(names) => format!(
                "no job's name contains {}; the run's jobs are\n{}",
                names
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(" or "),
                listing(jobs)
            ),
            Choice::All => "the run has no jobs".to_owned(),
        });
    }
    Ok(chosen)
}

/// The jobs, one a line: id, conclusion and name.
fn listing(jobs: &[Job]) -> String {
    let mut text = String::new();
    for job in jobs {
        let _ = writeln!(
            text,
            "  {}  {:<10}  {}",
            job.id,
            job.conclusion.as_deref().unwrap_or("not ended"),
            job.name
        );
    }
    text.trim_end().to_owned()
}

/// `text` without its terminal escape codes: CSI sequences (colours, cursor
/// moves, erasing), OSC sequences (titles, links) up to their bell or `ESC \`,
/// and the other two-character escapes. A byte-order mark GitHub puts at the
/// start of a log goes with them.
pub(crate) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{feff}' => {}
            '\u{1b}' => match chars.next() {
                // Parameters, intermediates, and a final byte of `@` to `~`.
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' {
                            // `ESC \` ends it.
                            chars.next_if_eq(&'\\');
                            break;
                        }
                    }
                }
                // `ESC` and one more character (`ESC c`, `ESC 7`).
                _ => {}
            },
            other => out.push(other),
        }
    }
    out
}

/// A log's lines: the escape codes taken out, and no line ending.
pub(crate) fn log_lines(log: &str) -> Vec<String> {
    strip_ansi(log).lines().map(str::to_owned).collect()
}

/// The lines of `lines` that match `pattern`, each with `context` lines
/// either side, as `grep -n -C` prints them: a match as `N:line`, a line
/// around it as `N-line`, and `--` between groups that do not touch. Empty
/// when nothing matches. The count is of the matches.
pub(crate) fn grep(lines: &[String], pattern: &Pattern, context: usize) -> (String, usize) {
    let matched: Vec<usize> = (0..lines.len())
        .filter(|at| pattern.is_match(&lines[*at]))
        .collect();
    let mut text = String::new();
    // The line after the last one printed.
    let mut next = 0;
    for &at in &matched {
        let from = at.saturating_sub(context).max(next);
        let to = (at + context + 1).min(lines.len());
        if next > 0 && from > next {
            text.push_str("--\n");
        }
        for (line, content) in lines.iter().enumerate().take(to).skip(from) {
            let mark = if matched.binary_search(&line).is_ok() {
                ':'
            } else {
                '-'
            };
            let _ = writeln!(text, "{}{mark}{content}", line + 1);
        }
        next = to;
    }
    (text, matched.len())
}

/// What is printed of one job's log under the request.
pub(crate) fn render(log: &str, request: &Request) -> Result<String> {
    let lines = log_lines(log);
    let mut text = match &request.grep {
        None => {
            let mut text = lines.join("\n");
            text.push('\n');
            text
        }
        Some(pattern) => {
            let (text, count) = grep(&lines, &Pattern::new(pattern)?, request.context);
            if count == 0 {
                return Ok(format!(
                    "(no line of {} matches `{pattern}`)\n",
                    lines.len()
                ));
            }
            text
        }
    };
    if let Some(tail) = request.tail {
        let all: Vec<&str> = text.lines().collect();
        let skipped = all.len().saturating_sub(tail);
        let mut kept = String::new();
        if skipped > 0 {
            let _ = writeln!(kept, "(… {skipped} earlier lines left out)");
        }
        for line in &all[skipped..] {
            kept.push_str(line);
            kept.push('\n');
        }
        text = kept;
    }
    Ok(text)
}

/// Whether `gh` said GitHub knows no such thing.
fn not_found(error: &str) -> bool {
    error.contains("404") || error.contains("Not Found")
}

/// `cargo xtask ci-log …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    let id = request.id;
    let jobs = match crate::runs::gh(&[
        "api",
        &format!("repos/{{owner}}/{{repo}}/actions/runs/{id}/jobs?per_page=100"),
    ]) {
        Ok(json) => parse_jobs(&json)?,
        Err(error) if not_found(&error) => {
            return print_job(
                &Job {
                    id,
                    name: String::new(),
                    conclusion: None,
                },
                &request,
            )
            .map_err(|job_error| {
                format!("{id} is not a run ({error}) and its log could not be read as a job's: {job_error}")
            });
        }
        Err(error) => return Err(error),
    };
    for job in choose(&jobs, &request.choice)? {
        print_job(job, &request)?;
    }
    Ok(())
}

/// Fetches one job's log and prints it under a heading.
fn print_job(job: &Job, request: &Request) -> Result<()> {
    let log = crate::runs::gh(&[
        "api",
        &format!("repos/{{owner}}/{{repo}}/actions/jobs/{}/logs", job.id),
    ])?;
    let heading = if job.name.is_empty() {
        format!("job {}", job.id)
    } else {
        format!(
            "job {} ({}, {})",
            job.name,
            job.id,
            job.conclusion.as_deref().unwrap_or("not ended")
        )
    };
    println!("==== {heading} ====");
    print!("{}", render(&log, request)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Choice, Job, Request, choose, grep, log_lines, not_found, parse_jobs, render, strip_ansi,
    };
    use crate::pattern::Pattern;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// What `GET …/actions/runs/N/jobs` returns, cut down to what is read.
    const JOBS: &str = r#"{"total_count":4,"jobs":[
        {"id":101,"name":"lint-linux","status":"completed","conclusion":"success"},
        {"id":102,"name":"test-windows","status":"completed","conclusion":"failure"},
        {"id":103,"name":"partial-landlock","status":"completed","conclusion":"timed_out"},
        {"id":104,"name":"test-linux","status":"in_progress","conclusion":null},
        {"name":"no id","conclusion":"failure"}]}"#;

    #[test]
    fn a_request_is_read_and_what_makes_no_sense_is_refused() {
        assert_eq!(
            Request::parse(&strings(&[
                "4242",
                "--job",
                "windows",
                "--job",
                "linux",
                "--grep",
                "FAIL",
                "--context",
                "4",
                "--tail",
                "30",
            ]))
            .unwrap(),
            Request {
                id: 4242,
                choice: Choice::Named(strings(&["windows", "linux"])),
                grep: Some("FAIL".to_owned()),
                context: 4,
                tail: Some(30),
            }
        );
        let plain = Request::parse(&strings(&["7"])).unwrap();
        assert_eq!(
            (plain.choice, plain.context, plain.tail),
            (Choice::Failed, 2, None)
        );
        assert_eq!(
            Request::parse(&strings(&["--all", "7"])).unwrap().choice,
            Choice::All
        );
        for (words, expected) in [
            (&[][..], "say which run or job"),
            (&["abc"], "is not a run's or a job's id"),
            (&["7", "8"], "unknown argument"),
            (&["7", "--all", "--job", "x"], "choose differently"),
            (&["7", "--grep"], "needs a pattern"),
            (&["7", "--grep", "(a"], "never closed"),
            (&["7", "--tail", "x"], "is not a number"),
            (&["7", "--follow"], "unknown argument"),
        ] {
            let error = Request::parse(&strings(words)).unwrap_err();
            assert!(error.contains(expected), "{words:?}: {error}");
        }
    }

    #[test]
    fn escape_codes_are_taken_out_and_the_text_between_them_is_not() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m plain"), "red plain");
        assert_eq!(strip_ansi("\u{1b}[1;38;5;208mbold\u{1b}[39;49m!"), "bold!");
        // Cursor moves and erasing end in other letters.
        assert_eq!(strip_ansi("a\u{1b}[2K\u{1b}[1Gb"), "ab");
        // A title or a link, ended by the bell or by `ESC \`.
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}x"), "x");
        assert_eq!(
            strip_ansi("\u{1b}]8;;https://x\u{1b}\\link\u{1b}]8;;\u{1b}\\"),
            "link"
        );
        assert_eq!(strip_ansi("\u{1b}7saved\u{1b}8"), "saved");
        // A byte-order mark, and a `[` that is not after an escape.
        assert_eq!(strip_ansi("\u{feff}2026 [warn] x"), "2026 [warn] x");
        // An escape cut off by the end of the text.
        assert_eq!(strip_ansi("end\u{1b}[3"), "end");
        assert_eq!(
            strip_ansi("ünïcode ✓ \u{1b}[32mok\u{1b}[0m"),
            "ünïcode ✓ ok"
        );
    }

    #[test]
    fn lines_end_at_either_line_ending() {
        assert_eq!(log_lines("a\r\nb\n\u{1b}[0mc"), ["a", "b", "c"]);
    }

    #[test]
    fn the_jobs_are_read_and_a_failed_run_is_its_failed_jobs() {
        let jobs = parse_jobs(JOBS).unwrap();
        // The entry with no id is passed over.
        assert_eq!(jobs.len(), 4);
        assert_eq!(
            jobs[3],
            Job {
                id: 104,
                name: "test-linux".to_owned(),
                conclusion: None
            }
        );
        let ids = |choice: &Choice| -> Vec<u64> {
            choose(&jobs, choice)
                .unwrap()
                .iter()
                .map(|job| job.id)
                .collect()
        };
        // Failed and timed out, in the order GitHub gave.
        assert_eq!(ids(&Choice::Failed), [102, 103]);
        assert_eq!(ids(&Choice::All), [101, 102, 103, 104]);
        // A name is part of the job's, in any case.
        assert_eq!(ids(&Choice::Named(strings(&["WINDOWS"]))), [102]);
        assert_eq!(
            ids(&Choice::Named(strings(&["test-", "landlock"]))),
            [102, 103, 104]
        );
        // A job's name has capitals of its own, as a matrix job's does.
        let capitals = parse_jobs(
            r#"{"jobs":[{"id":7,"name":"Test (Windows, Stable)","conclusion":"failure"}]}"#,
        )
        .unwrap();
        assert_eq!(
            choose(&capitals, &Choice::Named(strings(&["windows"]))).unwrap()[0].id,
            7
        );
        assert!(parse_jobs("{}").is_err());
        assert!(parse_jobs("not json").is_err());
    }

    #[test]
    fn a_choice_that_finds_nothing_lists_what_there_was() {
        let jobs = parse_jobs(JOBS).unwrap();
        let error = choose(&jobs, &Choice::Named(strings(&["macos"]))).unwrap_err();
        assert!(
            error.contains("`macos`") && error.contains("102  failure"),
            "{error}"
        );
        let passed = [Job {
            id: 1,
            name: "lint".to_owned(),
            conclusion: Some("success".to_owned()),
        }];
        let error = choose(&passed, &Choice::Failed).unwrap_err();
        assert!(
            error.contains("no job of the run failed") && error.contains("lint"),
            "{error}"
        );
    }

    const LOG: &str = "start\nline 2\nline 3\nFAIL one\nline 5\nline 6\nline 7\nline 8\nline 9\nFAIL two\nFAIL three\nend\n";

    fn grepped(pattern: &str, context: usize) -> (String, usize) {
        grep(&log_lines(LOG), &Pattern::new(pattern).unwrap(), context)
    }

    #[test]
    fn grep_shows_each_match_with_its_neighbours_and_marks_the_gaps() {
        let (text, count) = grepped("FAIL", 1);
        assert_eq!(count, 3);
        assert_eq!(
            text,
            "3-line 3\n4:FAIL one\n5-line 5\n--\n9-line 9\n10:FAIL two\n11:FAIL three\n12-end\n"
        );
        // Groups of lines that overlap are one, and a match inside another's
        // lines is printed once.
        let (text, _) = grepped("FAIL", 2);
        assert_eq!(
            text,
            "2-line 2\n3-line 3\n4:FAIL one\n5-line 5\n6-line 6\n--\n8-line 8\n9-line 9\n\
             10:FAIL two\n11:FAIL three\n12-end\n"
        );
        let (text, _) = grepped("^start|end", 0);
        assert_eq!(text, "1:start\n--\n12:end\n");
        assert_eq!(grepped("nothing", 2), (String::new(), 0));
    }

    #[test]
    fn a_log_is_printed_whole_or_by_its_matches_or_by_its_end() {
        let request = |grep: Option<&str>, tail: Option<usize>| Request {
            id: 1,
            choice: Choice::All,
            grep: grep.map(str::to_owned),
            context: 0,
            tail,
        };
        let coloured = "\u{1b}[32mone\u{1b}[0m\ntwo\nthree\n";
        assert_eq!(
            render(coloured, &request(None, None)).unwrap(),
            "one\ntwo\nthree\n"
        );
        assert_eq!(
            render(coloured, &request(None, Some(2))).unwrap(),
            "(… 1 earlier lines left out)\ntwo\nthree\n"
        );
        assert_eq!(
            render(coloured, &request(Some("o$"), None)).unwrap(),
            "2:two\n"
        );
        assert_eq!(
            render(coloured, &request(Some("x"), None)).unwrap(),
            "(no line of 3 matches `x`)\n"
        );
    }

    #[test]
    fn only_a_missing_run_sends_the_id_to_be_tried_as_a_job() {
        assert!(not_found("`gh api …` failed: gh: Not Found (HTTP 404)"));
        assert!(!not_found(
            "`gh api …` failed: HTTP 403: rate limit exceeded"
        ));
        assert!(!not_found(
            "could not run gh (is it installed?): no such file"
        ));
    }
}
