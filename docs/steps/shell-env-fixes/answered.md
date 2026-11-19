# Answered in the shell environment's fixes, on 2026-11-19

A small security fix outside the numbered steps. It settles two open items about the environment the `shell` tool gives a command: `a-provider-key-break-no-longer-bites` and `an-empty-path-runs-the-workspaces-files`. Both items' text is below, under each fix. The design is in [05-tools.md](../../architecture/05-tools.md), "`shell`", "Environment", and the table of [security.md](../../security.md), section 2.

## The provider keys' two paths

**What was open.** Since the reload (commit `24673066`, step 24), the providers' key variables reach a shell command's removal list by two paths:

- **The removal list.** The core seeds the configuration's `env_remove` from `provider::key_variables` (`assemble.rs`).
- **`ShellLive`.** It holds the key variables as a reload leaves them, and `ShellTool::removed` adds them to the list.

Step 13a's break `provider-key-variables-left-in` emptied only the first, so with the break made the second still removed every key, and the break bit nothing. No test could see either path lost alone, since each covers the other when a session starts.

**What was done.**

- **The break is at the source.** `provider-key-variables-left-in` now empties what `provider::key_variables` returns, which both paths read. It is caught by the two core system tests that print a command's environment.
- **Each path is a promise, and a test pins each one alone.** `cineris-core`'s `shell::sys_a_reload_withholds_a_new_key_variable_and_still_the_old_one` starts a session whose provider reads `OLD_PROVIDER_KEY`, reloads the configuration with it reading `NEW_PROVIDER_KEY`, and runs a command:
  - The new variable is withheld by `ShellLive` alone. A break of `ShellTool::removed`, or of the reload's hand-over, lets it through.
  - The old one is withheld by the removal list alone. A break of the seeding lets it through.
  - Each failure message names the path that was lost.
- **The seeding stays.** It keeps a variable that has stopped being a key withheld for the rest of the session, which is the safe way to be wrong. [05-tools.md](../../architecture/05-tools.md) now says so.

## A `PATH` that names no absolute directory

**What was open.** `env.rs` drops every `PATH` entry that is empty or relative, and passed `PATH` on empty when none was left. The unit test pinned "the variable stays, empty". An empty `PATH` makes `sh` and `bash` look in the current directory, which is the workspace: a workspace file named `ls` ran as `ls`, which is what dropping the relative entries is there to stop. Unsetting `PATH` is no fix either. With none, a shell takes its own default. Upstream bash's default ends with `.`: `DEFAULT_PATH_VALUE` in its `config-top.h`. Ubuntu's build drops the `.`; there both `bash` and `sh` gave `/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`.

**What was done.**

- **On Unix a command is never given an empty `PATH`, or none.** This holds when no absolute entry is left, when there is no `PATH`, and when `shell.env_remove` names it. The command is then given `PATH=/usr/bin:/bin`, the system's own directories, which root writes and a command does not.
- **The model is told.** While it holds, the tool's description says the harness was started with no absolute directory in `PATH`, that commands are given `PATH=/usr/bin:/bin` in its place, and that a program elsewhere must be run by its full path. The description is otherwise byte for byte what it was, and so is the request's cache.
  - Whether to say it is told by the code that replaces the `PATH` (`env::path_replaced`, from `env::assemble`), with the configured removal list, so the description and the command cannot disagree. A harness whose own `PATH` is `/usr/bin:/bin` is not told it was replaced.
- **An environment that names `PATH` twice gives a command one.** Unix allows the name twice in `environ`. The child would be given the last, and the harness reads the first. The first is kept, without its relative entries. This was found by the early review (below): with `PATH=/usr/bin:/bin` then `PATH=.`, the command was given an empty `PATH` and ran the workspace's `ls`. On Windows a second spelling of `Path` is dropped the same way.
- **Tests.**
  - `env::tests::path_keeps_only_its_absolute_directories` covers every replaced case, and the duplicate on both systems. It pins the value itself too: a directory such as `/tmp` would be one a command could put an `ls` in.
  - `shell::tests::the_model_is_told_when_path_is_replaced` covers the description.
  - `cineris-tools`' system test `shell::{bash,sh}::sys_a_workspace_file_named_like_a_command_does_not_run` plants an executable `ls` at the workspace's root and in `bin/`. It runs `ls` through the tool with `PATH` as `.`, empty, `:bin:./bin`, missing, absolute with `.` in front, and named twice. In every case it checks that the planted file did not run, and reads the `PATH` the command was given from `export -p`. `$PATH` alone would show a default the shell made up for itself.

**The value: settled by the coordinator, on the owner's delegation, on 2026-11-19.** The first build gave `PATH=/dev/null`, which finds nothing. A default of directories is a widening over that: before the fixes, a bare name under such a `PATH` found only the workspace's files. So it went to the owner, who left it to the coordinator, who took `/usr/bin:/bin` in both cases (no absolute entry left, and none at all). The reasons:

- **It is the smallest standard set.** The directories `getconf PATH` gives (`/bin:/usr/bin`), with `/usr/bin` first because a merged-`/usr` system links `/bin` to it. `/usr/local/bin` is left out: it holds what was installed by hand, not the system's own.
- **It finds what the engine already assumes.** A bare `ls` is the system's `ls` to the permission engine, and with this `PATH` it is.
- **It widens only to root's directories.** Under `/dev/null` a bare name finds nothing. Here it finds programs the system installed, which a command cannot write.
- **With no `PATH` at all, it narrows.** A shell's own default was wider: `/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin` on Ubuntu, and `.` after them in upstream bash.

## Windows: `cmd.exe` and the current directory

**Measured on a hosted Windows runner on 2026-11-19.** The test is `cineris-tools`' `shell::{cmd,pwsh,powershell}::sys_cmd_does_not_look_where_the_command_runs`, which the Windows commit of this fix adds. It runs one command through the shell tool, in a workspace that holds:

- a `hostname.bat` and a `git.cmd`, each printing that it was the workspace's;
- `outer.cmd`, which calls `inner` by its bare name;
- `outer_dp0.cmd`, which calls `"%~dp0inner.cmd"`;
- `inner.cmd`.

The command runs `hostname`, `git --version`, `.\hostname.bat`, `.\outer.cmd` and `.\outer_dp0.cmd`.

- **Without `NoDefaultCurrentDirectoryInExePath`:** try run 44434595893, on a scratch branch with the variable's name misspelt so that nothing reads it.
- **With it:** try run 44434599143, on this branch.

| | `hostname` ran the workspace's | `git` ran the workspace's | `.\hostname.bat` ran it | `inner` by bare name, from `outer.cmd` | `"%~dp0inner.cmd"` |
|---|---|---|---|---|---|
| `cmd.exe`, without | **yes** | **yes** | yes | found | found |
| `cmd.exe`, with | no: the machine's name | no: `git version 2.56.0.windows.4` | yes | not found | found |
| PowerShell 7 and Windows PowerShell 5.1, without | no | no | yes | found | found |
| PowerShell 7 and Windows PowerShell 5.1, with | no | no | yes | not found | found |

**What it shows.**

- **`cmd.exe` shadows real programs.** Through the shell tool, `cmd.exe` runs a workspace file named like a program on `PATH` in that program's place: a `git.cmd` beat the real git. It does so whatever `PATH` holds.
- **PowerShell never looks in the current directory for a command.** But a batch file it starts runs in `cmd.exe`, and there the batch file's own calls by bare name look in the current directory.
- **What the variable costs.** With it, a file named by its path still runs, from either shell. A batch file that calls another of its directory by its bare name no longer finds it, and one that calls it through `%~dp0` does.
- **The decision is the Windows commit's** (below).

**Settled by the coordinator, on the owner's delegation, on 2026-11-19: the variable is set.** The owner's item `cmd-searches-the-current-directory` was left to the coordinator, with this default: if the runner showed a workspace file shadowing a program on `PATH`, set the variable, because shadowing a real tool is worse than making batch files call each other with `.\` or `%~dp0`. It did (`git.cmd` over git), so every command on Windows is given `NoDefaultCurrentDirectoryInExePath=1`, and the item is closed. It is the last commit of the pull request, so that it can still be dropped. Step 13a's recommendation, to leave the search on, is reversed.

- **The workaround is told.** It is in [05-tools.md](../../architecture/05-tools.md) ("`cmd.exe` and the directory a command runs in"). The tool's description on Windows tells the model too: a program or batch file where the command runs is found only by its path (`.\build.cmd`), and a batch file calls another of its folder as `call "%~dp0other.cmd"`.
- **Tests.**
  - `env::tests::on_windows_cmd_is_told_not_to_look_where_the_command_runs`: the variable is given once, under Windows' spelling, and not on Linux.
  - `shell::tests::the_model_is_told_how_a_file_where_the_command_runs_is_found`: the description.
  - The system test above, on each Windows shell.
- **Breaks:** `mutations/shell-env-fixes-windows.breaks`, a list of its own so that the commit can be dropped without touching the fix's other list.

## Every other place a process is started

Looked at for the same two holes: a bare name searched in the workspace, and the key variables. The look covered every `Command::new` and process start in the product's crates.

| Where | Program, and how it is found | Its environment | Verdict |
|---|---|---|---|
| `cineris-tools`, `process` | Shell commands, and the project's checks, which run through the session's shell (`verify.rs`). An absolute program. | `env::build`'s, after `env_clear` | Fixed here |
| `cineris-sandbox`, `__sandbox` | The wrapped shell or git, by the absolute path `wrap` was given; `exec` searches nothing | What its launcher was given, and `TMPDIR` | Follows the caller |
| `cineris-base`, `git::start` | The harness's own git, absolute and found outside the write roots | The harness's whole environment, and `GIT_ENV` | **Open:** [the-harness-git-is-given-the-provider-keys.md](../../outstanding/the-harness-git-is-given-the-provider-keys.md) |
| `cineris-tools`, `process::brief` (the shell's version) | The session's shell, found by `program::find`, which keeps an absolute `PATH` entry inside the workspace | The harness's whole environment, unconfined | **Open:** [the-shell-version-probe-runs-unconfined-with-the-keys.md](../../outstanding/the-shell-version-probe-runs-unconfined-with-the-keys.md) |
| `cineris-tui`, `external` (the external editor) | Absolute, found outside the write roots; on Windows it sets `NoDefaultCurrentDirectoryInExePath` | The user's own, to the user's own editor | As designed (step 21) |
| `cineris-sandbox`, the process guard | The harness's own executable | The harness's | As designed |
| MCP servers, hooks | Not built: MCP is a skeleton and `hooks` is reserved | | Nothing to look at |
| `cineris-term` pseudo-terminals, and bare names elsewhere (`cmd`, `mkfifo`, `sh`) | | | Tests only |

Rust's own `Command` on Windows never searches the current directory for a bare name (since 1.58), so no product start of a program is the hole. The shell's `cmd.exe` is, as the section above measures.

## The reviews

### The early review of the environment change (Opus, 2026-11-19)

Read on its own, before the rest of the diff, by a reviewer that had not seen the reasoning.

| Finding | Fix |
|---|---|
| **Medium, confirmed:** a `PATH` named twice reached the command empty. The second `PATH=.` was filtered to empty, the first kept `absolute` true, and the child took the last. It ran the workspace's `ls`, and the description said nothing. | One `PATH` is passed on, the first's absolute entries ("A `PATH` that names no absolute directory", above). The unit test and the system test both have the case, and the break `a-second-path-passed-on` puts the defect back. |
| Low: the description could disagree with the command when `shell.env_remove` names `PATH` (or `P*`). | `path_replaced` is read from `build`'s own output, with the configured list. The break `the-description-not-told-of-the-removal-list`. |
| Tests that pass with the fix weakened: a `NO_ABSOLUTE_PATH` of `/tmp`; a check of `path_replaced` on a name other than `PATH`. | The value is pinned (`a-path-that-holds-a-program`), and a missing `PATH` beside an absolute `HOME` is a case. `path_replaced` is told by the code that replaces, not by the value's text (`the-description-judged-by-the-paths-text`). |
| A `false` in `ShellTool::new`'s judgement survives every test. The harness's own path never sets an inherited environment, and a test cannot change its own process's. | `new` and `with_inherited_env` both go through one function (`described`), whose two arms are tested through the second. What is left untested is `new` passing `None`, which is the harness reading its own environment. |
| Low: the design page was not changed yet. | Changed ([05-tools.md](../../architecture/05-tools.md), [security.md](../../security.md)). |
| Info: bash's `. name` and `source name` read `./name` when the `PATH` has none, whatever the `PATH`. | The command analysis lists `.` and `source` as words that run another file as code, never passed unasked (`cineris-policy`, `command/posix.rs`). Said in [security.md](../../security.md). |

The reviewer checked `/dev/null`, the value of the first build, against bash, dash and glibc's `execvp`: none falls back to the current directory, and making `/dev/null/<name>` resolve takes root's write to `/dev`. The value was then settled as `/usr/bin:/bin` (above).

### The review of the whole diff (Opus, 2026-11-19)

The three commits as they then were, read against `main` before the first push by a reviewer that had not seen the reasoning. **It found no hole:** both key paths come from one list, the new core test tells them apart, the sandbox launcher passes the built environment on and starts the shell by its absolute path, and nothing in the change reaches a variable or a program it should not.

| Finding | Fix |
|---|---|
| **High:** six breaks named `in: build` after the body moved to `assemble`, so `check` failed and none could run. | `in: assemble`; `mutate --check` passes, and the verdicts are below. |
| Medium: the record, written in the first commit, described the commits after it, and kept `/dev/null` where the last commit changed the value. | The value was settled meanwhile and folded into the first commit (above). The Windows commit writes its own part of the record. |
| Medium: the Windows commit called itself the owner's decision while it left the item open, and pointed to evidence not yet measured. | The Windows commit is written after the runner's evidence, and settles or leaves the item by it ("Windows", above). |
| Low: no test or break held two rules: Windows given no `PATH` where it had none, and two spellings of `Path` on Windows. | Both are cases of `path_keeps_only_its_absolute_directories`; the first has the break `windows-given-the-unix-path`. |
| Low, in the Windows commit: no break of the removal of the user's own spelling of the variable; one break marked `os: windows` that the unit test catches on Linux; markers that PowerShell writes through its pipeline beside output a batch file writes to the handle; a claim that Windows' own process search reads the variable, which its documentation does not say. | Taken in the Windows commit. |

What is left untested, as the early review found: `ShellTool::new` reading the harness's own environment, which a test cannot change.

## The breaks

`mutations/shell-env-fixes.breaks` holds this fix's breaks; `step-13a/provider-key-variables-left-in` and `step-13a/path-keeps-relative-entries` were moved to the code as it is now. TO COME: the verdicts.
