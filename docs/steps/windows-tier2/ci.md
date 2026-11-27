# After the Windows tier 2 fix: the integration tier made cheaper, and the layout measured

The runs that measured each change, both trees on one Windows runner, `main` at `0ce319cc` as the base. What was built and decided is in [answered.md](answered.md).

## Each change on one runner

| Change | `try` run | Rounds | Start | Tier 1 | Tier 2 | Tier 3 |
|---|---|---|---|---|---|---|
| The TUI's memory tests in one session | 45979883356 | 5 | 8 ms | 28.0 → 26.8 s | **61.1 → 57.9 s** | 95.1 → 95.3 s |
| Nineteen third-party crates optimised | 45979971472 | 5 | 8 ms | 26.2 → 25.0 s | **59.2 → 56.0 s** | 93.9 → 93.0 s |
| Both | 45984356087 | 5 | 5 to 6 ms | 15.4 → 15.0 s | **37.6 → 34.2 s** | 64.0 → 64.4 s |

Measured and not taken:

| Change | `try` run | Rounds | Start | Tier 2 | Build |
|---|---|---|---|---|---|
| Our own crates at `opt-level = 1` | 45970482928 (tiers), 45970486829 (job) | 5, 2 | 8 ms | 53.7 → 39.4 s | cold 213 → 660 s |
| `cineris-tui` at `opt-level = 1` | 45975998314 (job) | 2 | 5 ms | 33.3 → 31.6 s | cold 119 → 150 s |
| `cineris-core` at `opt-level = 1` | 45976001836 (job) | 2 | 8 ms | 63.1 → 61.6 s | cold 212 → 284 s |
| Windows' tiers 1 and 2 one at a time | 45987739680 | 5 | 7 ms | 60.3 → 53.6 s | none; the runs' wall time 116.8 → 167.9 s |

The job A/B of the third-party crates (`try` 45979975138, two rounds): a cold build 186.0 → 214.7 s, tier 2 52.4 → 51.6 s.

## The layout

`main` at `0ce319cc` built and run in one place against the same tests built into a nextest archive, extracted and run from it, six rounds on one runner, the first two cold (`try` 45974262733, `ab --job --from-archive head --keep-build`): tier 2 48.2 → 44.9 s cold and 45.3 → 43.8 s warm, tier 1 22.8 → 21.1 and 20.1 → 19.3 s, tier 3 87.0 → 80.3 and 73.7 → 73.4 s, the extraction 1.0 s. The spread of one side's own rounds in tier 2 is 5.4 s.

## The final tree, both trees on one runner

`try --final` (run 45994804608): the branch at `4ee0d01` against `main` at `f844b31c` on one Windows runner at 8 ms a process start, the median of five rounds a side; the tests on Linux 22.04; the binary's size.

| Tier | The branch | `main` | Change | Budget |
|---|---|---|---|---|
| 1, unit | 22.3 s (3,648 tests) | 22.6 s (3,641) | −0.3 s | 25 s |
| 2, integration | **46.8 s** (1,859 tests) | 50.5 s (1,859) | **−3.8 s** | 40 s |
| 3, system | 86.5 s (270 tests) | 86.8 s (270) | −0.2 s | 90 s |

Tier 2 in each round, `main` then the branch: 50.9 / 47.2, 50.8 / 47.1, 50.6 / 46.9, 52.2 / 48.2, 51.5 / 47.7 s. By crate: `cineris-tui` 5.88 → 3.98 s, `cineris-core` 22.06 → 21.43 s, `cineris-knowledge` 1.60 → 1.10 s, `cineris-ext` 1.04 → 0.69 s, every other crate within 0.1 s.

On Linux 22.04 (5 ms): tier 1 13.3 s, tier 2 30.9 s, tier 3 40.6 s, each under its budget. The `dist` binary on Linux: 17,583,160 bytes, the same as `main`'s, with none of the test hooks.
