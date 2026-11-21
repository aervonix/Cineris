//! Where a session's events go
//! (`docs/architecture/11-rpc-protocol.md`, "Events").
//!
//! The loop tells its front ends everything through one trait,
//! [`EventSink`]. The core gives it the bounded, coalescing channel of
//! `docs/architecture/02-runtime.md`; a test gives it a list.

use std::sync::{Arc, Mutex, PoisonError};

use cineris_protocol::event::{Event, SessionEvent};
use cineris_protocol::{SessionId, TurnId};

/// Takes the events of a session, in order.
///
/// `emit` is called from the session's task and from the tasks of its tool
/// calls, never two at a time for one session. It must not block: a sink
/// that cannot keep up merges or drops deltas
/// (`docs/architecture/02-runtime.md`, "Event backpressure"), and never
/// makes the loop wait.
///
/// **It must not call the session's handle.** An event can be emitted while
/// the session's inbox is locked, so that a change and the event that tells
/// of it cannot be reordered, and a handle's methods take that lock.
pub trait EventSink: Send + Sync {
    /// The next event of the session. Its `seq` is one more than the last.
    fn emit(&self, event: SessionEvent);

    /// The prompt the server gave `ticket`
    /// ([`SessionHandle::prompt_with`](crate::SessionHandle::prompt_with))
    /// is the loop's no more: it was recorded in the conversation, or an
    /// interrupt handed it back undelivered. A prompt the loop gives back
    /// is told by its `prompt_returned` notice instead, which carries the
    /// ticket. Told once a ticket at most: a prompt the loop lets go of
    /// otherwise (a log that failed before it was recorded, a session
    /// closed with it waiting) is never told of, so a server keeps a bound
    /// of its own as well. A sink that gives no tickets need not write it.
    fn prompt_settled(&self, ticket: u64) {
        let _ = ticket;
    }
}

/// Numbers a session's events and hands them to its sink.
///
/// The number is given and the event handed over under one lock, so the
/// order the sink sees is the order of `seq`, whichever task emitted.
pub(crate) struct Emitter {
    session: SessionId,
    sink: Arc<dyn EventSink>,
    seq: Mutex<u64>,
}

impl Emitter {
    pub(crate) fn new(session: SessionId, sink: Arc<dyn EventSink>) -> Self {
        Self {
            session,
            sink,
            seq: Mutex::new(0),
        }
    }

    /// Sends one event, as part of `turn` or outside any turn.
    pub(crate) fn emit(&self, turn: Option<&TurnId>, event: Event) {
        // The counter is one number: a panic elsewhere cannot have left it
        // half-written.
        let mut seq = self.seq.lock().unwrap_or_else(PoisonError::into_inner);
        *seq += 1;
        self.sink.emit(SessionEvent {
            session_id: self.session.clone(),
            seq: *seq,
            turn_id: turn.cloned(),
            event,
        });
    }

    /// Tells the sink a prompt's ticket is settled
    /// ([`EventSink::prompt_settled`]).
    pub(crate) fn settled(&self, ticket: Option<u64>) {
        if let Some(ticket) = ticket {
            self.sink.prompt_settled(ticket);
        }
    }

    /// The `seq` of the last event sent.
    pub(crate) fn seq(&self) -> u64 {
        *self.seq.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl std::fmt::Debug for Emitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emitter")
            .field("session", &self.session)
            .field("seq", &self.seq())
            .finish_non_exhaustive()
    }
}
