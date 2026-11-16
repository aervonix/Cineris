//! The verification stage's rules in the loop (step 19,
//! `docs/architecture/15-efficiency.md`, "Automatic verification"): when a
//! round runs, what the turn says of it, and what an interrupt, a stop and
//! a time limit do to it. The checks themselves are the core's; here a
//! verifier a test writes stands in for them.

use std::sync::Arc;
use std::time::Duration;

use cortex_agent::{Ledger, Verifier, VerifyCx, VerifyOutcome};
use cortex_protocol::event::{
    Notice, TurnOutcome, Verification, VerificationNotice, VerificationPhase,
};
use cortex_protocol::item::NoticeSource;
use cortex_protocol::lease::Budget;
use cortex_protocol::{Item, StopReason};
use cortex_testkit::{Gate, Response};
use cortex_tools::BoxFuture;
use serde_json::json;

use crate::{Log, Rig, Setup, World, calls, lock, says};

fn writes(path: &str) -> Response {
    calls(&[("c1", "write", json!({"path": path}))])
}

fn turn(world: &World, rig: &Rig) {
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
}

/// The rounds the scripted verifier was asked for: each one's number and
/// the paths it was told had changed, relative to the workspace.
fn rounds(rig: &Rig) -> Vec<(u32, usize)> {
    lock(&rig.verifier.rounds).clone()
}

#[test]
fn after_a_failure_the_checks_run_again_only_when_something_changed() {
    let (world, rig) = Rig::one(
        "verify_unchanged_after_failure",
        [writes("a"), says("Done."), says("I cannot fix it: …")],
    );
    lock(&rig.verifier.outcomes).push_back(VerifyOutcome::Failed {
        report: "the build failed".to_owned(),
    });
    turn(&world, &rig);
    // The model was told, changed nothing, and ended its turn: the same
    // checks on the same files would say the same.
    assert_eq!(rounds(&rig), [(1, 1)]);
    assert_eq!(world.provider.requests().len(), 3);
    let end = &rig.events.ends()[0];
    assert_eq!(end.outcome, TurnOutcome::Completed);
    assert_eq!(end.verification, Some(Verification::Failed));
}

#[test]
fn a_write_that_found_its_content_there_does_not_send_the_turn_to_the_checks() {
    let (world, rig) = Rig::one(
        "verify_unchanged_write",
        [
            calls(&[(
                "c1",
                "write",
                json!({"path": "a", "payload": {"path": "a", "kind": "unchanged"}}),
            )]),
            says("Done."),
        ],
    );
    turn(&world, &rig);
    assert_eq!(rounds(&rig), []);
    assert_eq!(
        rig.events.ends()[0].verification,
        Some(Verification::Skipped)
    );
}

#[test]
fn a_command_sends_the_turn_to_the_checks_by_what_its_look_found() {
    let ran = |name: &str, arguments: serde_json::Value| -> Vec<(u32, usize)> {
        let (world, rig) = Rig::one(name, [calls(&[("c1", "shell", arguments)]), says("Done.")]);
        turn(&world, &rig);
        rounds(&rig)
    };
    let changes = |modified: &[&str], complete: bool| {
        json!({"exit_code": 0, "changes": {
            "method": "git", "modified": modified, "created": [], "deleted": [],
            "complete": complete,
        }})
    };
    // A command declares a write of the whole workspace: that alone is
    // not a change.
    assert_eq!(
        ran("verify_cmd_quiet", json!({"payload": changes(&[], true)})),
        []
    );
    assert_eq!(ran("verify_cmd_unlooked", json!({})), []);
    // What its look found is.
    assert_eq!(
        ran(
            "verify_cmd_changed",
            json!({"payload": changes(&["src/a.rs", "src/b.rs"], true)})
        ),
        [(1, 2)]
    );
    // A command that failed has still changed what it changed.
    assert_eq!(
        ran(
            "verify_cmd_failed",
            json!({"fail": true, "payload": changes(&["src/a.rs"], true)})
        ),
        [(1, 1)]
    );
    // A look that did not finish: something may have changed.
    assert_eq!(
        ran(
            "verify_cmd_unknown",
            json!({"payload": changes(&[], false)})
        ),
        [(1, 1)]
    );
}

#[test]
fn a_check_that_stops_the_turn_ends_it_denied_and_the_model_is_told_why() {
    let (world, rig) = Rig::one(
        "verify_stopped",
        [writes("a"), says("Done."), says("Never.")],
    );
    lock(&rig.verifier.outcomes).push_back(VerifyOutcome::Stopped {
        reason: "`cargo test` changed `.git/hooks/pre-commit`, a protected path.".to_owned(),
    });
    turn(&world, &rig);
    let end = &rig.events.ends()[0];
    assert_eq!(end.outcome, TurnOutcome::Denied);
    assert_eq!(end.error.as_ref().unwrap().code, "permission_denied");
    assert_eq!(end.verification, Some(Verification::Incomplete));
    assert_eq!(world.provider.requests().len(), 2, "nothing more is asked");
    let told: Vec<String> = rig
        .store
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::Notice(notice) if notice.source == NoticeSource::Verification => {
                Some(notice.text)
            }
            _ => None,
        })
        .collect();
    assert_eq!(told.len(), 1);
    assert!(told[0].contains(".git/hooks/pre-commit"), "{told:?}");
}

#[test]
fn a_round_whose_checks_did_not_all_run_ends_the_turn_incomplete() {
    let (world, rig) = Rig::one("verify_incomplete", [writes("a"), says("Done.")]);
    lock(&rig.verifier.outcomes).push_back(VerifyOutcome::Incomplete);
    turn(&world, &rig);
    let end = &rig.events.ends()[0];
    assert_eq!(end.outcome, TurnOutcome::Completed);
    assert_eq!(end.verification, Some(Verification::Incomplete));
    assert_eq!(world.provider.requests().len(), 2);
}

/// A verifier that keeps what each round was told, and ends each round
/// with what `then` says.
struct Told {
    seen: std::sync::Mutex<Vec<(u32, u32, u32, Option<Duration>, Duration)>>,
    then: Box<dyn Fn(&VerifyCx<'_>) -> VerifyOutcome + Send + Sync>,
}

impl Verifier for Told {
    fn verify<'a>(&'a self, cx: VerifyCx<'a>) -> BoxFuture<'a, VerifyOutcome> {
        lock(&self.seen).push((cx.number, cx.round, cx.rounds, cx.deadline, cx.used));
        let outcome = (self.then)(&cx);
        Box::pin(std::future::ready(outcome))
    }
}

fn turn_budget(world: &World, seconds: u64) -> Ledger {
    Ledger::new(world.shared_clock()).with_turn_budget(Budget {
        wall_clock_ms: Some(seconds * 1000),
        ..Budget::default()
    })
}

#[test]
fn a_round_is_told_its_place_and_the_time_the_turns_limits_leave() {
    let world = World::new(
        "verify_told",
        [writes("a"), says("Done."), writes("b"), says("Fixed.")],
    );
    let clock = world.clock.clone();
    let verifier = Arc::new(Told {
        seen: std::sync::Mutex::default(),
        then: Box::new(move |cx| {
            // Each round takes twenty seconds on the fake clock.
            clock.advance(Duration::from_secs(20));
            if cx.round == 1 {
                VerifyOutcome::Failed {
                    report: "a test failed".to_owned(),
                }
            } else {
                VerifyOutcome::Passed
            }
        }),
    });
    let mut setup = Setup::new(&world);
    setup.ledger = Some(turn_budget(&world, 300));
    setup.verifier = Some(Arc::clone(&verifier) as Arc<dyn Verifier>);
    let rig = setup.start(&world);
    turn(&world, &rig);
    assert_eq!(
        *lock(&verifier.seen),
        [
            (1, 1, 2, Some(Duration::from_secs(300)), Duration::ZERO),
            (
                1,
                2,
                2,
                Some(Duration::from_secs(280)),
                Duration::from_secs(20)
            ),
        ]
    );
}

#[test]
fn no_round_starts_once_the_turns_time_is_spent() {
    let gate = Gate::new();
    let world = World::new(
        "verify_no_time",
        [
            writes("a"),
            Response::new()
                .wait(&gate)
                .text("Done.")
                .end(StopReason::End),
        ],
    );
    let mut setup = Setup::new(&world);
    setup.ledger = Some(turn_budget(&world, 60));
    let rig = setup.start(&world);
    world.run(async {
        rig.prompt("go");
        // The second response is under way, charged and sent, when the
        // turn's minute runs out.
        while world.provider.requests().len() < 2 {
            tokio::task::yield_now().await;
        }
        world.clock.advance(Duration::from_secs(61));
        gate.open();
        rig.events.turns(1).await;
    });
    assert_eq!(rounds(&rig), [], "the checks would run past the limit");
    let end = &rig.events.ends()[0];
    assert_eq!(end.outcome, TurnOutcome::Completed);
    assert_eq!(end.verification, Some(Verification::Skipped));
}

#[test]
fn the_verdict_stays_with_a_turn_that_then_runs_out_of_time() {
    let world = World::new(
        "verify_then_limit",
        [writes("a"), says("Done."), says("Never.")],
    );
    let clock = world.clock.clone();
    let verifier = Arc::new(Told {
        seen: std::sync::Mutex::default(),
        then: Box::new(move |_| {
            clock.advance(Duration::from_secs(61));
            VerifyOutcome::Failed {
                report: "a test failed".to_owned(),
            }
        }),
    });
    let mut setup = Setup::new(&world);
    setup.ledger = Some(turn_budget(&world, 60));
    setup.verifier = Some(verifier as Arc<dyn Verifier>);
    let rig = setup.start(&world);
    turn(&world, &rig);
    let end = &rig.events.ends()[0];
    assert_eq!(end.outcome, TurnOutcome::Limit);
    assert_eq!(
        end.verification,
        Some(Verification::Failed),
        "the failure is not lost with the turn"
    );
}

/// A verifier that, told to stop, takes its time to stop what it runs,
/// and stops at once on a second interrupt: what the runner does with a
/// check's process tree.
struct Slow {
    log: Arc<Log>,
}

impl Verifier for Slow {
    fn verify<'a>(&'a self, cx: VerifyCx<'a>) -> BoxFuture<'a, VerifyOutcome> {
        Box::pin(async move {
            self.log.push("checking".to_owned());
            cx.cancel.cancelled().await;
            self.log.push("asked to stop".to_owned());
            cx.kill.cancelled().await;
            self.log.push("killed".to_owned());
            VerifyOutcome::Skipped
        })
    }
}

#[test]
fn an_interrupt_waits_for_the_round_to_stop_what_it_runs() {
    let world = World::new("verify_interrupt_waits", [writes("a"), says("Done.")]);
    let mut setup = Setup::new(&world);
    setup.verifier = Some(Arc::new(Slow {
        log: Arc::clone(&world.log),
    }) as Arc<dyn Verifier>);
    let rig = setup.start(&world);
    world.run(async {
        rig.prompt("go");
        world.log.has("checking").await;
        let _ = rig.handle.interrupt();
        world.log.has("asked to stop").await;
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        // The check has not stopped yet, so the turn has not ended: a turn
        // that ended here would leave it running.
        assert!(
            rig.events.ends().is_empty(),
            "the turn waits for its checks"
        );
        let _ = rig.handle.interrupt_now();
        rig.events.turns(1).await;
    });
    assert_eq!(world.log.lines().last().map(String::as_str), Some("killed"));
    let end = &rig.events.ends()[0];
    assert_eq!(end.outcome, TurnOutcome::Interrupted);
    assert_eq!(end.verification, None);
    // Nothing of the round is in the conversation.
    assert!(
        !rig.store
            .items()
            .iter()
            .any(|item| matches!(item, Item::Notice(_)))
    );
}

/// A verifier that tells front ends of its round.
struct Telling;

impl Verifier for Telling {
    fn verify<'a>(&'a self, cx: VerifyCx<'a>) -> BoxFuture<'a, VerifyOutcome> {
        (cx.notify)(VerificationNotice {
            message: "Running the project's checks.".to_owned(),
            phase: VerificationPhase::Started,
            round: cx.round,
            rounds: cx.rounds,
            checks: Vec::new(),
            verdict: None,
        });
        Box::pin(std::future::ready(VerifyOutcome::Passed))
    }
}

#[test]
fn a_rounds_notices_reach_front_ends_with_its_turn() {
    let world = World::new("verify_notices", [writes("a"), says("Done.")]);
    let mut setup = Setup::new(&world);
    setup.verifier = Some(Arc::new(Telling) as Arc<dyn Verifier>);
    let rig = setup.start(&world);
    turn(&world, &rig);
    let told: Vec<(Option<String>, VerificationNotice)> = rig
        .events
        .all()
        .into_iter()
        .filter_map(|event| match event.event {
            cortex_protocol::event::Event::Notice(Notice::Verification(notice)) => {
                Some((event.turn_id.map(|turn| turn.to_string()), notice))
            }
            _ => None,
        })
        .collect();
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].0.as_deref(), Some("t_1"));
    assert_eq!(told[0].1.round, 1);
    assert_eq!(told[0].1.rounds, 2);
}
