# Step 27, its parts: attach

The third of step 27's three pull requests: `cortex attach`, the prompts at a turn's edges, a TypeScript client typed only by the generated declarations, the server's decoder in the fuzz job, a measurement of what the server adds, and the step's remaining records ([answered.md](../answered.md), "The design, before building", points 5 and 7). What it does is in [architecture/11-rpc-protocol.md](../../../architecture/11-rpc-protocol.md), "As built in step 27".

## What was built

- **`cortex attach [--socket PATH] [--continue | --resume ID] [PROMPT]`** (`cortex-cli`'s `attach`): the interactive application on a server's session, with no core of its own, through a second `Link` over the RPC connection. Operations are requests matched back to their tickets; events and the server's requests go into an `EventQueue` of the link's own, fenced at a history read as the core fences the in-process one; results are taken in `Order`; the end answers every operation still asked. Its I/O runs on a thread of its own with a current-thread runtime. Leaving is a disconnect, and the server keeps the session.
- **The server checked before it is trusted** (`cortex_rpc::socket::connect`): on Unix the socket and its folder the user's, and the process at the other end the user's; on Windows the pipe owned by the user and labelled medium or above, asked of the open handle (a pipe with no label is medium, as Windows reads it: only a process at medium or above can make one, and it must also be the user's to pass the owner check).
- **The core's queue** makes public what a link outside the core needs: `EventQueue::push_request`, `fence`, `unfence` and `close`. No behaviour changes.
- **Prompts at a turn's edges** (`cortex-tui`'s `app::state`, settling the open item): the prompts held while a session opens are sent one at a time, each once the one before was answered, a refused one giving back those behind it in order; the first is sent with `steer` when the open says a turn is running; a prompt refused `turn_running` is sent again once with `queue`.
- **A TypeScript client** (`crates/cortex-cli/tests/system/ts_client/turn.mts`), typed only by `docs/protocol/ts/cortex-protocol.d.ts` and run by Node 22.18 or later with no build step, starts `cortex serve --stdio` and runs a turn. `tsc` checks it against the declarations first, so a protocol change it no longer fits fails. It runs in a job that blocks nothing, `ts-client`, when `cortex-protocol`, `cortex-rpc`, the declarations, the client or its workflow change, and in the nightly ([testing-and-ci.md](../../../testing-and-ci.md), "The TypeScript client").
- **The server's decoder fuzzed** (`fuzz/`'s `rpc_decoder`, in `cargo xtask fuzz` and the nightly job, 120 s a night): any bytes as a client's stream, through the server's own `wire::read` (compiled from its file, so the fuzz build does not take in the core), into lines, a request's operation and an answer to each request the server puts. Nothing may panic, what cannot be read gets an error a client can read, and what is read is the same once written again. Its fuzzing found that a line that is not JSON could get -32600 rather than -32700, and that a JSON array was served as a message: both fixed (`cortex_base::framing::decode` classes a failed line by whether it reads as JSON text at all; `wire::read` refuses an array as a batch).
- **The measurement** `perf_a_requests_round_trip_through_the_server` ([performance-budgets.md](../../../performance-budgets.md), "Latency the harness adds").

## Where it departs from the design

| Design | As built | Why |
|---|---|---|
| `actions/setup-node` in the blocking `test` jobs (point 7) | A job of its own that blocks nothing, and the nightly | The coordinator's answer, by rule 2 |
| The client run with Node's type stripping alone | `tsc` from npm checks it first, in the job | Node checks no types: only a compiler shows the client no longer fits the declarations |
| A prompt typed while held ones are on their way, not named | It follows them; `/new` and a switch wait for them | Their order holds only so |
| Held prompts given back at an interrupt | The ones not yet sent are queued ahead of the interrupt, and the core holds them in order behind the next prompt | Giving them back let one overtake another (the review, below) |
| The fuzz target on `cortex-rpc` | On `wire.rs` compiled from its file | The fuzz build takes 57 crates (30 s) rather than 141 (51 s) with the core |

## The "Done when" items this part covers

| Item | Tests |
|---|---|
| A session survives its client disconnecting and can be re-attached | `cortex-cli::system tui::sys_attach_runs_a_turn_on_the_server_and_its_session_outlives_it` (on Linux over a socket and on Windows over a named pipe, under the pseudo-terminal): a turn over `attach`, then `attach --continue` draws it again |
| A minimal TypeScript client using only the generated declarations runs a turn | `cortex-cli::system ts_client::sys_a_typescript_client_runs_a_turn`, in the `ts-client` job (CI run 44009172980) |
| The decoder passes a fixed-length fuzz run in the non-blocking tier | `fuzz/`'s `rpc_decoder`, 120 s in the nightly workflow's `fuzz` job on this branch with no failure (run 44010331313: 70,495 runs); 21 minutes on the desktop, no failure |

## Breaks, stress and the tiers

- **Breaks:** `step-27`, `step-27-prompts` and `step-27-ts` did what their lists expect (70 of 70 before the review; the breaks the review's fixes added and moved after it, 6 of 6 and the prompts' 20 of 20); `step-27-pipe` on a Windows runner, 20 of 20 (`try` run 44018042197), its AppContainer break surviving as its list expects, as does the Unix check of the server's process here.
- **Stress:** `cortex-rpc`, `cortex-cli` and `cortex-tui`, 742 tests 30 times; `cortex-rpc` and `cortex-cli`, 175 tests 20 times with `--cores 2 --load 6`; every test passed every time.
- **The tiers,** one runner each, the pull request against `main`, the median of 5 rounds (`try` run 44015094662):

| OS (start) | Tier | The PR | `main` | Change | Budget |
|---|---|---|---|---|---|
| Linux (3 ms) | 1, unit | 12.3 s | 12.1 s | +0.2 s | 20 s |
| Linux | 2, integration | 17.3 s | 17.3 s | +0.0 s | 40 s |
| Linux | 3, system | 20.7 s | 19.1 s | +1.5 s | 60 s |
| Windows (8 ms) | 1, unit | 13.9 s | 12.2 s | +1.8 s | 20 s |
| Windows | 2, integration | 32.8 s | 31.4 s | +1.4 s | 40 s |
| Windows | 3, system | 71.9 s | 70.0 s | +2.0 s | 90 s |

Windows' tier 1 came mostly from `cortex-tui`'s new prompt tests (+0.55 s) and from one test that took a line of 64 MiB, since given a small one. Tier 3 has the pseudo-terminal tests of `attach` (about 1.6 s on Windows).

## The independent review

It read the whole pull request, and then the fix of the line's class. Every finding was fixed with a test that fails without the fix and a break the test catches, or is said here.

| Finding | Fix |
|---|---|
| attach fenced its queue at a read's answer only, and the server may send a delta after the read's `seq` before the answer: merged into one the read drew, its text was drawn twice | Merging of the session's deltas stops from the read's asking; the answer fences at its `seq` |
| An interrupt while a held prompt was on its way could turn the prompts' order round | The held prompts not sent are queued ahead of the interrupt |
| The Unix check of the server's process had no break | A break that survives, with why: setting up another user's server in a folder of the user's needs root |
| A Windows `SAFETY` claim was not shown: a security identifier read past its bytes was not ruled out | Every identifier read is bounded by the bytes it lies in, in the label's and the token's reading alike |
| attach said "the core" when the server went | Its own end is said: the server's connection has ended, its sessions go on there; a pseudo-terminal test ends attach by ending its server |
| An operation that could not be written, or a reader that panicked, left the application waiting | The writer answers it; the end runs from a guard |
| The records said the TypeScript job runs on more than its gate holds | They name its paths as the gate has them |
| The queue's public methods could be misused on the core's own queues | Their documents say they are for a queue the caller made |
| The class of a failed line passed over a byte that is not UTF-8 | A failed line is JSON only when it is text too |

## Open items

- **Settled:** [prompt delivery at a turn's edges](../answered.md) (the item's file is gone).
- **Opened:** [json-floats-read-within-a-few-units](../../../outstanding/json-floats-read-within-a-few-units.md) (the owner's: floats from a client are read to within a couple of units in their last place) and [the-fuzz-package-has-no-blocking-check](../../../outstanding/the-fuzz-package-has-no-blocking-check.md) (a tooling step's); and, for the coordinator, [a-verify-test-waits-on-a-timer-nothing-advances](../../../outstanding/a-verify-test-waits-on-a-timer-nothing-advances.md): step 26's test of a managed `ask` rule's check timed out once in this pull request's Windows CI, waiting on a fake timer nothing advances; not fixed here.
- **Shared, step 27's part said in [answered.md](../answered.md):** `headless-details-left-open` (its (6) left to step 32: the test would cost Windows' tier 3 about 3 s) and `waits-on-the-statistics-without-a-limit` (the server's end bounded).

## Public API

Added, nothing removed or changed: `cortex-core`'s `EventQueue::push_request`, `fence`, `unfence` and `close` (public now, for a link outside the core), and `cortex_rpc::socket::connect`.

