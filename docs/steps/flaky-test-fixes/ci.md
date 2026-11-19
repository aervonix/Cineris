# After the flaky-test fixes: an entity's escape cut mid-row, and a check's shell in tier 2

The fixes add four markdown unit tests and a process-runner unit test, and move one core test's shell from Windows' tier 2 to a Unix system test.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 104, on `main` at `f6fedc3`. "Main" is main's own run at `f6fedc3` (run 44440506912).
The PR's run is 44454844994, at `2021866`.

| Job | Main, `f6fedc3` | The PR at `2021866` |
|---|---|---|
| `lint-linux` | 61 s | 71 s |
| `test-linux` | 134 s | 125 s |
| `lint-windows` | 100 s | 88 s |
| `test-windows` | 237 s | 208 s |
| `pr-size-linux` (blocks nothing) | — | 223 s |
| `ts-client` (blocks nothing) | 0 s | 10 s |
| `pr-size-windows` (blocks nothing) | — | 297 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `2021866` (4 ms) | Windows, main (6 ms) | Windows, `2021866` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 20.9 s | 20.6 s | 14.5 s | 11.8 s |
| Tier 2, integration (40 s) | 46.4 s | 38.5 s | 33.1 s | 27.2 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 37.3 s | 30.8 s | 71.6 s | 64.1 s |

<!-- end of what cargo xtask ci-times wrote -->

## Linux, both trees on one runner, before the splits were sampled

At `059bd1c`, before the three escape tests split their documents only at every third width on a pull request (`1a62be9`). The three accounted for 1.1 s of tier 1's +1.3 s; sampled, they take 0.17 s at the desk in the `ci` profile, where they took 0.42 s.

<!-- written by cargo xtask ci-times, section "Linux, both trees on one runner, before the splits were sampled": it replaces this up to the end mark -->
### Linux, both trees on one runner

`try` with `-f against=main` (run 44451657672), the median of 3 runs each, on one runner at 5.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 17.8 s (2,925 tests) | 16.5 s (2,920) | +1.3 s | 20 s |
| 2, integration | 25.5 s (1,553 tests) | 26.0 s (1,553) | −0.5 s | 40 s |
| 3, system | 27.4 s (294 tests) | 27.2 s (293) | +0.1 s | 60 s |

<!-- end of what cargo xtask ci-times wrote, section "Linux, both trees on one runner, before the splits were sampled" -->

## Windows, both trees on one runner

**A `test-windows` cut off at its 5-minute limit** (run 44456228950, at `4c48992`, whose last commit added only records): the build took 2m20s, and all 4,449 tests of tiers 1 and 2 passed. Of the system tests, 193 passed, two were stopped by the cancellation (`0xc000013a`) and nine had not started. The same code at `2021866` ran the job in 3m28s, every test passing. Rebased on `main` at `45f0ee0` (run 44457768453, at `e345877`), it was cut off again at 5m4s after the job said "Every test passed": 4,456 tests of tiers 1 and 2 and 207 system tests, after a build of 2m16s. The A/B below, both trees on one runner, puts the change at no cost on Windows in any tier: each is lower, within a round's spread, and tier 2 is lower by the shell start the verify test no longer makes.

<!-- written by cargo xtask ci-times, section "Windows, both trees on one runner": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 44456899061), the median of 3 runs each, on one runner at 5.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 10.2 s (2,907 tests) | 11.4 s (2,902) | −1.2 s | 20 s |
| 2, integration | 22.5 s (1,542 tests) | 24.6 s (1,542) | −2.1 s | 40 s |
| 3, system | 55.0 s (204 tests) | 56.2 s (204) | −1.2 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "Windows, both trees on one runner" -->
