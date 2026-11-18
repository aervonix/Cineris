//! What the test kit's [`corpus`](super) and `cargo xtask corpus` both need
//! to know about the corpus of real code, in one file: its pin, how a folder
//! is measured and checked against the pin, and where the corpus is kept.
//!
//! **This file is included by two crates.** `xtask` takes it in with
//! `#[path]` (it cannot depend on the test kit: the test kit builds the HTTP
//! types and a terminal driver, which `xtask` leaves unbuilt on purpose), so
//! that the command that fetches the corpus and the test that reads it check
//! it with the same code. It uses the standard library alone, and what one
//! of the two crates does not call is allowed to go unused there.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "reads a folder of real files, from a test's own thread or a synchronous tool"
)]
#![allow(
    dead_code,
    unreachable_pub,
    missing_docs,
    reason = "included by two crates, each of which calls its own part"
)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

/// The pin: the corpus's hash, its number of files and its size in bytes,
/// as the repository's `corpus/PIN` holds them.
pub const PIN_TEXT: &str = include_str!("../../../../corpus/PIN");

/// The variable that names another folder to keep corpora in, in place of
/// the user's cache folder.
pub const ROOT_VARIABLE: &str = "CINERIS_CORPUS_DIR";

/// What a corpus must be: how many files, how many bytes, and the hash of
/// their paths and contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    pub hash: u64,
    pub files: u64,
    pub bytes: u64,
}

impl Pin {
    /// The repository's pin.
    ///
    /// # Panics
    ///
    /// If `corpus/PIN` is not a pin: a mistake of the repository's, found by
    /// the first test that reads it.
    #[must_use]
    pub fn pinned() -> Self {
        Self::parse(PIN_TEXT).unwrap_or_else(|problem| panic!("corpus/PIN: {problem}"))
    }

    /// A pin from the text of `corpus/PIN`: lines of `key = value`, with
    /// `#` comments and blank lines left out. `hash` is sixteen hexadecimal
    /// digits; `files` and `bytes` are numbers.
    ///
    /// # Errors
    ///
    /// Says which key is missing, repeated or not understood.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (mut hash, mut files, mut bytes) = (None, None, None);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .map(|(key, value)| (key.trim(), value.trim()))
                .ok_or_else(|| format!("`{line}` is not `key = value`"))?;
            let taken = match key {
                "hash" => hash.replace(parse_hash(value)?),
                "files" => files.replace(parse_number(key, value)?),
                "bytes" => bytes.replace(parse_number(key, value)?),
                _ => return Err(format!("`{key}` is not a key of a pin")),
            };
            if taken.is_some() {
                return Err(format!("`{key}` is given twice"));
            }
        }
        Ok(Self {
            hash: hash.ok_or("no `hash`")?,
            files: files.ok_or("no `files`")?,
            bytes: bytes.ok_or("no `bytes`")?,
        })
    }

    /// The folder's name: the hash in sixteen hexadecimal digits.
    #[must_use]
    pub fn name(&self) -> String {
        format!("{:016x}", self.hash)
    }

    /// The pin's text, as `corpus/PIN` holds it.
    #[must_use]
    pub fn text(&self) -> String {
        format!(
            "# The corpus of real code `cargo xtask corpus` keeps: what a folder of it must hold\n\
             # (docs/testing-and-ci.md, \"A tree of real code to measure\"). Written by\n\
             # `cargo xtask corpus pin`; not by hand.\n\
             hash = {}\nfiles = {}\nbytes = {}\n",
            self.name(),
            self.files,
            self.bytes
        )
    }
}

fn parse_hash(value: &str) -> Result<u64, String> {
    if value.len() != 16 {
        return Err(format!(
            "`hash` is sixteen hexadecimal digits, not `{value}`"
        ));
    }
    u64::from_str_radix(value, 16).map_err(|_| format!("`hash` is not hexadecimal: `{value}`"))
}

fn parse_number(key: &str, value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("`{key}` is a number, not `{value}`"))
}

/// 64-bit FNV-1a, the hash `mutate` keeps its results by: small, and the same
/// on every machine and toolchain. It is here to see that the folder holds
/// what was pinned, not to resist anyone: the crates themselves are checked
/// by cargo, against the SHA-256 sums of `corpus/Cargo.lock`. A changed byte
/// always changes it (each byte is one step of a bijection of the state).
#[derive(Debug, Clone, Copy)]
pub struct Fnv(pub u64);

impl Fnv {
    #[must_use]
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Self::new()
    }
}

/// What a folder holds: files, bytes and the hash of their paths and
/// contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measured {
    pub files: u64,
    pub bytes: u64,
    pub hash: u64,
}

impl Measured {
    /// The pin this is, for writing it down.
    #[must_use]
    pub fn as_pin(&self) -> Pin {
        Pin {
            hash: self.hash,
            files: self.files,
            bytes: self.bytes,
        }
    }
}

/// The files under `dir`, as paths from it written with `/`, in byte order of
/// those paths: the order the hash is taken in, the same on every system.
///
/// # Errors
///
/// A folder that cannot be read, a name that is not text, and anything that
/// is neither a folder nor a regular file (a link is not followed: a corpus
/// of links is not the corpus).
pub fn list(dir: &Path) -> Result<Vec<String>, String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), String> {
        let entries = fs::read_dir(dir)
            .map_err(|error| format!("cannot read the folder {}: {error}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
            let name = entry.file_name().into_string().map_err(|name| {
                format!("{} holds a name that is not text: {name:?}", dir.display())
            })?;
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let kind = entry
                .file_type()
                .map_err(|error| format!("cannot look at {relative}: {error}"))?;
            if kind.is_dir() {
                walk(&entry.path(), &relative, out)?;
            } else if kind.is_file() {
                out.push(relative);
            } else {
                return Err(format!("{relative} is neither a folder nor a regular file"));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    files.sort_unstable();
    Ok(files)
}

/// Counts and hashes what `dir` holds: for each file in the order of
/// [`list`], its path, a zero byte, its length as eight bytes (little-endian)
/// and its content.
///
/// # Errors
///
/// A file or folder that cannot be read, as [`list`] says.
pub fn measure(dir: &Path) -> Result<Measured, String> {
    let files = list(dir)?;
    let mut hash = Fnv::new();
    let mut bytes = 0u64;
    for relative in &files {
        let path = dir.join(relative);
        let content =
            fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        hash.write(relative.as_bytes());
        hash.write(&[0]);
        hash.write(&(content.len() as u64).to_le_bytes());
        hash.write(&content);
        bytes += content.len() as u64;
    }
    Ok(Measured {
        files: files.len() as u64,
        bytes,
        hash: hash.0,
    })
}

/// Checks that `dir` is the corpus `pin` describes: there, with the number
/// of files and the bytes the pin says, and the hash. A file that is missing
/// or not meant to be there changes the count; a byte that changed, or a file
/// of another name, changes the hash.
///
/// # Errors
///
/// What differs, in words for the person at the terminal.
pub fn verify(dir: &Path, pin: &Pin) -> Result<Measured, String> {
    if !dir.is_dir() {
        return Err(format!("{} is not there", dir.display()));
    }
    let found = measure(dir)?;
    if found.files != pin.files {
        return Err(format!(
            "{} holds {} files, and the pin says {}: a file is missing or is not meant to be there",
            dir.display(),
            found.files,
            pin.files
        ));
    }
    if found.bytes != pin.bytes {
        return Err(format!(
            "{} holds {} bytes, and the pin says {}: a file has changed",
            dir.display(),
            found.bytes,
            pin.bytes
        ));
    }
    if found.hash != pin.hash {
        return Err(format!(
            "{} hashes to {:016x}, and the pin says {}: a file has changed, or has another name",
            dir.display(),
            found.hash,
            pin.name()
        ));
    }
    Ok(found)
}

/// The folder corpora are kept in, from a way of reading the environment and
/// whether the system is Windows. `CINERIS_CORPUS_DIR` first; then on Windows
/// `%LOCALAPPDATA%\cineris-dev\corpus`; otherwise
/// `$XDG_CACHE_HOME/cineris-dev/corpus`, or `~/.cache/cineris-dev/corpus`
/// when that is not set. It is outside the repository and its `target`, so
/// that `cargo clean` and a new worktree leave it be.
///
/// # Errors
///
/// When the environment gives nowhere to keep it, or a relative path.
pub fn cache_root(
    variable: &dyn Fn(&str) -> Option<OsString>,
    windows: bool,
) -> Result<PathBuf, String> {
    let given = |name: &str| variable(name).filter(|value| !value.is_empty());
    if let Some(root) = given(ROOT_VARIABLE) {
        let root = PathBuf::from(root);
        if !root.is_absolute() {
            return Err(format!(
                "{ROOT_VARIABLE} is {}, which is not an absolute path",
                root.display()
            ));
        }
        return Ok(root);
    }
    let base = if windows {
        given("LOCALAPPDATA").map(PathBuf::from).ok_or(
            "%LOCALAPPDATA% is not set: set CINERIS_CORPUS_DIR to a folder to keep the corpus in",
        )?
    } else if let Some(cache) = given("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        cache
    } else {
        given("HOME")
            .map(|home| PathBuf::from(home).join(".cache"))
            .ok_or("$HOME is not set: set CINERIS_CORPUS_DIR to a folder to keep the corpus in")?
    };
    Ok(base.join("cineris-dev").join("corpus"))
}

/// [`cache_root`] for the system this runs on.
///
/// # Errors
///
/// As [`cache_root`].
pub fn system_root() -> Result<PathBuf, String> {
    cache_root(&|name| std::env::var_os(name), cfg!(windows))
}

/// Where the corpus of `pin` is kept under `root`.
#[must_use]
pub fn folder(root: &Path, pin: &Pin) -> PathBuf {
    root.join(pin.name())
}
