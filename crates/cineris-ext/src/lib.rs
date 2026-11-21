//! The extension host: custom commands, skills, hooks, the script runtime and its host functions,
//! trust pinning and extension authoring.
//!
//! Being built in step 28 (`docs/build-plan.md`; the design is in
//! `docs/steps/28/answered.md`). What is always here:
//!
//! - [`folder`]: an extension's folder read whole into a snapshot, and its
//!   hash, which is what is pinned;
//! - [`manifest`]: `extension.toml`, read strictly;
//! - [`capability`]: the manifest's capabilities compiled for a workspace,
//!   the check every host call meets first;
//! - [`host`]: the host functions' contract, and [`host::serve`], which
//!   checks the manifest before the session's side is asked;
//! - [`registration`]: what a script registers;
//! - [`event`]: the seven events hooks run on.
//!
//! Behind the `scripts` feature, [`engine`]: QuickJS, one runtime an
//! extension on a thread of its own.

pub mod capability;
#[cfg(feature = "scripts")]
pub mod engine;
pub mod event;
pub mod folder;
pub mod host;
pub mod manifest;
pub mod registration;
pub mod source;
