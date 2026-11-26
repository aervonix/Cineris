# RPC protocol

The RPC protocol lets another program drive Cineris: an editor extension, a GUI, a web UI, or another agent. It is complete by construction, because the built-in TUI uses the same operations and events in-process. Anything the TUI does, an external client can do.

The requirement from the TUI language decision applies here: a TypeScript client must be buildable on this protocol later with no change to the core.

## One contract, two carriers

```
            cineris-protocol types (Op, Event, Item, …)
                 │                          │
   in-process: typed channels        out of process: JSON-RPC 2.0
   (TUI, headless printer)           (cineris-rpc over stdio or a local socket)
```

- `cineris-protocol` defines the types once, in Rust, with serde. It is types and their JSON form: no I/O and no behaviour. In a normal build it depends on serde and serde_json and nothing else, which `cargo xtask rules` checks.
- `cineris-rpc` maps each JSON-RPC method to an `Op` and each `Event` to a notification. It adds no behaviour of its own. The mapping between a method name and its parameter and result types is part of `cineris-protocol` (`Op::from_wire`, `OpResult::from_wire` and their counterparts for the server's requests), so a client written in Rust and the server use the same table.
- A conformance test runs the same scripted scenarios through both carriers and requires identical event sequences.

## Transports

| Transport | Start | Framing | Clients | Authentication |
|---|---|---|---|---|
| Standard I/O | `cineris serve --stdio` | One JSON object per line, UTF-8, `\n` terminated. A trailing `\r` is tolerated on input. | One. The parent process. | None needed: the parent already controls the process. |
| Local socket | `cineris serve --socket [PATH]` | The same. | Several at once. | Unix: socket file with owner-only permissions in the runtime directory, and a peer-credential check that the caller is the same user. Windows: a named pipe whose access-control list admits only the current user, labelled at the server's integrity, and a check of each connection's own token, made on its first data. |
| WebSocket (after the first release) | `cineris serve --listen 127.0.0.1:PORT` | One JSON object per text frame. | Browsers and remote tools. | See "Network listening" below. |

In `--stdio` mode standard output carries only protocol messages. Logs go to standard error and the log file. Any code path that could print to standard output in this mode is a bug, and a test guards it.

Newline-delimited JSON was chosen over length-prefixed framing because every language can produce and consume it with a line reader, and because it is the same framing MCP uses for stdio, so the extension host, the MCP client and this server share one reader and writer. They are in `cineris-base` (`framing`): a decoder that does no I/O, into which bytes are pushed in chunks of any size, and thin wrappers for blocking streams. The details of the framing:

- **A message is written as one line** of compact JSON and a `\n`, in one write, and flushed. A line break inside a string is an escape, so a line never contains one.
- **Empty lines are skipped.**
- **A bad line is an error for that line only.** The reader goes on with the next line. A line that is not JSON is answered with -32700, and one that is JSON but not a message with -32600.
- **A line has a size limit,** 32 MiB by default, so that a peer that never sends a newline cannot use up the memory. A longer line is dropped as it arrives, reported once (-32600), and reading resumes at the next newline.
- **A stream that ends in the middle of a line** is an error (-32700): every message ends with a newline, so those bytes are a message that was cut off.

## Messages

JSON-RPC 2.0. Three kinds of message, in both directions.

| Kind | Has `id` | Example |
|---|---|---|
| Request | yes | client → server `turn/start`; server → client `approval/request` |
| Response | the request's `id` | `{"jsonrpc":"2.0","id":7,"result":{…}}` or `{"jsonrpc":"2.0","id":7,"error":{…}}` |
| Notification | no | server → client `session/event` |

Errors use the JSON-RPC error object.

| JSON-RPC `code` | When |
|---|---|
| -32700, -32600 | The line was not JSON, or not a JSON-RPC message. |
| -32601 | The method does not exist. |
| -32602 | The parameters do not fit the method. The message names the field. |
| -32603 | The server failed in a way that says nothing about the request. |
| -32000 | **Every application error.** A client never branches on this number. |

For an application error, `data` is an error object of our own: `{ "kind", "code", "message" }`, and where they apply `request_id` (the provider's id for the failed request), `retry_at` (when the provider said to try again) and `details` (structured detail whose shape depends on `code`).

- `kind` is from the error table in [02-runtime.md](02-runtime.md): `usage`, `config`, `auth`, `provider`, `transient`, `model_stop`, `permission`, `limit`, `context_overflow`, `tool`, `interrupted`, `internal`. It says what sort of failure this is and what a client should offer.
- `code` is a stable name for the exact failure, in `snake_case`: one the protocol defines (`not_initialized`, `already_initialized`, `unsupported_protocol_version`, `role_not_allowed`, `unknown_session`, `session_not_open`, `turn_running`, `history_changing`, `shutting_down`, `supervisor_present`, `unknown_value`, `sandbox_required`, `lease_not_understood`, `no_checks`, `invalid_cursor`, `unrestricted_not_allowed`, `plan_changed`, `prompt_denied`, `invalid_prompt`, `extensions_not_settled`, `unknown_command`, `command_arguments`, `command_refused`, `command_failed`), a provider's own code, or one a later version adds. (`history_changing` and `shutting_down` were added after the review of step 13a, `no_checks` by step 19, `invalid_cursor` and `unrestricted_not_allowed` by step 27, `plan_changed` by step 25, `prompt_denied`, `invalid_prompt`, `extensions_not_settled` and the four command codes by step 28.)

The same error object appears in `turn.completed` and on an assistant item whose request failed, so a failure looks the same whichever way it reaches a client.

## Conventions

These hold for every message, and the generated schema follows them.

| Matter | Rule |
|---|---|
| Names | Field names and enumeration values are `snake_case`. The one exception is the level `read-only`, spelled as the command line spells it. |
| Tagged unions | Events are told apart by `type`, with dotted names (`item.delta`). Every other union (items, parts, deltas, notices, plan edits, approval details, stop conditions) is told apart by `kind`. Two names, so that an event can carry a `kind` of its own beside its `type`. |
| Optional fields | A field with nothing in it is left out. A receiver reads `null` as absent. |
| Empty results | A method with nothing to return answers `{}`, not `null`, so that fields can be added. |
| Times | RFC 3339 strings in UTC: `2026-11-12T10:15:02Z`. |
| Durations | Whole milliseconds, in fields that end in `_ms`. |
| Money | Whole nano-dollars (1e-9 US dollars) in fields named `usd_nanos`, so that sums are exact. A cost is an amount with its basis (`metered`, `estimate`, `recorded`), and amounts with different bases are never added. Prices are nano-dollars per million tokens. |
| Identifiers | Opaque strings. A client compares and returns them and reads nothing into their text. |
| Provider JSON | Kept as a string that holds the JSON text (`raw`), never as a nested value, so that no parser reorders or re-escapes it ([04-providers.md](04-providers.md)). |
| Paths | A workspace is an absolute path on the server's machine. Paths inside a workspace are relative to its root. |

## Handshake and versioning

The first request on a connection must be `initialize`.

```json
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{
     "protocol_version":1,
     "client":{"name":"example-editor","version":"0.3.0"},
     "role":"user",
     "capabilities":{"approvals":true,"questions":true,"trust":true,"deltas":true}}}
← {"jsonrpc":"2.0","id":1,"result":{
     "protocol_version":1,
     "server":{"name":"cineris","version":"0.1.0"},
     "capabilities":{"sessions":true,"mcp":true,"extensions":true,"sandbox":"enforcing"}}}
```

**Client roles.** A client states its role in `initialize`.

| Role | Who | Receives approval and question requests |
|---|---|---|
| `user` | A person's front end: the TUI, an editor, a GUI. | Yes. A user's answer is final. |
| `supervisor` | Another session's orchestration layer, driving this session as a run or an Employee ([16-run-engine.md](16-run-engine.md), [17-coordinator.md](17-coordinator.md)). | With the users, at once, when it has a grant (since step 27). Only approvals, and an allow only within its grant; anything else it passes up by leaving it to the users. |
| `observer` | Something that only watches. | No. |

A session has at most one supervisor. A user client can attach to a supervised session at any time, see everything, send prompts, and answer pending requests; the supervisor is told when a user intervenes.

**Capabilities.** A capability that is absent is `false`: the server never sends what a client did not ask for.

| Client capability | Meaning |
|---|---|
| `approvals` | It answers `approval/request`. |
| `questions` | It answers `question/ask`. |
| `trust` | It answers `trust/request`. |
| `deltas` | It wants `item.delta` events. |
| `experimental` | A list of the experimental features it opts in to, by name. |

The server answers with what it offers: `sessions`, `mcp`, `extensions`, the state of its `sandbox` (`enforcing` or `off`), and the experimental features it turned on for the connection.

**Versioning rules.**

- `protocol_version` is one integer. It is 1. It changes only for a breaking change. A client sends the newest version it speaks, and `min_protocol_version` if it also speaks older ones. The server picks the newest version both sides speak. If there is none, `initialize` fails with the code `unsupported_protocol_version`, and `details.supported` lists the server's versions.
- Everything else evolves by addition, described by **capabilities**. A client states what it can handle (answering approvals, receiving deltas). A server states what it offers.
- **Receivers ignore unknown fields**, in every object, and unknown notification methods. Senders never rely on the other side understanding an addition unless a capability says it does.
- **Every enumeration is open**, not only the ones expected to grow. A string enumeration (a stop reason, an error kind, a level, a speed) may arrive with a value this version does not know. A tagged union (an event, an item, a part, a notice) may arrive with a `type` or `kind` this version does not know. Clients must have a default branch.
- **An unknown value is kept, not dropped.** The Rust types hold it as `Unknown` and write it back exactly as it arrived, so a client that stores or relays messages loses nothing. The generated TypeScript types say the same thing.
- **A known variant that is malformed is still an error.** A `text` part with no `text` is refused with a message that names the part and the field. It is never mistaken for an unknown variant.
- **Open does not mean accepted.** A server that receives a level, a role or a delivery mode it does not know refuses the request, with the code `unknown_value`. The types keep the value so that the refusal can name it. An answer to an approval request that the server does not know is a denial.
- **Enforced values are the exception: they fail closed** ([decision 0028](../decisions/0028-enforced-values-fail-closed.md)). A lease, a grant, a permission rule, a sandbox policy and its path sets, a budget, the hard limits, a stop condition, an expiry and a grantor refuse any field they do not know and any value they do not know (a level, an effect, a rule action, a network setting, a kind of stop condition, expiry or grantor). The value is refused **whole**, with the code `lease_not_understood`: nothing is ignored, nothing is kept, and nothing is applied in part. A build that dropped a restriction it did not understand would turn a narrower lease into a wider one. Everything that only informs stays tolerant. A level and an effect are open where they inform (a session's state, an approval prompt) and refused when unknown inside a grant or a rule; the generated schema lists only the known values at those places, and forbids additional properties in every enforced type.
- **Every notice has a `message`** in words, so a client can show a kind of notice it has never seen.
- Methods under an `x/` prefix are experimental and may change without a version bump. A client opts in to each by name, in `capabilities.experimental`.
- A deprecated method or field keeps working for at least two minor releases and is listed in the changelog.

**What is a breaking change**, and so needs version 2: removing or renaming a field, a method, an event type or an enumeration value; changing a field's type or meaning; making an optional field required; making the server send, by default, something a version 1 client would mishandle; **and adding a field or a value to an enforced type**, unless a capability tells the sender that the reader knows it, because a reader that does not will refuse the lease. **What is not:** a new method, a new optional field, a new event type, item kind, part kind or notice kind, a new enumeration value, a new capability, a new error code.

## Object model

| Object | Identity | Lifetime |
|---|---|---|
| Session | `session_id` | Stored. Opened by a connection. Several connections may open the same session. |
| Turn | `turn_id` | From `turn/start` until `turn.completed`. |
| Item | `item_id` | Created inside a turn. Immutable once completed. |
| Approval | the JSON-RPC request id | From the request until it is answered or times out. |

## Client → server methods

The first-release set. Each maps to one `Op`.

| Method | Purpose |
|---|---|
| `initialize` | Handshake. |
| `session/new` | Create a session: workspace, settings (models per role, reasoning level, speed, level), title, configuration overrides, and where its commands start when that is below the root (`cwd`, since the follow-ups of step 23; refused with `invalid_cwd` when it is not inside the workspace). |
| `session/open` | Open a stored session by id. Returns its metadata and current state. |
| `session/list` | List stored sessions for a workspace, the one added to last first (by `updated_at`). Paged by a cursor that is the last summary given (since step 27): a page holds what comes after it in that order. |
| `session/read` | Read items of a session, paged, from the log, or one item by id. Also used to resynchronise: an item in progress comes back with the content it has so far, and the result says which items are incomplete. Since step 27 the log is paged by a cursor of its own, and the items in progress come whole in the last page. Since step 25, `redacted` reads for an export: each item masked by the core's own secret filter as the log's writer masks it, the provider's blocks (`raw`) left out, the items in progress left out (named in `incomplete`), a session kept in memory the same; the result's `masked` says how many values are masked, and its `redacted` that the items were masked. A client that asked for `redacted` uses no result without it: a server that does not know the field reads the items unmasked, and its result reads `false`. Additions: no new protocol version. |
| `session/fork`, `session/rewind`, `session/undo` | Branching, as in [07-sessions-and-context.md](07-sessions-and-context.md). `session/undo` (since step 23) steps back before the last turn, its first included, and records the step as an undo. A rewind or an undo that would put back a file edited since may ask a question first (below, "As built in step 23"). Since step 25 `session/rewind` takes `plan`, the `mark` of the plan `session/rewind_plan` gave: the core makes the plan again and refuses with `plan_changed`, changing nothing, when it is not the one shown (when the point it goes back to, the active position it goes back from or the turns it undoes differ, and with `restore_files` also when what it does to any file does; the point and the position since the review of step 25's second pull request: a plan another client edited meanwhile, an entry of the conversation, moved the position and not the turns, and was undone unsaid). An addition: no new protocol version. **`before`** (since the review of step 25's second pull request, settled by the maintainers with the reviewer's agreement): `session/rewind`, `session/rewind_plan` and `session/fork` take `before`, an item, in the place of `to` (or a fork's `at`). The point is then the entry that item's own follows in the session's tree; for a prompt that began a turn, the entry the turn's first follows, which is where `session/undo` goes back to when that turn is the last. What the turn recorded before its prompt (the notices that waited for it, the date and the instruction files' changes, the epoch's map) goes with the turn, and what the log holds between the turn before and the turn is kept (that turn's last checkpoint, written after its last item when its checks changed a file, and a plan the person edited); `to` the item before the prompt took those entries with it. *Since the review of that review's fixes:* the prompt's own entry had been the point, which left the turn's first entries on the branch under a turn recorded as rewound, and the next `session/undo` took that turn for the last and undid those entries alone. A rewind and its plan name their point by exactly one of `to` and `before`, a fork by at most one of `at` and `before` (none still forks all of the session); two, or none for a rewind, are refused `invalid_params`. `before` an item that is not on the active branch is refused for a rewind as `to` one is (`invalid_entry`); a fork takes any branch. **An addition, and what a core from before it does:** `to` became optional, which a client of version 1 always sent; a core from before `before` ignores the field it does not know, so it refuses a rewind or a plan given `before` alone, for want of `to`, and forks all of the session for a fork given `before` alone. A rewind from the interface asks `session/rewind_plan` first, which came in the same step, so it never reaches such a core with `before`; a fork from its `/rewind` picker asks nothing first. **So the fork's result says where it ends** (since the review of that review's fixes): `session/fork` answers a `SessionForkResult`, the new session's `session` as before and the request's `before` given back when the fork ends before that item, as `session/read`'s `redacted` is given back. A client that asked for `before` and finds none in the result has a fork of all of the session, and says so; the interface does, and does not say it forked from before the turn. An addition too: the result's shape is the old one with an optional field, which a client from before passes over. |
| `session/rewind_plan` | Since step 25. What `session/rewind` to the same point (`session_id`, and `to` or `before`) would do now, changing nothing: the turns it undoes, and each file they changed, by name (since the review of step 25's second pull request: in the order first changed before, which came out differently from run to run for calls that run side by side), with what it goes back to (`content`, `removed`, or `not_recorded`), whether it can be put back (`as_left`, `already`, `edited_since`, or `cannot` with `why_not`: `not_recorded`, `too_large`, `missing`, `unreadable`, `through_a_link`), whether a checkpoint of those turns cannot be read (`unreadable`), and the plan's `mark`. A read, beside the lane. Refused as the rewind would be: `invalid_params` for both forms or neither, `role_not_allowed` for an observer, `session_not_stored` for a session with no log, `unknown_item`, `turn_running` while a turn runs, and `history_changing` while another change of the history is under way (a rewind's question still open); the observer and the change under way since the review of step 25's second pull request, which found a plan given that its rewind would refuse. It asks whether a change could begin and begins none (`SessionHandle::may_change_history` in `cineris-agent`): a change begun and given up would still have the loop read its history again. An addition: no new protocol version. |
| `session/close` | Stop using a session on this connection. |
| `workspace/files` | Since step 23. The paths of a session's workspace, for a front end's file completion: relative to its root, the most recently changed first, as the ignore rules, the hidden-file rule and the session's read rules leave them, at most a limit, with whether there were more. It walks beside the session's work and ends with the server. |
| `turn/start` | Send a prompt. If a turn is running, `delivery` must be `"steer"` or `"queue"`. Returns at once with the turn id and a disposition: `started`, `steering` or `queued`. Since step 28 it takes `command` (`{name, args}`) in place of a prompt written out: `input` then holds only text, which follows the command's expanded text, after a blank line, in one prompt (anything else is refused `invalid_prompt`), and the core expands the command before any `prompt.submit` hook sees it. A server that answers `commands/list` reads `command`; one that does not would take the empty `input` alone, so a client sends it only to such a server. |
| `turn/interrupt` | Cancel the running turn. Idempotent. The result hands back steering prompts that had not been delivered. With `now: true` (an agreed addition in step 13a) it is the second interrupt: running commands are killed at once, without the polite interrupt and the grace that normally come first ([02-runtime.md](02-runtime.md), "Cancellation"). The field is optional and absent means the ordinary interrupt, so a client that sends one twice, or retries one, never kills. |
| `session/compact` | Compact now, with optional focus instructions. |
| `session/set` | Change settings for the session: the model and reasoning level of a role, the speed tier (for the session, and per role), the level (permission mode). Anything left out stays as it is. Raising the level needs the `user` role. |
| `plan/get`, `plan/update` | Read the plan, and apply a user's edit to it (reorder, skip, reword, add, remove; since step 24 restore, which puts a skipped item back to be done and is told to the model as a skip is). |
| `usage/get` | Token, cost and cache figures for the session, by provider, model, role and speed. |
| `usage/report` | Usage and cost for a range (session, today, week, month, all time), grouped by provider, model, project, source or speed, with limits left. Reads local data only ([18-usage.md](18-usage.md)). Since step 16 a row grouped by provider, model or project says where its turns' time went (`time`), and the report has the range's time, each tool's (`tools`) and the zone its days were counted in. The `session` range is the open session's live figures, for a client that has it open; any other is refused (`session_not_open`), as `usage/get` is. Since the fixes after step 16's review a range this build does not know is refused (`unknown_value`), and aggregates of a newer build are refused (`stats_newer`) rather than reported without the months they alone hold. |
| `workspace/warm` | Since step 24. Files a person named with `@` in a prompt not yet sent, read ahead into the system's cache so that the model's first read of them is quick: at most 16, the first 64 KiB of each, only inside the workspace and only what the agent could read unasked, through the session's path rules. It answers at once and reads on a thread of its own ([15-efficiency.md](15-efficiency.md), "Prefetch"). |
| `session/artifact` | Since step 24. A session's stored artifact as text, up to 1 MiB, with its whole size and whether the text was cut: a check's whole output, for the viewer (`unknown_artifact` for one the session does not have). |
| `usage/refresh` | Added in step 16. Asks the documented usage endpoint of each provider in the user's `usage.refresh`, with that provider's own credential, and keeps what came back for `usage/report`. Only a `user` client (`role_not_allowed` otherwise). Each provider's outcome is its figures or its error; a failure changes nothing kept ([18-usage.md](18-usage.md), "The opt-in refresh"). |
| `project/list`, `project/update` | Read the project profile and memory, and accept, edit, disable, delete or promote entries. Since step 17: `project/update` takes `add`, entries the user writes (a kind, a text, and optionally a command and a check), each active at once; an entry carries `accepted_at` and, when it was observed, `observations`; and a result carries `problems`, a sentence for each of the project's files that cannot be read as it is. The new fields of results and events are ones an older client passes over. `add` is a request's: a development build from before step 17 does not know it (it ignores `add` beside `changes`, and refuses a request with `add` alone, which has no `changes`). No released server is that old, since the first release holds step 17, so no capability was added for it. **An entry in `add` is read strictly** ([decision 0028](../decisions/0028-enforced-values-fail-closed.md)): it is accepted from the moment it is written and its command may come to run without asking, so a field this build does not know, in the entry or in its command, refuses the request. |
| `project/detect` | Since step 25. The build, test and check commands a session's project names in its own files (`Cargo.toml`, `package.json`'s scripts, a `Makefile`'s `build`, `test`, `check` and `lint` targets, `go.mod`, `pyproject.toml`), for the first run's last step ([08-configuration.md](08-configuration.md), "First run"): each with the name the file gives it, the command line, what a script runs, the file and what it is for. Only lines of plain words the command observer classes are given, and a script only when what it runs is given whole: at most 1,000 characters, none of which acts unseen, or the script is not given. The files are read through the session's path resolver and its read rules on the blocking pool, each only when it is a regular file of at most 512 KiB (a FIFO or a device at its name is passed over, not waited on), by any client that has the session open, as `workspace/files` is; nothing is kept, learned or run. A read, beside the lane. An addition: no new protocol version. |
| `verify/run` | Run the project's checks now: a turn that only verifies (since step 19). It answers `{}` and the turn's id comes with `turn.started`, as for `session/compact`; `turn_running` while a turn runs, and `no_checks`, saying why, when no check could run. |
| `models/list`, `config/get`, `config/reload` | Read the catalog and the effective configuration. Reload configuration. A model's entry includes its reasoning control and the speed tiers it offers, each with its own prices. Since step 24 `config/get` says where each value came from (`origins`: the layer, the file and the line, nothing for a built-in default), and `config/reload` is built: its parameters (`session_id`, `profile`, `changed_only`), what it does and what it answers (`applied`, `pending`, `errors`, `notes`) are in [08-configuration.md](08-configuration.md), "Reloading without a restart", as built in step 24. `profile` takes a `user` client (`role_not_allowed`) and a session (`profile_needs_session`), and is refused for a profile only a project not trusted defines (`untrusted_profile`). `config/get` also answers `profiles`, the profiles that may be chosen for the session (the user's and the managed file's, and a project's only once it is trusted), and `profile`, the one in force. |
| `permissions/list`, `permissions/revoke` | Inspect and remove stored approvals. |
| `commands/list` | Since step 28. The custom commands a session may send, each with its source (`user` or `project`; for an extension's command, whose the extension is, and `extension`), description, arguments and file; and `not_used`, each command found and not used, with why. A read, beside the lane. An addition: no new protocol version. |
| `mcp/status`, `extensions/status` | State of MCP servers and extensions. |
| `shutdown` | Stop the server cleanly. |

`turn/start` returning means "accepted". Completion is the `turn.completed` event. The response never carries the model's answer.

**Settings and state.** A session's settings are a level, a speed, and per model role (`main`, `fast`, `judge`) a model, a reasoning level and optionally a speed of its own. The session's speed is what the `main` role runs at. The utility roles do not inherit it: they run at `standard` unless they are given a speed of their own. `session/new` and `session/set` take the same settings object, in which everything is optional. The session state that `session/open`, `session/set` and `session.updated` carry says, per role, what is set and what is in effect: the reasoning level as set and the model's own level it maps to; the speed as set, the speed the next request will ask for, and the end of a cooldown when one is running ([04-providers.md](04-providers.md), "Speed tiers").

After the first release, task leases add `lease/grant`, `lease/get`, `lease/extend` and `lease/revoke`, which only a `user` client may call, and the usage readers add `usage/sources`. The orchestration layer adds methods under `run/*` ([16-run-engine.md](16-run-engine.md)) and the Coordinator adds methods under `team/*` ([17-coordinator.md](17-coordinator.md)): the team's status, the escalation queue and answers to it, the decision log, and attaching to an Employee. A run and an Employee are ordinary sessions, so the methods and events on this page are how they are driven:

| The Coordinator needs | Provided by |
|---|---|
| Sending prompts and follow-ups to an Employee | `turn/start` with `delivery` |
| Streaming status and knowing when an Employee is idle | `session/event`, `turn.completed`, `session.idle` |
| An Employee's approval requests reaching its Manager | `approval/request` routed to the `supervisor` client |
| The user attaching to an Employee directly | A second client with role `user` on the same session |

## Server → client requests

These pause the turn until answered. A client that did not declare the matching capability is never sent them; the server then applies the non-interactive default.

| Method | Purpose | If unanswered within the timeout |
|---|---|---|
| `approval/request` | Ask whether a tool call may run. Params: the tool, the plan summary, the full detail (a command with its directory, a diff, a path or host, or a tool's arguments), and the choices on offer (`allow_once`, `allow_session`, `allow_project`, `deny`). Result: the choice and an optional note, which with `deny` reaches the model as the reason. | Deny. |
| `question/ask` | A question for the user from the agent or an extension, with optional choices. | The tool that asked receives "no answer". |
| `trust/request` | Ask whether to trust a project's executable resources, listing them. | Not trusted. |

When several clients are attached to a session, a request goes to all that declared the capability. The first answer wins, and the others receive `request.resolved`.

As built in step 13a:

- **`approval/request` gained two optional fields,** as agreed additions: `reasons`, why the call needs an answer, and `adds`, the rules that `allow_session` or `allow_project` would add. A front end shows them, so that "always allow" is never a blank cheque.
  - **Each rule of `adds` is an `AddedRule`,** `{effect, match}`, always an allow (since the review of step 13a, which found it typed as the enforced `Rule`). It only informs, so it is read as everything that informs is: an effect this version does not know is kept as written, and a field it does not know is ignored. A client of this version reads a whole approval request from a newer server, whatever its `adds` names. The rule an answer adds is the server's, never what a client sends back.
- **An approval for `write` or `edit` carries the change itself,** as the `diff` detail: the file and a unified diff of the lines the call would change, worked out as the tool would make the change ([05-tools.md](05-tools.md), "The preview"). A front end shows it without matching `call_id` to the call's arguments, which a client that did not ask for deltas may not have yet. It is bounded as a result's payload is, at 256 KiB: a larger diff is left out, with `diff` empty and `omitted` saying why in a sentence, and so is a change that cannot be worked out before the call runs, and a change to a file the agent may not read without asking (since the review of step 13a; `omitted` is an optional field added then).
- **Headless mode's own records are protocol types** (an agreed addition of step 13b): `HeadlessRecord`, told apart by `type`, with `HeadlessStart` (`session`, the first line of `--output jsonl`) and `HeadlessResult` (`result`, the last line, and the whole of `--output json`), and `PermissionDenial`. No method or event carries them: they are in the schema so that a script reading `jsonl` has every line's type, and their values are in `v1-values.jsonl` ([10-headless.md](10-headless.md), "Output formats").
- **`request.resolved` is an event of the session,** with a `seq` like any other, so every client of the session receives it, the one that answered included. A client drops the prompt whose id it names and ignores an id it does not have.
- **Only a `user` client that was sent the request may answer it.** Any other answer is refused with `role_not_allowed`. A `supervisor` is sent none until task leases exist (step 37). *Amended in step 27:* a `supervisor` that has a grant is sent every approval of its session, and its answer counts within the grant ("As built in step 27", below); one without a grant is still sent none, and its answer refused.
- **An `observer` watches, and that is all.** It may open a session that is running, read it, and close its own view of it. Everything that starts, changes or ends something is refused with `role_not_allowed`: `session/new`, `session/open` of a session that is not running (which starts it), `session/fork`, `session/rewind`, `turn/start`, `turn/interrupt`, `session/set`, `plan/update`, `permissions/revoke` and `shutdown`. So is `session/rewind_plan` (since the review of step 25's second pull request), the plan of a rewind it may not make.
- **A `user` client that opens a session is sent the approval requests that are waiting,** under their ids, and may answer them.
- **`session/set` is applied whole or not at all.** Each part is checked and made ready first; a request refused in any part (`unknown_model`, `unknown_effort`, `reasoning_budget_too_large`, `role_not_allowed`, no key for the new provider) has changed nothing. A reasoning level that is only carried over from the role's earlier model, and that the new model does not have or cannot take within its output limit, is left behind with a notice, where one that is asked for is refused.
- **`session/rewind` and `turn/start` exclude each other,** and so do `session/undo` and `turn/start` (since step 23). A rewind is refused while a turn runs (`turn_running`), and a prompt sent while a rewind is under way is refused with `history_changing`, to be sent again. Both are decided under the session's own lock (`SessionHandle::change_history` in `cineris-agent`), so they exclude each other from any thread (since the review of step 13a: the check had been the core's, beside the lock). The change is held by the rewind's work on the log, not by the operation: an operation given up (its client went away) leaves the work to finish, and no turn starts before it has.
- **`session/new` follows `session/set`'s rule** (since the review of step 13a). Its settings and its configuration overrides rank as the user's own command line, so from a client that is not a `user`: a level above the one the session would start at without them is refused with `role_not_allowed`, as a raise through `session/set` is; and an override is taken only if it is a preference, or a choice that raises what is spent (a model, a speed: what `session/set` lets such a client choose), by the rule the configuration's schema gives the key for a project's file. So anything under `[permissions]`, `[sandbox]` or `[limits]` (in a profile too) is refused whatever its value, and so is what is the user's alone (a provider's address or headers, the shell, the data directory, the credential store), what runs code or widens access, and a wait or a time limit that a project's file may only shorten (the retry waits and stream timeouts, `model.speed_wait`, the shell's time limits: since 2026-11-15, as an override is past the rule that holds a project to the value in effect). A lower level is taken.
- **A session started for a client that has gone does not run on.** A session is put together off the core's thread. A client that disconnects meanwhile (its `session/new`, `session/open` of a stored session, or `session/fork` still under way) gets no session: the core looks for the client again once the session is in its table, ends a session nobody else has open, and answers `not_initialized`. A `shutdown` that begins meanwhile ends it too, and no session starts after a shutdown has begun (`shutting_down`). The order of the two looks is what makes this hold: the core puts the session in its table before it looks for the client, and `disconnect` takes the client out before it looks at the table (since the review of step 13a).

As built in step 23, with the fixes after its review:

- **`session/rewind` and `session/undo` may ask a question.** With `restore_files`, a file edited since the harness left it is put back only if the person says so: the session's `user` clients that answer questions are put one `question/ask` naming the files, with the choices "Put them back" and "Keep them as they are". No client to ask, no answer, a client that goes away, the session's close and the server's end all keep the files. The result comes once the question is settled. A turn cannot start meanwhile (`history_changing`).
- **`trust/request` is put to the client that opens a session** (`session/new`, `session/open`, `session/fork`), when it declared `trust`, and the open waits for it ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md), "Project trust"). The answer is `{trusted, never}`: `trusted` alone is a yes, `never` alone is never for this project, neither is not now. **An answer with both is refused** (`invalid_answer`, an error of kind `usage`), as a value the harness enforces is, and the question stays open. An answer the server cannot keep (its trust store cannot be written) still opens the session, with a notice: a yes trusts the project for that session alone, and a never is taken as not now. **A question about what is new in a trusted project** (since the follow-ups of step 23) has `trusted: true` in the request, and its third answer is `keep` (`{keep: true}`: keep trusting it without what is new) in place of `never`; a `never` to it, a `keep` to a project not trusted, and a `keep` beside `trusted` or `never` are refused the same way. Both fields are additions: a server from before reads `keep` as not now. **What changes for a client from before:** one that does not read `trusted` and answers `never` to a question about a trusted project is refused (`invalid_answer`) where it was taken before, and the question stays open for another answer; a never there would take away the trust given before, git's included, which is what the owner's decision of 2026-11-16 rules out. It is a narrowing of what a value means, not a new value, so the protocol's version stays; the only client that answers trust questions is the interactive application of the same build.
- **A question that holds a session ends with it.** A `session/close` that ends the session (no other client has it open), and `shutdown`, withdraw the session's open trust and rewind questions: the waiting open or rewind gets no answer, an answer sent later is refused (`request_not_open`), and a rewind or undo whose session was closed while it asked changes nothing and is refused (`session_not_open`). The `shutdown` of one client ends the questions put to another.

## Events

All events arrive as one notification method.

```json
{"jsonrpc":"2.0","method":"session/event","params":{
   "session_id":"s_01JCX…","seq":412,"turn_id":"t_19","type":"item.delta",
   "item_id":"i_88","delta":{"kind":"text","text":"The build fails because"}}}
```

`seq` increases by one per event per session, and a client reads the events it is sent in that order. A client is not sent every event: deltas it did not ask for are left out, deltas are merged while it is behind (a merged delta has the `seq` of its last part, and never spans the `seq` a `session/read` of the client gave: [02-runtime.md](02-runtime.md), "Event backpressure"), and deltas are dropped when its queue is full. So a jump in `seq` is not by itself a loss. A loss of deltas is said by `item.resync`, which has the `seq` of the first delta dropped, and no whole event is ever dropped. After a reconnect, the `seq` of the last event a client saw is where `session/read` takes it up.

| Type | Meaning |
|---|---|
| `turn.started` | A turn began. |
| `item.started` | An item began: its kind and, for tool calls, the tool name. |
| `item.delta` | Streaming content for an item: text, reasoning, tool arguments, tool output. Sent only to clients that declared `deltas`. |
| `item.reset` | The partial content of an item was discarded (a retried request). |
| `item.completed` | The final item, in full. Authoritative. |
| `item.resync` | Deltas were dropped for a slow client. Fetch the item with `session/read`. |
| `usage.updated` | Token and cost totals, by provider, model, role and speed; the last request with the speed it ran at; cache hit rate; and rate-limit state, including a speed tier's own limit. Since step 24 also what the last request to the main model held, by category (`context`: system prompt, tool definitions, summary, the person's messages, answers, tool results, notices, the whole estimate, the limit and the window), for `/context`. |
| `plan.updated` | The plan changed, by the agent or by a user. |
| `project.updated` | Project profile or memory entries were proposed or changed. |
| `notice` | Something transient to show. Kinds: `retry`, `compaction`, `limit`, `sandbox`, `instructions`, `speed_fallback` (a request ran at a different speed from the one asked for), `speed_cooldown` (a speed tier's cooldown began or ended), `prompt_returned` (a prompt a hook refused, given back, since step 28), `info`. Each has a `message` in words. |
| `request.resolved` | A server request was answered by another client or timed out. |
| `turn.completed` | The turn is over and nothing automatic remains. Carries the outcome, stop reason, usage and error. |
| `session.idle` | The session has no running turn and no queued prompt. With `turn.completed`, this is what a supervisor waits on. |
| `session.updated` | Session metadata changed: title, models, reasoning level, speed (set and in effect), level, queue length. |
| `config.changed` | Since step 24. A reload of the configuration changed what is in force for the session, or what waits: the keys applied, those pending, the errors, and a sentence for each kind of wait. Every client of the session is sent it, whoever's reload, or the look the core takes before a turn, found the change, so that a client that keeps settings of its own (a terminal's theme and key bindings) takes them up. |

**Item kinds:** `user_message`, `assistant_message` (parts: text, reasoning, tool calls, provider blocks; stop reason; the request's usage), `tool_result` (model-visible content, how the call ended, a structured payload such as a diff or exit status, artifact references), `notice`, `summary`.

**Two things are called a notice.** A `notice` *item* is a message the harness added to the conversation: the model sees it, and the log keeps it. A `notice` *event* tells front ends about something transient and is never sent to the model. A retry is an event. "The user edited the plan" is an item.

**Deltas** have a `kind`: `text`, `reasoning`, `tool_call` (the model began a call), `tool_args` (a fragment of the call's JSON arguments, not yet valid JSON) and `tool_output` (output from a running tool).

## Reconnecting and several clients

- Events are not stored separately. A client that connects late, or notices a gap in `seq`, calls `session/read` to load items from the log, then continues with live events. `session/open` returns the current `seq` and whether a turn is running, so there is no race between reading and subscribing.
- Deltas for an item in progress are not replayed. A late client sees the item when it completes, or reads its partial content with `session/read`.
- Several clients may watch and drive one session. Operations are applied in the order the core receives them. A second `turn/start` without `delivery` while a turn runs is an error.
- **Persistent sessions.** A server started with `--socket` keeps its sessions running when a client disconnects. Another terminal (`cineris attach`), an editor, or later a web or phone client over the network transport can pick the session up. This is not a headline feature, and the protocol needs nothing more to allow it.

## Backpressure

Each connection has a bounded outbound queue.

1. When it grows, deltas for the same item are merged, never across the `seq` the client's last `session/read` gave (since the review of step 23's fixes), so the events after a read hold only what came after it.
2. If it is still full, deltas for that item are dropped and one `item.resync` is queued. Full is past its count of events, or past the bytes of the deltas it holds, merged ones counted whole (since step 27: 8 MiB at the usual count, [02-runtime.md](02-runtime.md)).
3. Completed items, turn events and server requests are never dropped. If they cannot be queued within a time limit, the connection is closed with an error. The session and the turn carry on.

A slow or stuck client therefore never stalls a model stream or grows memory without bound.

## As built in step 27

The server's first part: `cineris serve --stdio` (`cineris-rpc`; `docs/steps/27/answered.md`, "The design, before building").

- **The server adds transport and order, and no behaviour of its own.**
  - Each request is read with `Op::from_wire` and carried out by `Core::handle`; each event is a `session/event` line written straight from the event; each request the core puts to a client is a JSON-RPC request under the core's id; each answer goes to `Core::respond`.
  - Nothing but `initialize` is taken before it (`not_initialized`), and it is taken once (`already_initialized`).
  - The client's `ClientInfo` is made from `initialize`'s role and capabilities, and the core then answers it; a refused `initialize` disconnects that client and leaves the connection open for another.
  - A JSON-RPC error in answer to a server request declines it: the request stays open for the others asked and for its time limit.
- **Operations run in the order every front end's do** (`cineris_rpc::lane`): what changes a session in one lane, in the order sent; reads beside it, unless a write sent before them is under way. The terminal UI's in-process link uses the same lane.
- **A result is written after the events queued when it was ready, and no others** (`cineris_rpc::order`), so a `session/read`'s result never overtakes the events at or below its `seq`. The in-process link uses the same rule.
- **The writer takes the next thing to send only once the line before it has gone,** so a slow client's backlog stays in the core's queue, where deltas merge and drop.
- **The core's queue is full by bytes too:** past 8 KiB of deltas for each place of its capacity (8 MiB at 1,024), merged deltas counted whole, as past its count, so a client that stops reading during one long item holds a bounded part of it and one `item.resync` (`cineris_core::events::DELTA_BYTES_PER_PLACE`, measured: [02-runtime.md](02-runtime.md), "Event backpressure").
- **The rule that closes a connection:** while a line waits to be written, the writer looks at the client's queue at most once a second, and closes the connection when whole events wait past its capacity and the client has taken no line for `rpc.stalled_client` (30 s), or at once past four times its capacity. The session and its turn go on.
- **Inbound is bounded and reading never stops for it.**
  - A request counts from when it is read until its result is written: at most 256, and 64 MiB of their lines, or of their results once a result is larger than its request.
  - A request past either is refused at once (`overloaded`, of kind `limit`), and not run.
  - Refusals, and the answers to `initialize` and `shutdown`, count too, and a client with more of them not yet written than requests may run is closed: it sends and does not read.
  - Near a bound the reader lets the writer catch up before it reads on, so a client that reads is not taken for one that floods; one that sends far more in one burst than the bounds hold is closed all the same.
  - A line of more than 1 MiB is read on the blocking pool.
- **`serve --stdio` ends** when standard input ends, at `shutdown`, at a signal, or when its client is gone, stalled or flooding.
  - In order: the client is disconnected (its questions withdrawn, its sessions ended), its lane's operation waited for at most 1 s, the core shut down, the requests still running waited for at most 2 s, a `shutdown` answered `{}`, and what is queued written for at most 2 s, its last line out before the exit.
  - At `shutdown` the core is shut down first, when the request is read, and the rest follows.
  - The whole end has 5 s, counted from when serving stops (and, at `shutdown`, from before the core's end), since the core's end and the disconnect wait for each session's close, which nothing else bounds; a second signal ends the process at once; the exit code is 0, or 130 or 143 for a signal.
  - The requests read with a `shutdown`, after it, are answered `shutting_down`.
  - Standard input and output are served by threads that are not joined, and standard output carries nothing but messages.
- **The approval time limit** is `rpc.approval_timeout`, 10 minutes by default; `"0s"` is none. The terminal UI sets none.
- **A client asks for `unrestricted` only of a server started at it** (`--permission-mode unrestricted`, or the user's configuration).
  - Otherwise the core refuses a session there, `unrestricted_not_allowed`, of kind `permission` (`CoreOptions::allow_unrestricted`, which `serve` turns off unless its own command line or configuration is at `unrestricted`; on by default, so other front ends choose their own level as before). It judges what the configuration comes to, not the request's text, at one gate every way a session's level is set passes (`Inner::may_hold`): a `session/new` whose assembled level is `unrestricted`, by its settings or by an override however spelled (dotted, nested, quoted or spaced parts, in a profile); a stored session's `session/open`; a `session/fork`, whose fork is written only once its level has passed, so that a refused one leaves no stored session and the configuration is read once; each refused before anything of the session is written. And every change of an open session: a `session/set` raising to it, refused before anything of it is made ready; and a `config/reload` that would bring it, with a profile or after the user's file changed, explicit (of one session or every session of the client) or the look at a turn's start. **The rest of such a reload still applies**, a tightening included (a `deny` rule beside the level): the level is not applied, stays in its `pending`, and its `errors` say why (the server was not started at `unrestricted`: restart it with `--permission-mode unrestricted`). A profile at such a level is not chosen at all: neither its keys nor its taking the place of the session's own choices; only trust taken away in the same reload applies. (A check of the override's text in `cineris-rpc`, which the review of the second pull request found four spellings past, is gone.)
- **`cineris serve --socket [PATH]` serves several clients over a local socket** (`cineris_rpc::socket`, `serve_socket`).
  - Each client is a connection of its own, as `--stdio`'s one is, at most 64 at once; one past that is closed unread. A connection gives its place back when it stops serving.
  - An `accept` that fails for want of a resource (descriptors, memory) is tried again after a wait that doubles from 50 ms to 1 s; any other failure ends the server.
  - A connection that has not said `initialize` within 10 s is closed.
  - A client's `shutdown` ends every connection and the server.
  - Its sessions outlive their clients (`CoreOptions::keep_sessions`): another client opens one again, and an approval in a session with no client waits for a `user` client to open it, up to the approval time limit.
  - A `supervisor` connection is given the grant in `ServeOptions::supervisor_grant`, which the binary leaves empty.
  - On Unix the place is `$XDG_RUNTIME_DIR/cineris/rpc.sock`, or `/tmp/cineris-<uid>/rpc.sock` without one, in a folder the server makes owner-only and refuses when others can reach it; a path given must be in a folder others cannot write in, or root's with the sticky bit.
  - The socket is `0600`, and a lock beside it is held for the server's life: a second server is refused, and a socket left behind is removed only when nothing answers on it.
  - A peer of another user, by its credentials, and one whose line of parents reaches the server's process (one of the server's own commands) are closed unanswered; so is one whose own process cannot be read (it is gone) or is not known (another pid namespace). A parent that cannot be read, another user's that `/proc` hides, ends the line elsewhere.
  - On Windows the place is the named pipe `\\.\pipe\cineris-<the user's SID>` (`cineris_rpc::socket`'s Windows module, [decision 0037](../decisions/0037-the-named-pipe-is-served-from-cineris-rpc-in-one-windows-only-module-that-may-use-unsafe.md)).
  - Its security is given to the call that makes each instance: owned by the user, an access list that allows the user alone, and a mandatory label at the server's integrity with no write up and no read up.
  - The first instance refuses a name another process holds, every instance refuses remote clients, and the next instance is made before a connected one is handed on, so the name never lapses.
  - Each connection is checked on its first data, since Windows lets a pipe's server take the client's token only once it has read what the client wrote (`ERROR_CANNOT_IMPERSONATE` before that, seen on the hosted runners), and the token is that of the client's thread that wrote it.
  - The check, on a thread of its own: the token's user is the server's, its integrity at least the server's, and it is neither restricted nor an AppContainer's; only then is the client handed on, with its first data first.
  - A client that writes nothing holds up no other; past 16 of them, the one that has waited longest is closed.
  - The server says where it serves on standard error.
- **A core that keeps sessions** (`CoreOptions::keep_sessions`, off by default; what `serve --socket` needs for a session to survive its client).
  - A disconnect ends no session: the client leaves the session's events, and the session goes on, its turn with it, until a client that may drive it closes it with nobody else on it, or the server ends.
  - An approval asked in a session with no client at all is put to nobody and waits for a `user` client to open the session, which is then put it (as one whose askers have all gone is), until the approval time limit. A session whose clients are there and answer no approvals gets the default at once, as before.
  - A session put together for a client that left meanwhile is still ended (the 13a bullet above).
  - Headless mode, the terminal UI and `serve --stdio` leave it off: their sessions end with their client.
- **Hooks in the loop, as built in step 28** ([12-extensions.md](12-extensions.md), "Tier 3: hooks"), all additions:
  - **`approval/request` gained two optional fields.** `hooks`: the `tool.before` hooks that put the question, each `{hook, reason}`; a hook that answers `ask` puts a question about a call the rules alone would allow. `replaced`, `{hook, arguments, original, reason}`: a hook put `arguments` in place of the model's `original`, and `summary` and `detail` describe the call as it would run, with the hook's. Such a question is put whatever the rules say, offers only `allow_once` and `deny`, keeps no rule, is answered `deny` where nobody can be asked, and **is put to `user` clients only, a supervisor that opens the session while it is open included: a supervisor's grant never answers a call a hook replaced.** `reasons` names the hooks too, a line for each that asked and one for a replacement, for a client that reads neither field; a client that reads them draws those lines once.
  - **A tool result names a replacement:** `tool_result` gained `replaced`, `{hook, arguments, reason}`, once a question about it was answered; the result's content begins with a line that tells the model which hook put what in place of its arguments.
  - **A prompt a hook refuses is given back** to the client that sent it: the notice `prompt_returned` carries the prompt's `parts`, its `delivery` (`steer` for a steering message, at a boundary or as the prompt of a turn of its own; `queue` for a queued prompt; absent for a prompt sent to an idle session), and `hooks`, each `{hook, reason}`. Every other client is sent the notice under the same `seq` without its `parts`. A prompt that would have started a turn ends it `denied` with the error `prompt_denied` (kind `permission`), and nothing of it is recorded; a steering message is dropped and the running turn goes on.
  - **A hook's context** is a `notice` item of the new source `hook`, its first line naming the hook.
  - **`turn/start` takes only text and image parts** (`invalid_prompt` otherwise), since a hook sees a prompt's text.
  - **A session whose extensions could not be found and registered starts no turn:** `turn/start`, and `session/compact`, which begins as a turn does, are refused with `extensions_not_settled` (kind `internal`). The message says why the session's finding failed and to close the session and open it again. Without that refusal, a turn would start without a gate one of the extensions may hold. Every later start in that session is refused the same way, since the session does not try the finding again. An addition: no new protocol version.
- **Custom commands, as built in step 28** ([12-extensions.md](12-extensions.md), "As built in step 28: custom commands"), all additions:
  - **`commands/list`**, a read beside the lane, answers once the session's extensions are found and registered.
  - **`turn/start`'s `command`** is refused `unknown_command` for a command the session does not have, `command_refused` for one found and not used or one that names a settings profile, `command_arguments` when what was typed does not fit it, and `command_failed` when an extension's script fails or gives no text, or of kind `interrupted` when an interrupt stopped it, which a front end does not show as a failure; each message says why, and nothing of the prompt is recorded.
  - **`user_message` gained `command`,** `{name, source, extension, args}`: the item's `parts` are the expanded text.
  - **`prompt_returned` gained `command`** (`PromptCommand`), sent to the prompt's sender alone as `parts` are, and **`turn/interrupt`'s result gained `undelivered_commands`**, one entry per `undelivered` message, the command it came from or `null`, absent when none came from a command: a front end puts back `/name args` in place of the expanded text.
  - **An interrupt does not wait behind a command** (the lane's one exception): `cineris_rpc::lane::Dispatcher::start_with` is given an `early` hook, which the terminal UI's lane and each RPC connection set to `Core::early`; it runs the moment an operation is sent, before it is routed. For a `turn/start` that names a command it puts the command's run on the session then, queued or not; for `turn/interrupt` from a client that may drive the session it stops every command sent to the session and not yet ended, a queued one before its script runs (and with `now` kills their programs). The interrupt itself then runs in its order, as before: it cannot overtake a prompt that has not started.
  - **A question or an approval of an extension's command** names, in `turn_id`, its run (`cmd_<n>`), never a turn's id.
- **Supervisor routing, inert without a grant** (`cineris-core`; `Core::connect_with_grant`).
  - A `supervisor` has a grant only when its server gives it one at connect: nothing on the wire sets it, `serve` gives none, and the steps of task and team leases decide where a real one comes from. Without one it is asked nothing and answers nothing, as before.
  - A session has at most one supervisor: a second one's `session/open` of it is refused `supervisor_present`. (A `session/new` or a `session/fork` puts its client on a session of its own, which has no other.)
  - Every approval goes at once to the session's supervisor, when it declared `approvals` and has a grant, and to the session's `user` clients that answer approvals. Questions and trust stay the users'.
  - The grant is enforced on the supervisor's answer. A denial is always taken. An allow is taken only where the grant covers the call, and then as `allow_once`, whatever scope it named (a wider one is logged): it allows that call, settles the request, and records nothing. What covers the call is an engine made from the session's own engine template, with the session's standing lease narrowed by the grant for its scope (roots, protected paths, network, every `deny` and `ask` rule, never wider than the session's), and the grant's level, at most `autonomous`, and its allow rules for who decides; it is judged by the session as it is when the answer comes ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md), "When a session is supervised").
  - A grant never lifts a question about a path (beyond the roots, a file that looks like a secret, a protected path): its `read` and `write` allow rules are not used. A command is within it only where the narrowed scope is the session's own and the session's enforcing sandbox confines it: there its level (`autonomous`) or an `exec` allow rule of it covers a command; elsewhere neither does (never on Windows until its sandbox, nor at `unrestricted`, nor with a narrower scope), since the engine leaves a command's writes beyond the roots to the sandbox. A grant whose expiry or `time` to stop has passed covers nothing, and the refusal says so.
  - The supervisor is told what of its grant does not apply, and why, when it opens or joins the session: an `info` notice of the session (so every client of it reads it), naming the rules that do not apply and the reason (paths never; commands for want of an enforcing sandbox, or for a narrower scope; a `guided` level, not there yet, read as `supervised`), and what of its allow rules does apply (tools and hosts, which are not commands); or that the grant has expired, cannot be narrowed to the session, or makes no engine (a rule this build cannot read), with why: such a grant covers nothing. What judges an answer and what the notice says come from one function. A narrower scope is told by what the path sets hold, each within the other, not by the text of their roots. No protocol value was added.
  - An allow beyond the grant is not taken: it is refused `role_not_allowed`, which a JSON-RPC client never sees, so the log says it, and the request stays open for the users. That is how a supervisor passes a request up; it learns from `request.resolved` that its answer was taken, or that a user's was.
  - A user may answer at any moment until the request is settled, and the first answer taken settles it: a supervisor that answers within its grant before a person has read the request settles it. This is how 17-coordinator's "a user's answer always wins" is read.
- **Closing a view** (`cineris-core`; [settled in step 27](../steps/27/answered.md)).
  - An observer's `session/close` or disconnect never ends a session: only a client that may drive it ends one, when no other client has it open. Outside `--socket` an observer is never the last client of a session, so nothing else changes.
  - A client that closes its view of a session leaves the session's open questions, as one that disconnects does: its answer to them is refused (`role_not_allowed`), a question that ends with its askers and has none left ends as no answer, and opening the session again puts what is still open to it again.
- **`session/list` pages by the last summary given** (`cineris-core`), not by position. The cursor is that summary's `updated_at` and id, as opaque text, and the next page holds what comes after it in the list's order (the one used longer ago, the id breaking a tie). A session used between two pages moves ahead of the cursor and heads the next listing; every other session is listed once, none twice and none skipped. A cursor the server did not give is refused (`invalid_cursor`). No protocol type changed.
- **`session/read` pages the log by a cursor of its own** (`cineris-core`'s `paging`): the session's id, the number of the log's items given so far and the id of the last of them, as opaque text. A cursor of another session, one past the end, and one whose last item is not where it was (a rewind or an undo moved the session to another branch) are refused `invalid_cursor`, and the client reads again from the start. The items in progress come whole in the last page, past its limit, and never in another, so nothing moves between pages: an item that completes between two pages joins the log after the cursor, where the next page finds it. One item read by its id is given from the log, or in progress, with no cursor. A cursor the server did not give is refused (`invalid_cursor`). No protocol type changed.
- **The conformance test** runs one list of scenarios (`cineris_testkit::conformance`) in process, over the connection's in-memory ends, and through the real binary's `serve --stdio`, and compares what the client was sent.
- **A client written in TypeScript runs a turn knowing the protocol only from the generated declarations** (`crates/cineris-cli/tests/system/ts_client/turn.mts`).
  - It imports its types from `cineris-protocol.d.ts` and nothing but Node's own modules, and Node 22.18 or later runs it as it is, stripping the types.
  - It says `initialize` as a `user` client that answers approvals, opens a session, sends a prompt, answers the approval the turn asks, and reads `session/event` until `turn.completed` and then `session.idle`.
  - `tsc` checks it against the declarations first, so a protocol change it no longer fits fails, in the `ts-client` job that blocks nothing and in the nightly ([../testing-and-ci.md](../testing-and-ci.md), "The TypeScript client").

- **`cineris attach [--socket PATH] [--continue | --resume ID] [PROMPT]` runs the interactive application on a server's session,** with no core of its own (`cineris-cli`'s `attach`).
  - It reaches the server only once it is shown to be the user's (`cineris_rpc::socket::connect`): on Unix a socket of the user's with no write for others, in a folder that is the user's or root's with the sticky bit, and a process of the user's at the other end (`SO_PEERCRED`); on Windows a pipe the user owns, labelled medium or above (no label is medium), asked of the open handle.
  - It says `initialize` as a `user` client with every capability, and the application is the in-process one, through a second `Link`: an operation is a request matched back to its ticket by its id, and a result is taken in the same order as in process, after the events that came before it.
  - Events and the server's requests go into an `EventQueue` of the link's own, so that deltas merge and give way to `item.resync` for an interface that is behind; a `session/read`'s result fences it at its `seq`, and an open or a close lifts the fence, as the core does in process.
  - Its reading and writing run on a thread of its own with a current-thread runtime; when the connection ends, every operation still asked is answered with an error and the application is told once.
  - Leaving is a disconnect: the server keeps the session, for `attach --continue` or another client.
- **Prompts at a turn's edges** are the application's, so both front ends have them: the prompts held while a session opens go one at a time, the first with `steer` when the open says a turn is running, and a prompt refused `turn_running` is sent again once with `queue` ([09-tui.md](09-tui.md)).
- **A request's round trip through the server** (`models/list`, a release build on the reference machine, 2,000 each): 177 µs at the median and 283 µs at the 99th percentile over the transport's in-memory ends, 186 µs and 310 µs over the local socket; nearly all of it is the core's own work (`cineris-rpc`'s `perf_a_requests_round_trip_through_the_server`).
- **The server's decoder is fuzzed in the nightly workflow for 120 seconds a night** (the `rpc_decoder` target of `cargo xtask fuzz`; [testing-and-ci.md](../testing-and-ci.md), "Not blocking").
  - Any bytes, as a client's stream, are read as the server reads them: into lines, each line's kind, a request's operation, and an answer to each kind of request the server puts.
  - Nothing may panic, what cannot be read gets an error the server can write and a client can read, and an operation or an answer read is the same once written again and read.
  - A float may come back a unit or two in the last place off, since serde_json, as the harness builds it, reads a float only that closely.

## Network listening

Off by default, and not in the first release. When added:

- Binds to the loopback address unless `--allow-remote` is given.
- Requires a bearer token on every connection. A random token is generated per launch, written to a file with owner-only permissions, and printed once to standard error. There is no unauthenticated mode.
- Checks the `Origin` header against an allow-list, so a web page in the user's browser cannot connect to a local server.
- Offers no TLS of its own. Remote use goes through an SSH tunnel or a TLS-terminating proxy.
- `--allow-remote` without a token is refused.

The threats are in [../security.md](../security.md).

## Schema and generated types

- The Rust types in `cineris-protocol` are the source of truth.
- `cargo xtask schema` generates two files, both checked in ([../protocol/README.md](../protocol/README.md)):
  - `docs/protocol/schema/cineris-protocol.schema.json`: one JSON Schema document with every type under `$defs`, a root that accepts any JSON-RPC message, and a table (`x-cineris-methods`) that says, for each method and notification, which types its parameters and result are.
  - `docs/protocol/ts/cineris-protocol.d.ts`: TypeScript declarations for every type, with `ClientMethods`, `ServerMethods` and `ServerNotifications` maps from method name to types. It is translated from the schema, so the two cannot disagree.
- **A stale file fails the build twice over:** `cargo xtask check` has a `generated` step, and a test regenerates the files and compares, on both OSes. A protocol change therefore always shows up in review as a schema diff. A pull request that touches these files is never treated as documentation-only.
- **The golden transcript** (`crates/cineris-protocol/tests/golden/v1-session.jsonl`) is a session written by hand in protocol version 1: every method, every event type, and every kind of item, part, delta, notice and plan edit. Tests read each line into its typed form and write it back, and require the same JSON. A second test adds a field no version knows to every object and requires the same result. A third validates every line against the generated schema, which is what shows that the schema describes the wire form the types really have. The transcript is the record of version 1: a change that breaks it is a mistake or a new version, and the file is not edited to make a test pass. Later versions add transcripts beside it.
- **Every type is in a golden file.** The types that travel in no method and no event (a lease, a conversation message, the less common approval details) have values in `v1-values.jsonl` beside the transcript, with the same round-trip test. A test then checks that every type in the schema has a value in one of the files, so a type added to the protocol without one fails by name. Another checks that no two types share a schema name, which the generator would otherwise settle by calling the second one `Message2`. In the schema the JSON-RPC envelopes are `JsonRpcMessage`, `JsonRpcRequest`, `JsonRpcResponse`, `JsonRpcNotification` and `JsonRpcError`, because the conversation model has a `Message` of its own.
- A TypeScript client needs only the generated declarations and a line reader. No Rust code is involved, which is what makes the "TypeScript front end later" option real.

## What is deliberately left out

- **No HTTP REST surface.** One bidirectional message protocol covers requests, events and server requests. A REST façade can be layered on later if a client needs it.
- **No remote file or terminal access methods.** A client that needs files reads them itself, or asks the agent.
- **No UI-control methods** (driving the TUI from outside). The TUI is one client among others.
- **Agent Client Protocol.** Editors that speak that protocol can be served by a thin adapter that translates it to this one. It is a separate step after the first release, because that protocol's own API has been changing quickly.
