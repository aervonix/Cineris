//! The repository map (`docs/architecture/15-efficiency.md`, "Repository
//! map"; decision 0022): per-language outline extractors, the on-disk
//! cache, ranking and the map renderer.
//!
//! | Module | What it gives |
//! |---|---|
//! | [`outline`] | [`Outline`], [`Entry`] and [`Kind`]: a file's definitions with their lines. |
//! | [`language`] | [`Language`]: the fifteen languages with an extractor, known by a file's name or its `#!` line. |
//! | [`extract`] | [`extract()`]: text in, outline out. |
//! | [`text`] | A file's bytes as text: binary files, byte-order marks, bytes that are not UTF-8. |
//!
//! Nothing here holds the index in memory between uses, and nothing runs
//! on its own: the core starts the work, on the blocking pool, after the
//! session has started.

pub mod extract;
pub mod language;
mod lex;
pub mod outline;
pub mod text;

pub use extract::extract;
pub use language::Language;
pub use outline::{Entry, Kind, Outline};
