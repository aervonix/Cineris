# Answered in the Windows tier 2 fix, on 2026-11-27

A fix outside the numbered steps, given by the maintainers: Windows' integration tier (tier 2: 40 s, a warning above it, a failure at 60 s) read 59 to 62 s on slow hosted runners (about 8 ms a process start) in the new `test-windows-tiers` job, so a pull request that landed on one could fail for nothing it did; `main` itself read 148% of the budget there (run 45955638461). The brief: bring tier 2 to at least 10 s under its failure line on an 8 ms runner without changing the budget, and find out whether the Windows split's job layout costs time. The CI times are in [ci.md](ci.md).

## Where tier 2's time was

`main` at `51f6eeb7`, the Windows split's head, in `main`'s own two `ci` runs of that commit, which landed on runners of the two speeds (read from each run's `junit-windows-tiers` report; each crate's time above its run's start):

| Crate | Tests | 8 ms runner (run 45955638461) | 6 ms runner (run 45955615844) | Linux, 4 ms (run 45955638461) |
|---|---|---|---|---|
| `cineris-core` | 370 | 25.54 s | 18.54 s | 20.23 s |
| `cineris-tui` | 51 | 7.22 s | 4.96 s | 5.01 s |
| `cineris-llm` | 293 | 4.72 s | 3.35 s | 2.14 s |
| `cineris-tools` | 188 | 3.80 s | 2.69 s | 1.06 s |
| `cineris-session` | 61 | 3.30 s | 2.45 s | 1.92 s |
| `cineris-agent` | 337 | 3.09 s | 2.33 s | 1.27 s |
| `cineris-rpc` | 32 | 2.07 s | 1.50 s | 2.14 s |
| `cineris-policy` | 124 | 2.05 s | 1.35 s | 1.41 s |
| `cineris-knowledge` | 57 | 1.82 s | 1.33 s | 4.47 s |
| `cineris-index` | 53 | 1.52 s | 1.15 s | 1.07 s |
| `cineris-config` | 95 | 1.21 s | 0.90 s | 0.82 s |
| `cineris-ext` | 88 | 1.18 s | 0.96 s | 0.56 s |
| `cineris-term` | 10 | 1.09 s | 0.87 s | 0.68 s |
| `cineris-stats`, `cineris-base`, `cineris-sandbox` | 98 | 0.65 s | 0.43 s | 3.08 s |
| **Tier 2** | **1,857** | **59.3 s** | **42.8 s** | **45.9 s** |

The slowest tests on the 8 ms runner: `cineris-tui`'s `memory::a_long_reasoning_is_held_to_its_bound_in_each_presenter` 4.05 s and `memory::a_finished_reasoning_is_held_once_in_each_presenter` 1.92 s; `cineris-rpc`'s `conformance::every_scenario_is_the_same_in_process_and_over_the_wire` 1.35 s; `cineris-session`'s `durability::an_entry_over_the_line_limit_is_refused_and_nothing_is_lost` 1.14 s and `durability::a_log_cut_at_every_byte_of_a_last_response_opens_cleanly` 0.84 s; `cineris-core`'s three folded `extensions_hooks` tests 0.94, 0.78 and 0.64 s; `cineris-llm`'s `openai_chat::decode::at_every_cut::deepseek` 0.82 s. On the 6 ms runner the same tests in the same order, each about 0.7 times as long.

**What moved since CI budgets II.** `main`'s last one-job run before step 25's fixes (`5eda620`, run 45931692729, 8 ms) read tier 2 at 56.8 s, and the one after them (`e3c0175`, run 45951053644, 8 ms) at 61.0 s: `cineris-tui` went from 1.30 to 6.88 s, the two `memory::` tests step 25's fixes added (#134). Nothing else moved by more than 0.7 s. The split (#135) then read 59.3 s on another 8 ms runner: no sign of a cost of its own (below).

**Where a core test's time goes, on a runner.** `cineris-core`'s 370 integration tests with the test kit's profile on a Windows runner in CI's build (`try` 45971337551, `-f testkit=profile`): a session's connection and open 3.65 s in all (295 of them, 12.4 ms each, against 3.3 ms on the second machine), turns 4.0 s (about 11 ms each), the waits for a turn to end 3.06 s, approvals 0.72 s, a core's start 0.57 s, and the tests' own work outside the rig's phases 7.1 s (a median of 7.7 ms a test; the three folded `extensions_hooks` tests 0.4 to 0.55 s each, twelve worlds of their own apiece). Linux's runner reads `cineris-core`'s tier 2 at 20.2 s, more than a 6 ms Windows runner: the core's tests are CPU in CI's unoptimised build on both systems, more than Windows' file calls.

**No tier 2 test starts a process.** The test hooks abort any test not named `sys_` or `perf_` that starts one ([testing-and-ci.md](../../testing-and-ci.md), "The tiers"), so a change to what each command's start costs on Windows moves tier 3 alone.

## Does the split's layout cost time

**Built to answer it:** `cargo xtask ab --job --from-archive base|head` ([testing-and-ci.md](../../testing-and-ci.md), "A whole job two ways on one runner"): one side runs as the split Windows jobs do (the tests' nextest archive built, the build directory set aside, the archive extracted over the tree, the test jobs' runs from it), the other builds and runs in one place, as the job did before the split. `try`'s `test --job` passes it on.

**Measured:** `main` at `0ce319cc` against itself with the tool on top (no test differs), six rounds on one Windows runner, the first two cold and one in each order (`try` 45974262733, `--keep-build`):

| Phase, median (paired change) | One job | From the archive |
|---|---|---|
| Build, cold | 162.9 s | 156.7 s (−6.1 s) |
| Extraction | | 1.0 s |
| Tier 1, cold / warm | 22.8 / 20.1 s | 21.1 / 19.3 s (−1.6 / −1.0 s) |
| **Tier 2, cold / warm** | **48.2 / 45.3 s** | **44.9 / 43.8 s (−3.2 / −0.9 s)** |
| Tier 3, cold / warm | 87.0 / 73.7 s | 80.3 / 73.4 s (−6.8 / +1.3 s) |

Running from the archive costs the tiers nothing on one runner: every change is inside the spread of a side's own rounds (tier 2 43.2 to 48.6 s on the one-job side, 42.7 to 46.1 s on the archive's). The split's run that read tier 3 at 119 s against 82 s was, by this, the runners and not the layout. What this cannot reproduce is two runners: a cost that only a freshly started runner has, in its first minutes, is not in it.

## What was changed

1. **The TUI's memory tests read both turn counts in one session** (`crates/cineris-tui/tests/it/memory.rs`). Each of the two tests opened a session for four turns and another for eight, with a long reasoning and without, for each presenter: twelve turns that draw a finished reasoning, which is about 70 ms a turn in CI's build (the markdown reader's paragraph lines, about 200 µs a line unoptimised; the same code in a release build is not slow). Now one session takes four turns, reads the heap after a change of width (which drops the full screen's rows, as the old form's last frame did), takes four more and reads it again after another change of width: eight turns. The figures the tests read are the same (8,189 bytes a turn held for 16 KiB of reasoning, 6,159 for 6 KiB, in both presenters), and both breaks of step 25's fixes that name them are still caught, by the same margins (`reasoning-copied-into-its-summary`: 14,351 bytes a turn against a limit of 9,216; `reasoning-kept-without-a-bound`: 16,399 against 12,288; run by hand on the second machine). On one Windows runner, five rounds a side (`try` 45979883356): **tier 2 61.1 to 57.9 s (−3.2 s)**, `cineris-tui` 6.79 to 4.96 s; tiers 1 and 3 within their noise (−1.2 and +0.2 s).
2. **Nineteen third-party crates optimised in CI's build** (`Cargo.toml`, `[profile.ci.package.*]`, as `serde_json` and `sha2` already were): the async runtime (`tokio`, `tokio-util`, `mio`), the configuration's parser (`toml`, `toml_parser`, `toml_datetime`, `serde_spanned`, `winnow`), the script engine (`rquickjs`, `rquickjs-core`, `rquickjs-sys`), the text's widths and graphemes (`unicode-width`, `unicode-segmentation`) and the tools' searches, globs and diffs (`memchr`, `aho-corasick`, `globset`, `ignore`, `bstr`, `similar`). The cache serves a third-party crate after its first build, so a warm pull request pays nothing for them. On one Windows runner, five rounds a side (`try` 45979971472): **tier 2 59.2 to 56.0 s (−3.2 s)**, `cineris-core` −0.83 s, `cineris-knowledge` −0.63 s, `cineris-ext` −0.47 s, `cineris-tui` −0.47 s; tier 1 −1.2 s, tier 3 −1.0 s. The cost, from a job A/B of two rounds (`try` 45979975138): a cold build 186.0 to 214.7 s (+28.7 s), which a pull request pays only when the dependency cache misses (a change to `Cargo.lock` or the toolchain).

**Both together**, on one Windows runner at 5 to 6 ms a start, five rounds a side (`try` 45984356087): **tier 2 37.6 to 34.2 s (−3.4 s)**, tier 1 −0.4 s, tier 3 +0.4 s. Scaled by the 8 ms runners' 1.4 times the 6 ms runners' tier 2 (59.3 against 42.8 s for one commit, above), that is about −4.8 s on a slow runner. The run's one failure was the repository's rules test on the head: that branch held the testing page's link to this record before the record was committed (the review's first finding); the comparison is of the other tests. The final tree's tiers are `try --final`'s, in [ci.md](ci.md).

## What was measured and not taken

| Option | Tier 2 on one 8 ms Windows runner | Cost | Run |
|---|---|---|---|
| Every crate of ours at `opt-level = 1` in CI's build | 53.7 to 39.4 s (−14.3 s, five rounds); tier 1 −8.2 s, tier 3 −5.0 s | A cold Windows build 213 to 660 s (+447 s, two rounds): past the five minutes of `build-windows` on its own, and paid by every pull request for the crates it rebuilds | `try` 45970482928, 45970486829 |
| `cineris-tui` alone at `opt-level = 1` | −1.7 s (two rounds) | +31 s of a cold build on a 5 ms runner | `try` 45975998314 |
| `cineris-core` alone at `opt-level = 1` | −1.5 s (two rounds) | +72 s of a cold build on an 8 ms runner | `try` 45976001836 |

Most of what optimising our own code would save is spread across the crates under the core (the agent, the policy, the configuration, the base), which is why the core alone saves little. The owner declined `cineris-llm` at `opt-level = 1` for its build cost in the seventh tooling step ([steps/tooling-7/answered.md](../tooling-7/answered.md)); these figures say the same of every crate of ours.

**The markdown reader in CI's build.** A 16 KiB reasoning takes about 65 ms to draw in CI's build on the second machine, linear in its length, nearly all of it in reading the paragraph's lines, not in laying them out. Making the reader cheaper unoptimised is a change to the product's code for a test build's sake, and was left.

## Review

An independent review of the whole diff before the first push of the branch (a reviewer that had not seen the reasoning; it ran the old form of the memory tests beside the new and both breaks by hand on each):

| # | Finding | Verdict | What was done |
|---|---|---|---|
| 1 | The testing page and `Cargo.toml` linked this record before it was committed | Right | Committed with the change |
| 2 | The testing page said an exception costs a cold run "a few seconds", beside the 29 s of these nineteen; and a change to an exception rebuilds what is built on it, our own crates among them, in the pull request that makes it and in `main`'s first run after it | Right | Said so on the page |
| 3 | This pull request's own `build-windows` is near cold, within its five minutes on a slow runner only by the measured margin | Right; nothing to fix | Said in the pull request; its run's build time is in [ci.md](ci.md) |
| 4 | The archive side's build was put back at the start of its next round, inside that round's timed build, which the other side does not pay | Right | Put back at the end of its round, after the job's time is taken |
| 5 | The page said the archive side runs "the test jobs' runs": it runs neither the system job's start probe nor `ci`'s re-run, and moves no artifact | Right | Said on the page and in `ab --job`'s usage |
| 6 | A run left the archive side's tree with the extracted copy in `target/ci` and the build aside | Right | Gone with the fourth: the last round puts it back |
| 7 | The later turns of the new memory form are drawn after a change of width, which the old form's were not; a break kept only for reasoning finished before a session's first resize would pass | A break of that shape is contrived, and none is in the lists | Left; both listed breaks are caught by the same margins |
| 8 | The unit tests hold the archive side's helpers, not their order in the real side | Right; as every phase of `ab --job` is tested | Left |
| 9 | The test jobs that run from the archive make `target/ci/tmp` for `CARGO_TARGET_TMPDIR`, which cargo gives as `target/tmp` (true since the split) | Right: a CI-profile build has `target/tmp` and no `target/ci/tmp`. No test failed, since every test makes its own folders under it | Both the jobs and the A/B make `target/tmp` (`checks::tests_tmp_dir`), with a test |

## Left for the maintainers

- **The target is not reached on every slow runner** (rule 11). `main`'s tier 2 read 53.7 to 61.1 s on four runners at 8 ms (`try` 45970482928, 45979971472, 45979883356; run 45955638461), so one start speed spans 7 s by itself. This change takes about 3.4 s off on a 6 ms runner, about 4.8 s scaled to an 8 ms one: on the slowest of those runners about 56 s, 4 s from the failure line, and on the quickest about 49 s. The options measured, for the maintainers: our own crates optimised in CI's build (above: −14 s, for a build that no longer fits its job), and Windows' tiers 1 and 2 one at a time in place of two (`tiers-windows = { max-threads = 1 }`; `try` 45987739680, five rounds on one 7 ms runner: **tier 2 60.3 to 53.6 s (−6.7 s)**, tier 1 29.5 to 23.0 s, tier 3 unchanged, for 51 s more of the runs' wall time, 116.8 to 167.9 s, inside `test-windows-tiers`' five minutes). Two at a time is the owner's decision of the ninth tooling step ([testing-and-ci.md](../../testing-and-ci.md), "Each tier is measured under its own load"), so it is theirs to change; with this change, one at a time would put the slowest of those runners at about 49 s. The levers left in the product's code are step 33's: a session's open and turns in CI's build (12.4 ms and 11 ms a test on a runner), the open's walks ([a-sessions-open-walks-the-same-paths-again](../../outstanding/a-sessions-open-walks-the-same-paths-again.md)), and the markdown reader's cost a line unoptimised.
- **A change to what a command's start costs on Windows** (the signed approvals' consoles) moves tier 3 alone: no tier 2 test starts a process.
