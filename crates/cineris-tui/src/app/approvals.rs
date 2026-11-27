//! The person's approvals in the application (the signed approvals step;
//! `docs/architecture/09-tui.md`, "The approvals password"): what the
//! program gives the application so that it can open the approvals when a
//! session opens, and sign at the trust question's yes.
//!
//! This crate sees neither the policy nor the core, so the program gives an
//! [`Approvals`], as it gives a [`Setup`](super::Setup): the key is made,
//! the statement rebuilt and the signature made there, from the password
//! the application's dialog read. `cineris attach` gives none: until the
//! second pull request of the step, its yes cannot be signed.
//!
//! - **The password goes to the [`Approvals`] call and nowhere else:** the
//!   dialog hands it to the handler (`App::ask_password`), the handler
//!   moves it into the call, and the call drops it, wiped. It is never in
//!   the input box, its history, a draft, the scrollback, the log or an
//!   event.
//! - **None of it runs on the interface's thread.** Each call is made on a
//!   thread of its own, which wakes the loop when it answers: Argon2 at the
//!   standard cost takes a sixth of a second, which a frame does not wait
//!   for.
//! - **When a session opens with approvals to use** ([`Approvals::to_use`],
//!   asked once, after the first frame), the password is asked for, and
//!   the session opens once it is answered: with the approvals, or, after
//!   `Esc`, without them.
//! - **At the trust question's yes,** for a question that proposes what a
//!   yes writes, the password is asked for with the question's summary,
//!   and the answer is the signature. A password that is not the person's
//!   asks again; `Esc` is not now. With no key set up, the yes sets one up
//!   first: a password chosen, typed twice.

use cineris_base::Password;
use cineris_protocol::methods::ApprovalProposal;

/// How the person's approvals stand in the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalsState {
    /// No key has been set up: the first yes sets one up.
    NotSetUp,
    /// A key is set up, and this program's approvals are not open with it.
    Locked,
    /// This program's approvals are open.
    Open,
}

/// Why a call of [`Approvals`] did nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The password typed does not make the person's key: it is asked
    /// again, with this sentence.
    NotThePassword(String),
    /// Anything else (the key file unreadable, a debugger attached, a
    /// proposal that does not read): said, and the question goes on
    /// without the approvals.
    Failed(String),
}

impl Refused {
    /// The sentence it says.
    #[must_use]
    pub fn said(&self) -> &str {
        match self {
            Self::NotThePassword(said) | Self::Failed(said) => said,
        }
    }
}

/// What the program gives the application for the person's approvals.
/// Every call but [`to_use`](Self::to_use) and [`state`](Self::state)
/// makes a key from a password, which takes time: they run on a thread of
/// their own.
pub trait Approvals: Send + Sync {
    /// Whether the session about to open has approvals to use: a key is
    /// set up, the approvals are not open, and the user's own pins or the
    /// project's entry carry a signature. One cheap read, which opens
    /// nothing: it decides only whether the password is asked for.
    fn to_use(&self) -> bool;

    /// How the approvals stand now.
    fn state(&self) -> ApprovalsState;

    /// Why no password may be asked in this program now, if none may: a
    /// debugger attached, which could read it as it is typed, or, on
    /// Windows, the program not closed to the user's other programs. Asked
    /// before the password's dialog opens (the full review of the signed
    /// approvals: the program's check came only after the dialog had read
    /// the password); each call below asks again, for what changed while
    /// the dialog was open. One cheap check.
    fn why_not_asked(&self) -> Option<String>;

    /// Opens the approvals with the key `password` makes, for every session
    /// opened from now on.
    ///
    /// # Errors
    ///
    /// [`Refused::NotThePassword`] when it is not the person's password;
    /// [`Refused::Failed`] otherwise.
    fn unlock(&self, password: &Password) -> Result<(), Refused>;

    /// Sets up the person's key from the password they chose, and opens
    /// the approvals with it.
    ///
    /// # Errors
    ///
    /// [`Refused::Failed`], with why: a key is set up already, or the file
    /// could not be written.
    fn set_up(&self, password: &Password) -> Result<(), Refused>;

    /// Signs `proposal` with the key `password` makes: the statement is
    /// rebuilt from the proposal as the store makes it, the approvals are
    /// opened with the key if they are not, and the key is dropped once it
    /// has signed. The signature, as 128 lowercase hexadecimal digits.
    ///
    /// # Errors
    ///
    /// [`Refused::NotThePassword`] when it is not the person's password;
    /// [`Refused::Failed`] otherwise.
    fn sign(&self, password: &Password, proposal: &ApprovalProposal) -> Result<String, Refused>;
}

/// What a call made on its thread answered.
#[derive(Debug)]
pub(crate) enum Answer {
    /// The approvals were opened, or not.
    Unlocked(Result<(), Refused>),
    /// The trust question `id` was signed, or not: the request, for asking
    /// again.
    Signed {
        id: cineris_protocol::jsonrpc::RequestId,
        request: Box<cineris_protocol::methods::TrustRequest>,
        result: Result<String, Refused>,
    },
}

/// The line a password chosen at a yes adds to what it approves.
pub const SETS_UP: &str = "No password is set for your approvals yet: the one you choose now \
     signs this, and every approval you give after it.";

/// The line under the question when a session opens with approvals to use.
pub const UNLOCKS: &str = "Your trust in this project, and your approvals of your own \
     extensions, count only with it: without it they hold back what they approved, and no \
     question about this project's trust is asked.";

/// What is said when the session opened without the approvals.
pub(crate) const WITHOUT: &str = "This session opened without your approvals: what you approved \
     holds back what it approved, and this project's trust is not asked about. They are asked \
     for again at the next start.";

/// What is said when the program gave nothing to sign with.
pub(crate) const CANNOT_SIGN: &str = "This project's trust is signed with your approvals password, \
     which this interface cannot ask for: `cineris permissions trust`, in your own terminal, \
     trusts it. It was not trusted.";
