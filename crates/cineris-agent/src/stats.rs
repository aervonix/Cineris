//! The statistics events the loop emits
//! (`docs/architecture/03-agent-loop.md`, "What is recorded for
//! adaptation").
//!
//! At the end of each request, each tool call and each turn the loop hands
//! one small event to a [`Stats`]. `cineris-stats` records them; the core
//! connects the two. An event holds no prompt and no file content.

use std::time::Duration;

use cineris_base::UtcTime;
use cineris_llm::Variants;
use cineris_protocol::event::{TurnOutcome, Verification};
use cineris_protocol::model::ModelRole;
use cineris_protocol::usage::{ModelTime, ProviderLimits, RequestUsage, TokenUsage};
use cineris_protocol::{ErrorData, ModelId, ProviderId, SessionId, StopReason, TurnId};

pub use crate::context::PrefixDifference;
pub use crate::timing::TurnTimes;

/// Why a request was made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RequestPurpose {
    /// One of the turn's own requests, to the main model.
    #[default]
    Turn,
    /// A summary for a compaction, to the `fast` role's model or the main
    /// one.
    Summary,
}

/// One request to a model ended.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestStat {
    /// The session.
    pub session: SessionId,
    /// The turn.
    pub turn: TurnId,
    /// When the request ended.
    pub at: UtcTime,
    /// The provider.
    pub provider: ProviderId,
    /// The model.
    pub model: ModelId,
    /// The role it was made in.
    pub role: ModelRole,
    /// Why it was made.
    pub purpose: RequestPurpose,
    /// For a summary taken in the background, the turn that started it,
    /// when that is not `turn`: the turn that counted it, in which its
    /// usage, its cost and its charge to the ledger are.
    pub started_in: Option<TurnId>,
    /// The per-model variants in effect.
    pub variants: Variants,
    /// The reasoning level in effect, as the model names it.
    pub effort: Option<String>,
    /// What the request used, at which speed, and what it cost. `None` when
    /// it failed before any usage was reported.
    pub usage: Option<RequestUsage>,
    /// Where its time went. All zero for a summary taken beside the turn,
    /// whose time is its own.
    pub time: ModelTime,
    /// Why the response ended.
    pub stop: StopReason,
    /// The failure, when the request failed.
    pub error: Option<ErrorData>,
    /// How many attempts were given up before this one.
    pub retries: u32,
    /// The account spent elsewhere too that this request was charged to,
    /// by its id ([`CatchUp::id`](crate::CatchUp::id)): a day's purse, which
    /// counts the request as it is made and so does not read it back from
    /// the statistics. `None` when the request was charged to no such
    /// account.
    pub charged_to: Option<String>,
}

/// How a tool call ended, for the success rates of tools and schemas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolStatOutcome {
    /// The tool ran and succeeded.
    Ok,
    /// The arguments did not fit the tool, or were not JSON.
    InvalidArguments,
    /// The model named a tool that does not exist.
    UnknownTool,
    /// The tool ran and failed.
    Failed,
    /// The call was not allowed to run.
    Denied,
    /// The call was cancelled.
    Interrupted,
}

/// One tool call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallStat {
    /// The session.
    pub session: SessionId,
    /// The turn.
    pub turn: TurnId,
    /// The tool's name.
    pub tool: String,
    /// The model that made the call.
    pub model: ModelId,
    /// The per-model variants in effect.
    pub variants: Variants,
    /// The reasoning level in effect, as the model names it.
    pub effort: Option<String>,
    /// How the call ended.
    pub outcome: ToolStatOutcome,
    /// How long it ran. Zero for a call that never ran.
    pub duration: Duration,
    /// How long it waited, once allowed, before it ran: for a slot, for an
    /// earlier call, or for the response to end.
    pub queued: Duration,
    /// How long it waited for the user's answer.
    pub approval: Duration,
}

/// One turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnStat {
    /// The session.
    pub session: SessionId,
    /// The turn.
    pub turn: TurnId,
    /// The main model's provider.
    pub provider: ProviderId,
    /// The main model.
    pub model: ModelId,
    /// The variants in effect at the turn's end.
    pub variants: Variants,
    /// The reasoning level in effect at its end.
    pub effort: Option<String>,
    /// How many requests the turn made.
    pub requests: u64,
    /// How many tool calls it recorded a result for.
    pub tool_calls: u64,
    /// How it ended.
    pub outcome: TurnOutcome,
    /// What the project's checks said.
    pub verification: Option<Verification>,
    /// How long it took.
    pub duration: Duration,
    /// Where that time went: parts that add up to it.
    pub times: TurnTimes,
}

/// How many tokens of a request each part of it is estimated at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextSources {
    /// The system prompt.
    pub system: u64,
    /// The tool definitions.
    pub tools: u64,
    /// The summary that began the epoch, when it began with one.
    pub summary: u64,
    /// What the user wrote.
    pub user: u64,
    /// What the model answered, by its text and its calls.
    pub assistant: u64,
    /// What tools returned.
    pub tool_results: u64,
    /// What the harness added: notices.
    pub notices: u64,
}

/// What one request held, beside what it used: the per-request context
/// record (`docs/architecture/07-sessions-and-context.md`, "Budgeting").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextStat {
    /// The session.
    pub session: SessionId,
    /// The turn.
    pub turn: TurnId,
    /// The model the request went to.
    pub model: ModelId,
    /// The context epoch, counted on the branch from 1.
    pub epoch: u64,
    /// The estimate by source, in tokens.
    pub sources: ContextSources,
    /// The estimate of the whole request, in tokens.
    pub estimate: u64,
    /// The most the request may hold: the window less the room kept for
    /// the answer and a margin.
    pub limit: u64,
    /// The model's context window.
    pub window: u64,
    /// Where the request first differs from the one before it in the
    /// session: on a cache miss, what moved.
    pub difference: PrefixDifference,
    /// What the provider counted, when the request ended with a count.
    pub usage: Option<TokenUsage>,
}

/// One statistics event.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
#[expect(
    clippy::large_enum_variant,
    reason = "an event is made once and handed over; a box per request would buy nothing"
)]
pub enum StatEvent {
    /// A request ended.
    Request(RequestStat),
    /// A tool call ended.
    ToolCall(ToolCallStat),
    /// A turn ended.
    Turn(TurnStat),
    /// What a request held, by source: sent with its request's `Request`.
    Context(ContextStat),
    /// The limits a provider reported with a response: the latest are kept
    /// for the usage view.
    Limits(ProviderLimits),
}

/// Takes the loop's statistics events.
///
/// `record` is a queue push: it never blocks and never fails the loop.
pub trait Stats: Send + Sync {
    /// One event.
    fn record(&self, event: StatEvent);
}

/// A [`Stats`] that keeps nothing: statistics turned off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStats;

impl Stats for NoStats {
    fn record(&self, _event: StatEvent) {}
}
