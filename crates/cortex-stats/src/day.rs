//! What one local day spent, read from the monthly files: what a day's
//! limits count, across every process that records
//! (`docs/architecture/18-usage.md`, "Limits on one day"; found by the
//! independent review of step 16: the day's account had counted only its
//! own process's spending).
//!
//! A [`DayReader`] starts where an [`Aggregates`] brought up to date left
//! off, with what the day's requests spent up to there, and from then on
//! reads only what each monthly file gained since: a few lines a request.
//!
//! **A process's own spending is not read back.** The process that keeps a
//! day's account charges it as each request is made, so the lines its own
//! recorder writes for the sessions that draw on that account are already
//! counted. Each request line says which recorder wrote it
//! ([`Line::writer`](crate::Line::writer)); a reader skips the lines of
//! its own writer for the sessions it is told are its own
//! ([`DayReader::own`]). Every other line counts: other processes', this
//! process's sessions that have no day limit, and lines from builds before
//! the writer was recorded.
//!
//! Every function here blocks.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the monthly files are read off the core thread, by the day's own reader thread"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use cortex_base::{FileIdentity, LocalDate};
use cortex_protocol::usage::{Cost, TokenUsage};
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

/// How far one monthly file has been read.
#[derive(Debug, Clone, Copy)]
struct Tail {
    /// Which file it was: another under the name is read from its start.
    identity: Option<FileIdentity>,
    offset: u64,
}

/// What of a line a day's reader looks at.
#[derive(Deserialize)]
struct DayLine {
    day: String,
    kind: String,
    #[serde(default)]
    writer: Option<String>,
    #[serde(default)]
    session: Option<String>,
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
    /// This process's recorder, whose lines for `own` are counted already.
    writer: Option<String>,
    own: BTreeSet<String>,
}

impl DayReader {
    /// A reader that starts where `aggregates`, brought up to date from
    /// `dir` a moment ago, left off, and what the day spent up to there.
    /// `writer` is this process's recorder, when it has one.
    #[must_use]
    pub fn start(
        dir: &StatsDir,
        day: LocalDate,
        aggregates: &Aggregates,
        writer: Option<String>,
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
                let tail = Tail {
                    identity: FileIdentity::at(&dir.events_file(&month)).ok(),
                    offset: aggregates.files.get(&month).map_or(0, |file| file.offset),
                };
                (month, tail)
            })
            .collect();
        let reader = Self {
            dir: dir.clone(),
            day,
            day_text,
            tails,
            writer,
            own: BTreeSet::new(),
        };
        (reader, spent)
    }

    /// The day it reads.
    #[must_use]
    pub fn day(&self) -> LocalDate {
        self.day
    }

    /// Takes `session` as one whose spending this process counts as it
    /// happens: its lines by this process's writer are not read from now
    /// on. What it spent before, and what other processes spend in it, is.
    /// Reads what the files gained first, so that nothing written before
    /// is skipped, and returns that.
    ///
    /// # Errors
    ///
    /// When a file cannot be read. The session is taken as its own all the
    /// same.
    pub fn own(&mut self, session: &str) -> io::Result<DaySpent> {
        let read = self.catch_up();
        self.own.insert(session.to_owned());
        read
    }

    /// What the day's requests spent since the last call, from every
    /// monthly file that may hold the day's lines.
    ///
    /// # Errors
    ///
    /// When a file cannot be read. What was read of the others is kept and
    /// counted at the next call.
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
        let file = match open_to_read(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Gone (a reset): one made again is read from its start.
                self.tails.insert(
                    month.to_owned(),
                    Tail {
                        identity: None,
                        offset: 0,
                    },
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let identity = FileIdentity::of(&file).ok();
        let length = file.metadata()?.len();
        let Some(tail) = self.tails.get_mut(month) else {
            return Ok(());
        };
        if identity != tail.identity {
            // Another file under the name: everything in it is new.
            *tail = Tail {
                identity,
                offset: 0,
            };
        } else if length < tail.offset {
            // Cut back in place: what was counted stays counted, and what
            // is there now is not counted twice.
            tail.offset = length;
            return Ok(());
        }
        let (day, writer, own) = (&self.day_text, &self.writer, &self.own);
        read_lines(file, &mut tail.offset, |line| {
            let Ok(line) = serde_json::from_slice::<DayLine>(line) else {
                return;
            };
            if line.kind != "request" || line.day != *day {
                return;
            }
            let mine = line.writer.is_some()
                && line.writer == *writer
                && line
                    .session
                    .as_ref()
                    .is_some_and(|session| own.contains(session));
            if !mine {
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
