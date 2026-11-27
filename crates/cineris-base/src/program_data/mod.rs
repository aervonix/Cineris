//! The folder Windows keeps every user's program data in, `C:\ProgramData`
//! on most machines, as the system records it and never as a variable of
//! the process says (since the signed approvals step;
//! `docs/architecture/08-configuration.md`, "The managed file is an
//! administrator's").
//!
//! The managed configuration file lives under it, and that file is the
//! layer the model's code is meant not to be able to write. The
//! `ProgramData` variable is the process's own: `setx ProgramData …` puts a
//! value of the user's choosing into every later process of theirs, so a
//! folder read from it is a folder the user chose.
//!
//! **Where it is read from.** The system records the folder in the
//! registry, in the machine's part that only an administrator may change:
//! the value `ProgramData` of
//! `HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList`,
//! which the system's own known folder (`FOLDERID_ProgramData`) and the
//! `ProgramData` and `ALLUSERSPROFILE` variables of every logon are made
//! from. It is stored as `%SystemDrive%\ProgramData`. [`expand`] puts the
//! system's drive in its place, read from the system's Windows directory
//! (`GetSystemWindowsDirectoryW`, which no variable moves), and refuses any
//! other variable, so that nothing of the process's environment reaches the
//! answer.
//!
//! **Why not `SHGetKnownFolderPath`.** It is the documented call for the
//! same folder, and it lives in `shell32.dll` and frees its answer with
//! `combase.dll`'s `CoTaskMemFree`: two large libraries that the binary
//! does not load today, and that a static import would load at every start
//! of the process, `cineris --version` included. Whether it expands
//! `%SystemDrive%` from the process's own variables is not documented
//! either. The registry's value and the Windows directory come from
//! `advapi32.dll` and `kernel32.dll`, which every start loads already.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::folder;

/// The stored folder with its variables put in, from the system's Windows
/// directory (`C:\Windows`): `%SystemDrive%` is its drive (`C:`), and
/// `%SystemRoot%` and `%windir%` are the directory itself. A name is read
/// as Windows reads one, whatever its case. `None` when the text names any
/// other variable, has a `%` that closes none, or does not end as an
/// absolute path on a drive: the folder is then not known.
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "Windows reads the folder; the rule is tested on every OS"
    )
)]
pub(crate) fn expand(stored: &str, windows_directory: &str) -> Option<String> {
    let drive = windows_directory.get(..2).filter(|drive| {
        let bytes = drive.as_bytes();
        bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
    })?;
    let mut expanded = String::with_capacity(stored.len() + windows_directory.len());
    let mut rest = stored;
    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        let name = &after[..end];
        if name.eq_ignore_ascii_case("SystemDrive") {
            expanded.push_str(drive);
        } else if name.eq_ignore_ascii_case("SystemRoot") || name.eq_ignore_ascii_case("windir") {
            expanded.push_str(windows_directory);
        } else {
            return None;
        }
        rest = &after[end + 1..];
    }
    expanded.push_str(rest);
    let bytes = expanded.as_bytes();
    let absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    absolute.then_some(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_folder_takes_the_systems_drive_and_no_variable_else() {
        assert_eq!(
            expand(r"%SystemDrive%\ProgramData", r"C:\WINDOWS").as_deref(),
            Some(r"C:\ProgramData")
        );
        assert_eq!(
            expand(r"%systemdrive%\ProgramData", r"D:\Windows").as_deref(),
            Some(r"D:\ProgramData")
        );
        assert_eq!(
            expand(r"%SystemRoot%\..\Shared", r"C:\Windows").as_deref(),
            Some(r"C:\Windows\..\Shared")
        );
        assert_eq!(
            expand(r"E:\Data", r"C:\Windows").as_deref(),
            Some(r"E:\Data")
        );
        // The variables a user can set are never read: the folder is not
        // known, and there is no managed file.
        for refused in [
            r"%ProgramData%",
            r"%ALLUSERSPROFILE%\x",
            r"%USERPROFILE%\x",
            r"%SystemDrive\ProgramData",
            r"ProgramData",
            r"\\server\share\ProgramData",
            "",
        ] {
            assert_eq!(expand(refused, r"C:\Windows"), None, "{refused:?}");
        }
        // Nor is a Windows directory that is not on a drive.
        assert_eq!(expand(r"%SystemDrive%\ProgramData", r"\\server\w"), None);
        assert_eq!(expand(r"%SystemDrive%\ProgramData", "C"), None);
    }
}
