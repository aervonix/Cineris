# After small-fixes-2: its CI times

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 130, on `main` at `77ffbbc`. "Main" is main's own run at `77ffbbc` (run 45767289025).
The PR's run is 45787385919, at `dc04522`.

| Job | Main, `77ffbbc` | The PR at `dc04522` |
|---|---|---|
| `lint-linux` | 79 s | 76 s |
| `test-linux` | 119 s | 179 s |
| `lint-windows` | 76 s | 90 s |
| `test-windows` | 350 s | 293 s |
| `pr-size-linux` (blocks nothing) | — | 218 s |
| `ts-client` (blocks nothing) | — | 7 s |
| `pr-size-windows` (blocks nothing) | — | 363 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `dc04522` (4 ms) | Windows, main (9 ms) | Windows, `dc04522` (8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 12.6 s | 19.8 s | 28.6 s | 28.8 s |
| Tier 2, integration (40 s) | 23.1 s | 31.7 s | 57.6 s | 54.7 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 32.3 s | 43.6 s | 96.9 s | 106.4 s |

<!-- end of what cargo xtask ci-times wrote -->

## Both trees on one runner

The pull request's own run met a slower Linux runner than `main`'s (tier 1 19.8 s against 12.6). On one runner each, both trees side by side, the change is small:

| Tier | Windows, `try --final` 45784504601 (6 ms, medians of 5) | Linux, `try` 45789150185 (4 ms, medians of 3) | Budget |
|---|---|---|---|
| 1, unit | 16.6 s against `main`'s 16.7 (−0.1 s) | 11.4 s against 11.2 (+0.1 s) | 25 s Windows, 20 s Linux |
| 2, integration | 37.0 s against 37.8 (−0.8 s) | 21.8 s against 21.9 (−0.2 s) | 40 s |
| 3, system | 67.0 s against 66.2 (+0.8 s) | 34.3 s against 33.9 (+0.5 s) | 90 s Windows, 60 s Linux |

What moved: `cineris-core`'s trust tests now wait for the base capture before their turn, so the capture no longer runs beside it (Windows tier 3 +1.44 s for the crate, the longest test 2.47 s to 3.26 s); the decoder's test of `cineris auth login` is the largest new unit test (about 0.3 s); the two new tests of the stop's window are Linux's alone (about 0.06 s each). Linux 22.04 passed in the same final run, and the binary is 16.35 MB of 20.

**Windows stress, 20 times each under seeds 0 to 19:** the three terminal tests (`try` 45784535997) and 60 tests of `cineris-core` this fix touched, the trust, base state, asking, redecide, usage and learned-command tests among them (`try` 45784566331): every test passed every time.
