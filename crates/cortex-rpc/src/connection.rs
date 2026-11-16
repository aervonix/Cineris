//! One client's connection: lines in, operations through the lane, lines
//! out (`docs/architecture/11-rpc-protocol.md`; `docs/steps/27/answered.md`,
//! "The design, before building").
//!
//! Three parts run at once on the core's runtime:
//!
//! - **The reader** takes the client's bytes, splits them into lines and
//!   reads each: a request goes to the lane (or is answered at once when it
//!   cannot be run), an answer to a request the server put goes to
//!   `Core::respond`. **It never stops reading** while the connection is
//!   open: the answer an operation waits on comes in on the same stream.
//! - **The lane** ([`Dispatcher`]) runs the operations in the order every
//!   front end's run in.
//! - **The writer** takes the next thing to send, in [`Order`], only once
//!   the line before it has gone: a client that reads slowly leaves its
//!   backlog in the core's queue, where deltas merge and then drop. While a
//!   line waits, it looks at the client's queue at most once a second, and
//!   closes the connection when whole events wait past the queue's capacity
//!   and the client has taken no line for [`ServeOptions::stalled_client`],
//!   or when far more wait (02-runtime, "Event backpressure").
//!
//! **What the server holds for a client is bounded.** A request counts
//! against the connection's budget from when it is read until its result
//! is written: at most [`ServeOptions::max_requests`], and at most
//! [`ServeOptions::max_request_bytes`] of their lines. A request past
//! either is answered at once (`overloaded`, of kind `limit`) and not run.
//! Those answers, and the errors for lines that are not requests, are
//! counted until written too: a client with more of them unread than
//! requests may run is sending and not reading, and its connection is
//! closed ([`Ended::Flooded`]). Beside them: one line being read (the
//! framing's limit) and one being written.
//!
//! **The handshake.** Nothing but `initialize` is taken before it. Its role
//! and capabilities make the client's [`ClientInfo`]; the core then answers
//! it (the version, the role's validity), and a refused `initialize`
//! disconnects that client again and leaves the connection open for
//! another.
//!
//! **The end.** When the client's stream ends, the server stops or the
//! client stalls, the client is disconnected from the core first, which
//! withdraws what it was asked so that no question it never answered holds
//! its lane, and only then is the lane's operation waited for, at most
//! [`ServeOptions::lane_end`] (as the in-process link does: step 23's
//! second pull request).

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, poll_fn};
use std::io;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::Duration;

use cortex_base::framing::{DEFAULT_MAX_LINE_LEN, FrameDecoder};
use cortex_base::{CancelToken, SharedClock, SharedTimer, SystemClock};
use cortex_core::{Client, ClientInfo, Core, EventQueue, Outgoing, QUEUE_CAPACITY};
use cortex_protocol::error::code;
use cortex_protocol::event::Event;
use cortex_protocol::jsonrpc::{ErrorObject, Request, RequestId, Response};
use cortex_protocol::methods::{ClientRole, Empty, InitializeParams, Op, OpResult, ServerResponse};
use cortex_protocol::{ErrorData, ErrorKind};
use tokio::sync::{Notify, mpsc};

use crate::lane::{Dispatcher, Run};
use crate::order::{Next, Order};
use crate::transport::Transport;
use crate::wire::{self, Incoming};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every change behind these locks is whole before the lock is let go.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a connection is given.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Every wait of the server: the core's timer, so that a test's waits
    /// are on its fake clock.
    pub timer: SharedTimer,
    /// How long a client has taken nothing is read from it: the core's
    /// clock, which a test's timer moves.
    pub clock: SharedClock,
    /// How long a client may take no line while whole events wait past its
    /// queue's capacity, before its connection is closed (`rpc.stalled_client`).
    pub stalled_client: Duration,
    /// How many events may wait for a client before its deltas are dropped.
    pub queue: usize,
    /// The most requests of one client that may be running, or waiting to
    /// run or to have their results written.
    pub max_requests: usize,
    /// The most bytes of those requests' lines.
    pub max_request_bytes: usize,
    /// The longest line read.
    pub max_line: usize,
    /// A line longer than this is read on the blocking pool, so that the
    /// core thread is not held while it is parsed.
    pub long_line: usize,
    /// How long a connection's end waits for the operation its lane runs.
    pub lane_end: Duration,
    /// How long the end of the server waits for requests still running.
    pub requests_end: Duration,
    /// How long the end waits to write what is already queued.
    pub drain: Duration,
    /// Whether a client may ask for a session at `unrestricted`: only when
    /// the server's own command line or configuration says so
    /// (`docs/security.md`, section 5: a client is never more than the
    /// user at the terminal).
    pub allow_unrestricted: bool,
    /// How long a connection may go without `initialize` before it is
    /// closed: for a socket, which anyone of the user's may open and leave
    /// open. `None` for standard I/O, whose one client started the server.
    pub initialize_within: Option<Duration>,
}

impl ServeOptions {
    /// The defaults, on `timer` and the system's clock.
    #[must_use]
    pub fn new(timer: SharedTimer) -> Self {
        Self {
            timer,
            clock: Arc::new(SystemClock::new()),
            stalled_client: Duration::from_secs(30),
            queue: QUEUE_CAPACITY,
            max_requests: 256,
            max_request_bytes: 64 * 1024 * 1024,
            max_line: DEFAULT_MAX_LINE_LEN,
            long_line: 1024 * 1024,
            lane_end: Duration::from_secs(1),
            requests_end: Duration::from_secs(2),
            drain: Duration::from_secs(2),
            initialize_within: None,
            allow_unrestricted: false,
        }
    }
}

/// How often a writer that waits on its client looks at the client's
/// queue, at most. Only while a line is waiting to go: an idle connection
/// sets no timer.
const LOOK: Duration = Duration::from_secs(1);

/// How many times the queue's capacity may wait before a client that is
/// not reading is closed at once, whatever the time.
const CEILING: usize = 4;

/// How a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ended {
    /// The client's stream ended.
    Closed,
    /// The client's stream failed.
    Failed,
    /// Writing to the client failed: it has gone.
    Gone,
    /// The client took nothing while whole events waited past its queue's
    /// capacity, for too long or too many.
    Stalled,
    /// The client sent more than may wait for it and read none of what it
    /// was answered.
    Flooded,
    /// The client asked the server to stop (`shutdown`).
    Shutdown,
    /// It never said `initialize` within [`ServeOptions::initialize_within`].
    Silent,
    /// The server stopped it.
    Stopped,
}

/// What is counted against a connection's limits.
#[derive(Debug, Default)]
struct Budget {
    /// Requests read and not yet answered on the wire.
    requests: usize,
    /// The bytes of their lines, or of their results once larger.
    bytes: usize,
    /// Of those, the ones still running in the core.
    running: usize,
    /// Answers the reader gave at once (a refusal, an error for a line that
    /// is not a request, `initialize`, `shutdown`) that are not yet written.
    refusals: usize,
}

impl Budget {
    /// Takes a request of `bytes` if it fits under `most` requests and
    /// `most_bytes`.
    fn take_request(&mut self, bytes: usize, most: usize, most_bytes: usize) -> bool {
        let fits = self.requests < most && self.bytes.saturating_add(bytes) <= most_bytes;
        if fits {
            self.requests += 1;
            self.bytes += bytes;
            self.running += 1;
        }
        fits
    }

    /// A request of `bytes` has its result, of `length` bytes: what it
    /// counts from now on, which writing it gives back.
    fn result(&mut self, bytes: usize, length: usize) -> usize {
        let more = length.saturating_sub(bytes);
        self.bytes += more;
        bytes + more
    }

    /// Takes an answer given outside the lane, if fewer than `most` wait.
    fn take_refusal(&mut self, most: usize) -> bool {
        if self.refusals >= most.max(1) {
            return false;
        }
        self.refusals += 1;
        true
    }
}

/// A line ready to be written, and what it frees once it is.
#[derive(Debug)]
struct Ready {
    line: Vec<u8>,
    frees: Frees,
}

/// What writing a line gives back to the budget.
#[derive(Debug, Clone, Copy)]
enum Frees {
    /// A request's place, and its line's bytes.
    Request(usize),
    /// A refusal's place.
    Refusal,
}

/// What the parts of a connection share.
struct Shared {
    core: Core,
    options: ServeOptions,
    /// Responses, as lines, ready to be written in their turn.
    results: Mutex<VecDeque<Ready>>,
    /// Wakes the writer: a result is ready, the client's queue holds
    /// something, or the end is draining.
    wake: Notify,
    budget: Mutex<Budget>,
    /// Told when a request ends.
    freed: Notify,
    /// The requests put to this client and not yet answered, with each one's
    /// method, which its answer is read against.
    asked: Mutex<BTreeMap<RequestId, &'static str>>,
    client: Mutex<Option<Client>>,
    /// The end is under way: the writer stops once nothing is left.
    draining: AtomicBool,
}

impl Shared {
    fn queue(&self) -> Option<Arc<EventQueue>> {
        lock(&self.client)
            .as_ref()
            .map(|client| Arc::clone(&client.queue))
    }

    fn ready(&self, line: Vec<u8>, frees: Frees) {
        lock(&self.results).push_back(Ready { line, frees });
        self.wake.notify_one();
    }

    /// A request's result, as its line.
    fn answered(&self, id: RequestId, result: Result<OpResult, ErrorData>, frees: Frees) {
        let mut line = Vec::new();
        let written = wire::result(id.clone(), result, &mut line).or_else(|error| {
            tracing::error!(%error, "a result could not be written as a line");
            line.clear();
            wire::error(
                Some(id),
                wire::application(
                    ErrorKind::Internal,
                    "unwritable_result",
                    "The result could not be written as JSON.",
                ),
                &mut line,
            )
        });
        if written.is_ok() {
            // A result counts against the budget by its own size, once it
            // is larger than the request's line: a client that asks for
            // whole histories and reads none of them is refused more
            // before they pile up.
            let frees = match frees {
                Frees::Request(bytes) => {
                    Frees::Request(lock(&self.budget).result(bytes, line.len()))
                }
                other => other,
            };
            self.ready(line, frees);
        } else {
            // Nothing can be said; its place is given back all the same.
            self.free(frees);
        }
    }

    /// Answers at once with an error, counted as a refusal until written.
    /// False when the client has too many unread already: it is flooding.
    #[must_use]
    fn refuse(&self, id: Option<RequestId>, error: ErrorObject) -> bool {
        if !self.take_refusal() {
            return false;
        }
        let mut line = Vec::new();
        if wire::error(id, error, &mut line).is_ok() {
            self.ready(line, Frees::Refusal);
        } else {
            self.free(Frees::Refusal);
        }
        true
    }

    /// Counts an answer given outside the lane (a refusal, `initialize`,
    /// `shutdown`) until it is written. False when the client has as many
    /// unread as requests may run: it sends and does not read.
    #[must_use]
    fn take_refusal(&self) -> bool {
        lock(&self.budget).take_refusal(self.options.max_requests)
    }

    /// Whether the client is near a bound, so that the reader lets the
    /// writer catch up before it reads on: a client that reads is then
    /// not taken for one that floods.
    fn near_a_bound(&self) -> bool {
        let budget = lock(&self.budget);
        let most = self.options.max_requests.max(1);
        budget.refusals * 2 >= most || budget.requests * 2 >= most
    }

    /// The same, for an error of the protocol's own kind.
    #[must_use]
    fn refuse_with(&self, id: RequestId, kind: ErrorKind, code: &str, message: &str) -> bool {
        self.refuse(Some(id), wire::application(kind, code, message))
    }

    fn free(&self, frees: Frees) {
        let mut budget = lock(&self.budget);
        match frees {
            Frees::Request(bytes) => {
                budget.requests = budget.requests.saturating_sub(1);
                budget.bytes = budget.bytes.saturating_sub(bytes);
            }
            Frees::Refusal => budget.refusals = budget.refusals.saturating_sub(1),
        }
    }

    fn nothing_running(&self) -> bool {
        lock(&self.budget).running == 0
    }
}

/// One client's connection. Made with [`Connection::start`], served with
/// [`Connection::run`] until it ends, and finished with
/// [`Connection::finish`].
pub struct Connection {
    shared: Arc<Shared>,
    input: mpsc::Receiver<io::Result<Vec<u8>>>,
    decoder: FrameDecoder,
    dispatcher: Option<Dispatcher<(RequestId, usize)>>,
    writer: tokio::task::JoinHandle<Ended>,
    /// Whether the writer has ended: its handle is not polled again.
    writer_done: bool,
    /// The `shutdown` request that ended the connection, answered once the
    /// server has stopped.
    shutdown: Option<RequestId>,
    /// The role the client said in `initialize`.
    role: Option<ClientRole>,
    /// The server's own end, which a `shutdown` begins: for a socket's
    /// connection, apart from the connection's own end. `None`: the two
    /// are one (standard I/O).
    server_ending: Option<CancelToken>,
    /// When the connection began, on the core's clock.
    began: Duration,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").finish_non_exhaustive()
    }
}

/// What the reader waited for.
enum Step {
    Chunk(Option<io::Result<Vec<u8>>>),
    Writer(Ended),
}

impl Connection {
    /// Starts serving a client over `transport`. Must be called inside the
    /// core's runtime: the writer is a task of it.
    #[must_use]
    pub fn start(core: Core, transport: Transport, options: ServeOptions) -> Self {
        let decoder = FrameDecoder::with_max_line_len(options.max_line);
        let shared = Arc::new(Shared {
            core,
            options,
            results: Mutex::new(VecDeque::new()),
            wake: Notify::new(),
            budget: Mutex::new(Budget::default()),
            freed: Notify::new(),
            asked: Mutex::new(BTreeMap::new()),
            client: Mutex::new(None),
            draining: AtomicBool::new(false),
        });
        let writer = tokio::spawn(write(Arc::clone(&shared), transport.output));
        let began = shared.options.clock.monotonic();
        Self {
            shared,
            input: transport.input,
            decoder,
            dispatcher: None,
            writer,
            writer_done: false,
            shutdown: None,
            role: None,
            server_ending: None,
            began,
        }
    }

    /// The end a client's `shutdown` begins, when it is the server's and
    /// not only this connection's: one of a socket's connections.
    pub fn set_server_ending(&mut self, ending: CancelToken) {
        self.server_ending = Some(ending);
    }

    /// The client, once it has said `initialize`.
    #[must_use]
    pub fn client(&self) -> Option<Client> {
        lock(&self.shared.client).clone()
    }

    /// Reads and serves the client until its stream ends, the writer finds
    /// it gone or stalled, it floods, it asks for `shutdown`, or `stop` is
    /// cancelled. Then disconnects it from the core and waits for its
    /// lane's operation at most [`ServeOptions::lane_end`].
    ///
    /// `ending` is cancelled when the end begins: when serving stops, and
    /// before a `shutdown` ends the core, for a caller that bounds the
    /// whole end (the core's end waits for each session's close, which
    /// nothing bounds).
    pub async fn run(&mut self, stop: &CancelToken, ending: &CancelToken) -> Ended {
        let ended = self.serve(stop, ending).await;
        ending.cancel();
        // What it was asked and never answered is withdrawn, which ends an
        // operation waiting on the answer, before the lane is waited for.
        if let Some(client) = self.client() {
            self.shared.core.disconnect(client.id).await;
        }
        if let Some(lane) = self.dispatcher.as_mut().and_then(Dispatcher::close)
            && within(
                &self.shared.options.timer,
                self.shared.options.lane_end,
                lane,
            )
            .await
            .is_none()
        {
            tracing::warn!("the connection's last operation did not end in time");
        }
        tracing::debug!(?ended, "a connection ended");
        ended
    }

    /// The end of the server, for this connection: waits for its requests
    /// still running, at most [`ServeOptions::requests_end`]; answers the
    /// `shutdown` that ended it; and, unless the client is gone, stalled or
    /// flooding, writes what is queued, at most [`ServeOptions::drain`].
    pub async fn finish(mut self, ended: Ended) {
        let shared = Arc::clone(&self.shared);
        let running = until(&shared.freed, || shared.nothing_running());
        if within(&shared.options.timer, shared.options.requests_end, running)
            .await
            .is_none()
        {
            tracing::warn!("requests were still running at the server's end");
        }
        if let Some(id) = self.shutdown.take() {
            // Its place was taken when it was read.
            shared.answered(id, Ok(OpResult::Shutdown(Empty {})), Frees::Refusal);
        }
        shared.draining.store(true, Ordering::SeqCst);
        shared.wake.notify_one();
        if self.writer_done || matches!(ended, Ended::Gone | Ended::Stalled | Ended::Flooded) {
            self.writer.abort();
            return;
        }
        let writer = &mut self.writer;
        if within(&shared.options.timer, shared.options.drain, writer)
            .await
            .is_none()
        {
            self.writer.abort();
        }
    }

    async fn serve(&mut self, stop: &CancelToken, ending: &CancelToken) -> Ended {
        loop {
            let writer = &mut self.writer;
            let input = &mut self.input;
            let next = poll_fn(|context| {
                if let Poll::Ready(chunk) = input.poll_recv(context) {
                    return Poll::Ready(Step::Chunk(chunk));
                }
                if let Poll::Ready(ended) = std::pin::Pin::new(&mut *writer).poll(context) {
                    return Poll::Ready(Step::Writer(ended.unwrap_or(Ended::Gone)));
                }
                Poll::Pending
            });
            // A connection that has not said `initialize` has until its
            // bound to say it.
            let silent_after = self
                .shared
                .options
                .initialize_within
                .filter(|_| self.dispatcher.is_none())
                .map(|within| {
                    let gone = self
                        .shared
                        .options
                        .clock
                        .monotonic()
                        .saturating_sub(self.began);
                    within.saturating_sub(gone)
                });
            let next = async {
                match silent_after {
                    Some(left) => within(&self.shared.options.timer, left, next).await,
                    None => Some(next.await),
                }
            };
            let Some(step) = stop.race(next).await else {
                return Ended::Stopped;
            };
            let Some(step) = step else {
                return Ended::Silent;
            };
            match step {
                Step::Writer(ended) => {
                    self.writer_done = true;
                    return ended;
                }
                Step::Chunk(None) => {
                    if let Err(error) = self.decoder.finish() {
                        let _ = self.shared.refuse(None, error.to_error_object());
                    }
                    return Ended::Closed;
                }
                Step::Chunk(Some(Err(error))) => {
                    tracing::warn!(%error, "the client's stream failed");
                    return Ended::Failed;
                }
                Step::Chunk(Some(Ok(bytes))) => {
                    self.decoder.push(&bytes);
                    if let Some(ended) = self.lines(ending).await {
                        return ended;
                    }
                }
            }
        }
    }

    /// Serves every whole line the decoder holds. An end when one of them
    /// ended the connection.
    async fn lines(&mut self, ending: &CancelToken) -> Option<Ended> {
        let long_line = self.shared.options.long_line;
        loop {
            if self.shared.near_a_bound() {
                tokio::task::yield_now().await;
            }
            let (bytes, incoming) = match self.decoder.next_line() {
                None => return None,
                Some(Err(error)) => {
                    if !self.shared.refuse(None, error.to_error_object()) {
                        return Some(Ended::Flooded);
                    }
                    continue;
                }
                Some(Ok(line)) if line.len() > long_line => {
                    // Parsed on the blocking pool: a long line holds the
                    // core thread for as long as it takes to read.
                    let line = line.to_vec();
                    let bytes = line.len();
                    let read = tokio::task::spawn_blocking(move || wire::read(&line)).await;
                    match read {
                        Ok(incoming) => (bytes, incoming),
                        Err(_) => (
                            bytes,
                            Incoming::Bad {
                                id: None,
                                error: ErrorObject {
                                    code: cortex_protocol::jsonrpc::code::INTERNAL_ERROR,
                                    message: "the line could not be read".to_owned(),
                                    data: None,
                                },
                            },
                        ),
                    }
                }
                Some(Ok(line)) => (line.len(), wire::read(line)),
            };
            match self.line(bytes, incoming, ending).await {
                Served::On => {}
                Served::Ended(Ended::Shutdown) => {
                    self.after_shutdown();
                    return Some(Ended::Shutdown);
                }
                Served::Ended(ended) => return Some(ended),
            }
        }
    }

    /// The requests read with a `shutdown`, after it: each is answered
    /// `shutting_down`, so that none goes unanswered. Answers to requests
    /// the server put are still taken.
    fn after_shutdown(&mut self) {
        while let Some(line) = self.decoder.next_line() {
            let Ok(line) = line else { continue };
            match wire::read(line) {
                Incoming::Request(request) => {
                    let _ = self.shared.refuse_with(
                        request.id,
                        ErrorKind::Usage,
                        code::SHUTTING_DOWN,
                        "The server is shutting down, and takes nothing more.",
                    );
                }
                Incoming::Response(response) => self.answer(response),
                Incoming::Notification | Incoming::Bad { .. } => {}
            }
        }
    }

    /// Serves one line.
    async fn line(&mut self, bytes: usize, incoming: Incoming, ending: &CancelToken) -> Served {
        match incoming {
            Incoming::Bad { id, error } => {
                if self.shared.refuse(id, error) {
                    Served::On
                } else {
                    Served::Ended(Ended::Flooded)
                }
            }
            Incoming::Notification => Served::On,
            Incoming::Response(response) => {
                self.answer(response);
                Served::On
            }
            Incoming::Request(request) => self.request(bytes, request, ending).await,
        }
    }

    /// The client's answer to a request the server put to it. A JSON-RPC
    /// error in its place declines it: the request stays open for the
    /// others asked, and for its time limit.
    fn answer(&self, response: Response) {
        let Some(id) = response.id else {
            tracing::debug!("an answer with no id was dropped");
            return;
        };
        let Some(client) = self.client() else {
            return;
        };
        let Some(method) = lock(&self.shared.asked).get(&id).copied() else {
            tracing::debug!(%id, "an answer to a request this client was not put was dropped");
            return;
        };
        let value = match response.result {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!(%id, message = %error.message, "a client declined a request");
                return;
            }
        };
        let answer = match ServerResponse::from_wire(method, value) {
            Ok(answer) => answer,
            Err(error) => {
                tracing::debug!(%id, %error, "an answer that does not fit its request was dropped");
                return;
            }
        };
        match self.shared.core.respond(client.id, &id, answer) {
            Ok(()) => {
                lock(&self.shared.asked).remove(&id);
            }
            Err(error) => {
                // Settled meanwhile, or refused: the core says how the
                // request was settled in an event. One refused for its
                // content (an answer that contradicts itself) stays open.
                if error.code == "request_not_open" {
                    lock(&self.shared.asked).remove(&id);
                }
                tracing::debug!(%id, code = %error.code, "an answer was not taken");
            }
        }
    }

    /// A request.
    async fn request(&mut self, bytes: usize, request: Request, ending: &CancelToken) -> Served {
        let Request { id, method, params } = request;
        if self.dispatcher.is_none() {
            if method == "initialize" {
                return self.initialize(id, params).await;
            }
            return self.refused(self.shared.refuse_with(
                id,
                ErrorKind::Usage,
                code::NOT_INITIALIZED,
                "The first request on a connection must be `initialize`.",
            ));
        }
        let op = match Op::from_wire(&method, params) {
            Ok(op) => op,
            Err(error) => {
                return self.refused(self.shared.refuse(Some(id), error.to_error_object()));
            }
        };
        match op {
            Op::Initialize(_) => self.refused(self.shared.refuse_with(
                id,
                ErrorKind::Usage,
                code::ALREADY_INITIALIZED,
                "This connection has said `initialize` already.",
            )),
            Op::Shutdown(params) => self.shutdown(id, params, ending).await,
            op if !self.shared.options.allow_unrestricted && asks_for_unrestricted(&op) => self
                .refused(self.shared.refuse_with(
                    id,
                    ErrorKind::Permission,
                    "unrestricted_not_allowed",
                    "A session at `unrestricted` is the server's command line's to allow: start \
                     the server with --permission-mode unrestricted.",
                )),
            op => {
                let taken = lock(&self.shared.budget).take_request(
                    bytes,
                    self.shared.options.max_requests,
                    self.shared.options.max_request_bytes,
                );
                if !taken {
                    return self.refused(self.shared.refuse_with(
                        id,
                        ErrorKind::Limit,
                        "overloaded",
                        "Too many requests of this connection are running or waiting to be \
                         answered; send it again once some have been.",
                    ));
                }
                if let Some(dispatcher) = &self.dispatcher {
                    dispatcher.send((id, bytes), op);
                }
                Served::On
            }
        }
    }

    /// On, when a refusal was queued; the end, when the client is flooding.
    fn refused(&self, queued: bool) -> Served {
        if queued {
            Served::On
        } else {
            Served::Ended(Ended::Flooded)
        }
    }

    /// `shutdown`: the core stops, outside the lane. The end begins before
    /// the core's own end, which waits for each session's close.
    async fn shutdown(&mut self, id: RequestId, params: Empty, ending: &CancelToken) -> Served {
        let Some(client) = self.client() else {
            return Served::On;
        };
        // Counted as a refusal until its answer is written, as it is
        // answered outside the lane.
        if !self.shared.take_refusal() {
            return Served::Ended(Ended::Flooded);
        }
        // Only a role the core lets end it: a refused `shutdown` must not
        // start the bound on an end that is not coming.
        if matches!(self.role, Some(ClientRole::User | ClientRole::Supervisor)) {
            self.server_ending.as_ref().unwrap_or(ending).cancel();
        }
        match self
            .shared
            .core
            .handle(client.id, Op::Shutdown(params))
            .await
        {
            Ok(_) => {
                self.shutdown = Some(id);
                Served::Ended(Ended::Shutdown)
            }
            Err(error) => {
                self.shared.answered(id, Err(*error), Frees::Refusal);
                Served::On
            }
        }
    }

    /// `initialize`: the client is connected with the role and the
    /// capabilities it says, and the core answers.
    async fn initialize(&mut self, id: RequestId, params: Option<serde_json::Value>) -> Served {
        let params = match Op::from_wire("initialize", params) {
            Ok(Op::Initialize(params)) => params,
            Ok(_) => return Served::On,
            Err(error) => {
                return self.refused(self.shared.refuse(Some(id), error.to_error_object()));
            }
        };
        let InitializeParams {
            role, capabilities, ..
        } = &params;
        let role = role.clone();
        let info = ClientInfo {
            role: role.clone(),
            capabilities: capabilities.clone(),
            queue: self.shared.options.queue,
        };
        // Counted as a refusal until its answer is written, as it is
        // answered outside the lane: a client that says `initialize` over
        // and over and reads nothing is flooding.
        if !self.shared.take_refusal() {
            return Served::Ended(Ended::Flooded);
        }
        let core = self.shared.core.clone();
        let client = core.connect(info);
        match core.handle(client.id, Op::Initialize(params)).await {
            Ok(result) => {
                // Weak: the queue is the client's, which the connection
                // holds, and a strong hold back would keep both for good.
                let wake = Arc::downgrade(&self.shared);
                client.queue.set_wake(Arc::new(move || {
                    if let Some(shared) = wake.upgrade() {
                        shared.wake.notify_one();
                    }
                }));
                *lock(&self.shared.client) = Some(client.clone());
                self.role = Some(role);
                let client_id = client.id;
                let run: Run = Arc::new(move |op| {
                    let core = core.clone();
                    Box::pin(async move { core.handle(client_id, op).await })
                });
                let shared = Arc::clone(&self.shared);
                let done = Arc::new(move |(id, bytes): (RequestId, usize), result| {
                    shared.answered(id, result, Frees::Request(bytes));
                    let mut budget = lock(&shared.budget);
                    budget.running = budget.running.saturating_sub(1);
                    drop(budget);
                    shared.freed.notify_waiters();
                });
                self.dispatcher = Some(Dispatcher::start(run, done));
                self.shared.answered(id, Ok(result), Frees::Refusal);
            }
            Err(error) => {
                core.disconnect(client.id).await;
                self.shared.answered(id, Err(*error), Frees::Refusal);
            }
        }
        Served::On
    }
}

/// Whether an operation asks for a session at `unrestricted`: as its level
/// (`session/new`, `session/set`) or as a configuration override of
/// `permissions.mode`, written dotted or nested, in a profile too.
fn asks_for_unrestricted(op: &Op) -> bool {
    use cortex_protocol::lease::Level;
    let level = |settings: &cortex_protocol::session::SessionSettings| {
        matches!(settings.level, Some(Level::Unrestricted))
    };
    match op {
        Op::SessionNew(params) => {
            level(&params.settings)
                || params
                    .config
                    .iter()
                    .any(|(key, value)| sets_unrestricted(key, value))
        }
        Op::SessionSet(params) => level(&params.settings),
        _ => false,
    }
}

/// Whether the override at `key` sets a `permissions.mode` of
/// `unrestricted` anywhere inside it.
fn sets_unrestricted(key: &str, value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(inner) => inner
            .iter()
            .any(|(more, value)| sets_unrestricted(&format!("{key}.{more}"), value)),
        serde_json::Value::String(mode) => {
            mode == "unrestricted"
                && (key == "permissions.mode" || key.ends_with(".permissions.mode"))
        }
        _ => false,
    }
}

/// What serving a line came to.
enum Served {
    On,
    Ended(Ended),
}

/// Waits until `done` holds, looking again each time `notify` is told.
async fn until(notify: &Notify, done: impl Fn() -> bool) {
    loop {
        let mut notified = pin!(notify.notified());
        // Registered before the look, so that a change in between is not
        // missed.
        notified.as_mut().enable();
        if done() {
            return;
        }
        notified.await;
    }
}

/// `future`, or `None` once `limit` has passed on `timer`, for the server.
pub(crate) async fn within_public<F: Future>(
    timer: &SharedTimer,
    limit: Duration,
    future: F,
) -> Option<F::Output> {
    within(timer, limit, future).await
}

/// `future`, or `None` once `limit` has passed on `timer`.
async fn within<F: Future>(timer: &SharedTimer, limit: Duration, future: F) -> Option<F::Output> {
    let mut future = pin!(future);
    let mut sleep = timer.sleep(limit);
    poll_fn(|context| {
        if let Poll::Ready(output) = future.as_mut().poll(context) {
            return Poll::Ready(Some(output));
        }
        if sleep.as_mut().poll(context).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

/// The writer: the next thing for the client, in order, one line at a time.
/// What the writer sends next: before `initialize` only results; after it
/// the client's queue and the results in [`Order`], a result after the
/// events queued when it was ready.
fn next_to_write(
    order: &mut Order,
    queue: Option<&EventQueue>,
    results: &Mutex<VecDeque<Ready>>,
) -> Option<Next<Ready>> {
    match queue {
        Some(queue) => order.next(queue, results),
        None => lock(results).pop_front().map(Next::Result),
    }
}

async fn write(shared: Arc<Shared>, output: mpsc::Sender<Vec<u8>>) -> Ended {
    let mut order = Order::default();
    loop {
        let queue = shared.queue();
        let next = next_to_write(&mut order, queue.as_deref(), &shared.results);
        let mut line = Vec::new();
        let mut frees = None;
        match next {
            Some(Next::Result(ready)) => {
                line = ready.line;
                frees = Some(ready.frees);
            }
            Some(Next::Core(Outgoing::Event(event))) => {
                if let Event::RequestResolved(resolved) = &event.event {
                    lock(&shared.asked).remove(&resolved.request_id);
                }
                if let Err(error) = wire::event(&event, &mut line) {
                    tracing::error!(%error, "an event could not be written as a line");
                    continue;
                }
            }
            Some(Next::Core(Outgoing::Request { id, request })) => {
                lock(&shared.asked).insert(id.clone(), request.method());
                if let Err(error) = wire::request(id, &request, &mut line) {
                    tracing::error!(%error, "a request could not be written as a line");
                    continue;
                }
            }
            None => {
                let woken = shared.wake.notified();
                if shared.draining.load(Ordering::SeqCst) {
                    return Ended::Closed;
                }
                woken.await;
                continue;
            }
        }
        let sent = send(&shared, &output, line, queue.as_deref()).await;
        // A result written gives its place back, whether or not the client
        // took it: one that is gone ends the connection anyway.
        if let Some(frees) = frees {
            shared.free(frees);
        }
        if let Err(ended) = sent {
            return ended;
        }
    }
}

/// Sends one line, closing the connection when the client takes nothing
/// for too long while whole events wait past its queue's capacity, or
/// when far more than that wait.
async fn send(
    shared: &Shared,
    output: &mpsc::Sender<Vec<u8>>,
    line: Vec<u8>,
    queue: Option<&EventQueue>,
) -> Result<(), Ended> {
    let options = &shared.options;
    let look = LOOK
        .min(options.stalled_client)
        .max(Duration::from_millis(1));
    let mut sending = pin!(output.send(line));
    let began = options.clock.monotonic();
    loop {
        match within(&options.timer, look, sending.as_mut()).await {
            Some(Ok(())) => return Ok(()),
            Some(Err(_)) => return Err(Ended::Gone),
            None => {
                let waited = options.clock.monotonic().saturating_sub(began);
                let Some(queue) = queue else {
                    continue;
                };
                let waiting = queue.len();
                let past = waiting > options.queue;
                if waiting > options.queue.saturating_mul(CEILING)
                    || (past && waited >= options.stalled_client)
                {
                    tracing::warn!(
                        waiting,
                        waited_ms = u64::try_from(waited.as_millis()).unwrap_or(u64::MAX),
                        "a client took nothing while whole events waited for it: its connection \
                         is closed"
                    );
                    return Err(Ended::Stalled);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request fits under both bounds, or is refused; a result larger
    /// than its request counts by its own size from then on, so the next
    /// request is refused while it waits; writing it gives back all of it.
    #[test]
    fn the_budget_counts_requests_and_their_results_by_size() {
        let mut budget = Budget::default();
        assert!(budget.take_request(100, 4, 1000));
        assert!(budget.take_request(100, 4, 1000));
        assert!(!budget.take_request(900, 4, 1000), "past the bytes");
        let counted = budget.result(100, 700);
        assert_eq!(counted, 700);
        assert_eq!(budget.bytes, 800);
        assert!(
            !budget.take_request(300, 4, 1000),
            "the result's size counts"
        );
        assert_eq!(
            budget.result(100, 50),
            100,
            "a small result counts its request"
        );
        assert!(budget.take_request(100, 4, 1000));
        assert!(budget.take_request(100, 4, 1000));
        assert!(!budget.take_request(0, 4, 1000), "past the count");
    }

    fn event(seq: u64) -> cortex_protocol::SessionEvent {
        cortex_protocol::SessionEvent {
            session_id: cortex_protocol::SessionId::new("s_1"),
            seq,
            turn_id: None,
            event: Event::TurnStarted(cortex_protocol::event::TurnStarted {}),
        }
    }

    /// The writer sends a result after the events queued when it was ready,
    /// and only those: a read's result never overtakes an event at or below
    /// its `seq`, and events that keep coming do not hold it back.
    #[test]
    fn the_writer_sends_a_result_after_the_events_queued_before_it() {
        let queue = EventQueue::new(64, true);
        let results = Mutex::new(VecDeque::new());
        let mut order = Order::default();
        for seq in 1..=3 {
            queue.push(event(seq));
        }
        lock(&results).push_back(Ready {
            line: b"read".to_vec(),
            frees: Frees::Refusal,
        });
        let mut sent = Vec::new();
        while let Some(next) = next_to_write(&mut order, Some(&queue), &results) {
            sent.push(match next {
                Next::Core(Outgoing::Event(event)) => format!("event {}", event.seq),
                Next::Core(Outgoing::Request { .. }) => "request".to_owned(),
                Next::Result(ready) => String::from_utf8(ready.line).unwrap_or_default(),
            });
            if sent.len() == 2 {
                queue.push(event(4));
            }
        }
        assert_eq!(sent, ["event 1", "event 2", "event 3", "read", "event 4"]);
        // Before `initialize` there is no queue: results alone.
        lock(&results).push_back(Ready {
            line: b"early".to_vec(),
            frees: Frees::Refusal,
        });
        assert!(matches!(
            next_to_write(&mut order, None, &results),
            Some(Next::Result(ready)) if ready.line == b"early"
        ));
    }

    /// Answers given outside the lane are counted up to the bound, at least
    /// one.
    #[test]
    fn refusals_are_counted_up_to_their_bound() {
        let mut budget = Budget::default();
        assert!(budget.take_refusal(2));
        assert!(budget.take_refusal(2));
        assert!(!budget.take_refusal(2));
        let mut none = Budget::default();
        assert!(none.take_refusal(0));
        assert!(!none.take_refusal(0));
    }
}
