# After stop-ends-turn: a stop from a call's run ends the turn

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 123, on `main` at `36d8266`. "Main" is main's own run at `36d8266` (run 44886379853).
The PR's run is 44913035521, at `d70d34b`.

| Job | Main, `36d8266` | The PR at `d70d34b` |
|---|---|---|
| `lint-linux` | 83 s | 79 s |
| `test-linux` | 143 s | 126 s |
| `lint-windows` | 108 s | 118 s |
| `test-windows` | 255 s | 178 s |
| `pr-size-linux` (blocks nothing) | — | 225 s |
| `ts-client` (blocks nothing) | — | 6 s |
| `pr-size-windows` (blocks nothing) | — | 423 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `d70d34b` (3 ms) | Windows, main (6 ms) | Windows, `d70d34b` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 11.4 s | 8.8 s | 19.7 s | 16.8 s |
| Tier 2, integration (40 s) | 22.9 s | 15.3 s | 42.9 s | 38.6 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 51.8 s | 31.4 s | 78.3 s | 76.9 s |

<!-- end of what cargo xtask ci-times wrote -->

## The final `try` run

Run 44912996180, at `d70d34b`, against `main` at `36d8266` (`cargo xtask try --final`).

**Windows, both trees on one runner** (8.0 ms a process start), the median of 5 runs each:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 22.7 s (3,313 tests) | 22.3 s (3,312) | +0.4 s | 25 s |
| 2, integration | 52.5 s (1,747 tests) | 52.0 s (1,740) | +0.5 s | 40 s |
| 3, system | 89.1 s (227 tests) | 88.9 s (226) | +0.2 s | 90 s |

The new tests account for most of the change: the extension's last-call test (+0.06 s, tier 2) and the headless exit-6 test (+0.22 s, tier 3); the agent rig's tests together +0.19 s. Tier 2 is over its budget on this runner on both sides, as it is on `main` (the open item `ci-budgets`); this change adds 0.5 s to it.

One test failed on main's side only, in round 4, and is left out of the comparison: `cineris-cli::system tui::sys_a_turn_is_interrupted_mid_stream_and_mid_tool`, a panic in the test kit's pseudo-terminal (`pty.rs:259`) on the busy runner. It passed on the PR's side in every round. The run reports a failure for it all the same.

**Linux 22.04** (Landlock ABI 4): every test passed. Tiers above the start: 12.7 s, 32.2 s and 40.8 s, against 20 s, 40 s and 60 s.

**Size** (`x86_64-unknown-linux-musl`, `dist`): 16,210,840 bytes, budget 20 MB; +4,096 bytes against `main`.
