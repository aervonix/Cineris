//! The Windows half of [`super`]: a named pipe that only the user can open,
//! labelled at the server's integrity, and a check of each connection's own
//! token (`docs/steps/27/answered.md`, "The design, before building",
//! point 1, "Windows").
//!
//! - **The pipe's security is given to the call that makes it,** so no
//!   instance ever exists without it. It is written in Windows's own
//!   description language, as `cineris-base`'s owner-only files are
//!   (`docs/decisions/0029-owner-only-files-on-windows.md`):
//!   `O:<user>D:P(A;;GA;;;<user>)S:(ML;;NWNR;;;S-1-16-<level>)` reads: owned
//!   by the user; an access list protected from inheritance (`P`) whose one
//!   entry allows (`A`) everything (`GA`) to the user; and a mandatory label
//!   (`ML`) at the server's own integrity level, which keeps a process below
//!   it from opening the pipe at all (`NW` and `NR`: no write up, no read
//!   up). Without the label a process of the user's at medium integrity
//!   could drive a server that UAC elevated, and its commands would run
//!   elevated. Reading is kept out too, since a client is checked only once
//!   it has written (below): one that could open the pipe to read and never
//!   write could hold connections it could never be refused on.
//! - **Every instance is made with it,** the first with
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE`, so that a name another process holds
//!   (another server, or a squatter) refuses the start rather than joining
//!   it, and all with `PIPE_REJECT_REMOTE_CLIENTS`.
//! - **The next instance is made before a connected one is handed on,** so
//!   the name never lapses between two instances for another process to
//!   take, and is always there for the next client.
//! - **Each connection's own token is checked before anything is served:**
//!   the connection's, not its process's, since a thread that impersonates
//!   another user connects as that user, and a process id can be reused. A
//!   thread of its own impersonates the client (`ImpersonateNamedPipeClient`),
//!   opens the thread's token with the process's own right to
//!   (`OpenThreadToken` with `OpenAsSelf`), and stops impersonating at once
//!   (`RevertToSelf`), with nothing awaited in between; a thread that cannot
//!   stop ends the process, since it would go on as the client. The core's
//!   thread never impersonates. The client is admitted if its token's user
//!   is the server's, its integrity is at least the server's, and it is
//!   neither restricted nor an AppContainer's. A client that allows only
//!   anonymous impersonation has no token to read, and is refused.
//! - **The check waits for the client's first data.** Windows lets a server
//!   impersonate a pipe's client only once it has read something the client
//!   wrote: before that, `ImpersonateNamedPipeClient` fails with
//!   `ERROR_CANNOT_IMPERSONATE` (seen on the hosted Windows runners). And the
//!   token it gives is that of the client's thread that wrote what was read,
//!   not of the one that opened the pipe. So each connection is read, in a
//!   task of its own, until its first data, which is held, the client is
//!   checked, and only an admitted client is handed on, with that data first.
//!   A client that writes nothing holds up no other; past [`CHECKING`] of
//!   them, the one that has waited longest is closed. Nothing is written to
//!   a client before it is admitted, and what was read from a refused one is
//!   dropped with it.
//! - **The server's own commands are refused** (`docs/steps/27/answered.md`,
//!   "The harness's own commands"). A command the agent runs is a process of
//!   the user's, which could otherwise connect and answer its own session's
//!   approvals. When the first listener of a process is bound, the process
//!   joins an empty job of its own: no limits, no breakaway allowed. Every
//!   process it starts from then on (a command's tree, a hook, an MCP
//!   server) is in that job and cannot leave it; the sandbox's job for each
//!   command nests inside it. A client whose process is in the job is
//!   refused, and so is one whose process cannot be looked at. The server's
//!   own process may connect: it is in the job too, and it is the server.
//!   If the job cannot be made or joined, the listener is not bound. This is
//!   defence in depth, and a thin one (the independent review of step 27's
//!   second part): a process of the user at the same integrity may open
//!   any other such process with full rights by default, so a command can
//!   take a connection that is admitted already from a client outside the
//!   job (the user's terminal, `cineris attach`), act from inside the
//!   server's own process, or make an instance of the pipe's name for a new
//!   client to reach; and a service that starts processes for it (WMI's
//!   `Win32_Process.Create`, the task scheduler) puts one outside the job.
//!   Windows has no sandbox in the first release: what keeps a command from
//!   answering its session's approvals there is the person who approved
//!   the command (`docs/security.md`, section 5).
//! - **The front end's check of each client's process** ([`super::PeerCheck`]),
//!   after that: `cineris serve` refuses a client under any harness's
//!   session's command, read as the approving subcommands read their own
//!   process (the review of step 28, finding 3: another harness's command,
//!   another session of Cineris's included, could otherwise answer this
//!   server's trust questions and approvals). It is as thin, for the same
//!   reasons.
//!
//! **The client** ([`connect`], for `cineris attach`) checks the server from
//! the handle it opened, before it sends anything: the pipe's owner must be
//! the user, and its label at least medium (`GetSecurityInfo`). Another user
//! cannot make the user's SID the owner of a pipe of theirs, and a process
//! of the user's below medium cannot give its pipe a medium label: the
//! system labels what such a process makes at its own level. A pipe with no
//! label at all is taken as medium, as Windows itself takes it: only a
//! process at medium or above can make one, so refusing it would keep out
//! nothing that the owner's check does not. The client lets the server
//! identify it and no more (`SECURITY_IDENTIFICATION`): the server reads
//! its token and cannot act as it.
//!
//! The instances are made here (`CreateNamedPipeW`) rather than by tokio's
//! `ServerOptions`, which needs a runtime to make one: binding needs none,
//! and the first `accept` takes the first instance into the runtime it runs
//! in, as the Unix half does with its socket.

#![allow(
    unsafe_code,
    reason = "a pipe's security descriptor and the check of a client's token are only \
              reachable through the Windows API; this is the one module of cineris-rpc \
              where unsafe is allowed"
)]

use std::collections::VecDeque;
use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::os::windows::io::{
    AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, IntoRawHandle, OwnedHandle,
};
use std::path::Path;
use std::ptr;
use std::sync::OnceLock;
use std::task::Poll;

use tokio::io::AsyncReadExt;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer};
use tokio::sync::oneshot;
use tokio::task::{AbortHandle, JoinError, JoinSet};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_SUCCESS, FALSE, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree, SetLastError, TRUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, GetAce, GetAclInformation,
    GetLengthSid, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    IsTokenRestricted, IsValidSid, LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, RevertToSelf, SECURITY_ATTRIBUTES, SECURITY_MAX_SID_SIZE,
    SID_AND_ATTRIBUTES, SYSTEM_MANDATORY_LABEL_ACE, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TOKEN_USER, TokenIntegrityLevel, TokenIsAppContainer, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
    SECURITY_IDENTIFICATION,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
};
use windows_sys::Win32::System::Pipes::{
    CreateNamedPipeW, GetNamedPipeClientProcessId, ImpersonateNamedPipeClient, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcess, OpenProcessToken, OpenThreadToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::{Accepted, ListenOptions, PeerCheck, Place};

/// How every pipe's whole name begins.
const PIPES: &str = r"\\.\pipe\";

/// What each direction of an instance holds in the system: one read of the
/// connection's.
const BUFFER: u32 = 64 * 1024;

/// The most read of a client before its token is checked: what Windows
/// needs read before it lets the server impersonate the client. A client's
/// first line comes in it whole or in part; the rest follows once it is
/// admitted.
const FIRST: usize = 4 * 1024;

/// The most connections that may wait for their first data at once. One
/// more closes the one that has waited longest, so that clients that never
/// write cannot hold the pipe's instances. A client that writes as it
/// connects, as every client of the protocol does, is checked within
/// microseconds, long before so many come after it.
const CHECKING: usize = 16;

pub(super) fn default_place(_runtime: Option<&Path>) -> io::Result<Place> {
    let user = cineris_base::private::current_user_sid()?;
    Ok(Place::Pipe(format!("{PIPES}cineris-{user}")))
}

/// Memory that Windows allocated for its caller and that is given back with
/// `LocalFree`. Holding it here means it is freed once, on every path.
struct LocalMemory(*mut c_void);

impl Drop for LocalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer was returned by a Windows function that is
            // documented to allocate its result with `LocalAlloc` and to
            // leave freeing it to the caller. It is owned by this value
            // alone, so it is freed exactly once, and no reference into the
            // memory outlives this value.
            unsafe { LocalFree(self.0) };
        }
    }
}

/// Who the server is, as the check of a client compares it.
#[derive(Debug, Clone, Copy)]
struct Server {
    /// The user's security identifier, in text.
    user: &'static str,
    /// The server's integrity level ([`integrity_level`]).
    level: u32,
}

/// The text of the pipe's security descriptor (the module's first point).
/// `widen`, for tests only, also allows everyone to read and write, and
/// puts the label at low, so that nothing but the check of a connection's
/// token keeps another user, or a lower integrity, out.
fn description(server: Server, widen: bool) -> String {
    let Server { user, level } = server;
    if widen {
        format!("O:{user}D:P(A;;GA;;;{user})(A;;GRGW;;;WD)S:(ML;;NWNR;;;LW)")
    } else {
        format!("O:{user}D:P(A;;GA;;;{user})S:(ML;;NWNR;;;S-1-16-{level})")
    }
}

/// Whether the listener's options ask for the widened pipe, which only a
/// build with the `test-hooks` feature can.
fn widened(options: &ListenOptions) -> bool {
    #[cfg(feature = "test-hooks")]
    {
        options.widen_access_for_tests
    }
    #[cfg(not(feature = "test-hooks"))]
    {
        let _ = options;
        false
    }
}

/// Text as Windows takes it: UTF-16, ended with a zero.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Makes one instance of the pipe `name` with the descriptor `description`,
/// both zero-terminated UTF-16. `first` asks that no instance of the name
/// exist yet.
fn instance(name: &[u16], description: &[u16], first: bool) -> io::Result<OwnedHandle> {
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `description` is a zero-terminated UTF-16 string that is alive
    // for the call. `descriptor` is a live, writable location for the
    // pointer the call returns. The last argument is an optional output and
    // may be null.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            description.as_ptr(),
            SDDL_REVISION_1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(io::Error::last_os_error());
    }
    let descriptor = LocalMemory(descriptor);
    const BYTES: u32 = size_of::<SECURITY_ATTRIBUTES>() as u32;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: BYTES,
        lpSecurityDescriptor: descriptor.0,
        // The handle is not handed to child processes.
        bInheritHandle: 0,
    };
    let first = if first {
        FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        0
    };
    // SAFETY: `name` is a zero-terminated UTF-16 string and `attributes` a
    // fully initialised structure; both are alive for the call, and so is
    // the descriptor `attributes` points to, which `descriptor` owns until
    // this function returns.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            // Overlapped, as tokio's pipe is: it reads and writes it.
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | first,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER,
            BUFFER,
            // The wait a client's `WaitNamedPipe` takes by default: the
            // system's own (50 ms).
            0,
            &raw const attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // Read before anything else can change the thread's last error.
        return Err(io::Error::last_os_error());
    }
    drop(descriptor);
    // SAFETY: the call succeeded, so `handle` is an open pipe handle that
    // nothing else knows about. `OwnedHandle` takes sole ownership and
    // closes it.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// An instance taken into the runtime this is called in.
fn registered(instance: OwnedHandle) -> io::Result<NamedPipeServer> {
    // SAFETY: `instance` is the server end of a pipe made overlapped, which
    // is what tokio's pipe reads and writes, and it is given up here: the
    // result is its one owner, and closes it, on success or failure.
    unsafe { NamedPipeServer::from_raw_handle(instance.into_raw_handle()) }
}

/// Reads one class of a token's information into `into`.
///
/// # Safety
///
/// `T` must be the structure Windows writes for `class`, followed by room
/// for whatever it points to, and valid for any bytes Windows writes there.
unsafe fn token_information<T>(
    token: BorrowedHandle<'_>,
    class: TOKEN_INFORMATION_CLASS,
    into: &mut T,
) -> io::Result<()> {
    // Every structure read here is a few dozen bytes: its size always fits.
    let size = u32::try_from(size_of::<T>()).unwrap_or(u32::MAX);
    let mut written = 0_u32;
    // SAFETY: `token` is open, alive for the call, with `TOKEN_QUERY`, the
    // right a query needs. `into` is writable for `size` bytes, its whole
    // size, and aligned for `T`, which the caller guarantees is what this
    // class writes. `written` is a live `u32`.
    let read = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            class,
            ptr::from_mut(into).cast(),
            size,
            &raw mut written,
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// A security identifier as text, such as `S-1-5-18`.
///
/// # Safety
///
/// `identifier` must point to a valid security identifier that stays alive
/// and unchanged for the duration of the call.
unsafe fn identifier_text(identifier: PSID) -> io::Result<String> {
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: the caller guarantees `identifier` is valid. `text` is a live,
    // writable location for the pointer the call returns.
    if unsafe { ConvertSidToStringSidW(identifier, &raw mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // From here the string is freed on every path.
    let memory = LocalMemory(text.cast());
    let mut length = 0_usize;
    // SAFETY: the call succeeded, so `text` points to a UTF-16 string that
    // Windows allocated and ended with a zero. Each read is at or before
    // that zero, so inside the allocation, which `memory` keeps alive.
    while unsafe { text.add(length).read() } != 0 {
        length += 1;
    }
    // SAFETY: the `length` units before the zero were just read one by one,
    // so they are initialised and inside the allocation. `u16` has no
    // invalid values, the pointer is aligned for it, and `memory` outlives
    // the slice.
    let units = unsafe { std::slice::from_raw_parts(text, length) };
    let converted = String::from_utf16(units)
        .map_err(|_| io::Error::other("a security identifier did not convert to text"));
    drop(memory);
    converted
}

/// The user a token is of, as a security identifier in text.
fn token_user(token: BorrowedHandle<'_>) -> io::Result<String> {
    /// What Windows writes for `TokenUser`: the structure, and after it the
    /// identifier it points to, which is never longer than
    /// `SECURITY_MAX_SID_SIZE`.
    #[repr(C)]
    struct UserBuffer {
        user: TOKEN_USER,
        identifier: [u8; SECURITY_MAX_SID_SIZE as usize],
    }
    let mut buffer = UserBuffer {
        user: TOKEN_USER {
            User: SID_AND_ATTRIBUTES {
                Sid: ptr::null_mut(),
                Attributes: 0,
            },
        },
        identifier: [0; SECURITY_MAX_SID_SIZE as usize],
    };
    // SAFETY: `UserBuffer` is the structure this class writes with room
    // after it for its identifier; any bytes are valid for it.
    unsafe { token_information(token, TokenUser, &mut buffer) }?;
    let identifier = buffer.user.User.Sid;
    let (start, end) = bytes_of(&buffer);
    // SAFETY: the bytes from `start` to `end` are `buffer`'s, which is alive
    // and unchanged until this function returns.
    if !unsafe { identifier_within(identifier, start, end) } {
        return Err(io::Error::other("the token's user is not an identifier"));
    }
    // SAFETY: just shown to be a valid identifier lying wholly inside
    // `buffer`, which is alive and unchanged.
    unsafe { identifier_text(identifier) }
}

/// A token's integrity level: the last part of its label's identifier,
/// `S-1-16-<level>`, where 4096 is low, 8192 medium, 12288 high and 16384
/// the system's.
fn integrity_level(token: BorrowedHandle<'_>) -> io::Result<u32> {
    /// What Windows writes for `TokenIntegrityLevel`: the structure, and
    /// after it the identifier it points to.
    #[repr(C)]
    struct LabelBuffer {
        label: TOKEN_MANDATORY_LABEL,
        identifier: [u8; SECURITY_MAX_SID_SIZE as usize],
    }
    let mut buffer = LabelBuffer {
        label: TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: ptr::null_mut(),
                Attributes: 0,
            },
        },
        identifier: [0; SECURITY_MAX_SID_SIZE as usize],
    };
    // SAFETY: `LabelBuffer` is the structure this class writes with room
    // after it for its identifier; any bytes are valid for it.
    unsafe { token_information(token, TokenIntegrityLevel, &mut buffer) }?;
    let identifier = buffer.label.Label.Sid;
    let (start, end) = bytes_of(&buffer);
    // SAFETY: the bytes from `start` to `end` are `buffer`'s, which is alive
    // and unchanged until this function returns.
    if !unsafe { identifier_within(identifier, start, end) } {
        return Err(io::Error::other("the token's label is not an identifier"));
    }
    // SAFETY: just shown to be a valid identifier lying wholly inside
    // `buffer`, which is alive and unchanged.
    unsafe { last_part(identifier) }
}

/// The addresses of the first byte of `value` and of the byte after it.
fn bytes_of<T>(value: &T) -> (usize, usize) {
    let start = ptr::from_ref(value).addr();
    (start, start + size_of::<T>())
}

/// Whether `identifier` is a valid security identifier lying wholly
/// between the addresses `start` and `end`: its first eight bytes, which
/// hold its count of parts, and then the whole length that count gives.
/// A pointer Windows wrote, or an identifier inside an access list, is
/// read no further than shown.
///
/// # Safety
///
/// The bytes from `start` to `end` must be readable, and stay alive and
/// unchanged for the duration of the call.
unsafe fn identifier_within(identifier: PSID, start: usize, end: usize) -> bool {
    /// An identifier's fixed part: revision, count of parts, authority.
    const FIXED: usize = 8;
    let at = identifier.addr();
    if identifier.is_null() || at < start || at.checked_add(FIXED).is_none_or(|fixed| fixed > end) {
        return false;
    }
    // SAFETY: the eight bytes at `identifier` are between `start` and `end`,
    // which the caller holds readable; `IsValidSid` reads no more of them
    // than its revision and its count of parts.
    if unsafe { IsValidSid(identifier) } == 0 {
        return false;
    }
    // SAFETY: just checked valid; `GetLengthSid` reads its count of parts,
    // among the same eight bytes.
    let length = usize::try_from(unsafe { GetLengthSid(identifier) }).unwrap_or(usize::MAX);
    at.checked_add(length).is_some_and(|after| after <= end)
}

/// The last part of a label's identifier (`S-1-16-<level>`): its level.
///
/// # Safety
///
/// `identifier` must be a valid security identifier whose whole length is
/// readable, and stays alive and unchanged for the duration of the call:
/// what [`identifier_within`] shows.
unsafe fn last_part(identifier: PSID) -> io::Result<u32> {
    // SAFETY: the caller guarantees the identifier is valid and wholly
    // readable, so its count of parts is inside it.
    let parts = unsafe { GetSidSubAuthorityCount(identifier).read() };
    let Some(last) = u32::from(parts).checked_sub(1) else {
        return Err(io::Error::other("a label has no level"));
    };
    // SAFETY: `last` is below the identifier's count of parts, and the
    // caller guarantees the whole identifier, every part of it, readable.
    // Windows places an identifier where it likes, so the read makes no
    // assumption of alignment.
    Ok(unsafe { GetSidSubAuthority(identifier, last).read_unaligned() })
}

/// The integrity level of the server's own process.
fn own_integrity() -> io::Result<u32> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that is always
    // valid and is never closed. `token` is a live, writable location for
    // the one handle the call returns.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded, so `token` is an open handle that nothing
    // else knows about. `OwnedHandle` takes sole ownership and closes it.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    integrity_level(token.as_handle())
}

/// Whether a token is restricted: it has restricting identifiers, which a
/// sandbox gives the processes it confines.
fn is_restricted(token: BorrowedHandle<'_>) -> io::Result<bool> {
    // SAFETY: sets this thread's last error, and touches nothing else.
    unsafe { SetLastError(ERROR_SUCCESS) };
    // SAFETY: `token` is open, alive for the call, with `TOKEN_QUERY`.
    if unsafe { IsTokenRestricted(token.as_raw_handle()) } != 0 {
        return Ok(true);
    }
    // A zero is "not restricted" or a failure, told apart by the thread's
    // last error, which was cleared before the call.
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(0) {
        Ok(false)
    } else {
        Err(error)
    }
}

/// Whether a token is an AppContainer's.
fn is_app_container(token: BorrowedHandle<'_>) -> io::Result<bool> {
    let mut found = 0_u32;
    // SAFETY: this class writes one 32-bit number, which is what `found` is.
    unsafe { token_information(token, TokenIsAppContainer, &mut found) }?;
    Ok(found != 0)
}

/// The client's process id, `None` when Windows does not say it.
fn client_process(pipe: BorrowedHandle<'_>) -> Option<u32> {
    let mut pid = 0_u32;
    // SAFETY: `pipe` is the server end of a connected instance, alive for the
    // call, and `pid` is a live `u32`.
    let told = unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &raw mut pid) };
    (told != 0).then_some(pid)
}

/// The job of the server's own commands (the module's point on them): made
/// empty, with no limits, and joined by this process, the first time a
/// listener is bound. A process stays in a job for good, so one job serves
/// every listener of the process: a second, nested inside the first, would
/// tell nothing more.
static OWN_COMMANDS: OnceLock<OwnedHandle> = OnceLock::new();

/// The job of the server's own commands, made and joined if it is not yet.
fn own_commands() -> io::Result<BorrowedHandle<'static>> {
    if let Some(job) = OWN_COMMANDS.get() {
        return Ok(job.as_handle());
    }
    // SAFETY: both arguments may be null: default security, which makes the
    // handle not inheritable, and no name.
    let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` is a job that was just made and that nothing else
    // owns. `OwnedHandle` takes sole ownership and closes it.
    let job = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: `job` is open with every right its maker gets, and
    // `GetCurrentProcess` returns a pseudo-handle that is always valid. A
    // process already in a job (a test runner's, a terminal's) takes the
    // empty one as a job nested in that one.
    let joined = unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) };
    if joined == 0 {
        return Err(io::Error::last_os_error());
    }
    // Two threads binding at once may each make a job and join it: the
    // second is then nested in the first, so the one kept holds every
    // process started after either.
    Ok(OWN_COMMANDS.get_or_init(|| job).as_handle())
}

/// Whether the process `pid` is in `job`: one the server started, or one
/// started by those.
fn in_job(pid: u32, job: BorrowedHandle<'_>) -> io::Result<bool> {
    // SAFETY: the call takes plain values; its result is checked before use.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `process` was just opened and nothing else owns it.
    // `OwnedHandle` takes sole ownership and closes it.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    let mut inside = FALSE;
    // SAFETY: `process` is open with the right this asks, `job` is open with
    // every right, both are alive for the call, and `inside` is a live
    // `BOOL`.
    let asked = unsafe {
        IsProcessInJob(
            process.as_raw_handle(),
            job.as_raw_handle(),
            &raw mut inside,
        )
    };
    if asked == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(inside != 0)
}

/// Stops this thread impersonating. A thread that cannot would go on as the
/// client, so the process ends instead.
fn revert() {
    // SAFETY: takes no arguments and ends this thread's impersonation, if
    // any; it touches no memory of the caller's.
    if unsafe { RevertToSelf() } == 0 {
        tracing::error!(
            error = %io::Error::last_os_error(),
            "a thread that checked a client's token could not stop impersonating it"
        );
        std::process::abort();
    }
}

/// The connected client's own token, open for queries. This thread
/// impersonates the client to take it, and stops before this returns.
fn client_token(pipe: BorrowedHandle<'_>) -> io::Result<OwnedHandle> {
    // SAFETY: `pipe` is the server end of a connected instance, alive for
    // the call.
    if unsafe { ImpersonateNamedPipeClient(pipe.as_raw_handle()) } == 0 {
        let error = io::Error::last_os_error();
        // A failed impersonation leaves the thread as it was; reverting is
        // harmless and makes sure.
        revert();
        return Err(error);
    }
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `GetCurrentThread` returns a pseudo-handle that is always
    // valid and is never closed. `token` is a live, writable location for
    // the one handle the call returns. `OpenAsSelf`: the token is opened
    // with the process's own right, since the client's token may allow its
    // holder only to be identified, not to open anything.
    let opened =
        unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &raw mut token) } != 0;
    // Read before reverting, which may change the thread's last error.
    let error = (!opened).then(io::Error::last_os_error);
    revert();
    match error {
        Some(error) => Err(error),
        // SAFETY: the call succeeded, so `token` is an open handle that
        // nothing else knows about. `OwnedHandle` takes sole ownership and
        // closes it.
        None => Ok(unsafe { OwnedHandle::from_raw_handle(token) }),
    }
}

/// The check of a connected client: `Ok` with how to name it in the log, or
/// `Err` with why it is refused. It impersonates, so it runs on a thread of
/// its own. `job` is the job of the server's own commands; `peer_check` the
/// front end's check of each client's process, after it.
fn check(
    pipe: BorrowedHandle<'_>,
    server: Server,
    job: BorrowedHandle<'_>,
    peer_check: Option<&PeerCheck>,
) -> Result<String, String> {
    let pid = client_process(pipe);
    let peer = pid.map_or_else(|| "pid unknown".to_owned(), |pid| format!("pid {pid}"));
    let unread =
        |error: io::Error| format!("the client's token could not be read ({peer}): {error}");
    let token = client_token(pipe).map_err(unread)?;
    let token = token.as_handle();
    let user = token_user(token).map_err(unread)?;
    if user != server.user {
        return Err(format!("a client of another user ({user}, {peer})"));
    }
    let level = integrity_level(token).map_err(unread)?;
    if level < server.level {
        return Err(format!(
            "a client below the server's integrity (level {level} under {}, {peer})",
            server.level
        ));
    }
    if is_restricted(token).map_err(unread)? {
        return Err(format!("a client with a restricted token ({peer})"));
    }
    if is_app_container(token).map_err(unread)? {
        return Err(format!("a client in an AppContainer ({peer})"));
    }
    let Some(pid) = pid else {
        return Err("a client whose process is not known".to_owned());
    };
    // The server's own process is in the job too, and may connect to itself.
    if pid != std::process::id() {
        let own = in_job(pid, job).map_err(|error| {
            format!("a client whose process cannot be looked at ({peer}): {error}")
        })?;
        if own {
            return Err(format!("one of the server's own commands ({peer})"));
        }
    }
    if let Some(why) = peer_check.and_then(|check| check.refusal(pid)) {
        return Err(format!("a client refused ({peer}): {why}"));
    }
    Ok(peer)
}

/// The error for a first instance Windows refused.
fn held(name: &str, error: io::Error) -> io::Error {
    match error.raw_os_error().map(i32::cast_unsigned) {
        Some(ERROR_ACCESS_DENIED | ERROR_PIPE_BUSY) => io::Error::new(
            io::ErrorKind::AddrInUse,
            format!(
                "the pipe {name} exists already: another server listens there, or another \
                 process holds the name. Stop that server, or serve at another name"
            ),
        ),
        _ => io::Error::new(error.kind(), format!("{name}: {error}")),
    }
}

/// A connected client, read until its first data and then checked: its
/// bytes, that data first, when it is admitted; why not otherwise.
async fn admit(
    mut pipe: NamedPipeServer,
    server: Server,
    job: BorrowedHandle<'static>,
    peer_check: Option<PeerCheck>,
) -> Accepted {
    let mut first = vec![0; FIRST];
    let count = match pipe.read(&mut first).await {
        Ok(0) => return Accepted::Refused("a client that left before it sent anything".to_owned()),
        Ok(count) => count,
        Err(error) => {
            return Accepted::Refused(format!("a client that could not be read: {error}"));
        }
    };
    first.truncate(count);
    let (told, verdict) = oneshot::channel();
    let checking = std::thread::Builder::new()
        .name("cineris-pipe-check".to_owned())
        .spawn(move || {
            let verdict = check(pipe.as_handle(), server, job, peer_check.as_ref());
            let _ = told.send((pipe, verdict));
        });
    if let Err(error) = checking {
        return Accepted::Refused(format!("no thread to check the client's token on: {error}"));
    }
    let Ok((pipe, verdict)) = verdict.await else {
        return Accepted::Refused("the check of the client's token did not finish".to_owned());
    };
    match verdict {
        Ok(peer) => {
            let (read, write) = tokio::io::split(pipe);
            // What was read for the check goes first.
            let read = std::io::Cursor::new(first).chain(read);
            Accepted::Client {
                transport: super::transport_of(read, write),
                peer,
            }
        }
        Err(why) => Accepted::Refused(why),
    }
}

/// What the listener was woken by.
enum Woken {
    /// A connection's check ended.
    Checked(Result<Accepted, JoinError>),
    /// A client connected to the waiting instance.
    Connected(io::Result<()>),
}

fn gone() -> io::Error {
    io::Error::other("the listener's pipe is gone")
}

#[derive(Debug)]
pub(super) struct Listener {
    place: Place,
    /// The pipe's whole name, zero-terminated UTF-16.
    name: Vec<u16>,
    /// The security descriptor's text, zero-terminated UTF-16.
    description: Vec<u16>,
    server: Server,
    /// The job of the server's own commands.
    job: BorrowedHandle<'static>,
    /// The front end's check of each client's process, after the server's
    /// own.
    peer_check: Option<PeerCheck>,
    /// The first instance, until the first `accept` takes it into the
    /// runtime it runs in: binding needs no runtime.
    bound: Option<OwnedHandle>,
    /// The instance the next client connects to.
    waiting: Option<NamedPipeServer>,
    /// The connections waiting for their first data and their check.
    checking: JoinSet<Accepted>,
    /// Each of those, oldest first, to close the oldest past [`CHECKING`].
    order: VecDeque<AbortHandle>,
}

impl Listener {
    pub(super) fn bind(place: &Place, options: &ListenOptions) -> io::Result<Self> {
        let Place::Pipe(whole) = place else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a Windows server listens at a named pipe",
            ));
        };
        if !whole.starts_with(PIPES) || whole.len() == PIPES.len() || whole.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{whole} is not a pipe's whole name ({PIPES}NAME)"),
            ));
        }
        let server = Server {
            user: cineris_base::private::current_user_sid()?,
            level: own_integrity()?,
        };
        let job = own_commands().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "the server's own commands could not be told from other clients: a job \
                     for them could not be made or joined ({error})"
                ),
            )
        })?;
        let name = wide(whole);
        let description = wide(&description(server, widened(options)));
        let first = instance(&name, &description, true).map_err(|error| held(whole, error))?;
        Ok(Self {
            place: place.clone(),
            name,
            description,
            server,
            job,
            peer_check: options.peer_check.clone(),
            bound: Some(first),
            waiting: None,
            checking: JoinSet::new(),
            order: VecDeque::new(),
        })
    }

    pub(super) fn place(&self) -> &Place {
        &self.place
    }

    pub(super) async fn accept(&mut self) -> io::Result<Accepted> {
        if let Some(first) = self.bound.take() {
            self.waiting = Some(registered(first)?);
        }
        loop {
            let woken = {
                let Some(waiting) = &self.waiting else {
                    return Err(gone());
                };
                let checking = &mut self.checking;
                let mut connecting = std::pin::pin!(waiting.connect());
                std::future::poll_fn(|context| {
                    if let Poll::Ready(Some(checked)) = checking.poll_join_next(context) {
                        return Poll::Ready(Woken::Checked(checked));
                    }
                    connecting.as_mut().poll(context).map(Woken::Connected)
                })
                .await
            };
            match woken {
                Woken::Checked(Ok(accepted)) => return Ok(accepted),
                Woken::Checked(Err(ended)) if ended.is_cancelled() => {
                    return Ok(Accepted::Refused(
                        "a client that sent nothing, closed to make room for newer ones".to_owned(),
                    ));
                }
                Woken::Checked(Err(ended)) => {
                    return Ok(Accepted::Refused(format!(
                        "the check of a client's token failed: {ended}"
                    )));
                }
                Woken::Connected(connected) => {
                    connected?;
                    self.hand_to_check()?;
                }
            }
        }
    }

    /// Hands the instance a client just connected to to a check of its own,
    /// and puts the next instance in its place first.
    fn hand_to_check(&mut self) -> io::Result<()> {
        // The next instance is there before this one goes anywhere, so the
        // name never lapses.
        let next = registered(instance(&self.name, &self.description, false)?)?;
        let connected = self.waiting.replace(next).ok_or_else(gone)?;
        self.order.retain(|waiting| !waiting.is_finished());
        if self.order.len() >= CHECKING
            && let Some(oldest) = self.order.pop_front()
        {
            oldest.abort();
        }
        let started = self.checking.spawn(admit(
            connected,
            self.server,
            self.job,
            self.peer_check.clone(),
        ));
        self.order.push_back(started);
        Ok(())
    }
}

/// Medium integrity: the least a server's pipe may be labelled at, and how
/// a pipe with no label is taken.
const MEDIUM: u32 = 8192;

/// How many times a client waits for an instance of a pipe whose instances
/// are all taken, and how long each wait may be, in milliseconds. A server
/// makes its next instance as soon as a client connects, so a wait is short.
const BUSY_TRIES: u32 = 5;
const BUSY_WAIT_MS: u32 = 1_000;

/// The type of a mandatory label's entry in a system access list. It is the
/// value of the entry-type byte for one, and has been since Windows Vista.
const LABEL_ENTRY: u8 = 0x11;

fn not_reached(name: &str, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("the server at {name} is not reached: {why}"),
    )
}

/// Opens the pipe `name` as a client that lets the server identify it and
/// no more, waiting a while for an instance when all of them are taken.
async fn open_client(name: &str) -> io::Result<NamedPipeClient> {
    let mut options = ClientOptions::new();
    options.security_qos_flags(SECURITY_IDENTIFICATION);
    let mut tries = 0;
    loop {
        let error = match options.open(name) {
            Ok(client) => return Ok(client),
            Err(error) => error,
        };
        match error.raw_os_error().map(i32::cast_unsigned) {
            Some(ERROR_PIPE_BUSY) if tries < BUSY_TRIES => {
                tries += 1;
                let wide_name = wide(name);
                // The wait blocks, so it is on the blocking pool; a wait that
                // ends with no instance free is tried again, so many times.
                let _ = tokio::task::spawn_blocking(move || {
                    // SAFETY: `wide_name` is zero-terminated and alive for
                    // the call.
                    unsafe { WaitNamedPipeW(wide_name.as_ptr(), BUSY_WAIT_MS) }
                })
                .await;
            }
            Some(ERROR_FILE_NOT_FOUND) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no server listens at {name}"),
                ));
            }
            _ => return Err(io::Error::new(error.kind(), format!("{name}: {error}"))),
        }
    }
}

/// The level of the mandatory label in `list`, the system access list
/// Windows gives for a label: `None` when it has none.
///
/// # Safety
///
/// `list` must be null, or point to a valid access list that stays alive
/// and unchanged for the duration of the call.
unsafe fn label_level(list: *const ACL) -> io::Result<Option<u32>> {
    /// Where the label's identifier begins in its entry.
    const IDENTIFIER_AT: usize = std::mem::offset_of!(SYSTEM_MANDATORY_LABEL_ACE, SidStart);
    const SIZE_BYTES: u32 = size_of::<ACL_SIZE_INFORMATION>() as u32;

    if list.is_null() {
        return Ok(None);
    }
    let unreadable = || io::Error::other("the pipe's label cannot be read");
    // SAFETY: the caller guarantees `list` is a valid access list, so its
    // header is readable. The read is unaligned because a list is a byte
    // layout, not a Rust value.
    let whole = unsafe { list.read_unaligned() };
    let list_end = list.addr() + usize::from(whole.AclSize);
    let mut size = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: the caller guarantees `list` is a valid access list. `size` is
    // writable for `SIZE_BYTES` bytes, its whole size, and is the structure
    // this class fills.
    if unsafe { GetAclInformation(list, (&raw mut size).cast(), SIZE_BYTES, AclSizeInformation) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    for index in 0..size.AceCount {
        let mut entry: *mut c_void = ptr::null_mut();
        // SAFETY: `list` is valid, as above, and `entry` is a live location
        // for the pointer the call returns.
        if unsafe { GetAce(list, index, &raw mut entry) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // Every entry is read no further than the list's own size says.
        let at = entry.addr();
        if at < list.addr()
            || at
                .checked_add(size_of::<ACE_HEADER>())
                .is_none_or(|after| after > list_end)
        {
            return Err(unreadable());
        }
        // SAFETY: the entry's header was just shown to be inside the list,
        // which the caller keeps alive. The read is unaligned because a list
        // is a byte layout, not a Rust value.
        let header = unsafe { entry.cast::<ACE_HEADER>().read_unaligned() };
        if header.AceType != LABEL_ENTRY {
            continue;
        }
        let entry_end = at + usize::from(header.AceSize);
        if entry_end > list_end || at + IDENTIFIER_AT >= entry_end {
            return Err(unreadable());
        }
        // SAFETY: `IDENTIFIER_AT` is inside the entry, just shown.
        let identifier: PSID = unsafe { entry.cast::<u8>().add(IDENTIFIER_AT) }.cast();
        // SAFETY: the bytes from the entry to its end are inside the list,
        // just shown, which the caller keeps alive and unchanged.
        if !unsafe { identifier_within(identifier, at, entry_end) } {
            return Err(unreadable());
        }
        // SAFETY: just shown to be a valid identifier lying wholly inside the
        // entry, which is inside the list.
        return unsafe { last_part(identifier) }.map(Some);
    }
    Ok(None)
}

/// Whether the server at the other end of `pipe` is the user's: the pipe
/// owned by `user`, and labelled at least medium. `Err` says why not.
fn check_server(pipe: BorrowedHandle<'_>, user: &str) -> Result<(), String> {
    let mut owner: PSID = ptr::null_mut();
    let mut label: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `pipe` is open with the right to read its security (a client
    // that reads may), alive for the call. `owner`, `label` and `descriptor`
    // are live, writable locations for the pointers the call returns; the
    // group and the access list are not asked for, and their outputs are
    // null.
    let status = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION,
            &raw mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut label,
            &raw mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        // This call returns its error; it does not set the thread's.
        let error = io::Error::from_raw_os_error(status.cast_signed());
        return Err(format!("its security could not be read ({error})"));
    }
    // `owner` and `label` point into this allocation, alive until the end.
    let descriptor = LocalMemory(descriptor);
    // SAFETY: a non-null owner from a successful `GetSecurityInfo` is a
    // valid identifier inside `descriptor`, which is alive.
    if owner.is_null() || unsafe { IsValidSid(owner) } == 0 {
        return Err("it has no owner that can be read".to_owned());
    }
    // SAFETY: just checked valid, inside `descriptor`, which is alive.
    let owner = unsafe { identifier_text(owner) }
        .map_err(|error| format!("its owner could not be read ({error})"))?;
    if owner != user {
        return Err(format!(
            "the pipe is owned by {owner}, not by you: another user's server, or a pipe someone \
             else made at that name"
        ));
    }
    // SAFETY: `label` is null or the system access list of the successful
    // call above, inside `descriptor`, which is alive.
    let level = unsafe { label_level(label) }
        .map_err(|error| format!("its label could not be read ({error})"))?
        .unwrap_or(MEDIUM);
    if level < MEDIUM {
        return Err(format!(
            "the pipe is labelled at integrity level {level}, below medium ({MEDIUM}): a lower \
             process of yours made it"
        ));
    }
    drop(descriptor);
    Ok(())
}

/// Connects to the server at `place` once it is shown to be the user's: the
/// pipe's owner the user, and its label at least medium (the module's point
/// on the client). Nothing is sent before.
pub(super) async fn connect(place: &Place) -> io::Result<super::Transport> {
    let Place::Pipe(name) = place else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a Windows server is reached at a named pipe",
        ));
    };
    let user = cineris_base::private::current_user_sid()?;
    let pipe = open_client(name).await?;
    check_server(pipe.as_handle(), user).map_err(|why| not_reached(name, &why))?;
    let (read, write) = tokio::io::split(pipe);
    Ok(super::transport_of(read, write))
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "S-1-5-21-1-2-3-1001";

    #[test]
    fn the_descriptor_admits_the_user_alone_and_labels_the_pipe_at_the_servers_level() {
        let server = Server {
            user: USER,
            level: 12288,
        };
        assert_eq!(
            description(server, false),
            format!("O:{USER}D:P(A;;GA;;;{USER})S:(ML;;NWNR;;;S-1-16-12288)")
        );
    }

    #[test]
    fn the_widened_descriptor_also_admits_everyone_below_a_low_label() {
        let server = Server {
            user: USER,
            level: 8192,
        };
        assert_eq!(
            description(server, true),
            format!("O:{USER}D:P(A;;GA;;;{USER})(A;;GRGW;;;WD)S:(ML;;NWNR;;;LW)")
        );
    }

    /// An identifier is read only as far as it is shown to lie in the bytes
    /// it is in: its fixed part, then the length its count of parts gives.
    #[test]
    fn an_identifier_is_read_only_within_its_bytes() {
        // `S-1-16-8192`: revision 1, one part, authority 16, the part 8192.
        let mut label = [1_u8, 1, 0, 0, 0, 0, 0, 16, 0, 0x20, 0, 0];
        let identifier: PSID = label.as_mut_ptr().cast();
        let start = identifier.addr();
        // SAFETY: the twelve bytes of `label` are readable, and alive and
        // unchanged for each call.
        unsafe {
            assert!(identifier_within(identifier, start, start + 12));
            assert_eq!(last_part(identifier).unwrap(), 8192);
            // Its part past the end.
            assert!(!identifier_within(identifier, start, start + 11));
            // Its fixed part past the end.
            assert!(!identifier_within(identifier, start, start + 7));
            // Before the bytes it is said to be in.
            assert!(!identifier_within(identifier, start + 1, start + 12));
            assert!(!identifier_within(ptr::null_mut(), start, start + 12));
        }

        // A count of parts that runs past the bytes.
        let mut two_parts = [1_u8, 2, 0, 0, 0, 0, 0, 16, 0, 0x20, 0, 0];
        let identifier: PSID = two_parts.as_mut_ptr().cast();
        let start = identifier.addr();
        // SAFETY: as above, for `two_parts`.
        assert!(!unsafe { identifier_within(identifier, start, start + 12) });

        // Not an identifier: a revision that is not one.
        let mut not_one = [2_u8, 1, 0, 0, 0, 0, 0, 16, 0, 0x20, 0, 0];
        let identifier: PSID = not_one.as_mut_ptr().cast();
        let start = identifier.addr();
        // SAFETY: as above, for `not_one`.
        assert!(!unsafe { identifier_within(identifier, start, start + 12) });
    }

    #[test]
    fn the_default_pipe_is_named_for_the_user() {
        let Place::Pipe(name) = default_place(None).unwrap() else {
            panic!("not a pipe");
        };
        let user = cineris_base::private::current_user_sid().unwrap();
        assert_eq!(name, format!(r"\\.\pipe\cineris-{user}"));
    }
}
