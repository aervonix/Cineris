//! The project profile and project memory: what the harness knows about
//! one repository (`docs/architecture/14-adaptation.md`, "Project
//! knowledge: profile and memory").
//!
//! | Module | What it holds |
//! |---|---|
//! | [`entry`] | One entry: its kind, its standing, where it came from, and the three questions that decide whether it is used. |
//! | [`file`] | The two files as text: the private store and the repository's shared profile, read strictly. |
//! | [`store`] | The private store on disk: the lock, the atomic write, the good copy. |
//! | [`project`] | The two files put together: a [`Snapshot`] of every entry with its standing for this user, and the changes a user, a model and the clock make. |
//! | [`render`] | Section 5 of the system prompt, from a snapshot. |
//!
//! Everything here blocks, and is called on the blocking pool. Nothing
//! here is read when a session is made: the first read is at the start of
//! a session's first turn.

pub mod entry;
pub mod file;
pub mod project;
pub mod render;
pub mod store;

pub use entry::{Basis, Check, Command, Entry, Kind, Learned, Purpose, Scope, Status};
pub use file::{Decision, PrivateState, Problem};
pub use project::{
    ChangeError, Project, ProposalOutcome, Proposed, Snapshot, UpdateError, Updated, error_code,
};
pub use render::{Section, render};
pub use store::{Store, StoreError};
