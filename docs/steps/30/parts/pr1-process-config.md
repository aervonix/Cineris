# Step 30, its parts: a local server's process, and `[mcp]` in the configuration

The first of step 30's pull requests ([answered.md](../answered.md), point 7's first row): the launcher's live input, a local server started as the harness starts a program, `[mcp]` declared with a project's servers kept apart, and the configuration's servers as `cineris-mcp`'s definitions. Nothing starts a server in a session yet: the lifecycle (the fifth part) wires the spawner in.

## What was built

**A. The launcher's live input** (`cineris-sandbox`, its own commit, `6028faf8`, for its early review). `Launcher::wrap_live` puts the policy's exact length on the launcher's command line before `--` (`cineris __sandbox --policy-bytes N -- <command>`), and `Wrapped::stdin` is the policy alone, which the caller writes first. The launcher reads exactly that many bytes through a second descriptor of its standard input, with plain `read` calls and no buffer of the standard library's, so nothing after them is taken, reads them as a policy (one with an `input` field is refused), confines itself, and becomes the command with the same standard input. A length missing, not written as one (no sign, no leading zero), zero, past `MAX_BYTES`, given twice, beside any other option before `--`, or an input that ends before it, refuses the launch with the `MARK` line and `FAILED`; a length one short or one long of the policy refuses it too, since a policy must end with its `end` at its last byte. Nothing after `--` is read as an option. The sandbox's tests now start the launcher after a `--` of the test runner's, which would otherwise read `--policy-bytes` as its own.

**B. Starting a server** (`cineris-core`, behind `mcp`, `src/mcp/`): `ServerSpawner` implements `cineris_mcp::Spawner`.

- **The program** is found by `crate::programs`, shared with extensions' programs and hook commands: a bare name in `PATH`'s absolute folders outside the workspace and the write roots (on Windows `.exe`, `.cmd`, `.bat` a folder at a time, never `PATHEXT`); an absolute path as named, **refused inside the workspace or a write root whether or not the sandbox confines**; a relative path never (`Program::parse`). On Windows a `.cmd` or `.bat` runs through `%SystemRoot%\System32\cmd.exe` with `batch_line`'s refused characters and 8,000-unit limit; the look-alikes are refused everywhere.
- **Its own folder** for each start, made private under `SpawnerOptions::folders` (a folder of the harness's, never inside the workspace or a write root: the start fails if it is), holding `tmp` (its `TMPDIR`, its only write root) and `work` (an empty folder to run in); removed when the server ends, or when its start is refused.
- **Where it runs** (the maintainers' change 1): the workspace only when the project is trusted (`SpawnerOptions::trusted`); otherwise `work`. An entry's `cwd` is relative to the workspace, absolute, or under the home folder with a leading `~`; one that resolves inside the workspace counts as the workspace, and one outside it is taken as it is.
- **The explicit environment** (`server_env`, point 3.3): the base list (Unix `PATH`, `HOME`, `USER`, `LOGNAME`, `LANG`, `LC_*`, `TZ`; Windows those of point 3.3 and `HOMEDRIVE`, `HOMEPATH`), then each `EnvEntry` (`ValueSource::Env` read from the inherited environment at the start, absent refusing it naming the variable; `Literal` as it is), then the harness's own: `TMPDIR` (`TEMP` and `TMP`), on Windows `ComSpec` the system's `cmd.exe`, `PATHEXT` at its default and `NoDefaultCurrentDirectoryInExePath=1`. `PATH` keeps its absolute folders, the workspace and write roots taken out, whether it came from the base or an entry. Never a provider's key (`KNOWN_KEYS` and `SpawnerOptions::withheld`, the session's credential variables and `shell.env_remove`), a `CINERIS_` variable, or a source among them; a name that is empty, holds `=` or a NUL, begins with a digit, is `CINERIS_…`, or is one the harness sets itself refuses the start.
- **The secret filter** (the maintainers' change 3): every value read from the user's environment for a server is given by value to `SpawnerOptions::filter`, the session's shared filter, as a provider's header read from the environment is; a literal is not (as for a provider's literal header: it is in a file already, and is as often a mode as a token). The standard error's tail is shown through the same filter.
- **Confinement** where `SpawnerOptions::confine` is given (the session's sandbox enforces), at every level, `unrestricted` included (the maintainers' change 2): the launcher's live input with the session's reads, its `tmp` the only write root and `TMPDIR`, and the network only with `network = true`. A definition with `write = true` is refused (no server writes the workspace in this step). Elsewhere it runs unconfined, as a hook command does.
- **The process** (`mcp/process.rs`): `Tree::spawn` on a thread of its own that waits for the server and then ends its tree (what it left running is killed) and removes its folder; standard input a pipe a thread writes, the preamble (the policy) first and then each queued line with its line end, a line holding a line end of its own not written; standard output read as lines by `FrameDecoder` capped at 4 MiB, `LineError::TooLong` ending it; standard error always drained, its last 8 KiB kept. Both queues hold 64 lines. `shutdown(grace)` closes the input, waits `grace` on the session's timer, then interrupt, terminate (each given `STAGE_GRACE`, a command's 2 s, a stage that cannot be sent passed over) and kill, and then waits up to a second for the output to end before it stops the readers. `exited()` says how it ended in words (`exited with code 3`, `was ended by signal 2`, and for a confined server's 125 that the sandbox could not be applied). Dropping the handle kills what is left. **`shutdown` may be called twice, and from two tasks at once** (the client part's note: its reader stops a server that misbehaves, and the lifecycle then shuts the client down): one end at a time, the second waiting for the first and then returning at once, the server having ended; a call after the end returns at once.

**C. `[mcp]` declared** (`cineris-config`), replacing `Reserved`, enforced: `[mcp]` `start_timeout_ms` (10,000, 100 to 600,000) and `direct_share` (5, 0 to 100), tighten-only; `[mcp.servers.<name>]` `command`, `args`, `cwd`, `env` (a table of `EnvText`), `url`, `headers`, `network`, `description`, `enabled`, `expose` (`search` | `direct`), `tools`, `deny_tools`, `start_timeout_ms` (100 to 600,000), `timeout_ms` (60,000, 100 to 3,600,000), each kept apart. **No `write` key** (the maintainers' change 2). `cineris_config::mcp::server_problem` judges an entry whole once its layers are merged: the name (one to 64 of the ASCII letters, digits, `_`, `-`, `.`), both or neither of `command` and `url`, a key of the other kind, a variable's or header's name, a `CINERIS_` variable, a source that is a provider's key (`KNOWN_KEYS`, the configuration's providers' `key_env`), the harness's own or one of `shell.env_remove`, and **for a project's server a header not from `derived_variable(server, header)`, or written as text** (the maintainers' change 4); an entry that fails is a warning naming it, the rest loads. **A project's servers are kept apart** (`Trust::KeptApart`): never merged, read on their own into `Loaded::project_mcp_servers` (`ProjectMcpServer`: name, settings, origin, `problem`, and `for_consent()`), one whose name the managed file locks left out with a note; a reload takes the newer ones, as they are read live. The display rules: `args`, `url` and `command` by the import's rule in `config show`, every `--layer`, `config/get` and `value` (a path rule in `schema/redact.rs`, a profile's included) and in `Debug` (the keys' own types `ServerProgram`, `ServerArgs`, `ServerAddress`); `env` and `headers` literals `[redacted]` (declared `EnvText`); the trust question's view of a project's server whole by `ProjectMcpServer::for_consent`. The first run: `settable_name` is now the loader's rule, so a server named otherwise is `Refusal::Name`, and one whose variable's or header's name cannot be one, or that takes a provider's key or a `CINERIS_` variable from the environment, is `Refusal::NotUnderstood`; its writer's `check_mcp` stands, and its read-back is now strict over the declared keys. `docs/configuration/` regenerated.

**D. Definitions** (`cineris_core::mcp::definitions`): the user's servers (`Origin::User`, in name order) and the project's kept-apart ones (`Origin::ProjectConfig`), every key mapped (`start_timeout` falling back to `[mcp].start_timeout_ms`, `write` always `false`); a project's server whose name the user's configuration has, and any entry `server_problem` refuses, come back in `Definitions::refused` with why, and `Refused::notice()` says it to the user. No trust or approval judgement.

## The early review of the live input

A reviewer read commit A (`6028faf8`) alone, against the maintainers' rule that neither what is written to a server's input nor a crafted command line may change the policy, and found no way to; every refusal fails closed. What it asked for, all done in this part:

1. **Windows lint** (medium): the non-Unix `become_command` never reads `Given::Sealed`'s file, which the Windows check refuses as dead code. The field carries `#[cfg_attr(not(unix), expect(dead_code, …))]`.
2. **No test held `close_on_exec()`** (low), the only thing that keeps a stray inherited descriptor out of a server that lives all session. `confined::sys_a_descriptor_the_launcher_inherited_never_reaches_the_command` leaves a file open at descriptor 77 without close-on-exec and has a live `/bin/sh` say whether it has it ("closed") and then read a line ("got hello"); its control has the same shell, started directly, see it. Break: `stray-descriptor-reaches-the-command`.
3. **Two breaks that passed.** `Given::Closed` made the launcher's own input (at its end, so empty all the same): `sys_a_confined_command_with_no_input_reads_its_end_at_once` now asserts the input is the null device (`readlink /proc/self/fd/0`), break `closed-input-is-the-launchers-own`. The live arm narrowed to a non-empty `input`: `sys_a_live_length_that_is_not_the_policys_runs_nothing` has an empty `input` field among its cases, break `live-policy-with-an-empty-input-field-is-taken`.
4. **The caller's rules**, kept by the spawner and written in its module's documentation: the policy is written whole before any line; a launcher's 125 line never starts a server unconfined; and **a variable that tells the system's loader what to run is refused in any server's `env`**, the user's and a project's alike, at load (`cineris_config::mcp::loads_code`: every `LD_` variable, `LD_PRELOAD`, `LD_LIBRARY_PATH` and `LD_AUDIT` among them, `GLIBC_TUNABLES`, `DYLD_…`) and again at the start, and the first run imports no server that takes one; `NODE_OPTIONS` and `PYTHONPATH` are given (the approval shows them, the third part's). Every `LD_` variable, not only the four named: each is the loader's to read before `main`. Break: `loader-variable-given-to-a-server`. **The core's test launcher** had the test runner's problem with `--policy-bytes`: nothing of the core's tests uses the live mode through it yet, and its prefix now has the `--` before `LAUNCH` and `LAUNCH_GIT` all the same, for the lifecycle part; `LAUNCH_GIT`'s `rest[0] == "--"` still holds, the old mode putting `--` after the role.

**And one found while writing the tests:** a resolved path on Windows is the extended-length `\\?\C:\…` form, which `cmd.exe` cannot run in (it falls back to the Windows folder): a server's own folders and its working folder are given in the plain form.

## The full review

A reviewer read the whole part: one high finding, four medium, four low, and seven breaks no test caught. The reviewer's failing tests are folded into the crates' own (`cineris-core::it mcp_servers`, `cineris-config::it mcp`, `cineris-config mcp::tests`), and each fix has a test that failed without it and a break.

| Finding | The fix |
|---|---|
| **High: a `*` in a `{ env = "…" }` source read any variable.** `ANTHROPIC_API_KE*`, `*`, `CINERIS*` and `GATEWAY*` were tested as names and looked up as patterns (`child_env::is_named` takes a trailing `*` as a prefix): a server was given the Anthropic key. | A source that is not a variable's name, or holds `*`, `?`, `[`, `]` or `{`, is refused at load (`cineris_config::mcp::bad_source_name`, for an `env` entry's name and source and a user's header's source), at the start (`server_env`), and by the first run's import (`givable`). The start looks a variable up by its exact name (Windows's case rule on Windows) and judges the name it found against the withheld list as well as the text written. The workspace was searched for the same mechanism (`child_env::is_named` and `matches` callers, `Env::get`, hook commands' and extensions' programs' environments, `shell.env_remove`, the first run's import): every other caller looks up a constant name or removes by a pattern, and none reads a value through a name from configuration; nothing outside step 30's crates needed the fix. |
| **Medium: a project's derived header variable could be the user's token** (`github` and `token` made `GITHUB_TOKEN`; `my_tracker` the user's `MY_TRACKER_AUTHORIZATION`). | A project's header is taken only from `CINERIS_MCP_<SERVER>_<HEADER>` (`project_header_variable`), the one `CINERIS_` variable a project's header may take, and never from one the user's own servers take (refused at load, naming the user's server). `ProjectMcpServer::header_variables` gives the trust question each header's variable to name beside the host. The residual (two projects with a server of one name share a variable: set it per project, in that project's shell) is in 13-mcp and `docs/security.md` §3. |
| **Medium: built-in providers' keys were not withheld at load or in `definitions()`** (`OPENROUTER_API_KEY`, `DEEPSEEK_API_KEY`, `VENICE_API_KEY`). | One list: `cineris_base::child_env::KNOWN_KEYS` now holds every built-in provider's key variable, which the configuration checks against, and a core test holds the provider layer's table to it. `definitions()` uses the core's `provider::key_variables` (built-in, configured, and each provider header's variable); the configuration's `provider_keys` gives the configured ones as it loads. A remote server's headers are held to it as a local one's variables are. |
| **Medium: the trust question hid a project's `env` literals** (`NODE_OPTIONS = "--require ./.hidden/evil.js"` showed `[redacted]`). | `redact_for_consent` shows a server's `env` and `headers` whole, only the secret filter's formats masked, as the rest of a project's entry. |
| **Medium: `ProjectMcpServer`'s derived `Debug` printed its table**, literals included, into `Loaded`'s. | `Debug` written by hand without the table; `printing::no_debug_or_display_form_holds_a_key` has a project's server holding the canary. |
| **Low: a start that failed before its waiter left its folder.** | The folder is an owned value (`OwnFolder`) removed when dropped, on every path, before the end is told. |
| **Low: the output queue held up to 256 MiB.** | `OUTPUT_QUEUED = QUEUED_BYTES / MAX_LINE`: 4 lines, at most 16 MiB; past it the reader waits, and the server's writes wait on the pipe. |
| **Low: one stray key in `[mcp.servers.<name>]` failed the whole configuration.** | An unknown key, a wrong type or a value out of range inside a server's entry is a warning naming the server (`SERVER_NOT_STARTED`), which is not started; `Settings` reads the servers through `McpServers`, which keeps an entry that does not read in `refused`, and the loader (and a reload's `taking`) refuses a user's server the check warned of. A project's server keeps its `problem`. **A bad key directly under `[mcp]` still refuses the file.** |
| **Low: a `command` or `cwd` under `/proc`** was checked in the harness's process and resolved in the server's. | Refused at load (lexically) and at the start (through any link, `passes_through` with `/proc`); `/proc` is among the places no program is found and `PATH` leaves out. |
| **Breaks no test caught** | Tests now catch each: a `cwd` that is a link into the workspace (a case form and a trailing dot on Windows); the servers' folders inside the workspace; a dropped server killed; a server that ends by itself leaving a child, its tree ended and its folder gone before its end is told; a confined write to the system's temporary folder refused; a reload taking a project's servers from the newer load. |

## The interfaces added

```rust
// cineris-sandbox
pub const POLICY_BYTES: &str = "--policy-bytes";
impl Launcher { pub fn wrap_live(&self, program: &OsStr, arguments: &[OsString]) -> Wrapped; }

// cineris-config
pub mod mcp {
    pub const MAX_SERVER_NAME: usize; // 64
    pub fn server_name_ok(name: &str) -> bool;
    pub fn bad_variable_name(name: &str) -> Option<&'static str>;
    pub fn withheld_source(variable: &str, keys: &[String], env_remove: &[String]) -> bool;
    pub fn loads_code(variable: &str) -> bool; // LD_…, GLIBC_TUNABLES, DYLD_…
    pub fn bad_source_name(name: &str) -> Option<&'static str>; // a name, never a pattern
    pub fn project_header_variable(server: &str, header: &str) -> String; // CINERIS_MCP_…
    pub fn provider_keys(settings: &Settings) -> Vec<String>;
    pub fn sources(entry: &McpServerSettings) -> Vec<String>;
    pub fn server_of(key: &KeyPath) -> Option<&str>;
    pub const SERVER_NOT_STARTED: &str;
    pub fn server_problem(name: &str, entry: &McpServerSettings, project: bool,
                          keys: &[String], env_remove: &[String]) -> Option<String>;
    pub struct ProjectMcpServer { pub name: String, pub settings: McpServerSettings,
                                  pub origin: Origin, pub problem: Option<String>, /* table */ }
    impl ProjectMcpServer { pub fn for_consent(&self) -> toml::Value;
                            pub fn header_variables(&self) -> Vec<(String, String)>; }
}
// schema: McpSettings { start_timeout_ms: u64, direct_share: u64,
//   servers: McpServers { entries: BTreeMap<String, McpServerSettings>,
//   refused: BTreeMap<String, String> } }; McpServerSettings { command:
//   Option<ServerProgram>, args: ServerArgs, cwd: Option<String>, env: Variables, url:
//   Option<ServerAddress>, headers: Headers, network: bool, description: Option<String>,
//   enabled: bool, expose: ExposeChoice, tools: Option<Vec<String>>, deny_tools: Vec<String>,
//   start_timeout_ms: Option<u64>, timeout_ms: u64 }; ServerProgram(pub String),
//   ServerArgs(pub Vec<String>), ServerAddress(pub String), Variables(pub BTreeMap<String,
//   EnvText>), ExposeChoice { Search, Direct }.
// Loaded: pub project_mcp_servers: Vec<mcp::ProjectMcpServer>.

// cineris-core, behind `mcp`
pub mod mcp {
    pub struct Confinement { pub program: PathBuf, pub prefix: Vec<OsString>,
                             pub read: Reach, pub landlock: u8 }
    pub struct SpawnerOptions { pub workspace: PathBuf, pub trusted: bool, pub folders: PathBuf,
        pub confine: Option<Confinement>, pub write_roots: Vec<PathBuf>,
        pub inherited: Option<Vec<(OsString, OsString)>>, pub withheld: Vec<String>,
        pub filter: Arc<RwLock<SecretFilter>>, pub guard: Option<Guard>, pub timer: SharedTimer }
    pub struct ServerSpawner; impl ServerSpawner { pub fn new(options: SpawnerOptions) -> Self; }
    impl cineris_mcp::Spawner for ServerSpawner { /* spawn */ }
    pub struct EnvParts<'a> { pub inherited, pub withheld, pub avoid, pub temp, pub cmd, pub windows }
    pub struct ServerEnv { pub vars: Vec<(OsString, OsString)>, pub secrets: Vec<Secret> }
    pub fn server_env(entries: &[EnvEntry], parts: &EnvParts<'_>) -> Result<ServerEnv, SpawnError>;
    pub fn definitions(loaded: &cineris_config::Loaded) -> Definitions;
    pub struct Definitions { pub servers: Vec<ServerDefinition>, pub refused: Vec<Refused> }
    pub struct Refused { pub name: String, pub origin: Origin, pub why: String }
    impl Refused { pub fn notice(&self) -> String; }
    pub const MAX_LINE: usize; pub const STDERR_KEPT: usize;
    pub const LINES_QUEUED: usize; pub const STAGE_GRACE: Duration;
    pub const OUTPUT_QUEUED: usize; pub const QUEUED_BYTES: usize;
}
```

No signature of `cineris-mcp`'s shared interface changed. `Launch::Local::write` stays, and is never `true` from the configuration; the spawner refuses a definition that sets it.

**For the lifecycle part:** a session builds `SpawnerOptions` from its sandbox (`SessionSandbox::made_with`'s program, prefix and Landlock version, and its launcher's reads: an accessor is that part's, since nothing calls it yet), its workspace and trust, a folder under the harness's state directory, its write roots, its `ExecParts`-like withheld list and `shell.env_remove`, the core's `SharedFilter`, its guard and timer; and calls `spawn` off the core thread.

## The tests, by rule

| Rule | Test |
|---|---|
| The launcher's length, every wrong form refused | `cineris-sandbox launch::tests::a_policy_length_not_written_as_the_harness_writes_it_is_refused`; `cineris-sandbox::it confined::sys_a_live_policy_length_that_is_wrong_runs_nothing` |
| The length is the policy's exact length | `launch::tests::a_live_wrap_gives_the_policys_exact_length_before_the_marker`; `confined::sys_a_live_length_that_is_not_the_policys_runs_nothing` (one short, one long, an `input` field) |
| A live input reaches the confined command line by line | `confined::sys_a_confined_command_reads_its_live_input_line_by_line` |
| Bytes after the policy are input, never policy | `confined::sys_a_policy_written_after_the_live_policy_is_only_input` |
| The explicit environment | `cineris-core mcp::env::tests::*` (base list, sources, names, `PATH`, Windows' list); `cineris-core::it mcp_servers::sys_a_server_is_given_exactly_its_list` (a child printing its environment) |
| A program inside the workspace, by path or by `PATH`, a relative one, and `write = true` | `mcp_servers::a_program_inside_the_workspace_is_never_started` |
| The working folder by trust | `mcp_servers::sys_the_workspace_is_a_servers_folder_only_in_a_trusted_project` |
| Lines both ways; a line end inside a line; the end | `mcp_servers::sys_a_server_answers_each_line_and_ends_with_its_input` |
| The 4 MiB cap | `mcp_servers::sys_a_line_over_the_cap_ends_the_output` (over and at the cap) |
| Standard error's end, masked | `mcp_servers::sys_a_servers_standard_error_is_kept_to_its_end_and_masked` |
| The grace on the fake timer, then the tree, its child too | `mcp_servers::sys_a_server_that_ignores_its_closed_input_is_killed_after_its_grace` |
| The end asked for twice at once runs once; a third returns at once | `mcp_servers::sys_a_servers_end_asked_for_twice_at_once_ends_it_once` |
| Confinement (Linux): only its own folder written, the network only with `network` | `mcp_servers::sys_a_confined_server_writes_only_its_own_folder_and_reaches_the_network_only_when_told` |
| Windows: a `.cmd` shim started, an argument it cannot carry refused, a planted `cin-planted.cmd` never run | `mcp_servers::sys_a_cmd_shim_is_started_and_never_runs_a_program_from_the_workspace` |
| An entry judged whole; a project's header from its derived variable | `cineris-config mcp::tests::*` |
| The first run imports nothing the loader refuses | `mcp::tests::the_first_run_imports_no_server_the_loader_would_refuse`, `write::tests::a_hostile_name_stays_one_server` |
| Kept apart, by trust and by lock | `cineris-config::it trust::*`, `layers::a_value_cannot_replace_a_table_that_holds_a_locked_key` |
| The display rules | `cineris-config::it imports::*`, `printing::no_debug_or_display_form_holds_a_key` |
| Definitions | `cineris-core mcp::definitions::tests::the_users_and_a_projects_servers_become_definitions` |

Each rule has a break in [mutations/step-30-pr1-process-config.breaks](../../../../mutations/step-30-pr1-process-config.breaks).

## Left open, and untested on Windows

- **Untested on Windows by this part's own runs:** the shim test, `NoDefaultCurrentDirectoryInExePath` with a planted program, the environment's Windows list in a child, and the staged end through a Job Object (interrupt is a Ctrl-Break, terminate is not there, kill ends the job). The maintainers' runners run them.
- **A literal `env` value is not given to the secret filter**, as a provider's literal header is not; a value read from the environment is. Recommendation: keep it so (a literal is often a mode, `NODE_ENV = "production"`, and masking it would take ordinary words out of every log).
- **A project's profile's `[mcp]`** is not read: a project's servers come from its files' top level. A user's profile's servers merge as any setting's.
- **The name rule now refuses on import** a server whose name holds a space or a letter other than an ASCII one, which the first run took before step 30.
