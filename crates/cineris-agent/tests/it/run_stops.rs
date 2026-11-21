//! A call whose run stopped the turn (`docs/architecture/03-agent-loop.md`,
//! "Tool rounds"): a confined command that changed protected paths, or an
//! extension's host call answered with a stop, marks its result
//! (`ToolResult::stopping_turn`), and the round ends at that call as it
//! ends at a call the policy stopped. The rig's tools mark theirs with the
//! `stop` argument.
//!
//! The call keeps its own result; nothing the model wrote after it runs;
//! the model is not asked again, even when the call was the last of its
//! response; and the turn ends `denied`, as front ends are told.

use cineris_agent::{Decision, Verdict};
use cineris_protocol::event::TurnOutcome;
use cineris_protocol::item::{ToolOutcome, ToolResultItem};
use cineris_protocol::{ErrorKind, Item, Part, StopReason};
use cineris_testkit::{Gate, Response};
use serde_json::json;

use crate::transitions::{retry, start};
use crate::{Rig, Setup, World, assert_valid_history, calls, last_request, says};

fn results(rig: &Rig) -> Vec<ToolResultItem> {
    rig.store
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect()
}

fn text(result: &ToolResultItem) -> String {
    result
        .content
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect()
}

const REASON: &str = "a confined command changed protected paths";

/// The stopped call is the last of its response: the turn ends `denied`
/// with the stop's reason, no request follows, and `turn.completed` is
/// streamed with that outcome. Before, the model was asked once more, and
/// a reply of text alone completed the turn.
#[test]
fn a_run_that_stops_the_turn_as_the_last_call_ends_it_without_asking_the_model_again() {
    let response = calls(&[("c1", "shell", json!({"id": "fix", "stop": REASON}))]);
    let world = World::new("run_stop_last", [response, says("Unused.")]);
    let rig = Setup::new(&world).start(&world);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });

    // No further request.
    assert_eq!(world.provider.requests().len(), 1);
    assert_eq!(world.provider.remaining(), 1, "the model was asked again");
    // The streamed end says `denied`, with the error a policy's stop gives.
    let ends = rig.events.ends();
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].outcome, TurnOutcome::Denied);
    let error = ends[0].error.as_ref().expect("a denied turn says why");
    assert_eq!(error.kind, ErrorKind::Permission);
    assert_eq!(error.code, "permission_denied");
    assert!(error.message.contains(REASON), "{error:?}");
    let labels = rig.events.labels();
    assert_eq!(
        labels
            .iter()
            .filter(|label| label.starts_with("turn.completed"))
            .collect::<Vec<_>>(),
        ["turn.completed:denied"],
        "{labels:?}"
    );
    // The call keeps the result its run gave: it ran.
    let results = results(&rig);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, ToolOutcome::Success);
    assert_eq!(text(&results[0]), "shell fix ok");
    assert_eq!(world.log.count("end shell fix"), 1);

    // The history is whole: the next prompt is answered.
    world.run(async {
        rig.prompt("carry on");
        rig.events.turns(2).await;
    });
    assert_eq!(rig.events.ends()[1].outcome, TurnOutcome::Completed);
    assert_valid_history(&last_request(&world.provider).messages);
}

/// A stop in the middle of a response: a call the model wrote before the
/// stopped one runs, nothing it wrote after it runs, each of those gets the
/// denial the policy gives every call of a stopped turn, and no request
/// follows.
#[test]
fn a_run_that_stops_the_turn_keeps_the_calls_after_it_from_running() {
    let response = calls(&[
        ("c1", "read", json!({"path": "a"})),
        ("c2", "shell", json!({"id": "fix", "stop": REASON})),
        ("c3", "write", json!({"path": "b"})),
        ("c4", "shell", json!({"id": "after"})),
    ]);
    let world = World::new("run_stop_middle", [response, says("Unused.")]);
    let rig = Setup::new(&world).start(&world);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });

    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Denied);
    assert_eq!(world.provider.requests().len(), 1);
    assert_eq!(world.log.count("end read a"), 1, "the read before it ran");
    assert_eq!(world.log.count("start write b"), 0);
    assert_eq!(world.log.count("start shell after"), 0);
    let results = results(&rig);
    assert_eq!(results.len(), 4, "one result for every call");
    assert_eq!(results[0].outcome, ToolOutcome::Success);
    assert_eq!(results[1].outcome, ToolOutcome::Success);
    for later in &results[2..] {
        assert_eq!(later.outcome, ToolOutcome::Denied, "{later:?}");
        assert!(
            text(later).contains("the turn was stopped") && text(later).contains(REASON),
            "{}",
            text(later)
        );
    }
}

/// An ordinary failing result is not a stop: the model is told, and asked
/// again, whether the call was the last of its response or not.
#[test]
fn a_failing_result_that_is_not_a_stop_lets_the_turn_go_on() {
    let response = calls(&[
        ("c1", "shell", json!({"id": "build", "fail": true})),
        ("c2", "write", json!({"path": "b"})),
        ("c3", "shell", json!({"id": "test", "fail": true})),
    ]);
    let world = World::new("fail_goes_on", [response, says("I will fix it.")]);
    let rig = Setup::new(&world).start(&world);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });

    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Completed);
    assert_eq!(
        world.provider.requests().len(),
        2,
        "the model was asked again"
    );
    assert_eq!(world.log.count("end write b"), 1, "the write after it ran");
    assert_eq!(world.log.count("end shell test"), 1);
    let results = results(&rig);
    assert_eq!(results[0].outcome, ToolOutcome::Error);
    assert_eq!(results[2].outcome, ToolOutcome::Error);
}

/// A run kept from a response that was thrown away, and used for the same
/// call of the response sent again, stops the turn when it is used.
#[test]
fn a_kept_run_that_stopped_the_turn_stops_it_when_the_retry_uses_it() {
    let fails = Gate::new();
    let read = json!({"path": "a", "stop": REASON});
    let response = Response::new()
        .tool_call("c1", "read", &read)
        .wait(&fails)
        .event(retry(2))
        .event(start())
        .tool_call("c9", "read", &read)
        .end(StopReason::ToolCalls);
    let (world, rig) = Rig::one("run_stop_kept", [response, says("Unused.")]);
    world.run(async {
        rig.prompt("go");
        rig.events.seen("result:read:success", 1).await;
        fails.open();
        rig.events.turns(1).await;
    });

    assert_eq!(world.log.count("start read a"), 1, "the tool ran once");
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Denied);
    assert_eq!(world.provider.remaining(), 1, "the model was asked again");
    let results = results(&rig);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].call_id.as_str(), "c9");
}

/// A stop over a call of a response that is thrown away goes with the
/// response: the policy is shown the call again if the next response makes
/// it, and a next response of text alone completes the turn. This is the
/// path where a headless run's result is still held to a denial recorded
/// under `--fail-on-denial` (`docs/architecture/10-headless.md`,
/// "Approvals"), and why that hold stays.
#[test]
fn a_stop_over_a_call_of_a_response_sent_again_goes_with_that_response() {
    let fails = Gate::new();
    let response = Response::new()
        .tool_call("c1", "shell", &json!({"id": "deploy"}))
        .wait(&fails)
        .event(retry(2))
        .event(start())
        .text("I will not deploy.")
        .end(StopReason::End);
    let (world, rig) = Rig::one("stop_voided", [response]);
    rig.policy.decide_for("shell", Decision::Ask);
    rig.policy.answer_for(
        "shell",
        Verdict::Stop {
            reason: "nobody can be asked".to_owned(),
        },
    );
    world.run(async {
        rig.prompt("go");
        rig.policy.questions(1).await;
        crate::settle().await;
        fails.open();
        rig.events.turns(1).await;
    });

    assert_eq!(rig.policy.asked(), ["shell deploy"]);
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Completed);
    assert!(results(&rig).is_empty());
    assert_eq!(world.log.count("start shell deploy"), 0);
}
