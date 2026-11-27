//! The Windows half of [`super`]: the stored folder, read from the
//! registry, and the system's Windows directory.
//!
//! The third of the modules of this crate that may use `unsafe`: the
//! registry and the Windows directory can only be read through the Windows
//! API. Both calls write into a buffer of this module's, whose length they
//! are given.

#![allow(
    unsafe_code,
    reason = "the folder the system records can only be read through the Windows API"
)]

use std::io;
use std::path::PathBuf;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
};
use windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;

/// The key that holds the folder, under `HKEY_LOCAL_MACHINE`.
const PROFILE_LIST: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList";
/// The value that names it.
const VALUE: &str = "ProgramData";
/// Longer than any folder a value names: the registry's own limit for a
/// path is 32,767 characters, and one this long is refused.
const MOST_UNITS: usize = 32_768;

/// The folder for every user's program data, as the system records it
/// (`C:\ProgramData`), with no variable of the process read.
///
/// # Errors
///
/// When the registry does not hold the value, the Windows directory cannot
/// be read, or the value names a variable other than the system's drive
/// and root ([`super::expand`]).
pub fn folder() -> io::Result<PathBuf> {
    let stored = stored_value()?;
    let windows_directory = windows_directory()?;
    super::expand(&stored, &windows_directory)
        .map(PathBuf::from)
        .ok_or_else(|| {
            io::Error::other(format!(
                "the system records the program data folder as `{stored}`, which names a \
                 variable other than the system's drive"
            ))
        })
}

/// The value as it is stored, its variables not expanded.
fn stored_value() -> io::Result<String> {
    let key = wide(PROFILE_LIST);
    let value = wide(VALUE);
    let mut units = vec![0_u16; 260];
    loop {
        let mut bytes = u32::try_from(units.len() * 2).unwrap_or(u32::MAX);
        // SAFETY: `key` and `value` are zero-terminated UTF-16 strings, alive
        // for the call. `units` is writable for `bytes` bytes, its whole
        // length, and `bytes` is a live `u32` the call reads and writes. The
        // type is an optional output and is null. `RRF_NOEXPAND` keeps the
        // text as stored: nothing of the process's environment is read.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
                ptr::null_mut(),
                units.as_mut_ptr().cast(),
                &raw mut bytes,
            )
        };
        if status == ERROR_MORE_DATA && units.len() < MOST_UNITS {
            units.resize(MOST_UNITS, 0);
            continue;
        }
        if status != ERROR_SUCCESS {
            // This call returns its error; it does not set the thread's.
            return Err(io::Error::from_raw_os_error(status.cast_signed()));
        }
        // What was written, in bytes, the ending zero included.
        let written = (bytes as usize / 2).min(units.len());
        let text = &units[..written];
        let end = text
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(text.len());
        return String::from_utf16(&text[..end])
            .map_err(|_| io::Error::other("the program data folder is not valid text"));
    }
}

/// The system's Windows directory (`C:\Windows`), the one every user
/// shares, from the system and not from `SystemRoot`.
fn windows_directory() -> io::Result<String> {
    let mut units = vec![0_u16; 260];
    loop {
        let size = u32::try_from(units.len()).unwrap_or(u32::MAX);
        // SAFETY: `units` is writable for `size` UTF-16 units, its whole
        // length.
        let written = unsafe { GetSystemWindowsDirectoryW(units.as_mut_ptr(), size) } as usize;
        if written == 0 {
            return Err(io::Error::last_os_error());
        }
        // Too small: the answer is the length it needs, its zero included.
        if written >= units.len() {
            if written > MOST_UNITS {
                return Err(io::Error::other("the Windows directory's name is too long"));
            }
            units.resize(written, 0);
            continue;
        }
        return String::from_utf16(&units[..written])
            .map_err(|_| io::Error::other("the Windows directory's name is not valid text"));
    }
}

/// `text` as a zero-terminated UTF-16 string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder is on a drive, ends in the name Windows gives it, and is
    /// a folder that exists: every Windows machine has one.
    #[test]
    fn the_folder_is_the_machines_program_data() {
        #![allow(
            clippy::disallowed_methods,
            reason = "a test of the machine's own folder"
        )]
        let found = folder().unwrap();
        let text = found.to_string_lossy();
        assert!(found.is_absolute(), "{text}");
        assert!(
            text.to_ascii_lowercase().ends_with(r"\programdata"),
            "{text}"
        );
        assert!(found.is_dir(), "{text}");
        let windows = windows_directory().unwrap();
        assert_eq!(text.get(..2), windows.get(..2), "on the system's drive");
    }
}
