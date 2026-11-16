# Adaptation: the project, the model, the user

The first pillar of the [vision](../vision.md): the harness learns the project, the models and the user's habits. This page specifies the parts that do the learning.

Four rules hold for all of them.

1. **Evidence, not guesses.** The harness adapts to what it has measured or observed on this machine.
2. **Visible.** Everything learned can be read by the user, with where it came from.
3. **Reversible.** Everything learned can be edited, disabled or deleted, and the harness works without it.
4. **Local.** Nothing learned leaves the machine. There is no telemetry.

## Local statistics

`cortex-stats` records measurable outcomes. Every adaptive feature reads from it.

**What is recorded.** Small events, each a few fields, with no prompt or file content.

| Event | Fields | Used for |
|---|---|---|
| Request | time, provider, model, role, the speed tier it ran at, project key, session, variant ids, tokens by class, cost as priced at that moment for that speed, billing kind (metered, subscription, unknown), time to first token, tokens per second, stop reason, retries | The usage view ([18-usage.md](18-usage.md)), cost and cache views, routing, per-model choices. |
| Context | per request: token estimates by source (system prompt, each instruction file, profile, repository map, tool definitions by origin, history by item kind), and where the cached prefix first differed when the cache missed | The context profiler ([15-efficiency.md](15-efficiency.md)). |
| Tool call | tool, model, variant ids, outcome (ok, invalid arguments, failed, denied), duration | Tool-schema and edit-format success rates. |
| Edit | model, edit format, matching tier used, outcome | Choosing the edit format per model. |
| Turn | model, requests, tool calls, outcome, verification result, whether the user undid it | The quality signal for a set-up. |
| Command | project, command class (build, test, check), exit status, duration | The project profile, verification planning. **Not recorded:** step 16 built no such event, and since step 17 an entry of the profile keeps its own count and usual duration. |
| Run | from the run engine: task, variant, judge results | Replay and evaluation ([16-run-engine.md](16-run-engine.md)). |

**Where.** Append-only JSON lines in the state directory, one file per month, plus a small aggregates file that is rebuilt from them. The aggregates hold success rates per model and variant, and usage buckets by local day, provider, model, project and billing kind; day buckets fold into month buckets after 400 days, so all-time usage outlives the raw files. The comparison with a database is in [decision 0021](../decisions/0021-local-statistics.md).

**Cost, against the speed budgets.**

- Events are queued to a writer thread (since step 16 the statistics' own, not the log's). Recording never blocks the core.
- Nothing is read at start-up. The aggregates file (kilobytes) is loaded the first time an adaptive decision needs it.
- Old monthly files are folded into the aggregates and deleted after a retention period (default 3 months since step 16, the coordinator's decision, which the owner may revisit; 12 before: a year of a heavy user's raw events is about 1.1 GB, and every all-time figure is in the aggregates).

**Control.** `cortex stats` shows what is stored, in plain numbers. `cortex stats reset` deletes it. `stats.enabled = false` turns recording off, and every adaptive feature then falls back to its catalog default.

**As built in step 16** (`cortex-stats`; the core's `stats` and `usage`).

| Part | As built |
|---|---|
| **Events** | `request` (both the turn's and the summaries', with `purpose`, `role`, the variants, tokens, the cost and catalog version fixed then, the billing kind, time to first token, tokens a second, where the request's own time went, the stop and an error's code), `tool` (outcome, running, queued and approval times), `turn` (outcome, verification, the parts of its wall time, each tool's share), `context` (the per-request record) and `dropped`. Each is one JSON line with the local day it fell on (`day`), fixed when it is written. No text of a prompt, a file or an answer. |
| **Recording** | `Recorder`: a bounded queue (4,096 events, boxed, so its slots are small) and a thread of its own. `record` never waits: a full queue drops and counts, and the count is an event (`dropped` counts events and limits updates alike). The thread starts when the first session opens, from the blocking pool, and is flushed at `Core::shutdown`, which waits for it at most two seconds and logs what was left unwritten (since the fixes after the independent review of step 16). A writer whose thread has ended is counted the same way, and the log says once that it stopped. Each request line says which day's purse it was charged to (`charged_to`), for a day's limits, and carries what its attempts given up used and what an interrupted request used: the statistics hold what the ledger was charged. A provider's error code, a stop reason the client did not know and the name of a tool that does not exist are kept only when plain (`cortex_base::redact::is_plain_code`: a plain code in which the secret filter finds no credential, the refresh's rule too), and are `other` otherwise. |
| **Files** | `stats/events-YYYY-MM.jsonl` by the UTC month of the event; `aggregates.json`; `limits.json`. Every one owner-only (`cortex_base::private`), and the two last written atomically. The writer appends to the file the month's name gives now: one that `cortex stats reset` deleted is let go and made again. (On a volume without POSIX deletes, FAT or some network shares, a file deleted while another process holds it stays until that process lets it go: a writer at its next line, a day's reader at its next request. Until then no process can make the file again, and lines are lost there.) A blank line is not a line, and is passed over. A last line cut short is ended with a line break before the next line is appended. `limits.json` is updated under its lock (`limits.json.lock`). |
| **Aggregates** | Usage by day, provider, model, speed, project and billing; time by day, project and model; tools by day, project and tool; outcomes by model, variant set and tool (or the turn). Each monthly file has its part and the byte it was read to; the writer brings them up to date every five seconds while it writes, and a report does when it reads. Any process may: two that save out of order leave a file the next one carries on from. A file past `stats.retention` months moves its part to `kept` and is deleted. Days more than 400 back fold into months. A rebuild from the files gives the same values (tested). Sums saturate. **`kept` is the one copy of every pruned month,** so the file is flushed to the disk before it takes its name, and one this build cannot read is never written over (`cortex_stats::Found`): a newer build's is left alone, and nothing is pruned or saved beside it; one that does not parse is moved aside (`aggregates.json.unreadable-<when>`, owner-only, by a rename that deletes nothing) before anything is saved, by a writer or a report with statistics on, and `cortex stats` lists it. Saving and pruning check this themselves, and only aggregates read from the file prune: ones that started from nothing are saved first. |
| **Nothing at start-up** | Making a recorder reads nothing; the files are read by a report, `cortex stats`, a day's limit and the writer's own bringing up to date, never by starting or opening a session (a count of reads by directory, `cortex_stats::reads`). |
| **Test helper** | `cortex_stats::synthetic` (the `test-hooks` feature, dev-dependencies only): `write` fills a directory with a made-up month of events by a seed and a scale (light, typical, heavy), through the real recorder, and returns a summary to assert against. Deterministic, and never in a shipped build ([testing-and-ci.md](../testing-and-ci.md), "A month of statistics, made up"). |
| **Disabled** | No recorder is made; no file is written, a refresh's included, and nothing is moved aside or pruned. `usage/report` says `recording: false`. A session with a day's limit is refused at open (`day_limit_needs_stats`): there is nothing to count other processes' spending from ([18-usage.md](18-usage.md), "Limits on one day"). |

The size a year of raw files reaches for a heavy user, measured: [steps/16/answered.md](../steps/16/answered.md), point 5.

## Project knowledge: profile and memory

`cortex-knowledge` holds what the harness knows about one repository. The vision asks for two things: a project profile (commands, conventions, flaky tests) and project memory (facts with a source and an expiry). They are one store with typed entries, because they share everything that matters: provenance, review, expiry and how they reach the prompt.

### Entries

| Kind | Content | Typical source |
|---|---|---|
| `command` | A named command with its purpose: build, test, check, format, run. Its working directory, usual duration, and whether it only reads. | Observed when the agent or the user runs it and it succeeds. |
| `convention` | A rule in a sentence or two: style, structure, what not to touch. | Proposed by the model, or written by the user. |
| `flaky` | A test that fails intermittently, with the evidence. | Observed: failed, then passed with no change. |
| `fact` | Anything else worth keeping. | Proposed by the model, or written by the user. |

Every entry carries:

| Field | Meaning |
|---|---|
| `source` | `user`, or `learned` with the session and item it came from and the date. |
| `basis` | `observed` (the harness saw it happen) or `proposed` (a model inferred it). |
| `status` | `proposed`, `active` or `disabled`. |
| `confirmed` | When it was last seen to hold. |
| `expires` | When it stops being used unless confirmed again. User-written entries do not expire. Learned entries get a default by kind (commands 90 days, flaky tests 30, facts and conventions 60). |
| `check` | Optional. A cheap, read-only test that shows the entry still holds. See "Checked memory" below. |
| `accepted` | When the user accepted it. An entry the user wrote in their own store counts as accepted; one that became active by itself has none until the user accepts it. |

### Where it lives

| File | Scope | Who writes it |
|---|---|---|
| `<workspace>/.cortex/profile.toml` | Shared with the team through the repository. | The user. The harness writes here only when the user promotes an entry. |
| `<state dir>/projects/<project key>/knowledge.toml` | Private to this user and machine. | The harness, as it learns. The user, through the review panel or an editor. |

Both are plain TOML that a person can read and edit.

**Every entry of the shared file is a proposal to each user, until that user accepts it** (the owner's decision of 2026-11-16, in step 17, in place of "it is text for the model, like instruction files"). Nothing of it reaches the prompt, verification or the safe list before. The reasons: the model can write the file (after the approval a protected path asks for), so an unreviewed shared file would be a way round the review of `remember`'s proposals; a cloned repository's file is a stranger's text; and the profile sits in the system prompt under the harness's own heading. The user's acceptance is kept in the private store against the entry's content hash, so an entry that changes is a proposal again, and one command accepts them all. The shared file needs no project trust to be read and listed; a command accepted from it still waits for trust before it runs without asking, as a learned one does.

### How entries are learned

| Path | What happens | Becomes active |
|---|---|---|
| **Observed** | The command observer sees a build, test or check command succeed, or a test flip without a change. | Automatically after two consistent observations, with a notice. The user can undo it. Active puts it in the prompt and nowhere else: it runs without asking, and is offered to verification, only once the user accepts it. |
| **Proposed** | The model calls the `remember` tool with a fact or convention and a pointer to the evidence. Or a session-end pass (off by default, and run on the fast model) proposes entries. | Only when the user accepts it in the review panel. |
| **Written** | The user adds or edits an entry. | Immediately. |

The owner confirmed this split on 2026-11-12: observed facts become active with a notice, and anything a model inferred waits for review.

### Checked memory

Adopted by the owner on 2026-11-12 ([../ideas.md](../ideas.md)). An expiry date is a guess about when a fact goes stale. A check replaces the guess with evidence.

**What a check is.** One of a small set of forms, all read-only:

| Form | Example | Holds when |
|---|---|---|
| `exists` | `exists = "scripts/release.sh"` | The path exists. |
| `contains` | `contains = { path = "src/router.rs", pattern = "/v2" }` | A search in that file or folder finds the pattern. |
| `command` | `command = "cargo nextest list"` | The command exits with success. It must be one the command analysis classes as read-only, or one the user approved when accepting the entry. |

**Who writes it.** The model, as an optional argument to `remember`, shown with the proposal and accepted or dropped with it. Or the user. For an observed `command` entry the harness writes the obvious one itself: the command's program still exists.

**When it runs.**

- Before an entry is used after a long gap (default 7 days since `confirmed`).
- When a file the entry was learned from, or a file its check names, has changed since `confirmed` (by content hash).
- Never at start-up. In the background on the blocking pool, inside the sandbox with no write access and no network, with a short time limit, at most a few per session.

**What follows.**

| Result | Effect |
|---|---|
| Holds | `confirmed` is updated and the expiry moves out. |
| Fails | The entry is disabled, leaves the prompt at the next epoch, and the user gets a notice with the evidence and a way to restore it. |
| Flips more than once | The check itself is unreliable. It is dropped, and the entry goes back to plain expiry. |

An entry with no check behaves exactly as before.

**In the first release** the `check` field is part of the file format, is shown in the review panel, and is not run. Running checks is its own build step after the release, because it needs the verification runner and care about flaky checks.

### How entries are used

- **In the prompt.** At the start of each context epoch, active entries are rendered into a "project profile" section of the system prompt, inside a token budget (default 1,500), commands first. Proposed, expired and disabled entries are left out, and so is a shared entry this user has not accepted.
- **For verification.** `command` entries of class test and check are what automatic verification runs ([15-efficiency.md](15-efficiency.md)).
- **For permissions.** Accepted `command` entries are the "learned safe list": they run without asking ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md)).
- **For the user.** `/project` opens the review panel: entries by kind, with source, age and expiry, and actions to accept, edit, disable, delete, or promote to the shared file. `cortex project` does the same from the command line. (A settings profile, chosen with `/profile`, is a different thing: see [08-configuration.md](08-configuration.md).)

### As built in step 17

`cortex-knowledge`, with the design as it was reviewed and the owner's answers in [steps/17/answered.md](../steps/17/answered.md).

| Part | As built |
|---|---|
| **An entry** (`entry`) | Kind, text, status, basis, where it was learned, evidence, the times it was confirmed, expires, was accepted and lapsed, the count of observations, a command (purpose, line, directory, usual duration, whether it only reads) and a check. Three questions decide every use: `in_use` (active and not past its expiry: the prompt), `is_accepted` (the user accepted it, or wrote it in their own store), `runs_unasked` (a command, in use and accepted: the safe list and verification). A text is at most 600 characters; evidence, a command line, a pattern and a path 400; a directory and a check's path are relative to the workspace root and stay below it. |
| **The private store** (`file`, `store`) | `<state dir>/projects/<project key>/knowledge.toml`, `version = 1`: `[[entry]]` tables, a `[[shared]]` table for each decision about a shared entry (its content hash, `active` or `disabled`, when), and `next`, the number the next new entry's id takes. It is read strictly: a key or a value this build does not know, a newer version, or more than 256 KiB, 500 entries or 1,000 decisions refuses the whole file. An entry a person adds by hand may leave its `id` out. It is written again whole on every change, owner-only and atomically, flushed to the disk, so comments in it are not kept; the file says so in its first lines. |
| **Ids** | A private entry is `k_<n>` from the file's counter, which never goes down: the id of a deleted entry is not given again, so a change sent from an old listing names the entry the sender was shown or nothing. A shared entry is `s_` and 32 hexadecimal digits of its content hash (its kind, text, command and check): the same on every machine, and another when the entry changes. |
| **The shared file** | `<workspace>/.cortex/profile.toml`, `version = 1`: `[[entry]]` tables of a kind, a text, and optionally evidence, a command and a check, and nothing else. It is read as a regular file in a `.cortex` that is a directory of the workspace itself: a link at either name is not followed, and the file is then reported and not used. Text that is not TOML, a key outside the entries, a newer version or more than 200 entries refuses the file; **an entry with a key or a value this build does not know is left out alone**, and said, so a team on two builds keeps the entries both understand. An entry that claims a status, a date or an acceptance is left out that way: a repository cannot say that its entry is in use. It is written only by `promote`, which adds one entry to its end and keeps every byte a person wrote. |
| **A change** (`Store::change`) | Take the lock (`knowledge.toml.lock`, the operating system's), read the file, apply, write. No session writes a copy it held, so two that change the store at once both keep their change. Readers take no lock. A change is all or nothing. |
| **An invalid file** | Reported, with the line of a syntax error or of the entry at fault, and never written over, moved or deleted: a change is refused until a person puts it right (`project_store_invalid`). What is used in its place is the state from before: what a session already held, or, for a process that starts on it, **the good copy**, `knowledge.toml.good`, which holds the last store this build read or wrote as valid (a load that reads a valid store brings it up to date, so a person's valid edit is what a later typo falls back to). **While the store is invalid the state in its place feeds the prompt and never the safe list:** the file that cannot be read may hold the edit that turned a command off. A store that is not there is empty, and its good copy goes: removing the file is how a user starts again. An invalid shared file is reported, and what a session had read of it stays in use; a process that starts on it uses none of it. |
| **A user's changes** (`Project::update`) | `accept` makes an entry active and accepted, renews a learned entry's expiry (commands 90 days, flaky tests 30, facts and conventions 60, from now) and, for a shared entry, records the decision for its present hash; `disable` takes it out of use and its acceptance with it; `edit` replaces a private entry's text; `delete` removes a private entry; `promote` moves an entry in use that the user accepted to the shared file, written first and never through a link, and keeps the acceptance; `add` writes an entry of the user's own, active at once and never expiring. A shared entry is edited and removed in its file. |
| **A model's proposal** (`Project::propose`) | Stored as proposed, a fact or a convention, with its evidence, its check and its session. The same kind and text again is already known, whatever became of the first: waiting, accepted or turned down. At most 50 wait for review. |
| **Expiry** (`Project::lapse`) | An entry past its expiry is out of use by the clock at once. `lapse` then marks it: disabled, lapsed, and no longer accepted, so a command that lapsed is accepted again before it runs without asking. The user's `accept` brings it back. An entry a person wrote does not expire. |
| **Section 5** (`render`) | A fixed first line, then `Commands:` (by purpose: build, test, check, format, run), `Conventions:`, `Flaky tests:` and `Facts:`, each entry one list line, in the order the files hold them, the private store's first. Every piece of an entry goes through `cortex_base::visible`. Nothing in it depends on a clock, a count or a duration. The budget is `tokens` at a fixed three bytes a token; entries are taken in order while they fit, and a last line says how many were left out. No entry in use, or a budget that holds none, gives no section. |
| **The project key** | `AbsPath::project_key` of the resolved workspace root, as for sessions, trust and approvals. A moved workspace has another key, and its store stays under the old one; two worktrees of one repository are two stores; a workspace below a repository's top looks for its own `.cortex/profile.toml`. Nothing is shared across keys. |

### Compatibility

`AGENTS.md` and `CLAUDE.md` are instruction files and are read as they are ([07-sessions-and-context.md](07-sessions-and-context.md)). The harness never rewrites them. What it learns goes into its own store.

## Adapting to the model

Models differ in which edit format they use reliably, how they handle strict schemas, and how much guidance they need. The harness treats each of these as a **variant point**.

| Variant point | Variants (initial) |
|---|---|
| Edit format | `replace` (exact search and replace). Later: `patch` (a patch envelope) and `rewrite` (whole file, for small files). |
| Tool schema style | `strict` (provider strict mode) or `loose` (validated by the harness only). |
| Prompt style | `concise` or `guided` (more tool guidance and examples). |
| Reasoning default | The model's levels. |

**How a variant is chosen.**

1. **The catalog default.** Each model's catalog entry names a default for every variant point. This is where a new user starts.
2. **The user's override,** in configuration, always wins.
3. **Measured choice.** Where the statistics hold enough trials of two variants for a model on this machine (default 30 each), the variant with the better success rate is used.

**Rules that keep this safe.**

- A variant is fixed for a context epoch, because the tool list and the prompt are frozen for the epoch ([07-sessions-and-context.md](07-sessions-and-context.md)). Choices change only at epoch boundaries.
- The harness does not experiment on the user by default. Trying a variant that has no data is opt-in (`adapt.explore`), and limited to a share of new epochs.
- The better way to compare variants is offline, with the run engine, on recorded tasks ([16-run-engine.md](16-run-engine.md)).
- The status view shows which variants are in effect and why (default, override, or measured, with the numbers).

**In the first release:** the variant points exist, catalog defaults and overrides work, and every outcome is recorded. Measured choice needs a second edit format and enough data, so it follows after the release.

**As built in step 16.** The points are `edit_format` (`replace`), `tool_schema` (`strict` or `loose`), `prompt_style` (`concise`, or `guided`, which puts `GUIDED_NOTES` in section 3 of the system prompt) and `effort` (the reasoning level: a role's own `effort` wins over it). An override is `[adapt.variants."<model id>"]`, the user's alone. A choice this build does not have, or `strict` tools for a model that cannot be held to them, stops the session from starting (`unknown_variant`). The variants are a role's (`RoleModel::variants`), fixed for the epoch (the `epoch` entry's `variants`), and on every request, tool and turn event; the aggregates keep outcomes by model and variant set. The status view of them is the TUI's (step 24).

## Model roles and routing

Different work deserves different models. Configuration names models by role.

| Role | Used for | Default |
|---|---|---|
| `main` | The agent's own turns. | Required. |
| `fast` | Utility calls: compaction summaries, session titles, output summaries, proposed profile entries. | Falls back to `main` when not set. |
| `judge` | Model-graded comparisons in the run engine. | Falls back to `main`. |

**Budget.** The user sets limits per session and per day in money or tokens ([08-configuration.md](08-configuration.md)). The hint row shows the spend. Near a limit the harness says so; at the limit it stops and asks.

**As built in step 16.** One utility call exists, the compaction summary, and it goes to `fast` (`UtilityCall::Summary`, `Models::for_call`). Without a `fast` model it goes to `main`. When the `fast` model refuses a summary as too long, or fails it, the main model is asked once, and `fast` is not asked again in that epoch. `judge` has no call yet and falls back to `main` the same way. Each role is estimated with a budget of its own ([07-sessions-and-context.md](07-sessions-and-context.md), "Budgeting"). The spending limits are per turn, session and day (`[limits]`); the day's is one account for every session of the day, begun from the statistics.

**In the first release:** roles, with utility calls routed to `fast`. **Later:** delegating whole sub-tasks (search a large tree, read a long log and report) to the fast model, which needs sub-agents, and choosing the route from measured cost and success.

## Adapting to the user

| Feature | Where it is specified |
|---|---|
| Modes (permission and behaviour presets) | [06-permissions-and-sandbox.md](06-permissions-and-sandbox.md) |
| Profiles (named sets of settings chosen with one flag) | [08-configuration.md](08-configuration.md) |
| Hot-reload of configuration, extensions, themes and key bindings | [08-configuration.md](08-configuration.md) |
| Self-written extensions | [12-extensions.md](12-extensions.md) |

## First release and later

| Feature | First usable release | Later | Why later |
|---|---|---|---|
| Local statistics | Recording, the usage view for our own numbers in every range ([18-usage.md](18-usage.md)), reset and disable. | Usage recorded by other tools. | Separate reader work with its own fixtures. |
| Project profile | Observed commands and flaky tests. User-written entries. Review panel. Prompt section. | A session-end pass that proposes entries. | It costs tokens on every session, so it should follow evidence that proposals are good. |
| Project memory | `remember` proposals with source and expiry, accepted by review. The `check` field stored and shown. | Running checks (checked memory). | Adopted on 2026-11-12. It needs the verification runner, and handling of unreliable checks. |
| Per-model variants | Variant points, defaults, overrides, recording. | Measured choice. A second edit format. | Needs data that only exists after people use it, and a second format to choose between. |
| Routing | Roles for utility calls. Budgets. | Delegated sub-tasks. Measured routing. | Needs sub-agents. |
