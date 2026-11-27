//! Fake MCP servers of both eras, for tests: scripted, in process, over
//! in-memory pipes (`docs/steps/30/parts/pr4-client.md`, "The fake
//! servers"). Code for tests only, behind the `test-kit` feature, which
//! only dev-dependencies turn on.
//!
//! A [`FakeServer`] is a script and a record. Each [`FakeServer::spawn`]
//! starts a "process" of it: a task on the test's runtime that reads the
//! lines the client writes and answers them as the script says, with a
//! [`ServerProcess`] that exits when its input closes, when the script
//! says, or when the test says. The test opens gates to let calls answer,
//! sends what it likes on the server's behalf, and reads back every message
//! the server received.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cineris_tools::{BoxFuture, Json};
use serde_json::json;
use tokio::sync::{mpsc, oneshot, watch};

use crate::connect::Respawn;
use crate::process::{LineError, ServerPipes, ServerProcess};
use crate::transport::lock;

/// The longest line the fake pipes carry, as the harness's pipes cap
/// them.
pub const LINE_CAP: usize = 4 * 1024 * 1024;

/// A gate a test opens to let something go on. Clones are one gate.
#[derive(Debug, Clone)]
pub struct Gate {
    open: Arc<watch::Sender<bool>>,
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Gate {
    /// A closed gate.
    #[must_use]
    pub fn new() -> Self {
        Self {
            open: Arc::new(watch::channel(false).0),
        }
    }

    /// Opens it.
    pub fn open(&self) {
        self.open.send_replace(true);
    }

    /// Completes when it is open.
    pub async fn opened(&self) {
        let mut open = self.open.subscribe();
        let _open = open.wait_for(|open| *open).await;
    }
}

/// How a server answers `server/discover`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discover {
    /// With a discovery result (a modern server's default).
    Answer,
    /// Not at all.
    Silent,
    /// By exiting, as some legacy servers do on any request before
    /// `initialize`.
    Exit,
    /// With an error of this code (a legacy server's default is `-32601`).
    Error(i64),
    /// With an error of this code, and then by exiting.
    ErrorThenExit(i64),
    /// With `-32022` naming these revisions, the first `times` it is asked,
    /// and as [`Discover::Answer`] after.
    Unsupported {
        /// The revisions the error names.
        supported: Vec<String>,
        /// How many times it answers so.
        times: usize,
    },
}

/// How a server answers a call of one tool.
#[derive(Debug, Clone)]
pub enum Reply {
    /// With this result, as sent (a modern server adds `resultType`).
    Result(Json),
    /// With a JSON-RPC error.
    Error(i64, String),
    /// Once the gate is open, as the inner reply says.
    Gated(Gate, Box<Reply>),
    /// With `input_required` and each state in turn, checking that each
    /// call sent again gives back the state before; then as the inner reply.
    InputRequired(Vec<String>, Box<Reply>),
    /// With this whole `input_required` result, every time.
    AlwaysInput(Json),
    /// Twice, the second a duplicate.
    Twice(Box<Reply>),
    /// By exiting, the call unanswered.
    Crash,
    /// Never.
    Silent,
}

/// How a server ends its change stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndListen {
    /// With a result for the listen request.
    Result,
    /// With `notifications/cancelled` naming it.
    Cancelled,
}

/// What a fake server does: its era, its answers and its tools.
#[derive(Debug, Clone)]
struct Script {
    /// The legacy revision it answers `initialize` with; none for a modern
    /// server.
    legacy: Option<String>,
    /// The modern revisions it speaks.
    supported: Vec<String>,
    discover: Discover,
    discovers: usize,
    exits_before_initialize: bool,
    pings_during_initialize: bool,
    tools: Vec<Json>,
    page_size: usize,
    list_changed: bool,
    ttl_ms: Option<i64>,
    honours_listen: bool,
    replies: HashMap<String, Reply>,
    stderr: String,
    /// What `initialize` waits for before it is answered.
    initialize_gate: Option<Gate>,
    /// What each page of `tools/list` waits for before it is answered.
    list_gate: Option<Gate>,
}

/// Where a fake process writes its lines.
type Output = mpsc::Sender<Result<Vec<u8>, LineError>>;

/// What one process of the server has: its output, its exit, where it is
/// in the handshake.
struct Process {
    number: usize,
    /// Its input, without keeping it open: gone once the client closed it.
    input: mpsc::WeakSender<Vec<u8>>,
    output: Mutex<Option<Output>>,
    exited: watch::Sender<Option<String>>,
    initialized: Mutex<bool>,
    /// The answers the server waits for to its own requests, by id.
    awaiting: Mutex<HashMap<String, oneshot::Sender<Json>>>,
    /// The id of the open change stream.
    listen: Mutex<Option<Json>>,
}

impl Process {
    /// Writes one line of output; nothing after the exit.
    async fn write(&self, line: Result<Vec<u8>, LineError>) {
        let output = lock(&self.output).clone();
        if let Some(output) = output {
            let _gone = output.send(line).await;
        }
    }

    async fn send(&self, message: Json) {
        self.write(Ok(message.to_string().into_bytes())).await;
    }

    /// Exits with `words`: the exit is seen before the output ends.
    fn exit(&self, words: &str) {
        self.exited.send_if_modified(|exited| {
            if exited.is_some() {
                return false;
            }
            *exited = Some(words.to_owned());
            true
        });
        lock(&self.output).take();
    }

    fn has_exited(&self) -> bool {
        self.exited.borrow().is_some()
    }
}

struct Shared {
    script: Mutex<Script>,
    received: Mutex<Vec<(usize, Json)>>,
    count: watch::Sender<usize>,
    spawns: AtomicUsize,
    current: Mutex<Option<Arc<Process>>>,
    shutdowns: Mutex<Vec<Duration>>,
}

/// A scripted MCP server of either era. Clones are one server.
#[derive(Clone)]
pub struct FakeServer {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for FakeServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeServer")
            .field("spawns", &self.spawns())
            .finish_non_exhaustive()
    }
}

impl FakeServer {
    fn new(legacy: Option<&str>, discover: Discover) -> Self {
        let script = Script {
            legacy: legacy.map(str::to_owned),
            supported: vec!["2026-07-28".to_owned()],
            discover,
            discovers: 0,
            exits_before_initialize: false,
            pings_during_initialize: false,
            tools: Vec::new(),
            page_size: 100,
            list_changed: false,
            ttl_ms: None,
            honours_listen: true,
            replies: HashMap::new(),
            stderr: String::new(),
            initialize_gate: None,
            list_gate: None,
        };
        Self {
            shared: Arc::new(Shared {
                script: Mutex::new(script),
                received: Mutex::new(Vec::new()),
                count: watch::channel(0).0,
                spawns: AtomicUsize::new(0),
                current: Mutex::new(None),
                shutdowns: Mutex::new(Vec::new()),
            }),
        }
    }

    /// A modern server, speaking `2026-07-28`.
    #[must_use]
    pub fn modern() -> Self {
        Self::new(None, Discover::Answer)
    }

    /// A legacy server that answers `initialize` with `version`, and
    /// `server/discover` with `-32601`.
    #[must_use]
    pub fn legacy(version: &str) -> Self {
        Self::new(Some(version), Discover::Error(-32601))
    }

    fn script(&self, change: impl FnOnce(&mut Script)) -> &Self {
        change(&mut lock(&self.shared.script));
        self
    }

    /// The modern revisions it speaks.
    #[must_use]
    pub fn supported(self, versions: &[&str]) -> Self {
        self.script(|script| {
            script.supported = versions
                .iter()
                .map(|version| (*version).to_owned())
                .collect();
        });
        self
    }

    /// How it answers `server/discover`.
    #[must_use]
    pub fn on_discover(self, discover: Discover) -> Self {
        self.script(|script| script.discover = discover);
        self
    }

    /// A legacy server that exits on any request before `initialize`.
    #[must_use]
    pub fn exits_before_initialize(self) -> Self {
        self.script(|script| script.exits_before_initialize = true);
        self
    }

    /// A legacy server that pings the client, and waits for the answer,
    /// before it answers `initialize`.
    #[must_use]
    pub fn pings_during_initialize(self) -> Self {
        self.script(|script| script.pings_during_initialize = true);
        self
    }

    /// Its tools, each a definition as sent.
    #[must_use]
    pub fn tools(self, tools: Vec<Json>) -> Self {
        self.set_tools(tools);
        self
    }

    /// Changes its tools.
    pub fn set_tools(&self, tools: Vec<Json>) {
        self.script(|script| script.tools = tools);
    }

    /// How many tools a page of its list holds. The first page's next
    /// cursor is the empty string, which is a cursor.
    #[must_use]
    pub fn page_size(self, size: usize) -> Self {
        self.script(|script| script.page_size = size.max(1));
        self
    }

    /// That it says it tells the client when its tools change.
    #[must_use]
    pub fn list_changed(self) -> Self {
        self.script(|script| script.list_changed = true);
        self
    }

    /// The `ttlMs` of each page of its list (modern).
    #[must_use]
    pub fn ttl_ms(self, ttl: i64) -> Self {
        self.script(|script| script.ttl_ms = Some(ttl));
        self
    }

    /// A modern server that acknowledges a change stream without the change
    /// notices asked for.
    #[must_use]
    pub fn refuses_listen(self) -> Self {
        self.script(|script| script.honours_listen = false);
        self
    }

    /// How it answers a call of `tool`. Unscripted, a call is answered with
    /// one text block, `called TOOL`.
    #[must_use]
    pub fn reply(self, tool: &str, reply: Reply) -> Self {
        self.script(|script| {
            script.replies.insert(tool.to_owned(), reply);
        });
        self
    }

    /// That it answers `initialize` only once `gate` is open.
    #[must_use]
    pub fn initialize_gate(self, gate: &Gate) -> Self {
        self.script(|script| script.initialize_gate = Some(gate.clone()));
        self
    }

    /// That it answers each page of `tools/list` only once `gate` is open.
    #[must_use]
    pub fn list_gate(self, gate: &Gate) -> Self {
        self.script(|script| script.list_gate = Some(gate.clone()));
        self
    }

    /// What its process says on standard error.
    #[must_use]
    pub fn stderr(self, text: &str) -> Self {
        self.script(|script| script.stderr = text.to_owned());
        self
    }

    /// Starts a process of it.
    #[must_use]
    pub fn spawn(&self) -> ServerPipes {
        let number = self.shared.spawns.fetch_add(1, Ordering::AcqRel) + 1;
        let (input, input_lines) = mpsc::channel(64);
        let (output, output_lines) = mpsc::channel(64);
        let process = Arc::new(Process {
            number,
            input: input.downgrade(),
            output: Mutex::new(Some(output)),
            exited: watch::channel(None).0,
            initialized: Mutex::new(false),
            awaiting: Mutex::new(HashMap::new()),
            listen: Mutex::new(None),
        });
        *lock(&self.shared.current) = Some(Arc::clone(&process));
        tokio::spawn(serve(self.clone(), Arc::clone(&process), input_lines));
        let stderr = lock(&self.shared.script).stderr.clone();
        ServerPipes {
            input,
            output: output_lines,
            process: Box::new(FakeProcess {
                server: self.clone(),
                process,
                stderr,
            }),
        }
    }

    /// A closure that starts it again, for the client's start.
    #[must_use]
    pub fn respawn(&self) -> Respawn {
        let server = self.clone();
        Arc::new(move || Ok(server.spawn()))
    }

    /// How many processes of it were started.
    #[must_use]
    pub fn spawns(&self) -> usize {
        self.shared.spawns.load(Ordering::Acquire)
    }

    /// The graces its processes were given to exit in.
    #[must_use]
    pub fn shutdowns(&self) -> Vec<Duration> {
        lock(&self.shared.shutdowns).clone()
    }

    /// Every message it received, in order.
    #[must_use]
    pub fn received(&self) -> Vec<Json> {
        lock(&self.shared.received)
            .iter()
            .map(|(_, message)| message.clone())
            .collect()
    }

    /// The messages the process numbered `number` (from 1) received.
    #[must_use]
    pub fn received_by(&self, number: usize) -> Vec<Json> {
        lock(&self.shared.received)
            .iter()
            .filter(|(process, _)| *process == number)
            .map(|(_, message)| message.clone())
            .collect()
    }

    /// The methods of the requests and notifications it received, in
    /// order.
    #[must_use]
    pub fn methods(&self) -> Vec<String> {
        self.received()
            .iter()
            .filter_map(|message| message.get("method").and_then(Json::as_str))
            .map(str::to_owned)
            .collect()
    }

    /// The messages of `method` it received.
    #[must_use]
    pub fn of(&self, method: &str) -> Vec<Json> {
        self.received()
            .into_iter()
            .filter(|message| message.get("method").and_then(Json::as_str) == Some(method))
            .collect()
    }

    /// Completes when it has received `count` messages of `method`.
    pub async fn wait_for(&self, method: &str, count: usize) {
        self.wait_until(|server| server.of(method).len() >= count)
            .await;
    }

    /// Completes when `ready` holds of it, asked after each message.
    pub async fn wait_until(&self, ready: impl Fn(&Self) -> bool) {
        let mut count = self.shared.count.subscribe();
        loop {
            if ready(self) {
                return;
            }
            if count.changed().await.is_err() {
                return;
            }
        }
    }

    fn current(&self) -> Option<Arc<Process>> {
        lock(&self.shared.current).clone()
    }

    /// Sends `message` from its current process.
    pub async fn send(&self, message: Json) {
        if let Some(process) = self.current() {
            process.send(message).await;
        }
    }

    /// Sends one line of output, as it is.
    pub async fn send_line(&self, line: &[u8]) {
        if let Some(process) = self.current() {
            process.write(Ok(line.to_vec())).await;
        }
    }

    /// Has the pipes report a line longer than their cap.
    pub async fn send_too_long(&self) {
        if let Some(process) = self.current() {
            process.write(Err(LineError::TooLong(LINE_CAP))).await;
        }
    }

    /// Sends `message` `count` times.
    pub async fn flood(&self, count: usize, message: Json) {
        let Some(process) = self.current() else {
            return;
        };
        let line = message.to_string().into_bytes();
        for _ in 0..count {
            if process.has_exited() {
                return;
            }
            process.write(Ok(line.clone())).await;
        }
    }

    /// Tells the client its tools changed: a legacy server at once, a
    /// modern one on its open change stream.
    pub async fn notify_list_changed(&self) {
        let Some(process) = self.current() else {
            return;
        };
        let listen = lock(&process.listen).clone();
        let params =
            listen.map(|id| json!({"_meta": {"io.modelcontextprotocol/subscriptionId": id}}));
        let mut message = json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"});
        if let Some(params) = params {
            message["params"] = params;
        }
        process.send(message).await;
    }

    /// The id of its open change stream.
    #[must_use]
    pub fn listen_id(&self) -> Option<Json> {
        self.current()
            .and_then(|process| lock(&process.listen).clone())
    }

    /// Ends its open change stream as `how` says.
    pub async fn end_listen(&self, how: EndListen) {
        let Some(process) = self.current() else {
            return;
        };
        let Some(id) = lock(&process.listen).take() else {
            return;
        };
        let message = match how {
            EndListen::Result => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "resultType": "complete",
                    "_meta": {"io.modelcontextprotocol/subscriptionId": id},
                },
            }),
            EndListen::Cancelled => json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": {"requestId": id, "reason": "the server is going away"},
            }),
        };
        process.send(message).await;
    }

    /// Ends its current process with `words`, as a crash.
    pub fn crash(&self, words: &str) {
        if let Some(process) = self.current() {
            process.exit(words);
        }
    }

    fn record(&self, number: usize, message: &Json) {
        lock(&self.shared.received).push((number, message.clone()));
        self.shared.count.send_modify(|count| *count += 1);
    }
}

/// The process of a fake server, as the client sees it.
struct FakeProcess {
    server: FakeServer,
    process: Arc<Process>,
    stderr: String,
}

impl ServerProcess for FakeProcess {
    fn stderr_tail(&self) -> String {
        self.stderr.clone()
    }

    fn exited(&self) -> BoxFuture<'_, String> {
        Box::pin(async move {
            let mut exited = self.process.exited.subscribe();
            loop {
                if let Some(words) = exited.borrow_and_update().clone() {
                    return words;
                }
                if exited.changed().await.is_err() {
                    return "gone".to_owned();
                }
            }
        })
    }

    fn shutdown(&self, grace: Duration) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            lock(&self.server.shared.shutdowns).push(grace);
            if self.process.input.upgrade().is_some() {
                // Its input is still open: the staged kill.
                self.process.exit("ended by the staged kill");
                return;
            }
            // Its input closed: it reads what is left and exits.
            let mut exited = self.process.exited.subscribe();
            let _exited = exited.wait_for(Option::is_some).await;
        })
    }
}

/// Runs one process: reads its input until it closes or the process
/// exits.
async fn serve(server: FakeServer, process: Arc<Process>, mut input: mpsc::Receiver<Vec<u8>>) {
    let mut exited = process.exited.subscribe();
    let mut exit = std::pin::pin!(async move {
        let _exited = exited.wait_for(Option::is_some).await;
    });
    loop {
        // What the client wrote is read before an exit is seen, as a
        // process reads its pipe.
        let line = std::future::poll_fn(|context| {
            if let std::task::Poll::Ready(line) = input.poll_recv(context) {
                return std::task::Poll::Ready(line);
            }
            exit.as_mut().poll(context).map(|()| None)
        })
        .await;
        let Some(line) = line else {
            // Its input closed (it exits, as a server should), or it exited.
            process.exit("its input closed");
            return;
        };
        if process.has_exited() {
            return;
        }
        let Ok(message) = serde_json::from_slice::<Json>(&line) else {
            continue;
        };
        server.record(process.number, &message);
        handle(&server, &process, message);
    }
}

/// Answers one message as the script says.
fn handle(server: &FakeServer, process: &Arc<Process>, message: Json) {
    let method = message
        .get("method")
        .and_then(Json::as_str)
        .map(str::to_owned);
    let id = message.get("id").cloned();
    let Some(method) = method else {
        // An answer to one of the server's own requests.
        if let Some(id) = id.as_ref().and_then(Json::as_str)
            && let Some(waiter) = lock(&process.awaiting).remove(id)
        {
            let _gone = waiter.send(message);
        }
        return;
    };
    let script = lock(&server.shared.script).clone();
    let Some(id) = id else {
        notified(process, &method, &message);
        return;
    };
    let process = Arc::clone(process);
    let server = server.clone();
    tokio::spawn(async move {
        respond(&server, &script, &process, &method, id, &message).await;
    });
}

fn notified(process: &Process, method: &str, message: &Json) {
    match method {
        "notifications/initialized" => *lock(&process.initialized) = true,
        "notifications/cancelled" => {
            let named = message.pointer("/params/requestId").cloned();
            let mut listen = lock(&process.listen);
            if named.is_some() && *listen == named {
                *listen = None;
            }
        }
        _ => {}
    }
}

fn answer(id: &Json, result: Json) -> Json {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: &Json, code: i64, message: &str) -> Json {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Answers one request.
async fn respond(
    server: &FakeServer,
    script: &Script,
    process: &Arc<Process>,
    method: &str,
    id: Json,
    message: &Json,
) {
    let modern = script.legacy.is_none();
    if method == "server/discover" {
        if script.exits_before_initialize && !*lock(&process.initialized) {
            return process.exit("it exited on a request before initialize");
        }
        return discover(server, script, process, &id, message).await;
    }
    if !modern {
        let initialized = *lock(&process.initialized);
        if method == "initialize" {
            return initialize(script, process, &id).await;
        }
        if method == "ping" {
            return process.send(answer(&id, json!({}))).await;
        }
        if !initialized {
            if script.exits_before_initialize {
                return process.exit("it exited on a request before initialize");
            }
            return process
                .send(error(&id, -32600, "the server is not initialized"))
                .await;
        }
    } else {
        let version = message
            .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
            .and_then(Json::as_str);
        let Some(version) = version else {
            return process
                .send(error(&id, -32602, "no protocol version in _meta"))
                .await;
        };
        if !script.supported.iter().any(|ours| ours == version) {
            return process
                .send(unsupported(&id, &script.supported, version))
                .await;
        }
    }
    match method {
        "tools/list" => {
            if let Some(gate) = &script.list_gate {
                gate.opened().await;
            }
            process.send(list(script, &id, message)).await;
        }
        "tools/call" => {
            let tool = message
                .pointer("/params/name")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            let reply = script.replies.get(&tool).cloned().unwrap_or_else(|| {
                Reply::Result(
                    json!({"content": [{"type": "text", "text": format!("called {tool}")}]}),
                )
            });
            call(process, &id, message, reply, modern).await;
        }
        "subscriptions/listen" if modern => {
            *lock(&process.listen) = Some(id.clone());
            process
                .send(json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/subscriptions/acknowledged",
                    "params": {
                        "_meta": {"io.modelcontextprotocol/subscriptionId": id},
                        "notifications": if script.honours_listen {
                            json!({"toolsListChanged": true})
                        } else {
                            json!({})
                        },
                    },
                }))
                .await;
        }
        _ => process.send(error(&id, -32601, "method not found")).await,
    }
}

fn unsupported(id: &Json, supported: &[String], requested: &str) -> Json {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32022,
            "message": "Unsupported protocol version",
            "data": {"supported": supported, "requested": requested},
        },
    })
}

async fn discover(
    server: &FakeServer,
    script: &Script,
    process: &Process,
    id: &Json,
    message: &Json,
) {
    let asked = {
        let mut shared = lock(&server.shared.script);
        shared.discovers += 1;
        shared.discovers
    };
    match &script.discover {
        Discover::Silent => {}
        Discover::Exit => process.exit("it exited on a request before initialize"),
        Discover::Error(code) => process.send(error(id, *code, "method not found")).await,
        Discover::ErrorThenExit(code) => {
            process.send(error(id, *code, "method not found")).await;
            process.exit("it exited after it refused a request before initialize");
        }
        Discover::Unsupported { supported, times } if asked <= *times => {
            let requested = message
                .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
                .and_then(Json::as_str)
                .unwrap_or_default();
            process.send(unsupported(id, supported, requested)).await;
        }
        Discover::Answer | Discover::Unsupported { .. } => {
            process
                .send(answer(
                    id,
                    json!({
                        "resultType": "complete",
                        "supportedVersions": script.supported,
                        "capabilities": {"tools": {"listChanged": script.list_changed}},
                        "instructions": "Fake instructions, which must never reach a prompt.",
                        "ttlMs": 0,
                        "cacheScope": "public",
                        "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "fake-modern", "version": "1"}},
                    }),
                ))
                .await;
        }
    }
}

async fn initialize(script: &Script, process: &Process, id: &Json) {
    if let Some(gate) = &script.initialize_gate {
        gate.opened().await;
    }
    if script.pings_during_initialize {
        let (sender, receiver) = oneshot::channel();
        lock(&process.awaiting).insert("fake-ping".to_owned(), sender);
        process
            .send(json!({"jsonrpc": "2.0", "id": "fake-ping", "method": "ping"}))
            .await;
        // Any answer will do: the test reads what it was.
        if receiver.await.is_err() {
            return;
        }
    }
    let version = script.legacy.clone().unwrap_or_default();
    process
        .send(answer(
            id,
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": script.list_changed}},
                "serverInfo": {"name": "fake-legacy", "version": "1"},
                "instructions": "Fake instructions, which must never reach a prompt.",
            }),
        ))
        .await;
}

/// One page of the list. The cursor after page 0 is "", after page n the
/// text of n.
fn list(script: &Script, id: &Json, message: &Json) -> Json {
    let page = match message.pointer("/params/cursor").and_then(Json::as_str) {
        None => 0,
        Some("") => 1,
        Some(number) => number.parse().unwrap_or(usize::MAX),
    };
    let start = page.saturating_mul(script.page_size);
    let tools: Vec<Json> = script
        .tools
        .iter()
        .skip(start)
        .take(script.page_size)
        .cloned()
        .collect();
    let mut result = json!({"tools": tools});
    if start + script.page_size < script.tools.len() {
        let next = if page == 0 {
            String::new()
        } else {
            (page + 1).to_string()
        };
        result["nextCursor"] = Json::from(next);
    }
    if script.legacy.is_none() {
        result["resultType"] = Json::from("complete");
        result["cacheScope"] = Json::from("public");
        if let Some(ttl) = script.ttl_ms {
            result["ttlMs"] = Json::from(ttl);
        }
    }
    answer(id, result)
}

/// Answers a call as `reply` says.
fn call<'a>(
    process: &'a Process,
    id: &'a Json,
    message: &'a Json,
    reply: Reply,
    modern: bool,
) -> BoxFuture<'a, ()> {
    Box::pin(async move {
        match reply {
            Reply::Result(mut result) => {
                if modern && result.get("resultType").is_none() {
                    result["resultType"] = Json::from("complete");
                }
                process.send(answer(id, result)).await;
            }
            Reply::Error(code, text) => process.send(error(id, code, &text)).await,
            Reply::Gated(gate, inner) => {
                gate.opened().await;
                call(process, id, message, *inner, modern).await;
            }
            Reply::InputRequired(states, inner) => {
                let given = message
                    .pointer("/params/requestState")
                    .and_then(Json::as_str);
                let round = match given {
                    None => 0,
                    Some(state) => match states.iter().position(|ours| ours == state) {
                        Some(index) => index + 1,
                        None => {
                            return process
                                .send(error(id, -32602, "unknown request state"))
                                .await;
                        }
                    },
                };
                match states.get(round) {
                    Some(state) => {
                        process
                            .send(answer(
                                id,
                                json!({"resultType": "input_required", "requestState": state}),
                            ))
                            .await;
                    }
                    None => call(process, id, message, *inner, modern).await,
                }
            }
            Reply::AlwaysInput(result) => process.send(answer(id, result)).await,
            Reply::Twice(inner) => {
                call(process, id, message, (*inner).clone(), modern).await;
                call(process, id, message, *inner, modern).await;
            }
            Reply::Crash => process.exit("it crashed during a call"),
            Reply::Silent => {}
        }
    })
}
