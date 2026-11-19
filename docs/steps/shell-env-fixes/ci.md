# After the shell environment's fixes: a PATH that never names the workspace, the key variables' two paths, and cmd.exe's look

The fix adds six `sys_` tests: two in `cineris-tools` on Linux (bash and sh), three in `cineris-tools` on Windows (cmd, PowerShell 7, Windows PowerShell), and one in `cineris-core` on both. It also adds a few unit tests.

**`test-windows` was cut off at its 5-minute limit on the pull request's run** (44435264712) after every step had passed: build 142.8 s, tiers 1 and 2 55.7 s, system tests 49.7 s, then the cancellation during the report's upload. `main`'s own last run of the job took the same 305 s (run 44429746409): the job has been at its limit on slow runners, and a tooling step is shortening it.

The one-runner A/B against `main` (try run 44435990764, five rounds) puts the fix's cost in its new tests:
- **Tier 3:** +2.9 s, 204 tests against 200. The three Windows shells' new test costs 0.5, 0.4 and 0.1 s, and the core's reload test 0.4 s. Tier 3 is at 87.4 s of its 90 s budget, where `main` is at 84.5 s, and far from the 135 s line where it fails.
- **Tiers 1 and 2:** they moved by less than a round's spread. `main` is over tier 2's budget too, at 41.5 s.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 102, on `main` at `8823097`. "Main" is main's own run at `8823097` (run 44429746409).
The PR's run is 44435264712, at `d81ad3e`.

| Job | Main, `8823097` | The PR at `d81ad3e` |
|---|---|---|
| `lint-linux` | 66 s | 62 s |
| `test-linux` | 140 s | 117 s |
| `lint-windows` | 76 s | 96 s |
| `test-windows` | 305 s | 305 s |
| `ts-client` (blocks nothing) | 0 s | 0 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `d81ad3e` (4 ms) | Windows, main (8 ms) |
|---|---|---|---|
| Tier 1, unit (20 s) | 23.7 s | 18.6 s | 22.0 s |
| Tier 2, integration (40 s) | 32.3 s | 28.1 s | 45.9 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 28.4 s | 30.8 s | 90.9 s |

<!-- end of what cargo xtask ci-times wrote -->

## Windows, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 44435990764), the median of 5 runs each, on one runner at 8.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 20.3 s (2,902 tests) | 19.6 s (2,899) | +0.7 s | 20 s |
| 2, integration | 43.1 s (1,542 tests) | 41.5 s (1,542) | +1.6 s | 40 s |
| 3, system | 87.4 s (204 tests) | 84.5 s (200) | +2.9 s | 90 s |

<!-- end of what cargo xtask ci-times wrote -->
