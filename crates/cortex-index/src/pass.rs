//! A pass: the workspace listed, set beside the cache, the files that
//! changed read again, and the cache written back.
//!
//! **Only what changed is read.** A file whose stamp (size, modification
//! time, and on Unix the inode's change time) is the cached one, and which
//! was settled when it was read, keeps its outline without being opened.
//! Everything else is opened (never through a link, never waiting on a
//! FIFO), read up to the cap and outlined, on a few threads.
//!
//! **A pass never makes the cache worse.** One whose listing was cut short
//! (the session ended, a turn began, the time ran out while listing) writes
//! nothing: the files it did not see would lose their outlines. One whose
//! listing was whole but that stopped while reading writes what it has:
//! the files it did not reach have no record, and the next pass reads
//! them. That is how a pass that runs out of time carries on.
//!
//! **Two passes at once** (two sessions on one project) each write a whole
//! cache, by an atomic replace: the last one wins, and both are valid. What
//! the first one read and the second did not have is read again by a later
//! pass.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a pass blocks on the file system, and runs on the blocking pool"
)]

use std::io::Read as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cortex_base::{AbsPath, AtomicWriteOptions, SharedClock, open_regular, private, write_atomic};

use crate::cache::{CacheFile, Flags, Writer};
use crate::extract::extract;
use crate::language::Language;
use crate::outline::Outline;
use crate::text::{decode, is_binary};
use crate::walk::{self, Control, Gate, Limits, Listed, Partial, Skipped, Stamp};

/// How close to the pass's own time a file's modification may be for a
/// later change to go unseen: file systems record times in steps as coarse
/// as two seconds.
const UNSETTLED: Duration = Duration::from_secs(2);

/// The name of the cache file in its directory.
pub const CACHE_FILE: &str = "outlines";

/// The most threads a pass reads files on: it is background work.
const MAX_THREADS: usize = 4;

/// A project's index: where its workspace is, where its cache is, and
/// what a pass may do. It holds no outlines: each use reads the cache.
#[derive(Clone)]
pub struct Index {
    root: AbsPath,
    dir: PathBuf,
    limits: Limits,
    gate: Arc<dyn Gate>,
    clock: SharedClock,
    threads: usize,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index")
            .field("root", &self.root)
            .field("dir", &self.dir)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// What a pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassReport {
    /// Files the listing found, the ones past the cap included.
    pub found: usize,
    /// Files the cache now has a record of.
    pub indexed: usize,
    /// Files opened and outlined in this pass.
    pub read: usize,
    /// Files whose cached record held.
    pub kept: usize,
    /// Why the pass covers less than the whole workspace.
    pub partial: Vec<Partial>,
    /// Whether the cache was written.
    pub written: bool,
    /// What the listing stepped past.
    pub skipped: Skipped,
    /// The size of the cache now, in bytes.
    pub cache_bytes: usize,
}

impl PassReport {
    /// Whether the pass covered all it could: no cap bit and nothing
    /// stopped it.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.partial.is_empty()
    }

    /// Whether a later pass has more to do: this one ran out of time or was
    /// stopped. A cap on files is not that: the next pass meets it too.
    #[must_use]
    pub fn unfinished(&self) -> bool {
        self.partial
            .iter()
            .any(|why| matches!(why, Partial::Time | Partial::Stopped))
    }
}

/// A file a pass must read, and what it found.
struct Work<'a> {
    file: &'a Listed,
    found: Option<(Stamp, Flags, Outline)>,
}

impl Index {
    /// The index of the workspace at `root`, a resolved root, with its
    /// cache in `dir` (`<state>/projects/<project key>/index`).
    #[must_use]
    pub fn new(root: AbsPath, dir: PathBuf, limits: Limits, gate: Arc<dyn Gate>, clock: SharedClock) -> Self {
        let threads = std::thread::available_parallelism()
            .map_or(1, |cores| (cores.get() / 2).clamp(1, MAX_THREADS));
        Self {
            root,
            dir,
            limits,
            gate,
            clock,
            threads,
        }
    }

    /// Reads files on this many threads.
    #[must_use]
    pub fn with_threads(mut self, threads: usize) -> Self {
        self.threads = threads.max(1);
        self
    }

    /// The workspace's root.
    #[must_use]
    pub fn root(&self) -> &AbsPath {
        &self.root
    }

    /// The caps.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The cache file.
    #[must_use]
    pub fn cache_path(&self) -> PathBuf {
        self.dir.join(CACHE_FILE)
    }

    /// The cache as it is now, if it can be trusted.
    pub(crate) fn cache(&self) -> Option<CacheFile> {
        CacheFile::read(&self.cache_path(), self.root.as_str())
    }

    /// One pass over the workspace. Blocks; runs on the blocking pool.
    ///
    /// `control` says when to stop between files: the session's end, a
    /// turn that began, the time.
    pub fn pass(&self, cancel: &cortex_base::CancelToken, give_way: &(dyn Fn() -> bool + Sync)) -> PassReport {
        let started = self.clock.monotonic();
        let limit = self.limits.pass_time;
        let clock = Arc::clone(&self.clock);
        let out_of_time = move || clock.monotonic().saturating_sub(started) >= limit;
        let control = Control {
            cancel,
            give_way,
            out_of_time: &out_of_time,
        };
        let now = {
            let now = self.clock.now();
            i128::from(now.unix_secs()) * 1_000_000_000 + i128::from(now.subsec_nanos())
        };
        let listing = walk::list(&self.root, &self.gate, &self.limits, &control);
        let mut report = PassReport {
            found: listing.found,
            skipped: listing.skipped,
            partial: listing.partial.into_iter().collect(),
            ..PassReport::default()
        };
        if matches!(listing.partial, Some(Partial::Time | Partial::Stopped)) {
            // The listing is not the workspace: writing it would drop the
            // records of the files it did not reach.
            report.indexed = self.cache().map_or(0, |cache| cache.records.len());
            return report;
        }

        let old = self.cache();
        // Which files keep their record, and which must be read.
        let mut kept: Vec<Option<usize>> = Vec::with_capacity(listing.files.len());
        let mut work: Vec<Work<'_>> = Vec::new();
        for file in &listing.files {
            let held = old.as_ref().and_then(|old| {
                let at = old
                    .records
                    .binary_search_by(|record| walk::path_order(&record.path, &file.relative))
                    .ok()?;
                let record = &old.records[at];
                (record.stamp == file.stamp && !record.flags.unsettled).then_some(at)
            });
            if held.is_none() {
                work.push(Work { file, found: None });
            }
            kept.push(held);
        }

        let stopped = self.read_all(&mut work, &control, now);
        if let Some(why) = stopped {
            report.partial.push(why);
        }

        // The new cache, in the listing's order.
        let indexed = kept.iter().filter(|held| held.is_some()).count()
            + work.iter().filter(|work| work.found.is_some()).count();
        let mut writer = Writer::new(self.root.as_str(), listing.found, indexed, &report.partial);
        for (dir, files) in &listing.dirs {
            writer.dir(dir, *files);
        }
        let mut done = work.iter();
        for (file, held) in listing.files.iter().zip(&kept) {
            match (held, &old) {
                (Some(at), Some(old)) => {
                    writer.copy(old.lines_of(&old.records[*at]));
                    report.kept += 1;
                }
                _ => {
                    let Some(work) = done.next() else { continue };
                    if let Some((stamp, flags, outline)) = &work.found {
                        writer.file(&file.relative, stamp, *flags, outline);
                        report.read += 1;
                    }
                }
            }
        }
        report.indexed = indexed;
        let text = writer.finish();
        report.cache_bytes = text.len();
        let unchanged = old.as_ref().is_some_and(|old| {
            report.read == 0
                && old.records.len() == indexed
                && old.found == listing.found
                && old.partial == report.partial
                && old.dirs.len() == listing.dirs.len()
                && old
                    .dirs
                    .iter()
                    .zip(&listing.dirs)
                    .all(|((one, n), (other, m))| one == other && n == m)
        });
        if !unchanged {
            report.written = self.write(&text);
        }
        report
    }

    /// Reads and outlines each file of `work`, on a few threads. Returns
    /// why it stopped early, if it did.
    fn read_all(&self, work: &mut [Work<'_>], control: &Control<'_>, now: i128) -> Option<Partial> {
        let next = AtomicUsize::new(0);
        let stopped: std::sync::Mutex<Option<Partial>> = std::sync::Mutex::new(None);
        let results: Vec<std::sync::Mutex<Option<(Stamp, Flags, Outline)>>> =
            work.iter().map(|_| std::sync::Mutex::new(None)).collect();
        let threads = self.threads.min(work.len()).max(1);
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        if let Some(why) = control.stop() {
                            let mut stopped = stopped.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                            stopped.get_or_insert(why);
                            return;
                        }
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = work.get(at) else { return };
                        let found = self.read_one(item.file, now);
                        if let Some(slot) = results.get(at) {
                            *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = found;
                        }
                    }
                });
            }
        });
        for (item, slot) in work.iter_mut().zip(results) {
            item.found = slot.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        stopped.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reads one file and outlines it. `None` when it could not be opened
    /// as a regular file: it vanished, it is a link now, it is not readable.
    fn read_one(&self, file: &Listed, now: i128) -> Option<(Stamp, Flags, Outline)> {
        let (handle, metadata) = open_regular(&file.fs_path).ok()?;
        let stamp = Stamp::of(&metadata);
        let window = i128::try_from(UNSETTLED.as_nanos()).unwrap_or(i128::MAX);
        let mut flags = Flags {
            unsettled: stamp.modified > now - window,
            ..Flags::default()
        };
        if stamp.size > self.limits.max_file_bytes {
            flags.large = true;
            return Some((stamp, flags, Outline::default()));
        }
        // Read what the file held when it was opened, and no more.
        let mut bytes = Vec::with_capacity(usize::try_from(stamp.size).unwrap_or(0));
        handle.take(stamp.size).read_to_end(&mut bytes).ok()?;
        if is_binary(&bytes) {
            flags.binary = true;
            return Some((stamp, flags, Outline::default()));
        }
        let name = file.relative.rsplit('/').next().unwrap_or(&file.relative);
        let text = decode(&bytes);
        let outline = extract(Language::detect(name, &text), name, &text);
        flags.truncated = outline.truncated;
        Some((stamp, flags, outline))
    }

    /// Writes the cache, owner-only, by an atomic replace. Not flushed to
    /// the disk first: it can be rebuilt. Returns whether it was written.
    fn write(&self, text: &str) -> bool {
        if private::create_dir_all(&self.dir).is_err() {
            return false;
        }
        let options = AtomicWriteOptions {
            sync: false,
            owner_only: true,
            follow_links: false,
            ..AtomicWriteOptions::default()
        };
        write_atomic(&self.cache_path(), text.as_bytes(), &options, &*self.clock).is_ok()
    }
}
