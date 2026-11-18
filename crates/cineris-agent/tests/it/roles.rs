//! Roles and variants (step 16): which model a summary goes to, how its
//! request is recorded and in which turn, what is counted of one that
//! never finished, and the variants a request is made with and recorded
//! under.

use std::sync::Arc;

use cineris_agent::{RequestPurpose, RequestStat, RoleModel, StatEvent};
use cineris_llm::ToolSchemaStyle;
use cineris_protocol::model::ModelRole;
use cineris_protocol::usage::TokenUsage;
use cineris_testkit::{Response, ScriptedProvider, fixtures};

use crate::{Fill, Setup, World, says, settle};

fn summaries(stats: &[StatEvent]) -> Vec<RequestStat> {
    stats
        .iter()
        .filter_map(|event| match event {
            StatEvent::Request(request) if request.purpose == RequestPurpose::Summary => {
                Some(request.clone())
            }
            _ => None,
        })
        .collect()
}

fn tokens(input: u64, output: u64) -> TokenUsage {
    TokenUsage {
        input,
        output,
        ..TokenUsage::default()
    }
}

/// A summary taken in the background, started by one turn and counted at
/// the next one's first boundary, is recorded in the turn that counted it,
/// with the `fast` role and the turn that started it.
#[test]
fn a_background_summary_is_recorded_in_the_turn_that_counted_it() {
    let world = World::new("summary_owner", [says("One."), says("Two.")]);
    let fast = ScriptedProvider::new([says("Goal: one.").tokens(tokens(900, 40))]);
    let mut setup = Setup::new(&world);
    setup.utility = Some(RoleModel::new(Arc::new(fast.clone()), fixtures::model()));
    let rig = setup.start(&world);
    rig.window(5_000);
    rig.fill(Fill::Preparing, "first");
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
        while fast.requests().is_empty() {
            settle().await;
        }
        settle().await;
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    let stats = rig.stats.all();
    let recorded = summaries(&stats);
    assert_eq!(recorded.len(), 1, "{recorded:#?}");
    let summary = &recorded[0];
    assert_eq!(summary.role, ModelRole::Fast);
    let ends = rig.events.ends();
    assert_eq!(ends.len(), 2);
    let turn_ids: Vec<_> = rig
        .store
        .records()
        .into_iter()
        .map(|(turn, _)| turn)
        .collect();
    let (first, second) = (turn_ids[0].clone(), turn_ids.last().unwrap().clone());
    assert_ne!(first, second);
    assert_eq!(summary.turn, second, "counted by the second turn");
    assert_eq!(summary.started_in, Some(first), "started by the first");
    assert_eq!(summary.usage.as_ref().unwrap().tokens.input, 900);
    // The second turn's own usage has it: the two agree.
    assert!(ends[1].usage.tokens.input >= 900, "{:?}", ends[1].usage);
    assert_eq!(ends[1].usage.requests, 2);
}

/// With no `fast` model, a summary goes to the main model, and is recorded
/// under `main`.
#[test]
fn with_no_fast_role_a_summary_goes_to_the_main_model() {
    let world = World::new("summary_no_fast", [says("Goal: from main."), says("Done.")]);
    let rig = Setup::new(&world).start(&world);
    rig.window(4_000);
    rig.fill(Fill::Compacting, "go");
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let recorded = summaries(&rig.stats.all());
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].role, ModelRole::Main);
    assert_eq!(world.provider.requests()[0].role, ModelRole::Main);
}

/// A summary asked of the `fast` role's model at the trigger, which fails,
/// is asked once of the main model, in the same compaction.
#[test]
fn a_summary_the_fast_role_fails_at_the_trigger_is_asked_of_the_main_model() {
    let world = World::new(
        "summary_fast_fails_inline",
        [says("Goal: from the main model."), says("Done.")],
    );
    let fast =
        ScriptedProvider::new([
            Response::new().error(cineris_llm::error::ProviderError::new(
                cineris_base::ErrorKind::Provider,
                "invalid_request",
                "No.",
            )),
        ]);
    let mut setup = Setup::new(&world);
    setup.utility = Some(RoleModel::new(Arc::new(fast.clone()), fixtures::model()));
    setup.context = cineris_agent::ContextSettings {
        background: false,
        ..cineris_agent::ContextSettings::default()
    };
    let rig = setup.start(&world);
    rig.window(4_000);
    rig.fill(Fill::Compacting, "go");
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(
        rig.events.ends()[0].outcome,
        cineris_protocol::event::TurnOutcome::Completed,
        "{:?}",
        rig.events.ends()[0].error
    );
    assert_eq!(fast.requests().len(), 1);
    let main = world.provider.requests();
    assert_eq!(main[0].role, ModelRole::Main);
    assert_eq!(summaries(&rig.stats.all()).len(), 2);
}

/// A summary in the background that is stopped part-way (the turn was
/// interrupted) was paid for as far as it went: what the provider had said
/// it used is counted at the next boundary, and recorded.
#[test]
fn a_summary_stopped_part_way_is_counted() {
    // The turn's own request waits, so that the interrupt finds it running.
    let world = World::new("summary_stopped", [Response::new().stall(), says("Two.")]);
    let fast = ScriptedProvider::new([Response::new()
        .tokens(tokens(700, 3))
        .usage()
        .text("Goal:")
        .stall()]);
    let mut setup = Setup::new(&world);
    setup.utility = Some(RoleModel::new(Arc::new(fast.clone()), fixtures::model()));
    let rig = setup.start(&world);
    rig.window(5_000);
    rig.fill(Fill::Preparing, "first");
    world.run(async {
        rig.prompt("first");
        while fast.requests().is_empty() {
            settle().await;
        }
        settle().await;
        let _ = rig.handle.interrupt();
        rig.events.turns(1).await;
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    let recorded = summaries(&rig.stats.all());
    assert_eq!(recorded.len(), 1, "{recorded:#?}");
    assert_eq!(recorded[0].usage.as_ref().unwrap().tokens.input, 700);
    assert_eq!(
        recorded[0].error.as_ref().map(|error| error.code.as_str()),
        Some("interrupted")
    );
}

/// A summary still running when the session closes is counted when the
/// loop ends, in the last turn that ran.
#[test]
fn a_summary_left_running_when_the_session_closes_is_counted() {
    let world = World::new("summary_left", [says("One.")]);
    let fast = ScriptedProvider::new([Response::new().tokens(tokens(500, 2)).usage().stall()]);
    let mut setup = Setup::new(&world);
    setup.utility = Some(RoleModel::new(Arc::new(fast.clone()), fixtures::model()));
    let rig = setup.start(&world);
    rig.window(5_000);
    rig.fill(Fill::Preparing, "first");
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
        while fast.requests().is_empty() {
            settle().await;
        }
        settle().await;
        rig.handle.close();
        // The loop ends, and counts what was left, on its own task.
        for _ in 0..100 {
            if !summaries(&rig.stats.all()).is_empty() {
                break;
            }
            settle().await;
        }
    });
    let recorded = summaries(&rig.stats.all());
    assert_eq!(recorded.len(), 1, "{recorded:#?}");
    assert_eq!(recorded[0].usage.as_ref().unwrap().tokens.input, 500);
}

/// A request the provider refused as too long is recorded with its context
/// record before the compaction: the estimate that was wrong is what
/// explains the refusal.
#[test]
fn a_request_refused_as_too_long_keeps_its_context_record() {
    let world = World::new(
        "overflow_record",
        [
            Response::events([cineris_llm::ModelEvent::Error(
                crate::transitions::overflow(),
            )]),
            says("A summary."),
            says("Hello."),
        ],
    );
    let rig = Setup::new(&world).start(&world);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let stats = rig.stats.all();
    let contexts = stats
        .iter()
        .filter(|event| matches!(event, StatEvent::Context(_)))
        .count();
    let refused = stats.iter().any(|event| {
        matches!(event, StatEvent::Request(request)
            if request.stop == cineris_protocol::StopReason::ContextOverflow)
    });
    assert!(refused, "{stats:#?}");
    // The refused request's, and the one sent again after the compaction.
    assert_eq!(contexts, 2, "{stats:#?}");
}

/// The variants are fixed for an epoch: a session opened again with other
/// variants begins an epoch of its own with them, and goes on making its
/// requests with the epoch's.
#[test]
fn variants_changed_between_two_openings_begin_a_new_epoch() {
    let world = World::new("variants_epoch", [says("One."), says("Two.")]);
    let first = Setup::new(&world).start(&world);
    world.run(async {
        first.prompt("one");
        first.events.turns(1).await;
    });
    first.handle.close();
    let mut setup = Setup::new(&world);
    setup.memory = first.store.clone();
    setup.model.variants.tool_schema = ToolSchemaStyle::Loose;
    let second = setup.start(&world);
    world.run(async {
        second.prompt("two");
        second.events.turns(1).await;
    });
    let epochs = second.store.epochs();
    assert_eq!(epochs.len(), 2, "{epochs:#?}");
    assert_eq!(
        epochs[1].variants.as_ref().unwrap().tool_schema,
        ToolSchemaStyle::Loose
    );
    let requests = world.provider.requests();
    assert_eq!(requests[0].variants.tool_schema, ToolSchemaStyle::Strict);
    assert_eq!(requests[1].variants.tool_schema, ToolSchemaStyle::Loose);
}

/// A variant override changes the request, is recorded in the epoch and
/// with every outcome, and the `guided` style gives the prompt its notes.
#[test]
fn a_variant_override_changes_the_request_and_is_recorded() {
    let world = World::new("variants", [says("Done.")]);
    let mut setup = Setup::new(&world);
    setup.model.variants.tool_schema = ToolSchemaStyle::Loose;
    setup.model.variants.prompt_style = "guided".to_owned();
    let rig = setup.start(&world);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let request = &world.provider.requests()[0];
    assert_eq!(request.variants.tool_schema, ToolSchemaStyle::Loose);
    assert!(request.system.contains(cineris_agent::GUIDED_NOTES));
    let epochs = rig.store.epochs();
    let recorded = epochs[0].variants.as_ref().unwrap();
    assert_eq!(recorded.tool_schema, ToolSchemaStyle::Loose);
    assert_eq!(recorded.prompt_style, "guided");
    let stats = rig.stats.all();
    for event in &stats {
        match event {
            StatEvent::Request(request) => {
                assert_eq!(request.variants.tool_schema, ToolSchemaStyle::Loose);
            }
            StatEvent::Turn(turn) => assert_eq!(turn.variants.prompt_style, "guided"),
            _ => {}
        }
    }
}
