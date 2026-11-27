//! The MCP client. Registers server tools with the dynamic-tool registry and reads other clients'
//! server definitions (`docs/architecture/13-mcp.md`).
//!
//! This crate knows MCP and does no input or output of its own: the harness
//! gives it a way to start a local server's process ([`Spawner`]) and, for
//! remote servers, a way to make HTTP requests. Everything the rest of the
//! harness does with a server goes through [`Client`], whichever
//! implementation of the protocol is behind it.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`definition`] | [`ServerDefinition`]: a server as the configuration gives it, with where it came from. |
//! | [`client`] | [`Client`]: one connected server, its tools, its calls and its changes. |
//! | [`process`] | [`Spawner`]: how the harness starts a local server, and the pipes it gives back. |
//! | [`StdioClient`] | Our own client of the protocol, both eras, over a local server's standard input and output. |
//! | `fake` (feature `test-kit`) | Fake servers of both eras, scripted, for tests. |
//!
//! Inside, the client is in layers: `jsonrpc` reads and writes the
//! messages; a `transport` carries them (`stdio` here, Streamable HTTP
//! later) and matches answers to requests; a `session` speaks the protocol
//! over any transport; `connect` finds a local server's era and makes the
//! [`StdioClient`]; `wire` reads the tools part of the protocol's types.

pub mod client;
mod connect;
pub mod definition;
#[cfg(feature = "test-kit")]
pub mod fake;
mod jsonrpc;
pub mod process;
mod session;
mod stdio;
mod transport;
mod wire;

pub use client::{CallOutput, Client, ClientError, Content, Era, ServerInfo, ToolDefinition};
pub use connect::{Respawn, StdioClient};
pub use definition::{EnvEntry, Expose, Launch, Origin, ServerDefinition, ValueSource};
pub use process::{LineError, ServerPipes, ServerProcess, SpawnError, Spawner};
pub use session::{
    ClientOptions, LISTEN_AGAIN, MAX_LISTING_BYTES, MAX_PAGES, MAX_ROUNDS, MAX_TOOLS,
};
