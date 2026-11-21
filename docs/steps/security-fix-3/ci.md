# After security-fix-3: a confined start's protected change put back, and the harness's git confined at every level

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 125, on `main` at `b365d30`. "Main" is main's own run at `b365d30` (run 44921084018).
The PR's run is 44927858850, at `bd6e619`.

| Job | Main, `b365d30` | The PR at `bd6e619` |
|---|---|---|
| `lint-linux` | 75 s | 63 s |
| `test-linux` | 155 s | 144 s |
| `lint-windows` | 106 s | 95 s |
| `test-windows` | 315 s | 225 s |
| `pr-size-linux` (blocks nothing) | — | — |
| `ts-client` (blocks nothing) | — | 6 s |
| `pr-size-windows` (blocks nothing) | — | — |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `bd6e619` (4 ms) | Windows, main (8 ms) | Windows, `bd6e619` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 14.8 s | 12.3 s | 24.7 s | 17.2 s |
| Tier 2, integration (40 s) | 34.6 s | 26.3 s | 55.1 s | 38.2 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 47.6 s | 46.6 s | 93.9 s | 73.7 s |

<!-- end of what cargo xtask ci-times wrote -->
