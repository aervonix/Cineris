# After the ninth tooling step: room in Windows' tiers, the process-start estimate, the rigs' helpers and the A/B that survives a one-sided failure

This step went in as several pull requests. The tables were made with `cargo xtask ci-times`; each pull request's A/B against `main` is beside its run.

## The tiers' pull request

<!-- written by cargo xtask ci-times, section "The tiers' pull request": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 43891258419), the median of 5 runs each, on one runner at 9.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 18.5 s (2,363 tests) | 17.9 s (2,363) | +0.6 s | 20 s |
| 2, integration | 40.2 s (1,375 tests) | 39.3 s (1,375) | +1.0 s | 40 s |
| 3, system | 72.1 s (174 tests) | 85.2 s (174) | −13.1 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "The tiers' pull request" -->

## The tools' pull request

<!-- written by cargo xtask ci-times, section "The tools' pull request": it replaces this up to the end mark -->
Pull request 78, on `main` at `0458c35`. "Main" is main's own run at `0458c35` (run 43908344191).
The PR's run is 43909545536, at `1a8fef1`.

| Job | Main, `0458c35` | The PR at `1a8fef1` |
|---|---|---|
| `lint-linux` | 44 s | 67 s |
| `test-linux` | 115 s | 128 s |
| `lint-windows` | 93 s | 77 s |
| `test-windows` | 199 s | 267 s |
| `pr-size-linux` (blocks nothing) | — | 204 s |
| `pr-size-windows` (blocks nothing) | — | 344 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `1a8fef1` (5 ms) | Windows, main (7 ms) | Windows, `1a8fef1` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 9.1 s | 16.6 s | 10.9 s | 18.6 s |
| Tier 2, integration (40 s) | 15.0 s | 27.7 s | 25.1 s | 41.4 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 22.1 s | 24.2 s | 61.3 s | 91.3 s |

<!-- end of what cargo xtask ci-times wrote, section "The tools' pull request" -->

## The tools' pull request, both trees on one runner

<!-- written by cargo xtask ci-times, section "The tools' pull request, both trees on one runner": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 43911565004), the median of 3 runs each, on one runner at 9.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 19.3 s (2,409 tests) | 17.3 s (2,387) | +2.0 s | 20 s |
| 2, integration | 40.4 s (1,400 tests) | 38.4 s (1,395) | +2.0 s | 40 s |
| 3, system | 72.0 s (179 tests) | 70.4 s (177) | +1.6 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "The tools' pull request, both trees on one runner" -->

The pull request's side of round 2 ran at 10 ms a start and slow in every tier (25.7, 50.5 and 78.7 s), and it moved each median. Rounds 1 and 3 are even: 18.6 and 17.3 s against 17.7 and 17.4 s in tier 1, 39.4 and 39.0 against 39.3 and 38.4 in tier 2, 72.3 and 73.5 against 74.7 and 70.2 in tier 3. Every crate moved up by about the same share, those the pull request does not touch among them (`cineris-session`, `cineris-knowledge`, `cineris-tui`'s markdown). What the pull request adds is its new tests: the four of the core rig's wait for a question at about 0.06 s each, and `mutate --changed`'s new system test at 0.1 s. The A/B before the rebase onto step 23's hang-up fixes (run 43902429006, 9 ms) gave 14.1 against 14.9 s, 31.4 against 33.1 s and 67.1 against 68.6 s. The pull request's own CI run had slower runners than `main`'s (9 ms against 7 ms on Windows, 5 ms against 3 ms on Linux), which is why its table above shows every tier higher.
