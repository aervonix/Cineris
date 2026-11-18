//! Where a turn's time goes: the parts add up to the wall time, each moment
//! in the part the turn was waiting on (`crate::timing` in the loop).

use std::time::Duration;

use cineris_agent::{Decision, RequestPurpose, StatEvent, ToolCallStat, TurnStat, Verdict};
use cineris_llm::ModelEvent;
use cineris_protocol::StopReason;
use cineris_testkit::Response;
use serde_json::json;

use crate::{Setup, World, says, settle};

/// The start of a response, put where a test wants it.
fn start() -> ModelEvent {
    ModelEvent::Start {
        response_id: None,
        model: "test-model".to_owned(),
    }
}

fn turns(stats: &[StatEvent]) -> Vec<TurnStat> {
    stats
        .iter()
        .filter_map(|event| match event {
            StatEvent::Turn(turn) => Some(turn.clone()),
            _ => None,
        })
        .collect()
}

fn tools(stats: &[StatEvent]) -> Vec<ToolCallStat> {
    stats
        .iter()
        .filter_map(|event| match event {
            StatEvent::ToolCall(call) => Some(call.clone()),
            _ => None,
        })
        .collect()
}

/// One turn, waited out step by step on the fake clock: 100 ms to connect,
/// 200 ms to the first byte, 300 ms streaming (a read started early runs
/// through it, and a question is put), 400 ms more of the read once the
/// response ended, then 500 ms waiting for the user's answer.
#[test]
fn a_turns_time_is_cut_into_parts_that_add_up_to_its_wall_time() {
    let world = World::new("time_split", []);
    let (connect, first_byte, stream) = (
        world.gates.get("connect"),
        world.gates.get("first byte"),
        world.gates.get("stream"),
    );
    world.provider.push(
        Response::events([])
            .wait(&connect)
            .event(ModelEvent::Answered)
            .wait(&first_byte)
            .event(start())
            .text("Reading, and asking.")
            .tool_call("c1", "read", &json!({"path": "a", "gate": "read"}))
            .tool_call("c2", "shell", &json!({"id": "deploy"}))
            .wait(&stream)
            .end(StopReason::ToolCalls),
    );
    world.provider.push(says("Done."));
    let rig = Setup::new(&world).start(&world);
    rig.policy.decide_for("shell", Decision::Ask);
    rig.policy.answer_for("shell", Verdict::Allow);
    rig.policy.wait_at(&world.gates);
    let step = |ms: u64| world.clock.advance(Duration::from_millis(ms));
    world.run(async {
        rig.prompt("go");
        while world.provider.requests().is_empty() {
            settle().await;
        }
        step(100);
        connect.open();
        settle().await;
        step(200);
        first_byte.open();
        rig.policy.questions(1).await;
        settle().await;
        step(300);
        stream.open();
        settle().await;
        step(400);
        world.gates.open("read");
        settle().await;
        step(500);
        world.gates.open("ask shell");
        rig.events.turns(1).await;
    });
    let stats = rig.stats.all();
    let turn = &turns(&stats)[0];
    let times = &turn.times.totals;
    assert_eq!(times.wall_ms, 1500);
    assert_eq!(
        times.parts_ms(),
        times.wall_ms,
        "the parts add up to the wall time: {times:?}"
    );
    assert_eq!(times.model.connect_ms, 100);
    assert_eq!(times.model.first_byte_ms, 200);
    assert_eq!(times.model.streaming_ms, 300);
    assert_eq!(times.tools_ms, 400);
    assert_eq!(times.approval_ms, 500);
    assert_eq!(times.harness_ms, 0);
    // The command ran at once once allowed: a share of nothing.
    assert_eq!(
        turn.times.by_tool,
        [("read".to_owned(), 400), ("shell".to_owned(), 0)]
    );
    assert_eq!(rig.events.ends()[0].duration_ms, Some(1500));

    // Each call's own times: the read ran through the streaming and after
    // it; the command waited for its answer from the moment it was asked.
    let calls = tools(&stats);
    let read = calls.iter().find(|call| call.tool == "read").unwrap();
    assert_eq!(read.duration, Duration::from_millis(700));
    assert_eq!(read.queued, Duration::ZERO);
    let shell = calls.iter().find(|call| call.tool == "shell").unwrap();
    assert_eq!(shell.approval, Duration::from_millis(1200));
    assert_eq!(shell.queued, Duration::ZERO);

    // The first request's own time is the model's part of it.
    let first = stats
        .iter()
        .find_map(|event| match event {
            StatEvent::Request(request) if request.purpose == RequestPurpose::Turn => {
                Some(request.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(first.time.connect_ms, 100);
    assert_eq!(first.time.first_byte_ms, 200);
    assert_eq!(first.time.streaming_ms, 300);
}

/// A retry's wait is the model's `retry`, and the time after it until the
/// next attempt is answered is its connection.
#[test]
fn a_retrys_wait_is_its_own_part() {
    let world = World::new("time_retry", []);
    let (wait, again) = (world.gates.get("wait"), world.gates.get("again"));
    world.provider.push(
        Response::events([])
            .wait(&wait)
            .event(ModelEvent::Retry(cineris_llm::RetryInfo {
                attempt: 2,
                max_attempts: 5,
                error: cineris_llm::error::ProviderError::new(
                    cineris_base::ErrorKind::Transient,
                    "overloaded",
                    "Busy.",
                ),
                wait: Duration::from_millis(1000),
                next_attempt_at: cineris_base::UtcTime::UNIX_EPOCH,
            }))
            .wait(&again)
            .event(ModelEvent::Answered)
            .event(start())
            .text("Done.")
            .end(StopReason::End),
    );
    let rig = Setup::new(&world).start(&world);
    world.run(async {
        rig.prompt("go");
        while world.provider.requests().is_empty() {
            settle().await;
        }
        world.clock.advance(Duration::from_millis(50));
        wait.open();
        settle().await;
        // The wait of a second, and 30 ms to connect again.
        world.clock.advance(Duration::from_millis(1030));
        again.open();
        rig.events.turns(1).await;
    });
    let stats = rig.stats.all();
    let times = &turns(&stats)[0].times.totals;
    assert_eq!(times.model.retry_ms, 1000);
    assert_eq!(times.model.connect_ms, 50 + 30);
    assert_eq!(times.parts_ms(), times.wall_ms);
}
