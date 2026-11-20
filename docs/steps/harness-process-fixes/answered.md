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
  - `child_env::tests`: the keys and the rest; the `PATH` rule; Windows' variable and `Path`; a directory of `PATH` inside an avoided root left out (`a_harness_program_does_not_search_the_workspace`); the shared list.
  - `program::tests::a_link_into_or_out_of_an_avoided_root_is_passed_over` (Linux and Windows, whose runners can make links): a link in a root that leads out, one outside that leads in, a chain through a root, a directory of `PATH` that is a link into one; and `a_link_that_leads_to_itself_is_inside`.
- `cineris-tools`
  - `shell_search::sys_the_probe_is_given_no_key_variable` (Linux, a fake `bash`; Windows, a fake `pwsh.bat`): the probe sees no key, no variable of the test process, and of `PATH` only the absolute entry outside the workspace; the key not withheld, it sees it.
  - `shell_search::a_shell_in_an_avoided_directory_is_not_chosen` (both systems' rules): a `bash` or `pwsh.exe` in an avoided `.venv/bin` is passed over, by the default search and by a configured name.
- `cineris-core`
  - `harness_git::sys_a_filter_the_harness_git_runs_is_not_found_in_the_workspace`: with the sandbox off, a filter named by a bare name is in an activated `.venv/bin` and in a directory outside further down `PATH`; the outside one runs.
  - `shell::sys_the_shell_a_session_finds_is_not_in_the_workspace_and_its_probe_has_no_key` (Unix): what the core hands the shell's search and probe, end to end. With no sandbox, the session finds its own shell on the `PATH` of the environment its processes inherit: a `bash` planted first, in the workspace's `.venv/bin`, is never started; the one found, outside, is asked its version with no key and no workspace directory on its `PATH`. Asked for by the maintainers before the merge, so that a change to the core cannot drop the hand-over with every piece's test still passing. For it the core's search reads the options' environment for commands when one is given, as git and the probe already did (`ShellSearch::from_vars`).
  - `shell::sys_a_reload_withholds_a_variable_a_new_header_reads`: a reload that gives a provider a header read from a variable withholds it from the next command.
  - `harness_git::sys_a_reload_withholds_the_keys_from_a_filter_the_harness_git_runs`: a session at `unrestricted` whose command plants a filter; after a reload moves the provider's key from one variable to another, the filter the after-look's git runs sees neither, and sees the environment's other variables.
  - `sandbox::tests::the_harness_git_withholds_what_a_reload_sets_and_what_the_start_had`, `sandbox::tests::the_harness_git_is_never_one_inside_the_workspace`, `assemble::tests::the_workspace_is_avoided_with_or_without_a_sandbox`, `assemble::tests::the_write_roots_kept_out_of_are_placed_as_the_grant_places_them`, and `assemble::tests::a_shell_inside_a_write_root_is_passed_over` as changed.
- `xtask`: `rules::rust_source::tests::every_process_start_is_in_the_table`, for the rule below.

**What is tested on Linux only:** the core's hand-over to the shell's search and probe, end to end (the test above plants scripts). The pieces it hands them to run on both systems, and the code that hands them over is the same on both.

## The table, kept by a rule

[security.md](../../security.md), section 12, lists every place the product starts a process. `cargo xtask rules` now checks it (`process_starts`, beside the rule against a program started by its bare name): a product file that calls `Command::new`, `brief::run` or `CreateProcessW` outside its tests must be named in the table, and every file the table names must still start one. The test kit and `xtask` are left out, as the other rule leaves them.

## The rebases onto main

- **Onto `d0486fd3`** (step tooling 12's first part), by hand: two conflicts. In `assemble.rs`, main had added a test hook before the probe that runs inline; it was kept, beside the call of this fix's `probe`. In `xtask/src/rules/mod.rs`, main's `pty_keys` rule and this fix's `process_starts` were both kept. `git::start`'s new test hooks merged without a conflict and stay after the environment is built.
- **Onto `d6f07ee9`** (its second part, which brings `cargo xtask rebase`): the branch did not hold the command yet, and it rebases the tree its binary was built from, so the rebase was git's own, with no conflict. Then `cargo xtask rebase` from the rebased branch said:

```
Rebased worktree-step-security-fix-2 onto origin/main with no conflict.
docs/outstanding/README.md is up to date.
The tip before was 796287f8e110: `git reset --keep 796287f8e110` takes the branch back to it.
```

## The flake `a-git-cap-of-zero-that-git-can-still-beat`

Looked at while in `git.rs`. The fixes do not change what it measures.

## The reviews

### The early review of the builder and the search (Opus, 2026-11-19)

The environment builder and the shell's search, read on their own by a reviewer that had not seen the reasoning, as soon as they existed. It agreed that calls A to C only narrow, once its first two findings were fixed.

| Finding | Fix |
|---|---|
| **High, confirmed:** `confine_roots` read each `sandbox.write` entry as written: `~/.cargo` was a relative name that no path was ever inside, and a relative entry was not under the workspace. A `bash` a confined command wrote in `~/.cargo/bin`, which rustup puts first on `PATH`, would become the next session's shell and its unconfined probe. | One helper, `placed_entry`, places an entry for every reader: the sandbox's grant, the roots a front end is told of, and the roots the harness's programs are kept out of. `confine_roots` feeds nothing that grants: only what is avoided, and the refusal of a configured shell inside a write root. Test `assemble::tests::the_write_roots_kept_out_of_are_placed_as_the_grant_places_them`. |
| **High, confirmed:** `find_outside` looked only at where a candidate's last link led. `.venv/bin/bash`, a link to `/usr/bin/bash`, was taken by the link's path, and a confined command could then replace the link with a script. | A candidate is passed over when any place on its way is inside a root: a directory, the place it is found at, each link, and each link's target (`program::passes_through`). The core's own check (a configured shell, the cross-session cache) uses it too. Test `program::tests::a_link_into_or_out_of_an_avoided_root_is_passed_over`, on both systems, with a link in a root that leads out, one outside that leads in, a chain that passes through a root, and a directory of `PATH` that is a link into one. |
| Medium, plausible: git's `PATH` kept an absolute entry inside the workspace, so a filter the user's own configuration names by a bare name (`git-lfs`) could be a file in `.venv/bin`. | The harness's `PATH` leaves out the directories its confinement avoids (`child_env::for_harness`, `Confinement::avoid`). The commands' `PATH` keeps them. Tests `child_env::tests::a_harness_program_does_not_search_the_workspace` and `harness_git::sys_a_filter_the_harness_git_runs_is_not_found_in_the_workspace`, whose filter is found further down `PATH`, outside, as a positive check. |
| Medium, confirmed: a reload that gave a provider a header read from a variable did not look at the key variables again (only a change of `key_env` did), so that variable reached commands and git until the next session. | Any change under `providers` recomputes them. Test `shell::sys_a_reload_withholds_a_variable_a_new_header_reads`. |
| Low: a reload of `sandbox.write` leaves git's avoided roots as they were. | Settled in step 28: the harness's git avoids the write roots the sandbox holds now, a reload's included ([28/parts/pr1a-engine.md](../28/parts/pr1a-engine.md)). |
| Low: a variable one reload made a key and a later one did not is passed again. The shell's commands behave the same. | Open: [a-key-a-reload-added-is-passed-again-after-another-reload.md](../../outstanding/a-key-a-reload-added-is-passed-again-after-another-reload.md). |
| Info: the private temporary directory is not among the avoided roots; a shell or git found only inside the workspace is no longer found; a workspace of `~` puts the user's own tools inside it. | The temporary directory is the session's own and on no `PATH`. The other two are what calls A and B decided. |

**Why the first two were not found before.** Both came in with step 26's third review, whose independent review tested `find_outside` and `confine_roots` with roots that were already absolute and resolved, and with planted programs that were plain files. Neither test had a `~` or relative entry, or a link inside a root. **The shape to brief a review on:** a path check whose test inputs are already canonical, and which compares only the final target. Such a review tries inputs that are not in canonical form, and links in both directions.

### The review of the whole change (Opus, 2026-11-19)

The whole diff, against this record's design and [security.md](../../security.md), section 12, by a reviewer that had not seen the reasoning. **It found no hole:** each of the early review's four fixes is real; no key reaches git, a filter or the probe; no program reached through the workspace or a write root is chosen; the shell's commands' environment and the sandbox's grant are as they were; every new test fails with its fix reverted and has a check that the program it watches ran.

| Finding | Fix |
|---|---|
| **Medium, confirmed:** the break `a-link-only-followed-to-its-end` could not bite: the root's own directory is a place on the way before any link inside it. | Dropped. The rule is held by `a-link-in-a-root-that-leads-out-taken`. |
| Low, confirmed: calls A and B at the core's call sites were pinned only by unit tests. The core's tests run where the sandbox enforces, and there the write roots hold the workspace already, so handing git the write roots in place of the avoided roots passed. | `harness_git::sys_a_filter_the_harness_git_runs_is_not_found_in_the_workspace` turns the sandbox off; the break `git-given-only-the-write-roots-to-avoid`. The shell's call site, at first left untested, is pinned by `shell::sys_the_shell_a_session_finds_is_not_in_the_workspace_and_its_probe_has_no_key` and three breaks. |
| Low, confirmed: a `PATH` that is not Unicode, every directory of which is avoided, was left empty on Unix, which a shell reads as the directory it runs in. | It is `/usr/bin:/bin` there too, as in the Unicode case. |
| Low, plausible: the probe's `PATH` kept the workspace's directories, where a shell that is a wrapper (a version manager's shim) looks up its real program, unconfined. | The probe's `PATH` leaves out the avoided roots too (`child_env::keep_out_of`); the probe's test has an avoided `.venv/bin` in it, and the break `probe-searches-the-workspace`. |
| Low, docs: section 12 said the shell is never a program inside the workspace, where a configured absolute `shell.program`, `%ComSpec%` and the shells under `%SystemRoot%` are taken as named. | Said so. |
| Low, latent: the rule that keeps the table would miss `.map(Command::new)`, `CreateProcessAsUserW`, a start under another name, one in a macro. | The first two are seen now (the rule's test has them); the others are said in the rule's documentation: the table is the look, and the rule keeps it from going stale. A test `cfg` on a field taken to cover the next item is the rule's older scan of test modules, shared with the rule against bare names, and is left as it is. |
| Info: this record's list of tests left out five of them. | Listed. |
| Info: an empty Windows `Path` reaching Git for Windows' `sh`; a `PATH` entry under `/proc/self/cwd`; DLL search from the workspace; git looked up at every start. | An empty `Path` cannot reach git: git is found on an absolute entry outside the avoided roots, which stays. The others are noted, not changed: `/proc/self/cwd` in a user's `PATH` is the user's own; the DLL search is Windows' own for every program the user starts; a look-up costs a few metadata calls beside git's own start. |

### The narrow check of the review's fixes (Opus, 2026-11-19)

The same reviewer, on the commit that fixed its findings. Each is fixed.

| Finding | Fix |
|---|---|
| **Medium, confirmed:** `expect: survives` on `step-13a-fixes/harness-starts-git-by-name` hid a break the core's tests see: where commands are confined, the program `git::start` starts is the launcher, and a bare `git` in its place runs unconfined. | The break names the core's confined test that catches it, and runs there. |
| Info: the rule of `step-13a-fixes/program-search-looks-in-relative-entries` still said the shell's search was `find`. | Said which search it is. |

**The window between the look and the start.** The shell is found once a session, and git at each start. Nothing on the way to the program chosen can be written by a confined command, so a swap between the look and the start needs a writer the look has already ruled out. A command that is not confined can write anything the user can, which section 12 of [security.md](../../security.md) leaves open on purpose.

## The breaks

Run with `cargo xtask mutate` on the reference machine, beside other work: verdicts, not timings.

| What | Breaks | Verdict |
|---|---|---|
| `mutations/harness-process-fixes.breaks` | 24: git's environment cleared, built, told the withheld list, the reload's and the start's keys, the roots avoided, the call site's roots (6); its `PATH` without relative entries, without avoided ones, and told the avoided roots; Windows' variable; the known keys (5); the probe's environment cleared, built, and its `PATH` (3); the search's relative entries, a shell in a root, a configured one in a write root, a link in a root that leads out, a git in the workspace, the workspace avoided without a sandbox (6); a reload's look at any provider (1); the core's hand-over to the shell's search of the roots to avoid, and to its probe of the removal list and the roots (3) | 24 caught |
| Moved, in the shell environment's fixes' lists (11), `step-13a.breaks` (3), `step-13a-fixes.breaks` (1), `step-25-pathfix.breaks` (4) and `step-26-fixes.breaks` (3) | The code they break moved into `cineris_base::child_env` and `program`; each names the tests of `cineris-base` that catch it there too | Every one caught by the tests it names |
| `step-13a-fixes/program-search-looks-in-relative-entries` | The shell's search is `find_outside` now, so the shell's test no longer reaches `find`: the line is taken out, and `program-search-outside-looks-in-relative-entries` breaks the same rule in `find_outside` | Caught by `cineris-base`'s test |
| `step-13a-fixes/harness-starts-git-by-name` | Git's `PATH` holds no relative entry now, so in the session store's test a bare `git` finds what the search found. In a session that confines, though, the program started is the sandbox's launcher, and a bare `git` in its place runs outside the sandbox | Caught by the core's `harness_git::sys_a_filter_a_confined_command_plants_runs_inside_the_sandbox`, which it names now (the narrow check found that `expect: survives` was wrong) |
| Every other break `mutate --changed` chose for the files this fix touched (200 in all) | | As their lists expect, but five below |

**Five breaks that do not bite on `main` either**, found by `mutate --changed` because they break code in `assemble.rs`, and run again on a copy of `main` as it was (`6c43dfb1`) with the same verdicts. This fix does not touch the code they break, so they are reported, not changed:
- `step-13a/protected-directory-unresolved` and `step-13a/configuration-directory-not-closed`: not caught by `start::a_configuration_directory_behind_a_link_is_protected_where_it_really_is`. Something else closes the configuration directory too.
- `step-13a/repository-trust-not-given-to-the-shell` and `step-13a/shell-given-rules-trust`: they break `ShellConfig::repository_trusted`, which `ShellLive`'s trust overrides since step 24.
- `step-13a/engine-told-every-projects-rules-are-approved`: expected to survive, and caught.

**Stress:** `cargo xtask stress -p cineris-base -p cineris-tools -p cineris-core --count 50` on the reference machine: 1,226 tests, 50 times each, seeds 0 to 49; every test passed every time.
