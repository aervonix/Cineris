//! How the protocol's messages travel to one server and back, behind a
//! trait that standard I/O implements here and Streamable HTTP will
//! (`docs/steps/30/parts/pr4-client.md`, "How HTTP plugs in").
//!
//! The session ([`crate::session`]) knows the protocol: the eras, `_meta`,
//! the listings, the calls, the change stream. A [`Transport`] knows only
//! how a request reaches the server and how its answer comes back, how to
//! give up on one, and what arrives that is not an answer. Over standard
//! I/O that is one stream of lines both ways, with answers matched to
//! requests by id ([`Pending`]); over HTTP each request is a POST whose
//! response carries its answer.
//!
//! Both bound what a server can make the client hold: every message that is
//! not the answer to a pending request is counted with its bytes
//! ([`Budget`]), and a server that sends more than a minute's allowance is
//! ended as misbehaving (point 3.7 of `docs/steps/30/answered.md`, as the
//! review of the design widened it).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use cineris_tools::{BoxFuture, Json};
use tokio::sync::{oneshot, watch};

use crate::client::{ClientError, Era};
use crate::jsonrpc::{Answer, Message};

/// How the protocol's messages reach one server.
pub(crate) trait Transport: Send + Sync {
    /// Sends `message`, our request numbered `id`, and resolves with its
    /// answer, or with why the connection ended first. Dropping the future
    /// gives the request up without telling the server: an answer that
    /// comes later is a stray. [`Transport::cancel`] tells it.
    fn request(&self, id: u64, message: Json) -> BoxFuture<'_, Result<Answer, ClientError>>;

    /// Sends a notification, or the answer to a server's request, without
    /// waiting for room: one a server that is not reading has no room for
    /// is dropped.
    fn send(&self, message: Json);

    /// Tells the server that the request numbered `id` is given up, as this
    /// transport does it: over standard I/O a `notifications/cancelled`
    /// with the reason, in both eras.
    fn cancel(&self, id: u64, reason: &str);

    /// Tells the transport the era and the revision agreed, which can
    /// change how it frames messages: over standard I/O only a `2025-03-26`
    /// server may send a batch.
    fn agreed(&self, era: Era, version: &str);

    /// Resolves when the connection has ended, with why.
    fn closed(&self) -> BoxFuture<'_, ClientError>;

    /// Ends the connection: our side of it closed, then the server's
    /// process ended, within `grace` before the staged kill.
    fn shutdown(&self, grace: Duration) -> BoxFuture<'_, ()>;
}

/// What the session does with a message that is not the answer to one of
/// its requests: a server's request, a notification, a stray answer.
pub(crate) trait Inbound: Send + Sync {
    /// Handles `message`, and returns what to send back, if anything.
    fn handle(&self, message: Message) -> Option<Json>;
}

/// Locks a mutex, whether or not a thread panicked while holding it: each
/// one here guards a value that is whole after every statement.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The requests waiting for their answers, by id, and why the connection
/// ended once it has.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    state: Mutex<PendingState>,
}

#[derive(Debug, Default)]
struct PendingState {
    waiting: HashMap<u64, oneshot::Sender<Answer>>,
    ended: Option<ClientError>,
}

impl Pending {
    /// Registers request `id`: the receiver gets its answer. After the end,
    /// the error it ended with.
    pub(crate) fn insert(&self, id: u64) -> Result<oneshot::Receiver<Answer>, ClientError> {
        let mut state = lock(&self.state);
        if let Some(ended) = &state.ended {
            return Err(ended.clone());
        }
        let (sender, receiver) = oneshot::channel();
        state.waiting.insert(id, sender);
        Ok(receiver)
    }

    /// Forgets request `id`: an answer to it is a stray from now on.
    pub(crate) fn remove(&self, id: u64) {
        lock(&self.state).waiting.remove(&id);
    }

    /// Gives `answer` to request `id`. False when no such request waits:
    /// an unknown id, a second answer, or one after the request was given
    /// up.
    pub(crate) fn answer(&self, id: u64, answer: Answer) -> bool {
        let Some(sender) = lock(&self.state).waiting.remove(&id) else {
            return false;
        };
        // A receiver dropped since is a request given up a moment ago.
        sender.send(answer).is_ok()
    }

    /// How many requests are waiting.
    pub(crate) fn len(&self) -> usize {
        lock(&self.state).waiting.len()
    }

    /// Ends every request waiting, and every one made later, with `why`.
    /// The first end is the one kept.
    pub(crate) fn end(&self, why: ClientError) {
        let mut state = lock(&self.state);
        state.ended.get_or_insert(why);
        // Dropping the senders wakes each receiver, which reads the end.
        state.waiting.clear();
    }

    /// Why the connection ended, once it has.
    pub(crate) fn ended(&self) -> Option<ClientError> {
        lock(&self.state).ended.clone()
    }
}

/// A request waiting for its answer, given up when dropped.
pub(crate) struct Waiting<'a> {
    pending: &'a Pending,
    id: u64,
    receiver: oneshot::Receiver<Answer>,
}

impl<'a> Waiting<'a> {
    /// Registers request `id` in `pending`.
    pub(crate) fn new(pending: &'a Pending, id: u64) -> Result<Self, ClientError> {
        Ok(Self {
            pending,
            id,
            receiver: pending.insert(id)?,
        })
    }

    /// Its answer, or why the connection ended first.
    pub(crate) async fn answer(&mut self) -> Result<Answer, ClientError> {
        match (&mut self.receiver).await {
            Ok(answer) => Ok(answer),
            Err(_) => Err(self
                .pending
                .ended()
                .unwrap_or_else(|| ClientError::Stopped("the connection ended".to_owned()))),
        }
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.pending.remove(self.id);
    }
}

/// How the end of a connection is told to those waiting for it.
#[derive(Debug)]
pub(crate) struct Ending {
    sender: watch::Sender<Option<ClientError>>,
}

impl Default for Ending {
    fn default() -> Self {
        Self {
            sender: watch::channel(None).0,
        }
    }
}

impl Ending {
    /// Records the end, once: a second end changes nothing.
    pub(crate) fn end(&self, why: ClientError) {
        self.sender.send_if_modified(|ended| {
            if ended.is_some() {
                return false;
            }
            *ended = Some(why);
            true
        });
    }

    /// Why it ended, once it has.
    pub(crate) fn ended(&self) -> Option<ClientError> {
        self.sender.borrow().clone()
    }

    /// Resolves when it has ended, with why.
    pub(crate) async fn wait(&self) -> ClientError {
        let mut receiver = self.sender.subscribe();
        loop {
            if let Some(why) = receiver.borrow_and_update().clone() {
                return why;
            }
            if receiver.changed().await.is_err() {
                return ClientError::Stopped("the connection ended".to_owned());
            }
        }
    }
}

/// How many messages that are not answers to a pending request a server
/// may send in a minute.
pub(crate) const MESSAGES_A_MINUTE: u32 = 1000;

/// How many bytes of them.
pub(crate) const BYTES_A_MINUTE: u64 = 16 * 1024 * 1024;

/// The window the counts are kept over.
pub(crate) const BUDGET_WINDOW: Duration = Duration::from_secs(60);

/// What a server has sent in the current window that was not the answer to
/// a pending request: notifications, its requests, stray and duplicate
/// answers, and what could not be read.
#[derive(Debug, Default)]
pub(crate) struct Budget {
    messages: u32,
    bytes: u64,
}

impl Budget {
    /// Counts one message of `bytes`. The words of the end when it is one
    /// too many.
    pub(crate) fn spend(&mut self, bytes: usize) -> Result<(), String> {
        self.messages = self.messages.saturating_add(1);
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        if self.messages > MESSAGES_A_MINUTE {
            return Err(format!(
                "it sent more than {MESSAGES_A_MINUTE} messages in a minute that answered nothing \
                 the client asked, and was stopped as misbehaving"
            ));
        }
        if self.bytes > BYTES_A_MINUTE {
            return Err(format!(
                "it sent more than {} MiB in a minute that answered nothing the client asked, \
                 and was stopped as misbehaving",
                BYTES_A_MINUTE / 1024 / 1024
            ));
        }
        Ok(())
    }

    /// Whether anything was counted in this window.
    pub(crate) fn is_open(&self) -> bool {
        self.messages > 0
    }

    /// Starts a new window.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// An inbound handler shared by the transport and the session.
pub(crate) type SharedInbound = Arc<dyn Inbound>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_ends_at_one_message_too_many_or_one_byte_too_many() {
        let mut budget = Budget::default();
        assert!(!budget.is_open());
        for _ in 0..MESSAGES_A_MINUTE {
            budget.spend(10).unwrap();
        }
        assert!(budget.is_open());
        let words = budget.spend(10).unwrap_err();
        assert!(words.contains("more than 1000 messages"), "{words}");

        budget.reset();
        budget
            .spend(usize::try_from(BYTES_A_MINUTE).unwrap())
            .unwrap();
        let words = budget.spend(1).unwrap_err();
        assert!(words.contains("more than 16 MiB"), "{words}");
    }

    #[test]
    fn a_request_given_up_takes_no_answer_and_the_end_reaches_every_waiter() {
        let pending = Pending::default();
        let waiting = Waiting::new(&pending, 1).unwrap();
        assert_eq!(pending.len(), 1);
        drop(waiting);
        assert_eq!(pending.len(), 0, "forgotten when given up");
        assert!(!pending.answer(1, Ok(Json::Null)), "given up");
        assert!(!pending.answer(9, Ok(Json::Null)), "never asked");

        let mut waiting = Waiting::new(&pending, 2).unwrap();
        pending.end(ClientError::Stopped("it exited".to_owned()));
        pending.end(ClientError::Stopped("a second end".to_owned()));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(
            runtime.block_on(waiting.answer()),
            Err(ClientError::Stopped("it exited".to_owned()))
        );
        assert_eq!(
            Waiting::new(&pending, 3).err(),
            Some(ClientError::Stopped("it exited".to_owned())),
            "a request after the end ends at once"
        );
    }
}
