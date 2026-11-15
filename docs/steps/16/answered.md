# Answered in step 16, on 2026-11-15

Step 16 built the local statistics in `cortex-stats`. It also built `cortex stats` and `cortex usage`, `usage/report` and the opt-in `usage/refresh`, model roles with a budget each, variant points fixed per epoch, the per-day limits, and, at the owner's request, the split of every turn's time into the parts it was spent on. Its prompt asked it to settle five points. The coordinator gave the owner's answers.

## The five points

| Point | Settled | Recorded in |
|---|---|---|
| **1. How records leave the loop.** | **A bounded queue and a writer thread of the statistics' own** (`cortex_stats::Recorder`). The thread is not the log's. On the loop, `record` converts the event and makes one `try_send` of a boxed record. A full queue drops the event and counts it, and never blocks. The count is written as an event of its own (`kind: "dropped"`) when there is room. (A limits update dropped is counted in the same number: corrected after the independent review.) Making a recorder makes the queue and nothing else. The core starts the thread on the blocking pool when the first session opens, and flushes it at `shutdown`, because a detached thread dies with the process. Tested with a writer that never runs: the queue of four takes four events of ten and counts six, and none of the ten calls waits. | [14-adaptation.md](../../architecture/14-adaptation.md), "Local statistics", as built in step 16; [decision 0021](../../decisions/0021-local-statistics.md), amended |
| **2. Which calls go to which role.** | **One utility call exists after step 15: the compaction summary.** It goes to `fast`, in the background or not. With no `fast` model it goes to `main`. When the `fast` model refuses it as too long, or fails it in any way but an interrupt, the summary is asked once of `main`, and `fast` is not asked again in that epoch. `judge` has no call before the run engine. Its model is resolved, priced and shown, and it falls back to `main` in the same way. Titles, summaries of long output and proposed profile entries are not built; each goes to `fast` when it is. The routing is one function (`Models::for_call(UtilityCall)`), with the table in its documentation. | `cortex_agent::config::UtilityCall`; 14-adaptation.md, "Model roles and routing" |
| **3. The usage refresh.** | **Built for OpenRouter, DeepSeek and Venice.ai. There is no refresh for Anthropic.** Its usage, cost and rate-limit endpoints are Admin API endpoints ("workspace API keys don't work"), so they take another kind of credential, and the brief said to build nothing for one. The owner approved this on 2026-11-15, with five conditions, each met and tested (the next table). | "The refresh", below; [18-usage.md](../../architecture/18-usage.md), "The opt-in refresh" |
| **4. What a variant point is.** | **A named choice about how a model is addressed.** Its set of values is one this build knows. The catalog gives each model a default, and the user may override it for a model (`[adapt.variants."<model>"]`). The choice is fixed for a context epoch and recorded in the epoch's `epoch` entry (`variants`, an addition to the log's format) and on every outcome: each request, tool call and turn event carries the set. The aggregates count outcomes by model, variant set and tool. The points shipped: `edit_format` (`replace`), `tool_schema` (`strict`, where the model can be held to it, or `loose`), `prompt_style` (`concise`, or `guided`, which adds a section of tool guidance to the system prompt, `cortex_agent::GUIDED_NOTES`) and `effort` (the model's levels; a role's own `effort` wins over the variant's). An unknown choice stops the session from starting (`unknown_variant`). A change applies from the next epoch that builds a prompt. An epoch begun by a change of model keeps the prompt's style and takes the new model's tool and edit choices. | 14-adaptation.md, "Adapting to the model", as built; [07-sessions-and-context.md](../../architecture/07-sessions-and-context.md), "Context epochs" |
| **5. Retention and pruning.** | **Monthly files by the UTC month of the event, kept for `stats.retention` months after their own (**3 by default**: the coordinator's decision on the conflict below, 2026-11-15, which the owner may revisit; the brief's design said 12). Day buckets older than 400 days fold into month buckets.** Each monthly file has its own part of the aggregates, with the byte it was read to. A file past its retention moves its part into `kept` before it is deleted, so all-time totals outlive it. A rebuild from the files gives the same values, which is tested. **Disk use, measured:** a heavy user's synthetic day (3,000 requests, 4,500 tool calls and 200 turns) is 3.2 MB of events, and the per-request context records add about a third again. That is about 4.4 MB a working day and 90 MB a month: **about 1.1 GB at a year's retention, and about 360 MB at the default of three months** (the current month and three after it). The aggregates are 51 KB for a month and stay under 1 MB at 400 days of day buckets, and they keep every all-time figure, so the default can be short. A more usual heavy day of 800 requests is about 100 MB at the default. | 14-adaptation.md; [08-configuration.md](../../architecture/08-configuration.md) |

## The refresh

What each provider documents, read on 2026-11-15:

| Provider | Endpoint | Key | Figures read | What is never kept |
|---|---|---|---|---|
| Anthropic | `/v1/organizations/usage_report/messages`, `/cost_report`, `/analytics/*`, `/rate_limits` | **An Admin API credential**: an `sk-ant-admin` key, an `org:admin` OAuth token, or a personal or service-account key not scoped to a workspace. Workspace keys don't work. | none: **no refresh** | |
| OpenRouter | `GET <base>/key` | the ordinary key | `limit`, `limit_remaining`, `usage`, `free_model_daily_requests` | `label` (a shortened form of the key), `creator_user_id`, `organization_id`, `workspace_id`, budget ids. `GET /credits` takes a management key and is not used. |
| DeepSeek | `GET <base>/user/balance` | the ordinary key | `is_available`, and per currency `total_balance` | nothing to drop |
| Venice.ai | `GET <base>/api_keys/rate_limits` | an inference or an admin key, for the key making the request | `accessPermitted`, `balances`, `nextEpochBegins`, `rateLimits` | nothing to drop. `/billing/*` and `/rate_limits/log` are admin-only and are not used. |

**The owner's conditions, 2026-11-15, and how each is met.**

| Condition | Met by | Tested by |
|---|---|---|
| Venice.ai's `nextEpochBegins` shown as what it is | It is `ProviderLimits::next_epoch`, printed as "next epoch …", and no limit's `resets_at`. That is our assumption; the documentation does not say it resets the daily limits. **The live refresh could not settle it:** the desk's key has no per-day limits, only per-minute ones. | `openai_chat::endpoints` (the break `the-epoch-is-a-limits-reset`) |
| No credential past its own host | The shared transport follows no redirect (`Policy::none`). A 3xx is a failed refresh. | `usage::a_redirect_takes_the_key_to_no_other_host`, against two scripted servers |
| A failed refresh changes nothing stored | Only successful outcomes are written. A timeout (20 s, on the core's timer, for the send and the body), a 401, a 5xx or an answer that does not parse leaves `limits.json` byte for byte. The error names the provider and the status. The answer's text is not carried: not its message, not a code that is not a plain one, not its request id (the last two were found by the early review). | `usage::a_failed_refresh_changes_nothing_kept_and_says_why_without_the_answer` |
| `stats/limits.json` owner-only, written atomically | `cortex_stats::store::write_private`: `write_atomic` with `owner_only`, following no link. A read that fails for any reason but the file's absence refuses to write, rather than dropping the other providers' figures. | `store::every_file_is_owner_only`; the core's refresh test |
| One live refresh per provider, shapes only | `cargo xtask live --refresh PROVIDER`, new, which asks the endpoint once and prints which figures came back. | below |

**The live refresh, by hand, 2026-11-15** (free; amounts and ids are not recorded):

| Provider | Status | What came back |
|---|---|---|
| OpenRouter | 200 | No "available". A balance in USD with remaining, limit and used. One per-day request limit on the key, with what is left. No epoch. |
| DeepSeek | 200 | "Available" present. A balance in USD with remaining only. No limits. No epoch. |
| Venice.ai | 200 | "Available" present. Balances in `BUNDLED_CREDITS`, `DIEM` and `USD`, remaining only. 255 per-minute request limits and 115 per-minute token limits, each for one model and none with what is left. "Next epoch" present. |

A model's own limits (Venice.ai's) are not kept: a limit window names no model.

## From step 15

| Item | Settled | Recorded in |
|---|---|---|
| `fast` wired only for summaries; other utility calls, `judge`, budgets by role, variants, the context record | Point 2 and point 4 above. Budgets by role: below. `StatEvent::Context` is stored in the monthly files (`kind: "context"`) for the context profiler, and is not added to the aggregates. A request the provider refused as too long now leaves its record, which settles the item step 15 left for it (`the-context-record-of-an-overflowed-request`). | 07, "Budgeting"; 14 |
| **Which turn owns a background summary's usage** | **The turn that counts it**, at its first boundary after the request ended. That turn's usage, its `turn.completed`, its account of the ledger and the log's `usage` entry already had it, and the statistics event now agrees with them. The event names the turn that started it (`started_in`). A summary stopped part-way is kept and counted at the next boundary for what the provider said it used. One still running when the session closes is stopped and counted when the loop ends, in the last turn, in the ledger and the statistics, with no log entry: the turn has ended. A failure to log one summary no longer drops the others. This settles `a-summary-stopped-or-left-is-not-counted`. | 07, "In the background" |
| **Should a counted summary teach the ratio** | **Yes, its own role's.** The bytes sent against the input counted set that role's ratio and answer an earlier refusal (`Budget::learned`). The base the main model's next estimate goes on from stays the main request's own count. | 07, "Budgeting" |
| **The per-role seam** (`budget_for`, `budget_for_mut`) | **A budget of each role's own.** `main`'s is the context's. A utility role gets one the first time a request of its teaches it something, seeded with the main model's ratio, and is estimated at that ratio until then. A `fast` refusal marks only the `fast` budget, so the main model's retry of the summary is sent as its own estimate allows. This settles `a-fast-roles-refusal-cuts-the-main-models-summary`; three tests of step 15 that had pinned the shared budget were changed to say the new rule. | 07, "Budgeting" |
| `ContextStat.difference` at the speed sent | Kept as is, and stored. | |
| Effort aliases on a budget-reasoning model | A variant's `effort` goes through the same mapping, so `reasoning_budget_too_large` is refused at session start for it too. | `provider::variants_for`, `role_model` |
| **Budget summaries reserve the whole output** | **Left as it is.** The catalog cannot yet say which models refuse input plus `max_tokens` past the window (Anthropic's before Claude 4.5) and which do not. A flag for it would be a catalog fact read from each model's documentation, which no step has read. Relaxing the reserve without one could make every summary on such a model fail. It costs summary length, not correctness. | already in [outstanding](../../outstanding/a-reasoning-budget-request-past-the-window-on-older-models.md) |
| The new notices, `i_N`, `FileTable::touched()` | Unchanged, and used as they are. | |

## Added by the owner: where a turn's time goes

**The parts.** Every moment of a turn is in exactly one part, given by what the turn was waiting for (`cortex_agent::timing`):

- **The model:**
  - `connect`: from sending to the provider's answer;
  - `first_byte`: from the answer to the first event;
  - `streaming`: tools started early run inside it;
  - `retry`: a retry's wait, and after the wait it is the next attempt's `connect`;
  - `summary`: waiting for a summary at a compaction.
- **`tools`:** a tool round with a call running, shared equally among the calls running.
- **`approval`:** a round with no call running and a question open.
- **`verification`:** the slot, filled by step 19.
- **`harness`:** everything else.

Milliseconds are rounded where the parts lie end to end, so they add up to the turn's own `duration_ms` exactly. Per tool there are calls, the tool's share of the tools part, its running time, its time queued once allowed, and its wait for an answer.

**Where it goes.** It is in `TurnStat::times`, in the `turn` events, and in the aggregates by day, project and model, and by day, project and tool. It reaches `usage/report` as `UsageRow::time`, `UsageReport::time` and `UsageReport::tools`, and `cortex usage` prints it.

**The connection.** The client now says when the provider answered: `ModelEvent::Answered`, once per attempt, before that attempt's response, and only for a success status.

**The spans.** The loop opens `turn` with `session` and `turn`; `request`, for the turn's and the summaries', with `role`, `provider`, `model` and `attempts`; and `tool`, with `tool` and `call`, inside the request's or the turn's. This settles `spans-in-the-agent-loop`.

**The cost.** A span with no subscriber costs 2 ns. A change of part costs 2 ns. Recording one event on the loop costs a median of 371 ns.

## Choices made on the way

| Choice | Why | Recorded in |
|---|---|---|
| **The local day is fixed when an event is written**, from the zone: `TZ` or `/etc/localtime` (TZif, with its POSIX rule for the years past its table) on Linux, and the system's API on Windows (`cortex_base::zone`, the crate's second module that may use `unsafe`). | 18 says "UTC, with the local day it fell on", so a later change of zone does not move old events. No time-zone crate was added: the parsers are about 300 lines, tested on real zone files. | [01-workspace.md](../../architecture/01-workspace.md); `cortex_base::zone` |
| **Per-day limits** (`[limits.day]`: requests, tokens, money): one account shared by every session of a core, which begins with what the day's statistics already hold. They are read when a session with a day limit is put together, on the blocking pool, and never without one. | The ledger's accounts were built to be shared. A day's earlier spend is in the statistics and nowhere else. `limits.day.time`, and a session that runs past midnight, are [outstanding](../../outstanding/a-days-time-limit.md). | [03-agent-loop.md](../../architecture/03-agent-loop.md), "Loop guards and limits" |
| **`usage/refresh` is a new method,** for a `user` client only, and `ProviderLimits` gains `balances` (decimal strings, never added across currencies), `available` and `next_epoch`. | The owner's answer to the balance question. It settles `a-balance-in-the-usage-view`. | [11-rpc-protocol.md](../../architecture/11-rpc-protocol.md) |
| **The session range of `usage/report` is the open session's live figures.** A session that is not open is refused (`session_not_open`). `cortex usage` offers today, week, month and all. | 18 defines the range as "the open session, live". A stored session's figures would mean a scan of its log. | 18 |
| **The start-up test is a count of reads by directory** (`cortex_stats::reads`). | No tracing check of files opened at start-up exists yet; the coordinator asked for a test of the same shape. | `store::nothing_is_read_until_it_is_asked_for`; the core's `usage` test |

## The early review of the refresh

A reviewer read the refresh as soon as it worked. It found nothing that blocks.

| Found | Done |
|---|---|
| A failed answer's `error.code` and request id, which come from its body, reached the error shown | **Fixed:** a code that is not plain (lower-case, digits, `_.-`, at most 64) is shown as `refresh_refused`, and the request id and details are dropped. The test now puts the body's text in the code and the id. |
| A refresh wrote `limits.json` with `stats.enabled = false`, and the report could save the aggregates then | **Fixed:** with statistics off, nothing is written. A test covers it. |
| A provider listed twice was asked twice | **Fixed**, and tested. |
| A failed read of `limits.json` let the next update drop the other providers' figures | **Fixed:** the update refuses to write when the read fails other than by absence. |
| `usage.refresh` is read when the core starts, though declared "at once" | **Left** for step 24's refresh key with hot reload: the CLI starts a core for every command. |
| 18's example named `anthropic`, which has no refresh | **Corrected** in 18. |

## The full review, before the push

A reviewer that had not seen the work read the whole change against the specification. It found nothing that blocks, seven things to fix and some notes. Each fix has a test and a break.

| Found | Done |
|---|---|
| A summary the trigger was waiting for, stopped by an interrupt, was dropped uncounted: its task was raced by value | **Fixed:** the task is raced by reference and kept, and counted at the next boundary or when the loop ends. Test: `context::a_summary_the_trigger_waited_for_is_counted_when_an_interrupt_stops_it`. |
| The day's account took its budget from the first session of the day, so a project's lower day limit was lost (or held another project to it) | **Fixed:** one purse for the day (`Account::with_spent`), and each session an account of its own budget that spends from it (`Account::draws_on`). Test: `usage::each_session_holds_the_day_to_its_own_limits`. |
| A request's span stayed open until the next request replaced it, and the turn's with it | **Fixed:** the span is let go when its record is made, and at a turn's end. The spans test now checks the order spans close in. |
| A `fast` summary too large for the new epoch was asked of `fast` again | **Fixed:** it marks `fast` as turned away for the epoch, so the main model is asked. |
| The stored request record dropped `started_in`, and documented `turn` wrongly | **Fixed:** `RequestRecord::started_in`, and the doc of `turn`. |
| A turn's parts were cut a log write later than its `duration_ms` was taken | **Fixed:** one reading of the clock for the turn's start and its parts' start, and one for its end, its `duration_ms` and its parts' end. |
| `cortex stats`, and a report with statistics off, pruned old monthly files without saving | **Fixed:** `Aggregates::sync` prunes only when given a retention, which only callers that save give. Test: `usage::showing_the_statistics_prunes_nothing`. |

| Note | Settled |
|---|---|
| Tool records left the effort out of their variant set | **Fixed**: `ToolCallStat::effort`, so tools' and turns' outcome keys agree. |
| 14 still said the log's writer thread | **Corrected.** |
| "reset since" was printed for a reset still ahead | **Fixed**: only past a reset. |
| Nothing tested that the variants are fixed for an epoch | **Added**: `roles::variants_changed_between_two_openings_begin_a_new_epoch`, with a break. |
| An `epoch` entry with a choice this build does not know is read as one with none, and goes on | **Left**: a later build's choice cannot arise before a second edit format exists; the reader says so in its documentation. |
| The overflow record settles the refused request as the one sent | **Intended**: it was sent. The next request begins a new epoch, so the prefix check and the difference are of the new one. |
| `cortex stats reset` while another process writes | **Left**: the reset says what it removed, and a writer's later lines start a file again; a reset is a deliberate act at the keyboard. **Corrected after the independent review:** they did not. The writer kept the month's file open, so its later lines went to the file the reset had deleted, and were lost. It now opens the file the name gives before each line ("After the review", below). |
| A request with no usage counts as unpriced | **Left**: not shown anywhere. |

## Breaks and stress

- **`cargo xtask mutate step-16`:** all 41 breaks of `mutations/step-16.breaks` are caught by the tests they name. The run was on the desk's side slot at two jobs, with the last break run alone. The first two runs at four jobs never got past their baselines: file tests of `cortex-base`, `cortex-config` and `cortex-stats` all timed out at the same moment with nothing broken, which is the desk's disk stalling under four copies. A break that first went uncaught (the main model asked after a `fast` failure at the trigger, with no summary in the background) has its own test now.
- **`cargo xtask stress -p cortex-stats -p cortex-agent -p cortex-core --count 50`:** 49 of 50 passed. The one failure was a timeout, with no output, of `store::the_aggregates_rebuild_to_the_same_values_from_the_files` under seed 46, during a burst of load on the desk. It did not happen again in 200 repeats on the laptop, nor in 300 repeats of `cortex-stats` on two cores with load added (`--cores 2 --load 6`).
- **Spend:** none. The three live refreshes are free and are not in the spend ledger.

## After the review, 2026-11-15

An independent review of step 16 found nine defects, four rules no test held, two open items worse than described, docs that disagreed with the code, and seven notes of lower confidence. Each defect has a test that fails without its fix and a break in `mutations/step-16-fixes.breaks`. The review's own failing tests are kept, renamed to fit their crates.

### The findings

| Found | Done | Tests |
|---|---|---|
| **1. A day's limits held per process.** The day's purse read the statistics once, at a core's first day-limited session, then counted only its own spending: two processes with a limit of one request spent two. | **Fixed: across processes.** Before each request a day-limited session makes, the purse reads what the monthly files gained since it last looked (`cortex_stats::DayReader`), on a reader thread of its own, and the request waits for it at most a second (`cortex-core`'s `day`, `CATCH_UP_LIMIT`; the log says once when it took longer). A thread and not the blocking pool, because the runtime's end waits for the pool, and a read stuck on a stalled disk would hold the process's exit. Each request line says which day's purse it was charged to (`charged_to`: the recorder's id, unique to the process, and the day), so a purse skips exactly the lines of the requests it counted as they were made; every other line counts. The reader holds each file it reads open, so that on Linux a file made again after a reset cannot take the old one's inode number and pass for it. The statistics hold what the ledger was charged: a request's line carries what its attempts given up used, and an interrupted request has a line of its own (the early review below). A shared locked file was the other way: it would be a second record of our own numbers (18-usage's rule 2), written with statistics off, and locked on every request in every process. **Cost per request,** measured: median 28 µs, 95th percentile 37 µs. **The bound,** in [18-usage.md](../../architecture/18-usage.md), "Limits on one day": the requests other processes have in flight when the limit is reached, at most two for each session running there, and the tokens and money they use. | `day::two_processes_together_hold_the_day_to_its_limit`, `a_line_an_older_build_wrote_counts_as_another_processs`, `a_processs_own_spending_is_not_counted_twice`, `a_stalled_disk_holds_a_days_request_at_most_a_second` (`cortex-core`); `day::*` (`cortex-stats`), with `sys_two_processes_have_recorders_of_their_own`; `interrupt::an_interrupted_request_is_recorded_with_what_it_used`, `retries::what_a_discarded_attempt_used_is_still_counted` (`cortex-agent`) |
| **With statistics off,** a day's limit would hold per process. | **The owner's decision:** a session with a day's limit, the user's or a project's, is refused at open (`day_limit_needs_stats`); `cortex stats`, `cortex usage` and the configuration commands still work. Said in the setting's description and 18. | `day::a_days_limit_with_statistics_off_is_refused_at_open` |
| **2. All-time totals were lost when `aggregates.json` could not be read:** a newer build's, or one a crash left empty, loaded as fresh, and the next save wrote over `kept`, the one copy of every pruned month. | **Fixed.** `Aggregates::load` reads and changes nothing; `load_for_saving` (the writer, and a report with statistics on) moves a file that does not parse aside first: `aggregates.json.unreadable-<UTC time>`, owner-only, byte for byte. A newer version is left alone: nothing is saved or pruned beside it, the report refuses with `stats_newer`, and `cortex stats` says so. **Step 17's rule is held by a check, not a convention:** `Aggregates` knows what it found (`cortex_stats::Found`), and `save` and the prune in `sync` refuse unless it was the file, nothing, or a file moved aside. The aggregates are flushed to the disk before they take their name (`sync: true`): measured 16 ms at the median and 20 ms at the 95th percentile for a month's 78 KB and for 400 days' 1 MB, against 0.12 and 1.3 ms unflushed, at most once every five seconds, on the writer thread. The comment "Both files can be rebuilt" is gone. | `store::a_newer_builds_aggregates_file_is_left_for_it`, `an_aggregates_file_that_does_not_parse_is_moved_aside_before_anything_is_saved`, `aggregates_that_could_not_read_the_file_are_never_saved_over_it` (`cortex-stats`); `usage::the_report_and_cortex_stats_say_what_the_aggregates_file_was` (`cortex-core`). The flush itself cannot be seen by a test. |
| **3. With statistics off a day's limit pruned monthly files and wrote `aggregates.json`.** | **Fixed** by the refusal above, and the purse now only reads (`load` and `sync` with no retention): the writer alone prunes and saves. | `day::a_days_limit_with_statistics_off_is_refused_at_open` checks that the old file stays and nothing is written |
| **4. The TZif reader allocated from the header's counts** before it checked the file's length: a 94-byte file asked for 17 GB, and aborted the process under an address-space limit. | **Fixed:** each block's length is added up with checked arithmetic and compared with the file's before anything is allocated; a file whose counts do not fit is refused. | `zone::a_zone_file_that_lies_about_its_counts_is_refused_without_a_large_allocation` (Linux, under a 4 GiB limit), `a_zone_file_shorter_than_its_counts_is_refused` (`cortex-base`) |
| **5. `Core::shutdown` waited on the writer's flush with no limit.** | **Fixed:** at most two seconds on the core's timer, from a thread of its own; what was left unwritten, and the dropped count, go to the log. There is no session to tell: they have ended by then. Two seconds: a flush is a few appends, a read of the files' tails and one flushed save, milliseconds and a few hundred on a slow disk, and two seconds is about as long as a person waits for a command to end. **Changed signatures:** `Recorder::flush(&self) -> Flushed` (was `()`), and `Recorder::flush_until(&self, &GiveUp) -> Flushed`, with `Flushed` and `GiveUp` new. | `usage::a_stuck_statistics_writer_holds_the_end_at_most_two_seconds` (`cortex-core`, Linux); `recorder::tests::a_flush_given_up_returns_at_once_with_what_was_left` |
| **6. A provider's error code went into the statistics as sent.** | **Fixed** with the refresh's rule, now shared: `cortex_protocol::error::code::is_plain`. The writer keeps a code only when it is plain, and `other` otherwise, whoever made the record. **Its twins:** a stop reason the client did not know (`openai:<the provider's own>`) and the name of a tool the model called that does not exist went in as written too; the same rule now holds for both. Looked for and not found: provider text in the log. The context probe's errors are built from fixed words and the status; transport errors never carry the URL. | `usage::a_providers_error_code_reaches_the_statistics_only_when_it_is_plain` (`cortex-core`); `store::a_providers_own_words_are_kept_only_when_plain`; `error::tests::only_a_plain_code_is_plain` (`cortex-protocol`) |
| **7. A half-written last line took the next writer's first line with it.** | **Fixed:** before its first append to a file, the writer ends a last line that has no line break. The cut line stays one unreadable line. | `store::a_half_written_last_line_does_not_take_the_next_writers_first_line` |
| **8. `limits.json` was read, merged and written with no lock.** | **Fixed:** the update holds the file's lock (`limits.json.lock`) from the read to the write. The lock is `cortex_base::private::hold_lock`, which the trust store's and the approvals' lock in `cortex-policy` (step 12) now share: the same code, moved, not copied. | `limits::an_update_takes_the_files_lock`; the lock's holding is tested in `cortex-policy` |
| **9. `usage/report`'s session range did not check `open_for`.** | **Fixed:** it answers only a client that has the session open, as `usage/get` does. | `usage::a_client_that_has_not_opened_a_session_gets_no_report_of_it` |

### The four rules no test held

| Break | Test that holds it now |
|---|---|
| `a-month-too-few-is-kept` | `store::a_monthly_file_stays_for_its_retention_and_goes_the_month_after` |
| `a-line-being-written-is-read` | `store::a_line_still_being_written_is_read_once_it_is_whole` |
| `days-fold-after-forty-days` | `store::days_fold_into_months_after_four_hundred_days` (400 days back stays a day, 401 folds) |
| `a-limits-file-that-cannot-be-read-is-replaced` | `limits::a_limits_file_that_cannot_be_read_is_left_as_it_is` (Unix: a file of mode 000) |

### The open items worse than described

- **`cortex stats reset` while another process writes.** The "Left" note above said a writer's later lines start a file again; they went to the deleted file. **Fixed:** before each line the writer compares the file it holds with the one the month's name gives (`cortex_base::FileIdentity`: device and inode on Linux, the volume's serial and the file's index on Windows, through `private::windows`, [decision 0029](../../decisions/0029-owner-only-files-on-windows.md), amended) and opens the name again when they differ or the name is gone. On Windows a file deleted while another handle holds it cannot be looked at until it is let go: the writer then lets its handle go, which completes the delete, and makes the file again. The day's reader reads a new file under the name from its start. Tested by `store::a_reset_while_a_writer_runs_keeps_the_writers_later_lines` and `day::a_days_reader_reads_a_new_file_from_its_start`, on both OSes (the Windows run is in [ci.md](ci.md)).
- **`a-days-time-limit`:** updated. The day's limits hold across processes now, and with statistics off they are refused; `limits.day.time` is still accepted and not held, and the configuration reference now says so.

### The docs that disagreed with the code

| Disagreement | Changed |
|---|---|
| The reference marked `limits.day.time` "Enforced." | **The doc:** the time limit's description says a day's is not held yet, and the reference's marker for a strict section is now "Fails closed.", with its legend saying it is about how the file is read, not whether the setting is in force (`schema/reference.rs`). |
| `limits.day`: "Limits on one day, across sessions" (`sections.rs`) | **The code** now holds that, across processes; the description says how, with the refusal and the bound. |
| `store.rs`: "Both files can be rebuilt (the aggregates)" | **The code:** the aggregates are flushed, and the comment is gone. |
| `Variants::from_map`: "it is not gone on with" | **The doc:** an epoch read so goes on, as this record's "The full review" says is intended. |
| 18: "The latest values per provider and credential" | **The doc:** per provider and source; a provider has one credential in use at a time. |
| `stats.retention` declared to take effect at once, and read when the core starts | **The doc:** declared `restart`. Its twins, read from the user's configuration when the core starts and declared at once, were `usage.week_start`, `usage.ranges` and `usage.refresh`: the same. Step 23's reload ([outstanding](../../outstanding/config-reload-through-the-core.md)) should re-read the four for the core, and declare them at once again. |
| Point 1 and `recorder.rs`: dropped *events* | **The doc:** events and limits updates, one count. |
| "A writer's later lines start a file again"; 14's "Disabled: no file is written" | The first is corrected above. The second holds now that a day's limit is refused with statistics off. |

### The notes of lower confidence

| Note | Confirmed? | Done |
|---|---|---|
| 1. The 371 ns figure measures an awake writer. | **Confirmed, and over budget.** Events 20 ms apart cost 12.4 µs at the median (13.1 µs with the loop's thread busy between), against 2.6 µs with no writer to wake: about 10 µs is the push waking the parked writer. | **The owner chose to keep the design and state the wake** (2026-11-15): the budget is now "≤ 10 µs on the calling thread for the event; an event that wakes a parked writer adds that wake, about 10 µs", with the three measurements beside it in [performance-budgets.md](../../performance-budgets.md). A writer that polled instead would cost wake-ups when idle, which the idle budget forbids. |
| 2. `+=` overflow in the sums. | Confirmed by reading. | **Fixed:** every sum saturates, as the ledger's do. `store::sums_that_would_overflow_stop_at_the_most_a_figure_holds` |
| 3. A writer that dies is invisible. | Confirmed by reading. | **Fixed:** a send to a stopped writer is dropped and counted, the log says once that it stopped, and a flush says `Stopped`. `recorder::tests::a_writer_that_stopped_is_counted_not_hidden` |
| 4. A forward clock jump prunes and folds early. | Confirmed by reading. Not small: the files hold no date to check the clock against, since a jumped clock stamps its events with the jumped date, and a jump cannot be told from a machine left off for months. | **Filed** for step 33: [a-clock-that-jumps-forward](../../outstanding/a-clock-that-jumps-forward.md). |
| 5. An unknown range is answered as all time. | Confirmed by reading. | **Fixed:** refused with `unknown_value`. `usage::a_range_this_build_does_not_know_is_refused` |
| 6. On Windows a reset may leave the file delete-pending. | Settled with the reset fix: the writer lets the file go when it cannot look at it. | The reset test on the Windows runner, below. |
| 7. An epoch from before step 16 follows the live model's variants. | Confirmed by reading; latent until reload rebuilds role models. | **Filed** for step 24, with what reload must do: [an-epoch-read-without-its-variants](../../outstanding/an-epoch-read-without-its-variants.md). |

### Twins looked for

- **An allocation sized by a count from a file or a wire** (finding 4): every `with_capacity` and `vec![0; n]` in the workspace. The session log's reads are sized by the scan of the file itself; the index's, the read tool's and the edit tools' by a file's length after a size limit; Windows' by the system's own answer. One more in test code: the test kit's scripted server sizes a request's body by its `Content-Length`; its client is the harness's own, in tests, so it is left.
- **A path that writes or prunes under `stats/` without checking `stats.enabled`** (finding 3): the writer (made only with them on), the report (saves and moves aside only with them on), the refresh (writes only with them on), `cortex stats` (reads only), the day's purse (reads only), and `cortex stats reset`, which deletes when asked whatever the setting.
- **Provider text in the statistics or the log** (finding 6): above.
- **A read, merge and write with no lock** (finding 8): the aggregates are written so by design, and any order of saves converges (the review found it sound). The trust store and the approvals have a lock (step 12).
- **A file this build cannot read, written over** (finding 2): `limits.json` is replaced when it does not parse, as before: it holds nothing that is not asked again.

### What later steps should know

- **Step 17:** read the aggregates with `Aggregates::load`; `save` refuses what `load` found unreadable or newer, and the prune in `sync` does nothing then.
- **The day's limits:** `day::a_line_written_before_the_days_purse_began_counts_once` holds a line of this process's from before its purse (counted once), and `a_line_an_older_build_wrote_counts_as_another_processs` a line with no mark (counted).
- **Step 23:** `UsageReport` still carries no dropped or unreadable count; `cortex stats` says what the aggregates file was and lists files moved aside (`StatsSummary::aggregates`, `set_aside`). The reload should re-read `stats.retention` and the `[usage]` keys.
- **Step 24:** [an-epoch-read-without-its-variants](../../outstanding/an-epoch-read-without-its-variants.md).
- **Step 44:** the context records are in the raw files only; a jumped clock can prune them early ([outstanding](../../outstanding/a-clock-that-jumps-forward.md)).
- **Step 45:** request lines now carry `charged_to`; an adapter that indexes our own lines can ignore it.

### Changed or added for later steps

- **`cortex-stats`:**
  - `Aggregates::load_for_saving`, `Aggregates::found`, `Found` (`may_save`).
  - `save` and `sync` refuse or prune nothing per `Found`.
  - `StatsDir::set_aside`; `reset` and `size` include the files moved aside.
  - `RequestRecord::charged_to`, `Recorder::writer` (the recorder's unique id).
  - `Recorder::flush -> Flushed`, `Recorder::flush_until`, `Flushed`, `GiveUp`.
  - `DayReader`, `DaySpent`.
  - `RequestRecord::error` and `stop` and `ToolRecord::tool` hold only plain words or `other`.
  - A `tracing` dependency, already in the build.
- **`cortex-core`:** `StatsSummary::aggregates` and `set_aside`; `AggregatesFound` (re-exported `Found`); the codes `day_limit_needs_stats` (session open), `stats_newer` and `unknown_value` (`usage/report`).
- **`cortex-agent`:** the `CatchUp` trait (`catch_up`, `id`), `Account::catching_up`, `Account::add_spent`, `RequestStat::charged_to`; the loop awaits the ledger's catch-up before each request it charges, and a request's statistics event carries what the ledger was charged for it (the attempts given up, and an interrupted request's).
- **`cortex-base`:**
  - `FileIdentity` (the device and inode on Linux; the volume and the 128-bit id, or the 64-bit index, on Windows).
  - `redact::is_plain_code`: the protocol's rule and the secret filter's, the one rule for every provider code kept.
  - `private::hold_lock` and `private::lock_path`, moved from `cortex-policy`.
  - The TZif reader refuses a file whose counts do not fit.
- **`cortex-protocol`:** `error::code::is_plain` and `error::code::OTHER`. No field and no method changed.
- **File formats:**
  - A request line's `charged_to`, added: older readers ignore it, and its absence counts as another process's line.
  - `aggregates.json.unreadable-<time>`.
  - `limits.json.lock`.
- **Configuration:** `stats.retention`, `usage.week_start`, `usage.ranges` and `usage.refresh` take effect at restart; the descriptions of `limits.day` and of the time limit.

### The reviews of these fixes

**The early review**, of the fixes for findings 1 and 2 and of the reset as soon as they worked, by a reviewer that had not seen the work:

| Found | Done |
|---|---|
| Spending charged and never in a request line: an interrupted request wrote none, and an attempt given up was charged and left out of its request's line. Other processes never saw it, so a day could pass its limit by any amount. | **Fixed:** the line carries what the ledger was charged; an interrupted request has its own; one refused as too long carries what the provider said it used. `interrupt::an_interrupted_request_is_recorded_with_what_it_used`, `retries::what_a_discarded_attempt_used_is_still_counted`. A process killed during a request, and dropped events, are still unseen, and 18 says so. |
| The day's reader let go of each file, so on ext4 a file made again after a reset could take the old inode number and pass for it. | **Fixed:** it holds each file it reads open. `day::a_days_reader_reads_a_new_file_from_its_start` |
| The day's reader thread never ended: it held a way to ask itself. | **Fixed:** it adds through an account without the catch-up. |
| A session registered as the process's own through the reader's channel could have a line counted twice (registered after its first line) or skipped (an earlier line still queued). | **Fixed** by marking the line with the purse that counted it (`charged_to`): no registration at all. |
| A purse that cannot read the files goes on with what it read, unsaid in the bound. | **Said** in 18 and the module's documentation. |
| Moving an unreadable file aside checked and then deleted, and could delete a good file another process had just written. | **Fixed:** a rename, which deletes nothing. |
| Aggregates that started from nothing could prune, and two that did could each leave out a month the other pruned. | **Fixed:** only aggregates read from the file prune. `store::aggregates_that_start_from_nothing_prune_only_once_saved` |
| Ending a cut line could leave a blank line, counted as unreadable. | **Fixed:** a blank line is passed over. `store::a_blank_line_is_not_counted_as_unreadable` |
| Notes: ReFS's 64-bit file index is not unique; a newer file read whole at each first open; a reset racing a writer's settle; processes in different zones. | The 128-bit id (`FileIdInfo`) is used. The purse's first read covers only the day's months. The zones are said in 18. A reset racing a settle can bring back a deleted month's part: filed, [a-reset-during-a-writers-settle](../../outstanding/a-reset-during-a-writers-settle.md). |

**The full review**, of the whole change, by another reviewer that had not seen the work:

| Found | Done |
|---|---|
| The report's sums were copies of the aggregates' that still overflowed: a saturated day bucket wrapped a week's figure, or panicked a debug build. | **Fixed:** the report uses the aggregates' own sums. `ranges::a_reports_sums_stop_at_the_most_a_figure_holds` |
| Beside a newer build's file the writer brought aggregates it may not save up to date at every settle, reading every monthly file from its start, every five seconds. | **Fixed:** it leaves them. `store::beside_a_newer_file_the_writer_reads_no_monthly_file` |
| The stuck-writer test held no writer: opening a FIFO to read and append does not block, so the flush answered at once. | **Fixed:** the test holds `limits.json`'s lock, which the writer waits in, and checks that nothing was written under it; it runs on Windows too. |
| A key-shaped code (`sk-` and lower-case hex) is plain in its shape. | **Fixed:** a code is plain only when the secret filter also finds no credential in it (`cortex_base::redact::is_plain_code`), for the statistics and the refresh. |
| Two attempts of different cost bases kept the later cost only. | **Fixed:** the costs are added, as the ledger charged them. |
| A volume that gives no 128-bit file id would have lost every line. | **Fixed:** the older 64-bit index is the fallback. |
| A file moved aside and made private warned that another process had written it; a writer never started was reported as one that left events unwritten. | **Fixed** (both in the log only). |
| Docs: a holder of a deleted file on Windows is also a day's reader; a line reaches the files after its writer's settle, and a background summary's when a turn counts it; a test's comment still named `writer`. | **Corrected.** |
| The limits lock's test sees the lock taken, not held through the update; the purse's first read and the refresh behind the lock wait without limits of their own; two files moved aside in one second. | **Filed:** [tests-the-step-16-fixes-left-thin](../../outstanding/tests-the-step-16-fixes-left-thin.md), [waits-on-the-statistics-without-a-limit](../../outstanding/waits-on-the-statistics-without-a-limit.md). |

**The second review**, of what the full review's fixes changed, by a third reviewer: the Windows `unsafe`, the shared sums and the writer's settle sound.

| Found | Done |
|---|---|
| A key glued to a code by its separators (`invalid_key_sk-…`) passed: the filter finds a key that stands as a word. | **Fixed:** the code is looked at after each separator too; and a refresh's error also refuses a code the core's filter knows by value. `redact::plain_code_tests` |
| The refresh's side of the shared rule had no test, and nothing proved the stuck writer stuck. | **Fixed:** the refresh test has a key-shaped code; two breaks added, `a-key-shaped-code-reaches-the-refresh-error` and `the-end-does-not-give-up-on-the-writer`, which hangs only if the writer is really stuck. |
| The end's warning was silenced for a writer that gave up with nothing counted waiting, the stuck writer's case. | **Fixed:** only a writer never started, with nothing waiting, says nothing. |
| Adding two attempts' costs of different bases breaks "amounts with different bases are never added", unsaid. | **Said** where it reaches: 18's cost section and the function's documentation. It is the statistics' line only, never a protocol message, and rare (a provider that stated one attempt's cost and not another's). |
| Two docs named the old rule. | **Corrected.** |
| The test of a file moved aside never moved a file that was not private: overwriting a file keeps its mode. | **Fixed:** the test makes the damaged file anew (found by `mutate`, below). |

### Breaks and stress

- **`cargo xtask mutate step-16-fixes`: all 43 breaks are caught by the tests they name**, the review's four among them. The run was on the desk's side slot, at one job: the disk was busy, which `mutate` now sees. Twice the baselines stalled, with timeouts across tests that nothing touched; the run carried on where it stopped once the disk was quieter.
  - Five breaks first came back flagged, and each is settled.
  - `the-file-moved-aside-is-not-private`: the test's damaged file was already private, since writing over a file keeps its mode. The test now makes it anew.
  - `the-first-block-is-trusted`: never caught, because the check is never the one that refuses. A lying first block already fails when the second header is read. The break is gone; the checked sum stays.
  - `aggregates-that-may-not-be-saved-prune`: the writer no longer brings such aggregates up to date at all, so only the direct test reaches the check. It alone is named.
  - `the-days-readers-…`: the test now replaces the file with no read between, which is when the identity is compared.
  - `a-limits-file-that-cannot-be-read-is-replaced`: a file of mode 000 is refused by the atomic write before the rule is reached. The test now makes the file write-only.
- **The older breaks this change moved** (ten of `mutations/step-16.breaks`, and `step-12-fixes/no-store-lock`, now the shared lock) are each caught again. `aggregates-do-not-rebuild-to-the-same-values` now cuts an offset by two bytes, not one: a line break left alone is a blank line, which is now passed over.
- **`cargo xtask stress -p cortex-stats -p cortex-core --count 50`:** 249 tests, 50 times each, under seeds 0 to 49. Every one passed every time.
- **`cargo xtask stress -p cortex-agent --count 300`** (the loop changed): 305 tests, 300 times each, under seeds 0 to 299. Every one passed every time.
- **The whole workspace** after the rebase onto `76bed87`: 3,145 tests passed, and `cargo xtask check` passed. The Windows lint (`cargo xwin clippy`) passed at the desk.
- **Windows:** the `try` run and the A/B in [ci.md](ci.md).
- **Spend:** none.
