//! A local server's client: the probe, the fallback to the legacy
//! handshake, and the [`Client`] over standard I/O
//! (`docs/steps/30/answered.md`, points 0, 1 and 3.7; the research notes'
//! section 2.2).
//!
//! The probe asks `server/discover` with the revision the client prefers.
//! A discovery result, or one of the modern errors, is a modern server, and
//! is never spoken to with `initialize`. Any other error, or silence for
//! the probe's time, is a legacy server, and the handshake follows on the
//! same process. Some legacy servers exit on any request before
//! `initialize`: one that exits during the probe, or while it is answered
//! after it, is started again by the caller's closure and spoken to with
//! `initialize` alone. The whole start is held to the definition's start
//! limit.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cineris_base::{CancelToken, SharedTimer};
use cineris_tools::{BoxFuture, Json};
use tokio::task::AbortHandle;

use crate::client::{CallOutput, Client, ClientError, Era, ServerInfo, ToolDefinition};
use crate::definition::ServerDefinition;
use crate::process::{ServerPipes, SpawnError};
use crate::session::{
    Agreed, ClientOptions, Declined, Limit, Probe, Router, Rpc, Session, bounded, discover,
    initialize,
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
    /// The connection being tried now, so that one left by a start that
    /// ran out of time is shut down.
    current: Mutex<Option<Arc<StdioTransport>>>,
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

    /// The server started again: the old process is let go and ended in
    /// the background.
    fn again(&self, old: Link) -> Result<Link, ClientError> {
        retire(&old.transport, self.options.shutdown_grace);
        let pipes = (self.respawn)().map_err(|error| {
            ClientError::NotStarted(format!(
                "it exited when asked what it speaks, and could not be started again: {error}"
            ))
        })?;
        Ok(self.link(pipes))
    }

    /// Probes, falls back, and agrees a revision; `known` skips the probe
    /// for a server known to be legacy, probing after all if the handshake
    /// is refused.
    async fn agree(&self, first: Link, known: Option<Era>) -> Result<(Link, Agreed), ClientError> {
        let mut link = first;
        let mut started_again = false;
        if known == Some(Era::Legacy) {
            match initialize(&link.rpc).await {
                Ok(agreed) => return Ok((link, agreed)),
                // Refused: the server changed; what it speaks is asked.
                Err(Declined::Answered(_)) => {}
                Err(Declined::Stopped(_)) => {
                    link = self.again(link)?;
                    started_again = true;
                }
                Err(Declined::Failed(error)) => return Err(error),
            }
        }
        let mut limit = Limit::new(
            self.timer,
            "answer server/discover",
            self.options.probe_timeout,
        );
        let preferred = wire::MODERN_VERSIONS[0];
        match discover(&link.rpc, preferred, &mut limit).await {
            Probe::Modern(agreed) => return Ok((link, agreed)),
            Probe::Unsupported(supported) => return self.retry(link, &supported).await,
            Probe::Refused(why) => return Err(ClientError::NotStarted(why)),
            Probe::Failed(error) => return Err(error),
            Probe::Exited(_) if !started_again => {
                link = self.again(link)?;
                started_again = true;
            }
            Probe::Exited(stopped) => return Err(stopped),
            Probe::Legacy => {}
        }
        match initialize(&link.rpc).await {
            Ok(agreed) => Ok((link, agreed)),
            // A server that answered the probe and exits on it (rather than
            // at once) is started again once.
            Err(Declined::Stopped(_)) if !started_again => {
                let link = self.again(link)?;
                let agreed = initialize(&link.rpc).await.map_err(declined)?;
                Ok((link, agreed))
            }
            Err(declined_) => Err(declined(declined_)),
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
                "it speaks the modern era but none of its revisions ({}) is one this client \
                 speaks ({})",
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
            _ => Err(ClientError::NotStarted(format!(
                "it refused revision {version}, which it said it speaks"
            ))),
        }
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

/// Lets a connection go and ends its process in the background.
fn retire(transport: &Arc<StdioTransport>, grace: Duration) {
    transport.abandon();
    let process = Arc::clone(transport.process());
    tokio::spawn(async move { process.shutdown(grace).await });
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
    /// `initialize` at once, and probed after all if it refuses it; a
    /// modern server is asked `server/discover`, which tells its
    /// capabilities, as the probe does.
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
        };
        let first = start.link(pipes);
        let mut limit = Limit::new(&timer, "start", definition.start_timeout);
        let agreed = match bounded(start.agree(first, known), Some(&mut limit), None).await {
            Ok(Ok(agreed)) => agreed,
            Ok(Err(error)) | Err(error) => {
                let current = lock(&start.current).take();
                if let Some(transport) = current {
                    transport.shutdown(options.shutdown_grace).await;
                }
                return Err(error);
            }
        };
        let (link, agreed) = agreed;
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
        if tokio::runtime::Handle::try_current().is_ok() {
            retire(&self.transport, self.grace);
        } else {
            self.transport.abandon();
        }
    }
}
