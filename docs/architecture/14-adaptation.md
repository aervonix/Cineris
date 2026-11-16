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
| **An entry** (`entry`) | Kind, text, status, basis, where it was learned, evidence, the times it was confirmed, expires, was accepted and lapsed, the count of observations, a command (purpose, line, directory, usual duration, whether it only reads) and a check. Three questions decide every use: `in_use` (active and not past its expiry: the prompt), `is_accepted` (the user accepted it, or wrote it in their own store), `runs_unasked` (a command, in use and accepted: the safe list and verification). A text is at most 600 characters; evidence, a command line, a pattern and a path 400; a command line, a check's command and its pattern are one line with no control character; a directory and a check's path are relative to the workspace root and stay below it, and one that names a root, a drive or the home directory is refused as written. These are checks of the text: whatever runs a command or a check resolves its path against the workspace then (step 19, step 43). |
| **The private store** (`file`, `store`) | `<state dir>/projects/<project key>/knowledge.toml`, `version = 1`: `[[entry]]` tables, a `[[shared]]` table for each decision about a shared entry (its content hash, `active` or `disabled`, when), and `next`, the number the next new entry's id takes. It is read strictly: a key or a value this build does not know, a newer version, or more than 256 KiB, 500 entries or 1,000 decisions refuses the whole file. An entry a person adds by hand may leave its `id` out. It is written again whole on every change, owner-only and atomically, flushed to the disk, so comments in it are not kept; the file says so in its first lines. **Nothing is written that could not be read back** (since the review of the step's first pull request): a change that would take the file past 256 KiB is refused (`project_store_full`; the size binds before the count when entries are long, 243 of the longest fill it), and the text is read once more before it takes the file's name. **Room is kept for the changes that only narrow or decide:** a new entry (a user's, a model's proposal, an observation) is refused once the store is within 32 KiB of the cap, so a full store can still have an entry turned off, accepted, marked lapsed or deleted. |
| **Ids** | A private entry is `k_<n>` from the file's counter, which never goes down while the store lasts: the id of a deleted entry is not given again, so a change sent from an old listing names the entry the sender was shown or nothing. (A store that is removed starts again at `k_1`: removing it is starting again.) A number past 2^53, in an id or in the counter once every id is given, refuses the file, and so does a private id that begins `s_`. A shared entry is `s_` and 32 hexadecimal digits of its content hash: its kind, its text with its blanks made one (so a checkout that changed the line endings has the same entry), its command and its check. It is the same on every machine, and another when the entry changes. The evidence is outside the hash: it is shown in a listing, and never sent to the model or run. |
| **The shared file** | `<workspace>/.cortex/profile.toml`, `version = 1`: `[[entry]]` tables of a kind, a text, and optionally evidence, a command and a check, and nothing else. It is read as a regular file in a `.cortex` that is a directory of the workspace itself: a link at either name is not followed, and the file is then reported and not used. Text that is not TOML, a key outside the entries, a newer version or more than 200 entries refuses the file; **an entry with a key or a value this build does not know is left out alone**, and said, so a team on two builds keeps the entries both understand. An entry that claims a status, a date or an acceptance is left out that way: a repository cannot say that its entry is in use. It is written only by `promote`, which adds one entry to its end and keeps every byte a person wrote, never through a link at either name (checked again at the write, whatever the read found), and never past 256 KiB (`shared_profile_full`). What is said of an entry that was left out, and of a file that was refused, is escaped: it may quote a key the file's author chose. |
| **A change** (`Store::change`) | Take the lock (`knowledge.toml.lock`, the operating system's), read the file, apply, write. No session writes a copy it held, so two that change the store at once both keep their change. A change is all or nothing (a `promote` asks whether the store can take the change before it writes the repository's file), and what it gives back holds the file's stamp from the moment the store was written, under the lock: another process's change right after it, and a person's edit saved while the good copy was being written, show at the next look. Readers take no lock, and a load never waits for one: bringing the good copy up to date is tried, and left to the next load when a writer holds the lock. |
| **An invalid file** | Reported, with the line of a syntax error or of the entry at fault, and never written over, moved or deleted: a change is refused until a person puts it right (`project_store_invalid`). What is used in its place is the state from before: what a session already held, or, for a process that starts on it, **the good copy**, `knowledge.toml.good`, which holds the last store this build read or wrote as valid (a load that reads a valid store brings it up to date, so a person's valid edit is what a later typo falls back to). **While the store is invalid the state in its place feeds the prompt and never the safe list:** the file that cannot be read may hold the edit that turned a command off. A store that is not there is empty, and its good copy goes: removing the file is how a user starts again. An invalid shared file is reported, and what a session had read of it stays in use; a process that starts on it uses none of it. |
| **A user's changes** (`Project::update`) | `accept` makes an entry active and accepted, renews a learned entry's expiry (commands 90 days, flaky tests 30, facts and conventions 60, from now) and, for a shared entry, records the decision for its present hash; `disable` takes it out of use and its acceptance with it; `edit` replaces a private entry's text; `delete` removes a private entry; `promote` moves an entry in use that the user accepted to the shared file, written first and never through a link, and keeps the acceptance (one that stopped between its two writes left the entry in both files, which is read as one, and the next `promote` of it finishes the move and adds nothing); `add` writes an entry of the user's own, active at once and never expiring. A shared entry is edited and removed in its file. |
| **A model's proposal** (`Project::propose`) | Stored as proposed, a fact or a convention, with its evidence, its check and its session. The same kind and text again is already known, whatever became of the first: waiting, accepted or turned down. At most 50 wait for review. |
| **Expiry** (`Project::lapse`) | An entry past its expiry is out of use by the clock at once. `lapse` then marks it: disabled, lapsed, and no longer accepted, so a command that lapsed is accepted again before it runs without asking. The user's `accept` brings it back. An entry a person wrote does not expire. |
| **Section 5** (`render`) | A fixed first line, then `Commands:` (by purpose: build, test, check, format, run), `Conventions:`, `Flaky tests:` and `Facts:`, each entry one list line, in the order the files hold them, the private store's first. Every piece of an entry goes through `cortex_base::visible`. Nothing in it depends on a clock, a count or a duration. The budget is `tokens` at a fixed three bytes a token; entries are taken in order while they fit, and a last line says how many were left out. No entry in use, or a budget that holds none, gives no section. |
| **The project key** | `AbsPath::project_key` of the resolved workspace root, as for sessions, trust and approvals. A moved workspace has another key, and its store stays under the old one; two worktrees of one repository are two stores; a workspace below a repository's top looks for its own `.cortex/profile.toml`. Nothing is shared across keys. |
| **The command observer** (`observe`) | The core hands every finished `shell` call to the session's `Watch`. A line is observed only when it is **plain words** (letters, digits and `_ . / : = @ % + , -`, at most 160 characters: no quote, no operator, no pattern character, no line break), because an observed line reaches the system prompt with no one reviewing it and must not be able to carry a sentence, and because a line with an operator has an exit status that is not one command's; only in **a directory of plain characters** inside the workspace (the line's characters without the colon: the directory is shown beside the line, and a folder can be named anything); and only when **the table classes it** (`classify`) as a build, a test run or a check: `cargo`, `make`, `just`, `ninja`, `task`, `npm`, `pnpm`, `yarn`, `bun`, `go`, `pytest` and the other test runners, `python -m`, the linters and type checkers, `tsc`, `dotnet`, `swift`, `zig`, `mix`, `deno`, `gradle`, `mvn`, `cmake`, each by its sub-command where it has one. **The table errs towards not observing,** and refuses real lines for it (`./gradlew build -x test`, `tox -e py311`, `npm run test-unit`, `make CC=clang test`): a line that is not observed is one the user writes an entry for, and a line wrongly observed is listed for the model as the project's own. A sub-command is the first word after the program, a few options that take no value apart (`cargo --locked test`, `pnpm -r test`), so that an option's argument is never read as one (`pnpm --filter test publish`); `python -m` only as the first word. Where a program takes several targets or goals every one must class the same (`make test deploy` is not a test run); for `make`, `just`, `ninja` and `task` only `-j`, `-k`, `-s`, `-B` and `-C` with its directory are passed over, and an assignment, another makefile or any other option refuses the line; for `gradle` and `mvn` an option's argument is read as a goal too, which only refuses more. A script or a target is classed by its whole name, or by the word before a `:` and one of a few variants after it (`test:unit`, `test:e2e`; not `test-and-deploy`, not `test:publish`). `zig build` is classed by its steps. A line with an option that names something to run (`-exec=…`, `--script-shell`, `--init-script`, `--eval`), `tox` and `nox` with anything after them, `dotnet` with an MSBuild target, and `cmake --build` with a target or with `--`, are not classed. A program is known by its file name, whatever directory is written before it and with `.exe`, `.cmd` or `.bat` taken off. No line is observed as a formatter or a `run`: a user writes those. A success is exit status 0 of a command that ran to its end; one that timed out, was interrupted or left something running is neither a success nor a failure. **A flip** is a test command that failed and then succeeded in one session with no change the harness saw between the two runs (no file tool wrote a file; no command, the two runs included and one run outside the workspace too, reported a changed file or could not say what it changed; no rewind put files back) and under the same confinement. An edit made outside the harness between the runs is not seen, which is why one flip only proposes. A session kept in memory, whose file tools the harness does not see writing, tells no flip. |
| **Learning** (`Project::learn`) | One thing the watch saw is counted in the store, under the lock like every change. Nothing for this command: a `proposed` entry, seen once (at most 40 of those are kept, the one seen longest ago goes when another comes, and one not seen again in 7 days is dropped). Seen again: `active`, with its expiry, **and not accepted**: it is in the prompt, it still asks before it runs, and it is not offered to verification, until the user accepts it. Active already: confirmed at most once a day (one write a day a command, not one a run), which moves its expiry out and its usual duration towards the new one. Lapsed, or past its expiry and not yet marked: active again and unaccepted. Turned off by the user: nothing, however often it is seen. Known already as an entry someone wrote: nothing. A command entry's `check` is `exists` of its program where that is a relative path in the workspace; a program found on the search path, a rooted one or one on a drive has none. `Snapshot::wants` answers from memory whether an observation could change anything, so the common case (a command known and confirmed today) takes no lock and reads no file. |
| **In a session** (`cortex-core`'s `knowledge`) | One `Knowledge` for a session, made for the session's own workspace root and project key; the sessions of one project in a process share the last snapshot read. It fills four seams: the loop's `ProfileSource` (section 5), `remember`'s `Proposals`, `shell`'s `CommandProfile` (a command in use as a build, a test run or a check has the lines that report a problem kept first), and the engine's learned rules. **Nothing is read until the session's first turn:** making it touches no file (`the_knowledge_is_not_touched_until_the_session_is_open`, with the test kit's `Untouched`); the first read is where the loop asks for section 5, on the blocking pool. A read looks at the two files' stamps and reads them again only when one changed, tells the user of a file that cannot be read (once), and gives the engine its rules. **A turn's start and a command's result wait for no lock:** a read waits for nothing (it takes no lock for the store, and only tries the lock, without waiting, when it has the good copy to bring up to date), and what writes the store for the session itself (what the observer saw, the mark on an entry that expired, which is out of use by the clock already) is done beside the turn on the blocking pool, so a process that holds the lock and has stopped holds up neither. A `remember` call writes the store itself, and waits for the lock as any change does. `project/list` waits for that work: a listing after a turn holds what the turn's commands showed. The policy and the checkpoints hold the session's knowledge weakly, so a closed session is let go. `cortex-knowledge` does not implement the tools' traits itself: it would have to depend on `cortex-tools`, which the workspace's edges do not give it, so the core does (a departure from the build plan's wording, accepted by the coordinator). |
| **The safe list** | The engine is given one exact allow rule (`exact_command_rule`, [06-permissions-and-sandbox.md](06-permissions-and-sandbox.md)) for each command that `runs_unasked`: in use **and accepted**. They go in with `Engine::set_learned` and never in the rules a session starts with, so they can be taken back. The entry's directory is not part of the rule: an accepted line is allowed wherever in the workspace it runs. A line the engine cannot write as an exact rule is not learned at all. The rules wait for the project's trust like the project's own allows, and the user is told once that they wait. `Knowledge` is the one place that holds a session's learned rules: a change of level makes a new engine, which is given them from there as it is put in force (`put_in_force`: read under this module's lock, and handed over with the swap under the policy's, since the re-decide step of step 12). **Before every command is decided** (`EnginePolicy::on_command`) the session looks at the two files' stamps, and when either changed since **its own rules were made from them**, or the first of the entries behind the rules has expired, it reads the files again there and then (a read that waits for nothing: the cost of a load, and only when a file changed) and gives the engine the rules that follow. (The comparison is with the snapshot the session's rules came from, not the one the project's sessions share: another session may have read the files again since, and only its own engine was given what it read.) So a `cortex project disable` in another process takes a command off the safe list before its next decision in this one, and another session's or process's write of something else (a command it saw for the first time) leaves this session's accepted commands running unasked. A session with no learned rule in force does not look: what was accepted elsewhere since comes at its next turn's start. A `project/update` reaches every session of the project in the process at once, a turn that is running included. The rules are changed, and handed to an engine, only under their own lock, so a change of level and a `disable` at the same moment leave the new engine with the later list. And they are made only from a reading of the files that is still the files' while that lock is held: a snapshot that another change has overtaken (it was read before a `disable`, and arrives after it) is read again there, so it cannot put back the rule the change took away, even in the moment between a command's look and its decision. **A call decided before its command was turned off** is decided again when its turn comes: turning a command off moves the policy's count of narrowings, and the loop decides again every call of the turn that has not started (since the re-decide step of step 12: [03-agent-loop.md](03-agent-loop.md), "A call is decided again"). Of two calls in one response, the second is then asked about though it was allowed when it was planned. |
| **The profile in an epoch** (`cortex-agent`'s `context::profile`) | The loop asks the source for the section when it builds a prompt, and the epoch's record holds the prompt from then on: nothing that happens to the profile changes a byte of a request's prefix. At each turn's start the section the source gives is compared with the one the model has (the epoch's `profile` hash, then the last `profile` notice of the epoch's history); when it differs, one notice carries the whole section as it stands, or says that the profile is empty now. After a summary the old epoch's profile notices are not replayed, live or when the log is read again: the new prompt holds the profile as it is then. A resume that begins an epoch and keeps its history (its tools or its prompt changed) cannot take an older notice out of that history, so when the history holds one the profile is said again, once, after it. The profile is outside `base`, so a profile that changed between two processes never makes a resume a new epoch; the resumed session is told the change as a notice. |
| **What the user is told** | A command that became active: what it is, that it is in the profile and still asks, and the two commands that accept it and take it out. A flaky test that became active, an entry that expired, an entry that came back, a file that cannot be read, a profile larger than its budget (how many entries are left out), accepted commands that wait for trust. Front ends get `project.updated` with the entries that changed. Each line and text in a notice goes through `cortex_base::visible`. |
| **Credentials** | A command line the secret filter finds a credential in is not observed. A proposal, an added or edited entry and a promoted one are refused with the reason when their text, evidence, command line, directory or check holds one: a promote writes all of an entry into the repository's file. An entry that holds one all the same (a hand edit, a shared entry) is left out of the prompt, and the user is told how many were. |
| **`project/list`, `project/update`** | A project is named by a session the client has open, or by a workspace root, which only a `user` client may do: both or neither is `invalid_params`, a path that cannot be a workspace `invalid_workspace`, and with no state directory there is no profile (`not_available`). A change is refused whole, with nothing written: `unknown_entry`, `unknown_action`, `invalid_entry` (an entry that does not validate, or that holds what looks like a credential), `shared_entry_read_only` (an `edit` or a `delete` of the repository's entry), `not_promotable`, `project_store_full`, `project_store_invalid` and `project_store_unwritable` for the private store, `shared_profile_invalid` and `shared_profile_full` for the repository's file. `project/update` is a `user` client's alone (`role_not_allowed`): an entry reaches the prompt, and a command the safe list, only by an act of the user's. The listing carries every entry, the shared file's as this user stands to them, and `problems`, the files that could not be read. |
| **`cortex project`** | The command line's way to what `/project` will show (step 24), for the project of the working directory, outside a session: `list` (the default; proposals first, then what is in use; `--all` adds what is turned off, `--kind` keeps one kind, `--json` prints the same choice of entries as one line of JSON, in `project/list`'s form, with every character that acts unseen written as a `\u` escape), `accept ID…`, `accept --shared` (every entry of the repository's file that waits for this user, with one command), `disable`, `delete`, `promote`, `edit ID TEXT`, `add KIND TEXT` (a command with `--purpose` and `--command`, and `--cwd`, `--read-only`), and `path` (where the two files are). An id is given as the listing shows it; one of the repository's file, which is 34 characters, may be cut short to a start that only one of them has (an id of the user's own store may not: those are numbered in turn, and the start of one that is gone names a later entry). The listing prints a command's line and directory whole, and says of each entry whether it asks (`active, asks`: it became active by itself) or was accepted, where it came from, its evidence and its check, and until when it stands. **It says no more than the engine will do:** an accepted command in a project that is not trusted as it is has a line saying that it still asks; an entry past its expiry is `expired` whether or not it was marked. Two things it cannot say are written down in the step's record: a store that cannot be read (its problem is printed), and an accepted line that is not one plain command, which asks every time. **An entry is two lines, whatever it holds:** its text, evidence and check are escaped, a line break included, so a text from the repository's file cannot begin a line that reads as another entry. A file that could not be read is said, never taken for one with nothing in it. The commands go through the core (`Core::project_entries`, `change_project_entries`, `project_files`), which makes the change with the operation's own function and checks (`knowledge::apply`), so nothing can be done on the command line that `project/update` refuses. A session of the project in another process takes a change at its next turn's start, and a command turned off before its next decision. |
| **Settings** | `[project] learn` (default `true`; the user's alone, read at start): off, nothing is observed, and what is known is used as before. `[project] profile_tokens` (default 1,500, from 0 to 8,000; a project may lower it; from the next epoch): section 5's budget. |

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
