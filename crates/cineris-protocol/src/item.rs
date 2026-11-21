//! Items: what front ends display and what the session log stores
//! (`docs/architecture/11-rpc-protocol.md`, "Events").
//!
//! An item is created inside a turn and never changes once it is complete.
//! `item.completed` carries it in full and is authoritative.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ErrorData;
use crate::ids::{ArtifactId, CallId, ItemId, Timestamp};
use crate::message::{Origin, Part};
use crate::open::{open_enum, open_union};
use crate::usage::RequestUsage;

open_enum! {
    /// Why a response from the model ended. Each provider's own value is
    /// mapped onto this set, and kept beside it as `raw_stop`.
    pub enum StopReason {
        /// The model finished.
        End => "end",
        /// The model asked for tools to be run.
        ToolCalls => "tool_calls",
        /// The response reached the output limit.
        MaxOutput => "max_output",
        /// The request did not fit the context window.
        ContextOverflow => "context_overflow",
        /// The provider paused a server-side loop. The harness continues it.
        Paused => "paused",
        /// The model declined.
        Refusal => "refusal",
        /// A content filter stopped the response.
        ContentFilter => "content_filter",
        /// The model wrote a tool call that could not be read.
        MalformedToolCall => "malformed_tool_call",
        /// The request failed. The item's `error` says how.
        Error => "error",
        /// The user or a signal interrupted the response. Set by the
        /// harness, never by a provider.
        Interrupted => "interrupted",
    }
}

open_enum! {
    /// When a prompt sent during a running turn is delivered.
    pub enum Delivery {
        /// At the next safe boundary of the running turn.
        Steer => "steer",
        /// As a new turn, after the running one completes.
        Queue => "queue",
    }
}

open_enum! {
    /// How a tool call ended. Every started call ends with exactly one result.
    pub enum ToolOutcome {
        /// The tool ran and succeeded.
        Success => "success",
        /// The tool failed, or its arguments were invalid.
        Error => "error",
        /// The call was not allowed to run.
        Denied => "denied",
        /// The call was cancelled. The result holds the output so far.
        Interrupted => "interrupted",
    }
}

open_enum! {
    /// Why the harness added a notice to the conversation.
    pub enum NoticeSource {
        /// Instruction files changed.
        Instructions => "instructions",
        /// The user edited the plan.
        Plan => "plan",
        /// The same call keeps repeating without effect.
        Repetition => "repetition",
        /// The project's checks failed.
        Verification => "verification",
        /// A limit was reached.
        Limit => "limit",
        /// Something else the model needs to know.
        Harness => "harness",
        /// The repository map: an epoch's view of the workspace, given once
        /// in it.
        Map => "map",
        /// The project profile changed since the model was given it
        /// (since step 17).
        Profile => "profile",
        /// What extensions give the system prompt changed since the model
        /// was given it: the index of skills and tool sources, given whole,
        /// or an extension's text, withdrawn (since step 28).
        Extensions => "extensions",
        /// Text a hook added to the conversation: a `prompt.submit` hook's
        /// `context`, after the prompt it was given, under a line that names
        /// the hook (since step 28).
        Hook => "hook",
    }
}

/// A hook, named where what it answered shows: a hook that asked, refused
/// or replaced something (since step 28,
/// `docs/architecture/12-extensions.md`, "Tier 3: hooks").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HookReason {
    /// The hook's name: its `[hooks.<name>]`, or the one an extension's
    /// script hook is shown by.
    pub hook: String,
    /// What it gave as its reason, or how it failed. Absent when it gave
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Arguments a `tool.before` hook put in place of the ones the model wrote
/// (since step 28). Such a call is always put to the user, who may allow it
/// once or refuse it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReplacedArguments {
    /// The hook that replaced them.
    pub hook: String,
    /// The arguments the call runs with: the hook's.
    pub arguments: Value,
    /// The arguments the model wrote, where the message that carries the
    /// call is not at hand: in an approval request. Absent in a tool
    /// result, whose call in the assistant message has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<Value>,
    /// What the hook gave as its reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A message from the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct UserMessageItem {
    /// The item's id.
    pub id: ItemId,
    /// What the user sent: text and images.
    pub parts: Vec<Part>,
    /// How it was delivered, when it was sent during a running turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
    /// When it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamp>,
}

/// One response from the model: text, reasoning and tool calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AssistantMessageItem {
    /// The item's id.
    pub id: ItemId,
    /// The content, in the model's order.
    pub parts: Vec<Part>,
    /// Why the response ended. Absent only on an item that is still streaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    /// The provider's own stop value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_stop: Option<String>,
    /// More about the stop where the provider gave it, such as the category
    /// of a refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_detail: Option<String>,
    /// Whether the response was cut off by the output limit.
    #[serde(default)]
    pub truncated: bool,
    /// What produced the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// What the request used and cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RequestUsage>,
    /// The failure, when `stop_reason` is `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorData>,
    /// When it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamp>,
}

/// A reference to a stored artifact, such as the full output of a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ArtifactRef {
    /// The artifact's id.
    pub id: ArtifactId,
    /// What it holds, in a few words.
    pub label: String,
    /// Its size in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

/// The result of one tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ToolResultItem {
    /// The item's id.
    pub id: ItemId,
    /// The call this answers.
    pub call_id: CallId,
    /// The tool's name.
    pub tool: String,
    /// How the call ended.
    pub outcome: ToolOutcome,
    /// What the model is shown, after the output policy.
    pub content: Vec<Part>,
    /// A structured payload for front ends, whose shape belongs to the tool:
    /// a diff, an exit status, match counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Where the full output is, when the content was shortened.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    /// Milliseconds the call took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// When it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamp>,
    /// The arguments a hook put in place of the model's, when one did and
    /// the call was put to the user with them (since step 28): what ran, if
    /// it ran, and which hook chose it. The model's own are in its call.
    /// (Boxed: it is rare, and every item would otherwise be its size.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced: Option<Box<ReplacedArguments>>,
}

/// A message the harness added to the conversation. The model sees it.
///
/// This is not the `notice` event ([`Notice`](crate::event::Notice)), which
/// tells front ends about something transient and is never sent to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct NoticeItem {
    /// The item's id.
    pub id: ItemId,
    /// Why it was added.
    pub source: NoticeSource,
    /// What the model is told.
    pub text: String,
    /// When it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamp>,
}

/// A compaction summary. It starts a context epoch's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SummaryItem {
    /// The item's id.
    pub id: ItemId,
    /// The summary.
    pub text: String,
    /// The last item the summary covers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covers_through: Option<ItemId>,
    /// What the summarising request used and cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RequestUsage>,
    /// When it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamp>,
}

open_union! {
    /// One item of a session.
    pub enum Item [kind] {
        /// A message from the user.
        UserMessage(UserMessageItem) => "user_message",
        /// A response from the model.
        AssistantMessage(AssistantMessageItem) => "assistant_message",
        /// The result of a tool call.
        ToolResult(ToolResultItem) => "tool_result",
        /// A message the harness added to the conversation.
        Notice(NoticeItem) => "notice",
        /// A compaction summary.
        Summary(SummaryItem) => "summary",
    }
}

impl Item {
    /// The item's id. `None` for a kind this version does not know whose
    /// object has no string `id`.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::UserMessage(item) => Some(item.id.as_str()),
            Self::AssistantMessage(item) => Some(item.id.as_str()),
            Self::ToolResult(item) => Some(item.id.as_str()),
            Self::Notice(item) => Some(item.id.as_str()),
            Self::Summary(item) => Some(item.id.as_str()),
            Self::Unknown(unknown) => unknown.fields.get("id").and_then(Value::as_str),
        }
    }
}

open_enum! {
    /// Where a plan item stands.
    pub enum PlanStatus {
        /// Not started.
        Pending => "pending",
        /// Being worked on.
        Active => "active",
        /// Finished.
        Done => "done",
        /// Left out, by the agent or by the user.
        Skipped => "skipped",
    }
}

/// One task of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanItem {
    /// The task's id, stable across edits.
    pub id: crate::ids::PlanItemId,
    /// The task.
    pub text: String,
    /// Where it stands.
    pub status: PlanStatus,
}

/// The plan for the current work: a task list that the agent keeps and the
/// user can see and change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Plan {
    /// The tasks, in order.
    #[serde(default)]
    pub items: Vec<PlanItem>,
    /// Goes up by one with every change, so a client can tell which of two
    /// plans is newer.
    #[serde(default)]
    pub revision: u64,
}
