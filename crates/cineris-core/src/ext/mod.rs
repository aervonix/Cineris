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
//! session's first turn waiting for one the cache misses.
//!
//! The script engine is behind the `scripts` feature: without it, no
//! extension's tool is registered.

#[cfg(feature = "scripts")]
mod exec;
pub(crate) mod found;
#[cfg(feature = "scripts")]
mod host;
#[cfg(feature = "scripts")]
mod state;
#[cfg(feature = "scripts")]
mod tools;

use std::path::PathBuf;

use cineris_base::ContentHash;
use cineris_ext::manifest::Manifest;
use cineris_ext::registration::Registration;

pub use found::BEFORE_AN_EXTENSION_IS_REGISTERED;

#[cfg(feature = "scripts")]
pub(crate) use exec::{ExecParts, HarnessFiles};
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
