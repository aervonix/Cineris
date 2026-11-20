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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
use crate::host::{HostAnswer, HostCall, Phase, Scope};
use crate::manifest::Manifest;
use crate::registration::{
    CommandDef, HookDef, MAX_COMMANDS, MAX_DESCRIPTION, MAX_HOOKS, MAX_SCHEMA_BYTES, MAX_TOOLS,
    Registration, ToolDef,
};

/// How many runtimes this process has made: the start-up test holds it at
/// zero.
pub(crate) static RUNTIMES_MADE: AtomicUsize = AtomicUsize::new(0);

/// The largest result a call gives, as JSON.
pub(crate) const MAX_RESULT_BYTES: usize = 1024 * 1024;
/// The longest description of a thrown error.
const MAX_THROWN: usize = 4096;
/// The longest a hook's time may be set, in milliseconds: an observer's.
const MAX_HOOK_TIME_MS: u64 = 600_000;

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
}

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
    // Everything kept across calls goes before the runtime does.
    shared.borrow_mut().clear();
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
        } = invocation;
        if self.stop.load(Ordering::SeqCst) {
            self.alive.store(false, Ordering::SeqCst);
            let _ = outbox.send(FromThread::Done(Err(Failure::Limit(Limit::Time))));
            return After::TornDown;
        }
        let _ = outbox.send(FromThread::Started);
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
        let Some((resolve, reject)) = self.shared.borrow_mut().pending.remove(&id) else {
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
    pending: HashMap<u64, (Persistent<Function<'static>>, Persistent<Function<'static>>)>,
    next: u64,
    outbox: Option<async_mpsc::UnboundedSender<FromThread>>,
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
            next: 0,
            outbox: None,
        }
    }

    fn begin(&mut self, outbox: async_mpsc::UnboundedSender<FromThread>, phase: Phase) {
        self.outbox = Some(outbox);
        self.phase = phase;
    }

    /// The call has ended: its host calls that were not answered never
    /// will be.
    fn end(&mut self) {
        self.outbox = None;
        self.pending.clear();
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

/// A host call: checked for its shape here, sent to the call the thread
/// serves, and answered by a promise.
fn host_call<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<RefCell<Shared>>,
    kind: &str,
    payload: &str,
) -> rquickjs::Result<Promise<'js>> {
    let payload: Json = serde_json::from_str(payload)
        .map_err(|_| Exception::throw_type(ctx, "a host function's arguments must be JSON"))?;
    let call =
        parse_call(kind, &payload).map_err(|message| Exception::throw_type(ctx, &message))?;
    let mut shared = shared.borrow_mut();
    if shared.phase == Phase::Evaluation {
        return Err(Exception::throw_message(
            ctx,
            "a host function cannot be called while the extension's main module is first \
             evaluated: call it from a tool, a command or a hook",
        ));
    }
    let Some(outbox) = shared.outbox.clone() else {
        return Err(Exception::throw_message(
            ctx,
            "a host function can be called only while a call of the extension runs",
        ));
    };
    let (promise, resolve, reject) = Promise::new(ctx)?;
    let id = shared.next;
    shared.next += 1;
    shared.pending.insert(
        id,
        (
            Persistent::save(ctx, resolve),
            Persistent::save(ctx, reject),
        ),
    );
    let _ = outbox.send(FromThread::Call { id, call });
    Ok(promise)
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

/// A name of a tool or a command: what the registry takes.
fn check_name(name: &str) -> Result<(), String> {
    let fine = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        });
    if fine {
        Ok(())
    } else {
        Err(format!(
            "`{name}` cannot be a name: 1 to 64 of ASCII letters, digits, `_`, `-` and `.`"
        ))
    }
}

fn check_description(description: &str) -> Result<(), String> {
    if description.len() > MAX_DESCRIPTION {
        Err("a description is at most 2 KiB".to_owned())
    } else {
        Ok(())
    }
}

/// `cineris.tool`, `cineris.command` and `cineris.on`.
fn register<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<RefCell<Shared>>,
    kind: &str,
    def: &Object<'js>,
    function: Option<Function<'js>>,
) -> rquickjs::Result<()> {
    let refuse = |message: String| Exception::throw_type(ctx, &message);
    let mut shared = shared.borrow_mut();
    if shared.phase != Phase::Evaluation {
        return Err(Exception::throw_message(
            ctx,
            "an extension registers its tools, commands and hooks only while its main module is \
             first evaluated",
        ));
    }
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
                if !schema.is_object() || schema.to_string().len() > MAX_SCHEMA_BYTES {
                    return Err(refuse(
                        "`input` must be a JSON Schema object of at most 16 KiB".to_owned(),
                    ));
                }
                schema
            };
            let read_only = def.get::<_, Option<bool>>("readOnly")?.unwrap_or(false);
            let run: Option<Function<'js>> = def.get("run")?;
            let run = run.ok_or_else(|| refuse("a tool needs `run`, a function".to_owned()))?;
            if shared.tools.contains_key(&name) {
                return Err(refuse(format!("a tool `{name}` is registered already")));
            }
            if shared.registration.tools.len() >= MAX_TOOLS {
                return Err(refuse(format!(
                    "an extension registers at most {MAX_TOOLS} tools"
                )));
            }
            shared
                .tools
                .insert(name.clone(), Persistent::save(ctx, run));
            shared.registration.tools.push(ToolDef {
                name,
                description,
                input,
                read_only,
            });
        }
        "command" => {
            let name = text("name")?;
            check_name(&name).map_err(refuse)?;
            let description = def
                .get::<_, Option<String>>("description")?
                .unwrap_or_default();
            check_description(&description).map_err(refuse)?;
            let arguments = def
                .get::<_, Option<Vec<String>>>("arguments")?
                .unwrap_or_default();
            for argument in &arguments {
                check_name(argument).map_err(refuse)?;
            }
            let run: Option<Function<'js>> = def.get("run")?;
            let run = run.ok_or_else(|| refuse("a command needs `run`, a function".to_owned()))?;
            if shared.commands.contains_key(&name) {
                return Err(refuse(format!("a command `{name}` is registered already")));
            }
            if shared.registration.commands.len() >= MAX_COMMANDS {
                return Err(refuse(format!(
                    "an extension registers at most {MAX_COMMANDS} commands"
                )));
            }
            shared
                .commands
                .insert(name.clone(), Persistent::save(ctx, run));
            shared.registration.commands.push(CommandDef {
                name,
                description,
                arguments,
            });
        }
        "hook" => {
            let name = text("event")?;
            let Some(event) = Event::parse(&name) else {
                return Err(refuse(format!(
                    "`{name}` is not an event: {}",
                    Event::ALL.map(Event::as_str).join(", ")
                )));
            };
            if !shared.hooks_allowed.contains(&event) {
                return Err(refuse(format!(
                    "the extension's manifest does not allow a hook on `{name}`, which lets it see {} \
                     (`capabilities.hooks`)",
                    event.sees()
                )));
            }
            let tools = def
                .get::<_, Option<Vec<String>>>("tools")?
                .unwrap_or_default();
            let observe = def.get::<_, Option<bool>>("observe")?.unwrap_or(false);
            let timeout_ms = def.get::<_, Option<u64>>("timeout")?;
            if timeout_ms.is_some_and(|time| time == 0 || time > MAX_HOOK_TIME_MS) {
                return Err(refuse(
                    "a hook's `timeout` is from 1 to 600000 (milliseconds)".to_owned(),
                ));
            }
            let function =
                function.ok_or_else(|| refuse("a hook needs a function to run".to_owned()))?;
            if shared.registration.hooks.len() >= MAX_HOOKS {
                return Err(refuse(format!(
                    "an extension registers at most {MAX_HOOKS} hooks"
                )));
            }
            shared.hooks.push(Persistent::save(ctx, function));
            shared.registration.hooks.push(HookDef {
                event: name,
                tools,
                observe,
                timeout_ms,
            });
        }
        other => return Err(refuse(format!("`{other}` cannot be registered"))),
    }
    Ok(())
}
