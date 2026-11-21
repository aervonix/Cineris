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

use std::path::PathBuf;
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

/// The places a session's extensions were found in, each with its stamp.
#[derive(Debug, Default)]
pub(crate) struct Watched {
    places: Vec<(PathBuf, Stamp, UtcTime)>,
}

impl Watched {
    /// Stamps `places`, in their order: each folder before what is in it.
    pub(crate) fn stamp(places: Vec<PathBuf>, clock: &SharedClock) -> Self {
        let now = clock.now();
        Self {
            places: places
                .into_iter()
                .map(|place| {
                    let stamp = Stamp::of(&place);
                    (place, stamp, now)
                })
                .collect(),
        }
    }

    /// The first place that changed since it was stamped, looked at in
    /// order; `None` when none did. Nothing after a changed place is looked
    /// at.
    pub(crate) fn changed(&self) -> Option<PathBuf> {
        self.places.iter().find_map(|(place, stamp, taken)| {
            let now = Stamp::of(place);
            (now != *stamp || unsettled(stamp, *taken)).then(|| place.clone())
        })
    }
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
}
