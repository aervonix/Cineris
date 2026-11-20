# After the CI-budgets fix: the tiers back under their budgets, and the test job's time

The CI times before and after the fix, and the runs that measured it. What was decided is in [answered.md](answered.md).

## Before: `main` against wave 12, on one runner

`main` at `97feb4dc` against `327cfaa4`, a commit of wave 12 after the tenth tooling step's layout, three rounds of each on one runner of each system. In these two tables **"The PR" is `main` and "Main" is wave 12** (the tool names the sides of every A/B so). Read test by test in [answered.md](answered.md), "Where the time went".

<!-- written by cargo xtask ci-times, section "before-against-wave-12": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 44632492422), the median of 3 runs each, on one runner at 8.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 24.7 s (3,178 tests) | 15.7 s (2,500) | +9.0 s | 20 s |
| 2, integration | 45.2 s (1,546 tests) | 37.0 s (1,431) | +8.2 s | 40 s |
| 3, system | 104.7 s (215 tests) | 80.7 s (184) | +24.1 s | 90 s |

### Linux, both trees on one runner

`try` with `-f against=main` (run 44632492422), the median of 3 runs each, on one runner at 6.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 27.7 s (3,199 tests) | 15.4 s (2,521) | +12.4 s | 20 s |
| 2, integration | 34.5 s (1,557 tests) | 29.0 s (1,442) | +5.4 s | 40 s |
| 3, system | 37.4 s (314 tests) | 26.7 s (270) | +10.8 s | 60 s |

<!-- end of what cargo xtask ci-times wrote, section "before-against-wave-12" -->

## After, Linux: the branch against `main`, on one runner

The branch at `a329260` against `main` at `b620a1e`, three rounds of each.

<!-- written by cargo xtask ci-times, section "linux-ab": it replaces this up to the end mark -->
### Linux, both trees on one runner

`try` with `-f against=main` (run 44639249117), the median of 3 runs each, on one runner at 3.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 14.0 s (3,208 tests) | 24.8 s (3,200) | −10.8 s | 20 s |
| 2, integration | 19.0 s (1,557 tests) | 28.1 s (1,557) | −9.1 s | 40 s |
| 3, system | 33.0 s (314 tests) | 33.3 s (314) | −0.3 s | 60 s |

<!-- end of what cargo xtask ci-times wrote, section "linux-ab" -->

## After, Windows: `try --final`, on one runner

The branch at `a329260` against `main` at `b620a1e`, the median of three rounds of each on one runner at 5 ms a process start (`try` 44639221170, its `try-windows` job; `ci-times --ab` does not read a `try --final` run yet, whose other jobs hold no A/B, so the table is copied from `try-result`):

| Tier | The branch | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 12.6 s (3,187 tests) | 13.8 s (3,179) | −1.2 s | 20 s |
| 2, integration | 24.5 s (1,546 tests) | 25.1 s (1,546) | −0.6 s | 40 s |
| 3, system | 58.7 s (203 tests) | 69.5 s (215) | −10.8 s | 90 s |

Tier 3 by crate: `xtask` 13.41 to 4.09 s (the tests the nightly now runs on Windows), `cineris-core` 14.34 to 13.74 s, the rest within half a second. Both test runs' wall time, the median of the rounds: 75.6 s on `main`, 61.9 s on the branch. The same run's other jobs: the tests on Linux 22.04, a 5 ms runner, read tier 1 at 17.1 s, tier 2 at 23.4 s and tier 3 at 39.4 s; the binary's size, 13.85 MB, within its 20 MB.

## After, Windows: the branch against `main` on a usual (8 ms) runner

`try` 44640939444, three rounds of each. One round of the branch's ran at 9 ms a start and read every tier slow (24.8, 44.2 s); in the two at 8 ms, tier 1 read 20.6 and 20.7 s on the branch against 21.0 and 20.2 s on `main`, and tier 2 38.5 and 35.8 s against 36.6 and 36.3 s: the same, within the noise. Tier 3: 89.0, 80.6 and 81.2 s against 100.8, 94.0 and 96.7 s. Both test runs' wall time, the median of the rounds: 105.0 s on `main`, 96.0 s on the branch.

## The pull request's CI

The pull request's last run, at `22da14b`: its Windows job passed every test and was then cut off by the five minutes (305 s), before it uploaded its report, so its column is missing from the table below. Its log, on a 9 ms runner: tier 1 23.8 s, tier 2 46.6 s, tier 3 98.0 s (above the start; budgets 20 s, 40 s and 90 s; that run had the proposed 25 s for tier 1, which was dropped). Its first run, at `83c58ae` on a 5 ms runner, took 190 s and read 10.7, 22.5 and 64.5 s. Runners of the same start cost differ too: the A/B's 8 ms runner read the same code's tiers 2 and 3 at 37.6 and 81.5 s. The A/Bs above are the comparison.

<!-- written by cargo xtask ci-times, section "pull-request": it replaces this up to the end mark -->
Pull request 115, on `main` at `b620a1e`. "Main" is main's own run at `b620a1e` (run 44633500009).
The PR's run is 44644603576, at `22da14b`.

| Job | Main, `b620a1e` | The PR at `22da14b` |
|---|---|---|
| `lint-linux` | 63 s | 73 s |
| `test-linux` | 159 s | 117 s |
| `lint-windows` | 106 s | 95 s |
| `test-windows` | 296 s | 305 s |
| `ts-client` (blocks nothing) | 0 s | 0 s |

| Time above process start, against the budget | Linux, main (6 ms) | Linux, `22da14b` (4 ms) | Windows, main (8 ms) |
|---|---|---|---|
| Tier 1, unit (20 s) | 28.3 s | 8.5 s | 19.6 s |
| Tier 2, integration (40 s) | 35.2 s | 17.3 s | 35.6 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 38.1 s | 32.6 s | 98.5 s |

<!-- end of what cargo xtask ci-times wrote, section "pull-request" -->

## The 8 ms runner's table

<!-- written by cargo xtask ci-times, section "windows-ab": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 44640939444), the median of 3 runs each, on one runner at 8.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 21.2 s (3,188 tests) | 20.4 s (3,179) | +0.8 s | 20 s |
| 2, integration | 37.6 s (1,546 tests) | 36.0 s (1,546) | +1.6 s | 40 s |
| 3, system | 81.5 s (203 tests) | 94.4 s (215) | −12.8 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "windows-ab" -->

## The job's time for a pull request that touches `cineris-base`

Such a pull request builds every crate of the workspace; `xtask`'s own binary, built first, depends only on `cineris-protocol`, and the cache restores it in about a second. On `main`, which builds the workspace's crates with nothing restored, the build of the tests took 150 s on an 8 ms runner (run 44592310472); with the set-up (about 35 s), the restore and dating (about 9 s), and the two test runs (55 s each there), such a pull request's job was about 308 s. The branch takes off about 20 s of the build (no debug info, the job A/B above in [answered.md](answered.md)) and about 9 s of the two test runs' wall time on an 8 ms runner (105.0 to 96.0 s, the medians above): about 279 s, under the five minutes with about 20 s of room, not the 30 the maintainers asked for. What is left is the workspace's build itself, about 130 s on such a runner. **Measured on this pull request,** which changes `[profile.ci]` and so rebuilds every crate as such a pull request does, `xtask` too: 305 s on a 9 ms runner, cut off after every test passed (set-up 38 s, `xtask` 18 s, the build 130 s, the two test runs 57 and 53 s). Without the `xtask` rebuild, which a change to `cineris-base` alone does not cause, it would be about 287 s there: on a slow runner such a pull request is at the five minutes still, and the estimate above is a usual runner's. The rest is the workspace's build and Windows' system tests; the maintainers' rule for a job cut off after every test passed stays needed for such pull requests.
