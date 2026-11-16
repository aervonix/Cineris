//! A call planned and decided again, with the real engine, the real tools
//! and the machine's shell (`docs/architecture/03-agent-loop.md`, "Tool
//! rounds", "A call is decided again";
//! `docs/architecture/06-permissions-and-sandbox.md`, "What an answer
//! depends on").
//!
//! An earlier call of a response makes a link, `d`, and a later call names
//! a path through it. The later call was planned and decided while `d` did
//! not exist; when its turn to start comes it is planned and decided again,
//! and the engine judges where `d` now leads. On Linux the link is a
//! symbolic link (`ln -s`); on Windows a junction (`New-Item -ItemType
//! Junction`), the directory link any account can make.
//!
//! And the twin: a rule taken back, by a revoked approval or a lowered
//! level, while an earlier call's question is open.

use std::path::Path;

use cortex_protocol::event::TurnOutcome;
use cortex_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalRequest, Op, OpResult, PermissionsRevokeParams,
    SessionRef, SessionSetParams,
};
use cortex_protocol::session::SessionSettings;
use serde_json::json;

use crate::{Answering, Front, World, calls, says};

/// A command that makes `d`, in the workspace, a link to the directory
/// `target`.
fn link_d_to(target: &Path) -> String {
    if cfg!(windows) {
        format!(
            "New-Item -ItemType Junction -Path d -Target '{}'",
            target.display()
        )
    } else {
        format!("ln -s '{}' d", target.display())
    }
}

/// A command that copies `evil` to `to`.
fn copy_evil_to(to: &str) -> String {
    if cfg!(windows) {
        format!("Copy-Item evil {to}")
    } else {
        format!("cp evil {to}")
    }
}

/// A command that makes the empty file `name`.
fn touch(name: &str) -> String {
    if cfg!(windows) {
        format!("New-Item {name}")
    } else {
        format!("touch {name}")
    }
}

fn shell(id: &str, command: &str) -> (String, &'static str, serde_json::Value) {
    (id.to_owned(), "shell", json!({ "command": command }))
}

fn response(list: &[(String, &'static str, serde_json::Value)]) -> cortex_testkit::Response {
    let borrowed: Vec<(&str, &str, serde_json::Value)> = list
        .iter()
        .map(|(id, tool, arguments)| (id.as_str(), *tool, arguments.clone()))
        .collect();
    calls(&borrowed)
}

/// Whether a question names `wanted` among its reasons.
fn asks_about(request: &ApprovalRequest, wanted: &str) -> bool {
    request
        .reasons
        .iter()
        .any(|reason| reason.replace('\\', "/").contains(wanted))
}

/// Whether a question is asked because a path is protected: the engine
/// names the path where it leads (the command line itself names
/// `d/.bashrc`).
fn asks_about_protected(request: &ApprovalRequest, leads_to: &str) -> bool {
    request.reasons.iter().any(|reason| {
        let reason = reason.replace('\\', "/");
        reason.contains("protected path") && reason.contains(leads_to)
    })
}

/// A result by its call's tool and position among that tool's results:
/// results are listed as they completed.
fn outcome_of(front: &Front, tool: &str, nth: usize) -> String {
    front
        .results()
        .into_iter()
        .filter(|(name, _, _)| name == tool)
        .nth(nth)
        .map(|(_, outcome, _)| outcome)
        .unwrap_or_default()
}

/// The case the item names: a link to the workspace's root, then a copy to
/// `d/.bashrc`, a protected path once `d` leads to the root. The copy was
/// asked about as a command that is on no safe list, and allowed; decided
/// again, it is asked about once more, for the protected path, which is
/// another question; refused, nothing is written.
#[test]
fn sys_a_copy_through_a_link_an_earlier_command_made_is_asked_about_again() {
    let world = World::new("redecide_copy", []);
    let link = link_d_to(&world.workspace);
    world.provider.push(response(&[
        shell("c1", &link),
        shell("c2", &copy_evil_to("d/.bashrc")),
    ]));
    world.provider.push(says("Done."));
    world.put("evil", "echo evil\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::with(|request| {
            if asks_about_protected(request, "app/.bashrc") {
                ApprovalChoice::Deny
            } else {
                ApprovalChoice::AllowOnce
            }
        }));
        let end = front.turn("go").await;
        assert_eq!(end.outcome, TurnOutcome::Completed);
        let approvals = front.approvals();
        assert_eq!(approvals.len(), 3, "{approvals:#?}");
        assert_eq!(approvals[1].call_id.as_str(), "c2");
        assert!(
            !asks_about_protected(&approvals[1], ".bashrc"),
            "{:#?}",
            approvals[1]
        );
        assert_eq!(approvals[2].call_id.as_str(), "c2");
        assert!(
            asks_about_protected(&approvals[2], "app/.bashrc"),
            "the second question is about the protected path: {:#?}",
            approvals[2]
        );
        assert_eq!(world.read(".bashrc"), None, "nothing was written");
        assert_eq!(
            outcome_of(&front, "shell", 0),
            "success",
            "{:?}",
            front.results()
        );
        assert_eq!(
            outcome_of(&front, "shell", 1),
            "denied",
            "{:?}",
            front.results()
        );
        front.close().await;
    });
}

/// `write` and `edit` hold the path as it was resolved when they were
/// planned. Planned again, the write to `d/hooks/pre-commit` is a write
/// into `.git`, which the person is asked about; before, it was a write in
/// the workspace that needed no answer.
#[test]
fn sys_a_write_through_a_link_an_earlier_command_made_is_planned_again_and_asked_about() {
    let world = World::new("redecide_write", []);
    let link = link_d_to(&world.workspace.join(".git"));
    world.provider.push(response(&[
        shell("c1", &link),
        (
            "c2".to_owned(),
            "write",
            json!({"path": "d/hooks/pre-commit", "content": "evil\n"}),
        ),
    ]));
    world.provider.push(says("Done."));
    world.put(".git/hooks/README", "hooks\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::with(|request| {
            if request.tool == "write" {
                ApprovalChoice::Deny
            } else {
                ApprovalChoice::AllowOnce
            }
        }));
        let end = front.turn("go").await;
        assert_eq!(end.outcome, TurnOutcome::Completed);
        let approvals = front.approvals();
        assert_eq!(approvals.len(), 2, "{approvals:#?}");
        assert_eq!(approvals[1].tool, "write");
        let ApprovalDetail::Diff(change) = &approvals[1].detail else {
            panic!("not a change to a file: {:?}", approvals[1].detail);
        };
        assert!(
            change
                .path
                .replace('\\', "/")
                .ends_with(".git/hooks/pre-commit"),
            "the question names where the path leads: {}",
            change.path
        );
        assert_eq!(world.read(".git/hooks/pre-commit"), None);
        assert_eq!(
            outcome_of(&front, "write", 0),
            "denied",
            "{:?}",
            front.results()
        );
        front.close().await;
    });
}

/// The twin for reads: a read through a link to a directory outside the
/// workspace was planned as a read in the workspace. Planned again, it is
/// a read beyond the scope, and is asked about; refused, nothing of the
/// file reaches the model.
#[test]
fn sys_a_read_through_a_link_an_earlier_command_made_is_planned_again_and_asked_about() {
    let world = World::new("redecide_read", []);
    let outside = world.dir.join("outside");
    let link = link_d_to(&outside);
    world.provider.push(response(&[
        shell("c1", &link),
        ("c2".to_owned(), "read", json!({"path": "d/notes.txt"})),
    ]));
    world.provider.push(says("Done."));
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("notes.txt"), "not for the model\n").unwrap();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::with(|request| {
            if request.tool == "read" {
                ApprovalChoice::Deny
            } else {
                ApprovalChoice::AllowOnce
            }
        }));
        let end = front.turn("go").await;
        assert_eq!(end.outcome, TurnOutcome::Completed);
        let approvals = front.approvals();
        assert_eq!(approvals.len(), 2, "{approvals:#?}");
        assert_eq!(approvals[1].tool, "read");
        assert!(asks_about(&approvals[1], "outside"), "{:#?}", approvals[1]);
        let results = front.results();
        assert_eq!(outcome_of(&front, "read", 0), "denied", "{results:?}");
        assert!(
            results
                .iter()
                .all(|(_, _, text)| !text.contains("not for the model")),
            "{results:?}"
        );
        front.close().await;
    });
}

/// Point 1 of the design: "allow once" leaves no rule, and a command
/// decided again after an earlier command ran is asked the same question.
/// It is not asked again: the policy remembers what the person allowed,
/// for that call.
#[test]
fn sys_a_call_a_person_allowed_is_not_asked_again_when_it_is_decided_again() {
    let world = World::new(
        "redecide_allowed_once",
        [
            response(&[shell("c1", &touch("a")), shell("c2", &touch("b"))]),
            says("Done."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::allow_all());
        let end = front.turn("go").await;
        assert_eq!(end.outcome, TurnOutcome::Completed);
        let approvals = front.approvals();
        let asked: Vec<&str> = approvals
            .iter()
            .map(|asked| asked.call_id.as_str())
            .collect();
        assert_eq!(asked, ["c1", "c2"], "each asked once");
        assert!(world.read("a").is_some() && world.read("b").is_some());
        front.close().await;
    });
}

/// A read outside the workspace, asked about, then a write to a protected
/// path, which the session's approval allowed when it was planned: the
/// write waits behind the read's question.
fn read_then_protected_write(world: &World) -> cortex_testkit::Response {
    let outside = world.dir.join("outside.txt");
    calls(&[
        ("c3", "read", json!({"path": outside.to_str().unwrap()})),
        (
            "c4",
            "write",
            json!({"path": ".github/workflows/ci.yml", "content": "on: pull_request\n"}),
        ),
    ])
}

const PROTECTED: &str = ".github/workflows/ci.yml";

/// The twin from step 17, with an approval: allowed for the session in an
/// earlier turn, it is taken back while an earlier call's question is
/// open. The write was allowed when it was planned; when its turn comes the
/// rules have narrowed, and it is decided again, and asked about.
#[test]
fn a_call_allowed_by_an_approval_revoked_while_it_waits_is_asked_about() {
    let first = calls(&[(
        "c1",
        "write",
        json!({"path": PROTECTED, "content": "on: push\n"}),
    )]);
    let world = World::new("redecide_revoked", []);
    let second = read_then_protected_write(&world);
    for response in [first, says("Set up."), second, says("Done.")] {
        world.provider.push(response);
    }
    std::fs::write(world.dir.join("outside.txt"), "outside\n").unwrap();
    // Later calls with side effects wait behind an open question.
    let core = world.core_with(|options| options.answers_may_stop = true);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowSession).unwrap();
        front.turns(1).await;
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: push\n"));

        front.prompt("again").await;
        let (id, request) = front.approval().await;
        assert_eq!(request.tool, "read");
        // While the read's question is open, the approval is taken back.
        let listed = front
            .op(Op::PermissionsList(SessionRef {
                session_id: front.session.clone(),
            }))
            .await
            .unwrap();
        let OpResult::PermissionsList(listed) = listed else {
            panic!("not approvals");
        };
        front
            .op(Op::PermissionsRevoke(PermissionsRevokeParams {
                session_id: front.session.clone(),
                ids: vec![listed.approvals[0].id.clone()],
            }))
            .await
            .unwrap();
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        let (id, request) = front.approval().await;
        assert_eq!(request.call_id.as_str(), "c4", "the write is asked about");
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(2).await;
        assert_eq!(world.read(PROTECTED).as_deref(), Some("on: push\n"));
        front.close().await;
    });
}

/// The same with a lowered level: a write in the workspace, which needs no
/// answer at `supervised`, decided while the read's question is open; the
/// level goes down to `read-only`, where a write is refused, before the
/// read is answered.
#[test]
fn a_call_decided_before_the_level_was_lowered_is_decided_again_and_refused() {
    let world = World::new("redecide_lowered", []);
    let outside = world.dir.join("outside.txt");
    world.provider.push(calls(&[
        ("c1", "read", json!({"path": outside.to_str().unwrap()})),
        (
            "c2",
            "write",
            json!({"path": "notes.md", "content": "notes\n"}),
        ),
    ]));
    world.provider.push(says("Done."));
    std::fs::write(world.dir.join("outside.txt"), "outside\n").unwrap();
    let core = world.core_with(|options| options.answers_may_stop = true);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("go").await;
        let (id, request) = front.approval().await;
        assert_eq!(request.tool, "read");
        front
            .op(Op::SessionSet(SessionSetParams {
                session_id: front.session.clone(),
                settings: SessionSettings {
                    level: Some(cortex_protocol::lease::Level::ReadOnly),
                    ..SessionSettings::default()
                },
            }))
            .await
            .unwrap();
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        assert_eq!(world.read("notes.md"), None, "{:?}", front.results());
        assert_eq!(
            outcome_of(&front, "read", 0),
            "success",
            "{:?}",
            front.results()
        );
        assert_eq!(
            outcome_of(&front, "write", 0),
            "denied",
            "{:?}",
            front.results()
        );
        front.close().await;
    });
}

/// Found by the review of the design: the remembered answer is kept with
/// every access of the call as the person was shown it. A command's
/// reasons name its words, not the directory it runs in, so a command
/// allowed to run in `sub`, planned again after `sub` became a link to
/// `other`, asks the same question about another place: it is asked again.
/// (A link to a protected directory, `.git`, is asked about for that
/// reason too: `cortex-policy`'s `working_directory`.)
#[test]
fn sys_a_command_whose_directory_moved_is_asked_about_again() {
    let world = World::new("redecide_cwd", []);
    let other = world.workspace.join("other");
    let relink = if cfg!(windows) {
        format!(
            "Remove-Item sub; New-Item -ItemType Junction -Path sub -Target '{}'",
            other.display()
        )
    } else {
        format!("rmdir sub && ln -s '{}' sub", other.display())
    };
    world.provider.push(response(&[
        shell("c1", &relink),
        (
            "c2".to_owned(),
            "shell",
            json!({"command": "make", "cwd": "sub"}),
        ),
    ]));
    world.provider.push(says("Done."));
    world.put("other/README", "other\n");
    std::fs::create_dir_all(world.workspace.join("sub")).unwrap();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let in_other = |request: &ApprovalRequest| match &request.detail {
            ApprovalDetail::Command(command) => command.cwd.replace('\\', "/").ends_with("/other"),
            _ => false,
        };
        front.answering(Answering::with(move |request| {
            if in_other(request) {
                ApprovalChoice::Deny
            } else {
                ApprovalChoice::AllowOnce
            }
        }));
        let end = front.turn("go").await;
        assert_eq!(end.outcome, TurnOutcome::Completed);
        let approvals = front.approvals();
        assert_eq!(approvals.len(), 3, "{approvals:#?}");
        assert_eq!(approvals[1].call_id.as_str(), "c2");
        assert_eq!(approvals[2].call_id.as_str(), "c2");
        assert_eq!(
            approvals[1].reasons, approvals[2].reasons,
            "the same reasons, about another directory"
        );
        assert!(in_other(&approvals[2]), "{:#?}", approvals[2]);
        assert_eq!(
            outcome_of(&front, "shell", 1),
            "denied",
            "{:?}",
            front.results()
        );
        front.close().await;
    });
}

/// Found by the review of the design: a provider's call id may repeat (a
/// server that gives none has ids made up for it, `call_1` in every
/// response). What a person allowed is kept by the loop's own id for the
/// call, so a later call with the same provider id, in the same turn, is
/// asked about as any call is.
#[test]
fn sys_an_answer_is_not_carried_to_a_later_call_that_has_the_same_provider_id() {
    let world = World::new("redecide_same_id", []);
    for response in [
        calls(&[("call_1", "shell", json!({"command": "make"}))]),
        calls(&[("call_1", "shell", json!({"command": "make"}))]),
        says("Done."),
    ] {
        world.provider.push(response);
    }
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::allow_all());
        let end = front.turn("go").await;
        assert_eq!(end.outcome, TurnOutcome::Completed);
        let approvals = front.approvals();
        assert_eq!(approvals.len(), 2, "each call asked: {approvals:#?}");
        front.close().await;
    });
}

/// Found by the review of the design: a revoke that came while a change of
/// level was being made reached the engine being replaced only, and the
/// new engine, which had taken the approvals over before it, kept the
/// approval. The count of narrowings depends on it too. Each blocking
/// piece waits one turn here, so the test stops the change of level after
/// it has made its engine and before it puts it in force, and revokes
/// then.
#[test]
fn a_revoke_made_while_the_level_changes_is_not_lost() {
    let mut world = World::new(
        "redecide_revoke_race",
        [
            calls(&[(
                "c1",
                "write",
                json!({"path": PROTECTED, "content": "on: push\n"}),
            )]),
            says("Set up."),
        ],
    );
    world.one_turn_each();
    let core = world.core();
    // Every change of the approvals passes this point of the engine in
    // force, under the lock a change of level holds: the answer recorded
    // and the revoke below are counted there (the last review found both
    // could be routed round it with no test failing).
    let changing = cortex_testkit::Hold::new("core.policy.changing-approvals");
    changing.release();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowSession).unwrap();
        front.turns(1).await;
        assert_eq!(
            changing.arrivals(),
            1,
            "the answer was recorded round the lock"
        );
        let approval = approvals_of(&front).await[0].id.clone();

        let mut level = std::pin::pin!(front.op(Op::SessionSet(SessionSetParams {
            session_id: front.session.clone(),
            settings: SessionSettings {
                level: Some(cortex_protocol::lease::Level::ReadOnly),
                ..SessionSettings::default()
            },
        })));
        // The new engine is made, with the approvals of the old one, and
        // the change waits for its record to be written.
        assert!(crate::poll_once(level.as_mut()).await.is_pending());
        front
            .op(Op::PermissionsRevoke(PermissionsRevokeParams {
                session_id: front.session.clone(),
                ids: vec![approval],
            }))
            .await
            .unwrap();
        assert_eq!(changing.arrivals(), 2, "the revoke was made round the lock");
        level.await.unwrap();
        assert_eq!(
            approvals_of(&front).await,
            Vec::new(),
            "the revoke was lost"
        );
        front.close().await;
    });
}

/// The approvals the session's engine holds now.
async fn approvals_of(front: &Front) -> Vec<cortex_protocol::methods::StoredApproval> {
    match front
        .op(Op::PermissionsList(SessionRef {
            session_id: front.session.clone(),
        }))
        .await
        .unwrap()
    {
        OpResult::PermissionsList(listed) => listed.approvals,
        _ => panic!("not approvals"),
    }
}

/// Found by the last review: the sandbox can stop the turn around a
/// command the engine found only reads, which does not count as a change.
/// The stop counts as a narrowing, so the write after it in the same
/// response is decided again, and the policy's answer is the stop: the
/// turn ends `denied` at that write, with no further request. Linux only:
/// it needs the sandbox's look.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_stop_raised_around_a_command_that_only_reads_stops_the_calls_after_it() {
    if !crate::sandbox::enforcing_here() {
        return;
    }
    let world = World::new("redecide_stop_read_only", []);
    world.put(".git/config", "[core]\n");
    world.put(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": "echo one > one.txt"}),
    )]));
    world.provider.push(says("First done."));
    world.provider.push(calls(&[
        ("c2", "shell", json!({"command": "ls"})),
        (
            "c3",
            "write",
            json!({"path": "notes.md", "content": "notes\n"}),
        ),
    ]));
    world.provider.push(says("Unused."));
    let core = world.core_with(|options| options.launcher = Some(crate::sandbox::launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cortex_core::ClientInfo::user(),
            SessionSettings {
                level: Some(cortex_protocol::lease::Level::Autonomous),
                ..SessionSettings::default()
            },
        )
        .await
        .expect("the session starts");
        front.turn("first").await;
        // Between the turns, something writes a hook: the look before the
        // next command finds it and stops the turn.
        world.put(".git/hooks/pre-commit", "evil\n");
        let requests = world.provider.requests().len();
        let ended = front.turn("second").await;
        assert_eq!(ended.outcome, TurnOutcome::Denied, "{ended:?}");
        assert_eq!(
            world.provider.requests().len(),
            requests + 1,
            "a request after the stop"
        );
        assert_eq!(world.read("notes.md"), None);
        assert_eq!(
            outcome_of(&front, "write", 0),
            "denied",
            "{:?}",
            front.results()
        );
        front.close().await;
    });
}
