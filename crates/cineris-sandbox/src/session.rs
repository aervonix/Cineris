//! Whether this process runs under a session's command: what the
//! subcommands that approve, trust or answer for the person
//! (`cineris ext add`, `remove` and `disable`, `cineris permissions trust`,
//! `untrust`, `revoke` and `forget`, `cineris setup`, and since the review
//! of step 28 `cineris project`'s changes, `auth login` and `logout`, and
//! `stats reset`; and the RPC socket, of each peer) look at before they do
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
//!   line that cannot be read refuses, as if it reached one. The line ends
//!   at process 0 or at a process that is its own parent: process 1 is read
//!   as any other, so a harness that is a container's first process is seen
//!   (the review of step 28, finding 7). A parent that `/proc` does not show
//!   (gone, or hidden: another user's under `hidepid`) ends the line
//!   elsewhere; without `hidepid` another user's process (an SSH session's
//!   `sshd`, a `sudo`) is shown, its `stat` is read, and the line goes on
//!   through it. **It does not see** a command that detached itself from
//!   the line (its parent exited, and it was handed to the system's first
//!   process or a subreaper), **nor one in a namespace of processes of its
//!   own,** by any way it has to get one (`unshare -Urpf` where
//!   unprivileged user namespaces are open, `bwrap --unshare-user
//!   --unshare-pid`, a set-user-id `firejail`, the `docker` group, rootless
//!   podman): its parent reads as 0 at the
//!   namespace's edge, whoever reaps it, so a subreaper does not close this.
//!   The maintainers settled that it is stated and not built against (the
//!   review of step 28, finding 8): a rule on a namespace would refuse every
//!   person in WSL 2 (whose processes are all in a pid namespace that is
//!   not the first), in a toolbox or distrobox (a user namespace), or in a
//!   container, whose legitimate first process has a parent of 0 as the
//!   attack's does. Approvals signed with a key that needs a person close
//!   it.
//! - **On Windows, by the harness's own job, and by what started it.** A
//!   process that runs sessions calls [`mark`] at its start: it makes a job
//!   of its own, named [`JOB_PREFIX`] and its process's instance (its id and
//!   when it was made) in the logon session's own namespace of objects, open
//!   by its name to its user and its owner only to ask about it, with no
//!   limits and no breakaway, and joins it. Every process it starts from then
//!   on (a command's tree, a hook, an extension's runtime, an MCP server) is
//!   in that job and cannot leave it. **A process is under a session's
//!   command when a process of its line of parents is a harness,** told as
//!   on Linux, by its program (Cineris: the very file this process runs, or
//!   a program whose file is named `cineris.exe`, `cin.exe` or as this
//!   binary is) and its command line (not a subcommand that runs no session;
//!   one that cannot be read counts), **whatever its job:** a command that
//!   lets its harness's jobs let it leave, through the handles it can copy
//!   from its harness, or that closes the harness's own handle, which leaves
//!   the job with no name, is still refused while its line reaches its
//!   harness, and so is a command of a harness whose mark failed. It is also
//!   under one when it is in the job of that name of a process that is
//!   running and this user may open: first the jobs of the processes of its
//!   line of parents, each by its own instance, opened by name (its harness
//!   is there unless it detached); then the jobs of that prefix in the logon
//!   session's namespace of objects, each by the instance its name gives,
//!   where one call lists the namespace whole (never a listing joined from
//!   several calls, which could skip one), and otherwise the job's name of
//!   every process of the system's list, which one call gives whole. A job
//!   of that name that cannot be opened or asked, for any reason but there
//!   being none, is not known, which refuses, when the process it is named
//!   for is Cineris (the very file this process runs, or a program whose
//!   file is named `cineris.exe`, `cin.exe` or as this binary is) or what
//!   that process is cannot be read: one named for any other program (a job
//!   made to deny its opening, for a process that runs no sessions) is
//!   passed over, and so is one named for another user's process. Being in a
//!   harness's job wins over a job that cannot be told, met first: both
//!   refuse, and only the reason changes. Being in any other job (a
//!   terminal's, an editor's, a CI runner's) is nothing. One job each, and
//!   not one shared:
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
//!   started is not taken for one. A line that goes on through more
//!   interpreters and runtimes than the walk reads (64) is not known, which
//!   refuses, unless the last it read was made before every harness that
//!   runs, or none runs, when what started it counts for nothing, as a
//!   start within the bound; a line of parents longer than it reads is not
//!   known either (the review of step 28, finding 2). **Only a start after a harness began
//!   counts:** the process the service started on the line must have been
//!   made after the earliest harness of the user's that runs now (a process
//!   this user may open whose job of sessions' commands is there), so that a
//!   terminal a logon task opened, or one a service host opened for
//!   `runas`, before any harness is the person's own; with no harness
//!   running, none counts. **It does not see** a service's start
//!   through another program than an interpreter or a runtime, a process
//!   another of the user's processes starts outside the job (the desktop's
//!   shell asked to open a file, a process made with another parent), a
//!   command that both detached itself from its line of parents and left
//!   its harness's job, nor a command whose harness has exited, whose job
//!   no running process names, nor a service's start once every harness of
//!   the user's has exited, nor a program a service started before a
//!   harness began that is asked later to run the subcommand (an
//!   already-running daemon's).
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

/// Whether another process of this user, `pid`, runs under a session's
/// command, read as [`under_a_session`] reads this process: what the RPC
/// socket asks of each peer, so that a command of any harness, and not only
/// of the server's own sessions, is refused as a client (the review of step
/// 28, finding 3). The peer's own process must be there to be read: one that
/// has gone, or cannot be opened, is not known. On Linux a parent of it that
/// `/proc` does not show ends its line elsewhere, the first one included (a
/// client's launcher may exit as it connects), where this process's own
/// first parent not shown is not known.
#[must_use]
pub fn under_a_session_of(pid: u32) -> Under {
    sys::under_a_session_of(pid)
}

/// Under a session's command, a variable of this process that moves where
/// Cineris's configuration and stores are read from, set otherwise than
/// for the session that started it: on Linux by the session's process's own
/// environment, on Windows by what the session told what it starts
/// (`CINERIS_SESSION_STORES`, which [`mark`] sets, and which the permission
/// engine refuses a command line to set). `None` when none is. Asked only
/// where [`under_a_session`] refuses: where the session's own stores cannot
/// be read (on Linux, no harness found on the line of parents, or its
/// environment unreadable; on Windows, nothing told), it names that, which
/// refuses.
#[must_use]
pub fn moved_stores() -> Option<String> {
    sys::moved_stores()
}

/// This process's environment, entry by entry in its order, as Cineris
/// reads its own (`cineris_config::Env::from_process`, from
/// `std::env::vars_os`): an entry whose name or value is not Unicode is
/// left out.
fn entries_of_this_process() -> Vec<(String, String)> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// The value `entries` give `name`, as `cineris_config::Env` reads it: the
/// last entry of that name (its map keeps the last), names compared in
/// ASCII upper case with `fold_case` (Windows, where `ProgramData` and
/// `PROGRAMDATA` are one variable), and an empty value as none.
fn value_in(entries: &[(String, String)], name: &str, fold_case: bool) -> Option<String> {
    let same = |entry: &str| {
        if fold_case {
            entry.eq_ignore_ascii_case(name)
        } else {
            entry == name
        }
    };
    entries
        .iter()
        .rev()
        .find(|(entry, _)| same(entry))
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
}

/// Which of `names`, the variables that move the stores, a nested run's
/// environment `mine` (its entries in order) has otherwise than each of
/// `sessions` (each harness's entries) says, each read as Cineris reads its
/// own ([`value_in`]), so that what is compared is what the run would read
/// (the second review of step 28's fixes: `std::env::var_os` folds case as
/// the system does, and takes another entry of a name given twice). **A
/// variable given more than once refuses first** ([`given_twice`]): the
/// system's lookup and the configuration could take different entries of
/// it, and a person's own environment does not hold a name twice.
fn moved_from(
    names: &[&str],
    fold_case: bool,
    mine: &[(String, String)],
    sessions: &[Vec<(String, String)>],
) -> Option<String> {
    let twice = given_twice(names, fold_case, mine.iter().map(|(name, _)| name.clone()));
    if twice.is_some() {
        return twice;
    }
    for theirs in sessions {
        let moved = names
            .iter()
            .find(|name| value_in(mine, name, fold_case) != value_in(theirs, name, fold_case));
        if let Some(name) = moved {
            return Some((*name).to_owned());
        }
    }
    None
}

/// The first of `names` that `present`, the names of an environment's
/// entries in its order, holds more than once (with `fold_case`, in ASCII
/// upper case, as Windows reads a name), said as a variable that moves the
/// stores (the early review of step 28's fixes).
fn given_twice(
    names: &[&str],
    fold_case: bool,
    present: impl IntoIterator<Item = String>,
) -> Option<String> {
    let fold = |name: &str| {
        if fold_case {
            name.to_ascii_uppercase()
        } else {
            name.to_owned()
        }
    };
    let mut counts = vec![0_usize; names.len()];
    for name in present {
        let name = fold(&name);
        if let Some(at) = names.iter().position(|wanted| fold(wanted) == name) {
            counts[at] += 1;
        }
    }
    names
        .iter()
        .zip(counts)
        .find(|(_, count)| *count > 1)
        .map(|(name, _)| format!("{name}, given more than once in its environment,"))
}

/// The first words of Cineris's subcommands that run no session: a
/// process of Cineris whose first argument is one of these starts no
/// session's command.
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
fn runs_sessions(arguments: &[String]) -> bool {
    arguments
        .get(1)
        .is_none_or(|first| !NO_SESSION.contains(&first.as_str()))
}

/// Whether a process is a session's harness, told by its program and its
/// arguments, as both systems tell it: it is Cineris (`cineris`), and its
/// arguments are not a subcommand that runs no session ([`runs_sessions`]).
/// Arguments that cannot be read (`None`) count as running sessions,
/// failing closed.
fn runs_cineris_sessions(cineris: bool, arguments: Option<&[String]>) -> bool {
    cineris && arguments.is_none_or(runs_sessions)
}

/// The words of a Windows command line, as a Rust program reads its own
/// arguments from it (`std::env::args`, after the rules of Microsoft's C
/// runtime): Cineris reads its subcommand so, and a harness's command line
/// is read here as the harness read it.
///
/// - The program's word ends at the first space or tab outside quotes, and a
///   quote in it only opens or closes quotes.
/// - Every other word ends at a space or tab outside quotes, and runs of
///   them are one gap. A quote opens or closes quotes; inside quotes, two
///   quotes together are one quote character. Backslashes are characters,
///   but before a quote they are halved, and an odd one left over makes the
///   quote a character.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's command lines"))]
fn command_words(line: &str) -> Vec<String> {
    let blank = |c: &char| *c == ' ' || *c == '\t';
    let mut chars = line.chars().peekable();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in chars.by_ref() {
        match c {
            '"' => quoted = !quoted,
            ' ' | '\t' if !quoted => break,
            other => word.push(other),
        }
    }
    words.push(std::mem::take(&mut word));
    while chars.next_if(blank).is_some() {}
    quoted = false;
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' if !quoted => {
                words.push(std::mem::take(&mut word));
                while chars.next_if(blank).is_some() {}
            }
            '\\' => {
                let mut run = 1;
                while chars.next_if_eq(&'\\').is_some() {
                    run += 1;
                }
                if chars.peek() == Some(&'"') {
                    word.extend(std::iter::repeat_n('\\', run / 2));
                    if run % 2 == 1 {
                        chars.next();
                        word.push('"');
                    }
                } else {
                    word.extend(std::iter::repeat_n('\\', run));
                }
            }
            '"' if quoted => match chars.peek() {
                Some('"') => {
                    chars.next();
                    word.push('"');
                }
                Some(_) => quoted = false,
                // A quote left open at the end still makes a word, if empty.
                None => break,
            },
            '"' => quoted = true,
            other => word.push(other),
        }
    }
    if !word.is_empty() || quoted {
        words.push(word);
    }
    words
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
    walk(start, seen, true)
}

/// Where the line from `start` leads, `seen` reading each process; a first
/// process that is not shown is not known when `first_must_show`, and ends
/// the line elsewhere otherwise.
///
/// **The line ends only at process 0,** the parent the kernel gives the
/// first process of a namespace of processes and a process whose parent is
/// outside its namespace, **or at a process that is its own parent:**
/// process 1 is read as any other (the review of step 28, finding 7: a
/// harness that is a container's first process, `docker run image cineris
/// -p …` without `--init`, was never seen, as the walk stopped before
/// reading it). The system's own first process (`systemd`, WSL's `/init`, a
/// container's entry point) runs no sessions, and its parent is 0; one that
/// `/proc` does not show (another user's, under `hidepid`) is not this
/// user's harness, and ends the line elsewhere even first, as it cannot
/// have gone (its namespace ends with it).
#[cfg_attr(windows, allow(dead_code, reason = "Linux's line of parents"))]
fn walk(start: u32, seen: impl Fn(u32) -> Seen, first_must_show: bool) -> Under {
    let mut at = start;
    // A line of parents is short; the bound only stops a loop the table
    // could show while it changes.
    for _ in 0..4096 {
        if at == 0 {
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
            Seen::Unseen if at == start && at != 1 && first_must_show => {
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

/// Every process of the line from `start` that runs sessions, nearest
/// first, `seen` reading each process, as [`walk`] reads the line: to
/// process 0, process 1 included (the review of step 28, finding 7: a
/// harness that is a container's first process). **Every one, and not the
/// nearest alone** (the early review of that part's fixes): a process is a
/// harness by its name and its file's, so a copy of a shell named
/// `cineris`, run with a moved `HOME`, would be the nearest, and the stores
/// compared with its own. A process `/proc` does not show ends the line, as
/// it ends [`walk`]'s; `None` where a read of the line fails, or the line
/// does not end within the bound: what the session's stores are is then
/// not known.
#[cfg_attr(windows, allow(dead_code, reason = "Linux's line of parents"))]
fn harnesses_on_the_line(start: u32, seen: impl Fn(u32) -> Seen) -> Option<Vec<u32>> {
    let mut found = Vec::new();
    let mut at = start;
    for _ in 0..4096 {
        if at == 0 {
            return Some(found);
        }
        match seen(at) {
            Seen::Of { parent, harness } => {
                if harness {
                    found.push(at);
                }
                if parent == at {
                    return Some(found);
                }
                at = parent;
            }
            Seen::Unseen => return Some(found),
            Seen::Failed => return None,
        }
    }
    None
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

    use super::{Seen, Under, line, runs_cineris_sessions, walk};

    pub(super) fn mark() -> io::Result<()> {
        Ok(())
    }

    pub(super) fn under_a_session() -> Under {
        let own = Own::read();
        line(std::os::unix::process::parent_id(), |pid| seen(&own, pid))
    }

    /// Whether another process, `pid`, runs under a session's command, read
    /// as [`under_a_session`] reads this one's: its line of parents, from
    /// its parent up, each told by the same rules. The process itself must
    /// be shown (one that has gone is not known); a parent that is not
    /// shown, the first included, ends the line elsewhere, as a client's
    /// launcher may exit between two reads (the RPC socket's rule since the
    /// second review of step 27's second part).
    pub(super) fn under_a_session_of(pid: u32) -> Under {
        let own = Own::read();
        match seen(&own, pid) {
            Seen::Of { parent, .. } if parent == pid => Under::Nothing,
            Seen::Of { parent, .. } => walk(parent, |pid| seen(&own, pid), false),
            Seen::Unseen => Under::Unknown(format!("process {pid} is not shown")),
            Seen::Failed => Under::Unknown(format!("process {pid} could not be read")),
        }
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

    /// The entries of a process's environment as `/proc/<pid>/environ`
    /// gives them, in order: an entry that is not Unicode, or holds no `=`,
    /// is left out, as Cineris's own reading leaves it out.
    fn environ_entries(environ: &[u8]) -> Vec<(String, String)> {
        environ
            .split(|byte| *byte == 0)
            .filter_map(|entry| {
                let (name, value) = std::str::from_utf8(entry).ok()?.split_once('=')?;
                Some((name.to_owned(), value.to_owned()))
            })
            .collect()
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
        // Asked only under a session's command, or where that could not be
        // told: with no harness found on the line (one of it could not be
        // read), what the session's stores are is not known either, and
        // nothing is compared with them (the review of step 28: this took
        // only the `CINERIS_` variables, so a line made unreadable on
        // purpose let a nested run read stores moved by `XDG_STATE_HOME`).
        let own = Own::read();
        let found = super::harnesses_on_the_line(std::os::unix::process::parent_id(), |pid| {
            seen(&own, pid)
        })
        .unwrap_or_default();
        if found.is_empty() {
            return Some("the session's own environment, whose process was not found,".to_owned());
        }
        let mut sessions = Vec::new();
        for pid in found {
            let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ")) else {
                return Some("the session's own environment, which could not be read,".to_owned());
            };
            sessions.push(environ_entries(&environ));
        }
        super::moved_from(&names, false, &super::entries_of_this_process(), &sessions)
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
        // Its arguments, read only when it is Cineris: arguments that
        // cannot be read are taken as running sessions.
        let arguments = cineris
            .then(|| std::fs::read(format!("/proc/{pid}/cmdline")).ok())
            .flatten()
            .map(|line| {
                line.split(|byte| *byte == 0)
                    .map(|word| String::from_utf8_lossy(word).into_owned())
                    .collect::<Vec<String>>()
            });
        let harness = runs_cineris_sessions(cineris, arguments.as_deref());
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

        /// Makes `script` a `#!/bin/sh` program that says `up` and then
        /// waits for a line, starts it, and gives it back once it has said
        /// so. `spawn` returns while the exec is still under way: glibc's
        /// `posix_spawn` lets the parent go on as soon as the exec has let
        /// go of the memory the child shared with it, and the kernel gives
        /// the process its new name later in the same exec, so `/proc` read
        /// straight after `spawn` can show the name of the thread that
        /// started it (once in 30 runs of `stress`). The script's line
        /// comes once the exec is done.
        fn started_up(script: &std::path::Path) -> std::process::Child {
            use std::io::BufRead as _;
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::write(script, "#!/bin/sh\necho up\nread -r line\n").unwrap();
            std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut child = std::process::Command::new(script)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut said = String::new();
            std::io::BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut said)
                .unwrap();
            assert_eq!(said, "up\n", "what the script said first");
            child
        }

        /// A process of the line whose name is not UTF-8 is not read as no
        /// harness: the line is not known, which refuses, as it was before
        /// `stat` was read in one call (a copy of the harness under such a
        /// name would otherwise pass for another program).
        #[test]
        fn sys_a_name_that_is_not_utf8_leaves_the_line_not_known() {
            use std::os::unix::ffi::OsStrExt as _;
            let scratch = cineris_testkit::scratch::Scratch::outside(
                "cineris-sandbox-session",
                "sys_a_name_that_is_not_utf8_leaves_the_line_not_known",
            );
            let mut child = started_up(&scratch.join(std::ffi::OsStr::from_bytes(b"cin\xff")));
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
            let scratch = cineris_testkit::scratch::Scratch::outside(
                "cineris-sandbox-session",
                "sys_a_program_named_cin_that_is_another_is_not_cineris",
            );
            let mut child = started_up(&scratch.join("cin"));
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

        /// Another process's line is read as this one's (what the RPC
        /// socket asks of a peer, the review of step 28, finding 3): a
        /// script this test starts is under the test's own process, which
        /// runs this very file with a test runner's arguments, a harness as
        /// the check reads one; a process that is not there is not known.
        #[test]
        fn sys_another_processs_line_is_read_as_this_ones() {
            let scratch = cineris_testkit::scratch::Scratch::outside(
                "cineris-sandbox-session",
                "sys_another_processs_line_is_read_as_this_ones",
            );
            let mut child = started_up(&scratch.join("peer"));
            let under = crate::session::under_a_session_of(child.id());
            drop(child.stdin.take());
            child.wait().unwrap();
            let own = format!("process {} runs Cineris's sessions", std::process::id());
            assert_eq!(under, super::Under::Session(own));
            assert!(matches!(
                crate::session::under_a_session_of(u32::MAX),
                super::Under::Unknown(_)
            ));
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

/// Whether a process of the line of parents is a session's harness, told by
/// its program and its arguments ([`runs_cineris_sessions`]).
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's line of parents"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum Runs {
    /// It is not: another program, or Cineris running a subcommand that
    /// runs no session.
    No,
    /// It is.
    Sessions,
    /// What program it runs could not be read: why.
    Unknown(String),
}

/// What opening a process of the line of parents tells of it, for
/// [`parents`].
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's line of parents"))]
#[derive(Debug, Clone)]
enum Above {
    /// It runs, and this user may open it: its parent and when it was made
    /// (its program's name is not kept, and left empty), and whether it is
    /// a harness, which is not asked of this process itself.
    Is(Listed, Runs),
    /// It has gone, or this user may not open it: the line ends.
    Ended,
}

/// A process of the line of parents, whose job is asked first, and which
/// is itself a harness when its program and its arguments say so.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's line of parents"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parent {
    pid: u32,
    made: u64,
    runs: Runs,
}

/// The line of parents above `own`, each described by opening it
/// (`describe(pid, judged)`, which tells whether the process is a harness
/// only when `judged`: not for `own` itself): its parent, its parent's, and
/// so up, through any program, as far as each is a running process this
/// user may open that was made before the one below it (a process made
/// after has the id of a parent that has gone), at most [`LINE_OF_PARENTS`]
/// of them and the one past them, and no further than the first that is a
/// harness by its program
/// and its arguments. The harness of a session's command is on it unless
/// the command detached itself (its parents exited), so its job is asked by
/// its name first, which no listing can miss (the review of the start-up's
/// fix: a listing of the namespace that took several calls could skip it),
/// and it is a harness by what it runs whatever its job is.
///
/// **A line that goes on past the bound is not known** (the review of step
/// 28, beside finding 2): the process past it is kept, and is a harness
/// when it is one and not known otherwise, which refuses where nothing
/// holds the process; before, a command outside its harness's job (it let
/// the jobs it is in let it leave) whose line went up through 64 programs of
/// its own before its harness read as under no session.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's jobs"))]
fn parents(own: u32, describe: impl Fn(u32, bool) -> Above) -> Vec<Parent> {
    let mut line = Vec::new();
    let Above::Is(mut below, _) = describe(own, false) else {
        return line;
    };
    loop {
        let parent = below.parent;
        if parent == 0 || parent == below.pid {
            break;
        }
        let Above::Is(process, runs) = describe(parent, true) else {
            break;
        };
        if process.made > below.made {
            break;
        }
        if line.len() == LINE_OF_PARENTS {
            let past = format!(
                "its line of parents goes on through more than {LINE_OF_PARENTS} processes, past \
                 which it is not read"
            );
            let runs = if runs == Runs::Sessions {
                runs
            } else {
                Runs::Unknown(past)
            };
            line.push(Parent {
                pid: process.pid,
                made: process.made,
                runs,
            });
            break;
        }
        let harness = runs == Runs::Sessions;
        line.push(Parent {
            pid: process.pid,
            made: process.made,
            runs,
        });
        if harness {
            break;
        }
        below = process;
    }
    line
}

/// What one call that lists a directory of objects did with the list it
/// was given.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's jobs"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum Called {
    /// It put every entry there is in the list: the listing is whole.
    Whole,
    /// It put what fitted its room, and more entries follow.
    Part,
    /// It failed, for this reason.
    Failed(String),
}

/// The rooms, in bytes, a listing of the namespace of objects is asked into:
/// the first, and the largest, past which it is not asked again. A logon
/// session's namespace holds a few hundred objects, of about 150 bytes each
/// as listed (359 on a hosted runner fitted the first), and a busy desktop's
/// a few thousand, which the largest holds; one built to be larger than
/// that (a command's own objects, made to slow the check) is not listed
/// further, and the system's list of processes, which one call gives whole,
/// is taken in its place: so a namespace no call can hold costs the check
/// the list (0.6 to 0.8 ms on a hosted runner) and five calls of at most
/// 1 MiB, and no more memory than the list's own room.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's jobs"))]
const ROOMS: (usize, usize) = (64 * 1024, 1024 * 1024);

/// The rooms of this crate's tests' listing in parts
/// ([`switches::LISTING`]`=in-parts`): too small for any logon session's
/// namespace, so that every call leaves entries out.
#[cfg(test)]
const IN_PARTS: (usize, usize) = (64, 512);

/// The switches of this crate's own tests of the Windows check, read only
/// in its own test build (`#[cfg(test)]`): no binary that is built, a
/// build of the whole workspace with the test hooks included, honours
/// them or holds their names (since step 28's third pull request, which
/// had them among `cineris-base`'s test hooks before, where a session's
/// command could have set one on a `cineris` built with the workspace).
#[cfg(test)]
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's check"))]
mod switches {
    /// How the harnesses' jobs are listed: `in-parts`, each call that
    /// lists the namespace given a room too small to hold it, up to a
    /// bound as small ([`super::IN_PARTS`]), so that the system's list of
    /// processes is taken in its place; `empty`, nothing listed and no
    /// list of processes, so that only a harness on the line of parents is
    /// found. Any other value, or none: as the system gives them.
    pub(super) const LISTING: &str = "CINERIS_TEST_SESSION_LISTING";

    /// Set and not empty: this test binary counts as Cineris's when the
    /// process a job that cannot be opened is named for is judged, as the
    /// real binary counts. Without it a test binary is any other program,
    /// so that a job one test makes to deny its opening, named for its own
    /// process, is passed over by every other test's check.
    pub(super) const OWN_BINARY: &str = "CINERIS_TEST_SESSION_OWN_BINARY";

    /// What [`LISTING`] chooses.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Listing {
        /// Each call in a room too small: the listing is never whole.
        InParts,
        /// Nothing listed.
        Empty,
    }

    /// The listing [`LISTING`] chooses, if it chooses one.
    pub(super) fn listing() -> Option<Listing> {
        match std::env::var(LISTING).as_deref() {
            Ok("in-parts") => Some(Listing::InParts),
            Ok("empty") => Some(Listing::Empty),
            _ => None,
        }
    }

    /// Whether [`OWN_BINARY`] is set, and not empty.
    pub(super) fn own_binary() -> bool {
        std::env::var_os(OWN_BINARY).is_some_and(|value| !value.is_empty())
    }
}

/// A listing taken whole by one call. `list(room, from, found)` is one
/// call, in a room of `room` bytes, from the listing's place `from` (0: its
/// first entry; the system's count of the entries it has given, which the
/// call moves on), putting what it lists in `found`. Each call starts at
/// the first entry, with a list of its own, and the room is doubled after a
/// call that left entries out, up to `rooms`'s largest; past it, `None`,
/// and the caller takes a list made whole by one call some other way. A
/// part is never joined to another call's: the system's place is a count,
/// so entries removed between two calls shift the rest down, and the next
/// call skips entries no call gave (the review of the start-up's fix: a
/// command that keeps making and closing objects of its own could hide its
/// harness's job so).
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's jobs"))]
fn in_one_call<T>(
    rooms: (usize, usize),
    mut list: impl FnMut(usize, &mut u32, &mut Vec<T>) -> Called,
) -> Option<Result<Vec<T>, String>> {
    let (mut room, most) = rooms;
    loop {
        let (mut from, mut found) = (0, Vec::new());
        match list(room, &mut from, &mut found) {
            Called::Whole => return Some(Ok(found)),
            Called::Failed(why) => return Some(Err(why)),
            Called::Part if room < most => room = room.saturating_mul(2).min(most),
            Called::Part => return None,
        }
    }
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

/// Where this process stands to the job of sessions' commands of one
/// harness: the process `pid`, made at `made`.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's jobs"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum Job {
    /// There is no such job, or none of a harness of this user's.
    None,
    /// `pid` runs sessions, and this process is not in its job.
    Elsewhere,
    /// This process is in it: how that was told, in words.
    Holds(String),
    /// Whether it is cannot be told: what could not be read.
    Unknown(String),
}

/// The Windows check's verdict, from the jobs it asks in turn: those of the
/// processes of its line of parents (`parents`, each by its process's
/// instance), then those of every running harness `listed` gives, and
/// last, given when the earliest of those harnesses began, whether a
/// service started this process for a command (`service`). A job that
/// holds this process decides at once, and so does a service's start.
///
/// **A process of the line of parents that is a harness by its program and
/// its arguments decides at once too, whatever this process's place in its
/// job,** as Linux's line of parents decides (the maintainers' rule for the
/// third part, with the reviewer: a command can start the subcommand
/// outside its harness's job, by letting the jobs it is in let it leave
/// through the handles it can copy from its harness, and can leave the
/// harness's job with no name by closing the harness's own handle; and a
/// harness whose mark failed has no job). Its job is asked first, so that
/// a command in its harness's job is told so.
///
/// **A job that cannot be told is kept while the rest are asked,** and
/// decides only where nothing holds this process: being in a harness's
/// job wins over another job that cannot be told, met first (step 28's
/// third pull request: a job made to deny its opening, named for another
/// running process, made a check of a process in its own harness's job say
/// "not known"). Both refuse; only the reason given changes. Of several
/// that cannot be told, the first met is said; and so is a process of the
/// line whose program cannot be read.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's jobs"))]
fn judged(
    parents: impl IntoIterator<Item = Parent>,
    listed: impl FnOnce() -> Result<Vec<(u32, u64)>, String>,
    job: impl Fn(u32, u64) -> Job,
    service: impl FnOnce(Option<u64>) -> Under,
) -> Under {
    let mut unknown: Option<String> = None;
    for Parent { pid, made, runs } in parents {
        match job(pid, made) {
            Job::Holds(how) => return Under::Session(how),
            Job::Unknown(what) => {
                unknown.get_or_insert(what);
            }
            Job::None | Job::Elsewhere => {}
        }
        match runs {
            Runs::Sessions => {
                return Under::Session(format!("process {pid} runs Cineris's sessions"));
            }
            Runs::Unknown(what) => {
                unknown.get_or_insert(what);
            }
            Runs::No => {}
        }
    }
    let harnesses = match listed() {
        Ok(harnesses) => harnesses,
        Err(error) => return Under::Unknown(unknown.unwrap_or(error)),
    };
    // When the earliest harness of this user that runs now was made.
    let mut began: Option<u64> = None;
    for (pid, made) in harnesses {
        match job(pid, made) {
            Job::None => {}
            Job::Elsewhere => began = Some(began.map_or(made, |began| began.min(made))),
            Job::Holds(how) => return Under::Session(how),
            Job::Unknown(what) => {
                unknown.get_or_insert(what);
            }
        }
    }
    match service(began) {
        Under::Session(how) => Under::Session(how),
        Under::Unknown(what) => Under::Unknown(unknown.unwrap_or(what)),
        Under::Nothing => unknown.map_or(Under::Nothing, Under::Unknown),
    }
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
    // Still going up through interpreters past the bound: what started the
    // first of them is not known, and so not that it was no service (the
    // review of step 28, finding 2: a start through 64 `cmd /c` said
    // nothing, where Linux's walk refuses past its own bound).
    // What a service started above is older still than `at`: made before
    // every harness that runs, or with none running, it counts for nothing,
    // as a start within the bound would not.
    let older = began.is_none_or(|earliest| made <= earliest);
    if older {
        return Under::Nothing;
    }
    let past = format!(
        "its line of parents goes on through more than {LINE_OF_PARENTS} command interpreters \
         and runtimes, past which what started it is not read"
    );
    Under::Unknown(past)
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "decision 0030: a job, whether a process is in it, a process's start and the \
              service a process runs are only reachable through the Windows API; \
              cineris-sandbox is where such calls live"
)]
mod sys {
    use std::cell::OnceCell;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::sync::OnceLock;

    use cineris_base::FileIdentity;

    use super::{Called, Job, Listed, Opened, Runs, Under, job_name, started_by_a_service};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{NtOpenDirectoryObject, NtQueryDirectoryObject};
    use windows_sys::Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SystemProcessInformation,
    };
    use windows_sys::Wdk::System::Threading::{
        NtQueryInformationProcess, PROCESSINFOCLASS, ProcessBasicInformation,
        ProcessCommandLineInformation, ProcessImageFileName, ProcessImageInformation,
    };
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER,
        FALSE, FILETIME, HANDLE, LocalFree, STATUS_BUFFER_OVERFLOW, STATUS_BUFFER_TOO_SMALL,
        STATUS_INFO_LENGTH_MISMATCH, STATUS_MORE_ENTRIES, STATUS_NO_MORE_ENTRIES, SetLastError,
        UNICODE_STRING,
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
    /// Windows: `ProgramData` places the managed configuration, which the
    /// user's own cannot override (the review of step 28, finding 17).
    const STORES: [&str; 6] = [
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "ProgramData",
        "CINERIS_CONFIG",
        "CINERIS_DATA_DIR",
    ];

    /// What a harness says of its stores to everything it starts: each of
    /// [`STORES`] it has, `NAME=value`, joined by `|`, which no Windows path
    /// holds.
    const SAID: &str = "CINERIS_SESSION_STORES";

    fn stores_now() -> String {
        let entries = super::entries_of_this_process();
        STORES
            .iter()
            .filter_map(|name| {
                super::value_in(&entries, name, true).map(|value| format!("{name}={value}"))
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    pub(super) fn moved_stores() -> Option<String> {
        // This process's own entries, read as `cineris_config::Env` reads
        // them (in ASCII upper case, the last of a name), and not by the
        // system's lookup, whose fold of a name's case is its own (the
        // second review of step 28's fixes).
        let mine = super::entries_of_this_process();
        let Some(said) = super::value_in(&mine, SAID, true) else {
            return Some("its stores, which the session did not say,".to_owned());
        };
        let theirs: Vec<(String, String)> = said
            .split('|')
            .filter_map(|entry| {
                let (name, value) = entry.split_once('=')?;
                Some((name.to_owned(), value.to_owned()))
            })
            .collect();
        super::moved_from(&STORES, true, &mine, &[theirs])
    }

    /// The right to ask a job about itself and its processes (`winnt.h`'s
    /// `JOB_OBJECT_QUERY`), which `windows-sys` keeps under a feature of
    /// its own that this crate does not take for one number.
    pub(super) const JOB_OBJECT_QUERY: u32 = 0x0004;

    /// The right to wait on an object (`winnt.h`'s `SYNCHRONIZE`).
    const SYNCHRONIZE: u32 = 0x0010_0000;

    /// What the job's own user, and its owner, may do with it by its name:
    /// ask it about its processes, and nothing else. The harness's own
    /// handle has every right of a job, given when it is made, but those to
    /// change who may open it and who owns it (`WRITE_DAC`, `WRITE_OWNER`),
    /// which the owner's entry keeps from its maker too (`0x13003F` on a
    /// hosted runner, step 28's third pull request): a command that takes
    /// that handle cannot make the job unopenable. No other process may
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

    /// The system's list of processes, taken once at most in a check, and
    /// only where the cheaper reads cannot answer.
    pub(super) type Taken = std::cell::OnceCell<io::Result<Vec<Listed>>>;

    fn listed_in(list: &Taken) -> Result<&[Listed], String> {
        list.get_or_init(processes)
            .as_ref()
            .map(Vec::as_slice)
            .map_err(|error| format!("the system's processes could not be listed: {error}"))
    }

    /// Where the running harnesses were found.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Source {
        /// In the logon session's namespace of objects, listed whole by one
        /// call.
        Namespace,
        /// In the system's list of processes, which one call gives whole.
        Processes,
    }

    /// Every running harness that could hold this process, each by the
    /// process instance its job is named for: the jobs of sessions'
    /// commands in this logon session's namespace, by their names, where
    /// one call lists the namespace whole ([`named_jobs`]); otherwise, and
    /// where the namespace cannot be listed, every process of the system's
    /// list, each to be tried by its job's name. Never a set joined from
    /// several calls, which could leave a harness out: what it gives must be
    /// every one there is, for a process whose harness is not on its line of
    /// parents, and for when the earliest of them began.
    pub(super) fn harnesses(list: &Taken) -> Result<(Vec<(u32, u64)>, Source), String> {
        // This crate's own tests may choose how the namespace is listed
        // (`super::switches::LISTING`); nothing else can.
        #[cfg(test)]
        let named = match super::switches::listing() {
            Some(super::switches::Listing::InParts) => named_jobs(super::IN_PARTS),
            Some(super::switches::Listing::Empty) => return Ok((Vec::new(), Source::Namespace)),
            None => named_jobs(super::ROOMS),
        };
        #[cfg(not(test))]
        let named = named_jobs(super::ROOMS);
        match named {
            Ok(Some(named)) => Ok((named, Source::Namespace)),
            Ok(None) | Err(_) => {
                let listed = listed_in(list)?;
                let every = listed.iter().map(|process| (process.pid, process.made));
                Ok((every.collect(), Source::Processes))
            }
        }
    }

    pub(super) fn under_a_session() -> Under {
        // SAFETY: plain calls; the pseudo-handle is always valid, with
        // every right.
        let (own, process) = unsafe { (GetCurrentProcessId(), GetCurrentProcess()) };
        judged_for(own, process)
    }

    /// Whether the process `pid` runs under a session's command, read as
    /// this one is ([`judged_for`]), through a handle of it opened only to
    /// ask, which `IsProcessInJob` takes: one that cannot be opened (gone,
    /// another user's) is not known.
    pub(super) fn under_a_session_of(pid: u32) -> Under {
        // SAFETY: plain values.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
        if handle.is_null() {
            return Under::Unknown(format!(
                "process {pid} could not be opened: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let process = unsafe { OwnedHandle::from_raw_handle(handle) };
        judged_for(pid, process.as_raw_handle())
    }

    /// The check of the process `own`, whose handle `process` has at least
    /// the limited right to ask: this process's own, or another's (an RPC
    /// socket's peer).
    fn judged_for(own: u32, process: HANDLE) -> Under {
        let list = Taken::new();
        // This binary, read at the first process of the line of parents
        // whose program is judged, or where a job of a harness's name
        // cannot be opened; its file's identity only where a program's
        // name and size do not tell it apart.
        let binary = OnceCell::new();
        // The harnesses on its line of parents first, each by its job's
        // name and by its program and its arguments: a session's command's
        // own harness is there unless it detached, no listing's race can
        // hide it, and its job does not have to hold the command.
        let parents = super::parents(own, |pid, judged| lineage(pid, judged, &binary));
        super::judged(
            parents,
            // Then every running harness, from a listing made whole by one
            // call: a process whose parents exited stays in its harness's
            // job.
            || harnesses(&list).map(|(harnesses, _)| harnesses),
            |pid, made| in_the_job_of(pid, made, &binary, process),
            // The line of parents, each process on it opened, or the list
            // where one cannot be; the service manager is asked only where
            // the line reaches a service host.
            |began| {
                let line = super::line_of(own, described);
                let read: &[Listed] = match &line {
                    Some(line) => line,
                    None => match listed_in(&list) {
                        Ok(listed) => listed,
                        Err(error) => return Under::Unknown(error),
                    },
                };
                started_by_a_service(read, own, opened, scheduler, began)
            },
        )
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

    /// Whether a counted string the system wrote is `text`, read where it
    /// lies, unit by unit: no text is made of it (an entry's type, which
    /// every object of the namespace has, and only a job's matters).
    fn says(string: &UNICODE_STRING, text: &str) -> bool {
        if string.Buffer.is_null() {
            return text.is_empty();
        }
        // SAFETY: as in `counted`.
        let units =
            unsafe { std::slice::from_raw_parts(string.Buffer, usize::from(string.Length) / 2) };
        units.iter().copied().eq(text.encode_utf16())
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
    /// request). The namespace is taken whole by one call, in the rooms
    /// `rooms` gives ([`super::in_one_call`]): `None` where none of them
    /// holds it. `Err` where the namespace cannot be listed, and in session
    /// 0, which the services share and whose `Local\` is another directory.
    /// The system's list of processes is taken for either.
    pub(super) fn named_jobs(rooms: (usize, usize)) -> io::Result<Option<Vec<(u32, u64)>>> {
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
        let listed = super::in_one_call(rooms, |room, from, found| {
            // Zeroed: the list of entries ends with one of zeroes, which
            // the read below stops at.
            let mut buffer: Vec<u64> = vec![0; room / 8];
            let room = u32::try_from(buffer.len() * 8).unwrap_or(u32::MAX);
            let mut written = 0_u32;
            // SAFETY: the buffer is as large as said and aligned for the
            // entries; `from` and `written` are live `u32`s.
            let status = unsafe {
                NtQueryDirectoryObject(
                    directory.as_raw_handle(),
                    buffer.as_mut_ptr().cast(),
                    room,
                    false,
                    *from == 0,
                    from,
                    &raw mut written,
                )
            };
            match status {
                STATUS_NO_MORE_ENTRIES => Called::Whole,
                // Not even one entry fitted.
                STATUS_BUFFER_TOO_SMALL | STATUS_BUFFER_OVERFLOW => Called::Part,
                failed if failed < 0 => Called::Failed(format!(
                    "the namespace of objects could not be listed (status {failed:#010x})"
                )),
                // What fitted, with more entries after it.
                STATUS_MORE_ENTRIES => {
                    jobs_in(&buffer, found);
                    Called::Part
                }
                _ => {
                    jobs_in(&buffer, found);
                    Called::Whole
                }
            }
        });
        match listed {
            None => Ok(None),
            Some(Ok(found)) => Ok(Some(found)),
            Some(Err(why)) => Err(io::Error::other(why)),
        }
    }

    /// The harnesses' jobs among the entries one call wrote in `buffer`,
    /// put in `found`.
    fn jobs_in(buffer: &[u64], found: &mut Vec<(u32, u64)>) {
        let entries = buffer.as_ptr().cast::<DirectoryEntry>();
        let most = buffer.len() * 8 / std::mem::size_of::<DirectoryEntry>();
        for at in 0..most {
            // SAFETY: within the buffer; the list of entries ends with one
            // of zeroes, before the names they point at.
            let entry = unsafe { ptr::read_unaligned(entries.add(at)) };
            if entry.name.Buffer.is_null() {
                break;
            }
            if says(&entry.kind, JOB_TYPE)
                && let Some(harness) = super::harness_named(&counted(&entry.name))
            {
                found.push(harness);
            }
        }
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
    /// when it was made, as the system's list gives them. The name is read
    /// from the path the system keeps for the program (`\Device\…`), and
    /// not from the path in a drive's letters, which the system makes from
    /// it: on a hosted runner about 1 µs a process against 17 µs (step 28's
    /// third pull request). A process with no program's file (the system's
    /// own) is as one that cannot be opened: only the list names it.
    fn described(pid: u32) -> super::Described {
        use super::Described;
        let (process, mut listed) = match opened_with_parent(pid) {
            Ok(opened) => opened,
            Err(described) => return described,
        };
        let path = match process_text(process.as_raw_handle(), ProcessImageFileName) {
            Ok(path) if !path.is_empty() => path,
            _ => return Described::Closed,
        };
        let name = path.rsplit('\\').next().unwrap_or(&path).to_lowercase();
        listed.name = name;
        Described::Is(listed)
    }

    /// The process `pid` of the line of parents whose jobs are asked first,
    /// opened to ask its parent and when it was made, and, when `judged`
    /// (not this process itself), whether it is a harness by its program
    /// and its arguments ([`by_program`]), read through the same handle; its
    /// program's name is not kept.
    fn lineage(pid: u32, judged: bool, own: &OnceCell<Own>) -> super::Above {
        let Ok((process, listed)) = opened_with_parent(pid) else {
            return super::Above::Ended;
        };
        let runs = if judged {
            by_program(process.as_raw_handle(), pid, own)
        } else {
            Runs::No
        };
        super::Above::Is(listed, runs)
    }

    /// The process `pid`, opened only to ask, with its parent and when it
    /// was made, and its program's name left empty: `Err`, with what that
    /// tells, where it has gone or cannot be opened or asked.
    fn opened_with_parent(pid: u32) -> Result<(OwnedHandle, Listed), super::Described> {
        use super::Described;
        // SAFETY: plain values.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            return Err(
                if error.raw_os_error() == i32::try_from(ERROR_INVALID_PARAMETER).ok() {
                    Described::Gone
                } else {
                    Described::Closed
                },
            );
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let process = unsafe { OwnedHandle::from_raw_handle(handle) };
        let Some(made) = made_at(process.as_raw_handle()) else {
            return Err(Described::Closed);
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
            return Err(Described::Closed);
        }
        let parent = u32::try_from(basic.parent).unwrap_or(u32::MAX);
        Ok((
            process,
            Listed {
                pid,
                parent,
                name: String::new(),
                made,
            },
        ))
    }

    /// Whether the process `handle`, `pid`, of the line of parents is a
    /// session's harness by its program and its arguments, as Linux's line
    /// of parents tells one ([`super::runs_cineris_sessions`]): **its
    /// program is Cineris** ([`named_as_cineris`], or [`same_file`] where
    /// its image's size is this binary's), read from the path the system
    /// keeps for it (`\Device\…`), the cheapest read that gives it (about
    /// 1 µs a process on a hosted runner, where the path in a drive's
    /// letters cost 17 µs); **and its command line,** read only then, is
    /// not a subcommand that runs no session. A command line that cannot be
    /// read counts as running sessions; a program that cannot be read is
    /// not known. A process with no program's file (the system's own) is no
    /// harness.
    fn by_program(handle: HANDLE, pid: u32, own: &OnceCell<Own>) -> Runs {
        let image = match process_text(handle, ProcessImageFileName) {
            Ok(image) if image.is_empty() => return Runs::No,
            Ok(image) => image,
            Err(error) => {
                return Runs::Unknown(format!(
                    "process {pid} of its line of parents: what program it runs could not be \
                     read: {error}"
                ));
            }
        };
        // Opened, where its identity is read, through the root of the
        // system's namespace of objects, where `\Device\…` is.
        let path = PathBuf::from(format!(r"\\?\GLOBALROOT{image}"));
        let own = own.get_or_init(Own::read);
        let cineris = named_as_cineris(own, &path) || same_file(own, &path, || image_size(handle));
        let arguments = cineris
            .then(|| process_text(handle, ProcessCommandLineInformation).ok())
            .flatten()
            .map(|line| super::command_words(&line));
        if super::runs_cineris_sessions(cineris, arguments.as_deref()) {
            Runs::Sessions
        } else {
            Runs::No
        }
    }

    /// The room, in eight-byte units, the counted string a process's class
    /// writes is first asked into: a program's path or a command line of up
    /// to about a thousand characters, on the stack.
    const ROOM: usize = 256;

    /// The counted string the class `class` of `NtQueryInformationProcess`
    /// writes of the process `handle` (its program's path as the system
    /// keeps it, `\Device\…\cin.exe`; its command line, which takes Windows
    /// 8.1 or later): asked in a room on the stack, and once more in one as
    /// large as the call said it needs. Both classes take the limited right
    /// to ask.
    fn process_text(handle: HANDLE, class: PROCESSINFOCLASS) -> io::Result<String> {
        let mut room = [0_u64; ROOM];
        let mut needed = 0_u32;
        // SAFETY: `handle` is open with the limited right to ask; the room
        // is as large as said and aligned for the counted string the call
        // writes at its start; `needed` is a live `u32`.
        let status = unsafe {
            NtQueryInformationProcess(
                handle,
                class,
                room.as_mut_ptr().cast(),
                u32::try_from(std::mem::size_of_val(&room)).unwrap_or(0),
                &raw mut needed,
            )
        };
        if status >= 0 {
            return text_in(&room);
        }
        let short = [
            STATUS_INFO_LENGTH_MISMATCH,
            STATUS_BUFFER_OVERFLOW,
            STATUS_BUFFER_TOO_SMALL,
        ];
        if !short.contains(&status) || needed == 0 {
            return Err(io::Error::other(format!("status {status:#010x}")));
        }
        let mut larger = vec![0_u64; usize::try_from(needed).unwrap_or(usize::MAX).div_ceil(8)];
        // SAFETY: as above, in a room as large as the call said it needs.
        let status = unsafe {
            NtQueryInformationProcess(
                handle,
                class,
                larger.as_mut_ptr().cast(),
                u32::try_from(larger.len() * 8).unwrap_or(u32::MAX),
                &raw mut needed,
            )
        };
        if status < 0 {
            return Err(io::Error::other(format!("status {status:#010x}")));
        }
        text_in(&larger)
    }

    /// The counted string at the start of `room`, whose text the system
    /// wrote after it in the same room: read only from within it.
    fn text_in(room: &[u64]) -> io::Result<String> {
        // SAFETY: the room is at least as large as the structure, aligned
        // for it, and the call wrote it.
        let string: UNICODE_STRING = unsafe { ptr::read(room.as_ptr().cast()) };
        if string.Buffer.is_null() || string.Length == 0 {
            return Ok(String::new());
        }
        let bytes = room.len() * 8;
        let units = usize::from(string.Length) / 2;
        let within = (string.Buffer as usize)
            .checked_sub(room.as_ptr() as usize)
            .and_then(|offset| offset.checked_add(units * 2))
            .is_some_and(|end| end <= bytes);
        if !within {
            return Err(io::Error::other("the text was not where it was said to be"));
        }
        // SAFETY: the text lies within the room (checked above), which the
        // call wrote.
        let text = unsafe { std::slice::from_raw_parts(string.Buffer, units) };
        Ok(String::from_utf16_lossy(text))
    }

    /// `SECTION_IMAGE_INFORMATION`, which `windows-sys` does not declare:
    /// what the system keeps of a process's program as it mapped it. Only
    /// its file's size is read.
    #[repr(C)]
    struct ImageInformation {
        transfer_address: *mut core::ffi::c_void,
        zero_bits: u32,
        maximum_stack_size: usize,
        committed_stack_size: usize,
        subsystem_type: u32,
        subsystem_version: u32,
        operating_system_version: u32,
        image_characteristics: u16,
        dll_characteristics: u16,
        machine: u16,
        image_contains_code: u8,
        image_flags: u8,
        loader_flags: u32,
        image_file_size: u32,
        check_sum: u32,
    }

    /// The size of the file the process `handle` runs, as the system mapped
    /// it: the same for every process of one file, which cannot be written
    /// while it runs. `None` where it cannot be asked.
    pub(super) fn image_size(handle: HANDLE) -> Option<u32> {
        // SAFETY: the structure is plain data, for which all zeroes is valid.
        let mut information: ImageInformation = unsafe { std::mem::zeroed() };
        // SAFETY: `handle` is open with the limited right to ask, which
        // this class takes, and the structure is as large as said.
        let status = unsafe {
            NtQueryInformationProcess(
                handle,
                ProcessImageInformation,
                (&raw mut information).cast(),
                u32::try_from(std::mem::size_of::<ImageInformation>()).unwrap_or(0),
                ptr::null_mut(),
            )
        };
        (status >= 0).then_some(information.image_file_size)
    }

    /// The path of the program the process `handle` names runs, as Windows
    /// writes it (`C:\…\cin.exe`).
    fn image_of(handle: HANDLE) -> io::Result<String> {
        let mut path = [0_u16; 1024];
        let mut length = u32::try_from(path.len()).unwrap_or(0);
        // SAFETY: `handle` is open with the limited right to ask, and the
        // buffer holds `length` characters.
        let asked = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                path.as_mut_ptr(),
                &raw mut length,
            )
        };
        if asked == 0 {
            return Err(io::Error::last_os_error());
        }
        path.get(..usize::try_from(length).unwrap_or(usize::MAX))
            .map(String::from_utf16_lossy)
            .ok_or_else(|| io::Error::other("the path was longer than its room"))
    }

    /// This binary, as the program of another process is compared with
    /// ([`cineris`]): its file's name, in lower case; its file, by its
    /// volume and its index on it, read the first time it is needed; and,
    /// for the binary this process runs, its image's size.
    #[derive(Debug, Default)]
    pub(super) struct Own {
        path: Option<PathBuf>,
        file: OnceCell<Option<FileIdentity>>,
        name: Option<String>,
        size: Option<u32>,
    }

    impl Own {
        /// The binary at `path`, whose image's size is not known.
        pub(super) fn of(path: &Path) -> Self {
            Self {
                path: Some(path.to_path_buf()),
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_lowercase()),
                ..Self::default()
            }
        }

        /// This process's own binary. In this crate's own tests, none,
        /// unless the test switch says the test binary counts
        /// (`super::switches::OWN_BINARY`): a test binary is no harness.
        pub(super) fn read() -> Self {
            #[cfg(test)]
            if !super::switches::own_binary() {
                return Self::default();
            }
            std::env::current_exe().map_or_else(
                |_| Self::default(),
                |exe| Self {
                    // SAFETY: the pseudo-handle is always valid, with every
                    // right.
                    size: image_size(unsafe { GetCurrentProcess() }),
                    ..Self::of(&exe)
                },
            )
        }

        /// Its file's identity, read once.
        fn file(&self) -> Option<FileIdentity> {
            *self.file.get_or_init(|| {
                self.path
                    .as_deref()
                    .and_then(|path| FileIdentity::at(path).ok())
            })
        }
    }

    /// Whether a program whose file is at `image` is Cineris, as Linux
    /// tells it by `/proc` (where a process's name is its own and its
    /// file's another; on Windows a process's name is its file's): **the
    /// very file this binary is** (`own`: the same volume and index, so a
    /// hard link of it under any name too; [`same_file`]), **or a file
    /// named as Cineris's are,** `cineris.exe`, `cin.exe` or as this binary
    /// is, in any case, as NTFS folds names ([`named_as_cineris`]). A
    /// renamed copy is another file, under another name, and is not; a copy
    /// under one of those names, wherever it is installed, is; and so is any
    /// other program given one of them, taken for Cineris, which fails
    /// closed (a job of its name that cannot be opened refuses, and a
    /// process of the line of parents is a harness). A file whose identity
    /// cannot be read is told by its name alone.
    pub(super) fn cineris(own: &Own, image: &Path) -> bool {
        named_as_cineris(own, image) || same_file(own, image, || None)
    }

    /// Whether the file at `image` is named as Cineris's are: `cineris.exe`,
    /// `cin.exe` or as this binary is, in any case.
    pub(super) fn named_as_cineris(own: &Own, image: &Path) -> bool {
        image.file_name().is_some_and(|name| {
            let name = name.to_string_lossy().to_lowercase();
            name == "cineris.exe" || name == "cin.exe" || own.name.as_deref() == Some(name.as_str())
        })
    }

    /// Whether the file at `image` is the very file this binary is: the
    /// same volume, and the same index on it. **A program whose image is of
    /// another size than this binary's is another file,** and its file is
    /// not opened (`size`, the program's image's size as its process's
    /// mapping says, asked only where this binary's is known): opening each
    /// program of a line of parents of 13 to read its identity cost a
    /// process's first check about 0.25 ms on a hosted runner, and asking
    /// each one's size about 9 µs (step 28's third pull request).
    pub(super) fn same_file(own: &Own, image: &Path, size: impl FnOnce() -> Option<u32>) -> bool {
        if own.path.is_none() {
            return false;
        }
        if let Some(own) = own.size
            && size().is_some_and(|theirs| theirs != own)
        {
            return false;
        }
        own.file()
            .is_some_and(|own| FileIdentity::at(image).ok() == Some(own))
    }

    /// What the process a job of a harness's name that cannot be opened is
    /// named for is.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) enum NamedFor {
        /// Cineris ([`cineris`]).
        Cineris,
        /// No harness of this user's: gone, another process with its id,
        /// another user's or the system's, or another program.
        Other,
        /// What it is could not be read: as Cineris, failing closed.
        Unread(String),
    }

    /// What the process `pid`, made at `made`, is, a job of whose name
    /// cannot be opened: opened to ask when it was made and the path of
    /// its program, which is judged against this binary (`own`, read the
    /// first time it is needed). **A process this user may not open is
    /// another user's, or the system's,** as the check holds everywhere
    /// (its commands run as its user), and is not Cineris of this one's;
    /// one that is gone, or that has another instance, is not the one the
    /// job is named for.
    pub(super) fn named_for(pid: u32, made: u64, own: &OnceCell<Own>) -> NamedFor {
        // SAFETY: plain values.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            let other = [ERROR_INVALID_PARAMETER, ERROR_ACCESS_DENIED]
                .into_iter()
                .any(|code| error.raw_os_error() == i32::try_from(code).ok());
            return if other {
                NamedFor::Other
            } else {
                NamedFor::Unread(format!("process {pid} could not be opened: {error}"))
            };
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let process = unsafe { OwnedHandle::from_raw_handle(handle) };
        match made_at(process.as_raw_handle()) {
            Some(then) if then == made => {}
            Some(_) => return NamedFor::Other,
            None => {
                return NamedFor::Unread(format!(
                    "when process {pid} was made could not be asked: {}",
                    io::Error::last_os_error()
                ));
            }
        }
        match image_of(process.as_raw_handle()) {
            Ok(image) if cineris(own.get_or_init(Own::read), Path::new(&image)) => {
                NamedFor::Cineris
            }
            Ok(_) => NamedFor::Other,
            Err(error) => NamedFor::Unread(format!(
                "process {pid}'s program could not be read: {error}"
            )),
        }
    }

    /// Whether the process `process` (this one's pseudo-handle, or a
    /// handle with at least the limited right to ask) is in the job of
    /// sessions' commands of the harness `pid`, made at `made`.
    ///
    /// **A job of that name that cannot be opened,** for any reason but
    /// there being none, is not known, which refuses (the full review of
    /// the third part: a job made to deny its opening must not read as no
    /// job), **when the process it is named for is Cineris,** or what that
    /// process is cannot be read ([`named_for`], with `own`, this binary,
    /// read the first time it is needed). A job made so for any other
    /// program is no harness's, and is passed over: a harness's own job is
    /// open to its user and its owner to ask, and cannot be made otherwise
    /// after its mark (not even through the harness's own handle of it,
    /// which has no right to change who may open it, [`BY_NAME`]), so one
    /// that is not was made by another process (step 28's third pull
    /// request: one made for a test's own process made other tests' checks
    /// refuse, and such a job made for any running process would make the
    /// user's own approvals refuse). One made for a process of Cineris, a
    /// harness whose name was taken before its mark among them, is still not
    /// known.
    ///
    /// **Only a process this user may open is a harness of this one's**
    /// (another user's, the system's, are not: its commands run as its
    /// user). The job's name is tried first, with when the process was made
    /// as the list of processes gives it, and the process is opened only
    /// where there is a job of that name, or one that could not be told
    /// from none: most processes run no sessions, and opening each cost a
    /// headless run's start 0.35 ms on a hosted runner.
    fn in_the_job_of(pid: u32, made: u64, own: &OnceCell<Own>, process: HANDLE) -> Job {
        // The process, this user's and the same instance (a process that
        // has gone, whose id is another's now, is not).
        let harness = || opened(pid) == Opened::Made(made);
        let name = wide(&job_name(pid, made));
        // SAFETY: plain values, and a name that ends in a nul and lives
        // through the call.
        let handle = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, name.as_ptr()) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == i32::try_from(ERROR_FILE_NOT_FOUND).ok() {
                return Job::None;
            }
            let cannot = format!(
                "the job of process {pid}'s sessions' commands could not be opened: {error}"
            );
            return match named_for(pid, made, own) {
                NamedFor::Cineris => Job::Unknown(cannot),
                NamedFor::Other => Job::None,
                NamedFor::Unread(what) => Job::Unknown(format!(
                    "{cannot}, and what that process is was not told: {what}"
                )),
            };
        }
        // SAFETY: `handle` was just opened and nothing else owns it.
        let job = unsafe { OwnedHandle::from_raw_handle(handle) };
        if !harness() {
            return Job::None;
        }
        let mut inside = FALSE;
        // SAFETY: `process` has at least the limited right to ask, `job` is
        // open with `JOB_OBJECT_QUERY`, both live through the call, and
        // `inside` is a live `BOOL`.
        let asked = unsafe { IsProcessInJob(process, job.as_raw_handle(), &raw mut inside) };
        if asked == 0 {
            return Job::Unknown(format!(
                "whether it is in the job of process {pid}'s sessions' commands could not be \
                 asked: {}",
                io::Error::last_os_error()
            ));
        }
        if inside == 0 {
            return Job::Elsewhere;
        }
        Job::Holds(format!(
            "it is in the job of process {pid}'s sessions' commands"
        ))
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

    /// Process 1 is read as any other (the review of step 28, finding 7): a
    /// harness that is the first process of its namespace (`docker run image
    /// cineris -p …` without `--init`) is seen, near or far; the system's
    /// own first process, whose parent is 0, ends the line elsewhere; and one
    /// `/proc` does not show (another user's, under `hidepid`) ends it
    /// elsewhere even as the first parent, as it cannot have gone.
    #[test]
    fn a_harness_that_is_process_1_is_seen_and_the_line_ends_at_process_0() {
        let container = [(7, of(5, false)), (5, of(1, false)), (1, of(0, true))];
        let read = table(&container);
        assert!(
            matches!(line(7, &read), Under::Session(how) if how.contains("process 1 ")),
            "{:?}",
            line(7, &read)
        );
        assert!(matches!(line(1, &read), Under::Session(_)));
        let system = [(7, of(5, false)), (5, of(1, false)), (1, of(0, false))];
        let read = table(&system);
        assert_eq!(line(7, &read), Under::Nothing);
        assert_eq!(line(1, &read), Under::Nothing);
        // A parent that reads 0: outside the namespace (the stated gap of a
        // namespace of the command's own, which nothing here closes).
        assert_eq!(line(0, &read), Under::Nothing);
        let hidden = [(7, of(1, false))];
        let read = table(&hidden);
        assert_eq!(line(1, &read), Under::Nothing, "pid 1 hidden, first");
        assert_eq!(line(7, &read), Under::Nothing, "pid 1 hidden, further up");
    }

    /// A variable that moves the stores, given twice in an environment, is
    /// said, as Windows reads a name (in any case) or as Linux does (by its
    /// bytes): the system's lookup and Cineris's configuration could take
    /// different entries of it (the early review of step 28's fixes).
    #[test]
    fn a_store_variable_given_twice_is_said() {
        let present = |names: &[&str]| -> Vec<String> {
            names.iter().map(|name| (*name).to_owned()).collect()
        };
        let stores = ["HOME", "AppData"];
        assert_eq!(
            given_twice(&stores, false, present(&["HOME", "PATH"])),
            None
        );
        assert!(
            given_twice(&stores, false, present(&["HOME", "PATH", "HOME"]))
                .is_some_and(|said| said.starts_with("HOME, given more than once")),
        );
        assert_eq!(
            given_twice(&stores, false, present(&["AppData", "APPDATA"])),
            None,
            "two names to Linux"
        );
        assert!(
            given_twice(&stores, true, present(&["AppData", "APPDATA"]))
                .is_some_and(|said| said.starts_with("AppData, given more than once")),
            "one name to Windows"
        );
    }

    /// A nested run's stores are compared as Cineris reads them: the last
    /// entry of a name, in ASCII upper case on Windows, an empty value as
    /// none; a variable given twice refuses first; and every session's
    /// harness is compared with, not the first alone (the second review of
    /// step 28's fixes).
    #[test]
    fn a_nested_runs_stores_are_compared_as_cineris_reads_them() {
        let entries = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect()
        };
        let last = entries(&[("HOME", "/a"), ("PATH", "/bin"), ("HOME", "/b")]);
        assert_eq!(
            value_in(&last, "HOME", false).as_deref(),
            Some("/b"),
            "the last entry"
        );
        let cased = entries(&[("AppData", "C:\\a"), ("APPDATA", "C:\\b")]);
        assert_eq!(value_in(&cased, "AppData", true).as_deref(), Some("C:\\b"));
        assert_eq!(value_in(&cased, "AppData", false).as_deref(), Some("C:\\a"));
        assert_eq!(
            value_in(&entries(&[("HOME", "")]), "HOME", false),
            None,
            "empty"
        );
        let names = ["HOME", "XDG_CACHE_HOME"];
        let session = entries(&[("HOME", "/srv/me"), ("XDG_CACHE_HOME", "/srv/me/.s")]);
        assert_eq!(
            moved_from(&names, false, &session, std::slice::from_ref(&session)),
            None
        );
        // The session's own environment read for its last entry too.
        let session_twice = entries(&[
            ("HOME", "/tmp/x"),
            ("HOME", "/srv/me"),
            ("XDG_CACHE_HOME", "/srv/me/.s"),
        ]);
        assert_eq!(moved_from(&names, false, &session, &[session_twice]), None);
        let moved = entries(&[("HOME", "/srv/me"), ("XDG_CACHE_HOME", "/tmp/evil")]);
        assert_eq!(
            moved_from(&names, false, &moved, std::slice::from_ref(&session)).as_deref(),
            Some("XDG_CACHE_HOME")
        );
        // A variable given twice, its last entry the session's.
        let twice = entries(&[
            ("HOME", "/tmp/evil"),
            ("HOME", "/srv/me"),
            ("XDG_CACHE_HOME", "/srv/me/.s"),
        ]);
        assert!(
            moved_from(&names, false, &twice, std::slice::from_ref(&session))
                .is_some_and(|said| said.starts_with("HOME, given more than once")),
        );
        // Every session's harness: the nearest says the moved value.
        let near = entries(&[("HOME", "/srv/me"), ("XDG_CACHE_HOME", "/tmp/evil")]);
        assert_eq!(
            moved_from(&names, false, &moved, &[near, session]).as_deref(),
            Some("XDG_CACHE_HOME")
        );
    }

    /// Every harness of a line is found, nearest first, to process 0,
    /// process 1 included (a harness that is a container's first process,
    /// the review of step 28, finding 7); a process `/proc` does not show
    /// ends the line, and a read that fails leaves it not known.
    #[test]
    fn the_harnesses_of_a_line_are_every_one_to_process_0() {
        let processes = [
            (7, of(5, true)),
            (5, of(1, false)),
            (1, of(0, true)),
            (20, of(19, true)),
            (30, of(29, true)),
            (29, Seen::Failed),
        ];
        let read = table(&processes);
        assert_eq!(harnesses_on_the_line(7, &read), Some(vec![7, 1]));
        assert_eq!(harnesses_on_the_line(5, &read), Some(vec![1]));
        assert_eq!(harnesses_on_the_line(0, &read), Some(vec![]));
        assert_eq!(
            harnesses_on_the_line(20, &read),
            Some(vec![20]),
            "19 not shown"
        );
        assert_eq!(harnesses_on_the_line(30, &read), None);
    }

    /// Another process's line, as the RPC socket reads a peer's: the same
    /// rules, but a first parent `/proc` does not show ends it elsewhere (a
    /// client's launcher may exit as it connects), where this process's
    /// own first parent not shown is not known.
    #[test]
    fn a_peers_line_ends_elsewhere_at_a_first_parent_not_shown() {
        let processes = [(10, of(9, false)), (9, of(1, true)), (20, of(19, false))];
        let read = table(&processes);
        assert!(matches!(walk(9, &read, false), Under::Session(_)));
        assert_eq!(walk(19, &read, false), Under::Nothing);
        assert!(matches!(walk(19, &read, true), Under::Unknown(_)));
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

    /// A service's start through more command interpreters than the walk
    /// reads is not known, which refuses (the review of step 28, finding 2:
    /// `cmd /c` nested 64 times between WMI's host and the subcommand said
    /// nothing), read on the line [`line_of`] describes as on the system's
    /// list; up to the bound, the host is still reached.
    #[test]
    fn a_services_start_through_more_interpreters_than_the_bound_is_not_known() {
        // `own` (1000), `count` interpreters above it, the nearest first,
        // then WMI's host, made in that order downwards.
        let chain = |count: u32| {
            let mut processes = vec![(1000, 2000, "cin.exe".to_owned())];
            for at in 0..count {
                let parent = if at + 1 == count { 9000 } else { 2001 + at };
                processes.push((2000 + at, parent, "cmd.exe".to_owned()));
            }
            processes.push((9000, 4, "wmiprvse.exe".to_owned()));
            processes
                .into_iter()
                .map(|(pid, parent, name)| Listed {
                    pid,
                    parent,
                    name,
                    made: u64::from(10_000 - pid),
                })
                .collect::<Vec<_>>()
        };
        let opened = |pid: u32| match pid {
            4 | 9000 => Opened::Closed,
            _ => Opened::Made(u64::from(10_000 - pid)),
        };
        // Past the bound, a start older than every harness that runs, or
        // with none running, counts for nothing, as one within it.
        let processes = chain(70);
        assert_eq!(
            started_by_a_service(&processes, 1000, opened, || Some(0), None),
            Under::Nothing
        );
        assert_eq!(
            started_by_a_service(&processes, 1000, opened, || Some(0), Some(u64::MAX)),
            Under::Nothing
        );
        let bound = u32::try_from(LINE_OF_PARENTS).unwrap();
        for (count, seen) in [
            (3, true),
            (bound - 1, true),
            (bound, false),
            (bound + 10, false),
        ] {
            let processes = chain(count);
            let listed = started_by_a_service(&processes, 1000, opened, || Some(0), Some(0));
            let described = |pid: u32| {
                processes
                    .iter()
                    .find(|process| process.pid == pid)
                    .cloned()
                    .map_or(Described::Gone, |process| {
                        if pid == 9000 {
                            Described::Closed
                        } else {
                            Described::Is(process)
                        }
                    })
            };
            // Described by opening each, the line stops at the host, which
            // cannot be opened: the list is then taken. A line that never
            // reaches it within the bound is read to the bound alone.
            let line = line_of(1000, described);
            let on_the_line = line
                .as_deref()
                .map(|line| started_by_a_service(line, 1000, opened, || Some(0), Some(0)));
            if seen {
                assert!(matches!(listed, Under::Session(_)), "{count}: {listed:?}");
                assert!(line.is_none(), "{count}: the host is met, closed");
            } else {
                assert!(
                    matches!(&listed, Under::Unknown(why) if why.contains("more than 64")),
                    "{count}: {listed:?}"
                );
                assert_eq!(
                    line.as_ref().map(Vec::len),
                    Some(LINE_OF_PARENTS + 1),
                    "{count}"
                );
                assert!(
                    matches!(&on_the_line, Some(Under::Unknown(why)) if why.contains("more than 64")),
                    "{count}: {on_the_line:?}"
                );
            }
        }
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

    /// A directory of objects listed as the system lists one: a call gives
    /// the entries from the count it is given (or from the first, at 0) as
    /// far as `room / 16` of them fit, and moves the count on; `between`
    /// runs before each call after the first, as a command's own objects
    /// come and go. The rooms each call was given are kept in `asked`, and a
    /// listing asked without end fails at its 65th call.
    fn directory<'a>(
        mut entries: Vec<u32>,
        asked: &'a std::cell::RefCell<Vec<usize>>,
        between: impl Fn(usize, &mut Vec<u32>) + 'a,
    ) -> impl FnMut(usize, &mut u32, &mut Vec<u32>) -> Called + 'a {
        move |room: usize, from: &mut u32, found: &mut Vec<u32>| {
            asked.borrow_mut().push(room);
            let calls = asked.borrow().len();
            assert!(calls <= 64, "a listing asked without end: {calls} calls");
            if calls > 1 {
                between(calls, &mut entries);
            }
            let start = usize::try_from(*from).unwrap().min(entries.len());
            let rest = &entries[start..];
            let given = &rest[..(room / 16).min(rest.len())];
            found.extend_from_slice(given);
            *from = u32::try_from(start + given.len()).unwrap();
            if given.len() < rest.len() {
                Called::Part
            } else {
                Called::Whole
            }
        }
    }

    /// A listing that needs more than one call is never joined from its
    /// calls: each call starts at the first entry, in a room twice the last,
    /// until one holds it whole. Joined, it would miss the harness's job,
    /// `7`, which the first call left out and the next skipped once a
    /// command's object before it was removed between the two (the review of
    /// the start-up's fix: the system goes on from a count of the entries it
    /// gave).
    #[test]
    fn a_listing_that_needs_more_than_one_call_is_taken_again_whole_and_never_joined() {
        let asked = std::cell::RefCell::new(Vec::new());
        let list = directory(vec![1, 2, 7, 3, 4, 5], &asked, |calls, entries| {
            if calls == 2 {
                entries.remove(0);
            }
        });
        let listed = in_one_call((32, 128), list).unwrap().unwrap();
        assert_eq!(listed, [2, 7, 3, 4, 5]);
        assert_eq!(*asked.borrow(), [32, 64, 128]);
        // One call that holds it is the whole listing.
        let asked = std::cell::RefCell::new(Vec::new());
        let list = directory(vec![1, 2, 7], &asked, |_, _| {});
        assert_eq!(in_one_call((64, 128), list), Some(Ok(vec![1, 2, 7])));
        assert_eq!(*asked.borrow(), [64]);
        // A call that fails is the listing's failure.
        let failed = in_one_call::<u32>((64, 128), |_, _, _| Called::Failed("no".to_owned()));
        assert_eq!(failed, Some(Err("no".to_owned())));
    }

    /// A namespace that no room up to the largest holds, as one a command
    /// keeps growing would be, is given up after the largest, 1 MiB, five
    /// calls in all: the caller then takes the system's list of processes,
    /// which one call gives whole.
    #[test]
    fn a_listing_no_room_holds_is_given_up_after_the_largest() {
        let asked = std::cell::RefCell::new(Vec::new());
        let list = directory((0..1_000_000).collect(), &asked, |_, _| {});
        assert_eq!(in_one_call(ROOMS, list), None);
        assert_eq!(
            *asked.borrow(),
            [64 * 1024, 128 * 1024, 256 * 1024, 512 * 1024, 1024 * 1024]
        );
        // The tests' rooms, which no logon session's namespace fits.
        let asked = std::cell::RefCell::new(Vec::new());
        let list = directory((0..100).collect(), &asked, |_, _| {});
        assert_eq!(in_one_call(IN_PARTS, list), None);
        assert_eq!(*asked.borrow(), [64, 128, 256, 512]);
    }

    /// The line of parents above a process is read through any program (a
    /// harness's command may run under `make` or a test runner, not only a
    /// shell) up to the first process that has gone, that cannot be opened,
    /// that was made after the one below it (a gone parent's id taken by a
    /// later process), or that is its own parent or the system's idle
    /// process, and no further than [`LINE_OF_PARENTS`].
    #[test]
    fn the_line_of_parents_is_read_through_any_program_to_its_end() {
        let processes = listed(&[
            (100, 90, "cin.exe"),
            (90, 80, "make.exe"),
            (80, 70, "cin.exe"),
            (70, 60, "windowsterminal.exe"),
            (60, 4, "explorer.exe"),
            (200, 190, "cin.exe"),
            (190, 9999, "cmd.exe"),
            (300, 290, "cin.exe"),
            (290, 280, "cmd.exe"),
            (400, 390, "cin.exe"),
            (390, 390, "cmd.exe"),
            (500, 0, "cin.exe"),
        ]);
        let describe = |pid: u32, _judged: bool| match pid {
            4 | 280 => Above::Ended,
            // Made after the process below it.
            290 => Above::Is(
                Listed {
                    pid: 290,
                    parent: 280,
                    name: "cmd.exe".to_owned(),
                    made: 1_000,
                },
                Runs::No,
            ),
            _ => processes
                .iter()
                .find(|process| process.pid == pid)
                .cloned()
                .map_or(Above::Ended, |process| Above::Is(process, Runs::No)),
        };
        let pids = |own| {
            parents(own, describe)
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>()
        };
        assert_eq!(pids(100), [90, 80, 70, 60], "through make, to the closed");
        assert_eq!(pids(200), [190], "to the gone");
        assert_eq!(pids(300), [] as [u32; 0], "a parent made later");
        assert_eq!(pids(400), [390], "its own parent");
        assert_eq!(pids(500), [] as [u32; 0], "the idle process");
        assert_eq!(pids(9999), [] as [u32; 0], "itself gone");
        let endless = |pid: u32, _judged: bool| {
            Above::Is(
                Listed {
                    pid,
                    parent: pid + 1,
                    name: "cmd.exe".to_owned(),
                    made: u64::from(u32::MAX - pid),
                },
                Runs::No,
            )
        };
        // A line that does not end is read to the bound and one past it,
        // which is not known (the review of step 28, beside finding 2).
        let read = parents(1, endless);
        assert_eq!(read.len(), LINE_OF_PARENTS + 1);
        assert!(
            read[..LINE_OF_PARENTS]
                .iter()
                .all(|process| process.runs == Runs::No),
            "{read:?}"
        );
        assert!(
            matches!(&read[LINE_OF_PARENTS].runs, Runs::Unknown(why) if why.contains("more than 64")),
            "{read:?}"
        );
        // A harness past the bound is still a harness.
        let harness_past = |pid: u32, judged: bool| match endless(pid, judged) {
            Above::Is(process, _) if pid == 1 + 65 => Above::Is(process, Runs::Sessions),
            other => other,
        };
        let read = parents(1, harness_past);
        assert_eq!(read[LINE_OF_PARENTS].runs, Runs::Sessions);
        // Judged with nothing that holds the process, the line past its
        // bound refuses.
        let unknown = judged(
            parents(1, endless),
            || Ok(Vec::new()),
            |_, _| Job::None,
            |_| Under::Nothing,
        );
        assert!(
            matches!(&unknown, Under::Unknown(why) if why.contains("more than 64")),
            "{unknown:?}"
        );
    }

    /// The line of parents whose jobs are asked first asks each process
    /// above this one, and never this one, whether it is a harness by its
    /// program and its arguments, keeps what each says, and goes no further
    /// than the first that is: a process of Cineris that runs no session (an
    /// editor's `cineris config edit`) and one whose program could not be
    /// read do not end it.
    #[test]
    fn the_line_of_parents_ends_at_the_first_harness_by_its_program() {
        let asked = std::cell::RefCell::new(Vec::new());
        let describe = |pid: u32, judged: bool| {
            asked.borrow_mut().push((pid, judged));
            let runs = match pid {
                90 => Runs::No,
                80 => Runs::Unknown("unread".to_owned()),
                70 | 60 => Runs::Sessions,
                _ => Runs::No,
            };
            Above::Is(
                Listed {
                    pid,
                    parent: pid - 10,
                    name: String::new(),
                    made: u64::from(pid),
                },
                runs,
            )
        };
        let line = parents(100, describe);
        assert_eq!(
            line,
            [
                Parent {
                    pid: 90,
                    made: 90,
                    runs: Runs::No
                },
                Parent {
                    pid: 80,
                    made: 80,
                    runs: Runs::Unknown("unread".to_owned())
                },
                Parent {
                    pid: 70,
                    made: 70,
                    runs: Runs::Sessions
                },
            ]
        );
        assert_eq!(
            *asked.borrow(),
            [(100, false), (90, true), (80, true), (70, true)],
            "this process is not judged, and nothing past the harness is read"
        );
    }

    /// The words of a Windows command line are read as a Rust program reads
    /// its own (Cineris reads its subcommand so): the program's word ends
    /// at a blank outside quotes, with no escapes; then quotes group,
    /// backslashes before a quote are halved, an odd one makes it a
    /// character, and two quotes inside quotes are one.
    #[test]
    fn the_words_of_a_windows_command_line_are_read_as_rust_reads_its_own() {
        for (line, words) in [
            (r"cin.exe ext list", &["cin.exe", "ext", "list"][..]),
            (
                r#""C:\Program Files\Cineris\cin.exe" ext"#,
                &[r"C:\Program Files\Cineris\cin.exe", "ext"],
            ),
            (r"C:\x\cin.exe", &[r"C:\x\cin.exe"]),
            (r#""C:\a b\"cin.exe ext"#, &[r"C:\a b\cin.exe", "ext"]),
            ("cin \t  ext\t\tlist  ", &["cin", "ext", "list"]),
            (r#"cin "ext""#, &["cin", "ext"]),
            (r#"cin "e"xt"#, &["cin", "ext"]),
            (r#"cin "ext list""#, &["cin", "ext list"]),
            (r#"cin "" ext"#, &["cin", "", "ext"]),
            (r#"cin """#, &["cin", ""]),
            (r#"cin ""#, &["cin", ""]),
            (r#"cin \"ext"#, &["cin", "\"ext"]),
            (r#"cin \\"ext list""#, &["cin", r"\ext list"]),
            (r#"cin \\\"ext"#, &["cin", "\\\"ext"]),
            (r"cin a\\b \x", &["cin", r"a\\b", r"\x"]),
            (r#"cin "a""b" c"#, &["cin", "a\"b", "c"]),
            (r#"cin "a"""#, &["cin", "a\""]),
            ("cin", &["cin"]),
            ("", &[""]),
        ] {
            assert_eq!(command_words(line), words, "{line:?}");
        }
    }

    /// A process is a harness by its program and its arguments, on both
    /// systems: it is Cineris, and its first argument is not a subcommand
    /// that runs no session; arguments that cannot be read count as running
    /// them. Read from a Windows command line, a quoted subcommand is the
    /// subcommand, and a word that only looks like one is not.
    #[test]
    fn a_harness_is_told_by_its_program_and_its_arguments() {
        let read = |line: &str| Some(command_words(line));
        let harness =
            |cineris, line: Option<Vec<String>>| runs_cineris_sessions(cineris, line.as_deref());
        assert!(harness(true, read("cin.exe")));
        assert!(harness(true, read(r#""C:\x y\cineris.exe" -p "hello""#)));
        assert!(harness(true, read("cin serve --stdio")));
        assert!(harness(true, read(r#"cin "ext"x"#)), "the word is extx");
        assert!(harness(true, read("cin EXT list")), "a subcommand's case");
        assert!(harness(true, None), "arguments that cannot be read");
        assert!(!harness(true, read("cin.exe ext list")));
        assert!(!harness(true, read(r#""C:\x y\cin.exe" "config" edit"#)));
        assert!(!harness(true, read("cin attach")));
        assert!(!harness(false, read("cin.exe")), "another program");
        assert!(!harness(false, None), "another program, arguments unread");
    }

    /// The Windows check's verdict, from jobs that each say a fixed thing:
    /// `jobs` gives each harness's (by its process id), and a harness not in
    /// it has none.
    fn verdict(
        parents: &[u32],
        listed: Result<Vec<u32>, &str>,
        jobs: &[(u32, Job)],
        service: Under,
    ) -> (Under, Option<Option<u64>>) {
        let asked = std::cell::Cell::new(None);
        let job = |pid: u32, _made: u64| {
            jobs.iter()
                .find(|(at, _)| *at == pid)
                .map_or(Job::None, |(_, job)| job.clone())
        };
        let under = judged(
            parents.iter().map(|pid| Parent {
                pid: *pid,
                made: u64::from(*pid) * 10,
                runs: Runs::No,
            }),
            || {
                listed
                    .map(|pids| {
                        pids.iter()
                            .map(|pid| (*pid, u64::from(*pid) * 10))
                            .collect()
                    })
                    .map_err(str::to_owned)
            },
            job,
            |began| {
                asked.set(Some(began));
                service
            },
        );
        (under, asked.get())
    }

    /// Being in a harness's job wins over another harness's job that cannot
    /// be told, whichever is met first, on the line of parents or among the
    /// harnesses listed, and so does a service's start (step 28's third pull
    /// request: a job made to deny its opening, named for another test's
    /// process, made the check of a process in its own harness's job say
    /// "not known"); with nothing that holds the process, the first job that
    /// cannot be told decides, over a listing that fails and a service's
    /// check that says nothing or cannot tell. A job that holds it decides
    /// at once, and the harnesses' jobs are weighed for when the earliest
    /// began.
    #[test]
    fn being_in_a_harnesss_job_wins_over_another_job_that_cannot_be_told() {
        let unknown = |what: &str| Job::Unknown(what.to_owned());
        let held = |how: &str| Job::Holds(how.to_owned());
        let session = |how: &str| Under::Session(how.to_owned());
        let not_known = |what: &str| Under::Unknown(what.to_owned());
        let jobs = [
            (1, unknown("planted")),
            (2, held("its own")),
            (3, unknown("another")),
            (4, Job::Elsewhere),
            (5, Job::Elsewhere),
        ];
        let jobs = &jobs[..];
        // Its own met after one that cannot be told, among those listed.
        assert_eq!(
            verdict(&[], Ok(vec![1, 2]), jobs, Under::Nothing).0,
            session("its own")
        );
        assert_eq!(
            verdict(&[], Ok(vec![2, 1]), jobs, Under::Nothing).0,
            session("its own")
        );
        // One that cannot be told on the line of parents, its own listed.
        assert_eq!(
            verdict(&[1], Ok(vec![1, 4, 2]), jobs, Under::Nothing).0,
            session("its own")
        );
        // A service's start, after one that cannot be told.
        assert_eq!(
            verdict(&[1], Ok(vec![3]), jobs, session("a service")).0,
            session("a service")
        );
        // Nothing holds it: the first that cannot be told.
        assert_eq!(
            verdict(&[3], Ok(vec![1, 4]), jobs, Under::Nothing).0,
            not_known("another")
        );
        assert_eq!(
            verdict(&[], Ok(vec![1, 3]), jobs, not_known("the line")).0,
            not_known("planted")
        );
        assert_eq!(
            verdict(&[1], Err("not listed"), jobs, Under::Nothing).0,
            not_known("planted")
        );
        assert_eq!(
            verdict(&[], Err("not listed"), jobs, Under::Nothing),
            (not_known("not listed"), None)
        );
        assert_eq!(
            verdict(&[], Ok(vec![4]), jobs, not_known("the line")).0,
            not_known("the line")
        );
        // Nothing at all.
        assert_eq!(
            verdict(&[6], Ok(vec![4, 6]), jobs, Under::Nothing).0,
            Under::Nothing
        );
        // A job that holds it on the line of parents: nothing is listed,
        // and no service asked.
        let (under, asked) = verdict(&[2], Err("not listed"), jobs, session("a service"));
        assert_eq!((under, asked), (session("its own"), None));
        // When the earliest running harness began: the least of theirs.
        assert_eq!(
            verdict(&[], Ok(vec![5, 1, 4]), jobs, Under::Nothing).1,
            Some(Some(40))
        );
        assert_eq!(
            verdict(&[], Ok(vec![1, 6]), jobs, Under::Nothing).1,
            Some(None)
        );
    }

    /// A process of the line of parents that is a harness by its program
    /// and its arguments makes this process a session's command whatever
    /// its place in that harness's job: outside it (a command that left
    /// it), with no job of its name (a job left nameless, a harness whose
    /// mark failed), or a job that cannot be told; and before anything is
    /// listed or any service asked. A job that holds this process, met
    /// first, still says so. One that is no harness (another program, or
    /// Cineris running a subcommand that runs no session) leaves the verdict
    /// to the rest; one whose program could not be read is not known, and
    /// refuses where nothing holds this process (the maintainers' rule for
    /// the third part, with the reviewer).
    #[test]
    fn a_harness_on_the_line_of_parents_is_a_session_whatever_its_job() {
        let parent = |pid: u32, runs: Runs| Parent {
            pid,
            made: u64::from(pid),
            runs,
        };
        let asked = std::cell::Cell::new((false, false));
        let verdict = |parents: Vec<Parent>, job: Job| {
            asked.set((false, false));
            judged(
                parents,
                || {
                    asked.set((true, asked.get().1));
                    Ok(Vec::new())
                },
                |_, _| job.clone(),
                |_| {
                    asked.set((asked.get().0, true));
                    Under::Nothing
                },
            )
        };
        let harness = || vec![parent(20, Runs::No), parent(10, Runs::Sessions)];
        for job in [
            Job::Elsewhere,
            Job::None,
            Job::Unknown("cannot be told".to_owned()),
        ] {
            assert_eq!(
                verdict(harness(), job.clone()),
                Under::Session("process 10 runs Cineris's sessions".to_owned()),
                "{job:?}"
            );
            assert_eq!(asked.get(), (false, false), "nothing listed or asked");
        }
        assert_eq!(
            verdict(harness(), Job::Holds("in its job".to_owned())),
            Under::Session("in its job".to_owned())
        );
        // No harness by its program: as its job says, then the rest.
        let other = || vec![parent(20, Runs::No), parent(10, Runs::No)];
        assert_eq!(verdict(other(), Job::Elsewhere), Under::Nothing);
        assert_eq!(asked.get(), (true, true));
        assert_eq!(verdict(other(), Job::None), Under::Nothing);
        // A program that could not be read: not known, unless something
        // holds this process, or a harness is met further up.
        let unread = |above: Runs| {
            vec![
                parent(20, Runs::Unknown("unread".to_owned())),
                parent(10, above),
            ]
        };
        assert_eq!(
            verdict(unread(Runs::No), Job::None),
            Under::Unknown("unread".to_owned())
        );
        assert_eq!(
            verdict(unread(Runs::Sessions), Job::None),
            Under::Session("process 10 runs Cineris's sessions".to_owned())
        );
        assert_eq!(
            verdict(unread(Runs::No), Job::Holds("in its job".to_owned())),
            Under::Session("in its job".to_owned())
        );
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
