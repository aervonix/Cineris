# The tenth tooling step: runs on the runners, and a branch's history without the shell guard

Branch `worktree-step-02-tooling-10-runs`, on `origin/main` at 7f820234.

## What was built

### A. `try --dispatch` fixes `--against REF` to a commit at the dispatch

- `try --dispatch BRANCH --command … --against REF` asks GitHub for the commit REF names (`gh api repos/{owner}/{repo}/commits/REF --jq .sha`), sends its full hash as the `against` input, and prints `against main is <12 digits> (resolved at dispatch)`.
- Why GitHub and not `git ls-remote origin REF`: the workflow's checkout resolves the ref against GitHub, and the commits API resolves the same names (a branch, a tag, a hash) the same way; `ls-remote` matches a pattern by its tail (`main` also matches `refs/heads/x/main`) and resolves no expression; the dispatch needs `gh` and its sign-in already; and the copy `rx` builds in has no `.git`, where a local resolution would fail.
- A full 40-digit hash passes through unchanged, and GitHub is not asked. A ref GitHub does not know (HTTP 422) is refused, naming the ref, before anything is dispatched. An answer that is not a full hash is refused too.
- The workflow's `actions/checkout` takes a hash as `ref` (it fetches that commit), and `try`'s own check of the input (`plain`) takes one. The A/B's report labels the base by the hash's first twelve digits (`ab --label 0123456789ab`, and `perf-ab` likewise), since only the hash reaches the runner.
- `--dry-run` asks GitHub nothing, and adds a note that a dispatch would resolve the ref.

### B. A run queued with no jobs past a limit

- The rule is `runs::sat_queued(status, created, now, jobs_started, limit_minutes)`: a run that is `queued` (or `waiting`, `pending`, `requested`), or `in_progress` with no job started (running or ended), for longer than the limit. The text is the coordinator's: `run <id> (<workflow>) has sat queued with no jobs for <M> min: GitHub has not given it a runner. Cancel and dispatch it again (gh run cancel <id>), or keep waiting.`
- `--queued-warn MIN`, with `--queued-limit` as its alias (as `ci-wait` has it), 10 by default, 1 to 1440, in `try --dispatch` and `try-result`.
- `try-result RUN`: an unfinished run's error ("run N is queued, not finished: `ci-wait --runs N` waits for it") gets the line below it when the rule holds. It now asks `gh run view` for `createdAt` too.
- `try --dispatch`: after it finds its run, it lists the branch's `try` runs (`gh run list --workflow try --branch B --json databaseId,status,createdAt`) and asks about each unfinished one older than the limit (`gh run view --json workflowName,status,createdAt,jobs`), printing the line for each that holds. Its own run is seconds old, so in practice this names an earlier run on the branch that has waited that long, which says the new one may wait as long. A failure of this look is printed and does not fail the dispatch. `--queued-warn` without `--dispatch` is refused.

### C. Each side's wall time, for each round

- `ab` times each side's job in each round (`Instant` around the side's two runs, tiers 1 and 2 then the system tests), records whether it passed, and prints after the rounds a table: round, which side ran first, the base's and the head's seconds, and each side's median over the rounds it passed (a failed side is marked `!` and left out of its median). It goes in the log and the job summary.
- It writes `target/ab/wall-times.json` (label and rounds) after every round, so a run that is stopped keeps what it made. `try.yml` uploads `target/ab/wall-times.json` with the reports, and `try-result` prints the table for each OS when the artifact has it (a run from before this has none, and prints none).

### D. `cargo xtask branch` (no clash: no `branch` command before)

- `branch commit-records --branch B --message MSG FILE…` (as first built; the review changed it, below): plumbing only, in a temporary index under the git dir (`rev-parse --git-path xtask-records-<pid>.index`, removed after): `cat-file --batch-check` of `refs/heads/B^{commit}` and `^{tree}`, `worktree list --porcelain`, `read-tree B`, `hash-object -w -- FILE…` (as `git add` would hash them, filters included), `update-index --index-info`, `write-tree`, `commit-tree TREE -p B -m MSG` (the repository's configured identity, as `git commit` uses), `update-ref refs/heads/B NEW OLD`. Refuses: a FILE not `docs/**/*.md` or `mutations/*.breaks` (or not a path from the top: `..`, absolute, a drive), a branch checked out in any worktree (naming it), a branch that is not there, a file not in the working tree, a commit that changes nothing. Prints the commit and `git push origin B`; pushes nothing.
- `branch fold COMMIT [--last] [--base REF]` and `branch to-end COMMIT [--base REF]`: the current branch only, from COMMIT on. COMMIT must be in `rev-list BASE..HEAD` (BASE `origin/main` unless `--base`); when it is not, two `merge-base --is-ancestor` questions say whether it is published or not on the branch. Refuses a merge in the range (the target or after it), a history after it that is not a straight line, unstaged changes (fold takes the staged and only them; to-end and `--last` refuse any change, `diff --quiet HEAD`), nothing staged for a plain fold, a detached HEAD, a root commit, a commit with nowhere to move.
- How the history is made: **not by cherry-pick on a detached HEAD**, which the brief offered, but by `git merge-tree --write-tree --merge-base=P ONTO C` and `git commit-tree` for each commit (author name, email and date, and the message byte for byte, from `cat-file --batch`; the committer is the repository's identity, now). Why: it touches no ref, no index and no file while it works, so a conflict leaves the original state exactly with nothing to abort or restore, and a session's working tree is never checked out to another commit. It needs git 2.40 or later (`--merge-base`); the desk has 2.53, and GitHub's runners are newer than 2.40.
- The staged changes are read with `write-tree` from a copy of the index (the index itself is not written), made into a throwaway commit on HEAD, and folded in as a three-way merge.
- Before anything moves, the new tip's tree must be the one it must be: the old tip's (to-end, fold `--last`) or the staged tree (fold). Then one `update-ref --stdin` transaction creates `refs/xtask-backup/<branch>/<unix time>` at the old tip and moves the branch from the old tip to the new one: both or neither. A backup name of the same second gets `-1`, `-2`.
- Output: each commit made again (`new (was old)  subject`), the backup, and `git reset --keep <backup>` to go back.
- Tried on this branch itself: its fixes were committed as `fixup branch` and `fixup docs` and folded in with `branch fold <commit> --last` twice; the tree after was the tree before (`git diff <backup> HEAD` empty), and the commits kept the repository's identity. The two backup refs it made were deleted afterwards.

## Docs

- `CONTRIBUTING.md`: the `try` row says what `--dispatch` does now; a row for `branch`.
- `AGENTS.md` "Working here": the `try-result` row mentions the wall times and the queued line; a row for `try --dispatch`; a row each for `branch commit-records` and `branch fold`/`to-end`.
- `xtask/src/main.rs` USAGE: `try-result`, `try`, `ab` and the new `branch`.
- `docs/testing-and-ci.md`: two bullets after the `--dispatch` bullet in "A run on the runners without a pull request" (against fixed at dispatch; a queued run named), a bullet in "Two trees on one runner" (each side's wall time), and a new section "A branch's history without the shell guard" before "The public API between two trees".
- `docs/shell-guard.md`: a bullet in "The forms to use".
- `.github/workflows/try.yml`: uploads `target/ab/wall-times.json`.

## Tests

Unit (tier 1, no network, no clock read, no git):

- `runs::tests::a_run_that_sat_queued_with_no_jobs_past_the_limit_is_named_and_no_other`: the rule at fixed times (at the limit, a second past, each waiting status, in progress with and without a started job, finished, a custom limit, a clock behind the creation), the text, the jobs counted as started, `queued_note` on `gh run view` JSON, and `--queued-warn`/`--queued-limit` values.
- `try_result::tests::an_unfinished_run_says_so_and_names_a_run_that_sat_queued_too_long`, and `--queued-limit` in `a_request_is_read_and_a_wrong_one_refused`; the A/B artifact test now has a `wall-times.json` and checks the table and its medians.
- `try_run::tests::against_is_resolved_to_its_commit_at_dispatch_and_a_hash_passes_as_it_is` (a fake resolver: asked once for a ref, never for a hash, a refusal naming the ref, a short answer refused, the label, the URL escaping, and the dispatch arguments carrying the hash), `a_dry_run_says_it_would_resolve_against_and_asks_nothing`, `a_dispatch_takes_a_queued_limit_and_finds_the_runs_that_waited_past_it`.
- `ab::tests::each_round_shows_each_sides_wall_time_and_each_side_its_median`, `ab::tests::the_wall_times_are_kept_and_read_back_whole`.
- `branch::tests::only_a_record_is_committed_and_its_path_is_read_from_the_top`, `a_request_is_read_and_what_it_lacks_is_said`, `the_commits_after_the_target_are_a_straight_line_or_refused`, `the_worktrees_branches_and_a_commits_parts_are_read` (worktree list, a raw commit with a signature, `cat-file --batch-check` and `--batch` output, `merge-tree` output, the backup name).

System (tier 3, they start git), in real repositories made with one `git fast-import`, reading none of the machine's git configuration (`GIT_CONFIG_NOSYSTEM`, a `GIT_CONFIG_GLOBAL` that is not there, no identity variables: the identity is the repository's own):

- `branch::tests::sys_records_reach_a_branch_not_checked_out_and_a_fold_rewrites_only_the_current_one` (both systems, 29 git starts): records to a branch not checked out with HEAD, the index and the files untouched byte for byte; staged changes folded into an earlier commit; the backup.
- `branch::tests::sys_records_are_committed_to_a_branch_not_checked_out_and_the_worktree_is_left_alone` (Unix only, 41 git starts): every other ref, the index and the working tree as they were; each refusal (the branch checked out here and in a linked worktree, a branch not there, a file not there, nothing changed).
- `branch::tests::sys_fold_and_to_end_rewrite_only_the_current_branch_keep_a_backup_and_refuse_a_conflict` (Unix only, 107 git starts): to-end on a four-commit history (the tip's tree is the old one; the backup holds the old tip; the commit before and another branch untouched; author, date and message kept; the repository's identity as committer), fold `--last` (with a second backup in the same second), fold of staged changes, the refusals, a conflict (every ref, the index and the files as they were, no backup), a merge in the range.

## Measurements

At the desk, Linux, `rx -- cargo xtask test -p xtask` with the machine otherwise quiet (dev profile):

| Test | Tier | Time | git starts |
|---|---|---|---|
| `branch::tests::sys_records_reach_a_branch_not_checked_out_and_a_fold_rewrites_only_the_current_one` (both systems) | 3 | 0.053 s | 29 |
| `branch::tests::sys_records_are_committed_to_a_branch_not_checked_out_and_the_worktree_is_left_alone` (Unix) | 3 | 0.062 s | 41 |
| `branch::tests::sys_fold_and_to_end_rewrite_only_the_current_branch_keep_a_backup_and_refuse_a_conflict` (Unix) | 3 | 0.137 s | 107 |
| the eleven new unit tests (`branch`, `runs`, `try_run`, `try_result`, `ab`) | 1 | 13 to 41 ms each, as the crate's others under nextest | 0 |

- The git starts were counted with `strace -f -e trace=execve` on each test (successful `execve` of `git`, its own `unpack-objects` child included).
- **Windows, estimated, not measured:** at tooling 9's fitted 0.054 s a git start, the one test that runs there is about 1.6 s of `xtask`'s tier 3. Before the gits were cut (`cat-file --batch-check` for every name, `cat-file --batch` for every commit, one `rev-list` for the range, one `update-ref --stdin` transaction, the repositories made by one `git fast-import`, the test's state read from files where it can be), the two tests made about 250 starts, about 13 s on Windows.
- `rx -- cargo xtask ci`: every step ok (49.2 s at the desk). Its tier check warned that tier 1 was over its budget: a side run (another session's `mutate`) was loading the desktop, which `rx` warned of at the start; `xtask`'s unit tier there read 7.4 s.
- `rx check`: ok. Windows clippy (`cargo xwin clippy --target x86_64-pc-windows-msvc -p xtask --all-targets --locked -D warnings`): clean.

## Breaks

`mutations/tooling-10-runs.breaks`, 29 breaks; `rx --side -- cargo xtask mutate tooling-10-runs --jobs 2`, one run, exit 0: **29 of 29 did what the list expects**, 28 caught by the tests they name, and `rewrite-fold-reads-the-index-in-place` survived as listed (`expect: survives`: `write-tree` on the real index gives the same tree; what it may write there, a cache of trees, cannot be told from what `git diff` writes when it refreshes the index).

| Item | Breaks | Caught by |
|---|---|---|
| A | `dispatch-sends-a-ref-unresolved`, `dispatch-takes-any-answer-for-a-hash`, `dispatch-sends-the-name-it-resolved`, `ab-label-is-the-whole-hash`, `dry-run-says-nothing-of-resolving` | `try_run::tests::against_is_resolved_…`, `a_dry_run_says_it_would_resolve_…` |
| B | `queued-in-progress-with-a-job-started`, `queued-at-the-limit-is-past-it`, `queued-a-waiting-job-counts-as-started`, `try-result-leaves-out-the-queued-line`, `try-result-has-no-queued-limit-alias`, `dispatch-has-no-queued-limit-alias`, `dispatch-looks-at-every-run-of-the-branch` | `runs::tests::a_run_that_sat_queued_…`, `try_result::tests::an_unfinished_run_…`, `a_request_is_read_…`, `try_run::tests::a_dispatch_takes_a_queued_limit_…` |
| C | `walls-kept-with-the-sides-swapped`, `walls-median-takes-a-failed-side`, `walls-first-side-of-the-wrong-round`, `try-result-prints-no-wall-times` | `ab::tests::the_wall_times_…`, `each_round_shows_…`, `try_result::tests::an_ab_job_prints_…` |
| D | `records-takes-any-file`, `records-to-a-branch-checked-out`, `records-written-into-the-worktrees-index`, `records-commit-that-changes-nothing`, `rewrite-a-published-commit`, `rewrite-over-changes-not-committed`, `rewrite-backup-holds-the-new-tip`, `rewrite-loses-the-authors-date`, `rewrite-keeps-only-the-subject`, `rewrite-takes-a-conflict-for-a-clean-merge`, `rewrite-through-a-merge`, `rewrite-moves-the-commit-to-the-wrong-place`, `rewrite-fold-reads-the-index-in-place` (survives, as listed) | the `branch::tests` named in each |

## Stress

- `rx --side -- cargo xtask stress -p xtask --count 50`: 717 tests, 50 times each (seeds 0 to 49), every test passed every time (37 s).
- `rx --side -- cargo xtask stress -p xtask --count 50 --cores 2 --load 6`: the same, every test passed every time (249 s).

## Open, or for the coordinator to decide

- **Two of the three git tests run on Unix only.** Tooling 9 fitted a git start at 0.054 s on a hosted Windows runner. All three on Windows would be about 177 starts, about 9.5 s added to `xtask`'s tier 3 there (about 12 s now). What runs on Windows is the one test of both commands' paths (29 starts, about 1.6 s estimated). What the two Unix-only tests add (refusals, a conflict, a merge, `--last`, to-end) is git's own logic with hashes, the same on every system. One `#[cfg(unix)]` each turns them back on, if the coordinator wants them on Windows anyway. The Windows time is an estimate: it has not run on a Windows runner (nothing was pushed).
- **`try --dispatch` cannot see its own run sit queued**: it checks right after the dispatch, when the run is seconds old. It names the branch's earlier runs that have waited. Watching its own run would mean waiting at the desk for ten minutes; `try-result RUN` (and the owner's `ci-wait`) say it later.
- **Not caught by a test, by design:** the compare-and-swap of both `update-ref`s (a race, with no test that moves the branch between the read and the move), and the check that the new tip's tree is the one it must be (a second guard: no clean merge sequence in a test ends elsewhere). Neither has a break in the list. The break of the index copy in `fold` is listed `expect: survives`, with the reason.
- A commit made empty by a fold (its change already in the folded commit) is kept as an empty commit, not dropped: the rewrite never loses a commit.
- The backup refs (`refs/xtask-backup/…`) are not pruned by anything; `git for-each-ref refs/xtask-backup` lists them, `git update-ref -d` removes one.
- The first `try --dispatch … --against` on the runners is the one dispatched after this merges (the workflow must be on `main`); the upload of `wall-times.json` is tried by the first A/B run dispatched on a branch that has this `try.yml`.


## The review

An independent review of the branch (its experiments: the coordinator's `target/coordinator/review3/exp1.sh` to `exp6.sh`, run against a shim built from `branch.rs`). Each finding fixed with a test and a break, in commits `68e4a0e4`, `2aaf12eb` and `8f06e088`.

| # | Finding | Fix | Test | Break |
|---|---|---|---|---|
| 1 | `fold` of staged changes printed `git reset --keep <backup>` as its undo, which takes the folded fix out of the index and the files (exp5). | `undo(kind, backup)`: `git reset --soft <backup>` after a fold of the staged changes (the fix is staged again), `--keep` after `to-end` and `fold --last`. USAGE and the docs say so. | `branch::tests::what_each_command_prints_and_gives_git_is_what_it_must_be` | `fold-undo-discards-the-staged-fix` |
| 2, 13 | `commit-records` moved a branch another worktree was rebasing or bisecting (`worktree list` shows it detached; exp1, exp3); `fold`/`to-end` would rewrite a branch checked out twice (`worktree add --force`). | `worktree list` is no longer asked: each worktree's git directory is read as files (the common directory and `worktrees/*`): `HEAD`, `rebase-merge/head-name`, `rebase-apply/head-name`, `BISECT_START`, as `git branch -f` reads them, refused as "`B` is used by worktree at … (checked out / being rebased / being bisected there)". The rewrite checks every worktree but its own (`--absolute-git-dir`). The common directory comes in the `rev-parse` the commands already made, so no git start is added. | `a_branch_a_worktree_has_checked_out_rebases_or_bisects_is_in_use` (unit, on a git directory of files: rebase, bisect, checked out, a worktree's own); the Unix records test with a worktree mid-rebase (`rebase-merge/head-name` written as git writes it); the Unix rewrite test with `worktree add --force` | `records-to-a-branch-being-rebased`, `records-to-a-branch-checked-out` (moved), `rewrite-a-branch-checked-out-twice` |
| 3 | A record that is a link committed its target's contents, from anywhere (exp3). | `regular_inside`: `symlink_metadata` must say a regular file, and the canonical path must be inside the canonical top (a folder that is a link leading out). | the Unix records test: a link to a file outside, and a path through a folder that links out | `records-follows-a-link`, `records-through-a-folder-that-leads-out` |
| 4 | `commit-records` replaced the branch's whole file without a word (exp3). | For each record, its blob on the branch and on `HEAD` are looked up in the same `cat-file --batch-check`; when they differ, refused with both short hashes and the way on (`git show B:FILE > FILE`, make the record on it, run again). Neither having it is fine. | `what_each_command_prints_…` (the text, none on one side); the Unix records test (a diverged branch; and a second run after the first commit) | `records-written-over-another-version` |
| 5 | The final tree check was untested (exp2 case B, exp4); the `update-ref` old values too; the refusal said little. | The Unix rewrite test stages the deletion of the file C adds and folds it into A: refused, nothing moved. `move_transaction` builds the transaction text, unit-tested. `tree_refusal` says which tree it would end at and which it must, what that means, and what to do. | Unix rewrite test; `what_each_command_prints_…` | `rewrite-without-the-tree-check`, `rewrite-moves-the-branch-from-wherever-it-is`, `rewrite-backup-holds-the-new-tip` (moved) |
| 6 | Reports labelled the base by a hash in place of `main`. | A new workflow input `label` (passed to `try` as `--label` only when set, like `capture`), set by the dispatch to the ref as typed; `ab --label`/`perf-ab --label` use it, so `try-result` and `ci-times --ab` say `main` and `-f against=main`. The dispatch prints `against main is <sha12> (resolved at dispatch); the report calls it main`. `--label` without `--against` is refused. | `try_run::tests::against_is_resolved_to_its_commit_at_dispatch_and_a_hash_passes_as_it_is` (the dispatch arguments, the runner's `ab`/`perf-ab` command, a label given is kept) | `dispatch-labels-the-base-by-its-hash`, `ab-ignores-the-label` |
| 7 | `step-24~1` as a branch gave a misleading error. | `git check-ref-format --branch` first: "`step-24~1` is not a branch's name". | Unix records test | `records-takes-a-name-that-is-no-branchs` |
| 10 | The GitHub lookup parsed JSON. | `-H 'Accept: application/vnd.github.sha'`: the hash alone. | (network: not tested; `resolve_against` still checks the answer is a full hash) | none |
| 11 | A broken `wall-times.json` failed all of `try-result`. | `walls_text`: a note in its place. | `try_result::tests::a_wall_times_file_that_cannot_be_read_is_a_note_and_not_a_failure` (and the rest of the job still read) | `try-result-fails-on-bad-wall-times` |
| 14 | Records were all of `docs/**/*.md`. | `docs/steps/**/*.md`, `docs/outstanding/*.md`, `mutations/*.breaks` (a step's own page needs a folder: `docs/steps/README.md` is not a record). Docs, AGENTS.md, CONTRIBUTING.md and USAGE updated. | `only_a_record_is_committed_…` | `records-takes-any-page-of-the-docs` |

**Left, as the coordinator decided:**

- **8**, `:/text` names (a commit named by a message search): not handled specially; they resolve as git resolves them.
- **9**, file names that differ only by case on Windows: not handled.
- **12**, the pipe deadlock with about 2,000 commits: `cat-file --batch` is fed its whole input before its output is read, and a pipe holds 64 KiB. Not fixed; a branch of 2,000 commits after the one rewritten is not this command's case.

**What the fixes changed:**

- (Undone by the second round, below:) a second `commit-records` of the same file, after the first commit, was refused: the branch had the new copy and `HEAD` the old.
- A dispatch with `--commit` of a commit from before this step, and `--against`, fails at the job's start with "unknown flag" for `--label`, as for any input added since a commit. Documented in the input's row.

**Git starts after the fixes** (strace, as before): the test on both systems 29 (unchanged: `check-ref-format` added, `worktree list` gone); the Unix records test 62; the Unix rewrite test 130.

**`cargo xtask ci` after the fixes:** with rx's own `TMPDIR` it failed, twice, on `cineris-sandbox::it confined::sys_with_the_network_off_no_socket_reaches_anything`: "path must be shorter than SUN_LEN". The test binds a Unix socket under `Scratch::outside`, in `TMPDIR`, which rx sets to `/mnt/rxtmp/<worktree>.<pid>/tmp`. This worktree's name is `agent-a6438b4015a52c40b` and the pid now has seven digits, which takes the socket's path past 108 bytes. That crate is untouched here, and the first `ci` of this branch passed it. With `rx -- env TMPDIR=/mnt/rxtmp/t10r cargo xtask ci`, every step passed. The coordinator's worktree name is shorter. Worth a note for rx: a long worktree name can push a socket path past its limit.

**Breaks after the fixes:** `rx --side -- cargo xtask mutate tooling-10-runs --jobs 2`, one run, exit 0: **42 of 42 did what the list expects**. 41 were caught by the tests they name, and `rewrite-fold-reads-the-index-in-place` survived, as its list entry expects.

**Stress after the fixes:** `rx --side -- env TMPDIR=/mnt/rxtmp/t10s cargo xtask stress -p xtask --count 50 --cores 2 --load 6`: 720 tests, 50 times each (seeds 0 to 49), every test passed every time (278 s). The short `TMPDIR` is there because of the socket path above.


### The second round

The second review (c8539f82..8f06e088; its experiments `review3/exp7.sh` cases 1 to 5, and `exp3.sh`) found that the fix for finding 4 broke `commit-records`' main use, and two new gaps. Fixed in `7d3071aa`, `2f722979` and `d16c8ddb`, each with a test and a break.

| Finding | Fix | Test | Break(s) |
|---|---|---|---|
| A. The check compared the branch's copy with `HEAD`'s, which never changes, so the main use (the branch has `ci.md`, the worktree's `HEAD` has not) and its own advice (`git show B:FILE > FILE`, add a section, run again) were refused for ever (exp7 1–2). | For each file: taken when the branch has no copy, when the branch's copy is `HEAD`'s, or when the branch's copy's lines are all in the working file in their order (`built_on`: a subsequence of lines, `\r\n` read as `\n`); otherwise refused, with the branch's blob, `git show B:FILE > FILE`, and `--replace`. `--replace` (new) takes it knowingly. The branch's copies are read in one `cat-file --batch`, only when one is not `HEAD`'s. A second run after the first commit now works (the file has the branch's lines, and more). | Unit: `built_on` (equal, a section added, CRLF, out of order, a line dropped) and the decision for each case. Unix records test: a second run with a section added committed; the advice followed (the branch has a file `HEAD` has not, made from its copy plus a section) committed; a file that dropped the branch's lines refused, then committed with `--replace`. | `records-written-over-another-version` (moved), `records-built-on-ignores-the-order`, `records-replace-is-ignored` |
| B. `--branch '@{-1}'` (or `-`) was expanded by `check-ref-format --branch`, and moved a branch the user did not name (exp7 3). | Refused when the name `check-ref-format` gives back is not the one given: "`@{-1}` stands for `previous`: name the branch itself". | Unix records test (`@{-1}` after a switch) | `records-takes-a-name-git-expands` |
| C. A folder link inside the repository was followed: `docs/steps/99 -> ../../xtask` published `xtask/notes.md` (exp7 4). | Every part of the path, from the top down, is read with `symlink_metadata`, and a link anywhere is refused, wherever it leads, as `git add` refuses a path beyond one. The canonical-path check is gone (it let an inside link through). | Unix records test: a link to a file outside, a folder link out, a folder link inside | `records-follows-a-link` (moved), `records-through-a-folder-that-is-a-link` (renamed) |
| The compare-and-swap of `commit-records` had no test: dropping `&old` survived. | `records_transaction(branch, new, old)` builds the `update-ref --stdin` text, unit-tested. | `what_each_command_prints_and_gives_git_is_what_it_must_be` | `records-moves-the-branch-from-wherever-it-is` |

**Found while running the breaks:** the first run of `records-replace-is-ignored` was NOT CAUGHT by the Unix records test it named (caught only by the unit test). `commit_records` skipped reading the branch's copies under `--replace`, and an empty text reads as built on: a second path for `--replace` that hid the check's own. `d16c8ddb` reads them under `--replace` too (one git more, only then), and the break is caught by both tests.

**Left, as the coordinator decided (D):**
- Branches a `rebase --update-refs` will move (`rebase-merge/update-refs`) are not read as in use.
- A worktree whose folder was deleted and not pruned still blocks its branch: conservative, and what git does.

**Git starts:** the test on both systems still 29; the Unix records test 103 (it now commits four times and tries three links); the Unix rewrite test 130.

**`rx -- env TMPDIR=/mnt/rxtmp/t10r cargo xtask ci`:** every step ok (the socket-path failure with rx's own `TMPDIR` is fixed in PR #84, not on this base).

**Breaks:** `rx --side -- cargo xtask mutate tooling-10-runs --jobs 2`: first run exit 1, 45 of 46 (the flag above); after `d16c8ddb`, **46 of 46 did what the list expects**, 45 caught by the tests they name and `rewrite-fold-reads-the-index-in-place` survived, as listed.

**Stress:** `rx --side -- env TMPDIR=/mnt/rxtmp/t10s cargo xtask stress -p xtask --count 50 --cores 2 --load 6`: 720 tests, 50 times each (seeds 0 to 49), every test passed every time (261 s).
