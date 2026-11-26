//! Files and directories that only the current user can open
//! (`docs/decisions/0029-owner-only-files-on-windows.md`).
//!
//! Session logs, credential files and state files are private to the user.
//! On Linux that is a mode: `0600` for a file, `0700` for a directory. On
//! Windows a new file takes its folder's access rules, so privacy there is
//! an access list set on the file itself: one entry, which allows the
//! current user, with inheritance from the folder turned off.
//!
//! | Function | What it does |
//! |---|---|
//! | [`create_new`] | Creates a new, empty file that is private from the moment it exists, and returns it open for writing. |
//! | [`create_dir_all`] | Creates a directory, and any missing parents, each one private. |
//! | [`is_private`] | Whether only the current user has access to a file or a directory. |
//! | [`hold_lock`] | Takes the lock that keeps two writers of a stored file apart: the operating system's lock on a private file beside it. |
//! | [`lock_within`] | The same lock's bounded wait, for a store that opens its lock file itself. |
//! | [`access_list`] | Windows only: the access list, read back. |
//! | [`current_user_sid`] | Windows only: the security identifier of the user the process runs as, in text. |
//!
//! [`write_atomic`](crate::write_atomic) with
//! [`owner_only`](crate::AtomicWriteOptions::owner_only) is the third way to
//! make a private file, and uses [`create_new`] for it.
//!
//! # A private file is made private, never left to its folder
//!
//! Every private file goes through [`create_new`], or through
//! [`write_atomic`](crate::write_atomic) with `owner_only`. Do not rely on
//! the folder:
//!
//! - [`create_dir_all`] leaves a directory that already exists exactly as it
//!   is. The user may have chosen its permissions, and they are not changed
//!   behind the user's back. So a file created by an ordinary call inside
//!   "the private directory" may be inside a directory that was never
//!   private.
//! - On Windows a directory this module creates does hand its access list
//!   down to what is created inside it later. That is a second line of
//!   defence, not the first: a file that inherits is not private by
//!   [`is_private`]'s definition, which asks for inheritance to be off.
//!
//! # The access list is there before the file is
//!
//! The mode, or the access list, is given to the call that creates the file.
//! There is no moment at which the file exists with wider access. Setting it
//! afterwards would leave such a moment, and a program that opened the file
//! in it would keep its handle after the rules changed.
//!
//! # Who is on the list
//!
//! The current user and nobody else, which is what `0600` means. Not the
//! SYSTEM account and not the Administrators group: both can reach any file
//! regardless (an administrator can take ownership, and backup software
//! reads with a privilege that passes access lists by), so leaving them off
//! costs nothing that works and keeps the list equal to the Linux mode. The
//! file's owner is set to the current user as well. Without that, a file
//! created from an elevated prompt would belong to the Administrators group.
//!
//! # Where the promise cannot be kept
//!
//! A file system with no permissions of its own (FAT and exFAT, some network
//! and virtual shares) takes the file and drops the mode or the access list
//! without an error. These functions do not refuse to write there. What
//! they offer instead is [`is_private`], which then says no or fails, for
//! the caller to report.
//!
//! These functions block. Async code calls them on the blocking pool.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "this is a blocking file primitive; async callers run it on the blocking pool"
)]

use std::fs::{File, TryLockError};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    AccessEntry, AccessKind, AccessList, FULL_CONTROL, access_list, current_user_sid,
};

#[cfg(windows)]
pub(crate) use windows::{attributes, file_identity, on_a_fixed_disk, reparse_tag};

#[cfg(not(any(unix, windows)))]
compile_error!("owner-only files are implemented for Unix and for Windows");

/// Creates a new, empty file that only the current user can open, and
/// returns it open for writing.
///
/// The file is private from the moment it exists. The directory must exist.
/// To append to the file later, open it again: it keeps its mode or its
/// access list.
///
/// # Errors
///
/// [`io::ErrorKind::AlreadyExists`] when something has the name already,
/// including a link: nothing is followed and nothing is replaced. Otherwise
/// what the operating system reports.
pub fn create_new(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // The process's umask can only take bits away from this.
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(windows)]
    {
        windows::create_new(path)
    }
}

/// Creates a directory, and every parent that is missing, each one open to
/// the current user only.
///
/// **A directory that already exists is left as it is,** `path` itself
/// included: its permissions are the user's choice. Ask [`is_private`] when
/// it matters, and see the module's note on why a private file never relies
/// on its folder.
///
/// # Errors
///
/// When a name on the way is something other than a directory, or the
/// operating system refuses.
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        // Recursive creation gives the mode to every directory it makes and
        // touches none that exist.
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(windows)]
    {
        // Find what is missing, outermost last. A name's attributes answer
        // without opening it where it is no reparse point, as the path
        // resolver's walk asks (`windows::attributes`); anything else is
        // asked of `metadata`, which follows a link, as before.
        let mut missing = Vec::new();
        let mut current = path;
        loop {
            let attributes =
                windows::on_a_fixed_disk(current).then(|| windows::attributes(current));
            let looked = match attributes {
                Some(Ok(attributes)) if !attributes.is_reparse_point() => {
                    if attributes.is_directory() {
                        break;
                    }
                    return Err(io::ErrorKind::NotADirectory.into());
                }
                Some(Err(error)) if error.kind() == io::ErrorKind::NotFound => Err(error),
                _ => std::fs::metadata(current),
            };
            match looked {
                Ok(found) if found.is_dir() => break,
                Ok(_) => return Err(io::ErrorKind::NotADirectory.into()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    missing.push(current);
                    match current.parent() {
                        // A relative path with one name left: its parent is
                        // the current directory, which exists.
                        Some(parent) if parent.as_os_str().is_empty() => break,
                        Some(parent) => current = parent,
                        None => return Err(error),
                    }
                }
                Err(error) => return Err(error),
            }
        }
        for directory in missing.into_iter().rev() {
            match windows::create_dir(directory) {
                Ok(()) => {}
                // Another process made it between the look and the call.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if !std::fs::metadata(directory)?.is_dir() {
                        return Err(io::ErrorKind::NotADirectory.into());
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

/// Whether only the current user has access to the file or directory at
/// `path`. A link is followed: the answer is about what it points to.
///
/// - **Linux:** no permission bit is set for the group or for others.
/// - **Windows:** the owner is the current user, inheritance from the folder
///   is off, and the access list has at least one entry, every one of which
///   allows the current user. A deny entry, an entry for anyone else, an
///   inherited entry, or no access list at all (which Windows reads as
///   "everyone") each make the answer no.
///
/// This is a report, not a guard: the answer can be out of date as soon as
/// it is given.
///
/// # Errors
///
/// When the path cannot be looked at.
pub fn is_private(path: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(std::fs::metadata(path)?.permissions().mode() & 0o077 == 0)
    }
    #[cfg(windows)]
    {
        windows::access_list(path).map(|list| list.is_owner_only())
    }
}

/// The lock file of the stored file at `file`: its name with `.lock`
/// added (`trust.toml.lock`).
#[must_use]
pub fn lock_path(file: &Path) -> std::path::PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".lock");
    std::path::PathBuf::from(name)
}

/// Takes the lock that keeps two writers of the stored file at `file`
/// apart, for a change that reads the file, changes it and writes it whole:
/// held from before the read until after the write, so that a second writer
/// cannot write back what it read before the first one's change.
///
/// - **The lock is a file beside the store** ([`lock_path`]), created
///   owner-only and opened without following a link, with the store's
///   directory made owner-only if it is missing.
/// - **It is the operating system's lock on an open file** (`flock` on
///   Linux, `LockFileEx` on Windows, which keep two opens apart in one
///   process too), so it goes with its holder however that ends, and there
///   is no stale lock to clear. It is held until the returned file is
///   dropped.
/// - **It waits [`LOCK_WAIT`] at most** while another holds it, and is then
///   refused, saying which file: a hold that does not end (a session's
///   command holding a store's lock) cannot hang a person's yes (the full
///   review of step 28's third pull request).
/// - **A file system that says it cannot lock gives no lock** (`Ok(None)`),
///   and the change goes ahead: two writers are then the user's to keep
///   apart, as they were before there was a lock.
/// - **Readers take no lock:** a store replaced in one step is read whole,
///   old or new.
///
/// # Errors
///
/// When the directory or the lock file cannot be made or opened, a link
/// stands at the lock file's name, another process holds the lock past
/// [`LOCK_WAIT`], or the lock fails for any reason but the file system's
/// not having locks.
pub fn hold_lock(file: &Path) -> io::Result<Option<File>> {
    hold_lock_within(file, LOCK_WAIT)
}

/// How long [`hold_lock`] waits for another process's hold of the lock:
/// the store's change it is for is one read and one write of a small file.
pub const LOCK_WAIT: Duration = Duration::from_secs(10);

/// How often a held lock is tried again while it is waited for.
const LOCK_RETRY: Duration = Duration::from_millis(20);

/// [`hold_lock`], waiting `wait` at most for another process's hold.
///
/// # Errors
///
/// As [`hold_lock`]'s, the lock held past `wait` among them: its error's
/// kind is [`io::ErrorKind::WouldBlock`], and it names the file, under `~`
/// where it is in the home directory.
pub fn hold_lock_within(file: &Path, wait: Duration) -> io::Result<Option<File>> {
    if let Some(directory) = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_dir_all(directory)?;
    }
    let path = lock_path(file);
    let refused = |error: crate::OpenError| match error {
        crate::OpenError::Io(source) => source,
        other => io::Error::other(format!("the lock file {}: {other}", path.display())),
    };
    let lock = match crate::open_regular(&path) {
        Ok((lock, _)) => lock,
        Err(crate::OpenError::NotFound) => {
            match create_new(&path) {
                Ok(_) => {}
                // Another process made it first, or something else did: the
                // open says which. A link there is not followed.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            crate::open_regular(&path)
                .map(|(lock, _)| lock)
                .map_err(refused)?
        }
        Err(error) => return Err(refused(error)),
    };
    match lock_within(&lock, file, wait) {
        Ok(()) => Ok(Some(lock)),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(None),
        Err(error) => Err(error),
    }
}

/// Takes the operating system's lock on `lock`, the lock file of the
/// stored file `file`, waiting `wait` at most for another process's hold:
/// the wait of [`hold_lock`], for a store that opens its lock file itself
/// (the credentials file's, the narrow re-check of step 28's third pull
/// request: `cineris auth login` waited without end).
///
/// # Errors
///
/// The lock held past `wait`, with the kind [`io::ErrorKind::WouldBlock`]
/// and a message that names `file`, under `~` where it is in the home
/// directory; a file system that cannot lock, with the kind
/// [`io::ErrorKind::Unsupported`]; and any other failure of the lock.
pub fn lock_within(lock: &File, file: &Path, wait: Duration) -> io::Result<()> {
    let deadline = Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                #[allow(
                    clippy::disallowed_methods,
                    reason = "a lock held by another process gives no event to wait on; the \
                              wait is bounded by `wait`"
                )]
                std::thread::sleep(LOCK_RETRY);
            }
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    held_too_long(file, wait),
                ));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
}

/// What a lock held past its wait says: the store's file, under `~` where
/// it is in the home directory.
fn held_too_long(file: &Path, wait: Duration) -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from);
    format!(
        "{} is being changed by another process, which held its lock for longer than {} s: \
         nothing was changed. If a session's command holds it, end that command, then try again",
        under_home(file, home.as_deref()),
        wait.as_secs()
    )
}

/// `file` as it is shown: under `~` where it is in `home`.
fn under_home(file: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| crate::strip_location_prefix(file, home)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => file.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lock held by another open is waited for no longer than asked, and
    /// then refused, saying which file; once let go of, it is taken (the
    /// full review of step 28's third pull request: an endless wait hung a
    /// person's yes).
    #[test]
    fn a_held_lock_is_waited_for_no_longer_than_asked() {
        let folder = std::env::temp_dir().join(format!(
            "cineris-base-lock-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let file = folder.join("pins.toml");
        let held = hold_lock(&file).unwrap().expect("this file system locks");
        let error = hold_lock_within(&file, Duration::ZERO).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock, "{error}");
        let said = error.to_string();
        assert!(said.contains("pins.toml") && said.contains("0 s"), "{said}");
        drop(held);
        assert!(hold_lock_within(&file, Duration::ZERO).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A file in the home directory is shown under `~`, any other as it is.
    #[test]
    fn a_file_in_the_home_directory_is_shown_under_the_tilde() {
        let home = std::env::temp_dir().join("home");
        let file = home.join(".local").join("pins.toml");
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            under_home(&file, Some(&home)),
            format!("~{sep}.local{sep}pins.toml")
        );
        assert_eq!(under_home(&home, Some(&home)), "~");
        assert_eq!(under_home(&file, None), file.display().to_string());
        let elsewhere = std::env::temp_dir().join("other").join("pins.toml");
        assert_eq!(
            under_home(&elsewhere, Some(&home)),
            elsewhere.display().to_string()
        );
    }
}
