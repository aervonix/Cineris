# Answered in step 16, on 2026-11-15

Step 16 built the local statistics in `cortex-stats`. It also built `cortex stats` and `cortex usage`, `usage/report` and the opt-in `usage/refresh`, model roles with a budget each, variant points fixed per epoch, the per-day limits, and, at the owner's request, the split of every turn's time into the parts it was spent on. Its prompt asked it to settle five points. The coordinator gave the owner's answers.

## The five points

| Point | Settled | Recorded in |
|---|---|---|
| **1. How records leave the loop.** | **A bounded queue and a writer thread of the statistics' own** (`cortex_stats::Recorder`). The thread is not the log's. On the loop, `record` converts the event and makes one `try_send` of a boxed record. A full queue drops the event and counts it, and never blocks. The count is written as an event of its own (`kind: "dropped"`) when there is room. Making a recorder makes the queue and nothing else. The core starts the thread on the blocking pool when the first session opens, and flushes it at `shutdown`, because a detached thread dies with the process. Tested with a writer that never runs: the queue of four takes four events of ten and counts six, and none of the ten calls waits. | [14-adaptation.md](../../architecture/14-adaptation.md), "Local statistics", as built in step 16; [decision 0021](../../decisions/0021-local-statistics.md), amended |
| **2. Which calls go to which role.** | **One utility call exists after step 15: the compaction summary.** It goes to `fast`, in the background or not. With no `fast` model it goes to `main`. When the `fast` model refuses it as too long, or fails it in any way but an interrupt, the summary is asked once of `main`, and `fast` is not asked again in that epoch. `judge` has no call before the run engine. Its model is resolved, priced and shown, and it falls back to `main` in the same way. Titles, summaries of long output and proposed profile entries are not built; each goes to `fast` when it is. The routing is one function (`Models::for_call(UtilityCall)`), with the table in its documentation. | `cortex_agent::config::UtilityCall`; 14-adaptation.md, "Model roles and routing" |
| **3. The usage refresh.** | **Built for OpenRouter, DeepSeek and Venice.ai. There is no refresh for Anthropic.** Its usage, cost and rate-limit endpoints are Admin API endpoints ("workspace API keys don't work"), so they take another kind of credential, and the brief said to build nothing for one. The owner approved this on 2026-11-15, with five conditions, each met and tested (the next table). | "The refresh", below; [18-usage.md](../../architecture/18-usage.md), "The opt-in refresh" |
| **4. What a variant point is.** | **A named choice about how a model is addressed.** Its set of values is one this build knows. The catalog gives each model a default, and the user may override it for a model (`[adapt.variants."<model>"]`). The choice is fixed for a context epoch and recorded in the epoch's `epoch` entry (`variants`, an addition to the log's format) and on every outcome: each request, tool call and turn event carries the set. The aggregates count outcomes by model, variant set and tool. The points shipped: `edit_format` (`replace`), `tool_schema` (`strict`, where the model can be held to it, or `loose`), `prompt_style` (`concise`, or `guided`, which adds a section of tool guidance to the system prompt, `cortex_agent::GUIDED_NOTES`) and `effort` (the model's levels; a role's own `effort` wins over the variant's). An unknown choice stops the session from starting (`unknown_variant`). A change applies from the next epoch that builds a prompt. An epoch begun by a change of model keeps the prompt's style and takes the new model's tool and edit choices. | 14-adaptation.md, "Adapting to the model", as built; [07-sessions-and-context.md](../../architecture/07-sessions-and-context.md), "Context epochs" |
| **5. Retention and pruning.** | **Monthly files by the UTC month of the event, kept for `stats.retention` months after their own (12 by default). Day buckets older than 400 days fold into month buckets.** Each monthly file has its own part of the aggregates, with the byte it was read to. A file past its retention moves its part into `kept` before it is deleted, so all-time totals outlive it. A rebuild from the files gives the same values, which is tested. **Disk use, measured:** a heavy user's synthetic day (3,000 requests, 4,500 tool calls and 200 turns) is 3.2 MB of events, and the per-request context records add about a third again. That is about 4.4 MB a working day, 90 MB a month, and **about 1.1 GB at the steady state of a year's retention**. The aggregates are 51 KB for a month and stay under 1 MB at 400 days of day buckets. A more usual heavy day of 800 requests is about 300 MB a year. | 14-adaptation.md; the conflict is raised in the step's report, with a recommendation |

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
