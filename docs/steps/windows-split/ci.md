# After the Windows split: Windows' tests built in one job and run in two

The CI times of the split jobs, against `main`'s `test-windows`, and the runs that measured them. What was built and decided is in [answered.md](answered.md).

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 135, on `main` at `5eda620`. "Main" is main's own run at `5eda620` (run 45931692729).
The PR's run is 45951972676, at `72f566b`.

| Job | Main, `5eda620` | The PR at `72f566b` |
|---|---|---|
| `lint-linux` | 85 s | 87 s |
| `test-linux` | 182 s | 200 s |
| `lint-windows` | 120 s | 96 s |
| `build-windows` | — | 189 s |
| `test-windows` | 351 s | — |
| `test-windows-tiers` | — | 103 s |
| `test-windows-system` | — | 72 s |
| `pr-size-linux` (blocks nothing) | — | 271 s |
| `ts-client` (blocks nothing) | — | 63 s |
| `pr-size-windows` (blocks nothing) | — | 442 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `72f566b` (5 ms) | Windows, main (8 ms) | Windows, `72f566b` (8 ms; tier 3 6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 20.2 s | 18.9 s | 27.6 s | 26.4 s |
| Tier 2, integration (40 s) | 31.6 s | 37.1 s | 56.8 s | 58.1 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 43.3 s | 46.5 s | 100.7 s | 70.0 s |

<!-- end of what cargo xtask ci-times wrote -->

`main` at `e3c0175`, which the pull request is on, has no successful run: its `test-windows` failed its tier check (run 45951053644, the integration tier at 152% of its budget on an 8 ms runner, [main-windows-tiers-over-budget-on-slow-runners](../../outstanding/main-windows-tiers-over-budget-on-slow-runners.md)), so "Main" is its run before. The pull request's run builds every crate the merge of step 25's fixes changed, against the build `main` saved at `5eda620`: most of the workspace.

## Each job, and the time to the last required job

Runs of the `ci` workflow, 2026-11-27. Each new layout's run is this branch with one change on top; each old one is `main` at `5eda620` with the same change on top, dispatched beside it (`gh workflow run ci --ref …`: twelve minutes a job, not five, and the caches a pull request reads). "Start" is each tier report's process start: the tiers job's from its own tests, the system job's from its start probe (621 unit tests, 2.8 s at two at a time). "To the last" is from the run's creation to the end of its last required job; the runs marked † waited 76 to 86 s for a Windows runner before their first job started, which is in it.

| Change | Layout | Run | `build-windows` | `test-windows-tiers` (start) | `test-windows-system` (start) | `test-windows` (start) | To the last |
|---|---|---|---|---|---|---|---|
| `cineris-base` | split | 45949784135 | 183 s | 94 s (8 ms) | 98 s (8 ms) | | 287 s |
| `cineris-base` | split | 45951053150 | 170 s | 87 s (6 ms) | 107 s (7 ms) | | 282 s |
| `cineris-base` | one job | 45951058598 † | | | | 325 s (9 ms), its tier 2 far over | 402 s |
| `cineris-base` | one job | 45951969663 | | | | 251 s (7 ms) | 254 s |
| `cineris-stats` | split | 45951056029 | 104 s | 99 s (8 ms) | 83 s (6 ms) | | 242 s |
| `cineris-stats` | split | 45951966817 | 139 s | 99 s (8 ms) | 101 s (8 ms) | | 245 s |
| `cineris-stats` | one job | 45951061724 † | | | | 168 s (6 ms) | 263 s |
| `cineris-stats` | one job | 45951972079 | | | | 227 s (8 ms), its tier 2 far over | 231 s |
| `xtask` (the pull request's first run) | split | 45949754351 | 86 s | 88 s (6 ms) | 83 s (6 ms) | | 210 s |
| step 25's crates (the pull request, rebased) | split | 45951972676 | 189 s | 103 s (8 ms) | 72 s (6 ms) | | 296 s |

- **Every new job is far inside its five minutes:** the build 86 to 189 s, the tests 72 to 107 s, the system tests' job with its probe at most 107 s. The one job took 325 s on a slow runner for a change to `cineris-base`, past the five minutes a pull request has, which is what this change is for.
- **To the last required job is about 30 s longer** for a change to `cineris-base` on runners of the same speed (282 and 287 s against 254 s), and about 15 s for a change to one crate (242 and 245 s against 231 s): the test jobs' own set-up (the checkout, the toolchain, nextest, the archive's download and its extraction, about 25 s) after the build, less the build's share of the old job's set-up. The archive's upload took 4 s and its download 3 to 7 s.
- **The two runs of the `main` layout that failed** failed their tier check on slow runners, `main`'s own (the open item above), not the change on top.

## The tiers

Each tier is judged as it was, above the start of a test's process on the runner that ran it. Tier 3, above the start, by the start of its runner:

| Start | Split (system tests' job) | One job |
|---|---|---|
| 6 ms | 79.4, 82.0, 70.0 s | 78.3 s |
| 7 ms | 119.0, 100.3 s | 82.3 s |
| 8 ms | 105.5, 107.0 s | 100.7, 105.7 s |
| 9 ms | | 102.8 s |

The split runs are within the one job's spread at 6 and 8 ms; at 7 ms one run read 119.0 s (132% of the budget, a warning, under the failure at 135 s). Hosted runners at one start still differ: the comparison of record for a change is both trees on one runner, which a split run cannot give for the job layout itself. Tiers 1 and 2 run as they did, in one job, and read as `main`'s on runners of the same start (the table above, and the pull request's: 26.4 and 58.1 s at 8 ms, against `main`'s 27.6 and 56.8 s).

**One run's tier reports**, the pull request's first (45949754351). `test-windows-tiers`:

```text
  process start: 6 ms a test (the 1st percentile of the run's 5439 unit and integration tests' times), taken off every test before a tier is held to its budget
  tier             tests   time    above start  budget  used
  1, unit           3595   40.8 s       19.2 s    25 s   77%
  2, integration    1844   53.1 s       42.0 s    40 s  105%  over budget (warning)
```

`test-windows-system`:

```text
  process start: 6 ms a test (the 1st percentile of the 621 unit tests' times of the start probe, run first on this runner to measure it), taken off every test before a tier is held to its budget
  tier             tests   time    above start  budget  used
  3, system          266   81.0 s       79.4 s    90 s   88%
```

## A test that fails on purpose

Run 45951064742, this branch with a unit test of `cineris-config` and a system test of `cineris-cli` that panic: `test-windows-tiers` failed on the one (`cineris-config fails_on_purpose::a_unit_test_that_fails_on_purpose`) and `test-windows-system` on the other (`cineris-cli::system sys_a_system_test_that_fails_on_purpose`), each after the rest of its tests ran; `build-windows` passed, and `test-linux` failed on both.

**A build that fails.** Run 45953479706, this branch with a line `cineris-stats` cannot compile on Windows: `build-windows` failed, and `test-windows-tiers` and `test-windows-system` ran and failed in 4 and 5 s at their first step ("The build job ended as 'failure'"), rather than be skipped, which would have read as passed to a required check.
