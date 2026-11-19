# After the harness's own processes' fixes: an environment built for git and the probe, and programs found outside the workspace

The fix adds six `sys_` tests: two in `cineris-base` (a filter git runs is given no key, and is never found through a relative `PATH` entry; the second on Linux only), one in `cineris-tools` (the probe's environment, on both systems), and three in `cineris-core` (two on Linux, the reload of a header on both). It also adds unit tests in all three crates. A `cineris-base` change rebuilds every crate, so both test jobs ran cold.

**Every blocking job passed within its limit**, and no tier's time moved against `main`'s run at the same commit. `test-windows` took 281 s against `main`'s 342 s, on a runner whose processes started faster (7 ms against 9 ms). The tiers that warn on Windows (tier 1 at 22.2 s against 20 s, tier 3 at 99.5 s against 90 s) warn on `main`'s run too, by more (23.4 s and 103.4 s).

## The pull request's run

<!-- written by cargo xtask ci-times, section "The pull request's run": it replaces this up to the end mark -->
Pull request 111, on `main` at `6c43dfb`. "Main" is main's own run at `6c43dfb` (run 44592310472).
The PR's run is 44610509750, at `b29feaa`.

| Job | Main, `6c43dfb` | The PR at `b29feaa` |
|---|---|---|
| `lint-linux` | 60 s | 72 s |
| `test-linux` | 160 s | 131 s |
| `lint-windows` | 102 s | 98 s |
| `test-windows` | 342 s | 281 s |
| `pr-size-linux` (blocks nothing) | — | 230 s |
| `ts-client` (blocks nothing) | 0 s | 8 s |
| `pr-size-windows` (blocks nothing) | — | 423 s |

| Time above process start, against the budget | Linux, main (6 ms) | Linux, `b29feaa` (5 ms) | Windows, main (9 ms) | Windows, `b29feaa` (7 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 27.4 s | 18.5 s | 23.4 s | 22.2 s |
| Tier 2, integration (40 s) | 34.3 s | 30.5 s | 45.1 s | 38.8 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 36.8 s | 36.7 s | 103.4 s | 99.5 s |

<!-- end of what cargo xtask ci-times wrote, section "The pull request's run" -->
