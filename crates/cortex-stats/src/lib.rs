//! Local statistics: event recording, monthly files, aggregates, retention
//! and the usage query (`docs/architecture/14-adaptation.md`, "Local
//! statistics"; `docs/architecture/18-usage.md`;
//! `docs/decisions/0021-local-statistics.md`).
//!
//! | Module | What it gives |
//! |---|---|
//! | [`record`] | [`Record`]: one event, as a line of a monthly file holds it. |
//! | [`recorder`] | [`Recorder`]: the bounded queue and the writer thread. |
//! | [`store`] | [`StatsDir`], [`Aggregates`] and [`LimitsFile`]: the files, brought up to date, pruned and rebuilt. |
//! | [`aggregate`] | [`Part`]: what some events add up to, by the keys the view asks by. |
//! | [`query`] | [`report`]: a range and a grouping answered from the aggregates. |
//!
//! Nothing leaves the machine, and nothing here is read until it is asked
//! for: making a [`Recorder`] touches no file.

pub mod aggregate;
pub mod query;
pub mod record;
pub mod recorder;
pub mod store;

pub use aggregate::{DAYS_KEPT, Part};
pub use query::{Query, current_limits, days_of, report};
pub use record::{
    ContextRecord, Difference, Line, Purpose, Record, RequestRecord, Sources, ToolOutcome,
    ToolRecord, ToolShare, TurnRecord, Variants,
};
pub use recorder::{Recorder, RecorderOptions};
pub use store::{Aggregates, LimitsFile, StatsDir, month_of, reads};
