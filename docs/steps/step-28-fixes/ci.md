# After step-28-fixes: its CI times

## The first pull request

<!-- written by cargo xtask ci-times, section "The first pull request": it replaces this up to the end mark -->
Pull request 136, on `main` at `51f6eeb`. "Main" is main's own run at `51f6eeb` (run 45955638461).
The PR's run is 45958057261, at `e68c131`.

| Job | Main, `51f6eeb` | The PR at `e68c131` |
|---|---|---|
| `lint-linux` | 107 s | 91 s |
| `test-linux` | 166 s | 149 s |
| `lint-windows` | 117 s | 102 s |
| `build-windows` | 226 s | 184 s |
| `test-windows-tiers` | 105 s | 87 s |
| `test-windows-system` | 87 s | 98 s |
| `pr-size-linux` (blocks nothing) | — | 234 s |
| `ts-client` (blocks nothing) | 73 s | 7 s |
| `pr-size-windows` (blocks nothing) | — | 298 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `e68c131` (4 ms) | Windows, main (8 ms; tier 3 6 ms) | Windows, `e68c131` (6 ms; tier 3 8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 16.1 s | 13.8 s | 26.9 s | 21.1 s |
| Tier 2, integration (40 s) | 45.9 s | 31.6 s | 59.3 s | 45.8 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 42.7 s | 38.7 s | 84.0 s | 92.8 s |

<!-- end of what cargo xtask ci-times wrote, section "The first pull request" -->

## The second pull request

<!-- written by cargo xtask ci-times, section "The second pull request": it replaces this up to the end mark -->
Pull request 137, on `main` at `51f6eeb`. "Main" is main's own run at `51f6eeb` (run 45955638461).
The PR's run is 45975413121, at `2108f11`.

| Job | Main, `51f6eeb` | The PR at `2108f11` |
|---|---|---|
| `lint-linux` | 107 s | 103 s |
| `test-linux` | 166 s | 130 s |
| `lint-windows` | 117 s | 114 s |
| `build-windows` | 226 s | 106 s |
| `test-windows-tiers` | 105 s | 91 s |
| `test-windows-system` | 87 s | 72 s |
| `pr-size-linux` (blocks nothing) | — | 273 s |
| `ts-client` (blocks nothing) | 73 s | 73 s |
| `pr-size-windows` (blocks nothing) | — | 303 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `2108f11` (3 ms) | Windows, main (8 ms; tier 3 6 ms) | Windows, `2108f11` (6 ms; tier 3 5 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 16.1 s | 13.3 s | 26.9 s | 21.9 s |
| Tier 2, integration (40 s) | 45.9 s | 26.7 s | 59.3 s | 47.5 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 42.7 s | 36.2 s | 84.0 s | 72.2 s |

<!-- end of what cargo xtask ci-times wrote, section "The second pull request" -->
