# After step 21: the editor

Step 21 built the editor in `cineris-tui`: the text model, the key map and its actions, history, paste, the completion menus and pickers with a fuzzy matcher of our own, the focus ring, the external-editor hand-off, and the input box's and menus' views on step 22's foundation. In `cineris-term` it added `ignore_interrupts` and settled two corners of the input decoder. It adds no third-party package; `cineris-tui` gains the test kit as a dev-dependency. The editor demo (`examples/editor-demo`, behind a `demo` feature) and `cargo xtask perf --only editor` measure key latency with the editor in the loop.

## On the pull request

Pull request 47, on `main` at `e9d938c`. "Main" is main's own run at `e9d938c`. Hosted runners differ in speed by up to a factor of two, told by the cost of starting a test's process that the tier report prints: these two runs were not on runners of the same speed, so the A/B below is the comparison.

| Job | Main, `e9d938c` | The PR at `9bf8de2` |
|---|---|---|
| `lint-linux` | 50 s | 52 s |
| `test-linux` | 71 s | 91 s |
| `lint-windows` | 57 s | 76 s |
| `test-windows` | 192 s | 125 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `9bf8de2` (5 ms) | Windows, main (11 ms) | Windows, `9bf8de2` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 4.0 s | 7.1 s | 12.4 s | 8.4 s |
| Tier 2, integration (40 s) | 10.9 s | 17.6 s | 39.3 s | 23.2 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 10.5 s | 16.7 s | 94.2 s | 57.2 s |

**By crate, against main, from the same run** (`cargo xtask ci --tiers --compare 43625317248 --against main`), Linux: `cineris-tui` +1.4 s in tier 1, +1.7 s in tier 2 and +0.4 s in tier 3; `cineris-term` +0.2 s, +0.3 s and nothing. The two slowest new tests were the every-width check of the editor's rows (1.5 s) and of the input box's cursor (1.0 s); after this run they check a sample on a pull request and everything in the nightly run (under 0.1 s each in CI's build on the desktop).

## Windows, both trees on one runner

`try` with `-f against=main -f count=2` (run 43625579839), at `9bf8de2` against `e9d938c`, the median of two runs each, on one runner at 8.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 9.4 s (1,455 tests) | 8.9 s (1,397) | +0.5 s | 20 s |
| 2, integration | 28.6 s (1,027) | 27.1 s (987) | +1.5 s | 40 s |
| 3, system | 65.4 s (132) | 71.0 s (122) | −5.6 s | 90 s |

By crate: `cineris-tui` +1.2 s, +1.6 s and +1.0 s (the ten new system tests of the external editor, each a shell, the application and an editor in ConPTY); `cineris-term` −0.1 s in each. The tier 3 total fell because other crates' tests ran faster in this run (`cineris-tools` −3.7 s, `xtask` −1.5 s), which is the runner, not this step. The A/B ran before the two slow tests above were cut to a sample.

## Other runs

- **Windows tests** (`try`, run 43624926353): `test -p cineris-tui -p cineris-term` passed, the external editor's system tests under ConPTY among them, `Ctrl+C` reaching the editor and not the application included.
- **Windows key latency** (`try`, run 43625205958, `perf`): in [performance-budgets.md](../../performance-budgets.md), the key-latency rows.
- **Stress on the desktop:** `cargo xtask stress -p cineris-tui -p cineris-term --count 100 --cores 2 --load 6`, 206 tests, every one passed every time. An earlier loaded run found the external editor's `Ctrl+C` test failing 12 times in 50, and then 7, which led to the interrupt guard's final design ([answered.md](answered.md)).
- **Breaks:** `cargo xtask mutate step-21 --jobs 4`, 54 breaks, every one caught by the tests it names.
