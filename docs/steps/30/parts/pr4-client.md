# Step 30, its parts: the client

The fourth of step 30's pull requests: our own client of MCP in `cineris-mcp`, both eras over standard I/O, behind `Client` ([answered.md](../answered.md), points 0, 1, 3.5 and 3.7, and the table's answers), and fake servers of both eras for any crate's tests. Streamable HTTP is pull request 6, which plugs into the transport described below; the lifecycle (pull request 5) uses this client.

**Nothing calls it yet.** This pull request merges first, before the process, the registry and the approvals (pull requests 1 to 3): no session, command or code path of the shipped binary reaches the client until the lifecycle wires it in behind those gates. `cineris-core` links `cineris-mcp` (feature `mcp`, as before) and uses nothing of it.

## What was built

- **JSON-RPC** (`jsonrpc`): requests, notifications, results and errors read by hand from `serde_json::Value`, with no derived types. An id is a string or an integer, never null; an error with no id is a message of its own; a line holding an array is a batch, read only from a `2025-03-26` server (another's is counted and left). An answer whose id is one of ours written as a string of its digits is matched.
- **A transport** (`transport`, `stdio`): the trait the session speaks through, and its standard I/O form over `ServerPipes`. A task reads the server's output for as long as the connection lasts: an answer goes to its pending request; **every other message is counted with its bytes**, and past 1,000 messages or 16 MiB in a minute the server is stopped as misbehaving (its input closed, then `ServerProcess::shutdown`); the rest are handed to the session. A line holding a batch is counted once with its bytes, an empty one included, and each message in it that answers nothing too. The connection ends when the output ends (with `exited()`'s words, given up to the shutdown grace to come), when the process exits (what it wrote is read until its output ends, for the grace at most), on a line over the pipes' cap, or at that allowance; every pending request then fails with why, and `closed()` resolves. **A request written and then lost fails with `MayHaveRun`** (the call may have run, point 3.7); one never written (refused after the end, or still waiting for room) with `Stopped`. A listing, the probe and the handshake take either as `Stopped`.
- **The session** (`session`), the protocol over any transport: ids, at most 32 requests waiting at once, each request's limit on the `SharedTimer`, and a request given up (cancelled or past its limit) cancelled by `notifications/cancelled { requestId, reason }` in both eras, its late answer a stray. `_meta` on every modern request (`protocolVersion`, `clientCapabilities: {}`, `clientInfo`). Listings of every page (an empty string is a cursor) within 30 s, 20 pages, 512 tools and 16 MiB of definitions, each tool kept exactly as sent in `raw`; **one listing at a time**, shared with who asked while it ran unless a change came since it began; a modern listing's lifetime (`ttlMs`, the least of its pages', absent or negative being 0) bumps `changes()` when it ends, and a lifetime of 0 bumps nothing. Calls within the definition's limit; a modern `input_required` with only a `requestState` sent again with a new id and the state given back, at most three times; one with `inputRequests` the client never declared, or with no state, or a result of a type the client does not know, is a protocol error. Content blocks into `Content` (base64 counted, not kept; a block of an unknown type left out); a legacy `structuredContent` kept only when it is an object.
- **The server's messages** (`session::Router`): a legacy server's `ping` answered with `{}` at any time, the handshake included, and any other request with `-32601`; a modern server's request counted as a fault and never answered; `notifications/tools/list_changed` bumps `changes()` (legacy: any; modern: only on the open, acknowledged change stream it names); unknown notifications left.
- **The change stream** (modern, when the server says `listChanged`): `subscriptions/listen` with `toolsListChanged`, no time limit; the acknowledgement checked against what was asked (one without the change notices is cancelled and not asked again, and one answered with an error is over and not asked again); the server's end taken either way (a result, or `notifications/cancelled` naming it), and the stream opened again a second later, its acknowledgement bumping `changes()` for what was missed; ended by `notifications/cancelled` at shutdown, before the process.
- **The start** (`connect`): `server/discover` with `2026-07-28` in `_meta`, 3 s; a discovery result or `-32020`, `-32021`, `-32022` is modern and never sent `initialize` (`-32022` asked once more with a revision from its `supported` list; `-32020` and `-32021` end the start with why); any other error, a result that is not a discovery result, or silence, is legacy, and `initialize` (`2025-11-25`, `capabilities: {}`) follows on the same process, the answer taken if it is a revision the client speaks, then `notifications/initialized`. **After a silent probe, the probe is still heard while `initialize` waits:** a discovery result (or a modern error) coming then makes the server modern, for a modern server slower than the probe to start (a cold `npx` or `docker run`), and `initialize`'s answer reaches nothing; a server silent on the probe that refuses `initialize` is asked once more what it speaks, on the same process. A process that exits during the probe, or after answering it and before `initialize` is answered, is started again by the caller's closure, once, and sent `initialize` alone. A late answer to the probe once `initialize` is answered is a stray. **The whole start is held to `start_timeout`**, the probe included, and `initialize` is never cancelled; a start that fails ends the server's process. `connect_known(Era::Legacy, …)` skips the probe, probes after all when `initialize` is refused, and starts the server again and sends `initialize` alone when it exits; `connect_known(Era::Modern, …)` probes, which tells a modern server's capabilities.
- **`StdioClient`**, implementing `Client`: `shutdown()` ends the change stream, then closes the input and calls `ServerProcess::shutdown(2 s)`, once. Dropping it inside a Tokio runtime does the same without waiting, the process ended in a task of its own; outside one, no task can be started, so only the server's input is closed and its process is left to what dropping the `ServerProcess` does (`Client`'s doc says so).
- **Fake servers of both eras** (`cineris_mcp::fake`, behind the new `test-kit` feature, which only dev-dependencies turn on): scripted, in process, over in-memory `ServerPipes`, each `spawn` a fake process that exits when its input closes (reading what was written first), when the script says, or when the test says. They answer `server/discover` (a discovery result, a result of another kind, silence, silence the first time only, an exit, an error, an error then an exit, `-32022` a number of times) and `initialize` (any revision, pinging the client first and waiting for its answer, held behind a gate, or exiting in the first process), exit on any request before `initialize`, list tools in pages (held behind a gate if asked, a lifetime for each page), answer calls (a result, an error, behind a gate, `input_required` with states checked, twice, a crash, an exit with the answer handed on after it, never), acknowledge (behind a gate if asked) or refuse a change stream, answer it with an error, and end it either way, send list changes, anything the test writes (duplicates, unknown ids, floods, requests to the client, raw lines, a line over the cap), crash, and record every message each process received.
- **Docs:** [13-mcp.md](../../../architecture/13-mcp.md), "Implementation" (our own client, the probe, what a server can make the client hold); [01-workspace.md](../../../architecture/01-workspace.md) (`cineris-mcp`'s row, rule 5, the `test-kit` feature); [decision 0010](../../../decisions/0010-mcp-in-first-release.md), amended.

**Not built here: the interoperability check against `rmcp`'s server** (`interop/`, `cargo xtask interop`, the nightly job, the audit of its lock file, its row in `dependencies.md` and its line in `testing-and-ci.md`). It is part 4b, built apart and merged after this one; 13-mcp and decision 0010 say it is being added.

## The changes from the review of the design, folded in

The maintainers' message of 2026-11-27, binding on this part:

1. **The allowance counts every message that answers nothing**, notifications included, and their bytes: 1,000 messages or 16 MiB a minute. Tested with valid notices in a loop (the server stopped, another server's listing and call going on), and with each kind of message on its own.
2. **List changes coalesce:** `changes()` is a `watch` counter (a burst is one wake to a receiver that was not looking), and a listing asked for while one runs waits for it, so a burst leads to at most one listing in flight. Pacing the lifecycle's listings to one a second ([answered.md](../answered.md), point 3.7) is the lifecycle's.
3. **The probe's 3 s is inside the 10 s start limit:** a silent legacy server whose `initialize` is slow fails at 10 s from the start, not 13.
4. **`interop/` is a package outside the workspace**, not a dev-dependency (not built, above).

## The interfaces added

No existing signature changed. Added to `cineris-mcp`'s public interface:

```rust
pub struct StdioClient;                       // implements Client and Drop
impl StdioClient {
    pub async fn connect(definition: &ServerDefinition, pipes: ServerPipes, respawn: Respawn,
                         timer: SharedTimer, options: ClientOptions) -> Result<Self, ClientError>;
    pub async fn connect_known(era: Era, definition: &ServerDefinition, pipes: ServerPipes,
                               respawn: Respawn, timer: SharedTimer, options: ClientOptions)
                               -> Result<Self, ClientError>;
    pub fn faults(&self) -> u64;              // requests a modern server sent, ignored
    pub fn listening(&self) -> bool;          // the change stream open and acknowledged
}
pub type Respawn = Arc<dyn Fn() -> Result<ServerPipes, SpawnError> + Send + Sync>;
#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub probe_timeout: Duration,              // 3 s
    pub list_timeout: Duration,               // 30 s
    pub shutdown_grace: Duration,             // 2 s
    pub max_in_flight: usize,                 // 32
}                                             // and Default
pub const MAX_TOOLS: usize = 512;
pub const MAX_PAGES: usize = 20;
pub const MAX_ROUNDS: usize = 3;
pub const MAX_LISTING_BYTES: usize;           // 16 MiB
pub const LISTEN_AGAIN: Duration;             // 1 s
impl ToolDefinition {
    pub fn unusable(&self) -> Option<&'static str>;  // runs only as a task, or no object schema
}
```

Behind `test-kit`: `cineris_mcp::fake::{FakeServer, Gate, Discover, Reply, EndListen, LINE_CAP}`. A test makes a server with `FakeServer::legacy("2025-06-18")` or `FakeServer::modern()` and the builder methods (`on_discover`, `exits_before_initialize`, `exits_on_first_initialize`, `pings_during_initialize`, `initialize_gate`, `tools`, `page_size`, `list_gate`, `list_changed`, `ttl_ms`, `ttl_pages`, `refuses_listen`, `listen_error`, `ack_gate`, `reply`, `stderr`, `supported`), starts it with `spawn()` (and `respawn()` for the client's closure), and drives and reads it with `send`, `send_line`, `send_too_long`, `flood`, `notify_list_changed`, `end_listen`, `crash`, `set_tools`, `received`, `received_by`, `methods`, `of`, `wait_for`, `wait_until`, `listen_id`, `spawns` and `shutdowns`.

The lifecycle (pull request 5) keeps `client.info().era` for the definition and passes it to `connect_known` at the next start; it should relist when `changes()` moves, at most once a second and not on every lifetime's end at once (the specification asks a client not to poll by `ttlMs`).

### How Streamable HTTP plugs in

Pull request 6 implements the crate's `transport::Transport` for HTTP and reuses the rest as it is:

| `Transport` | Standard I/O | Streamable HTTP |
|---|---|---|
| `request(id, message)` | A line written; the answer matched by id in `Pending` | A POST; the answer from its JSON body or its SSE stream, whose notifications and (legacy) requests go to the same `Inbound` handler, counted against the same `Budget`; a stream broken during `tools/call` returns `ClientError::MayHaveRun` |
| `send(message)` | A line, dropped when the server is not reading | A POST of a notification or an answer (legacy; modern sends none) |
| `cancel(id, reason)` | `notifications/cancelled` | Modern: nothing, since `Rpc::ask` drops the request's future, which closes its stream; legacy: a POST of `notifications/cancelled` |
| `agreed(era, version)` | Batches for `2025-03-26` | The `MCP-Protocol-Version` header, and the cancel's form |
| `closed()`, `shutdown(grace)` | The process | The GET stream ended and the session deleted (legacy) |

`Rpc`, `Session` (listing, calls, the change stream, which on HTTP is a POST whose SSE stays open), `Router`, `discover` and `initialize` are written over the trait. The HTTP start is its own: a 400 whose body is a modern error is passed to `discover` as that error, a 400 without one as any other error (legacy), and 401, 403 and 5xx as a failure that is not evidence of an era. Sending a listing or the discovery again after a broken stream, as the specification asks, is the HTTP part's.

## The tests, by rule

All on the manual fake timer of the test kit, on a runtime of one thread; no test waits for real time. `cineris-mcp::it` unless said.

| Rule | Tests |
|---|---|
| Messages read, ids never null, an error with no id accepted, a string echo of our id matched | `jsonrpc::tests::*` (unit); `calls::an_error_with_no_id_ends_nothing`, `calls::an_answer_with_the_id_written_as_a_string_of_its_digits_is_matched` |
| A batch only from a `2025-03-26` server; a batch line counted once, an empty one included | `calls::a_batch_is_read_only_from_a_server_of_the_revision_that_has_them`, `flood::empty_batches_count_from_a_server_that_may_batch_and_from_one_that_may_not` |
| Answers out of order, duplicates and unknown ids | `calls::answers_out_of_order_reach_their_own_calls`, `calls::a_duplicate_answer_and_an_answer_to_no_request_are_ignored` |
| A bounded number in flight | `calls::only_so_many_requests_wait_for_their_answers_at_once` |
| Cancellation in both eras, the late answer ignored; a request never sent never cancelled | `calls::a_cancelled_call_returns_at_once_and_its_late_answer_is_ignored`, `calls::a_call_given_up_while_it_waits_for_room_is_not_cancelled_on_the_server`; `transport::tests::a_request_given_up_takes_no_answer_and_the_end_reaches_every_waiter` (unit) |
| A call's and a listing's limit, then cancellation | `calls::a_call_past_its_limit_is_cancelled_and_says_it_timed_out`, `calls::a_listing_past_its_limit_is_cancelled_and_says_it_timed_out` |
| Every message that answers nothing counted, 1,000 or 16 MiB a minute, the server stopped and others going on | `flood::a_server_that_sends_notices_in_a_loop_is_stopped_and_another_server_goes_on`, `flood::every_kind_of_message_that_answers_nothing_counts`, `flood::the_allowance_is_a_minutes`, `flood::sixteen_mib_of_notices_in_a_minute_is_too_much`; `transport::tests::the_budget_ends_at_one_message_too_many_or_one_byte_too_many` (unit) |
| A line over the cap ends the connection; what is not a message is left | `flood::a_line_longer_than_the_cap_ends_the_connection`, `flood::what_is_not_a_message_is_left_and_the_connection_goes_on` |
| The probe: modern found, any other error, another kind of result or silence legacy, the late answer ignored, never cancelled | `probe::a_modern_server_is_found_by_the_probe_and_is_never_sent_initialize`, `probe::a_legacy_server_that_refuses_the_probe_with_any_error_is_initialized_on_the_same_process`, `probe::a_server_that_answers_the_probe_with_a_result_of_another_kind_is_legacy`, `probe::a_silent_server_is_taken_as_legacy_after_the_probes_time_and_a_late_answer_is_ignored` |
| A modern server slower than the probe | `probe::a_modern_server_slower_than_the_probe_to_start_is_found_by_its_late_answer`, `probe::a_modern_server_whose_first_answer_was_lost_is_asked_again_once_it_refuses_initialize` |
| A process that exits on the probe started again | `probe::a_server_that_exits_on_the_probe_is_started_again_and_only_initialized`, `probe::a_server_that_refuses_the_probe_and_then_exits_is_started_again`, `probe::a_server_that_cannot_be_started_again_ends_the_start_with_why` |
| `-32022` asked again once; the modern errors never fall back | `probe::a_server_that_does_not_speak_the_revision_asked_for_is_asked_once_more_with_one_it_does`, `probe::a_modern_server_that_speaks_no_revision_of_the_clients_is_never_sent_initialize`, `probe::the_other_modern_errors_end_the_start_without_initialize` |
| A legacy revision the client does not speak ends the start | `probe::a_legacy_answer_of_a_revision_the_client_does_not_speak_ends_the_start` |
| The whole start within its limit; `initialize` never cancelled | `probe::a_silent_server_still_starts_within_the_start_limit_and_initialize_is_never_cancelled` |
| A known era | `probe::a_server_known_to_be_legacy_is_initialized_without_the_probe`, `probe::a_server_known_to_be_legacy_that_refuses_initialize_is_probed`, `probe::a_server_known_to_be_legacy_that_exits_on_initialize_is_started_again_and_only_initialized`, `probe::a_modern_server_whose_first_answer_was_lost_is_asked_again_once_it_refuses_initialize` (known modern) |
| Legacy: `ping` at any time, other requests `-32601`, unknown notifications left, list changes | `legacy::a_ping_is_answered_during_the_handshake_and_after_it`, `legacy::any_other_request_of_a_legacy_server_is_answered_that_the_method_is_not_offered`, `legacy::a_list_change_from_a_legacy_server_bumps_the_changes` |
| Changes coalesce; one listing in flight, shared unless a change came | `legacy::a_burst_of_changes_wakes_a_receiver_that_was_not_looking_once`, `legacy::listings_asked_for_while_one_runs_share_it`, `legacy::a_change_during_a_listing_makes_who_asks_after_it_list_again` |
| Every page, an empty cursor, both eras; the caps | `legacy::a_listing_reads_every_page_in_both_eras_an_empty_cursor_included`, `modern::a_listing_past_twenty_pages_or_past_512_tools_is_refused`, `modern::a_listing_larger_than_sixteen_mib_is_refused` |
| A task-only tool unusable, kept as sent | `legacy::a_tool_that_runs_only_as_a_task_is_listed_as_sent_and_said_to_be_unusable`; `wire::tests::a_tool_that_runs_only_as_a_task_or_has_no_object_schema_is_unusable` (unit) |
| Modern `_meta` | `modern::every_modern_request_carries_the_revision_the_capabilities_and_who_the_client_is` |
| A listing's lifetime | `modern::the_end_of_a_listings_lifetime_bumps_the_changes`, `modern::a_lifetime_of_zero_or_less_or_none_bumps_nothing`, `modern::a_new_listing_replaces_the_last_ones_lifetime`, `modern::a_listings_lifetime_is_its_shortest_pages` |
| `input_required` and unknown result types | `modern::a_call_the_server_needs_sent_again_is_sent_with_a_new_id_and_its_state_given_back`, `modern::a_server_that_keeps_asking_for_the_call_again_is_refused_after_three_rounds`, `modern::a_server_that_asks_for_input_the_client_never_offered_or_gives_no_state_is_refused`, `modern::a_result_of_a_type_the_client_does_not_know_is_refused` |
| A modern server's request a fault | `modern::a_modern_servers_request_is_a_fault_counted_and_never_answered` |
| The change stream | `modern::the_change_stream_is_acknowledged_and_only_its_own_notices_bump_the_changes`, `modern::a_change_stream_the_server_ends_is_opened_again_and_the_changes_bumped`, `modern::the_change_stream_has_no_time_limit`, `modern::a_change_stream_acknowledged_without_the_notices_asked_for_is_ended_and_not_asked_again`, `modern::a_change_stream_answered_with_an_error_is_over_and_not_asked_again`, `modern::a_cancel_naming_another_request_leaves_the_change_stream_open`, `modern::an_acknowledgement_of_another_stream_neither_acknowledges_nor_refuses_the_open_one`, `modern::shutdown_ends_the_change_stream_and_then_the_process`; only for a modern server: `legacy::a_legacy_server_is_never_asked_for_a_change_stream` |
| Results and errors | `calls::a_call_returns_its_content_its_structured_result_and_whether_the_tool_failed`, `calls::an_error_of_the_protocol_is_the_servers_error`; `wire::tests::*` (unit) |
| A crash: a call written may have run, one never written did not; `closed()` with the exit's words; what was written before the exit read | `calls::a_crash_during_calls_says_each_one_written_may_have_run_and_closes_the_client`, `calls::a_call_still_waiting_for_room_when_the_server_crashes_never_ran`, `calls::a_crash_while_nothing_is_asked_closes_the_client_with_the_exit`, `calls::an_answer_written_just_before_the_exit_reaches_its_call`, `calls::an_answer_a_pipe_hands_on_after_the_exit_reaches_its_call` |
| Shutdown and drop | `calls::shutdown_closes_the_input_and_gives_the_process_its_grace`, `calls::a_dropped_client_shuts_its_server_down_without_waiting` |

The breaks are in [mutations/step-30-pr4-client.breaks](../../../../mutations/step-30-pr4-client.breaks): 53, each naming the test that must catch it. Some make a test wait for what never comes, which nextest's limit then fails.

## Left open, and decided here

- **A tool entry with no name fails the whole listing** as a protocol error, since it cannot be named in `cineris mcp inspect`; a tool whose input schema is not an object is listed, and `unusable()` says why. Duplicate names within a server are left to admission (pull request 2).
- **An error with no id is accepted and left:** it cannot be matched to a request. A probe answered so waits out its 3 s and falls back.
- **A modern server's late discovery result is taken while `initialize` waits,** and `initialize`'s own answer, when it comes, is a stray and counted; `initialize` is not cancelled (a legacy server's rule, and a modern server answers it with an error anyway).
- **`ServerProcess::shutdown` may be called twice** for one process: once by the reader of a server stopped as misbehaving, and again by the lifecycle's `shutdown()`. Part 1's process must take that.
- **The lifetime's end bumps `changes()`** as the brief asks; the specification says a client should not poll by `ttlMs`, so the lifecycle should relist lazily on such a bump.
- **Untested on Windows:** nothing in this part is specific to a system; the tests run on both on the runners. The real pipes (part 1) are not under these tests.

## The full review

An independent review of this part (no high findings, 4 medium, 5 low, and 6 breaks no test caught), each finding fixed with a test that fails without the fix, and each new rule broken in the list. Two of the reviewer's tests are kept as they were written, with their hand-built servers: `probe::a_modern_server_slower_than_the_probe_to_start_is_found_by_its_late_answer` and `calls::an_answer_a_pipe_hands_on_after_the_exit_reaches_its_call`.

| Finding | Fix | Tests | Breaks |
|---|---|---|---|
| **M1.** A `2025-03-26` server's empty batch (`[]`) spent nothing, so it could send them without end | A batch line is counted once with its bytes before its messages, whatever it holds | `flood::empty_batches_count_from_a_server_that_may_batch_and_from_one_that_may_not` | `an-empty-batch-is-not-counted` |
| **M2.** A modern server slower than the probe's 3 s to start (a cold `npx` or `docker run`) never connected: its late discovery result was dropped as a stray, and it refused `initialize`. The same for `connect_known(Era::Modern, …)` | The probe is sent by hand and still heard while `initialize` waits: a modern answer then makes the server modern; and a server silent on the probe that refuses `initialize` is asked once more | `probe::a_modern_server_slower_than_the_probe_to_start_is_found_by_its_late_answer`, `probe::a_modern_server_whose_first_answer_was_lost_is_asked_again_once_it_refuses_initialize` (both ways of connecting) | `a-late-discovery-is-not-heard`, `a-server-that-refuses-initialize-after-a-silent-probe-is-not-asked-again` |
| **M3.** A call written and then lost to a crash said `Stopped`, as if it had never been sent | The transport fails a request it wrote with `MayHaveRun`; one never written stays `Stopped`; listings, the probe and the handshake read either as `Stopped` | `calls::a_crash_during_calls_says_each_one_written_may_have_run_and_closes_the_client`, `calls::a_call_still_waiting_for_room_when_the_server_crashes_never_ran` | `a-call-lost-to-a-crash-is-said-not-to-have-run` |
| **M4.** `Launch::Local`'s docs said `cwd` is the workspace when absent and that `write` allows writes to the workspace | Rewritten to points 3.1 and 5: the workspace only in a trusted project, an empty folder of the server's own otherwise; `write` never true in this step, refused by the spawner. The fields are kept | Docs only | None |
| **L1.** On the process's exit, only what was in the channel already was read, so a last answer still in the pipe's reader was lost | After the exit, the output is read until it ends, for the shutdown grace at most; the fake can hand a line on after its exit | `calls::an_answer_written_just_before_the_exit_reaches_its_call`, `calls::an_answer_a_pipe_hands_on_after_the_exit_reaches_its_call` | `what-is-written-before-the-exit-is-lost` (B3) |
| **L2.** A listing's bytes had no cap | 16 MiB of definitions (512 × 32 KiB), every page counted, written size measured without writing | `modern::a_listing_larger_than_sixteen_mib_is_refused` | `a-listing-of-any-size-is-taken` |
| **L3.** `connect_known(Era::Legacy, …)` that saw the process exit during `initialize` probed the new process | It starts it again and sends `initialize` alone | `probe::a_server_known_to_be_legacy_that_exits_on_initialize_is_started_again_and_only_initialized` | `a-known-legacy-server-that-exits-is-not-started-again` (B5) |
| **L4.** 13-mcp and decision 0010 stated the interoperability check as done, and 0010 priced our client at "a tenth or less" | Both say it is being added (part 4b); 0010 gives the note's 9 to 21% of `rmcp`'s | Docs only | None |
| **L5.** Dropping the client outside a runtime only closed the input, which the docs did not say | `Client`'s doc says what a drop does in each case: inside a runtime, `shutdown` without waiting; outside one, the input closed and the process left to its own handle | Docs only | None |
| **B1.** A `notifications/cancelled` naming another id ending the change stream: no test | | `modern::a_cancel_naming_another_request_leaves_the_change_stream_open` | `a-cancel-of-another-request-ends-the-change-stream` |
| **B2.** An acknowledgement of another stream acknowledging or refusing the open one: no test | The fake can hold its acknowledgement behind a gate | `modern::an_acknowledgement_of_another_stream_neither_acknowledges_nor_refuses_the_open_one` | `an-acknowledgement-of-another-stream-is-taken` |
| **B4.** A probe answered with a result that is not a discovery result: no test | | `probe::a_server_that_answers_the_probe_with_a_result_of_another_kind_is_legacy` | `a-result-of-another-kind-to-the-probe-is-not-legacy` |
| **B6.** `subscriptions/listen` for a legacy server: no test | | `legacy::a_legacy_server_is_never_asked_for_a_change_stream` | `a-legacy-server-is-asked-for-a-change-stream` |
| **Lesser.** The last listing's lifetime aborted; the least of the pages' lifetimes; `Rpc::ask`'s `sent` | | `modern::a_new_listing_replaces_the_last_ones_lifetime`, `modern::a_listings_lifetime_is_its_shortest_pages`, `calls::a_call_given_up_while_it_waits_for_room_is_not_cancelled_on_the_server` | `a-listings-old-lifetime-is-kept`, `a-listings-lifetime-is-its-longest-pages`, `a-request-never-sent-is-cancelled` |

The breaks of the start that moved with M2 and L3 were moved with the code (`the-probe-is-cancelled` now adds a cancel to the silent path, since the probe is sent by hand and has none to take away).
