# After step 28: extensions

## The first pull request: the engine

<!-- written by cargo xtask ci-times, section "The first pull request: the engine": it replaces this up to the end mark -->
Pull request 117, on `main` at `119f09c`. "Main" is main's own run at `119f09c` (run 44791484182).
The PR's run is 44824670735, at `96bd8b2`.

| Job | Main, `119f09c` | The PR at `96bd8b2` |
|---|---|---|
| `lint-linux` | 55 s | 88 s |
| `test-linux` | 149 s | 171 s |
| `lint-windows` | 108 s | 109 s |
| `test-windows` | 175 s | 230 s |
| `pr-size-linux` (blocks nothing) | — | 427 s |
| `ts-client` (blocks nothing) | 0 s | 6 s |
| `pr-size-windows` (blocks nothing) | — | 306 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `96bd8b2` (4 ms) | Windows, main (5 ms) | Windows, `96bd8b2` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 16.4 s | 18.8 s | 10.1 s | 17.7 s |
| Tier 2, integration (40 s) | 22.2 s | 25.5 s | 21.4 s | 36.3 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 38.1 s | 41.8 s | 61.4 s | 76.3 s |

<!-- end of what cargo xtask ci-times wrote, section "The first pull request: the engine" -->

The pull request's first run (44822683826, at `9f88e66`, the commit before this page) passed `lint-linux`, `lint-windows` and `test-linux`, and its `test-windows`, on a slow runner (a process start of 8 ms) and from a cold build (`rquickjs` changed the lock file), passed every test and was stopped at its five-minute limit after the line "Every test passed"; its tier check had warned at 122% of the integration tier's budget and 107% of the system tier's, under the failure at 150%. The unit tier, which this pull request hardly touches, took 23.9 s there against main's 10.1 s, and `cineris-core`'s share of the integration tier was the one both trees show on one runner below (19.1 s of 48.9 s, against 9.5 s of 23.9 s): the runner, not the change. The run above, at `96bd8b2`, is the second, every job of it passed, and two runs on runners of different speeds are not compared: the change is the one both trees show on one runner.

## The first pull request, both trees on one runner

`try` with `-f against=main` (run 44819307542, at `5d72474`, before the last two commits: the order of the engine's look at a call's end, and this pull request's record), the median of 5 runs each, on one runner at 5.0 ms a process start. Copied from `cargo xtask try-result 44819307542`: `ci-times --ab` reads every system's artifact of a run, and a `try --final` run's Linux artifact holds the size check's output, not an A/B.

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 10.6 s (3,215 tests) | 10.7 s (3,189) | −0.1 s | 25 s |
| 2, integration | 23.9 s (1,644 tests) | 22.5 s (1,547) | +1.4 s | 40 s |
| 3, system | 56.3 s (224 tests) | 54.9 s (203) | +1.4 s | 90 s |

The crates that moved: in the integration tier `cineris-core` 8.54 s to 9.51 s and the new `cineris-ext` 0.59 s; in the system tier `cineris-core` 12.84 s to 13.64 s. The same run's tests on Linux 22.04 used 79%, 83% and 74% of the three tiers' budgets.

## The first pull request: the binary's size

The pull request's `pr-size` jobs in run 44824670735 (at `96bd8b2`, whose code is `9f88e66`'s: it changed only the step's records), each against main's own size record at `119f09c` (run 44791484279), and the same numbers from `try` runs of `cargo xtask size --check --against-main` before them (Linux 44819307542, Windows 44823536457). The shipped `dist` build:

| System | The PR | Main | Change | Budget |
|---|---|---|---|---|
| Linux, `x86_64-unknown-linux-musl` | 15,784,728 bytes (15.05 MiB) | 14,522,792 bytes | + 1,261,936 bytes (1,232 KiB) | 20 MiB |
| Windows, `x86_64-pc-windows-msvc` | 18,881,536 bytes (18.01 MiB) | 17,498,112 bytes | + 1,383,424 bytes (1,351 KiB) | 25 MiB |

On Linux the crates that grew most were QuickJS's C (`[C, assembly and libc]`, + 601 KiB), the standard library (+ 104 KiB) and `cineris_core` (+ 102 KiB). The engine alone, linked by step 28's engine check, was + 815,408 bytes on Linux and + 775,168 on Windows ([dependencies.md](../../dependencies.md), `rquickjs`); the rest is this pull request's own code. Neither binary holds a path under a home folder.

## The second pull request: the fetch

<!-- written by cargo xtask ci-times, section "The second pull request: the fetch": it replaces this up to the end mark -->
Pull request 118, on `main` at `5ba4c95`. "Main" is main's own run at `5ba4c95` (run 44827163446).
The PR's run is 44842130923, at `b070fa5`.

| Job | Main, `5ba4c95` | The PR at `b070fa5` |
|---|---|---|
| `lint-linux` | 96 s | 90 s |
| `test-linux` | 150 s | 112 s |
| `lint-windows` | 101 s | 86 s |
| `test-windows` | 250 s | 272 s |
| `pr-size-linux` (blocks nothing) | — | 208 s |
| `ts-client` (blocks nothing) | — | 7 s |
| `pr-size-windows` (blocks nothing) | — | 287 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `b070fa5` (3 ms) | Windows, main (6 ms) | Windows, `b070fa5` (8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 12.1 s | 8.7 s | 15.4 s | 20.3 s |
| Tier 2, integration (40 s) | 22.0 s | 14.3 s | 34.8 s | 43.0 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 45.2 s | 40.3 s | 74.2 s | 89.5 s |

<!-- end of what cargo xtask ci-times wrote, section "The second pull request: the fetch" -->

Every job passed, `test-windows` in 272 s. Its runner was slower than `main`'s run's (a process start of 8 ms, against 6 ms), and there its tier check warned at 108% of the integration tier's budget (43.0 s of 40), under the failure at 150%. Two runs on runners of different speeds are not compared: the change is the one both trees show on one runner, below, and on such a runner `main`'s own tiers are over budget too ([main-windows-tiers-over-budget-on-slow-runners](../../outstanding/main-windows-tiers-over-budget-on-slow-runners.md)).

## The second pull request, both trees on one runner

`try --final` (run 44839011960, at `1642e06`, which is this pull request less its last commit, an item of the docs): Windows, the median of 5 runs each, on one runner at 8.0 ms a process start. Copied from `cargo xtask try-result 44839011960`, as for the first pull request.

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 22.1 s (3,242 tests) | 22.5 s (3,215) | −0.4 s | 25 s |
| 2, integration | 47.1 s (1,674 tests) | 46.7 s (1,644) | +0.4 s | 40 s |
| 3, system | 90.4 s (225 tests) | 90.1 s (224) | +0.3 s | 90 s |

The crates that moved: `cineris-llm`, the fetch client's tests, +0.33 s in tier 1 and +0.78 s in tier 2; every other crate within 0.35 s either way. The same run's tests on Linux 22.04 used 73%, 65% and 64% of the three tiers' budgets.

## The second pull request: the binary's size

The pull request's `pr-size` jobs in run 44842130923, each against `main`'s own size record at `5ba4c95` (run 44827163397). The shipped `dist` build:

| System | The PR | Main | Change | Budget |
|---|---|---|---|---|
| Linux, `x86_64-unknown-linux-musl` | 15,854,392 bytes (15.12 MiB) | 15,784,728 bytes | + 69,664 bytes (68 KiB) | 20 MiB |
| Windows, `x86_64-pc-windows-msvc` | 18,990,592 bytes (18.11 MiB) | 18,881,536 bytes | + 109,056 bytes (106.5 KiB) | 25 MiB |

On Windows the fetch client's roots bring `rustls-native-certs` and `schannel` into the build, which on Linux were there already. Neither binary holds a path under a home folder.

## The third pull request: extensions found and pinned

<!-- written by cargo xtask ci-times, section "The third pull request: extensions found and pinned": it replaces this up to the end mark -->
Pull request 121, on `main` at `5a02276`. "Main" is main's own run at `5a02276` (run 44876997123).
The PR's run is 44884999537, at `fba04d8`.

| Job | Main, `5a02276` | The PR at `fba04d8` |
|---|---|---|
| `lint-linux` | 73 s | 80 s |
| `test-linux` | 160 s | 125 s |
| `lint-windows` | 104 s | 107 s |
| `test-windows` | 244 s | 250 s |
| `pr-size-linux` (blocks nothing) | — | 449 s |
| `ts-client` (blocks nothing) | 0 s | 70 s |
| `pr-size-windows` (blocks nothing) | — | 335 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `fba04d8` (3 ms) | Windows, main (7 ms) | Windows, `fba04d8` (7 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 17.1 s | 11.5 s | 15.7 s | 17.0 s |
| Tier 2, integration (40 s) | 24.8 s | 21.3 s | 32.5 s | 35.5 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 39.8 s | 34.8 s | 74.7 s | 75.2 s |

<!-- end of what cargo xtask ci-times wrote, section "The third pull request: extensions found and pinned" -->

Every job passed, `test-windows` in 250 s. Both Windows runs were on runners at 7 ms a process start, and there this pull request's tier 2 is at 35.5 s of 40 (89%), +3.0 s on `main`'s, without a warning. The change is read from both trees on one runner, below.

## The third pull request, both trees on one runner

`try --final` (run 44883274241, at `c8517a2`, which is this pull request less its last commit, a break's test name): Windows, the median of 5 runs each, on one runner at 6.0 ms a process start. Copied from `cargo xtask try-result 44883274241`, as for the first pull request.

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 18.0 s (3,312 tests) | 17.3 s (3,251) | +0.6 s | 25 s |
| 2, integration | 39.3 s (1,740 tests) | 35.8 s (1,674) | **+3.5 s** | 40 s |
| 3, system | 72.3 s (227 tests) | 72.3 s (225) | +0.1 s | 90 s |

The crates that moved: `cineris-core`, +3.55 s in tier 2 (13.98 s to 17.53 s), its extension tests, the costliest the walk of a project's extension through its trust (0.18 s) and the state tests (0.11 to 0.13 s each); `cineris-ext`, +0.63 s in tier 1, its discovery and cache tests, moved there from tier 2; every other crate within 0.5 s either way. The same run's tests on Linux 22.04 used 90%, 69% and 69% of the three tiers' budgets.

**Tier 2 is a conflict with its budget, recorded** (rule 11). The extension tests added +5.1 s to Windows' tier 2 at first (`try` 44873812174, a runner at 6 ms). Folding the tests that share a setup, moving what needs no core into unit tests and chaining the one flow that is one story brought it to +2.6 s on that run's runner (`try` 44881859246) and +3.5 s on this one's; hosted runners of one start cost still differ. What is left is the cost of a session, about 20 ms on Windows in a release build with its world, core and close, and about twice that in CI's build; a test is one rule, so the tests are not folded further. On a slow runner, where `main`'s own tier 2 is over its budget, this change is about 9 s under the failure at 60 s. The lever, a session's open and first turn on Windows, is in [main-windows-tiers-over-budget-on-slow-runners](../../outstanding/main-windows-tiers-over-budget-on-slow-runners.md); [the part's record](parts/pr2a1-found.md), "The tests folded for Windows' tier 2", has each run's numbers.

## The third pull request: the binary's size

The pull request's `pr-size` jobs in run 44884999537, each against `main`'s own size record at `5a02276` (run 44876997098). The shipped `dist` build:

| System | The PR | Main | Change | Budget |
|---|---|---|---|---|
| Linux, `x86_64-unknown-linux-musl` | 16,206,744 bytes (15.46 MiB) | 15,854,392 bytes | + 352,352 bytes (344 KiB) | 20 MiB |
| Windows, `x86_64-pc-windows-msvc` | 19,471,360 bytes (18.57 MiB) | 18,990,592 bytes | + 480,768 bytes (469.5 KiB) | 25 MiB |

On Linux the crates that grew most are `cineris_core` (+96 KiB, the pipeline, the trust question's words and `cineris.state`), the standard library (+64 KiB) and `cineris_ext` (+60 KiB, discovery and the cache), from `cargo xtask size` in `try --final` (run 44883274241). Neither binary holds a path under a home folder.

## The fourth pull request: skills and the prompt

<!-- written by cargo xtask ci-times, section "The fourth pull request: skills and the prompt": it replaces this up to the end mark -->
Pull request 122, on `main` at `36d8266`. "Main" is main's own run at `36d8266` (run 44886379853).
The PR's run is 44911679902, at `8f3eafe`.

| Job | Main, `36d8266` | The PR at `8f3eafe` |
|---|---|---|
| `lint-linux` | 83 s | 74 s |
| `test-linux` | 143 s | 135 s |
| `lint-windows` | 108 s | 120 s |
| `test-windows` | 255 s | 305 s |
| `ts-client` (blocks nothing) | — | 0 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `8f3eafe` (4 ms) | Windows, main (6 ms) |
|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 11.4 s | 11.6 s | 19.7 s |
| Tier 2, integration (40 s) | 22.9 s | 22.4 s | 42.9 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 51.8 s | 37.0 s | 78.3 s |

<!-- end of what cargo xtask ci-times wrote, section "The fourth pull request: skills and the prompt" -->

`test-windows` was cut by its time limit, 5 min 5 s, in the system tests, in run 44911679902 and again when its failed jobs were run again: both times every test of tiers 1 and 2 had passed (5,133), and 173 and then 197 of the 228 system tests had run, the only failures those the cut made. Both runners were about 1.3 times slower than `main`'s run's (the build 2 min 11 s and 2 min 15 s against 1 min 37 s, tiers 1 and 2 60.8 s against 46.7 s), and this pull request changes `cineris-base`, which rebuilds the whole workspace past the restored cache: on such a runner `main`'s own job would take about 330 s ([main-windows-tiers-over-budget-on-slow-runners](../../outstanding/main-windows-tiers-over-budget-on-slow-runners.md)). So `pr-size` and `ts-client` did not run. **Windows' tests, every tier, the system tests included,** ran on the pull request's head in `try` run 44912907727, on a runner at 5 ms a process start: all passed, the job in 3 min 5 s, tier 1 11.4 s (46%), tier 2 27.6 s (69%), tier 3 61.8 s (69%). `ts-client` was taken by the maintainers from the pinned compiler's check run locally, the protocol's conformance test and the declarations' one added value, and runs on the next pull request.

## The fourth pull request, both trees on one runner

`try --final`, Windows, the median of 5 runs each, both trees on one runner. Copied from `cargo xtask try-result`, as for the first pull request.

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| **Run 44908721047, at `ce14c68` (the pull request's code), a runner at 8 ms** | | | | |
| 1, unit | 23.6 s (3,361 tests) | 22.2 s (3,312) | +1.4 s | 25 s |
| 2, integration | 53.4 s (1,772 tests) | 50.8 s (1,740) | +2.6 s | 40 s |
| 3, system | 90.5 s (228 tests) | 89.5 s (227) | +1.0 s | 90 s |
| **Run 44906099811, at `b34751f` (before the skill folders' read rule's tests and its setting), a runner at 6 ms** | | | | |
| 1, unit | 12.5 s (3,361 tests) | 12.8 s (3,312) | −0.3 s | 25 s |
| 2, integration | 28.8 s (1,770 tests) | 28.2 s (1,740) | +0.6 s | 40 s |
| 3, system | 58.3 s (228 tests) | 58.7 s (227) | −0.5 s | 90 s |

On the slow runner the crates that moved in tier 2 were `cineris-core` (+1.67 s, the new tests of the prompt's sections and of the skill folders' read rule, 0.07 to 0.14 s each) and `cineris-agent` (+0.24 s); in tier 1 `cineris-ext`'s skills tests, now unit tests. The wall time of a round was 107.7 s against `main`'s 106.9 s. Tier 2 stays the conflict with its budget the part before recorded ([parts/pr2a1-found.md](parts/pr2a1-found.md), "The tests folded for Windows' tier 2"). The same runs' tests on Linux 22.04 used 81% to 92%, 71% to 72% and 72% to 74% of the three tiers' budgets.

## The fourth pull request: the binary's size

`try` run 44913243284 (`size --check --against-main`, the pull request's head `8f3eafe`), against `main`'s own size record at `36d8266` (run 44886379885), as the pull request's `pr-size` jobs did not run. The shipped `dist` build:

| System | The PR | Main | Change | Budget |
|---|---|---|---|---|
| Linux, `x86_64-unknown-linux-musl` | 16,350,104 bytes (15.59 MiB) | 16,206,744 bytes | + 143,360 bytes (140 KiB) | 20 MiB |
| Windows, `x86_64-pc-windows-msvc` | 19,663,360 bytes (18.75 MiB) | 19,471,360 bytes | + 192,000 bytes (187.5 KiB) | 25 MiB |

On Linux the crates that grew most are the standard library (+30 KB), `cineris_ext` (+28 KB, the skills) and `cineris_core` (+21 KB). Neither binary holds a path under a home folder.

## The fifth pull request: the hooks' seam and hook commands

<!-- written by cargo xtask ci-times, section "The fifth pull request: the hooks' seam and hook commands": it replaces this up to the end mark -->
Pull request 126, on `main` at `fe16eae`. "Main" is main's own run at `fe16eae` (run 44928887339).
The PR's run is 44950381620, at `972cf2c`.

| Job | Main, `fe16eae` | The PR at `972cf2c` |
|---|---|---|
| `lint-linux` | 69 s | 84 s |
| `test-linux` | 171 s | 156 s |
| `lint-windows` | 109 s | 104 s |
| `test-windows` | 189 s | 229 s |
| `pr-size-linux` (blocks nothing) | — | 330 s |
| `ts-client` (blocks nothing) | 0 s | 73 s |
| `pr-size-windows` (blocks nothing) | — | 425 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `972cf2c` (3 ms) | Windows, main (5 ms) | Windows, `972cf2c` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 18.4 s | 15.0 s | 11.6 s | 13.9 s |
| Tier 2, integration (40 s) | 27.5 s | 25.3 s | 29.0 s | 34.2 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 43.4 s | 38.7 s | 62.9 s | 71.2 s |

<!-- end of what cargo xtask ci-times wrote, section "The fifth pull request: the hooks' seam and hook commands" -->

Every job passed, `test-windows` in 3 min 49 s at `972cf2c` (run 44950381620) and in 3 min 31 s at its last head, `bfc0f47`, whose commits after it are records and break lists only.

## The fifth pull request, both trees on one runner

`try --final` (run 44947980446, at `972cf2c`): Windows, the median of 5 runs each, on one runner at 8.0 ms a process start. Copied from `cargo xtask try-result 44947980446`, as for the first pull request.

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 23.3 s (3,418 tests) | 23.5 s (3,370) | −0.2 s | 25 s |
| 2, integration | 55.4 s (1,810 tests) | 54.8 s (1,781) | +0.7 s | 40 s |
| 3, system | 92.5 s (233 tests) | 92.5 s (231) | +0.0 s | 90 s |

The crates that moved in tier 2: `cineris-core` +0.51 s (the hooks' wiring and commands), `cineris-session` +0.12 s, `cineris-agent` +0.08 s. Before the hooks' tests were folded onto shared worlds (`try --final` 44934047604, at `5f5bc77`, a runner at 8 ms), tier 2 was +3.1 s and tier 3 +1.8 s; the maintainers asked for about +1.5 s. The same run's tests on Linux 22.04 used 94%, 74% and 76% of the three tiers' budgets. On this slow runner `main`'s own tier 2 is at 137% of its budget, 5.2 s from its failure, and its tier 3 over its budget ([main-windows-tiers-over-budget-on-slow-runners](../../outstanding/main-windows-tiers-over-budget-on-slow-runners.md)).

## The fifth pull request: the binary's size

The pull request's `pr-size` jobs in run 44950381620, each against `main`'s own size record (run 44928887211). The shipped `dist` build:

| System | The PR | Main | Change | Budget |
|---|---|---|---|---|
| Linux, `x86_64-unknown-linux-musl` | 16,780,280 bytes (16.00 MiB) | 16,472,984 bytes | + 307,296 bytes (300 KiB) | 20 MiB |
| Windows, `x86_64-pc-windows-msvc` | 20,239,360 bytes (19.30 MiB) | 19,793,408 bytes | + 445,952 bytes (435.5 KiB) | 25 MiB |

On Linux the crates that grew most are the standard library (+69 KB), `cineris_core` (+36 KB) and `cineris_agent` (+36 KB), from `cargo xtask size` in `try --final` (run 44947980446). Neither binary holds a path under a home folder.
