//! The loop's seam for hooks, filled (`cineris_agent::Hooks`;
//! `docs/steps/28/answered.md`, "6. Hooks in the loop").
//!
//! - **One snapshot.** The session's hook commands are settled when it
//!   opens (`super::hooks::in_session`), and a change of their
//!   configuration takes effect in the next session: so `wants` and
//!   `matching` read one set, and a reload never takes a gate away from a
//!   session it held, nor gives one a hook nobody approved for it.
//! - **Failing closed.** A hook that cannot run is on its event all the
//!   same, as a hook whose run fails with why and what to do: an entry that
//!   does not compile, a project's that takes a name of the user's, and a
//!   project's that has changed since it was approved. `wants` is `false`
//!   only when no hook, of any of these, is on the event for the target.
//! - **A stop is never lost.** A hook whose look at the protected paths
//!   stopped a turn is told to the user at once, and the next prompt is
//!   refused with it, by a `prompt.submit` hook of the harness's own that
//!   runs once (`HookCommands::stopped`).
//! - **A hook's run is its own.** Each run is a task of its own, so that
//!   what follows it (the look after a start with a write root, the stop)
//!   is done whatever became of the loop's wait for it: dropped, the run's
//!   future stops the program, its process tree with it, and the task goes
//!   on to the look.
//! - **A replacement may not write a protected path** (`check_replacement`):
//!   judged as an extension's write is (`super::protected`).

use std::sync::Arc;
use std::time::Duration;

use cineris_agent::{
    Hook, HookEvent as LoopEvent, HookInput, HookOutcome as Heard, HookSubject, Hooks, PlannedCall,
    ToolTarget,
};
use cineris_base::{CancelToken, in_code};
use cineris_ext::event::Event;
use cineris_ext::hook::{CommandHook, Filter};
use cineris_tools::{BoxFuture, Effect};

use super::hook_command::{
    HookCommands, HookCx, HookOutcome, Owned, SessionHooks, TURN_END_AT_ONCE,
};
use super::hooks::Refused;

/// The event of the loop's that an event of a hook's configuration is:
/// `None` for the two the core runs itself (`session.start`,
/// `session.end`).
pub(crate) fn loop_event(event: Event) -> Option<LoopEvent> {
    match event {
        Event::PromptSubmit => Some(LoopEvent::PromptSubmit),
        Event::ToolBefore => Some(LoopEvent::ToolBefore),
        Event::ToolAfter => Some(LoopEvent::ToolAfter),
        Event::TurnEnd => Some(LoopEvent::TurnEnd),
        Event::CompactionBefore => Some(LoopEvent::CompactionBefore),
        Event::SessionStart | Event::SessionEnd => None,
    }
}

/// One hook on an event of the loop's, as `wants` and `matching` read it.
struct OnEvent {
    event: LoopEvent,
    /// The tools it is on: `None` for every tool.
    filter: Option<Filter>,
    run: Arc<dyn Hook>,
}

/// A session's hooks as the loop reaches them.
pub(crate) struct PointHooks {
    on: Vec<OnEvent>,
    commands: Arc<HookCommands>,
    paths: Arc<cineris_base::PathResolver>,
}

impl std::fmt::Debug for PointHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PointHooks")
            .field("hooks", &self.on.len())
            .finish_non_exhaustive()
    }
}

impl PointHooks {
    /// The loop's view of `hooks`: each one on an event of the loop's, and
    /// each refused one there as a hook that fails.
    pub(crate) fn of(hooks: &SessionHooks) -> Self {
        let mut on = Vec::new();
        for hook in &hooks.hooks {
            if let Some(event) = loop_event(hook.event) {
                on.push(OnEvent {
                    event,
                    filter: Some(hook.filter.clone()),
                    run: Arc::new(CommandRun {
                        hook: hook.clone(),
                        commands: Arc::clone(&hooks.commands),
                        paths: Arc::clone(&hooks.paths),
                        running: Arc::default(),
                    }),
                });
            }
        }
        for refused in &hooks.refused {
            if let Some(event) = refused.event.and_then(loop_event) {
                on.push(OnEvent {
                    event,
                    filter: refused.filter.clone(),
                    run: Arc::new(Failing::of(refused)),
                });
            }
        }
        Self {
            on,
            commands: Arc::clone(&hooks.commands),
            paths: Arc::clone(&hooks.paths),
        }
    }
}

/// Whether a hook on `filter` (`None`: every tool) is on `target`.
fn on_target(filter: Option<&Filter>, target: Option<ToolTarget<'_>>) -> bool {
    match (filter, target) {
        (Some(filter), Some(target)) => filter.matches(target.tool, target.names),
        // A refused entry whose `tools` could not be read: on every call,
        // failing closed.
        (None, Some(_)) => true,
        // An event of no call (a prompt, a turn's end): every hook on it.
        (_, None) => true,
    }
}

impl Hooks for PointHooks {
    fn wants(&self, event: LoopEvent, target: Option<ToolTarget<'_>>) -> bool {
        (event == LoopEvent::PromptSubmit && self.commands.stop_pending())
            || self
                .on
                .iter()
                .any(|hook| hook.event == event && on_target(hook.filter.as_ref(), target))
    }

    fn matching(&self, event: LoopEvent, target: Option<ToolTarget<'_>>) -> Vec<Arc<dyn Hook>> {
        let mut matched: Vec<Arc<dyn Hook>> = self
            .on
            .iter()
            .filter(|hook| hook.event == event && on_target(hook.filter.as_ref(), target))
            .map(|hook| Arc::clone(&hook.run))
            .collect();
        if event == LoopEvent::PromptSubmit && self.commands.stop_pending() {
            matched.push(Arc::new(Stopped {
                commands: Arc::clone(&self.commands),
            }));
        }
        matched
    }

    fn check_replacement(&self, call: &PlannedCall<'_>) -> Result<(), String> {
        let protect = self
            .commands
            .policy()
            .engine()
            .lease()
            .grant
            .sandbox
            .protect
            .clone();
        for effect in &call.plan.effects {
            if let Effect::Write(path) = effect
                && let Some(why) = super::protected::refusal(&self.paths, path, &protect)
            {
                return Err(format!(
                    "The arguments a hook put in place of the model's would write a protected \
                     path, which no hook's replacement may: {why}"
                ));
            }
        }
        Ok(())
    }
}

/// A hook command, as the loop runs it.
struct CommandRun {
    hook: CommandHook,
    commands: Arc<HookCommands>,
    paths: Arc<cineris_base::PathResolver>,
    /// How many runs of it are going on: a waited-for `turn.end` hook has
    /// at most [`TURN_END_AT_ONCE`].
    running: Arc<std::sync::atomic::AtomicUsize>,
}

/// One run of a hook counted while its task lives.
struct Counted(Arc<std::sync::atomic::AtomicUsize>);

impl Counted {
    fn new(running: &Arc<std::sync::atomic::AtomicUsize>) -> Self {
        running.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self(Arc::clone(running))
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Stops a run when what waits for it lets it go: the program is stopped,
/// its process tree with it, and the run's task goes on to its look.
struct StopOnDrop(CancelToken, CancelToken);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
        self.1.cancel();
    }
}

impl Hook for CommandRun {
    fn name(&self) -> &str {
        &self.hook.name
    }

    fn observes(&self) -> bool {
        self.hook.observe
    }

    fn timeout(&self) -> Option<Duration> {
        Some(self.hook.timeout)
    }

    fn run<'a>(&'a self, input: &'a HookInput) -> BoxFuture<'a, Heard> {
        let json = input.to_json();
        let (tool, effects) = match &input.subject {
            HookSubject::Call {
                tool,
                arguments,
                effects,
                ..
            }
            | HookSubject::Ran {
                tool,
                arguments,
                effects,
                ..
            } => {
                // For a `tool_call`, the tool it names fills `{tool}`.
                let target = ToolTarget::of(tool, arguments);
                (
                    Some(target.names.unwrap_or(target.tool).to_owned()),
                    effects.clone(),
                )
            }
            _ => (None, Vec::new()),
        };
        // A waited-for `turn.end` hook runs at most so many times at once,
        // as an observer does: turns that end faster than it runs leave the
        // rest unstarted.
        if self.hook.event == Event::TurnEnd
            && !self.hook.observe
            && self.running.load(std::sync::atomic::Ordering::Acquire) >= TURN_END_AT_ONCE
        {
            return Box::pin(std::future::ready(Heard::Failed(format!(
                "The hook `{}` was not started: {TURN_END_AT_ONCE} runs of it are going on",
                in_code(&self.hook.name)
            ))));
        }
        let cancel = CancelToken::new();
        let kill = CancelToken::new();
        // Made before the run is: a future let go before it is first
        // polled stops the run too (the review of the second pull request,
        // M3: the loop's observers can be let go so).
        let stop = StopOnDrop(cancel.clone(), kill.clone());
        let cx = HookCx {
            session: input.session.clone(),
            turn: input.turn.clone(),
            cancel,
            kill,
            paths: Arc::clone(&self.paths),
        };
        // A task of its own, on the runtime's thread and never the blocking
        // pool's (a program's death signal is tied to the thread that
        // started it), kept by the session until its close waits for it:
        // what follows the program is done whatever becomes of the wait for
        // it.
        let outcome = self.commands.spawn_run(
            self.hook.clone(),
            Owned {
                input: json,
                tool,
                effects,
            },
            cx,
            Box::new(Counted::new(&self.running)),
        );
        Box::pin(async move {
            let _stop = stop;
            match outcome.await {
                Ok(HookOutcome::Answered(answer)) => Heard::Answer(answer),
                Ok(HookOutcome::Nothing | HookOutcome::Observed) => Heard::Nothing,
                Ok(HookOutcome::Failed(why) | HookOutcome::Stopped(why)) => Heard::Failed(why),
                Err(_) => Heard::Failed(format!(
                    "The hook `{}` failed: its run ended before it could say how",
                    in_code(&self.hook.name)
                )),
            }
        })
    }
}

/// A hook that cannot run, on its event all the same: its run fails, with
/// why and what to do.
struct Failing {
    name: String,
    observes: bool,
    why: String,
}

impl Failing {
    fn of(refused: &Refused) -> Self {
        let whose = match refused.source {
            cineris_ext::source::Source::User => "Your hook",
            cineris_ext::source::Source::Project => "This project's hook",
        };
        Self {
            name: refused.name.clone(),
            observes: refused.observe,
            why: format!(
                "{whose} `{}` cannot run: {}. Correct its entry, or turn it off with `enabled = \
                 false`",
                in_code(&refused.name),
                refused.reason
            ),
        }
    }
}

impl Hook for Failing {
    fn name(&self) -> &str {
        &self.name
    }

    fn observes(&self) -> bool {
        self.observes
    }

    fn timeout(&self) -> Option<Duration> {
        None
    }

    fn run<'a>(&'a self, _input: &'a HookInput) -> BoxFuture<'a, Heard> {
        Box::pin(std::future::ready(Heard::Failed(self.why.clone())))
    }
}

/// The harness's own `prompt.submit` hook while a hook's look has stopped
/// a turn: it refuses the next prompt once, with what the look found.
struct Stopped {
    commands: Arc<HookCommands>,
}

impl Hook for Stopped {
    fn name(&self) -> &str {
        "protected paths"
    }

    fn observes(&self) -> bool {
        false
    }

    fn timeout(&self) -> Option<Duration> {
        None
    }

    fn run<'a>(&'a self, _input: &'a HookInput) -> BoxFuture<'a, Heard> {
        let stop = self.commands.take_stop();
        Box::pin(std::future::ready(match stop {
            Some(why) => Heard::Failed(why),
            // Taken by another prompt in the meantime: this one goes on.
            None => Heard::Nothing,
        }))
    }
}
