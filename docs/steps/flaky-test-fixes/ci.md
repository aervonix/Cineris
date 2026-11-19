# After the flaky-test fixes: an entity's escape cut mid-row, and a check's shell in tier 2

The fixes add four markdown unit tests and a process-runner unit test, and move one core test's shell from Windows' tier 2 to a Unix system test.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 104, on `main` at `f6fedc3`. "Main" is main's own run at `f6fedc3` (run 44440506912).
The PR's run is 44449609218, at `059bd1c`.

| Job | Main, `f6fedc3` | The PR at `059bd1c` |
|---|---|---|
| `lint-linux` | 61 s | 83 s |
| `test-linux` | 134 s | 147 s |
| `lint-windows` | 100 s | 99 s |
| `test-windows` | 237 s | 180 s |
| `pr-size-linux` (blocks nothing) | — | 214 s |
| `ts-client` (blocks nothing) | 0 s | 13 s |
| `pr-size-windows` (blocks nothing) | — | 372 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `059bd1c` (5 ms) | Windows, main (6 ms) | Windows, `059bd1c` (5 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 20.9 s | 28.0 s | 14.5 s | 11.0 s |
| Tier 2, integration (40 s) | 46.4 s | 37.2 s | 33.1 s | 22.9 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 37.3 s | 32.9 s | 71.6 s | 60.7 s |

<!-- end of what cargo xtask ci-times wrote -->
