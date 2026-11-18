//! The JSON-RPC server over standard I/O and local sockets, on top of the
//! core (`docs/architecture/11-rpc-protocol.md`).
//!
//! The server adds transport and order, and no behaviour of its own: each
//! JSON-RPC method is an `Op` the core carries out (`Core::handle`), each
//! event a `session/event` notification, each request the core puts to a
//! client a JSON-RPC request, and each answer `Core::respond`. The method
//! tables are `cineris-protocol`'s, and the framing is `cineris-base`'s.
//!
//! | Module | What it does |
//! |---|---|
//! | [`lane`] | The order a front end's operations run in: one lane for what changes a session, reads beside it. The terminal UI's in-process link uses it too. |
//! | [`order`] | The order a client is given events and results: a result after the events queued before it. Shared with the in-process link. |
//! | `wire` | The protocol's types as lines of JSON-RPC, both ways. |
//! | [`transport`] | What carries a connection's bytes: channels to standard I/O's threads, a socket, or a test's own ends. |
//! | [`connection`] | One client: the reader, the lane, the writer, the bounds on what it may hold, the rule that closes a client that takes nothing, and the end. |
//! | [`socket`] | The local socket: where a server listens, who may connect, and each connection's bytes. |
//! | [`server`] | The core and its connections, and how they end together. |

pub mod connection;
pub mod lane;
pub mod order;
pub mod server;
pub mod socket;
pub mod transport;
mod wire;

pub use crate::connection::{Connection, Ended, ServeOptions};
pub use crate::server::{serve_one, serve_socket};
pub use crate::socket::{Accepted, ListenOptions, Listener, Place};
pub use crate::transport::{Ends, Transport, Written};
