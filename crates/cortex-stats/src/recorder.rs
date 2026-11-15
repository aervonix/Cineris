//! Recording: events leave the loop through a bounded queue to a writer
//! thread of their own (`docs/architecture/14-adaptation.md`, "Cost,
//! against the speed budgets").
//!
//! **Recording never waits.** [`Recorder::record`] is one push onto a
//! bounded queue. When the queue is full, because the writer is behind or
//! stalled on a slow disk, the event is dropped and counted, and the count
//! is written as an event of its own when there is room again. A record
//! that says it lost events is better than a loop that stalls.
//!
//! **Nothing is done at start-up.** Making a recorder makes the queue and
//! nothing else: no thread, no directory, no file. The thread is started
//! by [`Recorder::start`], which its owner calls off the start-up path
//! (the core does it on the blocking pool when the first session opens),
//! and events recorded before that wait in the queue. The writer reads the
//! local time zone the first time it writes, and the aggregates only when
//! it brings them up to date.
//!
//! **What the thread does.** It gives each event the local day it fell on,
//! appends it to its month's file, and every few seconds (and when asked to
//! flush) brings the aggregates up to date from the files and writes the
//! latest limits.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the writer thread exists to make blocking file calls"
)]

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cortex_base::{SharedClock, UtcTime, Zone};
use cortex_protocol::usage::ProviderLimits;

use crate::record::{Line, Record};
use crate::store::{Aggregates, Appender, LimitsFile, StatsDir, month_of};

/// How many events may wait for the writer.
pub const QUEUE: usize = 4096;

/// How the recorder is set.
#[derive(Debug, Clone)]
pub struct RecorderOptions {
    /// Where the files go.
    pub dir: StatsDir,
    /// The zone the local day is counted in. `None`: the system's, read by
    /// the writer the first time it needs it.
    pub zone: Option<Zone>,
    /// How many months a monthly file stays after its own.
    pub retention: u32,
    /// How many events may wait.
    pub queue: usize,
    /// How often the writer brings the aggregates up to date while events
    /// arrive.
    pub sync_every: Duration,
}

impl RecorderOptions {
    /// The defaults, in `dir`.
    #[must_use]
    pub fn new(dir: StatsDir) -> Self {
        Self {
            dir,
            zone: None,
            retention: 12,
            queue: QUEUE,
            sync_every: Duration::from_secs(5),
        }
    }
}

/// What the queue carries. The events are boxed, so that the queue's slots,
/// which a bounded channel holds from the start, are small.
enum Message {
    Record(Box<Record>),
    Limits(Box<ProviderLimits>),
    /// Write what is queued, bring the aggregates up to date, and answer.
    Flush(SyncSender<()>),
}

struct Writer {
    receiver: Receiver<Message>,
    options: RecorderOptions,
    clock: SharedClock,
}

/// Takes statistics events and writes them off the calling thread. Clones
/// write through the same queue.
#[derive(Clone)]
pub struct Recorder {
    queue: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
    /// The writer, until it is started.
    waiting: Arc<Mutex<Option<Writer>>>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("dropped", &self.dropped())
            .finish_non_exhaustive()
    }
}

impl Recorder {
    /// A recorder: its queue, and a writer that is not started yet.
    #[must_use]
    pub fn new(options: RecorderOptions, clock: SharedClock) -> Self {
        let (queue, receiver) = sync_channel(options.queue.max(1));
        Self {
            queue,
            dropped: Arc::new(AtomicU64::new(0)),
            waiting: Arc::new(Mutex::new(Some(Writer {
                receiver,
                options,
                clock,
            }))),
        }
    }

    /// Starts the writer thread, once. Calling it again does nothing.
    ///
    /// # Errors
    ///
    /// When the thread cannot be started. The events wait in the queue, or
    /// are dropped and counted once it is full.
    pub fn start(&self) -> io::Result<()> {
        let Some(writer) = self
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return Ok(());
        };
        let dropped = Arc::clone(&self.dropped);
        std::thread::Builder::new()
            .name("cortex-stats".to_owned())
            .spawn(move || run(&writer, &dropped))
            .map(drop)
    }

    /// Takes one event. Never waits: with the queue full it is dropped and
    /// counted.
    pub fn record(&self, record: Record) {
        self.send(Message::Record(Box::new(record)));
    }

    /// Takes the limits a provider reported, which the writer keeps as the
    /// latest for that provider. Never waits.
    pub fn limits(&self, limits: ProviderLimits) {
        self.send(Message::Limits(Box::new(limits)));
    }

    fn send(&self, message: Message) {
        if let Err(TrySendError::Full(_)) = self.queue.try_send(message) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// How many events were dropped and not yet written as a count.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Waits until everything taken so far is written and the aggregates
    /// are up to date. For a command's end and for tests: never on the core
    /// thread while a session runs. Returns at once when the writer was
    /// never started.
    pub fn flush(&self) {
        let started = self
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none();
        if !started {
            return;
        }
        let (done, wait) = sync_channel(1);
        if self.queue.send(Message::Flush(done)).is_ok() {
            let _ = wait.recv();
        }
    }
}

/// The writer's state between messages.
struct State {
    zone: Option<Zone>,
    appender: Appender,
    limits: BTreeMap<(String, String), ProviderLimits>,
    /// Lines appended since the aggregates were last brought up to date.
    unsynced: bool,
    /// When they were, on the monotonic clock.
    settled_at: Duration,
}

fn run(writer: &Writer, dropped: &AtomicU64) {
    let mut state = State {
        zone: writer.options.zone.clone(),
        appender: Appender::default(),
        limits: BTreeMap::new(),
        unsynced: false,
        settled_at: writer.clock.monotonic(),
    };
    loop {
        let message = if state.unsynced || !state.limits.is_empty() {
            match writer.receiver.recv_timeout(writer.options.sync_every) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match writer.receiver.recv() {
                Ok(message) => Some(message),
                Err(_) => break,
            }
        };
        match message {
            Some(Message::Record(record)) => {
                write_dropped(writer, &mut state, dropped);
                write_record(writer, &mut state, &record);
                // Events that never stop arriving still reach the
                // aggregates every so often.
                let since = writer.clock.monotonic().saturating_sub(state.settled_at);
                if since >= writer.options.sync_every {
                    settle(writer, &mut state);
                }
            }
            Some(Message::Limits(limits)) => {
                let key = (
                    limits.provider.as_str().to_owned(),
                    limits.source.as_str().to_owned(),
                );
                state.limits.insert(key, *limits);
            }
            Some(Message::Flush(done)) => {
                write_dropped(writer, &mut state, dropped);
                settle(writer, &mut state);
                let _ = done.send(());
            }
            None => settle(writer, &mut state),
        }
    }
    write_dropped(writer, &mut state, dropped);
    settle(writer, &mut state);
}

/// Writes the count of dropped events, when there is one.
fn write_dropped(writer: &Writer, state: &mut State, dropped: &AtomicU64) {
    let count = dropped.swap(0, Ordering::Relaxed);
    if count > 0 {
        let at = writer.clock.now().to_timestamp();
        write_record(writer, state, &Record::Dropped { at, count });
    }
}

fn write_record(writer: &Writer, state: &mut State, record: &Record) {
    let at = UtcTime::parse(record.at().as_str()).unwrap_or_else(|_| writer.clock.now());
    let zone = state.zone.get_or_insert_with(Zone::system);
    let line = Line {
        day: zone.date_of(at).to_string(),
        record: record.clone(),
    };
    let Ok(mut bytes) = serde_json::to_vec(&line) else {
        return;
    };
    bytes.push(b'\n');
    // A failed write loses that event: there is nobody to tell but the log.
    if writer
        .options
        .dir
        .ensure()
        .and_then(|()| {
            state
                .appender
                .append(&writer.options.dir, &month_of(at), &bytes)
        })
        .is_ok()
    {
        state.unsynced = true;
    } else {
        state.appender.close();
    }
}

/// Brings the aggregates up to date and writes the limits.
fn settle(writer: &Writer, state: &mut State) {
    state.settled_at = writer.clock.monotonic();
    let dir = &writer.options.dir;
    if state.unsynced {
        state.unsynced = false;
        let now = writer.clock.now();
        let zone = state.zone.get_or_insert_with(Zone::system);
        if let Ok(mut aggregates) = Aggregates::load(dir)
            && let Ok(true) = aggregates.sync(
                dir,
                zone.date_of(now),
                &month_of(now),
                writer.options.retention,
            )
        {
            let _ = aggregates.save(dir, writer.clock.as_ref());
        }
    }
    if !state.limits.is_empty() {
        let limits = std::mem::take(&mut state.limits);
        let _ = LimitsFile::update(dir, limits.into_values(), writer.clock.as_ref());
    }
}

#[cfg(test)]
mod tests;
