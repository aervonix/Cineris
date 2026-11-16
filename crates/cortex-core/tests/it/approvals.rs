//! Approvals: a call that needs an answer is put to the front ends that
//! may give one, and what happens when none can
//! (`docs/architecture/06-permissions-and-sandbox.md`, "Approvals";
//! `docs/architecture/11-rpc-protocol.md`, "Client roles";
//! `docs/architecture/10-headless.md`, "Approvals").
//!
//! The call that asks is a `write` to a continuous-integration workflow,
//! which is a protected path: at every level short of `unrestricted` it
//! needs an answer.

use std::time::Duration;

use cortex_core::{ClientInfo, Outgoing};
use cortex_protocol::event::{Event, Resolution, TurnOutcome};
use cortex_protocol::lease::Effect;
use cortex_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalResponse, ClientRole, Op, OpResult,
    PermissionsRevokeParams, ServerResponse, SessionRef,
};
use cortex_protocol::session::SessionSettings;
use cortex_testkit::Response;
use serde_json::json;

use crate::{Front, World, calls, says, settle};

const PROTECTED: &str = ".github/workflows/ci.yml";

fn protected_write(id: &str) -> Response {
    calls(&[(
        id,
        "write",
        json!({"path": PROTECTED, "content": "on: push\n"}),
    )])
}

fn resolutions(front: &Front) -> Vec<Resolution> {
    front
        .events()
        .into_iter()
        .filter_map(|event| match event.event {
            Event::RequestResolved(resolved) => Some(resolved.resolution),
            _ => None,
        })
        .collect()
}

#[test]
fn a_call_that_needs_an_answer_is_put_to_the_user_and_runs_when_allowed() {
    let world = World::new("approve", [protected_write("c1"), says("Done.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, request) = front.approval().await;
        assert_eq!(request.session_id, front.session);
        assert_eq!(request.tool, "write");
        assert_eq!(request.call_id.as_str(), "c1");
        // Exactly what would happen, as a change to the file, and why it
        // asks.
        let ApprovalDetail::Diff(change) = &request.detail else {
            panic!("not a change to a file: {:?}", request.detail);
        };
        assert!(change.path.replace('\\', "/").ends_with(PROTECTED));
        assert_eq!(change.diff, "@@ -0,0 +1 @@\n+on: push");
        assert_eq!(change.omitted, None);
        assert!(!request.reasons.is_empty());
        assert!(request.choices.contains(&ApprovalChoice::AllowOnce));
        assert!(request.choices.contains(&ApprovalChoice::Deny));
        // Nothing ran while the question was open.
        assert_eq!(world.read(PROTECTED), None);

        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: push\n"));
        assert_eq!(front.results()[0].1, "success");
        assert_eq!(resolutions(&front), [Resolution::Answered]);
        // Nobody was refused for want of somebody to ask.
        assert!(core.permission_denials(&front.session).is_empty());
        front.close().await;
    });
}

/// A change to a file is shown as its tool would make it, an edit as much
/// as a write, so that a front end needs none of the call's arguments to
/// show it (`docs/architecture/11-rpc-protocol.md`, "`approval/request`").
/// It is bounded: a change too large to carry says so and names the file,
/// and one that cannot be worked out before the call runs says why. And it
/// shows nothing the agent may not read without asking.
#[test]
fn an_approval_for_a_change_to_a_file_shows_the_change_within_a_bound() {
    let big: String = (0..10_000)
        .map(|line| format!("line {line:05} of a file far larger than a question carries\n"))
        .collect();
    let edit = |id: &str, old: &str, new: &str| {
        calls(&[(
            id,
            "edit",
            json!({"path": PROTECTED, "edits": [{"old": old, "new": new}]}),
        )])
    };
    let world = World::new(
        "approve_diff",
        [
            edit("c1", "push", "pull_request"),
            calls(&[("c2", "write", json!({"path": PROTECTED, "content": big}))]),
            edit("c3", "text that is not there", "x"),
            calls(&[(
                "c4",
                "write",
                json!({"path": ".env", "content": "TOKEN=new\n"}),
            )]),
            says("Left alone."),
        ],
    );
    // The write is 600 kB, far past what the model may write in one
    // response: what is under test is the bound on a question, not the
    // model's own.
    world.provider.set_output_limit(false);
    world.put(PROTECTED, "on: push\n");
    world.put(".env", "TOKEN=the-old-secret\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("change CI").await;
        let mut details = Vec::new();
        for file in [PROTECTED, PROTECTED, PROTECTED, ".env"] {
            let (id, request) = front.approval().await;
            let ApprovalDetail::Diff(change) = request.detail else {
                panic!("not a change to a file: {:?}", request.detail);
            };
            assert!(change.path.replace('\\', "/").ends_with(file));
            details.push((change.diff, change.omitted));
            front.answer(&id, ApprovalChoice::Deny).unwrap();
        }
        front.turns(1).await;

        // The edit, as the edit tool would make it.
        assert_eq!(
            details[0],
            ("@@ -1 +1 @@\n-on: push\n+on: pull_request".to_owned(), None)
        );
        // Too large to carry: left out, and said.
        let (diff, omitted) = &details[1];
        assert!(diff.is_empty());
        let omitted = omitted.as_deref().unwrap();
        assert!(omitted.contains("an approval request carries"), "{omitted}");
        // An edit that will fail: why.
        let (diff, omitted) = &details[2];
        assert!(diff.is_empty());
        let omitted = omitted.as_deref().unwrap();
        assert!(omitted.contains("cannot be worked out"), "{omitted}");
        assert!(omitted.contains("not found"), "{omitted}");
        // A file the agent may not read without asking (it looks like a
        // secret) is not shown in a question: its old content would be.
        let (diff, omitted) = &details[3];
        assert!(diff.is_empty(), "{diff}");
        let omitted = omitted.as_deref().unwrap();
        assert!(omitted.contains("may not read"), "{omitted}");
        assert!(!omitted.contains("the-old-secret"), "{omitted}");
        // Nothing was changed by a question.
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: push\n"));
        front.close().await;
    });
}

#[test]
fn a_denial_with_a_note_reaches_the_model_and_the_run_goes_on() {
    let world = World::new(
        "deny_note",
        [protected_write("c1"), says("I will not, then.")],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;
        core.respond(
            front.client.id,
            &id,
            ServerResponse::Approval(ApprovalResponse {
                choice: ApprovalChoice::Deny,
                note: Some("not in this repository".to_owned()),
            }),
        )
        .unwrap();
        let end = {
            front.turns(1).await;
            front.ends().pop().unwrap()
        };
        assert_eq!(end.outcome, TurnOutcome::Completed);
        assert_eq!(world.read(PROTECTED), None);
        let results = front.results();
        assert_eq!(results[0].1, "denied");
        assert!(
            results[0].2.contains("not in this repository"),
            "{}",
            results[0].2
        );
        assert_eq!(front.last_text(), "I will not, then.");
        front.close().await;
    });
}

#[test]
fn only_a_user_client_is_asked_and_only_its_answer_counts() {
    let world = World::new("roles", [protected_write("c1"), says("Done.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let observer = front.join(ClientInfo::observer()).await;
        let supervisor = front
            .join(ClientInfo {
                role: ClientRole::Supervisor,
                ..ClientInfo::user()
            })
            .await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;
        settle().await;

        // Neither was sent the request, though both see the session.
        for other in [&observer, &supervisor] {
            other.drain();
            assert!(
                !other
                    .seen
                    .borrow()
                    .iter()
                    .any(|outgoing| matches!(outgoing, Outgoing::Request { .. })),
                "a client that may not answer was asked"
            );
            assert!(!other.events().is_empty(), "it watches the session");
            // And an answer from it is refused, and changes nothing.
            let refused = other.answer(&id, ApprovalChoice::AllowOnce).unwrap_err();
            assert_eq!(refused.code, "role_not_allowed");
        }
        settle().await;
        assert_eq!(world.read(PROTECTED), None, "a refused answer ran the call");
        assert!(front.ends().is_empty());

        // An observer cannot drive the session either.
        let refused = observer
            .op(Op::TurnInterrupt(
                cortex_protocol::methods::TurnInterruptParams::new(front.session.clone()),
            ))
            .await
            .unwrap_err();
        assert_eq!(refused.code, "role_not_allowed");

        // The user's answer is the one that counts.
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: push\n"));
        front.close().await;
    });
}

#[test]
fn the_first_answer_wins_and_the_other_client_is_told() {
    let world = World::new("first_wins", [protected_write("c1"), says("Done.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let second = front.join(ClientInfo::user()).await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;
        let (same, _) = second.approval().await;
        assert_eq!(id, same, "one request, put to both");

        second.answer(&id, ApprovalChoice::Deny).unwrap();
        // The request is closed: a later answer is too late, and says so.
        let late = front.answer(&id, ApprovalChoice::AllowOnce).unwrap_err();
        assert_eq!(late.code, "request_not_open");
        front.turns(1).await;
        assert_eq!(world.read(PROTECTED), None, "the first answer was a denial");
        // Both are told that it was settled.
        second.turns(1).await;
        assert_eq!(resolutions(&front), [Resolution::Answered]);
        assert_eq!(resolutions(&second), [Resolution::Answered]);
        front.close().await;
        second.close().await;
    });
}

#[test]
fn with_nobody_to_ask_the_call_is_refused_recorded_and_the_run_goes_on() {
    let world = World::new(
        "nobody",
        [protected_write("c1"), says("I could not write it.")],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            ClientInfo::headless(),
            SessionSettings::default(),
        )
        .await
        .unwrap();
        let end = front.turn("set up CI").await;
        // The model is told, and carries on: the turn completes.
        assert_eq!(end.outcome, TurnOutcome::Completed);
        assert_eq!(world.provider.remaining(), 0, "the run went on");
        assert_eq!(world.read(PROTECTED), None);
        assert_eq!(front.results()[0].1, "denied");
        assert_eq!(front.last_text(), "I could not write it.");

        let denials = core.permission_denials(&front.session);
        assert_eq!(denials.len(), 1);
        assert_eq!(denials[0].tool, "write");
        assert_eq!(denials[0].call_id.as_str(), "c1");
        assert_eq!(denials[0].turn_id.as_str(), "t_1");
        assert!(!denials[0].reason.is_empty());
        front.close().await;
    });
}

#[test]
fn with_fail_on_denial_a_call_nobody_can_answer_ends_the_turn_as_denied() {
    let world = World::new(
        "fail_on_denial",
        [protected_write("c1"), says("never sent")],
    );
    let core = world.core_with(|options| options.fail_on_denial = true);
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            ClientInfo::headless(),
            SessionSettings::default(),
        )
        .await
        .unwrap();
        let end = front.turn("set up CI").await;
        assert_eq!(end.outcome, TurnOutcome::Denied);
        let error = end.error.expect("the turn says why it ended");
        assert_eq!(error.code, "permission_denied");
        assert_eq!(error.kind, cortex_protocol::ErrorKind::Permission);
        // The turn stopped there: the model was not asked again.
        assert_eq!(
            world.provider.remaining(),
            1,
            "the run went on after the denial"
        );
        assert_eq!(world.read(PROTECTED), None);
        assert_eq!(core.permission_denials(&front.session).len(), 1);
        front.close().await;
    });
}

#[test]
fn fail_on_denial_changes_nothing_when_somebody_can_answer() {
    let world = World::new("fail_asked", [protected_write("c1"), says("Done.")]);
    let core = world.core_with(|options| options.fail_on_denial = true);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        // A person refused: a result for the model, not the end of the turn.
        assert_eq!(front.ends()[0].outcome, TurnOutcome::Completed);
        assert!(core.permission_denials(&front.session).is_empty());
        front.close().await;
    });
}

#[test]
fn an_approval_nobody_answers_in_time_is_a_denial() {
    let world = World::new("timeout", [protected_write("c1"), says("Nobody answered.")]);
    let core = world.core_with(|options| options.approval_timeout = Some(Duration::from_secs(30)));
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;
        settle().await;
        assert!(world.timer.running().contains(&Duration::from_secs(30)));
        world.timer.advance(Duration::from_secs(29));
        settle().await;
        assert!(front.ends().is_empty(), "the limit has not passed");
        world.timer.advance(Duration::from_secs(2));
        front.turns(1).await;
        assert_eq!(front.results()[0].1, "denied");
        assert_eq!(world.read(PROTECTED), None);
        assert_eq!(resolutions(&front), [Resolution::TimedOut]);
        // The answer that comes after it is too late.
        let late = front.answer(&id, ApprovalChoice::AllowOnce).unwrap_err();
        assert_eq!(late.code, "request_not_open");
        front.close().await;
    });
}

#[test]
fn an_interrupt_withdraws_an_open_question() {
    let world = World::new("withdrawn", [protected_write("c1")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;
        front.interrupt(false).await;
        front.turns(1).await;
        assert_eq!(front.ends()[0].outcome, TurnOutcome::Interrupted);
        assert_eq!(resolutions(&front), [Resolution::Cancelled]);
        assert_eq!(
            front
                .answer(&id, ApprovalChoice::AllowOnce)
                .unwrap_err()
                .code,
            "request_not_open"
        );
        assert_eq!(world.read(PROTECTED), None);
        front.close().await;
    });
}

#[test]
fn allowed_for_the_session_is_not_asked_again_and_is_gone_with_the_session() {
    let world = World::new(
        "allow_session",
        [
            protected_write("c1"),
            calls(&[("c2", "read", json!({"path": PROTECTED}))]),
            calls(&[(
                "c3",
                "write",
                json!({"path": PROTECTED, "content": "on: pull_request\n"}),
            )]),
            says("Done."),
            protected_write("c4"),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, request) = front.approval().await;
        assert!(request.choices.contains(&ApprovalChoice::AllowSession));
        // What the answer would add is shown before it is given.
        assert_eq!(request.adds.len(), 1, "{:?}", request.adds);
        assert_eq!(request.adds[0].effect, Effect::Write);
        front.answer(&id, ApprovalChoice::AllowSession).unwrap();
        // The second write to the same file is not asked about.
        front.turns(1).await;
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: pull_request\n"));
        let listed = front
            .op(Op::PermissionsList(SessionRef {
                session_id: front.session.clone(),
            }))
            .await
            .unwrap();
        let OpResult::PermissionsList(listed) = listed else {
            panic!("not approvals");
        };
        assert_eq!(listed.approvals.len(), 1);
        front.close().await;

        // A new session asks again.
        let front = Front::start(&world, &core).await;
        front.prompt("again").await;
        let (_, request) = front.approval().await;
        assert_eq!(request.call_id.as_str(), "c4");
        front.interrupt(false).await;
        front.turns(1).await;
        front.close().await;
    });
}

#[test]
fn allowed_for_the_project_is_stored_outlives_the_run_and_can_be_revoked() {
    let world = World::new(
        "allow_project",
        [
            protected_write("c1"),
            says("Done."),
            calls(&[("c2", "read", json!({"path": PROTECTED}))]),
            protected_write("c3"),
            says("Again."),
            protected_write("c4"),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, request) = front.approval().await;
        assert!(request.choices.contains(&ApprovalChoice::AllowProject));
        front.answer(&id, ApprovalChoice::AllowProject).unwrap();
        front.turns(1).await;
        front.close().await;
    });
    // In the user's state directory, never in the repository.
    let state = world.dirs().state.unwrap();
    // One file for the project, beside the lock a change to it holds.
    let stored: Vec<_> = std::fs::read_dir(state.join("approvals"))
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().extension().is_none_or(|ext| ext != "lock"))
        .collect();
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert!(!world.workspace.join(".cortex").exists());

    // Another run of the program: the approval still stands.
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let end = front.turn("once more").await;
        assert_eq!(end.outcome, TurnOutcome::Completed);
        assert!(
            !front
                .seen
                .borrow()
                .iter()
                .any(|outgoing| matches!(outgoing, Outgoing::Request { .. })),
            "a stored approval was asked about again"
        );

        // Revoked, it asks again.
        let listed = front
            .op(Op::PermissionsList(SessionRef {
                session_id: front.session.clone(),
            }))
            .await
            .unwrap();
        let OpResult::PermissionsList(listed) = listed else {
            panic!("not approvals");
        };
        assert_eq!(listed.approvals.len(), 1);
        front
            .op(Op::PermissionsRevoke(PermissionsRevokeParams {
                session_id: front.session.clone(),
                ids: vec![listed.approvals[0].id.clone()],
            }))
            .await
            .unwrap();
        front.prompt("and again").await;
        let (_, request) = front.approval().await;
        assert_eq!(request.call_id.as_str(), "c4");
        front.interrupt(false).await;
        front.turns(2).await;
        front.close().await;
    });
}

/// A `user` client can attach to a session at any time and answer what is
/// waiting (`docs/architecture/11-rpc-protocol.md`, "Client roles").
#[test]
fn a_user_who_joins_while_a_question_waits_is_put_it_and_may_answer() {
    let world = World::new("late_join", [protected_write("c1"), says("Done.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;

        // An observer that joins now is put nothing.
        let observer = front.join(ClientInfo::observer()).await;
        settle().await;
        observer.drain();
        assert!(observer.approvals().is_empty());

        let late = front.join(ClientInfo::user()).await;
        let (same, request) = late.approval().await;
        assert_eq!(same, id, "the question that was waiting, under its id");
        assert_eq!(request.call_id.as_str(), "c1");
        late.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: push\n"));
        // The client that was asked first is told it was settled.
        assert_eq!(resolutions(&front), [Resolution::Answered]);
        front.close().await;
    });
}

/// An observer watches. It starts nothing, changes no history and ends
/// nothing.
#[test]
fn an_observer_starts_nothing_changes_no_history_and_ends_nothing() {
    let world = World::new("observer", [says("One."), says("Two.")]);
    world.put("a.txt", "original\n");
    let core = world.core();
    world.run(async {
        let refused = |result: Result<OpResult, cortex_core::OpError>| {
            let error = result.expect_err("an observer was let through");
            assert_eq!(error.code, "role_not_allowed", "{}", error.message);
        };
        let started = Front::start_as(
            &world,
            &core,
            ClientInfo::observer(),
            SessionSettings::default(),
        )
        .await;
        assert_eq!(started.err().unwrap().code, "role_not_allowed");

        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        let first = front
            .items()
            .iter()
            .find_map(|item| match item {
                cortex_protocol::Item::UserMessage(message) => Some(message.id.clone()),
                _ => None,
            })
            .unwrap();
        let observer = front.join(ClientInfo::observer()).await;
        refused(
            observer
                .op(Op::SessionRewind(
                    cortex_protocol::methods::SessionRewindParams {
                        session_id: front.session.clone(),
                        to: first,
                        restore_files: true,
                    },
                ))
                .await,
        );
        refused(
            observer
                .op(Op::SessionFork(
                    cortex_protocol::methods::SessionForkParams {
                        session_id: front.session.clone(),
                        at: None,
                    },
                ))
                .await,
        );
        refused(
            observer
                .op(Op::Shutdown(cortex_protocol::methods::Empty {}))
                .await,
        );
        // Nothing of it happened: the session is there, with its history.
        front.turn("two").await;
        assert_eq!(front.last_text(), "Two.");
        assert_eq!(world.provider.requests()[1].messages.len(), 3);
        observer.close().await;
        front.close().await;

        // A stored session is not started by one either.
        let watcher = core.connect(ClientInfo::observer());
        refused(
            core.handle(
                watcher.id,
                Op::SessionOpen(SessionRef {
                    session_id: front.session.clone(),
                }),
            )
            .await,
        );
    });
}

/// The approvals in force for the session.
async fn listed(front: &Front) -> Vec<cortex_protocol::methods::StoredApproval> {
    let result = front
        .op(Op::PermissionsList(SessionRef {
            session_id: front.session.clone(),
        }))
        .await
        .unwrap();
    let OpResult::PermissionsList(listed) = result else {
        panic!("not approvals");
    };
    listed.approvals
}

/// What the user allowed for the session is still the session's own after
/// the level changed: it still allows, it is listed as the session's, and
/// an approval given afterwards is another approval, with another id.
#[test]
fn what_was_allowed_for_the_session_stays_the_sessions_when_the_level_changes() {
    let other = ".github/workflows/release.yml";
    let world = World::new(
        "allow_session_level",
        [
            protected_write("c1"),
            says("Done."),
            calls(&[("c2", "read", json!({"path": PROTECTED}))]),
            calls(&[(
                "c3",
                "write",
                json!({"path": PROTECTED, "content": "on: pull_request\n"}),
            )]),
            calls(&[(
                "c4",
                "write",
                json!({"path": other, "content": "on: release\n"}),
            )]),
            says("Both."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowSession).unwrap();
        front.turns(1).await;
        let before = listed(&front).await;
        assert_eq!(before.len(), 1);

        // Down a level and up again: two new engines.
        for level in [
            cortex_protocol::lease::Level::ReadOnly,
            cortex_protocol::lease::Level::Supervised,
        ] {
            front
                .op(Op::SessionSet(cortex_protocol::methods::SessionSetParams {
                    session_id: front.session.clone(),
                    settings: SessionSettings {
                        level: Some(level),
                        ..SessionSettings::default()
                    },
                }))
                .await
                .unwrap();
        }
        let after = listed(&front).await;
        assert_eq!(after, before, "the same approval, still the session's");
        assert_eq!(
            after[0].scope,
            cortex_protocol::methods::ApprovalScope::Session
        );

        // It still allows; and another file under the same protection is
        // another question, whose approval is another approval.
        front.prompt("more CI").await;
        let (id, request) = front.approval().await;
        assert_eq!(
            request.call_id.as_str(),
            "c4",
            "the first file was asked again"
        );
        front.answer(&id, ApprovalChoice::AllowSession).unwrap();
        front.turns(2).await;
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: pull_request\n"));
        let both = listed(&front).await;
        assert_eq!(both.len(), 2, "{both:?}");
        assert_ne!(both[0].id, both[1].id, "two approvals under one id");

        // Taking the first back takes back the first alone.
        front
            .op(Op::PermissionsRevoke(
                cortex_protocol::methods::PermissionsRevokeParams {
                    session_id: front.session.clone(),
                    ids: vec![before[0].id.clone()],
                },
            ))
            .await
            .unwrap();
        let left = listed(&front).await;
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, both[1].id);
        front.close().await;
    });
}

/// Two calls in one response: a write that is asked about, and after it a
/// write that is allowed. Runs the turn as far as the open question, gives
/// the second write every chance to run, and says whether it did. Then
/// allows the first, and checks that both were written. `change` sets the
/// core up.
pub(crate) fn later_write_ran_while_the_question_was_open(
    test: &str,
    change: impl FnOnce(&mut cortex_core::CoreOptions),
) -> bool {
    let world = World::new(
        test,
        [
            calls(&[
                (
                    "c1",
                    "write",
                    json!({"path": PROTECTED, "content": "on: push\n"}),
                ),
                (
                    "c2",
                    "write",
                    json!({"path": "later.txt", "content": "x\n"}),
                ),
            ]),
            says("Done."),
        ],
    );
    let core = world.core_with(change);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, request) = front.approval().await;
        assert_eq!(request.call_id.as_str(), "c1");
        // Long enough for anything that may start to have finished.
        for _ in 0..2_000 {
            if world.read("later.txt").is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let ran = world.read("later.txt").is_some();
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: push\n"));
        assert_eq!(world.read("later.txt").as_deref(), Some("x\n"));
        front.close().await;
        ran
    })
}

/// A person's answer is never "stop the turn", so a later call that was
/// allowed does not wait for it.
#[test]
fn a_later_call_runs_beside_a_question_a_person_is_answering() {
    assert!(later_write_ran_while_the_question_was_open(
        "beside_question",
        |options| options.fail_on_denial = false
    ));
}

/// With `fail_on_denial` an answer may be to stop the turn, and a stop
/// holds for everything the model wrote after the refused call: nothing
/// after an open question that changes anything starts until it is
/// answered.
#[test]
fn with_fail_on_denial_a_later_call_waits_for_the_open_question() {
    assert!(!later_write_ran_while_the_question_was_open(
        "behind_question",
        |options| options.fail_on_denial = true
    ));
}

/// The interactive application can stop the turn from a dialog (its
/// interrupt key), so with `answers_may_stop` nothing after an open
/// question that changes anything starts until it is answered (step 23).
#[test]
fn with_answers_may_stop_a_later_call_waits_for_the_open_question() {
    assert!(!later_write_ran_while_the_question_was_open(
        "behind_question_tui",
        |options| options.answers_may_stop = true
    ));
}

/// The rig's own promise: a turn that asks what the test did not expect
/// fails the test at once, naming what it asked, instead of waiting for an
/// answer until the test is stopped at its limit.
#[test]
fn an_approval_the_test_did_not_expect_fails_it_at_once_naming_the_call() {
    let world = World::new("unexpected", [protected_write("c1"), says("Done.")]);
    let core = world.core();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.turn("set up CI").await;
        });
    }));
    let message = failed.expect_err("the turn waited for an answer nobody gives");
    let message = message
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(
        message.contains("the turn asked for an approval the test did not expect")
            && message.contains("tool `write`")
            && message.contains("front.answering(Answering::allow_all())"),
        "{message}"
    );
    assert_eq!(world.read(PROTECTED), None);
}

#[test]
fn a_front_answers_by_its_policy_what_the_test_did_not_take_up() {
    for (name, answering, written, outcome) in [
        (
            "allow_all",
            crate::Answering::allow_all(),
            Some("on: push\n"),
            "success",
        ),
        ("deny_all", crate::Answering::deny_all(), None, "denied"),
    ] {
        let world = World::new(name, [protected_write("c1"), says("Done.")]);
        let core = world.core();
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.answering(answering);
            front.turn("set up CI").await;
            assert_eq!(world.read(PROTECTED).as_deref(), written, "{name}");
            assert_eq!(front.results()[0].1, outcome, "{name}");
            assert_eq!(front.answered.borrow().len(), 1, "{name}");
            front.close().await;
        });
    }
    // A function sees each request, and chooses.
    let world = World::new("with", [protected_write("c1"), says("Done.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = std::rc::Rc::clone(&asked);
        front.answering(crate::Answering::with(move |request| {
            seen.borrow_mut().push(request.call_id.as_str().to_owned());
            ApprovalChoice::Deny
        }));
        front.turn("set up CI").await;
        assert_eq!(*asked.borrow(), ["c1"]);
        assert_eq!(front.results()[0].1, "denied");
        front.close().await;
    });
}

/// A request another client answers is not one this front must answer:
/// waiting for the turn's end with no policy, it sees the request and its
/// resolution, and goes on.
#[test]
fn a_request_another_client_answered_is_not_unexpected() {
    let world = World::new("answered_elsewhere", [protected_write("c1"), says("Done.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let second = front.join(ClientInfo::user()).await;
        front.prompt("set up CI").await;
        let (id, _) = second.approval().await;
        second.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        assert_eq!(resolutions(&front), [Resolution::Answered]);
        assert!(
            front.answered.borrow().is_empty(),
            "this front answered nothing"
        );
        front.close().await;
        second.close().await;
    });
}

/// The rig's wait for a question fails at once when the turn ends without
/// one: a `write` does not ask at `supervised`, and a test that waited for
/// its question used to wait until nextest stopped it a minute later.
#[test]
#[should_panic(expected = "the question will not come")]
fn a_wait_for_a_question_the_turn_never_asks_fails_at_once() {
    let world = World::new(
        "rig_no_question",
        [
            calls(&[("c1", "write", json!({"path": "a.txt", "content": "a\n"}))]),
            says("Written."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("write it").await;
        let _ = front.approval().await;
    });
}

/// A question that is asked is still found, whether the wait began before
/// it was sent or after, and a turn that ends after it was answered is not
/// taken for one that asked nothing.
#[test]
fn a_wait_for_a_question_finds_one_asked_before_or_after_it_began() {
    let world = World::new(
        "rig_question",
        [protected_write("c1"), protected_write("c2"), says("Done.")],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("write it twice").await;
        let (first, request) = front.approval().await;
        assert_eq!(request.call_id.as_str(), "c1");
        front.answer(&first, ApprovalChoice::Deny).unwrap();
        // The second question may already be on its way.
        settle().await;
        let (second, request) = front.approval().await;
        assert_eq!(request.call_id.as_str(), "c2");
        front.answer(&second, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        front.close().await;
    });
}

/// A turn that ends with nothing asked, while another is queued behind it,
/// is not the end of the wait: the queued turn's question is the one
/// waited for.
#[test]
fn a_wait_for_a_question_goes_on_while_a_turn_is_queued() {
    let world = World::new(
        "rig_queued",
        [
            says("Nothing to ask."),
            protected_write("c1"),
            says("Refused."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("first").await;
        let queued = front
            .op(cortex_protocol::methods::Op::TurnStart(
                cortex_protocol::methods::TurnStartParams {
                    session_id: front.session.clone(),
                    input: vec![cortex_protocol::message::Part::text("second")],
                    delivery: Some(cortex_protocol::item::Delivery::Queue),
                },
            ))
            .await
            .expect("the second prompt is accepted");
        assert!(
            matches!(
                queued,
                cortex_protocol::methods::OpResult::TurnStart(ref started)
                    if started.disposition == cortex_protocol::methods::Disposition::Queued
            ),
            "the second prompt waits behind the first: {queued:?}"
        );
        let (id, request) = front.approval().await;
        assert_eq!(request.call_id.as_str(), "c1");
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(2).await;
        front.close().await;
    });
}

/// `told` waits for a notice and gives back the one that says what it was
/// given, not another told before it; `turn_denying` refuses what is asked
/// and says which tools asked.
#[test]
fn a_notice_is_waited_for_and_a_refused_turn_names_what_asked() {
    let world = World::new("rig_told", [protected_write("c1"), says("Refused.")]);
    world.project_config(&format!("[model]\nmain = \"{}\"\n", crate::OTHER_MODEL));
    // A trust store that cannot be read: a notice of its own, beside the
    // one of what is held back for want of trust.
    let state = world.dirs().state.unwrap();
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("trust.toml"), "this is { not toml").unwrap();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let store = front.told("trust store could not be read").await;
        assert!(!store.contains("model.main"), "{store}");
        let notice = front.told("not trusted").await;
        assert!(notice.contains("model.main"), "{notice}");
        assert_ne!(store, notice);
        assert_eq!(front.turn_denying("write it").await, ["write"]);
        assert_eq!(
            world.read(PROTECTED),
            None,
            "the refused write was not made"
        );
        front.close().await;
    });
}

/// A tool's outcome compared with a name no outcome has (`"ok"`) fails the
/// test at once, where it would have asserted nothing; one of the
/// protocol's names compares as text does.
#[test]
#[should_panic(expected = "which no outcome is")]
fn a_tool_outcome_compared_with_a_name_no_outcome_has_fails_the_test() {
    let world = World::new(
        "rig_outcome",
        [
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            says("Read."),
        ],
    );
    world.put("a.txt", "a\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("read it").await;
        let results = front.results();
        assert_eq!(results[0].1, "success");
        assert_ne!(results[0].1, "denied");
        let _ = results[0].1 == "ok";
    });
}

/// A client that opens the session between the policy's look at who may
/// answer an approval and the request being open is put it too (the review
/// of step 27's core changes): the turn is held between the two, a second
/// client opens the session there, and once the request is open both
/// clients have it, under one id.
#[test]
fn a_client_that_opens_the_session_while_an_approval_is_being_put_is_put_it() {
    let world = World::new(
        "approval_join_race",
        [
            calls(&[("c1", "shell", json!({"command": "ninja"}))]),
            says("Refused."),
        ],
    );
    let core = world.core();
    let opening = cortex_testkit::Hold::new(cortex_core::BEFORE_THE_APPROVAL_IS_OPEN);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("build it").await;
        // The turn runs on the core's task until it stops at the point.
        let mut nothing = std::pin::pin!(std::future::pending::<()>());
        opening.drive_until_reached(nothing.as_mut()).await;
        let other = front.join(ClientInfo::user()).await;
        opening.release();
        let (id, request) = front.approval().await;
        assert_eq!(request.call_id.as_str(), "c1");
        // Put to both in the same step of the turn's task, so the second
        // client has it by now.
        other.drain();
        assert_eq!(
            other.approvals().len(),
            1,
            "the client that opened the session while the approval was being put was never put it"
        );
        other.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        assert_eq!(front.results()[0].1, "denied");
        assert_eq!(opening.timed_out(), 0, "the turn was let go, not timed out");
        front.close().await;
        other.close().await;
    });
}
