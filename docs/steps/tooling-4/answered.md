# Answered in the fourth tooling step, on 2026-11-14

This step is not a numbered step of the build plan: it built the fourth round of developer tooling, from what wave 7's five build sessions asked for. Its records are here, in a folder named for the job.

## The coordinator's answers and additions

| Question or addition | Answer |
|---|---|
| Where the local guard's terms are. | `~/.config/cortex-harness/owner-terms`, which the owner created (mode 600), or the file `CORTEX_OWNER_TERMS_FILE` names. The session never read it: every test names a terms file of its own and a home folder of its own. |
| Pushing to test with `try` before the first push of the branch. | Not possible as written, since `try` needs a pushed branch. The order: the reviewer's read and the fixes, then push the branch, then the Windows `try`, then the pull request. This session also used a scratch branch, `worktree-step-02-tooling-4-try`, for `try` runs while it built. |
| The force-push of this branch. | Allowed by the auto-mode rule for this branch only. |
| Additions to item 5 from step 15's review: (a) a building client in the test kit; (b) a table of the fixture models' windows, limits and thresholds; (c) a helper that shows a long string in an assertion's message by its size and its ends. | All three built: (a) `ScriptedProvider::built_as(protocol, provider)`, which replaced `Rendered` in `cortex-agent`'s context tests; (b) the table under "Sizing a context test by pressure" in [testing-and-ci.md](../../testing-and-ci.md); (c) `cortex_testkit::brief`, used in two of the context tests' messages. |

## Choices made in the step

- **The local guard reads each commit's own diff,** not only the net diff CI reads. A term one commit adds and a later one removes is not in the net diff, and the first commit stays reachable by its hash once it is pushed: the case wave 7 met. It also reads the names of the refs a push writes and an annotated tag's text, which CI does not see.
- **The hook is a copy in the repository's own hooks folder,** not `core.hooksPath` pointed at `xtask/hooks`: a hooks path in the worktree would run each branch's own hook, which a branch could turn off. What the copy runs, `cargo xtask owner-guard --pre-push`, is the branch's guard, as CI's is. The install command is `cp xtask/hooks/pre-push "$(git rev-parse --git-common-dir)/hooks/pre-push"`, for the owner to run.
- **The local guard fails closed once the file is there:** an empty file, one that cannot be read or is not UTF-8, one inside the repository, a file the variable names that is missing, a malformed update from git and a git that fails all refuse the push. Only a missing default file passes, with a warning, as CI does without its secret.
- **Every line the local guard prints is screened against the terms,** and a path to the terms file is never spelled out: the home folder is a term, and git's own error text can name it.
- **The comparison by place reads text and never the file system.** A short (8.3) name, a link or a hard link is a question for `canonicalize`; the helpers are for comparing what two `canonicalize`s, or one and a plain path, give back. They are `cortex_base::same_location`, `location_within` and `strip_location_prefix`, each also a method of `PathStyle` so that the Windows rules are tested on Linux.
- **No `rules` check for the path trap.** Whether a path is canonical is not in its text: both bugs it caused crossed a `let` or a function (`find_outside` canonicalized its candidate and compared it with roots from its caller; the instruction discovery compared a resolved root with a joined path), so a check of `canonicalize` beside `==` on one line would have seen neither, and would flag the many places that canonicalize both sides. The trap is in [AGENTS.md](../../../AGENTS.md)'s Windows facts.
- **`try` runs three of the nightly's jobs** (`partial-landlock`, `advisories`, `unused`), each exactly as the nightly runs it. The release builds and the fuzz job are not: they need other targets and a nightly compiler, and the nightly can be dispatched for those.
- **The profile hangs off what the test kit already marks.** Every wait of the kit goes through `watch::doing`, so the profile records those spans there, and a `Scratch` writes the report, so no test changed to be profiled. Process start on Linux is to the system's clock tick (10 ms).
- **The pressure helper measures a twin, not a formula.** The size of a session's first request (the system prompt with its instruction files, the tools' declarations, the dated first message, a notice's framing) is not something a test can add up without repeating the loop's code. A twin of the session, with the same prompt, tools, instruction files, thresholds and model at the same workspace path, runs once with a window it cannot fill, and its context record says the size. It is in `cortex-agent`'s rig, because the test kit cannot depend on `cortex-agent`; `cortex-core`'s tests, which the wave 9 fix session will use, build the same loop and can call the agent rig's approach the same way (a twin `World` and `Front`).
- **The `backends` trap is fixed by turning the feature on,** not by failing: `test`, `stress` and `ci --tiers` of `cortex-sandbox` on Linux add `--features cortex-sandbox/backends`, and say so. `mutate` does not; its docs say how a sandbox break names `cortex-core`.
- **`scratch-test` works in a copy of the tree,** as `mutate` does, so a killed run leaves nothing in the worktree; the cost is a first build in the copy's own build directory (about 10 s for `cortex-base` on the desktop).
- **`ModelClient` joined `WRAPPED`.** It already had no default bodies; tests wrap it (`Built`, the core's secrets tests). A search for every type that holds an implementation of the trait it implements found no other wrapper in the product: `Recording` (`cortex-base`'s git) is a test's fake, and the `Clock` a test wraps is a concrete `FakeClock`.
- **`live --headless`** runs every ignored test of `cortex-cli`'s `tests/system/live.rs` for a provider under `cargo test`; `live` itself still runs the first of them after its smoke test. `live` has no `--dry-run` (that is `record`'s), so the entry was tested without a key, where it stops and names the variable, and by its unit tests.
- **actionlint was not added** ([outstanding/actionlint-in-the-checks.md](../../outstanding/actionlint-in-the-checks.md)): the install action the workflows pin has no entry for it at any version, which a `try` run of `ci lint` showed, and a download checked against a checksum written by hand could not be verified from this session.
- **The adversarial fixture runs its command as a script in the workspace.** A command line that names a hook or a folder outside is asked about at `autonomous` too, so it never reaches the sandbox; `sh adversary.sh` does. The fixture refuses every question and lists them, and lets the fake timer's short waits go while the turn runs, since the shell's grace for a process left running is on it.

## Where a canonical path met a plain one

A search of the workspace for every comparison of a path that may be canonical (`canonicalize`, `to_fs_path`) with one that may not be. Moved onto the helpers:

| Place | What it compared | Now |
|---|---|---|
| `cortex-config`, `Env::display_path` | A resolved workspace path (extended-length on Windows) against the home folder from the environment, so `permissions list` and `config show --origin` printed `//?/C:/…` for a path under it, and the trust store kept that form. | `strip_location_prefix` in the environment's style; a test of the Windows forms on every OS. |
| `cortex-base`, `program::find_outside` | A candidate's real path against the avoided roots, canonical unless canonicalizing failed. | `location_within`. |
| `cortex-core`, `assemble::under_a_write_root` | A program's real path against the write roots, either side plain when it could not be resolved. | `location_within`. |
| `cortex-core`, `sandbox`, the temporary directory's write root | The roots against the temporary directory, which are plain and extended-length when the directory was not made, the case the check exists for. | `same_location`. |
| `cortex-sandbox`, `protect`, `through_a_link` and `leads_inside` | A fixed protected path (plain) and a link's target against the workspace's extended-length form. | `strip_location_prefix` and `location_within`. |
| `cortex-core`, `checkpoints::record_protected` | A protected path's change against the root's extended-length form, so a fixed path was reported as one to put back by hand. | `strip_location_prefix`. |

Looked at and left, because both sides have one form: the program search's own canonicalized roots (now also by place), the shell's cache key (through `AbsPath::from_os`), the instruction discovery (fixed in step 15's fixes through `AbsPath`), the snapshot and checkpoint code (both sides from `to_fs_path`), `xtask`'s local guard (both sides canonicalized), and the tests that canonicalize both sides. Two that the helpers do not reach are an open item: [outstanding/path-forms-on-windows-the-helper-does-not-reach.md](../../outstanding/path-forms-on-windows-the-helper-does-not-reach.md).

## Where a test's time went

`CORTEX_TESTKIT_PROFILE=1`, on the slowest test of each rig, in CI's build profile on the desktop:

```text
time profile of `approve_diff` (CORTEX_TESTKIT_PROFILE): 149.4 ms from the process's start
         at        took  what
     0.0 ms     10.0 ms  process start, before the test kit was first used
    10.0 ms      0.7 ms  phase: set-up
    10.9 ms      0.5 ms  phase: start a core
    11.4 ms      1.3 ms  phase: connect and open a session
   148.8 ms      0.6 ms  tear-down
                 3.0 ms  in phases and tear-down; 0.0 ms in the test kit's waits, on any thread; 136.4 ms in none of them (the test's own work)
```

That run was before the core rig marked its waits for an approval and for a turn's end, which now show where the 136 ms goes. `cortex-cli`'s slowest system test, `commands::sys_a_project_trusted_with_permissions_trust_is_trusted_on_the_next_start`, ran the binary twelve times: 285 ms of its 305 were the binary's runs (each 2 to 28 ms; the ones that send a request about 27 ms), 1.2 ms the scripted server, and 10 ms the test's own work. `cortex-agent`'s slowest integration tests are the seeded-order tests (`order::one_seed_is_one_order_of_events`, 0.10 s), whose time is the test's own: a world and a session each round.

## The independent review

A reviewer on Opus 5.5, which had not seen the session's reasoning, read the whole diff before the first push. It found no way the local guard prints a term, no workflow input in a script, no new right, and nothing that adds to the blocking checks' time. What it found, and what was done:

| Finding | Done |
|---|---|
| With the terms file in place, a push from a process with another `HOME` found no default file and passed with a warning; and the guard is the branch's own code. | Fixed for the first: a repository can require the file (`git config cortex.ownerTermsRequired true`, part of the install), and then a missing file refuses the push. Test and break. The second is CI's case too, and stated in the docs: it catches an accident. |
| Once the hook is installed, a branch made before this change cannot push: its guard has no `--pre-push`. | It fails closed. The docs and the install note say to install once this is merged, and to rebase open branches. |
| A byte-order mark at the start of the terms made the first term match nothing (CI's guard too). | Fixed in the shared parser; and a term with a character that cannot be seen refuses the push. Tests and breaks. |
| A push to a URL was held against every remote's branches, so a commit on a private remote went unchecked. | Fixed: held only against what git says the URL has. Test and break. |
| `log.showRoot=false` in the repository dropped a root commit's changes; `git replace` refs showed a replacement in place of what is sent. | Fixed: `--root`, and `GIT_NO_REPLACE_OBJECTS=1` for every git the guard runs (CI's too). Tests and breaks. |
| Tests that would pass with the code broken: `Built`'s refusal; `repositories()`; the local ref's name; the remote's object in the range; the screening of the verdict's lines. | Tests for the first four, each with a break. The last: a finding's text holds no term by construction (no ref name, no matching path), so the screening there cannot be seen to fail; it stays as a second guard. |
| The path helper normalised `.`, `..` and `/` in an extended-length path, which Windows reads literally, and let `..` climb above a relative path's start. | Fixed: both are compared exactly. Tests and a break. No caller passed such a path. |
| Docs: the number of git commands, the `try.yml` comment, `--features` passed twice to `partial-landlock`, `ci.md` missing. | Fixed the first two; the third is harmless and kept (the job's own check names the feature); `ci.md` is written with the pull request's runs. |
| The adversarial fixture misses a process that clears its environment, and spins a core while the turn runs. | Both stated in the docs. The kill's sweep is what holds the first. |

The step's own stress run found one more: the fixture's own test failed 20 times in 100, because a `sh -c` child had not yet become `sleep` when `/proc` was read. It starts `sleep` directly now, and passed 200 of 200.
