//! Calls in both eras: results and errors, answers out of order, duplicate
//! and stray answers, cancellation, the call's limit, the bound on requests
//! in flight, a crash, batches, and shutdown.

use cineris_base::CancelToken;
use cineris_mcp::fake::{Gate, Reply};
use cineris_mcp::{CallOutput, Client, ClientError, Content};
use cineris_testkit::join;

use super::*;

fn eras() -> [FakeServer; 2] {
    [FakeServer::legacy("2025-06-18"), FakeServer::modern()]
}

fn words(output: &CallOutput) -> String {
    match output.content.as_slice() {
        [Content::Text(words)] => words.clone(),
        other => panic!("not one text block: {other:?}"),
    }
}

#[test]
fn a_call_returns_its_content_its_structured_result_and_whether_the_tool_failed() {
    for server in eras() {
        run(async {
            let timer = timer();
            let server = server.reply(
                "find",
                Reply::Result(json!({
                    "content": [{"type": "text", "text": "not found"}],
                    "structuredContent": {"hits": 0},
                    "isError": true,
                })),
            );
            let client = connect(&server, &timer).await;
            let output = client
                .call("find", json!({"q": "x"}), &CancelToken::new())
                .await
                .unwrap();
            assert_eq!(words(&output), "not found");
            assert_eq!(output.structured, Some(json!({"hits": 0})));
            assert!(output.is_error);
            assert_eq!(server.of("tools/call")[0]["params"]["name"], "find");
            assert_eq!(
                server.of("tools/call")[0]["params"]["arguments"],
                json!({"q": "x"})
            );
        });
    }
}

#[test]
fn an_error_of_the_protocol_is_the_servers_error() {
    for server in eras() {
        run(async {
            let timer = timer();
            let server = server.reply(
                "gone",
                Reply::Error(-32602, "unknown tool: gone".to_owned()),
            );
            let client = connect(&server, &timer).await;
            let result = client.call("gone", json!({}), &CancelToken::new()).await;
            assert_eq!(
                result.unwrap_err(),
                ClientError::Server {
                    code: -32602,
                    message: "unknown tool: gone".to_owned(),
                }
            );
        });
    }
}

#[test]
fn answers_out_of_order_reach_their_own_calls() {
    run(async {
        let timer = timer();
        let (first_gate, second_gate) = (Gate::new(), Gate::new());
        let server = FakeServer::legacy("2025-06-18")
            .reply(
                "first",
                Reply::Gated(first_gate.clone(), Box::new(Reply::Result(text("one")))),
            )
            .reply(
                "second",
                Reply::Gated(second_gate.clone(), Box::new(Reply::Result(text("two")))),
            );
        let client = connect(&server, &timer).await;
        let cancel = CancelToken::new();
        let calls = join(
            client.call("first", json!({}), &cancel),
            client.call("second", json!({}), &cancel),
        );
        let ((first, second), ()) = join(calls, async {
            server.wait_for("tools/call", 2).await;
            second_gate.open();
            settle().await;
            first_gate.open();
        })
        .await;
        assert_eq!(words(&first.unwrap()), "one");
        assert_eq!(words(&second.unwrap()), "two");
    });
}

#[test]
fn a_duplicate_answer_and_an_answer_to_no_request_are_ignored() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").reply(
            "twice",
            Reply::Twice(Box::new(Reply::Result(text("first")))),
        );
        let client = connect(&server, &timer).await;
        let cancel = CancelToken::new();
        let output = client.call("twice", json!({}), &cancel).await.unwrap();
        assert_eq!(words(&output), "first");
        server
            .send(json!({"jsonrpc": "2.0", "id": 4242, "result": text("nobody asked")}))
            .await;
        server
            .send(json!({"jsonrpc": "2.0", "id": "a string", "result": text("nobody asked")}))
            .await;
        let next = client.call("other", json!({}), &cancel).await.unwrap();
        assert_eq!(
            words(&next),
            "called other",
            "the next call gets its own answer"
        );
    });
}

#[test]
fn a_cancelled_call_returns_at_once_and_its_late_answer_is_ignored() {
    for server in eras() {
        run(async {
            let timer = timer();
            let gate = Gate::new();
            let server = server.reply(
                "slow",
                Reply::Gated(gate.clone(), Box::new(Reply::Result(text("late")))),
            );
            let client = connect(&server, &timer).await;
            let cancel = CancelToken::new();
            let (result, ()) = join(client.call("slow", json!({}), &cancel), async {
                server.wait_for("tools/call", 1).await;
                cancel.cancel();
            })
            .await;
            assert_eq!(result.unwrap_err(), ClientError::Cancelled);
            settle().await;
            let cancelled = &server.of("notifications/cancelled")[0]["params"];
            assert_eq!(cancelled["requestId"], id_of(&server, "tools/call", 0));
            assert!(cancelled["reason"].is_string());

            // Its answer, now late, reaches nothing.
            gate.open();
            let next = client
                .call("other", json!({}), &CancelToken::new())
                .await
                .unwrap();
            assert_eq!(words(&next), "called other");
        });
    }
}

#[test]
fn a_call_past_its_limit_is_cancelled_and_says_it_timed_out() {
    for server in eras() {
        run(async {
            let timer = timer();
            let server = server.reply("stuck", Reply::Silent);
            let client = connect(&server, &timer).await;
            let cancel = CancelToken::new();
            let (result, ()) = join(client.call("stuck", json!({}), &cancel), async {
                server.wait_for("tools/call", 1).await;
                timer.sleeping_for(CALL).await;
                timer.advance(CALL);
            })
            .await;
            assert_eq!(
                result.unwrap_err(),
                ClientError::TimedOut {
                    what: "answer the call",
                    limit: CALL,
                }
            );
            settle().await;
            assert_eq!(
                server.of("notifications/cancelled")[0]["params"]["requestId"],
                id_of(&server, "tools/call", 0)
            );
        });
    }
}

#[test]
fn a_listing_past_its_limit_is_cancelled_and_says_it_timed_out() {
    run(async {
        let timer = timer();
        let never = Gate::new();
        let server = FakeServer::legacy("2025-06-18").list_gate(&never);
        let client = connect(&server, &timer).await;
        let limit = ClientOptions::default().list_timeout;
        let (result, ()) = join(client.list_tools(&CancelToken::new()), async {
            timer.sleeping_for(limit).await;
            timer.advance(limit);
        })
        .await;
        assert_eq!(
            result.unwrap_err(),
            ClientError::TimedOut {
                what: "list its tools",
                limit,
            }
        );
        settle().await;
        assert_eq!(server.of("notifications/cancelled").len(), 1);
    });
}

#[test]
fn only_so_many_requests_wait_for_their_answers_at_once() {
    run(async {
        let timer = timer();
        let gate = Gate::new();
        let server = FakeServer::legacy("2025-06-18").reply(
            "held",
            Reply::Gated(gate.clone(), Box::new(Reply::Result(text("held")))),
        );
        let options = ClientOptions {
            max_in_flight: 2,
            ..ClientOptions::default()
        };
        let client = connect_with(&server, &timer, options).await;
        let cancel = CancelToken::new();
        let three = join(
            join(
                client.call("held", json!({}), &cancel),
                client.call("held", json!({}), &cancel),
            ),
            client.call("free", json!({}), &cancel),
        );
        let (((first, second), third), ()) = join(three, async {
            server.wait_for("tools/call", 2).await;
            settle().await;
            assert_eq!(server.of("tools/call").len(), 2, "the third waits for room");
            gate.open();
        })
        .await;
        assert_eq!(words(&first.unwrap()), "held");
        assert_eq!(words(&second.unwrap()), "held");
        assert_eq!(words(&third.unwrap()), "called free");
    });
}

#[test]
fn a_crash_during_calls_says_each_one_written_may_have_run_and_closes_the_client() {
    for server in eras() {
        run(async {
            let timer = timer();
            let gate = Gate::new();
            let server = server
                .reply(
                    "held",
                    Reply::Gated(gate.clone(), Box::new(Reply::Result(text("held")))),
                )
                .reply("boom", Reply::Crash);
            let client = connect(&server, &timer).await;
            let cancel = CancelToken::new();
            let (held, boom) = join(
                client.call("held", json!({}), &cancel),
                client.call("boom", json!({}), &cancel),
            )
            .await;
            // Both were written: the server may have acted on them.
            let may_have_run = ClientError::MayHaveRun("it crashed during a call".to_owned());
            assert_eq!(held.unwrap_err(), may_have_run);
            assert_eq!(boom.unwrap_err(), may_have_run);
            let stopped = ClientError::Stopped("it crashed during a call".to_owned());
            assert_eq!(client.closed().await, stopped);
            assert_eq!(
                client.call("after", json!({}), &cancel).await.unwrap_err(),
                stopped,
                "a call after the end fails at once"
            );
        });
    }
}

#[test]
fn a_call_still_waiting_for_room_when_the_server_crashes_never_ran() {
    run(async {
        let timer = timer();
        let gate = Gate::new();
        let server = FakeServer::legacy("2025-06-18")
            .reply(
                "held",
                Reply::Gated(gate.clone(), Box::new(Reply::Result(text("held")))),
            )
            .reply("boom", Reply::Crash);
        let options = ClientOptions {
            max_in_flight: 1,
            ..ClientOptions::default()
        };
        let client = connect_with(&server, &timer, options).await;
        let cancel = CancelToken::new();
        let calls = join(
            client.call("held", json!({}), &cancel),
            client.call("queued", json!({}), &cancel),
        );
        let ((held, queued), ()) = join(calls, async {
            server.wait_for("tools/call", 1).await;
            server.crash("killed");
        })
        .await;
        assert_eq!(
            held.unwrap_err(),
            ClientError::MayHaveRun("killed".to_owned())
        );
        assert_eq!(
            queued.unwrap_err(),
            ClientError::Stopped("killed".to_owned()),
            "never written, so it never ran"
        );
        assert_eq!(server.of("tools/call").len(), 1);
    });
}

#[test]
fn a_call_given_up_while_it_waits_for_room_is_not_cancelled_on_the_server() {
    run(async {
        let timer = timer();
        let gate = Gate::new();
        let server = FakeServer::legacy("2025-06-18").reply(
            "held",
            Reply::Gated(gate.clone(), Box::new(Reply::Result(text("held")))),
        );
        let options = ClientOptions {
            max_in_flight: 1,
            ..ClientOptions::default()
        };
        let client = connect_with(&server, &timer, options).await;
        let (held_cancel, queued_cancel) = (CancelToken::new(), CancelToken::new());
        let calls = join(
            client.call("held", json!({}), &held_cancel),
            client.call("queued", json!({}), &queued_cancel),
        );
        let ((held, queued), ()) = join(calls, async {
            server.wait_for("tools/call", 1).await;
            queued_cancel.cancel();
            settle().await;
            assert!(
                server.of("notifications/cancelled").is_empty(),
                "a request never sent is not cancelled"
            );
            gate.open();
        })
        .await;
        assert_eq!(words(&held.unwrap()), "held");
        assert_eq!(queued.unwrap_err(), ClientError::Cancelled);
    });
}

#[test]
fn an_answer_written_just_before_the_exit_reaches_its_call() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").reply(
            "finish",
            Reply::ExitThenAnswer(Box::new(Reply::Result(text("done, and exiting")))),
        );
        let client = connect(&server, &timer).await;
        let output = client
            .call("finish", json!({}), &CancelToken::new())
            .await
            .unwrap();
        assert_eq!(words(&output), "done, and exiting");
        assert_eq!(
            client.closed().await,
            ClientError::Stopped("it exited after its answer".to_owned())
        );
    });
}

/// A legacy server that answers the handshake, then answers `tools/call`
/// and exits; the last line is handed on a moment after the exit, as a
/// thread reading a real pipe can hand it on after a thread waiting for the
/// process has seen it exit. Written by the review of the client.
async fn answers_then_exits(
    mut input: tokio::sync::mpsc::Receiver<Vec<u8>>,
    output: tokio::sync::mpsc::Sender<Result<Vec<u8>, cineris_mcp::LineError>>,
    exited: Arc<tokio::sync::watch::Sender<Option<String>>>,
) {
    while let Some(text) = input.recv().await {
        let message: Json = serde_json::from_slice(&text).unwrap();
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let reply = match message["method"].as_str().unwrap_or_default() {
            "initialize" => json!({"jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "x", "version": "1"},
            }}),
            "tools/call" => {
                let answer = json!({"jsonrpc": "2.0", "id": id, "result": {
                    "content": [{"type": "text", "text": "done, and exiting"}],
                }});
                // It wrote its answer and exited; the exit is seen first.
                exited.send_replace(Some("it exited with status 0".to_owned()));
                tokio::task::yield_now().await;
                let _sent = output.send(line(&answer)).await;
                return;
            }
            _ => json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32601, "message": "Method not found"}}),
        };
        if output.send(line(&reply)).await.is_err() {
            return;
        }
    }
}

#[test]
fn an_answer_a_pipe_hands_on_after_the_exit_reaches_its_call() {
    run(async {
        let timer = timer();
        let (input, input_lines) = tokio::sync::mpsc::channel(64);
        let (output, output_lines) = tokio::sync::mpsc::channel(64);
        let exited = Arc::new(tokio::sync::watch::channel(None).0);
        tokio::spawn(answers_then_exits(input_lines, output, Arc::clone(&exited)));
        let pipes = cineris_mcp::ServerPipes {
            input,
            output: output_lines,
            process: Box::new(HandProcess { exited }),
        };
        let client = StdioClient::connect(
            &definition(),
            pipes,
            no_respawn(),
            shared(&timer),
            ClientOptions::default(),
        )
        .await
        .unwrap();
        let output = client
            .call("finish", json!({}), &CancelToken::new())
            .await
            .unwrap();
        assert_eq!(words(&output), "done, and exiting");
    });
}

#[test]
fn a_crash_while_nothing_is_asked_closes_the_client_with_the_exit() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern();
        let client = connect(&server, &timer).await;
        server.crash("killed by a signal");
        assert_eq!(
            client.closed().await,
            ClientError::Stopped("killed by a signal".to_owned())
        );
    });
}

#[test]
fn a_batch_is_read_only_from_a_server_of_the_revision_that_has_them() {
    for (version, batched) in [("2025-03-26", true), ("2025-06-18", false)] {
        run(async {
            let timer = timer();
            let server = FakeServer::legacy(version)
                .list_changed()
                .reply("slow", Reply::Silent);
            let client = connect(&server, &timer).await;
            let changes = client.changes();
            let cancel = CancelToken::new();
            let (output, ()) = join(client.call("slow", json!({}), &cancel), async {
                server.wait_for("tools/call", 1).await;
                let id = id_of(&server, "tools/call", 0);
                server
                    .send(json!([
                        {"jsonrpc": "2.0", "id": id, "result": text("in a batch")},
                        {"jsonrpc": "2.0", "method": "notifications/tools/list_changed"},
                    ]))
                    .await;
                server
                    .send(json!({"jsonrpc": "2.0", "id": id, "result": text("alone")}))
                    .await;
            })
            .await;
            let expected = if batched { "in a batch" } else { "alone" };
            assert_eq!(words(&output.unwrap()), expected, "{version}");
            assert_eq!(changes.has_changed().unwrap(), batched, "{version}");
        });
    }
}

#[test]
fn an_error_with_no_id_ends_nothing() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18");
        let client = connect(&server, &timer).await;
        server
            .send(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}}))
            .await;
        server
            .send(json!({"jsonrpc": "2.0", "error": {"code": -32600, "message": "invalid"}}))
            .await;
        let output = client
            .call("a", json!({}), &CancelToken::new())
            .await
            .unwrap();
        assert_eq!(words(&output), "called a");
    });
}

#[test]
fn an_answer_with_the_id_written_as_a_string_of_its_digits_is_matched() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").reply("slow", Reply::Silent);
        let client = connect(&server, &timer).await;
        let cancel = CancelToken::new();
        let (output, ()) = join(client.call("slow", json!({}), &cancel), async {
            server.wait_for("tools/call", 1).await;
            let id = id_of(&server, "tools/call", 0)
                .as_u64()
                .unwrap()
                .to_string();
            server
                .send(json!({"jsonrpc": "2.0", "id": id, "result": text("as a string")}))
                .await;
        })
        .await;
        assert_eq!(words(&output.unwrap()), "as a string");
    });
}

#[test]
fn shutdown_closes_the_input_and_gives_the_process_its_grace() {
    for server in eras() {
        run(async {
            let timer = timer();
            let client = connect(&server, &timer).await;
            client.shutdown().await;
            assert_eq!(server.shutdowns(), [GRACE]);
            assert!(matches!(client.closed().await, ClientError::Stopped(_)));
            client.shutdown().await;
            assert_eq!(server.shutdowns(), [GRACE], "once");
        });
    }
}

#[test]
fn a_dropped_client_shuts_its_server_down_without_waiting() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18");
        let client = connect(&server, &timer).await;
        drop(client);
        until(|| server.shutdowns() == [GRACE]).await;
    });
}
