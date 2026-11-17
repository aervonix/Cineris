//! `session/rewind_plan` (since step 25): what a rewind would do, read
//! before it is done, changing nothing; and a rewind tied to the plan the
//! person was shown, refused when that plan has moved
//! (`docs/steps/25/answered.md`, point 4).

use cineris_core::ClientInfo;
use cineris_protocol::error::code;
use cineris_protocol::methods::{
    Op, OpResult, RewindFileStatus, RewindTarget, SessionNewParams, SessionRewindParams,
    SessionRewindPlanParams, SessionRewindPlanResult,
};
use cineris_protocol::{Item, ItemId};
use serde_json::json;

use crate::{Front, World, calls, says};

/// The rewind's plan to after `to`.
async fn plan(front: &Front, to: &ItemId) -> SessionRewindPlanResult {
    let result = front
        .op(Op::SessionRewindPlan(SessionRewindPlanParams {
            session_id: front.session.clone(),
            to: to.clone(),
        }))
        .await
        .expect("the plan is read");
    let OpResult::SessionRewindPlan(plan) = result else {
        panic!("not a plan: {result:?}");
    };
    plan
}

/// The status and the target of each file of `plan`, by its name.
fn files(plan: &SessionRewindPlanResult) -> Vec<(String, RewindTarget, RewindFileStatus)> {
    plan.files
        .iter()
        .map(|file| (file.path.clone(), file.to.clone(), file.status.clone()))
        .collect()
}

#[test]
fn a_rewind_plan_changes_nothing_and_says_what_the_rewind_then_does() {
    let world = World::new(
        "rewind_plan",
        [
            says("Nothing to do yet."),
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            calls(&[
                (
                    "c2",
                    "write",
                    json!({"path": "a.txt", "content": "changed\n"}),
                ),
                (
                    "c3",
                    "write",
                    json!({"path": "new.txt", "content": "new\n"}),
                ),
            ]),
            says("Changed."),
        ],
    );
    world.put("a.txt", "original\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hello").await;
        let keep = front
            .items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::AssistantMessage(message) => Some(message.id.clone()),
                _ => None,
            })
            .unwrap();
        front.turn("change it").await;
        let log = world.log();

        // The plan: each file the second turn changed, with what it goes
        // back to; read, it changes nothing.
        let first = plan(&front, &keep).await;
        assert_eq!(
            files(&first),
            [
                (
                    "a.txt".to_owned(),
                    RewindTarget::Content,
                    RewindFileStatus::AsLeft
                ),
                (
                    "new.txt".to_owned(),
                    RewindTarget::Removed,
                    RewindFileStatus::AsLeft
                ),
            ]
        );
        assert!(!first.unreadable);
        assert_eq!(first.turns.len(), 1, "the second turn alone: {first:?}");
        assert_eq!(world.read("a.txt").as_deref(), Some("changed\n"));
        assert_eq!(world.read("new.txt").as_deref(), Some("new\n"));
        assert_eq!(world.log(), log, "a plan wrote to the log");
        assert_eq!(plan(&front, &keep).await, first, "read twice, two plans");

        // A file edited by hand since: the plan says so, and a rewind that
        // puts files back by the plan shown first is refused, changing
        // nothing.
        world.put("a.txt", "mine\n");
        let edited = plan(&front, &keep).await;
        assert_eq!(edited.files[0].status, RewindFileStatus::EditedSince);
        assert_ne!(edited.mark, first.mark);
        let refused = front
            .op(Op::SessionRewind(SessionRewindParams {
                session_id: front.session.clone(),
                to: keep.clone(),
                restore_files: true,
                plan: Some(first.mark.clone()),
            }))
            .await
            .expect_err("a plan that moved is refused");
        assert_eq!(refused.code, code::PLAN_CHANGED);
        assert_eq!(world.read("a.txt").as_deref(), Some("mine\n"));
        assert_eq!(world.read("new.txt").as_deref(), Some("new\n"));
        assert_eq!(world.log(), log, "a refused rewind wrote to the log");

        // As the harness left it again: the plan shown first is the plan,
        // and the rewind does what it said.
        world.put("a.txt", "changed\n");
        assert_eq!(plan(&front, &keep).await.mark, first.mark);
        let rewound = front
            .op(Op::SessionRewind(SessionRewindParams {
                session_id: front.session.clone(),
                to: keep.clone(),
                restore_files: true,
                plan: Some(first.mark.clone()),
            }))
            .await
            .expect("the rewind by its plan");
        assert!(matches!(rewound, OpResult::SessionRewind(_)));
        assert_eq!(world.read("a.txt").as_deref(), Some("original\n"));
        assert_eq!(world.read("new.txt"), None, "a file the turn made");

        // Nothing is left to undo after it.
        let after = plan(&front, &keep).await;
        assert!(after.files.is_empty(), "{after:?}");
        front.close().await;
    });
}

/// A rewind of the conversation alone depends on the turns the plan undoes,
/// not on the files: a file edited since does not refuse it.
#[test]
fn a_conversation_only_rewind_is_held_to_the_plans_turns_alone() {
    let world = World::new(
        "rewind_plan_turns",
        [
            says("Nothing to do yet."),
            calls(&[("c1", "write", json!({"path": "b.txt", "content": "b\n"}))]),
            says("Wrote it."),
            says("A third."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hello").await;
        let keep = front
            .items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::AssistantMessage(message) => Some(message.id.clone()),
                _ => None,
            })
            .unwrap();
        front.turn("write b").await;
        let shown = plan(&front, &keep).await;

        // Another turn since: the plan's turns moved, and the rewind is
        // refused even of the conversation alone.
        front.turn("and a third").await;
        let refused = front
            .op(Op::SessionRewind(SessionRewindParams {
                session_id: front.session.clone(),
                to: keep.clone(),
                restore_files: false,
                plan: Some(shown.mark.clone()),
            }))
            .await
            .expect_err("a turn ran since the plan");
        assert_eq!(refused.code, code::PLAN_CHANGED);

        // A file edited since changes the plan's files, not its turns.
        let shown = plan(&front, &keep).await;
        world.put("b.txt", "edited\n");
        front
            .op(Op::SessionRewind(SessionRewindParams {
                session_id: front.session.clone(),
                to: keep.clone(),
                restore_files: false,
                plan: Some(shown.mark.clone()),
            }))
            .await
            .expect("the conversation alone");
        assert_eq!(world.read("b.txt").as_deref(), Some("edited\n"));
        front.close().await;
    });
}

#[test]
fn a_session_with_no_log_has_no_rewind_plan() {
    let world = World::new("rewind_plan_memory", [says("Hello.")]);
    let core = world.core();
    world.run(async {
        let client = core.connect(ClientInfo::user());
        let result = core
            .handle(
                client.id,
                Op::SessionNew(SessionNewParams {
                    cwd: None,
                    workspace: world.workspace.to_str().unwrap().to_owned(),
                    settings: cineris_protocol::session::SessionSettings::default(),
                    title: None,
                    config: serde_json::Map::new(),
                    ephemeral: true,
                }),
            )
            .await
            .unwrap();
        let OpResult::SessionNew(result) = result else {
            panic!("not a session");
        };
        let refused = core
            .handle(
                client.id,
                Op::SessionRewindPlan(SessionRewindPlanParams {
                    session_id: result.session.session_id.clone(),
                    to: ItemId::from("i_1"),
                }),
            )
            .await
            .expect_err("no log, no plan");
        assert_eq!(refused.code, "session_not_stored");
    });
}
