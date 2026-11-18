//! The core: operations in, events out
//! (`docs/architecture/00-overview.md`, `docs/architecture/11-rpc-protocol.md`).
//!
//! A [`Core`] holds the sessions of one process. A front end connects
//! ([`Core::connect`]), sends operations ([`Core::handle`]) and reads what
//! comes back from its queue: the events of the sessions it has open, and
//! requests it is to answer ([`Core::respond`]). The terminal UI, headless
//! mode and the RPC server are all front ends in this sense, and none of
//! them reaches a session any other way.
//!
//! **Where it runs.** On a Tokio runtime, which the caller provides: the
//! core thread's current-thread runtime (`docs/architecture/02-runtime.md`).
//! A session's loop is one task on it, and a shell command is started from
//! that thread. Every method of [`Core`] may be called from any thread;
//! the ones that start or resume a session must be called inside the
//! runtime.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the blocking calls here are inside closures run on the blocking pool"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use cineris_agent::{Session, SessionHandle};
use cineris_base::{
    CancelToken, IdGenerator, Secret, SharedClock, SharedTimer, SystemClock, SystemRng,
};
use cineris_config::{Env, LoadOptions, Loaded, Override, StateDir};
use cineris_llm::{ImageSource, Protocol, SpeedCooldowns, Transport};
use cineris_protocol::error::code;
use cineris_protocol::event::{Actor, Event, PlanUpdated, RequestResolved, Resolution};
use cineris_protocol::item::NoticeSource;
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::lease::{Lease, Level, Rule};
use cineris_protocol::methods::{
    ApprovalScope, ClientRole, ConfigGetResult, Empty, ExtensionsStatusResult, InitializeResult,
    McpStatusResult, ModelsListResult, Op, OpResult, PROTOCOL_VERSION, PeerInfo,
    PermissionsListResult, PlanResult, QuestionRequest, QuestionResponse,
    SUPPORTED_PROTOCOL_VERSIONS, ServerCapabilities, ServerRequest, ServerResponse,
    SessionForkResult, SessionListResult, SessionReadResult, SessionResult, TrustRequest,
    TrustResponse, UsageGetResult, WorkspaceFilesResult, negotiate_version,
};
use cineris_protocol::model::ModelRole;
use cineris_protocol::session::{SandboxStatus, SessionSettings, SessionSummary};
use cineris_protocol::{ErrorData, ErrorKind, LeaseId, SessionId};
use cineris_session::entry::{LeaseEvent, LeaseRecord};
use cineris_session::{Body, Git, Restore, SessionsRoot};
use cineris_tools::PlanBoard;
use cineris_tools::process::Runner;
use tokio::task::JoinHandle;

use crate::assemble::{self, Assembled, EngineTemplate, TrustAnswer, Wanted, Which};
use crate::clients::{ClientId, ClientInfo, Clients};
use crate::events::{EventQueue, SessionSink};
use crate::policy::{EnginePolicy, PermissionDenial};
use crate::provider::{self, Connector, Providers};
use crate::store::{Hooks, SharedLog};

/// What a change of model, reasoning level or speed costs once, said to
/// the user when it is made in a session that has history.
const UNCACHED: &str = "The next request sends the whole conversation again without the \
                        provider's cached prefix: it costs more and takes longer, once.";

/// The most paths `workspace/files` gives: past them a front end's
/// completion is told the list was cut.
const WORKSPACE_FILES: usize = 100_000;

/// The answers of the question an undo or a rewind asks about files edited
/// since the harness left them.
const PUT_BACK: &str = "Put them back";
const KEEP: &str = "Keep them as they are";

/// Where a rewind goes back to.
#[derive(Debug, Clone)]
enum Back {
    /// To a point named by an item: `session/rewind`.
    To(crate::rewind::Point),
    /// To before the last turn: `session/undo`.
    Undo,
}

/// How long the end of a session waits for the capture of the workspace's
/// starting state, when that has not finished.
const BASE_WAIT: Duration = Duration::from_secs(2);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Each change behind these locks is whole before the lock is let go.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where a value of the configuration came from, as the protocol says it:
/// `None` for a built-in default.
fn config_origin(
    origin: &cineris_config::Origin,
    env: &Env,
) -> Option<cineris_protocol::methods::ConfigOrigin> {
    use cineris_config::LayerKind;
    use cineris_protocol::methods::ConfigLayer;
    let layer = match origin.layer {
        LayerKind::Default => return None,
        LayerKind::Managed => ConfigLayer::Managed,
        LayerKind::User => ConfigLayer::User,
        LayerKind::Project => ConfigLayer::Project,
        LayerKind::ProjectLocal => ConfigLayer::ProjectLocal,
        LayerKind::Profile => ConfigLayer::Profile,
        LayerKind::Environment => ConfigLayer::Environment,
        LayerKind::CommandLine => ConfigLayer::CommandLine,
    };
    Some(cineris_protocol::methods::ConfigOrigin {
        layer,
        file: origin.file.as_deref().map(|file| env.display_path(file)),
        line: origin.line,
    })
}

pub(crate) fn error(kind: ErrorKind, code: &str, message: impl Into<String>) -> Box<ErrorData> {
    Box::new(ErrorData::new(kind, code, message))
}

/// What an operation ends with when it does not succeed.
pub type OpError = Box<ErrorData>;

/// What a core is started with.
pub struct CoreOptions {
    /// The environment: where configuration and credentials are looked
    /// for. The binary builds it from the process's own.
    pub env: Env,
    /// Where configuration, state and data are, in place of where `env`
    /// says: a test's, which must not read the machine's own files.
    pub dirs: Option<cineris_config::Dirs>,
    /// The time of everything.
    pub clock: SharedClock,
    /// Every wait: retries, time limits, the stages of stopping a command,
    /// an approval's time limit.
    pub timer: SharedTimer,
    /// The version of Cineris, recorded in every session it creates.
    pub harness_version: String,
    /// The profile to load configuration with (`--profile`).
    pub profile: Option<String>,
    /// Text added to the end of every session's system prompt
    /// (`--system-append`). The user's own, for this run.
    pub system_append: Option<String>,
    /// Settings from the command line (`-c key=value`, and flags such as
    /// `--model`, `--effort`, `--speed` and `--permission-mode` as their
    /// keys). They rank above every file and are the user's own, so
    /// nothing in them waits for trust.
    pub overrides: Vec<Override>,
    /// Permission rules from the command line (`--allow`, `--deny`).
    pub rules: Vec<Rule>,
    /// Treat every project as trusted for this run (`--trust-project`).
    pub trust_project: bool,
    /// End the turn at the first call that needs an answer nobody can give
    /// (`--fail-on-denial`). Without it such a call is refused and the run
    /// goes on. Either way the refusal is kept
    /// ([`Core::permission_denials`]).
    pub fail_on_denial: bool,
    /// How long an approval request waits for its answer before it is a
    /// denial. `None` waits as long as it takes, which is what a person at
    /// a terminal wants.
    pub approval_timeout: Option<Duration>,
    /// The guard of process trees: the program to start (the harness's own
    /// executable) and its arguments (the hidden subcommand whose body is
    /// `cineris_sandbox::tree::run_guard`). `None`: commands run without a
    /// guard, and can outlive a harness that is killed.
    pub guard: Option<(PathBuf, Vec<OsString>)>,
    /// The sandbox launcher: the program to start (the harness's own
    /// executable) and its arguments (the hidden subcommand whose body is
    /// `cineris_sandbox::launch::run`). Every shell command is started
    /// through it where the sandbox enforces. `None`: nothing is confined,
    /// and every session reports the sandbox off.
    pub launcher: Option<(PathBuf, Vec<OsString>)>,
    /// What the system offers the sandbox, in place of asking it
    /// (`cineris_sandbox::support::detect`): a test's, which says the kernel
    /// has no Landlock, or an old one.
    pub sandbox: Option<cineris_sandbox::support::Support>,
    /// How far the look at protected paths around a confined command goes,
    /// in place of its standard bounds: a test's, which reaches them.
    pub protect_limits: Option<cineris_sandbox::protect::Limits>,
    /// API keys given on the command line, by provider. Looked at before
    /// the environment and the credential store.
    pub api_keys: BTreeMap<String, Secret>,
    /// Told every provider key the core finds, once each: the place to
    /// give it to the log's own filter
    /// (`cineris_base::log::LogHandle::add_secret`), which was started
    /// before any key was known.
    pub key_found: Option<provider::KeyFound>,
    /// Makes model clients in place of the built-in way: a test's scripted
    /// provider.
    pub connector: Option<Arc<dyn Connector>>,
    /// What requests are sent through. `None`: the HTTP transport, which
    /// costs nothing until the first request.
    pub transport: Option<Arc<dyn Transport>>,
    /// Protocols beside the built-in ones, by name: a test's.
    pub protocols: BTreeMap<String, Arc<dyn Protocol>>,
    /// The environment shell commands inherit from, in place of the
    /// process's own: a test's.
    pub shell_env: Option<Vec<(OsString, OsString)>>,
    /// The process runner shell commands go through, in place of one on
    /// this core's timer and clock with the guard above: a test's.
    pub runner: Option<Runner>,
    /// The shell commands run in, in place of the one that is looked for
    /// and asked its version: a test's, which starts no process to find
    /// out what the model's tool description says.
    pub shell: Option<cineris_tools::Shell>,
    /// How git is run for the base state and for a file's earlier content,
    /// in place of the system's git: a test's. `None`: the system's git,
    /// started inside the session's sandbox, reading only, while commands
    /// are confined.
    pub git: Option<Arc<dyn Git>>,
    /// Whether a new session records the workspace's starting state.
    pub base_capture: bool,
    /// Whether a session has the repository index: the map of its context
    /// epochs and the `outline` tool's outlines.
    pub repo_index: bool,
    /// How long the first request of an epoch waits for the repository map
    /// when `index.map_wait` is not set: 150 ms for the interactive app,
    /// none for a headless run.
    pub map_wait: Duration,
    /// What the repository index may read, in place of the permission
    /// engine's gate: a test's, which watches when the index is first used
    /// (`cineris_testkit::Untouched`).
    pub index_gate: Option<Arc<dyn cineris_index::Gate>>,
    /// Called as each pass of the repository index starts, on the thread
    /// that starts it: a test's, which watches whether the first starts
    /// before the session's loop runs (`cineris_testkit::Untouched::note`).
    pub index_started: Option<crate::repo_map::Probe>,
    /// Called once a session's loop runs, just before its background work
    /// starts: a test's, which marks the end of the path that opens the
    /// session (`cineris_testkit::Untouched::open`).
    pub session_running: Option<crate::repo_map::Probe>,
    /// Called each time a session looks at or reads the files of its
    /// project's knowledge, on the thread that does: a test's, which
    /// watches that none is touched before the session's loop runs
    /// (`cineris_testkit::Untouched::note`).
    pub knowledge_read: Option<crate::repo_map::Probe>,
    /// Cancelled when the process is to stop: every session's token is a
    /// child of it.
    pub cancel: CancelToken,
    /// The time zone the statistics count local days in, in place of the
    /// system's: a test's. `None`: the system's, read by the statistics'
    /// writer when it first writes, and by the usage report.
    pub zone: Option<cineris_base::Zone>,
    /// The user's configuration, loaded by the caller with this core's
    /// `env`, `dirs`, `profile` and `overrides`, and no workspace: used in
    /// place of loading it again. The interactive application reads its
    /// own settings from it before the terminal is entered (step 23).
    pub loaded: Option<cineris_config::Loaded>,
    /// The process's log, whose level a reload of `log.level` changes when
    /// neither `--log-level` nor `CINERIS_LOG` set it (step 24). `None`: a
    /// reload leaves the log as it is.
    pub log: Option<Arc<cineris_base::log::LogHandle>>,
    /// Whether an answer to a question the core puts may stop the turn:
    /// the interactive application's, whose interrupt key stops it from a
    /// dialog (`docs/architecture/03-agent-loop.md`, "Approve"). With it,
    /// no call with side effects that the model wrote after an asked call
    /// starts while the question is open.
    pub answers_may_stop: bool,
    /// Keep a session running when its last client goes away: what
    /// `cineris serve --socket` does, so that a client can come back to it
    /// (`docs/architecture/11-rpc-protocol.md`, "Persistent sessions"). A
    /// disconnect then ends no session, and an approval asked in a session
    /// with no client at all waits for a `user` client to open it, for the
    /// approval's time limit, where it would be refused at once as one
    /// nobody can be asked. A session whose clients are there and answer no
    /// approvals still gets the default at once. Off by default: headless
    /// mode, the terminal UI and `serve --stdio` end a session with its
    /// client.
    pub keep_sessions: bool,
    /// Whether a session may be at `unrestricted`: started there (by its
    /// settings, or by a configuration override however it is spelled, as
    /// the configuration reads it) or raised there by `session/set`. On by
    /// default, so every front end behaves as it did: a person at the
    /// terminal or a headless run chooses its own level. A server turns it
    /// off unless its own command line or configuration is at
    /// `unrestricted`, so that a client is never more than the user at the
    /// terminal (`docs/security.md`, section 5): a session that would be
    /// there is refused `unrestricted_not_allowed`.
    pub allow_unrestricted: bool,
}

impl std::fmt::Debug for CoreOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreOptions")
            .field("env", &self.env)
            .field("harness_version", &self.harness_version)
            .field("profile", &self.profile)
            .field("trust_project", &self.trust_project)
            .field("fail_on_denial", &self.fail_on_denial)
            .field("approval_timeout", &self.approval_timeout)
            .field("keep_sessions", &self.keep_sessions)
            .field("allow_unrestricted", &self.allow_unrestricted)
            .finish_non_exhaustive()
    }
}

impl CoreOptions {
    /// The options of a core in `env`, on `timer`, with the system's clock
    /// and nothing from a command line.
    #[must_use]
    pub fn new(env: Env, timer: SharedTimer) -> Self {
        Self {
            env,
            dirs: None,
            clock: Arc::new(SystemClock::new()),
            timer,
            harness_version: env!("CARGO_PKG_VERSION").to_owned(),
            profile: None,
            system_append: None,
            overrides: Vec::new(),
            rules: Vec::new(),
            trust_project: false,
            fail_on_denial: false,
            approval_timeout: None,
            guard: None,
            launcher: None,
            sandbox: None,
            protect_limits: None,
            api_keys: BTreeMap::new(),
            key_found: None,
            connector: None,
            transport: None,
            protocols: BTreeMap::new(),
            shell_env: None,
            runner: None,
            shell: None,
            git: None,
            base_capture: true,
            repo_index: true,
            map_wait: Duration::from_millis(150),
            index_gate: None,
            index_started: None,
            session_running: None,
            knowledge_read: None,
            cancel: CancelToken::new(),
            zone: None,
            loaded: None,
            log: None,
            answers_may_stop: false,
            keep_sessions: false,
            allow_unrestricted: true,
        }
    }
}

/// A session that is open.
pub(crate) struct Live {
    pub(crate) handle: SessionHandle,
    task: Mutex<Option<JoinHandle<()>>>,
    pub(crate) sink: Arc<SessionSink>,
    pub(crate) log: SharedLog,
    pub(crate) hooks: Arc<Hooks>,
    pub(crate) policy: Arc<EnginePolicy>,
    pub(crate) board: Arc<PlanBoard>,
    /// The session's sandbox: its launcher and its temporary directory.
    pub(crate) sandbox: Arc<crate::sandbox::SessionSandbox>,
    /// Shared with the session's policy, which judges a supervisor's grant
    /// with it.
    pub(crate) template: Arc<Mutex<EngineTemplate>>,
    pub(crate) images: Arc<dyn ImageSource>,
    /// The session's configuration: what is in force, and what a reload
    /// works from (step 24). Locked only to read or commit.
    pub(crate) config: Mutex<crate::reload::SessionConfig>,
    /// The files the configuration was read from, stamped: locked only by a
    /// reload, across its reads.
    pub(crate) watched: Mutex<crate::reload::Watched>,
    /// One reload of the session at a time, from its look to its commit.
    pub(crate) reloading: tokio::sync::Mutex<()>,
    /// The accounts the ledger draws on, the lease's first.
    pub(crate) accounts: Mutex<Vec<cineris_agent::Account>>,
    /// The session's verifier, for `verify/run`, whose settings a reload
    /// changes.
    pub(crate) verifier: Arc<crate::verify::CoreVerifier>,
    /// What of the shell a reload changes.
    pub(crate) shell_live: Arc<cineris_tools::ShellLive>,
    /// Whether git may read the repository's own configuration.
    pub(crate) repository: Arc<AtomicBool>,
    /// The repository index, whose map's budget a reload changes.
    pub(crate) repo: Option<crate::repo_map::RepoIndex>,
    /// The session's store of versions and artifacts.
    pub(crate) content: Arc<dyn cineris_tools::ContentStore>,
    lease_count: AtomicU64,
    /// The project's knowledge as this session has it.
    pub(crate) knowledge: Option<Arc<crate::knowledge::Knowledge>>,
}

impl Inner {
    /// The one gate of `unrestricted`: whether a session may hold `level`
    /// in this core ([`CoreOptions::allow_unrestricted`]). Every way a
    /// session's level is set passes it, on the configuration as it comes
    /// to: a session put together (`session/new`, a stored session's
    /// `session/open`, `session/fork`, whose fork is written only after it,
    /// in `assemble`); a raise by `session/set`, before anything of it is
    /// made ready; and a `config/reload` (with or without a profile,
    /// explicit or the look at a turn's start), which leaves a refused
    /// level out and applies the rest. [`Core::put_engine`] asks it again,
    /// for a way not written yet.
    ///
    /// # Errors
    ///
    /// `unrestricted_not_allowed`, of kind `permission`.
    pub(crate) fn may_hold(&self, level: &Level) -> Result<(), OpError> {
        if *level == Level::Unrestricted && !self.options.allow_unrestricted {
            return Err(unrestricted_not_allowed());
        }
        Ok(())
    }

    /// The configuration with no project's layers, as it is in force.
    pub(crate) fn user(&self) -> Arc<Loaded> {
        lock(&self.user_config).in_force()
    }

    /// What the sessions of a project share of its knowledge, made the
    /// first time one of them is put together. Touches no file.
    pub(crate) fn knowledge_of(
        &self,
        project: &cineris_protocol::ids::ProjectKey,
    ) -> Arc<crate::knowledge::Shared> {
        let mut known = lock(&self.knowledge);
        // What no session holds any more is let go.
        known.retain(|_, shared| shared.strong_count() > 0);
        if let Some(shared) = known.get(project).and_then(std::sync::Weak::upgrade) {
            return shared;
        }
        let shared = Arc::new(crate::knowledge::Shared::default());
        known.insert(project.clone(), Arc::downgrade(&shared));
        shared
    }
}

impl Live {
    /// The configuration in force.
    pub(crate) fn loaded(&self) -> Arc<Loaded> {
        lock(&self.config).in_force()
    }
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live")
            .field("session", self.handle.id())
            .finish_non_exhaustive()
    }
}

/// What a core is, behind its handle.
pub(crate) struct Inner {
    pub(crate) options: CoreOptions,
    pub(crate) clients: Arc<Clients>,
    pub(crate) providers: Providers,
    /// The configuration with no project's layers: what `config/get`
    /// answers with when it is asked about no session, and what the usage
    /// reports and the statistics read. A reload changes its `Now` keys
    /// (step 24).
    pub(crate) user_config: Mutex<crate::reload::CoreConfig>,
    /// The aggregates the last usage report read, with the file they are
    /// of (step 24).
    pub(crate) aggregates_cache: Mutex<Option<crate::usage::AggregatesCache>>,
    /// The files that configuration was read from: locked only by its
    /// reload, across the reads it makes.
    pub(crate) user_watched: Mutex<crate::reload::CoreWatched>,
    /// One reload of the core's own configuration at a time.
    pub(crate) core_reloading: tokio::sync::Mutex<()>,
    ids: IdGenerator,
    sessions: Mutex<BTreeMap<SessionId, Arc<Live>>>,
    /// What the sessions of each project share of its knowledge: the two
    /// files as they were last read (`crate::knowledge`).
    knowledge: Mutex<
        BTreeMap<cineris_protocol::ids::ProjectKey, std::sync::Weak<crate::knowledge::Shared>>,
    >,
    /// The shells found so far, by the program and arguments configured:
    /// a shell is asked its version once in the life of a core.
    pub(crate) shells: Mutex<BTreeMap<ShellKey, cineris_tools::Shell>>,
    /// The threads asking a shell its version, to keep the answer for the
    /// runs after this one ([`crate::shell_cache`]). [`Core::shutdown`]
    /// waits for them, so that a short run still keeps it.
    pub(crate) probes: Mutex<Vec<std::thread::JoinHandle<()>>>,
    /// Set when [`Core::shutdown`] begins: no session starts after it.
    shutting_down: AtomicBool,
    /// What the system offers the sandbox, asked once.
    pub(crate) sandbox: crate::sandbox::CoreSandbox,
    /// The statistics' recorder: `None` when statistics are off
    /// (`stats.enabled = false`) or there is no state directory. Making it
    /// touches no file; its writer is started when the first session opens.
    pub(crate) stats: Option<cineris_stats::Recorder>,
    /// The account of today's spending limits (`[limits.day]`), shared by
    /// every session of the core, and the local day it is for: made the
    /// first time a session with a day limit opens, from what the day's
    /// statistics already hold.
    pub(crate) day: crate::day::DaySlot,
    /// The projects a person answered "not now" to: not asked about again
    /// in this core's life, and nothing stored (since step 23).
    pub(crate) not_now: Mutex<std::collections::HashSet<cineris_protocol::ProjectKey>>,
}

/// What a shell was looked for with: `shell.program` and `shell.args`.
pub(crate) type ShellKey = (Option<String>, Option<Vec<String>>);

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("sessions", &lock(&self.sessions).len())
            .finish_non_exhaustive()
    }
}

/// A front end's connection: its number, and the queue everything for it
/// arrives in.
#[derive(Debug, Clone)]
pub struct Client {
    /// The client's number, which every operation names.
    pub id: ClientId,
    /// What the core sends it: the events of the sessions it has open, and
    /// requests to answer.
    pub queue: Arc<EventQueue>,
}

/// The core. Clones are the same core.
#[derive(Debug, Clone)]
pub struct Core {
    pub(crate) inner: Arc<Inner>,
}

impl Core {
    /// Starts a core: reads the user's configuration, the model catalog
    /// with the user's entries, and opens the credential store. No session
    /// is started and no network connection is made.
    ///
    /// # Errors
    ///
    /// When the user's configuration is invalid, or an entry of the user's
    /// model catalog cannot be read: the error names the file, the entry
    /// and what is wrong with it, and nothing is started.
    pub fn start(mut options: CoreOptions) -> Result<Self, OpError> {
        let mut load = LoadOptions::new(options.env.clone());
        if let Some(dirs) = &options.dirs {
            load.dirs = dirs.clone();
        }
        load.profile.clone_from(&options.profile);
        load.overrides.clone_from(&options.overrides);
        let user = match options.loaded.take() {
            Some(loaded) => loaded,
            None => cineris_config::load(&load).map_err(|failure| {
                error(
                    ErrorKind::Config,
                    "invalid_configuration",
                    failure.render(&options.env),
                )
            })?,
        };
        let catalog = provider::catalog(&user, &options.env)?;
        let store = cineris_config::open_store(
            user.settings.credentials.store,
            &user.dirs,
            Arc::clone(&options.clock),
        )
        .unwrap_or_else(|failure| {
            tracing::warn!(error = %failure, "the credential store could not be opened");
            Box::new(provider::Unopened(failure.to_string()))
        });
        let mut protocols = provider::builtin_protocols();
        protocols.extend(options.protocols.clone());
        let transport: Arc<dyn Transport> = match &options.transport {
            Some(transport) => Arc::clone(transport),
            None => Arc::new(cineris_llm::HttpTransport::default()),
        };
        let providers = Providers {
            catalog,
            cooldowns: SpeedCooldowns::new(Arc::clone(&options.clock)),
            transport,
            clock: Arc::clone(&options.clock),
            timer: Arc::clone(&options.timer),
            protocols,
            connector: options.connector.clone(),
            store,
            flags: options.api_keys.clone(),
            filter: Arc::new(std::sync::RwLock::new(cineris_base::SecretFilter::new())),
            key_found: options.key_found.clone(),
        };
        let stats = crate::usage::recorder(&user, &options);
        let ids = IdGenerator::new(Arc::clone(&options.clock), Box::new(SystemRng::new()));
        let sandbox = crate::sandbox::CoreSandbox::new(
            options
                .sandbox
                .unwrap_or_else(cineris_sandbox::support::detect),
            options.launcher.clone(),
        );
        let clock = Arc::clone(&options.clock);
        Ok(Self {
            inner: Arc::new(Inner {
                options,
                clients: Arc::new(Clients::default()),
                providers,
                user_config: Mutex::new(crate::reload::CoreConfig::new(user)),
                aggregates_cache: Mutex::new(None),
                user_watched: Mutex::new(crate::reload::CoreWatched::new(load, clock)),
                core_reloading: tokio::sync::Mutex::new(()),
                ids,
                sessions: Mutex::new(BTreeMap::new()),
                knowledge: Mutex::new(BTreeMap::new()),
                shells: Mutex::new(BTreeMap::new()),
                probes: Mutex::new(Vec::new()),
                shutting_down: AtomicBool::new(false),
                sandbox,
                stats,
                day: Mutex::new(None),
                not_now: Mutex::new(std::collections::HashSet::new()),
            }),
        })
    }

    /// Connects a front end.
    #[must_use]
    pub fn connect(&self, info: ClientInfo) -> Client {
        self.connect_with_grant(info, None)
    }

    /// Connects a front end, and gives a `supervisor` its grant: what it
    /// may approve in the sessions it opens
    /// (`docs/architecture/17-coordinator.md`, "Approval routing"). With
    /// one, a supervisor that answers approvals is put each approval of its
    /// session at once with the users, and its allow is taken where the
    /// grant, held inside the session's own scope, allows the call
    /// (`docs/architecture/06-permissions-and-sandbox.md`, "When a session
    /// is supervised"). With none, or for any other role, it is
    /// [`connect`](Self::connect): a supervisor is then asked nothing and
    /// may answer nothing. Nothing on the wire gives a grant: a server
    /// gives it from its own options, and the steps of task and team
    /// leases decide where a real one comes from.
    #[must_use]
    pub fn connect_with_grant(&self, info: ClientInfo, grant: Option<Lease>) -> Client {
        let (id, queue) = self.inner.clients.connect(info, grant);
        Client { id, queue }
    }

    /// Disconnects a front end: it has no session open any more, and its
    /// queue takes nothing more. Sessions it alone had open are closed,
    /// unless the core keeps sessions ([`CoreOptions::keep_sessions`]), or
    /// it is an `observer`, which never ends a session (since step 27).
    ///
    /// The client is taken out first, and the sessions looked at after: a
    /// session that is put together for it meanwhile either is in the table
    /// by then, and is closed here, or finds the client gone when it is
    /// put in the table, and closes itself.
    pub async fn disconnect(&self, client: ClientId) {
        // Whether it may end a session, asked while it is still known.
        let drives = self.may_drive(client).is_ok();
        self.inner.clients.disconnect(client);
        let keep = self.inner.options.keep_sessions || !drives;
        let sessions: Vec<Arc<Live>> = lock(&self.inner.sessions).values().cloned().collect();
        for live in sessions {
            if live.sink.unsubscribe(client.0) && !keep && live.sink.subscribers().is_empty() {
                self.end(live.handle.id()).await;
            }
        }
    }

    /// The sessions this core has open, by id, in id order: what a server's
    /// status shows, and what is ended at [`shutdown`](Self::shutdown).
    #[must_use]
    pub fn open_sessions(&self) -> Vec<SessionId> {
        lock(&self.inner.sessions).keys().cloned().collect()
    }

    /// The live usage of an open session.
    pub(crate) fn session_usage(
        &self,
        session: &SessionId,
    ) -> Option<cineris_protocol::usage::SessionUsage> {
        self.live(session).ok().map(|live| live.handle.usage())
    }

    /// The calls of a session that were refused because nobody could be
    /// asked about them: a headless result's `permission_denials`.
    #[must_use]
    pub fn permission_denials(&self, session: &SessionId) -> Vec<PermissionDenial> {
        self.live(session)
            .map(|live| live.policy.denials())
            .unwrap_or_default()
    }

    /// Takes a client's answer to a request the core put to it: an
    /// approval, a question or trust (since step 23).
    ///
    /// # Errors
    ///
    /// `role_not_allowed` when the client is not a `user` client or was not
    /// asked; an error of kind `usage` when the request is not open any
    /// more, or the answer is not one to that kind of request.
    pub fn respond(
        &self,
        client: ClientId,
        request: &RequestId,
        response: ServerResponse,
    ) -> Result<(), OpError> {
        self.inner.clients.answer(client, request, response)?;
        Ok(())
    }

    fn live(&self, session: &SessionId) -> Result<Arc<Live>, OpError> {
        lock(&self.inner.sessions)
            .get(session)
            .cloned()
            .ok_or_else(|| {
                error(
                    ErrorKind::Usage,
                    code::SESSION_NOT_OPEN,
                    format!("The session `{session}` is not open."),
                )
            })
    }

    /// The session, when `client` has it open.
    pub(crate) fn open_for(
        &self,
        client: ClientId,
        session: &SessionId,
    ) -> Result<Arc<Live>, OpError> {
        let live = self.live(session)?;
        if live.sink.subscribers().contains(&client.0) {
            Ok(live)
        } else {
            Err(error(
                ErrorKind::Usage,
                code::SESSION_NOT_OPEN,
                format!("The session `{session}` is not open on this connection."),
            ))
        }
    }

    /// Runs blocking work on the blocking pool, through the one seam the
    /// loop's blocking work goes through too (`cineris_tools::run_blocking`):
    /// a test that orders the loop's work orders the core's as well
    /// (`docs/testing-and-ci.md`, "A seeded order of events"). The work is
    /// not cancelled: what the core starts on the pool, it finishes.
    pub(crate) async fn blocking<T: Send + 'static>(
        work: impl FnOnce() -> Result<T, OpError> + Send + 'static,
    ) -> Result<T, OpError> {
        match cineris_tools::run_blocking(&CancelToken::new(), move |_| work()).await {
            Ok(done) => done,
            Err(cineris_tools::BlockingError::Panicked(panic)) => Err(error(
                ErrorKind::Internal,
                "task_failed",
                format!("internal error: the work panicked part-way ({panic})."),
            )),
            Err(cineris_tools::BlockingError::Cancelled) => Err(error(
                ErrorKind::Internal,
                "task_failed",
                "The work was abandoned part-way.",
            )),
        }
    }

    fn not_connected() -> OpError {
        error(
            ErrorKind::Usage,
            code::NOT_INITIALIZED,
            "This client is not connected.",
        )
    }

    fn shutting_down() -> OpError {
        error(
            ErrorKind::Usage,
            code::SHUTTING_DOWN,
            "The server is shutting down, and starts no session.",
        )
    }

    /// Keeps a session that `client` was just put on, if the client is
    /// still there to have it.
    ///
    /// Called after the client is subscribed and the session is in the
    /// table, which is the order [`disconnect`](Self::disconnect) and
    /// [`shutdown`](Self::shutdown) look in the other way round: a client
    /// that left, or a shutdown that began, before this look is seen here,
    /// and one after it sees the session. A session nobody else has open is
    /// then ended, so it never runs with no client: one `put_together` for
    /// the client always, and one it joined while it ran unless the core
    /// keeps sessions, where it ran with no client before and goes on so.
    async fn still_wanted(
        &self,
        client: ClientId,
        live: &Arc<Live>,
        put_together: bool,
    ) -> Result<(), OpError> {
        let gone = self.inner.clients.queue(client).is_none();
        let stopping = self.inner.shutting_down.load(Ordering::SeqCst);
        if !gone && !stopping {
            return Ok(());
        }
        live.sink.unsubscribe(client.0);
        let kept = self.inner.options.keep_sessions && !put_together;
        if stopping || (!kept && live.sink.subscribers().is_empty()) {
            self.end(live.handle.id()).await;
        }
        Err(if stopping {
            Self::shutting_down()
        } else {
            Self::not_connected()
        })
    }

    /// Puts a session together and starts its loop.
    async fn open(&self, client: ClientId, wanted: Wanted) -> Result<Arc<Live>, OpError> {
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return Err(Self::shutting_down());
        }
        let Some(queue) = self.inner.clients.queue(client) else {
            return Err(Self::not_connected());
        };
        let looked = self.trust_for(client, &wanted).await?;
        let inner = Arc::clone(&self.inner);
        let cancel = self.inner.options.cancel.child();
        let assembled =
            Self::blocking(move || assemble::assemble(&inner, wanted, cancel, looked)).await?;
        let Assembled {
            parts,
            sink,
            log,
            hooks,
            policy,
            board,
            sandbox,
            template,
            loaded,
            images,
            content,
            notices,
            lease_count,
            repo,
            knowledge,
            config,
            watched,
            verifier,
            accounts,
            shell_live,
            repository,
        } = assembled;
        // The client is listening before the session can say anything.
        sink.subscribe(client.0, queue);
        let (session, handle) = Session::open(parts)
            .await
            .map_err(|failure| Box::new(cineris_base::Classified::to_error_data(&failure)))?;
        hooks.bind(handle.clone());
        for notice in notices {
            handle.emit(notice);
        }
        // Everything the session's loop logs is inside its span.
        let span = tracing::info_span!("session", session = %handle.id());
        let task = tokio::spawn(tracing::Instrument::instrument(session.run(), span));
        if let Some(probe) = &self.inner.options.session_running {
            probe();
        }
        // The repository index's first pass, now that the loop runs: never
        // on the path that puts the session together. With no map there is
        // nothing to make ahead: the `outline` tool reads what it needs.
        if let Some(repo) = repo.as_ref().filter(|_| loaded.settings.index.map) {
            repo.warm();
        }
        self.probe_context(&handle, &loaded);
        let live = Arc::new(Live {
            handle,
            task: Mutex::new(Some(task)),
            sink,
            log,
            hooks,
            policy,
            board,
            sandbox,
            template,
            images,
            config: Mutex::new(config),
            watched: Mutex::new(watched),
            reloading: tokio::sync::Mutex::new(()),
            accounts: Mutex::new(accounts),
            verifier,
            shell_live,
            repository,
            repo: repo.clone(),
            content,
            lease_count: AtomicU64::new(lease_count),
            knowledge,
        });
        lock(&self.inner.sessions).insert(live.handle.id().clone(), Arc::clone(&live));
        // The client may have left, or a shutdown begun, while the session
        // was put together off this thread.
        self.still_wanted(client, &live, true).await?;
        self.tell_grant_limits(client, &live);
        Ok(live)
    }

    /// Tells the session what of a supervisor's grant does not apply in it,
    /// and why, when the supervisor opens it: a notice of the session's,
    /// which every client of it is sent. Nothing for another client, or a
    /// grant all of which applies.
    fn tell_grant_limits(&self, client: ClientId, live: &Arc<Live>) {
        if let Some(grant) = self.inner.clients.grant_of(client)
            && let Some(message) = live.policy.grant_limits(&grant)
        {
            live.hooks.tell(message);
        }
    }

    /// The trust a session is put together with, after asking the client
    /// that opens it, when it answers trust questions and the project has
    /// something to trust it with (`docs/steps/23/answered.md`, "The design,
    /// before building", point 3, with the owner's three answers). `None`:
    /// nobody is asked, and the assembly works trust out as it always has.
    ///
    /// The question waits for the person, and ends as no answer when the
    /// client goes away or the core is ending: the client then gets no
    /// session.
    async fn trust_for(
        &self,
        client: ClientId,
        wanted: &Wanted,
    ) -> Result<Option<assemble::Trust>, OpError> {
        if self.inner.options.trust_project
            || self
                .inner
                .clients
                .askable(&[client.0], crate::clients::Kind::Trust)
                .is_empty()
        {
            return Ok(None);
        }
        let inner = Arc::clone(&self.inner);
        let look = wanted.clone();
        let (trust, question) = Self::blocking(move || assemble::look(&inner, &look)).await?;
        let Some(question) = question else {
            return Ok(Some(trust));
        };
        let session = match &wanted.which {
            Which::New { id, .. } | Which::Resume { id } => id.clone(),
        };
        let request = TrustRequest {
            session_id: session.clone(),
            workspace: question.root.clone(),
            resources: question.resources.clone(),
            trusted: question.trusted,
        };
        let (id, answer) = self.inner.clients.ask_once(&session, &[client.0], |_| {
            ServerRequest::Trust(request.clone())
        });
        // A shutdown that began since the look (its withdrawal came before
        // this question was put) takes the question back at once: it sets
        // its mark before it withdraws, so one or the other sees this
        // question (the review of step 23's fixes).
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            self.inner.clients.close(&id);
        }
        let answered = self.inner.options.cancel.race(answer).await;
        let answered = answered.map(|got| got.map(|answer| answer.response));
        self.inner.clients.close(&id);
        let answer = match answered {
            // Only a yes and nothing else is a yes: an answer that says
            // both is refused where it is given, and would be read here as
            // not trusted.
            Some(Ok(ServerResponse::Trust(TrustResponse {
                trusted: true,
                never: false,
                keep: false,
            }))) => Some(TrustAnswer::Yes),
            // Never is an answer to a project not trusted, and keep to one
            // trusted already; the other way round it is refused where it
            // is given, and would be read here as not now.
            Some(Ok(ServerResponse::Trust(TrustResponse {
                trusted: false,
                never: true,
                keep: false,
            }))) if !question.trusted => Some(TrustAnswer::Never),
            Some(Ok(ServerResponse::Trust(TrustResponse {
                trusted: false,
                never: false,
                keep: true,
            }))) if question.trusted => Some(TrustAnswer::Keep),
            Some(Ok(_)) => Some(TrustAnswer::NotNow),
            // The client went away, or the core is ending.
            Some(Err(_)) | None => None,
        };
        if self.inner.options.cancel.is_cancelled()
            || self.inner.shutting_down.load(Ordering::SeqCst)
        {
            return Err(Self::shutting_down());
        }
        let Some(answer) = answer else {
            return Err(Self::not_connected());
        };
        let inner = Arc::clone(&self.inner);
        let wanted = wanted.clone();
        let trust = Self::blocking(move || {
            assemble::answer_trust(&inner, &wanted, &question, answer, trust)
        })
        .await?;
        Ok(Some(trust))
    }

    /// Asks a local server what context size it loaded the main role's
    /// model with, where the provider's entry names a probe
    /// (`docs/architecture/04-providers.md`, "A local server's loaded
    /// context size"). In the background: nothing waits for it, and until
    /// it answers the catalog's figure stands. An answer that cannot be
    /// used leaves the catalog's figure, and is logged.
    fn probe_context(&self, handle: &cineris_agent::SessionHandle, loaded: &Loaded) {
        let Some(main) = handle.role_model(&cineris_protocol::model::ModelRole::Main) else {
            return;
        };
        let info = &main.entry.info;
        let Ok(provider) = crate::provider::provider_info(
            &loaded.settings,
            &self.inner.options.env,
            info.provider.as_str(),
        ) else {
            return;
        };
        let request = match cineris_llm::openai_chat::context::request(&provider) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%error, "the context probe cannot be made");
                return;
            }
        };
        let transport = Arc::clone(&self.inner.providers.transport);
        let handle = handle.clone();
        let model = info.id.clone();
        let cancel = self.inner.options.cancel.child();
        tokio::spawn(async move {
            /// The most of an answer that is read.
            const LIMIT: usize = 4 * 1024 * 1024;
            let Some(sent) = cancel.race(transport.send(request, cancel.clone())).await else {
                return;
            };
            let response = match sent {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!(%error, "the context probe was not answered");
                    return;
                }
            };
            let status = response.status;
            let mut body = response.body;
            let mut bytes = Vec::new();
            while let Some(Some(chunk)) = cancel.race(body.next()).await {
                let Ok(chunk) = chunk else {
                    return;
                };
                bytes.extend_from_slice(&chunk);
                if bytes.len() > LIMIT {
                    tracing::warn!("the context probe's answer is too long to be read");
                    return;
                }
            }
            match cineris_llm::openai_chat::context::read(&provider, &model, status, &bytes) {
                Ok(Some(tokens)) => {
                    handle.set_context_window(provider.id.clone(), model, tokens);
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "the context probe's answer cannot be used"),
            }
        });
    }

    /// Ends a session: its turn is interrupted, its loop ends, and its log
    /// is closed.
    async fn end(&self, session: &SessionId) {
        let Some(live) = lock(&self.inner.sessions).remove(session) else {
            return;
        };
        // A question that holds the session (an undo's) ends as no answer:
        // nothing it would do runs on a session that has ended.
        self.inner.clients.withdraw(Some(session));
        live.handle.close();
        let task = lock(&live.task).take();
        if let Some(task) = task {
            let _ = task.await;
        }
        let log = Arc::clone(&live.log);
        let sandbox = Arc::clone(&live.sandbox);
        let closed = Self::blocking(move || {
            // The protected paths are looked at once more against the look
            // after the last confined command: a change in between that the
            // harness's own tools did not make was made from outside — a
            // process a command left running, or the user's own hand. There
            // is no turn to stop at close, so it is recorded as a notice the
            // next session reads (found by the third review of step 26). The
            // undo waits on the resume
            // (`docs/outstanding/the-last-writes-of-a-killed-command.md`).
            let outside = sandbox.continuity_at_close();
            // Nothing of the session runs any more: what its commands left
            // in their temporary directory goes.
            sandbox.remove_temp();
            let mut kept = lock(&log);
            if let Some(disk) = kept.disk() {
                // What the harness's own tools changed since the last
                // after-look is not a change from outside.
                let touched = disk.rounds.touched();
                let outside: Vec<_> = outside
                    .into_iter()
                    .flatten()
                    .filter(|one| {
                        cineris_base::AbsPath::from_os(&one.path)
                            .ok()
                            .is_none_or(|path| !touched.contains(path.key()))
                    })
                    .collect();
                if !outside.is_empty() && disk.session.is_born() {
                    let names: Vec<String> = outside
                        .iter()
                        // Names from the workspace, which the model reads
                        // when the session is resumed.
                        .map(|one| cineris_base::visible(&one.path.to_string_lossy()).into_owned())
                        .collect();
                    // An id of its own, after every item in the log: two
                    // such notices with one id made a summary that covered
                    // the second read back as covering the first (the
                    // review of step 15, its second note).
                    let id = disk.session.next_item_id();
                    let _ = disk.session.append(
                        None,
                        Body::Notice(cineris_protocol::item::NoticeItem {
                            id,
                            source: NoticeSource::Harness,
                            text: format!(
                                "A protected path changed since the last command, and none of \
                                 the harness's own tools changed it: a process an earlier command \
                                 left running, or a change made by hand. They decide what runs \
                                 later outside the sandbox: {}. Look at them before trusting the \
                                 workspace, and undo the turn to put them back.",
                                names.join(", ")
                            ),
                            created_at: None,
                        }),
                    );
                }
                // The base state, if its capture ends in time. It nearly
                // always has by now: it takes far less than one request to
                // a model. A capture that is slower than this is let go
                // with the session, which is then not replayable.
                if let Some(state) = disk
                    .base
                    .take()
                    .and_then(|capture| capture.wait_for(BASE_WAIT))
                    && disk.session.is_born()
                {
                    let _ = disk.session.append(None, Body::Base(state));
                }
                let _ = disk.session.end_turn();
            }
            Ok(())
        })
        .await;
        if closed.is_err() {
            tracing::warn!(%session, "the session's log was not closed cleanly");
        }
    }

    /// Ends every session, and starts no more. For the end of the process.
    ///
    /// The mark that no session starts is set first, and the sessions are
    /// looked at after: one that is put together meanwhile either is in the
    /// table by then, and is ended here, or finds the mark when it is put
    /// in the table, and ends itself.
    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        // Every trust and rewind question still open ends as no answer, so
        // that no open and no undo waits for a person past the end
        // (`docs/architecture/06-permissions-and-sandbox.md`, "Project
        // trust"; the review of step 23).
        self.inner.clients.withdraw(None);
        let sessions: Vec<SessionId> = lock(&self.inner.sessions).keys().cloned().collect();
        for session in sessions {
            self.end(&session).await;
        }
        // A shell still being asked its version is waited for, so that the
        // answer is kept: each ask is bounded by its own time limit.
        let probes = std::mem::take(&mut *lock(&self.inner.probes));
        if !probes.is_empty() {
            let _ = Self::blocking(move || {
                for probe in probes {
                    let _ = probe.join();
                }
                Ok(())
            })
            .await;
        }
        // The statistics the sessions recorded are written before the
        // process goes: its writer is a thread that would die with it. A
        // writer stuck on its disk is waited for only so long.
        if let Some(recorder) = self.inner.stats.clone() {
            let flushed = crate::stats::flush(recorder.clone(), &self.inner.options.timer).await;
            // A writer never started with nothing waiting has nothing to
            // say; any other that did not finish is said, though nothing
            // counted is waiting: it may be stuck on the limits.
            if !matches!(
                flushed,
                cineris_stats::Flushed::Done | cineris_stats::Flushed::NotStarted { waiting: 0 }
            ) || recorder.dropped() > 0
            {
                tracing::warn!(
                    unwritten = flushed.unwritten(),
                    dropped = recorder.dropped(),
                    outcome = ?flushed,
                    "the statistics writer did not finish before the end: these events are not \
                     recorded"
                );
            }
        }
    }

    /// Carries out one operation for `client`.
    ///
    /// # Errors
    ///
    /// The protocol's error object: its kind, a stable code and a message
    /// for a person.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm an operation, each a few lines; splitting it would hide the table"
    )]
    pub async fn handle(&self, client: ClientId, op: Op) -> Result<OpResult, OpError> {
        match op {
            Op::Initialize(params) => {
                let Some(version) = negotiate_version(&params, SUPPORTED_PROTOCOL_VERSIONS) else {
                    return Err(error(
                        ErrorKind::Usage,
                        code::UNSUPPORTED_PROTOCOL_VERSION,
                        format!(
                            "This server speaks protocol version {PROTOCOL_VERSION}, and the \
                             client asked for {}.",
                            params.protocol_version
                        ),
                    ));
                };
                if !params.role.is_known() {
                    return Err(error(
                        ErrorKind::Usage,
                        code::UNKNOWN_VALUE,
                        format!("`{}` is not a client role.", params.role.as_str()),
                    ));
                }
                Ok(OpResult::Initialize(InitializeResult {
                    protocol_version: version,
                    server: PeerInfo {
                        name: "cineris".to_owned(),
                        version: self.inner.options.harness_version.clone(),
                    },
                    capabilities: ServerCapabilities {
                        sessions: true,
                        mcp: false,
                        extensions: false,
                        sandbox: self.inner.sandbox.status(),
                        experimental: Vec::new(),
                    },
                }))
            }
            Op::SessionNew(params) => {
                self.may_drive(client)?;
                let overrides = config_overrides(&params.config)?;
                // `session/set`'s rule, at the start: only a `user` raises a
                // level or loosens what the harness enforces. Anyone else's
                // session starts no higher than it would without them.
                let user =
                    self.inner.clients.info(client).map(|info| info.role) == Some(ClientRole::User);
                if !user {
                    only_preferences(&overrides)?;
                }
                let live = self
                    .open(
                        client,
                        Wanted {
                            fork: None,
                            which: Which::New {
                                id: self.inner.ids.session_id(),
                                title: params.title,
                                ephemeral: params.ephemeral,
                                cwd: params.cwd.map(PathBuf::from),
                            },
                            workspace: PathBuf::from(params.workspace),
                            settings: params.settings,
                            overrides,
                            no_raise: !user,
                        },
                    )
                    .await?;
                Ok(OpResult::SessionNew(SessionResult {
                    session: live.handle.state(),
                }))
            }
            Op::SessionOpen(params) => {
                // Open already: this client joins it.
                if let Ok(live) = self.live(&params.session_id) {
                    let Some(queue) = self.inner.clients.queue(client) else {
                        return Err(Self::not_connected());
                    };
                    // A session has at most one supervisor: a second is
                    // refused, looked at and added under one lock.
                    let clients = &self.inner.clients;
                    let supervisor = clients.is_supervisor(client.0);
                    if !live.sink.subscribe_unless(client.0, queue, |other| {
                        supervisor && clients.is_supervisor(other)
                    }) {
                        return Err(error(
                            ErrorKind::Usage,
                            code::SUPERVISOR_PRESENT,
                            format!(
                                "The session `{}` has a supervisor already: a session has at \
                                 most one.",
                                params.session_id
                            ),
                        ));
                    }
                    // A client that left in the meantime is not left on it.
                    self.still_wanted(client, &live, false).await?;
                    // A question that is waiting is put to a client that
                    // may answer it.
                    self.inner.clients.join(&params.session_id, client);
                    self.tell_grant_limits(client, &live);
                    return Ok(OpResult::SessionOpen(SessionResult {
                        session: live.handle.state(),
                    }));
                }
                // Starting a stored session writes to it (its lock, its
                // lease), which is more than watching.
                self.may_drive(client)?;
                let workspace = self.stored_workspace(&params.session_id).await?;
                let live = self
                    .open(
                        client,
                        Wanted {
                            fork: None,
                            which: Which::Resume {
                                id: params.session_id,
                            },
                            workspace,
                            settings: SessionSettings::default(),
                            overrides: Vec::new(),
                            no_raise: false,
                        },
                    )
                    .await?;
                Ok(OpResult::SessionOpen(SessionResult {
                    session: live.handle.state(),
                }))
            }
            Op::SessionList(params) => {
                let sessions = self.list(params.workspace).await?;
                let limit = params.limit.map_or(usize::MAX, |limit| limit as usize);
                // What comes after the last summary of the page before, in
                // the list's order: a session used between two pages moves
                // ahead of the cursor, and nothing after it moves (the
                // review of step 23; settled in step 27).
                let after = match params.cursor.as_deref() {
                    Some(cursor) => Some(ListCursor::read(cursor)?),
                    None => None,
                };
                let page: Vec<SessionSummary> = sessions
                    .iter()
                    .filter(|summary| after.as_ref().is_none_or(|after| after.is_before(summary)))
                    .take(limit)
                    .cloned()
                    .collect();
                let more = page.last().is_some_and(|last| {
                    let at = ListCursor::of(last);
                    sessions.iter().any(|summary| at.is_before(summary))
                });
                Ok(OpResult::SessionList(SessionListResult {
                    next_cursor: page
                        .last()
                        .filter(|_| more)
                        .map(|last| ListCursor::of(last).text()),
                    sessions: page,
                }))
            }
            Op::SessionRead(params) => {
                let live = self.open_for(client, &params.session_id)?;
                // The `seq` and what is in progress, taken together: an
                // event after it is not in the answer, and the client gets
                // it live.
                let reader = self.inner.clients.queue(client);
                let (seq, progress) = live.sink.snapshot(reader.as_deref());
                let log = Arc::clone(&live.log);
                let logged = Self::blocking(move || {
                    lock(&log)
                        .items()
                        .map_err(|problem| error(ErrorKind::Internal, "store_failed", problem))
                })
                .await?;
                // The log by a cursor of its own, and the items in progress
                // whole in the last page, so nothing moves between pages
                // (settled in step 27).
                let page = crate::paging::page(
                    &params.session_id,
                    logged,
                    progress,
                    params.item_id.as_ref(),
                    params.cursor.as_deref(),
                    params.limit,
                )?;
                let (items, masked) = if params.redacted {
                    let filter = Arc::clone(&self.inner.providers.filter);
                    crate::redact::for_export(page.items, &page.incomplete, filter)
                } else {
                    (page.items, 0)
                };
                Ok(OpResult::SessionRead(SessionReadResult {
                    items,
                    incomplete: page.incomplete,
                    next_cursor: page.next_cursor,
                    seq,
                    masked,
                    redacted: params.redacted,
                }))
            }
            Op::SessionFork(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                // Given back, so that a client can tell this fork from one
                // of all of the session by a core that does not know it.
                let before = params.before.clone();
                let point = crate::rewind::Point::at_most_one(params.at, params.before)?;
                let id = self.inner.ids.session_id();
                let log = Arc::clone(&live.log);
                // Where the fork goes back to, and its workspace; it is
                // written as it is put together, once its level has passed
                // the gate, so that a refused fork leaves no stored session
                // (the reviews of step 27's second pull request).
                let source = Arc::clone(&log);
                let (workspace, at) = Self::blocking(move || {
                    let mut kept = lock(&source);
                    let Some(disk) = kept.disk() else {
                        return Err(error(
                            ErrorKind::Usage,
                            "session_not_stored",
                            "A session with no log cannot be forked.",
                        ));
                    };
                    // Any branch of the tree: a fork does not move this
                    // session's active position.
                    let at = match &point {
                        Some(point) => Some(point.entry(&disk.session, false)?),
                        None => None,
                    };
                    Ok((disk.session.header().workspace.clone(), at))
                })
                .await?;
                let forked = self
                    .open(
                        client,
                        Wanted {
                            which: Which::Resume { id },
                            fork: Some(assemble::ForkFrom { log, at }),
                            workspace: PathBuf::from(workspace),
                            settings: SessionSettings::default(),
                            overrides: Vec::new(),
                            no_raise: false,
                        },
                    )
                    .await?;
                Ok(OpResult::SessionFork(SessionForkResult {
                    session: forked.handle.state(),
                    before,
                }))
            }
            Op::SessionRewind(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                let point = crate::rewind::Point::one(params.to, params.before)?;
                self.rewind(&live, Back::To(point), params.restore_files, params.plan)
                    .await?;
                Ok(OpResult::SessionRewind(SessionResult {
                    session: live.handle.state(),
                }))
            }
            Op::SessionRewindPlan(params) => {
                let live = self.open_for(client, &params.session_id)?;
                // Refused as the rewind would be (the review of step 25's
                // second pull request): an observer is not shown a plan it
                // may not carry out, and nothing is planned under a turn,
                // which may yet add to what it would undo, nor while
                // another change of the history is under way (a rewind's
                // question still open), which would move it. Asked, not
                // begun: a change begun and given up would still have the
                // loop read its history again.
                self.may_drive(client)?;
                let point = crate::rewind::Point::one(params.to, params.before)?;
                live.handle.may_change_history()?;
                let log = Arc::clone(&live.log);
                // Planned as `session/rewind` plans it, and nothing changed:
                // the files are looked at, never touched.
                let plan = Self::blocking(move || {
                    let mut kept = lock(&log);
                    let Some(disk) = kept.disk() else {
                        return Err(error(
                            ErrorKind::Usage,
                            "session_not_stored",
                            "A session with no log cannot be rewound.",
                        ));
                    };
                    let entry = point.entry(&disk.session, true)?;
                    disk.session
                        .plan_rewind(&entry)
                        .map_err(|failure| session_error(&failure))
                })
                .await?;
                Ok(OpResult::SessionRewindPlan(crate::rewind::shown(&plan)))
            }
            Op::SessionUndo(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                self.rewind(&live, Back::Undo, params.restore_files, None)
                    .await?;
                Ok(OpResult::SessionUndo(SessionResult {
                    session: live.handle.state(),
                }))
            }
            Op::WorkspaceFiles(params) => {
                let live = self.open_for(client, &params.session_id)?;
                let workspace = live.handle.state().workspace;
                let gate = cineris_agent::Policy::read_gate(live.policy.as_ref());
                let limit = params.limit.map_or(WORKSPACE_FILES, |limit| {
                    (limit as usize).min(WORKSPACE_FILES)
                });
                // The walk ends with the core (the review of step 23).
                let cancel = self.inner.options.cancel.child();
                let (paths, truncated) = Self::blocking(move || {
                    let resolver = cineris_base::PathResolver::new(
                        Path::new(&workspace),
                        cineris_base::PathOptions::new(),
                    )
                    .map_err(|failure| {
                        error(ErrorKind::Usage, "invalid_workspace", failure.to_string())
                    })?;
                    let root = resolver.resolve(".").map_err(|failure| {
                        error(ErrorKind::Usage, "invalid_workspace", failure.to_string())
                    })?;
                    Ok(cineris_tools::workspace_files(
                        &root,
                        gate.as_ref(),
                        &cancel,
                        limit,
                    ))
                })
                .await?;
                Ok(OpResult::WorkspaceFiles(WorkspaceFilesResult {
                    paths,
                    truncated,
                }))
            }
            Op::WorkspaceWarm(params) => Ok(OpResult::WorkspaceWarm(crate::reads::warm(
                self, client, params,
            )?)),
            Op::SessionArtifact(params) => Ok(OpResult::SessionArtifact(
                crate::reads::artifact(self, client, params).await?,
            )),
            Op::SessionClose(params) => {
                if let Ok(live) = self.live(&params.session_id) {
                    live.sink.unsubscribe(client.0);
                    // A client that closes its view leaves the session's
                    // open questions, as one that disconnects does: its
                    // answer to them is refused from now on.
                    self.inner.clients.leave(&params.session_id, client);
                    // Only a client that may drive a session ends it: an
                    // observer's close never does.
                    if self.may_drive(client).is_ok() && live.sink.subscribers().is_empty() {
                        self.end(&params.session_id).await;
                    }
                }
                Ok(OpResult::SessionClose(Empty {}))
            }
            Op::TurnStart(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                // The start of a turn is one of the moments the files are
                // looked at (08-configuration, "How changes are noticed"):
                // a prompt that steers a running turn starts none.
                // Asked as a prompt decides: a turn a prompt has started
                // and the loop has not picked up yet holds the session too,
                // and a look then would let it end before this prompt
                // queues behind it.
                if !live.handle.turn_running() {
                    crate::reload::at_turn_start(self, &live).await;
                }
                // Refused with `history_changing` while a rewind is under
                // way, under the session's own lock.
                let result = live.handle.prompt(params.input, params.delivery)?;
                Ok(OpResult::TurnStart(result))
            }
            Op::TurnInterrupt(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                let result = if params.now {
                    live.handle.interrupt_now()
                } else {
                    live.handle.interrupt()
                };
                Ok(OpResult::TurnInterrupt(result))
            }
            Op::SessionCompact(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                live.handle.compact(params.focus)?;
                Ok(OpResult::SessionCompact(Empty {}))
            }
            Op::SessionSet(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                let choices = assemble::settings_overrides(&params.settings);
                // Under the session's reload lock: a reload building from
                // the files never puts its models or level over a choice
                // made while it built (the second review of step 24's
                // first part).
                let _one = live.reloading.lock().await;
                self.set(client, &live, params.settings).await?;
                // What the person chose outranks the files from now on.
                crate::reload::chosen(&live, choices).await;
                Ok(OpResult::SessionSet(SessionResult {
                    session: live.handle.state(),
                }))
            }
            Op::PlanGet(params) => {
                let live = self.open_for(client, &params.session_id)?;
                Ok(OpResult::PlanGet(PlanResult {
                    plan: live.board.get(),
                }))
            }
            Op::PlanUpdate(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                let change = live.board.apply(&params.edits).map_err(|problem| {
                    error(ErrorKind::Usage, "invalid_plan_edit", problem.to_string())
                })?;
                if let Some(notice) = change.notice {
                    // The model is told at the next safe boundary, and the
                    // board is told when the loop has recorded that.
                    live.hooks.handed_plan_notice(change.plan.revision);
                    live.handle.notice(NoticeSource::Plan, notice);
                    let log = Arc::clone(&live.log);
                    let plan = change.plan.clone();
                    let recorded = Self::blocking(move || {
                        lock(&log)
                            .record_plan(None, &plan)
                            .map_err(|problem| error(ErrorKind::Internal, "store_failed", problem))
                    })
                    .await;
                    if let Err(problem) = recorded {
                        live.hooks.tell(format!(
                            "The plan was changed, and could not be written to the session: {}",
                            problem.message
                        ));
                    }
                    live.handle.emit(Event::PlanUpdated(PlanUpdated {
                        plan: change.plan.clone(),
                        by: Actor::User,
                    }));
                }
                Ok(OpResult::PlanUpdate(PlanResult { plan: change.plan }))
            }
            Op::UsageGet(params) => {
                let live = self.open_for(client, &params.session_id)?;
                Ok(OpResult::UsageGet(UsageGetResult {
                    usage: live.handle.usage(),
                }))
            }
            Op::UsageReport(params) => Ok(OpResult::UsageReport(
                crate::usage::report(self, client, &params).await?,
            )),
            Op::UsageRefresh(params) => {
                self.require_user(client)?;
                Ok(OpResult::UsageRefresh(
                    crate::usage::refresh(self, &params).await,
                ))
            }
            Op::ProjectList(params) => Ok(OpResult::ProjectList(
                crate::knowledge::list(self, client, &params).await?,
            )),
            Op::ProjectUpdate(params) => Ok(OpResult::ProjectUpdate(
                crate::knowledge::update(self, client, params).await?,
            )),
            // A turn that only verifies (`SessionHandle::verify`), when a
            // check could run: what would stop every one is said at once.
            Op::VerifyRun(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                if let Some(why) = live.verifier.why_none().await {
                    return Err(error(ErrorKind::Usage, code::NO_CHECKS, why.message()));
                }
                live.handle.verify()?;
                Ok(OpResult::VerifyRun(Empty {}))
            }
            Op::ModelsList(_) => Ok(OpResult::ModelsList(ModelsListResult {
                models: self
                    .inner
                    .providers
                    .catalog
                    .models()
                    .iter()
                    .map(|entry| entry.info.clone())
                    .collect(),
            })),
            Op::ConfigGet(params) => {
                let (loaded, trusted, chosen) = match &params.session_id {
                    Some(session) => {
                        let live = self.open_for(client, session)?;
                        let (trusted, chosen) = lock(&live.config).profile_and_trust();
                        (live.loaded(), trusted, chosen)
                    }
                    None => (self.inner.user(), false, None),
                };
                // The profiles that may be chosen: a project's only once it
                // is trusted (step 24).
                let mut profiles = loaded.profiles.clone();
                if trusted {
                    profiles.extend(loaded.project_profiles.iter().cloned());
                }
                profiles.sort_unstable();
                profiles.dedup();
                let profile = chosen.or_else(|| loaded.profile.clone());
                let mut config = serde_json::Map::new();
                let mut origins = std::collections::BTreeMap::new();
                for (path, value) in loaded.effective() {
                    let key = path.to_string();
                    if !params.keys.is_empty()
                        && !params
                            .keys
                            .iter()
                            .any(|wanted| key == *wanted || key.starts_with(&format!("{wanted}.")))
                    {
                        continue;
                    }
                    if let Ok(value) = serde_json::to_value(&value) {
                        // Where it came from, for `/config` (since step 24):
                        // a built-in default has no entry.
                        if let Some(origin) =
                            config_origin(&loaded.origin(&path), &self.inner.options.env)
                        {
                            origins.insert(key.clone(), origin);
                        }
                        config.insert(key, value);
                    }
                }
                Ok(OpResult::ConfigGet(ConfigGetResult {
                    config,
                    origins,
                    profiles,
                    profile,
                }))
            }
            Op::ConfigReload(params) => Ok(OpResult::ConfigReload(
                crate::reload::reload(self, client, params).await?,
            )),
            Op::PermissionsList(params) => {
                let live = self.open_for(client, &params.session_id)?;
                Ok(OpResult::PermissionsList(PermissionsListResult {
                    approvals: live.policy.engine().approvals(),
                }))
            }
            Op::PermissionsRevoke(params) => {
                let live = self.open_for(client, &params.session_id)?;
                self.may_drive(client)?;
                let policy = Arc::clone(&live.policy);
                Self::blocking(move || {
                    policy.revoke(&params.ids).map_err(|failure| {
                        error(ErrorKind::Internal, "store_failed", failure.to_string())
                    })
                })
                .await?;
                Ok(OpResult::PermissionsRevoke(Empty {}))
            }
            Op::McpStatus(_) => Ok(OpResult::McpStatus(McpStatusResult {
                servers: Vec::new(),
            })),
            Op::ExtensionsStatus(_) => Ok(OpResult::ExtensionsStatus(ExtensionsStatusResult {
                extensions: Vec::new(),
            })),
            Op::Shutdown(_) => {
                self.may_drive(client)?;
                self.shutdown().await;
                Ok(OpResult::Shutdown(Empty {}))
            }
        }
    }

    /// Only a `user` client: for what only the person at the keyboard may
    /// cause, such as a request to a provider's usage endpoint.
    fn require_user(&self, client: ClientId) -> Result<(), OpError> {
        match self.inner.clients.info(client).map(|info| info.role) {
            Some(ClientRole::User) => Ok(()),
            _ => Err(error(
                ErrorKind::Usage,
                code::ROLE_NOT_ALLOWED,
                "Only a `user` client may ask the providers' usage endpoints.",
            )),
        }
    }

    /// Whether the client may change a session, start one or end the
    /// core: an observer only watches.
    pub(crate) fn may_drive(&self, client: ClientId) -> Result<(), OpError> {
        match self.inner.clients.info(client).map(|info| info.role) {
            Some(ClientRole::User | ClientRole::Supervisor) => Ok(()),
            _ => Err(error(
                ErrorKind::Usage,
                code::ROLE_NOT_ALLOWED,
                "An `observer` client watches a session and cannot change it.",
            )),
        }
    }

    /// Moves a session back: to a point named by an item, after it or just
    /// before it (`session/rewind`), or to before its last turn
    /// (`session/undo`), putting files back if asked.
    ///
    /// A file changed since the harness left it is put back only if the
    /// person says so ([07-sessions-and-context.md], "Snapshots and undo"):
    /// with a client that answers questions, the plan's files edited since
    /// are put to it in one `question/ask`, and only "put them back"
    /// overwrites them; no answer keeps them. No turn starts meanwhile: the
    /// history change is held from the plan to the rewind.
    ///
    /// With the `mark` of a plan a client showed (`session/rewind_plan`,
    /// since step 25), the plan made here must be that one, or nothing is
    /// changed (`plan_changed`): its point, the position it goes back from
    /// and its turns, and with files put back what it does to each file.
    async fn rewind(
        &self,
        live: &Arc<Live>,
        back: Back,
        restore_files: bool,
        mark: Option<String>,
    ) -> Result<(), OpError> {
        // History is not changed under a turn, and no turn starts while it
        // is being changed: both are decided under the session's own lock.
        let change = live.handle.change_history()?;
        let log = Arc::clone(&live.log);
        let (change, plan) = Self::blocking(move || {
            let mut kept = lock(&log);
            let Some(disk) = kept.disk() else {
                return Err(error(
                    ErrorKind::Usage,
                    "session_not_stored",
                    "A session with no log cannot be rewound.",
                ));
            };
            let plan = match back {
                Back::To(point) => {
                    let to = point.entry(&disk.session, true)?;
                    disk.session
                        .plan_rewind(&to)
                        .map_err(|failure| session_error(&failure))?
                }
                Back::Undo => disk
                    .session
                    .plan_undo()
                    .map_err(|failure| session_error(&failure))?
                    .ok_or_else(|| {
                        error(
                            ErrorKind::Usage,
                            "nothing_to_undo",
                            "The session has no turn to undo.",
                        )
                    })?,
            };
            if let Some(mark) = &mark
                && !crate::rewind::still(&plan, mark, restore_files)
            {
                return Err(error(
                    ErrorKind::Usage,
                    code::PLAN_CHANGED,
                    "The rewind's plan changed after it was shown: a turn or a file it names is \
                     not as it was. Nothing was changed; plan it again.",
                ));
            }
            Ok((change, plan))
        })
        .await?;

        let mut overwrite = Vec::new();
        if restore_files {
            let edited: Vec<String> = plan
                .files
                .iter()
                .filter(|file| file.status == cineris_session::FileStatus::EditedSince)
                .map(|file| file.path.clone())
                .collect();
            if !edited.is_empty() && self.put_them_back(live, &edited).await {
                overwrite = edited;
            }
        }
        // Files put back are files changed: a test that failed before the
        // rewind and passes after it has not flipped. Counted before the
        // files are touched: the work below goes on to its end though the
        // client that asked for it has gone, and may put some files back
        // and then fail.
        if restore_files && let Some(knowledge) = &live.knowledge {
            knowledge.file_changed();
        }

        let log = Arc::clone(&live.log);
        let hooks = Arc::clone(&live.hooks);
        let board = Arc::clone(&live.board);
        let inner = Arc::clone(&self.inner);
        let this = Arc::clone(live);
        let not_restored = Self::blocking(move || {
            // The change is held by the work, not by this operation: the
            // work is never cancelled, so if the operation is given up (its
            // client went away) no turn starts until the log is no longer
            // being rewritten.
            let mut change = change;
            let mut kept = lock(&log);
            // A session that ended while its question was open, or since,
            // is not rewound: its log is closed, and its files are left as
            // they are (the review of step 23). Its end takes it out of the
            // table before it closes the log under this lock.
            let open = lock(&inner.sessions)
                .get(this.handle.id())
                .is_some_and(|now| Arc::ptr_eq(now, &this));
            if !open {
                return Err(error(
                    ErrorKind::Usage,
                    code::SESSION_NOT_OPEN,
                    "The session was closed before it could be moved back: nothing was changed.",
                ));
            }
            let Some(disk) = kept.disk() else {
                return Err(error(
                    ErrorKind::Usage,
                    "session_not_stored",
                    "A session with no log cannot be rewound.",
                ));
            };
            let restore = if restore_files {
                Restore {
                    overwrite_edited: overwrite,
                    ..Restore::files()
                }
            } else {
                Restore::conversation_only()
            };
            let done = disk
                .session
                .rewind(&plan, &restore)
                .map_err(|failure| session_error(&failure))?;
            // The table of seen files forgets every file, and the loop
            // reads the history again before its next turn.
            change.made();
            let entries = disk
                .session
                .read_branch()
                .map_err(|failure| session_error(&failure))?;
            // The plan goes back with the conversation, before a turn can
            // start on it.
            hooks.forget_plan_notices();
            board.restore(crate::store::plan_of(&entries).unwrap_or_default());
            drop(kept);
            drop(change);
            Ok(done.not_restored)
        })
        .await?;
        live.handle.emit(Event::PlanUpdated(PlanUpdated {
            plan: live.board.get(),
            by: Actor::Harness,
        }));
        if !not_restored.is_empty() {
            live.hooks.tell(format!(
                "These files could not be put back as they were: {}.",
                not_restored
                    .iter()
                    .map(|(file, why)| format!("{} ({why})", cineris_base::visible(file)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(())
    }

    /// Asks the session's clients that answer questions whether files
    /// edited since the harness left them are to be put back. No client to
    /// ask, no answer, a client that goes away and the end of the process
    /// are all "keep them".
    async fn put_them_back(&self, live: &Arc<Live>, edited: &[String]) -> bool {
        let asked = self
            .inner
            .clients
            .askable(&live.sink.subscribers(), crate::clients::Kind::Question);
        if asked.is_empty() {
            return false;
        }
        let names: Vec<String> = edited
            .iter()
            .map(|path| format!("`{}`", cineris_base::visible(path)))
            .collect();
        let question = QuestionRequest {
            session_id: live.handle.id().clone(),
            turn_id: None,
            question: format!(
                "{} {} changed after the turn that changed {}: {}. Put {} back as {} before that \
                 turn, losing those changes?",
                if edited.len() == 1 {
                    "This file was"
                } else {
                    "These files were"
                },
                "",
                if edited.len() == 1 { "it" } else { "them" },
                names.join(", "),
                if edited.len() == 1 { "it" } else { "them" },
                if edited.len() == 1 {
                    "it was"
                } else {
                    "they were"
                },
            )
            .replace("  ", " "),
            choices: vec![PUT_BACK.to_owned(), KEEP.to_owned()],
            extension: None,
        };
        let session = live.handle.id().clone();
        let (id, answer) = self.inner.clients.ask_once(&session, &asked, |_| {
            ServerRequest::Question(question.clone())
        });
        // A session that ended since its plan was made (its end withdrew
        // what was open then) takes the question back at once: its end
        // takes it out of the table before it withdraws, so one or the
        // other sees this question (the review of step 23's fixes).
        let still_open = lock(&self.inner.sessions)
            .get(&session)
            .is_some_and(|now| Arc::ptr_eq(now, live));
        if !still_open {
            self.inner.clients.close(&id);
        }
        let answered = self.inner.options.cancel.race(answer).await;
        let answered = answered.map(|got| got.map(|answer| answer.response));
        // Answered only when an answer came: a request withdrawn because its
        // askers left is closed already, and was not answered (the
        // re-review of step 23's second pull request).
        let closed = self.inner.clients.close(&id);
        let resolution = if !closed && matches!(answered, Some(Ok(_))) {
            Resolution::Answered
        } else {
            Resolution::Cancelled
        };
        live.handle.emit(Event::RequestResolved(RequestResolved {
            request_id: id,
            resolution,
        }));
        matches!(
            answered,
            Some(Ok(ServerResponse::Question(QuestionResponse { answer: Some(answer) })))
                if answer == PUT_BACK
        )
    }

    /// Changes a session's settings.
    ///
    /// Everything asked for is checked and made ready before anything is
    /// changed, so a request that is refused in any part changes nothing.
    async fn set(
        &self,
        client: ClientId,
        live: &Arc<Live>,
        settings: SessionSettings,
    ) -> Result<(), OpError> {
        let inner = &self.inner;
        let has_history = live.handle.has_history();
        let mut uncached = false;
        let mut told = Vec::new();

        // ------------------------------------------------------ checked
        let mut new_level = None;
        if let Some(level) = settings.level {
            if !level.is_known() {
                return Err(error(
                    ErrorKind::Usage,
                    code::UNKNOWN_VALUE,
                    format!("`{}` is not a level.", level.as_str()),
                ));
            }
            let current = live.policy.engine().level().clone();
            if level != current {
                let raises = !cineris_policy::level::no_higher_than(&level, &current);
                let role = inner.clients.info(client).map(|info| info.role);
                if raises && role != Some(ClientRole::User) {
                    return Err(error(
                        ErrorKind::Permission,
                        code::ROLE_NOT_ALLOWED,
                        "Only a `user` client may raise a session's level.",
                    ));
                }
                // Before anything is made ready: a level the core may not
                // give refuses the whole request.
                inner.may_hold(&level)?;
                new_level = Some(level);
            }
        }

        let mut roles = Vec::new();
        for (role, change) in settings.roles {
            let speed = change.speed.clone();
            if change.model.is_none() && change.effort.is_none() && change.provider.is_none() {
                roles.push((role, speed, None));
                continue;
            }
            let current = live.handle.role_model(&role);
            let in_force = live.loaded();
            let model_settings = &in_force.settings.model;
            // What is not named stays: the model, and the level as it was
            // set (not as it was mapped, which belongs to the old model).
            let entry = match (&change.model, &current) {
                (Some(model), _) => inner.providers.entry(
                    model.as_str(),
                    change
                        .provider
                        .as_ref()
                        .map(cineris_protocol::ProviderId::as_str),
                )?,
                (None, Some(current)) => match &change.provider {
                    Some(provider) => inner
                        .providers
                        .entry(current.entry.info.id.as_str(), Some(provider.as_str()))?,
                    None => Arc::clone(&current.entry),
                },
                (None, None) => {
                    return Err(error(
                        ErrorKind::Usage,
                        "no_model",
                        format!(
                            "The role `{}` has no model: name one to give it a reasoning level.",
                            role.as_str()
                        ),
                    ));
                }
            };
            let named = change.effort.clone();
            let carried = current.as_ref().and_then(|model| model.effort.clone());
            let max_output = current
                .as_ref()
                .map_or(model_settings.max_output, |model| model.max_output);
            let settings = live.loaded().settings.clone();
            let env = inner.options.env.clone();
            let images = Arc::clone(&live.images);
            let core = Arc::clone(inner);
            // Finding a key can read the credential store.
            let (model, left) = Self::blocking(move || {
                let make = |effort: Option<&str>| {
                    core.providers.role_model(
                        &settings,
                        &env,
                        Arc::clone(&entry),
                        effort,
                        max_output,
                        Arc::clone(&images),
                    )
                };
                match (named, carried) {
                    // A level that was asked for is refused if the model
                    // has no such level.
                    (Some(named), _) => make(Some(&named)).map(|model| (model, None)),
                    // One that is only carried over from the model before
                    // is left behind where the new model has no such
                    // level, or none that leaves the answer room within its
                    // output limit: nobody asked for it here.
                    (None, Some(carried)) => match make(Some(&carried)) {
                        Err(refused)
                            if matches!(
                                refused.code.as_str(),
                                "unknown_effort" | "reasoning_budget_too_large"
                            ) =>
                        {
                            make(None).map(|model| (model, Some((carried, refused.code))))
                        }
                        made => made.map(|model| (model, None)),
                    },
                    (None, None) => make(None).map(|model| (model, None)),
                }
            })
            .await?;
            if let Some((left, why)) = left {
                let cannot = if why == "unknown_effort" {
                    format!("has no reasoning level `{left}`")
                } else {
                    format!("cannot reason at `{left}` and leave its answer room")
                };
                told.push(format!(
                    "`{}` {cannot}, so the role `{}` runs at the model's own default.",
                    model.entry.info.id,
                    role.as_str()
                ));
            }
            roles.push((role, speed, Some(model)));
        }

        // ------------------------------------------------------ applied
        // The level first: it is the one change that can still fail (the
        // level cannot be held, or its lease cannot be written), and then
        // nothing else has been changed.
        if let Some(level) = new_level {
            self.set_level(live, level).await?;
        }

        if let Some(speed) = settings.speed {
            let model = live.handle.role_model(&ModelRole::Main);
            let shares = model
                .as_ref()
                .and_then(|model| model.entry.info.speed_offer(&speed))
                .and_then(|offer| offer.shares_cache);
            if live.handle.state().speed != speed && shares != Some(true) {
                uncached = true;
            }
            live.handle.set_speed(speed);
        }

        for (role, speed, model) in roles {
            if role == ModelRole::Main && (speed.is_some() || model.is_some()) {
                uncached = true;
            }
            if let Some(speed) = speed {
                live.handle.set_role_speed(role.clone(), speed);
            }
            if let Some(model) = model {
                live.handle.set_role_model(role, model);
            }
        }

        for notice in told {
            live.hooks.tell(notice);
        }
        if uncached && has_history {
            live.hooks.tell(UNCACHED);
        }
        Ok(())
    }

    /// Puts the session at another level: a new engine with the same
    /// rules, and a new standing lease in the log.
    async fn set_level(&self, live: &Arc<Live>, level: Level) -> Result<(), OpError> {
        let mut parts = lock(&live.template).parts.clone();
        parts.level = level;
        self.put_engine(live, parts).await
    }

    /// Whether a session of the project `key` runs a turn now.
    pub(crate) fn a_turn_runs_in(&self, key: &cineris_protocol::ids::ProjectKey) -> bool {
        lock(&self.inner.sessions).values().any(|live| {
            live.knowledge
                .as_ref()
                .is_some_and(|knowledge| knowledge.key == *key)
                && live.handle.turn_running()
        })
    }

    /// The sessions `client` has open.
    pub(crate) fn open_sessions_of(&self, client: ClientId) -> Vec<Arc<Live>> {
        lock(&self.inner.sessions)
            .values()
            .filter(|live| live.sink.subscribers().contains(&client.0))
            .cloned()
            .collect()
    }

    /// Puts a new engine made from `parts` in force: a change of level, or a
    /// reload of the configuration (step 24). What was approved so far
    /// stays approved, the lease is recorded again in the log, and the
    /// sandbox follows the level.
    pub(crate) async fn put_engine(
        &self,
        live: &Arc<Live>,
        mut parts: cineris_policy::Parts,
    ) -> Result<(), OpError> {
        // Every way here has passed the gate already (`session/set` in its
        // checks, a reload by leaving a refused level out); kept so that a
        // new way cannot put a level in force round it.
        self.inner.may_hold(&parts.level)?;
        let level = parts.level.clone();
        let old = live.policy.engine();
        let number = live.lease_count.fetch_add(1, Ordering::Relaxed) + 1;
        let lease_id = LeaseId::new(format!("l_{number}"));
        parts.lease_id = lease_id.clone();
        // What was approved so far stays approved, each as what it is:
        // what is stored for the project is read as stored, and what was
        // allowed for the session is taken over as the session's own.
        parts.approved = old
            .approvals()
            .into_iter()
            .filter(|approval| approval.scope == ApprovalScope::Project)
            .collect();
        let engine = assemble::start_engine(parts.clone())?;
        engine.keep_session_approvals_of(&old);
        let (ended, granted) = (old.lease().clone(), engine.lease().clone());
        let notice = engine.sandbox_notice();
        let log = Arc::clone(&live.log);
        Self::blocking(move || {
            let mut kept = lock(&log);
            if let Some(disk) = kept.disk() {
                for (event, lease) in [(LeaseEvent::Ended, ended), (LeaseEvent::Granted, granted)] {
                    disk.session
                        .append(None, Body::Lease(LeaseRecord { event, lease }))
                        .map_err(|failure| session_error(&failure))?;
                }
            }
            Ok(())
        })
        .await?;
        lock(&live.template).parts = parts;
        cineris_base::hook::await_point!(crate::policy::BEFORE_THE_NEW_LEVEL_IS_IN_FORCE);
        // The new engine is given the project's learned rules from the one
        // place that holds them, as it is put in force.
        match &live.knowledge {
            Some(knowledge) => knowledge.put_in_force(engine),
            None => live.policy.replace(engine),
        }
        // Nothing is confined at `unrestricted`, and everything is again
        // below it.
        live.sandbox.set_level(&level);
        live.policy.set_sandboxed(live.sandbox.confines());
        live.handle.set_sandbox(if live.sandbox.confines() {
            SandboxStatus::Enforcing
        } else {
            SandboxStatus::Off
        });
        live.handle.set_level(level, Some(lease_id));
        if let Some(message) = notice {
            live.hooks
                .emit(Event::Notice(cineris_protocol::Notice::Sandbox(
                    cineris_protocol::event::SandboxNotice {
                        message,
                        status: SandboxStatus::Off,
                    },
                )));
        }
        Ok(())
    }

    /// Where the sessions of every project are kept.
    fn sessions_root(&self) -> Result<SessionsRoot, OpError> {
        let data = self.inner.user().data_dir().ok_or_else(|| {
            error(
                ErrorKind::Config,
                "no_data_dir",
                "There is nowhere sessions are kept: no data directory could be worked out.",
            )
        })?;
        Ok(SessionsRoot::in_data_dir(&data))
    }

    /// The stored sessions of a workspace, or of every workspace, the one
    /// added to last first.
    async fn list(&self, workspace: Option<String>) -> Result<Vec<SessionSummary>, OpError> {
        let root = self.sessions_root()?;
        Self::blocking(move || {
            let projects: Vec<cineris_session::ProjectSessions> = match workspace {
                Some(workspace) => {
                    let Some(resolved) = assemble::resolve_dir(std::path::Path::new(&workspace))
                    else {
                        return Ok(Vec::new());
                    };
                    root.project(&resolved.project_key()).into_iter().collect()
                }
                None => match std::fs::read_dir(root.path()) {
                    Ok(entries) => entries
                        .flatten()
                        .map(|entry| cineris_session::ProjectSessions::at(entry.path()))
                        .collect(),
                    Err(_) => Vec::new(),
                },
            };
            let mut sessions: Vec<SessionSummary> = Vec::new();
            for project in projects {
                if let Ok(records) = project.index().list() {
                    sessions.extend(records.iter().map(SessionSummary::from));
                }
            }
            // The one used last first, across projects: each index is in that
            // order, and the merge keeps it. Times are compared as times,
            // the id breaking a tie.
            let time = |stamp: &cineris_protocol::Timestamp| {
                cineris_base::UtcTime::parse(stamp.as_str())
                    .unwrap_or(cineris_base::UtcTime::UNIX_EPOCH)
            };
            sessions.sort_by(|a, b| {
                time(&b.updated_at)
                    .cmp(&time(&a.updated_at))
                    .then_with(|| b.session_id.cmp(&a.session_id))
            });
            Ok(sessions)
        })
        .await
    }

    /// The workspace a stored session was made in.
    async fn stored_workspace(&self, session: &SessionId) -> Result<PathBuf, OpError> {
        let wanted = session.clone();
        let found = self
            .list(None)
            .await?
            .into_iter()
            .find(|summary| summary.session_id == wanted);
        found
            .map(|summary| PathBuf::from(summary.workspace))
            .ok_or_else(|| {
                error(
                    ErrorKind::Usage,
                    code::UNKNOWN_SESSION,
                    format!("There is no stored session `{session}`."),
                )
            })
    }

    /// The open sessions of the project with this key.
    pub(crate) fn sessions_of(
        &self,
        project: &cineris_protocol::ids::ProjectKey,
    ) -> Vec<Arc<Live>> {
        lock(&self.inner.sessions)
            .values()
            .filter(|live| {
                live.knowledge
                    .as_ref()
                    .is_some_and(|knowledge| knowledge.key == *project)
            })
            .cloned()
            .collect()
    }

    /// Where the state directory is: the trust store, stored approvals and
    /// log files. `None` when no home directory could be worked out.
    #[must_use]
    pub fn state_dir(&self) -> Option<StateDir> {
        StateDir::from_dirs(&self.inner.user().dirs)
    }
}

/// Where a page of `session/list` ended: the last summary's `updated_at`
/// and id, which order the list (the one used last first, the id breaking a
/// tie). As text it is the cursor a client is given, opaque to it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListCursor {
    updated_at: cineris_base::UtcTime,
    session: SessionId,
}

impl ListCursor {
    /// The time a summary was last used, as the list orders it: a time
    /// that cannot be read is the earliest.
    fn time(stamp: &cineris_protocol::Timestamp) -> cineris_base::UtcTime {
        cineris_base::UtcTime::parse(stamp.as_str()).unwrap_or(cineris_base::UtcTime::UNIX_EPOCH)
    }

    fn of(summary: &SessionSummary) -> Self {
        Self {
            updated_at: Self::time(&summary.updated_at),
            session: summary.session_id.clone(),
        }
    }

    /// Whether `summary` comes after this place in the list: used longer
    /// ago, or at the same time with a smaller id.
    fn is_before(&self, summary: &SessionSummary) -> bool {
        let time = Self::time(&summary.updated_at);
        time < self.updated_at || (time == self.updated_at && summary.session_id < self.session)
    }

    fn text(&self) -> String {
        format!("{} {}", self.updated_at, self.session)
    }

    /// The cursor a client sent back.
    fn read(text: &str) -> Result<Self, OpError> {
        let refused = || {
            error(
                ErrorKind::Usage,
                code::INVALID_CURSOR,
                "That cursor is not one this server gave for `session/list`: list again from \
                 the start.",
            )
        };
        let (time, session) = text.split_once(' ').ok_or_else(refused)?;
        let updated_at = cineris_base::UtcTime::parse(time).map_err(|_| refused())?;
        if session.is_empty() {
            return Err(refused());
        }
        Ok(Self {
            updated_at,
            session: SessionId::new(session),
        })
    }
}

/// The refusal of a session at `unrestricted` where the core may not give
/// one ([`CoreOptions::allow_unrestricted`]).
pub(crate) fn unrestricted_not_allowed() -> OpError {
    error(
        ErrorKind::Permission,
        code::UNRESTRICTED_NOT_ALLOWED,
        "A session at `unrestricted` is the server's own to allow, and this server was not \
         started at it: restart it at `unrestricted` (--permission-mode unrestricted).",
    )
}

fn session_error(failure: &cineris_session::SessionError) -> OpError {
    Box::new(cineris_base::Classified::to_error_data(failure))
}

/// Refuses a session's configuration overrides from a client that is not a
/// `user` when they set anything but a preference, a choice that raises
/// what is spent (a model, a speed: what `session/set` lets such a client
/// choose), or the variables commands are not given (a project's wait for
/// trust because they could stop every command; a client's are its own).
/// Such overrides rank as the user's own command line, so the rest is the
/// user's to give:
///
/// - anything under `[permissions]`, `[sandbox]` or `[limits]`, in a
///   profile too, whatever its value;
/// - what is the user's alone: a provider's address and headers, the
///   shell, the data directory, the credential store;
/// - what runs code or widens access;
/// - a wait or a time limit (tighten-only): a project's file may shorten
///   one and not lengthen it, but an override is past that rule, so a
///   client could make the session wait as long as the range allows.
///
/// Each key is judged by the rule the schema gives it for a project's file.
/// (Choosing a profile is not an override anyone may give: the loader
/// refuses it.)
fn only_preferences(overrides: &[Override]) -> Result<(), OpError> {
    use cineris_config::Trust;
    use cineris_config::schema::{Lookup, Meta, lookup};
    let preference = |trust: &Trust| {
        matches!(
            trust,
            Trust::Preference | Trust::RaisesCost | Trust::BySpeed | Trust::StopsCommands
        )
    };
    for one in overrides {
        let allowed = match cineris_config::KeyPath::parse(&one.key) {
            Ok(path) => match lookup(&path) {
                Lookup::Setting { child, enforced } => {
                    !enforced
                        && matches!(&child.meta, Meta::Leaf { trust, .. } if preference(trust))
                }
                Lookup::InsideSetting { child } => {
                    matches!(&child.meta, Meta::Leaf { trust, .. } if preference(trust))
                }
                Lookup::Reserved { trust } => preference(&trust),
                Lookup::Table { .. } | Lookup::Unknown { .. } => false,
            },
            // A key that cannot be read cannot be shown to be harmless.
            Err(_) => false,
        };
        if !allowed {
            return Err(error(
                ErrorKind::Permission,
                code::ROLE_NOT_ALLOWED,
                format!(
                    "Only a `user` client may set `{}` for a session: permissions, the sandbox, \
                     limits and what is the user's alone are the user's.",
                    one.key
                ),
            ));
        }
    }
    Ok(())
}

/// A session's configuration overrides, as the configuration file would
/// have them, flattened into the settings they set.
fn config_overrides(
    config: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<Override>, OpError> {
    fn walk(
        prefix: &str,
        value: &serde_json::Value,
        overrides: &mut Vec<Override>,
    ) -> Result<(), OpError> {
        if let serde_json::Value::Object(fields) = value {
            for (name, value) in fields {
                let key = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                walk(&key, value, overrides)?;
            }
            return Ok(());
        }
        let value = toml::Value::try_from(value).map_err(|problem| {
            error(
                ErrorKind::Config,
                "invalid_configuration",
                format!("`{prefix}` cannot be a configuration value: {problem}"),
            )
        })?;
        overrides.push(Override {
            key: prefix.to_owned(),
            value: value.to_string(),
            flag: None,
        });
        Ok(())
    }
    let mut overrides = Vec::new();
    walk(
        "",
        &serde_json::Value::Object(config.clone()),
        &mut overrides,
    )?;
    Ok(overrides)
}

#[cfg(test)]
mod tests {
    use cineris_protocol::Timestamp;

    use super::*;

    fn summary(id: &str, at: &str) -> SessionSummary {
        SessionSummary {
            session_id: SessionId::new(id),
            workspace: "/work/app".to_owned(),
            title: None,
            created_at: Timestamp::from("2026-11-13T09:00:00Z"),
            updated_at: Timestamp::from(at),
            parent: None,
            model: None,
            turns: 1,
        }
    }

    /// A page of `session/list` after a cursor holds what comes after the
    /// last summary of the page before, in the list's order: used longer
    /// ago, or at the same moment with a smaller id. A time is compared as
    /// a time, whatever its spelling, and the cursor reads back as it was
    /// written.
    #[test]
    fn a_list_cursor_gives_what_comes_after_it_ties_broken_by_the_id() {
        let at = ListCursor::of(&summary("s_b", "2026-11-13T10:00:00Z"));
        let after = |id: &str, time: &str| at.is_before(&summary(id, time));
        assert!(after("s_z", "2026-11-13T09:59:59Z"), "used longer ago");
        assert!(after("s_a", "2026-11-13T10:00:00Z"), "a tie, a smaller id");
        assert!(after("s_a", "2026-11-13T11:00:00+01:00"), "the same moment");
        assert!(!after("s_b", "2026-11-13T10:00:00Z"), "the cursor's own");
        assert!(!after("s_c", "2026-11-13T10:00:00Z"), "a tie, a larger id");
        assert!(!after("s_a", "2026-11-13T10:00:01Z"), "used since");

        assert_eq!(ListCursor::read(&at.text()).unwrap(), at);
        for wrong in ["", "2", "s_b", "2026-11-13T10:00:00Z", "yesterday s_b"] {
            let refused = ListCursor::read(wrong).unwrap_err();
            assert_eq!(refused.code, "invalid_cursor", "{wrong:?}");
        }
    }
}
