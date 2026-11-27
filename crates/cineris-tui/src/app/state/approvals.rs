//! The person's approvals in the application's state (the signed approvals
//! step): the password asked for when a session opens with approvals to
//! use, and at the trust question's yes. See `app::approvals`.
//!
//! - **At the first open:** after the first frame, [`Approvals::to_use`]
//!   is asked once. When it says so, the password is asked for with
//!   [`Purpose::Unlock`] and the open waits for the answer: the approvals
//!   opened, or `Esc`, after which it opens without them and says so. A
//!   password that is not the person's asks again.
//! - **At the trust question's yes:** a question that proposes what a yes
//!   writes is not answered yet. The password is asked for with
//!   [`Purpose::Sign`] and the question's summary (or [`Purpose::Choose`]
//!   with no key set up), the proposal is signed, and the answer is the
//!   yes with its signature. `Esc` is not now; a password that is not the
//!   person's asks again; a failure is said, and is not now.
//! - **Each call runs on a thread of its own,** which moves the password
//!   in and drops it there, and wakes the loop with its answer.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use cineris_base::Password;
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::methods::{ServerResponse, TrustRequest, TrustResponse};

use super::App;
use crate::app::Open;
use crate::app::approvals::{
    Answer, Approvals, ApprovalsState, CANNOT_SIGN, Refused, SETS_UP, UNLOCKS, WITHOUT,
};
use crate::secret::{PasswordAsk, Purpose};

/// The program's approvals, and where the application is with them.
pub(super) struct Here {
    approvals: Arc<dyn Approvals>,
    /// Whether the first open has been looked at: asked once.
    looked: bool,
    /// The open waits for the password's answer.
    holding: bool,
    /// The open that waits.
    held: Option<Open>,
    sender: Sender<Answer>,
    answers: Receiver<Answer>,
    /// Calls made and not yet answered.
    owed: usize,
}

impl std::fmt::Debug for Here {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Here")
            .field("looked", &self.looked)
            .field("holding", &self.holding)
            .field("owed", &self.owed)
            .finish_non_exhaustive()
    }
}

impl Here {
    pub(super) fn new(approvals: Arc<dyn Approvals>) -> Self {
        let (sender, answers) = mpsc::channel();
        Self {
            approvals,
            looked: false,
            holding: false,
            held: None,
            sender,
            answers,
            owed: 0,
        }
    }
}

/// A call to make on its thread.
enum Call {
    Unlock(Password),
    Sign {
        id: RequestId,
        request: Box<TrustRequest>,
        password: Password,
        set_up: bool,
    },
}

impl App {
    /// After a frame: on the first, whether the session about to open has
    /// approvals to use. If it has, the open is held until the password is
    /// answered, and `true`: the caller asks for it
    /// ([`ask_unlock`](Self::ask_unlock)) once the frame is done with.
    pub(super) fn approvals_look(&mut self) -> bool {
        let Some(here) = self.approvals.as_mut() else {
            return false;
        };
        if here.looked {
            return false;
        }
        here.looked = true;
        here.holding = here.approvals.to_use();
        here.holding
    }

    /// `open`, unless the approvals hold it: then it waits for the
    /// password's answer, and `None`.
    pub(super) fn approvals_hold(&mut self, open: Open) -> Option<Open> {
        match self.approvals.as_mut() {
            Some(here) if here.holding => {
                here.held = Some(open);
                None
            }
            _ => Some(open),
        }
    }

    /// Asks for the password to open the approvals, with `warning` above
    /// the field: the open waits for the answer.
    pub(super) fn ask_unlock(&mut self, warning: Option<String>) {
        let mut ask = PasswordAsk::new(Purpose::Unlock, vec![UNLOCKS.to_owned()]);
        if let Some(warning) = warning {
            ask = ask.warning(warning);
        }
        self.ask_password(ask, |app, password| match password {
            Some(password) => app.approvals_call(Call::Unlock(password)),
            None => {
                app.say(WITHOUT);
                app.approvals_release();
            }
        });
    }

    /// The open waits no more: it is sent, if it waited.
    fn approvals_release(&mut self) {
        let Some(here) = self.approvals.as_mut() else {
            return;
        };
        here.holding = false;
        if let Some(open) = here.held.take() {
            self.send_open(open);
        }
    }

    /// The person said yes to the trust question `id`, which proposes what
    /// a yes writes: it is answered once the proposal is signed.
    pub(super) fn trust_yes(&mut self, id: RequestId, request: TrustRequest) {
        let Some(here) = self.approvals.as_ref() else {
            self.say(CANNOT_SIGN);
            self.trust_not_now(id);
            return;
        };
        let set_up = here.approvals.state() == ApprovalsState::NotSetUp;
        self.ask_to_sign(id, Box::new(request), set_up, None);
    }

    fn ask_to_sign(
        &mut self,
        id: RequestId,
        request: Box<TrustRequest>,
        set_up: bool,
        warning: Option<String>,
    ) {
        let mut lines = request.summary.clone();
        if lines.is_empty() {
            lines.push("This project's trust, as the question showed it.".to_owned());
        }
        let purpose = if set_up {
            lines.push(SETS_UP.to_owned());
            Purpose::Choose
        } else {
            Purpose::Sign
        };
        let mut ask = PasswordAsk::new(purpose, lines);
        if let Some(warning) = warning {
            ask = ask.warning(warning);
        }
        self.ask_password(ask, move |app, password| match password {
            Some(password) => app.approvals_call(Call::Sign {
                id,
                request,
                password,
                set_up,
            }),
            None => app.trust_not_now(id),
        });
    }

    /// Not now, to the trust question `id`.
    fn trust_not_now(&mut self, id: RequestId) {
        self.link
            .respond(id, ServerResponse::Trust(TrustResponse::default()));
    }

    /// Makes `call` on a thread of its own, which answers through the
    /// channel and wakes the loop.
    fn approvals_call(&mut self, call: Call) {
        let Some(here) = self.approvals.as_mut() else {
            return;
        };
        let approvals = Arc::clone(&here.approvals);
        let sender = here.sender.clone();
        let waker = self.waker.clone();
        here.owed += 1;
        let made = std::thread::Builder::new()
            .name("cineris-approvals".to_owned())
            .spawn(move || {
                let answer = guarded(approvals.as_ref(), call);
                if sender.send(answer).is_ok()
                    && let Some(waker) = waker
                {
                    waker.wake();
                }
            });
        if let Err(error) = made {
            // The call went with the thread that was not made; what it was
            // is said, and nothing waits for it.
            here.owed -= 1;
            let said =
                format!("Your approvals could not be used: no thread could be made ({error}).");
            self.say(&said);
            self.approvals_release();
        }
    }

    /// What the calls answered, taken: when the loop is woken. Says whether
    /// one came, which is a change to draw.
    pub(super) fn approvals_answers(&mut self) -> bool {
        let mut any = false;
        while let Some(answer) = self
            .approvals
            .as_ref()
            .and_then(|here| here.answers.try_recv().ok())
        {
            self.approvals_answer(answer);
            any = true;
        }
        any
    }

    /// Waits at most `limit` for each answer a call owes, taking each as
    /// the loop would: for tests, which have no loop to wake. Whether every
    /// call has answered.
    #[cfg(any(test, feature = "test-kit"))]
    pub fn wait_for_approvals(&mut self, limit: std::time::Duration) -> bool {
        loop {
            let Some(here) = self.approvals.as_ref() else {
                return true;
            };
            if here.owed == 0 {
                return true;
            }
            let Ok(answer) = here.answers.recv_timeout(limit) else {
                return false;
            };
            self.approvals_answer(answer);
        }
    }

    fn approvals_answer(&mut self, answer: Answer) {
        if let Some(here) = self.approvals.as_mut() {
            here.owed = here.owed.saturating_sub(1);
        }
        match answer {
            Answer::Unlocked(Ok(())) => self.approvals_release(),
            Answer::Unlocked(Err(Refused::NotThePassword(said))) => self.ask_unlock(Some(said)),
            Answer::Unlocked(Err(Refused::Failed(said))) => {
                self.say(&format!("Your approvals were not opened: {said}"));
                self.say(WITHOUT);
                self.approvals_release();
            }
            Answer::Signed {
                id,
                result: Ok(signature),
                ..
            } => self.link.respond(
                id,
                ServerResponse::Trust(TrustResponse {
                    trusted: true,
                    signature: Some(signature),
                    ..TrustResponse::default()
                }),
            ),
            Answer::Signed {
                id,
                request,
                result: Err(Refused::NotThePassword(said)),
            } => self.ask_to_sign(id, request, false, Some(said)),
            Answer::Signed {
                id,
                result: Err(Refused::Failed(said)),
                ..
            } => {
                self.say(&format!("The project was not trusted: {said}"));
                self.trust_not_now(id);
            }
        }
    }
}

/// [`run`], and a failure for its answer when the call panics: a panic on
/// the call's thread is the program's to report (its hook logs it), not a
/// reason for the open or the question to wait for good.
fn guarded(approvals: &dyn Approvals, call: Call) -> Answer {
    let failed = || Refused::Failed("it failed inside the program; the log says how".to_owned());
    let fallback = match &call {
        Call::Unlock(_) => Answer::Unlocked(Err(failed())),
        Call::Sign { id, request, .. } => Answer::Signed {
            id: id.clone(),
            request: request.clone(),
            result: Err(failed()),
        },
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(approvals, call)))
        .unwrap_or(fallback)
}

/// Makes `call` with `approvals`, on the call's own thread: the password is
/// dropped, wiped, when it returns.
fn run(approvals: &dyn Approvals, call: Call) -> Answer {
    match call {
        Call::Unlock(password) => Answer::Unlocked(approvals.unlock(&password)),
        Call::Sign {
            id,
            request,
            password,
            set_up,
        } => {
            let result = match request.proposal.as_ref() {
                None => Err(Refused::Failed(
                    "the question proposes nothing to sign".to_owned(),
                )),
                Some(proposal) => (if set_up {
                    approvals.set_up(&password)
                } else {
                    Ok(())
                })
                .and_then(|()| approvals.sign(&password, proposal)),
            };
            Answer::Signed {
                id,
                request,
                result,
            }
        }
    }
}

#[cfg(test)]
mod tests;
