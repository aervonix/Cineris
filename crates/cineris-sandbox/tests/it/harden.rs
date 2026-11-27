//! The harness's process closed to the user's other programs, for real
//! (`cineris_sandbox::harden`): this binary run again as a process that
//! closes itself, or as one that does not, to compare; each starts a
//! command; and another process of the same user, a child of the test and
//! the parent of neither, tries to read each.

#![allow(
    unsafe_code,
    reason = "a process of the user's own that tries to read another, and a process that lets \
              any process of its user trace it, are only reachable through the system's calls"
)]

use std::io::{BufReader, Read as _, Write as _};
use std::process::{Child, Command, Stdio};

/// The roles this binary is run again in: a process that closes itself
/// (`closed`) or does not (`open`) and starts a command; the command,
/// which waits; and the probe, another process of the user.
const TARGET: &str = "cin-harden-target";
const COMMAND: &str = "cin-harden-command";
const PROBE: &str = "cin-harden-probe";

/// On Windows: a process with a console of its own, whose standard input
/// is that console's input, inheritable, as a shell starts Cineris; it
/// closes itself (`closed`) or not (`open`), writes a marker into its
/// console's input, starts `CONSOLE_PROBE` with no console of the target's,
/// and says what the probe found.
#[cfg(windows)]
const CONSOLE_TARGET: &str = "cin-harden-console-target";

/// On Windows: a process that walks its handle values and looks, in every
/// console input among them, for the target's marker.
#[cfg(windows)]
const CONSOLE_PROBE: &str = "cin-harden-console-probe";

/// A harness that closes itself and, given `mark`, gives its word
/// (`session::mark`), then runs a program, with the input a file holds, and
/// ends as it does: `HARNESS -- <mark|bare> <input|-> <program file>`, the
/// program and its arguments in a file, each ended by a nul, so that none
/// of them is a filter of the test runner's (the launcher's own test's name
/// would run that test in the harness too).
pub(crate) const HARNESS: &str = "cin-harden-harness";

/// A process that says where it stands: whether it is under a session's
/// command, and which of its stores moved. It is entered by the crate's
/// `child_entry`, as a command the launcher becomes is, so that the
/// launcher, whose arguments hold the command's, is given one test's name
/// to run.
pub(crate) const STANDING: &str = "cin-harden-standing";

/// The test that runs a role.
const ENTRY: &str = "harden::the_roles_this_binary_is_run_again_in";

/// This binary run again in `role`, with `rest` after it.
fn in_role(role: &str, rest: &[String]) -> Command {
    in_role_of(&std::env::current_exe().unwrap(), role, rest)
}

/// `program`, a copy of this binary, run in `role`, with `rest` after it.
fn in_role_of(program: &std::path::Path, role: &str, rest: &[String]) -> Command {
    let mut command = Command::new(program);
    command
        .args(["--exact", ENTRY, "--nocapture", role, "--"])
        .args(rest);
    command
}

/// What the test runner runs as an ordinary test, which passes at once,
/// and what a test runs again in one of the roles above.
#[test]
fn the_roles_this_binary_is_run_again_in() {
    let arguments: Vec<String> = std::env::args().collect();
    let has = |role: &str| arguments.iter().any(|argument| argument == role);
    let after: Vec<String> = arguments
        .iter()
        .position(|argument| argument == "--")
        .map_or_else(Vec::new, |at| arguments[at + 1..].to_vec());
    #[cfg(target_os = "linux")]
    if has(BETWEEN) {
        let code = between(&after);
        #[expect(clippy::exit, reason = "a process between ends as what it ran")]
        std::process::exit(code);
    }
    if has(TARGET) {
        target(after.first().map(String::as_str) == Some("closed"));
    } else if has(COMMAND) {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
    } else if has(PROBE) {
        probe(&after);
    } else if cfg!(windows) && sys::console_role(&arguments) {
        // The role has said what it found.
    } else if has(HARNESS) {
        let code = harness(&after);
        #[expect(clippy::exit, reason = "a harness ends as what it ran")]
        std::process::exit(code);
    } else {
        return;
    }
    #[expect(clippy::exit, reason = "a role ends when it has done its part")]
    std::process::exit(0);
}

/// A target: lets any process of its user trace it where Yama would only
/// let its parent (so that what keeps the probe out is the closing alone),
/// closes itself if `closed`, starts its command, checks what it still
/// reads of itself, says `ready`, and waits for its input to end.
fn target(closed: bool) {
    #[cfg(target_os = "linux")]
    {
        const NONE: libc::c_ulong = 0;
        // SAFETY: `prctl` with integer arguments only, each as wide as the
        // kernel reads.
        unsafe {
            libc::prctl(
                libc::PR_SET_PTRACER,
                libc::PR_SET_PTRACER_ANY,
                NONE,
                NONE,
                NONE,
            );
        }
    }
    if closed {
        cineris_sandbox::harden::harden().unwrap();
    }
    let late = sys::thread_made_now();
    let mut command = in_role(COMMAND, &[])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let own = if closed {
        own_reads_after_closing()
    } else {
        "ok".to_owned()
    };
    println!("own: {own}");
    println!("ready {} {} {}", command.id(), late, sys::this_thread());
    std::io::stdout().flush().unwrap();
    let _ = std::io::stdin().read_to_end(&mut Vec::new());
    drop(command.stdin.take());
    command.wait().unwrap();
}

/// The `STANDING` role: says where this process stands, and ends. Left by
/// a `detach` (`BETWEEN`), it first waits for the process that left it to
/// end, so that it has no parent of its line when it looks.
pub(crate) fn say_standing() -> ! {
    use cineris_sandbox::session::{Moved, Under};
    #[cfg(target_os = "linux")]
    if let Some(pid) = std::env::var(DETACHED_FROM)
        .ok()
        .and_then(|pid| pid.parse().ok())
    {
        wait_for_the_end_of(pid);
    }
    let under = match cineris_sandbox::session::under_a_session() {
        Under::Nothing => "nothing".to_owned(),
        Under::Session(how) => format!("session ({how})"),
        Under::Unknown(what) => format!("unknown ({what})"),
    };
    println!("under: {under}");
    let moved = match cineris_sandbox::session::moved_stores() {
        None => "none".to_owned(),
        Some(Moved::Variable(name)) => name,
        Some(Moved::NotKnown(why)) => format!("not known ({why})"),
    };
    println!("moved: {moved}");
    std::io::stdout().flush().unwrap();
    #[expect(clippy::exit, reason = "a role ends when it has said where it stands")]
    std::process::exit(0);
}

/// The harness: closes itself, gives its word if told to, runs the program
/// with the input given, and returns its exit code.
fn harness(after: &[String]) -> i32 {
    cineris_sandbox::harden::harden().unwrap();
    if after[0] == "mark" {
        cineris_sandbox::session::mark().unwrap();
    }
    let input = (after[1] != "-").then(|| std::fs::read(&after[1]).unwrap());
    let program = std::fs::read_to_string(&after[2]).unwrap();
    let program: Vec<&str> = program.split_terminator('\0').collect();
    let mut command = Command::new(program[0]);
    command
        .args(&program[1..])
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        let _ = child.stdin.take().unwrap().write_all(&input);
    }
    child.wait().unwrap().code().unwrap_or(99)
}

/// What a closed process still reads of itself, as its crash report needs:
/// that it is closed and not traced, its own program, its status and
/// descriptors, and a backtrace that names its frames. `ok`, or what
/// failed.
#[inline(never)]
fn own_reads_after_closing() -> String {
    let mut failed = Vec::new();
    if !cineris_sandbox::harden::hardened() {
        failed.push("not closed".to_owned());
    }
    if cineris_sandbox::harden::traced() {
        failed.push("traced".to_owned());
    }
    if let Err(error) = std::env::current_exe() {
        failed.push(format!("its program: {error}"));
    }
    #[cfg(target_os = "linux")]
    {
        if let Err(error) = std::fs::read_to_string("/proc/self/status") {
            failed.push(format!("its status: {error}"));
        }
        if let Err(error) = std::fs::read_dir("/proc/self/fd") {
            failed.push(format!("its descriptors: {error}"));
        }
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        if !trace.contains("own_reads_after_closing") {
            failed.push(format!("its backtrace does not name its frames: {trace}"));
        }
    }
    #[cfg(windows)]
    {
        let trace = std::backtrace::Backtrace::force_capture();
        if trace.status() != std::backtrace::BacktraceStatus::Captured {
            failed.push("its backtrace was not taken".to_owned());
        }
    }
    if failed.is_empty() {
        "ok".to_owned()
    } else {
        failed.join("; ")
    }
}

/// The probe: what this process, another of the user's, may do to each
/// process and thread named, one line each.
fn probe(after: &[String]) {
    let mut out = std::io::stdout().lock();
    for named in after {
        let (kind, id) = named.split_once(':').unwrap();
        let id: u32 = id.parse().unwrap();
        let line = match kind {
            "process" => sys::try_process(id),
            "thread" => sys::try_thread(id),
            other => panic!("nothing to probe of a {other}"),
        };
        writeln!(out, "{named} {line}").unwrap();
    }
    out.flush().unwrap();
}

/// A target started and ready: it, its command's id, a thread it made after
/// it closed itself, and its first thread.
struct Started {
    child: Child,
    command: u32,
    late: u32,
    first: u32,
}

impl Started {
    fn start(closed: bool) -> Self {
        let mut child = in_role(TARGET, &[if closed { "closed" } else { "open" }.to_owned()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        match ready(BufReader::new(child.stdout.take().unwrap())) {
            Ok([command, late, first]) => Self {
                child,
                command,
                late,
                first,
            },
            Err(why) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the target did not come to be ready: {why}");
            }
        }
    }

    fn end(mut self) {
        drop(self.child.stdin.take());
        self.child.wait().unwrap();
    }
}

/// What a target says once it is ready: its command's id, a thread made
/// after it closed itself, and its first thread; or why not.
fn ready(mut out: impl std::io::BufRead) -> Result<[u32; 3], String> {
    let mut own = None;
    loop {
        let mut line = String::new();
        if out
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            return Err("it ended".to_owned());
        }
        if let Some(said) = line.trim_end().strip_prefix("own: ") {
            own = Some(said.to_owned());
        }
        if let Some(rest) = line.trim_end().strip_prefix("ready ") {
            if own.as_deref() != Some("ok") {
                return Err(format!("what it reads of itself: {own:?}"));
            }
            let ids: Vec<u32> = rest.split(' ').filter_map(|id| id.parse().ok()).collect();
            return <[u32; 3]>::try_from(ids).map_err(|ids| format!("its ids: {ids:?}"));
        }
    }
}

/// What the probe said of each thing it was asked about, by its name
/// (`process:<id>`, `thread:<id>`).
fn probed(asked: &[String]) -> Vec<(String, String)> {
    let output = in_role(PROBE, asked).output().unwrap();
    let said = String::from_utf8_lossy(&output.stdout);
    asked
        .iter()
        .map(|name| {
            let line = said
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name} ")))
                .unwrap_or_else(|| panic!("the probe said nothing of {name}: {said}"));
            (name.clone(), line.to_owned())
        })
        .collect()
}

/// The verdict of `right` in a probe's line (`right=ok` or
/// `right=denied(…)`).
fn verdict<'a>(line: &'a str, right: &str) -> &'a str {
    line.split(' ')
        .find_map(|part| part.strip_prefix(&format!("{right}=")))
        .unwrap_or_else(|| panic!("no {right} in {line}"))
}

/// A process that closed itself keeps out another process of its user,
/// which is neither its parent nor its child: on Linux that process cannot
/// read its environment or its program's link, open its memory, or attach
/// to it, where it can all of those of a process that did not close itself
/// (Yama aside, which each target is told to let by); and the command it
/// started is readable as any process of the user is. On Windows the
/// probe, as an ordinary process of the user (no privilege, Administrators
/// for denying only), cannot open the closed process to read or write its
/// memory, make a thread, copy its handles, change it, suspend it or change
/// its access list, nor open its threads (its first, and one made after it
/// closed itself) to read or set their context, suspend them or change
/// them; it can still open it to wait for it, ask it the limited questions
/// and end it, and open its command as it opens any process of the user.
/// What the closed process still reads of itself (its program, its status,
/// its descriptors, a backtrace with its frames' names) is checked by the
/// process itself, which says so before it is ready.
#[test]
fn sys_a_closed_process_keeps_out_another_process_of_its_user() {
    let open = Started::start(false);
    let closed = Started::start(true);
    let asked: Vec<String> = [
        format!("process:{}", open.child.id()),
        format!("process:{}", closed.child.id()),
        format!("process:{}", closed.command),
        format!("thread:{}", closed.first),
        format!("thread:{}", closed.late),
    ]
    .to_vec();
    let said = probed(&asked);
    open.end();
    closed.end();
    let [
        (_, open_line),
        (_, closed_line),
        (_, command_line),
        (_, first_line),
        (_, late_line),
    ] = <[(String, String); 5]>::try_from(said).unwrap();
    sys::judge(
        &open_line,
        &closed_line,
        &command_line,
        &first_line,
        &late_line,
    );
}

/// What closing a process costs, in a process that closes itself again and
/// again: the whole call each time (on Windows the lists made, set on the
/// process and on each of its threads, which a list of the system's threads
/// finds).
#[test]
#[ignore = "a measurement, in a release build: `perf -p cineris-sandbox` on a runner (`try`)"]
fn perf_closing_a_process() {
    let mut times: Vec<f64> = (0..50)
        .map(|_| {
            let start = std::time::Instant::now();
            cineris_sandbox::harden::harden().unwrap();
            start.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    times.sort_by(f64::total_cmp);
    cineris_testkit::numbers::report("harden.call", times[times.len() / 2], "us");
    let start = std::time::Instant::now();
    for _ in 0..50 {
        assert!(!cineris_sandbox::harden::traced());
    }
    cineris_testkit::numbers::report(
        "harden.traced",
        start.elapsed().as_secs_f64() * 1e6 / 50.0,
        "us",
    );
}

#[cfg(target_os = "linux")]
mod sys {
    use super::verdict;

    /// Windows's alone.
    pub(super) fn console_role(_arguments: &[String]) -> bool {
        false
    }

    pub(super) fn thread_made_now() -> u32 {
        0
    }

    pub(super) fn this_thread() -> u32 {
        0
    }

    fn told<T>(outcome: std::io::Result<T>) -> String {
        match outcome {
            Ok(_) => "ok".to_owned(),
            Err(error) => format!("denied({})", error.raw_os_error().unwrap_or(0)),
        }
    }

    /// Its environment and its program's link (read as any process of the
    /// user may, Yama aside), its memory opened (as only a tracer may), and
    /// an attach (`PTRACE_SEIZE`, which does not stop it; the tracer's end
    /// lets it go).
    pub(super) fn try_process(pid: u32) -> String {
        let environ = told(std::fs::read(format!("/proc/{pid}/environ")));
        let exe = told(std::fs::read_link(format!("/proc/{pid}/exe")));
        let mem = told(std::fs::File::open(format!("/proc/{pid}/mem")));
        let traced = libc::pid_t::try_from(pid).unwrap();
        let none = std::ptr::null_mut::<libc::c_void>();
        // SAFETY: a process id and two null pointers, which a seize does not
        // read; a seize does not stop the process.
        let attach = if unsafe { libc::ptrace(libc::PTRACE_SEIZE, traced, none, none) } == 0 {
            "ok".to_owned()
        } else {
            told::<()>(Err(std::io::Error::last_os_error()))
        };
        format!("environ={environ} exe={exe} mem={mem} attach={attach}")
    }

    pub(super) fn try_thread(_id: u32) -> String {
        "none".to_owned()
    }

    /// Yama's scope, where it is built in: from 2 up, no process of the
    /// user may attach, whatever a target lets.
    fn yama_scope() -> u8 {
        std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0)
    }

    pub(super) fn judge(open: &str, closed: &str, command: &str, _first: &str, _late: &str) {
        for right in ["environ", "exe"] {
            assert_eq!(verdict(open, right), "ok", "{right}: {open}");
            assert_eq!(verdict(command, right), "ok", "{right}: {command}");
        }
        if yama_scope() < 2 {
            for right in ["mem", "attach"] {
                assert_eq!(verdict(open, right), "ok", "{right}: {open}");
            }
        } else {
            println!("Yama's scope is 2 or more: no process may attach, closed or not");
        }
        for right in ["environ", "exe", "mem", "attach"] {
            assert!(
                verdict(closed, right).starts_with("denied"),
                "{right}: {closed}"
            );
        }
    }
}

/// A process that closed itself keeps its standard handles from what it
/// starts (Windows; the OS-layer review): a shell starts Cineris with an
/// inheritable console input, and a process made with handles inherited,
/// even with no console of its own (`CREATE_NO_WINDOW`), would have it at
/// the same value, to peek at or read what the person types, or write
/// keys. The target writes a marker into its own console's input; a probe
/// it starts walks its handle values and peeks into every console input
/// among them. A target that did not close itself has it found, which
/// shows the walk finds what it looks for; one that did has it not found.
#[cfg(windows)]
#[test]
fn sys_a_closed_process_keeps_its_console_from_what_it_starts() {
    use std::os::windows::process::CommandExt as _;
    /// `CREATE_NO_WINDOW`: a console of its own, with no window.
    const NO_WINDOW: u32 = 0x0800_0000;
    let found = |closed: &str| {
        let output = in_role(CONSOLE_TARGET, &[closed.to_owned()])
            .creation_flags(NO_WINDOW)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&output.stdout).into_owned();
        said.lines()
            .find_map(|line| line.strip_prefix("console: "))
            .unwrap_or_else(|| {
                panic!(
                    "the target said nothing of its console: {said}{}",
                    String::from_utf8_lossy(&output.stderr)
                )
            })
            .to_owned()
    };
    assert_eq!(found("open"), "found", "a target that did not close itself");
    assert_eq!(found("closed"), "none", "a target that closed itself");
}

#[cfg(windows)]
mod sys {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Stdio;
    use std::ptr;

    use windows_sys::Win32::Foundation::{FALSE, HANDLE};
    use windows_sys::Win32::Security::{
        CreateRestrictedToken, CreateWellKnownSid, DISABLE_MAX_PRIVILEGE, DuplicateToken,
        RevertToSelf, SID_AND_ATTRIBUTES, SecurityImpersonation, TOKEN_DUPLICATE, TOKEN_QUERY,
        WinBuiltinAdministratorsSid,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken, OpenThread,
        PROCESS_CREATE_THREAD, PROCESS_DUP_HANDLE, PROCESS_QUERY_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SUSPEND_RESUME,
        PROCESS_TERMINATE, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE, SetThreadToken,
        THREAD_GET_CONTEXT, THREAD_QUERY_LIMITED_INFORMATION, THREAD_SET_CONTEXT,
        THREAD_SET_INFORMATION, THREAD_SUSPEND_RESUME,
    };

    use super::verdict;

    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WRITE_DAC: u32 = 0x0004_0000;
    const WRITE_OWNER: u32 = 0x0008_0000;

    /// The rights of a process asked, by name: those a closed process
    /// denies, then those it keeps.
    const PROCESS_RIGHTS: [(&str, u32); 13] = [
        ("vm_read", PROCESS_VM_READ),
        ("vm_write", PROCESS_VM_WRITE),
        ("vm_operation", PROCESS_VM_OPERATION),
        ("create_thread", PROCESS_CREATE_THREAD),
        ("dup_handle", PROCESS_DUP_HANDLE),
        ("set_information", PROCESS_SET_INFORMATION),
        ("suspend_resume", PROCESS_SUSPEND_RESUME),
        ("query_information", PROCESS_QUERY_INFORMATION),
        ("write_dac", WRITE_DAC),
        ("write_owner", WRITE_OWNER),
        ("query_limited", PROCESS_QUERY_LIMITED_INFORMATION),
        ("synchronize", SYNCHRONIZE),
        ("terminate", PROCESS_TERMINATE),
    ];

    /// How many of [`PROCESS_RIGHTS`] are denied by a closed process.
    const PROCESS_DENIED: usize = 10;

    /// The rights of a thread asked: denied, then kept.
    const THREAD_RIGHTS: [(&str, u32); 7] = [
        ("get_context", THREAD_GET_CONTEXT),
        ("set_context", THREAD_SET_CONTEXT),
        ("suspend_resume", THREAD_SUSPEND_RESUME),
        ("set_information", THREAD_SET_INFORMATION),
        ("write_dac", WRITE_DAC),
        ("query_limited", THREAD_QUERY_LIMITED_INFORMATION),
        ("synchronize", SYNCHRONIZE),
    ];

    const THREAD_DENIED: usize = 5;

    /// A thread made now, which waits for good: its id.
    pub(super) fn thread_made_now() -> u32 {
        let (told, id) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // SAFETY: takes nothing.
            told.send(unsafe { GetCurrentThreadId() }).unwrap();
            loop {
                std::thread::park();
            }
        });
        id.recv().unwrap()
    }

    pub(super) fn this_thread() -> u32 {
        // SAFETY: takes nothing.
        unsafe { GetCurrentThreadId() }
    }

    /// Runs `ask` as an ordinary process of the user would: this thread
    /// impersonating this process's token with no privilege and the
    /// Administrators group for denying only, which a runner's elevated
    /// token otherwise has.
    fn as_ordinary(ask: impl FnOnce() -> String) -> String {
        let mut own: HANDLE = ptr::null_mut();
        // SAFETY: the pseudo-handle is always valid; `own` is a live place.
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_DUPLICATE | TOKEN_QUERY,
                &raw mut own,
            )
        };
        assert_ne!(opened, 0, "{}", std::io::Error::last_os_error());
        // SAFETY: just opened, owned here.
        let own = unsafe { OwnedHandle::from_raw_handle(own) };
        let mut administrators = [0_u32; 17];
        let mut size = u32::try_from(size_of_val(&administrators)).unwrap();
        // SAFETY: a buffer as large as said.
        let made = unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                ptr::null_mut(),
                administrators.as_mut_ptr().cast(),
                &raw mut size,
            )
        };
        assert_ne!(made, 0, "{}", std::io::Error::last_os_error());
        let disabled = SID_AND_ATTRIBUTES {
            Sid: administrators.as_mut_ptr().cast(),
            Attributes: 0,
        };
        let mut restricted: HANDLE = ptr::null_mut();
        // SAFETY: `own` is open to duplicate; one SID to disable, which
        // lives through the call; nothing deleted or restricted.
        let made = unsafe {
            CreateRestrictedToken(
                own.as_raw_handle(),
                DISABLE_MAX_PRIVILEGE,
                1,
                &raw const disabled,
                0,
                ptr::null(),
                0,
                ptr::null(),
                &raw mut restricted,
            )
        };
        assert_ne!(made, 0, "{}", std::io::Error::last_os_error());
        // SAFETY: just made, owned here.
        let restricted = unsafe { OwnedHandle::from_raw_handle(restricted) };
        let mut impersonation: HANDLE = ptr::null_mut();
        // SAFETY: `restricted` is a token this process made, and
        // `impersonation` is a live place.
        let made = unsafe {
            DuplicateToken(
                restricted.as_raw_handle(),
                SecurityImpersonation,
                &raw mut impersonation,
            )
        };
        assert_ne!(made, 0, "{}", std::io::Error::last_os_error());
        // SAFETY: just made, owned here.
        let impersonation = unsafe { OwnedHandle::from_raw_handle(impersonation) };
        // SAFETY: a null thread is this thread; the token lives through
        // the impersonation, which is ended below.
        let set = unsafe { SetThreadToken(ptr::null(), impersonation.as_raw_handle()) };
        assert_ne!(set, 0, "{}", std::io::Error::last_os_error());
        let said = ask();
        // SAFETY: takes nothing.
        let reverted = unsafe { RevertToSelf() };
        assert_ne!(reverted, 0, "{}", std::io::Error::last_os_error());
        said
    }

    fn line(rights: &[(&str, u32)], open: impl Fn(u32) -> HANDLE) -> String {
        rights
            .iter()
            .map(|(name, right)| {
                let handle = open(*right);
                if handle.is_null() {
                    let error = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    format!("{name}=denied({error})")
                } else {
                    // SAFETY: just opened, owned here.
                    drop(unsafe { OwnedHandle::from_raw_handle(handle) });
                    format!("{name}=ok")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub(super) fn try_process(pid: u32) -> String {
        let open = |right| {
            // SAFETY: plain values.
            unsafe { OpenProcess(right, FALSE, pid) }
        };
        as_ordinary(|| line(&PROCESS_RIGHTS, open))
    }

    pub(super) fn try_thread(id: u32) -> String {
        let open = |right| {
            // SAFETY: plain values.
            unsafe { OpenThread(right, FALSE, id) }
        };
        as_ordinary(|| line(&THREAD_RIGHTS, open))
    }

    /// The marker a console target writes: a key with a scan code no
    /// keyboard sends, and a snowman.
    const MARKER_SCAN: u16 = 0x2a2a;
    const MARKER_CHAR: u16 = 0x2603;

    /// Runs a console role when `arguments` name one: `true` when it did.
    pub(super) fn console_role(arguments: &[String]) -> bool {
        let has = |role: &str| arguments.iter().any(|argument| argument == role);
        if has(super::CONSOLE_TARGET) {
            let closed = arguments.iter().any(|argument| argument == "closed");
            console_target(closed);
            true
        } else if has(super::CONSOLE_PROBE) {
            let found = console_probe();
            println!("probe: {}", if found { "found" } else { "none" });
            true
        } else {
            false
        }
    }

    fn console_target(closed: bool) {
        use std::io::Write as _;
        use std::os::windows::process::CommandExt as _;
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows_sys::Win32::System::Console::{
            INPUT_RECORD, KEY_EVENT, STD_INPUT_HANDLE, SetStdHandle, WriteConsoleInputW,
        };
        const GENERIC_READ: u32 = 0x8000_0000;
        const GENERIC_WRITE: u32 = 0x4000_0000;
        /// `CREATE_NO_WINDOW`: a console of its own, with no window.
        const NO_WINDOW: u32 = 0x0800_0000;
        let name: Vec<u16> = "CONIN$\0".encode_utf16().collect();
        let inherited = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap(),
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 1,
        };
        // SAFETY: a terminated name, and attributes that live through the
        // call.
        let input = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &raw const inherited,
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        };
        assert!(
            !input.is_null() && input != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            "the console's input: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: `input` is open, and stays open for the process's life.
        assert_ne!(unsafe { SetStdHandle(STD_INPUT_HANDLE, input) }, 0);
        if closed {
            cineris_sandbox::harden::harden().unwrap();
        }
        // SAFETY: plain data, for which all zeroes is valid.
        let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
        record.EventType = u16::try_from(KEY_EVENT).unwrap();
        record.Event.KeyEvent.bKeyDown = 1;
        record.Event.KeyEvent.wRepeatCount = 1;
        record.Event.KeyEvent.wVirtualScanCode = MARKER_SCAN;
        record.Event.KeyEvent.uChar.UnicodeChar = MARKER_CHAR;
        let mut written = 0_u32;
        // SAFETY: `input` is a console input; one record, which lives
        // through the call.
        let wrote = unsafe { WriteConsoleInputW(input, &raw const record, 1, &raw mut written) };
        assert!(
            wrote != 0 && written == 1,
            "{}",
            std::io::Error::last_os_error()
        );
        let output = super::in_role(super::CONSOLE_PROBE, &[])
            .creation_flags(NO_WINDOW)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&output.stdout);
        let found = said
            .lines()
            .find_map(|line| line.strip_prefix("probe: "))
            .unwrap_or("nothing said");
        let mut out = std::io::stdout().lock();
        writeln!(out, "console: {found}").unwrap();
        out.flush().unwrap();
    }

    /// Whether a console input among this process's handle values holds
    /// the marker.
    fn console_probe() -> bool {
        use windows_sys::Win32::System::Console::{
            GetConsoleMode, INPUT_RECORD, KEY_EVENT, PeekConsoleInputW,
        };
        (4_usize..=16_384).step_by(4).any(|value| {
            let handle: HANDLE = ptr::without_provenance_mut(value);
            let mut mode = 0_u32;
            // SAFETY: any value may be asked: one that is no console
            // handle fails the call.
            if unsafe { GetConsoleMode(handle, &raw mut mode) } == 0 {
                return false;
            }
            // SAFETY: plain data, for which all zeroes is valid.
            let mut records: [INPUT_RECORD; 64] = unsafe { std::mem::zeroed() };
            let mut read = 0_u32;
            // SAFETY: the buffer holds as many records as said; peeking
            // takes none of them.
            let peeked =
                unsafe { PeekConsoleInputW(handle, records.as_mut_ptr(), 64, &raw mut read) };
            peeked != 0
                && records
                    .iter()
                    .take(usize::try_from(read).unwrap_or(0))
                    .any(|record| {
                        u32::from(record.EventType) == KEY_EVENT
                            // SAFETY: a key event's record holds a key.
                            && unsafe {
                                record.Event.KeyEvent.wVirtualScanCode == MARKER_SCAN
                                    && record.Event.KeyEvent.uChar.UnicodeChar == MARKER_CHAR
                            }
                    })
        })
    }

    pub(super) fn judge(open: &str, closed: &str, command: &str, first: &str, late: &str) {
        for (at, (right, _)) in PROCESS_RIGHTS.iter().enumerate() {
            let closed_says = verdict(closed, right);
            if at < PROCESS_DENIED {
                assert!(closed_says.starts_with("denied"), "{right}: {closed}");
            } else {
                assert_eq!(closed_says, "ok", "{right}: {closed}");
            }
            // The command is as any process the user starts.
            assert_eq!(
                verdict(command, right),
                verdict(open, right),
                "{right}: the command {command}, a process not closed {open}"
            );
        }
        for thread in [first, late] {
            for (at, (right, _)) in THREAD_RIGHTS.iter().enumerate() {
                let says = verdict(thread, right);
                if at < THREAD_DENIED {
                    assert!(says.starts_with("denied"), "{right}: {thread}");
                } else {
                    assert_eq!(says, "ok", "{right}: {thread}");
                }
            }
        }
    }
}

/// What a process run by a closed harness (`HARNESS`) says of where it
/// stands, the harness given `mark` or `bare`, the input file or `-`, and
/// the program and its arguments.
#[cfg(target_os = "linux")]
pub(crate) fn standing(
    scratch: &cineris_testkit::scratch::Scratch,
    mark: &str,
    input: &str,
    program: &[String],
) -> (String, String) {
    standing_under(
        scratch,
        &std::env::current_exe().unwrap(),
        mark,
        input,
        program,
    )
}

/// A program and its arguments in a file of `scratch`, each ended by a
/// nul, as `HARNESS` and `BETWEEN` read what they run.
#[cfg(target_os = "linux")]
fn written(scratch: &cineris_testkit::scratch::Scratch, program: &[String]) -> String {
    static WRITTEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let at = WRITTEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut text = String::new();
    for word in program {
        text.push_str(word);
        text.push('\0');
    }
    scratch
        .file(format!("program-{at}"), text)
        .display()
        .to_string()
}

/// As [`standing`], the harness run from `harness`, a copy of this binary.
#[cfg(target_os = "linux")]
fn standing_under(
    scratch: &cineris_testkit::scratch::Scratch,
    harness: &std::path::Path,
    mark: &str,
    input: &str,
    program: &[String],
) -> (String, String) {
    let rest = [mark.to_owned(), input.to_owned(), written(scratch, program)];
    let output = in_role_of(harness, HARNESS, &rest).output().unwrap();
    let said = String::from_utf8_lossy(&output.stdout).into_owned();
    let line = |prefix: &str| {
        said.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_else(|| {
                panic!(
                    "nothing said `{prefix}`: {said}{}",
                    String::from_utf8_lossy(&output.stderr)
                )
            })
            .to_owned()
    };
    (line("under: "), line("moved: "))
}

/// A copy of this binary under another name and as another file: a
/// process of it is neither the very file a process of this binary runs,
/// nor named as this binary is, so the closed harness above it (this binary,
/// whose file it does not let be read) is told only by its word.
#[cfg(target_os = "linux")]
pub(crate) fn other_copy(scratch: &cineris_testkit::scratch::Scratch) -> String {
    let copy = scratch.join("other-name");
    std::fs::copy(std::env::current_exe().unwrap(), &copy).unwrap();
    copy.display().to_string()
}

/// The arguments that run a copy of this binary as `STANDING`.
#[cfg(target_os = "linux")]
pub(crate) fn standing_arguments(program: &str) -> Vec<String> {
    [program, "--exact", crate::ENTRY, "--nocapture", STANDING]
        .map(str::to_owned)
        .to_vec()
}

/// A closed harness's word reaches what it starts, and decides for it
/// (Linux, where the harness's `environ` and `exe` are root's to read once
/// it is closed): a process of another file under another name, which
/// nothing else tells the harness to, is under a session's command by its
/// word, and its stores are compared with what the word says: the same
/// stores pass, and a moved `HOME` is named. A harness that gave no word is
/// still told by its name, and its nested run refuses, since what its
/// stores were is not known.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_closed_harness_is_told_by_the_word_it_gives() {
    let scratch = cineris_testkit::scratch::Scratch::outside(
        "cineris-sandbox-harden",
        "sys_a_closed_harness_is_told_by_the_word_it_gives",
    );
    let copy = other_copy(&scratch);
    let (under, moved) = standing(&scratch, "mark", "-", &standing_arguments(&copy));
    assert!(under.starts_with("session"), "{under}");
    assert_eq!(moved, "none");

    let mut elsewhere = vec![
        "/usr/bin/env".to_owned(),
        format!("HOME={}", scratch.join("elsewhere").display()),
    ];
    elsewhere.extend(standing_arguments(&copy));
    let (under, moved) = standing(&scratch, "mark", "-", &elsewhere);
    assert!(under.starts_with("session"), "{under}");
    assert_eq!(moved, "HOME");

    let own = std::env::current_exe().unwrap().display().to_string();
    let (under, moved) = standing(&scratch, "bare", "-", &standing_arguments(&own));
    assert!(under.starts_with("session"), "{under}");
    assert!(
        moved.starts_with("not known") && moved.contains("no word"),
        "{moved}"
    );
}

/// On Linux: a process between a harness and what it runs, as a session's
/// command's own would be: `BETWEEN -- <actions> <home|-> <program file>`.
/// The actions, joined by commas: `close` closes every harness's word it
/// holds; `forge` makes a word of its own, naming itself, that says its
/// stores with `HOME` as given; `rename` gives it a name that is not UTF-8;
/// `detach` starts the program and ends at once, so that the program is
/// left with no parent of its line. It runs the program (its words in the
/// file, each ended by a nul, so that none is a filter of the test
/// runner's), with `HOME` as given (`-` leaves it), and ends as it does.
#[cfg(target_os = "linux")]
const BETWEEN: &str = "cin-harden-between";

/// Where a program `detach` left is told which process to wait to see end
/// before it says where it stands: its parent, of its line.
#[cfg(target_os = "linux")]
const DETACHED_FROM: &str = "CIN_HARDEN_DETACHED_FROM";

/// The arguments that run this binary as `BETWEEN`.
#[cfg(target_os = "linux")]
fn between_arguments(actions: &str, home: &str, program_file: &str) -> Vec<String> {
    let own = std::env::current_exe().unwrap().display().to_string();
    [
        own.as_str(),
        "--exact",
        ENTRY,
        "--nocapture",
        BETWEEN,
        "--",
        actions,
        home,
        program_file,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// The `BETWEEN` role.
#[cfg(target_os = "linux")]
fn between(after: &[String]) -> i32 {
    let actions: Vec<&str> = after[0].split(',').collect();
    let home = (after[1] != "-").then(|| after[1].clone());
    let program = std::fs::read_to_string(&after[2]).unwrap();
    let program: Vec<&str> = program.split_terminator('\0').collect();
    if actions.contains(&"close") {
        for word in words_held() {
            // SAFETY: closes a descriptor this process holds and nothing in
            // it uses: a sequenced-packet socket it inherited.
            unsafe { libc::close(word) };
        }
    }
    // Kept for the process's life: its maker runs while what it starts
    // does.
    let _forged = actions.contains(&"forge").then(|| forge(home.as_deref()));
    if actions.contains(&"rename") {
        // The thread group's own name, which `/proc/<pid>/stat` shows,
        // whichever thread writes it.
        std::fs::write("/proc/self/comm", b"\xff\xfe-x").unwrap();
    }
    let mut command = Command::new(program[0]);
    command.args(&program[1..]);
    if let Some(home) = &home {
        command.env("HOME", home);
    }
    if actions.contains(&"detach") {
        #[allow(
            clippy::zombie_processes,
            reason = "this process ends at once, and the program it leaves is handed to another \
                      parent, which waits for it"
        )]
        let _left = command
            .env(DETACHED_FROM, std::process::id().to_string())
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        return 0;
    }
    command.status().unwrap().code().unwrap_or(99)
}

/// The sequenced-packet sockets this process holds: a harness's word is
/// one.
#[cfg(target_os = "linux")]
fn words_held() -> Vec<i32> {
    let held: Vec<i32> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect();
    held.into_iter()
        .filter(|fd| {
            let mut kind: libc::c_int = 0;
            let mut length = libc::socklen_t::try_from(size_of::<libc::c_int>()).unwrap();
            // SAFETY: `kind` and `length` are live, and `length` says how
            // large `kind` is; a descriptor that is not a socket fails.
            let asked = unsafe {
                libc::getsockopt(
                    *fd,
                    libc::SOL_SOCKET,
                    libc::SO_TYPE,
                    (&raw mut kind).cast(),
                    &raw mut length,
                )
            };
            asked == 0 && kind == libc::SOCK_SEQPACKET
        })
        .collect()
}

/// A word this process makes of its own, as a command could: a pair of
/// sequenced-packet sockets, the record Cineris writes (its tag, this
/// process's start, and its stores, `HOME` as given) sent into the end
/// kept, the other end closed, and the end kept inherited by what it
/// starts. Its maker, as the kernel says, is this process.
#[cfg(target_os = "linux")]
fn forge(home: Option<&str>) -> std::os::fd::OwnedFd {
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::os::unix::ffi::OsStrExt as _;
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let start = stat[stat.rfind(')').unwrap() + 1..]
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_owned();
    let mut record = b"cineris-session-stores 1\0".to_vec();
    record.extend_from_slice(start.as_bytes());
    record.push(0);
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_DATA_HOME",
        "CINERIS_CONFIG",
        "CINERIS_DATA_DIR",
    ] {
        let value = match home {
            Some(home) if name == "HOME" => Some(std::ffi::OsString::from(home)),
            _ => std::env::var_os(name),
        };
        if let Some(value) = value {
            record.extend_from_slice(name.as_bytes());
            record.push(b'=');
            record.extend_from_slice(value.as_bytes());
            record.push(0);
        }
    }
    let mut pair = [0; 2];
    // SAFETY: `pair` has room for the two descriptors the call writes.
    let made =
        unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0, pair.as_mut_ptr()) };
    assert_eq!(made, 0, "{}", std::io::Error::last_os_error());
    // SAFETY: both were just made by the call and nothing else owns them.
    let (kept, writer) = unsafe { (OwnedFd::from_raw_fd(pair[0]), OwnedFd::from_raw_fd(pair[1])) };
    // SAFETY: `writer` is open, and the record lives through the call.
    let sent = unsafe { libc::send(writer.as_raw_fd(), record.as_ptr().cast(), record.len(), 0) };
    assert_eq!(usize::try_from(sent).ok(), Some(record.len()));
    drop(writer);
    kept
}

/// Waits for the process `pid` to end, by a descriptor of it
/// (`pidfd_open`), which becomes readable when it has ended, after the
/// processes it started were handed to another parent. One gone already
/// has handed them on.
#[cfg(target_os = "linux")]
fn wait_for_the_end_of(pid: u32) {
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    let pid = libc::pid_t::try_from(pid).unwrap();
    // SAFETY: the call takes a process id and flags, and gives a new
    // descriptor or -1.
    let made = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    let Some(made) = i32::try_from(made).ok().filter(|fd| *fd >= 0) else {
        return;
    };
    // SAFETY: just made by the call, and owned by nothing else.
    let end = unsafe { OwnedFd::from_raw_fd(made) };
    let mut ask = libc::pollfd {
        fd: end.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one live `pollfd`; no time limit, the process's end is waited
    // for, as a test waits for any event, bounded by the runner's limit.
    while unsafe { libc::poll(&raw mut ask, 1, -1) } < 0
        && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
    {}
}

/// A command's process between a closed harness and a nested run that
/// makes a word of its own, naming itself and a moved `HOME`, does not
/// stand for the session: the line is read on to the harness, whose word
/// says otherwise; and where the command closed the harness's word too, a
/// harness that is known by its name (`cin`) gave none it holds, which
/// refuses (the review of the harness's word, W1: the nearest word alone
/// decided, and the moved `HOME` passed).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_word_a_command_makes_does_not_stand_for_its_session() {
    let scratch = cineris_testkit::scratch::Scratch::outside(
        "cineris-sandbox-harden",
        "sys_a_word_a_command_makes_does_not_stand_for_its_session",
    );
    let copy = other_copy(&scratch);
    let elsewhere = scratch.join("elsewhere").display().to_string();
    let standing_file = written(&scratch, &standing_arguments(&copy));
    let forged = between_arguments("forge", &elsewhere, &standing_file);
    let (under, moved) = standing(&scratch, "mark", "-", &forged);
    assert!(under.starts_with("session"), "{under}");
    assert_eq!(moved, "HOME");

    let cin = scratch.join("cin");
    std::fs::copy(std::env::current_exe().unwrap(), &cin).unwrap();
    let closed = between_arguments("close,forge", &elsewhere, &standing_file);
    let (under, moved) = standing_under(&scratch, &cin, "mark", "-", &closed);
    assert!(under.starts_with("session"), "{under}");
    assert!(
        moved.starts_with("not known") && moved.contains("no word"),
        "{moved}"
    );
}

/// A process of the line whose name is not UTF-8 leaves the line not
/// known; with the harness's word closed too, nothing tells where the
/// session's stores are, and a nested run with `HOME` moved is refused (the
/// review of the harness's word, W2: only the `CINERIS_` variables were
/// compared, and it passed).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_line_that_cannot_be_read_refuses_a_nested_runs_stores() {
    let scratch = cineris_testkit::scratch::Scratch::outside(
        "cineris-sandbox-harden",
        "sys_a_line_that_cannot_be_read_refuses_a_nested_runs_stores",
    );
    let copy = other_copy(&scratch);
    let elsewhere = scratch.join("elsewhere").display().to_string();
    let standing_file = written(&scratch, &standing_arguments(&copy));
    let renamed = between_arguments("close,rename", &elsewhere, &standing_file);
    let (under, moved) = standing(&scratch, "mark", "-", &renamed);
    assert!(under.starts_with("unknown"), "{under}");
    assert!(
        moved.starts_with("not known") && moved.contains("could not be read"),
        "{moved}"
    );
}

/// A command that leaves its line of parents (its parent ends, and it is
/// handed to the system's first process or a subreaper) and keeps its
/// harness's word is under the session's command while the harness runs
/// (the review of the harness's word, W3: words were asked only of the
/// processes of the line); a nested run there finds no harness on its line
/// to read its session's stores from, and is refused, with its stores moved
/// or not.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_command_that_left_its_line_is_under_its_session_by_its_word() {
    let scratch = cineris_testkit::scratch::Scratch::outside(
        "cineris-sandbox-harden",
        "sys_a_command_that_left_its_line_is_under_its_session_by_its_word",
    );
    let copy = other_copy(&scratch);
    let elsewhere = scratch.join("elsewhere").display().to_string();
    let standing_file = written(&scratch, &standing_arguments(&copy));
    for home in ["-", elsewhere.as_str()] {
        // The harness waits on `cat`, which waits for the program the
        // process between left, on the pipe it holds.
        let mut program: Vec<String> = ["/bin/sh", "-c", "\"$@\" | cat", "sh"]
            .map(str::to_owned)
            .to_vec();
        program.extend(between_arguments("detach", home, &standing_file));
        let (under, moved) = standing(&scratch, "mark", "-", &program);
        assert!(
            under.starts_with("session") && under.contains("holds the word"),
            "{under}"
        );
        assert!(
            moved.starts_with("not known") && moved.contains("no process of its line"),
            "{home}: {moved}"
        );
    }
}
