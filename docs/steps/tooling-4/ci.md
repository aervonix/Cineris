# After the fourth tooling step: the owner guard before a push, paths by place, try's named jobs, test profiles, the pressure helper and scratch tests

This step added tools and test support, and a little product code (the comparison of paths by place in `cineris-base`, and the six places that use it). No dependency was added. The tables were made with `cargo xtask ci-times`.

## On the pull request

Pull request 46, on `main` at `4a23998`. "Main" is main's own run at `4a23998`.

| Job | Main, run 43620218786 | The PR at `b518e5e`, run 43624377361 |
|---|---|---|
| `lint-linux` | 44 s | 48 s |
| `test-linux` | 66 s | 65 s |
| `lint-windows` | 62 s | 70 s |
| `test-windows` | 192 s | 171 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, the PR (4 ms) | Windows, main (10 ms) | Windows, the PR (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 3.9 s | 3.5 s | 13.8 s | 11.5 s |
| Tier 2, integration (40 s) | 10.2 s | 10.6 s | 44.1 s | 32.5 s |
| Tier 3, system (60 s, 90 s on Windows) | 12.0 s | 9.9 s | 85.5 s | 80.2 s |

- **On Linux the step costs nothing that stands out of the noise.** By crate (`ci --tiers --compare`): the step's own new tests are `xtask`'s +0.8 s of the system tier (the local guard's tests start git, the two slowest 0.17 s each), and nothing else it changed moved more than 0.1 s; `cineris-agent`'s integration tier, with the pressure tests and their twins, moved +0.0 s. The system tier's −2.2 s is one PowerShell test of `cineris-policy` that took 4.0 s on main's runner and 0.6 s on this one; `cineris-session`'s +0.4 s of integration is its durability tests, which the step did not touch.
- **The Windows columns are two runners of different speed** (main's start cost 10 ms, the PR's 9 ms, and main's tiers slower everywhere): read for the share of each budget, not for the change. The system tier at 80 of 90 s is where it was before the step.
- **The static checks:** `rules` 0.6 s and `breaks` 0.2 s on the desktop for 743 breaks, as before; the step adds no check step (actionlint was tried and taken out: [outstanding/actionlint-in-the-checks.md](../../outstanding/actionlint-in-the-checks.md)).

## On the runners through `try`, on this branch

| Run | What | Result |
|---|---|---|
| 43624080450 | Windows, `test -p cineris-base -p cineris-config -p cineris-core -p cineris-sandbox -p cineris-testkit -p xtask` | 964 passed |
| 43624083017 | Ubuntu 22.04, `partial-landlock` (a nightly job, through `try`) | Passed; "Landlock 4 does not scope signals: the gap the status names" |
| 43624085233 | Linux, `command=ci` (cargo-deny installed for it) | Passed: the static checks and every test |

Earlier, on a scratch branch: the same Windows run before the review's fixes (43617898000, 652 tests), `partial-landlock` (43617900231), and the first `command=ci` (43617902191), which installed cargo-deny and failed on two real problems of the branch at the time (formatting, and two of step 26's breaks the step had moved code from under), both fixed.
