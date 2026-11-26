use super::*;
use crate::session::JOB_PREFIX;
use crate::session::switches::{self, LISTING, OWN_BINARY};

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

/// This test binary, run again in a process of its own for the test `test`
/// alone, with `arguments` after its name (a role, which names no test, and
/// what the role is given) and the variables `variables` beside those it
/// inherits: its process's id, its exit code, and what it wrote. (Its own
/// `cfg(test)` says to the repo rule over the product's process starts
/// what this file is: the rule tells a test module's file by its path, and
/// this one's `mod tests;` is inside `sys`, which it does not follow.)
#[cfg(test)]
#[allow(
    clippy::disallowed_types,
    reason = "a test runs itself again, in a process of its own"
)]
fn again(test: &str, arguments: &[&str], variables: &[(&str, &str)]) -> (u32, Option<i32>, String) {
    use std::process::Stdio;
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .args(arguments)
        .envs(variables.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let ran = child.wait_with_output().unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    (pid, ran.status.code(), said)
}

/// This process's own instance, as its job is named for it.
fn own_instance() -> (u32, u64) {
    // SAFETY: the pseudo-handle is always valid.
    let made = made_at(unsafe { GetCurrentProcess() }).unwrap();
    (std::process::id(), made)
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
    let under = under_a_session();
    assert!(matches!(under, Under::Session(_)), "{under:?}");
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

/// This test's own name, by which it runs itself again.
const DENIED_TEST: &str =
    "session::sys::tests::sys_a_harnesss_job_that_cannot_be_opened_is_not_known";

/// A job of a harness's name that this process may not open is not known,
/// which refuses, and is not taken for no job (the full review of the third
/// part), when the process it is named for is Cineris: here this test's own
/// process, run again with the test switch that makes this test binary
/// count as Cineris's (`CINERIS_TEST_SESSION_OWN_BINARY`), in that process
/// alone, so that no other test's check takes the job for a harness's.
#[test]
fn sys_a_harnesss_job_that_cannot_be_opened_is_not_known() {
    if switches::own_binary() {
        let (pid, made) = own_instance();
        // No one may open it: a descriptor with an empty list of who may.
        let denied = OwnerOnly::from_sddl("D:P").unwrap();
        let planted = job(Some(&own_name()), Some(&denied));
        let named = named_for(pid, made, &OnceCell::new());
        let direct = in_the_job_of(pid, made, &OnceCell::new());
        let under = under_a_session();
        drop(planted);
        assert_eq!(named, NamedFor::Cineris);
        assert!(
            matches!(&direct, Job::Unknown(what) if what.contains(&format!("process {pid}'s"))),
            "{direct:?}"
        );
        assert!(matches!(under, Under::Unknown(_)), "{under:?}");
        println!("checked as Cineris's: {under:?}");
        return;
    }
    let (_, code, said) = again(DENIED_TEST, &[], &[(OWN_BINARY, "1")]);
    assert_eq!(code, Some(0), "{said}");
    assert!(said.contains("checked as Cineris's: Unknown("), "{said}");
}

/// A job of a harness's name that cannot be opened, named for a process
/// that is not Cineris (this test's own, whose binary is no harness's
/// without the test switch), is passed over, and the check says what it
/// says without it (step 28's third pull request: such a job, made by one
/// test for its own process, made the checks of the tests running beside
/// it refuse, and one made for any running process would make the user's
/// own approvals refuse).
#[test]
fn sys_a_job_that_cannot_be_opened_named_for_another_program_is_passed_over() {
    assert!(!switches::own_binary(), "run without the test switch");
    let (pid, made) = own_instance();
    let before = under_a_session();
    let denied = OwnerOnly::from_sddl("D:P").unwrap();
    let planted = job(Some(&own_name()), Some(&denied));
    let named = named_for(pid, made, &OnceCell::new());
    let direct = in_the_job_of(pid, made, &OnceCell::new());
    let after = under_a_session();
    drop(planted);
    assert_eq!(named, NamedFor::Other);
    assert_eq!(direct, Job::None);
    assert_eq!(after, before);
    assert_eq!(after, Under::Nothing);
}

/// This test's own name, by which it runs itself again.
const AFTER_MARK_TEST: &str =
    "session::sys::tests::sys_a_command_cannot_make_its_harnesss_job_unopenable";

/// The role, in that test, of a command the harness started.
const COMMAND: &str = "cin-session-command";

/// The rights to change who may open an object, and who owns it
/// (`winnt.h`'s `WRITE_DAC` and `WRITE_OWNER`).
const WRITE_DAC_OR_OWNER: u32 = 0x0004_0000 | 0x0008_0000;

/// The rights `handle` was opened with, as the system keeps them.
fn granted(handle: HANDLE) -> u32 {
    use windows_sys::Wdk::Foundation::{NtQueryObject, ObjectBasicInformation};
    use windows_sys::Win32::System::WindowsProgramming::PUBLIC_OBJECT_BASIC_INFORMATION;
    // SAFETY: the structure is plain data, for which all zeroes is valid.
    let mut basic: PUBLIC_OBJECT_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a live handle, and a live structure as large as said.
    let status = unsafe {
        NtQueryObject(
            handle,
            ObjectBasicInformation,
            (&raw mut basic).cast(),
            u32::try_from(std::mem::size_of_val(&basic)).unwrap(),
            ptr::null_mut(),
        )
    };
    assert!(status >= 0, "{status:#010x}");
    basic.GrantedAccess
}

/// What a command of the harness `harness` can try on its job with its
/// user's rights: take the harness's own handle of it, `handle` (a number a
/// command can find by trying the harness's handles, small multiples of
/// four), into its own process with the rights the harness has
/// (`PROCESS_DUP_HANDLE`, which the user has on its own processes), and
/// give the job an empty list of who may open it. The rights the handle
/// came with, and what the change did.
fn deny_by_its_handle(harness: u32, handle: usize) -> (u32, io::Result<()>) {
    use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle};
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, SetKernelObjectSecurity};
    use windows_sys::Win32::System::Threading::PROCESS_DUP_HANDLE;
    // SAFETY: plain values.
    let process = unsafe { OpenProcess(PROCESS_DUP_HANDLE, FALSE, harness) };
    assert!(!process.is_null(), "{}", io::Error::last_os_error());
    // SAFETY: just opened, owned by nothing else.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    let mut taken: HANDLE = ptr::null_mut();
    // SAFETY: `process` is open to take its handles; the handle's number is
    // the harness's, which it holds for its life; `taken` is a live place.
    let duplicated = unsafe {
        DuplicateHandle(
            process.as_raw_handle(),
            ptr::without_provenance_mut(handle),
            GetCurrentProcess(),
            &raw mut taken,
            0,
            FALSE,
            DUPLICATE_SAME_ACCESS,
        )
    };
    assert_ne!(duplicated, 0, "{}", io::Error::last_os_error());
    // SAFETY: just made, owned by nothing else.
    let taken = unsafe { OwnedHandle::from_raw_handle(taken) };
    let rights = granted(taken.as_raw_handle());
    let denied = OwnerOnly::from_sddl("D:P").unwrap();
    // SAFETY: `taken` is open, and the descriptor lives through the call.
    let set = unsafe {
        SetKernelObjectSecurity(
            taken.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            denied.descriptor,
        )
    };
    let changed = if set == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    };
    (rights, changed)
}

/// A command cannot make its harness's job unopenable after the mark: the
/// harness's own handle of its job, which a command can take into its own
/// process, holds every right of a job but those to change who may open it
/// and who owns it (`WRITE_DAC`, `WRITE_OWNER`: the job's own list, whose
/// owner's entry gives only the right to ask, keeps them from its maker
/// too), so the command's change is refused, the job stays open to ask, and
/// the command's check finds itself in it. (A job of a harness's name that
/// cannot be opened, named for a process of Cineris, is not known, which
/// refuses: `sys_a_harnesss_job_that_cannot_be_opened_is_not_known`.)
#[test]
fn sys_a_command_cannot_make_its_harnesss_job_unopenable() {
    let arguments: Vec<String> = std::env::args().collect();
    if let Some(at) = arguments.iter().position(|argument| argument == COMMAND) {
        let harness: u32 = arguments[at + 1].parse().unwrap();
        let handle: usize = arguments[at + 2].parse().unwrap();
        let (rights, changed) = deny_by_its_handle(harness, handle);
        let under = under_a_session();
        println!("its command took {rights:#x}, changed {changed:?}, and checked {under:?}");
        assert_eq!(rights & WRITE_DAC_OR_OWNER, 0, "{rights:#x}");
        let refused = changed.unwrap_err();
        assert_eq!(
            refused.raw_os_error(),
            i32::try_from(ERROR_ACCESS_DENIED).ok(),
            "{refused}"
        );
        let held = format!("it is in the job of process {harness}'s sessions' commands");
        assert_eq!(under, Under::Session(held));
        return;
    }
    mark().unwrap();
    let held = JOB.get().unwrap().as_raw_handle().addr().to_string();
    let pid = std::process::id().to_string();
    let (_, code, said) = again(AFTER_MARK_TEST, &[COMMAND, &pid, &held], &[]);
    assert_eq!(code, Some(0), "{said}");
    assert!(said.contains("its command took"), "{said}");
    // Still open by its name to ask.
    let name = wide(&own_name());
    // SAFETY: plain values, and a name that ends in a nul.
    let handle = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, name.as_ptr()) };
    assert!(!handle.is_null(), "{}", io::Error::last_os_error());
    // SAFETY: just opened, owned by nothing else.
    drop(unsafe { OwnedHandle::from_raw_handle(handle) });
}

/// Cineris is told by its file, the very file this binary is (a hard link
/// of it too, under any name), or by its file's name, `cineris.exe`,
/// `cin.exe` or this binary's own, in any case and wherever it is
/// installed. A renamed copy is not; another program given one of those
/// names is (the rule takes the name for Cineris's, and fails closed); a
/// name that only begins or ends so is not. Without the test switch this
/// test binary is not Cineris's.
#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the test lays out the files it judges"
)]
fn sys_cineris_is_told_by_its_file_or_by_its_name() {
    let scratch = cineris_testkit::scratch::Scratch::outside(
        "cineris-sandbox-session",
        "sys_cineris_is_told_by_its_file_or_by_its_name",
    );
    let place = |folder: &str, name: &str| scratch.dir(folder).join(name);
    let exe = std::env::current_exe().unwrap();
    let own = Own::of(&exe);
    let own_name = exe.file_name().unwrap().to_string_lossy().into_owned();
    // The same binary.
    assert!(cineris(&own, &exe), "the same binary");
    // A renamed copy: another file, under another name.
    let copied = place("copied", "renamed.exe");
    std::fs::copy(&exe, &copied).unwrap();
    assert!(!cineris(&own, &copied), "a renamed copy");
    // The same file under another name: a hard link, made of the copy (on
    // the scratch's own volume), as the binary it stands for.
    let linked = place("linked", "linked.exe");
    std::fs::hard_link(&copied, &linked).unwrap();
    assert!(cineris(&Own::of(&copied), &linked), "the same file renamed");
    assert!(!cineris(&Own::of(&copied), &exe), "another file");
    // Installed elsewhere, under this binary's name or Cineris's.
    for (at, name) in [
        own_name.as_str(),
        "cin.exe",
        "cineris.exe",
        "CIN.EXE",
        "Cineris.Exe",
    ]
    .into_iter()
    .enumerate()
    {
        let installed = place(&format!("installed-{at}"), name);
        std::fs::hard_link(&copied, &installed).unwrap();
        assert!(cineris(&own, &installed), "{name} installed elsewhere");
    }
    // Another program, named as Cineris is.
    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    let another = place("another", "cineris.exe");
    std::fs::copy(system.join("System32").join("whoami.exe"), &another).unwrap();
    assert!(cineris(&own, &another), "another program named cineris.exe");
    // Names that are not Cineris's.
    for (at, name) in [
        "cin.exe.old",
        "cineris-x.exe",
        "xcin.exe",
        "cin",
        "cineris.com",
    ]
    .into_iter()
    .enumerate()
    {
        let near = place(&format!("near-{at}"), name);
        std::fs::hard_link(&copied, &near).unwrap();
        assert!(!cineris(&own, &near), "{name}");
    }
    assert!(!cineris(&own, &system.join("explorer.exe")), "explorer.exe");
    // A test binary without the switch: no binary of its own.
    assert!(!switches::own_binary(), "run without the test switch");
    let none = Own::read();
    assert!(
        !cineris(&none, &exe),
        "this test binary, without the switch"
    );
    assert!(cineris(&none, &another), "cineris.exe, without the switch");
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

/// A harness's job is found among its logon session's named objects, by
/// its name, which gives its process's instance; and the check finds it
/// there (the session's own job holds it, once it is marked).
#[test]
fn sys_a_harnesss_job_is_found_among_the_sessions_named_objects() {
    mark().unwrap();
    let own = own_instance();
    let named = named_jobs(crate::session::ROOMS)
        .expect("the runner's tests run in a session of their own")
        .expect("one call lists the session's named objects whole");
    assert!(named.contains(&own), "{own:?} in {named:?}");
    let (found, source) = harnesses(&Taken::new()).unwrap();
    assert_eq!(source, Source::Namespace);
    assert!(found.contains(&own), "{own:?} in {found:?}");
    let under = under_a_session();
    assert!(matches!(under, Under::Session(_)), "{under:?}");
}

/// This test's own name, by which it runs itself again.
const IN_PARTS_TEST: &str =
    "session::sys::tests::sys_a_listing_that_needs_more_than_one_call_takes_the_list_of_processes";

/// A listing of the logon session's named objects that needs more than one
/// call is never trusted, nor joined from its calls: the system's list of
/// processes, which one call gives whole, is taken in its place, and a
/// running harness is not missed: its job is among those found, and its
/// process is under a session's command (the review of the start-up's fix:
/// objects removed between two calls of a listing shift the rest, and the
/// next call skips some no call gave). The test switch
/// (`CINERIS_TEST_SESSION_LISTING=in-parts`) gives every call a room too
/// small for the namespace, up to a bound as small; the test runs itself
/// again with it, in a process of its own that marks itself as a harness.
#[test]
fn sys_a_listing_that_needs_more_than_one_call_takes_the_list_of_processes() {
    if switches::listing() == Some(switches::Listing::InParts) {
        mark().unwrap();
        let own = own_instance();
        let named = named_jobs(crate::session::IN_PARTS);
        assert!(matches!(named, Ok(None)), "{named:?}");
        let (found, source) = harnesses(&Taken::new()).unwrap();
        assert_eq!(source, Source::Processes);
        assert!(found.contains(&own), "{own:?} in {} found", found.len());
        let under = under_a_session();
        assert!(matches!(under, Under::Session(_)), "{under:?}");
        println!("checked in parts: {under:?}");
        return;
    }
    let (_, code, said) = again(IN_PARTS_TEST, &[], &[(LISTING, "in-parts")]);
    assert_eq!(code, Some(0), "{said}");
    assert!(said.contains("checked in parts"), "{said}");
}

/// This test's own name, by which it runs itself again.
const LINE_TEST: &str =
    "session::sys::tests::sys_a_harness_on_the_line_of_parents_is_found_with_nothing_listed";

/// The roles of that test: a harness that runs an approving subcommand's
/// check, and the check.
const HARNESS_OVER_PROBE: &str = "cin-session-harness-over-probe";
const PROBE: &str = "cin-session-probe";

/// A session's command whose harness is on its line of parents is found by
/// that route alone: with the listing made to give nothing (the test
/// switch `CINERIS_TEST_SESSION_LISTING=empty`, which takes no list of
/// processes either), a check a harness runs is under a session's command,
/// by its parent's job; run by the test itself, under no harness, the same
/// is under none. The harness's job is asked by its name on the line of
/// parents before anything is listed, so no listing's race can hide it (the
/// review of the start-up's fix: a listing of the namespace that took
/// several calls could skip it). The test runs itself again in each role,
/// in processes of its own, and only the check is given the switch.
#[test]
fn sys_a_harness_on_the_line_of_parents_is_found_with_nothing_listed() {
    let arguments: Vec<String> = std::env::args().collect();
    let has = |role: &str| arguments.iter().any(|argument| argument == role);
    if has(PROBE) {
        println!("probed: {:?}", under_a_session());
        return;
    }
    if has(HARNESS_OVER_PROBE) {
        // It marks itself, then runs the check with the listing made to
        // give nothing: the check can find it only on its line of parents.
        mark().unwrap();
        let (_, code, said) = again(LINE_TEST, &[PROBE], &[(LISTING, "empty")]);
        print!("{said}");
        assert_eq!(code, Some(0), "{said}");
        return;
    }
    let (harness, code, said) = again(LINE_TEST, &[HARNESS_OVER_PROBE], &[]);
    assert_eq!(code, Some(0), "{said}");
    let found =
        format!("probed: Session(\"it is in the job of process {harness}'s sessions' commands\")");
    assert!(said.contains(&found), "{said}");
    let (_, code, alone) = again(LINE_TEST, &[PROBE], &[(LISTING, "empty")]);
    assert_eq!(code, Some(0), "{alone}");
    assert!(alone.contains("probed: Nothing"), "{alone}");
}

/// A process described by opening it, as the line of parents is read, is
/// what the system's list says of it: its parent, its program's name and
/// when it was made; and a process id nothing has is gone.
#[test]
fn sys_a_process_opened_is_described_as_the_list_says() {
    let listed = processes().unwrap();
    let mut compared = 0;
    for pid in [std::process::id()]
        .into_iter()
        .chain(listed.iter().map(|p| p.pid))
    {
        let super::super::Described::Is(described) = described(pid) else {
            continue;
        };
        let Some(entry) = listed.iter().find(|process| process.pid == pid) else {
            continue;
        };
        assert_eq!(
            (described.parent, described.name.as_str(), described.made),
            (entry.parent, entry.name.as_str(), entry.made),
            "{pid}"
        );
        compared += 1;
    }
    assert!(compared > 1, "{compared}");
    assert!(matches!(
        described(u32::MAX - 3),
        super::super::Described::Gone
    ));
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
