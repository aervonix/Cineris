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
    assert!(!handle.is_null(), "{name:?}: {}", io::Error::last_os_error());
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
    for name in [None, Some(r"Local\Cineris.SessionCommands"), Some(old.as_str())] {
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
    unsafe { IsProcessInJob(GetCurrentProcess(), squatted.as_raw_handle(), &raw mut inside) };
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
            assert_eq!(error.raw_os_error(), i32::try_from(ERROR_ACCESS_DENIED).ok());
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
    // SAFETY: as in `under_a_session`.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    assert_ne!(snapshot, INVALID_HANDLE_VALUE);
    // SAFETY: just made, owned by nothing else.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    // SAFETY: plain data.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap();
    let mut line = Vec::new();
    // SAFETY: as in `under_a_session`.
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    while more {
        let end = entry.szExeFile.iter().position(|unit| *unit == 0).unwrap();
        line.push((
            entry.th32ProcessID,
            entry.th32ParentProcessID,
            String::from_utf16_lossy(&entry.szExeFile[..end]).to_lowercase(),
        ));
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    }
    let named = |pid: u32| line.iter().find(|(at, _, _)| *at == pid).map(|(_, _, name)| name);
    assert_eq!(
        named(scheduler).map(String::as_str),
        Some("svchost.exe"),
        "{scheduler}"
    );
    // The line from this test up, said when it fails.
    let mut parents = Vec::new();
    let mut at = std::process::id();
    while let Some((_, parent, name)) = line.iter().find(|(pid, _, _)| *pid == at) {
        parents.push(format!("{at} {name}"));
        if *parent == at || parents.len() > 16 {
            break;
        }
        at = *parent;
    }
    assert_eq!(under_a_session(), Under::Nothing, "{parents:?}");
}
