//! A folder read through handles that follow no link (since step 28's
//! `cineris ext add`; `docs/architecture/12-extensions.md`, "As built in step
//! 28: managing extensions").
//!
//! A folder is opened once, by the path given, and everything in it is
//! reached from that handle one name at a time: a folder in it through the
//! handle of the folder it is in, a file the same way, and a listing through
//! the folder's own handle. **No name is followed when it is a link**, and no
//! path is looked up again, so a folder swapped for a link while the tree is
//! read is refused where it is met, and nothing outside the tree is read
//! through one.
//!
//! | | How |
//! |---|---|
//! | Linux | The folder opened with `O_DIRECTORY` and `O_NOFOLLOW`; each name in it with `openat` relative to the folder's descriptor, `O_NOFOLLOW` always and `O_DIRECTORY` for a folder, a file also with `O_NONBLOCK` and `O_NOCTTY`, so that a FIFO or a terminal opens at once and is then refused as no file; the folder listed through a descriptor of its own (`fdopendir`). |
//! | Windows | The folder opened with `FILE_FLAG_OPEN_REPARSE_POINT`; each name in it with `NtCreateFile` relative to the folder's handle and `FILE_OPEN_REPARSE_POINT`; the folder listed through its handle (`GetFileInformationByHandleEx`). Every handle is held without sharing deletion, so that nothing held can be renamed or removed while the tree is read. A reparse point that leads somewhere (a symbolic link, a junction, and every tag not known to be a placeholder, `cineris_base::nofollow::is_placeholder_tag`) is refused. A placeholder (a cloud file, a file the system compressed) is opened a second time in the ordinary way relative to the same folder, while the first handle, which shares no deletion, keeps the name from being swapped, and the two are checked to be one file. |
//! | Elsewhere | Not built: every open is refused. |
//!
//! **The folder named is opened as its path says:** the names before its
//! last are followed as the system follows them (the person named the path),
//! and its last name is not: a folder named through a link at its own name is
//! refused, to be named where it leads.
//!
//! These functions block. Callers run them on the blocking pool.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a blocking file primitive; callers run it on the blocking pool"
)]

use std::ffi::{OsStr, OsString};
use std::fs::{File, Metadata};
use std::io;
use std::path::Path;

/// What a name in a folder is, as the folder's listing says. Only a hint:
/// what [`Folder::folder`] and [`Folder::file`] find when they open it
/// decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A folder.
    Folder,
    /// A regular file.
    File,
    /// A symbolic link, a junction, or another reparse point that leads
    /// somewhere.
    Link,
    /// A FIFO, a socket, a device.
    Other,
}

/// One name in a folder, and what the listing says it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The name, as the system gives it.
    pub name: OsString,
    /// What it is, by the listing.
    pub kind: Kind,
}

/// Why a name was not opened. Nothing was read from it.
#[derive(Debug, thiserror::Error)]
pub enum Refused {
    /// Nothing has that name.
    #[error("there is nothing of that name")]
    NotFound,
    /// It is a link, or on Windows a reparse point that leads somewhere. It
    /// was not followed.
    #[error("it is a link, which is not followed")]
    Link,
    /// A folder was asked for, and it is not one.
    #[error("it is not a folder")]
    NotAFolder,
    /// A file was asked for, and it is not a regular one: a folder, a FIFO,
    /// a socket or a device.
    #[error("it is not a regular file")]
    NotAFile,
    /// The name is not one name: empty, `.`, `..`, or with a separator in it.
    #[error("it is not one name")]
    NotAName,
    /// This system has no way to open a name relative to a folder here.
    #[error("a folder cannot be read through its handle on this system")]
    Unsupported,
    /// The system refused.
    #[error("{0}")]
    Io(#[source] io::Error),
}

/// A folder held open, opened following no link at its name.
#[derive(Debug)]
pub struct Folder {
    inner: sys::Folder,
}

impl Folder {
    /// Opens the folder at `path`: the names before its last followed as
    /// the system follows them, and its last name not followed.
    ///
    /// # Errors
    ///
    /// [`Refused`]: nothing there, a link at its name, or no folder.
    pub fn open(path: &Path) -> Result<Self, Refused> {
        sys::open(path).map(|inner| Self { inner })
    }

    /// Every name in the folder but `.` and `..`, in the order the system
    /// lists them, through the folder's own handle.
    ///
    /// # Errors
    ///
    /// When the system cannot list it to its end.
    pub fn entries(&mut self) -> io::Result<Vec<Entry>> {
        self.inner.entries()
    }

    /// The folder `name` in this one, opened relative to it: a link at the
    /// name is refused, not followed.
    ///
    /// # Errors
    ///
    /// [`Refused`]: nothing of the name, a link, or no folder.
    pub fn folder(&self, name: &OsStr) -> Result<Self, Refused> {
        one_name(name)?;
        self.inner.folder(name).map(|inner| Self { inner })
    }

    /// The regular file `name` in this one, opened for reading relative to
    /// it, with its handle's own metadata: a link at the name is refused,
    /// not followed, and a FIFO or a device is refused without waiting.
    ///
    /// # Errors
    ///
    /// [`Refused`]: nothing of the name, a link, or no regular file.
    pub fn file(&self, name: &OsStr) -> Result<(File, Metadata), Refused> {
        one_name(name)?;
        self.inner.file(name)
    }
}

/// Refuses what is not one name: empty, `.`, `..`, or holding a separator
/// (or, on Windows, a `:` that names a stream) or a NUL.
fn one_name(name: &OsStr) -> Result<(), Refused> {
    let bytes = name.as_encoded_bytes();
    let separators: &[u8] = if cfg!(windows) {
        b"/\\:\0"
    } else {
        b"/\0"
    };
    if bytes.is_empty()
        || bytes == b"."
        || bytes == b".."
        || bytes.iter().any(|byte| separators.contains(byte))
    {
        return Err(Refused::NotAName);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "decision 0030: a name opened relative to a folder's descriptor, and the folder \
              listed through it, are only reachable through system calls; cineris-sandbox is \
              where such calls live"
)]
mod sys {
    use std::ffi::{CStr, CString, OsStr, OsString};
    use std::fs::{File, Metadata, OpenOptions};
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    use super::{Entry, Kind, Refused};

    /// The flags every open here has: no link followed at the name, no wait
    /// on a FIFO, no terminal made the process's own. The standard library
    /// adds `O_CLOEXEC` to its own opens; `openat` is given it here.
    const UNFOLLOWED: i32 = libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC;

    #[derive(Debug)]
    pub(super) struct Folder {
        fd: OwnedFd,
    }

    fn refused(error: io::Error) -> Refused {
        match error.raw_os_error() {
            _ if error.kind() == io::ErrorKind::NotFound => Refused::NotFound,
            // What `O_NOFOLLOW` reports for a link at the name.
            Some(libc::ELOOP | libc::EMLINK) => Refused::Link,
            Some(libc::ENOTDIR) => Refused::NotAFolder,
            Some(libc::ENXIO | libc::ENODEV) => Refused::NotAFile,
            _ => Refused::Io(error),
        }
    }

    pub(super) fn open(path: &Path) -> Result<Folder, Refused> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(UNFOLLOWED | libc::O_DIRECTORY)
            .open(path)
            .map_err(refused)
            // With `O_DIRECTORY`, a link at the name is "not a folder":
            // told apart by a look at the name itself.
            .map_err(|refused| match refused {
                Refused::NotAFolder
                    if std::fs::symlink_metadata(path).is_ok_and(|found| found.is_symlink()) =>
                {
                    Refused::Link
                }
                other => other,
            })?;
        folder_of(file)
    }

    /// `file`, held as a folder when its own metadata says it is one.
    fn folder_of(file: File) -> Result<Folder, Refused> {
        let metadata = file.metadata().map_err(Refused::Io)?;
        if !metadata.is_dir() {
            return Err(Refused::NotAFolder);
        }
        Ok(Folder {
            fd: OwnedFd::from(file),
        })
    }

    fn c_name(name: &OsStr) -> Result<CString, Refused> {
        CString::new(name.as_bytes()).map_err(|_| Refused::NotAName)
    }

    impl Folder {
        /// `name` in this folder, opened with `flags` and no link followed.
        fn at(&self, name: &OsStr, flags: i32) -> Result<File, Refused> {
            let name = c_name(name)?;
            // SAFETY: an open directory descriptor, a NUL-terminated name
            // of one component (checked by the caller), and flags. It
            // returns a new descriptor or -1; the descriptor is owned at
            // once.
            let fd = unsafe {
                libc::openat(
                    self.fd.as_raw_fd(),
                    name.as_ptr(),
                    flags | UNFOLLOWED | libc::O_RDONLY,
                )
            };
            if fd < 0 {
                return Err(refused(io::Error::last_os_error()));
            }
            // SAFETY: `fd` was just returned by `openat` and nothing else
            // owns it.
            Ok(File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
        }

        pub(super) fn folder(&self, name: &OsStr) -> Result<Self, Refused> {
            match self.at(name, libc::O_DIRECTORY) {
                Ok(file) => folder_of(file),
                // With `O_DIRECTORY`, a link at the name is "not a folder":
                // told apart by a look at the name itself.
                Err(Refused::NotAFolder) => {
                    let link = self.kind_of(&c_name(name)?).ok() == Some(Kind::Link);
                    Err(if link {
                        Refused::Link
                    } else {
                        Refused::NotAFolder
                    })
                }
                Err(other) => Err(other),
            }
        }

        pub(super) fn file(&self, name: &OsStr) -> Result<(File, Metadata), Refused> {
            let file = self.at(name, 0)?;
            let metadata = file.metadata().map_err(Refused::Io)?;
            if !metadata.is_file() {
                return Err(Refused::NotAFile);
            }
            Ok((file, metadata))
        }

        /// What `name` is, not following a link: for a listing that does
        /// not say.
        fn kind_of(&self, name: &CStr) -> io::Result<Kind> {
            // SAFETY: `stat` is a plain C struct; `fstatat` fills it.
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: an open directory, a NUL-terminated name, a pointer
            // to a `stat` this function owns, and a flag that does not
            // follow a link at the name.
            let done = unsafe {
                libc::fstatat(
                    self.fd.as_raw_fd(),
                    name.as_ptr(),
                    std::ptr::addr_of_mut!(stat),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if done != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(match stat.st_mode & libc::S_IFMT {
                libc::S_IFDIR => Kind::Folder,
                libc::S_IFREG => Kind::File,
                libc::S_IFLNK => Kind::Link,
                _ => Kind::Other,
            })
        }

        pub(super) fn entries(&mut self) -> io::Result<Vec<Entry>> {
            // A descriptor of the listing's own, since `closedir` closes the
            // one it is given; it shares the folder's offset, which is put
            // back to the start first.
            let own = self.fd.try_clone()?.into_raw_fd();
            // SAFETY: `own` is an open directory descriptor this function
            // owns; on success its ownership passes to the stream.
            let stream = unsafe { libc::fdopendir(own) };
            if stream.is_null() {
                let error = io::Error::last_os_error();
                // SAFETY: the stream was not made, so `own` is still ours.
                drop(unsafe { OwnedFd::from_raw_fd(own) });
                return Err(error);
            }
            let stream = Stream(stream);
            // SAFETY: a stream `fdopendir` made, not yet closed.
            unsafe { libc::rewinddir(stream.0) };
            let mut entries = Vec::new();
            loop {
                // SAFETY: errno is this thread's own; `readdir` sets it only
                // on an error, so it is cleared first to tell an error from
                // the end of the listing.
                unsafe { *libc::__errno_location() = 0 };
                // SAFETY: a stream `fdopendir` made, not yet closed, used by
                // this thread alone.
                let entry = unsafe { libc::readdir(stream.0) };
                if entry.is_null() {
                    // SAFETY: as above.
                    let errno = unsafe { *libc::__errno_location() };
                    if errno != 0 {
                        return Err(io::Error::from_raw_os_error(errno));
                    }
                    break;
                }
                // SAFETY: `readdir` returned an entry that stays valid until
                // the next call on the stream; its name is NUL-terminated.
                let (name, d_type) = unsafe {
                    (
                        CStr::from_ptr((*entry).d_name.as_ptr()).to_owned(),
                        (*entry).d_type,
                    )
                };
                let bytes = name.as_bytes();
                if bytes == b"." || bytes == b".." {
                    continue;
                }
                let kind = match d_type {
                    libc::DT_DIR => Kind::Folder,
                    libc::DT_REG => Kind::File,
                    libc::DT_LNK => Kind::Link,
                    libc::DT_UNKNOWN => self.kind_of(&name)?,
                    _ => Kind::Other,
                };
                entries.push(Entry {
                    name: OsString::from_vec(name.into_bytes()),
                    kind,
                });
            }
            Ok(entries)
        }
    }

    /// A listing's stream, closed when it is dropped.
    struct Stream(*mut libc::DIR);

    impl Drop for Stream {
        fn drop(&mut self) {
            // SAFETY: a stream `fdopendir` made, closed once, here.
            unsafe { libc::closedir(self.0) };
        }
    }
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "decision 0030: a name opened relative to a folder's handle, and the folder listed \
              through it, are only reachable through the Windows API; cineris-sandbox is where \
              such calls live"
)]
mod sys {
    use std::ffi::{OsStr, OsString};
    use std::fs::{File, Metadata, OpenOptions};
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::Path;

    use cineris_base::FileIdentity;
    use cineris_base::nofollow::is_placeholder_tag;
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
        FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
    };
    use windows_sys::Win32::Foundation::{
        ERROR_NO_MORE_FILES, HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError,
        UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FULL_DIR_INFO,
        FILE_GENERIC_READ, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_TRAVERSE, FileAttributeTagInfo, FileFullDirectoryInfo,
        FileFullDirectoryRestartInfo, GetFileInformationByHandleEx, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    use super::{Entry, Kind, Refused};

    /// Every handle here shares reading and writing and never deletion: a
    /// name held cannot be renamed or removed while the tree is read.
    const SHARED: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;

    const STATUS_OBJECT_NAME_NOT_FOUND: NTSTATUS = 0xC000_0034_u32 as i32;
    const STATUS_OBJECT_PATH_NOT_FOUND: NTSTATUS = 0xC000_003A_u32 as i32;
    const STATUS_NOT_A_DIRECTORY: NTSTATUS = 0xC000_0103_u32 as i32;
    const STATUS_FILE_IS_A_DIRECTORY: NTSTATUS = 0xC000_00BA_u32 as i32;

    #[derive(Debug)]
    pub(super) struct Folder {
        handle: OwnedHandle,
    }

    fn refused(error: io::Error) -> Refused {
        match error.raw_os_error() {
            _ if error.kind() == io::ErrorKind::NotFound => Refused::NotFound,
            // ERROR_DIRECTORY: the name is not a folder.
            Some(267) => Refused::NotAFolder,
            _ => Refused::Io(error),
        }
    }

    /// The attributes and the reparse tag of an open handle.
    fn tag_of(handle: &OwnedHandle) -> io::Result<(u32, u32)> {
        let mut info = FILE_ATTRIBUTE_TAG_INFO {
            FileAttributes: 0,
            ReparseTag: 0,
        };
        // SAFETY: an open handle, the class whose buffer is a
        // `FILE_ATTRIBUTE_TAG_INFO`, a pointer to one this function owns and
        // its size.
        let done = unsafe {
            GetFileInformationByHandleEx(
                handle.as_raw_handle() as HANDLE,
                FileAttributeTagInfo,
                std::ptr::addr_of_mut!(info).cast(),
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        if done == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((info.FileAttributes, info.ReparseTag))
    }

    /// Whether what `handle` holds is no reparse point (`false`), a
    /// placeholder (`true`), or something that leads elsewhere (refused).
    fn placeholder(handle: &OwnedHandle) -> Result<bool, Refused> {
        let (attributes, tag) = tag_of(handle).map_err(Refused::Io)?;
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            Ok(false)
        } else if is_placeholder_tag(tag) {
            Ok(true)
        } else {
            Err(Refused::Link)
        }
    }

    pub(super) fn open(path: &Path) -> Result<Folder, Refused> {
        let open = |reparse: u32| {
            OpenOptions::new()
                .read(true)
                .share_mode(SHARED)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | reparse)
                .open(path)
                .map(OwnedHandle::from)
                .map_err(refused)
        };
        let first = open(FILE_FLAG_OPEN_REPARSE_POINT)?;
        let handle = if placeholder(&first)? {
            // A placeholder: opened again in the ordinary way, the first
            // handle keeping the name from being swapped meanwhile.
            let second = open(0)?;
            same_file(&first, &second)?;
            second
        } else {
            first
        };
        folder_of(handle)
    }

    fn folder_of(handle: OwnedHandle) -> Result<Folder, Refused> {
        let file = File::from(handle);
        if !file.metadata().map_err(Refused::Io)?.is_dir() {
            return Err(Refused::NotAFolder);
        }
        Ok(Folder {
            handle: OwnedHandle::from(file),
        })
    }

    /// Refuses two handles that are not of one file.
    fn same_file(first: &OwnedHandle, second: &OwnedHandle) -> Result<(), Refused> {
        let identity = |handle: &OwnedHandle| {
            let file = File::from(handle.try_clone().map_err(Refused::Io)?);
            FileIdentity::of(&file).map_err(Refused::Io)
        };
        if identity(first)? == identity(second)? {
            Ok(())
        } else {
            Err(Refused::Link)
        }
    }

    impl Folder {
        /// `name` in this folder, opened relative to its handle: a folder or
        /// a file as `folder` says, and with no reparse point followed
        /// unless `follow`.
        fn at(&self, name: &OsStr, folder: bool, follow: bool) -> Result<OwnedHandle, Refused> {
            let wide: Vec<u16> = name.encode_wide().collect();
            let length = u16::try_from(wide.len() * 2).map_err(|_| Refused::NotAName)?;
            let unicode = UNICODE_STRING {
                Length: length,
                MaximumLength: length,
                Buffer: wide.as_ptr().cast_mut(),
            };
            let attributes = OBJECT_ATTRIBUTES {
                Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
                RootDirectory: self.handle.as_raw_handle() as HANDLE,
                ObjectName: std::ptr::addr_of!(unicode),
                Attributes: OBJ_CASE_INSENSITIVE,
                SecurityDescriptor: std::ptr::null(),
                SecurityQualityOfService: std::ptr::null(),
            };
            let access = if folder {
                FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE
            } else {
                FILE_GENERIC_READ
            };
            let options = FILE_SYNCHRONOUS_IO_NONALERT
                | if folder {
                    FILE_DIRECTORY_FILE
                } else {
                    FILE_NON_DIRECTORY_FILE
                }
                | if follow { 0 } else { FILE_OPEN_REPARSE_POINT };
            // SAFETY: `IO_STATUS_BLOCK` is a plain C struct; all-zero is
            // its empty form, which the call fills.
            let mut status_block: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
            let mut handle: HANDLE = std::ptr::null_mut();
            // SAFETY: a pointer for the new handle, the access asked for,
            // object attributes naming one component relative to an open
            // folder handle (the name's buffer and the attributes live for
            // the call), a status block this function owns, no allocation
            // size, no attributes to set, the sharing, an open of what is
            // there and nothing made, the options, and no extended
            // attributes. On success the handle is new and owned at once.
            let status = unsafe {
                NtCreateFile(
                    &mut handle,
                    access,
                    &attributes,
                    &mut status_block,
                    std::ptr::null(),
                    0,
                    SHARED,
                    FILE_OPEN,
                    options,
                    std::ptr::null(),
                    0,
                )
            };
            if status < 0 {
                return Err(match status {
                    STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND => {
                        Refused::NotFound
                    }
                    STATUS_NOT_A_DIRECTORY => Refused::NotAFolder,
                    STATUS_FILE_IS_A_DIRECTORY => Refused::NotAFile,
                    _ => {
                        // SAFETY: a pure function of the status.
                        let code = unsafe { RtlNtStatusToDosError(status) };
                        Refused::Io(io::Error::from_raw_os_error(code as i32))
                    }
                });
            }
            // SAFETY: `handle` was just made by `NtCreateFile` and nothing
            // else owns it.
            Ok(unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
        }

        /// `name` opened as `at` opens it, a placeholder opened again in the
        /// ordinary way and checked to be the same file.
        fn unfollowed(&self, name: &OsStr, folder: bool) -> Result<OwnedHandle, Refused> {
            let first = self.at(name, folder, false)?;
            if !placeholder(&first)? {
                return Ok(first);
            }
            let second = self.at(name, folder, true)?;
            same_file(&first, &second)?;
            Ok(second)
        }

        pub(super) fn folder(&self, name: &OsStr) -> Result<Self, Refused> {
            folder_of(self.unfollowed(name, true)?)
        }

        pub(super) fn file(&self, name: &OsStr) -> Result<(File, Metadata), Refused> {
            let file = File::from(self.unfollowed(name, false)?);
            let metadata = file.metadata().map_err(Refused::Io)?;
            if !metadata.is_file() {
                return Err(Refused::NotAFile);
            }
            Ok((file, metadata))
        }

        pub(super) fn entries(&mut self) -> io::Result<Vec<Entry>> {
            // 64 KiB, aligned for the entries, which are on 8-byte bounds.
            let mut buffer = vec![0_u64; 8 * 1024];
            let size = (buffer.len() * 8) as u32;
            let mut entries = Vec::new();
            let mut class = FileFullDirectoryRestartInfo;
            loop {
                // SAFETY: an open folder handle with the right to list it,
                // the class whose buffer is a run of `FILE_FULL_DIR_INFO`, a
                // buffer this function owns and its size.
                let done = unsafe {
                    GetFileInformationByHandleEx(
                        self.handle.as_raw_handle() as HANDLE,
                        class,
                        buffer.as_mut_ptr().cast(),
                        size,
                    )
                };
                if done == 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                        break;
                    }
                    return Err(error);
                }
                class = FileFullDirectoryInfo;
                let base = buffer.as_ptr().cast::<u8>();
                let mut offset = 0_usize;
                loop {
                    // SAFETY: the system wrote whole entries into the buffer,
                    // each at the offset the one before gives, 8-byte
                    // aligned, its name `FileNameLength` bytes long and inside
                    // the buffer.
                    let (next, attributes, tag, name) = unsafe {
                        let info = base.add(offset).cast::<FILE_FULL_DIR_INFO>();
                        let length = (*info).FileNameLength as usize / 2;
                        let name = std::slice::from_raw_parts(
                            std::ptr::addr_of!((*info).FileName).cast::<u16>(),
                            length,
                        );
                        (
                            (*info).NextEntryOffset as usize,
                            (*info).FileAttributes,
                            // The reparse tag, for a reparse point.
                            (*info).EaSize,
                            OsString::from_wide(name),
                        )
                    };
                    if name != "." && name != ".." {
                        let kind = if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                            && !is_placeholder_tag(tag)
                        {
                            Kind::Link
                        } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                            Kind::Folder
                        } else {
                            Kind::File
                        };
                        entries.push(Entry { name, kind });
                    }
                    if next == 0 {
                        break;
                    }
                    offset += next;
                }
            }
            Ok(entries)
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod sys {
    use std::ffi::OsStr;
    use std::fs::{File, Metadata};
    use std::io;
    use std::path::Path;

    use super::{Entry, Refused};

    #[derive(Debug)]
    pub(super) struct Folder;

    pub(super) fn open(_path: &Path) -> Result<Folder, Refused> {
        Err(Refused::Unsupported)
    }

    impl Folder {
        pub(super) fn folder(&self, _name: &OsStr) -> Result<Self, Refused> {
            Err(Refused::Unsupported)
        }

        pub(super) fn file(&self, _name: &OsStr) -> Result<(File, Metadata), Refused> {
            Err(Refused::Unsupported)
        }

        pub(super) fn entries(&mut self) -> io::Result<Vec<Entry>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cineris-sandbox-unfollowed-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A folder is listed and its names opened through its handle; a name
    /// that is not one name is refused before anything is asked of the
    /// system.
    #[test]
    fn a_folder_is_read_one_name_at_a_time_through_its_handle() {
        let dir = scratch("read");
        std::fs::create_dir(dir.join("lib")).unwrap();
        std::fs::write(dir.join("main.js"), "a").unwrap();
        std::fs::write(dir.join("lib/x.js"), "bc").unwrap();
        let mut root = Folder::open(&dir).unwrap();
        let mut names: Vec<(String, Kind)> = root
            .entries()
            .unwrap()
            .into_iter()
            .map(|entry| (entry.name.to_string_lossy().into_owned(), entry.kind))
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                ("lib".to_owned(), Kind::Folder),
                ("main.js".to_owned(), Kind::File)
            ]
        );
        // Listed twice, the same.
        assert_eq!(root.entries().unwrap().len(), 2);
        let lib = root.folder(OsStr::new("lib")).unwrap();
        let (_, metadata) = lib.file(OsStr::new("x.js")).unwrap();
        assert_eq!(metadata.len(), 2);
        assert!(matches!(
            root.file(OsStr::new("lib")),
            Err(Refused::NotAFile)
        ));
        assert!(matches!(
            root.folder(OsStr::new("main.js")),
            Err(Refused::NotAFolder)
        ));
        assert!(matches!(
            root.file(OsStr::new("gone")),
            Err(Refused::NotFound)
        ));
        for name in ["", ".", "..", "lib/x.js"] {
            assert!(
                matches!(root.file(OsStr::new(name)), Err(Refused::NotAName)),
                "{name:?}"
            );
            assert!(
                matches!(root.folder(OsStr::new(name)), Err(Refused::NotAName)),
                "{name:?}"
            );
        }
        assert!(matches!(
            Folder::open(&dir.join("main.js")),
            Err(Refused::NotAFolder)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A link is never followed: at the folder's own name, at a folder's
    /// name in it, or at a file's; and a link's listing says what it is.
    #[cfg(unix)]
    #[test]
    fn a_link_at_any_name_is_refused_and_never_followed() {
        let dir = scratch("links");
        let outside = dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), "x").unwrap();
        let tree = dir.join("tree");
        std::fs::create_dir(&tree).unwrap();
        std::os::unix::fs::symlink(&outside, tree.join("folder")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret"), tree.join("file")).unwrap();
        std::os::unix::fs::symlink(&tree, dir.join("named")).unwrap();
        let mut root = Folder::open(&tree).unwrap();
        let kinds: Vec<Kind> = root
            .entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.kind)
            .collect();
        assert_eq!(kinds, [Kind::Link, Kind::Link]);
        assert!(matches!(
            root.folder(OsStr::new("folder")),
            Err(Refused::Link)
        ));
        assert!(matches!(root.file(OsStr::new("file")), Err(Refused::Link)));
        assert!(matches!(Folder::open(&dir.join("named")), Err(Refused::Link)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
