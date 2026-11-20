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
use std::sync::Arc;
use std::time::Duration;

use cineris_base::path::{AbsPath, ResolvedPath};
use cineris_tools::ReadGate;
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
    /// The turn the call is part of was stopped: nothing more of it runs,
    /// and the script's call ends here.
    #[error("the turn was stopped: {0}")]
    Stopped(String),
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
    /// How long it may run: what it asked for, at most what was left of
    /// the call's time when it was asked for.
    pub timeout: Duration,
    /// What was left of the call's time when the program was asked for:
    /// its decision, which may wait on a person, and its run are held to
    /// it.
    pub call_left: Duration,
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

/// What the `list` or `search` tool gave: its text, as the model would be
/// given it, and its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    /// The tool's text.
    pub text: String,
    /// The tool's payload: counts, and whether it was cut.
    pub payload: Json,
}

/// The manifest's `read` globs as a gate for a walk: every entry a listing
/// or a search meets is put to it before it is read, so that a walk of a
/// folder the manifest allows never reaches a file it does not.
///
/// It is made by [`serve`] and handed to the host, which puts it over the
/// session's own gate ([`ManifestGate::over`]): the host cannot walk for a
/// script without it.
#[derive(Debug, Clone)]
pub struct ManifestGate {
    capabilities: Capabilities,
}

impl ManifestGate {
    /// The manifest's gate, in front of `inner`, the session's own: an
    /// entry is read only when both allow it.
    #[must_use]
    pub fn over(self, inner: Arc<dyn ReadGate>) -> Arc<dyn ReadGate> {
        Arc::new(Both {
            manifest: self,
            inner,
        })
    }

    /// Whether the manifest lets a walk read or enter `path`: a folder a
    /// `read` glob can reach under, or a file it names. Whether `path` is
    /// a folder is asked of the entry itself: a walk follows no link.
    #[must_use]
    #[allow(
        clippy::disallowed_methods,
        reason = "a walk's gate is asked on the walk's own threads, which block"
    )]
    pub fn allows(&self, path: &AbsPath) -> bool {
        let folder = std::fs::symlink_metadata(path.to_fs_path()).is_ok_and(|meta| meta.is_dir());
        if folder {
            self.capabilities.reaches_under(path) || self.capabilities.reads(path)
        } else {
            self.capabilities.reads(path)
        }
    }
}

struct Both {
    manifest: ManifestGate,
    inner: Arc<dyn ReadGate>,
}

impl ReadGate for Both {
    fn may_read(&self, path: &AbsPath) -> bool {
        self.inner.may_read(path) && self.manifest.allows(path)
    }
}

/// An HTTP request a script asks for. Its `Debug` shows no header value
/// and no query: either may carry a credential.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchRequest {
    /// The method, as the script wrote it: the host refuses one it does
    /// not make.
    pub method: String,
    /// The URL.
    pub url: String,
    /// The headers, each a name and a value.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
    /// How long it may take.
    pub timeout: Duration,
}

/// What an HTTP request gave. Its `Debug` shows no header value and no
/// body: either may be a credential.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchOutput {
    /// The status code.
    pub status: u16,
    /// The response's headers.
    pub headers: Vec<(String, String)>,
    /// The body, up to the cap.
    pub body: Vec<u8>,
    /// Whether the body was cut at the cap.
    pub cut: bool,
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

    /// Lists a folder through the `list` tool, its walk gated by `gate`
    /// over the session's own; `glob` keeps the entries it matches.
    fn list(
        &self,
        path: ResolvedPath,
        glob: Option<String>,
        gate: ManifestGate,
    ) -> HostFuture<'_, Result<Listing, HostError>>;

    /// Searches files under a folder for a regular expression through the
    /// `search` tool, its walk gated by `gate` over the session's own.
    fn search(
        &self,
        pattern: String,
        path: ResolvedPath,
        glob: Option<String>,
        gate: ManifestGate,
    ) -> HostFuture<'_, Result<Listing, HostError>>;

    /// Makes an HTTP request for the script. `host` is the URL's host, which
    /// the manifest named; `literal` is that host when the manifest names it
    /// literally (not through a pattern), and empty otherwise: only then may
    /// it be a loopback or a private address.
    fn fetch(
        &self,
        request: FetchRequest,
        host: String,
        literal: Vec<String>,
    ) -> HostFuture<'_, Result<FetchOutput, HostError>>;

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
    /// `cineris.http.fetch(url, {method, headers, body, timeout})`.
    Fetch(FetchCall),
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
/// The longest question a script puts.
pub const ASK_MAX: usize = 1024;
/// The most choices a question offers.
pub const ASK_MAX_CHOICES: usize = 8;
/// The longest choice.
pub const ASK_CHOICE_MAX: usize = 80;
/// A fetch's longest time, and its time unless the script asks for less;
/// never more than its call has left.
pub const FETCH_MAX_TIMEOUT: Duration = Duration::from_secs(30);

/// `cineris.http.fetch(url, {method, headers, body, timeout})`, as a script
/// called it. Its `Debug` shows no header value, no body and no query.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchCall {
    /// The URL.
    pub url: String,
    /// The method; `GET` unless given.
    pub method: String,
    /// The headers.
    pub headers: Vec<(String, String)>,
    /// The body, as text.
    pub body: Option<String>,
    /// `options.timeout`, in milliseconds.
    pub timeout_ms: Option<u64>,
}

impl std::fmt::Debug for FetchCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("FetchCall")
            .field("method", &self.method)
            .field("url", &origin_of(&self.url))
            .field("headers", &names)
            .field("body_bytes", &self.body.as_ref().map(String::len))
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

impl std::fmt::Debug for FetchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("FetchRequest")
            .field("method", &self.method)
            .field("url", &origin_of(&self.url))
            .field("headers", &names)
            .field("body_bytes", &self.body.len())
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// A URL's scheme and host, without its path, query or fragment, any of
/// which may carry a credential (a webhook's path is one): `…` says more
/// was there.
fn origin_of(url: &str) -> std::borrow::Cow<'_, str> {
    let start = url.find("://").map_or(0, |at| at + 3);
    match url[start..].find(['/', '?', '#']) {
        Some(at) => format!("{}…", &url[..start + at]).into(),
        None => url.into(),
    }
}

impl std::fmt::Debug for FetchOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("FetchOutput")
            .field("status", &self.status)
            .field("headers", &names)
            .field("body_bytes", &self.body.len())
            .field("cut", &self.cut)
            .finish()
    }
}

/// The host of a URL, in lower case, as the manifest's `net` patterns are
/// matched against it and the permission engine judges it: the part after
/// `https://` or `http://` and before the first `/`, `\`, `?` or `#`,
/// without its port; an IPv6 address without its brackets and in its
/// shortest form.
///
/// The fetch client reads the URL again, as the URL standard does, and the
/// host checked here must be the one it connects to, so a URL that reading
/// would take differently is refused:
///
/// - one that does not begin with `https://` or `http://`: the standard
///   skips any run of `/` and `\` after a scheme, so `https:good.com://evil`
///   is `good.com`;
/// - a control character or a space anywhere: it drops a tab or a line
///   break wherever it is, and trims the ends;
/// - a `\` anywhere: it reads one as `/`, so `https://evil\.good/` is
///   `evil`, which a question showing the URL would hide;
/// - a user part;
/// - a name with anything but letters, digits, `.`, `-` and `_` (a `%` is
///   decoded), or that ends in a number and is not four decimal numbers
///   (`0x7f.1`, `2130706433`, `127.1` and `0177.0.0.1` are `127.0.0.1`);
/// - an IPv6 address with anything but hex digits, colons and dots, or an
///   IPv4-mapped one, whose IPv4 address is written instead.
///
/// # Errors
///
/// When the URL is not written so.
pub fn url_host(url: &str) -> Result<String, HostError> {
    let refused =
        |why: &str| HostError::Failed(format!("`{}` cannot be fetched: {why}", origin_of(url)));
    if url.chars().any(|c| c.is_control() || c == ' ') {
        return Err(refused(
            "it has a control character or a space, which the fetch would drop or trim",
        ));
    }
    // The standard reads a `\` as a `/`, so a question that showed one
    // would show a host the fetch does not reach (`https://evil\.good/`
    // is `evil`).
    if url.contains('\\') {
        return Err(refused(
            "it has a `\\`, which the fetch reads as `/`: write `/`",
        ));
    }
    let rest = ["https://", "http://"]
        .iter()
        .find_map(|scheme| {
            url.get(..scheme.len())
                .filter(|start| start.eq_ignore_ascii_case(scheme))
                .map(|_| &url[scheme.len()..])
        })
        .ok_or_else(|| refused("it does not begin with `https://` or `http://`"))?;
    let authority = rest.split(['/', '\\', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(refused("it names a user"));
    }
    if let Some(inside) = authority.strip_prefix('[') {
        let (written, after) = inside
            .split_once(']')
            .ok_or_else(|| refused("its address is not closed"))?;
        if !(after.is_empty() || after.starts_with(':')) {
            return Err(refused("its address is followed by something not a port"));
        }
        // Only an IPv6 address, written plainly: no zone, no `%`.
        let address = written
            .parse::<std::net::Ipv6Addr>()
            .map_err(|_| refused("its host is not an IPv6 address written plainly"))?;
        if address.to_ipv4_mapped().is_some() {
            return Err(refused(
                "its host is an IPv4 address in IPv6's form: write the IPv4 address",
            ));
        }
        return Ok(address.to_string());
    }
    let host = authority
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err(refused("it has no host"));
    }
    let plain = host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if !plain {
        return Err(refused(
            "its host has a character the fetch would read differently: write it plainly",
        ));
    }
    if ends_in_a_number(&host) && host.parse::<std::net::Ipv4Addr>().is_err() {
        return Err(refused(
            "its host is an IPv4 address in another written form: write it as four decimal \
             numbers",
        ));
    }
    Ok(host)
}

/// Whether a host's last label is a number as the URL standard reads one
/// (all digits, or `0x` and hex digits), which makes the whole host an
/// IPv4 address.
fn ends_in_a_number(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    let last = host.rsplit('.').next().unwrap_or_default();
    !last.is_empty()
        && (last.bytes().all(|byte| byte.is_ascii_digit())
            || last
                .strip_prefix("0x")
                .is_some_and(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit())))
}

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
                    call_left: time_left,
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
            let resolved = allowed(host, capabilities, &path, Access::Read).await?;
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
            let resolved = allowed(host, capabilities, &path, Access::Write).await?;
            host.write(resolved, text).await?;
            Ok(Json::Null)
        }
        HostCall::List { path, glob } => {
            let resolved = allowed(host, capabilities, &path, Access::Read).await?;
            let gate = ManifestGate {
                capabilities: capabilities.clone(),
            };
            Ok(listing_json(host.list(resolved, glob, gate).await?))
        }
        HostCall::Search {
            pattern,
            path,
            glob,
        } => {
            let resolved = allowed(host, capabilities, &path, Access::Read).await?;
            let gate = ManifestGate {
                capabilities: capabilities.clone(),
            };
            Ok(listing_json(
                host.search(pattern, resolved, glob, gate).await?,
            ))
        }
        HostCall::Fetch(FetchCall {
            url,
            method,
            headers,
            body,
            timeout_ms,
        }) => {
            let host_name = url_host(&url)?;
            capabilities.host(&host_name)?;
            // Only the host itself, when the manifest names it literally,
            // may be a loopback or a private address: never a name a `*` or
            // a domain's pattern matched, whatever else the manifest names.
            let literal = if capabilities.names_host_literally(&host_name) {
                vec![host_name.clone()]
            } else {
                Vec::new()
            };
            let wanted = timeout_ms
                .map_or(FETCH_MAX_TIMEOUT, Duration::from_millis)
                .min(FETCH_MAX_TIMEOUT);
            let output = host
                .fetch(
                    FetchRequest {
                        method,
                        url,
                        headers,
                        body: body.map(String::into_bytes).unwrap_or_default(),
                        timeout: wanted.min(time_left),
                    },
                    host_name,
                    literal,
                )
                .await?;
            Ok(serde_json::json!({
                "status": output.status,
                "headers": output.headers.iter().map(|(name, value)| [name, value]).collect::<Vec<_>>(),
                "body": String::from_utf8_lossy(&output.body),
                "cut": output.cut,
            }))
        }
        HostCall::Ask { question, choices } => {
            if phase == Phase::Hook {
                return Err(HostError::NotHere(
                    "a hook cannot put a question: it decides by its answer (`ask`)".to_owned(),
                ));
            }
            if choices.len() > ASK_MAX_CHOICES {
                return Err(HostError::Failed(format!(
                    "a question offers at most {ASK_MAX_CHOICES} choices"
                )));
            }
            let choices: Vec<String> = choices
                .iter()
                .map(|choice| shown(choice, ASK_CHOICE_MAX))
                .collect();
            let answer = host.ask(shown(&question, ASK_MAX), choices.clone()).await?;
            // An answer is one of the choices offered, when there were
            // choices: anything else is no answer.
            let answer = answer.filter(|answer| choices.is_empty() || choices.contains(answer));
            Ok(answer.map_or(Json::Null, Json::String))
        }
        HostCall::Notify(text) => {
            host.notify(shown(&text, NOTIFY_MAX));
            Ok(Json::Null)
        }
        HostCall::Progress(text) => {
            host.progress(shown(&text, PROGRESS_MAX));
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
            host.log(shown(&line, LOG_MAX));
            Ok(Json::Null)
        }
    }
}

/// Reading or writing, for [`allowed`].
#[derive(Clone, Copy)]
enum Access {
    Read,
    Write,
}

/// Where `path` leads, when the manifest lets the extension read or write
/// there.
///
/// A refusal names the path as the script gave it, and is the same however
/// the path failed: where it leads outside the workspace, and whether it
/// leads anywhere, are not the script's to learn. A manifest that grants
/// the access nowhere refuses before the path is resolved at all (the
/// review of step 28: a script with `read = []` could learn where a link
/// led, and what kind of entry a path outside was, from the refusal and
/// the resolver's errors).
async fn allowed<H: Host + ?Sized>(
    host: &H,
    capabilities: &Capabilities,
    path: &str,
    access: Access,
) -> Result<ResolvedPath, HostError> {
    let (capability, doing) = match access {
        Access::Read => ("read", "reading"),
        Access::Write => ("write", "writing"),
    };
    let refused = || {
        HostError::Refused(Refusal {
            capability,
            detail: format!("{doing} `{}`", cineris_base::visible(path)),
        })
    };
    let grants = match access {
        Access::Read => capabilities.grants_reading(),
        Access::Write => capabilities.grants_writing(),
    };
    if !grants {
        return Err(refused());
    }
    let resolved = host.resolve(path).await.map_err(|_| refused())?;
    let checked = match access {
        Access::Read => capabilities.read(&resolved),
        Access::Write => capabilities.write(&resolved),
    };
    checked.map_err(|_| refused())?;
    Ok(resolved)
}

/// A listing as the script is given it: the tool's text, and its payload.
fn listing_json(listing: Listing) -> Json {
    serde_json::json!({ "text": listing.text, "details": listing.payload })
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

/// `text` as the user or the log is shown it: cut to at most `most` bytes,
/// at a character's edge, with every character that acts unseen (a
/// control, a line break, a format character) written as an escape, so a
/// script's text cannot draw what the harness draws.
fn shown(text: &str, most: usize) -> String {
    cineris_base::visible(cineris_base::text::truncate_bytes(text, most)).into_owned()
}
