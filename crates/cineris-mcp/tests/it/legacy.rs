//! What a legacy server may send, and the listings both eras share.

use cineris_base::CancelToken;
use cineris_mcp::Client;
use cineris_mcp::fake::Gate;
use cineris_testkit::join;

use super::*;

/// The answer the server received to its request numbered `id`.
async fn answer_to(server: &FakeServer, id: Json) -> Json {
    let id_ = id.clone();
    server
        .wait_until(move |server| {
            server
                .received()
                .iter()
                .any(|message| message["id"] == id_ && message.get("method").is_none())
        })
        .await;
    server
        .received()
        .into_iter()
        .find(|message| message["id"] == id && message.get("method").is_none())
        .unwrap()
}

#[test]
fn a_ping_is_answered_during_the_handshake_and_after_it() {
    run(async {
        let timer = timer();
        // It answers `initialize` only once its ping is answered.
        let server = FakeServer::legacy("2025-11-25").pings_during_initialize();
        let _client = connect(&server, &timer).await;
        assert_eq!(
            answer_to(&server, json!("fake-ping")).await,
            json!({"jsonrpc": "2.0", "id": "fake-ping", "result": {}})
        );

        server
            .send(json!({"jsonrpc": "2.0", "id": 77, "method": "ping"}))
            .await;
        assert_eq!(
            answer_to(&server, json!(77)).await,
            json!({"jsonrpc": "2.0", "id": 77, "result": {}})
        );
    });
}

#[test]
fn any_other_request_of_a_legacy_server_is_answered_that_the_method_is_not_offered() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").tools(vec![tool("a")]);
        let client = connect(&server, &timer).await;
        for (id, method) in [
            (json!("s-1"), "sampling/createMessage"),
            (json!("s-2"), "roots/list"),
            (json!(3), "elicitation/create"),
        ] {
            server
                .send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": {}}))
                .await;
            let answer = answer_to(&server, id.clone()).await;
            assert_eq!(answer["error"]["code"], -32601, "{method}");
        }
        // A notification it does not know is left.
        server
            .send(json!({"jsonrpc": "2.0", "method": "notifications/whatever", "params": {}}))
            .await;
        assert_eq!(
            client.list_tools(&CancelToken::new()).await.unwrap().len(),
            1
        );
    });
}

#[test]
fn a_list_change_from_a_legacy_server_bumps_the_changes() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").list_changed();
        let client = connect(&server, &timer).await;
        assert!(client.info().list_changed);
        let mut changes = client.changes();
        server.notify_list_changed().await;
        changes.changed().await.unwrap();
        assert_eq!(*changes.borrow_and_update(), 1);
    });
}

#[test]
fn a_legacy_server_is_never_asked_for_a_change_stream() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-11-25").list_changed();
        let client = connect(&server, &timer).await;
        client.list_tools(&CancelToken::new()).await.unwrap();
        settle().await;
        assert!(server.of("subscriptions/listen").is_empty());
        assert!(!client.listening());
    });
}

#[test]
fn a_burst_of_changes_wakes_a_receiver_that_was_not_looking_once() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").list_changed();
        let client = connect(&server, &timer).await;
        let mut changes = client.changes();
        for _ in 0..10 {
            server.notify_list_changed().await;
        }
        changes.wait_for(|count| *count == 10).await.unwrap();
        changes.borrow_and_update();
        assert!(!changes.has_changed().unwrap(), "one wake for the burst");
    });
}

#[test]
fn listings_asked_for_while_one_runs_share_it() {
    run(async {
        let timer = timer();
        let gate = Gate::new();
        let server = FakeServer::legacy("2025-06-18")
            .tools(vec![tool("a"), tool("b")])
            .list_gate(&gate);
        let client = connect(&server, &timer).await;
        let cancel = CancelToken::new();
        let lists = async {
            let first = client.list_tools(&cancel);
            let second = client.list_tools(&cancel);
            let third = client.list_tools(&cancel);
            let ((first, second), third) = join(join(first, second), third).await;
            (first.unwrap(), second.unwrap(), third.unwrap())
        };
        let ((first, second, third), ()) = join(lists, async {
            server.wait_for("tools/list", 1).await;
            settle().await;
            assert_eq!(server.of("tools/list").len(), 1, "one listing in flight");
            gate.open();
        })
        .await;
        assert_eq!(first.len(), 2);
        assert_eq!(first, second);
        assert_eq!(first, third);
        assert_eq!(server.of("tools/list").len(), 1);

        // Asked after it ended, a listing is a new one.
        client.list_tools(&cancel).await.unwrap();
        assert_eq!(server.of("tools/list").len(), 2);
    });
}

#[test]
fn a_change_during_a_listing_makes_who_asks_after_it_list_again() {
    run(async {
        let timer = timer();
        let gate = Gate::new();
        let server = FakeServer::legacy("2025-06-18")
            .list_changed()
            .tools(vec![tool("a")])
            .list_gate(&gate);
        let client = connect(&server, &timer).await;
        let cancel = CancelToken::new();
        let mut changes = client.changes();
        let first = client.list_tools(&cancel);
        let second = async {
            server.wait_for("tools/list", 1).await;
            server.notify_list_changed().await;
            changes.changed().await.unwrap();
            // Asked while the first listing runs, after the change.
            let (second, ()) = join(client.list_tools(&cancel), async {
                settle().await;
                gate.open();
            })
            .await;
            second
        };
        let (first, second) = join(first, second).await;
        first.unwrap();
        second.unwrap();
        assert_eq!(
            server.of("tools/list").len(),
            2,
            "a listing begun before a change is not shared after it"
        );
    });
}

#[test]
fn a_tool_that_runs_only_as_a_task_is_listed_as_sent_and_said_to_be_unusable() {
    run(async {
        let timer = timer();
        let task = json!({
            "name": "long",
            "inputSchema": {"type": "object"},
            "execution": {"taskSupport": "required"},
        });
        let server = FakeServer::legacy("2025-11-25").tools(vec![task.clone(), tool("short")]);
        let client = connect(&server, &timer).await;
        let tools = client.list_tools(&CancelToken::new()).await.unwrap();
        assert_eq!(tools[0].raw, task, "kept exactly as sent");
        assert!(tools[0].unusable().is_some());
        assert_eq!(tools[1].unusable(), None);
    });
}

#[test]
fn a_listing_reads_every_page_in_both_eras_an_empty_cursor_included() {
    for server in [FakeServer::legacy("2025-06-18"), FakeServer::modern()] {
        run(async {
            let timer = timer();
            let names = ["a", "b", "c", "d", "e"];
            let server = server
                .tools(names.iter().map(|name| tool(name)).collect())
                .page_size(2);
            let client = connect(&server, &timer).await;
            let tools = client.list_tools(&CancelToken::new()).await.unwrap();
            let listed: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
            assert_eq!(listed, names);
            let cursors: Vec<Json> = server
                .of("tools/list")
                .iter()
                .map(|request| request["params"]["cursor"].clone())
                .collect();
            assert_eq!(cursors, [Json::Null, json!(""), json!("2")]);
        });
    }
}
