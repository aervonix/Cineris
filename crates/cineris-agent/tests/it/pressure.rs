//! [`Rig::fill`]: a session's first request sized to a named pressure from
//! what the session really sends, not from bytes counted by hand. Each
//! pressure is checked against what the loop then does, with the fake
//! tools and with the real ones, and with instruction files that the twin
//! must count too.

use std::sync::Arc;

use cineris_agent::{ContextSettings, RoleModel, StatEvent};
use cineris_protocol::event::TurnOutcome;
use cineris_testkit::{ScriptedProvider, fixtures};
use cineris_tools::{
    EditTool, Editing, ListTool, MemoryCheckpoints, PlanBoard, PlanTool, ReadTool, SearchTool,
    Toolbox, WriteTool,
};

use crate::{Fill, Filled, Rig, Setup, World, says};

/// The real tools that need no shell: what their declarations weigh is
/// what a session of the harness sends.
fn real_tools(world: &World) -> Toolbox {
    let editing = Editing::new(Arc::new(MemoryCheckpoints::new()), world.shared_clock());
    Toolbox::new()
        .with(ReadTool::new())
        .with(ListTool::new())
        .with(SearchTool::new())
        .with(WriteTool::new(editing.clone()))
        .with(EditTool::new(editing))
        .with(PlanTool::new(Arc::new(PlanBoard::new())))
}

/// The estimate of the session's first request, from its context record.
fn first_estimate(rig: &Rig) -> u64 {
    rig.stats
        .all()
        .into_iter()
        .find_map(|event| match event {
            StatEvent::Context(context) => Some(context.estimate),
            _ => None,
        })
        .expect("a request was sent")
}

/// The ways a session is put together that change its first request's
/// size: the fake tools, the real ones, and project instructions.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Fake,
    Real,
    Instructed,
}

const KINDS: [Kind; 3] = [Kind::Fake, Kind::Real, Kind::Instructed];

fn setup(world: &World, kind: Kind, auto_compact: bool, fast: Option<&ScriptedProvider>) -> Rig {
    let mut setup = Setup::new(world);
    setup.context = ContextSettings {
        auto_compact,
        ..ContextSettings::default()
    };
    if let Some(fast) = fast {
        setup.utility = Some(RoleModel::new(Arc::new(fast.clone()), fixtures::model()));
    }
    match kind {
        Kind::Fake => {}
        Kind::Real => setup.toolbox = Some(real_tools(world)),
        Kind::Instructed => {}
    }
    let rig = setup.start(world);
    if matches!(kind, Kind::Instructed) {
        let root =
            cineris_base::AbsPath::from_os(&std::fs::canonicalize(world.dir.join("s_1")).unwrap())
                .unwrap();
        rig.instructions.set(
            &root.join("AGENTS.md").unwrap(),
            "Follow the house rules. ".repeat(40),
        );
    }
    rig
}

#[test]
fn below_the_threshold_the_request_goes_as_it_is() {
    for kind in KINDS {
        let world = World::new("fill_below", [says("Fine.")]);
        let rig = setup(&world, kind, true, None);
        rig.window(6_000);
        let filled = rig.fill(Fill::Below, "go");
        world.run(async {
            rig.prompt("go");
            rig.events.turns(1).await;
        });
        let estimate = first_estimate(&rig);
        assert!(
            estimate.abs_diff(filled.estimate) <= 1,
            "{kind:?}: measured {estimate}, filled {filled:?}"
        );
        assert!(
            (estimate as f64) < filled.prepare,
            "{kind:?}: {estimate} is past the threshold: {filled:?}"
        );
        assert_eq!(world.provider.requests().len(), 1, "{kind:?}");
    }
}

#[test]
fn preparing_starts_a_summary_in_the_background() {
    for kind in KINDS {
        let world = World::new("fill_preparing", [says("Fine.")]);
        let fast = ScriptedProvider::new([says("A summary.")]);
        let rig = setup(&world, kind, true, Some(&fast));
        rig.window(6_000);
        let filled = rig.fill(Fill::Preparing, "go");
        world.run(async {
            rig.prompt("go");
            rig.events.turns(1).await;
        });
        let estimate = first_estimate(&rig) as f64;
        assert!(
            estimate > filled.prepare && estimate < filled.compact,
            "{kind:?}: {estimate} is not between the thresholds: {filled:?}"
        );
        assert_eq!(world.provider.requests().len(), 1, "{kind:?}");
        assert_eq!(
            fast.requests().len(),
            1,
            "{kind:?}: no summary was prepared"
        );
    }
}

/// Past the trigger, and past the window: with automatic compaction off
/// the turn stops before anything is sent, which it does only past the
/// trigger.
#[test]
fn past_the_trigger_and_past_the_window_the_turn_asks_for_a_compaction() {
    for fill in [Fill::Compacting, Fill::PastWindow] {
        for kind in KINDS {
            let world = World::new("fill_past", [says("Unreached.")]);
            let rig = setup(&world, kind, false, None);
            rig.window(6_000);
            let filled: Filled = rig.fill(fill, "go");
            #[expect(clippy::cast_precision_loss, reason = "small token counts")]
            let estimate = filled.estimate as f64;
            match fill {
                Fill::Compacting => assert!(
                    estimate > filled.compact && filled.estimate < filled.limit,
                    "{kind:?}: {filled:?}"
                ),
                _ => assert!(filled.estimate > filled.window, "{kind:?}: {filled:?}"),
            }
            world.run(async {
                rig.prompt("go");
                rig.events.turns(1).await;
            });
            let end = &rig.events.ends()[0];
            assert_eq!(end.outcome, TurnOutcome::Failed, "{kind:?} {fill:?}");
            assert_eq!(
                end.error.as_ref().unwrap().code,
                "compaction_needed",
                "{kind:?} {fill:?}"
            );
            assert!(world.provider.requests().is_empty(), "{kind:?} {fill:?}");
        }
    }
}

#[test]
fn the_twin_counts_the_tools_and_the_instructions() {
    let base = |kind| {
        let world = World::new("fill_base", [says("Fine.")]);
        let rig = setup(&world, kind, true, None);
        rig.fill(Fill::Below, "go").base_bytes
    };
    let (fake, real, instructed) = (base(Kind::Fake), base(Kind::Real), base(Kind::Instructed));
    assert!(
        real > fake + 1_000,
        "the real tools weigh more: {real} {fake}"
    );
    assert!(
        instructed >= fake + 900,
        "the instructions are in the prompt: {instructed} {fake}"
    );
}

#[test]
#[should_panic(expected = "cannot be reached")]
fn a_pressure_the_window_cannot_hold_is_refused() {
    let world = World::new("fill_too_small", [says("Unreached.")]);
    let rig = setup(&world, Kind::Real, true, None);
    // The real tools alone are past the trigger of so small a window.
    rig.window(800);
    let _ = rig.fill(Fill::Below, "go");
}

/// The estimate of each request the session sent, in order.
fn estimates(rig: &Rig) -> Vec<u64> {
    rig.stats
        .all()
        .into_iter()
        .filter_map(|event| match event {
            StatEvent::Context(context) => Some(context.estimate),
            _ => None,
        })
        .collect()
}

#[test]
fn the_first_request_of_a_later_turn_is_sized_with_the_history_it_carries() {
    for kind in KINDS {
        // The first answer is long, so that what the history adds is more
        // than any rounding.
        let world = World::new("fill_next", [says(&"One. ".repeat(100)), says("Two.")]);
        let fast = ScriptedProvider::new([says("A summary.")]);
        let rig = setup(&world, kind, true, Some(&fast));
        rig.window(6_000);
        world.run(async {
            rig.prompt("first");
            rig.events.turns(1).await;
        });
        let first = estimates(&rig)[0];
        assert!(
            fast.requests().is_empty(),
            "{kind:?}: nothing to prepare yet"
        );

        let filled = rig.fill_next(Fill::Preparing, "second");
        world.run(async {
            rig.prompt("second");
            rig.events.turns(2).await;
        });

        let measured = estimates(&rig)[1];
        // The twin is resumed from the items, and the session goes on in
        // its epoch: they may differ by the few tokens of a notice.
        assert!(
            measured.abs_diff(filled.estimate) <= 8,
            "{kind:?}: measured {measured}, filled {filled:?}"
        );
        assert!(
            (measured as f64) > filled.prepare && (measured as f64) < filled.compact,
            "{kind:?}: {measured} is not between the thresholds: {filled:?}"
        );
        assert!(
            filled.base_bytes > first * 3 + 450,
            "{kind:?}: the history is in the twin's request: {filled:?}, first {first}"
        );
        assert_eq!(world.provider.requests().len(), 2, "{kind:?}");
        assert_eq!(fast.requests().len(), 1, "{kind:?}: a summary was prepared");
    }
}

/// A response that calls the `read` tool, and waits for `gate` when given
/// one: what it says is the same either way.
fn read_call(gate: Option<&cineris_testkit::Gate>) -> cineris_testkit::Response {
    // Some text beside the call, so that the request after it is much
    // larger than the request before it.
    let response = cineris_testkit::Response::new()
        .text(&"Reading it. ".repeat(50))
        .tool_call("c1", "read", &serde_json::json!({"path": "a"}));
    match gate {
        Some(gate) => response.wait(gate),
        None => response,
    }
    .end(cineris_protocol::StopReason::ToolCalls)
}

#[test]
fn a_request_after_a_tool_round_is_sized_and_the_bulk_given_in_the_turn() {
    for fill in [Fill::Preparing, Fill::Compacting] {
        let gate = cineris_testkit::Gate::new();
        let mut script = vec![read_call(Some(&gate)), says("Done.")];
        if fill == Fill::Compacting {
            script.insert(1, says("Goal: the summary."));
        }
        let world = World::new("fill_after", script);
        let fast = ScriptedProvider::new([says("A summary."), says("Another.")]);
        let rig = setup(&world, Kind::Fake, true, Some(&fast));
        rig.window(6_000);
        let filled = rig.fill_after(fill, "go", [read_call(None)]);
        world.run(async {
            rig.prompt("go");
            rig.events.seen("started:assistant", 1).await;
            // The first request is on its way, with none of the bulk: the
            // notice reaches the turn at its next request.
            rig.bulk(filled.bulk);
            gate.open();
            rig.events.turns(1).await;
        });

        let sent = estimates(&rig);
        assert_eq!(sent.len(), 2, "{fill:?}: {sent:?}");
        assert!(
            (sent[0] as f64) < filled.prepare,
            "{fill:?}: the first request is small: {sent:?}, {filled:?}"
        );
        match fill {
            Fill::Preparing => {
                assert!(
                    sent[1].abs_diff(filled.estimate) <= 8,
                    "the second request measured {}, filled {filled:?}",
                    sent[1]
                );
                let after = sent[1] as f64;
                assert!(
                    after > filled.prepare && after < filled.compact,
                    "{filled:?}"
                );
                assert_eq!(fast.requests().len(), 1, "a summary was prepared");
            }
            // Past the trigger the epoch is compacted before the request,
            // so the request that is counted is the new epoch's.
            _ => assert_eq!(
                rig.store.summaries().len(),
                1,
                "compacted before the second request: {filled:?}"
            ),
        }
    }
}

#[test]
#[should_panic(expected = "this one has history")]
fn fill_refuses_a_session_that_has_history() {
    let world = World::new("fill_history", [says("One.")]);
    let rig = setup(&world, Kind::Fake, true, None);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    let _ = rig.fill(Fill::Below, "second");
}

#[test]
#[should_panic(expected = "this session has no history")]
fn fill_next_refuses_a_session_with_no_history() {
    let world = World::new("fill_next_none", [says("One.")]);
    let rig = setup(&world, Kind::Fake, true, None);
    let _ = rig.fill_next(Fill::Below, "first");
}

#[test]
#[should_panic(expected = "cannot be reached")]
fn a_pressure_the_history_leaves_no_room_for_is_refused_with_the_numbers() {
    let world = World::new("fill_next_full", [says(&"x".repeat(3_000)), says("Two.")]);
    let rig = setup(&world, Kind::Fake, true, None);
    rig.window(4_000);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    // The answer alone is a thousand tokens of a limit of 2,920: no bulk
    // can take the request past the trigger and keep it below the
    // threshold of a summary in the background.
    let _ = rig.fill_next(Fill::Below, &"y".repeat(6_000));
}

#[test]
#[should_panic(expected = "counted by its provider")]
fn a_session_whose_requests_were_counted_cannot_be_sized_by_pressure() {
    let counted = says("One.").tokens(cineris_protocol::usage::TokenUsage {
        input: 700,
        output: 5,
        ..cineris_protocol::usage::TokenUsage::default()
    });
    let world = World::new("fill_next_counted", [counted, says("Two.")]);
    let rig = setup(&world, Kind::Fake, true, None);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    let _ = rig.fill_next(Fill::Below, "second");
}

#[test]
#[should_panic(expected = "must hold every response of the turn")]
fn a_request_the_turn_never_makes_is_refused() {
    let world = World::new("fill_after_short", [says("One.")]);
    let rig = setup(&world, Kind::Fake, true, None);
    // The twin's turn ends at its first answer: there is no third request.
    let _ = rig.fill_after(Fill::Below, "go", [says("Two."), says("Three.")]);
}
