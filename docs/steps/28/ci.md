# After step 28: extensions

## The first pull request: the engine

<!-- written by cargo xtask ci-times, section "The first pull request: the engine": it replaces this up to the end mark -->
Pull request 117, on `main` at `119f09c`. "Main" is main's own run at `119f09c` (run 44791484182).
The PR's run is 44822683826, at `9f88e66`.

| Job | Main, `119f09c` | The PR at `9f88e66` |
|---|---|---|
| `lint-linux` | 55 s | 98 s |
| `test-linux` | 149 s | 151 s |
| `lint-windows` | 108 s | 127 s |
| `test-windows` | 175 s | 302 s |
| `ts-client` (blocks nothing) | 0 s | 0 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `9f88e66` (3 ms) | Windows, main (5 ms) | Windows, `9f88e66` (8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 16.4 s | 14.9 s | 10.1 s | 23.9 s |
| Tier 2, integration (40 s) | 22.2 s | 23.3 s | 21.4 s | 48.9 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 38.1 s | 35.5 s | 61.4 s | 95.9 s |

<!-- end of what cargo xtask ci-times wrote, section "The first pull request: the engine" -->

The PR's `test-windows` ran on a slow runner (a process start of 8 ms, against 5 ms for main's run) and from a cold build, `rquickjs` having changed the lock file. Every test passed, and the job's tier check warned at 122% of the integration tier's budget and 107% of the system tier's, under the failure at 150%; the job was then stopped at its five-minute limit after the line "Every test passed". The unit tier, which this pull request hardly touches, took 23.9 s against main's 10.1 s on that runner, about 2.4 times as long, and `cineris-core`'s share of the integration tier was the same as on the runner below (19.1 s of 48.9 s, against 9.5 s of 23.9 s). Two runs on runners of different speeds are not compared, so the change is the one both trees show on one runner.

## The first pull request, both trees on one runner

`try` with `-f against=main` (run 44819307542, at `5d72474`, before the last two commits: the order of the engine's look at a call's end, and this pull request's record), the median of 5 runs each, on one runner at 5.0 ms a process start. Copied from `cargo xtask try-result 44819307542`: `ci-times --ab` reads every system's artifact of a run, and a `try --final` run's Linux artifact holds the size check's output, not an A/B.

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 10.6 s (3,215 tests) | 10.7 s (3,189) | −0.1 s | 25 s |
| 2, integration | 23.9 s (1,644 tests) | 22.5 s (1,547) | +1.4 s | 40 s |
| 3, system | 56.3 s (224 tests) | 54.9 s (203) | +1.4 s | 90 s |

The crates that moved: in the integration tier `cineris-core` 8.54 s to 9.51 s and the new `cineris-ext` 0.59 s; in the system tier `cineris-core` 12.84 s to 13.64 s. The same run's tests on Linux 22.04 used 79%, 83% and 74% of the three tiers' budgets.

## The first pull request: the binary's size

The PR's `pr-size` jobs wait for its tests, and were not run once `test-windows` was stopped at its time limit, so the size was measured by `try` runs of `cargo xtask size --check --against-main`: Linux in run 44819307542 (at `5d72474`), Windows in run 44823536457 (at `9f88e66`), each against main's own size record at `119f09c` (run 44791484279). The shipped `dist` build:

| System | The PR | Main | Change | Budget |
|---|---|---|---|---|
| Linux, `x86_64-unknown-linux-musl` | 15,784,728 bytes (15.05 MiB) | 14,522,792 bytes | + 1,261,936 bytes (1,232 KiB) | 20 MiB |
| Windows, `x86_64-pc-windows-msvc` | 18,881,536 bytes (18.01 MiB) | 17,498,112 bytes | + 1,383,424 bytes (1,351 KiB) | 25 MiB |

On Linux the crates that grew most were QuickJS's C (`[C, assembly and libc]`, + 601 KiB), the standard library (+ 104 KiB) and `cineris_core` (+ 102 KiB). The engine alone, linked by step 28's engine check, was + 815,408 bytes on Linux and + 775,168 on Windows ([dependencies.md](../../dependencies.md), `rquickjs`); the rest is this pull request's own code. Neither binary holds a path under a home folder.

