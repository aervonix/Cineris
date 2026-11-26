//! An extension's folder, read whole into memory and hashed
//! (`docs/steps/28/answered.md`, "Trust pinning").
//!
//! The engine never evaluates a file from the disk. A runtime is made from
//! a [`Snapshot`]: every file of the folder read once, its hash compared
//! with the pinned one, and the modules served from it. A file changed
//! after the snapshot was taken is not seen until the next one, which is
//! checked again. So a file's time of change, which a command can set,
//! decides nothing about what runs.
//!
//! The folder holds regular files and directories only. A link of any
//! kind anywhere in it (a symbolic link, a Windows junction or any other
//! reparse point that leads somewhere) refuses the extension: a link could
//! lead out of the folder, or change where it leads without any file's
//! content changing.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "reading a folder blocks; callers run it on the blocking pool"
)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Read as _;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use cineris_base::{ContentHash, ContentHasher, in_code};
#[cfg(test)]
use cineris_base::{OpenError, open_regular};
use cineris_sandbox::unfollowed::{Folder, Kind, Refused};

/// The most files an extension's folder may hold.
pub const MAX_FILES: usize = 256;
/// The largest file an extension's folder may hold.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// The most bytes an extension's folder may hold in all.
pub const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024;
/// The deepest a file may be below the folder.
pub const MAX_DEPTH: usize = 16;
/// The most entries, folders and files together, an extension's folder
/// may hold: a folder of empty folders is bounded too (the review of step
/// 28).
pub const MAX_ENTRIES: usize = 1024;

/// A point of the read through handles: a folder of the extension's is
/// listed, and nothing in it opened yet. A test holds it to swap a folder
/// for a link between the look and the read, which the read by paths
/// followed and this one refuses.
pub const A_FOLDER_IS_LISTED: &str = "folder: a folder is listed, and nothing in it is opened yet";

/// What every snapshot's hash starts with, so that the hash of an
/// extension is never the hash of anything else the harness pins.
const DOMAIN: &[u8] = b"cineris extension folder, version 1\n";

/// Why a folder could not be read as an extension. Each name in it is
/// shown as code it cannot end (`cineris_base::in_code`): the names come
/// from whoever made the folder, a repository among them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FolderError {
    /// The folder, or something in it, could not be read.
    #[error("`{}` could not be read: {reason}", in_code(.path))]
    Unreadable {
        /// The path, relative to the folder (empty for the folder itself).
        path: String,
        /// What the system said.
        reason: String,
    },
    /// A link, a junction or another reparse point that leads somewhere.
    #[error(
        "`{}` is a link: an extension's folder holds only files and folders, so that nothing in it can lead out of it",
        in_code(.0)
    )]
    Link(String),
    /// Something that is neither a regular file nor a directory: a FIFO, a
    /// socket, a device.
    #[error("`{}` is not a regular file or a folder", in_code(.0))]
    NotRegular(String),
    /// A name that is not valid Unicode.
    #[error("a name in `{}` is not valid Unicode", in_code(.0))]
    Name(String),
    /// More files than [`MAX_FILES`].
    #[error("the folder holds more than {MAX_FILES} files")]
    TooManyFiles,
    /// A file larger than [`MAX_FILE_BYTES`].
    #[error("`{}` is larger than 1 MiB", in_code(.0))]
    FileTooLarge(String),
    /// More bytes in all than [`MAX_TOTAL_BYTES`].
    #[error("the folder holds more than 4 MiB")]
    TooLarge,
    /// A file deeper than [`MAX_DEPTH`].
    #[error("`{}` is more than {MAX_DEPTH} folders deep", in_code(.0))]
    TooDeep(String),
    /// More entries, folders and files together, than [`MAX_ENTRIES`].
    #[error("the folder holds more than {MAX_ENTRIES} files and folders")]
    TooManyEntries,
    /// A name Windows cannot hold, or holds as another: refused on every
    /// system, so that a folder read and written again is the same folder
    /// everywhere (since step 28's third pull request).
    #[error("`{}` is a name Windows cannot hold: {}", in_code(.0), .1)]
    Unportable(String, &'static str),
}

/// Every file of an extension's folder, as it was when it was read, and
/// the hash of all of them together.
#[derive(Clone, PartialEq, Eq)]
pub struct Snapshot {
    files: BTreeMap<String, Vec<u8>>,
    hash: ContentHash,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapshot")
            .field("files", &self.files.keys().collect::<Vec<_>>())
            .field("hash", &self.hash)
            .finish()
    }
}

impl Snapshot {
    /// Reads every file under `folder`, which is itself followed if it is a
    /// link (the user named it), and everything below it through handles
    /// that follow no link, as [`Snapshot::read_unfollowed`] reads it: what
    /// discovery and a runtime's start read an extension with (since step
    /// 28's third pull request, settled by the maintainers: it only narrows;
    /// before it, the names in the folder were looked at and then opened by
    /// their paths, so that a folder swapped for a link between the two was
    /// read through).
    ///
    /// It blocks: call it on the blocking pool.
    ///
    /// # Errors
    ///
    /// [`FolderError`] says what in the folder could not be taken.
    pub fn read(folder: &Path) -> Result<Self, FolderError> {
        Self::read_counted(folder).0
    }

    /// [`Snapshot::read`], and the bytes of files it read, a read refused
    /// part way included: what a bound on reading counts (the fresh review
    /// of step 28: a folder refused after its read was read for nothing).
    ///
    /// It blocks: call it on the blocking pool.
    pub fn read_counted(folder: &Path) -> (Result<Self, FolderError>, u64) {
        Self::read_through(Folder::open_following(folder))
    }

    /// The read by paths that step 28 shipped before its third pull request,
    /// kept for the measurement that compares the two
    /// (`perf_a_folder_read_through_handles_and_by_paths`).
    #[cfg(test)]
    fn read_by_paths(folder: &Path) -> (Result<Self, FolderError>, u64) {
        let mut total = 0_u64;
        let read = Self::read_into_total(folder, &mut total);
        (read, total)
    }

    #[cfg(test)]
    fn read_into_total(folder: &Path, total: &mut u64) -> Result<Self, FolderError> {
        let mut files = BTreeMap::new();
        let metadata = fs::metadata(folder).map_err(|error| FolderError::Unreadable {
            path: String::new(),
            reason: error.to_string(),
        })?;
        if !metadata.is_dir() {
            return Err(FolderError::Unreadable {
                path: String::new(),
                reason: "it is not a folder".to_owned(),
            });
        }
        let mut entries = 0;
        read_into(folder, "", 0, &mut files, total, &mut entries)?;
        Ok(Self::from_files(files))
    }

    /// [`Snapshot::read_counted`] through handles that follow no link
    /// (`cineris_sandbox::unfollowed`): the folder opened once, its last
    /// name not followed, and every name in it reached from the handle of
    /// the folder it is in, never by a path looked up again. A link at any
    /// name, the folder's own included, refuses the extension; a folder
    /// swapped for a link while it is read is refused where it is met, and
    /// nothing outside the folder is read through one. The caps are
    /// [`Snapshot::read`]'s. What `cineris ext add` reads an extension with,
    /// since the snapshot it shows is the one it writes and pins (since step
    /// 28's third pull request).
    ///
    /// It blocks: call it on the blocking pool.
    pub fn read_unfollowed(folder: &Path) -> (Result<Self, FolderError>, u64) {
        Self::read_through(Folder::open(folder))
    }

    /// [`Snapshot::read_counted`] of the folder `name` in the root `root`
    /// holds, opened relative to it and not followed, and everything below
    /// it through handles as [`Snapshot::read_unfollowed`] reads it: what
    /// discovery reads an extension with, from the root it listed, so that
    /// a folder above the root swapped for a link while the root is read
    /// sends no read through it (the narrow re-check of step 28's managing
    /// part).
    pub(crate) fn read_in(root: &HeldRoot, name: &OsStr) -> (Result<Self, FolderError>, u64) {
        Self::read_through(root.folder().folder(name))
    }

    /// Every file below the folder `opened`, through its handle.
    fn read_through(opened: Result<Folder, Refused>) -> (Result<Self, FolderError>, u64) {
        let mut total = 0_u64;
        let read = (|| {
            let mut root = opened.map_err(|refused| refusal(refused, ""))?;
            let mut files = BTreeMap::new();
            let mut entries = 0;
            read_unfollowed_into(&mut root, "", 0, &mut files, &mut total, &mut entries)?;
            Ok(Self::from_files(files))
        })();
        (read, total)
    }

    /// A snapshot of files given in memory, by their paths relative to the
    /// folder with `/` between names. For tests, and for what is read some
    /// other way. The caps are not applied here.
    #[must_use]
    pub fn from_files(files: BTreeMap<String, Vec<u8>>) -> Self {
        let mut hasher = ContentHasher::new();
        hasher.update(DOMAIN);
        for (path, content) in &files {
            hasher.update(&(path.len() as u64).to_le_bytes());
            hasher.update(path.as_bytes());
            hasher.update(&(content.len() as u64).to_le_bytes());
            hasher.update(content);
        }
        Self {
            files,
            hash: hasher.finish(),
        }
    }

    /// The hash of every file's path and content: what is pinned.
    #[must_use]
    pub fn hash(&self) -> ContentHash {
        self.hash
    }

    /// The file at `path`, relative to the folder with `/` between names.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// The files' paths, in order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// How many bytes the files hold in all.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.files
            .values()
            .map(|content| content.len() as u64)
            .sum()
    }
}

#[cfg(test)]
fn read_into(
    directory: &Path,
    prefix: &str,
    depth: usize,
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut u64,
    entries_seen: &mut usize,
) -> Result<(), FolderError> {
    let shown = |name: &str| {
        if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        }
    };
    let entries = fs::read_dir(directory).map_err(|error| FolderError::Unreadable {
        path: prefix.to_owned(),
        reason: error.to_string(),
    })?;
    let mut names: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| FolderError::Unreadable {
            path: prefix.to_owned(),
            reason: error.to_string(),
        })?;
        // Counted as it is read, so that a folder of a great many is not
        // taken whole first.
        *entries_seen += 1;
        if *entries_seen > MAX_ENTRIES {
            return Err(FolderError::TooManyEntries);
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| FolderError::Name(prefix.to_owned()))?;
        names.push((name, entry.path()));
    }
    names.sort();
    for (name, path) in names {
        let relative = shown(&name);
        // The entry itself, never what it leads to.
        let metadata = fs::symlink_metadata(&path).map_err(|error| FolderError::Unreadable {
            path: relative.clone(),
            reason: error.to_string(),
        })?;
        let kind = metadata.file_type();
        // On Windows a junction, and every link-like reparse point, is a
        // link here too.
        if kind.is_symlink() {
            return Err(FolderError::Link(relative));
        }
        if kind.is_dir() {
            if depth + 1 >= MAX_DEPTH {
                return Err(FolderError::TooDeep(relative));
            }
            read_into(&path, &relative, depth + 1, files, total, entries_seen)?;
        } else if kind.is_file() {
            if files.len() >= MAX_FILES {
                return Err(FolderError::TooManyFiles);
            }
            let content = read_file(&path, &relative, total)?;
            if *total > MAX_TOTAL_BYTES {
                return Err(FolderError::TooLarge);
            }
            files.insert(relative, content);
        } else {
            return Err(FolderError::NotRegular(relative));
        }
    }
    Ok(())
}

/// Why `name` cannot be one of a folder's names on Windows, or is another
/// name there: a write of the folder by its paths could not make it again.
/// `None` when it can.
#[must_use]
pub fn unportable(name: &str) -> Option<&'static str> {
    const DEVICES: [&str; 6] = ["con", "prn", "aux", "nul", "conin$", "conout$"];
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("Windows takes a dot or a space off the end of a name");
    }
    if name
        .chars()
        .any(|character| character.is_control() || "<>:\"|?*\\".contains(character))
    {
        return Some("it holds a character a Windows name cannot have");
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_lowercase();
    let numbered = base.chars().count() == 4
        && (base.starts_with("com") || base.starts_with("lpt"))
        && base
            .chars()
            .nth(3)
            .is_some_and(|last| last.is_ascii_digit() || "¹²³".contains(last));
    if DEVICES.contains(&base.as_str()) || numbered {
        return Some("it is the name of a device on Windows");
    }
    None
}

/// What a refused open of `relative` is as a folder's error.
fn refusal(refused: Refused, relative: &str) -> FolderError {
    match refused {
        Refused::Link => FolderError::Link(relative.to_owned()),
        Refused::NotAFolder | Refused::NotAFile => FolderError::NotRegular(relative.to_owned()),
        Refused::NotFound if relative.is_empty() => FolderError::Unreadable {
            path: String::new(),
            reason: "there is no folder there".to_owned(),
        },
        other => FolderError::Unreadable {
            path: relative.to_owned(),
            reason: other.to_string(),
        },
    }
}

/// A root of skills or of extensions, held open where it was resolved to,
/// so that what is listed in it and read from it is the folder that was
/// judged (the narrow re-check of step 28's managing part: a root opened
/// again by its path for each read followed a folder above it that had
/// been swapped for a link, into wherever the link led).
///
/// A root in the workspace is opened from the workspace down, by the names
/// of where it was resolved to, none followed: the workspace is the
/// person's, and a folder of it on the way that has become a link since is
/// refused. A root outside it (a user's, where their own link led) is
/// opened by its path, its own name not followed.
#[derive(Debug)]
pub(crate) struct HeldRoot {
    folder: Folder,
    /// Where it was resolved to, for the file system's calls.
    real: std::path::PathBuf,
    /// The workspace's real location.
    workspace: std::path::PathBuf,
    /// The folder's identity, when the system said it.
    identity: Option<cineris_base::FileIdentity>,
}

impl HeldRoot {
    /// Opens the root resolved to `real`, judged against the workspace
    /// whose real location is `workspace`.
    pub(crate) fn open(real: &Path, workspace: &Path) -> Result<Self, Refused> {
        let folder = match cineris_base::strip_location_prefix(real, workspace) {
            Some(below) => Folder::open_below(workspace, &below)?,
            None => Folder::open(real)?,
        };
        let identity = folder.identity().ok();
        Ok(Self {
            folder,
            real: real.to_path_buf(),
            workspace: workspace.to_path_buf(),
            identity,
        })
    }

    /// The folder held.
    pub(crate) fn folder(&self) -> &Folder {
        &self.folder
    }

    /// The folder held, to list.
    pub(crate) fn folder_mut(&mut self) -> &mut Folder {
        &mut self.folder
    }

    /// Whether the root's place still leads to the folder held, opened
    /// again as it was opened: `false` when it cannot be, or is another
    /// folder (or the system cannot say which). What was read from a root
    /// that has changed is not taken, and that is said.
    pub(crate) fn unchanged(&self) -> bool {
        let Some(held) = self.identity else {
            return false;
        };
        Self::open(&self.real, &self.workspace)
            .ok()
            .and_then(|again| again.identity)
            == Some(held)
    }
}

/// [`read_into`] through the handle of `folder`: each name opened relative
/// to it, none followed when it is a link.
fn read_unfollowed_into(
    folder: &mut Folder,
    prefix: &str,
    depth: usize,
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut u64,
    entries_seen: &mut usize,
) -> Result<(), FolderError> {
    // At most one past what the folder may hold: a folder of a great many
    // names is refused without its whole listing taken (the early review of
    // step 28's third pull request).
    let listed = folder
        .entries(MAX_ENTRIES.saturating_sub(*entries_seen))
        .map_err(|error| FolderError::Unreadable {
            path: prefix.to_owned(),
            reason: error.to_string(),
        })?;
    let mut names: Vec<(String, Kind)> = Vec::new();
    for entry in listed {
        *entries_seen += 1;
        if *entries_seen > MAX_ENTRIES {
            return Err(FolderError::TooManyEntries);
        }
        let name = entry
            .name
            .into_string()
            .map_err(|_| FolderError::Name(prefix.to_owned()))?;
        if let Some(why) = unportable(&name) {
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            return Err(FolderError::Unportable(relative, why));
        }
        names.push((name, entry.kind));
    }
    names.sort();
    cineris_base::hook::point(A_FOLDER_IS_LISTED);
    for (name, kind) in names {
        let relative = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        match kind {
            Kind::Link => return Err(FolderError::Link(relative)),
            Kind::Other => return Err(FolderError::NotRegular(relative)),
            Kind::Folder => {
                if depth + 1 >= MAX_DEPTH {
                    return Err(FolderError::TooDeep(relative));
                }
                let mut inner = folder
                    .folder(OsStr::new(&name))
                    .map_err(|refused| refusal(refused, &relative))?;
                read_unfollowed_into(&mut inner, &relative, depth + 1, files, total, entries_seen)?;
            }
            Kind::File => {
                if files.len() >= MAX_FILES {
                    return Err(FolderError::TooManyFiles);
                }
                let (file, metadata) = folder
                    .file(OsStr::new(&name))
                    .map_err(|refused| refusal(refused, &relative))?;
                let content = read_open(file, &metadata, &relative, total)?;
                if *total > MAX_TOTAL_BYTES {
                    return Err(FolderError::TooLarge);
                }
                files.insert(relative, content);
            }
        }
    }
    Ok(())
}

/// One file, opened without following a link, refused by its length before
/// it is read when that is past its cap or the folder's, and otherwise read
/// to at most one byte past its cap. Every byte read is added to `total`, a
/// read that fails, or finds the file grown past its cap, included (the
/// re-check of step 28's fresh review: a file over 1 MiB was read whole and
/// not counted).
#[cfg(test)]
fn read_file(path: &Path, relative: &str, total: &mut u64) -> Result<Vec<u8>, FolderError> {
    let (file, metadata) = open_regular(path).map_err(|error| match error {
        OpenError::Link => FolderError::Link(relative.to_owned()),
        OpenError::NotAFile | OpenError::Directory => FolderError::NotRegular(relative.to_owned()),
        other => FolderError::Unreadable {
            path: relative.to_owned(),
            reason: other.to_string(),
        },
    })?;
    read_open(file, &metadata, relative, total)
}

/// An open regular file's content, as [`read_file`] reads it: refused by its
/// length first, and every byte read counted.
fn read_open(
    file: fs::File,
    metadata: &fs::Metadata,
    relative: &str,
    total: &mut u64,
) -> Result<Vec<u8>, FolderError> {
    if metadata.len() > MAX_FILE_BYTES {
        return Err(FolderError::FileTooLarge(relative.to_owned()));
    }
    if total.saturating_add(metadata.len()) > MAX_TOTAL_BYTES {
        return Err(FolderError::TooLarge);
    }
    let mut content = Vec::new();
    let read = file.take(MAX_FILE_BYTES + 1).read_to_end(&mut content);
    *total += content.len() as u64;
    read.map_err(|error| FolderError::Unreadable {
        path: relative.to_owned(),
        reason: error.to_string(),
    })?;
    if content.len() as u64 > MAX_FILE_BYTES {
        return Err(FolderError::FileTooLarge(relative.to_owned()));
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(path, content)| ((*path).to_owned(), content.as_bytes().to_vec()))
            .collect()
    }

    /// What a read reports it read: a file past its cap, or one that would
    /// take the folder past its own, is refused by its length and not read;
    /// what was read before a refusal counts (the re-check of step 28's
    /// fresh review).
    #[test]
    fn a_read_reports_every_byte_it_read_and_reads_no_file_past_a_cap() {
        let dir = std::env::temp_dir().join(format!("cineris-ext-counted-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("large")).unwrap();
        fs::write(dir.join("large/big.bin"), vec![b'x'; 2 * 1024 * 1024]).unwrap();
        let (read, bytes) = Snapshot::read_counted(&dir.join("large"));
        assert_eq!(read, Err(FolderError::FileTooLarge("big.bin".to_owned())));
        assert_eq!(bytes, 0, "refused by its length, unread");
        // Five files of 1 MiB: four are read, and the fifth is refused
        // before it is.
        fs::create_dir_all(dir.join("many")).unwrap();
        for file in 0..5 {
            fs::write(
                dir.join(format!("many/{file}.bin")),
                vec![b'x'; usize::try_from(MAX_FILE_BYTES).unwrap()],
            )
            .unwrap();
        }
        let (read, bytes) = Snapshot::read_counted(&dir.join("many"));
        assert_eq!(read, Err(FolderError::TooLarge));
        assert_eq!(bytes, MAX_TOTAL_BYTES);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The read through handles takes what the plain read takes, with its
    /// hash and its caps, and refuses a link at any name, the folder's own
    /// included, where the plain read follows the folder's (step 28's `ext
    /// add`).
    #[test]
    fn the_read_through_handles_is_the_plain_reads_and_follows_no_link() {
        let dir =
            std::env::temp_dir().join(format!("cineris-ext-unfollowed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ext = dir.join("ext");
        fs::create_dir_all(ext.join("lib/deep")).unwrap();
        fs::create_dir_all(ext.join("empty")).unwrap();
        fs::write(ext.join("extension.toml"), "x").unwrap();
        fs::write(ext.join("lib/a.js"), "a").unwrap();
        fs::write(ext.join("lib/deep/b.js"), "bb").unwrap();
        let (plain, plain_bytes) = Snapshot::read_by_paths(&ext);
        let (unfollowed, bytes) = Snapshot::read_unfollowed(&ext);
        assert_eq!(unfollowed, plain);
        assert_eq!(bytes, plain_bytes);
        assert_eq!(
            unfollowed.unwrap().paths().collect::<Vec<_>>(),
            ["extension.toml", "lib/a.js", "lib/deep/b.js"]
        );
        fs::write(ext.join("lib/big.bin"), vec![b'x'; 2 * 1024 * 1024]).unwrap();
        let (read, bytes) = Snapshot::read_unfollowed(&ext);
        assert_eq!(
            read,
            Err(FolderError::FileTooLarge("lib/big.bin".to_owned()))
        );
        assert!(bytes < 1024, "refused by its length, unread: {bytes}");
        fs::remove_file(ext.join("lib/big.bin")).unwrap();
        assert_eq!(
            Snapshot::read_unfollowed(&dir.join("none")).0,
            Err(FolderError::Unreadable {
                path: String::new(),
                reason: "there is no folder there".to_owned()
            })
        );
        #[cfg(unix)]
        {
            let outside = dir.join("outside");
            fs::create_dir_all(&outside).unwrap();
            fs::write(outside.join("secret"), "s").unwrap();
            std::os::unix::fs::symlink(&outside, ext.join("lib/deep/out")).unwrap();
            assert_eq!(
                Snapshot::read_unfollowed(&ext).0,
                Err(FolderError::Link("lib/deep/out".to_owned()))
            );
            fs::remove_file(ext.join("lib/deep/out")).unwrap();
            std::os::unix::fs::symlink(outside.join("secret"), ext.join("secret.js")).unwrap();
            assert_eq!(
                Snapshot::read_unfollowed(&ext).0,
                Err(FolderError::Link("secret.js".to_owned()))
            );
            fs::remove_file(ext.join("secret.js")).unwrap();
            std::os::unix::fs::symlink(&ext, dir.join("named")).unwrap();
            assert_eq!(
                Snapshot::read_unfollowed(&dir.join("named")).0,
                Err(FolderError::Link(String::new()))
            );
            assert!(
                Snapshot::read(&dir.join("named")).is_ok(),
                "the plain read follows the folder itself, the user's to name"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// A folder swapped for a link between the look at it and its read is
    /// refused where it is met: the read by paths, which discovery and a
    /// runtime's start used before step 28's third pull request, read
    /// through it.
    #[test]
    fn a_folder_swapped_for_a_link_after_its_look_is_refused() {
        let dir = std::env::temp_dir().join(format!("cineris-ext-swapped-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ext = dir.join("ext");
        fs::create_dir_all(ext.join("lib")).unwrap();
        fs::write(ext.join("lib/a.js"), "a").unwrap();
        fs::create_dir_all(dir.join("outside")).unwrap();
        fs::write(dir.join("outside/secret"), "s").unwrap();
        let listed = cineris_base::hook::Hold::new(A_FOLDER_IS_LISTED);
        let reading = {
            let ext = ext.clone();
            std::thread::spawn(move || Snapshot::read(&ext))
        };
        assert!(listed.wait_reached(std::time::Duration::from_secs(10)));
        fs::remove_dir_all(ext.join("lib")).unwrap();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(dir.join("outside"), ext.join("lib"));
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(dir.join("outside"), ext.join("lib"));
        listed.release();
        let read = reading.join().unwrap();
        match made {
            Ok(()) => assert_eq!(read, Err(FolderError::Link("lib".to_owned()))),
            Err(error) => assert!(
                std::env::var_os("GITHUB_ACTIONS").is_none(),
                "the CI runner cannot make a symbolic link: {error}"
            ),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// A name Windows cannot hold, or holds as another, is refused in a
    /// snapshot on every system: a trailing dot or space, a device's name
    /// at any depth, a character a Windows name cannot have.
    #[test]
    fn a_name_windows_cannot_hold_is_refused_everywhere() {
        for fine in ["main.js", "console", "com10", "lpt", "a b", "Ünïcode"] {
            assert_eq!(unportable(fine), None, "{fine}");
        }
        for refused in [
            "x.",
            "x ",
            "con",
            "CON.txt",
            "nul",
            "aux.js",
            "com1",
            "LPT9",
            "conout$",
            "CONIN$.x",
            "com\u{b9}",
            "a:b",
            "a?",
            "a*",
            "a|b",
            "a\"b",
            "a<b",
            "a\u{1}b",
        ] {
            assert!(unportable(refused).is_some(), "{refused:?}");
        }
        #[cfg(unix)]
        {
            let dir =
                std::env::temp_dir().join(format!("cineris-ext-unportable-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("ext/lib")).unwrap();
            fs::write(dir.join("ext/lib/con"), "x").unwrap();
            assert!(matches!(
                Snapshot::read(&dir.join("ext")),
                Err(FolderError::Unportable(path, _)) if path == "lib/con"
            ));
            let _ = fs::remove_dir_all(&dir);
        }
    }

    /// What the read through handles costs against the read by paths it
    /// replaced, on a typical extension's folder: a manifest, a script of 3
    /// KiB, a README of 1 KiB and a folder of two modules. A release build's
    /// medians of 400: `cargo xtask perf -p cineris-ext`.
    #[test]
    #[ignore = "a measurement: `cargo xtask perf -p cineris-ext`"]
    fn perf_a_folder_read_through_handles_and_by_paths() {
        let dir =
            std::env::temp_dir().join(format!("cineris-ext-perf-read-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ext = dir.join("tracker");
        fs::create_dir_all(ext.join("lib")).unwrap();
        fs::write(
            ext.join("extension.toml"),
            "api = 1\nname = \"tracker\"\nversion = \"1\"\nmain = \"main.js\"\n",
        )
        .unwrap();
        fs::write(ext.join("main.js"), "x".repeat(3 * 1024)).unwrap();
        fs::write(ext.join("README.md"), "y".repeat(1024)).unwrap();
        fs::write(ext.join("lib/a.js"), "a".repeat(512)).unwrap();
        fs::write(ext.join("lib/b.js"), "b".repeat(512)).unwrap();
        let median = |read: &dyn Fn() -> Result<Snapshot, FolderError>| {
            let mut times: Vec<f64> = (0..400)
                .map(|_| {
                    let started = std::time::Instant::now();
                    std::hint::black_box(read().unwrap());
                    started.elapsed().as_secs_f64() * 1e6
                })
                .collect();
            times.sort_by(f64::total_cmp);
            times[times.len() / 2]
        };
        let by_paths = median(&|| Snapshot::read_by_paths(&ext).0);
        let unfollowed = median(&|| Snapshot::read_unfollowed(&ext).0);
        cineris_testkit::numbers::report("folder.read.by_paths", by_paths, "us");
        cineris_testkit::numbers::report("folder.read.unfollowed", unfollowed, "us");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hash_changes_with_any_path_or_content_and_not_with_order() {
        let base = Snapshot::from_files(files(&[("main.js", "a"), ("lib/x.js", "b")]));
        let same = Snapshot::from_files(files(&[("lib/x.js", "b"), ("main.js", "a")]));
        assert_eq!(base.hash(), same.hash());
        for changed in [
            files(&[("main.js", "a "), ("lib/x.js", "b")]),
            files(&[("main.js", "a"), ("lib/y.js", "b")]),
            files(&[("main.js", "a"), ("lib/x.js", "b"), ("extra", "")]),
            // A character moved from one file's content to the next path.
            files(&[("main.js", ""), ("alib/x.js", "b")]),
        ] {
            assert_ne!(base.hash(), Snapshot::from_files(changed).hash());
        }
    }
}
