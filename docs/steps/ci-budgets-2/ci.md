# After CI budgets II: a Windows session's open made cheaper, and the tests that run it

The CI times before and after, and the runs that measured them. What was found and changed is in [answered.md](answered.md).

## Before: `main`'s own CI at `42c90dcc`

Run 45721643017, the push to `main` of step 28's script hooks, on a Windows runner at 6 ms a process start:

| Job | Main, `42c90dc` |
|---|---|
| `lint-linux` | 88 s |
| `test-linux` | 148 s |
| `lint-windows` | 111 s |
| `test-windows` | 265 s |

| Time above process start, against the budget | Linux, main (4 ms) | Windows, main (6 ms) |
|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 11.5 s | 18.8 s |
| Tier 2, integration (40 s) | 26.0 s | **43.8 s** |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 39.4 s | 82.3 s |

## After: both trees on one Windows runner

The branch at `ed5cfbc` (the walk's look by attributes, the rigs' Windows PowerShell, the directories left on a hosted runner) against `main` at `42c90dc`, three rounds each, on one runner at 5 to 6 ms a process start. In these tables **"The PR" is the branch and "Main" is `main`**. The walk's change alone, on another 6 ms runner (`try` 45728496329): tier 2 44.6 s on `main` against 43.5 s.

<!-- written by cargo xtask ci-times, section "after-one-runner-ab": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 45730634759), the median of 3 runs each, on one runner at 6.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 11.6 s (3,426 tests) | 12.7 s (3,424) | −1.1 s | 25 s |
| 2, integration | 31.2 s (1,826 tests) | 36.7 s (1,826) | −5.6 s | 40 s |
| 3, system | 62.8 s (233 tests) | 62.5 s (233) | +0.2 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "after-one-runner-ab" -->
