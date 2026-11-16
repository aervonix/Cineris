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
//! **Who may answer** (since step 27; `docs/architecture/06-permissions-and-sandbox.md`,
//! "Whose answer counts"). A `user` client, any request it was put. A
//! `supervisor`, another session's orchestration layer, only an approval,
//! and only when it was connected with a grant
//! ([`Core::connect_with_grant`](crate::Core::connect_with_grant)): it is put every
//! approval of the session at once with the users, its denial is always
//! taken, and its allow only within the grant, as `allow_once`, which
//! allows the call and records nothing. An allow beyond the grant is
//! refused (`role_not_allowed`), and the request stays open for the users:
//! that is how a supervisor passes a request up. A supervisor with no grant
//! is asked nothing and its answers are refused, as an `observer`'s always
//! are. An answer from a client that was not asked is refused too. The
//! first answer taken settles a request: a user may answer at any moment
//! until then, which is how 17-coordinator's "a user's answer always wins"
//! is read.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cortex_protocol::error::code;
use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::lease::Lease;
use cortex_protocol::methods::{
    ApprovalChoice, ApprovalResponse, ClientCapabilities, ClientRole, ServerRequest,
    ServerResponse, TrustResponse,
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
    /// A `supervisor`'s grant: what it may approve
    /// ([`Core::connect_with_grant`](crate::Core::connect_with_grant)).
    grant: Option<Lease>,
    queue: Arc<EventQueue>,
}

/// Whether a supervisor's grant allows the call an approval is about:
/// what the policy that asked works out for the supervisor's allow
/// ([`crate::policy`]). Asked off the table's lock.
pub(crate) type Judge = Arc<dyn Fn(&Lease) -> bool + Send + Sync>;

/// An answer taken.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Answer {
    /// What the client answered; a supervisor's allow as `allow_once`.
    pub(crate) response: ServerResponse,
    /// A supervisor's answer: a denial, or an allow within its grant. It
    /// settles the call alone, and nothing is recorded for it.
    pub(crate) by_supervisor: bool,
}

/// A question that is open.
struct Open {
    session: SessionId,
    /// The clients it was put to.
    asked: Vec<u64>,
    /// What they were sent, for a client that joins while it is open.
    request: ServerRequest,
    answer: oneshot::Sender<Answer>,
    /// For an approval: whether a supervisor's grant covers the call. A
    /// request with none takes no supervisor's allow.
    judge: Option<Judge>,
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
    /// Adds a client, with a `supervisor`'s grant if it has one, and
    /// returns its number and its queue.
    pub(crate) fn connect(
        &self,
        info: ClientInfo,
        grant: Option<Lease>,
    ) -> (ClientId, Arc<EventQueue>) {
        let id = self.next_client.fetch_add(1, Ordering::Relaxed) + 1;
        let queue = Arc::new(EventQueue::new(info.queue, info.capabilities.deltas));
        lock(&self.clients).insert(
            id,
            Entry {
                info,
                grant,
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
    /// `user` clients that said they answer approvals, and a `supervisor`
    /// that said so and has a grant.
    pub(crate) fn approvers(&self, candidates: &[u64]) -> Vec<u64> {
        self.askable(candidates, Kind::Approval)
    }

    /// Which of `candidates` may be put a request of `kind`: the `user`
    /// clients that declared that they answer it; and for an approval, a
    /// `supervisor` that declared it and has a grant to answer within.
    pub(crate) fn askable(&self, candidates: &[u64], kind: Kind) -> Vec<u64> {
        let clients = lock(&self.clients);
        candidates
            .iter()
            .copied()
            .filter(|client| {
                clients.get(client).is_some_and(|entry| {
                    let info = &entry.info;
                    kind.declared(&info.capabilities)
                        && match info.role {
                            ClientRole::User => true,
                            ClientRole::Supervisor => {
                                kind == Kind::Approval && entry.grant.is_some()
                            }
                            _ => false,
                        }
                })
            })
            .collect()
    }

    /// Whether the client is a `supervisor`.
    pub(crate) fn is_supervisor(&self, client: u64) -> bool {
        lock(&self.clients)
            .get(&client)
            .is_some_and(|entry| entry.info.role == ClientRole::Supervisor)
    }

    /// Puts a request to `asked`, and returns its id and where the answer
    /// will arrive. For an approval, `judge` says whether a supervisor's
    /// grant covers its call: a supervisor's allow is taken only where it
    /// says so, and never with none.
    pub(crate) fn ask(
        &self,
        session: &SessionId,
        asked: &[u64],
        request: impl Fn(&RequestId) -> ServerRequest,
        judge: Option<Judge>,
    ) -> (RequestId, oneshot::Receiver<Answer>) {
        self.put(session, asked, request, false, judge)
    }

    /// Puts a request to `asked` that ends when they have all gone (see
    /// [`Open::ends_with_askers`]): its answer's receiver is then told
    /// there is none.
    pub(crate) fn ask_once(
        &self,
        session: &SessionId,
        asked: &[u64],
        request: impl Fn(&RequestId) -> ServerRequest,
    ) -> (RequestId, oneshot::Receiver<Answer>) {
        self.put(session, asked, request, true, None)
    }

    fn put(
        &self,
        session: &SessionId,
        asked: &[u64],
        request: impl Fn(&RequestId) -> ServerRequest,
        ends_with_askers: bool,
        judge: Option<Judge>,
    ) -> (RequestId, oneshot::Receiver<Answer>) {
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
                judge,
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
    /// `role_not_allowed` when the client is not a `user` client or a
    /// `supervisor` with a grant, was not asked, or is a supervisor whose
    /// allow is beyond its grant (the request stays open); and an error of
    /// kind `usage` (`request_not_open`) when no such request is open (it
    /// was answered by another client, timed out or was withdrawn) or the
    /// answer is of another kind than the request.
    pub(crate) fn answer(
        &self,
        client: ClientId,
        id: &RequestId,
        response: ServerResponse,
    ) -> Result<SessionId, Box<ErrorData>> {
        let who = lock(&self.clients)
            .get(&client.0)
            .map(|entry| (entry.info.role.clone(), entry.grant.clone()));
        let grant = match who {
            Some((ClientRole::User, _)) => None,
            Some((ClientRole::Supervisor, Some(grant))) => Some(grant),
            _ => {
                return Err(refused(
                    code::ROLE_NOT_ALLOWED,
                    "Only a `user` client, or a `supervisor` with a grant, may answer a request.",
                ));
            }
        };
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
        let mut response = response;
        let by_supervisor = grant.is_some();
        if let Some(grant) = grant {
            // A supervisor's denial is always taken, and so is an answer
            // this build does not know, which is one. Its allow, of any
            // scope, only within its grant, and then as `allow_once`.
            let ServerResponse::Approval(approval) = &response else {
                return Err(refused(
                    code::ROLE_NOT_ALLOWED,
                    "A `supervisor` answers approvals only.",
                ));
            };
            if matches!(
                approval.choice,
                ApprovalChoice::AllowOnce
                    | ApprovalChoice::AllowSession
                    | ApprovalChoice::AllowProject
            ) {
                let judge = request.judge.clone();
                let asked_for = approval.choice.clone();
                let note = approval.note.clone();
                // Worked out off the table's lock: it makes an engine.
                drop(open);
                let within = judge.is_some_and(|judge| judge(&grant));
                if !within {
                    // The client is not told by JSON-RPC that its answer was
                    // not taken: the log says it.
                    tracing::warn!(
                        %client,
                        request = %id,
                        "a supervisor's allow is beyond its grant: it is not taken, and the \
                         request stays open for the users"
                    );
                    return Err(refused(
                        code::ROLE_NOT_ALLOWED,
                        format!(
                            "The call of request `{id}` is beyond this supervisor's grant: the \
                             allow is not taken, and the request stays open for the session's \
                             users."
                        ),
                    ));
                }
                if asked_for != ApprovalChoice::AllowOnce {
                    tracing::info!(
                        %client,
                        request = %id,
                        choice = asked_for.as_str(),
                        "a supervisor's allow beyond this call is taken as allow_once: nothing \
                         is recorded"
                    );
                }
                response = ServerResponse::Approval(ApprovalResponse {
                    choice: ApprovalChoice::AllowOnce,
                    note,
                });
                open = lock(&self.open);
            }
        }
        // Still open, to this client: another answer may have settled it
        // while a supervisor's grant was looked at.
        if !open
            .get(id)
            .is_some_and(|request| request.asked.contains(&client.0))
        {
            return Err(refused(
                "request_not_open",
                format!("The request `{id}` was settled meanwhile."),
            ));
        }
        let Some(request) = open.remove(id) else {
            return Err(refused("request_not_open", "The request is gone."));
        };
        let session = request.session.clone();
        // If nobody waits for the answer any more, the question was
        // withdrawn in this instant, and the answer changes nothing.
        let _ = request.answer.send(Answer {
            response,
            by_supervisor,
        });
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use cortex_protocol::ids::LeaseId;
    use cortex_protocol::lease::{
        Budget, Expiry, ExpiryKind, Grant, Grantor, GrantorKind, HardLimits, Level, NetworkAccess,
        PathSet, SandboxPolicy,
    };
    use cortex_protocol::methods::{
        ApprovalChoice, ApprovalDetail, ApprovalRequest, ApprovalResponse, QuestionRequest,
        ToolDetail,
    };
    use cortex_protocol::{CallId, TurnId};

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

    fn answer_of(answer: Answer) -> Option<String> {
        match answer.response {
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
        let (gone, _) = clients.connect(ClientInfo::user(), None);
        clients.disconnect(gone);
        let (_, mut answer) = clients.ask_once(&session, &[gone.0], |_| request(&session));
        assert!(
            matches!(
                answer.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed)
            ),
            "the waiter was not told"
        );
        let (_, mut waiting) = clients.ask(&session, &[gone.0], |_| request(&session), None);
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
        let (user, _) = clients.connect(ClientInfo::user(), None);
        let (other_user, _) = clients.connect(ClientInfo::user(), None);
        let (observer, _) = clients.connect(ClientInfo::observer(), None);
        let (supervisor, _) = clients.connect(
            ClientInfo {
                role: ClientRole::Supervisor,
                ..ClientInfo::user()
            },
            None,
        );

        // Only user clients that answer approvals are approvers.
        let everyone = [user.0, other_user.0, observer.0, supervisor.0];
        assert_eq!(clients.approvers(&everyone), [user.0, other_user.0]);
        let (headless, _) = clients.connect(ClientInfo::headless(), None);
        assert!(clients.approvers(&[headless.0]).is_empty());

        // Suppose a request did reach clients that may not answer: their
        // role alone refuses the answer.
        let (id, mut answer) = clients.ask(
            &session,
            &[user.0, observer.0, supervisor.0],
            |_| request(&session),
            None,
        );
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
        let (first, _) = clients.connect(ClientInfo::user(), None);
        let (id, mut answer) = clients.ask(&session, &[first.0], |_| request(&session), None);
        let (elsewhere, _) = clients.ask(&other, &[first.0], |_| request(&other), None);
        // The first client goes away with the question unanswered.
        clients.disconnect(first);

        let (observer, watching) = clients.connect(ClientInfo::observer(), None);
        clients.join(&session, observer);
        assert!(watching.is_empty(), "an observer is asked nothing");
        let (headless, quiet) = clients.connect(ClientInfo::headless(), None);
        clients.join(&session, headless);
        assert!(
            quiet.is_empty(),
            "nor is a client that answers no approvals"
        );

        let (late, queue) = clients.connect(ClientInfo::user(), None);
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

    fn approval_request(session: &SessionId) -> ServerRequest {
        ServerRequest::Approval(ApprovalRequest {
            session_id: session.clone(),
            turn_id: TurnId::new("t_1"),
            call_id: CallId::new("c1"),
            tool: "shell".to_owned(),
            summary: "make".to_owned(),
            detail: ApprovalDetail::Tool(ToolDetail {
                arguments: serde_json::Value::Null,
            }),
            choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            reasons: Vec::new(),
            adds: Vec::new(),
        })
    }

    fn approval(choice: ApprovalChoice) -> ServerResponse {
        ServerResponse::Approval(ApprovalResponse {
            choice,
            note: Some("from the layer".to_owned()),
        })
    }

    /// A grant; what it allows is the judge's to say in these tests.
    fn a_grant() -> Lease {
        Lease {
            id: LeaseId::new("l_grant"),
            grant: Grant {
                level: Level::Supervised,
                rules: Vec::new(),
                sandbox: SandboxPolicy {
                    read: PathSet {
                        all: true,
                        roots: Vec::new(),
                    },
                    write: PathSet::default(),
                    protect: PathSet::default(),
                    network: NetworkAccess::Off,
                },
                models: Vec::new(),
                budget: Budget::default(),
                limits: HardLimits::default(),
            },
            stop_when: Vec::new(),
            expires: Expiry {
                kind: ExpiryKind::EndOfSession,
                at: None,
            },
            granted_by: Grantor {
                kind: GrantorKind::User,
                lease_id: None,
            },
        }
    }

    fn supervisor() -> ClientInfo {
        ClientInfo {
            role: ClientRole::Supervisor,
            ..ClientInfo::user()
        }
    }

    /// A supervisor with a grant is put approvals, and only approvals: a
    /// question and trust are a person's. One with no grant is put nothing.
    #[test]
    fn a_supervisor_with_a_grant_is_put_approvals_and_nothing_else() {
        let clients = Clients::default();
        let (user, _) = clients.connect(ClientInfo::user(), None);
        let (granted, _) = clients.connect(supervisor(), Some(a_grant()));
        let (bare, _) = clients.connect(supervisor(), None);
        let (silent, _) = clients.connect(
            ClientInfo {
                capabilities: ClientCapabilities::default(),
                ..supervisor()
            },
            Some(a_grant()),
        );
        let everyone = [user.0, granted.0, bare.0, silent.0];
        assert_eq!(clients.approvers(&everyone), [user.0, granted.0]);
        assert_eq!(clients.askable(&everyone, Kind::Question), [user.0]);
        assert_eq!(clients.askable(&everyone, Kind::Trust), [user.0]);
        // A grant given to a client of another role means nothing.
        let (observer, _) = clients.connect(ClientInfo::observer(), Some(a_grant()));
        assert!(clients.approvers(&[observer.0]).is_empty());

        // One that joins a session with an approval open is put it.
        let session = SessionId::new("s_1");
        let (id, _) = clients.ask(&session, &[user.0], |_| approval_request(&session), None);
        let (late, queue) = clients.connect(supervisor(), Some(a_grant()));
        clients.join(&session, late);
        assert!(
            matches!(queue.try_next(), Some(Outgoing::Request { id: put, .. }) if put == id),
            "the open approval was not put to the supervisor that joined"
        );
    }

    /// A supervisor's denial is always taken, and its allow, of any scope,
    /// only where the grant covers the call, and then as `allow_once`; an
    /// allow beyond it is refused and leaves the request open for a user.
    #[test]
    fn a_supervisors_allow_is_taken_only_within_its_grant_and_as_once() {
        let clients = Clients::default();
        let session = SessionId::new("s_1");
        let (user, _) = clients.connect(ClientInfo::user(), None);
        let (layer, _) = clients.connect(supervisor(), Some(a_grant()));
        let (bare, _) = clients.connect(supervisor(), None);
        let within = Arc::new(AtomicBool::new(false));
        let judged = Arc::clone(&within);
        let judge: Judge = Arc::new(move |grant: &Lease| {
            grant.id.as_str() == "l_grant" && judged.load(Ordering::SeqCst)
        });
        let asked = [user.0, layer.0, bare.0];

        // Beyond the grant: not taken, whatever its scope, and the request
        // stays open for the user, whose answer is taken as given.
        let (id, mut answer) = clients.ask(
            &session,
            &asked,
            |_| approval_request(&session),
            Some(Arc::clone(&judge)),
        );
        for choice in [
            ApprovalChoice::AllowOnce,
            ApprovalChoice::AllowSession,
            ApprovalChoice::AllowProject,
        ] {
            let refused = clients.answer(layer, &id, approval(choice)).unwrap_err();
            assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        }
        // A supervisor with no grant answers nothing, even a denial.
        let refused = clients
            .answer(bare, &id, approval(ApprovalChoice::Deny))
            .unwrap_err();
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        assert!(answer.try_recv().is_err(), "a refused answer was delivered");
        clients
            .answer(user, &id, approval(ApprovalChoice::AllowSession))
            .unwrap();
        let taken = answer.try_recv().unwrap();
        assert_eq!(taken.response, approval(ApprovalChoice::AllowSession));
        assert!(!taken.by_supervisor);

        // A denial is taken, beyond the grant as within it.
        let (id, mut answer) = clients.ask(
            &session,
            &asked,
            |_| approval_request(&session),
            Some(Arc::clone(&judge)),
        );
        clients
            .answer(layer, &id, approval(ApprovalChoice::Deny))
            .unwrap();
        let taken = answer.try_recv().unwrap();
        assert_eq!(taken.response, approval(ApprovalChoice::Deny));
        assert!(taken.by_supervisor);
        // The first answer taken settles it: the user's is late.
        let late = clients
            .answer(user, &id, approval(ApprovalChoice::AllowOnce))
            .unwrap_err();
        assert_eq!(late.code, "request_not_open");

        // Within the grant: "for the project" is taken as once, its note
        // kept.
        within.store(true, Ordering::SeqCst);
        let (id, mut answer) = clients.ask(
            &session,
            &asked,
            |_| approval_request(&session),
            Some(Arc::clone(&judge)),
        );
        clients
            .answer(layer, &id, approval(ApprovalChoice::AllowProject))
            .unwrap();
        let taken = answer.try_recv().unwrap();
        assert_eq!(taken.response, approval(ApprovalChoice::AllowOnce));
        assert!(taken.by_supervisor);

        // A request with no judge (none can be made for its call) takes no
        // supervisor's allow.
        let (id, mut answer) = clients.ask(&session, &asked, |_| approval_request(&session), None);
        let refused = clients
            .answer(layer, &id, approval(ApprovalChoice::AllowOnce))
            .unwrap_err();
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        assert!(answer.try_recv().is_err());
    }

    #[test]
    fn a_disconnected_client_gets_nothing_more() {
        let clients = Clients::default();
        let (client, queue) = clients.connect(ClientInfo::user(), None);
        clients.disconnect(client);
        assert!(clients.queue(client).is_none());
        assert!(clients.info(client).is_none());
        assert_eq!(cortex_testkit::block_on(queue.next()), None);
    }
}
