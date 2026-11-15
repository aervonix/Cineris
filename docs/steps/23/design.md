# Design of step 23: the interactive application

The six points the step's prompt asked to settle before building widely, each with the choice made, what it changes outside `cortex-tui` and `cortex-cli`, and where it departs from a page of `docs/`. Written against `main` at `d050503`.

## 1. Where the application lives

**In `cortex-tui`, as a module `app`,** with the loop, the state and the mapping of events, talking to the core only in `cortex-protocol`'s types through a small trait. `cortex-cli` makes the core, its thread and runtime, the terminal and the history store, and hands them in. `cortex-tui` keeps its three dependencies (`cortex-protocol`, `cortex-base`, `cortex-term`); no edge is added to [01-workspace.md](../../architecture/01-workspace.md).

```rust
// cortex_tui::app
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

pub fn run(terminal: Terminal, link: Box<dyn Link>, start: Start) -> Exit;
```

**What crosses the boundary:** `Op` and `OpResult` (or `ErrorData`), `SessionEvent`, `RequestId` with `ServerRequest` and `ServerResponse`. Nothing else: no `Core`, no `Outgoing`, no configuration type. The link is given the terminal's `Waker` and calls it when something arrives. **What stays on the client's side,** because it is the person's and not the session's: the `[tui]`, `[keys]` and `[terminal]` settings (`Start`, read by the binary), the prompt history (`HistoryStore`, step 21), the external editor, the clipboard.

**The in-process link** is `cortex-cli`'s (`interactive.rs`): a tokio channel of operations to the core thread, where a task calls `Core::handle`; results back through a small queue; events read straight from the client's `EventQueue` with `try_next`, so that its merging and its `item.resync` keep working for a client that is behind (an open external editor while a turn streams). Step 27's `attach` is a second implementation of `Link` over the RPC connection, and drives the same `app::run`.

**Changes this needs outside the two crates:**
- `cortex-core`: `EventQueue` calls a hook when it goes from empty to not empty (the link's waker). One call per burst, not per event.
- Ops run in order on one lane on the core thread, except `turn/interrupt`, `respond` and the reads (`session/list`, `session/read`, `models/list`, `config/get`, `permissions/list`, `workspace/files`), which run at once on tasks of their own, so that an interrupt never waits behind a slow `session/open`.
- **Departure from [02-runtime.md](../../architecture/02-runtime.md):** the ops channel stays bounded at 64, but the interface never waits for room ("the front end waits" there): an op that does not fit is kept, in order, and sent at the next wake. A full channel means the core thread is stuck, and the interface must not be stuck with it.

## 2. The threads and the loop

| Thread | Owns | Wakes the interface by |
|---|---|---|
| **Main: the interface** | `Terminal`, `Inline`, `Scheduler`, `Input`, `Transcript`, the dialogs, the app's state | (blocks in `Terminal::next(timeout)`) |
| **Core** | a current-thread Tokio runtime, timer and I/O on, and the `Core` | the link's waker: an event queue that stops being empty, an op's result, a request |
| The core's blocking pool | session reads, rewinds, the file listing for `@`, trust and approval files | (through the core thread) |
| Highlighter (step 22) | the grammars and each block's state | its waker, which is the terminal's |
| File filter (new, only past 20,000 paths) | the `@` candidates of a large workspace | the terminal's waker, with the query's generation |
| History (new, client side) | the project's history file | the terminal's waker, once loaded |
| Log writer (`cortex-base`) | the log file | nothing |

**One wake of the loop:**
1. Take every terminal event already read (keys, pastes, resizes), each through `Input` and the dialogs.
2. **If a key or a paste arrived, draw its frame now,** before anything from the core is looked at.
3. Then the inbox: every request and op result, and events for at most 4 ms or 2,000 events, whichever comes first. What is left stays in the core's queue, where deltas merge, and a stream frame is asked for.
4. Draw if the scheduler says so: input-driven frames at once, stream-driven ones at most 60 a second.

So a key's frame waits for the key's own handling and one frame, never for the core, a disk or the network: the interface thread makes no call that can wait on any of them (ops are queued, the history is another thread's, the file list is the core's pool's, the highlighter's colours are taken when ready, the file filter's rows a frame later).

**Idle** is no turn running, nothing animating, no highlight or filter outstanding: the scheduler says `Idle`, and the thread blocks in `poll`/`WaitForMultipleObjectsEx` with no time limit. The core thread's runtime is parked with no timer (02-runtime: an idle session has none), and the workers wait on their channels. **No timer is set for "a while":** the hint row's cost after a turn lasts until the next prompt, and the armed quit until the next key (step 21's rule). The second interrupt's window (2 s, from 02-runtime's table) is a comparison of two key times, not a timer. **Departure from 09-tui** ("the turn's cost and the session's, for a while"): until the next prompt instead.

**Order on the way in.** While a `session/new` or `session/open` of this app is outstanding, its session's events are held back in the app (an assembly sends a handful), so that the session's header (from the result) is the first block and the notices of its assembly come after it. Requests are not held: the trust question arrives while the open is outstanding.

## 3. The question and trust dialogs

Both are a change to what the core does. **Proposed, not built yet.**

**`Core::respond` takes all three answers.** The open-request table in `clients` holds a `ServerRequest` and a sender of `ServerResponse`; an answer of the wrong kind for the request is refused (`request_not_open`), as an answer to a request that is not open is now. Only a `user` client that was asked may answer, as for approvals.

**Trust: asked when a session opens** (`session/new`, `session/open` of a stored session, `session/fork`), when all of these hold:
- the client that opens it declared `trust` (the TUI does; headless does not, and nothing changes for it);
- the project is not trusted, or is trusted with its rules changed since;
- **its files hold something trust would turn on:** a setting held back (`Loaded::withheld`), or a rule of the project's (any rule: what is pinned is all of them). A project with no `.cortex/` files, or only preferences, is not asked about: there is nothing to say yes to.

The request lists each thing as a `TrustResource`: each rule (`rules`, the rule's text, `changed` when the rules changed since trust), and each setting held back as a new kind, **`settings`** (`name` the key, `detail` the value and what it raises or widens). `settings` is a new value of an open enumeration that only informs: an addition, with a schema diff, and no new protocol version.

**What the core does with the answer.** `trusted: true`: it pins the project's rules exactly as `cortex permissions trust` does (one function for both, `manage`), then puts the session together trusted. `false`, no answer, or a client that went away: the session is put together untrusted, as today, and the core does not ask again about that project for the rest of its life (a `/new` in the same run is not asked twice); nothing is written, so the next start asks again. The session's assembly is split at that point: the trust look (on the blocking pool), the question (awaited on the core thread, the client's open op waiting on it), then the rest of the assembly, which loads the configuration again only when the answer was yes. The TUI shows it in place of the input box: `? Trust this project?`, the folder, then each resource in full, and `y Trust it   n Not now`; `Esc` is not now.

**Tests:** in `cortex-core` (tier 2): a user client is asked and its yes pins and loads trusted; a no loads untrusted and the next `session/new` of the same core does not ask; a project whose rules changed is asked with `changed`; a headless client is never asked; a project with nothing to trust is not asked; a client that disconnects while asked gets no session (the existing rule). In the TUI: the dialog against a scripted link, and one end-to-end run where a project's `.cortex/config.toml` sets a model, the dialog shows it, `y` is pressed, and the header shows the project's model.

**The question: asked by `session/rewind` when a file was changed since the harness left it.** [07-sessions-and-context.md](../../architecture/07-sessions-and-context.md) says a restore "does not overwrite something the user has edited since unless the user says so", and `cortex-session` already takes that answer (`Restore::overwrite_edited`), but nothing asks it: today such a file is reported as not put back. With a client that declared `questions`, the core plans the rewind, and if the plan names files edited since, asks one `question/ask`: "These files were changed after the turn that changed them: `a.rs`, `b.rs`. Put them back as they were before that turn, losing those changes?", choices `Put them back` and `Keep them as they are`. The first rewinds with `overwrite_edited` set to those names; the second, no answer, or a client that cannot be asked rewinds as today. The history change is held across the question, so no turn starts meanwhile. This is the `/undo` end-to-end test's second half: the user edits the file the turn changed, `/undo`, the dialog, `Put them back`, the file is as before the turn.

**Alternative, if the owner does not want the rewind question:** the question dialog is built and tested against a scripted link only, `respond` takes question answers, and its first sender is step 28 (the `tool.before` hook's "ask the user").

## 4. The slash commands this step ships

| Ships here | Through |
|---|---|
| `/help` | a block listing the commands and the keys, from `ACTIONS` |
| `/new` | `session/close`, then `session/new` |
| `/sessions` | the picker over `session/list` (resume, switch). Fork is step 25's |
| `/undo` | `session/read` for the last turn's first item, then `session/rewind` to the item before it, with `restore_files` |
| `/model`, `/effort` | pickers over `models/list` and the model's levels, sending `session/set`; `/effort` has the fast-mode toggle (`Tab`, `tab - fast mode`) only when the model offers a fast speed, and the toggle sends `session/set` with the speed |
| `/mode` | a picker of the levels, `session/set` |
| `/permissions` | `permissions/list` in a picker; `Delete` removes the selected one (`permissions/revoke`) |
| `/compact [focus]` | `session/compact` |
| `/status` | a block: provider, model, effort, speed, level, sandbox, session id, workspace; MCP and extensions say "none" until their steps |
| `/view [n]` | the viewer |
| `/copy` | the last answer, through the terminal's clipboard sequence (OSC 52), an addition to `cortex-term` |
| `/config` | the effective configuration from `config/get`, without where each value came from (that needs a field the protocol lacks: an item) |
| `/theme` | a picker of the built-in themes (`auto`, `dark`, `light`) for the run |
| `/quit` | quits |

**Owned by later steps, not shipped:** `/context`, `/usage`, `/plan`, `/project`, `/profile`, `/verify`, `/reload` (24); `/rewind` and fork (25); `/extend` (29); `/lease` (after the release).

**Listed in 09-tui with no step, each filed as an open item:** `/mcp` (recommend step 30), `/extensions` (recommend step 28), `/export` (recommend step 32), and `/config`'s provenance.

## 5. The cut into two pull requests

**PR 1, `worktree-step-23-loop`:**
- `cortex_tui::app`: the loop, the `Link` trait, the state; the mapping of every event in 09-tui's table onto the transcript; the hint row from the session's state;
- the input wired: submit, steer, queue, interrupt and the second interrupt, `set_running`, `set_width`, `Ctrl+T` and the live boxes in the focus ring, `Ctrl+L`, `Ctrl+C` and `Ctrl+D`;
- the approval dialog (with the leave-the-sandbox form), answered through `Core::respond`, as today;
- `[tui]`, `[keys]` and `[terminal]` declared in `cortex-config`, the reference regenerated, the redaction test moved to step 21's form of `[keys]`; `KeyMap::with_table` with each `TableError` as a notice;
- interactive mode in `cortex-cli`: the log (file only), the core thread, the in-process link, the start-up order below, `--resume` and `--continue` opening a session (without the history drawn: PR 2);
- `cortex-term`: the panic hook puts the terminal back only for a panic on the thread that entered it (below);
- the pseudo-terminal harness for the real binary on both OSes, with phase timings, and the end-to-end tests of a full turn with tools, an approval, an interruption mid-stream and mid-tool, a steer and a queue;
- the first `try` measurements of start to first frame.

**PR 2:** the trust and question dialogs with their core changes; the other slash commands and pickers; the viewer (`cortex-term` gains the alternate screen, which its emergency restore leaves too); history; `@` files (`workspace/files`) with the filter off the key's path; the external editor; resume with its history drawn and undo, end to end; signals and a graceful end; `config/reload`; the remaining golden rows (2a, 2b, 8a, 8b, the placeholders); the budgets measured; the records.

**Start-up order** (15-efficiency's "open the session after the first frame"): parse the arguments; read the user's configuration for `[terminal]`, `[tui]`, `[keys]` and `log.level` (user layers, profile and `-c` only: every key there is user-only); start the log; start the core thread (`Core::start` runs there, beside what follows); enter the terminal with the overrides; draw the first frame (the input box and the hint row); only then send `session/new` or `session/open`. A `Core::start` that fails (an invalid catalog) leaves the terminal and prints the error, exit 2, as headless does. An open that fails (`day_limit_needs_stats`, a session locked elsewhere) is an error block with its hint, and the app stays up: `/new` and `/sessions` still work.

## 6. The open items naming step 23

| Item | What this step does | Why |
|---|---|---|
| `the-input-box-while-a-turn-runs` | **Closes:** `Type to steer this turn` while a turn runs, nothing while a box or panel has the focus | The input knows both states |
| `names-a-client-shows-as-they-are` | **Closes:** every text the app is sent passes one function, `cortex_base::visible_keeping(text, &['\n', '\t'])`, before a view (and so before `cortex_term::sanitize`): approval details, `adds` (escaped where drawn, kept exact as data), undo and rewind messages, notices, errors, model text and tool output. Tool output has `\r\n` and a lone `\r` taken as line breaks first | One place, as the command line does; `cortex-term` cannot see `cortex-base`'s set without a new edge |
| `config-reload-through-the-core` | **Closes,** in PR 2: `config/reload` re-checks the files (the old configuration stays on error), applies the keys marked `Now` to each open session (the engine's rules through a new engine as `session/set` makes one, the limits, the retry policy, the log's level), and lists the `NextEpoch` keys as `pending` and the `Restart` keys as `pending` too, with a sentence. `/reload` stays step 24's | The item names it; step 24 needs it |
| `continue-chooses-the-newest-session` | **Closes:** `session/list` orders by `updated_at`, newest first (a change to `cortex-core`'s list, as the item says); `--continue` and the picker take that order | One order for both |
| `files-over-the-undo-limit` | **Recommendation: keep the refusal, and close the item with that answer.** Asking would let `edit` and `write` read and match a file of any size in memory (16 MiB is also their memory bound), against the memory budgets; the model is told why, and a shell command's change to such a file is detected and listed as one an undo cannot put back. **The owner's call** | Asking costs more than it gives |
| `file-completion-on-a-large-workspace` | **Closes:** past 20,000 paths the filter runs on a thread of its own; the key's echo is drawn first and the menu's rows a frame later, a result for a query that is no longer the text's dropped | The item's recommendation |
| `signals-and-a-graceful-shutdown` | **Closes,** in PR 2. SIGTERM, SIGHUP, SIGINT and SIGQUIT (Ctrl+Break, close, log-off on Windows): the layer puts the terminal back at once, as now, then hands the signal to the app instead of ending the process (an addition to `cortex-term`: the handler wakes the loop through the waker's pipe), and a watchdog ends the process 3 s later if the app has not. The app interrupts the turn with `now`, runs `Core::shutdown` (at most 2 s), and exits 143, 129 or 130. **A failed write** after an emergency ends the app the same way. **A panic on the core thread** ends the app: terminal put back, the crash report's one line, exit 1 | The item asks for the three decisions |
| `log-level-from-configuration-in-headless-mode` | **Closes both halves:** the binary reads `log.level` from the user's configuration before the log starts, in both modes (`--log-level` and `CORTEX_LOG` still rank above it) | The same cheap load the TUI needs serves headless; to be measured against headless's start-up budget, and the headless half left if it costs |
| `reading-a-key-at-a-terminal` | **Closes,** in PR 2 if the time allows: `cortex auth login` at a terminal reads the key with echo off through `cortex-term`, reached through a small function in `cortex-tui` | It needs only the terminal layer |
| `a-background-summary-past-its-turn-cannot-be-interrupted` | **Leaves it, for step 27:** the TUI offers no interrupt between turns (`Esc` and `Ctrl+C` idle are the editor's), and quitting ends the session, which stops the summary | The item's condition ("when a front end offers an interrupt between turns") is not met |
| `session-compact-after-a-turns-last-request` | **Closes:** at the end of a turn with a compaction pending, the loop runs a turn that only compacts, as an idle session's `session/compact` does, and `docs/protocol/` says so | `/compact` is offered during a turn |
| `terminal-test-gaps` | **Does its half:** the real binary under a pseudo-terminal restored after exit, a panic and each of SIGTERM, SIGHUP, SIGINT and SIGQUIT one by one (Linux), and a panic on another thread (the core's) | The presenter's generated scripts and the stop under a shell with job control stay |
| `ambiguous-width-glyphs` | **Recommendation: leave it,** and document `tui.glyphs = "ascii"` for a terminal set to draw them wide. Most terminals in a CJK locale draw them narrow by default, so choosing ASCII by locale would take the look from the many for the few | **The owner's** |
| `highlighting-the-line-in-progress` | **Recommendation: leave it plain until it ends,** as built: a row that changes colour as it grows (an opening quote) is worse than a plain one, and colouring it costs a round trip to the worker each frame | **The owner's** |
| `trust-of-a-profile-a-project-defines` | **Recommendation: one pin of every rule the project's files hold, in their base and in every profile they define,** compared the same way at every start whichever profile is selected; the trust dialog and `permissions trust` show each rule with the profile it is in. One question, and `--profile review` works after it | **The owner's** |
| `the-default-editor-on-windows-11`, the folder half | **Builds:** a folder that cannot be removed is tried again when the app ends, and one still there is named on standard error after the terminal is left. **Checked by hand:** on Windows, `Ctrl+G` with `code --wait` or an editor whose helper holds the file; quit; the message names the folder | The other half is answered |

## What changes outside `cortex-tui` and `cortex-cli`

| Where | Change | Kind |
|---|---|---|
| Protocol | `workspace/files` (session, limit): the workspace's paths with the ignore rules, newest change first, at most 100,000, listed on the blocking pool by `cortex-tools`' walk; `@` needs them listed "by the core" (step 21), and over RPC the workspace is the server's | New method |
| Protocol | `TrustResourceKind::Settings` | New value of an open enumeration |
| Protocol | `session/list` ordered by `updated_at`; `session/compact` at a turn's last request runs a compact-only turn after it; `session/rewind` may ask a question | Behaviour, said in `docs/protocol/` |
| `cortex-core` | `Core::respond` takes question and trust answers; the trust question at open; the rewind's question; `config/reload`; `EventQueue`'s wake hook | API additions and behaviour |
| `cortex-agent` | The compact-only turn after a turn whose last request had a compaction pending | Behaviour |
| `cortex-config` | `[tui]`, `[keys]`, `[terminal]` declared | Schema, reference regenerated |
| `cortex-term` | The panic hook's thread; the signal hand-off; the alternate screen; the clipboard sequence | Public API additions, recorded in 09-tui |

**Also closed:** `a-caught-panic-leaves-the-terminal` (step 20's hook put the terminal back for any panic in any thread, so a tool's panic, which the core catches, would leave the app drawing on a terminal it no longer holds): the hook puts it back only for a panic on the thread that entered the terminal, and the app ends cleanly on any other that is not caught (above).
