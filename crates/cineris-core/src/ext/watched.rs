//! What a session's extensions were found in, stamped, so that a change is
//! noticed at the next turn's start (`docs/architecture/12-extensions.md`,
//! "Hot-reload").
//!
//! The stamps are of the places discovery read, where they really are:
//! each root, the names on the way to a project's root, each extension's
//! folder, each folder within it and each of its files. **No stamp follows
//! a link**: a repository may put one where a folder was (on Windows, one
//! to a network share, whose server would be sent the account's
//! credentials by a look through it), so a place is looked at itself, and
//! a place that has become a link is a change, which discovery, which
//! judges links, then reads again. The places are looked at in the order
//! they were found, a folder before what is in it, and the look stops at
//! the first change: nothing is looked at through a folder that has
//! changed.
//!
//! As `cineris_base::watch` does, a place modified within two seconds of
//! its stamp is taken as changed until that has passed: a second write in
//! the same step of the file system's clock would leave its time and size
//! as they were.
//!
//! These functions block: the caller runs them on the blocking pool.

#![allow(
    clippy::disallowed_methods,
    reason = "a look at each place found, on the blocking pool"
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use cineris_base::{SharedClock, UtcTime};

/// How close to its stamp a place's modification may be for a later
/// change to go unseen.
const UNSETTLED: Duration = Duration::from_secs(2);

/// What one place looked like, looked at itself and never through a link.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stamp {
    /// Nothing is there.
    Missing,
    /// Something is there that cannot be looked at.
    Unreadable(std::io::ErrorKind),
    /// Something of this kind, whatever its time and size: a name on the
    /// way to a place, whose kind alone matters (a file another tool writes
    /// beside it is no change of it).
    Kind(Kind),
    /// A folder, a file, a link or anything else.
    Present {
        modified: Option<SystemTime>,
        len: u64,
        kind: Kind,
        /// Which file this is, and when its metadata changed (Unix).
        identity: Option<(u64, i64, i64)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Folder,
    File,
    Link,
    Other,
}

impl Stamp {
    fn of(path: &std::path::Path) -> Self {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                let kind = metadata.file_type();
                Self::Present {
                    modified: metadata.modified().ok(),
                    len: metadata.len(),
                    kind: if kind.is_symlink() {
                        Kind::Link
                    } else if kind.is_dir() {
                        Kind::Folder
                    } else if kind.is_file() {
                        Kind::File
                    } else {
                        Kind::Other
                    },
                    identity: identity(&metadata),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::Missing,
            Err(error) => Self::Unreadable(error.kind()),
        }
    }

    /// What `path` is, looked at itself: its kind alone.
    fn kind_of(path: &std::path::Path) -> Self {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                let kind = metadata.file_type();
                Self::Kind(if kind.is_symlink() {
                    Kind::Link
                } else if kind.is_dir() {
                    Kind::Folder
                } else if kind.is_file() {
                    Kind::File
                } else {
                    Kind::Other
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::Missing,
            Err(error) => Self::Unreadable(error.kind()),
        }
    }

    fn modified(&self) -> Option<SystemTime> {
        match self {
            Self::Present { modified, .. } => *modified,
            _ => None,
        }
    }
}

#[cfg(unix)]
fn identity(metadata: &std::fs::Metadata) -> Option<(u64, i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.ino(), metadata.ctime(), metadata.ctime_nsec()))
}

#[cfg(not(unix))]
fn identity(_: &std::fs::Metadata) -> Option<(u64, i64, i64)> {
    None
}

/// The places a session's extensions were found in, each with its stamp,
/// in the order they were stamped, and the set of them, so that a place is
/// stamped once at no cost of a look through the others (the fresh review
/// of step 28: a look through the list for each place made a crafted
/// folder of 65,000 places take a minute).
#[derive(Debug, Default)]
pub(crate) struct Watched {
    places: Vec<(PathBuf, Stamp, UtcTime)>,
    known: HashSet<PathBuf>,
}

/// The most places stamped within the extensions of one root, their
/// folders, the folders in them and their files together. Past it the rest
/// of the root is not read: what cannot be watched for a change is not
/// loaded, and the root counts as not listed whole, so that its names are
/// taken (the fresh review of step 28). A user's few extensions of a few
/// files each are far below it.
pub(crate) const MAX_PLACES_PER_ROOT: usize = 4096;

/// The most places stamped within one extension's folder: its snapshot's
/// own cap on entries, past which it is not read either.
const MAX_IN_A_FOLDER: usize = 1024;

/// The deepest a folder within an extension's is stamped, as its snapshot
/// reads it.
const MAX_DEPTH: usize = 16;

impl Watched {
    /// Stamps `places`, in their order: each folder before what is in it.
    #[cfg(test)]
    pub(crate) fn stamp(places: Vec<PathBuf>, clock: &SharedClock) -> Self {
        let mut watched = Self::default();
        for place in places {
            watched.note(place, clock);
        }
        watched
    }

    /// Stamps `place` now, after what is stamped already; a place stamped
    /// already keeps its first stamp.
    pub(crate) fn note(&mut self, place: PathBuf, clock: &SharedClock) {
        if !self.known.insert(place.clone()) {
            return;
        }
        let stamp = Stamp::of(&place);
        self.places.push((place, stamp, clock.now()));
    }

    /// Stamps `place` now by its kind alone: a name on the way to a place,
    /// which changes when it comes, goes or becomes a link, and not when
    /// something is made in it. A place stamped already keeps its first
    /// stamp.
    pub(crate) fn note_kind(&mut self, place: PathBuf, clock: &SharedClock) {
        if !self.known.insert(place.clone()) {
            return;
        }
        let stamp = Stamp::kind_of(&place);
        self.places.push((place, stamp, clock.now()));
    }

    /// How many places are stamped.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.places.len()
    }

    /// Stamps an extension's `folder` now, before it is read: the folder,
    /// then each folder in it, an empty one included, before what is in
    /// that, and each file, walked with no link followed, within the caps
    /// its snapshot is read within. A link in it is stamped as a link and
    /// not walked into. Each place stamped takes one of `left`, what its
    /// root may still stamp ([`MAX_PLACES_PER_ROOT`]): `false` when they ran out
    /// before the folder was stamped whole, and it is then not to be read.
    pub(crate) fn note_tree(
        &mut self,
        folder: &Path,
        clock: &SharedClock,
        left: &mut usize,
    ) -> bool {
        let mut count = 0;
        self.walk(folder, 0, &mut count, clock, left)
    }

    fn walk(
        &mut self,
        folder: &Path,
        depth: usize,
        count: &mut usize,
        clock: &SharedClock,
        left: &mut usize,
    ) -> bool {
        let Some(less) = left.checked_sub(1) else {
            return false;
        };
        *left = less;
        self.note(folder.to_path_buf(), clock);
        if depth > MAX_DEPTH {
            return true;
        }
        let Ok(entries) = std::fs::read_dir(folder) else {
            return true;
        };
        let mut inside: Vec<(PathBuf, bool)> = Vec::new();
        for entry in entries.flatten() {
            if *count >= MAX_IN_A_FOLDER {
                break;
            }
            *count += 1;
            // The entry's own kind: a link is not followed to say it.
            let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
            inside.push((entry.path(), folder));
        }
        inside.sort();
        for (path, is_folder) in inside {
            if is_folder {
                if !self.walk(&path, depth + 1, count, clock, left) {
                    return false;
                }
            } else {
                let Some(less) = left.checked_sub(1) else {
                    return false;
                };
                *left = less;
                self.note(path, clock);
            }
        }
        true
    }

    /// What is stamped in `other`, after what is stamped here.
    pub(crate) fn extend(&mut self, other: Self) {
        for (place, stamp, taken) in other.places {
            if self.known.insert(place.clone()) {
                self.places.push((place, stamp, taken));
            }
        }
    }

    /// The first place that changed since it was stamped, looked at in
    /// order; `None` when none did. Nothing after a changed place is looked
    /// at.
    pub(crate) fn changed(&self) -> Option<PathBuf> {
        self.places.iter().find_map(|(place, stamp, taken)| {
            let now = match stamp {
                Stamp::Kind(_) => Stamp::kind_of(place),
                _ => Stamp::of(place),
            };
            (now != *stamp || unsettled(stamp, *taken)).then(|| place.clone())
        })
    }
}

/// The first name on the way to `folder` (it included) that is not a
/// folder of its own, looked at itself: a link, a file, or nothing there.
/// `folder` is where an extension really is, every link on the way already
/// followed when it was found, so a link on it now is one put there since.
/// Blocks.
pub(crate) fn linked_on_the_way(folder: &Path) -> Option<PathBuf> {
    let mut at = PathBuf::new();
    for component in folder.components() {
        at.push(component);
        // The root, or a Windows prefix, is not a name to look at.
        if !matches!(component, std::path::Component::Normal(_)) {
            continue;
        }
        match std::fs::symlink_metadata(&at) {
            Ok(metadata) if metadata.is_dir() => {}
            _ => return Some(at),
        }
    }
    None
}

/// Whether a place was modified so shortly before (or, by a skewed clock,
/// after) its stamp that a second write could hide behind its time.
fn unsettled(stamp: &Stamp, taken: UtcTime) -> bool {
    let Some(modified) = stamp.modified().and_then(UtcTime::from_system) else {
        return false;
    };
    taken
        .duration_since(modified)
        .or_else(|| modified.duration_since(taken))
        .is_some_and(|apart| apart < UNSETTLED)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cineris_base::FakeClock;

    use super::*;

    #[test]
    fn a_change_is_seen_and_nothing_is_looked_at_through_a_link() {
        let dir = std::env::temp_dir().join(format!("cineris-core-watched-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ext")).unwrap();
        std::fs::write(dir.join("ext/main.js"), "one").unwrap();
        // A clock far from the files' times: nothing is unsettled.
        let clock: SharedClock = Arc::new(FakeClock::starting_at("2001-01-01T00:00:00Z").unwrap());
        let places = vec![dir.join("ext"), dir.join("ext/main.js"), dir.join("gone")];
        let watched = Watched::stamp(places, &clock);
        assert_eq!(watched.changed(), None);
        std::fs::write(dir.join("ext/main.js"), "a longer one").unwrap();
        assert_eq!(watched.changed(), Some(dir.join("ext/main.js")));
        let watched = Watched::stamp(vec![dir.join("ext"), dir.join("gone")], &clock);
        std::fs::create_dir(dir.join("gone")).unwrap();
        assert_eq!(watched.changed(), Some(dir.join("gone")));
        // A folder swapped for a link is a change of the folder itself.
        let watched = Watched::stamp(vec![dir.join("ext"), dir.join("ext/main.js")], &clock);
        std::fs::rename(dir.join("ext"), dir.join("was")).unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(dir.join("was"), dir.join("ext"));
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(dir.join("was"), dir.join("ext"));
        // A machine that cannot make a link checks the rest; a CI runner can.
        if linked.is_ok() || std::env::var_os("GITHUB_ACTIONS").is_some() {
            linked.unwrap();
            assert_eq!(watched.changed(), Some(dir.join("ext")));
            // The link itself is looked at, never what it leads to.
            assert!(
                matches!(
                    Stamp::of(&dir.join("ext")),
                    Stamp::Present {
                        kind: Kind::Link,
                        ..
                    }
                ),
                "{:?}",
                Stamp::of(&dir.join("ext"))
            );
        }
        // A place modified just before its stamp stays changed for a time.
        let now: SharedClock = Arc::new(cineris_base::SystemClock::new());
        let watched = Watched::stamp(vec![dir.join("gone")], &now);
        assert_eq!(watched.changed(), Some(dir.join("gone")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A name on the way, stamped by its kind alone, changes when it comes,
    /// goes or becomes something else, and not when something is made in
    /// it: `~/.claude`, which another tool writes in, is not a change each
    /// time it does.
    #[test]
    fn a_name_stamped_by_its_kind_changes_only_with_its_kind() {
        let dir = std::env::temp_dir().join(format!("cineris-core-kind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("way")).unwrap();
        let clock: SharedClock = Arc::new(FakeClock::starting_at("2001-01-01T00:00:00Z").unwrap());
        let mut watched = Watched::default();
        watched.note_kind(dir.join("way"), &clock);
        watched.note_kind(dir.join("later"), &clock);
        std::fs::write(dir.join("way/history"), "written beside").unwrap();
        std::fs::create_dir(dir.join("way/projects")).unwrap();
        assert_eq!(watched.changed(), None, "made in it: no change of it");
        std::fs::create_dir(dir.join("later")).unwrap();
        assert_eq!(watched.changed(), Some(dir.join("later")), "come");
        let mut watched = Watched::default();
        watched.note_kind(dir.join("way"), &clock);
        std::fs::remove_dir_all(dir.join("way")).unwrap();
        std::fs::write(dir.join("way"), "a file now").unwrap();
        assert_eq!(watched.changed(), Some(dir.join("way")), "another kind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a root may stamp is bounded by count: a folder walked past it is
    /// reported, and nothing past the bound is stamped; a place stamped
    /// twice is stamped once.
    #[test]
    fn the_places_a_root_stamps_are_bounded_by_count() {
        let dir = std::env::temp_dir().join(format!("cineris-core-bounded-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for name in ["a", "b", "c", "d", "e"] {
            std::fs::create_dir_all(dir.join("ext").join(name)).unwrap();
        }
        let clock: SharedClock = Arc::new(FakeClock::starting_at("2001-01-01T00:00:00Z").unwrap());
        // The folder and its five: six places.
        let mut watched = Watched::default();
        let mut left = 6;
        assert!(watched.note_tree(&dir.join("ext"), &clock, &mut left));
        assert_eq!((watched.len(), left), (6, 0));
        let mut watched = Watched::default();
        let mut left = 4;
        assert!(!watched.note_tree(&dir.join("ext"), &clock, &mut left));
        assert_eq!((watched.len(), left), (4, 0));
        // Nothing is stamped twice, by a walk or by a merge.
        let mut again = Watched::default();
        let mut plenty = MAX_PLACES_PER_ROOT;
        assert!(again.note_tree(&dir.join("ext"), &clock, &mut plenty));
        again.note(dir.join("ext/a"), &clock);
        let mut other = Watched::default();
        other.note(dir.join("ext/b"), &clock);
        other.note(dir.join("elsewhere"), &clock);
        again.extend(other);
        assert_eq!(again.len(), 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file replaced by another of the same size and time is still a
    /// change, by which file it is (Unix's inode and change time).
    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_types,
        reason = "a file's modification time is set back by its handle"
    )]
    fn a_file_replaced_with_the_same_size_and_time_is_a_change() {
        let dir =
            std::env::temp_dir().join(format!("cineris-core-replaced-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.js");
        std::fs::write(&file, "one").unwrap();
        let time = std::fs::metadata(&file).unwrap().modified().unwrap();
        let clock: SharedClock = Arc::new(FakeClock::starting_at("2001-01-01T00:00:00Z").unwrap());
        let watched = Watched::stamp(vec![file.clone()], &clock);
        let other = dir.join("other.js");
        std::fs::write(&other, "two").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&other)
            .unwrap()
            .set_modified(time)
            .unwrap();
        std::fs::rename(&other, &file).unwrap();
        assert_eq!(watched.changed(), Some(file));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
