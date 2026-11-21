//! The sections and their settings: every key of a configuration file, declared
//! once (`docs/architecture/08-configuration.md`, "Sections").
//!
//! A section that a later build step owns is `reserved` here: its name is
//! known, its content is accepted unread, and from a project's files it is
//! ignored or held back until that step declares each key and says what a
//! project may set. A reserved section fails closed.

use std::collections::BTreeMap;
use std::time::Duration;

use cineris_protocol::lease::{NetworkAccess, Rule};
use cineris_protocol::{Level, ModelRole, Speed};

use super::types::{
    BackspaceChoice, Bindings, ColorChoice, DensityChoice, GlyphChoice, Headers, HookEventName,
    PresenterChoice, Profiles, ReasoningShown, ReasoningVisibility, Reserved, SessionDurability,
    StoreChoice, ToolOutputChoice, UsageRanges, VerifyMode, Weekday,
};
use super::{CONFIG_VERSION, settings};

settings! {
    /// Everything a configuration file can say, with every default.
    pub struct Settings {
        /// The version of the configuration schema the file was written
        /// for. A file with a newer version than this build understands is
        /// refused.
        [Preference, Restart, 1..=1_000_000]
        pub config_version: u32 = CONFIG_VERSION,
        /// The profile to apply: the name of a `[profiles.<name>]` table.
        /// The `--profile` flag overrides it. A project's file may define
        /// profiles and may not select one: a profile the user wrote is
        /// applied with the user's rights, so a repository that could pick
        /// it could switch on whatever the user's loosest profile allows.
        [UserOnly, NextEpoch]
        pub profile: Option<String>,
        /// Named sets of settings. A profile may hold any key of this file
        /// except `profile`, `profiles`, `managed`, `config_version` and
        /// `hooks`: a hook belongs to the file that defines it, which is
        /// what says whose it is (since step 28).
        [nested]
        pub profiles: Profiles,
        /// For the managed file of a shared machine only.
        [nested]
        pub managed: ManagedSettings,
        /// Models by role, the reasoning level and the speed tier.
        [nested]
        pub model: ModelSettings,
        /// Providers by name. A built-in provider needs no entry; an entry
        /// changes it or adds a new one. `enabled = false` removes one.
        [nested]
        pub providers: BTreeMap<String, ProviderSettings>,
        /// Catalog overrides and additions: limits, prices, reasoning
        /// control, capability flags. Each `[models.<name>]` table is one
        /// whole catalog entry, with the keys of a `[[model]]` table of the
        /// built-in catalog, and replaces the built-in entry for the same
        /// provider and model. The entries are read by the model catalog
        /// itself when the program starts, which refuses one it does not
        /// understand: nothing is started then, and the error names the
        /// file, the entry and what is wrong.
        [reserved UserOnly]
        pub models: Reserved,
        /// The HTTP client: the connect timeout, a proxy, and extra
        /// certificate authorities.
        [reserved UserOnly]
        pub http: Reserved,
        /// How a request that failed is tried again.
        [nested]
        pub retry: RetrySettings,
        /// The permission mode and the permission rules.
        [enforced]
        pub permissions: PermissionSettings,
        /// What a command the harness starts may touch.
        [enforced]
        pub sandbox: SandboxSettings,
        /// The most a turn, a session and a day may spend.
        [enforced]
        pub limits: LimitSettings,
        /// Where stored credentials are kept. Never the credentials
        /// themselves: no key of this file is meant for a key, and a
        /// provider's literal header, which may hold one, is redacted.
        [nested]
        pub credentials: CredentialSettings,
        /// Where sessions are stored.
        [nested]
        pub session: SessionSettings,
        /// Which files written for other tools are read.
        [nested]
        pub compat: CompatSettings,
        /// What is logged.
        [nested]
        pub log: LogSettings,
        /// Enabled tools, output limits and per-tool settings.
        [reserved UserOnly]
        pub tools: Reserved,
        /// The shell that runs commands, the time a command may take, and
        /// the environment variables it is not given.
        [nested]
        pub shell: ShellSettings,
        /// When the conversation is compacted, and which instruction files
        /// are read.
        [nested]
        pub context: ContextSettings,
        /// The project's checks at the end of a turn: whether they run,
        /// the user's own commands, the rounds and the time limits.
        [nested]
        pub verify: VerifySettings,
        /// Variant overrides per model.
        [nested]
        pub adapt: AdaptSettings,
        /// Whether local statistics are recorded, and for how long.
        [nested]
        pub stats: StatsSettings,
        /// The repository map: whether epochs are given one, its budget,
        /// and the caps of the index it is made from.
        [nested]
        pub index: IndexSettings,
        /// The project profile and memory: whether commands are learned,
        /// and the budget of the profile's section of the prompt.
        [nested]
        pub project: ProjectSettings,
        /// The run engine: concurrency, workspaces, budgets.
        [reserved UserOnly]
        pub runs: Reserved,
        /// The usage view: week start, ranges and the opt-in refresh.
        [nested]
        pub usage: UsageSettings,
        /// The terminal UI: how the transcript is drawn.
        [nested]
        pub tui: TuiSettings,
        /// The terminal UI's key bindings, by the context an action
        /// belongs to: `[keys] editor.new_line = ["shift+enter", "ctrl+j"]`.
        [nested]
        pub keys: KeySettings,
        /// What the terminal can do, in place of detecting it. Each is
        /// detected when it is not set.
        [nested]
        pub terminal: TerminalSettings,
        /// The RPC server (`cineris serve`): how long it waits for its
        /// clients.
        [nested]
        pub rpc: RpcSettings,
        /// Hook commands by name: `[hooks.<name>]`, a program run when an
        /// event of the session's happens (since step 28). The user's own
        /// layers merge as any setting's do; **a project's are kept apart**,
        /// never merged with the user's: each waits for the project's trust
        /// and its own approval, as the trust question showed it, and one
        /// with the name of one of the user's is refused. Enforced: a key
        /// this build does not know refuses the file.
        [enforced]
        pub hooks: BTreeMap<String, HookSettings>,
        /// Extensions by name: `[extensions.<name>]`, which only turns one
        /// off (`cineris ext disable` writes it). The user's alone: a
        /// project's entry is ignored, since a project's extension is kept
        /// out by not trusting the project, and a project may not turn the
        /// user's own off.
        [nested]
        pub extensions: BTreeMap<String, ExtensionSettings>,
        /// Where skills are looked for.
        [reserved UserOnly]
        pub skills: Reserved,
        /// MCP servers by name, and MCP settings.
        [reserved RunsCode]
        pub mcp: Reserved,
    }
}

settings! {
    /// What only the managed file may say.
    pub struct ManagedSettings {
        /// Keys that no layer above the managed file may change. A table's
        /// name locks every key in it. Read from the managed file only.
        [UserOnly, Now]
        pub locked: Vec<String>,
    }
}

settings! {
    /// Models by role, the reasoning level and the speed tier.
    pub struct ModelSettings {
        /// The model for the agent's own turns. Needed to run a turn. A
        /// project's file may choose one once the project is trusted: a
        /// model can cost more than the user's.
        [RaisesCost, NextEpoch]
        pub main: Option<String>,
        /// The model for utility calls: summaries, titles. Without one, the
        /// main model is used. From a project, after trust.
        [RaisesCost, NextEpoch]
        pub fast: Option<String>,
        /// The model for model-graded comparisons. Without one, the main
        /// model is used. From a project, after trust.
        [RaisesCost, NextEpoch]
        pub judge: Option<String>,
        /// The reasoning level of the main role: one of the model's own
        /// levels, or `minimal`, `low`, `medium`, `high` or `max`. From a
        /// project, after trust: more reasoning is more output.
        [RaisesCost, NextEpoch]
        pub effort: Option<String>,
        /// How much of the model's reasoning is shown.
        [Preference, NextEpoch]
        pub reasoning: ReasoningVisibility = ReasoningVisibility::Summary,
        /// The most output tokens one response may have. Without it, the
        /// model's own limit applies. From a project, after trust.
        [RaisesCost, NextEpoch, 1..=10_000_000]
        pub max_output: Option<u64>,
        /// The session's speed tier, which the main role runs at:
        /// `standard`, `fast` where the model offers it, or a provider's own
        /// tier name. The utility roles do not follow it: they run at
        /// `standard` unless `model.roles.<role>.speed` says otherwise. A
        /// project's file may set `standard`; any other tier costs more,
        /// and waits for the user to trust the project.
        [BySpeed, Now]
        pub speed: Speed = Speed::Standard,
        /// Whether a request that a speed tier refuses is sent again at
        /// `standard`. When off, such a refusal is an ordinary rate limit.
        [Preference, Now]
        pub speed_fallback: bool = true,
        /// The longest one request waits in place for a speed tier before it
        /// falls back to `standard`. `"0s"` never waits. At most two
        /// minutes: past that, `standard` answers sooner than the tier would,
        /// whatever keeping it saves. A project's file may lower it and may
        /// not raise it: every wait is the session's.
        [TightenOnly, Now, 0s..=2m]
        pub speed_wait: Duration = Duration::from_secs(20),
        /// How long requests stay at `standard` after a fallback, when the
        /// provider does not say when the tier is available again. From one
        /// minute, the window a provider counts its limits in, so that a
        /// session does not change speed (and perhaps write its cache again)
        /// every few requests; to an hour, past which a tier that keeps
        /// refusing is better turned off with `model.speed`.
        [Preference, Now, 1m..=1h]
        pub speed_cooldown: Duration = Duration::from_secs(600),
        /// The least time requests stay at `standard` after a fallback when
        /// the provider does say when the tier is available again: its time,
        /// or this long, whichever is later. `"0s"` takes the provider's
        /// time as given. At most an hour, as `speed_cooldown` is.
        [Preference, Now, 0s..=1h]
        pub speed_cooldown_min: Duration = Duration::from_secs(60),
        /// Settings of one role: `main`, `fast` or `judge`.
        [nested]
        pub roles: BTreeMap<String, RoleSettings>,
    }
}

settings! {
    /// The settings of one model role.
    pub struct RoleSettings {
        /// The provider that serves the role's model. Needed only when two
        /// providers have a model with the same name. From a project, after
        /// trust: it chooses the model as much as the name does.
        [RaisesCost, NextEpoch]
        pub provider: Option<String>,
        /// The role's reasoning level. For `main` it overrides
        /// `model.effort`. From a project, after trust.
        [RaisesCost, NextEpoch]
        pub effort: Option<String>,
        /// The role's speed tier. For `main` it overrides `model.speed`.
        /// From a project's file, a tier other than `standard` waits for
        /// trust.
        [BySpeed, Now]
        pub speed: Option<Speed>,
    }
}

settings! {
    /// One hook command (`docs/architecture/12-extensions.md`, "Tier 3:
    /// hooks"). Its program is started for each event it runs on, its start
    /// decided by the permission engine as an `exec` of its line in the
    /// workspace; it is given the event as JSON on standard input, and its
    /// answer is read from standard output.
    pub struct HookSettings {
        /// The event it runs on: `session.start`, `prompt.submit`,
        /// `tool.before`, `tool.after`, `turn.end`, `compaction.before` or
        /// `session.end`. Needed: an entry without one is refused.
        [KeptApart, NextSession]
        pub event: Option<HookEventName>,
        /// For `tool.before` and `tool.after` only: the tools it runs on,
        /// each a tool's name or `source/*` for every tool of a source. A
        /// `tool_call` is matched by `tool_call` and by the `source/tool` it
        /// names. Empty: every tool.
        [KeptApart, NextSession]
        pub tools: Vec<String>,
        /// The program and its arguments, each one argument: the first a
        /// bare name, found in `PATH`'s absolute directories outside the
        /// workspace, or an absolute path. An argument that is a whole
        /// placeholder is filled: `{path}` with each file within the
        /// workspace that the call reads or writes, one argument each (a
        /// folder never fills it), `{tool}`, `{workspace}`, `{event}`. An
        /// entry that uses `{path}` does not run for a call that names no
        /// file within the workspace. A placeholder is never part of an
        /// argument: `--file={path}` refuses the entry. At most 256
        /// arguments.
        [KeptApart, NextSession]
        pub command: Vec<String>,
        /// Whether it only observes: never waited for, its answer not read,
        /// its failure logged.
        [KeptApart, NextSession]
        pub observe: bool = false,
        /// How long it may run before its process tree is stopped: 5
        /// seconds by default for a hook that is waited for, at most a
        /// minute; 60 seconds by default for an observer, at most ten
        /// minutes.
        [KeptApart, NextSession, 1ms..=10m]
        pub timeout: Option<Duration>,
        /// Whether it may write the workspace where commands are confined.
        /// Without it, it reads what the session's commands may read and
        /// writes only the session's temporary folder; with it, never at
        /// `read-only`. What it writes is not recorded in the round's
        /// checkpoint, so undo does not put it back.
        [KeptApart, NextSession]
        pub write: bool = false,
        /// Whether it may reach the network where commands are confined.
        [KeptApart, NextSession]
        pub network: bool = false,
        /// Whether it runs. `false` keeps it out of new sessions.
        [KeptApart, NextSession]
        pub enabled: bool = true,
    }
}

settings! {
    /// One extension's entry. An extension's capabilities are its
    /// manifest's, approved with it, and are not set here.
    pub struct ExtensionSettings {
        /// Whether the user's own extension of this name is loaded. `false`
        /// keeps it out of every new session, its tools, commands and
        /// prompt with it; its pin is kept, so `true` again needs no new
        /// approval unless its folder changed. A project's extension of the
        /// name is not turned off by it: a project's is kept out by not
        /// trusting the project.
        [UserOnly, NextSession]
        pub enabled: bool = true,
    }
}

settings! {
    /// One provider: an endpoint that speaks a protocol.
    pub struct ProviderSettings {
        /// Whether the provider is available. `false` removes an entry that
        /// a lower layer or the built-in table defines.
        [UserOnly, NextEpoch]
        pub enabled: bool = true,
        /// The wire protocol the endpoint speaks.
        [UserOnly, NextEpoch]
        pub protocol: Option<String>,
        /// The endpoint's base URL.
        [UserOnly, NextEpoch]
        pub base_url: Option<String>,
        /// The environment variable that holds the provider's API key. It
        /// is looked at before the credential store. A built-in provider
        /// that takes a key has one already (`ANTHROPIC_API_KEY`,
        /// `OPENAI_API_KEY` and so on). A provider with a key variable
        /// and no key anywhere is refused before anything is sent. One
        /// with none, as the built-in local servers have none, is asked
        /// without a key unless one is stored for it.
        [UserOnly, Now]
        pub key_env: Option<String>,
        /// Extra headers for every request. A value is text, or
        /// `{ env = "NAME" }` to read an environment variable when used.
        /// Text may be a key (some gateways take one in a header), so it is
        /// held as a secret and `cineris config show` prints it as
        /// `[redacted]`; a variable's name is shown.
        [UserOnly, Now]
        pub headers: Headers,
        /// How long a response stream may be silent before the request
        /// fails. Overrides `retry.idle_timeout` for this provider, within
        /// the same range: 30 seconds to an hour.
        [UserOnly, Now, 30s..=1h]
        pub idle_timeout: Option<Duration>,
        /// How long to wait for the first event of a response. Overrides
        /// `retry.first_event_timeout` for this provider, within the same
        /// range: 30 seconds to an hour.
        [UserOnly, Now, 30s..=1h]
        pub first_event_timeout: Option<Duration>,
        /// Capability flags of the endpoint, as the provider's protocol
        /// names them. For `anthropic-messages`: `replay_reasoning`
        /// (`"all"` for a server that leaves out the reasoning a model
        /// cannot read, as Anthropic's own does), `eager_tool_streaming`,
        /// `api_version` and `beta`. Each value is text, a whole number or
        /// `true`/`false`.
        [reserved UserOnly]
        pub capabilities: Reserved,
    }
}

settings! {
    /// How a request that failed is tried again.
    pub struct RetrySettings {
        /// The most attempts for one request, the first included. From a
        /// project, after trust: each attempt can be paid for.
        [RaisesCost, Now, 1..=20]
        pub attempts: u32 = 5,
        /// The wait before the second attempt. Each later wait is this
        /// multiplied by `backoff_factor` again, up to `backoff_cap`. From
        /// 100 ms, below which the attempts are spent before a brief outage
        /// can pass, to a minute. A project's file may lower it and may not
        /// raise it.
        [TightenOnly, Now, 100ms..=1m]
        pub backoff_base: Duration = Duration::from_secs(1),
        /// What the wait is multiplied by after each attempt. A project's
        /// file may lower it and may not raise it: a larger one makes every
        /// later wait longer.
        [TightenOnly, Now, 1..=10]
        pub backoff_factor: f64 = 2.0,
        /// The longest wait between two attempts. From a second, so that the
        /// attempts span more than a moment, to five minutes, past which a
        /// provider is down rather than busy. A project's file may lower it
        /// and may not raise it.
        [TightenOnly, Now, 1s..=5m]
        pub backoff_cap: Duration = Duration::from_secs(30),
        /// The longest wait a provider may ask for. A longer one ends the
        /// turn with the error and the provider's hint. From a minute, the
        /// window a provider counts its limits in, so that no file can make
        /// an ordinary rate limit end a turn, to an hour, past which the wait
        /// is a quota and not a rate limit. A project's file may lower it and
        /// may not raise it.
        [TightenOnly, Now, 1m..=1h]
        pub max_wait: Duration = Duration::from_secs(120),
        /// How long a response stream may be silent before the request
        /// fails. From 30 seconds, within which a hosted provider that is
        /// still working sends a token or a ping, to an hour, past the five
        /// minutes the built-in entries give a local server. A project's
        /// file may lower it and may not raise it.
        [TightenOnly, Now, 30s..=1h]
        pub idle_timeout: Duration = Duration::from_secs(90),
        /// How long to wait for the first event of a response. From 30
        /// seconds, below which a long prompt would fail every time, to an
        /// hour, past the ten minutes the built-in entries give a local
        /// server. A project's file may lower it and may not raise it.
        [TightenOnly, Now, 30s..=1h]
        pub first_event_timeout: Duration = Duration::from_secs(60),
    }
}

settings! {
    /// The permission mode and the permission rules.
    pub struct PermissionSettings {
        /// The session's level: how much runs without asking.
        [ByLevel, Now]
        pub mode: Level = Level::Supervised,
        /// Permission rules. Every layer adds to the list; `deny` beats
        /// `ask` beats `allow`, whatever the order.
        [ByRuleAction, Now, joined]
        pub rules: Vec<Rule>,
        /// Let the model read the files of your own skills without asking:
        /// those found in your home directory's skill folders, wherever
        /// they lead (never in or around the workspace). Secret-looking
        /// files there, such as a `.env` a skill's scripts use, are read
        /// too, and sent to the model's provider. Off: each such read is
        /// asked about. The user's alone: a project's file may not set it.
        [UserOnly, NextSession]
        pub read_skill_folders: bool = false,
    }
}

settings! {
    /// What a command the harness starts may touch.
    pub struct SandboxSettings {
        /// Whether commands are confined by the OS sandbox where there is
        /// one.
        [UserOnly, NextSession]
        pub enabled: bool = true,
        /// Paths a command may write besides the workspace. Every layer
        /// adds to the list.
        [WidensAccess, Now, joined]
        pub write: Vec<String>,
        /// Paths a command may not change, besides the built-in ones. Every
        /// layer adds to the list.
        [Preference, NextSession, joined]
        pub protect: Vec<String>,
        /// Whether a command may use the network. `off`: no TCP or UDP,
        /// to this machine's own addresses included, and no Unix socket,
        /// so a test suite that starts a server on loopback or on a Unix
        /// socket fails in the sandbox; such a run needs `full`, or one
        /// approved run outside the sandbox. `full`: the network (TCP and
        /// UDP, loopback included), and nothing else: still no Unix socket
        /// (the session bus, an agent, the Docker socket), no virtual
        /// machine's socket to its host, no raw socket.
        [ByNetwork, Now]
        pub network: NetworkAccess = NetworkAccess::Off,
    }
}

settings! {
    /// The most a turn, a session and a day may spend. A limit that is not
    /// set does not apply.
    pub struct LimitSettings {
        /// Limits on one turn.
        [nested]
        pub turn: LimitSet,
        /// Limits on one session.
        [nested]
        pub session: LimitSet,
        /// Limits on one local day, across every session of every process
        /// on this machine, counted from the statistics: with
        /// `stats.enabled = false` a session with a day's limit is refused
        /// when it opens. Another process's requests are counted once they
        /// end, so the ones in flight when the limit is reached can pass
        /// it (`docs/architecture/18-usage.md`, "Limits on one day").
        [nested]
        pub day: LimitSet,
    }
}

settings! {
    /// One set of limits.
    pub struct LimitSet {
        /// Requests to a model.
        [TightenOnly, Now, 1..=1_000_000_000]
        pub requests: Option<u64>,
        /// Tokens, input and output together.
        [TightenOnly, Now, 1..=1_000_000_000_000_000]
        pub tokens: Option<u64>,
        /// Money, in US dollars.
        [TightenOnly, Now, 0..=1_000_000_000]
        pub cost: Option<f64>,
        /// Wall-clock time. Held for a turn and a session; a day's is
        /// accepted and not held yet
        /// (`docs/outstanding/a-days-time-limit.md`).
        [TightenOnly, Now]
        pub time: Option<Duration>,
    }
}

settings! {
    /// Where stored credentials are kept.
    pub struct CredentialSettings {
        /// The store for API keys saved with `cineris auth login`.
        [UserOnly, Restart]
        pub store: StoreChoice = StoreChoice::Auto,
    }
}

settings! {
    /// Where sessions are stored.
    pub struct SessionSettings {
        /// The data directory. A leading `~` is the home directory.
        /// Without it the platform's usual place is used.
        [UserOnly, Restart]
        pub data_dir: Option<String>,
        /// When the session log is flushed to the disk: after every item
        /// (`item`), at the end of every turn (`turn`), or never, leaving
        /// it to the operating system (`never`). The user's alone: a
        /// project that could choose `never` could cost the user their
        /// sessions. A session already open keeps what it was opened with.
        [UserOnly, Restart]
        pub durability: SessionDurability = SessionDurability::Item,
    }
}

settings! {
    /// When the conversation is compacted, and which instruction files are
    /// read (`docs/architecture/07-sessions-and-context.md`).
    pub struct ContextSettings {
        /// Whether the conversation is compacted by itself when a request
        /// would pass `compact_at`, or the provider says a request does not
        /// fit. Off, the turn stops there and asks for a compaction
        /// (`session/compact`) or a new session.
        [Preference, Now]
        pub auto_compact: bool = true,
        /// Whether a summary is prepared in the background from
        /// `prepare_at`, on the `fast` role's model (the main model when
        /// there is none), so that the compaction need not wait for it.
        /// The user's alone: off, every compaction waits at the trigger for
        /// its summary, and a repository's file must not make the sessions
        /// in it wait.
        [UserOnly, Now]
        pub background: bool = true,
        /// The trigger: the fraction of what a request may hold (the
        /// model's context window less room for the answer and a margin)
        /// past which the conversation is compacted before the request.
        /// Never below what a compaction needs, the prompt and the tools
        /// and room for a summary: a lower value is raised to that, and
        /// the user is told. From a project, after trust: a compaction
        /// costs a request.
        [RaisesCost, Now, 0..=1]
        pub compact_at: f64 = 0.85,
        /// The fraction past which a summary is prepared in the
        /// background. From a project, after trust.
        [RaisesCost, Now, 0..=1]
        pub prepare_at: f64 = 0.7,
        /// How much of the most recent conversation a compaction replays
        /// after its summary, without its reasoning, in tokens. 0 replays
        /// nothing. At most half of what the trigger leaves once the prompt
        /// and the tools are counted, and the user is told when that holds
        /// it back. From a project, after trust.
        [RaisesCost, NextEpoch, 0..=10_000_000]
        pub keep_recent_tokens: u64 = 0,
        /// More names of instruction files, read after `AGENTS.md` and
        /// `CLAUDE.md` in each directory. From a project, after trust:
        /// what they hold is sent with every request.
        [RaisesCost, NextEpoch]
        pub instruction_names: Vec<String>,
        /// The most the instruction files may hold in all, in bytes. Past
        /// it the innermost files are cut, and the user is warned. From a
        /// project, after trust.
        [RaisesCost, NextEpoch, 0..=100_000_000]
        pub instructions_cap: u64 = 49_152,
    }
}

settings! {
    /// The project profile and memory
    /// (`docs/architecture/14-adaptation.md`, "Project knowledge").
    pub struct ProjectSettings {
        /// Whether the harness learns from the commands a session runs: a
        /// build, test or check command that succeeds twice becomes an
        /// entry of the project profile, and a test command seen to fail
        /// and then pass with nothing changed a flaky one. Off, nothing is
        /// observed; what is already known is used as before, and
        /// `remember` and the user's own entries work. The user's alone: a
        /// repository's file does not decide what is kept about it.
        [UserOnly, Restart]
        pub learn: bool = true,
        /// The budget of the project profile's section of the system
        /// prompt, in tokens, counted at three bytes a token. Entries past
        /// it are left out, commands last, and the section says how many.
        /// A project's file may lower it and may not raise it: the section
        /// is sent with every request.
        [TightenOnly, NextEpoch, 0..=8_000]
        pub profile_tokens: u64 = 1_500,
    }
}

settings! {
    /// The project's checks, run at the end of a turn that changed files
    /// (`docs/architecture/15-efficiency.md`, "Automatic verification").
    /// They run only in a project the user trusts, and never at the
    /// `read-only` level.
    pub struct VerifySettings {
        /// `auto`: the checks run, and a failure goes back to the model.
        /// `ask`: they are offered, and run when the user asks. `off`: they
        /// run only when the user asks. A project's file may turn it down,
        /// and may not turn it up.
        [ByVerifyMode, Now]
        pub mode: VerifyMode = VerifyMode::Auto,
        /// The user's own checks, each a command line run in the workspace
        /// root, before the accepted test and check commands of the project
        /// profile. Every one of the user's layers adds to the list. The
        /// user's alone: a repository proposes its checks as command
        /// entries of `.cineris/profile.toml`, which each user accepts.
        [UserOnly, Now, joined]
        pub commands: Vec<String>,
        /// The most rounds of checks one turn gets: a failure is given to
        /// the model, and the checks run again when it has changed files,
        /// until this many have run. A project's file may lower it.
        [TightenOnly, Now, 1..=5]
        pub rounds: u64 = 2,
        /// The time one round's checks may take together. From ten
        /// seconds to an hour. A project's file may lower it.
        [TightenOnly, Now, 10s..=1h]
        pub round_time: Duration = Duration::from_secs(120),
        /// The time one turn's checks may take, every round together. From
        /// ten seconds to four hours. A project's file may lower it.
        [TightenOnly, Now, 10s..=4h]
        pub turn_time: Duration = Duration::from_secs(240),
    }
}

settings! {
    /// The repository map and the index it is made from
    /// (`docs/architecture/15-efficiency.md`, "Repository map").
    pub struct IndexSettings {
        /// Whether each context epoch is given the repository map: the
        /// workspace's directories and its most relevant files with their
        /// main definitions. The `outline` tool works either way.
        [Preference, NextSession]
        pub map: bool = true,
        /// The map's budget, in tokens. A project's file may lower it and
        /// may not raise it: the map is sent with every request of an
        /// epoch.
        [TightenOnly, NextEpoch, 0..=50_000]
        pub map_tokens: u64 = 1_500,
        /// How long the first request of an epoch waits for the map when it
        /// is not ready yet; past it, the map follows as a notice at the next
        /// boundary where it is. Unset, 150 ms in the interactive app and
        /// none in a headless run, whose start it would otherwise slow. At
        /// most one second. The user's alone: the wait is paid by every
        /// epoch's first request, and a repository's file must not make
        /// the sessions in it wait (found by the independent review of step
        /// 18).
        [UserOnly, NextSession, 0s..=1s]
        pub map_wait: Option<Duration>,
        /// The most files the index reads. Past it, the files nearest the
        /// top of the tree are indexed, and the map says it is partial.
        [TightenOnly, Restart, 1..=10_000_000]
        pub max_files: u64 = 20_000,
        /// The largest file the index reads, in bytes. A larger one is
        /// listed and not outlined, unless the `outline` tool is asked
        /// for it.
        [TightenOnly, Restart, 1..=1_073_741_824]
        pub max_file_bytes: u64 = 1_048_576,
        /// The most time one pass of the index spends reading files, after
        /// its listing (which its cap on entries bounds). One that runs out
        /// carries on when the session is next idle. At least 250 ms is
        /// used, whatever is set: less would make an idle session pass over
        /// the workspace again for every few files.
        [TightenOnly, Restart]
        pub pass_time: Duration = Duration::from_secs(2),
    }
}

settings! {
    /// Which files written for other tools are read. They are read and
    /// never changed.
    pub struct CompatSettings {
        /// Instruction files under the names other tools use, `AGENTS.md`
        /// and `CLAUDE.md`, in the workspace. Off, only the names in
        /// `context.instruction_names` are read there; the user's own files
        /// in the configuration directory are read either way.
        [UserOnly, NextSession]
        pub instructions: bool = true,
        /// MCP server definitions of other clients, offered for import.
        [UserOnly, NextSession]
        pub mcp: bool = true,
        /// Agent Skills folders: `.agents/skills` and `.claude/skills`.
        [UserOnly, NextSession]
        pub skills: bool = true,
    }
}

settings! {
    /// The shell that runs commands, the time a command may take, and the
    /// environment variables it is not given.
    pub struct ShellSettings {
        /// The shell: a program's name on `PATH`, or its path. How it is
        /// spoken to follows its name: `bash`, `pwsh`, `powershell` and
        /// `cmd` each by their own rules, and any other as a POSIX shell.
        /// Unset, it is bash and then sh on Linux, and on Windows
        /// PowerShell 7, then Windows PowerShell, then cmd.exe. A
        /// project's file may not choose the program that runs every
        /// command.
        [UserOnly, NextSession]
        pub program: Option<String>,
        /// The arguments before the command, in place of the shell's own:
        /// `["-l", "-c"]` for a login bash. The command is always passed
        /// after them, last.
        [UserOnly, NextSession]
        pub args: Option<Vec<String>>,
        /// The time limit of a command that asks for none, held to
        /// `max_timeout`. From a second, below which every command would
        /// fail, to half an hour, past which a command that hangs holds up
        /// its turn too long. A project's file may lower it and may not
        /// raise it.
        [TightenOnly, NextSession, 1s..=30m]
        pub timeout: Duration = Duration::from_secs(120),
        /// The longest time limit a command may ask for. From ten seconds,
        /// below which a build or a test run could never finish, to a day.
        /// A project's file may lower it and may not raise it.
        [TightenOnly, NextSession, 10s..=1d]
        pub max_timeout: Duration = Duration::from_secs(30 * 60),
        /// Environment variables that commands are not given, besides
        /// the key variables of the providers, which are always removed.
        /// A name may end with `*` for every variable that starts so.
        /// Every layer adds to the list. A project's entries wait for
        /// trust: `"*"` would take `PATH` away and stop every command.
        [StopsCommands, NextSession, joined]
        pub env_remove: Vec<String>,
    }
}

settings! {
    /// How the harness adapts to each model
    /// (`docs/architecture/14-adaptation.md`, "Adapting to the model").
    pub struct AdaptSettings {
        /// Variant overrides by model: `[adapt.variants."claude-opus-5-5"]`
        /// with `edit_format`, `tool_schema`, `prompt_style` or `effort`.
        /// An override wins over the catalog's default for that model, and
        /// applies from the next context epoch that builds a prompt. A
        /// choice the build does not know stops the session from starting,
        /// with the choices it has. The user's alone.
        [nested]
        pub variants: BTreeMap<String, VariantSettings>,
    }
}

settings! {
    /// The choices at each variant point for one model. A point left out
    /// keeps the catalog's default.
    pub struct VariantSettings {
        /// The edit format: `replace`.
        [UserOnly, NextEpoch]
        pub edit_format: Option<String>,
        /// How tools are declared: `strict` (the provider checks the
        /// arguments, where the model can) or `loose` (the harness alone).
        [UserOnly, NextEpoch]
        pub tool_schema: Option<String>,
        /// The prompt style: `concise`, or `guided` for more guidance on the
        /// tools.
        [UserOnly, NextEpoch]
        pub prompt_style: Option<String>,
        /// The reasoning level the model runs at unless a role sets its own:
        /// one of its levels, or an alias. `model.effort` and a role's
        /// `effort` win over it.
        [UserOnly, NextEpoch]
        pub effort: Option<String>,
    }
}

settings! {
    /// Local statistics (`docs/architecture/14-adaptation.md`, "Local
    /// statistics"). Nothing of them leaves the machine.
    pub struct StatsSettings {
        /// Whether events are recorded. Off, nothing is written, every
        /// adaptive feature uses its default, and the usage view has the
        /// open session's figures alone.
        [UserOnly, Restart]
        pub enabled: bool = true,
        /// How many months a monthly file of events stays after its own
        /// month: 3 by default. What it added up to stays in the
        /// aggregates, every all-time figure with it, so only the raw
        /// events go (a heavy user's are about 90 MB a month). Read each
        /// time the files are pruned, so a reload changes it at once.
        [UserOnly, Now, 1..=1200]
        pub retention: u32 = 3,
    }
}

settings! {
    /// The usage view (`docs/architecture/18-usage.md`).
    pub struct UsageSettings {
        /// The first day of a week: `monday` to `sunday`.
        [UserOnly, Now]
        pub week_start: Weekday = Weekday::Monday,
        /// `calendar` weeks and months, or `rolling`: the last 7 and 30
        /// days.
        [UserOnly, Now]
        pub ranges: UsageRanges = UsageRanges::Calendar,
        /// The providers whose documented usage endpoint
        /// `cineris usage --refresh` may ask, with that provider's own key:
        /// `["openrouter"]`. Empty, nothing is ever asked. Only when the
        /// user asks, never on its own. The user's alone: a project's file
        /// cannot turn it on.
        [UserOnly, Now]
        pub refresh: Vec<String>,
    }
}

settings! {
    /// The terminal UI: how the transcript is drawn
    /// (`docs/architecture/09-tui.md`). Every key is the user's own: a
    /// repository does not choose how the user's terminal looks.
    pub struct TuiSettings {
        /// The widest prose may be, in columns, its indent included. The
        /// window's width when that is less.
        [UserOnly, Now, 20..=1000]
        pub prose_width: u64 = 80,
        /// The most diff lines a tool's box opens on by itself, and the most
        /// rows of detail an open box shows while its round runs.
        [UserOnly, Now, 1..=1000]
        pub tool_detail: u64 = 12,
        /// Which tool detail starts open: `auto` (a failure's output and a
        /// diff of up to `tool_detail` lines), `always` or `never`.
        [UserOnly, Now]
        pub tool_output: ToolOutputChoice = ToolOutputChoice::Auto,
        /// What finished reasoning leaves in the transcript: its `summary`,
        /// or only how long it took (`duration`).
        [UserOnly, Now]
        pub reasoning: ReasoningShown = ReasoningShown::Summary,
        /// A blank line between blocks (`comfortable`), or only between
        /// turns (`compact`).
        [UserOnly, Now]
        pub density: DensityChoice = DensityChoice::Comfortable,
        /// A dim line after each turn with its time, tokens and cost.
        [UserOnly, Now]
        pub turn_summary: bool = false,
        /// A still mark in place of the spinner.
        [UserOnly, Now]
        pub reduced_motion: bool = false,
        /// The glyph set: `auto` (Unicode where the terminal can show it),
        /// `unicode` or `ascii`. `ascii` suits a terminal that draws the
        /// box's edges and the marks of East Asian ambiguous width wide.
        [UserOnly, Now]
        pub glyphs: GlyphChoice = GlyphChoice::Auto,
        /// The theme: `auto` (dark or light, from the terminal's
        /// background), `dark` or `light`. A name the interface does not
        /// know is said, and `auto` is used.
        [UserOnly, Now]
        pub theme: String = "auto".to_owned(),
        /// How the interface draws: `inline` (the transcript in the
        /// terminal's own scrollback) or `fullscreen` (the whole window, on
        /// the alternate screen, scrolled and searched there).
        /// `--fullscreen` chooses the second for one run.
        [UserOnly, Restart]
        pub presenter: PresenterChoice = PresenterChoice::Inline,
    }
}

settings! {
    /// The terminal UI's key bindings, by the context an action belongs to
    /// (`docs/architecture/09-tui.md`, "The editor as built"). Each table
    /// holds action names and the keys bound to them; a list replaces the
    /// action's defaults and an empty one unbinds it. An entry the
    /// interface cannot use is said, and the action keeps its defaults.
    pub struct KeySettings {
        /// The editor's actions: `submit`, `new_line`, `undo`, and the rest.
        [UserOnly, Now]
        pub editor: Bindings,
        /// A completion menu's.
        [UserOnly, Now]
        pub menu: Bindings,
        /// A picker's.
        [UserOnly, Now]
        pub picker: Bindings,
        /// The `/effort` picker's fast-mode toggle.
        [UserOnly, Now]
        pub toggle: Bindings,
        /// The fork of the pickers that fork (`/rewind`'s, `/sessions`'):
        /// `fork`.
        [UserOnly, Now]
        pub fork: Bindings,
        /// The history search's.
        [UserOnly, Now]
        pub search: Bindings,
        /// A focused tool box's or panel's.
        [UserOnly, Now]
        pub focus: Bindings,
        /// A dialog's: the answers to an approval, a question or trust.
        [UserOnly, Now]
        pub dialog: Bindings,
        /// The viewer's (`Ctrl+O`): moving between boxes, opening one,
        /// scrolling, searching, and the mouse's `click`, `wheel_up` and
        /// `wheel_down`.
        [UserOnly, Now]
        pub viewer: Bindings,
        /// The full-screen presenter's: scrolling, searching, and the
        /// mouse's `click`, `wheel_up` and `wheel_down`.
        [UserOnly, Now]
        pub screen: Bindings,
        /// Those looked up last, whatever has the keyboard: `interrupt`,
        /// `redraw`, `suspend`, and the rest.
        [UserOnly, Now]
        pub global: Bindings,
    }
}

settings! {
    /// What the terminal can do, in place of detecting it
    /// (`docs/architecture/09-tui.md`, "Terminal layer"). A key that is not
    /// set is detected.
    pub struct TerminalSettings {
        /// Synchronized updates: each frame shown at once.
        [UserOnly, Restart]
        pub sync: Option<bool>,
        /// The enhanced keyboard protocol, which tells `Shift+Enter` from
        /// `Enter`.
        [UserOnly, Restart]
        pub keyboard: Option<bool>,
        /// How many colours: `none`, `16`, `256` or `truecolor`.
        [UserOnly, Restart]
        pub color: Option<ColorChoice>,
        /// Links in text.
        [UserOnly, Restart]
        pub hyperlinks: Option<bool>,
        /// Unicode, or ASCII only.
        [UserOnly, Restart]
        pub unicode: Option<bool>,
        /// The mouse in the viewer and the full-screen presenter: a click
        /// opens or closes a box, and the wheel scrolls; `shift` and a drag
        /// still select text. Never in the inline screen. On unless set to
        /// `false`.
        [UserOnly, Restart]
        pub mouse: Option<bool>,
        /// What byte 0x08 is: `ctrl+h` reads it as Backspace, for a terminal
        /// whose Backspace sends `^H`; `del` as `Ctrl+Backspace`, which most
        /// terminals send it for; `auto` decides from the terminal's erase
        /// character.
        [UserOnly, Restart]
        pub backspace: BackspaceChoice = BackspaceChoice::Auto,
        /// Bracketed paste: `true` trusts the terminal's brackets; `false`
        /// does not ask for them, and takes keys that come many at once with
        /// a line break inside for a paste; unset, it asks, and takes such a
        /// burst for a paste only where the terminal has not said it knows
        /// the mode.
        [UserOnly, Restart]
        pub bracketed_paste: Option<bool>,
        /// How long a bracketed paste may pause before what it holds is
        /// given as a paste; what follows of it, to its end, is its rest.
        [UserOnly, Restart, 100ms..=1m]
        pub paste_wait: Duration = Duration::from_secs(2),
        /// Whether the terminal takes the clipboard sequence `/copy` sends:
        /// `false` also shows the text in the viewer, to select and copy by
        /// hand. Unset, it is guessed from the terminal: not the old console
        /// host, tmux or GNU screen.
        [UserOnly, Restart]
        pub clipboard: Option<bool>,
    }
}

settings! {
    /// What is logged.
    pub struct LogSettings {
        /// The level (`error`, `warn`, `info`, `debug` or `trace`),
        /// optionally followed by levels for target prefixes:
        /// `"info,cineris_llm=debug"`. The `CINERIS_LOG` variable and
        /// `--log-level` override it.
        [UserOnly, Now]
        pub level: Option<String>,
    }
}

settings! {
    /// The RPC server (`cineris serve`, `docs/architecture/11-rpc-protocol.md`).
    /// Read once, when the server starts, which loads no workspace: the
    /// user's alone.
    pub struct RpcSettings {
        /// How long an approval waits for a client's answer before it is a
        /// denial (`"0s"`: for as long as it takes). The terminal UI is not
        /// a server and sets none.
        [UserOnly, Restart, 0s..=1d]
        pub approval_timeout: Duration = Duration::from_secs(600),
        /// How long a client may take nothing the server sends while whole
        /// events wait for it past its queue, before its connection is
        /// closed. The session goes on.
        [UserOnly, Restart, 1s..=1h]
        pub stalled_client: Duration = Duration::from_secs(30),
    }
}

impl LimitSet {
    /// The money limit in nano-dollars, the unit the protocol's budgets
    /// use.
    #[must_use]
    pub fn cost_usd_nanos(&self) -> Option<u64> {
        self.cost.map(|dollars| {
            let nanos = (dollars * 1e9).round();
            if nanos <= 0.0 {
                0
            } else if nanos >= u64::MAX as f64 {
                u64::MAX
            } else {
                nanos as u64
            }
        })
    }

    /// Whether no limit is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requests.is_none()
            && self.tokens.is_none()
            && self.cost.is_none()
            && self.time.is_none()
    }
}

impl ModelSettings {
    /// The model configured for a role, without the fallback to the main
    /// model that an unset utility role has.
    #[must_use]
    pub fn model_for(&self, role: &ModelRole) -> Option<&str> {
        match role {
            ModelRole::Main => self.main.as_deref(),
            ModelRole::Fast => self.fast.as_deref(),
            ModelRole::Judge => self.judge.as_deref(),
            ModelRole::Unknown(_) => None,
        }
    }

    /// The speed a role is set to: its own setting; without one, the
    /// session's speed for `main` and `standard` for every other role.
    ///
    /// The utility roles do not inherit a fast session speed (decision
    /// 0027): summaries and titles gain little from speed, and an unset
    /// `fast` role falls back to the main model, so an inherited fast speed
    /// would double the price of every compaction.
    #[must_use]
    pub fn speed_for(&self, role: &ModelRole) -> Speed {
        if let Some(speed) = self
            .roles
            .get(role.as_str())
            .and_then(|settings| settings.speed.clone())
        {
            return speed;
        }
        if *role == ModelRole::Main {
            self.speed.clone()
        } else {
            Speed::Standard
        }
    }

    /// The reasoning level a role is set to: its own setting; without one,
    /// `model.effort` for `main` and nothing for every other role.
    #[must_use]
    pub fn effort_for(&self, role: &ModelRole) -> Option<&str> {
        let own = self
            .roles
            .get(role.as_str())
            .and_then(|settings| settings.effort.as_deref());
        match own {
            Some(effort) => Some(effort),
            None if *role == ModelRole::Main => self.effort.as_deref(),
            None => None,
        }
    }

    /// The provider a role's model is pinned to, when one is.
    #[must_use]
    pub fn provider_for(&self, role: &ModelRole) -> Option<&str> {
        self.roles
            .get(role.as_str())
            .and_then(|settings| settings.provider.as_deref())
    }
}

impl Settings {
    /// The retry settings for one provider: `[retry]`, with the two
    /// timeouts that provider's entry overrides.
    #[must_use]
    pub fn retry_for(&self, provider: &str) -> RetrySettings {
        let mut retry = self.retry.clone();
        if let Some(entry) = self.providers.get(provider) {
            if let Some(timeout) = entry.idle_timeout {
                retry.idle_timeout = timeout;
            }
            if let Some(timeout) = entry.first_event_timeout {
                retry.first_event_timeout = timeout;
            }
        }
        retry
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Setting, defaults};
    use super::*;

    fn settings(text: &str) -> Settings {
        let table: toml::Table = text.parse().unwrap();
        Settings::read(&toml::Value::Table(table)).unwrap()
    }

    #[test]
    fn the_defaults_are_what_the_docs_say() {
        let settings = Settings::default();
        assert_eq!(settings.config_version, 1);
        assert_eq!(settings.permissions.mode, Level::Supervised);
        assert_eq!(settings.sandbox.network, NetworkAccess::Off);
        assert!(settings.sandbox.enabled);
        assert_eq!(settings.model.speed, Speed::Standard);
        assert!(settings.model.speed_fallback);
        assert_eq!(settings.model.speed_wait, Duration::from_secs(20));
        assert_eq!(settings.model.speed_cooldown, Duration::from_secs(600));
        assert_eq!(settings.model.speed_cooldown_min, Duration::from_secs(60));
        assert_eq!(settings.retry.attempts, 5);
        assert_eq!(settings.retry.backoff_base, Duration::from_secs(1));
        assert_eq!(settings.retry.backoff_factor, 2.0);
        assert_eq!(settings.retry.backoff_cap, Duration::from_secs(30));
        assert_eq!(settings.retry.max_wait, Duration::from_secs(120));
        assert_eq!(settings.retry.idle_timeout, Duration::from_secs(90));
        assert_eq!(settings.retry.first_event_timeout, Duration::from_secs(60));
        assert_eq!(settings.credentials.store, StoreChoice::Auto);
        assert!(settings.limits.turn.is_empty() && settings.limits.day.is_empty());
        // Three months of raw events by default (the coordinator's decision
        // in step 16): a year of a heavy user's is about 1.1 GB, and every
        // all-time figure is in the aggregates.
        assert_eq!(settings.stats.retention, 3);
        assert!(settings.stats.enabled);
        assert!(settings.usage.refresh.is_empty());
        assert!(settings.compat.instructions && settings.compat.mcp && settings.compat.skills);
        assert!(settings.context.auto_compact && settings.context.background);
        assert!((settings.context.compact_at - 0.85).abs() < f64::EPSILON);
        assert!((settings.context.prepare_at - 0.7).abs() < f64::EPSILON);
        assert_eq!(settings.context.keep_recent_tokens, 0);
        assert_eq!(settings.context.instructions_cap, 48 * 1024);
        assert!(settings.context.instruction_names.is_empty());

        // Reading an empty file gives the same thing, and so does reading
        // the defaults written out.
        assert_eq!(self::settings(""), settings);
        assert_eq!(
            Settings::read(&toml::Value::Table(defaults())).unwrap(),
            settings
        );
    }

    #[test]
    fn utility_roles_run_at_standard_unless_set() {
        let fast_session = settings("[model]\nspeed = \"fast\"\neffort = \"high\"");
        assert_eq!(fast_session.model.speed_for(&ModelRole::Main), Speed::Fast);
        assert_eq!(
            fast_session.model.speed_for(&ModelRole::Fast),
            Speed::Standard,
            "a utility role does not inherit the session's speed"
        );
        assert_eq!(
            fast_session.model.speed_for(&ModelRole::Judge),
            Speed::Standard
        );
        assert_eq!(
            fast_session.model.effort_for(&ModelRole::Main),
            Some("high")
        );
        assert_eq!(fast_session.model.effort_for(&ModelRole::Fast), None);

        let per_role = settings(
            r#"
            [model]
            main = "big"
            fast = "small"
            speed = "fast"
            [model.roles.fast]
            speed = "fast"
            effort = "low"
            [model.roles.main]
            speed = "standard"
            effort = "max"
            provider = "elsewhere"
            [model.roles.judge]
            speed = "flex"
            "#,
        );
        assert_eq!(per_role.model.speed_for(&ModelRole::Fast), Speed::Fast);
        assert_eq!(
            per_role.model.speed_for(&ModelRole::Main),
            Speed::Standard,
            "a role's own speed wins over the session's"
        );
        assert_eq!(
            per_role.model.speed_for(&ModelRole::Judge),
            Speed::from_wire("flex")
        );
        assert_eq!(per_role.model.effort_for(&ModelRole::Main), Some("max"));
        assert_eq!(per_role.model.effort_for(&ModelRole::Fast), Some("low"));
        assert_eq!(per_role.model.model_for(&ModelRole::Main), Some("big"));
        assert_eq!(per_role.model.model_for(&ModelRole::Fast), Some("small"));
        assert_eq!(per_role.model.model_for(&ModelRole::Judge), None);
        assert_eq!(
            per_role.model.provider_for(&ModelRole::Main),
            Some("elsewhere")
        );
        assert_eq!(per_role.model.provider_for(&ModelRole::Fast), None);

        // A role this build has not heard of gets the safe answers.
        let other = ModelRole::from_wire("planner");
        assert_eq!(per_role.model.speed_for(&other), Speed::Standard);
        assert_eq!(per_role.model.model_for(&other), None);
    }

    #[test]
    fn a_provider_overrides_the_two_stream_timeouts() {
        let settings = settings(
            r#"
            [retry]
            attempts = 3
            idle_timeout = "2m"
            [providers.local]
            first_event_timeout = "5m"
            base_url = "http://localhost:11434"
            key_env = "LOCAL_KEY"
            headers = { X-Team = "a", Authorization = { env = "LOCAL_AUTH" } }
            [providers.off]
            enabled = false
            "#,
        );
        let local = settings.retry_for("local");
        assert_eq!(local.first_event_timeout, Duration::from_secs(300));
        assert_eq!(local.idle_timeout, Duration::from_secs(120));
        assert_eq!(local.attempts, 3);
        assert_eq!(settings.retry_for("anthropic"), settings.retry);
        assert_eq!(settings.providers["local"].headers.0.len(), 2);
        assert!(settings.providers["local"].enabled);
        assert!(!settings.providers["off"].enabled);
    }

    #[test]
    fn a_money_limit_is_whole_nano_dollars() {
        let settings = settings(
            "[limits.session]\ncost = 2.5\n[limits.day]\ncost = 10\ntokens = 2000000\ntime = \"8h\"",
        );
        assert_eq!(
            settings.limits.session.cost_usd_nanos(),
            Some(2_500_000_000)
        );
        assert_eq!(settings.limits.day.cost_usd_nanos(), Some(10_000_000_000));
        assert_eq!(settings.limits.day.tokens, Some(2_000_000));
        assert_eq!(settings.limits.day.time, Some(Duration::from_secs(28_800)));
        assert_eq!(settings.limits.turn.cost_usd_nanos(), None);
        let cents = LimitSet {
            cost: Some(0.01),
            ..LimitSet::default()
        };
        assert_eq!(cents.cost_usd_nanos(), Some(10_000_000));
        assert!(!cents.is_empty());
    }

    #[test]
    fn a_wrong_value_names_the_key_on_the_way() {
        let table: toml::Table = "[retry]\nattempts = \"five\"".parse().unwrap();
        let error = Settings::read(&toml::Value::Table(table)).unwrap_err();
        assert_eq!(
            error,
            "retry: attempts: expected a whole number that is not negative"
        );
    }
}
