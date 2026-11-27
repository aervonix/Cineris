//! The MCP client's tests: our own client against the fake servers of both
//! eras (`cineris_mcp::fake`), over in-memory pipes, with every wait on the
//! test kit's manual fake timer. Nothing waits for real time: a test moves
//! the clock past a limit once the limit's wait is running.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "an integration test's helpers fail the test by panicking"
)]

mod calls;
mod flood;
mod legacy;
mod modern;
mod probe;
mod start;

use std::sync::Arc;
use std::time::Duration;

use cineris_base::{FakeClock, SharedTimer};
use cineris_mcp::fake::FakeServer;
use cineris_mcp::{ClientOptions, Expose, Launch, Origin, ServerDefinition, StdioClient};
use cineris_testkit::FakeTimer;
use serde_json::{Value as Json, json};

/// The definition's start limit.
const START: Duration = Duration::from_secs(10);

/// The definition's call limit: apart from every other wait's length, so a
/// test can wait for it by its length.
const CALL: Duration = Duration::from_secs(45);

/// The probe's time.
const PROBE: Duration = Duration::from_secs(3);

/// The grace a server is given to exit.
const GRACE: Duration = Duration::from_secs(2);

/// Runs a test on a runtime of one thread, as the core runs.
fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime")
        .block_on(future)
}

/// A timer whose waits end only when the test moves its clock.
fn timer() -> FakeTimer {
    FakeTimer::manual(&FakeClock::default())
}

fn shared(timer: &FakeTimer) -> SharedTimer {
    Arc::new(timer.clone())
}

/// A local server's definition, with the limits above.
fn definition() -> ServerDefinition {
    ServerDefinition {
        name: "fake".to_owned(),
        origin: Origin::User,
        launch: Launch::Local {
            program: "fake-server".to_owned(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            network: false,
            write: false,
        },
        description: None,
        enabled: true,
        expose: Expose::Search,
        tools: None,
        deny_tools: Vec::new(),
        start_timeout: START,
        call_timeout: CALL,
    }
}

/// Connects to `server`, which must answer at once.
async fn connect(server: &FakeServer, timer: &FakeTimer) -> StdioClient {
    connect_with(server, timer, ClientOptions::default()).await
}

async fn connect_with(
    server: &FakeServer,
    timer: &FakeTimer,
    options: ClientOptions,
) -> StdioClient {
    StdioClient::connect(
        &definition(),
        server.spawn(),
        server.respawn(),
        shared(timer),
        options,
    )
    .await
    .expect("the client connected")
}

/// A tool with no parameters.
fn tool(name: &str) -> Json {
    json!({"name": name, "inputSchema": {"type": "object"}})
}

/// A result of one text block.
fn text(words: &str) -> Json {
    json!({"content": [{"type": "text", "text": words}]})
}

/// Gives every task that can run a turn, many times over: for a test that
/// asserts that something has *not* happened once everything ready ran.
async fn settle() {
    for _ in 0..256 {
        tokio::task::yield_now().await;
    }
}

/// Completes when `ready` holds, asked between turns of the other tasks.
async fn until(ready: impl Fn() -> bool) {
    while !ready() {
        tokio::task::yield_now().await;
    }
}

/// The id of the `index`th request of `method` the server received.
fn id_of(server: &FakeServer, method: &str, index: usize) -> Json {
    server.of(method)[index]["id"].clone()
}

/// A process of a test's own, for a server the fake cannot play: it exits
/// when the test says, or when it is shut down.
struct HandProcess {
    exited: Arc<tokio::sync::watch::Sender<Option<String>>>,
}

impl cineris_mcp::ServerProcess for HandProcess {
    fn stderr_tail(&self) -> String {
        String::new()
    }

    fn exited(&self) -> cineris_tools::BoxFuture<'_, String> {
        Box::pin(async move {
            let mut exited = self.exited.subscribe();
            match exited.wait_for(Option::is_some).await {
                Ok(words) => words.clone().unwrap_or_default(),
                Err(_) => "gone".to_owned(),
            }
        })
    }

    fn shutdown(&self, _grace: Duration) -> cineris_tools::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.exited.send_replace(Some("shut down".to_owned()));
        })
    }
}

/// A closure for a server that cannot be started again.
fn no_respawn() -> cineris_mcp::Respawn {
    Arc::new(|| {
        Err(cineris_mcp::SpawnError::Failed(
            "not in this test".to_owned(),
        ))
    })
}

/// One message as a line of a server's output.
fn line(message: &Json) -> Result<Vec<u8>, cineris_mcp::LineError> {
    Ok(message.to_string().into_bytes())
}
