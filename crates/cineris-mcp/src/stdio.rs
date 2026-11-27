//! The standard I/O transport: one line a message each way, over the pipes
//! the harness's [`Spawner`](crate::Spawner) gives (`docs/steps/30/answered.md`,
//! points 1 and 3.7).
//!
//! A task reads the server's output for as long as the connection lasts.
//! An answer to a pending request goes to it; everything else is counted
//! against the server's [`Budget`] and handed to the session, which may
//! answer it. The connection ends when the output ends, when the process
//! exits, when a line is longer than the pipes' cap, or when the server
//! spends its budget; every request waiting then fails with why.

use std::future::poll_fn;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use cineris_base::{SharedTimer, Sleep};
use cineris_tools::{BoxFuture, Json};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::client::{ClientError, Era};
use crate::jsonrpc::{self, Answer, Decoded, Message};
use crate::process::{LineError, ServerPipes, ServerProcess};
use crate::transport::{
    BUDGET_WINDOW, Budget, Ending, Pending, SharedInbound, Transport, Waiting, lock,
};

/// The one revision whose servers may send a batch.
const BATCH_REVISION: &str = "2025-03-26";

/// One server's standard input and output.
pub(crate) struct StdioTransport {
    /// Lines for its input. Taken at shutdown, which closes the input.
    input: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    pending: Pending,
    ending: Ending,
    process: Arc<dyn ServerProcess>,
    /// Whether a line may hold an array of messages.
    batches: AtomicBool,
    reader: Mutex<Option<AbortHandle>>,
}

impl std::fmt::Debug for StdioTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdioTransport")
            .field("pending", &self.pending.len())
            .field("ended", &self.ending.ended())
            .finish_non_exhaustive()
    }
}

impl StdioTransport {
    /// Starts reading `pipes`. What is not an answer goes to `inbound`; the
    /// budget's window is kept on `timer`; `grace` is how long a server
    /// ended for misbehaving has before the staged kill. Must be called
    /// inside a Tokio runtime.
    pub(crate) fn start(
        pipes: ServerPipes,
        inbound: SharedInbound,
        timer: SharedTimer,
        grace: Duration,
    ) -> Arc<Self> {
        let ServerPipes {
            input,
            output,
            process,
        } = pipes;
        let transport = Arc::new(Self {
            input: Mutex::new(Some(input)),
            pending: Pending::default(),
            ending: Ending::default(),
            process: Arc::from(process),
            batches: AtomicBool::new(false),
            reader: Mutex::new(None),
        });
        let reader = tokio::spawn(read(Arc::clone(&transport), output, inbound, timer, grace));
        *lock(&transport.reader) = Some(reader.abort_handle());
        transport
    }

    /// The server's process.
    pub(crate) fn process(&self) -> &Arc<dyn ServerProcess> {
        &self.process
    }

    /// Ends the connection with `why`: every request waiting fails.
    fn end(&self, why: ClientError) {
        self.pending.end(why.clone());
        self.ending.end(why);
    }

    /// Stops reading, without waiting for anything: for a client dropped.
    pub(crate) fn abandon(&self) {
        self.end(ClientError::Stopped("the client was dropped".to_owned()));
        lock(&self.input).take();
        if let Some(reader) = lock(&self.reader).take() {
            reader.abort();
        }
    }

    /// Takes one line of output. An error is why the connection ends.
    fn take(
        &self,
        line: &[u8],
        budget: &mut Budget,
        inbound: &SharedInbound,
    ) -> Result<(), ClientError> {
        let misbehaving = |words: String| ClientError::Protocol(words);
        match jsonrpc::decode(line) {
            Decoded::One(message) => self.route(message, line.len(), budget, inbound),
            Decoded::Batch(messages) if self.batches.load(Ordering::Acquire) => {
                // The line is counted once, with its bytes, whatever it
                // holds (an empty batch included); then each message in it
                // that answers nothing.
                budget.spend(line.len()).map_err(misbehaving)?;
                for message in messages {
                    match message {
                        Some(message) => self.route(message, 0, budget, inbound)?,
                        None => budget.spend(0).map_err(misbehaving)?,
                    }
                }
                Ok(())
            }
            // A batch from a server that may not send one is not read, and
            // neither is what is not a message: counted, and left.
            Decoded::Batch(_) | Decoded::Malformed => budget.spend(line.len()).map_err(misbehaving),
        }
    }

    /// Gives an answer to its request, or counts the message and hands it
    /// to the session.
    fn route(
        &self,
        message: Message,
        bytes: usize,
        budget: &mut Budget,
        inbound: &SharedInbound,
    ) -> Result<(), ClientError> {
        let message = match message {
            Message::Response { id, answer } => match id.ours() {
                Some(ours) if self.pending.answer(ours, answer.clone()) => return Ok(()),
                _ => Message::Response { id, answer },
            },
            other => other,
        };
        budget.spend(bytes).map_err(ClientError::Protocol)?;
        if let Some(reply) = inbound.handle(message) {
            self.send(reply);
        }
        Ok(())
    }
}

impl Transport for StdioTransport {
    fn request(&self, id: u64, message: Json) -> BoxFuture<'_, Result<Answer, ClientError>> {
        Box::pin(async move {
            let mut waiting = Waiting::new(&self.pending, id)?;
            let input = lock(&self.input).clone();
            let ended = || {
                self.pending
                    .ended()
                    .unwrap_or_else(|| ClientError::Stopped("its input closed".to_owned()))
            };
            let Some(input) = input else {
                return Err(ended());
            };
            let sent = input.send(encode(&message)).await;
            // Not kept while the answer is awaited: the input closes at
            // shutdown when the client's own sender goes.
            drop(input);
            if sent.is_err() {
                return Err(ended());
            }
            waiting.answer().await.map_err(|lost| match lost {
                // Written, and the connection ended before its answer: the
                // server may have acted on it.
                ClientError::Stopped(words) => ClientError::MayHaveRun(words),
                other => other,
            })
        })
    }

    fn send(&self, message: Json) {
        if let Some(input) = lock(&self.input).as_ref() {
            // A server with no room for it is not reading: it is dropped,
            // and nothing waits on that server.
            let _dropped = input.try_send(encode(&message));
        }
    }

    fn cancel(&self, id: u64, reason: &str) {
        self.send(jsonrpc::notification(
            "notifications/cancelled",
            Some(json!({"requestId": id, "reason": reason})),
        ));
    }

    fn agreed(&self, era: Era, version: &str) {
        self.batches.store(
            era == Era::Legacy && version == BATCH_REVISION,
            Ordering::Release,
        );
    }

    fn closed(&self) -> BoxFuture<'_, ClientError> {
        Box::pin(self.ending.wait())
    }

    fn shutdown(&self, grace: Duration) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.end(ClientError::Stopped("the client shut it down".to_owned()));
            // Its input closes when the last sender is gone.
            lock(&self.input).take();
            self.process.shutdown(grace).await;
            if let Some(reader) = lock(&self.reader).take() {
                reader.abort();
            }
        })
    }
}

/// One message as a line, without its line end. JSON written compactly
/// holds no line break: one in a string is escaped.
fn encode(message: &Json) -> Vec<u8> {
    message.to_string().into_bytes()
}

/// What woke the reader.
enum Event {
    Line(Option<Result<Vec<u8>, LineError>>),
    Exited(String),
    WindowEnded,
}

/// Reads the server's output until the connection ends.
async fn read(
    transport: Arc<StdioTransport>,
    mut output: mpsc::Receiver<Result<Vec<u8>, LineError>>,
    inbound: SharedInbound,
    timer: SharedTimer,
    grace: Duration,
) {
    let process = Arc::clone(&transport.process);
    let mut exited = process.exited();
    let mut window: Option<Sleep> = None;
    let mut budget = Budget::default();
    let why = loop {
        let event = poll_fn(|context| {
            // A window that has ended is seen before the lines after it.
            if let Some(sleep) = window.as_mut()
                && sleep.as_mut().poll(context).is_ready()
            {
                return Poll::Ready(Event::WindowEnded);
            }
            if let Poll::Ready(line) = output.poll_recv(context) {
                return Poll::Ready(Event::Line(line));
            }
            if let Poll::Ready(words) = exited.as_mut().poll(context) {
                return Poll::Ready(Event::Exited(words));
            }
            Poll::Pending
        })
        .await;
        match event {
            Event::Line(Some(Ok(line))) => {
                if let Err(why) = transport.take(&line, &mut budget, &inbound) {
                    break why;
                }
                if window.is_none() && budget.is_open() {
                    window = Some(timer.sleep(BUDGET_WINDOW));
                }
            }
            Event::Line(Some(Err(LineError::TooLong(cap)))) => {
                break ClientError::Protocol(format!("it wrote a line longer than {cap} bytes"));
            }
            Event::Line(None) => {
                // Its output closed: the process is exiting, and says how
                // when it has.
                let words = race(exited.as_mut(), timer.sleep(grace))
                    .await
                    .unwrap_or_else(|| "its output closed".to_owned());
                break ClientError::Stopped(words);
            }
            Event::Exited(words) => {
                // What it wrote before it exited is read first, as the
                // pipe's reader hands it on: until its output ends, or for
                // the grace at most (a process it left may hold the pipe).
                let mut limit = timer.sleep(grace);
                while let Some(Ok(line)) = poll_fn(|context| {
                    if let Poll::Ready(line) = output.poll_recv(context) {
                        return Poll::Ready(line);
                    }
                    limit.as_mut().poll(context).map(|()| None)
                })
                .await
                {
                    if transport.take(&line, &mut budget, &inbound).is_err() {
                        break;
                    }
                }
                break ClientError::Stopped(words);
            }
            Event::WindowEnded => {
                budget.reset();
                window = None;
            }
        }
    };
    let misbehaved = matches!(why, ClientError::Protocol(_));
    transport.end(why);
    if misbehaved {
        // Ended as misbehaving: its input closed, then its process ended.
        lock(&transport.input).take();
        drop(output);
        transport.process.shutdown(grace).await;
    }
}

/// The first of `work` and `limit` to finish: `None` when the limit did.
async fn race<F: Future + ?Sized>(mut work: Pin<&mut F>, mut limit: Sleep) -> Option<F::Output> {
    poll_fn(|context| {
        if let Poll::Ready(output) = work.as_mut().poll(context) {
            return Poll::Ready(Some(output));
        }
        limit.as_mut().poll(context).map(|()| None)
    })
    .await
}
