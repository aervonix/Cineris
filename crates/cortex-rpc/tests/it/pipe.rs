//! The named pipe on Windows: who may open it, and the check of each
//! connection's own token (`docs/steps/27/answered.md`, "The design, before
//! building", point 1, "Windows").
//!
//! A client here is the standard library's file opened on the pipe's name,
//! as any program of the user's would open it. The clients the pipe must
//! keep out (below the server's integrity, restricted, allowing only
//! anonymous impersonation, another user) are the same open, made from a
//! thread of the test that impersonates a token made for it.

#![allow(
    unsafe_code,
    reason = "a test makes the tokens of the clients the pipe refuses (a lower integrity, \
              restricting identifiers, a second user logged on), impersonates them, and reads \
              the pipe's security back: only the Windows API does these"
)]

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::hash::BuildHasher;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::mem::size_of;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::sync::Arc;

use cortex_base::CancelToken;
use cortex_rpc::{Accepted, ListenOptions, Listener, Place, ServeOptions};
use cortex_testkit::conformance::{self, Lines};
use cortex_testkit::rerun::{self, Rerun};
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
    ConvertStringSidToSidW, GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, DACL_SECURITY_INFORMATION, DuplicateTokenEx, GetLengthSid,
    GetTokenInformation, ImpersonateLoggedOnUser, LABEL_SECURITY_INFORMATION,
    LOGON32_LOGON_INTERACTIVE, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT, LogonUserW,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, RevertToSelf, SECURITY_MAX_SID_SIZE,
    SID_AND_ATTRIBUTES, SecurityImpersonation, SetTokenInformation, TOKEN_ADJUST_DEFAULT,
    TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenImpersonation,
    TokenIntegrityLevel,
};
use windows_sys::Win32::Storage::FileSystem::{READ_CONTROL, SECURITY_ANONYMOUS};
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
use windows_sys::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::{CoreThread, World};

/// A pipe of the test's own: the process's id keeps two runs apart.
fn pipe_of(test: &str) -> Place {
    Place::Pipe(format!(
        r"\\.\pipe\cortex-harness-test-{}-{test}",
        std::process::id()
    ))
}

fn name(place: &Place) -> &str {
    let Place::Pipe(name) = place else {
        panic!("not a pipe");
    };
    name
}

fn bind(place: &Place) -> io::Result<Listener> {
    Listener::bind(place, &ListenOptions::new(false))
}

/// A listener whose pipe admits everyone at any integrity, so that the
/// check of a connection's token alone keeps a client out.
fn bind_widened(place: &Place) -> Listener {
    Listener::bind(
        place,
        &ListenOptions {
            own_folder: false,
            widen_access_for_tests: true,
        },
    )
    .unwrap()
}

/// A runtime to accept on, for the tests that take connections one by one.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Opens the pipe to read and write, as a client of the user's.
fn open(place: &Place) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(name(place))
}

/// Serves `listener` on `core`'s thread until the returned token stops it.
fn serve(
    core: &CoreThread,
    listener: Listener,
    options: ServeOptions,
) -> (CancelToken, std::sync::mpsc::Receiver<()>) {
    let stop = CancelToken::new();
    let (finished, done) = std::sync::mpsc::channel();
    let (server, stopping) = (core.core.clone(), stop.clone());
    core.handle.spawn(async move {
        cortex_rpc::serve_socket(
            server,
            listener,
            options,
            64,
            &stopping,
            &CancelToken::new(),
        )
        .await;
        let _ = finished.send(());
    });
    (stop, done)
}

/// A client speaking lines over an open pipe. A pipe opened by the standard
/// library is not overlapped, so a read waiting on one thread would hold
/// back a write on another: the client reads and writes in turn, on the
/// thread that calls it.
fn client(pipe: File) -> Lines {
    let pipe = Arc::new(pipe);
    let mut reader = BufReader::new(Arc::clone(&pipe));
    Lines::new(
        move |bytes| {
            let _ = (&*pipe).write_all(&bytes);
        },
        move || {
            let mut line = Vec::new();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => None,
                Ok(_) => Some(line),
            }
        },
    )
}

/// Opens the pipe and asks `initialize`, from the calling thread: Windows
/// gives the server the token of the thread that wrote what it read.
fn said(place: &Place) -> io::Result<File> {
    let mut pipe = open(place)?;
    pipe.write_all(&initialize_line())?;
    Ok(pipe)
}

/// Waits, for two seconds at most, until an instance of the pipe waits
/// for a client: one the server makes while it handles another.
fn wait_for_instance(place: &Place) {
    let name = wide(name(place));
    // SAFETY: `name` is zero-terminated and alive for the call.
    let waited = unsafe { WaitNamedPipeW(name.as_ptr(), 2_000) };
    assert_ne!(
        waited,
        0,
        "no instance waited: {}",
        io::Error::last_os_error()
    );
}

/// The line that asks `initialize`.
fn initialize_line() -> Vec<u8> {
    let mut line = serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": 1,
        "method": "initialize", "params": conformance::initialize()}))
    .unwrap();
    line.push(b'\n');
    line
}

/// Whether the server end of `pipe` is closed with nothing written to it.
/// The runtime the server's end was in is driven meanwhile, as a server's
/// is: tokio's pipe, dropped with a read pending, lets its handle go only
/// once the runtime has seen the read cancelled.
fn closed_unanswered(runtime: &tokio::runtime::Runtime, pipe: &mut File) -> bool {
    std::thread::scope(|scope| {
        let (read, closed) = tokio::sync::oneshot::channel();
        scope.spawn(move || {
            let mut buffer = [0; 64];
            let _ = read.send(matches!(pipe.read(&mut buffer), Ok(0) | Err(_)));
        });
        runtime.block_on(closed).unwrap()
    })
}

/// A list of a descriptor's text (`P(A;;FA;;;LA)`): its flags, and each
/// entry's fields.
fn entries(list: &str) -> (&str, Vec<Vec<&str>>) {
    let (flags, rest) = list.split_at(list.find('(').unwrap_or(list.len()));
    let entries = rest
        .split(')')
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.trim_start_matches('(').split(';').collect())
        .collect();
    (flags, entries)
}

/// What a refused connection's reason was, or a panic naming what came.
fn refused(accepted: Accepted) -> String {
    match accepted {
        Accepted::Refused(why) => why,
        Accepted::Client { peer, .. } => panic!("the client was admitted ({peer})"),
    }
}

// ------------------------------------------------- the Windows API, for tests

/// Memory Windows allocated, given back once.
struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a Windows function that allocates
            // its result with `LocalAlloc` for the caller to free, and this
            // value is its one owner.
            unsafe { LocalFree(self.0) };
        }
    }
}

/// Zero-terminated UTF-16.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A zero-terminated UTF-16 string Windows allocated, as text, freed.
///
/// # Safety
///
/// `text` must be a zero-terminated UTF-16 string from `LocalAlloc`.
unsafe fn local_text(text: *mut u16) -> String {
    let memory = Local(text.cast());
    let mut length = 0;
    // SAFETY: the string ends with a zero, and each read is at or before it.
    while unsafe { text.add(length).read() } != 0 {
        length += 1;
    }
    // SAFETY: the `length` units before the zero were just read, and
    // `memory` keeps them alive past the slice's use.
    let units = unsafe { std::slice::from_raw_parts(text, length) };
    let converted = String::from_utf16(units).unwrap();
    drop(memory);
    converted
}

/// An identifier from its text (`S-1-16-12288`, or an alias such as `HI`).
fn identifier(text: &str) -> Local {
    let text = wide(text);
    let mut identifier: PSID = ptr::null_mut();
    // SAFETY: `text` is zero-terminated and alive for the call; `identifier`
    // is a live location for the pointer it returns.
    let converted = unsafe { ConvertStringSidToSidW(text.as_ptr(), &raw mut identifier) };
    assert_ne!(converted, 0, "{}", io::Error::last_os_error());
    Local(identifier)
}

/// An identifier's text in its long form: an alias such as `HI` as the
/// `S-1-16-12288` it stands for.
fn long_form(text: &str) -> String {
    let identifier = identifier(text);
    let mut out: *mut u16 = ptr::null_mut();
    // SAFETY: `identifier` is a valid identifier, alive for the call; `out`
    // is a live location for the string it returns.
    let converted = unsafe { ConvertSidToStringSidW(identifier.0, &raw mut out) };
    assert_ne!(converted, 0, "{}", io::Error::last_os_error());
    // SAFETY: the call succeeded, so `out` is a string Windows allocated.
    unsafe { local_text(out) }
}

/// The process's own token, open to query and to copy.
fn own_token() -> OwnedHandle {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: the pseudo-handle of the current process is always valid;
    // `token` is a live location for the handle it returns.
    let opened = unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE,
            &raw mut token,
        )
    };
    assert_ne!(opened, 0, "{}", io::Error::last_os_error());
    // SAFETY: the call succeeded: `token` is open and owned by nobody else.
    unsafe { OwnedHandle::from_raw_handle(token) }
}

/// The process's own integrity level, as its identifier's text.
fn own_level() -> String {
    #[repr(C)]
    struct LabelBuffer {
        label: TOKEN_MANDATORY_LABEL,
        identifier: [u8; SECURITY_MAX_SID_SIZE as usize],
    }
    let token = own_token();
    let mut buffer = LabelBuffer {
        label: TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: ptr::null_mut(),
                Attributes: 0,
            },
        },
        identifier: [0; SECURITY_MAX_SID_SIZE as usize],
    };
    let mut written = 0;
    // SAFETY: `token` is open to query; `buffer` is the structure this class
    // writes with room for its identifier after it, writable for its whole
    // size.
    let read = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            (&raw mut buffer).cast(),
            size_of::<LabelBuffer>() as u32,
            &raw mut written,
        )
    };
    assert_ne!(read, 0, "{}", io::Error::last_os_error());
    let mut out: *mut u16 = ptr::null_mut();
    // SAFETY: the label's identifier is valid and inside `buffer`, alive.
    let converted = unsafe { ConvertSidToStringSidW(buffer.label.Label.Sid, &raw mut out) };
    assert_ne!(converted, 0, "{}", io::Error::last_os_error());
    // SAFETY: the call succeeded, so `out` is a string Windows allocated.
    unsafe { local_text(out) }
}

/// A copy of the process's token for a thread to impersonate, at the
/// integrity level `level` (`S-1-16-<level>`), lower than its own.
fn own_token_at(level: u32) -> OwnedHandle {
    let own = own_token();
    let mut copy: HANDLE = ptr::null_mut();
    // SAFETY: `own` is open to copy; `copy` is a live location for the
    // handle the call returns; no attributes are given.
    let copied = unsafe {
        DuplicateTokenEx(
            own.as_raw_handle(),
            TOKEN_QUERY | TOKEN_IMPERSONATE | TOKEN_ADJUST_DEFAULT | TOKEN_DUPLICATE,
            ptr::null(),
            SecurityImpersonation,
            TokenImpersonation,
            &raw mut copy,
        )
    };
    assert_ne!(copied, 0, "{}", io::Error::last_os_error());
    // SAFETY: the call succeeded: `copy` is open and owned by nobody else.
    let copy = unsafe { OwnedHandle::from_raw_handle(copy) };
    let level = identifier(&format!("S-1-16-{level}"));
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: level.0,
            Attributes: SE_GROUP_INTEGRITY.cast_unsigned(),
        },
    };
    // SAFETY: `level` is a valid identifier, alive here.
    let length = unsafe { GetLengthSid(level.0) };
    // SAFETY: `copy` is open to adjust; `label` is the structure this class
    // takes, pointing to `level`, and both are alive for the call.
    let set = unsafe {
        SetTokenInformation(
            copy.as_raw_handle(),
            TokenIntegrityLevel,
            (&raw const label).cast(),
            size_of::<TOKEN_MANDATORY_LABEL>() as u32 + length,
        )
    };
    assert_ne!(set, 0, "{}", io::Error::last_os_error());
    copy
}

/// A restricted copy of the process's token: its restricting identifiers
/// are the user's and everyone's, so the pipe's access list still lets it
/// in, and only the check of its token can tell.
fn own_token_restricted() -> OwnedHandle {
    let own = own_token();
    let user = identifier(cortex_base::private::current_user_sid().unwrap());
    let everyone = identifier("S-1-1-0");
    let restricting = [
        SID_AND_ATTRIBUTES {
            Sid: user.0,
            Attributes: 0,
        },
        SID_AND_ATTRIBUTES {
            Sid: everyone.0,
            Attributes: 0,
        },
    ];
    let mut restricted: HANDLE = ptr::null_mut();
    // SAFETY: `own` is open to copy; `restricting` holds two valid
    // identifiers, alive for the call; nothing is disabled or deleted;
    // `restricted` is a live location for the handle the call returns.
    let made = unsafe {
        CreateRestrictedToken(
            own.as_raw_handle(),
            0,
            0,
            ptr::null(),
            0,
            ptr::null(),
            2,
            restricting.as_ptr(),
            &raw mut restricted,
        )
    };
    assert_ne!(made, 0, "{}", io::Error::last_os_error());
    // SAFETY: the call succeeded: `restricted` is open and owned by nobody
    // else.
    unsafe { OwnedHandle::from_raw_handle(restricted) }
}

/// Ends the thread's impersonation when dropped, on every path.
struct Reverting;

impl Drop for Reverting {
    fn drop(&mut self) {
        // SAFETY: ends this thread's impersonation; touches no memory.
        if unsafe { RevertToSelf() } == 0 {
            // A thread of the test that went on as another user would make
            // every later result a lie.
            std::process::abort();
        }
    }
}

/// Runs `act` on a thread that impersonates `token`, and stops after.
fn as_token<R: Send>(token: &OwnedHandle, act: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                // SAFETY: `token` is open with the rights impersonation
                // needs, and alive for the call.
                let impersonating = unsafe { ImpersonateLoggedOnUser(token.as_raw_handle()) };
                assert_ne!(impersonating, 0, "{}", io::Error::last_os_error());
                let _back = Reverting;
                act()
            })
            .join()
            .unwrap()
    })
}

/// The pipe's owner, access list and label, in Windows's description
/// language, read from a handle opened on its name with the right to read
/// them and nothing else.
fn security_of(place: &Place) -> String {
    let pipe = OpenOptions::new()
        .access_mode(READ_CONTROL)
        .open(name(place))
        .unwrap();
    let parts = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `pipe` is open with the right to read its security; only the
    // whole descriptor is asked for, into a live location; the other
    // outputs are null.
    let status = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_FILE_OBJECT,
            parts,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    assert_eq!(
        status,
        0,
        "{}",
        io::Error::from_raw_os_error(status.cast_signed())
    );
    let descriptor = Local(descriptor);
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: `descriptor` is the valid descriptor just read, alive for the
    // call; `text` is a live location for the string it returns.
    let converted = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.0,
            SDDL_REVISION_1,
            parts,
            &raw mut text,
            ptr::null_mut(),
        )
    };
    assert_ne!(converted, 0, "{}", io::Error::last_os_error());
    // SAFETY: the call succeeded, so `text` is a string Windows allocated.
    unsafe { local_text(text) }
}

// --------------------------------------------------------------- the tests

/// A client of the user's opens the pipe and is served `initialize` by the
/// server, as `serve --socket` serves it.
#[test]
fn a_client_of_the_user_is_served_over_the_pipe() {
    let world = World::new("pipe-served");
    let core = world.core();
    let place = pipe_of("served");
    let listener = bind(&place).unwrap();
    let (stop, done) = serve(&core, listener, world.serve_options());
    let mut lines = client(open(&place).expect("the user opens the pipe"));
    lines
        .initialize(conformance::initialize())
        .expect("initialize over the pipe");
    drop(lines);
    stop.cancel();
    done.recv().unwrap();
}

/// A second server at a name in use is refused, and so is a server at a
/// name another process holds with a pipe of its own; the name is free
/// again once the first server is gone.
#[test]
fn a_name_in_use_is_refused() {
    let place = pipe_of("in-use");
    let first = bind(&place).unwrap();
    let second = bind(&place).unwrap_err();
    assert_eq!(second.kind(), io::ErrorKind::AddrInUse, "{second}");
    assert!(second.to_string().contains(name(&place)), "{second}");
    drop(first);
    drop(bind(&place).expect("the name is free once the server is gone"));

    // A squatter: a pipe of the name made by another program, with the
    // system's own security.
    let runtime = runtime();
    let squatter = runtime.block_on(async {
        tokio::net::windows::named_pipe::ServerOptions::new()
            .create(name(&place))
            .unwrap()
    });
    let refused = bind(&place).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::AddrInUse, "{refused}");
    drop(squatter);
}

/// The pipe is owned by the user, its access list is protected and allows
/// the user alone, and its label is the server's own integrity, with no
/// write up and no read up.
#[test]
fn the_pipe_admits_the_user_alone_at_the_servers_integrity() {
    let place = pipe_of("security");
    let _listener = bind(&place).unwrap();
    let text = security_of(&place);
    let user = cortex_base::private::current_user_sid().unwrap();
    let (owner, rest) = text
        .strip_prefix("O:")
        .and_then(|rest| rest.split_once("D:"))
        .unwrap_or_else(|| panic!("no owner and access list: {text}"));
    let (list, label) = rest
        .split_once("S:")
        .unwrap_or_else(|| panic!("no label: {text}"));
    assert_eq!(long_form(owner), user, "the owner: {text}");
    let (flags, allowed) = entries(list);
    assert!(
        flags.contains('P'),
        "the access list is not protected: {text}"
    );
    let [entry] = allowed.as_slice() else {
        panic!("not one entry: {text}");
    };
    assert_eq!(entry[..5], ["A", "", "FA", "", ""], "the entry: {text}");
    assert_eq!(long_form(entry[5]), user, "the entry's user: {text}");
    let (_, labels) = entries(label);
    let [label] = labels.as_slice() else {
        panic!("not one label: {text}");
    };
    assert_eq!(label[..5], ["ML", "", "NWNR", "", ""], "the label: {text}");
    assert_eq!(
        long_form(label[5]),
        own_level(),
        "the label's level: {text}"
    );
}

/// A client that has sent nothing cannot be checked yet, since Windows
/// gives the server a client's token only once it has read from it, and it
/// holds up no other: a second client, which opens on the instance the
/// server made when the first connected, is admitted, with what it sent
/// first; and an instance, made when the second was handed on, waits for a
/// third client before the server is asked for another.
#[test]
fn a_silent_client_holds_up_no_other_and_the_name_never_lapses() {
    let place = pipe_of("silent");
    let mut listener = bind(&place).unwrap();
    let runtime = runtime();
    let _silent = open(&place).expect("the first client opens");
    let accepted = std::thread::scope(|scope| {
        let accepting = scope.spawn(|| runtime.block_on(listener.accept()).unwrap());
        wait_for_instance(&place);
        let mut second = open(&place).expect("the second client opens");
        second.write_all(b"second\n").unwrap();
        accepting.join().unwrap()
    });
    let Accepted::Client {
        mut transport,
        peer,
    } = accepted
    else {
        panic!("the second client was refused: {accepted:?}");
    };
    assert!(peer.starts_with("pid ") && peer != "pid unknown", "{peer}");
    let first = runtime.block_on(transport.input.recv());
    assert_eq!(
        first.map(Result::unwrap).as_deref(),
        Some(&b"second\n"[..]),
        "what the client sent first"
    );
    let _third = open(&place).expect("no instance waited after a connection was handed on");
}

/// Clients that send nothing cannot hold the pipe's instances: one more
/// than the server lets wait (16) closes the one that has waited longest.
#[test]
fn the_longest_silent_client_is_closed_to_make_room() {
    let place = pipe_of("crowd");
    let mut listener = bind(&place).unwrap();
    let runtime = runtime();
    let (accepted, mut silent) = std::thread::scope(|scope| {
        let accepting = scope.spawn(|| runtime.block_on(listener.accept()).unwrap());
        let silent: Vec<File> = (0..17)
            .map(|_| {
                wait_for_instance(&place);
                open(&place).expect("a silent client opens")
            })
            .collect();
        (accepting.join().unwrap(), silent)
    });
    let why = refused(accepted);
    assert!(why.contains("sent nothing"), "{why}");
    assert!(
        closed_unanswered(&runtime, &mut silent[0]),
        "the oldest was not closed"
    );
}

/// A client below the server's integrity cannot open the pipe, to read and
/// write or to read only (its label). Where the label lets it in, on the
/// widened pipe, the check of its token refuses it, and closes it with
/// nothing sent.
#[test]
fn a_client_below_the_servers_integrity_is_kept_out() {
    let place = pipe_of("integrity");
    let _listener = bind(&place).unwrap();
    let low = own_token_at(4096);
    let opens = as_token(&low, || {
        [
            open(&place),
            OpenOptions::new().read(true).open(name(&place)),
        ]
    });
    for opened in opens {
        let denied = opened.expect_err("a lower client opened the pipe");
        assert_eq!(
            denied.raw_os_error(),
            Some(ERROR_ACCESS_DENIED.cast_signed()),
            "{denied}"
        );
    }

    let widened = pipe_of("integrity-widened");
    let mut listener = bind_widened(&widened);
    let mut pipe = as_token(&low, || said(&widened)).expect("the widened pipe lets it in");
    let runtime = runtime();
    let why = refused(runtime.block_on(listener.accept()).unwrap());
    assert!(why.contains("integrity"), "{why}");
    assert!(closed_unanswered(&runtime, &mut pipe));
}

/// A client whose token is restricted is refused by the check of its token,
/// though the pipe's access list let it open the pipe.
#[test]
fn a_restricted_client_is_refused() {
    let place = pipe_of("restricted");
    let mut listener = bind(&place).unwrap();
    let restricted = own_token_restricted();
    let mut pipe = as_token(&restricted, || said(&place)).expect("a restricted client opens");
    let runtime = runtime();
    let why = refused(runtime.block_on(listener.accept()).unwrap());
    assert!(why.contains("restricted"), "{why}");
    assert!(closed_unanswered(&runtime, &mut pipe));
}

/// A client that allows the server only anonymous impersonation has no
/// token to read, and is refused.
#[test]
fn an_anonymous_client_is_refused() {
    let place = pipe_of("anonymous");
    let mut listener = bind(&place).unwrap();
    let mut pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .security_qos_flags(SECURITY_ANONYMOUS)
        .open(name(&place))
        .expect("an anonymous client opens");
    pipe.write_all(&initialize_line()).unwrap();
    let runtime = runtime();
    let why = refused(runtime.block_on(listener.accept()).unwrap());
    assert!(why.contains("token"), "{why}");
    assert!(closed_unanswered(&runtime, &mut pipe));
}

// ----------------------------------------------------------- a second user

/// A local user made for one test, deleted when it is dropped.
struct OtherUser {
    name: String,
}

impl Drop for OtherUser {
    fn drop(&mut self) {
        let _ = std::process::Command::new("net")
            .args(["user", &self.name, "/delete"])
            .output();
    }
}

/// Makes a second user with a random password and logs it on, or says why
/// it cannot (no administrator's rights).
fn other_user() -> Result<(OtherUser, OwnedHandle), String> {
    let random = std::collections::hash_map::RandomState::new();
    let name = format!("cxpipe{:08x}", random.hash_one("name") & 0xffff_ffff);
    // Fourteen characters at most: `net user` asks before it takes a longer
    // one. Upper case, lower case, a sign and digits, for any policy.
    let password = format!("Cx!{:010x}a", random.hash_one("password") & 0xff_ffff_ffff);
    let added = std::process::Command::new("net")
        .args(["user", &name, &password, "/add"])
        .output()
        .map_err(|error| format!("`net user` did not start: {error}"))?;
    if !added.status.success() {
        return Err(format!(
            "`net user /add` failed: {}{}",
            String::from_utf8_lossy(&added.stdout),
            String::from_utf8_lossy(&added.stderr)
        ));
    }
    let user = OtherUser { name };
    let (wide_name, domain, wide_password) = (wide(&user.name), wide("."), wide(&password));
    let mut failures = Vec::new();
    for kind in [LOGON32_LOGON_NETWORK, LOGON32_LOGON_INTERACTIVE] {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: the three strings are zero-terminated and alive for the
        // call; `token` is a live location for the handle it returns.
        let logged_on = unsafe {
            LogonUserW(
                wide_name.as_ptr(),
                domain.as_ptr(),
                wide_password.as_ptr(),
                kind,
                LOGON32_PROVIDER_DEFAULT,
                &raw mut token,
            )
        };
        if logged_on != 0 {
            // SAFETY: the call succeeded: `token` is open and owned by
            // nobody else.
            return Ok((user, unsafe { OwnedHandle::from_raw_handle(token) }));
        }
        failures.push(format!("logon {kind}: {}", io::Error::last_os_error()));
    }
    Err(format!(
        "the second user was not logged on: {}",
        failures.join("; ")
    ))
}

/// A client the server started (here a child of the test process, which is
/// the server's) is one of the server's own commands: though it is the
/// user's, at the server's integrity, its connection is closed unanswered.
/// The child is this test run again: it opens the pipe, asks `initialize`,
/// and says what it read. The server's own thread is served after it.
#[test]
fn sys_one_of_the_servers_own_commands_is_refused() {
    if rerun::is_child() {
        let name = std::env::var("CX_PIPE").unwrap();
        let mut pipe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
            .unwrap();
        pipe.write_all(&initialize_line()).unwrap();
        let mut buffer = [0; 256];
        let read = match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => "closed".to_owned(),
            Ok(count) => String::from_utf8_lossy(&buffer[..count]).into_owned(),
        };
        rerun::say("read", &read);
        return;
    }
    let world = World::new("pipe-own-command");
    let core = world.core();
    let place = pipe_of("own-command");
    let listener = bind(&place).unwrap();
    let (stop, done) = serve(&core, listener, world.serve_options());
    let ran = Rerun::this_test().env("CX_PIPE", name(&place)).run();
    assert_eq!(
        ran.said("read").as_deref(),
        Some("closed"),
        "the server's own command was answered"
    );
    let mut lines = client(open(&place).expect("the server's own thread opens the pipe"));
    lines
        .initialize(conformance::initialize())
        .expect("the server's own thread is served");
    drop(lines);
    stop.cancel();
    done.recv().unwrap();
}

/// Another user cannot open the pipe, while the test's own user is served:
/// its access list keeps the other user out, and so does its label where
/// the test runs at a higher integrity than the other user's logon (a
/// runner's administrator, elevated). Against a pipe whose access list
/// admits everyone and whose label is low, the other user opens it and the
/// check of its token alone refuses it, closing the connection unanswered.
///
/// It needs an administrator's rights to make the user: where `CI` is set
/// it must run, and elsewhere it says it was not run and passes.
#[test]
fn sys_another_user_cannot_open_the_pipe() {
    let (_user, token) = match other_user() {
        Ok(made) => made,
        Err(why) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "a second user is needed on a runner: {why}"
            );
            eprintln!("not run: {why}");
            return;
        }
    };
    let world = World::new("pipe-other-user");
    let core = world.core();
    let place = pipe_of("other-user");
    let listener = bind(&place).unwrap();
    let (stop, done) = serve(&core, listener, world.serve_options());
    let mut own = client(open(&place).expect("the test's own user opens the pipe"));
    own.initialize(conformance::initialize())
        .expect("the test's own user is served");
    drop(own);
    let denied = as_token(&token, || open(&place)).expect_err("another user opened the pipe");
    assert_eq!(
        denied.raw_os_error(),
        Some(ERROR_ACCESS_DENIED.cast_signed()),
        "{denied}"
    );
    stop.cancel();
    done.recv().unwrap();

    let widened = pipe_of("other-user-widened");
    let mut listener = bind_widened(&widened);
    let mut pipe = as_token(&token, || said(&widened)).expect("everyone may open the widened pipe");
    let runtime = runtime();
    let why = refused(runtime.block_on(listener.accept()).unwrap());
    assert!(why.contains("another user"), "{why}");
    assert!(closed_unanswered(&runtime, &mut pipe));
}
