//! A build without the script engine (`--no-default-features`), end to
//! end through the core (`docs/architecture/12-extensions.md`, "A
//! build without the script engine"). An extension of the user's that
//! stands approved holds its gating hooks, failing: the one its cached
//! registration names, on its tools; changed since it was approved, the
//! same while the cache keeps its pin's registration, and both gates when
//! nothing keeps it, so that a prompt is refused. And a hook command of
//! the user's that does not compile holds its gate as it does with the
//! engine.

use std::sync::Arc;

use cineris_config::StateDir;
use cineris_ext::cache::{Entry, RegistrationCache};
use cineris_ext::folder::Snapshot;
use cineris_ext::manifest::Manifest;
use cineris_ext::registration::{HookDef, Registration};
use cineris_policy::{Definition, PinStore};
use cineris_protocol::event::TurnOutcome;
use cineris_protocol::methods::TrustResourceKind;
use serde_json::json;

use crate::{Front, World, calls, says};

#[test]
fn an_approved_extensions_gates_hold_in_a_build_without_the_engine() {
    let world = World::new(
        "no_engine",
        [
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            says("Done."),
            calls(&[("c2", "list", json!({"path": "."}))]),
            says("Done."),
            calls(&[("c3", "read", json!({"path": "a.txt"}))]),
            says("Done."),
        ],
    );
    world.put("a.txt", "the file\n");
    let folder = world.dirs().config.unwrap().join("extensions").join("probe");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"probe\"\nversion = \"1\"\nmain = \"main.js\"\n\n[capabilities]\nhooks \
         = [\"tool.before\", \"prompt.submit\"]\n",
    )
    .unwrap();
    std::fs::write(
        folder.join("main.js"),
        "cineris.on('tool.before', { tools: ['read'] }, () => ({ decision: 'deny' }));\n",
    )
    .unwrap();
    let state = StateDir::from_dirs(&world.dirs()).unwrap();
    std::fs::create_dir_all(state.root()).unwrap();
    let clock: cineris_base::SharedClock = Arc::new(world.clock.clone());
    let snapshot = Snapshot::read(&folder).unwrap();
    let manifest = Manifest::of(&snapshot).unwrap();
    PinStore::new(state.pins_file(), Arc::clone(&clock))
        .pin(&Definition {
            kind: TrustResourceKind::Extension,
            name: "probe".to_owned(),
            hash: snapshot.hash(),
        })
        .unwrap();
    RegistrationCache::new(state.registration_cache_file())
        .put(
            &snapshot.hash(),
            Entry::new(
                &manifest,
                Registration {
                    tools: Vec::new(),
                    commands: Vec::new(),
                    hooks: vec![HookDef {
                        event: "tool.before".to_owned(),
                        tools: vec!["read".to_owned()],
                        observe: false,
                        timeout_ms: None,
                    }],
                },
                None,
            ),
        )
        .unwrap();

    // 1. Approved as it is found: the gate its registration names holds on
    // `read`.
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("read it").await;
        let results = front.results();
        let notices = front.notices();
        let [(tool, outcome, text)] = results.as_slice() else {
            panic!("{results:?}");
        };
        assert_eq!(tool, "read");
        assert!(*outcome != "success", "the held gate let the read go: {text}");
        assert!(text.contains("no script engine"), "{text}");
        assert!(
            notices.iter().any(|notice| notice.contains("is not loaded")),
            "{notices:?}"
        );
        front.close().await;
    });

    // 2. A call it is not on goes.
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("list it").await;
        let results = front.results();
        let [(tool, outcome, _)] = results.as_slice() else {
            panic!("{results:?}");
        };
        assert_eq!(tool, "list");
        assert!(*outcome == "success", "{results:?}");
        front.close().await;
    });

    // 3. Changed since it was approved, the cache keeping its pin's
    // registration: that registration's gate holds, on `read`.
    std::fs::write(folder.join("main.js"), "// changed\n").unwrap();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("read it").await;
        let results = front.results();
        let [(tool, outcome, _)] = results.as_slice() else {
            panic!("{results:?}");
        };
        assert_eq!(tool, "read");
        assert!(*outcome != "success", "{results:?}");
        front.close().await;
    });

    // 4. Changed, and nothing keeps what it registered: both gates, so a
    // prompt is refused before it reaches the model.
    std::fs::remove_file(state.registration_cache_file()).unwrap();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let ended = front.turn("hello").await;
        assert_eq!(ended.outcome, TurnOutcome::Denied, "{ended:?}");
        front.close().await;
    });
}

/// A user's hook command that does not compile holds its gate in a build
/// without the engine, as with it: the seam is the same code.
#[test]
fn a_users_refused_gate_holds_in_a_build_without_the_engine() {
    let world = World::new(
        "no_engine_command",
        [
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            says("Done."),
        ],
    );
    world.put("a.txt", "the file\n");
    world.user_config(
        "[hooks.gate]\nevent = \"tool.before\"\ntools = [\"read\"]\ncommand = [\"check\", \
         \"--file={path}\"]\n",
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("read it").await;
        let results = front.results();
        let [(tool, outcome, text)] = results.as_slice() else {
            panic!("{results:?}");
        };
        assert_eq!(tool, "read");
        assert!(*outcome != "success", "the refused gate let the read go: {text}");
        front.close().await;
    });
}
