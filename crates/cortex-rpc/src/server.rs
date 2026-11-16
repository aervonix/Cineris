//! A server: the core and its connections, and how they end together
//! (`docs/steps/27/answered.md`, "The design, before building", point 4).

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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

/// The first wait before an `accept` that failed for want of a resource is
/// tried again; each wait after it is twice the one before.
const ACCEPT_WAIT_FIRST: Duration = Duration::from_millis(50);

/// The longest wait between two tries of an `accept`.
const ACCEPT_WAIT_MOST: Duration = Duration::from_secs(1);

/// Whether an error of `accept` passes: the process or the system is out
/// of a resource for the moment (descriptors, memory, buffers), or the
/// client went before it was taken. Anything else is the listener's own end.
fn passes(error: &io::Error) -> bool {
    // Linux's numbers, and Windows's: the two systems the harness serves on.
    #[cfg(unix)]
    const OUT_OF: [i32; 4] = [
        12,  // ENOMEM
        23,  // ENFILE
        24,  // EMFILE
        105, // ENOBUFS
    ];
    #[cfg(windows)]
    const OUT_OF: [i32; 4] = [
        4,    // ERROR_TOO_MANY_OPEN_FILES
        8,    // ERROR_NOT_ENOUGH_MEMORY
        14,   // ERROR_OUTOFMEMORY
        1450, // ERROR_NO_SYSTEM_RESOURCES
    ];
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
            | io::ErrorKind::OutOfMemory
    ) || error
        .raw_os_error()
        .is_some_and(|code| OUT_OF.contains(&code))
}

/// Serves every client that connects to `listener` until `stop` is
/// cancelled or a client's `shutdown` is taken: `cortex serve --socket`.
///
/// - **Several clients at once,** each a [`Connection`] of its own, at most
///   `most` at a time: one past that is closed as it comes. A connection
///   gives its place back when it stops serving, before it ends, so that a
///   client that has gone is not counted against the next. A connection
///   that fails the listener's checks is closed with nothing served or
///   written, and the log says why.
/// - **A client that goes leaves the server serving.** What the core keeps
///   of its sessions is the core's (`CoreOptions::keep_sessions`).
/// - **An `accept` that fails for want of a resource** (descriptors,
///   memory) is tried again after a wait on `options.timer` that doubles
///   from 50 ms to at most 1 s; any other failure is the listener's end,
///   and the server's.
/// - **The end,** at a signal or a `shutdown`: no more connections are
///   taken and the socket is removed at once, so no client finds it while
///   the server ends; every connection ends (its client disconnected, its
///   lane's operation waited for); the core is shut down unless a
///   `shutdown` did it; and each connection waits for its requests still
///   running, answers a `shutdown`, and writes what is queued, each bounded
///   by `options`. `ending` is cancelled when the end begins, for a caller
///   that bounds the whole of it.
pub async fn serve_socket(
    core: Core,
    mut listener: Listener,
    options: ServeOptions,
    most: usize,
    stop: &CancelToken,
    ending: &CancelToken,
) {
    let mut connections = tokio::task::JoinSet::new();
    let places = Arc::new(tokio::sync::Semaphore::new(most));
    let shut = Arc::new(AtomicBool::new(false));
    let mut failed = 0_u32;
    loop {
        // Connections that ended are let go of: their places were given
        // back as they stopped serving.
        while connections.try_join_next().is_some() {}
        let Some(accepted) = stop.race(listener.accept()).await else {
            break;
        };
        match accepted {
            Ok(Accepted::Client { transport, peer }) => {
                failed = 0;
                let Ok(place) = Arc::clone(&places).try_acquire_owned() else {
                    tracing::warn!(%peer, most, "a connection past the most at once is closed");
                    drop(transport);
                    continue;
                };
                tracing::debug!(%peer, "a client connected");
                let (core, options) = (core.clone(), options.clone());
                let (stop, ending, shut) = (stop.clone(), ending.clone(), Arc::clone(&shut));
                connections.spawn(async move {
                    let mut connection = Connection::start(core, transport, options);
                    connection.set_server_ending(ending);
                    let ended = connection.run(&stop, &CancelToken::new()).await;
                    // It serves no more: its place is the next client's
                    // while it ends.
                    drop(place);
                    if ended == Ended::Shutdown {
                        shut.store(true, Ordering::SeqCst);
                        // Every other connection ends with this one.
                        stop.cancel();
                    }
                    connection.finish(ended).await;
                });
            }
            Ok(Accepted::Refused(why)) => {
                failed = 0;
                tracing::warn!(why = %why, "a connection was refused");
            }
            Err(error) if passes(&error) => {
                failed = failed.saturating_add(1);
                let wait = ACCEPT_WAIT_FIRST
                    .saturating_mul(1_u32 << (failed - 1).min(5))
                    .min(ACCEPT_WAIT_MOST);
                tracing::warn!(
                    %error,
                    wait_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                    "a connection could not be taken: the server tries again"
                );
                if stop.race(options.timer.sleep(wait)).await.is_none() {
                    break;
                }
            }
            Err(error) => {
                tracing::error!(%error, "the server can listen no more");
                stop.cancel();
                break;
            }
        }
    }
    // The socket goes first: no client finds it while the server ends.
    drop(listener);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Out of descriptors, memory or buffers, or a client gone before it
    /// was taken: the listener goes on. Anything else ends it.
    #[test]
    fn an_accept_out_of_a_resource_passes_and_any_other_failure_ends_the_listener() {
        #[cfg(unix)]
        let out_of = [12, 23, 24, 105];
        #[cfg(windows)]
        let out_of = [4, 8, 14, 1450];
        for code in out_of {
            assert!(passes(&io::Error::from_raw_os_error(code)), "{code}");
        }
        assert!(passes(&io::Error::from(io::ErrorKind::ConnectionAborted)));
        assert!(!passes(&io::Error::from(io::ErrorKind::NotFound)));
        assert!(!passes(&io::Error::from(io::ErrorKind::PermissionDenied)));
        assert!(!passes(&io::Error::other("the listener is gone")));
    }
}
