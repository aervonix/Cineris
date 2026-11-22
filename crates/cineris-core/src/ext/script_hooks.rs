//! An extension's hooks: the functions a loaded extension's script
//! registered with `cineris.on`, on the loop's seam beside the hook
//! commands, and on the session's own two events
//! (`docs/steps/28/answered.md`, points 4, 6 and 8;
//! `docs/architecture/12-extensions.md`, "Tier 3: hooks").
//!
//! - **Each hook a loaded extension registered,** on an event its
//!   manifest's `capabilities.hooks` lists (as the registration cache keeps
//!   what it registered), is one of the session's hooks from when the
//!   extension is loaded until it is withdrawn: an extension's hooks are its
//!   own, on their events only while it is loaded as it was approved. A
//!   project's extension is loaded only while the project is trusted and the
//!   extension is pinned as it is.
//! - **Its name** is the extension's, then `hook.` and the event
//!   (`tracker/hook.tool.before`), with a number from the second of the
//!   extension's hooks on one event on (`tracker/hook.tool.before.2`): each
//!   part a name by the registry's rules (`registration::check_name`). Its
//!   host calls are decided under it, so a rule about the extension
//!   (`deny ext tracker/*`) reaches them.
//! - **Its run is a call of the extension's runtime**, one of the runtime's
//!   own queue: the function is given the event's object
//!   (`HookInput::to_json`), and what it returns is the answer. `undefined`
//!   or `null` is no opinion; anything else, at most 64 KiB of JSON, is read
//!   by its event's contract: the loop's reading for the loop's five events,
//!   `cineris_ext::hook::Answer::of` for `session.start`.
//! - **Its time starts when it is asked** (`CallContext::asked`): its wait
//!   behind the runtime's other calls counts. When the time is up, a call
//!   still waiting is withdrawn before it starts, and one running is
//!   stopped as any call past its time is.
//! - **Its run is a task the session keeps,** under the session's token:
//!   the engine's stop of a call past its time, with its grace for the
//!   thread, is made whatever became of the loop's wait, and no run outlives
//!   its session. **Dropped, its future stops it,** polled or not: a call not
//!   started is withdrawn, and one running is stopped.
//! - **A stopped turn's hooks do not start,** as a hook command's do not:
//!   one asked after its turn stopped has failed, and is no stop of its own.
//!   A host call of one started before the stop is refused by the policy's
//!   stop (`CallHost::decide`). A stop that the hook's own program's look at
//!   the protected paths makes is told at once and holds back the next
//!   prompt, as a hook command's is (`HookCommands::stopped`).
//! - **Its host calls are a hook's** (`CallHost`, `Phase::Hook`): none puts
//!   a question, so a call the engine would ask about is refused, and none
//!   asks the user or writes.
//! - **A gate does not lapse** (point 5): an extension that stands approved
//!   and is not found as it was approved (changed, or unreadable as it is;
//!   withdrawn by a turn's look, or so as the session opens) has its gating
//!   hooks (`prompt.submit`, `tool.before`, not observing) held on their
//!   events from the registration its pin names, each a hook whose run
//!   fails with why and what approves or turns it off ([`Gate`]). Its other
//!   hooks leave. One approved again joins at the next turn's start, in
//!   their place. A runtime that cannot start (its files changed since the
//!   turn's look, a thread of it left running) fails its hooks' runs the
//!   same way, with what approves or turns it off.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cineris_agent::hooks::{OBSERVE_DEFAULT, OBSERVE_MAX, WAIT_DEFAULT, WAIT_MAX};
use cineris_agent::{
    Hook, HookEvent as LoopEvent, HookInput, HookOutcome as Heard, Policy as _, ToolTarget,
};
use cineris_base::{CancelToken, PathResolver, in_code, visible};
use cineris_ext::event::Event;
use cineris_ext::hook::Filter;
use cineris_ext::hook::answer::MAX_ANSWER;
use cineris_protocol::ids::{CallId, ItemId};
use cineris_protocol::{SessionId, TurnId};
use cineris_tools::{BoxFuture, CallRef, FileTable, Json, RunCx};

use super::hook_command::{
    HookCommands, TURN_END_AT_ONCE, session_input, session_notice, turn_number,
};
use super::hook_points::loop_event;
use super::tools::{HookRan, LoadedExtension};

/// The hooks of a session's loaded extensions.
pub(crate) struct ScriptHooks {
    /// What keeps their runs, tells a stop, and holds the session's token.
    commands: Arc<HookCommands>,
    /// The session's path resolver.
    paths: Arc<PathResolver>,
    /// Every hook of every extension loaded now.
    on: Mutex<Vec<Arc<ScriptHook>>>,
    /// The gating hooks of the extensions that stand approved and are not
    /// found as approved: each fails.
    held: Mutex<Vec<Arc<HeldHook>>>,
}

impl std::fmt::Debug for ScriptHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptHooks")
            .field("hooks", &lock(&self.on).len())
            .field("held", &lock(&self.held).len())
            .finish_non_exhaustive()
    }
}

/// An extension that stands approved at `hash` and is not found as it was
/// approved: its gating hooks are held, failing with `why`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Gate {
    /// The extension's name.
    pub(crate) name: String,
    /// The hash its pin names.
    pub(crate) hash: cineris_base::ContentHash,
    /// Why its hooks cannot run, and what approves or turns it off: the
    /// failure each of them gives.
    pub(crate) why: String,
    /// What it registered at that hash, as the registration cache keeps it:
    /// `None` when the cache does not, and the session's own knowledge of
    /// it, from when it was loaded at that hash, serves.
    pub(crate) hooks: Option<Vec<cineris_ext::registration::HookDef>>,
}

/// Each hook of `definitions` on an event `allowed` lists (`None`: any), with
/// its place among them and its name: the extension's, `hook.` and the
/// event, and a number from the second on one event on.
fn named<'a>(
    extension: &str,
    definitions: &'a [cineris_ext::registration::HookDef],
    allowed: Option<&[Event]>,
) -> Vec<(usize, String, Event, &'a cineris_ext::registration::HookDef)> {
    let mut counts: HashMap<Event, usize> = HashMap::new();
    let mut each = Vec::new();
    for (index, definition) in definitions.iter().enumerate() {
        let Some(event) = Event::parse(&definition.event)
            .filter(|event| allowed.is_none_or(|allowed| allowed.contains(event)))
        else {
            continue;
        };
        let count = counts.entry(event).or_insert(0);
        *count += 1;
        let name = if *count == 1 {
            format!("{extension}/hook.{}", event.as_str())
        } else {
            format!("{extension}/hook.{}.{count}", event.as_str())
        };
        each.push((index, name, event, definition));
    }
    each
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How a session's `session.start` hooks of its extensions are heard: the
/// session they are of, and how a hook's context reaches it, as a notice
/// labelled as the hook's.
pub(crate) struct SessionStart {
    /// The session.
    pub(crate) session: SessionId,
    /// Gives the session a notice of the source `hook`.
    pub(crate) notice: Arc<dyn Fn(String) + Send + Sync>,
}

impl std::fmt::Debug for SessionStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionStart")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl ScriptHooks {
    /// A session's: none until an extension is loaded.
    pub(crate) fn new(commands: Arc<HookCommands>, paths: Arc<PathResolver>) -> Self {
        Self {
            commands,
            paths,
            on: Mutex::new(Vec::new()),
            held: Mutex::new(Vec::new()),
        }
    }

    /// Puts what `extension` registered on its events: each hook on an
    /// event its manifest lists, in the order it registered them, in place
    /// of any of an extension of its name.
    pub(crate) fn add(&self, extension: &Arc<LoadedExtension>) {
        let approved = extension.approved();
        let allowed = &approved.manifest.capabilities.hooks;
        let mut added = Vec::new();
        // What a live runtime and the cache both checked: events the
        // manifest lists.
        for (index, name, event, definition) in named(
            extension.name(),
            &approved.registration.hooks,
            Some(allowed),
        ) {
            added.push(Arc::new(ScriptHook {
                name,
                event,
                filter: Filter::new(definition.tools.clone()),
                observe: definition.observe,
                timeout: definition.timeout_ms.map(Duration::from_millis),
                index,
                extension: Arc::clone(extension),
                commands: Arc::clone(&self.commands),
                paths: Arc::clone(&self.paths),
                running: Arc::default(),
            }));
        }
        let mut on = lock(&self.on);
        on.retain(|hook| hook.extension.name() != extension.name());
        // What was held of it, while it did not stand approved, left as the
        // session's finding settled before loading it (`hold`).
        on.extend(added);
    }

    /// Holds the gates of `gates`, in place of those held before: each
    /// one's gating hooks (`prompt.submit`, `tool.before`, not observing)
    /// on their events as hooks that fail with its `why`, made from what it
    /// registered at the hash approved (the cache's, or, when the cache does
    /// not keep it, the session's own from when it was loaded or held at
    /// that hash). A gate whose registration is known nowhere holds
    /// nothing.
    pub(crate) fn hold(&self, gates: Vec<Gate>) {
        let on = lock(&self.on);
        let mut held = lock(&self.held);
        let mut next: Vec<Arc<HeldHook>> = Vec::new();
        for gate in gates {
            let made: Vec<HeldHook> = match &gate.hooks {
                Some(definitions) => named(&gate.name, definitions, None)
                    .into_iter()
                    .filter(|(_, _, event, definition)| event.gates() && !definition.observe)
                    .filter_map(|(_, name, event, definition)| {
                        Some(HeldHook {
                            name,
                            extension: gate.name.clone(),
                            hash: gate.hash,
                            event: loop_event(event)?,
                            filter: Filter::new(definition.tools.clone()),
                            why: gate.why.clone(),
                        })
                    })
                    .collect(),
                None => on
                    .iter()
                    .filter(|hook| {
                        hook.extension.name() == gate.name
                            && hook.extension.approved().hash == gate.hash
                            && hook.event.gates()
                            && !hook.observe
                    })
                    .filter_map(|hook| {
                        Some(HeldHook {
                            name: hook.name.clone(),
                            extension: gate.name.clone(),
                            hash: gate.hash,
                            event: loop_event(hook.event)?,
                            filter: hook.filter.clone(),
                            why: gate.why.clone(),
                        })
                    })
                    .chain(
                        held.iter()
                            .filter(|hook| hook.extension == gate.name && hook.hash == gate.hash)
                            .map(|hook| HeldHook {
                                why: gate.why.clone(),
                                ..HeldHook::clone(hook)
                            }),
                    )
                    .collect(),
            };
            next.extend(made.into_iter().map(Arc::new));
        }
        *held = next;
    }

    /// Takes the hooks of the extension `name` off their events: it is
    /// withdrawn from the session.
    pub(crate) fn withdraw(&self, name: &str) {
        lock(&self.on).retain(|hook| hook.extension.name() != name);
    }

    /// Whether a hook of an extension is on `event` for `target`: cheap, no
    /// allocation.
    pub(crate) fn wants(&self, event: LoopEvent, target: Option<ToolTarget<'_>>) -> bool {
        lock(&self.on).iter().any(|hook| hook.is_on(event, target))
            || lock(&self.held)
                .iter()
                .any(|hook| hook.event == event && on_target(&hook.filter, target))
    }

    /// The hooks of extensions on `event` for `target`.
    pub(crate) fn matching(
        &self,
        event: LoopEvent,
        target: Option<ToolTarget<'_>>,
    ) -> Vec<Arc<dyn Hook>> {
        let mut matched: Vec<Arc<dyn Hook>> = lock(&self.on)
            .iter()
            .filter(|hook| hook.is_on(event, target))
            .map(|hook| Arc::clone(hook) as Arc<dyn Hook>)
            .collect();
        matched.extend(
            lock(&self.held)
                .iter()
                .filter(|hook| hook.event == event && on_target(&hook.filter, target))
                .map(|hook| Arc::clone(hook) as Arc<dyn Hook>),
        );
        matched
    }

    /// The hooks of extensions on one of the session's own events.
    fn of_session(&self, event: Event) -> Vec<Arc<ScriptHook>> {
        lock(&self.on)
            .iter()
            .filter(|hook| hook.event == event)
            .map(Arc::clone)
            .collect()
    }

    /// Runs the `session.start` hooks of the extensions the session loaded
    /// as it opened: the observers beside it, never waited for, a failure
    /// logged; the rest together, each within its own time, each one's
    /// context a notice before the first prompt (`started.notice`) and a
    /// failure told to the user (`tell`). What the first turn waits for.
    pub(crate) async fn session_started(
        &self,
        started: &SessionStart,
        tell: &(dyn Fn(String) + Send + Sync),
    ) {
        let hooks = self.of_session(Event::SessionStart);
        if hooks.is_empty() {
            return;
        }
        let input = session_input(Event::SessionStart, &started.session, &self.paths);
        // No turn of its own, as a hook command of the event has none.
        let turn = TurnId::new("t_0");
        let mut waited = Vec::new();
        for hook in hooks {
            let running = hook.start(
                input.clone(),
                &started.session,
                &turn,
                self.commands.runs_token(),
            );
            if hook.observe {
                // Never waited for, its answer not read: a failure is
                // logged, as an observer's is everywhere. Under the
                // session's token, which the close fires.
                tokio::spawn(async move {
                    if let HookRan::Failed(why) | HookRan::Stopped(why) = running.ended().await {
                        tracing::warn!(hook = %hook.name, %why, "a `session.start` observer failed");
                    }
                });
            } else {
                waited.push((hook, running));
            }
        }
        let timer = Arc::clone(&self.commands.parts().timer);
        let said = futures_together(waited.into_iter().map(|(hook, running)| {
            let timer = Arc::clone(&timer);
            async move {
                let time = hook.time();
                let ran = within(&timer, time, running.ended())
                    .await
                    .unwrap_or_else(|| {
                        HookRan::Failed(format!(
                            "it did not answer within {} s",
                            time.as_secs_f64()
                        ))
                    });
                (hook, ran)
            }
        }))
        .await;
        for (hook, ran) in said {
            let name = in_code(&hook.name).into_owned();
            let heard = match ran {
                HookRan::Answered(answer) => match bounded(&hook.name, answer) {
                    Ok(answer) => cineris_ext::hook::Answer::of(Event::SessionStart, Some(answer))
                        .map_err(|why| format!("The hook `{name}` failed: its answer {why}.")),
                    Err(why) => Err(why),
                },
                HookRan::Stopped(note) => Err(note),
                HookRan::Failed(why) => Err(failed(&hook.name, &why)),
            };
            match heard {
                Ok(answer) => {
                    if let Some(context) = answer.context.filter(|text| !text.is_empty()) {
                        (started.notice)(session_notice(&hook.name, &context));
                    }
                }
                Err(why) => tell(visible(&why).into_owned()),
            }
        }
    }

    /// Starts the `session.end` hooks of the session's extensions, as it
    /// closes: each observed, its answer not read, under a token of its own
    /// (the session's is fired by then). Each one's name and its run, which
    /// the close gives its time, stops, and waits for.
    pub(crate) fn session_ending(&self, session: &SessionId) -> Vec<(String, Running)> {
        let input = session_input(Event::SessionEnd, session, &self.paths);
        let turn = TurnId::new("t_0");
        self.of_session(Event::SessionEnd)
            .into_iter()
            .map(|hook| {
                let running = hook.start(input.clone(), session, &turn, CancelToken::new());
                (hook.name.clone(), running)
            })
            .collect()
    }
}

/// Runs every one of `all` together on this task, and gives their outputs.
async fn futures_together<T, F: std::future::Future<Output = T>>(
    all: impl IntoIterator<Item = F>,
) -> Vec<T> {
    let mut slots: Vec<(std::pin::Pin<Box<F>>, Option<T>)> = all
        .into_iter()
        .map(|future| (Box::pin(future), None))
        .collect();
    std::future::poll_fn(|context| {
        let mut pending = false;
        for (future, done) in &mut slots {
            if done.is_none() {
                match future.as_mut().poll(context) {
                    std::task::Poll::Ready(output) => *done = Some(output),
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    slots.into_iter().filter_map(|(_, done)| done).collect()
}

/// `work`, unless `limit` on `timer` is up first: `None` then.
async fn within<T>(
    timer: &cineris_base::SharedTimer,
    limit: Duration,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    let mut work = std::pin::pin!(work);
    let mut over = timer.sleep(limit);
    std::future::poll_fn(|context| {
        if let std::task::Poll::Ready(done) = work.as_mut().poll(context) {
            return std::task::Poll::Ready(Some(done));
        }
        if over.as_mut().poll(context).is_ready() {
            return std::task::Poll::Ready(None);
        }
        std::task::Poll::Pending
    })
    .await
}

/// A failure of the hook `name` for the user and the model: `why` a clause,
/// or a sentence of its own.
fn failed(name: &str, why: &str) -> String {
    let why = why.trim_end_matches('.');
    // The harness's own sentence (the extension could not start), not what
    // a script threw (`Error: …`).
    if why.starts_with("The ") {
        format!("The hook `{}` failed. {why}.", in_code(name))
    } else {
        format!("The hook `{}` failed: {why}.", in_code(name))
    }
}

/// One hook an extension registered.
pub(crate) struct ScriptHook {
    /// Its name: the extension's, `hook.` and its event, and a number from
    /// the second on one event on.
    name: String,
    event: Event,
    /// The tools it is on, for a tool event; why not, when its entries
    /// cannot be read, and then it is on every call and fails (closed).
    filter: Result<Filter, String>,
    observe: bool,
    timeout: Option<Duration>,
    /// Its place in what the extension registered: what the runtime calls.
    index: usize,
    extension: Arc<LoadedExtension>,
    commands: Arc<HookCommands>,
    paths: Arc<PathResolver>,
    /// How many runs of it are going on: a waited-for `turn.end` hook has
    /// at most [`TURN_END_AT_ONCE`], as a hook command has.
    running: Arc<AtomicUsize>,
}

/// Whether a hook on `filter` is on `target`: its `tools` that cannot be
/// read put it on every call, failing closed.
fn on_target(filter: &Result<Filter, String>, target: Option<ToolTarget<'_>>) -> bool {
    match (filter, target) {
        (Ok(filter), Some(target)) => filter.matches(target.tool, target.names),
        (Err(_), Some(_)) | (_, None) => true,
    }
}

/// A gating hook of an extension that stands approved and is not found as
/// approved, held on its event: its run fails, with why and what approves
/// or turns it off.
#[derive(Debug, Clone)]
struct HeldHook {
    name: String,
    /// Its extension's name.
    extension: String,
    /// The hash its extension was approved at.
    hash: cineris_base::ContentHash,
    event: LoopEvent,
    filter: Result<Filter, String>,
    why: String,
}

impl Hook for HeldHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn observes(&self) -> bool {
        false
    }

    fn timeout(&self) -> Option<Duration> {
        None
    }

    fn run<'a>(&'a self, _input: &'a HookInput) -> BoxFuture<'a, Heard> {
        Box::pin(std::future::ready(Heard::Failed(self.why.clone())))
    }
}

impl ScriptHook {
    /// Whether it is on `event` for `target`.
    fn is_on(&self, event: LoopEvent, target: Option<ToolTarget<'_>>) -> bool {
        loop_event(self.event) == Some(event) && on_target(&self.filter, target)
    }

    /// The time it is given, as the loop gives it: its own, or the default
    /// of a hook waited for or of an observer, within their most.
    fn time(&self) -> Duration {
        if self.observe {
            self.timeout
                .map_or(OBSERVE_DEFAULT, |time| time.min(OBSERVE_MAX))
        } else {
            self.timeout.map_or(WAIT_DEFAULT, |time| time.min(WAIT_MAX))
        }
    }

    /// Starts a run of it with `input`, of the turn `turn`, under `token`:
    /// a task the session keeps, its time from now. The stop is made
    /// before the task is, so that the run let go before it is first
    /// polled is stopped too.
    fn start(
        &self,
        input: Json,
        session: &SessionId,
        turn: &TurnId,
        token: CancelToken,
    ) -> Running {
        let (sender, outcome) = tokio::sync::oneshot::channel();
        let kill = CancelToken::new();
        let stop = StopOnDrop(token.clone(), kill.clone());
        if let Err(why) = &self.filter {
            let _ = sender.send(HookRan::Failed(format!(
                "its `tools` cannot be read, so it is on every call: {why}"
            )));
            return Running { outcome, stop };
        }
        // A waited-for `turn.end` hook runs at most so many times at once:
        // turns that end faster than it runs leave the rest unstarted.
        if self.event == Event::TurnEnd
            && !self.observe
            && self.running.load(Ordering::Acquire) >= TURN_END_AT_ONCE
        {
            let _ = sender.send(HookRan::Failed(format!(
                "it was not started: {TURN_END_AT_ONCE} runs of it are going on"
            )));
            return Running { outcome, stop };
        }
        let counted = Counted::new(&self.running);
        let asked = self.extension.parts().clock.monotonic();
        let time = self.time();
        let call = Call {
            extension: Arc::clone(&self.extension),
            commands: Arc::clone(&self.commands),
            paths: Arc::clone(&self.paths),
            name: self.name.clone(),
            index: self.index,
            session: session.clone(),
            turn: turn.clone(),
            cancel: token,
            kill,
        };
        let task = tokio::spawn(async move {
            let _counted = counted;
            let ran = call.run(input, (asked, time)).await;
            let _ = sender.send(ran);
        });
        self.commands.keep(task);
        Running { outcome, stop }
    }
}

impl Hook for ScriptHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn observes(&self) -> bool {
        self.observe
    }

    fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    fn run<'a>(&'a self, input: &'a HookInput) -> BoxFuture<'a, Heard> {
        let running = self.start(
            input.to_json(),
            &input.session,
            &input.turn,
            self.commands.runs_token(),
        );
        Box::pin(async move { heard(&self.name, running.ended().await) })
    }
}

/// What the loop hears of a run: `undefined` and `null` are no opinion, and
/// any other value is the answer, at most [`MAX_ANSWER`] bytes of JSON.
fn heard(name: &str, ran: HookRan) -> Heard {
    match ran {
        HookRan::Answered(Json::Null) => Heard::Nothing,
        HookRan::Answered(value) => match bounded(name, value) {
            Ok(value) => Heard::Answer(value),
            Err(why) => Heard::Failed(why),
        },
        HookRan::Stopped(note) => Heard::Failed(note),
        HookRan::Failed(why) => Heard::Failed(failed(name, &why)),
    }
}

/// The hook `name`'s answer, when it is at most [`MAX_ANSWER`] bytes of
/// JSON, as a hook command's is; its failure otherwise.
fn bounded(name: &str, value: Json) -> Result<Json, String> {
    let size = serde_json::to_vec(&value).map_or(usize::MAX, |bytes| bytes.len());
    if size > MAX_ANSWER {
        Err(failed(
            name,
            &format!(
                "its answer is {size} bytes, and an answer is at most {} KiB",
                MAX_ANSWER / 1024
            ),
        ))
    } else {
        Ok(value)
    }
}

/// A run of an extension's hook, started.
pub(crate) struct Running {
    outcome: tokio::sync::oneshot::Receiver<HookRan>,
    /// Dropped, it stops the run.
    stop: StopOnDrop,
}

impl Running {
    /// What stops the run while its end is awaited: the run still ends, and
    /// says how when it has.
    pub(crate) fn stopper(&self) -> Stopper {
        Stopper(self.stop.0.clone(), self.stop.1.clone())
    }

    /// How the run ended: its task's outcome, once it has.
    pub(crate) async fn ended(mut self) -> HookRan {
        match (&mut self.outcome).await {
            Ok(ran) => ran,
            Err(_) => HookRan::Failed("its run ended before it could say how".to_owned()),
        }
    }
}

/// Stops a run whose end is still awaited.
pub(crate) struct Stopper(CancelToken, CancelToken);

impl Stopper {
    /// Stops it: a call not started is withdrawn, and one running stopped.
    pub(crate) fn stop(&self) {
        self.0.cancel();
        self.1.cancel();
    }
}

/// Stops a run when what waits for it lets it go: the call is withdrawn if
/// it has not started, and stopped if it has, a program it runs with it.
struct StopOnDrop(CancelToken, CancelToken);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
        self.1.cancel();
    }
}

/// One run of a hook counted while its task lives.
struct Counted(Arc<AtomicUsize>);

impl Counted {
    fn new(running: &Arc<AtomicUsize>) -> Self {
        running.fetch_add(1, Ordering::AcqRel);
        Self(Arc::clone(running))
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// What one run's task owns.
struct Call {
    extension: Arc<LoadedExtension>,
    commands: Arc<HookCommands>,
    paths: Arc<PathResolver>,
    name: String,
    index: usize,
    session: SessionId,
    turn: TurnId,
    cancel: CancelToken,
    kill: CancelToken,
}

impl Call {
    /// The run, to its end.
    async fn run(self, input: Json, timed: (Duration, Duration)) -> HookRan {
        let policy = Arc::clone(self.commands.policy());
        // A stopped turn runs nothing more: a hook of it does not start,
        // and is no stop of its own (the maintainers, for hooks: a stopped
        // turn's hooks do not start).
        if policy.stopped_turn(turn_number(&self.turn)).is_some() {
            return HookRan::Failed("it was not started: its turn was stopped".to_owned());
        }
        let cx = RunCx::new(
            Arc::clone(&self.paths),
            Arc::clone(self.commands.content()),
            Arc::new(FileTable::new()),
        )
        .with_cancel(self.cancel)
        .with_kill(self.kill)
        // A walk of a hook's reads the session's gate, as the model's does.
        .with_gate(policy.read_gate())
        .with_turn(turn_number(&self.turn))
        .with_call(CallRef {
            session: self.session,
            turn: self.turn.clone(),
            call_id: CallId::new(format!("hook_{}", self.name)),
            item: ItemId::new(format!("hook_{}_{}", self.name, self.turn.as_str())),
        });
        let ran = self
            .extension
            .call_hook(self.index, &self.name, input, &cx, timed)
            .await;
        if let HookRan::Stopped(note) = &ran {
            // Told at once, and the next prompt held back, on any event.
            self.commands.stopped(&self.name, &self.turn, note);
        }
        ran
    }
}
