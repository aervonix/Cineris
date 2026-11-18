//! Where a session's events go: one bounded queue for each client, which
//! merges deltas and never makes the loop wait
//! (`docs/architecture/02-runtime.md`, "Event backpressure").
//!
//! The loop hands every event to the session's [`SessionSink`], which
//! gives a copy to the queue of each client that has the session open. A
//! queue takes an event without waiting, whatever its reader is doing:
//!
//! 1. **Deltas of one item are merged** when the reader is behind: a delta
//!    that follows a delta of the same kind for the same item, with nothing
//!    queued between them, is added to it. The reader sees fewer, larger
//!    deltas. **Never across a read:** once the client has read a session
//!    at a `seq` (`session/read`), a delta is not added to a queued one at
//!    or below it, so the events after the read hold only what came after
//!    it (the review of step 23's fixes: a merged delta that spanned the
//!    read was drawn twice in part).
//! 2. **When the queue is full, deltas of that item are dropped**, and one
//!    `item.resync` is queued in place of the first. From then until the
//!    item completes, the reader gets none of its deltas: it reads what the
//!    item has so far with `session/read`, and `item.completed` has all of
//!    it. **Full is by count or by bytes** (since step 27): past its
//!    capacity of events, or once the deltas it holds come to 8 KiB for
//!    each place of that capacity (8 MiB at [`QUEUE_CAPACITY`]), merged
//!    deltas counted whole. A merged delta takes no place, so without the
//!    bytes a reader that stops during one long item would hold all of it:
//!    a command's output streams at about 3.7 MiB a second.
//! 3. **Nothing else is ever dropped.** A completed item, a turn's events
//!    and a request to the client are queued even when the queue is full:
//!    they are few, and each is bounded. (A transport that cannot hold them
//!    closes the connection; that is the RPC server's rule, in its step.)
//!
//! A client that did not ask for deltas (`capabilities.deltas`) is sent
//! none.
//!
//! **A client reads `seq` in increasing order, with jumps.** Every event
//! keeps the number the session gave it; a merged delta has its last
//! part's, and a resync the number of the delta it stands for. Nothing is
//! queued out of order. A jump is a delta the client was not sent, so it
//! is no sign of a loss: a loss is said by `item.resync`.
//!
//! The sink also keeps **what each item in progress holds so far**
//! ([`InProgress`]), built from the same deltas, which is what
//! `session/read` returns for an item that has not completed.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cineris_agent::EventSink;
use cineris_protocol::event::{Delta, Event, ItemDelta, ItemKind, ItemRef, TextDelta};
use cineris_protocol::item::{AssistantMessageItem, ToolOutcome, ToolResultItem};
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::message::{ReasoningPart, ToolCallPart};
use cineris_protocol::methods::ServerRequest;
use cineris_protocol::{CallId, Item, ItemId, Part, SessionEvent, SessionId, TurnId};
use serde_json::Value;
use tokio::sync::Notify;

/// How many events may wait for one client before its deltas are dropped.
pub const QUEUE_CAPACITY: usize = 1024;

/// How many bytes of deltas a queue holds for each place of its capacity
/// before its deltas are dropped: 8 MiB at [`QUEUE_CAPACITY`].
///
/// Measured in step 27 (`docs/architecture/02-runtime.md`, "Event
/// backpressure"): a reader that reads nothing holds an answer as one
/// merged delta, 1.7 MiB for the longest the built-in catalog allows
/// (393,216 tokens), and a command's output at the runner's own rate, 3.7
/// MiB a second. So every answer stays whole, and a stopped reader holds
/// about two seconds of a command that writes as fast as it can.
pub const DELTA_BYTES_PER_PLACE: usize = 8 * 1024;

/// How many bytes of deltas may wait for a client with a queue of
/// [`QUEUE_CAPACITY`] before its deltas are dropped: 8 MiB.
pub const QUEUE_DELTA_BYTES: usize = QUEUE_CAPACITY * DELTA_BYTES_PER_PLACE;

/// What the outcome of a tool result in progress is called: the call is
/// still running.
pub const RUNNING: &str = "running";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every change to a queue or a tracker is complete before its lock is
    // let go.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One thing the core sends a client.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "nearly everything sent is an event, so a box would be an allocation per event"
)]
pub enum Outgoing {
    /// An event of a session the client has open.
    Event(SessionEvent),
    /// A request the client is to answer with
    /// [`Core::respond`](crate::Core::respond): an approval.
    Request {
        /// The request's id, which the answer names.
        id: RequestId,
        /// The request.
        request: ServerRequest,
    },
}

#[derive(Debug, Default)]
struct QueueState {
    items: VecDeque<Outgoing>,
    /// The bytes of the deltas in `items` ([`delta_size`]).
    delta_bytes: usize,
    /// The items whose deltas are being dropped, each told so once.
    dropping: BTreeSet<ItemId>,
    /// For each session the client has read, the `seq` it read at: no
    /// delta is merged into a queued one at or below it.
    fences: BTreeMap<SessionId, u64>,
    closed: bool,
}

/// What a front end that does not wait on [`EventQueue::next`] is told
/// when its queue has something again (step 23): the interface's loop,
/// which reads with [`EventQueue::try_next`] on a thread of its own.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// One client's queue of events and requests.
pub struct EventQueue {
    state: Mutex<QueueState>,
    wake: Notify,
    /// Called when the queue goes from empty to holding something, and when
    /// it closes. A reader that leaves something in it comes back by
    /// itself, so one call a burst is enough.
    hook: Mutex<Option<Wake>>,
    capacity: usize,
    /// Past this many bytes of deltas held, the queue is full too.
    delta_bytes: usize,
    deltas: bool,
}

impl std::fmt::Debug for EventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventQueue")
            .field("state", &self.state)
            .field("capacity", &self.capacity)
            .field("delta_bytes", &self.delta_bytes)
            .field("deltas", &self.deltas)
            .finish_non_exhaustive()
    }
}

impl EventQueue {
    /// A queue that holds `capacity` events, and deltas of at most
    /// [`DELTA_BYTES_PER_PLACE`] for each of them, before it drops deltas,
    /// for a client that does or does not want deltas at all.
    #[must_use]
    pub fn new(capacity: usize, deltas: bool) -> Self {
        let capacity = capacity.max(1);
        Self {
            state: Mutex::new(QueueState::default()),
            wake: Notify::new(),
            hook: Mutex::new(None),
            capacity,
            delta_bytes: capacity.saturating_mul(DELTA_BYTES_PER_PLACE),
            deltas,
        }
    }

    /// Calls `wake` whenever the queue goes from empty to holding
    /// something, and when it closes: for a reader on a thread of its own
    /// that takes what is there with [`try_next`](Self::try_next) and then
    /// waits for something else. A reader that leaves something in the
    /// queue is not called again until it has emptied it. Called at once if
    /// the queue holds something already.
    pub fn set_wake(&self, wake: Wake) {
        *lock(&self.hook) = Some(Arc::clone(&wake));
        let state = lock(&self.state);
        let now = !state.items.is_empty() || state.closed;
        drop(state);
        if now {
            wake();
        }
    }

    /// After something was queued: wakes a waiting reader, and the hook if
    /// the queue was empty before it.
    fn queued(&self, was_empty: bool) {
        self.wake.notify_one();
        if was_empty {
            let hook = lock(&self.hook).clone();
            if let Some(hook) = hook {
                hook();
            }
        }
    }

    /// Takes an event. Never waits.
    pub fn push(&self, event: SessionEvent) {
        let mut state = lock(&self.state);
        if state.closed {
            return;
        }
        match &event.event {
            Event::ItemDelta(delta) => {
                if !self.deltas || state.dropping.contains(&delta.item_id) {
                    return;
                }
                let size = delta_size(&delta.delta);
                // Full of bytes: what it holds of deltas has reached the
                // bound, merged ones counted whole. Nothing more is merged.
                let full_of_bytes = state.delta_bytes >= self.delta_bytes;
                let QueueState {
                    items,
                    fences,
                    delta_bytes,
                    ..
                } = &mut *state;
                if !full_of_bytes
                    && let Some(Outgoing::Event(last)) = items.back_mut()
                    && fences
                        .get(&last.session_id)
                        .is_none_or(|fence| last.seq > *fence)
                    && merge(last, &event)
                {
                    *delta_bytes += size;
                    return;
                }
                if full_of_bytes || state.items.len() >= self.capacity {
                    // The reader is far behind: this item's deltas stop
                    // here, and it is told once, in this delta's place and
                    // under its number. One small event an item is queued
                    // past the limit, as an item's other whole events are.
                    let resync = SessionEvent {
                        session_id: event.session_id.clone(),
                        seq: event.seq,
                        turn_id: event.turn_id.clone(),
                        event: Event::ItemResync(ItemRef {
                            item_id: delta.item_id.clone(),
                        }),
                    };
                    state.dropping.insert(delta.item_id.clone());
                    let was_empty = state.items.is_empty();
                    state.items.push_back(Outgoing::Event(resync));
                    drop(state);
                    self.queued(was_empty);
                    return;
                }
            }
            Event::ItemCompleted(completed) => {
                // The whole item follows, and its deltas flow again.
                if let Some(id) = completed.item.id() {
                    state.dropping.remove(&ItemId::new(id));
                }
            }
            Event::ItemReset(reset) => {
                state.dropping.remove(&reset.item_id);
            }
            _ => {}
        }
        if let Event::ItemDelta(delta) = &event.event {
            state.delta_bytes += delta_size(&delta.delta);
        }
        let was_empty = state.items.is_empty();
        state.items.push_back(Outgoing::Event(event));
        drop(state);
        self.queued(was_empty);
    }

    /// Queues a request to the client. Never dropped.
    ///
    /// For a queue the caller made (a link outside the core, such as
    /// `cineris attach`'s): the core's own client queues are the core's to
    /// fill, fence and close.
    pub fn push_request(&self, id: RequestId, request: ServerRequest) {
        let mut state = lock(&self.state);
        if state.closed {
            return;
        }
        let was_empty = state.items.is_empty();
        state.items.push_back(Outgoing::Request { id, request });
        drop(state);
        self.queued(was_empty);
    }

    /// The client read `session` at `seq`: from now on no delta is merged
    /// into one queued at or below it.
    ///
    /// For a queue the caller made: the core fences its own client queues
    /// itself, and an `unfence` there would bring back the double draw the
    /// fence keeps out.
    pub fn fence(&self, session: &SessionId, seq: u64) {
        let mut state = lock(&self.state);
        let fence = state.fences.entry(session.clone()).or_insert(seq);
        *fence = (*fence).max(seq);
    }

    /// Forgets the fence of `session`: the client opened it again, or
    /// closed it.
    ///
    /// For a queue the caller made, as [`fence`](Self::fence) says.
    pub fn unfence(&self, session: &SessionId) {
        lock(&self.state).fences.remove(session);
    }

    /// The next thing for the client, if there is one. Never waits.
    pub fn try_next(&self) -> Option<Outgoing> {
        let mut state = lock(&self.state);
        let next = state.items.pop_front();
        if let Some(Outgoing::Event(event)) = &next
            && let Event::ItemDelta(delta) = &event.event
        {
            state.delta_bytes = state.delta_bytes.saturating_sub(delta_size(&delta.delta));
        }
        next
    }

    /// The next thing for the client. `None` once the client is
    /// disconnected and its queue is empty.
    pub async fn next(&self) -> Option<Outgoing> {
        loop {
            // Asked for before the queue is looked at, so that a push
            // between the look and the wait is not missed.
            let notified = self.wake.notified();
            if let Some(next) = self.try_next() {
                return Some(next);
            }
            if lock(&self.state).closed {
                return None;
            }
            notified.await;
        }
    }

    /// How many events and requests are waiting.
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.state).items.len()
    }

    /// Whether nothing is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many bytes the deltas waiting hold: their text, a merged delta's
    /// whole. Past [`DELTA_BYTES_PER_PLACE`] for each place of the queue's
    /// capacity, its deltas are dropped.
    #[must_use]
    pub fn delta_bytes(&self) -> usize {
        lock(&self.state).delta_bytes
    }

    /// The client is gone: what is queued can still be read, and nothing
    /// more is taken.
    ///
    /// For a queue the caller made: a closed queue drops what is pushed to
    /// it, a request included, so the core's own client queues are closed
    /// only by the core.
    pub fn close(&self) {
        lock(&self.state).closed = true;
        self.wake.notify_waiters();
        self.wake.notify_one();
        let hook = lock(&self.hook).clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Whether the client is disconnected: nothing more is queued, and
    /// what is there is all there will be.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        lock(&self.state).closed
    }
}

/// What a delta holds, in bytes, as a queue counts it: its text, or a tool
/// call's name and id. The same for a delta merged from parts as for the
/// parts added up.
fn delta_size(delta: &Delta) -> usize {
    match delta {
        Delta::Text(text) | Delta::Reasoning(text) | Delta::ToolOutput(text) => text.text.len(),
        Delta::ToolArgs(arguments) => arguments.json.len(),
        Delta::ToolCall(call) => call.name.len() + call.call_id.as_str().len(),
        Delta::Unknown(_) => 0,
    }
}

/// Adds `next` to `last` when both are deltas of the same kind for the
/// same item. The merged delta has the later `seq`.
fn merge(last: &mut SessionEvent, next: &SessionEvent) -> bool {
    if last.session_id != next.session_id || last.turn_id != next.turn_id {
        return false;
    }
    let (Event::ItemDelta(queued), Event::ItemDelta(new)) = (&mut last.event, &next.event) else {
        return false;
    };
    if queued.item_id != new.item_id {
        return false;
    }
    let merged = match (&mut queued.delta, &new.delta) {
        (Delta::Text(queued), Delta::Text(new))
        | (Delta::Reasoning(queued), Delta::Reasoning(new))
        | (Delta::ToolOutput(queued), Delta::ToolOutput(new)) => {
            queued.text.push_str(&new.text);
            true
        }
        (Delta::ToolArgs(queued), Delta::ToolArgs(new)) if queued.call_id == new.call_id => {
            queued.json.push_str(&new.json);
            true
        }
        _ => false,
    };
    if merged {
        last.seq = next.seq;
    }
    merged
}

/// What an item in progress holds so far.
#[derive(Debug, Clone)]
struct Partial {
    kind: ItemKind,
    tool: Option<String>,
    call_id: Option<CallId>,
    reasoning: String,
    text: String,
    output: String,
    /// The tool calls the model has begun, with what has arrived of their
    /// arguments.
    calls: Vec<(CallId, String, String)>,
}

/// The items of a session that have begun and not completed, with their
/// content so far.
#[derive(Debug, Default)]
pub(crate) struct InProgress {
    items: Vec<(ItemId, Partial)>,
}

impl InProgress {
    fn observe(&mut self, event: &Event) {
        match event {
            Event::ItemStarted(started) => {
                self.items.retain(|(id, _)| *id != started.item_id);
                self.items.push((
                    started.item_id.clone(),
                    Partial {
                        kind: started.kind.clone(),
                        tool: started.tool.clone(),
                        call_id: started.call_id.clone(),
                        reasoning: String::new(),
                        text: String::new(),
                        output: String::new(),
                        calls: Vec::new(),
                    },
                ));
            }
            Event::ItemDelta(ItemDelta { item_id, delta }) => {
                let Some((_, partial)) = self.items.iter_mut().find(|(id, _)| id == item_id) else {
                    return;
                };
                match delta {
                    Delta::Text(TextDelta { text }) => partial.text.push_str(text),
                    Delta::Reasoning(TextDelta { text }) => partial.reasoning.push_str(text),
                    Delta::ToolOutput(TextDelta { text }) => partial.output.push_str(text),
                    Delta::ToolCall(call) => {
                        partial.calls.push((
                            call.call_id.clone(),
                            call.name.clone(),
                            String::new(),
                        ));
                    }
                    Delta::ToolArgs(arguments) => {
                        if let Some((_, _, json)) = partial
                            .calls
                            .iter_mut()
                            .find(|(id, _, _)| *id == arguments.call_id)
                        {
                            json.push_str(&arguments.json);
                        }
                    }
                    Delta::Unknown(_) => {}
                }
            }
            Event::ItemReset(reset) => {
                // The partial content was discarded: the item begins again.
                if let Some((_, partial)) =
                    self.items.iter_mut().find(|(id, _)| *id == reset.item_id)
                {
                    partial.reasoning.clear();
                    partial.text.clear();
                    partial.output.clear();
                    partial.calls.clear();
                }
            }
            Event::ItemCompleted(completed) => {
                if let Some(done) = completed.item.id() {
                    self.items.retain(|(id, _)| id.as_str() != done);
                }
            }
            // A turn that is over has nothing in progress: an item that
            // began and never completed (a response thrown away) goes with
            // it.
            Event::TurnCompleted(_) => self.items.clear(),
            _ => {}
        }
    }

    /// The items in progress, oldest first, each with what it has so far.
    fn items(&self) -> Vec<Item> {
        self.items
            .iter()
            .filter_map(|(id, partial)| partial.item(id))
            .collect()
    }
}

impl Partial {
    fn item(&self, id: &ItemId) -> Option<Item> {
        match &self.kind {
            ItemKind::AssistantMessage => {
                let mut parts = Vec::new();
                if !self.reasoning.is_empty() {
                    parts.push(Part::Reasoning(ReasoningPart {
                        text: self.reasoning.clone(),
                        raw: None,
                    }));
                }
                if !self.text.is_empty() {
                    parts.push(Part::text(self.text.clone()));
                }
                for (call, name, json) in &self.calls {
                    parts.push(Part::ToolCall(ToolCallPart {
                        id: call.clone(),
                        name: name.clone(),
                        // Not valid JSON until the call is complete: the
                        // text so far, as text.
                        arguments: Value::String(json.clone()),
                        raw: None,
                    }));
                }
                Some(Item::AssistantMessage(AssistantMessageItem {
                    id: id.clone(),
                    parts,
                    stop_reason: None,
                    raw_stop: None,
                    stop_detail: None,
                    truncated: false,
                    origin: None,
                    usage: None,
                    error: None,
                    created_at: None,
                }))
            }
            ItemKind::ToolResult => Some(Item::ToolResult(ToolResultItem {
                id: id.clone(),
                call_id: self.call_id.clone()?,
                tool: self.tool.clone().unwrap_or_default(),
                outcome: ToolOutcome::from_wire(RUNNING),
                content: if self.output.is_empty() {
                    Vec::new()
                } else {
                    vec![Part::text(self.output.clone())]
                },
                payload: None,
                artifacts: Vec::new(),
                duration_ms: None,
                created_at: None,
            })),
            // Nothing else streams.
            _ => None,
        }
    }
}

/// The sink of one session: every client that has the session open gets
/// each event, and the items in progress are kept current.
#[derive(Debug)]
pub(crate) struct SessionSink {
    /// The session, by which a client's queue keeps the fence of its read.
    session: SessionId,
    subscribers: Mutex<Vec<(u64, Arc<EventQueue>)>>,
    progress: Mutex<InProgress>,
    /// The turn that is running and the `seq` of the last event, for a
    /// reader that asks where the session is.
    last: Mutex<(u64, Option<TurnId>)>,
}

impl SessionSink {
    /// The sink of `session`, with nobody subscribed.
    pub(crate) fn new(session: SessionId) -> Self {
        Self {
            session,
            subscribers: Mutex::default(),
            progress: Mutex::default(),
            last: Mutex::default(),
        }
    }

    /// Adds a client's queue, under the client's number. A client is added
    /// once. A fence the queue kept for this session from an earlier
    /// opening goes: the session numbers its events from 1 again (the
    /// review of step 23's fixes).
    pub(crate) fn subscribe(&self, client: u64, queue: Arc<EventQueue>) {
        let mut subscribers = lock(&self.subscribers);
        if !subscribers.iter().any(|(known, _)| *known == client) {
            queue.unfence(&self.session);
            subscribers.push((client, queue));
        }
    }

    /// Adds a client's queue as [`subscribe`](Self::subscribe) does, unless
    /// a client that has the session open already is one `excludes` names:
    /// then adds nothing and says so. Looked at and added under one lock,
    /// so two clients that may not share a session are never both added.
    pub(crate) fn subscribe_unless(
        &self,
        client: u64,
        queue: Arc<EventQueue>,
        excludes: impl Fn(u64) -> bool,
    ) -> bool {
        let mut subscribers = lock(&self.subscribers);
        if subscribers
            .iter()
            .any(|(known, _)| *known != client && excludes(*known))
        {
            return false;
        }
        if !subscribers.iter().any(|(known, _)| *known == client) {
            queue.unfence(&self.session);
            subscribers.push((client, queue));
        }
        true
    }

    /// Removes a client's queue, and the fence it kept for this session.
    /// Says whether it was there.
    pub(crate) fn unsubscribe(&self, client: u64) -> bool {
        let mut subscribers = lock(&self.subscribers);
        let before = subscribers.len();
        subscribers.retain(|(known, queue)| {
            let keep = *known != client;
            if !keep {
                queue.unfence(&self.session);
            }
            keep
        });
        subscribers.len() != before
    }

    /// The clients that have the session open.
    pub(crate) fn subscribers(&self) -> Vec<u64> {
        lock(&self.subscribers)
            .iter()
            .map(|(client, _)| *client)
            .collect()
    }

    /// The `seq` of the last event and the items in progress, taken
    /// together: the items hold exactly the events up to that `seq`, so a
    /// reader that takes the events after it draws nothing twice (found
    /// beside the review of step 23).
    /// `reader`, when given, is the queue of the client that reads, fenced
    /// at that `seq` under the same lock, so that no delta after it is
    /// merged into one before it.
    pub(crate) fn snapshot(&self, reader: Option<&EventQueue>) -> (u64, Vec<Item>) {
        let progress = lock(&self.progress);
        let seq = lock(&self.last).0;
        if let Some(queue) = reader {
            queue.fence(&self.session, seq);
        }
        (seq, progress.items())
    }
}

impl EventSink for SessionSink {
    fn emit(&self, event: SessionEvent) {
        {
            // One step under the items' lock, which a snapshot takes too.
            let mut progress = lock(&self.progress);
            progress.observe(&event.event);
            *lock(&self.last) = (event.seq, event.turn_id.clone());
        }
        let subscribers = lock(&self.subscribers);
        if let Some(((_, last), rest)) = subscribers.split_last() {
            for (_, queue) in rest {
                queue.push(event.clone());
            }
            last.push(event);
        }
    }
}

/// The session a request or an event is about, for whoever routes it.
#[must_use]
pub fn session_of(outgoing: &Outgoing) -> Option<&SessionId> {
    match outgoing {
        Outgoing::Event(event) => Some(&event.session_id),
        Outgoing::Request { request, .. } => match request {
            ServerRequest::Approval(approval) => Some(&approval.session_id),
            ServerRequest::Question(question) => Some(&question.session_id),
            ServerRequest::Trust(trust) => Some(&trust.session_id),
        },
    }
}

#[cfg(test)]
mod tests {
    use cineris_protocol::event::{
        ItemCompleted, ItemStarted, ToolArgsDelta, ToolCallDelta, TurnStarted,
    };
    use cineris_protocol::item::NoticeItem;
    use cineris_protocol::item::NoticeSource;

    use super::*;

    fn event(seq: u64, event: Event) -> SessionEvent {
        SessionEvent {
            session_id: SessionId::new("s_1"),
            seq,
            turn_id: Some(TurnId::new("t_1")),
            event,
        }
    }

    fn delta(seq: u64, item: &str, delta: Delta) -> SessionEvent {
        event(
            seq,
            Event::ItemDelta(ItemDelta {
                item_id: ItemId::new(item),
                delta,
            }),
        )
    }

    fn text(seq: u64, item: &str, text: &str) -> SessionEvent {
        delta(
            seq,
            item,
            Delta::Text(TextDelta {
                text: text.to_owned(),
            }),
        )
    }

    fn completed(seq: u64, item: &str) -> SessionEvent {
        event(
            seq,
            Event::ItemCompleted(ItemCompleted {
                item: Item::Notice(NoticeItem {
                    id: ItemId::new(item),
                    source: NoticeSource::Harness,
                    text: "done".to_owned(),
                    created_at: None,
                }),
            }),
        )
    }

    #[test]
    fn the_wake_hook_is_called_once_a_burst_and_when_the_queue_closes() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let queue = EventQueue::new(8, true);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        queue.set_wake(Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing queued yet");
        // A burst: one call, however much it holds.
        queue.push(event(1, Event::TurnStarted(TurnStarted {})));
        queue.push(completed(2, "i_1"));
        queue.push(completed(3, "i_2"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A reader that leaves something is not called again.
        let _ = queue.try_next();
        queue.push(completed(4, "i_3"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Emptied, the next thing is a new burst.
        let _ = drain(&queue);
        queue.push_request(
            RequestId::Number(1),
            ServerRequest::Question(cineris_protocol::methods::QuestionRequest {
                session_id: SessionId::new("s_1"),
                turn_id: None,
                question: "Go on?".to_owned(),
                choices: Vec::new(),
                extension: None,
            }),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let _ = queue.try_next();
        // Closing is told too, so a reader learns the client is gone.
        queue.close();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(queue.is_closed());
        // A hook set on a queue that holds something is called at once.
        let full = EventQueue::new(8, true);
        full.push(completed(1, "i_1"));
        let late = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&late);
        full.set_wake(Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(late.load(Ordering::SeqCst), 1);
    }

    fn drain(queue: &EventQueue) -> Vec<SessionEvent> {
        std::iter::from_fn(|| queue.try_next())
            .map(|outgoing| match outgoing {
                Outgoing::Event(event) => event,
                Outgoing::Request { .. } => panic!("a request"),
            })
            .collect()
    }

    fn label(event: &SessionEvent) -> String {
        match &event.event {
            Event::ItemDelta(delta) => match &delta.delta {
                Delta::Text(text) => format!("text:{}:{}", delta.item_id, text.text),
                Delta::Reasoning(text) => format!("reasoning:{}:{}", delta.item_id, text.text),
                Delta::ToolOutput(text) => format!("output:{}:{}", delta.item_id, text.text),
                Delta::ToolArgs(arguments) => format!("args:{}:{}", delta.item_id, arguments.json),
                Delta::ToolCall(call) => format!("call:{}:{}", delta.item_id, call.name),
                Delta::Unknown(_) => "unknown".to_owned(),
            },
            Event::ItemResync(resync) => format!("resync:{}", resync.item_id),
            Event::ItemCompleted(done) => format!("completed:{}", done.item.id().unwrap_or("")),
            Event::TurnStarted(_) => "turn.started".to_owned(),
            other => format!("{other:?}"),
        }
    }

    fn labels(queue: &EventQueue) -> Vec<String> {
        drain(queue).iter().map(label).collect()
    }

    #[test]
    fn deltas_of_one_item_and_kind_are_merged_while_the_reader_is_behind() {
        let queue = EventQueue::new(16, true);
        queue.push(text(1, "i_1", "The "));
        queue.push(text(2, "i_1", "build "));
        queue.push(text(3, "i_1", "fails."));
        let events = drain(&queue);
        assert_eq!(events.len(), 1);
        assert_eq!(label(&events[0]), "text:i_1:The build fails.");
        // The merged delta is the last of them.
        assert_eq!(events[0].seq, 3);
        // A reader that keeps up sees each delta as it came.
        queue.push(text(4, "i_1", "a"));
        assert_eq!(labels(&queue), ["text:i_1:a"]);
        queue.push(text(5, "i_1", "b"));
        assert_eq!(labels(&queue), ["text:i_1:b"]);
    }

    #[test]
    fn only_neighbours_of_the_same_item_and_kind_merge() {
        let queue = EventQueue::new(16, true);
        let reasoning = |seq, text: &str| {
            delta(
                seq,
                "i_1",
                Delta::Reasoning(TextDelta {
                    text: text.to_owned(),
                }),
            )
        };
        let args = |seq, call: &str, json: &str| {
            delta(
                seq,
                "i_1",
                Delta::ToolArgs(ToolArgsDelta {
                    call_id: CallId::new(call),
                    json: json.to_owned(),
                }),
            )
        };
        queue.push(reasoning(1, "hm"));
        queue.push(reasoning(2, "m"));
        queue.push(text(3, "i_1", "a"));
        queue.push(text(4, "i_2", "b"));
        queue.push(text(5, "i_1", "c"));
        queue.push(delta(
            6,
            "i_1",
            Delta::ToolCall(ToolCallDelta {
                call_id: CallId::new("c1"),
                name: "read".to_owned(),
            }),
        ));
        queue.push(args(7, "c1", "{\"pa"));
        queue.push(args(8, "c1", "th\":1}"));
        queue.push(args(9, "c2", "{}"));
        // Something that is not a delta stands between two that would
        // otherwise merge: order is kept.
        queue.push(text(10, "i_1", "d"));
        queue.push(event(11, Event::TurnStarted(TurnStarted {})));
        queue.push(text(12, "i_1", "e"));
        assert_eq!(
            labels(&queue),
            [
                "reasoning:i_1:hmm",
                "text:i_1:a",
                "text:i_2:b",
                "text:i_1:c",
                "call:i_1:read",
                "args:i_1:{\"path\":1}",
                "args:i_1:{}",
                "text:i_1:d",
                "turn.started",
                "text:i_1:e",
            ]
        );
    }

    #[test]
    fn a_full_queue_drops_an_items_deltas_and_tells_it_once() {
        let queue = EventQueue::new(3, true);
        queue.push(event(1, Event::TurnStarted(TurnStarted {})));
        queue.push(text(2, "i_1", "a"));
        queue.push(event(3, Event::TurnStarted(TurnStarted {})));
        // Full. This one cannot merge: a resync stands in its place, and
        // every later delta of the item is dropped.
        queue.push(text(4, "i_1", "b"));
        queue.push(text(5, "i_1", "c"));
        // Another item is dropped on its own account.
        queue.push(text(6, "i_2", "x"));
        queue.push(text(7, "i_2", "y"));
        // A whole event in between is queued as ever.
        queue.push(event(8, Event::TurnStarted(TurnStarted {})));
        queue.push(text(9, "i_1", "d"));
        let taken = drain(&queue);
        assert_eq!(
            taken.iter().map(label).collect::<Vec<_>>(),
            [
                "turn.started",
                "text:i_1:a",
                "turn.started",
                "resync:i_1",
                "resync:i_2",
                "turn.started",
            ]
        );
        // What the client reads is in the session's order: each resync has
        // the number of the delta it stands for.
        assert_eq!(
            taken.iter().map(|event| event.seq).collect::<Vec<_>>(),
            [1, 2, 3, 4, 6, 8]
        );
        // The item's deltas stay dropped once the reader has caught up.
        queue.push(text(10, "i_1", "e"));
        assert!(queue.is_empty(), "dropped until the item completes");
        // Its completion is never dropped, and its deltas flow again after
        // (a new item with the same id cannot be, but the state is clean).
        queue.push(completed(11, "i_1"));
        queue.push(text(12, "i_1", "after"));
        assert_eq!(labels(&queue), ["completed:i_1", "text:i_1:after"]);
    }

    /// Whatever is pushed, in the session's order, a reader that is behind
    /// by any amount reads numbers that only go up.
    #[test]
    fn what_a_client_reads_is_always_in_the_sessions_order() {
        use cineris_base::Rng as _;

        let mut rng = cineris_base::SeededRng::new(13);
        for capacity in 1..=6 {
            let queue = EventQueue::new(capacity, true);
            let mut read = Vec::new();
            for seq in 1..=400_u64 {
                let item = format!("i_{}", seq / 40 * 2 + rng.below(2));
                match rng.below(10) {
                    0 => queue.push(event(seq, Event::TurnStarted(TurnStarted {}))),
                    1 => queue.push(completed(seq, &item)),
                    _ => queue.push(text(seq, &item, "x")),
                }
                // The reader takes something now and then.
                if rng.below(4) == 0
                    && let Some(Outgoing::Event(event)) = queue.try_next()
                {
                    read.push(event.seq);
                }
            }
            read.extend(drain(&queue).iter().map(|event| event.seq));
            assert!(
                read.windows(2).all(|pair| pair[0] < pair[1]),
                "capacity {capacity}: {read:?}"
            );
        }
    }

    /// A reader that stops during one long item: its deltas merge into one,
    /// which takes one place, so the count never fills; the bytes do. The
    /// queue then drops the item's deltas with one resync, and holds no
    /// more than its bound and one delta. Whole events still go in.
    #[test]
    fn a_stopped_reader_holds_a_bounded_number_of_bytes_of_one_long_item() {
        let queue = EventQueue::new(4, true);
        let bound = 4 * DELTA_BYTES_PER_PLACE;
        let piece = "x".repeat(1024);
        queue.push(event(1, Event::TurnStarted(TurnStarted {})));
        for seq in 2..=200 {
            queue.push(text(seq, "i_1", &piece));
        }
        assert!(queue.len() <= 3, "{}", queue.len());
        assert!(
            queue.delta_bytes() >= bound && queue.delta_bytes() <= bound + piece.len(),
            "{} bytes held, the bound is {bound}",
            queue.delta_bytes()
        );
        // A whole event past the bound is queued as ever, and so is the
        // item's completion.
        queue.push(event(201, Event::TurnStarted(TurnStarted {})));
        queue.push(completed(202, "i_1"));
        let taken = drain(&queue);
        let kinds: Vec<String> = taken
            .iter()
            .map(|event| match &event.event {
                Event::ItemDelta(_) => "text".to_owned(),
                _ => label(event),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "turn.started",
                "text",
                "resync:i_1",
                "turn.started",
                "completed:i_1"
            ]
        );
        // The resync stands for the first delta dropped, under its number.
        let held = u64::try_from(bound / piece.len()).expect("a small count");
        assert_eq!(taken[2].seq, 2 + held);
        assert_eq!(
            queue.delta_bytes(),
            0,
            "a reader that took them all holds none"
        );
        // The next item's deltas go in and merge again.
        queue.push(text(203, "i_2", "a"));
        queue.push(text(204, "i_2", "b"));
        assert_eq!(labels(&queue), ["text:i_2:ab"]);
    }

    /// The bytes held go down as the reader takes deltas, so a reader that
    /// keeps up is never full of them, however much passes through.
    #[test]
    fn the_bytes_held_go_down_as_the_reader_takes_deltas() {
        let queue = EventQueue::new(1, true);
        let piece = "y".repeat(DELTA_BYTES_PER_PLACE / 2);
        for seq in 1..=100 {
            queue.push(text(seq, "i_1", &piece));
            assert_eq!(queue.delta_bytes(), piece.len());
            let Some(Outgoing::Event(event)) = queue.try_next() else {
                panic!("the delta was dropped");
            };
            assert!(matches!(event.event, Event::ItemDelta(_)));
            assert_eq!(queue.delta_bytes(), 0);
        }
        // Each kind is counted: a call's name and id, and its arguments.
        queue.push(delta(
            101,
            "i_2",
            Delta::ToolCall(ToolCallDelta {
                call_id: CallId::new("c1"),
                name: "read".to_owned(),
            }),
        ));
        assert_eq!(queue.delta_bytes(), "read".len() + "c1".len());
        let _ = queue.try_next();
        queue.push(delta(
            102,
            "i_2",
            Delta::ToolArgs(ToolArgsDelta {
                call_id: CallId::new("c1"),
                json: "{\"path\":1}".to_owned(),
            }),
        ));
        assert_eq!(queue.delta_bytes(), "{\"path\":1}".len());
    }

    #[test]
    fn an_item_that_completes_while_the_reader_is_behind_is_told_and_then_whole() {
        let queue = EventQueue::new(1, true);
        queue.push(event(1, Event::TurnStarted(TurnStarted {})));
        queue.push(text(2, "i_1", "a"));
        queue.push(text(3, "i_1", "b"));
        queue.push(completed(4, "i_1"));
        // The completed item is queued although the queue was full.
        assert_eq!(
            labels(&queue),
            ["turn.started", "resync:i_1", "completed:i_1"]
        );
    }

    #[test]
    fn whole_events_and_requests_are_never_dropped() {
        let queue = EventQueue::new(2, true);
        for seq in 1..=50 {
            queue.push(completed(seq, &format!("i_{seq}")));
        }
        assert_eq!(queue.len(), 50);
        let seqs: Vec<u64> = drain(&queue).iter().map(|event| event.seq).collect();
        assert_eq!(seqs, (1..=50).collect::<Vec<u64>>());
    }

    #[test]
    fn a_client_that_did_not_ask_for_deltas_gets_none() {
        let queue = EventQueue::new(16, false);
        queue.push(text(1, "i_1", "a"));
        queue.push(completed(2, "i_1"));
        assert_eq!(labels(&queue), ["completed:i_1"]);
    }

    #[test]
    fn a_closed_queue_takes_nothing_more_and_ends_when_it_is_empty() {
        let queue = EventQueue::new(16, true);
        queue.push(completed(1, "i_1"));
        queue.close();
        queue.push(completed(2, "i_2"));
        let next = cineris_testkit::block_on(queue.next());
        assert!(matches!(next, Some(Outgoing::Event(event)) if event.seq == 1));
        assert_eq!(cineris_testkit::block_on(queue.next()), None);
    }

    /// A client behind its queue reads the session while an answer
    /// streams: what the read gives and what the queue holds after its
    /// `seq` together carry each part of the answer exactly once (the
    /// review of step 23's fixes: a delta merged across the read's `seq`
    /// was drawn twice in part).
    #[test]
    fn a_read_and_the_events_after_it_hold_each_part_once() {
        let sink = SessionSink::new(SessionId::new("s_1"));
        let queue = Arc::new(EventQueue::new(16, true));
        sink.subscribe(1, Arc::clone(&queue));
        sink.emit(event(
            1,
            Event::ItemStarted(ItemStarted {
                item_id: ItemId::new("i_1"),
                kind: ItemKind::AssistantMessage,
                tool: None,
                call_id: None,
            }),
        ));
        sink.emit(text(2, "i_1", "A"));
        sink.emit(text(3, "i_1", "B"));
        let (seq, items) = sink.snapshot(Some(&queue));
        assert_eq!(seq, 3);
        sink.emit(text(4, "i_1", "C"));
        sink.emit(text(5, "i_1", "D"));
        let Item::AssistantMessage(read) = &items[0] else {
            panic!("not an assistant message");
        };
        let mut drawn: String = read
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect();
        while let Some(outgoing) = queue.try_next() {
            if let Outgoing::Event(event) = outgoing
                && event.seq > seq
                && let Event::ItemDelta(ItemDelta {
                    delta: Delta::Text(text),
                    ..
                }) = event.event
            {
                drawn.push_str(&text.text);
            }
        }
        assert_eq!(drawn, "ABCD");
    }

    /// A fence from a read of an earlier opening of the session goes when
    /// the client opens it again: that opening numbers its events from 1
    /// again, and its deltas merge as any others (the review of step 23's
    /// fixes).
    #[test]
    fn a_fence_from_an_earlier_opening_does_not_stop_merging() {
        let queue = Arc::new(EventQueue::new(16, true));
        let before = SessionSink::new(SessionId::new("s_1"));
        before.subscribe(1, Arc::clone(&queue));
        before.emit(text(5000, "i_0", "old"));
        let _ = before.snapshot(Some(&queue));
        while queue.try_next().is_some() {}
        assert!(before.unsubscribe(1));
        let again = SessionSink::new(SessionId::new("s_1"));
        again.subscribe(1, Arc::clone(&queue));
        again.emit(text(1, "i_1", "A"));
        again.emit(text(2, "i_1", "B"));
        let mut queued = Vec::new();
        while let Some(outgoing) = queue.try_next() {
            queued.push(outgoing);
        }
        assert_eq!(queued.len(), 1, "merged: {queued:?}");
        // Subscribing alone clears it too, for a client that never said it
        // closed the earlier opening.
        let _ = again.snapshot(Some(&queue));
        let third = SessionSink::new(SessionId::new("s_1"));
        third.subscribe(2, Arc::clone(&queue));
        third.emit(text(1, "i_2", "C"));
        third.emit(text(2, "i_2", "D"));
        let mut queued = Vec::new();
        while let Some(outgoing) = queue.try_next() {
            queued.push(outgoing);
        }
        assert_eq!(queued.len(), 1, "merged: {queued:?}");
    }

    #[test]
    fn the_sink_gives_every_subscriber_each_event_and_tracks_what_is_in_progress() {
        let sink = SessionSink::new(SessionId::new("s_1"));
        let (one, two) = (
            Arc::new(EventQueue::new(16, true)),
            Arc::new(EventQueue::new(16, false)),
        );
        sink.subscribe(1, Arc::clone(&one));
        sink.subscribe(2, Arc::clone(&two));
        sink.subscribe(1, Arc::clone(&one));
        assert_eq!(sink.subscribers(), [1, 2]);

        let started = |seq, item: &str, kind: ItemKind, tool: Option<&str>| {
            event(
                seq,
                Event::ItemStarted(ItemStarted {
                    item_id: ItemId::new(item),
                    kind,
                    tool: tool.map(str::to_owned),
                    call_id: tool.map(|_| CallId::new("c1")),
                }),
            )
        };
        sink.emit(started(1, "i_1", ItemKind::AssistantMessage, None));
        sink.emit(delta(
            2,
            "i_1",
            Delta::Reasoning(TextDelta {
                text: "thinking".to_owned(),
            }),
        ));
        sink.emit(text(3, "i_1", "Reading "));
        sink.emit(text(4, "i_1", "it."));
        sink.emit(delta(
            5,
            "i_1",
            Delta::ToolCall(ToolCallDelta {
                call_id: CallId::new("c1"),
                name: "read".to_owned(),
            }),
        ));
        sink.emit(delta(
            6,
            "i_1",
            Delta::ToolArgs(ToolArgsDelta {
                call_id: CallId::new("c1"),
                json: "{\"path\":".to_owned(),
            }),
        ));
        sink.emit(started(7, "i_2", ItemKind::ToolResult, Some("shell")));
        sink.emit(delta(
            8,
            "i_2",
            Delta::ToolOutput(TextDelta {
                text: "compiling\n".to_owned(),
            }),
        ));
        assert_eq!(sink.snapshot(None).0, 8);

        let items = sink.snapshot(None).1;
        assert_eq!(items.len(), 2);
        let Item::AssistantMessage(message) = &items[0] else {
            panic!("not an assistant message");
        };
        assert_eq!(message.stop_reason, None, "still streaming");
        assert!(matches!(&message.parts[0], Part::Reasoning(part) if part.text == "thinking"));
        assert!(matches!(&message.parts[1], Part::Text(part) if part.text == "Reading it."));
        assert!(matches!(
            &message.parts[2],
            Part::ToolCall(call)
                if call.name == "read" && call.arguments == Value::String("{\"path\":".to_owned())
        ));
        let Item::ToolResult(result) = &items[1] else {
            panic!("not a tool result");
        };
        assert_eq!(result.outcome.as_str(), RUNNING);
        assert_eq!(result.tool, "shell");
        assert!(matches!(&result.content[0], Part::Text(part) if part.text == "compiling\n"));

        // A reset clears what the item had, and its completion ends it.
        sink.emit(event(
            9,
            Event::ItemReset(ItemRef {
                item_id: ItemId::new("i_1"),
            }),
        ));
        let Item::AssistantMessage(message) = &sink.snapshot(None).1[0] else {
            panic!("not an assistant message");
        };
        assert!(message.parts.is_empty());
        sink.emit(completed(10, "i_2"));
        assert_eq!(sink.snapshot(None).1.len(), 1);

        // The client that wanted deltas got them; the other got the whole
        // events only.
        assert_eq!(drain(&one).len(), 9, "merged where they could be");
        let whole: Vec<u64> = drain(&two).iter().map(|event| event.seq).collect();
        assert_eq!(whole, [1, 7, 9, 10]);

        assert!(sink.unsubscribe(2));
        assert!(!sink.unsubscribe(2));
        sink.emit(completed(11, "i_1"));
        assert!(two.is_empty());
        assert_eq!(one.len(), 1);
        assert!(sink.snapshot(None).1.is_empty());
    }
}
