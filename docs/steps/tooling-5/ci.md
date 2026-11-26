# After the fifth tooling step: tier 2's time on Windows, failure text in stress, edits by script, a rig that cannot hang on a question, and the owner's colours

This step added tools and test support, a little test-only code in `cineris-term` (two signal helpers behind its `pty` feature) and the owner's colours in `cineris-tui`'s theme. No dependency was added. The tables were made with `cargo xtask ci-times`, the A/B sections written by it.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 50, on `main` at `8a894c4`. "Main" is main's own run at `8a894c4` (run 43690617039).
The PR's run is 43691685559, at `ef5009d`.

| Job | Main, `8a894c4` | The PR at `ef5009d` |
|---|---|---|
| `lint-linux` | 36 s | 45 s |
| `test-linux` | 90 s | 80 s |
| `lint-windows` | 83 s | 61 s |
| `test-windows` | 123 s | 183 s |

| Time above process start, against the budget | Linux, main (5 ms) | Linux, `ef5009d` (4 ms) | Windows, main (6 ms) | Windows, `ef5009d` (10 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 10.3 s | 7.7 s | 8.3 s | 13.3 s |
| Tier 2, integration (40 s) | 18.3 s | 14.0 s | 21.1 s | 30.2 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 15.4 s | 13.9 s | 52.1 s | 82.6 s |

<!-- end of what cargo xtask ci-times wrote -->

The two runs were on runners of different speed (start 5 and 4 ms on Linux, 6 and 10 ms on Windows), so the A/B sections below are the comparison. On the PR's run Windows' tier 2 read 30.2 s at 10 ms a process start, where the A/B's branch side read 24.8 s on a runner of the same start cost: runners of one start cost still differ by a fifth under load. Before the rebase on #49, the PR's first run of `test-windows` (43672945463) failed at a flake of `cineris-tools`, which the step did not change, on a loaded runner ([outstanding/a-git-look-too-slow-on-a-loaded-windows-runner.md](../../outstanding/a-git-look-too-slow-on-a-loaded-windows-runner.md)); its rerun passed. The static checks: `rules` 0.7 s and `breaks` 0.3 s at the desk for 921 breaks, as before the step.

## Windows, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 43691692866), the median of 5 runs each, on one runner at 10.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 10.6 s (1,565 tests) | 13.7 s (1,528) | −3.1 s | 20 s |
| 2, integration | 24.8 s (1,064 tests) | 31.3 s (1,059) | −6.6 s | 40 s |
| 3, system | 76.5 s (132 tests) | 77.8 s (132) | −1.3 s | 90 s |

<!-- end of what cargo xtask ci-times wrote -->

## Linux, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 43691692866), the median of 5 runs each, on one runner at 3.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 5.8 s (1,582 tests) | 5.8 s (1,545) | +0.0 s | 20 s |
| 2, integration | 9.7 s (1,066 tests) | 9.9 s (1,061) | −0.2 s | 40 s |
| 3, system | 7.2 s (185 tests) | 9.3 s (182) | −2.1 s | 60 s |

<!-- end of what cargo xtask ci-times wrote -->

**The split, on the rebased tree (`main` at `8a894c4`, with #49's tests).** The branch's job runs tiers 1 and 2 and then the system tests alone, three at a time; `main`'s runs them in one, the system tests two at a time beside the rest (`ab` runs each tree as its own job does). On one Windows runner (10 ms a process start), tier 2 is **24.8 s against 31.3 s, 21% less**, and tier 1 23% less (10.6 s against 13.7 s), with the system tier 1.3 s less. On Linux tiers 1 and 2 are unchanged within noise and the system tier is 2.1 s less. The experiment before it (`try` run 43665893748) had said 21% for Windows' tier 2.

**Item 1's target is met:** Windows' tier 2 is 24.8 s against `main`'s 31.3 s on one runner, 21% less. It is 24.8 s of its 40 on this runner, under the 30 s the step was asked for. The coordinator's reading was a cut of about a quarter, so that a slow runner's 38 s lands near 29 s: the cut is 21%, and on the slowest Windows runner seen (`main`'s 38.3 s at 10 ms, run 43639354819) it would land near 30.2 s, at the line; the PR's own run read 30.2 s too. No test was moved or weakened, and the budgets are unchanged.

**The runner estimate, measured again for the split.** The first factor table, of `main` in one run, read Windows' tier 2 of the split job at 37.4 s against 24.8 s, 51% high, and Linux's at 14.8 s against 9.7 s. The table was measured again from the desk's quiet run of this tree (`ef5009d`) and the branch's side of the A/B above, and the slow runner's ratio from the PR's own CI run (1.44 on Linux, 1.22 on Windows). A second quiet run at the desk an hour later, which the table was not measured from, came within 15% on every tier of both OSes: tier 2 8.8 s against 9.7 s on Linux and 21.1 s against 24.8 s on Windows, low; tiers 1 and 3 4 to 8% high. The desk's own tier 2 was 14.2 s on the first run and 9.7 s on the second: the estimate is as steady as the desk's disk ([testing-and-ci.md](../../testing-and-ci.md), "What a runner would say, from a desk").

**Before the rebase,** on `main` at `9ddbf67` and the job in one run, the branch changed no tier beyond its noise: on one Windows runner (11 ms) tier 2 was 37.4 s against 37.5 s (run 43671665085), and on Linux 12.4 s against 12.6 s (run 43672128881).

## Other runs

| Run | What | Result |
|---|---|---|
| 43691692866 | Both OSes, A/B against `main` at `8a894c4`, five rounds | The sections above |
| 43692957473 | The PR's `ci` at `81c5589` | `test-windows` failed on the tier budgets alone, every test passing, on the slowest Windows runner seen (12 ms a process start): tier 1 22.2 s, tier 2 41.6 s, tier 3 138.8 s, each about twice the A/B's at 10 ms; rerun ([outstanding/windows-tier-three-margin-and-the-powershell-tests.md](../tooling-6/answered.md)) |
| 43691694394 | Linux, `advisories`, after the ignore of RUSTSEC-2025-0141 | Passed |
| 43691696481 | Windows, release build, `test -p cineris-testkit --run-ignored only watch::tests::sys_ --no-capture`, five repeats | Passed; the timeout report's tree 2.27 s after its waits the first time, 0.32 to 0.34 s after |
| 43671662851 | Windows, `test -p cineris-term -p cineris-tui -p cineris-core -p cineris-agent -p cineris-testkit -p xtask` | Passed |
| 43671665085 | Both OSes, A/B against `main` at `9ddbf67` before the rebase, five rounds | Windows: tier 2 37.4 s against 37.5 s. Linux ended in its first round on the branch's side, at a flake of `cineris-core`'s base capture (`a-base-capture-beside-the-turns-git`); run again as 43672128881 |
| 43667663980 | Both OSes, A/B of `cineris-llm` alone against `main`, five rounds | 2 min 24 s on Windows, 1 min 20 s on Linux, from start to end of the job |
| 43663369925, 43663367508 | Tier 2 alone, profiled, on the scratch branch at `6e96c25` | Linux 14.2 s, Windows 21.2 s above the start |
| 43663690282 | Both OSes, A/A of `main` (the scratch branch at `6e96c25`), three rounds | Windows tier 2 27.1 s (start 8 ms), Linux 10.8 s (start 3 ms) |
| 43664837834, 43665361002 | Windows, the two timing probes (scratch branch at `089a40a` and `e48399c`) | [performance-budgets.md](../../performance-budgets.md), "Open a session, on Windows against Linux" |
| 43665893748 | Both OSes, the layout experiment (scratch branch at `45fd2d1`), five rounds of three layouts | [testing-and-ci.md](../../testing-and-ci.md), "Each tier is measured under its own load" |
