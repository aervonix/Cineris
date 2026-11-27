//! Finding a server's era: the probe, the fallback to `initialize`, a
//! server that exits on the probe, and the start's limit.

use cineris_base::CancelToken;
use cineris_mcp::fake::{Discover, Gate};
use cineris_mcp::{Client, ClientError, Era, LineError, ServerInfo, ServerPipes};
use cineris_testkit::join;
use std::sync::Mutex;
use tokio::sync::{mpsc, watch};

use super::*;

#[test]
fn a_modern_server_is_found_by_the_probe_and_is_never_sent_initialize() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern();
        let client = connect(&server, &timer).await;
        assert_eq!(
            client.info(),
            &ServerInfo {
                era: Era::Modern,
                version: "2026-07-28".to_owned(),
                name: Some("fake-modern".to_owned()),
                list_changed: false,
            }
        );
        assert_eq!(server.methods(), ["server/discover"]);
        let meta = &server.of("server/discover")[0]["params"]["_meta"];
        assert_eq!(
            meta["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert_eq!(
            meta["io.modelcontextprotocol/clientCapabilities"],
            json!({})
        );
        assert_eq!(server.spawns(), 1);
    });
}

#[test]
fn a_legacy_server_that_refuses_the_probe_with_any_error_is_initialized_on_the_same_process() {
    // The fallback is not keyed to one code: legacy servers answer an
    // unknown request before `initialize` as they like.
    for code in [-32601, -32602, -32600, -32000, -32700] {
        run(async {
            let timer = timer();
            let server = FakeServer::legacy("2025-06-18").on_discover(Discover::Error(code));
            let client = connect(&server, &timer).await;
            assert_eq!(
                client.info(),
                &ServerInfo {
                    era: Era::Legacy,
                    version: "2025-06-18".to_owned(),
                    name: Some("fake-legacy".to_owned()),
                    list_changed: false,
                },
                "{code}"
            );
            server.wait_for("notifications/initialized", 1).await;
            assert_eq!(
                server.methods(),
                ["server/discover", "initialize", "notifications/initialized"],
                "{code}"
            );
            assert_eq!(server.spawns(), 1, "{code}");
            let params = &server.of("initialize")[0]["params"];
            assert_eq!(
                params["protocolVersion"], "2025-11-25",
                "the newest legacy revision"
            );
            assert_eq!(params["capabilities"], json!({}));
            assert_eq!(params["clientInfo"]["name"], "cineris");
        });
    }
}

#[test]
fn a_silent_server_is_taken_as_legacy_after_the_probes_time_and_a_late_answer_is_ignored() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-11-25")
            .on_discover(Discover::Silent)
            .tools(vec![tool("a")]);
        let server_definition = definition();
        let connecting = StdioClient::connect(
            &server_definition,
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        );
        let (client, ()) = join(connecting, async {
            server.wait_for("server/discover", 1).await;
            timer.sleeping_for(PROBE).await;
            settle().await;
            assert!(
                server.of("initialize").is_empty(),
                "not before the probe's time"
            );
            timer.advance(PROBE);
        })
        .await;
        let client = client.unwrap();
        assert_eq!(client.info().era, Era::Legacy);
        assert_eq!(server.spawns(), 1);
        assert!(
            server.of("notifications/cancelled").is_empty(),
            "the probe is not cancelled: a legacy server would not know it"
        );

        // The probe's answer, after the fallback, changes nothing.
        let probe = id_of(&server, "server/discover", 0);
        server
            .send(json!({"jsonrpc": "2.0", "id": probe, "result": {"supportedVersions": ["2026-07-28"]}}))
            .await;
        let tools = client.list_tools(&CancelToken::new()).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(client.info().era, Era::Legacy);
    });
}

#[test]
fn a_server_that_exits_on_the_probe_is_started_again_and_only_initialized() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-03-26").exits_before_initialize();
        let client = connect(&server, &timer).await;
        assert_eq!(client.info().era, Era::Legacy);
        assert_eq!(client.info().version, "2025-03-26");
        assert_eq!(server.spawns(), 2);
        server.wait_for("notifications/initialized", 1).await;
        let methods = |number| -> Vec<Json> {
            server
                .received_by(number)
                .iter()
                .map(|message| message["method"].clone())
                .collect()
        };
        assert_eq!(methods(1), ["server/discover"]);
        assert_eq!(methods(2), ["initialize", "notifications/initialized"]);
        // The first process is ended.
        until(|| server.shutdowns() == [GRACE]).await;
    });
}

#[test]
fn a_server_that_refuses_the_probe_and_then_exits_is_started_again() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").on_discover(Discover::ErrorThenExit(-32601));
        let client = connect(&server, &timer).await;
        assert_eq!(client.info().era, Era::Legacy);
        assert_eq!(server.spawns(), 2);
        server.wait_for("notifications/initialized", 1).await;
        assert_eq!(server.received_by(2)[0]["method"], "initialize");
    });
}

#[test]
fn a_server_that_cannot_be_started_again_ends_the_start_with_why() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").exits_before_initialize();
        let result = StdioClient::connect(
            &definition(),
            server.spawn(),
            Arc::new(|| {
                Err(cineris_mcp::SpawnError::NotFound(
                    "no such program".to_owned(),
                ))
            }),
            shared(&timer),
            ClientOptions::default(),
        )
        .await;
        let Err(ClientError::NotStarted(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("no such program"), "{why}");
    });
}

#[test]
fn a_server_that_does_not_speak_the_revision_asked_for_is_asked_once_more_with_one_it_does() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().on_discover(Discover::Unsupported {
            supported: vec!["2026-07-28".to_owned(), "2025-11-25".to_owned()],
            times: 1,
        });
        let client = connect(&server, &timer).await;
        assert_eq!(client.info().era, Era::Modern);
        assert_eq!(server.methods(), ["server/discover", "server/discover"]);
    });
}

#[test]
fn a_modern_server_that_speaks_no_revision_of_the_clients_is_never_sent_initialize() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().on_discover(Discover::Unsupported {
            supported: vec!["2027-03-30".to_owned(), "2025-11-25".to_owned()],
            times: 99,
        });
        let result = StdioClient::connect(
            &definition(),
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        )
        .await;
        assert!(
            matches!(result, Err(ClientError::NotStarted(_))),
            "{result:?}"
        );
        assert_eq!(server.methods(), ["server/discover"]);
        assert_eq!(
            server.shutdowns(),
            [GRACE],
            "a start that fails ends the server"
        );
    });
}

#[test]
fn a_server_whose_unsupported_version_names_only_legacy_revisions_is_initialized() {
    run(async {
        let timer = timer();
        let legacy_only = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
        let server = FakeServer::legacy("2025-11-25").on_discover(Discover::Unsupported {
            supported: legacy_only
                .iter()
                .map(|version| (*version).to_owned())
                .collect(),
            times: 99,
        });
        let client = connect(&server, &timer).await;
        assert_eq!(client.info().era, Era::Legacy);
        assert_eq!(client.info().version, "2025-11-25");
        server.wait_for("notifications/initialized", 1).await;
        assert_eq!(
            server.methods(),
            ["server/discover", "initialize", "notifications/initialized"]
        );
        assert_eq!(server.spawns(), 1, "on the same process");
    });
}

#[test]
fn a_server_known_to_be_legacy_that_refuses_initialize_and_exits_is_probed_on_a_new_process() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().refuses_initialize_and_exits();
        let client = StdioClient::connect_known(
            Era::Legacy,
            &definition(),
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(client.info().era, Era::Modern);
        assert_eq!(server.spawns(), 2);
        let methods: Vec<Json> = server
            .received_by(2)
            .iter()
            .map(|message| message["method"].clone())
            .collect();
        assert_eq!(methods, ["server/discover"], "asked, not initialized");
    });
}

#[test]
fn the_other_modern_errors_end_the_start_without_initialize() {
    for code in [-32020, -32021] {
        run(async {
            let timer = timer();
            let server = FakeServer::modern().on_discover(Discover::Error(code));
            let result = StdioClient::connect(
                &definition(),
                server.spawn(),
                server.respawn(),
                shared(&timer),
                ClientOptions::default(),
            )
            .await;
            assert!(
                matches!(result, Err(ClientError::NotStarted(_))),
                "{code}: {result:?}"
            );
            assert_eq!(server.methods(), ["server/discover"], "{code}");
        });
    }
}

#[test]
fn a_legacy_answer_of_a_revision_the_client_does_not_speak_ends_the_start() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2023-01-01");
        let result = StdioClient::connect(
            &definition(),
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        )
        .await;
        let Err(ClientError::NotStarted(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("2023-01-01"), "{why}");
        assert!(server.of("notifications/initialized").is_empty());
        assert_eq!(server.shutdowns(), [GRACE]);
    });
}

#[test]
fn a_silent_server_still_starts_within_the_start_limit_and_initialize_is_never_cancelled() {
    run(async {
        let timer = timer();
        let never = Gate::new();
        let server = FakeServer::legacy("2025-06-18")
            .on_discover(Discover::Silent)
            .initialize_gate(&never);
        let server_definition = definition();
        let connecting = StdioClient::connect(
            &server_definition,
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        );
        let (result, ()) = join(connecting, async {
            timer.sleeping_for(PROBE).await;
            timer.advance(PROBE);
            server.wait_for("initialize", 1).await;
            // Ten seconds from the start, not from `initialize`.
            timer.advance(START - PROBE);
        })
        .await;
        assert_eq!(
            result.unwrap_err(),
            ClientError::TimedOut {
                what: "start",
                limit: START,
            }
        );
        assert_eq!(server.shutdowns(), [GRACE]);
        assert!(server.of("notifications/cancelled").is_empty());
    });
}

#[test]
fn a_server_known_to_be_legacy_is_initialized_without_the_probe() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18");
        let client = StdioClient::connect_known(
            Era::Legacy,
            &definition(),
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(client.info().era, Era::Legacy);
        server.wait_for("notifications/initialized", 1).await;
        assert_eq!(
            server.methods(),
            ["initialize", "notifications/initialized"]
        );
    });
}

#[test]
fn a_server_known_to_be_legacy_that_refuses_initialize_is_probed() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern();
        let client = StdioClient::connect_known(
            Era::Legacy,
            &definition(),
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(client.info().era, Era::Modern);
        assert_eq!(server.methods(), ["initialize", "server/discover"]);
        assert_eq!(server.spawns(), 1);
    });
}

/// A modern-only server that reads nothing until `started`, then answers
/// what it read in order: `server/discover` with its discovery result, and
/// anything else (`initialize`) with an error, as a modern-only server does.
/// Written by the review of the client.
async fn modern_only(
    mut input: mpsc::Receiver<Vec<u8>>,
    output: mpsc::Sender<Result<Vec<u8>, LineError>>,
    mut started: watch::Receiver<bool>,
    received: Arc<Mutex<Vec<String>>>,
) {
    let _started = started.wait_for(|started| *started).await.is_ok();
    while let Some(text) = input.recv().await {
        let message: Json = serde_json::from_slice(&text).unwrap();
        let method = message["method"].as_str().unwrap_or_default().to_owned();
        received.lock().unwrap().push(method.clone());
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let reply = match method.as_str() {
            "server/discover" => json!({"jsonrpc": "2.0", "id": id, "result": {
                "resultType": "complete",
                "supportedVersions": ["2026-07-28"],
                "capabilities": {"tools": {}},
                "ttlMs": 0,
                "cacheScope": "public",
            }}),
            _ => json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32601, "message": "Method not found"}}),
        };
        if output.send(line(&reply)).await.is_err() {
            return;
        }
    }
}

#[test]
fn a_modern_server_slower_than_the_probe_to_start_is_found_by_its_late_answer() {
    run(async {
        let timer = timer();
        let (input, input_lines) = mpsc::channel(64);
        let (output, output_lines) = mpsc::channel(64);
        let (started, started_receiver) = watch::channel(false);
        let received = Arc::new(Mutex::new(Vec::new()));
        tokio::spawn(modern_only(
            input_lines,
            output,
            started_receiver,
            Arc::clone(&received),
        ));
        let pipes = ServerPipes {
            input,
            output: output_lines,
            process: Box::new(HandProcess {
                exited: Arc::new(watch::channel(None).0),
            }),
        };
        let server_definition = definition();
        let connecting = StdioClient::connect(
            &server_definition,
            pipes,
            no_respawn(),
            shared(&timer),
            ClientOptions::default(),
        );
        let (result, ()) = join(connecting, async {
            // Still starting (a cold `npx`, `uvx` or `docker run`) when the
            // probe's 3 s run out; then it starts, 4 s into a 10 s start.
            timer.sleeping_for(PROBE).await;
            timer.advance(PROBE);
            settle().await;
            timer.advance(Duration::from_secs(1));
            started.send_replace(true);
        })
        .await;
        let client = result.expect("a modern server that answered the probe late connects");
        assert_eq!(client.info().era, Era::Modern);
        settle().await;
        let discovers = received
            .lock()
            .unwrap()
            .iter()
            .filter(|method| *method == "server/discover")
            .count();
        assert_eq!(
            discovers, 1,
            "its late answer was heard, not asked for again"
        );
    });
}

#[test]
fn a_modern_server_whose_first_answer_was_lost_is_asked_again_once_it_refuses_initialize() {
    for known in [None, Some(Era::Modern)] {
        run(async {
            let timer = timer();
            let server = FakeServer::modern().on_discover(Discover::SilentFirst);
            let server_definition = definition();
            let connecting = async {
                match known {
                    None => {
                        StdioClient::connect(
                            &server_definition,
                            server.spawn(),
                            server.respawn(),
                            shared(&timer),
                            ClientOptions::default(),
                        )
                        .await
                    }
                    Some(era) => {
                        StdioClient::connect_known(
                            era,
                            &server_definition,
                            server.spawn(),
                            server.respawn(),
                            shared(&timer),
                            ClientOptions::default(),
                        )
                        .await
                    }
                }
            };
            let (result, ()) = join(connecting, async {
                timer.sleeping_for(PROBE).await;
                timer.advance(PROBE);
            })
            .await;
            let client = result.unwrap();
            assert_eq!(client.info().era, Era::Modern, "{known:?}");
            assert_eq!(
                server.methods(),
                ["server/discover", "initialize", "server/discover"],
                "{known:?}"
            );
            assert_eq!(server.spawns(), 1);
        });
    }
}

#[test]
fn a_server_that_answers_the_probe_with_a_result_of_another_kind_is_legacy() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").on_discover(Discover::Result(json!({})));
        let client = connect(&server, &timer).await;
        assert_eq!(client.info().era, Era::Legacy);
        server.wait_for("notifications/initialized", 1).await;
        assert_eq!(
            server.methods(),
            ["server/discover", "initialize", "notifications/initialized"]
        );
    });
}

#[test]
fn a_server_known_to_be_legacy_that_exits_on_initialize_is_started_again_and_only_initialized() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").exits_on_first_initialize();
        let client = StdioClient::connect_known(
            Era::Legacy,
            &definition(),
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(client.info().era, Era::Legacy);
        assert_eq!(server.spawns(), 2);
        server.wait_for("notifications/initialized", 1).await;
        let methods = |number| -> Vec<Json> {
            server
                .received_by(number)
                .iter()
                .map(|message| message["method"].clone())
                .collect()
        };
        assert_eq!(methods(1), ["initialize"]);
        assert_eq!(
            methods(2),
            ["initialize", "notifications/initialized"],
            "not probed"
        );
    });
}
