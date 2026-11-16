//! A server: the core and its connections, and how they end together
//! (`docs/steps/27/answered.md`, "The design, before building", point 4).

use cortex_base::CancelToken;
use cortex_core::Core;

use crate::connection::{Connection, Ended, ServeOptions};
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
/// and what is queued is written, each wait bounded by `options`.
/// `ending` is cancelled when the end begins, for a caller that bounds the
/// whole of it: `Core::shutdown` waits for each session's close.
pub async fn serve_one(
    core: Core,
    transport: Transport,
    options: ServeOptions,
    stop: &CancelToken,
    ending: &CancelToken,
) -> Ended {
    let mut connection = Connection::start(core.clone(), transport, options);
    let ended = connection.run(stop).await;
    ending.cancel();
    // A `shutdown` request has shut the core down already.
    if ended != Ended::Shutdown {
        core.shutdown().await;
    }
    connection.finish(ended).await;
    ended
}
