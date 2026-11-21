# Step 28, its parts: an extension's writes of protected paths

A fix of the first part's host functions ([pr1a-engine.md](pr1a-engine.md)), found by another session's grep and settled by the maintainers: an extension's script may not write a protected path, at any level.

## What was found

`cineris.fs.write`, the one host call that writes a file, was decided by the permission engine as the `write` tool's write. The engine asks about a write of a protected path at every level short of `unrestricted` and allows it unasked there ([06-permissions-and-sandbox.md](../../../architecture/06-permissions-and-sandbox.md), "Protected paths"), and nothing looked at the write afterwards, as the look at the protected paths does after a command. So a script at `unrestricted` could write `.git/config`, a hook in `.git/hooks` or `.cargo/config.toml`, and then:

- where the sandbox does not enforce (Windows today), the harness's own git ran a filter or a hook the script wrote, with the user's rights;
- where it does, the model's own `git status` at `unrestricted` ran it.

No other host call writes a file: `cineris.exec`'s programs are confined at every level where the sandbox enforces, and looked at after; `cineris.state` writes the extension's own values in the state directory. There is no host call that creates a folder, appends, renames or removes, so no rename from a protected path to judge.

## The fix

**The core's host refuses the write before the engine is asked, or the `write` tool planned, at every level, `unrestricted` included** (`cineris-core`'s `ext::protected`, called first in the host's `write`). It mirrors `cineris.exec`'s programs being confined at every level, and it changes none of the engine's rules: it is the extension host's own check, as the manifest's is.

- **What is protected** is the table of protected paths, which the look after a command watches too: the lease's protected paths (the names of `PROTECTED_IN_WORKSPACE` under the workspace, and `sandbox.protect`), each with everything under it, a protected folder itself included; at any depth, everything in a folder named `.git`, a file named `.git` or `HEAD`, `.cargo/config.toml`, `.cargo/config`, `.npmrc`, `.yarnrc`, `.yarnrc.yml` and `.envrc`, their names compared without regard to case on every system (protecting too much is the safe side, as the engine judges them); and everything in a folder that holds `HEAD`, `objects` and `refs`, which git takes for a repository, whatever its name, as the look finds one. The list is read where it is, from `cineris-policy` and the session's lease; nothing of the shell's look was moved.
- **Judged by place.** The target is where the session's path resolver led the script's path (`..`, every link followed, the spelling on disk), the same `ResolvedPath` the manifest judged, and it is compared with each protected path by `location_within`, so the extended-length form, `/` against `\` and another case are one place on Windows. A protected path that is a link inside the workspace is judged where it leads as well as where it is written: a `.git/hooks` that leads to `hooks` protects `hooks`. Finding that costs a look at each of its names that is there, and the resolver only for one that holds a link.
- **The refusal** is a host error the script catches as any other: ``not allowed: `.git/hooks/pre-commit` is in `.git/hooks`, a protected path, and extensions may not write the harness's or the repository's protected files, at any level (`unrestricted` included): whoever writes one can run code later, outside the sandbox``. The path is shown as code it cannot end (`cineris_base::in_code`), relative to the workspace, and a protected path that holds the workspace under `~`.
- The `write` tool still resolves its target again when it runs and refuses a path that now leads elsewhere, so a link placed between the check and the write is not written through; what is left is the window the built-in tools share ([a-file-is-opened-by-name-after-its-decision](../../../outstanding/a-file-is-opened-by-name-after-its-decision.md)).

## The tests

| Test | What it holds |
|---|---|
| `cineris-core::it extensions::a_host_write_of_a_protected_path_is_refused_at_every_level_before_the_engine_is_asked` | One world, a session at `supervised` and one at `unrestricted`. Each refuses, naming the path: `.git/config`, `.git/hooks/pre-commit`, `.husky` (a protected folder itself), `.cargo/config.toml`, `.github/workflows/ci.yml`, `src/../.git/config`, `vendor/lib/.git/config`, `crates/x/.cargo/config.toml`, a link in the workspace to `.git/hooks`, and on Windows `.GIT\CONFIG` and `.git/Hooks\post-checkout`. At `supervised`, where the engine would ask about each, nothing is asked (the rig fails the test on a question it did not expect) and no denial is recorded; at `unrestricted` none is written. An ordinary write is made at both. |
| `ext::protected::tests::every_protected_path_and_what_is_below_it_is_refused_and_nothing_beside_it` | Each name of `PROTECTED_IN_WORKSPACE` and a path below it, the names protected at any depth in another case, a `sandbox.protect` entry, and a protected path that holds the workspace; and not `.gitignore`, `.github/dependabot.yml`, `.cinerisrc`, `docs/config.toml`, `notes/HEAD.md`, `deploy/keys.txt` beside `deploy/keys`. |
| `ext::protected::tests::on_windows_a_protected_path_is_found_whatever_its_form_and_case` | The Windows rules on every system: a target in the extended-length form against the lease's paths written with `/`, another case, either separator, a share in `\\?\UNC\` form; and not another drive or a workspace that only begins like this one. |
| `ext::protected::tests::everything_in_a_folder_git_takes_for_a_repository_is_refused` | A folder with `HEAD`, `objects` and `refs` and not one with only `objects` and `refs`; the refusal's words; a backtick in a name shown escaped; a lease that protects everything. |
| `ext::protected::tests::a_protected_path_that_is_a_link_protects_where_it_leads` | `.git/hooks` a link to `hooks`: `hooks/pre-commit` is refused, and the refusal names `.git/hooks`. |

The two tests that wrote a CI workflow to have a host write asked about (`a_host_writes_question_offers_allow_once_only_and_keeps_no_rule`, renamed from `a_host_write_of_a_protected_path_asks_once_only_and_keeps_no_rule`, and `a_host_call_nobody_can_answer_with_fail_on_denial_ends_the_scripts_call`) write `.env` instead, a file that looks like a secret, which the engine still asks about with a rule to offer; their breaks in [mutations/step-28.breaks](../../../../mutations/step-28.breaks) name them still.

## The breaks

[mutations/protected-writes.breaks](../../../../mutations/protected-writes.breaks), run with `cargo xtask mutate protected-writes`: **15 breaks, each caught by the tests it names.**

| Break | Caught by |
|---|---|
| The check not made; made only short of `unrestricted`; made only at `unrestricted` | The session test, at the level the break leaves open (at `supervised` by the question the rig did not expect) |
| The target compared with a protected path by its text (`Path::starts_with`) | The Windows forms |
| The lease's protected paths not looked at | The names test, the link test and the session test (`.husky`, the CI workflow) |
| A protected name compared in its case on Linux | The names test (`.GIT/CONFIG`) |
| A `.git` folder, a file named `HEAD`, a build file, a `.cargo` configuration below the root, each left out | The names test, and for `.git` and `.cargo` the session test and the Windows forms |
| No folder looked for that git takes for a repository; one taken for a repository with only part of it | The repository test |
| A protected path that is a link judged only where it is written | The link test |
| A lease that protects everything lets a write through; the path not shown as code | The repository test |

A `sys_` test of the harness's git, which touches nothing of this fix, failed beside two of the breaks while the machine's disk stalled (the run said so); every break was caught by its own tests.

The tests ran on a Windows runner too, the session test with its Windows forms and the link test making a link there (try run 44911938396, 27 tests).

## What is left

- **A protected path that is a link to nothing** is judged where it is written only: the resolver refuses a link to nothing, and a write could make what it leads to.
- **A `.cargo` or a `.git` below the workspace's root that is a link** to a folder elsewhere in the workspace does not protect that folder (the look follows a `.cargo` link that leads inside the workspace; this check would need its walk), unless the folder is a repository by its `HEAD`, `objects` and `refs`.
- **`core.hooksPath`** naming a folder outside `.git` (`.githooks`, a `scripts/hooks`) leaves that folder unprotected here, by the engine and by the look alike; `.husky` is protected by name.
- **Where nothing confines programs** (Windows today), a script's `cineris.exec` of a program can still write a protected path: the manifest's `write` does not bind it and the approval says so (point 3 of [answered.md](../answered.md)).
