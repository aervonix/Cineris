# After step 27: the RPC server

Step 27 puts the protocol on a wire: `cineris serve --stdio` in its first pull request (81), `--socket` with its permissions, several clients and the core's changes in its second, and `cineris attach` with the TypeScript client in its third (`answered.md`, "The design, before building", point 7). The first adds no third-party package: `cineris-rpc` now uses `tokio` (`rt`, `sync`), `serde`, `serde_json` and `tracing`, which the build already had.

**What it adds to the tiers.** `cineris-rpc`'s tests are tier 2 and run in process against a real core reached over the real HTTP client, every wait on the fake timer; the conformance test runs its eleven scenarios on one core a carrier, 0.67 s on a Windows runner. Its three system tests start the real binary: the conformance over standard I/O is the slowest, 1.15 s on Windows. On one Windows runner the pull request's tiers against `main`'s (try 43924915298, two rounds each, 7 ms a process start): unit 12.3 s against 12.9 s, integration 28.6 s against 29.0 s (`cineris-rpc` +0.34 s, its conformance test left out of that run, which failed there on a path's spelling, since fixed), system 57.6 s against 64.7 s.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 81, on `main` at `7f82023`. "Main" is main's own run at `7f82023` (run 43920695997).
The PR's run is 43927222432, at `5e186c2`.

| Job | Main, `7f82023` | The PR at `5e186c2` |
|---|---|---|
| `lint-linux` | 74 s | 71 s |
| `test-linux` | 115 s | 105 s |
| `lint-windows` | 95 s | 99 s |
| `test-windows` | 238 s | 265 s |
| `pr-size-linux` (blocks nothing) | — | 376 s |
| `pr-size-windows` (blocks nothing) | — | 349 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `5e186c2` (4 ms) | Windows, main (7 ms) | Windows, `5e186c2` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 14.5 s | 12.4 s | 11.8 s | 18.7 s |
| Tier 2, integration (40 s) | 45.7 s | 22.4 s | 28.2 s | 43.4 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 29.0 s | 24.6 s | 61.9 s | 79.8 s |

<!-- end of what cargo xtask ci-times wrote -->

**Windows's integration tier on the pull request's run** read 43.4 s, over its budget of 40 s (a warning; the failure is at 60 s), on a runner of 9 ms a process start, where `main`'s run had one of 7 ms. The one-runner A/B above is the comparison that holds: on one 7 ms runner the pull request's tier 2 was 28.6 s against `main`'s 29.0 s without the conformance test, which adds 0.67 s, so about 29.3 s.
