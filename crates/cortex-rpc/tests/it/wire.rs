//! The server's own rules over the wire: the handshake, lines that are not
//! requests, the bounds on what a client may hold, the rule that closes a
//! client that takes nothing, and how a connection ends
//! (`docs/architecture/11-rpc-protocol.md`; `docs/steps/27/answered.md`,
//! points 2 to 4).

use std::time::Duration;

use cortex_core::{ClientInfo, Outgoing};
use cortex_protocol::event::Event;
use cortex_protocol::jsonrpc::{self, Message, RequestId, Response};
use cortex_protocol::methods::{Empty, Op, SessionNewParams, SessionRef, TurnStartParams};
use cortex_protocol::{Part, SessionId};
use cortex_rpc::Ended;
use cortex_testkit::conformance::{self, Carrier, anthropic};
use serde_json::{Map, Value, json};

use crate::{Wire, World};

/// The error of the next message, which must be a response.
fn error_of(wire: &mut Wire) -> (Option<RequestId>, i64, Option<String>) {
    match wire.lines.read() {
        Some(Message::Response(Response {
            id,
            result: Err(error),
        })) => {
            let code = error.error_data().map(|data| data.code);
            (id, error.code, code)
        }
        other => panic!("not an error response: {other:?}"),
    }
}

fn request(id: i64, method: &str, params: Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": id, "method": method, "params": params
    }))
    .unwrap();
    line.push(b'\n');
    line
}

fn new_session(world: &World) -> Op {
    Op::SessionNew(SessionNewParams {
        workspace: world.workspace.to_str().unwrap().to_owned(),
        settings: cortex_protocol::session::SessionSettings::default(),
        title: None,
        config: Map::new(),
        ephemeral: false,
    })
}

/// Nothing but `initialize` is taken before it, once; a refused
/// `initialize` leaves the connection open for another.
#[test]
fn nothing_is_taken_before_initialize_and_initialize_is_taken_once() {
    let world = World::new("handshake");
    let core = world.core();
    let mut wire = Wire::serve(&core, world.serve_options(), 64);
    wire.lines.write_raw(&request(1, "models/list", json!({})));
    assert_eq!(
        error_of(&mut wire),
        (
            Some(RequestId::Number(1)),
            jsonrpc::code::APPLICATION_ERROR,
            Some("not_initialized".to_owned())
        )
    );
    let mut params = serde_json::to_value(conformance::initialize()).unwrap();
    params["role"] = json!("overseer");
    wire.lines.write_raw(&request(2, "initialize", params));
    assert_eq!(error_of(&mut wire).2.as_deref(), Some("unknown_value"));
    wire.lines
        .initialize(conformance::initialize())
        .expect("a second initialize is taken after a refused one");
    assert!(wire.lines.call(Op::ModelsList(Empty {})).is_ok());
    let again = wire
        .lines
        .initialize(conformance::initialize())
        .unwrap_err();
    assert_eq!(again.code, "already_initialized");
}

/// A line that is not JSON, JSON that is not a message, an unknown method
/// and parameters that do not fit are each answered, and reading goes on.
#[test]
fn each_bad_line_is_answered_and_reading_goes_on() {
    let world = World::new("bad-lines");
    let core = world.core();
    let mut wire = Wire::connect(&core, world.serve_options());
    wire.lines.write_raw(b"{not json\n\n");
    assert_eq!(
        error_of(&mut wire),
        (None, jsonrpc::code::PARSE_ERROR, None)
    );
    wire.lines.write_raw(b"{\"jsonrpc\":\"2.0\",\"id\":3}\n");
    assert_eq!(
        error_of(&mut wire),
        (
            Some(RequestId::Number(3)),
            jsonrpc::code::INVALID_REQUEST,
            None
        )
    );
    wire.lines
        .write_raw(&request(4, "session/teleport", json!({})));
    assert_eq!(
        error_of(&mut wire),
        (
            Some(RequestId::Number(4)),
            jsonrpc::code::METHOD_NOT_FOUND,
            None
        )
    );
    wire.lines
        .write_raw(&request(5, "session/read", json!({"limit": 3})));
    assert_eq!(
        error_of(&mut wire),
        (
            Some(RequestId::Number(5)),
            jsonrpc::code::INVALID_PARAMS,
            None
        )
    );
    // A response to a request the server never put is dropped.
    wire.lines
        .write_raw(b"{\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{}}\n");
    assert!(wire.lines.call(Op::ModelsList(Empty {})).is_ok());
}

/// A stream that ends in the middle of a line: the cut line is a parse
/// error, and the connection ends as at the end of its input.
#[test]
fn a_stream_cut_mid_line_is_a_parse_error_and_the_end() {
    let world = World::new("cut");
    let core = world.core();
    let Wire {
        mut lines, ended, ..
    } = Wire::connect(&core, world.serve_options());
    lines.write_raw(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"meth");
    // The end of the server's input; the client keeps reading.
    lines.end_input();
    let Some(Message::Response(Response {
        id: None,
        result: Err(error),
    })) = lines.read()
    else {
        panic!("the cut line was not answered");
    };
    assert_eq!(error.code, jsonrpc::code::PARSE_ERROR);
    assert_eq!(ended.recv().unwrap(), Ended::Closed);
}

/// The end of the client's input ends its sessions, and the server.
#[test]
fn the_end_of_the_input_ends_the_clients_sessions_and_the_server() {
    let world = World::new("eof");
    let core = world.core();
    let wire = Wire::connect(&core, world.serve_options());
    let Wire {
        mut lines, ended, ..
    } = wire;
    let opened = lines.call(new_session(&world)).expect("a session opens");
    let cortex_protocol::methods::OpResult::SessionNew(opened) = opened else {
        panic!("not a session");
    };
    assert_eq!(
        core.core.open_sessions(),
        std::slice::from_ref(&opened.session.session_id)
    );
    drop(lines);
    assert_eq!(ended.recv().unwrap(), Ended::Closed);
    assert!(
        core.core.open_sessions().is_empty(),
        "the session outlived its client"
    );
    // The core itself has ended: it starts nothing more (the client's
    // disconnect alone would end its sessions, and leave the core running).
    let late = core.core.connect(ClientInfo::user());
    let refused = core
        .on({
            let core = core.core.clone();
            let op = new_session(&world);
            async move { core.handle(late.id, op).await }
        })
        .unwrap_err();
    assert_eq!(refused.code, "shutting_down");
}

/// `shutdown` stops the core, and is answered `{}` once it has.
#[test]
fn shutdown_is_answered_once_the_core_has_stopped() {
    let world = World::new("shutdown");
    let core = world.core();
    let Wire {
        mut lines, ended, ..
    } = Wire::connect(&core, world.serve_options());
    lines.call(new_session(&world)).expect("a session opens");
    let stopped = lines.call(Op::Shutdown(Empty {})).expect("shutdown");
    assert_eq!(stopped.result().unwrap(), json!({}));
    assert_eq!(ended.recv().unwrap(), Ended::Shutdown);
    assert!(core.core.open_sessions().is_empty());
    // The core starts nothing more.
    let observer = core.core.connect(ClientInfo::user());
    let refused = core
        .on({
            let core = core.core.clone();
            let op = new_session(&world);
            async move { core.handle(observer.id, op).await }
        })
        .unwrap_err();
    assert_eq!(refused.code, "shutting_down");
}

/// A client that sends more than may wait for it and reads nothing is
/// closed: what it is answered is bounded.
#[test]
fn a_client_that_floods_and_reads_nothing_is_closed() {
    let world = World::new("flood");
    let core = world.core();
    let mut options = world.serve_options();
    options.max_requests = 2;
    let mut wire = Wire::serve(&core, options, 1);
    wire.lines
        .initialize(conformance::initialize())
        .expect("initialize");
    let mut burst = Vec::new();
    for id in 10..40 {
        burst.extend(request(id, "models/list", json!({})));
    }
    wire.lines.write_raw(&burst);
    assert_eq!(wire.ended.recv().unwrap(), Ended::Flooded);
    // What reached the client: the two run, and the refusals up to the
    // bound, and no more.
    let mut answered = 0;
    let mut overloaded = 0;
    while let Some(message) = wire.lines.read() {
        if let Message::Response(Response { result, .. }) = message {
            match result {
                Ok(_) => answered += 1,
                Err(error) => {
                    assert_eq!(
                        error.error_data().map(|data| data.code).as_deref(),
                        Some("overloaded")
                    );
                    overloaded += 1;
                }
            }
        }
    }
    assert!(answered <= 2, "{answered} answered");
    assert!(overloaded <= 2, "{overloaded} refusals were held");
}

/// A client that takes nothing never holds the model's stream: a turn
/// completes for another client while it reads nothing, and once whole
/// events have waited past its queue's capacity for the time limit, its
/// connection is closed and the session goes on.
#[test]
fn a_client_that_takes_nothing_is_closed_and_the_session_goes_on() {
    let world = World::new("stall");
    let pieces: Vec<String> = (0..40).map(|piece| format!("piece {piece} ")).collect();
    let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
    world.server.push(anthropic::text(&pieces));
    let core = world.core();
    let mut options = world.serve_options();
    options.queue = 4;
    options.stalled_client = Duration::from_secs(30);
    // One connection of a server that outlives it, as a socket's are.
    let Wire {
        mut lines, ended, ..
    } = Wire::connection(&core, options, 1);
    lines
        .initialize(conformance::initialize())
        .expect("initialize");
    let opened = lines.call(new_session(&world)).expect("a session opens");
    let cortex_protocol::methods::OpResult::SessionNew(opened) = opened else {
        panic!("not a session");
    };
    let session: SessionId = opened.session.session_id;

    // Another client watches the session, and takes everything.
    let watcher = core.core.connect(ClientInfo::observer());
    let watching = {
        let core = core.core.clone();
        let session = session.clone();
        async move {
            core.handle(
                watcher.id,
                Op::SessionOpen(SessionRef {
                    session_id: session,
                }),
            )
            .await
        }
    };
    core.on(watching)
        .expect("the observer opens the running session");

    // The first client sends a prompt and reads nothing more.
    lines.write(&Message::Request(
        Op::TurnStart(TurnStartParams {
            session_id: session.clone(),
            input: vec![Part::text("go")],
            delivery: None,
        })
        .to_request(RequestId::Number(50))
        .unwrap(),
    ));
    // The turn completes for the watcher, with no time passing.
    loop {
        match cortex_testkit::block_on(watcher.queue.next()) {
            Some(Outgoing::Event(event)) if matches!(event.event, Event::SessionIdle(_)) => break,
            Some(_) => {}
            None => panic!("the watcher's queue closed"),
        }
    }
    assert!(
        ended.try_recv().is_err(),
        "the client was closed before its time"
    );
    // Its writer waits on it, and looks at its queue: past the limit, the
    // connection is closed.
    // The writer's look while a line waits: one second, told apart from
    // the core's own waits on the same timer.
    cortex_testkit::block_on(world.timer.sleeping_for(Duration::from_secs(1)));
    world.timer.advance(Duration::from_secs(31));
    assert_eq!(ended.recv().unwrap(), Ended::Stalled);
    assert_eq!(
        core.core.open_sessions(),
        [session],
        "the session ended with the client that stalled"
    );
}

/// A project whose configuration allows a command: the session that opens
/// it asks the client whether to trust it first.
fn untrusted_project(world: &World) {
    world.put(
        ".cortex/config.toml",
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = \"cargo *\"\naction = \"allow\"\n",
    );
}

/// Reads messages until a request of the server's, which it returns.
fn until_request(wire: &mut Wire) -> (RequestId, String) {
    loop {
        match wire.lines.read() {
            Some(Message::Request(request)) => return (request.id, request.method),
            Some(_) => {}
            None => panic!("the connection ended before a request"),
        }
    }
}

/// A client that leaves while its open waits on the trust question: the
/// question is withdrawn first, which ends the open, and only then is the
/// lane waited for, so the connection ends at once and no session is left
/// (with the lane waited for first, the open would hold the end for good:
/// the lane's bound is on the fake clock).
#[test]
fn a_client_that_leaves_while_its_open_waits_on_a_question_ends_at_once() {
    let world = World::new("leave-mid-open");
    untrusted_project(&world);
    let core = world.core();
    let mut wire = Wire::connect(&core, world.serve_options());
    let open = new_session(&world)
        .to_request(RequestId::Number(5))
        .unwrap();
    wire.lines.write(&Message::Request(open));
    let (_, method) = until_request(&mut wire);
    assert_eq!(method, "trust/request");
    let Wire { lines, ended, .. } = wire;
    drop(lines);
    assert_eq!(ended.recv().unwrap(), Ended::Closed);
    assert!(
        core.core.open_sessions().is_empty(),
        "a session outlived its client"
    );
}

/// `shutdown` while a session is being put together: the core stops, the
/// open still waiting ends, and its answer is written before the
/// shutdown's, which comes once the requests under way have ended (the
/// open item `shutdown-and-a-session-being-put-together`).
#[test]
fn a_shutdown_waits_for_the_requests_under_way_before_it_answers() {
    let world = World::new("shutdown-mid-open");
    untrusted_project(&world);
    let core = world.core();
    let mut wire = Wire::connect(&core, world.serve_options());
    let open = new_session(&world)
        .to_request(RequestId::Number(5))
        .unwrap();
    wire.lines.write(&Message::Request(open));
    let (_, method) = until_request(&mut wire);
    assert_eq!(method, "trust/request");
    let stop = Op::Shutdown(Empty {})
        .to_request(RequestId::Number(6))
        .unwrap();
    wire.lines.write(&Message::Request(stop));
    let mut answered = Vec::new();
    while let Some(message) = wire.lines.read() {
        if let Message::Response(Response {
            id: Some(id),
            result,
        }) = message
        {
            answered.push((
                id,
                result.map_err(|error| error.error_data().map(|data| data.code)),
            ));
        }
    }
    assert_eq!(
        answered,
        [
            (RequestId::Number(5), Err(Some("shutting_down".to_owned()))),
            (RequestId::Number(6), Ok(json!({}))),
        ]
    );
    assert_eq!(wire.ended.recv().unwrap(), Ended::Shutdown);
    assert!(core.core.open_sessions().is_empty());
}

/// The point where the project's store is written and its good copy is
/// not: `cortex_knowledge::store::AFTER_THE_STORE`, which this crate's tests
/// cannot name by its constant.
const AFTER_THE_STORE: &str = "cortex-knowledge: the store is written, before its good copy";

/// `shutdown` while an operation of the client's is still running past the
/// lane's bound (a change to the project's store, held as it is written,
/// which the core's own end does not wait for): the server still waits for
/// it, bounded, before it answers, so the operation's answer comes first.
/// Both bounds are on the fake clock: the lane's is let run out, and the
/// server's never is, so it ends only because the operation does.
#[test]
fn a_shutdown_waits_for_an_operation_still_running_past_the_lanes_bound() {
    let world = World::with_config("shutdown-mid-update", "[stats]\nenabled = false\n");
    let core = world.core();
    // Bounds of their own, told apart from the other waits on the timer.
    let mut options = world.serve_options();
    options.lane_end = Duration::from_secs(3);
    options.requests_end = Duration::from_secs(77);
    let mut wire = Wire::connect(&core, options);
    let hold = cortex_testkit::Hold::new(AFTER_THE_STORE);
    wire.lines.write_raw(&request(
        7,
        "project/update",
        json!({"workspace": world.workspace, "add": [{"kind": "fact", "text": "It builds with make."}]}),
    ));
    hold.reach();
    wire.lines.write_raw(&request(8, "shutdown", json!({})));
    // The connection's end waits for its lane, and gives up at its bound.
    cortex_testkit::block_on(world.timer.sleeping_for(Duration::from_secs(3)));
    world.timer.advance(Duration::from_secs(3));
    // The server's end then waits for the requests still running.
    cortex_testkit::block_on(world.timer.sleeping_for(Duration::from_secs(77)));
    hold.release();
    let mut answered = Vec::new();
    while let Some(message) = wire.lines.read() {
        if let Message::Response(Response { id: Some(id), .. }) = message {
            answered.push(id);
        }
    }
    assert_eq!(answered, [RequestId::Number(7), RequestId::Number(8)]);
    assert_eq!(wire.ended.recv().unwrap(), Ended::Shutdown);
}

/// The end's bound begins when serving stops, before the client's
/// disconnect and the wait for its lane, which wait for what holds them:
/// here a change to the project's store, held as it is written, keeps the
/// lane, and the end has begun all the same.
#[test]
fn the_ends_bound_begins_before_the_lane_is_waited_for() {
    let world = World::new("ending-before-the-lane");
    let core = world.core();
    let mut wire = Wire::connect(&core, world.serve_options());
    let hold = cortex_testkit::Hold::new(AFTER_THE_STORE);
    wire.lines.write_raw(&request(
        7,
        "project/update",
        json!({"workspace": world.workspace, "add": [{"kind": "fact", "text": "It builds with make."}]}),
    ));
    hold.reach();
    wire.lines.end_input();
    cortex_testkit::block_on(wire.ending.cancelled());
    hold.release();
    assert_eq!(wire.ended.recv().unwrap(), Ended::Closed);
}

/// The end's bound begins before a `shutdown` ends the core, which waits
/// for each session's close: here a change of level, held as it puts its
/// engine in force, which the core's end waits for.
#[test]
fn the_ends_bound_begins_before_a_shutdown_ends_the_core() {
    let world = World::new("ending-before-the-core");
    let core = world.core();
    let mut wire = Wire::connect(&core, world.serve_options());
    let opened = wire
        .lines
        .call(new_session(&world))
        .expect("a session opens");
    let cortex_protocol::methods::OpResult::SessionNew(opened) = opened else {
        panic!("not a session");
    };
    let hold = cortex_testkit::Hold::new(cortex_core::BEFORE_THE_NEW_LEVEL_IS_IN_FORCE);
    wire.lines.write_raw(&request(
        7,
        "session/set",
        json!({"session_id": opened.session.session_id, "settings": {"level": "read-only"}}),
    ));
    hold.reach();
    wire.lines.write_raw(&request(8, "shutdown", json!({})));
    cortex_testkit::block_on(wire.ending.cancelled());
    hold.release();
    assert_eq!(wire.ended.recv().unwrap(), Ended::Shutdown);
}

/// An observer's `shutdown` is refused, and does not begin the end: the
/// server goes on.
#[test]
fn a_refused_shutdown_does_not_begin_the_end() {
    let world = World::new("observer-shutdown");
    let core = world.core();
    let mut wire = Wire::serve(&core, world.serve_options(), 64);
    let mut params = conformance::initialize();
    params.role = cortex_protocol::methods::ClientRole::Observer;
    wire.lines.initialize(params).expect("initialize");
    let refused = wire.lines.call(Op::Shutdown(Empty {})).unwrap_err();
    assert_eq!(refused.code, "role_not_allowed");
    assert!(
        !wire.ending.is_cancelled(),
        "a refused shutdown began the end"
    );
    assert!(wire.lines.call(Op::ModelsList(Empty {})).is_ok());
}

/// A client that says `initialize` over and over and reads nothing is
/// closed, as one that floods with other requests is: each answer counts
/// until it is written.
#[test]
fn a_client_that_repeats_a_refused_initialize_and_reads_nothing_is_closed() {
    let world = World::new("initialize-flood");
    let core = world.core();
    let mut options = world.serve_options();
    options.max_requests = 2;
    let mut wire = Wire::serve(&core, options, 1);
    let mut params = serde_json::to_value(conformance::initialize()).unwrap();
    params["protocol_version"] = json!(99);
    let mut burst = Vec::new();
    for id in 10..40 {
        burst.extend(request(id, "initialize", params.clone()));
    }
    wire.lines.write_raw(&burst);
    assert_eq!(wire.ended.recv().unwrap(), Ended::Flooded);
}

/// Over the wire, a `session/read`'s result comes after every event at or
/// below its `seq` that the client was sent, however far behind the
/// client is: here it reads nothing while a turn's events wait for it,
/// then asks for the history, then reads everything.
#[test]
fn a_reads_result_comes_after_the_events_at_or_below_its_seq() {
    let world = World::new("read-after-events");
    let pieces: Vec<String> = (0..12).map(|piece| format!("piece {piece} ")).collect();
    let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
    world.server.push(anthropic::text(&pieces));
    let core = world.core();
    let Wire { mut lines, .. } = Wire::serve(&core, world.serve_options(), 1);
    lines
        .initialize(conformance::initialize())
        .expect("initialize");
    let opened = lines.call(new_session(&world)).expect("a session opens");
    let cortex_protocol::methods::OpResult::SessionNew(opened) = opened else {
        panic!("not a session");
    };
    let session: SessionId = opened.session.session_id;
    let watcher = core.core.connect(ClientInfo::observer());
    let watching = {
        let core = core.core.clone();
        let session = session.clone();
        async move {
            core.handle(
                watcher.id,
                Op::SessionOpen(SessionRef {
                    session_id: session,
                }),
            )
            .await
        }
    };
    core.on(watching)
        .expect("the observer opens the running session");
    lines.write_raw(&request(
        50,
        "turn/start",
        json!({"session_id": session, "input": [{"kind": "text", "text": "go"}]}),
    ));
    loop {
        match cortex_testkit::block_on(watcher.queue.next()) {
            Some(Outgoing::Event(event)) if matches!(event.event, Event::SessionIdle(_)) => break,
            Some(_) => {}
            None => panic!("the watcher's queue closed"),
        }
    }
    // The client's writer waits on it, with the turn's events queued.
    lines.write_raw(&request(51, "session/read", json!({"session_id": session})));
    let mut seqs = Vec::new();
    let mut read_at = None;
    while read_at.is_none() {
        match lines.read() {
            Some(Message::Notification(notification)) => {
                if let Some(seq) = notification
                    .params
                    .as_ref()
                    .and_then(|params| params["seq"].as_u64())
                {
                    seqs.push(seq);
                }
            }
            Some(Message::Response(Response {
                id: Some(RequestId::Number(51)),
                result,
            })) => {
                read_at = result.unwrap()["seq"].as_u64();
            }
            Some(_) => {}
            None => panic!("the connection ended before the read was answered"),
        }
    }
    let read_at = read_at.unwrap();
    assert!(!seqs.is_empty(), "no event came before the read");
    // Nothing at or below the read's `seq` comes after its result.
    lines.end_input();
    while let Some(message) = lines.read() {
        if let Message::Notification(notification) = message
            && let Some(seq) = notification
                .params
                .as_ref()
                .and_then(|params| params["seq"].as_u64())
        {
            assert!(
                seq > read_at,
                "event {seq} came after the read at {read_at}"
            );
        }
    }
}

/// A result counts against the budget by its own size: a client that
/// leaves large results unread is refused before they pile up.
#[test]
fn a_result_counts_against_the_budget_by_its_own_size() {
    let world = World::new("result-bytes");
    world
        .server
        .push(anthropic::text(&[&"many words ".repeat(400)]));
    let core = world.core();
    let mut options = world.serve_options();
    options.max_request_bytes = 4096;
    let Wire { mut lines, .. } = Wire::serve(&core, options, 1);
    lines
        .initialize(conformance::initialize())
        .expect("initialize");
    let opened = lines.call(new_session(&world)).expect("a session opens");
    let cortex_protocol::methods::OpResult::SessionNew(opened) = opened else {
        panic!("not a session");
    };
    let session = opened.session.session_id;
    lines
        .call(Op::TurnStart(TurnStartParams {
            session_id: session.clone(),
            input: vec![Part::text("go")],
            delivery: None,
        }))
        .expect("the turn starts");
    loop {
        if let Some(conformance::Inbound::Event(event)) = lines.next()
            && matches!(event.event, Event::SessionIdle(_))
        {
            break;
        }
    }
    // Two reads of the history, unread: the first fills the channel, the
    // second waits in the server, its writer waiting on the client.
    lines.write_raw(&request(60, "session/read", json!({"session_id": session})));
    lines.write_raw(&request(61, "session/read", json!({"session_id": session})));
    cortex_testkit::block_on(world.timer.sleeping_for(Duration::from_secs(1)));
    // Small as its line is, the next is past the bytes the results hold.
    lines.write_raw(&request(62, "session/read", json!({"session_id": session})));
    let mut refused = None;
    while refused.is_none() {
        match lines.read() {
            Some(Message::Response(Response {
                id: Some(RequestId::Number(62)),
                result,
            })) => {
                refused = Some(result.unwrap_err().error_data().map(|data| data.code));
            }
            Some(_) => {}
            None => panic!("the connection ended"),
        }
    }
    assert_eq!(refused.unwrap().as_deref(), Some("overloaded"));
}

/// The requests read with a `shutdown`, after it, are answered
/// `shutting_down`: none goes unanswered.
#[test]
fn the_requests_after_a_shutdown_are_answered() {
    let world = World::new("after-shutdown");
    let core = world.core();
    let mut wire = Wire::connect(&core, world.serve_options());
    let mut burst = request(8, "shutdown", json!({}));
    burst.extend(request(9, "models/list", json!({})));
    wire.lines.write_raw(&burst);
    let mut answered = Vec::new();
    while let Some(message) = wire.lines.read() {
        if let Message::Response(Response {
            id: Some(id),
            result,
        }) = message
        {
            answered.push((
                id,
                result
                    .err()
                    .and_then(|error| error.error_data())
                    .map(|data| data.code),
            ));
        }
    }
    assert_eq!(
        answered,
        [
            (RequestId::Number(9), Some("shutting_down".to_owned())),
            (RequestId::Number(8), None),
        ]
    );
}
