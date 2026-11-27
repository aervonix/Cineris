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
                      [--from-archive base|head] [-p CRATE]... [-- NEXTEST ARGUMENT...]

The whole blocking test job of this tree (the head) and of the tree at DIR (the
base), N rounds (2 when not given; an even number, at most 20, so that each
side goes first as often as the other) in alternating order on this machine,
and each phase's median on each side and its change paired by round. Each
side's round:

  clean   its build of the tests (target/ci and target/tmp) is removed: in
          every round, or with --keep-build (and --count 4 or more) in the
          first two alone, one in each order; then a cold round's phases are
          named apart from a warm one's (`build (cold)`, `build (warm)`)
  setup   FILE, a script in this tree, run by bash (a .ps1 by pwsh) in the
          side's tree: --setup-base for the base, --setup-head for the head.
          It may write KEY=VALUE lines to the file $CINERIS_AB_ENV, which are
          set for that side's build and tests. $CINERIS_AB_SIDE is base or
          head, $CINERIS_AB_ROUND the round
  build   the tests, as the blocking job builds them
  extract with --from-archive, on that side alone: the build is a nextest
          archive of the tests (`ci test --archive`'s), the build directory
          is set aside, and the archive is extracted over the side's tree
          as a Windows test job extracts it (`ci test PART --from-archive`)
  tests   the job's runs of them, as its own job makes them (tiers 1 and 2,
          then tier 3), each timed; from the archive on its side, as the
          Windows test jobs run them
  tier N  each tier's tests' time summed above their processes' start, from
          the report

Before the first round each tree is warmed, untimed (`cargo fetch --locked`,
`cargo nextest --version`). `the job` is the setup, the build and the runs
together; the clean is not in it. A failed clean, setup or build ends that side's round; a failed run of the
tests does not (the next run is made, and the tiers' sums and `the job` count
as failed). Either way the round is recorded, the run goes on, and the command
fails at the end. Each side's median is of the rounds the phase passed in on
that side; the change is the median of head minus base over the rounds the
phase passed in on both sides, with how many such rounds there were and in how
many the base ran first. --from-archive SIDE compares the two layouts of the
Windows test job (built and run in one job, or built into an archive and run
from it): give both sides one tree's commit, and no -p or nextest arguments,
since the archive holds the workspace's tests and its runs are the jobs'
own. --minutes M starts no round that the longest
round so far would carry past M minutes from the start; what it stopped
before is said. Everything measured is kept in target/ab/job-ab.json after
each side, and each side's report as target/ab/<side>-job-<round>.xml.";

/// Rounds when not told: each is two whole jobs, and an even number puts
/// each side first as often as the other.
const ROUNDS: usize = 2;

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
    /// The side whose tests are archived, extracted and run from the
    /// archive, as the Windows test jobs run them.
    pub(crate) from_archive: Option<Which>,
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
            from_archive: None,
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
                        .filter(|rounds: &usize| {
                            (2..=MOST_ROUNDS).contains(rounds) && rounds.is_multiple_of(2)
                        })
                        .ok_or_else(|| {
                            format!(
                                "`--count {text}`: an even number of rounds from 2 to \
                                 {MOST_ROUNDS}, so that each side goes first as often as the \
                                 other (the first pays for a cold disk cache)"
                            )
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
                "--from-archive" => {
                    let side = match value("a side, base or head")?.as_str() {
                        "base" => Which::Base,
                        "head" => Which::Head,
                        other => {
                            return Err(format!(
                                "`--from-archive {other}`: the side to run from an archive, \
                                 base or head"
                            ));
                        }
                    };
                    if request.from_archive.replace(side).is_some() {
                        return Err("`--from-archive` is given twice".to_owned());
                    }
                }
                "-p" | "--package" => request.packages.push(value("a crate")?.clone()),
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
            }
        }
        request.base = base.ok_or_else(|| format!("say which tree is the base\n\n{USAGE}"))?;
        if request.from_archive.is_some()
            && (!request.packages.is_empty() || !request.rest.is_empty())
        {
            return Err(
                "`--from-archive` takes no -p and no nextest arguments: the archive holds the \
                 workspace's tests, and its runs are the Windows test jobs' own"
                    .to_owned(),
            );
        }
        if request.keep_build && request.rounds < 2 * COLD_ROUNDS {
            return Err(format!(
                "`--keep-build` needs `--count 4` or more: the first {COLD_ROUNDS} rounds build \
                 from cold, one in each order, and the rounds after them, warm, are compared \
                 apart from them"
            ));
        }
        for file in request.setup.iter().flatten() {
            // A root alone (`/x.sh`) is not absolute on Windows, which wants
            // a drive too: any root, drive or `..` is refused, on both.
            if file.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            }) {
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
    let name = name
        .strip_suffix(" (cold)")
        .or_else(|| name.strip_suffix(" (warm)"))
        .unwrap_or(name);
    match name {
        "clean" => 0,
        "setup" => 1,
        "build" => 2,
        "extract" => 3,
        JOB => 6,
        tests if tests.starts_with("tests") => 4,
        _ => 5,
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

/// A phase's change, paired by round: the median of head minus base over the
/// rounds in which it passed on both sides, how many such rounds there were,
/// and in how many of them the base ran first. Each side's own median drops
/// its own failed rounds, so two medians can be of different rounds, and of
/// rounds one side always began; the pairs cannot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Paired {
    pub(crate) change: Option<f64>,
    pub(crate) pairs: usize,
    pub(crate) base_first: usize,
}

/// The paired change of the phase `name` ([`Paired`]).
pub(crate) fn paired(record: &Record, name: &str) -> Paired {
    let time = |which: Which, round: usize| {
        record
            .sides
            .iter()
            .filter(|side| side.which == which && side.round == round)
            .flat_map(|side| &side.phases)
            .find(|phase| phase.name == name && phase.passed)
            .map(|phase| phase.seconds)
    };
    let mut rounds: Vec<usize> = record.sides.iter().map(|side| side.round).collect();
    rounds.sort_unstable();
    rounds.dedup();
    let mut changes = Vec::new();
    let mut base_first = 0;
    for round in rounds {
        if let (Some(base), Some(head)) = (time(Which::Base, round), time(Which::Head, round)) {
            changes.push(head - base);
            if order(round.saturating_sub(1))[0] == Which::Base {
                base_first += 1;
            }
        }
    }
    Paired {
        pairs: changes.len(),
        change: median(changes),
        base_first,
    }
}

/// The table of each phase's median on each side, and its paired change.
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
        "Each phase's median, in seconds, over the rounds it passed in (passed of run), and \
         its change, head minus base, paired by round (the median over the rounds it passed \
         in on both sides; how many, and in how many the base ran first):\n\n\
         {:<name_width$}  {base:>width$}  {:>width$}  {:>24}\n",
        "phase", "head (this tree)", "paired change"
    );
    let mut lopsided = false;
    for (name, sides) in &rows {
        let cell = |side: usize| {
            let (seconds, passed) = sides[side];
            let shown = match seconds {
                Some(seconds) => format!("{seconds:.1}"),
                None => "-".to_owned(),
            };
            format!("{:>width$}", format!("{shown} ({passed}/{})", runs[side]))
        };
        let pair = paired(record, name);
        // A change that rounds to nothing is +0.0, never -0.0.
        let shown = match pair.change {
            Some(change) if change.abs() < 0.05 => "+0.0".to_owned(),
            Some(change) => format!("{change:+.1}"),
            None => "-".to_owned(),
        };
        // The order is even only when the base went first in half the pairs.
        let uneven = pair.pairs > 0 && pair.base_first * 2 != pair.pairs;
        lopsided |= uneven;
        let change = format!(
            "{shown} ({}, {} base first){}",
            pair.pairs,
            pair.base_first,
            if uneven { "*" } else { " " }
        );
        let _ = writeln!(
            text,
            "{name:<name_width$}  {}  {}  {change:>24}",
            cell(0),
            cell(1)
        );
    }
    if lopsided {
        text.push_str(
            "* the base did not go first in half of the paired rounds: its change is partly \
             the order's (the first side pays for a cold disk cache)\n",
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
             its median and of the paired change. A failed clean, setup or build ends the \
             side's round; a failed run of the tests does not, and the tiers' sums and `the \
             job` of that round count as failed"
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
    if record.stopped.is_some()
        && let Some(said) = one_order_only(record)
    {
        return Err(said);
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

/// What is said of a run that ended before it had a whole round in each
/// order (`--minutes` stopped it): its changes are of the order's as much as
/// of the trees. `None` for a run that has both.
pub(crate) fn one_order_only(record: &Record) -> Option<String> {
    let whole = |round: usize| {
        [Which::Base, Which::Head].iter().all(|which| {
            record
                .sides
                .iter()
                .any(|side| side.round == round && side.which == *which)
        })
    };
    let mut rounds: Vec<usize> = record.sides.iter().map(|side| side.round).collect();
    rounds.sort_unstable();
    rounds.dedup();
    let mut first = [false; 2];
    for round in rounds.into_iter().filter(|round| whole(*round)) {
        first[order(round - 1)[0] as usize] = true;
    }
    (first != [true; 2]).then(|| {
        format!(
            "the run has no whole round {}: every change it printed is of one order, and the \
             first side pays for what the second finds ready",
            if first[0] {
                "with the head first"
            } else if first[1] {
                "with the base first"
            } else {
                "at all"
            }
        )
    })
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
    for folder in ["ci", BUILT_ASIDE, "tmp"] {
        let path = tree.join("target").join(folder);
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .map_err(|error| format!("could not remove {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

/// Where an archive side's build of the tests is set aside while its
/// archive is extracted in its place, under `target/`: put back before the
/// side's next build, so that `--keep-build` keeps it.
const BUILT_ASIDE: &str = "ci-built";

/// The archive an archive side's build writes, under its tree.
fn archive_file(tree: &Path) -> PathBuf {
    tree.join("target")
        .join("job-ab-archive")
        .join("tests.tar.zst")
}

/// The Windows test jobs' runs from an archive extracted at `tree`: tiers 1
/// and 2, then the system tests, each with the filter its job gives it.
pub(crate) fn archive_runs(tree: &Path) -> Vec<Vec<String>> {
    use crate::checks::{SYSTEM_TESTS, from_archive_run};
    vec![
        from_archive_run(tree, &format!("not {SYSTEM_TESTS}")),
        from_archive_run(tree, SYSTEM_TESTS),
    ]
}

/// The side's build put back where the extracted archive was, if it was
/// set aside.
fn put_back(tree: &Path) -> Result<()> {
    let target = tree.join("target");
    let (built, aside) = (target.join("ci"), target.join(BUILT_ASIDE));
    if !aside.exists() {
        return Ok(());
    }
    if built.exists() {
        std::fs::remove_dir_all(&built)
            .map_err(|error| format!("could not remove {}: {error}", built.display()))?;
    }
    std::fs::rename(&aside, &built)
        .map_err(|error| format!("could not move {} back: {error}", aside.display()))
}

/// The side's build set aside, so that the archive's extraction has the
/// place to itself, as on a fresh runner.
fn set_build_aside(tree: &Path) -> Result<()> {
    let target = tree.join("target");
    let (built, aside) = (target.join("ci"), target.join(BUILT_ASIDE));
    if aside.exists() {
        std::fs::remove_dir_all(&aside)
            .map_err(|error| format!("could not remove {}: {error}", aside.display()))?;
    }
    if built.exists() {
        std::fs::rename(&built, &aside)
            .map_err(|error| format!("could not move {} aside: {error}", built.display()))?;
    }
    Ok(())
}

/// What one side's round does, phase by phase: [`RealSide`] runs the
/// commands, and a test answers from a list. Each returns how the phase
/// ended; [`drive_side`] times them and records them.
pub(crate) trait SidePhases {
    /// Removes the side's build of the tests.
    fn clean(&mut self) -> Result<()>;
    /// Runs the side's setup: `None` when it has none.
    fn setup(&mut self) -> Option<Result<()>>;
    /// Builds the tests.
    fn build(&mut self) -> Result<()>;
    /// Extracts the side's archive of the tests over its tree: `None` for a
    /// side that runs them where they were built.
    fn extract(&mut self) -> Option<Result<()>> {
        None
    }
    /// How many runs the job makes of them.
    fn runs(&self) -> usize;
    /// Makes run `index`, its report kept for the next.
    fn run(&mut self, index: usize) -> Result<()>;
    /// Each tier's tests' time summed above the start, from the report.
    fn tiers(&mut self) -> Result<[f64; 3]>;
}

/// What a side's round does before its phases are timed, and what its
/// phases are called: worked out here, apart from the commands, so that it
/// is tested.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SidePlan {
    /// Whether the round begins by removing the side's build.
    pub(crate) clean: bool,
    /// Under `--keep-build`, `cold` or `warm`: the round's phases are named
    /// with it (`build (cold)`), so that a cold build and a warm one are
    /// never in one median or one pair.
    pub(crate) temperature: Option<&'static str>,
    /// The side's own setup script, a path in the head's tree.
    pub(crate) setup: Option<PathBuf>,
    /// Whether the side builds an archive of the tests, extracts it over
    /// its tree and runs them from it (`--from-archive`).
    pub(crate) from_archive: bool,
}

/// The plan of `which`'s round `round` (from 1). The build is removed in
/// every round, or under `--keep-build` in the first two alone: both sides
/// start cold whatever their build directories held (on a runner, the cache
/// restores `main`'s build into the head's tree alone), and the two cold
/// rounds put each side first once, as the warm rounds after them do.
pub(crate) fn side_plan(request: &Request, which: Which, round: usize) -> SidePlan {
    let clean = !request.keep_build || round <= COLD_ROUNDS;
    SidePlan {
        clean,
        temperature: request
            .keep_build
            .then_some(if clean { "cold" } else { "warm" }),
        setup: request.setup[which as usize].clone(),
        from_archive: request.from_archive == Some(which),
    }
}

/// The rounds `--keep-build` builds cold: one in each order.
const COLD_ROUNDS: usize = 2;

/// `name` marked with the round's temperature, when it has one.
fn marked(name: &str, temperature: Option<&str>) -> String {
    match temperature {
        Some(temperature) => format!("{name} ({temperature})"),
        None => name.to_owned(),
    }
}

/// `cargo nextest …` in `tree`, with the side's environment set: what the
/// setup's file said.
pub(crate) fn nextest_command(
    tree: &Path,
    arguments: &[String],
    environment: &[(String, String)],
) -> Command {
    let mut command = crate::ab::nextest(tree, arguments);
    command.envs(environment.iter().map(|(key, value)| (key, value)));
    command
}

/// What runs, untimed, in each tree before the first round: the crates'
/// sources fetched and unpacked, and cargo and nextest started once, so that
/// what a first run pays for once (the registry's sources, the toolchain and
/// nextest read into the disk cache) falls on neither side's timed round.
pub(crate) fn warm_up_commands() -> [Vec<String>; 2] {
    [
        vec!["fetch".to_owned(), "--locked".to_owned()],
        vec!["nextest".to_owned(), "--version".to_owned()],
    ]
}

/// One side's round, every phase timed by `clock` (seconds) and recorded. It
/// never fails. A failed clean, setup or build ends the side's round: what
/// would follow would measure something else. A failed run of the tests does
/// not: the job's next run is still made and timed, as the blocking job
/// makes it, and the tiers' sums and `the job` count as failed.
pub(crate) fn drive_side(
    phases: &mut impl SidePhases,
    round: usize,
    which: Which,
    plan: &SidePlan,
    clock: &mut dyn FnMut() -> f64,
) -> SideRound {
    let named = |name: &str| marked(name, plan.temperature);
    let mut side = SideRound {
        round,
        which,
        phases: Vec::new(),
        error: None,
    };
    if plan.clean {
        let started = clock();
        let outcome = phases.clean();
        let took = clock() - started;
        if !phase(&mut side, "clean", took, outcome) {
            return side;
        }
    }
    let job_started = clock();
    let started = clock();
    if let Some(outcome) = phases.setup() {
        let took = clock() - started;
        if !phase(&mut side, "setup", took, outcome) {
            let took = clock() - job_started;
            return job_failed(side, &named(JOB), took);
        }
    }
    let started = clock();
    let outcome = phases.build();
    let took = clock() - started;
    if !phase(&mut side, &named("build"), took, outcome) {
        let took = clock() - job_started;
        return job_failed(side, &named(JOB), took);
    }
    let started = clock();
    if let Some(outcome) = phases.extract() {
        let took = clock() - started;
        if !phase(&mut side, &named("extract"), took, outcome) {
            let took = clock() - job_started;
            return job_failed(side, &named(JOB), took);
        }
    }
    let runs = phases.runs();
    let mut runs_passed = true;
    for index in 0..runs {
        let started = clock();
        let outcome = phases.run(index);
        let took = clock() - started;
        runs_passed &= phase(&mut side, &named(&run_name(index, runs)), took, outcome);
    }
    let job_seconds = clock() - job_started;
    match phases.tiers() {
        Ok(sums) => {
            for (tier, seconds) in Tier::ALL.into_iter().zip(sums) {
                side.phases.push(Phase {
                    name: named(&tier_name(tier)),
                    seconds,
                    passed: runs_passed,
                });
            }
        }
        Err(error) => {
            runs_passed = false;
            add_error(&mut side, error);
        }
    }
    side.phases.push(Phase {
        name: named(JOB),
        seconds: job_seconds,
        passed: runs_passed,
    });
    side
}

/// A side's phases as they really run: commands in its tree.
struct RealSide<'a> {
    head: &'a Path,
    tree: &'a Path,
    which: Which,
    round: usize,
    request: &'a Request,
    /// The side's setup script, from its plan.
    setup: Option<PathBuf>,
    /// What the setup's file set, for the build and the runs.
    environment: Vec<(String, String)>,
    /// The job's runs, as its own `.config/nextest.toml` makes them.
    runs: Vec<Vec<String>>,
    /// Whether the side runs its tests from an archive (`--from-archive`).
    from_archive: bool,
}

impl RealSide<'_> {
    fn command(&self, arguments: &[String]) -> Command {
        nextest_command(self.tree, arguments, &self.environment)
    }
}

impl SidePhases for RealSide<'_> {
    fn clean(&mut self) -> Result<()> {
        clean(self.tree)
    }

    fn setup(&mut self) -> Option<Result<()>> {
        let script = self.setup.as_ref()?;
        let env_file = self.tree.join("target").join("job-ab-env");
        let script = self.head.join(script);
        let (program, before) = setup_program(&script);
        let mut command = Command::new(program);
        command
            .args(before)
            .arg(&script)
            .current_dir(self.tree)
            .env("CINERIS_AB_ENV", &env_file)
            .env("CINERIS_AB_SIDE", side_name(self.which))
            .env("CINERIS_AB_ROUND", self.round.to_string());
        let outcome = std::fs::create_dir_all(self.tree.join("target"))
            .and_then(|()| workspace_forget(&env_file))
            .map_err(|error| format!("could not prepare {}: {error}", env_file.display()))
            .and_then(|()| timed(command, &format!("the setup {}", script.display())).1)
            .and_then(|()| match std::fs::read_to_string(&env_file) {
                Ok(text) => read_env(&text).map(|pairs| self.environment = pairs),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("could not read {}: {error}", env_file.display())),
            });
        Some(outcome)
    }

    fn build(&mut self) -> Result<()> {
        if self.from_archive {
            put_back(self.tree)?;
            let file = archive_file(self.tree);
            workspace::forget_report(&file)?;
            let archive = crate::checks::archive_command(&file);
            timed(self.command(&archive), "the build and its archive").1?;
            return crate::checks::forget_reports(self.tree);
        }
        let mut build = crate::checks::ci_run(&self.request.packages);
        build.push("--no-run".to_owned());
        timed(self.command(&build), "the build").1?;
        crate::checks::forget_reports(self.tree)
    }

    fn extract(&mut self) -> Option<Result<()>> {
        if !self.from_archive {
            return None;
        }
        let tree = self.tree;
        let extract = crate::checks::extract_command(&archive_file(tree), tree);
        Some(
            set_build_aside(tree)
                .and_then(|()| timed(self.command(&extract), "the extraction").1)
                .and_then(|()| {
                    let nextest = tree.join("target").join("nextest");
                    crate::checks::check_built_at(&nextest.join("cargo-metadata.json"), tree)
                })
                .and_then(|()| {
                    // As the test jobs make it: the archive does not hold it.
                    let tmp = tree.join("target").join("ci").join("tmp");
                    std::fs::create_dir_all(&tmp)
                        .map_err(|error| format!("could not make {}: {error}", tmp.display()))
                }),
        )
    }

    fn runs(&self) -> usize {
        self.runs.len()
    }

    fn run(&mut self, index: usize) -> Result<()> {
        let name = run_name(index, self.runs.len());
        let outcome = timed(self.command(&self.runs[index]), &name).1;
        let kept = if index + 1 == self.runs.len() {
            crate::checks::bring_together(self.tree)
        } else {
            crate::checks::set_aside(self.tree)
        };
        match (outcome, kept) {
            (Ok(()), kept) => kept,
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(also)) => Err(format!("{error}; {also}")),
        }
    }

    fn tiers(&mut self) -> Result<[f64; 3]> {
        let report = workspace::nextest_report(self.tree, "ci");
        let cases = junit::read(&report)?;
        let (start, _) = tiers::start_of(&cases);
        let kept = self
            .head
            .join("target")
            .join("ab")
            .join(report_name(self.which, self.round));
        if let Err(error) = std::fs::create_dir_all(kept.parent().unwrap_or(self.head))
            .and_then(|()| std::fs::copy(&report, &kept))
        {
            println!("(could not keep {}: {error})", kept.display());
        }
        Ok(Tier::ALL.map(|tier| tiers::total(&cases, tier, start).above_start))
    }
}

/// `error` added to the side's.
fn add_error(side: &mut SideRound, error: String) {
    side.error = Some(match side.error.take() {
        Some(earlier) => format!("{earlier}; {error}"),
        None => error,
    });
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
            add_error(side, error);
            false
        }
    }
}

/// A side's round that ended in a failed phase: the job, as far as it went.
fn job_failed(mut side: SideRound, name: &str, seconds: f64) -> SideRound {
    side.phases.push(Phase {
        name: name.to_owned(),
        seconds,
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

/// The rounds of `record` (its `rounds`), each side in its turn
/// ([`order`]): `side(which, round)` makes a side's round, counted from 1,
/// which never fails; `keep(record)` is called after every side; `clock`
/// gives seconds. A side that failed is said and the run goes on. A round
/// that the longest so far would carry past `minutes` is not begun, and
/// the record says where it stopped.
pub(crate) fn drive_rounds(
    record: &mut Record,
    minutes: Option<u64>,
    side: &mut dyn FnMut(Which, usize) -> SideRound,
    keep: &mut dyn FnMut(&Record),
    clock: &mut dyn FnMut() -> f64,
) {
    let started = clock();
    let mut longest: Option<f64> = None;
    for round in 0..record.rounds {
        let spent = clock() - started;
        if !round_fits(spent, longest, minutes) {
            record.stopped = Some(format!(
                "Stopped before round {} of {}: {:.0} min spent, and the longest round took \
                 {:.0} min, which would pass --minutes {}.",
                round + 1,
                record.rounds,
                spent / 60.0,
                longest.unwrap_or_default() / 60.0,
                minutes.unwrap_or_default()
            ));
            break;
        }
        let round_started = clock();
        for which in order(round) {
            let made = side(which, round + 1);
            if let Some(error) = &made.error {
                println!("== it failed: {}; carrying on ==", first_line(error));
            }
            record.sides.push(made);
            keep(record);
        }
        let took = clock() - round_started;
        longest = Some(longest.map_or(took, |longest| longest.max(took)));
    }
}

/// Runs [`warm_up_commands`] in `tree`, untimed. One that fails is said, and
/// the rounds go on: a fetch that cannot be made is the build's to fail.
fn warm_up(tree: &Path) {
    for arguments in warm_up_commands() {
        let mut command = Command::new(workspace::cargo());
        command
            .args(&arguments)
            .current_dir(tree)
            .env("CARGO_TARGET_DIR", tree.join("target"))
            .stdout(std::process::Stdio::null());
        let (_, outcome) = timed(command, &format!("cargo {}", arguments.join(" ")));
        if let Err(error) = outcome {
            println!("(the warm-up in {}: {error})", tree.display());
        }
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
    for which in [Which::Base, Which::Head] {
        warm_up(tree(which));
    }
    let started = Instant::now();
    let mut clock = || started.elapsed().as_secs_f64();
    let mut side = |which: Which, round: usize| {
        println!(
            "== round {round} of {}: the {} ==",
            request.rounds,
            match which {
                Which::Base => format!("base ({label})"),
                Which::Head => "head (this tree)".to_owned(),
            }
        );
        let tree = tree(which);
        let plan = side_plan(&request, which, round);
        let mut phases = RealSide {
            head: &head,
            tree,
            which,
            round,
            request: &request,
            setup: plan.setup.clone(),
            environment: Vec::new(),
            runs: if plan.from_archive {
                archive_runs(tree)
            } else {
                crate::checks::ci_runs(
                    &request.packages,
                    &request.rest,
                    crate::checks::splits(tree),
                )
            },
            from_archive: plan.from_archive,
        };
        let mut clock = || started.elapsed().as_secs_f64();
        drive_side(&mut phases, round, which, &plan, &mut clock)
    };
    // After every side, so that a run that is stopped keeps what it made. A
    // record that cannot be written is said, and the run goes on: the log
    // has every line of it too.
    let mut keep = |record: &Record| {
        if let Err(error) = std::fs::create_dir_all(record_file.parent().unwrap_or(&head))
            .and_then(|()| std::fs::write(&record_file, to_json(record)))
        {
            println!("(could not write {}: {error})", record_file.display());
        }
    };
    drive_rounds(
        &mut record,
        request.minutes,
        &mut side,
        &mut keep,
        &mut clock,
    );
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
        JOB, Paired, Phase, Record, Request, SidePhases, SidePlan, SideRound, conclusion,
        drive_rounds, drive_side, from_json, medians, medians_table, nextest_command,
        one_order_only, paired, read_env, render, report_name, report_of_name, round_fits,
        run_name, side_plan, to_json, warm_up_commands,
    };

    /// A plan that cleans or not, with no temperature and no setup.
    fn plan(clean: bool) -> SidePlan {
        SidePlan {
            clean,
            temperature: None,
            setup: None,
            from_archive: false,
        }
    }
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
            "4",
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
        assert_eq!(request.rounds, 4);
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
            2,
            "two rounds when not told: each side first once"
        );
        for (arguments, said) in [
            (&["--count", "2"][..], "say which tree"),
            (&["--base", "b", "--count", "0"], "an even number"),
            (&["--base", "b", "--count", "3"], "an even number"),
            (&["--base", "b", "--count", "1"], "an even number"),
            (&["--base", "b", "--count", "22"], "an even number"),
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
        // Tier 3 ran after tiers 1 and 2 failed in round 2, and counts.
        assert_eq!(rows[3].1[1], (Some(50.0), 3), "tier 3 of the failed round");
        let table = medians_table(&record);
        assert!(table.contains("base (main)"), "{table}");
        assert!(table.contains("110.0 (3/3)"), "{table}");
        assert!(table.contains("30.0 (2/3)"), "{table}");
        // The build paired by round: -10, +370 and -30, three pairs, the
        // base first in rounds 1 and 3: not half, so marked.
        let build = table
            .lines()
            .find(|line| line.starts_with("build"))
            .unwrap();
        assert!(build.ends_with("-10.0 (3, 2 base first)*"), "{build}");
        assert!(
            table.contains("* the base did not go first in half"),
            "{table}"
        );
        // The clean took 1.0 s on both sides: no change, and no minus sign.
        let clean = table
            .lines()
            .find(|line| line.starts_with("clean"))
            .unwrap();
        assert!(clean.contains(" +0.0 (3, "), "{clean}");
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
        assert!(line.trim_end().ends_with("- (0, 0 base first)"), "{line}");
    }

    #[test]
    fn a_change_is_paired_by_round_and_says_which_side_went_first() {
        // Two rounds: the base first in round 1, the head first in round 2.
        let even = record(vec![
            passed(1, Which::Base, 100.0),
            passed(1, Which::Head, 80.0),
            passed(2, Which::Head, 90.0),
            passed(2, Which::Base, 100.0),
        ]);
        assert_eq!(
            paired(&even, "build"),
            Paired {
                change: Some(-15.0),
                pairs: 2,
                base_first: 1
            }
        );
        assert!(
            !medians_table(&even).contains("* the base"),
            "an even order"
        );
        // A round the base failed is no pair: the head's round 2 alone, in
        // which the head went first, is not compared with the base's round 1.
        let mut failed_base = passed(2, Which::Base, 100.0);
        failed_base.phases[1].passed = false;
        let odd = record(vec![
            passed(1, Which::Base, 200.0),
            passed(1, Which::Head, 190.0),
            passed(2, Which::Head, 50.0),
            failed_base,
        ]);
        assert_eq!(
            paired(&odd, "build"),
            Paired {
                change: Some(-10.0),
                pairs: 1,
                base_first: 1
            }
        );
        let medians = medians(&odd);
        let build = &medians.iter().find(|(name, _)| name == "build").unwrap().1;
        assert_eq!(
            (build[0].0, build[1].0),
            (Some(200.0), Some(120.0)),
            "unpaired medians would say -80"
        );
    }

    #[test]
    fn each_side_has_its_own_setup_and_the_first_two_rounds_are_cold_under_keep_build() {
        let request = Request::parse(&strings(&[
            "--base",
            "b",
            "--setup-base",
            "a.sh",
            "--setup-head",
            "h.sh",
        ]))
        .unwrap();
        let base = side_plan(&request, Which::Base, 1);
        assert_eq!(base.setup.as_deref(), Some(std::path::Path::new("a.sh")));
        let head = side_plan(&request, Which::Head, 2);
        assert_eq!(head.setup.as_deref(), Some(std::path::Path::new("h.sh")));
        // Without --keep-build every round is cold, and unmarked.
        assert!(base.clean && head.clean);
        assert_eq!((base.temperature, head.temperature), (None, None));
        let keep =
            Request::parse(&strings(&["--base", "b", "--keep-build", "--count", "4"])).unwrap();
        let rounds: Vec<(bool, Option<&str>)> = (1..=4)
            .map(|round| {
                let plan = side_plan(&keep, Which::Head, round);
                (plan.clean, plan.temperature)
            })
            .collect();
        assert_eq!(
            rounds,
            [
                (true, Some("cold")),
                (true, Some("cold")),
                (false, Some("warm")),
                (false, Some("warm"))
            ],
            "one cold round in each order, then warm ones"
        );
        assert_eq!(side_plan(&keep, Which::Base, 1).setup, None);
        let error =
            Request::parse(&strings(&["--base", "b", "--keep-build", "--count", "2"])).unwrap_err();
        assert!(error.contains("needs `--count 4` or more"), "{error}");
        assert!(Request::parse(&strings(&["--base", "b", "--keep-build"])).is_err());
    }

    #[test]
    fn under_keep_build_cold_and_warm_rounds_are_paired_and_marked_apart() {
        let time = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let mut clock = {
            let time = time.clone();
            move || time.get()
        };
        let keep =
            Request::parse(&strings(&["--base", "b", "--keep-build", "--count", "4"])).unwrap();
        let mut side = |which: Which, round: usize| {
            let mut fake = Fake::new(&[], &time);
            fake.setup = false;
            let mut clock = || time.get();
            drive_side(
                &mut fake,
                round,
                which,
                &side_plan(&keep, which, round),
                &mut clock,
            )
        };
        let mut keep_record = |_: &Record| {};
        let mut record = Record {
            label: "main".to_owned(),
            rounds: 4,
            sides: Vec::new(),
            stopped: None,
        };
        drive_rounds(&mut record, None, &mut side, &mut keep_record, &mut clock);
        assert_eq!(
            names(&record.sides[0])[..3],
            ["clean", "build (cold)", "tests: tiers 1 and 2 (cold)"]
        );
        assert_eq!(
            names(&record.sides[4])[..2],
            ["build (warm)", "tests: tiers 1 and 2 (warm)"]
        );
        for name in [
            "build (cold)",
            "build (warm)",
            "the job (cold)",
            "the job (warm)",
        ] {
            let pair = paired(&record, name);
            assert_eq!((pair.pairs, pair.base_first), (2, 1), "{name}");
        }
        assert_eq!(
            paired(&record, "build").pairs,
            0,
            "never one median over both"
        );
        let table = medians_table(&record);
        assert!(
            !table.contains("* the base"),
            "each kind even in its order: {table}"
        );
        let rows: Vec<&str> = table
            .lines()
            .filter(|line| line.starts_with("build"))
            .collect();
        assert_eq!(rows.len(), 2, "{table}");
    }

    #[test]
    fn the_sides_environment_reaches_its_build_and_its_runs() {
        let environment = vec![("CARGO_PROFILE_CI_DEBUG".to_owned(), "false".to_owned())];
        let command = nextest_command(
            std::path::Path::new("tree"),
            &strings(&["nextest", "run"]),
            &environment,
        );
        let set: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(
            set.contains(&(
                "CARGO_PROFILE_CI_DEBUG".to_owned(),
                Some("false".to_owned())
            )),
            "{set:?}"
        );
        assert!(
            set.iter().any(|(key, _)| key == "CARGO_TARGET_DIR"),
            "its own build directory: {set:?}"
        );
    }

    #[test]
    fn both_trees_are_warmed_before_the_first_round() {
        assert_eq!(
            warm_up_commands(),
            [
                strings(&["fetch", "--locked"]),
                strings(&["nextest", "--version"])
            ]
        );
    }

    #[test]
    fn a_run_stopped_with_one_order_measured_fails() {
        let mut one = record(vec![
            passed(1, Which::Base, 1.0),
            passed(1, Which::Head, 1.0),
        ]);
        assert_eq!(
            one_order_only(&one)
                .as_deref()
                .map(|said| said.contains("with the head first")),
            Some(true)
        );
        assert!(conclusion(&one).is_ok(), "not stopped: a count of its own");
        one.stopped = Some("Stopped before round 2 of 4".to_owned());
        assert!(conclusion(&one).is_err());
        let mut both = record(vec![
            passed(1, Which::Base, 1.0),
            passed(1, Which::Head, 1.0),
            passed(2, Which::Head, 1.0),
            passed(2, Which::Base, 1.0),
        ]);
        both.stopped = Some("Stopped before round 3 of 4".to_owned());
        assert_eq!(one_order_only(&both), None);
        assert!(conclusion(&both).is_ok());
        // A round with one side only is no whole round.
        let half = record(vec![
            passed(1, Which::Base, 1.0),
            passed(1, Which::Head, 1.0),
            passed(2, Which::Head, 1.0),
        ]);
        assert!(one_order_only(&half).is_some());
    }

    /// A side's phases from a list: each phase takes `seconds[phase]` of the
    /// fake clock, and fails when named in `failing`.
    struct Fake {
        failing: Vec<&'static str>,
        setup: bool,
        extract: bool,
        runs: usize,
        made: Vec<String>,
        clock: std::rc::Rc<std::cell::Cell<f64>>,
    }

    impl Fake {
        fn new(failing: &[&'static str], clock: &std::rc::Rc<std::cell::Cell<f64>>) -> Self {
            Fake {
                failing: failing.to_vec(),
                setup: true,
                extract: false,
                runs: 2,
                made: Vec::new(),
                clock: clock.clone(),
            }
        }

        fn step(&mut self, name: &str, seconds: f64) -> crate::Result<()> {
            self.made.push(name.to_owned());
            self.clock.set(self.clock.get() + seconds);
            if self.failing.contains(&name) {
                Err(format!("{name} failed"))
            } else {
                Ok(())
            }
        }
    }

    impl SidePhases for Fake {
        fn clean(&mut self) -> crate::Result<()> {
            self.step("clean", 1.0)
        }
        fn setup(&mut self) -> Option<crate::Result<()>> {
            self.setup.then(|| self.step("setup", 2.0))
        }
        fn build(&mut self) -> crate::Result<()> {
            self.step("build", 100.0)
        }
        fn extract(&mut self) -> Option<crate::Result<()>> {
            self.extract.then(|| self.step("extract", 5.0))
        }
        fn runs(&self) -> usize {
            self.runs
        }
        fn run(&mut self, index: usize) -> crate::Result<()> {
            self.step(&format!("run {index}"), 10.0)
        }
        fn tiers(&mut self) -> crate::Result<[f64; 3]> {
            self.step("tiers", 0.0).map(|()| [1.0, 2.0, 3.0])
        }
    }

    fn names(side: &SideRound) -> Vec<String> {
        side.phases
            .iter()
            .map(|phase| format!("{}{}", phase.name, if phase.passed { "" } else { " !" }))
            .collect()
    }

    #[test]
    fn an_archive_side_is_asked_for_by_its_name_and_alone() {
        let request = Request::parse(&strings(&["--base", "b", "--from-archive", "head"])).unwrap();
        assert_eq!(request.from_archive, Some(Which::Head));
        assert!(side_plan(&request, Which::Head, 1).from_archive);
        assert!(!side_plan(&request, Which::Base, 1).from_archive);
        let request = Request::parse(&strings(&["--base", "b", "--from-archive", "base"])).unwrap();
        assert!(side_plan(&request, Which::Base, 2).from_archive);
        assert!(!side_plan(&request, Which::Head, 2).from_archive);
        assert_eq!(
            Request::parse(&strings(&["--base", "b"]))
                .unwrap()
                .from_archive,
            None
        );
        for (arguments, said) in [
            (
                &["--base", "b", "--from-archive", "both"][..],
                "base or head",
            ),
            (&["--base", "b", "--from-archive"], "needs a side"),
            (
                &[
                    "--base",
                    "b",
                    "--from-archive",
                    "head",
                    "--from-archive",
                    "base",
                ],
                "given twice",
            ),
            (
                &[
                    "--base",
                    "b",
                    "--from-archive",
                    "head",
                    "-p",
                    "cineris-base",
                ],
                "no -p",
            ),
            (
                &[
                    "--base",
                    "b",
                    "--from-archive",
                    "head",
                    "--",
                    "-E",
                    "test(/a/)",
                ],
                "no -p",
            ),
        ] {
            let error = Request::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(said), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn an_archive_side_runs_the_windows_test_jobs_runs_from_its_archive() {
        let tree = std::path::Path::new("/w/tree");
        let runs = super::archive_runs(tree);
        assert_eq!(runs.len(), 2, "tiers 1 and 2, then the system tests");
        for (run, filter) in runs.iter().zip([
            format!("not {}", crate::checks::SYSTEM_TESTS),
            crate::checks::SYSTEM_TESTS.to_owned(),
        ]) {
            assert_eq!(run, &crate::checks::from_archive_run(tree, &filter));
            assert!(
                run.iter().any(|word| word == "--binaries-metadata"),
                "{run:?}"
            );
            assert!(
                !run.iter().any(|word| word == "--cargo-profile"),
                "nothing is built: {run:?}"
            );
        }
    }

    #[test]
    fn an_archive_sides_build_is_set_aside_and_put_back() {
        let scratch = cineris_testkit::Scratch::outside("xtask-job-ab", "aside");
        let tree = scratch.path();
        let target = tree.join("target");
        std::fs::create_dir_all(target.join("ci").join("deps")).unwrap();
        std::fs::write(target.join("ci").join("deps").join("built"), "b").unwrap();
        super::set_build_aside(tree).unwrap();
        assert!(!target.join("ci").exists(), "the place is the archive's");
        assert!(
            target
                .join(super::BUILT_ASIDE)
                .join("deps")
                .join("built")
                .is_file()
        );
        // What the extraction writes there.
        std::fs::create_dir_all(target.join("ci").join("deps")).unwrap();
        std::fs::write(target.join("ci").join("deps").join("extracted"), "e").unwrap();
        super::put_back(tree).unwrap();
        assert!(target.join("ci").join("deps").join("built").is_file());
        assert!(
            !target.join("ci").join("deps").join("extracted").exists(),
            "the extracted copy goes"
        );
        assert!(!target.join(super::BUILT_ASIDE).exists());
        // Nothing set aside: nothing to do.
        super::put_back(tree).unwrap();
        assert!(target.join("ci").join("deps").join("built").is_file());
        // A clean takes the set-aside build too.
        super::set_build_aside(tree).unwrap();
        super::clean(tree).unwrap();
        assert!(!target.join(super::BUILT_ASIDE).exists());
        assert!(!target.join("ci").exists());
    }

    #[test]
    fn an_archive_sides_extraction_is_timed_after_its_build_and_a_failed_one_ends_the_round() {
        let time = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let mut clock = {
            let time = time.clone();
            move || time.get()
        };
        let mut fake = Fake::new(&[], &time);
        fake.setup = false;
        fake.extract = true;
        let side = drive_side(&mut fake, 1, Which::Head, &plan(false), &mut clock);
        assert_eq!(
            names(&side),
            [
                "build",
                "extract",
                "tests: tiers 1 and 2",
                "tests: tier 3",
                "tier 1, unit summed",
                "tier 2, integration summed",
                "tier 3, system summed",
                JOB
            ]
        );
        assert_eq!(side.phases[1].seconds, 5.0);
        assert_eq!(
            side.phases.last().unwrap().seconds,
            125.0,
            "the build, the extraction and the runs"
        );
        let mut fake = Fake::new(&["extract"], &time);
        fake.setup = false;
        fake.extract = true;
        let side = drive_side(&mut fake, 2, Which::Head, &plan(false), &mut clock);
        assert_eq!(names(&side), ["build", "extract !", "the job !"]);
        assert_eq!(fake.made, ["build", "extract"], "no test runs without it");
    }

    #[test]
    fn a_side_times_each_phase_and_a_failed_build_ends_its_round() {
        let time = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let mut clock = {
            let time = time.clone();
            move || time.get()
        };
        let mut fake = Fake::new(&[], &time);
        let side = drive_side(&mut fake, 1, Which::Base, &plan(true), &mut clock);
        assert_eq!(
            names(&side),
            [
                "clean",
                "setup",
                "build",
                "tests: tiers 1 and 2",
                "tests: tier 3",
                "tier 1, unit summed",
                "tier 2, integration summed",
                "tier 3, system summed",
                JOB
            ]
        );
        assert_eq!(side.phases[2].seconds, 100.0);
        assert_eq!(
            side.phases.last().unwrap().seconds,
            122.0,
            "setup, build, runs"
        );
        assert_eq!(side.error, None);

        let mut fake = Fake::new(&["build"], &time);
        let side = drive_side(&mut fake, 2, Which::Head, &plan(false), &mut clock);
        assert_eq!(names(&side), ["setup", "build !", "the job !"]);
        assert_eq!(
            fake.made,
            ["setup", "build"],
            "nothing after a failed build"
        );
        assert_eq!(side.error.as_deref(), Some("build failed"));

        let mut fake = Fake::new(&["setup"], &time);
        let side = drive_side(&mut fake, 2, Which::Head, &plan(true), &mut clock);
        assert_eq!(names(&side), ["clean", "setup !", "the job !"]);
    }

    #[test]
    fn a_failed_run_of_the_tests_does_not_end_the_round() {
        let time = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let mut clock = {
            let time = time.clone();
            move || time.get()
        };
        let mut fake = Fake::new(&["run 0"], &time);
        fake.setup = false;
        let side = drive_side(&mut fake, 1, Which::Head, &plan(false), &mut clock);
        assert_eq!(
            names(&side),
            [
                "build",
                "tests: tiers 1 and 2 !",
                "tests: tier 3",
                "tier 1, unit summed !",
                "tier 2, integration summed !",
                "tier 3, system summed !",
                "the job !"
            ],
            "tier 3 is still run and timed"
        );
        assert_eq!(side.error.as_deref(), Some("run 0 failed"));
    }

    #[test]
    fn a_failed_round_is_recorded_and_the_rounds_go_on_in_turn() {
        let time = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let mut clock = {
            let time = time.clone();
            move || time.get()
        };
        let mut made = Vec::new();
        let mut side = |which: Which, round: usize| {
            made.push((which, round));
            time.set(time.get() + 60.0);
            let failing: &[&'static str] = if which == Which::Base && round == 1 {
                &["build"]
            } else {
                &[]
            };
            let mut fake = Fake::new(failing, &time);
            let mut clock = || time.get();
            drive_side(&mut fake, round, which, &plan(true), &mut clock)
        };
        let mut kept = Vec::new();
        let mut keep = |record: &Record| kept.push(record.sides.len());
        let mut record = Record {
            label: "main".to_owned(),
            rounds: 4,
            sides: Vec::new(),
            stopped: None,
        };
        drive_rounds(&mut record, None, &mut side, &mut keep, &mut clock);
        assert_eq!(
            made,
            [
                (Which::Base, 1),
                (Which::Head, 1),
                (Which::Head, 2),
                (Which::Base, 2),
                (Which::Base, 3),
                (Which::Head, 3),
                (Which::Head, 4),
                (Which::Base, 4)
            ]
        );
        assert_eq!(kept, [1, 2, 3, 4, 5, 6, 7, 8], "kept after every side");
        assert_eq!(record.sides.len(), 8);
        assert!(record.sides[0].error.is_some(), "the failed round is kept");
        assert_eq!(record.stopped, None);
        assert!(conclusion(&record).is_err());
    }

    #[test]
    fn the_rounds_stop_before_one_that_would_pass_the_minutes() {
        let time = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let mut clock = {
            let time = time.clone();
            move || time.get()
        };
        // Each side takes 20 minutes, a round 40: within 100 minutes the
        // second round may start (40 spent + 40) and the third may not.
        let mut side = |which: Which, round: usize| {
            time.set(time.get() + 20.0 * 60.0);
            SideRound {
                round,
                which,
                phases: Vec::new(),
                error: None,
            }
        };
        let mut keep = |_: &Record| {};
        let mut record = Record {
            label: "main".to_owned(),
            rounds: 4,
            sides: Vec::new(),
            stopped: None,
        };
        drive_rounds(&mut record, Some(100), &mut side, &mut keep, &mut clock);
        assert_eq!(record.sides.len(), 4, "two rounds");
        let stopped = record.stopped.unwrap();
        assert!(
            stopped.starts_with("Stopped before round 3 of 4"),
            "{stopped}"
        );
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
