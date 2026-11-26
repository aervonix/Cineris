//! Settings: at session start and changed while a session runs
//! (`docs/architecture/11-rpc-protocol.md`, "Settings and state";
//! `docs/decisions/0027-speed-tiers.md`).

use cineris_config::Override;
use cineris_core::ClientInfo;
use cineris_protocol::event::TurnOutcome;
use cineris_protocol::lease::Level;
use cineris_protocol::methods::{ClientRole, Op, OpResult, SessionSetParams};
use cineris_protocol::model::ModelRole;
use cineris_protocol::session::{RoleSettings, SessionSettings, SessionState};
use cineris_protocol::{ModelId, Speed};
use serde_json::json;

use crate::{Front, MODEL, OTHER_MODEL, World, calls, says};

const UNCACHED: &str = "without the provider's cached prefix";

async fn set(
    front: &Front,
    settings: SessionSettings,
) -> Result<SessionState, cineris_core::OpError> {
    let result = front
        .op(Op::SessionSet(SessionSetParams {
            session_id: front.session.clone(),
            settings,
        }))
        .await?;
    let OpResult::SessionSet(result) = result else {
        panic!("not the result of session/set");
    };
    Ok(result.session)
}

fn role(settings: RoleSettings) -> SessionSettings {
    SessionSettings {
        roles: [(ModelRole::Main, settings)].into_iter().collect(),
        ..SessionSettings::default()
    }
}

fn warned(front: &Front) -> usize {
    front.drain();
    front
        .notices()
        .iter()
        .filter(|notice| notice.contains(UNCACHED))
        .count()
}

#[test]
fn a_session_can_be_started_at_a_speed_from_its_settings_or_from_the_command_line() {
    let world = World::new("speed_start", [says("One."), says("Two.")]);
    world.run(async {
        // As `session/new` gives it.
        let core = world.core();
        let front = Front::start_as(
            &world,
            &core,
            ClientInfo::user(),
            SessionSettings {
                speed: Some(Speed::Fast),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(front.state.speed, Speed::Fast);
        let main = &front.state.roles[&ModelRole::Main];
        assert_eq!(
            (&main.speed, &main.speed_in_effect),
            (&Speed::Fast, &Speed::Fast)
        );
        front.turn("go").await;
        assert_eq!(world.provider.requests()[0].speed, Speed::Fast);
        front.close().await;

        // As `--speed fast` gives it: an override of the user's own, so it
        // waits for nothing.
        let core = world.core_with(|options| {
            options.overrides.push(Override {
                key: "model.speed".to_owned(),
                value: "fast".to_owned(),
                flag: Some("--speed".to_owned()),
            });
        });
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.speed, Speed::Fast);
        front.turn("go").await;
        assert_eq!(world.provider.requests()[1].speed, Speed::Fast);
        front.close().await;
    });
}

#[test]
fn the_speed_can_be_changed_and_the_state_says_what_is_set_and_what_is_in_effect() {
    let world = World::new("speed_set", [says("One."), says("Two."), says("Three.")]);
    world.user_config(&format!("fast = \"{OTHER_MODEL}\"\n"));
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.speed, Speed::Standard);
        // Before anything was sent there is no cache to lose: no warning.
        let state = set(
            &front,
            SessionSettings {
                speed: Some(Speed::Fast),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(state.speed, Speed::Fast);
        assert_eq!(state.roles[&ModelRole::Main].speed_in_effect, Speed::Fast);
        assert_eq!(warned(&front), 0);
        // A utility role does not follow the session's speed.
        assert_eq!(state.roles[&ModelRole::Fast].speed, Speed::Standard);

        front.turn("one").await;
        assert_eq!(world.provider.requests()[0].speed, Speed::Fast);

        // Back to standard, with history: the user is told what it costs.
        let state = set(
            &front,
            SessionSettings {
                speed: Some(Speed::Standard),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(state.speed, Speed::Standard);
        assert_eq!(warned(&front), 1);
        front.turn("two").await;
        assert_eq!(world.provider.requests()[1].speed, Speed::Standard);

        // A role's own speed: the utility role's model has no fast tier,
        // so the state shows fast as set and standard as in effect.
        let state = set(
            &front,
            SessionSettings {
                roles: [(
                    ModelRole::Fast,
                    RoleSettings {
                        speed: Some(Speed::Fast),
                        ..RoleSettings::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        let utility = &state.roles[&ModelRole::Fast];
        assert_eq!(utility.speed, Speed::Fast);
        assert_eq!(utility.speed_in_effect, Speed::Standard);
        // The main role's own speed wins over the session's.
        let state = set(
            &front,
            role(RoleSettings {
                speed: Some(Speed::Fast),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(state.speed, Speed::Standard);
        assert_eq!(state.roles[&ModelRole::Main].speed, Speed::Fast);
        front.turn("three").await;
        assert_eq!(world.provider.requests()[2].speed, Speed::Fast);
        front.close().await;
    });
}

#[test]
fn a_roles_model_and_reasoning_level_can_be_changed_in_a_running_session() {
    let world = World::new(
        "model_set",
        [
            says("From the first."),
            says("From the second."),
            says("Third."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        assert_eq!(world.provider.requests()[0].model.info.id.as_str(), MODEL);
        assert_eq!(world.provider.requests()[0].reasoning.level, None);

        // Another model, and a level in the portable names.
        let state = set(
            &front,
            role(RoleSettings {
                model: Some(ModelId::new(OTHER_MODEL)),
                effort: Some("minimal".to_owned()),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap();
        let main = &state.roles[&ModelRole::Main];
        assert_eq!(main.model.as_str(), OTHER_MODEL);
        assert_eq!(main.effort.as_deref(), Some("minimal"));
        // Mapped onto the model's own scale, whose weakest level is `low`.
        assert_eq!(main.effort_in_effect.as_deref(), Some("low"));
        assert_eq!(
            warned(&front),
            1,
            "the change re-sends the context uncached"
        );

        front.turn("two").await;
        let request = &world.provider.requests()[1];
        assert_eq!(request.model.info.id.as_str(), OTHER_MODEL);
        assert_eq!(request.reasoning.level.as_deref(), Some("low"));
        // The conversation went with it, and the first model's message
        // still says who wrote it: what is replayed of its reasoning is the
        // provider layer's rule.
        assert_eq!(request.messages.len(), 3);
        assert_eq!(
            request.messages[1]
                .origin
                .as_ref()
                .map(|origin| origin.model.as_str()),
            Some(MODEL)
        );

        // The level alone: the model stays.
        let state = set(
            &front,
            role(RoleSettings {
                effort: Some("max".to_owned()),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap();
        let main = &state.roles[&ModelRole::Main];
        assert_eq!(main.model.as_str(), OTHER_MODEL);
        assert_eq!(main.effort_in_effect.as_deref(), Some("max"));
        front.turn("three").await;
        assert_eq!(
            world.provider.requests()[2].reasoning.level.as_deref(),
            Some("max")
        );
        front.close().await;
    });
}

/// A resumed session has history from its log before this process sends
/// anything, so its first change of model warns that the context goes out
/// again uncached. A new session with nothing in it does not warn: there
/// is nothing to re-send.
#[test]
fn a_resumed_sessions_first_change_of_model_warns_that_it_is_resent_uncached() {
    let world = World::new("model_set_resumed", [says("Before.")]);
    let core = world.core();
    world.run(async {
        let first = Front::start(&world, &core).await;
        // Nothing in it yet: no warning.
        set(
            &first,
            role(RoleSettings {
                model: Some(ModelId::new(OTHER_MODEL)),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(warned(&first), 0, "an empty session has nothing to re-send");
        first.turn("one").await;
        first.close().await;

        let client = core.connect(ClientInfo::user());
        let opened = core
            .handle(
                client.id,
                Op::SessionOpen(cineris_protocol::methods::SessionRef {
                    session_id: first.session.clone(),
                }),
            )
            .await
            .unwrap();
        let OpResult::SessionOpen(opened) = opened else {
            panic!("not the result of session/open");
        };
        let resumed = Front {
            session: first.session.clone(),
            state: opened.session,
            seen: std::cell::RefCell::default(),
            answered: std::cell::RefCell::default(),
            answering: std::cell::RefCell::default(),
            core: core.clone(),
            client,
        };
        assert_eq!(resumed.state.roles[&ModelRole::Main].model.as_str(), MODEL);
        set(
            &resumed,
            role(RoleSettings {
                model: Some(ModelId::new(OTHER_MODEL)),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            warned(&resumed),
            1,
            "the resumed conversation is re-sent uncached"
        );
        resumed.close().await;
    });
}

#[test]
fn a_model_or_a_level_that_does_not_exist_is_refused_and_changes_nothing() {
    let world = World::new("model_bad", [says("Still the first.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let refused = set(
            &front,
            role(RoleSettings {
                model: Some(ModelId::new("no-such-model")),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(refused.code, "unknown_model");
        let refused = set(
            &front,
            role(RoleSettings {
                effort: Some("ludicrous".to_owned()),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(refused.code, "unknown_effort");
        assert!(refused.message.contains("xhigh"), "{}", refused.message);
        front.turn("one").await;
        let request = &world.provider.requests()[0];
        assert_eq!(request.model.info.id.as_str(), MODEL);
        assert_eq!(request.reasoning.level, None);
        front.close().await;
    });
}

#[test]
fn the_level_can_be_lowered_by_whoever_drives_and_raised_only_by_a_user() {
    let world = World::new(
        "level_set",
        [
            calls(&[("c1", "write", json!({"path": "a.txt", "content": "x\n"}))]),
            says("Refused."),
            calls(&[("c2", "write", json!({"path": "a.txt", "content": "x\n"}))]),
            says("Written."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let supervisor = front
            .join(ClientInfo {
                role: ClientRole::Supervisor,
                ..ClientInfo::user()
            })
            .await;
        let level = |level: Level| SessionSettings {
            level: Some(level),
            ..SessionSettings::default()
        };
        // A supervisor lowers it.
        let state = set(&supervisor, level(Level::ReadOnly)).await.unwrap();
        assert_eq!(state.level, Level::ReadOnly);
        // The engine is at the new level: an edit is refused outright.
        let end = front.turn("write it").await;
        assert_eq!(end.outcome, TurnOutcome::Completed);
        assert_eq!(front.results()[0].1, "denied");
        assert_eq!(world.read("a.txt"), None);

        // It may not raise it again.
        let refused = set(&supervisor, level(Level::Supervised))
            .await
            .unwrap_err();
        assert_eq!(refused.code, "role_not_allowed");
        assert_eq!(refused.kind, cineris_protocol::ErrorKind::Permission);
        // A user may, and the edit then goes through.
        let state = set(&front, level(Level::Supervised)).await.unwrap();
        assert_eq!(state.level, Level::Supervised);
        assert_ne!(state.lease_id, front.state.lease_id, "a new standing lease");
        front.turn("write it now").await;
        assert_eq!(world.read("a.txt").as_deref(), Some("x\n"));

        // A level that cannot be held here is refused, and nothing changes:
        // there is no sandbox to confine an autonomous session.
        let refused = set(&front, level(Level::Autonomous)).await.unwrap_err();
        assert_eq!(refused.code, "sandbox_required");
        let refused = set(&front, level(Level::from_wire("godlike")))
            .await
            .unwrap_err();
        assert_eq!(refused.code, "unknown_value");

        // Each standing lease is in the log: granted, ended, granted, …
        let leases: Vec<(String, String)> = front
            .log(&world)
            .iter()
            .filter(|line| line["kind"] == "lease")
            .map(|line| {
                (
                    line["event"].as_str().unwrap().to_owned(),
                    line["lease"]["grant"]["level"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let expected: Vec<(String, String)> = [
            ("granted", "supervised"),
            ("ended", "supervised"),
            ("granted", "read-only"),
            ("ended", "read-only"),
            ("granted", "supervised"),
        ]
        .iter()
        .map(|(event, level)| ((*event).to_owned(), (*level).to_owned()))
        .collect();
        assert_eq!(leases, expected);
        front.close().await;
    });
}

/// `session/set`'s rule holds from the start. A client that is not a
/// `user` cannot start a session above the level it would start at,
/// whether through its settings or its configuration overrides, which rank
/// as the user's own command line; and its overrides cannot touch
/// permissions, the sandbox or limits at all, in a profile either. It may
/// start one lower, and set what the harness does not enforce. Found by the
/// review of step 13a.
#[test]
fn a_supervisor_cannot_start_a_session_above_the_level_it_could_set() {
    let world = World::new("supervisor_new", []);
    let core = world.core();
    let supervisor = ClientInfo {
        role: ClientRole::Supervisor,
        ..ClientInfo::user()
    };
    let at = |level: Level| SessionSettings {
        level: Some(level),
        ..SessionSettings::default()
    };
    let (for_start, workspace) = (core.clone(), world.workspace.clone());
    let start = move |info: ClientInfo, settings: SessionSettings, config: serde_json::Value| {
        let core = for_start.clone();
        let workspace = workspace.clone();
        async move {
            let client = core.connect(info);
            let started = core
                .handle(
                    client.id,
                    Op::SessionNew(cineris_protocol::methods::SessionNewParams {
                        cwd: None,
                        workspace: workspace.to_str().unwrap().to_owned(),
                        settings,
                        title: None,
                        config: config.as_object().unwrap().clone(),
                        ephemeral: true,
                    }),
                )
                .await;
            core.disconnect(client.id).await;
            match started {
                Ok(OpResult::SessionNew(result)) => Ok(result.session.level),
                Ok(_) => panic!("not the result of session/new"),
                Err(refused) => Err(refused),
            }
        }
    };
    world.run(async {
        // Above the level it would start at, by either way.
        for (settings, config) in [
            (at(Level::Unrestricted), json!({})),
            (
                SessionSettings::default(),
                json!({"permissions": {"mode": "unrestricted"}}),
            ),
        ] {
            let refused = start(supervisor.clone(), settings, config.clone())
                .await
                .expect_err("a supervisor started a session above its level");
            assert_eq!(refused.code, "role_not_allowed", "{config}");
            assert_eq!(refused.kind, cineris_protocol::ErrorKind::Permission);
        }
        // Nothing the harness enforces, whatever its value, nor a profile.
        for config in [
            json!({"permissions": {"mode": "read-only"}}),
            json!({"permissions": {"rules": [{"effect": "read", "match": "**", "action": "allow"}]}}),
            json!({"sandbox": {"write": ["/"]}}),
            json!({"limits": {"session": {"requests": 1}}}),
            json!({"profiles": {"mine": {"permissions": {"mode": "unrestricted"}}}}),
            // Nor what is the user's alone: where a provider's key is sent,
            // the program every command runs in, where sessions are kept
            // (and so what the engine protects), the HTTP client.
            json!({"providers": {"anthropic": {"base_url": "https://elsewhere.invalid"}}}),
            json!({"providers": {"anthropic": {"headers": {"x-leak": {"env": "HOME"}}}}}),
            json!({"shell": {"program": "/bin/sh"}}),
            json!({"session": {"data_dir": "/tmp/elsewhere"}}),
            json!({"http": {"proxy": "http://elsewhere.invalid"}}),
            json!({"credentials": {"store": "file"}}),
            json!({"no_such_section": {"key": 1}}),
            // Nor a wait or a time limit, which a project's file may only
            // shorten: an override ranks as the command line, past the rule
            // that holds a project to the value in effect, so a longer one
            // and a shorter one alike are the user's to give.
            json!({"retry": {"idle_timeout": "1h"}}),
            json!({"retry": {"max_wait": "1m"}}),
            json!({"shell": {"timeout": "1m"}}),
            json!({"model": {"speed_wait": "2m"}}),
        ] {
            let refused = start(supervisor.clone(), SessionSettings::default(), config.clone())
                .await
                .expect_err("a supervisor set what is the user's");
            assert_eq!(refused.code, "role_not_allowed", "{config}");
        }
        assert_eq!(core.open_sessions(), Vec::new());

        // Lower, a preference, and a model (as `session/set` would let it
        // choose), it may.
        let level = start(
            supervisor.clone(),
            at(Level::ReadOnly),
            json!({"model": {"speed_fallback": false, "main": OTHER_MODEL}}),
        )
        .await
        .unwrap();
        assert_eq!(level, Level::ReadOnly);
        // And at the level it would start at anyway.
        let level = start(supervisor.clone(), at(Level::Supervised), json!({}))
            .await
            .unwrap();
        assert_eq!(level, Level::Supervised);

        // A user starts where it likes, by either way.
        for (settings, config) in [
            (at(Level::Unrestricted), json!({})),
            (
                SessionSettings::default(),
                json!({"permissions": {"mode": "unrestricted"}}),
            ),
        ] {
            let level = start(ClientInfo::user(), settings, config).await.unwrap();
            assert_eq!(level, Level::Unrestricted);
        }
    });
}

/// A model of the user's catalog with two levels of its own.
const TWO_LEVELS: &str = "[models.small]
provider = \"anthropic\"
id = \"small-model\"
name = \"Small model\"
protocol = \"anthropic-messages\"
context_window = 100000
max_output = 8000
input = [\"text\"]

[models.small.reasoning]
levels = [\"low\", \"high\"]
default = \"low\"
";

/// One request that is refused in one part changes none of its parts.
#[test]
fn a_request_refused_in_one_part_changes_nothing() {
    let world = World::new("set_whole", [says("One.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        // A turn first, so that there is a log, and a cache to lose.
        front.turn("one").await;
        let before = set(&front, SessionSettings::default()).await.unwrap();
        assert_eq!(before.level, Level::Supervised);
        assert_eq!(before.speed, Speed::Standard);

        let refused = set(
            &front,
            SessionSettings {
                level: Some(Level::ReadOnly),
                speed: Some(Speed::Fast),
                roles: [
                    (
                        ModelRole::Fast,
                        RoleSettings {
                            model: Some(ModelId::new(OTHER_MODEL)),
                            speed: Some(Speed::Fast),
                            ..RoleSettings::default()
                        },
                    ),
                    (
                        ModelRole::Main,
                        RoleSettings {
                            speed: Some(Speed::Fast),
                            ..RoleSettings::default()
                        },
                    ),
                    // The part that is refused is the last one looked at.
                    (
                        ModelRole::Judge,
                        RoleSettings {
                            model: Some(ModelId::new("no-such-model")),
                            ..RoleSettings::default()
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(refused.code, "unknown_model");
        let after = set(&front, SessionSettings::default()).await.unwrap();
        assert_eq!(after, before, "a refused request changed the session");
        assert_eq!(warned(&front), 0);
        // And the log has no new lease.
        let leases = front
            .log(&world)
            .iter()
            .filter(|line| line["kind"] == "lease")
            .count();
        assert_eq!(leases, 1, "the level was changed before the refusal");
        front.close().await;
    });
}

/// A level carried from one model to the next that the new one cannot
/// take within its output limit (a budget model, whose smallest budget and
/// the answer's room do not fit) is left behind as an unknown level is, and
/// the user is told why.
#[test]
fn a_level_the_new_model_cannot_fit_in_its_output_is_left_behind_and_said() {
    let world = World::new("effort_too_large", [says("One.")]);
    world.raw_user_config(&format!(
        "[session]\ndurability = \"never\"\n\n[credentials]\nstore = \"file\"\n\n[model]\nmain \
         = \"{MODEL}\"\neffort = \"max\"\nmax_output = 1024\n\n[models.budget]\nprovider = \
         \"anthropic\"\nid = \"claude-budget\"\nname = \"Budget\"\nprotocol = \
         \"anthropic-messages\"\ncontext_window = 200000\nmax_output = 64000\ninput = \
         [\"text\"]\n\n[models.budget.reasoning]\nmin_tokens = 1024\nmax_tokens = 64000\n"
    ));
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let state = set(
            &front,
            role(RoleSettings {
                model: Some(ModelId::new("claude-budget")),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(state.roles[&ModelRole::Main].effort, None);
        front.drain();
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains("cannot reason at `max` and leave its answer room")),
            "{:?}",
            front.notices()
        );
        front.turn("one").await;
        let request = &world.provider.requests()[0];
        assert_eq!(request.model.info.id.as_str(), "claude-budget");
        assert_eq!(request.reasoning.budget_tokens, None);
        front.close().await;
    });
}

/// A level set for one model is that model's. When the model alone is
/// changed and the new one has no such level, it runs at its own default,
/// and the user is told; a level that is asked for and is not there is
/// refused.
#[test]
fn a_level_the_new_model_does_not_have_is_left_behind_and_said() {
    let world = World::new("effort_left", [says("One.")]);
    world.user_config(TWO_LEVELS);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let state = set(
            &front,
            role(RoleSettings {
                effort: Some("xhigh".to_owned()),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            state.roles[&ModelRole::Main].effort_in_effect.as_deref(),
            Some("xhigh")
        );

        // Asked for by name on the small model, it is refused.
        let refused = set(
            &front,
            role(RoleSettings {
                model: Some(ModelId::new("small-model")),
                effort: Some("xhigh".to_owned()),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(refused.code, "unknown_effort");

        // The model alone: nobody asked the small model for `xhigh`.
        let state = set(
            &front,
            role(RoleSettings {
                model: Some(ModelId::new("small-model")),
                ..RoleSettings::default()
            }),
        )
        .await
        .unwrap();
        let main = &state.roles[&ModelRole::Main];
        assert_eq!(main.model.as_str(), "small-model");
        assert_eq!(main.effort, None);
        front.drain();
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains("no reasoning level `xhigh`")),
            "{:?}",
            front.notices()
        );
        front.turn("one").await;
        let request = &world.provider.requests()[0];
        assert_eq!(request.model.info.id.as_str(), "small-model");
        assert_eq!(request.reasoning.level, None);
        front.close().await;
    });
}

/// A core that holds its sessions at a level at most (a harness that could
/// not mark its commands, since step 28's third pull request) refuses a
/// raise of a session's level past it by `session/set`, and says why; a
/// change within it is made.
#[test]
fn a_level_change_past_the_cores_cap_is_refused() {
    let world = World::new("level_cap", [says("Hi.")]);
    let core = world.core_with(|options| {
        options.allow_unrestricted = true;
        options.level_at_most = Some(Level::Supervised);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        let raised = set(
            &front,
            SessionSettings {
                level: Some(Level::Unrestricted),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(raised.code, "unrestricted_not_allowed");
        assert!(
            raised.message.contains("`supervised` at most"),
            "{}",
            raised.message
        );
        set(
            &front,
            SessionSettings {
                level: Some(Level::ReadOnly),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        front.close().await;
    });
}

/// Without `allow_unrestricted` (a server not started at `unrestricted`),
/// a session there is refused however it would come to be: its settings,
/// an override however spelled, a raise by `session/set`. With it (a
/// server whose own configuration is at `unrestricted`), its clients have
/// it, a fork included.
#[test]
fn unrestricted_is_refused_where_the_core_may_not_give_it() {
    let world = World::new("no_unrestricted", [says("Hi.")]);
    let core = world.core_with(|options| options.allow_unrestricted = false);
    world.run(async {
        let at_unrestricted = SessionSettings {
            level: Some(Level::Unrestricted),
            ..SessionSettings::default()
        };
        let refused = Front::start_as(&world, &core, ClientInfo::user(), at_unrestricted.clone())
            .await
            .err()
            .expect("a session at unrestricted");
        assert_eq!(refused.code, "unrestricted_not_allowed");
        assert_eq!(refused.kind, cineris_protocol::ErrorKind::Permission);
        let client = core.connect(ClientInfo::user());
        for (key, value) in [
            ("permissions . mode", json!("unrestricted")),
            ("\"permissions\".mode", json!("unrestricted")),
            ("permissions", json!({" mode": "unrestricted"})),
        ] {
            let mut config = serde_json::Map::new();
            config.insert(key.to_owned(), value);
            let refused = core
                .handle(
                    client.id,
                    Op::SessionNew(cineris_protocol::methods::SessionNewParams {
                        workspace: world.workspace.to_str().unwrap().to_owned(),
                        settings: SessionSettings::default(),
                        title: None,
                        config,
                        ephemeral: true,
                        cwd: None,
                    }),
                )
                .await
                .unwrap_err();
            assert_eq!(refused.code, "unrestricted_not_allowed", "{key}");
        }
        assert_eq!(core.open_sessions(), Vec::new(), "a refused session runs");

        let front = Front::start(&world, &core).await;
        let raised = set(&front, at_unrestricted.clone()).await.unwrap_err();
        assert_eq!(raised.code, "unrestricted_not_allowed");
        assert!(
            raised.message.contains("--permission-mode unrestricted"),
            "{}",
            raised.message
        );
        // The level is looked at before anything else of the request is
        // made ready: a role's model that would be refused too is never
        // looked for.
        let raised = set(
            &front,
            SessionSettings {
                roles: [(
                    ModelRole::Main,
                    RoleSettings {
                        model: Some(ModelId::new("no-such-model")),
                        ..RoleSettings::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..at_unrestricted
            },
        )
        .await
        .unwrap_err();
        assert_eq!(raised.code, "unrestricted_not_allowed");
        // Lowering is the user's as ever.
        let lowered = set(
            &front,
            SessionSettings {
                level: Some(Level::ReadOnly),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(lowered.level, Level::ReadOnly);
        front.close().await;
    });

    // A server whose own configuration is at `unrestricted`.
    world.user_config("[permissions]\nmode = \"unrestricted\"\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.level, Level::Unrestricted);
        front.turn("hello").await;
        let forked = front
            .op(Op::SessionFork(
                cineris_protocol::methods::SessionForkParams {
                    session_id: front.session.clone(),
                    at: None,
                    before: None,
                },
            ))
            .await
            .expect("the fork opens");
        let OpResult::SessionFork(forked) = forked else {
            panic!("not a session");
        };
        assert_eq!(forked.session.level, Level::Unrestricted);
        front.close().await;
    });
}

/// The session's level, as an empty `session/set` gives it.
async fn level_of(front: &Front) -> Level {
    set(front, SessionSettings::default())
        .await
        .expect("an empty session/set")
        .level
}

async fn reload(
    front: &Front,
    params: cineris_protocol::methods::ConfigReloadParams,
) -> cineris_protocol::methods::ConfigReloadResult {
    let OpResult::ConfigReload(result) = front
        .op(Op::ConfigReload(params))
        .await
        .expect("the reload answers")
    else {
        panic!("not the result of config/reload");
    };
    result
}

/// A `config/reload` reaches no level the core may not give, by any way
/// it comes: a profile of the user's at `unrestricted` chosen for the
/// session; the user's file changed to `unrestricted` while the server
/// runs, taken by the look at a turn's start, by a reload of the session,
/// and by a reload of every session of the client. Each is refused in the
/// reload's errors and the session keeps its level. Where the core may give
/// it, a profile and the look at a turn's start put the session there (the
/// review of step 27's second pull request: both reached `unrestricted`).
#[test]
fn a_reload_reaches_unrestricted_only_where_the_core_may_give_it() {
    use cineris_protocol::methods::{ConfigReloadParams, ConfigReloadResult};
    let refused = |result: &ConfigReloadResult| {
        result
            .errors
            .iter()
            .any(|error| error.contains("unrestricted"))
    };
    const PROFILE: &str = "[profiles.loose.permissions]\nmode = \"unrestricted\"\n";
    const FILE: &str = "[permissions]\nmode = \"unrestricted\"\n";

    // Where the core may not give it, one session tries every way.
    let world = World::new("reload_no_unrestricted", [says("Hi.")]);
    world.user_config(PROFILE);
    let core = world.core_with(|options| options.allow_unrestricted = false);
    world.run(async {
        let front = Front::start(&world, &core).await;
        let session = || Some(front.session.clone());
        let by_profile = reload(
            &front,
            ConfigReloadParams {
                session_id: session(),
                profile: Some("loose".to_owned()),
                ..ConfigReloadParams::default()
            },
        )
        .await;
        assert!(refused(&by_profile), "{by_profile:?}");
        assert_eq!(level_of(&front).await, Level::Supervised, "a profile");

        world.user_config(FILE);
        front.turn("hello").await;
        assert_eq!(level_of(&front).await, Level::Supervised, "a turn's look");
        let of_the_session = reload(
            &front,
            ConfigReloadParams {
                session_id: session(),
                ..ConfigReloadParams::default()
            },
        )
        .await;
        assert!(refused(&of_the_session), "{of_the_session:?}");
        assert_eq!(level_of(&front).await, Level::Supervised, "the session's");
        let of_every_session = reload(&front, ConfigReloadParams::default()).await;
        assert!(refused(&of_every_session), "{of_every_session:?}");
        assert_eq!(level_of(&front).await, Level::Supervised, "every session's");
        front.close().await;
    });

    // Where it may, as before.
    for by_profile in [true, false] {
        let world = World::new("reload_unrestricted", [says("Hi.")]);
        world.user_config(PROFILE);
        let core = world.core();
        world.run(async {
            let front = Front::start(&world, &core).await;
            if by_profile {
                let result = reload(
                    &front,
                    ConfigReloadParams {
                        session_id: Some(front.session.clone()),
                        profile: Some("loose".to_owned()),
                        ..ConfigReloadParams::default()
                    },
                )
                .await;
                assert!(result.errors.is_empty(), "{result:?}");
            } else {
                world.user_config(FILE);
                front.turn("hello").await;
            }
            assert_eq!(level_of(&front).await, Level::Unrestricted, "{by_profile}");
            front.close().await;
        });
    }
}

/// A stored session opened again (`session/open` of a session not
/// running) starts at the level the configuration comes to: refused where
/// the core may not give it, and nothing runs; opened there where it may.
#[test]
fn a_stored_session_opened_again_reaches_unrestricted_only_where_the_core_may_give_it() {
    let world = World::new("resume_unrestricted", [says("Hi.")]);
    let core = world.core();
    let session = world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hello").await;
        front.close().await;
        front.session.clone()
    });
    world.user_config("[permissions]\nmode = \"unrestricted\"\n");
    for allowed in [false, true] {
        let core = world.core_with(|options| options.allow_unrestricted = allowed);
        world.run(async {
            let client = core.connect(ClientInfo::user());
            let (opened, _) = crate::not_now(
                &core,
                &client,
                core.handle(
                    client.id,
                    Op::SessionOpen(cineris_protocol::methods::SessionRef {
                        session_id: session.clone(),
                    }),
                ),
            )
            .await;
            if allowed {
                let Ok(OpResult::SessionOpen(opened)) = opened else {
                    panic!("not a session: {opened:?}");
                };
                assert_eq!(opened.session.level, Level::Unrestricted);
                core.disconnect(client.id).await;
            } else {
                assert_eq!(
                    opened.map(|_| ()).unwrap_err().code,
                    "unrestricted_not_allowed"
                );
                assert_eq!(core.open_sessions(), Vec::new());
            }
        });
    }
}

/// A `session/fork` refused for a level the core may not give writes no
/// fork: the list of the workspace's sessions is as it was (the review of
/// step 27's second pull request found the fork's log left behind).
#[test]
fn a_refused_fork_leaves_no_stored_session() {
    let world = World::new("fork_refused", [says("Hi.")]);
    let core = world.core_with(|options| options.allow_unrestricted = false);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hello").await;
        let list = || async {
            let OpResult::SessionList(listed) = front
                .op(Op::SessionList(
                    cineris_protocol::methods::SessionListParams {
                        workspace: Some(world.workspace.to_str().unwrap().to_owned()),
                        limit: None,
                        cursor: None,
                    },
                ))
                .await
                .unwrap()
            else {
                panic!("not a list");
            };
            listed
                .sessions
                .into_iter()
                .map(|summary| summary.session_id)
                .collect::<Vec<_>>()
        };
        let before = list().await;
        // The user's file now says `unrestricted`, which this core may not
        // give: a fork would open there.
        world.user_config("[permissions]\nmode = \"unrestricted\"\n");
        let refused = front
            .op(Op::SessionFork(
                cineris_protocol::methods::SessionForkParams {
                    session_id: front.session.clone(),
                    at: None,
                    before: None,
                },
            ))
            .await
            .unwrap_err();
        assert_eq!(refused.code, "unrestricted_not_allowed");
        assert_eq!(list().await, before, "a refused fork left its session");
        assert_eq!(world.logs().len(), 1, "a refused fork left its log");
        front.close().await;
    });
}
