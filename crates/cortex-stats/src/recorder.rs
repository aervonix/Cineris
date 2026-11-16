//! Recording: events leave the loop through a bounded queue to a writer
//! thread of their own (`docs/architecture/14-adaptation.md`, "Cost,
//! against the speed budgets").
//!
//! **Recording never waits.** [`Recorder::record`] is one push onto a
//! bounded queue. When the queue is full, because the writer is behind or
//! stalled on a slow disk, the event is dropped and counted, and the count
//! is written as an event of its own when there is room again. A record
//! that says it lost events is better than a loop that stalls. A writer
//! whose thread has ended (it panicked) takes nothing more: every event
//! after it is dropped and counted too, and the log says once that it
//! stopped.
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
//!
//! **A flush can be given up** ([`Recorder::flush_until`]): the end of a
//! process waits for the writer only so long, since a writer stuck on its
//! disk would otherwise hold the process's exit for as long as it is stuck
//! (found by the independent review of step 16).

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the writer thread exists to make blocking file calls"
)]

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use cortex_base::redact::is_plain_code;
use cortex_base::{IdGenerator, SharedClock, SystemRng, UtcTime, Zone};
use cortex_protocol::error::code;
use cortex_protocol::usage::ProviderLimits;

use crate::record::{Line, Record};
use crate::store::{Aggregates, Appender, Found, LimitsFile, StatsDir, month_of};

/// How many events may wait for the writer.
pub const QUEUE: usize = 4096;

/// How many months a monthly file stays after its own, unless configured
/// otherwise (`stats.retention`): three. The coordinator's decision in step
/// 16, which the owner may revisit: a year of a heavy user's raw events is
/// about 1.1 GB, and every all-time figure is kept in the aggregates.
pub const DEFAULT_RETENTION: u32 = 3;

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
            retention: DEFAULT_RETENTION,
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
    Flush(Answer),
}

struct Writer {
    receiver: Receiver<Message>,
    options: RecorderOptions,
    /// How many months a monthly file stays after its own, as it is now:
    /// read each time the files are pruned, so that a reload of the
    /// configuration changes it (step 24).
    retention: Arc<std::sync::atomic::AtomicU32>,
    clock: SharedClock,
    /// Events taken and not yet written.
    queued: Arc<AtomicU64>,
}

/// Takes statistics events and writes them off the calling thread. Clones
/// write through the same queue.
#[derive(Clone)]
pub struct Recorder {
    queue: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
    /// Events taken and not yet written: what a flush that did not finish
    /// says was left.
    queued: Arc<AtomicU64>,
    /// Whether the log has been told that the writer stopped.
    stopped_told: Arc<AtomicBool>,
    /// The writer, until it is started.
    unstarted: Arc<Mutex<Option<Writer>>>,
    id: Arc<str>,
    /// The writer's retention, which [`Recorder::set_retention`] changes.
    retention: Arc<std::sync::atomic::AtomicU32>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("writer", &self.id)
            .field("dropped", &self.dropped())
            .finish_non_exhaustive()
    }
}

/// What a flush came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Flushed {
    /// Everything taken before it is written, and the aggregates and the
    /// limits are up to date.
    Done,
    /// The writer was never started: nothing was waited for, and
    /// `waiting` events are still in its queue.
    NotStarted {
        /// Events not written.
        waiting: u64,
    },
    /// It was given up before the writer finished: `waiting` events had not
    /// been written then.
    GaveUp {
        /// Events not written.
        waiting: u64,
    },
    /// The writer's thread has ended: nothing more is written, and
    /// `waiting` events were still in its queue.
    Stopped {
        /// Events not written.
        waiting: u64,
    },
}

impl Flushed {
    /// The events left unwritten: none when it is done.
    #[must_use]
    pub const fn unwritten(self) -> u64 {
        match self {
            Self::Done => 0,
            Self::NotStarted { waiting } | Self::GaveUp { waiting } | Self::Stopped { waiting } => {
                waiting
            }
        }
    }
}

/// How a flush ended, as its waiter learns it.
#[derive(Debug, Clone, Copy)]
enum Ended {
    Written,
    GaveUp,
    Stopped,
}

#[derive(Debug, Default)]
struct FlushState {
    ended: Mutex<Option<Ended>>,
    changed: Condvar,
}

impl FlushState {
    fn lock(&self) -> MutexGuard<'_, Option<Ended>> {
        self.ended.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ends the flush, unless it has ended already.
    fn end(&self, how: Ended) {
        let mut ended = self.lock();
        if ended.is_none() {
            *ended = Some(how);
        }
        drop(ended);
        self.changed.notify_all();
    }
}

/// What stops a [`Recorder::flush_until`] that is waiting: [`GiveUp::now`],
/// from any thread, at whatever time limit its owner keeps (the core's, on
/// its timer). One for each flush.
#[derive(Debug, Clone, Default)]
pub struct GiveUp {
    state: Arc<FlushState>,
}

impl GiveUp {
    /// A handle that has not given up.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stops the wait: the flush returns [`Flushed::GaveUp`], unless it has
    /// ended already. The writer goes on with what it was doing.
    pub fn now(&self) {
        self.state.end(Ended::GaveUp);
    }
}

/// The writer's answer to a flush. Given when everything before it is
/// written; given as "stopped" when it is dropped unanswered, which is what
/// a writer thread that panicked does with it.
struct Answer(Arc<FlushState>);

impl Answer {
    fn written(&self) {
        self.0.end(Ended::Written);
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        self.0.end(Ended::Stopped);
    }
}

impl Recorder {
    /// A recorder: its queue, and a writer that is not started yet.
    ///
    /// Its [`writer`](Self::writer) id is made here, as a unique id is
    /// (`cortex_base::IdGenerator`): the time in milliseconds and 60 random
    /// bits from a generator seeded by the operating system in each
    /// process, so two recorders' ids are the same only if they are made in
    /// one millisecond and draw the same 60 bits.
    #[must_use]
    pub fn new(options: RecorderOptions, clock: SharedClock) -> Self {
        let (queue, receiver) = sync_channel(options.queue.max(1));
        let id: Arc<str> = IdGenerator::new(Arc::clone(&clock), Box::new(SystemRng::new()))
            .unique("w_")
            .into();
        let queued = Arc::new(AtomicU64::new(0));
        let retention = Arc::new(std::sync::atomic::AtomicU32::new(options.retention));
        Self {
            queue,
            dropped: Arc::new(AtomicU64::new(0)),
            queued: Arc::clone(&queued),
            stopped_told: Arc::new(AtomicBool::new(false)),
            unstarted: Arc::new(Mutex::new(Some(Writer {
                receiver,
                options,
                retention: Arc::clone(&retention),
                clock,
                queued,
            }))),
            id,
            retention,
        }
    }

    /// How many months a monthly file stays after its own, from the next
    /// time the files are pruned (`stats.retention`, reloaded).
    pub fn set_retention(&self, months: u32) {
        self.retention
            .store(months, std::sync::atomic::Ordering::Relaxed);
    }

    /// This recorder's id (`w_…`), unique to it among every process's: what
    /// the day's purse of its process makes its own id from, which the
    /// request lines that purse counted carry.
    #[must_use]
    pub fn writer(&self) -> &str {
        &self.id
    }

    /// Starts the writer thread, once. Calling it again does nothing.
    ///
    /// # Errors
    ///
    /// When the thread cannot be started. The events wait in the queue, or
    /// are dropped and counted once it is full.
    pub fn start(&self) -> io::Result<()> {
        let Some(writer) = self
            .unstarted
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

    /// Takes one event. Never waits: with the queue full, or the writer
    /// stopped, it is dropped and counted.
    pub fn record(&self, record: Record) {
        self.send(Message::Record(Box::new(record)));
    }

    /// Takes the limits a provider reported, which the writer keeps as the
    /// latest for that provider. Never waits.
    pub fn limits(&self, limits: ProviderLimits) {
        self.send(Message::Limits(Box::new(limits)));
    }

    fn send(&self, message: Message) {
        self.queued.fetch_add(1, Ordering::Relaxed);
        match self.queue.try_send(message) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                self.dropped.fetch_add(1, Ordering::Relaxed);
                self.tell_stopped();
            }
        }
    }

    /// Says once in the log that the writer has stopped.
    fn tell_stopped(&self) {
        if !self.stopped_told.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "the statistics writer has stopped: events from now on are dropped and counted, \
                 and not written"
            );
        }
    }

    /// How many events were dropped and not yet written as a count: those
    /// of a full queue, and those that came after the writer stopped.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Waits until everything taken so far is written and the aggregates
    /// are up to date, however long the writer takes: for tests, and for
    /// commands that end with their own writes. The end of a process uses
    /// [`flush_until`](Self::flush_until).
    pub fn flush(&self) -> Flushed {
        self.flush_until(&GiveUp::new())
    }

    /// As [`flush`](Self::flush), until `give_up` says to stop waiting.
    /// Never on the core thread while a session runs. Returns at once when
    /// the writer was never started, and as soon as the writer is found to
    /// have stopped.
    pub fn flush_until(&self, give_up: &GiveUp) -> Flushed {
        let waiting = || self.queued.load(Ordering::Relaxed);
        let started = self
            .unstarted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none();
        if !started {
            return Flushed::NotStarted { waiting: waiting() };
        }
        let state = &give_up.state;
        let mut message = Message::Flush(Answer(Arc::clone(state)));
        loop {
            match self.queue.try_send(message) {
                Ok(()) => break,
                Err(TrySendError::Full(back)) => {
                    // The writer makes room as it takes events: look again
                    // shortly, unless given up meanwhile.
                    let ended = state.lock();
                    let (ended, _) = state
                        .changed
                        .wait_timeout(ended, Duration::from_millis(5))
                        .unwrap_or_else(PoisonError::into_inner);
                    if ended.is_some() {
                        return Flushed::GaveUp { waiting: waiting() };
                    }
                    drop(ended);
                    message = back;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.tell_stopped();
                    return Flushed::Stopped { waiting: waiting() };
                }
            }
        }
        let mut ended = state.lock();
        loop {
            match *ended {
                Some(Ended::Written) => return Flushed::Done,
                Some(Ended::GaveUp) => return Flushed::GaveUp { waiting: waiting() },
                Some(Ended::Stopped) => {
                    drop(ended);
                    self.tell_stopped();
                    return Flushed::Stopped { waiting: waiting() };
                }
                None => {
                    ended = state
                        .changed
                        .wait(ended)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
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
    /// What the aggregates file was found to be when it could not be
    /// saved, once said in the log.
    told: Option<Found>,
}

fn run(writer: &Writer, dropped: &AtomicU64) {
    let mut state = State {
        zone: writer.options.zone.clone(),
        appender: Appender::default(),
        limits: BTreeMap::new(),
        unsynced: false,
        settled_at: writer.clock.monotonic(),
        told: None,
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
                writer.queued.fetch_sub(1, Ordering::Relaxed);
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
                writer.queued.fetch_sub(1, Ordering::Relaxed);
            }
            Some(Message::Flush(answer)) => {
                write_dropped(writer, &mut state, dropped);
                settle(writer, &mut state);
                answer.written();
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
    let mut record = record.clone();
    keep_plain(&mut record);
    let line = Line {
        day: zone.date_of(at).to_string(),
        record,
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

/// Keeps of the provider's and the model's own words only plain ones, and
/// `other` in place of the rest, whoever made the record: they are the
/// text of an answer, which may hold anything, a secret included, and the
/// statistics keep them for months (found by the independent review of
/// step 16, for an error's code). Three fields hold such words: a
/// request's error code, its stop reason when it is not one of the neutral
/// ones (`openai:<the provider's own>`), and the name of a tool the model
/// called that does not exist.
pub(crate) fn keep_plain(record: &mut Record) {
    fn plain_or_other(text: &mut String, plain: bool) {
        if !plain {
            code::OTHER.clone_into(text);
        }
    }
    match record {
        Record::Request(request) => {
            if let Some(error) = &mut request.error {
                let plain = is_plain_code(error);
                plain_or_other(error, plain);
            }
            let plain = is_plain_code(&request.stop)
                || request
                    .stop
                    .split_once(':')
                    .is_some_and(|(provider, own)| is_plain_code(provider) && is_plain_code(own));
            plain_or_other(&mut request.stop, plain);
        }
        Record::Tool(tool) if tool.outcome == crate::ToolOutcome::UnknownTool => {
            let plain = is_plain_code(&tool.tool);
            plain_or_other(&mut tool.tool, plain);
        }
        _ => {}
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
        if let Ok(mut aggregates) = Aggregates::load_for_saving(dir, writer.clock.as_ref()) {
            let found = aggregates.found().clone();
            if !found.may_save() && state.told.as_ref() != Some(&found) {
                tracing::warn!(
                    aggregates = ?found,
                    "the statistics' aggregates file is not one this build may write: events are \
                     still recorded in the monthly files, and the aggregates are left as they are"
                );
                state.told = Some(found);
            }
            // Aggregates that may not be saved are not brought up to date
            // at all: they would read every monthly file from its start
            // each time, to throw what they read away (found by the review
            // of step 16's fixes).
            if aggregates.found().may_save()
                && let Ok(true) = aggregates.sync(
                    dir,
                    zone.date_of(now),
                    &month_of(now),
                    Some(writer.retention.load(std::sync::atomic::Ordering::Relaxed)),
                )
            {
                let _ = aggregates.save(dir, writer.clock.as_ref());
            }
        }
    }
    if !state.limits.is_empty() {
        let limits = std::mem::take(&mut state.limits);
        let _ = LimitsFile::update(dir, limits.into_values(), writer.clock.as_ref());
    }
}

#[cfg(test)]
mod tests;
