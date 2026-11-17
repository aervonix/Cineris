# Step 27, its parts: the socket

The second of step 27's three pull requests: `cortex serve --socket`, the checks on who may connect, several clients per session, roles, the backpressure across clients, and the changes to `cortex-core` they need ([answered.md](../answered.md), "The design, before building", points 1 to 4, with the coordinator's answers and the owner's). What it does is in [architecture/11-rpc-protocol.md](../../../architecture/11-rpc-protocol.md), "As built in step 27".

## What was built

- **`cortex serve --socket [PATH]`** (`cortex-rpc`'s `socket` and `serve_socket`, `cortex-cli`'s `serve`): each client a connection of its own, at most 64 at once, each given 10 s to say `initialize`; a client's `shutdown` ends them all. A connection gives its place back when it stops serving; an `accept` that fails for want of a resource is tried again; the socket is removed first at the end.
- **Unix** (`socket/unix.rs`): the folder secured before the socket exists, the socket `0600`, a lock beside it for the server's life, a stale socket removed only when nothing answers on it, every peer's credentials checked, and a peer whose line of parents reaches the server, or cannot be read, refused.
- **Windows** (`socket/windows.rs`, [decision 0037](../../../decisions/0037-the-named-pipe-is-served-from-cortex-rpc-in-one-windows-only-module-that-may-use-unsafe.md)): a named pipe owned by the user, admitting the user alone at the server's integrity (`NWNR`), each instance made with its security, the next made before one is handed on; each connection's token checked on its first data; the server's own commands refused through a job the server joins at its first bind.
- **The server's options:** `serve --socket` keeps sessions past their clients (`CoreOptions::keep_sessions`); a supervisor connection is given `ServeOptions::supervisor_grant` (none in the binary); `unrestricted` is the server's command line's to allow (`CoreOptions::allow_unrestricted`, off in `serve` unless the server is started at it).
- **`cortex-core`:** `keep_sessions` and `allow_unrestricted`, each changing nothing unless a front end sets it; the client's queue bounded by the bytes of its deltas (8 KiB a place of its capacity); supervisor routing (`Core::connect_with_grant`), inert without a grant; the close rules; `session/list`'s cursor by the last summary, and `session/read`'s tied to its session and to its place in the log.
- **The protocol:** `invalid_cursor` and `unrestricted_not_allowed` among the protocol's own codes. No type changed; the schema and the golden values are as they were. Step 24's `session/artifact` and `workspace/warm` run in the conformance scenarios.
- **The open item** [sessions-kept-with-no-client](../../../outstanding/sessions-kept-with-no-client.md), with what a kept session costs.

## Where it departs from the design

| Design | As built | Why |
|---|---|---|
| A pipe's client checked when it connects | Checked on its first data, which is held and handed on first | Windows gives the server the client's token only once it has read what the client wrote (`ERROR_CANNOT_IMPERSONATE` before; try run 43936070154), and the token is the writing thread's |
| The label `NW` (no write up) | `NWNR` | A client is checked only once it has written: one that could open the pipe to read and never write could hold connections it could never be refused on |
| `socket/windows.rs` through tokio's `ServerOptions` | `CreateNamedPipeW` | `ServerOptions` needs a runtime to make an instance; binding needs none |
| A job per listener | One per process, joined at the first bind | A process stays in a job for good; a second would only nest |
| `ClientInfo::grant` | `Core::connect_with_grant(info, grant)` | A field would break every `ClientInfo { .. }` literal of other crates |
| An engine on `standing.narrow(grant)` decides | The scope from `standing.narrow(grant)`; no read or write allow rule of the grant ever; its exec allow rules and its level (at most `autonomous`) only where the narrowed sandbox is the session's own and confines commands, and otherwise at most `supervised` with no exec rule, so never on Windows; the judge built when the answer comes; an expired grant covers nothing; the session told, when a supervisor with a grant joins it, what of the grant does not apply there and why | `narrow` keeps only the allow rules both leases share, so nothing the session asked about could ever be within; the rest is the reviews' (below) and the coordinator's ruling |
| The supervisor told what of its grant does not apply | An `info` notice to the whole session | A notice to one client would need an event outside the session's numbering, a protocol change |
| A queue bound of 8 MiB | 8 KiB a place of the client's capacity: 8 MiB at the default 1,024 | It scales with what the server gives a client |
| `unrestricted` refused by the server on the request's text | Refused by the core, at one gate (`may_hold`) every level passes: as a session is put together, and as an engine is put in force | The reviews found the text could be spelled past, and that a reload could reach it |

## The "Done when" items this part covers

| Item | Tests (each runs on Linux and Windows unless it says otherwise) |
|---|---|
| A slow client gets merged and dropped deltas with `item.resync`, bounded memory, and no stall of the model stream | `cortex-rpc::it wire::a_slow_client_is_sent_merged_deltas_and_a_resync` (over the wire, beside a client that takes everything); `cortex-core::it events::a_reader_that_stops_during_one_long_item_holds_bounded_bytes_and_one_resync`; `cortex-core events::tests::a_stopped_reader_holds_a_bounded_number_of_bytes_of_one_long_item`, `the_bytes_held_go_down_as_the_reader_takes_deltas`; the closing rule's `wire::a_client_that_takes_nothing_is_closed_and_the_session_goes_on` |
| With a supervisor attached, approvals route to it first, it cannot answer beyond a test lease, and a user client's answer wins | `cortex-rpc::it wire::a_supervisor_cannot_answer_beyond_the_servers_grant_and_the_user_does` (the server's grant over the wire); `cortex-core::it supervising::` `a_supervisor_allows_a_call_within_its_grant_once_and_nothing_is_recorded`, `a_call_beyond_the_grant_is_not_taken_from_the_supervisor_and_the_user_answers`, `a_users_answer_first_settles_it_and_the_supervisors_later_answer_is_refused`, `a_session_has_at_most_one_supervisor`, `an_expired_grant_takes_no_allow`, `a_reload_that_tightens_the_rules_reaches_an_open_requests_judge`, `a_supervisors_allow_is_judged_by_the_session_as_it_is_when_it_answers` (Linux); `cortex-core policy::tests::` `a_grant_lifts_no_question_about_a_path`, `a_command_rests_on_the_sandbox_only_where_it_runs_in_it`, `a_grant_never_widens_what_the_session_holds`, `an_expired_grant_covers_nothing` |
| A session survives its client disconnecting and can be re-attached | `cortex-rpc::it wire::a_kept_session_survives_its_client_and_another_opens_it`; `cortex-cli::system serve::sys_serve_socket_serves_at_the_default_place_and_removes_it` (the binary, Linux); `cortex-core::it lifecycle::with_keep_sessions_…` (six) |
| Several clients per session; roles | `cortex-rpc::it socket::two_clients_share_a_session_over_the_socket` (Linux), `pipe::a_client_of_the_user_is_served_over_the_pipe` (Windows); `wire::the_role_given_at_initialize_binds_the_client`; `cortex-core::it closing::` (an observer never ends a session; a closed view leaves its questions) |
| Socket permissions and peer checks | Linux: `cortex-rpc::it socket::` (the mode and the lock, a place in use and a stale socket, the folder, a silent connection and one too many, the server's own commands, a peer whose process is gone) and `cortex-cli::system serve::sys_another_user_is_refused_at_the_socket` (a second user through `sudo`, which must run where `CI` is set). Windows: `cortex-rpc::it pipe::` (the access list read back, the name in use, a lower integrity, a restricted and an anonymous token, `sys_another_user_cannot_open_the_pipe`, `sys_one_of_the_servers_own_commands_is_refused`) and `cortex-cli::system serve::sys_serve_socket_serves_on_a_named_pipe` |

## Measurements

- **The queue's bound by bytes,** measured before it was set, with a client that reads nothing (a release build): an answer of 32,768 tokens held 0.14 MiB in one merged delta, 131,072 tokens 0.57 MiB, 393,216 tokens (the catalog's largest `max_output`) 1.72 MiB; `timeout 3 yes` through the real runner 11.09 MiB after 3 s (3.69 MiB/s). 8 MiB is 4.6 times the longest answer and about 2.2 s of a command writing as fast as it can.
- **A kept session** with no turn costs about 236 KiB of the server's resident memory (40 sessions took the desktop's debug build from 27.7 to 36.9 MiB).

## The independent reviews

The first review read the whole pull request and confirmed two defects with tests of its own; the coordinator asked it to read the supervisor's grant with four questions (can a grant widen the scope, reach a call the session's rules deny, bring `unrestricted`, or be recorded). Every finding was fixed with a test that fails without the fix and a break the test catches.

| Finding | Fix |
|---|---|
| A client could have `unrestricted` from a server that does not allow it, by spelling the override's key otherwise (`"permissions" . mode`), since the server compared the key's text | Refused by the core on the configuration as it is assembled (`CoreOptions::allow_unrestricted`), for `session/new`, a stored session's open, `session/fork` and a raise by `session/set`; the server's own check is gone |
| A grant's allow rules reached paths beyond the session's narrowed scope: a write outside the workspace, a secret, a protected path | A grant lifts no question about a path: its read and write allow rules are not used |
| A command allowed by the grant's level ran in the session's sandbox, wider than the one it was judged against | The level is at most `supervised` unless the narrowed sandbox is the session's own and confines commands; below that only an explicit exec rule covers a command |
| The supervisor's judge was fixed when the question was put | Built when the answer is judged, from the template, the engine in force and the sandbox's state as they are then |
| A grant's expiry was not judged | An expired grant (its `at`, or a `time` stop) covers nothing |
| `session/read`'s cursor was not tied to its session or branch | The cursor names the session and the last item given; another session, a count past the end, or a last item no longer at its place is refused `invalid_cursor` |
| The Linux check of the server's own commands admitted a peer whose process was gone (a command connects, hands the socket to a child, and exits) | A peer whose line of parents cannot be read, or whose process is not known, is refused |
| The Windows residual risk understated how a command escapes the job | security.md, decision 0037 and the module say the check is thin, and that the person approving is the wall |
| Connections that had ended were counted against the cap | A connection gives its place back when it stops serving |
| Any `accept` error ended the server | One for want of a resource is tried again after a doubling wait |
| The end could be cut before the socket was removed | The socket is removed first |

The second review read the fixes alone, and confirmed two defects with probes of its own. The coordinator asked for the level's paths to be found by mechanism, not by spelling: every way a session's level is set as it opens or after.

| Finding | Fix |
|---|---|
| `config/reload` put a session at `unrestricted` on a server that does not allow it: with a profile at it, or after the user's file changed | One gate, `Core`'s `may_hold`, on the configuration as it is assembled and at the top of `put_engine`, which every engine put in force passes. Each path has a test: `session/new` (its level, an override in any spelling), a stored session's `session/open`, `session/fork`, `session/set`, `config/reload` (a profile, a changed file, every session of a client, the look at a turn's start), and trust taken away in the same reload, which still holds |
| A grant's exec allow rule covered a command that writes anywhere where commands are not confined, or with a narrower scope (`cp x ~/.bashrc`, `sh -c …`, `python3 -c …`): the engine asks only about the protected and secret paths a command names | A grant's exec rules apply only where the narrowed sandbox is the session's own and confines commands; the session is told what of a grant does not apply. 06's line on the paths a command names now says it holds for protected and secret paths only |
| Refusing a peer for any parent `/proc` would not show refused every client of an SSH session where other users' processes are hidden (`hidepid`) | Only the peer's own process must be readable; a hidden parent ends the line elsewhere |
| A refused `session/fork` left its stored log behind | The fork's level is judged before the fork is written |
| Two of the core's new tests failed on Windows, and one ran `make` through PowerShell in tier 2 | The tests use the system's own paths; the one that runs a command is `sys_`, on Linux, where a command can be within a grant |

A race the first round of the core work found, a client that opened the session while an approval was being put, has a test that stops the turn between the two with an awaited point (`approvals::a_client_that_opens_the_session_while_an_approval_is_being_put_is_put_it`), as the coordinator asked.

