//! The registration cache: what each approved extension's script
//! registered, kept by the extension's content hash in
//! `<state dir>/extensions/registered.json`, so that a session knows every
//! approved extension's tools, commands, hooks and prompt text without
//! starting the engine (`docs/steps/28/answered.md`, point 4, "The
//! registration cache"; point 8, "Start-up").
//!
//! # The stored file
//!
//! ```json
//! {"version": 1, "entries": {"5e884898da28…": {"name": "tracker", "api": 1,
//!   "tools": [{"name": "find_issue", "description": "…", "input": {…}, "read_only": true}],
//!   "commands": [], "hooks": [{"event": "tool.before", "tools": ["edit"], "observe": false,
//!   "timeout_ms": null}], "prompt": "…"}}}
//! ```
//!
//! - **Keyed by the content hash** of the extension's folder
//!   ([`Snapshot::hash`](crate::folder::Snapshot::hash)), which is what is
//!   pinned: an entry is the registration of exactly those bytes. The
//!   user's `tracker` and a project's `tracker` are two hashes and two
//!   entries. `prompt` is the text of the manifest's prompt file, absent
//!   when it names none: the approved text, which section 7 of the system
//!   prompt takes from here.
//! - **Written when an extension is approved** (its script evaluated once
//!   in a throwaway runtime whose host functions record what it
//!   registers), and by a session that finds an approved extension with no
//!   entry ([`RegistrationCache::put`]). **Pruned** of the hashes no pin
//!   names ([`RegistrationCache::prune`]).
//! - **Read strictly** (decision 0028, `docs/decisions/0028-enforced-values-fail-closed.md`):
//!   a field this build does not know (at the top, in an entry, a tool, a
//!   command or a hook), a value of the wrong type, a key that is not a
//!   content hash (64 lowercase hexadecimal digits), a key given twice, a
//!   `version` other than [`VERSION`], more than [`MAX_ENTRIES`] entries,
//!   an entry a live runtime could not have registered, or a file larger
//!   than [`MAX_BYTES`] or that is not a regular file sets **the whole
//!   file** aside: a file that cannot be read in full is not read in part.
//!   A version other than this one is said as such, not as a field it adds.
//! - **An entry is checked by the code a live registration meets**
//!   ([`Registration::check`], in [`crate::registration`]): every tool's and
//!   command's name, description (2 KiB, nothing that acts unseen) and
//!   schema (an object of 16 KiB, every text shown as written), a command's
//!   arguments (16, each a name), a hook's event, its filter (64 entries,
//!   each a tool's name or `source/*`) and its time (1 ms to 10 minutes),
//!   the counts (64 tools, 32 commands, 32 hooks), no tool or command name
//!   twice; and the entry's own name (an extension's) and prompt (4 KiB).
//!   Whether its hooks are ones the manifest allows cannot be asked here,
//!   with no manifest: [`Entry::fits`] asks it, with the name, the `api` and
//!   the prompt's text, of the manifest and snapshot of the extension the
//!   entry is taken for, which the core does before any runtime starts.
//! - **Set aside** is two things: the cache is taken as empty, and the
//!   caller is told why ([`Loaded::set_aside`]); and what stands at the
//!   name is moved to `registered.json.set-aside` beside it (replacing a
//!   file moved there before), so that the next write starts a new file and
//!   the old one is there for whoever wants to see what was in it.
//!   - **A file is moved only while it is still the file that was read,** so
//!     a cache another session has just written is not moved for what the
//!     one before it held. The look and the move are two steps: a cache
//!     another session writes between them is moved, seen to be another
//!     file, and put back by a hard link, which is made only where the name
//!     is free. Only the harness's own sessions write the state directory,
//!     which every tool is closed to, so the race is theirs alone, and what
//!     it could cost is a cache lost and every extension registered again.
//!   - **A link at the name, a folder, a FIFO, or a file this user may not
//!     open** is moved aside as it is (the link itself, never what it leads
//!     to): it is nothing the cache can read, and with it at the name no
//!     write could replace it, so that every session would register every
//!     approved extension again. A file another program holds open (on
//!     Windows) is left alone.
//!   - **A newer build's file is set aside too:** a user who went back to
//!     this build would otherwise have no cache, and the newer build
//!     registers again what it misses.
//! - **A missing entry fails closed.** The caller registers an approved
//!   extension the cache does not hold when the session opens, off the
//!   start-up path, and the first turn waits for it, so a gate it holds is
//!   never skipped. A missing file is an empty cache, and reading it makes
//!   nothing.
//! - **Written owner-only and atomically** ([`cineris_base::write_atomic`],
//!   [`cineris_base::private`]), its folder made owner-only when it is
//!   missing, never through a link at its name. It is not flushed to the
//!   disk before the rename: it can be rebuilt, and a file a power failure
//!   left short is set aside on the next read. A write that would make a
//!   file this build would set aside (too large, too many entries, an entry
//!   past the caps) is refused, and writes nothing.
//! - **Two processes writing at once may lose one entry.** Each
//!   [`put`](RegistrationCache::put) reads the file, adds its entry and
//!   replaces the file whole, with no lock between the read and the
//!   replace: when two do so at once, the later replace wins and the
//!   earlier one's entry is not in the file. That fails closed: the next
//!   session finds the extension missing and registers it again, which
//!   costs one evaluation of its script in a throwaway runtime and nothing
//!   else. No entry is ever made wrong by it, since an entry is keyed by the
//!   hash of what it registered. A lock (the one the pins are kept with)
//!   would save that evaluation at the price of a lock file and a wait on
//!   every write; for a cache that is not worth it. Readers take no lock:
//!   the file is replaced in one step and read whole, old or new.
//! - **No tool can reach it.** It is in the state directory, which the
//!   built-in protection closes to every tool, to reading and to writing,
//!   at every level.
//!
//! These functions block: the caller runs them on the blocking pool.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the cache is a file, read and written on the blocking pool"
)]

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cineris_base::{
    AtomicWriteError, AtomicWriteOptions, ContentHash, FileIdentity, OpenError, SharedClock,
    SystemClock, open_regular, private, visible, write_atomic,
};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::folder::Snapshot;
use crate::manifest::{self, Manifest};
use crate::registration::{CommandDef, HookDef, Registration, ToolDef};

/// The version of the file's format: the only one this build reads, and
/// the one it writes.
pub const VERSION: u32 = 1;

/// The most entries the file may hold.
///
/// An entry is one approved extension's registration, of the user's or of
/// a project the user trusts, and hashes no pin names are pruned: a user
/// has a handful, a machine with many projects a few dozen. 1024 is far
/// past that, and is still a map read in well under a millisecond.
pub const MAX_ENTRIES: usize = 1024;

/// The largest file read, and written: 16 MiB.
///
/// An entry is bounded by the registration's caps: 64 tools, each with a
/// 2 KiB description and a 16 KiB schema, 32 commands, 32 hooks naming 64
/// tools each, and a 4 KiB prompt make about 1.5 MiB of JSON, and a
/// typical extension's entry is a few KiB. 16 MiB holds ten extensions at
/// every cap, or thousands of typical ones, and bounds what a damaged or
/// planted file makes a session read before the file is set aside.
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// What is added to the file's name for the file set aside.
pub const SET_ASIDE_SUFFIX: &str = ".set-aside";

/// A point of the cache's: a file that cannot be read in full, looked at
/// and still the one that was read, is about to be moved aside.
pub const BEFORE_IT_IS_SET_ASIDE: &str = "cache: a file read and refused is about to be set aside";

/// A point of the cache's: the file is about to be written.
pub const BEFORE_IT_IS_WRITTEN: &str = "cache: the file is about to be written";

/// What one approved extension registered: the part of the cache a
/// session reads instead of starting the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The extension's name, its manifest's.
    pub name: String,
    /// The manifest's `api`: the version of the host functions it was
    /// written for.
    pub api: u32,
    /// Its tools, commands and hooks, in the order it registered them.
    pub registration: Registration,
    /// The text of its prompt file, when its manifest names one: section 7
    /// of the system prompt, at most 4 KiB.
    pub prompt: Option<String>,
}

impl Entry {
    /// The entry of an extension with this manifest, which registered
    /// `registration`, and whose prompt file holds `prompt`.
    #[must_use]
    pub fn new(manifest: &Manifest, registration: Registration, prompt: Option<String>) -> Self {
        Self {
            name: manifest.name.clone(),
            api: manifest::API,
            registration,
            prompt,
        }
    }

    /// Whether the entry is one the cache keeps: what a live runtime could
    /// have registered, checked by the same code a live registration meets
    /// ([`Registration::check`]: every name, description, schema, filter
    /// and time, and the caps), an extension's name, and a prompt of at
    /// most 4 KiB. Said as why it is not.
    fn check(&self) -> Result<(), String> {
        if !manifest::is_name(&self.name) {
            return Err(format!(
                "`{}` is not an extension's name: 1 to 64 of ASCII letters, digits, `_`, `-` and \
                 `.`, not starting with `.`",
                visible(cineris_base::text::truncate_bytes(&self.name, 80))
            ));
        }
        self.registration.check()?;
        if let Some(prompt) = &self.prompt
            && prompt.len() > manifest::MAX_PROMPT_BYTES
        {
            return Err(format!(
                "its prompt is {} bytes, more than the {} a prompt file may hold",
                prompt.len(),
                manifest::MAX_PROMPT_BYTES
            ));
        }
        Ok(())
    }

    /// Whether the entry is the registration of the extension with this
    /// manifest and snapshot, which it is kept under the hash of: its name
    /// is the manifest's, its `api` this build's, every hook on an event the
    /// manifest's `capabilities.hooks` allows (which the cache cannot ask
    /// when it reads the file, having no manifest), and its prompt the text
    /// of the snapshot's prompt file. The core asks it as it takes an entry
    /// for an extension, before any runtime starts.
    ///
    /// # Errors
    ///
    /// What does not fit, in words.
    pub fn fits(&self, manifest: &Manifest, snapshot: &Snapshot) -> Result<(), String> {
        if self.name != manifest.name {
            return Err(format!(
                "the entry is of `{}`, and the extension is `{}`",
                visible(&self.name),
                manifest.name
            ));
        }
        if self.api != manifest::API {
            return Err(format!(
                "the entry is of `api = {}`, and this build knows {}",
                self.api,
                manifest::API
            ));
        }
        self.registration
            .check_hooks(&manifest.capabilities.hooks)?;
        let prompt = manifest
            .prompt
            .as_deref()
            .and_then(|file| snapshot.file(file))
            .map(String::from_utf8_lossy);
        if self.prompt.as_deref() != prompt.as_deref() {
            return Err(
                "the entry's prompt is not the text of the extension's prompt file".to_owned(),
            );
        }
        Ok(())
    }
}

/// The entries of the cache, by content hash.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cache {
    entries: BTreeMap<ContentHash, Entry>,
}

impl Cache {
    /// The entry of the extension whose folder has this hash.
    #[must_use]
    pub fn get(&self, hash: &ContentHash) -> Option<&Entry> {
        self.entries.get(hash)
    }

    /// How many entries it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every entry, by hash.
    pub fn iter(&self) -> impl Iterator<Item = (&ContentHash, &Entry)> {
        self.entries.iter()
    }
}

/// What [`RegistrationCache::load`] read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loaded {
    /// The entries: none when the file is missing or was set aside.
    pub cache: Cache,
    /// When the file was set aside, why, in a sentence for the user that
    /// names the file and says where it went.
    pub set_aside: Option<String>,
}

/// Why the cache could not be written. The file is as it was.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CacheError {
    /// The entry is not one a live runtime could have registered.
    #[error("the registration of `{name}` is not one the registration cache keeps: {problem}")]
    Entry {
        /// The extension's name, escaped.
        name: String,
        /// Why.
        problem: String,
    },
    /// The file would hold more than [`MAX_ENTRIES`] entries.
    #[error(
        "the registration cache `{path}` would hold more than {MAX_ENTRIES} entries: the entry is not kept, and a session registers the extension again until the cache is pruned"
    )]
    Full {
        /// The file.
        path: String,
    },
    /// The file would be larger than [`MAX_BYTES`].
    #[error(
        "the registration cache `{path}` would be {bytes} bytes, more than {MAX_BYTES}: the entry is not kept, and a session registers the extension again"
    )]
    TooLarge {
        /// The file.
        path: String,
        /// How large it would have been.
        bytes: usize,
    },
    /// The file's folder could not be made.
    #[error("the folder of the registration cache `{path}` could not be made: {source}")]
    Folder {
        /// The folder.
        path: String,
        /// Why.
        #[source]
        source: io::Error,
    },
    /// The entries could not be written out as JSON.
    #[error("the registration cache could not be written out: {0}")]
    Encode(String),
    /// The file could not be written.
    #[error("the registration cache `{path}` could not be written: {source}")]
    Write {
        /// The file.
        path: String,
        /// Why.
        #[source]
        source: AtomicWriteError,
    },
}

/// The registration cache's file.
#[derive(Debug, Clone)]
pub struct RegistrationCache {
    file: PathBuf,
    clock: SharedClock,
    /// The largest file read or written: [`MAX_BYTES`], less in a test.
    max_bytes: u64,
}

impl RegistrationCache {
    /// The cache at `file`: `<state dir>/extensions/registered.json`.
    #[must_use]
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            clock: Arc::new(SystemClock::new()),
            max_bytes: MAX_BYTES,
        }
    }

    /// With the clock a write waits on, on Windows, while another program
    /// holds the file open ([`write_atomic`]).
    #[must_use]
    pub fn with_clock(mut self, clock: SharedClock) -> Self {
        self.clock = clock;
        self
    }

    /// The file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.file
    }

    /// Where a file set aside goes: its name with [`SET_ASIDE_SUFFIX`].
    #[must_use]
    pub fn set_aside_path(&self) -> PathBuf {
        let mut name = self.file.as_os_str().to_owned();
        name.push(SET_ASIDE_SUFFIX);
        PathBuf::from(name)
    }

    /// The entries. Never an error: a missing file is an empty cache, and
    /// one that cannot be read in full is an empty cache and set aside
    /// ([`Loaded::set_aside`] says why).
    #[must_use]
    pub fn load(&self) -> Loaded {
        match self.read() {
            Ok(cache) => Loaded {
                cache,
                set_aside: None,
            },
            Err(bad) => Loaded {
                cache: Cache::default(),
                set_aside: Some(self.set_aside(&bad)),
            },
        }
    }

    /// Keeps `entry` as the registration of the extension whose folder has
    /// `hash`, replacing an entry of the same hash: the file is read, the
    /// entry added and the file written whole, atomically and owner-only.
    /// A file that cannot be read in full is set aside first, as
    /// [`load`](Self::load) sets it aside, and a new one is begun.
    ///
    /// # Errors
    ///
    /// When the entry is not one a live runtime could have registered, when
    /// the file would be one that is set aside when read (too large, too
    /// many entries), or when it cannot be written. The file is then as it
    /// was.
    pub fn put(&self, hash: &ContentHash, entry: Entry) -> Result<(), CacheError> {
        entry.check().map_err(|problem| CacheError::Entry {
            name: visible(&entry.name).into_owned(),
            problem,
        })?;
        let mut cache = self.read_for_change();
        cache.entries.insert(*hash, entry);
        self.write(cache)
    }

    /// Takes out every entry whose hash `keep` does not keep: the hashes no
    /// pin names. Says how many were taken out; when none was, nothing is
    /// written. A file that cannot be read in full is set aside, as
    /// [`load`](Self::load) sets it aside, and nothing is taken out of it.
    ///
    /// # Errors
    ///
    /// When the file cannot be written. It is then as it was.
    pub fn prune(&self, keep: impl Fn(&ContentHash) -> bool) -> Result<usize, CacheError> {
        let mut cache = self.read_for_change();
        let before = cache.len();
        cache.entries.retain(|hash, _| keep(hash));
        let removed = before - cache.len();
        if removed > 0 {
            self.write(cache)?;
        }
        Ok(removed)
    }

    fn shown(&self) -> String {
        self.file.display().to_string()
    }

    /// The file as it is, for a change: a file that cannot be read in full
    /// is set aside, and the change begins from nothing.
    fn read_for_change(&self) -> Cache {
        self.read().unwrap_or_else(|bad| {
            let _ = self.set_aside(&bad);
            Cache::default()
        })
    }

    /// Reads the whole file: `Ok` with nothing in it when it is missing.
    fn read(&self) -> Result<Cache, Bad> {
        let (file, metadata) = match open_regular(&self.file) {
            Ok(opened) => opened,
            Err(OpenError::NotFound) => return Ok(Cache::default()),
            // Held open by another program, on Windows: nothing is wrong
            // with it, and it is left alone.
            Err(error) if error.is_in_use() => {
                return Err(Bad {
                    problem: format!("it could not be read: {error}"),
                    at: At::InUse,
                });
            }
            // A link, a folder, a FIFO, or a file this user may not open:
            // nothing the cache can read or write, moved aside as it is.
            Err(error) => {
                return Err(Bad {
                    problem: format!("it could not be read: {error}"),
                    at: At::NotAFile,
                });
            }
        };
        let identity = FileIdentity::of(&file).ok();
        let bad = |problem: String| Bad {
            problem,
            at: At::Read(identity),
        };
        let too_large = |bytes: u64| {
            bad(format!(
                "it is {bytes} bytes, more than the {} a registration cache may be",
                self.max_bytes
            ))
        };
        if metadata.len() > self.max_bytes {
            return Err(too_large(metadata.len()));
        }
        // One byte past the cap says the file grew past it since its size
        // was asked.
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        file.take(self.max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| bad(format!("it could not be read: {error}")))?;
        if bytes.len() as u64 > self.max_bytes {
            return Err(too_large(bytes.len() as u64));
        }
        parse(&bytes).map_err(bad)
    }

    /// Sets what cannot be read in full aside: moves it to
    /// [`set_aside_path`](Self::set_aside_path), a file only while it is
    /// still the file that was read, and anything else that stands at the
    /// name (a link, moved and never followed; a folder) as it is. Gives the
    /// sentence that says why and where.
    fn set_aside(&self, bad: &Bad) -> String {
        let aside = self.set_aside_path();
        let again = " It was left as it is: another session had just written it again.";
        let moved = match bad.at {
            At::InUse => " It was left as it is: another program holds it open.".to_owned(),
            // Not known: it cannot be told from a cache written since.
            At::Read(None) => " It was left as it is.".to_owned(),
            At::Read(Some(read)) if FileIdentity::at(&self.file).ok() != Some(read) => {
                again.to_owned()
            }
            At::Read(Some(read)) => {
                cineris_base::hook::point(BEFORE_IT_IS_SET_ASIDE);
                match self.move_aside(&aside) {
                    Err(error) => {
                        format!(" It could not be moved aside ({error}), and is left as it is.")
                    }
                    Ok(()) if FileIdentity::at(&aside).ok() == Some(read) => {
                        format!(" It was moved to `{}`.", aside.display())
                    }
                    // Another session wrote the file between the look and
                    // the move: what was moved is that session's cache,
                    // put back while the name is still free (a hard link
                    // is made only where nothing is).
                    Ok(()) => match std::fs::hard_link(&aside, &self.file) {
                        Ok(()) => {
                            let _ = std::fs::remove_file(&aside);
                            again.to_owned()
                        }
                        Err(_) => format!(
                            " A cache another session had just written was moved with it, to `{}`.",
                            aside.display()
                        ),
                    },
                }
            }
            At::NotAFile => match self.move_aside(&aside) {
                Ok(()) => format!(
                    " It was moved to `{}` as it is (a link is moved, and what it leads to is left \
                     alone), so that the next write makes the file anew.",
                    aside.display()
                ),
                Err(error) => format!(
                    " It could not be moved aside ({error}): until it is, the cache cannot be \
                     written, and every session registers each approved extension again."
                ),
            },
        };
        format!(
            "the registration cache `{}` was set aside, and every extension it held is registered again: {}.{moved}",
            self.shown(),
            bad.problem
        )
    }

    /// Moves what stands at the file's name to `aside`, replacing what was
    /// set aside before, unless that is a folder. A rename moves a name, so
    /// a link is moved, never what it leads to.
    fn move_aside(&self, aside: &Path) -> io::Result<()> {
        // A rename replaces a file but, on Linux, not with a folder.
        if let Ok(found) = std::fs::symlink_metadata(aside)
            && !found.is_dir()
        {
            std::fs::remove_file(aside)?;
        }
        std::fs::rename(&self.file, aside)
    }

    fn write(&self, cache: Cache) -> Result<(), CacheError> {
        if cache.len() > MAX_ENTRIES {
            return Err(CacheError::Full { path: self.shown() });
        }
        let file = RawFile {
            version: VERSION,
            entries: RawEntries(
                cache
                    .entries
                    .into_iter()
                    .map(|(hash, entry)| (hash, RawEntry::from(entry)))
                    .collect(),
            ),
        };
        let text =
            serde_json::to_vec(&file).map_err(|error| CacheError::Encode(error.to_string()))?;
        if text.len() as u64 > self.max_bytes {
            return Err(CacheError::TooLarge {
                path: self.shown(),
                bytes: text.len(),
            });
        }
        if let Some(folder) = self
            .file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            private::create_dir_all(folder).map_err(|source| CacheError::Folder {
                path: folder.display().to_string(),
                source,
            })?;
        }
        let options = AtomicWriteOptions {
            sync: false,
            owner_only: true,
            follow_links: false,
            ..AtomicWriteOptions::default()
        };
        cineris_base::hook::point(BEFORE_IT_IS_WRITTEN);
        write_atomic(&self.file, &text, &options, &*self.clock).map_err(|source| {
            CacheError::Write {
                path: self.shown(),
                source,
            }
        })
    }
}

/// What cannot be read in full: why, and what stands at the name.
struct Bad {
    problem: String,
    at: At,
}

/// What stands at the file's name, when it cannot be read in full.
enum At {
    /// A regular file, read, by its identity when the system gave one.
    Read(Option<FileIdentity>),
    /// Something the cache can neither read nor write: a link, a folder, a
    /// FIFO, a file this user may not open.
    NotAFile,
    /// A file another program holds open (only Windows refuses an open for
    /// that), left alone.
    InUse,
}

/// The entries of a file's text, or why the file is set aside.
fn parse(bytes: &[u8]) -> Result<Cache, String> {
    let raw = match serde_json::from_slice::<RawFile>(bytes) {
        Ok(raw) => raw,
        Err(error) => {
            // A file of another version is set aside for its version, not
            // for a field that version adds.
            if let Ok(Versioned { version }) = serde_json::from_slice::<Versioned>(bytes)
                && version != VERSION
            {
                return Err(version_problem(version));
            }
            return Err(format!(
                "it is not a registration cache this build reads in full: {error}"
            ));
        }
    };
    if raw.version != VERSION {
        return Err(version_problem(raw.version));
    }
    let mut entries = BTreeMap::new();
    for (hash, raw) in raw.entries.0 {
        let entry = Entry::from(raw);
        entry.check().map_err(|problem| {
            format!(
                "its entry `{hash}` (`{}`) is not one a live runtime could have registered: {problem}",
                visible(&entry.name)
            )
        })?;
        entries.insert(hash, entry);
    }
    Ok(Cache { entries })
}

fn version_problem(version: u32) -> String {
    if version > VERSION {
        format!(
            "it is of version {version}, written by a newer build, and this build reads version {VERSION}"
        )
    } else {
        format!("it is of version {version}, and this build reads version {VERSION}")
    }
}

/// The version alone, read when the whole file is not, to say why.
#[derive(Deserialize)]
struct Versioned {
    version: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    version: u32,
    entries: RawEntries,
}

/// The entries as the file holds them: a map whose keys are content
/// hashes, read so that a key given twice, or more than [`MAX_ENTRIES`]
/// keys, refuses the file.
#[derive(Serialize)]
#[serde(transparent)]
struct RawEntries(BTreeMap<ContentHash, RawEntry>);

impl<'de> Deserialize<'de> for RawEntries {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = RawEntries;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of entries, each under its content hash")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawEntries, A::Error> {
                let mut entries = BTreeMap::new();
                while let Some(hash) = map.next_key::<ContentHash>()? {
                    if entries.len() == MAX_ENTRIES {
                        return Err(de::Error::custom(format!(
                            "it holds more than {MAX_ENTRIES} entries"
                        )));
                    }
                    let entry: RawEntry = map.next_value()?;
                    if entries.insert(hash, entry).is_some() {
                        return Err(de::Error::custom(format!(
                            "it holds the entry `{hash}` twice"
                        )));
                    }
                }
                Ok(RawEntries(entries))
            }
        }
        deserializer.deserialize_map(Entries)
    }
}

/// One entry as the file holds it: every field required but `prompt`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    name: String,
    api: u32,
    tools: Vec<ToolDef>,
    commands: Vec<CommandDef>,
    hooks: Vec<HookDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
}

impl From<Entry> for RawEntry {
    fn from(entry: Entry) -> Self {
        Self {
            name: entry.name,
            api: entry.api,
            tools: entry.registration.tools,
            commands: entry.registration.commands,
            hooks: entry.registration.hooks,
            prompt: entry.prompt,
        }
    }
}

impl From<RawEntry> for Entry {
    fn from(raw: RawEntry) -> Self {
        Self {
            name: raw.name,
            api: raw.api,
            registration: Registration {
                tools: raw.tools,
                commands: raw.commands,
                hooks: raw.hooks,
            },
            prompt: raw.prompt,
        }
    }
}

#[cfg(test)]
mod tests {
    use cineris_testkit::scratch::Scratch;

    use super::*;

    fn entry(description: &str) -> Entry {
        Entry {
            name: "tracker".to_owned(),
            api: manifest::API,
            registration: Registration {
                tools: vec![ToolDef {
                    name: "find".to_owned(),
                    description: description.to_owned(),
                    input: serde_json::json!({"type": "object"}),
                    read_only: false,
                }],
                ..Registration::default()
            },
            prompt: None,
        }
    }

    /// One entry within every cap is far below 16 MiB, so the write's cap
    /// is held here, at a smaller one.
    #[test]
    fn a_put_that_would_make_the_file_larger_than_its_cap_writes_nothing() {
        let scratch = Scratch::outside(
            "cineris-ext-cache",
            "a_put_that_would_make_the_file_larger_than_its_cap_writes_nothing",
        );
        let cache = RegistrationCache {
            max_bytes: 1024,
            ..RegistrationCache::new(scratch.join("registered.json"))
        };
        let small = ContentHash::of(b"small");
        cache.put(&small, entry("short")).unwrap();
        let before = scratch.read("registered.json");
        let error = cache
            .put(&ContentHash::of(b"large"), entry(&"d".repeat(2000)))
            .unwrap_err();
        assert!(
            matches!(error, CacheError::TooLarge { bytes, .. } if bytes > 1024),
            "{error}"
        );
        assert_eq!(
            scratch.read("registered.json"),
            before,
            "nothing was written"
        );
        assert_eq!(cache.load().cache.len(), 1);
    }
}

/// The cache's file read, written, set aside and pruned, on a real disk.
#[cfg(test)]
mod on_disk;
