# Answered in the third security fix, on 2026-11-21

A security fix outside the numbered steps. It settles four open items found by step 28's reviews and the fixes to its process starts: [a-filter-a-confined-program-plants-runs-unconfined-later](../../outstanding/a-filter-a-confined-program-plants-runs-unconfined-later.md), [a-projects-write-root-holding-the-harness-files-turns-the-sandbox-off](../../outstanding/a-projects-write-root-holding-the-harness-files-turns-the-sandbox-off.md), [the-harness-git-reads-the-repository-git-names-in-the-environment](../../outstanding/the-harness-git-reads-the-repository-git-names-in-the-environment.md) and [a-command-started-confined-between-two-changes-of-level](../../outstanding/a-command-started-confined-between-two-changes-of-level.md). The design is in [security.md](../../security.md), sections 1, 11 and 12, and [06-permissions-and-sandbox.md](../../architecture/06-permissions-and-sandbox.md), "The sandbox".

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

  This changes the engine's levels, so it is not this step's. I recommend asking at every level about a script's write of a protected path, as `cineris.exec` confines at every level, and opening an item for it.
- Every other start was checked: the shell's version probe, the external editor, the checkpoints' `cat-file`, the base capture, the extension's launcher, the tree's guard. None runs a program a repository names, or runs after a confined start at the level of the moment, beyond what is above.

**What is left open on purpose.**

- **A change the look does not see:**
  - past its bound (20,000 directories, a quarter of a second);
  - a file git's configuration names that the list does not (an `include.path` file or a `core.hooksPath` folder inside the workspace, [files-git-runs-that-the-protected-list-does-not-name](../../outstanding/files-git-runs-that-the-protected-list-does-not-name.md)).

  Nothing puts these back. The harness's git runs them confined, but the model's commands at `unrestricted`, and the user's own tools, can run them.
- **A change between two commands** (the continuity check: a process left running, or the user's own hand) stops the turn and is not put back, because it may be the user's.
- **What cannot be put back** is named as something to put back by hand: an earlier content not kept (a link, a special file, a file over 256 KiB), or a directory on the way that is a link now.
- **Where the sandbox does not enforce,** the harness's git runs a trusted repository's filters as the user's own git would.
