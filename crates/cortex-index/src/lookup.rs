//! What the rest of the harness asks an index: the map of the workspace,
//! and the outline of one file.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a lookup blocks on the file system, and runs on the blocking pool"
)]

use std::io::Read as _;

use cortex_base::{AbsPath, open_regular};

use crate::extract::extract;
use crate::language::Language;
use crate::map::{MapRequest, render};
use crate::outline::Outline;
use crate::pass::Index;
use crate::text::{decode, is_binary};
use crate::walk::Stamp;

/// One file's outline, as the `outline` tool and `read` are given it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOutline {
    /// The outline.
    pub outline: Outline,
    /// The file's size in bytes.
    pub size: u64,
    /// The outline is of the file's first this many bytes: the file is
    /// larger than the cap on one file.
    pub covered: Option<u64>,
}

/// Why a file has no outline.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutlineError {
    /// The file is not inside the workspace: the index is the workspace's.
    #[error("it is outside the workspace")]
    Outside,
    /// It could not be opened as a regular file.
    #[error("it cannot be opened: {0}")]
    Unreadable(String),
    /// It is a binary file.
    #[error("it is a binary file")]
    Binary,
}

impl Index {
    /// The map of the workspace, built from the cache as it is now, or
    /// `None` when the cache is missing or has nothing to show. Nothing is
    /// kept once it is returned.
    #[must_use]
    pub fn map(&self, request: &MapRequest) -> Option<String> {
        let cache = self.cache()?;
        render(&cache, request, self.limits().max_files)
    }

    /// The outline of the file at `path`: the cached one when the file is
    /// as the cache last saw it, and one made now otherwise. A file over
    /// the cap is outlined from its first bytes up to the cap.
    ///
    /// # Errors
    ///
    /// [`OutlineError`] when the file is outside the workspace, cannot be
    /// opened as a regular file, or is binary.
    pub fn outline_of(&self, path: &AbsPath) -> Result<FileOutline, OutlineError> {
        let relative = path
            .strip_prefix(self.root())
            .filter(|relative| !relative.is_empty())
            .ok_or(OutlineError::Outside)?;
        let (handle, metadata) = open_regular(&path.to_fs_path())
            .map_err(|error| OutlineError::Unreadable(error.to_string()))?;
        let stamp = Stamp::of(&metadata);
        if let Some(cache) = self.cache()
            && let Some(record) = cache.find(relative.as_str())
            && record.stamp == stamp
            && !record.flags.unsettled
            && !record.flags.large
        {
            if record.flags.binary {
                return Err(OutlineError::Binary);
            }
            return Ok(FileOutline {
                outline: cache.outline_of(record),
                size: stamp.size,
                covered: None,
            });
        }
        let cap = self.limits().max_file_bytes;
        let wanted = stamp.size.min(cap);
        let mut bytes = Vec::with_capacity(usize::try_from(wanted).unwrap_or(0));
        handle
            .take(wanted)
            .read_to_end(&mut bytes)
            .map_err(|error| OutlineError::Unreadable(error.to_string()))?;
        if is_binary(&bytes) {
            return Err(OutlineError::Binary);
        }
        // A cut inside a character is not a character.
        if stamp.size > cap {
            let valid = match std::str::from_utf8(&bytes) {
                Ok(_) => bytes.len(),
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                Err(_) => bytes.len(),
            };
            bytes.truncate(valid);
        }
        let name = path.file_name().unwrap_or("");
        let text = decode(&bytes);
        let outline = extract(Language::detect(name, &text), name, &text);
        Ok(FileOutline {
            outline,
            size: stamp.size,
            covered: (stamp.size > cap).then_some(wanted),
        })
    }
}
