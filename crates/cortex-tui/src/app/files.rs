//! `@` completion filtered off the key's path (since step 23): the
//! workspace's paths, up to the core's 100,000, are matched on a thread of
//! their own, so that a key's frame never waits for them
//! (`docs/performance-budgets.md`: at the cap, matching takes longer than a
//! key's whole budget).
//!
//! The input hands each query out with its generation
//! ([`Input::take_file_query`](crate::input::Input::take_file_query)); the
//! worker matches the latest it has been given, drops any older one not yet
//! begun, and sends the rows back with the generation they are for, waking
//! the loop; the input takes only rows of its latest query
//! ([`Input::file_rows`](crate::input::Input::file_rows)).

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use cortex_term::Waker;

use crate::complete::fuzzy::{Candidates, Filter};
use crate::complete::{self, Row};
use crate::input::FileQuery;

/// What the worker has been given and not yet begun.
#[derive(Debug, Default)]
struct Asked {
    latest: Option<FileQuery>,
    /// The application has ended: the worker stops.
    done: bool,
}

/// The worker that matches the workspace's paths.
pub(crate) struct FileSearch {
    asked: Arc<(Mutex<Asked>, Condvar)>,
    answers: Receiver<(u64, Vec<Row>)>,
}

impl std::fmt::Debug for FileSearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSearch").finish_non_exhaustive()
    }
}

impl FileSearch {
    /// Starts the worker over `paths`, which wakes the loop through `wake`
    /// when it has rows. The paths are moved to it, not copied, and made
    /// into the compact list there: at the core's cap that takes longer than
    /// a key's budget. They come back when its thread could not be started.
    pub(crate) fn start(paths: Vec<String>, wake: Option<Waker>) -> Result<Self, Vec<String>> {
        let asked: Arc<(Mutex<Asked>, Condvar)> = Arc::default();
        let (send, answers) = mpsc::channel();
        let shared = Arc::clone(&asked);
        // Handed over once the thread runs, so that a thread that could not
        // be started leaves them here.
        let (give, take) = mpsc::channel::<Vec<String>>();
        let started = std::thread::Builder::new()
            .name("cortex-files".to_owned())
            .spawn(move || {
                if let Ok(paths) = take.recv() {
                    let candidates = Candidates::new(paths);
                    work(&candidates, &shared, &send, wake.as_ref());
                }
            });
        if started.is_err() {
            return Err(paths);
        }
        if let Err(mpsc::SendError(paths)) = give.send(paths) {
            return Err(paths);
        }
        Ok(Self { asked, answers })
    }

    /// Gives the worker `query`, in place of any it has not begun.
    pub(crate) fn ask(&self, query: FileQuery) {
        let (lock, wakes) = &*self.asked;
        lock.lock().unwrap_or_else(PoisonError::into_inner).latest = Some(query);
        wakes.notify_one();
    }

    /// The rows the worker has sent since the last call, oldest first.
    pub(crate) fn answers(&self) -> Vec<(u64, Vec<Row>)> {
        self.answers.try_iter().collect()
    }

    /// Waits at most `limit` for the next rows: for tests, which cannot
    /// wait for the loop's wake.
    #[cfg(test)]
    pub(crate) fn wait(&self, limit: std::time::Duration) -> Option<(u64, Vec<Row>)> {
        self.answers.recv_timeout(limit).ok()
    }
}

impl Drop for FileSearch {
    fn drop(&mut self) {
        let (lock, wakes) = &*self.asked;
        lock.lock().unwrap_or_else(PoisonError::into_inner).done = true;
        wakes.notify_one();
    }
}

fn work(
    candidates: &Candidates,
    asked: &(Mutex<Asked>, Condvar),
    send: &Sender<(u64, Vec<Row>)>,
    wake: Option<&Waker>,
) {
    let (lock, wakes) = asked;
    let mut filter = Filter::new();
    loop {
        let query = {
            let mut asked = lock.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if asked.done {
                    return;
                }
                if let Some(query) = asked.latest.take() {
                    break query;
                }
                asked = wakes.wait(asked).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let rows = complete::rows(&mut filter, candidates, &query.query, |_| String::new());
        if send.send((query.generation, rows)).is_err() {
            return;
        }
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}
