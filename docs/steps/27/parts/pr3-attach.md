# Step 27, its parts: attach

The third of step 27's three pull requests: `cortex attach`, the prompts at a turn's edges, a TypeScript client typed only by the generated declarations, the server's decoder in the fuzz job, a measurement of what the server adds, and the step's remaining records ([answered.md](../answered.md), "The design, before building", points 5 and 7). What it does is in [architecture/11-rpc-protocol.md](../../../architecture/11-rpc-protocol.md), "As built in step 27".

## What was built

- **`cortex attach [--socket PATH] [--continue | --resume ID] [PROMPT]`** (`cortex-cli`'s `attach`): the interactive application on a server's session, with no core of its own, through a second `Link` over the RPC connection. Operations are requests matched back to their tickets; events and the server's requests go into an `EventQueue` of the link's own, fenced at a history read as the core fences the in-process one; results are taken in `Order`; the end answers every operation still asked. Its I/O runs on a thread of its own with a current-thread runtime. Leaving is a disconnect, and the server keeps the session.
- **The server checked before it is trusted** (`cortex_rpc::socket::connect`): on Unix the socket and its folder the user's, and the process at the other end the user's; on Windows the pipe owned by the user and labelled medium or above, asked of the open handle (a pipe with no label is medium, as Windows reads it: only a process at medium or above can make one, and it must also be the user's to pass the owner check).
- **The core's queue** makes public what a link outside the core needs: `EventQueue::push_request`, `fence`, `unfence` and `close`. No behaviour changes.
- **Prompts at a turn's edges** (`cortex-tui`'s `app::state`, settling the open item): the prompts held while a session opens are sent one at a time, each once the one before was answered, a refused one giving back those behind it in order; the first is sent with `steer` when the open says a turn is running; a prompt refused `turn_running` is sent again once with `queue`.
- **A TypeScript client** (`crates/cortex-cli/tests/system/ts_client/turn.mts`), typed only by `docs/protocol/ts/cortex-protocol.d.ts` and run by Node 22.18 or later with no build step, starts `cortex serve --stdio` and runs a turn. `tsc` checks it against the declarations first, so a protocol change it no longer fits fails. It runs in a job that blocks nothing, `ts-client`, when the protocol, the server, the declarations or the client change, and in the nightly ([testing-and-ci.md](../../../testing-and-ci.md), "The TypeScript client").
- **The measurement** `perf_a_requests_round_trip_through_the_server` ([performance-budgets.md](../../../performance-budgets.md), "Latency the harness adds").

## Where it departs from the design

| Design | As built | Why |
|---|---|---|
| `actions/setup-node` in the blocking `test` jobs (point 7) | A job of its own that blocks nothing, and the nightly | The coordinator's answer, by rule 2 |
| The client run with Node's type stripping alone | `tsc` from npm checks it first, in the job | Node checks no types: only a compiler shows the client no longer fits the declarations |
| A prompt typed while held ones are on their way, not named | It follows them; `/new` and a switch wait for them | Their order holds only so |

## The "Done when" items this part covers

| Item | Tests |
|---|---|
| A session survives its client disconnecting and can be re-attached | `cortex-cli::system tui::sys_attach_runs_a_turn_on_the_server_and_its_session_outlives_it` (on Linux over a socket and on Windows over a named pipe, under the pseudo-terminal): a turn over `attach`, then `attach --continue` draws it again |
| A minimal TypeScript client using only the generated declarations runs a turn | `cortex-cli::system ts_client::sys_a_typescript_client_runs_a_turn`, in the `ts-client` job (CI run 44009172980) |
