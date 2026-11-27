//! The harness's own process, closed to the user's other programs (since
//! the signed approvals' work: the owner's call 8, that the harness closes
//! its own process, in this step).
//!
//! A session holds the person's checking key for its life, the key a
//! password makes while a signature is made, and the password while it is
//! typed. Code the model runs with the user's rights, where nothing confines
//! it, could otherwise open the harness and read them, or write the checking
//! key so that the session accepts what it forged. [`harden`] is the first
//! thing every process of Cineris's binary does, before it reads anything:
//!
//! - **On Linux, the process is made not dumpable** (`prctl(PR_SET_DUMPABLE,
//!   0)`). A process of the same user without `CAP_SYS_PTRACE` then cannot
//!   attach to it with `ptrace`, nor open its `/proc/<pid>/mem`, `environ`,
//!   `fd`, `maps` or `exe`, whatever Yama's `ptrace_scope` says, and no core
//!   dump is written of it. Its `stat`, `status`, `comm` and `cmdline` stay
//!   readable, and so does its `/proc/<pid>` folder's owner: what the checks
//!   of [`crate::session`] and the RPC server's peer check read of a
//!   harness. A program it starts is dumpable again from its `execve`, so a
//!   session's commands are as they were. Where `/proc` is mounted with
//!   `hidepid`, a process that is not dumpable is hidden from the other
//!   processes of its user: [`crate::session`] counts such a process on a
//!   line of parents as not known.
//! - **On Windows, its process and each of its threads get an access list**
//!   that gives its user, and `OWNER RIGHTS` (S-1-3-4, so that the owner
//!   cannot give itself more through `WRITE_DAC`), only what the rest of
//!   Cineris asks of another process of its own: to wait for it
//!   (`SYNCHRONIZE`), to ask the limited questions (when it was made, its
//!   parent, its program, its command line, whether it is in a job:
//!   `PROCESS_QUERY_LIMITED_INFORMATION`), and, for a process, to end it
//!   (`PROCESS_TERMINATE`, which a person's task manager needs). Reading or
//!   writing its memory, making a thread in it, copying its handles,
//!   changing it, suspending it, and changing who may do any of that are
//!   denied by being left out. A thread's list leaves out its context,
//!   suspending it and queueing work to it. SYSTEM and the Administrators
//!   group keep every right, which an elevated process can take with
//!   `SeDebugPrivilege` anyway. The threads that run when it is called are
//!   given the list then; a thread made after it gives itself the list as
//!   it starts, before its own code runs (a callback the loader runs in
//!   every new thread). **And its standard handles are kept from what it
//!   starts:** a shell starts Cineris with inheritable console handles (or,
//!   for `serve --stdio`, the protocol's pipes), and every process made with
//!   handles inherited would have them at the same values, a console's
//!   input among them, to read or write keys through. They are made not
//!   inheritable; a program given the standard handles on purpose (the
//!   external editor, through the standard library's `Stdio::inherit`) is
//!   given copies made for it.
//!
//! **What it leaves** (`docs/security.md`, section 3):
//!
//! - root, or a process holding `SeDebugPrivilege`;
//! - what was opened before it ran: a tracer attached, or a
//!   `/proc/<pid>/mem` opened, in the moment between the process's start and
//!   this call, and on Windows every handle opened before it, above all the
//!   full handle the program that started Cineris holds from
//!   `CreateProcess`, which another process of the user can copy out of
//!   that program; [`traced`] tells a tracer, not an open file or handle;
//! - on Windows, a thread made without the loader's notice, or opened in the
//!   moment between its making and its first instruction.

use std::io;

/// Closes this process to the user's other programs: called first thing
/// in every process of Cineris's binary.
///
/// # Errors
///
/// When the system refuses: on Linux the call cannot fail; on Windows the
/// list could not be made or set on the process. A thread whose list could
/// not be set is not an error: it is left as it was.
pub fn harden() -> io::Result<()> {
    sys::harden()
}

/// Whether this process is closed by [`harden`]: on Linux not dumpable; on
/// Windows its process's list is the one [`harden`] sets.
#[must_use]
pub fn hardened() -> bool {
    sys::hardened()
}

/// Whether a debugger or a tracer is attached to this process now: on
/// Linux a `TracerPid` other than 0 in the status of any of its threads
/// (`/proc/self/task/<tid>/status`: a tracer attaches to a thread, and one
/// attached to any thread reads the whole process; a status that cannot be
/// read fails closed); on Windows a debug port, as
/// the system keeps it (what `CheckRemoteDebuggerPresent` asks). A tracer
/// that attached before [`harden`] ran stays attached, and could read what
/// a prompt is given: the password prompt refuses where this is `true`.
///
/// With this crate's `test-hooks` feature, which only dev-dependencies turn
/// on (`cargo xtask rules`) and no shipped build has (`cargo xtask size`
/// fails a shipped binary that holds [`TEST_TRACED`]'s name), it is also
/// `true` while the file [`TEST_TRACED`] names exists: for the binary's
/// system tests of every prompt that asks it, which a runner cannot
/// otherwise attach a debugger to at the moment wanted. It can only say
/// `true`: it never hides a real tracer.
#[must_use]
pub fn traced() -> bool {
    test_traced() || sys::traced()
}

/// The variable a test sets on the binary it starts to name a file that
/// makes [`traced`] say `true` while it exists. Read only with the
/// `test-hooks` feature, so that only a build with it holds the name: a
/// constant reaches a binary only where code reads it.
pub const TEST_TRACED: &str = "CINERIS_TEST_TRACED";

/// Whether the file [`TEST_TRACED`] names exists: never without the
/// `test-hooks` feature.
#[cfg(not(feature = "test-hooks"))]
#[inline(always)]
const fn test_traced() -> bool {
    false
}

/// [`test_traced`] with the `test-hooks` feature.
#[cfg(feature = "test-hooks")]
#[allow(
    clippy::disallowed_methods,
    reason = "a test's file looked for, before a password is asked, in a build for tests only"
)]
fn test_traced() -> bool {
    std::env::var_os(TEST_TRACED)
        .filter(|file| !file.is_empty())
        .is_some_and(|file| std::path::Path::new(&file).exists())
}

#[cfg(test)]
mod tests {
    /// The test variable's name is the one `cargo xtask size` refuses in a
    /// shipped binary (`xtask/src/shipped.rs`, `TEST_HOOK_MARKS`), which
    /// cannot reach this crate's constant.
    #[test]
    fn the_test_variables_name_is_the_one_a_shipped_binary_may_not_hold() {
        assert_eq!(super::TEST_TRACED, "CINERIS_TEST_TRACED");
    }
}

#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "decision 0030: whether a process may be dumped is only reachable through `prctl`; \
              cineris-sandbox is where such calls live"
)]
mod sys {
    use std::io;

    /// An argument of `prctl` that says nothing, as wide as the kernel
    /// reads it.
    const NONE: libc::c_ulong = 0;

    pub(super) fn harden() -> io::Result<()> {
        // SAFETY: `prctl` with an option and integer arguments only, each
        // as wide as the kernel reads; it touches no memory of this process.
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, NONE, NONE, NONE, NONE) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn hardened() -> bool {
        // SAFETY: as above; this option only reads.
        unsafe { libc::prctl(libc::PR_GET_DUMPABLE, NONE, NONE, NONE, NONE) == 0 }
    }

    /// Whether any thread of this process is traced: a tracer attaches to
    /// one thread, not the process, and one attached to any thread reads
    /// and writes the whole process's memory through it; the process's own
    /// `status` names only its first thread's (the full review of the signed
    /// approvals: a thread made by the test harness, traced alone, was not
    /// seen). A thread that ended while the list was read is passed over;
    /// any other list or status that cannot be read fails closed.
    #[allow(
        clippy::disallowed_methods,
        reason = "small reads of this process's own threads' status, before a password is asked"
    )]
    pub(super) fn traced() -> bool {
        let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
            return true;
        };
        let mut seen = false;
        for task in tasks {
            let Ok(task) = task else {
                return true;
            };
            match std::fs::read_to_string(task.path().join("status")) {
                Ok(status) => {
                    seen = true;
                    if tracer_in(&status) != Some(0) {
                        return true;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return true,
            }
        }
        !seen
    }

    /// The `TracerPid` a process's status names: `None` where it names
    /// none, or not as a number.
    pub(super) fn tracer_in(status: &str) -> Option<u32> {
        status
            .lines()
            .find_map(|line| line.strip_prefix("TracerPid:"))
            .and_then(|field| field.trim().parse().ok())
    }

    #[cfg(test)]
    mod tests {
        /// The tracer is read from its line, and a status without one, or
        /// with one that is not a number, names none.
        #[test]
        fn the_tracer_is_read_from_the_status() {
            let status = "Name:\tcineris\nState:\tS (sleeping)\nTracerPid:\t4321\nUid:\t1000\n";
            assert_eq!(super::tracer_in(status), Some(4321));
            assert_eq!(super::tracer_in("TracerPid:\t0\n"), Some(0));
            assert_eq!(super::tracer_in("Name:\tcineris\n"), None);
            assert_eq!(super::tracer_in("TracerPid:\tx\n"), None);
        }

        /// This test's own process is not traced (nextest runs it
        /// untraced).
        #[test]
        fn a_test_is_not_traced() {
            assert!(!super::traced());
        }
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
mod sys {
    //! Not a target: nothing is closed, and nothing is said to be.

    use std::io;

    pub(super) fn harden() -> io::Result<()> {
        Ok(())
    }

    pub(super) fn hardened() -> bool {
        false
    }

    pub(super) fn traced() -> bool {
        false
    }
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "decision 0030: a process's and its threads' access lists, and its debug port, are \
              only reachable through the Windows API; cineris-sandbox is where such calls live"
)]
mod sys {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr;
    use std::sync::atomic::{AtomicPtr, Ordering};

    use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessDebugPort};
    use windows_sys::Win32::Foundation::{
        FALSE, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAce, CreateWellKnownSid,
        DACL_SECURITY_INFORMATION, GetLengthSid, GetTokenInformation, InitializeAcl,
        InitializeSecurityDescriptor, PSID, SECURITY_DESCRIPTOR, SetKernelObjectSecurity,
        SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser, WELL_KNOWN_SID_TYPE,
        WinBuiltinAdministratorsSid, WinCreatorOwnerRightsSid, WinLocalSystemSid,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetCurrentThread, OpenProcessToken, OpenThread,
        PROCESS_ALL_ACCESS, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        THREAD_ALL_ACCESS, THREAD_QUERY_LIMITED_INFORMATION,
    };

    /// The right to wait on an object (`winnt.h`'s `SYNCHRONIZE`).
    const SYNCHRONIZE: u32 = 0x0010_0000;

    /// The right to change an object's access list (`winnt.h`'s
    /// `WRITE_DAC`).
    const WRITE_DAC: u32 = 0x0004_0000;

    /// What the user and `OWNER RIGHTS` keep of the process: to wait for
    /// it, ask it the limited questions, and end it.
    pub(super) const PROCESS_KEPT: u32 =
        SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE;

    /// What they keep of each thread: to wait for it and ask it the limited
    /// questions.
    pub(super) const THREAD_KEPT: u32 = SYNCHRONIZE | THREAD_QUERY_LIMITED_INFORMATION;

    /// `winnt.h`'s `SECURITY_DESCRIPTOR_REVISION`.
    const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

    /// The loader's reason for a callback in a thread that is starting
    /// (`winnt.h`'s `DLL_THREAD_ATTACH`).
    const DLL_THREAD_ATTACH: u32 = 2;

    /// The threads' descriptor, once [`harden`] has made it: what a thread
    /// that starts after it gives itself. Made once and never freed, as it
    /// is read until the process ends.
    static THREAD_DESCRIPTOR: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

    /// What the loader runs in every thread as it starts and ends, before
    /// the thread's own code (a TLS callback, in the section the C
    /// runtime's list of them is made from, as the standard library's own
    /// is). Once the process is closed, a thread that starts gives itself
    /// the threads' list; before, it does nothing.
    #[used]
    #[unsafe(link_section = ".CRT$XLY")]
    static ON_THREAD: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) = on_thread;

    /// The callback: one call that sets the list on the starting thread,
    /// through its own pseudo-handle, which has every right. It loads no
    /// library and takes no lock but the system's, as code that runs under
    /// the loader's lock must not.
    unsafe extern "system" fn on_thread(_module: *mut c_void, reason: u32, _reserved: *mut c_void) {
        if reason != DLL_THREAD_ATTACH {
            return;
        }
        let descriptor = THREAD_DESCRIPTOR.load(Ordering::Acquire);
        if descriptor.is_null() {
            return;
        }
        // SAFETY: the pseudo-handle is always valid for the calling
        // thread, and `descriptor` points at a descriptor and its list that
        // were made once and are never freed.
        unsafe {
            SetKernelObjectSecurity(GetCurrentThread(), DACL_SECURITY_INFORMATION, descriptor)
        };
    }

    pub(super) fn harden() -> io::Result<()> {
        // The callback is named, so that nothing drops it from the build:
        // the linker keeps what is read.
        // SAFETY: a read of a static that is always initialised.
        let _ = unsafe { ptr::from_ref(&ON_THREAD).read_volatile() };
        keep_standard_handles();
        let user = User::read()?;
        let process = descriptor(user.sid(), PROCESS_KEPT, PROCESS_ALL_ACCESS)?;
        // SAFETY: the pseudo-handle is always valid, with every right, and
        // the descriptor and its list live through the call.
        if unsafe {
            SetKernelObjectSecurity(
                GetCurrentProcess(),
                DACL_SECURITY_INFORMATION,
                process.pointer(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let threads = descriptor(user.sid(), THREAD_KEPT, THREAD_ALL_ACCESS)?;
        let pointer = threads.leak();
        // From here every thread that starts closes itself; then each that
        // runs already is closed, so that none falls between the two.
        THREAD_DESCRIPTOR.store(pointer, Ordering::Release);
        close_running_threads(pointer);
        Ok(())
    }

    /// Makes this process's standard handles not inheritable, so that a
    /// process it starts with handles inherited does not have them. One
    /// that is not there, or not a handle that can be inherited, is left.
    fn keep_standard_handles() {
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: reads nothing but its argument.
            let handle = unsafe { GetStdHandle(which) };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                continue;
            }
            // SAFETY: `handle` is this process's own standard handle, open
            // for its life; the call changes one of its flags.
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        }
    }

    /// Gives every thread of this process that runs now the threads' list.
    /// A thread that cannot be opened (it has just ended) is passed over.
    fn close_running_threads(descriptor: *mut c_void) {
        // SAFETY: the call takes two integers. For a list of threads the
        // second is not used.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return;
        }
        // SAFETY: `snapshot` was just made and nothing else owns it.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
        // SAFETY: plain values.
        let own = unsafe { GetCurrentProcessId() };
        // SAFETY: the structure is plain data, for which all zeroes is valid.
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = u32::try_from(size_of::<THREADENTRY32>()).unwrap_or(0);
        // SAFETY: `snapshot` is open, and `entry` is writable and says how
        // large it is.
        let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &raw mut entry) } != 0;
        while more {
            if entry.th32OwnerProcessID == own {
                // SAFETY: plain values: the one right needed, no
                // inheritance, a thread id.
                let thread = unsafe { OpenThread(WRITE_DAC, FALSE, entry.th32ThreadID) };
                if !thread.is_null() {
                    // SAFETY: `thread` was just opened and nothing else
                    // owns it.
                    let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
                    // SAFETY: `thread` is open with the right to change its
                    // list; the descriptor is never freed.
                    unsafe {
                        SetKernelObjectSecurity(
                            thread.as_raw_handle(),
                            DACL_SECURITY_INFORMATION,
                            descriptor,
                        )
                    };
                }
            }
            // SAFETY: as for `Thread32First`.
            more = unsafe { Thread32Next(snapshot.as_raw_handle(), &raw mut entry) } != 0;
        }
    }

    pub(super) fn hardened() -> bool {
        !THREAD_DESCRIPTOR.load(Ordering::Acquire).is_null()
    }

    pub(super) fn traced() -> bool {
        let mut port: usize = 0;
        // SAFETY: the pseudo-handle is always valid with every right; this
        // class writes one pointer-sized number, which `port` is.
        let status = unsafe {
            NtQueryInformationProcess(
                GetCurrentProcess(),
                ProcessDebugPort,
                (&raw mut port).cast(),
                u32::try_from(size_of::<usize>()).unwrap_or(0),
                ptr::null_mut(),
            )
        };
        // A status that cannot be asked fails closed.
        status < 0 || port != 0
    }

    /// This process's user, as its token gives it: a `TOKEN_USER` at the
    /// buffer's start, whose SID points into the same buffer.
    struct User {
        buffer: Box<[u64; 32]>,
    }

    impl User {
        fn read() -> io::Result<Self> {
            let mut token: HANDLE = ptr::null_mut();
            // SAFETY: the pseudo-handle is always valid, and `token` is a
            // live handle's place.
            if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `token` was just opened and nothing else owns it.
            let token = unsafe { OwnedHandle::from_raw_handle(token) };
            let mut buffer = Box::new([0_u64; 32]);
            let mut needed = 0_u32;
            // SAFETY: `token` is open to query; the buffer is as large as
            // said and aligned for `TOKEN_USER`, and stays where it is (it
            // is boxed) for as long as the SID in it is used.
            let asked = unsafe {
                GetTokenInformation(
                    token.as_raw_handle(),
                    TokenUser,
                    buffer.as_mut_ptr().cast(),
                    u32::try_from(size_of::<[u64; 32]>()).unwrap_or(0),
                    &raw mut needed,
                )
            };
            if asked == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { buffer })
        }

        fn sid(&self) -> PSID {
            // SAFETY: `read` wrote a `TOKEN_USER` at the buffer's start.
            unsafe { (*self.buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
        }
    }

    /// A well-known SID, in room for the largest.
    fn well_known(kind: WELL_KNOWN_SID_TYPE) -> io::Result<[u32; 17]> {
        let mut sid = [0_u32; 17];
        let mut size = u32::try_from(size_of_val(&sid)).unwrap_or(0);
        // SAFETY: a buffer as large as said, for a SID that needs no domain.
        if unsafe {
            CreateWellKnownSid(
                kind,
                ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &raw mut size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(sid)
    }

    /// A descriptor and the list it points at, which live as long as it.
    pub(super) struct Descriptor {
        descriptor: Box<SECURITY_DESCRIPTOR>,
        _list: Vec<u32>,
    }

    impl Descriptor {
        pub(super) fn pointer(&self) -> *mut c_void {
            ptr::from_ref(&*self.descriptor).cast_mut().cast()
        }

        /// The descriptor, kept for the process's life.
        fn leak(self) -> *mut c_void {
            let pointer = self.pointer();
            std::mem::forget(self);
            pointer
        }
    }

    /// The list `kept` for the user and `OWNER RIGHTS`, and `all` for
    /// SYSTEM and the Administrators group, with nothing else: the same as
    /// the text `D:(A;;kept;;;<user>)(A;;kept;;;OW)(A;;all;;;SY)(A;;all;;;
    /// BA)`, built from its parts, as `session`'s job's is.
    pub(super) fn descriptor(user: PSID, kept: u32, all: u32) -> io::Result<Descriptor> {
        let failed = |asked: i32| {
            if asked == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        let mut owner = well_known(WinCreatorOwnerRightsSid)?;
        let mut system = well_known(WinLocalSystemSid)?;
        let mut administrators = well_known(WinBuiltinAdministratorsSid)?;
        let entries: [(PSID, u32); 4] = [
            (user, kept),
            (owner.as_mut_ptr().cast(), kept),
            (system.as_mut_ptr().cast(), all),
            (administrators.as_mut_ptr().cast(), all),
        ];
        let entry = |sid: PSID| {
            // SAFETY: a valid SID, given or made above.
            let sid = usize::try_from(unsafe { GetLengthSid(sid) }).unwrap_or(0);
            size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + sid
        };
        let length = size_of::<ACL>() + entries.iter().map(|(sid, _)| entry(*sid)).sum::<usize>();
        let mut list = vec![0_u32; length.div_ceil(4)];
        let length = u32::try_from(list.len() * 4).unwrap_or(0);
        let list_pointer: *mut ACL = list.as_mut_ptr().cast();
        // SAFETY: the list's memory is as large as said and aligned for an
        // `ACL`; each SID is valid through the calls, which copy it.
        unsafe {
            failed(InitializeAcl(list_pointer, length, ACL_REVISION))?;
            for (sid, rights) in entries {
                failed(AddAccessAllowedAce(list_pointer, ACL_REVISION, rights, sid))?;
            }
        }
        // SAFETY: plain data, for which all zeroes is valid.
        let mut descriptor: Box<SECURITY_DESCRIPTOR> = Box::new(unsafe { std::mem::zeroed() });
        let pointer: *mut c_void = (&raw mut *descriptor).cast();
        // SAFETY: `pointer` is a live descriptor that points at the list;
        // both are kept in what is returned, and neither moves (both are
        // on the heap).
        unsafe {
            failed(InitializeSecurityDescriptor(
                pointer,
                SECURITY_DESCRIPTOR_REVISION,
            ))?;
            failed(SetSecurityDescriptorDacl(pointer, 1, list_pointer, 0))?;
        }
        Ok(Descriptor {
            descriptor,
            _list: list,
        })
    }
}
