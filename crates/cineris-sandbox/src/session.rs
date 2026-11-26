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
//!   when it is in the job of that name of a process that is running and
//!   this user may open: the jobs of that prefix in the logon session's
//!   namespace of objects are tried, each by the instance its name gives
//!   (where the namespace cannot be listed, the job's name of every process
//!   of the system's list), and a job of that name that cannot be opened or
//!   asked, for any reason but there being none, is not known, which
//!   refuses. Being in any other job (a terminal's, an
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
//!   started is not taken for one. **Only a start after a harness began
//!   counts:** the process the service started on the line must have been
//!   made after the earliest harness of the user's that runs now (a process
//!   this user may open whose job of sessions' commands is there), so that a
//!   terminal a logon task opened, or one a service host opened for
//!   `runas`, before any harness is the person's own; with no harness
//!   running, none counts. **It does not see** a service's start
//!   through another program than an interpreter or a runtime, a process
//!   another of the user's processes starts outside the job (the desktop's
//!   shell asked to open a file, a process made with another parent), nor a
//!   command whose harness has exited, whose job no running process names,
//!   nor a service's start once every harness of the user's has exited, nor
//!   a program a service started before a harness began that is asked later
//!   to run the subcommand (an already-running daemon's).
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
    // A test build's switch, for the tests of what a harness does when its
    // mark fails (`cineris_base::hook::mark_fails`): nothing in a build
    // that is shipped.
    if cineris_base::hook::mark_fails() {
        return Err(io::Error::other(
            "the test hooks' switch made the mark fail",
        ));
    }
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
        let own = Own::read();
        line(std::os::unix::process::parent_id(), |pid| seen(&own, pid))
    }

    /// This process's own file, read once a check and not once a process of
    /// the line: its identity, by which the very file another process runs is
    /// told, and its name. A headless run asks before its first request, so
    /// each process of the line costs four calls (its `stat` opened, read and
    /// closed, and its file's identity), not twelve.
    pub(super) struct Own {
        file: Option<(u64, u64)>,
        name: Option<String>,
    }

    impl Own {
        pub(super) fn read() -> Self {
            Self {
                file: std::fs::metadata("/proc/self/exe")
                    .ok()
                    .map(|file| (file.dev(), file.ino())),
                name: std::env::current_exe().ok().and_then(|exe| {
                    exe.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                }),
            }
        }
    }

    /// The process of the line that runs sessions.
    fn harness() -> Option<u32> {
        let own = Own::read();
        let mut at = std::os::unix::process::parent_id();
        for _ in 0..4096 {
            match seen(&own, at) {
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
    fn named(own: &Own, name: &str) -> bool {
        let cut = |text: &str| -> String { text.bytes().take(15).map(char::from).collect() };
        name == "cineris"
            || name == "cin"
            || own.name.as_deref().is_some_and(|own| cut(own) == name)
    }

    /// Whether the file `pid` runs is named as Cineris is (`cineris`, `cin`
    /// or this binary's own name), its name and its file together: a
    /// process named `cin` that runs another program (a user's own
    /// `#!/bin/sh` wrapper of that name, which runs as `/bin/sh`) is not
    /// Cineris (the full review of the third part). A file that cannot be
    /// read (another user's process) is taken as Cineris's: what it is is
    /// not known.
    fn file_named(own: &Own, pid: u32) -> bool {
        match std::fs::read_link(format!("/proc/{pid}/exe")) {
            Ok(file) => {
                let name = file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // A file replaced or removed since it was started.
                let name = name.strip_suffix(" (deleted)").unwrap_or(&name);
                name == "cineris" || name == "cin" || own.name.as_deref() == Some(name)
            }
            Err(_) => true,
        }
    }

    /// Whether `pid` runs the very file this process runs.
    fn same_binary(own: &Own, pid: u32) -> bool {
        let (Some(own), Ok(theirs)) = (own.file, std::fs::metadata(format!("/proc/{pid}/exe")))
        else {
            return false;
        };
        own == (theirs.dev(), theirs.ino())
    }

    /// `/proc/<pid>/stat`, read whole in one call: the kernel writes the
    /// record at the first read, and `/proc` gives no size to read by, so
    /// `read_to_string` asks for one and then grows its buffer from 32 bytes
    /// (eight calls a process where this makes three). Text that is not
    /// UTF-8 (a name of any bytes) is an error, as `read_to_string` made it:
    /// the line is then not known, which refuses.
    fn stat_of(pid: u32) -> io::Result<String> {
        use std::io::Read as _;
        let mut file = std::fs::File::open(format!("/proc/{pid}/stat"))?;
        let mut buffer = [0_u8; 1024];
        let mut text = Vec::new();
        loop {
            let read = file.read(&mut buffer)?;
            text.extend_from_slice(&buffer[..read]);
            // The record ends in a line feed: read to it, and no further.
            if read == 0 || text.last() == Some(&b'\n') {
                break;
            }
        }
        String::from_utf8(text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn seen(own: &Own, pid: u32) -> Seen {
        let stat = match stat_of(pid) {
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
        let cineris = same_binary(own, pid) || (named(own, name) && file_named(own, pid));
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
            let read = super::Own::read();
            assert!(super::named(&read, "cineris") && super::named(&read, "cin"));
            for other in ["bash", "cineris-x", "Cineris", "ci", ""] {
                assert!(!super::named(&read, other), "{other}");
            }
            let own = std::env::current_exe().unwrap();
            let own = own.file_name().unwrap().to_string_lossy();
            let cut: String = own.bytes().take(15).map(char::from).collect();
            assert!(super::named(&read, &cut), "{cut}");
            assert!(super::same_binary(&read, std::process::id()));
            assert!(!super::same_binary(&read, 1));
            assert!(super::file_named(&read, std::process::id()));
        }

        /// A process's `stat` is read whole in one go, to its line feed: its
        /// own id and its parent's are in it, as the kernel wrote them.
        #[test]
        fn a_processs_stat_is_read_whole() {
            let pid = std::process::id();
            let stat = super::stat_of(pid).unwrap();
            assert!(stat.starts_with(&format!("{pid} (")), "{stat}");
            assert!(stat.ends_with('\n'), "{stat}");
            let seen = super::seen(&super::Own::read(), pid);
            assert!(
                matches!(seen, super::Seen::Of { parent, .. } if parent == std::os::unix::process::parent_id()),
                "{seen:?}"
            );
        }

        /// A process of the line whose name is not UTF-8 is not read as no
        /// harness: the line is not known, which refuses, as it was before
        /// `stat` was read in one call (a copy of the harness under such a
        /// name would otherwise pass for another program).
        #[test]
        fn sys_a_name_that_is_not_utf8_leaves_the_line_not_known() {
            use std::os::unix::ffi::OsStrExt as _;
            use std::os::unix::fs::PermissionsExt as _;
            let scratch = cineris_testkit::scratch::Scratch::outside(
                "cineris-sandbox-session",
                "sys_a_name_that_is_not_utf8_leaves_the_line_not_known",
            );
            let script = scratch.join(std::ffi::OsStr::from_bytes(b"cin\xff"));
            std::fs::write(&script, "#!/bin/sh\nread -r line\n").unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut child = std::process::Command::new(&script)
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let pid = child.id();
            let comm = std::fs::read(format!("/proc/{pid}/comm")).unwrap();
            let seen = super::seen(&super::Own::read(), pid);
            drop(child.stdin.take());
            child.wait().unwrap();
            assert_eq!(comm, b"cin\xff\n");
            assert_eq!(seen, super::Seen::Failed);
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
            let seen = super::seen(&super::Own::read(), pid);
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
    /// When it was made, as a `FILETIME`'s count: what its job of
    /// sessions' commands is named for, if it is a harness.
    made: u64,
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

/// What opening a process of the line of parents tells of it.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
#[derive(Debug, Clone)]
enum Described {
    /// Its parent, its program's name and when it was made.
    Is(Listed),
    /// No process has its id.
    Gone,
    /// It runs and cannot be opened (another user's, the system's, a
    /// service's host): only the system's list gives its name.
    Closed,
}

/// The processes of the line of parents from `own` that
/// [`started_by_a_service`] reads, each described by opening it: this
/// process, and its parents as far as they pass a command through
/// ([`passes_through`]), with the first that does not. `None` where it
/// meets one that cannot be opened, whose name only the system's list
/// gives: the list is then taken, as it was for every check before (since
/// step 28's third pull request: a headless run's line from a terminal or a
/// runner's step is a few of the user's own processes, and the list, every
/// process and thread of the system, cost its start 0.6 to 0.8 ms on a
/// hosted runner).
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
fn line_of(own: u32, describe: impl Fn(u32) -> Described) -> Option<Vec<Listed>> {
    let mut line = Vec::new();
    let mut at = own;
    for _ in 0..=LINE_OF_PARENTS {
        match describe(at) {
            Described::Is(process) => {
                let parent = process.parent;
                let goes_on = at == own || passes_through(&process.name);
                line.push(process);
                if !goes_on || parent == 0 || parent == at {
                    break;
                }
                at = parent;
            }
            Described::Gone => break,
            Described::Closed => return None,
        }
    }
    Some(line)
}

/// The process instance a job's name in the namespace of objects names,
/// when it is a harness's job of sessions' commands ([`job_name`], with
/// its `Local\` taken off, as the namespace lists it): `None` for any other
/// name, and for one [`job_name`] would not write so (a sign, a zero in
/// front).
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's jobs"))]
fn harness_named(name: &str) -> Option<(u32, u64)> {
    let prefix = JOB_PREFIX.strip_prefix(r"Local\")?;
    let (pid, made) = name.strip_prefix(prefix)?.split_once('.')?;
    let (pid, made) = (pid.parse().ok()?, made.parse().ok()?);
    (job_name(pid, made).strip_prefix(r"Local\") == Some(name)).then_some((pid, made))
}

/// Whether this process, `own`, was started by a service that starts what
/// a command asks of it, outside any job (WMI's provider host, the task
/// host, the service host that runs the task scheduler, which `scheduler`
/// asks for, only once the line reaches a service host: `None` where it
/// could not be asked, when any service host counts), directly or
/// through command interpreters and runtimes ([`passes_through`]: `cmd /c
/// cin …`, `python -c …`): the full review of the third part, H1, and its
/// narrow re-check, M2'. The line is followed up through those alone, each
/// made before the one below it (a later process may have the id of a
/// parent that has gone): any other program ends it, so that a person's
/// terminal, an editor or a CI runner that a service started long before
/// is not taken for one.
///
/// **Only a start after a harness began counts** (the false refusals of the
/// narrow re-check of the third part): the process the service started on
/// the line must have been made after `began`, when the earliest harness of
/// this user that runs now was made, and with no harness running none
/// counts. A session's command asks a service while its harness runs, so
/// what the service starts for it is made after; a terminal that a logon
/// task opened (`taskhostw.exe`, or the scheduler's host, then `cmd`), or
/// one a service host opened for `runas`, before any harness, is the
/// person's own, and the subcommands they type in it are theirs.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's services"))]
fn started_by_a_service(
    listed: &[Listed],
    own: u32,
    opened: impl Fn(u32) -> Opened,
    scheduler: impl Fn() -> Option<u32>,
    began: Option<u64>,
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
        // The service manager is asked which host runs the scheduler only
        // here, where the line reached a service host: a line that never
        // does (a terminal's, a runner's) asks it nothing. The line ends at
        // the first service host, so it is asked once at most.
        let asked = (name == "svchost.exe").then(&scheduler);
        let schedules =
            asked.is_some_and(|scheduler| scheduler.is_none_or(|scheduler| scheduler == parent));
        if SERVICE_STARTERS.contains(&name) || schedules {
            // `made` is when the process the service started, `at`, was.
            if began.is_none_or(|began| made <= began) {
                return Under::Nothing;
            }
            let which = if schedules && asked == Some(None) {
                "a service host, and which service it runs could not be asked"
            } else if schedules {
                "the task scheduler's service host"
            } else {
                "a service's host of what it starts"
            };
            return Under::Session(format!(
                "process {parent}, `{name}`, started it or a process above it while a harness \
                 ran: {which}, which starts what a session's command asks of it outside the \
                 session's job"
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
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr;
    use std::sync::OnceLock;

    use super::{Listed, Opened, Under, job_name, started_by_a_service};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{NtOpenDirectoryObject, NtQueryDirectoryObject};
    use windows_sys::Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SystemProcessInformation,
    };
    use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
    use windows_sys::Win32::Foundation::{
        ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER, FALSE, FILETIME,
        HANDLE, LocalFree, STATUS_INFO_LENGTH_MISMATCH, SetLastError, UNICODE_STRING,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAce, CreateWellKnownSid,
        GetLengthSid, GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor, PSID,
        SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SetSecurityDescriptorControl,
        SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenSessionId, TokenUser,
        WinCreatorOwnerRightsSid,
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
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION;

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

    /// `winnt.h`'s `SECURITY_DESCRIPTOR_REVISION`.
    const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

    /// The job's security: open by its name to its user and its owner only,
    /// and only to ask ([`BY_NAME`]); protected, so that nothing is
    /// inherited. The same as the text `D:P(A;;BY_NAME;;;<user>)(A;;
    /// BY_NAME;;;OW)`, built from its parts: the text's first conversion in
    /// a process (with the user's SID written out for it) cost a headless
    /// run's start 0.5 ms on a hosted runner (step 28's third pull request).
    pub(super) struct OwnerOnly {
        /// What `lpSecurityDescriptor` is given.
        descriptor: *mut core::ffi::c_void,
        /// A descriptor built here and the list it points at, which live
        /// as long as it; `None` for one from text, which `LocalFree`
        /// frees.
        built: Option<(Box<SECURITY_DESCRIPTOR>, Vec<u32>)>,
    }

    impl Drop for OwnerOnly {
        fn drop(&mut self) {
            if self.built.is_none() {
                // SAFETY: the descriptor was given by
                // `ConvertStringSecurityDescriptorToSecurityDescriptorW`, to
                // be freed with `LocalFree`, once.
                unsafe { LocalFree(self.descriptor) };
            }
        }
    }

    impl OwnerOnly {
        pub(super) fn new() -> io::Result<Self> {
            let failed = |asked: i32| {
                if asked == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            };
            // Room for the user's SID, aligned for the structure's pointer;
            // filled in place, as the SID's pointer points into it.
            let mut user = [0_u64; 32];
            user_into(&mut user)?;
            // SAFETY: the call wrote a `TOKEN_USER` at the buffer's start,
            // whose SID points into the same buffer, which has not moved and
            // lives through every use below (each entry copies the SID).
            let user_sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
            // `OW`, OWNER RIGHTS (S-1-3-4): room for the largest SID.
            let mut owner = [0_u32; 17];
            let mut owner_size = u32::try_from(std::mem::size_of_val(&owner)).unwrap_or(0);
            // SAFETY: a buffer as large as said, for a SID that needs no
            // domain.
            failed(unsafe {
                CreateWellKnownSid(
                    WinCreatorOwnerRightsSid,
                    ptr::null_mut(),
                    owner.as_mut_ptr().cast(),
                    &raw mut owner_size,
                )
            })?;
            let owner_sid: PSID = owner.as_mut_ptr().cast();
            // The list: its header, and each entry's header with its SID in
            // place of the entry's last field.
            let entry = |sid: PSID| {
                // SAFETY: a valid SID, made or given above.
                let sid = usize::try_from(unsafe { GetLengthSid(sid) }).unwrap_or(0);
                std::mem::size_of::<ACCESS_ALLOWED_ACE>() - std::mem::size_of::<u32>() + sid
            };
            let length = std::mem::size_of::<ACL>() + entry(user_sid) + entry(owner_sid);
            let mut list = vec![0_u32; length.div_ceil(4)];
            let length = u32::try_from(list.len() * 4).unwrap_or(0);
            let list_pointer: *mut ACL = list.as_mut_ptr().cast();
            // SAFETY: the list's memory is as large as said, aligned for an
            // `ACL`, and lives as long as the descriptor; each SID is valid.
            unsafe {
                failed(InitializeAcl(list_pointer, length, ACL_REVISION))?;
                failed(AddAccessAllowedAce(
                    list_pointer,
                    ACL_REVISION,
                    BY_NAME,
                    user_sid,
                ))?;
                failed(AddAccessAllowedAce(
                    list_pointer,
                    ACL_REVISION,
                    BY_NAME,
                    owner_sid,
                ))?;
            }
            // SAFETY: the structure is plain data, for which all zeroes is
            // valid.
            let mut descriptor: Box<SECURITY_DESCRIPTOR> = Box::new(unsafe { std::mem::zeroed() });
            let pointer: *mut core::ffi::c_void = (&raw mut *descriptor).cast();
            // SAFETY: `pointer` is a live descriptor, which points at the
            // list; both are kept in what is returned.
            unsafe {
                failed(InitializeSecurityDescriptor(
                    pointer,
                    SECURITY_DESCRIPTOR_REVISION,
                ))?;
                failed(SetSecurityDescriptorDacl(pointer, 1, list_pointer, 0))?;
                failed(SetSecurityDescriptorControl(
                    pointer,
                    SE_DACL_PROTECTED,
                    SE_DACL_PROTECTED,
                ))?;
            }
            Ok(Self {
                descriptor: pointer,
                built: Some((descriptor, list)),
            })
        }

        /// A descriptor from its text (a test's: one nobody may open).
        #[cfg(test)]
        pub(super) fn from_sddl(text: &str) -> io::Result<Self> {
            use windows_sys::Win32::Security::Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            };
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
            Ok(Self {
                descriptor,
                built: None,
            })
        }

        pub(super) fn attributes(&self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                    .unwrap_or(u32::MAX),
                lpSecurityDescriptor: self.descriptor,
                bInheritHandle: FALSE,
            }
        }
    }

    /// This process's user, as the token gives it, written into `buffer`: a
    /// `TOKEN_USER` at its start, whose SID points within it (so the buffer
    /// is filled where it stays, and not returned by value).
    fn user_into(buffer: &mut [u64; 32]) -> io::Result<()> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: the pseudo-handle is always valid, and `token` is a live
        // handle's place.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `token` was just opened and nothing else owns it.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut needed = 0u32;
        // SAFETY: `token` is open to query, and the buffer is as large as
        // said and aligned for `TOKEN_USER`.
        let asked = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                u32::try_from(std::mem::size_of_val(buffer)).unwrap_or(0),
                &raw mut needed,
            )
        };
        if asked == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
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

    /// Every process of the system, with its parent, its program's file
    /// name and when it was made, in one call (`SystemProcessInformation`).
    /// The toolhelp snapshot this replaced mapped its copy again for each
    /// process it listed, and each process was then opened to ask when it
    /// was made: 1.9 ms and 0.35 ms of a headless run's start on a hosted
    /// runner with 144 processes (step 28's third pull request).
    pub(super) fn processes() -> io::Result<Vec<Listed>> {
        // Room for several hundred processes and their threads, grown to
        // what the call says it needs when the system has more. It is not
        // zeroed: the call writes what it fills, and only that is read.
        let mut size: u32 = 1024 * 1024;
        for _ in 0..8 {
            let mut buffer: Vec<u64> = Vec::with_capacity((size as usize).div_ceil(8));
            let mut filled = 0u32;
            // SAFETY: the buffer's room is as large as said, aligned for
            // the records, and lives through the call; `filled` is a live
            // `u32`.
            let status = unsafe {
                NtQuerySystemInformation(
                    SystemProcessInformation,
                    buffer.as_mut_ptr().cast(),
                    size,
                    &raw mut filled,
                )
            };
            if status == STATUS_INFO_LENGTH_MISMATCH {
                size = filled.max(size).saturating_mul(2);
                continue;
            }
            if status < 0 {
                return Err(io::Error::other(format!(
                    "the system's processes could not be asked (status {status:#010x})"
                )));
            }
            let filled = usize::try_from(filled)
                .unwrap_or(usize::MAX)
                .min(buffer.capacity() * 8);
            // SAFETY: the call wrote the first `filled` bytes of the room,
            // which is at least that large.
            let written =
                unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), filled) };
            return Ok(records(written));
        }
        Err(io::Error::other(
            "the system's processes outgrew every buffer they were asked into",
        ))
    }

    /// The records the call wrote, `written`: each read where it lies, and
    /// only from within what was written.
    fn records(written: &[u8]) -> Vec<Listed> {
        let bytes = written.len();
        let base = written.as_ptr();
        let record = std::mem::size_of::<SYSTEM_PROCESS_INFORMATION>();
        let mut listed = Vec::new();
        let mut at = 0_usize;
        while at.checked_add(record).is_some_and(|end| end <= bytes) {
            // SAFETY: the record lies within what the call wrote (checked
            // above), and is read whatever its alignment.
            let entry: SYSTEM_PROCESS_INFORMATION =
                unsafe { ptr::read_unaligned(base.add(at).cast()) };
            // `CreateTime`, in the reserved bytes after the two counts at
            // its start (`winternl.h` leaves it unnamed): the same count
            // `GetProcessTimes` gives, which a harness names its job by.
            let mut made = [0_u8; 8];
            made.copy_from_slice(&entry.Reserved1[24..32]);
            // The name points into the same buffer; one that does not, or
            // runs past what was written, is left empty.
            let length = usize::from(entry.ImageName.Length);
            let name = (entry.ImageName.Buffer as usize)
                .checked_sub(base as usize)
                .and_then(|offset| written.get(offset..offset.checked_add(length)?))
                .map_or_else(String::new, |raw| {
                    let units: Vec<u16> = raw
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|unit| u16::from_le_bytes(*unit))
                        .collect();
                    String::from_utf16_lossy(&units).to_lowercase()
                });
            listed.push(Listed {
                pid: u32::try_from(entry.UniqueProcessId as usize).unwrap_or(u32::MAX),
                // `InheritedFromUniqueProcessId`, unnamed in `winternl.h`.
                parent: u32::try_from(entry.Reserved2 as usize).unwrap_or(u32::MAX),
                name,
                made: u64::from_le_bytes(made),
            });
            if entry.NextEntryOffset == 0 {
                break;
            }
            at += entry.NextEntryOffset as usize;
        }
        listed
    }

    pub(super) fn under_a_session() -> Under {
        // The system's list of processes, taken once at most, and only
        // where the cheaper reads cannot answer.
        let list = std::cell::OnceCell::new();
        let listed = || {
            list.get_or_init(processes)
                .as_ref()
                .map_err(|error| format!("the system's processes could not be listed: {error}"))
        };
        // The running harnesses that could hold it: the jobs of sessions'
        // commands in this logon session's namespace, by their names; where
        // the namespace cannot be listed, every process of the list, each
        // tried by its job's name.
        let harnesses: Vec<(u32, u64)> = match named_jobs() {
            Ok(named) => named,
            Err(_) => match listed() {
                Ok(listed) => listed
                    .iter()
                    .map(|process| (process.pid, process.made))
                    .collect(),
                Err(error) => return Under::Unknown(error),
            },
        };
        // When the earliest harness of this user that runs now was made.
        let mut began: Option<u64> = None;
        for (pid, made) in harnesses {
            match in_the_job_of(pid, made) {
                Job::None => {}
                Job::Elsewhere => began = Some(began.map_or(made, |began| began.min(made))),
                Job::Holds(under) => return under,
            }
        }
        // SAFETY: plain call.
        let own = unsafe { GetCurrentProcessId() };
        // The line of parents, each process on it opened, or the list where
        // one cannot be; the service manager is asked only where the line
        // reaches a service host.
        let line = super::line_of(own, described);
        let read: &[Listed] = match &line {
            Some(line) => line,
            None => match listed() {
                Ok(listed) => listed,
                Err(error) => return Under::Unknown(error),
            },
        };
        started_by_a_service(read, own, opened, scheduler, began)
    }

    /// `winnt.h`'s `DIRECTORY_QUERY`: the right to list a directory of
    /// objects.
    const DIRECTORY_QUERY: u32 = 0x0001;

    /// `OBJ_CASE_INSENSITIVE`.
    const OBJ_CASE_INSENSITIVE: u32 = 0x0040;

    /// What `NtQueryDirectoryObject` writes for each object
    /// (`OBJECT_DIRECTORY_INFORMATION`, which `windows-sys` does not
    /// declare): its name and its type's.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct DirectoryEntry {
        name: UNICODE_STRING,
        kind: UNICODE_STRING,
    }

    /// `ntstatus.h`'s `STATUS_MORE_ENTRIES` and `STATUS_NO_MORE_ENTRIES`.
    const STATUS_MORE_ENTRIES: i32 = 0x0000_0105;
    const STATUS_NO_MORE_ENTRIES: i32 = 0x8000_001A_u32.cast_signed();

    /// The type's name a job has in a directory of objects.
    const JOB_TYPE: &str = "Job";

    /// The text of a counted string the system wrote.
    fn counted(string: &UNICODE_STRING) -> String {
        if string.Buffer.is_null() {
            return String::new();
        }
        // SAFETY: the system wrote `Length` bytes of text at `Buffer`, in
        // the buffer that holds the entry, which lives through this read.
        let units =
            unsafe { std::slice::from_raw_parts(string.Buffer, usize::from(string.Length) / 2) };
        String::from_utf16_lossy(units)
    }

    /// This process's logon session's id, as its token gives it.
    fn session_id() -> io::Result<u32> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: the pseudo-handle is always valid, and `token` is a live
        // handle's place.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `token` was just opened and nothing else owns it.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut session = 0_u32;
        let mut needed = 0_u32;
        // SAFETY: `token` is open to query, and `session` is a live `u32`,
        // as large as said.
        let asked = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenSessionId,
                (&raw mut session).cast(),
                4,
                &raw mut needed,
            )
        };
        if asked == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(session)
    }

    /// The harnesses' jobs of sessions' commands in this logon session's
    /// own namespace of objects, `\Sessions\<id>\BaseNamedObjects`, where
    /// `Local\` names them: each by the process instance it is named for,
    /// read from its name. A few calls, where the system's list of
    /// processes lists every thread too (0.3 ms cold on a hosted runner,
    /// with 359 objects, against 0.6 to 0.8 ms; step 28's third pull
    /// request). `Err` where the namespace cannot be listed, and in session
    /// 0, which the services share and whose `Local\` is another directory:
    /// every process of the list is tried then, as before.
    fn named_jobs() -> io::Result<Vec<(u32, u64)>> {
        let session = session_id()?;
        if session == 0 {
            return Err(io::Error::other("session 0's namespace is the services'"));
        }
        let path: Vec<u16> = format!(r"\Sessions\{session}\BaseNamedObjects")
            .encode_utf16()
            .collect();
        let length = u16::try_from(path.len() * 2).map_err(io::Error::other)?;
        let name = UNICODE_STRING {
            Length: length,
            MaximumLength: length,
            Buffer: path.as_ptr().cast_mut(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: u32::try_from(std::mem::size_of::<OBJECT_ATTRIBUTES>()).unwrap_or(0),
            RootDirectory: ptr::null_mut(),
            ObjectName: &raw const name,
            Attributes: OBJ_CASE_INSENSITIVE,
            SecurityDescriptor: ptr::null(),
            SecurityQualityOfService: ptr::null(),
        };
        let mut directory: HANDLE = ptr::null_mut();
        // SAFETY: the attributes and the name they point at live through
        // the call, and `directory` is a live handle's place.
        let status = unsafe {
            NtOpenDirectoryObject(&raw mut directory, DIRECTORY_QUERY, &raw const attributes)
        };
        if status < 0 {
            return Err(io::Error::other(format!(
                "the namespace of objects could not be opened (status {status:#010x})"
            )));
        }
        // SAFETY: `directory` was just opened and nothing else owns it.
        let directory = unsafe { OwnedHandle::from_raw_handle(directory) };
        let mut buffer: Vec<u64> = vec![0; 64 * 1024 / 8];
        let room = u32::try_from(buffer.len() * 8).unwrap_or(0);
        let mut context = 0_u32;
        let mut restart = true;
        let mut found = Vec::new();
        // Bounded: a namespace that keeps growing as it is listed is not
        // listed for ever, and the list is taken in its place.
        for _ in 0..4096 {
            let mut written = 0_u32;
            // SAFETY: the buffer is as large as said and aligned for the
            // entries; `context` and `written` are live `u32`s.
            let status = unsafe {
                NtQueryDirectoryObject(
                    directory.as_raw_handle(),
                    buffer.as_mut_ptr().cast(),
                    room,
                    false,
                    restart,
                    &raw mut context,
                    &raw mut written,
                )
            };
            restart = false;
            if status == STATUS_NO_MORE_ENTRIES {
                return Ok(found);
            }
            if status < 0 {
                return Err(io::Error::other(format!(
                    "the namespace of objects could not be listed (status {status:#010x})"
                )));
            }
            let entries = buffer.as_ptr().cast::<DirectoryEntry>();
            let most = buffer.len() * 8 / std::mem::size_of::<DirectoryEntry>();
            for at in 0..most {
                // SAFETY: within the buffer; the list of entries ends with
                // one of zeroes, before the names they point at.
                let entry = unsafe { ptr::read_unaligned(entries.add(at)) };
                if entry.name.Buffer.is_null() {
                    break;
                }
                if counted(&entry.kind) == JOB_TYPE
                    && let Some(harness) = super::harness_named(&counted(&entry.name))
                {
                    found.push(harness);
                }
            }
            if status != STATUS_MORE_ENTRIES {
                return Ok(found);
            }
        }
        Err(io::Error::other("the namespace of objects did not end"))
    }

    /// `PROCESS_BASIC_INFORMATION`, which `windows-sys` keeps under a
    /// feature of its own: its parent's id is the last field.
    #[repr(C)]
    struct BasicInformation {
        exit_status: i32,
        environment_block: *mut core::ffi::c_void,
        affinity: usize,
        priority: i32,
        process: usize,
        parent: usize,
    }

    /// The process `pid`, opened to ask its parent, its program's name and
    /// when it was made, as the system's list gives them.
    fn described(pid: u32) -> super::Described {
        use super::Described;
        // SAFETY: plain values.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == i32::try_from(ERROR_INVALID_PARAMETER).ok() {
                Described::Gone
            } else {
                Described::Closed
            };
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let process = unsafe { OwnedHandle::from_raw_handle(handle) };
        let Some(made) = made_at(process.as_raw_handle()) else {
            return Described::Closed;
        };
        // SAFETY: the structure is plain data, for which all zeroes is valid.
        let mut basic: BasicInformation = unsafe { std::mem::zeroed() };
        // SAFETY: `process` is open with the limited right to ask, which
        // this class takes, and the structure is as large as said.
        let status = unsafe {
            NtQueryInformationProcess(
                process.as_raw_handle(),
                ProcessBasicInformation,
                (&raw mut basic).cast(),
                u32::try_from(std::mem::size_of::<BasicInformation>()).unwrap_or(0),
                ptr::null_mut(),
            )
        };
        if status < 0 {
            return Described::Closed;
        }
        let mut path = [0_u16; 1024];
        let mut length = u32::try_from(path.len()).unwrap_or(0);
        // SAFETY: `process` is open with the limited right to ask, and the
        // buffer holds `length` characters.
        let asked = unsafe {
            QueryFullProcessImageNameW(
                process.as_raw_handle(),
                PROCESS_NAME_WIN32,
                path.as_mut_ptr(),
                &raw mut length,
            )
        };
        if asked == 0 {
            return Described::Closed;
        }
        let Some(path) = path.get(..usize::try_from(length).unwrap_or(usize::MAX)) else {
            return Described::Closed;
        };
        let path = String::from_utf16_lossy(path);
        let name = path.rsplit('\\').next().unwrap_or(&path).to_lowercase();
        Described::Is(Listed {
            pid,
            parent: u32::try_from(basic.parent).unwrap_or(u32::MAX),
            name,
            made,
        })
    }

    /// Where this process stands to the job of sessions' commands of the
    /// process `pid`, made at `made`.
    enum Job {
        /// There is no such job: `pid` runs no sessions.
        None,
        /// `pid` runs sessions, and this process is not in its job.
        Elsewhere,
        /// This process is in it, or that cannot be told.
        Holds(Under),
    }

    /// Whether this process is in the job of sessions' commands of the
    /// harness `pid`, made at `made`. A job of that name that cannot be
    /// opened or asked, for any reason but there being none, is not known,
    /// which refuses (the full review of the third part: a job made to deny
    /// its opening must not read as no job).
    ///
    /// **Only a process this user may open is a harness of this one's**
    /// (another user's, the system's, are not: its commands run as its
    /// user). The job's name is tried first, with when the process was made
    /// as the list of processes gives it, and the process is opened only
    /// where there is a job of that name, or one that could not be told
    /// from none: most processes run no sessions, and opening each cost a
    /// headless run's start 0.35 ms on a hosted runner.
    fn in_the_job_of(pid: u32, made: u64) -> Job {
        // The process, this user's and the same instance (a process that
        // has gone, whose id is another's now, is not).
        let harness = || opened(pid) == Opened::Made(made);
        let name = wide(&job_name(pid, made));
        // SAFETY: plain values, and a name that ends in a nul and lives
        // through the call.
        let handle = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, name.as_ptr()) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == i32::try_from(ERROR_FILE_NOT_FOUND).ok() || !harness() {
                return Job::None;
            }
            return Job::Holds(Under::Unknown(format!(
                "the job of process {pid}'s sessions' commands could not be opened: {error}"
            )));
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let job = unsafe { OwnedHandle::from_raw_handle(handle) };
        if !harness() {
            return Job::None;
        }
        let mut inside = FALSE;
        // SAFETY: the pseudo-handle has every right, `job` is open with
        // `JOB_OBJECT_QUERY`, both live through the call, and `inside` is a
        // live `BOOL`.
        let asked =
            unsafe { IsProcessInJob(GetCurrentProcess(), job.as_raw_handle(), &raw mut inside) };
        if asked == 0 {
            return Job::Holds(Under::Unknown(format!(
                "whether it is in the job of process {pid}'s sessions' commands could not be \
                 asked: {}",
                io::Error::last_os_error()
            )));
        }
        if inside == 0 {
            return Job::Elsewhere;
        }
        Job::Holds(Under::Session(format!(
            "it is in the job of process {pid}'s sessions' commands"
        )))
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
                made: u64::from(*pid),
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
        let under = |own, scheduler: Option<u32>| {
            started_by_a_service(&processes, own, opened, move || scheduler, Some(0))
        };
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
        let under = |own| started_by_a_service(&processes, own, opened, || Some(104), Some(0));
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

    /// A service's start counts only when what the service started on the
    /// line was made after a harness of the user's that runs now began:
    /// a terminal a logon task opened (the task host, then `cmd`), or one a
    /// service host opened for `runas` where the scheduler's host could not
    /// be asked, before any harness, is the person's own; the same opened
    /// after one began is a session's command; and with no harness running,
    /// nothing a service started counts (the false refusals of the narrow
    /// re-check of the third part).
    #[test]
    fn a_services_start_counts_only_after_a_running_harness_began() {
        let processes = listed(&[
            // A logon task's terminal, made at 50.
            (300, 50, "cin.exe"),
            (50, 40, "cmd.exe"),
            (40, 4, "taskhostw.exe"),
            // A terminal a service host opened, made at 60.
            (310, 60, "cin.exe"),
            (60, 45, "pwsh.exe"),
            (45, 4, "svchost.exe"),
            // WMI's start of the subcommand itself, made at 320.
            (320, 46, "cin.exe"),
            (46, 4, "wmiprvse.exe"),
        ]);
        let opened = |pid: u32| match pid {
            4 | 40 | 45 | 46 => Opened::Closed,
            _ => Opened::Made(u64::from(pid)),
        };
        let under = |own, scheduler: Option<u32>, began| {
            matches!(
                started_by_a_service(&processes, own, opened, move || scheduler, began),
                Under::Session(_)
            )
        };
        assert!(!under(300, Some(45), Some(100)), "a logon task's terminal");
        assert!(under(300, Some(45), Some(30)), "a task's, after a harness");
        assert!(!under(300, Some(45), None), "no harness running");
        assert!(!under(310, None, Some(100)), "a runas terminal, not asked");
        assert!(under(310, None, Some(55)), "a service host's, after");
        assert!(under(320, Some(45), Some(100)), "WMI's, after");
        assert!(!under(320, Some(45), Some(320)), "WMI's, not after");
        assert!(!under(320, Some(45), None), "WMI's, no harness running");
    }

    /// The service manager is asked which host runs the task scheduler only
    /// when the line reaches a service host, and once: a headless run's
    /// line from a terminal or a runner's step, which never does, pays
    /// nothing for it (its start-up, since step 28's third pull request).
    #[test]
    fn the_scheduler_is_asked_only_where_the_line_reaches_a_service_host() {
        let processes = listed(&[
            (100, 90, "cin.exe"),
            (90, 80, "pwsh.exe"),
            (80, 70, "windowsterminal.exe"),
            (200, 190, "cin.exe"),
            (190, 180, "cmd.exe"),
            (180, 4, "wmiprvse.exe"),
            (300, 290, "cin.exe"),
            (290, 280, "cmd.exe"),
            (280, 600, "svchost.exe"),
            (600, 4, "services.exe"),
        ]);
        let opened = |pid: u32| match pid {
            4 | 180 | 280 | 600 => Opened::Closed,
            _ => Opened::Made(u64::from(pid)),
        };
        let asked = std::cell::Cell::new(0);
        let scheduler = || {
            asked.set(asked.get() + 1);
            Some(280)
        };
        assert_eq!(
            started_by_a_service(&processes, 100, opened, scheduler, Some(0)),
            Under::Nothing
        );
        assert!(matches!(
            started_by_a_service(&processes, 200, opened, scheduler, Some(0)),
            Under::Session(_)
        ));
        assert_eq!(asked.get(), 0, "no service host on either line");
        assert!(matches!(
            started_by_a_service(&processes, 300, opened, scheduler, Some(0)),
            Under::Session(_)
        ));
        assert_eq!(asked.get(), 1, "asked once, at the service host");
    }

    /// The line of parents a service's start is read on is described by
    /// opening each process of it, as far as they pass a command through,
    /// with the first that does not; one that cannot be opened (a service's
    /// host, whose name only the list gives) makes it take the list. Read
    /// on the line described, a check says what it says on the list.
    #[test]
    fn the_line_is_described_by_opening_each_process_and_the_list_taken_at_a_closed_one() {
        let processes = listed(&[
            (100, 90, "cin.exe"),
            (90, 80, "pwsh.exe"),
            (80, 70, "windowsterminal.exe"),
            (70, 4, "explorer.exe"),
            (200, 190, "cin.exe"),
            (190, 180, "cmd.exe"),
            (180, 4, "wmiprvse.exe"),
            (300, 290, "cin.exe"),
            (290, 9999, "cmd.exe"),
            (400, 390, "cin.exe"),
            (390, 380, "cmd.exe"),
            (380, 4, "taskhostw.exe"),
        ]);
        let closed = [4, 180];
        let describe = |pid: u32| {
            if closed.contains(&pid) {
                return Described::Closed;
            }
            processes
                .iter()
                .find(|process| process.pid == pid)
                .cloned()
                .map_or(Described::Gone, Described::Is)
        };
        let pids = |line: Option<Vec<Listed>>| {
            line.map(|line| line.iter().map(|process| process.pid).collect::<Vec<_>>())
        };
        // A terminal ends it: what is above the terminal is not read.
        assert_eq!(pids(line_of(100, describe)), Some(vec![100, 90, 80]));
        // WMI's host cannot be opened: the list is taken.
        assert_eq!(pids(line_of(200, describe)), None);
        // A parent that has gone ends it.
        assert_eq!(pids(line_of(300, describe)), Some(vec![300, 290]));
        // The task host, the user's own, is described and read.
        assert_eq!(pids(line_of(400, describe)), Some(vec![400, 390, 380]));
        let opened = |pid: u32| {
            if closed.contains(&pid) {
                Opened::Closed
            } else {
                Opened::Made(u64::from(pid))
            }
        };
        for own in [100, 200, 300, 400] {
            let line = line_of(own, describe).unwrap_or_else(|| processes.clone());
            assert_eq!(
                started_by_a_service(&line, own, opened, || Some(0), Some(0)),
                started_by_a_service(&processes, own, opened, || Some(0), Some(0)),
                "{own}"
            );
        }
    }

    /// A harness's job is found in the namespace of objects by its name,
    /// which gives its process's instance back; any other name, and one
    /// `job_name` would not write so, is no harness's.
    #[test]
    fn a_harnesss_job_is_known_by_its_name_in_the_namespace() {
        let listed_name = |pid, made| {
            job_name(pid, made)
                .strip_prefix(r"Local\")
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            harness_named(&listed_name(4242, 133_000_000_000)),
            Some((4242, 133_000_000_000))
        );
        for other in [
            "Cineris.SessionCommands.42",
            "Cineris.SessionCommands.042.7",
            "Cineris.SessionCommands.+42.7",
            "Cineris.SessionCommands.42.7.1",
            "Cineris.SessionCommands.x.7",
            "Other.42.7",
            r"Local\Cineris.SessionCommands.42.7",
        ] {
            assert_eq!(harness_named(other), None, "{other}");
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
