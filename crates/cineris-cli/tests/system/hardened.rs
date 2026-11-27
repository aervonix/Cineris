//! The binary closes its own process to the user's other programs, first
//! thing in `main` (`cineris_sandbox::harden`; the owner's call 8 in the
//! signed approvals' work). On Linux it is not dumpable, so the test, its
//! parent and a process of the same user, cannot read its environment or
//! its program's link, nor open its memory, whatever Yama says; its
//! `stat` and `cmdline`, which the checks of a session's commands read,
//! stay readable. On Windows its process and each of its threads deny an
//! ordinary process of the user everything but waiting for it, the limited
//! questions and (of the process) ending it: the real binary, not only the
//! sandbox's test binary, which a build that left the hardening's thread
//! callback out would still pass (the full review of the signed
//! approvals). What a session's commands are, and what a nested run reads,
//! is `nested`'s.

#[cfg(target_os = "linux")]
#[test]
fn sys_the_binary_closes_its_own_process() {
    use std::os::unix::fs::MetadataExt as _;
    use std::process::Stdio;
    use std::time::Duration;

    let machine = crate::Machine::new("sys_the_binary_closes_its_own_process");
    let mut command = machine.command(&["serve", "--stdio"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("the binary starts");
    let pid = child.id();
    // Root's once the process is not dumpable: what `/proc` shows of a
    // process that closed itself.
    cineris_testkit::children::until(
        Duration::from_secs(10),
        "the binary to close its own process",
        || std::fs::metadata(format!("/proc/{pid}/mem")).is_ok_and(|mem| mem.uid() == 0),
    );
    let environ = std::fs::read(format!("/proc/{pid}/environ"));
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"));
    let mem = std::fs::File::open(format!("/proc/{pid}/mem"));
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"));
    drop(child.stdin.take());
    child.wait().unwrap();
    for (what, error) in [
        ("environ", environ.err()),
        ("exe", exe.err()),
        ("mem", mem.err()),
    ] {
        assert_eq!(
            error.map(|error| error.kind()),
            Some(std::io::ErrorKind::PermissionDenied),
            "{what}"
        );
    }
    assert!(stat.unwrap().starts_with(&format!("{pid} (cineris)")));
    assert!(cmdline.unwrap().ends_with(b"serve\0--stdio\0"));
}

/// The real binary (`serve --stdio`, which makes threads of its own after
/// `main` began: its core's and its runtime's) denies a probe, an ordinary
/// process of the user (this test's thread with its token's privileges and
/// Administrators taken away, as the runner's elevated token otherwise has
/// them), reading or writing its memory, making a thread in it, copying its
/// handles, changing it, suspending it and changing its access list; and,
/// of every one of its threads, their context, suspending them and changing
/// them. The probe can still open it to wait for it, ask it the limited
/// questions and end it, and its threads to wait for them and ask the
/// limited questions.
#[cfg(windows)]
#[test]
fn sys_the_binary_closes_its_own_process_and_every_thread() {
    use std::process::Stdio;
    use std::time::Duration;

    let machine = crate::Machine::new("sys_the_binary_closes_its_own_process_and_every_thread");
    let mut command = machine.command(&["serve", "--stdio"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("the binary starts");
    let pid = child.id();
    cineris_testkit::children::until(
        Duration::from_secs(10),
        "the binary to close its own process",
        || windows::process_denies_reading(pid),
    );
    // The threads it made after `main` began are there too.
    cineris_testkit::children::until(Duration::from_secs(10), "the binary's threads", || {
        windows::threads_of(pid).len() >= 2
    });
    let process = windows::try_process(pid);
    let threads: Vec<(u32, String)> = windows::threads_of(pid)
        .into_iter()
        .map(|id| (id, windows::try_thread(id)))
        .collect();
    drop(child.stdin.take());
    child.wait().unwrap();
    windows::judge(&process, &threads);
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "a process of the user's own that tries to open another and its threads is only \
              reachable through the system's calls"
)]
mod windows {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr;

    use windows_sys::Win32::Foundation::{FALSE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::{
        CreateRestrictedToken, CreateWellKnownSid, DISABLE_MAX_PRIVILEGE, DuplicateToken,
        RevertToSelf, SID_AND_ATTRIBUTES, SecurityImpersonation, TOKEN_DUPLICATE, TOKEN_QUERY,
        WinBuiltinAdministratorsSid,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, OpenThread, PROCESS_CREATE_THREAD,
        PROCESS_DUP_HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SET_INFORMATION, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE, PROCESS_VM_OPERATION,
        PROCESS_VM_READ, PROCESS_VM_WRITE, SetThreadToken, THREAD_GET_CONTEXT,
        THREAD_QUERY_LIMITED_INFORMATION, THREAD_SET_CONTEXT, THREAD_SET_INFORMATION,
        THREAD_SUSPEND_RESUME,
    };

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

    /// How many of [`PROCESS_RIGHTS`] a closed process denies.
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

    /// Runs `ask` as an ordinary process of the user would: this thread
    /// impersonating this process's token with no privilege and the
    /// Administrators group for denying only.
    fn as_ordinary<T>(ask: impl FnOnce() -> T) -> T {
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
        // SAFETY: a null thread is this thread; the token lives through the
        // impersonation, which is ended below.
        let set = unsafe { SetThreadToken(ptr::null(), impersonation.as_raw_handle()) };
        assert_ne!(set, 0, "{}", std::io::Error::last_os_error());
        let said = ask();
        // SAFETY: takes nothing.
        let reverted = unsafe { RevertToSelf() };
        assert_ne!(reverted, 0, "{}", std::io::Error::last_os_error());
        said
    }

    /// Each right of `rights` asked with `open`: `name=ok` or
    /// `name=denied(error)`, one after another.
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

    /// Whether the probe is denied reading the memory of process `pid`.
    pub(super) fn process_denies_reading(pid: u32) -> bool {
        as_ordinary(|| {
            // SAFETY: plain values.
            let handle = unsafe { OpenProcess(PROCESS_VM_READ, FALSE, pid) };
            if handle.is_null() {
                true
            } else {
                // SAFETY: just opened, owned here.
                drop(unsafe { OwnedHandle::from_raw_handle(handle) });
                false
            }
        })
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

    /// The ids of process `pid`'s threads, from a list of the system's
    /// threads, which needs no right on the process.
    pub(super) fn threads_of(pid: u32) -> Vec<u32> {
        // SAFETY: two integers; for a list of threads the second is not
        // used.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        assert_ne!(
            snapshot,
            INVALID_HANDLE_VALUE,
            "{}",
            std::io::Error::last_os_error()
        );
        // SAFETY: just made, owned here.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
        // SAFETY: plain data, for which all zeroes is valid.
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = u32::try_from(size_of::<THREADENTRY32>()).unwrap();
        let mut threads = Vec::new();
        // SAFETY: `snapshot` is open, and `entry` is writable and says how
        // large it is.
        let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &raw mut entry) } != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                threads.push(entry.th32ThreadID);
            }
            // SAFETY: as for `Thread32First`.
            more = unsafe { Thread32Next(snapshot.as_raw_handle(), &raw mut entry) } != 0;
        }
        threads
    }

    /// The verdict of `right` in a probe's line.
    fn verdict<'a>(line: &'a str, right: &str) -> &'a str {
        line.split(' ')
            .find_map(|part| part.strip_prefix(&format!("{right}=")))
            .unwrap_or_else(|| panic!("no {right} in {line}"))
    }

    pub(super) fn judge(process: &str, threads: &[(u32, String)]) {
        for (at, (right, _)) in PROCESS_RIGHTS.iter().enumerate() {
            let says = verdict(process, right);
            if at < PROCESS_DENIED {
                assert!(says.starts_with("denied"), "{right}: {process}");
            } else {
                assert_eq!(says, "ok", "{right}: {process}");
            }
        }
        // A thread that ended between the list and the probe cannot be
        // opened at all (`ERROR_INVALID_PARAMETER`): it is passed over.
        let threads: Vec<&(u32, String)> = threads
            .iter()
            .filter(|(_, thread)| verdict(thread, "query_limited") != "denied(87)")
            .collect();
        assert!(threads.len() >= 2, "{threads:?}");
        for (id, thread) in threads {
            for (at, (right, _)) in THREAD_RIGHTS.iter().enumerate() {
                let says = verdict(thread, right);
                if at < THREAD_DENIED {
                    assert!(says.starts_with("denied"), "thread {id}, {right}: {thread}");
                } else {
                    assert_eq!(says, "ok", "thread {id}, {right}: {thread}");
                }
            }
        }
    }
}
