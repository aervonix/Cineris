# After the re-decide step of step 12: deciding a call again in the loop's tool round

The step adds about thirty-five tests to `cortex-agent`, `cortex-core` and `cortex-policy`, seven of them `sys_` tests that start a shell (four make a link). Linux's two runs were on runners of different start costs (4 ms on `main`, 6 ms on the pull request), so their tiers do not compare; tier 2 and tier 1 were over budget on `main`'s run and within it on the pull request's. Windows' two runs were on runners of one speed (9 ms), and differ by under a second in tiers 2 and 3 and by three seconds in tier 1, all within budget. `test-windows` took 8 s longer than `main`'s.

## On the pull request

<!-- written by cargo xtask ci-times, section "On the pull request": it replaces this up to the end mark -->
Pull request 70, on `main` at `3c5ee17`. "Main" is main's own run at `3c5ee17` (run 43871129198).
The PR's run is 43871373745, at `837555d`.

| Job | Main, `3c5ee17` | The PR at `837555d` |
|---|---|---|
| `lint-linux` | 65 s | 63 s |
| `test-linux` | 123 s | 125 s |
| `lint-windows` | 90 s | 87 s |
| `test-windows` | 242 s | 250 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | 0 s | — |
| `pr-size-linux` (blocks nothing) | — | 206 s |
| `pr-size-windows` (blocks nothing) | — | 217 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `837555d` (6 ms) | Windows, main (9 ms) | Windows, `837555d` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 20.5 s | 13.2 s | 19.9 s | 17.0 s |
| Tier 2, integration (40 s) | 46.0 s | 27.3 s | 37.5 s | 36.9 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 44.0 s | 22.2 s | 82.0 s | 83.0 s |

<!-- end of what cargo xtask ci-times wrote, section "On the pull request" -->
