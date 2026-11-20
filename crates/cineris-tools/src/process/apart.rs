//! A command run with its standard output and its standard error apart
//! ([`Runner::run_apart`]), for a caller that reads what a program answers
//! apart from what it says about its progress: an extension's
//! `cineris.exec` (`docs/architecture/12-extensions.md`), where `gh … --json`
//! prints its JSON on the one and its progress on the other.
//!
//! The command runs as [`Runner::run_until`] runs one, through the same
//! loop: the same tree, the same staged stop on the time limit and on
//! cancellation, the same grace for what it left running, and the same kill
//! when the future is dropped. Only the output differs.
//!
//! - **Two pipes, two readers.** Each stream has a pipe and a thread of its
//!   own ([`Output::attach_split`]), so a command that fills one while the
//!   other is quiet is never left waiting. The run's output has ended when
//!   both have.
//! - **Kept in memory, up to a limit.** The first bytes of each stream are
//!   kept, at most the limit the caller gives, and the bytes past it are
//!   counted ([`KeptStream`]). There is no artifact and no live view, and
//!   the bytes are not decoded: that is the caller's.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use cineris_base::CancelToken;
use cineris_sandbox::tree::{Output, ReadOutcome, SplitOutput};

use super::{
    Ending, Exit, Runner, Running, Shared, Start, StartError, Stop, Stops, build_command, waiter,
};
use crate::contract::NoOutput;

/// What is kept of one stream of a command run with its streams apart: its
/// first bytes, up to the run's limit, and how many it carried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeptStream {
    /// The stream's first bytes: all of them, unless it was cut. They are
    /// as the command wrote them, and not always text.
    pub bytes: Vec<u8>,
    /// How many bytes the command wrote to the stream, those past the
    /// limit included.
    pub total_bytes: u64,
}

impl KeptStream {
    /// Whether the stream carried more than was kept.
    #[must_use]
    pub fn is_cut(&self) -> bool {
        self.total_bytes > self.bytes.len() as u64
    }

    /// Takes the next bytes of the stream: all of them are counted, and as
    /// many as fit under `limit` are kept.
    fn push(&mut self, bytes: &[u8], limit: usize) {
        let room = limit.saturating_sub(self.bytes.len()).min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..room]);
        self.total_bytes += bytes.len() as u64;
    }
}

/// A command run with its streams apart that has ended, with nothing of
/// its tree left: what [`Finished`](super::Finished) says of a command,
/// with the two streams in place of its one output.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedApart {
    /// How the command itself ended. `None` when the system would not say.
    pub exit: Option<Exit>,
    /// Why the run ended.
    pub ending: Ending,
    /// The last stage of stopping that was needed, when the command was
    /// stopped.
    pub stopped_with: Option<Stop>,
    /// Whether the command exited and left processes running, which were
    /// then killed.
    pub left_running: bool,
    /// Whether something the command started was still holding one of its
    /// streams after the tree was killed: a process that left the tree,
    /// which the harness cannot stop. That stream may then be missing its
    /// end.
    pub outlived: bool,
    /// Whether a process of the command's session outran the kill of its
    /// tree ([`Finished::escaped`](super::Finished::escaped)).
    pub escaped: bool,
    /// How long the run took, from the start of the command to the end of
    /// its tree.
    pub duration: Duration,
    /// What it wrote to its standard output.
    pub stdout: KeptStream,
    /// What it wrote to its standard error.
    pub stderr: KeptStream,
}

/// What the two readers fill: each stream's kept bytes, and how many of
/// the readers have not ended.
struct Streams {
    stdout: Mutex<KeptStream>,
    stderr: Mutex<KeptStream>,
    readers_left: AtomicUsize,
}

impl Runner {
    /// Runs one command to the end of its tree, as
    /// [`run_until`](Self::run_until) does, with its standard output and
    /// its standard error read apart.
    ///
    /// - The first `limit` bytes of each stream are kept in memory, and the
    ///   bytes past them are counted: [`KeptStream::is_cut`] says whether a
    ///   stream was cut. Nothing goes to a content store, and nothing to a
    ///   front end while the command runs.
    /// - When `start.timeout` passes or `cancel` fires, the tree is stopped
    ///   in stages; when `kill_now` fires, it is killed at once.
    /// - When the command exits, what it left running has the runner's
    ///   linger to finish, and is then killed: whichever stream it held.
    /// - If the future is dropped, the tree is killed.
    ///
    /// `start.label` is not used: there is no artifact to name. It must be
    /// polled on a thread that outlives the command, as `run_until` must.
    pub async fn run_apart(
        &self,
        start: Start,
        cancel: &CancelToken,
        kill_now: &CancelToken,
        limit: usize,
    ) -> Result<FinishedApart, StartError> {
        let started = self.clock.monotonic();
        let shared = Arc::new(Shared::default());
        let streams = Arc::new(Streams {
            stdout: Mutex::default(),
            stderr: Mutex::default(),
            readers_left: AtomicUsize::new(2),
        });
        let running = self.start_apart(&start, limit, &shared, &streams)?;
        let ended = self
            .drive(&start, &running, &shared, cancel, kill_now, &mut NoOutput)
            .await;
        Ok(FinishedApart {
            exit: ended.exit,
            ending: ended.ending,
            stopped_with: ended.stopped_with,
            left_running: ended.left_running,
            outlived: ended.outlived,
            escaped: ended.escaped,
            duration: self.clock.monotonic().saturating_sub(started),
            // A reader that is still blocked, on a stream that a process
            // which left the tree holds, may add to its own copy after
            // this: what it had read is here.
            stdout: std::mem::take(&mut *lock(&streams.stdout)),
            stderr: std::mem::take(&mut *lock(&streams.stderr)),
        })
    }

    /// Starts the command, a reader for each of its streams, and its
    /// waiter.
    fn start_apart(
        &self,
        start: &Start,
        limit: usize,
        shared: &Arc<Shared>,
        streams: &Arc<Streams>,
    ) -> Result<Running, StartError> {
        let (mut command, input) = build_command(start)?;
        let SplitOutput {
            stdout: (stdout, stdout_stop),
            stderr: (stderr, stderr_stop),
        } = Output::attach_split(&mut command).map_err(StartError::Setup)?;
        let running = self.spawn(command, start, Stops(vec![stdout_stop, stderr_stop]))?;

        let out_reader = reader(
            "cineris-command-stdout",
            stdout,
            |streams| &streams.stdout,
            limit,
            shared,
            streams,
        );
        let err_reader = reader(
            "cineris-command-stderr",
            stderr,
            |streams| &streams.stderr,
            limit,
            shared,
            streams,
        );
        let waiter = waiter(&running, input, shared);
        match (out_reader, err_reader, waiter) {
            (Ok(_), Ok(_), Ok(_)) => Ok(running),
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                Err(StartError::Setup(error))
            }
        }
    }
}

/// Starts the thread that reads one stream into the place `kept` picks in
/// `streams`, until the pipe closes or the read is stopped.
fn reader(
    name: &str,
    mut output: Output,
    kept: fn(&Streams) -> &Mutex<KeptStream>,
    limit: usize,
    shared: &Arc<Shared>,
    streams: &Arc<Streams>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    let shared = Arc::clone(shared);
    let streams = Arc::clone(streams);
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let _done = StreamDone {
                shared: &shared,
                streams: &streams,
            };
            let mut buffer = vec![0u8; 16 * 1024];
            // A read that fails ends the reading like a closed pipe. What
            // is read is kept as it arrives, so that a reader that is let
            // go before its stream ends has handed over all it read.
            while let Ok(ReadOutcome::Bytes(count)) = output.read(&mut buffer) {
                lock(kept(&streams)).push(&buffer[..count], limit);
            }
        })
}

/// Says that one reader has ended, however it ended. The second to end
/// says that the output has: a run waits for both streams.
struct StreamDone<'a> {
    shared: &'a Shared,
    streams: &'a Streams,
}

impl Drop for StreamDone<'_> {
    fn drop(&mut self) {
        if self.streams.readers_left.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.shared.update(|state| state.reader_done = true);
        }
    }
}

fn lock(kept: &Mutex<KeptStream>) -> MutexGuard<'_, KeptStream> {
    kept.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::KeptStream;

    #[test]
    fn a_stream_keeps_its_first_bytes_up_to_the_limit_and_counts_the_rest() {
        let mut kept = KeptStream::default();
        kept.push(b"abc", 5);
        assert_eq!((kept.bytes.as_slice(), kept.total_bytes), (&b"abc"[..], 3));
        assert!(!kept.is_cut());
        // A piece that crosses the limit is kept up to it.
        kept.push(b"defg", 5);
        assert_eq!(
            (kept.bytes.as_slice(), kept.total_bytes),
            (&b"abcde"[..], 7)
        );
        assert!(kept.is_cut());
        // After it, nothing more is kept, and everything is counted.
        kept.push(b"hij", 5);
        assert_eq!(
            (kept.bytes.as_slice(), kept.total_bytes),
            (&b"abcde"[..], 10)
        );

        // Exactly the limit is not a cut.
        let mut exact = KeptStream::default();
        exact.push(b"12345", 5);
        assert!(!exact.is_cut());
        // A limit of nothing keeps nothing, and still counts.
        let mut none = KeptStream::default();
        none.push(b"xyz", 0);
        assert_eq!((none.bytes.len(), none.total_bytes), (0, 3));
        assert!(none.is_cut());
    }
}
