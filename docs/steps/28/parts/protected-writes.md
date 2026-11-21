# Step 28, its parts: an extension's writes of protected paths

A fix of the first part's host functions ([pr1a-engine.md](pr1a-engine.md)), found by another session's grep and settled by the maintainers: an extension's script may not write a protected path, at any level.

## What was found

`cineris.fs.write`, the one host call that writes a file, was decided by the permission engine as the `write` tool's write. The engine asks about a write of a protected path at every level short of `unrestricted` and allows it unasked there ([06-permissions-and-sandbox.md](../../../architecture/06-permissions-and-sandbox.md), "Protected paths"), and nothing looked at the write afterwards, as the look at the protected paths does after a command. So a script at `unrestricted` could write `.git/config`, a hook in `.git/hooks` or `.cargo/config.toml`, and then:

- where the sandbox does not enforce (Windows today), the harness's own git ran a filter or a hook the script wrote, with the user's rights;
- where it does, the model's own `git status` at `unrestricted` ran it.

No other host call a script chooses writes a file: `cineris.exec`'s programs are confined at every level where the sandbox enforces, and looked at after; `cineris.state` writes the extension's own values in the state directory. No host call appends, renames or removes, so there is no rename from a protected path to judge. `fs.write` does make the folders a new file needs (the `write` tool's `make_parents`), so a write below a protected folder that is not there yet makes it: such a write is refused as a write of the folder, and so is one that would make `objects` or `refs` beside a `HEAD` (below).

## The fix

**The core's host refuses the write before the engine is asked, or the `write` tool planned, at every level, `unrestricted` included, on every system** (`cineris-core`'s `ext::protected`, called first in the host's `write`). It changes none of the engine's rules: it is the extension host's own check, as the manifest's is. No rule and no answer lifts it.

- **What is protected** is the table of protected paths, which the look after a command watches too:
  - the lease's protected paths (the names of `PROTECTED_IN_WORKSPACE` under the workspace, and `sandbox.protect`), each with everything under it, a protected folder itself included;
  - at any depth, the workspace's own folders included (a workspace in a `.git` folder, or that is a `.cargo`): everything in a folder named `.git`, a file named `.git` or `HEAD`, `.cargo/config.toml`, `.cargo/config`, `.npmrc`, `.yarnrc`, `.yarnrc.yml` and `.envrc`, their names compared without regard to case on every system (protecting too much is the safe side, as the engine judges them);
  - everything in a folder that git takes for a repository, whatever its name, as the look finds one: one that holds `HEAD`, `objects` and `refs`, below the workspace, the workspace itself, or one above it when the workspace is in one of its own folders (`hooks`, `info`, `objects`, `refs`, `logs`, `modules`, `worktrees`), not a worktree beside them (`git clone --bare URL repo; cd repo; git worktree add main` writes in `repo/main` as anywhere, and its `.git` file is refused by name); and, so that a repository is not put together one write at a time, a write that would make `objects` or `refs` beside a `HEAD`.

  The list is read where it is, from `cineris-policy` and the session's lease; nothing of the shell's look was moved. **The table's "Files that look like secrets" row is not refused, on purpose:** a write of one is still asked about by the engine short of `unrestricted`, as it was, since it leaks a secret rather than running code later.
- **Judged by place.** The target is where the session's path resolver led the script's path (`..`, every link followed, the spelling on disk), the same `ResolvedPath` the manifest judged, and it is compared with each protected path by `location_within`, so the extended-length form, `/` against `\` and another case are one place on Windows. A protected path in the workspace is also matched name by name without regard to case, as a prefix of the target's names: on a mount on Linux that ignores case (`/mnt/c` under WSL, an exFAT or NTFS drive, a folder with case folding on) the resolver keeps the script's spelling, and `.GITHUB/workflows` would otherwise be another place than `.github/workflows`. A protected path that is a link is judged where it leads as well as where it is written: a `.git/hooks` that leads to `hooks` protects `hooks`, and one that is a link to nothing protects where its target's name leads, as the look reads a `.cargo` link, since a write could make it. Finding a link costs a look at each name of a protected path that is there, and the resolver only for one that holds a link.
- **The refusal** is a host error the script catches as any other: ``not allowed: `.git/hooks/pre-commit` is in `.git/hooks`, a protected path, and extensions may not write the harness's or the repository's protected files, at any level (`unrestricted` included): whoever writes one can run code later, outside the sandbox``. The path is shown as code it cannot end (`cineris_base::in_code`), relative to the workspace. A protected path outside the workspace that holds it (a `sandbox.protect` entry, a repository) is not named: "the workspace, which is inside a protected path", or "inside a folder that git takes for a repository", since where the workspace is, is not the script's to learn. Since nobody is asked and the script may catch the error, each refusal is in the harness's log at `warn`, with the extension's name and the path, escaped as the extension's own log lines are.
- The `write` tool still resolves its target again when it runs and refuses a path that now leads elsewhere, so a link placed between the check and the write is not written through; what is left is the window the built-in tools share ([a-file-is-opened-by-name-after-its-decision](../../../outstanding/a-file-is-opened-by-name-after-its-decision.md)).

## The tests

| Test | What it holds |
|---|---|
| `cineris-core::it extensions::a_host_write_of_a_protected_path_is_refused_at_every_level_before_the_engine_is_asked` | One world, a session at `supervised` and one at `unrestricted`. Each refuses, naming the path: `.git/config`, `.git/hooks/pre-commit`, `.husky` (a protected folder itself) and `.Husky` (another case, with nothing there to spell it), `.cargo/config.toml`, `.github/workflows/ci.yml`, `src/../.git/config`, `vendor/lib/.git/config`, `crates/x/.cargo/config.toml`, a link in the workspace to `.git/hooks`, and on Windows `.GIT\CONFIG`, `.git/Hooks\post-checkout` and `.GITHUB\Workflows\ci.yml` with no `.github` there. At `supervised`, where the engine would ask about each, nothing is asked (the rig fails the test on a question it did not expect), no denial is recorded, and the log has the refusal at `warn`, naming `probe` and the path; at `unrestricted` none is written. An ordinary write is made at both. |
| `ext::protected::tests::every_protected_path_and_what_is_below_it_is_refused_and_nothing_beside_it` | Each name of `PROTECTED_IN_WORKSPACE` and a path below it; the protected paths in another case (`.GITHUB/workflows`, `.Husky`, `.Pre-Commit-Config.yaml`, `.CINERIS`, `Deploy/Keys`), the names protected at any depth in another case, a `sandbox.protect` entry, a protected path that holds the workspace, a workspace in a `.git` folder and one that is a `.cargo`; and not `.gitignore`, `.github/dependabot.yml`, `.cinerisrc`, `docs/config.toml`, `notes/HEAD.md`, `deploy/keys.txt` beside `deploy/keys`. |
| `ext::protected::tests::on_windows_a_protected_path_is_found_whatever_its_form_and_case` | The Windows rules on every system: a target in the extended-length form against the lease's paths written with `/`, another case, NTFS's fold outside ASCII (`.huſky`), either separator, a share in `\\?\UNC\` form, a protected path that holds the workspace found by place alone; and not another drive or a workspace that only begins like this one. |
| `ext::protected::tests::everything_in_a_folder_git_takes_for_a_repository_is_refused` | A folder with `HEAD`, `objects` and `refs`; not one with only `objects` and `refs`, nor one with `HEAD` and `objects` for a file of its own, but that one when the write would make its `refs`; the refusal's words; a backtick in a name shown escaped; a lease that protects everything. |
| `ext::protected::tests::a_workspace_that_is_or_is_in_a_repository_refuses_every_write` | A workspace that holds `HEAD`, `objects` and `refs`, and ones in such a folder's `hooks` and `Worktrees/main` (its own folders, the second in another case), refused without naming the folder; a worktree of a bare repository, `repo/main`, writes. |
| `ext::protected::tests::a_protected_path_that_is_a_link_protects_where_it_leads` | `.git/hooks` a link to `hooks`, `.git/info` a link to a folder not there, and a `sandbox.protect` path that holds the workspace through a link: each protects where it leads, and the refusal names the protected path. |

The two tests that wrote a CI workflow to have a host write asked about (`a_host_writes_question_offers_allow_once_only_and_keeps_no_rule`, renamed from `a_host_write_of_a_protected_path_asks_once_only_and_keeps_no_rule`, and `a_host_call_nobody_can_answer_with_fail_on_denial_ends_the_scripts_call`) write `.env` instead, a file that looks like a secret, which the engine still asks about with a rule to offer; their breaks in [mutations/step-28.breaks](../../../../mutations/step-28.breaks) name them still.

## The breaks

[mutations/protected-writes.breaks](../../../../mutations/protected-writes.breaks), run with `cargo xtask mutate protected-writes`: **23 breaks, each caught by the tests it names** (15 with the fix, 6 more with the review's fixes, 2 more with the re-check's, and the moved ones run again).

| Break | Caught by |
|---|---|
| The check not made; made only short of `unrestricted`; made only at `unrestricted` | The session test, at the level the break leaves open (at `supervised` by the question the rig did not expect) |
| The target compared with a protected path by its text (`Path::starts_with`) | The Windows forms (the path that holds the workspace) |
| The lease's protected paths not looked at | The names test and the session test (`.husky`, the CI workflow) |
| A protected path not matched by its names in another case | The names test and the session test (`.Husky`) |
| A protected name compared in its case on Linux | The names test (`.GIT/CONFIG`) |
| A `.git` folder, a file named `HEAD`, a build file, a `.cargo` configuration below the root, each left out | The names test, and for `.git` and `.cargo` the session test and the Windows forms |
| The workspace's own folders not judged by their names | The names test (a workspace in `.git`, one that is a `.cargo`) |
| No folder looked for that git takes for a repository; one taken for a repository with only part of it; `objects` or `refs` made beside a `HEAD`; a repository above the workspace not looked for; a worktree of a bare repository refused | The repository tests |
| A protected path that is a link judged only where it is written; a link to nothing not read | The link test |
| A lease that protects everything lets a write through; the path not shown as code | The repository test |
| A refusal not in the log at `warn`; its path logged raw | The session test (a name holding U+202E) |

The tests ran on a Windows runner too, the session test with its Windows forms and the link tests making links there: try runs 44911938396 (the fix, 27 tests) and 44914950304 (the review's fixes, 28 tests).

## The review

An independent review of the fix found no high finding. It confirmed that the check comes first and fails closed, that no policy file changed, that `fs.write` is the only path a script chooses, that a write lands where it was judged (the `write` tool's `still_leads_there`, and its atomic write following no link), and that the resolver refuses an alternate data stream, a trailing dot or space, a device name and `\\?\` input, and gives a short (8.3) name its long one. What it raised, and what was done:

| Finding | Done |
|---|---|
| **Medium: a mount on Linux that ignores case.** The protected paths were compared by `location_within`, which on Linux is `Path::starts_with`, and the resolver keeps the script's spelling: `.GITHUB/workflows/x.yml`, `.Husky/pre-commit`, `.CINERIS/config.toml` got through on `/mnt/c` under WSL, an exFAT or NTFS drive, or a folder with case folding on. | A protected path in the workspace is matched by its names too, without regard to case. |
| A protected path that is a link to nothing (`.git/hooks` to `../githooks`, not there) protected nothing: a write of `githooks/pre-commit` made the hook. | The link is read by its target's name, as the look reads a `.cargo` link. |
| The names protected at any depth were judged only below the workspace: a workspace that is `~/.cargo` let `config.toml` through. | They are judged among the workspace's own folders too, and a repository is looked for above it. |
| A repository could be put together one write at a time. | A write that would make `objects` or `refs` beside a `HEAD` is refused. |
| Nobody was told of a refusal, and `HostError::Denied` said a refusal came from the engine or a person. | Each refusal is logged at `warn`; the variant's text says the host's own refusals too. |
| Tests that need `location_within` on Windows, `Entry::Around`, a workspace that is a repository, and NTFS's fold outside ASCII. | Added (above). |
| What is left, and the pages' words on `cineris.exec` and on the rules that lift a protected path's question. | Below, and in the pages. |

A re-check of those fixes confirmed each, and every break caught for the right reason, and raised three more:

| Finding | Done |
|---|---|
| **Medium: the check above the workspace refused every write in a worktree of a bare repository,** a common layout (`git clone --bare URL repo; cd repo; git worktree add main`), for nothing: the bare repository's data is outside the workspace, and the worktree's `.git` file is refused by name. | A repository above refuses only a workspace in one of its own folders (`hooks`, `info`, `objects`, `refs`, `logs`, `modules`, `worktrees`, compared with the same fold). |
| A refusal by a repository or a `.git` above the workspace named that folder, telling a script where the workspace is. | No protected path outside the workspace is named. |
| The log wrote the script's path raw. | It is escaped, as the extension's own log lines are. |

## What is left

- **A link in the workspace below its root that leads elsewhere in it is not followed** where this check does not name it: a `.cargo` or a `.git` folder below the root that is a link (the look follows a `.cargo` link that leads inside the workspace; this check would need its walk), and a file link such as `sub/.npmrc` to a shared file, or `crates/x/.cargo/config.toml` to `shared/cargo.toml`, whose target is written by its own name. A folder that is a repository by its `HEAD`, `objects` and `refs` is protected wherever it is.
- **A protected path that is a link to nothing is followed one step:** its target is taken by name, and a link on the way to that target is not followed again.
- **The repository's configuration can point git at a file of the workspace,** which then runs what it names: `core.hooksPath` naming a folder outside `.git` (`.githooks`, a `scripts/hooks`); `include.path` or `includeIf` naming a file of the workspace (`../.gitconfig`) that sets a filter, a `textconv`, a merge driver or `core.fsmonitor`; and a filter, `textconv` or merge-driver program given as a relative path to a script in the workspace. None of these is protected here, by the engine or by the look; `.husky` is protected by name. The maintainers route it to the fix that has the harness's own git refuse programs the repository's configuration names (security fix 3).
- **Where nothing confines programs** (Windows today), a script's `cineris.exec` of a program can still write a protected path: the manifest's `write` does not bind it and the approval says so (point 3 of [answered.md](../answered.md)). This refusal is of `fs.write`.
