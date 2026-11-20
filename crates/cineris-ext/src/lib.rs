//! The extension host: custom commands, skills, hooks, the script runtime and its host functions,
//! trust pinning and extension authoring.
//!
//! This crate is being built in step 28 (`docs/build-plan.md`). The script
//! engine is behind the `scripts` feature.

#[cfg(feature = "scripts")]
pub mod engine;
