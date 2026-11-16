# After step 23: the interactive application

Step 23 opens the interface on a real session. Its first pull request (58) builds the application's loop, the mapping of the session's events, the input wired, the approval dialog, interactive mode in `cortex-cli`, and the end-to-end tests of the real binary under a pseudo-terminal. Its second builds the rest (`answered.md`, "The design, before building (reviewed)", point 5). It adds no third-party package: `cortex-tui` now uses `serde_json`, which the workspace already had.

**What it adds to the tiers.** The four end-to-end tests of the interface run in tier 3. On Linux the slowest of them is the approval's, 0.38 s, most of it the dialog's 300 ms grace before its answers can be pressed; on Windows none of them is among the five slowest. The application's own tests are unit tests in `cortex-tui` (tier 1), run on scripted links and a terminal played back on a clock of its own, so none waits.

The first part's Windows `try` run (43785122305, `test -p cortex-tui -p cortex-cli -p cortex-core -p cortex-term`, at `762d7eb`): 544 unit and integration tests and 53 system tests, every one passed, at 5.0 s of 20, 8.2 s of 40 and 18.9 s of 90 above the start of their processes.

The tables below compare two runners of the same start cost on Windows (10 ms), and close ones on Linux. Each tier of the pull request is at or under main's. Windows' tiers 1 and 2 are just over their budgets on both trees (20.4 s of 20 and 42.1 s of 40 here, 21.4 s and 42.5 s on main): warnings, under the failure at one and a half times, there before this step; Windows tier 2's margin is tooling 7's first job.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 58, on `main` at `bd2e6a1`. "Main" is main's own run at `bd2e6a1` (run 43773439421).
The PR's run is 43785396220, at `762d7eb`.

| Job | Main, `bd2e6a1` | The PR at `762d7eb` |
|---|---|---|
| `lint-linux` | 47 s | 50 s |
| `test-linux` | 98 s | 91 s |
| `lint-windows` | 73 s | 75 s |
| `test-windows` | 214 s | 215 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `762d7eb` (3 ms) | Windows, main (10 ms) | Windows, `762d7eb` (10 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 10.1 s | 7.4 s | 21.4 s | 20.4 s |
| Tier 2, integration (40 s) | 24.0 s | 12.5 s | 42.5 s | 42.1 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 28.0 s | 11.7 s | 74.3 s | 66.7 s |

<!-- end of what cargo xtask ci-times wrote -->
