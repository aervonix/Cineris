//! A server that sends too much: more messages or bytes in a minute than
//! answer anything, a line over the pipes' cap, what is not a message.

use cineris_base::CancelToken;
use cineris_mcp::fake::Reply;
use cineris_mcp::{Client, ClientError};
use cineris_testkit::join;

use super::*;

fn log_line(words: &str) -> Json {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/message",
        "params": {"level": "info", "data": words},
    })
}

/// Whether `error` is the end of a server stopped as misbehaving.
fn misbehaved(error: &ClientError) -> bool {
    matches!(error, ClientError::Protocol(why) if why.contains("misbehaving"))
}

#[test]
fn a_server_that_sends_notices_in_a_loop_is_stopped_and_another_server_goes_on() {
    run(async {
        let timer = timer();
        let noisy = FakeServer::legacy("2025-06-18").reply("held", Reply::Silent);
        let quiet = FakeServer::modern().tools(vec![tool("a")]);
        let loud = connect(&noisy, &timer).await;
        let calm = connect(&quiet, &timer).await;
        let cancel = CancelToken::new();
        let (held, ()) = join(loud.call("held", json!({}), &cancel), async {
            noisy.wait_for("tools/call", 1).await;
            // Each one valid, and each one answering nothing.
            noisy.flood(1001, log_line("still here")).await;
        })
        .await;
        let error = held.unwrap_err();
        assert!(misbehaved(&error), "{error:?}");
        assert_eq!(loud.closed().await, error);
        until(|| noisy.shutdowns() == [GRACE]).await;

        // The other server's work goes on.
        assert_eq!(calm.list_tools(&cancel).await.unwrap().len(), 1);
        calm.call("a", json!({}), &cancel).await.unwrap();
    });
}

#[test]
fn every_kind_of_message_that_answers_nothing_counts() {
    let kinds = [
        json!({"jsonrpc": "2.0", "id": 99_999, "result": {}}),
        json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
        json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progressToken": 1, "progress": 1}}),
        json!({"jsonrpc": "2.0", "id": "p", "method": "ping"}),
        json!({"jsonrpc": "2.0", "error": {"code": -32700, "message": "parse error"}}),
        json!("not a message"),
    ];
    for kind in kinds {
        run(async {
            let timer = timer();
            let server = FakeServer::legacy("2025-06-18").list_changed();
            let client = connect(&server, &timer).await;
            server.flood(1001, kind.clone()).await;
            let error = client.closed().await;
            assert!(misbehaved(&error), "{kind}: {error:?}");
        });
    }
}

#[test]
fn the_allowance_is_a_minutes() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18");
        let client = connect(&server, &timer).await;
        let cancel = CancelToken::new();
        server.flood(1000, log_line("a minute's worth")).await;
        // Read in order: the answer comes after the notices.
        client.call("a", json!({}), &cancel).await.unwrap();
        timer.sleeping_for(Duration::from_secs(60)).await;
        timer.advance(Duration::from_secs(60));
        server.flood(1000, log_line("the next minute's")).await;
        client.call("b", json!({}), &cancel).await.unwrap();
        server.flood(1, log_line("one too many")).await;
        assert!(misbehaved(&client.closed().await));
    });
}

#[test]
fn sixteen_mib_of_notices_in_a_minute_is_too_much() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern();
        let client = connect(&server, &timer).await;
        let mebibyte = "x".repeat(1024 * 1024);
        server.flood(15, log_line(&mebibyte)).await;
        client.list_tools(&CancelToken::new()).await.unwrap();
        server.flood(2, log_line(&mebibyte)).await;
        let error = client.closed().await;
        assert!(misbehaved(&error), "{error:?}");
        let ClientError::Protocol(why) = error else {
            unreachable!()
        };
        assert!(why.contains("16 MiB"), "{why}");
    });
}

#[test]
fn a_line_longer_than_the_cap_ends_the_connection() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").reply("held", Reply::Silent);
        let client = connect(&server, &timer).await;
        let (result, ()) = join(client.call("held", json!({}), &CancelToken::new()), async {
            server.wait_for("tools/call", 1).await;
            server.send_too_long().await;
        })
        .await;
        let Err(ClientError::Protocol(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("longer than"), "{why}");
        assert!(matches!(client.closed().await, ClientError::Protocol(_)));
    });
}

#[test]
fn what_is_not_a_message_is_left_and_the_connection_goes_on() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern();
        let client = connect(&server, &timer).await;
        server.send_line(b"Server listening on stdio").await;
        server.send_line(b"{\"jsonrpc\": \"2.0\"").await;
        server.send_line(b"[]").await;
        client
            .call("a", json!({}), &CancelToken::new())
            .await
            .unwrap();
    });
}
