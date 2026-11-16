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
//!   an entry of the profile and not the user's own line. A profile entry is
//!   looked up again first, so one turned off elsewhere does not run.
//! - **Each runs through the session's own shell** (`RecordingShell`, as
//!   the harness's run): the same runner, sandbox, environment, looks at
//!   protected paths and checkpoint as the model's commands.
//! - **Time:** each runs with what is left of the round's limit, the
//!   turn's, and the turn's and session's own time limits. One that
//!   usually takes longer than the round's limit, or longer than what is
//!   left, is skipped. One stopped at its limit is a failure for the model
//!   only when it usually takes less than half of what it had.
//! - **A known flaky test** that fails runs once more.
//! - **What the model is told** of a failure is bounded, and every line of
//!   a check's output is escaped and marked as the command's.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cortex_agent::{Decision, PlannedCall, Policy, Verifier, VerifyCx, VerifyOutcome};
use cortex_base::{PathResolver, SharedClock, visible, visible_keeping};
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

/// The least a failed check's share of [`REPORT_BYTES`] is.
const LEAST_SHARE: usize = 2 * 1024;

/// The least time a check is started with when how long it usually takes is
/// not known.
const LEAST_TIME: Duration = Duration::from_secs(10);

/// The longest line of `verify.commands` that runs.
const LONGEST_LINE: usize = 400;

/// What begins every line of a check's output in the report to the model:
/// no line of it can begin with anything of the report's own.
pub(crate) const MARK: &str = "| ";

/// `[verify]`, as the verifier reads it.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub(crate) mode: VerifyMode,
    pub(crate) commands: Vec<String>,
    pub(crate) round_time: Duration,
    pub(crate) turn_time: Duration,
}

impl From<&VerifySettings> for Settings {
    fn from(settings: &VerifySettings) -> Self {
        Self {
            mode: settings.mode,
            commands: settings.commands.clone(),
            round_time: settings.round_time,
            turn_time: settings.turn_time,
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
            (entry.kind == Kind::Command
                && key_line(&command.run) == key.0
                && command.cwd == key.1)
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
}

impl Ran {
    fn not(ended: Ended, note: impl Into<String>) -> Self {
        Self {
            ended,
            exit_code: None,
            signal: None,
            duration: None,
            note: Some(note.into()),
            output: None,
            artifact: None,
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
    let blocked = field("sandbox").and_then(Json::as_str);
    ran.ended = if flag("timed_out") {
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
    } else if let (Some(line), false) = (blocked, ran.exit_code == Some(0)) {
        notes.push(visible(line).into_owned());
        Ended::Blocked
    } else if ran.exit_code == Some(0) && *result.outcome() == ToolOutcome::Success {
        Ended::Passed
    } else {
        Ended::Failed
    };
    if flag("background_stopped") {
        notes.push("It left processes running, and they were stopped.".to_owned());
    }
    if !notes.is_empty() {
        ran.note = Some(notes.join(" "));
    }
    if matches!(ran.ended, Ended::Failed) {
        ran.output = Some(result.text());
    }
    ran
}

/// A length of time as the reports say it.
fn shown_time(time: Duration) -> String {
    let seconds = time.as_secs();
    if seconds >= 60 && seconds % 60 == 0 {
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
    let share = (REPORT_BYTES / failed.len().max(1)).max(LEAST_SHARE);
    for (check, ran) in &failed {
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
        text.push_str(" What it printed, each line marked `| `:\n");
        let printed = ran.output.as_deref().unwrap_or("");
        let shaped = output::shape(
            printed,
            &OutputLimits::new(share, 400, 2_000),
            Keep::Important,
            Compression::Structural,
        );
        let mut first = true;
        for line in shaped.text.split('\n') {
            if !first {
                text.push('\n');
            }
            first = false;
            text.push_str(MARK);
            text.push_str(&visible_keeping(line, &['\t']));
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
            Ended::Flaky => "failed, then passed when run again: it is a known flaky test".to_owned(),
            Ended::TimedOut => "was stopped at its time limit, which is not taken for a failure of \
                                your changes"
                .to_owned(),
            Ended::Blocked => "was blocked by the sandbox, which is not a failure of your changes"
                .to_owned(),
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
        note: ran.and_then(|ran| match &ran.ended {
            Ended::Stopped(reason) => Some(visible(reason).into_owned()),
            _ => ran.note.clone(),
        }),
        artifact: ran.and_then(|ran| ran.artifact.clone()),
    }
}

/// The checks that failed in a turn and have not passed since.
#[derive(Debug, Default)]
struct Memory {
    turn: Option<TurnId>,
    failing: Vec<Check>,
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

/// A session's verifier: the project's checks, chosen and run.
pub(crate) struct CoreVerifier {
    parts: Parts,
    memory: Mutex<Memory>,
    told: Mutex<BTreeSet<String>>,
}

impl std::fmt::Debug for CoreVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreVerifier")
            .field("settings", &self.parts.settings)
            .field("trusted", &self.parts.trusted)
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
    /// When the round started, on the monotonic clock.
    started: Duration,
    /// How many checks it has planned, for their ids.
    planned: u32,
}

impl CoreVerifier {
    pub(crate) fn new(parts: Parts) -> Self {
        Self {
            parts,
            memory: Mutex::new(Memory::default()),
            told: Mutex::new(BTreeSet::new()),
        }
    }

    /// Tells the user something once a session.
    fn tell_once(&self, message: String) {
        if lock(&self.told).insert(message.clone()) {
            self.parts.hooks.tell(message);
        }
    }

    /// The project's knowledge as it is now. `None` with no store, or when
    /// the read could not be made.
    async fn current(
        &self,
        cx: &VerifyCx<'_>,
    ) -> Option<(Arc<Snapshot>, cortex_base::UtcTime)> {
        let knowledge = Arc::clone(self.parts.knowledge.as_ref()?);
        run_blocking(cx.cancel, move |_| knowledge.current()).await.ok()
    }

    async fn round(&self, cx: VerifyCx<'_>) -> VerifyOutcome {
        let settings = &self.parts.settings;
        if settings.mode == VerifyMode::Off {
            if self.parts.lowered_by_project {
                self.tell_once(
                    "The project's configuration turns verification off: its checks do not \
                     run at the end of a turn."
                        .to_owned(),
                );
            }
            return VerifyOutcome::Skipped;
        }
        let current = self.current(&cx).await;
        if cx.cancel.is_cancelled() {
            return VerifyOutcome::Skipped;
        }
        let chosen = choose(
            settings,
            current
                .as_ref()
                .map(|(snapshot, now)| (snapshot.as_ref(), *now)),
        );
        for refused in &chosen.refused {
            self.tell_once(refused.clone());
        }
        // Off until a check is known: nothing is said.
        if chosen.checks.is_empty() {
            return VerifyOutcome::Skipped;
        }
        if !self.parts.trusted {
            self.tell_once(
                "The project's checks do not run: they would run the repository's own code at \
                 the end of every turn, and you have not trusted this project. `cortex \
                 permissions trust` trusts it."
                    .to_owned(),
            );
            return VerifyOutcome::Skipped;
        }
        if *self.parts.policy.engine().level() == Level::ReadOnly {
            self.tell_once(
                "The project's checks do not run at the read-only level.".to_owned(),
            );
            return VerifyOutcome::Skipped;
        }
        let Some(shell) = &self.parts.shell else {
            self.tell_once("The project's checks do not run: no shell was found.".to_owned());
            return VerifyOutcome::Skipped;
        };
        let reports: Vec<CheckReport> = chosen
            .checks
            .iter()
            .map(|check| check_report(check, None))
            .collect();
        if settings.mode == VerifyMode::Ask {
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
            started: self.parts.clock.monotonic(),
            planned: 0,
        };
        let mut results: Vec<(Check, Ran)> = Vec::new();
        for check in &chosen.checks {
            if cx.cancel.is_cancelled() {
                return VerifyOutcome::Skipped;
            }
            let ran = self.check(&mut round, check, &chosen.flaky).await;
            let end = matches!(ran.ended, Ended::Stopped(_) | Ended::Interrupted);
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
        if first.ended != Ended::Failed
            || !flaky.contains(&check.key())
            || round.cx.cancel.is_cancelled()
        {
            return first;
        }
        let second = self.run_once(round, check).await;
        match second.ended {
            Ended::Passed => Ran {
                ended: Ended::Flaky,
                note: Some(format!(
                    "It failed with {}, then passed when run again: it is a known flaky test.",
                    first
                        .exit_code
                        .map_or_else(|| "no exit code".to_owned(), |code| format!("exit code {code}"))
                )),
                ..second
            },
            // A failure after the retry is a failure, never "flaky".
            _ => second,
        }
    }

    /// One run of a check: its time, its entry, its plan, the engine's
    /// answer, and the run itself.
    async fn run_once(&self, round: &mut Round<'_, '_>, check: &Check) -> Ran {
        let cx = round.cx;
        let settings = &self.parts.settings;
        // The time it may have.
        let in_round = self
            .parts
            .clock
            .monotonic()
            .saturating_sub(round.started);
        let mut left = settings
            .round_time
            .saturating_sub(in_round)
            .min(settings.turn_time.saturating_sub(cx.used + in_round));
        if let Some(deadline) = cx.deadline {
            left = left.min(deadline.saturating_sub(in_round));
        }
        if let Some(usual) = check.usual
            && usual > settings.round_time
        {
            return Ran::not(
                Ended::Skipped,
                format!(
                    "It usually takes {}, longer than a round's limit of {}.",
                    shown_time(usual),
                    shown_time(settings.round_time)
                ),
            );
        }
        let needs = check.usual.unwrap_or(LEAST_TIME);
        if left < needs || left.is_zero() {
            return Ran::not(
                Ended::Skipped,
                format!(
                    "Not enough of the round's time was left: {}.",
                    shown_time(left)
                ),
            );
        }
        // An entry of the profile must still be one that runs unasked: it
        // may have been turned off, or have lapsed, since the round began.
        if let Some(entry) = &check.entry {
            let current = self.current(cx).await;
            let still = current.as_ref().is_some_and(|(snapshot, now)| {
                snapshot
                    .runs_unasked(*now)
                    .any(|held| &held.id == entry)
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
            "timeout_ms": u64::try_from(left.as_millis()).unwrap_or(u64::MAX).max(1),
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
            let decision = policy.decide(&PlannedCall {
                session: &session,
                turn: &turn,
                call_id: &call,
                item: &item,
                spec: shell.spec(),
                arguments: &arguments,
                plan: &plan,
                preview: None,
            });
            Ok::<_, cortex_tools::ToolError>((plan, decision))
        })
        .await;
        let (mut plan, decision) = match planned {
            Ok(Ok(planned)) => planned,
            Ok(Err(error)) => {
                return Ran::not(
                    Ended::NotRun,
                    visible(&error.into_result().text()).into_owned(),
                );
            }
            Err(_) => return Ran::not(Ended::Interrupted, "The turn was interrupted."),
        };
        match decision {
            Decision::Allow => {}
            Decision::AllowReadOnly => plan.concurrency = Concurrency::ReadOnly,
            // The user's own line is the user's word.
            Decision::Ask if check.source == CheckSource::User => {}
            Decision::Ask => {
                return Ran::not(
                    Ended::NotRun,
                    "It would ask before it runs, and a check never asks: accept the entry \
                     again, or trust the project, for its learned rule to apply.",
                );
            }
            Decision::Deny { reason } => {
                return Ran::not(
                    Ended::NotRun,
                    format!(
                        "A rule refuses it: {}",
                        visible(reason.as_deref().unwrap_or("no reason was given."))
                    ),
                );
            }
            Decision::Stop { reason } => return Ran::not(Ended::Stopped(reason.clone()), reason),
            _ => return Ran::not(Ended::NotRun, "The engine's answer is not one this build knows."),
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
        let result = cortex_tools::execute(
            &AsCheck(round.shell.as_ref()),
            plan,
            &run_cx,
            &mut NoOutput,
        )
        .await;
        let mut ran = classify(&result, left, check.usual);
        // The look after it, or the look before it, may have stopped the
        // turn: nothing more of it runs, and the turn ends.
        if let Some(reason) = self.parts.policy.stopped_turn(cx.number) {
            ran.ended = Ended::Stopped(reason);
        }
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
        let failing: Vec<Check> = {
            let mut memory = lock(&self.memory);
            if memory.turn.as_ref() != Some(cx.turn) {
                *memory = Memory {
                    turn: Some(cx.turn.clone()),
                    failing: Vec::new(),
                };
            }
            for (check, ran) in &results {
                match ran.ended {
                    Ended::Passed | Ended::Flaky => {
                        memory.failing.retain(|held| held.key() != check.key());
                    }
                    Ended::Failed => {
                        if !memory.failing.iter().any(|held| held.key() == check.key()) {
                            memory.failing.push(check.clone());
                        }
                    }
                    _ => {}
                }
            }
            memory.failing.clone()
        };
        let stopped = results.iter().find_map(|(_, ran)| match &ran.ended {
            Ended::Stopped(reason) => Some(reason.clone()),
            _ => None,
        });
        let started = results.iter().filter(|(_, ran)| ran.started()).count();
        let all_passed = results.len() == chosen.len()
            && results
                .iter()
                .all(|(_, ran)| matches!(ran.ended, Ended::Passed | Ended::Flaky));
        let (outcome, verdict) = if let Some(reason) = stopped {
            (
                VerifyOutcome::Stopped { reason },
                Verification::Incomplete,
            )
        } else if !failing.is_empty() {
            let earlier: Vec<&Check> = failing
                .iter()
                .filter(|held| {
                    !results.iter().any(|(check, ran)| {
                        check.key() == held.key() && ran.ended == Ended::Failed
                    })
                })
                .collect();
            (
                VerifyOutcome::Failed {
                    report: report(cx.round, cx.rounds, &results, &earlier),
                },
                Verification::Failed,
            )
        } else if all_passed {
            (VerifyOutcome::Passed, Verification::Passed)
        } else if started == 0 {
            (VerifyOutcome::Skipped, Verification::Skipped)
        } else {
            (VerifyOutcome::Incomplete, Verification::Incomplete)
        };
        let message = match &verdict {
            Verification::Passed => "The project's checks passed.".to_owned(),
            Verification::Failed => format!(
                "The project's checks failed: {}.",
                names(&failing)
            ),
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
            verdict: Some(verdict),
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
