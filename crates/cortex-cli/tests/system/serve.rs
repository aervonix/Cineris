//! `cortex serve --stdio`: the real binary as an RPC server
//! (`docs/architecture/11-rpc-protocol.md`; `docs/steps/27/answered.md`,
//! points 4 and 6).
//!
//! The conformance scenarios of `cortex_testkit::conformance` run through
//! the binary and through the same core in this process, against the same
//! scripted server, and what the client was sent must be the same. Every
//! line the binary writes on standard output is read as a JSON-RPC message:
//! the stream carries nothing else.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Stdio};
use std::sync::{Arc, Condvar, Mutex};

use cortex_base::CancelToken;
use cortex_config::{Env, Platform};
use cortex_core::{ClientId, ClientInfo, Core, CoreOptions, EventQueue, Outgoing, TokioTimer};
use cortex_protocol::ErrorData;
use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::methods::{Op, OpResult, ServerResponse};
use cortex_rpc::lane::{Dispatcher, Run};
use cortex_sandbox::tree::{Exit, Tree};
use cortex_testkit::conformance::{self, Carrier, Inbound, Lines, Transcript};

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
        let binary = std::path::PathBuf::from(crate::CORTEX);
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
            queue: cortex_core::QUEUE_CAPACITY,
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
        serde_json::from_str::<cortex_protocol::jsonrpc::Message>(line)
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
        .call(Op::Shutdown(cortex_protocol::methods::Empty {}))
        .expect("shutdown is answered");
    assert_eq!(stopped.result().unwrap(), serde_json::json!({}));
    assert!(
        served.lines.read().is_none(),
        "the server wrote past its end"
    );
    assert_eq!(served.child.wait().unwrap().code(), Some(0));
}
