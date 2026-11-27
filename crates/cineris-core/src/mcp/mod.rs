//! MCP servers as the harness starts and defines them
//! (`docs/architecture/13-mcp.md`; `docs/steps/30/answered.md`, points 3
//! and 5). `cineris-mcp` knows the protocol and does no input or output of
//! its own: this is where its [`Spawner`](cineris_mcp::Spawner) is filled,
//! and where the configuration's servers become its definitions.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`spawn`] | [`ServerSpawner`]: a local server's program found, its folder, its environment and its confinement, as an extension's program's are. |
//! | `process` | A started server's tree, its three pipes and its staged end. |
//! | [`env`] | [`server_env`]: the explicit list of variables a server is given. |
//! | [`definitions`] | [`definitions`](definitions::definitions): the user's servers and a project's kept-apart ones, as `cineris-mcp`'s definitions. |
//!
//! Nothing here starts a server in a session yet: the lifecycle that does
//! is a later part of step 30.

pub mod definitions;
pub mod env;
mod process;
pub mod spawn;

pub use definitions::{Definitions, Refused, definitions};
pub use env::{EnvParts, ServerEnv, server_env};
pub use process::{LINES_QUEUED, MAX_LINE, STAGE_GRACE, STDERR_KEPT};
pub use spawn::{Confinement, ServerSpawner, SpawnerOptions};
