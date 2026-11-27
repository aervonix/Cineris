//! The core: every part of the harness wired into one thing that takes
//! operations and emits events (`docs/architecture/00-overview.md`).
//!
//! This is the only crate that knows all the parts. Each of them was built
//! against a seam of its own (a trait it defines, or a type it takes) and
//! none of them can see the others. The modules here are those seams,
//! filled:
//!
//! | Module | Joins |
//! |---|---|
//! | [`core`] | Front ends to sessions: operations, client roles, the requests a front end answers. |
//! | [`events`] | The loop's events to each client's bounded queue, with deltas merged. |
//! | `assemble` | Configuration, trust, the engine, the models, the store and the tools into one session. |
//! | `store` | The loop's session store to the session log. |
//! | `content` | The tools' content store and the provider layer's image source to the session's files. |
//! | `checkpoints` | The editing tools and the shell tool to the round's checkpoint. |
//! | `policy` | The loop's policy, the tools' read gate and the base state's keep gate to the permission engine. |
//! | `provider` | Configuration and credentials to model clients. |
//! | `redact` | The secret filter to the session log, and the rules for provider blocks. |
//! | `shell_cache` | The shell's version, kept in the state directory so that a session does not start the shell to ask it. |
//! | `manage` | The command line's subcommands to the credential store, the stored approvals and the trust store, judged as a session judges them. |
//! | `stats` | The loop's statistics events to the statistics store's recorder. |
//! | `usage` | `usage/report` from the aggregates, and `usage/refresh` to the providers' documented usage endpoints. |
//! | [`mcp`] | MCP's local servers started as the harness starts a program, and the configuration's servers as `cineris-mcp`'s definitions (behind the `mcp` feature). |
//!
//! A front end needs [`Core`], [`CoreOptions`], [`ClientInfo`], and the
//! protocol's own types: an [`Op`](cineris_protocol::methods::Op) in, an
//! [`Outgoing`] out.

mod assemble;
mod checkpoints;
mod clients;
mod commands;
mod content;
pub mod core;
mod day;
pub mod events;
pub mod ext;
pub use commands::A_COMMAND_IS_TAKEN;
#[cfg(feature = "scripts")]
pub use commands::A_SCRIPT_COMMAND_RUNS;
mod instructions;
mod knowledge;
mod manage;
// MCP's local servers as the harness starts them, and the configuration's
// servers as `cineris-mcp`'s definitions (step 30).
#[cfg(feature = "mcp")]
pub mod mcp;
mod paging;
mod policy;
// An extension's programs and a hook command's: found, quoted and started
// alike.
mod programs;
mod provider;
mod reads;
mod redact;
mod reload;
mod repo_map;
mod rewind;
mod rules;
mod sandbox;
mod shell_cache;
mod stats;
mod store;
mod usage;
mod verify;

pub use crate::assemble::SANDBOX_OFF_TOO;
pub use crate::checkpoints::{
    AFTER_THE_TURN_IS_STOPPED, BEFORE_THE_COMMAND_STARTS, BEFORE_THE_LOOK_AFTER,
};
pub use crate::clients::{ClientId, ClientInfo};
pub use crate::core::{
    BEFORE_A_SESSION_ENDS, BEFORE_THE_PUT_BACK_IS_ASKED, Client, Core, CoreOptions, OpError,
    RegistryProbe,
};
pub use crate::events::{EventQueue, Outgoing, QUEUE_CAPACITY};
pub use crate::ext::manage::{
    Added, DisableOutcome, ExtensionListed, ExtensionStanding, ExtensionsListing, Gates,
    Registered, Removal, Removed, ToAdd,
};
pub use crate::knowledge::AFTER_THE_RULES;
pub use crate::manage::{
    Project, ProjectExtension, ProjectFiles, ProjectHookCommand, ProviderCredential, TrustState,
    Trusted,
};
pub use crate::policy::{
    AFTER_THE_LOOK, BEFORE_THE_APPROVAL_IS_OPEN, BEFORE_THE_NEW_LEVEL_IS_IN_FORCE,
    CHANGING_APPROVALS, PermissionDenial,
};
pub use crate::provider::{ANTHROPIC_KEY_ENV, Connector, EFFORT_ALIASES, KeyFound};
pub use crate::reload::{CHOICE_BEING_KEPT, SKILL_FOLDERS_STAY, TRUST_JUDGED};
pub use crate::repo_map::Probe;
pub use crate::rules::parse_rule;
pub use crate::usage::AGGREGATES_READ;
pub use crate::usage::StatsSummary;
pub use cineris_ext::source::Source as ExtensionSource;
/// The runtime's timer, which the core's options take: the provider layer's
/// and the process runner's waits are on it.
pub use cineris_llm::TokioTimer;
/// The harness's own approving commands, as the permission engine refuses
/// them: what a front end gives its grammar to (`CoreOptions::own_commands`).
pub use cineris_policy::approving;
/// What `cineris stats` found where the aggregates file is.
pub use cineris_stats::Found as AggregatesFound;
