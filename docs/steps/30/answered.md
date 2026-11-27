# Answered in step 30, on 2026-11-27

Step 30 builds `cineris-mcp`, the MCP client of the first usable release: local and remote servers with static credentials, both protocol eras, tools through the dynamic-tool registry, direct exposure, per-tool hashing and approval, the lifecycle, the project's `.mcp.json` read live, other clients' files imported, and `cineris mcp add|list|inspect|import|remove`. The owner's answers come through the maintainers.

| Question | The answer | Recorded in |
|---|---|---|
| **The client** (point 1): `rmcp` behind our own interface, or a client of our own, decision 0039's first size reserve | **Our own client** (the owner, 2026-11-27): the first reserve is spent here. | Point 1 below |
| **`rmcp`'s server as a check of our client** (point 1), third-party code run in our tests | **Yes** (the owner, 2026-11-27): a dependency of the check alone, run in process in the nightly tier against both eras; pinned in its lock file, covered by the nightly audit of dependencies, kept out of the shipped binary and the blocking checks, and recorded in [dependencies.md](../../dependencies.md). | Point 1 below |
| **A server the user wrote into their configuration by hand** (point 4) | **Not started until `cineris mcp approve NAME`** (the owner, 2026-11-27): it is listed as needing approval, and `cineris mcp add` approves as it writes. | Point 4 below |
| **Legacy Streamable HTTP without `2025-11-25`'s polling and resumption** (point 1) | **Yes** (the maintainers, 2026-11-27): a stream that drops fails its request, and the GET stream is opened again from fresh. An item is opened for resumption. | Point 1 below |
| **A `tools/call` not sent again after its HTTP stream broke** (point 3.7) | **Yes** (the maintainers, 2026-11-27): it returns that the call may have run. [13-mcp.md](../../architecture/13-mcp.md) says so, as a deliberate difference from the specification's MUST, with the reason. | Point 3.7 below |
| **No pin without a person** (point 3.6) | **Yes** (the maintainers, 2026-11-27): no trust on first use; a held tool asks a person at every level, `unrestricted` included; no rule or grant answers it; a headless run refuses it. | Point 3.6 below |
| **The URL rules** (point 3.8) | **As written** (the maintainers, 2026-11-27), with no proxy in this step and an item opened for a proxy that keeps the address checks. | Point 3.8 below |
| **`a-kept-run-adopted-after-its-call-moved`** (point 6) | **Fixed in this step,** and the item deleted (the maintainers, 2026-11-27). `a-tool-that-makes-links-beside-other-calls` is settled by MCP tools being exclusive. | Point 6 below |
| **The changes the review of the note brought** (point 8, item 9): the working folder, approval by command in two steps and `import` of the user's own files, a project's header from its derived variable, no `write`, held tools by name only, `fetch`'s TLS check, the count of every message | **Agreed** (the maintainers, 2026-11-27); [13-mcp.md](../../architecture/13-mcp.md) says plainly each limit the user meets: `--root .` in a project that is not trusted, the derived variable of a project's header, no writes. | Points 3 to 5 below |
| **`.mcp.json`, `.cursor/mcp.json` and `.vscode/mcp.json` protected** (point 8, item 10) | **Yes** (the owner, 2026-11-27), beside `.cineris/` and `.npmrc`, with a test and a break. | Point 8 below |
| **The changes outside `cineris-mcp`** (point 5): the launcher's live input, the registry's kinds of source, section 6 without the script engine, guard A's `mcp` topic and guard B's list, `TrustResourceKind::McpTool` and the held tool's detail | **Accepted** (the maintainers, 2026-11-27); the launcher's live input has an early review of its own, as a change to the OS layer: neither a server's input nor a crafted command line may change the policy it reads. | Point 5 below |

## The design, before building

Written before any product code, so that each pull request is reviewed against rules written first.

### 0. What was read, and when

All read on 2026-11-27, from the official site and its schemas, as raw text:

- **The current revision is `2026-07-28`**, the first of the "modern" era: no `initialize`, no sessions, no `ping`, no GET stream. Every request carries its protocol version and the client's capabilities in `_meta`; every server answers `server/discover`; a server that needs something from the client returns `resultType: "input_required"` and the client sends the request again with a new id (multi round-trip requests); tool-list changes come on a `subscriptions/listen` stream; list results carry `ttlMs` and `cacheScope`; Streamable HTTP adds required headers (`Mcp-Method`, `Mcp-Name`, `Mcp-Param-*` from a tool's `x-mcp-header`), and a client must leave out a tool whose `x-mcp-header` is invalid. Its pages: the specification index, changelog, deprecated features, base protocol, versioning, `server/discover`, transports (standard I/O and Streamable HTTP), tools, caching, pagination, subscriptions, cancellation, multi round-trip requests, authorization, and `schema/2026-07-28/schema.ts`.
- **The "legacy" era is `2024-11-05`, `2025-03-26`, `2025-06-18` and `2025-11-25`,** all with the `initialize` handshake: lifecycle, ping, cancellation, tools and transports of each, and each schema. `2025-11-25`'s Streamable HTTP can deliver a response only after the client reconnects with `Last-Event-ID` (its "polling"); `2025-03-26` alone allows batches.
- **Era detection:** on standard I/O, probe with `server/discover`; a result or a modern error (`-32020`, `-32021`, `-32022`) is modern; any other error, or silence, is legacy and falls back to `initialize`; the verdict may be kept for the server's configuration. On HTTP, a 400 without a modern error body is legacy. The TypeScript SDK's documentation adds that servers built on `rmcp` exit on any request before `initialize`.
- **The security guidance:** annotations and descriptions are untrusted; a human in the loop who can deny a call; consent showing the exact command for a local server; tokens only in the `Authorization` header and only to the server they were issued for; no network `$ref`; bounds on every buffer, since the protocol sets no size limit.
- **The next revision** is planned for 2027-03-30; the cycle is now every six months.
- **`rmcp` 3.12.0** (published 2026-11-26), from crates.io, docs.rs, its repository and its source as published: Apache-2.0 (with older contributions under MIT); MSRV 1.88; 44 releases in 2026 and eight semver-incompatible lines (0.13 to 0.17, 1.0, 2.0, 3.0), stable at 3.x since 2026-07-28. It supports all five revisions; its plain `serve()` is legacy only, and dual-era is `ClientLifecycleMode::Auto`, which falls back after a fixed 10 s of silence. `call_tool` treats `input_required` as an error, so the retry loop is the caller's; it sets no request timeout by default; its child-process transport brings its own process handling (`process-wrap`, with the `windows` crate); its HTTP client is pluggable (`StreamableHttpClient`), and its reqwest form brings its own TLS choice.

### 1. The client: `rmcp` or our own (the owner's choice)

**Measured.** `rmcp` 3.12.0 linked into today's binary as this step would link it (`client`, `transport-async-rw` over pipes the harness gives, `transport-streamable-http-client` over the workspace's own `reqwest`), with a client that probes and falls back, lists, calls, cancels and listens for changes, called from a hidden switch, on a branch never merged (`08ccca5c`), the shipped `dist` build on hosted runners against `main` at `e3c01758`:

| | Linux musl | Windows |
|---|---:|---:|
| `main` | 17,574,968 | 21,306,880 |
| With `rmcp` | 19,292,984 | 23,478,784 |
| **`rmcp`'s cost** | **+1,718,016** | **+2,171,904** |
| Where (Linux symbols, `size --breakdown`) | `rmcp` +613.9 KB, `serde` +286.0 KB (its model's derives), `std` +152.5 KB | |

It adds 19 packages to the lock file (`futures`, `jiff`, `tokio-macros`, `tokio-stream`, `tracing-attributes`, `unicode-normalization`, `unicode-general-category`, `sse-stream`, …) and turns on `tokio`'s `macros`. Runs: `try` 45952065767 (Linux) and 45952314209 (Windows).

**What `rmcp` would not do for us.** Most of the client is ours either way:

- **The process.** Its child-process transport starts the program itself; a server must start through our launcher (the sandbox, the Windows shim rule, the explicit environment, the staged kill, the job). So we hand it pipes.
- **HTTP.** Only `cineris-llm` may link the HTTP stack (01-workspace, rule 5), and a remote server needs `fetch`'s address checks, no redirects and our TLS check. So we write its `StreamableHttpClient` over `cineris-llm`, GET and DELETE included for the legacy era.
- **The policy around the protocol.** Multi round-trip requests, request timeouts, the probe's timeout (fixed at 10 s in `rmcp`), a probe that kills a legacy server, the cap on tools and messages, what a broken stream means for a call that may have run: ours.
- **The fake servers.** The tests need servers that misbehave on purpose (floods, silence, answers out of order, a crash mid-call, a changed tool), in both eras, so they are ours either way.

What `rmcp` does for us is the JSON-RPC correlation, the message types and the two eras' handshakes: by the research's checklist about 1,500 to 2,500 of the 3,000 to 4,500 lines a whole client takes.

**Our own client.** The checklist (research notes, kept in the step's part records): the JSON-RPC codec and correlator (cineris-base's framing already reads and writes the lines), the tools subset of the types, the probe and fallback, the legacy handshake and its pings, the modern `_meta`, the minimum of multi round-trip requests, the list cache and the change stream, standard I/O over our pipes, and Streamable HTTP in both eras over `cineris-llm` (whose SSE decoder exists). **About 3,000 to 4,500 lines**, of which the legacy HTTP path is the largest share. Priced from the crates built so far (`cineris_ext` 213 KB of symbols for about 16,000 lines with tests; `cineris_llm` 233 KB for about 16,700), and the derives of about thirty types: **+0.15 to +0.35 MB on Linux and +0.2 to +0.45 MB on Windows, a guess.** Tracking the protocol by hand: a revision every six months (the next on 2027-03-30, with betas six weeks before), each read and its changes made, a small step's work.

**The rest of the step is the same either way:** the launcher's live input and explicit environment, the registry, the approvals, the lifecycle, the HTTP adapter, the configuration, the commands. Priced against step 28's code outside its engine (+1.15 MB and +1.5 MB for a larger surface): **+0.6 to +1.0 MB on Linux, +0.8 to +1.4 MB on Windows.**

**The outlook to step 34, both ways.** Left today under 20 MiB and 25 MiB: **3,396,552 bytes on Linux and 4,907,520 on Windows.** Steps 29 and 32 to 34 with the fixes between, as decision 0039's revisit took them (its third row less step 30's): +0.5 to +2.4 MB on Linux, +0.6 to +3.4 MB on Windows. Neither row prices the signed approvals being built alongside, nor the fixes of step 28 not yet merged; they come out of the same room. Our own client is priced by symbols a line, the method that missed step 28's own code by 1.6 to 2.5 times: at twice the guess the high end is −0.70 MB on Linux and −0.79 MB on Windows, which "z" still covers.

| | Step 30 | Left at step 34 (low end; middle; high end) | What the high end spends |
|---|---|---|---|
| **`rmcp`**, Linux | +2.3 to +2.7 | +0.60; −0.55; −1.70 MB | "z" for the whole profile (1.14), **and `panic = "abort"`** (2.04) |
| **`rmcp`**, Windows | +3.0 to +3.6 | +1.31; −0.39; −2.09 MB | "z" (1.72), **and `panic = "abort"`** (6.38) |
| **Our own**, Linux | +0.75 to +1.35 | +2.15; +0.90; −0.35 MB | "z" (1.14) |
| **Our own**, Windows | +1.0 to +1.85 | +3.31; +1.48; −0.34 MB | "z" (1.72) |

With `rmcp` the middle of the range is over both budgets, and the high end spends every reserve, `panic = "abort"` among them, the one decision 0039 keeps last because unwinding is what contains a server's data that trips a bug. With our own client the middle keeps 0.9 and 1.5 MB, and the high end needs "z" alone.

**Against our own client:**

- **Interoperability is tested only against servers we wrote.** `rmcp` is used by many clients and servers, and its fixes since July are mostly the era and HTTP corners (3.1.3 to 3.12.0). Our fakes are written from the specification's text and schemas, and a server that reads the specification differently fails with our client first. One mitigation needs the maintainers' yes, since the step's rule is that no third-party code runs: `rmcp`'s server in a **package of its own outside the workspace** (`interop/`, as `fuzz/` is, since a dev-dependency cannot be optional and would be built by every blocking test run), run in process over a pipe as an interoperability check in both eras, in the nightly tier. Nothing is installed or downloaded at run time.
- **A new revision is ours to read**, twice a year.

**Recommendation: our own client, with `rmcp`'s server as the nightly interoperability check if the maintainers allow it.** It spends the first reserve, which costs no speed, and keeps "z" and `panic = "abort"` for later; and the parts of a client that carry its risk (the process, HTTP, the policy) are ours in either case. **Either way the client sits behind one interface of our own** (point 5), so the pull requests that do not depend on the choice go ahead now, and the other choice stays open until the client's own pull request: `rmcp` can be put back behind it later, as decision 0010 has it the other way round.

**A cut in scope either way, for the maintainers:** legacy Streamable HTTP without `2025-11-25`'s polling and resumption (a response that would come only after a reconnection with `Last-Event-ID` fails its request, said plainly); the legacy GET stream opened, and opened again after a drop, without resuming. Most of what that leaves out is the costliest part of the legacy HTTP path, for servers of one revision that also close their streams early.

### 2. Who controls what

| Who | Controls | Can make the harness do | Held by |
|---|---|---|---|
| **The user** | `[mcp]` in their configuration; every approval; `cineris mcp add\|import\|approve\|remove` | Start the servers they approved, with the environment and access their entries give | Their configuration as written; pins of each server and tool in `pins.toml` (point 4). No project file can name, change or turn off one of the user's servers. |
| **A repository's files** | `.cineris/config.toml`'s `[mcp]`, the project's `.mcp.json` | **Nothing** until the project is trusted and each server is approved as it is now; then start that server, confined | Project trust, and pins of each server and tool in `trust.toml` |
| **A server** | Its tool list, descriptions, schemas, results, its timing | Text that reaches the model only as search results and tool results, labelled as the server's | The 2 KiB description cap and the cleaning of unseen characters, the caps on messages and tools, the output policy, the sandbox where it enforces |
| **The model** | `tool_search` and `tool_call` arguments; files it may write, `.mcp.json` among them | Call an approved tool (asked by default) | The permission engine; a new or changed definition held for the person (point 4); a changed `.mcp.json` is a changed definition |

**The invariants the reviews test against:**

1. **Nothing of a project's runs before trust and approval:** no program is found, no process started and no address reached for a server that is not approved as it is now.
2. **A definition the model sees was approved by the person:** a tool new since the server's approval, or changed since its own, is not callable until approved again; a server's changed entry is not started.
3. **The request prefix does not change** when servers connect, disconnect or change their tools, except for directly exposed tools at the start of a context epoch.
4. **No credential reaches the model, a log, the session log, an export, a view of the configuration or an RPC client.**
5. **A server can stall, flood or crash only itself:** every wait has a time limit, every buffer a cap, and the session goes on.
6. **Windows behaves the same,** shims included.

### 3. The threat note

**3.1 What a project's `.mcp.json` can make the harness run, and when.**

- A project's servers come from `.cineris/config.toml`'s `[mcp.servers]` and the project's `.mcp.json` (while `compat.mcp` is on). Both are **kept apart** from the user's configuration, as a project's hooks are, and read live, through handles that follow no link (`cineris_sandbox::unfollowed`), at a session's start and at each turn's start when the file changed. Each server is **proposed**: listed by `cineris mcp list`, `mcp/status` (`needs_approval`) and the trust question, and nothing else is done with it. A name the user's configuration has is refused; between the two project files, `.cineris/config.toml` wins and the other is named in a notice.
- **It runs only after both:** the project is trusted, and the server's definition is pinned as it is now (the trust question's yes, `cineris permissions trust --mcp-server NAME=HASH`, or `cineris mcp approve NAME`). The pin is of the whole entry in a canonical form (the file it came from, its name, command, arguments, working folder, each environment entry's name and where its value comes from, URL, each header's name and source, `network`, `enabled`, `expose`, the tool lists, its description and its time limits), so a project that drops a tool from its `deny_tools`, or turns a server off and on, has changed it.
- **Then, when:** at the session's start, in the background (the lifecycle, pull request 5). A project server is started only by a session of a trusted project; a headless run that is not trusted skips them and says so.
- **Where it runs (the user's servers too):** a server starts in the workspace only when the project is trusted; otherwise in an empty folder of its own (an entry's `cwd` that resolves inside the workspace counts as the workspace). A user's server starts in every session, an untrusted clone's included, and in the workspace `npx` would run the project's `node_modules/.bin`, `python -m` would import the project's module and `cmd` would look there for a program, so that cloning a repository would run its code at the session's start (found by the review of this note).
- **What the program may be:** found as an extension's program is (`cineris-core`'s `programs.rs`): absolute entries of `PATH` only, never a folder inside the workspace or a write root, so a program the model could have planted is never found. A `command` that names a path inside the workspace is refused. **A residual, as for hook commands:** a server's arguments can name a script in the workspace (`node server.js`), which the model can write; where the sandbox enforces, the server is confined, and on Windows in the first release it is not. The approval says so, as the hook question does.
- **Leaving with trust, and removal** (Review 28's findings 4 and 5, applied here from the start): when trust is lowered during a session (`reload::apply` finds it lowered, or a turn's start sees `trust.toml` changed), the project's servers are stopped and their sources leave the registry, with a notice to the model, before the next request. A server whose pinned entry is removed, turned off or changed is stopped the same way. Servers hold no gates, so a removed server leaves nothing held.

**3.2 A program found through `PATH`, and shims on Windows.** The rule is step 28's, shared: the lookup tries `.exe`, `.cmd` and `.bat` only, one folder at a time, never reads `PATHEXT`, and sets `NoDefaultCurrentDirectoryInExePath`, which the server's own environment carries too (point 3.3), so that a shim's `cmd` does not look in its working folder either. A `.cmd` or `.bat` (`npx.cmd`, `pnpm.cmd`) runs as `%SystemRoot%\System32\cmd.exe /d /e:on /v:off /s /c` with one line we build, and an argument holding `"`, `%`, `!`, `&`, `|`, `<`, `>`, `^`, `(`, `)`, a line break or NUL, or a line over 8,000 UTF-16 units, refuses the server with the reason (`cineris mcp list` and the hint row say which argument). Best-fit look-alikes are refused on every system. `.ps1` is never run. Tested on Windows with the test binary behind a `.cmd` shim.

**3.3 The environment a server gets: an explicit list.**

- **The base:** `PATH` (absolute entries, the workspace and write roots taken out), the home folder, the locale (`LANG`, `LC_*`, `TZ`), the user's name; on Windows the system's own (`SystemRoot`, `SystemDrive`, `windir`, `ComSpec` set to the system's `cmd.exe`, `PATHEXT` at its default, `APPDATA`, `LOCALAPPDATA`, `ProgramData`, `ProgramFiles`, `ProgramFiles(x86)`, `CommonProgramFiles`, `USERNAME`, `USERDOMAIN`, `COMPUTERNAME`, `NUMBER_OF_PROCESSORS`, `PROCESSOR_ARCHITECTURE`, `OS`, and `NoDefaultCurrentDirectoryInExePath=1`); `TMPDIR` (`TEMP` and `TMP` on Windows) set to a private folder of the server's own.
- **Then each `env` entry:** `{ env = "VAR" }` takes the user's variable when the server starts (an absent one fails the start, named); a literal is passed as it is (the first run's import writes one in one case, as 13-mcp says). **A project's entry may name the user's variables, and the trust question names each one** ("gives it your `$GITHUB_TOKEN`"). `${VAR}` in a project's `.mcp.json` meets the same refusals (pull request 7 settles [a-variable-reference-under-a-credentials-name-refuses-its-server](../../outstanding/a-variable-reference-under-a-credentials-name-refuses-its-server.md) with it).
- **Never:** the providers' keys (`KNOWN_KEYS`, and any variable the harness took a credential from), `CINERIS_*`, `shell.env_remove`. An entry that names one of them as its source is refused at load, named. A name that is empty, holds `=` or NUL, or begins with a digit is refused; a name that changes how programs load (`LD_PRELOAD`, `LD_LIBRARY_PATH`, `DYLD_*`, `NODE_OPTIONS`, `PYTHONPATH` and the like) is shown as such in the approval.
- **This differs from a hook command, which gets the shell's environment** (Review 28's finding 11): the approval of a server lists what it is given and says that nothing else of the user's environment is.

**3.4 Credentials for remote servers.**

- Header values are references to environment variables (`headers = { Authorization = { env = "TRACKER_AUTH" } }`), resolved at each connection and never written; the first run's import keeps its one case of a literal, as 13-mcp says, shown masked by the import's rule in every view.
- **A project's header takes its value only from its derived variable** (the first run's import's names, `TRACKER_AUTHORIZATION`), never from a variable the project chose: else a repository could send the user's existing token (`{ env = "GITHUB_TOKEN" }`) to a host it chose, which MCP forbids a client to do. The trust question says "sends your `$X` to host".
- **Every value resolved is given to the session's secret filter by value,** as the providers' header values are, so that a server which echoes one in a result has it masked. Invariant 4 is of what the harness itself writes: a server holding a credential can still put it in its own words.
- **Where a value never goes:** a log line, an error text, the session log, an export, `config show` and each `--layer`, `config/get`, `mcp/status`, `cineris mcp list|inspect` (they show `from $TRACKER_AUTH`, or `[redacted]` for a literal), the model, any RPC client. The client's types implement `Debug` by hand, without values. An error body from a server is cut to 2 KiB and put through the secret filter before it is shown or logged, since a server may echo what it was sent. A local server's standard error is drained always, its last 8 KiB kept in memory, filtered, and shown only to the person (`cineris mcp inspect`, the status view); the model is told that a server failed and how, never its standard error.
- **The trust question shows a project's entry whole**, with only what the secret filter's formats know masked (13-mcp's rule since the step 25 fixes), so the person sees every part of the command line they approve.
- **A token goes only to the origin it was configured for:** no redirect is followed, a URL with a user part is refused (as the import refuses it), and a project's header comes only from its derived variable (above).

**3.5 Tool descriptions and results are third-party text.**

- **Reach:** a server's text reaches the model only as `tool_search` results (headed as the authors' text, "data, not instructions", already) and as tool results. Section 6 lists a server by its name, its tool count and the entry's own `description` (the user's text, or the project's approved one), through `one_line` at 200 bytes; **a server's own `instructions` and `serverInfo` never reach the prompt.**
- **Cleaned before registration:** names, titles, descriptions and every string in a schema have the characters that act unseen (bidirectional controls, tag characters, a run of variation selectors, zero-width characters) taken out by `cineris_base::visible`, and control characters escaped; the hash is of the definition as the server sent it, so a change in unseen characters is a change. The description is cut at 2 KiB in a search result (already), and so is each string of a schema a search shows; a property's name holding a character that acts unseen or a control character leaves the tool out.
- **Bounded:** a tool name must be 1 to 128 of `A-Z a-z 0-9 _ - .` (the registry's rule and the specification's own guidance); a tool with another name, a schema over 32 KiB compact or deeper than 64, or an invalid `x-mcp-header` on HTTP is left out and named in `cineris mcp inspect`. At most 512 tools a server, and 20 pages of a list.
- **Results:** text blocks as text; `structuredContent` as compact JSON when no text block carries it; an image, audio or blob as one line naming its type and size (not sent to the model in this step); a resource link as its URI and name. The result begins with one line naming the server and the tool, as the safety table's "labelled as third-party" asks, and goes through the output policy (`execute_produced`): truncation, with the whole result as an artifact. `isError` is an error result.
- **Annotations are hints for the person only:** shown by `cineris mcp inspect`, never read by the permission engine or the scheduler.

**3.6 A tool definition changed after approval.**

- **Each tool is pinned** by the hash of its canonical definition (name, title, description, input and output schemas, annotations), as `mcp_tool` `server/tool`, beside its server's `mcp_server` pin: in `pins.toml` for the user's servers (as `trust/pins.rs`' doc already says) and in `trust.toml`'s project pins for a project's.
- **A tool with no pin, or with another hash, is held:** `tool_search` shows only its full name and that it needs approval, new or changed, **with no description and no schema** (a changed description is the payload of the attack, and must not reach the model before the person has seen it); `tool_call` asks the person whatever the rules say and at every level, `unrestricted` included, with the definition in full and whether it is new or changed. The core asks it in its policy layer before the permission engine: no new effect, nothing in a tool's `run`, no change to the engine's rules; [06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md)'s table of levels says that this question is asked at every level. Its answers are two, as the question about an unconfined command's are: approve the definition and run this call, or no. A yes pins it (by the hash shown) and the call goes on to the ordinary decision, which asks by default; no rule and no supervisor's grant can answer it, and where no person can answer (headless, or no `user` client over RPC) the call is refused with how to approve it (`cineris mcp approve NAME`).
- **The first approval of a server's tools, in two steps:** `cineris mcp add`, `import` and `approve` first show the entry and take a yes bound to the entry's hash; **only then** start the server under its confinement, list its tools, show each in full, and take a second yes bound to the hash of what was shown, which pins the tools. Nothing of a server runs, and no address is reached, before the first yes (MCP's own rule for a local server: the exact command shown, and approved, before it runs). A server approved without its tools listed (the trust question, the first run's import) has every tool held until approved, one at a time at its first call, or all at once by `cineris mcp approve`.
- **No pin is written without a person** (no trust on first use), so the signed approvals that are being built alongside cover every MCP pin without a store of their own.

**3.7 A server that floods, stalls, crashes or answers out of order.**

- **Flood:** a line or an SSE event over 4 MiB ends the connection (cineris-base's framing, with our own cap, read by a thread of the server's own); **every message that is not the answer to a pending request** (notifications included: progress, logging, list changes) is counted with its bytes, responses to no pending request, duplicates and answers after a cancel are dropped, and past 1,000 such messages or 16 MiB of them a minute the server is stopped as misbehaving; a result that asks to be sent again is sent at most three times; change notifications are coalesced (one listing in flight, at most one a second); standard error is drained always and only its tail kept; queues are bounded.
- **Stall:** the probe of a server's era (3 s, inside the start's limit, so that a silent legacy server still starts), the start (10 s, `start_timeout_ms`), each listing (30 s), each call (`timeout_ms`, 60 s by default) and the shutdown (2 s, then the staged kill) have limits; a call past its limit is cancelled as its era cancels and returns an error result. The modern change stream has no limit, by the specification, but is read with a bound on its buffer.
- **Crash:** the calls in flight return an error result naming the crash (with "the call may have run"); the server is started again at its next use, with backoff (1, 2, 4 … 60 s, on the fake clock in tests); the hint row and `mcp/status` say so.
- **Out of order:** every response is matched by id; requests from a legacy server are answered (`ping` with `{}`, anything else `-32601`); a modern server's request is a protocol fault, ignored and counted.
- **A broken stream on HTTP:** the specification says a request whose stream breaks is sent again with a new id. **We do that for discovery, listing and the change stream, and not for `tools/call`,** whose effect may already have happened: the call returns an error saying the connection broke and the call may have run. A deliberate difference from a MUST, for the maintainers.

**3.8 Remote URLs.** Against what step 28's `fetch` refuses:

| | Our rule for a server's URL |
|---|---|
| Scheme | `https`; `http` only to a loopback name or literal (`localhost`, `127.0.0.0/8`, `::1`), where local servers listen |
| Redirects | Never followed; a 3xx fails the connection naming where it pointed |
| Addresses | Resolved through `fetch`'s checked resolver and pinned for the connection, every address checked: never link-local, the metadata addresses, multicast or unspecified; loopback only by a loopback name or literal (so a name cannot be rebound to it); private, CGNAT and unique-local addresses only for the user's own entries, or a project's entry whose URL names the address literally, as the trust question then shows |
| Lookups | At most one in flight a server and eight in the process (Review 28's finding 10) |
| TLS | `fetch`'s check: the system's roots, and nothing fetched to check a certificate. Not the providers' transport, whose Windows verifier fetches what a certificate names, accepted only because the user alone chooses a provider ([the item](../../outstanding/the-provider-transports-tls-check-on-windows-fetches-what-a-certificate-names.md)); a project chooses a server's URL |
| Proxies | None in this step, as for `fetch`; an item opened for a proxy that keeps the address checks |
| Sizes | 4 MiB a JSON response or SSE event, 2 KiB of an error body shown |

### 4. Approvals and trust

- **Stores:** none new. Servers and tools are `Definition`s of kinds `mcp_server` and `mcp_tool` (adding `McpTool` to the protocol's open `TrustResourceKind`), pinned through `PinStore::pin` and `unpin` for the user's, and `TrustStore::approve`, `ProjectTrust::standing` and `pinned_of` for a project's, with the method step 28's fixes add to drop one project pin. **The signed-approvals step that runs alongside changes these stores; we use only these calls, and only on a person's answer**, so its signing covers MCP with no store of ours.
- **Where an approval is given, all behind the guards:**
  - `cineris mcp add NAME -- COMMAND ARG…` or `--url URL [--header NAME=VAR]…`, `[--env NAME[=VAR]]…`, `[--network]`: writes the user's entry, starts it confined, shows it and its tools, and a yes writes the pins; with no terminal only `--hash` of what was shown approves (C).
  - `cineris mcp import [NAME…]`: the other clients' servers **from the user's own files only**, one confirmation each, as `add`. A project's files are never imported into the user's configuration, which would carry a repository's server past trust into every project; a project's servers are approved as the project's.
  - `cineris mcp approve NAME [--hash H]`: a hand-written entry of the user's, a project's proposed server in a trusted project, or held tools.
  - `cineris mcp remove NAME [--hash H]`: the user's entry and its pins.
  - `cineris permissions trust --mcp-server NAME=HASH`, beside `--extension` and `--hook`.
  - The trust question (each project server, whole) and a held tool's question in a session (point 3.6).
- **Guard A** (the engine's grammar of the harness's own commands): `mcp add`, `import`, `approve` and `remove` are approving; `list` and `inspect` are not; `permissions trust`'s new flag is already covered by its subcommand. **Guard B:** `mcp` joins `NO_SESSION`, and the four approving actions and `inspect` (which starts a server) refuse under a session's command. **Guard C:** approval by the hash shown, shared with `cineris ext` (its helpers move to a module both use).
- **`list` and `inspect` start nothing unapproved;** `inspect` starts an approved server to show its live tools, refused under a session's command.

### 5. The shape of the code

- **`cineris-mcp`** knows MCP and does no I/O of its own: it is given a way to start a server's process and a way to make an HTTP request, as traits. Its parts: the **`Client`** interface (start, list tools, call with a cancel token, a stream of changes, shut down) that the rest of the step is written against, with our client (or `rmcp`) behind it; the server's lifecycle (start, era, restart with backoff); the tool wrapper (`McpTool`, implementing `Tool`, `Exclusive` in its spec); the definitions and their canonical hashes; the fake servers of both eras for tests (scripted, speaking lines over a pipe and HTTP over the test kit's server).
- **`cineris-core`** (behind `mcp`) implements the traits: the process through `programs.rs`, the sandbox's launcher and `cineris_sandbox::tree`; HTTP through `cineris-llm`; judges trust and pins; keeps the registry and section 6 in step; answers `mcp/status`.
- **`cineris-sandbox`**: a launcher mode that gives a confined program a live standard input (today the launcher's input carries the policy, then a closed or sealed input). The policy is read by its length, given on the launcher's command line, and the rest of the pipe is the program's.
- **`cineris-tools`**: the registry knows each source's kind and refuses a source of another kind that takes a name (Review 28's note: an MCP server and an extension of one name replaced each other's tools); MCP tools are always `Exclusive`. That refusal is defence in depth: **which source has a name is decided from the configuration before anything registers** (the lifecycle, pull request 5), the user's sources over the project's across both kinds, and between the user's own extension and server of one name the extension, with a notice; never by which registered first, since servers start in the background.
- **`cineris-agent`**: section 6's sources part no longer needs the `scripts` feature (today only the script build passes a source of sections).
- **`cineris-config`**: `[mcp]` declared, replacing `Reserved`:

```toml
[mcp]
start_timeout_ms = 10000
direct_share = 5                       # percent of the context window for directly exposed definitions

[mcp.servers.docs]
command = "docs-mcp"                   # local, over standard I/O
args = ["--root", "."]
env = { DOCS_TOKEN = { env = "DOCS_TOKEN" } }
network = true                         # confined: network for this server
description = "Search the product documentation"

[mcp.servers.tracker]
url = "https://tracker.example.com/mcp"
headers = { Authorization = { env = "TRACKER_AUTH" } }
tools = ["create_issue", "find_issue"] # offer only these
deny_tools = ["delete_issue"]          # never offer these
expose = "direct"                      # "search" (the default) or "direct"
timeout_ms = 30000
enabled = true
```

  A project's `[mcp.servers]` is kept apart (as `[hooks]` is); the display keeps both of 13-mcp's rules for `args`, `url` and `command`; `env` and `headers` values are `[redacted]`. The 13-mcp example is kept as it is, with `deny_tools` and `enabled` added. **No `write` in this step:** a server has no write root but its private temporary folder, as the safety table's default says, and is confined at every level, `unrestricted` included, where the sandbox enforces, as an extension's programs are. A long-running server with a write root would need the protected paths looked at after each of its calls and at each turn's start, and stopping when a lowered level narrows it; an item is opened for it.
- **`cineris-cli`**: `cineris mcp …` (point 4); `cineris-protocol`: `mcp/status` filled, `TrustResourceKind::McpTool`, a held tool's approval detail; `cineris-tui`: `/mcp` (the servers and their state), and the held tool's question.

### 5a. Direct exposure (pull request 8)

- A directly exposed tool is declared as `<server>__<tool>`; a server whose name holds `__`, or a full name over 64 characters, cannot be exposed directly (it stays reachable through the pair), and says so.
- Its definition is cleaned and capped as a search result shows it; only approved tools are declared. The list is fixed for the epoch: a tool held, changed or removed during it stays declared until the next epoch, and a call of one is held (its question) or refused as gone; a tool new during it is reached through the pair until the next.
- The warning past `direct_share` of the context window counts the declared definitions' tokens.
- A change of sources is told by section 6's notice at most once a turn, and only when its text changed since the last one, so that a server that comes and goes does not cost a notice each turn.

### 6. The open items the step owns

- **`a-tool-that-makes-links-beside-other-calls`:** MCP tools are `Exclusive` in their spec, so `tool_call` schedules every call of one alone whatever its plan says, as extension tools are. With both kinds of outside tool exclusive, nothing that declares its own scheduling is left: settled, deleted.
- **`a-kept-run-adopted-after-its-call-moved`:** an exclusive tool is never started early, so no MCP call reaches it; the defect stays in the round's cache. The item names the fix (do not adopt a kept run for a call whose plan moved, `Call::moved`), with a test. **Proposed: make that fix here**, a few lines in `cineris-agent` with its test, and delete the item; else leave it for the next change to the cache.
- **`the-binary-near-its-size-budget`:** this step's sizes go in its records and decision 0039's revisit; the item stays open for step 34.

### 7. The pull requests

Each with its own review, about 4,000 lines at most outside tests:

| | Pull request | Crates | Depends on the client choice |
|---|---|---|---|
| 1 | **The server's process and the configuration:** the launcher's live input, the explicit environment, the shim rule shared, the staged shutdown; `[mcp]` declared, kept apart for projects, its display rules | sandbox, core, config, base | No |
| 2 | **The registry and the tool:** source kinds, `McpTool` (exclusive, labelled, cleaned, results mapped through the output policy), section 6 without `scripts`, the notice on a change of sources, the prefix's tests, the two open items | tools, mcp, core, agent | No (against a fake `Client`) |
| 3 | **Hashing and approval:** canonical hashes, pins of servers and tools, held tools and their question, the trust question's servers, leaving with trust, guards A, B and C, `TrustResourceKind::McpTool` | mcp, policy, protocol, core, cli, sandbox | No |
| 4 | **The client:** JSON-RPC, both eras over standard I/O, the probe and fallback, listing, calling, cancellation, changes, the fake servers of both eras | mcp | **Yes** |
| 5 | **The lifecycle:** the background start, the era kept, changes into the registry, restart with backoff, shutdown, `mcp/status`, the hint row, start-up measured | mcp, core, tui | No (on the interface) |
| 6 | **Remote servers:** Streamable HTTP in both eras over `cineris-llm`, static credentials, the URL rules, the lookup cap | llm, mcp, core | Its protocol part, yes |
| 7 | **Configuration and management:** `.mcp.json` read live and proposed, the first run's imports wired (and VS Code's `servers`, Claude Code's per-project entries), `cineris mcp add\|list\|inspect\|import\|approve\|remove`, `/mcp` | config, core, cli, tui | No |
| 8 | **Direct exposure, the build without `mcp`, the close:** `server__tool` at the next epoch with the 5% warning, the `no-mcp` build in `check`, the size record | core, agent, mcp, xtask | No |

1, 2, 3 and 4 are built in parallel, each against the shared interface and the others' briefs; they merge in that order, each rebased on the last. 5 needs 1 to 4; 6 needs 4 and 5; 7 needs 3, 5 and 6 (`add --url`, `inspect`); 8 is last. The `no-mcp` build's check comes with 2, not 8, since the binary turns `mcp` on. The guards' grammar (A's fallback lists, B's list) comes with 3; the `cineris mcp` subcommands and their own entries in the binary's grammar come with 7. **Early reviews:** 1 (the process, its environment, the launcher's live input), 2 (a server's text into the prompt and the search results), 3 (the permission engine and the stores), 6 (credentials, URLs, TLS) and 7 (a project's files read live, and the import). **What this step waits on from step 28's fixes:** `one_line`'s run of variation selectors (finding 6), before 2 merges; the socket's refusal of another harness's client (finding 3), before 3 merges, since that client could otherwise answer the trust and held-tool questions that now pin MCP definitions; the method that drops one project pin.

### 8. What this asks of the maintainers and the owner

1. **The owner:** the client (point 1): our own, as recommended, or `rmcp`.
2. **The maintainers:** `rmcp`'s server as a dev-dependency for a nightly interoperability check (point 1).
3. Legacy HTTP without `2025-11-25`'s polling and resumption (point 1).
4. Not sending a `tools/call` again after a broken HTTP stream (point 3.7).
5. No pin without a person: tools held until approved, with no trust on first use, and a held tool asked at every level (point 3.6).
6. A user's hand-written server entry not started until approved (point 4), where the user's own hooks run unpinned.
7. The URL rules (point 3.8), and no proxy in this step.
8. The fix of `a-kept-run-adopted-after-its-call-moved` here (point 6).
9. **After the review of this note:** a server starting in the workspace only in a trusted project, and in a folder of its own otherwise (point 3.1); approval by command in two steps, and `import` of the user's own files only (point 3.6, point 4); a project's header only from its derived variable (point 3.4); no `write` in this step, with an item (point 5); a held tool shown to the model by name only (point 3.6); `fetch`'s TLS check for servers (point 3.8).
10. **Proposed, a change to the engine's rules that needs the maintainers' yes:** the project's `.mcp.json` (and `.cursor/mcp.json`, `.vscode/mcp.json`) on the list of protected paths, beside `.cineris/` and `.npmrc`, so that at `autonomous` the model cannot write a server's definition unasked. Approval stays the wall either way: a changed file is a changed definition.

### 9. The pages and decisions this changes

- [Decision 0010](../../decisions/0010-mcp-in-first-release.md): amended, our own client in place of `rmcp`, with `rmcp`'s server as the nightly interoperability check; its "Revisit when" kept for the other direction.
- [13-mcp.md](../../architecture/13-mcp.md): "Implementation" (our own client), "Configuration" (`deny_tools`, `enabled`, the user's own entries approved before they run, a project's headers from their derived variables), "Lifecycle" (the working folder, the probe's limit), "Safety" (held tools, the two-step approval, a call not sent again after a broken stream, with why).
- [01-workspace.md](../../architecture/01-workspace.md): `cineris-mcp`'s row ("rmcp (isolated here)" becomes no third-party protocol crate), and the rule that only `cineris-mcp` links the MCP SDK.
- [06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md): the table of levels (a held tool asked at every level), MCP pins, the guards' new entries.
- [05-tools.md](../../architecture/05-tools.md): kinds of source, and what a held tool shows in a search.
- [08-configuration.md](../../architecture/08-configuration.md) and the generated reference: `[mcp]` declared.
- [security.md](../../security.md) §3 and §4: the rows this note changes.
- [dependencies.md](../../dependencies.md): a row for `rmcp` in `interop/`.
- [measurements/binary-size.md](../../measurements/binary-size.md) and decision 0039's revisit: the measured size of `rmcp`, and this step's own.

## What the independent review of the note found, on 2026-11-27

A reviewer who had not seen the note's reasoning read it against the specification, the pages and the code it builds on: 2 high, 8 medium and 9 low findings. The size arithmetic of point 1 was checked and holds. What each changed is above; in short:

| Finding | What changed |
|---|---|
| **High:** a user's server started in the workspace runs a cloned repository's code at the session's start (`node_modules/.bin`, a module beside it, `cmd`'s look in its working folder) | The workspace only in a trusted project; a folder of its own otherwise; `NoDefaultCurrentDirectoryInExePath` in the server's environment (points 3.1, 3.2, 3.3) |
| **High:** `import` and `approve` would start a server before the person saw it, and `import` of a project's file would carry its server past trust | Two steps, nothing started before the first yes; `import` of the user's own files only (points 3.6, 4) |
| A project's entry could send the user's token to a host it chose | A project's header only from its derived variable; the trust question names every variable a project's server takes (points 3.3, 3.4) |
| The held tool's question had no mechanism at `unrestricted`, and a search would show the changed description | Asked by the core before the engine, with two answers; shown to the model by name only (point 3.6) |
| A name decided by which source registered first | Decided from the configuration first (point 5) |
| A server with a write root has no "after" for the protected paths, and outlives a lowered level | No `write` in this step, an item opened (point 5) |
| The providers' TLS check on Windows fetches what a certificate names, from a URL a project chose | `fetch`'s check (point 3.8) |
| Valid notifications could flood the session | Every message that is not an answer counted, with its bytes (point 3.7) |
| No list of the pages and decisions changed | Point 9 |
| The order of the pull requests; early reviews of 6 and 7; what waits on step 28's fixes; the `no-mcp` check last | Point 7 |
| Low: a dev-dependency cannot be optional; the outlook's method and what it leaves out; a schema shown whole past the description cap; direct exposure's rules; the probe's limit; the echo of a credential; `.mcp.json` not protected; a notice each turn from a flapping server | Points 1, 3.4, 3.5, 3.7, 5a, 8 (item 10) |
