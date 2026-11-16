//! `config/reload` through the core (step 24, `docs/steps/24/answered.md`,
//! "The design, before building", point 1;
//! `docs/architecture/08-configuration.md`, "Reloading without a restart").

use std::sync::Arc;

use cortex_base::{PathOptions, PathResolver};
use cortex_policy::{Definition, TrustStore};
use cortex_protocol::event::{ConfigChanged, Event};
use cortex_protocol::lease::{Effect, Level, Rule, RuleAction};
use cortex_protocol::methods::{
    ConfigGetParams, ConfigGetResult, ConfigLayer, ConfigReloadParams, ConfigReloadResult, Op,
    OpResult, SessionSetParams,
};
use cortex_protocol::session::{SessionSettings, SessionState, TrustStanding};
use cortex_protocol::{ErrorKind, SessionEvent};
use serde_json::Value;

use crate::{Front, World, says};

async fn reload(front: &Front, params: ConfigReloadParams) -> ConfigReloadResult {
    let OpResult::ConfigReload(result) = front
        .op(Op::ConfigReload(params))
        .await
        .expect("the reload answers")
    else {
        panic!("not the result of config/reload");
    };
    result
}

async fn set(
    front: &Front,
    settings: SessionSettings,
) -> Result<SessionState, cortex_core::OpError> {
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

/// The session's state now, through a `session/set` that sets nothing.
async fn state_of(front: &Front) -> SessionState {
    set(front, SessionSettings::default()).await.unwrap()
}

async fn get(front: &Front, key: &str) -> ConfigGetResult {
    let OpResult::ConfigGet(result) = front
        .op(Op::ConfigGet(ConfigGetParams {
            session_id: Some(front.session.clone()),
            keys: vec![key.to_owned()],
        }))
        .await
        .expect("config/get answers")
    else {
        panic!("not the result of config/get");
    };
    result
}

async fn value(front: &Front, key: &str) -> Option<Value> {
    get(front, key).await.config.get(key).cloned()
}

/// The `config.changed` events the front end was sent.
fn changes(front: &Front) -> Vec<ConfigChanged> {
    front.drain();
    front
        .events()
        .into_iter()
        .filter_map(|event| match event {
            SessionEvent {
                event: Event::ConfigChanged(changed),
                ..
            } => Some(changed),
            _ => None,
        })
        .collect()
}

fn trust_store(world: &World) -> (TrustStore, cortex_protocol::ProjectKey) {
    let state = world.dirs().state.unwrap();
    std::fs::create_dir_all(&state).unwrap();
    let store = TrustStore::new(state.join("trust.toml"), Arc::new(world.clock.clone()));
    let key = PathResolver::new(&world.workspace, PathOptions::new())
        .unwrap()
        .root()
        .project_key();
    (store, key)
}

fn allow_github() -> Rule {
    Rule {
        effect: Effect::Write,
        pattern: ".github/**".to_owned(),
        action: RuleAction::Allow,
    }
}

const ALLOW_GITHUB: &str =
    "[[permissions.rules]]\neffect = \"write\"\nmatch = \".github/**\"\naction = \"allow\"\n";
const ALLOW_CURL: &str =
    "[[permissions.rules]]\neffect = \"exec\"\nmatch = \"curl *\"\naction = \"allow\"\n";

/// A change to the user's file: the key that applies at once is in force,
/// each key that waits is listed with what it waits for, and every client
/// of the session is told.
#[test]
fn a_reload_applies_what_takes_effect_at_once_and_lists_what_waits() {
    let world = World::new("reload_applies", [says("Done.")]);
    world.user_config("[retry]\nattempts = 3\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(value(&front, "retry.attempts").await, Some(3.into()));
        world.user_config(
            "[retry]\nattempts = 2\n\n[context]\nkeep_recent_tokens = 100\n\n[shell]\ntimeout = \
             \"60s\"\n",
        );
        let result = reload(&front, ConfigReloadParams::default()).await;
        assert!(result.errors.is_empty(), "{result:?}");
        assert!(
            result.applied.contains(&"retry.attempts".to_owned()),
            "{result:?}"
        );
        for waits in ["context.keep_recent_tokens", "shell.timeout"] {
            assert!(
                result.pending.contains(&waits.to_owned()),
                "{waits}: {result:?}"
            );
            assert!(
                !result.applied.contains(&waits.to_owned()),
                "{waits}: {result:?}"
            );
        }
        let said = result.notes.join("\n");
        assert!(said.contains("next context epoch"), "{said}");
        assert!(said.contains("sessions opened from now on"), "{said}");
        assert_eq!(value(&front, "retry.attempts").await, Some(2.into()));
        // What waits is not in force.
        assert_eq!(
            value(&front, "context.keep_recent_tokens").await,
            Some(0.into())
        );
        let told = changes(&front);
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].applied.contains(&"retry.attempts".to_owned()));

        // Nothing changed: no event, and the person who asked is told so.
        let again = reload(&front, ConfigReloadParams::default()).await;
        assert!(
            again.applied.is_empty() && again.errors.is_empty(),
            "{again:?}"
        );
        assert!(again.notes.iter().any(|note| note == "Nothing changed."));
        assert_eq!(changes(&front).len(), 1);
        front.close().await;
    });
}

/// A file that does not load leaves the configuration in force, and is
/// said once: a look that finds the same broken file again says nothing,
/// and `/reload` says it again.
#[test]
fn a_broken_file_keeps_the_configuration_and_is_said_once() {
    let world = World::new("reload_broken", [says("Done.")]);
    world.user_config("[retry]\nattempts = 3\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        world.user_config("[retry]\nattempts = \"many\"\n");
        let first = reload(
            &front,
            ConfigReloadParams {
                changed_only: true,
                ..ConfigReloadParams::default()
            },
        )
        .await;
        assert_eq!(first.errors.len(), 1, "{first:?}");
        assert_eq!(value(&front, "retry.attempts").await, Some(3.into()));
        let second = reload(
            &front,
            ConfigReloadParams {
                changed_only: true,
                ..ConfigReloadParams::default()
            },
        )
        .await;
        assert!(second.errors.is_empty(), "said twice: {second:?}");
        let asked = reload(&front, ConfigReloadParams::default()).await;
        assert_eq!(asked.errors, first.errors, "the person asked: said again");
        front.close().await;
    });
}

/// A trusted project whose file gains an allow rule: the rule is never in
/// force, its rules are set aside until the change is approved, and the
/// session's standing says so.
#[test]
fn an_allow_rule_added_while_trusted_is_not_in_force() {
    let world = World::new("reload_new_rule", [says("Done.")]);
    let (store, key) = trust_store(&world);
    store
        .trust(
            &key,
            "~/work/app",
            &[Definition::of_rules(&[allow_github()])],
        )
        .unwrap();
    world.project_config(ALLOW_GITHUB);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let rules = value(&front, "permissions.rules").await.unwrap();
        assert!(rules.to_string().contains(".github/**"), "{rules}");
        world.project_config(&format!("{ALLOW_GITHUB}\n{ALLOW_CURL}"));
        let result = reload(&front, ConfigReloadParams::default()).await;
        let rules = value(&front, "permissions.rules")
            .await
            .unwrap_or(Value::Null)
            .to_string();
        assert!(!rules.contains("curl"), "the new rule is in force: {rules}");
        assert!(
            !rules.contains(".github/**"),
            "the pinned rules stay: {rules}"
        );
        assert!(
            result
                .notes
                .iter()
                .any(|note| note.contains("rules have changed")),
            "{result:?}"
        );
        let state = state_of(&front).await;
        assert_eq!(state.trust, Some(TrustStanding::RulesChanged));
        front.close().await;
    });
}

/// Trust taken away from outside (`cortex permissions untrust` in another
/// terminal): the next look that only asks whether anything changed sees
/// the trust store change, and what trust gave goes at once: a project's
/// dearer number of retries, a raised cost, waits for trust again.
#[test]
fn trust_taken_away_from_outside_is_seen_by_the_next_look() {
    let world = World::new("reload_untrust", [says("Done.")]);
    let (store, key) = trust_store(&world);
    store.trust(&key, "~/work/app", &[]).unwrap();
    world.project_config("[retry]\nattempts = 9\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(value(&front, "retry.attempts").await, Some(9.into()));
        assert!(store.forget(&key).unwrap());
        let result = reload(
            &front,
            ConfigReloadParams {
                changed_only: true,
                ..ConfigReloadParams::default()
            },
        )
        .await;
        assert!(
            result.applied.contains(&"retry.attempts".to_owned()),
            "{result:?}"
        );
        assert_ne!(value(&front, "retry.attempts").await, Some(9.into()));
        let state = state_of(&front).await;
        assert_eq!(state.trust, Some(TrustStanding::Untrusted));
        front.close().await;
    });
}

/// A project's file cannot raise a key only the user may set, nor loosen
/// one that may only be tightened, by a reload any more than at a start.
#[test]
fn a_reload_keeps_a_projects_file_to_what_it_may_set() {
    let world = World::new("reload_classes", [says("Done.")]);
    world.user_config("[limits.turn]\ncost = 1.0\n");
    let core = world.core_with(|options| options.trust_project = true);
    world.run(async {
        let front = Front::start(&world, &core).await;
        world.project_config(
            "[usage]\nrefresh = [\"openrouter\"]\n\n[limits.turn]\ncost = 5.0\n\n[limits.session]\n\
             cost = 0.5\n",
        );
        let result = reload(&front, ConfigReloadParams::default()).await;
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(
            value(&front, "usage.refresh").await,
            Some(Value::Array(Vec::new()))
        );
        assert_eq!(value(&front, "limits.turn.cost").await, Some(1.0.into()));
        // Tightening is anyone's: the session's limit is the project's.
        assert_eq!(value(&front, "limits.session.cost").await, Some(0.5.into()));
        front.close().await;
    });
}

/// What the person chose in the session outranks the files: a level set
/// with `session/set` stays through a reload that changes the file's.
#[test]
fn a_choice_made_in_the_session_outranks_the_files() {
    let world = World::new("reload_choice", [says("Done.")]);
    world.user_config("");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        set(
            &front,
            SessionSettings {
                level: Some(Level::ReadOnly),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        world.user_config("[permissions]\nmode = \"supervised\"\n\n[retry]\nattempts = 2\n");
        let result = reload(&front, ConfigReloadParams::default()).await;
        assert!(
            !result.applied.contains(&"permissions.mode".to_owned()),
            "{result:?}"
        );
        assert_eq!(state_of(&front).await.level, Level::ReadOnly);
        front.close().await;
    });
}

/// A profile is for one session, and only a person chooses it.
#[test]
fn a_profile_is_chosen_for_one_session_by_a_person() {
    let world = World::new("reload_profile", [says("Done.")]);
    world.user_config("[profiles.careful.retry]\nattempts = 1\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let refused = front
            .op(Op::ConfigReload(ConfigReloadParams {
                profile: Some("careful".to_owned()),
                ..ConfigReloadParams::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(refused.code, "profile_needs_session");
        let supervisor = front
            .join(cortex_core::ClientInfo {
                role: cortex_protocol::methods::ClientRole::Supervisor,
                ..cortex_core::ClientInfo::user()
            })
            .await;
        let refused = supervisor
            .op(Op::ConfigReload(ConfigReloadParams {
                session_id: Some(front.session.clone()),
                profile: Some("careful".to_owned()),
                ..ConfigReloadParams::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(refused.kind, ErrorKind::Permission);
        let result = reload(
            &front,
            ConfigReloadParams {
                session_id: Some(front.session.clone()),
                profile: Some("careful".to_owned()),
                ..ConfigReloadParams::default()
            },
        )
        .await;
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(value(&front, "retry.attempts").await, Some(1.into()));
        front.close().await;
    });
}

/// `config/get` says where each value came from: the user's file and its
/// line, and nothing for a built-in default.
#[test]
fn config_get_says_where_each_value_came_from() {
    let world = World::new("reload_origins", [says("Done.")]);
    world.user_config("[retry]\nattempts = 3\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let result = get(&front, "retry").await;
        let origin = &result.origins["retry.attempts"];
        assert_eq!(origin.layer, ConfigLayer::User);
        assert!(
            origin
                .file
                .as_deref()
                .is_some_and(|file| file.ends_with("config.toml")),
            "{origin:?}"
        );
        assert!(origin.line.is_some(), "{origin:?}");
        assert!(!result.origins.contains_key("retry.backoff_factor"));
        front.close().await;
    });
}

/// The start of a turn is a moment the files are looked at: a change made
/// between two turns is in force for the second, and told before it.
#[test]
fn a_turn_starts_with_the_files_looked_at() {
    let world = World::new("reload_turn", [says("One."), says("Two.")]);
    world.user_config("[retry]\nattempts = 3\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        world.user_config("[retry]\nattempts = 2\n");
        front.turn("two").await;
        let told = changes(&front);
        assert!(
            told.iter()
                .any(|changed| changed.applied.contains(&"retry.attempts".to_owned())),
            "{told:?}"
        );
        assert_eq!(value(&front, "retry.attempts").await, Some(2.into()));
        front.close().await;
    });
}

/// What waits for the next epoch comes into force when a compaction
/// begins one, and the clients are told then.
#[test]
fn what_waits_for_the_next_epoch_comes_in_with_a_compaction() {
    let world = World::new("reload_epoch", [says("One."), says("Goal: the summary.")]);
    world.user_config("");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        world.user_config("[context]\nkeep_recent_tokens = 100\n");
        let result = reload(&front, ConfigReloadParams::default()).await;
        assert!(
            result
                .pending
                .contains(&"context.keep_recent_tokens".to_owned()),
            "{result:?}"
        );
        front
            .op(Op::SessionCompact(
                cortex_protocol::methods::SessionCompactParams {
                    session_id: front.session.clone(),
                    focus: None,
                },
            ))
            .await
            .unwrap();
        front.turns(2).await;
        let told = changes(&front);
        assert!(
            told.iter().any(|changed| changed
                .applied
                .contains(&"context.keep_recent_tokens".to_owned())),
            "{told:?}"
        );
        assert_eq!(
            value(&front, "context.keep_recent_tokens").await,
            Some(100.into())
        );
        front.close().await;
    });
}

/// A session's stored artifact is read back whole (step 24's
/// `session/artifact`, for a check's whole output in the viewer), and one
/// the session does not have is refused.
#[test]
fn an_artifact_is_read_back_whole_and_one_not_there_is_refused() {
    let world = World::new(
        "reads_artifact",
        [
            crate::calls(&[("c1", "read", serde_json::json!({"path": "pixel.png"}))]),
            says("A pixel."),
        ],
    );
    world.put("pixel.png", crate::provider::PIXEL);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("look").await;
        let read = |artifact: &str| {
            Op::SessionArtifact(cortex_protocol::methods::SessionArtifactParams {
                session_id: front.session.clone(),
                artifact: cortex_protocol::ArtifactId::from(artifact),
            })
        };
        let OpResult::SessionArtifact(result) = front.op(read("a_0001")).await.unwrap() else {
            panic!("not the result of session/artifact");
        };
        assert_eq!(result.bytes, crate::provider::PIXEL.len() as u64);
        assert!(!result.cut);
        let refused = front.op(read("a_9999")).await.unwrap_err();
        assert_eq!(refused.code, "unknown_artifact");
        // Reading ahead answers at once, whatever it is given.
        let warmed = front
            .op(Op::WorkspaceWarm(
                cortex_protocol::methods::WorkspaceWarmParams {
                    session_id: front.session.clone(),
                    paths: vec!["pixel.png".to_owned(), "../elsewhere".to_owned()],
                },
            ))
            .await
            .unwrap();
        assert!(matches!(warmed, OpResult::WorkspaceWarm(_)));
        front.close().await;
    });
}

/// The reload judges trust first and loads with what it judged: held
/// between the two, nothing of the project's new rules is in force, and the
/// trust store changed there (the new rules approved) does not reach the
/// load already judged; the next look sees the store's change and loads
/// with it (the review of step 24's design: trust judged first).
#[test]
fn the_load_uses_the_trust_just_judged() {
    let world = World::new("reload_judged_first", [says("Done.")]);
    let (store, key) = trust_store(&world);
    store
        .trust(
            &key,
            "~/work/app",
            &[Definition::of_rules(&[allow_github()])],
        )
        .unwrap();
    world.project_config(ALLOW_GITHUB);
    let core = world.core();
    let judged = cortex_testkit::Hold::new(cortex_core::TRUST_JUDGED);
    world.run(async {
        let front = Front::start(&world, &core).await;
        world.project_config(&format!("{ALLOW_GITHUB}\n{ALLOW_CURL}"));
        // The session's own reload, held as it has judged trust.
        let mut reloading = std::pin::pin!(reload(
            &front,
            ConfigReloadParams {
                session_id: Some(front.session.clone()),
                ..ConfigReloadParams::default()
            },
        ));
        judged.drive_until_reached(reloading.as_mut()).await;
        // Judged, not loaded: the new rule is nowhere in force.
        let rules = value(&front, "permissions.rules")
            .await
            .unwrap_or(Value::Null)
            .to_string();
        assert!(!rules.contains("curl"), "in force before the load: {rules}");
        // The new rules approved while the reload is held.
        let curl = Rule {
            effect: Effect::Exec,
            pattern: "curl *".to_owned(),
            action: RuleAction::Allow,
        };
        store
            .approve(&key, &[Definition::of_rules(&[allow_github(), curl])])
            .unwrap();
        judged.release();
        let result = reloading.await;
        assert!(
            result
                .notes
                .iter()
                .any(|note| note.contains("rules have changed")),
            "{result:?}"
        );
        let rules = value(&front, "permissions.rules")
            .await
            .unwrap_or(Value::Null)
            .to_string();
        assert!(
            !rules.contains("curl"),
            "the load did not use the trust it judged: {rules}"
        );
        // The next look sees the store's change, judges again, and loads
        // with that.
        let next = reload(
            &front,
            ConfigReloadParams {
                changed_only: true,
                ..ConfigReloadParams::default()
            },
        )
        .await;
        assert!(next.errors.is_empty(), "{next:?}");
        let rules = value(&front, "permissions.rules")
            .await
            .unwrap_or(Value::Null)
            .to_string();
        assert!(rules.contains("curl"), "the approval was not seen: {rules}");
        assert_eq!(state_of(&front).await.trust, Some(TrustStanding::Trusted));
        front.close().await;
    });
}

/// A choice made in the session survives a reload that changes the file's
/// value of the same key, and a reload that rebuilds every role's model
/// (a change of `retry`) builds the chosen model, not the file's (the review
/// of step 24's first part, finding 1).
#[test]
fn a_reload_never_undoes_a_model_or_a_level_chosen_in_the_session() {
    let world = World::new("reload_choices_kept", [says("One."), says("Two.")]);
    world.user_config("");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        set(
            &front,
            SessionSettings {
                level: Some(Level::ReadOnly),
                roles: [(
                    cortex_protocol::model::ModelRole::Main,
                    cortex_protocol::session::RoleSettings {
                        model: Some(cortex_protocol::ModelId::new(crate::OTHER_MODEL)),
                        ..cortex_protocol::session::RoleSettings::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        // The file asks for another level, and a change that rebuilds every
        // role's model.
        world.user_config("[permissions]\nmode = \"unrestricted\"\n\n[retry]\nattempts = 2\n");
        let result = reload(&front, ConfigReloadParams::default()).await;
        assert!(result.errors.is_empty(), "{result:?}");
        assert!(
            !result.applied.contains(&"permissions.mode".to_owned()),
            "{result:?}"
        );
        assert!(
            !result.pending.contains(&"model.main".to_owned()),
            "{result:?}"
        );
        let state = state_of(&front).await;
        assert_eq!(state.level, Level::ReadOnly);
        assert_eq!(
            state.roles[&cortex_protocol::model::ModelRole::Main]
                .model
                .as_str(),
            crate::OTHER_MODEL,
            "the chosen model was rebuilt from the file's"
        );
        front.close().await;
    });
}

/// Trust taken away while a project file is broken is taken away all the
/// same, from the first look of the session: what trust gave goes (finding
/// 2).
#[test]
fn trust_taken_away_with_a_broken_file_is_taken_away() {
    let world = World::new("reload_untrust_broken", [says("Done.")]);
    let (store, key) = trust_store(&world);
    store.trust(&key, "~/work/app", &[]).unwrap();
    world.project_config("[retry]\nattempts = 9\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(value(&front, "retry.attempts").await, Some(9.into()));
        world.project_config("[retry\nattempts = 9\n");
        assert!(store.forget(&key).unwrap());
        let result = reload(
            &front,
            ConfigReloadParams {
                changed_only: true,
                ..ConfigReloadParams::default()
            },
        )
        .await;
        assert!(
            !result.errors.is_empty(),
            "the broken file was not said: {result:?}"
        );
        assert_ne!(
            value(&front, "retry.attempts").await,
            Some(9.into()),
            "what trust gave stayed: {result:?}"
        );
        assert_eq!(state_of(&front).await.trust, Some(TrustStanding::Untrusted));
        front.close().await;
    });
}

/// Trust taken away is taken away even when another part of the reload
/// cannot be built (the user's own model, which the project's trusted one
/// stood in for, is not one the catalog has): the rest waits, and says so
/// (finding 3).
#[test]
fn trust_taken_away_holds_when_another_part_of_the_reload_fails() {
    let world = World::new("reload_untrust_partial", [says("Done.")]);
    let (store, key) = trust_store(&world);
    store.trust(&key, "~/work/app", &[]).unwrap();
    world.raw_user_config(
        "[session]\ndurability = \"never\"\n\n[credentials]\nstore = \"file\"\n\n[model]\nmain = \
         \"no-such-model\"\n",
    );
    world.project_config(&format!("[model]\nmain = \"{}\"\n", crate::MODEL));
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert!(store.forget(&key).unwrap());
        let result = reload(&front, ConfigReloadParams::default()).await;
        assert!(!result.errors.is_empty(), "{result:?}");
        assert_eq!(
            state_of(&front).await.trust,
            Some(TrustStanding::Untrusted),
            "trust stayed because the model could not be built: {result:?}"
        );
        front.close().await;
    });
}

/// A limit a reload adds is kept by the next reload that changes another:
/// the day's account stays in the ledger (finding 4).
#[test]
fn a_limit_a_reload_added_stays_through_the_next() {
    let world = World::new(
        "reload_day_kept",
        [
            crate::calls(&[("c1", "read", serde_json::json!({"path": "a.txt"}))]),
            crate::calls(&[("c2", "read", serde_json::json!({"path": "a.txt"}))]),
            says("Unreached."),
        ],
    );
    world.put("a.txt", "one\n");
    world.user_config("");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        world.user_config("[limits.day]\nrequests = 2\n");
        let added = reload(&front, ConfigReloadParams::default()).await;
        assert!(added.errors.is_empty(), "{added:?}");
        world.user_config("[limits.day]\nrequests = 2\n\n[limits.session]\nrequests = 50\n");
        let next = reload(&front, ConfigReloadParams::default()).await;
        assert!(next.errors.is_empty(), "{next:?}");
        let end = front.turn("go").await;
        assert_eq!(
            end.outcome,
            cortex_protocol::event::TurnOutcome::Limit,
            "the day's limit was dropped: {:?}",
            end.error
        );
        front.close().await;
    });
}

/// Two reloads of one session never interleave: one held between judging
/// and loading holds the next before it looks at anything (finding 7).
#[test]
fn two_reloads_of_a_session_never_interleave() {
    let world = World::new("reload_one_at_a_time", [says("Done.")]);
    let (store, key) = trust_store(&world);
    store.trust(&key, "~/work/app", &[]).unwrap();
    world.project_config("[retry]\nattempts = 9\n");
    let core = world.core();
    let judged = cortex_testkit::Hold::new(cortex_core::TRUST_JUDGED);
    world.run(async {
        let front = Front::start(&world, &core).await;
        let params = || ConfigReloadParams {
            session_id: Some(front.session.clone()),
            ..ConfigReloadParams::default()
        };
        let mut first = std::pin::pin!(reload(&front, params()));
        judged.drive_until_reached(first.as_mut()).await;
        // Trust is taken away while the first is held, judged trusted.
        assert!(store.forget(&key).unwrap());
        let mut second = std::pin::pin!(reload(&front, params()));
        assert!(crate::poll_once(second.as_mut()).await.is_pending());
        crate::settle().await;
        assert!(crate::poll_once(second.as_mut()).await.is_pending());
        assert_eq!(
            judged.arrivals(),
            1,
            "the second looked while the first was held"
        );
        judged.release();
        first.await;
        second.await;
        // The second, which judged after the store changed, committed last.
        assert_eq!(state_of(&front).await.trust, Some(TrustStanding::Untrusted));
        assert_ne!(value(&front, "retry.attempts").await, Some(9.into()));
        front.close().await;
    });
}

/// A profile whose reload cannot be applied leaves the session's own
/// choices as they were (finding 8).
#[test]
fn a_profile_that_cannot_be_applied_keeps_the_sessions_choices() {
    let world = World::new("reload_profile_fails", [says("Done.")]);
    world.user_config("[stats]\nenabled = false\n\n[profiles.daily.limits.day]\nrequests = 5\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        set(
            &front,
            SessionSettings {
                level: Some(Level::ReadOnly),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        let result = reload(
            &front,
            ConfigReloadParams {
                session_id: Some(front.session.clone()),
                profile: Some("daily".to_owned()),
                ..ConfigReloadParams::default()
            },
        )
        .await;
        assert!(!result.errors.is_empty(), "{result:?}");
        // A later change of the file reloads without the profile, with the
        // choice.
        world.user_config(
            "[stats]\nenabled = false\n\n[profiles.daily.limits.day]\nrequests = 5\n\n\
             [permissions]\nmode = \"unrestricted\"\n",
        );
        let later = reload(&front, ConfigReloadParams::default()).await;
        assert!(later.errors.is_empty(), "{later:?}");
        assert_eq!(state_of(&front).await.level, Level::ReadOnly);
        front.close().await;
    });
}

/// A file written between the judgement and the load is checked against
/// the pins on what was loaded: a new allow rule written there is not in
/// force, and the project's rules are set aside (finding 5).
#[test]
fn a_rule_written_between_the_judgement_and_the_load_is_not_in_force() {
    let world = World::new("reload_written_between", [says("Done.")]);
    let (store, key) = trust_store(&world);
    store
        .trust(
            &key,
            "~/work/app",
            &[Definition::of_rules(&[allow_github()])],
        )
        .unwrap();
    world.project_config(ALLOW_GITHUB);
    let core = world.core();
    let judged = cortex_testkit::Hold::new(cortex_core::TRUST_JUDGED);
    world.run(async {
        let front = Front::start(&world, &core).await;
        world.user_config("[retry]\nattempts = 4\n");
        let mut reloading = std::pin::pin!(reload(
            &front,
            ConfigReloadParams {
                session_id: Some(front.session.clone()),
                ..ConfigReloadParams::default()
            },
        ));
        judged.drive_until_reached(reloading.as_mut()).await;
        // Judged trusted, the rules as pinned; then the file gains a rule.
        world.project_config(&format!("{ALLOW_GITHUB}\n{ALLOW_CURL}"));
        judged.release();
        let result = reloading.await;
        let rules = value(&front, "permissions.rules")
            .await
            .unwrap_or(Value::Null)
            .to_string();
        assert!(
            !rules.contains("curl"),
            "loaded trusted unchecked: {rules} {result:?}"
        );
        assert_eq!(
            state_of(&front).await.trust,
            Some(TrustStanding::RulesChanged)
        );
        front.close().await;
    });
}
