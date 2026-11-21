//! The loop itself: one session's turns, state by state
//! (`docs/architecture/03-agent-loop.md`, "Turn state machine").
//!
//! A [`Session`] is one task. It waits, idle, for a prompt; runs the turn
//! the prompt starts, as a walk through the states of
//! [`TurnState`](crate::TurnState); and when the turn completed and a
//! prompt is queued, runs the next. Each state is one method, which does
//! the state's work and returns where the turn goes next, with whatever
//! the next state needs. Nothing about a turn is kept in a flag.
//!
//! **Cancellation is an input.** Every wait that can take time is raced
//! against the turn's token. What must not be cut short (an append to the
//! log, recording the results of a round) is never raced.
//!
//! What the loop does for its context (epochs, instruction files, the
//! budget, compaction) is in [`epochs`].

mod epochs;
mod map;

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use cineris_base::{AbsPath, CancelToken, Classified, IdCounter, SharedClock};
use cineris_llm::error::ProviderError;
use cineris_llm::{CooldownChange, ModelEvent, ModelRequest, SpeedCooldowns, SpeedMove, Usage};
use cineris_protocol::error::code;
use cineris_protocol::event::{
    Event, InfoNotice, ItemCompleted, ItemDelta, ItemKind, ItemRef, ItemStarted, LimitNotice,
    Notice, PromptReturnedNotice, RetryNotice, SessionIdle, TurnCompleted, TurnOutcome,
    TurnStarted, UsageUpdated, Verification, VerificationNotice,
};
use cineris_protocol::item::{
    AssistantMessageItem, Delivery, HookReason, NoticeItem, NoticeSource, UserMessageItem,
};
use cineris_protocol::message::Origin;
use cineris_protocol::model::ModelRole;
use cineris_protocol::session::RoleState;
use cineris_protocol::speed::{CooldownState, SpeedFallbackBy};
use cineris_protocol::usage::{RequestUsage, UsageTotals};
use cineris_protocol::{ErrorData, ErrorKind, Item, ItemId, Part, StopReason, TurnId};
use tokio::sync::Notify;
use tracing::Instrument;

use crate::assistant::{Applied, Calls, Draft};
use crate::config::{LoopConfig, Models, RoleModel, SessionInfo, Workspace};
use crate::context::{Context, ContextParts, Pressure};
use crate::events::{Emitter, EventSink};
use crate::guard::{Repetition, RepetitionGuard};
use crate::guarded::Guarded;
use crate::hooks::{self, HookEvent, HookInput, HookSubject, LoopHooks, Submitted};
use crate::ledger::{Ledger, LimitReached};
use crate::policy::Policy;
use crate::round::{Report, Round, RoundEnv, Stage};
use crate::session::{Inbox, Pending, Prompt, Queued, Running, SessionHandle, Shared};
use crate::speed::{Moves, SpeedSettings, cooldown_notice, fallback_notice};
use crate::state::{Observer, TurnState};
use crate::stats::{
    ContextStat, RequestPurpose, RequestStat, StatEvent, Stats, ToolCallStat, TurnStat,
};
use crate::store::{Attempt, ITEM_PREFIX, Record, SessionStore, StoreError, TURN_PREFIX};
use crate::timing::{Spend, TurnClock};
use crate::toolset::ToolSet;
use crate::usage::{UsageBook, add_request};
use crate::verify::{Verifier, VerifyCx, VerifyOutcome};

/// The library point just after a turn's `turn.completed` is told, before
/// `session.updated` and `session.idle` (`cineris_base::hook`): a test holds
/// it to look at the session as a front end that acts on the event finds
/// it.
pub const TURN_COMPLETED_TOLD: &str = "conclude: turn.completed is told";

/// Everything a session is made of.
pub struct SessionParts {
    /// What the loop is told about the session and does not change.
    pub info: SessionInfo,
    /// The models, by role.
    pub models: Models,
    /// The speed the session and its roles start at.
    pub speeds: SpeedSettings,
    /// The tools the model may call.
    pub tools: Arc<dyn ToolSet>,
    /// Decides whether a planned call may run.
    pub policy: Arc<dyn Policy>,
    /// Where the session is kept.
    pub store: Box<dyn SessionStore>,
    /// What the session's context is made from: the system prompt's
    /// facts, the instruction files, the thresholds.
    pub context: ContextParts,
    /// Where the session's events go.
    pub events: Arc<dyn EventSink>,
    /// Where its statistics events go.
    pub stats: Arc<dyn Stats>,
    /// Runs the project's checks.
    pub verifier: Arc<dyn Verifier>,
    /// The clock: timestamps, durations and the wall-clock limit.
    pub clock: SharedClock,
    /// The table of speed-tier cooldowns: the same one the model clients
    /// were given.
    pub cooldowns: SpeedCooldowns,
    /// The limits the session draws on.
    pub ledger: Ledger,
    /// Where the session's tools work.
    pub workspace: Workspace,
    /// How the loop is set.
    pub config: LoopConfig,
    /// The session's cancellation token: a child of the process's.
    pub cancel: CancelToken,
    /// Told of every change of state.
    pub observer: Option<Arc<dyn Observer>>,
    /// The session's hooks, and the timer their times are kept on. `None`:
    /// the session has none, and every hook point is passed at once.
    pub hooks: Option<LoopHooks>,
}

impl std::fmt::Debug for SessionParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionParts")
            .field("info", &self.info)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// One session's loop. [`run`](Self::run) is the task; a [`SessionHandle`]
/// is how everything else reaches it.
pub struct Session {
    shared: Arc<Shared>,
    env: Arc<RoundEnv>,
    store: Box<dyn SessionStore>,
    context: Context,
    stats: Arc<dyn Stats>,
    verifier: Arc<dyn Verifier>,
    clock: SharedClock,
    ledger: Ledger,
    config: LoopConfig,
    observer: Option<Arc<dyn Observer>>,
    item_ids: IdCounter,
    /// The main model of the request that is being prepared or is in
    /// flight, and the version of the catalog its prices come from. It is
    /// read from the session's settings once for each request, in
    /// `Preparing`, so that a change of model in the middle of a request
    /// applies from the next one and a request is accounted to the model
    /// it was sent to.
    model: RoleModel,
    catalog_version: String,
    /// How many tool rounds have had their results recorded, and so sent
    /// to the model with the next request. A round's number is one more:
    /// it moves on only when the model will see what the round returned,
    /// which is what a tool takes a later round to mean.
    rounds_recorded: u64,
    state: TurnState,
    usage: UsageBook,
    moves: Moves,
    guard: RepetitionGuard,
    /// The roles' state as front ends were last told it.
    told_roles: Vec<RoleState>,
    /// When `usage.updated` was last sent, on the monotonic clock.
    usage_sent: Option<Duration>,
    /// A summary taken in the background, running or ready.
    background: Option<epochs::Background>,
    /// The epoch in which a summary in the background failed: no other is
    /// started in it.
    background_failed: Option<u64>,
    /// Whether the user was told that the trigger, or the turns a
    /// compaction keeps, are held to what a compaction needs.
    told_trigger: bool,
    told_keep: bool,
    /// The epoch in which the `fast` role's model refused a summary request
    /// as too long: summaries go to the main model for the rest of it.
    fast_refused: Option<u64>,
    /// Summaries given up after their request finished: counted at the next
    /// boundary.
    given_up: Vec<tokio::task::JoinHandle<epochs::Summarized>>,
    /// A compaction the user asked for is waiting.
    manual: bool,
    /// What it should keep in particular.
    manual_focus: Option<String>,
    /// The context record of the request in flight, sent with its
    /// `Request` event.
    pending_context: Option<ContextStat>,
    /// Where the running turn's time has gone (`crate::timing`).
    times: TurnClock,
    /// The budget of each utility role whose requests have taught it
    /// something: a refusal as too long, or a count. A role that has none
    /// is estimated at the main model's ratio. The main model's budget is
    /// the context's.
    role_budgets: std::collections::BTreeMap<ModelRole, crate::context::Budget>,
    /// The last turn that ran: what a summary that finishes after the
    /// session's last boundary is counted in.
    last_turn: Option<TurnId>,
    /// The role the last summary counted was asked of.
    summary_role: ModelRole,
    /// The span of the request in flight: its attempts are recorded on it
    /// when its record is made.
    request_span: tracing::Span,
    /// The repository map asked for, and the epochs it was given in.
    map: map::MapState,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.shared.info.id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// One turn, while it runs.
struct Turn {
    id: TurnId,
    /// The turn's number in the session, counted from 1.
    number: u32,
    cancel: CancelToken,
    /// The token of a second interrupt, which every tool call is given.
    kill: CancelToken,
    /// When it started, on the monotonic clock.
    started: Duration,
    usage: UsageTotals,
    requests: u64,
    tool_calls: u64,
    /// What its calls have changed, sorted (`cineris_tools::changed_by`).
    written: Vec<AbsPath>,
    /// Whether a call changed something since the last round of the
    /// project's checks: what sends the turn to another round.
    changed: bool,
    /// The rounds of checks it has run, and the time they took.
    verify_rounds: u32,
    verify_used: Duration,
    /// The verdict of its last round of checks.
    verification: Option<Verification>,
    /// The turn only runs the project's checks: `verify/run`.
    only_verify: bool,
    /// Why the last response ended.
    stop: Option<StopReason>,
}

/// What the request being prepared has already been through. It travels
/// with the request from state to state, and a new request starts clean.
#[derive(Debug, Clone, Copy, Default)]
struct Prep {
    /// The context was compacted for this request because it was over its
    /// budget.
    compacted: bool,
    /// The provider said the request did not fit, and a compaction answered
    /// that. A second time ends the turn.
    overflowed: bool,
    /// The response was cut off inside a tool call, and the request is
    /// sent once more with the model's whole output limit.
    raised_output: bool,
    /// The turn only compacts: `session/compact` on an idle session.
    only: bool,
}

/// The response as far as it has arrived.
struct Live {
    /// The id of the assistant item: one per request, whatever number of
    /// attempts it takes.
    item_id: ItemId,
    draft: Draft,
    /// Whether front ends were told the item began.
    shown: bool,
    /// Which attempt is arriving, counted from 1.
    attempt: u32,
    /// What the attempt has used so far, where the provider said.
    usage: Option<Usage>,
    /// When the request was sent, and when its first content arrived.
    sent: Duration,
    first_content: Option<Duration>,
    /// The turn's model time when the request was sent: the request's own
    /// is what it has grown by at its end.
    model_before: [Duration; 4],
    /// What the attempts given up so far used, which the ledger was charged
    /// as each was given up: the request's statistics event carries it with
    /// the last attempt's, so that the statistics hold what the ledger was
    /// charged, and other processes' day limits see it.
    voided: Option<RequestUsage>,
}

/// A response that ended.
struct Response {
    live: Live,
    round: Round,
    stop: StopReason,
    raw_stop: String,
    stop_detail: Option<String>,
    usage: Usage,
    prep: Prep,
}

/// How a turn ends, when it is not interrupted.
struct End {
    outcome: TurnOutcome,
    error: Option<ErrorData>,
}

impl End {
    fn completed() -> Self {
        Self {
            outcome: TurnOutcome::Completed,
            error: None,
        }
    }

    fn failed(error: ErrorData) -> Self {
        Self {
            outcome: TurnOutcome::Failed,
            error: Some(error),
        }
    }

    fn store(error: &StoreError) -> Self {
        Self::failed(error.to_error_data())
    }
}

/// Where a turn goes next.
enum Phase {
    Preparing(Prep),
    Compacting(Prep),
    Requesting(Box<ModelRequest>, Prep),
    Deciding(Box<Response>),
    ToolRound(Box<Round>),
    Verifying,
    Finishing(End),
    /// With the response that was arriving, if one was.
    Interrupted(Option<Box<(Live, Round)>>),
}

/// How a turn's start went.
enum Begun {
    /// What the turn begins with is recorded: it goes on.
    Ready,
    /// It ended before anything was recorded: the prompt's hooks refused
    /// it, or the turn was interrupted while they were asked.
    Ended(Phase),
}

/// What ended a wait while a response arrives.
enum Woke {
    Cancelled,
    Report(Report),
    Event(Option<ModelEvent>),
}

/// Waits for the turn to be cancelled, for a report from the round, or for
/// the next event of the response. Cancellation is looked at first, so
/// cancelled work does not go on; then the round, so that a call that could
/// start is not kept waiting by a response that has more to say.
async fn wake(
    cancel: &CancelToken,
    report: impl Future<Output = Report>,
    event: impl Future<Output = Option<ModelEvent>>,
) -> Woke {
    let mut cancelled = pin!(cancel.cancelled());
    let mut report = pin!(report);
    let mut event = pin!(event);
    std::future::poll_fn(|context| {
        if cancelled.as_mut().poll(context).is_ready() {
            return Poll::Ready(Woke::Cancelled);
        }
        if let Poll::Ready(report) = report.as_mut().poll(context) {
            return Poll::Ready(Woke::Report(report));
        }
        if let Poll::Ready(event) = event.as_mut().poll(context) {
            return Poll::Ready(Woke::Event(event));
        }
        Poll::Pending
    })
    .await
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl Session {
    /// Opens a session: reads it from its store, and returns its loop and a
    /// handle to it. Nothing runs until [`run`](Self::run) is polled.
    ///
    /// # Errors
    ///
    /// When the store cannot read the session.
    pub async fn open(parts: SessionParts) -> Result<(Self, SessionHandle), StoreError> {
        let SessionParts {
            info,
            models,
            speeds,
            tools,
            policy,
            mut store,
            context,
            events,
            stats,
            verifier,
            clock,
            cooldowns,
            ledger,
            workspace,
            config,
            cancel,
            observer,
            hooks,
        } = parts;
        let opened = store.open().await?;
        let mut context = Context::new(context, &tools.specs());
        context.load(&opened);
        context.fix_variants(&models.main().variants);

        let emitter = Arc::new(Emitter::new(info.id.clone(), events));
        let env = Arc::new(RoundEnv {
            session: info.id.clone(),
            tools,
            policy,
            workspace,
            emitter: Arc::clone(&emitter),
            clock: Arc::clone(&clock),
            max_parallel: config.max_parallel_tools.max(1),
            hooks,
        });
        let model = models.main().clone();
        let catalog_version = models.catalog_version().to_owned();
        let (level, lease_id, title, sandbox) = (
            info.level.clone(),
            info.lease_id.clone(),
            info.title.clone(),
            info.sandbox.clone(),
        );
        let shared = Arc::new(Shared {
            info,
            cooldowns,
            emitter,
            cancel,
            wake: Notify::new(),
            inbox: std::sync::Mutex::new(Inbox {
                turn_ids: IdCounter::new(TURN_PREFIX).after(opened.last_turn),
                running: None,
                starting: None,
                pending: std::collections::VecDeque::new(),
                queue: std::collections::VecDeque::new(),
                held: std::collections::VecDeque::new(),
                speeds,
                models,
                level,
                lease_id,
                sandbox,
                title,
                state: TurnState::Idle,
                usage: cineris_protocol::usage::SessionUsage::default(),
                reload: false,
                changing: false,
                history_changed: false,
                diverged: false,
                history: !opened.items.is_empty(),
                windows: std::collections::BTreeMap::new(),
                closed: false,
                reconfigured: None,
                next_epoch: None,
                trust: None,
            }),
        });
        let told_roles = shared.state(&shared.lock()).roles.into_values().collect();
        let session = Self {
            shared: Arc::clone(&shared),
            env,
            store,
            context,
            stats,
            verifier,
            clock,
            ledger,
            guard: RepetitionGuard::new(config.repetition),
            config,
            observer,
            item_ids: IdCounter::new(ITEM_PREFIX).after(opened.last_item),
            model,
            catalog_version,
            rounds_recorded: 0,
            state: TurnState::Idle,
            usage: UsageBook::default(),
            moves: Moves::default(),
            told_roles,
            usage_sent: None,
            background: None,
            background_failed: None,
            fast_refused: None,
            told_trigger: false,
            told_keep: false,
            given_up: Vec::new(),
            manual: false,
            manual_focus: None,
            pending_context: None,
            times: TurnClock::new(Duration::ZERO),
            role_budgets: std::collections::BTreeMap::new(),
            last_turn: None,
            summary_role: ModelRole::Main,
            request_span: tracing::Span::none(),
            map: map::MapState::default(),
        };
        Ok((session, SessionHandle { shared }))
    }

    /// Runs the session as a task of its own on the current runtime.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(self.run())
    }

    /// The loop: runs until the session is closed or its token is
    /// cancelled, and a turn that was running then has ended.
    pub async fn run(mut self) {
        while let Some(start) = self.next_prompt().await {
            // The session is no longer idle. (When one turn follows another
            // front ends are told as the first one ends.)
            self.shared.emit_updated(&self.shared.lock());
            let mut next = Some(start);
            while let Some((running, prompt)) = next.take() {
                let turn = running.id.clone();
                // The turn's span: its requests and its tools are in it
                // (`docs/architecture/02-runtime.md`, "Logging and
                // tracing").
                let span = tracing::info_span!(
                    "turn",
                    session = %self.shared.info.id,
                    turn = %turn,
                );
                // A panic is a bug. It ends the turn, and the session can
                // take the next prompt.
                let ended = Guarded(Box::pin(self.turn(running, prompt).instrument(span))).await;
                next = match ended {
                    Ok(next) => next,
                    Err(panic) => {
                        self.recover(&turn, &panic);
                        None
                    }
                };
            }
        }
        // A summary the session's last boundary did not count was spent all
        // the same.
        self.count_left_summaries().await;
    }

    /// Ends a turn whose code panicked: front ends get `turn.completed`
    /// with the failure, whatever was waiting is held, and the session is
    /// read again from its store before the next turn, because what the
    /// loop holds in memory can no longer be trusted.
    fn recover(&mut self, turn: &TurnId, panic: &str) {
        let completed = TurnCompleted {
            outcome: TurnOutcome::Failed,
            stop_reason: None,
            error: Some(ErrorData::new(
                ErrorKind::Internal,
                "loop_panicked",
                format!(
                    "internal error: the session's loop panicked ({panic}). The turn was ended."
                ),
            )),
            usage: UsageTotals::default(),
            verification: None,
            duration_ms: None,
        };
        self.shared
            .emitter
            .emit(Some(turn), Event::TurnCompleted(completed));
        self.state = TurnState::Idle;
        let mut inbox = self.shared.lock();
        Self::hold(&mut inbox);
        inbox.running = None;
        inbox.reload = true;
        inbox.diverged = true;
        inbox.state = TurnState::Idle;
        self.shared.emit_updated(&inbox);
    }

    // ------------------------------------------------------------------ idle

    /// Waits, with no timer, for a prompt to start a turn. `None` when the
    /// session is closed.
    async fn next_prompt(&mut self) -> Option<(Running, Prompt)> {
        loop {
            {
                let mut inbox = self.shared.lock();
                if let Some(prompt) = inbox.starting.take()
                    && let Some(running) = inbox.running.clone()
                {
                    return Some((running, prompt));
                }
                if inbox.closed || self.shared.cancel.is_cancelled() {
                    return None;
                }
            }
            // A wake that came since the look above is kept by the
            // notifier, so it is not missed.
            let woken = self.shared.cancel.race(self.shared.wake.notified()).await;
            if woken.is_none() && self.shared.lock().starting.is_none() {
                return None;
            }
        }
    }

    /// A limit of the ledger refused a request, or a turn that only
    /// verifies its round: the turn ends with it.
    async fn limit_reached(&mut self, turn: &mut Turn, reached: &LimitReached) -> Phase {
        let message = reached.message();
        self.notify(
            turn,
            Notice::Limit(LimitNotice {
                message: message.clone(),
                limit: reached.kind.clone(),
                reached: true,
            }),
        );
        // A turn that only verifies records nothing.
        if !turn.only_verify
            && let Err(error) = self
                .record_notice(turn, NoticeSource::Limit, message.clone())
                .await
        {
            return Phase::Finishing(End::store(&error));
        }
        Phase::Finishing(End {
            outcome: TurnOutcome::Limit,
            error: Some(ErrorData::new(ErrorKind::Limit, reached.code(), message)),
        })
    }

    /// Reads the session again from its store: its items changed under the
    /// loop, or an append failed and memory must be brought back to what
    /// the log holds.
    ///
    /// **The table of seen files starts a new context epoch here**, for
    /// every reason the history is read again: what the model was shown
    /// may not be in the history any more (a rewind took it away), so the
    /// next read of any file is in full and never "unchanged" or a diff
    /// against a version the model does not have. After a change of history
    /// ([`HistoryChange::made`](crate::HistoryChange::made)) it forgets
    /// every file whole, snapshots too: one read or written only in a turn
    /// the rewind took away would otherwise pass the staleness guard and be
    /// overwritten unseen. This is the one place a change of history
    /// reaches the table, and where compaction starts its epochs too
    /// (`docs/architecture/07-sessions-and-context.md`, "What each part of
    /// a session's state follows").
    async fn reload(&mut self) -> Result<(), StoreError> {
        let (changed, diverged) = {
            let mut inbox = self.shared.lock();
            (
                std::mem::take(&mut inbox.history_changed),
                std::mem::take(&mut inbox.diverged),
            )
        };
        self.new_file_epoch();
        if changed {
            self.env.workspace.files.forget_all();
        }
        // After a change of history, a summary taken over the history as it
        // was is not of the branch the loop reads now, and the request
        // before is not the one the next must begin with. A reload that
        // changed nothing keeps both.
        if changed {
            self.drop_background();
            // The epoch's number may come again, for another history.
            self.fast_refused = None;
            self.forget_map();
        }
        // The request before was sent as it was, whatever is read now.
        self.settle_sent();
        let opened = self.store.open().await?;
        // Nor is the request before the one the next must begin with when
        // memory held what the log does not, or the log holds an entry
        // otherwise than it was sent (masked by the secret filter, or
        // stored as its projection): the history read again is not the one
        // that request was built from.
        if changed || diverged || opened.masked {
            self.context.last_prefix = None;
        }
        self.context.load(&opened);
        self.context.fix_variants(&self.model.variants);
        self.shared.lock().history = !opened.items.is_empty();
        let next = self.item_ids.peek().max(opened.last_item.saturating_add(1));
        self.item_ids = IdCounter::new(ITEM_PREFIX).after(next - 1);
        Ok(())
    }

    // ------------------------------------------------------------ the turn

    /// Runs one turn to its end. Returns the prompt of the next turn, when
    /// one starts by itself.
    async fn turn(&mut self, running: Running, prompt: Prompt) -> Option<(Running, Prompt)> {
        let number = IdCounter::new(TURN_PREFIX)
            .number_of(running.id.as_str())
            .and_then(|number| u32::try_from(number).ok())
            .unwrap_or(0);
        // One reading for the clock of its parts and the turn's start, so
        // that they agree to the nanosecond (found by the review of step 16).
        let started = self.clock.monotonic();
        self.times = TurnClock::new(started);
        self.last_turn = Some(running.id.clone());
        let mut turn = Turn {
            id: running.id,
            number,
            cancel: running.cancel,
            kill: running.kill,
            started,
            usage: UsageTotals::default(),
            requests: 0,
            tool_calls: 0,
            written: Vec::new(),
            changed: false,
            verify_rounds: 0,
            verify_used: Duration::ZERO,
            verification: None,
            only_verify: prompt.verify,
            stop: None,
        };
        self.ledger.begin_turn();
        self.guard.reset();
        self.enter(TurnState::Preparing);
        self.emit(&turn, Event::TurnStarted(TurnStarted {}));

        // If the session's history changed under the loop it is read again
        // first. When that fails the turn fails, and the next turn tries
        // again: no request goes out on a history that may have a hole.
        let reload = std::mem::take(&mut self.shared.lock().reload);
        let ready = if reload { self.reload().await } else { Ok(()) };
        let only = prompt.compact.is_some();
        let begun = match ready {
            // A turn that only verifies records nothing: no epoch, no
            // notice, no prompt. The notices waiting stay for the next turn.
            Ok(()) if turn.only_verify => Ok(Begun::Ready),
            Ok(()) => self.begin(&turn, prompt).await,
            Err(error) => Err(error),
        };
        // After the turn's start has asked for the epoch's map, if it had
        // none: a pass in the background that gave way now would only run
        // again for that request (the review of step 18).
        self.map_turn_started();
        let mut phase = match begun {
            // The prompt's hooks refused it, or the turn was interrupted
            // while they were asked.
            Ok(Begun::Ended(phase)) => phase,
            // A time limit that is spent ends a turn that only verifies
            // before any check starts, as it ends a turn before its request.
            Ok(Begun::Ready) if turn.only_verify => match self.ledger.out_of_time() {
                Some(reached) => self.limit_reached(&mut turn, &reached).await,
                None => Phase::Verifying,
            },
            Ok(Begun::Ready) if only => Phase::Compacting(Prep {
                only: true,
                ..Prep::default()
            }),
            Ok(Begun::Ready) => Phase::Preparing(Prep::default()),
            Err(error) => Phase::Finishing(End::store(&error)),
        };
        loop {
            phase = match phase {
                Phase::Preparing(prep) => self.prepare(&mut turn, prep).await,
                Phase::Compacting(prep) => self.compacting(&mut turn, prep).await,
                Phase::Requesting(request, prep) => {
                    let span = tracing::info_span!(
                        "request",
                        role = "main",
                        provider = %request.model.info.provider,
                        model = %request.model.info.id,
                        attempts = tracing::field::Empty,
                    );
                    self.request_span = span.clone();
                    self.respond(&mut turn, *request, prep)
                        .instrument(span)
                        .await
                }
                Phase::Deciding(response) => self.decide(&mut turn, *response).await,
                Phase::ToolRound(round) => self.tool_round(&mut turn, *round).await,
                Phase::Verifying => self.verify(&mut turn).await,
                Phase::Finishing(end) => {
                    let next = self.finish(&turn, end).await;
                    self.map_turn_ended();
                    return next;
                }
                Phase::Interrupted(partial) => {
                    self.interrupted(&mut turn, partial).await;
                    self.map_turn_ended();
                    return None;
                }
            };
        }
    }

    /// The first thing a turn records: what its context needs (the epoch,
    /// changed instruction files, the date), the notices that were waiting
    /// for it, then the prompt, and what its hooks added after it. A turn
    /// that only compacts records no prompt.
    ///
    /// **The prompt's `prompt.submit` hooks are asked first.** A prompt they
    /// refuse is not recorded, and nor is anything else of the turn: it is
    /// given back to the front end with their reasons, and the turn ends
    /// `denied`.
    async fn begin(&mut self, turn: &Turn, prompt: Prompt) -> Result<Begun, StoreError> {
        let mut context = Vec::new();
        if prompt.compact.is_none() && self.wants_prompts() {
            match self
                .submit(turn, &prompt.input, prompt.delivery.clone())
                .await
            {
                None => {
                    self.give_back(
                        turn,
                        prompt.input,
                        prompt.delivery,
                        Vec::new(),
                        hooks::RETURNED_UNASKED.to_owned(),
                    );
                    return Ok(Begun::Ended(Phase::Interrupted(None)));
                }
                Some(Submitted::Deny(refused)) => {
                    let message = hooks::refused_prompt(&refused, false);
                    self.give_back(
                        turn,
                        prompt.input,
                        prompt.delivery,
                        refused,
                        message.clone(),
                    );
                    return Ok(Begun::Ended(Phase::Finishing(End {
                        outcome: TurnOutcome::Denied,
                        error: Some(ErrorData::new(
                            ErrorKind::Permission,
                            code::PROMPT_DENIED,
                            message,
                        )),
                    })));
                }
                Some(Submitted::Pass(added)) => context = added,
            }
        }
        self.context_turn_start(turn).await?;
        let notices: Vec<Pending> = {
            let mut inbox = self.shared.lock();
            let (notices, steering): (Vec<Pending>, Vec<Pending>) =
                std::mem::take(&mut inbox.pending)
                    .into_iter()
                    .partition(|pending| matches!(pending, Pending::Notice { .. }));
            inbox.pending = steering.into();
            notices
        };
        for notice in notices {
            if let Pending::Notice { source, text } = notice {
                self.record_notice(turn, source, text).await?;
            }
        }
        if let Some(focus) = prompt.compact {
            self.manual = true;
            self.manual_focus = focus;
            return Ok(Begun::Ready);
        }
        // The epoch's map, before the prompt, when it is ready by then.
        self.give_map(turn).await?;
        let item = Item::UserMessage(UserMessageItem {
            id: self.item_ids.next_id(),
            parts: prompt.input,
            delivery: prompt.delivery,
            created_at: Some(self.clock.now().to_timestamp()),
        });
        self.record_shown(turn, item).await?;
        self.shared.lock().history = true;
        self.add_context(turn, context).await?;
        Ok(Begun::Ready)
    }

    /// Whether any `prompt.submit` hook wants a prompt: asked before any
    /// await, so that a session with none passes the point at once.
    fn wants_prompts(&self) -> bool {
        self.env
            .hooks
            .as_ref()
            .is_some_and(|set| set.hooks.wants(HookEvent::PromptSubmit, None))
    }

    /// Asks a prompt's `prompt.submit` hooks, and combines what they say.
    /// `None` when the turn was interrupted before they all answered.
    /// (`&mut`, so that the future is `Send`: the session is not `Sync`.)
    async fn submit(
        &mut self,
        turn: &Turn,
        parts: &[Part],
        delivery: Option<Delivery>,
    ) -> Option<Submitted> {
        let Some(set) = self.env.hooks.as_ref() else {
            return Some(Submitted::Pass(Vec::new()));
        };
        let matched = set.hooks.matching(HookEvent::PromptSubmit, None);
        let input = HookInput {
            event: HookEvent::PromptSubmit,
            session: self.shared.info.id.clone(),
            turn: turn.id.clone(),
            workspace: self.env.workspace.paths.root().clone(),
            subject: HookSubject::Prompt {
                parts: parts.to_vec(),
                delivery,
            },
        };
        let heard = turn
            .cancel
            .race(hooks::ask_all(set, matched, input))
            .await?;
        Some(hooks::submitted(heard))
    }

    /// Gives a prompt that was not sent back to the front end, for its
    /// editor.
    fn give_back(
        &self,
        turn: &Turn,
        parts: Vec<Part>,
        delivery: Option<Delivery>,
        refused: Vec<HookReason>,
        message: String,
    ) {
        self.notify(
            turn,
            Notice::PromptReturned(PromptReturnedNotice {
                message,
                parts,
                delivery,
                hooks: refused,
            }),
        );
    }

    /// Runs a turn's `turn.end` hooks on a task of their own, not waited
    /// for. What each adds is a notice for the next turn: given at its
    /// start if it is in by then, else at the next safe boundary, as any
    /// notice waiting is. A hook that fails is shown to the user and adds
    /// nothing. Closing the session stops them.
    fn turn_ended(&self, turn: &Turn, outcome: &TurnOutcome, error: Option<String>) {
        let Some(set) = self
            .env
            .hooks
            .as_ref()
            .filter(|set| set.hooks.wants(HookEvent::TurnEnd, None))
        else {
            return;
        };
        let set = set.clone();
        let matched = set.hooks.matching(HookEvent::TurnEnd, None);
        let input = HookInput {
            event: HookEvent::TurnEnd,
            session: self.shared.info.id.clone(),
            turn: turn.id.clone(),
            workspace: self.env.workspace.paths.root().clone(),
            subject: HookSubject::TurnEnded {
                outcome: outcome.clone(),
                error,
            },
        };
        let shared = Arc::clone(&self.shared);
        tokio::spawn(async move {
            let Some(heard) = shared
                .cancel
                .race(hooks::ask_all(&set, matched, input))
                .await
            else {
                return;
            };
            let added = hooks::added(HookEvent::TurnEnd, heard);
            for failed in added.failed {
                // The turn has ended: the notice is the session's.
                shared.emitter.emit(
                    None,
                    Event::Notice(Notice::Info(InfoNotice { message: failed })),
                );
            }
            if !added.texts.is_empty() {
                let mut inbox = shared.lock();
                for (hook, text) in added.texts {
                    inbox.pending.push_back(Pending::Notice {
                        source: NoticeSource::Hook,
                        text: hooks::turn_end_notice(&hook, &text),
                    });
                }
            }
        });
    }

    /// Records what a prompt's hooks added, each after the prompt under a
    /// line that names the hook.
    async fn add_context(
        &mut self,
        turn: &Turn,
        context: Vec<(String, String)>,
    ) -> Result<(), StoreError> {
        for (hook, text) in context {
            self.record_notice(
                turn,
                NoticeSource::Hook,
                hooks::context_notice(&hook, &text),
            )
            .await?;
        }
        Ok(())
    }

    /// Delivers a steering message at a safe boundary, once its
    /// `prompt.submit` hooks allow it: one they refuse is given back with
    /// their reasons, and the turn goes on without it. `None` when the turn
    /// was interrupted while they were asked; the message is given back.
    async fn deliver_steer(
        &mut self,
        turn: &Turn,
        input: Vec<Part>,
    ) -> Option<Result<(), StoreError>> {
        let mut context = Vec::new();
        if self.wants_prompts() {
            match self.submit(turn, &input, Some(Delivery::Steer)).await {
                None => {
                    self.give_back(
                        turn,
                        input,
                        Some(Delivery::Steer),
                        Vec::new(),
                        hooks::RETURNED_UNASKED.to_owned(),
                    );
                    return None;
                }
                Some(Submitted::Deny(refused)) => {
                    let message = hooks::refused_prompt(&refused, true);
                    self.give_back(turn, input, Some(Delivery::Steer), refused, message);
                    return Some(Ok(()));
                }
                Some(Submitted::Pass(added)) => context = added,
            }
        }
        let item = Item::UserMessage(UserMessageItem {
            id: self.item_ids.next_id(),
            parts: input,
            delivery: Some(Delivery::Steer),
            created_at: Some(self.clock.now().to_timestamp()),
        });
        if let Err(error) = self.record_shown(turn, item).await {
            return Some(Err(error));
        }
        Some(self.add_context(turn, context).await)
    }

    /// What was taken to be delivered at a boundary and was not, because
    /// the turn was interrupted while a steering message's hooks were
    /// asked: a steering message is given back, as an interrupt gives back
    /// those still waiting; a notice waits for the next turn; a compaction
    /// is dropped, as an interrupt drops one.
    fn return_pending(&self, turn: &Turn, rest: std::collections::VecDeque<Pending>) {
        let mut kept = Vec::new();
        for pending in rest {
            match pending {
                Pending::Steer(input) => self.give_back(
                    turn,
                    input,
                    Some(Delivery::Steer),
                    Vec::new(),
                    hooks::RETURNED_UNASKED.to_owned(),
                ),
                Pending::Notice { .. } => kept.push(pending),
                Pending::Compact(_) => {}
            }
        }
        let mut inbox = self.shared.lock();
        for pending in kept.into_iter().rev() {
            inbox.pending.push_front(pending);
        }
    }

    // ----------------------------------------------------------- preparing

    async fn prepare(&mut self, turn: &mut Turn, prep: Prep) -> Phase {
        if self.state != TurnState::Preparing {
            self.enter(TurnState::Preparing);
        }
        if turn.cancel.is_cancelled() {
            return Phase::Interrupted(None);
        }

        // This is a safe boundary: no tool round is open. Steering
        // messages and notices are delivered here, in the order they came.
        let mut pending = std::mem::take(&mut self.shared.lock().pending);
        while let Some(next) = pending.pop_front() {
            let recorded = match next {
                Pending::Steer(input) => match self.deliver_steer(turn, input).await {
                    Some(recorded) => recorded,
                    None => {
                        self.return_pending(turn, pending);
                        return Phase::Interrupted(None);
                    }
                },
                Pending::Notice { source, text } => self.record_notice(turn, source, text).await,
                Pending::Compact(focus) => {
                    self.manual = true;
                    self.manual_focus = focus;
                    Ok(())
                }
            };
            if let Err(error) = recorded {
                return Phase::Finishing(End::store(&error));
            }
        }

        // What a reload changed applies from here: no request is under way.
        self.take_reconfigured();
        // The model of this request: what the role is set to now. A change
        // of it starts a context epoch, before anything else is sent.
        self.refresh_main();
        if let Err(error) = self.context_before_request(turn).await {
            return Phase::Finishing(End::store(&error));
        }
        // A safe boundary: the epoch's map, if it has none and one is
        // ready (or, at its first chance, comes within the wait).
        if let Err(error) = self.give_map(turn).await {
            return Phase::Finishing(End::store(&error));
        }
        if let Err(error) = self.collect_background(turn).await {
            return Phase::Finishing(End::store(&error));
        }
        self.tell_clamped(turn);
        if self.manual {
            return Phase::Compacting(prep);
        }
        let prepare_summary = match self.pressure() {
            Pressure::Compact if !prep.compacted => {
                if !self.context.parts.settings.auto_compact {
                    return Phase::Finishing(self.compaction_off());
                }
                return Phase::Compacting(prep);
            }
            Pressure::Compact | Pressure::Prepare => true,
            Pressure::Fine => false,
        };

        // Every request is charged to the ledger before it is sent: this
        // one first, then a summary prepared beside it. An account spent
        // elsewhere too (a day's) is brought up to date first.
        self.ledger.catch_up().await;
        if let Err(reached) = self.ledger.take_request() {
            return self.limit_reached(turn, &reached).await;
        }
        if prepare_summary {
            self.start_background(turn);
        }

        self.tell_speed();
        let model = self.model.clone();
        let assembled = self.context.assemble();
        let mut request = ModelRequest::new(Arc::clone(&model.entry), ModelRole::Main);
        request.system = assembled.system;
        request.tools = assembled.tools;
        request.messages = assembled.messages;
        request.cache = assembled.cache;
        request.reasoning = model.reasoning.clone();
        // The variants are the epoch's: fixed for it.
        request.variants = self.epoch_variants();
        // The speed the role is set to. The client sends `standard` in its
        // place while the tier cools down or when the model has no such
        // tier, and says so in the stream.
        request.speed = self.shared.lock().speeds.of(&ModelRole::Main);
        request.max_output = if prep.raised_output {
            None
        } else {
            model.max_output
        };
        self.context_request(turn, &model, &request);
        turn.requests += 1;
        Phase::Requesting(Box::new(request), prep)
    }

    // ------------------------------------- requesting, streaming, backoff

    async fn respond(&mut self, turn: &mut Turn, request: ModelRequest, prep: Prep) -> Phase {
        self.enter(TurnState::Requesting);
        let model = self.model.clone();
        let sent = self.clock.monotonic();
        let model_before = self.times.model_time(sent);
        self.times.enter(sent, Spend::Connect);
        let mut live = Live {
            item_id: self.item_ids.next_id(),
            draft: Draft::default(),
            shown: false,
            attempt: 1,
            usage: None,
            sent,
            first_content: None,
            model_before,
            voided: None,
        };
        let mut round = Round::new(
            Arc::clone(&self.env),
            turn.id.clone(),
            turn.number,
            self.rounds_recorded + 1,
            turn.cancel.clone(),
            turn.kill.clone(),
        );
        // The request has a token of its own below the turn's. The loop
        // itself watches the turn's, and drops the stream when it fires:
        // that closes the connection, the one signal every provider honours.
        let mut stream = model.client.stream(request, turn.cancel.child());

        loop {
            let event = match wake(&turn.cancel, round.recv(), stream.next()).await {
                Woke::Cancelled => return Phase::Interrupted(Some(Box::new((live, round)))),
                Woke::Report(report) => {
                    round.handle(report);
                    round.pump(Stage::Streaming {
                        open_call: live.draft.first_open_call(),
                    });
                    continue;
                }
                // A client that ends its stream without a last word has
                // broken its promise. The request is over all the same.
                Woke::Event(event) => event.unwrap_or_else(|| {
                    ModelEvent::Error(ProviderError::new(
                        cineris_base::ErrorKind::Transient,
                        "incomplete_stream",
                        "The response ended before the provider said it was complete.",
                    ))
                }),
            };
            if self.state == TurnState::Backoff {
                // Whatever comes next, the wait is over and the request
                // was sent again.
                self.enter(TurnState::Requesting);
            }
            self.spend_on(&event);
            match event {
                // Timing, which `spend_on` took.
                ModelEvent::Answered => {}
                ModelEvent::Start { .. } => {
                    if self.state == TurnState::Streaming {
                        // A response began over one that never ended: the
                        // earlier one no longer counts.
                        let error = ErrorData::new(
                            ErrorKind::Transient,
                            "incomplete_stream",
                            "The response was started again before it ended.",
                        );
                        if let Err(error) =
                            self.void(turn, &model, &mut live, &mut round, error).await
                        {
                            return self.abandon(round, End::store(&error)).await;
                        }
                    } else {
                        self.enter(TurnState::Streaming);
                    }
                    self.show(turn, &mut live);
                }
                ModelEvent::Retry(info) => {
                    let error = info.error.to_error_data_at(self.clock.now());
                    self.notify(
                        turn,
                        Notice::Retry(RetryNotice {
                            message: format!(
                                "The request failed ({}). Trying again in {} s (attempt {} of {}).",
                                info.error.code,
                                info.wait.as_secs_f32().ceil(),
                                info.attempt,
                                info.max_attempts
                            ),
                            attempt: info.attempt,
                            max_attempts: info.max_attempts,
                            error: error.clone(),
                            next_attempt_at: info.next_attempt_at.to_timestamp(),
                        }),
                    );
                    if let Err(error) = self.void(turn, &model, &mut live, &mut round, error).await
                    {
                        return self.abandon(round, End::store(&error)).await;
                    }
                    self.enter(TurnState::Backoff);
                }
                ModelEvent::SpeedFallback(moved) => {
                    self.sent_at(&moved);
                    self.speed_moved(turn, &model, &moved);
                    if moved.by == SpeedFallbackBy::Harness && self.state == TurnState::Streaming {
                        // The tier refused part-way and the request is sent
                        // again at standard speed: what arrived is void.
                        let error = ErrorData::new(
                            ErrorKind::Transient,
                            moved.reason.as_str(),
                            "The speed tier refused the request part-way. It was sent again at \
                             standard speed.",
                        );
                        if let Err(error) =
                            self.void(turn, &model, &mut live, &mut round, error).await
                        {
                            return self.abandon(round, End::store(&error)).await;
                        }
                        self.enter(TurnState::Requesting);
                    }
                }
                ModelEvent::SpeedCooldown(change) => self.cooldown_changed(turn, &model, &change),
                ModelEvent::Limits(windows) => {
                    let kept = self.usage.set_limits(
                        &model.entry.info.provider,
                        windows,
                        self.clock.now(),
                    );
                    self.stats.record(StatEvent::Limits(kept));
                    self.tell_usage(turn, &model, live.usage.as_ref(), false);
                }
                ModelEvent::Usage(usage) => {
                    live.usage = Some(usage);
                    self.tell_usage(turn, &model, live.usage.as_ref(), false);
                }
                ModelEvent::End {
                    stop,
                    raw_stop,
                    stop_detail,
                    usage,
                } => {
                    self.streaming(turn, &mut live);
                    return Phase::Deciding(Box::new(Response {
                        live,
                        round,
                        stop,
                        raw_stop,
                        stop_detail,
                        usage,
                        prep,
                    }));
                }
                ModelEvent::Error(error) => {
                    return self.failed(turn, &model, live, round, error, prep).await;
                }
                content => {
                    self.streaming(turn, &mut live);
                    match live.draft.apply(content) {
                        Applied::Nothing => {}
                        Applied::Delta(delta) => {
                            live.first_content
                                .get_or_insert_with(|| self.clock.monotonic());
                            self.emit(
                                turn,
                                Event::ItemDelta(ItemDelta {
                                    item_id: live.item_id.clone(),
                                    delta,
                                }),
                            );
                        }
                        // The call is planned at once, and runs now if it
                        // has no side effects.
                        Applied::Call(call) => {
                            live.first_content
                                .get_or_insert_with(|| self.clock.monotonic());
                            round.add_call(call, &mut self.item_ids);
                            round.pump(Stage::Streaming {
                                open_call: live.draft.first_open_call(),
                            });
                        }
                    }
                }
            }
        }
    }

    /// What an event of the response says the turn's time is spent on from
    /// now (`crate::timing`).
    fn spend_on(&mut self, event: &ModelEvent) {
        let now = self.clock.monotonic();
        match event {
            // The provider answered: the wait for the first byte.
            ModelEvent::Answered => self.times.enter(now, Spend::FirstByte),
            ModelEvent::Retry(info) => self.times.retry(now, info.wait),
            ModelEvent::SpeedFallback(moved)
                if moved.by == SpeedFallbackBy::Harness && self.state == TurnState::Streaming =>
            {
                // Sent again at standard speed.
                self.times.enter(now, Spend::Connect);
            }
            // What is not the response itself says nothing of where it is.
            ModelEvent::Usage(_)
            | ModelEvent::Limits(_)
            | ModelEvent::SpeedFallback(_)
            | ModelEvent::SpeedCooldown(_) => {}
            // The response itself, its end and a failure included: it is
            // arriving, or has.
            _ => {
                if self.state != TurnState::Streaming {
                    self.times.enter(now, Spend::Streaming);
                }
            }
        }
    }

    /// Content or an end arrived: the response is streaming, whether or not
    /// the client said that it started.
    fn streaming(&mut self, turn: &Turn, live: &mut Live) {
        if self.state != TurnState::Streaming {
            self.enter(TurnState::Streaming);
        }
        self.show(turn, live);
    }

    /// Tells front ends the assistant item began, once.
    fn show(&self, turn: &Turn, live: &mut Live) {
        if !live.shown {
            live.shown = true;
            self.emit(
                turn,
                Event::ItemStarted(ItemStarted {
                    item_id: live.item_id.clone(),
                    kind: ItemKind::AssistantMessage,
                    tool: None,
                    call_id: None,
                }),
            );
        }
    }

    /// The attempt that was arriving no longer counts. Front ends clear
    /// what they showed of it, the log keeps it as an `attempt` entry, and
    /// the round keeps the runs that started for the next response.
    async fn void(
        &mut self,
        turn: &mut Turn,
        model: &RoleModel,
        live: &mut Live,
        round: &mut Round,
        error: ErrorData,
    ) -> Result<(), StoreError> {
        // What the provider said the attempt used was used: charged now,
        // and carried into the request's statistics event.
        if let Some(usage) = live.usage.take() {
            let record = self.account(turn, model, usage, live);
            live.voided = Some(match live.voided.take() {
                Some(before) => crate::usage::together(before, &record),
                None => record,
            });
        }
        if live.shown {
            self.emit(
                turn,
                Event::ItemReset(ItemRef {
                    item_id: live.item_id.clone(),
                }),
            );
        }
        let attempt = Attempt {
            item_id: live.item_id.clone(),
            attempt: live.attempt,
            error,
            partial: live.draft.parts(Calls::Keep),
        };
        live.draft = Draft::default();
        live.attempt += 1;
        live.first_content = None;
        round.void();
        self.store.append(&turn.id, Record::Attempt(attempt)).await
    }

    /// Ends a turn whose response cannot go on, with nothing of the round
    /// left running.
    async fn abandon(&mut self, mut round: Round, end: End) -> Phase {
        round.void();
        round.discard_kept();
        round.settle().await;
        Phase::Finishing(end)
    }

    /// The request failed and will not be tried again by the client.
    async fn failed(
        &mut self,
        turn: &mut Turn,
        model: &RoleModel,
        mut live: Live,
        mut round: Round,
        error: ProviderError,
        prep: Prep,
    ) -> Phase {
        self.times.enter(self.clock.monotonic(), Spend::Harness);
        if error.kind == cineris_base::ErrorKind::Interrupted {
            return Phase::Interrupted(Some(Box::new((live, round))));
        }
        let data = error.to_error_data_at(self.clock.now());
        if error.kind == cineris_base::ErrorKind::ContextOverflow && !prep.overflowed {
            // The provider says the request does not fit: the budget
            // learns from it, and the loop compacts, once, and sends it
            // again. With automatic compaction off it stops and asks. The
            // request is recorded first, its context record with it: the
            // estimate that was wrong is what explains the refusal (since
            // step 16), with what the provider said it used, which is
            // charged here.
            let usage = live
                .usage
                .take()
                .map(|usage| self.account(turn, model, usage, &live));
            self.request_stat(
                turn,
                model,
                usage,
                StopReason::ContextOverflow,
                Some(data.clone()),
                &live,
            );
            if let Err(error) = self.void(turn, model, &mut live, &mut round, data).await {
                return self.abandon(round, End::store(&error)).await;
            }
            round.discard_kept();
            round.settle().await;
            return self.overflowed(turn, prep);
        }

        let usage = live
            .usage
            .take()
            .map(|usage| self.account(turn, model, usage, &live));
        turn.stop = Some(StopReason::Error);
        self.request_stat(
            turn,
            model,
            usage.clone(),
            StopReason::Error,
            Some(data.clone()),
            &live,
        );
        // None of the response's calls will run, and what ran early was
        // for a response that is not kept.
        let parts = live.draft.parts(Calls::Strip);
        let phase = self.abandon(round, End::failed(data.clone())).await;
        if live.shown {
            // A failure is data: the item ends with what arrived and the
            // error attached.
            let item = self.assistant_item(model, &live, parts, StopReason::Error, usage);
            let item = Item::AssistantMessage(AssistantMessageItem {
                error: Some(data),
                ..item
            });
            if let Err(error) = self.record_shown(turn, item).await {
                return Phase::Finishing(End::store(&error));
            }
        }
        phase
    }

    /// The provider refused the request as too long, for the first time in
    /// this request: what that shows of the ratio is learned, and the loop
    /// compacts and sends it once more, or, with automatic compaction off,
    /// stops and asks (`compaction_needed`).
    fn overflowed(&mut self, turn: &mut Turn, prep: Prep) -> Phase {
        self.main_refused();
        if !self.context.parts.settings.auto_compact {
            turn.stop = Some(StopReason::ContextOverflow);
            return Phase::Finishing(self.overflow_without_compaction());
        }
        Phase::Compacting(Prep {
            overflowed: true,
            ..prep
        })
    }

    // ------------------------------------------------------------ deciding

    #[expect(
        clippy::too_many_lines,
        reason = "one stop reason after another, each a few lines; splitting it would hide the order they are looked at in"
    )]
    async fn decide(&mut self, turn: &mut Turn, response: Response) -> Phase {
        self.enter(TurnState::Deciding);
        self.times.enter(self.clock.monotonic(), Spend::Harness);
        let Response {
            mut live,
            mut round,
            stop,
            raw_stop,
            stop_detail,
            usage,
            prep,
        } = response;
        let model = self.model.clone();
        live.usage = None;
        let usage = self.account(turn, &model, usage, &live);
        let usage_counted = usage.clone();
        self.tell_usage(turn, &model, None, true);
        turn.stop = Some(stop.clone());

        if stop == StopReason::ContextOverflow && !prep.overflowed {
            self.request_stat(turn, &model, Some(usage), stop, None, &live);
            let error = ErrorData::new(
                ErrorKind::ContextOverflow,
                "context_overflow",
                "The request did not fit the model's context window.",
            );
            if let Err(error) = self.void(turn, &model, &mut live, &mut round, error).await {
                return self.abandon(round, End::store(&error)).await;
            }
            round.discard_kept();
            round.settle().await;
            return self.overflowed(turn, prep);
        }

        // Cut off inside a tool call: send the request once more with all
        // the output the model allows.
        let room_to_raise = model
            .max_output
            .is_some_and(|limit| limit < model.entry.info.max_output);
        if stop == StopReason::MaxOutput
            && live.draft.ends_in_unfinished_call()
            && !prep.raised_output
            && room_to_raise
        {
            self.request_stat(turn, &model, Some(usage), stop, None, &live);
            let error = ErrorData::new(
                ErrorKind::ModelStop,
                "max_output",
                "The response reached the output limit inside a tool call. The request was sent \
                 again with a higher limit.",
            );
            if let Err(error) = self.void(turn, &model, &mut live, &mut round, error).await {
                return self.abandon(round, End::store(&error)).await;
            }
            round.discard_kept();
            round.settle().await;
            return Phase::Preparing(Prep {
                raised_output: true,
                ..prep
            });
        }

        // The stops that end the turn as a failure. Their tool calls, if
        // the response has any, do not run.
        let detail = stop_detail
            .as_deref()
            .map_or_else(String::new, |detail| format!(" ({detail})"));
        let failure = match &stop {
            StopReason::Refusal => Some(ErrorData::new(
                ErrorKind::ModelStop,
                "refusal",
                format!("The model declined to answer{detail}."),
            )),
            StopReason::ContentFilter => Some(ErrorData::new(
                ErrorKind::ModelStop,
                "content_filter",
                format!("A content filter stopped the response{detail}."),
            )),
            StopReason::ContextOverflow => Some(ErrorData::new(
                ErrorKind::ContextOverflow,
                "context_overflow",
                "The request does not fit the model's context window, even after compaction.",
            )),
            StopReason::Error | StopReason::Interrupted => Some(ErrorData::new(
                ErrorKind::Provider,
                "response_failed",
                format!("The provider ended the response with an error{detail}."),
            )),
            _ => None,
        };
        let parts = if failure.is_some() {
            round.void();
            live.draft.parts(Calls::Strip)
        } else {
            live.draft.parts(Calls::Keep)
        };

        let item = self.assistant_item(&model, &live, parts, stop.clone(), Some(usage.clone()));
        let item = Item::AssistantMessage(AssistantMessageItem {
            raw_stop: Some(raw_stop.clone()),
            stop_detail,
            truncated: stop == StopReason::MaxOutput,
            error: failure.clone(),
            ..item
        });
        self.request_stat(
            turn,
            &model,
            Some(usage),
            stop.clone(),
            failure.clone(),
            &live,
        );
        if let Err(error) = self.record_shown(turn, item).await {
            return self.abandon(round, End::store(&error)).await;
        }
        // The provider counted the request, and its answer is in the
        // history: the next estimate goes on from here.
        self.context_counted(&usage_counted);
        if let Some(error) = failure {
            return self.abandon(round, End::failed(error)).await;
        }
        if round.has_calls() {
            return Phase::ToolRound(Box::new(round));
        }

        // No call to run. What ran for an earlier attempt and was not asked
        // for again goes.
        round.discard_kept();
        round.settle().await;
        match stop {
            // The provider paused a loop of its own: the response is kept
            // as it is and the next request continues it.
            StopReason::Paused => Phase::Preparing(Prep::default()),
            StopReason::MalformedToolCall => {
                let note = "Your last tool call could not be read, so nothing was run. Write the \
                            call again, with its arguments as one JSON object that fits the \
                            tool's schema.";
                if let Err(error) = self
                    .record_notice(turn, NoticeSource::Harness, note.to_owned())
                    .await
                {
                    return Phase::Finishing(End::store(&error));
                }
                self.guarded(turn, format!("malformed\u{0}{raw_stop}"))
                    .await
            }
            StopReason::MaxOutput => Phase::Finishing(End::failed(ErrorData::new(
                ErrorKind::ModelStop,
                "max_output",
                "The response reached the output limit and was cut off. Send a prompt to \
                 continue.",
            ))),
            _ => {
                // A round of the project's checks when something changed
                // since the last, rounds are left, and the turn's time
                // limits leave time to run them.
                if turn.changed
                    && turn.verify_rounds < self.config.verify_rounds
                    && self.ledger.time_left() != Some(Duration::ZERO)
                {
                    Phase::Verifying
                } else {
                    Phase::Finishing(End::completed())
                }
            }
        }
    }

    // ----------------------------------------------------------- tool round

    async fn tool_round(&mut self, turn: &mut Turn, mut round: Round) -> Phase {
        self.enter(TurnState::ToolRound);
        // The response has ended, so calls with side effects may start, and
        // whatever was kept and is not asked for again is discarded.
        let interrupted = loop {
            round.pump(Stage::Ended);
            self.times
                .round(self.clock.monotonic(), round.running(), round.is_asking());
            if round.is_settled() {
                break false;
            }
            match turn.cancel.race(round.recv()).await {
                Some(report) => round.handle(report),
                None => {
                    // Every started call still gets its result.
                    round.interrupt();
                    round.discard_kept();
                    round.settle().await;
                    break true;
                }
            }
        };

        self.times.enter(self.clock.monotonic(), Spend::Harness);
        // Recorded in the model's call order, whatever order they finished
        // in, and never cut short: every provider rejects a call without a
        // result.
        let stopped = round.stopped().map(str::to_owned);
        let mut signature = String::new();
        if let Err(error) = self.record_results(turn, &mut round, &mut signature).await {
            return Phase::Finishing(End::store(&error));
        }
        if interrupted {
            return Phase::Interrupted(None);
        }
        if let Some(reason) = stopped {
            // The policy refused a call and said to stop: the results are
            // recorded, so the history is whole, and the turn ends here.
            let message =
                format!("A tool call was not allowed, and the turn was stopped: {reason}");
            self.notify(
                turn,
                Notice::Info(InfoNotice {
                    message: message.clone(),
                }),
            );
            return Phase::Finishing(End {
                outcome: TurnOutcome::Denied,
                error: Some(ErrorData::new(
                    ErrorKind::Permission,
                    "permission_denied",
                    message,
                )),
            });
        }
        self.guarded(turn, signature).await
    }

    /// Appends a settled round's results to the log and the context.
    async fn record_results(
        &mut self,
        turn: &mut Turn,
        round: &mut Round,
        signature: &mut String,
    ) -> Result<(), StoreError> {
        let model = self.model.clone();
        let mut recorded = false;
        for finished in round.finish() {
            recorded = true;
            turn.tool_calls += 1;
            self.stats.record(StatEvent::ToolCall(ToolCallStat {
                session: self.shared.info.id.clone(),
                turn: turn.id.clone(),
                tool: finished.item.tool.clone(),
                model: model.entry.info.id.clone(),
                variants: self.epoch_variants(),
                effort: model.effort.clone(),
                outcome: finished.stat,
                duration: finished.duration,
                queued: finished.queued,
                approval: finished.approval,
            }));
            signature.push_str(&finished.signature);
            turn.changed |= !finished.written.is_empty();
            turn.written.extend(finished.written);
            let item = Item::ToolResult(finished.item);
            self.store
                .append(&turn.id, Record::Item(item.clone()))
                .await?;
            self.context.push(&item);
        }
        if recorded {
            // Every result of the round is in the log and in the context:
            // the next request carries them. A round whose results could
            // not all be recorded returned above, and keeps its number.
            self.rounds_recorded += 1;
        }
        turn.written.sort();
        turn.written.dedup();
        Ok(())
    }

    /// Puts a round to the repetition guard, and goes on to the next
    /// request unless the guard stops the turn.
    async fn guarded(&mut self, turn: &Turn, signature: String) -> Phase {
        match self.guard.observe(signature) {
            Repetition::Fine => {}
            Repetition::Warn(count) => {
                let note = format!(
                    "The last {count} rounds made the same tool calls and got the same results. \
                     Repeating them changes nothing. Try another approach, or say what is in the \
                     way."
                );
                if let Err(error) = self
                    .record_notice(turn, NoticeSource::Repetition, note)
                    .await
                {
                    return Phase::Finishing(End::store(&error));
                }
            }
            Repetition::Stop(count) => {
                let message = format!(
                    "The model made the same tool calls {count} times in a row with the same \
                     results, so the turn was stopped. Say how to go on."
                );
                self.notify(
                    turn,
                    Notice::Info(InfoNotice {
                        message: message.clone(),
                    }),
                );
                return Phase::Finishing(End {
                    outcome: TurnOutcome::Limit,
                    error: Some(ErrorData::new(ErrorKind::Limit, "repetition", message)),
                });
            }
        }
        Phase::Preparing(Prep::default())
    }

    // ----------------------------------------------------------- verifying

    async fn verify(&mut self, turn: &mut Turn) -> Phase {
        self.enter(TurnState::Verifying);
        turn.verify_rounds += 1;
        turn.changed = false;
        let started = self.clock.monotonic();
        self.times.enter(started, Spend::Verification);
        let emitter = Arc::clone(&self.env.emitter);
        let turn_id = turn.id.clone();
        let notify = move |notice: VerificationNotice| {
            emitter.emit(Some(&turn_id), Event::Notice(Notice::Verification(notice)));
        };
        // A turn that only verifies has one round, and its failure goes to
        // no model.
        let rounds = if turn.only_verify {
            1
        } else {
            self.config.verify_rounds
        };
        let cx = VerifyCx {
            session: &self.shared.info.id,
            turn: &turn.id,
            number: turn.number,
            round: turn.verify_rounds,
            rounds,
            asked: turn.only_verify,
            written: &turn.written,
            used: turn.verify_used,
            deadline: self.ledger.time_left(),
            cancel: &turn.cancel,
            kill: &turn.kill,
            notify: &notify,
        };
        // Not raced against the turn's token, as a tool round's calls are
        // not: on an interrupt the verifier stops what it runs and returns,
        // and the loop waits for it, so that nothing of a check is left
        // running and what a check changed is recorded for undo.
        let outcome = self.verifier.verify(cx).await;
        let ended = self.clock.monotonic();
        turn.verify_used += ended.saturating_sub(started);
        self.times.enter(ended, Spend::Harness);
        if turn.cancel.is_cancelled() {
            // Nothing was added to the conversation: the history is what it
            // was after the last tool round.
            return Phase::Interrupted(None);
        }
        match outcome {
            VerifyOutcome::Passed => {
                turn.verification = Some(Verification::Passed);
                Phase::Finishing(End::completed())
            }
            VerifyOutcome::Skipped => {
                // A round that ran nothing to a result says nothing new: a
                // failure an earlier round found still stands.
                turn.verification.get_or_insert(Verification::Skipped);
                Phase::Finishing(End::completed())
            }
            VerifyOutcome::Incomplete => {
                turn.verification = Some(Verification::Incomplete);
                Phase::Finishing(End::completed())
            }
            VerifyOutcome::Failed { report } => {
                turn.verification = Some(Verification::Failed);
                if turn.verify_rounds >= rounds {
                    // The round limit: the turn completes and reports the
                    // failing state to the user, and the model is not
                    // sent it (`docs/architecture/15-efficiency.md`).
                    return Phase::Finishing(End::completed());
                }
                match self
                    .record_notice(turn, NoticeSource::Verification, report)
                    .await
                {
                    Ok(()) => Phase::Preparing(Prep::default()),
                    Err(error) => Phase::Finishing(End::store(&error)),
                }
            }
            VerifyOutcome::StillFailing => {
                // Nothing new for the model: the turn completes with what
                // still fails.
                turn.verification = Some(Verification::Failed);
                Phase::Finishing(End::completed())
            }
            VerifyOutcome::Stopped { reason, failing } => {
                turn.verification = Some(if failing {
                    Verification::Failed
                } else {
                    Verification::Incomplete
                });
                // The model is told why, as a stopped tool round tells it
                // in the call's result. A turn that only verifies sent the
                // model nothing, and records nothing.
                if !turn.only_verify {
                    let note = format!(
                        "The project's checks were stopped, and this turn with them: {reason}"
                    );
                    if let Err(error) = self
                        .record_notice(turn, NoticeSource::Verification, note)
                        .await
                    {
                        return Phase::Finishing(End::store(&error));
                    }
                }
                let message = format!("A check stopped the turn: {reason}");
                self.notify(
                    turn,
                    Notice::Info(InfoNotice {
                        message: message.clone(),
                    }),
                );
                Phase::Finishing(End {
                    outcome: TurnOutcome::Denied,
                    error: Some(ErrorData::new(
                        ErrorKind::Permission,
                        "permission_denied",
                        message,
                    )),
                })
            }
        }
    }

    // ------------------------------------------------ finishing, interrupted

    async fn finish(&mut self, turn: &Turn, end: End) -> Option<(Running, Prompt)> {
        self.enter(TurnState::Finishing);
        self.request_span = tracing::Span::none();
        let ended = self.clock.monotonic();
        // The verdict of the turn's checks, whenever a round ran, however
        // the turn then ended (one that ran out of time after a failed round
        // still says so); `skipped` for a turn that completed with none.
        let verification = match (&turn.verification, &end.outcome) {
            (Some(verdict), _) => Some(verdict.clone()),
            (None, TurnOutcome::Completed) => Some(Verification::Skipped),
            (None, _) => None,
        };
        let completed = TurnCompleted {
            verification,
            outcome: end.outcome,
            stop_reason: turn.stop.clone(),
            error: end.error,
            usage: turn.usage.clone(),
            duration_ms: Some(millis(ended.saturating_sub(turn.started))),
        };
        self.conclude(turn, completed, ended).await
    }

    async fn interrupted(&mut self, turn: &mut Turn, partial: Option<Box<(Live, Round)>>) {
        self.enter(TurnState::Interrupted);
        // However the turn got here, everything below it stops, a summary
        // in the background included.
        turn.cancel.cancel();
        self.stop_background();
        if let Some(partial) = partial {
            let (mut live, mut round) = *partial;
            let model = self.model.clone();
            let usage = live
                .usage
                .take()
                .map(|usage| self.account(turn, &model, usage, &live));
            // The request was sent and charged: its statistics event says
            // so, with what it used, so that what the ledger was charged is
            // in the statistics, where other processes' day limits read it
            // (found by the review of step 16's fixes).
            self.record_request(
                turn,
                &model,
                usage.clone(),
                StopReason::Interrupted,
                None,
                &live,
            );
            // The calls whose arguments were complete each get a result:
            // what a run that had started returns, or "interrupted".
            round.interrupt();
            round.discard_kept();
            round.settle().await;
            if live.shown {
                // What arrived is kept and marked interrupted. A call whose
                // arguments were still arriving is left out, and never runs.
                let parts = live.draft.parts(Calls::Keep);
                let item = Item::AssistantMessage(self.assistant_item(
                    &model,
                    &live,
                    parts,
                    StopReason::Interrupted,
                    usage,
                ));
                let mut signature = String::new();
                // If the log cannot be written there is nothing more to do
                // about it here: the turn ends as interrupted either way,
                // and the session is read again before the next one.
                let recorded = match self.record_shown(turn, item).await {
                    Ok(()) => self.record_results(turn, &mut round, &mut signature).await,
                    Err(error) => Err(error),
                };
                if recorded.is_err() {
                    let mut inbox = self.shared.lock();
                    inbox.reload = true;
                    inbox.diverged = true;
                }
            }
        }
        turn.stop = Some(StopReason::Interrupted);
        self.request_span = tracing::Span::none();
        let ended = self.clock.monotonic();
        let completed = TurnCompleted {
            outcome: TurnOutcome::Interrupted,
            stop_reason: Some(StopReason::Interrupted),
            error: None,
            usage: turn.usage.clone(),
            verification: None,
            duration_ms: Some(millis(ended.saturating_sub(turn.started))),
        };
        self.conclude(turn, completed, ended).await;
    }

    /// The end of every turn: the log is told how it ended, front ends get
    /// `turn.completed`, and the session either starts the next queued
    /// prompt or is idle.
    ///
    /// **Only a turn that completed starts the next prompt by itself.**
    /// After any other end the prompts that were waiting are held: they
    /// follow the next prompt the user sends.
    ///
    /// `ended` is when the turn ended, the moment its `duration_ms` was taken
    /// at: its parts are cut there too, so they add up to that.
    async fn conclude(
        &mut self,
        turn: &Turn,
        completed: TurnCompleted,
        ended: Duration,
    ) -> Option<(Running, Prompt)> {
        let stored = self.store.end_turn(&turn.id, &completed).await;
        let store_failed = stored.is_err()
            || completed
                .error
                .as_ref()
                .is_some_and(|error| error.code == "store_failed");
        let completed = match stored {
            Err(error) if completed.error.is_none() && self.state == TurnState::Finishing => {
                TurnCompleted {
                    outcome: TurnOutcome::Failed,
                    error: Some(error.to_error_data()),
                    verification: None,
                    ..completed
                }
            }
            _ => completed,
        };
        let outcome = completed.outcome.clone();
        let now = ended;
        // A turn that only verifies is not the model's: the statistics of
        // the model's turns, the quality signal of a set-up, leave it out.
        if !turn.only_verify {
            self.stats.record(StatEvent::Turn(TurnStat {
                session: self.shared.info.id.clone(),
                turn: turn.id.clone(),
                provider: self.model.entry.info.provider.clone(),
                model: self.model.entry.info.id.clone(),
                variants: self.epoch_variants(),
                effort: self.model.effort.clone(),
                requests: turn.requests,
                tool_calls: turn.tool_calls,
                outcome: outcome.clone(),
                verification: completed.verification.clone(),
                duration: now.saturating_sub(turn.started),
                times: self.times.finish(now),
            }));
        }
        // The session's state changes before the end is told: a front end
        // that acts on `turn.completed` (an undo the moment the idle box
        // shows) finds the turn no longer running (found by the review of
        // the eighth tooling step).
        let next = {
            let mut inbox = self.shared.lock();
            // An interrupt that came as the turn was finishing does not
            // change how the turn ended. It still means stop: nothing
            // follows by itself. The same holds for a session that is
            // being closed.
            let next = if outcome == TurnOutcome::Completed && !turn.cancel.is_cancelled() {
                self.next_turn(&mut inbox)
            } else {
                Self::hold(&mut inbox);
                // A compaction asked for during a turn that failed still
                // runs, as a turn that only compacts: no turn follows to
                // take it (the review of step 23). An interrupt drops it.
                if turn.cancel.is_cancelled() {
                    None
                } else {
                    self.compact_turn(&mut inbox)
                }
            };
            inbox.running = next.as_ref().map(|(running, _)| running.clone());
            if store_failed {
                // Memory is brought back to what the log holds.
                inbox.reload = true;
                inbox.diverged = true;
            }
            next
        };
        let error_code = completed.error.as_ref().map(|error| error.code.clone());
        self.emit(turn, Event::TurnCompleted(completed));
        cineris_base::hook::point(TURN_COMPLETED_TOLD);
        // The turn's `turn.end` hooks, not waited for: what they add waits
        // for the next turn, as any notice does.
        self.turn_ended(turn, &outcome, error_code);
        {
            let inbox = self.shared.lock();
            // Told under the lock, so that a prompt accepted from another
            // thread is not followed by a stale "idle": one accepted since
            // the turn ended is running.
            self.shared.emit_updated(&inbox);
            if inbox.running.is_none() && inbox.queue.is_empty() && inbox.held.is_empty() {
                self.shared
                    .emitter
                    .emit(None, Event::SessionIdle(SessionIdle {}));
            }
        }
        if next.is_none() {
            self.enter(TurnState::Idle);
        }
        next
    }

    /// The prompt that follows a completed turn: a steering message that
    /// found no boundary to be delivered at, then the first queued prompt,
    /// and otherwise a compaction that found none (since step 23).
    fn next_turn(&self, inbox: &mut Inbox) -> Option<(Running, Prompt)> {
        let steering = inbox
            .pending
            .iter()
            .position(|pending| matches!(pending, Pending::Steer(_)));
        let (turn, prompt) = if let Some(position) = steering
            && let Some(Pending::Steer(input)) = inbox.pending.remove(position)
        {
            let prompt = Prompt {
                input,
                delivery: Some(Delivery::Steer),
                compact: None,
                verify: false,
            };
            (inbox.turn_ids.next_id(), prompt)
        } else if let Some(queued) = inbox.queue.pop_front() {
            // A compaction still pending is taken at the queued turn's
            // first boundary, before its request.
            (queued.turn, queued.prompt)
        } else {
            // A compaction asked for during the turn's last request found
            // no boundary: a turn that only compacts, as an idle session's
            // `session/compact` runs.
            return self.compact_turn(inbox);
        };
        let running = Running {
            id: turn,
            cancel: self.shared.cancel.child(),
            kill: CancelToken::new(),
        };
        Some((running, prompt))
    }

    /// A turn that only compacts, for a compaction still pending when a
    /// turn ended with nothing to follow it.
    fn compact_turn(&self, inbox: &mut Inbox) -> Option<(Running, Prompt)> {
        let position = inbox
            .pending
            .iter()
            .position(|pending| matches!(pending, Pending::Compact(_)))?;
        let Some(Pending::Compact(focus)) = inbox.pending.remove(position) else {
            return None;
        };
        let prompt = Prompt {
            input: Vec::new(),
            delivery: None,
            compact: Some(focus),
            verify: false,
        };
        let running = Running {
            id: inbox.turn_ids.next_id(),
            cancel: self.shared.cancel.child(),
            kill: CancelToken::new(),
        };
        Some((running, prompt))
    }

    /// Holds everything that was waiting behind a turn that did not
    /// complete: steering messages that were never delivered, then the
    /// queue, in order.
    fn hold(inbox: &mut Inbox) {
        let (steering, notices): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut inbox.pending)
            .into_iter()
            .partition(|pending| matches!(pending, Pending::Steer(_)));
        inbox.pending = notices.into();
        let mut held = std::mem::take(&mut inbox.held);
        for pending in steering {
            if let Pending::Steer(input) = pending {
                held.push_back(Queued {
                    turn: inbox.turn_ids.next_id(),
                    prompt: Prompt {
                        input,
                        delivery: Some(Delivery::Steer),
                        compact: None,
                        verify: false,
                    },
                });
            }
        }
        held.extend(inbox.queue.drain(..));
        inbox.held = held;
    }

    // -------------------------------------------------------------- helpers

    fn enter(&mut self, next: TurnState) {
        let from = self.state;
        debug_assert!(
            from.may_become(next),
            "the loop went from {from} to {next}, which the table does not have"
        );
        self.state = next;
        self.shared.lock().state = next;
        if let Some(observer) = &self.observer {
            observer.transition(&self.shared.info.id, from, next);
        }
    }

    fn emit(&self, turn: &Turn, event: Event) {
        self.shared.emitter.emit(Some(&turn.id), event);
    }

    fn notify(&self, turn: &Turn, notice: Notice) {
        self.emit(turn, Event::Notice(notice));
    }

    /// Appends an item to the log, and only then to the context: memory
    /// never holds what the log does not.
    async fn record(&mut self, turn: &Turn, item: &Item) -> Result<(), StoreError> {
        self.store
            .append(&turn.id, Record::Item(item.clone()))
            .await?;
        self.context.push(item);
        Ok(())
    }

    /// Records an item and tells front ends it is complete.
    async fn record_shown(&mut self, turn: &Turn, item: Item) -> Result<(), StoreError> {
        self.record(turn, &item).await?;
        self.emit(turn, Event::ItemCompleted(ItemCompleted { item }));
        Ok(())
    }

    /// Adds a message from the harness to the conversation.
    async fn record_notice(
        &mut self,
        turn: &Turn,
        source: NoticeSource,
        text: String,
    ) -> Result<(), StoreError> {
        let item = Item::Notice(NoticeItem {
            id: self.item_ids.next_id(),
            source,
            text,
            created_at: Some(self.clock.now().to_timestamp()),
        });
        self.record_shown(turn, item).await
    }

    fn assistant_item(
        &self,
        model: &RoleModel,
        live: &Live,
        parts: Vec<Part>,
        stop: StopReason,
        usage: Option<RequestUsage>,
    ) -> AssistantMessageItem {
        let info = &model.entry.info;
        AssistantMessageItem {
            id: live.item_id.clone(),
            parts,
            stop_reason: Some(stop),
            raw_stop: None,
            stop_detail: None,
            truncated: false,
            origin: Some(Origin {
                protocol: info.protocol.clone(),
                provider: info.provider.clone(),
                model: info.id.clone(),
                account_scope: None,
            }),
            usage,
            error: None,
            created_at: Some(self.clock.now().to_timestamp()),
        }
    }

    /// The record of what a request used, priced at the speed it ran at.
    /// `timing` is when the request was sent and when its first content
    /// arrived, for a request that has ended.
    fn usage_record(
        &self,
        model: &RoleModel,
        usage: Usage,
        timing: Option<(Duration, Option<Duration>)>,
    ) -> RequestUsage {
        self.usage_record_as(model, ModelRole::Main, usage, timing)
    }

    /// [`usage_record`](Self::usage_record) for a request of any role.
    fn usage_record_as(
        &self,
        model: &RoleModel,
        role: ModelRole,
        usage: Usage,
        timing: Option<(Duration, Option<Duration>)>,
    ) -> RequestUsage {
        let mut record = cineris_llm::usage::request_usage(
            usage,
            &model.entry.info,
            role,
            model.billing.clone(),
            &self.catalog_version,
        );
        if let Some((sent, first_content)) = timing {
            let now = self.clock.monotonic();
            record.duration_ms = Some(millis(now.saturating_sub(sent)));
            record.time_to_first_token_ms =
                first_content.map(|first| millis(first.saturating_sub(sent)));
        }
        record
    }

    /// Counts what a request used: in the session's usage, in the turn's,
    /// and against every account of the ledger.
    fn account(
        &mut self,
        turn: &mut Turn,
        model: &RoleModel,
        usage: Usage,
        live: &Live,
    ) -> RequestUsage {
        let record = self.usage_record(model, usage, Some((live.sent, live.first_content)));
        self.usage.add(&record);
        self.ledger.charge(&record);
        add_request(&mut turn.usage, &record);
        if record.speed_requested.is_none() {
            // It ran at the speed that was asked for: the next fallback is
            // news again.
            self.moves.forget(&ModelRole::Main);
        }
        self.shared.lock().usage = self.usage.snapshot();
        record
    }

    /// Sends `usage.updated`: at most once a frame while a response
    /// arrives, and always when `now` is set.
    fn tell_usage(&mut self, turn: &Turn, model: &RoleModel, running: Option<&Usage>, now: bool) {
        let time = self.clock.monotonic();
        if !now
            && self
                .usage_sent
                .is_some_and(|sent| time.saturating_sub(sent) < self.config.usage_interval)
        {
            return;
        }
        self.usage_sent = Some(time);
        let usage = match running {
            Some(running) => {
                self.usage
                    .with_running(&self.usage_record(model, running.clone(), None))
            }
            None => self.usage.snapshot(),
        };
        self.emit(turn, Event::UsageUpdated(UsageUpdated { usage }));
    }

    /// The statistics event of a request that ended, its context record
    /// with it, and the request's prefix kept for the next one's.
    fn request_stat(
        &mut self,
        turn: &Turn,
        model: &RoleModel,
        usage: Option<RequestUsage>,
        stop: StopReason,
        error: Option<ErrorData>,
        live: &Live,
    ) {
        let tokens = usage.as_ref().map(|usage| usage.tokens.clone());
        self.record_request(turn, model, usage, stop, error, live);
        self.settle_sent();
        if let Some(mut context) = self.pending_context.take() {
            context.usage = tokens;
            self.stats.record(StatEvent::Context(context));
        }
    }

    /// The statistics event of a request, and nothing else: with what its
    /// attempts given up used added to what its last used.
    fn record_request(
        &mut self,
        turn: &Turn,
        model: &RoleModel,
        usage: Option<RequestUsage>,
        stop: StopReason,
        error: Option<ErrorData>,
        live: &Live,
    ) {
        let usage = match (usage, live.voided.clone()) {
            (Some(last), Some(voided)) => Some(crate::usage::together(voided, &last)),
            (usage, voided) => usage.or(voided),
        };
        let after = self.times.model_time(self.clock.monotonic());
        let spent = |at: usize| millis(after[at].saturating_sub(live.model_before[at]));
        let attempts = live.attempt;
        // The span is let go here, so that it closes as its request ends,
        // and not when the next request replaces it (found by the review of
        // step 16).
        let span = std::mem::replace(&mut self.request_span, tracing::Span::none());
        span.record("attempts", attempts);
        drop(span);
        self.stats.record(StatEvent::Request(RequestStat {
            session: self.shared.info.id.clone(),
            turn: turn.id.clone(),
            at: self.clock.now(),
            provider: model.entry.info.provider.clone(),
            model: model.entry.info.id.clone(),
            role: ModelRole::Main,
            purpose: RequestPurpose::Turn,
            started_in: None,
            variants: self.epoch_variants(),
            effort: model.effort.clone(),
            usage,
            time: cineris_protocol::usage::ModelTime {
                connect_ms: spent(0),
                first_byte_ms: spent(1),
                streaming_ms: spent(2),
                retry_ms: spent(3),
                summary_ms: 0,
            },
            stop,
            error,
            retries: attempts.saturating_sub(1),
            charged_to: self.ledger.charged_to(),
        }));
    }

    /// A request runs, or ran, at another speed than its role is set to.
    fn speed_moved(&mut self, turn: &Turn, model: &RoleModel, moved: &SpeedMove) {
        if self.moves.is_news(&ModelRole::Main, moved) {
            self.notify(
                turn,
                Notice::SpeedFallback(fallback_notice(&ModelRole::Main, &model.entry.info, moved)),
            );
        }
        self.tell_speed();
    }

    /// A speed tier's cooldown began or ended.
    fn cooldown_changed(&mut self, turn: &Turn, model: &RoleModel, change: &CooldownChange) {
        self.notify(
            turn,
            Notice::SpeedCooldown(cooldown_notice(&model.entry.info, change)),
        );
        if change.state == CooldownState::Ended {
            self.moves.forget(&ModelRole::Main);
        }
        self.tell_speed();
    }

    /// Keeps the session state's speed in effect current: when what a
    /// role's next request will ask for has changed, front ends are told.
    fn tell_speed(&mut self) {
        let inbox = self.shared.lock();
        let roles: Vec<RoleState> = self.shared.state(&inbox).roles.into_values().collect();
        if roles != self.told_roles {
            self.told_roles = roles;
            self.shared.emit_updated(&inbox);
        }
    }
}
