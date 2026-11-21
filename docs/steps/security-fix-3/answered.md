# Answered in the third security fix, on 2026-11-21

A security fix outside the numbered steps. It settles four open items found by step 28's reviews and the fixes to its process starts, whose files it deletes: `a-filter-a-confined-program-plants-runs-unconfined-later`, `a-projects-write-root-holding-the-harness-files-turns-the-sandbox-off`, `the-harness-git-reads-the-repository-git-names-in-the-environment` and `a-command-started-confined-between-two-changes-of-level`. The design is in [security.md](../../security.md), sections 1, 11 and 12, and [06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md), "The sandbox".

## The design, before building

Written before any code, and sent to the maintainers to settle.

**Who controls what.**

| Who | Controls | Trusted |
|---|---|---|
| The user | The harness's environment, the user's configuration (`sandbox.write` among it), the level, the programs outside the workspace | Yes |
| The environment the harness was started in | Everything the harness inherits: `PATH`, `HOME`, the keys, and, inside a git hook or `rebase --exec`, git's own `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_CONFIG_PARAMETERS` | The user's, but the place variables describe another repository's, not the workspace's |
| A repository's files | `.git/config` and what it includes, `.git/config.worktree`, a submodule's `.git/modules/*/config`, `.gitattributes` and `.git/info/attributes`, hooks, `.cargo/config.toml` and the other protected names | Only once the user trusts the project, and only as they were then: a confined program may rewrite them afterwards |
| A project's configuration, once trusted | Its rules, `sandbox.write` (which widens), its profiles | What the trust question showed |
| The model, through confined commands | Anything inside the write roots: the workspace, the private temporary folder, `sandbox.write` | No |
| An extension's programs (`cineris.exec`) | Their manifest's write roots, at every level where the sandbox enforces | What the approval showed, and no more |
| An extension's script | What its host calls may write (`cineris.fs.write`, within the manifest's `write`), decided by the engine at the session's level | The same |

**Every place the harness runs git, or lets git run a program the repository names.**

| Where | What it runs | Configuration and environment | Confined today | What a repository can make it run |
|---|---|---|---|---|
| The change detector's looks, before and after a shell command, a check or an extension's program (`cineris_tools::shell::ChangeDetector`) | `status --porcelain=v2 … --untracked-files=all`, and `diff --name-status A B` when `HEAD` moved | System, global, the repository's `config` and `config.worktree` and their includes; a submodule's own when `status` recurses into it; attributes from `.gitattributes`, `info/attributes`, `core.attributesFile`. The harness's environment without the keys, `PATH` absolute and outside the avoided roots, `GIT_ENV`, every `GIT_*` as it came | While the level confines, read at each look (item 4); after an extension's program, at every level (`FollowingTheProgram`) | A clean or process filter, the superproject's or a submodule's, on any file whose times no longer match the index. Turned off: the file-system monitor, hooks, optional locks, lazy fetch |
| The base capture, beside the session's first turn (`cineris_session::base`) | `rev-parse`, `symbolic-ref`, `diff HEAD` and `diff --cached` as patches, `diff --name-only`, `ls-files --others` | The same | While the level confines, read when it runs | Filters, as above. Text converters and external diffs are off (`--no-textconv`, `--no-ext-diff`) |
| The checkpoints, for a file's earlier content (`Rounds::earlier_content`) | `cat-file blob COMMIT:./PATH` | The same | As above | Nothing: `cat-file blob` runs no filter |
| The project's checks (`verify`) | A check is a shell command through the session's shell: its git is the command's | A command's environment | As a command | What a command's git runs, confined while the command is |
| The session's own git, run by the model (`git status`, `git commit`, `git diff`) | Whatever the model writes | A command's environment, with `core.fsmonitor=false` added | While the level confines; at `unrestricted` as the user | Filters, hooks (`git commit`), text converters, the pager, aliases, `core.sshCommand` and credential helpers on the network commands |

The commands the harness runs touch neither the network nor the pager, and run no alias, no editor, no signature check (`log`, `show` are not among them) and no hook (`core.hooksPath` is nowhere). **What is left is filters**, and through `core.worktree`, `commondir` and includes, configuration read from elsewhere, which is more configuration and no program of its own.

**Item 1: options 1 and 3, the third done by the sandbox rather than the command line.**

- **3, the harness's own git: inside the sandbox wherever it enforces, at every level.** Today it is confined while commands are, and after an extension's program. It becomes confined wherever the session's sandbox enforces, `unrestricted` included: one rule, with `FollowingTheProgram` folded into it. A program any configuration names, a submodule's included, runs with no write root and no network, whoever wrote it and whatever the level. It also makes the detector's git independent of the level, which removes half of item 4.
  - *Why not overrides on the command line* (`-c filter.<name>.clean=` for each driver): they need the driver's names first, from `git config`, which is one more git process before every look. That is two more for each shell command, at about 20 ms each on a Windows runner. They also miss a submodule's own configuration, which `status` reads in a git of its own, and they race with a writer between the listing and the use. Confining covers every program a configuration can name, including any not foreseen, for the launcher's 0.8 ms.
  - *Its cost:* a filter that must write or reach the network (git-lfs) fails at `unrestricted` too, and the list of what a command changed says it is incomplete, with the reason. It already does so below `unrestricted`.
  - Where the sandbox does not enforce, git runs as it is, as now. There nothing a command or a program does runs with fewer rights than that git, except an extension's script (below).
- **1, put a protected change back at once.** After the look that follows a confined start (a shell command, a check, or an extension's program with a write root), each protected path the start changed is put back from the look before's own record, which is also what the round's checkpoint records:
  - a path that appeared is removed (a link is removed, never followed);
  - a file that changed or went is written back;
  - a directory that went is made again;
  - nothing is written through a directory on the way that is a link now.

  Then the paths are looked at again. What still differs is named as something to put back by hand, as today, and the turn stops as it does now. The result tells the model and the user what was put back. This closes what option 3 leaves:
  - the model's own `git status` at `unrestricted`, and every later unconfined run of anything protected: `git commit`'s hooks, `cargo build`'s `.cargo/config.toml`, `.envrc`;
  - the user's own tools after the session.
- **Not 2.** Refusing write roots that cover the repository's git folder refuses `write = ["**"]`, the common manifest. The shell's confined commands have the workspace as a root, so it cannot cover them at all.
- **What 1 changes for the user (for you to settle):** a protected change a confined command makes is never kept. That holds even when the line named the path and the user approved it (`git config user.email …`, `cp hook .git/hooks/pre-commit`), and even when it was a side effect of ordinary work (`git remote add`, `git init sub`, `git checkout --track`). Today each of these already stops the turn. Under 1 it is put back as well. The note tells the model that a protected path is changed only outside the sandbox: a run with `sandbox: false`, which asks the user ("allow once"), or the user's own hand. **My recommendation is to accept that cost.** The alternative, keeping a change whose path the decision named, needs the engine's findings carried to the look. It would also keep a filter written beside an approved `git config` on the same line.

**Item 2.** Leave out a `sandbox.write` entry that holds the harness's files, with a notice naming it under `~`, and keep enforcing. `NotEnforcing(HarnessFilesWritable)` stays only for the workspace and its temporary folder, which cannot be left out.

- **The user's own entry is left out too** (recommended). A `sandbox.write` entry means "let commands write there", not "turn the sandbox off". Leaving it out keeps every other root confined, and the notice says what to name instead (`~/.cargo/registry`, not `~/.cargo`). Keeping today's behaviour for the user would leave two rules for one setting.
- **A reload** that adds such an entry leaves it out with the same notice, in place of refusing the whole reload.
- **The trust question** and `cineris permissions trust` say that the entry "is left out, and the sandbox stays on", not that the project's sessions run without it.
- **An extension's write root** that holds the harness's files is still refused.
- Judged only where the sandbox would enforce, so Windows is as before: nothing is left out and nothing is said.
- A new decision supersedes decision 0032's paragraph "A write root that holds the harness's own files is not confined".

**Item 3.** `cineris_base::git::start` takes out of git's environment what `git rev-parse --local-env-vars` lists:

- the list is git's `local_repo_env`, written in the code;
- a test compares it with the installed git's answer, so a git that adds one fails CI;
- asking git at every start would cost a process per test on Windows.

The list includes `GIT_CONFIG_PARAMETERS` and `GIT_CONFIG_COUNT`, as git's own does for a submodule: a harness started by `git -c x=y …` loses those for its own git. `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_NOSYSTEM`, `HOME` and `XDG_CONFIG_HOME` pass as they came. Shell commands' environments are not changed.

**Item 4.** A command's confinement is decided once, when the protected-path look before it is taken (`RecordingShell::run_by`, which the project's checks run through too):

- the session's launcher as it is then, or none;
- the look before and the look after;
- the start (`ShellTool` given that launcher for the one call);
- what the observer is told, the escape check and the forgetting of the last look.

All of them follow that one value. The detector's git no longer depends on the level (3 above). A level changed after the decision applies from the next command. An extension's program already decides once: its launcher confines it at every level.

**Found by the grep for the mechanism.**

- **An extension's script's `cineris.fs.write` of a protected path** is asked about below `unrestricted`. At `unrestricted` it is not asked about, and nothing looks at it afterwards:
  - where the sandbox does not enforce (Windows), the harness's git then runs a filter the script wrote, with the user's rights;
  - where it enforces, 3 confines the harness's git, but the model's own `git status` at `unrestricted` still runs it.

  This is the extension host's code, step 28's, so it is not this step's. The maintainers passed it to step 28, which refuses such a write in the extension host at every level, as `cineris.exec` confines at every level, with no change to the engine (step 28's records).
- Every other start was checked: the shell's version probe, the external editor, the checkpoints' `cat-file`, the base capture, the extension's launcher, the tree's guard. None runs a program a repository names, or runs after a confined start at the level of the moment, beyond what is above.
- **A repository whose configuration already names an unprotected folder for its hooks** (`core.hooksPath = .githooks`, `scripts/hooks`), found by step 28's review of protected writes while this fix was built: a confined command could write a hook there unseen, since neither the folder nor `.git/config` is on the protected list, and the model's own `git commit` at `unrestricted` would run it. The maintainers asked that it be covered here; it is, below ("The folder `core.hooksPath` names"). The open item that named it, [files-git-runs-that-the-protected-list-does-not-name](../../outstanding/files-git-runs-that-the-protected-list-does-not-name.md), is narrowed to what remains.

**What is left open on purpose.**

- **A change the look does not see:**
  - past its bound (20,000 directories, a quarter of a second);
  - a file git's configuration names that the list does not (an `include.path` file or a `core.hooksPath` folder inside the workspace, [files-git-runs-that-the-protected-list-does-not-name](../../outstanding/files-git-runs-that-the-protected-list-does-not-name.md)).

  Nothing puts these back. The harness's git runs them confined, but the model's commands at `unrestricted`, and the user's own tools, can run them.
- **A change between two commands** (the continuity check: a process left running, or the user's own hand) stops the turn and is not put back, because it may be the user's.
- **What cannot be put back** is named as something to put back by hand: an earlier content not kept (a link, a special file, a file over 256 KiB), or a directory on the way that is a link now.
- **Where the sandbox does not enforce,** the harness's git runs a trusted repository's filters as the user's own git would.

## Settled

The note was settled on 2026-11-21, before any product code.

- **By the owner:**
  - *Item 1's option 1:* a protected change a confined start makes is always put back, an approved one included. The model makes such a change outside the sandbox (`sandbox: false`, which the user allows once), or leaves it to the user; the model-facing text where the stop is explained says so.
  - *Item 2:* the user's own `sandbox.write` entry is left out too, under the one rule, with a notice that says what to name instead.
- **By the maintainers:**
  - *Option 3, as the sandbox:* the harness's git runs in the reading sandbox wherever the session's sandbox enforces, at every level, with `FollowingTheProgram` folded in. The git-lfs cost (an incomplete change list at `unrestricted`) is accepted.
  - *The put-back's rules,* as listed above: never follow a link, never write through a directory that is a link now, look again, and name what is left as one to put back by hand. Tested with links planted at each step of the path.
  - *The reload,* the trust question's words and `permissions trust`'s, an extension's write roots still refused, Windows unchanged, and a new decision ([decision 0042](../../decisions/0042-a-sandbox-write-entry-that-holds-the-harness-s-own-files-is-left-out-and-the-sandbox-stays-on.md)).
  - *Item 3's list* written in the code, with a test against the installed git's `--local-env-vars` that names what is missing or extra and is skipped, saying so, where git is too old to list them.
  - *Item 4* decided once in `RecordingShell::run_by`, both orders tested with test points.
  - *The folder `core.hooksPath` names,* watched in this step (the request above).

## What was built

- **The put-back** (`cineris-sandbox`, `protect::Watch::put_back`, in `protect/restore.rs`). After the look that follows a confined start, each changed protected path is made as the look before found it:
  - each directory a path lies in is opened with `openat2(RESOLVE_NO_SYMLINKS)`, and every change is made relative to that open directory, so that a link swapped in on the way after the check carries nothing elsewhere;
  - what was not there before is removed, a link itself and never what it leads to, a directory only when it is empty;
  - a file whose content was kept is written under a name of its own beside it, made with no link followed, and renamed into place, with its mode, so a link at its name is replaced and never written through;
  - a directory that went is made again with its mode; a file whose content was not kept has its mode put back and nothing else.

  Then the protected paths are looked at again. What still differs from the look before is `PutBack::left`, named in the result as one to put back by hand; a directory made again with its mode is as it was, though its inode is new. The core's `sandbox::protected_changes` does the look, the put-back and the look again for a confined shell command, a project's check (both through `RecordingShell::run_by`) and an extension's program with a write root (`ext::exec::look_after`). The look kept for the next confined start is the one taken after the put-back. What is left is recorded in the round's checkpoint, for undo where it can.
- **The note** (`sandbox::protected_note`): the changed paths, what was put back and what is left, and, for the model: "Nothing run inside the sandbox keeps a change to a protected path. If the user wants this change, make it with a command outside the sandbox (`sandbox: false`), which the user is asked to allow once, or leave it to the user."
- **The harness's git at every level** (`SessionGit::confines` and `SessionSandbox::start_git`): the reading launcher wherever the sandbox enforces; `FollowingTheProgram` and `start_git_confined` are gone, and an extension's program's look uses the session's git.
- **One decision a command** (`SessionSandbox::decide`, `Decided`): the launcher as it is when the look before is taken, or none, given to the shell tool for the one call (`ShellTool::run_through`, new: the tool's `Tool::run` passes its own launcher). The protected look before and after, the put-back, the command observer's flag, the escape's stop and the forgetting of the last look after an unconfined command all follow it. `SessionSandbox`'s own `Launcher` implementation decides at each start, for a caller that does not decide first.
- **Git's place variables** (`cineris_base::git::LOCAL_ENV_VARS`): the fifteen names git 2.53 lists, taken out of the harness's git's environment in `git::start`, after `child_env::for_harness`.
- **A `sandbox.write` entry that holds the harness's files** (`assemble::left_out_writes`, `left_out_notice`, `HarnessFilesAt`): judged on its own where the sandbox would enforce, left out of `sandbox_grant` (which takes the state directory now), and told at the session's start through `engine_parts`'s notices, and at a reload, of each entry the reload adds, through `Hooks::tell`; the reload is applied. The trust question's warning and `permissions trust` say the entry is left out. `NoSandbox::HarnessFilesWritable` names the workspace and its temporary folder.
- **The folder `core.hooksPath` names** (`cineris_tools::shell::changes::hooks_folder`, `SessionSandbox::follow_hooks`, `Watch::with_more`): read with the session's git (`git config --type=path --null --show-scope --get-all core.hooksPath`, the harness's own `command`-scope `/dev/null` passed over; a relative value from the working tree's top) before each confined start's look and again before the look after it, and watched with the protected paths where it lies inside a write root. A folder once watched stays watched for the session: a look that dropped one would take what is in it for gone.

**The points a test can stop at,** each an awaited point (`cineris_base::hook::await_point!`) on the turn's task, exported from `cineris_core`:

- `BEFORE_THE_COMMAND_STARTS`: a command's confinement is decided and its look before taken, and it has not started.
- `BEFORE_THE_LOOK_AFTER`: a command has run, and the look at protected paths after it is not taken.
- `AFTER_THE_TURN_IS_STOPPED`: a command has stopped its turn, and its result has not gone back to the round. Asked for by the maintainers for the step that makes a stop end the turn; reached only by a command that stopped its turn.

And in `cineris-sandbox`, `protect::PUTTING_BACK` (a blocking point): a path's directory is open and the path is not put back yet.

## Tests

- `cineris-base`
  - `git::tests::sys_git_answers_about_its_own_repository_whatever_the_environment_names`: `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE` and `GIT_CONFIG_PARAMETERS` naming another repository; the harness's git answers about its own, with its own index, and still reads `GIT_CONFIG_GLOBAL`.
  - `git::tests::sys_the_local_variables_are_the_ones_git_lists`: against the installed git's `rev-parse --local-env-vars`, naming what is missing and what is extra.
- `cineris-sandbox`, `protect::tests::putting_back`: what a start changed is made as it was (content, mode, a mode alone, a created directory); a file and a directory that went are made again; a link at a file, at a directory on the way and where nothing was is removed and never followed; a path whose directory above is a link now is left alone; a directory, and then a file, swapped for a link at `PUTTING_BACK`, while the path is put back.
- `cineris-tools`: `shell::changes::tests::the_hooks_folder_is_the_last_one_a_configuration_names`.
- `cineris-core`
  - Item 1, end to end: `harness_git::sys_a_filter_a_confined_command_plants_never_runs_afterwards` (a confined command plants a filter; the next turn at `unrestricted` runs a command and the model's own `git status`; the filter never runs) and `ext_exec::confined::sys_a_filter_a_program_plants_never_runs_afterwards` (the same through `cineris.exec`, at `unrestricted`).
  - The put-back in the core: `sandbox::sys_a_protected_path_a_command_changes_is_put_back_and_stops_the_turn`, `sandbox::sys_a_link_a_command_planted_is_put_back_and_nothing_is_written_through_it`, `sandbox::sys_after_a_put_back_the_next_command_runs`; and, changed to the new truth, `adversary::sys_an_adversarial_command_reaches_only_what_it_may`, `sandbox::sys_a_stopped_turn_runs_no_more_commands_of_the_same_response`, `ext_exec::confined::sys_a_program_stopped_at_its_time_is_still_looked_at` and `…_the_call_cancels_…`.
  - The git rule: `harness_git::sys_at_unrestricted_the_harness_git_still_runs_inside_the_sandbox` (a filter in an included file, which nothing puts back, still runs confined at `unrestricted`), `harness_git::sys_the_base_state_is_taken_by_a_git_inside_the_sandbox` (now at `unrestricted` too), `sandbox::tests::the_harness_git_runs_inside_the_sandbox_reading_only`; the controls moved to where the sandbox does not enforce (`harness_git::sys_where_the_sandbox_does_not_enforce_the_after_look_git_runs_the_planted_filter`), and `harness_git::sys_a_folder_a_reload_makes_writable_is_kept_out_of_the_harness_gits_path` tells which filter ran by git's own answer.
  - Item 4: `sandbox::sys_a_command_decided_as_it_is_runs_as_it_is_when_the_level_is_lowered_then_raised` (the item's order) and `sandbox::sys_a_command_decided_confined_runs_confined_when_the_level_is_raised_then_lowered`, each changing the level twice at `BEFORE_THE_COMMAND_STARTS` and `BEFORE_THE_LOOK_AFTER`.
  - Item 2: `sandbox::a_write_entry_that_holds_the_harness_files_is_left_out_and_the_sandbox_stays_on` (the user's `~`, `~/.cargo` with the launcher, an entry inside the configuration directory, a trusted project's `~/.cargo`, and on Unix a dotfiles folder the configuration file links into), `sandbox::sys_a_write_entry_left_out_grants_nothing_and_the_rest_is_granted`, `sandbox::a_reload_that_adds_a_write_entry_holding_the_harness_files_leaves_it_out`, `sandbox::where_the_sandbox_does_not_enforce_no_write_entry_is_left_out`; step 26's `sandbox::a_workspace_that_holds_the_harness_files_is_not_confined` now holds the launcher in the workspace, what is still true; and step 28's `extensions_review::the_trust_question_judges_the_sandbox_by_the_load_a_yes_puts_in_force` has the new words.
  - The hooks folder: `harness_git::sys_a_hook_a_confined_command_writes_in_the_hooks_folder_is_put_back` (a hook changed and one planted in `scripts/hooks`; a folder named in the same command through an included file) and `ext_exec::confined::sys_a_hook_a_program_plants_in_the_hooks_folder_is_put_back`.

## Measured

- **The harness's git through the launcher at `unrestricted`:** 0.78 ms a start, the launcher's cost measured in step 26 on the reference machine (release build, 200 runs, nothing else running; [performance-budgets.md](../../performance-budgets.md)). Two or three starts a command, as below `unrestricted`.
- **A read of the hooks folder:** 0.73 ms with git started as it is (median of 200, p90 0.83 ms, minimum 0.67 ms), release build, on the second machine with other work beside it; a throwaway `cargo xtask scratch-test cineris-tools <file> --cargo-profile release --ignored` timing `hooks_folder`. With the launcher, about 1.5 ms a read, two a confined command in a repository: about 3 ms beside the looks' 47 ms. Not read outside a repository, nor where the sandbox does not enforce (Windows).
- **The put-back:** one more look (23.6 ms in this repository, step 26's figure), only when a confined start changed a protected path.
