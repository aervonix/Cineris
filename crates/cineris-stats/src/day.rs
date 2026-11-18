//! What one local day spent, read from the monthly files: what a day's
//! limits count, across every process that records
//! (`docs/architecture/18-usage.md`, "Limits on one day"; found by the
//! independent review of step 16: the day's account had counted only its
//! own process's spending).
//!
//! A [`DayReader`] starts where an [`Aggregates`] left off, with what the
//! day's requests spent up to there, and from then on reads only what each
//! monthly file the day's lines can be in gained since: a few lines a
//! request.
//!
//! **A purse's own requests are not read back.** The day's purse of a
//! process charges each request as it is made, so the line of a request it
//! counted says so (`charged_to`, the purse's id), and its reader skips
//! exactly those lines. Every other line counts: other processes', this
//! process's sessions with no day's limit, a session's whose purse is
//! another day's, and lines from builds before the mark was written.
//!
//! **Each file it reads is held open,** so that the identity it compares
//! with the name's is one no other file can be given: on Linux a deleted
//! file's inode number is often given to the next file made in its
//! directory, and a reader that let go of the old one could take a file
//! made again after `cineris stats reset` for it.
//!
//! Every function here blocks.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the monthly files are read off the core thread, by the day's own reader thread"
)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io;

use cineris_base::{FileIdentity, LocalDate};
use cineris_protocol::usage::{Cost, TokenUsage};
use serde::Deserialize;

use crate::aggregate::UsageSum;
use crate::store::{Aggregates, StatsDir, open_to_read, read_lines};

/// What some requests spent, as a day's limits count it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaySpent {
    /// Requests to a model.
    pub requests: u64,
    /// Tokens: fresh input, cache reads and writes, and output.
    pub tokens: u64,
    /// Money, in nano-dollars, whatever its basis.
    pub usd_nanos: u64,
}

impl DaySpent {
    /// What an aggregate's sum spent.
    #[must_use]
    pub fn of_sum(sum: &UsageSum) -> Self {
        Self {
            requests: sum.requests,
            tokens: sum
                .input
                .saturating_add(sum.cache_read)
                .saturating_add(sum.cache_write)
                .saturating_add(sum.output),
            usd_nanos: sum
                .metered_nanos
                .saturating_add(sum.estimate_nanos)
                .saturating_add(sum.recorded_nanos),
        }
    }

    /// What one request spent.
    fn of_request(tokens: &TokenUsage, cost: Option<&Cost>) -> Self {
        Self {
            requests: 1,
            tokens: tokens
                .input
                .saturating_add(tokens.cache_read)
                .saturating_add(tokens.cache_write)
                .saturating_add(tokens.output),
            usd_nanos: cost.map_or(0, |cost| cost.usd_nanos),
        }
    }

    /// Both together.
    pub fn add(&mut self, other: Self) {
        self.requests = self.requests.saturating_add(other.requests);
        self.tokens = self.tokens.saturating_add(other.tokens);
        self.usd_nanos = self.usd_nanos.saturating_add(other.usd_nanos);
    }
}

/// How far one monthly file has been read, and the file.
#[derive(Debug, Default)]
struct Tail {
    /// The file read, held open, and which it is: another under the name
    /// is read from its start.
    file: Option<(File, FileIdentity)>,
    offset: u64,
}

/// What of a line a day's reader looks at.
#[derive(Deserialize)]
struct DayLine {
    day: String,
    kind: String,
    #[serde(default)]
    charged_to: Option<String>,
    #[serde(default)]
    tokens: TokenUsage,
    #[serde(default)]
    cost: Option<Cost>,
}

/// Reads what one local day's requests spent, across processes.
#[derive(Debug)]
pub struct DayReader {
    dir: StatsDir,
    day: LocalDate,
    day_text: String,
    /// The monthly files the day's lines may be in, by month.
    tails: BTreeMap<String, Tail>,
    /// The id of the purse this reader adds to: its lines are counted
    /// already.
    purse: Option<String>,
}

impl DayReader {
    /// A reader that starts where `aggregates`, as loaded from `dir`, left
    /// off, and what the day spent up to there. `purse` is the id of the
    /// purse it reads for, whose requests' lines it skips. Call
    /// [`catch_up`](Self::catch_up) next for what the files gained since
    /// the aggregates were saved.
    #[must_use]
    pub fn start(
        dir: &StatsDir,
        day: LocalDate,
        aggregates: &Aggregates,
        purse: Option<String>,
    ) -> (Self, DaySpent) {
        let day_text = day.to_string();
        let mut spent = DaySpent::default();
        for (key, sum) in &aggregates.total().usage {
            if key.period == day_text {
                spent.add(DaySpent::of_sum(sum));
            }
        }
        let tails = months_of(day)
            .into_iter()
            .map(|month| {
                // The file the aggregates read to their offset is the one
                // there now: a reset that replaced it took the aggregates
                // with it.
                let file = open_to_read(&dir.events_file(&month))
                    .ok()
                    .and_then(|file| {
                        FileIdentity::of(&file)
                            .ok()
                            .map(|identity| (file, identity))
                    });
                let offset = aggregates.files.get(&month).map_or(0, |part| part.offset);
                (month, Tail { file, offset })
            })
            .collect();
        let reader = Self {
            dir: dir.clone(),
            day,
            day_text,
            tails,
            purse,
        };
        (reader, spent)
    }

    /// The day it reads.
    #[must_use]
    pub fn day(&self) -> LocalDate {
        self.day
    }

    /// What the day's requests spent since the last call, from every
    /// monthly file that may hold the day's lines, less the requests of the
    /// purse it reads for.
    ///
    /// # Errors
    ///
    /// When a file cannot be read and nothing was read from the others.
    /// What was read is kept and counted, here or at the next call.
    pub fn catch_up(&mut self) -> io::Result<DaySpent> {
        let mut spent = DaySpent::default();
        let mut failed = None;
        let months: Vec<String> = self.tails.keys().cloned().collect();
        for month in months {
            if let Err(error) = self.read_month(&month, &mut spent) {
                failed.get_or_insert(error);
            }
        }
        match failed {
            Some(error) if spent == DaySpent::default() => Err(error),
            _ => Ok(spent),
        }
    }

    fn read_month(&mut self, month: &str, spent: &mut DaySpent) -> io::Result<()> {
        let path = self.dir.events_file(month);
        let Some(tail) = self.tails.get_mut(month) else {
            return Ok(());
        };
        match FileIdentity::at(&path) {
            Ok(now) if tail.file.as_ref().is_some_and(|(_, held)| *held == now) => {}
            Ok(_) => {
                // Another file under the name, or one where there was none:
                // everything in it is new.
                let file = open_to_read(&path)?;
                let identity = FileIdentity::of(&file)?;
                *tail = Tail {
                    file: Some((file, identity)),
                    offset: 0,
                };
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Gone (a reset): one made again is read from its start.
                *tail = Tail::default();
                return Ok(());
            }
            Err(error) => {
                // On Windows a file deleted while it is held open cannot be
                // looked at until every holder lets it go: this one does,
                // so that the delete completes and the file can be made
                // again. What was read of it stays counted.
                if cfg!(windows) {
                    tail.file = None;
                }
                return Err(error);
            }
        }
        let Some((file, _)) = tail.file.as_mut() else {
            return Ok(());
        };
        let length = file.metadata()?.len();
        if length < tail.offset {
            // Cut back in place: what was counted stays counted, and what
            // is there now is not counted twice.
            tail.offset = length;
            return Ok(());
        }
        let (day, purse) = (&self.day_text, &self.purse);
        read_lines(file, &mut tail.offset, |line| {
            let Ok(line) = serde_json::from_slice::<DayLine>(line) else {
                return;
            };
            if line.kind != "request" || line.day != *day {
                return;
            }
            let counted = line.charged_to.is_some() && line.charged_to == *purse;
            if !counted {
                spent.add(DaySpent::of_request(&line.tokens, line.cost.as_ref()));
            }
        })
    }
}

/// The months (UTC) whose files may hold lines of the local day `day`: the
/// day lies within a day of the UTC date of the same name, as no zone is
/// more than a day from UTC.
fn months_of(day: LocalDate) -> Vec<String> {
    let mut months: Vec<String> = [day.add_days(-1), day, day.add_days(1)]
        .iter()
        .map(|date| date.month_text())
        .collect();
    months.dedup();
    months
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_looked_for_in_the_months_it_may_reach_in_utc() {
        let day = |text| LocalDate::parse(text).unwrap();
        assert_eq!(months_of(day("2026-11-15")), ["2026-11"]);
        assert_eq!(months_of(day("2026-11-30")), ["2026-11", "2026-12"]);
        assert_eq!(months_of(day("2026-12-01")), ["2026-11", "2026-12"]);
    }
}
