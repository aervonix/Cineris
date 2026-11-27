//! Integration tests (tier 2): the loop, driven as the core will drive it,
//! with the scripted provider and fake tools.
//!
//! Nothing here waits for time. A session runs on a single-threaded
//! runtime, and a test waits for what it expects to see: an event, a line
//! in the tools' log, a gate. Where two things must happen in an order, one
//! side opens a [`Gate`] and the other waits for it.
//!
//! **The order of events is seeded.** The loop plans each call, and bounds
//! each result, on the blocking pool, and those threads would finish when
//! the machine let them: a plan before or after the response's next event,
//! one call's plan before or after another's. A [`World`] puts that work on
//! the test's own thread instead, at moments a seeded generator decides
//! (`cineris_tools::InlineBlocking`), so a test sees one order for one seed.
//! An ordinary run uses seed 0. `cargo xtask stress -p cineris-agent` runs
//! every test under many seeds, and a test that fails prints its seed:
//! `CINERIS_TEST_SEED=<seed>` runs it again in the same order. A test must
//! pass under every seed, so it asserts only on what the loop promises, and
//! fixes an order it needs with a gate.
//!
//! The pieces every test uses are in this file: [`Rig`], which puts a
//! session together and remembers everything it does; the fake tools
//! ([`Probe`]); and [`assert_valid_history`], which checks a request's
//! messages the way a provider would.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    missing_docs,
    missing_debug_implementations,
    unreachable_pub,
    reason = "an integration test prepares real directories and fails by panicking, and its \
              helpers are shared between the modules of this one binary"
)]

mod context;
mod denials;
mod early;
mod extensions;
mod hooks;
mod interrupt;
mod limits;
mod map;
mod measure;
mod order;
mod plan;
mod pressure;
mod profile;
mod reconfigure;
mod redecide;
mod retries;
mod roles;
mod rounds;
mod run_stops;
pub mod sessions;
mod settings;
mod speed;
mod steering;
mod streaming;
mod summaries;
mod timing;
mod transitions;
mod verifying;

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cineris_agent::{
    ContextParts, ContextSettings, Decision, EventSink, Ledger, LoopConfig, MemoryInstructionFiles,
    MemorySessionStore, Models, Observer, PlannedCall, Policy, RoleModel, Session, SessionHandle,
    SessionInfo, SessionParts, SessionStore, SpeedSettings, StatEvent, Stats, TurnState, Verdict,
    Verifier, VerifyCx, VerifyOutcome, Workspace,
};
use cineris_base::{CancelToken, FakeClock, PathOptions, PathResolver, SharedClock};
use cineris_llm::{ModelRequest, SpeedCooldowns};
use cineris_protocol::event::{Delta, Event, ItemKind, Notice, SessionEvent, TurnCompleted};
use cineris_protocol::item::Delivery;
use cineris_protocol::methods::TurnStartResult;
use cineris_protocol::{Item, Message, Part, Role, SessionId, StopReason};
use cineris_testkit::{Gate, Response, ScriptedProvider, fixtures};
use cineris_tools::{
    BoxFuture, Concurrency, ConcurrencyClass, Effect, Json, Plan, PlanCx, RunCx, Tool, ToolError,
    ToolOutput, ToolResult, ToolSpec, Toolbox,
};
use serde_json::json;
use tokio::sync::Notify;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ------------------------------------------------------------------ events

/// Everything a session told its front ends, in order.
#[derive(Default)]
pub struct Events {
    list: Mutex<Vec<SessionEvent>>,
    /// The prompts' tickets the loop said were settled, in order.
    settled: Mutex<Vec<u64>>,
    wake: Notify,
    /// Set by the one test that makes the loop panic on purpose.
    pub loop_panic_expected: std::sync::atomic::AtomicBool,
}

impl Events {
    /// The prompts' tickets the loop said were settled, in order.
    pub fn settled(&self) -> Vec<u64> {
        lock(&self.settled).clone()
    }
}

impl EventSink for Events {
    fn prompt_settled(&self, ticket: u64) {
        lock(&self.settled).push(ticket);
        self.wake.notify_one();
    }

    fn emit(&self, event: SessionEvent) {
        // The loop contains its own panics and ends the turn. In a test
        // that is a bug showing itself (a failed assertion in the loop),
        // and it must fail the test whatever else the test looks at.
        if let Event::TurnCompleted(completed) = &event.event
            && let Some(error) = &completed.error
            && error.code == "loop_panicked"
        {
            assert!(
                self.loop_panic_expected
                    .load(std::sync::atomic::Ordering::SeqCst),
                "the loop panicked: {}",
                error.message
            );
        }
        lock(&self.list).push(event);
        self.wake.notify_one();
    }
}

/// A short name for an event, for asserting on order.
pub fn label(event: &Event) -> String {
    match event {
        Event::TurnStarted(_) => "turn.started".to_owned(),
        Event::ItemStarted(started) => match &started.kind {
            ItemKind::AssistantMessage => "started:assistant".to_owned(),
            _ => format!("started:{}", started.tool.as_deref().unwrap_or("?")),
        },
        Event::ItemDelta(delta) => match &delta.delta {
            Delta::Text(_) => "delta:text".to_owned(),
            Delta::Reasoning(_) => "delta:reasoning".to_owned(),
            Delta::ToolCall(_) => "delta:tool_call".to_owned(),
            Delta::ToolArgs(_) => "delta:tool_args".to_owned(),
            Delta::ToolOutput(_) => "delta:tool_output".to_owned(),
            Delta::Unknown(_) => "delta:?".to_owned(),
        },
        Event::ItemReset(_) => "reset".to_owned(),
        Event::ItemCompleted(completed) => match &completed.item {
            Item::UserMessage(_) => "user".to_owned(),
            Item::AssistantMessage(_) => "assistant".to_owned(),
            Item::ToolResult(result) => {
                format!("result:{}:{}", result.tool, result.outcome.as_str())
            }
            Item::Notice(notice) => format!("note:{}", notice.source.as_str()),
            _ => "item:?".to_owned(),
        },
        Event::UsageUpdated(_) => "usage".to_owned(),
        Event::Notice(notice) => match notice {
            Notice::Retry(_) => "notice:retry".to_owned(),
            Notice::Compaction(compaction) => {
                format!("notice:compaction:{}", compaction.state.as_str())
            }
            Notice::Limit(_) => "notice:limit".to_owned(),
            Notice::SpeedFallback(_) => "notice:speed_fallback".to_owned(),
            Notice::SpeedCooldown(cooldown) => {
                format!("notice:speed_cooldown:{}", cooldown.state.as_str())
            }
            Notice::Info(_) => "notice:info".to_owned(),
            _ => "notice:?".to_owned(),
        },
        Event::TurnCompleted(completed) => format!("turn.completed:{}", completed.outcome.as_str()),
        Event::SessionIdle(_) => "idle".to_owned(),
        Event::SessionUpdated(_) => "updated".to_owned(),
        _ => "?".to_owned(),
    }
}

impl Events {
    /// Waits until `seen` holds of the events so far.
    pub async fn until(&self, seen: impl Fn(&[SessionEvent]) -> bool) {
        loop {
            if seen(&lock(&self.list)) {
                return;
            }
            self.wake.notified().await;
        }
    }

    /// Waits until `count` turns have ended.
    pub async fn turns(&self, count: usize) {
        self.until(|events| {
            events
                .iter()
                .filter(|event| matches!(event.event, Event::TurnCompleted(_)))
                .count()
                >= count
        })
        .await;
    }

    /// Waits until an event with this label has been seen `count` times.
    pub async fn seen(&self, wanted: &str, count: usize) {
        self.until(|events| {
            events
                .iter()
                .filter(|event| label(&event.event) == wanted)
                .count()
                >= count
        })
        .await;
    }

    pub fn all(&self) -> Vec<SessionEvent> {
        lock(&self.list).clone()
    }

    /// The labels of the events, without the two that say how things stand
    /// and not what happened (`usage.updated`, `session.updated`).
    pub fn labels(&self) -> Vec<String> {
        self.all()
            .iter()
            .map(|event| label(&event.event))
            .filter(|label| label != "usage" && label != "updated")
            .collect()
    }

    /// The labels of every event.
    pub fn every_label(&self) -> Vec<String> {
        self.all().iter().map(|event| label(&event.event)).collect()
    }

    /// The items that were completed, in order.
    pub fn items(&self) -> Vec<Item> {
        self.all()
            .into_iter()
            .filter_map(|event| match event.event {
                Event::ItemCompleted(completed) => Some(completed.item),
                _ => None,
            })
            .collect()
    }

    /// How each turn ended, in order.
    pub fn ends(&self) -> Vec<TurnCompleted> {
        self.all()
            .into_iter()
            .filter_map(|event| match event.event {
                Event::TurnCompleted(completed) => Some(completed),
                _ => None,
            })
            .collect()
    }

    /// The notices front ends were shown, in order.
    pub fn notices(&self) -> Vec<Notice> {
        self.all()
            .into_iter()
            .filter_map(|event| match event.event {
                Event::Notice(notice) => Some(notice),
                _ => None,
            })
            .collect()
    }

    /// The position of the first event with this label.
    pub fn position(&self, wanted: &str) -> usize {
        self.every_label()
            .iter()
            .position(|label| label == wanted)
            .unwrap_or_else(|| panic!("no `{wanted}` in {:?}", self.every_label()))
    }
}

// ------------------------------------------------------------- transitions

/// Every change of state a session made. It can also interrupt the turn
/// the moment a chosen state is entered.
#[derive(Default)]
pub struct Seen {
    edges: Mutex<Vec<(TurnState, TurnState)>>,
    /// When each state was entered, for a measurement.
    stamps: Mutex<Vec<(TurnState, std::time::Instant)>>,
    /// The state to interrupt at, how many entries of it to let pass
    /// first, and the session to interrupt.
    interrupt_at: Mutex<Option<(TurnState, usize, SessionHandle)>>,
}

impl Observer for Seen {
    fn transition(&self, _session: &SessionId, from: TurnState, to: TurnState) {
        lock(&self.edges).push((from, to));
        lock(&self.stamps).push((to, std::time::Instant::now()));
        let fire = {
            let mut armed = lock(&self.interrupt_at);
            match armed.as_mut() {
                Some((state, skip, _)) if *state == to && *skip > 0 => {
                    *skip -= 1;
                    None
                }
                Some((state, _, _)) if *state == to => armed.take(),
                _ => None,
            }
        };
        if let Some((_, _, handle)) = fire {
            let _ = handle.interrupt();
        }
    }
}

impl Seen {
    /// When each state was entered, in order.
    pub fn stamps(&self) -> Vec<(TurnState, std::time::Instant)> {
        lock(&self.stamps).clone()
    }

    /// The changes of state, in order.
    pub fn edges(&self) -> Vec<(TurnState, TurnState)> {
        lock(&self.edges).clone()
    }

    /// The states entered, in order, starting from idle.
    pub fn states(&self) -> Vec<TurnState> {
        let mut states = vec![TurnState::Idle];
        states.extend(self.edges().iter().map(|(_, to)| *to));
        states
    }

    /// Interrupts the running turn the next time `state` is entered.
    pub fn interrupt_at(&self, state: TurnState, handle: &SessionHandle) {
        self.interrupt_at_entry(state, 0, handle);
    }

    /// Interrupts the running turn when `state` is entered, after letting
    /// `skip` entries of it pass.
    pub fn interrupt_at_entry(&self, state: TurnState, skip: usize, handle: &SessionHandle) {
        *lock(&self.interrupt_at) = Some((state, skip, handle.clone()));
    }
}

// ------------------------------------------------------------------- tools

/// What the fake tools did, in order: `start read a`, `end read a`.
#[derive(Default)]
pub struct Log {
    lines: Mutex<Vec<String>>,
    /// The calls planned, by their plan's summary, in order: a call planned
    /// again is here twice.
    planned: Mutex<Vec<String>>,
    /// The call each run was told it runs for (`RunCx::call`), in the
    /// order the runs started.
    ran_for: Mutex<Vec<Option<cineris_tools::CallRef>>>,
    wake: Notify,
}

impl Log {
    /// The call each run was told it runs for, in the order the runs
    /// started.
    pub fn ran_for(&self) -> Vec<Option<cineris_tools::CallRef>> {
        lock(&self.ran_for).clone()
    }

    fn push(&self, line: String) {
        lock(&self.lines).push(line);
        self.wake.notify_one();
    }

    pub fn lines(&self) -> Vec<String> {
        lock(&self.lines).clone()
    }

    /// Waits until the log has this line.
    pub async fn has(&self, wanted: &str) {
        loop {
            if lock(&self.lines).iter().any(|line| line == wanted) {
                return;
            }
            self.wake.notified().await;
        }
    }

    /// Waits until the log has this line `times` times.
    pub async fn has_times(&self, wanted: &str, times: usize) {
        loop {
            if self.count(wanted) >= times {
                return;
            }
            self.wake.notified().await;
        }
    }

    pub fn count(&self, wanted: &str) -> usize {
        lock(&self.lines)
            .iter()
            .filter(|line| *line == wanted)
            .count()
    }

    /// How many times the call whose plan's summary is `summary` was
    /// planned.
    pub fn plans(&self, summary: &str) -> usize {
        lock(&self.planned)
            .iter()
            .filter(|line| *line == summary)
            .count()
    }
}

/// Gates by name, made when first asked for. A fake tool waits at the gate
/// its `gate` argument names.
#[derive(Clone, Default)]
pub struct Gates {
    gates: Arc<Mutex<BTreeMap<String, Gate>>>,
}

impl Gates {
    pub fn get(&self, name: &str) -> Gate {
        lock(&self.gates)
            .entry(name.to_owned())
            .or_default()
            .clone()
    }

    pub fn open(&self, name: &str) {
        self.get(name).open();
    }
}

/// What kind of tool a [`Probe`] stands in for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Reads `path`: read-only.
    Read,
    /// Writes `path`: keyed by it.
    Write,
    /// Runs a command: exclusive.
    Shell,
    /// Touches nothing.
    Plain,
    /// Declares no effect and is scheduled to run alone: what a tool from
    /// outside looks like when its author said nothing.
    Solo,
    /// Panics when it runs.
    Panic,
}

/// A fake tool. Its arguments say what one call does:
///
/// - `path`: the path it reads or writes;
/// - `id`: the call's label in the log, in place of the path;
/// - `gate`: the name of a gate to wait at before it finishes;
/// - `output`: text to send to front ends while it runs;
/// - `stubborn`: wait at the gate even after the call is cancelled;
/// - `graceful`: when the call is cancelled, go on until its second token
///   ("kill now") fires;
/// - `record`: note in the table of seen files, as `read` does, that the
///   path was shown;
/// - `fail`: end with an error result;
/// - `stop`: a reason: mark the result as one whose run stopped the turn
///   (`ToolResult::stopping_turn`), as a confined command that changed
///   protected paths, or an extension's stopped host call, marks its own;
/// - `session`: the call changes the session, as `plan` and `remember` do;
/// - `link`: `[target, name]`: when it runs, make `name` in the workspace a
///   link to the directory `target`, as `ln -s target name` would;
/// - `bad`: refuse the arguments when the call is planned;
/// - `held`: refuse the call when it is planned, as a held tool's is
///   (`ToolError::NeedsApproval`);
/// - `payload`: the result's payload, as a tool's: `{"kind": "unchanged"}`
///   for a write that found its content there, `{"changes": …}` for a
///   command whose look found what it changed.
pub struct Probe {
    spec: ToolSpec,
    kind: Kind,
    log: Arc<Log>,
    gates: Gates,
}

struct ProbeArgs {
    label: String,
    path: Option<cineris_base::AbsPath>,
    gate: Option<String>,
    output: Option<String>,
    stubborn: bool,
    graceful: bool,
    record: bool,
    fail: bool,
    stop: Option<String>,
    /// The link to make, and the directory it leads to.
    link: Option<(PathBuf, String)>,
    payload: Option<Json>,
}

impl Probe {
    pub fn new(name: &str, kind: Kind, log: &Arc<Log>, gates: &Gates) -> Self {
        let class = match kind {
            Kind::Read | Kind::Plain | Kind::Panic => ConcurrencyClass::ReadOnly,
            Kind::Write => ConcurrencyClass::Keyed,
            Kind::Shell | Kind::Solo => ConcurrencyClass::Exclusive,
        };
        Self {
            spec: ToolSpec::new(
                name,
                format!("The fake `{name}` tool."),
                json!({"type": "object"}),
            )
            .concurrency(class),
            kind,
            log: Arc::clone(log),
            gates: gates.clone(),
        }
    }
}

impl Tool for Probe {
    fn preview(&self, _plan: &Plan, _cx: &RunCx) -> Option<cineris_tools::Preview> {
        // It changes no file.
        None
    }

    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn plan(&self, args: &Json, cx: &PlanCx) -> Result<Plan, ToolError> {
        let text = |name: &str| args.get(name).and_then(Json::as_str).map(str::to_owned);
        if args.get("bad").is_some() {
            return Err(ToolError::invalid("`bad` is not an argument of this tool."));
        }
        if args.get("held").is_some() {
            return Err(ToolError::NeedsApproval(
                "The tool needs the person's approval before it can be called.".to_owned(),
            ));
        }
        let path = text("path")
            .map(|path| cx.paths.resolve(&path))
            .transpose()?;
        let label = text("id").or_else(|| text("path")).unwrap_or_default();
        let link = match args.get("link").and_then(Json::as_array).map(Vec::as_slice) {
            Some([target, name]) => {
                let (Some(target), Some(name)) = (target.as_str(), name.as_str()) else {
                    return Err(ToolError::invalid("`link` is two names."));
                };
                Some((cx.paths.root().to_fs_path().join(name), target.to_owned()))
            }
            _ => None,
        };
        let mut plan = Plan::new(
            format!("{} {label}", self.spec.name),
            ProbeArgs {
                label,
                path: path.as_ref().map(|path| path.abs().clone()),
                gate: text("gate"),
                output: text("output"),
                stubborn: args.get("stubborn").is_some(),
                graceful: args.get("graceful").is_some(),
                record: args.get("record").is_some(),
                fail: args.get("fail").is_some(),
                stop: text("stop"),
                link,
                payload: args.get("payload").cloned(),
            },
        );
        match (self.kind, path) {
            (Kind::Read, Some(path)) => plan = plan.with_effect(Effect::Read(path)),
            (Kind::Write, Some(path)) => {
                plan = plan
                    .with_concurrency(Concurrency::Keyed(vec![path.abs().clone()]))
                    .with_effect(Effect::Write(path));
            }
            (Kind::Solo, _) => plan = plan.with_concurrency(Concurrency::Exclusive),
            (Kind::Shell, _) => {
                plan = plan
                    .with_concurrency(Concurrency::Exclusive)
                    .with_effect(Effect::Exec {
                        command: "probe".to_owned(),
                        cwd: cx.paths.resolve(".")?,
                    });
            }
            _ => {}
        }
        if args.get("session").is_some() {
            plan = plan.with_session_change();
        }
        lock(&self.log.planned).push(plan.summary.clone());
        Ok(plan)
    }

    fn run<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        out: &'a mut dyn ToolOutput,
    ) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            let name = &self.spec.name;
            let args: ProbeArgs = match plan.into_args() {
                Ok(args) => args,
                Err(error) => return error.into_result(),
            };
            let label = &args.label;
            lock(&self.log.ran_for).push(cx.call.clone());
            self.log.push(format!("start {name} {label}"));
            if let Some(output) = &args.output {
                out.output(output);
            }
            if let Some(gate) = &args.gate {
                let opened = self.gates.get(gate).opened();
                if args.stubborn {
                    opened.await;
                } else if args.graceful {
                    // Asked to stop, it goes on until it is told to stop
                    // now: a command sitting out its grace.
                    if cx.cancel.race(opened).await.is_none() {
                        self.log.push(format!("asked {name} {label}"));
                        cx.kill.cancelled().await;
                        self.log.push(format!("killed {name} {label}"));
                        return ToolResult::interrupted(format!("{name} {label} was killed"));
                    }
                } else if cx.cancel.race(opened).await.is_none() {
                    self.log.push(format!("cancelled {name} {label}"));
                    return ToolResult::interrupted(format!("{name} {label} stopped part-way"));
                }
            }
            assert!(self.kind != Kind::Panic, "the probe was told to panic");
            if let (true, Some(path)) = (args.record, &args.path) {
                let snapshot = cineris_tools::FileSnapshot::of_content(b"one line\n", None);
                let shown = cineris_tools::Shown {
                    hash: snapshot.hash,
                    lines: cineris_tools::LineRanges::of(1, 1),
                    total_lines: 1,
                    turn: cx.turn,
                    kept: false,
                };
                cx.files.record_read(path, snapshot, shown);
            }
            if let Some((link, target)) = &args.link {
                make_link(target, link);
            }
            self.log.push(format!("end {name} {label}"));
            let mut result = if args.fail {
                ToolResult::error(format!("{name} {label} failed"))
            } else {
                ToolResult::success(format!("{name} {label} ok"))
            };
            if let Some(reason) = args.stop {
                result = result.stopping_turn(reason);
            }
            match args.payload {
                Some(payload) => result.with_payload(payload),
                None => result,
            }
        })
    }
}

/// Makes `link` a link to the directory `target`, which is written as a
/// link holds it: relative to the link's own directory. On Windows a
/// directory link, which the hosted runners may make.
pub fn make_link(target: &str, link: &std::path::Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("make the link");
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(target, link).expect("make the link");
}

// ------------------------------------------------------------------ policy

/// How a test decides a call itself, from what it is shown: `None` leaves
/// it to the decisions scripted by name.
pub type DecideBy = Arc<dyn Fn(&PlannedCall<'_>) -> Option<Decision> + Send + Sync>;

/// A policy a test scripts by tool name.
#[derive(Default)]
pub struct ScriptedPolicy {
    decisions: Mutex<BTreeMap<String, Decision>>,
    /// Decides before the decisions by name, when set.
    decide_by: Mutex<Option<DecideBy>>,
    /// What [`Policy::narrowed`] says.
    narrowed: std::sync::atomic::AtomicU64,
    /// The calls whose question was answered "allow": decided again, such
    /// a call is allowed rather than asked again, as the core's policy
    /// does with the same question (`docs/architecture/03-agent-loop.md`,
    /// "A call is decided again").
    allowed: Mutex<Vec<cineris_protocol::CallId>>,
    /// The answers to come for each tool's questions, in order. With none
    /// left the answer is [`ScriptedPolicy::answer_all`]'s.
    answers: Mutex<BTreeMap<String, VecDeque<Verdict>>>,
    /// The answer to a question with none scripted. Without one, such a
    /// question's call is refused and the test fails when its rig goes,
    /// naming the call: a default answer could hide a question that should
    /// not have been asked.
    unscripted: Mutex<Option<Verdict>>,
    /// The calls that were asked about with no answer scripted, by their
    /// plan's summary: each fails the test when its rig goes.
    unscripted_asked: Mutex<Vec<String>>,
    /// The calls it was shown to decide, by their plan's summary, in order.
    decided: Mutex<Vec<String>>,
    /// A session to interrupt at the moment a question is answered, so
    /// that the answer is on its way when the loop sees the interrupt.
    interrupts: Mutex<Option<SessionHandle>>,
    /// The tools whose question, or whose decision, panics.
    panics_asked: Mutex<Vec<String>>,
    panics_deciding: Mutex<Vec<String>>,
    /// The calls that were asked about, by their plan's summary, in order.
    asked: Mutex<Vec<String>>,
    /// What each question was put with, in order: the arguments it showed
    /// and what it showed of the call's hooks.
    asked_with: Mutex<Vec<Asked>>,
    /// The questions that were dropped before they were answered.
    withdrawn: Arc<Mutex<Vec<String>>>,
    /// Each question waits at the gate named after its tool, when one is
    /// set.
    wait: Mutex<Option<Gates>>,
    /// What `answers_may_stop` says. `None`: what `AllowAll` says.
    may_stop: Mutex<Option<bool>>,
    wake: Notify,
}

impl ScriptedPolicy {
    /// Decides the calls of `what`: a tool's name, or one call's plan
    /// summary (`write a`), which comes first.
    pub fn decide_for(&self, what: &str, decision: Decision) {
        lock(&self.decisions).insert(what.to_owned(), decision);
    }

    /// Decides each call with `by` first.
    pub fn decide_by(
        &self,
        by: impl Fn(&PlannedCall<'_>) -> Option<Decision> + Send + Sync + 'static,
    ) {
        *lock(&self.decide_by) = Some(Arc::new(by));
    }

    /// Says that the rules have narrowed: a rule was taken back.
    pub fn narrow(&self) {
        self.narrowed
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// The calls it was shown to decide, by their plan's summary, in order.
    pub fn decided(&self) -> Vec<String> {
        lock(&self.decided).clone()
    }

    /// Says whether an answer may be to stop the turn.
    pub fn answers_may_stop(&self, may: bool) {
        *lock(&self.may_stop) = Some(may);
    }

    /// Adds the answer to the next question about `tool` that has none yet.
    pub fn answer_for(&self, tool: &str, verdict: Verdict) {
        lock(&self.answers)
            .entry(tool.to_owned())
            .or_default()
            .push_back(verdict);
    }

    /// Answers every question that has no answer of its own with `verdict`.
    pub fn answer_all(&self, verdict: Verdict) {
        *lock(&self.unscripted) = Some(verdict);
    }

    /// Forgets what the test scripted (the decisions, the answers, the
    /// gate its questions wait at), for the next part of a test that runs
    /// its parts as turns of one session. What it was shown and asked is
    /// kept.
    pub fn forget(&self) {
        lock(&self.decisions).clear();
        *lock(&self.decide_by) = None;
        lock(&self.answers).clear();
        *lock(&self.unscripted) = None;
        *lock(&self.wait) = None;
    }

    /// Makes every question wait at the gate `ask <tool>`.
    pub fn wait_at(&self, gates: &Gates) {
        *lock(&self.wait) = Some(gates.clone());
    }

    pub fn asked(&self) -> Vec<String> {
        lock(&self.asked).clone()
    }

    /// What each question was put with, in order.
    pub fn asked_with(&self) -> Vec<Asked> {
        lock(&self.asked_with).clone()
    }

    /// Interrupts `session` in the same step as the next answer is given.
    pub fn interrupt_as_it_answers(&self, session: &SessionHandle) {
        *lock(&self.interrupts) = Some(session.clone());
    }

    /// Makes the question about `tool` panic.
    pub fn panic_when_asked(&self, tool: &str) {
        lock(&self.panics_asked).push(tool.to_owned());
    }

    /// Makes the decision about `tool` panic.
    pub fn panic_when_deciding(&self, tool: &str) {
        lock(&self.panics_deciding).push(tool.to_owned());
    }

    /// Waits until `count` calls have been planned and shown to the policy,
    /// and the loop has had the chance to start whatever it would start.
    pub async fn decisions(&self, count: usize) {
        loop {
            if lock(&self.decided).len() >= count {
                break;
            }
            self.wake.notified().await;
        }
        // The decision is made in the task that planned the call, which
        // then reports to the loop, which then starts the call, which logs
        // when it is first polled: a few turns of the runtime.
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    pub fn withdrawn(&self) -> Vec<String> {
        lock(&self.withdrawn).clone()
    }

    /// Waits until `count` questions have been put.
    pub async fn questions(&self, count: usize) {
        loop {
            if lock(&self.asked).len() >= count {
                return;
            }
            self.wake.notified().await;
        }
    }
}

/// What one question was put with.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    /// The plan's summary.
    pub summary: String,
    /// The arguments the question showed.
    pub arguments: Json,
    /// What it showed of the call's hooks.
    pub hooks: Option<cineris_agent::HookQuestion>,
}

/// Notes a question that is dropped while it is still open.
struct OpenQuestion {
    summary: Option<String>,
    withdrawn: Arc<Mutex<Vec<String>>>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let asked = lock(&self.policy.unscripted_asked).clone();
        if !asked.is_empty() && !std::thread::panicking() {
            panic!(
                "the loop asked about {asked:?}, and the test scripted no answer: give one with \
                 `policy.answer_for(tool, …)`, or answer every question with \
                 `policy.answer_all(…)`"
            );
        }
    }
}

impl Drop for OpenQuestion {
    fn drop(&mut self) {
        if let Some(summary) = self.summary.take() {
            lock(&self.withdrawn).push(summary);
        }
    }
}

impl Policy for ScriptedPolicy {
    fn decide(&self, call: &PlannedCall<'_>) -> Decision {
        lock(&self.decided).push(call.plan.summary.clone());
        self.wake.notify_one();
        assert!(
            !lock(&self.panics_deciding).contains(&call.spec.name),
            "the policy was told to panic deciding"
        );
        let by = lock(&self.decide_by).clone();
        let decision = by.and_then(|by| by(call)).unwrap_or_else(|| {
            let decisions = lock(&self.decisions);
            decisions
                .get(&call.plan.summary)
                .or_else(|| decisions.get(&call.spec.name))
                .cloned()
                .unwrap_or(Decision::Allow)
        });
        if decision == Decision::Ask && lock(&self.allowed).contains(call.call_id) {
            return Decision::Allow;
        }
        decision
    }

    fn answers_may_stop(&self) -> bool {
        lock(&self.may_stop).unwrap_or_else(|| cineris_agent::AllowAll.answers_may_stop())
    }

    fn narrowed(&self) -> u64 {
        self.narrowed.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn read_gate(&self) -> Arc<dyn cineris_tools::ReadGate> {
        cineris_agent::AllowAll.read_gate()
    }

    fn ask<'a>(&'a self, call: PlannedCall<'a>) -> BoxFuture<'a, Verdict> {
        Box::pin(async move {
            let tool = call.spec.name.clone();
            lock(&self.asked_with).push(Asked {
                summary: call.plan.summary.clone(),
                arguments: call.arguments.clone(),
                hooks: call.hooks.cloned(),
            });
            lock(&self.asked).push(call.plan.summary.clone());
            self.wake.notify_one();
            assert!(
                !lock(&self.panics_asked).contains(&tool),
                "the policy was told to panic asking"
            );
            let mut open = OpenQuestion {
                summary: Some(call.plan.summary.clone()),
                withdrawn: Arc::clone(&self.withdrawn),
            };
            let gate = lock(&self.wait)
                .as_ref()
                .map(|gates| gates.get(&format!("ask {tool}")));
            if let Some(gate) = gate {
                gate.opened().await;
            }
            open.summary = None;
            if let Some(session) = lock(&self.interrupts).take() {
                let _ = session.interrupt();
            }
            let scripted = lock(&self.answers)
                .get_mut(&tool)
                .and_then(VecDeque::pop_front);
            let unscripted = lock(&self.unscripted).clone();
            // A panic here would be the call's error and nothing more (the
            // loop turns a question's panic into one), so the question is
            // refused, nothing runs, and the rig fails the test when it
            // goes ([`Rig`]'s `Drop`).
            let verdict = scripted.or(unscripted).unwrap_or_else(|| {
                lock(&self.unscripted_asked).push(call.plan.summary.clone());
                Verdict::Deny {
                    note: Some("the test scripted no answer".to_owned()),
                }
            });
            if verdict == Verdict::Allow {
                lock(&self.allowed).push(call.call_id.clone());
            }
            verdict
        })
    }
}

// ---------------------------------------------------------------- verifier

/// A verifier that returns what a test scripted, round by round.
#[derive(Default)]
pub struct ScriptedVerifier {
    pub outcomes: Mutex<VecDeque<VerifyOutcome>>,
    /// The rounds that ran: the round number and how many files had been
    /// written.
    pub rounds: Mutex<Vec<(u32, usize)>>,
    /// Whether each round was asked for (`verify/run`), and the most rounds
    /// its turn had.
    pub asked: Mutex<Vec<(bool, u32)>>,
    pub gate: Mutex<Option<Gate>>,
}

impl Verifier for ScriptedVerifier {
    fn verify<'a>(&'a self, cx: VerifyCx<'a>) -> BoxFuture<'a, VerifyOutcome> {
        Box::pin(async move {
            lock(&self.rounds).push((cx.round, cx.written.len()));
            lock(&self.asked).push((cx.asked, cx.rounds));
            let gate = lock(&self.gate).clone();
            if let Some(gate) = gate
                && cx.cancel.race(gate.opened()).await.is_none()
            {
                return VerifyOutcome::Skipped;
            }
            lock(&self.outcomes)
                .pop_front()
                .unwrap_or(VerifyOutcome::Skipped)
        })
    }
}

// ------------------------------------------------------------------- stats

#[derive(Default)]
pub struct StatLog {
    events: Mutex<Vec<StatEvent>>,
}

impl Stats for StatLog {
    fn record(&self, event: StatEvent) {
        lock(&self.events).push(event);
    }
}

impl StatLog {
    pub fn all(&self) -> Vec<StatEvent> {
        lock(&self.events).clone()
    }
}

/// Lets every task that can run without the test's help run.
pub async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

// --------------------------------------------------------------------- rig

/// What every session of a test may share: the clock, the provider's
/// script, the tools' log and gates.
pub struct World {
    /// The seed of this test's order of events.
    pub seed: u64,
    /// Keeps the blocking pool's work on this thread, in the seeded order,
    /// for as long as the world lasts.
    _order: cineris_tools::InlineBlocking,
    pub runtime: tokio::runtime::Runtime,
    pub clock: FakeClock,
    pub provider: ScriptedProvider,
    pub log: Arc<Log>,
    pub gates: Gates,
    pub cooldowns: SpeedCooldowns,
    /// Where the sessions' workspaces are.
    pub dir: PathBuf,
    /// The directory itself, removed when the world is dropped.
    _scratch: cineris_testkit::Scratch,
}

impl World {
    /// A world whose order of events has the seed of this test run: 0, or
    /// what `cargo xtask stress` or `CINERIS_TEST_SEED` set. It prints the
    /// seed, which the test runner shows if the test fails.
    pub fn new(test: &str, responses: impl IntoIterator<Item = Response>) -> Self {
        Self::seeded(test, cineris_testkit::seed::for_this_test(), responses)
    }

    /// A world whose order of events has this seed.
    pub fn seeded(test: &str, seed: u64, responses: impl IntoIterator<Item = Response>) -> Self {
        let _set_up = cineris_testkit::profile::phase("set-up");
        let clock = FakeClock::starting_at("2026-11-13T10:00:00Z").unwrap();
        let shared: SharedClock = Arc::new(clock.clone());
        let scratch = cineris_testkit::scratch!(test);
        Self {
            seed,
            _order: cineris_tools::InlineBlocking::seeded(seed),
            runtime: tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a runtime"),
            cooldowns: SpeedCooldowns::new(shared),
            clock,
            provider: ScriptedProvider::new(responses),
            log: Arc::default(),
            gates: Gates::default(),
            dir: scratch.path().to_path_buf(),
            _scratch: scratch,
        }
    }

    pub fn shared_clock(&self) -> SharedClock {
        Arc::new(self.clock.clone())
    }

    /// The fake tools: `read` and `peek` (read-only), `write` (keyed by
    /// path), `shell` (exclusive), `plain` (touches nothing), `solo`
    /// (declares nothing and runs alone) and `panic`.
    pub fn toolbox(&self) -> Toolbox {
        let probe = |name: &str, kind: Kind| Probe::new(name, kind, &self.log, &self.gates);
        Toolbox::new()
            .with(probe("read", Kind::Read))
            .with(probe("peek", Kind::Read))
            .with(probe("write", Kind::Write))
            .with(probe("shell", Kind::Shell))
            .with(probe("plain", Kind::Plain))
            .with(probe("solo", Kind::Solo))
            .with(probe("panic", Kind::Panic))
    }

    /// Runs a future on the test's runtime, with the sessions.
    ///
    /// The run is named to the test kit's watch, so that a test still in
    /// it at four fifths of its limit says so, with its threads' states,
    /// before nextest stops it. A test of this rig uses none of the test
    /// kit's waits as a rule, and one that ran out of time once on the
    /// reference machine said nothing for itself
    /// (`docs/outstanding/a-summary-test-that-ran-out-of-time-once.md`).
    pub fn run<T>(&self, future: impl Future<Output = T>) -> T {
        let _running = cineris_testkit::watch::doing(format!(
            "running the test's future on the world's runtime (seed {})",
            self.seed
        ));
        self.runtime.block_on(future)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // What follows, the runtime and the directory going, is tear-down
        // in the test's time profile.
        cineris_testkit::profile::tear_down();
    }
}

/// A panic in a session's task would leave the test waiting for an event
/// that never comes, until the runner's time limit. This ends the test at
/// once, with the panic's message already printed.
struct AbortIfItPanics<F>(std::pin::Pin<Box<F>>);

impl<F: Future<Output = ()>> Future for AbortIfItPanics<F> {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let inner = self.0.as_mut();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.poll(context))) {
            Ok(poll) => poll,
            Err(_) => std::process::abort(),
        }
    }
}

/// How one session of a test is put together.
pub struct Setup {
    pub id: String,
    pub config: LoopConfig,
    pub ledger: Option<Ledger>,
    pub speeds: SpeedSettings,
    pub model: RoleModel,
    /// The model of the `fast` utility role, when the session has one.
    pub utility: Option<RoleModel>,
    pub store: Option<Box<dyn SessionStore>>,
    pub memory: MemorySessionStore,
    pub toolbox: Option<Toolbox>,
    /// The context's settings: the thresholds, the names of instruction
    /// files.
    pub context: ContextSettings,
    /// The instruction files the session can read.
    pub instructions: Arc<MemoryInstructionFiles>,
    /// The user's own instruction directory.
    pub user_dir: Option<cineris_base::AbsPath>,
    /// The plan a compaction carries.
    pub plan: Option<Arc<cineris_tools::PlanBoard>>,
    /// A verifier in place of the scripted one.
    pub verifier: Option<Arc<dyn Verifier>>,
    /// The workspace's path, in place of one under the world's directory:
    /// a twin's ([`Rig::fill`]), which must be where its instruction files
    /// are.
    pub root: Option<PathBuf>,
    /// Where the repository map comes from, when the session has one.
    pub map: Option<cineris_agent::context::RepoMap>,
    /// Where the project profile comes from, when the session has one.
    pub profile: Option<Arc<dyn cineris_agent::ProfileSource>>,
    /// Where sections 6 and 7 of the prompt come from, when the session
    /// has extensions.
    pub extensions: Option<Arc<dyn cineris_agent::ExtensionsSource>>,
    /// The session's hooks, when it has any (`hooks.rs`'s fakes).
    pub hooks: Option<cineris_agent::LoopHooks>,
}

/// One session, with everything it does remembered.
pub struct Rig {
    /// What a twin of this session is made from, to measure it
    /// ([`Rig::fill`]).
    twin: Twin,
    /// The window [`Rig::window`] set, if it did.
    window: std::sync::Mutex<Option<u64>>,
    pub handle: SessionHandle,
    pub events: Arc<Events>,
    pub seen: Arc<Seen>,
    pub stats: Arc<StatLog>,
    pub policy: Arc<ScriptedPolicy>,
    pub verifier: Arc<ScriptedVerifier>,
    pub instructions: Arc<MemoryInstructionFiles>,
    pub store: MemorySessionStore,
    pub workspace: Workspace,
}

impl Setup {
    pub fn new(world: &World) -> Self {
        Self {
            id: "s_1".to_owned(),
            config: LoopConfig::default(),
            ledger: None,
            speeds: SpeedSettings::default(),
            model: RoleModel::new(Arc::new(world.provider.clone()), fixtures::model()),
            utility: None,
            store: None,
            memory: MemorySessionStore::new(),
            toolbox: None,
            context: ContextSettings::default(),
            instructions: Arc::default(),
            user_dir: None,
            plan: None,
            verifier: None,
            root: None,
            map: None,
            profile: None,
            extensions: None,
            hooks: None,
        }
    }

    /// Opens the session and starts its loop on the world's runtime.
    pub fn start(self, world: &World) -> Rig {
        let root = self
            .root
            .clone()
            .unwrap_or_else(|| world.dir.join(&self.id));
        let twin = Twin {
            id: self.id.clone(),
            root: root.clone(),
            config: self.config.clone(),
            model: self.model.clone(),
            toolbox: self.toolbox.clone(),
            context: self.context.clone(),
            instructions: Arc::clone(&self.instructions),
            user_dir: self.user_dir.clone(),
        };
        std::fs::create_dir_all(&root).expect("create the workspace");
        let paths = Arc::new(PathResolver::new(&root, PathOptions::new()).expect("a workspace"));
        let workspace = Workspace::in_memory(paths);
        let toolbox = self.toolbox.unwrap_or_else(|| world.toolbox());
        let clock = world.shared_clock();
        let events = Arc::new(Events::default());
        let seen = Arc::new(Seen::default());
        let stats = Arc::new(StatLog::default());
        let policy = Arc::new(ScriptedPolicy::default());
        let verifier = Arc::new(ScriptedVerifier::default());
        let context = ContextParts {
            settings: self.context,
            instructions: Arc::clone(&self.instructions)
                as Arc<dyn cineris_agent::InstructionFiles>,
            user_dir: self.user_dir,
            plan: self.plan,
            map: self.map,
            profile: self.profile,
            extensions: self.extensions,
            ..ContextParts::new()
        };
        let parts = SessionParts {
            info: SessionInfo::new(SessionId::new(self.id), "/work/test", &clock),
            models: match self.utility {
                Some(utility) => Models::new(self.model, "test-1")
                    .with_role(cineris_protocol::model::ModelRole::Fast, utility),
                None => Models::new(self.model, "test-1"),
            },
            speeds: self.speeds,
            tools: Arc::new(toolbox),
            policy: Arc::clone(&policy) as Arc<dyn Policy>,
            store: self.store.unwrap_or_else(|| Box::new(self.memory.clone())),
            context,
            events: Arc::clone(&events) as Arc<dyn EventSink>,
            stats: Arc::clone(&stats) as Arc<dyn Stats>,
            verifier: self
                .verifier
                .unwrap_or_else(|| Arc::clone(&verifier) as Arc<dyn Verifier>),
            clock: Arc::clone(&clock),
            cooldowns: world.cooldowns.clone(),
            ledger: self.ledger.unwrap_or_else(|| Ledger::new(clock)),
            workspace: workspace.clone(),
            config: self.config,
            cancel: CancelToken::new(),
            observer: Some(Arc::clone(&seen) as Arc<dyn Observer>),
            hooks: self.hooks,
        };
        let (session, handle) = world
            .runtime
            .block_on(Session::open(parts))
            .expect("the session opens");
        world
            .runtime
            .spawn(AbortIfItPanics(Box::pin(session.run())));
        Rig {
            twin,
            window: std::sync::Mutex::new(None),
            handle,
            events,
            seen,
            stats,
            policy,
            verifier,
            instructions: self.instructions,
            store: self.memory,
            workspace,
        }
    }
}

impl Rig {
    /// A world with one session, at its defaults.
    pub fn one(test: &str, responses: impl IntoIterator<Item = Response>) -> (World, Self) {
        let world = World::new(test, responses);
        let rig = Setup::new(&world).start(&world);
        (world, rig)
    }

    /// Sends a prompt that starts a turn.
    pub fn prompt(&self, text: &str) -> TurnStartResult {
        self.handle
            .prompt(vec![Part::text(text)], None)
            .map_err(|error| *error)
            .expect("the session is idle")
    }

    /// Sends a prompt, queues another behind it while the first turn is
    /// still at its first request, and waits for the first turn to end.
    pub fn queue_after_start(&self, world: &World, queued: &str) {
        world.run(async {
            self.prompt("go");
            self.queue(queued);
            self.events.turns(1).await;
        });
    }

    pub fn steer(&self, text: &str) -> TurnStartResult {
        self.handle
            .prompt(vec![Part::text(text)], Some(Delivery::Steer))
            .map_err(|error| *error)
            .expect("a turn is running")
    }

    /// Holds the model's requests to a context window of `tokens`, as a
    /// local server's report would.
    pub fn window(&self, tokens: u64) {
        *self.window.lock().unwrap() = Some(tokens);
        self.handle.set_context_window(
            cineris_protocol::ProviderId::new(fixtures::PROVIDER),
            cineris_protocol::ModelId::new(fixtures::MODEL),
            tokens,
        );
    }

    /// Gives the next turn a notice of `bytes` bytes: bulk that fills the
    /// context and is not carried across a compaction.
    pub fn bulk(&self, bytes: usize) {
        self.handle.notice(
            cineris_protocol::item::NoticeSource::Harness,
            "x".repeat(bytes),
        );
    }

    pub fn queue(&self, text: &str) -> TurnStartResult {
        self.handle
            .prompt(vec![Part::text(text)], Some(Delivery::Queue))
            .map_err(|error| *error)
            .expect("a turn is running")
    }
}

// ---------------------------------------------------------------- pressure

/// How full a session's next request is, against its context thresholds
/// (`docs/architecture/07-sessions-and-context.md`, "Budgeting").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// Below the threshold for a summary in the background.
    Below,
    /// Past that threshold, below the compaction trigger: a summary is
    /// prepared while the turn goes on.
    Preparing,
    /// Past the trigger, within the limit: the epoch is compacted before
    /// the request.
    Compacting,
    /// Past the model's window itself.
    PastWindow,
}

/// What [`Rig::fill`] measured and chose, in tokens unless it says bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filled {
    /// The request without the bulk, in bytes, as the twin measured it.
    pub base_bytes: u64,
    /// The bulk given, in bytes.
    pub bulk: usize,
    /// The estimate the request will have.
    pub estimate: u64,
    /// The limit, and the two thresholds of it.
    pub limit: u64,
    pub prepare: f64,
    pub compact: f64,
    pub window: u64,
}

/// What a twin of a rig's session is made from: everything that goes into
/// its first request's size.
struct Twin {
    id: String,
    root: PathBuf,
    config: LoopConfig,
    model: RoleModel,
    toolbox: Option<Toolbox>,
    context: ContextSettings,
    instructions: Arc<MemoryInstructionFiles>,
    user_dir: Option<cineris_base::AbsPath>,
}

/// The notice the twin is given: large enough that the estimate's rounding
/// to a whole token is small beside it.
const MEASURE_BULK: usize = 3_000;

impl Rig {
    /// Gives the next turn the bulk that puts its first request at `fill`,
    /// in the window [`Rig::window`] set (the model's own when it set
    /// none), and returns what it measured. `prompt` is the text the test
    /// will send.
    ///
    /// The size of the request without the bulk is not counted by hand: a
    /// twin of this session (the same prompt, tools, instruction files,
    /// thresholds and model, at the same workspace path) is run once in a
    /// world of its own with a window it cannot fill, and its request's
    /// estimate is read from its context record. The bulk is then chosen
    /// to put the estimate in the middle of the band asked for, at the
    /// ratio of bytes to tokens the loop uses before the provider has
    /// counted a request.
    ///
    /// Call it before the session's first turn, and outside `world.run`:
    /// the twin has a runtime of its own. For a session that has history,
    /// see [`Rig::fill_next`] and [`Rig::fill_after`].
    pub fn fill(&self, fill: Fill, prompt: &str) -> Filled {
        assert!(
            !self.handle.has_history(),
            "Rig::fill sizes a session's first request: this one has history (Rig::fill_next \
             sizes the first request of its next turn)"
        );
        let filled = self.size(fill, prompt, Vec::new());
        self.bulk(filled.bulk);
        filled
    }

    /// [`Rig::fill`] for a session that has history: gives the next turn the
    /// bulk that puts its first request at `fill`, the second turn's first
    /// request of a test that runs two. Between turns, outside
    /// `world.run`.
    ///
    /// The twin is the session as it stands: it is resumed from a copy of
    /// the items the rig's store holds, and runs `prompt` as its next turn.
    /// So the history counts, as it will in the request. Refused, with the
    /// numbers, when the band cannot be reached with that history in the
    /// window; and refused when the session has no items in the rig's
    /// store, has begun a second context epoch, or has had a request
    /// counted by its provider (a response that reported tokens): the loop
    /// then estimates from the count, and the twin has none.
    pub fn fill_next(&self, fill: Fill, prompt: &str) -> Filled {
        assert!(
            self.handle.has_history(),
            "Rig::fill_next sizes the first request of a turn after the first: this session has \
             no history (Rig::fill sizes its first request)"
        );
        let filled = self.size(fill, prompt, Vec::new());
        self.bulk(filled.bulk);
        filled
    }

    /// Sizes a later request of a turn: the one that follows `earlier`, the
    /// responses of the turn `prompt` starts that come before it (the call
    /// of a tool round, whose result is in the request). Returns what it
    /// measured and gives the session nothing: the bulk is delivered with
    /// `rig.bulk(filled.bulk)` at the moment the test chooses while the
    /// turn runs (a notice reaches the turn at its next request), since
    /// given at the start of the turn it would be in the first request too.
    ///
    /// The twin runs the turn: the session's history as [`Rig::fill_next`]
    /// has it, then `prompt`, then `earlier` in order, then one request
    /// that it measures. So `earlier` must be what the session will be
    /// given, without a wait (a gate in a response would hold the twin for
    /// ever), and the session's tools must answer as the twin's do. Refused,
    /// with the numbers, when the band cannot be reached; and when the twin's
    /// turn ends before the request is made, or a response reported tokens.
    pub fn fill_after(
        &self,
        fill: Fill,
        prompt: &str,
        earlier: impl IntoIterator<Item = Response>,
    ) -> Filled {
        self.size(fill, prompt, earlier.into_iter().collect())
    }

    /// The bulk that puts the request that follows `earlier` in the turn
    /// `prompt` starts at `fill`, measured on a twin.
    fn size(&self, fill: Fill, prompt: &str, earlier: Vec<Response>) -> Filled {
        let entry = &self.twin.model.entry.info;
        let window = self.window.lock().unwrap().unwrap_or(entry.context_window);
        let max_output = self.twin.model.max_output.unwrap_or(entry.max_output);
        let limit = cineris_agent::context::limit_of(window, max_output);
        #[expect(clippy::cast_precision_loss, reason = "token counts far below 2^52")]
        let (prepare, compact) = (
            self.twin.context.prepare_at * limit as f64,
            self.twin.context.compact_at * limit as f64,
        );
        let base_bytes = self.measure(prompt, earlier);
        let ratio = cineris_agent::context::DEFAULT_RATIO;
        #[expect(clippy::cast_precision_loss, reason = "byte counts far below 2^52")]
        let base = base_bytes as f64 / ratio;
        #[expect(clippy::cast_precision_loss, reason = "token counts far below 2^52")]
        let (low, high) = match fill {
            Fill::Below => (base, prepare),
            Fill::Preparing => (prepare, compact),
            Fill::Compacting => (compact, limit as f64),
            Fill::PastWindow => (window as f64 * 1.05, window as f64 * 1.15),
        };
        assert!(
            base + 2.0 < high,
            "{fill:?} cannot be reached: the request is {base:.0} tokens before any bulk, and \
             the band ends at {high:.0} of a limit of {limit} (window {window}). Give the \
             model a larger window."
        );
        let target = (low.max(base) + high) / 2.0;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a positive byte count far below 2^52"
        )]
        let bulk = (target * ratio) as usize - usize::try_from(base_bytes).unwrap();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a positive token count far below 2^52"
        )]
        let estimate = ((base_bytes as f64 + bulk as f64) / ratio).ceil() as u64;
        Filled {
            base_bytes,
            bulk,
            estimate,
            limit,
            prepare,
            compact,
            window,
        }
    }

    /// The bytes of the request that follows `earlier` in the turn `prompt`
    /// starts, without its bulk, from a twin's context record. The twin has
    /// this session's history, if it has one.
    fn measure(&self, prompt: &str, earlier: Vec<Response>) -> u64 {
        static TWINS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let number = TWINS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let before = earlier.len();
        let world = World::seeded(
            &format!("twin-{}-{number}", std::process::id()),
            0,
            earlier.into_iter().chain([says("Measured.")]),
        );
        let mut setup = Setup::new(&world);
        setup.id.clone_from(&self.twin.id);
        setup.root = Some(self.twin.root.clone());
        setup.config = self.twin.config.clone();
        setup.model = RoleModel {
            client: Arc::new(world.provider.clone()),
            ..self.twin.model.clone()
        };
        setup.toolbox.clone_from(&self.twin.toolbox);
        setup.context = self.twin.context.clone();
        setup.instructions = Arc::clone(&self.twin.instructions);
        setup.user_dir.clone_from(&self.twin.user_dir);
        if self.handle.has_history() {
            setup.memory = self.history_so_far();
        }
        let twin = setup.start(&world);
        // A window the request cannot come near.
        twin.window(u64::MAX / 4);
        twin.bulk(MEASURE_BULK);
        world.run(async {
            twin.prompt(prompt);
            twin.events.turns(1).await;
        });
        let contexts_of = |stats: &StatLog| -> Vec<cineris_agent::ContextStat> {
            stats
                .all()
                .into_iter()
                .filter_map(|event| match event {
                    StatEvent::Context(context) => Some(context),
                    _ => None,
                })
                .collect()
        };
        let contexts = contexts_of(&twin.stats);
        for context in contexts.iter().chain(&contexts_of(&self.stats)) {
            let counted = context.usage.as_ref().map_or(0, |usage| {
                usage.input + usage.cache_read + usage.cache_write
            });
            assert!(
                counted == 0,
                "a request of the session was counted by its provider ({counted} input tokens): \
                 the loop estimates from the count, which the twin has not, so it cannot size a \
                 request by pressure here"
            );
        }
        let estimate = contexts
            .get(before)
            .unwrap_or_else(|| {
                panic!(
                    "the twin's turn made {} request(s), and the one to size is number {}: \
                     `earlier` must hold every response of the turn that comes before it",
                    contexts.len(),
                    before + 1
                )
            })
            .estimate;
        // The estimate is the bytes at the ratio, rounded up to a token.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a positive byte count far below 2^52"
        )]
        let bytes = (estimate as f64 * cineris_agent::context::DEFAULT_RATIO) as u64;
        bytes.saturating_sub(MEASURE_BULK as u64)
    }

    /// A store of its own that holds what this session's store holds now,
    /// for a twin to be resumed from.
    fn history_so_far(&self) -> MemorySessionStore {
        let items = self.store.items();
        assert!(
            !items.is_empty(),
            "the rig's store holds no items, so a twin cannot be resumed from them (a session \
             given a store of its own in `Setup::store` cannot be sized with history)"
        );
        assert!(
            self.store.epochs().len() <= 1 && self.store.summaries().is_empty(),
            "the session has compacted: a twin resumed from its items would not hold what its \
             next request holds"
        );
        let turn = self
            .store
            .records()
            .last()
            .map(|(turn, _)| turn.clone())
            .expect("a record");
        MemorySessionStore::with_items(&turn, items)
    }
}

// ----------------------------------------------------------------- history

/// What the first user message of a test's first epoch begins with: the
/// date of the world's clock.
pub const DATE: &str = "Today's date is 2026-11-13 (UTC).";

/// The text of a message's text parts.
pub fn text_of(message: &Message) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Checks a conversation the way a provider does before it accepts a
/// request: it begins with the user, no message is empty, every tool call
/// of an assistant message has exactly one result, in the message right
/// after it and in call order, and no result is without its call.
pub fn assert_valid_history(messages: &[Message]) {
    assert!(!messages.is_empty(), "a request has messages");
    assert_eq!(messages[0].role, Role::User, "the user speaks first");
    for (index, message) in messages.iter().enumerate() {
        assert!(
            !message.parts.is_empty(),
            "message {index} is empty: {messages:#?}"
        );
        let calls: Vec<&str> = message
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::ToolCall(call) => Some(call.id.as_str()),
                _ => None,
            })
            .collect();
        match message.role {
            Role::Assistant if !calls.is_empty() => {
                let results: Vec<&str> = messages
                    .get(index + 1)
                    .filter(|next| next.role == Role::Tool)
                    .map(|next| {
                        next.parts
                            .iter()
                            .filter_map(|part| match part {
                                Part::ToolResult(result) => Some(result.call_id.as_str()),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                assert_eq!(
                    results, calls,
                    "the calls of message {index} and the results after it: {messages:#?}"
                );
            }
            Role::Tool => {
                let after_calls = index > 0
                    && messages[index - 1].role == Role::Assistant
                    && messages[index - 1]
                        .parts
                        .iter()
                        .any(|part| matches!(part, Part::ToolCall(_)));
                assert!(
                    after_calls,
                    "results at {index} follow no call: {messages:#?}"
                );
                assert!(
                    message
                        .parts
                        .iter()
                        .all(|part| matches!(part, Part::ToolResult(_))),
                    "a tool message holds results only"
                );
            }
            _ => assert!(calls.is_empty(), "only the assistant calls tools"),
        }
    }
}

/// The last request the provider received.
pub fn last_request(provider: &ScriptedProvider) -> ModelRequest {
    provider.requests().pop().expect("a request was sent")
}

/// A response that calls one fake tool and asks for the round to run.
pub fn calls(calls: &[(&str, &str, Json)]) -> Response {
    let mut response = Response::new();
    for (id, tool, arguments) in calls {
        response = response.tool_call(id, tool, arguments);
    }
    response.end(StopReason::ToolCalls)
}

/// A response that only says something.
pub fn says(text: &str) -> Response {
    Response::new().text(text).end(StopReason::End)
}
