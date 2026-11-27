//! The protocol over a [`Transport`]: requests under their limits, the
//! eras' handshakes, listings, calls, and the change stream
//! (`docs/steps/30/answered.md`, points 1, 3.5 and 3.7).
//!
//! Nothing here knows whether the server is a process or an address. The
//! standard I/O client ([`crate::StdioClient`]) puts a session over its
//! transport after the probe; a Streamable HTTP client puts one over its
//! own.

use std::future::poll_fn;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use cineris_base::{CancelToken, SharedTimer, Sleep};
use cineris_tools::Json;
use serde_json::{Map, json};
use tokio::sync::{Semaphore, watch};
use tokio::task::AbortHandle;

use crate::client::{CallOutput, ClientError, Era, ToolDefinition};
use crate::jsonrpc::{self, Answer, Message, RequestId};
use crate::transport::{Inbound, Transport, lock};
use crate::wire::{self, ResultType};

/// The limits a client works under. The defaults are those of
/// `docs/steps/30/answered.md`.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// How long a server has to answer `server/discover` before it is taken
    /// to speak the legacy era (3 s). The whole start, the probe included,
    /// is held to the definition's start limit.
    pub probe_timeout: Duration,
    /// How long one listing of tools may take, every page included (30 s).
    pub list_timeout: Duration,
    /// How long a server has to exit after its input closes, before the
    /// staged kill (2 s).
    pub shutdown_grace: Duration,
    /// How many requests may wait for their answers at once; more wait for
    /// room, within their limits (32). The change stream is not counted.
    pub max_in_flight: usize,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            probe_timeout: Duration::from_secs(3),
            list_timeout: Duration::from_secs(30),
            shutdown_grace: Duration::from_secs(2),
            max_in_flight: 32,
        }
    }
}

/// At most this many tools a server.
pub const MAX_TOOLS: usize = 512;

/// At most this many pages of a listing.
pub const MAX_PAGES: usize = 20;

/// At most this many times a call is sent again for a server that needs
/// more (`input_required`).
pub const MAX_ROUNDS: usize = 3;

/// At most this many bytes of tool definitions in a listing, every page
/// counted, as the client holds them in memory: each JSON value at its
/// size in memory, every copy a definition keeps (`raw` and the fields read
/// from it). A listing the client returns holds at most this much.
pub const MAX_LISTING_BYTES: usize = 32 * 1024 * 1024;

/// How long the client waits before it opens a change stream again after
/// the server ended it.
pub const LISTEN_AGAIN: Duration = Duration::from_secs(1);

/// A time limit, kept across the requests of one piece of work: the pages
/// of a listing, the rounds of a call.
pub(crate) struct Limit {
    sleep: Sleep,
    what: &'static str,
    length: Duration,
    passed: bool,
}

impl Limit {
    /// A limit of `length` from now, for `what` ("list its tools").
    pub(crate) fn new(timer: &SharedTimer, what: &'static str, length: Duration) -> Self {
        Self {
            sleep: timer.sleep(length),
            what,
            length,
            passed: false,
        }
    }

    fn error(&self) -> ClientError {
        ClientError::TimedOut {
            what: self.what,
            limit: self.length,
        }
    }
}

/// Runs `work` until it ends, `limit` passes or `cancel` is cancelled,
/// whichever is first. A token already cancelled wins.
pub(crate) async fn bounded<F: Future>(
    work: F,
    mut limit: Option<&mut Limit>,
    cancel: Option<&CancelToken>,
) -> Result<F::Output, ClientError> {
    let never = CancelToken::new();
    let mut cancelled = pin!(cancel.unwrap_or(&never).cancelled());
    let mut work = pin!(work);
    poll_fn(|context| {
        if cancelled.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(ClientError::Cancelled));
        }
        if let Poll::Ready(output) = work.as_mut().poll(context) {
            return Poll::Ready(Ok(output));
        }
        if let Some(limit) = limit.as_deref_mut()
            && (limit.passed || limit.sleep.as_mut().poll(context).is_ready())
        {
            limit.passed = true;
            return Poll::Ready(Err(limit.error()));
        }
        Poll::Pending
    })
    .await
}

/// Requests over a transport: their ids, how many wait at once, and what
/// happens to one given up.
pub(crate) struct Rpc {
    transport: Arc<dyn Transport>,
    ids: AtomicU64,
    room: Semaphore,
}

impl Rpc {
    /// Requests over `transport`, at most `in_flight` waiting at once.
    pub(crate) fn new(transport: Arc<dyn Transport>, in_flight: usize) -> Self {
        Self {
            transport,
            ids: AtomicU64::new(1),
            room: Semaphore::new(in_flight.max(1)),
        }
    }

    /// The transport.
    pub(crate) fn transport(&self) -> &Arc<dyn Transport> {
        &self.transport
    }

    /// A new id, never used before on this connection.
    pub(crate) fn next_id(&self) -> u64 {
        self.ids.fetch_add(1, Ordering::Relaxed)
    }

    /// Sends a request and waits for its answer, within `limit` and until
    /// `cancel`. When either ends it, a request that was sent is cancelled
    /// as the transport does it, if `cancels` (never for `initialize`, nor
    /// for the probe of a server that may not know the method).
    pub(crate) async fn ask(
        &self,
        method: &str,
        params: Json,
        limit: Option<&mut Limit>,
        cancel: Option<&CancelToken>,
        cancels: bool,
    ) -> Result<Answer, ClientError> {
        let id = self.next_id();
        let message = jsonrpc::request(id, method, params);
        let sent = AtomicBool::new(false);
        let work = async {
            let _room = self
                .room
                .acquire()
                .await
                .map_err(|_| ClientError::Stopped("the client closed".to_owned()))?;
            sent.store(true, Ordering::Relaxed);
            self.transport.request(id, message).await
        };
        match bounded(work, limit, cancel).await {
            Ok(answer) => answer,
            Err(stopped) => {
                if cancels && sent.load(Ordering::Relaxed) {
                    let reason = match stopped {
                        ClientError::Cancelled => "the client cancelled it",
                        _ => "it took longer than the client's limit",
                    };
                    self.transport.cancel(id, reason);
                }
                Err(stopped)
            }
        }
    }
}

/// The change stream's state, as the server's messages move it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Listen {
    /// The id of the `subscriptions/listen` open now.
    id: Option<u64>,
    /// Whether the server acknowledged it.
    acked: bool,
    /// Whether it is the stream opened again after one the server ended.
    reopened: bool,
    /// Whether the server ended it with `notifications/cancelled`.
    ended: bool,
    /// Whether the server acknowledged it without the change notices.
    refused: bool,
}

/// What a server sends that is not an answer: its requests, the change
/// notices, the change stream's acknowledgement and end.
#[derive(Debug)]
pub(crate) struct Router {
    era: Mutex<Option<Era>>,
    changes: watch::Sender<u64>,
    listen: watch::Sender<Listen>,
    /// Requests from a modern server, which may send none.
    faults: AtomicU64,
}

impl Router {
    /// A router for a connection whose era is not known yet.
    pub(crate) fn new() -> Self {
        Self {
            era: Mutex::new(None),
            changes: watch::channel(0).0,
            listen: watch::channel(Listen::default()).0,
            faults: AtomicU64::new(0),
        }
    }

    /// The era found: until it is, a server's requests are answered as a
    /// legacy server's are, since only a legacy server may send one.
    pub(crate) fn set_era(&self, era: Era) {
        *lock(&self.era) = Some(era);
    }

    fn era(&self) -> Option<Era> {
        *lock(&self.era)
    }

    /// The counter of changes, bumped once for each notice: a burst of
    /// them is one change to a receiver that has not looked yet.
    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn bump(&self) {
        self.changes
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// How many requests a modern server sent, which it may not.
    pub(crate) fn faults(&self) -> u64 {
        self.faults.load(Ordering::Relaxed)
    }

    fn open_listen(&self, id: u64, reopened: bool) {
        self.listen.send_replace(Listen {
            id: Some(id),
            reopened,
            ..Listen::default()
        });
    }

    fn close_listen(&self) {
        self.listen.send_replace(Listen::default());
    }

    /// The id of the change stream open now.
    pub(crate) fn listen_id(&self) -> Option<u64> {
        self.listen.borrow().id
    }

    /// Whether the change stream open now was acknowledged.
    pub(crate) fn listen_acked(&self) -> bool {
        self.listen.borrow().acked
    }

    /// Whether `params` name the change stream open now.
    fn on_stream(&self, params: Option<&Json>) -> bool {
        let named = params
            .and_then(|params| params.get("_meta"))
            .and_then(|meta| meta.get(wire::SUBSCRIPTION_ID))
            .and_then(RequestId::from_json)
            .and_then(|id| id.ours());
        named.is_some() && named == self.listen_id()
    }

    fn notified(&self, method: &str, params: Option<&Json>) {
        let era = self.era();
        match (method, era) {
            ("notifications/tools/list_changed", Some(Era::Legacy)) => self.bump(),
            ("notifications/tools/list_changed", Some(Era::Modern)) => {
                if self.on_stream(params) && self.listen_acked() {
                    self.bump();
                }
            }
            ("notifications/subscriptions/acknowledged", Some(Era::Modern)) => {
                if !self.on_stream(params) {
                    return;
                }
                let honoured = params
                    .and_then(|params| params.pointer("/notifications/toolsListChanged"))
                    .and_then(Json::as_bool)
                    == Some(true);
                let mut missed = false;
                self.listen.send_if_modified(|listen| {
                    if listen.acked {
                        return false;
                    }
                    listen.acked = true;
                    listen.refused = !honoured;
                    missed = honoured && listen.reopened;
                    true
                });
                if missed {
                    // Changes made while no stream was open were missed.
                    self.bump();
                }
            }
            ("notifications/cancelled", Some(Era::Modern)) => {
                let ended = params
                    .and_then(|params| params.get("requestId"))
                    .and_then(RequestId::from_json)
                    .and_then(|id| id.ours());
                if ended.is_some() {
                    self.listen.send_if_modified(|listen| {
                        if listen.id != ended || listen.ended {
                            return false;
                        }
                        listen.ended = true;
                        true
                    });
                }
            }
            // Anything else is not the client's business: progress, log
            // lines, the other lists, notices it never asked for.
            _ => {}
        }
    }
}

impl Inbound for Router {
    fn handle(&self, message: Message) -> Option<Json> {
        match message {
            Message::Request { id, method, .. } => {
                if self.era() == Some(Era::Modern) {
                    // A modern server sends no requests: a fault, counted
                    // with what it costs and otherwise ignored.
                    self.faults.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
                Some(if method == "ping" {
                    jsonrpc::result(&id, json!({}))
                } else {
                    jsonrpc::error(
                        &id,
                        jsonrpc::METHOD_NOT_FOUND,
                        "the client does not offer this method",
                    )
                })
            }
            Message::Notification { method, params } => {
                self.notified(&method, params.as_ref());
                None
            }
            Message::Response { .. } | Message::Unaddressed(_) => None,
        }
    }
}

/// One listing, kept for those who asked while it ran.
struct Listed {
    /// Which listing it was, counted from 1.
    number: u64,
    /// The count of changes when it began.
    generation: u64,
    /// Its tools, kept only while someone who asked during it has not
    /// taken them: no copy is kept once all have.
    tools: Option<Vec<ToolDefinition>>,
    /// How many of those who asked during it have yet to take them.
    owed: usize,
}

/// One who asks for a listing, counted while it waits or lists.
struct Asking<'a>(&'a AtomicU64);

impl<'a> Asking<'a> {
    fn new(count: &'a AtomicU64) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self(count)
    }
}

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The protocol, both eras, over one connection whose handshake is done.
pub(crate) struct Session {
    rpc: Rpc,
    router: Arc<Router>,
    era: Era,
    version: String,
    timer: SharedTimer,
    options: ClientOptions,
    /// One listing at a time: who asks while one runs waits for it.
    listing: tokio::sync::Mutex<Option<Listed>>,
    /// How many listings have finished.
    listings: AtomicU64,
    /// How many ask for a listing now, waiting for it or running it.
    asking: AtomicU64,
    /// The wait for the end of the last listing's lifetime.
    lifetime: Mutex<Option<AbortHandle>>,
    /// Cancelled when the client shuts down: ends the change stream.
    stop: CancelToken,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("era", &self.era)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// A session of `era` at `version` over `rpc`, whose server's messages
    /// `router` handles.
    pub(crate) fn new(
        rpc: Rpc,
        router: Arc<Router>,
        era: Era,
        version: String,
        timer: SharedTimer,
        options: ClientOptions,
    ) -> Arc<Self> {
        router.set_era(era);
        rpc.transport().agreed(era, &version);
        Arc::new(Self {
            rpc,
            router,
            era,
            version,
            timer,
            options,
            listing: tokio::sync::Mutex::new(None),
            listings: AtomicU64::new(0),
            asking: AtomicU64::new(0),
            lifetime: Mutex::new(None),
            stop: CancelToken::new(),
        })
    }

    /// The router of the server's messages.
    pub(crate) fn router(&self) -> &Arc<Router> {
        &self.router
    }

    /// `params` with the `_meta` every modern request carries.
    fn params(&self, mut params: Map<String, Json>) -> Json {
        if self.era == Era::Modern {
            params.insert("_meta".to_owned(), wire::request_meta(&self.version));
        }
        Json::Object(params)
    }

    /// Every tool the server offers, every page read, within the listing's
    /// limit. Who asks while a listing runs waits for it, and shares it if
    /// no change came since it began.
    pub(crate) async fn list_tools(
        self: &Arc<Self>,
        cancel: &CancelToken,
    ) -> Result<Vec<ToolDefinition>, ClientError> {
        let mut limit = Limit::new(&self.timer, "list its tools", self.options.list_timeout);
        let arrived = self.listings.load(Ordering::Acquire);
        let _asking = Asking::new(&self.asking);
        let mut listed = bounded(self.listing.lock(), Some(&mut limit), Some(cancel)).await?;
        let generation = *self.router.changes.borrow();
        if let Some(last) = listed.as_mut()
            && last.number > arrived
            && last.generation == generation
            && last.tools.is_some()
        {
            // The last of those who asked during it takes the kept copy.
            last.owed = last.owed.saturating_sub(1);
            let tools = if last.owed == 0 {
                last.tools.take()
            } else {
                last.tools.clone()
            };
            return Ok(tools.unwrap_or_default());
        }
        let (tools, lifetime) = self.list_pages(&mut limit, cancel).await?;
        let number = self.listings.fetch_add(1, Ordering::AcqRel) + 1;
        // Those asking now, besides this one, began waiting during it.
        let owed = usize::try_from(self.asking.load(Ordering::Acquire).saturating_sub(1))
            .unwrap_or(usize::MAX);
        *listed = Some(Listed {
            number,
            generation,
            tools: (owed > 0).then(|| tools.clone()),
            owed,
        });
        drop(listed);
        self.keep_for(lifetime);
        Ok(tools)
    }

    async fn list_pages(
        &self,
        limit: &mut Limit,
        cancel: &CancelToken,
    ) -> Result<(Vec<ToolDefinition>, Duration), ClientError> {
        let mut tools: Vec<ToolDefinition> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut lifetime = u64::MAX;
        let mut bytes = 0usize;
        for _ in 0..MAX_PAGES {
            let mut params = Map::new();
            if let Some(cursor) = cursor.take() {
                params.insert("cursor".to_owned(), Json::from(cursor));
            }
            let result = self
                .rpc
                .ask(
                    "tools/list",
                    self.params(params),
                    Some(limit),
                    Some(cancel),
                    true,
                )
                .await
                .map_err(settled)?
                .map_err(wire::server_error)?;
            if self.era == Era::Modern && wire::result_type(&result)? != ResultType::Complete {
                return Err(ClientError::Protocol(
                    "it asked for input to list its tools, which no listing may".to_owned(),
                ));
            }
            let page = wire::tools_page(result)?;
            lifetime = lifetime.min(page.ttl_ms);
            if tools.len() + page.tools.len() > MAX_TOOLS {
                return Err(ClientError::Protocol(format!(
                    "it offers more than {MAX_TOOLS} tools"
                )));
            }
            bytes = bytes.saturating_add(page.bytes);
            if bytes > MAX_LISTING_BYTES {
                return Err(ClientError::Protocol(format!(
                    "its list of tools takes more than {} MiB in memory",
                    MAX_LISTING_BYTES / 1024 / 1024
                )));
            }
            tools.extend(page.tools);
            let Some(next) = page.next else {
                let lifetime = match self.era {
                    Era::Modern => Duration::from_millis(lifetime),
                    Era::Legacy => Duration::ZERO,
                };
                return Ok((tools, lifetime));
            };
            cursor = Some(next);
        }
        Err(ClientError::Protocol(format!(
            "its list of tools ran past {MAX_PAGES} pages"
        )))
    }

    /// Bumps the changes when a listing's lifetime ends. A lifetime of zero
    /// is stale at once and bumps nothing, or a server that gives none
    /// would be listed without end.
    fn keep_for(&self, lifetime: Duration) {
        let waiting = (!lifetime.is_zero()).then(|| {
            let router = Arc::clone(&self.router);
            let sleep = self.timer.sleep(lifetime);
            tokio::spawn(async move {
                sleep.await;
                router.bump();
            })
            .abort_handle()
        });
        if let Some(previous) = std::mem::replace(&mut *lock(&self.lifetime), waiting) {
            previous.abort();
        }
    }

    /// Calls `tool` with `arguments` within `limit`, sending it again for a
    /// modern server that asks with only a state to give back.
    pub(crate) async fn call(
        &self,
        tool: &str,
        arguments: &Json,
        cancel: &CancelToken,
        limit: Duration,
    ) -> Result<CallOutput, ClientError> {
        let mut limit = Limit::new(&self.timer, "answer the call", limit);
        let mut state: Option<String> = None;
        let mut answered = false;
        for _ in 0..=MAX_ROUNDS {
            let params = wire::call_params(tool, arguments, state.as_deref());
            let asked = self
                .rpc
                .ask(
                    "tools/call",
                    self.params(params),
                    Some(&mut limit),
                    Some(cancel),
                    true,
                )
                .await;
            let result = match asked {
                // A round that ran before this one was lost: the call may
                // have run.
                Err(ClientError::Stopped(words)) if answered => {
                    return Err(ClientError::MayHaveRun(words));
                }
                other => other?.map_err(wire::server_error)?,
            };
            answered = true;
            if self.era == Era::Legacy || wire::result_type(&result)? == ResultType::Complete {
                return Ok(wire::call_output(result, self.era));
            }
            let asks = result.get("inputRequests").is_some_and(|requests| {
                !(requests.is_null() || requests.as_object().is_some_and(Map::is_empty))
            });
            if asks {
                return Err(ClientError::Protocol(
                    "it asked for input the client never said it could give".to_owned(),
                ));
            }
            let Some(next) = result.get("requestState").and_then(Json::as_str) else {
                return Err(ClientError::Protocol(
                    "it asked for the call again with no state to give back".to_owned(),
                ));
            };
            state = Some(next.to_owned());
        }
        Err(ClientError::Protocol(format!(
            "it asked for the call again more than {MAX_ROUNDS} times"
        )))
    }

    /// Keeps a change stream open for as long as the session lasts: opened
    /// again a second after the server ends it, and not again after it
    /// refuses one.
    pub(crate) async fn listen(self: Arc<Self>) {
        let mut reopened = false;
        loop {
            let id = self.rpc.next_id();
            self.router.open_listen(id, reopened);
            let mut params = Map::new();
            params.insert(
                "notifications".to_owned(),
                json!({"toolsListChanged": true}),
            );
            let message = jsonrpc::request(id, "subscriptions/listen", self.params(params));
            let ended = self.listened(id, message).await;
            match ended {
                Listened::Stopped | Listened::Closed => return,
                Listened::Declined => {
                    self.router.close_listen();
                    return;
                }
                Listened::Refused => {
                    self.rpc
                        .transport()
                        .cancel(id, "the server does not send what was asked for");
                    self.router.close_listen();
                    return;
                }
                Listened::EndedByServer => {}
            }
            self.router.close_listen();
            let again = self.timer.sleep(LISTEN_AGAIN);
            if bounded(again, None, Some(&self.stop)).await.is_err() {
                return;
            }
            reopened = true;
        }
    }

    /// Waits for the end of the change stream numbered `id`.
    async fn listened(&self, id: u64, message: Json) -> Listened {
        let mut events = self.router.listen.subscribe();
        let server = async {
            events
                .wait_for(|listen| listen.id == Some(id) && (listen.ended || listen.refused))
                .await
                .map(|listen| listen.refused)
                .ok()
        };
        let mut server = pin!(server);
        let mut request = pin!(self.rpc.transport().request(id, message));
        let mut stop = pin!(self.stop.cancelled());
        poll_fn(|context| {
            if stop.as_mut().poll(context).is_ready() {
                return Poll::Ready(Listened::Stopped);
            }
            match request.as_mut().poll(context) {
                // The server's own end of it: a result.
                Poll::Ready(Ok(Ok(_))) => return Poll::Ready(Listened::EndedByServer),
                // An error: it will not keep one, so none is asked again.
                Poll::Ready(Ok(Err(_))) => return Poll::Ready(Listened::Declined),
                Poll::Ready(Err(_)) => return Poll::Ready(Listened::Closed),
                Poll::Pending => {}
            }
            match server.as_mut().poll(context) {
                Poll::Ready(Some(true)) => Poll::Ready(Listened::Refused),
                Poll::Ready(Some(false)) => Poll::Ready(Listened::EndedByServer),
                Poll::Ready(None) => Poll::Ready(Listened::Closed),
                Poll::Pending => Poll::Pending,
            }
        })
        .await
    }

    /// Ends the change stream and the lifetime's wait, at shutdown.
    pub(crate) fn stop(&self) {
        if let Some(id) = self.router.listen_id() {
            self.rpc
                .transport()
                .cancel(id, "the client is shutting down");
        }
        self.router.close_listen();
        self.stop.cancel();
        if let Some(lifetime) = lock(&self.lifetime).take() {
            lifetime.abort();
        }
    }
}

/// How a change stream ended.
enum Listened {
    /// The client is shutting down.
    Stopped,
    /// The connection ended.
    Closed,
    /// The server ended it: it is opened again.
    EndedByServer,
    /// The server will not send change notices on it: it is cancelled.
    Refused,
    /// The server answered it with an error: it is over, and not asked for
    /// again.
    Declined,
}

/// What a server answered `server/discover` with.
#[derive(Debug)]
pub(crate) enum Probe {
    /// A modern server: the revision chosen, its name and whether it sends
    /// change notices.
    Modern(Agreed),
    /// A modern server that does not speak the revision asked for: those it
    /// does.
    Unsupported(Vec<String>),
    /// A modern server that cannot be spoken to: why.
    Refused(String),
    /// Any other error, a result of another kind, or silence: a legacy
    /// server. What it answered, for a message.
    Legacy(String),
    /// The connection ended before an answer: a legacy server that exits
    /// on a request before `initialize`.
    Exited(ClientError),
    /// The server was ended as misbehaving while it was asked.
    Failed(ClientError),
}

/// What the client and a server agreed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Agreed {
    pub(crate) era: Era,
    pub(crate) version: String,
    pub(crate) name: Option<String>,
    pub(crate) list_changed: bool,
}

/// Asks the server what it speaks, with `version` as the revision of the
/// request, within `limit`. The probe is never cancelled: a legacy server
/// would not know the request it named.
pub(crate) async fn discover(rpc: &Rpc, version: &str, limit: &mut Limit) -> Probe {
    let answered = rpc
        .ask(
            "server/discover",
            discover_params(version),
            Some(limit),
            None,
            false,
        )
        .await;
    probed(answered, version)
}

/// The parameters of `server/discover` asked with `version`.
pub(crate) fn discover_params(version: &str) -> Json {
    let mut params = Map::new();
    params.insert("_meta".to_owned(), wire::request_meta(version));
    Json::Object(params)
}

/// What a server's answer to `server/discover`, asked with `version`, says
/// it speaks.
pub(crate) fn probed(answered: Result<Answer, ClientError>, version: &str) -> Probe {
    match answered {
        Err(ClientError::TimedOut { .. }) => Probe::Legacy("nothing".to_owned()),
        Err(stopped @ (ClientError::Stopped(_) | ClientError::MayHaveRun(_))) => {
            Probe::Exited(settled(stopped))
        }
        Err(other) => Probe::Failed(other),
        Ok(Ok(result)) => {
            let Some(supported) = result.get("supportedVersions").and_then(Json::as_array) else {
                // A result that is not the discovery's is a legacy server
                // answering what it does not know.
                return Probe::Legacy("a result that is not a discovery result".to_owned());
            };
            let supported: Vec<&str> = supported.iter().filter_map(Json::as_str).collect();
            let chosen = if supported.contains(&version) {
                Some(version)
            } else {
                wire::MODERN_VERSIONS
                    .iter()
                    .copied()
                    .find(|ours| supported.contains(ours))
            };
            let Some(chosen) = chosen else {
                return Probe::Refused(format!(
                    "it speaks only {}, none of which this client speaks",
                    shown(&supported)
                ));
            };
            let (name, list_changed) = wire::about(&result, Era::Modern);
            Probe::Modern(Agreed {
                era: Era::Modern,
                version: chosen.to_owned(),
                name,
                list_changed,
            })
        }
        Ok(Err(error)) => match error.code {
            wire::UNSUPPORTED_VERSION => Probe::Unsupported(wire::strings(
                error.data.as_ref().and_then(|data| data.get("supported")),
            )),
            wire::MISSING_CLIENT_CAPABILITY => Probe::Refused(
                "it needs a capability of the client's that this client does not have".to_owned(),
            ),
            wire::HEADER_MISMATCH => {
                Probe::Refused("it found the client's request malformed (error -32020)".to_owned())
            }
            _ => match wire::server_error(error) {
                ClientError::Server { code, message } => {
                    Probe::Legacy(format!("error {code}: {message}"))
                }
                other => Probe::Legacy(other.to_string()),
            },
        },
    }
}

/// Why `initialize` did not agree a revision.
#[derive(Debug)]
pub(crate) enum Declined {
    /// The server answered with an error: a modern server, or a legacy one
    /// that will not be spoken to.
    Answered(ClientError),
    /// The connection ended first.
    Stopped(ClientError),
    /// Something else: a revision the client does not speak, a server
    /// ended as misbehaving.
    Failed(ClientError),
}

/// The legacy handshake: `initialize` with the newest legacy revision,
/// then `notifications/initialized`. It has no limit of its own (the start
/// has one) and is never cancelled.
pub(crate) async fn initialize(rpc: &Rpc) -> Result<Agreed, Declined> {
    let params = json!({
        "protocolVersion": wire::LEGACY_VERSIONS[0],
        "capabilities": {},
        "clientInfo": wire::client_info(),
    });
    let result = match rpc.ask("initialize", params, None, None, false).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => return Err(Declined::Answered(wire::server_error(error))),
        Err(stopped @ (ClientError::Stopped(_) | ClientError::MayHaveRun(_))) => {
            return Err(Declined::Stopped(settled(stopped)));
        }
        Err(other) => return Err(Declined::Failed(other)),
    };
    let version = result
        .get("protocolVersion")
        .and_then(Json::as_str)
        .unwrap_or_default();
    if !wire::LEGACY_VERSIONS.contains(&version) {
        return Err(Declined::Failed(ClientError::NotStarted(format!(
            "it answered `initialize` with protocol revision {}, which this client does not speak",
            shown(&[version])
        ))));
    }
    let (name, list_changed) = wire::about(&result, Era::Legacy);
    rpc.transport()
        .send(jsonrpc::notification("notifications/initialized", None));
    Ok(Agreed {
        era: Era::Legacy,
        version: version.to_owned(),
        name,
        list_changed,
    })
}

/// A request lost to the end of the connection, for a request that changes
/// nothing on the server: it stopped, and nothing "may have run".
pub(crate) fn settled(error: ClientError) -> ClientError {
    match error {
        ClientError::MayHaveRun(words) => ClientError::Stopped(words),
        other => other,
    }
}

/// Revisions a server named, as a message shows them: quoted, at most
/// eight, each cut.
pub(crate) fn shown(versions: &[&str]) -> String {
    let mut shown: Vec<String> = versions
        .iter()
        .take(8)
        .map(|version| format!("{:?}", cineris_base::text::truncate_bytes(version, 32)))
        .collect();
    if versions.len() > 8 {
        shown.push("…".to_owned());
    }
    if shown.is_empty() {
        return "nothing".to_owned();
    }
    shown.join(", ")
}
