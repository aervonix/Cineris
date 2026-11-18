# Usage and cost: the `/usage` view

One place to see what was used and what it cost: per provider and per model, over time, per project, and across the other coding agents installed on the same machine. The owner asked for this on 2026-11-12. The choices are recorded in [decision 0025](../decisions/0025-usage-view.md).

It rests on four rules.

1. **Nothing leaves the machine.** The view reads local files and the responses the harness already receives. By default it sends nothing and asks nobody. The one exception is a refresh of limits that the user turns on for a provider: it calls that provider's documented usage endpoint with the user's own key, and nothing else ("The opt-in refresh", below). It is never telemetry.
2. **One store for our own numbers.** Cineris's figures come from the local statistics ([14-adaptation.md](14-adaptation.md)). There is no second record of them.
3. **Other tools' records are read, never written.** Each source is opt-in, and every number says where it came from.
4. **Nothing is guessed.** A limit is shown only when the provider officially reports it, and "not available" where it does not. A cost that is not a real bill is labelled an estimate.

## What it shows

**Measures,** for each provider and model:

| Measure | Meaning |
|---|---|
| Input tokens | Fresh input: tokens sent that were not read from a cache. |
| Cache reads | Input tokens served from the provider's prompt cache. |
| Cache writes | Input tokens written to the cache. |
| Output tokens | Everything the model produced, with reasoning tokens shown as a part of it where the provider reports them. |
| Requests | How many model requests. |
| Cost | In US dollars, with its basis (below). |

**Ranges.**

| Range | Definition |
|---|---|
| This session | The open session, live. This is the session view that the usage panel already had: tokens by class for the session and the last request, cache hit rate, cost by model and role, time to first token and tokens per second. |
| Today | The local calendar day. |
| Week | The calendar week, starting on Monday unless `usage.week_start` says otherwise. |
| Month | The calendar month. |
| All time | Everything on record. |

`usage.ranges = "rolling"` switches week and month to the last 7 and 30 days, for people whose allowance works that way.

**Groupings.** By provider, then model. By project. By source (Cineris, or another tool). By speed tier, which shows what fast mode cost. Any two together, for example a project's cost per model this month.

**A sketch of the panel** (the look follows [09-tui.md](09-tui.md); this shows content, not final wording):

```
Usage · month                      [session] today  week  month  all        by: provider  project  source
anthropic                                   in      cache read  cache write      out      cost
  opus-5.5                               1.21M         38.4M        2.90M      612k    $41.87
  haiku-5.5                              0.44M          6.1M        0.52M      201k     $1.92
openrouter
  some-open-model                        0.80M            —            —       310k     $0.61
                                                                              total    $44.40
Limits   anthropic   tokens 71% left · resets 14:32      requests 96% left      as of 2 min ago
         openrouter  not available
Sources  Cineris
```

## Where our own numbers come from

The statistics store already records one event per model request. For this view each request event carries:

| Field | Note |
|---|---|
| Time | UTC, with the local day it fell on. |
| Provider, model, role | Role is `main`, `fast` or `judge`. |
| Speed | The speed tier the request ran at ([04-providers.md](04-providers.md), "Speed tiers"): `standard` unless the provider served another. It is what the provider reported, not what was asked for. |
| Project | The project key used elsewhere in the state directory. No path is stored in the event. |
| Session | The session id. |
| Tokens by class | Fresh input, cache read, cache write, output, reasoning. |
| Cost | Computed when the event is recorded, from the catalog's prices at that moment for the speed the request ran at, so a later price change does not rewrite history. Stored with the catalog version it came from. Absent when the model has no known price. |
| Billing | `metered` (an API key billed per token), `subscription`, or `unknown`. |

**Aggregates.** The small aggregates file gains buckets keyed by local day, provider, model, speed tier, project and billing. Today, week, month and all time are sums over day buckets, so opening the view reads kilobytes and scans nothing. Day buckets older than 400 days are folded into month buckets, which are kept until the user resets statistics, so "all time" survives the pruning of raw event files.

**This costs nothing new at start-up.** The aggregates are read the first time the view, or an adaptive feature, needs them. **Since step 24 the core keeps what a usage report read** for the next report, with the state of the file it was read from (the file itself, its length and its time, taken before the read): a report reads the file again only when it is not that file, and keeps nothing it saved; what is kept is given back after a quiet minute. A heavy year's aggregates are megabytes of JSON, and the usage panel asks for a report at each change of its range or grouping.

**As built in step 16.** The request event carries all of the above, the cost as the loop priced it when the request ended. The aggregates' usage buckets are as described, with the speed tier among the keys, and time and tools beside them ([14-adaptation.md](14-adaptation.md), "Local statistics"). The raw monthly files are kept for `stats.retention` months after their own, **3 by default** (the coordinator's decision in step 16, which the owner may revisit): a heavy user's are about 90 MB a month, and a file's part stays in the aggregates when it goes, so every range of this view, all time included, is unchanged by the pruning. `usage/report` answers from them alone:

- **Ranges:** the open session's own figures for `session`; today, the week from `usage.week_start` (or the last 7 days with `usage.ranges = "rolling"`), the month from its first day (or the last 30) and everything for `all`. Days are counted in the local zone: `cineris_base::Zone`, from `TZ` or `/etc/localtime` on Linux and from the system on Windows.
- **Groupings:** any of provider, model, project, source and speed, outermost first, with each row's turns' time when the grouping is of provider, model or project.

Since step 24 the report also says what `cineris stats` says of the files behind it (`notes`): a file that could not be read and was set aside, and lines that could not be read; and `usage.week_start`, `usage.ranges` and `usage.refresh` are read as they are in force when a report or a refresh runs, so a reload changes them at once.

`cineris usage [--range today|week|month|all] [--by G,…] [--project KEY] [--json] [--refresh]` prints it. `cineris stats [show|reset]` shows or deletes the files. Measured: [../performance-budgets.md](../performance-budgets.md).

**If statistics are disabled** (`stats.enabled = false`), the session range still works, from the session itself, and the other ranges say that recording is off. A session with a day's limit is refused (below).

**Since the fixes after step 16's independent review.** The session range answers only a client that has the session open (`session_not_open`), as `usage/get` does. A range this build does not know is refused (`unknown_value`), not answered as all time. An aggregates file of a newer build is not read or changed, and the report refuses with `stats_newer` rather than show all time without the pruned months; one that does not parse is moved aside before anything is saved, and `cineris stats` lists where.

## Limits on one day

`[limits.day]` holds requests, tokens and money to a limit for one local day, across every session of every process on the machine ([03-agent-loop.md](03-agent-loop.md), "Loop guards and limits"). Built in step 16 for the sessions of one process, and across processes since its fixes (the independent review found two processes with a limit of one request spending two).

- **The day's spending is the statistics'.** There is no second record of it. A core's first day-limited session of the day reads what the aggregates and the monthly files hold for the day, and opens the day's purse with it. Each session holds the day to its own limits (a project may lower them) and spends from the purse.
- **Before each request** a day-limited session makes, the purse reads what the monthly files gained since it last looked: the lines of the requests other processes made, a few lines a request. It reads them on a thread of its own, holding each file it reads open (so that a file made again after `cineris stats reset` cannot pass for the old one), and the request waits for it at most one second.
- **A process's own spending is counted as it happens,** by the ledger, so its own lines are not read back: each request line says which day's purse it was charged to (`charged_to`: the process's recorder's id, which no other process's has, and the day), and a purse skips exactly its own. Everything else counts: other processes', this process's sessions without a day's limit, a session's that runs past midnight on the day before's purse, and the lines of builds before `charged_to` was written.
- **What the ledger is charged is what the statistics hold.** A request's line carries what its attempts given up used with what its last used, a request interrupted part-way has a line of its own with what it used, and one refused as too long carries what the provider said it used (since the review of the fixes; before, other processes never saw that spending).
- **With statistics off** there is nothing to read another process's spending from, and a day's limit would hold for one process only: a session with one, the user's or a project's, is refused when it opens (`day_limit_needs_stats`), and says how to turn the statistics on or remove the limit (the owner's decision, 2026-11-15). `cineris stats`, `cineris usage` and the configuration commands still work.
- **How far it can be passed.** Another process's request is in the files once it has ended and its writer has written it (at once, or after a settle of a few milliseconds, seconds on a disk that is slow), and not while it runs; a summary prepared in the background is written when a turn counts it, at that turn's next boundary or the session's end. So two processes can both pass the check before either's request ends: a day's limit can be passed by the requests other processes have in flight when it is reached, which is at most two for each session running in another process at that moment (its request, and a summary being prepared in the background), and by the tokens and money those requests use. Within one process the ledger holds it as it holds a session's limit: a token or money limit can be passed by the requests in flight when it is reached. **Spending other processes cannot see** is outside that bound: a process killed during a request, events its writer dropped (a full queue, or a writer that stopped), and a line it could not write never reach the files. And a purse that cannot read the files (a disk that stalls past the second, or a file it may not read) goes on with the day's spending as last read, and says so once in the log; until it can read them again the limit holds for its own process's spending and what it had read. Each process gives a line the local day of its own zone: two processes in different zones count the hours where their days differ on different days.
- **Not yet built:** `limits.day.time`, and a day that ends while a session runs ([outstanding](../outstanding/a-days-time-limit.md)).

## Cost, and what kind of number it is

Every cost on screen has one of four bases, shown beside it or in the column header.

| Basis | When | Shown as |
|---|---|---|
| **Metered** | The request was billed per token on an API key, and the catalog has the price. | A plain amount. It is our calculation from the provider's published prices, not the provider's invoice, and the view says so once. |
| **API-equivalent estimate** | The usage was on a subscription, or the billing kind is unknown. | An amount marked `est.`, with the legend "what this would have cost at API prices; a subscription is not billed this way". Never added into a metered total without the mark. |
| **Recorded by the tool** | Another tool's own files state a cost. | The amount, marked with the tool's name. |
| **Unknown price** | The model is not in the catalog. | Tokens only, cost `—`. |

A total that mixes bases shows them apart: `$44.40 metered + $212.10 est.`

One exception, since the fixes after step 16's review: a request's line in the statistics carries what its attempts given up cost with what its last cost, as the ledger charged them, and in the rare request whose attempts were priced on two bases (a provider that stated the cost of one attempt and not another) the sum is under the basis of the attempt that counted, rather than a part of the money lost.

## Limits left

For each provider: how much of its rate limit or allowance remains, and when it resets.

| Source of the figure | Used? |
|---|---|
| Rate-limit headers on responses the harness already receives (limit, remaining and reset time, for requests and tokens) | **Yes.** The agent loop already parses them ([03-agent-loop.md](03-agent-loop.md)). The latest values per provider and source (the headers, or the refresh) are kept, with the time they were seen: a provider has one credential in use at a time, so a second credential's figures replace the first's. |
| Usage or allowance fields inside those responses | **Yes**, where a provider documents them. |
| A provider's documented usage endpoint | **Only when the user has turned the refresh on for that provider.** Off by default. See "The opt-in refresh". |
| A subscription's remaining allowance | **Only** where the provider officially exposes it to third-party tools, in what the harness already receives or through the opt-in refresh. Otherwise "not available". |
| Scraping a web page, an undocumented endpoint, another tool's private files | **Never.** |

- A speed tier with a rate limit of its own (Anthropic's fast mode has one) is shown as its own line under the provider, labelled with the tier.
- A figure is shown with its age (`as of 2 min ago`). Once its reset time has passed it is shown as `reset since; unknown until the next request`, not as full.
- Where neither response headers nor the opt-in refresh gives a figure, the provider shows `not available`. As read on 2026-11-12, Anthropic and OpenAI document rate-limit headers, and none was found documented for Google's API. The provider build steps re-check this.
- These are rate limits per minute or per day, as each provider defines them. They are not a spending forecast, and the view does not turn them into one.

### The opt-in refresh

Response headers say what is left only after a request has been made, and some figures, a subscription's allowance among them, appear only at a provider's usage endpoint. The owner decided on 2026-11-12 that the view may ask for them, on these terms, and confirmed the terms as written here the same day.

| Rule | What it means in practice |
|---|---|
| **Opt-in per provider, off by default** | `usage.refresh = ["openrouter"]` names the providers the user allows. It is a user-only setting: a project's configuration cannot turn it on. With the list empty, which is the default, the view makes no request at all. |
| **Only the provider's documented usage endpoint** | The endpoint must be one the provider documents for use with an API key or an official sign-in. A provider without one has no refresh: listed, it is refused when the refresh is asked for (`no_usage_endpoint`), and nothing is sent. No scraping, no undocumented endpoint. |
| **The user's own key, to the user's own provider** | The request goes to the provider the key belongs to, with the credential the user already configured for it, over the same client as model requests. It carries nothing else: no usage figures, no project names, no identifiers of ours. |
| **Never telemetry** | Nothing is sent to us or to any third party. The response is used for the view and is not forwarded. |
| **Only when asked** | The request is made when the user asks for it: the refresh key in the view, or `cineris usage --refresh`. Nothing calls it at start-up, on a timer or in the background, so an idle session still makes no request. |
| **Labelled** | A refreshed figure shows its source and age (`from anthropic · as of 10 s ago`), like every other limit. A failed refresh keeps the last figure with its age and says the refresh failed. It never shows a guess. |

The endpoints themselves are provider facts that go stale. The step that builds each provider's refresh starts by reading that provider's current documentation and records what it read, as the provider steps do.

**The first three endpoints** were built with the `openai-chat` protocol in step 31, as far as a provider's side goes: which request to make, and how to read the answer ([04-providers.md](04-providers.md), "The `openai-chat` protocol"). OpenRouter, DeepSeek and Venice.ai each document one that an ordinary key can call. OpenAI's takes an admin key, so it has none. Two things came out of it for this view:

- **They report balances, in a currency.** A limit window counts requests or tokens, and none of the three reports those through its endpoint as something left. What they report is money: US dollars, yuan, one provider's own unit. Since step 16 a balance is carried by `ProviderLimits::balances`, below ([steps/16/answered.md](../steps/16/answered.md)).
- **An answer is read and dropped.** One provider's answer holds a shortened form of the key. Nothing of an answer but its figures may be kept, shown or logged.

**As built in step 16** (`cineris-core`'s `usage`; `cineris usage --refresh`; `usage/refresh`).

- **Which providers have one,** read on 2026-11-15: OpenRouter (`GET /api/v1/key`), DeepSeek (`GET /user/balance`) and Venice.ai (`GET /api/v1/api_keys/rate_limits`). Not Anthropic, whose usage, cost and rate-limit endpoints are its Admin API's and take an Admin API credential. Nor OpenAI, whose usage API takes an admin key. Each provider's endpoint, the key it takes, what it returns and what is kept are in one table, in [04-providers.md](04-providers.md), "The `openai-chat` protocol".
- **The request** is one `GET` with no body for each provider in the user's `usage.refresh` (a name listed twice is asked once), to the endpoint under the provider's own `base_url`, with its own configured headers and its own credential, through the transport model calls use, which follows no redirect. It is bounded to 20 seconds, and the answer to 256 KiB. It carries the client's `User-Agent` and nothing else of ours. It is made only by `usage/refresh`, from a `user` client, which `cineris usage --refresh` is.
- **What is kept** is the figures alone, as `ProviderLimits` with `source: refresh`: limits on the key (a model's own are not kept), balances as exact decimal strings one a currency, whether the key may make calls, and the next epoch where the provider names one. They go into `stats/limits.json`, owner-only and replaced atomically, and not at all with statistics off. A refresh that fails, however it fails, keeps what was there and says which provider failed and with what status, with nothing of the answer's text.
- **Limits from responses** reach the same file: each response's rate-limit headers are a statistics event, and the writer keeps the latest of each provider. A figure past its reset time is shown as not known (`remaining` dropped), never as full.

## Usage recorded by other tools

Many people run more than one coding agent. The view can include what the others recorded on this machine.

**Order of work.** Claude Code and Codex first. Then Opencode and Pi. Others as people ask.

**Each tool has its own adapter**, behind one small trait:

```rust
pub trait UsageSource {
    fn id(&self) -> SourceId;                        // "claude-code", "codex", ...
    /// Is this tool's data present, and in a version we can read?
    fn detect(&self, home: &Path) -> Detection;      // Absent | Readable { version } | Unreadable { version, reason }
    /// Read records newer than the cursor. Never writes. Never fails as a whole.
    fn scan(&self, cursor: &Cursor, sink: &mut dyn RecordSink) -> ScanReport;
}
```

| Rule | What it means in practice |
|---|---|
| **Opt-in per source** | Nothing is read until the user turns a source on: `usage.sources = ["claude-code"]`, or the prompt the first time `/usage` is opened ("Usage records from Claude Code and Codex were found. Include them?"). Detection looks only for the tool's data directory. It opens no file. |
| **Read-only** | Files are opened for reading, with sharing that never blocks the other tool. Nothing is created, changed or locked in another tool's directory. |
| **Only the numbers** | Other tools keep usage inside files that also hold conversations. An adapter extracts the time, provider, model, project path, token counts, any cost and a record id. Prompt and response text is never copied, cached or logged. |
| **Tolerant** | Unknown fields are ignored. A record it cannot understand is skipped and counted. A file it cannot parse is skipped and named. |
| **Honest about versions** | Each adapter lists the format versions it was written against. Faced with a newer one it does not recognise, the source reports "cannot read this version of the tool's records (last supported: …)" and shows `not available`. It never crashes the view and never shows numbers it is unsure of. |
| **No double counting** | Records are keyed by the tool's own record id, so reading a file twice, or a file the tool rewrote, adds nothing. |
| **Provenance** | Every row and every total says which source it came from. |

**Separate or merged.** The `by: source` grouping shows each tool on its own. The merged view adds them into one table per provider and model, with the source split one key away, and keeps estimates marked as estimates.

**Cost for other tools' usage** follows the table above: the tool's own recorded cost if there is one, otherwise an API-equivalent estimate from our catalog. Usage from a tool that was signed in to a subscription is the main case for the estimate label.

**Projects.** Other tools record a working directory. It is matched to a Cineris project by canonical path. Paths with no match are listed under their own path, shortened with `~`.

**The cache.** Other tools' history can be large, so it is never read when the view opens.

- A background task on the blocking pool indexes each enabled source into `<state dir>/usage-cache/<source>/`: day buckets in the same shape as our own aggregates, and a cursor per file (size, modification time, offset).
- Indexing is incremental, runs at low priority on one thread, and starts when the view is first opened or when a session has been idle for a while. It never runs at start-up.
- The view opens at once from whatever is cached, says `updating…` while the index catches up, and refreshes when it has.
- The cache is derived data. `cineris usage forget <source>` deletes it, and it can always be rebuilt. It holds numbers and project keys, nothing else.

**How adapters are written.** From the files each tool writes on disk and from its public documentation. The formats belong to those tools; we only read them. No code from any of them is used, and the adapters for Pi and Opencode are subject to the same rule as everything else in this project: study, then write our own. These formats are not promises, which is why every adapter is version-aware and why a failure is reported and never fatal. The step that builds each adapter starts by examining what the current version of the tool writes, and records the versions it handles.

## Commands and protocol

| Surface | What |
|---|---|
| TUI | `/usage` opens the panel. Keys switch range and grouping. The hint row shows a provider's limit when less than a fifth is left ([09-tui.md](09-tui.md)). |
| Command line | `cineris usage [--range session\|today\|week\|month\|all] [--by provider\|model\|project\|source] [--json]`. `cineris usage sources` lists detected sources and their state. `cineris usage forget <source>` deletes a source's cache. |
| RPC | `usage/get` (the session, live) and `usage/report` (a range and grouping). After the first release, `usage/sources` to list, enable and disable sources ([11-rpc-protocol.md](11-rpc-protocol.md)). |

Enabling a source is a user-only setting. A project's configuration cannot turn one on.

## Security and privacy

- **No network by default.** Stated above, and tested: with the refresh off, opening the view and running a full index make no connection (a test with the network denied). With the refresh on for a provider, a test asserts that the only connection is to that provider's documented endpoint, with that provider's credential, and that the request carries no usage data.
- **Other tools' files are untrusted input.** They are parsed with size and line-length limits. Symbolic links that lead out of the tool's data directory are not followed. A malformed file costs that file, not the view.
- **Least reading.** An adapter reads only the files that hold usage, and keeps only numbers.
- **The cache is owner-only**, like all state files.
- Threats and mitigations are listed in [../security.md](../security.md).

## Speed

| Budget | Limit |
|---|---|
| Opening `/usage` | A frame, from cached aggregates. No scan, no parse of raw history. |
| `cineris usage --range month` | Within the headless start-up budget plus reading the aggregates. |
| Background indexing | One low-priority thread, bounded memory, cancellable. Never on the start-up path, and paused while a turn is streaming. |

The figures are in [../performance-budgets.md](../performance-budgets.md).

## Testing

- Aggregation is tested with a fake clock across day, week, month and time-zone boundaries, and after raw files are pruned.
- Each adapter has fixtures: **small, made-up example files** written for the tests, one set per format version it supports, plus damaged and unknown-version examples. **Real session data is never used and never committed.** The repository check that rejects home-directory paths also covers fixtures.
- Adapters are fuzzed in the non-blocking tier.
- A test asserts that an adapter's output and the cache contain no text field from the input.

## First release, and later

| Part | When | Why |
|---|---|---|
| Our own usage and cost by provider and model, all five ranges, per-project breakdown | First release: recorded in the statistics step, shown in the panels step. | It is the statistics store plus a panel, and it makes the adaptive features' data visible from day one. |
| Limits left, from response headers | First release. | The loop parses them already. |
| The opt-in refresh from a provider's documented usage endpoint | First release: the setting, `cineris usage --refresh` and the first providers in the statistics step, the key in the panels step. Each later provider adds its own endpoint in its own step. | It depends on what each provider documents, so each provider's step reads that documentation first. |
| `cineris usage` and `usage/report` | First release. | The same query as the panel. |
| The API-equivalent estimate label | First release for the mechanism. It first matters with other tools' records and with sign-in flows. | The first release uses API keys only, which are metered. |
| Adapters for Claude Code and Codex, the cache, separate and merged views | After the release, as its own step. | Reading other tools' formats is separate work with its own fixtures, and nothing else depends on it. |
| Adapters for Opencode and Pi, then others | The same step or follow-ups to it. | |

## Settled on 2026-11-12: usage endpoints

The brief allows "documented usage endpoints" as a source of limits, and also says the view sends nothing anywhere. A usage endpoint is a network call, so the first draft of this page called none and left the question to the owner. The owner's answer: besides response headers, allow a refresh that the user opts in to per provider, which calls that provider's documented usage endpoint with the user's own key. It is off by default and is never telemetry. Where neither source gives a figure, the view shows "not available". "The opt-in refresh", above, is that answer in detail.
