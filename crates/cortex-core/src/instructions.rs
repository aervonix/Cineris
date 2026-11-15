//! Reading instruction files for the loop
//! (`docs/architecture/07-sessions-and-context.md`, "Project instruction
//! files"; `docs/security.md`, section 2).
//!
//! The loop asks for a file by its absolute path through its
//! `InstructionFiles` seam, and this is the core's answer. An instruction
//! file is text the model reads, and the text goes to the provider with
//! every request, so what is read is held to what the agent itself could
//! read unasked:
//!
//! - **A file in the workspace** is found through the session's path
//!   resolver: a link is followed only where it stays inside the workspace
//!   (a `CLAUDE.md` that links to `AGENTS.md` is read; one that points at a
//!   private key outside is not, and the user is warned, by the file's
//!   name), and the permission engine must let the agent read the file
//!   without asking (`Engine::may_store`). A file a rule denies, or one
//!   that looks like a secret, is not read.
//! - **The user's own file** in the configuration directory is the user's,
//!   and is read as it is: a link there, as a dotfile manager makes one
//!   (GNU Stow, home-manager, chezmoi's symlink mode), is followed to the
//!   file it names. One that leads nowhere is warned of. Since the review
//!   of step 15, which found such a file left out without a word.
//! - **Only a regular file** is opened, without waiting on a FIFO, and no
//!   more is read than the loop asks for.
//!
//! **Links on Windows.** A symbolic link to a file is followed in the
//! configuration directory as on Linux, and held to the workspace there.
//! A junction links only directories, so it is never an instruction file
//! itself; on the way to one it is followed as the resolver follows any
//! directory. A shortcut (`.lnk`) is a file of the shell's, not a link of
//! the file system: an `AGENTS.md.lnk` is another name, and is not read. A
//! hard link is the file itself, on either system.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the loop calls this on the blocking pool"
)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cortex_agent::{FileText, InstructionFiles};
use cortex_base::{AbsPath, OpenError, PathResolver, open_regular};
use cortex_session::KeepGate;

/// The core's instruction files.
pub(crate) struct DiskInstructions {
    pub(crate) resolver: Arc<PathResolver>,
    pub(crate) user_dir: Option<AbsPath>,
    pub(crate) gate: Arc<dyn KeepGate>,
}

impl std::fmt::Debug for DiskInstructions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskInstructions")
            .field("root", self.resolver.root())
            .finish_non_exhaustive()
    }
}

impl InstructionFiles for DiskInstructions {
    fn read(&self, path: &AbsPath, limit: usize) -> Result<Option<FileText>, String> {
        let users = self
            .user_dir
            .as_ref()
            .is_some_and(|dir| path.parent().as_ref() == Some(dir));
        let real = if users {
            // In the configuration directory, only the user's own
            // instruction files: never the harness's files beside them.
            let name = path.file_name().unwrap_or_default();
            if !cortex_agent::context::DEFAULT_NAMES.contains(&name) {
                return Ok(None);
            }
            match users_own(&path.to_fs_path())? {
                Some(real) => real,
                None => return Ok(None),
            }
        } else {
            let root = self.resolver.root();
            let Some(relative) = path.strip_prefix(root) else {
                return Ok(None);
            };
            let Ok(resolved) = self.resolver.resolve(relative.as_str()) else {
                return Ok(None);
            };
            if !resolved.exists() {
                return Ok(None);
            }
            // The file's name is in the workspace, so only a link takes it
            // outside. What it leads to is not the project's, and is not
            // read: the user is told, by the file's name.
            if !resolved.is_inside_workspace() {
                return Err(
                    "it is a link to a file outside the workspace, and a project's instruction \
                     files are read only from inside it"
                        .to_owned(),
                );
            }
            if !self.gate.may_keep(resolved.abs()) {
                return Ok(None);
            }
            resolved.to_fs_path()
        };
        let (file, metadata) = match open_regular(&real) {
            Ok(opened) => opened,
            Err(
                OpenError::NotFound | OpenError::Link | OpenError::Directory | OpenError::NotAFile,
            ) => {
                return Ok(None);
            }
            Err(error) => return Err(error.to_string()),
        };
        let mut bytes = Vec::new();
        file.take(limit as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        Ok(Some(FileText {
            bytes,
            len: metadata.len(),
        }))
    }
}

/// Where the user's own instruction file at `path` is: `path` itself, or
/// for a link the file it leads to, which is then opened as a regular file.
/// `None` when nothing has the name.
///
/// # Errors
///
/// A link that leads nowhere, or cannot be followed, in words.
fn users_own(path: &Path) -> Result<Option<PathBuf>, String> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if !metadata.file_type().is_symlink() => return Ok(Some(path.to_owned())),
        Ok(_) => {}
    }
    match std::fs::canonicalize(path) {
        Ok(target) => Ok(Some(target)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err("it is a link to a file that is not there".to_owned())
        }
        Err(error) => Err(format!("it is a link that cannot be followed: {error}")),
    }
}
