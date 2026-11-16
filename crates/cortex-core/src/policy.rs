//! The permission engine, as the loop, the tools and the session store
//! reach it (`docs/architecture/06-permissions-and-sandbox.md`).
//!
//! `cortex-policy`'s engine decides and records; it asks nobody and can see
//! none of the crates that need its answers. This module adapts it to the
//! three seams:
//!
//! | Seam | Here |
//! |---|---|
//! | The loop's `Policy::decide` | The plan's effects as the engine's accesses, and its outcome as the loop's decision. |
//! | The loop's `Policy::ask` | The question put to the front ends that may answer, the wait and its time limit, and the answer recorded. |
//! | The tools' `ReadGate` | `Engine::may_read`. |
//! | The base state's `KeepGate` | `Engine::may_store`. |
//!
//! # When nobody can be asked
//!
//! A call that needs an answer, in a session with no front end that can
//! give one, is refused: the model is told why, and the round goes on. The
//! refusal is kept ([`PermissionDenial`]), because it is the usual reason a
//! run without a person does less than was asked of it. With
//! `fail_on_denial` the same call ends the turn, through the loop's `Stop`,
//! with the outcome `denied`.

#![allow(
    clippy::disallowed_types,
    reason = "recording an approval for the project writes a file, on the blocking pool"
)]

use std::future::poll_fn;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::task::Poll;
use std::time::Duration;

use cortex_agent::{Decision, PlannedCall, Policy, Verdict};
use cortex_base::{AbsPath, SharedTimer};
use cortex_policy::{Access, Engine, Outcome, Question, Request};
use cortex_protocol::event::{Event, RequestResolved, Resolution};
use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::lease::Effect as EffectKind;
use cortex_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalRequest, ApprovalResponse, CommandDetail, DiffDetail,
    ServerRequest, ServerResponse, TargetDetail, ToolDetail,
};
use cortex_protocol::{CallId, ItemId, TurnId};
use cortex_session::KeepGate;
use cortex_tools::{BoxFuture, Effect, NetworkTarget, ReadGate};

use crate::clients::Clients;
use crate::events::SessionSink;
use crate::store::Hooks;

/// A tool call that was refused because nobody could be asked about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDenial {
    /// The turn the call belonged to.
    pub turn_id: TurnId,
    /// The call.
    pub call_id: CallId,
    /// The tool.
    pub tool: String,
    /// The call in one line.
    pub summary: String,
    /// Why it needed an answer.
    pub reason: String,
}

/// The point inside a change of the approvals or the learned commands, with
/// the engine in force taken and nothing changed yet: a test holds a
/// revoke, a record or a hand-over of learned commands there.
pub(crate) const CHANGING_APPROVALS: &str = "core.policy.changing-approvals";

/// The point where another engine is about to be put in force, with the
/// lock held.
pub(crate) const REPLACING: &str = "core.policy.replacing";

/// The engine in force, and what changes it beside the calls it decides:
/// another engine put in force for a change of level, approvals recorded
/// and taken back, learned commands handed over, and a turn stopped. The
/// learned commands are kept by `knowledge` under a lock of its own, taken
/// before this one: a change of level reads them under it and hands them
/// to the new engine here, with the swap ([`EngineInForce::replace_with`]).
///
/// A change of level makes its engine, writes the new lease to the log,
/// and only then puts the engine in force. An approval recorded or taken
/// back in between reached the old engine alone, and was lost (found by
/// the reviews of the re-decide step of step 12). So the new engine takes
/// over what changed in the old one when it is put in force, and a record,
/// a revoke and a replacement hold one lock, `changing`, so that none of
/// them works on an engine another is replacing.
pub(crate) struct EngineInForce {
    engine: RwLock<Arc<Engine>>,
    /// What [`Policy::narrowed`] adds to the engine in force's own count:
    /// the counts of the engines it replaced, one for each replacement, and
    /// one for each stop. Changed with the engine's lock held for writing,
    /// or only ever added to, and read with it held for reading, so the sum
    /// only grows.
    beside: std::sync::atomic::AtomicU64,
    changing: Mutex<()>,
}

impl EngineInForce {
    pub(crate) fn new(engine: Engine) -> Self {
        Self {
            engine: RwLock::new(Arc::new(engine)),
            beside: std::sync::atomic::AtomicU64::new(0),
            changing: Mutex::new(()),
        }
    }

    /// The engine in force.
    pub(crate) fn get(&self) -> Arc<Engine> {
        Arc::clone(&self.engine.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// How many times the rules in force have narrowed: [`Policy::narrowed`].
    pub(crate) fn narrowed(&self) -> u64 {
        let in_force = self.engine.read().unwrap_or_else(PoisonError::into_inner);
        self.beside.load(std::sync::atomic::Ordering::Acquire) + in_force.narrowed()
    }

    /// Counts a stop of the turn as a narrowing: every call of the turn
    /// that has not started is decided again, and the policy answers it
    /// with the stop, whatever the command that stopped it was (found by
    /// the review of the re-decide step: a command the engine found only
    /// reads does not count as a change, and its look may still stop the
    /// turn).
    pub(crate) fn stop(&self) {
        self.beside
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    /// Puts `engine` in force. It takes over the approvals of the engine in
    /// force ([`Engine::keep_approvals_of`]) as they are now, not as they
    /// were when `engine` was made. That counts as a narrowing,
    /// whichever way the level went: a change of level is rare, and a call
    /// decided before it is decided again by the new engine, at a cost of
    /// microseconds.
    pub(crate) fn replace(&self, engine: Engine) {
        // A session with no learned commands: the new engine has none, as
        // its template has none.
        let _ = self.replace_with(engine, Vec::new());
    }

    /// [`replace`](Self::replace), with the session's learned commands
    /// given to `engine` under the same lock, before it is in force: so the
    /// new engine never runs without them, and a command turned off while
    /// the level changed is off in it (step 17 keeps the list, under a lock
    /// of its own taken before this one). A list that cannot be used leaves
    /// no learned command in force, as [`Engine::set_learned`] does; the
    /// engine is put in force all the same.
    pub(crate) fn replace_with(
        &self,
        engine: Engine,
        learned: Vec<cortex_protocol::lease::Rule>,
    ) -> Result<(), Vec<cortex_policy::RuleError>> {
        let _changing = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        cortex_base::hook::point(REPLACING);
        let mut in_force = self.engine.write().unwrap_or_else(PoisonError::into_inner);
        engine.keep_approvals_of(&in_force);
        let given = engine.set_learned(learned);
        self.beside
            .fetch_add(in_force.narrowed() + 1, std::sync::atomic::Ordering::AcqRel);
        *in_force = Arc::new(engine);
        given
    }

    /// Gives the engine in force the session's learned commands, in place
    /// of the ones it had ([`Engine::set_learned`]), never one a change of
    /// level is replacing.
    pub(crate) fn set_learned(
        &self,
        rules: Vec<cortex_protocol::lease::Rule>,
    ) -> Result<(), Vec<cortex_policy::RuleError>> {
        let _changing = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let engine = self.get();
        cortex_base::hook::point(CHANGING_APPROVALS);
        engine.set_learned(rules)
    }

    /// Takes approvals back from the engine in force, by id. Writing the
    /// project's file blocks: call this off the core thread.
    pub(crate) fn revoke(
        &self,
        ids: &[cortex_protocol::ids::ApprovalId],
    ) -> Result<usize, cortex_policy::ApprovalError> {
        let _changing = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let engine = self.get();
        cortex_base::hook::point(CHANGING_APPROVALS);
        engine.revoke(ids)
    }

    /// Records a person's answer in the engine in force. Writing the
    /// project's file blocks: call this off the core thread.
    pub(crate) fn record(
        &self,
        question: &Question,
        choice: &ApprovalChoice,
    ) -> Result<cortex_policy::Recorded, cortex_policy::ApprovalError> {
        let _changing = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let engine = self.get();
        cortex_base::hook::point(CHANGING_APPROVALS);
        engine.record(question, choice)
    }
}

/// One session's engine, adapted.
pub(crate) struct EnginePolicy {
    in_force: Arc<EngineInForce>,
    /// The questions a person allowed, each with the call it was about: a
    /// call decided again whose question and accesses are the same is
    /// allowed without asking again ("allow once" leaves no rule). Only
    /// the latest turn's are kept.
    allowed: Mutex<Vec<Allowed>>,
    clients: Arc<Clients>,
    sink: Arc<SessionSink>,
    hooks: Arc<Hooks>,
    timer: SharedTimer,
    /// How long an approval may wait for its answer. `None`: as long as it
    /// takes.
    timeout: Option<Duration>,
    fail_on_denial: bool,
    /// The front end can stop the turn from a question: the interactive
    /// application's interrupt key (`CoreOptions::answers_may_stop`).
    stops_from_questions: bool,
    /// Whether commands run confined: what an approval's detail says. It
    /// follows the session's level, since nothing is confined at
    /// `unrestricted`.
    sandboxed: std::sync::atomic::AtomicBool,
    /// The turn whose later calls are stopped, and why: a confined command
    /// changed protected paths in it.
    stopped: Mutex<Option<(u64, String)>>,
    denials: Mutex<Vec<PermissionDenial>>,
    /// Called before a command is decided: the project's knowledge looks
    /// at whether its files changed, and gives the engine the learned
    /// rules that follow from them when they did (`crate::knowledge`).
    before_command: std::sync::OnceLock<Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for EnginePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnginePolicy")
            .field("fail_on_denial", &self.fail_on_denial)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// A question a person allowed, and the call it was about: the loop's own
/// id for the call, which is unique in the session (a provider's may
/// repeat), and every access of the call as it was shown, its paths where
/// they led and a command's directory included. The question's reasons
/// name a command by its words, not where it runs: a call planned again
/// into another directory is another question.
struct Allowed {
    turn: TurnId,
    item: ItemId,
    accesses: Vec<Access>,
    question: Question,
}

/// What a policy is made from.
pub(crate) struct PolicyParts {
    pub(crate) engine: Engine,
    pub(crate) clients: Arc<Clients>,
    pub(crate) sink: Arc<SessionSink>,
    pub(crate) hooks: Arc<Hooks>,
    pub(crate) timer: SharedTimer,
    pub(crate) timeout: Option<Duration>,
    pub(crate) fail_on_denial: bool,
    pub(crate) stops_from_questions: bool,
    pub(crate) sandboxed: bool,
}

impl EnginePolicy {
    pub(crate) fn new(parts: PolicyParts) -> Self {
        Self {
            in_force: Arc::new(EngineInForce::new(parts.engine)),
            allowed: Mutex::new(Vec::new()),
            clients: parts.clients,
            sink: parts.sink,
            hooks: parts.hooks,
            timer: parts.timer,
            timeout: parts.timeout,
            fail_on_denial: parts.fail_on_denial,
            stops_from_questions: parts.stops_from_questions,
            sandboxed: std::sync::atomic::AtomicBool::new(parts.sandboxed),
            stopped: Mutex::new(None),
            denials: Mutex::new(Vec::new()),
            before_command: std::sync::OnceLock::new(),
        }
    }

    /// Sets what is called before each command is decided. Once.
    pub(crate) fn on_command(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        let _ = self.before_command.set(hook);
    }

    /// The engine in force.
    pub(crate) fn engine(&self) -> Arc<Engine> {
        self.in_force.get()
    }

    /// Stops turn `turn` at its next call: a confined command changed
    /// protected paths, and the user is to look before anything else runs.
    /// Every call of it that has not started is decided again, and is
    /// answered with the stop.
    pub(crate) fn stop_turn(&self, turn: u32, reason: String) {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((u64::from(turn), reason));
        self.in_force.stop();
    }

    /// Why turn number `turn` was stopped, if it was.
    pub(crate) fn stopped_turn(&self, turn: u32) -> Option<String> {
        let stopped = self.stopped.lock().unwrap_or_else(PoisonError::into_inner);
        stopped
            .as_ref()
            .filter(|(stopped, _)| *stopped == u64::from(turn))
            .map(|(_, reason)| reason.clone())
    }

    /// Why `call`'s turn was stopped, if it was.
    fn stopped(&self, call: &PlannedCall<'_>) -> Option<String> {
        let number = cortex_base::IdCounter::new("t_").number_of(call.turn.as_str())?;
        let stopped = self.stopped.lock().unwrap_or_else(PoisonError::into_inner);
        stopped
            .as_ref()
            .filter(|(turn, _)| *turn == number)
            .map(|(_, reason)| reason.clone())
    }

    /// Says whether commands run confined from now on.
    pub(crate) fn set_sandboxed(&self, sandboxed: bool) {
        self.sandboxed
            .store(sandboxed, std::sync::atomic::Ordering::Release);
    }

    /// Puts another engine in force: the session's level changed
    /// ([`EngineInForce::replace`]).
    pub(crate) fn replace(&self, engine: Engine) {
        self.in_force.replace(engine);
    }

    /// Puts another engine in force with the session's learned commands
    /// ([`EngineInForce::replace_with`]).
    pub(crate) fn replace_with(
        &self,
        engine: Engine,
        learned: Vec<cortex_protocol::lease::Rule>,
    ) -> Result<(), Vec<cortex_policy::RuleError>> {
        self.in_force.replace_with(engine, learned)
    }

    /// Gives the engine in force the session's learned commands
    /// ([`EngineInForce::set_learned`]).
    pub(crate) fn set_learned(
        &self,
        rules: Vec<cortex_protocol::lease::Rule>,
    ) -> Result<(), Vec<cortex_policy::RuleError>> {
        self.in_force.set_learned(rules)
    }

    /// Takes approvals back from the engine in force, by id: never from one
    /// a change of level is replacing ([`EngineInForce::revoke`]). Writing
    /// the project's file blocks: call this off the core thread.
    pub(crate) fn revoke(
        &self,
        ids: &[cortex_protocol::ids::ApprovalId],
    ) -> Result<usize, cortex_policy::ApprovalError> {
        self.in_force.revoke(ids)
    }

    /// Whether a person allowed `question` for `call`, as it is planned
    /// now.
    fn was_allowed(&self, call: &PlannedCall<'_>, question: &Question) -> bool {
        let allowed = self.allowed.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(earlier) = allowed.iter().find(|allowed| allowed.item == *call.item) else {
            return false;
        };
        earlier.question == *question && Some(&earlier.accesses) == accesses(call).as_ref()
    }

    /// Remembers that a person allowed `question` for `call`, dropping
    /// what was allowed in earlier turns.
    fn remember_allowed(&self, call: &PlannedCall<'_>, question: Question) {
        let Some(accesses) = accesses(call) else {
            return;
        };
        let mut allowed = self.allowed.lock().unwrap_or_else(PoisonError::into_inner);
        allowed.retain(|allowed| allowed.turn == *call.turn && allowed.item != *call.item);
        allowed.push(Allowed {
            turn: call.turn.clone(),
            item: call.item.clone(),
            accesses,
            question,
        });
    }

    /// The calls refused so far because nobody could be asked.
    pub(crate) fn denials(&self) -> Vec<PermissionDenial> {
        self.denials
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// What the base state and the instruction files may keep and name:
    /// what the read rules of the engine in force would let the agent read
    /// unasked. The engine is asked for each path, not taken once: a change
    /// of level replaces it, and what is approved or refused after that is
    /// the new one's (the independent review of step 18 found the index,
    /// which [`EnginePolicy::index_gate`] serves now, left on the engine of
    /// the session's assembly).
    pub(crate) fn keep_gate(self: &Arc<Self>) -> Arc<dyn KeepGate> {
        let policy = Arc::downgrade(self);
        Arc::new(move |path: &AbsPath| {
            policy
                .upgrade()
                .is_some_and(|policy| policy.engine().may_store(path))
        })
    }

    /// The repository index's gate: [`EnginePolicy::keep_gate`]'s answer, with
    /// the engine in force taken once for each pass and each map rather
    /// than for each path. Taking it costs a lock and two counts shared
    /// by the pass's threads, which under four of them came to 0.55 µs a
    /// path, a pass at its cap of 100,000 entries 14 ms (found by the
    /// independent review of step 18's fixes). What is approved or refused
    /// after a change of level is still the next pass's and the next
    /// map's.
    pub(crate) fn index_gate(self: &Arc<Self>) -> Arc<dyn cortex_index::Gate> {
        let policy = Arc::downgrade(self);
        Arc::new(InForce {
            now: Box::new(move || policy.upgrade().map(|policy| policy.engine())),
            ask: Engine::may_store,
        })
    }

    fn outcome(&self, call: &PlannedCall<'_>) -> Outcome {
        let Some(accesses) = accesses(call) else {
            // The engine cannot be shown the whole of what the call does,
            // so it cannot be asked to allow it.
            return Outcome::Deny {
                reason: format!(
                    "The tool `{}` declared an effect this version of the permission engine does \
                     not know, so the call was not run.",
                    call.spec.name
                ),
            };
        };
        if accesses
            .iter()
            .any(|access| matches!(access, Access::Exec { .. }))
            && let Some(hook) = self.before_command.get()
        {
            hook();
        }
        self.engine().decide(&Request {
            tool: &call.spec.name,
            default: call.spec.permission,
            accesses: &accesses,
        })
    }

    /// The answer when nobody can be asked.
    fn nobody(&self, call: &PlannedCall<'_>, question: &Question) -> Verdict {
        let reason = question.refusal();
        self.denials
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(PermissionDenial {
                turn_id: call.turn.clone(),
                call_id: call.call_id.clone(),
                tool: call.spec.name.clone(),
                summary: call.plan.summary.clone(),
                reason: question.reasons.join("; "),
            });
        if self.fail_on_denial {
            Verdict::Stop { reason }
        } else {
            Verdict::Deny { note: Some(reason) }
        }
    }

    fn resolved(&self, id: &RequestId, resolution: Resolution) {
        self.hooks.emit(Event::RequestResolved(RequestResolved {
            request_id: id.clone(),
            resolution,
        }));
    }
}

/// The plan's effects, as the engine reads them. `None` when one of them
/// is an effect this version does not know: the call is then refused.
fn accesses(call: &PlannedCall<'_>) -> Option<Vec<Access>> {
    call.plan
        .effects
        .iter()
        .map(|effect| {
            Some(match effect {
                Effect::Read(path) => Access::Read {
                    path: path.abs().clone(),
                },
                Effect::ReadTree(path) => Access::ReadTree {
                    path: path.abs().clone(),
                },
                Effect::Write(path) => Access::Write {
                    path: path.abs().clone(),
                },
                Effect::Exec { command, cwd } => Access::Exec {
                    command: command.clone(),
                    cwd: cwd.abs().clone(),
                },
                Effect::Network(NetworkTarget::Host(host)) => Access::Network {
                    host: Some(host.clone()),
                },
                Effect::Network(NetworkTarget::Any) => Access::Network { host: None },
                Effect::Ext { tool } => Access::Ext { tool: tool.clone() },
                Effect::Unconfined => Access::Unconfined,
                // An effect this version does not know cannot be shown to
                // the engine as something it knows, and leaving it out
                // would have the engine decide on less than the call does.
                _ => return None,
            })
        })
        .collect()
}

/// Exactly what an approval would let happen, for the front end to show.
fn detail(call: &PlannedCall<'_>, sandboxed: bool) -> ApprovalDetail {
    let effects = &call.plan.effects;
    // A call that asked to leave the sandbox runs outside it, whatever the
    // session's other commands do.
    let sandboxed = sandboxed && !effects.contains(&Effect::Unconfined);
    let command = effects.iter().find_map(|effect| match effect {
        Effect::Exec { command, cwd } => Some(ApprovalDetail::Command(CommandDetail {
            command: command.clone(),
            cwd: cwd.abs().to_native_string(),
            outside_sandbox: !sandboxed,
        })),
        _ => None,
    });
    if let Some(command) = command {
        return command;
    }
    // A change to a file, as its tool would make it, so that a front end
    // shows it without the call's arguments.
    if let Some(preview) = call.preview {
        return ApprovalDetail::Diff(DiffDetail {
            path: preview.path.clone(),
            diff: preview.diff.clone(),
            omitted: preview.omitted.clone(),
        });
    }
    let target = |effect: EffectKind, target: String| {
        ApprovalDetail::Target(TargetDetail { effect, target })
    };
    match effects.first() {
        Some(Effect::Write(path)) => target(EffectKind::Write, path.abs().to_native_string()),
        Some(Effect::Read(path) | Effect::ReadTree(path)) => {
            target(EffectKind::Read, path.abs().to_native_string())
        }
        Some(Effect::Network(host)) => target(
            EffectKind::Network,
            match host {
                NetworkTarget::Host(host) => host.clone(),
                _ => "any host".to_owned(),
            },
        ),
        _ => ApprovalDetail::Tool(ToolDetail {
            arguments: call.arguments.clone(),
        }),
    }
}

/// An open request, which is closed and reported as withdrawn if the wait
/// for its answer is given up: the turn was interrupted, or the response
/// the call came from was thrown away.
struct Asked<'a> {
    policy: &'a EnginePolicy,
    id: RequestId,
    settled: bool,
}

impl Drop for Asked<'_> {
    fn drop(&mut self) {
        if !self.settled {
            // An answer may have been taken in this very instant. Nobody
            // will act on it, so the clients are told the same thing
            // either way: the question is withdrawn.
            self.policy.clients.close(&self.id);
            self.policy.resolved(&self.id, Resolution::Cancelled);
        }
    }
}

impl Policy for EnginePolicy {
    fn decide(&self, call: &PlannedCall<'_>) -> Decision {
        if let Some(reason) = self.stopped(call) {
            return Decision::Stop { reason };
        }
        match self.outcome(call) {
            Outcome::Allow { read_only: true } => Decision::AllowReadOnly,
            Outcome::Allow { read_only: false } => Decision::Allow,
            // The call is decided again, and a person already allowed it
            // when asked this very question.
            Outcome::Ask(question) if self.was_allowed(call, &question) => Decision::Allow,
            Outcome::Ask(_) => Decision::Ask,
            Outcome::Deny { reason } => Decision::Deny {
                reason: Some(reason),
            },
        }
    }

    fn ask<'a>(&'a self, call: PlannedCall<'a>) -> BoxFuture<'a, Verdict> {
        Box::pin(async move {
            if let Some(reason) = self.stopped(&call) {
                return Verdict::Stop { reason };
            }
            // Decided again: an answer to an earlier question of the round
            // may have settled this one.
            let question = match self.outcome(&call) {
                Outcome::Allow { .. } => return Verdict::Allow,
                Outcome::Deny { reason } => return Verdict::Deny { note: Some(reason) },
                Outcome::Ask(question) => question,
            };
            let asked = self.clients.approvers(&self.sink.subscribers());
            if asked.is_empty() {
                return self.nobody(&call, &question);
            }

            let request = ApprovalRequest {
                session_id: call.session.clone(),
                turn_id: call.turn.clone(),
                call_id: call.call_id.clone(),
                tool: call.spec.name.clone(),
                summary: call.plan.summary.clone(),
                detail: detail(
                    &call,
                    self.sandboxed.load(std::sync::atomic::Ordering::Acquire),
                ),
                choices: question.choices(),
                reasons: question.reasons.clone(),
                adds: question
                    .lifts
                    .iter()
                    .map(|rule| cortex_protocol::methods::AddedRule {
                        effect: rule.effect.clone(),
                        pattern: rule.pattern.clone(),
                    })
                    .collect(),
            };
            let (id, mut answer) = self.clients.ask(call.session, &asked, |_| {
                ServerRequest::Approval(request.clone())
            });
            let mut open = Asked {
                policy: self,
                id: id.clone(),
                settled: false,
            };
            let mut limit = self.timeout.map(|limit| self.timer.sleep(limit));
            let answered: Option<ApprovalResponse> = poll_fn(|cx| {
                if let Poll::Ready(response) = std::pin::Pin::new(&mut answer).poll(cx) {
                    return Poll::Ready(response.ok().and_then(approval_of));
                }
                if let Some(limit) = &mut limit
                    && limit.as_mut().poll(cx).is_ready()
                {
                    return Poll::Ready(None);
                }
                Poll::Pending
            })
            .await;
            open.settled = true;
            // The time is up. A request that is not open any more was
            // answered in this instant, and the client was told its answer
            // was taken: it is.
            let answered = match answered {
                None if !self.clients.close(&id) => answer.try_recv().ok().and_then(approval_of),
                other => other,
            };

            let Some(response) = answered else {
                // Nobody answered in time: a denial, as the protocol says.
                self.resolved(&id, Resolution::TimedOut);
                return Verdict::Deny {
                    note: Some(format!(
                        "Nobody answered the approval request in time. {}",
                        question.refusal()
                    )),
                };
            };
            // The clients that did not answer take their prompt down.
            self.resolved(&id, Resolution::Answered);
            // The turn may have been stopped while the question was open:
            // a command beside this call changed protected paths. Nothing
            // more of the turn runs, whatever the answer (found by the
            // review of step 26).
            if let Some(reason) = self.stopped(&call) {
                return Verdict::Stop { reason };
            }

            // In the engine in force when it is recorded, never in one a
            // change of level is replacing.
            let in_force = Arc::clone(&self.in_force);
            let choice = response.choice.clone();
            let asked = question.clone();
            let recorded = tokio::task::spawn_blocking(move || in_force.record(&question, &choice));
            let allowed = match recorded.await {
                Ok(Ok(recorded)) => recorded.allowed,
                // The call was approved; what could not be done is keeping
                // the approval for later.
                Ok(Err(error)) => {
                    self.hooks.tell(format!(
                        "The approval was given, and could not be stored for later: {error}"
                    ));
                    !matches!(
                        response.choice,
                        ApprovalChoice::Deny | ApprovalChoice::Unknown(_)
                    )
                }
                Err(_) => false,
            };
            if allowed {
                // Decided again before it runs, the call is not asked this
                // question a second time.
                self.remember_allowed(&call, asked);
                Verdict::Allow
            } else {
                Verdict::Deny {
                    note: response.note,
                }
            }
        })
    }

    /// Whether an answer may be to stop the turn. A person's choices have
    /// no "stop". The answers that are one come from here: a question
    /// nobody can be asked, with `fail_on_denial` set; and any question
    /// while commands are confined, since a command beside it may change
    /// protected paths and stop the turn. Whether somebody can be asked is
    /// not known until the question is put (a client may leave in
    /// between), so the settings alone decide: with either, the loop holds
    /// later calls back while a question is open, which is slower and never
    /// wrong.
    fn answers_may_stop(&self) -> bool {
        // While commands are confined, a command may stop the turn while a
        // question is open, and the answer is then to stop (found by the
        // second review of step 26).
        // A front end that can stop the turn from a question (step 23's
        // interrupt key in a dialog) says so too.
        self.fail_on_denial
            || self.stops_from_questions
            || self.sandboxed.load(std::sync::atomic::Ordering::Acquire)
    }

    fn narrowed(&self) -> u64 {
        self.in_force.narrowed()
    }

    fn read_gate(&self) -> Arc<dyn ReadGate> {
        Arc::new(EngineGate(self.engine()))
    }
}

/// What a directory walk may read, entry by entry: the engine's read rules.
struct EngineGate(Arc<Engine>);

impl ReadGate for EngineGate {
    fn may_read(&self, path: &AbsPath) -> bool {
        self.0.may_read(path)
    }
}

/// [`EnginePolicy::index_gate`]: a gate that asks what is in force now
/// (`now`, the engine in force; nothing once the session is gone) for each
/// path, and that a pass or a map takes once at its start.
struct InForce<T> {
    now: Box<dyn Fn() -> Option<Arc<T>> + Send + Sync>,
    ask: fn(&T, &AbsPath) -> bool,
}

impl<T: Send + Sync + 'static> cortex_index::Gate for InForce<T> {
    fn may_index(&self, path: &AbsPath) -> bool {
        (self.now)().is_some_and(|in_force| (self.ask)(&in_force, path))
    }

    fn for_pass(&self) -> Option<Arc<dyn cortex_index::Gate>> {
        let taken = (self.now)();
        let ask = self.ask;
        Some(Arc::new(move |path: &AbsPath| {
            taken.as_deref().is_some_and(|in_force| ask(in_force, path))
        }))
    }
}

/// An approval's answer, from the table's answers of every kind: the table
/// refuses an answer of another kind, so nothing else arrives here.
fn approval_of(response: ServerResponse) -> Option<ApprovalResponse> {
    match response {
        ServerResponse::Approval(response) => Some(response),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cortex_base::PathStyle;
    use cortex_index::Gate;
    use cortex_policy::{Access, Outcome, Request};

    use super::*;

    /// The index's gate takes what is in force once for a pass, which then
    /// answers by it alone, and the next pass by what replaced it (the
    /// independent review of step 18's fixes: taking the engine for each
    /// path cost 0.55 µs a path under four threads).
    #[test]
    fn the_index_gate_takes_what_is_in_force_once_a_pass() {
        let in_force = Arc::new(RwLock::new(Arc::new("first".to_owned())));
        let taken = Arc::new(AtomicUsize::new(0));
        let gate = InForce {
            now: {
                let (in_force, taken) = (Arc::clone(&in_force), Arc::clone(&taken));
                Box::new(move || {
                    taken.fetch_add(1, Ordering::SeqCst);
                    Some(Arc::clone(
                        &in_force.read().unwrap_or_else(PoisonError::into_inner),
                    ))
                })
            },
            ask: |rules: &String, path: &AbsPath| path.as_str().contains(rules.as_str()),
        };
        let path = |text: &str| AbsPath::parse(text, PathStyle::Posix).expect("a path");
        let (first, second) = (path("/ws/first.rs"), path("/ws/second.rs"));

        let pass = gate.for_pass().expect("a pass takes the gate once");
        assert_eq!(taken.load(Ordering::SeqCst), 1);
        for _ in 0..10 {
            assert!(pass.may_index(&first));
            assert!(!pass.may_index(&second));
        }
        assert_eq!(taken.load(Ordering::SeqCst), 1, "asked again for a path");

        *in_force.write().unwrap_or_else(PoisonError::into_inner) = Arc::new("second".to_owned());
        assert!(pass.may_index(&first), "a pass answers by what it took");
        let next = gate.for_pass().expect("a pass takes the gate once");
        assert!(next.may_index(&second) && !next.may_index(&first));
        assert!(gate.may_index(&second) && !gate.may_index(&first));

        let gone = InForce::<String> {
            now: Box::new(|| None),
            ask: |_, _| true,
        };
        assert!(!gone.may_index(&first));
        assert!(!gone.for_pass().expect("a gate").may_index(&first));
    }

    // ------------------------------------------------- the engine in force

    fn path(text: &str) -> AbsPath {
        AbsPath::parse(text, PathStyle::Posix).expect("a path")
    }

    /// An engine of a session at `supervised` in `/work/app`, with no rules.
    fn engine() -> Engine {
        let workspace = path("/work/app");
        Engine::start(cortex_policy::Parts {
            lease_id: cortex_protocol::ids::LeaseId::new("l_1"),
            level: cortex_protocol::lease::Level::Supervised,
            rules: Vec::new(),
            approved: Vec::new(),
            sandbox_policy: cortex_policy::default_sandbox(&workspace, &[], &[]),
            sandbox: cortex_policy::SandboxState::Enforcing,
            models: Vec::new(),
            budget: cortex_protocol::lease::Budget::default(),
            limits: cortex_protocol::lease::HardLimits::default(),
            workspace,
            home: Some(path("/users/dev")),
            trusted: true,
            repository_trusted: true,
            protected: cortex_policy::Protected::none(),
            shell: cortex_policy::ShellSyntax::Posix,
            locate: Arc::new(cortex_policy::Lexical {
                home: Some(path("/users/dev")),
            }),
            store: None,
            clock: Arc::new(
                cortex_base::FakeClock::starting_at("2026-11-13T10:00:00Z").expect("a time"),
            ),
        })
        .expect("an engine")
    }

    /// The question about `cargo build`, which "allow for this session"
    /// answers with the rule `cargo build *`.
    fn question(engine: &Engine) -> Question {
        let accesses = [Access::Exec {
            command: "cargo build".to_owned(),
            cwd: path("/work/app"),
        }];
        match engine.decide(&Request {
            tool: "shell",
            default: cortex_protocol::lease::RuleAction::Ask,
            accesses: &accesses,
        }) {
            Outcome::Ask(question) => question,
            other => panic!("not a question: {other:?}"),
        }
    }

    /// A revoke and a change of engine, at once: the revoke holds the lock
    /// and has taken the engine in force; the change of engine, made with
    /// the old engine's approvals as a change of level makes it, is about
    /// to be put in force. The revoke is not lost.
    #[test]
    fn a_revoke_and_a_change_of_engine_do_not_lose_each_other() {
        let first = engine();
        first
            .record(&question(&first), &ApprovalChoice::AllowSession)
            .expect("recorded");
        let id = first.approvals()[0].id.clone();
        let in_force = Arc::new(EngineInForce::new(first));
        let next = engine();
        next.keep_approvals_of(&in_force.get());
        assert_eq!(next.approvals().len(), 1);

        let revoking_held = cortex_testkit::Hold::new(CHANGING_APPROVALS);
        let revoking = {
            let in_force = Arc::clone(&in_force);
            std::thread::spawn(move || in_force.revoke(&[id]))
        };
        revoking_held.reach();
        let replacing = {
            let in_force = Arc::clone(&in_force);
            std::thread::spawn(move || in_force.replace(next))
        };
        // The change of engine waits for the lock, which the revoke holds.
        revoking_held.release();
        assert_eq!(revoking.join().expect("no panic").expect("revoked"), 1);
        replacing.join().expect("no panic");
        assert_eq!(
            in_force.get().approvals(),
            Vec::new(),
            "the revoke was lost"
        );
    }

    /// The same for an answer being recorded: "allow for this session"
    /// given while a change of level is being made is kept.
    #[test]
    fn an_answer_recorded_while_the_engine_changes_is_kept() {
        let first = engine();
        let asked = question(&first);
        let in_force = Arc::new(EngineInForce::new(first));
        let next = engine();
        next.keep_approvals_of(&in_force.get());

        let recording_held = cortex_testkit::Hold::new(CHANGING_APPROVALS);
        let recording = {
            let in_force = Arc::clone(&in_force);
            std::thread::spawn(move || in_force.record(&asked, &ApprovalChoice::AllowSession))
        };
        recording_held.reach();
        let replacing = {
            let in_force = Arc::clone(&in_force);
            std::thread::spawn(move || in_force.replace(next))
        };
        recording_held.release();
        assert!(
            recording
                .join()
                .expect("no panic")
                .expect("recorded")
                .allowed
        );
        replacing.join().expect("no panic");
        assert_eq!(in_force.get().approvals().len(), 1, "the answer was lost");
    }

    /// Each change of the engine in force holds the lock while it works, so
    /// that no other begins meanwhile: held at its point, the lock is taken.
    #[test]
    fn each_change_of_the_engine_in_force_holds_the_lock_while_it_works() {
        let first = engine();
        let asked = question(&first);
        let in_force = Arc::new(EngineInForce::new(first));
        type Change = Box<dyn FnOnce(&EngineInForce) + Send>;
        let rule = || {
            cortex_policy::exact_command_rule("cargo test", cortex_policy::ShellSyntax::Posix)
                .expect("a rule")
        };
        let changes: [(&str, &str, Change); 5] = [
            (
                "a record",
                CHANGING_APPROVALS,
                Box::new(move |in_force: &EngineInForce| {
                    in_force
                        .record(&asked, &ApprovalChoice::AllowSession)
                        .expect("recorded");
                }),
            ),
            (
                "a revoke",
                CHANGING_APPROVALS,
                Box::new(|in_force: &EngineInForce| {
                    in_force
                        .revoke(&[cortex_protocol::ids::ApprovalId::new("none")])
                        .expect("revoked");
                }),
            ),
            (
                "a replacement",
                REPLACING,
                Box::new(|in_force: &EngineInForce| in_force.replace(engine())),
            ),
            (
                "a hand-over of learned commands",
                CHANGING_APPROVALS,
                Box::new(move |in_force: &EngineInForce| {
                    in_force.set_learned(vec![rule()]).expect("learned");
                }),
            ),
            (
                "a replacement with learned commands",
                REPLACING,
                Box::new(move |in_force: &EngineInForce| {
                    in_force
                        .replace_with(engine(), vec![rule()])
                        .expect("learned");
                }),
            ),
        ];
        for (what, point, change) in changes {
            let held = cortex_testkit::Hold::new(point);
            let working = {
                let in_force = Arc::clone(&in_force);
                std::thread::spawn(move || change(&in_force))
            };
            held.reach();
            assert!(
                in_force.changing.try_lock().is_err(),
                "{what} does not hold the lock"
            );
            held.release();
            working.join().expect("no panic");
        }
    }

    /// A change of level hands the session's learned commands to the new
    /// engine with the swap, under the lock: the engine in force has them
    /// from the moment it is in force. A list that cannot be used leaves
    /// none in force, and the engine is put in force all the same.
    #[test]
    fn a_new_engine_is_put_in_force_with_the_learned_commands_given() {
        let rule =
            cortex_policy::exact_command_rule("cargo test", cortex_policy::ShellSyntax::Posix)
                .expect("a rule");
        let in_force = EngineInForce::new(engine());
        in_force
            .replace_with(engine(), vec![rule.clone()])
            .expect("learned");
        assert_eq!(in_force.get().learned(), vec![rule.clone()]);
        // Turned off: the next engine is given none.
        in_force.set_learned(Vec::new()).expect("turned off");
        in_force.replace_with(engine(), Vec::new()).expect("none");
        assert_eq!(in_force.get().learned(), Vec::new());
        let wrong = cortex_protocol::lease::Rule {
            effect: cortex_protocol::lease::Effect::Write,
            pattern: "x".to_owned(),
            action: cortex_protocol::lease::RuleAction::Allow,
        };
        let before = in_force.narrowed();
        assert!(in_force.replace_with(engine(), vec![rule, wrong]).is_err());
        assert_eq!(in_force.get().learned(), Vec::new());
        assert!(in_force.narrowed() > before, "the new engine is in force");
    }

    /// The count of narrowings moves for a stop of the turn and for a new
    /// engine, carries the count of the engine replaced, and only grows.
    #[test]
    fn a_stop_and_a_change_of_engine_move_the_count_which_only_grows() {
        let first = engine();
        first
            .record(&question(&first), &ApprovalChoice::AllowSession)
            .expect("recorded");
        let id = first.approvals()[0].id.clone();
        let in_force = EngineInForce::new(first);
        assert_eq!(in_force.narrowed(), 0);
        in_force.stop();
        assert_eq!(in_force.narrowed(), 1, "a stop narrows");
        in_force.revoke(&[id]).expect("revoked");
        assert_eq!(in_force.narrowed(), 2, "a revoke narrows");
        in_force.replace(engine());
        assert_eq!(
            in_force.narrowed(),
            3,
            "a new engine narrows, and its own count starts again"
        );
    }
}
