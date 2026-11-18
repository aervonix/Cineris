//! When a session ends: "Sessions end when their last client closes them
//! or disconnects" (`docs/architecture/11-rpc-protocol.md`), and every
//! session ends at shutdown. With `keep_sessions` (`serve --socket`) a
//! disconnect ends none, and a session with no client goes on.
//!
//! A session is put together off the core's thread, so a client can leave,
//! and a shutdown begin, while one is being put together for it. These
//! tests make that happen at a known step: every piece of blocking work
//! waits one turn of the runtime ([`World::one_turn_each`]), and the test
//! polls the operation by hand and acts between two of its steps. Found by
//! the review of step 13a.

use std::time::Duration;

use cineris_core::{ClientInfo, Outgoing};
use cineris_protocol::error::code;
use cineris_protocol::event::Event;
use cineris_protocol::methods::{
    ApprovalChoice, Op, OpResult, SessionForkParams, SessionNewParams, SessionReadParams,
    SessionReadResult,
};
use cineris_protocol::session::SessionSettings;
use cineris_protocol::{Item, Part, StopReason};
use cineris_testkit::{Gate, Response};
use serde_json::json;

use crate::{Front, World, calls, poll_once, says, settle};

fn new_session(world: &World, ephemeral: bool) -> Op {
    Op::SessionNew(SessionNewParams {
        cwd: None,
        workspace: world.workspace.to_str().unwrap().to_owned(),
        settings: SessionSettings::default(),
        title: None,
        config: serde_json::Map::new(),
        ephemeral,
    })
}

/// A client that disconnects while its `session/new` is being put
/// together leaves no session running with nobody on it.
#[test]
fn a_session_started_for_a_client_that_left_meanwhile_does_not_run_on() {
    let mut world = World::new("left_during_new", [says("Hi.")]);
    world.one_turn_each();
    let core = world.core();
    world.run(async {
        let leaving = core.connect(ClientInfo::user());
        let mut started = std::pin::pin!(core.handle(leaving.id, new_session(&world, true)));
        // The session is being put together: that work waits one turn.
        assert!(poll_once(started.as_mut()).await.is_pending());

        core.disconnect(leaving.id).await;
        let refused = started.await.expect_err("a session for a client that left");
        assert_eq!(refused.code, code::NOT_INITIALIZED);
        assert_eq!(core.open_sessions(), Vec::new(), "a session runs on");

        // A client that stays is given its session as before.
        let staying = Front::start(&world, &core).await;
        assert_eq!(core.open_sessions(), vec![staying.session.clone()]);
        staying.close().await;
        assert_eq!(core.open_sessions(), Vec::new());
    });
}

/// The same for `session/fork`, at each step: a client that leaves while
/// the fork is written refuses before any session is put together, and
/// one that leaves while the fork is put together leaves nothing running.
/// The session forked from was the client's alone, so it ends too.
#[test]
fn a_fork_for_a_client_that_left_meanwhile_does_not_run_on() {
    let mut world = World::new("left_during_fork", [says("One."), says("Two.")]);
    world.one_turn_each();
    let core = world.core();
    world.run(async {
        // Leaves after the first step (the fork written to disk), then
        // after the second (the fork put together).
        for steps in [1, 2] {
            let front = Front::start(&world, &core).await;
            front.turn("hello").await;
            let mut forked = std::pin::pin!(front.op(Op::SessionFork(SessionForkParams {
                session_id: front.session.clone(),
                at: None,
                before: None,
            })));
            for _ in 0..steps {
                assert!(
                    poll_once(forked.as_mut()).await.is_pending(),
                    "the fork was over in fewer than {steps} steps"
                );
            }
            core.disconnect(front.client.id).await;
            let refused = forked.await.expect_err("a fork for a client that left");
            assert_eq!(refused.code, code::NOT_INITIALIZED, "after {steps} steps");
            assert_eq!(
                core.open_sessions(),
                Vec::new(),
                "after {steps} steps, a session runs on"
            );
        }
    });
}

/// A shutdown that begins while a session is being put together ends that
/// session too, and no session starts after it.
#[test]
fn a_session_started_while_the_core_shuts_down_does_not_run_on() {
    let mut world = World::new("shutdown_during_new", [says("Hi.")]);
    world.one_turn_each();
    let core = world.core();
    world.run(async {
        let client = core.connect(ClientInfo::user());
        let mut started = std::pin::pin!(core.handle(client.id, new_session(&world, false)));
        assert!(poll_once(started.as_mut()).await.is_pending());

        core.shutdown().await;
        let refused = started
            .await
            .expect_err("a session started under a shutdown");
        assert_eq!(refused.code, code::SHUTTING_DOWN);
        assert_eq!(core.open_sessions(), Vec::new(), "a session runs on");

        // And after it, none starts.
        let after = core
            .handle(client.id, new_session(&world, true))
            .await
            .expect_err("a session started after a shutdown");
        assert_eq!(after.code, code::SHUTTING_DOWN);
        assert_eq!(core.open_sessions(), Vec::new());
    });
}

/// A session another client still has open is not ended when the client
/// that started it leaves, and goes on for the one that stayed.
#[test]
fn a_session_another_client_joined_goes_on_when_its_first_client_leaves() {
    let world = World::new("first_leaves", [says("Hi."), says("Still here.")]);
    let core = world.core();
    world.run(async {
        let first = Front::start(&world, &core).await;
        let second = first.join(ClientInfo::user()).await;
        core.disconnect(first.client.id).await;
        assert_eq!(core.open_sessions(), vec![first.session.clone()]);
        second.turn("hello").await;
        assert_eq!(second.last_text(), "Hi.");
        core.disconnect(second.client.id).await;
        assert_eq!(core.open_sessions(), Vec::new());
    });
}

// ------------------------------------------------- sessions kept running

const PROTECTED: &str = ".github/workflows/ci.yml";

/// How long a question waits in these tests: its wait on the test's timer
/// is the sign that it was put, and not refused.
const LIMIT: Duration = Duration::from_secs(600);

/// A protected write that the model makes once `gate` opens.
fn write_after(gate: &Gate) -> Response {
    Response::new()
        .wait(gate)
        .tool_call(
            "c1",
            "write",
            &json!({"path": PROTECTED, "content": "on: push\n"}),
        )
        .end(StopReason::ToolCalls)
}

/// The whole of the session, read by `front`.
async fn read_all(front: &Front) -> SessionReadResult {
    let result = front
        .op(Op::SessionRead(SessionReadParams {
            session_id: front.session.clone(),
            item_id: None,
            limit: None,
            cursor: None,
            redacted: false,
        }))
        .await
        .unwrap();
    let OpResult::SessionRead(result) = result else {
        panic!("not items");
    };
    result
}

/// With `keep_sessions` (`serve --socket`), a session goes on running when
/// its last client goes, and its turn completes with nobody watching; a
/// client that opens it later finds the turn over and the answer whole.
#[test]
fn with_keep_sessions_a_session_runs_on_when_its_client_goes() {
    let gate = Gate::new();
    let world = World::new(
        "kept_runs_on",
        [Response::new()
            .text_in(&["The build "])
            .wait(&gate)
            .text_in(&["fails."])
            .end(StopReason::End)],
    );
    let core = world.core_with(|options| options.keep_sessions = true);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("why does the build fail?").await;
        front
            .until(|seen| {
                seen.iter().any(|outgoing| {
                    matches!(outgoing, Outgoing::Event(event)
                        if matches!(&event.event, Event::ItemDelta(_)))
                })
            })
            .await;
        core.disconnect(front.client.id).await;
        assert_eq!(
            core.open_sessions(),
            vec![front.session.clone()],
            "the session ended with its client"
        );
        // The answer goes on with nobody to send it to.
        gate.open();
        settle().await;

        // Whether it has ended by now or not, the session is there to open,
        // and its turn ends with the answer whole.
        let again = front.join(ClientInfo::user()).await;
        if again.state.running_turn.is_some() {
            again.turns(1).await;
        }
        let result = read_all(&again).await;
        assert!(result.incomplete.is_empty(), "{:?}", result.incomplete);
        let Some(Item::AssistantMessage(answer)) = result.items.last() else {
            panic!("no answer: {:?}", result.items);
        };
        let text: String = answer
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "The build fails.");
        // A client that may drive it ends it by closing it.
        again.close().await;
        assert_eq!(core.open_sessions(), Vec::new());
    });
}

/// Without `keep_sessions` (headless, the terminal, `serve --stdio`), the
/// last client's disconnect ends its session, as before.
#[test]
fn without_keep_sessions_the_last_clients_disconnect_ends_the_session() {
    let world = World::new("not_kept", [says("Hi.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        core.disconnect(front.client.id).await;
        assert_eq!(core.open_sessions(), Vec::new(), "a session runs on");
    });
}

/// With `keep_sessions`, a call that asks in a session with no client at
/// all is put to nobody and waits: a `user` client that opens the session
/// is put it, and its answer counts.
#[test]
fn with_keep_sessions_a_question_with_no_client_waits_for_a_user_to_open_the_session() {
    let gate = Gate::new();
    let world = World::new("kept_asks", [write_after(&gate), says("Done.")]);
    let core = world.core_with(|options| {
        options.keep_sessions = true;
        options.approval_timeout = Some(LIMIT);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        core.disconnect(front.client.id).await;
        gate.open();
        // Not refused for want of somebody to ask: it waits.
        world.timer.sleeping_for(LIMIT).await;
        assert!(core.permission_denials(&front.session).is_empty());
        assert_eq!(world.provider.remaining(), 1, "the turn went on");
        assert_eq!(world.read(PROTECTED), None);

        // An observer that opens it is put nothing.
        let observer = front.join(ClientInfo::observer()).await;
        settle().await;
        observer.drain();
        assert!(observer.approvals().is_empty());

        let late = front.join(ClientInfo::user()).await;
        let (id, request) = late.approval().await;
        assert_eq!(request.call_id.as_str(), "c1");
        late.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        late.turns(1).await;
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: push\n"));
        assert!(core.permission_denials(&front.session).is_empty());
        late.close().await;
        observer.close().await;
    });
}

/// The question that waits for a client is denied at its time limit, as
/// any approval nobody answers is.
#[test]
fn with_keep_sessions_a_question_with_no_client_is_denied_at_its_time_limit() {
    let gate = Gate::new();
    let world = World::new("kept_times_out", [write_after(&gate), says("Nobody.")]);
    let core = world.core_with(|options| {
        options.keep_sessions = true;
        options.approval_timeout = Some(Duration::from_secs(30));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        core.disconnect(front.client.id).await;
        gate.open();
        world.timer.sleeping_for(Duration::from_secs(30)).await;
        // Something to see the turn end with, which is put nothing.
        let watcher = front.join(ClientInfo::observer()).await;
        world.timer.advance(Duration::from_secs(29));
        settle().await;
        watcher.drain();
        assert!(watcher.ends().is_empty(), "the limit has not passed");
        world.timer.advance(Duration::from_secs(2));
        watcher.turns(1).await;
        assert_eq!(world.read(PROTECTED), None);

        // The call was refused for the time.
        let result = read_all(&watcher).await;
        let refusal = result
            .items
            .iter()
            .find_map(|item| match item {
                Item::ToolResult(result) => Some(result.clone()),
                _ => None,
            })
            .expect("the call's result");
        assert_eq!(refusal.outcome.as_str(), "denied");
        assert!(
            format!("{:?}", refusal.content).contains("Nobody answered"),
            "{:?}",
            refusal.content
        );
        watcher.close().await;
    });
}

/// With `keep_sessions`, a session whose clients are there and answer no
/// approvals still gets the default at once: the call is refused.
#[test]
fn with_keep_sessions_a_client_that_answers_no_approvals_still_gets_the_default() {
    let world = World::new(
        "kept_headless",
        [
            calls(&[(
                "c1",
                "write",
                json!({"path": PROTECTED, "content": "on: push\n"}),
            )]),
            says("Refused."),
        ],
    );
    let core = world.core_with(|options| {
        options.keep_sessions = true;
        options.approval_timeout = Some(LIMIT);
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            ClientInfo::headless(),
            SessionSettings::default(),
        )
        .await
        .unwrap();
        front.prompt("set up CI").await;
        // Refused at once: the question never waits for a client to come.
        let mut waits = std::pin::pin!(world.timer.sleeping_for(LIMIT));
        let mut ended = std::pin::pin!(front.turns(1));
        std::future::poll_fn(|context| {
            assert!(
                waits.as_mut().poll(context).is_pending(),
                "the question waits for a client"
            );
            ended.as_mut().poll(context)
        })
        .await;
        assert_eq!(front.results()[0].1, "denied");
        assert_eq!(core.permission_denials(&front.session).len(), 1);
        front.close().await;
    });
}

/// With `keep_sessions`, a session put together for a client that left
/// meanwhile is still ended: nobody asked for it to be kept.
#[test]
fn with_keep_sessions_a_session_started_for_a_client_that_left_does_not_run_on() {
    let mut world = World::new("kept_left_during_new", [says("Hi.")]);
    world.one_turn_each();
    let core = world.core_with(|options| options.keep_sessions = true);
    world.run(async {
        let leaving = core.connect(ClientInfo::user());
        let mut started = std::pin::pin!(core.handle(leaving.id, new_session(&world, true)));
        assert!(poll_once(started.as_mut()).await.is_pending());
        core.disconnect(leaving.id).await;
        let refused = started.await.expect_err("a session for a client that left");
        assert_eq!(refused.code, code::NOT_INITIALIZED);
        assert_eq!(core.open_sessions(), Vec::new(), "a session runs on");
    });
}
