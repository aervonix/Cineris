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
//! | [`walk`] | What a pass lists: the ignore rules, the permission [`Gate`], the [`Limits`]. |
//! | [`pass`] | [`Index`] and [`Index::pass`]: the workspace set beside the cache, and what changed read again. |
//! | [`map`] | Ranking and rendering the map ([`MapRequest`], [`Index::map`]), and the [`RecentFiles`] seam. |
//! | [`lookup`] | [`Index::outline_of`]: one file's outline, made from the file as it is now; [`Index::map`]. |
//!
//! Nothing here holds the index in memory between uses, and nothing runs
//! on its own: the core starts the work, on the blocking pool, after the
//! session has started. The cache's format is in [`cache`]'s module
//! documentation.

pub mod cache;
pub mod extract;
pub mod language;
mod lex;
pub mod lookup;
pub mod map;
pub mod outline;
pub mod pass;
pub mod text;
pub mod walk;

pub use extract::extract;
pub use language::Language;
pub use lookup::{FileOutline, OutlineError};
pub use map::{DEFAULT_TOKENS, MapRequest, NoRecentFiles, RecentFiles};
pub use outline::{Entry, Kind, Outline};
pub use pass::{CACHE_FILE, Index, PassReport, Snapshot};
pub use walk::{Gate, Limits, OpenGate, Partial, Skipped, Stamp, path_order};
