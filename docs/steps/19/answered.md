# Answered in step 19, on 2026-11-16

Step 19 builds verification: when the model ends a turn that changed files, the loop runs the project's checks in the sandbox, with time limits, feeds a failure back within a limit of rounds, retries a known flaky test once, and reports the result. This record holds the design as it was reviewed before it was built, the coordinator's answers, and what changed in the building.

The specification is the build plan's step 19 and the "What later steps inherit" lines that name it (under steps 7, 10, 11, 13, 16 and 17), [architecture/15-efficiency.md](../../architecture/15-efficiency.md), "Automatic verification", and the `Verifying` row of [architecture/03-agent-loop.md](../../architecture/03-agent-loop.md). The owner settled the round limit of 2, a time limit a round (120 s) and a turn, `verify.mode` (`auto`, `ask`, `off`), "off until a check is known", one retry of a known flaky test, and end of turn only (conflict C7, 2026-11-12). Step 17 settled that only commands the user accepted, or wrote, run as checks ([steps/17/answered.md](../17/answered.md), Q2).

## The questions, and the answers

The note went to the coordinator with seven questions, each built as recommended meanwhile. The coordinator answered on 2026-11-16; what it sends on to the owner is marked.

| # | Question | The answer, 2026-11-16 |
|---|---|---|
| Q1 | **`verify.commands` the user's alone,** a repository proposing its checks in its shared profile (accepted by hash), in place of a project's list held for trust and pinned. A departure from 08-configuration's row and from the coordinator's first recommendation. | **Accepted as built, reversible** (the coordinator): it narrows, and the profile's per-user acceptance by hash is the pin 08 asked for. 08's row is changed. **To the owner, as a departure.** |
| Q2 | **The user's own check runs unconfined where nothing confines commands,** in a project the user trusts, where the engine would ask. | **Built as recommended** (the coordinator), **to the owner as a security trade-off**: said plainly in [security.md](../../security.md), section 1, and the user is told once a session when the checks run outside any sandbox. |
| Q3 | **A round is one run:** `rounds = 2` is run, feed back, run again. | **Agreed.** |
| Q4 | **`verify.turn_time` of 240 s,** two rounds' worth. | **Agreed;** in the owner's summary as a default the coordinator set. |
| Q5 | **The last round's failure is not recorded for the model,** as 15-efficiency says. | **Agreed.** |
| Q6 | **`incomplete`,** a new value of `turn.completed`'s `verification`. | **Agreed.** |
| Q7 | **`Preparing → Verifying`** for a turn that only verifies (`verify/run`), which changes a state the step's "Must not change" names. | **Not yet: the owner's to allow.** The second pull request writes up the alternative, `verify/run` calling the verifier from the core outside a turn (its own cancel, its result as a `verification` notice), with the cost of each, and builds that alternative unless the owner allows the transition; if it turns out unsound, the coordinator is told before either is built. |
| | **The "lower only" class of `verify.mode`.** | **Fine** (the coordinator): lowering runs fewer commands, and the user is told. |

## The design, before building

Written before the code, read by an independent reviewer (its findings are under "After the independent read", and are taken up here), and sent to the coordinator. **Reversible** marks a choice built as a recommendation that one change undoes. **To confirm by measurement** marks one that hangs on a number; what was measured is under "Measured", below.

### 1. Which commands run, and who chooses them

A **check** is a command line and the directory it runs in. A check runs a repository's own code (its test scripts, its build files) at the end of every turn, and nobody is asked. So:

**Nothing runs unless the user trusts the project** (`cortex permissions trust`, the trust question, or `--trust-project` for a run), whatever chose the check. In a project that is not trusted, verification says once a session that it waits for trust, and how to give it. **And nothing runs at the `read-only` level**, where the engine asks about every command (`cargo xtask decide --level read-only -- cargo test`: ASK).

In a trusted project, checks come from two places, in this order; the same line in the same directory runs once.

| Source | What | When it runs |
|---|---|---|
| **The user's own `verify.commands`** | The user's file, a profile the user's file defines, the managed file, the environment, the command line. Run in the workspace root. | The engine is asked, and a `deny` or a stop wins. **An `ask` does not hold it back:** the user listed the command, in a project the user trusts. Where the sandbox does not confine (Windows, a kernel without Landlock, `sandbox.enabled = false`), it then runs unconfined, as the user's own listed command. **Reversible:** requiring the sandbox as well would make checks on Windows need an allow rule. |
| **Accepted profile entries** | `Snapshot::runs_unasked(now)` of purpose `test` and `check`, in the profile's order: in use, and accepted or written by the user, a shared entry only as accepted at its present hash. Never an entry that became active by observation alone. | **Only where the engine allows it unasked**, which for most commands is its learned exact rule (step 17's safe list). An entry whose line cannot be an exact rule asks, and is not run. |

- **A project's `verify.commands` is not a source: the key is the user's alone.** A project's value is ignored with a warning, which says that a repository proposes its checks as command entries of `.cortex/profile.toml`. Each user accepts each of those by its content hash, so a changed entry waits for the user again: that is the pin [08-configuration.md](../../architecture/08-configuration.md) asks of "verification commands" ("applied only after the user trusts the project, and pinned by hash"). Trust pins only the rules today, and at `autonomous` the engine allows any command, so a list held back for trust and then applied would run a line changed after the trust (by a pull) without anyone seeing it. **A departure** from 08's row and from the coordinator's recommendation (hold until trust); 08 is changed to say so. **Reversible:** the alternative pins the project's list as a definition of its own in the trust store, which is the trust code the step 23 fixes are changing.
- **Each check is planned and decided just before it runs, and so is its retry.** Its directory is resolved then, after the checks before it ran: one of them may have made it a link. And the profile's snapshot is looked at again just before an entry runs (two `stat` calls when nothing changed): an entry no longer in `runs_unasked` (turned off in another process, lapsed) is dropped, whatever the engine would say of its line at the level in force.
- **Verification never puts a question.** A check that may not run is `not_run`, with what would let it run (accept the entry, trust the project, a rule).
- **What runs, runs through the session's own `shell` tool**, as the core put it together: the same `Runner` (`assemble::runner`), the same launcher (the sandbox), the same environment (the providers' key variables and `shell.env_remove` removed), the look for what it changed, the look at protected paths before and after it (a check that changes one stops the turn, as a command does), and the round's checkpoint (an undo of the turn puts back what the checks changed). Never with `sandbox: false`. At `unrestricted` nothing is confined, as for any command. **A check has no more access than the model's own command has at that level.**
- **The command observer is told of a check as the harness's run**, not the model's: it confirms an entry it already holds (its expiry and usual time move, so an accepted command does not lapse while it runs every turn) and counts towards a flip, and it never learns a new command from it. Step 17 learns from the model's and the user's commands.
- **A check's output keeps the lines that matter:** the `CommandProfile` the core gives the shell tool says yes to a check, whatever the table classes.

**No network.** A check runs with the session's network setting: `sandbox.network`, `off` unless the user's file, or a trusted project's, says `full`. Where the sandbox enforces (Linux), a build or a test run that fetches dependencies fails, as the model's own command would. When the launcher says the sandbox blocked the command (the line it adds to the result, which the shell tool will also put in its payload, as `sandbox`), the check is **blocked**: not a failure of the model's work, so it is not fed back. The user is told which check, the launcher's line, and the two remedies: fetch once outside the sandbox (in their own terminal, or by the model's `shell` call with `sandbox: false`, which asks), or `sandbox.network = "full"`. Windows confines nothing, so nothing is blocked there.

### 2. When a turn has changed files

What a tool round's call changed (the loop's `written`, which `VerifyCx::written` carries) is worked out from the call's result, by a function of `cortex-tools`, which owns the payloads:

| Call | What it changed |
|---|---|
| `write`, `edit` | Its path, when it succeeded and its payload's `kind` is not `unchanged`. |
| `shell` | **When the command ran** (it succeeded or failed; not when it was denied or never started): the files its payload's `changes` name, created, modified or deleted, whether git or the modification times found them (the second walk honours ignore files, so a build's ignored output is not among them). **A look that did not finish** (`complete: false`, or `method: none`) counts as a change of the workspace: what the command did is not known. Its declared `write` of the workspace root does not count: today every successful command, `cargo test` included, sends the turn to verification by it. A command the engine found only reads is not looked at, and changed nothing. |
| A tool from an extension (step 28) | The paths of its `write` effects, when it succeeded. |
| A tool from an MCP server (step 30) | Nothing: it declares no effect a change could be judged by, until step 30 gives it one. |

**A round runs only when files changed since the last round.** After a failure the model may end its turn having changed nothing (it cannot fix the failure, or says why): the checks are not run again on the same state, and the turn completes with the last round's verdict.

### 3. The stage in the loop

Its place is the one 03-agent-loop gives it: `Deciding → Verifying` when the model ended its turn with no tool call, files changed since the last round, rounds are left, and the turn has time left; `Verifying → Finishing` (passed, skipped, the round limit, a stop), `→ Preparing` (failed, with the notice) or `→ Interrupted`. **This pull request adds and removes no transition.** It changes, outside `Verifying` itself, the guard of `Deciding → Verifying` (changed since the last round, time left) and what `round.finish` counts as written (point 2).

| Point | Choice |
|---|---|
| **A round** | One run of the chosen checks. **`verify.rounds` (2) is the most runs one turn gets**, as step 7 built it: run, feed back, run again, and a second failure ends the turn `failed`. The model is told once to fix a failure. (The other reading, two feedbacks and three runs, costs a third round's time; it is one comparison.) **Reversible.** |
| **Every check of a round runs**, in order, whatever an earlier one said, until a check stops the turn: the model is told all that fails at once, which can save it a round, and a failed build makes a test run fail fast anyway. **Reversible.** |
| **What the model is told of a failure** (as built: [the first pull request](#as-built-the-first-pull-request) and the reviews after it changed the share and what is blocked) | One notice (`NoticeSource::Verification`, sent with the harness's lead-in, [decision 0034](../../decisions/0034-notices-in-the-users-role.md)). A first line: the project's checks failed after its changes, which round of how many, and that they run again when it ends its turn having changed files. Then each check that failed: its line, its directory, how it ended (exit code, signal, "stopped at its limit of 2m"), and its output, shaped by the lines that matter (`output::shape` with `Keep::Important`) in an equal share of 16 KiB, at least 2 KiB each. Then one line each for the checks that passed, were flaky, blocked, timed out, skipped or not run. |
| **What a check prints cannot pass for the harness or the user** | Its output is a block whose every line begins with a fixed mark, `| `, after `cortex_base::visible` (tabs kept): no line of it can begin with the notice's lead-in, a heading or anything else of a message's own, and a line break inside one is an escape. The block is introduced as the command's output. The command lines and directories pass `visible` too. A test feeds hostile output (the lead-in, a heading, a fence, control and unseen characters, a carriage return) and reads the request. [security.md](../../security.md) gains a row: tool output is never a user message, and this is the one place a command's output reaches the model in a notice. |
| **The last round's failure** | Reported to the user (the notice, `turn.completed`) and not recorded for the model, as 15-efficiency says ("the turn completes and reports the failing state"). Recording it, so the next turn starts knowing what still fails, is an option put to the coordinator; it would change when failures reach the model, which step 42 inherits. |
| **`turn.completed`** | `verification` is the turn's verdict, carried whenever a round ran to its end, however the turn then ended but interrupted (a turn that ran out of time after a failed round still says `failed`): **`failed`** while any check's last result in the turn is a failure (a check that failed in an earlier round and did not run to a result since included); **`passed`** when every chosen check passed or was flaky; **`incomplete`**, a new value, when none failed and some did not run to a result (blocked, timed out and not counted as a failure, skipped, not run); **`skipped`** when nothing ran to a result. Absent when no round ran. |
| **What front ends see** | A new kind of notice, `verification` (`VerificationNotice`, with its `message` in words as every notice has): when a round starts (its number, the limit, the checks it will run) and when it ends (each check's line, directory, source, outcome, exit code, duration, note, and the artifact of its full output; and the verdict). An outcome is `passed`, `failed`, `timed_out`, `flaky`, `blocked`, `skipped` or `not_run`. In `ask` mode, one that offers the checks. All additive (an open union, open enums, optional fields): no new `PROTOCOL_VERSION`. A running check's output is not streamed: a check is not an item to stream into. |
| **An interrupt** | The verifier is given the turn's two tokens (`VerifyCx::cancel`, and a new `kill` for the second interrupt), and **the loop waits for it** instead of dropping it, as a tool round waits for its calls. The runner stops the running check in stages, or at once on the second interrupt, with its whole tree; its look at protected paths and its checkpoint are still taken. Then `Interrupted`. A round appends nothing until it ends, so the history is what it was after the last tool round: valid. |
| **A check that stops the turn** | One that changed a protected path, or left a process that kept forking, stops the turn through the policy, as a command does; so does a change from outside that the look before a check finds. The checks after it do not run. The verifier says so (`VerifyOutcome::Stopped`); the reason is recorded as a notice for the model (a stopped tool round tells it in the call's result), and the turn ends `denied`. |
| **A result that is not a run** | Refused before it ran (the look before it found a change from outside, or the turn was already stopped): `Stopped`, above. Not started (the program is missing, the launcher refused): `not_run`, with the reason. Neither is a failure, and neither is fed back. |
| **A steer, a queued prompt** | The checks run to their end; only an interrupt cuts them short. A steer that arrives meanwhile is delivered in `Preparing` with the failure's notice, or, when the checks pass, starts a new turn, as a steer that found no boundary does today. A queued prompt waits for the turn to complete. |
| **Its time** | `Spend::Verification`, which the loop already enters. |

### 4. Time limits, flaky tests, hangs

| Point | Choice |
|---|---|
| **The limits** | `verify.round_time` (120 s) and `verify.turn_time` (**240 s**: two rounds' worth; the owner set the round's figure and not the turn's. **Reversible.**), and the turn's and the session's own time limits (`[limits.turn] time`, `[limits.session] time`), which are otherwise checked only before a request: the loop passes the time those leave (`VerifyCx::deadline`), and does not enter `Verifying` when none is left (the turn completes with the verdict it has). Each check runs with the least of what is left of them, and at most `shell.max_timeout`. |
| **How far past a limit** | A check stopped at its limit is stopped in stages: a polite interrupt, 2 s, terminate, 2 s, kill, and at most 5 s for the kill to be seen. Then the look at protected paths and the checkpoint. So a limit can be passed by about 9 s and those looks, once a round: the checks after it are not started. |
| **A check skipped by its time** | Before it starts. When the profile knows how long it usually takes (the entry's `usual_ms`; for a configured command, that of an entry with the same line and directory) and that is longer than the round's limit, it is `skipped`, "usually takes longer than the limit" (15-efficiency's rule). When less is left than its usual time, or than 10 s when its usual time is not known, it is `skipped`, "not enough of the round's time left". Neither is a failure. |
| **A check that runs past its limit** | Stopped by the runner, with its whole tree. It is a failure fed back to the model **only when the profile knows it usually finishes in less than half the time it had**: then the change most likely made it hang. Otherwise it is `timed_out`: told to the user, not fed back, and the verdict is `incomplete` (a cold build, a check that started with little time left, a limit a project lowered: none of them the model's doing). |
| **A known flaky test** | A check that fails, and whose line and directory are a `flaky` entry in use (`Snapshot::in_use(now)` of `Kind::Flaky`, by its `command.run`), runs once more at once, planned and decided again, in the time left. A flaky entry needs no acceptance: it runs nothing, it only earns a retry. Passing then, the check is `flaky`, not a failure, and the user is told ("failed, then passed when run again; it is known to be flaky: k_7"). Failing again, it is a failure, never "flaky". A check that timed out is not retried. Which test of the command flipped is not read from its output: an item in `docs/outstanding/`, as step 17 left it. |
| **What a check leaves running** | Killed by the runner, and said with the check, whose exit code still decides. A tree that kept forking stops the turn (above). |
| **How many** | At most 8 checks a round, in the order of point 1; the rest are said once a session. **Reversible.** |

### 5. The settings: `[verify]`

| Key | Default | Range | Class | Takes effect |
|---|---|---|---|---|
| `mode` | `auto` | `auto`, `ask`, `off` | **Lower only:** a project's value applies at or below the value in effect, in the order `off < ask < auto`, and above it is ignored with a warning. A new value rule, as `ByLevel` is for the level. When a project's value turns verification down, the user is told once a session. | at once |
| `commands` | `[]` | each one line, at most 400 characters, no control character | **User only**, joined: every layer of the user's adds to the list (point 1). | at once |
| `rounds` | `2` | `1..=5` | Tighten only | at once |
| `round_time` | `"2m"` | `10s..=1h` | Tighten only | at once |
| `turn_time` | `"4m"` | `10s..=4h` | Tighten only | at once |

- **`mode = "ask"`:** the stage runs nothing. A turn that changed files, with checks to run, ends `skipped`, with a `verification` notice that offers them; `verify/run` (point 6) runs them.
- **`mode = "off"`, or no check known:** nothing runs and nothing is said.
- **"At once" is honest only with a reload.** The settings are read when a session is put together. The verifier holds them where a reload can replace them, and the loop's `verify_rounds` likewise: step 24's `config/reload` applies them to a running session, as it applies the limits. Its item ([outstanding/config-reload-through-the-core.md](../../outstanding/config-reload-through-the-core.md)) gains `[verify]`.

### 6. `verify/run` and `/verify`

- **This step** builds `verify/run` in the core and the protocol (11-rpc-protocol's table: "Run the project's checks now"; the build plan's scope for step 19 names it with `/verify`), and the headless result's `verification` (the verdict and the last round's checks).
- **`verify/run` on an idle session starts a turn that only verifies:** one round of the checks chosen as above (trust, the level, the engine), whether or not files changed and whatever `verify.mode` says (the user asked), and no request to the model. It records nothing in the history, so it writes nothing before an epoch exists. It answers `{}`, as `session/compact` does: the turn's id arrives with `turn.started`, and the v1 golden transcript's answer stands. While a turn runs it answers `turn_running`; with nothing to run, `no_checks`, saying why (no check known, not trusted, `read-only`). The clients that may send a prompt may send it.
- **It needs one new transition, `Preparing → Verifying`, for that turn alone,** as `session/compact`'s turn uses `Preparing → Compacting`. That changes `Preparing`, which the step's "Must not change" names: **raised here, for the coordinator's agreement**, in the second pull request.
- **Step 24** adds the `/verify` command (in the application's `NOT_YET` today, build-plan line 521) and the panel's display of the `verification` notices (line 529).

### 7. The cut into two pull requests

| PR | Branch | Holds |
|---|---|---|
| 1 | `worktree-step-19-stage` | `[verify]` in `cortex-config` (with the rule for `mode`); `VerificationNotice` and `Verification::Incomplete` in `cortex-protocol`, with the schema and golden values; in `cortex-tools`, what a call changed and the shell payload's `sandbox`; in `cortex-agent`, the stage (a round only after a change, the deadline, waiting for the verifier on an interrupt, `Stopped`, the notices, the verdict); in `cortex-core`, the verifier (trust, choosing, the engine's answer, running through the session's shell as the harness's run, the flaky retry, the time limits, the report) and its wiring; this note; 03, 08, 15 and security.md where the stage is as built; the breaks. |
| 2 | `worktree-step-19` | `verify/run` and the turn that only verifies, headless's report, the measurements, `ci.md`, the build plan's lines for steps 24, 42 and 43, and the outstanding items. |

### Budgets

Verification's own time has no budget ([performance-budgets.md](../../performance-budgets.md)); the harness's share of it does. Rows proposed, **each to confirm by measurement** (a release build at the desk, then a hosted runner with `try`'s `perf`):

| What | Proposed |
|---|---|
| The stage when no check runs (a turn that changed files, mode `auto`, no check known), from `Deciding` to `Finishing` | ≤ 50 µs |
| Choosing a round's checks: the profile's snapshot from memory, 8 checks, the engine asked of each | ≤ 1 ms |
| A turn that changed no file | Nothing measurable: one comparison |
| What the harness adds to each check | Measured and stated, not budgeted here: the two looks at protected paths (about 24 ms each in a tree this size, which [the cost of the look at protected paths](../../outstanding/the-cost-of-the-look-at-protected-paths.md) already marks for step 33) and the look for what it changed, as for any command |

**Start-up is untouched:** nothing of verification is read or started when a session opens.

### Left to later steps

| What | Whose |
|---|---|
| Affected-test selection: 15-efficiency's "simple form", a command template that takes changed paths | Step 52, with the per-ecosystem selection: no entry can hold a template, and `VerifyCx::written` is the hand-off. 15-efficiency is changed to say so. |
| A running check's output, live | Step 24, if its panel wants it. |
| `/verify` and the panel | Step 24. |
| A head start for the checks | Step 42, with the stage's contract above unchanged. |
| Running a memory entry's `check` | Step 43. It needs a policy of its own (no writes, no network), which the session's shell tool does not give: a launcher made from a narrower `Policy`, as step 28's hooks have. |
| Hooks around a check | Step 28: a check is not a tool call, so a hook on the shell tool does not see it. Said there. |

## After the independent read of the design

A reviewer that had not seen the reasoning read the first draft of this note against the specification and the code. Its findings, each taken up above:

| # | Finding | Change |
|---|---|---|
| A1 | The user's own checks ran a cloned repository's code unasked, unconfined where there is no sandbox. | Nothing runs in a project the user has not trusted. |
| A2 | A project's commands were not pinned by hash: changed after the trust, they ran unseen at `autonomous`. | `verify.commands` is the user's alone; a repository proposes checks in its shared profile, accepted by hash. |
| A3 | A check's output in a user-role notice could begin a line with the notice's lead-in or a heading. | Every output line behind a fixed mark; a hostile-output test; a security.md row. |
| A4 | The turn's and the session's time limits were not held while checks ran, and the verdict was lost when the turn then ended `limit`. | `VerifyCx::deadline`; no round without time; the overshoot stated; the verdict carried whatever the end. |
| A5 | Checks decided at a round's start ran on stale decisions: a directory made a link, an entry turned off elsewhere. | Each check planned and decided just before it runs; the entry looked up again. |
| A6 | At `autonomous` an accepted entry ran in a project that is not trusted. | Trust is required of every check. |
| B1 | A command that failed, a look by modification times, and a look that did not finish did not count as changes. | Point 2. |
| B2 | A refusal, a stop and a launch failure would have been reported as failures. | "A result that is not a run". |
| B3 | A timeout would be fed back as the model's fault after a cold build or a lowered limit. | Fed back only against a known usual time. |
| B4 | The skip compared with the time left, not the limit, as 15-efficiency says. | Both rules, each named. |
| B5 | The verdict could say `skipped` or `passed` over a known failure or a check that never ran. | The verdict's rules, and `incomplete`. |
| B6 | The observer would learn new commands from the harness's own runs. | It confirms and counts flips only. |
| B7 | `verify/run`'s answer is `{}` in the v1 golden transcript; a notice needs its `message`. | `{}` kept; `message` added. |
| B8 | "At once" was not true of settings read when a session is put together. | The reload seam, and the reload item. |
| Notes | The last failure recorded beyond the spec; three changes outside `Verifying` to name; a stop the model is not told of; `verify/run`'s mode and clients; a project's `off` said nowhere; hooks and step 43's policy; which test flipped; a trusted project's network; each check's looks. | Each in its point above. |

## As built: the first pull request

The stage, the settings, choosing and running, on `worktree-step-19-stage`. Where the build departed from the note, and what it settled:

| Point | As built |
|---|---|
| **`turn.completed` with no round** | **`skipped`, as before,** for a turn that completed with no round: the note said "absent", but a completed turn has said `skipped` since step 7, and the v1 golden transcript holds it. The verdict is carried whenever a round ran, however the turn then ended (the note's rule); absent only for an interrupted turn, or another end with no round. |
| **What a call changed** | `cortex_tools::changed_by`, in `cortex-tools`, which owns the payloads it reads; the loop's round calls it. A command that was interrupted counts what its look found too: it ran. |
| **The time rules** | Two functions of their own, `time_left` and `too_long`, so that each limit is tested alone. The deadline the loop passes is `Ledger::time_left`: the least of what the accounts' wall-clock limits leave, the turn's own included. |
| **The verdict across rounds** | The verifier keeps, for the turn, the checks that failed and have not passed since: a round in which a check that failed earlier did not run to a result (skipped for its time, say) is still `failed`, and its report says so. |
| **A refusal** | The engine's answer is turned into one refusal or none, in one place: an `ask` lets the user's own line through; anything else that is not an allow, an answer this build does not know included, refuses. |
| **The harness's run** | `RecordingShell::run_by` takes who ran the command (`By::Model`, `By::Harness`), and the verifier hands `cortex_tools::execute` an `AsCheck` of the shell, so a check's result passes the same output policy as a call's. `Knowledge::observed_check` confirms an entry of the private store that is in use (`confirms`) and makes none. The breaks of steps 17 and 26 that named `RecordingShell::run` and `Knowledge::observed` follow the bodies to `run_by` and `observe`. |
| **The lines that matter** | The shell is given a `CommandProfile` (`ChecksProfile`) that says yes to the user's own check lines as well as to what the profile knows, so what the model is told of a long failure is cut from what the shell kept of the lines that report a problem. |
| **The settings** | Read when a session is put together. `Trust::ByVerifyMode` is the "lower only" rule. A line of `verify.commands` that is empty, longer than 400 characters or holds a control character is not run, and said once a session: the configuration's lists take any text, so the verifier checks it. |
| **`TrustClass::StopsCommands`** (added by the step 23 fixes while this was built, for settings held until trust) | **Not used:** no `[verify]` key waits for trust. `mode` may only go down, the limits only tighten, and `commands` is the user's; and a check runs only in a project the user trusts, so a project's lowered limits reach only a project the user has already trusted. |

**Windows.** The core's system tests that start a shell there are three: a failure fed back and fixed (two runs), a time limit, and a flaky test's retry (two runs). The rest, the same code on both systems, run on Linux; the loop's rules run on both, with a verifier a test writes and no process.

### After the review of the first pull request

An independent reviewer read the whole diff against the specification, without the author's reasoning. Its findings, each with the fix and the test that holds it (the breaks are in `mutations/step-19.breaks`, "after the review of the first pull request"):

| # | Finding | Fix | Held by |
|---|---|---|---|
| a1 | **A user's check ran at `read-only` when the level was lowered while its round ran:** the level was looked at once, at the round's start, and an `ask` let the user's line through. | The level is looked at again before each check. | `verify::sys_a_check_after_the_level_was_lowered_mid_round_does_not_run` |
| b1 | **A real failure was classed blocked and never fed back:** the sandbox's guesses match ordinary output (`error[E0433]: failed to resolve`, `dial tcp`, `connection refused`). | Only the launcher's own refusal is blocked; the other lines leave the result, and are said beside it with the remedy. | `verify::tests::only_the_launchers_refusal_is_blocked_and_a_compiler_error_is_a_failure` (through the sandbox's real `explain`) |
| b2 | **The flaky retry threw away the first failure** when the retry was skipped or refused, and retried a test that hung. | A test stopped at its limit is not retried; a retry that did not run to a result leaves the first failure, with a note. | `verify::tests::a_flaky_test_is_run_again_only_after_a_failure_that_was_not_a_hang`, `a_retry_that_did_not_run_to_a_result_leaves_the_first_failure` |
| b3 | **A stop hid a failure from the same round:** the verdict was `incomplete`. | `VerifyOutcome::Stopped` carries whether a check of the turn still fails. | `verify::tests::a_rounds_verdict_says_what_ran_to_a_result`; `cortex-agent`'s `verifying::a_stop_after_a_failure_in_its_round_ends_the_turn_failed` |
| b4 | **A round that ran nothing was fed back as a failure** while an earlier one stood, and one started with the turn's time for checks spent. | `VerifyOutcome::StillFailing`: the turn ends `failed`, with no notice and no request; no check starts once the turn's time for checks is spent. The verdict is a function of its own, `Memory::judge`. | `verify::tests::a_failure_stands_until_its_check_passes_and_is_forgotten_with_its_turn`; `verifying::a_round_with_nothing_new_ends_the_turn_failed_and_tells_the_model_nothing`; `verify::sys_once_the_turns_time_for_checks_is_spent_a_failure_stands_and_nothing_runs` |
| b5 | **The docs promised what the code did not say:** the remedy for a blocked check, a project's `ask`, a process that outlived its tree. | Each is said; the docs follow the code. | `verify::tests::a_process_left_outside_the_checks_tree_is_said`; the remedy in the b1 test |
| b6 | **The time a check was said to have, and the hang rule, used a limit the shell clamps** (`shell.max_timeout`). | `time_left` holds a check to `shell.max_timeout`. | `verify::tests::a_check_has_the_least_of_what_the_round_the_turn_and_the_limits_leave` |
| b7 | **A panic in planning was reported as an interrupt.** | A panic is `not_run`, with its message; only a cancel is an interrupt. | by reading |
| c1 | **The interrupt test did not look for the check's processes.** | It reads the shell's and its child's numbers and finds neither running. | `verify::sys_an_interrupt_during_a_check_stops_it_and_the_next_prompt_is_answered` (Linux) |
| c6 | **The report's bound was before escaping:** an unseen character is eight bytes escaped. | The share binds the escaped lines; what is past it is counted. | `verify::tests::the_report_is_bounded_after_escaping_too` |

**From the run of the breaks,** before the review: `an-entry-turned-off-still-runs` was caught by no test, since the engine refuses the entry too (its learned rule goes with it); the test now asserts why the check did not run. Four breaks of `touched.rs` name tests in `cortex-agent` too, and say so (`packages`).

**Written down, not changed:**

- **A managed `ask` rule** does not hold back the user's own check line: an `ask` from any layer lets the user's own line through. (Changed by the owner's decision of 2026-11-16: a managed `ask` rule holds the user's own line back, as a managed `deny` does; built by the tenth tooling step.) The managed file is the machine's administrator's; a `deny` there still refuses it. Whether a managed `ask` should hold it back is the owner's choice, and an item (`a-managed-ask-and-the-users-own-check`, settled on 2026-11-16) of its own, with a recommendation.
- **Trust is read when the session is put together,** as for the engine's rules: trust taken away from a running session stops its checks at its next session.
- **The look after a check that stopped the turn** has no break: its test (`verify::sys_a_check_that_changes_a_protected_path_stops_the_turn_and_undo_puts_it_back`) returns early where the sandbox does not enforce, which the desk's machine does not; CI's Linux runner does.
- **`lowered_by_project`** is true whenever a project's file gave the value; the notice says that the project's configuration sets it, which is so whatever the user's own file says.

### The second read

The same pull request's second and last round: a reviewer read the fixes above alone. Its findings, each fixed with a test and a break (`mutations/step-19.breaks`, "after the second read"):

| # | Finding | Fix | Held by |
|---|---|---|---|
| 1 | **The level was read before the check was planned,** so a level lowered while the plan and the engine's decision were made on the blocking pool let a user's line run at `read-only`. | It is read after the decision, beside it: the same window an allowed call of the model's has. | `verify::sys_a_check_after_the_level_was_lowered_mid_round_does_not_run` (Linux) |
| 2 | **The bound after escaping cut the end of what a check printed**, which the lines that matter keep on purpose (the summary, the exit line); and with many failures a share fell below the least the output policy shapes to. | `printed_block` shapes again, with fewer lines, until the marked and escaped text fits, and cuts (from the start) only output whose every line is long escaped. At most three failed checks' output is shown, in an equal share of 16 KiB; more are named with their artifact. | `verify::tests::the_report_is_bounded_after_escaping_too` (the error and the end stay) |
| 3 | **A stop overwrote the result of the check that made it,** so a check that failed and changed a protected path hid its failure, and a flaky retry that met a stop dropped the first failure. | A run keeps its result, and the stop goes beside it (`Ran::stop`); `judge` counts the result; the retry carries the stop. `outcome_of`, a function of its own, says whether a check still fails with the stop. | `verify::tests::a_check_that_failed_and_stopped_the_turn_still_fails`, `a_stop_says_whether_a_check_still_fails`, `a_stop_the_retry_met_goes_with_the_result` |
| 4 | **A skip by `shell.max_timeout` blamed the round's time.** | `too_long` names `shell.max_timeout` when that is the limit. | `verify::tests::a_check_is_not_started_when_it_usually_takes_longer_than_it_has` |
| 5 | **The remedy for a check that needs the network went with the launcher's own refusal,** where it does not apply. | The launcher's line alone for a refusal; the remedy with a guess. | `verify::tests::only_the_launchers_refusal_is_blocked_and_a_compiler_error_is_a_failure` |
| 6 | **The launcher's refusal is read from what the command printed:** a script that prints the launcher's mark and exits 127 is taken for blocked, and not fed back. | Outside this step: the sandbox's to give its refusal on a channel the command cannot write. | [an item](../../outstanding/a-launcher-refusal-read-from-the-commands-output.md) |
| 7, 8 | Tests for the mapping of a stop and for the retry's stop; the module's notes; the message for a round with nothing new. | Each added or corrected. | as above |

**Written down, not changed:** the look after a check whose stop overwrote its result has no break of its own (its test needs an enforcing sandbox, which the desk's machine has not).

## As built: the second pull request

Headless's report, `verify/run`, the measurements on the hosted runners and the records, on `worktree-step-19`.

| Point | As built |
|---|---|
| **Headless's result** | `HeadlessResult::verification` (`HeadlessVerification`): the turn's verdict as `turn.completed` gave it, how many rounds ended, and the last round's checks. Absent for a turn with no verdict. The exit code does not follow it ([architecture/10-headless.md](../../architecture/10-headless.md), "`json`"). |
| **What later steps inherit** | In the build plan, step 19's "What later steps inherit": the notices and the verdict for step 24, the stage's contract for step 42, the harness's run and its policy for step 43. |
| **A managed `ask`** | An item for the owner, `a-managed-ask-and-the-users-own-check`, with the recommendation that it holds the user's line back. The owner chose that on 2026-11-16, and the tenth tooling step built it ([tooling-10/parts/decisions.md](../tooling-10/parts/decisions.md)). |

### `verify/run`: the two ways

`verify/run` runs the project's checks now: one round, chosen as at the end of a turn (trust, the level, the engine), whatever `verify.mode` says, with no request to the model. It is `/verify`'s (step 24). Two ways to build it, for the owner (Q7):

- **A. A turn that only verifies** (recommended). `verify/run` on an idle session starts a turn, as `session/compact` does for a turn that only compacts, which goes `Idle → Preparing → Verifying → Finishing → Idle`. The new edge, `Preparing → Verifying`, is the one change to the table, and it changes `Preparing`, which the step's "Must not change" names. Everything a turn has, the checks then have:
  - the session is busy, so nothing of the model's runs beside a check;
  - an interrupt reaches them, and waits for the check's processes;
  - a check that changes a protected path stops the turn, which ends `denied`;
  - what a check changed is that turn's checkpoint, for undo;
  - `turn.completed` carries the verdict, the one event every client and headless wait for.
- **B. The core calls the verifier outside any turn,** with a cancel token of its own, and its result as a notice. The loop is untouched, but:
  1. the session is not busy: a prompt can start a turn beside a running check, and the look at protected paths of each reads the other's writes as a change made from outside, which stops the model's turn;
  2. a check that changes a protected path has no turn to stop, and the next command does not see the change;
  3. what a check changed lands in the next turn's checkpoint;
  4. an interrupt does not reach it without a route of its own;
  5. no `turn.completed` ends it.

  Each can be patched, but together the patches rebuild a turn outside the state machine, without its tests. As asked, B is not sound (points 1 and 2).

**Built: A,** as one commit on top of the second pull request, on the branch `worktree-step-19-verify-turn`. The owner had not answered by the time the rest was ready, so the pull request shipped without it (`verify/run` answers `not_available`, and the choice is filed for step 24); this commit, merged on the owner's yes, settles both.

| Point | As built |
|---|---|
| **The turn** | `SessionHandle::verify` starts it on an idle session, as `compact` does; `Prompt::verify` marks it, and the loop goes from `Preparing` to `Verifying` at once (`TRANSITIONS`, and the scenario `only_verifying`). It records nothing: no epoch, no notice, no prompt, so a session with no prompt yet writes no log. |
| **One round** | `VerifyCx::asked` is set and `rounds` is 1: a failure ends the turn `completed` with the verdict `failed`, and goes to no model, as at the round limit (Q5). A stop ends it `denied`, told to the user and recorded for no model. The verifier does not read `verify.mode`, and tells the user each reason nothing ran, every time. |
| **While a turn runs** | `turn_running`, to be sent again once the turn has ended (with `verify.mode = "auto"` its end runs the checks when it changed files). Nothing is queued for later, which would run checks the user may no longer want. |
| **Its limits, its statistics, undo** | A time limit that is spent ends it `limit` before a check starts, with the limit's notice told and not recorded (`Ledger::out_of_time`). It records no `TurnStat`: it is not the model's turn, and the statistics of the model's turns are the quality signal of a set-up ([14-adaptation.md](../../architecture/14-adaptation.md)). What its checks changed is its checkpoint, so an undo right after it puts that back; with nothing changed, an undo takes the turn before. |
| **Nothing could run** | The core asks the verifier first (`CoreVerifier::why_none`, the same `ready` a round uses: none known, not trusted, `read-only`, no shell) and answers `no_checks` with the reason; no turn starts. Each check is still decided as it runs. `no_checks` is a new error code, an addition to the protocol. |

**The review of the second pull request** (independent, the whole diff) found nothing against the bar. Its findings, each fixed with a test and a break, or written down:

| # | Finding | Fix | Held by |
|---|---|---|---|
| 1 | **A spent time limit:** the turn that only verifies started, told the user "did not run again: this turn's time for them is spent" (the session's limit, not `verify.turn_time`), once a session, and ended `skipped`. | It ends `limit` before any check, as a turn ends before its request; the round's message names the turn's or the session's limit when that is what is spent. | `verifying::a_turn_that_only_verifies_with_its_time_spent_ends_at_the_limit_and_runs_nothing` (the turn's limit and the session's); the round's message, by reading (it is reached only in the moment between the two looks) |
| 2 | **`turn_running` promised checks** that `off` and `ask` do not run at a turn's end. | It says to send it again once the turn has ended, and when `auto` runs them. | by reading |
| 3 | **Each `verify/run` was counted as the model's turn** in the statistics, with the model the last turn had. | No `TurnStat` for it. | `verifying::a_turn_that_only_verifies_is_not_counted_as_the_models_turn` |
| 4, 5 | **`ask` mode under `verify/run` was not tested, nor anything of it on Windows.** | The core's test runs `off` and `ask` on Linux, and `ask` on Windows (one shell start); a project's lowered mode is told as at a turn's end. | `verify::sys_verify_run_runs_the_checks_in_a_turn_of_their_own_and_sends_the_model_nothing` |
| 6 | **Docs:** the picture's arrows, the `Verifying` row, `turn_running`'s meaning, `verify/run` in 15-efficiency, and the `no_checks` texts. | Each corrected. | |
| 7 | **What waits behind the turn, and its log,** were not tested. | A steer, a queued prompt and a notice follow it; a session with no prompt has no log after it. | `verifying::what_waits_behind_a_turn_that_only_verifies_follows_it`; the core's test above (`World::logs`) |
| 8 | **Undo after it** takes back what its checks changed, or the turn before when they changed nothing. | As designed: said in 03-agent-loop. | by reading |
| 9 | A commit that changed the protocol without its generated files. | Squashed. | |

**The second read** (the fixes alone) found nothing against the bar and no change to another turn's behaviour. It found the test of finding 1 held only the turn's own limit, not the session's (a test case and a break added); the project's lowered mode, told under `verify/run`, held by nothing (the core's test now sets `off` in the project's file); and five places in the docs, a comment and a break's rule that lagged the fixes (each corrected).

Written down, not changed: `verify/run` asks the verifier before it looks for a running turn, so a running session with nothing to run answers `no_checks`; its answer is `{}`, and the turn's id comes with `turn.started`, as for `session/compact` (an optional `turn_id` in the answer would be an addition, for step 24 to ask for); `TRANSITIONS` allows `Preparing → Verifying` for any turn, so the table's check in debug builds does not tell a turn that only verifies from another; the reason told when a check is ready at `verify/run` and not at its round (the race between the two) has no test.

Held by `transitions::a_turn_that_only_verifies_goes_from_preparing_to_its_checks`, `verifying::a_turn_that_only_verifies_runs_one_round_and_sends_and_records_nothing`, `verify_is_refused_while_a_turn_runs`, `an_interrupted_turn_that_only_verifies_says_nothing_of_the_checks`, `a_check_that_stops_a_turn_that_only_verifies_tells_the_user_and_not_the_model`, and the core's `verify::verify_run_says_why_no_check_could_run_and_starts_no_turn` and `sys_verify_run_runs_the_checks_in_a_turn_of_their_own_and_sends_the_model_nothing`; the breaks are in `mutations/step-19.breaks`, "a turn that only verifies".

## Measured

A release build, medians: on the desktop, and on the hosted runners by `try`'s `perf` (`perf_the_verification_stage_when_no_check_runs` on the branch, run 43910965367; the core's two on `main` after the first pull request, runs 43904710553 and 43904705426). The rows are in [performance-budgets.md](../../performance-budgets.md), "Latency the harness adds".

| What | Proposed budget | Desktop | Hosted Linux | Hosted Windows |
|---|---|---|---|---|
| The stage when no check runs, from `Deciding` to `Finishing` | ≤ 50 µs | About 30 µs | About 25 µs | About 37 µs |
| … the loop's share | | 0.49 µs | 0.73 µs | 1.1 µs |
| … the hop to the blocking pool for the look at the project's files | | 23 µs | 13.9 µs | 1 µs |
| Choosing a round's checks: the profile's look (nothing changed) and choosing, 40 entries | ≤ 1 ms | 6.4 µs | 9.9 µs | 34.6 µs |
| A turn that changed no file | Nothing measurable | One comparison: the stage is not entered | | |
| What the harness adds to each check | Stated, not budgeted | A command's own: its start (≤ 5 ms on Linux, as step 11 measured), the look for what it changed, and the looks at protected paths (step 26's, marked over what is wanted) | | |

**The second pull request's tiers on Windows** (`try` 43910959613, the branch against `main` on one runner, the median of 5 runs each, 9 ms a process start): tier 1 18.0 s against 19.0 s, tier 2 40.1 s against 40.2 s, tier 3 69.9 s against 70.9 s. Nothing moved beyond the runner's noise; tier 2 is at its budget on both sides on a runner this slow. One test failed once on `main`'s side only (`trust::sys_a_project_trusted_in_the_store_is_trusted_while_its_rules_are_the_ones_the_user_saw`, round 1), and was left out of both.
