//! Extensions in a session (`docs/architecture/12-extensions.md`;
//! `docs/steps/28/answered.md`, "The design, before building").
//!
//! An approved extension's tools are registered with the session's
//! dynamic-tool registry from what it registered when it was approved, with
//! no engine started. The first call of one starts the extension's runtime
//! (`cineris_ext::engine`), from a snapshot of its folder whose hash must be
//! the approved one, and serves the script's host calls through the
//! session's host: each is checked against the manifest by
//! `cineris_ext::host::serve`, then decided by the session's permission
//! engine as a call of the extension's tool, and only then done.
//!
//! The extensions a session loads are found when it opens (`found`): the
//! user's checked against their pins, a project's through its trust, what
//! each approved one registered taken from the registration cache, and the
//! session's first turn waiting for one the cache misses; and found again
//! at a turn's start when what they were found in changed (`watched`).
//!
//! A session's hook commands (`hooks`) are its configuration's: the
//! user's own and a project's, kept apart, a project's waiting for trust
//! and pinned as the trust question showed it. Each start (`hook_command`)
//! goes through `cineris.exec`'s own path (`exec::run`), decided by the
//! permission engine without a question (`hook_command::hook_may_run`).
//! The hooks a loaded extension's script registered (`script_hooks`) are
//! beside them, each a call of the extension's runtime whose host calls put
//! no question.
//!
//! The script engine is behind the `scripts` feature. Without it no
//! extension is loaded, so none of its tools, commands or hooks is in a
//! session; the hook commands are, and start as they do with it, through
//! `cineris.exec`'s path, which is built either way. An extension that
//! stands approved holds its gating hooks there, failing, since nothing of
//! it runs (`without_engine`).

// What only an extension's script reaches (a tool's or a command's call,
// `cineris.http.fetch`, `cineris.state`, `cineris.ask`) is not reached
// without the script engine, which runs scripts; a hook command's start goes
// through the same path, and is built either way.
#[cfg_attr(
    not(feature = "scripts"),
    allow(
        dead_code,
        reason = "without the script engine only a hook command's start runs a program"
    )
)]
mod exec;
pub(crate) mod found;
mod hook_command;
mod hook_points;
pub(crate) mod hooks;
#[cfg_attr(
    not(feature = "scripts"),
    allow(
        dead_code,
        reason = "without the script engine only a hook command's start is served"
    )
)]
mod host;
pub mod manage;
mod prompt;
mod protected;
#[cfg(feature = "scripts")]
mod script_hooks;
#[cfg_attr(
    not(feature = "scripts"),
    allow(
        dead_code,
        reason = "without the script engine nothing keeps a value: only a script does"
    )
)]
mod state;
#[cfg(feature = "scripts")]
pub(crate) mod tools;
#[cfg_attr(
    not(feature = "scripts"),
    allow(
        dead_code,
        reason = "what is watched is looked at again by the scripts' session alone"
    )
)]
pub(crate) mod watched;
pub(crate) mod without_engine;

use std::path::PathBuf;

use cineris_base::ContentHash;
use cineris_ext::manifest::Manifest;
use cineris_ext::registration::Registration;

pub use found::{
    ANY_HOST, BEFORE_AN_EXTENSION_IS_REGISTERED, BEFORE_THE_CACHE_IS_KEPT, CONFINED_WHERE_ON,
    NOT_CONFINED, THE_QUESTIONS_TRUST_IS_READ, WAIT_SAID_AFTER, WAITING, WAITING_FOR_HOOKS,
};

pub(crate) use exec::{ExecParts, HarnessFiles};
#[cfg(feature = "scripts")]
pub(crate) use hook_command::turn_number;
pub use hook_command::{
    A_CLOSE_WAITS_FOR_A_RUN, A_FIRST_TURN_WAITS, A_HOOK_WRITES, A_SESSION_ENDS,
    A_SESSION_START_HOOK_IS_HEARD, HookOutcome, HookRun,
};
pub(crate) use hook_command::{HookCommands, SessionHooks};
pub(crate) use hook_points::PointHooks;
pub(crate) use host::HostParts;
pub(crate) use prompt::PromptParts;
#[cfg(feature = "scripts")]
pub use script_hooks::{A_SCRIPT_HOOK_ENDS, A_SCRIPT_HOOK_STARTS};
#[cfg(feature = "scripts")]
pub(crate) use script_hooks::{Gate, ScriptHooks, SeenHooks, SessionStart, Stand, way_out};
pub(crate) use state::{ExtState, ProjectStore};
#[cfg(feature = "scripts")]
pub(crate) use tools::register;

/// An extension the user approved, as the session is given it: its folder,
/// the hash it was approved at, and what it registered then.
#[derive(Debug, Clone)]
pub struct ApprovedExtension {
    /// Where it was found: the user's own, or the project's.
    pub source: cineris_ext::source::Source,
    /// Its folder.
    pub folder: PathBuf,
    /// The hash of its folder when it was approved: a runtime is made only
    /// from a snapshot with this hash.
    pub hash: ContentHash,
    /// Its manifest, as approved.
    pub manifest: Manifest,
    /// What it registered when it was approved: a live runtime must
    /// register the same.
    pub registration: Registration,
}
