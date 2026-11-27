//! The script engine: QuickJS through `rquickjs` ([decision 0005]), behind
//! the `scripts` feature (`docs/steps/28/answered.md`, "The runtime").
//!
//! One runtime an extension, each on a thread of its own, made the first
//! time something of the extension's is used. A runtime runs one call at a
//! time, to its end, and a call's time starts when it starts running, a
//! hook's when it is asked ([`CallContext::asked`]). A
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

use thread::{ENDED, FromThread, Invocation, Loaded, QUEUED, ToThread, WITHDRAWN};

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

/// The cap of threads of this process left running because their script
/// did not stop when told to. A native function of the engine that loops
/// without looking at the interrupt (`Array.prototype.reverse` over an
/// object whose length is huge) cannot be stopped, and its thread runs on
/// until the process ends (the review of step 28).
///
/// A thread counts toward the cap from when it is told to stop (its call's
/// time up, its call cancelled or given up, its start's time up or its
/// start given up, its runtime dropped) until it ends, and from then on if
/// it has not ended within [`GRACE`] of being told, when it is left
/// running. A start or a call is let in only while the threads counted are
/// fewer than this; one let in waits first, within that grace, for any
/// thread of its own extension being stopped, and is refused if that one is
/// left running (the review of step 28's close: the cap was looked at only
/// as a runtime started, and an extension disabled only once a grace was
/// over). **What that bounds:** at most this many threads left running in
/// a process, plus the calls and starts already running when the count
/// reached it, which nothing can stop; and one an extension, plus one for
/// each of its other runtimes (in the process's other sessions) whose call
/// was already running, or whose start was already evaluating, when the
/// first was told to stop.
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

/// A runtime's thread as the cap counts it: running; told to stop and not
/// yet ended (it counts toward the cap); left running (counted in
/// [`abandoned_threads`] for good); ended.
const THREAD_RUNS: u8 = 0;
const THREAD_TOLD: u8 = 1;
const THREAD_LEFT: u8 = 2;
const THREAD_ENDED: u8 = 3;
/// Told to stop, it came back within its grace (its call's end, or its
/// start's, was told): it stopped as told, and is ending, so it counts no
/// more and nothing waits for it.
const THREAD_BACK: u8 = 4;

/// A runtime's thread, shared by the runtime, its start and the thread
/// itself, which says when it ends: what [`MAX_ABANDONED`] counts. Every
/// change of its state is made under [`BEING_STOPPED`]'s lock.
#[derive(Debug)]
pub(crate) struct Watched {
    hash: ContentHash,
    state: AtomicU8,
}

/// The threads told to stop that have neither ended nor been left running.
static BEING_STOPPED: std::sync::Mutex<Vec<Arc<Watched>>> = std::sync::Mutex::new(Vec::new());
/// Told whenever a thread leaves [`BEING_STOPPED`].
static SETTLED: tokio::sync::Notify = tokio::sync::Notify::const_new();

fn being_stopped() -> std::sync::MutexGuard<'static, Vec<Arc<Watched>>> {
    BEING_STOPPED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Watched {
    fn new(hash: ContentHash) -> Arc<Self> {
        Arc::new(Self {
            hash,
            state: AtomicU8::new(THREAD_RUNS),
        })
    }

    /// Its thread is told to stop: from now until it ends, or is left
    /// running, it counts toward the cap, and its extension's starts and
    /// calls wait for it to be judged. Nothing when it was told before, or
    /// has ended.
    fn tell(self: &Arc<Self>) {
        let mut stopping = being_stopped();
        if self.state.load(Ordering::SeqCst) == THREAD_RUNS {
            self.state.store(THREAD_TOLD, Ordering::SeqCst);
            stopping.push(Arc::clone(self));
        }
    }

    /// Its thread has ended, however it ended: released from the cap,
    /// unless it was left running before, which stays counted.
    pub(crate) fn ended(&self) {
        let released = {
            let mut stopping = being_stopped();
            let was = self.state.load(Ordering::SeqCst);
            if was != THREAD_LEFT {
                self.state.store(THREAD_ENDED, Ordering::SeqCst);
            }
            if was == THREAD_TOLD {
                stopping.retain(|other| !std::ptr::eq(Arc::as_ptr(other), self));
            }
            was == THREAD_TOLD
        };
        if released {
            SETTLED.notify_waiters();
        }
    }

    /// Its thread, told to stop, has not ended within its grace: it is
    /// left running, counted once whoever judges it, and its extension is
    /// disabled. Whether it is left running, now or before.
    fn leave(&self) -> bool {
        let left = {
            let mut stopping = being_stopped();
            match self.state.load(Ordering::SeqCst) {
                THREAD_TOLD => {
                    self.state.store(THREAD_LEFT, Ordering::SeqCst);
                    stopping.retain(|other| !std::ptr::eq(Arc::as_ptr(other), self));
                    ABANDONED.fetch_add(1, Ordering::SeqCst);
                    disable(self.hash);
                    Some(true)
                }
                THREAD_LEFT => Some(false),
                _ => None,
            }
        };
        if left == Some(true) {
            SETTLED.notify_waiters();
        }
        left.is_some()
    }

    /// Its thread, told to stop, came back within its grace: it stopped as
    /// it was told, and ends at the shutdown it was sent, so it is released
    /// from the cap at once, not when it has ended. Counted until it ended,
    /// a start or a call of its extension made meanwhile waited for an end
    /// that takes real time while its grace ran on the session's timer, and
    /// one the test moved for another wait left it running (the review of
    /// the step 28 fixes' third pull request: a stress run on Windows).
    fn came_back(&self) {
        let released = {
            let mut stopping = being_stopped();
            let told = self.state.load(Ordering::SeqCst) == THREAD_TOLD;
            if told {
                self.state.store(THREAD_BACK, Ordering::SeqCst);
                stopping.retain(|other| !std::ptr::eq(Arc::as_ptr(other), self));
            }
            told
        };
        if released {
            SETTLED.notify_waiters();
        }
    }

    fn is_left(&self) -> bool {
        self.state.load(Ordering::SeqCst) == THREAD_LEFT
    }
}

/// How many threads have been told to stop and have neither ended nor been
/// left running: each counts toward [`MAX_ABANDONED`].
#[must_use]
pub fn threads_being_stopped() -> usize {
    being_stopped().len()
}

/// Whether a start or a call of the extension of `hash` must wait: a thread
/// of that extension is being stopped, or the threads being stopped would,
/// with those left running and the starts let go, reach the cap.
fn must_wait(hash: &ContentHash) -> bool {
    let stopping = being_stopped();
    if stopping.iter().any(|watched| watched.hash == *hash) {
        return true;
    }
    let abandoned = ABANDONED.load(Ordering::SeqCst);
    !stopping.is_empty()
        && abandoned < MAX_ABANDONED
        && abandoned + stopping.len() + LET_GO.load(Ordering::SeqCst) >= MAX_ABANDONED
}

/// Waits, on `timer`, while a start or a call of the extension of `hash`
/// must ([`must_wait`]). Each thread being stopped is given a [`GRACE`] of
/// its own from when this wait first sees it, and is left running when
/// that grace has passed and it has not ended, judged here as its own call
/// would judge it: a thread whose call or start was given up, or whose
/// runtime was dropped, has nobody else to judge it. A thread that ends,
/// or another being told, changes no grace already running (the review of
/// the step 28 fixes' third pull request: a round begun again at every
/// change could wait without bound while other threads kept ending). So
/// the wait ends at most a grace after the last thread it saw told.
///
/// The timer has no clock, and a thread may be told where no timer is (a
/// runtime dropped): the grace runs from when the wait sees the thread,
/// which is never before it was told, so a thread is never judged early.
async fn settle(hash: &ContentHash, timer: &dyn Timer) {
    let mut watching: Vec<(Arc<Watched>, cineris_base::Sleep)> = Vec::new();
    loop {
        let mut settled = pin!(SETTLED.notified());
        settled.as_mut().enable();
        {
            let stopping = being_stopped();
            watching
                .retain(|(watched, _)| stopping.iter().any(|other| Arc::ptr_eq(other, watched)));
            for watched in stopping.iter() {
                if !watching.iter().any(|(seen, _)| Arc::ptr_eq(seen, watched)) {
                    watching.push((Arc::clone(watched), timer.sleep(GRACE)));
                }
            }
        }
        if !must_wait(hash) {
            return;
        }
        let graced: Vec<Arc<Watched>> = poll_fn(|cx| {
            if settled.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Vec::new());
            }
            let up: Vec<Arc<Watched>> = watching
                .iter_mut()
                .filter_map(|(watched, grace)| {
                    grace
                        .as_mut()
                        .poll(cx)
                        .is_ready()
                        .then(|| Arc::clone(watched))
                })
                .collect();
            if up.is_empty() {
                Poll::Pending
            } else {
                Poll::Ready(up)
            }
        })
        .await;
        for watched in graced {
            watched.leave();
            watching.retain(|(seen, _)| !Arc::ptr_eq(seen, &watched));
        }
    }
}

/// A point of a runtime's thread just after it took a call to run, before
/// it says the call started: a test holds it to give the call up then.
pub const AFTER_A_CALL_IS_CLAIMED: &str = "cineris-ext: after a call is claimed";

/// A point of a runtime's thread after it said a call started, before it
/// looks whether the runtime was stopped meanwhile and, if not, runs the
/// call's script: a test holds it to stop the runtime then (since step 28's
/// third pull request).
pub const A_CALL_IS_STARTED: &str =
    "cineris-ext: a call is said to have started, and its script is not begun";

/// A point of a runtime's thread as a definition (`cineris.tool`,
/// `cineris.command`, `cineris.on`) the main module made is taken, before
/// the module goes on: a test counts it to know where the module's first
/// evaluation has got to, which no host call can tell while it runs.
pub const A_DEFINITION_IS_REGISTERED: &str =
    "cineris-ext: a definition of the main module is taken";

/// How many extension threads this process has abandoned.
#[must_use]
pub fn abandoned_threads() -> usize {
    ABANDONED.load(Ordering::SeqCst)
}

/// How many starts of a runtime nobody waits for any more are still
/// evaluating: each counts toward [`MAX_ABANDONED`] while it runs, as one
/// left running does, so that starts let go cannot stack.
static LET_GO: AtomicUsize = AtomicUsize::new(0);

/// How many starts nobody waits for any more are still evaluating.
#[must_use]
pub fn starts_let_go() -> usize {
    LET_GO.load(Ordering::SeqCst)
}

/// A start's state, as [`StartWatch`] keeps it: not begun, let go before
/// it began, running, running let go (the one state that counts), ended.
const START_IDLE: u8 = 0;
const START_IDLE_LET_GO: u8 = 1;
const START_RUNNING: u8 = 2;
const START_RUNNING_LET_GO: u8 = 3;
const START_ENDED: u8 = 4;

/// What its caller tells a runtime's start (the third re-check of the
/// script hooks, (b), and the fifth look, M1): let go
/// ([`let_go`](Self::let_go)), the start counts toward [`MAX_ABANDONED`]
/// while it runs, and only then: one that never begins (its caller found a
/// runtime alive, or refused it before) counts nothing. The count is taken
/// and given back only while [`ScriptRuntime::start_watched`] runs, by the
/// guard every way out of it goes through: ended within its time, it is
/// released; left running, it is the one count of an abandoned thread,
/// never two.
#[derive(Debug, Clone, Default)]
pub struct StartWatch {
    state: Arc<AtomicU8>,
}

impl StartWatch {
    /// Nobody waits for the start any more: while it runs it counts toward
    /// the cap. Nothing when it has ended, and nothing counted yet when it
    /// has not begun: it counts once it begins, if it does.
    pub fn let_go(&self) {
        loop {
            match self.state.load(Ordering::SeqCst) {
                START_IDLE => {
                    if self
                        .state
                        .compare_exchange(
                            START_IDLE,
                            START_IDLE_LET_GO,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_ok()
                    {
                        return;
                    }
                }
                START_RUNNING => {
                    LET_GO.fetch_add(1, Ordering::SeqCst);
                    if self
                        .state
                        .compare_exchange(
                            START_RUNNING,
                            START_RUNNING_LET_GO,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_ok()
                    {
                        return;
                    }
                    LET_GO.fetch_sub(1, Ordering::SeqCst);
                }
                _ => return,
            }
        }
    }

    /// The start begins: counted at once if it was let go before.
    fn began(&self) {
        loop {
            match self.state.load(Ordering::SeqCst) {
                START_IDLE => {
                    if self
                        .state
                        .compare_exchange(
                            START_IDLE,
                            START_RUNNING,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_ok()
                    {
                        return;
                    }
                }
                START_IDLE_LET_GO => {
                    LET_GO.fetch_add(1, Ordering::SeqCst);
                    self.state.store(START_RUNNING_LET_GO, Ordering::SeqCst);
                    return;
                }
                _ => return,
            }
        }
    }

    /// The start has ended, however it did: released from the cap.
    fn ended(&self) {
        if self.state.swap(START_ENDED, Ordering::SeqCst) == START_RUNNING_LET_GO {
            LET_GO.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// A runtime's start, to its end: its thread is told to stop when the start
/// is dropped before it ends (its caller gone, the async runtime ended), so
/// that a main module in a loop is interrupted rather than left to run for
/// ever unnoticed; and its [`StartWatch`] is told it ended (the re-check of
/// the script hooks, (a)).
struct Starting<'a> {
    /// Its thread's stop, and the thread as the cap counts it: told to
    /// stop, it counts toward the cap until it ends (the review of step
    /// 28's close: a start dropped in a native loop left a thread nothing
    /// counted).
    stop: Option<(Arc<AtomicBool>, Arc<Watched>)>,
    watch: &'a StartWatch,
}

impl Starting<'_> {
    /// It ended, as its result says: nothing to stop.
    fn ended(mut self) {
        self.stop = None;
    }
}

impl Drop for Starting<'_> {
    fn drop(&mut self) {
        if let Some((stop, watched)) = &self.stop {
            stop.store(true, Ordering::SeqCst);
            watched.tell();
        }
        self.watch.ended();
    }
}

/// A point of a runtime's thread once its main module's first evaluation
/// is over, before its start is told how it went: a test holds it to have
/// the start's time run out just as the evaluation succeeds.
pub const A_MAIN_MODULE_IS_EVALUATED: &str =
    "cineris-ext: a main module is evaluated, and its start not yet told";

/// A point of a runtime's thread as it ends, however it ends (a point a
/// test can stop at: [`cineris_base::hook`]).
pub const A_THREAD_ENDS: &str = "cineris-ext: a runtime's thread ends";

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
    /// It never ran: its runtime was torn down, or stopped, before its
    /// script began, so nothing of it happened (since step 28's third pull
    /// request).
    #[error("the extension's runtime was stopped before the call ran")]
    NotRun,
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
    /// Starts of runtimes nobody waits for any more (their sessions closed)
    /// are still evaluating, and with the threads left running reach the cap.
    #[error(
        "{0} extension starts that closed sessions let go are still evaluating, so no extension \
         is started until one of them ends, which it does within its evaluation time"
    )]
    StartsLetGo(usize),
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
    /// The runtime is gone: torn down by a limit, or its thread ended,
    /// after the call began.
    #[error("the extension's runtime is not running")]
    Gone,
    /// The call never ran: it was queued under a runtime that was torn down
    /// before its turn, or stopped as it was taken, so nothing of it
    /// happened, and a fresh runtime may take it (since step 28's third
    /// pull request: it said its time ran out, or that the runtime was
    /// gone).
    #[error("the extension's runtime was stopped before the call ran")]
    NotRun,
    /// Another runtime of the extension left its thread running.
    #[error(
        "the extension is disabled: its script did not stop when its time was up, and its thread \
         was left running. It is started again after the harness restarts"
    )]
    Disabled,
    /// Too many extensions' threads were left running: no call runs, as no
    /// runtime starts (the review of step 28's close: a runtime alive
    /// before the cap was reached took calls past it).
    #[error(
        "{0} extension threads did not stop when told to and are still running, so no extension's \
         call runs until the harness restarts"
    )]
    TooManyAbandoned(usize),
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
    /// The call's time: from when it starts running, or from [`asked`]
    /// when that is given.
    ///
    /// [`asked`]: Self::asked
    pub time: Duration,
    /// When the call's time began, on `clock`'s monotonic reading, for a
    /// call whose time starts before it is sent: a hook's, whose time starts
    /// when it is asked (`docs/architecture/12-extensions.md`, "Tier 3:
    /// hooks"), so that its wait behind the runtime's other calls is within
    /// it. Its deadline then runs from the first, and a call still waiting
    /// for its turn when it comes is withdrawn before it starts, the
    /// runtime's other calls going on; one that has started is stopped as
    /// any call past its time is. `None`: from when it starts running, a
    /// tool's and a command's.
    pub asked: Option<Duration>,
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
    /// Its thread as the cap counts it: whether it was told to stop, left
    /// running, or ended.
    watched: Arc<Watched>,
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
        Self::start_watched(name, source, expected, timer, &StartWatch::default()).await
    }

    /// [`start`](Self::start), told by `watch` when nobody waits for it any
    /// more: it then counts toward [`MAX_ABANDONED`] until it ends.
    ///
    /// # Errors
    ///
    /// As [`start`](Self::start); and, while threads left running and starts
    /// let go together reach [`MAX_ABANDONED`],
    /// [`LoadError::StartsLetGo`].
    pub async fn start_watched(
        name: &str,
        source: Source,
        expected: Option<Registration>,
        timer: &dyn Timer,
        watch: &StartWatch,
    ) -> Result<Self, LoadError> {
        // Every way out of here tells `watch` the start ended; once its
        // thread runs, a drop before its end stops it.
        let mut starting = Starting { stop: None, watch };
        let hash = match &source {
            Source::Folder { pinned, .. } => *pinned,
            Source::Snapshot(snapshot) => snapshot.hash(),
        };
        // A thread of this extension being stopped (one in its grace) is
        // judged before another of it starts, and threads being stopped
        // count toward the cap (the review of step 28's close: a runtime
        // started in another's grace, before the extension was disabled,
        // left a second thread).
        settle(&hash, timer).await;
        let abandoned = ABANDONED.load(Ordering::SeqCst);
        if abandoned >= MAX_ABANDONED {
            return Err(LoadError::TooManyAbandoned(abandoned));
        }
        let let_go = LET_GO.load(Ordering::SeqCst);
        if abandoned + let_go >= MAX_ABANDONED {
            return Err(LoadError::StartsLetGo(let_go));
        }
        if is_disabled(&hash) {
            return Err(LoadError::Disabled);
        }
        // It begins: from here, let go, it counts toward the cap, and the
        // guard above gives the count back on every way out.
        watch.began();
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let watched = Watched::new(hash);
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
            thread::Alive(Arc::clone(&alive), Arc::clone(&watched)),
            ready,
        )
        .map_err(|error| LoadError::Engine(format!("its thread could not start: {error}")))?;
        starting.stop = Some((Arc::clone(&stop), Arc::clone(&watched)));
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
                watched.tell();
                let mut wait = timer.sleep(GRACE);
                // Polled once, so that it wakes this task when it is up.
                let _ = wait.as_mut().poll(cx);
                grace = Some(wait);
            }
            Poll::Pending
        })
        .await;
        starting.ended();
        // Its thread told its start's end: one told to stop at the start's
        // time came back.
        if loaded.is_some() {
            watched.came_back();
        }
        match loaded {
            // Evaluated in its grace, its time up: its time ran out, and the
            // runtime, stopped, is no runtime. Its inbox is dropped here, so
            // its thread, idle and told to stop, ends, and is not left
            // running for want of a shutdown (the review of the step 28
            // fixes' third pull request).
            Some(Ok(_)) if grace.is_some() => Err(LoadError::Limit(Limit::Time)),
            Some(Ok(loaded)) => Ok(Self {
                inbox,
                stop,
                alive,
                watched,
                loaded,
            }),
            Some(Err(error)) => Err(error),
            // Its grace passed: the thread is left running, unless it ended
            // just then.
            None if grace.is_some() && watched.leave() => Err(LoadError::Abandoned),
            None => Err(LoadError::Limit(Limit::Time)),
        }
    }

    /// Whether this runtime's thread was left running: told to stop, it
    /// did not within its grace. An extension whose thread was abandoned is
    /// not given another in this process.
    #[must_use]
    pub fn was_abandoned(&self) -> bool {
        self.watched.is_left()
    }

    /// Tells the runtime's thread to stop: it counts toward the cap until it
    /// ends, or is left running. A thread waiting for a host call's answer
    /// runs none of the script's code, so the interrupt never comes: the
    /// shutdown wakes it, and it ends.
    fn stop_thread(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.watched.tell();
        let _ = self.inbox.send(ToThread::Shutdown);
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
        // A hook's time began when it was asked: its deadline runs while it
        // waits for its turn. Any other call's begins when it starts.
        let mut started = cx.asked.unwrap_or_else(|| cx.clock.monotonic());
        let mut deadline: Option<cineris_base::Sleep> = cx.asked.map(|asked| {
            let waited = cx.clock.monotonic().saturating_sub(asked);
            cx.timer.sleep(cx.time.saturating_sub(waited))
        });
        let mut cancelled = pin!(cx.cancel.cancelled());
        // A thread of this extension being stopped (another runtime's, in
        // its grace) is judged before this call is let in, and threads
        // being stopped count toward the cap, which is looked at here as
        // at a start (the review of step 28's close: a call let in during
        // another runtime's grace, or on a runtime alive before the cap was
        // reached, left one more thread). The wait is within a hook's time
        // and ends at the call's cancel: nothing has been sent.
        {
            let mut settling = pin!(settle(&self.loaded.hash, cx.timer));
            let waited = poll_fn(|context| {
                if settling.as_mut().poll(context).is_ready() {
                    return Poll::Ready(None);
                }
                if let Some(deadline) = &mut deadline
                    && deadline.as_mut().poll(context).is_ready()
                {
                    return Poll::Ready(Some(CallError::TimedOut));
                }
                if cancelled.as_mut().poll(context).is_ready() {
                    return Poll::Ready(Some(CallError::Cancelled));
                }
                Poll::Pending
            })
            .await;
            if let Some(error) = waited {
                return Err(error);
            }
        }
        // Another runtime of this extension left its thread running: this
        // one takes no more calls either (the review of step 28's fixes:
        // each live runtime could leave one more).
        if is_disabled(&self.loaded.hash) {
            return Err(CallError::Disabled);
        }
        let abandoned = ABANDONED.load(Ordering::SeqCst);
        if abandoned >= MAX_ABANDONED {
            return Err(CallError::TooManyAbandoned(abandoned));
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
        // Set once the call has ended and its cancel has come: only its end
        // is waited for then.
        let mut ended_cancel = false;
        // Whether the thread said the call began.
        let mut began = false;
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
                if !ended_cancel && cancelled.as_mut().poll(context).is_ready() {
                    return Poll::Ready(Event::Cancelled);
                }
                Poll::Pending
            })
            .await;
            match event {
                // The thread ended: a call it never began did not run.
                Event::Reply(None) if !began => return Err(CallError::NotRun),
                Event::Reply(None) => return Err(CallError::Gone),
                Event::Reply(Some(FromThread::Started)) => {
                    began = true;
                    if cx.asked.is_none() {
                        started = cx.clock.monotonic();
                        deadline = Some(cx.timer.sleep(cx.time));
                    }
                }
                // A host call of a call that has ended (one its script did
                // not await) is not made: its end is on its way. One that
                // only tells (`log`, `notify`, `progress`) is, at once.
                Event::Reply(Some(FromThread::Call { call, .. }))
                    if has_ended(&claim) && !only_tells(&call) => {}
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
                            // The call's end is looked at before the host
                            // call's answer: an answer that comes at the
                            // same moment (a program's own limit, which is
                            // the call's) does not let the script go on
                            // past its time (the stress run of step 28's
                            // fixes).
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
                            if let Poll::Ready(answer) = served.as_mut().poll(context) {
                                return Poll::Ready(Some(answer));
                            }
                            Poll::Pending
                        })
                        .await
                    };
                    // Its time ran out, or it was cancelled, after it had
                    // ended: nothing runs to be stopped, and its end, on its
                    // way, is its result.
                    if ending.is_some() && has_ended(&claim) {
                        deadline = None;
                        ended_cancel = true;
                        continue;
                    }
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
                        Failure::NotRun => CallError::NotRun,
                        Failure::Limit(Limit::Time) if cx.cancel.is_cancelled() => {
                            CallError::Cancelled
                        }
                        Failure::Limit(Limit::Time) => CallError::TimedOut,
                        other => CallError::Failed(other),
                    });
                }
                // A hook's time, up while it waits for its turn: withdrawn,
                // and the call the runtime is serving goes on.
                Event::Deadline if withdrawn(&claim) => return Err(CallError::TimedOut),
                // Ended as its time ran out: its end is on its way.
                Event::Deadline if has_ended(&claim) => deadline = None,
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
                // Ended as it was cancelled: its end is on its way.
                Event::Cancelled if has_ended(&claim) => ended_cancel = true,
                Event::Cancelled => {
                    return Err(self
                        .stopped(&mut replies, cx.timer, CallError::Cancelled)
                        .await);
                }
            }
        }
    }

    /// Stops the runtime for a call that must end now, and waits a grace
    /// for its thread to say so; past it, the thread is abandoned. From the
    /// moment it is told, the thread counts toward the cap, and the
    /// extension's other starts and calls wait for it to be judged.
    async fn stopped(
        &self,
        replies: &mut async_mpsc::UnboundedReceiver<FromThread>,
        timer: &dyn Timer,
        error: CallError,
    ) -> CallError {
        self.stop_thread();
        let mut grace = timer.sleep(GRACE);
        poll_fn(|context| {
            loop {
                match replies.poll_recv(context) {
                    Poll::Ready(Some(FromThread::Done(_)) | None) => {
                        self.watched.came_back();
                        return Poll::Ready(());
                    }
                    Poll::Ready(Some(_)) => {}
                    Poll::Pending => break,
                }
            }
            if grace.as_mut().poll(context).is_ready() {
                // Abandoned: it counts as torn down, and as one of the
                // process's threads that would not stop, once, whoever
                // judged it first.
                self.alive.store(false, Ordering::SeqCst);
                self.watched.leave();
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
        // run on with no deadline, and counted until its thread ends.
        self.stop_thread();
    }
}

/// Whether a host call only tells something, at once, and is bounded by
/// its count in a call: what is still made of a call that has ended.
fn only_tells(call: &HostCall) -> bool {
    matches!(
        call,
        HostCall::Log(_) | HostCall::Notify(_) | HostCall::Progress(_)
    )
}

/// Whether the thread has run the call to its end.
fn has_ended(claim: &AtomicU8) -> bool {
    claim.load(Ordering::SeqCst) == ENDED
}

/// Takes a call that has not started from the thread: `true` when it was
/// still waiting, and is now withdrawn, never to run.
fn withdrawn(claim: &AtomicU8) -> bool {
    claim
        .compare_exchange(QUEUED, WITHDRAWN, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
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
        // Ended already: nothing of it runs, and the runtime's next call is
        // another's.
        if withdrawn(&self.claim) || has_ended(&self.claim) {
            return;
        }
        // Nobody judges its thread now: it counts toward the cap until it
        // ends, and the next start or call of its extension judges it when
        // a grace has passed (the review of step 28's close: a call given
        // up in a native loop left a thread nothing counted).
        runtime.stop_thread();
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

#[cfg(test)]
mod tests {
    use super::{StartWatch, starts_let_go};

    /// A start's count is taken and given back by its own guard alone, on
    /// every way: let go before it began, it counts nothing until it begins,
    /// and nothing at all if it never does (its caller returned first: a
    /// runtime alive, a link on the way, a look that failed); let go while
    /// it runs, it counts until it ends; ended, a later let-go counts
    /// nothing (the fifth look at the script hooks, M1).
    #[test]
    fn a_starts_count_is_held_only_while_it_runs() {
        let before = starts_let_go();
        // Never begun: nothing, whenever it is let go.
        let never = StartWatch::default();
        never.let_go();
        never.let_go();
        assert_eq!(starts_let_go(), before);
        drop(never);
        assert_eq!(starts_let_go(), before);
        // Let go before it began: counted from its start to its end.
        let early = StartWatch::default();
        early.let_go();
        assert_eq!(starts_let_go(), before);
        early.began();
        assert_eq!(starts_let_go(), before + 1);
        early.ended();
        assert_eq!(starts_let_go(), before);
        // Let go while it runs: counted until it ends, once.
        let running = StartWatch::default();
        running.began();
        assert_eq!(starts_let_go(), before);
        running.let_go();
        running.let_go();
        assert_eq!(starts_let_go(), before + 1);
        running.ended();
        assert_eq!(starts_let_go(), before);
        // Ended: a let-go counts nothing.
        running.let_go();
        assert_eq!(starts_let_go(), before);
    }
}
