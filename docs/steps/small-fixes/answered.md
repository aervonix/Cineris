# Answered in the small fixes, on 2026-11-20

A small fix outside the numbered steps, given by the maintainers: a backlog of test-quality debts (breaks that no test caught or that tested nothing any more, two known flakes, a timeout seen only on the reference machine), and one budget the owner approved.

## Windows' unit tier at 25 s

The owner's decision of 2026-11-20: Windows' unit tier is held to 25 s above the start of its tests' processes, Linux's stays at 20 s, as the system tier is 90 s on Windows and 60 s on Linux. A warning past 25 s and a failure past 37.5 s on Windows. The measurement behind it is in [../ci-budgets/answered.md](../ci-budgets/answered.md), "Left for the owner": on an 8 ms runner the tier read 20.2 to 21.0 s, of which 8.8 s since wave 12 was 680 new tests. `cargo xtask ci --tiers` judges each system's run against its own budget (`Tier::budget` in `xtask/src/tiers.rs`). No other budget changed.
