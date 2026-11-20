//! What a script may ask the host to do, and the order every such call is
//! checked in (`docs/steps/28/answered.md`, "The host functions").
//!
//! [`Host`] is the session's side: the core implements it, deciding each
//! call with the permission engine as the same effect a built-in tool
//! would declare, and doing the work. [`serve`] is the only way a script's
//! call reaches a host, and it checks the extension's manifest **first**:
//! a call the manifest refuses never reaches the host, so the policy only
//! ever sees what the manifest already allowed. The order is in this
//! function's code, not in a convention a caller could forget.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use cineris_base::path::{AbsPath, ResolvedPath};
use serde_json::Value as Json;

use crate::capability::{Capabilities, Refusal};
use crate::manifest::Program;

/// A future a host returns.
pub type HostFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Why a host call did not do what it asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    /// The extension's manifest does not allow it.
    #[error("{0}")]
    Refused(#[from] Refusal),
    /// The permission engine, or the person asked, did not allow it.
    #[error("not allowed: {0}")]
    Denied(String),
    /// What the call asked for is not allowed here: a question in a hook,
    /// a write outside a tool call.
    #[error("{0}")]
    NotHere(String),
    /// It was allowed and failed: the reason.
    #[error("{0}")]
    Failed(String),
}

/// Where the script's call is running: what it may do depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The main module's first evaluation: only registering is allowed.
    Evaluation,
    /// A tool call, in a round of the session.
    Tool,
    /// A hook. It decides by its answer: it may not put a question, and,
    /// outside any round, may not write.
    Hook,
    /// A command the user ran, outside any round: it may not write.
    Command,
}

/// A program to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    /// The program, as the manifest lists it.
    pub program: Program,
    /// Its arguments, each one argument.
    pub args: Vec<String>,
    /// Where it runs, as the script gave it: resolved by the host, inside
    /// the workspace. `None`: the workspace's root.
    pub cwd: Option<String>,
    /// Bytes for its standard input. `None`: none.
    pub stdin: Option<Vec<u8>>,
    /// How long it may run.
    pub timeout: Duration,
    /// Whether the manifest lets its programs reach the network.
    pub network: bool,
    /// The folders it may write where the sandbox confines it: made from
    /// the manifest's `write` globs.
    pub write_roots: Vec<AbsPath>,
}

/// What a program did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOutput {
    /// Its exit code. `None` when it was stopped or the system would not
    /// say.
    pub code: Option<i32>,
    /// What it wrote to its standard output, up to the cap.
    pub stdout: Vec<u8>,
    /// What it wrote to its standard error, up to the cap.
    pub stderr: Vec<u8>,
    /// Whether standard output was cut at the cap.
    pub stdout_cut: bool,
    /// Whether standard error was cut at the cap.
    pub stderr_cut: bool,
    /// Whether it ran into its time and was stopped.
    pub timed_out: bool,
}

/// One entry of a listing or a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Where it is, resolved.
    pub path: AbsPath,
    /// How it is shown to the script: relative to the workspace, with `/`.
    pub shown: String,
    /// Whether it is a folder.
    pub folder: bool,
    /// For a search: the line's number, from 1.
    pub line: Option<u64>,
    /// For a search: the line's text.
    pub text: Option<String>,
}

/// Which store a value is kept in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The session's, kept in its log: a resume or a fork has it as it was
    /// there.
    Session,
    /// The project's, kept in the state directory.
    Project,
}

/// The session's side of every host function. The core implements it.
///
/// Each method is called only after [`serve`] has checked the manifest,
/// with what it allowed. Each decides the call with the permission engine
/// as the same effect a built-in tool would declare, and does the work.
/// **No method has a default body**: a wrapper written before a method was
/// added would otherwise not forward it.
pub trait Host: Send + Sync {
    /// Resolves a path a script gave, as the built-in tools resolve theirs:
    /// `~`, a relative path, `..`, links followed to where they lead.
    fn resolve<'a>(&'a self, path: &'a str) -> HostFuture<'a, Result<ResolvedPath, HostError>>;

    /// Runs a program.
    fn exec(&self, request: ExecRequest) -> HostFuture<'_, Result<ExecOutput, HostError>>;

    /// Reads a file's text: from line `offset` (0 is the first), at most
    /// `limit` lines.
    fn read(
        &self,
        path: ResolvedPath,
        offset: u64,
        limit: Option<u64>,
    ) -> HostFuture<'_, Result<String, HostError>>;

    /// Writes a file's text, through the `write` tool's code.
    fn write(&self, path: ResolvedPath, text: String) -> HostFuture<'_, Result<(), HostError>>;

    /// Lists a folder, following no link; `glob` keeps the entries it
    /// matches.
    fn list(
        &self,
        path: ResolvedPath,
        glob: Option<String>,
    ) -> HostFuture<'_, Result<Vec<Entry>, HostError>>;

    /// Searches files under a folder for a regular expression, following
    /// no link.
    fn search(
        &self,
        pattern: String,
        path: ResolvedPath,
        glob: Option<String>,
    ) -> HostFuture<'_, Result<Vec<Entry>, HostError>>;

    /// Puts a question to the user. `None`: nobody answered, or nobody
    /// could be asked.
    fn ask(
        &self,
        question: String,
        choices: Vec<String>,
    ) -> HostFuture<'_, Result<Option<String>, HostError>>;

    /// Tells the user something.
    fn notify(&self, text: String);

    /// Says what the call is doing, for the user.
    fn progress(&self, text: String);

    /// A stored value.
    fn state_get(
        &self,
        key: String,
        scope: Scope,
    ) -> HostFuture<'_, Result<Option<Json>, HostError>>;

    /// Stores a value; `None` removes it.
    fn state_set(
        &self,
        key: String,
        value: Option<Json>,
        scope: Scope,
    ) -> HostFuture<'_, Result<(), HostError>>;

    /// A line for the harness's log.
    fn log(&self, line: String);
}

/// A call a script made, as it arrives from its runtime.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HostCall {
    /// `cineris.exec(program, args, options)`.
    Exec {
        /// The program as the script named it.
        program: String,
        /// Its arguments.
        args: Vec<String>,
        /// `options.cwd`.
        cwd: Option<String>,
        /// `options.stdin`.
        stdin: Option<String>,
        /// `options.timeout`, in milliseconds.
        timeout_ms: Option<u64>,
    },
    /// `cineris.fs.read(path, {offset, limit})`.
    Read {
        /// The path as the script gave it.
        path: String,
        /// The first line, from 0.
        offset: u64,
        /// The most lines.
        limit: Option<u64>,
    },
    /// `cineris.fs.write(path, text)`.
    Write {
        /// The path as the script gave it.
        path: String,
        /// The text.
        text: String,
    },
    /// `cineris.fs.list(path, {glob})`.
    List {
        /// The folder as the script gave it.
        path: String,
        /// Which entries.
        glob: Option<String>,
    },
    /// `cineris.fs.search(pattern, {path, glob})`.
    Search {
        /// The regular expression.
        pattern: String,
        /// Under which folder; `.` unless given.
        path: String,
        /// Which files.
        glob: Option<String>,
    },
    /// `cineris.ask(question, choices)`.
    Ask {
        /// The question.
        question: String,
        /// The answers to pick from; empty for free text.
        choices: Vec<String>,
    },
    /// `cineris.notify(text)`.
    Notify(String),
    /// `cineris.progress(text)`.
    Progress(String),
    /// `cineris.state.get(key, {scope})`.
    StateGet {
        /// The key.
        key: String,
        /// The store.
        scope: Scope,
    },
    /// `cineris.state.set(key, value, {scope})`.
    StateSet {
        /// The key.
        key: String,
        /// The value; `None` removes it.
        value: Option<Json>,
        /// The store.
        scope: Scope,
    },
    /// `cineris.log(...)` and `console.*`.
    Log(String),
}

/// What a host call gives back to the script.
pub type HostAnswer = Result<Json, HostError>;

/// The most a program's standard output, or its standard error, is kept.
pub const EXEC_OUTPUT_CAP: usize = 1024 * 1024;
/// A program's time unless the script says otherwise.
pub const EXEC_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// The most arguments a program is given, and the longest one.
pub const EXEC_MAX_ARGS: usize = 256;
/// The longest argument.
pub const EXEC_MAX_ARG_BYTES: usize = 32 * 1024;
/// The most input a program is given.
pub const EXEC_MAX_STDIN: usize = 1024 * 1024;
/// The most of a file `fs.read` returns, and the most `fs.write` takes.
pub const FS_MAX_BYTES: usize = 1024 * 1024;
/// The longest key of a stored value.
pub const STATE_MAX_KEY: usize = 256;
/// The longest text of a notice to the user.
pub const NOTIFY_MAX: usize = 1024;
/// The longest progress line.
pub const PROGRESS_MAX: usize = 200;
/// The longest line of the log.
pub const LOG_MAX: usize = 4096;

/// Serves one call a script made: the manifest first, then the host.
///
/// `time_left` bounds a program's own timeout. The answer is what the
/// script's promise resolves to, or the error it is rejected with.
pub async fn serve<H: Host + ?Sized>(
    call: HostCall,
    capabilities: &Capabilities,
    host: &H,
    phase: Phase,
    time_left: Duration,
) -> HostAnswer {
    if phase == Phase::Evaluation {
        return Err(HostError::NotHere(
            "a host function cannot be called while the extension's main module is first \
             evaluated: call it from a tool, a command or a hook"
                .to_owned(),
        ));
    }
    match call {
        HostCall::Exec {
            program,
            args,
            cwd,
            stdin,
            timeout_ms,
        } => {
            let program = capabilities.exec(&program)?.clone();
            if args.len() > EXEC_MAX_ARGS {
                return Err(HostError::Failed(format!(
                    "a program is given at most {EXEC_MAX_ARGS} arguments"
                )));
            }
            if let Some(long) = args.iter().position(|arg| arg.len() > EXEC_MAX_ARG_BYTES) {
                return Err(HostError::Failed(format!(
                    "argument {} is longer than 32 KiB",
                    long + 1
                )));
            }
            if stdin
                .as_ref()
                .is_some_and(|text| text.len() > EXEC_MAX_STDIN)
            {
                return Err(HostError::Failed(
                    "the input is longer than 1 MiB".to_owned(),
                ));
            }
            let asked = timeout_ms.map_or(EXEC_DEFAULT_TIMEOUT, Duration::from_millis);
            let output = host
                .exec(ExecRequest {
                    program,
                    args,
                    cwd,
                    stdin: stdin.map(String::into_bytes),
                    timeout: asked.min(time_left),
                    network: capabilities.declared().exec_network,
                    write_roots: capabilities.write_roots(),
                })
                .await?;
            Ok(serde_json::json!({
                "code": output.code,
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
                "stdoutCut": output.stdout_cut,
                "stderrCut": output.stderr_cut,
                "timedOut": output.timed_out,
            }))
        }
        HostCall::Read {
            path,
            offset,
            limit,
        } => {
            let resolved = host.resolve(&path).await?;
            capabilities.read(&resolved)?;
            let text = host.read(resolved, offset, limit).await?;
            Ok(Json::String(text))
        }
        HostCall::Write { path, text } => {
            if phase != Phase::Tool {
                return Err(HostError::NotHere(
                    "`cineris.fs.write` works only in a tool call, whose round records the change \
                     so that it can be undone"
                        .to_owned(),
                ));
            }
            if text.len() > FS_MAX_BYTES {
                return Err(HostError::Failed(
                    "the text is longer than 1 MiB".to_owned(),
                ));
            }
            let resolved = host.resolve(&path).await?;
            capabilities.write(&resolved)?;
            host.write(resolved, text).await?;
            Ok(Json::Null)
        }
        HostCall::List { path, glob } => {
            let resolved = host.resolve(&path).await?;
            capabilities.read(&resolved)?;
            let entries = host.list(resolved, glob).await?;
            Ok(entries_json(capabilities, entries))
        }
        HostCall::Search {
            pattern,
            path,
            glob,
        } => {
            let resolved = host.resolve(&path).await?;
            capabilities.read(&resolved)?;
            let entries = host.search(pattern, resolved, glob).await?;
            Ok(entries_json(capabilities, entries))
        }
        HostCall::Ask { question, choices } => {
            if phase == Phase::Hook {
                return Err(HostError::NotHere(
                    "a hook cannot put a question: it decides by its answer (`ask`)".to_owned(),
                ));
            }
            let answer = host.ask(question, choices).await?;
            Ok(answer.map_or(Json::Null, Json::String))
        }
        HostCall::Notify(text) => {
            host.notify(cut(&text, NOTIFY_MAX));
            Ok(Json::Null)
        }
        HostCall::Progress(text) => {
            host.progress(cut(&text, PROGRESS_MAX));
            Ok(Json::Null)
        }
        HostCall::StateGet { key, scope } => {
            check_key(&key)?;
            Ok(host.state_get(key, scope).await?.unwrap_or(Json::Null))
        }
        HostCall::StateSet { key, value, scope } => {
            check_key(&key)?;
            host.state_set(key, value, scope).await?;
            Ok(Json::Null)
        }
        HostCall::Log(line) => {
            host.log(cut(&line, LOG_MAX));
            Ok(Json::Null)
        }
    }
}

/// The entries the manifest lets the script read, each checked on its own:
/// a walk can reach a file the glob of its folder does not name.
fn entries_json(capabilities: &Capabilities, entries: Vec<Entry>) -> Json {
    Json::Array(
        entries
            .into_iter()
            .filter(|entry| capabilities.reads(&entry.path))
            .map(|entry| {
                let mut object = serde_json::Map::new();
                object.insert("path".to_owned(), Json::String(entry.shown));
                object.insert("folder".to_owned(), Json::Bool(entry.folder));
                if let Some(line) = entry.line {
                    object.insert("line".to_owned(), Json::from(line));
                }
                if let Some(text) = entry.text {
                    object.insert("text".to_owned(), Json::String(text));
                }
                Json::Object(object)
            })
            .collect(),
    )
}

fn check_key(key: &str) -> Result<(), HostError> {
    if key.is_empty() || key.len() > STATE_MAX_KEY {
        Err(HostError::Failed(format!(
            "a key is 1 to {STATE_MAX_KEY} bytes"
        )))
    } else {
        Ok(())
    }
}

/// `text` cut to at most `most` bytes, at a character's edge.
fn cut(text: &str, most: usize) -> String {
    cineris_base::text::truncate_bytes(text, most).to_owned()
}
