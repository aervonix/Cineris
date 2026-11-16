//! The order a client is given what the core sends it and the results of
//! its operations (the review of step 23's fixes).
//!
//! A result that is ready is given only after the events and requests that
//! were queued when it was first seen, and no others: a history read's
//! result then never overtakes the events it holds, which a front end takes
//! while it reads and filters, and a stream of events that goes on does not
//! hold the result back. With no result ready, the client's queue in order.
//!
//! The in-process link of the terminal UI takes from it on the interface's
//! thread; an RPC connection's writer takes from it before each line. So a
//! client over the wire reads in the order an in-process one would.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use cortex_core::{EventQueue, Outgoing};

/// What is given next.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "nearly everything given is an event, taken and handled at once: a box would be an \
              allocation per event"
)]
pub enum Next<R> {
    /// An event or a request, from the client's queue.
    Core(Outgoing),
    /// The result of an operation.
    Result(R),
}

/// The count of queued events still owed before the result that is ready.
#[derive(Debug, Clone, Default)]
pub struct Order {
    ahead: Option<usize>,
}

impl Order {
    /// What comes next, if anything is there. Never waits.
    pub fn next<R>(&mut self, queue: &EventQueue, results: &Mutex<VecDeque<R>>) -> Option<Next<R>> {
        let ready = !results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty();
        if ready {
            let owed = self.ahead.get_or_insert_with(|| queue.len());
            if *owed > 0 {
                *owed -= 1;
                if let Some(next) = queue.try_next() {
                    return Some(Next::Core(next));
                }
            }
            self.ahead = None;
            return results
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .map(Next::Result);
        }
        queue.try_next().map(Next::Core)
    }
}
