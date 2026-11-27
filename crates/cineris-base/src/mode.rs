//! A file's permission bits, as a put-back keeps them
//! (`docs/architecture/07-sessions-and-context.md`, "Snapshots and undo").
//!
//! A rewind that creates a file again must create it as the person had it:
//! a script executable, a private file private (found by the review of
//! step 25, whose put-back gave every file it created the process's
//! default mode). [`FileMode`] is what is kept of a file's permissions for
//! that, and [`AtomicWriteOptions::mode`](crate::AtomicWriteOptions::mode)
//! is how a write puts it back.
//!
//! | | What is kept | Where it comes from |
//! |---|---|---|
//! | Linux | The nine permission bits (`rwx` for the owner, the group and others), never the set-user-id, set-group-id or sticky bit | The open file's own metadata ([`FileMode::of`]), or, for a file read from a commit, git's execute bit and nothing for the group or others ([`FileMode::from_git`]) |
//! | Windows | Whether the file is private: [`FileMode::PRIVATE`] for one whose own access list keeps it from everyone but the current user (`private::AccessList::is_kept_from_others`, Windows only), nothing for any other | The open file's access list, read through its handle |
//!
//! These functions block. Async code calls them on the blocking pool.

#![allow(
    clippy::disallowed_types,
    reason = "a file's mode is asked of its open handle, on the blocking pool"
)]

use std::fmt;
use std::fs::File;
use std::io;

/// The nine permission bits of a file: `0o755`, `0o600`.
///
/// Only those nine are ever held. A set-user-id, set-group-id or sticky bit
/// given to [`from_bits`](Self::from_bits) is dropped, so a mode read from
/// a session log, which a person can edit, can never make a program run as
/// someone else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileMode(u32);

impl FileMode {
    /// Read and write for the owner, and nothing for anyone else.
    pub const PRIVATE: Self = Self(0o600);

    /// The mode of these bits, less any outside the nine permission bits.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits & 0o777)
    }

    /// The nine permission bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether neither the group nor others have any right.
    #[must_use]
    pub const fn is_private(self) -> bool {
        self.0 & 0o077 == 0
    }

    /// The bits both modes have: never wider than either.
    #[must_use]
    pub const fn narrowed_to(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The mode in octal digits, as `chmod` takes it: `755`, `600`.
    #[must_use]
    pub fn to_octal(self) -> String {
        format!("{:03o}", self.0)
    }

    /// A mode written in octal digits, as [`to_octal`](Self::to_octal)
    /// writes it: one to four digits, a leading set-id or sticky digit
    /// dropped. `None` for anything else.
    #[must_use]
    pub fn from_octal(text: &str) -> Option<Self> {
        if text.is_empty()
            || text.len() > 4
            || !text.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
        {
            return None;
        }
        u32::from_str_radix(text, 8).ok().map(Self::from_bits)
    }

    /// What git's entry mode for a regular file says of the file: `700`
    /// for `100755`, an executable, and `600` for `100644`. Git keeps the
    /// execute bit and nothing else, so what the group and others may do
    /// is not known, and is not given: a file the person had made private
    /// (`chmod 600`, which git does not see) must not come back readable by
    /// others (the full review of the step 25 fixes, which found `777` and
    /// `666`, a checkout's modes, making such a file `0644` again). `None`
    /// for any other entry (a link, a submodule, a tree).
    #[must_use]
    pub fn from_git(mode: &str) -> Option<Self> {
        match mode {
            "100755" => Some(Self(0o700)),
            "100644" => Some(Self(0o600)),
            _ => None,
        }
    }

    /// The same mode with no execute bit: for git's entry mode where the
    /// repository says it does not keep the execute bit
    /// (`core.fileMode=false`), and its `100755` may be any file's.
    #[must_use]
    pub const fn without_execute(self) -> Self {
        Self(self.0 & !0o111)
    }

    /// The mode of an open file, asked of its handle and never of its name.
    ///
    /// On Linux its nine permission bits. On Windows [`Self::PRIVATE`] for a
    /// file whose own access list keeps it from everyone but the current
    /// user, and `None` for any other: such a file takes its folder's rules
    /// when it is made again, as it did when it was first made. The handle
    /// must have the right to read the file's permissions, which a handle
    /// opened for reading has.
    ///
    /// # Errors
    ///
    /// When the operating system does not report them.
    pub fn of(file: &File) -> io::Result<Option<Self>> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Ok(Some(Self::from_bits(file.metadata()?.permissions().mode())))
        }
        #[cfg(windows)]
        {
            Ok(crate::private::access_list_of(file)?
                .is_kept_from_others()
                .then_some(Self::PRIVATE))
        }
    }
}

impl fmt::Display for FileMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03o}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::FileMode;

    #[test]
    fn only_the_nine_permission_bits_are_held() {
        assert_eq!(FileMode::from_bits(0o104_755).bits(), 0o755);
        assert_eq!(FileMode::from_bits(0o4755).bits(), 0o755, "set-user-id");
        assert_eq!(FileMode::from_bits(0o2755).bits(), 0o755, "set-group-id");
        assert_eq!(FileMode::from_bits(0o1777).bits(), 0o777, "sticky");
        assert_eq!(
            FileMode::from_octal("4755"),
            Some(FileMode::from_bits(0o755))
        );
    }

    #[test]
    fn a_mode_is_written_and_read_in_octal() {
        for bits in [0o755, 0o644, 0o600, 0o000, 0o007, 0o777] {
            let mode = FileMode::from_bits(bits);
            assert_eq!(mode.to_octal().len(), 3, "{mode}");
            assert_eq!(FileMode::from_octal(&mode.to_octal()), Some(mode));
        }
        assert_eq!(FileMode::from_bits(0o7).to_octal(), "007");
        for bad in ["", "8", "75a", "12345", "-1", "+7", " 755", "0x1ed"] {
            assert_eq!(FileMode::from_octal(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn git_keeps_the_execute_bit_alone() {
        assert_eq!(
            FileMode::from_git("100755"),
            Some(FileMode::from_bits(0o700))
        );
        assert_eq!(
            FileMode::from_git("100644"),
            Some(FileMode::from_bits(0o600))
        );
        assert_eq!(
            FileMode::from_bits(0o755).without_execute(),
            FileMode::from_bits(0o644)
        );
        for other in ["120000", "160000", "040000", "100664", ""] {
            assert_eq!(FileMode::from_git(other), None, "{other:?}");
        }
    }

    #[test]
    fn private_is_nothing_for_the_group_or_others() {
        assert!(FileMode::PRIVATE.is_private());
        assert!(FileMode::from_bits(0o700).is_private());
        assert!(FileMode::from_bits(0o400).is_private());
        assert!(!FileMode::from_bits(0o640).is_private());
        assert!(!FileMode::from_bits(0o601).is_private());
        assert_eq!(
            FileMode::from_bits(0o755).narrowed_to(FileMode::PRIVATE),
            FileMode::from_bits(0o600)
        );
    }
}
