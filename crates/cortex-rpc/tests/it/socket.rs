//! The local socket on Unix: where it may be made, the place secured, a
//! stale socket and a live one, several clients at once, and the bounds on
//! connections (`docs/steps/27/answered.md`, "The design, before
//! building", point 1). Another user's refusal needs a second user, and is
//! `cortex-cli`'s system test.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cortex_base::CancelToken;
use cortex_protocol::methods::{Op, OpResult, SessionNewParams, SessionRef};
use cortex_rpc::{ListenOptions, Listener, Place, ServeOptions};
use cortex_testkit::conformance::{self, Carrier, Lines};
use serde_json::Map;

use crate::{CoreThread, World};

/// A folder of the test's own, of `mode`.
fn folder(world: &World, name: &str, mode: u32) -> PathBuf {
    let folder = world.root.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode)).unwrap();
    folder
}

fn bind(path: &Path, own_folder: bool) -> std::io::Result<Listener> {
    Listener::bind(
        &Place::Path(path.to_path_buf()),
        &ListenOptions {
            own_folder,
            ..ListenOptions::default()
        },
    )
}

/// Serves `listener` on `core`'s thread until the returned token stops it,
/// with `options`, at most `most` clients at once.
fn serve(
    core: &CoreThread,
    listener: Listener,
    options: ServeOptions,
    most: usize,
) -> (CancelToken, std::sync::mpsc::Receiver<()>) {
    let stop = CancelToken::new();
    let (finished, done) = std::sync::mpsc::channel();
    let (server, stopping) = (core.core.clone(), stop.clone());
    core.handle.spawn(async move {
        cortex_rpc::serve_socket(
            server,
            listener,
            options,
            most,
            &stopping,
            &CancelToken::new(),
        )
        .await;
        let _ = finished.send(());
    });
    (stop, done)
}

/// A client of the socket at `path`, speaking lines.
fn client(path: &Path) -> Lines {
    client_and_stream(path).0
}

/// A client of the socket at `path`, and its stream, to end it with.
fn client_and_stream(path: &Path) -> (Lines, UnixStream) {
    let stream = UnixStream::connect(path).expect("the socket takes a client");
    let kept = stream.try_clone().unwrap();
    let mut writing = stream.try_clone().unwrap();
    let (lines_read, read) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = Vec::new();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    if lines_read.send(line).is_err() {
                        return;
                    }
                }
            }
        }
    });
    let lines = Lines::new(
        move |bytes| {
            let _ = writing.write_all(&bytes);
        },
        move || read.recv().ok(),
    );
    (lines, kept)
}

fn new_session(workspace: &Path) -> Op {
    Op::SessionNew(SessionNewParams {
        workspace: workspace.to_str().unwrap().to_owned(),
        settings: cortex_protocol::session::SessionSettings::default(),
        title: None,
        config: Map::new(),
        ephemeral: false,
    })
}

/// The socket is the user's alone, a lock beside it is held, and both are
/// gone (the socket) or let go (the lock) when the listener goes.
#[test]
fn the_socket_is_the_users_alone_and_is_removed_at_the_end() {
    let world = World::new("socket-mode");
    let path = folder(&world, "named", 0o755).join("rpc.sock");
    let listener = bind(&path, false).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "the socket's mode");
    let lock = PathBuf::from(format!("{}.lock", path.display()));
    let lock_mode = std::fs::metadata(&lock).unwrap().permissions().mode() & 0o777;
    assert_eq!(lock_mode & 0o077, 0, "the lock's mode");
    drop(listener);
    assert!(!path.exists(), "the socket was left behind");
    // The lock was let go: a second server may listen there now.
    drop(bind(&path, false).unwrap());
}

/// A second server at the same place is refused while the first listens; a
/// socket no server answers on is removed; anything that is not a socket
/// is left alone and refuses the start.
#[test]
fn a_place_in_use_is_refused_and_a_stale_socket_cleared() {
    let world = World::new("socket-stale");
    let named = folder(&world, "named", 0o755);
    let path = named.join("rpc.sock");
    let first = bind(&path, false).unwrap();
    let second = bind(&path, false).unwrap_err();
    assert_eq!(second.kind(), std::io::ErrorKind::AddrInUse, "{second}");
    drop(first);

    // A socket left by a server that is gone, and no lock held.
    let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
    drop(stale);
    assert!(path.exists());
    drop(bind(&path, false).expect("a stale socket is cleared"));

    // A socket a live server answers on, with its lock gone (a cleaner of
    // the folder removed it): the live one is kept.
    let live = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let kept = bind(&path, false).unwrap_err();
    assert_eq!(kept.kind(), std::io::ErrorKind::AddrInUse, "{kept}");
    drop(live);
    std::fs::remove_file(&path).unwrap();

    std::fs::write(&path, "not a socket").unwrap();
    let refused = bind(&path, false).unwrap_err();
    assert!(refused.to_string().contains("not a socket"), "{refused}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a socket");
}

/// The server's own folder is made owner-only, and refused when it exists
/// open to others; a folder the user names must not be writable by others.
#[test]
fn the_folder_is_secured_before_the_socket_exists() {
    let world = World::new("socket-folder");
    let own = world.root.join("own");
    drop(bind(&own.join("rpc.sock"), true).expect("the own folder is made"));
    let mode = std::fs::metadata(&own).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    let open = folder(&world, "open", 0o755);
    let refused = bind(&open.join("rpc.sock"), true).unwrap_err();
    assert_eq!(
        refused.kind(),
        std::io::ErrorKind::PermissionDenied,
        "{refused}"
    );
    let shared = folder(&world, "shared", 0o777);
    let refused = bind(&shared.join("rpc.sock"), false).unwrap_err();
    assert_eq!(
        refused.kind(),
        std::io::ErrorKind::PermissionDenied,
        "{refused}"
    );
    assert!(
        !shared.join("rpc.sock").exists(),
        "a socket was made where others write"
    );
}

/// Two clients over the socket on one session: the second opens the
/// session the first started, and both are sent its turn.
#[test]
fn two_clients_share_a_session_over_the_socket() {
    let world = World::new("socket-two");
    world.server.push(conformance::anthropic::text(&["Hello."]));
    let core = world.core();
    let path = folder(&world, "named", 0o700).join("rpc.sock");
    let listener = bind(&path, false).unwrap();
    let (stop, done) = serve(&core, listener, world.serve_options(), 64);
    let mut first = client(&path);
    let mut second = client(&path);
    first.initialize(conformance::initialize()).unwrap();
    second.initialize(conformance::initialize()).unwrap();
    let OpResult::SessionNew(opened) = first.call(new_session(&world.workspace)).unwrap() else {
        panic!("not a session");
    };
    let session = opened.session.session_id;
    second
        .call(Op::SessionOpen(SessionRef {
            session_id: session.clone(),
        }))
        .expect("the second client joins");
    first
        .call(Op::TurnStart(cortex_protocol::methods::TurnStartParams {
            session_id: session,
            input: vec![cortex_protocol::Part::text("hi")],
            delivery: None,
        }))
        .unwrap();
    for lines in [&mut first, &mut second] {
        loop {
            if let Some(conformance::Inbound::Event(event)) = lines.next()
                && matches!(event.event, cortex_protocol::event::Event::TurnCompleted(_))
            {
                break;
            }
        }
    }
    stop.cancel();
    done.recv().unwrap();
    assert!(!path.exists(), "the socket outlived the server");
}

/// A connection that never says `initialize` is closed at its bound; one
/// past the most at once is closed as it comes.
#[test]
fn silent_connections_and_too_many_are_closed() {
    let world = World::new("socket-bounds");
    let core = world.core();
    let path = folder(&world, "named", 0o700).join("rpc.sock");
    let listener = bind(&path, false).unwrap();
    let mut options = world.serve_options();
    options.initialize_within = Some(Duration::from_secs(10));
    let (stop, done) = serve(&core, listener, options, 1);
    let mut silent = client(&path);
    // The one place taken, a second connection is closed unread.
    let mut extra = client(&path);
    assert!(
        extra.read().is_none(),
        "a connection past the most was served"
    );
    cortex_testkit::block_on(world.timer.sleeping_for(Duration::from_secs(10)));
    world.timer.advance(Duration::from_secs(10));
    assert!(silent.read().is_none(), "a silent connection was kept");
    stop.cancel();
    done.recv().unwrap();
}

/// A client's `shutdown` ends the server: every connection, and the socket.
#[test]
fn a_clients_shutdown_ends_the_server() {
    let world = World::new("socket-shutdown");
    let core = world.core();
    let path = folder(&world, "named", 0o700).join("rpc.sock");
    let listener = bind(&path, false).unwrap();
    let (_stop, done) = serve(&core, listener, world.serve_options(), 64);
    let mut other = client(&path);
    other.initialize(conformance::initialize()).unwrap();
    let mut asking = client(&path);
    asking.initialize(conformance::initialize()).unwrap();
    let stopped = asking
        .call(Op::Shutdown(cortex_protocol::methods::Empty {}))
        .unwrap();
    assert_eq!(stopped.result().unwrap(), serde_json::json!({}));
    assert!(
        other.read().is_none(),
        "another client's connection was kept"
    );
    done.recv().unwrap();
    assert!(!path.exists());
}

/// A peer whose line of parents reaches the server's process is one of the
/// server's own commands, and is closed unanswered: here a child of the test
/// process, which is the server's. Python, on every Linux image, is the
/// child: it connects, says `initialize`, and prints what it is sent.
#[test]
fn sys_one_of_the_servers_own_commands_is_refused() {
    let world = World::new("socket-own-command");
    let core = world.core();
    let path = folder(&world, "named", 0o700).join("rpc.sock");
    let listener = bind(&path, false).unwrap();
    let (stop, done) = serve(&core, listener, world.serve_options(), 64);
    let initialize = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": conformance::initialize()});
    // A refused connection may come back reset rather than ended: both are
    // a connection closed with nothing sent.
    let script = "import socket, sys\n\
        s = socket.socket(socket.AF_UNIX)\n\
        s.connect(sys.argv[1])\n\
        try:\n    s.sendall(sys.argv[2].encode() + b'\\n')\n    data = s.recv(4096)\n\
        except OSError:\n    data = b''\n\
        sys.stdout.write(repr(data))\n";
    let output = std::process::Command::new("python3")
        .args([
            "-c",
            script,
            path.to_str().unwrap(),
            &initialize.to_string(),
        ])
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "python3 is not on this runner: {error}"
            );
            eprintln!("not run: python3 cannot be started here ({error})");
            stop.cancel();
            done.recv().unwrap();
            return;
        }
    };
    let said = String::from_utf8_lossy(&output.stdout);
    assert_eq!(said, "b''", "the server's own command was answered: {said}");
    // The server goes on: a client that is not its command is served.
    let mut lines = client(&path);
    assert!(lines.initialize(conformance::initialize()).is_ok());
    stop.cancel();
    done.recv().unwrap();
}

/// A connection gives its place back when it stops serving: at the most at
/// once, the place of a client that has gone is the next client's.
#[test]
fn the_place_of_a_client_that_went_is_the_next_ones() {
    let world = World::new("socket-place");
    let core = world.core();
    let path = folder(&world, "named", 0o700).join("rpc.sock");
    let listener = bind(&path, false).unwrap();
    let (stop, done) = serve(&core, listener, world.serve_options(), 1);
    let (mut first, stream) = client_and_stream(&path);
    first
        .initialize(conformance::initialize())
        .expect("initialize");
    // Its input ends; the server ends the connection, and closes it.
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    assert!(first.read().is_none(), "the server wrote past the end");
    let mut second = client(&path);
    second
        .initialize(conformance::initialize())
        .expect("the place the first client left is the second's");
    stop.cancel();
    done.recv().unwrap();
}

/// An `accept` that fails for want of a resource is tried again after a
/// wait that doubles, and the client waiting is served once one passes.
#[test]
fn an_accept_that_fails_for_want_of_a_resource_is_tried_again() {
    let world = World::new("socket-retry");
    let core = world.core();
    let path = folder(&world, "named", 0o700).join("rpc.sock");
    let failing = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(2));
    let mut options = ListenOptions::new(false);
    options.failing_accepts = Some(std::sync::Arc::clone(&failing));
    let listener = Listener::bind(&Place::Path(path.clone()), &options).unwrap();
    let (stop, done) = serve(&core, listener, world.serve_options(), 64);
    let mut lines = client(&path);
    // Two failures, each waited out on the timer: 50 ms, then 100 ms.
    for wait in [Duration::from_millis(50), Duration::from_millis(100)] {
        cortex_testkit::block_on(world.timer.sleeping_for(wait));
        world.timer.advance(wait);
    }
    lines
        .initialize(conformance::initialize())
        .expect("the client is served once an accept passes");
    assert_eq!(failing.load(std::sync::atomic::Ordering::SeqCst), 0);
    stop.cancel();
    done.recv().unwrap();
}

/// A peer whose line of parents cannot be read is refused: here the
/// process that connected handed the socket to a child of its own and
/// exited before the server looked (the independent review of step 27's
/// second part). Held at the point before the look, the test waits for that
/// process to be gone; then the child speaks, and is closed unanswered.
#[test]
fn sys_a_peer_whose_process_is_gone_is_refused() {
    let world = World::new("socket-gone");
    let core = world.core();
    let path = folder(&world, "named", 0o700).join("rpc.sock");
    let listener = bind(&path, false).unwrap();
    let looking = cortex_testkit::Hold::new(cortex_rpc::socket::BEFORE_A_PEERS_PARENTS_ARE_READ);
    let (stop, done) = serve(&core, listener, world.serve_options(), 64);
    let initialize = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": conformance::initialize()});
    // The child waits for its standard input to end before it speaks.
    let script = "import os, socket, sys\n\
        s = socket.socket(socket.AF_UNIX)\n\
        s.connect(sys.argv[1])\n\
        if os.fork() != 0:\n    os._exit(0)\n\
        sys.stdin.read()\n\
        try:\n    s.sendall(sys.argv[2].encode() + b'\\n')\n    data = s.recv(4096)\n\
        except OSError:\n    data = b''\n\
        sys.stdout.write(repr(data))\n\
        sys.stdout.flush()\n";
    let started = std::process::Command::new("python3")
        .args([
            "-c",
            script,
            path.to_str().unwrap(),
            &initialize.to_string(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn();
    let mut connecting = match started {
        Ok(child) => child,
        Err(error) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "python3 is not on this runner: {error}"
            );
            eprintln!("not run: python3 cannot be started here ({error})");
            looking.release();
            stop.cancel();
            done.recv().unwrap();
            return;
        }
    };
    looking.reach();
    // The process that connected is gone, and reaped.
    assert!(connecting.wait().unwrap().success());
    looking.release();
    drop(connecting.stdin.take());
    let mut said = String::new();
    std::io::Read::read_to_string(&mut connecting.stdout.take().unwrap(), &mut said).unwrap();
    assert_eq!(
        said, "b''",
        "a peer whose process was gone was served: {said}"
    );
    assert_eq!(
        looking.timed_out(),
        0,
        "the server was let go, not timed out"
    );
    stop.cancel();
    done.recv().unwrap();
}
