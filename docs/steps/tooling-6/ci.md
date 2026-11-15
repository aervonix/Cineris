# After the sixth tooling step: Windows' system tier, stalls, stale break lists, dependents, the test kit, runs and logs, live tests by name

This step added tools and test support: the stall check, `test --dependents`, `mutate --check` of `caught-by` names and `--fix-in`, `ab --keep-going`, `ci-log`, the `perf` preset of `try`, `live --test`, the last good copy of `xtask` and `regen`, `ScriptedProvider`'s output limit, `Rig::fill_next` and `fill_after`, `Phases`, and a made-up month of statistics. It changed system tests of `cortex-tools` and `xtask` to start PowerShell and git less, and, by the owner's decision, runs Windows' system tests two at a time. No dependency was added. The tables were made with `cargo xtask ci-times`.

## On the pull request

(Written when the pull request's CI has run.)

## Windows, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 43718003767), the median of 5 runs each, on one runner at 6.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 9.8 s (1,721 tests) | 8.5 s (1,620) | +1.2 s | 20 s |
| 2, integration | 20.6 s (1,170 tests) | 20.1 s (1,161) | +0.5 s | 40 s |
| 3, system | 39.2 s (133 tests) | 53.5 s (133) | −14.3 s | 90 s |

<!-- end of what cargo xtask ci-times wrote -->

## Linux, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 43718003767), the median of 5 runs each, on one runner at 4.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 5.3 s (1,740 tests) | 5.1 s (1,639) | +0.3 s | 20 s |
| 2, integration | 11.0 s (1,176 tests) | 11.0 s (1,167) | +0.0 s | 40 s |
| 3, system | 8.7 s (186 tests) | 8.4 s (186) | +0.3 s | 60 s |

<!-- end of what cargo xtask ci-times wrote -->
