# Measurements

What was built and measured to settle design questions, how, and the numbers. Taken on 2026-11-12, before any harness code existed.

| Page | Question it answers | Used by |
|---|---|---|
| [tui-language-spike.md](tui-language-spike.md) | Rust or TypeScript for the TUI? Which rendering approach? | Decisions [0001](../decisions/0001-tui-language.md), [0002](../decisions/0002-tui-rendering.md) |
| [dependency-costs.md](dependency-costs.md) | What does each candidate dependency cost in build time, binary size and start-up? Which build profile? | Decisions [0003](../decisions/0003-async-runtime.md), [0004](../decisions/0004-http-and-tls.md), [0005](../decisions/0005-extension-mechanism.md), [0006](../decisions/0006-session-storage.md), [0011](../decisions/0011-cli-parsing.md), [0012](../decisions/0012-release-profile.md), [0014](../decisions/0014-search-in-process.md), [0015](../decisions/0015-syntax-highlighting.md), [0018](../decisions/0018-credential-storage.md), [0019](../decisions/0019-logging.md); the [performance budgets](../performance-budgets.md); the CI time estimate in [testing-and-ci.md](../testing-and-ci.md) |
| [extension-mechanisms.md](extension-mechanisms.md) | WASM, subprocess or embedded scripting for extensions? Which language can a model write reliably? | Decision [0005](../decisions/0005-extension-mechanism.md) |
| [binary-size.md](binary-size.md) | What is the shipped binary made of, what does each way of shrinking it save and cost, what did step 33's size fix change, and what did the size step cut and project to step 34? Measured on 2026-11-16 and 2026-11-19, on the reference machine and the hosted runners. | Decision [0036](../decisions/0036-the-shipped-binary-is-optimised-for-size.md); the [performance budgets](../performance-budgets.md), "Binary size" |
| [windows-terminal-checklist.md](windows-terminal-checklist.md) | The manual check the owner runs in Windows Terminal, which the measurements could not cover. | Decision [0001](../decisions/0001-tui-language.md) |

## Machines

| | Reference machine | Second machine |
|---|---|---|
| CPU | Intel Core i7-14700K, 28 threads | Intel Core i5-1135G7, 8 threads |
| System | Linux 6.18 under WSL 2 | Linux 7.0, native |
| Toolchain | Rust 1.99.1 | Rust 1.99.1 |

The reference machine is a 28-thread desktop (Linux under WSL 2, which is why the TUI spike notes that the wall clock steps there); the second machine is an 8-thread laptop, native Linux. Heavy builds and the main measurement runs were done on the reference machine. Build-time measurements were pinned to 4 cores to approximate a GitHub-hosted runner. The second machine is the second data point for the TUI and extension measurements.

## What these numbers are, and are not

- **They are from throwaway spikes,** each a few hundred lines, kept outside this repo. The pages describe them in enough detail to rebuild.
- **They are ratios more than absolutes.** A real application costs more than a prototype in every column. The gaps between candidates come from runtimes and libraries and should carry over.
- **Nothing was measured on native Windows.** The Rust code was checked to compile for Windows. Windows build times, binary sizes and terminal behaviour are measured in the repo skeleton step (CI timings) and by the manual checklist.
- **Hosted-runner times are estimated** from 4 pinned cores of the reference machine, which are faster than a runner's. The repo skeleton step replaces the estimates with real CI timings.

## Re-measuring

The real harness measures itself with `cargo xtask perf`, which the terminal-layer step builds from the method in [tui-language-spike.md](tui-language-spike.md). The budgets and where each is checked are in [../performance-budgets.md](../performance-budgets.md).
