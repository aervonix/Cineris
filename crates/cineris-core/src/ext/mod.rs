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
//! The script engine is behind the `scripts` feature: without it, no
//! extension's tool is registered.

#[cfg(feature = "scripts")]
mod exec;
#[cfg(feature = "scripts")]
mod host;
#[cfg(feature = "scripts")]
mod tools;

use std::path::PathBuf;

use cineris_base::ContentHash;
use cineris_ext::manifest::Manifest;
use cineris_ext::registration::Registration;

#[cfg(feature = "scripts")]
pub(crate) use exec::{ExecParts, HarnessFiles};
#[cfg(feature = "scripts")]
pub(crate) use tools::{HostParts, register};

/// An extension the user approved, as the session is given it: its folder,
/// the hash it was approved at, and what it registered then.
#[derive(Debug, Clone)]
pub struct ApprovedExtension {
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
