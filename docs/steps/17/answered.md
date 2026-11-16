# Answered in step 17, on 2026-11-16

Step 17 builds the project profile and memory: `cortex-knowledge` (entries, the private store and the repository's shared profile, the text of section 5), with what `cortex-policy`, `cortex-agent`, `cortex-session` and `cortex-core` gain for it. This record holds the questions it put to the owner, the design as it was reviewed before it was built, and what changed in the building.

The specification is [architecture/14-adaptation.md](../../architecture/14-adaptation.md), "Project knowledge: profile and memory", with the build plan's step 17 and the "What later steps inherit" lines that name it.

## The questions, and the answers

Three choices of the design change what a feature does, and went to the owner through the coordinator with a recommendation each. All three were answered on 2026-11-16.

| # | Question | The answer, 2026-11-16 | Recorded in |
|---|---|---|---|
| Q1 | **Does text from the repository's shared file reach the prompt before this user accepted it?** 14-adaptation said the shared file "needs no trust to be read: like instruction files, it is text for the model". The step's brief listed that text reaching the prompt before acceptance as a mistake to prevent. | **No (the owner).** Every entry of `.cortex/profile.toml` is `proposed` for each user until that user accepts it. The acceptance is kept in the private store against the entry's content hash, so a changed entry is a proposal again, and one command accepts them all. The reasons given: the model can write the file (after the approval a protected path asks for), so an unreviewed shared file is a way round `remember`'s review; a cloned repository's file is a stranger's text; and section 5 sits in the system prompt under the harness's own heading. | [14-adaptation.md](../../architecture/14-adaptation.md), "Where it lives"; [06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md), "Project trust"; [security.md](../../security.md), section 2 |
| Q2 | **Is an observed command that became active by itself on the safe list?** The owner's decision of 2026-11-12 makes it *active* after two observations. 14-adaptation, 06 and 15-efficiency give the safe list and verification to *accepted* commands. | **No (the coordinator, as the pages already say).** Active puts it in the prompt and nowhere else. It runs without asking, and is offered to verification (step 19, build-plan line 456), only once the user accepts it, or wrote it. The notice that says it became active names the command that accepts it and the one that undoes it. | 14-adaptation, "How entries are used" |
| Q3 | **A field for the session log:** `profile` in the `epoch` entry, the hash of section 5 as the epoch's prompt holds it, beside `instructions`. Step 15's rule is "the session log format beyond agreed additions". | **Agreed (the coordinator).** Absent means no profile was given; a `model` epoch carries it over. | [07-sessions-and-context.md](../../architecture/07-sessions-and-context.md), "On-disk layout" |
| | **Step 43's "Must not change: the memory file format"** meets this step's strict reading of the store: step 43 needs fields for a check's state. | **"Except by a new version" (the owner):** a build that reads version 1 refuses a version 2 file whole and never writes one. | [build-plan.md](../../build-plan.md), step 43 |
| | **The item an entry was learned from** (`learned.item`): `cortex-tools` gives a tool neither its call's id nor its item's. | **An outstanding item (the coordinator);** the tools' API is not changed in this step. | [outstanding](../../outstanding/the-item-an-entry-was-learned-from.md) |
| | **Two departures from the build plan's hand-off lines:** the observer hooks the core's shell wrapper and does not read the aggregates, which hold no command text; and the core, not `cortex-knowledge`, implements `cortex_tools::Proposals` and `CommandProfile`, since the crate may depend only on the protocol and the base crate. | **Accepted (the coordinator).** The build plan's lines under steps 10, 11 and 16 are corrected. | [build-plan.md](../../build-plan.md) |

**The step's design note could not be a file of its own:** a step's folder holds `answered.md` and `ci.md` and nothing else (`cargo xtask links`), so the note is the next section, as step 23's is in its record.

## The design, before building

Written before the code, read by an independent reviewer, and sent to the coordinator. **A choice marked "to confirm by measurement" hangs on a number;** what was measured is under "Measured", below.

### 1. The two files

**The private store** is `<state dir>/projects/<project key>/knowledge.toml` (`StateDir::knowledge_file`, which exists). **The shared file** is `<workspace>/.cortex/profile.toml`.

```toml
# knowledge.toml
version = 1

[[entry]]
id = "k_3"
kind = "command"                       # command | convention | flaky | fact
text = "Runs the tests."
status = "active"                      # proposed | active | disabled
basis = "observed"                     # observed | proposed; absent: the user wrote it
learned = { session = "s_…", item = "i_…", at = "2026-11-16T10:00:00Z" }   # absent: the user wrote it
evidence = "Ran and succeeded twice."
confirmed = "2026-11-16T10:00:00Z"
expires = "2027-02-14T10:00:00Z"       # absent: does not expire
accepted = "2026-11-16T10:05:00Z"      # when the user accepted it; absent: not accepted
seen = 2                               # for an observed entry: consistent observations
command = { purpose = "test", run = "cargo test", cwd = "crates/app", usual_ms = 4200 }
check = { exists = "Cargo.toml" }      # or contains = { path, pattern }, or command = "…"

[[shared]]                             # what this user decided about a shared entry
hash = "9f2c…"                         # the shared entry's content hash
status = "active"                      # active | disabled
at = "2026-11-16T10:06:00Z"
```

```toml
# .cortex/profile.toml
version = 1

[[entry]]
kind = "command"
text = "Runs the unit tests."
command = { purpose = "test", run = "cargo test" }

[[entry]]
kind = "convention"
text = "Errors are returned, never panicked on."
```

| Point | Choice |
|---|---|
| **Versions** | Each file has `version = 1`. A newer version refuses the whole file. |
| **Strict reading** | The private store refuses an unknown key or value, whole: an accepted command becomes an allow rule, so an entry is an enforced value in the sense of [decision 0028](../../decisions/0028-enforced-values-fail-closed.md). In the shared file a syntax error, a newer version or a cap refuses the whole file, and an entry with a key or a value this build does not know is left out alone, and reported: an entry only ever adds, so leaving one out widens nothing, and a team on two builds keeps the entries both understand. A shared entry carries no status, source, date or acceptance of its own, and the file no table of decisions: a repository cannot say that its entry is in use. |
| **A shared entry's id** | `s_` and the first 32 hexadecimal digits (128 bits, as a project key has) of its content hash (kind, text, command, check, each with its length). The same on every machine, and another id when the entry changes, so an id names exactly what the user was shown: a shorter one could be made to collide by a repository. The command line takes a prefix that names one entry. |
| **A private entry's id** | `k_<n>`, from a counter the file keeps (`next`) that never goes down, given under the lock: the id of a deleted entry is not given again, so an `accept` sent from an old listing cannot land on an entry the user never saw. An entry a person adds by hand may leave its id out and is given one. |
| **A valid hand edit** | Taken up at the start of each session's next turn (a look at both files' size and modification time, `cortex_base::watch`), at `project/list` and `project/update`, and before every write, which reads the file again under the lock. |
| **An invalid hand edit** | Reported: a notice to the user with the file, the line and what is wrong, and `problems` in `project/list`. Nothing of the invalid file is used. **The file is never written over, moved or deleted.** |
| **Where the old state is kept** | For the private store, in `knowledge.toml.good` beside it: the bytes of the last store this build read or wrote as valid, written owner-only and atomically after the store itself, and brought up to date under the lock by a load that finds a valid store that differs from it (a valid hand edit). A process that finds `knowledge.toml` invalid uses `knowledge.toml.good`; one that finds neither readable uses nothing. A session that had the file loaded keeps what it had. **A store that is not there is an empty store, never the good copy:** deleting the file is how a user starts again. For the shared file, in the memory of the sessions that had loaded it; a process that starts on an invalid shared file uses none of it. |
| **While the private store is invalid, nothing runs without asking for it** | The state in its place (the good copy, or what a session held) feeds the prompt, with the notice. It feeds the safe list nothing: the invalid file may hold the edit that turned a command off. |
| **Writing while the private file is invalid** | Refused, and said once a session: an observation is dropped, `remember` answers that the proposal could not be stored, `project/update` answers `project_store_invalid`. Writing would mean choosing between the user's edit and the old state. |
| **Two sessions writing at once** | Every change is an operation on the file as it is read under the lock (`cortex_base::private::hold_lock`, the file `knowledge.toml.lock`): read, apply, write. A session never writes its own copy of the file. Two that accept different entries both keep theirs; ids are given under the lock. Readers take no lock: the file is replaced in one step. |
| **Owner-only, atomic** | The private store, its good copy and its lock: `write_atomic` with `owner_only`, in a directory made with `private::create_dir_all`. The shared file is the repository's: written atomically with the directory's own permissions, and only by `promote`. |
| **The shared file is read where the agent may read** | Through the session's path resolver and the permission engine's gate, as an instruction file is: `.cortex` or `profile.toml` as a link out of the workspace is not followed, and a file a rule denies is not read. It is opened with `open_regular`. `promote` refuses unless `.cortex` is a plain directory inside the workspace. |
| **Caps** | Either file: 256 KiB, read with a bound (*to confirm by measurement*: the cap is the size that still loads in 5 ms on the desktop). Private: 500 entries, of which at most 50 are a model's proposals waiting for review. Shared: 200 entries. An entry: a text of 600 characters; evidence, a command, a pattern or a path of 400 (the limits `remember` already has). A file past a cap is invalid, as above. A full store takes no new observation or proposal and says so. |
| **The project key** | `AbsPath::project_key` of the resolved workspace root, as sessions, trust and approvals are keyed (step 8). **A moved workspace** has another key: its private store is not found, and stays under the old key; the shared file moves with the repository. **Two worktrees of one repository** have two keys and two private stores, and each has the repository's shared file as its own checkout holds it. **Two repositories** have two keys. Nothing is shared across keys: an acceptance in one worktree allows nothing in another. |

### 2. The command observer

| Point | Choice |
|---|---|
| **What is observed** | A `shell` call's command, directory, exit status, duration and reported changes, where the core already wraps the shell tool (`RecordingShell`, which folds a command's changes into the round's checkpoint). Nothing else is on the tool's path: the observation is handed to the store's worker on the blocking pool and the call returns. |
| **Not the aggregates** | The statistics hold no command text, by their own rule ("no text of a prompt, a file or an answer"), and step 16 built no `command` event. So the observer reads neither the aggregates nor the `tool` and `turn` events: they cannot name a command. The build plan's line for step 16 ("Step 17 learns from the aggregates … and from the `tool` and `turn` events") is changed to say so. A departure. |
| **The same command** | The line as the model wrote it, trimmed, with runs of blanks made one space, and the directory it ran in relative to the workspace root. Case is kept. **Only a line of plain words is observed:** letters, digits and `_ . / : = @ % + , -`, at most 160 characters, no quote, no operator, no pattern character, no line break, and one that `cortex-policy` can write as an exact rule. An observed line reaches the system prompt without a review, so it must not be able to carry a sentence: this is the one place text a model chose gets there, and [security.md](../../security.md), section 2, gains a row for it. A line the session's secret filter would mask is not observed. A run outside the workspace is not observed. |
| **Its class** | A table of program and subcommand, after any leading `NAME=value` words: `cargo build`, `make`, `npm run build`, `go build`, `tsc` are *build*; `cargo test`, `cargo nextest run`, `pytest`, `go test`, `npm test`, `make test` are *test*; `cargo clippy`, `cargo check`, `eslint`, `ruff check`, `mypy`, `make lint`, `cargo fmt --check` are *check*; and so on for the common ecosystems. A command the table does not class is not observed. *Format* and *run* are classes a user's own entry may have; the observer learns build, test and check, as 14-adaptation says. |
| **Two consistent observations** | The same command, in the same directory, of the same class, exits with success in two different tool calls. A failure between them does not count against it: a test command fails while the work is unfinished. The first makes a `proposed` entry with `seen = 1` (the user may accept it at once); the second makes it `active`, with a notice, unless the user disabled it. Observations in two sessions count. **Entries seen once do not fill the store:** at most 40 are kept, the oldest goes when a new one comes, and one not seen again in 7 days is dropped. |
| **An entry the user disabled is not learned again.** One the user deleted is forgotten, and may be learned again from two new observations. The notice names `disable` as the undo. |
| **Staying confirmed** | A later success of an active observed command moves `confirmed` and `expires`, and its usual duration (a moving average), at most once a day: one write a day a command, not one a run. |
| **Its check** | Where the program is a path inside the workspace (`./scripts/test.sh`), `exists` of that path. No form says "is on the search path" without running something, so a program found there gets none (step 43 runs checks: build-plan line 714). |
| **A test seen to flip** | A *test* command fails, then the same command in the same directory succeeds, in one session, with **no change** between the two runs. **No change** is measured against what the harness saw change in that session: no file tool wrote a file, and no command (the failed run included) reported a changed file or a look that did not finish. A result that does not say what changed (detection off, a call planned as read-only, a detector that failed) counts as a change. A run that timed out, was interrupted or was blocked by the sandbox is not a failure, and the two runs must have run under the same confinement: "fails in the sandbox, passes outside it" is not a flip. An edit made outside the harness between the runs is not seen; that is why one flip only proposes. In a project that is not trusted, changes are found by modification times, which count a build's output, so little will ever be seen to flip there (*to confirm by measurement*). The first flip makes a `proposed` `flaky` entry that names the command; the second makes it active, with a notice. Which test inside the command flipped is not read from its output: that is step 19's reading of test output (build-plan line 456). |
| **`CommandProfile`** | `is_build_or_test(command)` is true when the line, made the same way, is an active command entry of class build, test or check, or the table classes it so. It reads the store's snapshot in memory and never waits. |
| **Where the seams are filled** | `cortex-knowledge` depends on the protocol and the base crate only ([01-workspace.md](../../architecture/01-workspace.md)), so it cannot name `cortex_tools::Proposals` or `CommandProfile`. The core implements both over the store, as it fills every other seam. The build plan's "implements `cortex_tools::Proposals` in `cortex-knowledge`" is changed to say so. A departure. |

### 3. Section 5

```
Project profile: what is known about this project from earlier sessions, each entry
learned by Cortex-Harness or written by the user, and none of it part of the present task.

Commands:
- test: `cargo test` (in `crates/app`)
- build: `cargo build`

Conventions:
- Errors are returned, never panicked on.

Flaky tests:
- `cargo test -p net` has failed and then passed with nothing changed.

Facts:
- The release script is scripts/release.sh.
```

| Point | Choice |
|---|---|
| **What is in it** | Entries that are `active` and not past `expires`, private and shared (a shared one only when this user accepted it at its present hash, Q1). Never `proposed`, `disabled` or expired. |
| **Order** | Commands (by class: build, test, check, format, run), conventions, flaky tests, facts; within each, private before shared, then by id. Nothing in the text depends on a clock, a duration or a count, so confirming an entry does not change it. |
| **The budget** | `project.profile_tokens`, default 1,500, at a fixed 3 bytes a token (the estimate's starting ratio): 4,500 bytes. The measured ratio is not used, so the same entries always give the same bytes. Entries are taken in order while they fit; then one line: `(N more entries are not shown, to keep this within its size.)`, and the user is told once a session. |
| **Escaping** | Every piece of every entry goes through `cortex_base::visible`: a line break, a control or an unseen character is written as an escape, so an entry is one list line and cannot begin a line, a heading or a message. A test renders the test kit's hostile names and the notice lead-in itself. |
| **It leaves `base`** | `prompt::build` takes section 5 as text and leaves it out of the `base` hash, as it leaves section 4. `Built` and the epoch record its hash (Q3). A resumed session whose profile changed goes on in its epoch. |
| **The change notice** | At the start of a turn, where changed instruction files are told: when the section the harness would build now is not the one the model has (the epoch's, or the last notice's), one `notice` item of a new source, `profile`: `The project profile changed. This is all of it now, in place of what you were given before:` and the section, or `The project profile has no entries now. Disregard the one you were given before.` At most one a turn; never inside a turn. |
| **The next epoch** | Built with the whole current profile. A summary's replay leaves the old epoch's profile notices out, live and read again, as it leaves the map's out: one replayed after the new prompt would put an old profile over the current one. What the model has is the epoch's hash, then the profile notices on the branch after the epoch and after the last summary, so a rewind and a fork read it the same way. 07's table of what follows the history gains the row. |
| **The seam** | `ContextParts::profile`, an `Arc<dyn ProfileSource>` of `cortex-agent`'s own, asked on the blocking pool when a prompt is built and at each turn's start; the core fills it. When the files cannot be read the section is what the last good state gives, or nothing, and the user is told. |
| **Secrets** | An entry whose line the session's secret filter would change is left out of the section, and said: a masked prompt in the log would make every resume a new epoch. `remember`, `add` and `promote` refuse such a text. |
| **Fixed for an epoch** | The prompt is built once and sent from the `epoch` entry. Tests, with real protocol bytes (`built_as`): the same system prompt through a steer, an interrupt, a retry, a resume after the store changed, and a change of model; another at a compaction. |

### 4. The learned safe list

| Point | Choice |
|---|---|
| **What `cortex-policy` receives** | One `allow exec` rule, of source `Learned`, for each command entry that is active, not expired, and **accepted by this user** (accepted, or written by the user; a shared one accepted at its present hash). The pattern is the exact line, word for word, written by a new function of `cortex-policy` with the writer an approval's exact-line rule uses. A line it cannot write as a pattern gives no rule, and the entry says so in its listing: an operator, more than one command, a word with a blank, a word with a pattern character (`* ? [ ]`, which a rule would read as a wildcard, and a last `*` as "any further words"), a home path. |
| **Additions to `cortex-policy`, none to evaluation** | `exact_command_rule(command, syntax) -> Option<Rule>` and `Engine::set_learned(rules)`, which replaces the rules of source `Learned` and compiles the set again; on a failure it leaves none of them in force. How a rule is matched, ranked and held back for trust is untouched: `RuleSet::compile` already sets a `Learned` allow aside until `Parts::trusted`. |
| **When** | *At open:* nothing is read (the start-up path). *At the session's first turn,* before its first request, with the store's first load. *At acceptance and rejection:* `project/update` gives every live session of the project in the process its new rules before it answers; another process takes them at its next turn's start. *At trust:* the engine is made with the project's trust, so a session opened in an untrusted project holds its learned allows back for its life, and the next session after `cortex permissions trust` applies them. |
| **A hostile repository** | Its `profile.toml` is proposals (Q1): nothing of it reaches the prompt, verification or the safe list until this user accepts an entry, and an accepted command still waits for the project's trust. Changing the entry afterwards makes it a proposal again. |
| **Not in the standing lease** | The lease records the rules in force when the session started. Learned rules arrive after, as approvals do, and are listed the same way. |
| **One place holds a session's learned rules** | The core keeps them beside the session, and both `set_learned` and a change of level (which makes a new engine from the session's template) read them there. The template never holds them: a rule turned off must not come back with the next change of level. Tested both ways. |
| **A rejection made by another process** | Before a decision about a command, a session that holds learned rules looks at the two files' stamps (*to confirm by measurement* against the decision's 1 ms). When either changed it drops every learned rule at once, and reads the files again at its next safe boundary. So `cortex project disable` in another terminal takes a command off the safe list before its next run, not at the next turn. |
| **`promote` and the sandbox's watch** | `.cortex` is a protected path the sandbox looks at around a confined command. The core tells that look about a write `promote` made, so the write does not stop a running turn as a change from outside. |

### 5. The protocol and the commands

**Additions, all tolerant** (optional fields and new open-enum values; no new `PROTOCOL_VERSION`):

| Addition | For |
|---|---|
| `ProjectEntry.accepted_at`, `ProjectEntry.observations` | The panel and `cortex project list`: whether a command is accepted, and how often an observed entry was seen. |
| `ProjectUpdateParams.add`: a list of new entries (kind, text, command, check) | The "written" path: an entry the user adds is active at once. |
| `ProjectEntriesResult.problems`: a list of sentences | A file that is invalid, and what is used in its place. |
| `NoticeSource::Profile` (`profile`) | The change notice to the model. |
| No new kind of notice | What the user is told (an entry became active, expired, or was disabled; a file is invalid; the section was cut) is an `info` notice, with `project.updated` beside it carrying the entries. A kind of its own would add a line to the golden transcript for a message a client shows the same way. |

**`project/list`** names a project by a session the client has open, or by a workspace root, which only a `user` client may do. **`project/update`** is a `user` client's alone. It is all or nothing, under the lock, and answers the changed entries; every client of the project's sessions gets `project.updated`.

| Action | Does | Refuses |
|---|---|---|
| `accept` | Makes a proposed, disabled or expired entry active and accepted, and renews its expiry. For a shared entry, records the acceptance of its present hash. | |
| `edit` | Replaces a private entry's text. | A shared entry: it is the repository's file to edit. |
| `disable` | Takes it out of use and keeps it; for a shared entry, for this user. | |
| `delete` | Removes a private entry. | A shared entry. |
| `promote` | Moves an accepted private entry to the shared file, keeping this user's acceptance. The shared file is written first, never through a link, and the private entry removed after: a stop between the two leaves the entry in both, which is read as one. | An entry that is proposed, disabled or active without the user's acceptance; an invalid shared file; `.cortex` that is not a plain directory in the workspace, or a `profile.toml` that is a link. |
| any | | An unknown id (`unknown_entry`), an unknown action, a text that is empty or too long (`invalid_entry`), a client that is not a `user` (`role_not_allowed`), an invalid private store (`project_store_invalid`). |

**`remember`** answers as it does now: stored and not in use, or already known (the same kind and text in any status, so a rejected proposal is not proposed again). New: an error when the store is full of proposals or invalid.

**`cortex project`**: `list [--kind K] [--all]`, `accept`, `disable`, `delete`, `promote` (each with ids), `edit ID TEXT`, `add KIND TEXT [--purpose P --command LINE --cwd DIR]`, `path`. Text output is a table by kind with id, status, source and expiry, every entry's text through `visible`; `--json` prints the protocol's result. It talks to the core as the other subcommands do.

**For step 24's `/project` panel** (build-plan line 524): `project/list`, `project/update` with `add`, `project.updated`, and each entry's `accepted_at`, `observations`, `basis`, `scope` and `check`. It will want a way to edit and delete a shared entry, which this step refuses.

### 6. Budgets

Rows proposed for [performance-budgets.md](../../performance-budgets.md), "Data paths". **Each is to confirm by measurement** (release build, the desktop, then hosted runners with `try`'s `perf`), on a typical store (40 entries) and one at the caps.

| Budget | Proposed |
|---|---|
| Load both files, parse and validate | ≤ 1 ms typical, ≤ 5 ms at the caps. On the blocking pool, at the first turn's start and after a change. Never at start-up. |
| The look for a change at a turn's start, nothing changed | ≤ 50 µs: two `stat` calls. |
| Render section 5 | ≤ 100 µs typical, ≤ 1 ms at the caps. |
| The observer, per `shell` call, on the call's path | ≤ 20 µs: make the line the same, class it, hand it over. Nothing for any other tool. |
| The store in memory | ≤ 100 KB typical, ≤ 1.5 MB at the caps, one copy a project a process. |

**The start-up path stays untouched.** Making a session reads neither file; the first read is at the first turn's start. A test in `cortex-core` holds it with `cortex_testkit::Untouched`, as the index's does. No existing budget is loosened.

### 7. The cut into three pull requests

| PR | Branch | Holds |
|---|---|---|
| 1 | `worktree-step-17-store` | `cortex-knowledge`: entries, the two file formats, strict reading and the caps, the store (load, the lock, the good copy, operations, expiry), rendering section 5 as text, the protocol's additions with schema and golden values, the load and render measurements. And the three small seams that are testable alone (the reviewer's cut): `cortex-policy`'s two additions, the `epoch` entry's `profile`, and section 5 in `prompt::build` with its place outside `base`. |
| 2 | `worktree-step-17-wiring` | The observer; `remember` on the store; the profile source, the change notice and its replay rule in `cortex-agent`; the core's wiring (`project/*`, `CommandProfile`, the learned rules, the notices, the settings); the start-up and other-project tests. |
| 3 | `worktree-step-17` | `cortex project`; the remaining measurements; the records and the architecture pages. |

### Settings

| Key | Default | A project's file |
|---|---|---|
| `project.learn` | `true` | The user's alone: no class of setting says "a project may turn it off and not on", and one key does not earn a class. |
| `project.profile_tokens` | `1500`, from 0 to 8,000 | May lower it. |

### Left to later steps (the design)

| What | Whose |
|---|---|
| Running `test` and `check` entries at the end of a turn, and retrying a known flaky test once | Step 19 (build-plan line 456). It reads the command entries that are in use **and accepted**, by class, and puts each to the permission engine. A flaky entry names a command, not a test: it earns that command one retry, and a failure after the retry is a failure, never "flaky". |
| Running checks | Step 43 (line 714). The `check` field is stored, shown and never run here. |
| The `/project` panel | Step 24 (line 524). |
| Learned commands in the trust question and the first-run flow's command detection | Step 25 (line 533). Until then a session in an untrusted project says, in the notice, that its learned commands wait for trust. |
| The `command` statistics event of 14-adaptation's table | Not built by step 16 and not needed here: an entry keeps its own count and usual duration. 14-adaptation's table is changed to say so. |
| The item an entry was learned from | `cortex-tools` gives a tool neither its call's id nor its item's (`Proposal` has the turn, `RunCx` neither), so `learned.item` stays empty: an item in `docs/outstanding/`. |
| Fields for a check's state (flips, the last result, an approved command) | Step 43 (line 714) adds them under `version = 2`: a build that reads version 1 refuses a version 2 file whole and never writes one. Its "Must not change: the memory file format" should read "except by a new version"; raised with the coordinator. |
| A workspace below a repository's top | Has its own key and looks for its own `.cortex/profile.toml`: the repository's, at its top, is not read. Stated in 14-adaptation. |

## After the independent read of the design

A reviewer that had not seen the reasoning read this note against the specification and the code before anything was built on it. Its findings, each taken up above:

| # | Finding | Change |
|---|---|---|
| 1 | An active command the user had not accepted was offered to verification; 15-efficiency gives verification accepted commands only. | Q2, and the line for step 19. |
| 2 | A private id was given again after a delete, so an action from an old listing could land on an entry the user never saw; a 48-bit shared id could be made to collide. | The `next` counter; 128-bit shared ids. |
| 3 | The good copy went stale after a valid hand edit, and could bring back a command the user had turned off. | The good copy follows valid loads; a missing store is empty; an invalid store feeds the safe list nothing. |
| 4 | "Exact" did not rule out a rule's own pattern characters. | Section 4. |
| 5 | A change of level would lose learned rules or bring back a disabled one. | One place holds them. |
| 6 | `promote` could write through a planted link, lose an entry between its two writes, and stop a turn as a change from outside. | No link followed, the shared file first, the watch told. |
| 7 | The shared reader had to be shown to refuse every key that carries a user's decision. | A structure of its own, and a test of each key. |
| 8 | A profile notice replayed after a compaction would override the new prompt. | Left out of the replay. |
| 9 to 17 | Secrets in learned text; model-chosen text in the system prompt; holes in the flip rule; a flaky entry hiding real failures; a store filled by entries seen once; a rejection from another process waiting a turn; step 43's room to grow; the item an entry came from; pages this step changes. | Sections 2 to 5 and "Left to later steps". |
| 18 | The second pull request held most of the risk. | Three small seams moved to the first. |

## After the review of the first pull request

An independent reviewer read the whole diff of the first pull request (the store, the two files, section 5's text, the policy's two additions, the epoch's profile) against the specification, without the author's reasoning. Its findings, each with the fix and the test that holds it:

| # | Finding | Fix | Held by |
|---|---|---|---|
| B1 | **The 256 KiB cap was checked when a file was read and never when one was written.** 243 of the longest entries fill it, well below the cap of 500 entries, so ordinary use could write a store that every later load refused whole, its good copy with it; and a `promote` could do the same to the shared file for a whole team. | `Store::change` refuses a change that would pass the cap (`project_store_full`) and reads its text back before it is written; `append_shared` holds the shared file to the same size. | `store::a_change_that_would_pass_the_size_cap_is_refused_and_the_store_stays_readable`, `project::a_promote_that_would_pass_the_shared_files_size_is_refused`, `file::tests::an_entry_that_would_take_the_shared_file_past_its_size_is_not_added` |
| B2 | **The snapshot after a change took the store's stamp after the lock was let go.** Another process's change in that moment (a command turned off) was then recorded beside our own state and never seen. | The stamp is taken in `Store::change`, under the lock, and handed on. | `store::a_change_made_by_another_right_after_ours_is_seen`, which holds a point between the two |
| S1 | **Every load could wait for the lock** (to bring the good copy up to date), though the docs said readers take none: a stuck writer would hang every session. | The lock is tried, and the copy left for the next load when it is held. | `store::a_load_does_not_wait_for_a_writer` |
| S2 | **A hostile shared file's key names reached `problems` unescaped** (through the parser's own message), and from there clients and the terminal. | Every problem's message, and what is said of an entry left out, goes through `visible`. | `file::tests::a_problem_never_carries_what_acts_unseen` |
| S3 | **A hand-edited id or counter near the top of 64 bits** overflowed, and in a release build wrapped round to an id in use. | A number past 2^53 refuses the file. | `file::tests::a_number_past_what_an_id_may_hold_refuses_the_file` |
| S4 | **`project/update`'s `add` was read leniently,** against decision 0028: an entry the user writes is accepted at once. | `NewEntry` and its command (`NewCommand`, a type of its own) refuse a field this build does not know. | `cortex-protocol`'s `tests::the_golden_values_round_trip_exactly` |
| S5 | **The link rules were tested on Linux only, and the write's own link checks nowhere:** the read's check stood in front of them. | The link tests run on both systems (a symbolic link, made on the Windows runner as `cortex-base`'s are); a test plants a link between the read and the write, at the file and at `.cortex`. | `project::a_link_planted_between_the_read_and_the_write_is_not_written_through`, `a_shared_file_behind_a_link_is_not_read`, `a_promote_does_not_write_through_a_planted_link`, `store::a_link_at_the_store_is_not_followed` |

**From its notes,** fixed with a test each: a private id may not begin `s_` (it would stand in front of a shared entry); a `promote` that stopped between its two writes is finished by the next (`project::a_promote_that_stopped_half_way_is_finished_by_the_next`); a directory that names a root is refused as written, where it had been read as a path below the workspace (`project::a_rooted_directory_is_refused_not_reread_as_a_relative_one`); a check's command and pattern take no control character; an entry's hash is of its text with its blanks made one, so a checkout that changed line endings keeps its ids; and the test of the cut's singular line now always asserts. **Written down, not changed:** the evidence is outside the hash; ids start again after a store is removed; an older server and `add`; PowerShell compares a program's name without regard to case.

**For the second pull request,** from the same notes: learned rules are never passed in `Parts::rules`, where they would enter the standing lease and outlive `set_learned`; and an entry's directory and a check's path are resolved against the workspace by whatever uses them, since the store checks their text only.

The breaks of these rules are in `mutations/step-17.breaks`, under "after the review of the store".

### The second read

The same pull request's second and last round: a reviewer read the fixes alone. No finding lost state or let an entry through; three were fixed, with the notes that had a cheap fix.

| # | Finding | Fix | Held by |
|---|---|---|---|
| 1 | **A `promote` wrote the repository's file before the store's size was checked,** so a change refused as "nothing written" had already written it. | The store is asked whether it can take the change before the shared file is written (`store::writable`). | `project::a_promote_the_store_cannot_take_writes_nothing_to_the_shared_file` |
| 2 | **The stamp a change returned was taken after its good copy was written,** so a person's edit saved in that moment (no lock holds a person's editor) was taken for the state returned. | The stamp is taken before the read, and again the moment the store is written. | `store::an_edit_saved_while_a_change_is_written_is_seen` |
| 3 | **An id at exactly 2^53 was read, and then no change to that store could be written;** and nothing tested the new rule that a change is read back before it is written. | The counter is held to the bound once every id is given; the read-back has a test and a break. | `file::tests::a_number_past_what_an_id_may_hold_refuses_the_file`, `store::a_change_that_could_not_be_read_back_is_not_written` |
| | **At its cap the store refused the changes that only narrow** (`disable` adds two bytes), and a lapse that could not be written was tried at every turn. | A new entry is refused within 32 KiB of the cap, which keeps room for those. | `store::a_store_near_its_cap_takes_no_new_entry_and_can_still_be_told_to_use_less` |
| | **A load could still wait for the lock** where no lock file existed yet. | The lock file is made as a change makes it, then tried. | by reading; no test can hold another process in that instant |
| | The shared file's size refusal had the code of an invalid file; `Store::change`'s errors and `NewEntry`'s strictness were described wrongly. | `shared_profile_full`; the comments corrected (a `check` in `add` is read as every check is). | `project::a_promote_that_would_pass_the_shared_files_size_is_refused` |

**Left open,** as items: [a second write of the same size within a clock's tick on Windows](../../outstanding/a-same-size-rewrite-within-a-clock-tick-on-windows.md), which `FileStamp` cannot tell there. **Written down, not changed:** a spacing-only edit of an accepted shared entry's text keeps its acceptance (the hash is of the text with its blanks made one; a command line, a path and a pattern are hashed as they are).

## The second pull request: the wiring

What the design's sections 2 to 5 became is in [14-adaptation.md](../../architecture/14-adaptation.md), "As built in step 17" (the observer, learning, a session's seams, the safe list, the profile in an epoch, the notices, credentials, the two operations, the settings). What was settled while building, beyond the design above:

- **The observer's table classes a program by its file name,** whatever directory is written before it: `./gradlew test` is a test run, and so is a file in the workspace that is called `jest`. A program in the workspace that the table does not know is not observed; the user writes its entry.
- **A session kept in memory tells no flip.** The harness does not see what its file tools write there, so "nothing changed between the two runs" cannot be said.
- **The look before a command is decided.** A session reads the files at a turn's start, so a rejection made in another process would have waited for the next turn. The files' stamps are looked at before each command's decision (two `stat` calls, and only while the session has a learned rule), so it takes effect at the next decision.

### After the read of the trust path

As soon as the path existed (a shared or learned command reaching `cortex-policy`, an entry reaching the prompt), an independent reviewer read those two things and nothing else, without the author's reasoning. Its findings, each with the fix and the test that holds it:

| # | Finding | Fix | Held by |
|---|---|---|---|
| B1 | **A session kept a turned-off command's rule when another session of the project read the files first.** The look before a command compared the files with the snapshot the project's sessions share, which the other session had just replaced; only the other session's engine was given the new rules. | The rules are kept with the snapshot they were made from, and the look compares the files with that one. | `knowledge::sys_a_command_turned_off_elsewhere_asks_though_another_session_read_the_files_first` |
| B2 | **A change of level could put a turned-off command back on the new engine.** The rules were copied, the lock let go, and the copy then handed to the engine: a `disable` between the two was overtaken by the older list, on an engine the module then believed had no rule. | The rules are handed to an engine only while their lock is held (`regive`). | `knowledge::tests::the_rules_are_handed_to_a_new_engine_while_they_are_held` |
| S3 | **A resume into a new epoch replayed a notice of an older profile after the new prompt.** A resume whose tools or prompt changed begins an epoch and keeps its history; a profile notice in it then stood after the prompt's section as the last word, and no later turn corrected it, the hashes being equal. | When such a history holds a profile notice, the profile is said again, once, after it. | `cortex-agent`'s `profile::a_resume_into_a_new_epoch_says_the_profile_again_after_an_older_notice` |
| S4 | **A call decided before its rule was taken back runs on that decision:** of two calls in one response, the second runs unasked though its command was turned off while the first waited. | **Not changed here:** it is how the loop treats every rule (a revoked approval, a lowered level), and deciding again is a change to the loop's tool round. Shown with a test, written down in 14-adaptation, and left as an item with a recommendation (closed by [the re-decide step](../step-12-redecide/answered.md)). The coordinator's answer: it stays out of this step, and goes to the queued "re-decide" step, which changes the same place. | the item |
| S5 | **A credential in an entry's check, its directory, or a promoted entry's evidence was not looked for.** A check is kept in the store, and a promote writes all of an entry into the repository's file. | Every text of an entry that a file keeps is looked at: in a proposal, an added entry, and a promote. | `knowledge::sys_a_credential_is_not_learned_and_not_remembered`, `an_entry_with_a_credential_in_its_evidence_is_not_promoted`, `knowledge::tests::every_text_of_an_entry_that_a_file_keeps_is_looked_at_for_a_credential` |

**From its notes:**

- **Expiry was not enforced inside a turn:** fixed. The rules are kept with the time the first of their entries expires, and the look before a command drops them then (`knowledge::sys_an_accepted_command_that_expires_while_a_turn_runs_asks`).
- **The directory is not part of a learned rule:** written down, in 06 and 14.
- **The message for rules the engine refused carried a pattern unescaped:** it goes through `visible` now. No test: the engine refuses no rule that `exact_command_rule` writes.
- **The observer is a way into the prompt that no one reviews,** and the words of an observed line are the model's own: `npm test` with arguments of its choosing is a test run. The plain-words rule keeps a line from carrying a quote or a sentence with blanks; it does not keep it from spelling one with `-` or `,`. Not changed: the alternative is to keep an observed command out of the prompt until it is accepted, which the coordinator's answer to the second question decided against. [security.md](../../security.md) says what is left. The coordinator left it as built, and raised it with the owner as a security trade-off, because the text lasts across sessions.
- **A log written before `base` existed** begins a new epoch at its first resume once it has a profile. It costs one epoch, once, on a log no released build wrote.

**The same reading found one of the author's own tests empty:** three assertions compared a tool result's outcome with `"ok"`, which no outcome is called. They compare with `error` now.

### After the review of the second pull request

An independent reviewer then read the whole diff against the specification, without the author's reasoning, and checked the fixes above. Its findings, each with the fix and the test that holds it:

| # | Finding | Fix | Held by |
|---|---|---|---|
| B1 | **The Windows half of the flaky test's program could not work:** `echo %m%>file` is `echo 1>file` to `cmd`, a redirection of a handle. | The redirection is written first, and `exit` is kept out of a block. | the test itself, on a Windows runner |
| B2 | **The directory a command ran in reached the system prompt unchecked.** The line was held to plain words and looked at for a credential; the directory beside it was neither, and a folder can be named anything, a sentence with a backquote included. | A run is observed only in a directory of plain characters, at most 160 of them, and the directory is looked at for a credential. | `observe::tests::a_run_in_a_directory_that_is_not_plain_words_is_not_observed` |
| 3 | **A closed session was never let go:** its knowledge held the policy and the checkpoints, whose callbacks held the knowledge. | The callbacks hold it weakly. What the sessions of a project share is held weakly by the core too, and let go with the last of them. | `knowledge::a_closed_session_lets_its_knowledge_go` |
| 4 | **Any write to the store by another session or process took this session's safe list away for the rest of its turn** (a first sighting, a daily confirmation), and with no one to answer, its accepted commands were refused. | When a file changed, the look before a command reads the files again and gives the engine the rules that follow. | `knowledge::sys_a_write_to_the_store_by_another_process_does_not_take_the_safe_list_away` |
| 5 | **The table classed deploying lines as builds and tests:** `make test deploy`, `make -C build deploy`, `npm run build-and-deploy`, `tox -e release`, `mvn -pl test deploy`, `cmake --build . --target install`. | A sub-command is the first word after the program; several targets or goals must all class the same; a script's name is classed whole, or by the word before its first `:`; `tox` and `nox` with an environment named, and `cmake` with a target named, are not classed. | `observe::tests::the_table_classes_the_common_builds_tests_and_checks`, with the reviewer's lines |
| 6 | **Two flips that were not flips:** a command run outside the workspace was not handed to the watch, so what it changed was not counted; and files a rewind put back were not counted as changed. | Every run is handed to the watch, and one outside the workspace teaches nothing; a rewind that restores files counts as a change. | `observe::tests::a_run_outside_the_workspace_counts_as_a_change_and_teaches_nothing`, `knowledge::sys_a_test_that_passes_after_an_undo_put_a_file_back_has_not_flipped` |
| 7 | **Two tests asserted "nothing was stored" while the store was still being written beside the turn,** so the breaks they hold could pass by a race. | The work started beside a turn is kept track of, and `project/list` waits for it. | every test that lists after a turn |
| 8 | **A program on a drive (`C:/tools/jest.cmd`) got a check the store refuses,** so the command was never learned and the user was told at every session. | No check for it. | `learn::a_program_in_the_workspace_gets_a_check_that_it_exists` |

**From its notes,** fixed:

- **A turn's start could wait for the store's lock** (to mark an expired entry), and a process holding the lock and stopped would have hung it past any interrupt. The mark is made beside the turn (`knowledge::an_entry_that_expires_between_two_turns_drops_out_and_is_told`, which is also the core's test of the "expired" notice the reviewer found missing).
- **Two sentences of the docs were not true of the code:** a run "blocked by the sandbox" is a failure like any other (the sentence is gone; the two runs of a flip must have the same confinement), and a command that left a process running was read as a result (it is not, now).
- **The step 24 line of the build plan named a table of error codes that did not exist:** 14-adaptation's row for the two operations lists them. "Both or neither" is `invalid_params`, as `usage/report` says it.
- **Two tests named `sys_` started no process:** renamed.
- **A promote tells the sessions of its write after it,** so a confined command whose two looks at the protected paths span the write takes it for a change from outside and stops its turn. Telling them before as well was tried and taken out again by the last read: it does not close the window, and a promote that is then refused would leave the file marked as the harness's own. Left as [an item](../../outstanding/a-promote-while-a-confined-command-runs.md).
- **Errors in a notice went out unescaped,** and what a session says once was not bounded: through `visible`, and 256 things at most.

**Written down, not changed:** the watch forgets the failed command with the smallest name first, not the oldest, when it holds 64; a store that is full is read at every observed run of a command it does not hold; `at()` says only that a path "cannot be a workspace", whatever was wrong with it; the measurements of the observer go into `performance-budgets.md` with the third pull request.

### The last read

The second pull request's last round: a reviewer read the fixes above alone.

| # | Finding | Fix | Held by |
|---|---|---|---|
| 1 | **A test read the "expired" notice without waiting for it.** The mark is made beside the turn now, so its notice may still be in the client's queue when the turn has ended. The stress run met it once in fifty. | The test waits for the notice, and so does the one that reads "Learned:". | the two tests, under stress |
| 2 | **The stricter table still classed lines that do something else,** one of them new with the fix: `make SHELL=./deploy.sh test` (an assignment was passed over), `make --eval=…`, `python deploy.py -m pytest`, `zig build deploy`, `dotnet build -t:Deploy`, `cmake --build . -- install`, `go test -exec=./deploy.sh`, `npm run test:publish`. | Make-like lines pass over a short list of options and refuse everything else; `-m` is the first word; `zig build` is classed by its steps; an option that names something to run refuses the line; a script's variant after `:` is one of a few words. A few options that take no value may stand before a sub-command again (`cargo --locked test`, `pnpm -r test`). | `observe::tests::the_table_classes_the_common_builds_tests_and_checks`, with the reviewer's lines |
| 3 | **Sentences and comments that the fixes had made untrue:** the look before a command "drops every rule"; the read "takes no lock" (it tries one, without waiting, for the good copy); "holds up no turn" (a `remember` call waits for the lock). | Corrected, in the code, the breaks and 14-adaptation. | by reading |
| 4 | **A rewind's count of changed files could be skipped,** when the client that asked went away or the restore failed half way. | Counted before the files are touched. | `knowledge::sys_a_test_that_passes_after_an_undo_put_a_file_back_has_not_flipped` (the ordinary case; the two others by reading) |
| 5 | **Telling the sessions before a promote's write did not close its window,** and could hide a change from outside after a refused promote. | Taken out again; the window is an item. | [the item](../../outstanding/a-promote-while-a-confined-command-runs.md) |

**From its notes:** the flag that keeps one mark of expired entries at a time is reset however the mark ends, and if it never starts; a directory with a `:` is not observed (the store refuses one that could name a drive). **Written down, not changed:** a `took` that carries an older snapshot can still put older rules in force for the moment between a command's look and its decision, and the next look corrects it; a guard against it was written and left out, being logic on the trust path that no reviewer would have read. Two `project/list` at once: the second may answer before the first one's wait is over.

**No further round follows:** these last fixes were not read again by a reviewer. They are the table (stricter only, with its rows), a test's wait, comments, and two calls moved.

**The tests' cost on Windows.** The new system tests started about 34 shell commands, and a shell's start on a hosted Windows runner is a third of a second: the A/B (`try` 43863357445, the median of five runs on one runner) put tier 3 at 58.2 s against `main`'s 46.5 s, and on a slower runner that is the tier's budget. So a command that only has to ask is refused where it was allowed (no process starts), and the call that waits for an answer while a test acts is refused too: about 18 commands are left.

## The third pull request: the commands, the budgets and the records

- **`cortex project`** is in [14-adaptation.md](../../architecture/14-adaptation.md), "As built in step 17". It names a project by the working directory, as `cortex permissions` does, and makes its changes through the core with the operation's own function, so the checks are the operation's.
- **`cortex project` with nothing after it lists.** The other subcommands print their help when given nothing; this one's listing is what one most often wants, and it changes nothing.
- **`--json` prints one line,** as `cortex usage --json` does: `project/list`'s result as the protocol writes it.
- **The budget rows** are in [performance-budgets.md](../../performance-budgets.md): the load, the look for a change, section 5's rendering, what the observer adds to a `shell` call, a change to the store, and what a project's knowledge holds in memory (by design, not measured). The load at the caps is written as the recommendation that is with the owner, and marked pending.
- **Start-up is untouched:** `knowledge::the_knowledge_is_not_touched_until_the_session_is_open`, with `cortex_testkit::Untouched`.

## Measured

On the desktop, a release build, `perf_` tests of `cortex-knowledge` (`crates/cortex-knowledge/tests/it/measure.rs`), medians of 200 runs or more. The hosted runners' figures are in [ci.md](ci.md).

| What | Proposed budget | Typical store (40 private and 10 shared entries, 18 KB) | At the caps (500 and 200 entries, 247 KB) |
|---|---|---|---|
| Load both files, parse and validate | ≤ 1 ms typical, ≤ 5 ms at the caps | 0.20 ms | 2.65 ms |
| The look for a change, nothing changed | ≤ 50 µs | 0.9 µs | 0.9 µs |
| Render section 5 at 1,500 tokens | ≤ 100 µs typical, ≤ 1 ms at the caps | 24 µs | 0.30 ms |
| One change: the lock, the read, the write flushed to the disk, the good copy, the read back | none: off the turn's path | 15 ms | 20 ms |

- **The cap on a file's size stands at 256 KiB:** a store of 255 KB of the longest entries the format takes (243 of them) loads in 0.77 ms, and the 500-entry store above is the slowest a valid file can be.
- **A change costs a flush to the disk,** as the statistics' aggregates do: it is made on the blocking pool, a few times a session.
