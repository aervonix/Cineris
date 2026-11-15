//! The repository map's seam: where the loop asks for one
//! (`docs/architecture/15-efficiency.md`, "Repository map";
//! `docs/architecture/07-sessions-and-context.md`, "What goes into a
//! request").
//!
//! The loop owns no I/O, so the map comes through [`MapSource`], which the
//! core fills from `cortex-index`. What the loop does with it:
//!
//! - **An epoch is given one map, once,** as a notice
//!   ([`NoticeSource::Map`](cortex_protocol::item::NoticeSource::Map)) in
//!   the user's role, which says it is the harness's (decision 0034): the
//!   map is made from the repository's own names, and must not carry the
//!   user's authority.
//! - **Where:** just before the epoch's first prompt when it is ready by
//!   then, waiting up to [`RepoMap::wait`] at the epoch's first chance (the
//!   interactive app waits 150 ms, headless does not wait); otherwise at the
//!   first safe boundary at which it is ready. A boundary is never inside
//!   a tool round, so the notice never sits between a call and its result.
//! - **Which epochs:** a compaction's starts without one (its summary
//!   replaces the history that held the last), and is given a fresh map. A
//!   change of model and a resume keep the history, and with it the map it
//!   holds: nothing is built again or compared, so the prefix and the
//!   cache are what they were. An epoch whose history holds none (a session
//!   from before the map, a map that was never ready) is given one at its
//!   next chance.
//! - **After a summary** the turns it did not cover are replayed without the
//!   old epoch's map notice, live and when the log is read again, so that
//!   the two histories are the same.

use std::sync::Arc;
use std::time::Duration;

use cortex_base::SharedTimer;
use tokio::sync::oneshot;

/// Where the repository map comes from. The core implements it.
pub trait MapSource: Send + Sync {
    /// Asks for a map of the workspace as it is now, with the files in
    /// `touched` (under the workspace, with `/`) ranked first. It returns at
    /// once; the map is built off the loop, and the answer is `None` when
    /// there is none to give (an empty workspace, an index that is off).
    fn request(&self, touched: Vec<String>) -> MapPending;

    /// A turn has begun: work in the background gives way to it.
    fn turn_started(&self) {}

    /// A turn has ended, and the session is idle: indexing that has more to
    /// do carries on.
    fn turn_ended(&self) {}
}

/// A map asked for and not yet given.
#[derive(Debug)]
pub struct MapPending {
    answer: oneshot::Receiver<Option<String>>,
}

/// Where the source sends the answer.
#[derive(Debug)]
pub struct MapAnswer {
    sender: oneshot::Sender<Option<String>>,
}

impl MapAnswer {
    /// Sends the map, or `None` when there is none.
    pub fn send(self, map: Option<String>) {
        let _ = self.sender.send(map);
    }
}

impl MapPending {
    /// A request and where its answer goes.
    #[must_use]
    pub fn channel() -> (MapAnswer, Self) {
        let (sender, answer) = oneshot::channel();
        (MapAnswer { sender }, Self { answer })
    }

    /// An answer that is known at once.
    #[must_use]
    pub fn ready(map: Option<String>) -> Self {
        let (answer, pending) = Self::channel();
        answer.send(map);
        pending
    }

    /// The answer if it has come: `Some(None)` when there is no map,
    /// `None` when it has not come yet. A source that went away without
    /// answering has no map.
    pub(crate) fn try_take(&mut self) -> Option<Option<String>> {
        match self.answer.try_recv() {
            Ok(map) => Some(map),
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => Some(None),
        }
    }

    /// Waits for the answer: the map, or `None` when there is none. Call it
    /// once: a second call after the answer has come does not return.
    pub async fn wait(&mut self) -> Option<String> {
        (&mut self.answer).await.unwrap_or(None)
    }
}

/// The map's part of a session's context.
#[derive(Clone)]
pub struct RepoMap {
    /// Where maps come from.
    pub source: Arc<dyn MapSource>,
    /// What the wait at an epoch's first chance waits on.
    pub timer: SharedTimer,
    /// How long that wait may be. Zero gives the map at the first
    /// boundary where it is ready, and never waits.
    pub wait: Duration,
}

impl std::fmt::Debug for RepoMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoMap")
            .field("wait", &self.wait)
            .finish_non_exhaustive()
    }
}
