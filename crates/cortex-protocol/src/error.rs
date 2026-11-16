//! Errors as data: the kind every error has, and the object that carries one
//! to a client (`docs/architecture/02-runtime.md`, "Error handling").

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::ids::Timestamp;
use crate::open::open_enum;

open_enum! {
    /// What sort of failure an error is. It decides whether the harness
    /// retries, which exit code a headless run ends with, and what a client
    /// should offer the user.
    pub enum ErrorKind {
        /// Bad flags, arguments or request parameters.
        Usage => "usage",
        /// Invalid configuration.
        Config => "config",
        /// Missing or rejected credentials.
        Auth => "auth",
        /// The provider refused the request for a reason a retry will not
        /// change: an invalid request, a missing model, billing.
        Provider => "provider",
        /// A network failure, a timeout, an overload or a retryable rate
        /// limit. Retried automatically; seen by a client only once the
        /// retries are used up or the wait is too long.
        Transient => "transient",
        /// The model stopped in a way that is not success: a refusal, a
        /// content filter, the output limit.
        ModelStop => "model_stop",
        /// A required approval was denied or could not be asked.
        Permission => "permission",
        /// A configured budget was reached: requests, cost, tokens or time.
        Limit => "limit",
        /// The context cannot be made to fit, even after compaction.
        ContextOverflow => "context_overflow",
        /// A tool failed. Reported to the model, which may recover.
        Tool => "tool",
        /// Cancelled by the user or by a signal.
        Interrupted => "interrupted",
        /// A bug in the harness.
        Internal => "internal",
    }
}

/// An error, as every front end sees it: in `turn.completed`, on an
/// assistant item whose request failed, and as the `data` of a JSON-RPC
/// application error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ErrorData {
    /// What sort of failure this is. Clients branch on it.
    pub kind: ErrorKind,
    /// A stable, machine-readable name for the exact failure, in
    /// `snake_case`: one of [`code`](crate::error::code), a provider's own
    /// error code, or a code a later version adds. Clients may branch on the
    /// ones they know and must tolerate the rest.
    pub code: String,
    /// One or two sentences for a person.
    pub message: String,
    /// The provider's id for the failed request, for its support channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// When the provider said the request may be tried again, if it said so
    /// and the harness did not wait itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<Timestamp>,
    /// Structured detail whose shape depends on `code`. For
    /// `unsupported_protocol_version`: `{"supported": [1]}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Map<String, Value>>,
}

impl ErrorData {
    /// An error with a kind, a code and a message, and nothing else.
    #[must_use]
    pub fn new(kind: ErrorKind, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
            request_id: None,
            retry_at: None,
            details: None,
        }
    }
}

/// The error codes the protocol itself defines. Other crates add theirs; the
/// field is a string so that they can.
pub mod code {
    /// A request arrived before `initialize`.
    pub const NOT_INITIALIZED: &str = "not_initialized";
    /// `initialize` was sent twice on one connection.
    pub const ALREADY_INITIALIZED: &str = "already_initialized";
    /// Client and server have no protocol version in common. `details.supported`
    /// lists the server's.
    pub const UNSUPPORTED_PROTOCOL_VERSION: &str = "unsupported_protocol_version";
    /// The client's role may not call this method, or may not make this change.
    pub const ROLE_NOT_ALLOWED: &str = "role_not_allowed";
    /// No stored session has this id.
    pub const UNKNOWN_SESSION: &str = "unknown_session";
    /// The session is not open on this connection.
    pub const SESSION_NOT_OPEN: &str = "session_not_open";
    /// A prompt arrived while a turn was running and did not say whether to
    /// steer or to queue; or `session/rewind`, `session/undo` or
    /// `verify/run` arrived while one was running.
    pub const TURN_RUNNING: &str = "turn_running";
    /// A prompt, or a second change of history, arrived while the session's
    /// history was being changed (`session/rewind`). It is sent again when
    /// that is over.
    pub const HISTORY_CHANGING: &str = "history_changing";
    /// The server is shutting down and starts no session.
    pub const SHUTTING_DOWN: &str = "shutting_down";
    /// `verify/run` found no check it could run, and says why: none known,
    /// a project the user has not trusted, the `read-only` level, no shell
    /// (since step 19).
    pub const NO_CHECKS: &str = "no_checks";
    /// The session already has a supervisor.
    pub const SUPERVISOR_PRESENT: &str = "supervisor_present";
    /// A value named something this build does not know: a model, a level, a
    /// speed, an item.
    pub const UNKNOWN_VALUE: &str = "unknown_value";
    /// The level needs an enforcing sandbox and there is none.
    pub const SANDBOX_REQUIRED: &str = "sandbox_required";
    /// A lease, a grant or a rule has a field or a value this version does
    /// not know. It is refused whole: a restriction that is not understood
    /// cannot be enforced, and must not be dropped.
    pub const LEASE_NOT_UNDERSTOOD: &str = "lease_not_understood";

    /// What a code that is not a plain one is kept as, where a provider's
    /// code would be kept: the statistics.
    pub const OTHER: &str = "other";

    /// Whether `code` is a plain one (`invalid_api_key`,
    /// `rate_limit_exceeded`): lower-case ASCII letters, digits, `_`, `.`
    /// and `-`, at most 64 of them.
    ///
    /// A provider's error code is the text of its answer, which may be
    /// anything, a secret included: a code is kept, shown or logged only
    /// when it is plain (the early review of step 16's refresh, and the
    /// independent review's twin in the statistics).
    #[must_use]
    pub fn is_plain(code: &str) -> bool {
        !code.is_empty()
            && code.len() <= 64
            && code.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::code;

    #[test]
    fn only_a_plain_code_is_plain() {
        for plain in [
            "invalid_api_key",
            "rate_limit_exceeded",
            "a.b-c_1",
            &"x".repeat(64),
        ] {
            assert!(code::is_plain(plain), "{plain}");
        }
        for not in [
            "",
            "Refused",
            "your prompt 'sk-live-0123' was refused",
            "tab\there",
            &"x".repeat(65),
        ] {
            assert!(!code::is_plain(not), "{not}");
        }
    }
}
