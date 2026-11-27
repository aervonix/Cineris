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
#[cfg(feature = "scripts")]
use cineris_policy::Definition;
use cineris_policy::{PinStore, TrustStore};
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
    core.trust_project_with(&project, &[], &hooks, &crate::person_key())
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
    let (pins, _) = stores(&world);
    let person = crate::person();
    person.pin_all(
        &pins,
        &[
            crate::extensions_found::definition(&folder),
            Definition::of(
                cineris_protocol::methods::TrustResourceKind::Extension,
                "other",
                ["x"],
            ),
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
    assert!(!held.to_use());
}
