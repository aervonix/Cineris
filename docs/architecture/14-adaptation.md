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
| Command | project, command class (build, test, check), exit status, duration | The project profile, verification planning. |
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
| **Recording** | `Recorder`: a bounded queue (4,096 events, boxed, so its slots are small) and a thread of its own. `record` never waits: a full queue drops and counts, and the count is an event (`dropped` counts events and limits updates alike). The thread starts when the first session opens, from the blocking pool, and is flushed at `Core::shutdown`, which waits for it at most two seconds and logs what was left unwritten (since the fixes after the independent review of step 16). A writer whose thread has ended is counted the same way, and the log says once that it stopped. Each request line says which recorder wrote it (`writer`, an id unique to the recorder), for a day's limits. A provider's error code is kept only when it is a plain one (`cortex_protocol::error::code::is_plain`, the refresh's rule), and is `other` otherwise. |
| **Files** | `stats/events-YYYY-MM.jsonl` by the UTC month of the event; `aggregates.json`; `limits.json`. Every one owner-only (`cortex_base::private`), and the two last written atomically. The writer appends to the file the month's name gives now: one that `cortex stats reset` deleted is let go and made again. A last line cut short is ended with a line break before the next line is appended. `limits.json` is updated under its lock (`limits.json.lock`). |
| **Aggregates** | Usage by day, provider, model, speed, project and billing; time by day, project and model; tools by day, project and tool; outcomes by model, variant set and tool (or the turn). Each monthly file has its part and the byte it was read to; the writer brings them up to date every five seconds while it writes, and a report does when it reads. Any process may: two that save out of order leave a file the next one carries on from. A file past `stats.retention` months moves its part to `kept` and is deleted. Days more than 400 back fold into months. A rebuild from the files gives the same values (tested). Sums saturate. **`kept` is the one copy of every pruned month,** so the file is flushed to the disk before it takes its name, and one this build cannot read is never written over (`cortex_stats::Found`): a newer build's is left alone, and nothing is pruned or saved beside it; one that does not parse is moved aside (`aggregates.json.unreadable-<when>`, owner-only) before anything is saved, by a writer or a report with statistics on, and `cortex stats` lists it. Saving and pruning check this themselves. |
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

### Where it lives

| File | Scope | Who writes it |
|---|---|---|
| `<workspace>/.cortex/profile.toml` | Shared with the team through the repository. | The user. The harness writes here only when the user promotes an entry. |
| `<state dir>/projects/<project key>/knowledge.toml` | Private to this user and machine. | The harness, as it learns. The user, through the review panel or an editor. |

Both are plain TOML that a person can read and edit. The shared file needs no trust to be read: like instruction files, it is text for the model and cannot grant anything. Commands in it are used for verification and for the safe list only after the user has accepted them.

### How entries are learned

| Path | What happens | Becomes active |
|---|---|---|
| **Observed** | The command observer sees a build, test or check command succeed, or a test flip without a change. | Automatically after two consistent observations, with a notice. The user can undo it. |
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

- **In the prompt.** At the start of each context epoch, active entries are rendered into a "project profile" section of the system prompt, inside a token budget (default 1,500), commands first. Expired and disabled entries are left out.
- **For verification.** `command` entries of class test and check are what automatic verification runs ([15-efficiency.md](15-efficiency.md)).
- **For permissions.** Accepted `command` entries are the "learned safe list": they run without asking ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md)).
- **For the user.** `/project` opens the review panel: entries by kind, with source, age and expiry, and actions to accept, edit, disable, delete, or promote to the shared file. `cortex project` does the same from the command line. (A settings profile, chosen with `/profile`, is a different thing: see [08-configuration.md](08-configuration.md).)

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
