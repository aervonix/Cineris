# After step 22: transcript rendering

Step 22 built the transcript's rendering in `cortex-tui`: the theme, the glyph sets, text layout and the rounded box (pull request 45, the foundation step 21 built on), then streaming markdown with its commit rule on a parser of our own, highlighting with syntect on a worker thread of its own, the views of every element, the transcript, and the golden screens (pull request 48). It adds one third-party package, `syntect` (behind the default feature `highlight`), and in CI's build optimises it and the crates it loads its grammars with, as `serde_json` and `sha2` already are.

## On the pull request

Pull request 48, on `main` at `8c818f3`. "Main" is main's own run at `8c818f3`. Hosted runners differ in speed by up to a factor of two, told by the cost of starting a test's process that the tier report prints: the two runs below were not on runners of the same speed, so the A/B runs further down are the comparison.

| Job | Main, `8c818f3` | The PR at `99f3307` |
|---|---|---|
| `lint-linux` | 56 s | 45 s |
| `test-linux` | 83 s | 106 s |
| `lint-windows` | 72 s | 59 s |
| `test-windows` | 197 s | 219 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `99f3307` (4 ms) | Windows, main (11 ms) | Windows, `99f3307` (10 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 3.5 s | 11.7 s | 12.2 s | 18.5 s |
| Tier 2, integration (40 s) | 8.8 s | 19.0 s | 38.5 s | 39.2 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 10.9 s | 15.5 s | 92.1 s | 91.6 s |

**What the pull request's first runs found.** The first run (at `95d568d`) put the Windows unit tier at 17.7 s of 20 s, and the third (at `18b9de2`) at 23.0 s, over its budget; the Windows A/B of that tree put `cortex-tui` at +8.7 s in tier 1. The markdown tests ran three to four times as slowly on the hosted runners as in CI's build on the desktop, and the highlighter's tests each loaded syntect's grammars unoptimised. Three changes followed, each keeping every break caught:

- syntect and the crates it loads with are optimised in CI's build (a highlighter test went from 0.09 s to 0.02 s on the desktop);
- on a pull request the random-document property takes 25 cases, and the tests that split a text at many places a sample of the places; the nightly run, whose case scale is over one, takes them all;
- the tests of the bounds on the text kept give the stream smaller bounds than the product's (`Markdown::with_bounds`, for tests only), so that they pass them on a few kilobytes of text.

`cortex-tui`'s unit tier in CI's build on the desktop went from 5.7 s to 2.1 s, its slowest test from 0.6 s to 0.13 s.

## Windows, both trees on one runner

`try` with `-f against=main` (run 43637453134), at `99f3307` against `8c818f3`, the median of five runs each, on one runner at 8.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 11.3 s (1,525 tests) | 8.8 s (1,455) | +2.5 s | 20 s |
| 2, integration | 26.7 s (1,031) | 26.6 s (1,027) | +0.1 s | 40 s |
| 3, system | 66.8 s (132) | 65.9 s (132) | +0.9 s | 90 s |

By crate: `cortex-tui` +3.0 s in tier 1 and +0.5 s in tier 2 (the golden screens); `cortex-term` −0.1 s. The slowest new tests on that runner: the long word's (0.33 s), the random-document property (0.30 s), deep nesting and many marks (0.30 s), nothing finished parsed again (0.27 s), the chained resizes (0.23 s).

## Linux, both trees on one runner

`try` with `-f os=linux -f against=main` (run 43638480070), at `99f3307` against `8c818f3`, the median of five runs each, at 3.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 5.5 s (1,542 tests) | 3.7 s (1,472) | +1.7 s | 20 s |
| 2, integration | 10.3 s (1,034) | 10.0 s (1,030) | +0.3 s | 40 s |
| 3, system | 8.6 s (181) | 8.5 s (181) | +0.1 s | 60 s |

By crate: `cortex-tui` +1.8 s in tier 1 and +0.3 s in tier 2; `cortex-term` nothing. The pull request's own Linux run above was on a slower runner than main's, which is why its tiers there are higher.

## Other runs

- **Windows tests** (`try`, run 43634619626, at `95d568d`): `test -p cortex-tui` passed, 201 tests.
- **Stress on the desktop:** `cargo xtask stress -p cortex-tui --count 50` at `99f3307`, 201 tests, every one passed every time. An earlier run found the colours test failing 4 times in 50: a quick worker could have the line's colours before the commit looked, so the test holds the worker until it has looked.
- **Breaks:** `cargo xtask mutate step-22 --jobs 4`, 46 breaks, every one caught by the tests it names.
- **Frame cost and size:** in [performance-budgets.md](../../performance-budgets.md), the frame-cost and binary-size rows.
