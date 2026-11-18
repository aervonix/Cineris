//! Pieces of work shared out among the copies of the tree, for `cargo
//! xtask mutate --jobs N`.
//!
//! Each job is a thread with a copy of the tree and a build directory of
//! its own, and a [`Runner`] for them. A job takes the next piece of work
//! when it is free; what a piece came to goes back to the thread that
//! started the jobs, which alone writes the results and the report, in the
//! order the pieces finished. With one job there is no thread: the pieces
//! are done in order, and the time is asked before each, as it always was.
//!
//! **The answer "the time is up" is binding.** A job takes the next piece
//! under a lock that the starting thread holds while it asks the time and
//! writes the answer down, so a job either took its piece before the
//! question, or finds the answer when it asks for one. Without the lock a
//! job could be let go by something the question did (a gate a test opens,
//! a piece it was waiting on) and look for its next piece before the answer
//! was written: the cause of the one failure in 50 repeats of the test of
//! this, found by reading it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use super::drive::Runner;
use crate::Result;

/// How often the starting thread asks whether the time is up while every
/// job is busy.
const ASK_EVERY: Duration = Duration::from_millis(250);

/// Does `work` for each item, on the jobs, and hands each result to `done`
/// on this thread. Returns whether every item was done: not when the time
/// ran out (`out_of_time`), which stops new pieces being started while the
/// ones under way finish and are kept: a piece is started only by a job that
/// took it before `out_of_time` said so. An error from `done` stops the jobs
/// in the same way and is returned once they have.
pub(crate) fn for_each<T, R>(
    runners: &mut [&mut dyn Runner],
    copies: &[PathBuf],
    items: &[T],
    work: &(dyn Fn(&mut dyn Runner, &Path, &T) -> R + Sync),
    out_of_time: &mut dyn FnMut() -> bool,
    done: &mut dyn FnMut(&T, R) -> Result<()>,
) -> Result<bool>
where
    T: Sync,
    R: Send,
{
    if let ([runner], [copy]) = (&mut *runners, copies) {
        for item in items {
            if out_of_time() {
                return Ok(false);
            }
            let outcome = work(&mut **runner, copy, item);
            done(item, outcome)?;
        }
        return Ok(true);
    }

    // Asked once before any job starts, so that a run with no time left
    // starts nothing. After this it is asked as pieces finish, and every few
    // moments while all are busy. A job that took its piece before the
    // answer "up" is written down keeps it; none takes one after.
    if items.is_empty() {
        return Ok(true);
    }
    if out_of_time() {
        return Ok(false);
    }
    let next = AtomicUsize::new(0);
    let halt = AtomicBool::new(false);
    // Held to take a piece, and to ask the time and write the answer down.
    let claim = Mutex::new(());
    let (sender, received) = mpsc::channel::<(usize, R)>();
    std::thread::scope(|scope| {
        for (runner, copy) in runners.iter_mut().zip(copies) {
            let (sender, next, halt, claim) = (sender.clone(), &next, &halt, &claim);
            scope.spawn(move || {
                loop {
                    let (index, item) = {
                        let _claiming = claim
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if halt.load(Ordering::SeqCst) {
                            return;
                        }
                        let index = next.fetch_add(1, Ordering::SeqCst);
                        let Some(item) = items.get(index) else {
                            return;
                        };
                        (index, item)
                    };
                    let outcome = work(&mut **runner, copy, item);
                    if sender.send((index, outcome)).is_err() {
                        return;
                    }
                }
            });
        }
        drop(sender);

        let mut finished = 0;
        let mut failure = None;
        loop {
            if failure.is_none() && !halt.load(Ordering::SeqCst) {
                let _asking = claim
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if out_of_time() {
                    halt.store(true, Ordering::SeqCst);
                }
            }
            match received.recv_timeout(ASK_EVERY) {
                Ok((index, outcome)) => {
                    // After a failure, what is still finishing is let go
                    // unrecorded: the run is ending with an error.
                    if failure.is_none() {
                        finished += 1;
                        if let Err(error) = done(&items[index], outcome) {
                            let _stopping = claim
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            halt.store(true, Ordering::SeqCst);
                            failure = Some(error);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(finished == items.len()),
        }
    })
}

/// The copy of the tree that job `job` works in: `tree` for the first, as
/// a run without `--jobs` uses, and `tree-2`, `tree-3` … for the others.
pub(crate) fn copy(scratch: &Path, job: usize) -> PathBuf {
    scratch.join(numbered("tree", job))
}

/// `name` for the first job, `name-<n>` for the n-th after it.
pub(crate) fn numbered(name: &str, job: usize) -> String {
    if job == 0 {
        name.to_owned()
    } else {
        format!("{name}-{}", job + 1)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use cineris_testkit::Gate;

    use super::for_each;
    use crate::Result;
    use crate::mutate::drive::{Runner, Tested};

    /// A runner the work below never asks anything of.
    struct Idle;

    impl Runner for Idle {
        fn compiles(&mut self, _: &Path, _: &[String]) -> Result<std::result::Result<(), String>> {
            Ok(Ok(()))
        }

        fn test(&mut self, _: &Path, _: &[String]) -> Result<Tested> {
            Ok(Tested::Ran(Vec::new()))
        }
    }

    #[test]
    fn no_job_starts_a_piece_once_the_time_is_up() {
        // Two jobs, four pieces. Each piece waits for the gate, which the
        // run's own thread opens when it says the time is up, the second
        // time it asks: so by the time either job is free again, the time
        // is up, and neither starts another piece.
        let (mut first, mut second) = (Idle, Idle);
        let mut runners: Vec<&mut dyn Runner> = vec![&mut first, &mut second];
        let copies = [PathBuf::from("tree"), PathBuf::from("tree-2")];
        let gate = Gate::new();
        let started = Mutex::new(Vec::new());
        let mut asked = 0;
        let mut done = Vec::new();
        let finished = for_each(
            &mut runners,
            &copies,
            &[1, 2, 3, 4],
            &|_, _, item: &i32| {
                started.lock().unwrap().push(*item);
                // A bound on a mistake: with nothing to stop them, the
                // jobs go on after it.
                let _ = gate.wait(Duration::from_secs(2));
                *item
            },
            &mut || {
                asked += 1;
                if asked >= 2 {
                    // The jobs are let go before the answer is written
                    // down. Each is given the chance to look for a next
                    // piece in that moment (a few milliseconds of the
                    // clock, which a job that is let go needs far less
                    // of): they find the answer, since taking a piece
                    // waits for the lock this question is asked under.
                    gate.open();
                    let moment = Instant::now();
                    while moment.elapsed() < Duration::from_millis(5)
                        && started.lock().unwrap().len() <= 2
                    {
                        std::thread::yield_now();
                    }
                }
                asked >= 2
            },
            &mut |_, item| {
                done.push(item);
                Ok(())
            },
        )
        .unwrap();
        assert!(!finished, "the time ran out");
        let started = started.into_inner().unwrap();
        assert!(
            started.len() <= 2,
            "a piece was started after the time was up: {started:?}"
        );
        // What was started was finished and kept.
        done.sort_unstable();
        let mut expected = started.clone();
        expected.sort_unstable();
        assert_eq!(done, expected);
    }

    #[test]
    fn one_job_does_the_pieces_in_order_and_asks_the_time_before_each() {
        let mut only = Idle;
        let mut runners: Vec<&mut dyn Runner> = vec![&mut only];
        let mut asked = 0;
        let mut done = Vec::new();
        let finished = for_each(
            &mut runners,
            &[PathBuf::from("tree")],
            &["a", "b", "c"],
            &|_, _, item: &&str| (*item).to_owned(),
            &mut || {
                asked += 1;
                asked > 2
            },
            &mut |_, item| {
                done.push(item);
                Ok(())
            },
        )
        .unwrap();
        assert!(!finished);
        assert_eq!(done, ["a", "b"]);
        assert_eq!(asked, 3);
    }
}
