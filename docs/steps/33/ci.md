# After step 33: the size fix, an early part of the performance pass

The size fix changes how the shipped binary is built, not the tests: `release` at `opt-level = "s"` touches no test job, which builds in the `ci` profile. What it adds to a pull request's run is the `pr-size` job ([testing-and-ci.md](../../testing-and-ci.md), "Each pull request's binary size"), which starts after the four required jobs and blocks nothing; the tables below give its time beside theirs. Its sizes and speed figures are in [measurements/binary-size.md](../../measurements/binary-size.md).

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 63, on `main` at `b08a745`. "Main" is main's own run at `b08a745` (run 43819869096).
The PR's run is 43822660544, at `bb86234`.

| Job | Main, `b08a745` | The PR at `bb86234` |
|---|---|---|
| `lint-linux` | 59 s | 55 s |
| `pr-size-linux` | — | 196 s |
| `test-linux` | 97 s | 102 s |
| `lint-windows` | 91 s | 93 s |
| `pr-size-windows` | — | 269 s |
| `test-windows` | 155 s | 188 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `bb86234` (3 ms) | Windows, main (6 ms) | Windows, `bb86234` (7 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 8.2 s | 8.1 s | 8.6 s | 11.1 s |
| Tier 2, integration (40 s) | 13.1 s | 12.8 s | 20.2 s | 23.9 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 16.0 s | 16.6 s | 48.4 s | 58.3 s |

<!-- end of what cargo xtask ci-times wrote -->

**The size job's cost, measured on this run:** `pr-size-windows` 4 min 29 s (its `dist` build 3 min 19 s) and `pr-size-linux` 3 min 16 s (2 min 19 s), 7 min 45 s of runner time for a pull request that can change the binary, and seconds for one that cannot. Each started 2 s after the last required job (`test-windows`) ended, so none of them waited for it. It found the binaries at 13,828,096 bytes on Windows and 11,875,688 on Linux, 5,398,016 and 4,382,720 under `main`'s last record (run 43817284507, at `b08a745`), with no home path in either.

**The tiers.** Nothing here changes a test's code path but the new tests; the `ci` profile is untouched. Windows' tiers are 2 to 10 s above `main`'s on a runner with a start cost of 7 ms against 6 ms, a slower runner by the measure the tier report gives, and each is far inside its budget.
