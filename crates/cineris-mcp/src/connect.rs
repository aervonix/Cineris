//! A local server's client: the probe, the fallback to the legacy
//! handshake, and the [`Client`] over standard I/O
//! (`docs/steps/30/answered.md`, points 0, 1 and 3.7; the research notes'
//! section 2.2).
//!
//! The probe asks `server/discover` with the revision the client prefers.
//! A discovery result, or one of the modern errors, is a modern server, and
//! is never spoken to with `initialize`. Any other error, or silence for
//! the probe's time, is a legacy server, and the handshake follows on the
//! same process.
//!
//! **A process that was sent `initialize` is never spoken to as modern:** a
//! legacy server scopes it to the legacy era, and a modern one may exit on
//! refusing it. Silence may be a modern server still starting (a cold `npx`
//! or `docker run`): when its answer to the probe comes while `initialize`
//! waits, or it refuses `initialize`, that process is let go, and the
//! server is started again and asked what it speaks with no time of its own
//! (the start's limit holds), as a server known to be modern is. Some
//! legacy servers exit on any request before `initialize`: one that exits
//! during the probe, or while it is answered after it, is started again and
//! sent `initialize` alone.

use std::future::poll_fn;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use cineris_base::{CancelToken, SharedTimer};
use cineris_tools::{BoxFuture, Json};
use tokio::task::AbortHandle;

use crate::client::{CallOutput, Client, ClientError, Era, ServerInfo, ToolDefinition};
use crate::definition::ServerDefinition;
use crate::jsonrpc::{self, Answer};
use crate::process::{ServerPipes, SpawnError};
use crate::session::{
    Agreed, ClientOptions, Declined, Limit, Probe, Router, Rpc, Session, bounded, discover,
    discover_params, initialize, probed, shown,
};
use crate::stdio::StdioTransport;
use crate::transport::{Transport, lock};
use crate::wire;

/// Starts the server again, as it was started: for a legacy server that
/// exits on the probe.
pub type Respawn = Arc<dyn Fn() -> Result<ServerPipes, SpawnError> + Send + Sync>;

/// A local server, connected over its standard input and output.
pub struct StdioClient {
    info: ServerInfo,
    session: Arc<Session>,
    transport: Arc<StdioTransport>,
    listen: Option<AbortHandle>,
    call_timeout: Duration,
    grace: Duration,
    shut: AtomicBool,
}

impl std::fmt::Debug for StdioClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdioClient")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

/// One process and the connection to it.
struct Link {
    router: Arc<Router>,
    transport: Arc<StdioTransport>,
    rpc: Rpc,
}

/// What the start needs between its attempts.
struct Start<'a> {
    timer: &'a SharedTimer,
    options: &'a ClientOptions,
    respawn: &'a Respawn,
    /// The connection being tried now. A start that ends without handing
    /// it to a client (it failed, ran out of time, or its caller dropped it)
    /// ends its process.
    current: Mutex<Option<Arc<StdioTransport>>>,
    /// The words of a refusal of `initialize` that sent the start on to ask
    /// a new process patiently: what a start that then runs out of time
    /// reports.
    refusal: Mutex<Option<String>>,
}

impl Drop for Start<'_> {
    fn drop(&mut self) {
        if let Some(transport) = lock(&self.current).take() {
            retire(&transport, self.options.shutdown_grace);
        }
    }
}

impl Start<'_> {
    fn link(&self, pipes: ServerPipes) -> Link {
        let router = Arc::new(Router::new());
        let transport = StdioTransport::start(
            pipes,
            Arc::clone(&router) as _,
            Arc::clone(self.timer),
            self.options.shutdown_grace,
        );
        *lock(&self.current) = Some(Arc::clone(&transport));
        let rpc = Rpc::new(
            Arc::clone(&transport) as Arc<dyn Transport>,
            self.options.max_in_flight,
        );
        Link {
            router,
            transport,
            rpc,
        }
    }

    /// The server started again, once the process let go has ended: a
    /// server that cannot run twice (a fixed name, a lock, a port) would
    /// refuse the next. The wait is within the start's limit, and the old
    /// process stays the start's own until it has ended, so that a start
    /// dropped meanwhile still ends it. `why` is why, for a server that
    /// cannot be started again.
    async fn again(&self, old: Link, why: &str) -> Result<Link, ClientError> {
        old.transport.abandon();
        old.transport
            .process()
            .shutdown(self.options.shutdown_grace)
            .await;
        let pipes = (self.respawn)().map_err(|error| {
            ClientError::NotStarted(format!("{why}, and could not be started again: {error}"))
        })?;
        Ok(self.link(pipes))
    }

    /// Probes, falls back, and agrees a revision. A server known to be
    /// legacy is sent `initialize` at once, and started again and probed
    /// when it refuses it; one known to be modern is asked with no time of
    /// its own. **A server known or found to be modern is never sent
    /// `initialize`:** its process asked patiently either answers as a
    /// modern server, or the start ends with what it said.
    async fn agree(&self, first: Link, known: Option<Era>) -> Result<(Link, Agreed), ClientError> {
        let preferred = wire::MODERN_VERSIONS[0];
        let mut link = first;
        let mut next = match known {
            Some(Era::Legacy) => Next::Initialize,
            Some(Era::Modern) => Next::Probe { patient: true },
            None => Next::Probe { patient: false },
        };
        // How many processes asked patiently exited before they answered.
        let mut patient_exits = 0;
        loop {
            match next {
                Next::Initialize => match initialize(&link.rpc).await {
                    Ok(agreed) => return Ok((link, agreed)),
                    // Refused: the server changed. Asked what it speaks, on
                    // a process that was not sent `initialize`.
                    Err(Declined::Answered(_)) => {
                        link = self.again(link, "it refused `initialize`").await?;
                        next = Next::Probe { patient: false };
                    }
                    // It exited: started again, and sent `initialize` alone.
                    Err(Declined::Stopped(_)) => {
                        return self
                            .initialize_again(link, "it exited during `initialize`")
                            .await;
                    }
                    Err(Declined::Failed(error)) => return Err(error),
                },
                Next::Probe { patient } => {
                    // Sent by hand, not by `Rpc::ask`, so that an answer
                    // after the probe's time is still heard while
                    // `initialize` waits. Never cancelled: a legacy server
                    // would not know the request it named.
                    let transport = Arc::clone(link.rpc.transport());
                    let id = link.rpc.next_id();
                    let message =
                        jsonrpc::request(id, "server/discover", discover_params(preferred));
                    let mut probe = transport.request(id, message);
                    let answered = if patient {
                        Ok((&mut probe).await)
                    } else {
                        let mut limit = Limit::new(
                            self.timer,
                            "answer server/discover",
                            self.options.probe_timeout,
                        );
                        bounded(&mut probe, Some(&mut limit), None).await
                    };
                    let Ok(answered) = answered else {
                        // Silent: legacy, unless it shows itself modern while
                        // `initialize` waits.
                        match self.handshake_hearing(&link, probe).await {
                            Heard::Legacy(agreed) => return Ok((link, agreed)),
                            Heard::Modern { refusal } => {
                                if let Some(words) = refusal {
                                    *lock(&self.refusal) = Some(words);
                                }
                                link = self
                                    .again(
                                        link,
                                        "it showed itself modern once it was sent `initialize`",
                                    )
                                    .await?;
                                next = Next::Probe { patient: true };
                                continue;
                            }
                            Heard::Exited => {
                                return self
                                    .initialize_again(link, "it exited during `initialize`")
                                    .await;
                            }
                            Heard::Failed(error) => return Err(error),
                        }
                    };
                    drop(probe);
                    let probe = probed(answered, preferred);
                    if patient {
                        // Known or found to be modern: never `initialize`.
                        match probe {
                            Probe::Exited(_) if patient_exits == 0 => {
                                patient_exits += 1;
                                link = self
                                    .again(link, "it exited before it answered `server/discover`")
                                    .await?;
                                continue;
                            }
                            other => return self.patiently(link, other).await,
                        }
                    }
                    return match probe {
                        Probe::Modern(agreed) => Ok((link, agreed)),
                        // An error that is not a modern one: legacy.
                        Probe::Legacy(_) => self.handshake(link).await,
                        // A `-32022` naming only legacy revisions: a server
                        // of the legacy era that says so.
                        Probe::Unsupported(supported) if only_legacy(&supported) => {
                            self.handshake(link).await
                        }
                        Probe::Unsupported(supported) => self.retry(link, &supported).await,
                        Probe::Refused(why) => Err(ClientError::NotStarted(why)),
                        Probe::Exited(_) => {
                            self.initialize_again(link, "it exited when asked what it speaks")
                                .await
                        }
                        Probe::Failed(error) => Err(error),
                    };
                }
            }
        }
    }

    /// What a server known or found to be modern answered, asked
    /// patiently: a modern answer, or the end of the start with what it
    /// said. Never `initialize`.
    async fn patiently(&self, link: Link, probe: Probe) -> Result<(Link, Agreed), ClientError> {
        let known = "though it was known or found to be modern, so it is not sent `initialize`";
        match probe {
            Probe::Modern(agreed) => Ok((link, agreed)),
            Probe::Unsupported(supported) if only_legacy(&supported) => {
                let named: Vec<&str> = supported.iter().map(String::as_str).collect();
                Err(ClientError::NotStarted(format!(
                    "it named only legacy revisions ({}) {known}",
                    shown(&named)
                )))
            }
            Probe::Unsupported(supported) => self.retry(link, &supported).await,
            Probe::Refused(why) => Err(ClientError::NotStarted(why)),
            Probe::Legacy(what) => Err(ClientError::NotStarted(format!(
                "it answered `server/discover` with {what}, as a legacy server does, {known}"
            ))),
            Probe::Exited(stopped) => Err(ClientError::NotStarted(format!(
                "it exited twice before it answered `server/discover` ({stopped})"
            ))),
            Probe::Failed(error) => Err(error),
        }
    }

    /// The server started again, and sent `initialize` alone.
    async fn initialize_again(&self, old: Link, why: &str) -> Result<(Link, Agreed), ClientError> {
        let link = self.again(old, why).await?;
        let agreed = initialize(&link.rpc).await.map_err(declined)?;
        Ok((link, agreed))
    }

    /// The legacy handshake on a process that answered the probe as a
    /// legacy server does; one that exits during it is started again once.
    async fn handshake(&self, link: Link) -> Result<(Link, Agreed), ClientError> {
        match initialize(&link.rpc).await {
            Ok(agreed) => Ok((link, agreed)),
            Err(Declined::Stopped(_)) => {
                self.initialize_again(link, "it exited during `initialize`")
                    .await
            }
            Err(declined_) => Err(declined(declined_)),
        }
    }

    /// The legacy handshake on a process silent on the probe, which is
    /// still heard: a modern answer to it shows a modern server slower than
    /// the probe, and so does a refusal of `initialize`, unless the probe
    /// was answered as a legacy server answers it.
    async fn handshake_hearing(
        &self,
        link: &Link,
        mut probe: BoxFuture<'_, Result<Answer, ClientError>>,
    ) -> Heard {
        let preferred = wire::MODERN_VERSIONS[0];
        let mut handshake = pin!(initialize(&link.rpc));
        let mut probe_done = false;
        let mut heard_legacy = false;
        let outcome = poll_fn(|context| {
            if !probe_done && let Poll::Ready(answered) = probe.as_mut().poll(context) {
                probe_done = true;
                match probed(answered, preferred) {
                    // A `-32022` naming only legacy revisions, or a legacy
                    // server's late error: the handshake goes on.
                    Probe::Unsupported(supported) if only_legacy(&supported) => {
                        heard_legacy = true;
                    }
                    Probe::Legacy(_) => heard_legacy = true,
                    Probe::Modern(_) | Probe::Unsupported(_) | Probe::Refused(_) => {
                        return Poll::Ready(None);
                    }
                    // An end the handshake meets too.
                    Probe::Exited(_) | Probe::Failed(_) => {}
                }
            }
            handshake.as_mut().poll(context).map(Some)
        })
        .await;
        match outcome {
            None => Heard::Modern { refusal: None },
            Some(Ok(agreed)) => Heard::Legacy(agreed),
            // Refused by a server that answered the probe as a legacy
            // server does: its last word.
            Some(Err(Declined::Answered(refusal))) if heard_legacy => {
                Heard::Failed(declined(Declined::Answered(refusal)))
            }
            Some(Err(Declined::Answered(refusal))) => Heard::Modern {
                refusal: Some(refusal_words(refusal)),
            },
            Some(Err(Declined::Stopped(_))) => Heard::Exited,
            Some(Err(Declined::Failed(error))) => Heard::Failed(error),
        }
    }

    /// A modern server that does not speak the revision asked for: asked
    /// once more with one it does, and never with `initialize`.
    async fn retry(&self, link: Link, supported: &[String]) -> Result<(Link, Agreed), ClientError> {
        let Some(version) = wire::MODERN_VERSIONS
            .iter()
            .find(|ours| supported.iter().any(|theirs| theirs == *ours))
        else {
            return Err(ClientError::NotStarted(format!(
                "it speaks the modern era, and none of the {} revisions it named is one this \
                 client speaks ({})",
                supported.len(),
                wire::MODERN_VERSIONS.join(", ")
            )));
        };
        let mut limit = Limit::new(
            self.timer,
            "answer server/discover",
            self.options.probe_timeout,
        );
        match discover(&link.rpc, version, &mut limit).await {
            Probe::Modern(agreed) => Ok((link, agreed)),
            Probe::Failed(error) | Probe::Exited(error) => Err(error),
            Probe::Legacy(what) => Err(ClientError::NotStarted(format!(
                "asked again with revision {version}, which it said it speaks, it answered {what}"
            ))),
            Probe::Unsupported(_) | Probe::Refused(_) => Err(ClientError::NotStarted(format!(
                "it refused revision {version}, which it said it speaks"
            ))),
        }
    }
}

/// What the start does next with the process it has.
#[derive(Debug, Clone, Copy)]
enum Next {
    /// Send `initialize` (a server known to be legacy).
    Initialize,
    /// Ask what it speaks: for the probe's time, or (`patient`) for as long
    /// as the start's limit allows.
    Probe { patient: bool },
}

/// How a process silent on the probe answered the handshake.
enum Heard {
    /// It agreed a legacy revision.
    Legacy(Agreed),
    /// Its late answer to the probe, or its refusal of `initialize` (whose
    /// words it keeps), shows a modern server: the process is let go.
    Modern { refusal: Option<String> },
    /// It exited.
    Exited,
    /// Something else ended the start.
    Failed(ClientError),
}

/// Whether a `-32022`'s list of revisions names only legacy ones (every one
/// dated before the first modern revision) and one of them is a revision
/// the client speaks: a server of the legacy era that says so, as `rmcp`'s
/// servers narrowed to the legacy revisions do. A list naming any modern
/// revision is a modern server's.
fn only_legacy(supported: &[String]) -> bool {
    let first_modern = wire::MODERN_VERSIONS
        .iter()
        .copied()
        .min()
        .unwrap_or("2026-07-28");
    supported
        .iter()
        .all(|version| version.as_str() < first_modern)
        && supported
            .iter()
            .any(|version| wire::LEGACY_VERSIONS.contains(&version.as_str()))
}

/// The words of a refusal of `initialize`, for a message.
fn refusal_words(refusal: ClientError) -> String {
    match declined(Declined::Answered(refusal)) {
        ClientError::NotStarted(words) => words,
        other => other.to_string(),
    }
}

/// What the client reports for a refused handshake.
fn declined(declined: Declined) -> ClientError {
    match declined {
        Declined::Answered(ClientError::Server { code, message }) => ClientError::NotStarted(
            format!("it refused `initialize` with error {code}: {message}"),
        ),
        Declined::Answered(error) | Declined::Stopped(error) | Declined::Failed(error) => error,
    }
}

/// Lets a connection go and ends its process in the background, inside a
/// Tokio runtime; outside one, its input is closed and nothing more.
fn retire(transport: &Arc<StdioTransport>, grace: Duration) {
    transport.abandon();
    if tokio::runtime::Handle::try_current().is_ok() {
        let process = Arc::clone(transport.process());
        tokio::spawn(async move { process.shutdown(grace).await });
    }
}

impl StdioClient {
    /// Connects to the server `pipes` came from, finding its era by the
    /// probe. `respawn` starts it again when it exits on the probe. Every
    /// wait is on `timer`; the whole start is held to the definition's
    /// start limit, and a start that fails ends the server's process. Must
    /// be called inside a Tokio runtime.
    ///
    /// # Errors
    ///
    /// [`ClientError::TimedOut`] past the start limit;
    /// [`ClientError::NotStarted`] for a server that speaks no revision the
    /// client does, or that could not be started again;
    /// [`ClientError::Stopped`] for one that exited.
    pub async fn connect(
        definition: &ServerDefinition,
        pipes: ServerPipes,
        respawn: Respawn,
        timer: SharedTimer,
        options: ClientOptions,
    ) -> Result<Self, ClientError> {
        Self::start(definition, pipes, respawn, timer, options, None).await
    }

    /// Connects to a server whose era is known from an earlier start, as
    /// [`ServerInfo::era`] gave it: a legacy server is spoken to with
    /// `initialize` at once, and started again and probed if it refuses it;
    /// a modern server is asked `server/discover` (which tells its
    /// capabilities) with no time of its own, and is never sent
    /// `initialize`.
    ///
    /// # Errors
    ///
    /// As [`StdioClient::connect`].
    pub async fn connect_known(
        era: Era,
        definition: &ServerDefinition,
        pipes: ServerPipes,
        respawn: Respawn,
        timer: SharedTimer,
        options: ClientOptions,
    ) -> Result<Self, ClientError> {
        Self::start(definition, pipes, respawn, timer, options, Some(era)).await
    }

    async fn start(
        definition: &ServerDefinition,
        pipes: ServerPipes,
        respawn: Respawn,
        timer: SharedTimer,
        options: ClientOptions,
        known: Option<Era>,
    ) -> Result<Self, ClientError> {
        let start = Start {
            timer: &timer,
            options: &options,
            respawn: &respawn,
            current: Mutex::new(None),
            refusal: Mutex::new(None),
        };
        let first = start.link(pipes);
        let mut limit = Limit::new(&timer, "start", definition.start_timeout);
        // A start that fails ends its process in the background, by
        // `Start`'s drop, and returns at once: within its limit.
        let (link, agreed) = match bounded(start.agree(first, known), Some(&mut limit), None).await
        {
            Ok(Ok(agreed)) => agreed,
            Ok(Err(error)) => return Err(error),
            Err(timed_out) => {
                let refusal = lock(&start.refusal).take();
                return Err(match refusal {
                    Some(words) => ClientError::NotStarted(format!(
                        "{words}, and started again it did not answer `server/discover` within \
                         the start's {} s",
                        definition.start_timeout.as_secs()
                    )),
                    None => timed_out,
                });
            }
        };
        // Handed to the client: no longer the start's to end.
        lock(&start.current).take();
        drop(start);
        let info = ServerInfo {
            era: agreed.era,
            version: agreed.version.clone(),
            name: agreed.name.clone(),
            list_changed: agreed.list_changed,
        };
        let grace = options.shutdown_grace;
        let session = Session::new(
            link.rpc,
            link.router,
            agreed.era,
            agreed.version,
            timer,
            options,
        );
        let listen = (agreed.era == Era::Modern && agreed.list_changed)
            .then(|| tokio::spawn(Arc::clone(&session).listen()).abort_handle());
        Ok(Self {
            info,
            session,
            transport: link.transport,
            listen,
            call_timeout: definition.call_timeout,
            grace,
            shut: AtomicBool::new(false),
        })
    }

    /// How many requests a modern server sent, which it may not: each was
    /// ignored.
    #[must_use]
    pub fn faults(&self) -> u64 {
        self.session.router().faults()
    }

    /// Whether the change stream is open and acknowledged.
    #[must_use]
    pub fn listening(&self) -> bool {
        self.session.router().listen_acked()
    }
}

impl Client for StdioClient {
    fn info(&self) -> &ServerInfo {
        &self.info
    }

    fn list_tools<'a>(
        &'a self,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<Vec<ToolDefinition>, ClientError>> {
        Box::pin(self.session.list_tools(cancel))
    }

    fn call<'a>(
        &'a self,
        tool: &'a str,
        arguments: Json,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<CallOutput, ClientError>> {
        Box::pin(async move {
            self.session
                .call(tool, &arguments, cancel, self.call_timeout)
                .await
        })
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.session.router().changes()
    }

    fn closed(&self) -> BoxFuture<'_, ClientError> {
        self.transport.closed()
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if self.shut.swap(true, Ordering::AcqRel) {
                return;
            }
            self.session.stop();
            if let Some(listen) = &self.listen {
                listen.abort();
            }
            self.transport.shutdown(self.grace).await;
        })
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        if self.shut.swap(true, Ordering::AcqRel) {
            return;
        }
        self.session.stop();
        if let Some(listen) = &self.listen {
            listen.abort();
        }
        retire(&self.transport, self.grace);
    }
}
