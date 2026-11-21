//! The permission engine, as the loop, the tools and the session store
//! reach it (`docs/architecture/06-permissions-and-sandbox.md`).
//!
//! `cineris-policy`'s engine decides and records; it asks nobody and can see
//! none of the crates that need its answers. This module adapts it to the
//! three seams:
//!
//! | Seam | Here |
//! |---|---|
//! | The loop's `Policy::decide` | The plan's effects as the engine's accesses, and its outcome as the loop's decision. |
//! | The loop's `Policy::ask` | The question put to the front ends that may answer, the wait and its time limit, and the answer recorded; a supervisor's allow judged against its grant. |
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
//! with the outcome `denied`. A core that keeps sessions
//! (`CoreOptions::keep_sessions`) asks later instead where the session has
//! no client at all: the question is put to nobody, and waits for a `user`
//! client to open the session, until its time limit.
//!
//! # A supervisor's answer
//!
//! A `supervisor` with a grant is put every approval of the session, at
//! once with the users (`crate::clients`). Its allow is taken only where
//! the grant allows the call ([`within_grant`]): an engine made from the
//! session's own engine template, with the session's standing lease
//! narrowed by the grant for its scope, and the grant's level and allow
//! rules for who decides. A call it allows is allowed once, and nothing is
//! recorded: a supervisor never reaches `Engine::record`.

#![allow(
    clippy::disallowed_types,
    reason = "recording an approval for the project writes a file, on the blocking pool"
)]

use std::future::poll_fn;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::task::Poll;
use std::time::Duration;

use cineris_agent::{Decision, PlannedCall, Policy, Verdict};
use cineris_base::{AbsPath, SharedTimer};
use cineris_policy::{Access, Engine, LeaseExt, Outcome, Question, Request};
use cineris_protocol::event::{Event, RequestResolved, Resolution};
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::lease::{Effect as EffectKind, Lease, Level, RuleAction};
use cineris_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalRequest, ApprovalResponse, CommandDetail, DiffDetail,
    ServerRequest, ServerResponse, TargetDetail, ToolDetail,
};
use cineris_protocol::{CallId, ItemId, TurnId};
use cineris_session::KeepGate;
use cineris_tools::{BoxFuture, Effect, NetworkTarget, ReadGate};

use crate::assemble::EngineTemplate;
use crate::clients::{Answer, Clients, Judge, Judged};
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
        learned: Vec<cineris_protocol::lease::Rule>,
    ) -> Result<(), Vec<cineris_policy::RuleError>> {
        let _changing = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        cineris_base::hook::point(REPLACING);
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
        rules: Vec<cineris_protocol::lease::Rule>,
    ) -> Result<(), Vec<cineris_policy::RuleError>> {
        let _changing = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let engine = self.get();
        cineris_base::hook::point(CHANGING_APPROVALS);
        engine.set_learned(rules)
    }

    /// Takes approvals back from the engine in force, by id. Writing the
    /// project's file blocks: call this off the core thread.
    pub(crate) fn revoke(
        &self,
        ids: &[cineris_protocol::ids::ApprovalId],
    ) -> Result<usize, cineris_policy::ApprovalError> {
        let _changing = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let engine = self.get();
        cineris_base::hook::point(CHANGING_APPROVALS);
        engine.revoke(ids)
    }

    /// Records a person's answer in the engine in force. Writing the
    /// project's file blocks: call this off the core thread.
    pub(crate) fn record(
        &self,
        question: &Question,
        choice: &ApprovalChoice,
    ) -> Result<cineris_policy::Recorded, cineris_policy::ApprovalError> {
        let _changing = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let engine = self.get();
        cineris_base::hook::point(CHANGING_APPROVALS);
        engine.record(question, choice)
    }

    /// Takes back the session's approvals `take_back`, then allows `add`
    /// for the session, in the engine in force and in one change of it:
    /// the rules for reading the folders of the session's skills, which
    /// follow the skills found (step 28). Each rule is added as "allow for
    /// this session" adds one ([`Engine::record`]), and taken back as one
    /// is ([`Engine::revoke`]): it is a session rule, listed with the
    /// session's approvals and kept over a change of level, and nothing is
    /// written. The rules are added as one answer, so that the set is
    /// compiled again once, however many skills there are. A rule an
    /// approval already holds is not added again, and is not claimed: it is
    /// not the skills' to take back. Says the id of
    /// the approval made for each of `add`, or `None`. Waits for a person's
    /// answer being stored for the project: call this off the core thread.
    #[cfg(feature = "scripts")]
    pub(crate) fn follow_reads(
        &self,
        add: &[cineris_protocol::lease::Rule],
        take_back: &[cineris_protocol::ids::ApprovalId],
    ) -> Vec<Option<cineris_protocol::ids::ApprovalId>> {
        let _changing = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let engine = self.get();
        cineris_base::hook::point(CHANGING_APPROVALS);
        if !take_back.is_empty() {
            // Session approvals alone: no file is written, and nothing fails.
            let _ = engine.revoke(take_back);
        }
        let held = engine.approvals();
        let new: Vec<cineris_protocol::lease::Rule> = add
            .iter()
            .filter(|rule| !held.iter().any(|approval| approval.rule == **rule))
            .cloned()
            .collect();
        // One answer for them all: the rules are compiled again once.
        let made = if new.is_empty() {
            Vec::new()
        } else {
            let question = Question {
                reasons: Vec::new(),
                lifts: new,
                storable: false,
                managed: false,
            };
            engine
                .record(&question, &ApprovalChoice::AllowSession)
                .map(|recorded| recorded.added)
                .unwrap_or_default()
        };
        add.iter()
            .map(|rule| {
                made.iter()
                    .find(|approval| approval.rule == *rule)
                    .map(|approval| approval.id.clone())
            })
            .collect()
    }
}

/// The rule that lets the session read `folder` and everything below it,
/// unasked: a skill's folder where it really is, as the skills were found
/// (step 28). It is written as the engine writes the lift for a read of one
/// path: the folder's name in the file system's own spelling, as a
/// resolved path has it and as an allow rule is matched, every character a
/// path glob reads as a pattern taken as itself, then `/**`. `None` when
/// the pattern would not compile, which no name is known to make: a rule
/// that did not compile would leave the session's approvals as they are
/// from then on, since each later one compiles the set again and keeps the
/// rules in force when that fails.
#[cfg(feature = "scripts")]
pub(crate) fn reading_below(folder: &AbsPath) -> Option<cineris_protocol::lease::Rule> {
    let mut pattern = String::with_capacity(folder.as_str().len() + 3);
    for character in folder.as_str().chars() {
        match character {
            '*' | '?' | '[' | '{' | '}' => {
                pattern.push('[');
                pattern.push(character);
                pattern.push(']');
            }
            ']' => pattern.push_str("[]]"),
            other => pattern.push(other),
        }
    }
    pattern.push_str("/**");
    let anchors = cineris_policy::glob::Anchors {
        workspace: folder,
        home: None,
    };
    cineris_policy::glob::PathGlob::compile(&pattern, anchors, cineris_policy::glob::Case::Exact)
        .ok()?;
    Some(cineris_protocol::lease::Rule {
        effect: EffectKind::Read,
        pattern,
        action: RuleAction::Allow,
    })
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
    /// What the session's engine was made from, as a change of level
    /// leaves it: what a supervisor's grant is judged with.
    template: Arc<Mutex<EngineTemplate>>,
    timer: SharedTimer,
    /// How long an approval may wait for its answer. `None`: as long as it
    /// takes.
    timeout: Option<Duration>,
    fail_on_denial: bool,
    /// The front end can stop the turn from a question: the interactive
    /// application's interrupt key (`CoreOptions::answers_may_stop`).
    stops_from_questions: bool,
    /// A question in a session with no client at all waits for a `user`
    /// client to open it, rather than being refused at once: the core
    /// keeps sessions (`CoreOptions::keep_sessions`).
    waits_for_a_client: bool,
    /// Whether commands run confined: what an approval's detail says. It
    /// follows the session's level, since nothing is confined at
    /// `unrestricted`.
    sandboxed: Arc<std::sync::atomic::AtomicBool>,
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
    pub(crate) template: Arc<Mutex<EngineTemplate>>,
    pub(crate) timer: SharedTimer,
    pub(crate) timeout: Option<Duration>,
    pub(crate) fail_on_denial: bool,
    pub(crate) stops_from_questions: bool,
    pub(crate) waits_for_a_client: bool,
    pub(crate) sandboxed: bool,
}

/// The project's files were looked at for a command, which is not
/// decided yet (a point a test can stop at: [`cineris_base::hook`]).
pub const AFTER_THE_LOOK: &str = "policy: the look before a command is over, and it is not decided";

/// An approval's question was looked at for who may answer it, and its
/// request is not open yet: a client that opens the session now finds no
/// request to be put, and is put it once it is open (the review of step
/// 27's core changes). An awaited point ([`cineris_base::hook::await_point!`]),
/// on the turn's task.
pub const BEFORE_THE_APPROVAL_IS_OPEN: &str =
    "policy: who may answer an approval was looked at, and its request is not open";

/// A change of level has made its engine, with the approvals of the engine
/// in force as they were then, and written its lease, and has not put the
/// engine in force: a revoke or an answer recorded now must not be lost to
/// it. An awaited point ([`cineris_base::hook::await_point!`]): the change
/// runs on the core's task, which a test of the core runs on its own
/// thread, and a blocking point there would stop the revoke too.
pub const BEFORE_THE_NEW_LEVEL_IS_IN_FORCE: &str = "policy: a change of level has made its engine and written its lease, and the engine is not \
     in force";

impl EnginePolicy {
    pub(crate) fn new(parts: PolicyParts) -> Self {
        Self {
            in_force: Arc::new(EngineInForce::new(parts.engine)),
            allowed: Mutex::new(Vec::new()),
            clients: parts.clients,
            sink: parts.sink,
            hooks: parts.hooks,
            template: parts.template,
            timer: parts.timer,
            timeout: parts.timeout,
            fail_on_denial: parts.fail_on_denial,
            stops_from_questions: parts.stops_from_questions,
            waits_for_a_client: parts.waits_for_a_client,
            sandboxed: Arc::new(std::sync::atomic::AtomicBool::new(parts.sandboxed)),
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

    /// Stops turn `turn` during a call's run: a confined command changed
    /// protected paths, and the user is to look before anything else runs;
    /// or (since step 28) an extension's host call was answered with a stop,
    /// which a call of the round's own would have ended the turn with.
    /// Every call of it that has not started is decided again, and is
    /// answered with the stop. The call whose run stopped it marks its
    /// result so (`ToolResult::stopping_turn`), and the round ends at that
    /// call: the model is not asked again, even when it was the last call
    /// of its response.
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
        let number = cineris_base::IdCounter::new("t_").number_of(call.turn.as_str())?;
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
        learned: Vec<cineris_protocol::lease::Rule>,
    ) -> Result<(), Vec<cineris_policy::RuleError>> {
        self.in_force.replace_with(engine, learned)
    }

    /// Gives the engine in force the session's learned commands
    /// ([`EngineInForce::set_learned`]).
    pub(crate) fn set_learned(
        &self,
        rules: Vec<cineris_protocol::lease::Rule>,
    ) -> Result<(), Vec<cineris_policy::RuleError>> {
        self.in_force.set_learned(rules)
    }

    /// Takes approvals back from the engine in force, by id: never from one
    /// a change of level is replacing ([`EngineInForce::revoke`]). Writing
    /// the project's file blocks: call this off the core thread.
    pub(crate) fn revoke(
        &self,
        ids: &[cineris_protocol::ids::ApprovalId],
    ) -> Result<usize, cineris_policy::ApprovalError> {
        self.in_force.revoke(ids)
    }

    /// The session's rules for reading its skills' folders, followed
    /// ([`EngineInForce::follow_reads`]). Call this off the core thread.
    #[cfg(feature = "scripts")]
    pub(crate) fn follow_reads(
        &self,
        add: &[cineris_protocol::lease::Rule],
        take_back: &[cineris_protocol::ids::ApprovalId],
    ) -> Vec<Option<cineris_protocol::ids::ApprovalId>> {
        self.in_force.follow_reads(add, take_back)
    }

    /// Which of `ids` the engine in force holds no approval of: taken back
    /// since they were made, as `permissions/revoke` takes one back.
    #[cfg(feature = "scripts")]
    pub(crate) fn not_held(
        &self,
        ids: &[cineris_protocol::ids::ApprovalId],
    ) -> Vec<cineris_protocol::ids::ApprovalId> {
        let held = self.engine().approvals();
        ids.iter()
            .filter(|id| !held.iter().any(|approval| approval.id == **id))
            .cloned()
            .collect()
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
    pub(crate) fn index_gate(self: &Arc<Self>) -> Arc<dyn cineris_index::Gate> {
        let policy = Arc::downgrade(self);
        Arc::new(InForce {
            now: Box::new(move || policy.upgrade().map(|policy| policy.engine())),
            ask: Engine::may_store,
        })
    }

    pub(crate) fn outcome(&self, call: &PlannedCall<'_>) -> Outcome {
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
            cineris_base::hook::point(AFTER_THE_LOOK);
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

    /// What judges a supervisor's allow of `call`: whether its grant
    /// allows the call ([`within_grant`]), with the session's engine
    /// template, its standing lease and whether its commands are confined
    /// as they are **when the answer is judged**, not when the question was
    /// put: a change of level while the request is open reaches it (the
    /// review of step 27's second pull request). `None` for a call whose
    /// effects the engine cannot be shown: no grant covers it.
    fn judge(&self, call: &PlannedCall<'_>) -> Option<Judge> {
        let accesses = accesses(call)?;
        let template = Arc::clone(&self.template);
        let in_force = Arc::clone(&self.in_force);
        let sandboxed = Arc::clone(&self.sandboxed);
        let tool = call.spec.name.clone();
        let default = call.spec.permission;
        Some(Arc::new(move |grant: &Lease| {
            let parts = template
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .parts
                .clone();
            let standing = in_force.get().lease().clone();
            let confined = sandboxed.load(std::sync::atomic::Ordering::Acquire);
            within_grant(
                &Session {
                    template: &parts,
                    standing: &standing,
                    confined,
                },
                grant,
                &tool,
                default,
                &accesses,
            )
        }))
    }

    /// What of `grant` does not apply in this session as it is now, and
    /// why, in a sentence for the session's clients ([`limits_in`]).
    pub(crate) fn grant_limits(&self, grant: &Lease) -> Option<String> {
        let parts = self
            .template
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .parts
            .clone();
        let standing = self.engine().lease().clone();
        let confined = self.sandboxed.load(std::sync::atomic::Ordering::Acquire);
        limits_in(
            &Session {
                template: &parts,
                standing: &standing,
                confined,
            },
            grant,
        )
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

/// The session a supervisor's grant is judged in, as it is when the answer
/// comes.
pub(crate) struct Session<'a> {
    /// What the session's engine is made from.
    pub(crate) template: &'a cineris_policy::Parts,
    /// Its standing lease.
    pub(crate) standing: &'a Lease,
    /// Whether its commands run confined now: not at `unrestricted`, nor
    /// where the sandbox does not enforce.
    pub(crate) confined: bool,
}

/// Whether a supervisor's `grant` allows a call of `tool` with these
/// `accesses` in `session` (`docs/architecture/06-permissions-and-sandbox.md`,
/// "When a session is supervised").
///
/// A grant that has expired covers nothing ([`Judged::Expired`]): its
/// expiry `at` a moment the core's clock has passed (one that cannot be
/// read has passed), or a `time` to stop that has. Its other conditions to
/// stop (failed verification, spending, a step out of scope) are the
/// holder's to report, which a supervisor's grant has no one to do here:
/// they are not judged.
///
/// Otherwise an engine is made from the session's own template with a
/// lease of:
///
/// - **the session's scope, narrowed by the grant:** `standing.narrow(grant)`,
///   so the writable roots, the protected paths, the network and every
///   `deny` and `ask` rule are the session's or narrower, and a grant never
///   widens what the session holds. A narrow that fails covers nothing.
/// - **who decides, from the grant:** its level, at most `autonomous`
///   (`unrestricted` is never handed on; `guided`, not there yet, is read
///   as `supervised`), and its allow rules. Narrowed
///   alone, both would be the session's, under which the call was asked
///   about, and no grant could cover any call the session asks about.
/// - **no allow of a path:** a grant never lifts a question about a path (one
///   beyond the writable roots, one that looks like a secret, a protected
///   path): its `read` and `write` allow rules are not used, and such a call
///   stays with the users. An allow of a command lifts no question about a
///   protected path or a secret that a command line names, which the engine
///   asks about whatever allows the command.
/// - **a command within only where it runs in the sandbox it was judged
///   in:** at `autonomous` a command is the holder's because the sandbox
///   confines it, and a command an allow rule names writes where the
///   sandbox lets it (the engine asks only about the protected and secret
///   paths a command line names, and leaves the rest to the sandbox); but
///   it runs in the session's sandbox, not the narrowed one. So where the
///   narrowed scope (read, write, protected, network) is not the session's
///   own, or the session's commands are not confined, the level is at most
///   `supervised` and the grant's `exec` allow rules are not used either: a
///   command stays with the users (the review of step 27's second pull
///   request found `cp x ~/.bashrc` covered there).
///
/// It decides the call, and only `allow` is within. The session's stored
/// and session approvals are not in it: they are the user's, and the call
/// was asked about despite them.
pub(crate) fn within_grant(
    session: &Session<'_>,
    grant: &Lease,
    tool: &str,
    default: RuleAction,
    accesses: &[Access],
) -> Judged {
    let engine = match grant_engine(session, grant) {
        Ok(made) => made.engine,
        Err(Unmade::Expired) => return Judged::Expired,
        Err(Unmade::Not(_)) => return Judged::Beyond,
    };
    match engine.decide(&Request {
        tool,
        default,
        accesses,
    }) {
        Outcome::Allow { .. } => Judged::Within,
        _ => Judged::Beyond,
    }
}

/// The engine a supervisor's grant is judged with in a session
/// ([`within_grant`]), with what of the grant it leaves out and why: made
/// in one place, so that what judges an answer and what the supervisor is
/// told of its grant ([`limits_in`]) never differ.
struct GrantEngine {
    engine: Engine,
    /// Whether a command runs confined in the session's own scope.
    confined_here: bool,
    /// Whether the session's commands are confined at all.
    confined: bool,
}

/// Why a grant covers nothing in a session.
enum Unmade {
    /// It has expired.
    Expired,
    /// Its lease cannot be narrowed to the session's, or its engine cannot
    /// be made: why, in words.
    Not(String),
}

/// The level who decides by under a grant: the grant's own, at most
/// `autonomous` (`unrestricted` is never handed on) and never `guided`,
/// which is not there yet (it is read as `supervised`); and at most
/// `supervised` where a command would not run confined in the session's own
/// scope.
fn grant_level(grant: &Lease, confined_here: bool) -> Level {
    let level = cineris_policy::level::lower(&grant.grant.level, &Level::Autonomous);
    let level = if level == Level::Guided {
        Level::Supervised
    } else {
        level
    };
    if confined_here {
        level
    } else {
        cineris_policy::level::lower(&level, &Level::Supervised)
    }
}

/// Whether an allow rule of a grant is left out where the session's
/// commands run as `confined_here` says: an allow of a path always, and an
/// allow of a command where a command would not run confined in the
/// session's own scope.
fn left_out(rule: &cineris_protocol::lease::Rule, confined_here: bool) -> bool {
    rule.action == RuleAction::Allow
        && (matches!(rule.effect, EffectKind::Read | EffectKind::Write)
            || (rule.effect == EffectKind::Exec && !confined_here))
}

/// Makes the engine a supervisor's `grant` is judged with in `session`
/// ([`within_grant`] says how), or says why there is none.
fn grant_engine(session: &Session<'_>, grant: &Lease) -> Result<GrantEngine, Unmade> {
    let Session {
        template,
        standing,
        confined,
    } = *session;
    if expired(grant, template.clock.now()) {
        return Err(Unmade::Expired);
    }
    let mut lease = standing
        .narrow(grant, template.sandbox)
        .map_err(|failure| {
            Unmade::Not(format!(
                "it cannot be narrowed to this session's lease ({failure})"
            ))
        })?;
    // Whether a command the grant lets run runs in the sandbox it was
    // judged in: the session's own scope, confined.
    let confined_here = confined && same_scope(&lease.grant.sandbox, &standing.grant.sandbox);
    lease.grant.level = grant_level(grant, confined_here);
    // No allow of a path; and no allow of a command where it would not run
    // confined in the session's own scope, since the engine leaves a
    // command's writes beyond the roots to the sandbox.
    lease
        .grant
        .rules
        .retain(|rule| !left_out(rule, confined_here));
    for rule in &grant.grant.rules {
        if rule.action == RuleAction::Allow
            && !left_out(rule, confined_here)
            && !lease.grant.rules.contains(rule)
        {
            lease.grant.rules.push(rule.clone());
        }
    }
    let mut parts = template.clone();
    parts.level = lease.grant.level;
    parts.rules = lease
        .grant
        .rules
        .into_iter()
        .map(|rule| cineris_policy::SourcedRule::new(rule, cineris_policy::RuleSource::Lease))
        .collect();
    parts.approved = Vec::new();
    parts.sandbox_policy = lease.grant.sandbox;
    parts.models = lease.grant.models;
    parts.budget = lease.grant.budget;
    parts.limits = lease.grant.limits;
    parts.store = None;
    let engine = Engine::start(parts)
        .map_err(|failure| Unmade::Not(format!("its engine cannot be made from it ({failure})")))?;
    Ok(GrantEngine {
        engine,
        confined_here,
        confined,
    })
}

/// What of a supervisor's `grant` does not apply in `session`, and why, as
/// a sentence: so that the supervisor is not left to find out by its
/// answers being refused. `None` when all of it applies. Said, from the
/// engine that judges its answers ([`grant_engine`]): a grant that has
/// expired, cannot be narrowed to the session or whose engine cannot be
/// made, with why (it covers nothing); its `read` and `write` allow rules
/// (they never apply); its `exec` allow rules, and a level that would let a
/// command run unasked, where a command would not run confined in the
/// session's own scope (no enforcing sandbox, `unrestricted`, or a
/// narrower scope); a `guided` level, read as `supervised`; and then what
/// does apply of its allow rules of tools and hosts, which are not
/// commands.
pub(crate) fn limits_in(session: &Session<'_>, grant: &Lease) -> Option<String> {
    let id = &grant.id;
    let made = match grant_engine(session, grant) {
        Ok(made) => made,
        Err(Unmade::Expired) => {
            return Some(format!(
                "The supervisor's grant `{id}` has expired: it covers nothing, and every \
                 approval stays with the session's users."
            ));
        }
        Err(Unmade::Not(why)) => {
            return Some(format!(
                "The supervisor's grant `{id}` covers nothing in this session: {why}. Every \
                 approval stays with the session's users."
            ));
        }
    };
    let allows = |effects: &[EffectKind]| -> Vec<String> {
        grant
            .grant
            .rules
            .iter()
            .filter(|rule| rule.action == RuleAction::Allow && effects.contains(&rule.effect))
            .map(|rule| format!("`{} {}`", rule.effect.as_str(), rule.pattern))
            .collect()
    };
    let mut said = Vec::new();
    let paths = allows(&[EffectKind::Read, EffectKind::Write]);
    if !paths.is_empty() {
        said.push(format!(
            "its allow rules of paths ({}) never apply, since a question about a path stays \
             with the users",
            paths.join(", ")
        ));
    }
    if grant.grant.level == Level::Guided {
        said.push(
            "its level `guided` is read as `supervised`, since it is not there yet".to_owned(),
        );
    }
    if !made.confined_here {
        let why = if made.confined {
            "its scope is narrower than the session's, and an approved command runs in the \
             session's sandbox"
        } else {
            "the session's commands are not confined by an enforcing sandbox"
        };
        let commands = allows(&[EffectKind::Exec]);
        if !commands.is_empty() {
            said.push(format!(
                "its allow rules of commands ({}) do not apply, since {why}",
                commands.join(", ")
            ));
        }
        if grant_level(grant, true) == Level::Autonomous {
            said.push(format!(
                "its level lets no command run unasked, since {why}"
            ));
        }
    }
    if said.is_empty() {
        return None;
    }
    let others = allows(&[EffectKind::Ext, EffectKind::Network]);
    let applies = if others.is_empty() {
        String::new()
    } else {
        format!(
            " Its allow rules of tools and hosts ({}) apply: they are not commands.",
            others.join(", ")
        )
    };
    Some(format!(
        "The supervisor's grant `{id}` is limited in this session: {}.{applies} What it does not \
         cover stays with the session's users.",
        said.join("; ")
    ))
}

/// Whether a grant has expired by `now`: its expiry `at` has passed, or a
/// `time` to stop has. A moment that cannot be read has passed.
fn expired(grant: &Lease, now: cineris_base::UtcTime) -> bool {
    let passed = |at: Option<&cineris_protocol::Timestamp>| {
        at.and_then(|at| cineris_base::UtcTime::parse(at.as_str()).ok())
            .is_none_or(|at| at <= now)
    };
    let at = grant.expires.kind == cineris_protocol::lease::ExpiryKind::At
        && passed(grant.expires.at.as_ref());
    at || grant.stop_when.iter().any(|condition| {
        matches!(condition, cineris_protocol::lease::StopCondition::Time(stop)
            if passed(Some(&stop.at)))
    })
}

/// Whether two sandbox policies give a command the same scope: the same
/// paths to read and write, the same protected, and the same network. Path
/// sets are compared by what they hold, each within the other, not by the
/// text of their roots: a root and one inside it hold what the first alone
/// does.
fn same_scope(
    first: &cineris_protocol::lease::SandboxPolicy,
    second: &cineris_protocol::lease::SandboxPolicy,
) -> bool {
    use cineris_protocol::lease::PathSet;
    fn roots(set: &PathSet) -> Option<Vec<AbsPath>> {
        set.roots
            .iter()
            .map(|root| AbsPath::parse(root, cineris_base::PathStyle::NATIVE).ok())
            .collect()
    }
    // Whether every path of `inner` is in `outer`. A root that cannot be
    // read as a path is held by nothing.
    fn within(inner: &PathSet, outer: &PathSet) -> bool {
        if outer.all {
            return true;
        }
        if inner.all {
            return false;
        }
        let (Some(inner), Some(outer)) = (roots(inner), roots(outer)) else {
            return false;
        };
        inner
            .iter()
            .all(|root| outer.iter().any(|holds| root.starts_with_exactly(holds)))
    }
    let same = |a: &PathSet, b: &PathSet| within(a, b) && within(b, a);
    same(&first.read, &second.read)
        && same(&first.write, &second.write)
        && same(&first.protect, &second.protect)
        && first.network == second.network
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
    // What the call touches, wherever it stands among the effects: an
    // `ext` effect names the tool, not a place (step 28 found a host call's
    // question showing its arguments because its `ext` effect came first,
    // and `tool_call`'s plan, which MCP's tools reach through too, puts it
    // first as well).
    let touched = effects
        .iter()
        .find(|effect| !matches!(effect, Effect::Ext { .. }));
    match touched {
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
        self.ask_with(call, None)
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

impl EnginePolicy {
    /// Puts an extension's question to the session's users
    /// (`question/ask`, naming the extension), as an approval is put, and
    /// waits for the answer within the approvals' time limit. `None` when
    /// nobody can be asked, nobody answered in time, or the user declined
    /// to answer. Dropped, it withdraws the question.
    #[cfg(feature = "scripts")]
    pub(crate) async fn put_question(
        &self,
        session: &cineris_protocol::SessionId,
        turn: &TurnId,
        question: String,
        choices: Vec<String>,
        extension: &str,
    ) -> Option<String> {
        let asked = self.clients.approvers(&self.sink.subscribers());
        if asked.is_empty() {
            return None;
        }
        let request = cineris_protocol::methods::QuestionRequest {
            session_id: session.clone(),
            turn_id: Some(turn.clone()),
            question,
            choices,
            extension: Some(extension.to_owned()),
        };
        let (id, mut answer) = self.clients.ask(
            session,
            &asked,
            |_| ServerRequest::Question(request.clone()),
            None,
        );
        let mut open = Asked {
            policy: self,
            id: id.clone(),
            settled: false,
        };
        let mut limit = self.timeout.map(|limit| self.timer.sleep(limit));
        let answered = poll_fn(|cx| {
            if let Poll::Ready(response) = std::pin::Pin::new(&mut answer).poll(cx) {
                return Poll::Ready(response.ok());
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
        let Some(answered) = answered else {
            self.clients.close(&id);
            self.resolved(&id, Resolution::TimedOut);
            return None;
        };
        self.resolved(&id, Resolution::Answered);
        match answered.response {
            ServerResponse::Question(response) => response.answer,
            _ => None,
        }
    }

    /// Puts a planned call's question, as [`Policy::ask`] does. With
    /// `host`, the call is an extension's host call (step 28): only "allow
    /// once" and "deny" are offered, and what it shows and asks is as
    /// `host` says.
    ///
    /// With
    /// `once_only` the question offers only "allow once" and "deny", and
    /// no answer records a rule: what an extension's host call asks (a
    /// rule kept from it would reach past the extension: an
    /// `exec_network` call names no host, so its rule would be
    /// `network *`).
    pub(crate) fn ask_with<'a>(
        &'a self,
        call: PlannedCall<'a>,
        host: Option<HostQuestion>,
    ) -> BoxFuture<'a, Verdict> {
        // What the call's `tool.before` hooks said the question must show
        // (step 28): a hook that asked puts the question even where the
        // rules allow the call; a replacement of the arguments is new work
        // nobody chose, offered only "allow once" and "deny", and answered
        // only by a person (a `user` client), never by a supervisor's grant
        // (the early review of the hook seam, M1).
        let hooked = call.hooks;
        let replaced = hooked.is_some_and(|hooks| hooks.replaced.is_some());
        let once_only = host.is_some() || replaced;
        Box::pin(async move {
            if let Some(reason) = self.stopped(&call) {
                return Verdict::Stop { reason };
            }
            // Decided again: an answer to an earlier question of the round
            // may have settled this one.
            let mut question = match self.outcome(&call) {
                Outcome::Allow { read_only: true }
                    if host.as_ref().is_some_and(|host| host.read_only_asks) =>
                {
                    Question {
                        reasons: vec![READ_ONLY_IN_ANOTHER_SHELL.to_owned()],
                        lifts: Vec::new(),
                        storable: false,
                        managed: false,
                    }
                }
                // A hook's question is put whatever the rules say: they had
                // nothing to ask, and nothing to add.
                Outcome::Allow { .. } if hooked.is_some() => Question {
                    reasons: Vec::new(),
                    lifts: Vec::new(),
                    storable: false,
                    managed: false,
                },
                Outcome::Allow { .. } => return Verdict::Allow,
                Outcome::Deny { reason } => return Verdict::Deny { note: Some(reason) },
                Outcome::Ask(question) => question,
            };
            if once_only {
                // Nothing to add: only "allow once" and "deny" are
                // offered, and whatever a client answers keeps no rule.
                question.lifts.clear();
            }
            let subscribers = self.sink.subscribers();
            // A replacement is put to the session's users alone.
            let people = |clients: Vec<u64>| -> Vec<u64> {
                if !replaced {
                    return clients;
                }
                clients
                    .into_iter()
                    .filter(|client| {
                        self.clients
                            .info(crate::clients::ClientId(*client))
                            .is_some_and(|info| {
                                info.role == cineris_protocol::methods::ClientRole::User
                            })
                    })
                    .collect()
            };
            let asked = people(self.clients.approvers(&subscribers));
            // A session kept with no client at all puts the question to
            // nobody: a `user` client that opens the session is put it
            // (`Clients::join`), until the time limit. One whose clients
            // are there and answer no approvals is answered by the default
            // at once, as always.
            let waits = self.waits_for_a_client && subscribers.is_empty();
            if asked.is_empty() && !waits {
                return self.nobody(&call, &question);
            }
            cineris_base::hook::await_point!(BEFORE_THE_APPROVAL_IS_OPEN);

            let request = ApprovalRequest {
                session_id: call.session.clone(),
                turn_id: call.turn.clone(),
                call_id: call.call_id.clone(),
                tool: call.spec.name.clone(),
                summary: call.plan.summary.clone(),
                detail: detail(
                    &call,
                    host.as_ref()
                        .and_then(|host| host.confined)
                        .unwrap_or_else(|| {
                            self.sandboxed.load(std::sync::atomic::Ordering::Acquire)
                        }),
                ),
                choices: question.choices(),
                // The engine's reasons, then what the host call adds: a
                // program's input, which no rule judges; then a line for
                // each hook that asked and one for a replacement, so that a
                // client that reads neither `hooks` nor `replaced` sees why
                // it is asked, and that the call is not the model's.
                reasons: question
                    .reasons
                    .iter()
                    .cloned()
                    .chain(host.as_ref().and_then(|host| host.extra.clone()))
                    .chain(
                        hooked
                            .map(cineris_agent::HookQuestion::reasons)
                            .unwrap_or_default(),
                    )
                    .collect(),
                adds: question
                    .lifts
                    .iter()
                    .map(|rule| cineris_protocol::methods::AddedRule {
                        effect: rule.effect.clone(),
                        pattern: rule.pattern.clone(),
                    })
                    .collect(),
                hooks: hooked.map(|hooks| hooks.asked.clone()).unwrap_or_default(),
                replaced: hooked
                    .and_then(|hooks| hooks.replaced.clone())
                    .map(Box::new),
            };
            // No grant answers a replacement: a supervisor's allow of it is
            // not taken, and the request stays open for the users.
            let judge = if replaced { None } else { self.judge(&call) };
            let (id, mut answer) = self.clients.ask(
                call.session,
                &asked,
                |_| ServerRequest::Approval(request.clone()),
                judge,
            );
            // A client that opened the session from another thread between
            // the look at who may answer and the request being open found
            // nothing to be put (`Clients::join`), and is put it now; one
            // that opens it after this is put it by its own join. Found by
            // the review of step 27's core changes: with `keep_sessions` the
            // client that opens a session with nobody on it is the only one
            // who can answer.
            for client in people(self.clients.approvers(&self.sink.subscribers())) {
                if !asked.contains(&client) {
                    self.clients
                        .join(call.session, crate::clients::ClientId(client));
                }
            }
            let mut open = Asked {
                policy: self,
                id: id.clone(),
                settled: false,
            };
            let mut limit = self.timeout.map(|limit| self.timer.sleep(limit));
            let answered: Option<(ApprovalResponse, bool)> = poll_fn(|cx| {
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

            let Some((response, by_supervisor)) = answered else {
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
            // A supervisor's answer settles this call alone: an allow within
            // its grant (taken as `allow_once`), or a denial. Nothing is
            // recorded for it.
            if by_supervisor {
                // A replacement is a person's to allow, whatever a grant
                // would cover; a supervisor's denial of it stands.
                return if response.choice == ApprovalChoice::AllowOnce && !replaced {
                    if !once_only {
                        self.remember_allowed(&call, question);
                    }
                    Verdict::Allow
                } else {
                    Verdict::Deny {
                        note: response.note,
                    }
                };
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
                // question a second time. A host call is never decided
                // again, and what it was allowed is not remembered: the
                // next host call of the same tool call asking the same
                // question would be allowed by it, unasked (the review of
                // step 28).
                if !once_only {
                    self.remember_allowed(&call, asked);
                }
                Verdict::Allow
            } else {
                Verdict::Deny {
                    note: response.note,
                }
            }
        })
    }
}

/// How an extension's host call is put to a person (step 28).
#[derive(Debug, Clone, Default)]
pub(crate) struct HostQuestion {
    /// For a program: whether the extension's own launcher confines it.
    /// Its question says so, whatever the session's commands do: at
    /// `unrestricted` the shell's commands run unconfined and an
    /// extension's programs do not.
    pub(crate) confined: Option<bool>,
    /// Whether a call the engine allows as read-only is asked all the same:
    /// a program's line judged in a shell whose own commands and aliases
    /// are not the program that runs (in PowerShell `write` is
    /// `Write-Output`; the program is `write.exe`), or, on Windows, whose
    /// lookup is not the one that finds the program.
    pub(crate) read_only_asks: bool,
    /// One more reason the question gives, after the engine's: for a
    /// program given input, what the input is, which no rule judges.
    pub(crate) extra: Option<String>,
}

/// Why a program the engine judged read-only is asked about.
pub(crate) const READ_ONLY_IN_ANOTHER_SHELL: &str = "The program's line was judged in the \
     session's shell, whose own commands and aliases are not the program that runs (and on \
     Windows whose lookup is not the one that finds it), so what that shell calls read-only is \
     asked about.";

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

impl<T: Send + Sync + 'static> cineris_index::Gate for InForce<T> {
    fn may_index(&self, path: &AbsPath) -> bool {
        (self.now)().is_some_and(|in_force| (self.ask)(&in_force, path))
    }

    fn for_pass(&self) -> Option<Arc<dyn cineris_index::Gate>> {
        let taken = (self.now)();
        let ask = self.ask;
        Some(Arc::new(move |path: &AbsPath| {
            taken.as_deref().is_some_and(|in_force| ask(in_force, path))
        }))
    }
}

/// An approval's answer, and whether a supervisor gave it, from the
/// table's answers of every kind: the table refuses an answer of another
/// kind, so nothing else arrives here.
fn approval_of(answer: Answer) -> Option<(ApprovalResponse, bool)> {
    match answer.response {
        ServerResponse::Approval(response) => Some((response, answer.by_supervisor)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cineris_base::PathStyle;
    use cineris_index::Gate;
    use cineris_policy::{Access, Outcome, Request};

    use super::*;

    /// A skill's rule compiles whatever its folder is named, and matches
    /// its folder and what is below it alone: names that hold what a path
    /// glob reads as a pattern (a brace open or closed, a set, a negated
    /// set, `**`, a comma), on both systems' paths.
    #[cfg(feature = "scripts")]
    #[test]
    fn a_skills_rule_compiles_and_matches_its_own_folder_alone() {
        use cineris_policy::glob::{Anchors, Case, PathGlob};
        let names = [
            "{a", "a}", "{a,b}", "]", "[!x", "[a]", "**", "a,b", "*", "?", "plain",
        ];
        let mut checked = 0;
        for (base, style) in [
            ("/w/skills", PathStyle::Posix),
            ("C:\\w\\skills", PathStyle::Windows),
        ] {
            let base = AbsPath::parse(base, style).unwrap();
            for name in names {
                // `*` and `?` cannot be in a name on Windows.
                let Ok(folder) = base.join(name) else {
                    assert_eq!(style, PathStyle::Windows, "{name}");
                    continue;
                };
                let rule = reading_below(&folder).expect(name);
                let anchors = Anchors {
                    workspace: &base,
                    home: None,
                };
                let glob = PathGlob::compile(&rule.pattern, anchors, Case::Exact).unwrap();
                let below = folder.join("SKILL.md").unwrap();
                let deep = folder.join("a").unwrap().join("b.md").unwrap();
                for inside in [&folder, &below, &deep] {
                    assert!(glob.matches(inside), "{}: {inside}", rule.pattern);
                }
                for other in names.iter().chain(&["x", "a", "b", "w"]) {
                    for beside in [other.to_string(), format!("{name}x"), format!("x{name}")] {
                        let Ok(beside) = base.join(&beside) else {
                            continue;
                        };
                        if beside == folder {
                            continue;
                        }
                        let file = beside.join("f").unwrap();
                        assert!(
                            !glob.matches(&beside) && !glob.matches(&file),
                            "{}: {beside}",
                            rule.pattern
                        );
                    }
                }
                assert!(!glob.matches(&base), "{}", rule.pattern);
                checked += 1;
            }
        }
        // Every name on Posix paths; on Windows' those without `*` or `?`.
        let not_on_windows = names
            .iter()
            .filter(|name| name.contains(['*', '?']))
            .count();
        assert_eq!(checked, 2 * names.len() - not_on_windows);
    }

    /// An approval names what a call touches wherever its `ext` effect
    /// stands: a host call's plan, and `tool_call`'s, which MCP's tools
    /// reach through too, put one beside the place (step 28).
    #[test]
    fn an_approval_names_what_a_call_touches_wherever_its_ext_effect_stands() {
        let (session, turn) = (cineris_protocol::SessionId::new("s_1"), TurnId::new("t_1"));
        let (call_id, item) = (CallId::new("c_1"), ItemId::new("i_1"));
        let spec = cineris_tools::ToolSpec::new("probe/get", "", serde_json::json!({}));
        let arguments = serde_json::json!({"url": "https://api.example/x"});
        let ext = Effect::Ext {
            tool: "probe/get".to_owned(),
        };
        let host = Effect::Network(NetworkTarget::Host("api.example".to_owned()));
        for effects in [vec![ext.clone(), host.clone()], vec![host, ext.clone()]] {
            let mut plan = cineris_tools::Plan::new("get", ());
            plan.effects = effects;
            let call = PlannedCall {
                session: &session,
                turn: &turn,
                call_id: &call_id,
                item: &item,
                spec: &spec,
                arguments: &arguments,
                plan: &plan,
                preview: None,
                hooks: None,
            };
            assert_eq!(
                detail(&call, false),
                ApprovalDetail::Target(TargetDetail {
                    effect: EffectKind::Network,
                    target: "api.example".to_owned(),
                })
            );
        }
        // A call of a tool that touches nothing it declares is shown by
        // its arguments.
        let mut plan = cineris_tools::Plan::new("get", ());
        plan.effects = vec![ext];
        let call = PlannedCall {
            session: &session,
            turn: &turn,
            call_id: &call_id,
            item: &item,
            spec: &spec,
            arguments: &arguments,
            plan: &plan,
            preview: None,
            hooks: None,
        };
        assert_eq!(
            detail(&call, false),
            ApprovalDetail::Tool(ToolDetail { arguments })
        );
    }

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
        cineris_policy::test_support::TestParts::at(path("/work/app"))
            .home(path("/users/dev"))
            .engine()
    }

    // ------------------------------------------------- a supervisor's grant

    /// A path of the system the test runs on, written as on Linux: on
    /// Windows, on drive `C:`. A grant's scope is narrowed with the system's
    /// own style of path, which reads a Linux root as no root on Windows.
    fn native(posix: &str) -> AbsPath {
        if cfg!(windows) {
            AbsPath::parse(&format!("C:{posix}"), PathStyle::Windows).expect("a path")
        } else {
            path(posix)
        }
    }

    /// The parts of a session's engine at `supervised` in `/work/app`, with
    /// `rules` of the user's own; its sandbox enforces.
    fn session_parts(rules: Vec<cineris_protocol::lease::Rule>) -> cineris_policy::Parts {
        cineris_policy::test_support::TestParts::at(native("/work/app"))
            .home(native("/users/dev"))
            .rules(
                rules
                    .into_iter()
                    .map(|rule| {
                        cineris_policy::SourcedRule::new(rule, cineris_policy::RuleSource::User)
                    })
                    .collect(),
            )
            .parts()
    }

    fn rule(
        effect: cineris_protocol::lease::Effect,
        pattern: &str,
        action: RuleAction,
    ) -> cineris_protocol::lease::Rule {
        cineris_protocol::lease::Rule {
            effect,
            pattern: pattern.to_owned(),
            action,
        }
    }

    /// A grant at `level` with `rules`, that writes and reads anywhere: its
    /// scope is narrowed to the session's.
    fn grant(level: Level, rules: Vec<cineris_protocol::lease::Rule>) -> Lease {
        use cineris_protocol::lease::{NetworkAccess, PathSet, SandboxPolicy};
        let everything = PathSet {
            all: true,
            roots: Vec::new(),
        };
        let mut lease = cineris_policy::standing_lease(
            cineris_protocol::ids::LeaseId::new("l_grant"),
            cineris_policy::StandingTerms {
                level: Level::ReadOnly,
                rules,
                sandbox: SandboxPolicy {
                    read: everything.clone(),
                    write: everything,
                    protect: PathSet::default(),
                    network: NetworkAccess::Full,
                },
                models: Vec::new(),
                budget: cineris_protocol::lease::Budget::default(),
                limits: cineris_protocol::lease::HardLimits::default(),
            },
            cineris_policy::SandboxState::Enforcing,
        )
        .expect("a lease");
        lease.grant.level = level;
        lease
    }

    fn command(line: &str) -> Vec<Access> {
        vec![Access::Exec {
            command: line.to_owned(),
            cwd: native("/work/app"),
        }]
    }

    /// What `grant` comes to for `accesses` in a session made from `parts`,
    /// whose commands are `confined` or not.
    fn judged(
        parts: &cineris_policy::Parts,
        confined: bool,
        grant: &Lease,
        accesses: &[Access],
    ) -> Judged {
        let standing = Engine::start(parts.clone())
            .expect("the session's engine")
            .lease()
            .clone();
        within_grant(
            &Session {
                template: parts,
                standing: &standing,
                confined,
            },
            grant,
            "shell",
            RuleAction::Ask,
            accesses,
        )
    }

    /// Whether `grant` covers `accesses` in a session made from `parts`,
    /// whose commands are confined where its sandbox enforces.
    fn covers(parts: &cineris_policy::Parts, grant: &Lease, accesses: &[Access]) -> bool {
        let confined = parts.sandbox == cineris_policy::SandboxState::Enforcing;
        judged(parts, confined, grant, accesses) == Judged::Within
    }

    fn write(at: &str) -> Vec<Access> {
        vec![Access::Write { path: native(at) }]
    }

    fn read(at: &str) -> Vec<Access> {
        vec![Access::Read { path: native(at) }]
    }

    /// A grant never lifts a question about a path, whatever its allow
    /// rules name: a write or a read beyond the session's roots, a file that
    /// looks like a secret, a protected path (built in, or the session's own
    /// `sandbox.protect`). Nor does an allow of a command lift one that its
    /// command line raises. (The review of step 27's second pull request
    /// found a grant's `write` allow taken for a write outside the
    /// workspace.)
    #[test]
    fn a_grant_lifts_no_question_about_a_path() {
        use cineris_protocol::lease::Effect;
        let mut parts = session_parts(Vec::new());
        parts
            .sandbox_policy
            .protect
            .roots
            .push(native("/work/app/vendor").as_str().to_owned());
        for (effect, pattern, accesses) in [
            (
                Effect::Write,
                "/elsewhere/**",
                write("/elsewhere/notes.txt"),
            ),
            (
                Effect::Write,
                "~/elsewhere/**",
                write("/users/dev/elsewhere/notes.txt"),
            ),
            (Effect::Read, "/elsewhere/**", read("/elsewhere/notes.txt")),
            (Effect::Read, "**/.env", read("/work/app/.env")),
            (Effect::Write, "**/.env", write("/work/app/.env")),
            (
                Effect::Write,
                ".git/hooks/**",
                write("/work/app/.git/hooks/pre-commit"),
            ),
            (
                Effect::Write,
                ".cargo/config.toml",
                write("/work/app/.cargo/config.toml"),
            ),
            (Effect::Write, "vendor/**", write("/work/app/vendor/lib.rs")),
        ] {
            for level in [Level::Supervised, Level::Autonomous] {
                let grant = grant(
                    level.clone(),
                    vec![rule(effect.clone(), pattern, RuleAction::Allow)],
                );
                assert!(
                    !covers(&parts, &grant, &accesses),
                    "{pattern} at {level:?} lifted {accesses:?}"
                );
            }
        }
        // A command line that names such a path is asked about whatever
        // allows the command.
        for (pattern, line) in [
            ("cp *", "cp x .git/hooks/pre-commit"),
            ("git config *", "git config core.hooksPath hooks"),
            ("cat *", "cat .env"),
        ] {
            let grant = grant(
                Level::Supervised,
                vec![rule(Effect::Exec, pattern, RuleAction::Allow)],
            );
            assert!(!covers(&parts, &grant, &command(line)), "{line}");
        }
        // The same allow of a command covers one that names no such path.
        let copy = grant(
            Level::Supervised,
            vec![rule(Effect::Exec, "cp *", RuleAction::Allow)],
        );
        assert!(covers(&parts, &copy, &command("cp a.txt b.txt")));
    }

    /// A command is within a grant only where it runs in the sandbox it was
    /// judged in: the session's own scope, confined. There, at `autonomous`
    /// a command is the holder's, and an allow rule of the grant names one.
    /// Where the narrowed scope is not the session's own, or the session's
    /// commands are not confined (at `unrestricted`, or where the sandbox
    /// does not enforce), the engine would leave a command's writes beyond
    /// the narrowed roots to a sandbox that is not there: neither the level
    /// nor an allow rule of the grant covers a command (the second review of
    /// step 27's fixes found `cp x ~/.bashrc` covered by `exec cp *`).
    #[test]
    fn a_command_is_within_only_where_it_runs_in_the_sandbox_it_was_judged_in() {
        use cineris_protocol::lease::{Effect, NetworkAccess};
        let parts = session_parts(Vec::new());
        let anything = grant(Level::Autonomous, Vec::new());
        let make = grant(
            Level::Supervised,
            vec![rule(Effect::Exec, "make", RuleAction::Allow)],
        );
        // In the session's own scope, confined.
        assert!(covers(&parts, &anything, &command("ninja")));
        assert!(covers(&parts, &make, &command("make")));

        // Narrower roots to write.
        let src = |mut grant: Lease| {
            grant.grant.sandbox.write.all = false;
            grant.grant.sandbox.write.roots = vec![native("/work/app/src").as_str().to_owned()];
            grant
        };
        assert!(!covers(&parts, &src(anything.clone()), &command("ninja")));
        assert!(!covers(&parts, &src(make.clone()), &command("make")));
        let copy = grant(
            Level::Supervised,
            vec![rule(Effect::Exec, "cp *", RuleAction::Allow)],
        );
        assert!(covers(&parts, &copy, &command("cp x README.md")));
        assert!(
            !covers(&parts, &src(copy), &command("cp x README.md")),
            "a write outside the narrowed roots"
        );

        // A narrower network: the session's is on, the grant's off.
        let mut online = parts.clone();
        online.sandbox_policy.network = NetworkAccess::Full;
        let mut offline = anything.clone();
        offline.grant.sandbox.network = NetworkAccess::Off;
        assert!(covers(&online, &anything, &command("ninja")));
        assert!(!covers(&online, &offline, &command("ninja")));

        // The session's commands not confined: an allow rule of the grant
        // names the program, and the command writes outside the workspace.
        for (pattern, line) in [
            ("make", "make"),
            ("cp *", "cp x ~/.bashrc"),
            ("tar *", "tar -xf e.tar -C ~/.config"),
            ("sh *", "sh -c 'echo x > ~/.bashrc'"),
            (
                "python3 *",
                "python3 -c \"open('/users/dev/.bashrc','w').write('x')\"",
            ),
            ("rsync *", "rsync -a x /users/dev/"),
            ("sed *", "sed -i s/a/b/ /users/dev/.bashrc"),
            ("rm *", "rm -rf /users/dev"),
            ("git *", "git -C /users/dev/other reset --hard"),
        ] {
            let named = grant(
                Level::Supervised,
                vec![rule(Effect::Exec, pattern, RuleAction::Allow)],
            );
            assert_eq!(
                judged(&parts, false, &named, &command(line)),
                Judged::Beyond,
                "unconfined: {line}"
            );
            assert!(
                !covers(&parts, &src(named), &command(line)),
                "narrowed: {line}"
            );
        }
        assert_eq!(
            judged(&parts, false, &anything, &command("ninja")),
            Judged::Beyond
        );
    }

    /// What the supervisor is told of `grant` in a session made from
    /// `parts`, whose commands are `confined` or not.
    fn told(parts: &cineris_policy::Parts, confined: bool, grant: &Lease) -> Option<String> {
        let standing = Engine::start(parts.clone())
            .expect("the session's engine")
            .lease()
            .clone();
        limits_in(
            &Session {
                template: parts,
                standing: &standing,
                confined,
            },
            grant,
        )
    }

    /// A grant whose engine cannot be made covers nothing, and the
    /// supervisor is told so, with why: a rule of an effect this build does
    /// not know, a pattern that cannot be read (the review of step 27's
    /// second pull request found such a grant covering nothing while the
    /// notice said nothing, or named only its commands).
    #[test]
    fn a_grant_whose_engine_cannot_be_made_covers_nothing_and_says_why() {
        use cineris_protocol::lease::Effect;
        let parts = session_parts(Vec::new());
        let make = rule(Effect::Exec, "make", RuleAction::Allow);
        for (case, rules) in [
            (
                "an unknown effect",
                vec![
                    make.clone(),
                    rule(
                        Effect::Unknown("teleport".to_owned()),
                        "*",
                        RuleAction::Deny,
                    ),
                ],
            ),
            (
                "an empty command",
                vec![rule(Effect::Exec, "", RuleAction::Allow)],
            ),
            (
                "a path pattern that cannot be read",
                vec![
                    make.clone(),
                    rule(Effect::Write, "a/**b/[", RuleAction::Deny),
                ],
            ),
        ] {
            let grant = grant(Level::Supervised, rules);
            assert_eq!(
                judged(&parts, true, &grant, &command("make")),
                Judged::Beyond,
                "{case}"
            );
            let told = told(&parts, true, &grant).unwrap_or_default();
            assert!(
                told.contains("covers nothing") && told.contains("cannot be made"),
                "{case}: {told}"
            );
        }
    }

    /// `guided` is not there yet: a grant at it is read as `supervised`,
    /// in what it covers and in what it is told.
    #[test]
    fn a_guided_grant_is_read_as_supervised() {
        use cineris_protocol::lease::Effect;
        let parts = session_parts(Vec::new());
        let guided = grant(
            Level::Guided,
            vec![rule(Effect::Exec, "make", RuleAction::Allow)],
        );
        assert!(covers(&parts, &guided, &command("make")));
        assert!(!covers(&parts, &guided, &command("ninja")));
        let told = told(&parts, true, &guided).expect("told");
        assert!(told.contains("`guided` is read as `supervised`"), "{told}");
    }

    /// Scopes are compared by what they hold, each within the other, not by
    /// the text of their roots: a root beside one inside it is the session's
    /// own scope, and so is the same root spelled another way.
    #[test]
    fn a_scope_is_compared_by_what_it_holds() {
        let parts = session_parts(Vec::new());
        let mut nested = grant(Level::Autonomous, Vec::new());
        nested.grant.sandbox.write.all = false;
        nested.grant.sandbox.write.roots = parts
            .sandbox_policy
            .write
            .roots
            .iter()
            .cloned()
            .chain([native("/work/app/src").as_str().to_owned()])
            .collect();
        assert!(covers(&parts, &nested, &command("ninja")));
        let mut spelled = grant(Level::Autonomous, Vec::new());
        spelled.grant.sandbox.write.all = false;
        spelled.grant.sandbox.write.roots = parts
            .sandbox_policy
            .write
            .roots
            .iter()
            .map(|root| format!("{root}/./"))
            .collect();
        assert!(covers(&parts, &spelled, &command("ninja")));
        assert_eq!(told(&parts, true, &spelled), None);
    }

    /// Where a command would not run confined, the supervisor is told what
    /// of its grant does not apply, and what does: its allow rules of tools
    /// and hosts, which are not commands.
    #[test]
    fn the_supervisor_is_told_what_of_its_grant_applies_unconfined() {
        use cineris_protocol::lease::Effect;
        let parts = session_parts(Vec::new());
        let grant = grant(
            Level::Supervised,
            vec![
                rule(Effect::Exec, "make", RuleAction::Allow),
                rule(Effect::Ext, "fs_*", RuleAction::Allow),
                rule(Effect::Network, "example.com", RuleAction::Allow),
            ],
        );
        let told = told(&parts, false, &grant).expect("told");
        for part in [
            "`exec make`",
            "not confined",
            "`ext fs_*`",
            "`network example.com`",
            "apply",
        ] {
            assert!(told.contains(part), "{part} in {told}");
        }
        assert_eq!(
            judged(
                &parts,
                false,
                &grant,
                &[Access::Ext {
                    tool: "fs_write".to_owned()
                }]
            ),
            Judged::Within
        );
    }

    /// A grant that has expired covers nothing, and says so: its `at` has
    /// passed by the core's clock, or a `time` to stop has, or the moment
    /// cannot be read. One that has not expired covers what it allows.
    #[test]
    fn an_expired_grant_covers_nothing() {
        use cineris_protocol::lease::{Effect, ExpiryKind, StopCondition, TimeStop};
        let parts = session_parts(Vec::new());
        let now = parts.clock.now();
        let moment = |seconds: i64| {
            let at = if seconds < 0 {
                now.checked_sub(std::time::Duration::from_secs(seconds.unsigned_abs()))
            } else {
                now.checked_add(std::time::Duration::from_secs(seconds.unsigned_abs()))
            };
            cineris_protocol::Timestamp::from(at.expect("a moment").to_string().as_str())
        };
        let make = || {
            grant(
                Level::Supervised,
                vec![rule(Effect::Exec, "make", RuleAction::Allow)],
            )
        };
        let at = |at: Option<cineris_protocol::Timestamp>| {
            let mut grant = make();
            grant.expires.kind = ExpiryKind::At;
            grant.expires.at = at;
            judged(&parts, true, &grant, &command("make"))
        };
        assert_eq!(at(Some(moment(3600))), Judged::Within);
        assert_eq!(at(Some(moment(-1))), Judged::Expired);
        assert_eq!(at(Some(moment(0))), Judged::Expired, "at the moment itself");
        assert_eq!(
            at(Some(cineris_protocol::Timestamp::from("soon"))),
            Judged::Expired
        );
        assert_eq!(at(None), Judged::Expired);
        let mut stopped = make();
        stopped
            .stop_when
            .push(StopCondition::Time(TimeStop { at: moment(-60) }));
        assert_eq!(
            judged(&parts, true, &stopped, &command("make")),
            Judged::Expired
        );
        let mut later = make();
        later
            .stop_when
            .push(StopCondition::Time(TimeStop { at: moment(60) }));
        assert_eq!(
            judged(&parts, true, &later, &command("make")),
            Judged::Within
        );
    }

    /// The grant decides who decides: a command its allow rule names is
    /// within it, and one it does not name is not, as the session's own
    /// level would ask about both. A grant with no rule at the session's
    /// level covers nothing the session asks about.
    #[test]
    fn a_grant_covers_what_its_rules_and_level_allow() {
        use cineris_protocol::lease::Effect;
        let parts = session_parts(Vec::new());
        let make = grant(
            Level::Supervised,
            vec![rule(Effect::Exec, "make", RuleAction::Allow)],
        );
        assert!(covers(&parts, &make, &command("make")));
        assert!(!covers(&parts, &make, &command("ninja")));
        assert!(!covers(
            &parts,
            &grant(Level::Supervised, Vec::new()),
            &command("make")
        ));
        // At `autonomous`, inside an enforcing sandbox, a command is the
        // holder's: any command in the session's scope is within.
        assert!(covers(
            &parts,
            &grant(Level::Autonomous, Vec::new()),
            &command("ninja")
        ));
    }

    /// A grant never widens what the session holds: the session's `ask` and
    /// `deny` rules beat the grant's allow; a path beyond the session's
    /// scope stays beyond it; `unrestricted` is not handed on; a level the
    /// session could not hold, and a narrow that fails, cover nothing.
    #[test]
    fn a_grant_never_widens_what_the_session_holds() {
        use cineris_protocol::lease::Effect;
        let make = |level| grant(level, vec![rule(Effect::Exec, "make", RuleAction::Allow)]);
        // The user's own `ask` and `deny` hold.
        for action in [RuleAction::Ask, RuleAction::Deny] {
            let parts = session_parts(vec![rule(Effect::Exec, "make", action)]);
            assert!(
                !covers(&parts, &make(Level::Supervised), &command("make")),
                "{action:?}"
            );
        }
        let parts = session_parts(Vec::new());
        // A write outside the session's writable roots, which the grant's
        // own roots would reach.
        let outside = vec![Access::Write {
            path: native("/elsewhere/notes.txt"),
        }];
        assert!(!covers(&parts, &make(Level::Autonomous), &outside));
        // `unrestricted` would make that write the holder's: it is read as
        // `autonomous`.
        assert!(!covers(&parts, &make(Level::Unrestricted), &outside));
        assert!(covers(&parts, &make(Level::Unrestricted), &command("make")));
        // `autonomous` where the sandbox does not enforce cannot be held.
        let unconfined = cineris_policy::test_support::TestParts::at(native("/work/app"))
            .home(native("/users/dev"))
            .sandbox(cineris_policy::SandboxState::NotEnforcing(
                cineris_policy::NoSandbox::TurnedOff,
            ))
            .parts();
        assert!(!covers(
            &unconfined,
            &make(Level::Autonomous),
            &command("ninja")
        ));
        // A grant whose models the session has none of.
        let mut models = parts.clone();
        models.models = vec![cineris_protocol::ids::ModelId::new("m_session")];
        let mut other = make(Level::Supervised);
        other.grant.models = vec![cineris_protocol::ids::ModelId::new("m_other")];
        assert!(!covers(&models, &other, &command("make")));
        assert!(covers(&models, &make(Level::Supervised), &command("make")));
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
            default: cineris_protocol::lease::RuleAction::Ask,
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

        let revoking_held = cineris_testkit::Hold::new(CHANGING_APPROVALS);
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

        let recording_held = cineris_testkit::Hold::new(CHANGING_APPROVALS);
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
            cineris_policy::exact_command_rule("cargo test", cineris_policy::ShellSyntax::Posix)
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
                        .revoke(&[cineris_protocol::ids::ApprovalId::new("none")])
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
            let held = cineris_testkit::Hold::new(point);
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
            cineris_policy::exact_command_rule("cargo test", cineris_policy::ShellSyntax::Posix)
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
        let wrong = cineris_protocol::lease::Rule {
            effect: cineris_protocol::lease::Effect::Write,
            pattern: "x".to_owned(),
            action: cineris_protocol::lease::RuleAction::Allow,
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
