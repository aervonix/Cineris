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

pub mod client;
pub mod definition;
pub mod process;

pub use client::{CallOutput, Client, ClientError, Content, Era, ServerInfo, ToolDefinition};
pub use definition::{EnvEntry, Expose, Launch, Origin, ServerDefinition, ValueSource};
pub use process::{LineError, ServerPipes, ServerProcess, SpawnError, Spawner};
