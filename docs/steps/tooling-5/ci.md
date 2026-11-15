# After the fifth tooling step: tier 2's time on Windows, failure text in stress, edits by script, a rig that cannot hang on a question, and the owner's colours

This step added tools and test support, a little test-only code in `cortex-term` (two signal helpers behind its `pty` feature) and the owner's colours in `cortex-tui`'s theme. No dependency was added. The tables were made with `cargo xtask ci-times`, the A/B sections written by it.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 50, on `main` at `9ddbf67`. "Main" is main's own run at `9ddbf67` (run 43639354819).
The PR's run is 43672945463, at `0ce8622`.

| Job | Main, `9ddbf67` | The PR at `0ce8622` |
|---|---|---|
| `lint-linux` | 46 s | 45 s |
| `test-linux` | 115 s | 77 s |
| `lint-windows` | 72 s | 76 s |
| `test-windows` | 213 s | 151 s |

| Time above process start, against the budget | Linux, main (5 ms) | Linux, `0ce8622` (3 ms) | Windows, main (10 ms) | Windows, `0ce8622` (8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 11.3 s | 5.4 s | 17.8 s | 12.1 s |
| Tier 2, integration (40 s) | 18.0 s | 9.4 s | 38.3 s | 26.6 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 17.7 s | 13.4 s | 90.3 s | 68.5 s |

<!-- end of what cargo xtask ci-times wrote -->

The two runs were on runners of different speed (start 5 and 3 ms on Linux, 10 and 8 ms on Windows), so the A/B sections below are the comparison. The first run of `test-windows` failed at a flake of `cortex-tools`, which the step did not change, on a loaded runner ([outstanding/a-git-look-too-slow-on-a-loaded-windows-runner.md](../../outstanding/a-git-look-too-slow-on-a-loaded-windows-runner.md)); its rerun passed. The static checks: `rules` 0.7 s and `breaks` 0.3 s at the desk for 867 breaks, as before the step.

## Windows, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 43671665085), the median of 5 runs each, on one runner at 11.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 15.4 s (1,555 tests) | 16.2 s (1,525) | −0.8 s | 20 s |
| 2, integration | 37.4 s (1,036 tests) | 37.5 s (1,031) | −0.2 s | 40 s |
| 3, system | 87.2 s (132 tests) | 88.2 s (132) | −1.0 s | 90 s |

<!-- end of what cargo xtask ci-times wrote -->

## Linux, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 43672128881), the median of 5 runs each, on one runner at 5.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 6.0 s (1,572 tests) | 6.1 s (1,542) | −0.1 s | 20 s |
| 2, integration | 12.4 s (1,039 tests) | 12.6 s (1,034) | −0.2 s | 40 s |
| 3, system | 11.7 s (184 tests) | 11.8 s (181) | +0.0 s | 60 s |

<!-- end of what cargo xtask ci-times wrote -->

The step changes no test's time on either OS: every tier is within its noise of `main`'s on one runner. The Windows runner was a slow one (11 ms a process start), on which `main`'s tier 2 is 37.5 s of 40; the step's headroom for tier 2 is the split of the test job, measured below.

**The runner estimate, checked on this tree.** `cargo xtask ci --tiers --like-runner` at the desk, twice on a quiet desktop, gave for tier 2: Linux 14.2 and 14.8 s (25.2 and 26.2 on a slow runner), against 12.4 s on the A/B's Linux runner (start 5 ms); Windows 35.8 and 37.1 s (50.5 and 52.2), against 37.4 s on the A/B's slow Windows runner (start 11 ms). So on these runners it came within 15% on Linux, high, and within 5% on Windows, though that runner was a slow one, for which it said 50 s: the slow-runner figure is the worst runner seen, not the usual one. The desk's own tier 2 was 9.5 s for `main` on 2026-11-15 and 14.6 to 15.2 s for this branch later the same night, mostly `cortex-session`'s flushing tests on the desktop's virtual disk: the estimate is as steady as the desk's disk.

## Other runs

| Run | What | Result |
|---|---|---|
| 43671662851 | Windows, `test -p cortex-term -p cortex-tui -p cortex-core -p cortex-agent -p cortex-testkit -p xtask` | Passed |
| 43671665085 | Both OSes, A/B against `main`, five rounds | Windows as above. Linux ended in its first round on the branch's side, at a flake of `cortex-core`'s base capture ([outstanding/a-base-capture-beside-the-turns-git.md](../../outstanding/a-base-capture-beside-the-turns-git.md)); run again as 43672128881 |
| 43667663980 | Both OSes, A/B of `cortex-llm` alone against `main`, five rounds | 2 min 24 s on Windows, 1 min 20 s on Linux, from start to end of the job |
| 43663369925, 43663367508 | Tier 2 alone, profiled, on the scratch branch at `6e96c25` | Linux 14.2 s, Windows 21.2 s above the start |
| 43663690282 | Both OSes, A/A of `main` (the scratch branch at `6e96c25`), three rounds | Windows tier 2 27.1 s (start 8 ms), Linux 10.8 s (start 3 ms) |
| 43664837834, 43665361002 | Windows, the two timing probes (scratch branch at `089a40a` and `e48399c`) | [performance-budgets.md](../../performance-budgets.md), "Open a session, on Windows against Linux" |
| 43665893748 | Both OSes, the layout experiment (scratch branch at `45fd2d1`), five rounds of three layouts | [testing-and-ci.md](../../testing-and-ci.md), "Each tier is measured under its own load" |
