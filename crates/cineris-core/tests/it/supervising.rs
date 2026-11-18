//! A session's supervisor: another session's orchestration layer, which
//! answers approvals within its grant (`docs/architecture/17-coordinator.md`,
//! "Approval routing"; `docs/architecture/06-permissions-and-sandbox.md`,
//! "When a session is supervised").
//!
//! The grant here is a test lease that allows the command `make`: a
//! command nothing else allows, which the session at `supervised` asks
//! about. A call the tests do not need to run is refused, so that only one
//! command starts.

use cineris_core::{ClientInfo, OpError};
use cineris_protocol::error::code;
use cineris_protocol::event::{Event, Resolution};
use cineris_protocol::lease::{
    Budget, Effect, HardLimits, Lease, Level, NetworkAccess, PathSet, Rule, RuleAction,
    SandboxPolicy,
};
use cineris_protocol::methods::{
    ApprovalChoice, ApprovalResponse, ClientRole, Op, OpResult, ServerResponse, SessionRef,
};
use serde_json::json;

use crate::{Answering, Front, World, calls, says, settle};

/// The test lease: `make` and nothing else, at `supervised`, anywhere the
/// session itself may reach.
fn grant() -> Lease {
    let everything = PathSet {
        all: true,
        roots: Vec::new(),
    };
    cineris_policy::standing_lease(
        cineris_protocol::ids::LeaseId::new("l_test"),
        cineris_policy::StandingTerms {
            level: Level::Supervised,
            rules: vec![Rule {
                effect: Effect::Exec,
                pattern: "make".to_owned(),
                action: RuleAction::Allow,
            }],
            sandbox: SandboxPolicy {
                read: everything.clone(),
                write: everything,
                protect: PathSet::default(),
                network: NetworkAccess::Off,
            },
            models: Vec::new(),
            budget: Budget::default(),
            limits: HardLimits::default(),
        },
        cineris_policy::SandboxState::Enforcing,
    )
    .expect("a lease")
}

fn supervisor() -> ClientInfo {
    ClientInfo {
        role: ClientRole::Supervisor,
        ..ClientInfo::user()
    }
}

/// A supervisor, with `grant`, that opens `front`'s session.
async fn supervise(front: &Front, grant: Option<Lease>) -> Result<Front, OpError> {
    let client = front.core.connect_with_grant(supervisor(), grant);
    let result = front
        .core
        .handle(
            client.id,
            Op::SessionOpen(SessionRef {
                session_id: front.session.clone(),
            }),
        )
        .await?;
    let OpResult::SessionOpen(result) = result else {
        panic!("not the result of session/open");
    };
    Ok(Front {
        core: front.core.clone(),
        client,
        session: front.session.clone(),
        state: result.session,
        seen: std::cell::RefCell::new(Vec::new()),
        answered: std::cell::RefCell::new(Vec::new()),
        answering: std::cell::RefCell::new(Answering::default()),
    })
}

/// A core on `world` whose sessions' commands are confined: the launcher,
/// and a kernel said to have Landlock. Only there does a grant cover a
/// command, so the tests of a command within a grant run on Linux.
#[cfg(target_os = "linux")]
fn confining(world: &World) -> cineris_core::Core {
    world.core_with(|options| {
        options.launcher = Some(crate::sandbox::launcher());
        options.sandbox = Some(cineris_sandbox::support::Support::Landlock(8));
    })
}

fn shell(id: &str, command: &str) -> cineris_testkit::Response {
    calls(&[(id, "shell", json!({ "command": command }))])
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

/// A call within the grant is put to the supervisor and the user at once,
/// and the supervisor's allow settles it. "For the project" is taken as
/// once: the call runs, nothing is recorded or stored, and the same call
/// later is asked again.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_supervisor_allows_a_call_within_its_grant_once_and_nothing_is_recorded() {
    let world = World::new(
        "supervisor_within",
        [
            shell("c1", "make"),
            says("Built."),
            shell("c2", "make"),
            says("Not again."),
        ],
    );
    let core = confining(&world);
    // Every answer recorded in the engine passes this point, under the lock
    // a change of level holds: counted, a supervisor's answer is not.
    let recorded = cineris_testkit::Hold::new("core.policy.changing-approvals");
    recorded.release();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let layer = supervise(&front, Some(grant())).await.unwrap();
        front.prompt("build it").await;
        let (id, request) = layer.approval().await;
        assert_eq!(request.call_id.as_str(), "c1");
        // The user was put it too, under the same id.
        let (same, _) = front.approval().await;
        assert_eq!(same, id);
        layer.answer(&id, ApprovalChoice::AllowProject).unwrap();
        front.turns(1).await;
        assert_ne!(front.results()[0].1, "denied", "{:?}", front.results());
        assert_eq!(resolutions(&front), [Resolution::Answered]);
        assert_eq!(
            recorded.arrivals(),
            0,
            "a supervisor's answer reached the engine's record"
        );
        // The user's answer after it is too late.
        let late = front.answer(&id, ApprovalChoice::Deny).unwrap_err();
        assert_eq!(late.code, "request_not_open");

        // Nothing was recorded: no approval for the session or the project.
        let listed = front
            .op(Op::PermissionsList(SessionRef {
                session_id: front.session.clone(),
            }))
            .await
            .unwrap();
        let OpResult::PermissionsList(listed) = listed else {
            panic!("not approvals");
        };
        assert!(listed.approvals.is_empty(), "{:?}", listed.approvals);
        let stored = world.dirs().state.unwrap().join("approvals");
        let files: Vec<_> = std::fs::read_dir(&stored)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().extension().is_none_or(|ext| ext != "lock"))
            .collect();
        assert!(files.is_empty(), "stored: {files:?}");

        // The same call in the next turn is asked again; the user refuses.
        front.prompt("again").await;
        let (again, request) = front.approval().await;
        assert_eq!(request.call_id.as_str(), "c2");
        front.answer(&again, ApprovalChoice::Deny).unwrap();
        front.turns(2).await;
        assert_eq!(front.results()[1].1, "denied");
        // The user's answer is recorded, as any person's is.
        assert_eq!(recorded.arrivals(), 1);
        front.close().await;
        layer.close().await;
    });
}

/// A call beyond the grant: the supervisor's allow is not taken, and the
/// request stays open for the user, who answers it. A supervisor's denial
/// is taken, beyond its grant as within it.
#[test]
fn a_call_beyond_the_grant_is_not_taken_from_the_supervisor_and_the_user_answers() {
    let world = World::new(
        "supervisor_beyond",
        [
            shell("c1", "ninja"),
            says("Refused."),
            shell("c2", "ninja"),
            says("Refused again."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let layer = supervise(&front, Some(grant())).await.unwrap();
        front.prompt("build it").await;
        let (id, _) = layer.approval().await;
        for choice in [ApprovalChoice::AllowOnce, ApprovalChoice::AllowProject] {
            let refused = layer.answer(&id, choice).unwrap_err();
            assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        }
        settle().await;
        front.drain();
        assert!(front.ends().is_empty(), "a refused allow settled the call");
        assert!(resolutions(&front).is_empty());
        // Still open: the user answers it.
        let (same, _) = front.approval().await;
        assert_eq!(same, id);
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        assert_eq!(front.results()[0].1, "denied");

        // The supervisor's denial is taken, with its note.
        front.prompt("again").await;
        let (id, _) = layer.approval().await;
        layer
            .core
            .respond(
                layer.client.id,
                &id,
                ServerResponse::Approval(ApprovalResponse {
                    choice: ApprovalChoice::Deny,
                    note: Some("not in this team's task".to_owned()),
                }),
            )
            .unwrap();
        front.turns(2).await;
        let results = front.results();
        assert_eq!(results[1].1, "denied");
        assert!(
            results[1].2.contains("not in this team's task"),
            "{results:?}"
        );
        front.close().await;
        layer.close().await;
    });
}

/// A user may answer at any moment until the request is settled: the
/// user's answer, first, settles it, and the supervisor's allow after it
/// is refused as late, though within the grant.
#[test]
fn a_users_answer_first_settles_it_and_the_supervisors_later_answer_is_refused() {
    let world = World::new("supervisor_late", [shell("c1", "make"), says("Refused.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let layer = supervise(&front, Some(grant())).await.unwrap();
        front.prompt("build it").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        let late = layer.answer(&id, ApprovalChoice::AllowOnce).unwrap_err();
        assert_eq!(late.code, "request_not_open");
        front.turns(1).await;
        assert_eq!(front.results()[0].1, "denied");
        // The supervisor is told it was settled.
        layer.turns(1).await;
        assert_eq!(resolutions(&layer), [Resolution::Answered]);
        front.close().await;
        layer.close().await;
    });
}

/// A session has at most one supervisor: a second one's `session/open` is
/// refused `supervisor_present`, with a grant or without, while a user may
/// still join. Once the first has closed its view, another may open it.
#[test]
fn a_session_has_at_most_one_supervisor() {
    let world = World::new("one_supervisor", [says("Hi.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let first = supervise(&front, None).await.unwrap();
        for grant in [None, Some(grant())] {
            let refused = supervise(&front, grant).await.err().expect("a second one");
            assert_eq!(refused.code, code::SUPERVISOR_PRESENT);
        }
        // The first opening it again is not a second one.
        let again = first
            .op(Op::SessionOpen(SessionRef {
                session_id: front.session.clone(),
            }))
            .await;
        assert!(again.is_ok(), "{again:?}");
        let user = front.join(ClientInfo::user()).await;
        first.close().await;
        let next = supervise(&front, Some(grant())).await.unwrap();
        next.close().await;
        user.close().await;
        front.close().await;
    });
}

/// The supervisor's allow is judged by the session as it is when the
/// answer comes, not when the question was put (the review of step 27's
/// second pull request). Here the grant is `autonomous`, so a command is
/// within it while the session's commands are confined; the user moves the
/// session to `unrestricted` while the question is open, where nothing is
/// confined, and the same allow is then beyond the grant.
#[cfg(target_os = "linux")]
#[test]
fn a_supervisors_allow_is_judged_by_the_session_as_it_is_when_it_answers() {
    let world = World::new(
        "supervisor_judged_late",
        [shell("c1", "ninja"), says("Refused.")],
    );
    let core = confining(&world);
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(
            front.state.sandbox,
            cineris_protocol::session::SandboxStatus::Enforcing
        );
        let mut autonomous = grant();
        autonomous.grant.level = Level::Autonomous;
        autonomous.grant.rules.clear();
        let layer = supervise(&front, Some(autonomous)).await.unwrap();
        front.prompt("build it").await;
        let (id, _) = layer.approval().await;
        front
            .op(Op::SessionSet(
                cineris_protocol::methods::SessionSetParams {
                    session_id: front.session.clone(),
                    settings: cineris_protocol::session::SessionSettings {
                        level: Some(Level::Unrestricted),
                        ..cineris_protocol::session::SessionSettings::default()
                    },
                },
            ))
            .await
            .expect("the user raises the level");
        let refused = layer.answer(&id, ApprovalChoice::AllowOnce).unwrap_err();
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED, "{}", refused.message);
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        assert_eq!(front.results()[0].1, "denied");
        front.close().await;
        layer.close().await;
    });
}

/// A grant that has expired by the core's clock covers nothing, and the
/// refusal says so: the request stays open for the users.
#[test]
fn an_expired_grant_takes_no_allow() {
    let world = World::new(
        "supervisor_expired",
        [shell("c1", "make"), says("Refused.")],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let mut expired = grant();
        expired.expires.kind = cineris_protocol::lease::ExpiryKind::At;
        expired.expires.at = Some(cineris_protocol::Timestamp::from("2026-11-13T09:00:00Z"));
        let layer = supervise(&front, Some(expired)).await.unwrap();
        front.prompt("build it").await;
        let (id, _) = layer.approval().await;
        let refused = layer.answer(&id, ApprovalChoice::AllowOnce).unwrap_err();
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        assert!(refused.message.contains("expired"), "{}", refused.message);
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        assert_eq!(front.results()[0].1, "denied");
        front.close().await;
        layer.close().await;
    });
}

/// The same for a reload that tightens the rules while the request is open
/// (step 24's `config/reload`): the user's configuration now asks about the
/// command, and the supervisor's allow, within the grant when the question
/// was put, is beyond it when the answer comes.
#[cfg(target_os = "linux")]
#[test]
fn a_reload_that_tightens_the_rules_reaches_an_open_requests_judge() {
    let world = World::new("supervisor_reload", [shell("c1", "make"), says("Refused.")]);
    let core = confining(&world);
    world.run(async {
        let front = Front::start(&world, &core).await;
        let layer = supervise(&front, Some(grant())).await.unwrap();
        front.prompt("build it").await;
        let (id, _) = layer.approval().await;
        world.user_config(
            "[[permissions.rules]]\neffect = \"exec\"\nmatch = \"make\"\naction = \"ask\"\n",
        );
        let reloaded = front
            .op(Op::ConfigReload(
                cineris_protocol::methods::ConfigReloadParams {
                    session_id: Some(front.session.clone()),
                    ..cineris_protocol::methods::ConfigReloadParams::default()
                },
            ))
            .await
            .expect("the reload answers");
        let OpResult::ConfigReload(reloaded) = reloaded else {
            panic!("not the result of config/reload");
        };
        assert!(reloaded.errors.is_empty(), "{reloaded:?}");
        let refused = layer.answer(&id, ApprovalChoice::AllowOnce).unwrap_err();
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED, "{}", refused.message);
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        assert_eq!(front.results()[0].1, "denied");
        front.close().await;
        layer.close().await;
    });
}

/// A supervisor is told, when it opens a session, what of its grant does
/// not apply there, and why: here the session's commands are not confined
/// (no launcher), so its allow rule of a command does not apply, and its
/// allow rule of a path never does. Every client of the session is told.
#[test]
fn a_supervisor_is_told_what_of_its_grant_does_not_apply_in_the_session() {
    let world = World::new("supervisor_told", [says("Hi.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let mut limited = grant();
        limited.grant.rules.push(Rule {
            effect: Effect::Write,
            pattern: "src/**".to_owned(),
            action: RuleAction::Allow,
        });
        let layer = supervise(&front, Some(limited)).await.unwrap();
        let told = layer.told("is limited in this session").await;
        for part in [
            "`l_test`",
            "`exec make`",
            "not confined by an enforcing sandbox",
            "`write src/**`",
            "never apply",
        ] {
            assert!(told.contains(part), "{part} in {told}");
        }
        assert_eq!(front.told("is limited in this session").await, told);
        // A supervisor with no grant is told nothing.
        layer.close().await;
        let other = supervise(&front, None).await.unwrap();
        crate::settle().await;
        other.drain();
        assert!(
            !other
                .notices()
                .iter()
                .any(|notice| notice.contains("is limited")),
            "{:?}",
            other.notices()
        );
        other.close().await;
        front.close().await;
    });
}
