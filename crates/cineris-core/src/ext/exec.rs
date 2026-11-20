//! `cineris.exec`: a program an extension asks the host to run
//! (`docs/steps/28/answered.md`, "The processes").
//!
//! Being built: the program's lookup (outside the workspace and the write
//! roots; on Windows `.exe`, `.cmd`, `.bat` a directory at a time), the
//! command line the permission engine reads, the decision, the launcher of
//! the extension's own (write roots from its manifest's globs, the network
//! only with `exec_network`, at every level), the look at protected paths
//! around a start with a write root, the shim's command line on Windows,
//! and the run with standard output and standard error apart.

use cineris_ext::host::{ExecOutput, ExecRequest, HostError};

use super::host::CallHost;

/// What a session runs an extension's programs with.
#[derive(Debug, Default)]
pub(crate) struct ExecParts {}

/// Runs `request` for the call `host` serves.
pub(crate) async fn run(
    _parts: &ExecParts,
    request: ExecRequest,
    _host: &CallHost<'_>,
) -> Result<ExecOutput, HostError> {
    Err(HostError::Failed(format!(
        "running `{}` is not available in this build yet",
        request.program.as_str()
    )))
}
