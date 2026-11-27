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

The job A/B of the third-party crates (`try` 45979975138, two rounds): a cold build 186.0 → 214.7 s, tier 2 52.4 → 51.6 s.

## The layout

`main` at `0ce319cc` built and run in one place against the same tests built into a nextest archive, extracted and run from it, six rounds on one runner, the first two cold (`try` 45974262733, `ab --job --from-archive head --keep-build`): tier 2 48.2 → 44.9 s cold and 45.3 → 43.8 s warm, tier 1 22.8 → 21.1 and 20.1 → 19.3 s, tier 3 87.0 → 80.3 and 73.7 → 73.4 s, the extraction 1.0 s. The spread of one side's own rounds in tier 2 is 5.4 s.
