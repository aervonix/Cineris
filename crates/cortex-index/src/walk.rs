//! The listing of a workspace that a pass indexes: what the ignore rules
//! and the permission gate leave, under the caps.
//!
//! - **The ignore rules** are the ones `search` and `list` honour
//!   (`docs/architecture/05-tools.md`): `.gitignore` at every level whether
//!   or not the tree is a git repository, `.ignore`, `.git/info/exclude`,
//!   and the hidden-file rule, which leaves out `.git` and `.cortex`. The
//!   user's global git ignore file is not read: it is the machine's.
//! - **The gate** is the permission engine's: a file or a directory the
//!   agent could not read unasked is not listed, so it is never indexed or
//!   named in the map.
//! - **Links are not followed** and not listed, wherever they point.
//! - **A name the path rules refuse** (not valid Unicode, a Windows device
//!   name) is stepped past, with everything under it.
//! - **On the pass's threads,** with the walker `search` and `list` use,
//!   each thread looking before each entry it is handed whether to stop, and
//!   the files put in [`path_order`] after: the same listing on any number
//!   of threads. One that meets its cap on entries is made again on one
//!   thread, in name order, so that a cut listing is the same on every
//!   machine.
//! - **The caps:** a listing stops at [`Limits::max_listed`] entries (not at
//!   the pass's time limit, which is the reading's). Past
//!   [`Limits::max_files`] files, the shallowest of what was listed are
//!   indexed.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a pass blocks on the file system, and runs on the blocking pool"
)]

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::time::Duration;

use cortex_base::{AbsPath, CancelToken};
use ignore::{WalkBuilder, WalkState};

/// The caps of a pass (`docs/architecture/15-efficiency.md`, "Cost
/// control").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most files a pass indexes.
    pub max_files: usize,
    /// The largest file that is read; a larger one is listed, not outlined.
    pub max_file_bytes: u64,
    /// The most time one pass spends reading files, after its listing. One
    /// that runs out carries on in the next.
    pub pass_time: Duration,
    /// The most entries a listing gathers, files and directories, so that
    /// a tree of millions costs a bounded walk.
    pub max_listed: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_file_bytes: 1024 * 1024,
            pass_time: Duration::from_secs(2),
            max_listed: 100_000,
        }
    }
}

/// Decides, path by path, what may be indexed: the permission engine's
/// "could the agent read this unasked?".
pub trait Gate: Send + Sync {
    /// Whether the file or directory at `path`, a real location inside the
    /// workspace with no link in it followed, may be indexed.
    fn may_index(&self, path: &AbsPath) -> bool;

    /// The gate one pass or one map asks, taken once at its start: for a
    /// gate whose answers come from something that may be replaced (the
    /// permission engine in force), so that what it costs to find that is
    /// paid once and not for every path. `None`, the default, is this gate
    /// itself.
    fn for_pass(&self) -> Option<Arc<dyn Gate>> {
        None
    }
}

/// `gate` as one pass or one map asks it ([`Gate::for_pass`]).
pub(crate) fn for_pass(gate: &Arc<dyn Gate>) -> Arc<dyn Gate> {
    gate.for_pass().unwrap_or_else(|| Arc::clone(gate))
}

impl<F> Gate for F
where
    F: Fn(&AbsPath) -> bool + Send + Sync,
{
    fn may_index(&self, path: &AbsPath) -> bool {
        self(path)
    }
}

/// A [`Gate`] that lets everything through.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenGate;

impl Gate for OpenGate {
    fn may_index(&self, _path: &AbsPath) -> bool {
        true
    }
}

/// What a file looked like when it was listed or opened: what decides
/// whether its cached outline still holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// Its size in bytes.
    pub size: u64,
    /// Its modification time, in nanoseconds from 1970 (before it, below
    /// zero). 0 where the file system does not say.
    pub modified: i128,
    /// On Unix, the inode and the inode's change time in nanoseconds: no
    /// program can set the change time back, so a file rewritten with its
    /// old size and modification time is still told apart. `None`
    /// elsewhere.
    pub change: Option<(u64, i128)>,
}

impl Stamp {
    /// The stamp of a file's metadata.
    #[must_use]
    pub fn of(metadata: &std::fs::Metadata) -> Self {
        let modified = metadata.modified().map_or(0, nanos);
        Self {
            size: metadata.len(),
            modified,
            change: change_of(metadata),
        }
    }
}

/// Nanoseconds from 1970 of a time.
pub(crate) fn nanos(time: std::time::SystemTime) -> i128 {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(after) => i128::try_from(after.as_nanos()).unwrap_or(i128::MAX),
        Err(before) => -i128::try_from(before.duration().as_nanos()).unwrap_or(i128::MAX),
    }
}

#[cfg(unix)]
fn change_of(metadata: &std::fs::Metadata) -> Option<(u64, i128)> {
    use std::os::unix::fs::MetadataExt;
    Some((
        metadata.ino(),
        i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec()),
    ))
}

#[cfg(not(unix))]
fn change_of(_: &std::fs::Metadata) -> Option<(u64, i128)> {
    None
}

/// One file the listing found.
#[derive(Debug, Clone)]
pub(crate) struct Listed {
    /// Its path under the workspace, with `/` between names.
    pub(crate) relative: String,
    /// How many directories deep it is: 0 at the root.
    pub(crate) depth: usize,
    /// What it looked like when it was listed.
    pub(crate) stamp: Stamp,
}

/// Why a listing or a pass covers less than the whole workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Partial {
    /// More files than the cap: the shallowest were indexed.
    Files,
    /// The listing stopped at its cap on entries.
    Listing,
    /// The pass ran out of time while reading; the next carries on.
    Time,
    /// The pass was stopped (the session ended, or a turn began).
    Stopped,
}

impl Partial {
    /// The letter the cache stores.
    pub(crate) fn letter(self) -> char {
        match self {
            Self::Files => 'f',
            Self::Listing => 'l',
            Self::Time => 't',
            Self::Stopped => 's',
        }
    }

    pub(crate) fn from_letter(letter: char) -> Option<Self> {
        Some(match letter {
            'f' => Self::Files,
            'l' => Self::Listing,
            't' => Self::Time,
            's' => Self::Stopped,
            _ => return None,
        })
    }
}

/// What was stepped past, by why.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skipped {
    /// Entries whose names the path rules refuse.
    pub refused: usize,
    /// Entries the gate does not let through.
    pub denied: usize,
    /// Entries the file system would not give.
    pub unreadable: usize,
    /// Links, which are not followed.
    pub links: usize,
}

/// A listing of the workspace.
#[derive(Debug, Default)]
pub(crate) struct Listing {
    /// The files to index, in [`path_order`].
    pub(crate) files: Vec<Listed>,
    /// How many files were found, the ones past the cap included.
    pub(crate) found: usize,
    /// Each directory with files under it, and how many it has at any
    /// depth, the ones past the cap included. The root is `""`.
    pub(crate) dirs: BTreeMap<String, usize>,
    pub(crate) partial: Option<Partial>,
    pub(crate) skipped: Skipped,
}

/// Where a file of the listing is, for the file functions: the root's path
/// with the file's names, one by one (on Windows the root's is the
/// extended-length form, in which only `\\` separates names).
pub(crate) fn fs_path_of(root_fs: &Path, relative: &str) -> PathBuf {
    let mut path = root_fs.to_path_buf();
    for name in relative.split('/') {
        path.push(name);
    }
    path
}

/// The order a listing and the cache keep files in: name by name, so that
/// a directory's files sort together whatever the names' characters.
///
/// It compares the bytes with `/` taken as the least of them, below any
/// byte a name can hold (a name holds no NUL): the same order as comparing
/// the names one by one, without splitting the paths.
#[must_use]
pub fn path_order(one: &str, other: &str) -> Ordering {
    let key = |byte: &u8| if *byte == b'/' { 0 } else { *byte };
    one.as_bytes()
        .iter()
        .map(key)
        .cmp(other.as_bytes().iter().map(key))
}

/// What tells a pass to stop between files.
#[derive(Clone, Copy)]
pub struct Control<'a> {
    /// Fires when the session ends.
    pub cancel: &'a CancelToken,
    /// Says, between files, that the pass should give way: a turn began
    /// and nobody waits for a map.
    pub give_way: &'a (dyn Fn() -> bool + Sync),
    /// Says whether the pass has run out of its time.
    pub out_of_time: &'a (dyn Fn() -> bool + Sync),
}

impl std::fmt::Debug for Control<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control").finish_non_exhaustive()
    }
}

impl Control<'_> {
    /// Why a listing should stop now, if it should: the session's end, or a
    /// turn to give way to. Not the time: a listing is bounded by its cap on
    /// entries, and one cut by the time would be begun again by every pass,
    /// and so never finish.
    pub(crate) fn stop_listing(&self) -> Option<Partial> {
        (self.cancel.is_cancelled() || (self.give_way)()).then_some(Partial::Stopped)
    }

    /// Why the pass should stop now, if it should.
    pub(crate) fn stop(&self) -> Option<Partial> {
        if self.cancel.is_cancelled() || (self.give_way)() {
            Some(Partial::Stopped)
        } else if (self.out_of_time)() {
            Some(Partial::Time)
        } else {
            None
        }
    }
}

/// The directory a run of entries is in, so that its path is checked
/// once.
struct Parent {
    fs_path: PathBuf,
    abs: AbsPath,
    relative: String,
    depth: usize,
}

fn parent_of(root_fs: &Path, root: &AbsPath, fs_parent: &Path) -> Option<Parent> {
    let below = fs_parent.strip_prefix(root_fs).ok()?;
    let mut abs = root.clone();
    let mut relative = String::new();
    let mut depth = 0;
    for component in below.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        abs = abs.child(name).ok()?;
        if !relative.is_empty() {
            relative.push('/');
        }
        relative.push_str(abs.file_name()?);
        depth += 1;
    }
    Some(Parent {
        fs_path: fs_parent.to_owned(),
        abs,
        relative,
        depth,
    })
}

/// The walker over the workspace with the ignore rules and the gate: the
/// same for the walk on one thread and the walk on several.
fn walker(root_fs: &Path, root: &AbsPath, gate: &Arc<dyn Gate>) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root_fs);
    builder
        .follow_links(false)
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false)
        .git_global(false);
    let gate_root = root.clone();
    let root_for_filter = root_fs.to_path_buf();
    let filter_gate = Arc::clone(gate);
    // A directory the gate refuses is not gone into.
    builder.filter_entry(move |entry| {
        if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
            return true;
        }
        let Some(parent) = entry
            .path()
            .parent()
            .and_then(|parent| parent_of(&root_for_filter, &gate_root, parent))
        else {
            return false;
        };
        parent
            .abs
            .child(entry.file_name())
            .is_ok_and(|abs| filter_gate.may_index(&abs))
    });
    builder
}

/// What a walk gathers: on several threads, one of these each.
#[derive(Default)]
struct Gathered {
    files: Vec<Listed>,
    skipped: Skipped,
    /// The directory the last entry was in, so that a run of entries in one
    /// directory has its path checked once.
    parent: Option<Parent>,
}

impl Gathered {
    /// Takes one entry of the walk: a file to index, or the reason it is
    /// not one.
    fn take(
        &mut self,
        entry: Result<ignore::DirEntry, ignore::Error>,
        root_fs: &Path,
        root: &AbsPath,
        gate: &dyn Gate,
    ) {
        let Ok(entry) = entry else {
            self.skipped.unreadable += 1;
            return;
        };
        if entry.depth() == 0 {
            return;
        }
        let Some(kind) = entry.file_type() else {
            return;
        };
        if kind.is_symlink() {
            self.skipped.links += 1;
            return;
        }
        if !kind.is_file() {
            // A directory is listed by the files under it; anything else
            // (a FIFO, a socket, a device) is not a file to read.
            return;
        }
        let Some(fs_parent) = entry.path().parent() else {
            return;
        };
        if self
            .parent
            .as_ref()
            .is_none_or(|known| known.fs_path != fs_parent)
        {
            self.parent = parent_of(root_fs, root, fs_parent);
        }
        let Some(known) = self.parent.as_ref() else {
            self.skipped.refused += 1;
            return;
        };
        let Ok(abs) = known.abs.child(entry.file_name()) else {
            self.skipped.refused += 1;
            return;
        };
        if !gate.may_index(&abs) {
            self.skipped.denied += 1;
            return;
        }
        let Ok(metadata) = entry.metadata() else {
            self.skipped.unreadable += 1;
            return;
        };
        let Some(name) = abs.file_name() else {
            return;
        };
        let relative = if known.relative.is_empty() {
            name.to_owned()
        } else {
            format!("{}/{name}", known.relative)
        };
        self.files.push(Listed {
            relative,
            depth: known.depth,
            stamp: Stamp::of(&metadata),
        });
    }
}

/// A thread's [`Gathered`], handed over when the walk ends with it.
struct HandedOver<'a> {
    gathered: Gathered,
    into: &'a std::sync::Mutex<Vec<Gathered>>,
}

impl Drop for HandedOver<'_> {
    fn drop(&mut self) {
        self.into
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(std::mem::take(&mut self.gathered));
    }
}

/// Lists the workspace at `root`, on `threads` threads.
///
/// The walk is the one `search` and `list` make, on however many threads:
/// its entries come in no set order, and the files are put in
/// [`path_order`] after, so the listing is the same on every machine. A
/// listing that reaches its cap on entries is made again in name order, on
/// one thread, since which entries the threads reach first differs from one
/// run to the next and the cut must not; with `capped` (the last listing
/// met the cap) it is made in name order at once.
///
/// The walk in name order opens each ignore file by its name, so on a file
/// system that does not tell case apart it also honours one named in
/// another case (`.GITIGNORE`), which the other walk, like `search` and
/// `list`, passes over: only a listing at its cap can differ that way.
pub(crate) fn list(
    root: &AbsPath,
    gate: &Arc<dyn Gate>,
    limits: &Limits,
    control: &Control<'_>,
    threads: usize,
    capped: bool,
) -> Listing {
    let root_fs = root.to_fs_path();
    if !capped
        && let Some(listing) =
            list_on_threads(&root_fs, root, gate, limits, control, threads.max(1))
    {
        return listing;
    }
    list_in_order(&root_fs, root, gate, limits, control)
}

/// The walk on one thread, in name order.
fn list_in_order(
    root_fs: &Path,
    root: &AbsPath,
    gate: &Arc<dyn Gate>,
    limits: &Limits,
    control: &Control<'_>,
) -> Listing {
    let mut builder = walker(root_fs, root, gate);
    // In name order, so that a listing cut at its cap is the same one on
    // every machine, whatever order a directory gives its entries.
    builder.sort_by_file_name(std::ffi::OsStr::cmp);
    let mut gathered = Gathered::default();
    let mut partial = None;
    let mut entries = 0usize;
    for entry in builder.build() {
        if let Some(why) = control.stop_listing() {
            partial = Some(why);
            break;
        }
        entries += 1;
        if entries > limits.max_listed {
            partial = Some(Partial::Listing);
            break;
        }
        gathered.take(entry, root_fs, root, &**gate);
    }
    finish(vec![gathered], partial, limits)
}

/// The walk on several threads; `None` when it reached the cap on entries,
/// whose cut the walk in name order makes.
fn list_on_threads(
    root_fs: &Path,
    root: &AbsPath,
    gate: &Arc<dyn Gate>,
    limits: &Limits,
    control: &Control<'_>,
    threads: usize,
) -> Option<Listing> {
    let entries = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let capped = AtomicBool::new(false);
    let handed = std::sync::Mutex::new(Vec::new());
    let (entries, stopped, capped, handed) = (&entries, &stopped, &capped, &handed);
    walker(root_fs, root, gate)
        .threads(threads)
        .build_parallel()
        .run(|| {
            let mut mine = HandedOver {
                gathered: Gathered::default(),
                into: handed,
            };
            Box::new(move |entry| {
                // Each thread looks before each entry it is handed: a turn
                // that begins stops them all at their next one. (The gate
                // is asked about a directory's subdirectories as it is read,
                // before they are handed on.)
                if control.stop_listing().is_some() {
                    stopped.store(true, AtomicOrdering::SeqCst);
                    return WalkState::Quit;
                }
                if entries.fetch_add(1, AtomicOrdering::SeqCst) >= limits.max_listed {
                    capped.store(true, AtomicOrdering::SeqCst);
                    return WalkState::Quit;
                }
                mine.gathered.take(entry, root_fs, root, &**gate);
                WalkState::Continue
            })
        });
    if capped.load(AtomicOrdering::SeqCst) {
        return None;
    }
    let partial = stopped
        .load(AtomicOrdering::SeqCst)
        .then_some(Partial::Stopped);
    let gathered = std::mem::take(
        &mut *handed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    Some(finish(gathered, partial, limits))
}

/// The listing from what the walk gathered: the files in path order, each
/// directory's count, and the cap on files.
fn finish(gathered: Vec<Gathered>, partial: Option<Partial>, limits: &Limits) -> Listing {
    let mut listing = Listing {
        partial,
        ..Listing::default()
    };
    let mut all: Vec<Listed> = Vec::new();
    for part in gathered {
        all.extend(part.files);
        let skipped = &mut listing.skipped;
        skipped.refused += part.skipped.refused;
        skipped.denied += part.skipped.denied;
        skipped.unreadable += part.skipped.unreadable;
        skipped.links += part.skipped.links;
    }

    listing.found = all.len();
    if !all.is_empty() {
        listing.dirs.insert(String::new(), all.len());
    }
    for file in &all {
        let mut at = 0;
        while let Some(slash) = file.relative[at..].find('/') {
            at += slash;
            // A directory's name is made once, not once for each file in
            // it.
            let dir = &file.relative[..at];
            if let Some(count) = listing.dirs.get_mut(dir) {
                *count += 1;
            } else {
                listing.dirs.insert(dir.to_owned(), 1);
            }
            at += 1;
        }
    }
    if all.len() > limits.max_files {
        // The shallowest files, then by path: the top of the tree.
        all.sort_by(|one, other| {
            one.depth
                .cmp(&other.depth)
                .then_with(|| path_order(&one.relative, &other.relative))
        });
        all.truncate(limits.max_files);
        listing.partial.get_or_insert(Partial::Files);
    }
    all.sort_by(|one, other| path_order(&one.relative, &other.relative));
    listing.files = all;
    listing
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cortex_base::{CancelToken, PathOptions, PathResolver};
    use cortex_testkit::Scratch;

    use super::{Control, Gate, Limits, Listing, list_in_order, list_on_threads};

    /// Each file's path, depth and size; how many were found; each
    /// directory's count.
    type Summary = (Vec<(String, usize, u64)>, usize, Vec<(String, usize)>);

    fn summary(listing: &Listing) -> Summary {
        (
            listing
                .files
                .iter()
                .map(|file| (file.relative.clone(), file.depth, file.stamp.size))
                .collect(),
            listing.found,
            listing
                .dirs
                .iter()
                .map(|(dir, count)| (dir.clone(), *count))
                .collect(),
        )
    }

    /// Below the cap the walk in name order and the walk on threads list
    /// the same files, counts and directories, with ignore files at every
    /// level and the gate: the walk in name order stands in for the other
    /// only at the cap, and must cut the same workspace.
    #[test]
    fn the_walk_in_name_order_lists_what_the_walk_on_threads_lists() {
        let scratch = Scratch::outside("cortex-index-unit", "the_walk_in_name_order_lists");
        scratch.file("ws/.gitignore", "*.log\n");
        scratch.file("ws/.ignore", "vendor/\n");
        for top in [2, 0, 3, 1] {
            scratch.file(format!("ws/p{top}/.gitignore"), "generated.rs\n");
            for sub in [1, 0, 2] {
                scratch.file(format!("ws/p{top}/s{sub}/.ignore"), "local.rs\n");
                for name in ["b.rs", "generated.rs", "local.rs", "a.rs", "x.log"] {
                    scratch.file(format!("ws/p{top}/s{sub}/{name}"), "fn f() {}\n");
                }
            }
        }
        scratch.file("ws/vendor/v.rs", "fn v() {}\n");
        scratch.file("ws/secrets/k.rs", "fn k() {}\n");
        scratch.file("ws/.hidden/h.rs", "fn h() {}\n");
        let root = PathResolver::new(&scratch.dir("ws"), PathOptions::new())
            .expect("a workspace")
            .root()
            .clone();
        let gate: Arc<dyn Gate> =
            Arc::new(|path: &cortex_base::AbsPath| !path.as_str().ends_with("/secrets"));
        let cancel = CancelToken::new();
        let control = Control {
            cancel: &cancel,
            give_way: &|| false,
            out_of_time: &|| false,
        };
        let limits = Limits::default();
        let root_fs = root.to_fs_path();
        let in_order = list_in_order(&root_fs, &root, &gate, &limits, &control);
        assert_eq!(
            in_order.files.len(),
            4 * 3 * 2,
            "{:?}",
            summary(&in_order).0
        );
        for threads in [1, 2, 4] {
            let on_threads = list_on_threads(&root_fs, &root, &gate, &limits, &control, threads)
                .expect("under the cap");
            assert_eq!(
                summary(&on_threads),
                summary(&in_order),
                "{threads} threads"
            );
            assert_eq!(on_threads.skipped, in_order.skipped, "{threads} threads");
            assert_eq!(on_threads.partial, in_order.partial, "{threads} threads");
        }
    }
}
