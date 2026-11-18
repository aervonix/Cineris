# Answered in step 23, on 2026-11-15

Step 23 builds the interactive application: `cineris` with no `-p` opens the terminal UI on a real session. The owner's answers came through the coordinator.

| Question | The owner's answer, 2026-11-15 | Recorded in |
|---|---|---|
| **Where the design note lives.** `cargo xtask rules` allows only `answered.md` and `ci.md` in a step's folder. | **The rule stays.** The note is a working file until the end, left out of every pull request, and is then folded into this file as "The design, before building (reviewed)", with its review's findings. | This record |
| **The ops channel from the interface to the core thread.** 02-runtime had it bounded at 64, with the front end waiting for room; the interface must never wait on the core thread. | **Unbounded**, with 02-runtime changed in the same change: every operation is a person's key. | [architecture/02-runtime.md](../../architecture/02-runtime.md) |
| **The core's changes** (the design's point 3): `Core::respond` taking question and trust answers; trust asked when a session of an untrusted project opens; the rewind's question; `Esc` and `Ctrl+C` in a dialog, with `answers_may_stop` for the interface; `session/undo`, `workspace/files` and `SessionState::write_roots`; `session/list` by `updated_at`, the compact-only turn, `config/reload`; and `cineris-term`'s changes (the escape in `sanitize`, the signal hand-off, SIGABRT, the alternate screen, OSC 52 for `/copy`). | **All approved, with the schema diff,** and one change to trust: **three answers**, not two. `Trust it`; `Not now`, as designed (not asked again in this process, nothing stored); and **`Never for this project`**, stored as untrusted in the same private store as the trust pins, so that later starts do not ask either. `cineris permissions trust` and its listing show a project marked never, and can trust it later; an untrust or forget form is added if there is none. The never state shows on the status line or in `/status`, with the command that changes it. Tests: a never survives a restart, and `permissions trust` clears it. | [architecture/06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md), "Project trust"; [architecture/11-rpc-protocol.md](../../architecture/11-rpc-protocol.md) |
| **The rewind's question.** 07 says a restore does not overwrite a file edited since "unless the user says so", and nothing asked. | **Approved as designed:** an undo and a rewind ask before overwriting files edited since, and no answer keeps them. | [architecture/07-sessions-and-context.md](../../architecture/07-sessions-and-context.md), "Snapshots and undo" |
| **Glyphs of ambiguous width** (the open item `ambiguous-width-glyphs`, closed here). | **Left as they are,** with `tui.glyphs = "ascii"` documented for a terminal that draws them wide. The item is closed. | [configuration/reference.md](../../configuration/reference.md), `tui.glyphs` |
| **Highlighting the line in progress** (the open item `highlighting-the-line-in-progress`, closed here). | **Plain until it ends,** as built. The item is closed. | [decision 0015](../../decisions/0015-syntax-highlighting.md) |
| **Trusting the rules of a profile a project defines** (the open item `trust-of-a-profile-a-project-defines`, closed here). | **One pin of every rule the project's files hold,** in their base and in every profile they define, each shown with the profile it is in, compared the same way whichever profile is selected. | [architecture/06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md), "Project trust" |
| **Changing a file over the undo limit** (the open item `files-over-the-undo-limit`, closed here). | **The refusal stays,** and the item is closed: asking would let `edit` and `write` hold a file of any size in memory, and 16 MiB is their memory bound too. | [architecture/05-tools.md](../../architecture/05-tools.md), "Checkpoints" |
| **The zero-width joiner and non-joiner** in drawn text, which the new escape in `sanitize` could have written as escapes. | **Kept in drawn text:** they join emoji sequences and shape Persian and the Indic scripts. A line is added to [names-that-escape-to-the-same-text](../../outstanding/names-that-escape-to-the-same-text.md): two names that differ only by a joiner or a non-joiner look alike. | [architecture/09-tui.md](../../architecture/09-tui.md), "Sanitising" |
| **A summary running past its turn** ([outstanding item](../../outstanding/a-background-summary-past-its-turn-cannot-be-interrupted.md)). The interface offers no interrupt between turns, and step 27's "Must not change" is the core's behaviour. | **Moved to step 32,** with the reason in the item. | The item |
| **A project's time limit of nothing** (the open item `a-projects-time-limit-of-nothing`, closed here; answered 2026-11-16, and added to the second pull request). A project's first limit where there was none applied without trust, so an untrusted repository's `[limits.turn] time = "0s"`, `requests = 1` or `cost = 0` stopped every turn at once. | **A first limit applies only after trust.** A value at or below a limit already in effect still applies always. A limit where there was none waits for trust, held back as any setting that needs trust is, so the session runs without it until then; it is listed with its key and value in the trust question and in `cineris permissions trust`. It holds for every tighten-only key with no default: `requests`, `tokens`, `cost` and `time` under `[limits.turn]`, `[limits.session]` and `[limits.day]`, the twelve keys that change (every other tighten-only key has a default, so there is always a limit in effect). The class is `first limit` (`TrustClass::FirstLimit`). | [architecture/08-configuration.md](../../architecture/08-configuration.md), "Project trust"; [configuration/reference.md](../../configuration/reference.md) |
| **What a diff's emphasis covers** (asked 2026-11-16 with the second pull request, whose reference screen 2a had moved the bold four columns and whose views bolded the changed middle, `, clippy`, where 09-tui says "changed words are bold"). | **Whole changed words only:** `clippy`, not `, clippy`. A word is a run of letters, digits and `_`; the punctuation and spaces around a changed word stay plain; a change of punctuation alone still shows, its character bold. The 2a references are drawn again from the views. | [architecture/09-tui.md](../../architecture/09-tui.md), "Diff"; [architecture/tui-look/README.md](../../architecture/tui-look/README.md) |
| **`config/reload`, and the end of the interface outside the core's lane, in the second pull request** (asked 2026-11-16 under rule 11: the pull request was already as large as the first). | **`config/reload` moves to step 24,** which owns `/reload` and hot-reload end to end; its item stays open, naming step 24 and the four rules this design settled. **The end outside the lane stays here,** because a test showed quitting waited on it: with the trust question open at the start or on a resumed session, or the undo's question open, `Ctrl+C` twice, SIGTERM and SIGHUP each took 5 s (the binary's wait for the core) before the program ended. With the lane on a task of its own, and the interface disconnected first, which withdraws what it was asked, each takes a few milliseconds. `/quit` cannot be given while a dialog has the keys. | [build-plan.md](../../build-plan.md), step 24; outstanding/config-reload-through-the-core.md (the open item `config-reload-through-the-core`, closed in step 24); [architecture/09-tui.md](../../architecture/09-tui.md), "The application as built (step 23)" |

## Open items settled

Beside the owner's items above, which the table closes:

| Item | Settled | Recorded in |
|---|---|---|
| `the-input-box-while-a-turn-runs` | **Closed:** the empty box says `Type to steer this turn` while a turn runs, and holds only its `◆` while a box or a panel has the focus (screens 1b to 1d, and 4). | [architecture/09-tui.md](../../architecture/09-tui.md), "The application as built (step 23)" |
| `names-a-client-shows-as-they-are` | **Closed:** `cineris-term`'s `sanitize`, through which every view draws text from outside, writes every character that acts unseen and is not a control as an escape, keeping the two joiners. Approval details, the rules an answer would add, the undo and assembly notices and errors are all drawn through it; data stays exact. | [architecture/09-tui.md](../../architecture/09-tui.md), "Sanitising" |
| `a-caught-panic-leaves-the-terminal` | **Closed:** the terminal layer's panic hook puts the terminal back only for a panic on the thread that entered it. A system test panics on another thread, catches it, and the demo goes on drawing in raw mode. | [architecture/09-tui.md](../../architecture/09-tui.md), "The application as built (step 23)" |
| `signals-and-a-graceful-shutdown` | **Closed** (the second pull request): the terminal layer hands SIGTERM, SIGHUP, SIGINT and SIGQUIT (on Windows `Ctrl+Break`, `Ctrl+C` as an event, and the console's close, log-off and shutdown) to the application, which stops the turn at once, ends the core within 1.5 s (after the history's last prompts, at most 1 s, so that the whole end fits the watchdog) and exits 143, 129, 130 or 131; a watchdog ends the process 3 s after the signal; a second signal ends it at once; SIGABRT is not handed over, and ends the process by its default action | [architecture/09-tui.md](../../architecture/09-tui.md), "The terminal layer as built (step 20)" and "The application as built (step 23)" |
| `continue-chooses-the-newest-session` | **Closed:** `session/list` gives the session added to last first (by `updated_at`, the id breaking a tie), across projects; `--continue` and `/sessions` take that order | [architecture/11-rpc-protocol.md](../../architecture/11-rpc-protocol.md); `docs/protocol/` |
| `session-compact-after-a-turns-last-request` | **Closed:** a compaction asked for during a turn's last request runs in a turn of its own when that turn ends and nothing follows it | [architecture/07-sessions-and-context.md](../../architecture/07-sessions-and-context.md); `docs/protocol/` |
| `file-completion-on-a-large-workspace` | **Closed:** the `@` match runs on a thread of its own at any size, off the key's path; rows for a query that is no longer the latest are dropped | [architecture/09-tui.md](../../architecture/09-tui.md), "The application as built (step 23)"; [performance-budgets.md](../../performance-budgets.md) |
| `trust-of-a-profile-a-project-defines` | **Closed** with the owner's answer above: one pin of every rule, base and every profile, each shown with its profile | [architecture/06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md) |
| `terminal-test-gaps` | **Its half done:** the real application's end under each of SIGTERM, SIGHUP, SIGINT and SIGQUIT, and while a question holds the core's lane, on Linux; a panic on another thread, in `cineris-term`. **Left, written into the item:** the Windows console closing (the test kit cannot close a pseudo-console under a running child), `/quit` under a dialog, and the real binary's own panic (no way to make it panic that is not a hook in the product) | The item |
| `key-latency-over-conpty` | **Measured** with the real application on a hosted Windows runner, beside the harness's own share on Linux, the streamed piece's time among them; the decision stays the owner's | The item; [performance-budgets.md](../../performance-budgets.md) |
| `log-level-from-configuration-in-headless-mode` | **Its interactive half done;** the headless half is left, and the item names step 32 | The item |
| `reading-a-key-at-a-terminal` | **Left** (it was "if the time allows"): moved to step 25's first-run flow, which asks for the key | The item |
| `the-default-editor-on-windows-11`, the folder half | **Not built:** a folder the editor still holds is removed once and, failing that, left with nothing said. The item names step 32 for it, with the test it needs | The item |

## The design, before building (reviewed)

The six points the step's prompt asked to settle before building widely, each with the choice made, what it changes outside `cineris-tui` and `cineris-cli`, and where it departs from a page of `docs/`. Written against `main` at `d050503` as a note of its own, revised after its independent review (the findings are at the end, with what each changed), and folded in here before the first pull request. The owner's answers to it are the table above; where they changed it (trust's third answer), the table wins.

### 1. Where the application lives

**In `cineris-tui`, as a module `app`,** with the loop, the state and the mapping of events, talking to the core only in `cineris-protocol`'s types through a small trait. `cineris-cli` makes the core, its thread and runtime, the terminal and the history store, and hands them in. `cineris-tui` keeps its three dependencies (`cineris-protocol`, `cineris-base`, `cineris-term`); no edge is added to [01-workspace.md](../../architecture/01-workspace.md).

```rust
// cineris_tui::app
pub trait Link: Send {
    /// Queues an operation. Never waits: its result comes back from `next`.
    fn send(&mut self, op: Op) -> Ticket;
    /// Answers a request the core put to this client. Never waits.
    fn respond(&mut self, request: RequestId, response: ServerResponse);
    /// What has arrived, oldest first. Never waits.
    fn next(&mut self) -> Option<Inbound>;
}

pub enum Inbound {
    Event(SessionEvent),
    Request(RequestId, ServerRequest),
    Done(Ticket, Result<OpResult, ErrorData>),
    /// The core is gone (its thread ended, or the connection closed).
    Gone(ErrorData),
}
```

**What crosses the boundary:** `Op` and `OpResult` (or `ErrorData`), `SessionEvent`, `RequestId` with `ServerRequest` and `ServerResponse`. Nothing else: no `Core`, no `Outgoing`, no configuration type. The link is given the terminal's `Waker` and calls it when something arrives. **What stays on the client's side,** because it is the person's and not the session's: the `[tui]`, `[keys]` and `[terminal]` settings (read by the binary and handed in), the prompt history (`HistoryStore`, step 21), the external editor, the clipboard.

**The in-process link** is `cineris-cli`'s (`interactive.rs`): a channel of operations to the core thread, where one task calls `Core::handle` for each in order; results back through a small queue; `Core::respond` called on the interface's thread, since it takes a lock and sends on a channel and never waits on a disk; events read straight from the client's `EventQueue` with `try_next`, so that its merging and its `item.resync` keep working for a client that is behind (an open external editor while a turn streams). Step 27's `attach` is a second implementation of `Link` over the RPC connection, and drives the same application.

**Order of operations.** Every operation that changes the session (`turn/start`, `turn/interrupt`, `session/set`, `session/compact`, `session/rewind`, `session/undo`, `session/new`, `session/open`, `session/close`) runs in one lane, in the order sent: an interrupt cannot overtake the prompt it is meant to stop. Reads (`session/list`, `session/read`, `models/list`, `config/get`, `permissions/list`, `workspace/files`) run on tasks of their own beside it. `shutdown` runs outside the lane, so a lane stuck behind an open cannot hold the end of the program.

**Departure from [02-runtime.md](../../architecture/02-runtime.md):** the ops channel from the interface to the core thread is **unbounded** ("bounded MPSC (64); the front end waits" there). The interface must never wait on the core thread, and every operation is caused by a person's key, so the queue is bounded by typing; a bounded channel would only move the queue into the interface. 02-runtime's table is changed in the same change.

**Changes this needs outside the two crates:** `cineris-core`'s `EventQueue` calls a hook when it goes from empty to not empty and when it closes (the link's waker), one call a burst; and `CoreOptions` takes the user's configuration the binary already loaded, so that it is not read twice.

### 2. The threads and the loop

| Thread | Owns | Wakes the interface by |
|---|---|---|
| **Main: the interface** | `Terminal`, `Inline`, `Scheduler`, `Input`, `Transcript`, the dialogs, the app's state | (blocks in `Terminal::next(timeout)`) |
| **Core** | a current-thread Tokio runtime, timer and I/O on, and the `Core` | the link's waker: an event queue that stops being empty, an op's result |
| The core's blocking pool | session reads, rewinds, the file listing for `@`, trust and approval files | (through the core thread) |
| Highlighter (step 22) | the grammars and each block's state | its waker, which is the terminal's |
| File filter (new, only past 20,000 paths) | the `@` candidates of a large workspace | the terminal's waker, with the query's generation |
| History (new, client side) | the project's history file | the terminal's waker, once loaded |
| Log writer (`cineris-base`) | the log file | nothing |

**One wake of the loop:**
1. Take every terminal event already read (keys, pastes, resizes), each through `Input` or the open dialog.
2. **If a key or a paste arrived, draw its frame now,** before anything from the core is looked at.
3. Then the inbox, in its order (requests are queued with the events, and are taken in turn): at most 1 ms of it, looking for terminal input every 64 events and going back to step 1 when there is some. What is left stays in the core's queue, where deltas merge, and a stream frame is asked for.
4. Draw if the scheduler says so: input-driven frames at once, stream-driven ones at most 60 a second.

So a key's frame waits for the key's own handling, at most one batch of 64 events, and one frame, never for the core, a disk or the network: the interface thread makes no call that can wait on any of them (ops are queued, `Core::respond` takes a lock held only to change a table, the history is another thread's, the file list is the core's pool's, the highlighter's colours are taken when ready, the file filter's rows a frame later).

**Idle** is no turn running, nothing animating, no highlight or filter outstanding: the scheduler says `Idle`, and the thread blocks in `poll`/`WaitForMultipleObjectsEx` with no time limit. The core thread's runtime is parked with no timer (02-runtime: an idle session has none), and the workers wait on their channels. **No timer is set for "a while":** the hint row's cost after a turn lasts until the next prompt, and the armed quit until the next key (step 21's rule; 02-runtime's "within 1 s: quit" is changed to it). The second interrupt's window (2 s, from 02-runtime's table) is a comparison of two key times, not a timer. **Departure from 09-tui** ("the turn's cost and the session's, for a while"): until the next prompt instead.

**Order on the way in.**
- While the app's `session/new` or `session/open` is outstanding, that session's events are held back in the app (an assembly sends a handful), so that the header (from the result) is the first block and the notices of its assembly come after it. Requests are not held: the trust question arrives while the open is outstanding.
- **A prompt sent before the session is open is held** and sent when the open returns, the session's first; so is a prompt given on the command line (`cineris "fix the build"` at a terminal), which is sent as soon as the session is open.
- **`set_running(true)` on submit,** not on `turn.started`, so that `Esc` straight after `Enter` interrupts the turn it started; it is set back when the prompt is refused or the turn completes.
- **`item.resync`:** the item's deltas stop until it completes; the completed item's text past what arrived is then added, so an answer never loses its middle.
- **A resumed session's history** is read with `session/read` and drawn into the scrollback; a live event with a `seq` at or below the read's `seq` is dropped.

**Dialogs take only keys meant for them.** A dialog that appears while the person is typing (the trust question arrives just after the first frame; an approval while a steering message is being written) must not take that typing as its answer. A dialog takes keys read after its first frame was written and 300 ms more (a comparison of the key's time with the frame's, no timer); earlier keys go on into the editor, where they were going. A paste never answers a dialog. *As built (after PR 1's review):* the answers are drawn only once the grace has passed, the loop woken for that, and a key also needs 300 ms since the key before it, so that typing that carries on through the grace never answers. A first version kept the grace only for a dialog drawn while someone was typing; the review found that weaker than this design, and it was put back.

### 3. The question and trust dialogs

Both are a change to what the core does. **Proposed, not built yet.**

**`Core::respond` takes all three answers.** The open-request table in `clients` holds a `ServerRequest` and a sender of `ServerResponse`; an answer of the wrong kind for the request is refused (`request_not_open`), as an answer to a request that is not open is now. Only a `user` client that was asked may answer, as for approvals.

**A wait for an answer never hangs.** Today an approval whose askers have all gone waits for another to join ([06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md), as built in step 13a). That is right for a session that is running, which another client can open; a session still being put together, or a rewind under way, has nobody who can join. So the trust and rewind questions are raced against the asking client's queue closing and the core's cancel token: either ends the wait as "no answer".

#### Trust

**Asked when a session opens** (`session/new`, `session/open` of a stored session, `session/fork`) and all of these hold:
- the client that opens it declared `trust` (the TUI does; headless does not, and nothing changes for it);
- the project is not trusted, or is trusted with its rules changed since;
- `--trust-project` was not given.

That is [06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md)'s "opening a project for the first time asks one question": every project, with or without a `.cineris/` folder, since trust also lets git read the repository's own configuration (`repository_trusted`: the base state, git's record of what a command changed, `git status` as a read). The request lists what trust turns on, each a `TrustResource`:
- **git**, as a new kind `repository`, with what it allows;
- **each of the project's rules** (`rules`; `changed` when they changed since trust), every rule of its files and of every profile they define (point 6);
- **each setting held back**, as a new kind `settings`: the key, its value, and what it raises or widens.

`repository` and `settings` are new values of an open enumeration that only informs: additions, with a schema diff, and no new protocol version. A project trusted with its rules changed is asked about the rules only.

**What the core does with the answer.**
- **Yes:** it pins **the definition it showed**, not the files as they are when the answer comes (a `git pull` or another session may change `.cineris/config.toml` while the dialog is open), with the same function `cineris permissions trust` uses. The session is then put together trusted, and a change made meanwhile reads as "rules changed", as at any start.
- **No, no answer, or a client that went away:** the session is put together untrusted, as today. The core does not ask again about that project for the rest of its life (a `/new` in the same run is not asked twice). Nothing is written, so the next start asks again.
- The session's assembly is split at that point: the trust look (on the blocking pool), the question (awaited on the core thread, the open waiting on it), then the rest of the assembly, which loads the configuration again only when the answer was yes.

**In the TUI** it is shown in place of the input box: `? Trust this project?`, the folder, then each resource in full, and `y Trust it   n Not now`; `Esc` is not now.

**Tests,** in `cineris-core` (tier 2):
- a user client is asked, and its yes pins and loads trusted;
- a no loads untrusted, and the next `session/new` of the same core does not ask;
- a project whose rules changed is asked about its rules, with `changed`;
- a change to the rules while the question is open is not pinned;
- a headless client is never asked;
- a client that disconnects while asked gets no session, and the core's lane goes on.

In the TUI: the dialog against a scripted link, and one end-to-end run where a project's `.cineris/config.toml` sets a model, the dialog shows it, `y` is pressed, and the header shows the project's model.

#### The question

**Asked by an undo or a rewind when a file was changed since the harness left it.** [07-sessions-and-context.md](../../architecture/07-sessions-and-context.md) says a restore "does not overwrite something the user has edited since unless the user says so", and `cineris-session` already takes that answer (`Restore::overwrite_edited`), but nothing asks it: today such a file is reported as not put back.

- **The ask.** With a client that declared `questions`, the core plans the rewind, and if the plan names files edited since, asks one `question/ask`: "These files were changed after the turn that changed them: `a.rs`, `b.rs`. Put them back as they were before that turn, losing those changes?", with the choices `Put them back` and `Keep them as they are`.
- **The answer.** The first rewinds with `overwrite_edited` set to those names. The second, no answer, or a client that cannot be asked rewinds as today.
- **No turn starts meanwhile:** the history change is held across the question.
- **The test** is the `/undo` end-to-end test's second half: the user edits the file the turn changed, `/undo`, the dialog, `Put them back`, and the file is as it was before the turn.

**Alternative, if the owner does not want the rewind question:** the question dialog is built and tested against a scripted link only, `respond` takes question answers, and its first sender is step 28 (the `tool.before` hook's "ask the user").

#### Esc, Ctrl+C and a question

09-tui says `esc denies`; 02-runtime's table says the first interrupt with an approval pending denies it and stops the turn. Both hold, on two keys:
- **`Esc` answers the dialog:** an approval denied (the call gets an error result and the round goes on), trust not now, a question unanswered.
- **`Ctrl+C` interrupts the turn,** which withdraws the open question (`request.resolved`, `cancelled`).

Since the TUI can stop the turn from a question, the policy must say so ([03-agent-loop.md](../../architecture/03-agent-loop.md), "Approve", names step 23 for it): `Policy::answers_may_stop` becomes true for a core whose front end can stop the turn from a question (a new `CoreOptions` flag that interactive mode sets). With it, no call with side effects that the model wrote after the asked call starts while the question is open; calls that only read still run. That is slower when several calls ask, and never wrong. Its cost is measured in the step.

### 4. The slash commands this step ships

| Ships here | Through |
|---|---|
| `/help` | a block listing the commands and the keys, from `ACTIONS` |
| `/new` | `session/close`, then `session/new` |
| `/sessions` | the picker over `session/list` (resume, switch). Fork is step 25's |
| `/undo` | **`session/undo`**, a new method: `cineris-session`'s `Session::plan_undo`, which steps back before the last turn, its first turn included, records the rewind as an undo (which the corrections-as-evaluations step reads), and puts files back. `session/rewind` takes the last item to keep, so it cannot undo a first turn, and finding the item would read the whole log |
| `/model`, `/effort` | pickers over `models/list` and the model's levels, sending `session/set`; `/effort` has the fast-mode toggle (`Tab`, `tab - fast mode`) only when the model offers a fast speed, and the toggle sends `session/set` with the speed |
| `/mode` | a picker of the levels, `session/set` |
| `/permissions` | `permissions/list` in a picker; a new picker action, `picker.remove` (`Delete`), removes the selected one (`permissions/revoke`) |
| `/compact [focus]` | `session/compact`. Offered by itself (the hint row and an error's next step) where a turn ends with `compaction_needed` |
| `/status` | a block: provider, model, effort, speed, level, sandbox, the limits in force, session id, workspace; MCP and extensions say "none" until their steps |
| `/view [n]` | the viewer |
| `/copy` | the last answer, through the terminal's clipboard sequence (OSC 52), an addition to `cineris-term` |
| `/config` | the effective configuration from `config/get`, without where each value came from (that needs a field the protocol lacks: an item) |
| `/theme` | a picker of the built-in themes (`auto`, `dark`, `light`) for the run |
| `/quit` | quits |

**Owned by later steps, not shipped:** `/context`, `/usage`, `/plan`, `/project`, `/profile`, `/verify`, `/reload` (24); `/rewind` and fork (25); `/extend` (29); `/lease` (after the release).

**Listed in 09-tui with no step, each filed as an open item:** `/mcp` (recommend step 30), `/extensions` (recommend step 28), `/export` (recommend step 32), and `/config`'s provenance.

**The command line in interactive mode.** `--resume`, `--continue`, `--fork`, `--model`, `--effort`, `--speed`, `--permission-mode`, `--profile`, `-c`, `--allow`, `--deny`, `--trust-project`, `--system-append`, `--log-level` and a prompt given as words are honoured as headless honours them; `--no-session` opens an ephemeral session. The headless-only flags (`--output`, `--include-deltas`, `--fail-on-denial`, `--quiet`) are refused with a usage error, exit 2, and a sentence saying they are for `-p`.

### 5. The cut into two pull requests

**PR 1, `worktree-step-23-loop`:**
- `cineris_tui::app`: the loop, the `Link` trait, the state; the mapping of every event in 09-tui's table onto the transcript; the hint row from the session's state;
- the input wired: submit, steer, queue, interrupt and the second interrupt, `set_running`, `set_width`, `Ctrl+T` and the live boxes in the focus ring, `Ctrl+L`, `Ctrl+C` and `Ctrl+D`, `global.suspend` to `Terminal::suspend` and `Inline::restart`;
- the approval dialog (with the leave-the-sandbox form), answered through `Core::respond`, closed by `request.resolved`, with the dialog's own keys in a new `dialog` context of `ACTIONS` (and `[keys] dialog`);
- `[tui]`, `[keys]` and `[terminal]` declared in `cineris-config` (done), `KeyMap::with_table` with each `TableError` as a notice, `Look::hyperlinks` from the terminal;
- interactive mode in `cineris-cli`: the log (file only), the core thread, the in-process link, the start-up order below, `--resume` and `--continue` opening a session (without the history drawn: PR 2), the held prompt;
- `cineris-term`: the panic hook (done); `sanitize` escapes what acts unseen (point 6); SIGABRT in the emergency set; a crash report, written by the binary's own panic hook to the log directory with a backtrace, and its one-line pointer printed after the terminal is put back (02-runtime specifies it; nothing builds it yet);
- the core's `answers_may_stop` for the TUI;
- the pseudo-terminal harness for the real binary on both OSes, with phase timings, and the end-to-end tests of a full turn with tools, an approval, an interruption mid-stream and mid-tool, a steer and a queue;
- the first `try` measurements of start to first frame.

**PR 2:** the trust and question dialogs with their core changes; `session/undo`; the other slash commands and pickers; the viewer (`cineris-term` gains the alternate screen, which its emergency restore leaves too, only while it is on); history (with a setter on `Input`, `set_history`, since it is filled after the first frame); `@` files (`workspace/files`) with the filter off the key's path (an `Input` hook to hand the query out and take the rows back); the external editor, with the session's write roots in `Env::avoid` (a new field of the session's state); resume with its history drawn, and undo, end to end; signals and a graceful end; `config/reload`; the remaining golden rows (2a, 2b, 8a, 8b, the placeholders); the budgets measured; the records.

**Start-up order** (15-efficiency's "open the session after the first frame"):
1. Parse the arguments.
2. Read the configuration: the user's layers, the profile and `-c`. `[terminal]`, `[tui]`, `[keys]` and `log.level` come from it, and every key of those is user-only.
3. Start the log.
4. `Core::start`, given that configuration, on the main thread: it opens nothing more than headless does before its first request, and an invalid catalog stops the program before the terminal is touched, exit 2, as headless does.
5. Move the core to its thread.
6. Enter the terminal with the overrides, and draw the first frame: the input box and the hint row.
7. Only then send `session/new` or `session/open`.

An open that fails (`day_limit_needs_stats`, a session locked elsewhere) is an error block with its next step, and the app stays up: `/new` and `/sessions` still work. Unreadable statistics at start (`stats_newer`, a file set aside) are a notice, and the error codes `day_limit_needs_stats`, `stats_newer` and `unknown_value` have sentences of their own.

### 6. The open items naming step 23

| Item | What this step does | Why |
|---|---|---|
| `the-input-box-while-a-turn-runs` | **Closes:** `Type to steer this turn` while a turn runs, nothing while a box or panel has the focus | The input knows both states |
| `names-a-client-shows-as-they-are` | **Closes, in `cineris-term`'s `sanitize`:** besides drawing each control character as a symbol, it writes every other character that acts unseen (`cineris_base::acts_unseen`: the bidirectional controls, the characters of no width, the tag characters, the line and paragraph separators) as `\u{…}`, with `cineris_base::visible`'s rule for variation selectors. Text is escaped once, when it is drawn, and stays exact as data (`adds`, the `@` paths put in a prompt, ids, configuration values). Every view, menu and picker row, the approval's details and the undo and assembly notices already pass `sanitize`. **The zero-width joiner and non-joiner are kept** in what is drawn: they join an emoji sequence and shape Persian and the Indic scripts, and they can neither reorder text nor hide any. **The owner's to confirm** | One place, at the point of drawing; `cineris-term` already depends on `cineris-base` |
| `config-reload-through-the-core` | **Moves to step 24** (the owner's answer above), with what this design settled, which the item now carries: `config/reload` checks the files again (the old configuration stays on error), **judges trust again** (a trusted project that adds an allow rule has its rules set aside, as at a start), applies the keys marked `Now` to each open session (the engine's rules through a new engine as `session/set` makes one, the limits, the retry policy, the log's level, and `stats.retention` and the `[usage]` keys, which step 16 asked for), applies the `NextEpoch` keys when the session's next epoch begins and lists them as `pending` until then, and lists the `Restart` keys as `pending` with a sentence. `/reload` stays step 24's | The item names it; step 24 needs it |
| `continue-chooses-the-newest-session` | **Closes:** `session/list` orders by `updated_at`, newest first (a change to `cineris-core`'s list, as the item says); `--continue` and the picker take that order | One order for both |
| `files-over-the-undo-limit` | **Recommendation: keep the refusal, and close the item with that answer.** Asking would let `edit` and `write` read and match a file of any size in memory (16 MiB is also their memory bound), against the memory budgets; the model is told why, and a shell command's change to such a file is detected and listed as one an undo cannot put back. **The owner's call** | Asking costs more than it gives |
| `file-completion-on-a-large-workspace` | **Closes:** past 20,000 paths the filter runs on a thread of its own; the key's echo is drawn first and the menu's rows a frame later, a result for a query that is no longer the text's dropped | The item's recommendation |
| `signals-and-a-graceful-shutdown` | **Closes,** in PR 2. See below | The item asks for three decisions |
| `log-level-from-configuration-in-headless-mode` | **Closes both halves:** the binary reads `log.level` from the user's configuration before the log starts, in both modes (`--log-level` and `CINERIS_LOG` still rank above it), and hands the same load to `Core::start`, which no longer reads it again | One load for both, which costs headless nothing |
| `reading-a-key-at-a-terminal` | **Closes,** in PR 2 if the time allows: `cineris auth login` at a terminal reads the key with echo off through `cineris-term`, reached through a small function in `cineris-tui` | It needs only the terminal layer |
| `a-background-summary-past-its-turn-cannot-be-interrupted` | **Leaves it, and recommends step 32:** the TUI offers no interrupt between turns (`Esc` and `Ctrl+C` idle are the editor's), and quitting ends the session, which stops the summary. Step 27 cannot take it either: its "Must not change" is the core's behaviour | Neither front end offers an interrupt between turns |
| `session-compact-after-a-turns-last-request` | **Closes:** at the end of a turn with a compaction pending, the loop runs a turn that only compacts, as an idle session's `session/compact` does, and `docs/protocol/` says so | `/compact` is offered during a turn |
| `terminal-test-gaps` | **Does its half:** the real binary under a pseudo-terminal restored after exit, a panic and each of SIGTERM, SIGHUP, SIGINT and SIGQUIT one by one (Linux), and a panic on another thread (done, in `cineris-term`) | The presenter's generated scripts and the stop under a shell with job control stay |
| `ambiguous-width-glyphs` | **Recommendation: leave it,** and document `tui.glyphs = "ascii"` for a terminal set to draw them wide (done in the reference). Most terminals in a CJK locale draw them narrow by default, so choosing ASCII by locale would take the look from the many for the few | **The owner's** |
| `highlighting-the-line-in-progress` | **Recommendation: leave it plain until it ends,** as built: a row that changes colour as it grows (an opening quote) is worse than a plain one, and colouring it costs a round trip to the worker each frame | **The owner's** |
| `trust-of-a-profile-a-project-defines` | **Recommendation: one pin of every rule the project's files hold, in their base and in every profile they define,** compared the same way at every start whichever profile is selected; the trust dialog and `permissions trust` show each rule with the profile it is in. One question, and `--profile review` works after it | **The owner's** |
| `the-default-editor-on-windows-11`, the folder half | **Builds:** a folder that cannot be removed is tried again when the app ends, and one still there is named on standard error after the terminal is left. **Checked by hand:** on Windows, `Ctrl+G` with `code --wait` or an editor whose helper holds the file; quit; the message names the folder | The other half is answered |
| `key-latency-over-conpty` | **Measures** the harness's own share with the real application on a hosted Windows runner, as the item recommends, and records the console's part beside it; the decision stays the owner's | Due before this step |

**Also closed:** `a-caught-panic-leaves-the-terminal` (done: the hook puts the terminal back only for a panic on the thread that entered it, and a system test shows the demo still drawing after a panic caught on another thread).

#### Signals, a failed write and a panic

- **SIGTERM, SIGHUP, SIGINT and SIGQUIT** (Ctrl+Break, the console's close, log-off and shutdown on Windows):
  - the layer puts the terminal back at once, as now, and then hands the signal to the app instead of ending the process: the handler wakes the loop through the waker's pipe;
  - **the watchdog is armed in the handler itself:** `alarm(3)` on Unix, whose default action ends the process; on Windows the handler's thread waits up to 3 s for the app to end, since Windows ends the process when the handler returns from a close, log-off or shutdown event;
  - the app interrupts the turn with `now`, runs `Core::shutdown` (at most 2 s) and exits 143, 129, 130 or 131.
  - While the external editor has the terminal, the interface thread is waiting for it: the watchdog ends the process, and the editor has the signal from its terminal too.
- **SIGABRT** (an allocation that failed, a panic while panicking) joins the emergency set: the terminal is put back, and the default action ends the process.
- **A failed write** after an emergency ends the app the same way.
- **A panic** is judged by the thread it ends, since a hook cannot tell a caught panic from one that is not:
  - the interface's own puts the terminal back (the hook);
  - the core thread's ends the app: its link reports `Gone`, the app leaves the terminal, prints the crash report's one line, and exits 1;
  - the highlighter's turns highlighting off, which step 22 built it to survive;
  - any other thread's is in the log.

### What changes outside `cineris-tui` and `cineris-cli`

| Where | Change | Kind |
|---|---|---|
| Protocol | `workspace/files` (session, limit): the workspace's paths with the ignore rules, newest change first, at most 100,000, listed on the blocking pool by `cineris-tools`' walk; `@` needs them listed "by the core" (step 21), and over RPC the workspace is the server's | New method |
| Protocol | `session/undo` (session, `restore_files`): `Session::plan_undo` | New method |
| Protocol | `TrustResourceKind::Repository` and `Settings` | New values of an open enumeration |
| Protocol | `SessionState::write_roots`: the roots the session's commands may write, for the external editor's `Env::avoid` | New optional field |
| Protocol | `session/list` ordered by `updated_at`; `session/compact` at a turn's last request runs a compact-only turn after it; `session/rewind` and `session/undo` may ask a question | Behaviour, said in `docs/protocol/` |
| `cineris-core` | `Core::respond` takes question and trust answers; the trust question at open; the rewind's question; `config/reload`; `EventQueue`'s wake hook; `CoreOptions` takes the loaded configuration and the TUI's `answers_may_stop` | API additions and behaviour |
| `cineris-agent` | The compact-only turn after a turn whose last request had a compaction pending | Behaviour |
| `cineris-config` | `[tui]`, `[keys]`, `[terminal]` declared (done) | Schema, reference regenerated |
| `cineris-term` | The panic hook's thread (done); `sanitize` escapes what acts unseen; the signal hand-off and its watchdog; SIGABRT; the alternate screen; the clipboard sequence | Public API additions and behaviour, recorded in 09-tui |

### The review of this note

An independent reviewer read the first version against the specification and the code. Its findings, and what each changed above:

| Finding | Changed |
|---|---|
| Keys typed before a dialog appeared could answer it | Dialogs take only keys read after their first frame and 300 ms more; a paste never answers (point 2) |
| A yes pinned the files as they were at the answer, not what was shown | The shown definition is pinned (point 3) |
| Trust was asked only when `.cineris/` held something, which left repository trust unasked against 06 | Every untrusted project is asked, with git listed (point 3) |
| `Esc` and `Ctrl+C` in a dialog, and `answers_may_stop`, were not decided | Decided (point 3) |
| The watchdog's place, Windows' close events, SIGQUIT's code, SIGABRT, and per-thread panics | Decided (point 6, signals) |
| A prompt before the session opened, `Esc` straight after `Enter`, an interrupt overtaking its prompt | Held prompts, running on submit, one ordered lane (points 1 and 2) |
| Kept ops were sent only at the next key | The channel is unbounded (point 1) |
| A false claim that `cineris-term` cannot reach `cineris-base`; escaping at the boundary would damage data, `\r` and joiners | Escaping moves into `sanitize` (point 6) |
| `config/reload` did not judge trust again; `NextEpoch` keys were only listed | Both fixed (point 6) |
| Waits for a trust or rewind answer could hang the lane | Raced against the client's queue and the cancel token; `shutdown` outside the lane (points 1 and 3) |
| `/undo` could not undo a first turn and read the whole log | `session/undo` (point 4) |
| `item.resync` and a resume's duplicate events were not handled | Both handled (point 2) |
| The external editor could not be given the write roots | `SessionState::write_roots` (point 5) |
| Omissions: `/compact` on `compaction_needed`, error wording, the statistics notice, suspend, hyperlinks, `request.resolved`, `Input`'s setters, the remove action, `/status`'s limits, the interactive flags, the crash report, `key-latency-over-conpty`, the quit window, step 27's "Must not change" | Each added where it belongs |
| Load the configuration once; the 4 ms slice was the whole streaming key budget; requests share the events' queue; the alternate screen in an emergency | One load; a 1 ms slice in batches of 64; taken in order; only while it is on |

## What changed in the building (the second pull request)

What the second pull request built differently from the design above, or added, and why. The owner's answers that came during it are in the first table.

| What | As built | Why |
|---|---|---|
| The `@` filter | Designed off the key's path past 20,000 paths; for a while built on the key's path, then built as designed, at every size: a worker thread matches, the input hands out each query with a generation and takes rows of its latest only | At the core's cap of 100,000 a match takes 3.6 ms in a release build at the desk, over a key's 2 ms; a key's frame at the cap is now 0.007 ms at the median and 0.028 ms at the 99th percentile (0.073 and 0.148 ms in a debug build) |
| The end of the interface | `shutdown` outside the lane, as the design had it, built as a lane task of its own, the interface disconnected first (`Core::disconnect`) and the lane's last operation given 1 s | A test showed that with the trust question, or the undo's, open, `Ctrl+C` twice, SIGTERM and SIGHUP each took the binary's 5 s wait before the program ended; now a few milliseconds |
| The viewer | Built to 09's "The viewer": the title and key rows, boxes kept as rounds (`Transcript::commit_parts`) and drawn again, closed with the latest selected, `Up`, `Down`, `Enter`, the page keys, `/` and `n`, `/view n`; and `Ctrl+O` in an approval or the trust question shows the request whole | A scroller over the committed lines had been built first, which the golden screens 8a and 8b showed short of its specification |
| A command's lines | `Block::Listing`: a title, then lines as they are, each cut to the width | A notice joins its lines into one paragraph, which `/help`, `/status` and `/config` cannot use |
| The project's standing | `SessionState.trust`, an informational and open value (untrusted, trusted, rules changed, never, for the run), with the schema diff | `/status` says the standing and the command that changes it, as the owner asked, and a client needs it from the session's state |
| The history file | `history` in the state directory, one JSON line (`project`, `text`) for each prompt; a project's newest 1,000 kept, the file written again past 20,000 lines under its lock, private to the user | 09's "History" leaves the format to the build |
| A dialog's sentences | Wrapped at the prose width, as the transcript's are; commands, paths and diffs at the whole width | Screen 2b wraps its warning at 80 columns |
| The golden screens | The dialogs, the viewer's frame and the input box's placeholder composed from the real views; only screen 4's panels left uncompared. Style alone rewritten from the views in three reference files: in 2a the bold of line 30's change (`clippy`, the word added, by the owner's rule below), which was four columns to its left; in the light 2a and 2b the light theme's bold on the warning colour and the diff's signs, which screen 1's light file has | The emphasis follows the owner's rule of 2026-11-16 (whole changed words); the bold is the light theme's as screen 1 shows it. The pull request lists every cell, before and after, for the owner |
| `config/reload` | Moved to step 24 (the owner's answer above) | Step 24 owns `/reload` and hot-reload end to end |
| The statistics notice | Left to step 24's usage panel (the open item `statistics-a-build-cannot-use-are-said-only-in-the-log`, closed in step 24) | A look at the totals at the start reads the statistics when nobody asked, which step 16's rule forbids and a test holds; the writer finds such a file only when it settles |
| `/config`'s provenance, `/export` | Open items of their own (provenance (the open item `config-provenance-in-the-application`, closed in step 24), [`/export`](../../outstanding/the-export-command-has-no-step.md)) | `config/get` gives values only; no step builds `/export` |

### The review of the second pull request

An independent reviewer read the whole of the second pull request before it was opened, the dialogs and the session's opening first. Its findings, and what each changed:

| Finding | Changed |
|---|---|
| A dialog that came while the viewer was open had its grace run unseen, and a key typed just after the viewer closed could answer it; the loop spun while the viewer stayed open | A dialog is shown by the inline frame alone; a key in the viewer is the key before the next; nothing is due under the viewer. Tested (`a_dialog_that_comes_under_the_viewer_waits_for_the_inline_screen`) |
| `workspace/files` ran in the lane, so a first prompt waited for the walk, which was not bounded | A read, beside the lane; the walk stops once it has met four times its limit |
| With the lane a task of its own a read could overtake a write sent before it (`/permissions`' list after a revoke) | A read sent while a write is under way runs after it, in the lane (`Routes`) |
| Never did not apply to the session it was said in | The trust is worked out again after the refusal |
| A setting the question listed that changes while it is open is used after a yes | As designed: what a yes pins is the rules ([06](../../architecture/06-permissions-and-sandbox.md)); a trusted project's settings may change after it, and one changed while the question is open is no different |
| A prompt sent after another process compacted the history file was lost | Each prompt appended to the file opened afresh, under the store's lock |
| `join` took the clients' lock under the open requests' lock, the other order from `put` and `disconnect` | One order |
| A request that ends with its askers, put when they had gone, never ended | Dropped at once |
| SIGABRT was handed over with the four signals that end a program | Left to its default action; a system test sends one |
| A question's choices were drawn without `sanitize` | Drawn through it |
| The viewer's transcript is kept for the life of the application, unmeasured against the 200-turn memory budget | Left to step 25's long-session memory test, which covers both presenters ([build-plan.md](../../build-plan.md), step 23, "What later steps inherit") |
| After a signal the history's and the core's waits could reach the watchdog | The core has 1.5 s after a signal |
| A profile the project defines that fails to load only when selected might fail every open | Not so: a project's profiles are checked when its files are loaded, and one that cannot be read refuses the file at every open, as before this step |
| 09 said the history is read after the first frame | It is read from the start, beside the start-up; 09 says so |

A second reviewer then read the fixes alone. What it found, and what each changed:

| Finding | Changed |
|---|---|
| On `--fork`, `workspace/files` still waited in the lane, behind the close of the session forked | The workspace's files never wait in the lane: they are asked for only once an open has answered |
| A lane task that panicked would leave its write counted for good, and every later read unanswered | A write is done when its run is dropped, panic or not; what cannot be handed to the lane is answered as the core gone |
| No test held the viewer's deadline or its last key | `a_viewer_opened_over_a_dialog_in_its_grace_leaves_nothing_due`, `a_key_straight_after_the_viewer_closes_does_not_answer` |
| A question withdrawn because its askers left was resolved as answered | Answered only when an answer came |
| The abort test made a real core dump | The binary's core limit is set to nothing first |

The diff's rule (the owner's answer above) was read by a reviewer of its own. It found that every token counted the same in the comparison, so punctuation could outweigh a word that stayed and make it bold (`A,,,B` against `A,B,,,` bolded `B`); that "punctuation alone" was decided for each line, not the pair; that lines too long to compare are real in this repository; and that marks, viramas and joiners cut words of some scripts. Words are now compared with words alone and punctuation only when the pair changed no word, tokens follow grapheme clusters, the bound applies to what is left between the lines' common start and end, and 09-tui says what a line too long to compare shows. It also found that the round's view draws a line holding a tab without its bold: an [open item](../../outstanding/diff-emphasis-on-a-line-with-a-tab.md) for step 32.


## After the review

An independent review of step 23 read `main` at `e73ef24` and confirmed twenty defects with failing tests, ranked as fifteen findings, with the rules its tests showed untested and the docs that disagreed with the code. The owner settled the calls below; the fixes came in two pull requests, the first the dialogs, input, signals and the terminal's end, the second the rest. The rules each fix holds are broken one at a time in `mutations/step-23-fixes.breaks` (and its parts).

### The first pull request: the dialogs, input, signals and the terminal's end

**The dialogs' rule of keys (finding 1, the owner's rule).** Steady typing at a key every 350 ms, a key held down whose first repeat came after the answers were drawn, and a key 100 ms after a paste all answered a dialog under step 23's rule (a key 300 ms after the one before answers). As built now:

- **The quiet:** if anything reached the editor while the dialog was up (a key it passed on, a paste), it is ready only after **1 s with no input**. The figure: 40 words a minute is a key every 300 ms and 15 a minute one every 800 ms, so a typist's gaps inside a sentence are shorter, and a second is short beside the time it takes to read a dialog, so a person who stopped to read it does not wait.
- **The answers row is drawn only when the answers can be pressed,** and is hidden again by input that reaches the editor after it was drawn: the screen never offers a key the dialog would drop, which `cineris_testkit::pty::Session::answer_dialog` relies on.
- **A key held down never answers.** The terminal marks repeats where it can: the enhanced keyboard protocol's event types (the layer now asks for flags 1 and 2, `CSI > 3 u`; a repeat is the new `Event::Repeat`, a release is dropped as before) and win32 input (a key going down again with no record of it going up). Elsewhere, and for keys that type text, which the protocol still sends as text, the same key as the one before it, when that one was typing, within **1 s** (terminals repeat after 250 ms to 1 s) is taken for one. A key the dialog took is not typing, so pressing the same answer twice is two presses.
- **What stays:** the 300 ms grace after the first frame, the 300 ms pause since the key before, keys read together never answering (`App::burst`), a paste never answering.
- **The ordinary case:** the dialog shows, the person reads it and presses one key, and it answers.
- **The same rule** holds for the trust question (`y`, `x`) and a question's digits; the grace starts again after `fg` and after the external editor.

Tests of each trigger (`cineris-tui`'s `app::tests::dialogs`): `steady_typing_slower_than_the_pause_never_answers` and `steady_typing_with_an_e_never_becomes_a_deny_note` (the review's), `a_held_key_whose_first_repeat_comes_after_the_grace_never_answers` (the review's), `a_key_held_from_before_the_dialog_never_answers_and_a_press_after_does`, `a_repeat_the_terminal_marks_never_answers`, `a_key_soon_after_a_paste_does_not_answer` and `a_key_soon_after_a_paste_does_not_trust_the_project` (the review's), `typing_through_the_trust_question_neither_trusts_nor_refuses`, `typing_through_a_question_does_not_pick_a_choice`, `once_the_typing_stops_for_a_second_one_key_answers`, `in_the_ordinary_case_one_key_answers_once_the_grace_has_passed`, `answers_shown_are_hidden_again_by_input_that_reaches_the_editor`, `after_fg_the_dialog_starts_its_grace_again`, `after_the_external_editor_the_dialog_starts_its_grace_again`; and in `cineris-term`, `a_repeat_is_told_apart_and_a_release_dropped`, `a_win32_key_down_again_before_it_went_up_is_a_repeat` and `the_keyboard_protocol_is_asked_for_its_event_types`.

**A note cut off (finding 8).** A deny note being written when its request ends from outside (the turn's end or time limit, another client's answer) goes into the editor as a draft, a notice says so, and an `Enter` before a pause of a second does not send it.

**The terminal closing (finding 2).** A read that finds the terminal gone (end of input or EIO on Unix, a failed console read on Windows) is SIGHUP's end, exit 129, with the watchdog, and the SIGHUP that follows it is that same end. The hand-off's handlers now stay installed after the terminal is left, until `signal_ended`, so a signal while the program ends or while the external editor has the terminal is handed over: point 249 above ("the watchdog ends the process") was not so before, and is now. The external editor's hand-back does not enter the terminal again once a signal came; the program ends as it asks.

**A signal mid-key (finding 5).** `app::interactive` returns `Exit::Signal` whenever a signal was handed over, whatever the loop or leaving the terminal said; the binary gives the core 1.5 s from a signal that comes while it ends, takes that signal's code, and bounds the log's last flush (`LogHandle::flush_within`).

**Quitting from the viewer (finding 10)** leaves the alternate screen before the presenter erases the live region, and so does a stop (`Ctrl+Z`); a resume re-enters the alternate screen the stop's restore left (step 25's note). The alternate screen's record is set before turning it on and cleared only after turning it off, so a signal in between leaves the terminal on the main screen.

**The restore erases the whole live region (finding 14's first part).** Each frame tells the terminal how many of the region's rows are above the cursor (`Sink::region`), and the restore moves up that many before it erases; the input box's top border is no longer left over. On Windows a frame being written finishes before the console handler's restore (a lock it waits on for at most 250 ms; finding 15's Windows note), and a console that fails a read ends the program as its close would, with a watchdog of its own.

**The lower-confidence notes of this part:** `Ctrl+C` over the trust or undo question, or in the viewer, with no turn running leaves the hidden editor's text and says that a second quits; a paste while the viewer is open goes to its search, never the hidden editor; `End` then `PageDown` in the viewer no longer overflows. A signal sent to the program alone while the external editor runs still leaves the editor running: [a-signal-while-the-external-editor-runs](../../outstanding/a-signal-while-the-external-editor-runs.md), step 32.

**Records corrected:** point 254 above says a core-thread panic prints the crash report's pointer; only the interface thread's panic writes a report, and the core thread's says that the log has the details (09-tui was right). 09-tui's signals, viewer, approval-dialog and question-dialog paragraphs say what is built now; its line that a question offers `y` beside its choices was wrong (`y` is for a question with no choices). The known items [terminal-test-gaps](../../outstanding/terminal-test-gaps.md) and [a-paste-that-stalls](../../outstanding/a-paste-that-stalls.md) say what the review found and what these fixes closed.

**The review of the signal half.** A reviewer read the signal and end fixes as soon as they were written. What it found, and what each changed:

| Finding | Changed |
|---|---|
| A SIGHUP handed over, then the hang-up's own SIGHUP (a shell resends it to its job before the kernel sends its own), killed the program part-way, since only a read's end of input marked the hang-up | Any SIGHUP after a SIGHUP's end, or after the terminal went away, is that same end; tested by `cineris-term`'s `handoff::sys_a_second_sighup_is_the_same_end`, which raises it again on the child's own thread |
| The pseudo-terminal tests of the terminal closing pass in either order of the hang-up's signal and the reader's end of input, so the breaks listed for them were caught only by timing | Each rule is tested without a race: a child with no controlling terminal, which the kernel sends no signal, so only its read says the terminal went away (`handoff::sys_a_terminal_that_goes_away_under_a_read_is_sighups_end`, on `cineris_term::pty::Pty::spawn_without_control`); the library-level tests stay as tests of the outcome |
| The bounded log flush was followed by the handle's drop, which flushed again with no bound | `LogHandle::finish_within` flushes within its limit and leaves the handle undropped |
| The live region's rows were published after the frame was written: a restore during a write moved up the old count from wherever the frame's cursor was | The count is said before the write, and a restore that interrupts a write moves up nothing; the write sends it again, with the frame's count, once the frame has landed |
| Entering the terminal again installs only the handlers left out, untested | The editor's children hand signals over as the application does, and a resize reaches the application after the editor |
| 09-tui said none of it | It does now, with the new public items in the terminal layer's table |
| The Windows watchdog exited at once when it had no event to wait on; the end's margins (history 1 s, core 1.5 s, log 0.4 s against the watchdog's 3 s) left 0.1 s | It waits its 3 s; the log's bound is 0.3 s |
| A comment said a stray `?1049l` on the main screen does nothing | It moves the cursor to the position saved last in xterm and VTE; the order chosen still never leaves the terminal on the alternate screen, and the comment says so |

Left as the reviewer found them, by reading: when a signal comes while the core ends after the core went away, the program ends as the signal, and the core's failure message is not printed; a terminal that hangs up during a normal end makes the last drain of replies wait out its 100 ms.

**The full review of the first pull request.** A second reviewer read all of it. What it found, and what each changed:

| Finding | Changed |
|---|---|
| Asking for the enhanced protocol's event types made every arrow, `Home`, `End`, `PageUp`, `PageDown`, `Delete`, `Insert` and function key arrive twice: their legacy forms carry the event type too, and a release was read as a second press | The legacy forms read the event type as the `u` form does: a release is nothing, a repeat is `Event::Repeat` (`a_legacy_keys_release_is_dropped_and_its_repeat_told_apart`) |
| Typing slower than 300 ms a key still answered a dialog whose first frame came between two keys: the loop drew the answers at the grace's end, before any key reached the editor, and the first tests left the loop's wake out | The quiet counts any input no dialog took, from just before the dialog as well as while it is up (a dialog's first frame takes the last such time); a key a ready dialog has no use for that types a character goes on to the editor as typing; the tests wake as the loop does (`steady_typing_through_the_loops_wake_never_answers`, `typing_just_before_the_dialog_counts_as_typing`, `a_typing_key_the_dialog_has_no_use_for_goes_to_the_editor`) |
| A key held after answering one dialog answered the next, which was never seen | The dialog after one a key answered is ready no sooner than a second after that key (`a_key_held_after_answering_one_dialog_never_answers_the_next`) |
| The row could offer a key the dialog dropped as a key held down (typing that ended in `y`, then `y` at the row; `Esc` closing the viewer, then `Esc` to deny) | The row shows only a quiet second after the last key no dialog took, so the same-key rule never meets a key the row offers (`a_key_the_row_offers_is_taken`, `esc_after_closing_the_viewer_denies_when_the_row_offers_it`); `Ctrl+O` works before the dialog is ready, since it answers nothing |
| A quit armed by `Ctrl+C` over a dialog stayed armed after the dialog took a key | A key a dialog takes disarms it (`answering_the_dialog_disarms_the_quit`) |
| The enhanced protocol marks no repeats of the keys that type text, which the docs implied | `dialog.rs` and 09-tui say so: those rely on the same-key rule, the quiet and the wait after an answer |
| Five rules had no break | Each has one, with a test where it had none (`a_paste_after_a_cut_note_holds_the_enter_too`) |

Left, by reading: a win32 key-up that never arrives (focus lost while the key is down) makes the next press of that key one repeat; any other key, or its up, ends it.

**The review of what changed after it.** A third reader read the screens and the fixes above. What it found, and what each changed:

| Finding | Changed |
|---|---|
| The pause of 300 ms since the key before still dropped a key the row offered, after a global key (`Ctrl+T`, `Ctrl+L`), or after `e` then `Esc` back out of a note | The pause is gone: keys read together are input no dialog took (`App::burst` holds the dialog a quiet second), and the quiet covers what the pause did (`a_key_the_row_offers_soon_after_a_global_key_is_taken`) |
| Reading the request whole (`Ctrl+O`), then `Esc` and `y`: the viewer's keys held the dialog a second, and `y` was typed into the editor | Keys in the viewer while a dialog is up are not typing at it, a search typed there excepted; back from the viewer the dialog starts its grace again (`ctrl_o_then_esc_then_one_key_answers`) |
| Approving one dialog after another quickly typed stray answer keys into the hidden editor | An answer key right after a key a dialog took does nothing while the next is not ready, and holds it (`an_answer_key_before_the_next_dialogs_row_is_not_typed`) |
| The same-key rule for unmarked repeats could no longer change anything, since every key not taken holds the dialog a second (as long as the repeat window) | It is gone, and a compile-time check holds the quiet at least the repeat window; a marked repeat goes where its press went (the editor for typing, nowhere for a key a dialog took) |
| Breaks that no longer bit, or no longer applied | Each was rewritten to the code as it is, or marked `expect: survives` with the second guard named |
| Under load, the tests of a signal read a reply's failed write as a failure, and two compared screens before the last output was read | The children look at the hand-over before an error, the editor waits for the application to take the signal (Linux's `/proc` status), and the screens are compared once the result is shown |

**The golden screens** (shipped with the first pull request). The golden screens now compare the application's own dialogs. Screens 2a and 2b are drawn by the real `App`: a scripted core gives it the request, in a window of the screen's size, and the frame it draws once the dialog takes keys is compared cell by cell: the answers, their order and labels, the fitting to the window, the blank row and the hint row. Both matched the reference files unchanged. Two new screens, 2c (the trust question, in a window it exactly fills) and 2d (the undo's question), were written from the application for the owner to see. Six breaks show the comparison now catches a reordered or relabelled answer, a dropped blank row and a change of fitting; five of them are caught by the screens test alone.

### The second pull request, its first part: what reaches the screen, the sessions, the budgets

**What reaches the terminal and the clipboard (findings 3, 7 and 14).** The review found four paths by which model text reached the screen or the clipboard past the escape of what acts unseen, and they are closed. A numeric entity in markdown (`&#x202E;`) is decoded after its line was sanitised, so the decoded character now goes through `cineris_term::sanitize` too. `/copy` drops control characters but line feeds and tabs and escapes what acts unseen before the text leaves for the clipboard, and says it *sent* the text: a terminal does not answer OSC 52, so the promise in 09-tui that `/copy` reports when it cannot tell was corrected, and the fallback of showing the text for copying by hand is the outstanding item `copy-where-the-clipboard-sequence-is-not-taken` (step 25). An emoji's variation selector in the next delta was drawn as `\u{fe0f}` for good. `cineris_base::visible_keeping_after` and `cineris_term::sanitize::sanitize_after` now escape a piece as it reads after the character before it, and the markdown stream passes that character, so the same answer gives the same lines streamed or whole. The prompt editor lays each cluster out in the columns of what is drawn for it, so a zero-width space no longer puts the cursor inside its own escape.

Only `http` and `https` addresses become terminal hyperlinks (`cineris_term::line::linkable`). Any other, `file:` included, is drawn as its text followed by its address, in paragraphs and table cells alike. The hostile-text tests of the dialogs, the listing and the menus now hold a bidirectional override, a zero-width space and a tag character, not only C0 and C1, and `testing::problems` finds what acts unseen in drawn lines, so removing `sanitize` at any of the nine sites the review named now fails a test (`mutations/step-23-fixes-output.breaks`).

**The sessions (findings 4 and 6).** A history read taken while a turn streams now draws the items the read gives in progress as far as they had gone, and continues them with the events after the read; an item the read holds whole drops all its held events. Whether a turn was running at the read comes from those items and the turn's held events, not only from the open's state, and a turn found running is one `Enter` steers, which step 27's `attach` needs. A response whose `item.started` this client never saw is drawn whole when it completes. `/sessions` and `/new` wait while a session opens or its history is read; an undo, a settings change, `/status` and a history page carry their session and are dropped once it is left; with no session open only notices are shown. 09-tui's Resume paragraph was right and the code was wrong; its line on undelivered steering was wrong (the core holds it after a failure, as 03-agent-loop says) and now says so.

Of the lower-confidence notes: a prompt given back no longer expands the chips the editor holds, though the given-back text itself still comes back whole (added to `editor-cost-on-very-large-text`); a command line's prompt that is a slash command is done as one, as typing it would be, as the owner decided (below); the order and delivery of prompts at a turn's edges is the new item `prompt-delivery-at-a-turns-edges`. The core's `session/read` took its `seq` and its items in progress under separate locks, so an item's content could already hold an event after the result's `seq`; the core's part of the fixes takes them together.

**The budgets (finding 12).** The review found three of the five budget figures measured off the path they name (finding 12). The keys "while streaming" were typed between pieces with the server idle; the streamed piece was let go about a millisecond after a frame, so its 17 ms was the 60-a-second cap; and idle memory was taken in an empty workspace. Each is now measured on its own path with the release binary under the pseudo-terminal: a key while the answer is let go at 2,000 words a second (0.27 ms at the 99th percentile on the desk and a Linux runner, against 4 ms); a streamed piece let go once the screen has been still for longer than a frame (0.40 and 0.50 ms at the 95th percentile, against 20 ms); and the harness's own share of a key on Windows, measured through the real loop with a timing device (0.036 ms at the 99th percentile on a hosted runner), which replaces the claim at `:239` that step 23 had made without doing it. One blocking system test now holds each key's echo in the real binary under 250 ms (500 ms on Windows).

Idle memory: the owner applied the 25 MB budget to a workspace of 20,000 files as well. The `@` list, which the review found adding 6.6 MB at idle, is now asked for on the first `@` and kept in one buffer. With 20,000 files of real code: musl `dist` (what ships) 12.3 MB, Windows runner 16.4 MB, glibc release 30.5 MB on the desk (34.7 before) and 41.1 MB on a Linux runner. The glibc excess is the index's first pass kept by glibc's per-thread arenas (15.6 MB with `index.map = false`, 19.0 MB with one arena and trimming); the owner left it to step 33's `memory-a-pass-leaves-resident` and no trim was added, since what ships is within the budget.

**Left open by this part:** [copy-where-the-clipboard-sequence-is-not-taken](../../outstanding/copy-where-the-clipboard-sequence-is-not-taken.md) (step 25), prompt delivery at a turn's edges ([settled in step 27](../27/answered.md)), `a-history-reads-pages-can-move-an-item` ([settled in step 27](../27/answered.md)), [one-character-wider-than-the-prompts-row](../../outstanding/one-character-wider-than-the-prompts-row.md) (step 25), and `a-leader-close-test-under-heavy-load` (settled by the third pull request, below), the one flake the stress runs of the first pull request found, which then failed once on a hosted runner in this pull request's CI and is looked at in a third pull request. Updated: [editor-cost-on-very-large-text](../../outstanding/editor-cost-on-very-large-text.md) and [memory-a-pass-leaves-resident](../../outstanding/memory-a-pass-leaves-resident.md).

**The owner's answers while this part was built,** each decided by the owner on 2026-11-16, as built: a command-line prompt that is a slash command keeps what typing it does (a known command runs, an unknown one goes to the model); every `file:` link is refused, a hostless one too; screens 2a and 2b stay composed with no turn running and no rules to add until the owner settles the spinner in a dialog's hint row and the line of what a lasting answer allows.

### The second pull request, its second part: trust, the core's questions and the lane

The core's part of the review fixes. A profile only the project's files define can now be selected: a load with no workspace leaves the profile's existence to each session's load, and the trust question and `permissions trust` list every project profile's held-back settings beside its rules, so "`--profile review` works after trust" (points 13 and 237 above) holds. By the owner's call a project's `shell.env_remove` waits for trust, in a new class, *stops commands*. A trust answer the store cannot keep still opens the session (a yes trusts it for that session, a never is not now); an answer of both `trusted` and `never` is refused. A session's close and the core's end withdraw its open trust and rewind questions, and a rewind on a session that ended while it asked changes nothing. A panic in one of the interactive lane's operations answers that operation and the lane goes on; the crash report goes through the log's secret filter; the history file is read line by line and a cut last line is ended; a `/compact` asked during a turn that fails runs when it ends; `workspace/files` ends with the core. The agent loop now clears a turn's running state before it tells `turn.completed`, and `session/read` takes its `seq` and its items in progress together.

Left open: the history's size in bytes (`history-has-no-size-limit`), "never" on a rules-changed question and git trusted unshown (`trust-question-corners`), `session/list` paging by position (`session-list-paging-by-index`), and the workspace root being the working directory where 08 says the nearest `.cineris/` or `.git` (`the-workspace-root-is-the-working-directory`, predates step 23, the owner's).

**The owner's answer while this part was built** (decided by the owner on 2026-11-16, as built): a project's `shell.env_remove` keeps its new trust class, *stops commands*, since every class is held until trust and each says why. The tooling step's reviewer found that the agent loop told `turn.completed` before the turn stopped running, so a `/undo` sent the moment the idle box showed could be refused `turn_running`; that is fixed here, and no other event of the core's is told before its state changes.

### The third pull request: the hang-up test

`cineris-tui`'s `signals::sys_closing_the_terminal_ends_as_sighup` and `…_of_a_leader_ends_as_sighup` failed now and then with a report that held only the first frame's mark (pull request 74's `test-linux`, run 43879303889; `main`'s A/B, try run 43881887984; 1 in 100 at the desk under load). **The cause was the product's, a race in the SIGHUP handler of `cineris-term`'s Unix layer.** The handler asked whether the terminal had already hung up, then put the terminal back, then handed the signal over. When a terminal closes, the kernel sends SIGHUP as the loop's read finds the terminal gone, and gives the signal to another thread than the reader's (the test harness's main thread, here). The read could take the hand-over as SIGHUP's end between the handler's question and its hand-over; the handler then found the hand-over taken, took its SIGHUP for a second signal, and ended the process by SIGHUP's default action before the application's result was said. The application did not outlive its terminal: it was killed part-way through its end, with exit code 129 not given.

- **The evidence:** the test now says, when no result comes, how the application's process ended, from `/proc` (the test's process adopts the job whose leader dies, `cineris_term::pty::adopt_orphans`, so that its zombie stays). Every failure said `killed by signal 1`: 11 of 300 runs of `stress -p cineris-tui --count 300 -f filter="test(/signals::sys_closing/)"` on a hosted Linux runner (try run 43885321175), 1 of 300 with `load=6 cores=2` (43885325510), and 1 of 300 for each test at the desk under `--cores 2 --load 6`. Not the watchdog, and not a stall.
- **The fix:** the handler asks whether the SIGHUP is the hang-up's same end only once it finds the hand-over taken, as Windows's handler already did (`hand_over(code) || hung_up_already(code)`). `docs/architecture/09-tui.md`, "Signals", says so.
- **Its test, with no timing:** `cineris-term`'s `handoff::sys_a_sighup_handled_on_another_thread_as_the_read_finds_the_end_is_the_same_end` (Linux). The child sends SIGHUP to a thread of its own whose handler then waits in the restore's write to a full pipe, closes its input under the loop's read, which takes the hand-over, and only then lets the pipe go. Without the fix it is killed by SIGHUP every time.
- **The breaks:** `the-hang-up-asked-before-the-hand-over` (the old order) is caught by the new test, and `the-sighup-after-a-hang-up-ends-the-process` by it and by `sys_a_second_sighup_is_the_same_end` (`mutations/step-23-fixes.breaks`).
- **After the fix,** on hosted Linux: 0 failures in 1,000 runs of each test with no load (43886906810), 1,000 under `load=6 cores=2` (43886391968) and 300 more (43885978671); Windows' `test -p cineris-tui -p cineris-term` passed (43885988396).
- **The review of this pull request** found the same end cut short another way: a SIGTERM handed over, then the window closed. The read that found the terminal gone returned at once, the end being under way, without noting the hang-up, so the kernel's SIGHUP was taken as a second signal and killed the program part-way through the SIGTERM's end. The read now notes it on both systems (`hung_up_while_ending`), and the SIGHUP is that same end; a second SIGTERM or SIGINT still ends the program at once. Tested in `cineris-term`'s `handoff::sys_a_sighup_after_the_terminal_went_away_during_a_sigterms_end_is_that_end` and `…_a_second_signal_after_the_terminal_went_away_during_a_sigterms_end_ends_it_at_once` (Linux; the Windows twin is reasoned about), with the break `the-terminal-gone-during-an-end-is-not-a-hang-up`.
- **The stress of both crates whole** (`stress -p cineris-tui -p cineris-term`) then found an older flake of the same tests' family: the hand-off tests' message was written after query replies that came once the terminal was put back, which the terminal echoed, and it wrapped (9 of 200 at the desk, 1 of 200 on a runner, 43885982902). It now starts on a row of its own. After it: 200 rounds plain and 100 under load on hosted Linux (43887630985, 43887639700), and 200 plain and 200 under `--cores 2 --load 6` at the desk, with no failure.

**The first frame of the real binary on a cold Windows runner.** `cineris-cli`'s `tui::sys_a_never_survives_a_restart_and_permissions_trust_clears_it` failed on `main`'s side of a Windows A/B (try run 43883744404, its first round): the first frame took 790 ms, over the coarse check of 600 ms (five times the runner's budget of 120 ms). The check measured from the shell's start of the binary, and that was the runner's first start of the `cineris` executable, whose scan by the system's antivirus and pages read from disk are the runner's cost, not the interface's; the budget's own runs (`cargo xtask startup`, the `perf_` test) are of a binary started before. `cineris-term`'s first-frame check starts the test binary that is already running, so it has no such first start. The shell of `cineris-cli`'s TUI tests now starts the binary once with `--version` before the measured start; the limit is unchanged.

## After the review: the follow-ups

Five open items the reviews of steps 17 and 23 found, built by the owner's decisions of 2026-11-16 after step 23 merged: what each changed, the trust store's version 2, the measurements and the review are in [after-review/](after-review/).
