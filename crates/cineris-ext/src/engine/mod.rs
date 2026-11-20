//! The script engine: QuickJS through `rquickjs` ([decision 0005]), behind
//! the `scripts` feature (`docs/steps/28/answered.md`, "The runtime").
//!
//! One runtime an extension, each on a thread of its own, made the first
//! time something of the extension's is used. A runtime runs one call at a
//! time, to its end, and a call's time starts when it starts running. A
//! script reaches nothing but the host functions: the context has only the
//! built-in objects a script needs (no `Performance`, no `Proxy`), the
//! binding adds nothing to the global object, the C library's `std` and
//! `os` modules are not compiled, and modules load only from the
//! extension's own snapshot.
//!
//! A limit (the call's time, the runtime's memory) tears the runtime down:
//! the call fails with the limit named, and the next use makes a new
//! runtime from a fresh snapshot. The session is never affected. A thread
//! that does not come back within a grace after its deadline is abandoned:
//! the runtime counts as torn down, and the thread's memory stays held to
//! its runtime's limit until it ends.
//!
//! [decision 0005]: ../../../docs/decisions/0005-extension-mechanism.md

mod loader;
mod thread;

use std::future::poll_fn;
use std::path::PathBuf;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::task::Poll;
use std::time::Duration;

use cineris_base::{CancelToken, Clock, ContentHash, Timer};
use serde_json::Value as Json;
use tokio::sync::{mpsc as async_mpsc, oneshot};

use crate::capability::Capabilities;
use crate::folder::{FolderError, Snapshot};
use crate::host::{Host, HostCall, HostError, Phase, serve};
use crate::manifest::{Manifest, ManifestError};
use crate::registration::Registration;

use thread::{FromThread, Invocation, Loaded, QUEUED, ToThread, WITHDRAWN};

/// The engine's stack limit for a script's calls.
pub const STACK_BYTES: usize = 256 * 1024;
/// A point of the runtime's thread just after it told a call's end, before
/// it takes the next call or ends: a test holds it to look at the runtime
/// as whoever heard of the end sees it.
pub const AFTER_A_CALLS_END_IS_TOLD: &str = "cineris-ext: after a call's end is told";

/// The longest the main module's first evaluation may take.
pub const EVALUATION_TIME: Duration = Duration::from_secs(10);
/// How long a call is given to stop after its deadline before its thread
/// is abandoned.
pub const GRACE: Duration = Duration::from_secs(2);

/// How many runtimes this process has made. Extensions add nothing to
/// start-up: the start-up test holds this at zero.
#[must_use]
pub fn runtimes_made() -> usize {
    thread::RUNTIMES_MADE.load(Ordering::SeqCst)
}

/// The most threads of this process left running because their script did
/// not stop when told to. A native function of the engine that loops
/// without looking at the interrupt (`Array.prototype.reverse` over an
/// object whose length is huge) cannot be stopped, and its thread runs on
/// until the process ends: past this many, no extension is started at
/// all (the review of step 28).
pub const MAX_ABANDONED: usize = 4;

/// The threads this process abandoned, of every extension.
static ABANDONED: AtomicUsize = AtomicUsize::new(0);

/// The extensions, by their snapshot's hash, whose thread this process
/// left running: none is started again until the harness restarts or it
/// is approved again ([`forget_abandoned`]). Kept for the process, not a
/// session: each session's start of it would leave another thread (the
/// review of step 28's fixes).
static DISABLED: std::sync::Mutex<Vec<ContentHash>> = std::sync::Mutex::new(Vec::new());

/// Marks the extension of `hash` as one whose thread was left running.
fn disable(hash: ContentHash) {
    let mut disabled = DISABLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !disabled.contains(&hash) {
        disabled.push(hash);
    }
}

/// Whether the extension of `hash` had a thread left running in this
/// process.
#[must_use]
pub fn is_disabled(hash: &ContentHash) -> bool {
    DISABLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(hash)
}

/// Lets the extension of `hash` be started again: the user approved it
/// anew, knowing its thread was left running.
pub fn forget_abandoned(hash: &ContentHash) {
    DISABLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|disabled| disabled != hash);
}

/// A point of a runtime's thread just after it took a call to run, before
/// it says the call started: a test holds it to give the call up then.
pub const AFTER_A_CALL_IS_CLAIMED: &str = "cineris-ext: after a call is claimed";

/// How many extension threads this process has abandoned.
#[must_use]
pub fn abandoned_threads() -> usize {
    ABANDONED.load(Ordering::SeqCst)
}

/// What a call runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A tool, by its name within the extension.
    Tool(String),
    /// A command, by its name.
    Command(String),
    /// A hook, by its place in the registration's list.
    Hook(usize),
}

/// A limit a runtime hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// The call's time.
    Time,
    /// The runtime's memory.
    Memory,
}

impl std::fmt::Display for Limit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Time => "it ran out of time",
            Self::Memory => "it ran out of memory",
        })
    }
}

/// Why a call failed inside the runtime.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    /// The script threw: the error and where.
    #[error("{0}")]
    Thrown(String),
    /// A limit stopped it, and the runtime was torn down.
    #[error("the extension's runtime was stopped: {0}")]
    Limit(Limit),
    /// What it returned cannot be a result.
    #[error("{0}")]
    Result(String),
    /// It registered nothing by that name.
    #[error("the extension has no {0}")]
    NoTarget(String),
}

/// Why a runtime could not be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    /// The folder could not be read as an extension.
    #[error("{0}")]
    Folder(#[from] FolderError),
    /// The manifest was refused.
    #[error("{0}")]
    Manifest(#[from] ManifestError),
    /// The folder is not what was approved.
    #[error(
        "the extension's files changed since they were approved, so it is not run until they are approved again"
    )]
    Changed {
        /// The approved hash.
        pinned: ContentHash,
        /// The hash found.
        found: ContentHash,
    },
    /// The main module threw while it was evaluated.
    #[error("the extension's main module failed: {0}")]
    Evaluation(String),
    /// A limit stopped the main module.
    #[error("the extension's main module was stopped: {0}")]
    Limit(Limit),
    /// It registered something other than what was approved.
    #[error(
        "the extension registered other tools, commands or hooks than when it was approved, so it is not run until it is approved again"
    )]
    Mismatch,
    /// The engine could not be made.
    #[error("the script engine failed: {0}")]
    Engine(String),
    /// The main module did not stop when its time was up, and its thread
    /// was left running.
    #[error(
        "the extension's main module did not stop when its time was up, and its thread was left \
         running: it is not started again until the harness restarts or it is approved again"
    )]
    Abandoned,
    /// An earlier thread of this extension was left running.
    #[error(
        "the extension is disabled: its script did not stop when its time was up, and its thread \
         was left running. It is started again after the harness restarts, or once it is \
         approved again"
    )]
    Disabled,
    /// Too many extension threads did not stop when told to.
    #[error(
        "{0} extension threads did not stop when told to and are still running, so no extension \
         is started until the harness restarts"
    )]
    TooManyAbandoned(usize),
}

/// Why a call did not give a result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CallError {
    /// It failed in the runtime.
    #[error("{0}")]
    Failed(#[from] Failure),
    /// Its time ran out.
    #[error("the extension's call ran out of its time, and its runtime was stopped")]
    TimedOut,
    /// It was cancelled.
    #[error("the extension's call was interrupted")]
    Cancelled,
    /// A host call found its turn stopped: the call ends there, and its
    /// runtime with it.
    #[error("the turn was stopped: {0}")]
    Stopped(String),
    /// The runtime is gone: torn down by a limit, or its thread ended.
    #[error("the extension's runtime is not running")]
    Gone,
    /// Another runtime of the extension left its thread running.
    #[error(
        "the extension is disabled: its script did not stop when its time was up, and its thread \
         was left running. It is started again after the harness restarts"
    )]
    Disabled,
}

/// What a runtime is made from.
#[derive(Debug, Clone)]
pub enum Source {
    /// The extension's folder, and the hash it must have: what a session
    /// runs.
    Folder {
        /// The folder.
        path: PathBuf,
        /// The pinned hash.
        pinned: ContentHash,
    },
    /// A snapshot already read: the one shown for approval, registered
    /// before it is pinned.
    Snapshot(Arc<Snapshot>),
}

/// What a call needs from the session.
pub struct CallContext<'a, H: Host + ?Sized> {
    /// The session's side of the host functions.
    pub host: &'a H,
    /// The extension's capabilities, compiled for the workspace.
    pub capabilities: &'a Capabilities,
    /// The call's time, from when it starts running.
    pub time: Duration,
    /// The session's timer.
    pub timer: &'a dyn Timer,
    /// The session's clock, which the timer moves: what is left of the
    /// call's time when a host call is made.
    pub clock: &'a dyn Clock,
    /// Fires when the call must stop.
    pub cancel: &'a CancelToken,
}

impl<H: Host + ?Sized> std::fmt::Debug for CallContext<'_, H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallContext")
            .field("time", &self.time)
            .finish_non_exhaustive()
    }
}

/// One extension's runtime, on its thread.
pub struct ScriptRuntime {
    inbox: mpsc::Sender<ToThread>,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    /// Whether its thread was left running: told to stop, it did not.
    abandoned: AtomicBool,
    loaded: Loaded,
}

impl std::fmt::Debug for ScriptRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptRuntime")
            .field("name", &self.loaded.manifest.name)
            .field("alive", &self.is_alive())
            .finish_non_exhaustive()
    }
}

impl ScriptRuntime {
    /// Starts a runtime: a thread that reads the source, checks it, makes
    /// the runtime and evaluates the main module. `expected` is what the
    /// extension registered when it was approved; registering anything
    /// else refuses it.
    ///
    /// # Errors
    ///
    /// [`LoadError`] says why the runtime could not be made.
    pub async fn start(
        name: &str,
        source: Source,
        expected: Option<Registration>,
        timer: &dyn Timer,
    ) -> Result<Self, LoadError> {
        let abandoned = ABANDONED.load(Ordering::SeqCst);
        if abandoned >= MAX_ABANDONED {
            return Err(LoadError::TooManyAbandoned(abandoned));
        }
        let hash = match &source {
            Source::Folder { pinned, .. } => *pinned,
            Source::Snapshot(snapshot) => snapshot.hash(),
        };
        if is_disabled(&hash) {
            return Err(LoadError::Disabled);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let (ready, readied) = oneshot::channel();
        let source = match source {
            Source::Folder { path, pinned } => thread::Source::Folder { path, pinned },
            Source::Snapshot(snapshot) => thread::Source::Snapshot(snapshot),
        };
        let inbox = thread::spawn(
            name,
            source,
            expected,
            STACK_BYTES,
            Arc::clone(&stop),
            Arc::clone(&alive),
            ready,
        )
        .map_err(|error| LoadError::Engine(format!("its thread could not start: {error}")))?;
        let mut readied = pin!(readied);
        let mut limit = timer.sleep(EVALUATION_TIME);
        let mut grace: Option<cineris_base::Sleep> = None;
        let loaded = poll_fn(|cx| {
            if let Poll::Ready(result) = readied.as_mut().poll(cx) {
                return Poll::Ready(result.ok());
            }
            if let Some(grace) = &mut grace {
                if grace.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(None);
                }
            } else if limit.as_mut().poll(cx).is_ready() {
                stop.store(true, Ordering::SeqCst);
                let mut wait = timer.sleep(GRACE);
                // Polled once, so that it wakes this task when it is up.
                let _ = wait.as_mut().poll(cx);
                grace = Some(wait);
            }
            Poll::Pending
        })
        .await;
        match loaded {
            Some(Ok(loaded)) => Ok(Self {
                inbox,
                stop,
                alive,
                abandoned: AtomicBool::new(false),
                loaded,
            }),
            Some(Err(error)) => Err(error),
            // Its grace passed: the thread is left running.
            None if grace.is_some() => {
                ABANDONED.fetch_add(1, Ordering::SeqCst);
                disable(hash);
                Err(LoadError::Abandoned)
            }
            None => Err(LoadError::Limit(Limit::Time)),
        }
    }

    /// Whether this runtime's thread was left running: told to stop, it
    /// did not within its grace. An extension whose thread was abandoned is
    /// not given another in this process.
    #[must_use]
    pub fn was_abandoned(&self) -> bool {
        self.abandoned.load(Ordering::SeqCst)
    }

    /// The extension's manifest, as the snapshot holds it.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.loaded.manifest
    }

    /// What the main module registered.
    #[must_use]
    pub fn registration(&self) -> &Registration {
        &self.loaded.registration
    }

    /// The hash of the snapshot the runtime was made from.
    #[must_use]
    pub fn hash(&self) -> ContentHash {
        self.loaded.hash
    }

    /// The text of the extension's prompt file, if it has one.
    #[must_use]
    pub fn prompt(&self) -> Option<&str> {
        self.loaded.prompt.as_deref()
    }

    /// Whether the runtime can still take calls.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && !self.stop.load(Ordering::SeqCst)
    }

    /// Runs a call: waits its turn, then serves its host calls through
    /// the call's context until it ends, its time is up, or it is cancelled.
    ///
    /// # Errors
    ///
    /// [`CallError`] says why there is no result.
    pub async fn call<H: Host + ?Sized>(
        &self,
        target: Target,
        input: Json,
        cx: CallContext<'_, H>,
    ) -> Result<Json, CallError> {
        // A call already cancelled is not sent at all.
        if cx.cancel.is_cancelled() {
            return Err(CallError::Cancelled);
        }
        let claim = Arc::new(AtomicU8::new(QUEUED));
        // A call whose future is dropped before it ends (its task given
        // up) is withdrawn if it has not started, and otherwise stops the
        // runtime: nothing else would.
        let guard = StopIfDropped {
            runtime: Some(self),
            claim: Arc::clone(&claim),
        };
        let result = self.call_to_its_end(target, input, cx, claim).await;
        guard.disarm();
        result
    }

    async fn call_to_its_end<H: Host + ?Sized>(
        &self,
        target: Target,
        input: Json,
        cx: CallContext<'_, H>,
        claim: Arc<AtomicU8>,
    ) -> Result<Json, CallError> {
        if !self.is_alive() {
            return Err(CallError::Gone);
        }
        // Another runtime of this extension left its thread running: this
        // one takes no more calls either (the review of step 28's fixes:
        // each live runtime could leave one more).
        if is_disabled(&self.loaded.hash) {
            return Err(CallError::Disabled);
        }
        let phase = match target {
            Target::Tool(_) => Phase::Tool,
            Target::Command(_) => Phase::Command,
            Target::Hook(_) => Phase::Hook,
        };
        let (outbox, mut replies) = async_mpsc::unbounded_channel();
        self.inbox
            .send(ToThread::Invoke(Invocation {
                target,
                input,
                outbox,
                claim: Arc::clone(&claim),
            }))
            .map_err(|_| CallError::Gone)?;
        let mut deadline: Option<cineris_base::Sleep> = None;
        let mut started = cx.clock.monotonic();
        let mut cancelled = pin!(cx.cancel.cancelled());
        loop {
            let event = poll_fn(|context| {
                if let Poll::Ready(reply) = replies.poll_recv(context) {
                    return Poll::Ready(Event::Reply(reply));
                }
                if let Some(deadline) = &mut deadline
                    && deadline.as_mut().poll(context).is_ready()
                {
                    return Poll::Ready(Event::Deadline);
                }
                if cancelled.as_mut().poll(context).is_ready() {
                    return Poll::Ready(Event::Cancelled);
                }
                Poll::Pending
            })
            .await;
            match event {
                Event::Reply(None) => return Err(CallError::Gone),
                Event::Reply(Some(FromThread::Started)) => {
                    started = cx.clock.monotonic();
                    deadline = Some(cx.timer.sleep(cx.time));
                }
                Event::Reply(Some(FromThread::Call { id, call })) => {
                    let left = cx
                        .time
                        .saturating_sub(cx.clock.monotonic().saturating_sub(started));
                    // A program is followed to its end: what it changed is
                    // looked at and recorded after it stops, which the
                    // runner does on the call's cancellation and at its
                    // time (the review of step 28: dropped, its look and
                    // its record were skipped).
                    let to_its_end = matches!(call, HostCall::Exec { .. });
                    // Any other host call runs under the same deadline and
                    // cancellation: dropped when either fires.
                    let mut ending: Option<CallError> = None;
                    let answer = {
                        let mut served = pin!(serve(call, cx.capabilities, cx.host, phase, left));
                        poll_fn(|context| {
                            if let Poll::Ready(answer) = served.as_mut().poll(context) {
                                return Poll::Ready(Some(answer));
                            }
                            if ending.is_none() {
                                if let Some(deadline) = &mut deadline
                                    && deadline.as_mut().poll(context).is_ready()
                                {
                                    ending = Some(CallError::TimedOut);
                                } else if cancelled.as_mut().poll(context).is_ready() {
                                    ending = Some(CallError::Cancelled);
                                }
                                if ending.is_some() && !to_its_end {
                                    return Poll::Ready(None);
                                }
                            }
                            Poll::Pending
                        })
                        .await
                    };
                    if let Some(error) = ending {
                        // A program whose look found its turn stopped says
                        // so, over the cancel or the deadline that ended it.
                        let error = match answer {
                            Some(Err(HostError::Stopped(reason))) => CallError::Stopped(reason),
                            _ => error,
                        };
                        return Err(self.stopped(&mut replies, cx.timer, error).await);
                    }
                    match answer {
                        // Nothing more of a stopped turn runs, the
                        // script's code included.
                        Some(Err(HostError::Stopped(reason))) => {
                            return Err(self
                                .stopped(&mut replies, cx.timer, CallError::Stopped(reason))
                                .await);
                        }
                        Some(answer) => {
                            let _ = self.inbox.send(ToThread::Answer { id, answer });
                        }
                        None => {
                            return Err(self
                                .stopped(&mut replies, cx.timer, CallError::TimedOut)
                                .await);
                        }
                    }
                }
                Event::Reply(Some(FromThread::Done(result))) => {
                    return result.map_err(|failure| match failure {
                        Failure::Limit(Limit::Time) if cx.cancel.is_cancelled() => {
                            CallError::Cancelled
                        }
                        Failure::Limit(Limit::Time) => CallError::TimedOut,
                        other => CallError::Failed(other),
                    });
                }
                Event::Deadline => {
                    return Err(self
                        .stopped(&mut replies, cx.timer, CallError::TimedOut)
                        .await);
                }
                // Cancelled before its turn came: withdrawn, and the call
                // the runtime is serving goes on. One that started after
                // all is stopped as any other.
                Event::Cancelled
                    if claim
                        .compare_exchange(QUEUED, WITHDRAWN, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok() =>
                {
                    return Err(CallError::Cancelled);
                }
                Event::Cancelled => {
                    return Err(self
                        .stopped(&mut replies, cx.timer, CallError::Cancelled)
                        .await);
                }
            }
        }
    }

    /// Stops the runtime for a call that must end now, and waits a grace
    /// for its thread to say so; past it, the thread is abandoned.
    async fn stopped(
        &self,
        replies: &mut async_mpsc::UnboundedReceiver<FromThread>,
        timer: &dyn Timer,
        error: CallError,
    ) -> CallError {
        self.stop.store(true, Ordering::SeqCst);
        // A thread waiting for a host call's answer runs none of the
        // script's code, so the interrupt never comes: this wakes it, and
        // it ends.
        let _ = self.inbox.send(ToThread::Shutdown);
        let mut grace = timer.sleep(GRACE);
        poll_fn(|context| {
            loop {
                match replies.poll_recv(context) {
                    Poll::Ready(Some(FromThread::Done(_)) | None) => return Poll::Ready(()),
                    Poll::Ready(Some(_)) => {}
                    Poll::Pending => break,
                }
            }
            if grace.as_mut().poll(context).is_ready() {
                // Abandoned: it counts as torn down, and as one of the
                // process's threads that would not stop.
                self.alive.store(false, Ordering::SeqCst);
                if !self.abandoned.swap(true, Ordering::SeqCst) {
                    ABANDONED.fetch_add(1, Ordering::SeqCst);
                    disable(self.loaded.hash);
                }
                return Poll::Ready(());
            }
            Poll::Pending
        })
        .await;
        error
    }
}

impl Drop for ScriptRuntime {
    fn drop(&mut self) {
        // A script running when its runtime goes is stopped, not left to
        // run on with no deadline.
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.inbox.send(ToThread::Shutdown);
    }
}

/// Withdraws a call whose future was dropped before it started, and stops
/// the runtime of one dropped after it started.
struct StopIfDropped<'a> {
    runtime: Option<&'a ScriptRuntime>,
    claim: Arc<AtomicU8>,
}

impl StopIfDropped<'_> {
    fn disarm(mut self) {
        self.runtime = None;
    }
}

impl Drop for StopIfDropped<'_> {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime else {
            return;
        };
        // Not started: withdrawn, and the call the runtime serves goes on.
        if self
            .claim
            .compare_exchange(QUEUED, WITHDRAWN, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return;
        }
        runtime.stop.store(true, Ordering::SeqCst);
        let _ = runtime.inbox.send(ToThread::Shutdown);
    }
}

enum Event {
    Reply(Option<FromThread>),
    Deadline,
    Cancelled,
}

/// Evaluates an extension's snapshot once, in a runtime of its own that is
/// then ended, and gives its manifest and what it registered: what is
/// shown for approval and kept in the registration cache.
///
/// # Errors
///
/// [`LoadError`] says why it could not be evaluated.
pub async fn register(
    snapshot: Arc<Snapshot>,
    timer: &dyn Timer,
) -> Result<(Manifest, Registration), LoadError> {
    let name = Manifest::of(&snapshot)?.name;
    let runtime = ScriptRuntime::start(&name, Source::Snapshot(snapshot), None, timer).await?;
    Ok((
        runtime.loaded.manifest.clone(),
        runtime.loaded.registration.clone(),
    ))
}

impl From<HostError> for Failure {
    fn from(error: HostError) -> Self {
        Self::Thrown(error.to_string())
    }
}
