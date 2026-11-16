//! A running session's settings changed by a reload of the configuration
//! (step 24, `docs/steps/24/answered.md`, "The design, before building",
//! point 1): what applies at once is taken at the next request, and what
//! waits for the next epoch when a compaction begins.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cortex_agent::{
    Account, Compaction, ContextSettings, EpochChange, EpochContext, Ledger, Reconfigure, RoleModel,
};
use cortex_protocol::event::TurnOutcome;
use cortex_protocol::lease::Budget;
use cortex_protocol::model::ModelRole;
use cortex_testkit::{ScriptedProvider, fixtures};
use serde_json::json;

use crate::{Fill, Setup, World, calls, says};

fn reads(id: &str) -> cortex_testkit::Response {
    calls(&[(id, "read", json!({"path": "a"}))])
}

/// Automatic compaction turned off by a reload: the next request past the
/// trigger stops and asks, as a session started with it off does.
#[test]
fn a_reload_turns_automatic_compaction_off_from_the_next_request() {
    let world = World::new("reconfigure_compaction", [says("Unreached.")]);
    let mut setup = Setup::new(&world);
    setup.context = ContextSettings::default();
    let rig = setup.start(&world);
    rig.window(6_000);
    let filled = rig.fill(Fill::Compacting, "go");
    rig.handle.reconfigure(Reconfigure {
        compaction: Some(Compaction {
            auto_compact: false,
            background: true,
            compact_at: 0.85,
            prepare_at: 0.70,
        }),
        ..Reconfigure::default()
    });
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let end = &rig.events.ends()[0];
    assert_eq!(end.outcome, TurnOutcome::Failed, "{filled:?}");
    assert_eq!(end.error.as_ref().unwrap().code, "compaction_needed");
    assert!(world.provider.requests().is_empty());
}

/// Limits changed by a reload: the session's account keeps what was spent
/// under its new budget, and the turn's budget holds from the next request.
#[test]
fn a_reload_changes_the_limits_and_keeps_what_was_spent() {
    let world = World::new(
        "reconfigure_limits",
        [
            reads("c1"),
            says("One."),
            reads("c2"),
            reads("c3"),
            says("Unreached."),
        ],
    );
    let account = Account::new(
        "the session",
        Budget {
            requests: Some(100),
            ..Budget::default()
        },
        world.shared_clock(),
    );
    let mut setup = Setup::new(&world);
    setup.ledger = Some(Ledger::new(world.shared_clock()).with_account(account.clone()));
    setup.config.repetition = None;
    let rig = setup.start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    assert_eq!(account.spent().requests, 2);

    let tighter = account.with_budget(Budget {
        requests: Some(3),
        ..Budget::default()
    });
    rig.handle.reconfigure(Reconfigure {
        accounts: Some(vec![tighter.clone()]),
        ..Reconfigure::default()
    });
    world.run(async {
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    // Two spent before, one now: the session's limit of three is reached.
    let ends = rig.events.ends();
    assert_eq!(ends[1].outcome, TurnOutcome::Limit);
    assert_eq!(world.provider.requests().len(), 3);
    assert_eq!(account.spent().requests, 3, "the spending is shared");
    assert_eq!(tighter.spent().requests, 3);

    // A turn's budget changed: the next turn holds to it.
    let looser = account.with_budget(Budget::default());
    rig.handle.reconfigure(Reconfigure {
        accounts: Some(vec![looser]),
        turn_budget: Some(Budget {
            requests: Some(1),
            ..Budget::default()
        }),
        ..Reconfigure::default()
    });
    world.run(async {
        rig.prompt("third");
        rig.events.turns(3).await;
    });
    let ends = rig.events.ends();
    assert_eq!(ends[2].outcome, TurnOutcome::Limit);
    assert_eq!(ends[2].error.as_ref().unwrap().code, "request_limit");
    assert_eq!(world.provider.requests().len(), 4);
}

/// What waits for the next epoch is not taken at a turn's start, and is
/// taken when a compaction begins: the summary is the old model's, and the
/// new epoch's requests go to the new one; whoever set it is told once.
#[test]
fn what_waits_for_the_next_epoch_is_taken_by_a_compaction_and_not_before() {
    let world = World::new(
        "reconfigure_epoch",
        [says("One."), says("Two."), says("Goal: the summary.")],
    );
    let rig = Setup::new(&world).start(&world);
    let other = ScriptedProvider::new([says("From the second model.")]);
    let model = RoleModel::new(Arc::new(other.clone()), fixtures::plain_model());
    let told = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&told);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    rig.handle.at_next_epoch(Some(EpochChange {
        models: vec![(ModelRole::Main, model)],
        context: Some(EpochContext {
            keep_recent_tokens: 0,
            instruction_names: vec!["AGENTS.md".to_owned()],
            instructions_cap: 4_096,
        }),
        taken: Some(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })),
    }));
    world.run(async {
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    assert_eq!(told.load(Ordering::SeqCst), 0, "a turn's start took it");
    assert!(other.requests().is_empty());
    assert_eq!(
        rig.handle.state().roles[&ModelRole::Main].model.as_str(),
        fixtures::MODEL
    );

    world.run(async {
        rig.handle.compact(None).map_err(|error| *error).unwrap();
        rig.events.turns(3).await;
        rig.prompt("third");
        rig.events.turns(4).await;
    });
    assert_eq!(told.load(Ordering::SeqCst), 1);
    assert_eq!(
        world.provider.requests().len(),
        3,
        "the summary is the old epoch's model's"
    );
    assert_eq!(other.requests().len(), 1, "the new epoch's request");
    assert_eq!(
        rig.handle.state().roles[&ModelRole::Main].model.as_str(),
        fixtures::PLAIN_MODEL
    );
}
