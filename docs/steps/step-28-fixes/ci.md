# After step-28-fixes: its CI times

## The first pull request

<!-- written by cargo xtask ci-times, section "The first pull request": it replaces this up to the end mark -->
Pull request 136, on `main` at `51f6eeb`. "Main" is main's own run at `51f6eeb` (run 45955638461).
The PR's run is 45958057261, at `e68c131`.

| Job | Main, `51f6eeb` | The PR at `e68c131` |
|---|---|---|
| `lint-linux` | 107 s | 91 s |
| `test-linux` | 166 s | 149 s |
| `lint-windows` | 117 s | 102 s |
| `build-windows` | 226 s | 184 s |
| `test-windows-tiers` | 105 s | 87 s |
| `test-windows-system` | 87 s | 98 s |
| `pr-size-linux` (blocks nothing) | — | 234 s |
| `ts-client` (blocks nothing) | 73 s | 7 s |
| `pr-size-windows` (blocks nothing) | — | 298 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `e68c131` (4 ms) | Windows, main (8 ms; tier 3 6 ms) | Windows, `e68c131` (6 ms; tier 3 8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 16.1 s | 13.8 s | 26.9 s | 21.1 s |
| Tier 2, integration (40 s) | 45.9 s | 31.6 s | 59.3 s | 45.8 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 42.7 s | 38.7 s | 84.0 s | 92.8 s |

<!-- end of what cargo xtask ci-times wrote, section "The first pull request" -->

## The second pull request

<!-- written by cargo xtask ci-times, section "The second pull request": it replaces this up to the end mark -->
Pull request 137, on `main` at `51f6eeb`. "Main" is main's own run at `51f6eeb` (run 45955638461).
The PR's run is 45975413121, at `2108f11`.

| Job | Main, `51f6eeb` | The PR at `2108f11` |
|---|---|---|
| `lint-linux` | 107 s | 103 s |
| `test-linux` | 166 s | 130 s |
| `lint-windows` | 117 s | 114 s |
| `build-windows` | 226 s | 106 s |
| `test-windows-tiers` | 105 s | 91 s |
| `test-windows-system` | 87 s | 72 s |
| `pr-size-linux` (blocks nothing) | — | 273 s |
| `ts-client` (blocks nothing) | 73 s | 73 s |
| `pr-size-windows` (blocks nothing) | — | 303 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `2108f11` (3 ms) | Windows, main (8 ms; tier 3 6 ms) | Windows, `2108f11` (6 ms; tier 3 5 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 16.1 s | 13.3 s | 26.9 s | 21.9 s |
| Tier 2, integration (40 s) | 45.9 s | 26.7 s | 59.3 s | 47.5 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 42.7 s | 36.2 s | 84.0 s | 72.2 s |

<!-- end of what cargo xtask ci-times wrote, section "The second pull request" -->

## Both trees on one runner, start-up and the first frame

**The first pull request** (`try --final`, run 45958063732, Windows, the median of 5 runs each at 5 ms a process start): tier 1 15.2 s against main's 16.7 s, tier 2 34.3 s against 37.2 s, tier 3 62.2 s against 64.3 s. Nothing it changed is on a test's path; the differences are the runner's.

**The second pull request** (`try --final`, run 45976355152, Windows, the median of 5 runs each at 6 ms a process start):

| Tier | The pull request | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 19.7 s | 19.2 s | +0.5 s | 25 s |
| 2, integration | 44.1 s | 43.2 s | +0.9 s | 40 s |
| 3, system | 74.9 s | 72.4 s | +2.5 s | 90 s |

Of tier 3's 2.5 s, 1.1 s is the new and grown tests of the guards: the start through 64 interpreters (0.57 s, after the control through 63 was left to the unit test that holds the bound, which halved it), the socket's refusal of another harness's command (0.34 s) and the nested run's eleven more subcommands (0.21 s). The rest is in crates the pull request does not touch (`cineris-session`'s capture test, +0.73 s). Each side's whole run, from start to end, was 88.3 s against main's 89.0 s at the median. Tier 3's +2.5 s is over the 1 s the fixes were held to; the maintainers accepted it, since 1.1 s of it is the new tests of the guards on both systems, which earn their time, and the rest is in crates the pull request does not touch, which is the runner's noise.

**Start-up** (`cargo xtask startup`, the `dist` profile; Windows runners differ in speed, so each run is read against its own `cineris --version`): headless `cineris -p` to its request took 31.9, 28.5 and 23.7 ms on three runs of the pull request (3.62, 3.62 and 3.41 times its `--version`) and 24.3, 17.5 and 31.2 ms on three of main (3.20, 3.47 and 3.52 times). On Linux 12.47 ms against 12.12 ms. Within the runners' noise.

**The first frame** (`cargo xtask perf-ab … perf_start_to_first_frame_under_both_names`, run 45983967736, both trees on one runner, 3 rounds of 30 starts): on Linux `cineris` 3.94 ms against main's 3.59 ms at the median of the rounds (3.10 to 4.13 against 3.52 to 4.07) and `cin` 3.59 against 3.61; on Windows `cineris` 10.38 ms against 14.06 ms (10.33 to 10.45 against 10.41 to 16.03) and `cin` 10.47 against 12.02. Within noise on both.
