//! `session/rewind_plan` (since step 25): what a rewind would do, read
//! before it is done, changing nothing; a rewind tied to the plan the
//! person was shown, refused when that plan has moved
//! (`docs/steps/25/answered.md`, point 4); and the point "before a prompt"
//! that a rewind, its plan and a fork name with `before` (since the review
//! of step 25's second pull request).

use cineris_core::{ClientInfo, OpError};
use cineris_protocol::error::code;
use cineris_protocol::event::Event;
use cineris_protocol::item::{Delivery, PlanStatus};
use cineris_protocol::methods::{
    Op, OpResult, PlanUpdateParams, RewindFileStatus, RewindTarget, RewindWhyNot,
    SessionForkParams, SessionNewParams, SessionRef, SessionRewindParams, SessionRewindPlanParams,
    SessionRewindPlanResult,
};
use cineris_protocol::project::{PlanAdd, PlanEdit};
use cineris_protocol::{Item, ItemId, TurnId};
use serde_json::json;

use crate::{Front, World, calls, says};

/// The rewind's plan to after `to`.
async fn plan(front: &Front, to: &ItemId) -> SessionRewindPlanResult {
    let result = front
        .op(Op::SessionRewindPlan(SessionRewindPlanParams {
            session_id: front.session.clone(),
            to: Some(to.clone()),
            before: None,
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
                to: Some(keep.clone()),
                before: None,
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
                to: Some(keep.clone()),
                before: None,
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
                to: Some(keep.clone()),
                before: None,
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
                to: Some(keep.clone()),
                before: None,
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
                    to: Some(ItemId::from("i_1")),
                    before: None,
                }),
            )
            .await
            .expect_err("no log, no plan");
        assert_eq!(refused.code, "session_not_stored");
    });
}

// ------------------------------------------------- the point before a prompt

/// A point as a rewind, its plan and a fork name it: `to` (or `at`), and
/// `before`.
type Named = (Option<ItemId>, Option<ItemId>);

/// "Before this prompt", as `/rewind` names it: `before` the prompt's item.
fn before(prompt: &ItemId) -> Named {
    (None, Some(prompt.clone()))
}

/// The plan of a rewind to `point`, or why there is none.
async fn plan_of(front: &Front, (to, before): Named) -> Result<SessionRewindPlanResult, OpError> {
    match front
        .op(Op::SessionRewindPlan(SessionRewindPlanParams {
            session_id: front.session.clone(),
            to,
            before,
        }))
        .await?
    {
        OpResult::SessionRewindPlan(plan) => Ok(plan),
        other => panic!("not a plan: {other:?}"),
    }
}

/// A rewind to `point`.
async fn rewind_to(
    front: &Front,
    (to, before): Named,
    restore_files: bool,
    plan: Option<String>,
) -> Result<OpResult, OpError> {
    front
        .op(Op::SessionRewind(SessionRewindParams {
            session_id: front.session.clone(),
            to,
            before,
            restore_files,
            plan,
        }))
        .await
}

/// A fork of the session at `point`, as a front end of its own on the
/// same client.
async fn fork_at(front: &Front, (at, before): Named) -> Result<Front, OpError> {
    let OpResult::SessionFork(forked) = front
        .op(Op::SessionFork(SessionForkParams {
            session_id: front.session.clone(),
            at,
            before,
        }))
        .await?
    else {
        panic!("not a fork");
    };
    Ok(Front {
        session: forked.session.session_id.clone(),
        state: forked.session,
        seen: std::cell::RefCell::default(),
        answered: std::cell::RefCell::default(),
        answering: std::cell::RefCell::default(),
        core: front.core.clone(),
        client: front.client.clone(),
    })
}

/// The prompts that began a turn, in order.
fn prompts(front: &Front) -> Vec<ItemId> {
    front
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::UserMessage(message) if message.delivery != Some(Delivery::Steer) => {
                Some(message.id)
            }
            _ => None,
        })
        .collect()
}

/// The turn an item was made in.
fn turn_of(front: &Front, item: &ItemId) -> TurnId {
    front
        .events()
        .into_iter()
        .find_map(|event| match &event.event {
            Event::ItemCompleted(completed) if completed.item.id() == Some(item.as_str()) => {
                event.turn_id.clone()
            }
            _ => None,
        })
        .expect("the item was made in a turn")
}

/// The person adds an item to the session's plan.
async fn edit_plan(front: &Front, text: &str) {
    front
        .op(Op::PlanUpdate(PlanUpdateParams {
            session_id: front.session.clone(),
            edits: vec![PlanEdit::Add(PlanAdd {
                text: text.to_owned(),
                after: None,
            })],
        }))
        .await
        .expect("the plan is edited");
}

/// The texts of the session's plan.
async fn plan_texts(front: &Front) -> Vec<String> {
    let OpResult::PlanGet(result) = front
        .op(Op::PlanGet(SessionRef {
            session_id: front.session.clone(),
        }))
        .await
        .expect("the plan is read")
    else {
        panic!("not a plan");
    };
    result
        .plan
        .items
        .into_iter()
        .filter(|item| item.status != PlanStatus::Skipped)
        .map(|item| item.text)
        .collect()
}

/// The ids of the items of the active branch.
async fn branch(front: &Front) -> Vec<ItemId> {
    let OpResult::SessionRead(read) = front
        .op(Op::SessionRead(
            cineris_protocol::methods::SessionReadParams {
                session_id: front.session.clone(),
                item_id: None,
                limit: None,
                cursor: None,
                redacted: false,
            },
        ))
        .await
        .expect("the branch is read")
    else {
        panic!("not a read");
    };
    read.items
        .iter()
        .filter_map(|item| item.id().map(ItemId::from))
        .collect()
}

/// The turns the session's log records as undone, in order.
fn undone(front: &Front, world: &World) -> Vec<String> {
    front
        .log(world)
        .iter()
        .filter(|entry| entry["kind"] == "outcome" && entry.get("reverted").is_some())
        .map(|entry| entry["turn"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// "Before this prompt" is the state before the prompt's entry: a plan the
/// person edited between the turn before and the prompt is kept by a fork
/// and by a rewind from there, and the plan undoes the prompt's turn alone
/// (the review of step 25's second pull request: a rewind to the item
/// before the prompt took the edit with it, unseen).
#[test]
fn a_fork_and_a_rewind_before_a_prompt_keep_the_plan_edited_before_it() {
    let world = World::new("rewind_before_plan", [says("One."), says("Two.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        edit_plan(&front, "Keep the tests green").await;
        front.turn("two").await;
        let second = prompts(&front)[1].clone();

        let fork = fork_at(&front, before(&second)).await.expect("the fork");
        assert_eq!(
            plan_texts(&fork).await,
            ["Keep the tests green"],
            "the fork holds the edit"
        );
        fork.close().await;

        let shown = plan_of(&front, before(&second)).await.expect("the plan");
        assert_eq!(shown.turns, [turn_of(&front, &second)]);
        rewind_to(&front, before(&second), false, Some(shown.mark))
            .await
            .expect("the rewind by its plan");
        assert_eq!(
            plan_texts(&front).await,
            ["Keep the tests green"],
            "the rewind keeps the edit"
        );
        assert_eq!(undone(&front, &world), [turn_of(&front, &second).as_str()]);
        let kept = branch(&front).await;
        assert!(!kept.contains(&second), "the prompt is kept: {kept:?}");
        assert_eq!(
            kept.len(),
            3,
            "the first turn and the edit's notice: {kept:?}"
        );
        // The prompt is on another branch now: a rewind is not planned to
        // before it, as one to after it is not (a fork may still be).
        let refused = plan_of(&front, before(&second))
            .await
            .expect_err("a point off the active branch");
        assert_eq!(refused.code, "invalid_entry");
        front.close().await;
    });
}

#[cfg(unix)]
fn git(dir: &std::path::Path, arguments: &[&str]) -> bool {
    std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(arguments)
        .current_dir(dir)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// The same for files: what the project's checks changed at the end of the
/// turn before is that turn's, in a checkpoint written after its last
/// item. A rewind to before the next prompt keeps it, and does not count
/// that turn as undone; a fork from there holds the checkpoint (the review
/// of step 25's second pull request: a rewind to the item before the
/// prompt put those files back and counted the turn before as undone, and
/// a fork from it left the checkpoint out). A check starts a shell, and git
/// says what it changed: on Linux only, as the rest of what runs is the
/// same code on both systems.
#[cfg(unix)]
#[test]
fn sys_a_rewind_before_a_prompt_keeps_what_the_checks_of_the_turn_before_changed() {
    use crate::verify::{program, user_checks, writes};
    let world = World::new(
        "rewind_before_checks",
        [writes("c1", "a.txt"), says("Done."), says("Two.")],
    );
    world.put("notes.txt", "committed\n");
    world.put("dirty.txt", "committed\n");
    let check = program(
        &world,
        "check",
        "echo made > made.txt\necho more >> notes.txt\necho more >> dirty.txt",
        "",
    );
    user_checks(&world, &[&check], "");
    if !(git(&world.workspace, &["init", "-q"])
        && git(&world.workspace, &["add", "."])
        && git(&world.workspace, &["commit", "-q", "-m", "first"]))
    {
        return;
    }
    // Changed before the session: what it held before the check is not
    // what git has.
    world.put("dirty.txt", "edited\n");
    let core = world.core_with(|options| options.trust_project = true);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("change it").await;
        assert_eq!(world.read("made.txt").as_deref(), Some("made\n"));
        assert_eq!(
            world.read("notes.txt").as_deref(),
            Some("committed\nmore\n")
        );
        front.turn("two").await;
        let [first, second] = &prompts(&front)[..] else {
            panic!("two prompts: {:?}", front.items());
        };
        let checked = front.log(&world).into_iter().any(|entry| {
            entry["kind"] == "checkpoint"
                && entry["turn"] == turn_of(&front, first).as_str()
                && entry.to_string().contains("made.txt")
        });
        assert!(
            checked,
            "the check's change is in the first turn's checkpoint"
        );

        // A fork from before the second prompt holds that checkpoint.
        let fork = fork_at(&front, before(second)).await.expect("the fork");
        let forked = fork
            .log(&world)
            .into_iter()
            .any(|entry| entry["kind"] == "checkpoint" && entry.to_string().contains("made.txt"));
        assert!(forked, "the fork holds the first turn's last checkpoint");
        fork.close().await;

        // The plan undoes the second turn alone, which changed no file.
        let shown = plan_of(&front, before(second)).await.expect("the plan");
        assert_eq!(shown.turns, [turn_of(&front, second)]);
        assert!(shown.files.is_empty(), "{shown:?}");
        rewind_to(&front, before(second), true, Some(shown.mark))
            .await
            .expect("the rewind by its plan");
        assert_eq!(world.read("made.txt").as_deref(), Some("made\n"));
        assert_eq!(
            world.read("notes.txt").as_deref(),
            Some("committed\nmore\n")
        );
        assert_eq!(undone(&front, &world), [turn_of(&front, second).as_str()]);

        // Before the first prompt, the first turn is undone with what its
        // check did: a file it made goes, one git has goes back, and one
        // whose earlier content nobody recorded cannot, and says why.
        let all = plan_of(&front, before(first)).await.expect("the plan");
        assert_eq!(all.turns, [turn_of(&front, first)]);
        let file = |path: &str| {
            let file = all
                .files
                .iter()
                .find(|file| file.path == path)
                .unwrap_or_else(|| panic!("`{path}` is in the plan: {all:?}"));
            (file.to.clone(), file.status.clone(), file.why_not.clone())
        };
        assert_eq!(
            file("made.txt"),
            (RewindTarget::Removed, RewindFileStatus::AsLeft, None)
        );
        assert_eq!(
            file("notes.txt"),
            (RewindTarget::Content, RewindFileStatus::AsLeft, None)
        );
        assert_eq!(
            file("dirty.txt"),
            (
                RewindTarget::NotRecorded,
                RewindFileStatus::Cannot,
                Some(RewindWhyNot::NotRecorded)
            )
        );
        front.close().await;
    });
}

/// A rewind and its plan name their point by exactly one of `to` and
/// `before`, and a fork by at most one of `at` and `before`: two, or none
/// for a rewind, are refused, and nothing is changed or made.
#[test]
fn a_rewind_its_plan_and_a_fork_name_their_point_by_one_form() {
    let world = World::new("rewind_one_form", [says("One."), says("Two.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        front.turn("two").await;
        let second = prompts(&front)[1].clone();
        let log = world.log();
        for named in [(Some(second.clone()), Some(second.clone())), (None, None)] {
            let refused = plan_of(&front, named.clone())
                .await
                .expect_err("a plan named two ways, or none");
            assert_eq!(refused.code, "invalid_params", "{named:?}");
            let refused = rewind_to(&front, named.clone(), true, None)
                .await
                .expect_err("a rewind named two ways, or none");
            assert_eq!(refused.code, "invalid_params", "{named:?}");
        }
        let refused = fork_at(&front, (Some(second.clone()), Some(second.clone())))
            .await
            .err()
            .expect("a fork named two ways");
        assert_eq!(refused.code, "invalid_params");
        let refused = plan_of(&front, before(&ItemId::from("i_999")))
            .await
            .expect_err("an item the session does not have");
        assert_eq!(refused.code, "unknown_item");
        // A mark given back with another point than its plan's is refused:
        // to after the second prompt undoes the same turn, from the same
        // place, with the same files (none), and is not the rewind shown.
        let shown = plan_of(&front, before(&second)).await.expect("the plan");
        let after = plan_of(&front, (Some(second.clone()), None))
            .await
            .expect("the plan");
        assert_eq!(after.turns, shown.turns);
        let refused = rewind_to(
            &front,
            (Some(second.clone()), None),
            false,
            Some(shown.mark),
        )
        .await
        .expect_err("a mark of another point");
        assert_eq!(refused.code, code::PLAN_CHANGED);
        assert_eq!(world.log(), log, "nothing was written, and no fork made");
        front.close().await;
    });
}

/// The plan read is refused as the rewind would be: an observer is not
/// shown a plan it may not carry out (the review of step 25's second pull
/// request).
#[test]
fn an_observer_is_not_shown_a_rewinds_plan() {
    let world = World::new("rewind_plan_observer", [says("One."), says("Two.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        front.turn("two").await;
        let second = prompts(&front)[1].clone();
        let observer = front.join(ClientInfo::observer()).await;
        let refused = plan_of(&observer, before(&second))
            .await
            .expect_err("an observer's plan");
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        observer.close().await;
        front.close().await;
    });
}

/// Nor is a plan made while another change of the history is under way
/// (a rewind whose question is open, or whose work is not done): it would
/// be refused `history_changing` as the rewind, and the plan it shows
/// would move under it (the review of step 25's second pull request).
#[test]
fn no_plan_is_made_while_another_rewind_is_under_way() {
    let mut world = World::new("rewind_plan_changing", [says("One."), says("Two.")]);
    world.one_turn_each();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        front.turn("two").await;
        let second = prompts(&front)[1].clone();
        let mut rewind = std::pin::pin!(rewind_to(&front, before(&second), false, None));
        assert!(
            crate::poll_once(rewind.as_mut()).await.is_pending(),
            "the rewind was over at once"
        );
        let refused = plan_of(&front, before(&second))
            .await
            .expect_err("a plan under a rewind");
        assert_eq!(refused.code, code::HISTORY_CHANGING);
        rewind.await.expect("the rewind");
        front.close().await;
    });
}

/// The mark holds where the rewind goes from: a plan the person (or
/// another client) edits after the plan was shown is an entry of the
/// conversation, and the rewind by the old mark is refused, as it would
/// undo the edit unsaid (the review of step 25's second pull request). A
/// record of the session (usage, a lease) is not of the conversation, and
/// does not move it.
#[test]
fn a_plan_edited_after_the_rewind_was_shown_refuses_it() {
    let world = World::new("rewind_plan_edited", [says("One."), says("Two.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        front.turn("two").await;
        let second = prompts(&front)[1].clone();
        let shown = plan_of(&front, before(&second)).await.expect("the plan");
        edit_plan(&front, "Another step").await;
        let again = plan_of(&front, before(&second)).await.expect("the plan");
        assert_eq!(again.turns, shown.turns, "the same turns");
        assert_ne!(again.mark, shown.mark, "a plan from elsewhere");
        let log = world.log();
        let refused = rewind_to(&front, before(&second), false, Some(shown.mark))
            .await
            .expect_err("the plan moved");
        assert_eq!(refused.code, code::PLAN_CHANGED);
        assert_eq!(world.log(), log, "a refused rewind wrote to the log");
        assert_eq!(plan_texts(&front).await, ["Another step"]);
        // Planned again, it goes, and the edit with it, as the plan shown
        // then says.
        rewind_to(&front, before(&second), false, Some(again.mark))
            .await
            .expect("the rewind by the plan shown");
        assert!(plan_texts(&front).await.is_empty());
        front.close().await;
    });
}

/// Puts `byte` at `offset` of the file at `path`, in place: the session
/// reads its log at offsets, so the line keeps its length.
fn poke(path: &std::path::Path, offset: usize, byte: u8) {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(offset as u64)).unwrap();
    file.write_all(&[byte]).unwrap();
}

/// Each reason a file cannot be put back reaches the plan by its own name
/// (`why_not`), as the session's store judges it: a directory on its way
/// now a link, its earlier content gone from the store, a checkpoint of
/// the turns that cannot be read. (What a command changed with nothing
/// recorded of it is shown by the checks' test above; content too large to
/// keep, which no tool writes, by the core's own unit test.)
#[test]
fn a_plan_says_why_each_file_cannot_be_put_back() {
    let world = World::new(
        "rewind_plan_why_not",
        [
            calls(&[("c0", "read", json!({"path": "a.txt"}))]),
            calls(&[
                (
                    "c1",
                    "write",
                    json!({"path": "a.txt", "content": "changed\n"}),
                ),
                (
                    "c2",
                    "write",
                    json!({"path": "sub/b.txt", "content": "new\n"}),
                ),
            ]),
            calls(&[(
                "c3",
                "write",
                json!({"path": "c.txt", "content": "later\n"}),
            )]),
            says("Changed."),
        ],
    );
    world.put("a.txt", "original\n");
    world.put("sub/keep.txt", "kept\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("change them").await;
        let first = prompts(&front)[0].clone();
        let why = |plan: &SessionRewindPlanResult| -> Vec<(String, RewindFileStatus, Option<RewindWhyNot>)> {
            let mut files: Vec<_> = plan
                .files
                .iter()
                .map(|file| (file.path.clone(), file.status.clone(), file.why_not.clone()))
                .collect();
            files.sort();
            files
        };
        let plan = plan_of(&front, before(&first)).await.expect("the plan");
        assert_eq!(
            why(&plan),
            [
                ("a.txt".to_owned(), RewindFileStatus::AsLeft, None),
                ("c.txt".to_owned(), RewindFileStatus::AsLeft, None),
                ("sub/b.txt".to_owned(), RewindFileStatus::AsLeft, None),
            ]
        );

        // A directory on the way is now a link.
        #[cfg(unix)]
        {
            let sub = world.workspace.join("sub");
            let real = world.workspace.join("sub-real");
            std::fs::rename(&sub, &real).unwrap();
            std::os::unix::fs::symlink(&real, &sub).unwrap();
            let plan = plan_of(&front, before(&first)).await.expect("the plan");
            assert_eq!(
                why(&plan)[2],
                (
                    "sub/b.txt".to_owned(),
                    RewindFileStatus::Cannot,
                    Some(RewindWhyNot::ThroughALink)
                )
            );
            std::fs::remove_file(&sub).unwrap();
            std::fs::rename(&real, &sub).unwrap();
        }

        // The earlier content is no longer in the session's store: a file
        // that did not exist needs none.
        let logs = world.logs();
        let checkpoints = logs[0].parent().unwrap().join("checkpoints");
        for blob in std::fs::read_dir(&checkpoints).unwrap() {
            std::fs::remove_file(blob.unwrap().path()).unwrap();
        }
        let plan = plan_of(&front, before(&first)).await.expect("the plan");
        assert_eq!(
            why(&plan),
            [
                (
                    "a.txt".to_owned(),
                    RewindFileStatus::Cannot,
                    Some(RewindWhyNot::Missing)
                ),
                ("c.txt".to_owned(), RewindFileStatus::AsLeft, None),
                ("sub/b.txt".to_owned(), RewindFileStatus::AsLeft, None),
            ]
        );

        // A checkpoint of the turn cannot be read (written, say, in a
        // newer format): what it changed is not known, so no file can be
        // put back, though another checkpoint names it.
        let text = std::fs::read_to_string(&logs[0]).unwrap();
        let mut offset = 0;
        let mut found = None;
        for line in text.split_inclusive('\n') {
            if line.contains("\"kind\":\"checkpoint\"") {
                found = Some(offset + line.find("\"v\":1").expect("a version") + 4);
            }
            offset += line.len();
        }
        poke(&logs[0], found.expect("a checkpoint"), b'2');
        let plan = plan_of(&front, before(&first)).await.expect("the plan");
        assert!(plan.unreadable);
        assert_eq!(
            why(&plan),
            [
                (
                    "a.txt".to_owned(),
                    RewindFileStatus::Cannot,
                    Some(RewindWhyNot::Unreadable)
                ),
                (
                    "sub/b.txt".to_owned(),
                    RewindFileStatus::Cannot,
                    Some(RewindWhyNot::Unreadable)
                ),
            ]
        );
        front.close().await;
    });
}
