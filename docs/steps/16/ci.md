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
