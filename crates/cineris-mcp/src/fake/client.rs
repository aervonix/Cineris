//! A client for tests, with no server, no process and no protocol: behind
//! the `test-kit` feature, which only dev-dependencies turn on (`cargo
//! xtask rules`).
//!
//! [`FakeClient`] is a [`Client`], and so a [`ToolCaller`](crate::ToolCaller):
//! its tools are a list the test sets, each call is answered as the test
//! scripted it (or with its arguments given back), and a call of a tool the
//! test holds waits at a [`CallGate`] until the test opens it, or its
//! cancel token fires. [`definition`] makes a tool's definition as a server
//! would send it.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use cineris_base::CancelToken;
use cineris_tools::{BoxFuture, Json};
use serde_json::json;
use tokio::sync::watch;

use crate::client::{CallOutput, Client, ClientError, Content, Era, ServerInfo, ToolDefinition};
use crate::transport::lock;

/// A tool's definition as a server sends it: `raw` holds the same name,
/// description and schema.
#[must_use]
pub fn definition(name: &str, description: &str, input_schema: Json) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        title: None,
        description: Some(description.to_owned()),
        raw: json!({"name": name, "description": description, "inputSchema": input_schema}),
        input_schema,
        output_schema: None,
        annotations: None,
    }
}

/// A result of text blocks alone.
#[must_use]
pub fn text_output(blocks: &[&str]) -> CallOutput {
    CallOutput {
        content: blocks
            .iter()
            .map(|text| Content::Text((*text).to_owned()))
            .collect(),
        structured: None,
        is_error: false,
    }
}

/// A place a call waits at until the test opens it. Clones are the same
/// gate.
#[derive(Clone)]
pub struct CallGate {
    open: Arc<watch::Sender<bool>>,
    reached: Arc<watch::Sender<usize>>,
}

impl std::fmt::Debug for CallGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallGate")
            .field("open", &*self.open.borrow())
            .field("reached", &*self.reached.borrow())
            .finish()
    }
}

impl Default for CallGate {
    fn default() -> Self {
        Self {
            open: Arc::new(watch::channel(false).0),
            reached: Arc::new(watch::channel(0).0),
        }
    }
}

impl CallGate {
    /// Lets every call waiting here, and every later one, go on.
    pub fn open(&self) {
        self.open.send_replace(true);
    }

    /// Resolves once a call has come to the gate.
    pub async fn reached(&self) {
        let mut reached = self.reached.subscribe();
        // The sender is held by `self`: it cannot close while this waits.
        let _ = reached.wait_for(|count| *count > 0).await;
    }

    /// How many calls have come to the gate.
    #[must_use]
    pub fn arrivals(&self) -> usize {
        *self.reached.borrow()
    }

    async fn pass(&self) {
        let mut open = self.open.subscribe();
        self.reached.send_modify(|count| *count += 1);
        let _ = open.wait_for(|open| *open).await;
    }
}

/// A client the test scripts. See the module's description.
pub struct FakeClient {
    info: ServerInfo,
    tools: Mutex<Vec<ToolDefinition>>,
    answers: Mutex<BTreeMap<String, VecDeque<Result<CallOutput, ClientError>>>>,
    gates: Mutex<BTreeMap<String, CallGate>>,
    calls: Mutex<Vec<(String, Json)>>,
    changes: watch::Sender<u64>,
    closed: watch::Sender<Option<ClientError>>,
}

impl std::fmt::Debug for FakeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeClient")
            .field("tools", &lock(&self.tools).len())
            .field("calls", &lock(&self.calls).len())
            .finish_non_exhaustive()
    }
}

impl FakeClient {
    /// A server of the current era that offers `tools`.
    #[must_use]
    pub fn new(tools: Vec<ToolDefinition>) -> Arc<Self> {
        Arc::new(Self {
            info: ServerInfo {
                era: Era::Modern,
                version: "2026-07-28".to_owned(),
                name: Some("fake".to_owned()),
                list_changed: true,
            },
            tools: Mutex::new(tools),
            answers: Mutex::new(BTreeMap::new()),
            gates: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(Vec::new()),
            changes: watch::channel(0).0,
            closed: watch::channel(None).0,
        })
    }

    /// Answers the next call of `tool` with `answer`: the answers of a tool
    /// are taken in the order given, and a call with none left is given
    /// its arguments back as text.
    pub fn answer(&self, tool: &str, answer: Result<CallOutput, ClientError>) {
        lock(&self.answers)
            .entry(tool.to_owned())
            .or_default()
            .push_back(answer);
    }

    /// Holds every call of `tool` at a gate until the test opens it.
    #[must_use]
    pub fn hold(&self, tool: &str) -> CallGate {
        lock(&self.gates)
            .entry(tool.to_owned())
            .or_default()
            .clone()
    }

    /// The calls made, in order: each tool's name and its arguments.
    #[must_use]
    pub fn calls(&self) -> Vec<(String, Json)> {
        lock(&self.calls).clone()
    }

    /// Changes the tools it offers, and says they changed.
    pub fn set_tools(&self, tools: Vec<ToolDefinition>) {
        *lock(&self.tools) = tools;
        self.changes.send_modify(|count| *count += 1);
    }

    /// Ends the connection, for `why`: every later call fails with it.
    pub fn close(&self, why: ClientError) {
        self.closed.send_replace(Some(why));
    }

    fn closed_with(&self) -> Option<ClientError> {
        self.closed.borrow().clone()
    }
}

impl Client for FakeClient {
    fn info(&self) -> &ServerInfo {
        &self.info
    }

    fn list_tools<'a>(
        &'a self,
        _cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<Vec<ToolDefinition>, ClientError>> {
        Box::pin(async move {
            match self.closed_with() {
                Some(why) => Err(why),
                None => Ok(lock(&self.tools).clone()),
            }
        })
    }

    fn call<'a>(
        &'a self,
        tool: &'a str,
        arguments: Json,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<CallOutput, ClientError>> {
        Box::pin(async move {
            if let Some(why) = self.closed_with() {
                return Err(why);
            }
            lock(&self.calls).push((tool.to_owned(), arguments.clone()));
            let gate = lock(&self.gates).get(tool).cloned();
            if let Some(gate) = gate
                && cancel.race(gate.pass()).await.is_none()
            {
                return Err(ClientError::Cancelled);
            }
            if cancel.is_cancelled() {
                return Err(ClientError::Cancelled);
            }
            let scripted = lock(&self.answers)
                .get_mut(tool)
                .and_then(VecDeque::pop_front);
            scripted
                .unwrap_or_else(|| Ok(text_output(&[&format!("{tool} called with {arguments}")])))
        })
    }

    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn closed(&self) -> BoxFuture<'_, ClientError> {
        Box::pin(async move {
            let mut closed = self.closed.subscribe();
            match closed.wait_for(Option::is_some).await {
                Ok(why) => why
                    .clone()
                    .unwrap_or_else(|| ClientError::Stopped("closed".to_owned())),
                Err(_) => ClientError::Stopped("closed".to_owned()),
            }
        })
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.close(ClientError::Stopped("shut down".to_owned()));
        })
    }
}
