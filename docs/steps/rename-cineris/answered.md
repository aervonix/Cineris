# Answered in the rename to Cineris, on 2026-11-18

This is not a numbered step of the build plan. It renamed the project from Cortex-Harness to Cineris ([decision 0038](../../decisions/0038-the-project-is-named-cineris.md)) with a tool, `cargo xtask rename` ([CONTRIBUTING.md](../../../CONTRIBUTING.md), "Renaming the project"), in one pull request. This record names the old forms, so the tool keeps it whole (it is on the allow-list below). Its CI times and sizes are in [ci.md](ci.md).

## The pull request's commits

1. **The tool, on its own:** `xtask/src/rename.rs` (the table, the kept lines, `apply`, `--check`), `xtask/src/rename/branch.rs` (the branch rewrite) and their tests.
2. **Pure tool output over `main`:** `cargo xtask rename apply`, then `cargo xtask regen` and `cargo xtask outstanding`. Nothing by hand.
3. Then the hand edits, each a commit of its own: decision 0038 (with 0013's "superseded" line, its index row and the build plan's rows); the README's name and pronunciation line; `AGENTS.md`'s rule 5; the build plan's rule 5, step 34 and the docs on the alias pointed at 0038; the check in the repo rules, and the tool in `CONTRIBUTING.md`; the fixes the tool could not make; `rules::file_rules` and its test; the break list.

## The inventory

Counted on `main` at `1e733de5`, before the rename.

**In the files' text:** 1,009 files hold the name or the alias, and 1,305 paths hold the name.

| Form | Occurrences | Becomes |
|---|---|---|
| `cortex` (crates, identifiers, the command, `.cortex/`, `cortex.ownerTermsRequired`) | 17,821 | `cineris` |
| `CORTEX` (`CORTEX_*`) | 443 | `CINERIS` |
| `cortex-harness` (folders, `@cortex-harness.invalid`, the logo files) | 267 | `cineris`, or `cineris-dev` for the developer's folders |
| `Cortex-Harness` (the brand, the identity's name, `aervonix/Cortex-Harness`) | 160 | `Cineris` |
| `Cortex` (`Cortex Test`, the identity of test repositories) | 10 | `Cineris` |
| `CORTEX-HARNESS` (Windows paths in upper case, in tests) | 3 | `CINERIS` |
| The alias `cx`, where it is the alias | 169 | `cin` |

- **By folder:** `crates/` 619 files, `docs/` 256, `xtask/` 60, `mutations/` 52, `fuzz/` 8, `.github/` 5, and the root files (`Cargo.toml`, `Cargo.lock`, `README.md`, `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`), `catalog/models.toml`, `corpus/Cargo.lock` and `.config/nextest.toml`.
- **Compound forms:** the 20 crates `cortex-*`, and `cortex-fuzz` and `cortex-corpus` outside the workspace; the Rust paths `cortex_*::`; `Cortex Test <test@cortex-harness.invalid>` and the other `@cortex-harness.invalid` addresses (`dev@`, `test@`, `branch@`, `combine@`); `cortex.ownerTermsRequired`; the Windows Credential Manager's entries `cortex-harness:<provider>`; the protocol's `server.name` and the statistics' `SOURCE` (`cortex-harness`); the schema annotations `x-cortex-*`; the sandbox policy's magic `cortex-sandbox-policy`; temp prefixes `cortex-harness-*` and `.cortex-*.tmp`; the live tests' `cortex-live` folders.
- **The alias, which is also the tools' and the loop's context variable:** 169 uses took the alias's forms (a few of them a made-up test binary of that name, renamed with it consistently). They were quoted (`` `cx` ``, `"cx"`), part of a path (`bin/cx`, `/config/cx/`), a file (`cx.exe`, `CX.EXE`), a command with what follows (`` `cx --version` ``, `"cx: …"`, `Usage: cx`, `binary_id(=cx)`), or the start of a name made from it: the sandbox guard's process name `cx-tree-guard` (13 characters; `cin-tree-guard` is 14, within Linux's 15), and the test kit's markers `cx-child=`, `cx-echo`, `cx-rerun:`, `cx-launch` and the others. Three comments name the variable as `` `cx` `` and are kept. `cx.cancel`, `&cx`, `as_cx`, `cx: &RunCx` and `cx::it` are not the alias, and stay.
- **60 environment variables:** `CORTEX_ADVERSARY`, `CORTEX_CONFIG`, `CORTEX_CORPUS_DIR`, `CORTEX_DATA_DIR`, `CORTEX_DECIDE_*` (`COMMAND`, `CWD`, `LEVEL`, `RULES`, `SHELL`, `UNTRUSTED`, `WORKSPACE`), `CORTEX_LIVE_*` (`ANTHROPIC_KEY_FILE`, `DEEPSEEK_KEY_FILE`, `OPENROUTER_KEY_FILE`, `VENICE_KEY_FILE`, `DRY_RUN`, `LEDGER`, `SCENARIOS`), `CORTEX_LOG`, `CORTEX_MODEL_MAIN`, `CORTEX_NO_SUCH_VARIABLE`, `CORTEX_OWNER_TERMS_FILE`, `CORTEX_PERF_*` (`CONFIG`, `DEMO`, `DIR`, `EDITOR_DEMO`, `FILES`, `JSON`, `ROUNDS`, `RUNS`, `SECONDS`, `SOURCE`, `SYNC`, `THREADS`, `TREE`), `CORTEX_PERMISSIONS_MODE`, `CORTEX_PRESSURE_FILE`, `CORTEX_PROPERTY_SCALE`, `CORTEX_PROPERTY_SEED`, `CORTEX_TESTKIT_*` (`PAUSE_MS`, `PROFILE`, `PROFILE_DIR`, `RERUN`, `SHELL_MODES`, `STARTS`), `CORTEX_TEST_FIRST_SEED`, `CORTEX_TEST_SEED`, `CORTEX_TEST_TMPDIR`, `CORTEX_TS_CLIENT`, `CORTEX_UPDATE_*` (`GENERATED`, `SCREENS`, `SNAPSHOT`, `SNAPSHOTS`, `TOOLBOX`), `CORTEX_WATCH_HANG`, `CORTEX_WATCH_LIMIT_MS`, `CORTEX_WATCH_STAY`.
- **File and folder names:** the 20 crates' folders; `docs/protocol/schema/cortex-protocol.schema.json` and `docs/protocol/ts/cortex-protocol.d.ts`; decisions 0030 and 0037, whose names hold a crate's; and the two logos, `assets/cortex-harness-logo-*.png`. The logos are images of a "C", which fits the new name. They were renamed, not redrawn.
- **Generated files:** the protocol's schema and declarations, the configuration's schema and reference, `Cargo.lock` (and `fuzz/` and `corpus/`'s), the golden screens and snapshots, and the open items' index. They were renamed as text, and their own commands then wrote them again (below). The two golden transcripts (`crates/cineris-protocol/tests/golden/`) have no generator. They were renamed as text, and the names are all that changed in them (checked: the old text with the names replaced is the new text): the product's folders in example paths and messages, and the statistics' source `cineris`.
- **Recorded provider fixtures:** the six Venice `request.json` files (the `prompt_cache_key` holds the product's name) and the fixtures' snapshots. Renamed as text, consistently with what the code sends, and not re-recorded. The Anthropic fixtures moved with their crate, unchanged.
- **Disguised forms**, which no table can find: the policy test's glob `.cort?x`, which stood for the project folder; the compare tests' columns, padded to the old crates' names; a start record that named the alias after a tab. Fixed by hand (below).
- **What did not need changing:** the CI jobs' and steps' names (`lint-linux`, `test-windows`, …), so the required checks keep their names; the one secret (`OWNER_TERMS`); `AGENTS.md` as the instruction file's name.

**Outside the repository,** and who changes it:

| Name | Where | Who |
|---|---|---|
| The repository `aervonix/Cortex-Harness` → `aervonix/Cineris` | GitHub. GitHub redirects the old address. No check fetches a GitHub URL. | The coordinator, before the merge |
| The required checks, the secret `OWNER_TERMS`, the repository's settings | GitHub: no name in them changes | Nothing to do |
| `~/.config/cortex-harness/owner-terms` → `~/.config/cineris-dev/owner-terms`, and `cortex.ownerTermsRequired` → `cineris.ownerTermsRequired` | The laptop and the desktop. Both are set already: the new file is a hard link to the old, and the shared git configuration has both keys. Remove the old ones once step 25 runs the new code. | The coordinator |
| The pre-push hook installed in the repository's hooks folder | It runs `cargo xtask owner-guard --pre-push`, which reads the new path and key from the branch's own code. Copy `xtask/hooks/pre-push` again after the merge, for its comments. | The coordinator |
| The desktop's `rx`: `CORTEX_TEST_TMPDIR` | It exports `CINERIS_TEST_TMPDIR` too since 2026-11-18. Drop the old one when step 25 is across. The `cortex-rs-tests*.lock` names stay. | The coordinator |
| `~/.config/cortex-live/*-key` → `~/.config/cineris-live/`, and `CORTEX_LIVE_*_KEY_FILE` → `CINERIS_LIVE_*_KEY_FILE` | The desktop | The coordinator moves the folder, without reading the keys |
| `~/.local/state/cortex-live/spend.jsonl` → `~/.local/state/cineris-live/` | The spend ledger, on the desktop (`%LOCALAPPDATA%` on Windows) | The coordinator: move it, or the new ledger starts empty |
| `~/.cache/cortex-harness/corpus` → `~/.cache/cineris-dev/corpus` (`%LOCALAPPDATA%\cineris-dev\corpus`) | The laptop and the desktop | The coordinator: move it, or `cargo xtask corpus fetch` makes it again |
| The product's own folders, `~/.config/cortex-harness/` (config, credentials), `~/.local/state/cortex-harness/` (trust, approvals) and `~/.local/share/cortex-harness/` (sessions); a project's `.cortex/`; Windows Credential Manager entries `cortex-harness:<provider>` | Any machine the harness ran on | The owner, by hand: with the clean switch nothing reads them, so they also lose the permission engine's protection (the engine protects the folders it names, which are the new ones). Delete them, or move them to the new names. |
| The TypeScript client | It has no package and no registry name: it is a test client in `crates/cineris-cli/tests/system/ts_client/`, which names itself `cineris-ts-client` to the server (renamed) and installs only `typescript` and `@types/node` from npm, in CI | Nothing outside |
| The domain `cineris.dev` | Bought by the owner; nothing links to it yet | The owner |
| Memory notes and prompts of the build sessions that name the old paths | The coordinator's and the sessions' own notes | The coordinator |

## The allow-list

`cargo xtask rename --kept` prints it from the table. These are the only places an old name stays.

| Kept | Why |
|---|---|
| `xtask/src/rename.rs`, whole | The tool's own table names the old names |
| `xtask/src/rename/`, every file | The branch rewrite and the tests, which need the old names as input |
| `docs/decisions/0013-naming.md`, whole | The decision of the old names, kept as the record of what was decided then (with a line at its top saying 0038 supersedes it) |
| `docs/decisions/0038-the-project-is-named-cineris.md`, whole | The decision of the rename says what the names were and why they changed |
| `docs/steps/rename-cineris/`, every file | This record: the inventory lists the old forms |
| `docs/decisions/README.md`, the line with `[0013](0013-naming.md)` | 0013's row in the index carries 0013's own title |
| `docs/build-plan.md`, the line with "The name clash step 2 found." | The owner's decision on the clash with Snowflake's `cortex`, as it was taken (with a pointer to 0038) |
| `crates/cineris-tools/src/contract.rs`, two comment lines; `crates/cineris-tools/src/lib.rs`, one | `` `cx` `` there is the tools' context variable, not the alias |

The coordinator agreed on 2026-11-18 to add 0038 and this record to the spec's list, which had only the tool and the two history records.

## Decisions on the way

- **The developer's folders** (agreed with the coordinator): the corpus cache goes to `cineris-dev` too, not into the product's cache folder; the live tests' folders are `cineris-live`.
- **The commit identity:** the worktree guard refused `git config --worktree` (it needs `extensions.worktreeConfig` in the shared configuration), so every commit of this branch was made with `git -c user.name=Cineris -c user.email=dev@cineris.invalid`. The shared configuration still has the old identity, which step 25 uses until it is across.
- **Nothing reads an identity.** No check compares a commit's identity with an expected one: the owner guard looks for the owner's terms in every commit. A test (`rename::tests::both_identities_pass_the_owner_guard`) shows commits of both identities pass it.
- **The tool's kept code names no crate.** Its tests keep their text through the rename, so they reach the test kit through `crate::testkit`, a re-export in `xtask/src/main.rs`, which the rename does rename. A test holds them to that. The first desk run found this, when the tests still named the old crate.
- **The repo rules run the check** in `rules::file_rules`, which the lint job's `rules` step and the tier-1 test of the rules both run. It adds about 0.3 s to `cargo xtask rules` at the desk.

## The generated files, and the layout

Of the generated files, seven came out of their commands differently from the text rename, all from the brand's width. Each was read:

- **The header** (`header-and-hint.txt`, `app-approval-in-its-grace.txt`, the reference screen 0 and its two `.ans` copies): "Cineris" is seven characters shorter than "Cortex-Harness", so the box's padding grows by seven. Nothing wraps.
- **`project.txt`:** a problem line naming `.cineris/profile.toml` is one character longer, and at 60 columns its ellipsis comes one character earlier ("the file i…" for "the file is…"). A truncation that moved, not a wrap.
- **`show-origin.txt`:** the comment column of `log.level = "info,cineris_llm=debug"` is one space narrower, because the value is one character longer and the column is fixed.

## The dry run of step 25's branches

Every branch was copied locally and rewritten with `cargo xtask rename branch BASE..<copy>`. A second run found each one already there. Nothing was pushed to step 25's branches, and the copies were deleted afterwards.

**The current branches** (the coordinator's word: `worktree-step-25`, PR 1, and `worktree-step-25-session`, PR 2, both on `main` at `1e733de5`, 25 and 36 commits):

- **The rebase onto this pull request's head** (`git rebase --onto <head> <R(BASE)> <copy>-renamed`, in a second checkout under `target/`) goes through with **no conflict** for both.
- **The result,** in the session branch's tip: `cargo xtask rename --check` passes, and so do `cargo xtask outstanding --check` and `cargo fmt --all --check`.
- **What step 25 still runs after its rebase:** `cargo xtask regen`. Its new golden screens were renamed as text, and a screen whose header holds the brand is laid out again by its command.

**The branches from the pause** (`worktree-step-25-wip`, `-session-wip`, `-rewind-wip` and `-firstrun-wip`: 18, 25, 24 and 24 commits on `24305e3d`, 37 commits behind `main` before the rename). Each was compared with a rebase of the same branch, not renamed, onto `main` as it was before the rename. The comparison used `git merge-tree`, which checks nothing out:

- **A rebase, commit after commit:** each branch stops at the same commits, on the same files, before the rename and after it. `wip` stops once and the others twice:
  - `crates/cineris-cli/src/interactive.rs`, at "Fix what the independent review of the full screen found": `main` changed `keymap` after these branches left it. Not the rename.
  - `docs/build-plan.md`, at "Describe /export in 09-tui and the protocol page, and close its open item": `main` changed the same lines. Not the rename. The rename's own edits to the build plan are elsewhere on the page.
- **Each commit picked alone,** a stricter test because a commit then lacks the ones before it: the same files conflict before the rename and after it, commit for commit, in all four. The rename adds no conflict and takes none away.
- **The rewritten tips** hold no old name, read from git, path and text, outside the kept lines.

**What the rewrite costs:** 35 s for an 18-commit branch on the laptop, most of it `rustfmt` on the first tree. **What it gives:** R(BASE) is byte for byte the tool's output commit in every file but the seven that `regen` lays out again (the review compared 1,757 files).

## The reviews

An independent reviewer, run on Fable, read the changes twice: first only the paths that feed the system prompt and the permission engine, then the whole diff.

**The focused round** (the system prompt, the permission engine, the folders it trusts, `CINERIS_PERMISSIONS_MODE`, `CINERIS_DECIDE_*`, the program's name):

- **Nothing changes in what the engine trusts or protects.** The product's folders come from `APP_DIR` and `PROJECT_DIR`, and the protected set and the workspace's discovery derive from them. The system prompt's name comes from one constant. `CINERIS_DECIDE_*` matches between `xtask/src/decide.rs` and the probe. `CINERIS_PERMISSIONS_MODE` appears only in the test that shows it is ignored.
- **The alias was half renamed in tests** (the names tests, help, the project view, instruction discovery, xtask's rerun tests). The first rule knew too few of the alias's forms. **Fixed** in the tool: the rule now knows a path's part, a name made from the alias, a command with what follows, `=cx`, `Usage: cx`, and leaves `cx::` alone. The whole output was made again from it, and the desk's run showed the tests pass.
- **Found and fixed with it:** `startup.rs` made the alias as `cx{EXE_SUFFIX}` and labelled it `cin`. A break's rule and a near miss of the alias (`name_for("cxx")`) named the old alias; they are fixed by hand.
- **The owner's to know:** the guard reads only the new terms file and key; the old folders lose their protection; `server.name` and the statistics' source changed with the names. These are under "Left open".

**The full round,** with nothing found against behaviour, formats or what the engine protects:

| # | Finding | What was done |
|---|---|---|
| 1 | The owner guard fails open on a clone that has only the old key and the old file: the new code reads only the new ones | Not changed: it follows from the clean switch. Both keys and both files are set on the owner's machines. Under "Left open" for the owner |
| 2 | `branch --force` could move a branch a worktree was rebasing (its `HEAD` is detached then, so `git worktree list` named none); nothing tested the command itself | **Fixed:** it reads each worktree's git directory, as `branch commit-records` does, and refuses a branch checked out, being rebased or bisected. Tested: naming, finding, refusing and replacing, and the refusal mid-rebase. Break `a-branch-being-rebased-is-replaced` |
| 3 | The check finds the alias only in the forms the rename renames; names built on the old short form stay (`cxterm-*=`, `cxtui-*=`, `CX_PIPE`, `CXTUI_SCRATCH` and the like, `as_cx`); two test messages said "of cx" | The messages were **fixed**. The short-form names are left on purpose (below). The check's limit is written in the tool's help |
| 4 | The alias rule renamed a made-up binary `cx` in xtask's tests to `cin` beside `cx::it`; `/config/cx` became `/config/cin` | No effect: those tests are consistent and pass. The inventory's 169 counts these uses too |
| 5 | A `rustfmt` that cannot run was silent, and a rewrite would then differ from the output commit | **Fixed:** `apply` and `branch` fail when `rustfmt --version` does not run in the repository |
| 6 | Symbolic links: `apply` read and wrote through one, and `branch` renamed its target | **Fixed:** both refuse a link (none is tracked). Tested on Unix; break `apply-goes-through-a-link` |
| 7 | A file that is not UTF-8 was skipped by the check without a word | **Fixed:** the check scans its bytes for the old name. Tested; break `the-check-reads-only-text` |
| 8 | The branch test gave the author and the committer one identity and one date; a failed write hid fast-import's message; slips in this record | **Fixed:** the test gives each its own and checks both are kept; fast-import's message comes first, and a stop of the tool's own ends the stream cleanly, with no crash report (tested). The record is corrected |

**The second round,** of the changes since the full round: <!-- filled in below -->

**The breaks:** `cargo xtask mutate rename-cineris`, at the desk: 18 breaks, all caught by the tests their list names. One break, the check missing the alias, was caught at first by only one of its two tests. That test planted the old name and the alias on one line, and now plants them on two. `cargo xtask mutate --check --tests`: all 2,742 breaks apply, and every `caught-by` names a test that exists.

## Left open

- **Names built on the old short form, left on purpose.** Some test markers and variables are built from `cx` run on without a separator: `cxterm-*=`, `cxtui-*=`, `cxcli-tui=`, `cxstats-writer=`, `CX_PIPE`, `CX_EMPTY`, `CX_SYSTEM`, `CX_RERUN_SET`, `CX_RERUN_REMOVED`, `CXTUI_SCRATCH`, `CXTUI_SIGNALS_REPORT`, `CXTERM_HANDOFF_REPORT`, and `as_cx`. None is the brand or the alias; each is a test's own label. They are left: a rule for them could not tell them from other words, and step 25 changes the files most of them are in, so renaming them by hand would make conflicts for it. A later fix session may rename them, when no branch is open over those files.
- **For the owner** (see the table of names outside the repository):
  - Move or delete the product's old folders and the Credential Manager's entries by hand. Nothing reads them any more, and the permission engine no longer protects them.
  - Keep the new owner-terms file and key in place before the old ones go. The new code reads only the new ones, and without the key it would warn and pass on a machine missing the file.
  - The protocol's `server.name` and the statistics' `source` are `cineris` now. They are names, with no format version change.
- **The rename tool stays** for `rename --check` and for any branch still made before the rename. Once step 25 is across, `branch` has nothing left to do, but costs nothing to keep.
