# After step 19: verification

The CI times of step 19's pull requests, and what their runs showed.

## first-pull-request

<!-- written by cargo xtask ci-times, section "first-pull-request": it replaces this up to the end mark -->
Pull request 77, on `main` at `ea6081e`. "Main" is main's own run at `ea6081e` (run 43897405347).
The PR's run is 43902655704, at `7e7b218`.

| Job | Main, `ea6081e` | The PR at `7e7b218` |
|---|---|---|
| `lint-linux` | 57 s | 46 s |
| `test-linux` | 102 s | 127 s |
| `lint-windows` | 91 s | 93 s |
| `test-windows` | 252 s | 162 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | — | — |
| `pr-size-linux` (blocks nothing) | — | 224 s |
| `pr-size-windows` (blocks nothing) | — | 232 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `7e7b218` (6 ms) | Windows, main (9 ms) | Windows, `7e7b218` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 15.1 s | 13.4 s | 24.5 s | 8.4 s |
| Tier 2, integration (40 s) | 22.1 s | 25.0 s | 44.1 s | 21.2 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 23.3 s | 26.0 s | 74.7 s | 53.1 s |

<!-- end of what cargo xtask ci-times wrote, section "first-pull-request" -->

## second-pull-request

<!-- written by cargo xtask ci-times, section "second-pull-request": it replaces this up to the end mark -->
Pull request 79, on `main` at `40a6658`. "Main" is main's own run at `40a6658` (run 43914134969).
The PR's run is 43914726509, at `d6d77ca`.

| Job | Main, `40a6658` | The PR at `d6d77ca` |
|---|---|---|
| `lint-linux` | 59 s | 69 s |
| `test-linux` | 107 s | 105 s |
| `lint-windows` | 86 s | 82 s |
| `test-windows` | 202 s | 262 s |
| `pr-size-linux` (blocks nothing) | — | 193 s |
| `pr-size-windows` (blocks nothing) | — | 276 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `d6d77ca` (3 ms) | Windows, main (6 ms) | Windows, `d6d77ca` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 9.4 s | 9.5 s | 14.3 s | 25.5 s |
| Tier 2, integration (40 s) | 16.1 s | 15.5 s | 29.4 s | 47.6 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 22.7 s | 22.4 s | 61.1 s | 83.3 s |

<!-- end of what cargo xtask ci-times wrote, section "second-pull-request" -->
