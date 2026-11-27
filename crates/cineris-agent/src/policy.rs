//! What the loop asks of the permission engine
//! (`docs/architecture/06-permissions-and-sandbox.md`).
//!
//! The engine itself is `cineris-policy`, and the core connects it to the
//! front ends that answer questions. The loop sees both as one [`Policy`]:
//! it shows a planned call, and is told to run it, to refuse it, or to ask.
//! Asking is the policy's too, so that the loop never knows who answers, how
//! long they may take, or what an answer adds to the rules.
//!
//! # Three things a policy can say beyond allow, ask and deny
//!
//! - **"It changes nothing"** ([`Decision::AllowReadOnly`]). A shell command
//!   is planned as a call that may change anything, and so runs alone. When
//!   the engine has analysed the command and knows that it only reads, the
//!   loop schedules the call as read-only: it runs beside other calls, and
//!   the tool does not look for what it changed.
//! - **"Stop the turn here"** ([`Decision::Stop`], [`Verdict::Stop`]). A
//!   denial is normally a result for the model, and the round goes on: the
//!   model may find another way. Where that is wrong (a front end that
//!   cannot ask and was told to fail on the first denial; later, the edge of
//!   a task lease), the policy says stop, and the turn ends with the
//!   outcome `denied`.
//!
//! [`AllowAll`] is the stand-in for a session with no engine.

use std::sync::Arc;

use cineris_protocol::{CallId, ItemId, SessionId, TurnId};
use cineris_tools::{BoxFuture, ChangePreview, Json, OpenGate, Plan, ReadGate, ToolSpec};

use crate::hooks::HookQuestion;

/// One planned tool call, as the policy is shown it.
#[derive(Debug, Clone, Copy)]
pub struct PlannedCall<'a> {
    /// The session the call belongs to.
    pub session: &'a SessionId,
    /// The turn.
    pub turn: &'a TurnId,
    /// The provider's id for the call.
    pub call_id: &'a CallId,
    /// The loop's own id for the call: its result's item, unique in the
    /// session. A provider's id may repeat (a server that gives none has
    /// ids made up for it), and a call made again by a response sent again
    /// has a new one of these. So it is what an answer is kept by, for a
    /// call that is decided again.
    pub item: &'a ItemId,
    /// The tool's spec: its name, and what it does by default.
    pub spec: &'a ToolSpec,
    /// The arguments, as the model wrote them.
    pub arguments: &'a Json,
    /// What the call would do.
    pub plan: &'a Plan,
    /// What the call would change in a file, worked out by its tool
    /// ([`cineris_tools::Tool::preview`]): given to [`Policy::ask`] for the
    /// question it puts, and `None` to [`Policy::decide`] and for a tool
    /// that changes no file.
    pub preview: Option<&'a ChangePreview>,
    /// What the call's `tool.before` hooks said that the question must show
    /// (step 28): the hooks that asked, and a replacement of the arguments.
    /// Given to [`Policy::ask`] only, and `None` for a call no hook
    /// narrowed.
    ///
    /// With it, [`Policy::ask`] puts the question **even when the rules
    /// alone would allow the call**: a hook that asks turns an allowed call
    /// into a question. Where nobody can be asked, such a question is
    /// answered "deny", as any question nobody can answer is.
    ///
    /// With a replacement (`replaced`), `arguments` and `plan` are the
    /// hook's, the model's own are its `original`, and the question offers
    /// only "allow once" and "deny", keeps no rule, and where nobody can be
    /// asked is answered "deny", whatever a default answer would be: a
    /// replacement is new work nobody chose. **It is answered only by a
    /// person** (a `user` client), never by a supervisor's grant: the core
    /// enforces it.
    pub hooks: Option<&'a HookQuestion>,
}

/// What the policy says of a planned call, before anybody is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Decision {
    /// The call may run.
    Allow,
    /// The call may run, and the policy knows that it changes nothing: the
    /// loop schedules it as read-only, whatever its plan said. The engine
    /// says this of a shell command it has analysed and found on its table
    /// of commands that only read.
    AllowReadOnly,
    /// Whoever supervises the session decides: the loop calls
    /// [`Policy::ask`].
    Ask,
    /// The call may not run. It gets an error result, and the round goes
    /// on: the model may find another way.
    Deny {
        /// Why, for the model.
        reason: Option<String>,
    },
    /// The call may not run, and the turn ends here, with the outcome
    /// `denied`. Nothing that has not started yet starts.
    Stop {
        /// Why, for the model and for whoever reads how the turn ended.
        reason: String,
    },
}

/// The answer to a question.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verdict {
    /// The call may run.
    Allow,
    /// The call may not run. It gets an error result, and the round goes
    /// on.
    Deny {
        /// What the person who refused wrote, for the model.
        note: Option<String>,
    },
    /// The call may not run, and the turn ends here, with the outcome
    /// `denied`: nobody could be asked, and the session was told to stop
    /// then.
    Stop {
        /// Why, for the model and for whoever reads how the turn ended.
        reason: String,
    },
}

/// Decides whether a planned call may run.
///
/// **No method has a default body**, as for `cineris_tools::Tool`: a policy
/// may wrap another, and a method added later with a default would be
/// silently not forwarded by a wrapper that predates it. Every
/// implementation writes every method; `cargo xtask rules` keeps default
/// bodies out of this trait.
pub trait Policy: Send + Sync {
    /// Decides a call from its plan, without asking anybody. It is called
    /// off the session's task, once per call, as soon as the call is
    /// planned, which can be while the model is still writing its response.
    fn decide(&self, call: &PlannedCall<'_>) -> Decision;

    /// Puts the question to whoever supervises the session, and returns the
    /// answer. Whatever an answer changes (a rule for the session, a stored
    /// approval) is the policy's to record.
    ///
    /// The loop asks one question at a time, in the model's call order. It
    /// drops the future when the turn is interrupted or the response the
    /// call came from is thrown away: the question is then withdrawn.
    fn ask<'a>(&'a self, call: PlannedCall<'a>) -> BoxFuture<'a, Verdict>;

    /// Whether an answer [`ask`](Self::ask) gives may be [`Verdict::Stop`].
    ///
    /// The turn stops at the refused call in the model's order, so nothing
    /// the model wrote after it that changes anything may have started
    /// (`docs/architecture/03-agent-loop.md`, "Tool rounds"). While a
    /// question is open, the loop therefore holds back every later call
    /// with side effects for as long as this is `true`; calls that only
    /// read still run beside the question.
    ///
    /// Return `false` only when **no** answer the front end can give is
    /// `Stop`. Today that is a person answering: the choices of an approval
    /// have no "stop", so a later edit may run while they decide. A front end
    /// that gains a way to stop the turn from a question (the terminal
    /// application, step 23; an approval answered over RPC, step 27), and a
    /// policy that answers `Stop` when nobody can be asked
    /// (`--fail-on-denial`), returns `true`. When in doubt, `true`: slower,
    /// and never wrong.
    fn answers_may_stop(&self) -> bool;

    /// How many times the rules this policy decides by have narrowed: a
    /// count that only grows, and moves whenever something that let calls
    /// run is taken back (an approval revoked, a learned command turned
    /// off, the level changed).
    ///
    /// The loop dates each decision by it, and decides a call again when
    /// its turn to start comes if it has moved since
    /// (`docs/architecture/03-agent-loop.md`, "Tool rounds"): a call
    /// decided before a rule was taken back must not run on the rule.
    /// It is read for every call that may start, so it must be cheap. A
    /// policy whose rules never change returns a constant.
    fn narrowed(&self) -> u64;

    /// What a directory walk may read, entry by entry.
    fn read_gate(&self) -> Arc<dyn ReadGate>;
}

/// The policy that allows every call: the stand-in until `cineris-policy`
/// exists.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAll;

impl Policy for AllowAll {
    fn decide(&self, _call: &PlannedCall<'_>) -> Decision {
        Decision::Allow
    }

    /// Allows what it is asked about, with nobody to ask: but for a call a
    /// hook put the question about (one it asked about, or whose arguments
    /// it replaced), which is refused as any question nobody can answer is:
    /// this stand-in asks nobody. Elsewhere a replacement is a person's to
    /// answer, and a hook's plain `ask` is whoever supervises the session's,
    /// a supervisor's grant included (`docs/architecture/03-agent-loop.md`).
    fn ask<'a>(&'a self, call: PlannedCall<'a>) -> BoxFuture<'a, Verdict> {
        let verdict = match call.hooks {
            None => Verdict::Allow,
            Some(hooks) => Verdict::Deny {
                note: Some(
                    if hooks.replaced.is_some() {
                        "Nobody could be asked, and only a person may allow arguments a hook \
                         replaced."
                    } else {
                        "Nobody could be asked what a hook asked."
                    }
                    .to_owned(),
                ),
            },
        };
        Box::pin(std::future::ready(verdict))
    }

    fn answers_may_stop(&self) -> bool {
        true
    }

    fn narrowed(&self) -> u64 {
        0
    }

    fn read_gate(&self) -> Arc<dyn ReadGate> {
        Arc::new(OpenGate)
    }
}

#[cfg(test)]
mod tests {
    use cineris_protocol::item::{HookReason, ReplacedArguments};
    use serde_json::json;

    use super::*;

    /// Where nobody can be asked, a question a hook put is refused, as any
    /// question nobody can answer is: `AllowAll` asks nobody. What it is
    /// asked about with no hook it allows.
    #[test]
    fn nobody_to_ask_refuses_a_replacement_and_allows_the_rest() {
        let (session, turn) = (SessionId::new("s_1"), TurnId::new("t_1"));
        let (call_id, item) = (CallId::new("c_1"), ItemId::new("i_1"));
        let spec = ToolSpec::new("write", "", json!({"type": "object"}));
        let arguments = json!({"path": "b"});
        let plan = Plan::new("write b", ());
        let asked = HookQuestion {
            asked: vec![HookReason {
                hook: "look".to_owned(),
                reason: None,
            }],
            replaced: None,
        };
        let replaced = HookQuestion {
            asked: Vec::new(),
            replaced: Some(ReplacedArguments {
                hook: "swap".to_owned(),
                arguments: arguments.clone(),
                original: Some(json!({"path": "a"})),
                reason: None,
            }),
        };
        let ask = |hooks| {
            cineris_testkit::block_on(AllowAll.ask(PlannedCall {
                session: &session,
                turn: &turn,
                call_id: &call_id,
                item: &item,
                spec: &spec,
                arguments: &arguments,
                plan: &plan,
                preview: None,
                hooks,
            }))
        };
        assert_eq!(ask(None), Verdict::Allow);
        assert!(matches!(ask(Some(&asked)), Verdict::Deny { .. }));
        assert!(matches!(ask(Some(&replaced)), Verdict::Deny { .. }));
    }
}
