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
use std::time::Duration;

use cortex_base::{AbsPath, CancelToken};
use ignore::WalkBuilder;

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
#[must_use]
pub fn path_order(one: &str, other: &str) -> Ordering {
    one.split('/').cmp(other.split('/'))
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

/// Lists the workspace at `root`.
pub(crate) fn list(
    root: &AbsPath,
    gate: &Arc<dyn Gate>,
    limits: &Limits,
    control: &Control<'_>,
) -> Listing {
    let root_fs = root.to_fs_path();
    let mut builder = WalkBuilder::new(&root_fs);
    builder
        .follow_links(false)
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false)
        .git_global(false)
        // In name order, so that a listing cut at its cap is the same one
        // on every machine, whatever order a directory gives its entries.
        .sort_by_file_name(std::ffi::OsStr::cmp);
    let gate_root = root.clone();
    let root_for_filter = root_fs.clone();
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

    let mut listing = Listing::default();
    let mut all: Vec<Listed> = Vec::new();
    let mut parent: Option<Parent> = None;
    let mut entries = 0usize;
    for entry in builder.build() {
        if let Some(why) = control.stop_listing() {
            listing.partial = Some(why);
            break;
        }
        entries += 1;
        if entries > limits.max_listed {
            listing.partial = Some(Partial::Listing);
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                listing.skipped.unreadable += 1;
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        let Some(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            listing.skipped.links += 1;
            continue;
        }
        if !kind.is_file() {
            // A directory is listed by the files under it; anything else
            // (a FIFO, a socket, a device) is not a file to read.
            continue;
        }
        let Some(fs_parent) = entry.path().parent() else {
            continue;
        };
        if parent
            .as_ref()
            .is_none_or(|known| known.fs_path != fs_parent)
        {
            parent = parent_of(&root_fs, root, fs_parent);
        }
        let Some(known) = parent.as_ref() else {
            listing.skipped.refused += 1;
            continue;
        };
        let Ok(abs) = known.abs.child(entry.file_name()) else {
            listing.skipped.refused += 1;
            continue;
        };
        if !gate.may_index(&abs) {
            listing.skipped.denied += 1;
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            listing.skipped.unreadable += 1;
            continue;
        };
        let Some(name) = abs.file_name() else {
            continue;
        };
        let relative = if known.relative.is_empty() {
            name.to_owned()
        } else {
            format!("{}/{name}", known.relative)
        };
        all.push(Listed {
            relative,
            depth: known.depth,
            stamp: Stamp::of(&metadata),
        });
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
