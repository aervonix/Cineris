//! One connected server, behind an interface of our own
//! (`docs/architecture/13-mcp.md`, "Implementation").
//!
//! [`Client`] is what the registry's tools, the lifecycle and the
//! management commands are written against. Which implementation of the
//! protocol is behind it (both eras, over standard I/O or Streamable HTTP)
//! is the client's own business; a test gives a fake.

use std::time::Duration;

use cineris_base::CancelToken;
use cineris_tools::{BoxFuture, Json};

/// Which era of the protocol a server speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Era {
    /// The revisions from `2026-07-28` on: no handshake, `server/discover`.
    Modern,
    /// The revisions with the `initialize` handshake, `2025-11-25` and
    /// earlier.
    Legacy,
}

/// What a server said about itself when it was connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// Its era.
    pub era: Era,
    /// The protocol revision agreed, such as `2025-06-18`.
    pub version: String,
    /// The name it gave itself, for the person only. Third-party text: it
    /// never reaches the model.
    pub name: Option<String>,
    /// Whether it says it tells the client when its tools change.
    pub list_changed: bool,
}

/// One tool, as the server defines it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    /// Its name within the server.
    pub name: String,
    /// Its title, if it gave one.
    pub title: Option<String>,
    /// Its description, if it gave one.
    pub description: Option<String>,
    /// The JSON Schema of its arguments.
    pub input_schema: Json,
    /// The JSON Schema of its structured result, if it gave one.
    pub output_schema: Option<Json>,
    /// Its annotations, as hints for the person only.
    pub annotations: Option<Json>,
    /// The definition exactly as the server sent it, which is what its hash
    /// is of.
    pub raw: Json,
}

/// One block of a tool's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// Text.
    Text(String),
    /// An image, not passed on in this step: its type and size.
    Image {
        /// Its media type.
        mime: String,
        /// Its size in bytes, decoded.
        bytes: usize,
    },
    /// Audio, not passed on in this step: its type and size.
    Audio {
        /// Its media type.
        mime: String,
        /// Its size in bytes, decoded.
        bytes: usize,
    },
    /// A link to a resource.
    Link {
        /// Its URI.
        uri: String,
        /// Its name.
        name: String,
    },
    /// A resource embedded in the result.
    Resource {
        /// Its URI.
        uri: String,
        /// Its media type, if given.
        mime: Option<String>,
        /// Its text, when it is text.
        text: Option<String>,
        /// The size of its data in bytes, when it is not text.
        bytes: usize,
    },
}

/// What a call returned.
#[derive(Debug, Clone, PartialEq)]
pub struct CallOutput {
    /// Its content blocks, in order.
    pub content: Vec<Content>,
    /// Its structured result, if it gave one.
    pub structured: Option<Json>,
    /// Whether the server marked it as the tool's error.
    pub is_error: bool,
}

/// Why something asked of a server did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// The server could not be started or reached.
    #[error("the server did not start: {0}")]
    NotStarted(String),
    /// The server's process ended, or its connection closed.
    #[error("the server stopped: {0}")]
    Stopped(String),
    /// It took longer than its limit.
    #[error("the server took longer than {} s to {what}", .limit.as_secs())]
    TimedOut {
        /// What it was asked to do.
        what: &'static str,
        /// The limit it passed.
        limit: Duration,
    },
    /// It was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The server broke the protocol.
    #[error("the server broke the protocol: {0}")]
    Protocol(String),
    /// The server answered with an error of the protocol.
    #[error("the server answered with error {code}: {message}")]
    Server {
        /// The JSON-RPC error code.
        code: i64,
        /// Its message, third-party text.
        message: String,
    },
    /// The connection broke while a call was running: the call may have
    /// run, and is not sent again (`docs/architecture/13-mcp.md`).
    #[error("the connection broke during the call, which may have run: {0}")]
    MayHaveRun(String),
}

/// One connected server.
///
/// Every method is bounded in time: a call by the definition's call limit,
/// a listing by the client's own. Dropping the client shuts the server down
/// as [`Client::shutdown`] does, without waiting.
pub trait Client: Send + Sync {
    /// What the server said about itself.
    fn info(&self) -> &ServerInfo;

    /// Every tool it offers now, every page read.
    fn list_tools<'a>(
        &'a self,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<Vec<ToolDefinition>, ClientError>>;

    /// Calls one tool. Cancelling `cancel` cancels the call as the server's
    /// era and transport say, and returns [`ClientError::Cancelled`].
    fn call<'a>(
        &'a self,
        tool: &'a str,
        arguments: Json,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<CallOutput, ClientError>>;

    /// A counter bumped each time the server says its tools changed (or its
    /// list's lifetime ran out): the lifecycle lists them again.
    fn changes(&self) -> tokio::sync::watch::Receiver<u64>;

    /// Resolves when the connection has ended, with why.
    fn closed(&self) -> BoxFuture<'_, ClientError>;

    /// Ends the connection: for a local server, its input closed, then its
    /// process tree ended by the staged kill.
    fn shutdown(&self) -> BoxFuture<'_, ()>;
}
