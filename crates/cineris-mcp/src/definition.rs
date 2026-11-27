//! A server as the configuration defines it (`docs/architecture/13-mcp.md`,
//! "Configuration").
//!
//! The harness's configuration crate reads `[mcp.servers.<name>]` and the
//! project's `.mcp.json`; the core turns each entry into a
//! [`ServerDefinition`]. Values that may be credentials are held as where
//! they come from ([`ValueSource`]), and resolved only when the server
//! starts.

use std::time::Duration;

use cineris_base::Secret;

/// Where a server's definition came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// The user's own configuration.
    User,
    /// The project's `.cineris/config.toml`.
    ProjectConfig,
    /// The project's `.mcp.json`, another client's file read live.
    ProjectMcpJson,
}

impl Origin {
    /// Whether the definition is a project's, which needs trust and its own
    /// approval before it runs.
    #[must_use]
    pub fn is_project(self) -> bool {
        !matches!(self, Self::User)
    }
}

/// How the model reaches a server's tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Expose {
    /// Through `tool_search` and `tool_call` (the default).
    #[default]
    Search,
    /// Declared to the model like built-in tools, from the next context
    /// epoch.
    Direct,
}

/// Where a value given to a server comes from.
#[derive(Clone)]
pub enum ValueSource {
    /// The user's environment variable of this name, read when the server
    /// starts.
    Env(String),
    /// A literal written in the configuration.
    Literal(Secret),
}

impl PartialEq for ValueSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Env(first), Self::Env(second)) => first == second,
            (Self::Literal(first), Self::Literal(second)) => first.expose() == second.expose(),
            _ => false,
        }
    }
}

impl Eq for ValueSource {}

impl std::fmt::Debug for ValueSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Env(name) => write!(f, "Env({name:?})"),
            Self::Literal(_) => f.write_str("Literal([redacted])"),
        }
    }
}

/// One environment variable or one header given to a server: its name, and
/// where its value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvEntry {
    /// The variable's or the header's name, as the server receives it.
    pub name: String,
    /// Where the value comes from.
    pub value: ValueSource,
}

/// How a server is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// A local server: a program the harness starts, spoken to over its
    /// standard input and output.
    Local {
        /// The program, as the entry names it.
        program: String,
        /// Its arguments.
        args: Vec<String>,
        /// The folder it starts in, as the entry gives it; the workspace when
        /// absent.
        cwd: Option<String>,
        /// The variables the entry adds to the base environment.
        env: Vec<EnvEntry>,
        /// Whether its confinement allows the network.
        network: bool,
        /// Whether its confinement allows writes to the workspace.
        write: bool,
    },
    /// A remote server, over Streamable HTTP.
    Remote {
        /// Its endpoint.
        url: String,
        /// The headers sent with every request.
        headers: Vec<EnvEntry>,
    },
}

/// One server, as the configuration defines it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDefinition {
    /// Its name: the part of a tool's full name before the `/`.
    pub name: String,
    /// Where it was defined.
    pub origin: Origin,
    /// How it is reached.
    pub launch: Launch,
    /// The entry's own one-line description, for the system prompt's list
    /// of sources. Never the server's own text.
    pub description: Option<String>,
    /// Whether the entry is turned on.
    pub enabled: bool,
    /// How the model reaches its tools.
    pub expose: Expose,
    /// The only tools offered, when given.
    pub tools: Option<Vec<String>>,
    /// Tools never offered.
    pub deny_tools: Vec<String>,
    /// How long its start may take.
    pub start_timeout: Duration,
    /// How long one call may take.
    pub call_timeout: Duration,
}
