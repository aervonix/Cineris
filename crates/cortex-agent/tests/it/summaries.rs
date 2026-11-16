//! The summary request where the estimate was wrong, or the request is
//! not what a plain model takes (`docs/architecture/07-sessions-and-context.md`,
//! "Budgeting" and "Compaction"): a summary refused as too long, at the
//! trigger, in the background and by the `fast` role; one cut off at the
//! output limit; one on a model whose reasoning is a token budget; an
//! overflow with automatic compaction off; and the settings a compaction
//! needs room from.
//!
//! The first two tests began as the review of step 15's, which failed on
//! `main` at `0f74f5c`: nothing taught the budget when the provider refused
//! the summary request itself, so every later turn sent it whole again.

use std::sync::Arc;

use cortex_agent::{ContextSettings, RoleModel, StatEvent};
use cortex_llm::{Catalog, ModelEntry, ModelEvent, ModelRequest};
use cortex_protocol::event::{CompactionState, Notice, TurnOutcome};
use cortex_protocol::model::ModelRole;
use cortex_protocol::usage::TokenUsage;
use cortex_protocol::{ModelId, ProviderId, StopReason};
use cortex_testkit::{Response, ScriptedProvider, fixtures};
use serde_json::json;

use crate::{Fill, Rig, Setup, World, calls, says, settle, text_of};

/// A response that reports this many input tokens.
fn counted(response: Response, input: u64) -> Response {
    response.tokens(TokenUsage {
        input,
        output: 100,
        ..TokenUsage::default()
    })
}

/// A request's size in bytes, as the loop's budget counts one of text
/// alone: its system prompt, its tools and its messages' text.
fn bytes_of(request: &ModelRequest) -> u64 {
    let tools: usize = request
        .tools
        .iter()
        .map(|tool| tool.name.len() + tool.description.len() + tool.schema.to_string().len())
        .sum();
    let messages: usize = request
        .messages
        .iter()
        .map(|message| text_of(message).len())
        .sum();
    (request.system.len() + tools + messages) as u64
}

/// The provider's refusal of a request as too long.
fn overflow() -> Response {
    Response::events([ModelEvent::Error(crate::transitions::overflow())])
}

fn texts(request: &ModelRequest) -> Vec<String> {
    request.messages.iter().map(text_of).collect()
}

/// Each message's size and its first words, for a failure message.
fn shortly(request: &ModelRequest) -> Vec<String> {
    texts(request)
        .iter()
        .map(|text| {
            let start: String = text.chars().take(60).collect();
            format!("{} bytes: {start}", text.len())
        })
        .collect()
}

/// Whether a summary request was cut: it says the earlier part is left out.
fn is_cut(request: &ModelRequest) -> bool {
    texts(request)
        .iter()
        .any(|text| text.contains("is left out here"))
}

/// Whether a request is a summary request.
fn is_summary(request: &ModelRequest) -> bool {
    request
        .messages
        .last()
        .is_some_and(|last| text_of(last).contains(cortex_agent::SUMMARY_INSTRUCTIONS))
}

/// Each turn's error code, or `None` for one that ended without an error.
fn codes(rig: &Rig) -> Vec<Option<String>> {
    rig.events
        .ends()
        .iter()
        .map(|end| end.error.as_ref().map(|error| error.code.clone()))
        .collect()
}

/// The estimate of the first request the session sent.
fn first_estimate(stats: &[StatEvent]) -> u64 {
    stats
        .iter()
        .find_map(|event| match event {
            StatEvent::Context(context) => Some(context.estimate),
            _ => None,
        })
        .expect("a context record")
}

/// Two turns: the first small, the second with bulk that puts the estimate
/// between the trigger (3,102) and the limit (3,650) of a 5,000-token
/// window, so that the compaction's summary request is sent whole. No
/// summary in the background: only the trigger asks for one.
fn past_the_trigger(name: &str, script: Vec<Response>) -> (World, Rig) {
    past_the_trigger_with(name, script, ContextSettings::default())
}

/// The same, with these settings.
fn past_the_trigger_with(
    name: &str,
    script: Vec<Response>,
    context: ContextSettings,
) -> (World, Rig) {
    let world = World::new(name, script);
    let mut setup = Setup::new(&world);
    setup.context = ContextSettings {
        background: false,
        ..context
    };
    let rig = setup.start(&world);
    // limit_of(5_000, 8_000) = 5_000 - 1_250 - 100 = 3_650.
    rig.window(5_000);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    let first = first_estimate(&rig.stats.all());
    assert!(first < 2_500, "the first request is small: {first}");
    rig.fill_next(Fill::Compacting, "second");
    world.run(async {
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    (world, rig)
}

/// The estimate was low (content with fewer bytes to the token than the
/// ratio measured), so the summary request at the trigger went whole and
/// the provider refused it. The refusal teaches the budget, and the
/// request is cut to half the limit and sent once more.
#[test]
fn a_summary_request_refused_as_too_long_is_cut_and_tried_again() {
    let (world, rig) = past_the_trigger(
        "summary_refused_retried",
        vec![
            says("One."),
            overflow(),
            says("Goal: the summary."),
            says("Two."),
        ],
    );
    let requests = world.provider.requests();
    let said: Vec<Vec<String>> = requests.iter().map(shortly).collect();
    let ends = rig.events.ends();
    assert_eq!(
        ends[1].outcome,
        TurnOutcome::Completed,
        "turn 2 ended with {:?} after {} requests; what they held: {said:#?}",
        ends[1].error,
        requests.len()
    );
    assert_eq!(requests.len(), 4, "{said:#?}");
    assert!(
        is_summary(&requests[1]) && !is_cut(&requests[1]),
        "whole first"
    );
    assert!(
        is_summary(&requests[2]) && is_cut(&requests[2]),
        "{said:#?}"
    );
    assert!(text_of(&requests[3].messages[0]).contains("Goal: the summary."));
    assert_eq!(rig.store.summaries().len(), 1);
}

/// A second refusal in one compaction ends the turn: the retry is bounded.
/// What the budget learned stays, so the next turn's summary request is
/// cut from the start, and the session goes on.
#[test]
fn a_second_refusal_ends_the_turn_and_the_next_turns_summary_is_cut() {
    let (world, rig) = past_the_trigger(
        "summary_refused_twice",
        vec![
            says("One."),
            overflow(),
            overflow(),
            says("Goal: the summary."),
            says("Three."),
        ],
    );
    let after_two = world.provider.requests();
    assert_eq!(
        after_two.len(),
        3,
        "turn 1, then two summary requests in turn 2 and no more: {:#?}",
        after_two.iter().map(shortly).collect::<Vec<_>>()
    );
    assert_eq!(
        codes(&rig),
        [None, Some("compaction_failed".to_owned())],
        "a second refusal ends the turn"
    );
    assert!(rig.store.summaries().is_empty(), "nothing was recorded");
    world.run(async {
        rig.prompt("third");
        rig.events.turns(3).await;
    });
    let requests = world.provider.requests();
    assert!(
        is_summary(&requests[3]) && is_cut(&requests[3]),
        "the third turn's summary request is cut from the start: {:#?}",
        shortly(&requests[3])
    );
    assert_eq!(codes(&rig)[2], None, "{:?}", rig.events.ends()[2].error);
}

/// A `fast` model whose real window is too small for the epoch refuses
/// the summary: the main model is asked, and since step 16, when each role
/// has a budget of its own, as the main model's own estimate allows. Its
/// requests at this size were just accepted: the refusal was the `fast`
/// model's (the item step 15 left open, settled in step 16).
#[test]
fn a_summary_the_fast_role_refused_is_asked_of_the_main_model() {
    let world = World::new(
        "summary_fast_refused",
        [says("Goal: from the main model."), says("Done.")],
    );
    let fast = ScriptedProvider::new([overflow()]);
    let mut setup = Setup::new(&world);
    setup.utility = Some(RoleModel::new(Arc::new(fast.clone()), fixtures::model()));
    setup.context = ContextSettings {
        background: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    rig.window(4_000);
    rig.fill(Fill::Compacting, "go");
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(codes(&rig), [None], "{:?}", rig.events.ends()[0].error);
    assert_eq!(fast.requests().len(), 1, "the fast role was asked first");
    let main = world.provider.requests();
    assert_eq!(main.len(), 2);
    assert_eq!(main[0].role, ModelRole::Main);
    assert!(is_summary(&main[0]));
    assert!(
        !is_cut(&main[0]),
        "the main model's summary is not cut for the fast model's refusal: {:#?}",
        shortly(&main[0])
    );
    assert!(text_of(&main[1].messages[0]).contains("Goal: from the main model."));
}

/// A `fast` model whose real window is smaller than its catalog says
/// refuses the summary of an epoch the estimate puts well inside that
/// window. The refusal shows no lower ratio than the one measured, so the
/// estimate does not move: only the refusal itself keeps the retry off the
/// `fast` role, which would refuse it again.
#[test]
fn a_summary_the_fast_role_refused_is_not_asked_of_it_again_in_the_epoch() {
    let world = World::new(
        "summary_fast_refused_again",
        [
            counted(calls(&[("c1", "read", json!({"path": "a"}))]), 140_000),
            counted(calls(&[("c2", "read", json!({"path": "b"}))]), 165_000),
            says("Goal: from the main model."),
            says("Done."),
        ],
    );
    let fast = ScriptedProvider::new([overflow()]);
    let mut setup = Setup::new(&world);
    setup.utility = Some(RoleModel::new(Arc::new(fast.clone()), fixtures::model()));
    setup.context = ContextSettings {
        background: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    world.run(async {
        rig.prompt("read them");
        rig.events.turns(1).await;
    });
    assert_eq!(codes(&rig), [None], "{:?}", rig.events.ends()[0].error);
    assert_eq!(fast.requests().len(), 1, "the fast role was asked once");
    let main = world.provider.requests();
    assert_eq!(main.len(), 4);
    assert!(is_summary(&main[2]) && main[2].role == ModelRole::Main);
    assert!(text_of(&main[3].messages[0]).contains("Goal: from the main model."));
}

/// After a refusal, and until the provider counts a request again, a
/// summary request is held to half the limit at the ratio the refusal
/// showed: a refusal shows only that the true count is past the window,
/// so half the limit is the margin. The cut the estimate alone calls for,
/// to nine tenths, would keep about twice as much.
#[test]
fn after_a_refusal_the_summary_request_is_held_to_half_the_limit() {
    const WINDOW: u64 = 20_000;
    let world = World::new(
        "summary_refused_half",
        [overflow(), says("Goal: the summary."), says("Done.")],
    );
    let mut setup = Setup::new(&world);
    setup.context = ContextSettings {
        background: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    rig.window(WINDOW);
    // Twelve notices of about a thousand tokens each: past the trigger
    // (12,410 of a limit of 14,600) before anything is sent, and each a
    // place the cut can be made.
    for _ in 0..12 {
        rig.bulk(3_000);
    }
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(codes(&rig), [None], "{:?}", rig.events.ends()[0].error);
    let requests = world.provider.requests();
    assert!(
        is_summary(&requests[0]) && !is_cut(&requests[0]),
        "whole first"
    );
    assert!(is_summary(&requests[1]) && is_cut(&requests[1]));
    // The ratio the refusal showed: the refused request's bytes over the
    // window. At it the retry is at most half the limit, and a little for
    // the notice that says what is left out.
    let limit = cortex_agent::context::limit_of(WINDOW, 8_000);
    #[expect(clippy::cast_precision_loss, reason = "byte counts far below 2^52")]
    let ratio = bytes_of(&requests[0]) as f64 / WINDOW as f64;
    #[expect(clippy::cast_precision_loss, reason = "byte counts far below 2^52")]
    let retry = bytes_of(&requests[1]) as f64 / ratio;
    #[expect(clippy::cast_precision_loss, reason = "a token count far below 2^52")]
    let half = limit as f64 / 2.0;
    assert!(
        retry <= half + 150.0,
        "the retry is about {retry:.0} tokens at the ratio the refusal showed, past half the \
         limit ({half:.0}): {:#?}",
        shortly(&requests[1])
    );
}

/// A summary prepared in the background that the `fast` model refused as
/// too long teaches the `fast` role's budget, and only it (since step 16):
/// the main model's estimate of the next turn does not move, so the turn is
/// sent whole, and the `fast` model is not asked again in the epoch.
#[test]
fn a_background_summary_refused_as_too_long_teaches_the_fast_roles_budget_alone() {
    let world = World::new("summary_background_refused", [says("One."), says("Two.")]);
    let fast = ScriptedProvider::new([overflow()]);
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
    assert_eq!(fast.requests().len(), 1, "no second try in the background");
    let main = world.provider.requests();
    let said: Vec<Vec<String>> = main.iter().map(shortly).collect();
    assert_eq!(main.len(), 2, "{said:#?}");
    assert!(
        !is_summary(&main[1]),
        "the second turn is sent whole, not compacted: {said:#?}"
    );
    assert_eq!(codes(&rig), [None, None]);
}

/// A response that ends because the request did not fit is a refusal as
/// too long, as an error of that kind is.
#[test]
fn a_summary_whose_response_ends_as_too_long_is_a_refusal() {
    let world = World::new(
        "summary_stop_overflow",
        [
            Response::new().end(StopReason::ContextOverflow),
            says("Goal: the second try."),
            says("Done."),
        ],
    );
    let mut setup = Setup::new(&world);
    setup.context = ContextSettings {
        background: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    rig.window(4_000);
    rig.fill(Fill::Compacting, "go");
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(codes(&rig), [None], "{:?}", rig.events.ends()[0].error);
    let requests = world.provider.requests();
    assert!(!is_cut(&requests[0]) && is_cut(&requests[1]));
    assert!(text_of(&requests[2].messages[0]).contains("Goal: the second try."));
}

/// A summary whose response stopped at the output limit is used, so that
/// the session is not stuck at the trigger, and marked as cut: in its text,
/// where the model reads it, and in the notice that says the compaction
/// finished.
#[test]
fn a_summary_cut_off_at_the_output_limit_is_used_and_marked() {
    let world = World::new(
        "summary_cut_off",
        [
            Response::new()
                .text("Goal: the first half of a summa")
                .end(StopReason::MaxOutput),
            says("Done."),
        ],
    );
    let mut setup = Setup::new(&world);
    setup.context = ContextSettings {
        background: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    rig.window(4_000);
    rig.fill(Fill::Compacting, "go");
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(codes(&rig), [None]);
    let summary = rig.store.summaries().pop().expect("a summary");
    assert!(
        summary.item.text.contains(
            "Goal: the first half of a summa\n\n[This summary was cut off here at the model's \
             output limit"
        ),
        "{}",
        cortex_testkit::brief(&summary.item.text)
    );
    let finished: Vec<String> = rig
        .events
        .notices()
        .into_iter()
        .filter_map(|notice| match notice {
            Notice::Compaction(compaction) if compaction.state == CompactionState::Finished => {
                Some(compaction.message)
            }
            _ => None,
        })
        .collect();
    assert_eq!(finished.len(), 1);
    assert!(
        finished[0].contains("cut off at the model's output limit"),
        "{finished:?}"
    );
    // A whole summary is not marked.
    let world = World::new("summary_whole", [says("Goal: all of it."), says("Done.")]);
    let mut setup = Setup::new(&world);
    setup.context = ContextSettings {
        background: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    rig.window(4_000);
    rig.fill(Fill::Compacting, "go");
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let summary = rig.store.summaries().pop().expect("a summary");
    assert!(!summary.item.text.contains("cut off"));
}

/// The owner's decision after the review of step 15: with automatic
/// compaction off, a provider's overflow stops the turn with
/// `compaction_needed`, as the trigger does, and nothing is compacted. The
/// budget learns from it, so the next turn stops at the trigger with
/// nothing sent.
#[test]
fn with_automatic_compaction_off_an_overflow_stops_the_turn_and_asks() {
    let world = World::new("auto_off_overflow", [overflow(), says("Unreached.")]);
    let mut setup = Setup::new(&world);
    setup.context = ContextSettings {
        auto_compact: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    rig.window(5_000);
    rig.fill(Fill::Below, "go");
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let end = &rig.events.ends()[0];
    assert_eq!(end.outcome, TurnOutcome::Failed);
    let error = end.error.as_ref().unwrap();
    assert_eq!(error.code, "compaction_needed");
    assert!(error.message.contains("does not fit"), "{}", error.message);
    assert_eq!(
        world.provider.requests().len(),
        1,
        "no summary was asked for"
    );
    assert!(rig.store.summaries().is_empty());
    assert!(
        rig.events
            .notices()
            .iter()
            .all(|notice| !matches!(notice, Notice::Compaction(_)))
    );
    world.run(async {
        rig.prompt("again");
        rig.events.turns(2).await;
    });
    let error = rig.events.ends()[1].error.clone().unwrap();
    assert_eq!(error.code, "compaction_needed");
    assert!(error.message.contains("automatic compaction is off"));
    assert_eq!(world.provider.requests().len(), 1, "nothing more was sent");
}

/// A model of a user's catalog whose reasoning is a token budget.
fn budget_model() -> Arc<ModelEntry> {
    let catalog = Catalog::from_json(
        r#"{"version": "budget", "model": [{
            "provider": "anthropic", "id": "claude-budget", "name": "Budget",
            "protocol": "anthropic-messages",
            "context_window": 200000, "max_output": 64000,
            "input": ["text"],
            "prices": {"input": 3, "output": 15},
            "reasoning": {"min_tokens": 1024, "max_tokens": 64000}
        }]}"#,
    )
    .unwrap();
    Arc::clone(
        catalog
            .get(
                &ProviderId::new("anthropic"),
                &ModelId::new("claude-budget"),
            )
            .unwrap(),
    )
}

/// A compaction asked for on a model with this reasoning budget, its
/// requests built by Anthropic's own request builder.
fn compacted_on_budget(name: &str, budget: u64) -> (World, Rig) {
    let world = World::new(name, [says("One."), says("Goal: the summary.")]);
    let built = world.provider.built_as(
        Arc::new(cortex_llm::anthropic::AnthropicMessages),
        cortex_llm::anthropic::provider(),
    );
    let mut setup = Setup::new(&world);
    let mut model = RoleModel::new(Arc::new(built), budget_model());
    model.reasoning.budget_tokens = Some(budget);
    model.max_output = Some(64_000);
    setup.model = model;
    let rig = setup.start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
        rig.handle.compact(None).map_err(|error| *error).unwrap();
        rig.events.turns(2).await;
    });
    (world, rig)
}

/// On an Anthropic model whose reasoning is a token budget, the summary's
/// output is its own on top of the budget, which is sent as the epoch's
/// requests send it, so that the epoch's cache is read. The review of step
/// 15 found every summary of a budget of 16,384 or more refused by the
/// request builder.
#[test]
fn a_summary_on_a_budget_reasoning_model_can_be_built() {
    let (world, rig) = compacted_on_budget("summary_budget", 20_000);
    let ends = rig.events.ends();
    assert_eq!(
        ends[0].outcome,
        TurnOutcome::Completed,
        "{:?}",
        ends[0].error
    );
    assert_eq!(
        ends[1].outcome,
        TurnOutcome::Completed,
        "the compaction failed: {:?}",
        ends[1].error
    );
    let requests = world.provider.requests();
    assert_eq!(requests.len(), 2);
    let summary = &requests[1];
    assert_eq!(summary.max_output, Some(20_000 + 16_384));
    assert_eq!(
        summary.reasoning, requests[0].reasoning,
        "the thinking parameters are the epoch's"
    );
}

/// Where the model's maximum cannot hold the budget and a summary's
/// output, the summary has what the maximum leaves.
#[test]
fn a_summary_on_a_budget_too_large_for_its_room_gets_what_the_maximum_leaves() {
    let (world, rig) = compacted_on_budget("summary_budget_large", 60_000);
    assert_eq!(rig.events.ends()[1].outcome, TurnOutcome::Completed);
    let requests = world.provider.requests();
    assert_eq!(requests[1].max_output, Some(64_000));
    assert_eq!(requests[1].reasoning.budget_tokens, Some(60_000));
}

/// The info notices the user was given.
fn infos(rig: &Rig) -> Vec<String> {
    rig.events
        .notices()
        .into_iter()
        .filter_map(|notice| match notice {
            Notice::Info(info) => Some(info.message),
            _ => None,
        })
        .collect()
}

/// A trigger set below what a compaction needs (the epoch's prompt and
/// tools, and room for a summary) would make every compaction too large
/// for the new epoch, and stop the session at its first request. It is
/// raised to what a compaction needs, and the user is told, once.
#[test]
fn a_trigger_below_what_a_compaction_needs_is_raised_and_said() {
    let world = World::new("trigger_raised", [says("Hello."), says("Again.")]);
    let mut setup = Setup::new(&world);
    setup.context = ContextSettings {
        compact_at: 0.0,
        background: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    assert_eq!(codes(&rig), [None, None], "{:?}", rig.events.ends());
    let requests = world.provider.requests();
    assert_eq!(requests.len(), 2, "no summary was asked for");
    assert!(!requests.iter().any(is_summary));
    let told = infos(&rig);
    assert_eq!(told.len(), 1, "once: {told:#?}");
    assert!(
        told[0].starts_with("`context.compact_at` (0) puts the trigger at 0 of the"),
        "{}",
        told[0]
    );
}

/// Kept turns past what a compaction can replay would fill the new epoch,
/// and fail every compaction as too large for it. What is kept is held to
/// half of what the trigger leaves beyond the prompt and the tools, and
/// the user is told.
#[test]
fn kept_turns_past_what_a_compaction_can_replay_are_capped_and_said() {
    let (world, rig) = past_the_trigger_with(
        "keep_capped",
        vec![says("One."), says("Goal: the summary."), says("Two.")],
        ContextSettings {
            keep_recent_tokens: 10_000_000,
            ..ContextSettings::default()
        },
    );
    assert_eq!(
        codes(&rig),
        [None, None],
        "{:?}",
        rig.events.ends()[1].error
    );
    let requests = world.provider.requests();
    assert!(is_summary(&requests[1]));
    // The bulk was summarised, not replayed.
    assert!(
        texts(&requests[1]).iter().any(|text| text.len() > 3_000),
        "{:#?}",
        shortly(&requests[1])
    );
    assert!(
        texts(&requests[2]).iter().all(|text| text.len() < 3_000),
        "{:#?}",
        shortly(&requests[2])
    );
    let told = infos(&rig);
    assert!(
        told.iter()
            .any(|message| message.starts_with("`context.keep_recent_tokens` (10000000)")),
        "{told:#?}"
    );
}

/// With the trigger raised to what a compaction needs, the summary is
/// asked for no more than half of what the trigger leaves beyond the
/// prompt and the tools, so that what it writes fits the new epoch. With
/// its usual 16,384 tokens (8,000 on the fixture model) a summary a model
/// wrote in full would be too large for the new epoch at every trigger.
/// Found by the review of the fixes after step 15's review.
#[test]
fn a_summary_writes_no_more_than_the_new_epoch_has_room_for() {
    let world = World::new("summary_share", [says("Goal: short."), says("Done.")]);
    let mut setup = Setup::new(&world);
    setup.context = ContextSettings {
        compact_at: 0.0,
        background: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    rig.fill(Fill::Compacting, "go");
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(codes(&rig), [None], "{:?}", rig.events.ends()[0].error);
    let requests = world.provider.requests();
    assert!(is_summary(&requests[0]));
    // The trigger is the prompt and the tools and 8,192 tokens of room on
    // this window: the summary has half of that room.
    assert_eq!(requests[0].max_output, Some(4_096));
}

/// A summary request with a reasoning budget keeps its whole output in
/// reserve: Anthropic's models before Claude 4.5 refuse a request with
/// thinking whose input and `max_tokens` together pass the window. With a
/// budget of 48,000 the output is 64,000, so an epoch of about 137,000
/// tokens, which the capped reserve would send whole, is cut.
#[test]
fn a_summary_with_a_reasoning_budget_keeps_its_whole_output_in_reserve() {
    let world = World::new(
        "summary_budget_reserve",
        [says("Goal: the summary."), says("Done.")],
    );
    let built = world.provider.built_as(
        Arc::new(cortex_llm::anthropic::AnthropicMessages),
        cortex_llm::anthropic::provider(),
    );
    let mut setup = Setup::new(&world);
    let mut model = RoleModel::new(Arc::new(built), budget_model());
    model.reasoning.budget_tokens = Some(48_000);
    model.max_output = Some(64_000);
    setup.model = model;
    setup.context = ContextSettings {
        background: false,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    // Past the trigger, so the loop compacts before it sends anything: the
    // notice is sized from what the session sends, and it fills a model of
    // 200,000 tokens, which is what the epoch of about 137,000 tokens above
    // needs it to.
    rig.fill(Fill::Compacting, "go");
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(codes(&rig), [None], "{:?}", rig.events.ends()[0].error);
    let summary = &world.provider.requests()[0];
    assert!(is_summary(summary));
    assert_eq!(summary.max_output, Some(64_000));
    assert!(is_cut(summary), "{:#?}", shortly(summary));
    #[expect(clippy::cast_precision_loss, reason = "byte counts far below 2^52")]
    let tokens = bytes_of(summary) as f64 / cortex_agent::context::DEFAULT_RATIO;
    assert!(
        tokens + 64_000.0 <= 200_000.0,
        "its input ({tokens:.0} tokens) and its output pass the window"
    );
}
