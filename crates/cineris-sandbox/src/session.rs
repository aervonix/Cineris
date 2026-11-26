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
//!   when it is Cineris (the very file this process runs, or a process
//!   named `cineris` or `cin`, or as this binary is, whose file is named so
//!   too: never by its name alone) and its first argument is not a
//!   subcommand that runs no session (`ext`, `config`, `attach` and the
//!   rest): the application, a headless run and `serve` run sessions. A
//!   line that cannot be read refuses, as if it reached one. A parent that
//!   `/proc` does not show (another user's: an SSH session's `sshd`, a
//!   `sudo`) ends the line elsewhere. **It does not see** a command that
//!   detached itself from the line (its parent exited, and it was handed to
//!   the system's first process or a subreaper), or one in a namespace of
//!   processes of its own.
//! - **On Windows, by the harness's own job, and by what started it.** A
//!   process that runs sessions calls [`mark`] at its start: it makes a job
//!   of its own, named [`JOB_PREFIX`] and its process's instance (its id and
//!   when it was made) in the logon session's own namespace of objects, open
//!   by its name to its user and its owner only to ask about it, with no
//!   limits and no breakaway, and joins it. Every process it starts from then
//!   on (a command's tree, a hook, an extension's runtime, an MCP server) is
//!   in that job and cannot leave it. A process is under a session's command
//!   when it is in the job of that name of a process that is running: each
//!   running process this user may open is tried, and a job of that name
//!   that cannot be opened or asked, for any reason but there being none,
//!   is not known, which refuses. Being in any other job (a terminal's, an
//!   editor's, a CI runner's) is nothing. One job each, and not one shared:
//!   a process already in a job can join a job only where that job can nest
//!   in its own, so a job shared by harnesses started under different jobs
//!   could be joined by the first alone. **The mark fails closed:** a job of
//!   its name there already (made ahead, by another process), one that lets
//!   what it holds leave, or one that cannot be made or joined is not
//!   taken, and the harness holds its sessions below `unrestricted` and says
//!   so. **A process a service started for a command is under one too:**
//!   one whose line of parents, through processes of this user, reaches
//!   WMI's provider host (`WmiPrvSE.exe`, `Win32_Process.Create`), the task
//!   host (`taskhostw.exe`) or the service host that runs the task
//!   scheduler (a scheduled task's), directly or through command
//!   interpreters (`cmd /c`) and language runtimes (`python -c`, `node -e`,
//!   a `git` alias, `wsl`), which start a process outside the job (the full
//!   review of the third part, H1, and its narrow re-check, M2'); where which
//!   host runs the scheduler cannot be asked, any service host counts. Any
//!   other program ends the line: a terminal or a CI runner a service
//!   started is not taken for one. **It does not see** a service's start
//!   through another program than an interpreter or a runtime, a process
//!   another of the user's processes starts outside the job (the desktop's
//!   shell asked to open a file, a process made with another parent), nor a
//!   command whose harness has exited, whose job no running process names.
//!
//! Neither is a boundary against a command running with the user's rights:
//! such a command can write the files these subcommands write. It keeps the
//! harness's own subcommands from approving for a session.

use std::io;

/// The name of the job a Windows harness makes and joins, before its
/// process id: `Local\` is the logon session's own namespace.
pub const JOB_PREFIX: &str = r"Local\Cineris.SessionCommands.";

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
/// process: on Windows it joins a job of its own ([`JOB_PREFIX`]), and so
/// does everything it starts from then on. On Linux the line of parents
/// serves, and nothing is done.
///
/// # Errors
///
/// On Windows, when the job cannot be made or joined, a job of its name was
/// there already, or the job lets the processes in it leave: the harness
/// then holds its sessions below `unrestricted`.
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
/// for the session that started it: on Linux by the session's process's own
/// environment, on Windows by what the session told what it starts
/// (`CINERIS_SESSION_STORES`, which [`mark`] sets, and which the permission
/// engine refuses a command line to set). `None` when none is.
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

    /// Whether the file `pid` runs is named as Cineris is (`cineris`, `cin`
    /// or this binary's own name), its name and its file together: a
    /// process named `cin` that runs another program (a user's own
    /// `#!/bin/sh` wrapper of that name, which runs as `/bin/sh`) is not
    /// Cineris (the full review of the third part). A file that cannot be
    /// read (another user's process) is taken as Cineris's: what it is is
    /// not known.
    fn file_named(pid: u32) -> bool {
        match std::fs::read_link(format!("/proc/{pid}/exe")) {
            Ok(file) => {
                let name = file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // A file replaced or removed since it was started.
                let name = name.strip_suffix(" (deleted)").unwrap_or(&name);
                let own = std::env::current_exe().ok().and_then(|exe| {
                    exe.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                });
                name == "cineris" || name == "cin" || own.is_some_and(|own| own == name)
            }
            Err(_) => true,
        }
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
        let cineris = same_binary(pid) || (named(name) && file_named(pid));
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
            assert!(super::file_named(std::process::id()));
        }

        /// A process named as Cineris is whose file is another program's
        /// (a user's wrapper script named `cin`, run by `/bin/sh`) is not
        /// Cineris: its name alone is not enough (the full review of the
        /// third part).
        #[test]
        fn sys_a_program_named_cin_that_is_another_is_not_cineris() {
            use std::os::unix::fs::PermissionsExt as _;
            let scratch = cineris_testkit::scratch::Scratch::outside(
                "cineris-sandbox-session",
                "sys_a_program_named_cin_that_is_another_is_not_cineris",
            );
            let script = scratch.join("cin");
            std::fs::write(&script, "#!/bin/sh\nread -r line\n").unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut child = std::process::Command::new(&script)
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let pid = child.id();
            let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap();
            let seen = super::seen(pid);
            drop(child.stdin.take());
            child.wait().unwrap();
            assert_eq!(comm.trim_end(), "cin");
            assert!(
                matches!(seen, super::Seen::Of { harness: false, .. }),
                "{seen:?}"
            );
        }
    }
}

/// The name of the job of the harness process `pid`, made at `made` (a
/// `FILETIME`'s count): its process's instance, which no other process has,
/// so that a job of a harness that has gone, whose process id is another's
/// now, is not taken for that one's (the full review of the third part).
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's jobs"))]
fn job_name(pid: u32, made: u64) -> String {
    format!("{JOB_PREFIX}{pid}.{made}")
}

/// One process of the system's list of processes, as Windows gives it.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
#[derive(Debug, Clone)]
struct Listed {
    pid: u32,
    parent: u32,
    /// Its program's file name, in lower case.
    name: String,
}

/// What opening a listed process tells.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opened {
    /// When it was made, as a `FILETIME`'s count.
    Made(u64),
    /// It could not be opened: another user's, or the system's.
    Closed,
}

/// The programs through which a service starts a process that a command
/// asked it for, apart from the task scheduler's own service host: WMI's
/// provider host (`Win32_Process.Create`), the task host and the old task
/// engine (a scheduled task's).
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
const SERVICE_STARTERS: [&str; 3] = ["wmiprvse.exe", "taskhostw.exe", "taskeng.exe"];

/// The command interpreters and console hosts a service's start of a
/// command may pass through (`cmd /c cin …`): the line of parents is
/// followed up through these alone.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
const INTERPRETERS: [&str; 12] = [
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "conhost.exe",
    "openconsole.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
    "rundll32.exe",
    "forfiles.exe",
    "bash.exe",
    "sh.exe",
];

/// The language runtimes and the programs that run a line of their own a
/// service's start of a command may pass through (`python -c "…run(['cin',
/// …])"`, `node -e`, `perl -e`, a `git` alias, `wsl`): followed up as the
/// interpreters are (the narrow re-check of the third part, M2'). By name,
/// with a version after it taken off (`python3.12.exe`).
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
const RUNTIMES: [&str; 16] = [
    "python", "pythonw", "py", "node", "perl", "ruby", "php", "java", "javaw", "deno", "bun",
    "rscript", "wsl", "git", "awk", "gawk",
];

/// Whether a program's file name (in lower case) is a command interpreter's
/// or a runtime's: the line of parents is followed up through these alone.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
fn passes_through(name: &str) -> bool {
    if INTERPRETERS.contains(&name) {
        return true;
    }
    let bare = name.strip_suffix(".exe").unwrap_or(name);
    let bare = bare.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    RUNTIMES.contains(&bare)
}

/// How far up a line of parents is followed on Windows.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
const LINE_OF_PARENTS: usize = 64;

/// Whether this process, `own`, was started by a service that starts what
/// a command asks of it, outside any job (WMI's provider host, the task
/// host, the service host that runs the task scheduler, `scheduler`: `None`
/// where it could not be asked, when any service host counts), directly or
/// through command interpreters and runtimes ([`passes_through`]: `cmd /c
/// cin …`, `python -c …`): the full review of the third part, H1, and its
/// narrow re-check, M2'. The line is followed up through those alone, each
/// made before the one below it (a later process may have the id of a
/// parent that has gone): any other program ends it, so that a person's
/// terminal, an editor or a CI runner that a service started long before
/// is not taken for one.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
fn started_by_a_service(
    listed: &[Listed],
    own: u32,
    opened: impl Fn(u32) -> Opened,
    scheduler: Option<u32>,
) -> Under {
    let find = |pid: u32| listed.iter().find(|process| process.pid == pid);
    let Opened::Made(mut made) = opened(own) else {
        return Under::Nothing;
    };
    let mut at = own;
    for _ in 0..LINE_OF_PARENTS {
        let Some(parent) = find(at).map(|process| process.parent) else {
            return Under::Nothing;
        };
        if parent == 0 || parent == at {
            return Under::Nothing;
        }
        // Its parent has gone, and nothing has its id.
        let Some(above) = find(parent) else {
            return Under::Nothing;
        };
        let seen = opened(parent);
        // A process made after it has the id of its parent, which has gone.
        if let Opened::Made(then) = seen
            && then > made
        {
            return Under::Nothing;
        }
        let name = above.name.as_str();
        let schedules =
            name == "svchost.exe" && scheduler.is_none_or(|scheduler| scheduler == parent);
        if SERVICE_STARTERS.contains(&name) || schedules {
            let which = if schedules && scheduler.is_none() {
                "a service host, and which service it runs could not be asked"
            } else if schedules {
                "the task scheduler's service host"
            } else {
                "a service's host of what it starts"
            };
            return Under::Session(format!(
                "process {parent}, `{name}`, started it or a process above it: {which}, which \
                 starts what a session's command asks of it outside the session's job"
            ));
        }
        match seen {
            Opened::Made(then) if passes_through(name) => {
                at = parent;
                made = then;
            }
            _ => return Under::Nothing,
        }
    }
    Under::Nothing
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "decision 0030: a job, whether a process is in it, a process's start and the \
              service a process runs are only reachable through the Windows API; \
              cineris-sandbox is where such calls live"
)]
mod sys {
    use std::collections::HashMap;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr;
    use std::sync::OnceLock;

    use super::{Listed, Opened, Under, job_name, started_by_a_service};
    use windows_sys::Win32::Foundation::{
        ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, FALSE, FILETIME, HANDLE, INVALID_HANDLE_VALUE,
        LocalFree, SetLastError,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, OpenJobObjectW, QueryInformationJobObject,
    };
    use windows_sys::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx, SC_MANAGER_CONNECT,
        SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS, SERVICE_STATUS_PROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess, OpenProcessToken,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// The variables that place Cineris's configuration and stores on
    /// Windows.
    const STORES: [&str; 5] = [
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "CINERIS_CONFIG",
        "CINERIS_DATA_DIR",
    ];

    /// What a harness says of its stores to everything it starts: each of
    /// [`STORES`] it has, `NAME=value`, joined by `|`, which no Windows path
    /// holds.
    const SAID: &str = "CINERIS_SESSION_STORES";

    fn stores_now() -> String {
        STORES
            .iter()
            .filter_map(|name| {
                std::env::var_os(name).map(|value| format!("{name}={}", value.to_string_lossy()))
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    pub(super) fn moved_stores() -> Option<String> {
        let Some(said) = std::env::var_os(SAID) else {
            return Some("its stores, which the session did not say,".to_owned());
        };
        let said = said.to_string_lossy().into_owned();
        let theirs = |wanted: &str| {
            said.split('|').find_map(|entry| {
                entry
                    .split_once('=')
                    .filter(|(name, _)| *name == wanted)
                    .map(|(_, value)| value.to_owned())
            })
        };
        STORES
            .iter()
            .find(|name| {
                let mine = std::env::var_os(name).map(|value| value.to_string_lossy().into_owned());
                mine != theirs(name)
            })
            .map(|name| (*name).to_owned())
    }

    /// The right to ask a job about itself and its processes (`winnt.h`'s
    /// `JOB_OBJECT_QUERY`), which `windows-sys` keeps under a feature of
    /// its own that this crate does not take for one number.
    pub(super) const JOB_OBJECT_QUERY: u32 = 0x0004;

    /// The right to wait on an object (`winnt.h`'s `SYNCHRONIZE`).
    const SYNCHRONIZE: u32 = 0x0010_0000;

    /// What the job's own user, and its owner, may do with it by its name:
    /// ask it about its processes, and nothing else. The harness's own
    /// handle has every right, given when it is made; no other process may
    /// change its limits, join a process to it or end it by its name.
    const BY_NAME: u32 = JOB_OBJECT_QUERY | SYNCHRONIZE;

    /// The job, held for the process's life, so that its name stays.
    static JOB: OnceLock<OwnedHandle> = OnceLock::new();

    pub(super) fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// When the process `handle` names was made, as a `FILETIME`'s count.
    pub(super) fn made_at(handle: HANDLE) -> Option<u64> {
        let mut times = [FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        }; 4];
        let [made, ended, kernel, user] = &mut times;
        // SAFETY: `handle` is open with at least the limited right to ask,
        // and each pointer is to a live `FILETIME`.
        let asked = unsafe { GetProcessTimes(handle, made, ended, kernel, user) };
        (asked != 0)
            .then(|| (u64::from(times[0].dwHighDateTime) << 32) | u64::from(times[0].dwLowDateTime))
    }

    /// The job's security: open by its name to its user and its owner only,
    /// and only to ask ([`BY_NAME`]); `P`, so that nothing is inherited.
    pub(super) struct OwnerOnly(*mut core::ffi::c_void);

    impl Drop for OwnerOnly {
        fn drop(&mut self) {
            // SAFETY: the descriptor was given by
            // `ConvertStringSecurityDescriptorToSecurityDescriptorW`, to be
            // freed with `LocalFree`, once.
            unsafe { LocalFree(self.0) };
        }
    }

    impl OwnerOnly {
        pub(super) fn new() -> io::Result<Self> {
            Self::from_sddl(&format!(
                "D:P(A;;0x{BY_NAME:x};;;{})(A;;0x{BY_NAME:x};;;OW)",
                user_sid()?
            ))
        }

        /// A descriptor from its text.
        pub(super) fn from_sddl(text: &str) -> io::Result<Self> {
            let text = wide(text);
            let mut descriptor = ptr::null_mut();
            // SAFETY: a text that ends in a nul, and a live pointer for the
            // descriptor, which the call allocates.
            let made = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    text.as_ptr(),
                    SDDL_REVISION_1,
                    &raw mut descriptor,
                    ptr::null_mut(),
                )
            };
            if made == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(descriptor))
        }

        pub(super) fn attributes(&self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                    .unwrap_or(u32::MAX),
                lpSecurityDescriptor: self.0,
                bInheritHandle: FALSE,
            }
        }
    }

    /// This process's user, as a SID's text (`S-1-5-21-…`).
    fn user_sid() -> io::Result<String> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: the pseudo-handle is always valid, and `token` is a live
        // handle's place.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `token` was just opened and nothing else owns it.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        // Room for the user's SID, aligned for the structure's pointer.
        let mut buffer = [0u64; 32];
        let mut needed = 0u32;
        // SAFETY: `token` is open to query, and the buffer is as large as
        // said and aligned for `TOKEN_USER`.
        let asked = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                u32::try_from(std::mem::size_of_val(&buffer)).unwrap_or(0),
                &raw mut needed,
            )
        };
        if asked == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call wrote a `TOKEN_USER` at the buffer's start.
        let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        let mut text = ptr::null_mut();
        // SAFETY: `sid` points into the buffer, which lives through the
        // call; the text is allocated by it.
        if unsafe { ConvertSidToStringSidW(sid, &raw mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `text` is a nul-terminated wide string the call made.
        let length = (0..)
            .take_while(|at| unsafe { *text.add(*at) } != 0)
            .count();
        // SAFETY: as above, `length` characters before the nul.
        let said = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
        // SAFETY: allocated by the call, to be freed with `LocalFree`, once.
        unsafe { LocalFree(text.cast()) };
        Ok(said)
    }

    /// Whether the job lets the processes in it leave (`BREAKAWAY_OK` or
    /// `SILENT_BREAKAWAY_OK`): a process started that way is out of it, and
    /// the job would not see it.
    pub(super) fn lets_processes_leave(job: &OwnedHandle) -> io::Result<bool> {
        // SAFETY: the structure is plain data, for which all zeroes is valid.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `job` is open with the right to ask, and the structure is
        // as large as said.
        let asked = unsafe {
            QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw mut limits).cast(),
                u32::try_from(std::mem::size_of_val(&limits)).unwrap_or(0),
                ptr::null_mut(),
            )
        };
        if asked == 0 {
            return Err(io::Error::last_os_error());
        }
        let leave = JOB_OBJECT_LIMIT_BREAKAWAY_OK | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK;
        Ok(limits.BasicLimitInformation.LimitFlags & leave != 0)
    }

    fn refuse_leaving(job: &OwnedHandle) -> io::Result<()> {
        if lets_processes_leave(job)? {
            return Err(io::Error::other(
                "its job lets the processes in it leave, so the job would not see them",
            ));
        }
        Ok(())
    }

    pub(super) fn mark() -> io::Result<()> {
        if JOB.get().is_some() {
            return Ok(());
        }
        // SAFETY: the pseudo-handle is always valid, with every right.
        let made = made_at(unsafe { GetCurrentProcess() }).ok_or_else(io::Error::last_os_error)?;
        let name = wide(&job_name(std::process::id(), made));
        let security = OwnerOnly::new()?;
        let attributes = security.attributes();
        // SAFETY: the code is a plain value; it is set so that what the
        // call leaves is its own.
        unsafe { SetLastError(0) };
        // SAFETY: the attributes and their descriptor live through the
        // call, and the name ends in a nul.
        let handle = unsafe { CreateJobObjectW(&raw const attributes, name.as_ptr()) };
        let said = io::Error::last_os_error();
        if handle.is_null() {
            return Err(said);
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let job = unsafe { OwnedHandle::from_raw_handle(handle) };
        // A job of this name made before this one: by another process, as
        // its name is this process's own instance. It is not joined, and
        // not trusted (the full review of the third part: it could let
        // what it holds leave, or keep this process's out).
        if said.raw_os_error() == i32::try_from(ERROR_ALREADY_EXISTS).ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a job of its name was there already, made by another process",
            ));
        }
        refuse_leaving(&job)?;
        // SAFETY: `job` is open with every right its maker gets, and
        // `GetCurrentProcess` returns a pseudo-handle that is always valid.
        // A process in a job already (a terminal's) takes this one as a job
        // nested in that one.
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // Asked again once joined: a limit set between the making and the
        // joining is seen.
        refuse_leaving(&job)?;
        let _ = JOB.set(job);
        // What it starts is told where its stores are, by a variable it
        // inherits: a nested run under it reads them there, or not at all
        // (the re-look at the early review, B). Windows has no process's
        // environment another can read as Linux's `/proc` has.
        let stores = stores_now();
        // SAFETY: on Windows the environment is the system's, and its calls
        // are serialised by it: no other thread's read of it is torn.
        unsafe { std::env::set_var(SAID, stores) };
        Ok(())
    }

    /// The process `pid`, opened only to ask when it was made: `Closed`
    /// where this user may not (another user's, the system's).
    fn opened(pid: u32) -> Opened {
        // SAFETY: plain values.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
        if handle.is_null() {
            return Opened::Closed;
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let process = unsafe { OwnedHandle::from_raw_handle(handle) };
        made_at(process.as_raw_handle()).map_or(Opened::Closed, Opened::Made)
    }

    /// The process that runs the task scheduler's service, `Schedule`:
    /// `Some(0)` when it does not run, `None` when that cannot be asked.
    pub(super) fn scheduler() -> Option<u32> {
        let service = wide("Schedule");
        // SAFETY: the local machine's active database, asked only to
        // connect.
        let manager = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_CONNECT) };
        if manager.is_null() {
            return None;
        }
        // SAFETY: `manager` is open, and the name ends in a nul.
        let opened = unsafe { OpenServiceW(manager, service.as_ptr(), SERVICE_QUERY_STATUS) };
        let found = (!opened.is_null()).then(|| {
            // SAFETY: the structure is plain data, for which all zeroes is
            // valid.
            let mut status: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
            let mut needed = 0u32;
            // SAFETY: `opened` is open to ask its status, and the buffer is
            // a live structure as large as said.
            let asked = unsafe {
                QueryServiceStatusEx(
                    opened,
                    SC_STATUS_PROCESS_INFO,
                    (&raw mut status).cast(),
                    u32::try_from(std::mem::size_of_val(&status)).unwrap_or(0),
                    &raw mut needed,
                )
            };
            (asked != 0).then_some(status.dwProcessId)
        });
        // SAFETY: each handle was opened above and is closed once.
        unsafe {
            if !opened.is_null() {
                CloseServiceHandle(opened);
            }
            CloseServiceHandle(manager);
        }
        found.flatten()
    }

    pub(super) fn under_a_session() -> Under {
        // SAFETY: the call takes two integers; for a list of processes the
        // second is not used.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Under::Unknown(format!(
                "the system's processes could not be listed: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: `snapshot` was just made and nothing else owns it.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
        // SAFETY: the structure is plain data, for which all zeroes is valid.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(u32::MAX);
        let mut listed = Vec::new();
        let mut times = HashMap::new();
        // SAFETY: `snapshot` is open, and `entry` is a writable entry whose
        // `dwSize` says how large it is.
        let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
        while more {
            let pid = entry.th32ProcessID;
            let length = entry
                .szExeFile
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(entry.szExeFile.len());
            listed.push(Listed {
                pid,
                parent: entry.th32ParentProcessID,
                name: String::from_utf16_lossy(&entry.szExeFile[..length]).to_lowercase(),
            });
            let seen = opened(pid);
            times.insert(pid, seen);
            // A harness's job is named for its process's instance: one this
            // user may not open (another user's, the system's) is no
            // harness of this process's, whose commands run as its user.
            if let Opened::Made(made) = seen
                && let Some(under) = in_the_job_of(pid, made)
            {
                return under;
            }
            // SAFETY: as for `Process32FirstW`.
            more = unsafe { Process32NextW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
        }
        // SAFETY: plain call.
        let own = unsafe { GetCurrentProcessId() };
        started_by_a_service(
            &listed,
            own,
            |pid| times.get(&pid).copied().unwrap_or(Opened::Closed),
            scheduler(),
        )
    }

    /// Whether this process is in the job of sessions' commands of the
    /// harness `pid`, made at `made`: `None` when it is not, or there is no
    /// such job. A job of that name that cannot be opened or asked, for
    /// any reason but there being none, is not known, which refuses (the
    /// full review of the third part: a job made to deny its opening must
    /// not read as no job).
    fn in_the_job_of(pid: u32, made: u64) -> Option<Under> {
        let name = wide(&job_name(pid, made));
        // SAFETY: plain values, and a name that ends in a nul and lives
        // through the call.
        let handle = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, name.as_ptr()) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == i32::try_from(ERROR_FILE_NOT_FOUND).ok() {
                return None;
            }
            return Some(Under::Unknown(format!(
                "the job of process {pid}'s sessions' commands could not be opened: {error}"
            )));
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
            return Some(Under::Unknown(format!(
                "whether it is in the job of process {pid}'s sessions' commands could not be \
                 asked: {}",
                io::Error::last_os_error()
            )));
        }
        (inside != 0).then(|| {
            Under::Session(format!(
                "it is in the job of process {pid}'s sessions' commands"
            ))
        })
    }

    #[cfg(test)]
    mod tests;
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

    fn listed(processes: &[(u32, u32, &str)]) -> Vec<Listed> {
        processes
            .iter()
            .map(|(pid, parent, name)| Listed {
                pid: *pid,
                parent: *parent,
                name: (*name).to_owned(),
            })
            .collect()
    }

    /// A process started by WMI's provider host, the task host or the task
    /// scheduler's service host, directly or through a `cmd /c` of the
    /// user's, is under a session's command (the full review of the third
    /// part, H1); any other service host is not, unless which service the
    /// scheduler runs in could not be asked; any program but an interpreter
    /// (the person's desktop, a CI runner's test runner), a process that
    /// cannot be opened, or a parent made after its child ends the line
    /// elsewhere.
    #[test]
    fn a_process_a_service_started_for_a_command_is_under_a_session() {
        let processes = listed(&[
            (100, 90, "cin.exe"),
            (90, 80, "cmd.exe"),
            (80, 4, "wmiprvse.exe"),
            (200, 190, "cin.exe"),
            (190, 600, "svchost.exe"),
            (300, 290, "cin.exe"),
            (290, 700, "svchost.exe"),
            (400, 390, "cin.exe"),
            (390, 380, "pwsh.exe"),
            (380, 370, "explorer.exe"),
            (370, 4, "wmiprvse.exe"),
            (500, 490, "cin.exe"),
            (490, 480, "pwsh.exe"),
            (480, 4, "taskhostw.exe"),
            (510, 505, "cin.exe"),
            (505, 4, "wmiprvse.exe"),
            (520, 515, "cin.exe"),
            (515, 4, "sshd.exe"),
            (540, 535, "cin.exe"),
            (535, 530, "cargo-nextest.exe"),
            (530, 600, "svchost.exe"),
            (550, 9999, "cin.exe"),
            (600, 4, "services.exe"),
            (700, 4, "services.exe"),
        ]);
        let opened = |pid: u32| match pid {
            // The system's and the services': not this user's.
            4 | 80 | 190 | 290 | 370 | 515 | 600 | 700 => Opened::Closed,
            // Made after its child: it has a gone parent's id.
            505 => Opened::Made(2_000),
            _ => Opened::Made(u64::from(pid)),
        };
        let under = |own, scheduler| started_by_a_service(&processes, own, opened, scheduler);
        assert!(
            matches!(under(100, Some(600)), Under::Session(_)),
            "WMI, through cmd"
        );
        assert!(
            matches!(under(200, Some(190)), Under::Session(_)),
            "the scheduler's host"
        );
        assert_eq!(
            under(300, Some(190)),
            Under::Nothing,
            "another service's host"
        );
        assert!(
            matches!(under(300, None), Under::Session(_)),
            "not asked: any host"
        );
        assert!(
            matches!(under(300, Some(0)), Under::Nothing),
            "no scheduler running"
        );
        assert_eq!(
            under(400, None),
            Under::Nothing,
            "the desktop ends the line"
        );
        assert_eq!(
            under(540, None),
            Under::Nothing,
            "a runner a service started"
        );
        assert!(
            matches!(under(500, Some(600)), Under::Session(_)),
            "the task host"
        );
        assert_eq!(
            under(510, Some(600)),
            Under::Nothing,
            "made after its child"
        );
        assert_eq!(
            under(520, Some(600)),
            Under::Nothing,
            "another user's process"
        );
        assert_eq!(
            under(550, Some(600)),
            Under::Nothing,
            "a parent that has gone"
        );
        let Under::Session(how) = under(100, Some(600)) else {
            unreachable!()
        };
        assert!(
            how.contains("process 80") && how.contains("wmiprvse.exe"),
            "{how}"
        );
    }

    /// A service's start of a command through a language runtime (`python
    /// -c "…run(['cin',…])"`, `node -e`, `perl -e`, a `git` alias) is
    /// followed up through it, a version after its name taken off (the
    /// narrow re-check of the third part, M2'); a person's own runtime, run
    /// from their terminal or from a CI runner's step, still ends at the
    /// terminal or the runner.
    #[test]
    fn a_services_start_through_a_runtime_is_under_a_session_and_a_persons_is_not() {
        let processes = listed(&[
            (100, 90, "cin.exe"),
            (90, 80, "python.exe"),
            (80, 4, "wmiprvse.exe"),
            (110, 105, "cin.exe"),
            (105, 104, "python3.12.exe"),
            (104, 600, "svchost.exe"),
            (120, 115, "cin.exe"),
            (115, 114, "sh.exe"),
            (114, 113, "git.exe"),
            (113, 112, "node.exe"),
            (112, 4, "taskhostw.exe"),
            (130, 125, "cin.exe"),
            (125, 124, "perl.exe"),
            (124, 123, "cmd.exe"),
            (123, 4, "wmiprvse.exe"),
            // A person's: the desktop, their terminal, its shell, a runtime.
            (200, 195, "cin.exe"),
            (195, 194, "python.exe"),
            (194, 193, "pwsh.exe"),
            (193, 192, "windowsterminal.exe"),
            (192, 191, "explorer.exe"),
            (191, 4, "userinit.exe"),
            // A CI runner's step, which a service started long before.
            (300, 295, "cin.exe"),
            (295, 294, "bash.exe"),
            (294, 293, "git.exe"),
            (293, 292, "node.exe"),
            (292, 291, "pwsh.exe"),
            (291, 290, "runner.worker.exe"),
            (290, 289, "runner.listener.exe"),
            (289, 600, "svchost.exe"),
            (600, 4, "services.exe"),
        ]);
        let opened = |pid: u32| match pid {
            4 | 80 | 104 | 112 | 123 | 289 | 600 => Opened::Closed,
            _ => Opened::Made(u64::from(pid)),
        };
        // Process 104 runs the task scheduler.
        let under = |own| started_by_a_service(&processes, own, opened, Some(104));
        for (own, through) in [
            (100, "python"),
            (110, "a versioned python"),
            (120, "node, git and sh"),
            (130, "perl and cmd"),
        ] {
            assert!(matches!(under(own), Under::Session(_)), "{through}");
        }
        assert_eq!(under(200), Under::Nothing, "a person's terminal");
        assert_eq!(under(300), Under::Nothing, "a CI runner's step");
        for name in [
            "python.exe",
            "pythonw.exe",
            "py.exe",
            "rscript.exe",
            "wsl.exe",
            "gawk.exe",
        ] {
            assert!(passes_through(name), "{name}");
        }
        for name in [
            "explorer.exe",
            "windowsterminal.exe",
            "code.exe",
            "pythonista.exe",
        ] {
            assert!(!passes_through(name), "{name}");
        }
    }

    /// A harness's job is named for its process's instance: the same id
    /// made at another time is another job.
    #[test]
    fn a_harnesss_job_is_named_for_its_processs_instance() {
        assert_eq!(job_name(42, 7), format!("{JOB_PREFIX}42.7"));
        assert_ne!(job_name(42, 7), job_name(42, 8));
        assert_ne!(job_name(4, 27), job_name(42, 7));
    }
}
