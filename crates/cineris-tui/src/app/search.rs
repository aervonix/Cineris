//! The full-screen presenter's search over the kept transcript, off the
//! key's path (since step 25; `docs/steps/25/answered.md`, point 2): each
//! kept block's text is handed once, as it is kept, to a thread of its own,
//! which counts a query's matches in every block and answers with the
//! blocks that hold it, for the latest query only, waking the loop. A key's
//! frame never waits for it: the search row shows what the worker last
//! gave.
//!
//! Matching ignores case, as the viewer's search does: the text and the
//! query are compared lower-cased.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use cineris_term::Waker;

/// A query's answer: its generation, and each block that holds it with how
/// many times, in the blocks' order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) generation: u64,
    pub(crate) blocks: Vec<(usize, usize)>,
}

/// What the worker has been given and not yet taken.
#[derive(Debug, Default)]
struct Asked {
    /// Blocks' texts to add, by their place among the kept blocks.
    added: Vec<(usize, String)>,
    /// Everything kept before is gone (the kept transcript was made again).
    cleared: bool,
    /// The latest query, with its generation.
    latest: Option<(u64, String)>,
    /// The application has ended: the worker stops.
    done: bool,
}

/// The worker. See the [module documentation](self).
pub(crate) struct TextSearch {
    asked: Arc<(Mutex<Asked>, Condvar)>,
    answers: Receiver<Found>,
    /// The generation of the last query asked.
    generation: u64,
    /// An answer a test waited for and left to be taken (`wait_ready`).
    #[cfg(test)]
    held: std::cell::RefCell<Option<Found>>,
}

impl std::fmt::Debug for TextSearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextSearch")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl TextSearch {
    /// Starts the worker, which wakes the loop through `wake` when it has an
    /// answer. `None` when its thread could not be started: there is then no
    /// search, and the search row says so.
    pub(crate) fn start(wake: Option<Waker>) -> Option<Self> {
        let asked: Arc<(Mutex<Asked>, Condvar)> = Arc::default();
        let (send, answers) = mpsc::channel();
        let shared = Arc::clone(&asked);
        std::thread::Builder::new()
            .name("cineris-search".to_owned())
            .spawn(move || work(&shared, &send, wake.as_ref()))
            .ok()?;
        Some(Self {
            asked,
            answers,
            generation: 0,
            #[cfg(test)]
            held: std::cell::RefCell::default(),
        })
    }

    /// Hands over the text of the blocks kept since the last call.
    pub(crate) fn add(&self, blocks: Vec<(usize, String)>) {
        if blocks.is_empty() {
            return;
        }
        let (lock, wakes) = &*self.asked;
        lock.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .added
            .extend(blocks);
        wakes.notify_one();
    }

    /// Forgets every block handed over: the kept transcript was made again.
    pub(crate) fn clear(&self) {
        let (lock, wakes) = &*self.asked;
        let mut asked = lock.lock().unwrap_or_else(PoisonError::into_inner);
        asked.added.clear();
        asked.cleared = true;
        wakes.notify_one();
    }

    /// Asks for `query`'s matches, in place of any query not begun; gives
    /// its generation.
    pub(crate) fn ask(&mut self, query: &str) -> u64 {
        self.generation += 1;
        let (lock, wakes) = &*self.asked;
        lock.lock().unwrap_or_else(PoisonError::into_inner).latest =
            Some((self.generation, query.to_lowercase()));
        wakes.notify_one();
        self.generation
    }

    /// The answer to the latest query, if it has come: older ones are
    /// dropped.
    pub(crate) fn answer(&self) -> Option<Found> {
        let newest = self
            .answers
            .try_iter()
            .filter(|found| found.generation == self.generation)
            .last();
        #[cfg(test)]
        let newest = newest.or_else(|| {
            self.held
                .take()
                .filter(|found| found.generation == self.generation)
        });
        newest
    }

    /// Waits at most `limit` for the latest query's answer and leaves it to
    /// be taken, as the worker's own would be: for tests of what takes it.
    #[cfg(test)]
    pub(crate) fn wait_ready(&self, limit: std::time::Duration) -> bool {
        let found = self.wait(limit);
        let ready = found.is_some();
        *self.held.borrow_mut() = found;
        ready
    }

    /// Waits at most `limit` for the latest query's answer: for tests,
    /// which cannot wait for the loop's wake.
    #[cfg(test)]
    pub(crate) fn wait(&self, limit: std::time::Duration) -> Option<Found> {
        let until = std::time::Instant::now() + limit;
        loop {
            let left = until.saturating_duration_since(std::time::Instant::now());
            match self.answers.recv_timeout(left) {
                Ok(found) if found.generation == self.generation => return Some(found),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }
}

impl Drop for TextSearch {
    fn drop(&mut self) {
        let (lock, wakes) = &*self.asked;
        lock.lock().unwrap_or_else(PoisonError::into_inner).done = true;
        wakes.notify_one();
    }
}

/// How many times `query` is in `text`, not overlapping.
pub(crate) fn count(text: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    text.match_indices(query).count()
}

fn work(asked: &(Mutex<Asked>, Condvar), send: &Sender<Found>, wake: Option<&Waker>) {
    let (lock, wakes) = asked;
    let mut texts: Vec<(usize, String)> = Vec::new();
    loop {
        let (added, cleared, query) = {
            let mut asked = lock.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if asked.done {
                    return;
                }
                if asked.cleared || !asked.added.is_empty() || asked.latest.is_some() {
                    break (
                        std::mem::take(&mut asked.added),
                        std::mem::take(&mut asked.cleared),
                        asked.latest.take(),
                    );
                }
                asked = wakes.wait(asked).unwrap_or_else(PoisonError::into_inner);
            }
        };
        if cleared {
            texts.clear();
        }
        texts.extend(
            added
                .into_iter()
                .map(|(block, text)| (block, text.to_lowercase())),
        );
        let Some((generation, query)) = query else {
            continue;
        };
        let blocks = texts
            .iter()
            .filter_map(|(block, text)| {
                let found = count(text, &query);
                (found > 0).then_some((*block, found))
            })
            .collect();
        if send.send(Found { generation, blocks }).is_err() {
            return;
        }
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const BOUND: Duration = Duration::from_secs(8);

    #[test]
    fn it_counts_each_blocks_matches_ignoring_case_and_answers_the_latest_query() {
        let mut search = TextSearch::start(None).expect("a thread");
        search.add(vec![
            (0, "Separator, separator".to_owned()),
            (1, "nothing here".to_owned()),
            (2, "a SEPARATOR".to_owned()),
        ]);
        search.ask("sep");
        let found = search.ask("separator");
        let answer = search.wait(BOUND).expect("an answer");
        assert_eq!(answer.generation, found);
        assert_eq!(answer.blocks, vec![(0, 2), (2, 1)]);
        // Blocks added later are searched too; cleared ones are not.
        search.add(vec![(3, "one more separator".to_owned())]);
        search.ask("separator");
        assert_eq!(
            search.wait(BOUND).expect("an answer").blocks,
            vec![(0, 2), (2, 1), (3, 1)]
        );
        search.clear();
        search.ask("separator");
        assert_eq!(search.wait(BOUND).expect("an answer").blocks, vec![]);
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        assert_eq!(count("abc", ""), 0);
        assert_eq!(count("aaaa", "aa"), 2);
    }
}
