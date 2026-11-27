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

## The third pull request

<!-- written by cargo xtask ci-times, section "The third pull request": it replaces this up to the end mark -->
Pull request 138, on `main` at `16a87c5`. "Main" is main's own run at `16a87c5` (run 46002323031).
The PR's run is 46002438389, at `a8db68a`.

| Job | Main, `16a87c5` | The PR at `a8db68a` |
|---|---|---|
| `lint-linux` | 113 s | 118 s |
| `test-linux` | 188 s | 225 s |
| `lint-windows` | 121 s | 84 s |
| `build-windows` | 310 s | 282 s |
| `test-windows-tiers` | 110 s | 85 s |
| `test-windows-system` | 77 s | 94 s |
| `pr-size-linux` (blocks nothing) | — | 243 s |
| `ts-client` (blocks nothing) | 0 s | 7 s |
| `pr-size-windows` (blocks nothing) | — | 351 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `a8db68a` (4 ms) | Windows, main (8 ms; tier 3 5 ms) | Windows, `a8db68a` (6 ms; tier 3 7 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 15.5 s | 21.3 s | 25.6 s | 20.8 s |
| Tier 2, integration (40 s) | 34.0 s | 32.8 s | 55.3 s | 44.2 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 47.0 s | 46.1 s | 75.6 s | 95.4 s |

<!-- end of what cargo xtask ci-times wrote, section "The third pull request" -->

**The third pull request, both trees on one runner** (`try --final`, run 46002436050, Windows, the median of 5 runs each at 8 ms a process start, on main with the ci profile's third-party crates optimised): tier 1 22.6 s against main's 22.3 s (+0.3 s), tier 2 47.7 s against 47.5 s (+0.2 s), tier 3 87.4 s against 87.9 s (−0.5 s); each side's whole run 104.9 s against 106.5 s. On the main before that (run 45997823438, 5 ms) the three tiers were 1.4, 1.5 and 1.0 s faster than main's. The new tests that cost most: the waiting host calls' 12 MiB (0.18 s), the approved project gate's cases (0.16 s), and in tier 3 the secret file held back from a hook (0.20 s) and the unreadable trust store (0.18 s).

**Its start-up** (`cargo xtask startup`, `dist`, run 45994849109): headless `cineris -p` 12.71 ms on Linux (9.2 times its `--version`, as main's 9.3) and 27.93 ms on Windows (3.54 times, in main's 3.20 to 3.52). **Its first frame** (`perf-ab`, run 45994852931, one runner, 3 rounds of 30): on Linux 3.18 to 3.23 ms against main's 3.06 to 3.16 ms, a tenth of a millisecond, the instruction files' reading now taking out what acts unseen; on Windows the measurement under ConPTY read 0.00 ms in two of the head's rounds, so it says nothing there (main's 9.65 to 9.94 ms; the head's one real round 9.65 ms).

**Its stress and breaks:** `stress -p cineris-core -p cineris-agent -p cineris-ext --count 50` passed every test every time on both systems (run 45994842033), after two races it found on Windows were fixed in the product (parts/engine.md); `mutate step-28-fixes` caught 85 of 85 breaks on Linux and 89 of 89 on Windows (run 45994845594).
