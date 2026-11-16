//! The verification stage, behind a trait
//! (`docs/architecture/15-efficiency.md`, "Automatic verification").
//!
//! When the model ends a turn that changed files, the loop enters
//! `Verifying` and asks a [`Verifier`] for one round of the project's
//! checks. A failure is appended as a notice and the model carries on, up
//! to `LoopConfig::verify_rounds` rounds. Choosing the checks, running them
//! and shaping their output is the verifier's: the core's, which runs them
//! through the session's own shell. [`NoVerifier`] skips.
//!
//! **What the loop promises a verifier** (step 19):
//!
//! - A round is asked for only when files changed since the last round, and
//!   only while the turn's time limits leave time ([`VerifyCx::deadline`]).
//! - The loop **waits** for the round, an interrupt included: it fires
//!   [`VerifyCx::cancel`] (and [`VerifyCx::kill`] on a second interrupt)
//!   and waits for the verifier to stop what it runs, so that nothing of a
//!   check is left running and its results are recorded. A verifier must
//!   therefore return promptly once `cancel` fires.
//! - Nothing is added to the conversation while a round runs. A failure
//!   becomes one notice, after the round.

use std::time::Duration;

use cortex_base::{AbsPath, CancelToken};
use cortex_protocol::event::VerificationNotice;
use cortex_protocol::{SessionId, TurnId};
use cortex_tools::BoxFuture;

/// What one round of verification is given.
#[derive(Clone, Copy)]
pub struct VerifyCx<'a> {
    /// The session.
    pub session: &'a SessionId,
    /// The turn.
    pub turn: &'a TurnId,
    /// The turn's number in the session, counted from 1: what the turn's
    /// tool calls are told (`cortex_tools::RunCx::turn`).
    pub number: u32,
    /// Which round of this turn it is, counted from 1.
    pub round: u32,
    /// The most rounds the turn gets: on the last, a failure ends the turn.
    pub rounds: u32,
    /// What the turn's calls have changed so far, each a real location,
    /// sorted: the workspace root stands for a change whose files are not
    /// known (`cortex_tools::changed_by`).
    pub written: &'a [AbsPath],
    /// The time the turn's earlier rounds took.
    pub used: Duration,
    /// The time the turn's and the session's own time limits leave. `None`
    /// when they set none.
    pub deadline: Option<Duration>,
    /// Fires when the checks must stop: the turn is interrupted.
    pub cancel: &'a CancelToken,
    /// Fires when they must stop **now**: a second interrupt.
    pub kill: &'a CancelToken,
    /// Tells front ends of the round: when it starts, and how it ended.
    pub notify: &'a (dyn Fn(VerificationNotice) + Send + Sync),
}

impl std::fmt::Debug for VerifyCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifyCx")
            .field("session", self.session)
            .field("turn", self.turn)
            .field("round", &self.round)
            .field("rounds", &self.rounds)
            .field("written", &self.written.len())
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

/// What a round of verification found: the turn's verdict after it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VerifyOutcome {
    /// Every check passed, or failed once and passed when run again.
    Passed,
    /// Nothing ran to a result: no check is known, the mode is not `auto`,
    /// the project is not trusted, or none was allowed or had time.
    Skipped,
    /// No check failed, and some did not run to a result: blocked by the
    /// sandbox, stopped at their time limit, skipped or not allowed.
    Incomplete,
    /// A check failed, now or in an earlier round of the turn and not run
    /// to a result since.
    Failed {
        /// What failed, for the model: bounded, with the lines that matter,
        /// and every line of a check's output escaped and marked as the
        /// command's.
        report: String,
    },
    /// A check made the turn stop: it changed a protected path, or left a
    /// process that could not be cleared, or a change from outside was
    /// found before it. The turn ends with the outcome `denied`.
    Stopped {
        /// Why, for the model and for whoever reads how the turn ended.
        reason: String,
    },
}

/// Runs the project's checks.
pub trait Verifier: Send + Sync {
    /// One round. It returns promptly once `cx.cancel` fires, having
    /// stopped what it started.
    fn verify<'a>(&'a self, cx: VerifyCx<'a>) -> BoxFuture<'a, VerifyOutcome>;
}

/// The verifier of a session with no checks: every round is skipped.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoVerifier;

impl Verifier for NoVerifier {
    fn verify<'a>(&'a self, _cx: VerifyCx<'a>) -> BoxFuture<'a, VerifyOutcome> {
        Box::pin(std::future::ready(VerifyOutcome::Skipped))
    }
}
