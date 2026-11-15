# After step 16: statistics, roles and variants

Step 16 built `cortex-stats` and wired it into the loop and the core. It also added `cortex_base::zone`, the turn-time split, roles with budgets of their own, variant points, per-day limits, `usage/report`, `usage/refresh`, `cortex usage` and `cortex stats`. It added no dependency: `tracing` in `cortex-agent`, and `serde` and `serde_json` in `cortex-stats`, were in the build already.

Measured on the step's pull request (#52) on 2026-11-15, against the main branch's run at `143e7f1`, the base the branch was made from:

| Job | Main, `143e7f1` (run 43693618771) | The PR at `1b66819` (run 43698474363) |
|---|---|---|
| `lint-linux` | 41 s | 38 s |
| `test-linux` | 95 s | 107 s |
| `lint-windows` | 76 s | 77 s |
| `test-windows` | 196 s | 189 s |

| Time above process start, against the budget | Linux, main (6 ms) | Linux, `1b66819` (5 ms) | Windows, main (10 ms) | Windows, `1b66819` (7 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 8.8 s | 10.3 s | 15.9 s | 10.5 s |
| Tier 2, integration (40 s) | 18.1 s | 19.3 s | 33.6 s | **24.7 s** |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 14.2 s | 17.1 s | 92.8 s | 67.8 s |

- **The two Windows runs are not on runners of the same speed.** A test's process started in 10 ms for main's run and 7 ms for the PR's ([AGENTS.md](../../../AGENTS.md), "Hosted Windows runners differ in speed"). So the Windows columns say only that every tier is within its budget, not what the step costs there.
- **On Linux, where the start cost is about the same,** the step adds about 1.5 s to tier 1, 1.2 s to tier 2 and 2.9 s to tier 3. Tier 3 is the new `sys_cortex_usage_reads_the_local_files_and_connects_to_nothing`, which starts the binary five times, and the scripted servers of the refresh's tests. Tier 1 is the zone's and the timing's unit tests. Tier 2 is the new `cortex-stats` integration tests and the core's `usage` tests.
- **At the desk** a run of `cargo xtask ci` while another session's mutation run held the machine put tier 3 at 156% and tier 2 at 112%. 80 s of tier 3 was `cortex-session`'s git tests, which this step does not touch. The hosted runs above are the measurement.
- **The Windows `try`** on the branch before the PR (run 43698008737) ran 1,304 tests of `cortex-stats`, `cortex-base`, `cortex-core`, `cortex-agent`, `cortex-cli`, `cortex-config` and `cortex-llm`, and all passed. That includes the statistics files' owner-only checks, the monthly files and the zone read through the Windows API.

## After the review: the fixes' pull request (#55)

Measured on 2026-11-15, against the main branch's run at `76bed87`, which the branch was rebased on.

| Job | Main, `76bed87` (run 43719893293) | The PR at `fe80c82` (run 43721486747) |
|---|---|---|
| `lint-linux` | 45 s | 44 s |
| `test-linux` | 112 s | 76 s |
| `lint-windows` | 65 s | 91 s |
| `test-windows` | 173 s | 205 s |

| Time above process start, against the budget | Linux, main (6 ms) | Linux, `fe80c82` (4 ms) | Windows, main (8 ms) | Windows, `fe80c82` (10 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 10.7 s | 6.3 s | 12.7 s | 16.1 s |
| Tier 2, integration (40 s) | 19.6 s | 13.5 s | 27.5 s | 34.1 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 13.6 s | 11.1 s | 52.4 s | 63.2 s |

- **The PR's Windows runner was the slower kind** (10 ms a process start against main's 8 ms). The rise was spread over crates the fixes do not touch: `cortex-tools`, `cortex-llm`, `cortex-tui` and `xtask` together moved more than `cortex-stats` and `cortex-core`.
- **Both trees on one Windows runner** (`try` with `-f against=main`, run 43721859296, the median of 5 runs each, at 8.0 ms a process start):

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 11.1 s (1,728 tests) | 11.5 s (1,721) | −0.4 s | 20 s |
| 2, integration | 25.9 s (1,202 tests) | 25.7 s (1,170) | +0.2 s | 40 s |
| 3, system | 46.8 s (134 tests) | 46.8 s (133) | +0.0 s | 90 s |

  The fixes cost 0.2 s of tier 2 for 32 more integration tests, the aggregates' flushed saves included.
- **The Windows `try`** before the PR (run 43721236679), `test -p cortex-stats -p cortex-core -p cortex-base`: 461 tests and 15 system tests, all passed. They include the reset while a writer holds the month's file, the stuck writer held at the limits file's lock, two processes' recorders, and the file identity through the Windows API.
