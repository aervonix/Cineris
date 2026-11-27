# After the step 25 fixes: their CI times

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 134, on `main` at `5eda620`. "Main" is main's own run at `5eda620` (run 45931692729).
The PR's run is 45947445853, at `96e2663`.

| Job | Main, `5eda620` | The PR at `96e2663` |
|---|---|---|
| `lint-linux` | 85 s | 93 s |
| `test-linux` | 182 s | 185 s |
| `lint-windows` | 120 s | 115 s |
| `test-windows` | 351 s | 257 s |
| `pr-size-linux` (blocks nothing) | — | 205 s |
| `ts-client` (blocks nothing) | — | 7 s |
| `pr-size-windows` (blocks nothing) | — | 451 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `96e2663` (4 ms) | Windows, main (8 ms) | Windows, `96e2663` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 20.2 s | 21.1 s | 27.6 s | 19.5 s |
| Tier 2, integration (40 s) | 31.6 s | 36.4 s | 56.8 s | 43.7 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 43.3 s | 46.0 s | 100.7 s | 85.6 s |

<!-- end of what cargo xtask ci-times wrote -->

The pull request's tiers and main's ran on different runners, so they are not compared here: the tiers on one runner are below. Linux's tier 1 is over its 20 s budget on main (20.2 s) and on the pull request (21.1 s) alike, a warning that blocks nothing.

## Both trees on one runner

`try --final` (run 45943838358), Windows, the pull request against `main` at `5eda620`, the median of 2 rounds each, on one runner at 5.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 15.9 s (3,606 tests) | 17.6 s (3,571) | −1.7 s | 25 s |
| 2, integration | 34.4 s (1,857 tests) | 36.6 s (1,844) | −2.2 s | 40 s |
| 3, system | 61.9 s (267 tests) | 66.0 s (266) | −4.2 s | 90 s |

No tier is slower: the new tests (the most, `cineris-tui`'s, +0.37 s in tier 1) cost less than the runner's own swing between the rounds. The shipped Linux binary in the same run: 17,574,968 bytes, +61,440 against main's, within its 20 MB budget.
