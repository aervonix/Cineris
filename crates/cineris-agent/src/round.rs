//! One tool round: the calls of one response, planned, approved, run and
//! handed back in the model's order
//! (`docs/architecture/03-agent-loop.md`, "Tool rounds").
//!
//! A [`Round`] begins with the request, not with the end of the response:
//! a call is planned the moment its block ends, a question is put the
//! moment the plan needs one, and a call with no side effects starts
//! running while the model is still writing.
//!
//! # How it runs
//!
//! Planning, a question and a tool run are each a task. A task reports on
//! one channel, and the round changes only when the loop hands it a report
//! ([`Round::handle`]), so every decision about what may start is made in
//! one place, [`Round::pump`], by these rules:
//!
//! - a call waits for every **earlier** call it conflicts with
//!   ([`Plan::conflicts_with`]), and for every earlier call whose plan is
//!   not known yet, or whose block has not ended yet;
//! - a call **with side effects** waits for every earlier call whose
//!   question is open or still to be put, when the answer could be to stop
//!   the turn ([`Policy::answers_may_stop`]): a stop holds for everything
//!   the model wrote after the refused call;
//! - while the response is still arriving only calls **without side
//!   effects** start, and a call that is not scheduled as read-only is taken
//!   to have some;
//! - no more than the limit run at once;
//! - questions are asked one at a time, in call order;
//! - once the policy has said to stop the turn, nothing the model wrote
//!   after the refused call starts, and nothing more is asked. A call it
//!   wrote before the refused one, and that was allowed, still starts:
//!   the turn stops at the refused call, in the model's order. A call
//!   whose run stopped the turn (its result is marked so,
//!   [`ToolResult::stopping_turn`]) stops it in the same way, at that
//!   call;
//! - a call waits for every call it conflicts with that is running, a
//!   later one too: a call decided again may be scheduled otherwise than
//!   it was at first.
//!
//! # Deciding again
//!
//! A call is decided when it is planned, which can be before an earlier
//! call of the same response has run: `ln -s . d` and then a copy to
//! `d/.bashrc` would be judged as the file system was before the link.
//! So a call that has not started is **planned and decided again** as soon
//! as the round looks at what may start, if since it was decided a call
//! that may have changed the file system has finished
//! ([`Footprint::may_change`]), or the policy's rules have narrowed
//! ([`Policy::narrowed`]). Each decision is dated by both counts when its
//! planning starts ([`Dated`]), so a change made while the plan was being
//! worked out is after it. The new decision is acted on as the first was.
//! Until it is in, the call is one whose plan is not known yet: nothing
//! after it that could conflict with it starts, and no question after it
//! is put.
//!
//! # Hooks
//!
//! A call the policy allowed, allowed as read-only, or will ask about is
//! shown to its `tool.before` hooks ([`crate::hooks`]) in the task that
//! planned it, before it reports: so while they are asked the call is one
//! whose plan is not known yet, and nothing that could conflict with it
//! starts. A call the policy denied or stopped is not shown to them. What
//! they say only narrows: a `deny` refuses the call, an `ask` puts the
//! question about a call the policy allowed, and a replacement of the
//! arguments is planned, decided and checked again and then always put to
//! the user. A call decided again is shown to them again, with the model's
//! own arguments; what they said before is dropped.
//!
//! Two rules keep the model's order when a decision changes:
//!
//! - while a call that may change the file system runs, and an answer may
//!   be to stop the turn, a later call with side effects waits for every
//!   earlier call that has not started: decided again when the running
//!   call ends, such a call may need an answer, which may be a stop;
//! - a call planned again onto other paths, which a call written after it
//!   has already used, does not run: its result says so.
//!
//! # Exactly one result
//!
//! Every call the round is given ends with one result, whatever happens:
//! the tool's own, an error for arguments that do not fit, a denial, or
//! "interrupted". A task that panics reports an error instead of nothing.
//!
//! # When a response is thrown away
//!
//! A response can fail part-way and be sent again. Its calls are then void
//! ([`Round::void`]). The ones that already ran had no side effects, and
//! their results are kept in a cache for as long as the request lasts: if
//! the new response asks for the same call, the result is used again, when
//! that call's turn to start comes. One that is not asked for again is
//! discarded, and so that nothing later
//! mistakes it for something the model saw, the files it read are forgotten
//! by the session's table of seen files.

/// An awaited point ([`cineris_base::hook::await_point!`]): a call has
/// started and its tool is about to run, on the call's own task. A test
/// that needs a later call of the round decided before an earlier one has
/// changed anything holds the earlier one here. Nothing in a build without
/// `test-hooks`.
pub const BEFORE_A_CALL_RUNS: &str = "round: a call has started, and its tool is about to run";

/// An awaited point ([`cineris_base::hook::await_point!`]): a call's plan
/// has been made and the policy has decided it, on the call's own planning
/// task, and the round has not been told. A test that needs a call decided
/// before the rules change (its plan runs on the blocking pool, and comes
/// when it comes) waits for it here. Nothing in a build without
/// `test-hooks`.
pub const A_CALL_IS_DECIDED: &str =
    "round: a call is planned and decided, and the round is not told";
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use cineris_base::{AbsPath, CancelToken, IdCounter, SharedClock};
use cineris_protocol::event::{
    Delta, Event, InfoNotice, ItemCompleted, ItemDelta, ItemKind, ItemRef, ItemStarted, Notice,
    TextDelta,
};
use cineris_protocol::item::{HookReason, ReplacedArguments, ToolOutcome, ToolResultItem};
use cineris_protocol::{CallId, Item, ItemId, Part, SessionId, TurnId};
use cineris_tools::{
    BlockingError, CallRef, Concurrency, Effect, Json, Plan, RunCx, Tool, ToolError, ToolOutput,
    ToolResult, bound, execute, run_blocking,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::AbortHandle;

use crate::assistant::EndedCall;
use crate::config::Workspace;
use crate::events::Emitter;
use crate::guarded::Guarded;
use crate::hooks::{
    self, Before, HookEvent, HookInput, HookQuestion, HookSubject, LoopHooks, Observers, ToolTarget,
};
use crate::policy::{Decision, PlannedCall, Policy, Verdict};
use crate::stats::ToolStatOutcome;
use crate::toolset::ToolSet;

/// What every round of a session shares.
pub(crate) struct RoundEnv {
    pub(crate) session: SessionId,
    pub(crate) tools: Arc<dyn ToolSet>,
    pub(crate) policy: Arc<dyn Policy>,
    pub(crate) workspace: Workspace,
    pub(crate) emitter: Arc<Emitter>,
    pub(crate) clock: SharedClock,
    /// The most calls that run at once.
    pub(crate) max_parallel: usize,
    /// The session's hooks, when it has any.
    pub(crate) hooks: Option<LoopHooks>,
}

/// A report from one of the round's tasks.
pub(crate) enum Report {
    /// A call was planned, and the policy, and the call's hooks, have
    /// looked at the plan.
    Planned {
        token: u64,
        plan: Plan,
        decision: Decision,
        /// What its hooks said that puts the question: whatever the
        /// decision, the call is asked about. With a replacement, `plan`
        /// and `decision` are the replacement's.
        hooked: Option<Box<HookQuestion>>,
    },
    /// A question was answered.
    Answered {
        token: u64,
        plan: Plan,
        verdict: Verdict,
    },
    /// A call has its result.
    Finished {
        token: u64,
        result: ToolResult,
        duration: Duration,
        stat: ToolStatOutcome,
        /// What its `tool.after` hooks add to its result, after the output
        /// policy: each note under a line that names its hook.
        notes: Vec<Part>,
    },
}

/// Where the response is when the round is asked what may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// It is still arriving. Only calls without side effects start, and
    /// only those before `open_call`: the index of the first tool call
    /// whose block has not ended yet, which could turn out to conflict with
    /// anything after it.
    Streaming { open_call: Option<u32> },
    /// It has ended.
    Ended,
}

/// What a planned call touches, kept after the plan itself has gone to the
/// tool.
struct Footprint {
    /// A plan with the same effects and scheduling, for
    /// [`Plan::conflicts_with`].
    shadow: Plan,
    side_effects: bool,
    /// Whether its run may have changed the file system: an effect that is
    /// not a read (a command, a write, an extension's tool), or scheduling
    /// that is not read-only. A command the policy found only reads does
    /// not: every part of it is on the table of commands that only read,
    /// none of which makes a link. Nor does a call that changes only the
    /// session (`plan`, `remember`).
    may_change: bool,
    /// Every path an effect names.
    paths: Vec<AbsPath>,
    /// The files it reads one by one: what the table of seen files may
    /// have recorded for it.
    reads: Vec<AbsPath>,
}

impl Footprint {
    fn of(plan: &Plan) -> Self {
        let mut shadow =
            Plan::new(plan.summary.clone(), ()).with_concurrency(plan.concurrency.clone());
        shadow.effects.clone_from(&plan.effects);
        shadow.changes_session = plan.changes_session;
        let mut footprint = Self {
            shadow,
            // A call that is not scheduled as read-only is taken to change
            // something, whatever effects it declares: a tool whose author
            // never said what it does must not start early. And a call that
            // changes the session (`plan`, `remember`) has no effect to
            // declare and is scheduled as read-only, so it says so itself.
            side_effects: plan.has_side_effects()
                || plan.changes_session()
                || !matches!(plan.concurrency, Concurrency::ReadOnly),
            may_change: plan.has_side_effects()
                || !matches!(plan.concurrency, Concurrency::ReadOnly),
            paths: Vec::new(),
            reads: Vec::new(),
        };
        for effect in &plan.effects {
            if let Some(path) = effect.path() {
                footprint.paths.push(path.abs().clone());
            }
            if let Effect::Read(path) = effect {
                footprint.reads.push(path.abs().clone());
            }
        }
        footprint
    }
}

/// What a call's decision was made against: the round's count of finished
/// calls that may have changed the file system, and the policy's count of
/// narrowings, both read when its planning started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Dated {
    changes: u64,
    narrowed: u64,
}

/// Whether any path of one list is a path of the other, or lies inside
/// one.
fn overlap(first: &[AbsPath], second: &[AbsPath]) -> bool {
    first.iter().any(|one| {
        second
            .iter()
            .any(|other| one.starts_with(other) || other.starts_with(one))
    })
}

/// A call's result, ready to record.
struct Outcome {
    item: ToolResultItem,
    stat: ToolStatOutcome,
    duration: Duration,
    /// How long it waited, once allowed, before it ran.
    queued: Duration,
    /// How long it waited for the user's answer.
    approval: Duration,
    /// Why its run stopped the turn, if it did
    /// ([`ToolResult::turn_stop`]).
    turn_stop: Option<String>,
}

/// When things happened to a call, on the monotonic clock: what its time
/// is cut into (`crate::timing`).
#[derive(Debug, Clone, Copy, Default)]
struct CallTimes {
    /// When it was allowed: planned and allowed, or answered yes.
    ready: Option<Duration>,
    /// When its question was put.
    asked: Option<Duration>,
    /// How long it waited for the answer.
    approval: Duration,
    /// When it started to run.
    started: Option<Duration>,
}

impl CallTimes {
    fn queued(&self) -> Duration {
        match (self.ready, self.started) {
            (Some(ready), Some(started)) => started.saturating_sub(ready),
            _ => Duration::ZERO,
        }
    }
}

enum CallState {
    /// A task is at work on it and will report: planning it, or bounding
    /// a result it gets without running.
    Pending,
    /// Planned, and the policy says to ask.
    NeedsAnswer(Plan),
    /// Its question is open.
    Asking,
    /// Allowed, and waiting for its turn to run.
    Ready(Plan),
    /// Running.
    Running,
    /// It has its result. (Boxed: a result is many times the size of the
    /// other states.)
    Done(Box<Outcome>),
}

struct Call {
    /// What its tasks report under.
    token: u64,
    /// The index of its block in the response.
    index: u32,
    call_id: CallId,
    item_id: ItemId,
    name: String,
    arguments: Json,
    /// What a repeat of the call is recognised by.
    key: String,
    tool: Option<Arc<dyn Tool>>,
    footprint: Option<Footprint>,
    state: CallState,
    cancel: CancelToken,
    /// Whether front ends have been told the item began.
    shown: bool,
    /// Whether a tool run was started for it.
    ran: bool,
    /// When things happened to it.
    times: CallTimes,
    /// What its last decision was made against.
    decided: Dated,
    /// While it is planned again: the paths its earlier plan named.
    replanned: Option<Vec<AbsPath>>,
    /// Whether a later plan of it names other paths than an earlier one.
    moved: bool,
    /// What its hooks said that its question shows, from its last
    /// decision: dropped when it is decided again.
    hooked: Option<Box<HookQuestion>>,
    /// Whether the question of its last decision was answered: only then
    /// does its result say the user was asked about a hook's arguments.
    answered: bool,
}

impl Call {
    /// The arguments a hook put in place of the model's, when one did.
    fn replaced(&self) -> Option<&ReplacedArguments> {
        self.hooked
            .as_ref()
            .and_then(|hooked| hooked.replaced.as_ref())
    }

    /// The arguments a hook put in place of the model's, when the user was
    /// asked about them and answered: what its result and its item say.
    fn replaced_and_asked(&self) -> Option<&ReplacedArguments> {
        self.replaced().filter(|_| self.answered)
    }
}

/// A run from a response that was thrown away, kept in case the next
/// response asks for the same call.
struct Cached {
    token: u64,
    name: String,
    key: String,
    item_id: ItemId,
    footprint: Footprint,
    cancel: CancelToken,
    /// `None` while it is still running.
    outcome: Option<Outcome>,
    /// The arguments a hook put in place of the model's, which it ran with.
    replaced: Option<ReplacedArguments>,
}

/// A discarded run that has been cancelled and has not reported yet. Until
/// it does, it may still write to the table of seen files, so nothing that
/// touches the same files starts.
struct Draining {
    token: u64,
    paths: Vec<AbsPath>,
    reads: Vec<AbsPath>,
}

/// One call's result, as the loop records it.
pub(crate) struct Finished {
    pub(crate) item: ToolResultItem,
    pub(crate) stat: ToolStatOutcome,
    pub(crate) duration: Duration,
    /// How long it waited, once allowed, before it ran.
    pub(crate) queued: Duration,
    /// How long it waited for the user's answer.
    pub(crate) approval: Duration,
    /// The files the call wrote, if it succeeded.
    pub(crate) written: Vec<AbsPath>,
    /// What a repeat of the call with the same result is recognised by.
    pub(crate) signature: String,
}

/// The tool calls of one request.
pub(crate) struct Round {
    env: Arc<RoundEnv>,
    turn: TurnId,
    turn_number: u32,
    /// The round's number in the session: one more than the number of
    /// rounds whose results have been recorded. Tools are told it
    /// (`RunCx::round`).
    number: u64,
    /// The turn's token. Every call's token is a child of it.
    cancel: CancelToken,
    /// The turn's second token, "kill now", which every call is given as
    /// it is.
    kill: CancelToken,
    tx: UnboundedSender<Report>,
    rx: UnboundedReceiver<Report>,
    /// The calls of the response that counts, in the model's order: by the
    /// index of their blocks.
    calls: Vec<Call>,
    cache: Vec<Cached>,
    draining: Vec<Draining>,
    /// The open question: the call's token, and the task that asks.
    asking: Option<(u64, AbortHandle)>,
    /// The stop, once the policy, or a call's run, has stopped the turn.
    stopped: Option<Stopped>,
    /// How many calls that may have changed the file system have finished:
    /// what a decision is dated by ([`Dated`]).
    changes: u64,
    next_token: u64,
}

/// The turn was stopped at a call: the policy refused it and said to
/// stop, or its run stopped the turn (its result is marked so,
/// [`ToolResult::stopping_turn`]).
///
/// The turn stops *at that call*, in the model's order, as it would if the
/// calls ran one after another: a call the model wrote before it, and that
/// was allowed, runs; nothing the model wrote after it starts. So what a
/// stopped turn did does not depend on which answer arrived first.
struct Stopped {
    /// The block index of the call: the first one in the model's order,
    /// if the turn was stopped at several.
    at: u32,
    /// Why, from the first stop to arrive.
    reason: String,
    /// Whether the first stop to arrive came from a call's run rather than
    /// from the policy: what a call it keeps from running is told
    /// ([`Round::kept_from_running`]).
    by_a_run: bool,
}

/// The text of the result of a call that never ran because the turn was
/// interrupted.
const NOT_RUN: &str = "The turn was interrupted before this call ran.";

/// The text of the result of a call that never ran because the policy
/// stopped the turn over another call.
const NOT_RUN_STOPPED: &str = "This call did not run: the turn was stopped because another call \
                               was not allowed.";

impl Round {
    pub(crate) fn new(
        env: Arc<RoundEnv>,
        turn: TurnId,
        turn_number: u32,
        number: u64,
        cancel: CancelToken,
        kill: CancelToken,
    ) -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            env,
            turn,
            turn_number,
            number,
            cancel,
            kill,
            tx,
            rx,
            calls: Vec::new(),
            cache: Vec::new(),
            draining: Vec::new(),
            asking: None,
            stopped: None,
            changes: 0,
            next_token: 0,
        }
    }

    /// What a decision made now is made against.
    fn now_dated(&self) -> Dated {
        Dated {
            changes: self.changes,
            narrowed: self.env.policy.narrowed(),
        }
    }

    /// Why the turn was stopped, by the policy or by a call's run, if it
    /// was: the turn then ends with the outcome `denied` once the round is
    /// settled and recorded, and the model is not asked again.
    pub(crate) fn stopped(&self) -> Option<&str> {
        self.stopped.as_ref().map(|stopped| stopped.reason.as_str())
    }

    /// Whether the response that counts has any call.
    pub(crate) fn has_calls(&self) -> bool {
        !self.calls.is_empty()
    }

    /// The tools of the calls running now, in the model's order: what the
    /// round's time is spent on.
    pub(crate) fn running(&self) -> Vec<String> {
        self.calls
            .iter()
            .filter(|call| matches!(call.state, CallState::Running))
            .map(|call| call.name.clone())
            .collect()
    }

    /// Whether a question is open.
    pub(crate) fn is_asking(&self) -> bool {
        self.asking.is_some()
    }

    fn now(&self) -> Duration {
        self.env.clock.monotonic()
    }

    /// Whether every call has its result, no run is being kept and no
    /// discarded run is still to report.
    pub(crate) fn is_settled(&self) -> bool {
        self.cache.is_empty()
            && self.draining.is_empty()
            && self
                .calls
                .iter()
                .all(|call| matches!(call.state, CallState::Done(_)))
    }

    /// The next report from one of the round's tasks.
    pub(crate) async fn recv(&mut self) -> Report {
        match self.rx.recv().await {
            Some(report) => report,
            // The round holds a sender itself, so the channel never closes.
            None => std::future::pending().await,
        }
    }

    /// Waits until the round is settled, taking reports as they come and
    /// starting nothing new.
    pub(crate) async fn settle(&mut self) {
        while !self.is_settled() {
            let report = self.recv().await;
            self.handle(report);
        }
    }

    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    fn run_cx(&self, cancel: CancelToken) -> RunCx {
        let workspace = &self.env.workspace;
        let mut cx = RunCx::new(
            Arc::clone(&workspace.paths),
            Arc::clone(&workspace.content),
            Arc::clone(&workspace.files),
        )
        .with_cancel(cancel)
        .with_kill(self.kill.clone())
        .with_limits(workspace.limits)
        .with_gate(self.env.policy.read_gate())
        .with_turn(self.turn_number)
        .with_round(self.number);
        if let Some(outliner) = &workspace.outliner {
            cx = cx.with_outliner(Arc::clone(outliner));
        }
        if let Some(images) = &workspace.images {
            cx = cx.with_images(images.clone());
        }
        cx
    }

    /// Runs `work` as a task that reports once, whatever happens: if the
    /// work panics, the report is an error result for the call.
    fn spawn(
        &self,
        token: u64,
        name: &str,
        work: impl Future<Output = Report> + Send + 'static,
    ) -> AbortHandle {
        let tx = self.tx.clone();
        let cx = self.run_cx(CancelToken::new());
        let name = name.to_owned();
        tokio::spawn(async move {
            let report = match Guarded(Box::pin(work)).await {
                Ok(report) => report,
                Err(panic) => Report::Finished {
                    token,
                    result: bound(
                        ToolResult::error(format!(
                            "internal error: the call panicked ({panic}). It did not finish."
                        )),
                        &name,
                        &cx,
                    )
                    .await,
                    duration: Duration::ZERO,
                    stat: ToolStatOutcome::Failed,
                    notes: Vec::new(),
                },
            };
            // The round is gone only when the session is: nobody is left
            // to tell.
            let _ = tx.send(report);
        })
        .abort_handle()
    }

    /// Gives a call a result it gets without running, through the output
    /// policy.
    fn settle_with(&self, token: u64, name: &str, result: ToolResult, stat: ToolStatOutcome) {
        let cx = self.run_cx(CancelToken::new());
        let tool = name.to_owned();
        self.spawn(token, name, async move {
            Report::Finished {
                token,
                result: bound(result, &tool, &cx).await,
                duration: Duration::ZERO,
                stat,
                notes: Vec::new(),
            }
        });
    }

    /// A tool call's block ended: the call joins the round and is planned
    /// at once.
    pub(crate) fn add_call(&mut self, ended: EndedCall, ids: &mut IdCounter) {
        let EndedCall {
            index,
            id: call_id,
            name,
            arguments,
        } = ended;
        let arguments = match arguments {
            Ok(arguments) => arguments,
            Err(problem) => {
                let call = self.new_call(index, call_id, &name, Json::Null, String::new(), ids);
                self.settle_with(
                    call.token,
                    &name,
                    ToolResult::error(format!(
                        "The arguments of this call were not valid JSON ({problem}). Send the \
                         arguments as one JSON object that fits the tool's schema."
                    )),
                    ToolStatOutcome::InvalidArguments,
                );
                self.insert(call);
                return;
            }
        };

        let key = format!("{name}\u{0}{arguments}");
        let mut call = self.new_call(index, call_id, &name, arguments, key, ids);
        match self.env.tools.get(&name) {
            Some(tool) => {
                call.decided = self.now_dated();
                self.plan(&call, &tool, Observers::Run);
                call.tool = Some(tool);
            }
            None => {
                let known: Vec<&str> = self
                    .env
                    .tools
                    .specs()
                    .iter()
                    .map(|spec| spec.name.as_str())
                    .collect();
                self.settle_with(
                    call.token,
                    &name,
                    ToolResult::error(format!(
                        "There is no tool `{name}`. The tools are: {}.",
                        known.join(", ")
                    )),
                    ToolStatOutcome::UnknownTool,
                );
            }
        }
        self.insert(call);
    }

    /// Puts a call among the others at the place of its block: the calls
    /// are always in the model's order, whatever order their blocks ended
    /// in.
    fn insert(&mut self, call: Call) {
        let position = self
            .calls
            .partition_point(|other| other.index <= call.index);
        self.calls.insert(position, call);
    }

    fn new_call(
        &mut self,
        index: u32,
        call_id: CallId,
        name: &str,
        arguments: Json,
        key: String,
        ids: &mut IdCounter,
    ) -> Call {
        Call {
            token: self.token(),
            index,
            call_id,
            item_id: ids.next_id(),
            name: name.to_owned(),
            arguments,
            key,
            tool: None,
            footprint: None,
            state: CallState::Pending,
            cancel: self.cancel.child(),
            shown: false,
            ran: false,
            times: CallTimes::default(),
            decided: Dated::default(),
            replanned: None,
            moved: false,
            hooked: None,
            answered: false,
        }
    }

    /// Whether the call at `position` was decided before something that
    /// could change its answer: a call that may have changed the file
    /// system finished, or the policy's rules narrowed. `narrowed` is the
    /// policy's count now.
    fn is_stale(&self, position: usize, narrowed: u64) -> bool {
        self.calls[position].decided
            != Dated {
                changes: self.changes,
                narrowed,
            }
    }

    /// Plans the call at `position` again and shows the new plan to the
    /// policy: it was decided before something that could change the
    /// answer. Until the report comes it is a call whose plan is not known
    /// yet, so nothing after it that could conflict with it starts, and no
    /// question after it is put. The report is taken as the first was
    /// ([`planned`](Self::planned)), and its decision acted on.
    fn decide_again(&mut self, position: usize, narrowed: u64) {
        let changes = self.changes;
        let call = &mut self.calls[position];
        call.decided = Dated { changes, narrowed };
        // A call that was planned has its tool.
        let Some(tool) = call.tool.clone() else {
            return;
        };
        call.state = CallState::Pending;
        call.replanned = call.footprint.take().map(|footprint| footprint.paths);
        // Its hooks are shown it again, with the model's own arguments, and
        // what they said before is dropped, the answer to it with it.
        call.answered = false;
        if call.hooked.take().is_some() {
            call.key = format!("{}\u{0}{}", call.name, call.arguments);
        }
        let call = &self.calls[position];
        self.plan(call, &tool, Observers::Skip);
    }

    /// The call at `position` was planned again onto other paths, and a
    /// call the model wrote after it has already used one of them: run
    /// now, it would act after that call, against the model's order.
    fn moved_under_a_later_call(&mut self, position: usize) {
        let call = &mut self.calls[position];
        call.state = CallState::Pending;
        let (token, name) = (call.token, call.name.clone());
        self.settle_with(
            token,
            &name,
            ToolResult::error(
                "This call did not run: since it was planned, a path it names has come to lead \
                 somewhere else, which a call written after it has already used. Nothing was \
                 changed. Make the call again if it is still wanted.",
            ),
            ToolStatOutcome::Failed,
        );
    }

    /// Starts the task that plans a call, shows the plan to the policy, and
    /// shows what the policy did not refuse to the call's hooks: its
    /// observers too at its first decision, and not when it is decided
    /// again.
    fn plan(&self, call: &Call, tool: &Arc<dyn Tool>, observers: Observers) {
        let planning = Planning {
            observers,
            token: call.token,
            tool: Arc::clone(tool),
            policy: Arc::clone(&self.env.policy),
            hooks: self.env.hooks.clone(),
            session: self.env.session.clone(),
            turn: self.turn.clone(),
            call_id: call.call_id.clone(),
            item: call.item_id.clone(),
            arguments: call.arguments.clone(),
            name: call.name.clone(),
            root: self.env.workspace.paths.root().clone(),
            cx: self.run_cx(call.cancel.clone()),
        };
        self.spawn(call.token, &call.name, planning.run());
    }

    /// The call at `position` may start, and a run of the same call was kept
    /// from the response before this one: that run, or its result, is used
    /// in place of a new one.
    ///
    /// This happens when the call's turn to start has come, and not when its
    /// block ends, so a kept run is held back by the same rules as a new
    /// one: a read the model placed after a write is not answered with what
    /// was read before the write.
    fn adopt(&mut self, position: usize, kept: usize) {
        let cached = self.cache.remove(kept);
        let call = &mut self.calls[position];
        // The run reports under its own token and writes its output under
        // its own item, so the call takes both over.
        call.token = cached.token;
        call.item_id = cached.item_id;
        call.cancel = cached.cancel;
        call.footprint = Some(cached.footprint);
        call.shown = true;
        call.ran = true;
        // A kept run waited for nothing of this response.
        call.times.ready = None;
        call.state = match cached.outcome {
            Some(mut outcome) => {
                outcome.item.call_id = call.call_id.clone();
                CallState::Done(Box::new(outcome))
            }
            None => CallState::Running,
        };
        let call = &self.calls[position];
        self.emit_started(call);
        if let CallState::Done(outcome) = &call.state {
            self.emit_completed(&outcome.item);
            // A kept run that stopped the turn stops it when it is used.
            if let Some(reason) = outcome.turn_stop.clone() {
                let index = call.index;
                self.stop_at(index, reason, true);
            }
        }
    }

    /// Takes one report and changes the round by it. Nothing is started
    /// here: [`pump`](Self::pump) does that.
    pub(crate) fn handle(&mut self, report: Report) {
        match report {
            Report::Planned {
                token,
                plan,
                decision,
                hooked,
            } => self.planned(token, plan, decision, hooked),
            Report::Answered {
                token,
                plan,
                verdict,
            } => self.answered(token, plan, verdict),
            Report::Finished {
                token,
                result,
                duration,
                stat,
                notes,
            } => self.finished(token, result, duration, stat, notes),
        }
    }

    fn position(&self, token: u64) -> Option<usize> {
        self.calls.iter().position(|call| call.token == token)
    }

    fn planned(
        &mut self,
        token: u64,
        mut plan: Plan,
        decision: Decision,
        hooked: Option<Box<HookQuestion>>,
    ) {
        // A call of a response that was thrown away meanwhile: nothing to do.
        let Some(position) = self.position(token) else {
            return;
        };
        let only_reads = matches!(decision, Decision::AllowReadOnly);
        if only_reads {
            // The policy has analysed the call and knows it changes
            // nothing: it runs beside others. Its effects stay as declared,
            // so a call with an `exec` effect still waits for the response
            // to end.
            plan.concurrency = Concurrency::ReadOnly;
        }
        let call = &mut self.calls[position];
        if !matches!(call.state, CallState::Pending) || call.footprint.is_some() {
            // It already has its plan, or its result.
            return;
        }
        let mut footprint = Footprint::of(&plan);
        footprint.may_change &= !only_reads;
        if let Some(earlier) = call.replanned.take() {
            call.moved |= earlier != footprint.paths;
        }
        // A call that runs with a hook's arguments is not the call the model
        // wrote: a kept run of the model's own is not it, nor the other way.
        if let Some(replaced) = hooked.as_ref().and_then(|hooked| hooked.replaced.as_ref()) {
            call.key = format!(
                "{}\u{0}{}\u{0}{}",
                call.name, call.arguments, replaced.arguments
            );
        }
        call.hooked = hooked;
        let call = &self.calls[position];
        // A kept run of another call that touched the same files is not what
        // the model was shown: it goes before this call can meet what it
        // left behind. A kept run of this same call stays, to be used when
        // the call's turn comes; unless this plan names other paths than
        // the run's did (a link was made since the run was planned, by a
        // call of this response): the run is of where the old ones led, so
        // it goes, and the call runs again. It is held against the run's
        // own plan, not the call's first: the call may have been planned
        // first after the link was made, and its second plan then names
        // what its first did.
        let key = call.key.clone();
        self.discard_where(|cached| {
            if cached.key == key {
                cached.footprint.paths != footprint.paths
            } else {
                overlap(&cached.footprint.paths, &footprint.paths)
            }
        });
        self.calls[position].footprint = Some(footprint);
        if self.cancel.is_cancelled() {
            self.not_run(position, NOT_RUN);
            return;
        }
        let index = self.calls[position].index;
        if self
            .stopped
            .as_ref()
            .is_some_and(|stopped| index > stopped.at)
        {
            // The model wrote this call after the one the turn stopped at.
            self.kept_from_running(position);
            return;
        }
        // A hook that asked, or replaced the arguments, puts the question
        // whatever the policy said: a hook only narrows.
        let hooks_ask = self.calls[position].hooked.is_some();
        match decision {
            Decision::Allow | Decision::AllowReadOnly if !hooks_ask => {
                let now = self.now();
                let call = &mut self.calls[position];
                call.state = CallState::Ready(plan);
                // A call decided again was allowed when it was first.
                call.times.ready.get_or_insert(now);
            }
            Decision::Allow | Decision::AllowReadOnly | Decision::Ask => {
                self.calls[position].state = CallState::NeedsAnswer(plan);
            }
            Decision::Deny { reason } => {
                let mut text = "This call was not allowed to run.".to_owned();
                if let Some(reason) = reason {
                    text.push_str(" Reason: ");
                    text.push_str(&reason);
                }
                self.deny(position, text);
            }
            Decision::Stop { reason } => self.stop(position, reason),
        }
    }

    /// The policy refused the call at `position` and said to stop the turn.
    ///
    /// - The call gets its denial.
    /// - A call the model wrote after it that has not started gets a result
    ///   that says it did not run, and none of those starts from here on.
    /// - A call the model wrote before it, and that was allowed, runs as it
    ///   would have: it was allowed, and its place in the response is before
    ///   the stop. A call that is running finishes.
    /// - The open question is withdrawn, and no other is put: a call that
    ///   needs an answer does not run, wherever it stands
    ///   ([`ask_next`](Self::ask_next) gives it its result).
    fn stop(&mut self, position: usize, reason: String) {
        let at = self.calls[position].index;
        self.deny(
            position,
            format!("This call was not allowed to run, and the turn was stopped. Reason: {reason}"),
        );
        self.stop_at(at, reason, false);
    }

    /// The turn stops at the call of block `at`: the policy refused it
    /// ([`stop`](Self::stop)), or its run stopped the turn and the call has
    /// its own result ([`finished`](Self::finished)). The rest is the same
    /// for both: what the model wrote after it does not start, and the open
    /// question is withdrawn.
    fn stop_at(&mut self, at: u32, reason: String, by_a_run: bool) {
        let at = match &mut self.stopped {
            Some(stopped) => {
                stopped.at = stopped.at.min(at);
                stopped.at
            }
            None => {
                self.stopped = Some(Stopped {
                    at,
                    reason,
                    by_a_run,
                });
                at
            }
        };
        if let Some((token, question)) = self.asking.take() {
            question.abort();
            if let Some(asked) = self.position(token) {
                self.kept_from_running(asked);
            }
        }
        for other in 0..self.calls.len() {
            if matches!(self.calls[other].state, CallState::Ready(_))
                && self.calls[other].index > at
            {
                self.kept_from_running(other);
            }
        }
    }

    /// Gives the call at `position` its result for not running because the
    /// turn was stopped. After the policy's stop of another call, it says
    /// it did not run. After a call's run stopped the turn, it is the
    /// denial the policy gives every call of a stopped turn when it decides
    /// it again: the same result whichever of the two reaches the call
    /// first.
    fn kept_from_running(&mut self, position: usize) {
        match &self.stopped {
            Some(stopped) if stopped.by_a_run => {
                let text = format!(
                    "This call was not allowed to run, and the turn was stopped. Reason: {}",
                    stopped.reason
                );
                self.deny(position, text);
            }
            _ => self.not_run(position, NOT_RUN_STOPPED),
        }
    }

    fn answered(&mut self, token: u64, plan: Plan, verdict: Verdict) {
        if self
            .asking
            .as_ref()
            .is_some_and(|(asked, _)| *asked == token)
        {
            self.asking = None;
        }
        let Some(position) = self.position(token) else {
            return;
        };
        let now = self.now();
        let call = &mut self.calls[position];
        if let Some(asked) = call.times.asked.take() {
            call.times.approval += now.saturating_sub(asked);
        }
        if !matches!(self.calls[position].state, CallState::Asking) {
            // The question was withdrawn while the answer was on its way,
            // and the call has been given its result already.
            return;
        }
        if self.cancel.is_cancelled() {
            self.not_run(position, NOT_RUN);
            return;
        }
        // Somebody answered it: its result may say so.
        self.calls[position].answered = true;
        match verdict {
            Verdict::Allow => {
                let call = &mut self.calls[position];
                call.state = CallState::Ready(plan);
                call.times.ready = Some(now);
            }
            Verdict::Deny { note } => {
                let mut text = "The user did not allow this call.".to_owned();
                if let Some(note) = note {
                    text.push_str(" They wrote: ");
                    text.push_str(&note);
                }
                self.deny(position, text);
            }
            Verdict::Stop { reason } => self.stop(position, reason),
        }
    }

    fn deny(&mut self, position: usize, text: String) {
        let call = &mut self.calls[position];
        call.state = CallState::Pending;
        let (token, name) = (call.token, call.name.clone());
        self.settle_with(
            token,
            &name,
            ToolResult::denied(text),
            ToolStatOutcome::Denied,
        );
    }

    fn not_run(&mut self, position: usize, why: &str) {
        let call = &mut self.calls[position];
        call.state = CallState::Pending;
        let (token, name) = (call.token, call.name.clone());
        self.settle_with(
            token,
            &name,
            ToolResult::interrupted(why),
            ToolStatOutcome::Interrupted,
        );
    }

    fn finished(
        &mut self,
        token: u64,
        result: ToolResult,
        duration: Duration,
        stat: ToolStatOutcome,
        notes: Vec<Part>,
    ) {
        debug_assert!(result.is_bounded(), "every result passes the output policy");
        let turn_stop = result.turn_stop().map(str::to_owned);
        if self
            .asking
            .as_ref()
            .is_some_and(|(asked, _)| *asked == token)
        {
            // The task that asked ended without an answer: it panicked.
            // The call gets the error, and the next question may be put.
            self.asking = None;
        }
        if let Some(position) = self.position(token) {
            let call = &self.calls[position];
            if matches!(call.state, CallState::Done(_)) {
                debug_assert!(false, "a call has exactly one result");
                return;
            }
            // A call that ran and may have changed the file system: every
            // call decided before now is decided again when its turn comes.
            if call.ran
                && call
                    .footprint
                    .as_ref()
                    .is_some_and(|footprint| footprint.may_change)
            {
                self.changes += 1;
            }
            let mut item = self.item(
                &call.item_id,
                &call.call_id,
                &call.name,
                result,
                duration,
                call.replaced_and_asked(),
            );
            // What its `tool.after` hooks added, after the tool's own.
            item.content.extend(notes);
            if !call.shown {
                self.emit_started(call);
            }
            self.emit_completed(&item);
            let now = self.now();
            let call = &mut self.calls[position];
            call.shown = true;
            // A question withdrawn without an answer (an interrupt, a stop)
            // was waited on until now.
            if let Some(asked) = call.times.asked.take() {
                call.times.approval += now.saturating_sub(asked);
            }
            let (queued, approval) = (call.times.queued(), call.times.approval);
            let index = call.index;
            call.state = CallState::Done(Box::new(Outcome {
                item,
                stat,
                duration,
                queued,
                approval,
                turn_stop: turn_stop.clone(),
            }));
            // Its run stopped the turn: the round ends here, as at a call
            // the policy stopped, and the model is not asked again.
            if let Some(reason) = turn_stop {
                self.stop_at(index, reason, true);
            }
        } else if let Some(position) = self.cache.iter().position(|cached| cached.token == token) {
            // A kept run finished. Nobody is shown it unless the next
            // response asks for the same call, whose id then replaces the
            // empty one here.
            let cached = &self.cache[position];
            let mut item = self.item(
                &cached.item_id,
                &CallId::new(""),
                &cached.name,
                result,
                duration,
                cached.replaced.as_ref(),
            );
            item.content.extend(notes);
            self.cache[position].outcome = Some(Outcome {
                item,
                stat,
                duration,
                queued: Duration::ZERO,
                approval: Duration::ZERO,
                turn_stop,
            });
        } else if let Some(position) = self.draining.iter().position(|run| run.token == token) {
            let run = self.draining.swap_remove(position);
            self.forget(&run.reads);
        }
        // Anything else is a report for a call that no longer exists.
    }

    /// A call's result as the item front ends see and the log keeps. A call
    /// whose arguments a hook replaced says so: its result begins with the
    /// line that tells the model which hook put what in their place, and
    /// the item names the hook and the arguments.
    fn item(
        &self,
        item_id: &ItemId,
        call_id: &CallId,
        name: &str,
        result: ToolResult,
        duration: Duration,
        replaced: Option<&ReplacedArguments>,
    ) -> ToolResultItem {
        let (outcome, mut content, payload, artifacts) = result.into_parts();
        if let Some(replaced) = replaced {
            content.insert(0, Part::text(hooks::replaced_note(replaced)));
        }
        ToolResultItem {
            id: item_id.clone(),
            call_id: call_id.clone(),
            tool: name.to_owned(),
            outcome,
            content,
            payload,
            artifacts,
            duration_ms: Some(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
            created_at: Some(self.env.clock.now().to_timestamp()),
            // The model's own arguments are in its call.
            replaced: replaced.map(|replaced| {
                Box::new(ReplacedArguments {
                    hook: replaced.hook.clone(),
                    arguments: replaced.arguments.clone(),
                    original: None,
                    reason: replaced.reason.clone(),
                })
            }),
        }
    }

    /// Starts whatever may start now.
    pub(crate) fn pump(&mut self, stage: Stage) {
        if self.cancel.is_cancelled() {
            return;
        }
        let (streaming, open_call) = match stage {
            Stage::Streaming { open_call } => (true, open_call),
            Stage::Ended => {
                // No more calls are coming: a kept run that none of the
                // calls still to start asks for was never shown to the
                // model.
                self.discard_unclaimed();
                (false, None)
            }
        };
        // A call that has not started, decided before something that could
        // change its answer, is decided again now, whatever its turn.
        let narrowed = self.env.policy.narrowed();
        for position in 0..self.calls.len() {
            if matches!(
                self.calls[position].state,
                CallState::Ready(_) | CallState::NeedsAnswer(_)
            ) && self.is_stale(position, narrowed)
            {
                self.decide_again(position, narrowed);
            }
        }
        self.ask_next(open_call);

        let stop_may_answer = self.env.policy.answers_may_stop();
        let mut running = self
            .calls
            .iter()
            .filter(|call| matches!(call.state, CallState::Running))
            .count();
        // A call that may change the file system is running: when it ends,
        // every call that has not started is decided again. A call started
        // below counts from then on (found by `stress`).
        let mut changing = self.calls.iter().any(|call| {
            matches!(call.state, CallState::Running)
                && call
                    .footprint
                    .as_ref()
                    .is_some_and(|footprint| footprint.may_change)
        });
        for position in 0..self.calls.len() {
            if running >= self.env.max_parallel {
                break;
            }
            let call = &self.calls[position];
            if open_call.is_some_and(|open| call.index > open) {
                // An earlier call is still being written.
                break;
            }
            let (CallState::Ready(_), Some(footprint)) = (&call.state, &call.footprint) else {
                continue;
            };
            if streaming && footprint.side_effects {
                continue;
            }
            let waits_for_earlier = self.calls[..position].iter().any(|earlier| {
                if matches!(earlier.state, CallState::Done(_)) {
                    return false;
                }
                // An earlier call whose question is open, or still to be
                // put, may be answered "stop the turn here": then this
                // call, written after it, must not have changed anything.
                if footprint.side_effects
                    && stop_may_answer
                    && matches!(earlier.state, CallState::NeedsAnswer(_) | CallState::Asking)
                {
                    return true;
                }
                // So may an earlier call that has not started, once a
                // running call that may change the file system ends and it
                // is decided again.
                if footprint.side_effects
                    && stop_may_answer
                    && changing
                    && matches!(earlier.state, CallState::Ready(_))
                {
                    return true;
                }
                // An earlier call that is not planned yet could turn out to
                // conflict.
                earlier
                    .footprint
                    .as_ref()
                    .is_none_or(|other| other.shadow.conflicts_with(&footprint.shadow))
            });
            // A call decided again may be scheduled otherwise than it was:
            // it waits for a call it now conflicts with that is running, a
            // later one too. In the ordinary order no later call that
            // conflicts with an earlier one has started.
            let waits_for_running = self.calls[position + 1..].iter().any(|later| {
                matches!(later.state, CallState::Running)
                    && later
                        .footprint
                        .as_ref()
                        .is_some_and(|other| other.shadow.conflicts_with(&footprint.shadow))
            });
            let waits_for_discarded = self
                .draining
                .iter()
                .any(|run| overlap(&run.paths, &footprint.paths));
            if waits_for_earlier || waits_for_running || waits_for_discarded {
                continue;
            }
            let used_later = call.moved
                && self.calls[position + 1..].iter().any(|later| {
                    later.ran
                        && later
                            .footprint
                            .as_ref()
                            .is_some_and(|other| other.shadow.conflicts_with(&footprint.shadow))
                });
            if used_later {
                self.moved_under_a_later_call(position);
                continue;
            }
            changing |= footprint.may_change;
            self.start(position);
            running += 1;
        }
    }

    /// Puts the next question, if none is open: the first call in order
    /// that needs an answer, once every call before it has been planned.
    ///
    /// Once the turn is stopped nobody is asked anything: a call that needs
    /// an answer gets a result that says it did not run. This is the one
    /// place that happens, whether the call was waiting its turn when the
    /// stop came or was planned after it.
    ///
    /// A call decided before something that could change its answer has
    /// been decided again by then ([`pump`](Self::pump)): the question is
    /// about the call as it is now.
    fn ask_next(&mut self, open_call: Option<u32>) {
        if self.asking.is_some() {
            return;
        }
        for position in 0..self.calls.len() {
            if open_call.is_some_and(|open| self.calls[position].index > open) {
                return;
            }
            match &self.calls[position].state {
                CallState::Pending if self.calls[position].footprint.is_none() => return,
                CallState::NeedsAnswer(_) if self.stopped.is_some() => {
                    self.kept_from_running(position);
                }
                CallState::NeedsAnswer(_) => {
                    self.ask(position);
                    return;
                }
                _ => {}
            }
        }
    }

    fn ask(&mut self, position: usize) {
        let call = &mut self.calls[position];
        let CallState::NeedsAnswer(plan) = std::mem::replace(&mut call.state, CallState::Asking)
        else {
            return;
        };
        let Some(tool) = call.tool.clone() else {
            // A call that was planned has its tool.
            call.state = CallState::Ready(plan);
            return;
        };
        call.times.asked = Some(self.env.clock.monotonic());
        let token = call.token;
        let policy = Arc::clone(&self.env.policy);
        let session = self.env.session.clone();
        let turn = self.turn.clone();
        let call_id = call.call_id.clone();
        let item = call.item_id.clone();
        // A call whose arguments a hook replaced is asked about with the
        // hook's: they are what would run, and its plan is theirs.
        let arguments = call.replaced().map_or_else(
            || call.arguments.clone(),
            |replaced| replaced.arguments.clone(),
        );
        let hooked = call.hooked.clone();
        let name = call.name.clone();
        let cancel = call.cancel.clone();
        let cx = self.run_cx(cancel);
        // What the call would change in a file, for the question: worked
        // out by its tool as it would make the change.
        let preview = tool.preview(&plan, &cx);
        let handle = self.spawn(token, &name, async move {
            // It reads the file, which blocks.
            let preview = match preview {
                Some(work) => run_blocking(&cx.cancel, move |_| work()).await.ok(),
                None => None,
            };
            let verdict = policy
                .ask(PlannedCall {
                    session: &session,
                    turn: &turn,
                    call_id: &call_id,
                    item: &item,
                    spec: tool.spec(),
                    arguments: &arguments,
                    plan: &plan,
                    preview: preview.as_ref(),
                    hooks: hooked.as_deref(),
                })
                .await;
            Report::Answered {
                token,
                plan,
                verdict,
            }
        });
        self.asking = Some((token, handle));
    }

    fn start(&mut self, position: usize) {
        let key = &self.calls[position].key;
        if let Some(kept) = self.cache.iter().position(|cached| cached.key == *key) {
            self.adopt(position, kept);
            return;
        }
        let call = &mut self.calls[position];
        let CallState::Ready(plan) = std::mem::replace(&mut call.state, CallState::Running) else {
            return;
        };
        let Some(tool) = call.tool.clone() else {
            call.state = CallState::Ready(plan);
            return;
        };
        call.ran = true;
        call.shown = true;
        call.times.started = Some(self.env.clock.monotonic());
        let token = call.token;
        let name = call.name.clone();
        let call = &self.calls[position];
        self.emit_started(call);

        // The run names its call, for a tool that puts questions on its
        // own account (an extension's host calls). No built-in tool reads
        // it, so for them the run is as it was.
        let cx = self.run_cx(call.cancel.clone()).with_call(CallRef {
            session: self.env.session.clone(),
            turn: self.turn.clone(),
            call_id: call.call_id.clone(),
            item: call.item_id.clone(),
        });
        let clock = Arc::clone(&self.env.clock);
        let mut out = OutputDeltas {
            emitter: Arc::clone(&self.env.emitter),
            turn: self.turn.clone(),
            item_id: call.item_id.clone(),
        };
        // Its `tool.after` hooks, when any is on it: asked once it has run,
        // before its result is recorded. With none, nothing is kept for
        // them.
        let after = self
            .env
            .hooks
            .as_ref()
            .filter(|set| {
                set.hooks.wants(
                    HookEvent::ToolAfter,
                    Some(ToolTarget::of(&call.name, &call.arguments)),
                )
            })
            .map(|set| After {
                set: set.clone(),
                emitter: Arc::clone(&self.env.emitter),
                session: self.env.session.clone(),
                turn: self.turn.clone(),
                root: self.env.workspace.paths.root().clone(),
                tool: call.name.clone(),
                call_id: call.call_id.clone(),
                // What it runs with: a hook's arguments, if they were
                // allowed.
                arguments: call.replaced().map_or_else(
                    || call.arguments.clone(),
                    |replaced| replaced.arguments.clone(),
                ),
                model_arguments: call.arguments.clone(),
                summary: plan.summary.clone(),
                effects: plan.effects.clone(),
            });
        // One call's run: its parent is the span the loop is in, the
        // turn's or the request's (`docs/architecture/02-runtime.md`,
        // "Spans").
        let span = tracing::info_span!("tool", tool = %name, call = %call.call_id);
        self.spawn(
            token,
            &name,
            tracing::Instrument::instrument(
                async move {
                    cineris_base::hook::await_point!(BEFORE_A_CALL_RUNS);
                    let started = clock.monotonic();
                    let result = execute(tool.as_ref(), plan, &cx, &mut out).await;
                    let duration = clock.monotonic().saturating_sub(started);
                    let stat = match result.outcome() {
                        ToolOutcome::Success => ToolStatOutcome::Ok,
                        ToolOutcome::Denied => ToolStatOutcome::Denied,
                        ToolOutcome::Interrupted => ToolStatOutcome::Interrupted,
                        _ => ToolStatOutcome::Failed,
                    };
                    let notes = match after {
                        Some(after) => after.notes(&result, &cx.cancel).await,
                        None => Vec::new(),
                    };
                    Report::Finished {
                        token,
                        result,
                        duration,
                        stat,
                        notes,
                    }
                },
                span,
            ),
        );
    }

    /// The response these calls came from no longer counts: it failed and
    /// is being sent again. Runs that started are kept for the next
    /// response; everything else is dropped.
    pub(crate) fn void(&mut self) {
        if let Some((_, question)) = self.asking.take() {
            // The question is withdrawn.
            question.abort();
        }
        // The call the policy stopped the turn over is void with the rest.
        // If the next response makes it again, the policy is shown it again.
        //
        // A stop from a call's run goes too. A kept run that stopped the
        // turn stops it again when the next response uses it
        // ([`adopt`](Self::adopt)); one it does not use is discarded, and
        // its stop with it. Neither happens today: both stops that mark a
        // result (a confined command, an extension's call) have side
        // effects, and such a call never starts while a response streams,
        // so only a run of a call with no side effects is ever kept. A tool
        // with no side effects that marks a stop would need this looked at
        // again.
        self.stopped = None;
        for call in std::mem::take(&mut self.calls) {
            if call.shown {
                self.env.emitter.emit(
                    Some(&self.turn),
                    Event::ItemReset(ItemRef {
                        item_id: call.item_id.clone(),
                    }),
                );
            }
            let outcome = match call.state {
                CallState::Running => None,
                CallState::Done(outcome) if call.ran => Some(*outcome),
                _ => {
                    // Not started: nothing to keep. A plan in progress stops.
                    call.cancel.cancel();
                    continue;
                }
            };
            // Only calls without side effects start before a response has
            // ended, so only those are ever kept.
            if let Some(footprint) = call.footprint {
                self.cache.push(Cached {
                    token: call.token,
                    name: call.name,
                    key: call.key,
                    item_id: call.item_id,
                    footprint,
                    cancel: call.cancel,
                    outcome,
                    replaced: call
                        .hooked
                        .and_then(|hooked| hooked.replaced)
                        .filter(|_| call.answered),
                });
            }
        }
    }

    /// Discards every kept run: the request is over, and whatever the last
    /// response did not ask for again was never shown to the model.
    pub(crate) fn discard_kept(&mut self) {
        self.discard_where(|_| true);
    }

    /// Discards the kept runs that no call still to start asks for. A call
    /// that has not started yet takes a kept run of the same call when its
    /// turn comes, so those stay until then.
    fn discard_unclaimed(&mut self) {
        let claimed: Vec<String> = self
            .calls
            .iter()
            .filter(|call| !matches!(call.state, CallState::Running | CallState::Done(_)))
            .map(|call| call.key.clone())
            .collect();
        self.discard_where(|cached| !claimed.contains(&cached.key));
    }

    fn discard_where(&mut self, discard: impl Fn(&Cached) -> bool) {
        let (gone, kept): (Vec<Cached>, Vec<Cached>) = std::mem::take(&mut self.cache)
            .into_iter()
            .partition(discard);
        self.cache = kept;
        for cached in gone {
            if cached.outcome.is_some() {
                self.forget(&cached.footprint.reads);
            } else {
                // Still running. It is told to stop, and may write to the
                // table of seen files until it has: the files are forgotten
                // when it reports.
                cached.cancel.cancel();
                self.draining.push(Draining {
                    token: cached.token,
                    paths: cached.footprint.paths,
                    reads: cached.footprint.reads,
                });
            }
        }
    }

    /// Makes the table of seen files forget what a discarded run recorded:
    /// the model was never shown it, so a later read must not be answered
    /// with "unchanged".
    fn forget(&self, reads: &[AbsPath]) {
        for path in reads {
            self.env.workspace.files.forget(path);
        }
    }

    /// The turn was interrupted. Calls that are running have been told to
    /// stop through their tokens and will report. Every call that has not
    /// started gets its result now: "interrupted", or for the one whose
    /// question was open, a denial.
    pub(crate) fn interrupt(&mut self) {
        if let Some((token, question)) = self.asking.take() {
            question.abort();
            if let Some(position) = self.position(token) {
                self.deny(
                    position,
                    "The turn was interrupted before anyone answered whether this call may run."
                        .to_owned(),
                );
            }
        }
        for position in 0..self.calls.len() {
            if matches!(
                self.calls[position].state,
                CallState::NeedsAnswer(_) | CallState::Ready(_)
            ) {
                self.not_run(position, NOT_RUN);
            }
        }
    }

    /// The results, in the model's call order. The round must be settled.
    pub(crate) fn finish(&mut self) -> Vec<Finished> {
        let root = self.env.workspace.paths.root().clone();
        std::mem::take(&mut self.calls)
            .into_iter()
            .filter_map(|call| {
                let CallState::Done(outcome) = call.state else {
                    debug_assert!(false, "every call of a settled round has its result");
                    return None;
                };
                let outcome = *outcome;
                // What the call changed, from its result: a command's
                // declared write of the whole workspace is not a change,
                // and a write that found its content there made none
                // (`cineris_tools::changed_by`, step 19).
                let written = match &call.footprint {
                    Some(footprint) => cineris_tools::changed_by(
                        &footprint.shadow.effects,
                        &outcome.item.outcome,
                        outcome.item.payload.as_ref(),
                        &root,
                    ),
                    None => Vec::new(),
                };
                let signature = format!(
                    "{}\u{0}{}\u{0}{}\u{0}{}\u{0}",
                    call.name,
                    call.key,
                    outcome.item.outcome.as_str(),
                    text_of(&outcome.item.content)
                );
                Some(Finished {
                    item: outcome.item,
                    stat: outcome.stat,
                    duration: outcome.duration,
                    queued: outcome.queued,
                    approval: outcome.approval,
                    written,
                    signature,
                })
            })
            .collect()
    }

    fn emit_started(&self, call: &Call) {
        self.env.emitter.emit(
            Some(&self.turn),
            Event::ItemStarted(ItemStarted {
                item_id: call.item_id.clone(),
                kind: ItemKind::ToolResult,
                tool: Some(call.name.clone()),
                call_id: Some(call.call_id.clone()),
            }),
        );
    }

    fn emit_completed(&self, item: &ToolResultItem) {
        self.env.emitter.emit(
            Some(&self.turn),
            Event::ItemCompleted(ItemCompleted {
                item: Item::ToolResult(item.clone()),
            }),
        );
    }
}

impl Drop for Round {
    fn drop(&mut self) {
        // A round is settled before it is dropped. If the loop itself is
        // torn down part-way, nothing it started is left running.
        if let Some((_, question)) = self.asking.take() {
            question.abort();
        }
        for call in &self.calls {
            call.cancel.cancel();
        }
        for cached in &self.cache {
            cached.cancel.cancel();
        }
    }
}

/// A call's planning, as the task that does it sees it: the plan, the
/// policy's decision, and what the call's `tool.before` hooks say of what
/// the policy did not refuse.
struct Planning {
    /// Whether its observers are run: at a call's first decision only.
    observers: Observers,
    token: u64,
    tool: Arc<dyn Tool>,
    policy: Arc<dyn Policy>,
    hooks: Option<LoopHooks>,
    session: SessionId,
    turn: TurnId,
    call_id: CallId,
    item: ItemId,
    /// The arguments, as the model wrote them.
    arguments: Json,
    name: String,
    /// The workspace's root, for the hooks' input.
    root: AbsPath,
    cx: RunCx,
}

/// A call's `tool.after` hooks, as the task that ran it asks them.
struct After {
    set: LoopHooks,
    emitter: Arc<Emitter>,
    session: SessionId,
    turn: TurnId,
    root: AbsPath,
    tool: String,
    call_id: CallId,
    /// What it ran with.
    arguments: Json,
    /// What the model wrote: what the hooks' filters were matched by.
    model_arguments: Json,
    summary: String,
    effects: Vec<Effect>,
}

impl After {
    /// Shows the call's result to its hooks, and returns the notes they add
    /// to it, each under a line that names the hook. A hook that fails is
    /// shown to the user and adds nothing; an interrupt meanwhile leaves the
    /// result as it is.
    async fn notes(self, result: &ToolResult, cancel: &CancelToken) -> Vec<Part> {
        let target = ToolTarget::of(&self.tool, &self.model_arguments);
        let matched = self.set.hooks.matching(HookEvent::ToolAfter, Some(target));
        let input = HookInput {
            event: HookEvent::ToolAfter,
            session: self.session.clone(),
            turn: self.turn.clone(),
            workspace: self.root.clone(),
            subject: HookSubject::Ran {
                tool: self.tool.clone(),
                call_id: self.call_id.clone(),
                arguments: self.arguments.clone(),
                summary: self.summary.clone(),
                effects: self.effects.clone(),
                outcome: result.outcome().clone(),
                result: result.text(),
            },
        };
        let asked = hooks::ask_all(&self.set, matched, input, Observers::Run);
        let Some(heard) = cancel.race(asked).await else {
            return Vec::new();
        };
        let added = hooks::added(HookEvent::ToolAfter, heard);
        for failed in added.failed {
            self.emitter.emit(
                Some(&self.turn),
                Event::Notice(Notice::Info(InfoNotice { message: failed })),
            );
        }
        added
            .texts
            .iter()
            .map(|(hook, text)| Part::text(hooks::result_note(hook, text)))
            .collect()
    }
}

/// What became of a replacement of a call's arguments on the blocking pool:
/// the replacement and its plan, the policy's decision of it, and whether
/// it may be put to the user.
type Replanned = (Json, Plan, Decision, Result<(), String>);

impl Planning {
    async fn run(self) -> Report {
        let planner = Arc::clone(&self.tool);
        let plan_cx = self.cx.plan_cx();
        let arguments = self.arguments.clone();
        // Planning may ask the file system where a path leads, which
        // blocks.
        let planned =
            run_blocking(&self.cx.cancel, move |_| planner.plan(&arguments, &plan_cx)).await;
        let (result, stat) = match planned {
            Ok(Ok(plan)) => {
                let decision = self.policy.decide(&PlannedCall {
                    session: &self.session,
                    turn: &self.turn,
                    call_id: &self.call_id,
                    item: &self.item,
                    spec: self.tool.spec(),
                    arguments: &self.arguments,
                    plan: &plan,
                    preview: None,
                    hooks: None,
                });
                cineris_base::hook::await_point!(A_CALL_IS_DECIDED);
                return self.hooked(plan, decision).await;
            }
            Ok(Err(error)) => {
                let stat = match &error {
                    ToolError::Internal(_) => ToolStatOutcome::Failed,
                    ToolError::UnknownTool(_) => ToolStatOutcome::UnknownTool,
                    _ => ToolStatOutcome::InvalidArguments,
                };
                (error.into_result(), stat)
            }
            Err(BlockingError::Cancelled) => (
                ToolResult::interrupted(NOT_RUN),
                ToolStatOutcome::Interrupted,
            ),
            Err(BlockingError::Panicked(message)) => (
                ToolResult::error(format!(
                    "internal error: planning the call panicked ({message})."
                )),
                ToolStatOutcome::Failed,
            ),
        };
        self.finished(result, stat).await
    }

    /// A result the call gets without running, through the output policy.
    async fn finished(&self, result: ToolResult, stat: ToolStatOutcome) -> Report {
        Report::Finished {
            token: self.token,
            result: bound(result, &self.name, &self.cx).await,
            duration: Duration::ZERO,
            stat,
            notes: Vec::new(),
        }
    }

    /// The report of a call its hooks did not narrow, or refused.
    fn planned(&self, plan: Plan, decision: Decision) -> Report {
        Report::Planned {
            token: self.token,
            plan,
            decision,
            hooked: None,
        }
    }

    fn refused(&self, plan: Plan, why: String) -> Report {
        self.planned(plan, Decision::Deny { reason: Some(why) })
    }

    /// Shows the call to its `tool.before` hooks, when the policy did not
    /// refuse it and any hook is on it, and acts on what they say. With no
    /// such hook the decision stands, and nothing is awaited.
    async fn hooked(self, plan: Plan, decision: Decision) -> Report {
        let narrowable = matches!(
            decision,
            Decision::Allow | Decision::AllowReadOnly | Decision::Ask
        );
        let target = ToolTarget::of(&self.name, &self.arguments);
        let Some(set) = self
            .hooks
            .as_ref()
            .filter(|set| narrowable && set.hooks.wants(HookEvent::ToolBefore, Some(target)))
        else {
            return self.planned(plan, decision);
        };
        let matched = set.hooks.matching(HookEvent::ToolBefore, Some(target));
        let input = HookInput {
            event: HookEvent::ToolBefore,
            session: self.session.clone(),
            turn: self.turn.clone(),
            workspace: self.root.clone(),
            subject: HookSubject::Call {
                tool: self.name.clone(),
                call_id: self.call_id.clone(),
                arguments: self.arguments.clone(),
                summary: plan.summary.clone(),
                effects: plan.effects.clone(),
            },
        };
        let Some(heard) = self
            .cx
            .cancel
            .race(hooks::ask_all(set, matched, input, self.observers))
            .await
        else {
            return self
                .finished(
                    ToolResult::interrupted(NOT_RUN),
                    ToolStatOutcome::Interrupted,
                )
                .await;
        };
        match hooks::before(heard) {
            Before::Pass => self.planned(plan, decision),
            Before::Ask(asked) => Report::Planned {
                token: self.token,
                plan,
                decision,
                hooked: Some(Box::new(HookQuestion {
                    asked,
                    replaced: None,
                })),
            },
            Before::Deny(why) => self.refused(plan, why),
            Before::Replace {
                hook,
                arguments,
                reason,
                asked,
            } => self.replace(plan, hook, arguments, reason, asked).await,
        }
    }

    /// One hook replaced the call's arguments. The replacement is held to
    /// what the model's call is held to (the tool's schema, its tool, the
    /// policy), and to what only it is: a `tool_call` may not be turned
    /// into a call of another tool, and nothing it writes may be a
    /// protected path. Then it is put to the user, whatever the policy said
    /// of it: the report asks. A refusal stands, and a stop stops the turn.
    async fn replace(
        self,
        plan: Plan,
        hook: String,
        replacement: Json,
        reason: Option<String>,
        asked: Vec<HookReason>,
    ) -> Report {
        // The hook's name as the harness writes it inside backticks.
        let shown = cineris_base::in_code(&hook).into_owned();
        if let Some(other) = hooks::names_another_tool(&self.name, &self.arguments, &replacement) {
            return self.refused(
                plan,
                format!(
                    "the hook `{shown}` replaced its arguments with ones that call another tool \
                     (`{}`), which a hook may not do",
                    cineris_base::in_code(&other)
                ),
            );
        }
        if let Some(size) = hooks::too_large(&replacement) {
            return self.refused(
                plan,
                format!(
                    "the hook `{shown}` replaced its arguments with {size} bytes of JSON, more \
                     than the {} a replacement may have",
                    hooks::ARGUMENTS_CAP
                ),
            );
        }
        if let Err(mismatch) =
            cineris_tools::schema::validate(&self.tool.spec().parameters, &replacement)
        {
            return self.refused(
                plan,
                format!(
                    "the hook `{shown}` replaced its arguments with ones that do not fit the \
                     tool's schema:\n{mismatch}"
                ),
            );
        }
        let checked = self.replan(replacement).await;
        let (replacement, replanned, decision, checked) = match checked {
            Ok(Ok(replanned)) => replanned,
            Ok(Err(error)) => {
                return self.refused(
                    plan,
                    format!(
                        "the hook `{shown}` replaced its arguments with ones its tool refuses: \
                         {error}"
                    ),
                );
            }
            Err(BlockingError::Cancelled) => {
                return self
                    .finished(
                        ToolResult::interrupted(NOT_RUN),
                        ToolStatOutcome::Interrupted,
                    )
                    .await;
            }
            Err(BlockingError::Panicked(message)) => {
                return self
                    .finished(
                        ToolResult::error(format!(
                            "internal error: planning the call a hook replaced panicked \
                             ({message})."
                        )),
                        ToolStatOutcome::Failed,
                    )
                    .await;
            }
        };
        match decision {
            Decision::Deny { reason } => {
                let mut why = format!(
                    "the hook `{shown}` replaced its arguments, and the call they make is not \
                     allowed"
                );
                if let Some(reason) = reason {
                    why.push_str(": ");
                    why.push_str(&reason);
                }
                self.refused(replanned, why)
            }
            // A stop stands, as the policy gave it.
            Decision::Stop { .. } => self.planned(replanned, decision),
            _ => match checked {
                Err(refused) => self.refused(
                    replanned,
                    format!(
                        "the hook `{shown}` replaced its arguments with ones that may not be put \
                         to the user: {refused}"
                    ),
                ),
                Ok(()) => Report::Planned {
                    token: self.token,
                    plan: replanned,
                    decision,
                    hooked: Some(Box::new(HookQuestion {
                        asked,
                        replaced: Some(ReplacedArguments {
                            hook,
                            arguments: replacement,
                            original: Some(self.arguments),
                            reason,
                        }),
                    })),
                },
            },
        }
    }

    /// Plans the replacement, has the policy decide it, and, unless it
    /// refused it, asks the hooks whether it may be put to the user: on
    /// the blocking pool, where planning may ask the file system where a
    /// path leads, and the check may read it.
    async fn replan(
        &self,
        replacement: Json,
    ) -> Result<Result<Replanned, ToolError>, BlockingError> {
        let tool = Arc::clone(&self.tool);
        let policy = Arc::clone(&self.policy);
        let hooks = self.hooks.as_ref().map(|set| Arc::clone(&set.hooks));
        let plan_cx = self.cx.plan_cx();
        let (session, turn) = (self.session.clone(), self.turn.clone());
        let (call_id, item) = (self.call_id.clone(), self.item.clone());
        run_blocking(&self.cx.cancel, move |_| {
            let replanned = tool.plan(&replacement, &plan_cx)?;
            let call = PlannedCall {
                session: &session,
                turn: &turn,
                call_id: &call_id,
                item: &item,
                spec: tool.spec(),
                arguments: &replacement,
                plan: &replanned,
                preview: None,
                hooks: None,
            };
            let decision = policy.decide(&call);
            let checked = match (&decision, &hooks) {
                (Decision::Deny { .. } | Decision::Stop { .. }, _) | (_, None) => Ok(()),
                (_, Some(hooks)) => hooks.check_replacement(&call),
            };
            Ok((replacement, replanned, decision, checked))
        })
        .await
    }
}

/// The text parts of a result, one after another.
fn text_of(content: &[Part]) -> String {
    let mut text = String::new();
    for part in content {
        if let Part::Text(part) = part {
            text.push_str(&part.text);
            text.push('\n');
        }
    }
    text
}

/// Sends a running tool's output to front ends as it arrives.
struct OutputDeltas {
    emitter: Arc<Emitter>,
    turn: TurnId,
    item_id: ItemId,
}

impl ToolOutput for OutputDeltas {
    fn output(&mut self, text: &str) {
        self.emitter.emit(
            Some(&self.turn),
            Event::ItemDelta(ItemDelta {
                item_id: self.item_id.clone(),
                delta: Delta::ToolOutput(TextDelta {
                    text: text.to_owned(),
                }),
            }),
        );
    }
}
