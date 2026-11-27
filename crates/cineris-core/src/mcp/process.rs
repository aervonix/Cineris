//! A local server's process once started: its tree, its three pipes, and
//! its end (`docs/steps/30/answered.md`, points 3.3, 3.4 and 3.7).
//!
//! - **Its tree** is started on a thread of its own, which lives as long as
//!   the server does and waits for it ([`Tree::spawn`]'s rule: on Linux the
//!   death signal is tied to the thread that started it). When the server
//!   exits, whatever it left running is killed with its tree, and its own
//!   folder is removed.
//! - **Its standard input** is a pipe a thread of its own writes: first the
//!   policy, when the launcher confines it (the launcher's live input), and
//!   then each line the client queues, with its line end. A line that holds
//!   a line end of its own is not written: it would be two messages.
//! - **Its standard output** is read as lines by `cineris_base::framing`'s
//!   decoder, at most [`MAX_LINE`] bytes a line; a longer one is passed on
//!   as [`LineError::TooLong`], and the output ends there.
//! - **Its standard error** is always read, so that a server that writes
//!   much there never stalls, and only its last [`STDERR_KEPT`] bytes are
//!   kept: shown, through the session's secret filter, to the person
//!   alone.
//! - **Its end** ([`ServerProcess::shutdown`]): its input is closed, it is
//!   given its grace on the session's timer, and then its tree is stopped
//!   in stages, as a command's is: interrupted, told to terminate, killed.
//!   It may be asked for twice, and from two tasks at once (the client
//!   stops a server that misbehaves, and the lifecycle then shuts the
//!   client down): the second waits for the first, and then returns at
//!   once, the server having ended.
//!
//! The client's queues are bounded ([`OUTPUT_QUEUED`] lines of the
//! server's output, at most [`QUEUED_BYTES`], and [`LINES_QUEUED`] of its
//! input): a client that does not read holds the server, and a server that
//! does not read holds the client's sends.

#![allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a server's process is started on a thread of its own, and its pipes are read and \
              written on threads of their own"
)]

use std::io::{PipeWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::task::Poll;
use std::time::Duration;

use cineris_base::{FrameDecoder, SecretFilter, SharedTimer, Sleep};
use cineris_mcp::{LineError, ServerPipes, ServerProcess, SpawnError};
use cineris_sandbox::tree::{Exit, Guard, Output, OutputStop, ReadOutcome, Tree};
use cineris_tools::BoxFuture;
use tokio::sync::{mpsc, watch};

/// The longest line a server may write to its standard output: 4 MiB.
pub const MAX_LINE: usize = 4 * 1024 * 1024;

/// How much of a server's standard error is kept: its last 8 KiB.
pub const STDERR_KEPT: usize = 8 * 1024;

/// How many lines the client's queue of input holds: the client's own.
pub const LINES_QUEUED: usize = 64;

/// The most bytes of a server's output in lines handed to the client and
/// not yet taken: those waiting in its queue and the one being handed over,
/// 16 MiB. Besides them the reader holds the line it is cutting out: its
/// decoder's buffer, at most `2 * MAX_LINE`, a read (64 KiB) and a line's
/// ending, since it drops the lines it has handed over once they are half
/// of what it holds; and a read buffer of 64 KiB. So a server writing lines
/// at the cap holds at most about 24 MiB and 128 KiB of the harness's
/// memory.
pub const QUEUED_BYTES: usize = 16 * 1024 * 1024;

/// How many lines of a server's output wait for the client: as many as
/// [`QUEUED_BYTES`] holds of the longest beside the one being handed over
/// (the full review of step 30's first part: 64 lines were 256 MiB; the
/// second: the one being handed over was not counted). Past it, the
/// reader waits, and the server's writes wait on the pipe.
pub const OUTPUT_QUEUED: usize = QUEUED_BYTES / MAX_LINE - 1;
const _: () = assert!(OUTPUT_QUEUED >= 1 && (OUTPUT_QUEUED + 1) * MAX_LINE <= QUEUED_BYTES);

/// The grace between two stages of stopping a server's tree, after its own
/// grace: a command's ([`cineris_tools::process::DEFAULT_GRACE`]).
pub const STAGE_GRACE: Duration = cineris_tools::process::DEFAULT_GRACE;

/// How long a killed server is waited for before it is given up on: a
/// kill takes milliseconds, so this bounds a failure of the system.
const KILL_WAIT: Duration = Duration::from_secs(5);

/// How long the end of a server's output is waited for once it has ended:
/// at once, unless a process that left its tree holds a pipe.
const SETTLE: Duration = Duration::from_secs(1);

/// What a server is started from: the command, ready but for its standard
/// streams.
pub(crate) struct Prepared {
    /// The command: its program, arguments, environment and folder set.
    pub(crate) command: std::process::Command,
    /// What is written to its input before any line: the launcher's policy.
    pub(crate) preamble: Vec<u8>,
    /// Whether the launcher confines it.
    pub(crate) confined: bool,
    /// The program as its entry names it, for a message.
    pub(crate) shown: String,
    /// The server's own folder, removed when it has ended.
    pub(crate) folder: PathBuf,
}

/// What the server's threads tell its handle.
struct Shared {
    /// The end of its standard error.
    tail: Mutex<Vec<u8>>,
    /// Set when its input is to be closed.
    closing: AtomicBool,
}

/// Starts `prepared` as a tree, with its pipes and threads.
///
/// # Errors
///
/// [`SpawnError::Failed`] when the system could not start it, or a pipe
/// or a thread for it could not be made.
pub(crate) fn start(
    prepared: Prepared,
    guard: Option<Guard>,
    filter: Arc<RwLock<SecretFilter>>,
    timer: SharedTimer,
) -> Result<ServerPipes, SpawnError> {
    let Prepared {
        mut command,
        preamble,
        confined,
        shown,
        folder,
    } = prepared;
    // Removed with whatever returns first: a start that fails here, the
    // waiter's thread when the server has ended, or that thread not made.
    let folder = OwnFolder(folder);
    let setup = |error: std::io::Error| {
        SpawnError::Failed(format!("`{shown}` could not be set up: {error}"))
    };
    let (input_reader, input_writer) = std::io::pipe().map_err(setup)?;
    command.stdin(input_reader);
    let split = Output::attach_split(&mut command).map_err(setup)?;
    let (exit_sender, exit) = watch::channel(None::<String>);
    let (readers_done, readers) = watch::channel(0u8);
    let (started_sender, started) = std::sync::mpsc::channel();

    let waiter = std::thread::Builder::new()
        .name("cineris-mcp-server".to_owned())
        .spawn(move || {
            let tree = match Tree::spawn(command, guard.as_ref()) {
                Ok(tree) => tree,
                Err(error) => {
                    drop(folder);
                    let _ = started_sender.send(Err(error));
                    return;
                }
            };
            let _ = started_sender.send(Ok(tree.clone()));
            let ended = tree.wait();
            // Whatever it left running ends with it, and its folder goes,
            // before its end is told.
            tree.end();
            drop(folder);
            let _ = exit_sender.send(Some(words(&ended, confined)));
        });
    if let Err(error) = waiter {
        return Err(setup(error));
    }
    let tree = match started.recv() {
        Ok(Ok(tree)) => tree,
        Ok(Err(error)) => {
            return Err(SpawnError::Failed(format!(
                "`{shown}` could not be started: {error}"
            )));
        }
        Err(_) => {
            return Err(SpawnError::Failed(format!(
                "`{shown}` could not be started: its thread ended first"
            )));
        }
    };

    let shared = Arc::new(Shared {
        tail: Mutex::new(Vec::new()),
        closing: AtomicBool::new(false),
    });
    let (input, lines_in) = mpsc::channel::<Vec<u8>>(LINES_QUEUED);
    let (lines_out, output) = mpsc::channel(OUTPUT_QUEUED);
    let (stdout, stdout_stop) = split.stdout;
    let (stderr, stderr_stop) = split.stderr;
    let threads = [
        {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("cineris-mcp-input".to_owned())
                .spawn(move || write_input(input_writer, &preamble, lines_in, &shared))
                .map(drop)
        },
        {
            let done = Done(readers_done.clone());
            std::thread::Builder::new()
                .name("cineris-mcp-output".to_owned())
                .spawn(move || read_lines(stdout, &lines_out, done))
                .map(drop)
        },
        {
            let shared = Arc::clone(&shared);
            let done = Done(readers_done);
            std::thread::Builder::new()
                .name("cineris-mcp-error".to_owned())
                .spawn(move || drain(stderr, &shared, done))
                .map(drop)
        },
    ];
    let process = Running {
        weak_input: input.downgrade(),
        tree,
        exit,
        readers,
        stops: [stdout_stop, stderr_stop],
        shared,
        filter,
        timer,
        ending: tokio::sync::Mutex::new(()),
    };
    if let Some(Err(error)) = threads.into_iter().find(Result::is_err) {
        // Dropped, the handle kills what it started.
        drop(process);
        return Err(setup(error));
    }
    Ok(ServerPipes {
        input,
        output,
        process: Box::new(process),
    })
}

/// How a server ended, in words.
fn words(ended: &std::io::Result<Exit>, confined: bool) -> String {
    match ended {
        Ok(Exit::Code(code)) if confined && *code == cineris_sandbox::launch::FAILED => {
            format!("exited with code {code}: the sandbox could not be applied, so it was not run")
        }
        Ok(Exit::Code(code)) => format!("exited with code {code}"),
        Ok(Exit::Signal(signal)) => format!("was ended by signal {signal}"),
        Err(error) => format!("ended, and how could not be seen: {error}"),
    }
}

/// A server's own folder, removed with everything in it, following no link,
/// when this is dropped.
struct OwnFolder(PathBuf);

impl Drop for OwnFolder {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(folder = %self.0.display(), %error, "an MCP server's folder was not removed");
        }
    }
}

/// The input's thread: the preamble, then each line the client queues,
/// until the client lets go of its queue, the server stops reading, or the
/// input is to be closed. Dropping the pipe closes the server's input.
fn write_input(
    mut pipe: PipeWriter,
    preamble: &[u8],
    mut lines: mpsc::Receiver<Vec<u8>>,
    shared: &Shared,
) {
    // A launcher that does not read its policy has failed, and says so.
    if !preamble.is_empty() && pipe.write_all(preamble).is_err() {
        return;
    }
    while let Some(mut line) = lines.blocking_recv() {
        if shared.closing.load(Ordering::Acquire) {
            break;
        }
        if line.contains(&b'\n') {
            tracing::warn!("a line for an MCP server held a line end, and was not written");
            continue;
        }
        line.push(b'\n');
        if pipe.write_all(&line).is_err() {
            break;
        }
    }
}

/// Counts a reader's end when dropped, however the reader ended.
struct Done(watch::Sender<u8>);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.send_modify(|ended| *ended += 1);
    }
}

/// The output's thread: lines to the client, until the pipe closes, the
/// client lets go of its queue, or a line is longer than [`MAX_LINE`].
fn read_lines(mut output: Output, lines: &mpsc::Sender<Result<Vec<u8>, LineError>>, _done: Done) {
    let mut decoder = FrameDecoder::with_max_line_len(MAX_LINE);
    let mut buffer = vec![0u8; 64 * 1024];
    while let Ok(ReadOutcome::Bytes(count)) = output.read(&mut buffer) {
        decoder.push(&buffer[..count]);
        while let Some(line) = decoder.next_line() {
            let sent = match line {
                Ok(line) => lines.blocking_send(Ok(line.to_vec())),
                Err(_) => {
                    // The output ends after a line too long to take.
                    let _ = lines.blocking_send(Err(LineError::TooLong(MAX_LINE)));
                    return;
                }
            };
            if sent.is_err() {
                return;
            }
        }
    }
}

/// The standard error's thread: read to its end, its last
/// [`STDERR_KEPT`] bytes kept.
fn drain(mut output: Output, shared: &Shared, _done: Done) {
    let mut buffer = vec![0u8; 8 * 1024];
    while let Ok(ReadOutcome::Bytes(count)) = output.read(&mut buffer) {
        let mut kept = shared.tail.lock().unwrap_or_else(PoisonError::into_inner);
        kept.extend_from_slice(&buffer[..count]);
        let over = kept.len().saturating_sub(STDERR_KEPT);
        kept.drain(..over);
    }
}

/// A started server's handle. Dropping it kills what is left of its tree.
struct Running {
    tree: Tree,
    /// How it ended, once it has.
    exit: watch::Receiver<Option<String>>,
    /// How many of its two readers have ended.
    readers: watch::Receiver<u8>,
    stops: [OutputStop; 2],
    /// The client's queue of input, held weakly: the input closes when the
    /// client lets go of its end, or when it is closed here.
    weak_input: mpsc::WeakSender<Vec<u8>>,
    shared: Arc<Shared>,
    filter: Arc<RwLock<SecretFilter>>,
    timer: SharedTimer,
    /// Held by a call of `shutdown` from its start to its end: a second
    /// call, from the client that stopped a misbehaving server and then
    /// from the lifecycle, or from two tasks at once, waits for the first,
    /// and then finds the server ended and returns.
    ending: tokio::sync::Mutex<()>,
}

impl Running {
    /// Closes the server's input: the input's thread lets the pipe go at
    /// the next line, and it is woken with one that is not written.
    fn close_input(&self) {
        self.shared.closing.store(true, Ordering::Release);
        if let Some(input) = self.weak_input.upgrade() {
            let _ = input.try_send(Vec::new());
        }
    }

    /// Whether the server ended within `limit`.
    async fn ended_within(&self, limit: Duration) -> bool {
        let mut exit = self.exit.clone();
        let ended = async move {
            let _ = exit.wait_for(Option::is_some).await;
        };
        first(ended, self.timer.sleep(limit)).await
    }

    /// Whether both readers ended within `limit`.
    async fn read_within(&self, limit: Duration) -> bool {
        let mut readers = self.readers.clone();
        let ended = async move {
            let _ = readers.wait_for(|ended| *ended >= 2).await;
        };
        first(ended, self.timer.sleep(limit)).await
    }
}

/// Whether `done` completes before `over`.
async fn first(done: impl Future<Output = ()>, mut over: Sleep) -> bool {
    let mut done = std::pin::pin!(done);
    std::future::poll_fn(|context| {
        if done.as_mut().poll(context).is_ready() {
            return Poll::Ready(true);
        }
        if over.as_mut().poll(context).is_ready() {
            return Poll::Ready(false);
        }
        Poll::Pending
    })
    .await
}

impl ServerProcess for Running {
    fn stderr_tail(&self) -> String {
        let kept = self
            .shared
            .tail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let text = String::from_utf8_lossy(&kept);
        let masked = self
            .filter
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .redact(&text)
            .into_owned();
        cineris_base::visible_keeping(&masked, &['\n', '\t']).into_owned()
    }

    fn exited(&self) -> BoxFuture<'_, String> {
        let mut exit = self.exit.clone();
        Box::pin(async move {
            match exit.wait_for(Option::is_some).await {
                Ok(said) => said.clone().unwrap_or_default(),
                Err(_) => "ended, and how could not be seen".to_owned(),
            }
        })
    }

    fn shutdown(&self, grace: Duration) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            // One end at a time; one that comes after it finds the server
            // ended, and its output read, and returns at once.
            let _one = self.ending.lock().await;
            self.close_input();
            if !self.ended_within(grace).await {
                // As a command's tree is stopped: a stage that cannot be
                // sent is passed over without its grace.
                let mut ended = false;
                let stages: [fn(&Tree) -> std::io::Result<()>; 2] =
                    [Tree::interrupt, Tree::terminate];
                for stage in stages {
                    if stage(&self.tree).is_ok() && self.ended_within(STAGE_GRACE).await {
                        ended = true;
                        break;
                    }
                }
                if !ended {
                    let _ = self.tree.kill();
                    let _ = self.ended_within(KILL_WAIT).await;
                }
            }
            // What it wrote before it ended is read to the end, unless a
            // process that left its tree holds a pipe.
            if !self.read_within(SETTLE).await {
                for stop in &self.stops {
                    stop.stop();
                }
            }
        })
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shared.closing.store(true, Ordering::Release);
        if self.exit.borrow().is_none() {
            let _ = self.tree.kill();
        }
        for stop in &self.stops {
            stop.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server's folder goes when its owner is dropped, whatever ended the
    /// start: a pipe or a thread that could not be made returns early and
    /// drops it (the full review of step 30's first part).
    #[test]
    fn a_servers_folder_goes_with_its_owner() {
        let scratch = cineris_testkit::Scratch::outside("cineris-core-mcp-process", "folder");
        let folder = scratch.dir("own");
        std::fs::write(folder.join("tmp-file"), "x").unwrap();
        drop(OwnFolder(folder.clone()));
        assert!(!folder.exists());
        // One already gone is no failure.
        drop(OwnFolder(folder));
    }

    #[test]
    fn an_end_is_said_in_words() {
        assert_eq!(words(&Ok(Exit::Code(3)), false), "exited with code 3");
        assert_eq!(words(&Ok(Exit::Signal(9)), true), "was ended by signal 9");
        let failed = words(&Ok(Exit::Code(cineris_sandbox::launch::FAILED)), true);
        assert!(failed.contains("sandbox could not be applied"), "{failed}");
        assert_eq!(
            words(&Ok(Exit::Code(cineris_sandbox::launch::FAILED)), false),
            "exited with code 125"
        );
    }
}
