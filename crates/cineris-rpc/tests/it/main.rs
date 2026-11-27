//! Integration tests (tier 2): the RPC server in process, over the
//! transport's in-memory ends, against a real core.
//!
//! Behind the core is everything real but the model, which is the test
//! kit's scripted SSE server, reached over the real HTTP client: the model
//! a scenario scripts is then the same for every carrier, the real binary's
//! included (`cineris-cli`'s system tests). The core runs on a thread of its
//! own with its runtime, as the binary's does, and every wait of it and of
//! the server is on the fake timer.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    missing_docs,
    missing_debug_implementations,
    unreachable_pub,
    reason = "an integration test prepares real files and fails by panicking, and its \
              helpers are shared between the modules of this one binary"
)]

mod conformance;
mod perf;
#[cfg(windows)]
mod pipe;
#[cfg(unix)]
mod socket;
mod wire;

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use cineris_base::{CancelToken, SharedTimer};
use cineris_config::{Dirs, Env, Platform};
use cineris_core::{ClientId, ClientInfo, Core, CoreOptions, EventQueue, Outgoing, QUEUE_CAPACITY};
use cineris_protocol::ErrorData;
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::methods::{Op, OpResult, ServerResponse};
use cineris_rpc::lane::{Dispatcher, Run};
use cineris_rpc::{Ended, ServeOptions, Transport};
use cineris_testkit::conformance::{self as kit, Carrier, Inbound, Lines};
use cineris_testkit::{FakeClock, FakeTimer, Scratch, SseServer};
use cineris_tools::process::Runner;

/// A machine of the test's own: a home, a workspace, a scripted model
/// server, a clock and a timer.
pub struct World {
    pub root: PathBuf,
    pub home: PathBuf,
    pub workspace: PathBuf,
    pub server: SseServer,
    pub clock: FakeClock,
    pub timer: FakeTimer,
    pub env: Env,
    _scratch: Scratch,
}

impl World {
    pub fn new(test: &str) -> Self {
        Self::with_config(test, "")
    }

    /// A world whose user configuration has `extra` at its end.
    pub fn with_config(test: &str, extra: &str) -> Self {
        let scratch = Scratch::outside("cineris-rpc-it", test);
        let root = scratch.path().to_path_buf();
        let home = root.join("home");
        let workspace = root.join("work").join("app");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let home_variable = match Platform::CURRENT {
            Platform::Unix => "HOME",
            Platform::Windows => "USERPROFILE",
        };
        let env = Env::empty(Platform::CURRENT)
            .with(home_variable, home.to_str().unwrap())
            .with("ANTHROPIC_API_KEY", "sk-ant-test-key");
        let clock = FakeClock::starting_at("2026-11-16T10:00:00Z").unwrap();
        let timer = FakeTimer::manual(&clock);
        let world = Self {
            root,
            home,
            workspace,
            server: SseServer::start(),
            clock,
            timer,
            env,
            _scratch: scratch,
        };
        world.user_config(extra);
        world
    }

    pub fn dirs(&self) -> Dirs {
        let mut dirs = Dirs::from_env(&self.env);
        dirs.managed_config = Some(self.root.join("managed").join("config.toml"));
        dirs
    }

    /// The user's configuration: the scripted server as Anthropic, and
    /// nothing kept on the disk past what a test needs.
    fn user_config(&self, extra: &str) {
        let path = self.dirs().user_config.unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!(
                "[session]\ndurability = \"never\"\n\n[credentials]\nstore = \"file\"\n\n\
                 [model]\nmain = \"{}\"\n\n[retry]\nattempts = 1\n\n\
                 [providers.anthropic]\nbase_url = \"{}\"\n\n{extra}",
                kit::MODEL,
                self.server.base_url()
            ),
        )
        .unwrap();
    }

    /// Puts a file in the workspace.
    pub fn put(&self, relative: &str, content: &str) {
        let path = self.workspace.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    pub fn timer(&self) -> SharedTimer {
        Arc::new(self.timer.clone())
    }

    /// The core's options: the scripted server, no background work whose
    /// events come when they like, and every wait on the fake timer.
    pub fn options(&self) -> CoreOptions {
        let mut options = CoreOptions::new(self.env.clone(), self.timer());
        options.dirs = Some(self.dirs());
        options.clock = Arc::new(self.clock.clone());
        options.harness_version = "0.0.0-test".to_owned();
        options.base_capture = false;
        options.repo_index = false;
        options.runner = Some(
            Runner::new()
                .with_timer(self.timer())
                .with_clock(Arc::new(self.clock.clone()))
                .with_frame(Duration::ZERO),
        );
        // The machine's shell, found once and not asked its version: no
        // process is started to open a session.
        static SHELL: std::sync::OnceLock<Option<cineris_tools::Shell>> =
            std::sync::OnceLock::new();
        // On Windows, Windows PowerShell where Windows keeps it, one look,
        // before a search of `PATH`, which looks in each of a hosted
        // runner's dozens of folders: 1.9 ms of a test's process there
        // (`docs/steps/ci-budgets-2/answered.md`).
        options.shell = SHELL
            .get_or_init(|| {
                let search = cineris_tools::ShellSearch::from_env();
                let kept = search.system_root.as_ref().map(|root| {
                    root.join("System32")
                        .join("WindowsPowerShell")
                        .join("v1.0")
                        .join("powershell.exe")
                });
                cfg!(windows)
                    .then(|| {
                        kept.as_deref()
                            .and_then(std::path::Path::to_str)
                            .and_then(|kept| search.find(Some(kept), None).ok())
                            .or_else(|| search.find(Some("powershell"), None).ok())
                    })
                    .flatten()
                    .or_else(|| search.find(None, None).ok())
            })
            .clone();
        options.cancel = CancelToken::new();
        options
    }

    /// The server's options: the core's timer and clock.
    pub fn serve_options(&self) -> ServeOptions {
        let mut options = ServeOptions::new(self.timer());
        options.clock = Arc::new(self.clock.clone());
        options
    }

    /// Starts a core on a thread of its own.
    pub fn core(&self) -> CoreThread {
        CoreThread::start(self.options())
    }
}

/// A core on a thread of its own, with its runtime, as the binary runs it.
pub struct CoreThread {
    pub core: Core,
    pub handle: tokio::runtime::Handle,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CoreThread {
    pub fn start(options: CoreOptions) -> Self {
        let (handed, handle) = std::sync::mpsc::channel();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("test-core".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                handed.send(runtime.handle().clone()).unwrap();
                runtime.block_on(async {
                    let _ = stopped.await;
                });
            })
            .unwrap();
        let handle: tokio::runtime::Handle = handle.recv().unwrap();
        let core = on(&handle, async move { Core::start(options) }).expect("the core starts");
        Self {
            core,
            handle,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    /// Runs `future` on the core's thread and waits for its output.
    pub fn on<T: Send + 'static>(&self, future: impl Future<Output = T> + Send + 'static) -> T {
        on(&self.handle, future)
    }
}

impl Drop for CoreThread {
    fn drop(&mut self) {
        let core = self.core.clone();
        on(&self.handle, async move { core.shutdown().await });
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Runs `future` on the runtime of `handle`, from a thread outside it, and
/// waits for its output.
pub fn on<T: Send + 'static>(
    handle: &tokio::runtime::Handle,
    future: impl Future<Output = T> + Send + 'static,
) -> T {
    let (sent, output) = std::sync::mpsc::channel();
    handle.spawn(async move {
        let _ = sent.send(future.await);
    });
    output.recv().expect("the core's thread ran the task")
}

/// Wakes a thread that waits for a client's queue.
#[derive(Default)]
struct Bell {
    rung: Mutex<bool>,
    bell: Condvar,
}

impl Bell {
    fn ring(&self) {
        *self.rung.lock().unwrap() = true;
        self.bell.notify_all();
    }

    fn wait(&self) {
        let mut rung = self.rung.lock().unwrap();
        while !*rung {
            rung = self.bell.wait(rung).unwrap();
        }
        *rung = false;
    }
}

/// The carrier in process: the client's operations through the same lane
/// the terminal UI's link runs them in, and its queue read directly.
pub struct InProcess {
    core: Core,
    handle: tokio::runtime::Handle,
    client: ClientId,
    queue: Arc<EventQueue>,
    bell: Arc<Bell>,
    dispatcher: Dispatcher<u64>,
    results: std::sync::mpsc::Receiver<(u64, Result<OpResult, ErrorData>)>,
    /// Results that came while another was awaited.
    early: std::collections::BTreeMap<u64, Result<OpResult, ErrorData>>,
    next: u64,
}

impl InProcess {
    pub fn connect(thread: &CoreThread) -> Self {
        let core = thread.core.clone();
        let params = kit::initialize();
        let info = ClientInfo {
            role: params.role.clone(),
            capabilities: params.capabilities.clone(),
            queue: QUEUE_CAPACITY,
        };
        let client = core.connect(info);
        let started = core.clone();
        thread
            .on(async move { started.handle(client.id, Op::Initialize(params)).await })
            .expect("initialize");
        let bell = Arc::new(Bell::default());
        let rung = Arc::clone(&bell);
        client.queue.set_wake(Arc::new(move || rung.ring()));
        let (done, results) = std::sync::mpsc::channel();
        let run_core = core.clone();
        let id = client.id;
        let run: Run = Arc::new(move |op| {
            let core = run_core.clone();
            Box::pin(async move { core.handle(id, op).await })
        });
        let done = Mutex::new(done);
        let dispatcher = thread.on(async move {
            Dispatcher::start(
                run,
                Arc::new(move |ticket, result| {
                    let _ = done.lock().unwrap().send((ticket, result));
                }),
            )
        });
        Self {
            core,
            handle: thread.handle.clone(),
            client: client.id,
            queue: client.queue,
            bell,
            dispatcher,
            results,
            early: std::collections::BTreeMap::new(),
            next: 0,
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
            let (answered, result) = self.results.recv().expect("the lane answered");
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
                None => self.bell.wait(),
            }
        }
    }

    fn respond(&mut self, id: RequestId, response: ServerResponse) {
        self.core
            .respond(self.client, &id, response)
            .expect("the answer is taken");
    }

    fn forget(&mut self) {
        while self.queue.try_next().is_some() {}
    }
}

/// The carrier over the wire: the RPC connection `serve --stdio` runs, over
/// the transport's in-memory ends, and a client that speaks lines.
pub struct Wire {
    pub lines: Lines,
    pub ended: std::sync::mpsc::Receiver<Ended>,
    /// Cancelled when the server's end begins.
    pub ending: CancelToken,
}

impl Wire {
    /// Serves one client on `thread`'s core with `options`, and connects to
    /// it with no handshake yet.
    pub fn serve(thread: &CoreThread, options: ServeOptions, lines: usize) -> Self {
        let (transport, ends) = Transport::pair(lines);
        let (finished, ended) = std::sync::mpsc::channel();
        let core = thread.core.clone();
        let ending = CancelToken::new();
        let told = ending.clone();
        thread.handle.spawn(async move {
            let stop = CancelToken::new();
            let how = cineris_rpc::serve_one(core, transport, options, &stop, &told).await;
            let _ = finished.send(how);
        });
        Self::client(ends, ended, ending)
    }

    /// One connection, run and finished, of a server that does not end with
    /// it: its sessions are the core's after it.
    pub fn connection(thread: &CoreThread, options: ServeOptions, lines: usize) -> Self {
        let (transport, ends) = Transport::pair(lines);
        let (finished, ended) = std::sync::mpsc::channel();
        let core = thread.core.clone();
        thread.handle.spawn(async move {
            let mut connection = cineris_rpc::Connection::start(core, transport, options);
            let how = connection
                .run(&CancelToken::new(), &CancelToken::new())
                .await;
            connection.finish(how).await;
            let _ = finished.send(how);
        });
        Self::client(ends, ended, CancelToken::new())
    }

    fn client(
        ends: cineris_rpc::Ends,
        ended: std::sync::mpsc::Receiver<Ended>,
        ending: CancelToken,
    ) -> Self {
        let input = ends.input;
        let mut output = ends.output;
        let lines = Lines::new(
            move |bytes| {
                let _ = input.blocking_send(Ok(bytes));
            },
            move || output.blocking_recv(),
        );
        Self {
            lines,
            ended,
            ending,
        }
    }

    /// The same, said `initialize` to.
    pub fn connect(thread: &CoreThread, options: ServeOptions) -> Self {
        let mut wire = Self::serve(thread, options, 64);
        wire.lines
            .initialize(kit::initialize())
            .expect("initialize");
        wire
    }
}

/// A world whose hook command and whose extension's program each connect
/// to the server's own place, as `connect` does (Python's lines, which set
/// `d` to what was read and `opened` to whether the place opened), and
/// answer, as their `session.start` hook's context, what they read: the
/// socket's check of the server's own commands, met by a hook and by
/// `cineris.exec` (since step 28's third pull request). Python is `python`
/// on Windows and `python3` elsewhere; `None` where it is not found, and
/// the test then says it was not run (it must run where `CI` is set).
pub fn probing(test: &str, connect: &str) -> Option<(World, CoreThread)> {
    let python = if cfg!(windows) { "python" } else { "python3" };
    let found = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            dir.is_absolute()
                && (dir.join(python).is_file() || dir.join(format!("{python}.exe")).is_file())
        })
    });
    if !found {
        assert!(
            std::env::var_os("CI").is_none(),
            "{python} is not on this runner's PATH"
        );
        // Away from CI, with no Python, the test says why it did not run.
        #[allow(clippy::print_stderr, reason = "a test that cannot run here says why")]
        {
            eprintln!("not run: {python} is not on the PATH here");
        }
        return None;
    }
    let script = format!(
        "import sys\nopened = False\ntry:\n{connect}\nexcept OSError:\n    d = b''\n\
         print('{{\"context\": \"%s opened %s read %d bytes\"}}' % (sys.argv[1], opened, len(d)))\n"
    );
    // A hook of the user's own, and nothing confined: what refuses a probe
    // is the socket's own check.
    let world = World::with_config(
        test,
        &format!(
            "[permissions]\nmode = \"unrestricted\"\n\n[sandbox]\nenabled = false\n\n\
             [hooks.probe]\nevent = \"session.start\"\n\
             command = [\"{python}\", \"-c\", '''{script}''', \"hook\"]\n"
        ),
    );
    let folder = world
        .dirs()
        .config
        .unwrap()
        .join("extensions")
        .join("prober");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        format!(
            "api = 1\nname = \"prober\"\nversion = \"1\"\nmain = \"main.js\"\n\n\
             [capabilities]\nexec = [\"{python}\"]\nhooks = [\"session.start\"]\n"
        ),
    )
    .unwrap();
    std::fs::write(
        folder.join("main.js"),
        format!(
            "cineris.on('session.start', {{ timeout: 60000 }}, async () => {{\n\
             const ran = await cineris.exec('{python}', ['-c', {}, 'exec'], {{ timeout: 60000 }});\n\
             return JSON.parse(ran.stdout);\n}});\n",
            serde_json::to_string(&script).unwrap()
        ),
    )
    .unwrap();
    // And a program the model's shell runs, as a hand-written client of the
    // server would be: the probe from a file of the workspace.
    std::fs::write(world.workspace.join("probe.py"), &script).unwrap();
    world
        .server
        .push(cineris_testkit::conformance::anthropic::tool(
            "toolu_probe",
            "shell",
            &serde_json::json!({"command": format!("{python} probe.py shell")}),
        ));
    let core = world.core();
    let project = core.core.project(&world.workspace).unwrap();
    let to_add = core.core.extension_to_add(&project, &folder).unwrap();
    let adding = core.core.clone();
    // Signed with the person's key, as `cineris ext add` signs it, which
    // opens the core's approvals with it (the signed approvals step).
    core.on(async move { adding.add_extension(to_add, &person_key()).await })
        .expect("the probe extension is approved");
    Some((world, core))
}

/// The person's key, made from a password at a cost small enough for a
/// test: what signs an approval of the tests' own.
pub fn person_key() -> cineris_core::signed::PersonKey {
    use cineris_core::signed::{Cost, KeyFile, Salt};
    KeyFile::make(
        b"the person's own password",
        Salt::from_bytes([7; Salt::LEN]),
        Cost::new(64, 1, 1).expect("within the caps"),
        cineris_protocol::ids::Timestamp::new("2026-11-25T10:00:00Z"),
    )
    .expect("a test's key is made")
    .1
}

/// What the `probing` world's probes said: every request of the session's
/// first turn, where a `session.start` hook's context is, and the shell's
/// result.
pub fn probes_said(world: &World) -> String {
    let requests = world.server.requests();
    assert!(!requests.is_empty(), "the turn asked the model");
    requests
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}
