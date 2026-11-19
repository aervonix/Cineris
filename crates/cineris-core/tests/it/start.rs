//! What a core and a session start with, and what stops them starting
//! (`docs/architecture/08-configuration.md`,
//! `docs/architecture/06-permissions-and-sandbox.md`).

use cineris_core::{ClientInfo, Core};
use cineris_protocol::ErrorKind;
use cineris_protocol::event::{Event, Notice};
use cineris_protocol::methods::{Op, OpResult};
use cineris_protocol::model::ModelRole;
use cineris_protocol::session::SessionSettings;
use serde_json::json;

use crate::{Front, MODEL, World, calls, says};

const LOCAL_MODEL: &str = "[models.local]
provider = \"local\"
id = \"house-model\"
name = \"House model\"
protocol = \"anthropic-messages\"
context_window = 100000
max_output = 8000
input = [\"text\"]

[providers.local]
protocol = \"anthropic-messages\"
base_url = \"http://127.0.0.1:9\"
";

fn start_error(world: &World) -> Box<cineris_protocol::ErrorData> {
    match Core::start(world.options()) {
        Ok(_) => panic!("the core started"),
        Err(error) => error,
    }
}

async fn session_error(world: &World, core: &Core) -> Box<cineris_protocol::ErrorData> {
    match Front::start_as(world, core, ClientInfo::user(), SessionSettings::default()).await {
        Ok(_) => panic!("the session started"),
        Err(error) => error,
    }
}

#[test]
fn an_invalid_entry_of_the_users_catalog_stops_the_core_and_says_where_and_what() {
    let world = World::new("catalog_bad", []);
    // Its output limit is larger than its context window.
    world.user_config(&LOCAL_MODEL.replace("max_output = 8000", "max_output = 800000"));
    let error = start_error(&world);
    assert_eq!(error.kind, ErrorKind::Config);
    assert_eq!(error.code, "invalid_catalog");
    let message = &error.message;
    // The file and the line, the entry, and what is wrong with it.
    assert!(
        message.contains("~/.config/cineris/config.toml:")
            || message.contains("cineris/config.toml:")
            || message.contains("cineris\\config.toml:"),
        "{message}"
    );
    assert!(message.contains("[models.local]"), "{message}");
    assert!(message.contains("local/house-model"), "{message}");
    assert!(message.contains("max_output"), "{message}");
    assert!(message.contains("not started"), "{message}");

    // A key the catalog does not know is refused the same way, by name.
    world.user_config(&LOCAL_MODEL.replace("input = [\"text\"]", "inputs = [\"text\"]"));
    let error = start_error(&world);
    assert_eq!(error.code, "invalid_catalog");
    assert!(error.message.contains("inputs"), "{}", error.message);
    assert!(
        error.message.contains("[models.local]"),
        "{}",
        error.message
    );

    // Something that is not an entry at all.
    world.user_config("[models]\nlocal = \"house-model\"\n");
    let error = start_error(&world);
    assert_eq!(error.code, "invalid_catalog");
    assert!(
        error.message.contains("[models.local]"),
        "{}",
        error.message
    );
}

/// The model the first run writes for a provider is the one the catalog
/// marks as its default, not the first it lists (Anthropic's dearest); a
/// user's entry marked so takes its provider's; a provider with none
/// marked has none; and a model is shared when another provider has one of
/// its id (the review of the fixes of step 25's third pull request).
#[test]
fn the_first_runs_model_for_a_provider_is_the_one_the_catalog_marks() {
    let world = World::new("first_run_model", []);
    world.raw_user_config(LOCAL_MODEL);
    let core = world.core();
    let first = cineris_llm::Catalog::builtin()
        .unwrap()
        .models()
        .iter()
        .find(|entry| entry.info.provider.as_str() == "anthropic")
        .map(|entry| entry.info.id.as_str().to_owned());
    assert_eq!(
        core.default_model("anthropic").as_deref(),
        Some("claude-sonnet-5-5")
    );
    assert_ne!(
        core.default_model("anthropic"),
        first,
        "not the first listed"
    );
    assert_eq!(core.default_model("local"), None, "none is marked");
    assert_eq!(core.default_model("nobody"), None);
    assert!(!core.model_shared("claude-sonnet-5-5"));
    assert!(!core.model_shared("house-model"));

    // The user's own entries: one marked default for its provider, one
    // taking Anthropic's from the built-in one, and one of an id Anthropic
    // has.
    let entry = |name: &str, provider: &str, id: &str, default: bool| {
        format!(
            "[models.{name}]\nprovider = \"{provider}\"\nid = \"{id}\"\nname = \"{name}\"\n\
             protocol = \"anthropic-messages\"\ncontext_window = 100000\nmax_output = 8000\n\
             default = {default}\n\n"
        )
    };
    world.raw_user_config(&format!(
        "{}{}{}[providers.local]\nprotocol = \"anthropic-messages\"\nbase_url = \
         \"http://127.0.0.1:9\"\n",
        entry("house", "local", "house-model", true),
        entry("mine", "anthropic", "claude-haiku-5-5", true),
        entry("twin", "local", "claude-sonnet-5-5", false),
    ));
    let core = world.core();
    assert_eq!(core.default_model("local").as_deref(), Some("house-model"));
    assert_eq!(
        core.default_model("anthropic").as_deref(),
        Some("claude-haiku-5-5")
    );
    assert!(core.model_shared("claude-sonnet-5-5"));
    assert!(!core.model_shared("house-model"));

    // An entry that replaces the built-in default, unmarked, keeps it the
    // default; two of the user's entries marked for one provider stop the
    // core (the short review of the fixes).
    world.raw_user_config(&entry("sonnet", "anthropic", "claude-sonnet-5-5", false));
    assert_eq!(
        world.core().default_model("anthropic").as_deref(),
        Some("claude-sonnet-5-5")
    );
    world.raw_user_config(&format!(
        "{}{}",
        entry("first", "anthropic", "claude-haiku-5-5", true),
        entry("second", "anthropic", "claude-opus-5-5", true),
    ));
    let error = start_error(&world);
    assert_eq!(error.code, "invalid_catalog");
    assert!(
        error.message.contains("[models.first]") && error.message.contains("[models.second]"),
        "{}",
        error.message
    );
}

/// Every provider the first run offers (the core's, as `credentials`
/// lists them) has exactly one default in the built-in catalog, but for a
/// server on the person's own machine, whose models the catalog cannot
/// know (the review of the fixes of step 25's third pull request).
#[test]
fn every_provider_the_first_run_offers_has_one_default_unless_it_is_local() {
    let world = World::new("first_run_defaults", []);
    world.user_config("");
    let core = world.core();
    let catalog = cineris_llm::Catalog::builtin().unwrap();
    let offered = core.credentials();
    assert!(offered.len() > 5, "{}", offered.len());
    for credential in offered {
        let name = credential.provider.as_str();
        let defaults: Vec<&str> = catalog
            .models()
            .iter()
            .filter(|entry| entry.default && entry.info.provider.as_str() == name)
            .map(|entry| entry.info.id.as_str())
            .collect();
        let local =
            cineris_llm::openai_chat::providers::builtin(name).is_some_and(|builtin| builtin.local);
        if local {
            assert!(defaults.is_empty(), "{name}: {defaults:?}");
            assert_eq!(core.default_model(name), None, "{name}");
        } else {
            assert_eq!(defaults.len(), 1, "{name}: {defaults:?}");
            assert_eq!(core.default_model(name).as_deref(), Some(defaults[0]));
        }
    }
}

/// After a first run, `--model` and a hand edit of `model.main` can name
/// another provider's model, and the session runs it: the first run names
/// its provider beside the model only when another provider has a model of
/// that id (the review of the fixes of step 25's third pull request:
/// `model.roles.main.provider` held every lookup to the provider chosen).
#[test]
fn another_providers_model_can_be_named_after_a_first_run() {
    let world = World::new("after_first_run", []);
    world.user_config("");
    let core = world.core();
    let model = core.default_model("anthropic");
    let choices = cineris_config::write::FirstRunChoices {
        provider: Some("anthropic".to_owned()),
        model_shared: model
            .as_deref()
            .is_some_and(|model| core.model_shared(model)),
        model,
        ..cineris_config::write::FirstRunChoices::default()
    };
    drop(core);
    let written = cineris_config::write::user_config_text(&choices).unwrap();
    assert!(
        written.contains("main = \"claude-sonnet-5-5\""),
        "{written}"
    );
    // The test's own: nothing kept, and the keys in a file of its own.
    let file = format!(
        "{written}\n[session]\ndurability = \"never\"\n\n[credentials]\nstore = \"file\"\n"
    );
    let main_of = |core: &Core| {
        world.run(async {
            let front = Front::start(&world, core).await;
            let main = &front.state.roles[&ModelRole::Main];
            let model = (
                main.model.as_str().to_owned(),
                main.provider.as_str().to_owned(),
            );
            front.close().await;
            model
        })
    };
    world.raw_user_config(&file);
    assert_eq!(
        main_of(&world.core()),
        ("claude-sonnet-5-5".to_owned(), "anthropic".to_owned())
    );
    // `--model`.
    let core = world.core_with(|options| {
        options.overrides.push(cineris_config::Override {
            key: "model.main".to_owned(),
            value: "deepseek-flash".to_owned(),
            flag: Some("--model".to_owned()),
        });
    });
    assert_eq!(
        main_of(&core),
        ("deepseek-flash".to_owned(), "deepseek".to_owned())
    );
    // A hand edit of the file.
    world.raw_user_config(&file.replace(
        "main = \"claude-sonnet-5-5\"",
        "main = \"openai/gpt-6-luna\"",
    ));
    assert_eq!(
        main_of(&world.core()),
        ("openai/gpt-6-luna".to_owned(), "openrouter".to_owned())
    );
}

#[test]
fn a_valid_entry_of_the_users_catalog_is_a_model_like_any_other() {
    let world = World::new("catalog_good", [says("From the house model.")]);
    world.raw_user_config(&format!("[model]\nmain = \"house-model\"\n\n{LOCAL_MODEL}"));
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let main = &front.state.roles[&ModelRole::Main];
        assert_eq!(main.model.as_str(), "house-model");
        assert_eq!(main.provider.as_str(), "local");
        front.turn("hello").await;
        let request = &world.provider.requests()[0];
        assert_eq!(request.model.info.context_window, 100_000);
        // It is listed with the built-in ones.
        let listed = front
            .op(Op::ModelsList(cineris_protocol::methods::Empty {}))
            .await
            .unwrap();
        let OpResult::ModelsList(listed) = listed else {
            panic!("not models");
        };
        let ids: Vec<&str> = listed
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert!(
            ids.contains(&"house-model") && ids.contains(&MODEL),
            "{ids:?}"
        );
        front.close().await;
    });
}

#[test]
fn a_session_is_refused_when_a_protected_path_cannot_be_placed() {
    let world = World::new("protect_bad", []);
    // No path can hold a NUL: nothing can place this entry.
    world.user_config("[sandbox]\nprotect = [\"secrets\\u0000hidden\"]\n");
    let core = world.core();
    world.run(async {
        let error = session_error(&world, &core).await;
        assert_eq!(error.kind, ErrorKind::Config);
        assert!(error.message.contains("secrets"), "{}", error.message);
        assert!(
            error.message.contains("sandbox.protect"),
            "{}",
            error.message
        );
    });
    assert!(world.logs().is_empty());
}

#[test]
fn a_session_is_refused_at_a_level_that_cannot_be_held_and_told_when_there_is_no_sandbox() {
    let world = World::new("level_refused", [says("Hi.")]);
    world.user_config("[permissions]\nmode = \"autonomous\"\n");
    let core = world.core();
    world.run(async {
        let error = session_error(&world, &core).await;
        assert_eq!(error.code, "sandbox_required");
        assert_eq!(error.kind, ErrorKind::Permission);
    });

    // At a level that can be held, the session starts and says that
    // nothing confines its commands.
    world.user_config("");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hi").await;
        let sandbox: Vec<String> = front
            .events()
            .into_iter()
            .filter_map(|event| match event.event {
                Event::Notice(Notice::Sandbox(notice)) => Some(notice.message),
                _ => None,
            })
            .collect();
        assert_eq!(sandbox.len(), 1, "{sandbox:?}");
        assert!(sandbox[0].starts_with("sandbox: off."), "{}", sandbox[0]);
        front.close().await;
    });
}

/// A user's catalog entry whose reasoning is a budget of 1,024 to 64,000
/// tokens, with an output of 64,000: the review of step 15's.
const BUDGET_MODEL: &str = "[models.budget]
provider = \"anthropic\"
id = \"claude-budget\"
name = \"Budget\"
protocol = \"anthropic-messages\"
context_window = 200000
max_output = 64000
input = [\"text\"]

[models.budget.reasoning]
min_tokens = 1024
max_tokens = 64000
";

/// The user's configuration with the budget model as the main one.
fn on_the_budget_model(world: &World, model: &str) {
    world.raw_user_config(&format!(
        "[session]\ndurability = \"never\"\n\n[credentials]\nstore = \"file\"\n\n[model]\nmain \
         = \"claude-budget\"\n{model}\n\n{BUDGET_MODEL}"
    ));
}

/// A reasoning budget leaves the answer room within what the main role's
/// requests may write (since the review of step 15, which found `max`
/// placed at the top of the range, where Anthropic refuses every request).
/// One that cannot is refused when the session starts, with both numbers,
/// and not at every request.
#[test]
fn a_reasoning_budget_that_leaves_the_answer_no_room_is_refused_at_the_start() {
    let world = World::new("budget_refused", [says("Hi.")]);
    on_the_budget_model(&world, "effort = \"max\"\nmax_output = 1024");
    let core = world.core();
    world.run(async {
        let error = session_error(&world, &core).await;
        assert_eq!(
            (error.kind.clone(), error.code.as_str()),
            (ErrorKind::Config, "reasoning_budget_too_large"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("1024") && error.message.contains("max_output"),
            "{}",
            error.message
        );
    });
    assert!(world.provider.requests().is_empty());

    // An output limit of the user's that holds a budget and the answer:
    // `max` is placed below it, and the request says so.
    on_the_budget_model(&world, "effort = \"max\"\nmax_output = 20000");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hi").await;
        front.close().await;
    });
    let request = &world.provider.requests()[0];
    assert_eq!(request.max_output, Some(20_000));
    assert_eq!(request.reasoning.budget_tokens, Some(15_000));
}

#[test]
fn a_session_needs_a_model_and_a_key() {
    let world = World::new("no_model", []);
    world.raw_user_config("");
    let core = world.core();
    world.run(async {
        let error = session_error(&world, &core).await;
        assert_eq!(
            (error.kind.clone(), error.code.as_str()),
            (ErrorKind::Config, "no_model")
        );
    });

    // With a model and the real provider layer, and no key anywhere.
    world.user_config("");
    let core = world.core_with(|options| options.connector = None);
    world.run(async {
        let error = session_error(&world, &core).await;
        assert_eq!(error.kind, ErrorKind::Auth);
        assert_eq!(error.code, "no_credentials");
        assert!(
            error.message.contains("ANTHROPIC_API_KEY"),
            "{}",
            error.message
        );
    });

    // A provider whose protocol this build does not speak.
    world.raw_user_config(&format!(
        "[model]\nmain = \"house-model\"\n\n{}",
        LOCAL_MODEL.replace(
            "[providers.local]\nprotocol = \"anthropic-messages\"",
            "[providers.local]\nprotocol = \"gopher\""
        )
    ));
    let core = world.core_with(|options| options.connector = None);
    world.run(async {
        let error = session_error(&world, &core).await;
        assert_eq!(error.code, "unknown_protocol");
        assert!(error.message.contains("gopher"), "{}", error.message);
    });
}

#[test]
fn the_harnesss_own_configuration_is_closed_to_the_tools() {
    let world = World::new(
        "protected_own",
        [
            calls(&[(
                "c1",
                "read",
                json!({"path": world_config_path_placeholder()}),
            )]),
            says("Could not."),
        ],
    );
    // The call names the real file.
    let config = world.dirs().user_config.unwrap();
    world.provider.push(calls(&[(
        "c2",
        "read",
        json!({"path": config.to_str().unwrap()}),
    )]));
    world.provider.push(says("Could not either."));
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
        front.turn("read a file that is not there").await;
        front.turn("read your own configuration").await;
        let results = front.results();
        let own = results.last().unwrap();
        assert_ne!(own.1, "success", "the configuration was read: {}", own.2);
        assert!(!own.2.contains(MODEL), "{}", own.2);
        front.close().await;
    });
}

fn world_config_path_placeholder() -> &'static str {
    "no-such-file.txt"
}

/// "Built-in deny rules protect the harness's own files and credentials":
/// no allow and no level lifts them. The stored keys are closed to reading
/// wherever they are kept: in the configuration directory on Linux, and on
/// Windows in a folder of their own in the local folder, which is the data
/// directory and is otherwise closed only to writing
/// (`docs/architecture/08-configuration.md`, "Where each system keeps the
/// harness's files"). Both layouts are given to the core on every system,
/// as `Dirs::from_env` makes them. The files are the ones the real store
/// leaves (the keys and their lock), and the temporary file of a write that
/// was killed, which holds every key. Found on Windows by the review of
/// step 13a, and the temporary file by the review of its fixes.
#[test]
fn the_stored_keys_are_closed_to_the_tools_in_each_systems_layout() {
    for windows in [false, true] {
        let world = World::new(
            if windows {
                "credentials_windows"
            } else {
                "credentials_linux"
            },
            [],
        );
        let mut dirs = world.dirs();
        if windows {
            // `%LOCALAPPDATA%\cineris\credentials`, beside the
            // sessions.
            // (`cineris-config`'s own test pins that layout.)
            dirs.credentials = dirs.data.as_ref().map(|data| data.join("credentials"));
        } else {
            // `~/.config/cineris`, beside the configuration.
            dirs.credentials.clone_from(&dirs.config);
        }
        let store = cineris_config::open_store(
            cineris_config::schema::StoreChoice::File,
            &dirs,
            std::sync::Arc::new(world.clock.clone()),
        )
        .unwrap();
        store
            .set(
                "openrouter",
                &cineris_base::Secret::new("stored-key-material-0000000000"),
            )
            .unwrap();
        let mut files: Vec<std::path::PathBuf> =
            std::fs::read_dir(dirs.credentials.as_ref().unwrap())
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("credentials"))
                })
                .collect();
        files.sort();
        assert_eq!(files.len(), 2, "the keys and their lock: {files:?}");
        // What a write that was killed between its temporary file and the
        // rename leaves beside the file: every key.
        let left = dirs
            .credentials
            .as_ref()
            .unwrap()
            .join(".cineris-0123456789abcdef.tmp");
        std::fs::write(
            &left,
            "[openrouter]\nkey = \"stored-key-material-0000000000\"\n",
        )
        .unwrap();
        files.push(left);
        for (index, file) in files.iter().enumerate() {
            world.provider.push(calls(&[(
                &format!("c{index}"),
                "read",
                json!({"path": file.to_str().unwrap()}),
            )]));
        }
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.dirs = Some(dirs.clone());
            // The session's own key is given, so the store is not asked.
            options.api_keys.insert(
                "anthropic".to_owned(),
                cineris_base::Secret::new("sk-ant-test-0000000000000000000000"),
            );
        });
        world.run(async {
            // At `unrestricted`, with a user there who would say yes.
            let front = Front::start_as(
                &world,
                &core,
                ClientInfo::user(),
                SessionSettings {
                    level: Some(cineris_protocol::lease::Level::Unrestricted),
                    ..SessionSettings::default()
                },
            )
            .await
            .unwrap();
            let asked = front.turn_allowing("read the stored keys").await;
            assert!(asked.is_empty(), "the stored keys were asked about");
            let results = front.results();
            assert_eq!(results.len(), files.len());
            for ((_, outcome, text), file) in results.iter().zip(&files) {
                assert_eq!(
                    outcome,
                    "denied",
                    "{} was read ({}): {text}",
                    file.display(),
                    if windows {
                        "Windows layout"
                    } else {
                        "Linux layout"
                    }
                );
                assert!(!text.contains("stored-key-material"), "{text}");
            }
            front.close().await;
        });
    }
}

/// The user's configuration file is the harness's own wherever it is:
/// `CINERIS_CONFIG` can name one outside the configuration directory, and
/// it holds the level and the rules the next session starts with. Found by
/// the independent review of the fixes to step 13a.
#[test]
fn the_users_configuration_file_is_closed_wherever_it_is_kept() {
    let world = World::new("config_elsewhere", []);
    let mut dirs = world.dirs();
    let elsewhere = world.dir.join("dotfiles").join("cineris.toml");
    std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    std::fs::copy(dirs.user_config.as_ref().unwrap(), &elsewhere).unwrap();
    dirs.user_config = Some(elsewhere.clone());
    let path = elsewhere.to_str().unwrap();
    world
        .provider
        .push(calls(&[("c1", "read", json!({"path": path}))]));
    world.provider.push(calls(&[(
        "c2",
        "write",
        json!({"path": path, "content": "[permissions]\nmode = \"unrestricted\"\n"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.dirs = Some(dirs.clone()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            ClientInfo::user(),
            SessionSettings {
                level: Some(cineris_protocol::lease::Level::Unrestricted),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        let asked = front.turn_allowing("raise your own level").await;
        assert!(asked.is_empty(), "the configuration file was asked about");
        for (tool, outcome, text) in front.results() {
            assert_eq!(outcome, "denied", "{tool}: {text}");
            assert!(!text.contains(MODEL), "{text}");
        }
        front.close().await;
    });
    let kept = std::fs::read_to_string(&elsewhere).unwrap();
    assert!(!kept.contains("unrestricted"), "{kept}");
}

/// A configuration file that is a link (kept in a dotfiles repository) is
/// closed where it leads too: a read or a write of either path reaches
/// the same file. Found by the second look of the independent review of
/// the fixes to step 13a.
#[cfg(unix)]
#[test]
fn a_configuration_file_that_is_a_link_is_closed_where_it_leads() {
    let world = World::new("config_link", []);
    let config = world.dirs().user_config.unwrap();
    let real = world.dir.join("dotfiles").join("cineris.toml");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::rename(&config, &real).unwrap();
    std::os::unix::fs::symlink(&real, &config).unwrap();
    let real_path = real.to_str().unwrap();
    world
        .provider
        .push(calls(&[("c1", "read", json!({"path": real_path}))]));
    world.provider.push(calls(&[(
        "c2",
        "write",
        json!({"path": real_path, "content": "[permissions]\nmode = \"unrestricted\"\n"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core();
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            ClientInfo::user(),
            SessionSettings {
                level: Some(cineris_protocol::lease::Level::Unrestricted),
                ..SessionSettings::default()
            },
        )
        .await
        .unwrap();
        let asked = front.turn_allowing("raise your own level").await;
        assert!(asked.is_empty(), "the configuration file was asked about");
        for (tool, outcome, text) in front.results() {
            assert_eq!(outcome, "denied", "{tool}: {text}");
        }
        front.close().await;
    });
    let kept = std::fs::read_to_string(&real).unwrap();
    assert!(!kept.contains("unrestricted"), "{kept}");
}

/// The configuration directory reached through a link is protected as
/// where it really is.
#[cfg(unix)]
#[test]
fn a_configuration_directory_behind_a_link_is_protected_where_it_really_is() {
    let world = World::new("protected_link", []);
    // Move the configuration to another place, and leave a link.
    let config_dir = world.dirs().config.unwrap();
    let real = world.dir.join("elsewhere").join("real-config");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::rename(&config_dir, &real).unwrap();
    std::os::unix::fs::symlink(&real, &config_dir).unwrap();

    let real_file = real.join("config.toml");
    world.provider.push(calls(&[(
        "c1",
        "read",
        json!({"path": real_file.to_str().unwrap()}),
    )]));
    world.provider.push(says("Could not."));
    let core = world.core();
    world.run(async {
        // A user is there to be asked, and would say yes: a read outside
        // the workspace is a question, and a read of the harness's own
        // configuration is a refusal nobody is asked about.
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.roles[&ModelRole::Main].model.as_str(), MODEL);
        let asked = front.turn_allowing("read the real file").await;
        assert!(asked.is_empty(), "a protected file was asked about");
        let result = &front.results()[0];
        assert_eq!(
            result.1, "denied",
            "read through its real path: {}",
            result.2
        );
        assert!(!result.2.contains(MODEL), "{}", result.2);
        front.close().await;
    });
}
