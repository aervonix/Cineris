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
//! - a directory that went is made again, open to its owner while what goes
//!   in it is put back, and given its own mode last, deepest first, so that
//!   a directory whose mode lacks the owner's write right still gets its
//!   children back;
//! - a link that was there before is made again, to where it led;
//! - a mode is put back through a descriptor opened with no link followed
//!   and with no right to read what it names (`O_PATH`), so that a
//!   directory a command made unreadable gets its mode back too;
//! - a file whose content was kept is written under a name of its own
//!   beside it, made with no link followed, and renamed into place, so that
//!   whatever is at the name (a link, another file) is replaced, never
//!   written through; its mode is put back with it;
//! - a file whose content was not kept (too large) has its mode put back,
//!   and nothing else;
//! - a directory on the way that is a link now, or is gone, refuses the
//!   path, which is left as it is.
//!
//! **Nothing is lost.** Before a file is written over or removed, what it
//! holds is kept, as it is then, in a folder of the session's own
//! ([`Keeper`]), up to [`KEEP_BYTES`] a file, owner-only: what a confined
//! start wrote, or what the user wrote there meanwhile, can be found again.
//! A file that cannot be kept (too large, nowhere to keep it) is left as it
//! is, and named to put back by hand (the full review of the third security
//! fix). A path the look before never watched (one the configuration named
//! during the command, or one a link leads to only now) is never touched.
//!
//! Each path is put back on its own, and a failure leaves that path alone.
//! What is left the caller finds by looking again ([`super::Watch::put_back`]),
//! and names as a path to put back by hand. Only Linux puts anything back:
//! protected paths are looked at only where the sandbox enforces.

use std::path::{Path, PathBuf};

use super::{Changed, Snapshot};

/// The most of one file the put-back keeps before it writes over or removes
/// it.
pub const KEEP_BYTES: u64 = 256 * 1024;

/// Where the put-back keeps what it writes over or removes, and what it
/// kept.
#[derive(Debug, Default)]
pub(super) struct Keeper {
    /// The folder, the session's own; `None` keeps nothing, and then
    /// nothing that would be lost is touched.
    #[cfg_attr(
        not(target_os = "linux"),
        expect(dead_code, reason = "only the Linux put-back writes, and keeps")
    )]
    pub(super) dir: Option<PathBuf>,
    /// Each path whose content was kept, and its copy.
    pub(super) kept: Vec<(PathBuf, PathBuf)>,
    /// Each path left as it is because what it holds could not be kept.
    pub(super) unkept: Vec<PathBuf>,
}

#[cfg(target_os = "linux")]
impl Keeper {
    fn could_not_keep(&mut self, path: &Path) {
        if !self.unkept.iter().any(|known| known == path) {
            self.unkept.push(path.to_owned());
        }
    }
}

/// Puts back each path of `changed` as `before` holds it, as far as it
/// safely can, keeping in `keeper` what it writes over or removes. It
/// blocks.
pub(super) fn put_back(before: &Snapshot, changed: &[Changed], keeper: &mut Keeper) {
    #[cfg(target_os = "linux")]
    linux::put_back(before, changed, keeper);
    #[cfg(not(target_os = "linux"))]
    let _ = (before, changed, keeper);
}

/// The name a kept copy of `path` is given in its folder: its count, and
/// the path's last name, shortened to [`NAME_BYTES`] so that the copy's
/// name fits where the path's did (the narrow check of the third security
/// fix: a name near the system's bound did not, with the count before it).
#[cfg(target_os = "linux")]
fn copy_name(count: usize, path: &Path) -> String {
    let name = path
        .file_name()
        .map_or_else(|| "file".into(), |name| name.to_string_lossy());
    let mut short = String::new();
    for character in name.chars() {
        if short.len() + character.len_utf8() > NAME_BYTES {
            break;
        }
        short.push(character);
    }
    format!("{count:03}-{short}")
}

/// The most of a path's last name a kept copy's name holds.
#[cfg(target_os = "linux")]
const NAME_BYTES: usize = 200;

/// Gives the folder at `path` its owner's right to list it back, through a
/// descriptor that needs no read right, where a command took it away: the
/// look that follows sees what is in it. Whether its mode was changed: a
/// folder its owner can list already is left as it is. It blocks.
pub(super) fn reopen(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    return linux::reopen(path).unwrap_or(false);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        false
    }
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
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;

    use super::super::{Before, Changed, Entry, Kind, Snapshot};
    use super::{KEEP_BYTES, Keeper, copy_name};

    /// What is at a name now, not following a link.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Now {
        Missing,
        Directory,
        File,
        /// A link, a special file.
        Other,
    }

    pub(super) fn put_back(before: &Snapshot, changed: &[Changed], keeper: &mut Keeper) {
        // Only what the look before watched: a path it never looked at (one
        // first watched after the command) has nothing to go back to.
        let mut paths: Vec<&Path> = changed
            .iter()
            .filter(|one| one.before == Before::Absent || before.entries.contains_key(&one.path))
            .map(|one| one.path.as_path())
            .collect();
        // What is in the way goes first, deepest first, so that what is
        // under a path goes before the path; then what was there is made
        // again, shallowest first, so that a directory is there before
        // what goes in it; then each directory's own mode, deepest first,
        // since a directory whose mode lacks the owner's write right would
        // refuse what goes in it.
        paths.sort_by_key(|path| Reverse(path.components().count()));
        let mut refused: Vec<&Path> = Vec::new();
        for path in &paths {
            if matches!(
                clear(path, before.entries.get(*path), keeper),
                Ok(Cleared::Refused)
            ) {
                keeper.could_not_keep(path);
                refused.push(path);
            }
        }
        paths.reverse();
        for path in &paths {
            if refused.contains(path) {
                continue;
            }
            if let Some(old) = before.entries.get(*path) {
                let _ = restore(path, old, keeper);
            }
        }
        paths.reverse();
        for path in &paths {
            if let Some(old) = before.entries.get(*path)
                && old.kind == Kind::Directory
            {
                let _ = directory_mode(path, old);
            }
        }
    }

    /// Whether `old` can be made again: nothing (the path was not there), a
    /// directory, a file whose content was kept, or a link whose target was
    /// read. Anything else is left as it is now: removing it would lose
    /// what is there and put nothing back.
    fn can_be_made_again(old: Option<&Entry>) -> bool {
        old.is_none_or(|old| match old.kind {
            Kind::Directory => true,
            Kind::File => old.content.is_some(),
            Kind::Link => old.link.is_some(),
            Kind::Other => false,
        })
    }

    /// Removes what is at `path` now where it is in the way of what was
    /// there before: anything, where nothing was; something of another kind,
    /// where something was.
    /// What clearing a path came to.
    #[derive(Debug, PartialEq, Eq)]
    enum Cleared {
        /// Cleared, or nothing in the way, or it failed for a reason that
        /// does not stop the path being made again (its folder gone too).
        Done,
        /// What is there could not be kept: the path is not touched.
        Refused,
    }

    fn clear(path: &Path, old: Option<&Entry>, keeper: &mut Keeper) -> io::Result<Cleared> {
        if !can_be_made_again(old) {
            return Ok(Cleared::Done);
        }
        let (dir, name) = parent_of(path)?;
        let now = kind_at(&dir, &name)?;
        // A folder the start made, which it may have closed to its owner:
        // opened to its owner, so that the next round can list it and put
        // back what is in it (the full review of the third security fix).
        if old.is_none() && now == Now::Directory {
            let _ = chmod_at(&dir, &name, 0o700, Now::Directory);
        }
        let in_the_way = match (old.map(|old| old.kind), now) {
            (_, Now::Missing)
            | (Some(Kind::Directory), Now::Directory)
            // A file is replaced by the rename that puts it back.
            | (Some(Kind::File), Now::File) => false,
            // A link to where the old one led is the old one.
            (Some(Kind::Link), Now::Other) => {
                link_at(&dir, &name).ok() != old.and_then(|old| old.link.clone())
            }
            _ => true,
        };
        if in_the_way {
            if now != Now::Directory && keep(keeper, &dir, &name, path).is_err() {
                return Ok(Cleared::Refused);
            }
            unlink_at(&dir, &name, now == Now::Directory)?;
        }
        Ok(Cleared::Done)
    }

    /// Keeps what `name` in `dir` holds now, before it is written over or
    /// removed: a file's content, up to [`KEEP_BYTES`], or where a link
    /// leads. Nothing for a folder (removed only when empty) or a special
    /// file (it holds nothing). An error where it cannot be kept: then the
    /// path is not touched.
    fn keep(keeper: &mut Keeper, dir: &OwnedFd, name: &CString, path: &Path) -> io::Result<()> {
        if keeper.kept.iter().any(|(kept, _)| kept == path) {
            return Ok(());
        }
        let content = match kind_at(dir, name)? {
            Now::Missing | Now::Directory => return Ok(()),
            Now::File => read_at(dir, name)?,
            Now::Other => match link_at(dir, name) {
                Ok(target) => target.into_os_string().into_vec(),
                Err(_) => return Ok(()),
            },
        };
        let into = keeper
            .dir
            .clone()
            .ok_or_else(|| io::Error::other("nowhere to keep what is replaced"))?;
        cineris_base::private::create_dir_all(&into)?;
        // The next name free in the folder: it holds the copies of every
        // put-back of the session, and of a session opened again (the
        // narrow check of the third security fix: each put-back counted
        // from one, and a second copy of a file of the same name was
        // refused).
        let mut count = keeper.kept.len() + 1;
        let (copy, mut file) = loop {
            let copy = into.join(copy_name(count, path));
            match cineris_base::private::create_new(&copy) {
                Ok(file) => break (copy, file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && count < COPIES => {
                    count += 1;
                }
                Err(error) => return Err(error),
            }
        };
        file.write_all(&content)?;
        keeper.kept.push((path.to_owned(), copy));
        Ok(())
    }

    /// The most copies the folder is looked through for a free name.
    const COPIES: usize = 100_000;

    /// Gives the folder `path` its owner's right to list it, through an
    /// `O_PATH` descriptor, keeping the rest of its mode.
    pub(super) fn reopen(path: &Path) -> io::Result<bool> {
        let (dir, name) = parent_of(path)?;
        let (now, mode) = mode_at(&dir, &name)?;
        // Only where its owner lacks the right to read or search it: a mode
        // set again, even the same, would move the folder's change time.
        if now != Now::Directory || mode & 0o500 == 0o500 {
            return Ok(false);
        }
        chmod_at(&dir, &name, (mode & 0o7777) | 0o500, Now::Directory)?;
        Ok(true)
    }

    /// What `name` in `dir` is, with its mode, with no link followed.
    fn mode_at(dir: &OwnedFd, name: &CString) -> io::Result<(Now, u32)> {
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
            return Err(io::Error::last_os_error());
        }
        let now = match stat.st_mode & libc::S_IFMT {
            libc::S_IFDIR => Now::Directory,
            libc::S_IFREG => Now::File,
            _ => Now::Other,
        };
        Ok((now, stat.st_mode))
    }

    /// The content of the regular file `name` in `dir`, opened with no link
    /// followed and without waiting: an error past [`KEEP_BYTES`].
    fn read_at(dir: &OwnedFd, name: &CString) -> io::Result<Vec<u8>> {
        use std::io::Read;
        let flags = libc::O_RDONLY | libc::O_NONBLOCK;
        let opened = match open_at(dir, name, flags, 0) {
            // A file its owner may not read (a program runs without the
            // right): given its owner's read right through a descriptor
            // that needs none, since it is written over or removed next
            // (the narrow check of the third security fix).
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                let (now, mode) = mode_at(dir, name)?;
                if now != Now::File {
                    return Err(error);
                }
                chmod_at(dir, name, (mode & 0o7777) | 0o400, Now::File)?;
                open_at(dir, name, flags, 0)?
            }
            other => other?,
        };
        let file = std::fs::File::from(opened);
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > KEEP_BYTES {
            return Err(io::Error::other("too large to keep, or not a file"));
        }
        let mut content = Vec::new();
        file.take(KEEP_BYTES + 1).read_to_end(&mut content)?;
        if content.len() as u64 > KEEP_BYTES {
            return Err(io::Error::other("too large to keep"));
        }
        Ok(content)
    }

    /// Makes `path` again as `old` was.
    fn restore(path: &Path, old: &Entry, keeper: &mut Keeper) -> io::Result<()> {
        let mode = old.stamp.mode & 0o7777;
        let (dir, name) = parent_of(path)?;
        cineris_base::hook::point(super::super::PUTTING_BACK);
        match (old.kind, &old.content) {
            // Open to its owner until what goes in it is back: its own mode
            // is given last ([`directory_mode`]).
            (Kind::Directory, _) => match kind_at(&dir, &name)? {
                Now::Missing => {
                    mkdir_at(&dir, &name, mode | 0o700)?;
                    chmod_at(&dir, &name, mode | 0o700, Now::Directory)
                }
                Now::Directory => chmod_at(&dir, &name, mode | 0o700, Now::Directory),
                Now::File | Now::Other => Ok(()),
            },
            (Kind::Link, _) => match (&old.link, kind_at(&dir, &name)?) {
                (Some(target), Now::Missing) => symlink_at(&dir, &name, target),
                _ => Ok(()),
            },
            (Kind::File, Some(content)) => {
                // What is there now is kept first, or the path is left.
                if kind_at(&dir, &name)? == Now::File && keep(keeper, &dir, &name, path).is_err() {
                    keeper.could_not_keep(path);
                    return Ok(());
                }
                write_at(&dir, &name, content, mode)
            }
            // Its content was not kept: only its mode can be put back.
            (Kind::File, None) if kind_at(&dir, &name)? == Now::File => {
                chmod_at(&dir, &name, mode, Now::File)
            }
            _ => Ok(()),
        }
    }

    /// Gives the directory `path` the mode `old` had, once what goes in it
    /// is back.
    fn directory_mode(path: &Path, old: &Entry) -> io::Result<()> {
        let (dir, name) = parent_of(path)?;
        if kind_at(&dir, &name)? != Now::Directory {
            return Ok(());
        }
        // Never a mode its owner cannot list: a folder no look can list hides
        // what is in it from every look after (the full review).
        chmod_at(
            &dir,
            &name,
            (old.stamp.mode & 0o7777) | 0o500,
            Now::Directory,
        )
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
    /// `kind` says: through a descriptor opened with no link followed and
    /// with no right needed to read what it names (`O_PATH`), so that a
    /// directory a command made unreadable gets its mode back, and a link
    /// swapped in meanwhile is never followed (its descriptor is the link's,
    /// which is not what `kind` says). The mode is set through the
    /// descriptor's own entry in `/proc`, which names that very node.
    fn chmod_at(dir: &OwnedFd, name: &CString, mode: u32, kind: Now) -> io::Result<()> {
        cineris_base::hook::point(super::super::PUTTING_BACK_A_MODE);
        let flags = libc::O_PATH
            | if kind == Now::Directory {
                libc::O_DIRECTORY
            } else {
                0
            };
        let node = open_at(dir, name, flags, 0)?;
        // SAFETY: `stat` is a plain C struct; `fstat` fills it.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: an open descriptor and a pointer to a `stat` this
        // function owns.
        if unsafe { libc::fstat(node.as_raw_fd(), std::ptr::addr_of_mut!(stat)) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let wanted = if kind == Now::Directory {
            libc::S_IFDIR
        } else {
            libc::S_IFREG
        };
        if stat.st_mode & libc::S_IFMT != wanted {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            format!("/proc/self/fd/{}", node.as_raw_fd()),
            std::fs::Permissions::from_mode(mode),
        )
    }

    /// Where the link `name` in `dir` leads, as it is written.
    fn link_at(dir: &OwnedFd, name: &CString) -> io::Result<std::path::PathBuf> {
        let mut buffer = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: an open directory, a NUL-terminated name, and a buffer of
        // the length given, which `readlinkat` fills without a NUL.
        let read = unsafe {
            libc::readlinkat(
                dir.as_raw_fd(),
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        let read = usize::try_from(read).map_err(|_| io::Error::last_os_error())?;
        buffer.truncate(read);
        Ok(std::path::PathBuf::from(
            <std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(buffer),
        ))
    }

    /// Makes the link `name` in `dir`, leading to `target`.
    fn symlink_at(dir: &OwnedFd, name: &CString, target: &Path) -> io::Result<()> {
        let target = c_name(target.as_os_str().as_bytes())?;
        // SAFETY: a NUL-terminated target, an open directory and a
        // NUL-terminated name.
        let done = unsafe { libc::symlinkat(target.as_ptr(), dir.as_raw_fd(), name.as_ptr()) };
        if done != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Writes `content` as the file `name` in `dir`, with the permission
    /// bits `mode`: under a name of its own beside it first, made with no
    /// link followed and never one that is there already, then renamed into
    /// place, which replaces whatever is at the name and writes through
    /// nothing.
    fn write_at(dir: &OwnedFd, name: &CString, content: &[u8], mode: u32) -> io::Result<()> {
        use cineris_base::rng::{Rng, SystemRng};
        // The name shortened, so that the name of its own fits where the
        // name did (the narrow check of the third security fix).
        let mut temporary = b".".to_vec();
        temporary.extend(name.as_bytes().iter().take(super::NAME_BYTES));
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
