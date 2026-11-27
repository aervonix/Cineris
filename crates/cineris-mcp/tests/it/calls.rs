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
fn a_crash_during_calls_fails_every_one_in_flight_with_the_exit_and_closes_the_client() {
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
            let stopped = ClientError::Stopped("it crashed during a call".to_owned());
            assert_eq!(held.unwrap_err(), stopped);
            assert_eq!(boom.unwrap_err(), stopped);
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
