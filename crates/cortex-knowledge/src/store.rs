//! The private store on disk: `<state dir>/projects/<project
//! key>/knowledge.toml` (`docs/architecture/14-adaptation.md`, "Where it
//! lives").
//!
//! What was learned about a project lasts across sessions in this one
//! file, so the rules here are about not losing it.
//!
//! - **Every change is an operation on the file as it is read under the
//!   lock** ([`Store::change`]): take the lock, read, apply, write. No
//!   session ever writes its own copy of the file, so two sessions of one
//!   project that change it at the same moment both keep their change, and
//!   an id is given under the lock.
//! - **It is written owner-only and atomically**, flushed to the disk
//!   before it takes its name, in a directory made owner-only.
//! - **A file that cannot be read as it is, is never written over, moved
//!   or deleted.** A person's edit went wrong: the file is theirs to put
//!   right, and a change is refused until they have
//!   ([`StoreError::Invalid`]).
//! - **The old state is kept beside it,** in `knowledge.toml.good`: the
//!   bytes of the harness's last successful write. A load that finds the
//!   store invalid uses the good copy, and says so ([`Loaded::problem`]).
//!   With neither, nothing is used.
//! - **Readers take no lock.** The file is replaced in one step, so a
//!   reader sees the old one or the new one, whole. Bringing the good copy
//!   up to date after a read is tried, and left for the next read when
//!   another process holds the lock: a load never waits.
//! - **Nothing is written that could not be read back.** A change that
//!   would take the file past its size cap is refused
//!   ([`StoreError::Full`]), and so is one whose text this build would not
//!   read again: a store written past what the reader takes would be
//!   refused whole at the next load, its good copy with it.
//!
//! The lock is the operating system's, on `knowledge.toml.lock`
//! (`cortex_base::private::hold_lock`). Everything here blocks.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "blocking file calls; the store is read and changed on the blocking pool"
)]

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use cortex_base::{
    AtomicWriteError, AtomicWriteOptions, FileStamp, OpenError, SharedClock, UtcTime, private,
    write_atomic,
};

use crate::file::{MAX_FILE_BYTES, PrivateState, Problem, parse_private, write_private};

/// The private store as a person names it in a message.
pub const STORE_NAME: &str = "knowledge.toml";

/// Where a load's state came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The store itself.
    File,
    /// The good copy: the store could not be read as it is.
    GoodCopy,
    /// Nothing: there is no store yet, or neither file can be read.
    Nothing,
}

/// The private store, loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// What is used.
    pub state: PrivateState,
    /// Where it came from.
    pub source: Source,
    /// What is wrong with the store, when it could not be read as it is.
    pub problem: Option<Problem>,
    /// What the store looked like when it was read.
    pub stamp: FileStamp,
}

/// Why the store could not be changed. In every case the file is as it
/// was.
#[derive(Debug, thiserror::Error)]
pub enum StoreError<E> {
    /// The store cannot be read as it is, so it is not written over.
    #[error("{0}. Nothing in it was changed: put the file right, or remove it")]
    Invalid(Problem),
    /// The lock, the read or the write failed.
    #[error("the project's store `{path}` could not be changed: {source}")]
    Io {
        /// The file.
        path: String,
        /// Why.
        source: io::Error,
    },
    /// The write failed.
    #[error(transparent)]
    Write(#[from] AtomicWriteError),
    /// The change would take the store past what it may hold. Nothing was
    /// written.
    #[error("{0}")]
    Full(String),
    /// The change would leave a store this build could not read back.
    /// Nothing was written.
    #[error(
        "the change would leave a store that could not be read again ({0}), so it was not made"
    )]
    Unreadable(Problem),
    /// The change itself was refused, and nothing was written.
    #[error("{0}")]
    Refused(E),
}

/// One project's private store.
#[derive(Debug, Clone)]
pub struct Store {
    file: PathBuf,
    clock: SharedClock,
}

/// What reading one file gave.
enum Read1 {
    Absent,
    Text(String),
    Problem(String),
}

/// Reads a file of at most [`MAX_FILE_BYTES`], following no link and
/// waiting on nothing that is not a regular file.
pub(crate) fn read_bounded(path: &Path) -> io::Result<Option<Result<String, String>>> {
    let (file, metadata) = match cortex_base::open_regular(path) {
        Ok(opened) => opened,
        Err(OpenError::NotFound) => return Ok(None),
        Err(OpenError::Io(error)) => return Err(error),
        Err(other) => return Ok(Some(Err(other.to_string()))),
    };
    if metadata.len() > MAX_FILE_BYTES {
        return Ok(Some(Err(format!(
            "it is {} bytes, and may be up to {MAX_FILE_BYTES}",
            metadata.len()
        ))));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    // The size was looked at a moment ago; a file that grew since is still
    // held to the cap.
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Ok(Some(Err(format!("it is more than {MAX_FILE_BYTES} bytes"))));
    }
    Ok(Some(
        String::from_utf8(bytes).map_err(|_| "it is not UTF-8 text".to_owned()),
    ))
}

fn read_one(path: &Path) -> Read1 {
    match read_bounded(path) {
        Ok(None) => Read1::Absent,
        Ok(Some(Ok(text))) => Read1::Text(text),
        Ok(Some(Err(problem))) => Read1::Problem(problem),
        Err(error) => Read1::Problem(error.to_string()),
    }
}

impl Store {
    /// The store at `file`: `StateDir::knowledge_file` of `cortex-config`
    /// for the project's key. Touches nothing.
    #[must_use]
    pub fn new(file: impl Into<PathBuf>, clock: SharedClock) -> Self {
        Self {
            file: file.into(),
            clock,
        }
    }

    /// The store's file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.file
    }

    /// The good copy: the store's name with `.good` added.
    #[must_use]
    pub fn good_copy(&self) -> PathBuf {
        let mut name = self.file.as_os_str().to_owned();
        name.push(".good");
        PathBuf::from(name)
    }

    /// The time now, by the store's clock.
    #[must_use]
    pub fn now(&self) -> UtcTime {
        self.clock.now()
    }

    /// What the store looks like now: one `stat`. Another stamp than a
    /// load's means the file changed since.
    #[must_use]
    pub fn stamp(&self) -> FileStamp {
        FileStamp::of(&self.file)
    }

    /// Reads the store as it is, strictly.
    fn read_strict(&self) -> Result<PrivateState, Problem> {
        match read_one(&self.file) {
            Read1::Absent => Ok(PrivateState::default()),
            Read1::Text(text) => parse_private(&text, STORE_NAME),
            Read1::Problem(problem) => Err(Problem::new(STORE_NAME, problem)),
        }
    }

    /// Loads the store. It never fails: a store that cannot be read as it
    /// is gives the good copy's state with the problem, and one with no
    /// readable good copy gives nothing with the problem.
    #[must_use]
    pub fn load(&self) -> Loaded {
        // The stamp first: a change between the two is then seen as one at
        // the next look, never missed.
        let stamp = self.stamp();
        let read = read_one(&self.file);
        let parsed = match &read {
            Read1::Absent => Ok(PrivateState::default()),
            Read1::Text(text) => parse_private(text, STORE_NAME),
            Read1::Problem(problem) => Err(Problem::new(STORE_NAME, problem.clone())),
        };
        match parsed {
            Ok(state) => {
                self.keep_good_copy(&read);
                Loaded {
                    source: if matches!(read, Read1::Text(_)) {
                        Source::File
                    } else {
                        Source::Nothing
                    },
                    state,
                    problem: None,
                    stamp,
                }
            }
            Err(problem) => {
                let good = match read_one(&self.good_copy()) {
                    Read1::Text(text) => parse_private(&text, STORE_NAME).ok(),
                    Read1::Absent | Read1::Problem(_) => None,
                };
                let (state, source) = match good {
                    Some(state) => (state, Source::GoodCopy),
                    None => (PrivateState::default(), Source::Nothing),
                };
                Loaded {
                    state,
                    source,
                    problem: Some(problem),
                    stamp,
                }
            }
        }
    }

    /// Brings the good copy up to the store a load just read as valid: a
    /// person's valid edit is then what a later, invalid one falls back
    /// to, and not an older state that may hold what they turned off. A
    /// store that is not there has no good copy: removing the file is how
    /// a user starts again, and a file made by hand afterwards must not
    /// fall back to what was removed.
    ///
    /// Best effort, under the lock, from the file as it is then: a load
    /// never fails for it.
    fn keep_good_copy(&self, read: &Read1) {
        let good = self.good_copy();
        let stale = match (read, read_one(&good)) {
            (Read1::Text(text), Read1::Text(copy)) => *text != copy,
            (Read1::Text(_), _) => true,
            (Read1::Absent, Read1::Absent) => false,
            (Read1::Absent, _) => true,
            (Read1::Problem(_), _) => false,
        };
        if !stale {
            return;
        }
        // Tried, never waited for: a load must not hang on a writer that
        // is stuck. The next load tries again.
        let Some(_lock) = self.try_lock() else {
            return;
        };
        match read_one(&self.file) {
            Read1::Text(text) if parse_private(&text, STORE_NAME).is_ok() => {
                let options = AtomicWriteOptions {
                    owner_only: true,
                    follow_links: false,
                    sync: false,
                    ..AtomicWriteOptions::default()
                };
                if let Err(error) = write_atomic(&good, text.as_bytes(), &options, &*self.clock) {
                    tracing::warn!(error = %error, "the good copy of the project's store was not brought up to date");
                }
            }
            Read1::Absent => {
                let _ = std::fs::remove_file(&good);
            }
            Read1::Text(_) | Read1::Problem(_) => {}
        }
    }

    /// The store's lock, when it can be had at once. `None` when another
    /// holds it, or it cannot be opened.
    fn try_lock(&self) -> Option<Option<std::fs::File>> {
        self.try_lock_once(false)
    }

    fn try_lock_once(&self, retried: bool) -> Option<Option<std::fs::File>> {
        let path = private::lock_path(&self.file);
        match cortex_base::open_regular(&path) {
            Ok((lock, _)) => match lock.try_lock() {
                Ok(()) => Some(Some(lock)),
                Err(std::fs::TryLockError::WouldBlock) => None,
                // A file system that cannot lock gives no lock, as for a
                // change.
                Err(std::fs::TryLockError::Error(error))
                    if error.kind() == io::ErrorKind::Unsupported =>
                {
                    Some(None)
                }
                Err(std::fs::TryLockError::Error(_)) => None,
            },
            // No change was ever made here. The lock file is made as a
            // change makes it, and then tried like any other: another
            // process may have made and taken it in this instant.
            Err(OpenError::NotFound) if !retried => {
                if let Some(directory) = self.file.parent() {
                    private::create_dir_all(directory).ok()?;
                }
                match private::create_new(&path) {
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(_) => return None,
                }
                self.try_lock_once(true)
            }
            Err(_) => None,
        }
    }

    /// Changes the store: takes the lock, reads the file, applies `change`
    /// and writes the result, when `change` says it changed anything (the
    /// second of what it returns). Returns `change`'s answer, the state as
    /// it now is, and what the file looked like while the lock was still
    /// held: a later change by another process then shows as a change,
    /// whenever it comes.
    ///
    /// # Errors
    ///
    /// [`StoreError::Invalid`] when the file cannot be read as it is: it
    /// is not written over. [`StoreError::Refused`] when `change` refuses.
    /// [`StoreError::Full`] when the result would pass the file's size
    /// cap, and [`StoreError::Unreadable`] when this build would not read
    /// it back. Otherwise what went wrong with the lock or the write. The
    /// store is not written in any of them.
    pub fn change<T, E>(
        &self,
        change: impl FnOnce(&mut PrivateState, UtcTime) -> Result<(T, bool), E>,
    ) -> Result<(T, PrivateState, FileStamp), StoreError<E>> {
        let io_error = |source| StoreError::Io {
            path: self.file.display().to_string(),
            source,
        };
        let _lock = private::hold_lock(&self.file).map_err(io_error)?;
        // The stamp is taken before the read, and again the moment the
        // store is written: a person's edit takes no lock, and one that
        // lands after either must show as a change at the next look, not
        // be taken for the state returned here.
        let read_at = self.stamp();
        let mut state = self.read_strict().map_err(StoreError::Invalid)?;
        let (answer, changed) =
            change(&mut state, self.clock.now()).map_err(StoreError::Refused)?;
        let mut stamp = read_at;
        if changed {
            let text = match writable(&state) {
                Ok(text) => text,
                Err(Unwritable::Full(message)) => return Err(StoreError::Full(message)),
                Err(Unwritable::Unreadable(problem)) => {
                    return Err(StoreError::Unreadable(problem));
                }
            };
            let options = AtomicWriteOptions {
                owner_only: true,
                follow_links: false,
                ..AtomicWriteOptions::default()
            };
            write_atomic(&self.file, text.as_bytes(), &options, &*self.clock)?;
            stamp = self.stamp();
            cortex_base::hook::point(AFTER_THE_STORE);
            // The good copy is what a later load falls back to. It can be
            // made again from the store, so it is not flushed; and a
            // failure to write it loses nothing that the store does not
            // hold.
            let copy = AtomicWriteOptions {
                sync: false,
                ..options
            };
            if let Err(error) =
                write_atomic(&self.good_copy(), text.as_bytes(), &copy, &*self.clock)
            {
                tracing::warn!(error = %error, "the good copy of the project's store was not written");
            }
        }
        Ok((answer, state, stamp))
    }
}

/// Why a state is not written.
pub(crate) enum Unwritable {
    /// Its text would pass the file's size cap.
    Full(String),
    /// This build would not read its text back.
    Unreadable(Problem),
}

/// The store's text for `state`, when it is one that may be written:
/// within the size cap, and read back by this build as it is. A promote
/// asks before it writes the shared file, so that a store that cannot
/// take the change stops it before anything is written.
pub(crate) fn writable(state: &PrivateState) -> Result<String, Unwritable> {
    let text = write_private(state);
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(Unwritable::Full(format!(
            "the change would make this project's store {} bytes, and it may be up to \
             {MAX_FILE_BYTES}: delete some entries, with `cortex project delete` or in the file",
            text.len()
        )));
    }
    parse_private(&text, STORE_NAME).map_err(Unwritable::Unreadable)?;
    Ok(text)
}

/// The point of [`Store::change`] where the store has its new content and
/// the good copy has not: a test holds it (`cortex_base::hook`) to see
/// what a crash between the two leaves.
pub const AFTER_THE_STORE: &str = "cortex-knowledge: the store is written, before its good copy";
