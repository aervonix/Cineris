//! The extension host: custom commands, skills, hooks, the script runtime and its host functions,
//! trust pinning and extension authoring.
//!
//! Being built in step 28 (`docs/build-plan.md`; the design is in
//! `docs/steps/28/answered.md`). What is always here:
//!
//! - [`discover`]: the extensions in the user's folder of extensions and
//!   the project's, each folder read whole and its manifest checked;
//! - [`folder`]: an extension's folder read whole into a snapshot, and its
//!   hash, which is what is pinned;
//! - [`manifest`]: `extension.toml`, read strictly;
//! - [`capability`]: the manifest's capabilities compiled for a workspace,
//!   the check every host call meets first;
//! - [`host`]: the host functions' contract, and [`host::serve`], which
//!   checks the manifest before the session's side is asked;
//! - [`registration`]: what a script registers;
//! - [`cache`]: the registration cache, what each approved extension
//!   registered, kept by its hash;
//! - [`event`]: the seven events hooks run on;
//! - [`hook`]: a hook's contract: its filter of tools, its answer read
//!   strictly, and a hook command compiled from its configuration's entry;
//! - [`skills`]: the skills in the project's and the user's folders, and
//!   the list of them in section 6 of the system prompt.
//!
//! Behind the `scripts` feature, [`engine`]: QuickJS, one runtime an
//! extension on a thread of its own.

pub mod cache;
pub mod capability;
pub mod discover;
#[cfg(feature = "scripts")]
pub mod engine;
pub mod event;
pub mod folder;
pub mod hook;
pub mod host;
pub mod manifest;
pub mod registration;
pub mod skills;
pub mod source;
