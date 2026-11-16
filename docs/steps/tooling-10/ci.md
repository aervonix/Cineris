# After the tenth tooling step: records that do not conflict, known flakes, and tools for runs and branches

The CI times of each pull request of the step, each against `main` of its moment, as `cargo xtask ci-times` writes them.

## The first pull request: the open items' index

<!-- written by cargo xtask ci-times, section "first-pull-request": it replaces this up to the end mark -->
Pull request 82, on `main` at `f3cb594`. "Main" is main's own run at `f3cb594` (run 43931005537).
The PR's run is 43931346290, at `546ed27`.

| Job | Main, `f3cb594` | The PR at `546ed27` |
|---|---|---|
| `lint-linux` | 74 s | 67 s |
| `test-linux` | 138 s | 111 s |
| `lint-windows` | 107 s | 95 s |
| `test-windows` | 282 s | 171 s |
| `pr-size-linux` (blocks nothing) | — | 178 s |
| `pr-size-windows` (blocks nothing) | — | 323 s |

| Time above process start, against the budget | Linux, main (6 ms) | Linux, `546ed27` (4 ms) | Windows, main (9 ms) | Windows, `546ed27` (5 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 14.3 s | 12.6 s | 19.6 s | 9.3 s |
| Tier 2, integration (40 s) | 27.7 s | 24.1 s | 43.3 s | 21.3 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 26.3 s | 24.6 s | 87.6 s | 50.4 s |

<!-- end of what cargo xtask ci-times wrote, section "first-pull-request" -->

## The second pull request: known flakes

<!-- written by cargo xtask ci-times, section "second-pull-request": it replaces this up to the end mark -->
Pull request 83, on `main` at `81321cf`. "Main" is main's own run at `81321cf` (run 43932885743).
The PR's run is 43934275471, at `b6bc6af`.

| Job | Main, `81321cf` | The PR at `b6bc6af` |
|---|---|---|
| `lint-linux` | 67 s | 68 s |
| `test-linux` | 130 s | 97 s |
| `lint-windows` | 79 s | 93 s |
| `test-windows` | 259 s | 275 s |
| `pr-size-linux` (blocks nothing) | — | 189 s |
| `pr-size-windows` (blocks nothing) | — | 277 s |

| Time above process start, against the budget | Linux, main (5 ms) | Linux, `b6bc6af` (3 ms) | Windows, main (9 ms) | Windows, `b6bc6af` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 16.9 s | 11.4 s | 18.4 s | 20.0 s |
| Tier 2, integration (40 s) | 29.2 s | 31.3 s | 42.6 s | 44.4 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 25.2 s | 20.2 s | 78.4 s | 81.3 s |

<!-- end of what cargo xtask ci-times wrote, section "second-pull-request" -->
