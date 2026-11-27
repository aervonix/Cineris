//! Whether the managed configuration file is an administrator's
//! (`docs/architecture/08-configuration.md`, "The managed file is an
//! administrator's"; since the signed approvals step).
//!
//! The managed file outranks the user's own: its rules cannot be weakened
//! by the user's, and its locks hold against every layer above it. It is
//! the one layer the model's code, which runs with the user's rights, is
//! meant not to be able to write. A managed file the user could change is
//! no administrator's file, so [`check`] looks at who could change it
//! before it is read, and a load does not use one that the user could: it
//! says so, with a warning, and goes on without it.
//!
//! - **Linux:** the file, and every folder the system looks it up through
//!   (`/`, `/etc`, `/etc/cineris`, and the folders of any link on the way,
//!   whose targets are walked as the system walks them), must be owned by
//!   root and not writable by its group or by others. A folder that others
//!   may write and that has the sticky bit (`/tmp`'s `1777`, the Nix
//!   store's `1775`) passes when the name looked up in it is root's: the
//!   sticky bit keeps anyone but root and that name's owner from removing
//!   or renaming it. A link's own mode means nothing and is not asked; the
//!   folder it is in is.
//! - **Windows:** the file and its folder must each be owned by SYSTEM, the
//!   Administrators group or TrustedInstaller, be no link or junction, and
//!   be out of the user's reach, asked of Windows's own access check with
//!   the process's token and the Administrators group set aside
//!   ([`cineris_base::private::standing_without_administrators`]): the
//!   user may not write to the file, append to it, delete it, or change its
//!   access list or its owner; nor delete the folder or a file in it, or
//!   change its access list or owner. **Adding a file to the folder is not
//!   refused:** a folder an administrator makes under `C:\ProgramData`
//!   lets every user add files by default (the folder's inherited entry
//!   for `Users`), and a file the user adds there is the user's, which the
//!   file's own check refuses; the one file read cannot be replaced
//!   without deleting it. The folders above it are the system's own.
//! - **A file that is not there** is no managed layer, as before, whoever
//!   could have put one there.
//!
//! **What this does not close.** An administrator can change the file, and
//! the model's code becomes one where the person's account lets a program
//! of theirs elevate without the person: on Linux under WSL with Windows
//! interop on (the default), `wsl.exe -u root` gives any program of the
//! user's a root shell with no password, and `/etc` is then the model's to
//! write; a `sudo` rule with `NOPASSWD` is the same. On a Windows
//! administrator's account with UAC at its default level, public ways to
//! elevate without a prompt are known, and Microsoft does not treat UAC as
//! a boundary. Both are residuals of the design, not of this check.
//!
//! The facts are read here, and judged by [`unix_refusal`] and
//! [`windows_refusal`], which take them as values, so that each system's
//! rules are tested on every system.

#![allow(
    clippy::disallowed_methods,
    reason = "a few `lstat` calls, or on Windows two reads of an access list, at a load, which runs at start-up and on the blocking pool"
)]

use std::io;
use std::path::{Path, PathBuf};

/// What [`check`] found.
#[derive(Debug)]
pub(crate) enum Verdict {
    /// There is no file: no managed layer, as before.
    Missing,
    /// Only an administrator can change it: it is read.
    Administrators,
    /// The user could change it, or what it is in, or it could not be
    /// looked at where that could be the user's doing. It is not used, and
    /// the load says why, a sentence.
    NotAdministrators(String),
    /// Every folder on the way is an administrator's, and the file could not
    /// be looked at: reported as an error, as a managed file that cannot be
    /// read is, since skipping it would drop an administrator's locks.
    /// Linux only: on Windows a file the user made can deny the user the
    /// right to read it, so what cannot be looked at is not used.
    #[cfg_attr(
        windows,
        allow(dead_code, reason = "on Windows what cannot be looked at is not used")
    )]
    Unreadable(io::Error),
}

/// Whether the managed file at `file` is an administrator's. It looks at
/// the file system and blocks: a few `lstat` calls on Linux, and on Windows
/// a read of the file's and its folder's owners and access lists.
pub(crate) fn check(file: &Path) -> Verdict {
    #[cfg(unix)]
    {
        unix::check(file)
    }
    #[cfg(windows)]
    {
        windows::check(file)
    }
}

/// What a name on the way to the file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(unix),
    allow(dead_code, reason = "Linux's rule is tested on every OS")
)]
pub(crate) enum Kind {
    /// A folder the walk goes through.
    Folder,
    /// A link, whose target is walked next.
    Link,
    /// The file itself (or whatever is at its name).
    File,
}

/// One name the system looks up on the way to the managed file on Linux,
/// as `lstat` found it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(unix),
    allow(dead_code, reason = "Linux's rule is tested on every OS")
)]
pub(crate) struct UnixEntry {
    /// Where it is.
    pub(crate) path: PathBuf,
    /// What it is.
    pub(crate) kind: Kind,
    /// Its owner's user id.
    pub(crate) owner: u32,
    /// Its mode: the permission bits and the sticky bit.
    pub(crate) mode: u32,
}

/// The first name on the way that the user could change, and why; `None`
/// when only root can change any. `entries` are in the order the system
/// looks them up, the file last.
#[cfg_attr(
    not(unix),
    allow(dead_code, reason = "Linux's rule is tested on every OS")
)]
pub(crate) fn unix_refusal(entries: &[UnixEntry]) -> Option<String> {
    /// Written by the group, by others.
    const GROUP_WRITES: u32 = 0o020;
    const OTHERS_WRITE: u32 = 0o002;
    /// The sticky bit: in a folder, only a name's owner (and root, and the
    /// folder's owner) may remove or rename it.
    const STICKY: u32 = 0o1000;

    for (index, entry) in entries.iter().enumerate() {
        let shown = entry.path.display();
        if entry.kind == Kind::Link {
            continue;
        }
        if entry.owner != 0 {
            return Some(format!(
                "`{shown}` is owned by user {}, not by root",
                entry.owner
            ));
        }
        let (group, others) = (
            entry.mode & GROUP_WRITES != 0,
            entry.mode & OTHERS_WRITE != 0,
        );
        if !group && !others {
            continue;
        }
        let guarded = entry.kind == Kind::Folder
            && entry.mode & STICKY != 0
            && entries.get(index + 1).is_some_and(|next| next.owner == 0);
        if !guarded {
            let who = match (group, others) {
                (true, true) => "its group and by others",
                (true, false) => "its group",
                _ => "others",
            };
            return Some(format!("`{shown}` can be written by {who}"));
        }
    }
    None
}

/// Whether a Windows entry is the managed file or its folder: they are
/// asked different rights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "Windows's rule is tested on every OS")
)]
pub(crate) enum Role {
    /// The folder the file is in.
    Folder,
    /// The file.
    File,
}

/// The managed file or its folder on Windows, as Windows reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "Windows's rule is tested on every OS")
)]
pub(crate) struct WindowsEntry {
    /// Where it is.
    pub(crate) path: PathBuf,
    /// Whether it is a link or a junction.
    pub(crate) link: bool,
    /// Its owner, a security identifier in text.
    pub(crate) owner: String,
    /// What the user may do to it with the Administrators group set aside,
    /// an access mask.
    pub(crate) granted: u32,
}

/// The rights of Windows's access masks this check asks about. They are
/// the system's own values, written out so that the rule is tested on
/// every system.
pub(crate) mod rights {
    /// Write to a file; add a file to a folder.
    pub(crate) const WRITE_DATA: u32 = 0x0002;
    /// Append to a file; add a folder to a folder.
    pub(crate) const APPEND_DATA: u32 = 0x0004;
    /// Delete anything in a folder, whatever its own access list says.
    pub(crate) const DELETE_CHILD: u32 = 0x0040;
    /// Delete the object, or rename it.
    pub(crate) const DELETE: u32 = 0x0001_0000;
    /// Change the object's access list.
    pub(crate) const WRITE_DAC: u32 = 0x0004_0000;
    /// Change the object's owner.
    pub(crate) const WRITE_OWNER: u32 = 0x0008_0000;
}

/// Why the user may not have the managed file or its folder on Windows;
/// `None` when it is an administrator's.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "Windows's rule is tested on every OS")
)]
pub(crate) fn windows_refusal(entry: &WindowsEntry, role: Role) -> Option<String> {
    /// SYSTEM, the Administrators group, and TrustedInstaller, which owns
    /// the system's own files.
    const ADMINISTRATORS: [&str; 3] = [
        "S-1-5-18",
        "S-1-5-32-544",
        "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
    ];
    let shown = entry.path.display();
    if entry.link {
        return Some(format!(
            "`{shown}` is a link or a junction, which is not followed"
        ));
    }
    if !ADMINISTRATORS
        .iter()
        .any(|owner| owner.eq_ignore_ascii_case(&entry.owner))
    {
        let owner = if entry.owner.is_empty() {
            "no one"
        } else {
            &entry.owner
        };
        return Some(format!(
            "`{shown}` is owned by {owner}, not by SYSTEM, the Administrators group or \
             TrustedInstaller"
        ));
    }
    let asked: &[(u32, &str)] = match role {
        Role::File => &[
            (rights::WRITE_DATA, "write to"),
            (rights::APPEND_DATA, "append to"),
            (rights::DELETE, "delete"),
            (rights::WRITE_DAC, "change the access list of"),
            (rights::WRITE_OWNER, "change the owner of"),
        ],
        Role::Folder => &[
            (rights::DELETE_CHILD, "delete files in"),
            (rights::DELETE, "delete or rename"),
            (rights::WRITE_DAC, "change the access list of"),
            (rights::WRITE_OWNER, "change the owner of"),
        ],
    };
    asked
        .iter()
        .find(|(right, _)| entry.granted & right != 0)
        .map(|(_, what)| format!("you may {what} `{shown}` without an administrator's rights"))
}

#[cfg(unix)]
mod unix {
    use std::ffi::OsString;
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Component, Path, PathBuf};

    use super::{Kind, UnixEntry, Verdict, unix_refusal};

    /// The most links one walk follows, as Linux's own limit.
    const MOST_LINKS: usize = 40;

    pub(super) fn check(file: &Path) -> Verdict {
        let (entries, stopped) = walk(file);
        match stopped {
            Some(error) if error.kind() == io::ErrorKind::NotFound => Verdict::Missing,
            Some(error) => match unix_refusal(&entries) {
                Some(why) => Verdict::NotAdministrators(why),
                None => Verdict::Unreadable(error),
            },
            None => match unix_refusal(&entries) {
                Some(why) => Verdict::NotAdministrators(why),
                None => Verdict::Administrators,
            },
        }
    }

    /// The names the system looks up on the way to `file`, in order: `/`,
    /// each folder, each link (whose target is walked next, from `/` or
    /// from the link's folder), and the file. It stops at the first name
    /// that cannot be looked at, and gives what it found before it with the
    /// error.
    pub(super) fn walk(file: &Path) -> (Vec<UnixEntry>, Option<io::Error>) {
        let mut entries = Vec::new();
        if !file.is_absolute() {
            return (
                entries,
                Some(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "the managed file's path is not absolute",
                )),
            );
        }
        let mut at = PathBuf::from("/");
        match std::fs::symlink_metadata(&at) {
            Ok(found) => entries.push(entry(&at, &found, Kind::Folder)),
            Err(error) => return (entries, Some(error)),
        }
        let mut pending = Vec::new();
        push_names(&mut pending, file);
        let mut links = 0;
        while let Some(name) = pending.pop() {
            if name == ".." {
                // `at` holds no link, so its parent is the folder above.
                at.pop();
                continue;
            }
            let next = at.join(&name);
            let found = match std::fs::symlink_metadata(&next) {
                Ok(found) => found,
                Err(error) => return (entries, Some(error)),
            };
            if found.file_type().is_symlink() {
                entries.push(entry(&next, &found, Kind::Link));
                links += 1;
                if links > MOST_LINKS {
                    return (
                        entries,
                        Some(io::Error::other(format!(
                            "more than {MOST_LINKS} links on the way"
                        ))),
                    );
                }
                let target = match std::fs::read_link(&next) {
                    Ok(target) => target,
                    Err(error) => return (entries, Some(error)),
                };
                if target.is_absolute() {
                    at = PathBuf::from("/");
                }
                push_names(&mut pending, &target);
                continue;
            }
            let kind = if found.is_dir() {
                Kind::Folder
            } else {
                Kind::File
            };
            entries.push(entry(&next, &found, kind));
            if !pending.is_empty() && kind != Kind::Folder {
                return (
                    entries,
                    Some(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        format!("`{}` is not a folder", next.display()),
                    )),
                );
            }
            at = next;
        }
        (entries, None)
    }

    fn entry(path: &Path, found: &std::fs::Metadata, kind: Kind) -> UnixEntry {
        UnixEntry {
            path: path.to_path_buf(),
            kind,
            owner: found.uid(),
            mode: found.mode() & 0o7777,
        }
    }

    /// Puts the names of `path` on the stack, its first name on top.
    fn push_names(pending: &mut Vec<OsString>, path: &Path) {
        let names: Vec<OsString> = path
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name.to_os_string()),
                Component::ParentDir => Some(OsString::from("..")),
                Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
            })
            .collect();
        pending.extend(names.into_iter().rev());
    }
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::path::Path;

    use super::{Role, Verdict, WindowsEntry, windows_refusal};

    pub(super) fn check(file: &Path) -> Verdict {
        let Some(folder) = file.parent() else {
            return Verdict::NotAdministrators(format!("`{}` is in no folder", file.display()));
        };
        // The folder first, and its own entry: a link there could lead to
        // a network share, and following it would send the account's
        // credentials (`docs/security.md`, section 7).
        let folder = match entry(folder) {
            Ok(Some(folder)) => folder,
            Ok(None) => return Verdict::Missing,
            Err(why) => return Verdict::NotAdministrators(why),
        };
        if let Some(why) = windows_refusal(&folder, Role::Folder) {
            return Verdict::NotAdministrators(why);
        }
        let file = match entry(file) {
            Ok(Some(file)) => file,
            Ok(None) => return Verdict::Missing,
            Err(why) => return Verdict::NotAdministrators(why),
        };
        match windows_refusal(&file, Role::File) {
            Some(why) => Verdict::NotAdministrators(why),
            None => Verdict::Administrators,
        }
    }

    /// The entry at `path`, its own and not a link's target. `None` when
    /// nothing is there. An error is a sentence: what could not be checked
    /// is not used, since a file the user made can deny the user the right
    /// to read its owner.
    fn entry(path: &Path) -> Result<Option<WindowsEntry>, String> {
        let not_checked =
            |error: io::Error| format!("`{}` could not be checked: {error}", path.display());
        let found = match std::fs::symlink_metadata(path) {
            Ok(found) => found,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(not_checked(error)),
        };
        let link = found.file_type().is_symlink();
        let standing =
            cineris_base::private::standing_without_administrators(path).map_err(not_checked)?;
        Ok(Some(WindowsEntry {
            path: path.to_path_buf(),
            link,
            owner: standing.owner,
            granted: standing.granted,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix(path: &str, kind: Kind, owner: u32, mode: u32) -> UnixEntry {
        UnixEntry {
            path: PathBuf::from(path),
            kind,
            owner,
            mode,
        }
    }

    /// `/`, `/etc`, `/etc/cineris` and the file, as a distribution leaves
    /// them and an administrator writes the file.
    fn usual() -> Vec<UnixEntry> {
        vec![
            unix("/", Kind::Folder, 0, 0o755),
            unix("/etc", Kind::Folder, 0, 0o755),
            unix("/etc/cineris", Kind::Folder, 0, 0o755),
            unix("/etc/cineris/config.toml", Kind::File, 0, 0o644),
        ]
    }

    #[test]
    fn on_linux_root_owns_the_file_and_every_folder_and_no_one_else_writes_them() {
        assert_eq!(unix_refusal(&usual()), None);
        // Read-only for everyone is fine too, and so is a file of root's
        // that root alone may read: reading it is not the check's business.
        let mut strict = usual();
        strict[3].mode = 0o400;
        strict[2].mode = 0o700;
        assert_eq!(unix_refusal(&strict), None);

        // Anyone else's file or folder, at any place on the way.
        for index in 0..4 {
            let mut theirs = usual();
            theirs[index].owner = 1000;
            let why = unix_refusal(&theirs).unwrap();
            assert!(
                why.contains(&format!(
                    "`{}` is owned by user 1000",
                    theirs[index].path.display()
                )),
                "{why}"
            );
        }
        // Written by the group or by others, at any place on the way.
        for index in 0..4 {
            for (mode, who) in [
                (0o775, "its group"),
                (0o757, "others"),
                (0o777, "its group and by others"),
                (0o664, "its group"),
            ] {
                let mut open = usual();
                open[index].mode = mode;
                let why = unix_refusal(&open).unwrap();
                assert!(why.ends_with(&format!("can be written by {who}")), "{why}");
                assert!(
                    why.contains(&open[index].path.display().to_string()),
                    "{why}"
                );
            }
        }
    }

    /// The user's own file is refused even where everything above it is
    /// root's, and root's own file passes when root runs: the rule is
    /// about the owner, not about who runs.
    #[test]
    fn on_linux_the_users_own_file_is_refused_and_roots_passes() {
        let mut mine = usual();
        mine[3].owner = 1000;
        mine[3].mode = 0o600;
        assert!(unix_refusal(&mine).unwrap().contains("config.toml"));
        // A folder of the user's that holds root's file: the user may
        // replace the file.
        let mut folder = usual();
        folder[2].owner = 1000;
        assert!(unix_refusal(&folder).unwrap().contains("`/etc/cineris`"));
    }

    /// A sticky folder that others may write (`/tmp`, the Nix store) keeps
    /// a name of root's in place, and so passes when the name looked up in
    /// it is root's; not when the name is someone else's, and not without
    /// the bit.
    #[test]
    fn on_linux_a_sticky_folder_passes_only_over_a_name_of_roots() {
        let store = vec![
            unix("/", Kind::Folder, 0, 0o755),
            unix("/etc", Kind::Folder, 0, 0o755),
            unix("/etc/cineris", Kind::Link, 0, 0o777),
            unix("/nix", Kind::Folder, 0, 0o755),
            unix("/nix/store", Kind::Folder, 0, 0o1775),
            unix("/nix/store/abc-cineris", Kind::Folder, 0, 0o555),
            unix("/nix/store/abc-cineris/config.toml", Kind::File, 0, 0o444),
        ];
        assert_eq!(unix_refusal(&store), None);

        let mut planted = store.clone();
        planted[5].owner = 1000;
        // The sticky folder no longer guards it, and is named first.
        assert!(
            unix_refusal(&planted)
                .unwrap()
                .contains("`/nix/store` can be written by its group")
        );
        planted[4].mode = 0o755;
        assert!(
            unix_refusal(&planted)
                .unwrap()
                .contains("`/nix/store/abc-cineris` is owned by user 1000")
        );

        let mut not_sticky = store.clone();
        not_sticky[4].mode = 0o775;
        assert!(
            unix_refusal(&not_sticky)
                .unwrap()
                .contains("`/nix/store` can be written by its group")
        );

        // The bit guards names in the folder, and a file with it is still
        // written by others.
        let mut sticky_file = usual();
        sticky_file[3].mode = 0o1666;
        assert!(unix_refusal(&sticky_file).is_some());
        // Nor does it guard what is not there to look up.
        let mut last = usual();
        last.truncate(3);
        last[2].mode = 0o1777;
        assert!(unix_refusal(&last).is_some());
    }

    /// A link's own mode (always `0777` on Linux) and owner mean nothing:
    /// its folder, and where it leads, are what is asked.
    #[test]
    fn on_linux_a_links_own_mode_is_not_asked() {
        let linked = vec![
            unix("/", Kind::Folder, 0, 0o755),
            unix("/etc", Kind::Folder, 0, 0o755),
            unix("/etc/cineris", Kind::Link, 1000, 0o777),
            unix("/opt", Kind::Folder, 0, 0o755),
            unix("/opt/cineris", Kind::Folder, 0, 0o755),
            unix("/opt/cineris/config.toml", Kind::File, 0, 0o644),
        ];
        assert_eq!(unix_refusal(&linked), None);
        // Where it leads is the user's.
        let mut theirs = linked.clone();
        theirs[4].owner = 1000;
        assert!(unix_refusal(&theirs).unwrap().contains("`/opt/cineris`"));
    }

    fn windows(path: &str, owner: &str, granted: u32) -> WindowsEntry {
        WindowsEntry {
            path: PathBuf::from(path),
            link: false,
            owner: owner.to_owned(),
            granted,
        }
    }

    /// What a user who is not an administrator may do to a folder and a
    /// file an administrator made under `C:\ProgramData`, as Windows
    /// leaves them: read and run, and in the folder add files and folders
    /// (the inherited entry for `Users`). Synchronising and reading the
    /// access list are in both.
    const READ: u32 = 0x0012_00A9;
    const FOLDER_DEFAULT: u32 = READ | rights::WRITE_DATA | rights::APPEND_DATA;

    #[test]
    fn on_windows_an_administrators_file_in_an_administrators_folder_passes() {
        for owner in [
            "S-1-5-18",
            "S-1-5-32-544",
            "s-1-5-32-544",
            "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
        ] {
            let folder = windows(r"C:\ProgramData\cineris", owner, FOLDER_DEFAULT);
            let file = windows(r"C:\ProgramData\cineris\config.toml", owner, READ);
            assert_eq!(windows_refusal(&folder, Role::Folder), None, "{owner}");
            assert_eq!(windows_refusal(&file, Role::File), None, "{owner}");
        }
        // Nothing granted at all passes as well.
        let file = windows(r"C:\ProgramData\cineris\config.toml", "S-1-5-18", 0);
        assert_eq!(windows_refusal(&file, Role::File), None);
    }

    /// A folder the user made under `C:\ProgramData` (its default access
    /// list lets them) is the user's: refused by its owner, whatever it
    /// grants. So is any owner but the three.
    #[test]
    fn on_windows_a_folder_or_file_of_anyone_else_is_refused() {
        for owner in [
            "S-1-5-21-1-2-3-1001",
            "S-1-5-32-545",
            "S-1-1-0",
            "S-1-5-32-5440",
            "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478465",
            "",
        ] {
            for role in [Role::Folder, Role::File] {
                let entry = windows(r"C:\ProgramData\cineris", owner, READ);
                let why = windows_refusal(&entry, role).unwrap();
                assert!(why.contains("is owned by"), "{owner}: {why}");
            }
        }
        let mut link = windows(r"C:\ProgramData\cineris", "S-1-5-18", READ);
        link.link = true;
        assert!(
            windows_refusal(&link, Role::Folder)
                .unwrap()
                .contains("is a link or a junction")
        );
    }

    /// Each right that lets the user change the file, or remove it from its
    /// folder, refuses it, alone.
    #[test]
    fn on_windows_each_right_to_change_the_file_or_its_folder_is_refused() {
        for (right, what) in [
            (rights::WRITE_DATA, "write to"),
            (rights::APPEND_DATA, "append to"),
            (rights::DELETE, "delete"),
            (rights::WRITE_DAC, "change the access list of"),
            (rights::WRITE_OWNER, "change the owner of"),
        ] {
            let file = windows(
                r"C:\ProgramData\cineris\config.toml",
                "S-1-5-32-544",
                READ | right,
            );
            let why = windows_refusal(&file, Role::File).unwrap();
            assert!(why.starts_with(&format!("you may {what} `")), "{why}");
        }
        for (right, what) in [
            (rights::DELETE_CHILD, "delete files in"),
            (rights::DELETE, "delete or rename"),
            (rights::WRITE_DAC, "change the access list of"),
            (rights::WRITE_OWNER, "change the owner of"),
        ] {
            let folder = windows(
                r"C:\ProgramData\cineris",
                "S-1-5-32-544",
                FOLDER_DEFAULT | right,
            );
            let why = windows_refusal(&folder, Role::Folder).unwrap();
            assert!(why.starts_with(&format!("you may {what} `")), "{why}");
        }
        // Full control is refused for either.
        let all = windows(r"C:\ProgramData\cineris", "S-1-5-18", 0x001F_01FF);
        assert!(windows_refusal(&all, Role::Folder).is_some());
        assert!(windows_refusal(&all, Role::File).is_some());
    }

    /// The walk follows each link as the system does, and lists every
    /// folder it goes through and every link, so that a folder only a
    /// link leads to is asked about too.
    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_the_walk_lists_every_folder_and_link_on_the_way() {
        #![allow(
            clippy::disallowed_methods,
            reason = "a test of the file system; a folder of its own"
        )]
        let scratch = cineris_testkit::scratch::Scratch::outside("cineris-config-managed", "walk");
        let base = scratch.path().to_path_buf();
        std::fs::create_dir_all(base.join("real").join("inner")).unwrap();
        std::fs::write(base.join("real").join("inner").join("config.toml"), "").unwrap();
        // A relative link to a folder, and an absolute link to the file.
        std::os::unix::fs::symlink("real", base.join("near")).unwrap();
        std::os::unix::fs::symlink(
            base.join("near").join("inner").join("config.toml"),
            base.join("far.toml"),
        )
        .unwrap();

        let (entries, stopped) = unix::walk(&base.join("far.toml"));
        assert!(stopped.is_none(), "{stopped:?}");
        let tail: Vec<(PathBuf, Kind)> = entries
            .iter()
            .filter(|entry| entry.path.starts_with(&base) && entry.path != base)
            .map(|entry| (entry.path.clone(), entry.kind))
            .collect();
        assert_eq!(
            tail,
            [
                (base.join("far.toml"), Kind::Link),
                (base.join("near"), Kind::Link),
                (base.join("real"), Kind::Folder),
                (base.join("real").join("inner"), Kind::Folder),
                (
                    base.join("real").join("inner").join("config.toml"),
                    Kind::File
                ),
            ]
        );
        assert_eq!(entries[0].path, Path::new("/"));
        // The folders above the scratch folder are walked twice, once for
        // the path and once for the absolute link.
        assert_eq!(entries.iter().filter(|entry| entry.path == base).count(), 2);

        // Not there: the walk stops, and says so.
        let (_, stopped) = unix::walk(&base.join("real").join("missing.toml"));
        assert_eq!(stopped.unwrap().kind(), io::ErrorKind::NotFound);
        assert!(matches!(
            check(&base.join("real").join("missing.toml")),
            Verdict::Missing
        ));
        // Through a file: not a folder.
        let (_, stopped) = unix::walk(
            &base
                .join("real")
                .join("inner")
                .join("config.toml")
                .join("x"),
        );
        assert_eq!(stopped.unwrap().kind(), io::ErrorKind::NotADirectory);
        // A loop of links.
        std::os::unix::fs::symlink("loop-b", base.join("loop-a")).unwrap();
        std::os::unix::fs::symlink("loop-a", base.join("loop-b")).unwrap();
        let (_, stopped) = unix::walk(&base.join("loop-a"));
        assert!(stopped.unwrap().to_string().contains("links on the way"));
        // A relative path is not walked.
        let (_, stopped) = unix::walk(Path::new("etc/cineris/config.toml"));
        assert_eq!(stopped.unwrap().kind(), io::ErrorKind::InvalidInput);
    }

    /// The file the test made is the test user's, and is refused; a file of
    /// root's that every Linux system has, in root's folders, passes.
    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_a_file_of_the_users_is_refused_and_one_of_roots_passes() {
        #![allow(
            clippy::disallowed_methods,
            reason = "a test of the file system; a folder of its own"
        )]
        let scratch = cineris_testkit::scratch::Scratch::outside("cineris-config-managed", "users");
        let file = scratch.join("config.toml");
        std::fs::write(&file, "").unwrap();
        if std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&file).unwrap()) == 0 {
            // Run as root, every file the test makes is root's.
            return;
        }
        match check(&file) {
            // Named by the first name on the way the user could change: a
            // folder of theirs, or a shared temporary folder whose sticky
            // bit guards nothing of root's below it.
            Verdict::NotAdministrators(why) => {
                assert!(
                    why.contains(", not by root") || why.contains("can be written by"),
                    "{why}"
                );
            }
            other => panic!("the user's file was not refused: {other:?}"),
        }
        assert!(matches!(
            check(Path::new("/etc/passwd")),
            Verdict::Administrators
        ));
    }
}
