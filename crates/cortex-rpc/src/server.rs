//! A server: the core and its connections, and how they end together
//! (`docs/steps/27/answered.md`, "The design, before building", point 4).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cortex_base::CancelToken;
use cortex_core::Core;

use crate::connection::{Connection, Ended, ServeOptions};
use crate::socket::{Accepted, Listener};
use crate::transport::Transport;

/// Serves one client until its stream ends, it asks for `shutdown`, it is
/// gone, stalled or flooding, or `stop` is cancelled; then ends the core:
/// `cortex serve --stdio`, whose one client is the process that started it.
///
/// The end, in order: the client is disconnected (what it was asked is
/// withdrawn, its sessions end) and its lane's operation waited for; the
/// core is shut down; the requests still running are waited for, so that a
/// session being put together when the end began ends itself in the core
/// rather than being cut off by the exit (the open item
/// `shutdown-and-a-session-being-put-together`); a `shutdown` is answered;
/// and what is queued is written, each wait bounded by `options`. After a
/// `shutdown`, the core's end comes first, as the request asks.
/// `ending` is cancelled when the end begins (before a `shutdown` ends the
/// core, or when serving stops), for a caller that bounds the whole of it:
/// `Core::shutdown` and the client's disconnect wait for each session's
/// close, which nothing bounds.
pub async fn serve_one(
    core: Core,
    transport: Transport,
    options: ServeOptions,
    stop: &CancelToken,
    ending: &CancelToken,
) -> Ended {
    let mut connection = Connection::start(core.clone(), transport, options);
    let ended = connection.run(stop, ending).await;
    // A `shutdown` request has shut the core down already.
    if ended != Ended::Shutdown {
        core.shutdown().await;
    }
    connection.finish(ended).await;
    ended
}

/// Serves every client that connects to `listener` until `stop` is
/// cancelled or a client's `shutdown` is taken: `cortex serve --socket`.
///
/// - **Several clients at once,** each a [`Connection`] of its own, at most
///   `most` at a time: one past that is closed as it comes. A connection
///   that fails the listener's checks is closed with nothing read or
///   written, and the log says why.
/// - **A client that goes leaves the server serving.** What the core keeps
///   of its sessions is the core's (`CoreOptions::keep_sessions`).
/// - **The end,** at a signal or a `shutdown`: no more connections are
///   taken; every connection ends (its client disconnected, its lane's
///   operation waited for); the core is shut down unless a `shutdown` did
///   it; each connection waits for its requests still running, answers a
///   `shutdown`, and writes what is queued, each bounded by `options`; and
///   the socket is removed when the listener is dropped. `ending` is
///   cancelled when the end begins, for a caller that bounds the whole of
///   it.
pub async fn serve_socket(
    core: Core,
    mut listener: Listener,
    options: ServeOptions,
    most: usize,
    stop: &CancelToken,
    ending: &CancelToken,
) {
    let mut connections = tokio::task::JoinSet::new();
    let shut = Arc::new(AtomicBool::new(false));
    loop {
        // Connections that ended are let go of before the next is taken.
        while connections.try_join_next().is_some() {}
        let Some(accepted) = stop.race(listener.accept()).await else {
            break;
        };
        match accepted {
            Ok(Accepted::Client { transport, peer }) => {
                if connections.len() >= most {
                    tracing::warn!(%peer, most, "a connection past the most at once is closed");
                    drop(transport);
                    continue;
                }
                tracing::debug!(%peer, "a client connected");
                let (core, options) = (core.clone(), options.clone());
                let (stop, ending, shut) = (stop.clone(), ending.clone(), Arc::clone(&shut));
                connections.spawn(async move {
                    let mut connection = Connection::start(core, transport, options);
                    connection.set_server_ending(ending);
                    let ended = connection.run(&stop, &CancelToken::new()).await;
                    if ended == Ended::Shutdown {
                        shut.store(true, Ordering::SeqCst);
                        // Every other connection ends with this one.
                        stop.cancel();
                    }
                    connection.finish(ended).await;
                });
            }
            Ok(Accepted::Refused(why)) => {
                tracing::warn!(why = %why, "a connection was refused");
            }
            Err(error) => {
                tracing::error!(%error, "the server can listen no more");
                stop.cancel();
                break;
            }
        }
    }
    ending.cancel();
    if !shut.load(Ordering::SeqCst) {
        core.shutdown().await;
    }
    let all = async { while connections.join_next().await.is_some() {} };
    let bound = options.lane_end + options.requests_end + options.drain;
    if crate::connection::within_public(&options.timer, bound, all)
        .await
        .is_none()
    {
        tracing::warn!("connections were still ending at the server's end");
        connections.abort_all();
    }
    drop(listener);
}
