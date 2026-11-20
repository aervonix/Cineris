//! What an extension's script registers when its main module is first
//! evaluated: its tools, its commands and its hooks.
//!
//! It is what the registration cache keeps for an approved extension, so
//! that a session knows an extension's contributions without starting the
//! engine. A live runtime registers again when it starts, and must register
//! exactly this, or the extension is disabled (`docs/steps/28/answered.md`,
//! "The manifest and the registration cache").

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The most tools one extension registers.
pub const MAX_TOOLS: usize = 64;
/// The most commands one extension registers.
pub const MAX_COMMANDS: usize = 32;
/// The most hooks one extension registers.
pub const MAX_HOOKS: usize = 32;
/// The longest description of a tool or a command.
pub const MAX_DESCRIPTION: usize = 2048;
/// The largest schema of a tool's arguments, as JSON.
pub const MAX_SCHEMA_BYTES: usize = 16 * 1024;

/// One tool a script registered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDef {
    /// Its name within the extension: the part after the `/`.
    pub name: String,
    /// What it does, for the model.
    pub description: String,
    /// The JSON Schema of its arguments.
    pub input: Json,
    /// What its author says: it changes nothing. Shown to the user; it
    /// decides nothing, since an extension's tool always runs alone.
    #[serde(default)]
    pub read_only: bool,
}

/// One command a script registered: a slash command its code handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDef {
    /// The name the user types after `/`.
    pub name: String,
    /// What it does, for the menu.
    pub description: String,
    /// Its named arguments.
    #[serde(default)]
    pub arguments: Vec<String>,
}

/// One hook a script registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookDef {
    /// The event, as its name is written (`tool.before`).
    pub event: String,
    /// For a tool event: the tools it runs on, by name or as `source/*`.
    /// Empty: every tool.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Whether it only observes: never waited for, its answer not read.
    #[serde(default)]
    pub observe: bool,
    /// Its time, in milliseconds, when not the default.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Everything one script registered, in the order it registered it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    /// Its tools.
    #[serde(default)]
    pub tools: Vec<ToolDef>,
    /// Its commands.
    #[serde(default)]
    pub commands: Vec<CommandDef>,
    /// Its hooks.
    #[serde(default)]
    pub hooks: Vec<HookDef>,
}
