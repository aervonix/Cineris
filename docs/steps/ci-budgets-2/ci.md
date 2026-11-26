# After CI budgets II: a Windows session's open made cheaper, and the tests that run it

The CI times before and after, and the runs that measured them. What was found and changed is in [answered.md](answered.md).

## Before: `main`'s own CI at `42c90dcc`

Run 45721643017, the push to `main` of step 28's script hooks, on a Windows runner at 6 ms a process start:

| Job | Main, `42c90dc` |
|---|---|
| `lint-linux` | 88 s |
| `test-linux` | 148 s |
| `lint-windows` | 111 s |
| `test-windows` | 265 s |

| Time above process start, against the budget | Linux, main (4 ms) | Windows, main (6 ms) |
|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 11.5 s | 18.8 s |
| Tier 2, integration (40 s) | 26.0 s | **43.8 s** |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 39.4 s | 82.3 s |
