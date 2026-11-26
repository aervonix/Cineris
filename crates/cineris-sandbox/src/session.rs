//! Whether this process runs under a session's command: what the
//! subcommands that approve, trust or answer for the person
//! (`cineris ext add`, `remove` and `disable`, `cineris permissions trust`,
//! `untrust`, `revoke` and `forget`, `cineris setup`) look at before they do
//! anything, and refuse under (since step 28's third pull request, the
//! owner's answer to the early review's H1). The permission engine refuses
//! a command line that names them; this is the backstop for one that runs
//! them some other way (a script, a renamed copy, `cargo run`).
//!
//! - **On Linux, by the line of parents,** as the RPC server tells its own
//!   commands (`cineris-rpc`'s `socket::unix`): from this process's parent
//!   up, each process's `/proc/<pid>/stat`. A process is a session's harness
//!   when it is Cineris (its name `cineris` or `cin`, or this binary's own,
//!   or the very file this process runs) and its first argument is not a
//!   subcommand that runs no session (`ext`, `config`, `attach` and the
//!   rest): the application, a headless run and `serve` run sessions. A
//!   line that cannot be read refuses, as if it reached one. A parent that
//!   `/proc` does not show (another user's: an SSH session's `sshd`, a
//!   `sudo`) ends the line elsewhere. **It does not see** a command that
//!   detached itself from the line (its parent exited, and it was handed to
//!   the system's first process or a subreaper), or one in a namespace of
//!   processes of its own.
//! - **On Windows, by the harness's job.** A process that runs sessions
//!   calls [`mark`] at its start: it makes, or opens, the job
//!   [`JOB_NAME`] (named in the session's own namespace of objects, so one
//!   per logon of the user), with no limits and no breakaway, and joins it.
//!   Every process it starts from then on (a command's tree, a hook, an
//!   extension's runtime, an MCP server) is in that job and cannot leave
//!   it. A process in the job is under a session's command, whoever its
//!   parent was and whether or not that parent is still running. **It does
//!   not see** a process that a service starts for a command (WMI's
//!   `Win32_Process.Create`, the task scheduler), which is outside the job.
//!   And where the job cannot be made or joined (one an elevated Cineris
//!   made holds the name), the harness says so in its log and runs
//!   unmarked.
//!
//! Neither is a boundary against a command running with the user's rights:
//! such a command can write the files these subcommands write. It keeps the
//! harness's own subcommands from approving for a session.

use std::io;

/// The name of the job every Windows harness joins: `Local\` is the logon
/// session's own namespace.
pub const JOB_NAME: &str = r"Local\Cineris.SessionCommands";

/// Where this process stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Under {
    /// Not under a session's command, as far as can be told.
    Nothing,
    /// Under one: how that was told, in words.
    Session(String),
    /// Not known: what could not be read. Taken as under one.
    Unknown(String),
}

impl Under {
    /// Why an approving subcommand refuses here, when it does: under a
    /// session's command, or where that cannot be told.
    #[must_use]
    pub fn refusal(&self) -> Option<String> {
        match self {
            Self::Nothing => None,
            Self::Session(how) => Some(format!("it runs under a session's command ({how})")),
            Self::Unknown(what) => Some(format!(
                "whether it runs under a session's command could not be told ({what})"
            )),
        }
    }
}

/// Marks this process as one that runs sessions, before it starts any
/// process: on Windows it joins [`JOB_NAME`]'s job, and so does everything
/// it starts from then on. On Linux the line of parents serves, and
/// nothing is done.
///
/// # Errors
///
/// On Windows, when the job cannot be made, opened or joined.
pub fn mark() -> io::Result<()> {
    sys::mark()
}

/// Whether this process runs under a session's command.
#[must_use]
pub fn under_a_session() -> Under {
    sys::under_a_session()
}

/// Under a session's command, a variable of this process that moves where
/// Cineris's configuration and stores are read from, set otherwise than
/// for the session that started it (on Linux, by the session's process's
/// own environment) or than the system's own folders say (on Windows, by
/// the known folders; `CINERIS_CONFIG` and `CINERIS_DATA_DIR` there by being
/// set at all). `None` when none is.
#[must_use]
pub fn moved_stores() -> Option<String> {
    sys::moved_stores()
}

/// The first words of Cineris's subcommands that run no session: a
/// process of Cineris whose first argument is one of these starts no
/// session's command.
#[cfg_attr(windows, allow(dead_code, reason = "Linux's line of parents"))]
const NO_SESSION: [&str; 11] = [
    "sessions",
    "config",
    "permissions",
    "auth",
    "models",
    "usage",
    "stats",
    "project",
    "ext",
    "setup",
    "attach",
];

/// Whether a process of Cineris with these arguments (its program's word
/// first) runs sessions: the application, a headless run, `serve`, and
/// anything not told apart (fails closed).
#[cfg_attr(windows, allow(dead_code, reason = "Linux's line of parents"))]
fn runs_sessions(arguments: &[String]) -> bool {
    arguments
        .get(1)
        .is_none_or(|first| !NO_SESSION.contains(&first.as_str()))
}

/// What is known of one process of the line.
#[cfg_attr(windows, allow(dead_code, reason = "Linux's line of parents"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// Its parent, and whether it runs sessions.
    Of { parent: u32, harness: bool },
    /// Not shown: gone, or another user's that `/proc` hides.
    Unseen,
    /// A read that failed for another reason.
    Failed,
}

/// Where the line from `start` (this process's parent) leads, `seen`
/// reading each process.
#[cfg_attr(windows, allow(dead_code, reason = "Linux's line of parents"))]
fn line(start: u32, seen: impl Fn(u32) -> Seen) -> Under {
    let mut at = start;
    // A line of parents is short; the bound only stops a loop the table
    // could show while it changes.
    for _ in 0..4096 {
        if at <= 1 {
            return Under::Nothing;
        }
        match seen(at) {
            Seen::Of { harness: true, .. } => {
                return Under::Session(format!("process {at} runs Cineris's sessions"));
            }
            Seen::Of { parent, .. } if parent == at => return Under::Nothing,
            Seen::Of { parent, .. } => at = parent,
            // The parent itself not shown: it went as this started, and
            // what it was is not known.
            Seen::Unseen if at == start => {
                return Under::Unknown(format!("its parent, process {at}, is not shown"));
            }
            Seen::Unseen => return Under::Nothing,
            Seen::Failed => {
                return Under::Unknown(format!("process {at} of its line could not be read"));
            }
        }
    }
    Under::Unknown("its line of parents does not end".to_owned())
}

#[cfg(not(windows))]
#[allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a few small reads of `/proc`, once, before an approving subcommand does anything"
)]
mod sys {
    use std::io;
    use std::os::unix::fs::MetadataExt;

    use super::{Seen, Under, line, runs_sessions};

    pub(super) fn mark() -> io::Result<()> {
        Ok(())
    }

    pub(super) fn under_a_session() -> Under {
        line(std::os::unix::process::parent_id(), seen)
    }

    /// The process of the line that runs sessions.
    fn harness() -> Option<u32> {
        let mut at = std::os::unix::process::parent_id();
        for _ in 0..4096 {
            match seen(at) {
                Seen::Of { harness: true, .. } => return Some(at),
                Seen::Of { parent, .. } if parent > 1 && parent != at => at = parent,
                _ => return None,
            }
        }
        None
    }

    pub(super) fn moved_stores() -> Option<String> {
        let names = [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_DATA_HOME",
            "CINERIS_CONFIG",
            "CINERIS_DATA_DIR",
        ];
        let Some(pid) = harness() else {
            return names
                .iter()
                .find(|name| name.starts_with("CINERIS_") && std::env::var_os(name).is_some())
                .map(|name| (*name).to_owned());
        };
        let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ")) else {
            return Some("the session's own environment, which could not be read,".to_owned());
        };
        let theirs = |wanted: &str| {
            environ.split(|byte| *byte == 0).find_map(|entry| {
                let text = String::from_utf8_lossy(entry);
                text.split_once('=')
                    .filter(|(name, _)| *name == wanted)
                    .map(|(_, value)| value.to_owned())
            })
        };
        names
            .iter()
            .find(|name| {
                let mine = std::env::var_os(name).map(|value| value.to_string_lossy().into_owned());
                mine != theirs(name)
            })
            .map(|name| (*name).to_owned())
    }

    /// The names a process of Cineris has in `/proc/<pid>/stat`: its own
    /// two, and this binary's, each cut as the kernel cuts a name (to 15
    /// bytes).
    fn named(name: &str) -> bool {
        let cut = |text: &str| -> String { text.bytes().take(15).map(char::from).collect() };
        let own = std::env::current_exe().ok().and_then(|exe| {
            exe.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        name == "cineris" || name == "cin" || own.is_some_and(|own| cut(&own) == name)
    }

    /// Whether `pid` runs the very file this process runs.
    fn same_binary(pid: u32) -> bool {
        let (Ok(own), Ok(theirs)) = (
            std::fs::metadata("/proc/self/exe"),
            std::fs::metadata(format!("/proc/{pid}/exe")),
        ) else {
            return false;
        };
        own.dev() == theirs.dev() && own.ino() == theirs.ino()
    }

    fn seen(pid: u32) -> Seen {
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                return Seen::Unseen;
            }
            Err(_) => return Seen::Failed,
        };
        // The name is in parentheses and may hold anything: it ends at the
        // last `)`.
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
            return Seen::Failed;
        };
        let Some(parent) = stat
            .get(close + 1..)
            .and_then(|rest| rest.split_whitespace().nth(1))
            .and_then(|field| field.parse().ok())
        else {
            return Seen::Failed;
        };
        let name = stat.get(open + 1..close).unwrap_or("");
        let cineris = named(name) || same_binary(pid);
        // Its arguments, when it is Cineris: one that cannot be read is
        // taken as running sessions.
        let harness = cineris
            && std::fs::read(format!("/proc/{pid}/cmdline")).map_or(true, |line| {
                let arguments: Vec<String> = line
                    .split(|byte| *byte == 0)
                    .map(|word| String::from_utf8_lossy(word).into_owned())
                    .collect();
                runs_sessions(&arguments)
            });
        Seen::Of { parent, harness }
    }

    #[cfg(test)]
    mod tests {
        /// A process of Cineris is told by its own two names and this
        /// binary's, as the kernel cuts a name; any other name is not.
        #[test]
        fn cineris_is_told_by_its_names() {
            assert!(super::named("cineris") && super::named("cin"));
            for other in ["bash", "cineris-x", "Cineris", "ci", ""] {
                assert!(!super::named(other), "{other}");
            }
            let own = std::env::current_exe().unwrap();
            let own = own.file_name().unwrap().to_string_lossy();
            let cut: String = own.bytes().take(15).map(char::from).collect();
            assert!(super::named(&cut), "{cut}");
            assert!(super::same_binary(std::process::id()));
            assert!(!super::same_binary(1));
        }
    }
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "decision 0030: a job, and whether a process is in it, are only reachable through \
              the Windows API; cineris-sandbox is where such calls live"
)]
mod sys {
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, FALSE};
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, OpenJobObjectW,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT,
        SHGetKnownFolderPath,
    };

    use super::{JOB_NAME, Under};

    /// A known folder's path, as the system keeps it for this user.
    fn known(folder: &windows_sys::core::GUID) -> Option<std::path::PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        let mut path = std::ptr::null_mut();
        // SAFETY: `folder` is a live GUID, the flags and the token are
        // plain (none: this user's), and `path` is a live pointer for the
        // call to write the system's string's address to.
        let said = unsafe {
            SHGetKnownFolderPath(
                folder,
                KF_FLAG_DEFAULT as u32,
                ptr::null_mut(),
                &raw mut path,
            )
        };
        let found = (said == 0 && !path.is_null()).then(|| {
            // SAFETY: on success `path` is a nul-terminated string the
            // system made, read up to its nul.
            let length = (0..)
                .take_while(|at| unsafe { *path.add(*at) } != 0)
                .count();
            // SAFETY: as above, `length` units of it are there.
            let units = unsafe { std::slice::from_raw_parts(path, length) };
            std::path::PathBuf::from(std::ffi::OsString::from_wide(units))
        });
        // SAFETY: the string is the system's to free, freed once; a null
        // pointer is allowed.
        unsafe { CoTaskMemFree(path.cast()) };
        found
    }

    pub(super) fn moved_stores() -> Option<String> {
        for name in ["CINERIS_CONFIG", "CINERIS_DATA_DIR"] {
            if std::env::var_os(name).is_some() {
                return Some(name.to_owned());
            }
        }
        for (name, folder) in [
            ("USERPROFILE", &FOLDERID_Profile),
            ("APPDATA", &FOLDERID_RoamingAppData),
            ("LOCALAPPDATA", &FOLDERID_LocalAppData),
        ] {
            let mine = std::env::var_os(name).map(std::path::PathBuf::from);
            let same = match (mine, known(folder)) {
                (Some(mine), Some(theirs)) => cineris_base::same_location(&mine, &theirs),
                (None, _) => true,
                (Some(_), None) => false,
            };
            if !same {
                return Some(name.to_owned());
            }
        }
        None
    }

    /// The right to ask a job about itself and its processes (`winnt.h`'s
    /// `JOB_OBJECT_QUERY`), which `windows-sys` keeps under a feature of
    /// its own that this crate does not take for one number.
    const JOB_OBJECT_QUERY: u32 = 0x0004;

    /// The job, held for the process's life, so that its name stays.
    static JOB: OnceLock<OwnedHandle> = OnceLock::new();

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn mark() -> io::Result<()> {
        if JOB.get().is_some() {
            return Ok(());
        }
        let name = wide(JOB_NAME);
        // SAFETY: default security (a handle not inherited) and a name that
        // ends in a nul and lives through the call. A job of that name made
        // already (another harness's) is opened, which is what is wanted.
        let handle = unsafe { CreateJobObjectW(ptr::null(), name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let job = unsafe { OwnedHandle::from_raw_handle(handle) };
        // SAFETY: `job` is open with every right its maker gets, and
        // `GetCurrentProcess` returns a pseudo-handle that is always valid.
        // A process in a job already (a terminal's) takes this one as a job
        // nested in that one.
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _ = JOB.set(job);
        Ok(())
    }

    pub(super) fn under_a_session() -> Under {
        let name = wide(JOB_NAME);
        // SAFETY: plain values, and a name that ends in a nul and lives
        // through the call.
        let handle = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, name.as_ptr()) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            // No harness is running in this logon: nothing is under one.
            return if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
                Under::Nothing
            } else {
                Under::Unknown(format!("the sessions' job could not be opened: {error}"))
            };
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let job = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut inside = FALSE;
        // SAFETY: the pseudo-handle has every right, `job` is open with
        // `JOB_OBJECT_QUERY`, both live through the call, and `inside` is a
        // live `BOOL`.
        let asked =
            unsafe { IsProcessInJob(GetCurrentProcess(), job.as_raw_handle(), &raw mut inside) };
        if asked == 0 {
            return Under::Unknown(format!(
                "whether it is in the sessions' job could not be asked: {}",
                io::Error::last_os_error()
            ));
        }
        if inside == 0 {
            Under::Nothing
        } else {
            Under::Session("it is in the job of Cineris's sessions' commands".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of processes, as `/proc` would show them; one not in it is
    /// not shown.
    fn table(processes: &[(u32, Seen)]) -> impl Fn(u32) -> Seen + '_ {
        |pid| {
            processes
                .iter()
                .find(|(at, _)| *at == pid)
                .map_or(Seen::Unseen, |(_, seen)| *seen)
        }
    }

    const fn of(parent: u32, harness: bool) -> Seen {
        Seen::Of { parent, harness }
    }

    /// The line reaches a harness however far up it is; it ends elsewhere
    /// at the first process, at a parent `/proc` hides, or at one that is
    /// its own parent; and a line that cannot be read, or whose first
    /// parent is gone, is not known.
    #[test]
    fn a_line_of_parents_reaches_a_harness_ends_elsewhere_or_is_not_known() {
        let processes = [
            (10, of(9, false)),
            (9, of(8, false)),
            (8, of(1, true)),
            (20, of(19, false)),
            (19, of(1, false)),
            (30, of(29, false)),
            (40, Seen::Failed),
            (50, of(50, false)),
            (60, of(40, false)),
        ];
        let read = table(&processes);
        assert!(matches!(line(10, &read), Under::Session(_)));
        assert!(matches!(line(8, &read), Under::Session(_)));
        assert_eq!(line(20, &read), Under::Nothing);
        // 29 is not shown: another user's, which ends the line.
        assert_eq!(line(30, &read), Under::Nothing);
        assert_eq!(line(50, &read), Under::Nothing);
        assert_eq!(line(1, &read), Under::Nothing);
        assert!(matches!(line(40, &read), Under::Unknown(_)));
        assert!(matches!(line(60, &read), Under::Unknown(_)));
        // The parent itself gone.
        assert!(matches!(line(70, &read), Under::Unknown(_)));
        assert!(Under::Nothing.refusal().is_none());
        assert!(line(40, &read).refusal().is_some());
    }

    /// A process of Cineris runs sessions unless its first argument is a
    /// subcommand that runs none.
    #[test]
    fn a_process_of_cineris_runs_sessions_unless_it_is_a_subcommand_that_runs_none() {
        let words =
            |line: &[&str]| -> Vec<String> { line.iter().map(|w| (*w).to_owned()).collect() };
        for runs in [
            &["cineris"][..],
            &["cin", "-p", "hello"],
            &["cineris", "serve", "--stdio"],
            &["cineris", "--resume", "x"],
            &["/usr/bin/cineris", "--profile", "ext"],
        ] {
            assert!(runs_sessions(&words(runs)), "{runs:?}");
        }
        for runs_none in [
            &["cineris", "ext", "list"][..],
            &["cineris", "config", "edit"],
            &["cin", "attach"],
            &["cineris", "permissions", "trust"],
        ] {
            assert!(!runs_sessions(&words(runs_none)), "{runs_none:?}");
        }
    }
}
