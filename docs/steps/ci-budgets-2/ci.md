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

## A pull request that touches `cineris-base`, on an 8 ms runner

The test job of both trees on one Windows runner, at 8 ms a process start in every round (`try` 45731497689: `test --job --keep-build`, four rounds, one in each order cold and two warm; `.github/scripts/touch-base.sh`, the setup of both sides, touches `cineris-base` so that a warm round rebuilds every crate of the workspace, as such a pull request's job does, its third-party crates already built). The branch at `b7d13659` (the walk's look, the rigs' Windows PowerShell, the directories left on a hosted runner; not yet the four other walks or the dialogs' hook). The head's tiers 1 and 2's run failed in every round on one test of the test kit, fixed since (`scratch::tests::a_directory_outside_leaves_no_parent_behind_once_the_last_is_gone`, which assumed a desk), so the tool left the head's sums out of its medians; the rounds' own figures are below.

| Phase, the warm rounds (3 and 4) | Main | The branch |
|---|---|---|
| The build, every crate of the workspace | 125.3, 126.4 s | 126.7, 127.8 s |
| Tiers 1 and 2's run | 64.3, 63.3 s | 61.5, 60.0 s |
| The system tests' run | 48.3, 49.9 s | 48.5, 48.0 s |
| The job as `ab --job` times it (setup, build, runs) | 238.0, 239.6 s | 236.8, 235.8 s |
| Tier 1, above the start | 23.9, 23.9 s | 25.0, 23.4 s |
| **Tier 2, above the start** | **60.2, 58.3 s** | **53.5, 52.2 s** |
| Tier 3, above the start | 91.0, 93.4 s | 90.8, 89.9 s |

By crate, tier 2, the medians of the two warm rounds: `cineris-core` 29.69 to 25.00 s (−4.69), `cineris-config` −0.51, `cineris-agent` −0.41, `cineris-rpc` −0.28, the rest within 0.2 s; in all 59.3 to 52.8 s. The cold rounds read the same: tier 2 63.2 s on `main` and 52.0 s on the branch.

**The job's time on this runner:** the runner's own steps before the tests (the checkout, the toolchain, the build cache, nextest) took about 31 s in `main`'s own CI (run 45721643017), so such a pull request's test job is about 270 s here, on either tree: under the five minutes by about 30 s. What the branch saves in the tests' runs (about 3 s of wall time; the tiers are summed over two tests at a time) the build does not change. This runner is as slow as the one that cut step 28's fourth pull request's job at 5 min 5 s (run 44911679902, its build 131 s and tiers 1 and 2 60.8 s), whose system tests had not ended when the job was cut; here they took 48 s on either tree, so what made that job run long was not the tests' work as this A/B measures it, and one run does not say what it was. The build is about half the job, and it is the next lever: [testing-and-ci.md](../../testing-and-ci.md), "Where the five minutes go".
