// The Cineris protocol, as TypeScript declarations.
//
// Generated from the Rust types in crates/cineris-protocol by `cargo xtask schema`.
// Do not edit: change the types and run the command again.
//
// Every enumeration and every tagged union is open. A string enumeration
// lists the values this version knows and also accepts any other string.
// A tagged union lists the variants this version knows; a receiver must
// also tolerate an object whose `type` or `kind` is not listed, and skip
// it or show it generically. Objects may carry fields that are not listed.

/** The protocol version these declarations describe. */
export type ProtocolVersion = 1;

/** Requests a client sends to the server, by method name. */
export interface ClientMethods {
  /** The custom commands a session may send, and those found and not used, with why. Only reads. */
  "commands/list": { params: CommandsListParams; result: CommandsListResult };
  /** Reads the effective configuration. */
  "config/get": { params: ConfigGetParams; result: ConfigGetResult };
  /** Re-reads the configuration files, judges the project's trust again, and applies what changed to the open sessions. */
  "config/reload": { params: ConfigReloadParams; result: ConfigReloadResult };
  /** The state of the extensions. */
  "extensions/status": { params: ExtensionsStatusParams; result: ExtensionsStatusResult };
  /** The handshake. It must be the first request on a connection. */
  initialize: { params: InitializeParams; result: InitializeResult };
  /** The state of the MCP servers. */
  "mcp/status": { params: Empty; result: McpStatusResult };
  /** Reads the model catalog. */
  "models/list": { params: Empty; result: ModelsListResult };
  /** Lists the approvals in force for a session. */
  "permissions/list": { params: SessionRef; result: PermissionsListResult };
  /** Removes stored approvals. */
  "permissions/revoke": { params: PermissionsRevokeParams; result: Empty };
  /** Reads the plan. */
  "plan/get": { params: SessionRef; result: PlanResult };
  /** Applies a user's edits to the plan. */
  "plan/update": { params: PlanUpdateParams; result: PlanResult };
  /** The commands a project's own files name, for the first run. */
  "project/detect": { params: ProjectDetectParams; result: ProjectDetectResult };
  /** Reads the project profile and memory. */
  "project/list": { params: ProjectListParams; result: ProjectEntriesResult };
  /** Accepts, edits, disables, deletes or promotes entries. */
  "project/update": { params: ProjectUpdateParams; result: ProjectEntriesResult };
  /** Reads a stored artifact of a session. */
  "session/artifact": { params: SessionArtifactParams; result: SessionArtifactResult };
  /** Stops using a session on this connection. The session is kept. */
  "session/close": { params: SessionRef; result: Empty };
  /** Compacts now, with optional focus instructions. */
  "session/compact": { params: SessionCompactParams; result: Empty };
  /** Makes a new session from a point in this one. */
  "session/fork": { params: SessionForkParams; result: SessionForkResult };
  /** Lists stored sessions. */
  "session/list": { params: SessionListParams; result: SessionListResult };
  /** Creates a session: workspace, models, level, overrides. */
  "session/new": { params: SessionNewParams; result: SessionResult };
  /** Opens a stored session. Returns its metadata and current state. */
  "session/open": { params: SessionRef; result: SessionResult };
  /** Reads items of a session from the log, in pages. Also how a client catches up after a gap in `seq` or an `item.resync`. */
  "session/read": { params: SessionReadParams; result: SessionReadResult };
  /** Moves the active branch back to an earlier item. */
  "session/rewind": { params: SessionRewindParams; result: SessionResult };
  /** What `session/rewind` to an item would do now, changing nothing. */
  "session/rewind_plan": { params: SessionRewindPlanParams; result: SessionRewindPlanResult };
  /** Changes a session's settings: models, reasoning level, speed, level. */
  "session/set": { params: SessionSetParams; result: SessionResult };
  /** Steps back before the last turn, putting its files back if asked. */
  "session/undo": { params: SessionUndoParams; result: SessionResult };
  /** Stops the server cleanly. */
  shutdown: { params: Empty; result: Empty };
  /** Cancels the running turn. It is not an error when none is running. With `now`, running commands are killed at once. */
  "turn/interrupt": { params: TurnInterruptParams; result: TurnInterruptResult };
  /** Sends a prompt. If a turn is running, `delivery` says whether to steer it or to queue behind it. */
  "turn/start": { params: TurnStartParams; result: TurnStartResult };
  /** Token, cost and cache figures for the session, live. */
  "usage/get": { params: SessionRef; result: UsageGetResult };
  /** Asks the providers' usage endpoints that the user turned on, with each one's own credential. Only a `user` client. */
  "usage/refresh": { params: UsageRefreshParams; result: UsageRefreshResult };
  /** Usage and cost for a range, grouped, with limits left. */
  "usage/report": { params: UsageReportParams; result: UsageReport };
  /** Runs the project's checks now. The outcome arrives as events. */
  "verify/run": { params: SessionRef; result: Empty };
  /** Lists the workspace's files, for file completion. */
  "workspace/files": { params: WorkspaceFilesParams; result: WorkspaceFilesResult };
  /** Reads ahead files a person named, so that the model's first read of them finds them in the system's cache. Answers at once. */
  "workspace/warm": { params: WorkspaceWarmParams; result: Empty };
}

/** Requests the server sends to a client, by method name. Each pauses the turn until it is answered or times out. */
export interface ServerMethods {
  /** Asks whether a tool call may run. Unanswered: deny. */
  "approval/request": { params: ApprovalRequest; result: ApprovalResponse };
  /** Asks the user a question. Unanswered: the asker is told "no answer". */
  "question/ask": { params: QuestionRequest; result: QuestionResponse };
  /** Asks whether to trust a project's executable resources. Unanswered: not trusted. */
  "trust/request": { params: TrustRequest; result: TrustResponse };
}

/** Notifications the server sends, by method name, with their parameters. A client ignores a method it does not know. */
export interface ServerNotifications {
  /** One event of a session. The only notification the server sends. */
  "session/event": SessionEvent;
}

/** Who made a change. */
export type Actor = "agent" | "user" | "harness" | (string & {});

/** A rule an answer to an approval request would add: always an `allow`. It only informs, so it is read as everything that informs is: an effect this version does not know is kept as it was written, and a field it does not know is ignored. It is never enforced: the rule an answer adds is the one the server worked out, never what a client sends back. The enforced `Rule`, which refuses both, is for rules that are applied (`docs/decisions/0028-enforced-values-fail-closed.md`). */
export interface AddedRule {
  /** The effect it allows. */
  effect: Effect;
  /** What it matches: a path glob, a command pattern, a host, or a `server/tool` name. */
  match: string;
}

/** An answer to an approval request. A note may go with any of them, and with `deny` it reaches the model as the reason. */
export type ApprovalChoice = "allow_once" | "allow_session" | "allow_project" | "deny" | (string & {});

/** Exactly what an approval would let happen. */
export type ApprovalDetail = ({
    kind: "command";
  } & CommandDetail) | ({
    kind: "diff";
  } & DiffDetail) | ({
    kind: "target";
  } & TargetDetail) | ({
    kind: "tool";
  } & ToolDetail);

/** One stored approval. */
export type ApprovalId = string;

/** A unit of a store of approvals as a yes would write it, which the person signs whole (since the signed approvals step; `docs/architecture/06-permissions-and-sandbox.md`, "Signed approvals"). Hashes are lowercase hexadecimal. The bytes signed are made from it by `cineris_policy::signed`, in one way only, so that a front end and the store agree on them. */
export type ApprovalProposal = {
    /** Whether git may read the repository's own configuration. */
    git_trusted: boolean;
    /** The project's key. */
    key: ProjectKey;
    /** Every pin of the entry after the change. */
    pins: Array<ProposedPin>;
    /** The entry's revision after the change. */
    revision: number;
    /** The root, as it is shown to a person. */
    root: string;
    unit: "project";
  } | {
    /** Every pin after the change. */
    pins: Array<ProposedPin>;
    /** The revision after the change. */
    revision: number;
    unit: "user_pins";
  };

/** `approval/request`: may this tool call run? */
export interface ApprovalRequest {
  /** The rules that `allow_session` or `allow_project` would add, so that a front end can show exactly what such an answer allows from then on. Empty when only this one call can be allowed. */
  adds?: Array<AddedRule>;
  /** The tool call. */
  call_id: CallId;
  /** The answers on offer. */
  choices: Array<ApprovalChoice>;
  /** Exactly what would happen. */
  detail: ApprovalDetail;
  /** The `tool.before` hooks that put the question (since step 28): each that answered `ask`, with its reason. A hook only narrows, so it may put a question about a call the rules alone would have allowed. `reasons` holds a line for each as well ([`HookReason::asking_line`]), and one for `replaced` ([`ReplacedArguments::reason_line`]), for a client that reads neither. */
  hooks?: Array<HookReason>;
  /** Why the call needs an answer, a reason an entry: the rule that asks, the part of a command that is not on a safe list, a protected path. */
  reasons?: Array<string>;
  /** The arguments a hook put in place of the model's (since step 28), with the model's own as `original`: the call would run with the hook's, and `summary` and `detail` describe it so. Such a question is always put, whatever the rules say, and offers only `allow_once` and `deny`. (Boxed: it is rare, and every request would otherwise be its size.) */
  replaced?: ReplacedArguments | null;
  /** The session. */
  session_id: SessionId;
  /** The call in one line. */
  summary: string;
  /** The tool's name. */
  tool: string;
  /** The turn. */
  turn_id: TurnId;
}

/** The answer to `approval/request`. */
export interface ApprovalResponse {
  /** The choice: one of those on offer. */
  choice: ApprovalChoice;
  /** A note for the model. */
  note?: string | null;
}

/** How long an approval lasts. */
export type ApprovalScope = "session" | "project" | (string & {});

/** A stored artifact, such as the full output of a command or an attached image. */
export type ArtifactId = string;

/** A reference to a stored artifact, such as the full output of a command. */
export interface ArtifactRef {
  /** Its size in bytes. */
  bytes?: number | null;
  /** The artifact's id. */
  id: ArtifactId;
  /** What it holds, in a few words. */
  label: string;
}

/** One response from the model: text, reasoning and tool calls. */
export interface AssistantMessageItem {
  /** When it was recorded. */
  created_at?: Timestamp | null;
  /** The failure, when `stop_reason` is `error`. */
  error?: ErrorData | null;
  /** The item's id. */
  id: ItemId;
  /** What produced the message. */
  origin?: Origin | null;
  /** The content, in the model's order. */
  parts: Array<Part>;
  /** The provider's own stop value. */
  raw_stop?: string | null;
  /** More about the stop where the provider gave it, such as the category of a refusal. */
  stop_detail?: string | null;
  /** Why the response ended. Absent only on an item that is still streaming. */
  stop_reason?: StopReason | null;
  /** Whether the response was cut off by the output limit. */
  truncated?: boolean;
  /** What the request used and cost. */
  usage?: RequestUsage | null;
}

/** An amount of money a provider reported as left, in one currency. */
export interface Balance {
  /** The currency as the provider names it: `USD`, `CNY`, or a unit of the provider's own. */
  currency: string;
  /** The budget it is left of, where there is one. */
  limit?: string | null;
  /** What is left, as an exact decimal number: `110.00`. It may be negative. Absent when the provider reports only a limit or a spend. */
  remaining?: string | null;
  /** What has been spent of it, where the provider says. */
  used?: string | null;
}

/** How the usage is paid for. */
export type Billing = "metered" | "subscription" | "unknown" | (string & {});

/** What a lease may spend. A field that is absent is not limited by this lease. */
export interface Budget {
  /** Requests to a model. */
  requests?: number | null;
  /** Tokens, input and output together. */
  tokens?: number | null;
  /** Money, in nano-dollars. */
  usd_nanos?: number | null;
  /** Wall-clock time, in milliseconds. */
  wall_clock_ms?: number | null;
}

/** The price of writing to the prompt cache, for one lifetime class. */
export interface CacheWritePrice {
  /** The provider's name for the lifetime class, for example `5m` or `1h`. */
  class: string;
  /** Nano-dollars per million tokens. */
  price: number;
}

/** Input tokens written to the cache, for one lifetime class. */
export interface CacheWriteTokens {
  /** The provider's name for the lifetime class, for example `5m` or `1h`. */
  class: string;
  /** The tokens written in that class. */
  tokens: number;
}

/** One tool call. The provider chose it, so it is unique only within its session. */
export type CallId = string;

/** How one check ended. */
export type CheckOutcome = "passed" | "failed" | "timed_out" | "flaky" | "blocked" | "skipped" | "not_run" | (string & {});

/** One check, as a [`VerificationNotice`] tells of it. */
export interface CheckReport {
  /** All of what it printed, when the notice to the model held less. */
  artifact?: ArtifactRef | null;
  /** The command line. */
  command: string;
  /** The directory it runs in, relative to the workspace root. Absent: the root. */
  cwd?: string | null;
  /** How long it ran, in milliseconds. */
  duration_ms?: number | null;
  /** The project profile's entry it came from. */
  entry?: EntryId | null;
  /** Its exit code, when it exited with one. */
  exit_code?: number | null;
  /** What else to say of it, in words: why it was not run, what it left running, what the sandbox blocked. */
  note?: string | null;
  /** How it ended. Absent in a notice of a round that is starting, and of checks that are offered. */
  outcome?: CheckOutcome | null;
  /** Where it came from. */
  source: CheckSource;
}

/** Where a check came from. */
export type CheckSource = "user" | "profile" | (string & {});

/** What a client can handle. A capability that is absent is `false`: the server never sends what the client did not ask for. */
export interface ClientCapabilities {
  /** It answers `approval/request`. */
  approvals?: boolean;
  /** It wants `item.delta` events. */
  deltas?: boolean;
  /** The experimental features it opts in to, by name. */
  experimental?: Array<string>;
  /** It answers `question/ask`. */
  questions?: boolean;
  /** It answers `trust/request`. */
  trust?: boolean;
}

/** What a client is to the sessions it opens. */
export type ClientRole = "user" | "supervisor" | "observer" | (string & {});

/** An argument a command names. */
export interface CommandArgument {
  /** What it is when it is not given, if anything. */
  default?: string | null;
  /** Its name. */
  name: string;
}

/** A command named in `turn/start` (since step 28). */
export interface CommandCall {
  /** What was typed after the name: its arguments, as words, a word in double quotes holding spaces, and `name=value` for an argument by its name. */
  args?: string;
  /** Its name, as `commands/list` gives it, without the `/`. */
  name: string;
}

/** The detail of an approval for a command. */
export interface CommandDetail {
  /** The full command line. Nothing is shortened. */
  command: string;
  /** The directory it would run in. */
  cwd: string;
  /** Whether it would run outside the sandbox. */
  outside_sandbox?: boolean;
}

/** One command `commands/list` gives. */
export interface CommandInfo {
  /** The arguments it names, in order. */
  arguments?: Array<CommandArgument>;
  /** Its description, for a menu. Empty when it gives none. */
  description?: string;
  /** The extension whose script handles it, for an extension's command. */
  extension?: string | null;
  /** Its name, without the `/`. */
  name: string;
  /** Its file, relative to the workspace for a project's, under `~` for the user's; absent for an extension's. */
  path?: string | null;
  /** Whose it is. */
  source: CommandSource;
}

/** A command found and not used, and why. */
export interface CommandNotUsed {
  /** The extension it is of, for an extension's command. */
  extension?: string | null;
  /** Its name, as it would be typed; empty when the file's name is none. */
  name?: string;
  /** Its file, or the folder left out, as [`CommandInfo::path`] shows it. */
  path?: string | null;
  /** Why it is not used, in a sentence. */
  reason: string;
  /** Whose it is. */
  source: CommandSource;
}

/** What a command entry is for. */
export type CommandPurpose = "build" | "test" | "check" | "format" | "run" | (string & {});

/** Whose a command is (since step 28). An extension's command is its extension's: the user's or a project's. */
export type CommandSource = "user" | "project" | (string & {});

/** `commands/list` (since step 28). */
export interface CommandsListParams {
  /** The session whose commands they are: its workspace's and the user's, and those of the extensions it has loaded. */
  session_id: SessionId;
}

/** The result of `commands/list`. */
export interface CommandsListResult {
  /** The commands that may be sent, by name. */
  commands: Array<CommandInfo>;
  /** The commands found that are not used, and why. */
  not_used?: Array<CommandNotUsed>;
}

/** Notice: a compaction started, finished or failed. */
export interface CompactionNotice {
  /** The estimated size of the new context epoch, in tokens, on a compaction that finished. */
  after_tokens?: number | null;
  /** Whether it runs in the background while the turn carries on. */
  background?: boolean;
  /** The estimated size of the conversation before, in tokens, on a compaction that finished. */
  before_tokens?: number | null;
  /** The notice in words. */
  message: string;
  /** Where it stands. */
  state: CompactionState;
}

/** Where a compaction stands. */
export type CompactionState = "started" | "finished" | "failed" | (string & {});

/** The state of an MCP server or an extension. */
export type ComponentState = "starting" | "ready" | "failed" | "disabled" | "needs_approval" | (string & {});

/** The state of one MCP server or one extension. */
export interface ComponentStatus {
  /** An extension's: the names of its commands (since step 28). */
  commands?: Array<string>;
  /** What went wrong, for `failed`; why it waits, for `needs_approval`; why it is off, for `disabled`. */
  error?: string | null;
  /** An extension's: its folder, under `~` for the user's and relative to the workspace for a project's (since step 28). */
  folder?: string | null;
  /** An extension's: the gating hooks the session holds of it, failing, while it does not stand as it was approved (since step 28). */
  gates?: Array<HeldGate>;
  /** An extension's: the hash of its folder read whole, as it is pinned (since step 28). */
  hash?: string | null;
  /** An extension's: its hooks, each its event and the tools it is on (since step 28). */
  hooks?: Array<string>;
  /** Its configured name. */
  name: string;
  /** An extension's: whose it is (since step 28). */
  source?: ExtensionSource | null;
  /** An extension's: how it stands against its approval, as a session judges it (since step 28). */
  standing?: ExtensionStanding | null;
  /** Its state. */
  state: ComponentState;
  /** How many tools it provides. */
  tools?: number;
}

/** What a reload of the configuration changed for a session, whoever asked for it: a front end, another client, or the look the core takes at the start of a turn. Each client of the session is sent it, so that one that keeps settings of its own (a terminal's theme and key bindings) takes them up whoever found the change. Added in step 24. */
export interface ConfigChanged {
  /** Keys whose new values are in force. */
  applied?: Array<string>;
  /** What was wrong with a file. The configuration in force stays. */
  errors?: Array<string>;
  /** What to tell the person, a sentence each. */
  notes?: Array<string>;
  /** Keys whose new values wait; `notes` says for what. */
  pending?: Array<string>;
}

/** `config/get` */
export interface ConfigGetParams {
  /** Only these keys, in dotted form. Empty: everything. */
  keys?: Array<string>;
  /** The session whose effective configuration is wanted. Absent: the configuration without any project's layers. */
  session_id?: SessionId | null;
}

/** The result of `config/get`. */
export interface ConfigGetResult {
  /** The effective configuration, as the configuration file would have it. Credentials are never in it. */
  config: { [key: string]: unknown };
  /** Where each key of `config` came from, by the same dotted key. A key with no entry has its built-in default. Added in step 24. */
  origins?: { [key: string]: ConfigOrigin };
  /** The profile in force: the one chosen for the session, or the one the files select. Added in step 24. */
  profile?: string | null;
  /** The settings profiles that may be chosen for the session (`/profile`): those the user's and the managed files define, and a project's only once the project is trusted. Added in step 24. */
  profiles?: Array<string>;
}

/** The layer of the configuration a value came from (`docs/architecture/08-configuration.md`, "Layers"). */
export type ConfigLayer = "default" | "managed" | "user" | "project" | "project-local" | "profile" | "environment" | "command-line" | (string & {});

/** Where one value of the configuration came from. Added in step 24. */
export interface ConfigOrigin {
  /** The file, as a person reads it (`~` for the home directory). Absent for a layer that is not a file. */
  file?: string | null;
  /** The layer. */
  layer: ConfigLayer;
  /** The line in the file, from 1. */
  line?: number | null;
}

/** `config/reload`. Every field may be left out: `{}` reloads every open session and the core's own configuration, whether or not a file changed. Before step 24 it took no parameters. */
export interface ConfigReloadParams {
  /** Reload only where a file, or the trust store, changed since it was last looked at: the check a front end makes when the terminal regains focus or a key is pressed. */
  changed_only?: boolean;
  /** Load the session with this settings profile (`/profile`). Needs `session_id`, and a `user` client. The session's own choices of level, models and speeds give way to the profile's. */
  profile?: string | null;
  /** Only this session. Absent: every session this client has open, and the core's own configuration. */
  session_id?: SessionId | null;
}

/** The result of `config/reload`, over every session it reloaded. */
export interface ConfigReloadResult {
  /** Keys whose new values are in force. */
  applied?: Array<string>;
  /** What was wrong with a file, or with what a change needs. When this is not empty the configuration in force stays as it was, except that trust taken away is taken away. */
  errors?: Array<string>;
  /** What to tell the person, a sentence each: what waits and for what, what trust held back or took away, which of the session's own choices a profile replaced. Added in step 24. */
  notes?: Array<string>;
  /** Keys whose new values wait: for the session's next context epoch (a compaction, or the session opened again), for sessions opened from now on, or for the harness to be started again. `notes` says which. */
  pending?: Array<string>;
}

/** What one request's context held, by category, in tokens as the harness estimates them (`docs/architecture/15-efficiency.md`, "Context profiler"). Added in step 24. */
export interface ContextBreakdown {
  /** The model's answers and calls. */
  assistant?: number;
  /** The whole request. */
  estimate?: number;
  /** The most the request may hold: the window less the room kept for the answer and a margin. */
  limit?: number;
  /** The harness's notices. */
  notices?: number;
  /** A compaction's summary. */
  summary?: number;
  /** The system prompt. */
  system?: number;
  /** The tools' results. */
  tool_results?: number;
  /** The tools' definitions. */
  tools?: number;
  /** The person's messages. */
  user?: number;
  /** The model's context window. */
  window?: number;
}

/** Whether a cooldown began or ended. */
export type CooldownState = "started" | "ended" | (string & {});

/** An amount of money with its basis. Amounts with different bases are never added together. */
export interface Cost {
  /** What kind of number it is. */
  basis: CostBasis;
  /** US dollars, as a whole number of nano-dollars (1e-9). Whole numbers make sums exact: `184_200_000` is $0.1842. */
  usd_nanos: number;
}

/** What kind of number a cost is. */
export type CostBasis = "metered" | "estimate" | "recorded" | (string & {});

/** When a prompt sent during a running turn is delivered. */
export type Delivery = "steer" | "queue" | (string & {});

/** Streaming content for an item. A display convenience: deltas may be merged or dropped for a slow client, and `item.completed` is what counts. */
export type Delta = ({
    kind: "text";
  } & TextDelta) | ({
    kind: "reasoning";
  } & TextDelta) | ({
    kind: "tool_call";
  } & ToolCallDelta) | ({
    kind: "tool_args";
  } & ToolArgsDelta) | ({
    kind: "tool_output";
  } & TextDelta);

/** A command a project's own files name, found by `project/detect` (since step 25): offered at the first run, each shown in full before it is kept. Nothing is learned, proposed or run by finding it. */
export interface DetectedCommand {
  /** The command line that would run: plain words, as an entry the user writes holds them. */
  command: string;
  /** What the file calls it: a script's or a target's name, or what it is for when the file names no command (`Cargo.toml`'s `test`). */
  name: string;
  /** What it is for. */
  purpose: CommandPurpose;
  /** For a script: what the script itself runs, whole, as its file has it: a script longer than 1,000 characters, or with a character that acts unseen, is not given. The command line runs it. */
  runs?: string | null;
  /** The file it was found in, relative to the workspace root. */
  source: string;
}

/** The detail of an approval for a change to a file. */
export interface DiffDetail {
  /** The change, as a unified diff. Empty when it is left out. */
  diff: string;
  /** Why the diff is left out, in a sentence: it is larger than a request carries, or the change cannot be worked out before the call runs. Absent when `diff` is the whole change. */
  omitted?: string | null;
  /** The file. */
  path: string;
}

/** What became of a prompt. */
export type Disposition = "started" | "steering" | "queued" | (string & {});

/** What a planned tool call does. */
export type Effect = "read" | "write" | "exec" | "network" | "ext" | (string & {});

/** A result with nothing in it. Written as `{}`, so fields can be added later. */
export interface Empty {
  [key: string]: unknown;
}

/** What a user does to an entry. */
export type EntryAction = "accept" | "edit" | "disable" | "delete" | "promote" | (string & {});

/** How the harness came to hold an entry. */
export type EntryBasis = "observed" | "proposed" | (string & {});

/** One change to a project entry. */
export interface EntryChange {
  /** What to do to it. */
  action: EntryAction;
  /** The entry. */
  id: EntryId;
  /** For `edit`: the new text. */
  text?: string | null;
}

/** A cheap, read-only test that shows an entry still holds. */
export interface EntryCheck {
  /** For `command`: the command line. */
  command?: string | null;
  /** The form of the check. */
  kind: EntryCheckKind;
  /** For `exists` and `contains`: the path, relative to the workspace root. */
  path?: string | null;
  /** For `contains`: the pattern. */
  pattern?: string | null;
}

/** The form of a check. */
export type EntryCheckKind = "exists" | "contains" | "command" | (string & {});

/** The command of a `command` entry. */
export interface EntryCommand {
  /** The command line. */
  command: string;
  /** The directory it runs in, relative to the workspace root. */
  cwd?: string | null;
  /** What it is for. */
  purpose: CommandPurpose;
  /** Whether it only reads. */
  read_only?: boolean;
  /** How long it usually takes, in milliseconds. */
  usual_duration_ms?: number | null;
}

/** One entry of the project profile or memory. */
export type EntryId = string;

/** What an entry is. */
export type EntryKind = "command" | "convention" | "flaky" | "fact" | (string & {});

/** Which file an entry lives in. */
export type EntryScope = "shared" | "private" | (string & {});

/** Where an entry came from. */
export interface EntrySource {
  /** For `learned`: when. */
  at?: Timestamp | null;
  /** For `learned`: the item. */
  item_id?: ItemId | null;
  /** Who it came from. */
  kind: EntrySourceKind;
  /** For `learned`: the session. */
  session_id?: SessionId | null;
}

/** Who an entry came from. */
export type EntrySourceKind = "user" | "learned" | (string & {});

/** Whether an entry is in use. */
export type EntryStatus = "proposed" | "active" | "disabled" | (string & {});

/** An error, as every front end sees it: in `turn.completed`, on an assistant item whose request failed, and as the `data` of a JSON-RPC application error. */
export interface ErrorData {
  /** A stable, machine-readable name for the exact failure, in `snake_case`: one of [`code`](crate::error::code), a provider's own error code, or a code a later version adds. Clients may branch on the ones they know and must tolerate the rest. */
  code: string;
  /** Structured detail whose shape depends on `code`. For `unsupported_protocol_version`: `{"supported": [1]}`. */
  details?: { [key: string]: unknown } | null;
  /** What sort of failure this is. Clients branch on it. */
  kind: ErrorKind;
  /** One or two sentences for a person. */
  message: string;
  /** The provider's id for the failed request, for its support channel. */
  request_id?: string | null;
  /** When the provider said the request may be tried again, if it said so and the harness did not wait itself. */
  retry_at?: Timestamp | null;
}

/** What sort of failure an error is. It decides whether the harness retries, which exit code a headless run ends with, and what a client should offer the user. */
export type ErrorKind = "usage" | "config" | "auth" | "provider" | "transient" | "model_stop" | "permission" | "limit" | "context_overflow" | "tool" | "interrupted" | "internal" | (string & {});

/** One event of a session. */
export type Event = ({
    type: "turn.started";
  } & TurnStarted) | ({
    type: "item.started";
  } & ItemStarted) | ({
    type: "item.delta";
  } & ItemDelta) | ({
    type: "item.reset";
  } & ItemRef) | ({
    type: "item.completed";
  } & ItemCompleted) | ({
    type: "item.resync";
  } & ItemRef) | ({
    type: "usage.updated";
  } & UsageUpdated) | ({
    type: "plan.updated";
  } & PlanUpdated) | ({
    type: "project.updated";
  } & ProjectUpdated) | ({
    type: "notice";
  } & Notice) | ({
    type: "request.resolved";
  } & RequestResolved) | ({
    type: "turn.completed";
  } & TurnCompleted) | ({
    type: "session.idle";
  } & SessionIdle) | ({
    type: "session.updated";
  } & SessionUpdated) | ({
    type: "config.changed";
  } & ConfigChanged);

/** When a lease ends. */
export interface Expiry {
  /** For `at`: the moment. */
  at?: Timestamp | null;
  /** Which kind of end. */
  kind: ExpiryKind;
}

/** When a lease ends. */
export type ExpiryKind = "never" | "at" | "end_of_task" | "end_of_session";

/** Whose an extension is (since step 28). */
export type ExtensionSource = "user" | "project" | (string & {});

/** How an extension stands against its approval (since step 28; `docs/architecture/12-extensions.md`, the table of how a found extension stands). */
export type ExtensionStanding = "approved" | "changed" | "not_approved" | "turned_off" | "unjudged" | "not_used" | "project_not_trusted" | "gone" | "left_out" | (string & {});

/** `extensions/status` */
export interface ExtensionsStatusParams {
  /** The session whose extensions are asked about, which the client has open (since step 28). Absent: the user's own, as a session with no project would find them. */
  session_id?: SessionId | null;
}

/** The result of `extensions/status`. */
export interface ExtensionsStatusResult {
  /** The extensions: each the session found, left out or holds a gate of, with its standing (since step 28). */
  extensions: Array<ComponentStatus>;
  /** What could not be read, each a sentence: the user's pins or the trust store, the two stores of approvals (since step 28). */
  problems?: Array<string>;
}

/** What a lease allows. */
export interface Grant {
  /** What may be spent. */
  budget?: Budget;
  /** The level. A level that is not listed is refused. */
  level: "read-only" | "supervised" | "guided" | "autonomous" | "unrestricted";
  /** What no level relaxes. */
  limits?: HardLimits;
  /** The models that may be used. Empty means the configured ones. */
  models?: Array<ModelId>;
  /** The rules. */
  rules?: Array<Rule>;
  /** The sandbox policy. */
  sandbox: SandboxPolicy;
}

/** Who granted a lease. */
export interface Grantor {
  /** Which kind of grantor. */
  kind: GrantorKind;
  /** For `parent`: the lease this one was narrowed from. */
  lease_id?: LeaseId | null;
}

/** Who granted a lease. */
export type GrantorKind = "user" | "parent";

/** What no level relaxes. Set by the user in the grant and enforced by the harness. */
export interface HardLimits {
  /** Whether work may be merged into the repository's main branch. */
  merge_to_main?: boolean;
  /** The most sessions a team may run at once. */
  team_size?: number | null;
}

/** The level a headless run was held to, and why. */
export interface HeadlessCap {
  /** The open item that names what is left of it, by its file's name in `docs/outstanding/`. */
  item?: string | null;
  /** The level it ran at. */
  level: Level;
  /** Why, in words. */
  reason: string;
}

/** A record of headless mode's own, told apart by `type`. */
export type HeadlessRecord = ({
    type: "session";
  } & HeadlessStart) | ({
    type: "result";
  } & HeadlessResult);

/** How a headless run ended: the whole of `--output json`, and the last line of `jsonl`. It is written on success and on failure alike, and its `outcome` and the process's exit code always agree. */
export interface HeadlessResult {
  /** The run was held to a narrower level than its configuration's, and why (since step 28's third pull request: a headless run under a session's command is held to `read-only`). Absent when it was not. */
  capped?: HeadlessCap | null;
  /** What the run cost, one amount per basis, in whole nano-dollars: `usd_nanos / 1e9` is US dollars. Empty when no price is known. */
  costs?: Array<Cost>;
  /** Milliseconds from the start of the process to this record. */
  duration_ms: number;
  /** The failure, when there was one. */
  error?: ErrorData | null;
  /** The files the run created, changed or deleted, relative to the workspace, sorted. */
  files_changed: Array<string>;
  /** How the run ended. */
  outcome: TurnOutcome;
  /** The calls that were refused because nothing could approve them. */
  permission_denials: Array<PermissionDenial>;
  /** How many requests were made to a model. */
  requests: number;
  /** The text of the assistant's last message. Empty when it wrote none. */
  result: string;
  /** The version of the records' shape, 1 for this one. */
  schema: number;
  /** The session. Absent when the run failed before one was started. */
  session_id?: SessionId | null;
  /** Why the last response ended. */
  stop_reason?: StopReason | null;
  /** How many tool calls were made. */
  tool_calls: number;
  /** Tokens, by class. */
  usage: TokenUsage;
  /** What the project's checks said of the run's turn (since step 19). Absent when the turn ended with no verdict: it was interrupted, or ended another way before any round of checks ran. */
  verification?: HeadlessVerification | null;
}

/** The first line of `jsonl`: which session the run is, on which model, in which workspace. */
export interface HeadlessStart {
  /** The main role's model. */
  model: ModelId;
  /** The version of the records' shape, 1 for this one. */
  schema: number;
  /** The session the run is in. */
  session_id: SessionId;
  /** The workspace root: an absolute path. */
  workspace: string;
}

/** What the project's checks said of a headless run's turn (`docs/architecture/10-headless.md`, "`json`"; since step 19). */
export interface HeadlessVerification {
  /** The checks of the last round that ran, as each ended. */
  checks?: Array<CheckReport>;
  /** How many rounds of checks ran. */
  rounds?: number;
  /** The turn's verdict, as `turn.completed` gave it. */
  verdict: Verification;
}

/** A gating hook a session holds of an extension that does not stand as it was approved: on its event, its run fails, refusing what it gates (since step 28). */
export interface HeldGate {
  /** Its event: `tool.before` or `prompt.submit`. */
  event: string;
  /** The hook's name: `<extension>/hook.<event>`. */
  hook: string;
  /** Why it refuses, with the way out, as the model is told it. */
  why: string;
}

/** A hook, named where what it answered shows: a hook that asked, refused or replaced something (since step 28, `docs/architecture/12-extensions.md`, "Tier 3: hooks"). */
export interface HookReason {
  /** The hook's name: its `[hooks.<name>]`, or the one an extension's script hook is shown by. */
  hook: string;
  /** What it gave as its reason, or how it failed. Absent when it gave none. */
  reason?: string | null;
}

/** An image. Input only. */
export interface ImagePart {
  /** The stored artifact that holds the image. */
  artifact?: ArtifactId | null;
  /** The image bytes in base64. Exactly one of `data` and `artifact` is set. */
  data?: string | null;
  /** The media type, for example `image/png`. */
  media_type: string;
}

/** Notice: anything else worth telling the user. */
export interface InfoNotice {
  /** The notice in words. */
  message: string;
}

/** The first request on a connection. */
export interface InitializeParams {
  /** What the client can handle. */
  capabilities?: ClientCapabilities;
  /** The client. */
  client: PeerInfo;
  /** The oldest protocol version the client speaks. Absent: only `protocol_version`. */
  min_protocol_version?: number | null;
  /** The newest protocol version the client speaks. */
  protocol_version: number;
  /** The client's role. */
  role: ClientRole;
}

/** The server's answer to `initialize`. */
export interface InitializeResult {
  /** What the server offers. */
  capabilities: ServerCapabilities;
  /** The version the connection uses: the newest one both sides speak. */
  protocol_version: number;
  /** The server. */
  server: PeerInfo;
}

/** Notice: instruction files changed and will be re-read. */
export interface InstructionsNotice {
  /** The files, relative to the workspace root where they are inside it. */
  files?: Array<string>;
  /** The notice in words. */
  message: string;
}

/** One item of a session. */
export type Item = ({
    kind: "user_message";
  } & UserMessageItem) | ({
    kind: "assistant_message";
  } & AssistantMessageItem) | ({
    kind: "tool_result";
  } & ToolResultItem) | ({
    kind: "notice";
  } & NoticeItem) | ({
    kind: "summary";
  } & SummaryItem);

/** An item is complete. This is the authoritative content. */
export interface ItemCompleted {
  /** The item, in full. */
  item: Item;
}

/** Streaming content for an item. Sent only to clients that declared `deltas`. */
export interface ItemDelta {
  /** The content. */
  delta: Delta;
  /** The item. */
  item_id: ItemId;
}

/** One item of a session: a message, a tool result, a notice or a summary. */
export type ItemId = string;

/** The kind of an item, where only the kind is needed. */
export type ItemKind = "user_message" | "assistant_message" | "tool_result" | "notice" | "summary" | (string & {});

/** Names an item: the payload of `item.reset` and `item.resync`. */
export interface ItemRef {
  /** The item. */
  item_id: ItemId;
}

/** An item began. Its content follows as deltas, and then in full. */
export interface ItemStarted {
  /** For a tool result: the call it answers. */
  call_id?: CallId | null;
  /** The item. */
  item_id: ItemId;
  /** Its kind. */
  kind: ItemKind;
  /** For a tool result: the tool that is running. */
  tool?: string | null;
}

/** The error member of a failed response. */
export interface JsonRpcError {
  /** A JSON-RPC error code. -32000 for every application error. */
  code: number;
  /** For an application error (code -32000), an ErrorData. Otherwise optional detail. */
  data?: ErrorData | unknown;
  /** One sentence for a person. */
  message: string;
}

/** Any JSON-RPC message: one JSON object per line. */
export type JsonRpcMessage = JsonRpcRequest | JsonRpcNotification | JsonRpcResponse;

/** A JSON-RPC notification: a message that gets no answer. */
export interface JsonRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: unknown;
}

/** A JSON-RPC request: the receiver must answer it with a response that has the same id. */
export interface JsonRpcRequest {
  id: RequestId;
  jsonrpc: "2.0";
  method: string;
  params?: unknown;
}

/** The answer to a JSON-RPC request. It has `result` or `error`, never both. */
export interface JsonRpcResponse {
  error?: JsonRpcError;
  id: RequestId | null;
  jsonrpc: "2.0";
  result?: unknown;
}

/** Everything a session is allowed to do, as one value. */
export interface Lease {
  /** When the lease ends. */
  expires: Expiry;
  /** What is allowed. */
  grant: Grant;
  /** Who granted it. */
  granted_by: Grantor;
  /** The lease's id. */
  id: LeaseId;
  /** When the holder must stop and report. */
  stop_when?: Array<StopCondition>;
}

/** A lease: what a session is allowed to do. */
export type LeaseId = string;

/** How much a session may do without asking: who decides when no rule has settled the question. The command line calls a session's level its permission mode. */
export type Level = "read-only" | "supervised" | "guided" | "autonomous" | "unrestricted" | (string & {});

/** Which limit was reached. */
export type LimitKind = "requests" | "cost" | "tokens" | "time" | (string & {});

/** What a limit counts. */
export type LimitMeasure = "requests" | "tokens" | "input_tokens" | "output_tokens" | (string & {});

/** Notice: a configured limit was reached, or is near. */
export interface LimitNotice {
  /** Which limit. */
  limit: LimitKind;
  /** The notice in words. */
  message: string;
  /** Whether it was reached. `false` is a warning that it is near. */
  reached: boolean;
}

/** Where a limit figure came from. */
export type LimitSource = "headers" | "refresh" | (string & {});

/** One limit a provider reported: how much of it is left and when it resets. */
export interface LimitWindow {
  /** The limit. */
  limit?: number | null;
  /** What the limit counts. */
  measure: LimitMeasure;
  /** What is left of it. Absent when not known, which includes "the reset time has passed and no request has been made since". */
  remaining?: number | null;
  /** When it resets. */
  resets_at?: Timestamp | null;
  /** The speed tier the limit belongs to, when the provider keeps a separate limit for that tier. Absent for the model's ordinary limit. */
  speed?: Speed | null;
}

/** The result of `mcp/status`. */
export interface McpStatusResult {
  /** The servers. */
  servers: Array<ComponentStatus>;
}

/** One message of a conversation. */
export interface Message {
  /** For an assistant message: what produced it. */
  origin?: Origin | null;
  /** Its content, in order. */
  parts: Array<Part>;
  /** Who it is from. */
  role: Role;
}

/** A kind of input or output a model handles. */
export type Modality = "text" | "image" | (string & {});

/** Capability flags of a model. A flag that is absent is `false`. */
export interface ModelCapabilities {
  /** System messages may appear in the middle of a conversation. */
  mid_conversation_system?: boolean;
  /** The smallest prefix the provider will cache, in tokens. */
  min_cacheable_tokens?: number | null;
  /** The provider checks tool arguments against the schema. */
  strict_tools?: boolean;
  /** The provider can hold the model to an output schema. */
  structured_output?: boolean;
}

/** A model, as the catalog names it. Unique within its provider. */
export type ModelId = string;

/** A model, as the catalog describes it. */
export interface ModelInfo {
  /** Capability flags. */
  capabilities?: ModelCapabilities;
  /** The context window, in tokens. */
  context_window: number;
  /** The name to show. */
  display_name: string;
  /** The model's id at its provider. */
  id: ModelId;
  /** What it accepts as input. */
  input?: Array<Modality>;
  /** The most output tokens one response may have. */
  max_output: number;
  /** The rate card at `standard` speed. Absent when no price is known. */
  prices?: Prices | null;
  /** The wire dialect used to reach it, for example `anthropic-messages`. */
  protocol: string;
  /** The provider that serves it. */
  provider: ProviderId;
  /** The models whose opaque reasoning this model can read back. Its own id is not listed. */
  reads_reasoning_from?: Array<ModelId>;
  /** How reasoning depth is controlled. */
  reasoning: ReasoningControl;
  /** The speed tiers it offers besides `standard`. Empty for most models. A front end shows a speed control only when this is not empty. */
  speeds?: Array<SpeedOffer>;
}

/** The job a model is doing. Configuration names one model per role (`docs/architecture/14-adaptation.md`, "Model roles and routing"). */
export type ModelRole = "main" | "fast" | "judge" | (string & {});

/** The time a model's part of some turns took, in milliseconds. */
export interface ModelTime {
  /** From sending a request to the provider's answering it: the connection, the request's upload and the provider's queue. */
  connect_ms?: number;
  /** From the answer to the response's first event. */
  first_byte_ms?: number;
  /** Waiting to send a failed request again. */
  retry_ms?: number;
  /** From the first event to the response's end. */
  streaming_ms?: number;
  /** Waiting for a summary, when a compaction could not go on without one. */
  summary_ms?: number;
}

/** A session's usage for one provider, model, role and speed. */
export interface ModelUsage {
  /** The model. */
  model: ModelId;
  /** The provider. */
  provider: ProviderId;
  /** The role. */
  role: ModelRole;
  /** The speed the requests ran at. */
  speed?: Speed;
  /** What was used. */
  totals: UsageTotals;
}

/** The result of `models/list`. */
export interface ModelsListResult {
  /** Every model of every configured provider. */
  models: Array<ModelInfo>;
}

/** Whether sandboxed processes may use the network. There is no per-host list. */
export type NetworkAccess = "off" | "full";

/** The command of an entry the user writes. Added in step 17. It is read strictly, as [`NewEntry`] is: the command may come to run without asking. */
export interface NewCommand {
  /** The command line. */
  command: string;
  /** The directory it runs in, relative to the workspace root. */
  cwd?: string | null;
  /** What it is for. */
  purpose: CommandPurpose;
  /** Whether it only reads. */
  read_only?: boolean;
}

/** An entry the user writes: it is active at once. Added in step 17. **It is read strictly:** a field this build does not know, in the entry or in its command, refuses the request (`docs/decisions/0028-enforced-values-fail-closed.md`). An entry the user writes is accepted from the moment it is written, and a command among them becomes an allow rule once the project is trusted: a field that is not understood could be the one that narrows it, such as an expiry. Its `check` is read as every check is: it is stored and shown, and whatever runs checks puts each one to the permission engine. */
export interface NewEntry {
  /** A test that shows it still holds. */
  check?: EntryCheck | null;
  /** For a `command` entry: the command. */
  command?: NewCommand | null;
  /** What it is. */
  kind: EntryKind;
  /** The entry in words. */
  text: string;
}

/** Something transient that front ends should show. It is never sent to the model and is not an item. Every notice has a `message` in words, so a client can show a kind of notice it does not know. */
export type Notice = ({
    kind: "retry";
  } & RetryNotice) | ({
    kind: "compaction";
  } & CompactionNotice) | ({
    kind: "limit";
  } & LimitNotice) | ({
    kind: "sandbox";
  } & SandboxNotice) | ({
    kind: "instructions";
  } & InstructionsNotice) | ({
    kind: "speed_fallback";
  } & SpeedFallback) | ({
    kind: "speed_cooldown";
  } & SpeedCooldown) | ({
    kind: "info";
  } & InfoNotice) | ({
    kind: "verification";
  } & VerificationNotice) | ({
    kind: "prompt_returned";
  } & PromptReturnedNotice);

/** A message the harness added to the conversation. The model sees it. This is not the `notice` event ([`Notice`](crate::event::Notice)), which tells front ends about something transient and is never sent to the model. */
export interface NoticeItem {
  /** When it was recorded. */
  created_at?: Timestamp | null;
  /** The item's id. */
  id: ItemId;
  /** Why it was added. */
  source: NoticeSource;
  /** What the model is told. */
  text: string;
}

/** Why the harness added a notice to the conversation. */
export type NoticeSource = "instructions" | "plan" | "repetition" | "verification" | "limit" | "harness" | "map" | "profile" | "extensions" | "hook" | (string & {});

/** What produced an assistant message. Raw blocks are valid only for replay to the same protocol, and to a model that can read them. */
export interface Origin {
  /** The account or organisation the credential belonged to, where the provider ties opaque reasoning to it. Never the credential. */
  account_scope?: string | null;
  /** The model. */
  model: ModelId;
  /** The wire dialect, for example `anthropic-messages`. */
  protocol: string;
  /** The provider. */
  provider: ProviderId;
}

/** One part of a message. */
export type Part = ({
    kind: "text";
  } & TextPart) | ({
    kind: "image";
  } & ImagePart) | ({
    kind: "tool_call";
  } & ToolCallPart) | ({
    kind: "tool_result";
  } & ToolResultPart) | ({
    kind: "reasoning";
  } & ReasoningPart) | ({
    kind: "provider_block";
  } & ProviderBlockPart);

/** A set of paths: everything, or the trees under some roots. */
export interface PathSet {
  /** Every path. When set, `roots` is ignored. */
  all?: boolean;
  /** The roots of the trees in the set. */
  roots?: Array<string>;
}

/** The name and version of one side of a connection. */
export interface PeerInfo {
  /** The program's name. */
  name: string;
  /** Its version. */
  version: string;
}

/** A tool call that was refused because nothing could approve it: the usual reason a headless run does less than was expected. */
export interface PermissionDenial {
  /** The call. */
  call_id: CallId;
  /** Why it needed an answer. */
  reason: string;
  /** The call in one line. */
  summary: string;
  /** The tool. */
  tool: string;
  /** The turn the call belonged to. */
  turn_id: TurnId;
}

/** The result of `permissions/list`. */
export interface PermissionsListResult {
  /** The approvals in force for the session. */
  approvals: Array<StoredApproval>;
}

/** `permissions/revoke` */
export interface PermissionsRevokeParams {
  /** The approvals to remove. */
  ids: Array<ApprovalId>;
  /** The session. */
  session_id: SessionId;
}

/** The plan for the current work: a task list that the agent keeps and the user can see and change. */
export interface Plan {
  /** The tasks, in order. */
  items?: Array<PlanItem>;
  /** Goes up by one with every change, so a client can tell which of two plans is newer. */
  revision?: number;
}

/** Adds a plan item. */
export interface PlanAdd {
  /** The item it goes after. Absent: at the end. */
  after?: PlanItemId | null;
  /** The task. */
  text: string;
}

/** One edit a user makes to the plan. It takes effect at once, and reaches the model as a notice at the next safe boundary. */
export type PlanEdit = ({
    kind: "reorder";
  } & PlanReorder) | ({
    kind: "skip";
  } & PlanItemRef) | ({
    kind: "restore";
  } & PlanItemRef) | ({
    kind: "reword";
  } & PlanReword) | ({
    kind: "add";
  } & PlanAdd) | ({
    kind: "remove";
  } & PlanItemRef);

/** One task of a plan. */
export interface PlanItem {
  /** The task's id, stable across edits. */
  id: PlanItemId;
  /** Where it stands. */
  status: PlanStatus;
  /** The task. */
  text: string;
}

/** One item of a plan. */
export type PlanItemId = string;

/** Names one plan item: the payload of `skip` and `remove`. */
export interface PlanItemRef {
  /** The item. */
  id: PlanItemId;
}

/** Puts the plan's items in a new order. */
export interface PlanReorder {
  /** Every item's id, in the new order. */
  order: Array<PlanItemId>;
}

/** A result that is the plan. */
export interface PlanResult {
  /** The plan now. */
  plan: Plan;
}

/** Replaces a plan item's text. */
export interface PlanReword {
  /** The item. */
  id: PlanItemId;
  /** The new text. */
  text: string;
}

/** Where a plan item stands. */
export type PlanStatus = "pending" | "active" | "done" | "skipped" | (string & {});

/** `plan/update` */
export interface PlanUpdateParams {
  /** The edits, applied in order. */
  edits: Array<PlanEdit>;
  /** The session. */
  session_id: SessionId;
}

/** The plan changed. */
export interface PlanUpdated {
  /** Who changed it. */
  by: Actor;
  /** The plan now. */
  plan: Plan;
}

/** The prices of a request whose input is larger than a threshold. */
export interface PriceTier {
  /** The tier applies when the request's input tokens exceed this. */
  above_input_tokens: number;
  /** Cache reads, nano-dollars per million. */
  cache_read?: number | null;
  /** Cache writes per lifetime class. */
  cache_write?: Array<CacheWritePrice>;
  /** Input tokens, nano-dollars per million. */
  input: number;
  /** Output tokens, nano-dollars per million. */
  output: number;
}

/** One rate card: what a million tokens of each class cost. Every price is a whole number of nano-dollars (1e-9 US dollars) per million tokens, so `4_000_000_000` is $4.00 per million. */
export interface Prices {
  /** Input tokens read from the cache. Absent when the model has no cache price. */
  cache_read?: number | null;
  /** Input tokens written to the cache, per lifetime class. */
  cache_write?: Array<CacheWritePrice>;
  /** Input tokens that were not read from or written to the cache. */
  input: number;
  /** Output tokens, reasoning included. */
  output: number;
  /** Rate cards that replace this one for larger requests, by ascending threshold. The last tier whose threshold the request passes applies. */
  tiers?: Array<PriceTier>;
}

/** `project/detect`: the build, test and check commands a session's project names in its own files (`Cargo.toml`, `package.json`'s scripts, a `Makefile`'s targets, `go.mod`, `pyproject.toml`), for the first run's last step (since step 25). A read, beside the lane, for any client that has the session open, as `workspace/files` is: the files are read through the session's path resolver and its read rules, and nothing is kept, learned or run. */
export interface ProjectDetectParams {
  /** The session. */
  session_id: SessionId;
}

/** The result of `project/detect`. */
export interface ProjectDetectResult {
  /** The commands found, by file in the order the files are looked at, each named once. Only lines of plain words that the command observer classes as a build, a test run or a check are given. */
  commands: Array<DetectedCommand>;
}

/** A result that is project entries. */
export interface ProjectEntriesResult {
  /** The entries: all of them for `project/list`, the changed ones for `project/update`. */
  entries: Array<ProjectEntry>;
  /** What is wrong with the project's files, each in a sentence: a file that cannot be read as it is, and what is used in its place. Added in step 17. */
  problems?: Array<string>;
}

/** One entry of the project profile or memory. */
export interface ProjectEntry {
  /** When the user accepted it. For a `command` entry this is what puts it on the project's safe list; an entry that became active by itself has none. Added in step 17. */
  accepted_at?: Timestamp | null;
  /** How the harness came to hold it. Absent for an entry the user wrote. */
  basis?: EntryBasis | null;
  /** A test that shows it still holds. */
  check?: EntryCheck | null;
  /** For a `command` entry: the command. */
  command?: EntryCommand | null;
  /** When it was last seen to hold. */
  confirmed_at?: Timestamp | null;
  /** Where it was learned: a path and line, a command, or the user's words. */
  evidence?: string | null;
  /** When it stops being used unless confirmed again. Entries the user wrote do not expire. */
  expires_at?: Timestamp | null;
  /** The entry's id. */
  id: EntryId;
  /** What it is. */
  kind: EntryKind;
  /** For an observed entry: how many times the harness saw it hold. Added in step 17. */
  observations?: number | null;
  /** Which file it lives in. */
  scope: EntryScope;
  /** Where it came from. */
  source: EntrySource;
  /** Whether it is in use. */
  status: EntryStatus;
  /** The entry in words: the rule, the fact, the test's name, or what the command is. */
  text: string;
}

/** The key that names a project in the state directory. Never a path. */
export type ProjectKey = string;

/** Names a project by a session of it, or by its workspace root. Exactly one is set. */
export interface ProjectListParams {
  /** A session of the project. */
  session_id?: SessionId | null;
  /** The project's workspace root. */
  workspace?: string | null;
}

/** `project/update` */
export interface ProjectUpdateParams {
  /** Entries the user writes, added after the changes. Each is active at once. Added in step 17. */
  add?: Array<NewEntry>;
  /** The changes, applied in order. */
  changes?: Array<EntryChange>;
  /** A session of the project. Exactly one of this and `workspace` is set. */
  session_id?: SessionId | null;
  /** The project's workspace root. */
  workspace?: string | null;
}

/** Project profile or memory entries were proposed or changed. */
export interface ProjectUpdated {
  /** The entries that are new or changed, as they are now. */
  entries?: Array<ProjectEntry>;
  /** The entries that were deleted. */
  removed?: Array<EntryId>;
}

/** The command a prompt came from (since step 28). */
export interface PromptCommand {
  /** What was typed after the command's name. */
  args?: string;
  /** The extension whose script gave the text, for an extension's command. */
  extension?: string | null;
  /** The command's name as it is typed after `/`: a file's (`review`), or an extension's, its extension's name and a `:` before it (`tracker:deploy`). */
  name: string;
  /** Whose it is. */
  source: CommandSource;
}

/** Notice: a prompt given back to the front end, for its editor, and never sent to the model (since step 28). A `prompt.submit` hook refused it, and `hooks` names each that did, with its reason; or the turn was interrupted while its hooks were asked, and `hooks` is empty. A prompt that would have started a turn ends that turn `denied` (or `interrupted`); a steering message is dropped, and the running turn goes on. */
export interface PromptReturnedNotice {
  /** The command the prompt came from, when it was a command's (since step 28): a front end puts back what was typed, `/name args`, in place of `parts`, its expanded text. Sent, as `parts` are, to the client that sent the prompt alone. */
  command?: PromptCommand | null;
  /** How it was sent while a turn ran, as its user message would have said: `steer` for a steering message, whether it was refused at a boundary of the running turn or as the prompt of a turn of its own (one that found no boundary); `queue` for a queued prompt. Absent for a prompt sent to an idle session. */
  delivery?: Delivery | null;
  /** The hooks that refused it, each with its reason or its failure. */
  hooks?: Array<HookReason>;
  /** The notice in words: which hooks refused the prompt, and why. */
  message: string;
  /** The prompt, as it was sent. */
  parts: Array<Part>;
}

/** One pin of an [`ApprovalProposal`]. */
export interface ProposedPin {
  /** The hash of what is approved. */
  hash: string;
  /** What kind of thing it is. */
  kind: TrustResourceKind;
  /** Its name. */
  name: string;
  /** The hash of what it registered, for an extension evaluated before the question. */
  registration?: string | null;
}

/** Something the provider produced that the neutral model does not name: server-side tool use and its results, compaction blocks, fallback markers. */
export interface ProviderBlockPart {
  /** The provider's name for the block type, for display. */
  block_type?: string | null;
  /** The provider's own block. */
  raw: RawJson;
}

/** A provider: an endpoint that speaks a protocol, by its configured name. */
export type ProviderId = string;

/** The limits one provider last reported. A provider that reports none has no entry, and a front end shows "not available". */
export interface ProviderLimits {
  /** Whether the account or key may make calls now, where the provider says. Added in step 16. */
  available?: boolean | null;
  /** Money left, one entry a currency, where the provider's usage endpoint reports it (the opt-in refresh). Amounts in different currencies are never added together or converted. Added in step 16. */
  balances?: Array<Balance>;
  /** When the provider's next accounting period begins, where it names one without saying which limits it resets (Venice.ai's "epoch"). It is not a limit's reset time. Added in step 16. */
  next_epoch?: Timestamp | null;
  /** The provider. */
  provider: ProviderId;
  /** When the figures were seen. */
  seen_at: Timestamp;
  /** Where they came from. */
  source: LimitSource;
  /** The limits. */
  windows: Array<LimitWindow>;
}

/** `question/ask`: a question for the user, from the agent or an extension. */
export interface QuestionRequest {
  /** Answers to pick from. Empty: free text. */
  choices?: Array<string>;
  /** The extension that asks. Absent: the agent. */
  extension?: string | null;
  /** The question. */
  question: string;
  /** The session. */
  session_id: SessionId;
  /** The turn, when the question belongs to one. */
  turn_id?: TurnId | null;
}

/** The answer to `question/ask`. */
export interface QuestionResponse {
  /** The answer. Absent: the user declined to answer. */
  answer?: string | null;
}

/** JSON text kept exactly as a provider sent it, carried as a string so that no parser on the way can reorder keys, change escapes or round numbers. Replay sends it back unchanged. Clients treat it as opaque. */
export type RawJson = string;

/** A model's reasoning control. Depth is a per-model set, never a number on a shared scale. */
export interface ReasoningControl {
  /** For `levels`: the level used when none is set. */
  default?: string | null;
  /** Which kind of control this is. */
  kind: ReasoningControlKind;
  /** For `levels`: the model's own level names, from least to most. */
  levels?: Array<string>;
  /** For `budget`: the largest budget, in tokens. */
  max_tokens?: number | null;
  /** For `budget`: the smallest budget, in tokens. */
  min_tokens?: number | null;
}

/** How a model's reasoning depth is controlled. */
export type ReasoningControlKind = "none" | "levels" | "budget" | (string & {});

/** Reasoning by the model. */
export interface ReasoningPart {
  /** The provider's own block, which holds the opaque payload (a signature, encrypted content). It is replayed unchanged to a model that can read it and dropped for any other. */
  raw?: RawJson | null;
  /** The visible reasoning or its summary. Often empty. */
  text?: string;
}

/** What one provider's usage endpoint answered. */
export interface RefreshOutcome {
  /** Why it did not. The figures kept from before stay as they were. */
  error?: ErrorData | null;
  /** The figures, when the refresh worked. They are also kept, and `usage/report` shows them from then on. */
  limits?: ProviderLimits | null;
  /** The provider. */
  provider: ProviderId;
}

/** Arguments a `tool.before` hook put in place of the ones the model wrote (since step 28). Such a call is always put to the user, who may allow it once or refuse it. */
export interface ReplacedArguments {
  /** The arguments the call runs with: the hook's. */
  arguments: unknown;
  /** The hook that replaced them. */
  hook: string;
  /** The arguments the model wrote, where the message that carries the call is not at hand: in an approval request. Absent in a tool result, whose call in the assistant message has them. */
  original?: unknown;
  /** What the hook gave as its reason. */
  reason?: string | null;
}

/** The id that ties a response to its request: a number or a string, chosen by the side that sends the request. */
export type RequestId = number | string;

/** A request from the server (an approval, a question, a trust prompt) was settled without this client. The client withdraws its prompt. */
export interface RequestResolved {
  /** The JSON-RPC id of the request, as this client received it. */
  request_id: RequestId;
  /** How it was settled. */
  resolution: Resolution;
}

/** The usage of one request to a model: what was used, at which speed, and what it cost. */
export interface RequestUsage {
  /** How the usage is paid for. */
  billing: Billing;
  /** The version of the catalog whose prices were used. */
  catalog_version?: string | null;
  /** The cost, fixed when the request was recorded. Absent when the model has no known price. */
  cost?: Cost | null;
  /** Milliseconds the request took. */
  duration_ms?: number | null;
  /** The model. */
  model: ModelId;
  /** The provider. */
  provider: ProviderId;
  /** The provider's own usage object. */
  raw?: RawJson | null;
  /** The role the request was made in. */
  role: ModelRole;
  /** The speed the request **ran at**: the provider's own report where it gives one, otherwise the speed that was sent. The cost is priced at this speed's rate. */
  speed?: Speed;
  /** The speed that was asked for, when the request ran at another one. */
  speed_requested?: Speed | null;
  /** Milliseconds from sending the request to the first token. */
  time_to_first_token_ms?: number | null;
  /** Tokens by class. */
  tokens: TokenUsage;
}

/** How a request from the server to its clients was settled. */
export type Resolution = "answered" | "timed_out" | "cancelled" | (string & {});

/** Notice: a request failed and will be tried again. */
export interface RetryNotice {
  /** Which attempt comes next, counting from 1. */
  attempt: number;
  /** Why the last attempt failed. */
  error: ErrorData;
  /** The most attempts a request gets. */
  max_attempts: number;
  /** The notice in words. */
  message: string;
  /** When the next attempt starts. */
  next_attempt_at: Timestamp;
}

/** One file of a rewind's plan. */
export interface RewindFile {
  /** The file as the session's checkpoints name it: relative to the workspace's root, or absolute outside it. */
  path: string;
  /** Whether it can be put back now. */
  status: RewindFileStatus;
  /** What it would go back to. */
  to: RewindTarget;
  /** Why it cannot be put back, when `status` is `cannot`. */
  why_not?: RewindWhyNot | null;
}

/** Whether a file of a rewind's plan can be put back now. */
export type RewindFileStatus = "as_left" | "already" | "edited_since" | "cannot" | (string & {});

/** What a rewind would put a file back to. */
export type RewindTarget = "content" | "removed" | "not_recorded" | (string & {});

/** Why a file of a rewind's plan cannot be put back. */
export type RewindWhyNot = "not_recorded" | "too_large" | "missing" | "unreadable" | "through_a_link" | (string & {});

/** Who a message is from. */
export type Role = "user" | "assistant" | "tool" | (string & {});

/** A change to the settings of one model role. A field that is absent is left as it is. */
export interface RoleSettings {
  /** The reasoning level: one of the model's own levels, or a portable alias (`minimal`, `low`, `medium`, `high`, `max`). */
  effort?: string | null;
  /** The model. */
  model?: ModelId | null;
  /** The provider that serves the model. Needed only when two providers have a model with the same id. */
  provider?: ProviderId | null;
  /** The speed for this role. For `main` it overrides the session's speed. The utility roles (`fast`, `judge`) do not follow the session's speed: they run at `standard` unless this is set. */
  speed?: Speed | null;
}

/** What one model role of a session is set to, and what is in effect. */
export interface RoleState {
  /** The reasoning level as it was set. */
  effort?: string | null;
  /** The model's own level that the setting maps to. */
  effort_in_effect?: string | null;
  /** The model. */
  model: ModelId;
  /** The provider. */
  provider: ProviderId;
  /** The speed this role is set to: its own setting; without one, the session's speed for `main` and `standard` for a utility role. Required, so that a reader never has to guess a default. */
  speed: Speed;
  /** When a cooldown of `speed` ends, while one is running. */
  speed_cooldown_until?: Timestamp | null;
  /** The speed its next request will ask for. It differs from `speed` when the model does not offer that speed, or while the speed is in a cooldown. Required. */
  speed_in_effect: Speed;
}

/** A permission rule. `deny` beats `ask` beats `allow`, whatever the order. */
export interface Rule {
  /** What it decides. */
  action: RuleAction;
  /** The effect the rule is about. An effect that is not listed is refused. */
  effect: "read" | "write" | "exec" | "network" | "ext";
  /** What it matches: a path glob, a command pattern, a host, or a `server/tool` name. */
  match: string;
}

/** What a rule decides. */
export type RuleAction = "allow" | "ask" | "deny";

/** Notice: something about the sandbox the user should know: it is not enforcing, a command ran outside it, a protected path changed. */
export interface SandboxNotice {
  /** The notice in words. */
  message: string;
  /** Whether the sandbox is enforcing. */
  status: SandboxStatus;
}

/** What a child process may touch, whatever it runs. */
export interface SandboxPolicy {
  /** Whether the network may be used. */
  network: NetworkAccess;
  /** Paths inside `write` that must not be changed. */
  protect?: PathSet;
  /** What may be read. */
  read: PathSet;
  /** What may be written. */
  write: PathSet;
}

/** Whether the OS sandbox confines child processes on this machine. */
export type SandboxStatus = "enforcing" | "off" | (string & {});

/** What a server offers. */
export interface ServerCapabilities {
  /** The experimental features it turned on for this connection: the ones the client asked for that it has. */
  experimental?: Array<string>;
  /** It was built with extensions. */
  extensions?: boolean;
  /** It was built with the MCP client. */
  mcp?: boolean;
  /** Whether its sandbox confines commands. */
  sandbox: SandboxStatus;
  /** It stores sessions. */
  sessions?: boolean;
}

/** `session/artifact`: a stored artifact of a session, such as the whole output of a check. Added in step 24. */
export interface SessionArtifactParams {
  /** The artifact. */
  artifact: ArtifactId;
  /** The session. */
  session_id: SessionId;
}

/** The result of `session/artifact`. */
export interface SessionArtifactResult {
  /** Its whole size in bytes. */
  bytes: number;
  /** Whether `text` holds less than all of it. */
  cut?: boolean;
  /** Its text, as stored, up to the server's limit. Not UTF-8 where it is stored is replaced with `U+FFFD`. */
  text: string;
}

/** `session/compact`: an idle session compacts in a turn of its own that sends nothing else; a running one at its next safe boundary. A turn whose last request is under way has none left: when it ends, the turn that follows takes the compaction at its first boundary, and when none follows, a turn that only compacts runs. An interrupt drops it. */
export interface SessionCompactParams {
  /** What the summary should keep in particular. */
  focus?: string | null;
  /** The session. */
  session_id: SessionId;
}

/** An event with its envelope: which session, where in its sequence, and which turn. The event's own fields sit beside the envelope's. */
export type SessionEvent = {
    /** Goes up by one per event per session. A gap means events were missed. */
    seq: number;
    session_id: SessionId;
    turn_id?: TurnId;
  } & Event;

/** `session/fork`. Names where the new session ends by at most one of `at` and `before`; with neither it keeps all of the session. Both are refused (`invalid_params`). */
export interface SessionForkParams {
  /** The last item the new session keeps. */
  at?: ItemId | null;
  /** The item the new session ends just before (since step 25): it keeps what the session's log holds before that item's entry, and nothing of the item or after it. The id of a prompt that began a turn forks from before the turn's first entry: what the turn recorded before its prompt (the notices that waited for it, the epoch's map) is left out with it, and whatever the log holds between the turn before and the turn (that turn's last checkpoint, a plan the person edited) is kept. A core from before step 25 ignores it, and forks all of the session: its result's `before` is then absent. */
  before?: ItemId | null;
  /** The session to fork. */
  session_id: SessionId;
}

/** The result of `session/fork`: the new session's state, and where it ends when it was asked to end before an item (since the review of the fixes of step 25's second pull request). */
export interface SessionForkResult {
  /** The `before` of the request, given back when the new session ends just before that item. A client that asked for `before` takes a result without it for a fork of all of the session: a core that does not know the field ignores it, and forks all of the session, as `session/read`'s `redacted` is given back for the same reason. */
  before?: ItemId | null;
  /** The new session's state. */
  session: SessionState;
}

/** A stored session. */
export type SessionId = string;

/** The session has no running turn and no queued prompt. */
export interface SessionIdle {
  [key: string]: unknown;
}

/** `session/list` */
export interface SessionListParams {
  /** Where to continue: the `next_cursor` of the page before. */
  cursor?: string | null;
  /** The most sessions to return. */
  limit?: number | null;
  /** Only sessions of this workspace. Absent: every workspace. */
  workspace?: string | null;
}

/** The result of `session/list`: the session added to last first (by `updated_at`, the id breaking a tie), not the one made last. */
export interface SessionListResult {
  /** Present when there are more. */
  next_cursor?: string | null;
  /** The sessions. */
  sessions: Array<SessionSummary>;
}

/** `session/new` */
export interface SessionNewParams {
  /** Configuration overrides for this session, as the configuration file would have them. They rank as command-line flags do. */
  config?: { [key: string]: unknown };
  /** Where the session's commands start when they name no directory: an absolute path inside `workspace` (the directory the person started the harness in, below the root it found). Absent: the root. Kept in the session's log, so that a resumed session starts there too (since the follow-ups of step 23). */
  cwd?: string | null;
  /** Whether to keep the session in memory only, with no log. */
  ephemeral?: boolean;
  /** The settings to start with. Anything absent comes from configuration. */
  settings?: SessionSettings;
  /** A title. */
  title?: string | null;
  /** The workspace root: an absolute path on the server's machine. */
  workspace: string;
}

/** `session/read` */
export interface SessionReadParams {
  /** Where to continue: the `next_cursor` of the page before. */
  cursor?: string | null;
  /** Read only this item. An item in progress comes back with the content it has so far. */
  item_id?: ItemId | null;
  /** The most items to return. */
  limit?: number | null;
  /** For an export (since step 25): every item's text masked by the core's own secret filter, as the session log's writer masks it, the provider's blocks (`raw`) left out, and the items still in progress left out (named in `incomplete`). A session kept only in memory is masked the same. The result says it was masked (its `redacted`). */
  redacted?: boolean;
  /** The session. */
  session_id: SessionId;
}

/** The result of `session/read`: items of the active branch, oldest first. */
export interface SessionReadResult {
  /** The ones among them that are still in progress. */
  incomplete?: Array<ItemId>;
  /** The items. */
  items: Array<Item>;
  /** With `redacted`: how many values are masked in `items`, by the log as it was written or as they were read (since step 25). */
  masked?: number;
  /** Present when there are more. */
  next_cursor?: string | null;
  /** The items were masked for an export, as `redacted` asked (since step 25). A client that asked for `redacted` writes nothing without it: a server that does not know the field reads the items unmasked, and its result reads `false` here. */
  redacted?: boolean;
  /** The `seq` of the last event sent for the session when the items were read. Events after it are not in this result. */
  seq: number;
}

/** Names a session: the parameters of every method that takes nothing else. */
export interface SessionRef {
  /** The session. */
  session_id: SessionId;
}

/** A result that is the session's state. */
export interface SessionResult {
  /** The session's state. */
  session: SessionState;
}

/** `session/rewind`: moves the active branch back to an item, or to just before one, putting files back if asked. The point is named by exactly one of `to` and `before`: both, or neither, are refused (`invalid_params`). **It may ask a question** (since step 23). With `restore_files`, a file edited since the harness left it is put back only if the person says so: the session's `user` clients that answer questions are put one `question/ask` naming those files, with the choices "Put them back" and "Keep them as they are". No client to ask, no answer, a client that goes away, the session's close and the core's end all keep the files. The result comes once the question is settled; a session closed meanwhile is not moved back (`session_not_open`). */
export interface SessionRewindParams {
  /** The item of the active branch to go back to just before (since step 25): the active position becomes the entry the item's own follows, and everything from the item on is undone, nothing before it. The id of a prompt that began a turn goes back to before the turn's first entry, where `session/undo` goes back to when that turn is the last: the turn and every later one are undone, what the turn recorded before its prompt (the notices that waited for it, the epoch's map) with them, and what the log holds between the turn before and the turn (that turn's last checkpoint, a plan the person edited) is kept. A core from before step 25 refuses a rewind with `before` alone, for want of `to`. */
  before?: ItemId | null;
  /** The `mark` of the plan `session/rewind_plan` gave, which the person was shown (since step 25). The plan is made again, and the rewind is refused (`plan_changed`), changing nothing, when it is not that one: when the point it goes back to, the active position it goes back from or its turns differ, and with `restore_files` also when what it would do to any file does. Absent: no plan is checked. */
  plan?: string | null;
  /** Whether to put files back as they were at that point, from the snapshots of the turns that are undone. */
  restore_files?: boolean;
  /** The session. */
  session_id: SessionId;
  /** The last item to keep on the active branch. */
  to?: ItemId | null;
}

/** `session/rewind_plan`: what `session/rewind` to the same point would do now, changing nothing (since step 25). A front end shows it before it asks whether to put files back: the turns it would undo, and each file they changed with what it would go back to. The point is named as the rewind names it, by exactly one of `to` and `before`. Refused as `session/rewind` would be: `invalid_params` for both or neither, `role_not_allowed` for an observer, `session_not_stored` for a session with no log, `unknown_item`, `turn_running` while a turn runs, and `history_changing` while another change of the history is under way (a rewind's question still open). */
export interface SessionRewindPlanParams {
  /** The item of the active branch the rewind would go back to just before, as `session/rewind`'s `before`. */
  before?: ItemId | null;
  /** The session. */
  session_id: SessionId;
  /** The last item the rewind would keep on the active branch. */
  to?: ItemId | null;
}

/** The result of `session/rewind_plan`: the plan the core would carry out (`docs/architecture/07-sessions-and-context.md`, "Snapshots and undo"). */
export interface SessionRewindPlanResult {
  /** The files those turns changed, by name. */
  files?: Array<RewindFile>;
  /** The plan in a few characters: given back as `session/rewind`'s `plan`, it has that rewind refused when the plan has changed since. It holds the point the rewind goes back to, the active position it goes back from, the turns it undoes and what it does to each file. */
  mark: string;
  /** The turns it would undo, wholly or in part, oldest first. */
  turns: Array<TurnId>;
  /** A checkpoint of those turns cannot be read: what they changed is not wholly known, and no file would be put back. */
  unreadable?: boolean;
}

/** `session/set` */
export interface SessionSetParams {
  /** The session. */
  session_id: SessionId;
  /** What to change. Anything absent stays as it is. */
  settings: SessionSettings;
}

/** A change to a session's settings: the parameters of `session/set`, and the settings a new session starts with. A field that is absent is left as it is. */
export interface SessionSettings {
  /** The level (permission mode). Raising it needs the `user` role. */
  level?: Level | null;
  /** Settings per model role. */
  roles?: { [key: string]: RoleSettings };
  /** The session's speed: what the `main` role runs at unless it sets its own. The utility roles do not inherit it. */
  speed?: Speed | null;
}

/** A session's metadata and current state. */
export interface SessionState {
  /** When it was created. */
  created_at: Timestamp;
  /** The lease in force. `lease/get` returns its terms. */
  lease_id?: LeaseId | null;
  /** The level of the lease in force. */
  level: Level;
  /** The session it was forked from. */
  parent?: SessionId | null;
  /** How many prompts are queued behind the running turn. */
  queue_length?: number;
  /** The model roles. */
  roles: { [key: string]: RoleState };
  /** The turn that is running, if one is. */
  running_turn?: TurnId | null;
  /** Whether the sandbox confines the session's commands. */
  sandbox: SandboxStatus;
  /** The `seq` of the last event sent for this session. Events after it are live. */
  seq: number;
  /** The session. */
  session_id: SessionId;
  /** The session's speed setting: what the `main` role runs at unless it sets its own. Required. */
  speed: Speed;
  /** The session's title. */
  title?: string | null;
  /** What the user has said of the project's trust, for a front end's status (since step 23). It informs only: what trust turns on is the core's to decide. */
  trust?: TrustStanding | null;
  /** The workspace root. An absolute path on the server's machine. */
  workspace: string;
  /** The directories the session's commands may write to, as absolute paths: the workspace, the session's temporary directory and any the configuration adds. A front end keeps a program it runs for the person (an external editor) out of them, since a command could have put one there (since step 23). The list is the same whether or not commands are confined: where they are not (the sandbox off, the `unrestricted` level, or a system the sandbox does not confine yet) a command may write anywhere, and keeping out of these protects nothing then. */
  write_roots?: Array<string>;
}

/** A stored session, in a list. */
export interface SessionSummary {
  /** When it was created. */
  created_at: Timestamp;
  /** The main model it last used. */
  model?: ModelId | null;
  /** The session it was forked from. */
  parent?: SessionId | null;
  /** The session. */
  session_id: SessionId;
  /** The session's title. */
  title?: string | null;
  /** How many turns it has. */
  turns?: number;
  /** When it was last added to. */
  updated_at: Timestamp;
  /** The workspace root. */
  workspace: string;
}

/** `session/undo`: steps back before the session's last turn, its first included, and records the step as an undo (since step 23). **It may ask a question** (since step 23). With `restore_files`, a file edited since the harness left it is put back only if the person says so: the session's `user` clients that answer questions are put one `question/ask` naming those files, with the choices "Put them back" and "Keep them as they are". No client to ask, no answer, a client that goes away, the session's close and the core's end all keep the files. The result comes once the question is settled; a session closed meanwhile is not moved back (`session_not_open`). */
export interface SessionUndoParams {
  /** Whether to put files back as they were before the turn, from its snapshots. */
  restore_files?: boolean;
  /** The session. */
  session_id: SessionId;
}

/** Session metadata changed: title, models, speed, level, queue length. */
export interface SessionUpdated {
  /** The session's state now. */
  session: SessionState;
}

/** The live usage of a session: what `usage/get` returns and `usage.updated` carries. */
export interface SessionUsage {
  /** The same, per provider, model, role and speed. */
  by_model?: Array<ModelUsage>;
  /** Cache reads as a share of all input tokens sent, from 0 to 1. */
  cache_hit_rate?: number | null;
  /** What the context of the most recent request to the main model held, by category: estimates, as the harness counts them. Added in step 24. */
  context?: ContextBreakdown | null;
  /** The tokens in the context of the most recent request to the main model. */
  context_tokens?: number | null;
  /** The most recent request. */
  last_request?: RequestUsage | null;
  /** The latest limits each provider reported. */
  limits?: Array<ProviderLimits>;
  /** Output tokens per second of the most recent request. */
  tokens_per_second?: number | null;
  /** Everything the session has used, out-of-band requests included (compaction, titles). */
  totals?: UsageTotals;
}

/** A speed tier, by name. Which ones a model offers is in its [`ModelInfo`](crate::model::ModelInfo). */
export type Speed = "standard" | "fast" | (string & {});

/** Notice: a speed tier's cooldown began or ended. During a cooldown the harness sends requests at `standard` and does not flip back and forth, because each change of speed can cost a prompt-cache rewrite. */
export interface SpeedCooldown {
  /** The notice in words. */
  message: string;
  /** The model. */
  model: ModelId;
  /** The provider. */
  provider: ProviderId;
  /** The tier that is cooling down. */
  speed: Speed;
  /** Whether the cooldown began or ended. */
  state: CooldownState;
  /** When it ends, for one that began. */
  until?: Timestamp | null;
}

/** What a speed tier buys, compared with `standard`. */
export type SpeedEffect = "faster_output" | "priority" | "deferred" | (string & {});

/** Notice: a request ran, or will run, at a different speed from the one the session asked for. The usage record of the request says the same thing in numbers: it is priced at the speed it ran at. */
export interface SpeedFallback {
  /** Who moved it. */
  by: SpeedFallbackBy;
  /** When the harness will ask for the requested speed again, if it has put the tier in a cooldown. */
  cooldown_until?: Timestamp | null;
  /** The notice in words, so that a client that does not know this kind of notice can still show it. */
  message: string;
  /** The model. */
  model: ModelId;
  /** The provider. */
  provider: ProviderId;
  /** Why. */
  reason: SpeedFallbackReason;
  /** The speed the session asked for. */
  requested: Speed;
  /** The role whose request it was. */
  role: ModelRole;
  /** The speed the request ran at, or will run at. */
  served: Speed;
}

/** Who moved a request to another speed. */
export type SpeedFallbackBy = "harness" | "provider" | (string & {});

/** Why a request did not run at the speed that was asked for. */
export type SpeedFallbackReason = "rate_limited" | "overloaded" | "cooldown" | "not_offered" | "unspecified" | (string & {});

/** One speed tier a model offers besides `standard`, with everything that makes it different from another provider's tier of the same name. */
export interface SpeedOffer {
  /** What the tier buys. */
  effect: SpeedEffect;
  /** What the provider calls it, for display: "Fast mode", "Flex processing". */
  label: string;
  /** Whether the provider marks the tier as a preview that may change or need access to be granted. */
  preview?: boolean;
  /** The tier's own full rate card. Absent when its price is not known, in which case its usage shows tokens only. */
  prices?: Prices | null;
  /** Whether the tier has a rate limit of its own, apart from the model's standard limit. */
  separate_limit?: boolean;
  /** Whether a prompt-cache prefix written at `standard` can be read at this tier and the other way round. `false` means a change of speed is a cache miss. Absent when the provider does not say. */
  shares_cache?: boolean | null;
  /** The tier's name: what a setting names and what a usage record carries. */
  speed: Speed;
  /** What the provider does when it cannot serve the tier. */
  when_unavailable: SpeedUnavailable;
}

/** What the provider does when it cannot serve a request at the tier that was asked for. */
export type SpeedUnavailable = "error" | "downgrade" | (string & {});

/** A condition on which the holder of a lease must stop and report. The harness checks it. It is data, not an instruction to the model. */
export type StopCondition = {
    kind: "verification_failed";
    /** The number of failed rounds in a row. */
    rounds: number;
  } | {
    kind: "out_of_scope";
  } | {
    kind: "spending";
    /** The threshold, in nano-dollars. */
    usd_nanos: number;
  } | {
    /** The moment. */
    at: Timestamp;
    kind: "time";
  };

/** Why a response from the model ended. Each provider's own value is mapped onto this set, and kept beside it as `raw_stop`. */
export type StopReason = "end" | "tool_calls" | "max_output" | "context_overflow" | "paused" | "refusal" | "content_filter" | "malformed_tool_call" | "error" | "interrupted" | (string & {});

/** An approval the user gave earlier that still applies. */
export interface StoredApproval {
  /** When it was given. */
  created_at: Timestamp;
  /** The approval's id. */
  id: ApprovalId;
  /** The rule it added. */
  rule: Rule;
  /** How long it lasts. */
  scope: ApprovalScope;
}

/** A compaction summary. It starts a context epoch's history. */
export interface SummaryItem {
  /** The last item the summary covers. */
  covers_through?: ItemId | null;
  /** When it was recorded. */
  created_at?: Timestamp | null;
  /** The item's id. */
  id: ItemId;
  /** The summary. */
  text: string;
  /** What the summarising request used and cost. */
  usage?: RequestUsage | null;
}

/** The detail of an approval for access to a path or a host. */
export interface TargetDetail {
  /** The effect. */
  effect: Effect;
  /** The path or the host. */
  target: string;
}

/** More text of the item. */
export interface TextDelta {
  /** The text to append. */
  text: string;
}

/** A piece of text. */
export interface TextPart {
  /** The provider's own block, on assistant messages. */
  raw?: RawJson | null;
  /** The text. */
  text: string;
}

/** Where the wall time of some turns went, in milliseconds. Every moment of a turn is in exactly one part, so the parts add up to `wall_ms`: a tool that runs while the response is still arriving is in the model's time, and a question to the user while a tool runs is in the tools'. */
export interface TimeTotals {
  /** Waiting for the user to answer whether a call may run, with no tool running. */
  approval_ms?: number;
  /** The harness's own work: assembling requests, recording, planning. */
  harness_ms?: number;
  /** Waiting for a model. */
  model?: ModelTime;
  /** Waiting for tools once the response had ended. */
  tools_ms?: number;
  /** How many turns. */
  turns?: number;
  /** Running the project's checks. */
  verification_ms?: number;
  /** Their wall time, from the prompt to `turn.completed`. */
  wall_ms?: number;
}

/** A moment in time, in RFC 3339 form in UTC: `2026-11-12T10:15:02Z`. Fractional seconds are allowed. */
export type Timestamp = string;

/** Token counts by class. The classes do not overlap: the tokens sent are `input + cache_read + cache_write`. */
export interface TokenUsage {
  /** Input tokens read from the cache. */
  cache_read?: number;
  /** Input tokens written to the cache, all lifetime classes together. */
  cache_write?: number;
  /** `cache_write` split by lifetime class, where the provider has classes. */
  cache_write_classes?: Array<CacheWriteTokens>;
  /** Input tokens billed at the normal rate: not read from the cache and not written to it. */
  input?: number;
  /** Output tokens, reasoning included. */
  output?: number;
  /** The part of `output` that was reasoning, when the provider reports it. */
  reasoning?: number | null;
}

/** More of a tool call's arguments: a fragment of JSON text that is not valid JSON until the call is complete. */
export interface ToolArgsDelta {
  /** The call. */
  call_id: CallId;
  /** The fragment to append. */
  json: string;
}

/** The model began a tool call. */
export interface ToolCallDelta {
  /** The call. */
  call_id: CallId;
  /** The tool's name. */
  name: string;
}

/** A request by the model to run a tool. */
export interface ToolCallPart {
  /** The arguments, as the JSON value the model wrote. When the model's text was not valid JSON this is that text as a JSON string, and the call's result is an error that says so. */
  arguments: unknown;
  /** The provider's id for the call. Its result names it. */
  id: CallId;
  /** The tool's name. */
  name: string;
  /** The provider's own block. A protocol with a second id for the call keeps it here. */
  raw?: RawJson | null;
}

/** The detail of an approval for an extension or MCP tool. */
export interface ToolDetail {
  /** The arguments the tool would be called with. */
  arguments: unknown;
}

/** How a tool call ended. Every started call ends with exactly one result. */
export type ToolOutcome = "success" | "error" | "denied" | "interrupted" | (string & {});

/** The result of one tool call. */
export interface ToolResultItem {
  /** Where the full output is, when the content was shortened. */
  artifacts?: Array<ArtifactRef>;
  /** The call this answers. */
  call_id: CallId;
  /** What the model is shown, after the output policy. */
  content: Array<Part>;
  /** When it was recorded. */
  created_at?: Timestamp | null;
  /** Milliseconds the call took. */
  duration_ms?: number | null;
  /** The item's id. */
  id: ItemId;
  /** How the call ended. */
  outcome: ToolOutcome;
  /** A structured payload for front ends, whose shape belongs to the tool: a diff, an exit status, match counts. */
  payload?: unknown;
  /** The arguments a hook put in place of the model's, when one did and the call was put to the user with them (since step 28): what ran, if it ran, and which hook chose it. The model's own are in its call. (Boxed: it is rare, and every item would otherwise be its size.) */
  replaced?: ReplacedArguments | null;
  /** The tool's name. */
  tool: string;
}

/** The result of one tool call, as the model sees it. */
export interface ToolResultPart {
  /** The call this answers. */
  call_id: CallId;
  /** What the model is shown: text and images. */
  content: Array<Part>;
  /** Whether the call failed. A failure is a result, written so the model can correct itself. */
  is_error?: boolean;
}

/** The time one tool took over some turns, in milliseconds. */
export interface ToolTime {
  /** How long its calls waited for the user's answer. */
  approval_ms?: number;
  /** How many calls. */
  calls?: number;
  /** How long its calls waited, once allowed, before they ran: for a slot, for an earlier call they conflict with, or for the response to end. */
  queued_ms?: number;
  /** Its calls' running time, added up. Calls that ran side by side are each counted whole. */
  running_ms?: number;
  /** Its part of the turns' tool time (`TimeTotals::tools_ms`): when calls run side by side each has an equal part of that moment. */
  share_ms?: number;
  /** The tool's name. */
  tool: string;
}

/** `trust/request`: may this project's executable resources be used? */
export interface TrustRequest {
  /** What a yes would write, which the person signs (since the signed approvals step): the project's whole entry as it would then be. A front end that answers yes rebuilds the bytes signed from it, checks that it holds what it shows, asks the person for their password, and answers with the signature. Absent when nothing a yes would write can be kept (no state directory): a yes then holds for the session alone. */
  proposal?: ApprovalProposal | null;
  /** What it asks to be trusted with. */
  resources: Array<TrustResource>;
  /** The session. */
  session_id: SessionId;
  /** What a yes changes, one line each, for the person to read before they sign. */
  summary?: Array<string>;
  /** The project is trusted already, and the question is about what is new in it: its rules changed (each resource marked `changed`), git, now that it is a repository, or (since step 28) an extension new in it or changed since it was approved (`changed`). `never` is not an answer to it; `keep` is (since the follow-ups of step 23), which of an extension is kept by the core for its run alone. */
  trusted?: boolean;
  /** The project's workspace root. */
  workspace: string;
}

/** One thing a project asks to be trusted with. */
export interface TrustResource {
  /** Whether it was approved before and has changed since: a project's rules trusted as other rules, or (since step 28) an extension approved at another hash, whose `detail` says so too. */
  changed?: boolean;
  /** What it would do, in full: the command, the rules. */
  detail: string;
  /** What kind of thing. */
  kind: TrustResourceKind;
  /** Its name. */
  name: string;
}

/** A kind of project resource that can run code or grant permissions. */
export type TrustResourceKind = "rules" | "hook" | "extension" | "mcp_server" | "commands" | "repository" | "settings" | (string & {});

/** The answer to `trust/request`. */
export interface TrustResponse {
  /** With `trusted` false, to a question about a project trusted already (the request's `trusted`): keep trusting it as it was, without what is new, and remember that, so that what is new is not asked about again while it stays as it is (since the follow-ups of step 23). An answer with `keep` and `trusted` or `never`, `keep` to a project not trusted, or `never` to one trusted, is refused (`invalid_answer`). */
  keep?: boolean;
  /** With `trusted` false: remember the refusal, so that the project is not asked about at later starts either (since step 23). It is kept beside the trusted projects, and `cineris permissions trust` lifts it. An answer with both `trusted` and `never` is refused (`invalid_answer`), and the question stays open. */
  never?: boolean;
  /** With `trusted`, to a request with a `proposal`: the person's signature over it, 128 lowercase hexadecimal digits (since the signed approvals step). A yes without one, or with one that is not the person's key's over the proposal, trusts nothing, not even for the session, and the question stays open. */
  signature?: string | null;
  /** Whether the resources listed may be used. */
  trusted?: boolean;
}

/** What the user has said of a project's trust (since step 23). */
export type TrustStanding = "untrusted" | "trusted" | "rules_changed" | "never" | "for_the_run" | (string & {});

/** The turn is over and nothing automatic remains: no retry pending, no compaction running, no tool running. Clients wait on this one event. */
export interface TurnCompleted {
  /** Milliseconds the turn took. */
  duration_ms?: number | null;
  /** The failure, when there was one. */
  error?: ErrorData | null;
  /** How it ended. */
  outcome: TurnOutcome;
  /** Why the last response ended. */
  stop_reason?: StopReason | null;
  /** What the turn used and cost. */
  usage?: UsageTotals;
  /** What the project's checks said. */
  verification?: Verification | null;
}

/** One turn of a session: from a prompt until the session is settled again. */
export type TurnId = string;

/** `turn/interrupt` */
export interface TurnInterruptParams {
  /** Whether to stop running commands at once, without the polite interrupt and the grace that normally come first: what a second interrupt means. Absent or `false` is the ordinary interrupt, which stays safe to send twice. */
  now?: boolean;
  /** The session. */
  session_id: SessionId;
}

/** The result of `turn/interrupt`. */
export interface TurnInterruptResult {
  /** Whether a turn was running. */
  interrupted?: boolean;
  /** Steering prompts that had not been delivered, handed back so the front end can put them in its editor. Queued prompts stay queued. */
  undelivered?: Array<Array<Part>>;
  /** Beside `undelivered`, one for each of its messages, when any came from a command (since step 28): the command it came from, or `null`. A front end puts back `/name args` for one, in place of its expanded text. Absent when none did. */
  undelivered_commands?: Array<PromptCommand | null>;
}

/** How a turn ended. It and a headless run's exit code always agree. */
export type TurnOutcome = "completed" | "failed" | "interrupted" | "limit" | "denied" | (string & {});

/** `turn/start` */
export interface TurnStartParams {
  /** A command to send in place of a prompt written out (since step 28): the core expands it, and the expanded text is the prompt, before any `prompt.submit` hook sees it. `input` then holds text parts only, or none: their text follows the expanded text, after a blank line, in the one prompt the hooks see (headless mode's standard input); an image or any other part is refused (`invalid_prompt`). A server that answers `commands/list` reads it; one that does not would take `input` alone. */
  command?: CommandCall | null;
  /** How to deliver it if a turn is running. Required then, and ignored when the session is idle. */
  delivery?: Delivery | null;
  /** The prompt: text and images. */
  input: Array<Part>;
  /** The session. */
  session_id: SessionId;
}

/** The result of `turn/start`. It means "accepted": the turn ends with the `turn.completed` event, and the result never carries the model's answer. */
export interface TurnStartResult {
  /** What became of the prompt. */
  disposition: Disposition;
  /** The turn the prompt belongs to: the one it started, the running one it steers, or the one it will start. */
  turn_id: TurnId;
}

/** A turn began. */
export interface TurnStarted {
  [key: string]: unknown;
}

/** The result of `usage/get`. */
export interface UsageGetResult {
  /** The session's usage now. */
  usage: SessionUsage;
}

/** A way to group the rows of a usage report. */
export type UsageGroup = "provider" | "model" | "project" | "source" | "speed" | (string & {});

/** The period a usage report covers. */
export type UsageRange = "session" | "today" | "week" | "month" | "all" | (string & {});

/** `usage/refresh`: asks the documented usage endpoint of each provider the user turned the refresh on for (`usage.refresh`), with that provider's own credential. Only a `user` client may call it. Added in step 16. */
export interface UsageRefreshParams {
  /** Only these providers, each of which must be in `usage.refresh`. Empty: every provider in it. */
  providers?: Array<ProviderId>;
}

/** The result of `usage/refresh`: one entry for each provider asked. */
export interface UsageRefreshResult {
  /** What each provider answered, in the order of `usage.refresh`. */
  providers: Array<RefreshOutcome>;
}

/** Usage and cost over a range: the result of `usage/report`. */
export interface UsageReport {
  /** The start of the range. Absent for all time. */
  from?: Timestamp | null;
  /** How the rows are grouped, outermost first. */
  group_by: Array<UsageGroup>;
  /** The latest limits each provider reported. */
  limits?: Array<ProviderLimits>;
  /** What to tell the person of the statistics behind the report, a sentence each: a file set aside because it could not be read, one a newer build wrote. Added in step 24. */
  notes?: Array<string>;
  /** The range covered. */
  range: UsageRange;
  /** Whether usage history is being recorded. When statistics are off, only the session range has figures. */
  recording: boolean;
  /** The rows. */
  rows: Array<UsageRow>;
  /** The sources included: `cineris`, and other tools the user turned on. */
  sources?: Array<string>;
  /** Where the time of the range's turns went, all rows together. Added in step 16. */
  time?: TimeTotals | null;
  /** The end of the range. */
  to?: Timestamp | null;
  /** The time each tool took in the range, the most first. Added in step 16. */
  tools?: Array<ToolTime>;
  /** All rows together. */
  totals: UsageTotals;
  /** The time zone the days were counted in, as the user is shown it. Added in step 16. */
  zone?: string | null;
}

/** `usage/report`. It reads local data only. */
export interface UsageReportParams {
  /** How to group the rows, outermost first. Empty: by provider, then model. */
  group_by?: Array<UsageGroup>;
  /** Only this project. */
  project?: ProjectKey | null;
  /** The period. */
  range: UsageRange;
  /** For the `session` range: the session. */
  session_id?: SessionId | null;
}

/** One row of a usage report. The fields that are set are the ones the report is grouped by. */
export interface UsageRow {
  /** The model. */
  model?: ModelId | null;
  /** The project's key. */
  project?: ProjectKey | null;
  /** The project's name or shortened path, for display. */
  project_label?: string | null;
  /** The provider. */
  provider?: ProviderId | null;
  /** The tool that recorded the usage: `cineris`, or another tool's id. */
  source?: string | null;
  /** The speed the requests ran at. */
  speed?: Speed | null;
  /** Where the time of the turns in the row went: for a report grouped by provider, model or project only, since a turn has no one speed or source. Added in step 16. */
  time?: TimeTotals | null;
  /** What was used. */
  totals: UsageTotals;
}

/** Usage added up over some requests: one row of a breakdown. */
export interface UsageTotals {
  /** The cost, one amount per basis. Empty when no price is known. */
  costs?: Array<Cost>;
  /** How many requests. */
  requests?: number;
  /** Tokens by class. */
  tokens?: TokenUsage;
}

/** The session's usage changed. */
export interface UsageUpdated {
  /** The session's usage now. */
  usage: SessionUsage;
}

/** A message from the user. */
export interface UserMessageItem {
  /** The command it came from, when it was a command's: its parts are the command's text, expanded (since step 28). */
  command?: PromptCommand | null;
  /** When it was recorded. */
  created_at?: Timestamp | null;
  /** How it was delivered, when it was sent during a running turn. */
  delivery?: Delivery | null;
  /** The item's id. */
  id: ItemId;
  /** What the user sent: text and images. */
  parts: Array<Part>;
}

/** What the project's checks said at the end of a turn. */
export type Verification = "passed" | "failed" | "skipped" | "incomplete" | (string & {});

/** Notice: the project's checks, at the end of a turn that changed files (`docs/architecture/15-efficiency.md`, "Automatic verification"; since step 19). */
export interface VerificationNotice {
  /** The checks. */
  checks?: Array<CheckReport>;
  /** The notice in words. */
  message: string;
  /** Which moment it tells of. */
  phase: VerificationPhase;
  /** The round, counted from 1 in its turn. 0 in a notice that offers the checks. */
  round: number;
  /** The most rounds the turn gets. */
  rounds: number;
  /** The turn's verdict so far, when a round ended. */
  verdict?: Verification | null;
}

/** Which moment of verification a [`VerificationNotice`] tells of. */
export type VerificationPhase = "started" | "finished" | "offered" | (string & {});

/** `workspace/files`: the paths of a session's workspace, for a front end's file completion (since step 23). */
export interface WorkspaceFilesParams {
  /** The most paths to return. Absent: the server's own limit. */
  limit?: number | null;
  /** The session. */
  session_id: SessionId;
}

/** The result of `workspace/files`. */
export interface WorkspaceFilesResult {
  /** The files, relative to the workspace's root with `/` between parts, the most recently changed first, as the ignore rules and the session's read rules leave them. */
  paths: Array<string>;
  /** Whether there were more than the limit. */
  truncated?: boolean;
}

/** `workspace/warm`: files a person named with `@` in a prompt not yet sent, to be read ahead into the system's cache. Added in step 24. */
export interface WorkspaceWarmParams {
  /** The files, relative to the workspace's root with `/` between parts. */
  paths: Array<string>;
  /** The session. */
  session_id: SessionId;
}
