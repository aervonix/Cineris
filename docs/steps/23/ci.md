# After step 23: the interactive application

Step 23 opens the interface on a real session. Its first pull request (58) builds the application's loop, the mapping of the session's events, the input wired, the approval dialog, interactive mode in `cineris-cli`, and the end-to-end tests of the real binary under a pseudo-terminal. Its second builds the rest (`answered.md`, "The design, before building (reviewed)", point 5). It adds no third-party package: `cineris-tui` now uses `serde_json`, which the workspace already had.

**What it adds to the tiers.** The four end-to-end tests of the interface run in tier 3. On Linux the slowest of them is the approval's, 0.38 s, most of it the dialog's 300 ms grace before its answers can be pressed; on Windows none of them is among the five slowest. The application's own tests are unit tests in `cineris-tui` (tier 1), run on scripted links and a terminal played back on a clock of its own, so none waits.

The first part's Windows `try` run (43785122305, `test -p cineris-tui -p cineris-cli -p cineris-core -p cineris-term`, at `762d7eb`): 544 unit and integration tests and 53 system tests, every one passed, at 5.0 s of 20, 8.2 s of 40 and 18.9 s of 90 above the start of their processes.

The tables below compare two runners of the same start cost on Windows (10 ms), and close ones on Linux. Each tier of the pull request is at or under main's. Windows' tiers 1 and 2 are just over their budgets on both trees (20.4 s of 20 and 42.1 s of 40 here, 21.4 s and 42.5 s on main): warnings, under the failure at one and a half times, there before this step; Windows tier 2's margin is tooling 7's first job.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 58, on `main` at `bd2e6a1`. "Main" is main's own run at `bd2e6a1` (run 43773439421).
The PR's run is 43785396220, at `762d7eb`.

| Job | Main, `bd2e6a1` | The PR at `762d7eb` |
|---|---|---|
| `lint-linux` | 47 s | 50 s |
| `test-linux` | 98 s | 91 s |
| `lint-windows` | 73 s | 75 s |
| `test-windows` | 214 s | 215 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `762d7eb` (3 ms) | Windows, main (10 ms) | Windows, `762d7eb` (10 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 10.1 s | 7.4 s | 21.4 s | 20.4 s |
| Tier 2, integration (40 s) | 24.0 s | 12.5 s | 42.5 s | 42.1 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 28.0 s | 11.7 s | 74.3 s | 66.7 s |

<!-- end of what cargo xtask ci-times wrote -->

## On the second pull request

The second pull request (60) builds the trust and question dialogs, the commands and pickers, the viewer, history, `@` files off the key's path, resume and undo, graceful signals and the end outside the core's lane.

**What it adds to the tiers.** Both trees on one Linux runner (the section after the tables), the medians of five runs: tier 1 +0.3 s (7.8 s of 20), tier 2 +0.3 s (14.3 s of 40), tier 3 +5.9 s (16.2 s of 60). Tier 3's growth is the end-to-end tests of the interface, most of it the three that end the application while a question holds the core's lane (1.0 to 1.35 s each: each ends it three ways, on Linux, under its question). On Windows, where they end it one way, `cineris-cli`'s system tests grew by 1.6 s, within the 12 s the coordinator set aside for this step's end-to-end tests; the pull request's tier 3 was 44.3 s against main's 53.5 s, on a faster runner (5 ms a process start against 7). `test-windows` took 144 s against main's 184 s, and 214 s on the first part's run, under the 5 minutes.

**The pull request's Linux tier 1 (19.0 s) is the runner, not the change.** In that run `xtask`'s tests of the `mutate` driver took 0.3 to 1.3 s each, against 3 to 35 ms on main's run, at the desk and in the A/B, where tier 1 moved by 0.3 s. This pull request changes nothing of `xtask`.


<!-- written by cargo xtask ci-times, section "On the second pull request": it replaces this up to the end mark -->
Pull request 60, on `main` at `de5a686`. "Main" is main's own run at `de5a686` (run 43791749637).
The PR's run is 43807561590, at `e3a76d6`.

| Job | Main, `de5a686` | The PR at `e3a76d6` |
|---|---|---|
| `lint-linux` | 67 s | 48 s |
| `test-linux` | 85 s | 112 s |
| `lint-windows` | 113 s | 88 s |
| `test-windows` | 184 s | 144 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `e3a76d6` (4 ms) | Windows, main (7 ms) | Windows, `e3a76d6` (5 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 7.6 s | 19.0 s | 10.2 s | 9.0 s |
| Tier 2, integration (40 s) | 15.8 s | 20.0 s | 22.4 s | 18.6 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 15.6 s | 19.5 s | 53.5 s | 44.3 s |

<!-- end of what cargo xtask ci-times wrote, section "On the second pull request" -->

## The second pull request, both trees on one runner

<!-- written by cargo xtask ci-times, section "The second pull request, both trees on one runner": it replaces this up to the end mark -->
### Linux, both trees on one runner

`try` with `-f against=main` (run 43807971215), the median of 5 runs each, on one runner at 4.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 7.8 s (2,089 tests) | 7.5 s (2,048) | +0.3 s | 20 s |
| 2, integration | 14.3 s (1,268 tests) | 14.0 s (1,249) | +0.3 s | 40 s |
| 3, system | 16.2 s (206 tests) | 10.3 s (196) | +5.9 s | 60 s |

<!-- end of what cargo xtask ci-times wrote, section "The second pull request, both trees on one runner" -->

## after-the-review-pr1

<!-- written by cargo xtask ci-times, section "after-the-review-pr1": it replaces this up to the end mark -->
Pull request 69, on `main` at `1870799`. "Main" is main's own run at `1870799` (run 43860492699).
The PR's run is 43865327897, at `d3d6a72`.

| Job | Main, `1870799` | The PR at `d3d6a72` |
|---|---|---|
| `lint-linux` | 57 s | 49 s |
| `test-linux` | 123 s | 110 s |
| `lint-windows` | 79 s | 84 s |
| `test-windows` | 211 s | 167 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | — | — |
| `pr-size-linux` (blocks nothing) | — | 139 s |
| `pr-size-windows` (blocks nothing) | — | 361 s |

| Time above process start, against the budget | Linux, main (6 ms) | Linux, `d3d6a72` (3 ms) | Windows, main (7 ms) | Windows, `d3d6a72` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 13.2 s | 8.8 s | 14.1 s | 9.2 s |
| Tier 2, integration (40 s) | 24.5 s | 14.4 s | 28.4 s | 22.0 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 21.1 s | 21.1 s | 70.0 s | 51.8 s |

<!-- end of what cargo xtask ci-times wrote, section "after-the-review-pr1" -->

## after-the-review-pr2-trust

<!-- written by cargo xtask ci-times, section "after-the-review-pr2-trust": it replaces this up to the end mark -->
Pull request 72, on `main` at `fa8b464`. "Main" is main's own run at `fa8b464` (run 43873311224).
The PR's run is 43878054090, at `5949de0`.

| Job | Main, `fa8b464` | The PR at `5949de0` |
|---|---|---|
| `lint-linux` | 47 s | 55 s |
| `test-linux` | 109 s | 125 s |
| `lint-windows` | 76 s | 86 s |
| `test-windows` | 251 s | 163 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | — | — |
| `pr-size-linux` (blocks nothing) | — | 211 s |
| `pr-size-windows` (blocks nothing) | — | 228 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `5949de0` (5 ms) | Windows, main (9 ms) | Windows, `5949de0` (5 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 9.5 s | 16.2 s | 17.8 s | 9.5 s |
| Tier 2, integration (40 s) | 14.9 s | 27.7 s | 38.5 s | 21.6 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 19.1 s | 25.3 s | 83.8 s | 56.4 s |

<!-- end of what cargo xtask ci-times wrote, section "after-the-review-pr2-trust" -->

## after-the-review-pr2-output

<!-- written by cargo xtask ci-times, section "after-the-review-pr2-output": it replaces this up to the end mark -->
Pull request 74, on `main` at `fa8b464`. "Main" is main's own run at `fa8b464` (run 43873311224).
The PR's run is 43879984102, at `39b6348`.

| Job | Main, `fa8b464` | The PR at `39b6348` |
|---|---|---|
| `lint-linux` | 47 s | 58 s |
| `test-linux` | 109 s | 120 s |
| `lint-windows` | 76 s | 96 s |
| `test-windows` | 251 s | 177 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | — | — |
| `pr-size-linux` (blocks nothing) | — | 206 s |
| `pr-size-windows` (blocks nothing) | — | 347 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `39b6348` (4 ms) | Windows, main (9 ms) | Windows, `39b6348` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 9.5 s | 20.4 s | 17.8 s | 11.6 s |
| Tier 2, integration (40 s) | 14.9 s | 46.0 s | 38.5 s | 24.5 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 19.1 s | 35.1 s | 83.8 s | 65.0 s |

<!-- end of what cargo xtask ci-times wrote, section "after-the-review-pr2-output" -->

## after-the-review-pr3-hangup

<!-- written by cargo xtask ci-times, section "after-the-review-pr3-hangup": it replaces this up to the end mark -->
Pull request 76, on `main` at `8fca310`. "Main" is main's own run at `8fca310` (run 43904353374).
The PR's run is 43904654868, at `c11b98a`.

| Job | Main, `8fca310` | The PR at `c11b98a` |
|---|---|---|
| `lint-linux` | 48 s | 66 s |
| `test-linux` | 116 s | 127 s |
| `lint-windows` | 64 s | 84 s |
| `test-windows` | 174 s | 216 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | 0 s | — |
| `pr-size-linux` (blocks nothing) | — | 170 s |
| `pr-size-windows` (blocks nothing) | — | 351 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `c11b98a` (6 ms) | Windows, main (6 ms) | Windows, `c11b98a` (7 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 15.2 s | 15.0 s | 9.6 s | 13.3 s |
| Tier 2, integration (40 s) | 25.6 s | 27.0 s | 23.3 s | 30.4 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 21.9 s | 26.2 s | 51.8 s | 67.7 s |

<!-- end of what cargo xtask ci-times wrote, section "after-the-review-pr3-hangup" -->
