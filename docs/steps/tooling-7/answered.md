# Answered in the seventh tooling step, on 2026-11-16

This step is not a numbered step of the build plan: it built the seventh round of developer tooling, from what wave 10's five sessions asked for. Its records are here, in a folder named for the job.

## Windows' integration tier: where its time goes

**The profile** (`try` run 43778010011, `main` at `bd2e6a1`, tier 2 alone with `-f testkit=profile`, an 11 ms runner): 35.1 s above the start of the processes, spread thin. The slowest 20 tests were a quarter of it.

| Crate | Tests | Above the start | Median test |
|---|---|---|---|
| `cortex-core` | 115 | 8.2 s | 62 ms |
| `cortex-llm` | 269 | 6.1 s | 4 ms (the replays at every cut: 3.1 s in three tests) |
| `cortex-session` | 57 | 4.7 s | 11 ms (the durability tests: 4.0 s in ten) |
| `cortex-tools` | 185 | 4.3 s | 9 ms |
| `cortex-agent` | 244 | 3.5 s | 9 ms |
| `cortex-index` | 53 | 2.2 s | 18 ms |

- **The slowest tests:** `durability::an_entry_over_the_line_limit_is_refused_and_nothing_is_lost` 1.8 s (a 66 MiB line serialised unoptimised), `openai_chat::decode::at_every_cut::deepseek` 1.5 s, `durability::a_log_cut_at_every_byte_of_a_last_response_opens_cleanly` 1.2 s, the other two servers' replays 0.9 and 0.75 s, `approvals::an_approval_for_a_change_to_a_file_shows_the_change_within_a_bound` 0.6 s (a 600 kB write diffed).
- **`cortex-core`'s time is the session's set-up on Windows,** not the tests' work: from the profiles, opening a session took 19.5 ms on average (122 of them, 2.4 s), a turn 15 ms; the same test opens a session in 1.4 ms at the desk. Timed inside the core on a scratch branch (`try` 43780823181, one test at a time): `session/new` took 20.1 ms on a Windows runner against 3.3 ms on a Linux one, and 9.2 ms of the 20 was 27 constructions of the path resolver, 0.34 ms each (0.64 ms for all 27 on Linux). 20 of them, 6.5 ms, are `engine_parts`'s `resolve_dir`, which builds a resolver for a path that does not exist yet and, when that fails, one for each parent in turn, each walking the path from its root. The fix (the deepest ancestor that exists, found with a plain `metadata`, and one resolver there) is product code in `cortex-core`, where step 23 is working: it waits for step 23's first pull request.
- **Windows Defender is not the cost:** its real-time scanning is off on the hosted runners, with `C:\` and `D:\` excluded (read on a runner, `try` 43779123608).

## The owner's decision: Windows' tiers 1 and 2 three at a time

No test change found takes 9 s off without testing less: the time is a few milliseconds of the operating system's in each of a thousand tests. Four ways were measured, each against `main` on one runner (`try` with `-f against=main -f count=3`):

| Change | Run | Runner | Tier 2, `main` → change | Tier 1 | Wall time of the tiers 1 and 2 run | At 11 ms, tier 2 about |
|---|---|---|---|---|---|---|
| Tiers 1 and 2 three at a time | 43778700279 | 6 ms | 23.4 → 19.0 s (−19%) | 11.3 → 7.9 s | 13.6 → 15.1 s (+1.5 s) | 35 s |
| Tiers 1 and 2 two at a time | 43778703107 | 8 ms | 38.3 → 23.6 s (−38%) | 19.4 → 9.5 s | 20.8 → 28.8 s (+8 s) | 27 s |
| `cortex-llm` at `opt-level = 1` in the `ci` build | 43778840384 | 11 ms | 43.1 → 38.8 s (−10%) | 19.4 → 17.7 s | unchanged; the build +9.3 s warm and +11.5 s cold on Windows, +5.7 and +6.2 s on Linux (43780885900) | 39 s |
| Three at a time and `cortex-llm` optimised | 43780837934 | 10 ms | 39.6 → 31.9 s (−19%) | 18.3 → 14.0 s | 22.0 → 25.3 s (+2.6 s), and the build | 34 s |

`main` itself read its integration tier at 43.1 s on the 11 ms runner of the third row: over its 40 s budget.

The coordinator held two at a time, which reached the 30 s target, because it made the Windows job about 8 s longer to lower a figure that falls as the tests stop slowing each other. (Reversed by the owner on 2026-11-16, after the ninth tooling step measured again: two at a time on Windows; [testing-and-ci.md](../../testing-and-ci.md), "Each tier is measured under its own load".) **The owner decided (2026-11-16) on three at a time on Windows, Linux unchanged,** with the target "under the 40 s budget at 11 ms, with room" in place of 30 s: tier 2 about 35 s at 11 ms, for about 1.5 s of the job's time. **`cortex-llm` at `opt-level = 1`: no.** Every CI job builds every workspace crate (the cache holds only third-party crates), so every pull request would pay its 6 to 11 s of build for less than 2 s of the test run's wall time (this settles the open item `cortex-llm-optimised-in-the-ci-build`, deleted). The session-open fix is this step's, once step 23's first pull request has merged.

## What it came to

The step's branch against `main` (`9baacb6`, with step 23's first part) on one Windows runner at 9 to 10 ms, three rounds (`try` 43787460318):

| Tier | `main` | The branch | Change | Budget |
|---|---|---|---|---|
| 1, unit | 22.0 s (1,857 tests) | 15.8 s (2,028) | −6.1 s | 20 s |
| 2, integration | 44.9 s (1,235) | 33.1 s (1,238) | −11.8 s | 40 s |
| 3, system | 65.7 s (139) | 66.4 s (139) | +0.8 s | 90 s |

`main`'s own unit and integration tiers were over their budgets on that runner. The run of tiers 1 and 2 took 23.4, 23.4 and 28.6 s on `main` and 26.1, 27.4 and 26.1 s on the branch, which has 174 more tests (the hostile names' and the tools'). By crate in tier 2: `cortex-core` 11.1 → 7.0 s (three at a time and the set-up's fix), `cortex-llm` 7.4 → 5.5, `cortex-session` 5.4 → 4.2, `cortex-agent` 4.3 → 3.3. The slowest tests are the ones the profile found, each a fifth to a third quicker; one test moved onto the hostile names reads more of them (`read::a_file_is_read_by_the_escaped_name_it_is_shown_by`, 0.02 → 0.23 s).

## The session's set-up on Windows, after the fix

`resolve_dir` now finds the deepest folder that exists with a `metadata` call a level and makes one resolver there (the owner's go-ahead, after step 23's first part merged). Timed inside the core as before, one test at a time (`try` 43789358331, a scratch branch of the step's head with the marks): **13 resolvers where there were 27,** 2.1 to 2.9 ms of them where there were 9.2 ms, and `session/new` 9.1 to 11.5 ms where `main` read 20.1 ms (`try` 43780823181). The two runs were on two runners, and this one made each resolver in 0.16 to 0.22 ms where the other took 0.34, so part of the difference is the runner. On one runner, `cortex-core`'s tests with and without the fix (`try` 43788709078, five rounds, the step's branch both sides) moved by +1 ms a test at the median (quartiles −5 and +5 ms): the saving there, 14 resolvers at 0.2 ms, is under the noise of a test. The 115 tests' tier-2 time moved by +0.2 s of 5.7.

In one of that run's rounds two of `cortex-core`'s trust tests failed on the side with the fix ("the command's change is in a checkpoint", beside the known [a-base-capture-beside-the-turns-git](../../outstanding/a-base-capture-beside-the-turns-git.md)). Thirty repeats of the trust tests on Windows with the fix (`try` 43789177397) and thirty without (43789436509) all passed.

## What was built, item by item

The work was shared out to seven sub-agents, each in a worktree and with a breaks list of its own, merged at the end with the step's own `mutate --merge` (the session kept the tier-2 work, the settings' ranges, the docs and the review).

| Item | Built | Where it is documented |
|---|---|---|
| An A/B that leaves out a test failed on both sides | `ab --keep-going` compares a round in which both sides failed only by tests, and the same tests; they are left out of every round on both sides and named (`N TEST(S) FAILED ON BOTH SIDES`); a test failed on one side only, missing on one, or beside a run that broke still fails the round. A run that broke keeps its report as `<side>-round-<n>-broke.xml`. | testing-and-ci.md, "Two trees on one runner" |
| `try` on a commit | The input `commit` (40 hex digits, checked in the workflow and by `try`, which also checks the checkout's `HEAD`); `--ref` stays the workflow file. A variant is a commit on one branch, pushed normally. | "A run on the runners without a pull request", "Running an experiment" |
| `mutate` on Windows | Both: `try -f command="mutate …"` (`--minutes 0`, results and logs uploaded) and an `os: windows|linux` key on a break, applied by the pre-flight everywhere and run only there; a run of nothing but the other system's breaks fails. The demonstration break is `tooling-7/nofollow-windows-follows-a-reparse-point`. | "Breaking the code on purpose" |
| `ci --tiers --compare --os windows|both` | Windows' report from a `ci` run, `main` or a `try` run of `test`; `both` in turn; a side with failed tests or a missing tier fails the command. | "A CI run's test times" |
| `ci-times --ab` detail, `--section`, `--heading` | Rounds, crates and tests that moved, the rounds left out and the tests left out; named sections with marks of their own; a dated heading for a page with no marks. A report with failed tests is written into no record. | "A CI run's test times" |
| `cargo xtask try-result RUN` | A `try` run's A/B tables, tiers, stress or perf summary, failed tests and the end of a failed job's log, by OS, from its artifacts. | "A CI run's test times", AGENTS.md |
| `try` and `--dependents`; refusals; dry run | `--dependents` resolved into the crates as `test --dependents` resolves it; every word `try` cannot pass on (another command's options, inputs, what its runs give nextest already) refused before anything is built; `try --dry-run`. | "A run on the runners without a pull request" |
| `try -f capture=no` (asked for by step 23) | `--no-capture` for `test` and `stress`, refused with `against`. | "Running an experiment on the runners" |
| Shared hostile names | `cortex_testkit::hostile`: 13 kinds, the same on every system, with `takes_file_name` and `accepted` asking the file system; the per-OS `BREAK` constants of `cortex-tools` gone; 20 tests moved onto it. No product bug was found by running every line break on Linux. | testing-and-ci.md, "Hostile names" |
| Library points behind `test-hooks` | `cortex_base::hook::point` (nothing when the feature is off: the release assembly is byte for byte the same without the call; one atomic load when on), `Hold` in the base and the test kit, named in the timeout report; `cortex-base`'s atomic replace stops at `BEFORE_THE_RENAME`, and its race test reads the old content at each stop. The test that races a reader against a hundred replaces is kept beside it. | "A point the library stops at" |
| The reviewer's kit | A section of its own; `cortex_testkit::stalled_file`; each technique tied to a test. Settles `a-reviewers-kit`. | "A reviewer's kit" |
| `api-diff` | rustdoc's JSON on the pinned toolchain (`RUSTC_BOOTSTRAP=1`, format 61 only), a shared clone of the other ref; 10.5 s for one crate the first time at the desk, 0.7 s again, 21.8 s and 2.6 s for 19 crates. | "The public API between two trees" |
| `scratch-test --cargo-profile P --ignored` | And `--include-ignored`; an ignored run shows its output, in nextest's `perf` profile. | "A throwaway test" |
| One-off break, `--flagged`, `--merge` | `mutate --break FILE --find … --replace … [--test …]`, kept apart from a list's results; `--flagged` re-runs COULD NOT BE JUDGED and DOES NOT COMPILE; `--merge STEP PART…` merges the parts named. | "Breaking the code on purpose" |
| A pinned corpus of real code | 20,162 `.rs` files of 670 crates, 324 MB (373 MB on disk), fetched by `cargo fetch` from a lock in `corpus/`, named by its hash `285100c9ac0c85f0`, cached in the user's cache folder; `cortex_testkit::corpus::real_code()`; `try` fetches it for `perf -p`. Cold fetch 6.2 s at the desk. `cortex-index`'s real-code measurement reads it. | testing-and-ci.md, "A tree of real code to measure"; performance-budgets.md |
| The blocking pool's rule; the engine's inputs | 02-runtime.md; 06-permissions-and-sandbox.md, "What an answer depends on" | |
| Duration ranges in `settings!` | `[TightenOnly, Now, 100ms..=1m]`, read when the crate is built in the field's unit; generated files unchanged. | 08-configuration.md |
| The shell guard's interpreter heredocs | Rows for what wave 11 and this step saw refused, and the accepted form. | shell-guard.md |
| Item 8 | `the-start-cost-of-a-tier-report` settled (a warning above 8 ms on Linux and 15 ms on Windows); `the-mutate-lock-and-a-forked-child` settled (a 200 ms retry, now shared with `scratch-test` and `api-diff`, whose lock test had failed the same way). `caught-by-names-checked-only-at-the-desk` left: the owner's choice, and nothing here makes it cheaper. | |

## The independent review

One full review (Opus 5.5, with two helpers, none having seen the session's reasoning) of the whole diff. What it found, and what was done:

| Finding | Done |
|---|---|
| `ci-times --ab` and `try-result` read an A/B's rounds by a rule of their own: a round whose run broke after tiers 1 and 2 was compared (its tier 3 read faster), and a round in which the same tests failed on both sides, which `ab` now compares, was left out, and `try-result` called a job that had succeeded failed. | Fixed: `ab` keeps a broken run's report as `-broke.xml`, and the readers apply `ab`'s rule. Tests and six breaks. |
| `mutate --merge STEP` took every `STEP-*.breaks` for a part: `--merge step-12` would have swallowed and deleted `step-12-fixes`. | Fixed: the parts are named. Test and two breaks. |
| `ci --tiers --compare`, now fed `try` runs, read a side with failed tests or a missing tier as a measurement. | Fixed, with its twin in `ci-times --write`. Tests and three breaks. |
| `try` passed on `--workspace`, `--locked`, `--no-tests` and `-Pfast`, which nextest refuses after both trees of an A/B are built. | Fixed: refused before the build. Tests and two breaks. |
| A `mutate` run of nothing but the other system's breaks exited 0. | Fixed: it fails. Test and break. |
| One migrated hostile-names test sorted both sides, losing main's check of a search's order. | Fixed: the order is held again. |
| `stalled_file` frees a writer, not a reader; `let_one_go` frees one thread in no order; six messages had lost their line continuations; a doc and the workflow's example were stale. | Corrected. |
| Tests that would pass a broken implementation (a hold's wait for another, `ab`'s glue, `api-diff`'s report held only by an ignored test, `os:` breaks' names, the range bound's order), and rough edges that let no comparison pass wrongly. | Left open: [tests-the-seventh-tooling-step-left-thin](../../outstanding/tests-the-seventh-tooling-step-left-thin.md), [rough-edges-of-the-seventh-tooling-steps-commands](../../outstanding/rough-edges-of-the-seventh-tooling-steps-commands.md). |

## Breaks and stress

- **`cargo xtask mutate tooling-7`,** the 264 breaks of the step's list (its own, the seven parts merged, and the review's fixes), on the desk's side slot with one job (a first run with two found the disk too busy and stopped, as it should): 263 did what the list expects (two of them `expect: survives`, caught only by an ignored test of `api-diff`, which `mutate` never runs), and one was left to a Windows run. That one, `nofollow-windows-follows-a-reparse-point`, was run on a Windows runner (`try -f command="mutate tooling-7/nofollow-windows-follows-a-reparse-point"`, run 43787464849) and caught by the three Windows-only tests of `nofollow`.
- **Stress** on the desk's side slot: `cortex-base`, `cortex-testkit`, `cortex-tools`, `cortex-index`, `cortex-policy`, `cortex-sandbox`, `cortex-agent`, `cortex-term`, `cortex-config` and `xtask` 50 times, before the review: the failures were 28 timeouts at two seeds, within moments of each other, while the disk waited 164 s of the run's 359 (the stall report said so), and `cortex-sandbox`'s `tree::sys_a_guard_that_died_is_started_again` (not touched here) 3 times in 50. The atomic replace's two race tests were then repeated 300 times alone and passed every time. After the review: `cortex-core`, `cortex-session` and `xtask` 50 times, 873 tests: every one every time but a test of `mutate`'s jobs this step did not change, once (a-mutate-jobs-test-that-once-finished-every-piece, settled by the eighth tooling step ([its record](../tooling-8/answered.md))).
- **Linux's integration tier on the pull request's first CI run** read 44.9 s against `main`'s 22.6 s, nearly all of it tests that write files (`cortex-session`'s durability tests 0.4 → 5.4 s, crates this step did not touch). The same three crates against `main` on one Linux runner (`try` 43789085254, three rounds) read 4.1 s and 4.1 s: the runner's disk.

## Untested on Windows

What only a hosted runner could show, and was not run: `api-diff` and its ignored real-rustdoc test there; the corpus's fetch, its hash and its file dates on a Windows runner (`perf -p` with the corpus); `try-result`, `ci-times --ab` and `ci --tiers --compare --os both` read this step's real runs from the desk (the A/B, the `mutate` run and the pull request's CI run, Linux's table then Windows'). The hostile names' file-system probe ran on Windows in the A/B's tests (the moved tests passed there).
