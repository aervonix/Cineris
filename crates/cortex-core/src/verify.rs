//! The project's checks, at the end of a turn that changed files
//! (`docs/architecture/15-efficiency.md`, "Automatic verification"; the
//! design is in `docs/steps/19/answered.md`): the core's [`Verifier`].
//!
//! The loop decides when a round runs and what becomes of its verdict. This
//! module decides which checks run and runs them.
//!
//! - **Nothing runs unless the user trusts the project**, and nothing at the
//!   `read-only` level: a check runs the repository's own code, and nobody
//!   is asked.
//! - **The checks** are the user's own `verify.commands`, then the project
//!   profile's test and check commands that the user accepted
//!   (`Snapshot::runs_unasked`). The same line in the same directory runs
//!   once; at most [`MOST_CHECKS`] run a round.
//! - **Each is planned and decided just before it runs**, its retry too:
//!   the engine is asked, and a `deny` or a stop wins. An `ask` holds back
//!   an entry of the profile and not the user's own line, unless an `ask`
//!   rule of the managed configuration is among what asks. A profile entry is
//!   looked up again first, so one turned off elsewhere does not run.
//! - **Each runs through the session's own shell** (`RecordingShell`, as
//!   the harness's run): the same runner, sandbox, environment, looks at
//!   protected paths and checkpoint as the model's commands.
//! - **Time:** each runs with what is left of the round's limit, the
//!   turn's, and the turn's and session's own time limits, and no longer
//!   than `shell.max_timeout`. One that usually takes longer than the
//!   round's limit or `shell.max_timeout`, or longer than what is left, is
//!   skipped. One stopped at its limit is a failure for the model only when
//!   it usually takes less than half of what it had.
//! - **A known flaky test** that fails runs once more, unless it was
//!   stopped at its time limit.
//! - **What the model is told** of a failure is bounded, and every line of
//!   a check's output is escaped and marked as the command's.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cortex_agent::{Decision, PlannedCall, Policy, Verifier, VerifyCx, VerifyOutcome};
use cortex_base::{CancelToken, PathResolver, SharedClock, visible, visible_keeping};
use cortex_config::schema::{VerifyMode, VerifySettings};
use cortex_knowledge::{Kind, Purpose, Snapshot, normal_line};
use cortex_protocol::event::{
    CheckOutcome, CheckReport, CheckSource, Verification, VerificationNotice, VerificationPhase,
};
use cortex_protocol::ids::EntryId;
use cortex_protocol::item::{ArtifactRef, ToolOutcome};
use cortex_protocol::{CallId, ItemId, Level, TurnId};
use cortex_tools::output::{self, Compression, Keep, OutputLimits};
use cortex_tools::shell::CommandProfile;
use cortex_tools::{
    BoxFuture, Concurrency, ContentStore, FileTable, Json, NoOutput, PlanCx, RunCx, Tool,
    ToolResult, run_blocking,
};
use serde_json::json;

use crate::checkpoints::{AsCheck, RecordingShell};
use crate::knowledge::Knowledge;
use crate::policy::EnginePolicy;
use crate::sandbox::SessionSandbox;
use crate::store::Hooks;

/// The most checks one round runs.
pub(crate) const MOST_CHECKS: usize = 8;

/// The most bytes of checks' output one report to the model holds, shared
/// between the checks that failed.
const REPORT_BYTES: usize = 16 * 1024;

/// The most failed checks whose output a report holds: [`REPORT_BYTES`]
/// in more shares would leave each too little to show what failed. Any
/// more are named, with where all of what they printed is.
const MOST_SHOWN: usize = 3;

/// The most lines of one check's output a report holds.
const MOST_LINES: usize = 400;

/// The least time a check is started with when how long it usually takes is
/// not known.
const LEAST_TIME: Duration = Duration::from_secs(10);

/// The longest line of `verify.commands` that runs.
const LONGEST_LINE: usize = 400;

/// How the sandbox's launcher begins the line it gives a command it refused
/// to start (`cortex_sandbox::explain`): a check it refused never ran.
const LAUNCHER_STOPPED: &str = "[The sandbox launcher stopped:";

/// What is said of a check the sandbox may have stopped, to the user and
/// to the model.
const SANDBOX_REMEDY: &str = "A check that needs the network (to fetch dependencies, say), or to \
                              write outside the workspace, runs only outside the sandbox (once, \
                              by hand) or with `sandbox.network = \"full\"` or `sandbox.write` in \
                              the user's own configuration.";

/// What begins every line of a check's output in the report to the model:
/// no line of it can begin with anything of the report's own.
pub(crate) const MARK: &str = "| ";

/// `[verify]`, as the verifier reads it, with the longest time limit the
/// shell gives a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    pub(crate) mode: VerifyMode,
    pub(crate) commands: Vec<String>,
    pub(crate) round_time: Duration,
    pub(crate) turn_time: Duration,
    /// `shell.max_timeout`: a check runs no longer than this, whatever its
    /// round leaves, and is said to have had no more.
    pub(crate) max_timeout: Duration,
}

impl Settings {
    pub(crate) fn of(verify: &VerifySettings, max_timeout: Duration) -> Self {
        Self {
            mode: verify.mode,
            commands: verify.commands.clone(),
            round_time: verify.round_time,
            turn_time: verify.turn_time,
            max_timeout,
        }
    }
}

/// One check: a command line, and the directory it runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Check {
    pub(crate) line: String,
    /// Relative to the workspace root, with `/`; `None` is the root.
    pub(crate) cwd: Option<String>,
    pub(crate) source: CheckSource,
    /// The profile's entry it came from.
    pub(crate) entry: Option<EntryId>,
    /// How long it usually takes, by the profile.
    pub(crate) usual: Option<Duration>,
}

/// A check's line and directory, as two runs of it share them.
type Key = (String, Option<String>);

impl Check {
    fn key(&self) -> Key {
        (key_line(&self.line), self.cwd.clone())
    }
}

/// A line as the profile keeps it, when it is one the profile can hold.
fn key_line(line: &str) -> String {
    normal_line(line).unwrap_or_else(|| line.trim().to_owned())
}

/// What a round will run, and what it was told of the rest.
#[derive(Debug, Default)]
pub(crate) struct Chosen {
    pub(crate) checks: Vec<Check>,
    /// Lines of `verify.commands` that cannot run, and why.
    pub(crate) refused: Vec<String>,
    /// How many were past [`MOST_CHECKS`].
    pub(crate) left_out: usize,
    /// The checks a known flaky test earns a second run.
    pub(crate) flaky: BTreeSet<Key>,
}

/// Chooses a round's checks: the user's own lines, then the profile's
/// accepted test and check commands. Pure: the snapshot is what the
/// project's files held when it was read, with the time by the profile's
/// clock; `None` for a session with no store.
pub(crate) fn choose(
    settings: &Settings,
    profile: Option<(&Snapshot, cortex_base::UtcTime)>,
) -> Chosen {
    let snapshot = profile.map(|(snapshot, _)| snapshot);
    let mut chosen = Chosen::default();
    let usual_of = |key: &Key| -> Option<Duration> {
        snapshot?.entries().iter().find_map(|entry| {
            let command = entry.command.as_ref()?;
            (entry.kind == Kind::Command && key_line(&command.run) == key.0 && command.cwd == key.1)
                .then_some(command.usual_ms)
                .flatten()
                .map(Duration::from_millis)
        })
    };
    let mut all: Vec<Check> = Vec::new();
    for line in &settings.commands {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.chars().count() > LONGEST_LINE
            || trimmed.chars().any(char::is_control)
        {
            chosen.refused.push(format!(
                "`verify.commands` holds a line that is not run: {}. A check is one line of at \
                 most {LONGEST_LINE} characters, with no control character.",
                visible(line)
            ));
            continue;
        }
        let mut check = Check {
            line: trimmed.to_owned(),
            cwd: None,
            source: CheckSource::User,
            entry: None,
            usual: None,
        };
        check.usual = usual_of(&check.key());
        all.push(check);
    }
    if let Some((snapshot, now)) = profile {
        for entry in snapshot.runs_unasked(now) {
            let Some(command) = &entry.command else {
                continue;
            };
            if !matches!(command.purpose, Purpose::Test | Purpose::Check) {
                continue;
            }
            all.push(Check {
                line: command.run.clone(),
                cwd: command.cwd.clone(),
                source: CheckSource::Profile,
                entry: Some(entry.id.clone()),
                usual: command.usual_ms.map(Duration::from_millis),
            });
        }
        for entry in snapshot.in_use(now) {
            if let (Kind::Flaky, Some(command)) = (entry.kind, &entry.command) {
                chosen
                    .flaky
                    .insert((key_line(&command.run), command.cwd.clone()));
            }
        }
    }
    let mut seen: BTreeSet<Key> = BTreeSet::new();
    for check in all {
        if !seen.insert(check.key()) {
            continue;
        }
        if chosen.checks.len() == MOST_CHECKS {
            chosen.left_out += 1;
        } else {
            chosen.checks.push(check);
        }
    }
    chosen
}

/// How one run of a check ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ended {
    Passed,
    /// A failure the model is told of.
    Failed,
    /// Stopped at its limit, and not taken for the model's doing.
    TimedOut,
    /// Failed, and passed when run again.
    Flaky,
    /// The sandbox blocked something it needed.
    Blocked,
    /// Not started, for its time.
    Skipped,
    /// Not allowed, or not started.
    NotRun,
    /// The turn was stopped: by this check, or before it.
    Stopped(String),
    /// The turn was interrupted.
    Interrupted,
}

/// One check of a round, as it ended.
#[derive(Debug, Clone)]
pub(crate) struct Ran {
    pub(crate) ended: Ended,
    pub(crate) exit_code: Option<i32>,
    pub(crate) signal: Option<String>,
    pub(crate) duration: Option<Duration>,
    /// What else to say of it, in words, already escaped.
    pub(crate) note: Option<String>,
    /// What it printed, as its result holds it, for a check that failed.
    pub(crate) output: Option<String>,
    pub(crate) artifact: Option<ArtifactRef>,
    /// Whether it was stopped at its time limit: such a check is not run
    /// again as a flaky test.
    pub(crate) stopped_at_limit: bool,
    /// The turn was stopped after it ran: the look at protected paths after
    /// it, or before it, found a change. Its own result stands beside this.
    pub(crate) stop: Option<String>,
}

impl Ran {
    pub(crate) fn not(ended: Ended, note: impl Into<String>) -> Self {
        Self {
            ended,
            exit_code: None,
            signal: None,
            duration: None,
            note: Some(note.into()),
            output: None,
            artifact: None,
            stopped_at_limit: false,
            stop: None,
        }
    }

    /// Whether it was started.
    fn started(&self) -> bool {
        matches!(
            self.ended,
            Ended::Passed | Ended::Failed | Ended::TimedOut | Ended::Flaky | Ended::Blocked
        )
    }
}

/// Reads how a check's run ended from its result: what the shell's payload
/// says.
pub(crate) fn classify(result: &ToolResult, limit: Duration, usual: Option<Duration>) -> Ran {
    let payload = result.payload();
    let field = |key: &str| payload.and_then(|payload| payload.get(key));
    let flag = |key: &str| field(key).and_then(Json::as_bool).unwrap_or(false);
    let mut ran = Ran {
        ended: Ended::Failed,
        exit_code: field("exit_code")
            .and_then(Json::as_i64)
            .and_then(|code| i32::try_from(code).ok()),
        signal: field("signal").and_then(Json::as_str).map(str::to_owned),
        duration: field("duration_ms")
            .and_then(Json::as_u64)
            .map(Duration::from_millis),
        note: None,
        output: None,
        artifact: result.artifacts().first().cloned(),
        stopped_at_limit: flag("timed_out"),
        stop: None,
    };
    let mut notes: Vec<String> = Vec::new();
    match result.outcome() {
        ToolOutcome::Interrupted => {
            ran.ended = Ended::Interrupted;
            return ran;
        }
        // Refused before it ran. Whether the turn was stopped is the
        // caller's to see.
        ToolOutcome::Denied => {
            ran.ended = Ended::NotRun;
            ran.note = Some(visible(&result.text()).into_owned());
            return ran;
        }
        _ => {}
    }
    if payload.is_none() {
        // It never started: a program that is not there, a launcher that
        // refused it. Not the model's doing.
        ran.ended = Ended::NotRun;
        ran.note = Some(visible(&result.text()).into_owned());
        return ran;
    }
    // What the launcher said its confinement did. Only its own refusal
    // (the command never ran) is the sandbox's for certain: its other lines
    // are read from what the command printed, and a compiler's "failed to
    // resolve" reads like a name that could not be looked up. So those
    // leave the result as it is, and are said beside it.
    let sandbox = field("sandbox").and_then(Json::as_str);
    let refused = sandbox.is_some_and(|line| line.starts_with(LAUNCHER_STOPPED));
    ran.ended = if ran.stopped_at_limit {
        // It hung only if it usually ends well within what it had.
        if usual.is_some_and(|usual| usual.saturating_mul(2) <= limit) {
            notes.push(format!(
                "It was stopped at its time limit of {}, and it usually takes {}.",
                shown_time(limit),
                shown_time(usual.unwrap_or_default())
            ));
            Ended::Failed
        } else {
            notes.push(format!(
                "It was stopped at its time limit of {}.",
                shown_time(limit)
            ));
            Ended::TimedOut
        }
    } else if refused && ran.exit_code != Some(0) {
        Ended::Blocked
    } else if ran.exit_code == Some(0) && *result.outcome() == ToolOutcome::Success {
        Ended::Passed
    } else {
        Ended::Failed
    };
    if let Some(line) = sandbox
        && ran.ended != Ended::Passed
    {
        // The launcher's refusal says why itself; a guess from the output
        // is said with what a check that needs more than the sandbox gives
        // can do.
        notes.push(if refused {
            visible(line).into_owned()
        } else {
            format!("{} {SANDBOX_REMEDY}", visible(line))
        });
    }
    if flag("background_stopped") {
        notes.push("It left processes running, and they were stopped.".to_owned());
    }
    if flag("outlived") {
        notes.push(
            "A process it started is still running outside its process tree, and could not be \
             stopped."
                .to_owned(),
        );
    }
    if !notes.is_empty() {
        ran.note = Some(notes.join(" "));
    }
    if matches!(ran.ended, Ended::Failed) {
        ran.output = Some(result.text());
    }
    ran
}

/// The time a check may have, `in_round` into its round: what is left of
/// the round's limit, of the turn's (`used` by its earlier rounds), and of
/// the turn's and the session's own time limits (`deadline`, as the round
/// began); and no more than the shell gives a command (`max_timeout`).
pub(crate) fn time_left(
    settings: &Settings,
    used: Duration,
    deadline: Option<Duration>,
    in_round: Duration,
) -> Duration {
    let mut left = settings
        .round_time
        .saturating_sub(in_round)
        .min(settings.turn_time.saturating_sub(used + in_round));
    if let Some(deadline) = deadline {
        left = left.min(deadline.saturating_sub(in_round));
    }
    left.min(settings.max_timeout)
}

/// Why a check is not started for its time, when it is not: it usually
/// takes longer than a command may run (`shell.max_timeout`), or than a
/// round's limit (15-efficiency's rule), or than what is `left` (at least
/// [`LEAST_TIME`] when its usual time is not known).
pub(crate) fn too_long(
    usual: Option<Duration>,
    round_time: Duration,
    max_timeout: Duration,
    left: Duration,
) -> Option<String> {
    if let Some(usual) = usual
        && usual > max_timeout
    {
        return Some(format!(
            "It usually takes {}, longer than a command may run (`shell.max_timeout`, {}).",
            shown_time(usual),
            shown_time(max_timeout)
        ));
    }
    if let Some(usual) = usual
        && usual > round_time
    {
        return Some(format!(
            "It usually takes {}, longer than a round's limit of {}.",
            shown_time(usual),
            shown_time(round_time)
        ));
    }
    let needs = usual.unwrap_or(LEAST_TIME);
    (left < needs || left.is_zero()).then(|| {
        format!(
            "Not enough of the round's time was left: {}.",
            shown_time(left)
        )
    })
}

/// A length of time as the reports say it.
fn shown_time(time: Duration) -> String {
    let seconds = time.as_secs();
    if seconds >= 60 && seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else if time.subsec_millis() == 0 || seconds >= 10 {
        format!("{seconds}s")
    } else {
        format!("{:.1}s", time.as_secs_f64())
    }
}

/// A check's line and directory, for a person or the model: escaped.
fn named(check: &Check) -> String {
    match &check.cwd {
        Some(cwd) => format!("`{}` (in {})", visible(&check.line), visible(cwd)),
        None => format!("`{}`", visible(&check.line)),
    }
}

/// What the model is told of a round that failed.
///
/// `earlier` are the checks that failed in an earlier round of the turn and
/// did not run to a result in this one. Every line of a check's output is
/// marked [`MARK`] and escaped.
pub(crate) fn report(
    round: u32,
    rounds: u32,
    results: &[(Check, Ran)],
    earlier: &[&Check],
) -> String {
    let mut text = format!(
        "The project's checks failed after your changes (round {round} of {rounds}). Fix what \
         they report. When you end your turn having changed files, they run again."
    );
    let failed: Vec<&(Check, Ran)> = results
        .iter()
        .filter(|(_, ran)| ran.ended == Ended::Failed)
        .collect();
    let share = REPORT_BYTES / failed.len().clamp(1, MOST_SHOWN);
    for (shown, (check, ran)) in failed.iter().enumerate() {
        let how = match (&ran.exit_code, &ran.signal) {
            (Some(code), _) => format!("exit code {code}"),
            (None, Some(signal)) => format!("signal {}", visible(signal)),
            (None, None) => "no exit code".to_owned(),
        };
        let after = ran
            .duration
            .map(|duration| format!(" after {}", shown_time(duration)))
            .unwrap_or_default();
        let _ = write!(text, "\n\n{} failed with {how}{after}.", named(check));
        if let Some(note) = &ran.note {
            let _ = write!(text, " {note}");
        }
        if shown < MOST_SHOWN {
            text.push_str(" What it printed, each line marked `| `:\n");
            text.push_str(&printed_block(ran.output.as_deref().unwrap_or(""), share));
        } else {
            text.push_str(" What it printed is not shown here, for room.");
        }
        if let Some(artifact) = &ran.artifact {
            let _ = write!(
                text,
                "\nAll of what it printed is the artifact `{}`.",
                visible(artifact.id.as_str())
            );
        }
    }
    for check in earlier {
        let _ = write!(
            text,
            "\n\n{} failed in an earlier round and did not run to a result in this one.",
            named(check)
        );
    }
    let mut others = String::new();
    for (check, ran) in results {
        let said = match ran.ended {
            Ended::Passed => "passed".to_owned(),
            Ended::Flaky => {
                "failed, then passed when run again: it is a known flaky test".to_owned()
            }
            Ended::TimedOut => {
                "was stopped at its time limit, which is not taken for a failure of \
                                your changes"
                    .to_owned()
            }
            Ended::Blocked => {
                "was blocked by the sandbox, which is not a failure of your changes".to_owned()
            }
            Ended::Skipped => "was not run, for its time".to_owned(),
            Ended::NotRun => "was not run".to_owned(),
            Ended::Failed | Ended::Stopped(_) | Ended::Interrupted => continue,
        };
        let _ = write!(others, "\n{} {said}.", named(check));
    }
    if !others.is_empty() {
        text.push('\n');
        text.push_str(&others);
    }
    text
}

/// What a failed check printed, as the model is sent it: shaped by the
/// lines that matter, every line escaped and marked [`MARK`], in at most
/// `share` bytes **as sent**. A line of unseen characters is longer escaped
/// than printed, so the output is shaped again, with fewer lines, until the
/// marked text fits: the lines that report a problem and the end stay,
/// which cutting the marked text would lose.
pub(crate) fn printed_block(printed: &str, share: usize) -> String {
    let mark = |text: &str| -> String {
        text.split('\n')
            .map(|line| format!("{MARK}{}", visible_keeping(line, &['\t'])))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut bytes = share.saturating_sub(MARK.len() * MOST_LINES);
    let mut lines = MOST_LINES;
    let mut block = String::new();
    for _ in 0..8 {
        let shaped = output::shape(
            printed,
            &OutputLimits::new(bytes, lines, 2_000),
            Keep::Important,
            Compression::Structural,
        );
        block = mark(&shaped.text);
        if block.len() <= share {
            return block;
        }
        // Shaped again with as many fewer lines as the marked text is over.
        let shown = shaped.text.split('\n').count();
        lines = (shown * share / block.len() * 9 / 10).max(8);
        bytes = (bytes * share / block.len()).max(1);
    }
    // Output whose every line is long escaped: whole lines from the end,
    // where a run says how it went, and a line that says more was cut.
    let all: Vec<&str> = block.split('\n').collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 64;
    for line in all.iter().rev() {
        if used + line.len() + 1 > share {
            break;
        }
        used += line.len() + 1;
        kept.push(line);
    }
    kept.reverse();
    format!(
        "{MARK}… ({} lines before these are not shown)\n{}",
        all.len() - kept.len(),
        kept.join("\n")
    )
}

/// The protocol's account of a check.
fn check_report(check: &Check, ran: Option<&Ran>) -> CheckReport {
    CheckReport {
        command: check.line.clone(),
        cwd: check.cwd.clone(),
        source: check.source.clone(),
        entry: check.entry.clone(),
        outcome: ran.map(|ran| match &ran.ended {
            Ended::Passed => CheckOutcome::Passed,
            Ended::Failed => CheckOutcome::Failed,
            Ended::TimedOut => CheckOutcome::TimedOut,
            Ended::Flaky => CheckOutcome::Flaky,
            Ended::Blocked => CheckOutcome::Blocked,
            Ended::Skipped => CheckOutcome::Skipped,
            Ended::NotRun | Ended::Stopped(_) | Ended::Interrupted => CheckOutcome::NotRun,
        }),
        exit_code: ran.and_then(|ran| ran.exit_code),
        duration_ms: ran
            .and_then(|ran| ran.duration)
            .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
        note: ran.and_then(|ran| match (&ran.ended, &ran.stop) {
            (Ended::Stopped(reason), _) => Some(visible(reason).into_owned()),
            (_, Some(reason)) => Some(
                format!(
                    "{} The turn stopped after it: {}",
                    ran.note.clone().unwrap_or_default(),
                    visible(reason)
                )
                .trim_start()
                .to_owned(),
            ),
            _ => ran.note.clone(),
        }),
        artifact: ran.and_then(|ran| ran.artifact.clone()),
    }
}

/// The checks that failed in a turn and have not passed since.
#[derive(Debug, Default)]
pub(crate) struct Memory {
    turn: Option<TurnId>,
    failing: Vec<Check>,
}

/// A round's verdict, with what is left failing in its turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Passed,
    /// A check failed in this round: the model is told.
    Failed,
    /// No check failed in this round, and one that failed earlier in the
    /// turn has not passed since: nothing new to tell the model.
    StillFailing,
    Incomplete,
    Skipped,
    /// A check stopped the turn.
    Stopped(String),
}

impl Memory {
    /// The checks that fail in `turn`, now that a round ended with
    /// `results`; and the round's verdict. `chosen` is how many checks the
    /// round chose. Pure: it only reads and writes the memory.
    pub(crate) fn judge(
        &mut self,
        turn: &TurnId,
        chosen: usize,
        results: &[(Check, Ran)],
    ) -> (Verdict, Vec<Check>) {
        if self.turn.as_ref() != Some(turn) {
            *self = Self {
                turn: Some(turn.clone()),
                failing: Vec::new(),
            };
        }
        let mut failed_now = false;
        for (check, ran) in results {
            match ran.ended {
                Ended::Passed | Ended::Flaky => {
                    self.failing.retain(|held| held.key() != check.key());
                }
                Ended::Failed => {
                    failed_now = true;
                    if !self.failing.iter().any(|held| held.key() == check.key()) {
                        self.failing.push(check.clone());
                    }
                }
                _ => {}
            }
        }
        let started = results.iter().filter(|(_, ran)| ran.started()).count();
        let all_passed = results.len() == chosen
            && chosen > 0
            && results
                .iter()
                .all(|(_, ran)| matches!(ran.ended, Ended::Passed | Ended::Flaky));
        // A stop is the round's verdict; the result of the check that made
        // it, recorded above, still counts for what fails.
        let stopped = results.iter().find_map(|(_, ran)| match &ran.ended {
            Ended::Stopped(reason) => Some(reason.clone()),
            _ => ran.stop.clone(),
        });
        let verdict = if let Some(reason) = stopped {
            Verdict::Stopped(reason)
        } else if failed_now {
            Verdict::Failed
        } else if !self.failing.is_empty() {
            Verdict::StillFailing
        } else if all_passed {
            Verdict::Passed
        } else if started == 0 {
            Verdict::Skipped
        } else {
            Verdict::Incomplete
        };
        (verdict, self.failing.clone())
    }

    /// The verdict of a round that ran nothing in `turn`: what is still
    /// failing in it, if anything is.
    pub(crate) fn unresolved(&self, turn: &TurnId) -> bool {
        self.turn.as_ref() == Some(turn) && !self.failing.is_empty()
    }
}

/// Whether a check that ended as `first` runs once more: it failed, and is
/// a known flaky test, and it was not stopped at its time limit (a test
/// that hangs is not retried).
pub(crate) fn retries(first: &Ran, flaky: bool) -> bool {
    first.ended == Ended::Failed && flaky && !first.stopped_at_limit
}

/// What a check that failed and was run again ended as: flaky when the
/// second run passed, a failure when it failed; and when the second run
/// did not run to a result, the first failure, with a note.
pub(crate) fn after_retry(first: Ran, second: Ran) -> Ran {
    // A stop the second run met, before it or after it, goes with the
    // result, whichever run's that is.
    let stop = second.stop.clone().or_else(|| match &second.ended {
        Ended::Stopped(reason) => Some(reason.clone()),
        _ => None,
    });
    let mut ran = match second.ended {
        Ended::Passed => Ran {
            ended: Ended::Flaky,
            note: Some(format!(
                "It failed with {}, then passed when run again: it is a known flaky test.",
                first.exit_code.map_or_else(
                    || "no exit code".to_owned(),
                    |code| format!("exit code {code}")
                )
            )),
            ..second
        },
        // A failure after the retry is a failure, never "flaky".
        Ended::Failed => second,
        _ => {
            let why = second.note.unwrap_or_default();
            Ran {
                note: Some(
                    format!(
                        "{} It is a known flaky test, and was to run once more, which it did \
                         not: {why}",
                        first.note.clone().unwrap_or_default()
                    )
                    .trim_start()
                    .to_owned(),
                ),
                ..first
            }
        }
    };
    if stop.is_some() {
        ran.stop = stop;
    }
    ran
}

/// What the loop is told of a round's verdict, and what front ends are:
/// a stop says whether a check of the turn still fails.
pub(crate) fn outcome_of(
    verdict: Verdict,
    failing: &[Check],
    report: impl FnOnce() -> String,
) -> (VerifyOutcome, Verification) {
    let failed = !failing.is_empty();
    match verdict {
        Verdict::Stopped(reason) => (
            VerifyOutcome::Stopped {
                reason,
                failing: failed,
            },
            if failed {
                Verification::Failed
            } else {
                Verification::Incomplete
            },
        ),
        Verdict::Failed => (
            VerifyOutcome::Failed { report: report() },
            Verification::Failed,
        ),
        Verdict::StillFailing => (VerifyOutcome::StillFailing, Verification::Failed),
        Verdict::Passed => (VerifyOutcome::Passed, Verification::Passed),
        Verdict::Skipped => (VerifyOutcome::Skipped, Verification::Skipped),
        Verdict::Incomplete => (VerifyOutcome::Incomplete, Verification::Incomplete),
    }
}

/// Everything a session's verifier is made of.
pub(crate) struct Parts {
    pub(crate) settings: Settings,
    /// The session's shell. `None` when no shell was found: nothing runs.
    pub(crate) shell: Option<Arc<RecordingShell>>,
    pub(crate) policy: Arc<EnginePolicy>,
    pub(crate) knowledge: Option<Arc<Knowledge>>,
    /// Whether the user trusts the project.
    pub(crate) trusted: bool,
    /// Whether a project's file turned `verify.mode` down.
    pub(crate) lowered_by_project: bool,
    pub(crate) sandbox: Arc<SessionSandbox>,
    pub(crate) hooks: Arc<Hooks>,
    pub(crate) paths: Arc<PathResolver>,
    pub(crate) content: Arc<dyn ContentStore>,
    pub(crate) files: Arc<FileTable>,
    pub(crate) limits: OutputLimits,
    pub(crate) clock: SharedClock,
}

/// Why a round would run no check, before its time is looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unready {
    /// No line of the user's and no accepted entry of the profile.
    NoneKnown,
    /// The user has not trusted the project.
    Untrusted,
    /// The session is at `read-only`.
    ReadOnly,
    /// No shell was found.
    NoShell,
}

impl Unready {
    /// What the user is told.
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::NoneKnown => {
                "No check could be chosen for this project: `verify.commands` has no line that \
                 can run, and the project's profile holds no test or check command you accepted."
            }
            Self::Untrusted => {
                "The project's checks do not run: they run the repository's own code, and you \
                 have not trusted this project. `cortex permissions trust` trusts it."
            }
            Self::ReadOnly => "The project's checks do not run at the read-only level.",
            Self::NoShell => "The project's checks do not run: no shell was found.",
        }
    }
}

/// What a reload of the configuration changes of a session's verifier
/// (step 24): the seam step 19 left. A round reads it as it begins, and
/// keeps what it read to its end.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VerifyLive {
    pub(crate) settings: Settings,
    /// Whether the user trusts the project.
    pub(crate) trusted: bool,
    /// Whether a project's file turned `verify.mode` down.
    pub(crate) lowered_by_project: bool,
}

/// A session's verifier: the project's checks, chosen and run.
pub(crate) struct CoreVerifier {
    parts: Parts,
    live: Mutex<VerifyLive>,
    memory: Mutex<Memory>,
    told: Mutex<BTreeSet<String>>,
}

impl std::fmt::Debug for CoreVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreVerifier")
            .field("live", &*lock(&self.live))
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the round is in, for each of its checks.
struct Round<'r, 'a> {
    cx: &'r VerifyCx<'a>,
    shell: &'r Arc<RecordingShell>,
    /// The settings the round began with.
    settings: Settings,
    /// When the round started, on the monotonic clock.
    started: Duration,
    /// How many checks it has planned, for their ids.
    planned: u32,
}

impl CoreVerifier {
    pub(crate) fn new(parts: Parts) -> Self {
        let live = VerifyLive {
            settings: parts.settings.clone(),
            trusted: parts.trusted,
            lowered_by_project: parts.lowered_by_project,
        };
        Self {
            parts,
            live: Mutex::new(live),
            memory: Mutex::new(Memory::default()),
            told: Mutex::new(BTreeSet::new()),
        }
    }

    /// What a reload changed: from the next round.
    pub(crate) fn set_live(&self, live: VerifyLive) {
        *lock(&self.live) = live;
    }

    /// What the next round begins with.
    pub(crate) fn live(&self) -> VerifyLive {
        lock(&self.live).clone()
    }

    /// Tells the user something once a session.
    fn tell_once(&self, message: String) {
        if lock(&self.told).insert(message.clone()) {
            self.parts.hooks.tell(message);
        }
    }

    /// The project's knowledge as it is now. `None` with no store, or when
    /// the read could not be made.
    async fn current(&self, cancel: &CancelToken) -> Option<(Arc<Snapshot>, cortex_base::UtcTime)> {
        let knowledge = Arc::clone(self.parts.knowledge.as_ref()?);
        run_blocking(cancel, move |_| knowledge.current())
            .await
            .ok()
    }

    /// The checks a round would run and the shell to run them in, or why
    /// it would run none. A line of the user's that cannot run is told,
    /// once a session.
    async fn ready(
        &self,
        live: &VerifyLive,
        cancel: &CancelToken,
    ) -> Result<(Chosen, Arc<RecordingShell>), Unready> {
        let current = self.current(cancel).await;
        let chosen = choose(
            &live.settings,
            current
                .as_ref()
                .map(|(snapshot, now)| (snapshot.as_ref(), *now)),
        );
        for refused in &chosen.refused {
            self.tell_once(refused.clone());
        }
        if chosen.checks.is_empty() {
            return Err(Unready::NoneKnown);
        }
        if !live.trusted {
            return Err(Unready::Untrusted);
        }
        if *self.parts.policy.engine().level() == Level::ReadOnly {
            return Err(Unready::ReadOnly);
        }
        let Some(shell) = &self.parts.shell else {
            return Err(Unready::NoShell);
        };
        Ok((chosen, Arc::clone(shell)))
    }

    /// Why `verify/run` would run no check now: `None` when it would run
    /// one. Each check is still decided as it runs.
    pub(crate) async fn why_none(&self) -> Option<Unready> {
        self.ready(&self.live(), &CancelToken::new()).await.err()
    }

    async fn round(&self, cx: VerifyCx<'_>) -> VerifyOutcome {
        let live = self.live();
        let settings = &live.settings;
        if live.lowered_by_project && settings.mode != VerifyMode::Auto {
            self.tell_once(format!(
                "The project's configuration sets `verify.mode` to `{}`: its checks do not run by \
                 themselves at the end of a turn.",
                settings.mode.as_str()
            ));
        }
        // The user asked (`verify/run`): the mode is not read.
        if !cx.asked && settings.mode == VerifyMode::Off {
            return VerifyOutcome::Skipped;
        }
        let ready = self.ready(&live, cx.cancel).await;
        if cx.cancel.is_cancelled() {
            return VerifyOutcome::Skipped;
        }
        let (chosen, shell) = match ready {
            Ok(ready) => ready,
            // Off until a check is known: nothing is said, unless the user
            // asked.
            Err(Unready::NoneKnown) if !cx.asked => return VerifyOutcome::Skipped,
            Err(why) => {
                // The user who asked is told every time; `verify/run` said
                // it before the turn started, so only what changed since
                // reaches here.
                if cx.asked {
                    self.parts.hooks.tell(why.message().to_owned());
                } else {
                    self.tell_once(why.message().to_owned());
                }
                return VerifyOutcome::Skipped;
            }
        };
        let shell = &shell;
        // The turn's time for checks, or its own time limit, is spent:
        // nothing is started, and what still fails stays failed.
        let spent = if cx.deadline.is_some_and(|deadline| deadline.is_zero()) {
            Some(
                "The project's checks did not run: the turn's or the session's time limit is \
                 reached."
                    .to_owned(),
            )
        } else if settings.turn_time.saturating_sub(cx.used).is_zero() {
            Some(format!(
                "The project's checks did not run again: this turn's time for them ({}) is spent.",
                shown_time(settings.turn_time)
            ))
        } else {
            None
        };
        if let Some(message) = spent {
            self.tell_once(message);
            return if lock(&self.memory).unresolved(cx.turn) {
                VerifyOutcome::StillFailing
            } else {
                VerifyOutcome::Skipped
            };
        }
        let reports: Vec<CheckReport> = chosen
            .checks
            .iter()
            .map(|check| check_report(check, None))
            .collect();
        if settings.mode == VerifyMode::Ask && !cx.asked {
            (cx.notify)(VerificationNotice {
                message: format!(
                    "The project's checks were not run (verify.mode is ask): {}.",
                    names(&chosen.checks)
                ),
                phase: VerificationPhase::Offered,
                round: 0,
                rounds: cx.rounds,
                checks: reports,
                verdict: None,
            });
            return VerifyOutcome::Skipped;
        }
        if chosen.left_out > 0 {
            self.tell_once(format!(
                "{} of the project's checks are not run: a round runs at most {MOST_CHECKS}.",
                chosen.left_out
            ));
        }
        if !self.parts.sandbox.confines() {
            self.tell_once(
                "The project's checks run outside any sandbox in this session, as its commands \
                 do: with your full rights, writes anywhere you can, and the network."
                    .to_owned(),
            );
        }
        (cx.notify)(VerificationNotice {
            message: format!(
                "Running the project's checks (round {} of {}): {}.",
                cx.round,
                cx.rounds,
                names(&chosen.checks)
            ),
            phase: VerificationPhase::Started,
            round: cx.round,
            rounds: cx.rounds,
            checks: reports,
            verdict: None,
        });

        let mut round = Round {
            cx: &cx,
            shell,
            settings: settings.clone(),
            started: self.parts.clock.monotonic(),
            planned: 0,
        };
        let mut results: Vec<(Check, Ran)> = Vec::new();
        for check in &chosen.checks {
            if cx.cancel.is_cancelled() {
                return VerifyOutcome::Skipped;
            }
            let ran = self.check(&mut round, check, &chosen.flaky).await;
            let end =
                matches!(ran.ended, Ended::Stopped(_) | Ended::Interrupted) || ran.stop.is_some();
            results.push((check.clone(), ran));
            if end {
                break;
            }
        }
        if cx.cancel.is_cancelled() {
            // The loop goes to `Interrupted`: nothing more is said.
            return VerifyOutcome::Skipped;
        }
        self.conclude(&cx, &chosen.checks, results)
    }

    /// One check, and its retry when it is a known flaky test.
    async fn check(&self, round: &mut Round<'_, '_>, check: &Check, flaky: &BTreeSet<Key>) -> Ran {
        let first = self.run_once(round, check).await;
        if first.stop.is_some()
            || !retries(&first, flaky.contains(&check.key()))
            || round.cx.cancel.is_cancelled()
        {
            return first;
        }
        let second = self.run_once(round, check).await;
        if second.ended == Ended::Interrupted {
            return second;
        }
        after_retry(first, second)
    }

    /// One run of a check: its time, its entry, its plan, the engine's
    /// answer, and the run itself.
    async fn run_once(&self, round: &mut Round<'_, '_>, check: &Check) -> Ran {
        let cx = round.cx;
        let settings = &round.settings.clone();
        // The time it may have: what the round, the turn and the limits
        // leave, and no more than the shell gives a command.
        let in_round = self.parts.clock.monotonic().saturating_sub(round.started);
        let limit = time_left(settings, cx.used, cx.deadline, in_round);
        if let Some(why) = too_long(
            check.usual,
            settings.round_time,
            settings.max_timeout,
            limit,
        ) {
            return Ran::not(Ended::Skipped, why);
        }
        // An entry of the profile must still be one that runs unasked: it
        // may have been turned off, or have lapsed, since the round began.
        if let Some(entry) = &check.entry {
            let current = self.current(cx.cancel).await;
            let still = current.as_ref().is_some_and(|(snapshot, now)| {
                snapshot.runs_unasked(*now).any(|held| &held.id == entry)
            });
            if !still {
                return Ran::not(
                    Ended::NotRun,
                    format!(
                        "The project profile's entry {} is no longer in use, or no longer \
                         accepted.",
                        visible(entry.as_str())
                    ),
                );
            }
        }
        // Planned and decided now, just before it runs: what the checks
        // before it did is what its plan sees.
        let mut arguments = json!({
            "command": check.line,
            "timeout_ms": u64::try_from(limit.as_millis()).unwrap_or(u64::MAX).max(1),
        });
        if let (Some(cwd), Some(fields)) = (&check.cwd, arguments.as_object_mut()) {
            fields.insert("cwd".to_owned(), Json::String(cwd.clone()));
        }
        round.planned += 1;
        let call = CallId::new(format!("check_{}_{}", cx.round, round.planned));
        let item = ItemId::new(format!("check_{}_{}_{}", cx.turn, cx.round, round.planned));
        let shell = Arc::clone(round.shell);
        let paths = Arc::clone(&self.parts.paths);
        let policy = Arc::clone(&self.parts.policy);
        let session = cx.session.clone();
        let turn = cx.turn.clone();
        let planned = run_blocking(cx.cancel, move |_| {
            let plan = shell.plan(&arguments, &PlanCx::new(paths))?;
            let planned_call = PlannedCall {
                session: &session,
                turn: &turn,
                call_id: &call,
                item: &item,
                spec: shell.spec(),
                arguments: &arguments,
                plan: &plan,
                preview: None,
            };
            let decision = policy.decide(&planned_call);
            // Whether an `ask` rule of the managed configuration is among
            // what asks: that holds back the user's own line too.
            let managed = decision == Decision::Ask
                && matches!(policy.outcome(&planned_call), cortex_policy::engine::Outcome::Ask(question) if question.managed);
            // The level is read after the decision, so that one lowered
            // since the round began, or while this check was planned, holds.
            let read_only = *policy.engine().level() == Level::ReadOnly;
            Ok::<_, cortex_tools::ToolError>((plan, decision, read_only, managed))
        })
        .await;
        let (mut plan, decision, read_only, managed) = match planned {
            Ok(Ok(planned)) => planned,
            Ok(Err(error)) => {
                return Ran::not(
                    Ended::NotRun,
                    visible(&error.into_result().text()).into_owned(),
                );
            }
            Err(cortex_tools::BlockingError::Cancelled) => {
                return Ran::not(Ended::Interrupted, "The turn was interrupted.");
            }
            Err(cortex_tools::BlockingError::Panicked(why)) => {
                return Ran::not(
                    Ended::NotRun,
                    format!("Planning it failed: {}", visible(&why)),
                );
            }
        };
        // What the engine says decides, but for one thing: the user's own
        // line runs where the engine would ask, as the user's word, unless
        // the managed configuration's `ask` rule is among what asks: an
        // administrator's word binds the user's check as a managed `deny`
        // does (the owner's decision of 2026-11-16). A check never puts a
        // question.
        let refused = match decision {
            Decision::Allow => None,
            Decision::AllowReadOnly => {
                plan.concurrency = Concurrency::ReadOnly;
                None
            }
            Decision::Ask if managed => Some(Ran::not(
                Ended::NotRun,
                "It would ask before it runs, by a rule of the managed configuration, which an \
                 administrator wrote for a person to decide each time, and a check never asks: \
                 run it by hand, or ask for an `allow` rule.",
            )),
            Decision::Ask if check.source == CheckSource::User => None,
            Decision::Ask => Some(Ran::not(
                Ended::NotRun,
                "It would ask before it runs, and a check never asks: its line is not one a \
                 rule can allow exactly, or the project is not trusted for its learned rule.",
            )),
            Decision::Deny { reason } => Some(Ran::not(
                Ended::NotRun,
                format!(
                    "A rule refuses it: {}",
                    visible(reason.as_deref().unwrap_or("no reason was given."))
                ),
            )),
            Decision::Stop { reason } => Some(Ran::not(Ended::Stopped(reason.clone()), reason)),
            _ => Some(Ran::not(
                Ended::NotRun,
                "The engine's answer is not one this build knows.",
            )),
        };
        if let Some(refused) = refused {
            return refused;
        }
        if read_only {
            return Ran::not(
                Ended::NotRun,
                "The session is at the read-only level now, where no check runs.",
            );
        }
        if let Some(reason) = self.parts.policy.stopped_turn(cx.number) {
            return Ran::not(Ended::Stopped(reason.clone()), reason);
        }
        let run_cx = RunCx::new(
            Arc::clone(&self.parts.paths),
            Arc::clone(&self.parts.content),
            Arc::clone(&self.parts.files),
        )
        .with_cancel(cx.cancel.clone())
        .with_kill(cx.kill.clone())
        .with_limits(self.parts.limits)
        .with_gate(self.parts.policy.read_gate())
        .with_turn(cx.number);
        let result =
            cortex_tools::execute(&AsCheck(round.shell.as_ref()), plan, &run_cx, &mut NoOutput)
                .await;
        let mut ran = classify(&result, limit, check.usual);
        // The look after it, or the look before it, may have stopped the
        // turn: nothing more of it runs, and the turn ends. Its own result
        // stands beside the stop.
        ran.stop = self.parts.policy.stopped_turn(cx.number);
        ran
    }

    /// The verdict, what the model and front ends are told, and the turn's
    /// memory of what still fails.
    fn conclude(
        &self,
        cx: &VerifyCx<'_>,
        chosen: &[Check],
        results: Vec<(Check, Ran)>,
    ) -> VerifyOutcome {
        let (verdict, failing) = lock(&self.memory).judge(cx.turn, chosen.len(), &results);
        let still = verdict == Verdict::StillFailing;
        let (outcome, said) = outcome_of(verdict, &failing, || {
            let earlier: Vec<&Check> = failing
                .iter()
                .filter(|held| {
                    !results
                        .iter()
                        .any(|(check, ran)| check.key() == held.key() && ran.ended == Ended::Failed)
                })
                .collect();
            report(cx.round, cx.rounds, &results, &earlier)
        });
        let message = match &said {
            Verification::Passed => "The project's checks passed.".to_owned(),
            Verification::Failed if still => format!(
                "The project's checks still fail: {}, which did not run to a result in this round.",
                names(&failing)
            ),
            Verification::Failed => format!("The project's checks failed: {}.", names(&failing)),
            Verification::Skipped => "None of the project's checks ran.".to_owned(),
            _ => "The project's checks did not all run to a result.".to_owned(),
        };
        (cx.notify)(VerificationNotice {
            message,
            phase: VerificationPhase::Finished,
            round: cx.round,
            rounds: cx.rounds,
            checks: results
                .iter()
                .map(|(check, ran)| check_report(check, Some(ran)))
                .collect(),
            verdict: Some(said),
        });
        outcome
    }
}

/// What the shell is told of a command's output: the project profile's
/// builds and tests, and the user's own checks too, keep the lines that
/// report a problem when the output must be cut. A check is a build or a
/// test run whatever the table classes it as, and what the model is told of
/// a failed one is cut from what the shell kept.
#[derive(Debug)]
pub(crate) struct ChecksProfile {
    pub(crate) inner: Option<Arc<dyn CommandProfile>>,
    /// `verify.commands`.
    pub(crate) lines: Vec<String>,
}

impl CommandProfile for ChecksProfile {
    fn is_build_or_test(&self, command: &str) -> bool {
        let key = key_line(command);
        self.lines.iter().any(|line| key_line(line) == key)
            || self
                .inner
                .as_ref()
                .is_some_and(|inner| inner.is_build_or_test(command))
    }
}

/// The checks' lines, for a sentence.
fn names(checks: &[Check]) -> String {
    checks.iter().map(named).collect::<Vec<_>>().join(", ")
}

impl Verifier for CoreVerifier {
    fn verify<'a>(&'a self, cx: VerifyCx<'a>) -> BoxFuture<'a, VerifyOutcome> {
        Box::pin(self.round(cx))
    }
}

#[cfg(test)]
mod tests;
