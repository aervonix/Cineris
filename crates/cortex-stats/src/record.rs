//! What one line of a monthly file holds: one event
//! (`docs/architecture/14-adaptation.md`, "Local statistics").
//!
//! An event holds no prompt and no file content: names, counts, durations
//! and the outcome. Each line is one JSON object with a `kind`, written as
//! it was recorded, and read back by the same types.

use cortex_protocol::event::{TurnOutcome, Verification};
use cortex_protocol::model::ModelRole;
use cortex_protocol::usage::{Billing, Cost, ModelTime, TokenUsage};
use cortex_protocol::{ModelId, ProjectKey, ProviderId, SessionId, Speed, Timestamp, TurnId};
use serde::{Deserialize, Serialize};

/// The choices at each variant point a request was made with
/// (`docs/architecture/14-adaptation.md`, "Adapting to the model"): fixed
/// for a context epoch, and recorded with every outcome so that the
/// variants' success rates can be compared.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Variants {
    /// The edit format: `replace`.
    pub edit_format: String,
    /// How tools are declared: `strict` or `loose`.
    pub tool_schema: String,
    /// The prompt style: `concise` or `guided`.
    pub prompt_style: String,
    /// The reasoning level in effect, as the model names it, when the model
    /// has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl Variants {
    /// The variants as one key: `replace/strict/concise/high`.
    #[must_use]
    pub fn key(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.edit_format,
            self.tool_schema,
            self.prompt_style,
            self.effort.as_deref().unwrap_or("-")
        )
    }
}

/// Where a request's own time went, in milliseconds.
pub type RequestTime = ModelTime;

/// Why a request was made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// One of the turn's own requests.
    #[default]
    Turn,
    /// A summary for a compaction.
    Summary,
}

/// One request to a model ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestRecord {
    /// When it ended.
    pub at: Timestamp,
    /// The session.
    pub session: SessionId,
    /// The turn it belongs to: for a summary prepared in the background,
    /// the turn that started it.
    pub turn: TurnId,
    /// The project's key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectKey>,
    /// The provider.
    pub provider: ProviderId,
    /// The model.
    pub model: ModelId,
    /// The role it was made in.
    pub role: ModelRole,
    /// Why it was made.
    #[serde(default)]
    pub purpose: Purpose,
    /// The speed it ran at.
    #[serde(default, skip_serializing_if = "Speed::is_standard")]
    pub speed: Speed,
    /// The speed asked for, when it ran at another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_requested: Option<Speed>,
    /// The variants in effect.
    pub variants: Variants,
    /// Tokens by class. All zero when it failed before any were counted.
    #[serde(default)]
    pub tokens: TokenUsage,
    /// The cost, fixed when the request was recorded, at the prices of the
    /// catalog then. Absent when the model has no known price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    /// The catalog the prices came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_version: Option<String>,
    /// How it is paid for.
    pub billing: Billing,
    /// Milliseconds from sending to the first token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<u64>,
    /// Output tokens a second, from the first token to the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
    /// Where its time went.
    #[serde(default)]
    pub time: RequestTime,
    /// Why the response ended: the neutral stop reason.
    pub stop: String,
    /// The code of the failure, when it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Attempts given up before the one that counted.
    #[serde(default)]
    pub retries: u32,
}

/// How a tool call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    /// It ran and succeeded.
    Ok,
    /// Its arguments did not fit the tool.
    InvalidArguments,
    /// The model named a tool that does not exist.
    UnknownTool,
    /// It ran and failed.
    Failed,
    /// It was not allowed to run.
    Denied,
    /// It was cancelled.
    Interrupted,
}

impl ToolOutcome {
    /// The outcome as the files write it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::InvalidArguments => "invalid_arguments",
            Self::UnknownTool => "unknown_tool",
            Self::Failed => "failed",
            Self::Denied => "denied",
            Self::Interrupted => "interrupted",
        }
    }
}

/// One tool call ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRecord {
    /// When it ended.
    pub at: Timestamp,
    /// The session.
    pub session: SessionId,
    /// The turn.
    pub turn: TurnId,
    /// The project's key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectKey>,
    /// The tool's name.
    pub tool: String,
    /// The model that made the call.
    pub model: ModelId,
    /// The variants in effect.
    pub variants: Variants,
    /// How it ended.
    pub outcome: ToolOutcome,
    /// How long it ran.
    #[serde(default)]
    pub running_ms: u64,
    /// How long it waited, once allowed, before it ran.
    #[serde(default)]
    pub queued_ms: u64,
    /// How long it waited for the user's answer.
    #[serde(default)]
    pub approval_ms: u64,
}

/// One tool's part of a turn's tool time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolShare {
    /// The tool.
    pub tool: String,
    /// Its part, in milliseconds.
    pub ms: u64,
}

/// One turn ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRecord {
    /// When it ended.
    pub at: Timestamp,
    /// The session.
    pub session: SessionId,
    /// The turn.
    pub turn: TurnId,
    /// The project's key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectKey>,
    /// The main model's provider.
    pub provider: ProviderId,
    /// The main model.
    pub model: ModelId,
    /// The variants in effect at its end.
    pub variants: Variants,
    /// Requests the turn made of its main model.
    #[serde(default)]
    pub requests: u64,
    /// Tool calls it recorded a result for.
    #[serde(default)]
    pub tool_calls: u64,
    /// How it ended.
    pub outcome: TurnOutcome,
    /// What the project's checks said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<Verification>,
    /// Its wall time.
    pub wall_ms: u64,
    /// Waiting for the model.
    #[serde(default)]
    pub model_time: ModelTime,
    /// Waiting for tools once the response had ended.
    #[serde(default)]
    pub tools_ms: u64,
    /// Waiting for the user's answer with no tool running.
    #[serde(default)]
    pub approval_ms: u64,
    /// Running the project's checks.
    #[serde(default)]
    pub verification_ms: u64,
    /// The harness's own work.
    #[serde(default)]
    pub harness_ms: u64,
    /// `tools_ms` by tool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_tool: Vec<ToolShare>,
}

/// Where the cached prefix of a request first differed from the request
/// before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "at")]
pub enum Difference {
    /// The first request of the session, or of a new epoch.
    First,
    /// Nothing differed: the whole earlier request was a prefix.
    None,
    /// The model, speed or settings.
    Settings,
    /// The system prompt.
    System,
    /// The tool definitions.
    Tools,
    /// A message of the history, counted from 0.
    Message {
        /// Its index.
        index: u64,
    },
}

/// How many tokens of a request each part of it is estimated at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sources {
    /// The system prompt.
    pub system: u64,
    /// The tool definitions.
    pub tools: u64,
    /// The summary that began the epoch.
    pub summary: u64,
    /// What the user wrote.
    pub user: u64,
    /// What the model answered.
    pub assistant: u64,
    /// What tools returned.
    pub tool_results: u64,
    /// The harness's notices.
    pub notices: u64,
}

/// What one request held: the per-request context record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRecord {
    /// When its request ended.
    pub at: Timestamp,
    /// The session.
    pub session: SessionId,
    /// The turn.
    pub turn: TurnId,
    /// The project's key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectKey>,
    /// The model.
    pub model: ModelId,
    /// The context epoch, from 1.
    pub epoch: u64,
    /// The estimate by source.
    pub sources: Sources,
    /// The estimate of the whole request.
    pub estimate: u64,
    /// The most the request may hold.
    pub limit: u64,
    /// The model's window.
    pub window: u64,
    /// Where it first differed from the request before it.
    pub difference: Difference,
    /// What the provider counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenUsage>,
}

/// One line of a monthly file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Record {
    /// A request ended.
    Request(RequestRecord),
    /// A tool call ended.
    Tool(ToolRecord),
    /// A turn ended.
    Turn(TurnRecord),
    /// What a request held.
    Context(ContextRecord),
    /// Events the writer could not take, because its queue was full.
    Dropped {
        /// When it was written.
        at: Timestamp,
        /// How many.
        count: u64,
    },
}

impl Record {
    /// When the event happened.
    #[must_use]
    pub fn at(&self) -> &Timestamp {
        match self {
            Self::Request(record) => &record.at,
            Self::Tool(record) => &record.at,
            Self::Turn(record) => &record.at,
            Self::Context(record) => &record.at,
            Self::Dropped { at, .. } => at,
        }
    }
}

/// A line as it is stored: the event, and the local day it fell on, fixed
/// when it was written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    /// The local date, `YYYY-MM-DD`.
    pub day: String,
    /// The event.
    #[serde(flatten)]
    pub record: Record,
}
