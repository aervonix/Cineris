# After the ninth tooling step: room in Windows' tiers, the process-start estimate, the rigs' helpers and the A/B that survives a one-sided failure

This step went in as several pull requests. The tables were made with `cargo xtask ci-times`; each pull request's A/B against `main` is beside its run.

## The tiers' pull request

<!-- written by cargo xtask ci-times, section "The tiers' pull request": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 43891258419), the median of 5 runs each, on one runner at 9.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 18.5 s (2,363 tests) | 17.9 s (2,363) | +0.6 s | 20 s |
| 2, integration | 40.2 s (1,375 tests) | 39.3 s (1,375) | +1.0 s | 40 s |
| 3, system | 72.1 s (174 tests) | 85.2 s (174) | −13.1 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "The tiers' pull request" -->
