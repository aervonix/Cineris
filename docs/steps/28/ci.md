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

