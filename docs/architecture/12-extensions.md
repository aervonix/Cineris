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

**As built in step 28** (`cineris_ext::skills`; [steps/28/answered.md](../steps/28/answered.md), point 7). `skills::find` looks in `.cineris/skills/`, `.agents/skills/` and `.claude/skills/` in the workspace, then the same three in the home directory; with `compat.skills` off, only the two `.cineris/skills/`. It reads the folders and the `SKILL.md` files and nothing else, and blocks, so the caller runs it on the blocking pool.

- **A skill is `<root>/<folder>/SKILL.md`, one level down.** A `SKILL.md` in the root itself or deeper than one folder is not one, and a folder with none is passed over without a word.
- **Paths are judged where they lead.** Every root, folder and file is found through the session's path resolver (`~`, a relative path, `..`, links, the case Windows stores), and two places are compared by place (`cineris_base::location_within` and its friends), never by their text. A root may be a link the user made (`~/.claude/skills` often is), and is followed; **a project's root must lead to a place inside the workspace.** A skill's folder or its `SKILL.md` reached through a link is followed only where it stays below its root. The file is opened with `open_regular`, which refuses a link, a FIFO or a device at its last name. A root that is the same folder as one looked at before it is looked at once.
- **The front matter** is a block between two `---` lines at the top of the file, read by a small reader of the plain YAML skills are written in (`FrontMatter`), with no YAML library: `key: value` lines, a value in either quote with its escapes, a value folded over lines, and a `>` or `|` block. A file with no front matter, or without a `name`, is left out with its reason; a missing description is an empty one.
- **For the system prompt** each name and description is one line: white space becomes one space, and the control characters and Unicode's format characters (the bidirectional controls, the zero-width characters, U+FEFF) are taken out. A name is cut to 64 bytes and a description to 300, between two characters, ending in `…`. The path shown is relative to the workspace for a project's skill and under `~` for the user's, never the home directory written out, with every character that acts unseen written as an escape.
- **Caps:** at most 1,000 entries looked at in a root (every entry counts, a file too); a `SKILL.md` of at most 64 KiB; at most 100 skills listed, the rest counted.
- **One name, one skill.** Of two skills with one name, the project's is taken over the user's, and `.cineris/` over `.agents/` over `.claude/`, whatever order the roots are given in; the other is left out with its reason. The list is by name.
- **What was left out** (a link out of its root or to nothing, a file that is no file, too large, no front matter, no name, a duplicate, a root past its cap) is returned with its reason, for the user; the model is told only how many skills it is not shown.
- **Section 6's part** (`skills::render`) is a heading line, one line a skill, `- name: description (path)` (`- name (path)` with no description), and a line that counts the skills not listed, within 8 KiB.

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

**Read strictly** (since step 28): a key this build does not know, a value of the wrong type, or an `api` it does not know refuses the extension, with the reason. The capabilities are values the harness enforces ([decision 0028](../decisions/0028-enforced-values-fail-closed.md)). The folder holds only regular files and folders, at most 256 files of 1 MiB each and 4 MiB in all: a link of any kind anywhere in it refuses the extension. What the events a script may hook let it see is shown in words when it is approved ("every prompt you send"), since a hook sees the session's content.

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
| `cineris.exec(program, args, options)` | Asks the host to run a program. Returns its exit code, and its standard output and standard error apart, each bounded. | `capabilities.exec`, the program exactly as listed; then the permission engine as an `exec` effect (and a `network` one with `exec_network`); then the sandbox, with write roots made from `capabilities.write`. |
| `cineris.fs.read`, `cineris.fs.write`, `cineris.fs.list`, `cineris.fs.search` | Workspace file access through the same code as the built-in tools: a write through the `write` tool (its checkpoint, its protected paths), a file overwritten only after the script read it as it is; a listing or a search through the `list` and `search` tools, whose walk is gated by the manifest on every entry. | `capabilities.read` and `write`, against where the path leads; then the permission engine. |
| `cineris.http.fetch(url, options)` | An HTTP request made by the host. | `capabilities.net`, then the permission engine as a `network` effect. |
| `cineris.ask(question, choices)`, `cineris.notify(text)`, `cineris.progress(text)` | Talk to the user through the front end. Not `ask` in a hook. | Caps on length and count. |
| `cineris.state.get`, `cineris.state.set` | A small key-value store per extension, kept in the session and the state directory. | A size cap. |
| `cineris.log(...)`, and `console.log`, `info`, `warn`, `error`, `debug` as its names | Writes to the log. | |

There is no function that returns credentials, reads the session, changes configuration or grants permission. **The manifest is asked first**, in code (`cineris_ext::host::serve`): a call it refuses never reaches the session. Then each call is decided by the permission engine as a call of the extension's tool (`tracker/find_issue`) with the built-in tools' default and an `ext` effect beside its own, so a rule about the extension covers its host calls too, and a question it needs offers only "allow once" and "deny" and keeps no rule (since step 28: a rule kept from it would reach past the extension).

**The runtime.**

| Property | Design |
|---|---|
| Isolation | One runtime per extension. Extensions cannot see each other. |
| Limits | Memory per runtime (default 32 MB, hard cap 64 MB, the manifest's `[limits]` within them). The stack, 256 KiB. Time per call (2 minutes for tools, the hook timeouts for hooks), enforced by the engine's interrupt hook, so a tight loop is stopped, and by the host, which stops waiting. A result is at most 1 MiB of JSON. |
| Language | JavaScript modules. Imports resolve only inside the extension's own folder, and are served from the snapshot of it whose hash was approved, never from the disk. Types for the host functions ship as a declaration file for editors. |
| Threading | **One thread per runtime** (since step 28, in place of one thread for all: a script that computes until its time is up never holds up another extension's hook), made the first time something of the extension's is used. **A runtime runs one call at a time,** to its end, and a call's time starts when it starts running: the interrupt check belongs to the runtime and cannot tell two calls apart. Host functions are asynchronous: a script awaits them, and the core does the work. A thread that does not come back within a grace after its deadline is abandoned, and its runtime counts as torn down. |
| Start-up cost | None. What an extension registers is cached with its hash when it is approved. At start-up the harness reads the cache and knows the tools, commands and hooks without starting the engine. The engine starts the first time something of the extension's is used. |
| Failure | An exception is an error result for that call. A runtime that hits a limit is torn down and recreated on next use. The session is never affected. |

**Tools from extensions** are reached through `tool_search` and `tool_call` ([05-tools.md](05-tools.md)), so adding one never changes the declared tool list. **An extension's tool always runs alone** (since step 28), whatever its definition says: its script may make a link, and a call beside it would have been decided before the link was there. `readOnly` is shown to the user and decides nothing. Declaring an extension's tool directly from the next context epoch (`expose = "direct"`), and promoting frequently used ones, are left to step 30, which needs the same for MCP; the manifest has no such key until then.

**As built in step 28** (`cineris-ext`; the core's `ext` module; [steps/28/answered.md](../steps/28/answered.md)): the folder's snapshot and hash (`folder`), the manifest (`manifest`), the capabilities compiled for a workspace (`capability`), the host functions' contract and the order they are checked in (`host`), and the engine (`engine`, behind `scripts`). The core registers an approved extension's tools from what it registered when approved (`CoreOptions::extensions` until the harness finds extensions itself, in the step's second pull request) and serves their host calls.

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

## Safety

| Measure | Detail |
|---|---|
| No ambient access | A script can do nothing except call host functions. This holds on every OS. |
| Declared capabilities | The manifest bounds what the extension may ask for. The runtime checks every host call against it. |
| Policy still applies | After the manifest check, each host call goes through the permission engine like a built-in tool's effects. An extension's tool asks by default. |
| The sandbox still applies | Programs run on an extension's behalf go through the sandbox launcher. |
| Review before load | Nothing the agent or a repository supplies runs before the user has seen it. |
| Trust pinned by hash | Any change disables the extension until approved again. |
| Limits | Memory, time and output size, per call and per runtime. |
| Bounded, labelled output | Tool results pass the standard output policy and are labelled as coming from the extension. |

The threats are in [../security.md](../security.md), section 3.

## Windows

- Extension scripts behave identically: the engine is in the binary and the host functions are the same code.
- `cineris.exec` names a program and its arguments as a list. Nothing passes through a shell. Script shims (`.cmd`, `.bat`) are resolved by the host and run with arguments it escapes itself; arguments that cannot be escaped safely are refused.
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
