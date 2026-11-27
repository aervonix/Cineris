//! How the harness starts a local server (`docs/architecture/13-mcp.md`,
//! "Lifecycle"; `docs/steps/30/answered.md`, points 3.1 to 3.3).
//!
//! The harness's core implements [`Spawner`]: it finds the program as an
//! extension's program is found, runs a script shim on Windows through the
//! command interpreter with arguments it escaped itself, gives the explicit
//! environment, confines the process where the sandbox enforces, and puts
//! it in a process tree the staged kill can end. What comes back is the
//! server's standard input and output as lines, and its process.

use std::time::Duration;

use cineris_tools::BoxFuture;

use crate::definition::ServerDefinition;

/// Why a line of a server's output was not passed on. The output ends
/// after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LineError {
    /// A line longer than the cap.
    #[error("the server wrote a line longer than {0} bytes")]
    TooLong(usize),
}

/// A started server: its pipes and its process.
pub struct ServerPipes {
    /// Lines for its standard input, each without its line end, which the
    /// writer adds. Dropping the sender closes the server's input.
    pub input: tokio::sync::mpsc::Sender<Vec<u8>>,
    /// The lines it writes to its standard output, without their line ends.
    /// The channel ends when its output closes, after an error if one
    /// ended it.
    pub output: tokio::sync::mpsc::Receiver<Result<Vec<u8>, LineError>>,
    /// The process.
    pub process: Box<dyn ServerProcess>,
}

impl std::fmt::Debug for ServerPipes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerPipes").finish_non_exhaustive()
    }
}

/// A local server's process.
pub trait ServerProcess: Send + Sync {
    /// The end of what it wrote to its standard error, with what looks like
    /// a secret masked: for the person only, never the model.
    fn stderr_tail(&self) -> String;

    /// Resolves when the process has exited, with how, in words.
    fn exited(&self) -> BoxFuture<'_, String>;

    /// Closes its input, waits up to `grace` for it to exit, then ends its
    /// process tree by the staged kill.
    fn shutdown(&self, grace: Duration) -> BoxFuture<'_, ()>;
}

/// Why a server could not be started.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpawnError {
    /// Its program was not found where programs are looked for.
    #[error("{0}")]
    NotFound(String),
    /// It was refused before it ran: a program inside the workspace, an
    /// argument a script shim cannot carry, a variable it may not be given.
    #[error("{0}")]
    Refused(String),
    /// The system could not start it.
    #[error("{0}")]
    Failed(String),
}

/// Starts local servers, as the harness starts them.
pub trait Spawner: Send + Sync {
    /// Starts `server`, which must be [`crate::Launch::Local`].
    fn spawn(&self, server: &ServerDefinition) -> Result<ServerPipes, SpawnError>;
}
