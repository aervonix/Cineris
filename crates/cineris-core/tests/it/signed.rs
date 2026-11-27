//! Signed approvals in the core (the signed approvals step;
//! `docs/steps/signed-approvals/answered.md`;
//! `docs/architecture/06-permissions-and-sandbox.md`, "Project trust"):
//! what a session uses counts only signed by the key the person opened their
//! approvals with, and what the stores pin restricts whatever its
//! signature. End to end, through the core: the stores are real files, and
//! what the model's code could write to them is written as it would write
//! it.
//!
//! - **Forgeries are refused:** the user's pins signed by another key, a
//!   project trusted by another key, an entry copied from another project,
//!   and a pin taken out by hand each approve nothing, and what they pin
//!   holds its gates.
//! - **Gates hold without the key:** a core whose approvals nobody opened
//!   (a headless run's, or a session opened with Esc at the password) holds
//!   a pinned extension's gates and a project's pinned hook's gate.
//! - **A yes is a signature:** one without a signature, or signed by
//!   another key, or given while the approvals are not open, trusts
//!   nothing, and the question stays open; the person's signed yes trusts.
//! - **An older store asks again,** and **a reset lifts no gate.**

#[cfg(feature = "scripts")]
use std::path::Path;

use cineris_core::{ClientInfo, Core};
use cineris_policy::{Definition, PinStore, TrustStore};
use cineris_protocol::methods::{
    Op, OpResult, ServerRequest, ServerResponse, SessionNewParams, TrustRequest, TrustResponse,
};
use cineris_protocol::session::{SessionSettings, SessionState, TrustStanding};
use cineris_testkit::Response;
use serde_json::json;

use crate::{OTHER_MODEL, Outcome, World, calls, says};

/// The project's hook that gates `read`, which never starts: a session
/// either holds it failing, or does not have it.
const GATE_HOOK: &str = "[hooks.gate]\nevent = \"tool.before\"\ntools = [\"read\"]\ncommand = \
                         [\"no-such-gate-program\"]\n";

/// The state directory's stores, as the world's core reads them.
fn stores(world: &World) -> (PinStore, TrustStore) {
    let state = cineris_config::StateDir::from_dirs(&world.dirs()).unwrap();
    std::fs::create_dir_all(state.root()).unwrap();
    let clock: cineris_base::SharedClock = std::sync::Arc::new(world.clock.clone());
    (
        PinStore::new(state.pins_file(), std::sync::Arc::clone(&clock)),
        TrustStore::new(state.trust_file(), clock),
    )
}

/// The world's project's key.
fn key_of(world: &World) -> cineris_protocol::ProjectKey {
    cineris_base::PathResolver::new(&world.workspace, cineris_base::PathOptions::new())
        .unwrap()
        .root()
        .project_key()
}

/// A turn whose model calls `tool` once, on the workspace's `notes.md`
/// (`read`) or the workspace (`list`), then ends.
fn one_call(tool: &str) -> [Response; 2] {
    let path = if tool == "read" { "notes.md" } else { "." };
    [
        calls(&[("c1", tool, json!({ "path": path }))]),
        says("Done."),
    ]
}

/// How the session's last call of `tool` ended, and what the model was
/// shown.
fn last_of(front: &crate::Front, tool: &str) -> (Outcome, String) {
    let results = front.results();
    let (_, outcome, text) = results
        .iter()
        .rev()
        .find(|(called, _, _)| called == tool)
        .unwrap_or_else(|| panic!("no call of `{tool}`: {results:?}"))
        .clone();
    (outcome, text)
}

/// Opens a session in the world's workspace as a user's front end that
/// answers each trust question with what `answer` makes of it (`None`: not
/// now), and gives every answer the core refused, by its code: the
/// session's state, the questions asked, and the refusals.
async fn open(
    world: &World,
    core: &Core,
    answer: &dyn Fn(&TrustRequest) -> Vec<TrustResponse>,
) -> (SessionState, Vec<TrustRequest>, Vec<String>) {
    let client = core.connect(ClientInfo::user());
    let opening = core.handle(
        client.id,
        Op::SessionNew(SessionNewParams {
            cwd: None,
            workspace: world.workspace.to_str().unwrap().to_owned(),
            settings: SessionSettings::default(),
            title: None,
            config: serde_json::Map::new(),
            ephemeral: true,
        }),
    );
    let mut asked = Vec::new();
    let mut refused = Vec::new();
    let result = {
        let answering = async {
            loop {
                match client.queue.next().await {
                    Some(cineris_core::Outgoing::Request {
                        id,
                        request: ServerRequest::Trust(request),
                    }) => {
                        // Each answer in turn, until one is taken; not now
                        // when none is.
                        let mut taken = false;
                        for response in answer(&request) {
                            match core.respond(client.id, &id, ServerResponse::Trust(response)) {
                                Ok(()) => {
                                    taken = true;
                                    break;
                                }
                                Err(error) => refused.push(error.code.clone()),
                            }
                        }
                        if !taken {
                            core.respond(
                                client.id,
                                &id,
                                ServerResponse::Trust(TrustResponse::default()),
                            )
                            .expect("the question is still open");
                        }
                        asked.push(request);
                    }
                    Some(_) => {}
                    None => std::future::pending::<()>().await,
                }
            }
        };
        let mut opening = std::pin::pin!(opening);
        let mut answering = std::pin::pin!(answering);
        std::future::poll_fn(|context| {
            let _ = answering.as_mut().poll(context);
            opening.as_mut().poll(context)
        })
        .await
    };
    let Ok(OpResult::SessionNew(result)) = result else {
        panic!("the session does not open: {result:?}");
    };
    // Closed: a test that needs a turn opens its session as `Front` does.
    let _ = core
        .handle(
            client.id,
            Op::SessionClose(cineris_protocol::methods::SessionRef {
                session_id: result.session.session_id.clone(),
            }),
        )
        .await;
    (result.session, asked, refused)
}

/// A yes, unsigned.
fn plain_yes() -> TrustResponse {
    TrustResponse {
        trusted: true,
        ..TrustResponse::default()
    }
}

// ------------------------------------------------------------ a yes

/// A yes to the trust question is the person's signature over what it
/// proposes: one without a signature, and one signed by another key, are
/// refused as an answer that cannot be read is, trust nothing, not even
/// for the session, and leave the question open; the person's signed yes
/// is taken, and trusts the project. While the person's approvals are not
/// open, even their signed yes is refused: nothing can check it.
#[test]
fn a_yes_trusts_only_signed_by_the_person() {
    let world = World::new("signed_yes", []);
    world.project_config(&format!(
        "[model]\nmain = \"{OTHER_MODEL}\"\n\n[[permissions.rules]]\neffect = \"write\"\nmatch = \
         \"docs/**\"\naction = \"allow\"\n"
    ));
    let (_, trust) = stores(&world);
    let key = key_of(&world);
    let model = crate::model_key();
    // Asked with no key open: the question is put, and no yes is taken.
    let closed = world.closed_core();
    let (state, asked, refused) = world.run(open(&world, &closed, &|request| {
        vec![plain_yes(), crate::yes_to(request)]
    }));
    assert_eq!(asked.len(), 1);
    assert!(asked[0].proposal.is_some() && !asked[0].summary.is_empty());
    assert_eq!(refused, ["invalid_answer", "invalid_answer"]);
    assert_eq!(state.trust, Some(TrustStanding::Untrusted));
    assert!(
        !trust.project(&key, None).unwrap().has_entry(),
        "nothing was written"
    );
    // With the person's approvals open: unsigned and the model's refused,
    // the question still open, and the person's taken.
    let core = world.core();
    let (state, asked, refused) = world.run(open(&world, &core, &|request| {
        vec![
            plain_yes(),
            TrustResponse {
                signature: Some("not a signature".to_owned()),
                ..plain_yes()
            },
            crate::signed_by(&model, request, plain_yes()),
            crate::yes_to(request),
        ]
    }));
    assert_eq!(asked.len(), 1);
    assert_eq!(
        refused,
        ["invalid_answer", "invalid_answer", "invalid_answer"]
    );
    assert_eq!(state.trust, Some(TrustStanding::Trusted));
    assert_eq!(
        state.roles[&cineris_protocol::model::ModelRole::Main]
            .model
            .as_str(),
        OTHER_MODEL
    );
    let stored = trust
        .project(&key, Some(&crate::person().verifier()))
        .unwrap();
    assert!(stored.is_trusted());
    assert!(
        !trust
            .project(&key, Some(&model.verifier()))
            .unwrap()
            .is_trusted()
    );
}

/// An unsigned yes, and one signed by another key, alone: the question
/// stays open after each, and the session opens untrusted at not now, with
/// nothing written.
#[test]
fn an_unsigned_or_wrongly_signed_yes_trusts_nothing() {
    let world = World::new("signed_yes_refused", []);
    world.project_config(&format!("[model]\nmain = \"{OTHER_MODEL}\"\n"));
    let (_, trust) = stores(&world);
    let key = key_of(&world);
    let core = world.core();
    let model = crate::model_key();
    let (state, asked, refused) = world.run(open(&world, &core, &|request| {
        vec![plain_yes(), crate::signed_by(&model, request, plain_yes())]
    }));
    assert_eq!(asked.len(), 1);
    assert_eq!(refused, ["invalid_answer", "invalid_answer"]);
    assert_eq!(state.trust, Some(TrustStanding::Untrusted));
    assert_eq!(
        state.roles[&cineris_protocol::model::ModelRole::Main]
            .model
            .as_str(),
        crate::MODEL
    );
    assert!(!trust.project(&key, None).unwrap().has_entry());
}

// ------------------------------------------------------------ the project's hook

/// Trusts the world's project with its hooks pinned, as `cineris
/// permissions trust` does with the person's key.
fn trust_with_its_hooks(world: &World) {
    let core = world.core();
    let project = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&project).unwrap();
    assert_eq!(hooks.len(), 1, "{hooks:?}");
    let reading = core.trust_reading(&project).unwrap();
    core.trust_project_with(&project, &reading, &[], &hooks, &crate::person_key())
        .unwrap();
}

/// A core whose approvals nobody opened holds the gate of a hook the
/// project's trust pins: the project is not trusted, nothing of it runs,
/// its gate fails every call it is on, the session is told why once, and
/// nobody is asked about the project. A project with no entry holds
/// nothing.
#[test]
fn a_projects_pinned_hook_holds_its_gate_without_the_key() {
    let world = World::new("signed_hook_keyless", one_call("read"));
    world.put("notes.md", "notes\n");
    world.project_config(GATE_HOOK);
    trust_with_its_hooks(&world);
    let core = world.closed_core();
    // Nobody is asked: the person's password is what is missing. (A
    // question answered not now would be asked no more in this core.)
    let (state, asked, _) = world.run(open(&world, &core, &|_| Vec::new()));
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(state.trust, Some(TrustStanding::Untrusted));
    world.run(async {
        let front = crate::Front::start(&world, &core).await;
        front.turn("read it").await;
        let (outcome, text) = last_of(&front, "read");
        assert_eq!(outcome, "denied", "{text}");
        assert!(
            text.contains(
                "this project's trust does not count in this session (your approvals are not open"
            ),
            "{text}"
        );
        front.drain();
        let told = "This project's trust does not count in this session";
        assert_eq!(
            front
                .notices()
                .iter()
                .filter(|notice| notice.contains(told))
                .count(),
            1,
            "{:?}",
            front.notices()
        );
        front.close().await;
    });
}

/// With no entry for the project, its hook holds nothing in a core whose
/// approvals are not open: no repository gates a session by itself.
#[test]
fn a_project_with_no_entry_holds_no_gate() {
    let world = World::new("signed_hook_no_entry", one_call("read"));
    world.put("notes.md", "notes\n");
    world.project_config(GATE_HOOK);
    let core = world.closed_core();
    world.run(async {
        let front = crate::Front::start(&world, &core).await;
        front.turn("read it").await;
        let (outcome, text) = last_of(&front, "read");
        assert_eq!(outcome, "success", "{text}");
        front.close().await;
    });
}

// ------------------------------------------------------------ extensions

/// An extension with a tool, `probe/echo`, and a gate on `list`.
#[cfg(feature = "scripts")]
const GATED: &str = "cineris.tool({ name: 'echo', description: 'Echoes.', input: { type: \
                     'object' }, run: (args) => ({ got: args }) });\ncineris.on('tool.before', \
                     { tools: ['list'] }, () => undefined);\n";

/// Writes the extension `probe` in `folder`: a tool, and a gate on `list`.
#[cfg(feature = "scripts")]
fn write_gated(folder: &Path) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"A gate\"\nmain = \
         \"main.js\"\n[capabilities]\nhooks = [\"tool.before\"]\n",
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), GATED).unwrap();
}

/// A turn whose model looks for the extension's tool and lists the
/// workspace, then ends.
#[cfg(feature = "scripts")]
fn looks() -> [Response; 2] {
    [
        calls(&[
            ("s1", "tool_search", json!({"query": "probe/"})),
            ("c1", "list", json!({"path": "."})),
        ]),
        says("Done."),
    ]
}

/// `count` turns of [`looks`].
#[cfg(feature = "scripts")]
fn looks_times(count: usize) -> Vec<Response> {
    (0..count).flat_map(|_| looks()).collect()
}

/// What the session's last turn of [`looks`] found: whether the
/// extension's tool was offered, and how its `list` ended, with what the
/// model was shown.
#[cfg(feature = "scripts")]
fn looked(front: &crate::Front) -> (bool, Outcome, String) {
    let (searched, found) = last_of(front, "tool_search");
    let offered = searched == "success" && found.contains("probe/echo");
    let (outcome, text) = last_of(front, "list");
    (offered, outcome, text)
}

/// Opens a session in `core`, answering a trust question not now, and
/// runs one turn of [`looks`]: what it found, and the session's state and
/// notices.
#[cfg(feature = "scripts")]
fn look_in(world: &World, core: &Core) -> (bool, Outcome, String, SessionState, Vec<String>) {
    world.run(async {
        let front = crate::extensions_found::start(world, core).await;
        front.turn("look").await;
        let (offered, outcome, text) = looked(&front);
        front.drain();
        let notices = front.notices();
        let state = front.state.clone();
        front.close().await;
        (offered, outcome, text, state, notices)
    })
}

/// The user's `probe`, written with its gate and cached as approving it
/// does.
#[cfg(feature = "scripts")]
fn users_gated(world: &World) -> std::path::PathBuf {
    let folder = crate::extensions_found::users(world);
    write_gated(&folder);
    crate::extensions_found::cache(world, &folder);
    folder
}

/// The user's pins signed by another key (the model's code, with a key of
/// its own) count for nothing in a session the person opened: the
/// extension is not loaded, and the gate its cached registration and its
/// manifest name holds, saying why. Pinned by the person, it is loaded.
#[cfg(feature = "scripts")]
#[test]
fn an_extension_pinned_with_another_key_is_not_loaded() {
    let world = World::new("signed_pinned_by_another", looks_times(2));
    let folder = users_gated(&world);
    let (pins, _) = stores(&world);
    let pinned = crate::extensions_found::definition(&folder);
    crate::model_key().pin(&pins, &pinned);
    let core = world.core();
    let (offered, outcome, text, _, notices) = look_in(&world, &core);
    assert!(!offered, "the model's pin loaded it");
    assert_eq!(outcome, "denied", "its gate holds: {text}");
    assert!(text.contains("not signed by your key"), "{text}");
    assert!(
        notices.iter().any(
            |notice| notice.contains("Your approvals of your own extensions")
                && notice.contains("not signed by your key")
        ),
        "{notices:?}"
    );
    // The person's own pin loads it, and its gate runs as it is.
    crate::person().pin(&pins, &pinned);
    let (offered, outcome, text, _, _) = look_in(&world, &core);
    assert!(offered);
    assert_eq!(outcome, "success", "{text}");
}

/// A pin taken out of the person's pins by hand (to lift its gate) leaves
/// a set that is not theirs: nothing of it counts, and the extension found
/// holds its gate.
#[cfg(feature = "scripts")]
#[test]
fn a_pin_taken_out_by_hand_lifts_no_gate() {
    let world = World::new("signed_pin_taken_out", looks());
    let folder = users_gated(&world);
    // A second of the user's, with no gate, pinned beside it: found, so
    // that what it pins holds nothing (one pinned and gone would hold both
    // gates, the pins counting or not).
    let other = folder.with_file_name("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("extension.toml"),
        "api = 1\nname = \"other\"\nversion = \"1\"\ndescription = \"No gate\"\nmain = \
         \"main.js\"\n",
    )
    .unwrap();
    std::fs::write(other.join("main.js"), "").unwrap();
    let (pins, _) = stores(&world);
    let person = crate::person();
    person.pin_all(
        &pins,
        &[
            crate::extensions_found::definition(&folder),
            Definition {
                kind: cineris_protocol::methods::TrustResourceKind::Extension,
                name: "other".to_owned(),
                hash: cineris_ext::folder::Snapshot::read(&other).unwrap().hash(),
            },
        ],
    );
    let written = std::fs::read_to_string(pins.path()).unwrap();
    let probe = written.find("name = \"probe\"").unwrap();
    let start = written[..probe].rfind("[[pin]]").unwrap();
    let end = written[probe..]
        .find("[[pin]]")
        .map_or(written.len(), |at| at + probe);
    let without = format!("{}{}", &written[..start], &written[end..]);
    assert!(!without.contains("probe"), "{without}");
    std::fs::write(pins.path(), &without).unwrap();
    let core = world.core();
    let (offered, outcome, text, _, _) = look_in(&world, &core);
    assert!(!offered);
    assert_eq!(
        outcome, "denied",
        "the pin taken out lifted its gate: {text}"
    );
    assert!(text.contains("not signed by your key"), "{text}");
}

/// A project trusted by another key, and an entry copied from another
/// project the person trusted, are not trusted: the question asks about the
/// project as one never trusted, nothing of it is loaded, and the extension
/// its entry pins holds its gate.
#[cfg(feature = "scripts")]
#[test]
fn a_project_trusted_with_another_key_or_copied_from_another_is_not_trusted() {
    for case in ["another key", "copied"] {
        let world = World::new("signed_project_forged", looks());
        world.project_config(&format!("[model]\nmain = \"{OTHER_MODEL}\"\n"));
        let folder = crate::extensions_found::projects(&world);
        write_gated(&folder);
        crate::extensions_found::cache(&world, &folder);
        let (_, trust) = stores(&world);
        let key = key_of(&world);
        let pinned = [crate::extensions_found::definition(&folder)];
        match case {
            "another key" => {
                crate::model_key().trust(&trust, &key, "~/work/app", &pinned, false);
            }
            _ => {
                let elsewhere = cineris_protocol::ProjectKey::new("another");
                crate::person().trust(&trust, &elsewhere, "~/another", &pinned, false);
                let written = std::fs::read_to_string(trust.path()).unwrap();
                let entry = &written[written.find("[[project]]").unwrap()..];
                let copied = entry.replace("key = \"another\"", &format!("key = \"{key}\""));
                assert_ne!(copied, entry);
                std::fs::write(trust.path(), format!("{written}\n{copied}")).unwrap();
            }
        }
        let core = world.core();
        let (asked, offered, outcome, text, state) = world.run(async {
            let (front, asked) =
                crate::extensions_found::open_answering(&world, &core, TrustResponse::default())
                    .await;
            front.turn("look").await;
            let (offered, outcome, text) = looked(&front);
            let state = front.state.clone();
            front.close().await;
            (asked, offered, outcome, text, state)
        });
        assert_eq!(asked.len(), 1, "{case}");
        assert!(!asked[0].trusted, "{case}: asked as a project not trusted");
        assert_eq!(state.trust, Some(TrustStanding::Untrusted), "{case}");
        assert_eq!(
            state.roles[&cineris_protocol::model::ModelRole::Main]
                .model
                .as_str(),
            crate::MODEL,
            "{case}"
        );
        assert!(!offered, "{case}");
        assert_eq!(outcome, "denied", "{case}: {text}");
        assert!(
            text.contains(
                "this project's trust does not count in this session (not signed by your key"
            ),
            "{case}: {text}"
        );
    }
}

/// A core whose approvals nobody opened holds the gates of the user's
/// pinned extension: not loaded, its gate holds, and the session says why.
#[cfg(feature = "scripts")]
#[test]
fn a_pinned_extension_holds_its_gates_without_the_key() {
    let world = World::new("signed_extension_keyless", looks());
    let folder = users_gated(&world);
    let (pins, _) = stores(&world);
    crate::person().pin(&pins, &crate::extensions_found::definition(&folder));
    let core = world.closed_core();
    let (offered, outcome, text, _, notices) = look_in(&world, &core);
    assert!(!offered);
    assert_eq!(outcome, "denied", "{text}");
    assert!(text.contains("your approvals are not open"), "{text}");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("your approvals are not open in this session")),
        "{notices:?}"
    );
}

// ------------------------------------------------------------ older stores, a reset

/// The project's hook's hash, as `cineris permissions trust` would show it.
fn the_hooks_hash(world: &World) -> String {
    let core = world.closed_core();
    let project = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&project).unwrap();
    assert_eq!(hooks.len(), 1, "{hooks:?}");
    hooks[0].hash().to_hex()
}

/// Opens a session in `core` whose trust question is answered not now,
/// then one that runs a turn calling `read`: the question asked, the
/// session's state, and how its `read` ended.
fn read_in(world: &World, core: &Core) -> (Vec<TrustRequest>, SessionState, Outcome, String) {
    let (_, asked, _) = world.run(open(world, core, &|_| Vec::new()));
    world.run(async {
        let front = crate::Front::start(world, core).await;
        front.turn("read it").await;
        let (outcome, text) = last_of(&front, "read");
        let state = front.state.clone();
        front.close().await;
        (asked, state, outcome, text)
    })
}

/// A `trust.toml` of version 2, which signed nothing, trusting the project
/// with its hook pinned, trusts nothing and keeps the hook's gate: the
/// project is asked about as one never trusted, its settings are not
/// applied, and the hook's gate fails its calls, saying why.
#[test]
fn an_older_trust_store_trusts_nothing_and_keeps_its_hooks_gate() {
    let world = World::new("signed_older_trust", one_call("read"));
    world.put("notes.md", "notes\n");
    world.project_config(&format!("[model]\nmain = \"{OTHER_MODEL}\"\n\n{GATE_HOOK}"));
    let (_, trust) = stores(&world);
    std::fs::write(
        trust.path(),
        format!(
            "version = 2\n\n[[project]]\nkey = \"{}\"\nroot = \"~/work/app\"\ntrusted_at = \
             \"2026-11-13T10:00:00Z\"\n\n[[project.pinned]]\nkind = \"hook\"\nname = \
             \"gate\"\nhash = \"{}\"\n",
            key_of(&world),
            the_hooks_hash(&world)
        ),
    )
    .unwrap();
    let core = world.core();
    let (asked, state, outcome, text) = read_in(&world, &core);
    assert_eq!(asked.len(), 1);
    assert!(!asked[0].trusted, "asked again as a project never trusted");
    assert_eq!(state.trust, Some(TrustStanding::Untrusted));
    assert_eq!(
        state.roles[&cineris_protocol::model::ModelRole::Main]
            .model
            .as_str(),
        crate::MODEL
    );
    assert_eq!(outcome, "denied", "the hook's gate: {text}");
    assert!(
        text.contains(
            "this project's trust does not count in this session (written by an older version \
             of Cineris"
        ),
        "{text}"
    );
}

/// A reset of the person's password takes the signature off the project's
/// entry and deletes nothing: under the new key the project is asked about
/// again, and the hook its entry pins keeps its gate until the person's
/// next yes.
#[test]
fn a_reset_lifts_no_hook_gate() {
    let world = World::new("signed_reset_hook", one_call("read"));
    world.put("notes.md", "notes\n");
    world.project_config(GATE_HOOK);
    trust_with_its_hooks(&world);
    let core = world.core();
    let (_, report) = core
        .reset_approvals_at(
            b"a new password, chosen at a reset",
            cineris_policy::test_support::Person::cost(),
        )
        .unwrap();
    assert_eq!((report.pins, report.projects), (0, 1));
    assert!(core.approvals_open(), "open with the new key");
    let (asked, state, outcome, text) = read_in(&world, &core);
    assert_eq!(asked.len(), 1, "asked again");
    assert_eq!(state.trust, Some(TrustStanding::Untrusted));
    assert_eq!(
        outcome, "denied",
        "the reset lifted the hook's gate: {text}"
    );
}

/// A `pins.toml` of version 1 (step 28's), which signed nothing, approves
/// nothing and keeps its gates: the user's extension it pins is not loaded,
/// and its gate holds, saying why.
#[cfg(feature = "scripts")]
#[test]
fn an_older_pin_store_approves_nothing_and_keeps_its_gates() {
    let world = World::new("signed_older_pins", looks());
    let folder = users_gated(&world);
    let (pins, _) = stores(&world);
    std::fs::write(
        pins.path(),
        format!(
            "version = 1\n\n[[pin]]\nkind = \"extension\"\nname = \"probe\"\nhash = \"{}\"\nat = \
             \"2026-11-13T10:00:00Z\"\n",
            crate::extensions_found::definition(&folder).hash.to_hex()
        ),
    )
    .unwrap();
    let core = world.core();
    let (offered, outcome, text, _, notices) = look_in(&world, &core);
    assert!(!offered);
    assert_eq!(outcome, "denied", "{text}");
    assert!(
        text.contains("written by an older version of Cineris"),
        "{text}"
    );
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("Your approvals of your own extensions")),
        "{notices:?}"
    );
}

/// A reset takes the signature off the user's pins and deletes nothing:
/// under the new key the extension they pin is not loaded, and holds its
/// gate, until the person approves it again.
#[cfg(feature = "scripts")]
#[test]
fn a_reset_lifts_no_extension_gate() {
    let world = World::new("signed_reset_extension", looks());
    let folder = users_gated(&world);
    let (pins, _) = stores(&world);
    crate::person().pin(&pins, &crate::extensions_found::definition(&folder));
    let core = world.core();
    let (_, report) = core
        .reset_approvals_at(
            b"a new password, chosen at a reset",
            cineris_policy::test_support::Person::cost(),
        )
        .unwrap();
    assert_eq!((report.pins, report.projects), (1, 0));
    let (offered, outcome, text, _, _) = look_in(&world, &core);
    assert!(!offered, "the reset approved it");
    assert_eq!(outcome, "denied", "the reset lifted its gate: {text}");
}

// ------------------------------------------------------------ what is held

/// What `cineris approvals status` and the application's question at a
/// session's opening read: what carries a signature, whosever it is, and
/// never what counts. It opens nothing: a core whose approvals are closed
/// stays closed.
#[test]
fn what_carries_a_signature_is_counted_and_nothing_is_opened() {
    use cineris_core::HeldKey;
    use cineris_protocol::methods::TrustResourceKind;
    let world = World::new("signed_held", []);
    let (pins, trust) = stores(&world);
    let core = world.closed_core();
    let project = core.project(&world.workspace).unwrap();
    // Nothing yet: no key, no pins, no entries.
    let held = core.approvals_held(Some(&project)).unwrap();
    assert_eq!(held.key, HeldKey::None);
    assert_eq!((held.pins, held.pins_carry_signature), (0, false));
    assert_eq!((held.projects_carrying, held.projects_not_carrying), (0, 0));
    assert!(!held.project_carries && !held.to_use());
    // A key file, the user's pins and this project's entry signed by the
    // person, and another project's entry signed by another key: each
    // carries a signature, whatever key it is by.
    let state = cineris_config::StateDir::from_dirs(&world.dirs()).unwrap();
    let key_file = state.root().join(cineris_core::signed::KEY_FILE);
    crate::person()
        .key_file()
        .write(&key_file, &world.clock)
        .unwrap();
    let tool = cineris_policy::Definition::of(TrustResourceKind::Hook, "probe", ["a"]);
    crate::person().pin(&pins, &tool);
    crate::person().trust(
        &trust,
        &project.key,
        &project.shown,
        std::slice::from_ref(&tool),
        false,
    );
    let elsewhere = cineris_protocol::ProjectKey::new("elsewhere");
    crate::model_key().trust(&trust, &elsewhere, "/elsewhere", &[], false);
    let held = core.approvals_held(Some(&project)).unwrap();
    let fingerprint = crate::person().key_file().fingerprint();
    assert!(
        matches!(&held.key, HeldKey::Made { fingerprint: made, .. } if *made == fingerprint),
        "{:?}",
        held.key
    );
    assert_eq!((held.pins, held.pins_carry_signature), (1, true));
    assert_eq!((held.projects_carrying, held.projects_not_carrying), (2, 0));
    assert!(held.project_carries && held.to_use());
    assert_eq!(
        held.project_pins,
        [("hook".to_owned(), "probe".to_owned(), tool.hash)]
    );
    // Another project's entry is not this one's.
    let other = core.approvals_held(None).unwrap();
    assert!(!other.project_carries);
    // A reset's way: the signatures taken off. Nothing carries one, and a
    // session has nothing to ask the password for.
    pins.strip_signature().unwrap();
    trust.strip_signatures().unwrap();
    let held = core.approvals_held(Some(&project)).unwrap();
    assert_eq!((held.pins, held.pins_carry_signature), (1, false));
    assert_eq!((held.projects_carrying, held.projects_not_carrying), (0, 2));
    assert!(!held.project_carries && !held.to_use());
    assert!(!core.approvals_open(), "a read opened the approvals");
    // A key file that cannot be read is said, and the stores still are.
    std::fs::write(&key_file, "nonsense").unwrap();
    let held = core.approvals_held(None).unwrap();
    assert!(matches!(held.key, HeldKey::Unreadable(_)), "{:?}", held.key);
    assert_eq!(held.pins, 1);
    assert!(!held.to_use(), "nothing carries a signature");
    // With something that carries one, the person is asked all the same,
    // so that the unlock's refusal names the key file and the reset (the
    // full review of the signed approvals, M1).
    crate::person().pin(&pins, &tool);
    let held = core.approvals_held(None).unwrap();
    assert!(held.pins_carry_signature && held.to_use(), "{held:?}");
    assert!(!core.approvals_open());
}

// ------------------------------------------------------------ --trust-project

/// `--trust-project`, for a core's options.
fn trust_project(options: &mut cineris_core::CoreOptions) {
    options.trust_project = true;
}

/// Opens a session in `core` and runs one turn of [`one_call`]'s `read`:
/// how its `read` ended, what the model was shown, and the session's
/// notices.
fn read_once(world: &World, core: &Core) -> (Outcome, String, Vec<String>) {
    world.run(async {
        let front = crate::Front::start(world, core).await;
        front.turn("read it").await;
        let (outcome, text) = last_of(&front, "read");
        front.drain();
        let notices = front.notices();
        front.close().await;
        (outcome, text, notices)
    })
}

/// A run with `--trust-project` and no key (a headless run's, whose
/// approvals nobody opened), in a project whose trust pins its gating hook:
/// `--trust-project` approves no hook, the entry does not count, so the hook
/// does not run unverified, and its gate holds, failing, saying why; the
/// session is told once (the review of the permission path: the gate lapsed;
/// the maintainers' answer to call 4 keeps every gate in a headless run).
#[test]
fn under_trust_project_without_the_key_a_pinned_hooks_gate_holds() {
    let world = World::new("signed_trust_project_hook", one_call("read"));
    world.put("notes.md", "notes\n");
    world.project_config(GATE_HOOK);
    trust_with_its_hooks(&world);
    let closed = world.closed_core_with(trust_project);
    let (outcome, text, notices) = read_once(&world, &closed);
    assert_eq!(outcome, "denied", "the pinned hook's gate lapsed: {text}");
    assert!(
        text.contains(
            "this project's trust does not count in this session (your approvals are not open"
        ),
        "{text}"
    );
    assert_eq!(
        notices
            .iter()
            .filter(|notice| notice.contains("trusted for this run by `--trust-project`"))
            .count(),
        1,
        "{notices:?}"
    );
}

/// The control: the same run with the person's key open runs the hook as
/// approved, which fails to start, so the call is denied by the hook itself,
/// and nothing is said of the entry.
#[test]
fn under_trust_project_with_the_key_the_pinned_hook_runs() {
    let world = World::new("signed_trust_project_hook_key", one_call("read"));
    world.put("notes.md", "notes\n");
    world.project_config(GATE_HOOK);
    trust_with_its_hooks(&world);
    let open = world.core_with(trust_project);
    let (outcome, text, notices) = read_once(&world, &open);
    assert_eq!(outcome, "denied", "{text}");
    assert!(!text.contains("does not count"), "{text}");
    assert!(
        !notices
            .iter()
            .any(|notice| notice.contains("trusted for this run by `--trust-project`")),
        "{notices:?}"
    );
}

/// A project's extension with a gate, pinned by the person's trust: a run
/// with `--trust-project` and no key does not load it, and holds its gate
/// (the review of the permission path).
#[cfg(feature = "scripts")]
#[test]
fn under_trust_project_without_the_key_a_pinned_extensions_gate_holds() {
    let world = World::new("signed_trust_project_extension", looks());
    let folder = crate::extensions_found::projects(&world);
    write_gated(&folder);
    crate::extensions_found::cache(&world, &folder);
    crate::extensions_found::trust(&world, &[crate::extensions_found::definition(&folder)]);
    let closed = world.closed_core_with(trust_project);
    let (offered, outcome, text, _, notices) = look_in(&world, &closed);
    assert!(!offered, "loaded unverified");
    assert_eq!(outcome, "denied", "its gate lapsed: {text}");
    assert!(
        text.contains("this project's trust does not count in this session"),
        "{text}"
    );
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("trusted for this run by `--trust-project`")),
        "{notices:?}"
    );
}

// ------------------------------------------------------------ what restricts while it does not count

/// While the user's pins do not count, an extension they pin that is gone
/// holds its gates as it does while they count: from what the cache keeps
/// at the hash pinned (the review of the permission path: it held nothing).
#[cfg(feature = "scripts")]
#[test]
fn a_pinned_users_extension_gone_holds_its_gates_without_the_key() {
    let world = World::new("signed_gone_keyless", looks());
    let folder = users_gated(&world);
    let (pins, _) = stores(&world);
    crate::person().pin(&pins, &crate::extensions_found::definition(&folder));
    std::fs::remove_dir_all(&folder).unwrap();
    let core = world.closed_core();
    let (offered, outcome, text, _, _) = look_in(&world, &core);
    assert!(!offered);
    assert_eq!(
        outcome, "denied",
        "the gone extension's gate lapsed: {text}"
    );
    assert!(
        text.contains("cannot be found where it was approved"),
        "{text}"
    );
}

/// The stated residual (`docs/architecture/12-extensions.md`, "A gate is the
/// person's own check, not a bound on the model"): a pin taken out of
/// `pins.toml` lifts no gate, but `pins.toml` deleted whole leaves nothing
/// that pins, and the extension's gate goes with it, as it does for one
/// never approved.
#[cfg(feature = "scripts")]
#[test]
fn the_pin_store_deleted_whole_holds_nothing() {
    let world = World::new("signed_pins_deleted", looks());
    let folder = users_gated(&world);
    let (pins, _) = stores(&world);
    crate::person().pin(&pins, &crate::extensions_found::definition(&folder));
    std::fs::remove_file(pins.path()).unwrap();
    let core = world.core();
    let (offered, outcome, text, _, _) = look_in(&world, &core);
    assert!(!offered);
    assert_eq!(outcome, "success", "{text}");
}

/// The gates `cineris ext disable` asks about are found by the pin, signed
/// or not: a subcommand's core never has the person's approvals open, and a
/// pinned extension holding a gate is still turned off only on a yes naming
/// it (step 28's M1; the front-end part's finding: "holds no gates" let it
/// go unasked). One not pinned holds none; a pin store that cannot be read
/// leaves what it holds not known.
#[cfg(feature = "scripts")]
#[test]
fn the_gates_a_disable_lets_go_are_found_without_the_key() {
    let world = World::new("signed_disable_gates", []);
    let folder = users_gated(&world);
    let (pins, _) = stores(&world);
    let pinned = crate::extensions_found::definition(&folder);
    crate::person().pin(&pins, &pinned);
    for core in [world.closed_core(), world.core()] {
        let project = core.project(&world.workspace).unwrap();
        let gates = core.extension_gates(&project, "probe").unwrap();
        assert_eq!(gates.hash, Some(pinned.hash));
        assert!(!gates.unknown);
        assert_eq!(gates.held, ["`tool.before on list`"], "{gates:?}");
        let none = core.extension_gates(&project, "other").unwrap();
        assert!(none.held.is_empty() && !none.unknown && none.hash.is_none());
    }
    std::fs::write(pins.path(), "version = 99\n").unwrap();
    let core = world.closed_core();
    let project = core.project(&world.workspace).unwrap();
    assert!(core.extension_gates(&project, "probe").unwrap().unknown);
}

// ------------------------------------------------------------ the core's keys

/// The core's refusals of a key, each with a key file the test wrote as
/// set-up writes it: no key file (`NoKey`); a password that does not make
/// the file's key (`NotThePassword`, with the file's fingerprint); a key
/// that opens the file while the core's approvals are open with another
/// (`NotTheOpenKey`, from `person_key` and from `open_approvals`); and a
/// set-up over a key file there (`KeyExists`, before any key is made).
#[test]
fn the_cores_key_refusals() {
    use cineris_core::KeyRefusal;
    use cineris_core::signed::PersonKey;
    let world = World::new("signed_key_refusals", []);
    let core = world.closed_core();
    let password = cineris_policy::test_support::PASSWORD.as_bytes();
    assert_eq!(core.person_key(password).err(), Some(KeyRefusal::NoKey));
    world.set_up_key(&core);
    let model = crate::model_key();
    let refused = core.person_key(b"the model's own password").err();
    let Some(KeyRefusal::NotThePassword { fingerprint, .. }) = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(fingerprint, crate::person().key_file().fingerprint());
    assert_eq!(
        core.set_up_approvals(b"a password long enough").err(),
        Some(KeyRefusal::KeyExists)
    );
    // Open with another key (the model's, made from its own key file): the
    // person's own key is refused while it is.
    let models_key =
        PersonKey::derive(b"the model's own password", model.key_file()).expect("it is made");
    core.open_approvals(&models_key).unwrap();
    assert_eq!(
        core.person_key(password).err(),
        Some(KeyRefusal::NotTheOpenKey)
    );
    assert_eq!(
        core.open_approvals(&crate::person_key()).err(),
        Some(KeyRefusal::NotTheOpenKey)
    );
    // With the person's key open, the person's password makes their key.
    let mine = world.core();
    assert!(mine.person_key(password).is_ok());
}

/// The core's methods that sign check the key before they propose or write
/// anything (the review of the permission path): with no key file, with a
/// key that does not open it, and with one that is not the key the core's
/// approvals are open with, `trust_project` is refused, saying which, and
/// nothing is written. The person's key, opening the file and open in the
/// core, trusts.
#[test]
fn a_key_that_may_not_sign_writes_nothing() {
    use cineris_core::signed::PersonKey;
    let world = World::new("signed_key_writes_nothing", []);
    world.project_config(&format!("[model]\nmain = \"{OTHER_MODEL}\"\n"));
    let (_, trust) = stores(&world);
    let key = key_of(&world);
    let models_key = PersonKey::derive(b"the model's own password", crate::model_key().key_file())
        .expect("it is made");
    let refused = |core: &Core, signing: &PersonKey, why: &str| {
        let project = core.project(&world.workspace).unwrap();
        let error = core.trust_project(&project, signing).unwrap_err();
        assert_eq!(error.code, "key_refused", "{why}: {}", error.message);
        assert!(
            error.message.contains("nothing was written") && error.message.contains(why),
            "{why}: {}",
            error.message
        );
        assert!(
            !trust.project(&key, None).unwrap().has_entry(),
            "{why}: nothing was written"
        );
    };
    let closed = world.closed_core();
    refused(&closed, &crate::person_key(), "no password has been set");
    world.set_up_key(&closed);
    refused(&closed, &models_key, "that is not the password");
    closed.open_approvals(&models_key).unwrap();
    refused(&closed, &crate::person_key(), "another key than the one");
    let core = world.core();
    let project = core.project(&world.workspace).unwrap();
    core.trust_project(&project, &crate::person_key()).unwrap();
    assert!(
        trust
            .project(&key, Some(&crate::person().verifier()))
            .unwrap()
            .is_trusted()
    );
}

/// `add_extension` and `remove_extension` check the key first too: the
/// model's key adds nothing (no folder written, no pin) and removes nothing
/// (the pin and the folder stay).
#[cfg(feature = "scripts")]
#[test]
fn an_extension_is_neither_added_nor_removed_with_a_key_that_may_not_sign() {
    use cineris_core::signed::PersonKey;
    let world = World::new("signed_key_extensions", []);
    let core = world.core();
    let project = core.project(&world.workspace).unwrap();
    let models_key = PersonKey::derive(b"the model's own password", crate::model_key().key_file())
        .expect("it is made");
    let (pins, _) = stores(&world);
    // Added from a folder elsewhere.
    let source = world.dir.join("source").join("probe");
    write_gated(&source);
    let to_add = core.extension_to_add(&project, &source).unwrap();
    let error = world
        .run(core.add_extension(to_add, &models_key))
        .unwrap_err();
    assert_eq!(error.code, "key_refused", "{}", error.message);
    assert!(
        !crate::extensions_found::users(&world).exists(),
        "nothing was written"
    );
    assert!(
        pins.pins(None).unwrap().held().is_empty(),
        "nothing was pinned"
    );
    // The person's own, pinned: not removed with the model's key.
    let folder = users_gated(&world);
    crate::person().pin(&pins, &crate::extensions_found::definition(&folder));
    let removal = core.extension_removal(&project, "probe").unwrap();
    let error = core
        .remove_extension(&removal, true, Some(&models_key))
        .unwrap_err();
    assert_eq!(error.code, "key_refused", "{}", error.message);
    assert!(folder.exists(), "nothing was deleted");
    assert_eq!(pins.pins(None).unwrap().held().len(), 1, "the pin stays");
}

// ------------------------------------------------------------ a reset nothing blocks

/// The password a reset in these tests chooses.
const NEW_PASSWORD: &[u8] = b"a new password, chosen at a reset";

/// What a reset gives.
type Reset =
    Result<(cineris_core::signed::PersonKey, cineris_core::ResetReport), cineris_core::KeyRefusal>;

/// Resets the core's approvals at a test's cost, each lock waited for not
/// at all.
fn reset_now(core: &Core) -> Reset {
    core.reset_approvals_within(
        NEW_PASSWORD,
        cineris_policy::test_support::Person::cost(),
        std::time::Duration::ZERO,
    )
}

/// What stands beside `path` under a name a reset sets things aside with
/// (`<name>.aside-…`).
fn set_aside_of(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let name = format!("{}.aside-", path.file_name().unwrap().to_string_lossy());
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|file| file.to_string_lossy().starts_with(&name))
        })
        .collect();
    found.sort();
    found
}

/// One of the user's pins, for a store to hold.
fn a_pin() -> Definition {
    Definition::of(
        cineris_protocol::methods::TrustResourceKind::Extension,
        "tracker",
        ["x"],
    )
}

/// A store another program made unreadable does not block the person's
/// reset (the review of the permission path: a forgotten password's only
/// way out): a `pins.toml` of a version this build does not understand, and
/// a folder at `trust.toml`'s name, are each left as they are and said; the
/// other store's signatures are taken off; and the new password opens the
/// approvals. The unreadable store is never moved aside, which would lift
/// its gates.
#[test]
fn a_store_not_understood_does_not_block_a_reset() {
    for case in ["pins", "trust"] {
        let world = World::new("signed_reset_unreadable", []);
        let state = cineris_config::StateDir::from_dirs(&world.dirs()).unwrap();
        let core = world.core();
        let (pins, trust) = stores(&world);
        let left = if case == "pins" {
            crate::person().trust(&trust, &key_of(&world), "~/work/app", &[], false);
            std::fs::write(state.pins_file(), "version = 99\n").unwrap();
            state.pins_file()
        } else {
            crate::person().pin(&pins, &a_pin());
            std::fs::create_dir_all(state.trust_file()).unwrap();
            state.trust_file()
        };
        let (_, report) =
            reset_now(&core).unwrap_or_else(|refusal| panic!("{case}: blocked: {refusal}"));
        let [said] = report.left.as_slice() else {
            panic!("{case}: {:?}", report.left);
        };
        let file = left.file_name().unwrap().to_string_lossy().into_owned();
        assert!(said.contains(&file), "{case}: {said}");
        assert_eq!(report.pins + report.projects, 1, "{case}: {report:?}");
        if case == "pins" {
            assert_eq!(std::fs::read_to_string(&left).unwrap(), "version = 99\n");
        } else {
            assert!(left.is_dir());
        }
        assert!(set_aside_of(&left).is_empty(), "{case}: never moved aside");
        assert!(core.person_key(NEW_PASSWORD).is_ok(), "{case}");
    }
}

/// A folder put at a lock's name, the stores' and the key file's, does not
/// block a reset: each is moved aside, and the reset takes the signatures
/// off and sets the new key.
#[test]
fn a_folder_at_a_locks_name_does_not_block_a_reset() {
    let world = World::new("signed_reset_lock_folders", []);
    let state = cineris_config::StateDir::from_dirs(&world.dirs()).unwrap();
    let core = world.core();
    let (pins, trust) = stores(&world);
    crate::person().trust(&trust, &key_of(&world), "~/work/app", &[], false);
    crate::person().pin(&pins, &a_pin());
    let key_file = core.approval_key_file().unwrap();
    let locks: Vec<std::path::PathBuf> = [state.pins_file(), state.trust_file(), key_file]
        .iter()
        .map(|file| cineris_base::private::lock_path(file))
        .collect();
    for lock in &locks {
        let _ = std::fs::remove_file(lock);
        std::fs::create_dir_all(lock.join("inside")).unwrap();
    }
    let (_, report) = reset_now(&core).unwrap();
    assert_eq!((report.pins, report.projects), (1, 1));
    assert!(report.left.is_empty(), "{:?}", report.left);
    for lock in &locks {
        assert_eq!(set_aside_of(lock).len(), 1, "{}", lock.display());
    }
    assert!(core.person_key(NEW_PASSWORD).is_ok());
}

/// A store's lock held by another program (a process left running) is
/// waited for no longer than the reset's wait, and the store is left as it
/// is and said (the review of the permission path: the reset waited on
/// it). The key file's lock still held after the wait is moved aside, as a
/// folder at its name is, and a fresh one locked: the reset goes on (the
/// full review of the signed approvals, M2: it stopped every reset).
#[test]
fn a_held_lock_is_waited_for_no_longer_than_the_wait() {
    let world = World::new("signed_reset_held_lock", []);
    let state = cineris_config::StateDir::from_dirs(&world.dirs()).unwrap();
    let core = world.core();
    let (pins, trust) = stores(&world);
    let key = key_of(&world);
    crate::person().trust(&trust, &key, "~/work/app", &[], false);
    crate::person().pin(&pins, &a_pin());
    // The key file's lock held: moved aside, and the reset goes on.
    let key_file = core.approval_key_file().unwrap();
    let lock = cineris_base::private::lock_path(&key_file);
    let held = cineris_base::private::hold_lock(&key_file)
        .unwrap()
        .expect("this file system locks");
    let (_, report) = reset_now(&core).unwrap_or_else(|refusal| panic!("blocked: {refusal}"));
    drop(held);
    assert_eq!((report.pins, report.projects), (1, 1));
    assert!(report.left.is_empty(), "{:?}", report.left);
    assert_eq!(set_aside_of(&lock).len(), 1, "the held lock is aside");
    let person = crate::person().verifier();
    assert!(!trust.project(&key, Some(&person)).unwrap().is_trusted());
    assert!(core.person_key(NEW_PASSWORD).is_ok());
    // The pins' lock held: they are left signed by the key before, and said.
    crate::person().pin(&pins, &a_pin());
    let held = cineris_base::private::hold_lock(&state.pins_file())
        .unwrap()
        .expect("this file system locks");
    let (_, report) = reset_now(&core).unwrap();
    drop(held);
    assert_eq!((report.pins, report.projects), (0, 1));
    let [said] = report.left.as_slice() else {
        panic!("{:?}", report.left);
    };
    assert!(
        said.contains("pins.toml") && said.contains("lock"),
        "{said}"
    );
    assert!(core.person_key(NEW_PASSWORD).is_ok());
}

/// A state directory another program took the write away from does not
/// block a reset: the reset makes it owner-writable again before it writes
/// the new key (the full review of the signed approvals, M2).
#[cfg(unix)]
#[test]
fn a_state_directory_without_its_write_does_not_block_a_reset() {
    use std::os::unix::fs::PermissionsExt;
    let world = World::new("signed_reset_read_only_state", []);
    let state = cineris_config::StateDir::from_dirs(&world.dirs()).unwrap();
    let core = world.core();
    let (pins, _) = stores(&world);
    crate::person().pin(&pins, &a_pin());
    std::fs::set_permissions(state.root(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let reset = reset_now(&core);
    let mode = std::fs::metadata(state.root())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    std::fs::set_permissions(state.root(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (_, report) = reset.unwrap_or_else(|refusal| panic!("blocked: {refusal}"));
    assert_eq!(report.pins, 1);
    assert_eq!(mode, 0o700, "made owner-writable");
    assert!(core.person_key(NEW_PASSWORD).is_ok());
}

/// Makes `link`, a link to the file `target`. `false` when this machine
/// cannot (Windows without the right to), where the test then checks
/// nothing of it; a CI runner can, and must.
#[expect(
    clippy::print_stdout,
    reason = "a test that checks nothing on this machine says so, as the test runner shows it"
)]
fn make_file_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    match made {
        Ok(()) => true,
        Err(error) if std::env::var_os("GITHUB_ACTIONS").is_none() => {
            println!("skipped: cannot make a link here ({error})");
            false
        }
        Err(error) => panic!("the CI runner cannot make a link: {error}"),
    }
}

/// Whatever another program put at the key file's name does not stop a
/// reset, nor leave it half-done: a key file of its own made read-only (the
/// review of the system's layer: the reset took every store's signatures off
/// and then failed to write the key), a folder, and a link to a file
/// elsewhere are each moved aside, the new password opens the approvals,
/// and what was there is kept beside it, a link's target untouched.
#[test]
fn what_stands_at_the_key_files_name_is_moved_aside_at_a_reset() {
    for case in ["read-only", "folder", "link"] {
        let world = World::new("signed_reset_key_aside", []);
        let core = world.closed_core();
        let key_file = core.approval_key_file().unwrap();
        std::fs::create_dir_all(key_file.parent().unwrap()).unwrap();
        let model = crate::model_key();
        let target = world.dir.join("elsewhere.toml");
        match case {
            "read-only" => {
                model.key_file().write(&key_file, &world.clock).unwrap();
                let mut permissions = std::fs::metadata(&key_file).unwrap().permissions();
                permissions.set_readonly(true);
                std::fs::set_permissions(&key_file, permissions).unwrap();
            }
            "folder" => std::fs::create_dir_all(key_file.join("inside")).unwrap(),
            _ => {
                model.key_file().write(&target, &world.clock).unwrap();
                if !make_file_link(&target, &key_file) {
                    continue;
                }
            }
        }
        let (_, trust) = stores(&world);
        crate::person().trust(&trust, &key_of(&world), "~/work/app", &[], false);
        let (_, report) = reset_now(&core).unwrap_or_else(|refusal| panic!("{case}: {refusal}"));
        assert_eq!(report.projects, 1, "{case}");
        assert!(
            core.person_key(NEW_PASSWORD).is_ok(),
            "{case}: the new password opens"
        );
        let found = set_aside_of(&key_file);
        let [aside] = found.as_slice() else {
            panic!("{case}: {found:?}");
        };
        if case == "folder" {
            assert!(aside.join("inside").is_dir());
        } else {
            let kept = if case == "link" {
                assert!(std::fs::symlink_metadata(aside).unwrap().is_symlink());
                &target
            } else {
                aside
            };
            let kept = cineris_core::signed::KeyFile::read(kept).unwrap().unwrap();
            assert_eq!(kept.fingerprint(), model.key_file().fingerprint(), "{case}");
        }
        if case == "read-only" {
            // Writable again, for the world's folder to be removed.
            let mut permissions = std::fs::metadata(aside).unwrap().permissions();
            #[expect(
                clippy::permissions_set_readonly_false,
                reason = "the test's own file, made writable for its folder's removal"
            )]
            permissions.set_readonly(false);
            std::fs::set_permissions(aside, permissions).unwrap();
        }
        // Nothing of the new key is left under another name.
        let staged = format!("{}.new-", cineris_core::signed::KEY_FILE);
        let leftovers: Vec<std::path::PathBuf> = std::fs::read_dir(key_file.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|file| file.to_string_lossy().starts_with(&staged))
            })
            .collect();
        assert!(leftovers.is_empty(), "{case}: {leftovers:?}");
    }
}

/// A reset the person makes writes the standard cost: the cost it makes the
/// key at, seen without making one (which takes seconds unoptimised); and a
/// password too short is refused before any key is made.
#[test]
fn a_reset_writes_the_standard_cost() {
    assert_eq!(Core::reset_cost(), cineris_core::signed::Cost::STANDARD);
    let world = World::new("signed_reset_cost", []);
    let core = world.core();
    assert_eq!(
        core.reset_approvals(b"too short").err(),
        Some(cineris_core::KeyRefusal::TooShort)
    );
    // Not text: refused as such, before anything changes, and counted with
    // no copy made (the full review of the signed approvals, L10).
    let Some(cineris_core::KeyRefusal::Failed(why)) = core.reset_approvals(&[0xff; 32]).err()
    else {
        panic!("a password that is not text was taken");
    };
    assert!(why.contains("is text"), "{why}");
    assert!(
        core.person_key(cineris_policy::test_support::PASSWORD.as_bytes())
            .is_ok()
    );
}

// ------------------------------------------------------------ one reading, and a bound

/// The trust question is put only with a proposal worked out against the
/// very entry it is shaped by (the review of the permission path): an older
/// entry, signed by the person, put back between the reading and the
/// proposal puts no question, and the session opens with the trust as it
/// stands; nothing is signed again at a new revision.
#[test]
fn a_question_is_put_only_with_a_proposal_from_its_own_reading() {
    let mut world = World::new("signed_one_reading", []);
    world.project_config(GATE_HOOK);
    let (_, trust) = stores(&world);
    let key = key_of(&world);
    let person = crate::person();
    person.trust(&trust, &key, "~/work/app", &[], false);
    let older = std::fs::read(trust.path()).unwrap();
    person.approve(
        &trust,
        &key,
        &[Definition::of(
            cineris_protocol::methods::TrustResourceKind::Hook,
            "other",
            ["x"],
        )],
        false,
    );
    // The core's blocking work on threads of its own, so that the look can
    // be held while the test goes on.
    world._order = None;
    let core = world.core();
    let shaped = cineris_testkit::Hold::new(cineris_core::ext::THE_QUESTIONS_TRUST_IS_READ);
    let answer = |request: &TrustRequest| vec![crate::yes_to(request)];
    let (state, asked) = world.run(async {
        let mut opening = std::pin::pin!(open(&world, &core, &answer));
        shaped.drive_until_reached(opening.as_mut()).await;
        std::fs::write(trust.path(), &older).unwrap();
        shaped.release();
        let (state, asked, _) = opening.await;
        (state, asked)
    });
    assert!(
        asked.is_empty(),
        "a question from another reading: {asked:?}"
    );
    assert_eq!(state.trust, Some(TrustStanding::Trusted));
    assert_eq!(
        trust.project(&key, None).unwrap().revision(),
        1,
        "not signed again"
    );
}

/// A project's entry that moves under each signed yes (something other than
/// Cineris writing the store as the person answers) has its question put
/// three times at most: then the session opens with the trust as it stands,
/// and says why (the review of the permission path: it was put without
/// end).
#[test]
fn the_question_is_put_three_times_at_most_when_the_entry_moves_under_each_yes() {
    let world = World::new("signed_asked_at_most", []);
    world.project_config(
        "[[permissions.rules]]\neffect = \"write\"\nmatch = \"docs/**\"\naction = \"allow\"\n",
    );
    let (_, trust) = stores(&world);
    let key = key_of(&world);
    let core = world.core();
    let rounds = std::cell::Cell::new(0);
    let answer = |request: &TrustRequest| {
        // The entry moves before each of the first four answers.
        let round = rounds.get() + 1;
        rounds.set(round);
        let person = crate::person();
        if round == 1 {
            person.trust(&trust, &key, "~/work/app", &[], false);
        } else if round <= 4 {
            let moved = format!("moved-{round}");
            person.approve(
                &trust,
                &key,
                &[Definition::of(
                    cineris_protocol::methods::TrustResourceKind::Hook,
                    &moved,
                    ["x"],
                )],
                false,
            );
        }
        vec![crate::yes_to(request)]
    };
    let (state, asked, refused) = world.run(open(&world, &core, &answer));
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(asked.len(), 3, "{asked:?}");
    assert_eq!(state.trust, Some(TrustStanding::RulesChanged));
    let stored = trust
        .project(&key, Some(&crate::person().verifier()))
        .unwrap();
    assert_eq!(stored.revision(), 3, "no yes was written");
}

// ------------------------------------------------------------ the full review

/// What a proposal's summary says of a pin it carries as held.
#[cfg(feature = "scripts")]
const PROBE_HELD: &str = "the extension `probe`: approved earlier but not signed by your key; \
                          kept, holding its gates, approving nothing";

/// The full review of the signed approvals, M1: after a reset, or an
/// upgrade, nothing carries a signature, so the application asks for no
/// password and its core stays closed. The project whose entry the reset
/// left unsigned is asked about again all the same, as its session's words
/// say, its yes setting up or opening the approvals; only one that carries
/// a signature waits for the password, which is what is missing.
#[test]
fn a_project_whose_signature_a_reset_took_off_is_asked_about_again() {
    let world = World::new("signed_unsigned_asked", []);
    world.project_config(
        "[[permissions.rules]]\neffect = \"write\"\nmatch = \".github/**\"\naction = \"allow\"\n",
    );
    let (_, trust) = stores(&world);
    let key = key_of(&world);
    crate::person().trust(&trust, &key, "~/work/app", &[], false);
    assert_eq!(trust.strip_signatures().unwrap(), 1);
    let core = world.closed_core();
    let project = core.project(&world.workspace).unwrap();
    assert!(
        !core.approvals_held(Some(&project)).unwrap().to_use(),
        "nothing carries a signature: no password is asked for"
    );
    let (state, asked, _) = world.run(open(&world, &core, &|_| Vec::new()));
    assert_eq!(
        asked.len(),
        1,
        "a project left unsigned is asked about again"
    );
    assert!(!asked[0].trusted && asked[0].proposal.is_some());
    assert_eq!(state.trust, Some(TrustStanding::Untrusted));
    // Signed by the person again: a closed core does not ask.
    crate::person().trust(&trust, &key, "~/work/app", &[], false);
    let core = world.closed_core();
    let (_, asked, _) = world.run(open(&world, &core, &|_| Vec::new()));
    assert!(asked.is_empty(), "{asked:?}");
}

/// The extension `other`, with a tool and no gate, in `folder`.
#[cfg(feature = "scripts")]
fn write_other(folder: &Path) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"other\"\nversion = \"1\"\ndescription = \"Another\"\nmain = \
         \"main.js\"\n",
    )
    .unwrap();
    std::fs::write(
        folder.join("main.js"),
        "cineris.tool({ name: 'x', description: 'X.', input: { type: 'object' }, run: () => \
         ({}) });\n",
    )
    .unwrap();
}

/// The full review of the signed approvals (the front ends' finding 1, the
/// core's L2), the reviewer's two cases: after a reset, and after the
/// model's code re-signed `pins.toml` with a key of its own, the person's
/// next yes over their pins (`cineris ext add` of another extension, which
/// shows them only that one) signed a set without the pin that held a
/// gate, and the gate lapsed. The pin is carried as held now: its gate
/// holds, it approves nothing, and the summary the front end says names it.
#[cfg(feature = "scripts")]
#[test]
fn an_unrelated_yes_keeps_the_gate_of_a_pin_it_did_not_show() {
    for case in ["reset", "model"] {
        let world = World::new(&format!("signed_unshown_{case}"), looks_times(2));
        let folder = users_gated(&world);
        let (pins, _) = stores(&world);
        let pinned = crate::extensions_found::definition(&folder);
        crate::person().pin(&pins, &pinned);
        let core = world.core();
        let key = if case == "reset" {
            core.reset_approvals_at(
                b"a new password, chosen at a reset",
                cineris_policy::test_support::Person::cost(),
            )
            .unwrap()
            .0
        } else {
            crate::model_key().pin(&pins, &pinned);
            crate::person_key()
        };
        let (_, before, text, _, _) = look_in(&world, &core);
        assert_eq!(before, "denied", "{case}: the gate held before: {text}");
        // The person adds an unrelated extension: shown `other` and its
        // hash, and signed.
        let source = world.dir.join("source").join("other");
        write_other(&source);
        let project = core.project(&world.workspace).unwrap();
        let to_add = core.extension_to_add(&project, &source).unwrap();
        let added = world.run(core.add_extension(to_add, &key)).unwrap();
        assert!(
            added.summary.iter().any(|line| line == PROBE_HELD),
            "{case}: {:?}",
            added.summary
        );
        let file = std::fs::read_to_string(pins.path()).unwrap();
        assert!(
            file.contains("[[held]]") && file.contains("probe"),
            "{case}: the yes over `other` dropped the pin of `probe`:\n{file}"
        );
        let counted = pins.pins(Some(&key.verifier())).unwrap();
        assert!(counted.counts(), "{case}");
        assert_eq!(
            counted.standing(&pinned),
            cineris_policy::Standing::New,
            "{case}: the held pin approves nothing"
        );
        let (offered, after, text, _, notices) = look_in(&world, &core);
        assert!(!offered, "{case}: the held pin loaded it");
        assert_eq!(after, "denied", "{case}: the gate lapsed: {text}\n{file}");
        assert!(text.contains("approved earlier"), "{case}: {text}");
        assert!(
            notices
                .iter()
                .any(|notice| notice.contains("Your extension `probe`")
                    && notice.contains("approved earlier and is not signed by your key")),
            "{case}: {notices:?}"
        );
    }
}

/// `cineris permissions trust` answered no to the project's extensions,
/// over an entry a reset or an older version left unsigned (the full
/// review of the signed approvals): the trust is signed for what was
/// shown, and the extension the entry pinned is carried as held, its gate
/// holding, approving nothing; the summary says so.
#[cfg(feature = "scripts")]
#[test]
fn a_trust_that_leaves_an_extension_out_keeps_the_gate_an_unsigned_entry_pinned() {
    let world = World::new("signed_trust_no_held", looks());
    let folder = crate::extensions_found::projects(&world);
    write_gated(&folder);
    crate::extensions_found::cache(&world, &folder);
    let probe = crate::extensions_found::definition(&folder);
    crate::extensions_found::trust(&world, std::slice::from_ref(&probe));
    let (_, trust) = stores(&world);
    trust.strip_signatures().unwrap();
    let core = world.core();
    let project = core.project(&world.workspace).unwrap();
    let shown = core.project_extensions(&project).unwrap();
    assert_eq!(shown.len(), 1, "{shown:?}");
    // No to its extensions: the project's trust alone is signed.
    let trusted = core
        .trust_project_with(&project, &[], &[], &crate::person_key())
        .unwrap();
    assert!(
        trusted.summary.iter().any(|line| line == PROBE_HELD),
        "{:?}",
        trusted.summary
    );
    let stored = trust
        .project(&key_of(&world), Some(&crate::person().verifier()))
        .unwrap();
    assert!(stored.is_trusted());
    assert_eq!(stored.standing(&probe), cineris_policy::Standing::New);
    let (offered, outcome, text, state, notices) = look_in(&world, &core);
    assert_eq!(state.trust, Some(TrustStanding::Trusted));
    assert!(!offered, "the held pin loaded it");
    assert_eq!(outcome, "denied", "its gate lapsed: {text}");
    assert!(text.contains("approved earlier"), "{text}");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("This project's extension `probe`")
                && notice.contains("approved earlier")),
        "{notices:?}"
    );
}

/// Writes `probe` in `folder`: its tool, its gate on `list`, and `text` as
/// its prompt file.
#[cfg(feature = "scripts")]
fn write_gated_with_prompt(folder: &Path, text: &str) {
    write_gated(folder);
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"A gate\"\nmain = \
         \"main.js\"\nprompt = \"prompt.md\"\n[capabilities]\nhooks = [\"tool.before\"]\n",
    )
    .unwrap();
    std::fs::write(folder.join("prompt.md"), text).unwrap();
}

/// Registers the extension in `folder` as approving it does, and keeps
/// what it registered in the cache with the text of its prompt file.
#[cfg(feature = "scripts")]
fn cache_with_prompt(world: &World, folder: &Path) {
    let snapshot = std::sync::Arc::new(cineris_ext::folder::Snapshot::read(folder).unwrap());
    let timer = cineris_testkit::FakeTimer::manual(&cineris_testkit::FakeClock::new(
        cineris_base::UtcTime::UNIX_EPOCH,
    ));
    let (manifest, registration) = cineris_testkit::block_on(cineris_ext::engine::register(
        std::sync::Arc::clone(&snapshot),
        &timer,
    ))
    .unwrap();
    let prompt = manifest
        .prompt
        .as_deref()
        .and_then(|file| snapshot.file(file))
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    crate::extensions_found::cache_file(world)
        .put(
            &snapshot.hash(),
            cineris_ext::cache::Entry::new(&manifest, registration, prompt),
        )
        .unwrap();
}

/// The maintainers, on the full review of the signed approvals: nothing
/// reads a held entry as a grant. One alone, of the user's or of a
/// trusted project's, at the very hash found and with its registration and
/// prompt text in the cache, approves nothing, loads nothing, adds no
/// prompt text, is never shown as approved before, and holds its gate: a
/// held entry another program writes can only deny.
#[cfg(feature = "scripts")]
#[test]
fn a_held_entry_alone_approves_loads_and_adds_prompt_text_for_nothing() {
    const TEXT: &str = "Held prompt text, which must never reach the model.";
    for source in ["user", "project"] {
        let world = World::new(&format!("signed_held_grants_nothing_{source}"), looks());
        let folder = if source == "user" {
            crate::extensions_found::users(&world)
        } else {
            crate::extensions_found::projects(&world)
        };
        write_gated_with_prompt(&folder, TEXT);
        cache_with_prompt(&world, &folder);
        let probe = crate::extensions_found::definition(&folder);
        let (pins, trust) = stores(&world);
        let held = format!(
            "kind = \"extension\"\nname = \"probe\"\nhash = \"{}\"\n",
            probe.hash.to_hex()
        );
        if source == "user" {
            // Approvals that count, of something else, and `probe` held
            // beside them.
            crate::person().pin(
                &pins,
                &Definition::of(
                    cineris_protocol::methods::TrustResourceKind::Hook,
                    "elsewhere",
                    ["x"],
                ),
            );
            let mut text = std::fs::read_to_string(pins.path()).unwrap();
            text.push_str(&format!(
                "\n[[held]]\n{held}at = \"2026-11-13T10:00:00Z\"\n"
            ));
            std::fs::write(pins.path(), text).unwrap();
        } else {
            crate::extensions_found::trust(&world, &[]);
            let mut text = std::fs::read_to_string(trust.path()).unwrap();
            text.push_str(&format!("\n[[project.held]]\n{held}"));
            std::fs::write(trust.path(), text).unwrap();
        }
        let core = world.core();
        let (offered, outcome, text, _, _) = look_in(&world, &core);
        assert!(!offered, "{source}: the held entry loaded it");
        assert_eq!(outcome, "denied", "{source}: its gate holds: {text}");
        let systems: Vec<String> = world
            .provider
            .requests()
            .iter()
            .map(|request| request.system.clone())
            .collect();
        assert!(!systems.is_empty(), "{source}");
        assert!(
            systems.iter().all(|system| !system.contains(TEXT)),
            "{source}: its prompt text reached the model"
        );
        let project = core.project(&world.workspace).unwrap();
        let held_now = core.approvals_held(Some(&project)).unwrap();
        assert!(
            held_now.project_pins.is_empty(),
            "{source}: shown as approved before: {:?}",
            held_now.project_pins
        );
    }
}

/// The full review of the signed approvals, M3: a folder of the user's that
/// takes a project's extension's name released the project's gate, though
/// any program of the user's can make one. The name is the user's only
/// while their own extension of it is pinned with a signature that counts:
/// until then the project's extension is not used, and its gate holds.
#[cfg(feature = "scripts")]
#[test]
fn a_folder_of_the_users_that_takes_a_projects_name_releases_no_gate() {
    let world = World::new("signed_taken_name", looks_times(2));
    let theirs = crate::extensions_found::projects(&world);
    write_gated(&theirs);
    crate::extensions_found::cache(&world, &theirs);
    crate::extensions_found::trust(&world, &[crate::extensions_found::definition(&theirs)]);
    // A folder of the user's of the same name: an extension with no gate,
    // not approved.
    let mine = crate::extensions_found::users(&world);
    crate::extensions_found::write_extension(&mine, "", "Echoes.");
    crate::extensions_found::cache(&world, &mine);
    let core = world.core();
    let (_, outcome, text, _, notices) = look_in(&world, &core);
    assert_eq!(outcome, "denied", "the project's gate lapsed: {text}");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("not approved with your key")),
        "{notices:?}"
    );
    // The user's own, pinned with their key: the name is theirs, and the
    // project's gate goes with it.
    crate::extensions_found::pin(&world, &mine);
    let (offered, outcome, text, _, _) = look_in(&world, &core);
    assert!(offered, "the user's own is loaded");
    assert_eq!(outcome, "success", "{text}");
}
