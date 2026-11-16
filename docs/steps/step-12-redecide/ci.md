# After the re-decide step of step 12: deciding a call again in the loop's tool round

The step adds about forty tests to `cortex-agent`, `cortex-core` and `cortex-policy`, nine of them `sys_` tests that start a shell (four make a link). The figures below are of the pull request's last head, rebased onto step 17's wiring. Linux's two runs were on runners of different start costs (3 ms on `main`, 5 ms on the pull request), so their tiers do not compare, and all are within budget. Windows' two runs were on runners of one speed (9 ms): tiers 1 and 2 are the same within a second, and tier 3 is 3.1 s more, within its 90 s budget: the step's four new `sys_` tests in `cortex-core` start PowerShell. `test-windows` took 3 s less than `main`'s. An earlier head, before the rebase onto step 17, measured the same within a second on Windows (run 43871373745).

## On the pull request

<!-- written by cargo xtask ci-times, section "On the pull request": it replaces this up to the end mark -->
Pull request 70, on `main` at `fa8b464`. "Main" is main's own run at `fa8b464` (run 43873311224).
The PR's run is 43874424555, at `61a0c32`.

| Job | Main, `fa8b464` | The PR at `61a0c32` |
|---|---|---|
| `lint-linux` | 47 s | 54 s |
| `test-linux` | 109 s | 127 s |
| `lint-windows` | 76 s | 98 s |
| `test-windows` | 251 s | 248 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | — | — |
| `pr-size-linux` (blocks nothing) | — | 170 s |
| `pr-size-windows` (blocks nothing) | — | 330 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `61a0c32` (5 ms) | Windows, main (9 ms) | Windows, `61a0c32` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 9.5 s | 15.7 s | 17.8 s | 17.8 s |
| Tier 2, integration (40 s) | 14.9 s | 26.6 s | 38.5 s | 38.3 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 19.1 s | 24.7 s | 83.8 s | 86.9 s |

<!-- end of what cargo xtask ci-times wrote, section "On the pull request" -->
