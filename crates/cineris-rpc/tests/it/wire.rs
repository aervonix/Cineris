//! The server's own rules over the wire: the handshake, lines that are not
//! requests, the bounds on what a client may hold, the rule that closes a
//! client that takes nothing, and how a connection ends
//! (`docs/architecture/11-rpc-protocol.md`; `docs/steps/27/answered.md`,
//! points 2 to 4).

use std::time::Duration;

use cineris_core::{ClientInfo, Outgoing};
use cineris_protocol::event::Event;
use cineris_protocol::jsonrpc::{self, Message, RequestId, Response};
use cineris_protocol::methods::{Empty, Op, SessionNewParams, SessionRef, TurnStartParams};
use cineris_protocol::{Part, SessionId};
use cineris_rpc::Ended;
use cineris_testkit::conformance::{self, Carrier, anthropic};
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
        cwd: None,
        workspace: world.workspace.to_str().unwrap().to_owned(),
        settings: cineris_protocol::session::SessionSettings::default(),
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
    let cineris_protocol::methods::OpResult::SessionNew(opened) = opened else {
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
    let cineris_protocol::methods::OpResult::SessionNew(opened) = opened else {
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
        match cineris_testkit::block_on(watcher.queue.next()) {
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
    cineris_testkit::block_on(world.timer.sleeping_for(Duration::from_secs(1)));
    world.timer.advance(Duration::from_secs(31));
    assert_eq!(ended.recv().unwrap(), Ended::Stalled);
    assert_eq!(
        core.core.open_sessions(),
        [session],
        "the session ended with the client that stalled"
    );
}

/// A client that reads slowly holds the core's queue, not the server's: its
/// deltas merge, and once its queue is full an item's deltas give way to one
/// `item.resync`; what it is sent stays bounded however long the answer,
/// the turn completes for another client while it reads nothing, and the
/// completed item it is sent at last is whole.
#[test]
fn a_slow_client_is_sent_merged_deltas_and_a_resync() {
    let world = World::new("slow");
    let pieces: Vec<String> = (0..200).map(|piece| format!("piece {piece} ")).collect();
    let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
    world
        .server
        .push(anthropic::thinking_then_text("A thought.", &pieces));
    let core = world.core();
    let mut options = world.serve_options();
    options.queue = 3;
    let Wire {
        mut lines, ended, ..
    } = Wire::connection(&core, options, 1);
    lines
        .initialize(conformance::initialize())
        .expect("initialize");
    let opened = lines.call(new_session(&world)).expect("a session opens");
    let cineris_protocol::methods::OpResult::SessionNew(opened) = opened else {
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

    lines.write(&Message::Request(
        Op::TurnStart(TurnStartParams {
            session_id: session.clone(),
            input: vec![Part::text("go")],
            delivery: None,
        })
        .to_request(RequestId::Number(50))
        .unwrap(),
    ));
    // The model's stream runs to its end while the slow client reads
    // nothing, with no time passing.
    loop {
        match cineris_testkit::block_on(watcher.queue.next()) {
            Some(Outgoing::Event(event)) if matches!(event.event, Event::SessionIdle(_)) => break,
            Some(_) => {}
            None => panic!("the watcher's queue closed"),
        }
    }
    assert!(ended.try_recv().is_err(), "the slow client was closed");

    let (mut events, mut deltas, mut resyncs) = (0, 0, 0);
    let mut completed = Vec::new();
    loop {
        // The turn's own result is among them, and passed over.
        let Message::Notification(notification) = lines.read().expect("the connection is open")
        else {
            continue;
        };
        let Ok(cineris_protocol::ServerNotification::Event(event)) =
            cineris_protocol::ServerNotification::from_wire(notification)
        else {
            continue;
        };
        events += 1;
        match &event.event {
            Event::ItemDelta(_) => deltas += 1,
            Event::ItemResync(_) => resyncs += 1,
            Event::ItemCompleted(_) => completed.push(serde_json::to_string(&event).unwrap()),
            Event::SessionIdle(_) => break,
            _ => {}
        }
    }
    assert_eq!(resyncs, 1, "one resync, for the item whose deltas dropped");
    assert!(
        deltas <= 2,
        "{deltas} deltas reached a client that read nothing"
    );
    assert!(
        events <= 16,
        "{events} events were held for a client that read nothing, of 200 deltas"
    );
    let whole: String = pieces.concat();
    assert!(
        completed.iter().any(|item| item.contains(whole.trim_end())),
        "the completed item is not whole: {completed:?}"
    );
    assert!(ended.try_recv().is_err(), "the slow client was closed");
}

/// A project whose configuration allows a command: the session that opens
/// it asks the client whether to trust it first.
fn untrusted_project(world: &World) {
    world.put(
        ".cineris/config.toml",
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
/// not: `cineris_knowledge::store::AFTER_THE_STORE`, which this crate's tests
/// cannot name by its constant.
const AFTER_THE_STORE: &str = "cineris-knowledge: the store is written, before its good copy";

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
    let hold = cineris_testkit::Hold::new(AFTER_THE_STORE);
    wire.lines.write_raw(&request(
        7,
        "project/update",
        json!({"workspace": world.workspace, "add": [{"kind": "fact", "text": "It builds with make."}]}),
    ));
    hold.reach();
    wire.lines.write_raw(&request(8, "shutdown", json!({})));
    // The connection's end waits for its lane, and gives up at its bound.
    cineris_testkit::block_on(world.timer.sleeping_for(Duration::from_secs(3)));
    world.timer.advance(Duration::from_secs(3));
    // The server's end then waits for the requests still running.
    cineris_testkit::block_on(world.timer.sleeping_for(Duration::from_secs(77)));
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
    let hold = cineris_testkit::Hold::new(AFTER_THE_STORE);
    wire.lines.write_raw(&request(
        7,
        "project/update",
        json!({"workspace": world.workspace, "add": [{"kind": "fact", "text": "It builds with make."}]}),
    ));
    hold.reach();
    wire.lines.end_input();
    cineris_testkit::block_on(wire.ending.cancelled());
    hold.release();
    assert_eq!(wire.ended.recv().unwrap(), Ended::Closed);
}

/// The point where a session's loop has started a tool call and is about to
/// run it: `cineris_agent::BEFORE_A_CALL_RUNS`, which this crate's tests
/// cannot name by its constant.
const BEFORE_A_CALL_RUNS: &str = "round: a call has started, and its tool is about to run";

/// The end's bound begins before a `shutdown` ends the core, whose end
/// waits for each session's loop: here a loop held as it is about to run a
/// tool call. The end has begun while the loop is still held; begun only
/// after the core's end, it would wait for the hold's own time limit to let
/// the loop go.
#[test]
fn the_ends_bound_begins_before_a_shutdown_ends_the_core() {
    let world = World::new("ending-before-the-core");
    world.put("notes.txt", "notes\n");
    world
        .server
        .push(anthropic::tool("c1", "read", &json!({"path": "notes.txt"})));
    let core = world.core();
    let mut wire = Wire::connect(&core, world.serve_options());
    let opened = wire
        .lines
        .call(new_session(&world))
        .expect("a session opens");
    let cineris_protocol::methods::OpResult::SessionNew(opened) = opened else {
        panic!("not a session");
    };
    let hold = cineris_testkit::Hold::new(BEFORE_A_CALL_RUNS);
    wire.lines.write_raw(&request(
        7,
        "turn/start",
        json!({"session_id": opened.session.session_id, "input": [{"kind": "text", "text": "read it"}]}),
    ));
    hold.reach();
    wire.lines.write_raw(&request(8, "shutdown", json!({})));
    cineris_testkit::block_on(wire.ending.cancelled());
    hold.release();
    assert_eq!(wire.ended.recv().unwrap(), Ended::Shutdown);
    assert_eq!(
        hold.timed_out(),
        0,
        "the loop was let go by the hold's own limit"
    );
}

/// An observer's `shutdown` is refused, and does not begin the end: the
/// server goes on.
#[test]
fn a_refused_shutdown_does_not_begin_the_end() {
    let world = World::new("observer-shutdown");
    let core = world.core();
    let mut wire = Wire::serve(&core, world.serve_options(), 64);
    let mut params = conformance::initialize();
    params.role = cineris_protocol::methods::ClientRole::Observer;
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

/// A session at `unrestricted` is refused unless the server's own command
/// line allowed it (`CoreOptions::allow_unrestricted`, which `serve` sets):
/// as a level, through `session/set`, and as a configuration override
/// however it is spelled, since the core judges what the configuration
/// comes to (the review of step 27's second pull request found four
/// spellings that a check of the override's text let through).
#[test]
fn unrestricted_is_the_servers_command_lines_to_allow() {
    let world = World::new("unrestricted");
    let mut options = world.options();
    options.allow_unrestricted = false;
    let core = crate::CoreThread::start(options);
    let mut wire = Wire::connect(&core, world.serve_options());
    let workspace = world.workspace.to_str().unwrap().to_owned();
    for params in [
        json!({"workspace": workspace, "settings": {"level": "unrestricted"}}),
        json!({"workspace": workspace, "config": {"permissions": {"mode": "unrestricted"}}}),
        json!({"workspace": workspace, "config": {"permissions.mode": "unrestricted"}}),
        json!({"workspace": workspace, "config": {"\"permissions\".mode": "unrestricted"}}),
        json!({"workspace": workspace, "config": {"permissions . mode": "unrestricted"}}),
        json!({"workspace": workspace, "config": {"permissions": {" mode": "unrestricted"}}}),
        json!({"workspace": workspace, "config": {" permissions.mode": "unrestricted"}}),
    ] {
        let refused = wire
            .lines
            .call(Op::from_wire("session/new", Some(params.clone())).unwrap())
            .unwrap_err();
        assert_eq!(refused.code, "unrestricted_not_allowed", "{params}");
    }
    let opened = wire
        .lines
        .call(new_session(&world))
        .expect("a session opens");
    let cineris_protocol::methods::OpResult::SessionNew(opened) = opened else {
        panic!("not a session");
    };
    let raise = Op::from_wire(
        "session/set",
        Some(json!({"session_id": opened.session.session_id,
            "settings": {"level": "unrestricted"}})),
    )
    .unwrap();
    let refused = wire.lines.call(raise).unwrap_err();
    assert_eq!(refused.code, "unrestricted_not_allowed");

    // Allowed by the server's command line, a client has it.
    let allowed = world.core();
    let mut allowed = Wire::connect(&allowed, world.serve_options());
    let reached = allowed
        .lines
        .call(
            Op::from_wire(
                "session/new",
                Some(json!({"workspace": workspace, "settings": {"level": "unrestricted"}})),
            )
            .unwrap(),
        )
        .expect("a session at unrestricted");
    let cineris_protocol::methods::OpResult::SessionNew(reached) = reached else {
        panic!("not a session");
    };
    assert_eq!(
        reached.session.level,
        cineris_protocol::lease::Level::Unrestricted
    );
}

/// The role a client gives at `initialize` is the one the core holds it to:
/// an observer may watch a session another client runs, and may not start
/// one, or a turn in the one it watches.
#[test]
fn the_role_given_at_initialize_binds_the_client() {
    let world = World::new("roles");
    world.server.push(anthropic::text(&["Hello."]));
    let core = world.core();
    let mut user = Wire::connect(&core, world.serve_options());
    let cineris_protocol::methods::OpResult::SessionNew(opened) = user
        .lines
        .call(new_session(&world))
        .expect("a session opens")
    else {
        panic!("not a session");
    };
    let session = opened.session.session_id;

    let mut observer = Wire::serve(&core, world.serve_options(), 64);
    let mut params = conformance::initialize();
    params.role = cineris_protocol::methods::ClientRole::Observer;
    observer.lines.initialize(params).expect("initialize");
    let refused = observer.lines.call(new_session(&world)).unwrap_err();
    assert_eq!(refused.code, "role_not_allowed");
    observer
        .lines
        .call(Op::SessionOpen(SessionRef {
            session_id: session.clone(),
        }))
        .expect("an observer watches a running session");
    let refused = observer
        .lines
        .call(Op::TurnStart(TurnStartParams {
            session_id: session,
            input: vec![Part::text("go")],
            delivery: None,
        }))
        .unwrap_err();
    assert_eq!(refused.code, "role_not_allowed");
}

/// With the core keeping sessions, as `serve --socket` runs it, a session
/// survives its client's going: the session runs on, and another client
/// opens it again (an observer may open only a session that is running).
#[test]
fn a_kept_session_survives_its_client_and_another_opens_it() {
    let world = World::new("kept");
    world.server.push(anthropic::text(&["Hello."]));
    let mut options = world.options();
    options.keep_sessions = true;
    let core = crate::CoreThread::start(options);
    let Wire {
        mut lines, ended, ..
    } = Wire::connection(&core, world.serve_options(), 64);
    lines
        .initialize(conformance::initialize())
        .expect("initialize");
    let cineris_protocol::methods::OpResult::SessionNew(opened) =
        lines.call(new_session(&world)).expect("a session opens")
    else {
        panic!("not a session");
    };
    let session: SessionId = opened.session.session_id;
    lines
        .call(Op::TurnStart(TurnStartParams {
            session_id: session.clone(),
            input: vec![Part::text("go")],
            delivery: None,
        }))
        .expect("a turn starts");
    // The client goes: its input ends, and its connection with it.
    drop(lines);
    assert_eq!(ended.recv().unwrap(), Ended::Closed);
    assert_eq!(
        core.core.open_sessions(),
        std::slice::from_ref(&session),
        "the session ended with its client"
    );

    let mut observer = Wire::serve(&core, world.serve_options(), 64);
    let mut params = conformance::initialize();
    params.role = cineris_protocol::methods::ClientRole::Observer;
    observer.lines.initialize(params).expect("initialize");
    observer
        .lines
        .call(Op::SessionOpen(SessionRef {
            session_id: session,
        }))
        .expect("another client opens the kept session");
}

/// A supervisor connection is given the server's grant: it is put each
/// approval of the session it opens, at once with the user; its allow of a
/// call beyond the grant (the test lease allows `make` alone) is not taken;
/// and the user's answer settles the request.
#[test]
fn a_supervisor_cannot_answer_beyond_the_servers_grant_and_the_user_does() {
    use cineris_protocol::methods::{
        ApprovalChoice, ApprovalResponse, ClientRole, OpResult, ServerRequest, ServerResponse,
    };
    use conformance::Inbound;

    let world = World::new("supervisor");
    world
        .server
        .push(anthropic::tool("c1", "shell", &json!({"command": "ninja"})));
    world.server.push(anthropic::text(&["Refused."]));
    let core = world.core();
    let mut options = world.serve_options();
    options.supervisor_grant = Some(
        serde_json::from_value(json!({
            "id": "l_test",
            "grant": {
                "level": "supervised",
                "rules": [{"effect": "exec", "match": "make", "action": "allow"}],
                "sandbox": {"read": {"all": true}, "write": {"all": true},
                    "protect": {"all": false}, "network": "off"},
                "budget": {},
                "limits": {"merge_to_main": false}
            },
            "expires": {"kind": "end_of_session"},
            "granted_by": {"kind": "user"}
        }))
        .expect("the test lease reads"),
    );
    let mut user = Wire::connect(&core, options.clone());
    let OpResult::SessionNew(opened) = user.lines.call(new_session(&world)).expect("a session")
    else {
        panic!("not a session");
    };
    let session: SessionId = opened.session.session_id;
    let mut layer = Wire::serve(&core, options, 64);
    let mut params = conformance::initialize();
    params.role = ClientRole::Supervisor;
    layer.lines.initialize(params).expect("initialize");
    layer
        .lines
        .call(Op::SessionOpen(SessionRef {
            session_id: session.clone(),
        }))
        .expect("the supervisor opens the session");

    user.lines
        .call(Op::TurnStart(TurnStartParams {
            session_id: session.clone(),
            input: vec![Part::text("build it")],
            delivery: None,
        }))
        .expect("a turn starts");
    let approval = |lines: &mut conformance::Lines| loop {
        match lines.next() {
            Some(Inbound::Request(id, ServerRequest::Approval(_))) => break id,
            Some(_) => {}
            None => panic!("the connection closed"),
        }
    };
    let id = approval(&mut layer.lines);
    assert_eq!(approval(&mut user.lines), id, "the user was put it too");

    layer.lines.respond(
        id.clone(),
        ServerResponse::Approval(ApprovalResponse {
            choice: ApprovalChoice::AllowOnce,
            note: None,
        }),
    );
    // A request read after the answer: the answer was put to the core
    // before it, and was not taken, or the user's below would be too late.
    layer
        .lines
        .call(Op::PermissionsList(SessionRef {
            session_id: session.clone(),
        }))
        .expect("a supervisor reads");
    user.lines.respond(
        id,
        ServerResponse::Approval(ApprovalResponse {
            choice: ApprovalChoice::Deny,
            note: None,
        }),
    );
    let mut outcomes = Vec::new();
    loop {
        match user.lines.next() {
            Some(Inbound::Event(event)) => match &event.event {
                Event::ItemCompleted(_) => {
                    let value = serde_json::to_value(&event).unwrap();
                    if let Some(outcome) = value.pointer("/item/outcome").and_then(Value::as_str) {
                        outcomes.push(outcome.to_owned());
                    }
                }
                Event::SessionIdle(_) => break,
                _ => {}
            },
            Some(_) => {}
            None => panic!("the user's connection closed"),
        }
    }
    assert_eq!(outcomes, ["denied"], "the user's answer did not settle it");
}
