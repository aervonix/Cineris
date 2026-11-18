//! Secrets and the session's files (`docs/security.md`, section 4;
//! `docs/architecture/07-sessions-and-context.md`, "Secrets in sessions").
//!
//! A credential that turns up in a session is masked before anything is
//! written; a provider's own block that holds one is never written at all;
//! and a session that is resumed after either does not send a provider
//! reasoning that no longer matches the history it was written against.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use cineris_base::{CancelToken, Secret};
use cineris_core::{ClientInfo, Connector, Core};
use cineris_llm::{
    ImageSource, ModelClient, ModelEvent, ModelRequest, ModelStream, ProviderError, ProviderInfo,
};
use cineris_protocol::event::TurnOutcome;
use cineris_protocol::methods::{Op, OpResult, SessionRef};
use cineris_protocol::{Message, Part, Role, StopReason};
use cineris_testkit::{Response, ScriptedProvider};
use serde_json::{Value, json};

use crate::{Front, World, calls, says};

/// The harness's own key in these tests. Made up.
const KEY: &str = "cineris-core-test-key-3d9f71ac";
/// A key in a well-known format. Made up.
const TOKEN: &str = "ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8";

fn with_key(world: &World) -> Core {
    world.core_with(|options| {
        options
            .api_keys
            .insert("anthropic".to_owned(), Secret::new(KEY));
    })
}

/// Whether any file the harness stored holds `secret`.
fn stored_anywhere(world: &World, secret: &str) -> Vec<String> {
    world
        .stored()
        .into_iter()
        .filter(|(_, content)| {
            content
                .windows(secret.len())
                .any(|window| window == secret.as_bytes())
        })
        .map(|(path, _)| path.display().to_string())
        .collect()
}

fn text_of(message: &Message) -> String {
    message
        .parts
        .iter()
        .map(|part| match part {
            Part::Text(text) => text.text.clone(),
            Part::ToolResult(result) => result
                .content
                .iter()
                .filter_map(|part| match part {
                    Part::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
                .collect(),
            _ => String::new(),
        })
        .collect()
}

fn has_raw(message: &Message) -> bool {
    message.parts.iter().any(|part| match part {
        Part::Text(text) => text.raw.is_some(),
        Part::ToolCall(call) => call.raw.is_some(),
        Part::Reasoning(reasoning) => reasoning.raw.is_some(),
        Part::ProviderBlock(_) => true,
        _ => false,
    })
}

fn resume(world: &World, core: &Core, session: &cineris_protocol::SessionId) -> Front {
    world.run(async {
        let client = core.connect(ClientInfo::user());
        let opened = core
            .handle(
                client.id,
                Op::SessionOpen(SessionRef {
                    session_id: session.clone(),
                }),
            )
            .await
            .expect("the session opens");
        let OpResult::SessionOpen(opened) = opened else {
            panic!("not a session");
        };
        Front {
            core: core.clone(),
            client,
            session: session.clone(),
            state: opened.session,
            seen: std::cell::RefCell::new(Vec::new()),
            answered: std::cell::RefCell::new(Vec::new()),
            answering: std::cell::RefCell::new(crate::Answering::default()),
        }
    })
}

#[test]
fn sys_the_harnesss_own_key_and_a_key_in_a_known_format_are_masked_on_disk() {
    let world = World::new(
        "mask",
        [
            calls(&[(
                "c1",
                "shell",
                // Quoted, so that every shell prints the two on one line:
                // PowerShell prints each argument of `echo` on its own.
                json!({"command": format!("echo \"{KEY} {TOKEN}\"")}),
            )]),
            says("I saw them."),
        ],
    );
    let core = with_key(&world);
    let session = world.run(async {
        let front = Front::start(&world, &core).await;
        front
            .turn_allowing(&format!("my key is {KEY}, do not lose it"))
            .await;
        // The model saw the output as it was: nothing is masked in the
        // conversation that is running.
        let request = world.provider.requests().pop().unwrap();
        assert!(text_of(&request.messages[0]).contains(KEY));
        assert!(text_of(&request.messages[2]).contains(KEY));
        assert!(text_of(&request.messages[2]).contains(TOKEN));
        // Front ends are shown it as it was too.
        assert!(front.results()[0].2.contains(KEY));
        let log = front.log(&world);
        front.close().await;

        // On disk it is masked, in the prompt and in the result, and each
        // entry that was changed says so.
        let user = log.iter().find(|line| line["kind"] == "user").unwrap();
        assert!(user.to_string().contains("my key is [redacted]"), "{user}");
        assert_eq!(user["redacted"], true);
        let result = log
            .iter()
            .find(|line| line["kind"] == "tool_result")
            .unwrap();
        assert_eq!(result["redacted"], true);
        assert!(
            result.to_string().contains("[redacted] [redacted]"),
            "{result}"
        );
        // An entry with nothing to mask has no mark.
        let last = log
            .iter()
            .rev()
            .find(|line| line["kind"] == "assistant")
            .unwrap();
        assert!(last.get("redacted").is_none(), "{last}");
        front.session.clone()
    });
    assert_eq!(stored_anywhere(&world, KEY), Vec::<String>::new());
    assert_eq!(stored_anywhere(&world, TOKEN), Vec::<String>::new());
    let _ = session;
}

#[test]
fn a_provider_block_that_holds_a_secret_is_never_written_and_the_live_session_keeps_it() {
    let world = World::new(
        "raw_block",
        [
            // The model repeats the key, in its text and in its reasoning:
            // both of the provider's own blocks hold it.
            Response::new()
                .reasoning(&format!("The user's key is {KEY}."))
                .text(&format!("Your key is {KEY}."))
                .end(StopReason::End),
            Response::new()
                .reasoning("Thinking about the next step.")
                .text("Second answer.")
                .end(StopReason::End),
            says("After the resume."),
        ],
    );
    let core = with_key(&world);
    let session = world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("what is my key?").await;
        // The user is told, and the notice does not hold the secret.
        let notices = front.notices();
        let notice = notices
            .iter()
            .find(|notice| notice.contains("stored without its provider blocks"))
            .unwrap_or_else(|| panic!("no notice in {notices:?}"));
        assert!(!notice.contains(KEY));

        // The session that is running still replays the message as the
        // provider sent it: raw blocks, and who produced them.
        front.turn("and then?").await;
        let request = world.provider.requests().pop().unwrap();
        let first = &request.messages[1];
        assert_eq!(first.role, Role::Assistant);
        assert!(has_raw(first), "the live session lost the raw blocks");
        assert!(first.origin.is_some());

        let log = front.log(&world);
        front.close().await;
        // On disk: the message without its provider blocks, marked, and
        // nothing that says a protocol may replay it.
        let stored: Vec<&Value> = log
            .iter()
            .filter(|line| line["kind"] == "assistant")
            .collect();
        assert_eq!(stored[0]["redacted"], true);
        assert!(!stored[0].to_string().contains("\"raw\""), "{}", stored[0]);
        assert!(stored[0]["item"].get("origin").is_none(), "{}", stored[0]);
        assert!(stored[0].to_string().contains("Your key is [redacted]."));
        // The message after it had nothing to hide, and is stored whole.
        assert!(stored[1].get("redacted").is_none());
        assert!(stored[1].to_string().contains("\"raw\""));
        front.session.clone()
    });
    assert_eq!(stored_anywhere(&world, KEY), Vec::<String>::new());

    // Resumed: the altered message is not replayable, and neither is the
    // reasoning that came after it, which was written against a history
    // the log no longer holds.
    let core = with_key(&world);
    let front = resume(&world, &core, &session);
    world.run(async {
        front.turn("go on").await;
        let request = world.provider.requests().pop().unwrap();
        let assistants: Vec<&Message> = request
            .messages
            .iter()
            .filter(|message| message.role == Role::Assistant)
            .collect();
        assert_eq!(assistants.len(), 2);
        for message in &assistants {
            assert!(
                !has_raw(message),
                "a raw block was replayed after the resume"
            );
            assert!(message.origin.is_none());
        }
        // The text is all there, masked where it was masked.
        assert_eq!(text_of(assistants[0]), "Your key is [redacted].");
        assert_eq!(text_of(assistants[1]), "Second answer.");
        // And the user is told, in a line.
        let notices = front.notices();
        assert!(
            notices
                .iter()
                .any(|notice| notice.contains("reasoning of 1 earlier response is not sent")),
            "{notices:?}"
        );
        front.close().await;
    });
}

/// A provider that ties each reasoning block to the history before it, as
/// Anthropic's current models do: a block sent back after anything before
/// it has changed is refused.
#[derive(Clone)]
struct Tying {
    script: ScriptedProvider,
    /// For each reasoning block seen, the history it was first sent with.
    written_against: Arc<Mutex<BTreeMap<String, String>>>,
    refused: Arc<Mutex<Vec<String>>>,
}

impl ModelClient for Tying {
    fn stream(&self, request: ModelRequest, cancel: CancelToken) -> ModelStream {
        let mut known = self.written_against.lock().unwrap();
        for (index, message) in request.messages.iter().enumerate() {
            for part in &message.parts {
                let Part::Reasoning(reasoning) = part else {
                    continue;
                };
                let Some(raw) = &reasoning.raw else { continue };
                let before = serde_json::to_string(&request.messages[..index]).unwrap();
                let written = known
                    .entry(raw.as_str().to_owned())
                    .or_insert_with(|| before.clone());
                if *written != before {
                    self.refused.lock().unwrap().push(raw.as_str().to_owned());
                    return ModelStream::from_events(vec![ModelEvent::Error(ProviderError::new(
                        cineris_base::ErrorKind::Provider,
                        "invalid_request_error",
                        "a reasoning block does not match the conversation before it",
                    ))]);
                }
            }
        }
        drop(known);
        self.script.stream(request, cancel)
    }

    fn stable_prefix(&self, request: &ModelRequest) -> Option<Vec<u8>> {
        self.script.stable_prefix(request)
    }
}

impl Connector for Tying {
    fn connect(
        &self,
        _provider: &ProviderInfo,
        _images: Arc<dyn ImageSource>,
    ) -> Option<Arc<dyn ModelClient>> {
        Some(Arc::new(self.clone()))
    }
}

/// The owner's rule for a resume, shown against a provider that would
/// refuse the request without it: a tool result is masked on disk, the
/// response after it has reasoning, and the session is resumed.
#[test]
fn a_resumed_session_does_not_replay_reasoning_against_a_history_redaction_changed() {
    let world = World::new(
        "tied_reasoning",
        [
            Response::new()
                .reasoning("First, read the notes.")
                .tool_call("c1", "read", &json!({"path": "notes.txt"}))
                .end(StopReason::ToolCalls),
            Response::new()
                .reasoning("The output held a key; I will not repeat it.")
                .text("Done.")
                .end(StopReason::End),
            Response::new()
                .reasoning("Continuing.")
                .text("Carried on.")
                .end(StopReason::End),
        ],
    );
    world.put("notes.txt", format!("deploy with the key {KEY}\n"));
    let tying = Tying {
        script: world.provider.clone(),
        written_against: Arc::default(),
        refused: Arc::default(),
    };
    let core = |world: &World| {
        let tying = tying.clone();
        world.core_with(move |options| {
            options
                .api_keys
                .insert("anthropic".to_owned(), Secret::new(KEY));
            options.connector = Some(Arc::new(tying));
        })
    };
    let first = core(&world);
    let session = world.run(async {
        let front = Front::start(&world, &first).await;
        front.turn_allowing("look around").await;
        assert_eq!(front.ends()[0].outcome, TurnOutcome::Completed);
        front.close().await;
        front.session.clone()
    });
    // The log holds the tool's result masked. (The file's own content is
    // kept exactly, in the checkpoint store, for changed-only re-reads:
    // that store is never redacted, because what it holds must be put
    // back, or compared, byte for byte.)
    assert!(!world.log().contains(KEY));
    assert!(world.log().contains("deploy with the key [redacted]"));

    let second = core(&world);
    let front = resume(&world, &second, &session);
    world.run(async {
        let end = front.turn("carry on").await;
        let refused = tying.refused.lock().unwrap().clone();
        assert_eq!(
            refused,
            Vec::<String>::new(),
            "a reasoning block was replayed against the masked history"
        );
        assert_eq!(end.outcome, TurnOutcome::Completed, "{:?}", end.error);
        assert_eq!(front.last_text(), "Carried on.");
        // What was before the change is replayed as it was: the first
        // response's reasoning was written before the tool's output.
        let request = world.provider.requests().pop().unwrap();
        let assistants: Vec<&Message> = request
            .messages
            .iter()
            .filter(|message| message.role == Role::Assistant)
            .collect();
        assert!(
            has_raw(assistants[0]),
            "reasoning from before the change was dropped too"
        );
        assert!(!has_raw(assistants[1]));
        front.close().await;
    });
}

/// A key of a second provider, and what one of its headers reads from the
/// environment. Made up, and in no known format: only their values can
/// give them away.
const OTHER_KEY: &str = "other-provider-key-7f3a9c1e";
const GATEWAY: &str = "gateway-pass-5b2d8e4f6a";

const OTHER_PROVIDER: &str = "[providers.other]
protocol = \"anthropic-messages\"
base_url = \"https://llm.example.invalid\"
key_env = \"OTHER_KEY\"

[providers.other.headers]
x-gateway = { env = \"GATEWAY_PASS\" }
x-team = \"2026-11-13\"

[models.elsewhere]
provider = \"other\"
id = \"elsewhere-model\"
name = \"Elsewhere\"
protocol = \"anthropic-messages\"
context_window = 100000
max_output = 8000
input = [\"text\"]
";

/// A private key, made up: what the filter takes for one.
const PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7\n-----END PRIVATE KEY-----\n";

/// The items of `front`'s session, read for an export or not.
async fn read_items(front: &Front, redacted: bool) -> cineris_protocol::methods::SessionReadResult {
    let result = front
        .op(Op::SessionRead(
            cineris_protocol::methods::SessionReadParams {
                session_id: front.session.clone(),
                item_id: None,
                limit: None,
                cursor: None,
                redacted,
            },
        ))
        .await
        .expect("the session reads");
    let OpResult::SessionRead(result) = result else {
        panic!("not items");
    };
    result
}

/// A session, logged or kept in memory only, whose prompt holds the
/// harness's key and whose tool read a private key and a token: what an
/// export reads of it (`session/read` with `redacted`, step 25) holds none
/// of them and no provider block, and says how many it masked; a plain
/// read is as it was.
fn an_export_read_of(ephemeral: bool) {
    let world = World::new(
        if ephemeral {
            "export_kept"
        } else {
            "export_logged"
        },
        [
            calls(&[("c1", "read", json!({"path": "secrets.txt"}))]),
            Response::new()
                .reasoning("Thinking it over.")
                .text("Done.")
                .end(StopReason::End),
        ],
    );
    world.put("secrets.txt", format!("{PEM}token {TOKEN}\n"));
    let core = with_key(&world);
    world.run(async {
        let front = if ephemeral {
            start_kept_in_memory(&world, &core).await
        } else {
            Front::start(&world, &core).await
        };
        front.turn_allowing(&format!("my key is {KEY}")).await;
        let plain = read_items(&front, false).await;
        let plain_text = serde_json::to_string(&plain.items).unwrap();
        assert!(
            plain_text.contains("\"raw\""),
            "the session's items keep the provider's blocks"
        );
        // A logged session reads back from its log, masked; one kept in
        // memory was never masked.
        assert_eq!(plain_text.contains(KEY), ephemeral);
        assert_eq!(plain.masked, 0);
        assert!(!plain.redacted, "a plain read does not say it masked");

        let export = read_items(&front, true).await;
        let text = serde_json::to_string(&export.items).unwrap();
        for secret in [KEY, TOKEN, "MIIEvQIBADANBgkqhkiG9w0BAQEFAASC"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
        assert!(!text.contains("\"raw\""), "{text}");
        assert!(text.contains("my key is [redacted]"), "{text}");
        assert_eq!(export.items.len(), plain.items.len());
        // The key, the private key and the token.
        assert_eq!(export.masked, 3);
        // It says it masked them: a client writes nothing without that.
        assert!(export.redacted);
        front.close().await;
    });
}

#[test]
fn an_export_read_of_a_logged_session_holds_no_secret_and_no_provider_block() {
    an_export_read_of(false);
}

#[test]
fn an_export_read_of_a_session_kept_in_memory_is_masked_the_same() {
    an_export_read_of(true);
}

/// A session kept in memory only (`--no-session`).
async fn start_kept_in_memory(world: &World, core: &Core) -> Front {
    let client = core.connect(ClientInfo::user());
    let open = core.handle(
        client.id,
        Op::SessionNew(cineris_protocol::methods::SessionNewParams {
            cwd: None,
            workspace: world.workspace.to_str().unwrap().to_owned(),
            settings: cineris_protocol::session::SessionSettings::default(),
            title: None,
            config: serde_json::Map::new(),
            ephemeral: true,
        }),
    );
    let (result, read) = crate::not_now(core, &client, open).await;
    let OpResult::SessionNew(result) = result.expect("the session starts") else {
        panic!("not the result of session/new");
    };
    Front {
        core: core.clone(),
        client,
        session: result.session.session_id.clone(),
        state: result.session,
        seen: std::cell::RefCell::new(read),
        answered: std::cell::RefCell::new(Vec::new()),
        answering: std::cell::RefCell::new(crate::Answering::default()),
    }
}

/// A session that comes to use another provider while it runs masks that
/// provider's key from then on, in its own log, and the log's filter is
/// told too.
#[test]
fn a_key_found_while_a_session_runs_is_masked_in_that_session() {
    let mut world = World::new(
        "late_key",
        [
            says("One."),
            calls(&[("c1", "read", json!({"path": "notes.txt"}))]),
            says("Read."),
        ],
    );
    world.user_config(OTHER_PROVIDER);
    world.env = world
        .env
        .clone()
        .with("OTHER_KEY", OTHER_KEY)
        .with("GATEWAY_PASS", GATEWAY);
    world.put(
        "notes.txt",
        format!("key {OTHER_KEY}\npass {GATEWAY}\ndate 2026-11-13\n"),
    );
    let told: Arc<Mutex<Vec<String>>> = Arc::default();
    let core = world.core_with(|options| {
        let told = Arc::clone(&told);
        options.key_found = Some(Arc::new(move |secret: &Secret| {
            told.lock().unwrap().push(secret.expose().to_owned());
        }));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn(&format!("one, and {OTHER_KEY}")).await;
        assert!(told.lock().unwrap().is_empty(), "no key was found yet");

        front
            .op(Op::SessionSet(
                cineris_protocol::methods::SessionSetParams {
                    session_id: front.session.clone(),
                    settings: cineris_protocol::session::SessionSettings {
                        roles: [(
                            cineris_protocol::model::ModelRole::Main,
                            cineris_protocol::session::RoleSettings {
                                model: Some(cineris_protocol::ModelId::new("elsewhere-model")),
                                ..Default::default()
                            },
                        )]
                        .into_iter()
                        .collect(),
                        ..Default::default()
                    },
                },
            ))
            .await
            .unwrap();
        // The log's filter is told each, once.
        let mut heard = told.lock().unwrap().clone();
        heard.sort();
        assert_eq!(heard, [GATEWAY, OTHER_KEY]);

        front.turn("read the notes").await;
        // The model was shown the file as it is.
        let request = world.provider.requests().pop().unwrap();
        assert!(
            request
                .messages
                .iter()
                .any(|m| text_of(m).contains(OTHER_KEY))
        );
        // An export masks the key by its value in the prompt written before
        // it was known (step 25).
        let export = read_items(&front, true).await;
        let text = serde_json::to_string(&export.items).unwrap();
        assert!(!text.contains(OTHER_KEY), "{text}");
        assert!(text.contains("one, and [redacted]"), "{text}");
        front.close().await;
    });
    let log = world.log();
    // The prompt was written before the key was known: the log holds it
    // there, and only there.
    assert_eq!(log.matches(OTHER_KEY).count(), 1, "{log}");
    assert!(log.contains("one, and other-provider-key"), "{log}");
    assert!(!log.contains(GATEWAY), "the header's secret is in the log");
    assert!(log.contains("key [redacted]"), "{log}");
    assert!(log.contains("pass [redacted]"), "{log}");
    // A header written out in the configuration file is not taken for a
    // secret: masking it would take a date out of every session.
    assert!(log.contains("date 2026-11-13"), "{log}");
}
