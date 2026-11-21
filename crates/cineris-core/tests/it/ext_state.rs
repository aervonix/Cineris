//! What an extension keeps, end to end (step 28): `cineris.state`'s
//! `session` values in the session's log, through a resume, a fork and a
//! rewind; its `project` values in the state directory, through the next
//! session of the project and not another project's; each kept apart by
//! where the extension comes from and its name, held to its caps, and
//! never a question. The extension is approved by the test and handed in
//! through `CoreOptions::extensions`, as in `extensions`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cineris_core::ext::ApprovedExtension;
use cineris_core::{ClientInfo, Core};
use cineris_ext::folder::Snapshot;
use cineris_ext::source::Source;
use cineris_protocol::methods::{
    Op, OpResult, SessionForkParams, SessionNewParams, SessionRef, SessionRewindParams,
};
use cineris_protocol::session::SessionSettings;
use cineris_protocol::{Item, SessionId};
use cineris_testkit::{FakeClock, FakeTimer, Response, block_on};
use serde_json::{Value, json};

use crate::{Front, World, calls, not_now, says};

/// The extension's one tool: each op of `ops` in turn, and what came of
/// it. `{"get": key}` gives `got <JSON>`; `{"set": key, "value": v}` (or
/// `"fill": n`, a text of `n` x's) gives `kept`; `{"len": key}` gives the
/// length of a text kept; `{"repeat": n, "key": key}` sets the key to 0, 1,
/// … `n - 1` and says how far it got. Each may name a `scope`; anything
/// refused gives `refused: ` and the reason.
const SCRIPT: &str = r#"
cineris.tool({ name: 'state', input: { type: 'object' }, run: async (args) => {
  const out = [];
  for (const op of args.ops) {
    const scope = op.scope === undefined ? undefined : { scope: op.scope };
    try {
      if (op.get !== undefined) {
        out.push('got ' + JSON.stringify(await cineris.state.get(op.get, scope)));
      } else if (op.len !== undefined) {
        const value = await cineris.state.get(op.len, scope);
        out.push('length ' + (value === null ? 'none' : value.length));
      } else if (op.repeat !== undefined) {
        let i = 0;
        try {
          for (; i < op.repeat; i++) await cineris.state.set(op.key, i, scope);
          out.push('kept ' + i);
        } catch (e) { out.push('refused at ' + i + ': ' + e.message); }
      } else {
        const value = op.fill === undefined ? op.value : 'x'.repeat(op.fill);
        await cineris.state.set(op.set, value, scope);
        out.push('kept');
      }
    } catch (e) { out.push('refused: ' + e.message); }
  }
  return out;
} });
"#;

/// Writes the extension `probe` in `folder` and approves it as it is, as
/// coming from `source`.
fn approve(folder: &Path, source: Source) -> ApprovedExtension {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"Keeps values\"\nmain = \
         \"main.js\"\n",
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), SCRIPT).unwrap();
    let snapshot = Arc::new(Snapshot::read(folder).unwrap());
    let timer = FakeTimer::manual(&FakeClock::new(cineris_base::UtcTime::UNIX_EPOCH));
    let (manifest, registration) =
        block_on(cineris_ext::engine::register(Arc::clone(&snapshot), &timer)).unwrap();
    ApprovedExtension {
        source,
        folder: folder.to_path_buf(),
        hash: snapshot.hash(),
        manifest,
        registration,
    }
}

/// The user's own `probe`.
fn users(world: &World) -> ApprovedExtension {
    approve(&world.dir.join("user-ext"), Source::User)
}

/// A core whose sessions have `extension`.
fn core_with(world: &World, extension: &ApprovedExtension) -> Core {
    let extension = extension.clone();
    world.core_with(move |options| options.extensions = vec![extension])
}

/// A response that calls the tool with `ops`.
fn ops(id: &str, ops: Value) -> Response {
    calls(&[(
        id,
        "tool_call",
        json!({"tool": "probe/state", "arguments": {"ops": ops}}),
    )])
}

/// Runs a turn, allowing the tool's call, and gives what came of each op of
/// its last call. Nothing but the call of the tool is asked about: `state`
/// puts no question.
async fn turn(front: &Front, text: &str) -> Vec<String> {
    let asked = front.turn_allowing(text).await;
    assert!(
        asked.iter().all(|tool| tool == "tool_call"),
        "only the tool's call is asked about: {asked:?}"
    );
    let results = front.results();
    let (_, outcome, text) = results.last().expect("the tool ran");
    assert_eq!(*outcome, "success", "{text}");
    let line = text
        .lines()
        .find(|line| line.starts_with('['))
        .unwrap_or_else(|| panic!("no outcomes in {text}"));
    serde_json::from_str(line).unwrap()
}

/// A user's front end on the stored session `session`, opened by `core`.
async fn reopen(core: &Core, session: &SessionId) -> Front {
    let client = core.connect(ClientInfo::user());
    let open = core.handle(
        client.id,
        Op::SessionOpen(SessionRef {
            session_id: session.clone(),
        }),
    );
    let (result, read) = not_now(core, &client, open).await;
    let OpResult::SessionOpen(result) = result.expect("the session opens") else {
        panic!("not the result of session/open");
    };
    Front {
        core: core.clone(),
        client,
        session: session.clone(),
        state: result.session,
        seen: std::cell::RefCell::new(read),
        answered: std::cell::RefCell::default(),
        answering: std::cell::RefCell::default(),
    }
}

/// A user's front end on a new session in `workspace`, another project's.
async fn start_in(core: &Core, workspace: &Path) -> Front {
    let client = core.connect(ClientInfo::user());
    let open = core.handle(
        client.id,
        Op::SessionNew(SessionNewParams {
            cwd: None,
            workspace: workspace.to_str().unwrap().to_owned(),
            settings: SessionSettings::default(),
            title: None,
            config: serde_json::Map::new(),
            ephemeral: false,
        }),
    );
    let (result, read) = not_now(core, &client, open).await;
    let OpResult::SessionNew(result) = result.expect("the session starts") else {
        panic!("not the result of session/new");
    };
    Front {
        core: core.clone(),
        client,
        session: result.session.session_id.clone(),
        state: result.session,
        seen: std::cell::RefCell::new(read),
        answered: std::cell::RefCell::default(),
        answering: std::cell::RefCell::default(),
    }
}

/// The `ext` records of the session's log.
fn records(front: &Front, world: &World) -> Vec<Value> {
    front
        .log(world)
        .into_iter()
        .filter(|entry| entry["kind"] == "ext")
        .collect()
}

/// Every project's file of values in the state directory.
fn project_files(world: &World) -> Vec<PathBuf> {
    let projects = world.dirs().state.unwrap().join("projects");
    let mut found = Vec::new();
    for project in std::fs::read_dir(projects).into_iter().flatten().flatten() {
        let folder = project.path().join("extensions");
        for file in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            if file.path().extension().is_some_and(|end| end == "json") {
                found.push(file.path());
            }
        }
    }
    found.sort();
    found
}

/// The id of the result of the call `call`.
fn result_of(front: &Front, call: &str) -> cineris_protocol::ids::ItemId {
    front
        .items()
        .iter()
        .find_map(|item| match item {
            Item::ToolResult(result) if result.call_id.as_str() == call => Some(result.id.clone()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn a_session_value_is_read_back_in_the_session_and_a_set_to_null_removes_it() {
    let world = World::new(
        "ext_state_session",
        [
            ops(
                "c1",
                json!([
                    {"get": "n"},
                    {"set": "n", "value": {"a": 1}},
                    {"get": "n"},
                    {"get": "other"},
                    {"set": "n", "value": null},
                    {"get": "n"},
                    {"set": "n", "value": 2},
                    {"set": "n", "value": 2},
                    {"set": "gone"},
                ]),
            ),
            says("Done."),
        ],
    );
    let core = core_with(&world, &users(&world));
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(
            turn(&front, "keep something").await,
            [
                "got null",
                "kept",
                r#"got {"a":1}"#,
                "got null",
                "kept",
                "got null",
                "kept",
                "kept",
                "kept",
            ]
        );
        // One record a change, of the extension's own key; a set that
        // changed nothing wrote nothing.
        let records = records(&front, &world);
        assert_eq!(records.len(), 3, "{records:?}");
        for record in &records {
            assert_eq!(record["ext"], "user/probe");
            assert_eq!(record["data"]["state"]["key"], "n");
            assert!(record["parent"].is_null(), "a record: {record}");
        }
        assert_eq!(records[1]["data"]["state"]["value"], Value::Null);
        assert_eq!(records[2]["data"]["state"]["value"], 2);
        front.close().await;
    });
}

#[test]
fn a_resumed_session_has_its_values_as_it_left_them() {
    let world = World::new(
        "ext_state_resume",
        [
            ops(
                "c1",
                json!([
                    {"set": "n", "value": 1},
                    {"set": "n", "value": 2},
                    {"set": "gone", "value": 1},
                    {"set": "gone", "value": null},
                ]),
            ),
            says("Kept."),
            ops("c2", json!([{"get": "n"}, {"get": "gone"}])),
            says("Read."),
        ],
    );
    let extension = users(&world);
    let core = core_with(&world, &extension);
    let session = world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(turn(&front, "keep").await, ["kept"; 4]);
        front.close().await;
        front.session.clone()
    });
    let core = core_with(&world, &extension);
    world.run(async {
        let front = reopen(&core, &session).await;
        assert_eq!(turn(&front, "read").await, ["got 2", "got null"]);
        front.close().await;
    });
}

#[test]
fn a_fork_has_the_values_as_they_were_at_its_point_and_neither_sees_the_others_sets() {
    let world = World::new(
        "ext_state_fork",
        [
            ops("c1", json!([{"set": "n", "value": 1}])),
            says("One."),
            ops("c2", json!([{"set": "n", "value": 2}])),
            says("Two."),
            ops(
                "c3",
                json!([{"get": "n"}, {"set": "n", "value": 3}, {"get": "n"}]),
            ),
            says("In the fork."),
            ops("c4", json!([{"get": "n"}])),
            says("In the original."),
        ],
    );
    let core = core_with(&world, &users(&world));
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(turn(&front, "one").await, ["kept"]);
        assert_eq!(turn(&front, "two").await, ["kept"]);
        let forked = front
            .op(Op::SessionFork(SessionForkParams {
                session_id: front.session.clone(),
                at: Some(result_of(&front, "c1")),
                before: None,
            }))
            .await
            .unwrap();
        let OpResult::SessionFork(forked) = forked else {
            panic!("not a fork");
        };
        let on_fork = Front {
            session: forked.session.session_id.clone(),
            state: forked.session,
            seen: std::cell::RefCell::default(),
            answered: std::cell::RefCell::default(),
            answering: std::cell::RefCell::default(),
            core: front.core.clone(),
            client: front.client.clone(),
        };
        assert_eq!(
            turn(&on_fork, "in the fork").await,
            ["got 1", "kept", "got 3"],
            "the value as it was at the first call's result"
        );
        on_fork.close().await;
        assert_eq!(
            turn(&front, "in the original").await,
            ["got 2"],
            "the fork's set is the fork's"
        );
        front.close().await;
    });
}

/// A fork is a session of its own, stored: closed and opened again, it has
/// the values as they were at the fork's point, which its log was given
/// when the fork was made.
#[test]
fn a_resumed_fork_has_the_values_as_they_were_at_the_fork() {
    let world = World::new(
        "ext_state_fork_resume",
        [
            ops("c1", json!([{"set": "n", "value": 1}])),
            says("One."),
            ops(
                "c2",
                json!([{"set": "n", "value": 2}, {"set": "m", "value": 1}]),
            ),
            says("Two."),
            ops("c3", json!([{"get": "n"}, {"get": "m"}])),
            says("In the fork, again."),
        ],
    );
    let extension = users(&world);
    let core = core_with(&world, &extension);
    let fork = world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(turn(&front, "one").await, ["kept"]);
        assert_eq!(turn(&front, "two").await, ["kept", "kept"]);
        let forked = front
            .op(Op::SessionFork(SessionForkParams {
                session_id: front.session.clone(),
                at: Some(result_of(&front, "c1")),
                before: None,
            }))
            .await
            .unwrap();
        let OpResult::SessionFork(forked) = forked else {
            panic!("not a fork");
        };
        let fork = forked.session.session_id.clone();
        front
            .op(Op::SessionClose(SessionRef {
                session_id: fork.clone(),
            }))
            .await
            .unwrap();
        front.close().await;
        fork
    });
    let core = core_with(&world, &extension);
    world.run(async {
        let front = reopen(&core, &fork).await;
        assert_eq!(
            turn(&front, "in the fork, again").await,
            ["got 1", "got null"],
            "the values as they were at the first call's result"
        );
        front.close().await;
    });
}

/// What an extension's `session` records may hold in one session's log
/// (16 MiB; here a test's 4 KiB, through `CoreOptions`): at it a set is
/// kept, one byte past it a set is refused and nothing is written, and a
/// resume counts what the log already holds. A record counts its `ext` and
/// `data` as compact JSON and 128 bytes for the rest of its line.
#[test]
fn a_sessions_records_are_held_to_their_budget_at_its_edge_and_after_a_resume() {
    const BUDGET: usize = 4096;
    let counted = |length: usize| {
        let data = json!({"state": {"key": "k", "value": "x".repeat(length)}});
        serde_json::to_vec(&json!({"ext": "user/probe", "data": data}))
            .unwrap()
            .len()
            + 128
    };
    let first = 1000;
    // The second record takes them to the budget exactly.
    let second = BUDGET - counted(first) - counted(0);
    assert_eq!(counted(first) + counted(second), BUDGET);
    let world = World::new(
        "ext_state_session_budget",
        [
            ops(
                "c1",
                json!([
                    {"set": "k", "fill": first},
                    {"set": "k", "fill": second + 1},
                    {"len": "k"},
                    {"set": "k", "fill": second},
                    {"set": "k", "fill": 1},
                    {"set": "other", "value": 1},
                    {"set": "k"},
                    {"len": "k"},
                    {"set": "k", "fill": second},
                    {"set": "p", "value": 1, "scope": "project"},
                ]),
            ),
            says("Kept."),
            ops(
                "c2",
                json!([{"set": "k", "fill": 2}, {"len": "k"}, {"get": "other"}]),
            ),
            says("Resumed."),
        ],
    );
    let extension = users(&world);
    let budgeted = |extension: &ApprovedExtension| {
        let extension = extension.clone();
        world.core_with(move |options| {
            options.extensions = vec![extension];
            options.ext_session_budget = Some(BUDGET);
        })
    };
    let refused = |outcome: &str| {
        outcome.starts_with("refused: ")
            && outcome.contains(&format!(
                "more than the {BUDGET} bytes it may write to one session"
            ))
    };
    let core = budgeted(&extension);
    let session = world.run(async {
        let front = Front::start(&world, &core).await;
        let outcomes = turn(&front, "keep").await;
        assert_eq!(outcomes[0], "kept", "{outcomes:?}");
        assert!(refused(&outcomes[1]), "one byte past: {outcomes:?}");
        assert_eq!(outcomes[2], format!("length {first}"), "nothing was kept");
        assert_eq!(outcomes[3], "kept", "at the budget: {outcomes:?}");
        for past in &outcomes[4..7] {
            assert!(refused(past), "past it, a removal too: {outcomes:?}");
        }
        assert_eq!(outcomes[7], format!("length {second}"));
        // A set that changes nothing writes nothing, and is not refused;
        // the project's values have no log.
        assert_eq!(outcomes[8..], ["kept", "kept"]);
        assert_eq!(records(&front, &world).len(), 2);
        front.close().await;
        front.session.clone()
    });
    let core = budgeted(&extension);
    world.run(async {
        let front = reopen(&core, &session).await;
        let outcomes = turn(&front, "resumed").await;
        assert!(
            refused(&outcomes[0]),
            "the log still holds it: {outcomes:?}"
        );
        assert_eq!(
            outcomes[1..],
            [format!("length {second}"), "got null".to_owned()]
        );
        assert_eq!(records(&front, &world).len(), 2);
        front.close().await;
    });
}

/// A record belongs to the session, not to a branch: a rewind leaves an
/// extension's values as they are, as it leaves the lease.
#[test]
fn a_rewind_leaves_the_session_values_as_they_are() {
    let world = World::new(
        "ext_state_rewind",
        [
            ops("c1", json!([{"set": "n", "value": 1}])),
            says("One."),
            ops("c2", json!([{"set": "n", "value": 2}])),
            says("Two."),
            ops("c3", json!([{"get": "n"}])),
            says("Read."),
        ],
    );
    let core = core_with(&world, &users(&world));
    world.run(async {
        let front = Front::start(&world, &core).await;
        turn(&front, "one").await;
        let keep = front
            .items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::AssistantMessage(message) => Some(message.id.clone()),
                _ => None,
            })
            .unwrap();
        turn(&front, "two").await;
        front
            .op(Op::SessionRewind(SessionRewindParams {
                session_id: front.session.clone(),
                to: Some(keep),
                before: None,
                restore_files: false,
                plan: None,
            }))
            .await
            .unwrap();
        assert_eq!(turn(&front, "read").await, ["got 2"]);
        front.close().await;
    });
}

#[test]
fn a_project_value_is_seen_by_the_next_session_of_the_project() {
    let world = World::new(
        "ext_state_project",
        [
            ops(
                "c1",
                json!([
                    {"set": "p", "value": [1, "two"], "scope": "project"},
                    {"get": "p", "scope": "project"},
                    {"get": "p"},
                ]),
            ),
            says("Kept."),
            ops("c2", json!([{"get": "p", "scope": "project"}])),
            says("Read."),
        ],
    );
    let extension = users(&world);
    let core = core_with(&world, &extension);
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(
            turn(&front, "keep").await,
            ["kept", r#"got [1,"two"]"#, "got null"],
            "the session's values are another store"
        );
        front.close().await;
    });
    assert_eq!(project_files(&world).len(), 1);
    let core = core_with(&world, &extension);
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(turn(&front, "read").await, [r#"got [1,"two"]"#]);
        front.close().await;
    });
}

#[test]
fn a_project_value_is_not_seen_by_another_project() {
    let world = World::new(
        "ext_state_other_project",
        [
            ops("c1", json!([{"set": "p", "value": 1, "scope": "project"}])),
            says("Kept."),
            ops(
                "c2",
                json!([
                    {"get": "p", "scope": "project"},
                    {"set": "p", "value": 2, "scope": "project"},
                ]),
            ),
            says("Elsewhere."),
            ops("c3", json!([{"get": "p", "scope": "project"}])),
            says("Back."),
        ],
    );
    let other = world.dir.join("work").join("other");
    std::fs::create_dir_all(&other).unwrap();
    let core = core_with(&world, &users(&world));
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(turn(&front, "keep").await, ["kept"]);
        front.close().await;
        let elsewhere = start_in(&core, &other).await;
        assert_eq!(turn(&elsewhere, "elsewhere").await, ["got null", "kept"]);
        elsewhere.close().await;
        let back = Front::start(&world, &core).await;
        assert_eq!(turn(&back, "back").await, ["got 1"]);
        back.close().await;
    });
    // One file for each project.
    let files = project_files(&world);
    assert_eq!(files.len(), 2, "{files:?}");
    assert_ne!(files[0].parent(), files[1].parent());
}

/// A project's `probe` and the user's `probe` are two extensions: neither
/// reads or writes the other's values, in either scope. The two never run
/// in one session (a project's extension of the user's name is not used),
/// so the session is opened with one and then the other.
#[test]
fn the_users_and_the_projects_extension_of_one_name_never_see_each_others_values() {
    let world = World::new(
        "ext_state_sources",
        [
            ops(
                "c1",
                json!([
                    {"set": "s", "value": "user's"},
                    {"set": "p", "value": "user's", "scope": "project"},
                ]),
            ),
            says("The user's."),
            ops(
                "c2",
                json!([
                    {"get": "s"},
                    {"get": "p", "scope": "project"},
                    {"set": "s", "value": "project's"},
                    {"set": "p", "value": "project's", "scope": "project"},
                ]),
            ),
            says("The project's."),
            ops(
                "c3",
                json!([{"get": "s"}, {"get": "p", "scope": "project"}]),
            ),
            says("The user's again."),
        ],
    );
    let user = users(&world);
    // Where its folder is decides nothing here: the session is handed
    // the approved extension, with its source.
    let project = approve(&world.dir.join("project-ext"), Source::Project);
    let core = core_with(&world, &user);
    let session = world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(turn(&front, "the user's").await, ["kept", "kept"]);
        front.close().await;
        front.session.clone()
    });
    let core = core_with(&world, &project);
    world.run(async {
        let front = reopen(&core, &session).await;
        assert_eq!(
            turn(&front, "the project's").await,
            ["got null", "got null", "kept", "kept"]
        );
        front.close().await;
    });
    let core = core_with(&world, &user);
    world.run(async {
        let front = reopen(&core, &session).await;
        assert_eq!(
            turn(&front, "the user's again").await,
            [r#"got "user's""#, r#"got "user's""#]
        );
        front.close().await;
    });
    let names: Vec<String> = project_files(&world)
        .iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["project-probe.json", "user-probe.json"]);
}

/// 64 KiB of values an extension a scope, as the compact JSON of them all:
/// at it they are kept, one byte past it a set is refused and nothing is
/// written; and the two scopes are counted apart.
#[test]
fn the_values_of_a_scope_are_kept_to_64_kib_exactly() {
    // `{"k":"…"}` is 8 bytes and the text.
    let edge = 64 * 1024 - 8;
    let at_the_edge = |scope: &str| {
        json!([
            {"set": "k", "fill": edge, "scope": scope},
            {"len": "k", "scope": scope},
            {"set": "n", "value": 1, "scope": scope},
            {"get": "n", "scope": scope},
            {"set": "k", "fill": edge + 1, "scope": scope},
            {"len": "k", "scope": scope},
        ])
    };
    let world = World::new(
        "ext_state_cap",
        [
            ops("c1", at_the_edge("session")),
            says("The session's."),
            ops("c2", at_the_edge("project")),
            says("The project's."),
        ],
    );
    let core = core_with(&world, &users(&world));
    world.run(async {
        let front = Front::start(&world, &core).await;
        for (text, records_after) in [("the session's", 1), ("the project's", 1)] {
            let outcomes = turn(&front, text).await;
            assert_eq!(outcomes[0], "kept", "{outcomes:?}");
            assert_eq!(outcomes[1], format!("length {edge}"));
            for refused in [&outcomes[2], &outcomes[4]] {
                assert!(
                    refused.starts_with("refused: ") && refused.contains("64 KiB"),
                    "{outcomes:?}"
                );
            }
            assert_eq!(outcomes[3], "got null", "nothing was kept");
            assert_eq!(outcomes[5], format!("length {edge}"), "nothing was kept");
            // Only the set that was kept is in the log: the project's
            // values are not.
            assert_eq!(records(&front, &world).len(), records_after);
        }
        front.close().await;
    });
    let file = &project_files(&world)[0];
    let kept: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(kept["version"], 1);
    assert_eq!(
        kept["values"]["k"].as_str().map(str::len),
        Some(edge),
        "the file holds what was kept"
    );
    assert!(kept["values"].get("n").is_none());
}

/// A call sets at most a hundred values: each `session` one is a line of
/// the log. The next call may set again.
#[test]
fn a_call_sets_at_most_a_hundred_values() {
    let world = World::new(
        "ext_state_count",
        [
            calls(&[
                (
                    "c1",
                    "tool_call",
                    json!({"tool": "probe/state", "arguments": {"ops": [{"repeat": 101, "key": "n"}, {"get": "n"}]}}),
                ),
                (
                    "c2",
                    "tool_call",
                    json!({"tool": "probe/state", "arguments": {"ops": [{"set": "n", "value": "again"}]}}),
                ),
            ]),
            says("Done."),
        ],
    );
    let core = core_with(&world, &users(&world));
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(turn(&front, "count").await, ["kept"]);
        let first = front.results()[0].2.clone();
        assert!(
            first.contains("refused at 100: ") && first.contains("at most 100"),
            "{first}"
        );
        assert!(first.contains("got 99"), "{first}");
        assert_eq!(records(&front, &world).len(), 101);
        front.close().await;
    });
}

/// A project's file is read strictly: one this build did not write is
/// refused with the reason, the call that would read or change it fails,
/// and the file is left as it is.
#[test]
fn a_project_file_this_build_did_not_write_is_refused_and_left_as_it_is() {
    let both = || {
        json!([
            {"get": "p", "scope": "project"},
            {"set": "p", "value": 2, "scope": "project"},
        ])
    };
    let world = World::new(
        "ext_state_strict",
        [
            ops("c1", json!([{"set": "p", "value": 1, "scope": "project"}])),
            says("Kept."),
            ops("c2", both()),
            says("Unknown."),
            ops("c3", both()),
            says("Newer."),
            ops("c4", both()),
            says("Broken."),
            ops("c5", both()),
            says("Too large."),
        ],
    );
    // Well formed, and larger than any file this build writes.
    let large = format!(
        r#"{{"version":1,"values":{{"p":"{}"}}}}"#,
        "x".repeat(1024 * 1024)
    );
    let core = core_with(&world, &users(&world));
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(turn(&front, "keep").await, ["kept"]);
        let file = project_files(&world).remove(0);
        for (text, written, reason) in [
            (
                "unknown",
                r#"{"version":1,"values":{"p":1},"more":true}"#,
                "a field this build does not know, `more`",
            ),
            (
                "newer",
                r#"{"version":2,"values":{"p":1}}"#,
                "written by a newer version of Cineris",
            ),
            ("broken", r#"{"version":1,"values":{"p":1"#, "is not JSON"),
            ("too large", large.as_str(), "is larger than the 1024 KiB"),
        ] {
            std::fs::write(&file, written).unwrap();
            let outcomes = turn(&front, text).await;
            assert_eq!(outcomes.len(), 2);
            for outcome in &outcomes {
                assert!(
                    outcome.starts_with("refused: ")
                        && outcome.contains(reason)
                        && outcome.contains("left as it is"),
                    "{outcome}"
                );
            }
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                written,
                "the file is not written over"
            );
        }
        front.close().await;
    });
}

/// A project's file is the user's alone, written whole: no other user can
/// open it, and nothing half-written is left beside it. On Linux its mode
/// is `0600` and its folder's `0700`; on Windows its access list allows the
/// user alone and takes nothing from its folder, which a file written
/// without being made owner-only would (`cineris_base::private::is_private`
/// reads either).
#[test]
fn a_project_file_is_written_owner_only() {
    let world = World::new(
        "ext_state_private",
        [
            ops("c1", json!([{"set": "p", "value": 1, "scope": "project"}])),
            says("Kept."),
        ],
    );
    let core = core_with(&world, &users(&world));
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(turn(&front, "keep").await, ["kept"]);
        front.close().await;
    });
    let file = project_files(&world).remove(0);
    let folder = file.parent().unwrap();
    for path in [&file, folder, &cineris_base::private::lock_path(&file)] {
        assert!(
            cineris_base::private::is_private(path).unwrap(),
            "{} is open to other users",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(folder), 0o700);
    }
    let mut beside: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    beside.sort();
    assert_eq!(beside, ["user-probe.json", "user-probe.json.lock"]);
}
