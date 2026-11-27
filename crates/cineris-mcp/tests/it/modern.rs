//! The modern era: `_meta`, results of other types, the listing's caps and
//! lifetime, and the change stream.

use cineris_base::CancelToken;
use cineris_mcp::fake::{EndListen, Gate, Reply};
use cineris_mcp::{Client, ClientError, LISTEN_AGAIN};

use super::*;

/// A modern server that says it tells the client of changes, connected,
/// with its change stream acknowledged.
async fn listening(server: &FakeServer, timer: &FakeTimer) -> StdioClient {
    let client = connect(server, timer).await;
    until(|| client.listening()).await;
    client
}

#[test]
fn every_modern_request_carries_the_revision_the_capabilities_and_who_the_client_is() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().tools(vec![tool("a")]);
        let client = connect(&server, &timer).await;
        let cancel = CancelToken::new();
        client.list_tools(&cancel).await.unwrap();
        client.call("a", json!({"x": 1}), &cancel).await.unwrap();
        let requests: Vec<Json> = server
            .received()
            .into_iter()
            .filter(|message| message.get("id").is_some())
            .collect();
        assert_eq!(requests.len(), 3);
        for request in requests {
            let meta = &request["params"]["_meta"];
            assert_eq!(
                meta["io.modelcontextprotocol/protocolVersion"], "2026-07-28",
                "{request}"
            );
            assert_eq!(
                meta["io.modelcontextprotocol/clientCapabilities"],
                json!({}),
                "{request}"
            );
            assert_eq!(
                meta["io.modelcontextprotocol/clientInfo"]["name"], "cineris",
                "{request}"
            );
        }
        assert_eq!(
            server.of("tools/call")[0]["params"]["arguments"],
            json!({"x": 1})
        );
    });
}

#[test]
fn a_listing_past_twenty_pages_or_past_512_tools_is_refused() {
    run(async {
        let timer = timer();
        let pages = FakeServer::modern()
            .tools((0..21).map(|number| tool(&format!("t{number}"))).collect())
            .page_size(1);
        let client = connect(&pages, &timer).await;
        let result = client.list_tools(&CancelToken::new()).await;
        let Err(ClientError::Protocol(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("20 pages"), "{why}");
        assert_eq!(pages.of("tools/list").len(), 20);

        let tools = FakeServer::modern()
            .tools((0..513).map(|number| tool(&format!("t{number}"))).collect())
            .page_size(1000);
        let client = connect(&tools, &timer).await;
        let result = client.list_tools(&CancelToken::new()).await;
        let Err(ClientError::Protocol(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("512 tools"), "{why}");

        let enough = FakeServer::modern()
            .tools((0..512).map(|number| tool(&format!("t{number}"))).collect())
            .page_size(26);
        let client = connect(&enough, &timer).await;
        assert_eq!(
            client.list_tools(&CancelToken::new()).await.unwrap().len(),
            512
        );
    });
}

#[test]
fn the_end_of_a_listings_lifetime_bumps_the_changes() {
    run(async {
        let timer = timer();
        let lifetime = Duration::from_millis(7_500);
        let server = FakeServer::modern().tools(vec![tool("a")]).ttl_ms(7_500);
        let client = connect(&server, &timer).await;
        let mut changes = client.changes();
        client.list_tools(&CancelToken::new()).await.unwrap();
        timer.sleeping_for(lifetime).await;
        assert!(!changes.has_changed().unwrap());
        timer.advance(lifetime);
        changes.changed().await.unwrap();
        assert_eq!(*changes.borrow(), 1);
    });
}

#[test]
fn a_lifetime_of_zero_or_less_or_none_bumps_nothing() {
    for server in [
        FakeServer::modern().ttl_ms(0),
        FakeServer::modern().ttl_ms(-5),
        FakeServer::modern(),
        FakeServer::legacy("2025-06-18"),
    ] {
        run(async {
            let timer = timer();
            let server = server.tools(vec![tool("a")]);
            let client = connect(&server, &timer).await;
            let changes = client.changes();
            client.list_tools(&CancelToken::new()).await.unwrap();
            settle().await;
            assert!(timer.running().is_empty(), "{:?}", timer.running());
            assert!(!changes.has_changed().unwrap());
        });
    }
}

#[test]
fn a_call_the_server_needs_sent_again_is_sent_with_a_new_id_and_its_state_given_back() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().reply(
            "slow",
            Reply::InputRequired(
                vec!["first state".to_owned(), "second state".to_owned()],
                Box::new(Reply::Result(text("done"))),
            ),
        );
        let client = connect(&server, &timer).await;
        let output = client
            .call("slow", json!({"q": "x"}), &CancelToken::new())
            .await
            .unwrap();
        assert_eq!(
            output.content,
            [cineris_mcp::Content::Text("done".to_owned())]
        );
        let calls = server.of("tools/call");
        assert_eq!(calls.len(), 3);
        let ids: Vec<&Json> = calls.iter().map(|call| &call["id"]).collect();
        assert!(
            ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2],
            "{ids:?}"
        );
        assert_eq!(calls[0]["params"].get("requestState"), None);
        assert_eq!(calls[1]["params"]["requestState"], "first state");
        assert_eq!(calls[2]["params"]["requestState"], "second state");
        for call in &calls {
            assert_eq!(
                call["params"]["arguments"],
                json!({"q": "x"}),
                "the same call each time"
            );
        }
    });
}

#[test]
fn a_server_that_keeps_asking_for_the_call_again_is_refused_after_three_rounds() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().reply(
            "loop",
            Reply::AlwaysInput(json!({"resultType": "input_required", "requestState": "again"})),
        );
        let client = connect(&server, &timer).await;
        let result = client.call("loop", json!({}), &CancelToken::new()).await;
        assert!(
            matches!(result, Err(ClientError::Protocol(_))),
            "{result:?}"
        );
        assert_eq!(
            server.of("tools/call").len(),
            4,
            "the call and three rounds"
        );
    });
}

#[test]
fn a_server_that_asks_for_input_the_client_never_offered_or_gives_no_state_is_refused() {
    for result in [
        json!({"resultType": "input_required", "inputRequests": {"q": {"method": "elicitation/create"}}, "requestState": "s"}),
        json!({"resultType": "input_required"}),
    ] {
        run(async {
            let timer = timer();
            let server = FakeServer::modern().reply("ask", Reply::AlwaysInput(result.clone()));
            let client = connect(&server, &timer).await;
            let outcome = client.call("ask", json!({}), &CancelToken::new()).await;
            assert!(
                matches!(outcome, Err(ClientError::Protocol(_))),
                "{outcome:?}"
            );
            assert_eq!(server.of("tools/call").len(), 1, "never sent again");
        });
    }
}

#[test]
fn a_result_of_a_type_the_client_does_not_know_is_refused() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().reply(
            "task",
            Reply::Result(json!({"resultType": "task", "task": {"id": "t"}})),
        );
        let client = connect(&server, &timer).await;
        let result = client.call("task", json!({}), &CancelToken::new()).await;
        assert!(
            matches!(result, Err(ClientError::Protocol(_))),
            "{result:?}"
        );
    });
}

#[test]
fn a_modern_servers_request_is_a_fault_counted_and_never_answered() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().tools(vec![tool("a")]);
        let client = connect(&server, &timer).await;
        server
            .send(json!({"jsonrpc": "2.0", "id": "s-1", "method": "ping"}))
            .await;
        // Read in order: once the listing is answered, the request was read.
        client.list_tools(&CancelToken::new()).await.unwrap();
        assert_eq!(client.faults(), 1);
        settle().await;
        assert!(
            server
                .received()
                .iter()
                .all(|message| message["id"] != "s-1"),
            "no answer"
        );
    });
}

#[test]
fn the_change_stream_is_acknowledged_and_only_its_own_notices_bump_the_changes() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().list_changed();
        let client = listening(&server, &timer).await;
        assert!(client.info().list_changed);
        let listen = &server.of("subscriptions/listen")[0];
        assert_eq!(
            listen["params"]["notifications"],
            json!({"toolsListChanged": true})
        );
        assert!(listen["params"]["_meta"].is_object());

        let mut changes = client.changes();
        // One on another stream, and one on none: not this stream's.
        server
            .send(json!({
                "jsonrpc": "2.0",
                "method": "notifications/tools/list_changed",
                "params": {"_meta": {"io.modelcontextprotocol/subscriptionId": 999}},
            }))
            .await;
        server
            .send(json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}))
            .await;
        server.notify_list_changed().await;
        changes.changed().await.unwrap();
        assert_eq!(*changes.borrow_and_update(), 1);
    });
}

#[test]
fn a_change_stream_the_server_ends_is_opened_again_and_the_changes_bumped() {
    for how in [EndListen::Result, EndListen::Cancelled] {
        run(async {
            let timer = timer();
            let server = FakeServer::modern().list_changed();
            let client = listening(&server, &timer).await;
            let first = server.listen_id().unwrap();
            let mut changes = client.changes();
            server.end_listen(how).await;
            timer.sleeping_for(LISTEN_AGAIN).await;
            assert!(!client.listening(), "{how:?}");
            assert_eq!(server.of("subscriptions/listen").len(), 1, "not at once");
            timer.advance(LISTEN_AGAIN);
            settle().await;
            assert_eq!(server.of("subscriptions/listen").len(), 2, "{how:?}");
            until(|| client.listening()).await;
            // What changed while no stream was open was missed: a change.
            changes.changed().await.unwrap();
            assert_eq!(*changes.borrow_and_update(), 1, "{how:?}");
            assert_ne!(server.listen_id().unwrap(), first);

            // A late end of the first changes nothing.
            server
                .send(json!({"jsonrpc": "2.0", "id": first, "result": {"resultType": "complete"}}))
                .await;
            server.notify_list_changed().await;
            changes.changed().await.unwrap();
            assert!(client.listening());
            settle().await;
            assert_eq!(server.of("subscriptions/listen").len(), 2, "{how:?}");
        });
    }
}

#[test]
fn the_change_stream_has_no_time_limit() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().list_changed();
        let client = listening(&server, &timer).await;
        timer.advance(Duration::from_secs(24 * 60 * 60));
        settle().await;
        assert!(client.listening());
        assert_eq!(server.of("subscriptions/listen").len(), 1);
        assert!(server.of("notifications/cancelled").is_empty());
    });
}

#[test]
fn a_change_stream_acknowledged_without_the_notices_asked_for_is_ended_and_not_asked_again() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().list_changed().refuses_listen();
        let client = connect(&server, &timer).await;
        server.wait_for("notifications/cancelled", 1).await;
        let listen = id_of(&server, "subscriptions/listen", 0);
        assert_eq!(
            server.of("notifications/cancelled")[0]["params"]["requestId"],
            listen
        );
        timer.advance(Duration::from_secs(60 * 60));
        settle().await;
        assert!(!client.listening());
        assert_eq!(server.of("subscriptions/listen").len(), 1);
    });
}

#[test]
fn a_change_stream_answered_with_an_error_is_over_and_not_asked_again() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().list_changed().listen_error(-32601);
        let client = connect(&server, &timer).await;
        server.wait_for("subscriptions/listen", 1).await;
        settle().await;
        assert!(
            server.of("notifications/cancelled").is_empty(),
            "an answered request is not cancelled"
        );
        timer.advance(Duration::from_secs(60 * 60));
        settle().await;
        assert!(!client.listening());
        assert_eq!(server.of("subscriptions/listen").len(), 1);
    });
}

#[test]
fn a_cancel_naming_another_request_leaves_the_change_stream_open() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().list_changed();
        let client = listening(&server, &timer).await;
        let mut changes = client.changes();
        server
            .send(json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": {"requestId": 9_999, "reason": "not the stream"},
            }))
            .await;
        server.notify_list_changed().await;
        changes.changed().await.unwrap();
        assert!(client.listening());
        timer.advance(LISTEN_AGAIN);
        settle().await;
        assert_eq!(server.of("subscriptions/listen").len(), 1, "never ended");
    });
}

#[test]
fn an_acknowledgement_of_another_stream_neither_acknowledges_nor_refuses_the_open_one() {
    run(async {
        let timer = timer();
        let gate = Gate::new();
        let server = FakeServer::modern().list_changed().ack_gate(&gate);
        let client = connect(&server, &timer).await;
        server.wait_for("subscriptions/listen", 1).await;
        // Another stream's, refusing the notices: not this one's.
        server
            .send(json!({
                "jsonrpc": "2.0",
                "method": "notifications/subscriptions/acknowledged",
                "params": {
                    "_meta": {"io.modelcontextprotocol/subscriptionId": 9_999},
                    "notifications": {},
                },
            }))
            .await;
        settle().await;
        assert!(!client.listening(), "not acknowledged by another's");
        assert!(
            server.of("notifications/cancelled").is_empty(),
            "not refused by another's"
        );
        gate.open();
        until(|| client.listening()).await;
        settle().await;
        assert!(server.of("notifications/cancelled").is_empty());
    });
}

#[test]
fn a_new_listing_replaces_the_last_ones_lifetime() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().tools(vec![tool("a")]).ttl_ms(7_500);
        let client = connect(&server, &timer).await;
        let changes = client.changes();
        let cancel = CancelToken::new();
        client.list_tools(&cancel).await.unwrap();
        timer.advance(Duration::from_secs(5));
        client.list_tools(&cancel).await.unwrap();
        // The first listing's lifetime would end now.
        timer.advance(Duration::from_millis(2_500));
        settle().await;
        assert!(
            !changes.has_changed().unwrap(),
            "the first lifetime is gone"
        );
        assert_eq!(timer.running(), [Duration::from_millis(7_500)]);
    });
}

#[test]
fn a_listings_lifetime_is_its_shortest_pages() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern()
            .tools(vec![tool("a"), tool("b")])
            .page_size(1)
            .ttl_pages(&[10_000, 5_000]);
        let client = connect(&server, &timer).await;
        client.list_tools(&CancelToken::new()).await.unwrap();
        assert_eq!(timer.running(), [Duration::from_secs(5)]);
    });
}

#[test]
fn a_listing_larger_than_the_cap_is_refused() {
    run(async {
        let timer = timer();
        let description = "x".repeat(1024 * 1024);
        let tools = (0..16)
            .map(|number| {
                json!({
                    "name": format!("t{number}"),
                    "description": description,
                    "inputSchema": {"type": "object"},
                })
            })
            .collect();
        let server = FakeServer::modern().tools(tools).page_size(1);
        let client = connect(&server, &timer).await;
        let result = client.list_tools(&CancelToken::new()).await;
        let Err(ClientError::Protocol(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("32 MiB"), "{why}");
        assert_eq!(
            server.of("tools/list").len(),
            16,
            "every page read until then"
        );
    });
}

/// A call whose first round was written and answered (`input_required`
/// with a state), and whose server exits before the next round is written,
/// may have run. Written by the second review of the client.
#[test]
fn a_call_whose_first_round_ran_says_it_may_have_run_when_the_next_is_never_written() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().reply(
            "step",
            Reply::ExitThenAnswer(Box::new(Reply::AlwaysInput(
                json!({"resultType": "input_required", "requestState": "s1"}),
            ))),
        );
        let client = connect(&server, &timer).await;
        let result = client.call("step", json!({}), &CancelToken::new()).await;
        assert_eq!(server.of("tools/call").len(), 1, "round 1 was written");
        assert!(
            matches!(result, Err(ClientError::MayHaveRun(_))),
            "a call whose first round ran: {result:?}"
        );
    });
}

/// A tool whose input schema holds `count` zeros: two bytes each of a line,
/// and a JSON value each in memory.
fn zeros_tool(count: usize) -> Json {
    json!({"name": "zeros", "inputSchema": {"type": "object", "x": vec![0; count]}})
}

/// How many JSON values `value` is, and the bytes of its strings and keys:
/// what holding it takes, counted apart from the client's own measure.
fn values_and_text(value: &Json) -> (usize, usize) {
    match value {
        Json::Array(items) => items
            .iter()
            .map(values_and_text)
            .fold((1, 0), |sum, item| (sum.0 + item.0, sum.1 + item.1)),
        Json::Object(map) => map.iter().fold((1, 0), |sum, (key, item)| {
            let item = values_and_text(item);
            (sum.0 + item.0, sum.1 + key.len() + item.1)
        }),
        Json::String(text) => (1, text.len()),
        _ => (1, 0),
    }
}

#[test]
fn a_listing_is_capped_by_what_it_holds_in_memory_and_one_near_the_cap_stays_under_its_bound() {
    run(async {
        let timer = timer();
        let value = std::mem::size_of::<Json>();
        // 600,000 zeros: 1.2 MB of a line, under the first round's cap of
        // 16 MiB written, and 19 MB in memory for each of the two copies a
        // definition keeps (`raw` and `input_schema`): over 32 MiB.
        let over = FakeServer::modern().tools(vec![zeros_tool(600_000)]);
        let client = connect(&over, &timer).await;
        let result = client.list_tools(&CancelToken::new()).await;
        let Err(ClientError::Protocol(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("32 MiB"), "{why}");

        // 450,000 zeros: 14.4 MB a copy, under the cap.
        let near = FakeServer::modern().tools(vec![zeros_tool(450_000)]);
        let client = connect(&near, &timer).await;
        let tools = client.list_tools(&CancelToken::new()).await.unwrap();
        let held = tools
            .iter()
            .flat_map(|tool| [&tool.raw, &tool.input_schema])
            .map(values_and_text)
            .map(|(values, text)| values * value + text)
            .sum::<usize>();
        // The bound: what the listing the client returns holds, its two
        // copies of each schema included, is under 32 MiB; the first
        // round's cap let a listing of zeros hold about 1 GiB.
        assert!(held > 28_000_000, "{held}");
        assert!(held < cineris_mcp::MAX_LISTING_BYTES, "{held}");
    });
}

#[test]
fn shutdown_ends_the_change_stream_and_then_the_process() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().list_changed();
        let client = listening(&server, &timer).await;
        let listen = server.listen_id().unwrap();
        client.shutdown().await;
        settle().await;
        let cancelled = server.of("notifications/cancelled");
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0]["params"]["requestId"], listen);
        assert_eq!(server.shutdowns(), [GRACE]);
        assert!(matches!(client.closed().await, ClientError::Stopped(_)));
    });
}
