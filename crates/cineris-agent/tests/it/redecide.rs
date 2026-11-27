//! A call planned and decided again when its turn to start comes
//! (`docs/architecture/03-agent-loop.md`, "Tool rounds", "A call is decided
//! again").
//!
//! A call is decided when it is planned, which can be before an earlier
//! call of the same response has run, or before a rule is taken back. So
//! when its turn to start comes, a call is planned and decided again if a
//! call that may have changed the file system has finished since it was
//! decided, or the policy's rules have narrowed (`Policy::narrowed`); and
//! not otherwise.
//!
//! Here the policy is scripted and the tools are fakes: a fake command
//! makes a link (`link`), and the policy decides by where a call's planned
//! path leads. The same cases with the real engine, the real tools and a
//! real shell are in `cineris-core`'s `redecide`.

use cineris_agent::{Decision, PlannedCall, Verdict};
use cineris_protocol::event::TurnOutcome;
use cineris_protocol::item::{ToolOutcome, ToolResultItem};
use cineris_protocol::{Item, Part, StopReason};
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

/// Whether a path the call's plan names leads into the directory
/// `guarded`: what the engine would hold back.
fn leads_into_guarded(call: &PlannedCall<'_>) -> bool {
    call.plan.effects.iter().any(|effect| {
        effect
            .path()
            .is_some_and(|path| path.abs().as_str().replace('\\', "/").contains("/guarded/"))
    })
}

/// A session whose workspace has a directory `guarded`.
fn with_guarded(world: &World) -> Rig {
    std::fs::create_dir_all(world.dir.join("s_1").join("guarded")).unwrap();
    Setup::new(world).start(world)
}

fn count(list: &[String], wanted: &str) -> usize {
    list.iter().filter(|seen| *seen == wanted).count()
}

/// The case the item names, with a fake command: `ln -s guarded d`, then a
/// write to `d/x`. The write is planned while `d` does not exist, so its
/// path is `d/x` in the workspace, and allowed. When its turn comes the
/// command has made the link: planned again, its path leads into
/// `guarded`, and the policy's new answer (ask) is acted on: the question
/// is put by the round itself, and refused, so nothing is written.
#[test]
fn a_write_through_a_link_an_earlier_call_made_is_planned_and_decided_again() {
    let response = calls(&[
        ("c1", "shell", json!({"id": "ln", "link": ["guarded", "d"]})),
        ("c2", "write", json!({"path": "d/x"})),
    ]);
    let world = World::new("redecide_write", [response, says("Done.")]);
    let rig = with_guarded(&world);
    rig.policy
        .decide_by(|call| leads_into_guarded(call).then_some(Decision::Ask));
    rig.policy.answer_for(
        "write",
        Verdict::Deny {
            note: Some("not into guarded".to_owned()),
        },
    );
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });

    assert_eq!(world.log.count("end shell ln"), 1);
    assert_eq!(world.log.plans("write d/x"), 2, "planned again");
    assert_eq!(count(&rig.policy.decided(), "write d/x"), 2);
    assert_eq!(
        rig.policy.asked(),
        ["write d/x"],
        "the round put the question"
    );
    assert_eq!(world.log.count("start write d/x"), 0, "nothing was written");
    let results = results(&rig);
    assert_eq!(results[1].outcome, ToolOutcome::Denied);
    assert!(text(&results[1]).contains("not into guarded"));
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Completed);
    assert_valid_history(&last_request(&world.provider).messages);
}

/// Where nothing that may change the file system has finished, and no rule
/// was taken back, no call is decided twice: reads, a call that touches
/// nothing, and a write after them, each decided once.
#[test]
fn nothing_is_decided_twice_when_nothing_ran_that_could_change_the_answer() {
    let response = calls(&[
        ("c1", "read", json!({"path": "a"})),
        ("c2", "plain", json!({"id": "p"})),
        ("c3", "read", json!({"path": "b"})),
        ("c4", "write", json!({"path": "c"})),
    ]);
    let world = World::new("redecide_count", [response, says("Done.")]);
    let rig = Setup::new(&world).start(&world);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let mut decided = rig.policy.decided();
    decided.sort();
    assert_eq!(
        decided,
        ["plain p", "read a", "read b", "write c"],
        "each call decided once"
    );
    for summary in ["read a", "plain p", "read b", "write c"] {
        assert_eq!(world.log.plans(summary), 1, "{summary}");
    }
    assert_eq!(world.log.count("end write c"), 1);
}

/// A call that waits behind one with side effects is decided again when
/// it finishes; a read that does not wait for it has run by then (the
/// write waits at a gate until it has), and is decided once.
#[test]
fn only_a_call_decided_before_a_call_with_side_effects_finished_is_decided_again() {
    let response = calls(&[
        ("c1", "write", json!({"path": "c", "gate": "c"})),
        ("c2", "read", json!({"path": "c"})),
        ("c3", "read", json!({"path": "e"})),
    ]);
    let world = World::new("redecide_waits", [response, says("Done.")]);
    let rig = Setup::new(&world).start(&world);
    world.run(async {
        rig.prompt("go");
        world.log.has("end read e").await;
        world.gates.open("c");
        rig.events.turns(1).await;
    });
    let decided = rig.policy.decided();
    assert_eq!(count(&decided, "write c"), 1);
    assert_eq!(
        count(&decided, "read c"),
        2,
        "it waited for the write: {decided:?}"
    );
    assert_eq!(
        count(&decided, "read e"),
        1,
        "it ran before the write ended"
    );
    let lines = world.log.lines();
    let at = |line: &str| lines.iter().position(|seen| seen == line).unwrap();
    assert!(at("end write c") < at("start read c"), "{lines:?}");
}

/// Point 2 of the design: a command the policy took as one that only reads
/// runs beside other calls. Decided again as one that may write, it has
/// not started, it is scheduled as its tool says (alone), and it waits for
/// a later read already running beside it.
#[test]
fn a_command_no_longer_taken_as_read_only_waits_for_a_later_read_already_running() {
    let response = calls(&[
        ("c1", "write", json!({"path": "w", "gate": "w"})),
        ("c2", "shell", json!({"id": "r"})),
        ("c3", "read", json!({"path": "x", "gate": "x"})),
    ]);
    let world = World::new("redecide_read_only", [response, says("Done.")]);
    let rig = Setup::new(&world).start(&world);
    let log = std::sync::Arc::clone(&world.log);
    // Read-only until the write has run; after it, a command that may write.
    rig.policy.decide_by(move |call| {
        (call.plan.summary == "shell r").then(|| {
            if log.count("end write w") == 0 {
                Decision::AllowReadOnly
            } else {
                Decision::Allow
            }
        })
    });
    world.run(async {
        rig.prompt("go");
        // The read runs beside the write and the command taken as only
        // reading, which waits for the write.
        world.log.has("start read x").await;
        world.gates.open("w");
        world.log.has("end write w").await;
        // The command has been decided again, and waits for the read.
        while count(&rig.policy.decided(), "shell r") < 2 {
            tokio::task::yield_now().await;
        }
        crate::settle().await;
        assert_eq!(
            world.log.count("start shell r"),
            0,
            "{:?}",
            world.log.lines()
        );
        world.gates.open("x");
        rig.events.turns(1).await;
    });
    let lines = world.log.lines();
    let at = |line: &str| lines.iter().position(|seen| seen == line).unwrap();
    assert!(at("end read x") < at("start shell r"), "{lines:?}");
    assert!(at("end write w") < at("start shell r"), "{lines:?}");
    assert_eq!(world.log.plans("shell r"), 2);
}

/// The twin from step 17: a rule taken back while an earlier call's
/// question is open. The later call was allowed when it was planned, and
/// waits behind the question; when its turn comes the policy's rules have
/// narrowed, and it is decided again, and refused.
#[test]
fn a_call_decided_before_a_rule_was_taken_back_is_decided_again() {
    let response = calls(&[
        ("c1", "plain", json!({"id": "q"})),
        ("c2", "write", json!({"path": "b"})),
    ]);
    let world = World::new("redecide_narrowed", [response, says("Done.")]);
    let rig = Setup::new(&world).start(&world);
    rig.policy.decide_for("plain q", Decision::Ask);
    rig.policy.answers_may_stop(true);
    rig.policy.wait_at(&world.gates);
    rig.policy.answer_for("plain", Verdict::Allow);
    world.run(async {
        rig.prompt("go");
        rig.policy.questions(1).await;
        // The write was decided, allowed, before its rule is taken back:
        // under some seeds its plan comes after the question is put.
        rig.policy.decisions(2).await;
        // While the question is open, the write's rule is taken back.
        rig.policy.decide_for(
            "write b",
            Decision::Deny {
                reason: Some("the rule was taken back".to_owned()),
            },
        );
        rig.policy.narrow();
        world.gates.open("ask plain");
        rig.events.turns(1).await;
    });
    assert_eq!(world.log.count("end plain q"), 1);
    assert_eq!(world.log.count("start write b"), 0);
    assert_eq!(count(&rig.policy.decided(), "write b"), 2);
    let results = results(&rig);
    assert_eq!(results[1].outcome, ToolOutcome::Denied);
    assert!(text(&results[1]).contains("the rule was taken back"));
}

/// The same response with no rule taken back: the write is decided once,
/// and runs. The call that was asked about changes nothing, so its run
/// does not count.
#[test]
fn a_question_answered_and_a_call_that_changes_nothing_do_not_make_a_second_decision() {
    let response = calls(&[
        ("c1", "plain", json!({"id": "q"})),
        ("c2", "write", json!({"path": "b"})),
    ]);
    let world = World::new("redecide_not_narrowed", [response, says("Done.")]);
    let rig = Setup::new(&world).start(&world);
    rig.policy.decide_for("plain q", Decision::Ask);
    rig.policy.answers_may_stop(true);
    rig.policy.answer_for("plain", Verdict::Allow);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(world.log.count("end write b"), 1);
    assert_eq!(count(&rig.policy.decided(), "write b"), 1);
}

/// A call waiting for its question to be put, while a call with side
/// effects finishes beside an earlier question, is decided again before
/// its question is put: the question is about the call as it is now.
#[test]
fn a_call_that_waits_to_be_asked_is_decided_again_before_it_is_asked() {
    let response = calls(&[
        ("c1", "plain", json!({"id": "q1"})),
        ("c2", "write", json!({"path": "w"})),
        ("c3", "plain", json!({"id": "q2"})),
    ]);
    let world = World::new("redecide_before_asking", [response, says("Done.")]);
    let rig = Setup::new(&world).start(&world);
    rig.policy.decide_for("plain q1", Decision::Ask);
    rig.policy.decide_for("plain q2", Decision::Ask);
    // A later write runs beside a question.
    rig.policy.answers_may_stop(false);
    rig.policy.wait_at(&world.gates);
    rig.policy.answer_all(Verdict::Allow);
    world.run(async {
        rig.prompt("go");
        rig.policy.questions(1).await;
        world.log.has("end write w").await;
        world.gates.open("ask plain");
        rig.policy.questions(2).await;
        assert_eq!(
            count(&rig.policy.decided(), "plain q2"),
            2,
            "decided again before its question was put"
        );
        rig.events.turns(1).await;
    });
    assert_eq!(rig.policy.asked(), ["plain q1", "plain q2"]);
    assert_eq!(world.log.count("end plain q2"), 1);
}

/// A second decision that says to stop the turn stops it at that call, in
/// the model's order: a later call that was waiting does not run.
#[test]
fn a_second_decision_that_stops_the_turn_stops_it_at_that_call() {
    let response = calls(&[
        ("c1", "shell", json!({"id": "ln", "link": ["guarded", "d"]})),
        ("c2", "write", json!({"path": "d/x"})),
        ("c3", "write", json!({"path": "y"})),
    ]);
    let world = World::new("redecide_stop", [response, says("Unused.")]);
    let rig = with_guarded(&world);
    rig.policy.decide_by(|call| {
        leads_into_guarded(call).then(|| Decision::Stop {
            reason: "it leads into guarded".to_owned(),
        })
    });
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Denied);
    assert_eq!(world.log.count("start write d/x"), 0);
    assert_eq!(world.log.count("start write y"), 0);
    let results = results(&rig);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].outcome, ToolOutcome::Success);
    assert_eq!(results[1].outcome, ToolOutcome::Denied);
    assert!(text(&results[1]).contains("leads into guarded"));
    assert_eq!(results[2].outcome, ToolOutcome::Interrupted);
}

/// A response that mixes reads, commands taken as read-only and calls with
/// side effects, one of which makes a link: under every seed's order, no
/// call reaches through the link without the policy having been shown
/// where it leads, and every call has one result. `cargo xtask stress`
/// runs it under many seeds.
#[test]
fn in_a_mixed_response_nothing_reaches_through_a_link_unjudged_whatever_the_order() {
    let response = calls(&[
        ("c1", "read", json!({"path": "a"})),
        ("c2", "shell", json!({"id": "cat"})),
        ("c3", "write", json!({"path": "b"})),
        ("c4", "shell", json!({"id": "ln", "link": ["guarded", "d"]})),
        ("c5", "read", json!({"path": "d/s"})),
        ("c6", "shell", json!({"id": "ls"})),
        ("c7", "write", json!({"path": "d/x"})),
        ("c8", "read", json!({"path": "e"})),
        ("c9", "write", json!({"path": "f"})),
    ]);
    let world = World::new("redecide_mixed", [response, says("Done.")]);
    let rig = with_guarded(&world);
    rig.policy.decide_by(|call| {
        if leads_into_guarded(call) {
            return Some(Decision::Deny {
                reason: Some("it leads into guarded".to_owned()),
            });
        }
        matches!(call.plan.summary.as_str(), "shell cat" | "shell ls")
            .then_some(Decision::AllowReadOnly)
    });
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    for reached in ["start read d/s", "start write d/x"] {
        assert_eq!(world.log.count(reached), 0, "{:?}", world.log.lines());
    }
    let results = results(&rig);
    assert_eq!(results.len(), 9);
    for (position, result) in results.iter().enumerate() {
        let expected = if matches!(position, 4 | 6) {
            ToolOutcome::Denied
        } else {
            ToolOutcome::Success
        };
        assert_eq!(
            result.outcome,
            expected,
            "call {}: {}",
            position + 1,
            text(result)
        );
    }
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Completed);
    assert_valid_history(&last_request(&world.provider).messages);
}

/// Found by the review of the design: a call that has not started, and
/// that a running call's end will have decided again, may then need an
/// answer that is a stop. While such a call runs, and an answer may be a
/// stop, a later call with side effects waits for it: the turn stops at
/// the call decided again, and the later call, written after it, has not
/// run.
#[test]
fn a_later_call_does_not_run_past_a_call_whose_second_decision_stops_the_turn() {
    // The response ends only once all three calls are decided, so the
    // round's first look after it starts the write and considers the
    // later call in one pass (`stress` found the pass missing a change it
    // had just started).
    let decided = cineris_testkit::Gate::default();
    let response = cineris_testkit::Response::new()
        .tool_call("c1", "write", &json!({"path": "a", "gate": "a"}))
        .tool_call("c2", "read", &json!({"path": "a"}))
        .tool_call("c3", "plain", &json!({"id": "s", "session": true}))
        .wait(&decided)
        .end(cineris_protocol::StopReason::ToolCalls);
    let world = World::new("redecide_stop_order", [response, says("Unused.")]);
    let rig = Setup::new(&world).start(&world);
    rig.policy.answers_may_stop(true);
    let log = std::sync::Arc::clone(&world.log);
    // The read is allowed until the write has run; decided after it, it
    // stops the turn.
    rig.policy.decide_by(move |call| {
        (call.plan.summary == "read a" && log.count("end write a") > 0).then(|| Decision::Stop {
            reason: "the write changed what it reads".to_owned(),
        })
    });
    world.run(async {
        rig.prompt("go");
        rig.policy.decisions(3).await;
        decided.open();
        world.log.has("start write a").await;
        crate::settle().await;
        assert_eq!(
            world.log.count("start plain s"),
            0,
            "{:?}",
            world.log.lines()
        );
        world.gates.open("a");
        rig.events.turns(1).await;
    });
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Denied);
    assert_eq!(
        world.log.count("start plain s"),
        0,
        "{:?}",
        world.log.lines()
    );
    let results = results(&rig);
    assert_eq!(results[1].outcome, ToolOutcome::Denied);
    assert_eq!(results[2].outcome, ToolOutcome::Interrupted);
}

/// Found by the review of the design: a call planned again onto a path a
/// later call has already used would act after it, against the model's
/// order. Here a write that makes a link runs beside a question; the write
/// after the question, planned while `d` was nothing, is planned again into
/// `guarded`, where a later read has already read. It does not run, and
/// its result says why.
#[test]
fn a_call_planned_again_onto_a_path_a_later_call_used_does_not_run() {
    let response = calls(&[
        (
            "c1",
            "write",
            json!({"path": "w", "gate": "w", "link": ["guarded", "d"]}),
        ),
        ("c2", "plain", json!({"id": "q"})),
        ("c3", "write", json!({"path": "d/x"})),
        ("c4", "read", json!({"path": "guarded/x"})),
    ]);
    let world = World::new("redecide_moved", [response, says("Done.")]);
    let rig = with_guarded(&world);
    rig.policy.decide_for("plain q", Decision::Ask);
    rig.policy.answers_may_stop(true);
    rig.policy.wait_at(&world.gates);
    rig.policy.answer_for("plain", Verdict::Allow);
    world.run(async {
        rig.prompt("go");
        world.log.has("end read guarded/x").await;
        world.gates.open("w");
        world.log.has("end write w").await;
        world.gates.open("ask plain");
        rig.events.turns(1).await;
    });
    assert_eq!(world.log.plans("write d/x"), 2);
    assert_eq!(
        world.log.count("start write d/x"),
        0,
        "{:?}",
        world.log.lines()
    );
    let results = results(&rig);
    assert_eq!(results[2].outcome, ToolOutcome::Error);
    assert!(
        text(&results[2]).contains("already used"),
        "{}",
        text(&results[2])
    );
}

/// A command the policy found only reads is on the table of commands that
/// only read, none of which makes a link: when it ends, nothing is decided
/// again.
#[test]
fn a_command_found_to_only_read_does_not_make_a_second_decision() {
    let response = calls(&[
        ("c1", "shell", json!({"id": "cat"})),
        ("c2", "write", json!({"path": "w"})),
    ]);
    let world = World::new("redecide_cat", [response, says("Done.")]);
    let rig = Setup::new(&world).start(&world);
    rig.policy.decide_for("shell cat", Decision::AllowReadOnly);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let lines = world.log.lines();
    let at = |line: &str| lines.iter().position(|seen| seen == line).unwrap();
    assert!(at("end shell cat") < at("start write w"), "{lines:?}");
    assert_eq!(count(&rig.policy.decided(), "write w"), 1);
}

/// A read kept from a response that was sent again is not used for its
/// call once the call was planned again onto another path: the run read
/// where the old path led (the open item
/// `a-kept-run-adopted-after-its-call-moved`, fixed in step 30). Here the
/// response sent again first calls a tool from outside that declares no
/// path and makes `d` a link into `guarded`, as an MCP server's tool can;
/// the read of `d/x` after it is planned again into `guarded`, and runs
/// again there.
#[test]
fn a_kept_read_is_not_used_for_a_call_planned_again_onto_another_path() {
    let fails = Gate::new();
    let response = Response::new()
        .tool_call("c1", "read", &json!({"path": "d/x"}))
        .wait(&fails)
        .event(retry(2))
        .event(start())
        .tool_call("c2", "solo", &json!({"id": "ln", "link": ["guarded", "d"]}))
        .tool_call("c3", "read", &json!({"path": "d/x"}))
        .end(StopReason::ToolCalls);
    let world = World::new("redecide_kept_moved", [response, says("Done.")]);
    let rig = with_guarded(&world);
    world.run(async {
        rig.prompt("go");
        // The read ran early, for the response that then failed.
        rig.events.seen("result:read:success", 1).await;
        fails.open();
        rig.events.turns(1).await;
    });
    assert_eq!(
        world.log.lines(),
        [
            "start read d/x",
            "end read d/x",
            "start solo ln",
            "end solo ln",
            "start read d/x",
            "end read d/x"
        ],
        "the read runs again, after the link: planned {} times, decided {:?}",
        world.log.plans("read d/x"),
        rig.policy.decided()
    );
    assert_eq!(
        world.log.plans("read d/x"),
        3,
        "once a response, and again after the link"
    );
    let results = results(&rig);
    assert_eq!(
        results
            .iter()
            .map(|result| result.call_id.to_string())
            .collect::<Vec<_>>(),
        ["c2", "c3"]
    );
    assert_eq!(results[1].outcome, ToolOutcome::Success);
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Completed);
    assert_valid_history(&last_request(&world.provider).messages);
}
