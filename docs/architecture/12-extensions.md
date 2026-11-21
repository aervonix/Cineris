# The extension system

Extensions let users add their own tools, slash commands and hooks without changing the harness. The [vision](../vision.md) gives the extension system a concrete job on top of that:

- **Self-written extensions.** The user asks for a new tool or command; the harness writes it, the user reviews it, and it loads at once.
- **Sandboxed**, for an audience that includes Windows users, where no OS sandbox exists in the first release.
- **Hot-reloaded**, like configuration, themes and key bindings.
- **For everyone.** It cannot assume that Python, Node or a compiler is installed.

This page compares the ways to build it, recommends one, and specifies it. The choice is recorded in [decision 0005](../decisions/0005-extension-mechanism.md). The owner accepted it on 2026-11-12.

## What an extension can add

| Contribution | What it is | Example |
|---|---|---|
| **Custom command** | A prompt template in a markdown file. No code. | `/review` expands to the team's review checklist. |
| **Skill** | A folder of instructions the model reads on demand. No code of ours. | "How we write database migrations." |
| **Hook** | Code that runs when something happens in the agent's lifecycle. It can observe, add context, or decide. | Refuse edits to generated files. Run the formatter after every edit. |
| **Tool** | A function the model can call. | Query the team's issue tracker. |
| **Command with code** | A slash command handled by code. | `/deploy-preview` runs a script and reports back. |

## The options

### Three mechanisms

The same tool (JSON in, JSON out) was built behind each mechanism and measured, together with the cost each adds to the host. Method and full tables: [../measurements/extension-mechanisms.md](../measurements/extension-mechanisms.md) and [../measurements/dependency-costs.md](../measurements/dependency-costs.md). Reference-machine figures.

| | WASM components | Subprocess speaking JSON | Embedded scripting |
|---|---|---|---|
| **What it is** | Extensions compiled to WebAssembly and run in-process by an embedded runtime. | Extensions are ordinary programs. The harness starts them and exchanges JSON over standard I/O. | An interpreter inside the harness runs extension scripts. |
| **Can the harness write one and load it at once, on any machine?** | No. The source must be compiled to WASM first, which needs a toolchain on the user's machine and about 13 s per build. | Only if the right runtime (Python, Node) happens to be installed. | **Yes.** The interpreter is in the binary. |
| **Sandbox when the OS cannot help** | Strongest. No access to anything the host does not hand over. | None. The process has the user's rights. | Strong. A script has only the host functions it is given. |
| **Start-up** | 0.14 ms precompiled, 10 ms when compiled at load | 0.34 ms native, 10 to 16 ms for a script runtime | 0.04 to 0.12 ms |
| **Per call (200 bytes)** | 1.4 µs | 27 to 35 µs | 4 to 20 µs |
| **Per call (64 KiB)** | 0.06 ms | 0.10 to 0.31 ms | 0.8 to 5.3 ms |
| **Memory** | 0.3 MB per extension; 17 MB transiently when compiling at load | A process each: 2 MB native, 11 MB Python, 23 MB Bun, 47 MB Node | 0.2 to 1.4 MB per extension |
| **Cost to the host** | The largest dependency measured: 10.2 MB of binary and 65 s of clean release build on 4 cores when added to the harness (10.5 MB and 86 s built alone), and a new major version about monthly. | Nothing beyond what the harness already has. | Small to moderate, depending on the language. See the next table. |
| **Linux and Windows** | Identical. | Script shims, no shebang lines, newline translation, code pages, signals. | Identical once built. |
| **Access to libraries** | Whatever compiles to WASM. | Everything. | Only what the host exposes. |

**What decides it.**

- Per-call speed decides nothing. Every option adds microseconds to a tool call that sits inside a model round trip of seconds.
- The vision's requirements decide it. Only embedded scripting lets the harness write an extension and load it immediately on a machine with nothing else installed, with a sandbox that works on Windows today.
- Subprocesses keep one job: tools that need a real ecosystem (an official client library, a native tool). That job already has a standard, MCP, which the harness supports anyway ([13-mcp.md](13-mcp.md)).

### Which language

With embedded scripting chosen, the candidates are the languages themselves, plus WASM as the target for code the harness writes.

| | JavaScript on QuickJS | JavaScript on a pure-Rust engine (Boa) | Lua 5.4 | Rhai | Rust compiled to WASM |
|---|---|---|---|---|---|
| **Familiar to extension authors** | Very widely known. The first language of most people who script their tools. | The same language. | Known from editors and games; a minority. | Almost unknown. | Known to few; hard to write casually. |
| **Models write it correctly on the first try** (19 tools, a strong and a small model, 38 attempts) | 38 of 38 | 38 of 38 (the same sources) | 38 of 38 | 34 of 38. All four failures were the small model calling functions that do not exist. | 38 of 38 compile and pass, given a compiler. |
| **Start a runtime** | 0.07 ms | 0.15 ms | 0.04 ms | 0.12 ms | 0.14 ms precompiled |
| **Per call, 200 bytes** | 20 µs | 120 µs | 4 µs | 8 µs | 1.4 µs |
| **Per call, 64 KiB of text processing** | 5.3 ms | 155 ms | 0.8 ms | 1.9 ms | 0.06 ms |
| **Memory per runtime** | 0.9 MB | 4.2 MB | 0.2 MB | 1.4 MB | 0.3 MB |
| **Added to the harness binary** | 1.1 MB | 6.7 MB | 0.4 MB | 2.4 MB | 10.2 MB |
| **Added to a clean release build of the harness (4 cores)** | 8.6 s | 55 s | 1.5 s | 11 s | 65 s |
| **Sandbox with no help from the OS** | Strong: no ambient access; memory limit and interrupt hook. The engine is C, so its own bugs are a risk. | Strong, and the engine is memory-safe Rust. | Strong: libraries are chosen by the host; memory limit and instruction hook. The engine is C, small and old. | Strong, memory-safe Rust, with built-in operation limits. | Strongest. |
| **Build needs** | A C compiler. Its Windows (MSVC) support in the Rust binding must be confirmed. | Rust only. | A C compiler. Well-trodden on Windows. | Rust only. Brings one MPL-licensed crate. | Rust only for the host. A full toolchain on the user's machine to write one. |

How the first-try experiment was run, and its limits, are in the measurement page: the code was written by subagents of the session that produced these docs, told to write each file once without running anything, and then executed in the real engines against fixed tests.

**Reading the table.**

- **Models do not separate JavaScript, Lua and Rust** at this size of tool. Rhai trails with a weaker model, because little Rhai exists to learn from.
- **People do separate them.** The audience is the broad public, and JavaScript is the language most of them already write.
- **The pure-Rust JavaScript engine is too slow for the job.** A tool that tidies a 64 KiB command output takes 155 ms in it on the reference machine and 296 ms on the second machine, about 30 times longer than in QuickJS, and it costs six times the binary.
- **WASM is the strongest sandbox and cannot do the job**: the harness cannot write and load one on a machine without a toolchain.

## Decision

1. **Extensions are JavaScript, run by an embedded QuickJS engine.** One runtime per extension, with memory and time limits, and no access to anything except host functions that are checked one by one.
2. **Tools that need an ecosystem are MCP servers.** There is no second, bespoke process protocol.
3. **The simplest hooks can be a command line** in configuration, with no script.
4. **Fallback: Lua.** If the QuickJS binding does not build cleanly on Windows in the extension step's first hour, the step switches to Lua 5.4, which is smaller, faster and proven on Windows. The host functions are defined once in Rust and are the same for either language, so the switch is contained.

**What is given up.** The script engine is a C library inside the harness. A memory-safety bug in it could, in principle, let a script escape. Against that: scripts are code the user approved, each has a manifest that bounds what it may ask for, and the engine is the one C dependency accepted. The alternative that removes this risk, a pure-Rust JavaScript engine, measured too slow today.

**When to revisit.**
- A pure-Rust JavaScript engine reaches within a few times of QuickJS: switch engines, same language, same extensions.
- A public registry of third-party extensions is wanted: reconsider a WASM tier for code the user has not read.
- Models' reliability in JavaScript and Lua diverges on larger tools.

## The tiers

Each is simpler than the next. Most needs are met by the first three.

### Tier 1: custom commands (files, no code)

A markdown file is a slash command.

```markdown
---
description: Review the staged changes
arguments: [focus]
---
Review the staged changes in this repository. Focus on {{focus | "correctness"}}.
Report problems as a list, most serious first.
```

- **Where.** `<workspace>/.cineris/commands/*.md` and `<config dir>/commands/*.md`. The file name is the command name. Project commands override user commands of the same name.
- **Front matter.** A description for the menu, named arguments with optional defaults, and optionally a settings profile for the turn.
- **Body.** Sent as the user's prompt, with arguments substituted. `{{args}}` is everything typed after the command.
- **No execution.** A command file cannot run anything or include files by itself. It is text for the model, so it needs no trust.

### Tier 2: skills (folders, read on demand)

A skill is a folder with a `SKILL.md` that starts with a name and a description. This is the open Agent Skills layout, read as it is so that people's existing skills work.

- **Where.** `.cineris/skills/`, `.agents/skills/` and `.claude/skills/`, in the project and in the user's home.
- **In the prompt:** each skill's name, description and path, and nothing else. The model reads the file when a task calls for it.
- **Scripts inside a skill** are just files. If the model runs one, it runs through the shell tool under the normal permissions.
- Skills need no trust, for the same reason as instruction files.

### Tier 3: hooks

A hook runs when an event matches. It is either a command line or a function in an extension script.

```toml
[hooks.format-after-edit]
event   = "tool.after"
tools   = ["edit", "write"]
command = ["cargo", "fmt", "--", "{path}"]
observe = true
```

**Events in the first release.**

| Event | When | A hook may |
|---|---|---|
| `session.start` | A session opens. | Add context. |
| `prompt.submit` | The user sends a prompt, before it reaches the model. | Add context. Refuse the prompt with a message. |
| `tool.before` | A tool call has been planned and passed policy, before approval and execution. | Allow, deny, ask the user, or change the arguments. |
| `tool.after` | A tool call finished. | Add a note to the result. |
| `turn.end` | A turn completed. | Observe. Add a notice for the next turn. |
| `compaction.before` | Before a summary is requested. | Add instructions for the summary. |
| `session.end` | The session closes. | Observe. |

The set is small on purpose. Every event is a compatibility promise, and both reference harnesses show how large an event list grows when adding is free.

**Contract, the same for both forms.**

- **Input.** One JSON object: the event name, session and turn ids, the workspace path, and the event's own fields (for tool events: the tool name, the arguments, the plan summary and, afterwards, the result). A command receives it on standard input. A script function receives it as its argument.
- **Output.** Nothing, or one object.

| Field | Meaning |
|---|---|
| `decision` | For gating events: `allow`, `deny` or `ask`. Absent means no opinion. |
| `reason` | Shown to the user, and to the model when the decision is `deny`. |
| `arguments` | For `tool.before`: replacement arguments. They are validated, re-planned and re-checked by policy, and the change is shown to the user. |
| `context` | Text appended to the conversation as a notice, labelled as coming from the hook. Capped at 8 KiB. |

- **Failure and timeout.** A gating hook that fails or times out counts as `deny`, with the failure shown. A hook marked `observe = true` cannot decide anything, and its failure is logged and ignored. Default timeouts: 5 s for gating events, 60 s for observers.
- **Several hooks on one event** run concurrently. Decisions combine as `deny` over `ask` over `allow`. Argument changes apply only if exactly one hook proposes one. The outcome does not depend on order.
- **`{path}`-style placeholders** in a `command` are filled as separate arguments. They are never interpolated into a shell string. Hook commands run in the sandbox ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md)).

### Tier 4: script extensions

An extension is a folder with a manifest and JavaScript.

```
<config dir>/extensions/tracker/
    extension.toml
    main.js
```

**The manifest** says what the extension is and what it may ask the host to do.

```toml
api = 1                         # the version of this format and of the host functions
name = "tracker"
version = "0.1.0"
description = "Issue tracker tools"
main = "main.js"
prompt = "prompt.md"            # optional: text for the system prompt, 4 KiB

[capabilities]                  # each is empty unless given
exec  = ["gh"]                  # programs it may ask the host to run: bare names or absolute paths
exec_network = true             # whether those programs may reach the network
read  = ["**"]                  # workspace paths it may read
write = []                      # workspace paths it may write
net   = []                      # hosts it may contact through the host's fetch
hooks = ["tool.before"]         # the events its script may hook

[limits]                        # optional, within the hard caps
memory = 32                     # MiB, at most 64
time = 120                      # seconds a tool call, at most 600
```

**Read strictly** (since step 28): a key this build does not know, a value of the wrong type, or an `api` it does not know refuses the extension, with the reason. The capabilities are values the harness enforces ([decision 0028](../decisions/0028-enforced-values-fail-closed.md)). The folder holds only regular files and folders, at most 256 files of 1 MiB each and 4 MiB in all: a link of any kind anywhere in it refuses the extension. What the events a script may hook let it see is shown in words when it is approved ("every prompt you send"), since a hook sees the session's content. Since the step's second pull request the manifest also refuses a backtick in a program, a glob or the version, and a program or a glob of more than 256 bytes: each is shown as code in an approval, and a backtick would end the code and let what follows read as the harness's words.

**The script** registers what it contributes.

```js
cineris.tool({
  name: "find_issue",
  description: "Search issues by text",
  input: { type: "object", properties: { query: { type: "string" } },
           required: ["query"], additionalProperties: false },
  readOnly: true,
  async run(args) {
    const r = await cineris.exec("gh", ["issue", "list", "--search", args.query, "--json", "number,title"]);
    if (r.code !== 0) throw new Error(r.stderr);
    return { issues: JSON.parse(r.stdout) };
  },
});

cineris.on("tool.before", { tools: ["edit", "write"] }, (event) => {
  if (event.arguments.path.startsWith("generated/")) {
    return { decision: "deny", reason: "generated files are rebuilt from schema/" };
  }
});
```

**Host functions.** This is the whole surface a script has. There is no file, process, network, timer or module API other than these.

| Function | Does | Checked against |
|---|---|---|
| `cineris.tool(def)`, `cineris.command(def)`, `cineris.on(event, filter, fn)` | Register contributions, only while the main module is first evaluated. | The manifest's name and API version; for `on`, `capabilities.hooks`. |
| `cineris.exec(program, args, options)` | Asks the host to run a program. Returns its exit code, and its standard output and standard error apart, each bounded. Its arguments and input together, as the script passes them, are at most 8 MiB. | `capabilities.exec`, the program exactly as listed; then the permission engine as an `exec` effect (and a `network` one with `exec_network`); then the sandbox, with write roots made from `capabilities.write`. The engine judges the program's line, its arguments, and not its input (`options.stdin`): a question about a program given input shows how long it is and its first lines (at most three, and 200 bytes, escaped). |
| `cineris.fs.read`, `cineris.fs.write`, `cineris.fs.list`, `cineris.fs.search` | Workspace file access through the same code as the built-in tools: a write through the `write` tool (its checkpoint, its protected paths), a file overwritten only after the script read it as it is; a listing or a search through the `list` and `search` tools, whose walk is gated by the manifest on every entry. | `capabilities.read` and `write`, against where the path leads; then the permission engine. |
| `cineris.http.fetch(url, options)` | An HTTP request made by the host, through `cineris-llm`'s fetch client (since step 28): `https`, or plain `http` to an address the manifest names literally (or `localhost`); the name looked up once and the connection made to an address that was checked; a certificate checked against the system's roots with nothing fetched; no redirect followed (a 3xx is returned), no proxy, no cookies; `Host`, the hop-by-hop headers, `Content-Length`, `Expect` and any `Proxy-` header refused, a URL of at most 8 KiB, at most 64 headers of 64 KiB, a request body of 1 MiB, a response cut at 4 MiB, and 30 s at most (less if the script asks, or its call has less left). | `capabilities.net`, then what the client would refuse before its lookup, then the permission engine as a `network` effect on the host (its question shows the URL up to its query, and names the sizes of what is sent and the headers' names, never their values), then every address the name has: one in `cineris_base::address::RANGES`, or an IPv6 form of one, only where the manifest names that very host literally, as an address or `localhost`. The host they judge is the host connected to: a URL the URL standard would read as another (`https:good://evil`, a `\`, a `%`, `0x7f.1`) is refused. The machine's own and its network's addresses of the internet are not known here, and a wildcard reaches them ([security.md](../security.md), section 2). |
| `cineris.ask(question, choices)`, `cineris.notify(text)`, `cineris.progress(text)` | Talk to the user through the front end. Not `ask` in a hook. | Caps on length and count (since step 28: in one call, five questions of at most eight choices, whose answer is one of them or none; ten notices; a hundred progress lines), and every character that acts unseen shown as an escape. |
| `cineris.state.get(key, {scope})`, `cineris.state.set(key, value, {scope})` | A small key-value store of the extension's own, of JSON values (since step 28). `get` of a key with no value gives `null`; `set` to `null`, or with no value, removes the key, so a stored value is never `null`; a set that changes nothing writes nothing. **`session`** (the default) is kept in the session's log, one `ext` record a change ([07-sessions-and-context.md](07-sessions-and-context.md)): a resume has the values as the session left them, a fork as they were at its point, and a rewind leaves them, as a record belongs to the session and not to a branch; a session with no log keeps them in memory. **`project`** is kept in the state directory, one file an extension and project, `projects/<project key>/extensions/<source>-<name>.json`, `{"version": 1, "values": {…}}`: read strictly, so a file this build did not write (a field it does not know, another version, bad JSON, a key it would not take, a `null` value, more than 1 MiB) refuses the call with the reason and is left as it is, never written over; changed under its lock and written whole, owner-only and atomically; and a link at its name is neither read through nor written through. Both are keyed by where the extension comes from and its name (`user/tracker`, `project/tracker`), so a project's `tracker` never reads or writes the user's `tracker`'s values. With no state directory, `project` values cannot be kept, and the call says so. | No capability and no permission question: it touches only the extension's own values. 64 KiB of values an extension a scope, counted as the compact JSON of them all: a set that would take them past it, and grow them, is refused with the reason, and nothing is kept. A key of 1 to 256 bytes. At most a hundred `set` calls in one call of the extension. And 16 MiB of `session` records an extension a session, each record counted as its `ext` and `data` in compact JSON and 128 bytes for the rest of its line, from the records already in the log when the values are first read, so a resume does not start it again: past it a set, a removal included, is refused with the reason, and nothing is written; a new session starts again. Why each bound is what it is: [decision 0041](../decisions/0041-an-extension-s-state-is-kept-in-the-session-s-records-and-a-project-file-read-strictly-within-fixed-bounds.md). |
| `cineris.log(...)`, and `console.log`, `info`, `warn`, `error`, `debug` as its names | Writes to the log. | A hundred lines in one call (since step 28), each shown as `notify` is. |

There is no function that returns credentials, reads the session, changes configuration or grants permission. **The manifest is asked first**, in code (`cineris_ext::host::serve`): a call it refuses never reaches the session. Then each call is decided by the permission engine as a call of the extension's tool (`tracker/find_issue`) with the built-in tools' default and an `ext` effect beside its own, so a rule about the extension covers its host calls too, and a question it needs offers only "allow once" and "deny" and keeps no rule (since step 28: a rule kept from it would reach past the extension).

**The runtime.**

| Property | Design |
|---|---|
| Isolation | One runtime per extension. Extensions cannot see each other. |
| Limits | Memory per runtime (default 32 MB, hard cap 64 MB, the manifest's `[limits]` within them). The stack, 256 KiB. Time per call (2 minutes for tools, the hook timeouts for hooks), enforced by the engine's interrupt hook, so a tight loop is stopped, and by the host, which stops waiting. A native function that loops without looking at the interrupt cannot be stopped: its thread is left running when its grace is up, and its extension is not started again in the process (since step 28; past four such threads, none is). A result is at most 1 MiB of JSON. |
| Language | JavaScript modules. Imports resolve only inside the extension's own folder, and are served from the snapshot of it whose hash was approved, never from the disk. Types for the host functions ship as a declaration file for editors. |
| Threading | **One thread per runtime** (since step 28, in place of one thread for all: a script that computes until its time is up never holds up another extension's hook), made the first time something of the extension's is used. **A runtime runs one call at a time,** to its end, and a call's time starts when it starts running: the interrupt check belongs to the runtime and cannot tell two calls apart. Host functions are asynchronous: a script awaits them, and the core does the work. A thread that does not come back within a grace after its deadline is abandoned, and its runtime counts as torn down. |
| Start-up cost | None. What an extension registers is cached with its hash when it is approved. At start-up the harness reads the cache and knows the tools, commands and hooks without starting the engine. The engine starts the first time something of the extension's is used. |
| Failure | An exception is an error result for that call. A runtime that hits a limit is torn down and recreated on next use. The session is never affected. |

**Tools from extensions** are reached through `tool_search` and `tool_call` ([05-tools.md](05-tools.md)), so adding one never changes the declared tool list. **An extension's tool always runs alone** (since step 28), whatever its definition says: its script may make a link, and a call beside it would have been decided before the link was there. `readOnly` is shown to the user and decides nothing. Declaring an extension's tool directly from the next context epoch (`expose = "direct"`), and promoting frequently used ones, are left to step 30, which needs the same for MCP; the manifest has no such key until then.

**As built in step 28** (`cineris-ext`; the core's `ext` module; [steps/28/answered.md](../steps/28/answered.md)): the folder's snapshot and hash (`folder`), the manifest (`manifest`), the capabilities compiled for a workspace (`capability`), the host functions' contract and the order they are checked in (`host`), and the engine (`engine`, behind `scripts`). The core registers an approved extension's tools from what it registered when approved (since the step's second pull request, the extensions each session finds itself, below; `CoreOptions::extensions` stays as a test's seam, which hands an extension in with no pin and no registration cache) and serves their host calls. A host call's arguments are bounded by size before the harness takes them, and at most sixteen calls a script awaits wait for their answers at once: the memory they take is the harness's, which the runtime's limit does not count. A path the manifest refuses is refused by the script's own text, and without being resolved where the manifest grants no reading or writing, so a script learns nothing of where a path outside leads. A write or a walk acts only on the path the manifest judged, and a read decided while a person was asked opens its file only if the path still leads there. A host call's "allow once" allows that call alone. A host call that finds its turn stopped ends the script's call, and so does a program whose look at the protected paths stops the turn: a script that catches the error does not go on. A program decided after a person was asked is given only what is left of the call's time, and what it did is looked at whatever ended it (its own time, the call's cancellation or its deadline). The look at what a program changed in the workspace runs the harness's git inside the sandbox wherever the program's launcher confined it, `unrestricted` included: the program may have written the repository's configuration, which can name a program for git to run.

**As built in step 28's second pull request: the extensions a session finds and loads** (`cineris-ext`'s `discover` and `cache`; `cineris-core`'s `ext::found`; [steps/28/parts/pr2a1-found.md](../steps/28/parts/pr2a1-found.md)).

- **Where extensions are found** (`cineris_ext::discover`).
  An extension is a folder directly under one of two roots, the user's `<config dir>/extensions/` and the project's `.cineris/extensions/`, whose manifest gives the folder's own name: one that gives another is left out with both names, and a file beside the folders, or a folder whose name begins with `.`, is passed over.
  Each folder is read whole into the snapshot whose hash is pinned.
  A root is found through the session's path resolver and judged where it leads, compared by place: the user's may be a link, and is followed; the project's must lead inside the workspace, so a `.cineris` or a `.cineris/extensions` that is a link out of it gives nothing and says so; an extension's folder reached through a link is refused, as a link inside one is; the same folder given as two roots is looked in once.
  **A root that is not there costs one look,** and the resolver is not asked: the user's, given as an absolute path, is looked at as it is, and the project's one name at a time from the workspace's real location, without following a link (two looks for a workspace whose `.cineris` has no `extensions`).
  That is 1.2 µs a session with no extension on the reference machine, against 21 µs for resolving both roots (a release build, on an otherwise idle machine).
  At most 1,024 entries of a root are listed, and the rest of it is not looked at, which is said; of those, at most 64 folders are read, the first by name, and the rest are left out, counted.
  The user's `tracker` and the project's `tracker` are both found, two extensions kept apart by where they come from.
  Within one source, extensions whose names differ only in case (`Tracker` and `tracker`, in one root or in two roots of one source) are all refused, on every system, each left out naming the others: on Windows they are one name and would share what an extension keeps, and none is kept, so that nothing depends on which was listed first.
  Every refusal names the folder, with every character that acts unseen written as an escape, since a folder's name comes from a repository.
  Discovery reads folders, and runs on the blocking pool, off the start-up path.
- **The registration cache** (`cineris_ext::cache`) is `<state dir>/extensions/registered.json`, `{"version": 1, "entries": {"<content hash>": {"name", "api", "tools", "commands", "hooks", "prompt"}}}`: what each approved extension's script registered, under the hash of its folder, with the text of its prompt file, so that a session knows them without starting the engine.
  **It is read strictly** ([decision 0028](../decisions/0028-enforced-values-fail-closed.md)): a field this build does not know, a value of the wrong type, a key that is not a content hash or that is given twice, another `version`, more than 1,024 entries, an entry past a registration's caps (64 tools, 32 commands, 32 hooks, a prompt of 4 KiB, a name of 1 to 64 bytes), or a file over 16 MiB or that is not a regular file sets the whole file aside.
  A file set aside is read as an empty cache, and moved to `registered.json.set-aside` while it is still the file that was read, so that a cache another session has just written is not moved for what the one before it held.
  A missing file is an empty cache.
  The file is written owner-only and atomically, never through a link at its name and never past what it would read: a write that would make a file this build sets aside writes nothing.
  Two sessions that write it at once may lose one entry; that fails closed too, since the next session registers that extension again, and no entry can be wrong, since each is keyed by the hash of what registered it.
- **What a session looks in.**
  The user's folder, given as the configuration directory is (the session's path resolver takes `~`, a relative path, `..` and a link), and the project's, under the workspace's real location, only while the project is trusted (in the trust store, or for the run): a session of a project that is not trusted does not look in it.
  Each extension found has its definition (kind `extension`, its name, the hash of its folder) judged against its pin: the user's in `pins.toml`, a project's in the project's entry of `trust.toml`, or, after a yes the store could not keep, the hashes that question showed, for that session alone.

| Found | Its pin | In the session |
|---|---|---|
| The user's | with its hash | loaded |
| The user's | with another hash, or none | not loaded; the session says so, with `cineris ext add <folder>`, the folder under `~` |
| The user's, `pins.toml` unreadable | any | not loaded; the session says the store is why |
| The user's, `[extensions.<name>] enabled = false` | any | not loaded, and nothing is said or asked |
| A project's | with its hash | loaded |
| A project's | with another hash, or none | not loaded; the session says so, with the trust question (`cineris permissions trust` under `--trust-project`) |
| A project's, of a name a folder of the user's has | any | not used; the session says why |

- **`cineris ext add` is the step's third pull request.**
  Until it is built, nothing in the harness writes `pins.toml`, so a user's own extension is pinned by no command, and the notice names the command that will pin it.
- **A name is the user's** whenever the user's folder of extensions holds a folder of that name, compared with ASCII's case folded, whether that folder could be used or was left out (a link in it, a manifest refused): a user's extension left out does not free its name for a project's, to which the user's rules about it would apply.
  A user's folder that cannot be listed whole takes every name.
- **Everything is decided before anything runs.**
  The folders, the pins, the trust store and the registration cache are read, and every standing decided, before any script is evaluated.
  An approved extension's tools, commands, hooks and prompt text are taken from the cache by its hash, with no engine started.
  One the cache misses fails closed: the snapshot whose hash was compared with the pin is evaluated once in a throwaway runtime, the session says so ("Registering the extension …"), what it registered is written to the cache, and it is loaded.
  No runtime is made for the session before an extension's first call.
  What discovery left out is said, with its reason.
  A cache set aside is said once, with its files under `~`, and every approved extension is registered again.
  A session that found an approved extension then prunes the cache of the hashes no pin names, the user's and every trusted project's, since projects share it, and of nothing when a store cannot be read.
- **Where it runs, and what waits.**
  On a task the core spawns once the session's loop runs, off the path that opens the session and so after the interactive application's first frame: the reads and writes on the blocking pool, each evaluation on a thread of its own.
  The first turn waits for the extensions' tools (`turn/start` waits before it hands its prompt to the loop, as it waits for the look at the configuration's files), not for the cache, which is written and pruned after.
  A session closed meanwhile ends the task, and so does the core's end.
- **Measured** (release build, an otherwise idle reference machine, 40 runs): a cache miss holds the first turn for one evaluation, a median of 0.34 ms for an extension of one tool, 0.50 ms of 16 and 0.98 ms of 64 (95th: 0.41, 0.58, 1.04 ms); misses are registered one after another.
  With no extension nothing of one is read: the session's finding costs one look at the user's folder, and the trust question's look, for a client that answers it, one at the project's, two with a `.cineris` and no `extensions` in it.
  With extensions, none is read on the path that opens the session but a project's, which the trust question reads once for a client that answers it, and which the session then takes on rather than reading it again.

## Self-written extensions

"Write me a tool that…" is an ordinary request to the agent, or `/extend <request>`.

1. **The agent writes a proposal** with the `propose_extension` tool: a name, a manifest, and the files. It can include test cases (arguments and expected results).
2. **The harness checks it** before the user sees it:
   - the manifest is valid, and its capabilities are flagged if broad (any program, any host, write access to everything);
   - every file parses;
   - the script is loaded in a throwaway runtime with host functions stubbed, to list what it registers;
   - test cases run in the same throwaway runtime. Host calls made during tests are recorded and shown, not performed.
3. **The user reviews** in a dialog: what it adds, what it may ask the host to do in plain words, the test results, and the full source in the viewer. Choices: approve for me, approve for this project, edit, or reject with a note that goes back to the agent.
4. **On approval** the files are written, the hash is pinned, and the extension is live: its tools through `tool_search`, its commands in the menu, its hooks from the next event. Nothing is restarted.
5. **Later changes** by the agent are new proposals, shown as a diff. Changes the user makes by hand are picked up by hot-reload and ask for the same review.

The agent writes extensions with no more power than the user grants at review: the manifest shown in the dialog is exactly what the runtime enforces.

## Hot-reload

Covered with configuration in [08-configuration.md](08-configuration.md). For extensions: a changed file is noticed at the next check, the extension's hash is compared with the approved one, and either the runtime is recreated (hash unchanged or change approved) or the extension is disabled pending review.

**As built in step 28** (`cineris-core`'s `ext::watched` and `ext::found`; [steps/28/parts/pr2a1-found.md](../steps/28/parts/pr2a1-found.md)).

- **What is stamped:** what the extensions were found in.
  The stores of the pins (`pins.toml`, `trust.toml`), the user's folder where it really is, the names on the way to the project's (none past a link or a name that is not there), each extension's folder, the folders in it and its files, and each extension folder left out.
  No stamp follows a link: a place is looked at itself, so a link a repository puts where a folder was is a change, and is never looked through (on Windows a look through a link to a share would send the account's credentials).
- **When it is looked at:** at each turn's start, before the look at the configuration's files.
  The places are looked at in order, a folder before what is in it, stopping at the first change; a place modified within two seconds of its stamp counts as changed until that has passed.
- **What a change does:** every folder is found and judged again.
  An extension that no longer stands approved as it was loaded (changed, unpinned, its project's trust taken away, its folder behind a link out) is withdrawn, its tools out of the registry, and the session is told.
  One that now stands approved (pinned in another process, or changed back) is loaded before that turn starts, registered first if the cache misses it.
- **What it costs:** a look at each place.
  Three with no extension (the two stores and the user's folder), more with the project's way and each extension's files.
  A configuration directory given as a link is watched where it leads: a link pointed elsewhere is seen by the next session.

## Safety

| Measure | Detail |
|---|---|
| No ambient access | A script can do nothing except call host functions. This holds on every OS. |
| Declared capabilities | The manifest bounds what the extension may ask for. The runtime checks every host call against it. |
| Policy still applies | After the manifest check, each host call goes through the permission engine like a built-in tool's effects. An extension's tool asks by default. |
| The sandbox still applies | Programs run on an extension's behalf go through a sandbox launcher of the extension's own, with the manifest's write roots and no network unless `exec_network`, at every level, where the sandbox enforces. Where nothing confines programs (Windows in the first release), `write` and `exec_network = false` do not bind them, and the approval says the program runs outside the sandbox. |
| Review before load | Nothing the agent or a repository supplies runs before the user has seen it. |
| Trust pinned by hash | Any change disables the extension until approved again. |
| Limits | Memory, time and output size, per call and per runtime. |
| Bounded, labelled output | Tool results pass the standard output policy and are labelled as coming from the extension. |

The threats are in [../security.md](../security.md), section 3.

## Windows

- Extension scripts behave identically: the engine is in the binary and the host functions are the same code.
- `cineris.exec` names a program and its arguments as a list. Nothing passes through a shell. Script shims (`.cmd`, `.bat`) are resolved by the host and run with arguments it escapes itself; arguments that cannot be escaped safely are refused.
- **A `.exe` is given every argument in double quotes,** escaped by the C runtime's rules (a `"` as `\"`, the backslashes before a quote and before the closing quote doubled), and not by the standard library's quoting, which quotes only an argument that is empty or has a space or a tab. A program of the Cygwin family (MSYS2's, and Git for Windows' `usr\bin`) expands `* ? [` and braces in an argument that is not quoted, against the folder it runs in, and takes `'` as a quote: an argument `.en*` the permission engine judged as a file name read `.env`. Inside double quotes it does neither. **What such a program does not read exactly** is a run of two or more backslashes that is not before a quote: inside quotes it takes `\\` as one, where the C runtime takes it as two, so `\\server\share` reaches it as `\server\share` and `a\\b` as `a\b` (seen on a Windows runner with Git for Windows' `echo.exe`; a single backslash, one at the end, and `\"` reach it as written). No quoting is read exactly by both, and the C runtime's is the one nearly every program reads.
- **A batch script is never given `& | < > ^ ( )`,** as it is never given `"`, `%`, `!`, a carriage return, a line feed or a NUL: an argument that holds one is refused before anything is asked, naming the argument and the character. Inside the double quotes `cmd.exe` puts round each argument they are plain, but a script that takes the quotes off and reads the argument again (`set A=%~1`, then `echo %A%`; `call :x %~1`; `if "%~1"==… (…)`) runs what follows `&` or `|`, redirects at `<` or `>`, groups at a parenthesis, and joins a line that ends in `^` to its next. The shims package managers write are such scripts' callers, so an npm shim's argument such as `--x=(a)` is refused too: an accepted cost.
- **A shim whose path holds `(` or `)` is refused,** as an argument with one is: the script's path is one of the quoted items of `cmd.exe`'s line, and a parenthesis cannot be carried through `cmd.exe` safely. So a shim under `C:\Program Files (x86)\` does not run; one installed elsewhere, or the program it starts named by its own path, does.
- **On every system, no argument holds `＂` (U+FF02), `ʺ` (U+02BA), `＼` (U+FF3C), `∖` (U+2216), `¥` (U+00A5), `₩` (U+20A9), a soft hyphen (U+00AD) or `－` (U+FF0D):** the call is refused, naming the argument and the character. A program that reads its command line in a Windows code page (an "ANSI" program) is given it converted with a "best fit", which makes the first two `"`, the next two `\`, the yen sign `\` in code page 932 and the won sign `\` in code page 949, and the last two `-`: a quote that ends the quoting, an escape before one, or an option where the engine saw a word. These are the ones known; checking every argument as Windows converts it is open ([an-exec-argument-is-not-checked-against-the-code-pages-best-fit](../outstanding/an-exec-argument-is-not-checked-against-the-code-pages-best-fit.md)).
- Hook commands follow the same rules.

## What is in the first usable release

| | First usable release | Later |
|---|---|---|
| Custom commands | yes | |
| Skills | yes, read from the three folder conventions | |
| Hooks | yes: the seven events, as commands or script functions | More events as needs are shown. |
| Script extensions | yes: tools, commands, hooks, the host functions above | UI contributions (status items, panels). |
| Self-written extensions | yes: proposal, checks, review, immediate load | Sharing and installing extensions from a git URL at a pinned commit. |
| Hot-reload | yes | |
| Management | `cineris ext list`, `inspect`, `disable`, `remove`, `add <path>` | |
| Examples | Three small extensions under `examples/`, used as tests | |

## Relation to MCP

| If you want to… | Use |
|---|---|
| Add a prompt you type often | A custom command. |
| Teach the agent a procedure | A skill. |
| Gate or observe what the agent does | A hook. |
| Add a tool that glues together things already on the machine | A script extension. The agent can write it for you. |
| Add a tool that needs a client library, a database driver or a long-running process | An MCP server ([13-mcp.md](13-mcp.md)). |

Script extensions and MCP servers share the dynamic-tool registry, the trust store, the permission check and the output policy.
