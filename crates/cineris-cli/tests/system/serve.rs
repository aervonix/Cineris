//! `cineris serve`: the real binary as an RPC server, over standard I/O and
//! a local socket (`docs/architecture/11-rpc-protocol.md`;
//! `docs/steps/27/answered.md`, points 1, 4 and 6).
//!
//! The conformance scenarios of `cineris_testkit::conformance` run through
//! the binary and through the same core in this process, against the same
//! scripted server, and what the client was sent must be the same. Every
//! line the binary writes on standard output is read as a JSON-RPC message:
//! the stream carries nothing else.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Stdio};
use std::sync::{Arc, Condvar, Mutex};

use cineris_base::CancelToken;
use cineris_config::{Env, Platform};
use cineris_core::{ClientId, ClientInfo, Core, CoreOptions, EventQueue, Outgoing, TokioTimer};
use cineris_protocol::ErrorData;
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::methods::{Op, OpResult, ServerResponse};
use cineris_rpc::lane::{Dispatcher, Run};
use cineris_sandbox::tree::{Exit, Tree};
use cineris_testkit::conformance::{self, Carrier, Inbound, Lines, Transcript};

use crate::Machine;

/// The binary serving its standard I/O, and a client that speaks lines to
/// it.
struct Served {
    child: Child,
    lines: Lines,
}

fn serve(machine: &Machine) -> Served {
    let mut command = machine.command(&["serve", "--stdio"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(
            File::create(machine.home.join("stderr.txt")).unwrap(),
        ));
    let mut child = command.spawn().expect("the binary starts");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (lines_read, read) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
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
            let _ = stdin.write_all(&bytes).and_then(|()| stdin.flush());
        },
        move || read.recv().ok(),
    );
    Served { child, lines }
}

impl Served {
    /// Ends standard input, reads every line left as a message, and waits
    /// for the exit.
    fn end(mut self) -> Option<i32> {
        self.lines.end_input();
        while self.lines.read().is_some() {}
        self.child.wait().unwrap().code()
    }
}

/// A core in this process, as the binary makes one for the same machine.
struct InProcess {
    core: Core,
    handle: tokio::runtime::Handle,
    client: ClientId,
    queue: Arc<EventQueue>,
    bell: Arc<(Mutex<bool>, Condvar)>,
    dispatcher: Dispatcher<u64>,
    results: std::sync::mpsc::Receiver<(u64, Result<OpResult, ErrorData>)>,
    /// Results that came while another was awaited.
    early: std::collections::BTreeMap<u64, Result<OpResult, ErrorData>>,
    next: u64,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

fn on<T: Send + 'static>(
    handle: &tokio::runtime::Handle,
    future: impl std::future::Future<Output = T> + Send + 'static,
) -> T {
    let (sent, output) = std::sync::mpsc::channel();
    handle.spawn(async move {
        let _ = sent.send(future.await);
    });
    output.recv().unwrap()
}

impl InProcess {
    fn start(machine: &Machine) -> Self {
        let mut env = Env::empty(Platform::CURRENT);
        for (name, value) in machine.vars() {
            env.set(name, value.to_str().unwrap());
        }
        let mut options = CoreOptions::new(env, Arc::new(TokioTimer));
        options.dirs = Some(machine.dirs());
        options.harness_version = "0.0.0-test".to_owned();
        options.cancel = CancelToken::new();
        // As the binary makes its core: the sandbox's launcher and the
        // guard of process trees are the binary's hidden subcommands, and
        // an answer may stop the turn.
        let binary = std::path::PathBuf::from(crate::CINERIS);
        options.launcher = Some((binary.clone(), vec!["__sandbox".into()]));
        options.guard = Some((binary, vec!["__tree-guard".into()]));
        options.answers_may_stop = true;
        let (handed, handle) = std::sync::mpsc::channel();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            handed.send(runtime.handle().clone()).unwrap();
            runtime.block_on(async {
                let _ = stopped.await;
            });
        });
        let handle: tokio::runtime::Handle = handle.recv().unwrap();
        let core = on(&handle, async move { Core::start(options) }).expect("the core starts");
        let params = conformance::initialize();
        let client = core.connect(ClientInfo {
            role: params.role.clone(),
            capabilities: params.capabilities.clone(),
            queue: cineris_core::QUEUE_CAPACITY,
        });
        let first = core.clone();
        on(&handle, async move {
            first.handle(client.id, Op::Initialize(params)).await
        })
        .expect("initialize");
        let bell = Arc::new((Mutex::new(false), Condvar::new()));
        let rung = Arc::clone(&bell);
        client.queue.set_wake(Arc::new(move || {
            *rung.0.lock().unwrap() = true;
            rung.1.notify_all();
        }));
        let (done, results) = std::sync::mpsc::channel();
        let done = Mutex::new(done);
        let (runs, id) = (core.clone(), client.id);
        let run: Run = Arc::new(move |op| {
            let core = runs.clone();
            Box::pin(async move { core.handle(id, op).await })
        });
        let dispatcher = on(&handle, async move {
            Dispatcher::start(
                run,
                Arc::new(move |ticket, result| {
                    let _ = done.lock().unwrap().send((ticket, result));
                }),
            )
        });
        Self {
            core,
            handle,
            client: client.id,
            queue: client.queue,
            bell,
            dispatcher,
            results,
            early: std::collections::BTreeMap::new(),
            next: 0,
            stop: Some(stop),
        }
    }
}

impl Drop for InProcess {
    fn drop(&mut self) {
        let core = self.core.clone();
        on(&self.handle, async move { core.shutdown().await });
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Carrier for InProcess {
    fn start(&mut self, op: Op) -> u64 {
        self.next += 1;
        let _inside = self.handle.enter();
        self.dispatcher.send(self.next, op);
        self.next
    }

    fn finish(&mut self, ticket: u64) -> Result<OpResult, ErrorData> {
        loop {
            if let Some(result) = self.early.remove(&ticket) {
                return result;
            }
            let (answered, result) = self.results.recv().unwrap();
            self.early.insert(answered, result);
        }
    }

    fn next(&mut self) -> Option<Inbound> {
        loop {
            match self.queue.try_next() {
                Some(Outgoing::Event(event)) => return Some(Inbound::Event(event)),
                Some(Outgoing::Request { id, request }) => {
                    return Some(Inbound::Request(id, request));
                }
                None if self.queue.is_closed() => return None,
                None => {
                    let mut rung = self.bell.0.lock().unwrap();
                    while !*rung {
                        rung = self.bell.1.wait(rung).unwrap();
                    }
                    *rung = false;
                }
            }
        }
    }

    fn respond(&mut self, id: RequestId, response: ServerResponse) {
        self.core.respond(self.client, &id, response).unwrap();
    }

    fn forget(&mut self) {
        while self.queue.try_next().is_some() {}
    }
}

/// Runs every scenario on one machine, through the carrier `carry` makes,
/// each scenario in a workspace of its own (`work/sNN`): one core, or one
/// binary, a carrier.
fn transcripts(
    test: &str,
    carry: impl FnOnce(
        &Machine,
        &mut dyn FnMut(&mut dyn Carrier) -> Vec<(&'static str, Transcript)>,
    ) -> Vec<(&'static str, Transcript)>,
) -> Vec<(&'static str, Transcript)> {
    // The repository map is background work whose notice comes when it
    // likes: off on both sides, as the scenarios ask.
    let machine = Machine::with_config(test, "index.map = false\n");
    let root = machine.home.parent().unwrap().to_str().unwrap().to_owned();
    let server = machine.server.base_url();
    let work = machine.home.parent().unwrap().join("work");
    let mut run = |carrier: &mut dyn Carrier| {
        let known = [(root.as_str(), "<root>"), (server.as_str(), "<server>")];
        // A fresh list: a gate a scenario opens is opened for good.
        let mut out = Vec::new();
        for (number, scenario) in conformance::scenarios().into_iter().enumerate() {
            let workspace = work.join(format!("s{number:02}"));
            std::fs::create_dir_all(&workspace).unwrap();
            for (path, text) in &scenario.files {
                let path = workspace.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            for reply in scenario.model.clone() {
                machine.server.push(reply);
            }
            let workspace = workspace.to_str().unwrap().to_owned();
            out.push((
                scenario.name,
                conformance::run(&scenario, carrier, &workspace, &known),
            ));
        }
        out
    };
    carry(&machine, &mut run)
}

/// Every conformance scenario gives the same transcript through the real
/// binary's standard I/O as through the same core in this process, and the
/// binary writes nothing on standard output but messages.
#[test]
fn sys_serve_stdio_conforms_to_the_core_in_process() {
    // Folders of one length: a path's length shows in a token estimate.
    let in_process = transcripts("serve-a", |machine, run| {
        let mut carrier = InProcess::start(machine);
        run(&mut carrier)
    });
    let over_stdio = transcripts("serve-b", |machine, run| {
        let mut served = serve(machine);
        served
            .lines
            .initialize(conformance::initialize())
            .expect("initialize");
        let transcripts = run(&mut served.lines);
        assert_eq!(served.end(), Some(0), "the server's exit");
        transcripts
    });
    assert_eq!(in_process.len(), conformance::scenarios().len());
    for ((name, first), (_, second)) in in_process.iter().zip(&over_stdio) {
        for (at, (one, other)) in first.0.iter().zip(&second.0).enumerate() {
            if one != other {
                let around = |entries: &[serde_json::Value]| -> Vec<String> {
                    entries[at.saturating_sub(3)..(at + 3).min(entries.len())]
                        .iter()
                        .map(|entry| {
                            let text = entry.to_string();
                            text.chars().take(200).collect()
                        })
                        .collect()
                };
                panic!(
                    "{name}: entry {at} differs.\nin process:\n{:#?}\nover standard I/O:\n{:#?}",
                    around(&first.0),
                    around(&second.0)
                );
            }
        }
        assert_eq!(
            first.0.len(),
            second.0.len(),
            "{name}: one carrier has more entries"
        );
    }
}

/// A signal ends the server as that signal, with no client's help: its
/// sessions end, and it exits 143 (SIGTERM on Linux, Ctrl-Break on
/// Windows), with standard input still open.
#[test]
fn sys_a_signal_ends_the_server_with_its_code() {
    let machine = Machine::new("serve-signal");
    let (input, mut writer) = std::io::pipe().unwrap();
    let (output, written) = std::io::pipe().unwrap();
    let mut command = machine.command(&["serve", "--stdio"]);
    command
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(written));
    // Spawning takes the command, and with it this side's copies of the
    // binary's ends.
    let tree = Tree::spawn(command, None).expect("the binary starts");
    let mut output = BufReader::new(output);
    // The server is serving once it has answered.
    let initialize = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": conformance::initialize()});
    writer
        .write_all(format!("{initialize}\n").as_bytes())
        .unwrap();
    let mut answer = String::new();
    output.read_line(&mut answer).unwrap();
    assert!(answer.contains("\"result\""), "{answer}");
    #[cfg(unix)]
    tree.terminate().unwrap();
    #[cfg(windows)]
    tree.interrupt().unwrap();
    let exit = tree.wait().unwrap();
    tree.end();
    drop(writer);
    assert!(matches!(exit, Exit::Code(143)), "{exit:?}");
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut output, &mut rest).unwrap();
    for line in rest.lines() {
        serde_json::from_str::<cineris_protocol::jsonrpc::Message>(line)
            .unwrap_or_else(|error| panic!("not a message ({error}): {line}"));
    }
}

/// `shutdown` is answered before the binary exits, and the exit is 0: the
/// last line is written before the process ends.
#[test]
fn sys_shutdown_is_answered_before_the_server_exits() {
    let machine = Machine::with_config("serve-shutdown", "index.map = false\n");
    let mut served = serve(&machine);
    served
        .lines
        .initialize(conformance::initialize())
        .expect("initialize");
    let stopped = served
        .lines
        .call(Op::Shutdown(cineris_protocol::methods::Empty {}))
        .expect("shutdown is answered");
    assert_eq!(stopped.result().unwrap(), serde_json::json!({}));
    assert!(
        served.lines.read().is_none(),
        "the server wrote past its end"
    );
    assert_eq!(served.child.wait().unwrap().code(), Some(0));
}

/// What a client of the socket at `path`, run as `user` (or as the test's
/// own user with `None`), was sent after it said `initialize`: `refused:`
/// and the error when it could not connect, `b''` when the server closed
/// the connection unanswered, and the answer otherwise.
#[cfg(unix)]
fn connect_as(user: Option<&str>, path: &std::path::Path) -> String {
    let initialize = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": conformance::initialize()});
    let script = "import socket, sys\n\
        s = socket.socket(socket.AF_UNIX)\n\
        try:\n    s.connect(sys.argv[1])\n\
        except OSError as error:\n    sys.stdout.write('refused: ' + str(error.errno))\n    sys.exit(0)\n\
        try:\n    s.sendall(sys.argv[2].encode() + b'\\n')\n    data = s.recv(65536)\n\
        except OSError:\n    data = b''\n\
        sys.stdout.write(repr(data))\n";
    let mut command = match user {
        Some(user) => {
            let mut command = std::process::Command::new("sudo");
            command.args(["-n", "-u", user, "python3"]);
            command
        }
        None => std::process::Command::new("python3"),
    };
    let output = command
        .args([
            "-c",
            script,
            path.to_str().unwrap(),
            &initialize.to_string(),
        ])
        .output()
        .expect("python3 starts");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Another user cannot use the server: the socket's mode keeps them from
/// connecting at all, and, with the mode opened by the test, the check of
/// the peer's credentials closes their connection unanswered. It needs a
/// second user and `sudo` with no password, which a hosted Linux runner
/// has: where `CI` is set it must run; elsewhere it says it was not run.
#[cfg(unix)]
#[test]
fn sys_another_user_is_refused_at_the_socket() {
    use std::os::unix::fs::PermissionsExt;
    const OTHER: &str = "nobody";
    let can = std::process::Command::new("sudo")
        .args(["-n", "-u", OTHER, "true"])
        .status()
        .is_ok_and(|status| status.success());
    if !can {
        assert!(
            std::env::var_os("CI").is_none(),
            "this runner cannot run a command as `{OTHER}` with `sudo -n`"
        );
        eprintln!("not run: no second user to run as here (`sudo -n -u {OTHER}` is refused)");
        return;
    }
    let machine = Machine::new("serve-two-users");
    // A folder another user can reach: under `/tmp`, not the test's own,
    // whose parents may not be reachable.
    let folder = std::path::PathBuf::from(format!("/tmp/cineris-two-users-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    struct Gone(std::path::PathBuf);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _gone = Gone(folder.clone());
    let path = folder.join("rpc.sock");
    let mut command = machine.command(&["serve", "--socket", path.to_str().unwrap()]);
    command.stdin(Stdio::piped());
    let mut server = command.spawn().expect("the binary starts");
    let mut said = String::new();
    BufReader::new(server.stderr.take().unwrap())
        .read_line(&mut said)
        .unwrap();
    assert!(said.contains("serving at"), "{said}");

    // The controls: the other user reaches the folder, and the test's own
    // user is served.
    let reaches = std::process::Command::new("sudo")
        .args(["-n", "-u", OTHER, "test", "-x", folder.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(reaches.success(), "the other user cannot reach the folder");
    assert!(
        connect_as(None, &path).contains("\"result\""),
        "the test's own user was not served"
    );
    // The socket's mode.
    let refused = connect_as(Some(OTHER), &path);
    assert!(refused.starts_with("refused: 13"), "{refused}");
    // The peer check by itself.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(connect_as(Some(OTHER), &path), "b''");
    drop(server.stdin.take());
    let _ = server.kill();
    let _ = server.wait();
}

/// A client of the Unix socket at `path`, speaking lines.
#[cfg(unix)]
fn socket_client(path: &std::path::Path) -> (Lines, std::os::unix::net::UnixStream) {
    let stream = std::os::unix::net::UnixStream::connect(path).expect("the socket takes a client");
    let mut writing = stream.try_clone().unwrap();
    let kept = stream.try_clone().unwrap();
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

/// `serve --socket` with no path serves in the runtime directory, in a
/// folder of its own that it makes owner-only, says where on standard
/// error, and serves its clients there. A session outlives the client that
/// started it, for another to open (an observer may open only a session
/// that is running). The server ends at a client's `shutdown`, and removes
/// the socket.
#[cfg(unix)]
#[test]
fn sys_serve_socket_serves_at_the_default_place_and_removes_it() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::new("serve-socket");
    // Short, as a real runtime directory is: a socket's path has at most
    // 107 bytes, which a test's own folder may pass in a deep checkout.
    let runtime = std::path::PathBuf::from(format!("/tmp/cineris-rt-{}", std::process::id()));
    struct Gone(std::path::PathBuf);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _gone = Gone(runtime.clone());
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = machine.command(&["serve", "--socket"]);
    command.env("XDG_RUNTIME_DIR", &runtime);
    let mut server = command.spawn().expect("the binary starts");
    let mut errors = BufReader::new(server.stderr.take().unwrap());
    let mut said = String::new();
    errors.read_line(&mut said).unwrap();
    let folder = runtime.join("cineris");
    let path = folder.join("rpc.sock");
    assert!(said.contains(&path.display().to_string()), "{said}");
    let mode = std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "the server's own folder");

    let (mut first, stream) = socket_client(&path);
    first
        .initialize(conformance::initialize())
        .expect("initialize");
    let OpResult::SessionNew(opened) = first
        .call(Op::SessionNew(
            cineris_protocol::methods::SessionNewParams {
                workspace: machine.workspace.to_str().unwrap().to_owned(),
                settings: cineris_protocol::session::SessionSettings::default(),
                title: None,
                config: serde_json::Map::new(),
                ephemeral: false,
                cwd: None,
            },
        ))
        .expect("a session opens")
    else {
        panic!("not a session");
    };
    // The first client goes.
    stream.shutdown(std::net::Shutdown::Both).unwrap();
    drop(first);

    let (mut watcher, _) = socket_client(&path);
    let mut params = conformance::initialize();
    params.role = cineris_protocol::methods::ClientRole::Observer;
    watcher.initialize(params).expect("initialize");
    watcher
        .call(Op::SessionOpen(cineris_protocol::methods::SessionRef {
            session_id: opened.session.session_id,
        }))
        .expect("the session outlived the client that started it");
    let (mut lines, _) = socket_client(&path);
    lines
        .initialize(conformance::initialize())
        .expect("initialize");
    lines
        .call(Op::Shutdown(cineris_protocol::methods::Empty {}))
        .expect("shutdown is answered");
    assert_eq!(server.wait().unwrap().code(), Some(0));
    assert!(!path.exists(), "the socket outlived the server");
    drop(errors);
}

/// `serve --socket NAME` on Windows serves a named pipe of that name, says
/// where on standard error, serves a client of the user there, and ends at
/// its `shutdown`. The client writes and reads in turn on one thread: a
/// pipe opened without overlapped I/O does one at a time.
#[cfg(windows)]
#[test]
fn sys_serve_socket_serves_on_a_named_pipe() {
    let machine = Machine::new("serve-pipe");
    let name = format!("cineris-test-{}", std::process::id());
    let mut server = machine
        .command(&["serve", "--socket", &name])
        .spawn()
        .expect("the binary starts");
    let mut errors = BufReader::new(server.stderr.take().unwrap());
    let mut said = String::new();
    errors.read_line(&mut said).unwrap();
    let pipe = format!(r"\\.\pipe\{name}");
    assert!(said.contains(&pipe), "{said}");

    let mut client = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&pipe)
        .expect("the pipe opens for the user");
    let mut reader = BufReader::new(client.try_clone().unwrap());
    let initialize = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": conformance::initialize()});
    writeln!(client, "{initialize}").unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(
        line.contains(r#""id":1"#) && line.contains(r#""result""#),
        "{line}"
    );
    let shutdown =
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": {}});
    writeln!(client, "{shutdown}").unwrap();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).unwrap();
        assert!(read > 0, "the pipe closed before shutdown was answered");
        if line.contains(r#""id":2"#) {
            assert!(line.contains(r#""result""#), "{line}");
            break;
        }
    }
    assert_eq!(server.wait().unwrap().code(), Some(0));
    drop(errors);
}
