# Configuration

`cineris-config` loads settings from several layers, merges them, records where each value came from, and validates the result. It also owns credential storage.

The [vision](../vision.md) adds four requirements that this page covers: profiles, reloading without a restart, reading what other tools already wrote, and a first run that needs no manual set-up.

**Every key, with its type, default and rules, is in the generated [configuration reference](../configuration/reference.md).** This page is the design; that page is the list.

## Format

TOML. One schema, versioned with a top-level `config_version = 1`. The reasons for TOML over JSON are in [decision 0007](../decisions/0007-configuration-format.md).

A length of time is written as text with a unit: `"20s"`, `"10m"`, `"1h30m"`, `"500ms"` (units `ms`, `s`, `m`, `h`, `d`). A bare number is refused, so nobody has to remember whether a key counts seconds or milliseconds.

## Layers

Lowest priority first. Later layers override earlier ones.

| # | Layer | Location | Notes |
|---|---|---|---|
| 1 | Built-in defaults | compiled in | `cineris config defaults` prints them. |
| 2 | Managed | `/etc/cineris/config.toml`; `%ProgramData%\cineris\config.toml` | Optional. For shared machines. Can lock keys (below). |
| 3 | User | `$XDG_CONFIG_HOME/cineris/config.toml` (usually `~/.config/cineris/`); `%APPDATA%\cineris\config.toml` | The usual place. `CINERIS_CONFIG` names another file. |
| 4 | Project | `<workspace>/.cineris/config.toml` | Committed with the repo. Applies only as far as trust allows (below). |
| 5 | Project, local | `<workspace>/.cineris/config.local.toml` | Not committed. The repo skeleton adds it to the ignore file template. Held to the same trust rules as layer 4. |
| 6 | Profile | a `[profiles.<name>]` table from layers 2 to 5, selected by `--profile <name>` or `profile = "<name>"` | A named set of overrides. See "Profiles". |
| 7 | Environment | `CINERIS_*` variables | A short, fixed list, below. Not a general override mechanism: a variable that is not on the list does nothing. |
| 8 | Command line | dedicated flags, and `-c key=value` for any key | Highest priority. A mistake here is always an error. |

**The environment layer.**

| Variable | Sets |
|---|---|
| `CINERIS_LOG` | `log.level` |
| `CINERIS_DATA_DIR` | `session.data_dir`. An absolute path. It is read here and nowhere else, so a managed lock on the key holds against it. |
| `CINERIS_CONFIG` | Not a key: the path of the user's configuration file (layer 3). |
| A provider's key variable (`ANTHROPIC_API_KEY` and so on) | Not a key: a credential ("Credentials", below). |

Colour is the fifth item of the list. Its key is in the `[terminal]` section, which the terminal step declares with the variable that sets it.

**On the command line** `-c model.main=some-model` needs no quotes: the value is read as TOML where the key takes that (`-c retry.attempts=3`, `-c sandbox.protect=["a"]`) and as plain text otherwise.

The workspace root is the nearest ancestor of the working directory that contains a `.cineris/` folder or a `.git`, **either owned by the user**, or the working directory itself. Anyone can make a folder in `C:\` or in a shared temporary directory, and without the check a `.git` or `.cineris` that another account put there would make every directory below it one workspace, whose project files that account writes (the class of git's CVE-2022-24765). Whose a candidate is:

- **Linux:** owned by the process's effective user. The entry itself is asked, so a link at `.git` counts by its own owner, not its target's. The user is read from `/proc/self/status`, because the standard library has no safe way to ask; where that cannot be read, no candidate counts and the root is the working directory.
- **Windows:** owned by the current user, or by the Administrators group (`S-1-5-32-544`). An elevated process, and on Windows Server any administrator's, creates objects that the group owns, not the user. Accepting the group is safe: only an administrator can create an object it owns, and an administrator can already change the user's own files. A link or a junction at `.git` or `.cineris` is passed over: its owner cannot be read without following it, and following a link can reach a network share.

A worktree's or a submodule's `.git` is a plain file, and counts like a folder. A `.cineris` must be a folder: a link there is passed over on both OSes. What was passed over is reported (`find_workspace_root`), so that the front end can say why a folder the user expected to be the project is not.

**As built since the follow-ups of step 23** (the owner's choice of 2026-11-16; until then every binary took the working directory for the root). Each binary finds the root this way before the core is asked anything: the interactive application, headless mode and every subcommand (`cineris-cli`'s `workspace::Here`). What was passed over is said, as a notice in the application and on standard error elsewhere. The interface's header shows the root. The working directory, when it is below the root, is where the session's commands start: it is given to `session/new` as `cwd` ([11-rpc-protocol.md](11-rpc-protocol.md)) and kept in the session's log ([07-sessions-and-context.md](07-sessions-and-context.md), "What goes into a request"). Started in `repo/src`, then:

- the repository's `.cineris/config.toml` and `.cineris/config.local.toml` are read;
- its instruction files are found from the root down to `src` ([07-sessions-and-context.md](07-sessions-and-context.md), "Project instruction files");
- the trust, never and not-now said at `repo/` apply ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md), "Project trust");
- the session's project key is the root's, and so are its stored approvals, its knowledge, its statistics and the prompt history;
- `cineris permissions …`, `cineris project …`, `cineris sessions list` and `cineris config …` act on the root's project.

**What was kept under a subfolder's key before the change stays where it is,** and is no longer reached from below a root that holds it:

- **A session started in `repo/src` before** is filed under `src`'s key, with `src` as its workspace. `--continue` and `sessions list` from `repo/src` now look under the root's key and do not list it; `--resume <id>` (and `sessions list --all`) still find it, and it opens with the workspace it was made in.
- **Trust, a never, or stored approvals given at `repo/src` before** are filed under `src`'s key, and are no longer consulted from `repo/src`, where the root's apply. They are inert, not lost: `cineris permissions` lists the trusted projects by the root each was trusted at. A folder with no `.cineris` or `.git` above it is its own root, as before, and keeps what was given there.

**A project's files are found without following a planted link.** A repository can put a link where its own configuration file goes, or where another tool's file is looked for. On Windows a link to a network share makes the system send the account's credentials to the server it names as soon as it is followed, by opening the file or merely asking about it ([security.md](../security.md), section 7). So the project's two files, and what discovery looks for in the workspace, are found through `cineris-base`'s path resolver, which follows each link itself and refuses a share before the network is asked. A project file whose way is refused, or that is a link to nothing, is an error, not a layer to skip, as a broken project file is; so is a workspace root the resolver cannot take (one whose name is not Unicode, say), since nothing in it could be checked. A workspace that is itself on a share is where the user chose to work: its own share is allowed, and a link to any other share is refused.

**Nothing is read behind the caller's back.** The loader is given the environment and the workspace as values. It does not read the process's environment or its working directory, so a test loads configuration from a folder of its own and cannot see the real home directory.

## Merge rules

- **Tables merge key by key**, recursively.
- **Scalars and arrays replace.** There is no syntax for appending to or removing from an inherited array. A layer that sets an array sets all of it.
- **Named collections are tables keyed by name**, so layers can add or override one entry without restating the rest: `[providers.<name>]`, `[models.<name>]`, `[mcp.servers.<name>]`, `[extensions.<name>]`, `[hooks.<name>]`. An entry is removed by setting `enabled = false`. `[keys]` is a table for each context of the key map (`editor`, `menu`, `dialog`, …), each one setting: a layer that sets a context's table sets all of it.
- **Three lists are joined, not replaced:** `[[permissions.rules]]`, `sandbox.write` and `sandbox.protect`. Each layer adds its entries to those of the layers below. Their meaning does not depend on order ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md)), and a layer must not be able to drop a restriction by restating the list without it: if `sandbox.protect` were replaced, a project's `protect = []` would unprotect what the user protected.
- **Locks.** The managed layer can list keys under `[managed] locked = [...]`. A locked key ignores every higher layer, the command line included, and `cineris config show` says so. Naming a table locks every key in it. A lock on a key this version does not know is kept and warned about, not dropped. `[managed]` in any other file is ignored, with a warning; inside a profile it is an error (see "Profiles").
  - **A lock cannot be passed by another way in.** `--profile` does not move a lock on `profile`: the managed file's profile stays, or none when it chose none. A value set where it would replace a table that holds a locked key (`servers = "none"` over a table in which `mcp.servers.corp` is locked) counts as locked, since setting it would remove the locked entry. And `CINERIS_DATA_DIR` sets `session.data_dir` in the environment layer only, where a lock holds against it.

## Profiles

A profile is a named table of settings that is applied on top of the file layers when selected.

```toml
[profiles.review]
model.main = "claude-opus-5-5"
model.effort = "high"
permissions.mode = "read-only"

[profiles.quick]
model.main = "claude-haiku-5-5"
verify.mode = "off"
```

- Any key may appear in a profile except the four that describe a file (`config_version`, `profile`, `profiles`, `managed`), with the same trust rules as the layer the profile is defined in. One of the four in a profile is an error that refuses the file, the managed file included.
- A profile defined in several layers merges like any other table, higher layer last.
- `cineris --profile review` selects one for a run. `/profile <name>` switches in a session; settings that affect the prompt or the tool list take effect at the next context epoch, and the UI says so. A profile that does not exist is an error that lists the ones that do. It is judged by a load that reads the project's files: a load with no workspace (the binary's own, for the user's settings) cannot see a profile only the project defines, and leaves the name to each session's load (since the review of step 23). A project's profile is held to the trust rules below, and the trust question lists its settings that wait for trust.
- **A project may define a profile and may not select one.** `profile = "<name>"` is the user's key. A profile the user wrote is applied with the user's rights, so a repository that could select it could switch on whatever the user's loosest profile allows.
- "Modes" are the permission and behaviour presets in [06-permissions-and-sandbox.md](06-permissions-and-sandbox.md). A profile can select a mode along with anything else.

## Reloading without a restart

Configuration, extensions, themes and key bindings reload while the harness runs.

**How changes are noticed.** There is no file watcher and no polling timer, which keeps the idle cost at zero. The harness checks the modification times of the files it loaded:

- when the terminal regains focus (terminals report this);
- on a key press, at most once a second;
- at the start of every turn;
- when the user runs `/reload`.

The check is a handful of `stat` calls, well under a millisecond. It compares each file's modification time and size with what they were when the file was loaded (and on Unix which file it is, so that a replaced file shows even when both are the same). A file that did not exist is watched too: creating it is a change. Two details keep it from missing a change:

- **The stamp is taken before the file is read.** A change that lands while a file is being read and parsed is then seen by the next check.
- **A file written in the last two seconds is not trusted yet.** File systems record times in steps as coarse as two seconds, so a second write can leave the time as it was. Such a file is reported as changed until it has settled. The cost is a few reloads of a file that did not change again, so a reload that produces the same configuration is not announced.
- **The project's files are found again before they are looked at.** Asking the file system about a file follows links, and a link to a network share that a repository adds at `.cineris` after the load would otherwise be followed by the next check. So each check first finds the project's two files again through the path resolver, which looks at each name on the way without following a link it has not checked, and compares where they are with where they were. If any is somewhere else, or is refused now, the check reloads through the resolver instead of looking. That is an `lstat` call for each name on the way to each file: measured in step 9's fixes on the reference machine, release build, a check takes 22 µs with a project nine directories below the root against 0.9 µs without one, within the 1 ms budget ([performance-budgets.md](../performance-budgets.md)). A link swapped in during the moment between that look and the check's own is still followed: a residual risk, listed for the whole-repo review.

**What happens to a change.** Each key says when a change to it takes effect, in the [reference](../configuration/reference.md), and a reload returns the changed keys with that: `at once`, `next epoch`, `next session` (since step 24) or `restart`.

| Changed | Takes effect |
|---|---|
| Theme, key bindings, UI settings | Immediately, on the next frame. |
| Permission rules, limits, retry and verification settings, the speed tier | Immediately, for the next tool call or request. |
| Model, reasoning level, anything that shapes the system prompt or the declared tool list | At the session's next context epoch that builds the prompt again: a compaction, or the session opened again ([07-sessions-and-context.md](07-sessions-and-context.md)). A notice says what is pending. `/reload --now`, which would start a new epoch at once, is not built ([outstanding](../outstanding/reload-now-begins-an-epoch.md)). |
| What a session is made with once, as it opens: its shell (`shell.*`), its sandbox's switch and protected paths (`sandbox.enabled`, `sandbox.protect`), the compatibility readers (`compat.*`), the map's wait and whether there is a map (`index.map_wait`, `index.map`) | In sessions opened from then on (`next session`, since step 24). A notice says so, and says with more weight when the change would tighten the sandbox, since the running session stays as it was. |
| The data directory, the credential store | When the harness is next started. |
| An extension's script or manifest | The extension is reloaded. If its hash no longer matches what was approved, it is disabled and the user is asked to review the change ([12-extensions.md](12-extensions.md)). |
| An MCP server entry | That server is restarted. A changed command needs approval again. |
| An invalid file | The old configuration stays in force and the error is shown. A reload never leaves the harness without a valid configuration. The one exception is lowering trust, below. |

**An invalid file never takes the configuration away.** There is no partly loaded configuration: a load either gives a whole one or lists what is wrong, in every layer, in one go. The same broken file is reported once, not on every check.

**As built in step 24** (`cineris-core`'s `reload`; [steps/24/answered.md](../steps/24/answered.md), "The design, before building", point 1):

- **`config/reload`** reloads every open session of the client and the core's own configuration (the user's, with no project), or one session (`session_id`), or one session with another profile (`profile`, for `/profile`; only from a `user` client, since a profile can raise the level). With `changed_only` it reloads only where a file, or the trust store, changed since it last looked: the look the interface takes on a focus event and on a key press at most once a second. The core takes the same look before each turn's prompt.
- **The configuration in force is a mix** (`Loaded::taking`): the `at once` keys are taken from what was loaded, the `next epoch` keys when the session's next compaction begins, and the `next session` and `restart` keys never, for that session. The core's own configuration is a mix too, so a reload never moves the data directory or turns the statistics on or off under a running core.
- **Trust is judged first,** as at a start (`assemble::judge`), from loads that are never put in force; one load follows with that trust. The trust store's file is looked at with the configuration's, so trust taken away in another terminal is seen by the next look. Lowered trust takes at once everything trust gave: the engine's two answers, the verifier's, whether git may read the repository's own configuration (the shell's look at what a command changed, a base capture still under way), the project's write roots and network in the sandbox's grant, and the project's settings that need trust, whatever their effect.
- **One reload of a session at a time,** from its look to its commit: another client's, the look before a turn, and `session/set` (whose choice is then the last word) wait for it. The core's own configuration is reloaded one at a time too, its files read under a lock of their own, so `config/get` never waits for a disk. A reload commits on top of what the loop took for a new epoch meanwhile, and builds again if the loop took it while it built.
- **What the person chose is in the configuration in force,** and out of what waits for the next epoch: `config/get` says it, and a reload that builds a chosen model again keeps it at the model's own default where the files' reasoning level is not one it has, as `session/set` does.
- **A yes the trust store could not keep** holds for the session to the rules the person saw: its looks judge against those, not the store, which has no record of it.
- **The session's files are stamped as it is put together,** by the load it was made from, so that trust taken away while a file is broken is taken away from the first look on. **A load made trusted is checked against the pins again,** on what it actually loaded: a file written between the judgement and the load is never in force unchecked.
- **Each session's reload is checked whole before anything is applied:** the engine, the roles' models with their credentials, the day's account, and what waits for the next epoch. A failure refuses that session's reload with its error, and its configuration stays as it was; **except that trust taken away is taken away all the same,** with what failed left waiting and said.
- **What applies at once, and when a running turn sees it:** the permission rules and level at the next decision; `sandbox.write` and `sandbox.network` at the next command (the session's launcher is made again); the limits at the next request, what was spent kept; the retry policy, a provider's key variable, headers and timeouts, and the speeds at the role's next request (the role's model is built again from the configuration in force, so nothing that waits comes with it, and no epoch begins); the compaction's thresholds and `verify.rounds` at the next request; the rest of `[verify]` at the next round; `log.level` at once, unless the command line or `CINERIS_LOG` chose it; `usage.*` and `stats.retention` at the next report, refresh or pruning. A variable a new `key_env` names is taken from every command at once.
- **What the person chose in the session outranks the files:** a level, model, reasoning level or speed set with `session/set` is kept as an override of the session's load, so a reload never undoes it, and a role set so no longer waits for the next epoch. `/profile` starts from the profile, dropping those choices, and says which.
- **Who is told:** every client of a session the reload changed, by the event `config.changed` (the keys applied, those that wait, the errors, and a sentence for each kind of wait), whoever's reload or look found the change; the result of `config/reload` holds the same for the client that asked. A broken file is said once, and again when the person asks (`/reload`); a reload that changed nothing is said only to the person who asked.

**Changing what is loaded from** (another profile, the project trusted, the project's trust taken away) reloads at once, with two rules:

- **Lowering trust always takes effect, even when a file is broken**, whichever file it is. Otherwise a trusted repository could commit a broken file and make taking its trust away do nothing. The files in force are merged again as they were read, without reading any file, with the project untrusted, and the error is reported. That is what an untrusted load of those files gives: what needs trust (an `allow` rule, a hook, a dearer model) is held back again, and the project's restrictions (`deny` and `ask` rules, protected paths, lower limits) and preferences stay, since dropping them would loosen what is in force, the opposite of what taking trust away is for. Once the file is fixed, the next check reads everything again.
- **Any other change that does not load is undone.** The options stay as they were, the configuration in force stays, and the change is refused with the error every time it is tried, even when a check has reported that error already.

## Reading other tools' set-up

People switching from another agent should not have to rewrite anything. The harness reads, and never modifies, these:

| What | Where it is looked for | How it is used |
|---|---|---|
| Instruction files | `AGENTS.md` and `CLAUDE.md`, as in [07-sessions-and-context.md](07-sessions-and-context.md) | As project instructions. |
| MCP server definitions | The project's `.mcp.json`, and the per-user files of common MCP clients | Offered at onboarding and by `cineris mcp import`. Project-level definitions appear as proposed servers that need the same trust and approval as any other. See [13-mcp.md](13-mcp.md). |
| Agent Skills folders | `.cineris/skills/`, `.agents/skills/` and `.claude/skills/` in the project and the user's home | Listed in the prompt by name and description, and read on demand. See [12-extensions.md](12-extensions.md). |

Everything else (settings, permissions, hooks, key bindings, themes) uses Cineris's own formats. Each compatibility reader can be turned off under `[compat]`, which is the user's alone: a project cannot turn a reader back on.

**Discovery reports and does not read.** `cineris-config` answers "what is there?" for the onboarding flow and the import commands: it asks the file system whether each candidate exists and whether it is a file or a folder, and opens nothing. Another tool's file can hold that tool's tokens. Reading one is an import, which the user asks for.

| Kind | In the workspace | In the home directory |
|---|---|---|
| Instruction files | `AGENTS.md`, `CLAUDE.md` | |
| MCP server definitions | `.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json` | `.claude.json`, `.cursor/mcp.json`, `.codex/config.toml`, and `Claude/claude_desktop_config.json` in the configuration folder |
| Agent Skills folders | `.agents/skills/`, `.claude/skills/` | `.agents/skills/`, `.claude/skills/` |

The other tools' locations were written down on 2026-11-12 and go stale. The steps that read the files confirm them against each tool's current documentation.

## First run

Running `cineris` with no configuration starts a short guided set-up in the TUI. It can be skipped, and every step has a default.

1. **Provider.** Detect provider keys in the environment and offer them. Otherwise ask for a provider and a key, and store the key in the credential store.
2. **Import.** List what was found from other tools (instruction files, MCP servers, skills) and ask what to bring in.
3. **Mode.** Permission mode, with `supervised` preselected and explained in two lines.
4. **Project.** Ask whether to trust the project, showing what trust enables. Offer to detect the build, test and check commands, and show them for confirmation before they enter the profile.

The mode comes before the project because the configuration file is written before the first session opens, so that the session is made with what was chosen, and the project's trust question is that session's own.

Headless mode never runs this. It reports what is missing and how to provide it.

**As built since step 25** (screens F1a to F4 in [tui-look/](tui-look/README.md); [09-tui.md](09-tui.md), "The first run", for how it is drawn and its keys):

- **What starts it:** an interactive start at a terminal when the user's configuration file does not exist: the file `CINERIS_CONFIG` names, or `config.toml` in the configuration directory (anything at that name, a link included, counts as there). A stored key, a key in the environment or a managed file does not stop it. `cineris attach` never runs it: the server it attaches to has its configuration.
- **The provider:** the providers the core knows, each with where its key comes from now. A key in the environment is offered by its variable's name, never its value, and choosing it stores nothing; a key already stored is offered too. Otherwise the key is typed or pasted without echo and stored in the credential store `cineris auth login` uses; the step says where that is: the owner-only `credentials.toml` on Linux (the keychain is not built, "Credentials" below), Windows Credential Manager, for this machine only, on Windows.
- **The imports:** discovery's list, which opens nothing. **Every row starts unpicked,** and a skip means the same as nothing picked: `compat.instructions` and `compat.skills` are written `false` unless picked, each row saying so and naming its key, which turns it on later. A user's MCP file of another client is read only when the person asks to see inside it (`space`); its servers are listed under it, each command or address in full, wrapped and never cut, every value of its environment and headers dropped to its name, and every argument and address through the secret filter's formats and the address's user-part rule. A part that matches is drawn `[redacted]` and that server cannot be imported; nor can one with a control character or one that acts unseen. A server picked is written as an `[mcp.servers.<name>]` table in [13-mcp.md](13-mcp.md)'s form, its environment and headers as references to variables (a header's from a variable named for the server and the header, `TRACKER_AUTHORIZATION`), never `network` or `expose`. A project's own MCP files are listed as not imported: they are offered live in its sessions, after trust.
- **The mode:** the levels in the ladder's order, `supervised` preselected, each with what it means as [06-permissions-and-sandbox.md](06-permissions-and-sandbox.md)'s table has it; `autonomous` only where the sandbox enforces; **never `unrestricted`**, which 06 has chosen explicitly (`/mode` and this file still reach it). It is written as `permissions.mode`; a skip keeps `supervised` and writes nothing for it.
- **The file is written new** after the mode, through the TOML serialiser (a name from another tool's file stays one key), read strictly as a user's file and compared with what was meant before it is created and again from the disk after; it is never written over anything at its name. It is private to the user. Each step skipped leaves a comment saying how to do it later. Then the core reads the configuration again (`config/reload`) and the session opens.
- **The project:** the session's own trust question (screen 2c), when the project has something to trust; then, once the session is idle (no turn running, no dialog, the input box empty: a prompt given on the command line runs first; [09-tui.md](09-tui.md), "The first run"), the commands its files name (`project/detect`: `Cargo.toml`, `package.json`'s scripts, a `Makefile`'s `build`, `test`, `check` and `lint` targets, `go.mod`, `pyproject.toml`), each shown in full with what a script runs, **every one unpicked.** Only lines of plain words the command observer classes are offered, so a script's or a target's name that is not plain words never reaches the screen as a command. **A script is offered only when what it runs can be read whole, as it is written** (since the full review of the third pull request): one whose text is longer than 1,000 characters, or holds a control character or a character that acts unseen (a tab, a line break, a direction override, a character of no width), is not offered at all, and nothing offered is cut. `e` edits one. Those picked join the profile as entries the person wrote (`project/update`'s `add`), their text saying where they were found (`found in Cargo.toml by the set-up`); nothing is learned, proposed or run. For a project not trusted the step says that a command still asks until it is.
- **Skipping:** `s` skips a step and `x` the whole set-up: what was done stays done (a stored key), the rest take their defaults, and the file is written all the same, so the first run never starts again by itself. `Ctrl+C` twice leaves with nothing written but what a finished step wrote, and the first run starts again next time.
- **There is no `cineris setup`.** The writer makes only a file that does not exist. When the first run ends or is skipped, its last line names what does each step again: `cineris auth login <provider>` for a key, `/mode` for the mode, `/project` for the project's commands, and the configuration file, by its path, for the rest.

## Provenance

Every effective value remembers its layer, its file and its line. In this example the project is trusted, so its choice of model applies.

```
$ cineris config show --origin
model.effort = "high"                       # profile review (user: ~/.config/cineris/config.toml:32)
model.fast = "claude-haiku-5-5"             # user: ~/.config/cineris/config.toml:6
model.main = "claude-sonnet-5-5"            # project: ~/work/app/.cineris/config.toml:2
model.speed_wait = "5s"                     # command-line: -c model.speed_wait
permissions.rules = [
    { action = "deny", effect = "write", match = "/etc/**" },  # managed: /etc/cineris/config.toml:9
    { action = "allow", effect = "exec", match = "cargo test *" },  # user: ~/.config/cineris/config.toml:23
]
retry.backoff_cap = "30s"                   # default
sandbox.enabled = true                      # managed: /etc/cineris/config.toml:6, locked
```

- `cineris config show` prints the effective configuration. `--origin` adds the source of each value. `--layer <name>` prints one layer as it was read, before locks and the trust rules: `default`, `managed`, `user`, `project`, `project-local`, `profile`, `environment` or `command-line`.
- Each entry of a joined list has an origin of its own. That is how the policy engine knows which layer wrote a rule: managed rules cannot be weakened by user rules, nor user rules by project rules.
- `cineris config check` validates all layers and exits non-zero on any problem.

## Validation

- Unknown keys are warnings with a nearest-match suggestion. `--strict-config` (and `cineris config check`) makes them errors.
- **In `[permissions]`, `[sandbox]` and `[limits]` an unknown key or value is always an error, and the whole file is refused** ([decision 0028](../decisions/0028-enforced-values-fail-closed.md)). These are the values the harness enforces, and a configuration file is read by more than one build. A build that skipped a restriction it did not understand would run with a wider configuration than the file asks for. So nothing is skipped: the load fails, and a harness that is running keeps the configuration it had. A permission rule is read by the protocol's own strict reader, and a mode must be a level this build lists.
- Type and range errors name the file, line and key.
- A value that depends on the catalog (a model name, a reasoning level, a speed tier) is checked for shape here. `cineris-config` has no catalog; the crate that has one checks the value against it and the user's overlays.
- A JSON Schema for the configuration file is generated from the Rust types and published with each release, so editors can complete and check the file. It is checked in at [configuration/schema.json](../configuration/schema.json), beside the [reference](../configuration/reference.md) generated from the same declarations, and a test fails when either is stale.
- A file with a newer `config_version` than the build understands is refused with a clear message, before any of its keys is judged. An older one is migrated in memory, with a note. A new enforced section is added only with a new version, because an older build only warns about a top-level table it does not know.
- A file that exists and cannot be read, is not UTF-8, is not TOML or is larger than 1 MiB is an error. It is never a layer to skip: skipping a managed file would drop its locks.

**One declaration per key.** A key is declared once, as a field of a settings struct in `cineris-config`: its type, its default, what a project's file may do with it, and when a change takes effect. Validation, the trust rules, the reload notices, the JSON Schema and the reference are all read from that declaration, so none of them can describe a different configuration from the one the code reads.

**Reserved sections.** A section that belongs to a part of the harness that is not built yet is reserved: a file may hold it, what is in it is kept and not checked, and the step that builds the part declares its keys. Until then a reserved section fails closed for projects: it is the user's alone, or (hooks, extensions, MCP servers) waits for trust. It is never treated as a harmless preference by default.

## Trust and what a project may set

A repository must not be able to take over the harness by containing a configuration file.

| Class of key | Examples | Project layer |
|---|---|---|
| Harmless preferences | how much reasoning is shown, the `standard` speed tier, whether a refused tier falls back and how long its cooldown lasts, a `deny` or `ask` permission rule, a protected path | Applied always. |
| Can run code | hooks, extensions, MCP servers, custom commands with shell steps | Applied only after the user trusts the project, and pinned by hash ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md)). |
| Can widen access | permission `allow` rules, sandbox `write` paths, network on, a permission mode above the one in effect (up to `supervised`) | Applied only after trust. Shown to the user at that time. |
| Can raise cost | a speed tier other than `standard`, for the session or for a role; the model, for the main role and each other role (`model.main`, `fast`, `judge`), and a role's `provider`; the reasoning level (`model.effort`, a role's `effort`); `model.max_output`; `retry.attempts` | Applied only after trust; until then the user's own value applies. Shown to the user at that time. |
| First limit | a limit under `[limits]` where there was none, which could stop every turn (since step 23) | Applied only after trust; until then the session runs without it. Shown to the user at that time. |
| Stops commands | `shell.env_remove`, the variables commands are not given: `"*"` would take `PATH` from every command (the owner's decision of 2026-11-16, after the review of step 23) | Applied only after trust; until then the user's own list applies. Shown to the user at that time. |
| User-only | credentials, everything about a provider (base URL, headers, protocol, key variable), permission modes above `supervised`, the sandbox switch (`sandbox.enabled`, on or off), a higher limit, the selected profile, usage sources, the data directory, proxy and CA settings, the log level, the compatibility readers, the background summary (`context.background`), the map's wait (`index.map_wait`), anything under `[managed]` | Ignored in project layers, with a warning. Trust does not change this. |

- **Some keys are classed by their value.** A permission rule that denies or asks is a preference, and one that allows widens access. `sandbox.network = "off"` is a preference and `"full"` widens access. A speed of `standard` is a preference, and any other tier raises cost: a fast tier costs about twice as much per token, and an untrusted repository must not be able to raise what its user spends (the owner's decision, 2026-11-13). A project can always bring the speed down. A permission mode at or below the one already in effect is a preference; above it, up to `supervised`, it widens access; above `supervised` it is the user's alone.
- **The other keys that raise cost wait for trust whatever their value** (the owner's decision, 2026-11-13): the model and a role's provider, the reasoning level, `model.max_output` and `retry.attempts`. Whether one model or level costs more than another depends on the model catalog, which `cineris-config` does not have, so a project's value of any of them waits, and the user's own applies until the project is trusted. A trusted project may set them like any other key.
- **The sandbox switch is the user's either way.** A project's `sandbox.enabled` is ignored with a warning whether it is `false` or `true`: a user who turned the sandbox off did so on purpose, and the key keeps one rule.
- **A limit can be tightened by anyone and loosened by the user alone.** A project's limit applies when it is at or below the one in effect. A higher one is ignored, trusted or not: a budget is the user's.
- **A first limit waits for trust** (the owner's decision, 2026-11-16). A limit where there was none applies only once the user trusts the project: an untrusted repository's `[limits.turn] time = "0s"`, `requests = 1` or `cost = 0` would otherwise stop every turn in it at once. Until then it is held back as a setting that needs trust is, listed in the trust question with its value, and the session runs without it. It is the class *first limit*, and of the tighten-only keys it reaches only those with no default: `requests`, `tokens`, `cost` and `time` under `[limits.turn]`, `[limits.session]` and `[limits.day]`. A value at or below a limit already in effect, the user's or a default, still applies untrusted.
- **So can a wait or a time limit** (the owner's decision, 2026-11-15): `retry.backoff_base`, `backoff_factor`, `backoff_cap` and `max_wait`, the stream timeouts `retry.idle_timeout` and `first_event_timeout`, `model.speed_wait`, `shell.timeout` and `shell.max_timeout`. A project's file may shorten one and may not lengthen it, trusted or not, so that a repository cannot make the sessions in it wait; the warning for a longer one names the value in effect. The cooldowns `model.speed_cooldown` and `speed_cooldown_min` slow a session without stalling it, and stay preferences. Turning the background summary off would make every compaction wait for its summary, so `context.background` is the user's alone.
- **A length of time or a number has a range, and the range binds every file,** the user's own included: a value outside it is an error, and the file is refused. The floor matters as much as the ceiling, because a project may lower a tighten-only key: no file may make every request fail with a millisecond's `first_event_timeout`, or retry at once with no `backoff_base`. Each key's range, and the reason for each end, are in the [reference](../configuration/reference.md). **A length of time's range is declared as the reference writes it** (since the seventh tooling step): `[TightenOnly, Now, 100ms..=1m]` in `crates/cineris-config/src/schema/sections.rs`, not `100..=60_000` in milliseconds. The bounds are read when the crate is built, in the field's unit, so a length of time's range written as a bare number, a number's written with a unit, an unknown unit or a low bound above the high one fails the build. A test over the schema fails for a key a project's file can set that holds a length of time or a number and has no range, unless its short list gives the reason (`waits::every_number_a_project_can_set_has_a_range`): the time limits under `[limits]`, which stop on purpose and say so, and `index.pass_time`, which the core holds to 250 ms at least where it starts the index.
- **Joining a list is not a way in.** The entries a project adds to a joined list are judged one by one: its `allow` rules and its writable paths wait for trust, and the user's entries stay.
- **What is held back is listed.** A load returns every project setting that is waiting for trust, with its class, its value and the line it is on, so a front end can show exactly what trusting the project would turn on.
- **Trust is an input.** `cineris-config` is told whether the project is trusted. The trust store, and the hashes that pin what was trusted, are `cineris-policy`'s (built in step 12: [06-permissions-and-sandbox.md](06-permissions-and-sandbox.md), "Project trust"). A project counts as trusted for its allow rules only while those rules are the ones the user approved: the core asks the trust store with the rules the project's files hold now, and passes the answer here and to the engine.
- **The engine checks again.** Every rule reaches the permission engine with the layer it came from, and the engine sets a project's allow rules aside itself when it is told the project is not trusted. Holding them back here keeps them out of `cineris config show`; holding them back there is what enforces it.
- **The rules apply to both project files and to a profile a project defines.** The user's own layers, the environment and the command line are not subject to them.

The class of every key is part of the schema. A test snapshots the whole table, so a new key cannot arrive without a reviewer reading its class, and another checks the classes this page names.

## Sections

The schema, in outline. The full list, generated from the declarations, is the [configuration reference](../configuration/reference.md).

| Section | Holds | State |
|---|---|---|
| top level | `config_version`, `profile` | Declared. |
| `[profiles.<name>]` | The profiles. | Declared. |
| `[managed]` | `locked`: the keys no higher layer may change. | Declared. |
| `[model]` | Models by role (`main`, `fast`, `judge`), reasoning level (`effort`) and visibility (`reasoning`), maximum output, and the speed tier: `speed` for the session, which the `main` role runs at; `roles.<role>.speed` for a role of its own (the utility roles are at `standard` unless set); `speed_fallback`, `speed_wait` and `speed_cooldown` for what happens when a tier is refused ([04-providers.md](04-providers.md), "Speed tiers"; [decision 0027](../decisions/0027-speed-tiers.md)). `roles.<role>` also holds a role's own `effort` and `provider`. `speed_cooldown_min` (step 13a, default `"1m"`, a preference) is the least a cooldown lasts when the provider names a time ([decision 0027](../decisions/0027-speed-tiers.md)). See [14-adaptation.md](14-adaptation.md). | Declared. |
| `[providers.<name>]` | Protocol, base URL, the key's environment variable (`key_env`), extra headers, the two stream timeouts, capability flags. Built-in providers need no entry. | Declared, except the flags, which are read since step 13a as the flags of the provider's protocol (`[providers.<name>.capabilities]`: text, a whole number or `true`/`false`; [04-providers.md](04-providers.md) lists the ones `anthropic-messages` reads). |
| `[models.<name>]` | Catalog overrides and additions. Each table is one catalog entry, with exactly the keys of a `[[model]]` table in `catalog/models.toml`, whose header documents them: `provider`, `id`, `name`, `protocol`, the limits, `prices` in US dollars per million tokens, `reasoning`, `capabilities`, `variants`, `flags`, and a `speed` list with each tier's own rate card ([04-providers.md](04-providers.md), "Model catalog"). The table's name is the user's own label for it. An entry **replaces** the built-in entry for the same provider and model id whole, and nothing is merged field by field, so an entry says everything about its model. An unknown key is an error, as it is in the built-in file: a misspelt price must not become "no price". The catalog file's own form, a `version` and a list of `[[model]]` tables, cannot be used here, because `[model]` is already a section of this file; the loader gives the user's entries a version of their own, so that a usage record shows that a price came from them. | Read since step 13a, by the catalog's own strict reader when the core starts: an entry it cannot read stops the core, with the file, the entry and the problem ([04-providers.md](04-providers.md), "Model catalog"). Not declared key by key here: the catalog's file documents its keys, and refuses one it does not know. User only. |
| `[http]` | The HTTP client: connect timeout, proxy, extra certificate authorities. | Reserved. |
| `[retry]` | `attempts`, `backoff_base`, `backoff_factor`, `backoff_cap`, `max_wait`, and the stream timeouts `idle_timeout` and `first_event_timeout`, which a provider's entry can override for that provider, within the same range. The waits and the timeouts are tighten-only (2026-11-15). | Declared. |
| `[permissions]` | `mode` and `rules`. Enforced. | Declared. |
| `[sandbox]` | `enabled`, the extra writable paths (`write`), the protected paths (`protect`), `network`. Enforced. | Declared. |
| `[limits]` | `requests`, `tokens`, `cost` (US dollars) and `time`, under `turn`, `session` and `day`. Enforced. | Declared. |
| `[credentials]` | `store`: where stored keys are kept. Never a key. | Declared. |
| `[session]` | Data directory (`data_dir`), `durability` (`item`, `turn` or `never`: when the session log is flushed; user only, because `never` could cost the user their sessions), retention. | `data_dir` and `durability` declared. |
| `[compat]` | Which of the other tools' files to read: `instructions`, `mcp`, `skills`. | Declared. |
| `[log]` | Level, targets, rotation. | `level` declared. |
| `[tools]` | Enabled tools, output limits, per-tool settings. | Reserved. |
| `[shell]` | Program and arguments (user only), the default and the longest time limit of a command (`timeout`, 2 minutes, from a second to half an hour; `max_timeout`, 30 minutes, from ten seconds to a day; both tighten-only, and a command's limit is held to `max_timeout` where it is read), environment variables to remove. | Declared. |
| `[context]` | `auto_compact`, `background`, `compact_at` (0.85), `prepare_at` (0.70), `keep_recent_tokens`, `instruction_names` (more names, plain `.md` files only) and `instructions_cap` (48 KiB) ([07-sessions-and-context.md](07-sessions-and-context.md)). The thresholds and the names raise what is spent, so a project's value waits for trust; turning compaction off is a preference, and turning the background summary off is the user's alone (2026-11-15). A `compact_at` below what a compaction needs, or a `keep_recent_tokens` past what one can replay, is held to it when it is used, and the user is told (since the review of step 15). | Declared in step 15. |
| `[verify]` | Verification mode, commands, time limits, rounds. See [15-efficiency.md](15-efficiency.md). | **Declared in step 19:** `mode` (`auto`, `ask`, `off`; a project's file may turn it down and not up), `commands` (the user's alone: a repository proposes its checks as command entries of its shared profile, which each user accepts by their content hash, in place of a project's list held for trust and pinned, which an earlier draft of this page had; [steps/19/answered.md](../steps/19/answered.md)), `rounds` (2, from 1 to 5), `round_time` (2 minutes, from 10 seconds to an hour) and `turn_time` (4 minutes, from 10 seconds to 4 hours), the last three tighten-only. All at once: read when a session is put together, and applied to a running one by step 24's reload. |
| `[adapt]` | Variant overrides per model, whether to explore, (the profile's token budget is `project.profile_tokens`, the map's `index.map_tokens`). | **`variants` declared in step 16:** `[adapt.variants."<model id>"]` with `edit_format`, `tool_schema`, `prompt_style` and `effort`, each user-only and from the next epoch. Whether to explore, and the budgets, are later steps'. |
| `[project]` | What the harness learns about a project ([14-adaptation.md](14-adaptation.md)). | **Declared in step 17:** `learn` (default `true`; user-only, at restart: whether the command observer learns from the commands a session runs) and `profile_tokens` (1,500, from 0 to 8,000; tighten-only, from the next epoch: the budget of section 5 of the system prompt). |
| `[stats]` | Recording on or off, retention. | **Declared in step 16:** `enabled` (default `true`, user-only, at restart) and `retention` (months a monthly file stays after its own, default 3, user-only, at restart: read when the core starts, until step 23's reload re-reads it). |
| `[index]` | The repository map ([15-efficiency.md](15-efficiency.md)): `map` (whether epochs are given one), `map_tokens` (1,500), `map_wait` (unset: 150 ms in the interactive app, none headless), and the index's caps `max_files` (20,000), `max_file_bytes` (1 MiB) and `pass_time` (2 s of reading a pass, never less than 250 ms whatever is set). The budget and the caps are tighten-only: a project's file may lower them, and raising one is the user's alone. | Declared in step 18. |
| `[runs]` | Run engine: concurrency, workspace location, set-up command, budgets (after the first release). | Reserved. |
| `[usage]` | Week start, calendar or rolling ranges, the opt-in refresh, and, after the first release, which other tools' usage records may be read ([18-usage.md](18-usage.md)). | **Declared in step 16:** `week_start` (`monday` to `sunday`), `ranges` (`calendar` or `rolling`) and `refresh` (the providers whose usage endpoint may be asked), each user-only and read when the core starts (at restart, until step 23's reload re-reads them). The other tools' sources are step 45's. |
| `[tui]` | Presenter (inline or full screen), theme, density, markdown and diff options. | **Declared in step 23:** `prose_width`, `tool_detail`, `tool_output`, `reasoning`, `density`, `turn_summary`, `reduced_motion`, `glyphs` and `theme` (`auto`, `dark`, `light`; a name not known is said, and `auto` used), each user-only and in force at once. The presenter is step 25's. |
| `[keys]` | Key bindings ([09-tui.md](09-tui.md), "The editor as built"): a table for each context of the key map (`editor`, `menu`, `picker`, `toggle`, `search`, `focus`, `dialog`, `global`), from an action's name to the keys bound to it. | **Declared in step 23,** user-only. The interface checks the names and the keys, and says each entry it cannot use. |
| `[terminal]` | Capability overrides, each defaulting to automatic detection. All terminal quirks live here. | **Declared in step 23:** `sync`, `keyboard`, `color` (`none`, `16`, `256`, `truecolor`), `hyperlinks` and `unicode`, each user-only and read at start. |
| `[rpc]` | The RPC server's waits: `approval_timeout`, `stalled_client` (since step 27). The user's alone: read once, when `cineris serve` starts. | Declared. |
| `[hooks.<name>]`, `[extensions.<name>]`, `[skills]` | [12-extensions.md](12-extensions.md) | Reserved. |
| `[mcp.servers.<name>]`, `[mcp]` | [13-mcp.md](13-mcp.md) | Reserved. |

**Values from elsewhere.** A value can be written as `{ env = "NAME" }` to read an environment variable when it is used, where the reference says a key takes "text, or `{ env = "NAME" }`": today a provider's extra headers. A value read this way is a `Secret`, and so is literal text in such a key (see "Credentials"). There is no general interpolation syntax and no "run this command to get a value" form in the first release: a configuration file that runs commands is a larger attack surface than the feature is worth.

## Credentials

- **Never in `config.toml`, by design.** No key is meant for a credential. A provider entry names where its key comes from: an environment variable (`key_env`) or the credential store.
- **One key can hold one anyway: a provider's extra headers.** Some gateways take their key in a header (Azure's `api-key`), and a header's value may be written as text. So literal text there is held as a `Secret` from the moment it is read, and `cineris config show`, `--layer`, the configuration's `Debug` form and what a load lists as held back for trust all print it as `[redacted]`. A variable's name, `{ env = "NAME" }`, is shown, and is the better way to write such a header. A later step that declares a key that can hold a secret gives it the same kind, text or `{ env = "NAME" }`, so that it is redacted the same way.
- **Until then, a reserved section is redacted by a guess.** A section whose keys are not declared yet holds whatever a file puts there, an MCP server's token included, so its values are redacted whole under a key that names a secret by its look: `env` or `headers`, a name with `token`, `secret`, `password`, `auth` or `credential` in it, or a part of the name that is `key` or ends in `apikey` (`api_key`, `X-Api-Key`, and not `keymap`). Everything else in the section is shown. The step that declares the section replaces the guess with the declared kind. The section's own reader, `Loaded::section`, gets the values as written.
- **A web address's user part is redacted in any value**, declared or not: `http://user:password@proxy` shows as `http://[redacted]@proxy`, in the views above and in the `Debug` form of the typed settings.
- **Lookup order:** command-line flag, then the provider's environment variable, then the credential store. A flag or a variable that is empty counts as not given. The store is not asked when an earlier source answers, so a broken store does not get in the way of a key from the environment.
- **The store.**

| OS | Default | Alternative |
|---|---|---|
| Windows | Windows Credential Manager | A file, `credentials.toml` in `%LOCALAPPDATA%\cineris\credentials`: in the local folder, not the roaming one beside the configuration, so that a stored key stays on its machine (Windows copies a roaming profile to every machine the user signs in to); and in a folder of its own there, which the harness closes whole to its tools. |
| Linux | A file, `credentials.toml` in the configuration directory, with owner-only permissions | The Secret Service keychain, with the `keychain` build feature and `credentials.store = "keychain"`. **Not built yet:** the value gives an error that says so and names the file store. |

On Linux the keychain is not the default because it depends on a desktop session service that is absent on servers, in containers and over SSH, and because the client library for it is a large dependency. See [decision 0018](../decisions/0018-credential-storage.md).

- **`credentials.store`** is `auto` (the OS default), `file` or `keychain`. It is the user's alone.
- **Commands.** `cineris auth login <provider>` reads a key without echo and stores it. `cineris auth status` shows which source is active for each provider, never the value. `cineris auth logout <provider>` removes it.
  - **As built in step 13b,** `auth login` and `auth logout` take only a provider the core knows (a key stored under a typo would never be used), and `auth login` reads the key from standard input when it is not a terminal (`cineris auth login anthropic < key-file`). `auth status` lists the built-in providers and the user's own, each with where its key comes from.
  - **Since step 25, `auth login` at a terminal** reads the key there without echo, with the first run's input (`cineris_tui::secret::read_key`): the row says how many characters it holds and never one of them, `Ctrl+U` empties it, `Esc` or `Ctrl+C` stores nothing (exit 2), and the terminal is put back whatever happens. A key typed or pasted is held in a `cineris_base::SecretBuf`, of fixed capacity, and given to the store as a `Secret`; a paste with a control character in it is refused whole, on every system, except for one line break at its very end, which is dropped (09, "The first run"). **The buffer is written over with zeros as a best effort,** when a character is taken away, when it is emptied and when it is dropped: ordinary writes, then `std::hint::black_box` on the buffer and a compiler fence, so that the compiler does not drop them as writes nothing reads again, with no `unsafe`. It promises no more than that: `black_box` is a hint the compiler is asked to honour, not a guarantee, there is no volatile write (it would need `unsafe`, or a crate for it), and a page of the buffer the kernel swapped out, or a core file written, while the key was in it holds a copy the wipe never reaches. **What is not wiped:** on its way in, the key passes through the terminal layer's read buffer and its decoder's, and a paste through `Event::Paste`'s string; the `Secret` given to the store is a `String` of its own. Those are the process's own memory, short-lived, cleared rather than written over (the allocator takes them back with the key still in them), and never written anywhere: no log line, history, notice, transcript block or crash report holds the key.
  - **The subcommands of step 13b,** all through the core so that each sees what a session would: `config show [--origin] [--layer NAME]` and `config check` read the project's layers with the project's trust as a session judges it, and `check` loads strictly (an unknown key is an error, exit 2); `config defaults`; `sessions list [--all] [--limit N] [--json]`; `permissions list | revoke ID… | trust | untrust` ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md), "Project trust"); `models`, the catalog.
- When both an environment variable and a stored key exist, the environment variable is used and `auth status` says so. A store that cannot be read does not fail `auth status` for a key that comes from a flag or the environment: the store's error is shown beside it.
- **In memory** a credential is a `Secret`: not printable, not serialisable, never logged. No error from a store carries one, and an error about a malformed credentials file names a line number and never the line.

**The file store.**

- `credentials.toml` is written owner-only on every write: mode `0600` on Linux, and on Windows an access list with the user alone ([decision 0029](../decisions/0029-owner-only-files-on-windows.md)). The file is private from the moment its temporary file is created.
- A file someone made by hand with wider permissions is still read, and reported as not private: the status of a provider's credential carries the warning, for `cineris auth status` and the diagnostics to show. The next write makes it private. Its directory, if it was already there, is left as it is. Refusing to read it would stop people working on single-user machines for little gain.
- Access to the credentials file is guarded by a lock file, so two processes cannot interleave writes. The lock is the operating system's, held on an open file across the read, the change and the write: it goes when its holder exits, however it exits, so there is no stale lock to clear. A reader needs no lock, because the file is replaced in one step.
- A file with a newer format version, or one that is not TOML, is an error and is left untouched. Entries this version does not know are kept when the file is rewritten.
- **The file and its lock are files of their own, never a link.** A link at either path is refused with an error that says so, and `cineris auth status` reports it: a link there could lead to any file of the user's, one in a repository of dotfiles say, and the key would leave with it. There is no look first and use after, so nothing to race: both files are opened with `cineris-base`'s `open_regular`, which refuses a link at the name instead of following it, and the file is written with the atomic write's `follow_links` off, which refuses a link there and replaces, never follows, one swapped in at the last moment. A directory on the way that is a link is followed, as the user's own choice.

**Windows Credential Manager.** One generic credential per provider, named `cineris:<provider>`, visible and removable in the Windows "Credential Manager" control panel. It is stored for this machine only, not copied to the user's other domain machines. A key is printable ASCII: an entry that reads back as anything else (another program's key stored as UTF-8 bytes reads back as letters of other scripts) is an error, never sent to a provider, and such a key is not stored.

**Where `cineris-config` ends.** The `AuthMethod` trait, which puts a credential on a request ([04-providers.md](04-providers.md)), is in `cineris-llm`, and `cineris-config` may not depend on that crate. `cineris-config` owns the stores and the lookup order, and returns the key with a description of where it came from. `cineris-core` hands both to the provider layer's API-key `AuthMethod`. A sign-in flow added later stores its tokens through the same store.

## Other state

Separate from configuration, in the state directory (`$XDG_STATE_HOME/cineris`, `%LOCALAPPDATA%\cineris\state`):

| File | Holds |
|---|---|
| `trust.toml` | Trusted projects and the hashes of what was trusted. `cineris-policy`'s: one `[[project]]` table per project, read strictly, so that a file this build does not understand in full trusts nothing. |
| `projects/<project key>/knowledge.toml` | What the harness has learned about a project ([14-adaptation.md](14-adaptation.md)). Since step 17, beside it: `knowledge.toml.good`, the last store read or written as valid, and `knowledge.toml.lock`. |
| `projects/<project key>/index/` | The repository-map cache ([15-efficiency.md](15-efficiency.md)). |
| `stats/` | Local statistics ([14-adaptation.md](14-adaptation.md)). |
| `approvals/<project key>.toml` | "Always allow in this project" rules. `cineris-policy`'s: one `[[approval]]` table per rule, read strictly, and only ever an allow. |
| `catalog-overlay.toml` | Results of `cineris models refresh`, in the catalog file's own form: a `version` and `[[model]]` tables. |
| `history` | Prompt history for the editor, per project. |
| `logs/` | Log files and crash reports. |

Nothing in the state directory is needed to run. Deleting it loses convenience, not data.

- **`cineris-config` says where each file is.** What is in a file is its owner's business: the trust store is `cineris-policy`'s, the knowledge file `cineris-knowledge`'s, and so on.
- **A project key is one directory name.** A key that could be a path (a separator, a dot, a drive letter) has no place in the state directory.
- **The state directory is created for the user alone.** One that already exists is not changed: if it is open to other users, that is reported as a warning and left, because its permissions may be the user's choice. So a private file in it is written private itself, and never relies on the directory.

## Where each system keeps the harness's files

Each folder beside what the permission engine closes in it ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md), "The built-in protection"). The layouts differ, and what is closed has to be read against both: on Windows the stored keys were in the data directory, which is otherwise closed only to writing, and nothing closed them to reading until the review of step 13a found it. They are now in a folder of their own there, closed whole. `Dirs::from_env` in `cineris-config` works the folders out; a test of the core gives it each system's layout on every system (`start::the_stored_keys_are_closed_to_the_tools_in_each_systems_layout`).

| What | Linux | Windows | Closed by the engine |
|---|---|---|---|
| The configuration directory: `config.toml` | `$XDG_CONFIG_HOME/cineris`, usually `~/.config/cineris` | `%APPDATA%\cineris`, the roaming folder | Reading and writing, the whole directory. |
| The user's configuration file, when `CINERIS_CONFIG` names one elsewhere | Where it names | Where it names | Reading and writing, the file by name, and where it leads when it is a link (a dotfiles repository), here and in the configuration directory alike (since the review of the fixes to step 13a): it holds the level and the rules the next session starts with. A file it names inside the workspace (a team's configuration, committed) is closed there too, so a search over the workspace (`rg`, `grep -r`) is asked about at every level, as one over any tree with a closed file in it is. |
| The stored keys: `credentials.toml`, its lock `credentials.toml.lock`, and a write's temporary file | In the configuration directory | In `%LOCALAPPDATA%\cineris\credentials`, a folder of their own in the data directory | Reading and writing, the folder whole (on Windows its own; on Linux the configuration directory). A write's temporary file (`.cineris-<hex>.tmp`), which a write that was killed leaves behind holding every key, is in the folder, and closed with it (since the review of the fixes to step 13a). |
| The state directory: trust, stored approvals, `shells.json`, logs | `$XDG_STATE_HOME/cineris`, usually `~/.local/state/cineris` | `%LOCALAPPDATA%\cineris\state`, inside the data directory | Reading and writing, the whole directory. |
| The data directory: sessions | `$XDG_DATA_HOME/cineris`, usually `~/.local/share/cineris`, or `session.data_dir` | `%LOCALAPPDATA%\cineris`, or `session.data_dir` | Writing, the whole directory; and reading, the sessions of other projects. |
| The managed file | `/etc/cineris/config.toml` | `%ProgramData%\cineris\config.toml` | Not closed: only an administrator writes it, and it holds an administrator's policy, not a key. |

`session.data_dir` moves the sessions and not the stored keys: on Windows these stay in their folder in the local folder, and are closed there.

## Windows notes

- Paths in configuration accept `/` or `\`. A leading `~` expands to the user's profile directory.
- Files are read as UTF-8, with or without a byte-order mark, and with either line ending. A line number in an error is the same either way.
- Owner-only permissions are applied through access-control lists ([decision 0029](../decisions/0029-owner-only-files-on-windows.md)).
- The names of environment variables ignore case, as they do on Windows.
- A project's files are reached through the path resolver, which refuses a link to a network share before the network is asked; the workspace's own share is allowed when the workspace is on one ("Layers").
- The workspace root counts a `.git` or `.cineris` owned by the user or by the Administrators group, and passes over a link or a junction there ("Layers").
- The managed file's folder is read from `%ProgramData%`. A user who changes that variable can make the harness miss the managed file: locks are an administrator's policy for a shared machine, not a boundary against the person running the program.
