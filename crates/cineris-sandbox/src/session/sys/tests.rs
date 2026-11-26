use super::*;
use crate::session::JOB_PREFIX;

use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows_sys::Win32::System::JobObjects::SetInformationJobObject;

/// The right to change a job's limits (`winnt.h`'s
/// `JOB_OBJECT_SET_ATTRIBUTES`).
const JOB_OBJECT_SET_ATTRIBUTES: u32 = 0x0010;

/// A job of the given security, named or not.
fn job(name: Option<&str>, security: Option<&OwnerOnly>) -> OwnedHandle {
    let wide_name = name.map(wide);
    let pointer = wide_name.as_ref().map_or(ptr::null(), |name| name.as_ptr());
    let attributes = security.map(OwnerOnly::attributes);
    let attributes = attributes
        .as_ref()
        .map_or(ptr::null(), |attributes| &raw const *attributes);
    // SAFETY: attributes that live through the call, or none, and a name
    // that ends in a nul, or none.
    let handle = unsafe { CreateJobObjectW(attributes, pointer) };
    assert!(
        !handle.is_null(),
        "{name:?}: {}",
        io::Error::last_os_error()
    );
    // SAFETY: `handle` was just made and nothing else owns it.
    unsafe { OwnedHandle::from_raw_handle(handle) }
}

/// This process's own job's name, as `mark` makes it.
fn own_name() -> String {
    // SAFETY: the pseudo-handle is always valid.
    let made = made_at(unsafe { GetCurrentProcess() }).unwrap();
    job_name(std::process::id(), made)
}

fn join(job: &OwnedHandle) {
    // SAFETY: as in `mark`.
    let joined = unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) };
    assert_ne!(joined, 0, "{}", io::Error::last_os_error());
}

/// A job of no harness's, unnamed or named, the old names among them, is
/// no session's: a process in it, as a terminal's, an editor's or a CI
/// runner's children are, is under none. Once it runs sessions itself, it
/// is in its own job.
#[test]
fn sys_a_job_of_no_harnesss_is_no_sessions() {
    let old = format!("{JOB_PREFIX}{}", std::process::id());
    for name in [
        None,
        Some(r"Local\Cineris.SessionCommands"),
        Some(old.as_str()),
    ] {
        let made = job(name, None);
        join(&made);
        assert_eq!(under_a_session(), Under::Nothing, "{name:?}");
        std::mem::forget(made);
    }
    mark().unwrap();
    assert!(matches!(under_a_session(), Under::Session(_)));
}

/// A job of this process's own name made before it marks itself is not
/// joined: the mark fails, and is not taken (the full review of the third
/// part: a job made ahead could let what it holds leave).
#[test]
fn sys_a_job_made_ahead_of_the_harness_is_not_joined() {
    let squatted = job(Some(&own_name()), None);
    let error = mark().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert!(JOB.get().is_none());
    let mut inside = FALSE;
    // SAFETY: `squatted` is open, the pseudo-handle has every right, and
    // `inside` is a live `BOOL`.
    unsafe {
        IsProcessInJob(
            GetCurrentProcess(),
            squatted.as_raw_handle(),
            &raw mut inside,
        )
    };
    assert_eq!(inside, FALSE);
}

/// The harness's job may be opened by its name only to ask about it: not
/// to change its limits, by its user or its owner; and it lets nothing in
/// it leave.
#[test]
fn sys_the_harnesss_job_is_open_by_its_name_only_to_ask() {
    mark().unwrap();
    let name = wide(&own_name());
    for (rights, opens) in [(JOB_OBJECT_QUERY, true), (JOB_OBJECT_SET_ATTRIBUTES, false)] {
        // SAFETY: plain values, and a name that ends in a nul.
        let handle = unsafe { OpenJobObjectW(rights, FALSE, name.as_ptr()) };
        let error = io::Error::last_os_error();
        assert_eq!(!handle.is_null(), opens, "{rights:#x}: {error}");
        if handle.is_null() {
            assert_eq!(
                error.raw_os_error(),
                i32::try_from(ERROR_ACCESS_DENIED).ok()
            );
        } else {
            // SAFETY: just opened, owned by nothing else.
            drop(unsafe { OwnedHandle::from_raw_handle(handle) });
        }
    }
    assert!(!lets_processes_leave(JOB.get().unwrap()).unwrap());
}

/// A job that lets the processes in it leave, quietly or when asked, is
/// told apart from one that does not.
#[test]
fn sys_a_job_that_lets_its_processes_leave_is_told() {
    let plain = job(None, None);
    assert!(!lets_processes_leave(&plain).unwrap());
    for flag in [
        JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
    ] {
        let leaving = job(None, None);
        // SAFETY: the structure is plain data, for which all zeroes is
        // valid.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = flag;
        // SAFETY: `leaving` is open with every right, and the structure is
        // as large as said.
        let set = unsafe {
            SetInformationJobObject(
                leaving.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                u32::try_from(std::mem::size_of_val(&limits)).unwrap(),
            )
        };
        assert_ne!(set, 0, "{}", io::Error::last_os_error());
        assert!(lets_processes_leave(&leaving).unwrap(), "{flag:#x}");
    }
}

/// A job of a harness's name that this process may not open is not known,
/// which refuses, and is not taken for no job (the full review of the third
/// part).
#[test]
fn sys_a_harnesss_job_that_cannot_be_opened_is_not_known() {
    // No one may open it: a descriptor with an empty list of who may.
    let denied = OwnerOnly::from_sddl("D:P").unwrap();
    let made = job(Some(&own_name()), Some(&denied));
    assert!(
        matches!(under_a_session(), Under::Unknown(_)),
        "{:?}",
        under_a_session()
    );
    drop(made);
}

/// The task scheduler's service is found in the service host that runs
/// it, and a test's own line of parents holds no service that starts what
/// a command asks.
#[test]
fn sys_the_task_schedulers_host_is_found_and_a_tests_line_is_no_services() {
    let scheduler = scheduler().expect("the task scheduler's service could be asked");
    assert_ne!(scheduler, 0, "the task scheduler runs");
    let line = processes().unwrap();
    let named = |pid: u32| {
        line.iter()
            .find(|process| process.pid == pid)
            .map(|process| process.name.as_str())
    };
    assert_eq!(named(scheduler), Some("svchost.exe"), "{scheduler}");
    // The line from this test up, said when it fails.
    let mut parents = Vec::new();
    let mut at = std::process::id();
    while let Some(process) = line.iter().find(|process| process.pid == at) {
        parents.push(format!("{at} {}", process.name));
        if process.parent == at || parents.len() > 16 {
            break;
        }
        at = process.parent;
    }
    assert_eq!(under_a_session(), Under::Nothing, "{parents:?}");
}

/// The system's list of processes, read in one call, gives each process
/// when it was made as opening it and asking does: the count a harness
/// names its job by, so that a job is found by the list's count alone. And
/// it gives this process under its own name, with the test runner above
/// it, and the system's first processes.
#[test]
fn sys_the_list_of_processes_gives_each_when_it_was_made() {
    let listed = processes().unwrap();
    assert!(listed.len() > 2, "{}", listed.len());
    let own = listed
        .iter()
        .find(|process| process.pid == std::process::id())
        .expect("this process is listed");
    // SAFETY: the pseudo-handle is always valid.
    let made = made_at(unsafe { GetCurrentProcess() }).unwrap();
    assert_eq!(own.made, made);
    let exe = std::env::current_exe().unwrap();
    let exe = exe.file_name().unwrap().to_string_lossy().to_lowercase();
    assert_eq!(own.name, exe);
    assert!(
        listed.iter().any(|process| process.pid == own.parent),
        "its parent, {}, is listed",
        own.parent
    );
    let mut compared = 0;
    for process in &listed {
        if let Opened::Made(made) = opened(process.pid) {
            assert_eq!(process.made, made, "{} {}", process.pid, process.name);
            compared += 1;
        }
    }
    assert!(compared > 1, "{compared}");
}

/// A nul-terminated wide text a call allocated, read and freed.
fn taken(text: *mut u16) -> String {
    // SAFETY: a nul-terminated text a call made.
    let length = (0..)
        .take_while(|at| unsafe { *text.add(*at) } != 0)
        .count();
    // SAFETY: as above, `length` characters before the nul.
    let said = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
    // SAFETY: allocated by the call, to be freed with `LocalFree`, once.
    unsafe { LocalFree(text.cast()) };
    said
}

/// A descriptor's list of who may do what, as text.
fn rendered(security: &OwnerOnly) -> String {
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
    let mut text = ptr::null_mut();
    // SAFETY: a live descriptor, and a live pointer for the text, which the
    // call allocates.
    let said = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            security.descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &raw mut text,
            ptr::null_mut(),
        )
    };
    assert_ne!(said, 0, "{}", io::Error::last_os_error());
    taken(text)
}

/// The harness's job's security, built from its parts, is the one its text
/// makes, `D:P(A;;BY_NAME;;;<user>)(A;;BY_NAME;;;OW)`: its user and its
/// owner may ask, nobody else may do anything, and nothing is inherited.
#[test]
fn sys_the_jobs_security_is_its_text() {
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    let mut user = [0_u64; 32];
    user_into(&mut user).unwrap();
    // SAFETY: a `TOKEN_USER` at the buffer's start, its SID within it.
    let sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text = ptr::null_mut();
    // SAFETY: a valid SID, and a live pointer for the text the call makes.
    assert_ne!(unsafe { ConvertSidToStringSidW(sid, &raw mut text) }, 0);
    let sid = taken(text);
    let from_text = OwnerOnly::from_sddl(&format!(
        "D:P(A;;0x{BY_NAME:x};;;{sid})(A;;0x{BY_NAME:x};;;OW)"
    ))
    .unwrap();
    let built = OwnerOnly::new().unwrap();
    let said = rendered(&built);
    assert_eq!(said, rendered(&from_text));
    assert!(said.starts_with("D:P("), "{said}");
}
