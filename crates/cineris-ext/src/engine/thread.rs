//! The thread a runtime lives on (`docs/steps/28/answered.md`, "The
//! runtime").
//!
//! The thread reads the extension's folder into a snapshot, compares its
//! hash with the pin, makes the runtime with its limits, evaluates the
//! prelude and then the main module (when only registering is allowed),
//! and then runs one call at a time, to its end. While a call awaits a
//! host function, the thread waits for the answer, and a call that arrives
//! meanwhile waits its turn: the engine's interrupt check belongs to the
//! runtime, so two calls in it at once could not be told apart.
//!
//! Every value the thread keeps across calls (a registered function, a
//! promise's resolvers) is freed before the runtime is: the engine checks,
//! as it is freed, that nothing is left, and that check ends the process.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use cineris_base::ContentHash;
use rquickjs::context::intrinsic;
use rquickjs::function::Opt;
use rquickjs::promise::PromiseState;
use rquickjs::{
    CatchResultExt, CaughtError, Context, Ctx, Exception, Function, Module, Object, Persistent,
    Promise, Runtime, Value,
};
use serde_json::Value as Json;
use tokio::sync::{mpsc as async_mpsc, oneshot};

use super::loader::{SnapshotLoader, SnapshotResolver};
use super::{Failure, Limit, LoadError, Target};
use crate::event::Event;
use crate::folder::Snapshot;
use crate::host::{FetchCall, HostAnswer, HostCall, Phase, Scope};
use crate::manifest::Manifest;
use crate::registration::{
    CommandDef, HookDef, MAX_ARGUMENTS, MAX_COMMANDS, MAX_HOOK_TOOLS, MAX_HOOKS, MAX_TOOLS,
    Registration, ToolDef, check_allowed, check_description, check_filter, check_name,
    check_schema, check_timeout,
};

/// How many runtimes this process has made: the start-up test holds it at
/// zero.
pub(crate) static RUNTIMES_MADE: AtomicUsize = AtomicUsize::new(0);

/// The largest result a call gives, as JSON.
pub(crate) const MAX_RESULT_BYTES: usize = 1024 * 1024;
/// The longest description of a thrown error.
const MAX_THROWN: usize = 4096;

type Builtins = (
    intrinsic::Date,
    intrinsic::Eval,
    intrinsic::RegExpCompiler,
    intrinsic::RegExp,
    intrinsic::Json,
    intrinsic::MapSet,
    intrinsic::TypedArrays,
    intrinsic::Promise,
    intrinsic::WeakRef,
);

const PRELUDE: &str = include_str!("prelude.js");

/// What a runtime is made from.
pub(crate) enum Source {
    /// The extension's folder, read on the thread, and the hash it must
    /// have.
    Folder {
        /// The folder.
        path: PathBuf,
        /// The pinned hash.
        pinned: ContentHash,
    },
    /// A snapshot already read: the one shown for approval.
    Snapshot(Arc<Snapshot>),
}

/// What the thread is told to do.
pub(crate) enum ToThread {
    /// Run a call.
    Invoke(Invocation),
    /// The answer to a host call.
    Answer {
        /// The host call's number.
        id: u64,
        /// The answer.
        answer: HostAnswer,
    },
    /// End.
    Shutdown,
}

/// One call to run.
pub(crate) struct Invocation {
    pub(crate) target: Target,
    pub(crate) input: Json,
    pub(crate) outbox: async_mpsc::UnboundedSender<FromThread>,
    /// Who has the call: [`QUEUED`] until the thread starts it
    /// ([`STARTED`]) or its caller gives it up first ([`WITHDRAWN`]). Each
    /// side takes it with one compare-and-swap, so exactly one of them
    /// does (the review of step 28's fixes: a withdrawal told by closing
    /// the channel could miss a start already under way).
    pub(crate) claim: Arc<AtomicU8>,
}

/// A call waiting its turn.
pub(crate) const QUEUED: u8 = 0;
/// A call the thread has started.
pub(crate) const STARTED: u8 = 1;
/// A call its caller gave up before it started.
pub(crate) const WITHDRAWN: u8 = 2;

/// What the thread says about a call.
#[derive(Debug)]
pub(crate) enum FromThread {
    /// The call started running: its time starts now.
    Started,
    /// The script called a host function.
    Call {
        /// Its number, for the answer.
        id: u64,
        /// What it asked.
        call: HostCall,
    },
    /// The call ended.
    Done(Result<Json, Failure>),
}

/// What a loaded runtime says it is.
#[derive(Debug, Clone)]
pub(crate) struct Loaded {
    pub(crate) manifest: Manifest,
    pub(crate) registration: Registration,
    pub(crate) hash: ContentHash,
    pub(crate) prompt: Option<String>,
}

/// Starts the thread. `ready` is told once the main module has been
/// evaluated, or why it was not.
pub(crate) fn spawn(
    name: &str,
    source: Source,
    expected: Option<Registration>,
    stack: usize,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    ready: oneshot::Sender<Result<Loaded, LoadError>>,
) -> std::io::Result<mpsc::Sender<ToThread>> {
    let (sender, inbox) = mpsc::channel();
    std::thread::Builder::new()
        .name(format!("cineris-ext-{name}"))
        // The engine's own stack limit is far below this, and is measured
        // from where the runtime starts.
        .stack_size(stack * 8 + 1024 * 1024)
        .spawn(move || {
            let _alive = Alive(Arc::clone(&alive));
            body(source, expected, stack, &stop, &alive, ready, &inbox);
        })?;
    Ok(sender)
}

/// Says the runtime is gone when the thread ends, however it ends.
struct Alive(Arc<AtomicBool>);

impl Drop for Alive {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn body(
    source: Source,
    expected: Option<Registration>,
    stack: usize,
    stop: &Arc<AtomicBool>,
    alive: &AtomicBool,
    ready: oneshot::Sender<Result<Loaded, LoadError>>,
    inbox: &mpsc::Receiver<ToThread>,
) {
    let snapshot = match source {
        Source::Folder { path, pinned } => match Snapshot::read(&path) {
            Ok(snapshot) if snapshot.hash() == pinned => Arc::new(snapshot),
            Ok(snapshot) => {
                let _ = ready.send(Err(LoadError::Changed {
                    pinned,
                    found: snapshot.hash(),
                }));
                return;
            }
            Err(error) => {
                let _ = ready.send(Err(LoadError::Folder(error)));
                return;
            }
        },
        Source::Snapshot(snapshot) => snapshot,
    };
    let manifest = match Manifest::of(&snapshot) {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = ready.send(Err(LoadError::Manifest(error)));
            return;
        }
    };
    let prompt = manifest
        .prompt
        .as_deref()
        .and_then(|path| snapshot.file(path))
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    let Ok(runtime) = Runtime::new() else {
        let _ = ready.send(Err(LoadError::Engine(
            "the runtime could not be made".to_owned(),
        )));
        return;
    };
    RUNTIMES_MADE.fetch_add(1, Ordering::SeqCst);
    let memory = usize::try_from(manifest.limits.memory_mib * 1024 * 1024).unwrap_or(usize::MAX);
    runtime.set_memory_limit(memory);
    runtime.set_max_stack_size(stack);
    let interrupt = Arc::clone(stop);
    runtime.set_interrupt_handler(Some(Box::new(move || interrupt.load(Ordering::SeqCst))));
    runtime.set_loader(
        SnapshotResolver {
            snapshot: Arc::clone(&snapshot),
        },
        SnapshotLoader {
            snapshot: Arc::clone(&snapshot),
        },
    );
    let Ok(context) = Context::builder().with::<Builtins>().build(&runtime) else {
        let _ = ready.send(Err(LoadError::Engine(
            "the context could not be made".to_owned(),
        )));
        return;
    };
    let shared = Rc::new(RefCell::new(Shared::new(
        manifest.capabilities.hooks.clone(),
    )));
    // Everything kept across calls goes before the context and the runtime
    // do, however the thread ends: the engine checks, as the runtime is
    // freed, that nothing is left, and that check ends the process.
    let _clear = ClearOnDrop(&shared);
    let mut work = Work {
        runtime: &runtime,
        context: &context,
        shared: &shared,
        stop,
        alive,
        inbox,
        queue: VecDeque::new(),
    };
    let loaded = work
        .load(&manifest, &snapshot)
        .and_then(|registration| match expected {
            Some(expected) if expected != registration => Err(LoadError::Mismatch),
            _ => Ok(Loaded {
                manifest: manifest.clone(),
                registration,
                hash: snapshot.hash(),
                prompt,
            }),
        });
    let failed = loaded.is_err();
    let _ = ready.send(loaded);
    if !failed {
        work.serve();
    }
}

/// Frees what the thread keeps across calls when it is dropped, before the
/// context and the runtime are: on any end of the thread, an unwind
/// included.
struct ClearOnDrop<'a>(&'a Rc<RefCell<Shared>>);

impl Drop for ClearOnDrop<'_> {
    fn drop(&mut self) {
        if let Ok(mut shared) = self.0.try_borrow_mut() {
            shared.clear();
        }
    }
}

/// Where a call stands.
enum Running {
    /// It returned a value.
    Value(Json),
    /// It returned a promise, kept until it settles.
    Promise(Persistent<Promise<'static>>),
}

/// What a call's end means for the runtime.
enum After {
    /// The runtime goes on.
    Goes,
    /// A limit was hit, or the thread was told to end: it is torn down.
    TornDown,
}

struct Work<'a> {
    runtime: &'a Runtime,
    context: &'a Context,
    shared: &'a Rc<RefCell<Shared>>,
    stop: &'a Arc<AtomicBool>,
    alive: &'a AtomicBool,
    inbox: &'a mpsc::Receiver<ToThread>,
    queue: VecDeque<Invocation>,
}

impl Work<'_> {
    /// Evaluates the prelude and the main module, and gives what the
    /// module registered.
    fn load(
        &mut self,
        manifest: &Manifest,
        snapshot: &Snapshot,
    ) -> Result<Registration, LoadError> {
        let main = snapshot
            .file(&manifest.main)
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        let evaluated =
            self.context
                .with(|ctx| -> Result<Persistent<Promise<'static>>, LoadError> {
                    install(&ctx, self.shared)
                        .map_err(|error| LoadError::Engine(error.to_string()))?;
                    ctx.eval::<(), _>(PRELUDE)
                        .catch(&ctx)
                        .map_err(|error| LoadError::Engine(error.to_string()))?;
                    let promise = Module::evaluate(ctx.clone(), manifest.main.as_str(), main)
                        .catch(&ctx)
                        .map_err(|error| self.load_failure(&error))?;
                    Ok(Persistent::save(&ctx, promise))
                })?;
        // A module's evaluation is a promise: top-level `await` runs as
        // jobs. No host call can be answered now, so whatever it awaits
        // must settle by the jobs alone.
        if let Err(failure) = self.run_jobs() {
            return Err(failure_to_load(failure));
        }
        let state = self.context.with(|ctx| -> Result<(), LoadError> {
            let promise = evaluated.restore(&ctx).map_err(|error| LoadError::Engine(error.to_string()))?;
            match promise.state() {
                PromiseState::Resolved => Ok(()),
                PromiseState::Rejected => {
                    let error = promise
                        .result::<Value>()
                        .map(|result| result.catch(&ctx))
                        .and_then(Result::err)
                        .map_or_else(|| "the main module failed".to_owned(), |error| describe(&error));
                    Err(LoadError::Evaluation(error))
                }
                PromiseState::Pending => Err(LoadError::Evaluation(
                    "the main module waits for something that never comes: a host function cannot be \
                     called while it is first evaluated"
                        .to_owned(),
                )),
            }
        });
        let mut shared = self.shared.borrow_mut();
        shared.phase = Phase::Tool;
        state?;
        Ok(shared.registration.clone())
    }

    fn load_failure(&self, error: &CaughtError<'_>) -> LoadError {
        match self.classify(error) {
            Failure::Limit(limit) => LoadError::Limit(limit),
            other => LoadError::Evaluation(other.to_string()),
        }
    }

    /// Runs calls one at a time until the thread is told to end or a call
    /// tears the runtime down.
    fn serve(&mut self) {
        loop {
            let invocation = if let Some(invocation) = self.queue.pop_front() {
                invocation
            } else {
                match self.inbox.recv() {
                    Ok(ToThread::Invoke(invocation)) => invocation,
                    // An answer for a call that has ended.
                    Ok(ToThread::Answer { .. }) => continue,
                    Ok(ToThread::Shutdown) | Err(_) => return,
                }
            };
            if let After::TornDown = self.run(invocation) {
                return;
            }
        }
    }

    fn run(&mut self, invocation: Invocation) -> After {
        let Invocation {
            target,
            input,
            outbox,
            claim,
        } = invocation;
        if self.stop.load(Ordering::SeqCst) {
            self.alive.store(false, Ordering::SeqCst);
            let _ = outbox.send(FromThread::Done(Err(Failure::Limit(Limit::Time))));
            return After::TornDown;
        }
        // A call withdrawn before it started (its caller was cancelled
        // while it waited its turn) is not run: nobody waits for it.
        if claim
            .compare_exchange(QUEUED, STARTED, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return After::Goes;
        }
        cineris_base::hook::point(super::AFTER_A_CALL_IS_CLAIMED);
        if outbox.send(FromThread::Started).is_err() {
            return After::Goes;
        }
        self.shared
            .borrow_mut()
            .begin(outbox.clone(), phase_of(&target));
        let (result, after) = self.call(&target, &input);
        self.shared.borrow_mut().end();
        // A runtime that is torn down says so before the call's end is
        // told: whoever hears of the end must not hand it another call.
        if let After::TornDown = after {
            self.alive.store(false, Ordering::SeqCst);
        }
        let _ = outbox.send(FromThread::Done(result));
        cineris_base::hook::point(super::AFTER_A_CALLS_END_IS_TOLD);
        after
    }

    fn call(&mut self, target: &Target, input: &Json) -> (Result<Json, Failure>, After) {
        let started = self.context.with(|ctx| -> Result<Running, Failure> {
            let function = self
                .shared
                .borrow()
                .function(target)
                .ok_or_else(|| Failure::NoTarget(format!("{target:?}")))?;
            let function = function
                .restore(&ctx)
                .map_err(|error| Failure::Thrown(error.to_string()))?;
            let argument = to_js(&ctx, input)?;
            let value = function
                .call::<_, Value>((argument,))
                .catch(&ctx)
                .map_err(|error| self.classify(&error))?;
            if let Some(promise) = value.as_promise() {
                Ok(Running::Promise(Persistent::save(&ctx, promise.clone())))
            } else {
                Ok(Running::Value(from_js(&ctx, value)?))
            }
        });
        let promise = match started {
            Ok(Running::Value(value)) => return (Ok(value), After::Goes),
            Ok(Running::Promise(promise)) => promise,
            Err(failure) => {
                let after = after_failure(&failure);
                return (Err(failure), after);
            }
        };
        loop {
            if let Err(failure) = self.run_jobs() {
                return (Err(failure), After::TornDown);
            }
            let settled = self.context.with(|ctx| -> Option<Result<Json, Failure>> {
                let promise = match promise.clone().restore(&ctx) {
                    Ok(promise) => promise,
                    Err(error) => return Some(Err(Failure::Thrown(error.to_string()))),
                };
                match promise.state() {
                    PromiseState::Pending => None,
                    PromiseState::Resolved => Some(match promise.result::<Value>() {
                        Some(Ok(value)) => from_js(&ctx, value),
                        Some(Err(error)) => Err(Failure::Thrown(error.to_string())),
                        None => Ok(Json::Null),
                    }),
                    PromiseState::Rejected => Some(Err(match promise.result::<Value>() {
                        Some(result) => match result.catch(&ctx) {
                            Err(error) => self.classify(&error),
                            Ok(value) => Failure::Thrown(format!("{value:?}")),
                        },
                        None => Failure::Thrown("the call failed".to_owned()),
                    })),
                }
            });
            if let Some(result) = settled {
                let after = match &result {
                    Err(failure) => after_failure(failure),
                    Ok(_) => After::Goes,
                };
                return (result, after);
            }
            if self.shared.borrow().pending.is_empty() {
                return (
                    Err(Failure::Thrown(
                        "the call waits for something that never comes: it awaits no host function"
                            .to_owned(),
                    )),
                    After::Goes,
                );
            }
            // Wait for an answer; a call that arrives meanwhile waits its
            // turn.
            match self.inbox.recv() {
                Ok(ToThread::Answer { id, answer }) => {
                    if let Err(failure) = self.answer(id, answer) {
                        return (Err(failure), After::TornDown);
                    }
                }
                Ok(ToThread::Invoke(invocation)) => self.queue.push_back(invocation),
                Ok(ToThread::Shutdown) | Err(_) => {
                    return (Err(Failure::Limit(Limit::Time)), After::TornDown);
                }
            }
        }
    }

    /// Settles the promise of a host call.
    fn answer(&mut self, id: u64, answer: HostAnswer) -> Result<(), Failure> {
        let Some(Waiting {
            resolve, reject, ..
        }) = self.shared.borrow_mut().answered(id)
        else {
            return Ok(());
        };
        self.context.with(|ctx| -> Result<(), Failure> {
            let thrown = |error: rquickjs::Error| Failure::Thrown(error.to_string());
            match answer {
                Ok(value) => {
                    let value = to_js(&ctx, &value)?;
                    resolve
                        .restore(&ctx)
                        .map_err(thrown)?
                        .call::<_, ()>((value,))
                        .map_err(thrown)
                }
                Err(error) => {
                    let exception =
                        Exception::from_message(ctx.clone(), &error.to_string()).map_err(thrown)?;
                    reject
                        .restore(&ctx)
                        .map_err(thrown)?
                        .call::<_, ()>((exception,))
                        .map_err(thrown)
                }
            }
        })
    }

    /// Runs the jobs that are ready. A limit hit in one ends the call.
    fn run_jobs(&self) -> Result<(), Failure> {
        loop {
            match self.runtime.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(job) => {
                    let failure = job.0.with(|ctx| {
                        let value = ctx.catch();
                        let text = describe_value(&value);
                        self.classify_text(&text)
                    });
                    if let Failure::Limit(_) = failure {
                        return Err(failure);
                    }
                    // Another job threw: the promise it belonged to says so.
                }
            }
        }
    }

    fn classify(&self, error: &CaughtError<'_>) -> Failure {
        self.classify_text(&describe(error))
    }

    fn classify_text(&self, text: &str) -> Failure {
        if self.stop.load(Ordering::SeqCst) {
            Failure::Limit(Limit::Time)
        } else if text.contains("out of memory") {
            Failure::Limit(Limit::Memory)
        } else {
            Failure::Thrown(text.to_owned())
        }
    }
}

fn after_failure(failure: &Failure) -> After {
    if let Failure::Limit(_) = failure {
        After::TornDown
    } else {
        After::Goes
    }
}

fn failure_to_load(failure: Failure) -> LoadError {
    match failure {
        Failure::Limit(limit) => LoadError::Limit(limit),
        other => LoadError::Evaluation(other.to_string()),
    }
}

fn phase_of(target: &Target) -> Phase {
    match target {
        Target::Tool(_) => Phase::Tool,
        Target::Command(_) => Phase::Command,
        Target::Hook(_) => Phase::Hook,
    }
}

fn describe(error: &CaughtError<'_>) -> String {
    cineris_base::text::truncate_bytes(&error.to_string(), MAX_THROWN).to_owned()
}

fn describe_value(value: &Value<'_>) -> String {
    let text = value
        .as_exception()
        .map(|exception| {
            let message = exception.message().unwrap_or_default();
            match exception.stack() {
                Some(stack) if !stack.is_empty() => format!("{message}\n{stack}"),
                _ => message,
            }
        })
        .unwrap_or_else(|| format!("{value:?}"));
    cineris_base::text::truncate_bytes(&text, MAX_THROWN).to_owned()
}

fn to_js<'js>(ctx: &Ctx<'js>, value: &Json) -> Result<Value<'js>, Failure> {
    ctx.json_parse(value.to_string())
        .map_err(|error| Failure::Thrown(error.to_string()))
}

fn from_js<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Json, Failure> {
    let text = ctx
        .json_stringify(value)
        .catch(ctx)
        .map_err(|error| Failure::Result(describe(&error)))?;
    let Some(text) = text else {
        return Ok(Json::Null);
    };
    let text = text
        .to_string()
        .map_err(|error| Failure::Result(error.to_string()))?;
    if text.len() > MAX_RESULT_BYTES {
        return Err(Failure::Result(format!(
            "the result is {} bytes, more than the 1 MiB a call may give",
            text.len()
        )));
    }
    serde_json::from_str(&text).map_err(|error| Failure::Result(error.to_string()))
}

/// What the native functions share with the thread.
struct Shared {
    phase: Phase,
    hooks_allowed: Vec<Event>,
    registration: Registration,
    tools: HashMap<String, Persistent<Function<'static>>>,
    commands: HashMap<String, Persistent<Function<'static>>>,
    hooks: Vec<Persistent<Function<'static>>>,
    /// The host calls waiting for their answers.
    pending: HashMap<u64, Waiting>,
    /// How many of them a script awaits: every kind but the three that
    /// only tell (`log`, `notify`, `progress`), which their own counts
    /// bound.
    awaited: usize,
    /// The bytes of arguments the waiting calls hold.
    pending_bytes: usize,
    next: u64,
    outbox: Option<async_mpsc::UnboundedSender<FromThread>>,
    /// What the running call has said so far, for its caps.
    notified: u32,
    progressed: u32,
    logged: u32,
    asked: u32,
    /// `cineris.state.set` calls: each `session` one is a line of the
    /// session's log, which this keeps bounded.
    stored: u32,
}

/// A host call waiting for its answer.
struct Waiting {
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
    /// The size of its arguments.
    bytes: usize,
    /// Whether it counts in [`Shared::awaited`].
    awaited: bool,
}

/// Whether a host call of `kind` only tells something, and is bounded by
/// its count in a call rather than by the calls waiting.
fn only_tells(kind: &str) -> bool {
    matches!(kind, "log" | "notify" | "progress")
}

impl Shared {
    fn new(hooks_allowed: Vec<Event>) -> Self {
        Self {
            phase: Phase::Evaluation,
            hooks_allowed,
            registration: Registration::default(),
            tools: HashMap::new(),
            commands: HashMap::new(),
            hooks: Vec::new(),
            pending: HashMap::new(),
            awaited: 0,
            pending_bytes: 0,
            next: 0,
            outbox: None,
            notified: 0,
            progressed: 0,
            logged: 0,
            asked: 0,
            stored: 0,
        }
    }

    fn begin(&mut self, outbox: async_mpsc::UnboundedSender<FromThread>, phase: Phase) {
        self.outbox = Some(outbox);
        self.phase = phase;
        self.notified = 0;
        self.progressed = 0;
        self.logged = 0;
        self.asked = 0;
        self.stored = 0;
    }

    /// The call has ended: its host calls that were not answered never
    /// will be.
    fn end(&mut self) {
        self.outbox = None;
        self.pending.clear();
        self.awaited = 0;
        self.pending_bytes = 0;
    }

    /// Keeps a host call's promise until its answer comes.
    fn wait<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        kind: &str,
        bytes: usize,
        resolve: Function<'js>,
        reject: Function<'js>,
    ) -> u64 {
        let id = self.next;
        self.next += 1;
        let awaited = !only_tells(kind);
        self.awaited += usize::from(awaited);
        self.pending_bytes += bytes;
        self.pending.insert(
            id,
            Waiting {
                resolve: Persistent::save(ctx, resolve),
                reject: Persistent::save(ctx, reject),
                bytes,
                awaited,
            },
        );
        id
    }

    /// Takes the host call `id` off the waiting ones.
    fn answered(&mut self, id: u64) -> Option<Waiting> {
        let waiting = self.pending.remove(&id)?;
        self.pending_bytes = self.pending_bytes.saturating_sub(waiting.bytes);
        self.awaited -= usize::from(waiting.awaited);
        Some(waiting)
    }

    /// Whether a host call of `kind` with arguments of `bytes` may be
    /// sent now, and where to. The memory a call's arguments take is the
    /// harness's, not the runtime's, so what waits is bounded here.
    fn admit(
        &mut self,
        kind: &str,
        bytes: usize,
    ) -> Result<async_mpsc::UnboundedSender<FromThread>, Refused> {
        if self.phase == Phase::Evaluation {
            return Err(Refused::Thrown(
                "message",
                "a host function cannot be called while the extension's main module is first \
                 evaluated: call it from a tool, a command or a hook"
                    .to_owned(),
            ));
        }
        let Some(outbox) = self.outbox.clone() else {
            return Err(Refused::Thrown(
                "message",
                "a host function can be called only while a call of the extension runs".to_owned(),
            ));
        };
        if (!only_tells(kind) && self.awaited >= MAX_PENDING)
            || self.pending_bytes + bytes > MAX_PENDING_BYTES
        {
            return Err(Refused::Rejected(format!(
                "too many host calls are waiting for their answers (at most {MAX_PENDING}, holding \
                 at most {} MiB): await them before making more",
                MAX_PENDING_BYTES / (1024 * 1024)
            )));
        }
        let (count, most, what) = match kind {
            "notify" => (&mut self.notified, MAX_NOTIFY, "notices"),
            "progress" => (&mut self.progressed, MAX_PROGRESS, "progress lines"),
            "log" => (&mut self.logged, MAX_LOG, "lines of the log"),
            "ask" => (&mut self.asked, MAX_ASK, "questions"),
            "state.set" => (&mut self.stored, MAX_STATE_SET, "`cineris.state.set` calls"),
            _ => return Ok(outbox),
        };
        if *count >= most {
            return Err(Refused::Rejected(format!(
                "a call of the extension gives at most {most} {what}"
            )));
        }
        *count += 1;
        Ok(outbox)
    }

    fn function(&self, target: &Target) -> Option<Persistent<Function<'static>>> {
        match target {
            Target::Tool(name) => self.tools.get(name).cloned(),
            Target::Command(name) => self.commands.get(name).cloned(),
            Target::Hook(index) => self.hooks.get(*index).cloned(),
        }
    }

    /// Frees every value kept across calls, while the runtime is still
    /// there.
    fn clear(&mut self) {
        self.tools.clear();
        self.commands.clear();
        self.hooks.clear();
        self.pending.clear();
        self.outbox = None;
    }
}

/// The two native entry points the prelude takes off the global object.
fn install<'js>(ctx: &Ctx<'js>, shared: &Rc<RefCell<Shared>>) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let for_host = Rc::clone(shared);
    globals.set(
        "__cineris_host",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, kind: String, payload: String| -> rquickjs::Result<Promise<'js>> {
                host_call(&ctx, &for_host, &kind, &payload)
            },
        )?,
    )?;
    let for_register = Rc::clone(shared);
    globals.set(
        "__cineris_register",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  kind: String,
                  def: Object<'js>,
                  function: Opt<Function<'js>>|
                  -> rquickjs::Result<()> {
                register(&ctx, &for_register, &kind, &def, function.0)
            },
        )?,
    )?;
    Ok(())
}

/// The most host calls of one runtime waiting for their answers.
const MAX_PENDING: usize = 16;
/// The most bytes of arguments those calls may hold together.
const MAX_PENDING_BYTES: usize = 12 * 1024 * 1024;
/// The most `notify` calls in one call of the extension.
const MAX_NOTIFY: u32 = 10;
/// The most `progress` calls in one call of the extension.
const MAX_PROGRESS: u32 = 100;
/// The most lines `log` writes in one call of the extension.
const MAX_LOG: u32 = 100;
/// The most questions one call of the extension puts.
const MAX_ASK: u32 = 5;
/// The most `cineris.state.set` calls in one call of the extension: each
/// `session` one is a line of the session's log, up to 64 KiB, so a script
/// that sets in a loop cannot grow the log without end.
const MAX_STATE_SET: u32 = 100;

/// The largest arguments of a host call of `kind`, as the JSON text the
/// prelude passes: room for what the call may carry, escaped.
fn payload_cap(kind: &str) -> usize {
    match kind {
        "write" | "exec" => 8 * 1024 * 1024,
        "fetch" => 4 * 1024 * 1024,
        "state.set" => 512 * 1024,
        _ => 64 * 1024,
    }
}

/// Why a host call was not sent, as a promise already rejected: told to
/// the script without stopping its code where it stands.
enum Refused {
    /// Thrown at once: a mistake in how the call was made.
    Thrown(&'static str, String),
    /// A rejected promise: the call was made well and is not served.
    Rejected(String),
}

/// A host call: checked for its shape and its caps here, sent to the call
/// the thread serves, and answered by a promise.
///
/// Nothing here holds `shared` while the script's own code can run: an
/// exception being made runs code of the script's (`Error.prepareStackTrace`),
/// and a call that came back in while `shared` was held would panic the
/// thread.
fn host_call<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<RefCell<Shared>>,
    kind: &str,
    payload: &str,
) -> rquickjs::Result<Promise<'js>> {
    let refused = (|| {
        if payload.len() > payload_cap(kind) {
            return Err(Refused::Thrown(
                "type",
                format!(
                    "the arguments of `{kind}` are {} bytes, more than the {} it takes",
                    payload.len(),
                    payload_cap(kind)
                ),
            ));
        }
        let payload: Json = serde_json::from_str(payload).map_err(|_| {
            Refused::Thrown(
                "type",
                "a host function's arguments must be JSON".to_owned(),
            )
        })?;
        parse_call(kind, &payload).map_err(|message| Refused::Thrown("type", message))
    })();
    let call = match refused {
        Ok(call) => call,
        Err(refusal) => return refuse(ctx, refusal),
    };
    let (promise, resolve, reject) = Promise::new(ctx)?;
    let sent = {
        let Ok(mut shared) = shared.try_borrow_mut() else {
            drop((resolve, reject));
            return refuse(ctx, Refused::Thrown("message", INSIDE_ANOTHER.to_owned()));
        };
        shared.admit(kind, payload.len()).map(|outbox| {
            (
                shared.wait(ctx, kind, payload.len(), resolve, reject),
                outbox,
            )
        })
    };
    match sent {
        Ok((id, outbox)) => {
            let _ = outbox.send(FromThread::Call { id, call });
            Ok(promise)
        }
        Err(refusal) => refuse(ctx, refusal),
    }
}

/// A refusal told to the script: thrown, or as a rejected promise.
fn refuse<'js>(ctx: &Ctx<'js>, refusal: Refused) -> rquickjs::Result<Promise<'js>> {
    match refusal {
        Refused::Thrown("type", message) => Err(Exception::throw_type(ctx, &message)),
        Refused::Thrown(_, message) => Err(Exception::throw_message(ctx, &message)),
        Refused::Rejected(message) => {
            let (promise, _resolve, reject) = Promise::new(ctx)?;
            let error = Exception::from_message(ctx.clone(), &message)?;
            reject.call::<_, ()>((error,))?;
            Ok(promise)
        }
    }
}

fn field<'a>(payload: &'a Json, key: &str) -> Option<&'a Json> {
    payload.get(key).filter(|value| !value.is_null())
}

fn text_field(payload: &Json, key: &str, what: &str) -> Result<String, String> {
    match field(payload, key) {
        Some(Json::String(text)) => Ok(text.clone()),
        _ => Err(format!("{what} must be text")),
    }
}

fn optional_text(payload: &Json, key: &str, what: &str) -> Result<Option<String>, String> {
    match field(payload, key) {
        None => Ok(None),
        Some(Json::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("{what} must be text")),
    }
}

fn optional_count(payload: &Json, key: &str, what: &str) -> Result<Option<u64>, String> {
    match field(payload, key) {
        None => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{what} must be a whole number, 0 or more")),
    }
}

fn texts(payload: &Json, key: &str, what: &str) -> Result<Vec<String>, String> {
    match field(payload, key) {
        None => Ok(Vec::new()),
        Some(Json::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{what} must be a list of text"))
            })
            .collect(),
        Some(_) => Err(format!("{what} must be a list of text")),
    }
}

fn scope(payload: &Json) -> Result<Scope, String> {
    match field(payload, "scope") {
        None => Ok(Scope::Session),
        Some(Json::String(text)) if text == "session" => Ok(Scope::Session),
        Some(Json::String(text)) if text == "project" => Ok(Scope::Project),
        Some(_) => Err("`scope` is \"session\" or \"project\"".to_owned()),
    }
}

/// Reads a host call's arguments as the prelude passed them.
fn parse_call(kind: &str, payload: &Json) -> Result<HostCall, String> {
    Ok(match kind {
        "exec" => HostCall::Exec {
            program: text_field(payload, "program", "the program")?,
            args: texts(payload, "args", "the arguments")?,
            cwd: optional_text(payload, "cwd", "`cwd`")?,
            stdin: optional_text(payload, "stdin", "`stdin`")?,
            timeout_ms: optional_count(payload, "timeout", "`timeout`")?,
        },
        "read" => HostCall::Read {
            path: text_field(payload, "path", "the path")?,
            offset: optional_count(payload, "offset", "`offset`")?.unwrap_or(0),
            limit: optional_count(payload, "limit", "`limit`")?,
        },
        "write" => HostCall::Write {
            path: text_field(payload, "path", "the path")?,
            text: text_field(payload, "text", "the text")?,
        },
        "list" => HostCall::List {
            path: text_field(payload, "path", "the path")?,
            glob: optional_text(payload, "glob", "`glob`")?,
        },
        "search" => HostCall::Search {
            pattern: text_field(payload, "pattern", "the pattern")?,
            path: optional_text(payload, "path", "`path`")?.unwrap_or_else(|| ".".to_owned()),
            glob: optional_text(payload, "glob", "`glob`")?,
        },
        "fetch" => HostCall::Fetch(FetchCall {
            url: text_field(payload, "url", "the URL")?,
            method: optional_text(payload, "method", "`method`")?
                .unwrap_or_else(|| "GET".to_owned()),
            headers: match field(payload, "headers") {
                None => Vec::new(),
                Some(Json::Array(pairs)) => pairs
                    .iter()
                    .map(|pair| match pair.as_array().map(Vec::as_slice) {
                        Some([Json::String(name), Json::String(value)]) => {
                            Ok((name.clone(), value.clone()))
                        }
                        _ => Err("`headers` must be names and texts".to_owned()),
                    })
                    .collect::<Result<_, _>>()?,
                Some(_) => return Err("`headers` must be names and texts".to_owned()),
            },
            body: optional_text(payload, "body", "`body`")?,
            timeout_ms: optional_count(payload, "timeout", "`timeout`")?,
        }),
        "ask" => HostCall::Ask {
            question: text_field(payload, "question", "the question")?,
            choices: texts(payload, "choices", "the choices")?,
        },
        "notify" | "progress" | "log" => {
            let text = payload
                .as_str()
                .ok_or_else(|| "the text must be text".to_owned())?
                .to_owned();
            match kind {
                "notify" => HostCall::Notify(text),
                "progress" => HostCall::Progress(text),
                _ => HostCall::Log(text),
            }
        }
        "state.get" => HostCall::StateGet {
            key: text_field(payload, "key", "the key")?,
            scope: scope(payload)?,
        },
        "state.set" => HostCall::StateSet {
            key: text_field(payload, "key", "the key")?,
            value: field(payload, "value").cloned(),
            scope: scope(payload)?,
        },
        other => return Err(format!("there is no host function `{other}`")),
    })
}

/// A list of texts from `def[key]`, at most `most` of them: its length is
/// read before any of it is taken into the harness's memory, which the
/// runtime's limit does not count (the review of step 28).
fn bounded_texts<'js>(
    ctx: &Ctx<'js>,
    def: &Object<'js>,
    key: &str,
    most: usize,
) -> rquickjs::Result<Vec<String>> {
    let value: Value<'js> = def.get(key)?;
    if value.is_undefined() || value.is_null() {
        return Ok(Vec::new());
    }
    let Some(array) = value.as_array() else {
        return Err(Exception::throw_type(
            ctx,
            &format!("`{key}` must be a list of text"),
        ));
    };
    if array.len() > most {
        return Err(Exception::throw_type(
            ctx,
            &format!(
                "`{key}` lists at most {most}, and this lists {}",
                array.len()
            ),
        ));
    }
    (0..array.len())
        .map(|index| {
            array.get::<Option<String>>(index)?.ok_or_else(|| {
                Exception::throw_type(ctx, &format!("`{key}` must be a list of text"))
            })
        })
        .collect()
}

/// What one call of `cineris.tool`, `cineris.command` or `cineris.on`
/// registers, read whole before anything of the thread's is held.
enum Entry<'js> {
    Tool(ToolDef, Function<'js>),
    Command(CommandDef, Function<'js>),
    Hook(HookDef, Event, Function<'js>),
}

/// `cineris.tool`, `cineris.command` and `cineris.on`.
///
/// Every field of the definition is read before `shared` is borrowed, and
/// every error is made after it is let go: reading a field runs the
/// script's code (a getter, `toJSON`), and so does making an error
/// (`Error.prepareStackTrace`), and that code may call back in. A call
/// that came back in while `shared` was held would panic the thread.
fn register<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<RefCell<Shared>>,
    kind: &str,
    def: &Object<'js>,
    function: Option<Function<'js>>,
) -> rquickjs::Result<()> {
    let late = "an extension registers its tools, commands and hooks only while its main module \
                is first evaluated";
    match shared.try_borrow().map(|shared| shared.phase) {
        Ok(Phase::Evaluation) => {}
        Ok(_) => return Err(Exception::throw_message(ctx, late)),
        Err(_) => return Err(Exception::throw_message(ctx, INSIDE_ANOTHER)),
    }
    let entry = read_entry(ctx, kind, def, function)?;
    let added = match shared.try_borrow_mut() {
        // The phase changes only between calls, never while a definition is
        // read: what was checked above holds.
        Ok(mut shared) => shared.add(ctx, entry),
        Err(_) => Err(INSIDE_ANOTHER.to_owned()),
    };
    cineris_base::hook::point(super::A_DEFINITION_IS_REGISTERED);
    added.map_err(|message| Exception::throw_type(ctx, &message))
}

/// What a native function says when it is called from inside another.
const INSIDE_ANOTHER: &str = "a host function cannot be called from inside another";

/// Reads a definition's fields, holding nothing.
fn read_entry<'js>(
    ctx: &Ctx<'js>,
    kind: &str,
    def: &Object<'js>,
    function: Option<Function<'js>>,
) -> rquickjs::Result<Entry<'js>> {
    let refuse = |message: String| Exception::throw_type(ctx, &message);
    let text = |key: &str| -> rquickjs::Result<String> {
        def.get::<_, Option<String>>(key)?
            .ok_or_else(|| Exception::throw_type(ctx, &format!("`{key}` must be text")))
    };
    match kind {
        "tool" => {
            let name = text("name")?;
            check_name(&name).map_err(refuse)?;
            let description = def
                .get::<_, Option<String>>("description")?
                .unwrap_or_default();
            check_description(&description).map_err(refuse)?;
            let input: Value<'js> = def.get("input")?;
            let input = if input.is_undefined() || input.is_null() {
                serde_json::json!({"type": "object", "properties": {}, "additionalProperties": false})
            } else {
                let schema = from_js(ctx, input).map_err(|failure| {
                    refuse(format!("`input` must be a JSON Schema: {failure}"))
                })?;
                check_schema(&schema).map_err(refuse)?;
                schema
            };
            let read_only = def.get::<_, Option<bool>>("readOnly")?.unwrap_or(false);
            let run: Option<Function<'js>> = def.get("run")?;
            let run = run.ok_or_else(|| refuse("a tool needs `run`, a function".to_owned()))?;
            Ok(Entry::Tool(
                ToolDef {
                    name,
                    description,
                    input,
                    read_only,
                },
                run,
            ))
        }
        "command" => {
            let name = text("name")?;
            check_name(&name).map_err(refuse)?;
            let description = def
                .get::<_, Option<String>>("description")?
                .unwrap_or_default();
            check_description(&description).map_err(refuse)?;
            let arguments = bounded_texts(ctx, def, "arguments", MAX_ARGUMENTS)?;
            for argument in &arguments {
                check_name(argument).map_err(refuse)?;
            }
            let run: Option<Function<'js>> = def.get("run")?;
            let run = run.ok_or_else(|| refuse("a command needs `run`, a function".to_owned()))?;
            Ok(Entry::Command(
                CommandDef {
                    name,
                    description,
                    arguments,
                },
                run,
            ))
        }
        "hook" => {
            let name = text("event")?;
            let Some(event) = Event::parse(&name) else {
                return Err(refuse(format!(
                    "`{name}` is not an event: {}",
                    Event::ALL.map(Event::as_str).join(", ")
                )));
            };
            let tools = bounded_texts(ctx, def, "tools", MAX_HOOK_TOOLS)?;
            for tool in &tools {
                check_filter(tool).map_err(refuse)?;
            }
            let observe = def.get::<_, Option<bool>>("observe")?.unwrap_or(false);
            let timeout_ms = def.get::<_, Option<u64>>("timeout")?;
            check_timeout(timeout_ms).map_err(refuse)?;
            let function =
                function.ok_or_else(|| refuse("a hook needs a function to run".to_owned()))?;
            Ok(Entry::Hook(
                HookDef {
                    event: name,
                    tools,
                    observe,
                    timeout_ms,
                },
                event,
                function,
            ))
        }
        other => Err(refuse(format!("`{other}` cannot be registered"))),
    }
}

impl Shared {
    /// Adds what a definition registers. Runs none of the script's code.
    fn add<'js>(&mut self, ctx: &Ctx<'js>, entry: Entry<'js>) -> Result<(), String> {
        match entry {
            Entry::Tool(definition, run) => {
                if self.tools.contains_key(&definition.name) {
                    return Err(format!(
                        "a tool `{}` is registered already",
                        definition.name
                    ));
                }
                if self.registration.tools.len() >= MAX_TOOLS {
                    return Err(format!("an extension registers at most {MAX_TOOLS} tools"));
                }
                self.tools
                    .insert(definition.name.clone(), Persistent::save(ctx, run));
                self.registration.tools.push(definition);
            }
            Entry::Command(definition, run) => {
                if self.commands.contains_key(&definition.name) {
                    return Err(format!(
                        "a command `{}` is registered already",
                        definition.name
                    ));
                }
                if self.registration.commands.len() >= MAX_COMMANDS {
                    return Err(format!(
                        "an extension registers at most {MAX_COMMANDS} commands"
                    ));
                }
                self.commands
                    .insert(definition.name.clone(), Persistent::save(ctx, run));
                self.registration.commands.push(definition);
            }
            Entry::Hook(definition, event, function) => {
                check_allowed(&definition.event, event, &self.hooks_allowed)?;
                if self.registration.hooks.len() >= MAX_HOOKS {
                    return Err(format!("an extension registers at most {MAX_HOOKS} hooks"));
                }
                self.hooks.push(Persistent::save(ctx, function));
                self.registration.hooks.push(definition);
            }
        }
        Ok(())
    }
}
