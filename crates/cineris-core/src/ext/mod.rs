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
//!
//! The script engine is behind the `scripts` feature: without it, no
//! extension's tool is registered, and no hook command is started yet: its
//! start is `cineris.exec`'s, which the feature builds.

#[cfg(feature = "scripts")]
mod exec;
pub(crate) mod found;
#[cfg(feature = "scripts")]
mod hook_command;
#[cfg(feature = "scripts")]
mod hook_points;
pub(crate) mod hooks;
#[cfg(feature = "scripts")]
mod host;
#[cfg(feature = "scripts")]
mod prompt;
#[cfg(feature = "scripts")]
mod protected;
#[cfg(feature = "scripts")]
mod state;
#[cfg(feature = "scripts")]
mod tools;
#[cfg_attr(
    not(feature = "scripts"),
    allow(
        dead_code,
        reason = "what is watched is looked at again by the scripts' session alone"
    )
)]
pub(crate) mod watched;

use std::path::PathBuf;

use cineris_base::ContentHash;
use cineris_ext::manifest::Manifest;
use cineris_ext::registration::Registration;

pub use found::{
    ANY_HOST, BEFORE_AN_EXTENSION_IS_REGISTERED, BEFORE_THE_CACHE_IS_KEPT, NOT_CONFINED,
    THE_QUESTIONS_TRUST_IS_READ, WAIT_SAID_AFTER, WAITING,
};

#[cfg(feature = "scripts")]
pub(crate) use exec::{ExecParts, HarnessFiles};
#[cfg(feature = "scripts")]
pub(crate) use hook_command::{HookCommands, SessionHooks};
#[cfg(feature = "scripts")]
pub use hook_command::{HookOutcome, HookRun};
#[cfg(feature = "scripts")]
pub(crate) use hook_points::PointHooks;
#[cfg(feature = "scripts")]
pub(crate) use prompt::PromptParts;
#[cfg(feature = "scripts")]
pub(crate) use state::{ExtState, ProjectStore};
#[cfg(feature = "scripts")]
pub(crate) use tools::{HostParts, register};

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
