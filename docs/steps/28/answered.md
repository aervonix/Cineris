# Answered in step 28, on 2026-11-20

Step 28 builds `cineris-ext`, the extension system of the first usable release: custom commands, skills, hooks as commands and as script functions, the QuickJS runtime and its host functions, manifests and capability checks, per-extension limits, the registration cache, tools through the dynamic-tool registry, trust pinning, `cineris ext list|inspect|disable|remove|add <path>`, and three example extensions used as tests. The owner's answers come through the maintainers.

| Question | The answer | Recorded in |
|---|---|---|
| **The engine check:** does QuickJS (`rquickjs`) build and pass its tests with the Windows toolchain, or does the step switch to Lua? | **QuickJS stays** (2026-11-20): it builds with MSVC on the hosted runner and passes there and at a desk, and its size matches the size step's measurement. | "The engine check", below |
| **The paths of the build machine in QuickJS's C code** (its asserts' `__FILE__`). | **The shipped build gives the C compiler `-ffile-prefix-map` for the folders it already remaps, and QuickJS keeps its asserts** (the maintainers, 2026-11-20): an abort on a broken engine invariant is the safer failure. Windows desk builds are left to step 34. | "The engine check", below |

## The engine check

`rquickjs` 0.14.0, which vendors QuickJS (the `quickjs-ng` line, 0.16.2), with default features off and `std` only, behind `cineris-ext`'s feature `scripts`, which `cineris-core`'s `scripts` (on by default) turns on. The context has Date, Eval, RegExp, JSON, Map and Set, typed arrays, Promise and WeakRef; not Performance (a high-resolution clock) or Proxy. The binding compiles none of QuickJS's own `std` and `os` modules.

| Check | Result |
|---|---|
| Tests at a desk (Linux) | 6 of 6: a script runs; nothing ambient is on the global object (`require`, `process`, `std`, `os`, `print`, `console`, `setTimeout`, `fetch`, `XMLHttpRequest`, `performance`, `Proxy` and others); a dynamic `import()` loads nothing; an endless loop is stopped by the interrupt check, a memory bomb by the memory limit, endless recursion by the stack limit, and the runtime is usable after it. Lints clean for Linux and, cross-checked, Windows. |
| Tests on the Windows runner (`try`, `test -p cineris-ext`) | 6 of 6. `rquickjs-sys` compiled with MSVC in about 5.4 s of the job's `ci` build. |
| Size, linked into the shipped binary (`try`, `size --check --against-main`, on a commit that is never merged, with the engine called from a hidden argument) | Linux musl 15,342,296 bytes, **+815,408** over `main`; Windows 18,279,424 bytes, **+775,168**. The size step measured +823,600 and +762,880 ([measurements/binary-size.md](../../measurements/binary-size.md)): within 1.0% and 1.6%, so [decision 0005](../../decisions/0005-extension-mechanism.md)'s figures hold. |
| Paths of the build machine | The Linux binary held its target folder four times: QuickJS's asserts name their source file, which the binding copies into its build folder. Fixed in the first pull request (above). |
| Clean build at a desk, 4 cores, with and without the engine | To be measured on an otherwise idle machine; [dependencies.md](../../dependencies.md) gets the figure. |

## The design, before building

Settled with the maintainers before any product code, so that each pull request is reviewed against rules written first.

### 1. Who controls what

| Who | Controls | Can make the harness do | Held by |
|---|---|---|---|
| **The user** | Their configuration (`[hooks.<name>]`, `[extensions.<name>]`), `<config dir>/extensions/`, `<config dir>/commands/`, the skill folders in their home; every approval; `cineris ext add` | What they approve | Their own configuration is taken as written, as their rules are: no tool can write it. Script extensions are pinned all the same (point 5). |
| **A repository's files** | `.cineris/config.toml`'s hooks, `.cineris/extensions/`, `.cineris/commands/`, `.cineris/skills/`, `.agents/skills/`, `.claude/skills/` | Commands and skills: text for the model, as `AGENTS.md` is, with no trust. Hooks and extensions: **nothing** until the project is trusted and each definition is approved as it is now. | Project trust and a pin per definition. `.cineris/` is a protected path, so the model's writes there are asked about. |
| **An extension's author** | Its script and manifest | Register tools, commands, hooks and a prompt section; call host functions | The manifest the user approved, then the permission engine, then the sandbox for programs; the runtime's limits. |
| **The model, through tool calls** | The arguments of extension tools; which calls a hook sees; files it may write | Call an extension's tool (asked by default); feed a hook's placeholders (resolved paths only, point 3); write a skill or a command file (text) | The permission engine. It cannot approve anything, cannot write the configuration or state directories (closed to every tool), and cannot add an extension in this step. |
| **What a hook or an extension returns** | Text | Reach the model as a labelled notice, a note on a result or a tool's result | Caps (8 KiB a hook's context; the output policy for results), a label naming the source. |

**The invariants the reviews test against.**

1. **Nothing runs unapproved.** The engine evaluates only bytes whose hash is the pinned one (point 5), and a hook command starts only from a definition that is approved as it is now (a project's) or that is in the user's own configuration.
2. **No ambient access.** A script reaches files, programs, the network, the user and storage only through the host functions below. Every host call is checked against the manifest first, refused there with the capability named, then decided by the permission engine as the same effect a built-in tool would declare, and a program it starts is confined by the sandbox where the sandbox enforces. No host function returns a credential or the session's content, changes configuration or grants a permission.
3. **Extensions cost start-up nothing.** The engine is not initialised until something of an extension's is used; what extensions register is read from the registration cache.
4. **A limit stops one runtime, never the session.**
5. **Windows behaves the same,** with script shims escaped or refused (point 3).

**A residual risk, stated:** QuickJS is C. A memory-safety bug in it could let a script out of the runtime; a script that breaks one of the engine's own invariants trips an assert, which **ends the whole process**: the assert's line (expression, file, line) on standard error, the process gone with the signal of an abort. On Linux the terminal is put back by the handler step 23 installed for `SIGABRT`; on Windows the console may be left in the harness's modes. The session log is flushed at item boundaries, so the session resumes with `--resume`, without the call that was running.

### 2. The host functions

The whole surface. Registration happens only while an extension's main module is first evaluated; a call to register later is refused.

| Function | Manifest | The permission engine decides it as | Limits |
|---|---|---|---|
| `cineris.tool(def)`, `cineris.command(def)`, `cineris.on(event, filter, fn)` | `api` | (registration) | Names by the registry's rules (ASCII letters, digits, `_`, `-`, `.`); 64 tools, 32 commands, 32 hooks; a description 2 KiB, a schema 16 KiB; the seven events only |
| `cineris.exec(program, args, {cwd, stdin, timeout})` | `exec`: the program exactly as listed | `exec`: the arguments quoted in the session shell's syntax, so the command analysis reads the same words; `cwd` resolved as the shell tool resolves it, inside the workspace; and `network` (any host) beside it when `exec_network = true` | 30 s by default, never past the call's own time; 256 arguments of 32 KiB; input 1 MiB; standard output and error 1 MiB each, cut with a flag |
| `cineris.fs.read(path, {offset, limit})` | `read` globs | `read`, through the `read` tool's own code (the read gate, files that look like secrets) | 1 MiB of text a call; a binary file refused |
| `cineris.fs.write(path, text)` | `write` globs | `write`, through the `write` tool's code: protected paths, the round's checkpoint (so undo covers it) | 1 MiB; refused inside a hook (no round to record it in) |
| `cineris.fs.list(path, {glob})`, `cineris.fs.search(pattern, {path, glob})` | `read` globs | `read` and `read_tree`, through the `list` and `search` tools | Those tools' own caps |
| `cineris.http.fetch(url, {method, headers, body})` | `net` host patterns | `network`, the host | `https`, or `http` to a loopback address; no redirect followed (a 3xx is returned, and the next fetch is checked again); request body 1 MiB, response 4 MiB, 30 s; no cookies, and no header the host adds |
| `cineris.ask(question, choices)` | none | (a question to the user, `question/ask` with the extension's name) | In tools and commands only, never in a hook; one open at a time for an extension, five a call; `null` where nobody can be asked |
| `cineris.notify(text)`, `cineris.progress(text)` | none | (to the front ends, not the model) | 1 KiB and 10 a call; progress 200 characters, the latest kept |
| `cineris.state.get(key, {scope})`, `cineris.state.set(key, value, {scope})` | none | (none) | JSON values. `session` (the default) is kept in the log's `ext` entries, so a resume or a fork has it as it was there; `project` in the state directory. 64 KiB an extension a scope; a key 256 bytes |
| `cineris.log(...)`, and `console.log`, `warn`, `error` as its names | none | (the harness's own log) | 4 KiB a line, 100 lines a call |

- **Paths are checked as the built-in tools check them.** A path given to `fs.*` is resolved by the session's path resolver first (`~`, a relative path, `..`, a link followed to where it leads, case on Windows), and the manifest's globs are matched against the **resolved** path relative to the workspace. A path that resolves outside the workspace is outside every manifest glob in this step. Tests give every check non-canonical input: `~`, relative, `..`, a link inside the workspace leading out, a link outside leading in.
- **Inside a hook,** a host call the engine would ask about is refused, with the rule that would allow it named: a hook decides by its answer, not by putting questions.
- **Where the calls run.** A host call is made on the core's side, inside the future of the tool call or hook that the script serves, with that call's context: a write lands in that round's checkpoint, and a question names the tool that asked.

### 3. The processes: `cineris.exec` and hook commands

Both start through the shell tool's process runner (`cineris-tools`' `process::Runner`), so the process tree, the staged kill, the Job Object's limits and the guard are the shell's.

| | `cineris.exec` | A hook command |
|---|---|---|
| **The program** | A name in `exec`: a bare name found in `PATH`'s absolute directories outside the workspace and the write roots (`cineris_base::program::find_outside`); an absolute path as named, refused inside a write root while commands are confined. A relative path is refused. | `command[0]`, by the same rules. |
| **Its arguments** | As the script gives them, each one argument. | Placeholders are whole arguments, never part of one: `{path}` (each path of the call's plan, **resolved**, one argument each, so a model's `--flag` cannot become an option), `{tool}`, `{workspace}`, `{event}`. `--file={path}` is refused when the entry is read. |
| **Its environment** | What a shell command is given (`env::build`): no provider key, no `shell.env_remove` variable, an absolute `PATH`, `NoDefaultCurrentDirectoryInExePath` on Windows; the session's private `TMPDIR` when confined. | The same. The event's input is on standard input. |
| **The sandbox, where it enforces** | A launcher of its own (`cineris_sandbox::launch`): no write root unless the manifest's `write` is not empty (then the session's write roots), network only with `exec_network = true`. At every level, `unrestricted` included. | A launcher of its own: reads, no write root, no network, unless the entry says `write = true` or `network = true` (for a project's hook, part of what is pinned). |
| **Where it does not** (Windows, no Landlock, sandbox off) | Started as it is, as the shell's commands are; the permission engine has decided it. | The same. |
| **What reaches the model** | Nothing directly: the script gets the exit code and the bounded output. | The hook's answer (point 6). |

**Windows.** The program is looked for as `name.exe`, then `name.cmd`, then `name.bat`; nothing else of `PATHEXT` (`.ps1`, `.vbs`, `.js`) is ever run. A `.exe` is started directly, and the standard quoting carries any argument safely. A `.cmd` or `.bat` is started through `%SystemRoot%\System32\cmd.exe /d /e:off /v:off /s /c` with a command line the harness writes itself: the script's path and each argument in double quotes, where `^ & | < > ( )` are plain characters. **An argument, or the path, holding `"`, `%`, `!`, a carriage return, a line feed or a NUL is refused,** naming the argument and the character, because `cmd.exe` would read it. Tested on a Windows runner, every refused character and a shim run with arguments that need quoting.

**The socket refuses them** (step 27): they are the server's own commands, by their line of parents on Linux and by the job the server joins on Windows, and a test of each starts one that tries.

**Two rows in [security.md](../../security.md), section 12,** one for each, naming `crates/cineris-tools/src/process/mod.rs` called from the core's modules for them, as the row for the shell's version does.

### 4. The manifest and the registration cache

Both are files users keep, and both carry a version from the first one.

```toml
# <config dir>/extensions/tracker/extension.toml, or .cineris/extensions/tracker/
api = 1                     # the version of this file's format and of the host functions
name = "tracker"            # the registry's name rules; the folder's name
version = "0.1.0"           # the author's, shown
description = "Issue tracker tools"
main = "main.js"            # a module in the folder
prompt = "prompt.md"        # optional: section 7 of the system prompt, 4 KiB

[capabilities]              # each empty unless given
exec = ["gh"]               # programs: bare names or absolute paths
exec_network = false        # whether those programs may reach the network
read = ["**"]               # workspace path globs (the permission rules' glob)
write = []
net = ["api.github.com"]    # hosts for cineris.http.fetch (`*.example.com` allowed, `*` alone is any host)

[limits]                    # optional, never above the hard caps
memory = 32                 # MiB, at most 64
time = 120                  # seconds a tool call, at most 600
```

- **Read strictly:** an unknown key, a wrong type, or an `api` this build does not know refuses the extension with the reason. The capabilities are values the harness enforces, so they fail closed ([decision 0028](../../decisions/0028-enforced-values-fail-closed.md)).
- **The folder** holds only regular files and directories: a link of any kind (a symbolic link, a junction) anywhere in it refuses the extension. At most 256 files, 1 MiB each and 4 MiB in all. Imports resolve only to files in it.
- **The configuration it adds:** `[hooks.<name>]` with `event`, `tools` (tool events only: names, or `source/*`), `command`, `observe`, `timeout`, `write`, `network` and `enabled`, a project's waiting for trust as the reserved section did; and `[extensions.<name>]` with `enabled` alone, which `cineris ext disable` writes and a project's file may only turn off. `[skills]` stays reserved.
- **The registration cache** is `<state dir>/extensions/registered.json`, in the state directory that every tool is closed to: `{"version": 1, "entries": {"<content hash>": {"name", "api", "tools": […], "commands": […], "hooks": […], "prompt": "…"}}}`. It is keyed by the extension's content hash, written when an extension is approved (the script is evaluated once in a throwaway runtime whose host functions are stubbed and record what it registers), and pruned of hashes no pin names. A session reads it and knows every approved extension's tools, commands, hooks and prompt text without starting the engine. A missing, unreadable or newer file is set aside, and an approved extension missing from it is registered again in the background, after the session has opened.

### 5. Trust pinning

- **What is hashed.** An extension: every file of its folder, sorted by its path with `/`, each as its path, its length and its bytes (SHA-256, `cineris_base`'s content hash), the manifest among them. A project's hook: its whole entry in a canonical form (event, filter, command and arguments, `observe`, `timeout`, `write`, `network`).
- **Where the pin lives.**
  - A project's hooks and extensions: in the project's entry of `trust.toml` (step 12's store, kinds `hook` and `extension`), approved through the trust question, which lists them, or `cineris permissions trust`.
  - The user's own extensions: in a new store beside it, `<state dir>/pins.toml` (`version = 1`, one `[[pin]]` of kind, name, hash and time each), owned by `cineris-policy`'s `trust` module, written owner-only and atomically under a lock and read strictly, as `trust.toml` is. A new file, rather than a version 3 of `trust.toml`, so that an older build does not refuse every project's trust. Step 30 keeps the user's MCP servers' per-tool pins there.
  - The user's own hooks: not pinned. They are the user's configuration, which no tool can write, as the user's rules are.
- **What a change does.** The extension is disabled: no runtime, its tools leave the registry, its hooks do not run, its commands are not listed, and the session says so with the command that approves it. The engine **never** evaluates a changed file: a runtime is made from a snapshot of the folder read whole, whose hash is compared with the pin, and modules are served from that snapshot, never from the disk. A file changed after the snapshot was taken is not seen until the next runtime, which checks again. So a file's time of change, which a command can set, decides nothing; it only tells hot-reload when to look.
- **Approving** is `cineris ext add <path>` for the user's own (it shows the manifest in words and every file, asks, copies the folder into `<config dir>/extensions/<name>/`, pins it and writes its registration), and the trust question for a project's. A project's extension of the same name as the user's is used in its place only once it is approved, and `ext list` says which one is used.

### 6. Hooks in the loop

The loop gets **one seam, `cineris_agent::Hooks`**, a trait the core implements with `cineris-ext`, beside `Policy`. With no hook whose filter matches, every point is passed at once: the loop's behaviour, and every existing test of it, are unchanged.

| Event | Where | A hook may | Waits for it |
|---|---|---|---|
| `session.start` | The core, when a session opens (new, open, fork), after trust is settled | Add context: a notice before the first prompt | Its hooks, up to their time, before the first turn starts |
| `prompt.submit` | The loop, as a prompt is taken to start a turn, or as a steering message is delivered | `deny`: the prompt is not recorded and never reaches the model, the turn ends `denied` with the hook's reason, and the front end is given the text back. Add context: a notice after the prompt | Yes |
| `tool.before` | The round, after `Policy::decide` said allow or ask, before a question is put or the call starts | `deny`, `ask`, `allow`, or replacement arguments | Yes; the call counts as one whose plan is not known yet, so nothing that could conflict with it starts |
| `tool.after` | The round, after the call ran, before its result is recorded | Add a note to the result, labelled with the hook's name | Yes |
| `turn.end` | The loop, as a turn ends, whatever its outcome | Add a notice for the next turn | No: delivered at the next turn's start if it is in by then, else at the next safe boundary |
| `compaction.before` | The loop, before a summary is requested (by hand, at the threshold, in the background) | Add instructions for the summary | Yes |
| `session.end` | The core, as a session closes | Observe | No; given up to 5 s while the process ends |

- **`tool.before`, decided.** Hooks on one event run together, and their answers combine as `deny` over `ask` over `allow`, so order never matters. **A hook only narrows:** `ask` turns the policy's allow into a question (the question shows each hook's reason), and `allow` never lifts the policy's question. A call the policy denied or stopped is not shown to hooks.
- **Replacement arguments,** from exactly one hook: checked against the tool's schema, planned by the tool, and decided by the policy again, whose answer then stands (a denial denies; a question is put). The hooks are not run on the replacement. The call's item is updated for the front ends, and its result tells the model which hook changed the arguments to what. **Two hooks that both replace the arguments deny the call** (the page's "apply only if exactly one" stays true, and the call does not run with arguments neither hook accepted).
- **A call decided again** (the re-decide step) is shown to its hooks again with its new plan and the model's own arguments; what they said before is dropped.
- **Failure.** A hook that fails, times out, or answers with something that is not the contract (an unknown field or decision: the answer is enforced, so it is read strictly) counts as `deny` on `prompt.submit` and `tool.before`, with the failure named; on the other events it is shown to the user and adds nothing. An `observe = true` hook is never waited for, its answer is not read, and its failure is logged.
- **Times:** 5 s by default for a hook that is waited for, 60 s for an observer, at most 60 s and 10 minutes. A script hook's time is the engine's interrupt and the host's deadline, both on the session's clock, so the tests use the fake clock.
- **A hook's writes** (a `write = true` command such as a formatter) are not recorded in the round's checkpoint, and the model's view of a file a hook rewrote is stale until it reads it again; stated in the page.
- **The outstanding items that wait for this step** (`a-tool-that-makes-links-beside-other-calls`, `a-kept-run-adopted-after-its-call-moved`) are closed by one rule: **an extension's tool is always scheduled as exclusive,** whatever its definition says (`readOnly` is shown to the user and decides nothing). So no later call starts beside it, it counts as a call that may change the file system, and it is never started before its response ends.

### 7. Sections 6 and 7 of the system prompt

- **Section 6, the index:** the skills (name, description cut to 300 bytes on one line, control characters taken out, and the path to `SKILL.md`, relative to the workspace or under `~`), at most 100, the rest counted; then each source of dynamic tools (`name/`, its tool count and its description's first line), with one line on `tool_search`. 8 KiB in all. Custom commands are not listed: they are for the user to type, and the model cannot run one.
- **Section 7:** the `prompt` file of each enabled extension, ordered by name, each under a line that names the extension and says the text is its author's, 4 KiB each and 16 KiB in all, taken from the registration cache (so it is the approved text).
- **The epoch rule.** Both are built when an epoch begins and are fixed for it. They are outside the epoch's `base` hash, as sections 4 and 5 are, so a skill added between two processes does not make a resume a new epoch. A change during an epoch (an extension approved, disabled or changed; a skill added) is told at the next turn's start in one notice of a new source, `extensions`, that gives both sections whole, as the profile's notice does. The declared tool list never changes: extensions' tools are reached through `tool_search` and `tool_call`. Declaring an extension's tool directly (`expose = "direct"`) is left to step 30, which needs the same for MCP; the manifest has no such key until then.
- **Reading a skill.** The folders in section 6 are given to the session as an allowed read (a session rule, source `session`, for each skill folder found), so that reading a skill in the user's home is not asked about; a `deny` still wins, and nothing else outside the workspace is opened by it. `compat.skills = false` leaves `.agents/skills/` and `.claude/skills/` out.

### 8. The runtime

- **One runtime an extension, on a thread of its own,** made the first time something of the extension's is used and kept while the session is open. The page said one thread for all; a thread each means a script that computes until its time is up never holds up another extension's hook. The thread runs an event loop: it starts a call, and while that call awaits a host function the thread can start the extension's next call (a hook while its tool runs). Host functions are promises answered from the core.
- **Limits.** Memory 32 MiB by default, 64 at most; the stack 256 KiB; a tool call's time 2 minutes by default, a hook's its own. The time is enforced inside the engine by its interrupt check against the call's deadline, and outside it by the host, which stops waiting. A tool's result is at most 1 MiB of JSON before the output policy, which then applies as to any tool, and the result is labelled as the extension's.
- **A limit hit, or the engine's out-of-memory,** tears the runtime down: every call of that extension still running fails with the limit named, and the next use makes a new runtime from a fresh snapshot. An exception is an error result for that call only.
- **Start-up.** Opening a session reads the registration cache and the pins, and stats nothing of an extension's folder. A process-wide count of runtimes made (`cineris_ext::engine::runtimes_made`) lets the start-up test assert that none was.

### 9. What becomes a decision record, and what changes in the pages

One decision record, numbered at merge (`cargo xtask decision new`): the manifest's format, the registration cache, the user's pin store, an extension's tools always exclusive, hooks only narrowing, and a thread each runtime. [architecture/12-extensions.md](../../architecture/12-extensions.md) (`exec_network`, `[limits]`, `prompt`, the thread, the `state` scopes, `console`, two replacements deny), [architecture/03-agent-loop.md](../../architecture/03-agent-loop.md) (the hook points, an extension's tool exclusive), [architecture/06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md) (what runs in the sandbox), [architecture/07-sessions-and-context.md](../../architecture/07-sessions-and-context.md) (sections 6 and 7) and [security.md](../../security.md) (sections 3 and 12) are brought to what is built in the pull request that builds it.

**Left out of this step, each said in the page:** a custom command's settings profile for its turn (the page's "optionally"; a command's front matter has `description` and `arguments` only, and an unknown key is shown as not used); declaring an extension's tool directly (step 30, above); and UI contributions (later, as the page says).

### 10. The pull requests

Each about 4,000 lines, more where the rest is tests, each green on both systems with its own record in [parts/](parts/).

1. **The engine and the host functions.** `cineris-ext`: the folder snapshot and its hash, the manifest, the capability checks, the runtime on its thread with its limits, the bindings, and a `Host` trait; `cineris-core`: the host (each function through the permission engine, `exec` through the runner and a launcher of its own, `fetch`) and an extension's tool as a `Tool`, so a test can load an extension and call it end to end. The shipped build's C paths. Early reviews: the capability and permission path, and the process starts.
2. **Extensions in a session.** Finding extensions, the registration cache, the pins (the project's in `trust.toml`, the user's in `pins.toml`) and the trust question's entries, the registry's sources, hot-reload; hooks, as commands and as script functions, at the seven points; skills; sections 6 and 7. Early reviews: the prompt path, and hooks in the loop.
3. **Commands, management and the examples.** Custom commands, from files and from scripts, with `commands/list` (beside the lane, it only reads) and a prompt that names a command (in the lane), and the menu's entries; `extensions/status` filled; `cineris ext list|inspect|disable|remove|add <path>`; three examples under `examples/`, run as tests; the conformance scenario; the socket's refusal of hooks and extension processes.
