//! `cargo xtask ab --job`: a whole job, two ways, on one machine, round by
//! round, and the median of each of its phases (`docs/testing-and-ci.md`,
//! "A whole job two ways on one runner").
//!
//! `ab` builds each tree once and compares the tests' times. A change to how
//! the job is built (a cache, a linker, a profile, a setting in the
//! environment) is in the build, which `ab` does once and does not time. This
//! makes each round a whole job on each side: the side's build directory is
//! emptied, its setup script runs, its tests are built, and the job's runs of
//! them are made, each phase timed. The rounds alternate the order of the two
//! sides, as `ab`'s do. A side that fails in a phase is recorded with the
//! phases it finished, and the run goes on: a run of many rounds of many
//! minutes each must not be lost to one of them. What has been measured is
//! written to `target/ab/job-ab.json` after every side, so that a run that is
//! stopped keeps what it made, and `try-result` reads it.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::Result;
use crate::ab::{Which, order};
use crate::junit;
use crate::tiers::{self, Tier};
use crate::workspace;

const USAGE: &str = "\
usage: cargo xtask ab --job --base DIR [--label NAME] [--count N] [--minutes M]
                      [--setup-base FILE] [--setup-head FILE] [--keep-build]
                      [-p CRATE]... [-- NEXTEST ARGUMENT...]

The whole blocking test job of this tree (the head) and of the tree at DIR (the
base), N rounds (3 when not given, at most 20) in alternating order on this
machine, and the median of each phase on each side. Each side's round:

  clean   its build of the tests (target/ci and target/tmp) is removed, unless
          --keep-build is given (then only the first round builds from cold)
  setup   FILE, a script in this tree, run by bash (a .ps1 by pwsh) in the
          side's tree: --setup-base for the base, --setup-head for the head.
          It may write KEY=VALUE lines to the file $CINERIS_AB_ENV, which are
          set for that side's build and tests. $CINERIS_AB_SIDE is base or
          head, $CINERIS_AB_ROUND the round
  build   the tests, as the blocking job builds them
  tests   the job's runs of them, as its own job makes them (tiers 1 and 2,
          then tier 3), each timed
  tier N  each tier's tests' time summed above their processes' start, from
          the report

`the job` is the setup, the build and the runs together; the clean is not in
it. A side that fails in a phase is recorded with the phases it finished, the
run goes on, and the command fails at the end, after the medians, which are of
the rounds each phase passed in. --minutes M starts no round that the longest
round so far would carry past M minutes from the start; what it stopped
before is said. Everything measured is kept in target/ab/job-ab.json after
each side, and each side's report as target/ab/<side>-job-<round>.xml.";

/// Rounds when not told: each is two whole jobs.
const ROUNDS: usize = 3;

/// The most rounds a run may ask for.
const MOST_ROUNDS: usize = 20;

/// Where the measurements are kept, under the head's `target/ab/`.
pub(crate) const RECORD_FILE: &str = "job-ab.json";

/// What `cargo xtask ab --job` was asked for.
#[derive(Debug, PartialEq)]
pub(crate) struct Request {
    pub(crate) base: PathBuf,
    pub(crate) label: Option<String>,
    pub(crate) rounds: usize,
    /// Start no round past this many minutes.
    pub(crate) minutes: Option<u64>,
    /// Each side's setup script, base then head: a path in this tree.
    pub(crate) setup: [Option<PathBuf>; 2],
    /// Leave the build directories as they are between rounds.
    pub(crate) keep_build: bool,
    pub(crate) packages: Vec<String>,
    /// For nextest, after `--`.
    pub(crate) rest: Vec<String>,
}

impl Request {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut base = None;
        let mut request = Request {
            base: PathBuf::new(),
            label: None,
            rounds: ROUNDS,
            minutes: None,
            setup: [None, None],
            keep_build: false,
            packages: Vec::new(),
            rest: Vec::new(),
        };
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            let mut value = |what: &str| {
                words
                    .next()
                    .ok_or_else(|| format!("`{word}` needs {what}\n\n{USAGE}"))
            };
            match word.as_str() {
                "--" => {
                    request.rest = words.cloned().collect();
                    break;
                }
                "--job" => {}
                "--base" => base = Some(PathBuf::from(value("a directory")?)),
                "--label" => request.label = Some(value("a name")?.clone()),
                "--count" => {
                    let text = value("a number")?;
                    request.rounds = text
                        .parse()
                        .ok()
                        .filter(|rounds| (1..=MOST_ROUNDS).contains(rounds))
                        .ok_or_else(|| {
                            format!("`--count {text}`: a number of rounds from 1 to {MOST_ROUNDS}")
                        })?;
                }
                "--minutes" => {
                    let text = value("a number of minutes")?;
                    request.minutes = Some(
                        text.parse()
                            .ok()
                            .filter(|minutes| *minutes > 0)
                            .ok_or_else(|| format!("`--minutes {text}`: a number above 0"))?,
                    );
                }
                "--setup-base" | "--setup-head" => {
                    let file = PathBuf::from(value("a script's path in this tree")?);
                    let at = usize::from(word == "--setup-head");
                    if request.setup[at].replace(file).is_some() {
                        return Err(format!("`{word}` is given twice"));
                    }
                }
                "--keep-build" => request.keep_build = true,
                "-p" | "--package" => request.packages.push(value("a crate")?.clone()),
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
            }
        }
        request.base = base.ok_or_else(|| format!("say which tree is the base\n\n{USAGE}"))?;
        for file in request.setup.iter().flatten() {
            if file.is_absolute()
                || file
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err(format!(
                    "`{}`: a setup script is a path inside this tree, so that each side's \
                     setup is part of what was committed",
                    file.display()
                ));
            }
        }
        Ok(request)
    }
}

/// Whether `arguments` ask for a job A/B: `--job` before a `--` of their own.
pub(crate) fn asked(arguments: &[String]) -> bool {
    arguments
        .iter()
        .take_while(|word| *word != "--")
        .any(|word| word == "--job")
}

/// One phase of one side's round, and how long it took. A phase that failed
/// is kept with the time it took to fail, and left out of its median.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Phase {
    pub(crate) name: String,
    pub(crate) seconds: f64,
    pub(crate) passed: bool,
}

/// One side's round: its phases, in order, and what went wrong.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SideRound {
    /// Counted from 1.
    pub(crate) round: usize,
    pub(crate) which: Which,
    pub(crate) phases: Vec<Phase>,
    pub(crate) error: Option<String>,
}

/// What a run measured: what the base is called, the rounds asked for, each
/// side's rounds, and why it stopped early, when it did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Record {
    pub(crate) label: String,
    pub(crate) rounds: usize,
    pub(crate) sides: Vec<SideRound>,
    pub(crate) stopped: Option<String>,
}

/// The name of the phase that is the whole job, the setup, the build and
/// the runs together.
const JOB: &str = "the job";

/// Where a phase goes in the tables: the order a round makes them in.
fn rank(name: &str) -> usize {
    match name {
        "clean" => 0,
        "setup" => 1,
        "build" => 2,
        JOB => 5,
        tests if tests.starts_with("tests") => 3,
        _ => 4,
    }
}

/// The name of the run `index` of a job of `runs` runs.
pub(crate) fn run_name(index: usize, runs: usize) -> String {
    match (runs, index) {
        (1, _) => "tests".to_owned(),
        (2, 0) => "tests: tiers 1 and 2".to_owned(),
        (2, _) => "tests: tier 3".to_owned(),
        (_, index) => format!("tests: run {}", index + 1),
    }
}

/// The name of a tier's summed time.
fn tier_name(tier: Tier) -> String {
    format!("tier {} summed", tier.name())
}

/// Whether a round may start: with no limit, always; with one, when the
/// longest round so far, added to the minutes already spent, ends within it.
/// The first round always starts: there is no round to judge it by.
pub(crate) fn round_fits(
    spent_seconds: f64,
    longest_round: Option<f64>,
    minutes: Option<u64>,
) -> bool {
    match (minutes, longest_round) {
        (None, _) | (_, None) => true,
        (Some(minutes), Some(longest)) => spent_seconds + longest <= minutes as f64 * 60.0,
    }
}

/// The median of `values`, none when there are none.
fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[middle]
    } else {
        f64::midpoint(values[middle - 1], values[middle])
    })
}

/// A phase's median on one side, none when no round passed it, and how many
/// rounds passed it.
pub(crate) type SideMedian = (Option<f64>, usize);

/// Each phase's median on each side, over the rounds it passed in, and how
/// many rounds that was: `(name, [base, head])`, in the order a round makes
/// the phases.
pub(crate) fn medians(record: &Record) -> Vec<(String, [SideMedian; 2])> {
    let mut names: Vec<String> = Vec::new();
    for side in &record.sides {
        for phase in &side.phases {
            if !names.contains(&phase.name) {
                names.push(phase.name.clone());
            }
        }
    }
    // Stable: a phase's place among those of its rank is its first.
    names.sort_by_key(|name| rank(name));
    names
        .into_iter()
        .map(|name| {
            let of = |which: Which| {
                let times: Vec<f64> = record
                    .sides
                    .iter()
                    .filter(|side| side.which == which)
                    .flat_map(|side| &side.phases)
                    .filter(|phase| phase.name == name && phase.passed)
                    .map(|phase| phase.seconds)
                    .collect();
                let passed = times.len();
                (median(times), passed)
            };
            let both = [of(Which::Base), of(Which::Head)];
            (name, both)
        })
        .collect()
}

/// How many rounds each side ran.
fn rounds_run(record: &Record, which: Which) -> usize {
    record
        .sides
        .iter()
        .filter(|side| side.which == which)
        .count()
}

/// The table of each phase's median on each side, and the difference.
pub(crate) fn medians_table(record: &Record) -> String {
    let base = format!("base ({})", record.label);
    let width = base.len().max(16);
    let rows = medians(record);
    let name_width = rows
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(5)
        .max(5);
    let runs = [
        rounds_run(record, Which::Base),
        rounds_run(record, Which::Head),
    ];
    let mut text = format!(
        "Each phase's median, in seconds, over the rounds it passed in (passed of run):\n\n\
         {:<name_width$}  {base:>width$}  {:>width$}  {:>8}\n",
        "phase", "head (this tree)", "change"
    );
    for (name, sides) in &rows {
        let cell = |side: usize| {
            let (seconds, passed) = sides[side];
            let shown = match seconds {
                Some(seconds) => format!("{seconds:.1}"),
                None => "-".to_owned(),
            };
            format!("{:>width$}", format!("{shown} ({passed}/{})", runs[side]))
        };
        let change = match (sides[0].0, sides[1].0) {
            // A change that rounds to nothing is +0.0, never -0.0.
            (Some(base), Some(head)) if (head - base).abs() < 0.05 => "+0.0".to_owned(),
            (Some(base), Some(head)) => format!("{:+.1}", head - base),
            _ => "-".to_owned(),
        };
        let _ = writeln!(
            text,
            "{name:<name_width$}  {}  {}  {change:>8}",
            cell(0),
            cell(1)
        );
    }
    text
}

/// One line for each side's round: its phases' times, a failed one marked
/// `!`, and what went wrong.
pub(crate) fn rounds_text(record: &Record) -> String {
    let mut text = String::new();
    for side in &record.sides {
        let phases: Vec<String> = side
            .phases
            .iter()
            .map(|phase| {
                format!(
                    "{} {:.1} s{}",
                    phase.name,
                    phase.seconds,
                    if phase.passed { "" } else { " !" }
                )
            })
            .collect();
        let _ = writeln!(
            text,
            "round {}, {}: {}",
            side.round,
            side_name(side.which),
            phases.join(", ")
        );
        if let Some(error) = &side.error {
            let _ = writeln!(text, "  ! {}", first_line(error));
        }
    }
    text
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// Everything a reader is shown: each round, the medians, the sides that
/// failed and why the run stopped early, when it did.
pub(crate) fn render(record: &Record) -> String {
    let mut text = format!(
        "A whole job two ways, the head against {} ({} of {} rounds run):\n\n",
        record.label,
        rounds_run(record, Which::Head).max(rounds_run(record, Which::Base)),
        record.rounds
    );
    text.push_str(&rounds_text(record));
    text.push('\n');
    text.push_str(&medians_table(record));
    let failed = record
        .sides
        .iter()
        .filter(|side| side.error.is_some())
        .count();
    if failed > 0 {
        let _ = writeln!(
            text,
            "\n{failed} side round(s) failed (marked ! above): a failed phase is left out of \
             its median, and so is every phase after it"
        );
    }
    if let Some(stopped) = &record.stopped {
        let _ = writeln!(text, "\n{stopped}");
    }
    text
}

/// What the run ends as: a side that failed is a failure of the run, after
/// the medians are printed; so is a run with no round on both sides.
pub(crate) fn conclusion(record: &Record) -> Result<()> {
    let failed: Vec<String> = record
        .sides
        .iter()
        .filter(|side| side.error.is_some())
        .map(|side| format!("round {} {}", side.round, side_name(side.which)))
        .collect();
    if record.sides.is_empty() {
        return Err("no round was run".to_owned());
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} side round(s) failed ({}): the medians are of the rounds each phase passed in",
            failed.len(),
            failed.join(", ")
        ))
    }
}

fn side_name(which: Which) -> &'static str {
    match which {
        Which::Base => "base",
        Which::Head => "head",
    }
}

/// The record as `ab --job` keeps it: JSON.
pub(crate) fn to_json(record: &Record) -> String {
    let sides: Vec<serde_json::Value> = record
        .sides
        .iter()
        .map(|side| {
            serde_json::json!({
                "round": side.round,
                "side": side_name(side.which),
                "error": side.error,
                "phases": side.phases.iter().map(|phase| serde_json::json!({
                    "name": phase.name,
                    "seconds": phase.seconds,
                    "passed": phase.passed,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut text = serde_json::json!({
        "label": record.label,
        "rounds": record.rounds,
        "stopped": record.stopped,
        "sides": sides,
    })
    .to_string();
    text.push('\n');
    text
}

/// A record [`to_json`] wrote.
pub(crate) fn from_json(text: &str) -> Result<Record> {
    use serde_json::Value;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("the job A/B's record is not JSON: {error}"))?;
    let label = value
        .get("label")
        .and_then(Value::as_str)
        .ok_or("the job A/B's record names no base")?
        .to_owned();
    let rounds = value
        .get("rounds")
        .and_then(Value::as_u64)
        .and_then(|rounds| usize::try_from(rounds).ok())
        .ok_or("the job A/B's record says no number of rounds")?;
    let stopped = value
        .get("stopped")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut sides = Vec::new();
    for side in value
        .get("sides")
        .and_then(Value::as_array)
        .ok_or("the job A/B's record has no sides")?
    {
        let whole = || format!("a side of the job A/B's record is not whole: {side}");
        let round = side
            .get("round")
            .and_then(Value::as_u64)
            .and_then(|round| usize::try_from(round).ok())
            .ok_or_else(whole)?;
        let which = match side.get("side").and_then(Value::as_str) {
            Some("base") => Which::Base,
            Some("head") => Which::Head,
            _ => return Err(whole()),
        };
        let mut phases = Vec::new();
        for phase in side
            .get("phases")
            .and_then(Value::as_array)
            .ok_or_else(whole)?
        {
            phases.push(Phase {
                name: phase
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(whole)?
                    .to_owned(),
                seconds: phase
                    .get("seconds")
                    .and_then(Value::as_f64)
                    .ok_or_else(whole)?,
                passed: phase
                    .get("passed")
                    .and_then(Value::as_bool)
                    .ok_or_else(whole)?,
            });
        }
        sides.push(SideRound {
            round,
            which,
            phases,
            error: side.get("error").and_then(Value::as_str).map(str::to_owned),
        });
    }
    Ok(Record {
        label,
        rounds,
        sides,
        stopped,
    })
}

/// The `KEY=VALUE` lines a setup script wrote: blank lines and lines that
/// start with `#` are passed over; anything else that is not one is an
/// error naming it.
pub(crate) fn read_env(text: &str) -> Result<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!(
                "`{line}` in the setup's environment file is not KEY=VALUE"
            ));
        };
        let key = key.trim();
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(format!(
                "`{key}` in the setup's environment file is not a variable's name"
            ));
        }
        pairs.push((key.to_owned(), value.to_owned()));
    }
    Ok(pairs)
}

/// The report a side's round is kept under in `target/ab/`:
/// `<side>-job-<round>.xml`, a name `ab`'s readers do not take for one of
/// its rounds' (`<side>-round-<n>.xml`).
pub(crate) fn report_name(which: Which, round: usize) -> String {
    format!("{}-job-{round}.xml", side_name(which))
}

/// The side and the round of a report [`report_name`] named.
pub(crate) fn report_of_name(file: &str) -> Option<(&'static str, usize)> {
    let (side, round) = file.strip_suffix(".xml")?.split_once("-job-")?;
    let side = match side {
        "base" => "base",
        "head" => "head",
        _ => return None,
    };
    Some((side, round.parse().ok()?))
}

/// The program that runs a setup script, and its arguments before the
/// script: pwsh for a `.ps1`, bash for anything else. On Windows, Git's own
/// bash when it is where Git for Windows puts it: a plain `bash` there may
/// be the one in the system folder, which starts WSL.
fn setup_program(script: &Path) -> (PathBuf, Vec<&'static str>) {
    if script
        .extension()
        .is_some_and(|extension| extension == "ps1")
    {
        return (PathBuf::from("pwsh"), vec!["-NoProfile", "-File"]);
    }
    if cfg!(windows)
        && let Some(git_bash) = std::env::var_os("ProgramFiles")
            .map(|folder| {
                PathBuf::from(folder)
                    .join("Git")
                    .join("bin")
                    .join("bash.exe")
            })
            .filter(|path| path.is_file())
    {
        return (git_bash, Vec::new());
    }
    (PathBuf::from("bash"), Vec::new())
}

/// Runs `command`, and says how long it took and whether it succeeded.
fn timed(mut command: Command, what: &str) -> (f64, Result<()>) {
    let started = Instant::now();
    let status = command.status();
    let seconds = started.elapsed().as_secs_f64();
    let outcome = match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{what} failed ({status})")),
        Err(error) => Err(format!("could not start {what}: {error}")),
    };
    (seconds, outcome)
}

/// Removes the side's build of the tests: the `ci` profile's folder and the
/// tests' scratch folder. `xtask`'s own build (`target/debug`) is left, as
/// it is running, and so are the kept reports (`target/ab`).
fn clean(tree: &Path) -> Result<()> {
    for folder in ["ci", "tmp"] {
        let path = tree.join("target").join(folder);
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .map_err(|error| format!("could not remove {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

/// One side's round: every phase, timed, and what went wrong. It never
/// fails: a phase that fails ends the side's round, and is recorded.
fn run_side(head: &Path, tree: &Path, which: Which, round: usize, request: &Request) -> SideRound {
    let mut side = SideRound {
        round,
        which,
        phases: Vec::new(),
        error: None,
    };
    if !request.keep_build {
        let started = Instant::now();
        let outcome = clean(tree);
        if !phase(&mut side, "clean", started.elapsed().as_secs_f64(), outcome) {
            return side;
        }
    }
    let job_started = Instant::now();
    let mut environment = Vec::new();
    if let Some(script) = &request.setup[which as usize] {
        let env_file = tree.join("target").join("job-ab-env");
        let script = head.join(script);
        let (program, before) = setup_program(&script);
        let mut command = Command::new(program);
        command
            .args(before)
            .arg(&script)
            .current_dir(tree)
            .env("CINERIS_AB_ENV", &env_file)
            .env("CINERIS_AB_SIDE", side_name(which))
            .env("CINERIS_AB_ROUND", round.to_string());
        let prepared = std::fs::create_dir_all(tree.join("target"))
            .and_then(|()| workspace_forget(&env_file))
            .map_err(|error| format!("could not prepare {}: {error}", env_file.display()));
        let (seconds, outcome) = match prepared {
            Ok(()) => timed(command, &format!("the setup {}", script.display())),
            Err(error) => (0.0, Err(error)),
        };
        let outcome = outcome.and_then(|()| match std::fs::read_to_string(&env_file) {
            Ok(text) => read_env(&text).map(|pairs| environment = pairs),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("could not read {}: {error}", env_file.display())),
        });
        if !phase(&mut side, "setup", seconds, outcome) {
            return job_failed(side, job_started);
        }
    }
    let with_environment = |arguments: &[String]| {
        let mut command = crate::ab::nextest(tree, arguments);
        command.envs(environment.iter().map(|(key, value)| (key, value)));
        command
    };
    let mut build = crate::checks::ci_run(&request.packages);
    build.push("--no-run".to_owned());
    let (seconds, outcome) = timed(with_environment(&build), "the build");
    if !phase(&mut side, "build", seconds, outcome) {
        return job_failed(side, job_started);
    }
    if let Err(error) = crate::checks::forget_reports(tree) {
        phase(&mut side, "tests", 0.0, Err(error));
        return job_failed(side, job_started);
    }
    let runs = crate::checks::ci_runs(
        &request.packages,
        &request.rest,
        crate::checks::splits(tree),
    );
    let mut runs_passed = true;
    for (index, run) in runs.iter().enumerate() {
        let name = run_name(index, runs.len());
        let (seconds, outcome) = timed(with_environment(run), &name);
        runs_passed &= phase(&mut side, &name, seconds, outcome);
        let last = index + 1 == runs.len();
        let kept = if last {
            crate::checks::bring_together(tree)
        } else {
            crate::checks::set_aside(tree)
        };
        if let Err(error) = kept {
            runs_passed = false;
            side.error = Some(match side.error.take() {
                Some(earlier) => format!("{earlier}; {error}"),
                None => error,
            });
        }
    }
    let job_seconds = job_started.elapsed().as_secs_f64();
    let report = workspace::nextest_report(tree, "ci");
    match junit::read(&report) {
        Ok(cases) => {
            let (start, _) = tiers::start_of(&cases);
            for tier in Tier::ALL {
                let total = tiers::total(&cases, tier, start);
                side.phases.push(Phase {
                    name: tier_name(tier),
                    seconds: total.above_start,
                    passed: runs_passed,
                });
            }
            let kept = head
                .join("target")
                .join("ab")
                .join(report_name(which, round));
            if let Err(error) = std::fs::create_dir_all(kept.parent().unwrap_or(head))
                .and_then(|()| std::fs::copy(&report, &kept))
            {
                println!("(could not keep {}: {error})", kept.display());
            }
        }
        Err(error) => {
            runs_passed = false;
            if side.error.is_none() {
                side.error = Some(error);
            }
        }
    }
    side.phases.push(Phase {
        name: JOB.to_owned(),
        seconds: job_seconds,
        passed: runs_passed,
    });
    side
}

/// Records a phase of `side` that took `seconds` and ended as `outcome`, an
/// error added to the side's: whether it passed.
fn phase(side: &mut SideRound, name: &str, seconds: f64, outcome: Result<()>) -> bool {
    side.phases.push(Phase {
        name: name.to_owned(),
        seconds,
        passed: outcome.is_ok(),
    });
    match outcome {
        Ok(()) => true,
        Err(error) => {
            side.error = Some(match side.error.take() {
                Some(earlier) => format!("{earlier}; {error}"),
                None => error,
            });
            false
        }
    }
}

/// A side's round that ended in a failed phase: the job, as far as it went.
fn job_failed(mut side: SideRound, started: Instant) -> SideRound {
    side.phases.push(Phase {
        name: JOB.to_owned(),
        seconds: started.elapsed().as_secs_f64(),
        passed: false,
    });
    side
}

/// Removes `path` if it is there.
fn workspace_forget(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// `cargo xtask ab --job …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    crate::checks::require_nextest()?;
    let head = workspace::root();
    let base = workspace::normalised(
        &std::path::absolute(&request.base)
            .map_err(|error| format!("{}: {error}", request.base.display()))?,
    );
    if !base.join("Cargo.toml").is_file() {
        return Err(format!(
            "{} is not a checkout of the workspace",
            base.display()
        ));
    }
    crate::ab::refuse_base(&head, &base)?;
    for script in request.setup.iter().flatten() {
        if !head.join(script).is_file() {
            return Err(format!(
                "the setup script {} is not in this tree",
                script.display()
            ));
        }
    }
    let label = request
        .label
        .clone()
        .unwrap_or_else(|| base.display().to_string());
    let tree = |which: Which| if which == Which::Base { &base } else { &head };
    let record_file = head.join("target").join("ab").join(RECORD_FILE);
    let mut record = Record {
        label: label.clone(),
        rounds: request.rounds,
        sides: Vec::new(),
        stopped: None,
    };
    let started = Instant::now();
    let mut longest: Option<f64> = None;
    for round in 0..request.rounds {
        let spent = started.elapsed().as_secs_f64();
        if !round_fits(spent, longest, request.minutes) {
            record.stopped = Some(format!(
                "Stopped before round {} of {}: {:.0} min spent, and the longest round took \
                 {:.0} min, which would pass --minutes {}.",
                round + 1,
                request.rounds,
                spent / 60.0,
                longest.unwrap_or_default() / 60.0,
                request.minutes.unwrap_or_default()
            ));
            break;
        }
        let round_started = Instant::now();
        for which in order(round) {
            println!(
                "== round {} of {}: the {} ==",
                round + 1,
                request.rounds,
                match which {
                    Which::Base => format!("base ({label})"),
                    Which::Head => "head (this tree)".to_owned(),
                }
            );
            let side = run_side(&head, tree(which), which, round + 1, &request);
            if let Some(error) = &side.error {
                println!("== it failed: {}; carrying on ==", first_line(error));
            }
            record.sides.push(side);
            // After every side, so that a run that is stopped keeps what it
            // made. A record that cannot be written is said, and the run
            // goes on: the log has every line of it too.
            if let Err(error) = std::fs::create_dir_all(record_file.parent().unwrap_or(&head))
                .and_then(|()| std::fs::write(&record_file, to_json(&record)))
            {
                println!("(could not write {}: {error})", record_file.display());
            }
        }
        let took = round_started.elapsed().as_secs_f64();
        longest = Some(longest.map_or(took, |longest| longest.max(took)));
    }
    if let Err(error) = std::fs::write(&record_file, to_json(&record)) {
        println!("(could not write {}: {error})", record_file.display());
    }
    let text = render(&record);
    println!("\n{text}");
    crate::steps::append_job_summary(&format!(
        "### A whole job two ways on one runner\n\n```text\n{text}```\n"
    ));
    conclusion(&record)
}

/// Each side's reports kept by a run, by round: for a reader of the kept
/// files (`try-result`).
pub(crate) fn kept_reports(files: &[PathBuf]) -> BTreeMap<(usize, &'static str), PathBuf> {
    files
        .iter()
        .filter_map(|file| {
            let name = file.file_name()?.to_str()?;
            let (side, round) = report_of_name(name)?;
            Some(((round, side), file.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        JOB, Phase, Record, Request, SideRound, conclusion, from_json, medians, medians_table,
        read_env, render, report_name, report_of_name, round_fits, run_name, to_json,
    };
    use crate::ab::Which;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn phase(name: &str, seconds: f64, passed: bool) -> Phase {
        Phase {
            name: name.to_owned(),
            seconds,
            passed,
        }
    }

    fn side(round: usize, which: Which, phases: Vec<Phase>, error: Option<&str>) -> SideRound {
        SideRound {
            round,
            which,
            phases,
            error: error.map(str::to_owned),
        }
    }

    /// A side that passed every phase: a build of `build` seconds, and the
    /// job's two runs.
    fn passed(round: usize, which: Which, build: f64) -> SideRound {
        side(
            round,
            which,
            vec![
                phase("clean", 1.0, true),
                phase("build", build, true),
                phase("tests: tiers 1 and 2", 30.0, true),
                phase("tests: tier 3", 50.0, true),
                phase("tier 1, unit summed", 10.0, true),
                phase(JOB, build + 80.0, true),
            ],
            None,
        )
    }

    fn record(sides: Vec<SideRound>) -> Record {
        Record {
            label: "main".to_owned(),
            rounds: 3,
            sides,
            stopped: None,
        }
    }

    #[test]
    fn a_request_is_read_and_what_makes_no_sense_is_refused() {
        let request = Request::parse(&strings(&[
            "--job",
            "--base",
            "../b",
            "--label",
            "main",
            "--count",
            "2",
            "--minutes",
            "200",
            "--setup-head",
            ".github/scripts/x.sh",
            "--keep-build",
            "-p",
            "cineris-base",
            "--",
            "-E",
            "test(/a/)",
        ]))
        .unwrap();
        assert_eq!(request.rounds, 2);
        assert_eq!(request.minutes, Some(200));
        assert_eq!(request.setup[0], None);
        assert_eq!(
            request.setup[1].as_deref(),
            Some(std::path::Path::new(".github/scripts/x.sh"))
        );
        assert!(request.keep_build);
        assert_eq!(request.packages, ["cineris-base"]);
        assert_eq!(request.rest, ["-E", "test(/a/)"]);
        assert_eq!(
            Request::parse(&strings(&["--base", "b"])).unwrap().rounds,
            3,
            "three rounds when not told"
        );
        for (arguments, said) in [
            (&["--count", "1"][..], "say which tree"),
            (&["--base", "b", "--count", "0"], "from 1 to 20"),
            (&["--base", "b", "--count", "21"], "from 1 to 20"),
            (&["--base", "b", "--minutes", "0"], "above 0"),
            (
                &["--base", "b", "--setup-base", "/abs.sh"],
                "inside this tree",
            ),
            (
                &["--base", "b", "--setup-base", "../x.sh"],
                "inside this tree",
            ),
            (
                &["--base", "b", "--setup-head", "a", "--setup-head", "b"],
                "given twice",
            ),
            (&["--base", "b", "--top", "3"], "unknown argument"),
        ] {
            let error = Request::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(said), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn a_job_ab_is_asked_for_by_job_before_the_tests_own_words() {
        assert!(super::asked(&strings(&["--base", "b", "--job"])));
        assert!(!super::asked(&strings(&["--base", "b", "--", "--job"])));
        assert!(!super::asked(&strings(&["--base", "b"])));
    }

    #[test]
    fn the_jobs_runs_are_named_by_their_tiers() {
        assert_eq!(run_name(0, 1), "tests");
        assert_eq!(run_name(0, 2), "tests: tiers 1 and 2");
        assert_eq!(run_name(1, 2), "tests: tier 3");
        assert_eq!(run_name(2, 3), "tests: run 3");
    }

    #[test]
    fn each_phase_takes_its_median_over_the_rounds_it_passed_in() {
        let mut failed_head = passed(2, Which::Head, 500.0);
        failed_head.phases[2].passed = false;
        failed_head.error = Some("tests failed".to_owned());
        let record = record(vec![
            passed(1, Which::Base, 100.0),
            passed(1, Which::Head, 90.0),
            failed_head,
            passed(2, Which::Base, 130.0),
            passed(3, Which::Base, 110.0),
            passed(3, Which::Head, 80.0),
        ]);
        let rows = medians(&record);
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "clean",
                "build",
                "tests: tiers 1 and 2",
                "tests: tier 3",
                "tier 1, unit summed",
                JOB
            ],
            "in the order a round makes them"
        );
        let build = &rows[1].1;
        assert_eq!(build[0], (Some(110.0), 3));
        // The head's build passed in all three rounds, its slow one too.
        assert_eq!(build[1], (Some(90.0), 3));
        let tiers_1_and_2 = &rows[2].1;
        assert_eq!(
            tiers_1_and_2[1],
            (Some(30.0), 2),
            "the round it failed in is left out"
        );
        let table = medians_table(&record);
        assert!(table.contains("base (main)"), "{table}");
        assert!(table.contains("110.0 (3/3)"), "{table}");
        assert!(table.contains("30.0 (2/3)"), "{table}");
        assert!(table.contains("-20.0"), "{table}");
        // The clean took 1.0 s on both sides: no change, and no minus sign.
        let clean = table
            .lines()
            .find(|line| line.starts_with("clean"))
            .unwrap();
        assert!(clean.trim_end().ends_with(" +0.0"), "{clean}");
    }

    #[test]
    fn a_phase_no_round_passed_has_no_median_and_no_change() {
        let record = record(vec![
            side(
                1,
                Which::Base,
                vec![phase("build", 5.0, false), phase(JOB, 5.0, false)],
                Some("the build failed"),
            ),
            passed(1, Which::Head, 90.0),
        ]);
        let rows = medians(&record);
        let build = rows.iter().find(|(name, _)| name == "build").unwrap();
        assert_eq!(build.1[0], (None, 0));
        let table = medians_table(&record);
        let line = table
            .lines()
            .find(|line| line.starts_with("build"))
            .unwrap();
        assert!(line.contains("- (0/1)"), "{line}");
        assert!(line.trim_end().ends_with('-'), "{line}");
    }

    #[test]
    fn a_failed_side_is_shown_and_fails_the_run_and_a_clean_run_passes() {
        let failed = record(vec![
            passed(1, Which::Base, 100.0),
            side(
                1,
                Which::Head,
                vec![phase("setup", 0.5, false), phase(JOB, 0.5, false)],
                Some("the setup x.sh failed (exit status: 1)\nmore"),
            ),
        ]);
        let text = render(&failed);
        assert!(text.contains("round 1, head: setup 0.5 s !"), "{text}");
        assert!(
            text.contains("  ! the setup x.sh failed (exit status: 1)"),
            "{text}"
        );
        assert!(!text.contains("more"), "only the first line: {text}");
        assert!(text.contains("1 side round(s) failed"), "{text}");
        let error = conclusion(&failed).unwrap_err();
        assert!(error.contains("round 1 head"), "{error}");
        assert!(conclusion(&record(vec![passed(1, Which::Base, 1.0)])).is_ok());
        assert!(
            conclusion(&record(Vec::new())).is_err(),
            "no round is no measurement"
        );
    }

    #[test]
    fn a_round_starts_only_when_the_longest_so_far_fits_in_what_is_left() {
        assert!(
            round_fits(0.0, None, Some(1)),
            "the first round always starts"
        );
        assert!(round_fits(10_000.0, Some(10_000.0), None), "no limit");
        // 200 minutes: 150 spent, rounds of 45 fit, of 51 do not.
        assert!(round_fits(150.0 * 60.0, Some(45.0 * 60.0), Some(200)));
        assert!(round_fits(150.0 * 60.0, Some(50.0 * 60.0), Some(200)));
        assert!(!round_fits(150.0 * 60.0, Some(51.0 * 60.0), Some(200)));
    }

    #[test]
    fn the_record_is_kept_and_read_back_whole() {
        let mut kept = record(vec![
            passed(1, Which::Base, 100.0),
            side(
                1,
                Which::Head,
                vec![phase("build", 2.5, false), phase(JOB, 2.5, false)],
                Some("the build failed"),
            ),
        ]);
        kept.stopped = Some("Stopped before round 2 of 3".to_owned());
        assert_eq!(from_json(&to_json(&kept)).unwrap(), kept);
        assert!(from_json("{}").is_err());
        assert!(from_json(r#"{"label":"m","rounds":1,"sides":[{"round":1}]}"#).is_err());
        let text = render(&kept);
        assert!(text.contains("Stopped before round 2 of 3"), "{text}");
    }

    #[test]
    fn a_setups_environment_is_key_value_lines() {
        assert_eq!(
            read_env("# a note\nRUSTFLAGS=-C link-arg=x\r\n\nCARGO_X=a=b\n").unwrap(),
            [
                ("RUSTFLAGS".to_owned(), "-C link-arg=x".to_owned()),
                ("CARGO_X".to_owned(), "a=b".to_owned()),
            ]
        );
        assert!(read_env("no equals sign").is_err());
        assert!(read_env("A B=1").is_err());
        assert!(read_env("=1").is_err());
    }

    #[test]
    fn a_sides_report_has_a_name_that_is_not_an_ab_rounds() {
        let name = report_name(Which::Base, 2);
        assert_eq!(name, "base-job-2.xml");
        assert_eq!(report_of_name(&name), Some(("base", 2)));
        assert_eq!(crate::runs::ab_report_name(&name), None);
        assert_eq!(report_of_name("base-round-2.xml"), None);
        assert_eq!(report_of_name("other-job-2.xml"), None);
    }
}
