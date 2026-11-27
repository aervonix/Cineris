//! The start, as the third review read it: a server known or found to be
//! modern is never sent `initialize`; a start given up or failed ends its
//! process at once; a server is started again only once the process let go
//! has ended; and each way a silent probe can end.

use std::future::poll_fn;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::task::Poll;

use cineris_mcp::fake::{Discover, Gate};
use cineris_mcp::{Client, ClientError, Era, ServerProcess};
use cineris_testkit::join;

use super::*;

/// The methods the server's process numbered `number` received.
fn methods_of(server: &FakeServer, number: usize) -> Vec<Json> {
    server
        .received_by(number)
        .iter()
        .map(|message| message["method"].clone())
        .collect()
}

fn legacy_only() -> Vec<String> {
    ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
        .iter()
        .map(|version| (*version).to_owned())
        .collect()
}

/// A server known to be modern whose process dies before it answers the
/// probe is started again and asked, never sent `initialize`. Written by
/// the review of the start.
#[test]
fn a_server_known_to_be_modern_whose_process_dies_on_the_probe_is_asked_again_not_initialized() {
    run(async {
        let timer = timer();
        // The first process dies before it answers; the next one would.
        let server = FakeServer::modern().on_discover(Discover::SilentFirst);
        let server_definition = definition();
        let connecting = StdioClient::connect_known(
            Era::Modern,
            &server_definition,
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        );
        let (result, ()) = join(connecting, async {
            server.wait_for("server/discover", 1).await;
            server.crash("it crashed while starting");
        })
        .await;
        assert!(
            server.of("initialize").is_empty(),
            "a server known to be modern was sent initialize: spawns {}, methods {:?}, result {:?}",
            server.spawns(),
            server.methods(),
            result.as_ref().map(|_| ())
        );
        assert_eq!(result.unwrap().info().era, Era::Modern);
        assert_eq!(server.spawns(), 2);
    });
}

/// A server found modern after `initialize` was sent (it refused it) is
/// started again and asked patiently; when that process dies before it
/// answers, the next is asked again, never sent `initialize`. Written by
/// the review of the start.
#[test]
fn a_server_found_modern_whose_next_process_dies_is_never_sent_initialize_again() {
    run(async {
        let timer = timer();
        let never = Gate::new();
        let server = FakeServer::modern().on_discover(Discover::Gated(never.clone()));
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
            // Process 1 refused `initialize`; process 2 is asked patiently.
            server
                .wait_until(|server| server.spawns() == 2 && !server.received_by(2).is_empty())
                .await;
            server.crash("it crashed while starting");
            server
                .wait_until(|server| server.spawns() == 3 && !server.received_by(3).is_empty())
                .await;
            timer.advance(START - PROBE);
        })
        .await;
        let third = methods_of(&server, 3);
        assert!(
            third.iter().all(|method| method != "initialize"),
            "a server found modern was sent initialize on process 3: {third:?}; result {:?}",
            result.as_ref().map(|_| ())
        );
        assert_eq!(third, ["server/discover"]);
        assert!(result.is_err());
    });
}

#[test]
fn a_server_known_to_be_modern_that_exits_twice_before_it_answers_is_not_started() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().on_discover(Discover::Exit);
        let result = StdioClient::connect_known(
            Era::Modern,
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
        assert!(why.contains("exited twice"), "{why}");
        assert_eq!(server.spawns(), 2, "one more process, and no more");
        assert!(server.of("initialize").is_empty());
    });
}

#[test]
fn a_server_known_to_be_modern_that_answers_as_a_legacy_server_is_not_started_nor_initialized() {
    for (discover, said) in [
        (Discover::Error(-32601), "error -32601"),
        (
            Discover::Unsupported {
                supported: legacy_only(),
                times: 99,
            },
            "only legacy revisions",
        ),
        (
            Discover::Result(json!({})),
            "a result that is not a discovery result",
        ),
    ] {
        run(async {
            let timer = timer();
            let server = FakeServer::legacy("2025-11-25").on_discover(discover.clone());
            let result = StdioClient::connect_known(
                Era::Modern,
                &definition(),
                server.spawn(),
                server.respawn(),
                shared(&timer),
                ClientOptions::default(),
            )
            .await;
            let Err(ClientError::NotStarted(why)) = result else {
                panic!("{said}: {result:?}");
            };
            assert!(why.contains(said), "{why}");
            assert!(server.of("initialize").is_empty(), "{said}");
        });
    }
}

/// A start its caller gives up (drops) ends its process: `connect` takes
/// no cancel token, so dropping is the way to give it up. Written by the
/// review of the start.
#[test]
fn a_start_given_up_by_its_caller_ends_its_process() {
    run(async {
        let timer = timer();
        let server = FakeServer::legacy("2025-06-18").on_discover(Discover::Silent);
        let server_definition = definition();
        let mut connecting = Box::pin(StdioClient::connect(
            &server_definition,
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        ));
        let mut probed = Box::pin(server.wait_for("server/discover", 1));
        poll_fn(|context| {
            if let Poll::Ready(result) = connecting.as_mut().poll(context) {
                panic!("the start ended: {:?}", result.map(|_| ()));
            }
            probed.as_mut().poll(context)
        })
        .await;
        drop(connecting);
        settle().await;
        assert_eq!(
            server.shutdowns(),
            [GRACE],
            "the process of a start given up was never ended"
        );
    });
}

#[test]
fn a_start_that_fails_returns_at_once_though_its_process_is_slow_to_end() {
    run(async {
        let timer = timer();
        let slow = Gate::new();
        let server = FakeServer::legacy("2023-01-01").shutdown_gate(&slow);
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
        settle().await;
        assert_eq!(server.shutdowns(), [GRACE], "ended in the background");
        slow.open();

        // And one that runs out of time returns at its limit.
        let slow = Gate::new();
        let never = Gate::new();
        let server = FakeServer::legacy("2025-06-18")
            .on_discover(Discover::Silent)
            .initialize_gate(&never)
            .shutdown_gate(&slow);
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
        slow.open();
    });
}

/// A process whose end is watched: by its own exit or its shutdown.
struct Watched {
    inner: Box<dyn ServerProcess>,
    ended: Arc<AtomicBool>,
}

impl ServerProcess for Watched {
    fn stderr_tail(&self) -> String {
        self.inner.stderr_tail()
    }

    fn exited(&self) -> cineris_tools::BoxFuture<'_, String> {
        Box::pin(async move {
            let words = self.inner.exited().await;
            self.ended.store(true, AtomicOrdering::SeqCst);
            words
        })
    }

    fn shutdown(&self, grace: Duration) -> cineris_tools::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.inner.shutdown(grace).await;
            self.ended.store(true, AtomicOrdering::SeqCst);
        })
    }
}

/// The server is started again only once the process let go has ended (a
/// `docker run --name`, a lock file or a fixed port refuses a second
/// instance). Written by the review of the start.
#[test]
fn a_server_is_started_again_only_once_the_process_let_go_has_ended() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().on_discover(Discover::SilentFirst);
        let ends: Arc<Mutex<Vec<Arc<AtomicBool>>>> = Arc::new(Mutex::new(Vec::new()));
        let alive_at_respawn = Arc::new(Mutex::new(Vec::new()));
        let watched = {
            let ends = Arc::clone(&ends);
            move |mut pipes: cineris_mcp::ServerPipes| {
                let ended = Arc::new(AtomicBool::new(false));
                ends.lock().unwrap().push(Arc::clone(&ended));
                pipes.process = Box::new(Watched {
                    inner: pipes.process,
                    ended,
                });
                pipes
            }
        };
        let first = watched.clone()(server.spawn());
        let respawn: cineris_mcp::Respawn = {
            let server = server.clone();
            let ends = Arc::clone(&ends);
            let alive = Arc::clone(&alive_at_respawn);
            let watched = watched.clone();
            Arc::new(move || {
                let running = ends
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|ended| !ended.load(AtomicOrdering::SeqCst))
                    .count();
                alive.lock().unwrap().push(running);
                Ok(watched(server.spawn()))
            })
        };
        let server_definition = definition();
        let connecting = StdioClient::connect(
            &server_definition,
            first,
            respawn,
            shared(&timer),
            ClientOptions::default(),
        );
        let (result, ()) = join(connecting, async {
            timer.sleeping_for(PROBE).await;
            timer.advance(PROBE);
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(
            *alive_at_respawn.lock().unwrap(),
            [0],
            "processes still running when the server was started again"
        );
    });
}

#[test]
fn a_silent_server_that_exits_during_initialize_is_started_again_and_only_initialized() {
    run(async {
        let timer = timer();
        // An `rmcp` legacy server's cold start: silent, then gone.
        let server = FakeServer::legacy("2025-06-18")
            .on_discover(Discover::Silent)
            .exits_on_first_initialize();
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
        })
        .await;
        assert_eq!(result.unwrap().info().era, Era::Legacy);
        assert_eq!(server.spawns(), 2);
        server.wait_for("notifications/initialized", 1).await;
        assert_eq!(methods_of(&server, 1), ["server/discover", "initialize"]);
        assert_eq!(
            methods_of(&server, 2),
            ["initialize", "notifications/initialized"]
        );
    });
}

#[test]
fn a_late_legacy_only_unsupported_version_while_initialize_waits_leaves_the_handshake() {
    run(async {
        let timer = timer();
        let (late, held) = (Gate::new(), Gate::new());
        let server = FakeServer::legacy("2025-11-25")
            .on_discover(Discover::Late(
                late.clone(),
                Box::new(Discover::Unsupported {
                    supported: legacy_only(),
                    times: 99,
                }),
            ))
            .initialize_gate(&held);
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
            late.open();
            settle().await;
            held.open();
        })
        .await;
        assert_eq!(result.unwrap().info().era, Era::Legacy);
        assert_eq!(
            server.spawns(),
            1,
            "the handshake went on, on the same process"
        );
    });
}

#[test]
fn a_late_modern_error_while_initialize_waits_starts_the_server_again() {
    for (late_answer, modern) in [
        (Discover::Error(-32021), false),
        (
            Discover::Unsupported {
                supported: vec!["2026-07-28".to_owned()],
                times: 1,
            },
            true,
        ),
    ] {
        run(async {
            let timer = timer();
            let (late, never) = (Gate::new(), Gate::new());
            let server = FakeServer::modern()
                .on_discover(Discover::Late(late.clone(), Box::new(late_answer.clone())))
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
                late.open();
            })
            .await;
            assert_eq!(server.spawns(), 2, "{late_answer:?}");
            assert_eq!(
                methods_of(&server, 2),
                ["server/discover"],
                "{late_answer:?}"
            );
            if modern {
                assert_eq!(result.unwrap().info().era, Era::Modern);
            } else {
                assert!(
                    matches!(result, Err(ClientError::NotStarted(_))),
                    "{result:?}"
                );
            }
        });
    }
}

#[test]
fn a_refusal_of_initialize_after_a_late_legacy_answer_ends_the_start_with_its_words() {
    run(async {
        let timer = timer();
        let (late, held) = (Gate::new(), Gate::new());
        let server = FakeServer::legacy("2025-06-18")
            .on_discover(Discover::Late(
                late.clone(),
                Box::new(Discover::Error(-32601)),
            ))
            .initialize_gate(&held)
            .initialize_error(-32602);
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
            late.open();
            settle().await;
            held.open();
        })
        .await;
        let Err(ClientError::NotStarted(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("initialize refused"), "{why}");
        assert_eq!(
            server.spawns(),
            1,
            "a legacy server's refusal is its last word"
        );
    });
}

#[test]
fn a_silent_server_that_refuses_initialize_and_stays_silent_ends_with_the_refusals_words() {
    run(async {
        let timer = timer();
        // A legacy server that ignores what it does not know, and refuses
        // `initialize`.
        let server = FakeServer::legacy("2025-06-18")
            .on_discover(Discover::Silent)
            .initialize_error(-32602);
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
            server
                .wait_until(|server| server.spawns() == 2 && !server.received_by(2).is_empty())
                .await;
            timer.advance(START - PROBE);
        })
        .await;
        let Err(ClientError::NotStarted(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("initialize refused"), "{why}");
        assert!(why.contains("did not answer"), "{why}");
    });
}

#[test]
fn a_server_that_cannot_be_started_again_is_told_why_it_was() {
    run(async {
        let timer = timer();
        // Known to be legacy, it refuses `initialize`, and cannot be started
        // again.
        let server = FakeServer::modern();
        let result = StdioClient::connect_known(
            Era::Legacy,
            &definition(),
            server.spawn(),
            no_respawn(),
            shared(&timer),
            ClientOptions::default(),
        )
        .await;
        let Err(ClientError::NotStarted(why)) = result else {
            panic!("{result:?}");
        };
        assert!(
            why.starts_with("it refused `initialize`, and could not be started again"),
            "{why}"
        );
    });
}

#[test]
fn a_server_silent_when_asked_again_with_a_revision_it_named_says_so() {
    run(async {
        let timer = timer();
        let server = FakeServer::modern().on_discover(Discover::Then(
            Box::new(Discover::Unsupported {
                supported: vec!["2026-07-28".to_owned()],
                times: 99,
            }),
            Box::new(Discover::Silent),
        ));
        let server_definition = definition();
        let connecting = StdioClient::connect(
            &server_definition,
            server.spawn(),
            server.respawn(),
            shared(&timer),
            ClientOptions::default(),
        );
        let (result, ()) = join(connecting, async {
            server.wait_for("server/discover", 2).await;
            timer.sleeping_for(PROBE).await;
            timer.advance(PROBE);
        })
        .await;
        let Err(ClientError::NotStarted(why)) = result else {
            panic!("{result:?}");
        };
        assert!(why.contains("it answered nothing"), "{why}");
    });
}
