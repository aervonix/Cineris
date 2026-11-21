//! Putting protected paths back as the look before a confined start found
//! them (the third security fix; `docs/architecture/06-permissions-and-sandbox.md`,
//! "Protected paths after each command").
//!
//! A confined command, a project's check or an extension's program may
//! change a protected path: the look after it finds the change, and the
//! change is put back at once, from the look before's own record, so that
//! nothing a confined start wrote there runs later outside the sandbox (the
//! model's own `git status` at `unrestricted`, a hook on the next `git
//! commit`, the user's own tools after the session).
//!
//! **Nothing is written through a link.** The directory a path lies in is
//! opened with no symbolic link followed anywhere on its way
//! (`openat2(RESOLVE_NO_SYMLINKS)`), and every change is made relative to
//! that open directory, so that a link swapped in after the check cannot
//! carry it elsewhere:
//!
//! - a path that was not there before is removed: a link at it is removed
//!   itself, never what it leads to, and a directory only when it is empty;
//! - a directory that went is made again, with its mode;
//! - a file whose content was kept is written under a name of its own
//!   beside it, made with no link followed, and renamed into place, so that
//!   whatever is at the name (a link, another file) is replaced, never
//!   written through; its mode is put back with it;
//! - a file whose content was not kept (too large) has its mode put back,
//!   and nothing else;
//! - a directory on the way that is a link now, or is gone, refuses the
//!   path, which is left as it is.
//!
//! Each path is put back on its own, and a failure leaves that path alone.
//! What is left the caller finds by looking again ([`super::Watch::put_back`]),
//! and names as a path to put back by hand. Only Linux puts anything back:
//! protected paths are looked at only where the sandbox enforces.

use super::{Changed, Snapshot};

/// Puts back each path of `changed` as `before` holds it, as far as it
/// safely can. It blocks.
pub(super) fn put_back(before: &Snapshot, changed: &[Changed]) {
    #[cfg(target_os = "linux")]
    linux::put_back(before, changed);
    #[cfg(not(target_os = "linux"))]
    let _ = (before, changed);
}

#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "decision 0030: a change made relative to a directory opened with no link followed \
              is only reachable through system calls; cineris-sandbox is where such calls live"
)]
mod linux {
    use std::cmp::Reverse;
    use std::ffi::CString;
    use std::io::{self, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use super::super::{Changed, Entry, Kind, Snapshot};

    /// What is at a name now, not following a link.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Now {
        Missing,
        Directory,
        File,
        /// A link, a special file.
        Other,
    }

    pub(super) fn put_back(before: &Snapshot, changed: &[Changed]) {
        let mut paths: Vec<&Path> = changed.iter().map(|one| one.path.as_path()).collect();
        // What is in the way goes first, deepest first, so that what is
        // under a path goes before the path; then what was there is made
        // again, shallowest first, so that a directory is there before
        // what goes in it.
        paths.sort_by_key(|path| Reverse(path.components().count()));
        for path in &paths {
            let _ = clear(path, before.entries.get(*path));
        }
        paths.reverse();
        for path in &paths {
            if let Some(old) = before.entries.get(*path) {
                let _ = restore(path, old);
            }
        }
    }

    /// Whether `old` can be made again: nothing (the path was not there), a
    /// directory, or a file whose content was kept. Anything else is left
    /// as it is now: removing it would lose what is there and put nothing
    /// back.
    fn can_be_made_again(old: Option<&Entry>) -> bool {
        old.is_none_or(|old| {
            old.kind == Kind::Directory || (old.kind == Kind::File && old.content.is_some())
        })
    }

    /// Removes what is at `path` now where it is in the way of what was
    /// there before: anything, where nothing was; something of another kind,
    /// where something was.
    fn clear(path: &Path, old: Option<&Entry>) -> io::Result<()> {
        if !can_be_made_again(old) {
            return Ok(());
        }
        let (dir, name) = parent_of(path)?;
        let now = kind_at(&dir, &name)?;
        let in_the_way = match (old.map(|old| old.kind), now) {
            (_, Now::Missing)
            | (Some(Kind::Directory), Now::Directory)
            // A file is replaced by the rename that puts it back.
            | (Some(Kind::File), Now::File) => false,
            _ => true,
        };
        if in_the_way {
            unlink_at(&dir, &name, now == Now::Directory)?;
        }
        Ok(())
    }

    /// Makes `path` again as `old` was.
    fn restore(path: &Path, old: &Entry) -> io::Result<()> {
        let mode = old.stamp.mode & 0o7777;
        let (dir, name) = parent_of(path)?;
        cineris_base::hook::point(super::super::PUTTING_BACK);
        match (old.kind, &old.content) {
            (Kind::Directory, _) => match kind_at(&dir, &name)? {
                Now::Missing => {
                    mkdir_at(&dir, &name, mode)?;
                    chmod_at(&dir, &name, mode, Now::Directory)
                }
                Now::Directory => chmod_at(&dir, &name, mode, Now::Directory),
                Now::File | Now::Other => Ok(()),
            },
            (Kind::File, Some(content)) => write_at(&dir, &name, content, mode),
            // Its content was not kept: only its mode can be put back.
            (Kind::File, None) if kind_at(&dir, &name)? == Now::File => {
                chmod_at(&dir, &name, mode, Now::File)
            }
            _ => Ok(()),
        }
    }

    /// The directory `path` is in, opened with no link followed anywhere on
    /// its way, and its last name.
    fn parent_of(path: &Path) -> io::Result<(OwnedFd, CString)> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok((open_dir(parent)?, c_name(name.as_bytes())?))
    }

    fn c_name(bytes: &[u8]) -> io::Result<CString> {
        CString::new(bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    }

    /// Opens the directory at the absolute `path`, refusing a symbolic link
    /// anywhere on the way (`ELOOP`) and anything that is not a directory.
    fn open_dir(path: &Path) -> io::Result<OwnedFd> {
        let raw = c_name(path.as_os_str().as_bytes())?;
        // SAFETY: `open_how` is a plain C struct of integers; all-zero is
        // its documented empty form, which the fields below then fill.
        let mut how: libc::open_how = unsafe { std::mem::zeroed() };
        how.flags = (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64;
        how.resolve = libc::RESOLVE_NO_SYMLINKS;
        // SAFETY: `openat2` takes a directory descriptor (`AT_FDCWD`: the
        // path is absolute), a NUL-terminated path, a pointer to `open_how`
        // and its size. It returns a new descriptor or -1; the descriptor
        // is owned at once.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                libc::AT_FDCWD,
                raw.as_ptr(),
                std::ptr::addr_of!(how),
                std::mem::size_of::<libc::open_how>(),
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = i32::try_from(fd).map_err(|_| io::Error::from(io::ErrorKind::Other))?;
        // SAFETY: `fd` was just returned by `openat2` and nothing else owns
        // it.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// What is at `name` in `dir` now, not following a link.
    fn kind_at(dir: &OwnedFd, name: &CString) -> io::Result<Now> {
        // SAFETY: `stat` is a plain C struct; `fstatat` fills it.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: an open directory, a NUL-terminated name, a pointer to a
        // `stat` this function owns, and a flag that does not follow a
        // link at the name.
        let done = unsafe {
            libc::fstatat(
                dir.as_raw_fd(),
                name.as_ptr(),
                std::ptr::addr_of_mut!(stat),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if done != 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(Now::Missing)
            } else {
                Err(error)
            };
        }
        Ok(match stat.st_mode & libc::S_IFMT {
            libc::S_IFDIR => Now::Directory,
            libc::S_IFREG => Now::File,
            _ => Now::Other,
        })
    }

    /// Removes `name` from `dir`: the entry itself, a link and not what it
    /// leads to; a directory only when it is empty.
    fn unlink_at(dir: &OwnedFd, name: &CString, directory: bool) -> io::Result<()> {
        let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
        // SAFETY: an open directory, a NUL-terminated name, and a flag.
        let done = unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), flags) };
        if done != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Makes the directory `name` in `dir`.
    fn mkdir_at(dir: &OwnedFd, name: &CString, mode: u32) -> io::Result<()> {
        // SAFETY: an open directory, a NUL-terminated name, and a mode.
        let done = unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), mode) };
        if done != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Opens `name` in `dir` with no link followed at it, and without
    /// waiting on a special file.
    fn open_at(dir: &OwnedFd, name: &CString, flags: i32, mode: u32) -> io::Result<OwnedFd> {
        // SAFETY: an open directory, a NUL-terminated name, flags, and the
        // mode a file made by them is given. It returns a new descriptor
        // or -1; the descriptor is owned at once.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                mode,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` was just returned by `openat` and nothing else owns
        // it.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// Gives `name` in `dir` the permission bits `mode`, when it is what
    /// `kind` says: through a descriptor opened with no link followed, so a
    /// link swapped in meanwhile is refused (`ELOOP`), never followed.
    fn chmod_at(dir: &OwnedFd, name: &CString, mode: u32, kind: Now) -> io::Result<()> {
        let flags = libc::O_RDONLY
            | libc::O_NONBLOCK
            | if kind == Now::Directory {
                libc::O_DIRECTORY
            } else {
                0
            };
        let file = std::fs::File::from(open_at(dir, name, flags, 0)?);
        let is_it = match kind {
            Now::Directory => file.metadata()?.is_dir(),
            _ => file.metadata()?.is_file(),
        };
        if !is_it {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(mode))
    }

    /// Writes `content` as the file `name` in `dir`, with the permission
    /// bits `mode`: under a name of its own beside it first, made with no
    /// link followed and never one that is there already, then renamed into
    /// place, which replaces whatever is at the name and writes through
    /// nothing.
    fn write_at(dir: &OwnedFd, name: &CString, content: &[u8], mode: u32) -> io::Result<()> {
        use cineris_base::rng::{Rng, SystemRng};
        let mut temporary = b".".to_vec();
        temporary.extend_from_slice(name.as_bytes());
        temporary
            .extend_from_slice(format!(".cineris-{:016x}", SystemRng::new().next_u64()).as_bytes());
        let temporary = c_name(&temporary)?;
        let made = open_at(
            dir,
            &temporary,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        let written = (|| {
            let mut file = std::fs::File::from(made);
            file.write_all(content)?;
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
            drop(file);
            // SAFETY: an open directory twice and two NUL-terminated names.
            let done = unsafe {
                libc::renameat(
                    dir.as_raw_fd(),
                    temporary.as_ptr(),
                    dir.as_raw_fd(),
                    name.as_ptr(),
                )
            };
            if done != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        if written.is_err() {
            let _ = unlink_at(dir, &temporary, false);
        }
        written
    }
}
