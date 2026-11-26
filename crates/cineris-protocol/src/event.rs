//! Events: everything the core tells its front ends
//! (`docs/architecture/11-rpc-protocol.md`, "Events").
//!
//! In-process front ends receive [`SessionEvent`] values on a channel. Over
//! RPC each one is the `params` of a `session/event` notification. The two
//! carry the same thing.

use serde::{Deserialize, Serialize};

use crate::error::ErrorData;
use crate::ids::{CallId, EntryId, ItemId, SessionId, Timestamp, TurnId};
use crate::item::{ArtifactRef, Delivery, HookReason, Item, Plan, PromptCommand, StopReason};
use crate::jsonrpc::RequestId;
use crate::message::Part;
use crate::open::{open_enum, open_union};
use crate::project::ProjectEntry;
use crate::session::{SandboxStatus, SessionState};
use crate::speed::{SpeedCooldown, SpeedFallback};
use crate::usage::{SessionUsage, UsageTotals};

/// A turn began.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TurnStarted {}

open_enum! {
    /// The kind of an item, where only the kind is needed.
    pub enum ItemKind {
        /// A message from the user.
        UserMessage => "user_message",
        /// A response from the model.
        AssistantMessage => "assistant_message",
        /// The result of a tool call.
        ToolResult => "tool_result",
        /// A message the harness added to the conversation.
        Notice => "notice",
        /// A compaction summary.
        Summary => "summary",
    }
}

/// An item began. Its content follows as deltas, and then in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemStarted {
    /// The item.
    pub item_id: ItemId,
    /// Its kind.
    pub kind: ItemKind,
    /// For a tool result: the tool that is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// For a tool result: the call it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<CallId>,
}

/// More text of the item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TextDelta {
    /// The text to append.
    pub text: String,
}

/// The model began a tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ToolCallDelta {
    /// The call.
    pub call_id: CallId,
    /// The tool's name.
    pub name: String,
}

/// More of a tool call's arguments: a fragment of JSON text that is not
/// valid JSON until the call is complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ToolArgsDelta {
    /// The call.
    pub call_id: CallId,
    /// The fragment to append.
    pub json: String,
}

open_union! {
    /// Streaming content for an item. A display convenience: deltas may be
    /// merged or dropped for a slow client, and `item.completed` is what counts.
    pub enum Delta [kind] {
        /// More of the assistant's text.
        Text(TextDelta) => "text",
        /// More of the assistant's visible reasoning.
        Reasoning(TextDelta) => "reasoning",
        /// The model began a tool call.
        ToolCall(ToolCallDelta) => "tool_call",
        /// More of a tool call's arguments.
        ToolArgs(ToolArgsDelta) => "tool_args",
        /// More output from a running tool.
        ToolOutput(TextDelta) => "tool_output",
    }
}

/// Streaming content for an item. Sent only to clients that declared `deltas`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemDelta {
    /// The item.
    pub item_id: ItemId,
    /// The content.
    pub delta: Delta,
}

/// Names an item: the payload of `item.reset` and `item.resync`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemRef {
    /// The item.
    pub item_id: ItemId,
}

/// An item is complete. This is the authoritative content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemCompleted {
    /// The item, in full.
    pub item: Item,
}

/// The session's usage changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct UsageUpdated {
    /// The session's usage now.
    pub usage: SessionUsage,
}

open_enum! {
    /// Who made a change.
    pub enum Actor {
        /// The agent.
        Agent => "agent",
        /// A user.
        User => "user",
        /// The harness itself.
        Harness => "harness",
    }
}

/// The plan changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanUpdated {
    /// The plan now.
    pub plan: Plan,
    /// Who changed it.
    pub by: Actor,
}

/// Project profile or memory entries were proposed or changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProjectUpdated {
    /// The entries that are new or changed, as they are now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ProjectEntry>,
    /// The entries that were deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<EntryId>,
}

/// Notice: a request failed and will be tried again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RetryNotice {
    /// The notice in words.
    pub message: String,
    /// Which attempt comes next, counting from 1.
    pub attempt: u32,
    /// The most attempts a request gets.
    pub max_attempts: u32,
    /// Why the last attempt failed.
    pub error: ErrorData,
    /// When the next attempt starts.
    pub next_attempt_at: Timestamp,
}

open_enum! {
    /// Where a compaction stands.
    pub enum CompactionState {
        /// It began.
        Started => "started",
        /// It finished, and a new context epoch began.
        Finished => "finished",
        /// It failed. The session carries on with what it had.
        Failed => "failed",
    }
}

/// Notice: a compaction started, finished or failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CompactionNotice {
    /// The notice in words.
    pub message: String,
    /// Where it stands.
    pub state: CompactionState,
    /// Whether it runs in the background while the turn carries on.
    #[serde(default)]
    pub background: bool,
    /// The estimated size of the conversation before, in tokens, on a
    /// compaction that finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_tokens: Option<u64>,
    /// The estimated size of the new context epoch, in tokens, on a
    /// compaction that finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_tokens: Option<u64>,
}

open_enum! {
    /// Which limit was reached.
    pub enum LimitKind {
        /// Requests in one turn.
        Requests => "requests",
        /// Money.
        Cost => "cost",
        /// Tokens.
        Tokens => "tokens",
        /// Wall-clock time.
        Time => "time",
    }
}

/// Notice: a configured limit was reached, or is near.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LimitNotice {
    /// The notice in words.
    pub message: String,
    /// Which limit.
    pub limit: LimitKind,
    /// Whether it was reached. `false` is a warning that it is near.
    pub reached: bool,
}

/// Notice: something about the sandbox the user should know: it is not
/// enforcing, a command ran outside it, a protected path changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SandboxNotice {
    /// The notice in words.
    pub message: String,
    /// Whether the sandbox is enforcing.
    pub status: SandboxStatus,
}

/// Notice: instruction files changed and will be re-read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct InstructionsNotice {
    /// The notice in words.
    pub message: String,
    /// The files, relative to the workspace root where they are inside it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

/// Notice: anything else worth telling the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct InfoNotice {
    /// The notice in words.
    pub message: String,
}

/// Notice: a prompt given back to the front end, for its editor, and never
/// sent to the model (since step 28). A `prompt.submit` hook refused it,
/// and `hooks` names each that did, with its reason; or the turn was
/// interrupted while its hooks were asked, and `hooks` is empty. A prompt
/// that would have started a turn ends that turn `denied` (or
/// `interrupted`); a steering message is dropped, and the running turn goes
/// on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PromptReturnedNotice {
    /// The notice in words: which hooks refused the prompt, and why.
    pub message: String,
    /// The prompt, as it was sent.
    pub parts: Vec<Part>,
    /// How it was sent while a turn ran, as its user message would have
    /// said: `steer` for a steering message, whether it was refused at a
    /// boundary of the running turn or as the prompt of a turn of its own
    /// (one that found no boundary); `queue` for a queued prompt. Absent
    /// for a prompt sent to an idle session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
    /// The hooks that refused it, each with its reason or its failure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookReason>,
    /// The command the prompt came from, when it was a command's (since
    /// step 28): a front end puts back what was typed, `/name args`, in
    /// place of `parts`, its expanded text. Sent, as `parts` are, to the
    /// client that sent the prompt alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<PromptCommand>,
    /// The ticket the server gave the prompt when it handed it to the
    /// session's loop, carried back untouched: what the server knows the
    /// prompt's sender by. **Never on the wire:** it is not written, and a
    /// value read is always `None`.
    #[serde(skip)]
    pub ticket: Option<u64>,
}

open_union! {
    /// Something transient that front ends should show. It is never sent to
    /// the model and is not an item.
    ///
    /// Every notice has a `message` in words, so a client can show a kind of
    /// notice it does not know.
    pub enum Notice [kind] {
        /// A request failed and will be tried again.
        Retry(RetryNotice) => "retry",
        /// A compaction started, finished or failed.
        Compaction(CompactionNotice) => "compaction",
        /// A configured limit was reached or is near.
        Limit(LimitNotice) => "limit",
        /// Something about the sandbox.
        Sandbox(SandboxNotice) => "sandbox",
        /// Instruction files changed.
        Instructions(InstructionsNotice) => "instructions",
        /// A request ran at a different speed from the one asked for.
        SpeedFallback(SpeedFallback) => "speed_fallback",
        /// A speed tier's cooldown began or ended.
        SpeedCooldown(SpeedCooldown) => "speed_cooldown",
        /// Anything else.
        Info(InfoNotice) => "info",
        /// The project's checks: a round starting or ended, or checks
        /// offered (since step 19).
        Verification(VerificationNotice) => "verification",
        /// A prompt given back, not sent to the model: a hook refused it
        /// (since step 28).
        PromptReturned(PromptReturnedNotice) => "prompt_returned",
    }
}

impl Notice {
    /// The notice in words. `None` only for a kind this version does not
    /// know that broke the rule and has no `message`.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Retry(notice) => Some(&notice.message),
            Self::Compaction(notice) => Some(&notice.message),
            Self::Limit(notice) => Some(&notice.message),
            Self::Sandbox(notice) => Some(&notice.message),
            Self::Instructions(notice) => Some(&notice.message),
            Self::SpeedFallback(notice) => Some(&notice.message),
            Self::SpeedCooldown(notice) => Some(&notice.message),
            Self::Info(notice) => Some(&notice.message),
            Self::Verification(notice) => Some(&notice.message),
            Self::PromptReturned(notice) => Some(&notice.message),
            Self::Unknown(unknown) => unknown.message(),
        }
    }
}

open_enum! {
    /// How a request from the server to its clients was settled.
    pub enum Resolution {
        /// Another client answered it.
        Answered => "answered",
        /// Nobody answered in time, and the default was applied.
        TimedOut => "timed_out",
        /// The turn was interrupted before anybody answered.
        Cancelled => "cancelled",
    }
}

/// A request from the server (an approval, a question, a trust prompt) was
/// settled without this client. The client withdraws its prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RequestResolved {
    /// The JSON-RPC id of the request, as this client received it.
    pub request_id: RequestId,
    /// How it was settled.
    pub resolution: Resolution,
}

open_enum! {
    /// How a turn ended. It and a headless run's exit code always agree.
    pub enum TurnOutcome {
        /// The model ended normally.
        Completed => "completed",
        /// It ended with an error.
        Failed => "failed",
        /// The user or a signal interrupted it.
        Interrupted => "interrupted",
        /// A configured limit stopped it.
        Limit => "limit",
        /// A denied permission stopped it.
        Denied => "denied",
    }
}

open_enum! {
    /// What the project's checks said at the end of a turn.
    pub enum Verification {
        /// They passed.
        Passed => "passed",
        /// A check failed in the turn and had not passed when the turn
        /// ended: at the round limit, after a round with nothing new, or a
        /// turn that then ended another way.
        Failed => "failed",
        /// They did not run to a result: no files changed, no check is
        /// known, the mode is not `auto`, the project is not trusted, the
        /// level is `read-only`, or none had time or was allowed.
        Skipped => "skipped",
        /// None failed, and some did not run to a result: blocked by the
        /// sandbox, stopped at their time limit, skipped or not allowed
        /// (since step 19).
        Incomplete => "incomplete",
    }
}

open_enum! {
    /// Which moment of verification a [`VerificationNotice`] tells of.
    pub enum VerificationPhase {
        /// A round is starting: the checks it will run.
        Started => "started",
        /// A round ended: how each check ended, and the verdict.
        Finished => "finished",
        /// The checks did not run, and are offered: `verify.mode` is
        /// `ask`.
        Offered => "offered",
    }
}

open_enum! {
    /// Where a check came from.
    pub enum CheckSource {
        /// The user's own `verify.commands`.
        User => "user",
        /// A test or check command of the project profile that the user
        /// accepted.
        Profile => "profile",
    }
}

open_enum! {
    /// How one check ended.
    pub enum CheckOutcome {
        /// It exited with success.
        Passed => "passed",
        /// It failed: the model is told.
        Failed => "failed",
        /// It ran past its time limit and was stopped, and is not taken
        /// for the model's doing.
        TimedOut => "timed_out",
        /// It failed, and passed when run again: it is known to be flaky.
        Flaky => "flaky",
        /// The sandbox blocked something it needed, such as the network.
        Blocked => "blocked",
        /// It was not started, for its time: it usually takes longer than
        /// the limit, or too little of it was left.
        Skipped => "skipped",
        /// It may not run unasked, or could not be started.
        NotRun => "not_run",
    }
}

/// One check, as a [`VerificationNotice`] tells of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CheckReport {
    /// The command line.
    pub command: String,
    /// The directory it runs in, relative to the workspace root. Absent:
    /// the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Where it came from.
    pub source: CheckSource,
    /// The project profile's entry it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<EntryId>,
    /// How it ended. Absent in a notice of a round that is starting, and
    /// of checks that are offered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CheckOutcome>,
    /// Its exit code, when it exited with one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// How long it ran, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// What else to say of it, in words: why it was not run, what it left
    /// running, what the sandbox blocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// All of what it printed, when the notice to the model held less.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactRef>,
}

/// Notice: the project's checks, at the end of a turn that changed files
/// (`docs/architecture/15-efficiency.md`, "Automatic verification"; since
/// step 19).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VerificationNotice {
    /// The notice in words.
    pub message: String,
    /// Which moment it tells of.
    pub phase: VerificationPhase,
    /// The round, counted from 1 in its turn. 0 in a notice that offers
    /// the checks.
    pub round: u32,
    /// The most rounds the turn gets.
    pub rounds: u32,
    /// The checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<CheckReport>,
    /// The turn's verdict so far, when a round ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verification>,
}

/// The turn is over and nothing automatic remains: no retry pending, no
/// compaction running, no tool running. Clients wait on this one event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TurnCompleted {
    /// How it ended.
    pub outcome: TurnOutcome,
    /// Why the last response ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    /// The failure, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorData>,
    /// What the turn used and cost.
    #[serde(default)]
    pub usage: UsageTotals,
    /// What the project's checks said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<Verification>,
    /// Milliseconds the turn took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// The session has no running turn and no queued prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SessionIdle {}

/// Session metadata changed: title, models, speed, level, queue length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SessionUpdated {
    /// The session's state now.
    pub session: SessionState,
}

open_union! {
    /// One event of a session.
    pub enum Event [type] {
        /// A turn began.
        TurnStarted(TurnStarted) => "turn.started",
        /// An item began.
        ItemStarted(ItemStarted) => "item.started",
        /// Streaming content for an item.
        ItemDelta(ItemDelta) => "item.delta",
        /// The partial content of an item was discarded, because its request
        /// is being retried. The client clears what it has shown.
        ItemReset(ItemRef) => "item.reset",
        /// The final item, in full. Authoritative.
        ItemCompleted(ItemCompleted) => "item.completed",
        /// Deltas were dropped for a slow client. It fetches the item with
        /// `session/read`.
        ItemResync(ItemRef) => "item.resync",
        /// Token and cost totals, cache hit rate and rate-limit state.
        UsageUpdated(UsageUpdated) => "usage.updated",
        /// The plan changed, by the agent or by a user.
        PlanUpdated(PlanUpdated) => "plan.updated",
        /// Project profile or memory entries were proposed or changed.
        ProjectUpdated(ProjectUpdated) => "project.updated",
        /// Something transient to show: a retry, a compaction, a limit, a
        /// sandbox report, changed instructions, a change of speed.
        Notice(Notice) => "notice",
        /// A server request was answered by another client or timed out.
        RequestResolved(RequestResolved) => "request.resolved",
        /// The turn is over and nothing automatic remains.
        TurnCompleted(TurnCompleted) => "turn.completed",
        /// The session has no running turn and no queued prompt.
        SessionIdle(SessionIdle) => "session.idle",
        /// Session metadata changed.
        SessionUpdated(SessionUpdated) => "session.updated",
        /// A reload of the configuration changed what is in force for the
        /// session, or what waits. Added in step 24.
        ConfigChanged(ConfigChanged) => "config.changed",
    }
}

/// What a reload of the configuration changed for a session, whoever asked
/// for it: a front end, another client, or the look the core takes at the
/// start of a turn. Each client of the session is sent it, so that one that
/// keeps settings of its own (a terminal's theme and key bindings) takes
/// them up whoever found the change. Added in step 24.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ConfigChanged {
    /// Keys whose new values are in force.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applied: Vec<String>,
    /// Keys whose new values wait; `notes` says for what.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
    /// What was wrong with a file. The configuration in force stays.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// What to tell the person, a sentence each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// An event with its envelope: which session, where in its sequence, and
/// which turn. This is the `params` of a `session/event` notification.
///
/// On the wire the event's own fields sit beside the envelope's:
/// `{"session_id": …, "seq": 412, "turn_id": …, "type": "item.delta", …}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    /// The session.
    pub session_id: SessionId,
    /// Goes up by one per event per session. A gap means events were missed,
    /// and the client reads the session to catch up.
    pub seq: u64,
    /// The turn the event belongs to. Absent for events outside a turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    /// The event.
    #[serde(flatten)]
    pub event: Event,
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for SessionEvent {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SessionEvent".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let session_id = generator.subschema_for::<SessionId>();
        let turn_id = generator.subschema_for::<TurnId>();
        let event = generator.subschema_for::<Event>();
        schemars::json_schema!({
            "description": "An event with its envelope: which session, where in its sequence, and which turn. The event's own fields sit beside the envelope's.",
            "allOf": [
                {
                    "type": "object",
                    "properties": {
                        "session_id": session_id,
                        "seq": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Goes up by one per event per session. A gap means events were missed.",
                        },
                        "turn_id": turn_id,
                    },
                    "required": ["session_id", "seq"],
                },
                event,
            ],
        })
    }
}
