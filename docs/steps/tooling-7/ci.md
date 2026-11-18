# After the seventh tooling step: Windows' tiers 1 and 2 three at a time, A/B runs that keep their rounds, the test kit's names and points

This step added tools and test support: an A/B that leaves out a test failed on both sides, `try` on a commit, with `mutate` and with a passing test's output, `try-result`, `ci-times` detail and named sections, `api-diff`, a one-off break, `mutate --flagged` and `--merge`, a pinned corpus of real code, the shared hostile names and the library points. By the owner's decision it runs Windows' tiers 1 and 2 three at a time, and a session is put together with one path resolver where it made one for each missing folder. No dependency was added. The tables were made with `cargo xtask ci-times`.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 59, on `main` at `9baacb6`. "Main" is main's own run at `9baacb6` (run 43786487043).
The PR's run is 43790434698, at `87ee5ef`.

| Job | Main, `9baacb6` | The PR at `87ee5ef` |
|---|---|---|
| `lint-linux` | 52 s | 46 s |
| `test-linux` | 116 s | 109 s |
| `lint-windows` | 76 s | 87 s |
| `test-windows` | 226 s | 212 s |

| Time above process start, against the budget | Linux, main (5 ms) | Linux, `87ee5ef` (4 ms) | Windows, main (10 ms) | Windows, `87ee5ef` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 13.4 s | 12.3 s | 20.6 s | 12.2 s |
| Tier 2, integration (40 s) | 22.6 s | 21.1 s | 43.9 s | 25.5 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 16.2 s | 17.6 s | 67.6 s | 77.5 s |

<!-- end of what cargo xtask ci-times wrote -->

- **The first CI run of the pull request** (43788764531, at `252a4ce`) read Linux's integration tier at 44.9 s, over its budget, nearly all of it tests that write files in crates the step did not touch (`cineris-session`'s durability tests 0.4 → 5.4 s): the same three crates against `main` on one Linux runner read 4.1 s on both sides (`try` 43789085254), and this run, on another runner, 21.1 s. Windows read 17.6 s there, on a 5 ms runner.
- **Windows' system tier** reads 77.5 s here against `main`'s 67.6 s, on runners of 9 and 10 ms; on one runner (below) it moved by +0.8 s. Step 23's first part, on `main` since `9baacb6`, added the pseudo-terminal tests of `cineris-cli` to both sides.
- **The Windows test job** took 212 s against `main`'s 226 s: three at a time adds about 1.5 s to the run of tiers 1 and 2, and the tiers' tests are quicker.

## Windows, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 43787460318), the median of 3 runs each, on one runner at 9.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 15.8 s (2,028 tests) | 22.0 s (1,857) | −6.1 s | 20 s |
| 2, integration | 33.1 s (1,238 tests) | 44.9 s (1,235) | −11.8 s | 40 s |
| 3, system | 66.4 s (139 tests) | 65.7 s (139) | +0.8 s | 90 s |

<!-- end of what cargo xtask ci-times wrote -->
