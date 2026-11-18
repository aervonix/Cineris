# After the rename to Cineris: the tool, and the tree in the new names

The rename changes names, not what is built or tested: the same tests run, and the binary holds the same code with longer names. What it adds to a run is the rename's check in the repo rules (about 0.3 s of `cargo xtask rules` at the desk) and the tool's own tests in `xtask` (20 tests, the slowest the branch rewrite's end to end at about 0.35 s on the laptop, then the whole-tree pass).

**Both A/B runs were made against `main` at `88e98f85`**, the `main` the pull request then sat on. The pull request was rebased onto `e2f1941d` afterwards, a step 25 fix in `cortex-term`, `cortex-tui` and the test kit that touched no part of the rename, and its tool output was made again there. The A/B runs still compare what the rename adds.

- **Windows' first A/B** (run 44372939141) showed tier 3 at 91.1 s against `main`'s 84.2 s, over its 90 s budget. 6.2 s of that was the rename's end-to-end branch test, which starts some 70 gits. That test now runs on Unix. A short test of the rewrite, a few gits long, runs on every system. The second A/B is below: +0.7 s.
- **Linux, tier 1:** +0.8 s for the 17 new unit tests, the rename's own among them. Tiers 2 and 3 moved down. Every crate is new under its new name, so those moves are the runner's noise.

## The shipped binary's size

`cargo xtask size --check --against-main` on a runner (`try` runs 44373001085 and 44373021142), against `main`'s last recorded size at `88e98f85`:

| Target | `main` | The pull request | Change | Budget |
|---|---|---|---|---|
| `x86_64-unknown-linux-musl` | 14,539,176 bytes | 14,551,464 bytes | +12,288 bytes (12 KB) | 16 MB |
| `x86_64-pc-windows-msvc` | 17,571,328 bytes | 17,571,328 bytes | +0 | 18 MB |

Both are within the 16 KB the rename may move them. The Linux binary grows by the names' extra character, in paths and messages: three pages of 4 KiB.

## Windows, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 44377861359), the median of 5 runs each, on one runner at 9.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 17.1 s (2,791 tests) | 16.2 s (2,774) | +0.9 s | 20 s |
| 2, integration | 37.8 s (1,527 tests) | 37.6 s (1,527) | +0.3 s | 40 s |
| 3, system | 81.6 s (199 tests) | 80.9 s (197) | +0.7 s | 90 s |

<!-- end of what cargo xtask ci-times wrote -->

## Linux, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 44372962015), the median of 5 runs each, on one runner at 5.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 15.0 s (2,810 tests) | 14.2 s (2,793) | +0.8 s | 20 s |
| 2, integration | 24.5 s (1,538 tests) | 28.6 s (1,538) | −4.1 s | 40 s |
| 3, system | 27.9 s (286 tests) | 28.8 s (283) | −0.9 s | 60 s |

<!-- end of what cargo xtask ci-times wrote -->

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 95, on `main` at `e2f1941`. "Main" is main's own run at `e2f1941` (run 44380789740).
The PR's run is 44383998597, at `77635c3`.

| Job | Main, `e2f1941` | The PR at `77635c3` |
|---|---|---|
| `lint-linux` | 59 s | 73 s |
| `test-linux` | 117 s | 144 s |
| `lint-windows` | 82 s | 97 s |
| `test-windows` | 293 s | 240 s |
| `pr-size-linux` (blocks nothing) | — | 225 s |
| `ts-client` (blocks nothing) | — | 68 s |
| `pr-size-windows` (blocks nothing) | — | 277 s |

| Time above process start, against the budget | Linux, main (5 ms) | Linux, `77635c3` (4 ms) | Windows, main (8 ms) | Windows, `77635c3` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 14.3 s | 23.2 s | 16.1 s | 13.7 s |
| Tier 2, integration (40 s) | 32.5 s | 35.3 s | 40.3 s | 31.5 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 27.7 s | 32.9 s | 87.3 s | 70.0 s |

<!-- end of what cargo xtask ci-times wrote -->

**Reading the pull request's run:** Linux tier 1 is over its budget (23.2 s of 20 s; the fail line is 30 s) in this run, and was not in the run before (20.7 s against `main`'s 19.5 s). The time moved in every crate at once, by about the same share. The tests are the same tests under new names, so that is the runner, and the A/B on one runner above is the measure: +0.8 s. On its first try, `test-windows` ran out of its five minutes while uploading its report: every test had passed, on a slow runner (8 ms a process start), with the workspace's crates built from cold (2 min 12 s). It passed when run again: 240 s.
