//! The conformance scenarios of the RPC server: the same steps through
//! every carrier, and what each client was sent, compared
//! (`docs/architecture/11-rpc-protocol.md`, "One contract, two carriers";
//! `docs/steps/27/answered.md`, "The design, before building", point 6).
//!
//! **A scenario is data:** its workspace's files, the scripted model's
//! replies (an Anthropic stream each, which the scripted SSE server sends),
//! and its steps: send an operation, answer the next request the server
//! puts, wait for an event of a type. [`scenarios`] is the one list; a later
//! step adds a scenario there, and every carrier runs it.
//!
//! **A carrier** is a way to the core ([`Carrier`]): in process (the test
//! binary's own, over `Core::handle` and the client's queue), the RPC
//! connection over the transport's in-memory ends, or the real binary's
//! `cineris serve --stdio`. The last two speak lines of JSON-RPC, which
//! [`Lines`] speaks for them. The carriers in process are the test
//! binaries': this crate cannot see the core.
//!
//! **What is compared** ([`Transcript`]): the result of each operation and
//! every event the steps took, in order, with ids numbered by their first
//! appearance, times and durations taken out, paths under the scenario's
//! folder written from it, and runs of deltas of one item and kind joined
//! into one: merging depends on how fast a reader is, so where a stream is
//! cut differs between carriers, and nothing else may. Events that arrive
//! while an operation's result is awaited are kept for the steps that wait
//! for events, and those no step took are not compared, so the transcript
//! does not depend on timing. Each carrier's `seq` is checked to increase,
//! and [`Lines`] checks that no event at or below a `session/read`'s `seq`
//! arrives after the read's result. A `session/read` with `redacted` (an
//! export's) must say in its result that it masked: a client writes
//! nothing without it, and a server that dropped the field on one carrier
//! would read the same as one that never knew it.
//!
//! **`usage.updated`** is compared as each session's last one, after the
//! sequence: the loop sends one at most once a frame while a response
//! arrives, by the clock, so where one falls among the deltas is timing,
//! and what the usage came to is not.
//!
//! **What it does not see,** by design: events after a scenario's last
//! step that waits for one, and where a result falls among the events
//! (each is kept at its step: when it arrives beside the events is the
//! carrier's timing, and the one rule about it, a read's fence, is checked
//! on the wire as said). An answer the server does not take shows as a
//! scenario that waits for good, which the test's time limit ends: the
//! wire gives no reply to an answer. A request is kept with its method and
//! its parameters.

#![expect(
    clippy::result_large_err,
    clippy::large_enum_variant,
    reason = "a test's carrier hands each result and event to the test once, whole: a box would \
              only add an allocation"
)]

use std::collections::{BTreeMap, VecDeque};

use cineris_protocol::SessionEvent;
use cineris_protocol::jsonrpc::{Message, Request, RequestId, Response};
use cineris_protocol::methods::{
    ApprovalChoice, ApprovalResponse, ClientCapabilities, ClientRole, ConfigGetParams, Empty,
    InitializeParams, Op, OpResult, PROTOCOL_VERSION, PeerInfo, ServerNotification, ServerRequest,
    ServerResponse, SessionArtifactParams, SessionListParams, SessionNewParams, SessionReadParams,
    SessionRef, SessionRewindParams, TurnInterruptParams, TurnStartParams, UsageReportParams,
    WorkspaceFilesParams, WorkspaceWarmParams,
};
use cineris_protocol::{ErrorData, ErrorKind, ItemId, SessionId};
use serde_json::{Map, Value, json};

use crate::server::ServerReply;

/// The model the scenarios' configuration names, and the replies say.
pub const MODEL: &str = "claude-opus-5-5";

/// Anthropic streams, as the scripted SSE server sends them.
pub mod anthropic {
    use serde_json::{Value, json};

    use super::MODEL;
    use crate::server::ServerReply;

    fn start() -> Value {
        json!({"type": "message_start", "message": {"model": MODEL, "id": "msg_test",
            "type": "message", "role": "assistant", "content": [], "stop_reason": null,
            "usage": {"input_tokens": 20, "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0, "output_tokens": 1}}})
    }

    fn end(stop: &str) -> [Value; 2] {
        [
            json!({"type": "message_delta", "delta": {"stop_reason": stop},
                "usage": {"output_tokens": 12}}),
            json!({"type": "message_stop"}),
        ]
    }

    /// Events as an Anthropic stream writes them.
    #[must_use]
    pub fn body(events: &[Value]) -> Vec<u8> {
        let mut body = String::new();
        for event in events {
            body.push_str("event: ");
            body.push_str(event["type"].as_str().unwrap_or("message"));
            body.push_str("\ndata: ");
            body.push_str(&event.to_string());
            body.push_str("\n\n");
        }
        body.into_bytes()
    }

    fn text_block(index: usize, pieces: &[&str]) -> Vec<Value> {
        let mut events = vec![json!({"type": "content_block_start", "index": index,
            "content_block": {"type": "text", "text": ""}})];
        for piece in pieces {
            events.push(json!({"type": "content_block_delta", "index": index,
                "delta": {"type": "text_delta", "text": piece}}));
        }
        events.push(json!({"type": "content_block_stop", "index": index}));
        events
    }

    /// The model says `pieces`, one delta each, and ends its turn.
    #[must_use]
    pub fn text(pieces: &[&str]) -> ServerReply {
        let mut events = vec![start()];
        events.extend(text_block(0, pieces));
        events.extend(end("end_turn"));
        ServerReply::sse().send(body(&events))
    }

    /// The model thinks `thought`, then says `pieces`, and ends its turn.
    #[must_use]
    pub fn thinking_then_text(thought: &str, pieces: &[&str]) -> ServerReply {
        let mut events = vec![
            start(),
            json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "thinking", "thinking": ""}}),
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "thinking_delta", "thinking": thought}}),
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "signature_delta", "signature": "c2lnbmVk"}}),
            json!({"type": "content_block_stop", "index": 0}),
        ];
        events.extend(text_block(1, pieces));
        events.extend(end("end_turn"));
        ServerReply::sse().send(body(&events))
    }

    /// The model calls `tool` with `input`, its arguments in two pieces.
    #[must_use]
    pub fn tool(id: &str, tool: &str, input: &Value) -> ServerReply {
        let arguments = input.to_string();
        let middle = arguments.len() / 2;
        let middle = (middle..arguments.len())
            .find(|at| arguments.is_char_boundary(*at))
            .unwrap_or(arguments.len());
        let mut events = vec![
            start(),
            json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "tool_use", "id": id, "name": tool, "input": {}}}),
        ];
        for piece in [&arguments[..middle], &arguments[middle..]] {
            events.push(json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "input_json_delta", "partial_json": piece}}));
        }
        events.push(json!({"type": "content_block_stop", "index": 0}));
        events.extend(end("tool_use"));
        ServerReply::sse().send(body(&events))
    }

    /// The model says `pieces` and ends its turn, once `held` opens: the
    /// response begins at once and marks `arrived`, then waits.
    #[must_use]
    pub fn held(pieces: &[&str], arrived: &crate::Gate, held: &crate::Gate) -> ServerReply {
        let mut events = vec![start()];
        events.extend(text_block(0, pieces));
        events.extend(end("end_turn"));
        ServerReply::sse()
            .mark(arrived)
            .wait(held)
            .send(body(&events))
    }

    /// The model begins to say `piece` and then sends nothing more, until
    /// the request is given up.
    #[must_use]
    pub fn text_that_stalls(piece: &str) -> ServerReply {
        let mut events = vec![start()];
        let block = text_block(0, &[piece]);
        // The block is never stopped: the stream stalls inside it.
        events.extend(block.into_iter().take(2));
        ServerReply::sse()
            .send(body(&events))
            .hang_until_closed(&crate::Gate::new())
    }
}

/// What a step may use of what came before it.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    /// The scenario's workspace, as the server's machine names it.
    pub workspace: String,
    /// The session the scenario opened last.
    pub session: Option<SessionId>,
    /// The items that completed, in order.
    pub items: Vec<ItemId>,
    /// Every session the scenario opened, closed at its end.
    pub opened: Vec<SessionId>,
}

impl Ctx {
    /// The session, which a step after `session/new` may rely on.
    ///
    /// # Panics
    ///
    /// When no session was opened.
    #[must_use]
    pub fn session(&self) -> SessionId {
        self.session
            .clone()
            .expect("a step that needs a session comes after one is opened")
    }

    /// `{ session_id }` of the session.
    #[must_use]
    pub fn session_ref(&self) -> SessionRef {
        SessionRef {
            session_id: self.session(),
        }
    }
}

/// Makes an operation from what came before.
pub type MakeOp = Box<dyn Fn(&Ctx) -> Op + Send + Sync>;

/// Answers a request.
pub type MakeAnswer = Box<dyn Fn(&ServerRequest) -> ServerResponse + Send + Sync>;

/// One step of a scenario.
pub enum Step {
    /// Sends an operation, and keeps its result.
    Call(MakeOp),
    /// Sends an operation and goes on: its result is kept at the
    /// [`Finish`](Self::Finish) after it (an open that asks a question
    /// first, which a step must answer meanwhile).
    Start(MakeOp),
    /// Waits for the result of the oldest operation started and not yet
    /// finished, and keeps it.
    Finish,
    /// Takes events until a request comes, and answers it.
    Answer(MakeAnswer),
    /// Takes events until one of this type (`turn.completed`).
    Until(&'static str),
    /// Opens a gate a scripted reply waits on: the model goes on.
    Open(crate::Gate),
    /// Waits until a scripted reply has marked a gate: the model's request
    /// has reached the server.
    Reached(crate::Gate),
}

impl std::fmt::Debug for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Call(_) => f.write_str("Call"),
            Self::Start(_) => f.write_str("Start"),
            Self::Finish => f.write_str("Finish"),
            Self::Answer(_) => f.write_str("Answer"),
            Self::Until(event) => write!(f, "Until({event})"),
            Self::Open(_) => f.write_str("Open"),
            Self::Reached(_) => f.write_str("Reached"),
        }
    }
}

/// A scenario.
#[derive(Debug)]
pub struct Scenario {
    /// Its name, for a failure's message.
    pub name: &'static str,
    /// The workspace's files: a path relative to it, and the text.
    pub files: Vec<(&'static str, &'static str)>,
    /// The scripted model's replies, in order.
    pub model: Vec<ServerReply>,
    /// The steps.
    pub steps: Vec<Step>,
}

fn call(make: impl Fn(&Ctx) -> Op + Send + Sync + 'static) -> Step {
    Step::Call(Box::new(make))
}

/// An operation from its method and its parameters as JSON: for the
/// methods whose parameters a scenario only writes out.
fn json_call(method: &'static str, params: impl Fn(&Ctx) -> Value + Send + Sync + 'static) -> Step {
    call(move |ctx| {
        Op::from_wire(method, Some(params(ctx)))
            .unwrap_or_else(|error| panic!("a scenario's `{method}` does not read: {error}"))
    })
}

fn prompt_with(text: &'static str, delivery: cineris_protocol::item::Delivery) -> Step {
    call(move |ctx| {
        Op::TurnStart(TurnStartParams {
            session_id: ctx.session(),
            input: vec![cineris_protocol::Part::text(text)],
            delivery: Some(delivery.clone()),
        })
    })
}

fn answer(make: impl Fn(&ServerRequest) -> ServerResponse + Send + Sync + 'static) -> Step {
    Step::Answer(Box::new(make))
}

fn new_session() -> Step {
    call(|ctx| {
        Op::SessionNew(SessionNewParams {
            cwd: None,
            workspace: ctx.workspace.clone(),
            settings: cineris_protocol::session::SessionSettings::default(),
            title: None,
            config: Map::new(),
            ephemeral: false,
        })
    })
}

fn prompt(text: &'static str) -> Step {
    call(move |ctx| {
        Op::TurnStart(TurnStartParams {
            session_id: ctx.session(),
            input: vec![cineris_protocol::Part::text(text)],
            delivery: None,
        })
    })
}

fn read() -> Step {
    call(|ctx| {
        Op::SessionRead(SessionReadParams {
            session_id: ctx.session(),
            item_id: None,
            limit: None,
            cursor: None,
            redacted: false,
        })
    })
}

/// A read of the whole session for an export (since step 25).
fn read_for_export() -> Step {
    call(|ctx| {
        Op::SessionRead(SessionReadParams {
            session_id: ctx.session(),
            item_id: None,
            limit: None,
            cursor: None,
            redacted: true,
        })
    })
}

fn close() -> Step {
    call(|ctx| Op::SessionClose(ctx.session_ref()))
}

/// The path of a file the agent may not write without asking.
const PROTECTED: &str = ".github/workflows/ci.yml";

/// Every scenario, in the order they run.
#[must_use]
pub fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "a turn that thinks and answers",
            files: vec![("README.md", "# app\n")],
            model: vec![anthropic::thinking_then_text(
                "The user greets.",
                &["Hello", ", there", "."],
            )],
            steps: vec![
                new_session(),
                prompt("hello"),
                Step::Until("session.idle"),
                read(),
                close(),
            ],
        },
        Scenario {
            name: "a call approved once",
            files: vec![],
            model: vec![
                anthropic::tool(
                    "c1",
                    "write",
                    &json!({"path": PROTECTED, "content": "on: push\n"}),
                ),
                anthropic::text(&["Written."]),
            ],
            steps: vec![
                new_session(),
                prompt("set up CI"),
                answer(|_| {
                    ServerResponse::Approval(ApprovalResponse {
                        choice: ApprovalChoice::AllowOnce,
                        note: None,
                    })
                }),
                Step::Until("session.idle"),
                read(),
                close(),
            ],
        },
        Scenario {
            name: "a call denied with a note",
            files: vec![],
            model: vec![
                anthropic::tool(
                    "c1",
                    "write",
                    &json!({"path": PROTECTED, "content": "on: push\n"}),
                ),
                anthropic::text(&["Understood."]),
            ],
            steps: vec![
                new_session(),
                prompt("set up CI"),
                answer(|_| {
                    ServerResponse::Approval(ApprovalResponse {
                        choice: ApprovalChoice::Deny,
                        note: Some("not today".to_owned()),
                    })
                }),
                Step::Until("session.idle"),
                read(),
                close(),
            ],
        },
        Scenario {
            name: "an interrupt mid-stream",
            files: vec![],
            model: vec![anthropic::text_that_stalls("Let me")],
            steps: vec![
                new_session(),
                prompt("think hard"),
                Step::Until("item.delta"),
                call(|ctx| Op::TurnInterrupt(TurnInterruptParams::new(ctx.session()))),
                Step::Until("session.idle"),
                read(),
                close(),
            ],
        },
        Scenario {
            name: "a rewind to the first prompt",
            files: vec![],
            model: vec![anthropic::text(&["One."]), anthropic::text(&["Two."])],
            steps: vec![
                new_session(),
                prompt("first"),
                Step::Until("session.idle"),
                prompt("second"),
                Step::Until("session.idle"),
                call(|ctx| {
                    Op::SessionRewind(SessionRewindParams {
                        session_id: ctx.session(),
                        to: ctx.items[1].clone(),
                        restore_files: false,
                        plan: None,
                    })
                }),
                read(),
                close(),
            ],
        },
        Scenario {
            name: "the reads of the method table",
            files: vec![("src/main.rs", "fn main() {}\n")],
            model: vec![],
            steps: vec![
                new_session(),
                call(|_| Op::ModelsList(Empty {})),
                call(|_| Op::ConfigGet(ConfigGetParams::default())),
                call(|ctx| Op::PermissionsList(ctx.session_ref())),
                call(|ctx| Op::PlanGet(ctx.session_ref())),
                call(|ctx| Op::UsageGet(ctx.session_ref())),
                call(|ctx| {
                    Op::UsageReport(UsageReportParams {
                        range: cineris_protocol::usage::UsageRange::Session,
                        group_by: Vec::new(),
                        session_id: Some(ctx.session()),
                        project: None,
                    })
                }),
                call(|ctx| {
                    Op::WorkspaceFiles(WorkspaceFilesParams {
                        session_id: ctx.session(),
                        limit: None,
                    })
                }),
                call(|_| Op::McpStatus(Empty {})),
                call(|_| Op::ExtensionsStatus(Empty {})),
                call(|ctx| {
                    Op::WorkspaceWarm(WorkspaceWarmParams {
                        session_id: ctx.session(),
                        paths: vec!["src/main.rs".to_owned()],
                    })
                }),
                call(|ctx| {
                    Op::SessionArtifact(SessionArtifactParams {
                        session_id: ctx.session(),
                        artifact: cineris_protocol::ArtifactId::new("a_not_there"),
                    })
                }),
                call(|ctx| {
                    Op::SessionList(SessionListParams {
                        workspace: Some(ctx.workspace.clone()),
                        ..SessionListParams::default()
                    })
                }),
                call(|ctx| {
                    Op::SessionOpen(SessionRef {
                        session_id: SessionId::new(format!("{}-not-there", ctx.session())),
                    })
                }),
                close(),
            ],
        },
        {
            // The reply begins, and holds before its events until the step
            // opens it: the steer or the queue comes while the model's
            // request is under way, never before it is sent.
            let (arrived, held) = (crate::Gate::new(), crate::Gate::new());
            Scenario {
                name: "steering a running turn",
                files: vec![],
                model: vec![
                    anthropic::held(&["First."], &arrived, &held),
                    anthropic::text(&["Also done."]),
                ],
                steps: vec![
                    new_session(),
                    prompt("first"),
                    Step::Reached(arrived),
                    prompt_with("and this too", cineris_protocol::item::Delivery::Steer),
                    Step::Open(held),
                    Step::Until("session.idle"),
                    read(),
                    close(),
                ],
            }
        },
        {
            // The reply begins, and holds before its events until the step
            // opens it: the steer or the queue comes while the model's
            // request is under way, never before it is sent.
            let (arrived, held) = (crate::Gate::new(), crate::Gate::new());
            Scenario {
                name: "a prompt queued behind a turn",
                files: vec![],
                model: vec![
                    anthropic::held(&["First."], &arrived, &held),
                    anthropic::text(&["Second."]),
                ],
                steps: vec![
                    new_session(),
                    prompt("first"),
                    Step::Reached(arrived),
                    prompt_with("then this", cineris_protocol::item::Delivery::Queue),
                    Step::Open(held),
                    Step::Until("session.idle"),
                    read(),
                    close(),
                ],
            }
        },
        Scenario {
            name: "compacting an idle session",
            files: vec![],
            model: vec![
                anthropic::text(&["One."]),
                anthropic::text(&["The user said one thing, and it was answered."]),
            ],
            steps: vec![
                new_session(),
                prompt("one"),
                Step::Until("session.idle"),
                json_call(
                    "session/compact",
                    |ctx| json!({"session_id": ctx.session()}),
                ),
                Step::Until("session.idle"),
                read(),
                close(),
            ],
        },
        Scenario {
            name: "trusting the project when asked",
            files: vec![(
                ".cineris/config.toml",
                "[[permissions.rules]]\neffect = \"exec\"\nmatch = \"cargo *\"\naction = \"allow\"\n",
            )],
            model: vec![],
            steps: vec![
                Step::Start(Box::new(|ctx| {
                    Op::SessionNew(SessionNewParams {
                        cwd: None,
                        workspace: ctx.workspace.clone(),
                        settings: cineris_protocol::session::SessionSettings::default(),
                        title: None,
                        config: Map::new(),
                        ephemeral: false,
                    })
                })),
                answer(|_| {
                    ServerResponse::Trust(cineris_protocol::methods::TrustResponse {
                        trusted: true,
                        never: false,
                        keep: false,
                    })
                }),
                Step::Finish,
                call(|ctx| Op::PermissionsList(ctx.session_ref())),
                close(),
            ],
        },
        Scenario {
            name: "the writes of the method table",
            files: vec![],
            model: vec![anthropic::text(&["Done."])],
            steps: vec![
                new_session(),
                prompt("one"),
                Step::Until("session.idle"),
                json_call(
                    "session/set",
                    |ctx| json!({"session_id": ctx.session(), "settings": {"level": "read-only"}}),
                ),
                json_call(
                    "plan/update",
                    |ctx| json!({"session_id": ctx.session(), "edits": []}),
                ),
                json_call(
                    "permissions/revoke",
                    |ctx| json!({"session_id": ctx.session(), "ids": ["a_not_there"]}),
                ),
                json_call("usage/refresh", |_| json!({"providers": []})),
                json_call("project/list", |ctx| json!({"workspace": ctx.workspace})),
                json_call("project/update", |ctx| {
                    json!({"workspace": ctx.workspace,
                        "add": [{"kind": "fact", "text": "It builds with make."}]})
                }),
                json_call("config/reload", |_| json!({})),
                json_call("verify/run", |ctx| json!({"session_id": ctx.session()})),
                call(|ctx| Op::TurnInterrupt(TurnInterruptParams::new(ctx.session()))),
                json_call(
                    "session/undo",
                    |ctx| json!({"session_id": ctx.session(), "restore_files": false}),
                ),
                read(),
                json_call("session/fork", |ctx| json!({"session_id": ctx.session()})),
                close(),
            ],
        },
        // Last, so that the scenarios before keep their folders' numbers.
        Scenario {
            name: "a session read for an export",
            files: vec![],
            model: vec![anthropic::text(&["Noted."])],
            steps: vec![
                new_session(),
                prompt("the token is ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8"),
                Step::Until("session.idle"),
                read_for_export(),
                close(),
            ],
        },
    ]
}

/// What the server sent that is not a result.
#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    /// An event of a session the client has open.
    Event(SessionEvent),
    /// A request to answer.
    Request(RequestId, ServerRequest),
}

/// A way to the core: in process, or over the wire.
pub trait Carrier {
    /// Sends an operation, and returns a ticket for its result. Never
    /// waits.
    fn start(&mut self, op: Op) -> u64;

    /// Waits for the result of the operation `ticket` names. Events and
    /// requests that come meanwhile are kept for [`next`](Self::next), and
    /// other results for their own tickets.
    ///
    /// # Errors
    ///
    /// The operation's error.
    fn finish(&mut self, ticket: u64) -> Result<OpResult, ErrorData>;

    /// Sends an operation and waits for its result.
    ///
    /// # Errors
    ///
    /// The operation's error.
    fn call(&mut self, op: Op) -> Result<OpResult, ErrorData> {
        let ticket = self.start(op);
        self.finish(ticket)
    }

    /// The next event or request, waiting for one. `None` when the
    /// connection ended.
    fn next(&mut self) -> Option<Inbound>;

    /// Answers a request.
    fn respond(&mut self, id: RequestId, response: ServerResponse);

    /// Drops every event and request that has arrived and no step took,
    /// without waiting: what a scenario left behind, so that the next one
    /// on the same carrier starts clean.
    fn forget(&mut self);
}

/// The client's side of the handshake every carrier sends first: a `user`
/// client that answers everything and wants deltas.
#[must_use]
pub fn initialize() -> InitializeParams {
    InitializeParams {
        protocol_version: PROTOCOL_VERSION,
        min_protocol_version: None,
        client: PeerInfo {
            name: "conformance".to_owned(),
            version: "0".to_owned(),
        },
        role: ClientRole::User,
        capabilities: ClientCapabilities {
            approvals: true,
            questions: true,
            trust: true,
            deltas: true,
            experimental: Vec::new(),
        },
    }
}

/// What a scenario left on one carrier, normalised for comparing.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcript(pub Vec<Value>);

/// Runs `scenario` on `carrier`, with the scenario's workspace at
/// `workspace` (as the server's machine names it). Each text of `known`
/// (the scenario's folder, the scripted server's address) is written as
/// the name given with it wherever it appears.
///
/// # Panics
///
/// When a step cannot be taken: the connection ended, or an event's `seq`
/// did not increase; or when a read for an export does not say it masked.
pub fn run(
    scenario: &Scenario,
    carrier: &mut dyn Carrier,
    workspace: &str,
    known: &[(&str, &str)],
) -> Transcript {
    let mut ctx = Ctx {
        workspace: workspace.to_owned(),
        ..Ctx::default()
    };
    let mut kept = Vec::new();
    let mut last_seq: BTreeMap<SessionId, u64> = BTreeMap::new();
    let mut take = |carrier: &mut dyn Carrier, ctx: &mut Ctx, kept: &mut Vec<Value>| {
        let next = carrier
            .next()
            .unwrap_or_else(|| panic!("{}: the connection ended", scenario.name));
        match next {
            Inbound::Event(event) => {
                let last = last_seq.entry(event.session_id.clone()).or_insert(0);
                assert!(
                    event.seq > *last,
                    "{}: `seq` went from {} to {}",
                    scenario.name,
                    last,
                    event.seq
                );
                *last = event.seq;
                if let cineris_protocol::event::Event::ItemCompleted(completed) = &event.event
                    && let Some(id) = completed.item.id()
                {
                    ctx.items.push(ItemId::new(id));
                }
                let value = serde_json::to_value(&event).unwrap_or(Value::Null);
                let kind = value["type"].as_str().unwrap_or_default().to_owned();
                kept.push(json!({"event": value}));
                Some((kind, None))
            }
            Inbound::Request(id, request) => {
                kept.push(json!({"request": request.method(),
                    "params": request.params().unwrap_or(Value::Null)}));
                Some((String::new(), Some((id, request))))
            }
        }
    };
    let mut started: VecDeque<(u64, &'static str)> = VecDeque::new();
    let keep = |ctx: &mut Ctx,
                kept: &mut Vec<Value>,
                method: &str,
                result: Result<OpResult, ErrorData>| {
        if let Ok(
            OpResult::SessionNew(opened)
            | OpResult::SessionOpen(opened)
            | OpResult::SessionFork(opened),
        ) = &result
        {
            ctx.session = Some(opened.session.session_id.clone());
            ctx.opened.push(opened.session.session_id.clone());
        }
        kept.push(match result {
            Ok(result) => json!({"method": method, "result": result.result().unwrap_or(Value::Null)}),
            Err(error) => json!({"method": method, "error": serde_json::to_value(error).unwrap_or(Value::Null)}),
        });
    };
    for step in &scenario.steps {
        match step {
            Step::Call(make) => {
                let op = make(&ctx);
                let method = op.method();
                let for_export = matches!(&op, Op::SessionRead(params) if params.redacted);
                let result = carrier.call(op);
                if for_export && let Ok(OpResult::SessionRead(read)) = &result {
                    assert!(
                        read.redacted,
                        "{}: a read for an export does not say it masked the items",
                        scenario.name
                    );
                }
                keep(&mut ctx, &mut kept, method, result);
            }
            Step::Start(make) => {
                let op = make(&ctx);
                let method = op.method();
                started.push_back((carrier.start(op), method));
            }
            Step::Finish => {
                let (ticket, method) = started
                    .pop_front()
                    .unwrap_or_else(|| panic!("{}: a finish with nothing started", scenario.name));
                let result = carrier.finish(ticket);
                keep(&mut ctx, &mut kept, method, result);
            }
            Step::Open(gate) => gate.open(),
            Step::Reached(gate) => assert!(
                gate.wait(std::time::Duration::from_secs(30)),
                "{}: the model's request never reached the server",
                scenario.name
            ),
            Step::Answer(make) => loop {
                if let Some((_, Some((id, request)))) = take(carrier, &mut ctx, &mut kept) {
                    carrier.respond(id, make(&request));
                    break;
                }
            },
            Step::Until(kind) => loop {
                if let Some((taken, _)) = take(carrier, &mut ctx, &mut kept)
                    && taken == *kind
                {
                    break;
                }
            },
        }
    }
    // Every session the scenario opened is closed, which ends its events:
    // those queued before a close's result come before it, and none after.
    // What no step took is dropped, so the next scenario on this carrier
    // starts clean.
    for session in std::mem::take(&mut ctx.opened) {
        let _ = carrier.call(Op::SessionClose(SessionRef {
            session_id: session,
        }));
    }
    carrier.forget();
    Transcript(normalise(&kept, known))
}

/// Numbers ids by their first appearance, takes out times, durations and
/// `seq`, writes each text of `known` as its name, and joins runs of
/// deltas of one item and kind.
#[must_use]
pub fn normalise(entries: &[Value], known: &[(&str, &str)]) -> Vec<Value> {
    // Every id first, in the order they appear: the id of a session, a
    // turn or an item is also written inside other text (a session's
    // temporary folder is named for it).
    let mut names = Names::default();
    for entry in entries {
        learn(entry, &mut names, None);
    }
    // A Windows path is written with `\` in some places and `/` in others
    // (a rule's pattern, the reasons it is quoted in): both are known.
    let slashed: Vec<(String, &str)> = known
        .iter()
        .filter(|(from, _)| from.contains('\\'))
        .map(|(from, to)| (from.replace('\\', "/"), *to))
        .collect();
    let mut every: Vec<(&str, &str)> = known.to_vec();
    every.extend(slashed.iter().map(|(from, to)| (from.as_str(), *to)));
    let known = every.as_slice();
    let mut out: Vec<Value> = Vec::new();
    // Each session's last `usage.updated`, in the order the sessions first
    // had one: compared after the sequence, which leaves them out.
    let mut usage: Vec<(Value, Value)> = Vec::new();
    for entry in entries {
        let entry = rename(entry, &mut names, known, None);
        // The loop sends `usage.updated` at most once a frame while a
        // response arrives, by the clock: whether one comes before or after
        // a delta is timing. What a session's usage came to is not.
        if entry.pointer("/event/type").and_then(Value::as_str) == Some("usage.updated") {
            let session = entry
                .pointer("/event/session_id")
                .cloned()
                .unwrap_or(Value::Null);
            match usage.iter_mut().find(|(known, _)| *known == session) {
                Some((_, last)) => *last = entry,
                None => usage.push((session, entry)),
            }
            continue;
        }
        // A delta that follows a delta of the same item and kind, and of
        // the same call, is added to it, as the core's queue merges them.
        if let (Some(last), Some(delta)) = (out.last_mut(), delta_of(&entry))
            && let Some(before) = delta_of(last)
            && before.0 == delta.0
            && before.1 == delta.1
            && continues(last, &entry)
        {
            join(last, &entry);
            continue;
        }
        out.push(entry);
    }
    out.extend(
        usage
            .into_iter()
            .map(|(_, last)| json!({"last_usage": last})),
    );
    out
}

fn delta_of(entry: &Value) -> Option<(String, String)> {
    let event = entry.get("event")?;
    if event.get("type")?.as_str()? != "item.delta" {
        return None;
    }
    Some((
        event.get("item_id")?.as_str()?.to_owned(),
        event.get("delta")?.get("kind")?.as_str()?.to_owned(),
    ))
}

/// The fields of a delta that carry its content, which a merged delta
/// holds joined: every other field (its kind, a call's id) is the same in
/// both, or the two are not one run.
const CONTENT: [&str; 2] = ["text", "json"];

/// Whether `next`'s delta continues `last`'s: every field but the content
/// is the same.
fn continues(last: &Value, next: &Value) -> bool {
    let (Some(Value::Object(before)), Some(Value::Object(after))) =
        (last.pointer("/event/delta"), next.pointer("/event/delta"))
    else {
        return false;
    };
    let same = |one: &Map<String, Value>, other: &Map<String, Value>| {
        one.iter()
            .filter(|(key, _)| !CONTENT.contains(&key.as_str()))
            .all(|(key, value)| other.get(key) == Some(value))
    };
    same(before, after) && same(after, before)
}

/// Adds the content of `next`'s delta to `last`'s.
fn join(last: &mut Value, next: &Value) {
    let (Some(Value::Object(into)), Some(Value::Object(from))) = (
        last.pointer_mut("/event/delta"),
        next.pointer("/event/delta"),
    ) else {
        return;
    };
    for key in CONTENT {
        if let (Some(Value::String(into)), Some(Value::String(more))) =
            (into.get_mut(key), from.get(key))
        {
            into.push_str(more);
        }
    }
}

/// The ids seen, each with its number; and those of a kind that is also
/// written inside other text.
#[derive(Debug, Default)]
struct Names {
    numbers: BTreeMap<String, String>,
    inside: Vec<String>,
}

impl Names {
    fn of(&mut self, id: &str) -> String {
        let next = format!("#{}", self.numbers.len() + 1);
        self.numbers.entry(id.to_owned()).or_insert(next).clone()
    }
}

/// Numbers each id `value` holds that `names` does not know yet.
fn learn(value: &Value, names: &mut Names, key: Option<&str>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                learn(value, names, Some(key));
            }
        }
        Value::Array(values) => {
            for value in values {
                learn(value, names, key);
            }
        }
        Value::String(text) if key.is_some_and(|key| key == "id" || key.ends_with("_id")) => {
            names.of(text);
            if key.is_some_and(|key| key.ends_with("_id")) && !names.inside.contains(text) {
                names.inside.push(text.clone());
            }
        }
        _ => {}
    }
}

fn rename(value: &Value, names: &mut Names, known: &[(&str, &str)], key: Option<&str>) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                // When things happened, how long they took, and a rate
                // measured against the clock.
                if key == "seq"
                    || key.ends_with("_at")
                    || key.ends_with("_ms")
                    || key.ends_with("_per_second")
                {
                    continue;
                }
                out.insert(key.clone(), rename(value, names, known, Some(key)));
            }
            Value::Object(out)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| rename(value, names, known, key))
                .collect(),
        ),
        Value::String(text) => {
            let is_id = key.is_some_and(|key| key == "id" || key.ends_with("_id"));
            if is_id {
                return Value::String(names.of(text));
            }
            let mut text = text.clone();
            for (from, to) in known {
                if !from.is_empty() {
                    text = text.replace(from, to);
                }
            }
            // An id of a session, a turn or an item inside other text, the
            // longest first.
            let mut inside: Vec<String> = names.inside.clone();
            inside.sort_by_key(|id| std::cmp::Reverse(id.len()));
            for id in inside {
                if id.len() >= 8 && text.contains(id.as_str()) {
                    let name = names.of(&id);
                    text = text.replace(id.as_str(), &name);
                }
            }
            Value::String(text)
        }
        other => other.clone(),
    }
}

/// A message read: a response with its id, or something else the server
/// sent.
type Read = Result<(RequestId, Result<Value, ErrorData>), Inbound>;

/// A JSON-RPC client over lines: the RPC connection's in-memory ends, or
/// the real binary's standard input and output.
pub struct Lines {
    send: Box<dyn FnMut(Vec<u8>) + Send>,
    receive: Box<dyn FnMut() -> Option<Vec<u8>> + Send>,
    next_id: i64,
    /// Events and requests that came while a result was awaited.
    kept: VecDeque<Inbound>,
    /// For each session, the `seq` of the last read's result: no event at
    /// or below it may come after it.
    fences: BTreeMap<SessionId, u64>,
    /// The operations sent and not yet answered: each one's method, and
    /// the session a read is of.
    pending: BTreeMap<u64, (&'static str, Option<SessionId>)>,
    /// Results that came while another was awaited.
    results: BTreeMap<u64, Result<Value, ErrorData>>,
}

impl std::fmt::Debug for Lines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lines").finish_non_exhaustive()
    }
}

impl Lines {
    /// A client that writes a line with `send` and waits for one with
    /// `receive` (`None` at the end of the stream).
    #[must_use]
    pub fn new(
        send: impl FnMut(Vec<u8>) + Send + 'static,
        receive: impl FnMut() -> Option<Vec<u8>> + Send + 'static,
    ) -> Self {
        Self {
            send: Box::new(send),
            receive: Box::new(receive),
            next_id: 0,
            kept: VecDeque::new(),
            fences: BTreeMap::new(),
            pending: BTreeMap::new(),
            results: BTreeMap::new(),
        }
    }

    /// Sends `initialize`, and returns its result.
    ///
    /// # Errors
    ///
    /// The server's refusal.
    pub fn initialize(&mut self, params: InitializeParams) -> Result<OpResult, ErrorData> {
        self.call(Op::Initialize(params))
    }

    /// Writes one message as a line.
    pub fn write(&mut self, message: &Message) {
        let mut line = serde_json::to_vec(message).unwrap_or_default();
        line.push(b'\n');
        (self.send)(line);
    }

    /// Writes raw bytes, as they are.
    pub fn write_raw(&mut self, bytes: &[u8]) {
        (self.send)(bytes.to_vec());
    }

    /// Ends what the client sends, and goes on reading: the end of the
    /// server's input.
    pub fn end_input(&mut self) {
        self.send = Box::new(|_| {});
    }

    /// The next message the server wrote, read as one. `None` at the end.
    ///
    /// # Panics
    ///
    /// When a line is not a JSON-RPC message: standard output carries
    /// nothing else.
    pub fn read(&mut self) -> Option<Message> {
        let line = (self.receive)()?;
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\n', '\r']);
        Some(
            serde_json::from_str::<Message>(text).unwrap_or_else(|error| {
                panic!("the server wrote a line that is not a JSON-RPC message ({error}): {text:?}")
            }),
        )
    }

    fn inbound(&mut self, message: Message) -> Option<Read> {
        match message {
            Message::Response(Response { id, result }) => {
                let id = id?;
                Some(Ok((
                    id,
                    result.map_err(|error| {
                        error.error_data().unwrap_or_else(|| {
                            ErrorData::new(ErrorKind::Internal, "jsonrpc", error.message)
                        })
                    }),
                )))
            }
            Message::Notification(notification) => {
                match ServerNotification::from_wire(notification) {
                    Ok(ServerNotification::Event(event)) => {
                        if let Some(fence) = self.fences.get(&event.session_id) {
                            assert!(
                                event.seq > *fence,
                                "event {} came after a read's result at {fence}",
                                event.seq
                            );
                        }
                        Some(Err(Inbound::Event(*event)))
                    }
                    _ => None,
                }
            }
            Message::Request(Request { id, method, params }) => {
                let request = ServerRequest::from_wire(&method, params)
                    .unwrap_or_else(|error| panic!("a request the client cannot read: {error}"));
                Some(Err(Inbound::Request(id, request)))
            }
        }
    }
}

impl Carrier for Lines {
    fn start(&mut self, op: Op) -> u64 {
        self.next_id += 1;
        let ticket = u64::try_from(self.next_id).unwrap_or(u64::MAX);
        let method = op.method();
        let read_session = match &op {
            Op::SessionRead(params) => Some(params.session_id.clone()),
            _ => None,
        };
        let request = op
            .to_request(RequestId::Number(self.next_id))
            .unwrap_or_else(|error| panic!("`{method}` cannot be written: {error}"));
        self.pending.insert(ticket, (method, read_session));
        self.write(&Message::Request(request));
        ticket
    }

    fn finish(&mut self, ticket: u64) -> Result<OpResult, ErrorData> {
        let (method, read_session) = self
            .pending
            .get(&ticket)
            .cloned()
            .unwrap_or_else(|| panic!("no operation {ticket} was started"));
        let value = loop {
            if let Some(result) = self.results.remove(&ticket) {
                break result;
            }
            let message = self
                .read()
                .unwrap_or_else(|| panic!("the connection ended before `{method}` was answered"));
            match self.inbound(message) {
                Some(Ok((RequestId::Number(answered), result))) => {
                    let answered = u64::try_from(answered).unwrap_or(u64::MAX);
                    assert!(
                        self.pending.contains_key(&answered),
                        "a response to a request not sent"
                    );
                    // A read's result is the fence of the events after it,
                    // from when it arrives.
                    if let (Ok(value), Some((_, Some(session)))) =
                        (&result, self.pending.get(&answered))
                        && let Some(seq) = value.get("seq").and_then(Value::as_u64)
                    {
                        self.fences.insert(session.clone(), seq);
                    }
                    self.results.insert(answered, result);
                }
                Some(Ok((other, _))) => panic!("a response to `{other}`, which was not sent"),
                Some(Err(inbound)) => self.kept.push_back(inbound),
                None => {}
            }
        };
        self.pending.remove(&ticket);
        let _ = read_session;
        value.and_then(|value| {
            OpResult::from_wire(method, value).map_err(|error| {
                ErrorData::new(ErrorKind::Internal, "unreadable", error.to_string())
            })
        })
    }

    fn next(&mut self) -> Option<Inbound> {
        if let Some(kept) = self.kept.pop_front() {
            return Some(kept);
        }
        loop {
            let message = self.read()?;
            match self.inbound(message) {
                Some(Err(inbound)) => return Some(inbound),
                Some(Ok((id, _))) => panic!("a response to `{id}`, which was not awaited"),
                None => {}
            }
        }
    }

    fn forget(&mut self) {
        self.kept.clear();
    }

    fn respond(&mut self, id: RequestId, response: ServerResponse) {
        let result = response.result().unwrap_or(Value::Null);
        self.write(&Message::Response(Response {
            id: Some(id),
            result: Ok(result),
        }));
    }
}
