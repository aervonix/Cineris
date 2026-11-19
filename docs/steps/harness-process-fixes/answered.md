# Answered in the harness's own processes' fixes, on 2026-11-19

A small security fix outside the numbered steps. It settles two open items the shell environment's fixes found ([../shell-env-fixes/answered.md](../shell-env-fixes/answered.md), "Every other place a process is started"): `the-harness-git-is-given-the-provider-keys` and `the-shell-version-probe-runs-unconfined-with-the-keys`. It opens one, `the-harness-git-reads-the-repository-git-names-in-the-environment`. The design is in [security.md](../../security.md), section 12, which holds the table of every process the product starts; [05-tools.md](../../architecture/05-tools.md), "Which shell"; and [06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md), "The harness's own git".

## The design, before building

Written before any code and settled by the maintainers on the owner's delegation, so that the rules did not grow case by case.

**Who controls what.**

| Who | Controls | Trusted |
|---|---|---|
| The user | The harness's environment (`PATH`, the provider keys, `HOME`, `GIT_*`), the user configuration, the programs outside the workspace | Yes |
| The repository's configuration | `.git/config` (filters, includes), `.gitattributes`, `.cineris/` | Only after the user trusts the project; and a confined command may rewrite it afterwards (a protected path: asked about at every level below `unrestricted`, and found by the look around a confined command) |
| The model, through commands | Anything inside the write roots while commands are confined; anything the user can write when nothing confines them (no sandbox, Windows, `unrestricted`) | No |
| The workspace's files | Whatever the model or the repository's author put there, an activated `.venv/bin` included | No |

**The line drawn:** a process the harness starts for itself, without the model asking, never sees more than a command sees: no provider key variable, no empty or relative `PATH` entry, and never a program a command could have written.

**The processes,** from an audit of every process start in `crates/` and `xtask/` by a sub-agent: the table of [security.md](../../security.md), section 12, as it was before the fixes. Two rows were holes. The harness's git was given the harness's whole environment (the keys, a relative `PATH`, every inherited `GIT_*`). The shell's version probe was given the same, unconfined, and the shell was found by a search that took an absolute `PATH` entry inside the workspace when nothing confined.

**What each part closes.**

1. **One builder,** `cineris_base::child_env`. It removes the keys of the providers the harness knows (`KNOWN_KEYS`, which was the shell's `DEFAULT_REMOVED`) and the list the core gives. It leaves one `PATH` of absolute entries (the first `PATH`; on Unix `/usr/bin:/bin` when none is left), and on Windows sets `NoDefaultCurrentDirectoryInExePath`. Everything else passes as it came. The shell's own builder (`env.rs`) is built on its pieces, so there is one `PATH` rule.
2. **The key list is passed in, not moved down.** `provider::key_variables` reads the configuration, which `cineris-base` cannot see, and a reload changes it. The core holds it in one value, `child_env::Withheld`: `ShellLive` holds it in place of a list of its own, and `SessionGit` holds the same value. `Confinement` gains `withheld()` (with no default body, so every confinement says what it withholds) and `inherited()`.
3. **Git.** `git::start` clears the environment and gives the builder's, then `GIT_ENV`. The sandbox launcher passes its environment on as it is given it, so the confined git and the unconfined one are both clean. Like the shell's commands, git still withholds a variable that was a key when the session started after a reload makes it stop being one.
4. **The probe.** `probe_version` runs with what a command is given (`env::build`, with the shell tool's removal list), after clearing. `brief::run` no longer inherits: it takes a whole environment. The probe stays unconfined: its program is never one a command could write, and it is given nothing a command is not.
5. **The search.** `ShellSearch` has `avoid`, as the external editor's search has, and finds with `find_outside`. The loop in `assemble.rs` that did the same by hand is gone; the note that names the shell passed over stays.

**The calls, settled by the maintainers (the owner can overturn them).**

- **A. The workspace is avoided for the shell whether commands are confined or not.** Where nothing confines (Windows above all), a file written in the workspace, by the `write` tool too, would otherwise become the program that runs every command, including the ones the engine passes unasked. It only narrows. A configured absolute `shell.program` inside the workspace is still taken.
- **B. The same for the harness's git.** Change detection runs git after every command, unasked. It only narrows.
- **C. `NoDefaultCurrentDirectoryInExePath` for every harness process on Windows.** A filter may start `cmd.exe`. It only narrows.
- **D. Git is given the keys' list only, not `shell.env_remove`.** That list is the user's, for commands; a pattern on it (`GIT_*`, `HOME`) would change what the user's own git configuration reads. Said in [security.md](../../security.md), section 12.
- **E. Inherited `GIT_DIR`, `GIT_INDEX_FILE` and the like** reach the harness's git: correctness, not a key or a program. Left as an open item, [the-harness-git-reads-the-repository-git-names-in-the-environment.md](../../outstanding/the-harness-git-reads-the-repository-git-names-in-the-environment.md), with `git rev-parse --local-env-vars` as the likely fix.
- **The launcher and the guard are the harness's own binary** (`current_exe`). The audit asked whether a confined command could replace it; it cannot use that, since a write root that holds the launcher's program turns the sandbox off for the session (`holds_harness_files`, step 26's second review).

**Left open on purpose.** Where nothing confines commands, withholding a key is defence in depth: a command running as the user can read the harness's own `/proc/<pid>/environ` (Landlock's ptrace scoping refuses that inside the sandbox), and can write an absolute `PATH` entry outside the workspace or a shell's start-up file, which no lookup tells from the user's own. A trusted repository's filters still run with the user's other variables, as under the user's own `git status`.

## What git relies on in its environment

Checked by the audit: the harness's git calls rely on nothing in the environment by name. Every behaviour is set by `GIT_HARDENING` and `GIT_ENV`, and its output is `-z` porcelain or plumbing, whose locale does not matter. Git itself reads `HOME` or `XDG_CONFIG_HOME` for the user's configuration (`core.excludesFile`, `safe.directory`, `core.autocrlf`), and on Windows `HOME`, `HOMEDRIVE`, `HOMEPATH`, `USERPROFILE` and `SystemRoot`. All of them pass as they came. `PATH` only matters for the programs a repository's configuration names, which is why it is fixed.

## The callers of `brief::run`

`brief::run` takes a whole environment and clears the inherited one. It has one caller, `Shell::probe_version_from`, which passes `env::build` of the environment it was given (the harness's own, or the options' environment for commands in a test) with the shell tool's removal list: everything a command is given, so a shell that needs `HOME`, `TMPDIR` or, on Windows, `SystemRoot` and `PSModulePath` to start still has them. `brief::answer`, which reads the change detection's git, starts nothing: that git is started by `git::start`.

## On Windows, with no absolute `PATH` entry left

The builder passes `Path` on empty when the harness's had entries but none absolute, and passes none when it had none, as the shell's commands are given; it never makes up a `PATH`, since Windows has no `/usr/bin`. With `NoDefaultCurrentDirectoryInExePath` set beside it, a bare name then finds nothing: the system finds its own programs through `SystemRoot`, and git is started by its absolute path. `child_env::tests::on_windows_a_harness_program_is_told_not_to_look_where_it_runs` pins the value; on the Windows runner, `git::tests::sys_a_filter_driver_is_given_no_key_variable` and the probe's test run through the builder there.

## Tests

- `cineris-base`
  - `git::tests::sys_a_filter_driver_is_given_no_key_variable` (Linux and Windows): a repository whose `.gitattributes` sends a file through a clean filter that writes three variables, hashed by `git::start`. Neither a key the confinement withholds, nor a known key, nor a variable of the test process left out of the environment given reaches the filter. The same filter, the key not withheld, sees it: so it ran.
  - `git::tests::sys_a_relative_path_entry_does_not_find_a_filter` (Linux): a filter named by a bare name, the same name in the repository, and `PATH` with `.`, an empty entry in front, behind, alone, and `./`. The program never runs; through an absolute entry it does.
  - `child_env::tests`: the keys and the rest; the `PATH` rule; Windows' variable and `Path`; the shared list.
- `cineris-tools`
  - `shell_search::sys_the_probe_is_given_no_key_variable` (Linux, a fake `bash`; Windows, a fake `pwsh.bat`): the probe sees no key, no variable of the test process, and only the absolute entry of `PATH`; the key not withheld, it sees it.
  - `shell_search::a_shell_in_an_avoided_directory_is_not_chosen` (both systems' rules): a `bash` or `pwsh.exe` in an avoided `.venv/bin` is passed over, by the default search and by a configured name.
- `cineris-core`
  - `harness_git::sys_a_reload_withholds_the_keys_from_a_filter_the_harness_git_runs`: a session at `unrestricted` whose command plants a filter; after a reload moves the provider's key from one variable to another, the filter the after-look's git runs sees neither, and sees the environment's other variables.
  - `sandbox::tests::the_harness_git_withholds_what_a_reload_sets_and_what_the_start_had`, `sandbox::tests::the_harness_git_is_never_one_inside_the_workspace`, `assemble::tests::the_workspace_is_avoided_with_or_without_a_sandbox`, and `assemble::tests::a_shell_inside_a_write_root_is_passed_over` as changed.
- `xtask`: `rules::rust_source::tests::every_process_start_is_in_the_table`, for the rule below.

**What is left untested:** that the core gives the probe the shell tool's removal list (`find_shell`'s `removed`). The core's tests give their sessions a shell, so none probes one. The probe's environment itself is tested in `cineris-tools`.

## The table, kept by a rule

[security.md](../../security.md), section 12, lists every place the product starts a process. `cargo xtask rules` now checks it (`process_starts`, beside the rule against a program started by its bare name): a product file that calls `Command::new`, `brief::run` or `CreateProcessW` outside its tests must be named in the table, and every file the table names must still start one. The test kit and `xtask` are left out, as the other rule leaves them.

## The flake `a-git-cap-of-zero-that-git-can-still-beat`

Looked at while in `git.rs`. The fixes do not change what it measures.

## The reviews

To come.

## The breaks

To come.
