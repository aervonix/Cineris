//! The user's own pins (since step 28): the hash of each thing of the
//! user's own that runs code and that the user approved, in `pins.toml` in
//! the state directory (`docs/architecture/06-permissions-and-sandbox.md`,
//! "Project trust", "As built in step 28").
//!
//! A project's definitions are pinned with its trust, in `trust.toml`. The
//! user's own extensions, in the configuration directory's `extensions/`,
//! belong to no project, and are pinned here: the user approves one, its
//! hash is pinned, and it is used only while it is still what was approved.
//! Step 30 keeps the user's MCP servers' pins of each tool here too, so a
//! pin has a kind, as the pins of `trust.toml` have.
//!
//! # The stored file
//!
//! ```toml
//! version = 1
//!
//! [[pin]]
//! kind = "extension"
//! name = "tracker"
//! hash = "5e884898da28…"
//! at = "2026-11-20T10:15:02Z"
//! ```
//!
//! - **A file of its own, not a version 3 of `trust.toml`.** A build that
//!   reads `trust.toml` up to version 2 refuses a version 3 whole, and so
//!   would trust no project at all for the sake of pins it has no use for.
//!   A build from before step 28 does not read `pins.toml`.
//! - **One pin for each kind and name.** Pinning a definition again
//!   replaces its pin, in its place; `at` is when it was last pinned.
//! - **Written owner-only, atomically, on every write,** under a lock held
//!   from the read to the write (`pins.toml.lock`, `crate::store_lock`), as
//!   `trust.toml` is: two sessions that pin at once both keep their pin.
//! - **Read strictly.** A key this build does not know, a value of the
//!   wrong type, a `version` other than 1, a hash that is not one, or two
//!   pins of one kind and name refuse the whole file, and then **nothing is
//!   taken as pinned**: a file that cannot be read in full is not read in
//!   part (decision 0028's rule,
//!   `docs/decisions/0028-enforced-values-fail-closed.md`). The error says
//!   why, and the file is left as it is: a change through the store fails
//!   rather than write over it.
//! - **A kind this build does not know** is kept as it was written, and
//!   written back: a later build's pin, which approves nothing this build
//!   asks about.
//! - **A missing file pins nothing,** and the first pin makes it, with its
//!   folder, owner-only.
//! - **A link at its name is not followed,** as at the registration
//!   cache's and an extension's values' (since the review of step 28): the
//!   store is not read through it, so it pins nothing and says why, and no
//!   change writes through it, so what it names is left as it is.
//! - **No tool can reach it.** It is in the state directory, which the
//!   built-in protection closes to every tool, to reading and to writing,
//!   at every level.

#![allow(
    clippy::disallowed_methods,
    reason = "blocking file calls; the store is used at start-up and on the blocking pool"
)]

use std::io;
use std::path::{Path, PathBuf};

use cineris_base::{
    AtomicWriteError, AtomicWriteOptions, ContentHash, OpenError, SharedClock, write_atomic,
};
use cineris_protocol::ids::Timestamp;
use cineris_protocol::methods::TrustResourceKind;
use serde::{Deserialize, Serialize};

use super::{Definition, Standing};
use crate::store_lock;

/// The version of the file's format: the only one this build reads, and
/// the one it writes.
const VERSION: u32 = 1;

/// The largest file read.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// A point of the store's (`cineris_base::hook`): a change has read the
/// file and changed it, under the lock, and is about to write it. A test
/// holds it to bring a second writer in between a read and its write.
pub const BEFORE_THE_PINS_ARE_WRITTEN: &str =
    "pins: the file is read and changed, and is about to be written";

/// One thing of the user's own that the user approved: what kind of thing,
/// its name, the hash of what was approved, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserPin {
    /// What kind of thing it is: `extension`.
    pub kind: TrustResourceKind,
    /// Its name, unique within its kind.
    pub name: String,
    /// The hash of what the user approved ([`Definition::hash`]).
    pub hash: ContentHash,
    /// When it was pinned.
    pub at: Timestamp,
}

impl UserPin {
    /// The definition the user approved.
    #[must_use]
    pub fn definition(&self) -> Definition {
        Definition {
            kind: self.kind.clone(),
            name: self.name.clone(),
            hash: self.hash,
        }
    }

    /// Whether this is the pin of the thing of this kind and name. Kinds
    /// are compared as they are written, so that a known kind held as an
    /// unknown one is still the same kind.
    fn is_of(&self, kind: &TrustResourceKind, name: &str) -> bool {
        self.kind.as_str() == kind.as_str() && self.name == name
    }
}

/// What the user has pinned, read once: a value to ask about each
/// definition as it is met, as [`ProjectTrust`](super::ProjectTrust) is of a
/// project's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserPins {
    pins: Vec<UserPin>,
}

impl UserPins {
    /// The pin of the thing of this kind and name, if there is one.
    #[must_use]
    pub fn get(&self, kind: &TrustResourceKind, name: &str) -> Option<&UserPin> {
        self.pins.iter().find(|pin| pin.is_of(kind, name))
    }

    /// How a definition of the user's own stands: `Approved` when its hash
    /// is the pinned one, `Changed` when a pin of its kind and name holds
    /// another hash, and `New` when nothing of its kind and name is pinned.
    /// Never `Untrusted`: the user's own things belong to no project.
    #[must_use]
    pub fn standing(&self, definition: &Definition) -> Standing {
        match self.get(&definition.kind, &definition.name) {
            Some(pin) if pin.hash == definition.hash => Standing::Approved,
            Some(_) => Standing::Changed,
            None => Standing::New,
        }
    }

    /// Every pin, in the order each was first pinned.
    #[must_use]
    pub fn list(&self) -> &[UserPin] {
        &self.pins
    }

    /// Whether nothing is pinned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
    }
}

/// Why the user's pins could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum PinError {
    /// The file could not be read.
    #[error("the pin store `{path}` could not be read, so nothing is taken as pinned: {source}")]
    Read {
        /// The file.
        path: String,
        /// Why.
        source: io::Error,
    },
    /// The file is not one this build understands in full. Nothing in it
    /// is taken as pinned, and it is left as it is.
    #[error(
        "the pin store `{path}` is not understood, so nothing is taken as pinned: {problem}. The \
         file was left as it is"
    )]
    NotUnderstood {
        /// The file.
        path: String,
        /// What is wrong with it.
        problem: String,
    },
    /// The file could not be written.
    #[error(transparent)]
    Write(#[from] AtomicWriteError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    #[serde(default, rename = "pin", skip_serializing_if = "Vec::is_empty")]
    pins: Vec<UserPin>,
}

/// The user's pin store: the hashes of what the user approved of their own.
#[derive(Debug, Clone)]
pub struct PinStore {
    file: PathBuf,
    clock: SharedClock,
}

impl PinStore {
    /// The store at `file`: `StateDir::pins_file` of `cineris-config`.
    #[must_use]
    pub fn new(file: impl Into<PathBuf>, clock: SharedClock) -> Self {
        Self {
            file: file.into(),
            clock,
        }
    }

    /// The file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.file
    }

    /// What the user has pinned. A missing file pins nothing, and reading
    /// it creates nothing.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood in full. The
    /// caller takes nothing as pinned, and shows the error.
    pub fn pins(&self) -> Result<UserPins, PinError> {
        Ok(UserPins {
            pins: self.read()?.pins,
        })
    }

    /// Pins a definition as it is now: adds its pin, or replaces the pin of
    /// the same kind and name, in its place. Gives what is pinned after.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood (it is then not
    /// overwritten), or cannot be written.
    pub fn pin(&self, definition: &Definition) -> Result<UserPins, PinError> {
        self.update(|file| {
            let pin = UserPin {
                kind: definition.kind.clone(),
                name: definition.name.clone(),
                hash: definition.hash,
                at: self.clock.now().to_timestamp(),
            };
            match file
                .pins
                .iter()
                .position(|old| old.is_of(&pin.kind, &pin.name))
            {
                Some(index) => file.pins[index] = pin,
                None => file.pins.push(pin),
            }
            (
                UserPins {
                    pins: file.pins.clone(),
                },
                true,
            )
        })
    }

    /// Takes away the pin of the thing of this kind and name. Says whether
    /// there was one; when there was not, nothing is written.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood, or cannot be
    /// written.
    pub fn unpin(&self, kind: &TrustResourceKind, name: &str) -> Result<bool, PinError> {
        self.update(|file| {
            let before = file.pins.len();
            file.pins.retain(|pin| !pin.is_of(kind, name));
            let removed = file.pins.len() != before;
            (removed, removed)
        })
    }

    /// Reads the file, changes it and writes it if `change` says it changed
    /// (the second of what it returns), with the lock held throughout. Every
    /// change to the file goes through here.
    fn update<T>(&self, change: impl FnOnce(&mut File) -> (T, bool)) -> Result<T, PinError> {
        let _lock = store_lock::hold(&self.file).map_err(|source| AtomicWriteError::Io {
            path: self.file.display().to_string(),
            step: "lock",
            source,
        })?;
        let mut file = self.read()?;
        let (answer, changed) = change(&mut file);
        if changed {
            cineris_base::hook::point(BEFORE_THE_PINS_ARE_WRITTEN);
            self.write(&file)?;
        }
        Ok(answer)
    }

    fn not_understood(&self, problem: String) -> PinError {
        PinError::NotUnderstood {
            path: self.file.display().to_string(),
            problem,
        }
    }

    fn read(&self) -> Result<File, PinError> {
        let read_error = |source| PinError::Read {
            path: self.file.display().to_string(),
            source,
        };
        // A link at the store's name is not followed: what it names is not
        // the user's pins, and it is refused, as a folder there is.
        let (opened, found) = match cineris_base::open_regular(&self.file) {
            Ok(opened) => opened,
            Err(OpenError::NotFound) => {
                return Ok(File {
                    version: VERSION,
                    pins: Vec::new(),
                });
            }
            Err(OpenError::Io(error)) => return Err(read_error(error)),
            Err(other) => return Err(read_error(io::Error::other(other.to_string()))),
        };
        let size = found.len();
        if size > MAX_BYTES {
            return Err(self.not_understood(format!(
                "it is {size} bytes, more than a pin store could be"
            )));
        }
        let mut text = String::new();
        io::Read::read_to_string(&mut io::Read::take(opened, MAX_BYTES), &mut text)
            .map_err(read_error)?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let file: File = toml::from_str(text)
            .map_err(|error| self.not_understood(error.message().to_owned()))?;
        if file.version != VERSION {
            return Err(self.not_understood(format!(
                "its version is {}, and this version of Cineris reads version {VERSION}",
                file.version
            )));
        }
        // A file this build wrote has one pin for each kind and name: one
        // with two would leave it to the order which of them stands.
        for (index, pin) in file.pins.iter().enumerate() {
            if file.pins[..index]
                .iter()
                .any(|earlier| earlier.is_of(&pin.kind, &pin.name))
            {
                return Err(self.not_understood(format!(
                    "it pins the {} `{}` twice",
                    pin.kind,
                    pin.name.escape_debug()
                )));
            }
        }
        Ok(file)
    }

    fn write(&self, file: &File) -> Result<(), PinError> {
        // The directory was made when the lock was taken.
        let text = toml::to_string(file).map_err(|error| {
            self.not_understood(format!("it could not be written out: {error}"))
        })?;
        // A link at the store's name is not written through: the write is
        // refused, and what the link names is left as it is.
        let options = AtomicWriteOptions {
            owner_only: true,
            follow_links: false,
            ..AtomicWriteOptions::default()
        };
        write_atomic(&self.file, text.as_bytes(), &options, &*self.clock)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cineris_base::FakeClock;

    use super::*;
    use crate::store_lock::tests::{Scratch, is_held};

    #[test]
    fn every_change_to_the_pins_is_made_under_their_lock() {
        let scratch = Scratch::new();
        let file = scratch.0.join("pins.toml");
        let clock: SharedClock = Arc::new(FakeClock::starting_at("2026-11-20T10:00:00Z").unwrap());
        let store = PinStore::new(&file, clock);
        // Every public change goes through `update`, and `update` holds the
        // lock from the read to the write.
        let held = store.update(|_| (is_held(&file), false)).unwrap();
        assert!(held, "the lock is held while the file is changed");
        assert!(!is_held(&file), "and let go after");
        let tracker = Definition::of(TrustResourceKind::Extension, "tracker", ["v1"]);
        store.pin(&tracker).unwrap();
        assert!(!is_held(&file));
        assert!(
            store
                .unpin(&TrustResourceKind::Extension, "tracker")
                .unwrap()
        );
        assert!(!is_held(&file));
        assert!(store.pins().unwrap().is_empty());
    }

    /// A link put at the store's name after it was read, before it is
    /// written, is not written through: the change fails, and what the link
    /// names is left as it is.
    #[test]
    fn a_link_put_at_the_stores_name_after_its_read_is_not_written_through() {
        let scratch = Scratch::new();
        std::fs::create_dir_all(&scratch.0).unwrap();
        let file = scratch.0.join("pins.toml");
        let named = scratch.0.join("elsewhere.toml");
        std::fs::write(&named, "the user's own file").unwrap();
        let clock: SharedClock = Arc::new(FakeClock::starting_at("2026-11-21T10:00:00Z").unwrap());
        let store = PinStore::new(&file, clock);
        let mut put = false;
        let written = store.update(|pins| {
            #[cfg(unix)]
            let made = std::os::unix::fs::symlink(&named, &file);
            #[cfg(windows)]
            let made = std::os::windows::fs::symlink_file(&named, &file);
            match made {
                Ok(()) => put = true,
                Err(error) if std::env::var_os("GITHUB_ACTIONS").is_some() => {
                    panic!("the CI runner cannot make a link: {error}")
                }
                Err(error) => println!("skipped: cannot make a link here ({error})"),
            }
            pins.pins.clear();
            ((), true)
        });
        if !put {
            return;
        }
        assert!(written.is_err(), "{written:?}");
        assert_eq!(
            std::fs::read_to_string(&named).unwrap(),
            "the user's own file"
        );
    }
}
