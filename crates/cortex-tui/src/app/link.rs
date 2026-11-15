//! The application's one way to the core: operations out, events, requests
//! and results in, in `cortex-protocol`'s types and nothing else
//! (`docs/steps/23/answered.md`, point 1).
//!
//! The binary's in-process link runs the core on a thread of its own and
//! calls `cortex_core::Core::handle` there; step 27's `attach` is a second
//! link, over the RPC connection. Neither ever makes the interface wait:
//! [`Link::send`] and [`Link::respond`] queue, and [`Link::next`] takes what
//! has arrived. A link is given the terminal's waker, and wakes the
//! interface's loop when something arrives.

use cortex_protocol::ErrorData;
use cortex_protocol::SessionEvent;
use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::methods::{Op, OpResult, ServerRequest, ServerResponse};

/// Names an operation sent, so that its result can be matched to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ticket(pub u64);

/// What arrives from the core.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "nearly everything is an event, taken out and handled at once: a box would be an \
              allocation per event"
)]
pub enum Inbound {
    /// An event of a session this client has open.
    Event(SessionEvent),
    /// A request for the person to answer: an approval, a question, trust.
    Request(RequestId, ServerRequest),
    /// The result of an operation this client sent.
    Done(Ticket, Result<OpResult, ErrorData>),
    /// The core is gone: its thread ended, or the connection closed.
    Gone(ErrorData),
}

/// The application's way to the core. See the [module documentation](self).
pub trait Link: Send {
    /// Queues an operation. Never waits: its result comes back from
    /// [`next`](Self::next) as [`Inbound::Done`] with the ticket returned.
    fn send(&mut self, op: Op) -> Ticket;

    /// Answers a request the core put to this client. Never waits. An
    /// answer the core refuses (the request was settled meanwhile) is
    /// dropped: the core says how the request was settled in an event.
    fn respond(&mut self, request: RequestId, response: ServerResponse);

    /// What has arrived, oldest first. Never waits.
    fn next(&mut self) -> Option<Inbound>;
}
