//! The front ends connected to a core, and the requests the core has put
//! to them (`docs/architecture/11-rpc-protocol.md`, "Client roles",
//! "Server → client requests").
//!
//! A client has a role and capabilities, which it states when it
//! connects, and a queue ([`EventQueue`]) that everything for it goes
//! into. A request to the user (an approval, a question, trust) goes to
//! every client that may answer that kind, and the first answer wins. An
//! answer of another kind than its request is refused (since step 23, when
//! questions and trust began to be asked).
//!
//! **Who may answer.** Only a client with the `user` role. A `supervisor`
//! is another session's orchestration layer, which may answer within its
//! own grant once task leases exist (step 37); until then it is asked
//! nothing and its answers are refused, as an `observer`'s always are. An
//! answer from a client that was not asked is refused too.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cortex_protocol::error::code;
use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::methods::{
    ClientCapabilities, ClientRole, ServerRequest, ServerResponse, TrustResponse,
};
use cortex_protocol::{ErrorData, ErrorKind, SessionId};
use tokio::sync::oneshot;

use crate::events::EventQueue;

/// A connected front end, by the number the core gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub(crate) u64);

impl std::fmt::Display for ClientId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "client {}", self.0)
    }
}

/// What a front end is, and what it can handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// What it is to the sessions it opens.
    pub role: ClientRole,
    /// What it can handle. A capability that is not stated is not used.
    pub capabilities: ClientCapabilities,
    /// How many events may wait for it before its deltas are dropped; and,
    /// at [`DELTA_BYTES_PER_PLACE`](crate::events::DELTA_BYTES_PER_PLACE)
    /// each, how many bytes of deltas (8 MiB at the usual 1,024).
    pub queue: usize,
}

impl ClientInfo {
    /// A person's front end that answers approvals and wants deltas: what
    /// the terminal UI is.
    #[must_use]
    pub fn user() -> Self {
        Self {
            role: ClientRole::User,
            capabilities: ClientCapabilities {
                approvals: true,
                questions: true,
                trust: true,
                deltas: true,
                experimental: Vec::new(),
            },
            queue: crate::events::QUEUE_CAPACITY,
        }
    }

    /// A front end that answers nothing and wants whole items only: what
    /// headless mode is. With no other client, a call that needs an answer
    /// is denied.
    #[must_use]
    pub fn headless() -> Self {
        Self {
            role: ClientRole::User,
            capabilities: ClientCapabilities::default(),
            queue: crate::events::QUEUE_CAPACITY,
        }
    }

    /// Something that only watches.
    #[must_use]
    pub fn observer() -> Self {
        Self {
            role: ClientRole::Observer,
            capabilities: ClientCapabilities::default(),
            queue: crate::events::QUEUE_CAPACITY,
        }
    }
}

#[derive(Debug)]
struct Entry {
    info: ClientInfo,
    queue: Arc<EventQueue>,
}

/// A question that is open.
struct Open {
    session: SessionId,
    /// The clients it was put to.
    asked: Vec<u64>,
    /// What they were sent, for a client that joins while it is open.
    request: ServerRequest,
    answer: oneshot::Sender<ServerResponse>,
    /// The request ends when every client it was put to has gone: one
    /// about a session nobody else can join (a trust question while the
    /// session is put together) or work that holds the session (a rewind's
    /// question). Its answer's receiver is then told there is none.
    ends_with_askers: bool,
}

/// The kinds of request, by the capability a client declares to be put
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `approval/request`: the `approvals` capability.
    Approval,
    /// `question/ask`: `questions`.
    Question,
    /// `trust/request`: `trust`.
    Trust,
}

impl Kind {
    fn of(request: &ServerRequest) -> Self {
        match request {
            ServerRequest::Approval(_) => Self::Approval,
            ServerRequest::Question(_) => Self::Question,
            ServerRequest::Trust(_) => Self::Trust,
        }
    }

    fn answers(self, response: &ServerResponse) -> bool {
        matches!(
            (self, response),
            (Self::Approval, ServerResponse::Approval(_))
                | (Self::Question, ServerResponse::Question(_))
                | (Self::Trust, ServerResponse::Trust(_))
        )
    }

    fn declared(self, capabilities: &ClientCapabilities) -> bool {
        match self {
            Self::Approval => capabilities.approvals,
            Self::Question => capabilities.questions,
            Self::Trust => capabilities.trust,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Each change to these tables is whole before the lock is let go.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The connected clients and the open requests.
#[derive(Default)]
pub(crate) struct Clients {
    next_client: AtomicU64,
    next_request: AtomicU64,
    clients: Mutex<BTreeMap<u64, Entry>>,
    open: Mutex<BTreeMap<RequestId, Open>>,
}

impl std::fmt::Debug for Clients {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clients")
            .field("clients", &lock(&self.clients).len())
            .field("open", &lock(&self.open).len())
            .finish()
    }
}

fn refused(code: &str, message: impl Into<String>) -> Box<ErrorData> {
    Box::new(ErrorData::new(ErrorKind::Usage, code, message))
}

/// What is wrong with a trust answer to `request`, if anything: one that
/// contradicts itself is refused, as an unknown value of a grant is, and
/// the question stays open for an answer that can be read (the review of
/// step 23: it was taken as a yes). Never is an answer to a project not
/// trusted, and keep to one trusted already: a never to a question about
/// what is new in a trusted project would take away the trust given before
/// (the follow-ups of step 23).
fn trust_answer_problem(request: &ServerRequest, answer: &TrustResponse) -> Option<&'static str> {
    let trusted_project = matches!(request, ServerRequest::Trust(request) if request.trusted);
    match answer {
        TrustResponse {
            trusted: true,
            never: true,
            ..
        } => Some(
            "A trust answer cannot be both `trusted` and `never`: `never` goes with `trusted` \
             false.",
        ),
        TrustResponse {
            keep: true,
            trusted,
            never,
        } if *trusted || *never => Some(
            "A trust answer cannot be `keep` with `trusted` or `never`: `keep` goes with both \
             false.",
        ),
        TrustResponse { never: true, .. } if trusted_project => Some(
            "The project is trusted already: `never` is not an answer to what is new in it. \
             `keep` keeps trusting it without what is new.",
        ),
        TrustResponse { keep: true, .. } if !trusted_project => Some(
            "The project is not trusted: `keep` is an answer only to what is new in a project \
             trusted already.",
        ),
        _ => None,
    }
}

impl Clients {
    /// Adds a client, and returns its number and its queue.
    pub(crate) fn connect(&self, info: ClientInfo) -> (ClientId, Arc<EventQueue>) {
        let id = self.next_client.fetch_add(1, Ordering::Relaxed) + 1;
        let queue = Arc::new(EventQueue::new(info.queue, info.capabilities.deltas));
        lock(&self.clients).insert(
            id,
            Entry {
                info,
                queue: Arc::clone(&queue),
            },
        );
        (ClientId(id), queue)
    }

    /// Removes a client. Its queue takes nothing more.
    pub(crate) fn disconnect(&self, client: ClientId) {
        let mut clients = lock(&self.clients);
        if let Some(entry) = clients.remove(&client.0) {
            entry.queue.close();
        }
        // A request that ends with its askers, put to nobody connected now:
        // dropping it tells whoever waits that no answer will come.
        lock(&self.open).retain(|_, open| {
            !open.ends_with_askers || open.asked.iter().any(|asked| clients.contains_key(asked))
        });
    }

    pub(crate) fn queue(&self, client: ClientId) -> Option<Arc<EventQueue>> {
        lock(&self.clients)
            .get(&client.0)
            .map(|entry| Arc::clone(&entry.queue))
    }

    pub(crate) fn info(&self, client: ClientId) -> Option<ClientInfo> {
        lock(&self.clients)
            .get(&client.0)
            .map(|entry| entry.info.clone())
    }

    /// Which of `candidates` may be asked whether a tool call may run: the
    /// `user` clients that said they answer approvals.
    pub(crate) fn approvers(&self, candidates: &[u64]) -> Vec<u64> {
        self.askable(candidates, Kind::Approval)
    }

    /// Which of `candidates` may be put a request of `kind`: the `user`
    /// clients that declared that they answer it.
    pub(crate) fn askable(&self, candidates: &[u64], kind: Kind) -> Vec<u64> {
        let clients = lock(&self.clients);
        candidates
            .iter()
            .copied()
            .filter(|client| {
                clients.get(client).is_some_and(|entry| {
                    entry.info.role == ClientRole::User && kind.declared(&entry.info.capabilities)
                })
            })
            .collect()
    }

    /// Puts a request to `asked`, and returns its id and where the answer
    /// will arrive.
    pub(crate) fn ask(
        &self,
        session: &SessionId,
        asked: &[u64],
        request: impl Fn(&RequestId) -> ServerRequest,
    ) -> (RequestId, oneshot::Receiver<ServerResponse>) {
        self.put(session, asked, request, false)
    }

    /// Puts a request to `asked` that ends when they have all gone (see
    /// [`Open::ends_with_askers`]): its answer's receiver is then told
    /// there is none.
    pub(crate) fn ask_once(
        &self,
        session: &SessionId,
        asked: &[u64],
        request: impl Fn(&RequestId) -> ServerRequest,
    ) -> (RequestId, oneshot::Receiver<ServerResponse>) {
        self.put(session, asked, request, true)
    }

    fn put(
        &self,
        session: &SessionId,
        asked: &[u64],
        request: impl Fn(&RequestId) -> ServerRequest,
        ends_with_askers: bool,
    ) -> (RequestId, oneshot::Receiver<ServerResponse>) {
        let number = self.next_request.fetch_add(1, Ordering::Relaxed) + 1;
        let id = RequestId::Number(i64::try_from(number).unwrap_or(i64::MAX));
        let (answer, receiver) = oneshot::channel();
        let request = request(&id);
        // The request is open before any client can see it, so that an
        // answer cannot arrive for a request nobody knows.
        lock(&self.open).insert(
            id.clone(),
            Open {
                session: session.clone(),
                asked: asked.to_vec(),
                request: request.clone(),
                answer,
                ends_with_askers,
            },
        );
        let clients = lock(&self.clients);
        let mut reached = false;
        for client in asked {
            if let Some(entry) = clients.get(client) {
                entry.queue.push_request(id.clone(), request.clone());
                reached = true;
            }
        }
        // One that ends with its askers, put when none of them is connected
        // any more (one left while it was being made): dropped at once, so
        // that whoever waits is told no answer will come, as a disconnect
        // tells it (the review of step 23's second pull request). The
        // locks are taken in `disconnect`'s order: the clients, then the
        // open requests.
        if ends_with_askers && !reached {
            lock(&self.open).remove(&id);
        }
        (id, receiver)
    }

    /// Puts the open requests of `session` to a client that has just
    /// opened it, if it is one that may answer: a `user` client can attach
    /// to a session at any time and answer what is waiting. A question
    /// whose askers have all gone waits for this.
    pub(crate) fn join(&self, session: &SessionId, client: ClientId) {
        let Some(queue) = self.queue(client) else {
            return;
        };
        // What the client may answer, before the open requests are locked:
        // the clients' lock is never taken under theirs, which `put` and
        // `disconnect` take in the other order.
        let may = |kind: Kind| !self.askable(&[client.0], kind).is_empty();
        let answers = [
            (Kind::Approval, may(Kind::Approval)),
            (Kind::Question, may(Kind::Question)),
            (Kind::Trust, may(Kind::Trust)),
        ];
        for (id, open) in lock(&self.open).iter_mut() {
            let kind = Kind::of(&open.request);
            if !answers.iter().any(|(each, may)| *each == kind && *may) {
                continue;
            }
            if open.session == *session && !open.asked.contains(&client.0) {
                open.asked.push(client.0);
                queue.push_request(id.clone(), open.request.clone());
            }
        }
    }

    /// Withdraws the open requests that end with their askers (a trust
    /// question, a rewind's question) of one session, or of every session
    /// with `None`: whoever waits for one is told that no answer will
    /// come, and an answer that comes later is refused. For a session's
    /// end and the core's (the review of step 23: an undo's question
    /// outlived both).
    pub(crate) fn withdraw(&self, session: Option<&SessionId>) {
        lock(&self.open).retain(|_, open| {
            !open.ends_with_askers || session.is_some_and(|session| *session != open.session)
        });
    }

    /// Closes a request that will not be answered through here any more:
    /// it timed out, or was withdrawn. Says whether it was open.
    pub(crate) fn close(&self, id: &RequestId) -> bool {
        lock(&self.open).remove(id).is_some()
    }

    /// Takes a client's answer to a request it was put.
    ///
    /// # Errors
    ///
    /// `role_not_allowed` when the client is not a `user` client or was not
    /// asked, and an error of kind `usage` (`request_not_open`) when no
    /// such request is open (it was answered by another client, timed out
    /// or was withdrawn) or the answer is of another kind than the request.
    pub(crate) fn answer(
        &self,
        client: ClientId,
        id: &RequestId,
        response: ServerResponse,
    ) -> Result<SessionId, Box<ErrorData>> {
        let role = self.info(client).map(|info| info.role);
        if role != Some(ClientRole::User) {
            return Err(refused(
                code::ROLE_NOT_ALLOWED,
                "Only a `user` client may answer a request.",
            ));
        }
        let mut open = lock(&self.open);
        let Some(request) = open.get(id) else {
            return Err(refused(
                "request_not_open",
                format!(
                    "No request `{id}` is waiting for an answer: it was answered already, timed \
                     out, or was withdrawn."
                ),
            ));
        };
        if !request.asked.contains(&client.0) {
            return Err(refused(
                code::ROLE_NOT_ALLOWED,
                format!("The request `{id}` was not put to this client."),
            ));
        }
        if !Kind::of(&request.request).answers(&response) {
            return Err(refused(
                "request_not_open",
                format!("No request `{id}` of that kind is waiting for an answer."),
            ));
        }
        // A grant that contradicts itself is refused, as an unknown value
        // of a grant is, and the question stays open for an answer that
        // can be read (the review of step 23: it was taken as a yes).
        if let ServerResponse::Trust(answer) = &response
            && let Some(problem) = trust_answer_problem(&request.request, answer)
        {
            return Err(refused("invalid_answer", problem));
        }
        let Some(request) = open.remove(id) else {
            return Err(refused("request_not_open", "The request is gone."));
        };
        let session = request.session.clone();
        // If nobody waits for the answer any more, the question was
        // withdrawn in this instant, and the answer changes nothing.
        let _ = request.answer.send(response);
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use cortex_protocol::methods::{ApprovalChoice, ApprovalResponse, QuestionRequest};

    use super::*;
    use crate::Outgoing;

    fn request(session: &SessionId) -> ServerRequest {
        ServerRequest::Question(QuestionRequest {
            session_id: session.clone(),
            turn_id: None,
            question: "May it run?".to_owned(),
            choices: Vec::new(),
            extension: None,
        })
    }

    fn allow() -> ServerResponse {
        ServerResponse::Question(cortex_protocol::methods::QuestionResponse {
            answer: Some("yes".to_owned()),
        })
    }

    fn answer_of(response: ServerResponse) -> Option<String> {
        match response {
            ServerResponse::Question(question) => question.answer,
            _ => None,
        }
    }

    /// A request that ends with its askers, put when none of them is
    /// connected any more, is dropped at once: whoever waits is told no
    /// answer will come. One that waits for a client to join is kept.
    #[test]
    fn a_request_put_to_askers_already_gone_ends_at_once() {
        let clients = Clients::default();
        let session = SessionId::new("s_1");
        let (gone, _) = clients.connect(ClientInfo::user());
        clients.disconnect(gone);
        let (_, mut answer) = clients.ask_once(&session, &[gone.0], |_| request(&session));
        assert!(
            matches!(
                answer.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed)
            ),
            "the waiter was not told"
        );
        let (_, mut waiting) = clients.ask(&session, &[gone.0], |_| request(&session));
        assert!(
            matches!(
                waiting.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "a request that waits for a client to join was dropped"
        );
    }

    /// Each of the two checks on an answer holds by itself: the client's
    /// role, and whether it was asked.
    #[test]
    fn an_answer_counts_only_from_a_user_client_that_was_asked() {
        let clients = Clients::default();
        let session = SessionId::new("s_1");
        let (user, _) = clients.connect(ClientInfo::user());
        let (other_user, _) = clients.connect(ClientInfo::user());
        let (observer, _) = clients.connect(ClientInfo::observer());
        let (supervisor, _) = clients.connect(ClientInfo {
            role: ClientRole::Supervisor,
            ..ClientInfo::user()
        });

        // Only user clients that answer approvals are approvers.
        let everyone = [user.0, other_user.0, observer.0, supervisor.0];
        assert_eq!(clients.approvers(&everyone), [user.0, other_user.0]);
        let (headless, _) = clients.connect(ClientInfo::headless());
        assert!(clients.approvers(&[headless.0]).is_empty());

        // Suppose a request did reach clients that may not answer: their
        // role alone refuses the answer.
        let (id, mut answer) = clients.ask(&session, &[user.0, observer.0, supervisor.0], |_| {
            request(&session)
        });
        for client in [observer, supervisor] {
            let refused = clients.answer(client, &id, allow()).unwrap_err();
            assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        }
        // A user client that was not asked is refused too.
        let refused = clients.answer(other_user, &id, allow()).unwrap_err();
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        // A client that is not connected at all.
        let refused = clients.answer(ClientId(999), &id, allow()).unwrap_err();
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        assert!(answer.try_recv().is_err(), "a refused answer was delivered");

        // The one it was put to answers it, once.
        // An answer of another kind is not one.
        let wrong = clients
            .answer(
                user,
                &id,
                ServerResponse::Approval(ApprovalResponse {
                    choice: ApprovalChoice::AllowOnce,
                    note: None,
                }),
            )
            .unwrap_err();
        assert_eq!(wrong.code, "request_not_open");
        assert!(
            answer.try_recv().is_err(),
            "an answer of another kind was delivered"
        );
        assert_eq!(clients.answer(user, &id, allow()).unwrap(), session);
        assert_eq!(
            answer_of(answer.try_recv().unwrap()).as_deref(),
            Some("yes")
        );
        let late = clients.answer(user, &id, allow()).unwrap_err();
        assert_eq!(late.code, "request_not_open");
        assert!(!clients.close(&id), "an answered request is closed");
    }

    /// A user client that opens the session while a question waits is
    /// put the question, and may answer it. Nobody else is.
    #[test]
    fn a_user_client_that_joins_is_put_what_is_waiting() {
        let clients = Clients::default();
        let (session, other) = (SessionId::new("s_1"), SessionId::new("s_2"));
        let (first, _) = clients.connect(ClientInfo::user());
        let (id, mut answer) = clients.ask(&session, &[first.0], |_| request(&session));
        let (elsewhere, _) = clients.ask(&other, &[first.0], |_| request(&other));
        // The first client goes away with the question unanswered.
        clients.disconnect(first);

        let (observer, watching) = clients.connect(ClientInfo::observer());
        clients.join(&session, observer);
        assert!(watching.is_empty(), "an observer is asked nothing");
        let (headless, quiet) = clients.connect(ClientInfo::headless());
        clients.join(&session, headless);
        assert!(
            quiet.is_empty(),
            "nor is a client that answers no approvals"
        );

        let (late, queue) = clients.connect(ClientInfo::user());
        clients.join(&session, late);
        // Joining twice asks once.
        clients.join(&session, late);
        let Some(Outgoing::Request { id: put, .. }) = queue.try_next() else {
            panic!("the open request was not put to the client that joined");
        };
        assert_eq!(put, id);
        assert!(queue.is_empty(), "only this session's, and once");
        // A question of another session is not this client's to answer.
        let refused = clients.answer(late, &elsewhere, allow()).unwrap_err();
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        assert_eq!(clients.answer(late, &id, allow()).unwrap(), session);
        assert_eq!(
            answer_of(answer.try_recv().unwrap()).as_deref(),
            Some("yes")
        );
    }

    #[test]
    fn a_disconnected_client_gets_nothing_more() {
        let clients = Clients::default();
        let (client, queue) = clients.connect(ClientInfo::user());
        clients.disconnect(client);
        assert!(clients.queue(client).is_none());
        assert!(clients.info(client).is_none());
        assert_eq!(cortex_testkit::block_on(queue.next()), None);
    }
}
