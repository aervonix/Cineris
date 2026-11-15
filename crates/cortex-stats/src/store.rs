//! The statistics directory: monthly files of events, the aggregates file,
//! and the limits the providers last reported
//! (`docs/decisions/0021-local-statistics.md`).
//!
//! | File | What it holds |
//! |---|---|
//! | `events-YYYY-MM.jsonl` | One event a line, appended, by the month (UTC) it happened in. |
//! | `aggregates.json` | Each monthly file's [`Part`] and how far it was read, and the part of the files already pruned. |
//! | `limits.json` | The latest limits and balances each provider reported, with when and how. |
//!
//! **The aggregates are brought up to date from the files.** Each monthly
//! file's part records the byte it was read to. [`Aggregates::sync`] reads
//! each file from there to its last whole line, adds what it finds, and
//! moves the part of a file past its retention into `kept` before the file
//! is deleted. Any process may do it: two that do it at once each write a
//! file that is "kept + every monthly file read to some point", which the
//! next sync carries on from. So [`Aggregates::rebuild`] from the files
//! gives the same values.
//!
//! **Every file is owner-only**, written through `cortex_base::private`,
//! and the aggregates and the limits are replaced atomically.
//!
//! Every function here blocks.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the statistics files are read and written off the core thread: by the writer thread, or by a command"
)]

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use cortex_base::{AtomicWriteOptions, Clock, LocalDate, UtcTime, private, write_atomic};
use cortex_protocol::usage::ProviderLimits;
use serde::{Deserialize, Serialize};

use crate::aggregate::Part;
use crate::record::Line;

/// The version of the aggregates file this build writes. A file of another
/// version is rebuilt from the monthly files.
pub const AGGREGATES_VERSION: u32 = 1;

/// How many times a statistics file was opened for reading in this
/// process, by directory: what the test that nothing is read at start-up
/// counts.
static READS: Mutex<BTreeMap<PathBuf, u64>> = Mutex::new(BTreeMap::new());

/// How many times this process opened a file of `dir` for reading.
#[must_use]
pub fn reads(dir: &StatsDir) -> u64 {
    READS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(dir.root())
        .copied()
        .unwrap_or(0)
}

fn open_to_read(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        *READS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(dir.to_path_buf())
            .or_default() += 1;
    }
    File::open(path)
}

/// Where the statistics are kept: `stats/` in the state directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsDir {
    root: PathBuf,
}

impl StatsDir {
    /// The directory at `root`. Nothing is created until something is
    /// written.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The monthly file of `month` (`YYYY-MM`).
    #[must_use]
    pub fn events_file(&self, month: &str) -> PathBuf {
        self.root.join(format!("events-{month}.jsonl"))
    }

    /// The aggregates file.
    #[must_use]
    pub fn aggregates_file(&self) -> PathBuf {
        self.root.join("aggregates.json")
    }

    /// The file of the latest limits.
    #[must_use]
    pub fn limits_file(&self) -> PathBuf {
        self.root.join("limits.json")
    }

    /// Creates the directory, owner-only, if it is not there.
    ///
    /// # Errors
    ///
    /// When it cannot be made.
    pub fn ensure(&self) -> io::Result<()> {
        private::create_dir_all(&self.root)
    }

    /// The months that have a file, oldest first.
    ///
    /// # Errors
    ///
    /// When the directory cannot be listed. A missing one has none.
    pub fn months(&self) -> io::Result<Vec<String>> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut months: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let month = name.strip_prefix("events-")?.strip_suffix(".jsonl")?;
                is_month(month).then(|| month.to_owned())
            })
            .collect();
        months.sort();
        Ok(months)
    }

    /// Deletes every statistics file: `cortex stats reset`.
    ///
    /// # Errors
    ///
    /// When a file cannot be removed.
    pub fn reset(&self) -> io::Result<u64> {
        let mut removed = 0;
        for month in self.months()? {
            remove_if_there(&self.events_file(&month))?;
            removed += 1;
        }
        for path in [self.aggregates_file(), self.limits_file()] {
            if remove_if_there(&path)? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// The bytes the statistics take on the disk.
    #[must_use]
    pub fn size(&self) -> u64 {
        let mut paths: Vec<PathBuf> = self
            .months()
            .unwrap_or_default()
            .iter()
            .map(|month| self.events_file(month))
            .collect();
        paths.push(self.aggregates_file());
        paths.push(self.limits_file());
        paths
            .iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len())
            .sum()
    }
}

fn remove_if_there(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Whether `text` is `YYYY-MM`.
fn is_month(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 7
        && bytes[4] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(at, byte)| at == 4 || byte.is_ascii_digit())
}

/// The month (`YYYY-MM`, UTC) a moment falls in: the monthly file its event
/// goes to.
#[must_use]
pub fn month_of(time: UtcTime) -> String {
    time.to_string().chars().take(7).collect()
}

/// A month's number, counted from the year 0, for retention.
fn month_number(month: &str) -> Option<i64> {
    let year: i64 = month.get(0..4)?.parse().ok()?;
    let month: i64 = month.get(5..7)?.parse().ok()?;
    Some(year * 12 + month - 1)
}

/// What one monthly file has added, and how far it was read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePart {
    /// The byte after the last whole line that was read.
    pub offset: u64,
    /// What the lines up to there add up to.
    pub part: Part,
}

/// The aggregates file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aggregates {
    /// The version of the file.
    pub version: u32,
    /// Each monthly file still on the disk, by month.
    #[serde(default)]
    pub files: BTreeMap<String, FilePart>,
    /// What the files already pruned added up to.
    #[serde(default)]
    pub kept: Part,
}

impl Aggregates {
    /// Everything counted: the pruned files' part and every file's.
    #[must_use]
    pub fn total(&self) -> Part {
        let mut total = self.kept.clone();
        for file in self.files.values() {
            total.merge(&file.part);
        }
        total
    }

    /// Reads the aggregates file. A missing file is empty aggregates; a
    /// file that cannot be read, or of another version, gives empty
    /// aggregates too, which [`sync`](Self::sync) then fills from the
    /// monthly files (the part of files already pruned is lost, and
    /// `unreadable` says so).
    ///
    /// # Errors
    ///
    /// When the file is there and cannot be opened.
    pub fn load(dir: &StatsDir) -> io::Result<Self> {
        let path = dir.aggregates_file();
        let mut file = match open_to_read(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::fresh());
            }
            Err(error) => return Err(error),
        };
        let mut text = Vec::new();
        file.read_to_end(&mut text)?;
        match serde_json::from_slice::<Self>(&text) {
            Ok(read) if read.version == AGGREGATES_VERSION => Ok(read),
            _ => {
                let mut fresh = Self::fresh();
                fresh.kept.unreadable = 1;
                Ok(fresh)
            }
        }
    }

    fn fresh() -> Self {
        Self {
            version: AGGREGATES_VERSION,
            ..Self::default()
        }
    }

    /// Brings the aggregates up to date with the monthly files: reads each
    /// from where it was last read to its last whole line, keeps the part
    /// of a file someone else deleted, prunes files more than `retention`
    /// months older than `this_month`, and folds old days into months.
    /// Returns whether anything changed.
    ///
    /// # Errors
    ///
    /// When a file cannot be read or removed.
    pub fn sync(
        &mut self,
        dir: &StatsDir,
        today: LocalDate,
        this_month: &str,
        retention: u32,
    ) -> io::Result<bool> {
        let before = self.clone();
        let months = dir.months()?;
        // A file in the aggregates that is gone was pruned by another
        // process, which kept its part as it was: so is it here.
        let gone: Vec<String> = self
            .files
            .keys()
            .filter(|month| !months.contains(month))
            .cloned()
            .collect();
        for month in gone {
            if let Some(file) = self.files.remove(&month) {
                self.kept.merge(&file.part);
            }
        }
        for month in &months {
            let entry = self.files.entry(month.clone()).or_default();
            read_from(&dir.events_file(month), entry)?;
        }
        // Retention: a file stays for `retention` whole months after its
        // own.
        let now = month_number(this_month).unwrap_or(i64::MAX);
        for month in &months {
            let Some(number) = month_number(month) else {
                continue;
            };
            if number + i64::from(retention) < now {
                if let Some(file) = self.files.remove(month) {
                    self.kept.merge(&file.part);
                }
                remove_if_there(&dir.events_file(month))?;
            }
        }
        self.kept.fold_days(today);
        for file in self.files.values_mut() {
            file.part.fold_days(today);
        }
        Ok(*self != before)
    }

    /// The aggregates the monthly files give from nothing, beside the part
    /// of the files already pruned: what [`sync`](Self::sync) must agree
    /// with.
    ///
    /// # Errors
    ///
    /// When a file cannot be read.
    pub fn rebuild(&self, dir: &StatsDir, today: LocalDate) -> io::Result<Self> {
        let mut rebuilt = Self {
            version: AGGREGATES_VERSION,
            files: BTreeMap::new(),
            kept: self.kept.clone(),
        };
        for month in dir.months()? {
            let mut entry = FilePart::default();
            read_from(&dir.events_file(&month), &mut entry)?;
            entry.part.fold_days(today);
            rebuilt.files.insert(month, entry);
        }
        rebuilt.kept.fold_days(today);
        Ok(rebuilt)
    }

    /// Writes the aggregates file, owner-only and atomically.
    ///
    /// # Errors
    ///
    /// When it cannot be written.
    pub fn save(&self, dir: &StatsDir, clock: &dyn Clock) -> io::Result<()> {
        dir.ensure()?;
        let text = serde_json::to_vec(self).map_err(io::Error::other)?;
        write_private(&dir.aggregates_file(), &text, clock)
    }
}

/// Reads a monthly file from `entry.offset` to its last whole line, adding
/// each line to the entry's part.
fn read_from(path: &Path, entry: &mut FilePart) -> io::Result<()> {
    let mut file = match open_to_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let length = file.metadata()?.len();
    if length < entry.offset {
        // The file is shorter than what was read of it: it was replaced.
        // What was counted stays counted; reading starts again at its
        // start would count it twice, so the file is read from its end.
        entry.offset = length;
        return Ok(());
    }
    file.seek(SeekFrom::Start(entry.offset))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 || line.last() != Some(&b'\n') {
            // The end, or a line another process is still writing.
            break;
        }
        entry.offset += read as u64;
        match serde_json::from_slice::<Line>(&line) {
            Ok(parsed) => entry.part.add_line(&parsed),
            Err(_) => entry.part.unreadable += 1,
        }
    }
    Ok(())
}

/// Replaces a file's content, owner-only and atomically.
pub(crate) fn write_private(path: &Path, contents: &[u8], clock: &dyn Clock) -> io::Result<()> {
    let options = AtomicWriteOptions {
        owner_only: true,
        // Both files can be rebuilt (the aggregates) or asked again (the
        // limits): no flush to the disk for them.
        sync: false,
        follow_links: false,
        ..AtomicWriteOptions::default()
    };
    write_atomic(path, contents, &options, clock).map_err(io::Error::other)
}

/// Appends lines to monthly files, one open file at a time.
#[derive(Debug, Default)]
pub(crate) struct Appender {
    open: Option<(String, File)>,
}

impl Appender {
    /// Appends one whole line, newline included, to `month`'s file.
    pub(crate) fn append(&mut self, dir: &StatsDir, month: &str, line: &[u8]) -> io::Result<()> {
        if self.open.as_ref().is_none_or(|(open, _)| open != month) {
            dir.ensure()?;
            let path = dir.events_file(month);
            // A new file is made private as it is made; one that is there
            // keeps what it was given.
            match private::create_new(&path) {
                Ok(file) => drop(file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            let file = OpenOptions::new().append(true).open(&path)?;
            self.open = Some((month.to_owned(), file));
        }
        if let Some((_, file)) = &mut self.open {
            file.write_all(line)?;
        }
        Ok(())
    }

    /// Lets the open file go.
    pub(crate) fn close(&mut self) {
        self.open = None;
    }
}

/// The limits file: the latest figures of each provider, by where they
/// came from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitsFile {
    /// One entry for each provider and source, the latest seen.
    #[serde(default)]
    pub providers: Vec<ProviderLimits>,
}

impl LimitsFile {
    /// Reads the limits file. Missing or unreadable, it is empty: limits
    /// are asked again by the next request.
    #[must_use]
    pub fn load(dir: &StatsDir) -> Self {
        let Ok(mut file) = open_to_read(&dir.limits_file()) else {
            return Self::default();
        };
        let mut text = Vec::new();
        if file.read_to_end(&mut text).is_err() {
            return Self::default();
        }
        serde_json::from_slice(&text).unwrap_or_default()
    }

    /// Takes newer figures: an entry replaces the one of the same provider
    /// and source when it was seen later.
    pub fn merge(&mut self, newer: impl IntoIterator<Item = ProviderLimits>) {
        for limits in newer {
            match self
                .providers
                .iter_mut()
                .find(|known| known.provider == limits.provider && known.source == limits.source)
            {
                Some(known) => {
                    if seen(&limits) >= seen(known) {
                        *known = limits;
                    }
                }
                None => self.providers.push(limits),
            }
        }
        self.providers.sort_by(|a, b| {
            (a.provider.as_str(), a.source.as_str()).cmp(&(b.provider.as_str(), b.source.as_str()))
        });
    }

    /// Reads the file, takes `newer` into it, and writes it back.
    ///
    /// # Errors
    ///
    /// When it cannot be written.
    pub fn update(
        dir: &StatsDir,
        newer: impl IntoIterator<Item = ProviderLimits>,
        clock: &dyn Clock,
    ) -> io::Result<()> {
        let mut file = Self::load(dir);
        file.merge(newer);
        dir.ensure()?;
        let text = serde_json::to_vec(&file).map_err(io::Error::other)?;
        write_private(&dir.limits_file(), &text, clock)
    }
}

fn seen(limits: &ProviderLimits) -> Option<UtcTime> {
    UtcTime::parse(limits.seen_at.as_str()).ok()
}
