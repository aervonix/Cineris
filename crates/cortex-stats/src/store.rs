//! The statistics directory: monthly files of events, the aggregates file,
//! and the limits the providers last reported
//! (`docs/decisions/0021-local-statistics.md`).
//!
//! | File | What it holds |
//! |---|---|
//! | `events-YYYY-MM.jsonl` | One event a line, appended, by the month (UTC) it happened in. |
//! | `aggregates.json` | Each monthly file's [`Part`] and how far it was read, and the part of the files already pruned (`kept`). |
//! | `aggregates.json.unreadable-…` | An aggregates file this build could not read, moved aside with the time it was moved (`2026-11-15T10-00-00Z`), owner-only, for the user to keep. |
//! | `limits.json` | The latest limits and balances each provider reported, with when and how. |
//! | `limits.json.lock` | The lock its writers take ([`cortex_base::private::hold_lock`]). |
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
//! **`kept` is the one copy of every pruned month,** so the aggregates file
//! is flushed to the disk before it takes its name, and a file this build
//! cannot read is never written over ([`Found`]; found by the independent
//! review of step 16): one of a newer version is left as it is, and one
//! that does not parse is moved aside before anything is saved.
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

use cortex_base::{
    AtomicWriteOptions, Clock, FileIdentity, LocalDate, UtcTime, private, write_atomic,
};
use cortex_protocol::usage::ProviderLimits;
use serde::{Deserialize, Serialize};

use crate::aggregate::Part;
use crate::record::Line;

/// The version of the aggregates file this build writes. A file of a
/// newer version is left as it is ([`Found::Newer`]).
pub const AGGREGATES_VERSION: u32 = 1;

/// The start of the name an unreadable aggregates file is moved aside to.
const SET_ASIDE: &str = "aggregates.json.unreadable-";

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

pub(crate) fn open_to_read(path: &Path) -> io::Result<File> {
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
        let mut months: Vec<String> = self
            .names()?
            .iter()
            .filter_map(|name| {
                let month = name.strip_prefix("events-")?.strip_suffix(".jsonl")?;
                is_month(month).then(|| month.to_owned())
            })
            .collect();
        months.sort();
        Ok(months)
    }

    /// The aggregates files that could not be read and were moved aside,
    /// oldest first: each still holds the part of the months pruned before
    /// it was.
    ///
    /// # Errors
    ///
    /// When the directory cannot be listed. A missing one has none.
    pub fn set_aside(&self) -> io::Result<Vec<PathBuf>> {
        let mut names: Vec<String> = self
            .names()?
            .into_iter()
            .filter(|name| name.starts_with(SET_ASIDE))
            .collect();
        names.sort();
        Ok(names.iter().map(|name| self.root.join(name)).collect())
    }

    fn names(&self) -> io::Result<Vec<String>> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        Ok(entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect())
    }

    /// Deletes every statistics file: `cortex stats reset`. The aggregates
    /// files moved aside go with the rest; the lock file stays, as another
    /// process may hold it.
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
        let mut paths = vec![self.aggregates_file(), self.limits_file()];
        paths.extend(self.set_aside()?);
        for path in paths {
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
        paths.extend(self.set_aside().unwrap_or_default());
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

/// What [`Aggregates::load`] found where the aggregates file is, which
/// decides whether the aggregates may be saved and may prune.
///
/// `kept` holds the only copy of every pruned month, so aggregates that
/// started from nothing because the file could not be read must never be
/// saved over it: saving and pruning check this, so no caller can forget
/// to (found by the independent review of step 16).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Found {
    /// Aggregates that were not loaded from a directory: never saved.
    #[default]
    NotLoaded,
    /// No file: the aggregates start from nothing.
    Missing,
    /// The file, read.
    Read,
    /// A file of a version this build does not read (a newer build's), left
    /// as it is, with the part of every pruned month in it. These
    /// aggregates are the monthly files' alone, and are neither saved nor
    /// pruned.
    Newer {
        /// The file's version.
        version: u32,
    },
    /// A file that could not be read as aggregates, left as it is by a
    /// caller that only reads ([`Aggregates::load`]). These start from the
    /// monthly files, and are neither saved nor pruned.
    Unreadable,
    /// A file that could not be read, moved aside to `to`, owner-only: the
    /// part of the months already pruned is in it. These aggregates start
    /// from the monthly files, and may be saved.
    MovedAside {
        /// Where it is now.
        to: PathBuf,
    },
}

impl Found {
    /// Whether aggregates loaded so may be saved.
    #[must_use]
    pub const fn may_save(&self) -> bool {
        matches!(self, Self::Missing | Self::Read | Self::MovedAside { .. })
    }

    /// Whether aggregates loaded so may prune: only those read from the
    /// file. Aggregates that started from nothing are saved first and prune
    /// at the next sync, from what they saved: two processes that both
    /// started from nothing (a first run, or a file moved aside) could
    /// otherwise each prune a month the other's save then leaves out
    /// (found by the review of step 16's fixes).
    #[must_use]
    pub const fn may_prune(&self) -> bool {
        matches!(self, Self::Read)
    }
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
    /// What was there when these were loaded.
    #[serde(skip)]
    found: Found,
}

/// What reading the aggregates file gave.
enum Reading {
    Missing,
    Parsed(Aggregates),
    Newer(u32),
    /// Which file it was.
    Unparsed(Option<FileIdentity>),
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

    /// What was there when these were loaded.
    #[must_use]
    pub fn found(&self) -> &Found {
        &self.found
    }

    /// Reads the aggregates file, changing nothing on the disk: for a caller
    /// that reads (`cortex stats`, a report with statistics off, a day's
    /// limits). A missing file is empty aggregates. A file of a newer
    /// version, or one that does not parse, gives empty aggregates too,
    /// which [`sync`](Self::sync) fills from the monthly files, and which
    /// may not be saved ([`Found`]).
    ///
    /// # Errors
    ///
    /// When the file is there and cannot be opened or read.
    pub fn load(dir: &StatsDir) -> io::Result<Self> {
        Ok(match read_aggregates(dir)? {
            Reading::Missing => Self::fresh(Found::Missing),
            Reading::Parsed(read) => read,
            Reading::Newer(version) => Self::fresh(Found::Newer { version }),
            Reading::Unparsed(..) => Self::fresh(Found::Unreadable),
        })
    }

    /// Reads the aggregates file for a caller that will save what it reads:
    /// the writer, and a report with statistics on. As [`load`](Self::load),
    /// except that a file that does not parse is first moved aside
    /// (`aggregates.json.unreadable-<when>`, owner-only, with what it held),
    /// so that saving cannot lose it, and the log says where. A file of a
    /// newer version is still left alone: the newer build reads it.
    ///
    /// # Errors
    ///
    /// When the file is there and cannot be opened or read. A file that
    /// cannot be moved aside gives aggregates that may not be saved.
    pub fn load_for_saving(dir: &StatsDir, clock: &dyn Clock) -> io::Result<Self> {
        // Twice at most: once more when another process replaced the file
        // between the read and the move.
        for _ in 0..2 {
            let identity = match read_aggregates(dir)? {
                Reading::Missing => return Ok(Self::fresh(Found::Missing)),
                Reading::Parsed(read) => return Ok(read),
                Reading::Newer(version) => return Ok(Self::fresh(Found::Newer { version })),
                Reading::Unparsed(identity) => identity,
            };
            match move_aside(dir, identity, clock) {
                Ok(Some(to)) => {
                    tracing::warn!(
                        moved_to = %to.display(),
                        "the statistics' aggregates file could not be read, and was moved aside: \
                         the months pruned before are in it"
                    );
                    return Ok(Self::fresh(Found::MovedAside { to }));
                }
                // Not the file that was read any more: read again.
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "the statistics' aggregates file could not be read or moved aside, so it \
                         is left as it is and not written"
                    );
                    return Ok(Self::fresh(Found::Unreadable));
                }
            }
        }
        Ok(Self::fresh(Found::Unreadable))
    }

    fn fresh(found: Found) -> Self {
        Self {
            version: AGGREGATES_VERSION,
            found,
            ..Self::default()
        }
    }

    /// Brings the aggregates up to date with the monthly files: reads each
    /// from where it was last read to its last whole line, keeps the part
    /// of a file someone else deleted, prunes files more than `retention`
    /// months older than `this_month` (with `retention` given, and only
    /// when these aggregates were read from the file: `None` only reads, for
    /// a caller that will not save what it read), and folds old days into
    /// months.
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
        retention: Option<u32>,
    ) -> io::Result<bool> {
        // A pruned file's part lives on only in what is saved: aggregates
        // that were not read from the file prune nothing.
        let retention = retention.filter(|_| self.found.may_prune());
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
        if let Some(retention) = retention {
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
            found: self.found.clone(),
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

    /// Writes the aggregates file, owner-only and atomically, flushed to
    /// the disk before it takes its name: a power cut cannot leave it
    /// empty, and `kept` with it.
    ///
    /// # Errors
    ///
    /// When it cannot be written, and when these aggregates may not be
    /// saved ([`Found::may_save`]): they did not read the file that is
    /// there, which is left as it is.
    pub fn save(&self, dir: &StatsDir, clock: &dyn Clock) -> io::Result<()> {
        if !self.found.may_save() {
            return Err(io::Error::other(format!(
                "the aggregates were not saved: {}",
                match &self.found {
                    Found::Newer { version } => format!(
                        "the file is a newer build's (version {version}), and is left for it"
                    ),
                    Found::Unreadable => {
                        "the file could not be read, and is left as it is".to_owned()
                    }
                    _ => "they were not loaded from the directory".to_owned(),
                }
            )));
        }
        dir.ensure()?;
        let text = serde_json::to_vec(self).map_err(io::Error::other)?;
        let options = AtomicWriteOptions {
            owner_only: true,
            // `kept` is the one copy of every pruned month: no rename
            // before the content is on the disk.
            sync: true,
            follow_links: false,
            ..AtomicWriteOptions::default()
        };
        write_atomic(&dir.aggregates_file(), &text, &options, clock).map_err(io::Error::other)
    }
}

/// Reads the aggregates file, and says what it is.
fn read_aggregates(dir: &StatsDir) -> io::Result<Reading> {
    let path = dir.aggregates_file();
    let mut file = match open_to_read(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Reading::Missing),
        Err(error) => return Err(error),
    };
    let identity = FileIdentity::of(&file).ok();
    let mut text = Vec::new();
    file.read_to_end(&mut text)?;
    // The version first, from a file that may be a later build's whole
    // other shape.
    #[derive(Deserialize)]
    struct Version {
        version: u32,
    }
    Ok(match serde_json::from_slice::<Version>(&text) {
        Ok(Version { version }) if version > AGGREGATES_VERSION => Reading::Newer(version),
        Ok(Version { version }) if version == AGGREGATES_VERSION => {
            match serde_json::from_slice::<Aggregates>(&text) {
                Ok(mut read) => {
                    read.found = Found::Read;
                    Reading::Parsed(read)
                }
                Err(_) => Reading::Unparsed(identity),
            }
        }
        _ => Reading::Unparsed(identity),
    })
}

/// Moves the aggregates file aside, when it is still the file that was
/// read (`identity`), to a name of its own with the time in it, by a
/// rename: nothing is deleted, whatever another process does meanwhile.
/// Returns where, or `None` when another file had the name. The file moved
/// is owner-only afterwards, whatever it was.
fn move_aside(
    dir: &StatsDir,
    identity: Option<FileIdentity>,
    clock: &dyn Clock,
) -> io::Result<Option<PathBuf>> {
    let path = dir.aggregates_file();
    if identity.is_none() || FileIdentity::at(&path).ok() != identity {
        return Ok(None);
    }
    let stamp = clock.now().to_string().replace(':', "-");
    let mut to = dir.root().join(format!("{SET_ASIDE}{stamp}"));
    let mut n = 2;
    while to.exists() {
        to = dir.root().join(format!("{SET_ASIDE}{stamp}-{n}"));
        n += 1;
    }
    std::fs::rename(&path, &to)?;
    let moved = FileIdentity::at(&to).ok();
    if !private::is_private(&to).unwrap_or(false) {
        let mut bytes = Vec::new();
        open_to_read(&to)?.read_to_end(&mut bytes)?;
        let options = AtomicWriteOptions {
            owner_only: true,
            sync: true,
            follow_links: false,
            ..AtomicWriteOptions::default()
        };
        write_atomic(&to, &bytes, &options, clock).map_err(io::Error::other)?;
    }
    if moved != identity {
        // Another process wrote the file between the look and the rename:
        // it is kept where it went, and the user is told of it as of any
        // file moved aside. Nothing was lost.
        tracing::warn!(
            moved_to = %to.display(),
            "an aggregates file another process had just written was moved aside with the \
             unreadable one it replaced"
        );
    }
    Ok(Some(to))
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
    let part = &mut entry.part;
    read_lines(
        &mut file,
        &mut entry.offset,
        |line| match serde_json::from_slice::<Line>(line) {
            Ok(parsed) => part.add_line(&parsed),
            Err(_) => part.unreadable = part.unreadable.saturating_add(1),
        },
    )
}

/// Reads `file` from `offset` to its last whole line, giving each whole line
/// (its newline included) to `each` and moving `offset` past it. A last
/// line with no newline yet is left for later: another process may still
/// be writing it.
pub(crate) fn read_lines(
    file: &mut File,
    offset: &mut u64,
    mut each: impl FnMut(&[u8]),
) -> io::Result<()> {
    file.seek(SeekFrom::Start(*offset))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 || line.last() != Some(&b'\n') {
            // The end, or a line another process is still writing.
            break;
        }
        *offset += read as u64;
        // A line break with nothing before it is not a line: two writers
        // that both ended a cut line, or one that saw a line crossing a
        // page as cut, leave one.
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        each(&line);
    }
    Ok(())
}

/// Replaces the limits file's content, owner-only and atomically. Not
/// flushed to the disk first: the limits are asked again by the next
/// request, so a power cut costs nothing that is not told again.
fn write_limits(path: &Path, contents: &[u8], clock: &dyn Clock) -> io::Result<()> {
    let options = AtomicWriteOptions {
        owner_only: true,
        sync: false,
        follow_links: false,
        ..AtomicWriteOptions::default()
    };
    write_atomic(path, contents, &options, clock).map_err(io::Error::other)
}

/// Appends lines to monthly files, one open file at a time.
#[derive(Debug, Default)]
pub(crate) struct Appender {
    open: Option<Open>,
}

#[derive(Debug)]
struct Open {
    month: String,
    file: File,
    /// Which file it is: when the name names another, or none, the file is
    /// opened again.
    identity: FileIdentity,
}

impl Appender {
    /// Appends one whole line, newline included, to `month`'s file.
    ///
    /// The file is the one the name gives now. One held open that is no
    /// longer it, because `cortex stats reset` deleted it or something
    /// replaced it, is let go first, so that no line goes to a file nobody
    /// can read (found by the independent review of step 16). On Windows a
    /// file deleted while this writer held it cannot be looked at until it
    /// is let go, which this does.
    pub(crate) fn append(&mut self, dir: &StatsDir, month: &str, line: &[u8]) -> io::Result<()> {
        let path = dir.events_file(month);
        if let Some(open) = &self.open
            && (open.month != month
                || FileIdentity::at(&path).map_or(true, |now| now != open.identity))
        {
            self.open = None;
        }
        let open = match &mut self.open {
            Some(open) => open,
            None => self.open.insert(open_to_append(dir, month, &path)?),
        };
        open.file.write_all(line)
    }

    /// Lets the open file go.
    pub(crate) fn close(&mut self) {
        self.open = None;
    }
}

/// Opens `month`'s file to append to it, making it private if it is new.
/// A last line cut short (a full disk, a power cut) is ended first, so
/// that it does not take this writer's first line with it (found by the
/// independent review of step 16): it stays one unreadable line.
fn open_to_append(dir: &StatsDir, month: &str, path: &Path) -> io::Result<Open> {
    dir.ensure()?;
    // A new file is made private as it is made; one that is there keeps
    // what it was given.
    match private::create_new(path) {
        Ok(file) => drop(file),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let mut file = OpenOptions::new().read(true).append(true).open(path)?;
    let identity = FileIdentity::of(&file)?;
    let length = file.metadata()?.len();
    if length > 0 {
        let mut last = [0u8; 1];
        file.seek(SeekFrom::Start(length - 1))?;
        file.read_exact(&mut last)?;
        if last != *b"\n" {
            file.write_all(b"\n")?;
        }
    }
    Ok(Open {
        month: month.to_owned(),
        file,
        identity,
    })
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

    /// Reads the file, takes `newer` into it, and writes it back, holding
    /// the file's lock from before the read until after the write, so that
    /// another process's update between the two is not written over (found
    /// by the independent review of step 16).
    ///
    /// # Errors
    ///
    /// When the lock cannot be taken, when the file cannot be read for any
    /// reason but its not being there (it is then left as it is, so that a
    /// passing failure to read it cannot lose the other providers'
    /// figures), or when it cannot be written.
    pub fn update(
        dir: &StatsDir,
        newer: impl IntoIterator<Item = ProviderLimits>,
        clock: &dyn Clock,
    ) -> io::Result<()> {
        dir.ensure()?;
        let path = dir.limits_file();
        let _held = private::hold_lock(&path)?;
        let mut file = match open_to_read(&path) {
            Ok(mut opened) => {
                let mut text = Vec::new();
                opened.read_to_end(&mut text)?;
                // A file that is not one this build reads is replaced: it
                // holds nothing that is not asked again.
                serde_json::from_slice(&text).unwrap_or_default()
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error),
        };
        file.merge(newer);
        let text = serde_json::to_vec(&file).map_err(io::Error::other)?;
        write_limits(&path, &text, clock)
    }
}

fn seen(limits: &ProviderLimits) -> Option<UtcTime> {
    UtcTime::parse(limits.seen_at.as_str()).ok()
}
