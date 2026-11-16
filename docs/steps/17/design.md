# Step 17: the design, before building

Written before the code, read by an independent reviewer, and sent to the coordinator. What changed in the building is in [answered.md](answered.md).

The specification is [architecture/14-adaptation.md](../../architecture/14-adaptation.md), "Project knowledge: profile and memory", with the build plan's step 17 and the "What later steps inherit" lines that name it. Each point below says what is built, and where it departs from those pages.

**A choice marked "to confirm by measurement" hangs on a number.** It is measured in the first pull request, before anything is built on it, and the result is recorded in `answered.md`.

## Questions for the owner

Three of the choices below change what a feature does. Each has a recommendation, and the code is built so that the other answer is one predicate away.

| # | Question | Recommendation |
|---|---|---|
| Q1 | **Does text from the repository's shared file reach the prompt before this user accepted it?** 14-adaptation says the shared file "needs no trust to be read: like instruction files, it is text for the model". The step's brief lists "text from a repository's shared file … reaching the prompt before the user accepted it" as a mistake to prevent. | **No: a shared entry is `proposed` for each user until that user accepts it.** The model can write `.cortex/profile.toml` (after the approval a protected path asks for), so an unreviewed shared file is a way round `remember`'s review; a cloned repository's file is a stranger's text; and section 5 sits in the system prompt under the harness's own heading. Acceptance is recorded in the private store against the entry's content hash, so a shared entry that changes is proposed again. 14-adaptation's sentence is changed to say so. |
| Q2 | **Is an observed command that became active by itself on the safe list?** The owner's decision makes it *active* after two observations. 14-adaptation and 06 give the safe list to *accepted* commands. | **No.** Active puts it in the prompt and nowhere else. It runs without asking, and is offered to verification (step 19, build-plan line 456; [15-efficiency.md](../../architecture/15-efficiency.md) gives verification the commands "accepted by the user"), only once the user accepts it (`accept`), or wrote it. The notice that says it became active names the command that accepts it and the one that undoes it. |
| Q3 | **The session log gains one field:** `profile` in the `epoch` entry, the hash of section 5 as the epoch's prompt holds it, beside `instructions`. Step 15's rule is "the session log format beyond agreed additions". | **Agree the addition.** Without it a resumed session cannot tell what the model was given of the profile except by cutting it out of the stored prompt's text. |

## 1. The two files

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

## 2. The command observer

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

## 3. Section 5

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

## 4. The learned safe list

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

## 5. The protocol and the commands

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

## 6. Budgets

Rows proposed for [performance-budgets.md](../../performance-budgets.md), "Data paths". **Each is to confirm by measurement** (release build, the desktop, then hosted runners with `try`'s `perf`), on a typical store (40 entries) and one at the caps.

| Budget | Proposed |
|---|---|
| Load both files, parse and validate | ≤ 1 ms typical, ≤ 5 ms at the caps. On the blocking pool, at the first turn's start and after a change. Never at start-up. |
| The look for a change at a turn's start, nothing changed | ≤ 50 µs: two `stat` calls. |
| Render section 5 | ≤ 100 µs typical, ≤ 1 ms at the caps. |
| The observer, per `shell` call, on the call's path | ≤ 20 µs: make the line the same, class it, hand it over. Nothing for any other tool. |
| The store in memory | ≤ 100 KB typical, ≤ 1.5 MB at the caps, one copy a project a process. |

**The start-up path stays untouched.** Making a session reads neither file; the first read is at the first turn's start. A test in `cortex-core` holds it with `cortex_testkit::Untouched`, as the index's does. No existing budget is loosened.

## 7. The cut into three pull requests

| PR | Branch | Holds |
|---|---|---|
| 1 | `worktree-step-17-store` | `cortex-knowledge`: entries, the two file formats, strict reading and the caps, the store (load, the lock, the good copy, operations, expiry), rendering section 5 as text, the protocol's additions with schema and golden values, the load and render measurements. And the three small seams that are testable alone (the reviewer's cut): `cortex-policy`'s two additions, the `epoch` entry's `profile`, and section 5 in `prompt::build` with its place outside `base`. |
| 2 | `worktree-step-17-wiring` | The observer; `remember` on the store; the profile source, the change notice and its replay rule in `cortex-agent`; the core's wiring (`project/*`, `CommandProfile`, the learned rules, the notices, the settings); the start-up and other-project tests. |
| 3 | `worktree-step-17` | `cortex project`; the remaining measurements; the records and the architecture pages. |

## Settings

| Key | Default | A project's file |
|---|---|---|
| `project.learn` | `true` | The user's alone: no class of setting says "a project may turn it off and not on", and one key does not earn a class. |
| `project.profile_tokens` | `1500`, from 0 to 8,000 | May lower it. |

## Left to later steps

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

## After the independent read

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
