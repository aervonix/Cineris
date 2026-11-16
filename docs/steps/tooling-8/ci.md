# After the eighth tooling step: the dialog tests, apply, the re-run after a stall, mutate --changed, perf-ab and the test kit

This step added tools and test support in four pull requests: the dialog tests' fix (#64), `apply`, the re-run after a stall, `decision new` and `ci-times`' jobs that block nothing (#65), `mutate --changed` and `in-string` (#67), and the measurement commands and the test kit (the last). No dependency was added. The tables were made with `cargo xtask ci-times`; each pull request's A/B against `main` is beside its run.

## The dialog tests (#64)

<!-- written by cargo xtask ci-times, section "pr-64": it replaces this up to the end mark -->
Pull request 64, on `main` at `e73ef24`. "Main" is main's own run at `e73ef24` (run 43830230952).
The PR's run is 43844985596, at `6589a11`.

| Job | Main, `e73ef24` | The PR at `6589a11` |
|---|---|---|
| `lint-linux` | 55 s | 56 s |
| `test-linux` | 105 s | 122 s |
| `lint-windows` | 71 s | 86 s |
| `test-windows` | 190 s | 192 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | — | — |
| `pr-size-linux` (blocks nothing) | — | 150 s |
| `pr-size-windows` (blocks nothing) | — | 319 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `6589a11` (6 ms) | Windows, main (7 ms) | Windows, `6589a11` (8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 8.3 s | 13.6 s | 10.8 s | 11.8 s |
| Tier 2, integration (40 s) | 13.4 s | 24.1 s | 23.9 s | 25.8 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 16.7 s | 19.9 s | 54.7 s | 63.9 s |

<!-- end of what cargo xtask ci-times wrote, section "pr-64" -->

## apply, the re-run after a stall, decision new (#65)

<!-- written by cargo xtask ci-times, section "pr-65": it replaces this up to the end mark -->
Pull request 65, on `main` at `00e5482`. "Main" is main's own run at `00e5482` (run 43846484206).
The PR's run is 43851681932, at `e40053a`.

| Job | Main, `00e5482` | The PR at `e40053a` |
|---|---|---|
| `lint-linux` | 58 s | 61 s |
| `test-linux` | 120 s | 126 s |
| `lint-windows` | 73 s | 56 s |
| `test-windows` | 181 s | 233 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | 0 s | — |
| `pr-size-linux` (blocks nothing) | — | 213 s |
| `pr-size-windows` (blocks nothing) | — | 340 s |

| Time above process start, against the budget | Linux, main (6 ms) | Linux, `e40053a` (4 ms) | Windows, main (7 ms) | Windows, `e40053a` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 13.3 s | 13.8 s | 11.1 s | 16.9 s |
| Tier 2, integration (40 s) | 24.2 s | 19.4 s | 23.9 s | 35.4 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 19.9 s | 39.6 s | 54.5 s | 70.3 s |

<!-- end of what cargo xtask ci-times wrote, section "pr-65" -->

## mutate --changed and in-string (#67)

<!-- written by cargo xtask ci-times, section "pr-67": it replaces this up to the end mark -->
Pull request 67, on `main` at `5431388`. "Main" is main's own run at `5431388` (run 43853266285).
The PR's run is 43855401812, at `e14cd85`.

| Job | Main, `5431388` | The PR at `e14cd85` |
|---|---|---|
| `lint-linux` | 60 s | 63 s |
| `test-linux` | 117 s | 114 s |
| `lint-windows` | 76 s | 97 s |
| `test-windows` | 237 s | 204 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | — | — |
| `pr-size-linux` (blocks nothing) | — | 7 s |
| `pr-size-windows` (blocks nothing) | — | 15 s |

| Time above process start, against the budget | Linux, main (6 ms) | Linux, `e14cd85` (5 ms) | Windows, main (9 ms) | Windows, `e14cd85` (8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 13.7 s | 15.1 s | 18.3 s | 11.6 s |
| Tier 2, integration (40 s) | 23.0 s | 26.5 s | 36.0 s | 25.6 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 20.6 s | 20.8 s | 73.7 s | 61.4 s |

<!-- end of what cargo xtask ci-times wrote, section "pr-67" -->

## Pull request 65 against the main before it, on one Windows runner

<!-- written by cargo xtask ci-times, section "ab-65": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=00e5482` (run 43853400752), the median of 3 runs each, on one runner at 9.0 ms a process start:

| Tier | The PR | `00e5482` | Change | Budget |
|---|---|---|---|---|
| 1, unit | 13.5 s (2,150 tests) | 14.5 s (2,090) | −1.0 s | 20 s |
| 2, integration | 27.9 s (1,259 tests) | 29.9 s (1,259) | −2.0 s | 40 s |
| 3, system | 64.3 s (148 tests) | 64.7 s (148) | −0.4 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "ab-65" -->

## Pull request 67 against main, both trees on one runner

<!-- written by cargo xtask ci-times, section "ab-67": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=5431388` (run 43853362786), the median of 3 runs each, on one runner at 9.0 ms a process start:

| Tier | The PR | `5431388` | Change | Budget |
|---|---|---|---|---|
| 1, unit | 16.3 s (2,177 tests) | 18.0 s (2,150) | −1.6 s | 20 s |
| 2, integration | 34.1 s (1,259 tests) | 35.5 s (1,259) | −1.4 s | 40 s |
| 3, system | 67.6 s (149 tests) | 67.5 s (148) | +0.1 s | 90 s |

### Linux, both trees on one runner

`try` with `-f against=5431388` (run 43853362786), the median of 3 runs each, on one runner at 5.0 ms a process start:

| Tier | The PR | `5431388` | Change | Budget |
|---|---|---|---|---|
| 1, unit | 9.9 s (2,198 tests) | 9.7 s (2,171) | +0.2 s | 20 s |
| 2, integration | 17.8 s (1,270 tests) | 17.8 s (1,270) | +0.0 s | 40 s |
| 3, system | 17.8 s (207 tests) | 17.4 s (206) | +0.4 s | 60 s |

<!-- end of what cargo xtask ci-times wrote, section "ab-67" -->

## The last pull request against the one before it, both trees on one runner

<!-- written by cargo xtask ci-times, section "ab-measure": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=worktree-step-02-tooling-8-mutate` (run 43855504863), the median of 3 runs each, on one runner at 10.0 ms a process start:

| Tier | The PR | `worktree-step-02-tooling-8-mutate` | Change | Budget |
|---|---|---|---|---|
| 1, unit | 16.5 s (2,241 tests) | 17.0 s (2,203) | −0.4 s | 20 s |
| 2, integration | 35.7 s (1,312 tests) | 36.2 s (1,312) | −0.5 s | 40 s |
| 3, system | 75.1 s (152 tests) | 72.7 s (149) | +2.4 s | 90 s |

### Linux, both trees on one runner

`try` with `-f against=worktree-step-02-tooling-8-mutate` (run 43855504863), the median of 3 runs each, on one runner at 6.0 ms a process start:

| Tier | The PR | `worktree-step-02-tooling-8-mutate` | Change | Budget |
|---|---|---|---|---|
| 1, unit | 12.8 s (2,262 tests) | 12.7 s (2,224) | +0.2 s | 20 s |
| 2, integration | 24.2 s (1,323 tests) | 24.0 s (1,323) | +0.2 s | 40 s |
| 3, system | 19.2 s (213 tests) | 19.0 s (207) | +0.2 s | 60 s |

<!-- end of what cargo xtask ci-times wrote, section "ab-measure" -->
