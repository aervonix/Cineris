# After the fifth tooling step: tier 2's time on Windows, failure text in stress, edits by script, a rig that cannot hang on a question, and the owner's colours

The pull request's runs are recorded here when it is open.

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

The step changes no test's time on either OS: every tier is within its noise of `main`'s on one runner. The Windows runner was a slow one (11 ms a process start), on which `main`'s tier 2 is 37.5 s of 40; the step's headroom for tier 2 is the split of the test job, which is the owner's to decide ([outstanding/running-the-system-tests-after-tiers-one-and-two.md](../../outstanding/running-the-system-tests-after-tiers-one-and-two.md)).

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
| 43665893748 | Both OSes, the layout experiment (scratch branch at `45fd2d1`), five rounds of three layouts | [outstanding/running-the-system-tests-after-tiers-one-and-two.md](../../outstanding/running-the-system-tests-after-tiers-one-and-two.md) |
