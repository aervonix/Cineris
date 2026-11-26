//! The order a front end's operations run in: one lane for what changes a
//! session, reads beside it (`docs/steps/23/answered.md`, point 1;
//! `docs/architecture/02-runtime.md`, "Event backpressure").
//!
//! Every operation that changes a session (`turn/start`, `turn/interrupt`,
//! `session/set`, `session/compact`, `session/rewind`, `session/undo`,
//! `session/new`, `session/open`, `session/close`, …) runs in **one lane,
//! one at a time, in the order sent**: an interrupt cannot overtake the
//! prompt it is meant to stop. A read (`session/list`, `session/read`,
//! `models/list`, `config/get`, …) runs on a task of its own beside the
//! lane, unless a write sent before it is not done yet: it then goes into
//! the lane after that write, so that a read sees every write sent before it
//! (the review of step 23's second pull request).
//!
//! The in-process link of the terminal UI and each RPC connection have a
//! [`Dispatcher`] of their own: the same order whichever carrier a client
//! uses, which the conformance test relies on. `shutdown` and `initialize`
//! are not given to it: each carrier handles them itself, outside the lane,
//! so that a lane stuck behind an open cannot hold the end of the program.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cineris_protocol::methods::{Op, OpResult};
use cineris_protocol::{ErrorData, ErrorKind};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// What an operation ends with, as the core gives it.
pub type Outcome = Result<OpResult, Box<ErrorData>>;

/// Carries out one operation: the core's `handle`, for one client.
pub type Run = Arc<dyn Fn(Op) -> Pin<Box<dyn Future<Output = Outcome> + Send>> + Send + Sync>;

/// What an operation does the moment it is sent, before it is routed: the
/// core's `early`, for one client (since step 28: a `turn/interrupt` stops
/// an extension's command running in the lane's `turn/start`, which it
/// would otherwise wait behind). It must not wait.
pub type Early = Arc<dyn Fn(&Op) + Send + Sync>;

/// Where an operation runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// On a task of its own, beside the lane.
    Beside,
    /// In the lane, in the order sent; `write` when it changes a session.
    Lane {
        /// Whether it changes a session, and so holds the reads sent after
        /// it back until it is done.
        write: bool,
    },
}

/// Whether an operation only reads, and so may run beside the ordered ones.
#[must_use]
pub fn reads(op: &Op) -> bool {
    matches!(
        op,
        Op::SessionList(_)
            | Op::SessionRead(_)
            | Op::WorkspaceFiles(_)
            | Op::ProjectDetect(_)
            | Op::ModelsList(_)
            | Op::ConfigGet(_)
            | Op::PermissionsList(_)
            | Op::PlanGet(_)
            | Op::UsageGet(_)
            | Op::UsageReport(_)
            | Op::UsageRefresh(_)
            | Op::ProjectList(_)
            | Op::SessionArtifact(_)
            | Op::SessionRewindPlan(_)
            | Op::WorkspaceWarm(_)
            | Op::McpStatus(_)
            | Op::ExtensionsStatus(_)
            | Op::CommandsList(_)
    )
}

/// How the operations are ordered: the count of writes routed and not done.
#[derive(Debug, Clone, Default)]
pub struct Routes {
    in_flight: Arc<AtomicUsize>,
}

impl Routes {
    /// Where `op` runs. A write is counted until it is done.
    pub fn route(&self, op: &Op) -> Route {
        if !reads(op) {
            self.in_flight.fetch_add(1, Ordering::SeqCst);
            return Route::Lane { write: true };
        }
        // The workspace's files are asked for only once an open has
        // answered, and depend on no write after it: they never wait in the
        // lane, behind a fork's close of the session it forked (the
        // re-review of step 23's second pull request).
        if matches!(op, Op::WorkspaceFiles(_)) {
            return Route::Beside;
        }
        // Nor do two reads of step 24's panels: a refresh of the usage
        // limits may wait up to 20 s on a provider and must never hold a
        // prompt behind it in the lane, and reading named files ahead
        // depends on nothing a write changes (the review of step 24's
        // design, finding 7). A refresh reads `usage.refresh` as it is in
        // force when it runs.
        if matches!(op, Op::UsageRefresh(_) | Op::WorkspaceWarm(_)) {
            return Route::Beside;
        }
        if self.in_flight.load(Ordering::SeqCst) > 0 {
            Route::Lane { write: false }
        } else {
            Route::Beside
        }
    }

    /// A write the lane is running, counted as done when what this gives is
    /// dropped: once it has answered, and also if it panicked.
    fn running(&self, write: bool) -> Running {
        Running {
            routes: write.then(|| self.clone()),
        }
    }

    /// A write is done.
    fn written(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A write under way in the lane: done when dropped.
struct Running {
    routes: Option<Routes>,
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(routes) = &self.routes {
            routes.written();
        }
    }
}

/// Runs one operation on a task of its own, so that a panic in it is that
/// operation's failure and not its runner's: it is answered `Internal`, and
/// whatever ran it goes on (the review of step 23: a panic ended the lane,
/// leaving everything behind it unanswered).
#[expect(
    clippy::result_large_err,
    reason = "the result goes straight to the client's results, which hold it so"
)]
async fn contained(
    work: Pin<Box<dyn Future<Output = Outcome> + Send>>,
) -> Result<OpResult, ErrorData> {
    match tokio::spawn(work).await {
        Ok(result) => result.map_err(|error| *error),
        Err(_) => Err(ErrorData::new(
            ErrorKind::Internal,
            "operation_failed",
            "The operation failed inside the core; the log file has the details.",
        )),
    }
}

/// The answer to an operation that could not be run: its lane is gone.
#[must_use]
pub fn core_gone() -> ErrorData {
    ErrorData::new(ErrorKind::Internal, "core_gone", "The core has stopped.")
}

/// Runs a client's operations in the lane's order and hands each result to
/// `done` with the ticket it was sent with. Must be made, and sent to,
/// inside the core's runtime.
pub struct Dispatcher<T> {
    routes: Routes,
    run: Run,
    early: Option<Early>,
    done: Arc<dyn Fn(T, Result<OpResult, ErrorData>) + Send + Sync>,
    lane: Option<mpsc::UnboundedSender<(T, Op, bool)>>,
    task: Option<JoinHandle<()>>,
}

impl<T> std::fmt::Debug for Dispatcher<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dispatcher")
            .field("routes", &self.routes)
            .finish_non_exhaustive()
    }
}

impl<T: Send + 'static> Dispatcher<T> {
    /// Starts the lane's task. `run` carries out an operation; `done` is
    /// told each result, from the core's thread.
    pub fn start(
        run: Run,
        done: Arc<dyn Fn(T, Result<OpResult, ErrorData>) + Send + Sync>,
    ) -> Self {
        Self::start_with(run, None, done)
    }

    /// [`start`](Self::start), with `early` told of each operation as it
    /// is sent, before it is routed.
    pub fn start_with(
        run: Run,
        early: Option<Early>,
        done: Arc<dyn Fn(T, Result<OpResult, ErrorData>) + Send + Sync>,
    ) -> Self {
        let routes = Routes::default();
        let (lane, written) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_lane(
            written,
            routes.clone(),
            Arc::clone(&run),
            Arc::clone(&done),
        ));
        Self {
            routes,
            run,
            early,
            done,
            lane: Some(lane),
            task: Some(task),
        }
    }

    /// Runs `op` where it belongs: in the lane, or on a task of its own
    /// beside it. Never waits.
    pub fn send(&self, ticket: T, op: Op) {
        if let Some(early) = &self.early {
            early(&op);
        }
        match self.routes.route(&op) {
            Route::Lane { write } => {
                let refused = match &self.lane {
                    Some(lane) => lane
                        .send((ticket, op, write))
                        .err()
                        .map(|mpsc::error::SendError((ticket, _, _))| ticket),
                    None => Some(ticket),
                };
                // The lane is gone (closed, or its task ended): what it is
                // sent is answered here.
                if let Some(ticket) = refused {
                    if write {
                        self.routes.written();
                    }
                    (self.done)(ticket, Err(core_gone()));
                }
            }
            Route::Beside => {
                let (run, done) = (Arc::clone(&self.run), Arc::clone(&self.done));
                tokio::spawn(async move {
                    let result = contained(run(op)).await;
                    done(ticket, result);
                });
            }
        }
    }

    /// Takes no more operations into the lane, and gives back its task: it
    /// ends once the operations already in it are done. The caller waits
    /// for it as long as it chooses.
    pub fn close(&mut self) -> Option<JoinHandle<()>> {
        self.lane = None;
        self.task.take()
    }
}

/// The lane: runs what it is sent one at a time, in the order sent, and
/// answers each. An operation that panics is answered `Internal` and the
/// lane goes on: the ones queued behind it are run and answered, and its
/// write is counted as done.
async fn run_lane<T>(
    mut written: mpsc::UnboundedReceiver<(T, Op, bool)>,
    routes: Routes,
    run: Run,
    done: Arc<dyn Fn(T, Result<OpResult, ErrorData>) + Send + Sync>,
) {
    while let Some((ticket, op, write)) = written.recv().await {
        let running = routes.running(write);
        let result = contained(run(op)).await;
        drop(running);
        done(ticket, result);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use cineris_protocol::methods::{
        Empty, PermissionsRevokeParams, SessionRef, WorkspaceFilesParams,
    };
    use cineris_protocol::{ApprovalId, SessionId};

    use super::*;

    fn list(session: &SessionId) -> Op {
        Op::PermissionsList(SessionRef {
            session_id: session.clone(),
        })
    }

    fn revoke(session: &SessionId) -> Op {
        Op::PermissionsRevoke(PermissionsRevokeParams {
            session_id: session.clone(),
            ids: vec![ApprovalId::from("a_1")],
        })
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
    }

    /// A read runs beside the lane while no write is under way, and after
    /// the writes sent before it while one is: a list after a revoke sees
    /// the revoke. The workspace's files never wait in the lane, and a write
    /// is done when its run is dropped, whether it answered or panicked.
    #[test]
    fn a_read_after_a_write_waits_for_it_and_one_alone_runs_beside() {
        let session = SessionId::from("s_1");
        let routes = Routes::default();
        assert_eq!(routes.route(&list(&session)), Route::Beside);
        assert_eq!(routes.route(&revoke(&session)), Route::Lane { write: true });
        assert_eq!(routes.route(&list(&session)), Route::Lane { write: false });
        let files = Op::WorkspaceFiles(WorkspaceFilesParams {
            session_id: session.clone(),
            limit: None,
        });
        assert_eq!(
            routes.route(&files),
            Route::Beside,
            "files waited in the lane"
        );
        drop(routes.running(true));
        assert_eq!(routes.route(&list(&session)), Route::Beside);
        // A run that panics is done all the same.
        let _ = routes.route(&revoke(&session));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _running = routes.running(true);
            panic!("a write that panics");
        }));
        assert!(panicked.is_err());
        assert_eq!(routes.route(&list(&session)), Route::Beside);
    }

    /// The panels' reads (step 24): a refresh of the usage limits and the
    /// files read ahead never wait in the lane, even behind a write, so a
    /// refresh that waits on a provider never holds a prompt behind it;
    /// the project's entries and an artifact are reads, which wait for the
    /// writes sent before them and run beside the lane otherwise; a reload
    /// changes sessions, and runs in the lane.
    #[test]
    fn the_panels_reads_run_beside_the_lane_and_a_refresh_never_waits_in_it() {
        use cineris_protocol::methods::{
            ConfigReloadParams, ProjectListParams, SessionArtifactParams, UsageRefreshParams,
            WorkspaceWarmParams,
        };
        let session = SessionId::from("s_1");
        let routes = Routes::default();
        let entries = || {
            Op::ProjectList(ProjectListParams {
                session_id: Some(session.clone()),
                ..ProjectListParams::default()
            })
        };
        let artifact = || {
            Op::SessionArtifact(SessionArtifactParams {
                session_id: session.clone(),
                artifact: cineris_protocol::ArtifactId::from("a_1"),
            })
        };
        let refresh = || Op::UsageRefresh(UsageRefreshParams::default());
        let warm = || {
            Op::WorkspaceWarm(WorkspaceWarmParams {
                session_id: session.clone(),
                paths: vec!["src/lib.rs".to_owned()],
            })
        };
        for read in [entries(), artifact(), refresh(), warm()] {
            assert_eq!(routes.route(&read), Route::Beside, "{read:?}");
        }
        let reload = Op::ConfigReload(ConfigReloadParams::default());
        assert_eq!(routes.route(&reload), Route::Lane { write: true });
        assert_eq!(routes.route(&entries()), Route::Lane { write: false });
        assert_eq!(routes.route(&artifact()), Route::Lane { write: false });
        assert_eq!(routes.route(&refresh()), Route::Beside, "a refresh waited");
        assert_eq!(routes.route(&warm()), Route::Beside, "a warm waited");
    }

    /// A rewind's plan is a read (since step 25): beside the lane, or after
    /// the writes sent before it; the rewind itself is a write.
    #[test]
    fn a_rewinds_plan_is_a_read_and_the_rewind_a_write() {
        use cineris_protocol::ItemId;
        use cineris_protocol::methods::{SessionRewindParams, SessionRewindPlanParams};
        let session = SessionId::from("s_1");
        let routes = Routes::default();
        let plan = || {
            Op::SessionRewindPlan(SessionRewindPlanParams {
                session_id: session.clone(),
                to: Some(ItemId::from("i_1")),
                before: None,
            })
        };
        assert_eq!(routes.route(&plan()), Route::Beside);
        let rewind = Op::SessionRewind(SessionRewindParams {
            session_id: session.clone(),
            to: Some(ItemId::from("i_1")),
            before: None,
            restore_files: true,
            plan: None,
        });
        assert_eq!(routes.route(&rewind), Route::Lane { write: true });
        assert_eq!(routes.route(&plan()), Route::Lane { write: false });
    }

    /// The first run's look at the project's commands (since step 25) is
    /// a read: beside the lane alone, behind a write sent before it.
    #[test]
    fn detecting_a_projects_commands_is_a_read() {
        let session = SessionId::from("s_1");
        let routes = Routes::default();
        let detect = || {
            Op::ProjectDetect(cineris_protocol::methods::ProjectDetectParams {
                session_id: session.clone(),
            })
        };
        assert!(reads(&detect()));
        assert_eq!(routes.route(&detect()), Route::Beside);
        assert_eq!(routes.route(&revoke(&session)), Route::Lane { write: true });
        assert_eq!(routes.route(&detect()), Route::Lane { write: false });
    }

    /// An operation that panics in the lane is answered as failed, and the
    /// ones queued behind it are run and answered in order; its write is
    /// done, so a read after it runs beside the lane again (the review of
    /// step 23: a panic ended the lane, leaving the app waiting for good).
    #[test]
    fn a_panic_in_the_lane_answers_that_operation_and_the_lane_goes_on() {
        let session = SessionId::from("s_1");
        type Answers = Vec<(u64, Result<(), String>)>;
        let answered: Arc<Mutex<Answers>> = Arc::default();
        let routes = runtime().block_on(async {
            let run: Run = Arc::new(|op| {
                Box::pin(async move {
                    match op {
                        Op::PermissionsRevoke(_) => panic!("an operation that panics"),
                        _ => Ok(OpResult::SessionClose(Empty {})),
                    }
                })
            });
            let told = Arc::clone(&answered);
            let mut dispatcher = Dispatcher::start(
                run,
                Arc::new(move |ticket: u64, result: Result<OpResult, ErrorData>| {
                    told.lock()
                        .unwrap()
                        .push((ticket, result.map(drop).map_err(|error| error.code)));
                }),
            );
            let close = Op::SessionClose(SessionRef {
                session_id: session.clone(),
            });
            for (ticket, op) in [revoke(&session), revoke(&session), close]
                .into_iter()
                .enumerate()
            {
                dispatcher.send(ticket as u64, op);
            }
            let routes = dispatcher.routes.clone();
            dispatcher.close().unwrap().await.unwrap();
            routes
        });
        assert_eq!(
            *answered.lock().unwrap(),
            [
                (0, Err("operation_failed".to_owned())),
                (1, Err("operation_failed".to_owned())),
                (2, Ok(()))
            ]
        );
        assert_eq!(
            routes.route(&list(&session)),
            Route::Beside,
            "a write is counted for good"
        );
    }

    /// A lane that is closed answers what it is sent `core_gone`, at once,
    /// and does not count it as a write under way.
    #[test]
    fn a_closed_lane_answers_what_it_is_sent() {
        let session = SessionId::from("s_1");
        let answered: Arc<Mutex<Vec<String>>> = Arc::default();
        runtime().block_on(async {
            let run: Run = Arc::new(|_| Box::pin(async { Ok(OpResult::SessionClose(Empty {})) }));
            let told = Arc::clone(&answered);
            let mut dispatcher = Dispatcher::start(
                run,
                Arc::new(move |_: u64, result: Result<OpResult, ErrorData>| {
                    if let Err(error) = result {
                        told.lock().unwrap().push(error.code);
                    }
                }),
            );
            let _ = dispatcher.close();
            dispatcher.send(1, revoke(&session));
            assert_eq!(dispatcher.routes.route(&list(&session)), Route::Beside);
        });
        assert_eq!(*answered.lock().unwrap(), ["core_gone"]);
    }
}
