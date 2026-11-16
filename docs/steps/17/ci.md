# After step 17: the project profile and memory

Step 17 added `cortex-knowledge` (entries, the private store and the repository's shared profile, the text of section 5), two additions to `cortex-policy` (the exact rule of an accepted command, and a running session's learned rules), the epoch's `profile` in the log, and section 5 in the prompt's builder. Its CI times, and the measurements taken on the hosted runners, are below, one part for each of its pull requests.

## The first pull request: the store

On the desktop, built as CI builds (`cargo xtask ci`, 2026-11-16, nothing else running):

| Tier | Tests | Above the tests' start | Budget | Used |
|---|---:|---:|---:|---:|
| 1, unit | 2,136 | 9.0 s | 20 s | 45% |
| 2, integration | 1,323 | 14.3 s | 40 s | 36% |
| 3, system | 206 | 9.2 s | 60 s | 15% |

`cortex-knowledge`'s own 80 tests take 0.06 s of it; `cortex-policy`'s new property test, at 40 cases, 0.08 s.

**The breaks:** `mutations/step-17.breaks`, 59 breaks, every one caught by the tests its list names (`cargo xtask mutate step-17 --jobs 2`, on the desktop). **Stress:** `cortex-knowledge`, `cortex-policy` and `cortex-protocol`, 298 tests, 100 times each under seeds 0 to 99, every test passing every time; before the review's fixes, the six crates whose tests changed, 943 tests, 50 times each.

## The first pull request, on the hosted runners

<!-- written by cargo xtask ci-times, section "store": it replaces this up to the end mark -->
Pull request 66, on `main` at `5431388`. "Main" is main's own run at `5431388` (run 43853266285).
The PR's run is 43853462567, at `e450d36`.

| Job | Main, `5431388` | The PR at `e450d36` |
|---|---|---|
| `lint-linux` | 60 s | 62 s |
| `test-linux` | 117 s | 118 s |
| `lint-windows` | 76 s | 80 s |
| `test-windows` | 237 s | 239 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | — | — |
| `pr-size-linux` (blocks nothing) | — | 181 s |
| `pr-size-windows` (blocks nothing) | — | 226 s |

| Time above process start, against the budget | Linux, main (6 ms) | Linux, `e450d36` (6 ms) | Windows, main (9 ms) | Windows, `e450d36` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 13.7 s | 13.0 s | 18.3 s | 18.1 s |
| Tier 2, integration (40 s) | 23.0 s | 25.6 s | 36.0 s | 38.5 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 20.6 s | 20.2 s | 73.7 s | 71.3 s |

<!-- end of what cargo xtask ci-times wrote, section "store" -->

### Its measurements on the runners

`try` run 43851211647 (`perf -p cortex-knowledge`, a release build, medians), beside the desktop's and the budgets proposed for `docs/performance-budgets.md`:

| What | Proposed | Desktop | Linux runner | Windows runner |
|---|---|---|---|---|
| Load both files, typical (50 entries, 18 KB) | 1 ms | 0.20 ms | 0.33 ms | 0.55 ms |
| Load both files, at the caps (700 entries, 247 KB) | 5 ms on the desktop | 2.65 ms | 5.06 ms | 4.67 ms (95th 5.9 ms) |
| The look for a change | 50 µs | 0.9 µs | 1.3 µs | 18 µs |
| Render section 5, typical | 100 µs | 24 µs | 32 µs | 27 µs |
| Render section 5, at the caps | 1 ms | 0.30 ms | 0.40 ms | 0.37 ms |
| One change (lock, read, flushed write, good copy) | none | 15 to 20 ms | 1.1 ms, 11 ms at the caps | 1.6 ms, 11 ms at the caps |

The load at the caps is over 5 ms on the hosted runners. The recommendation, which the coordinator has put to the owner: the row takes columns as the start-up rows have, 5 ms on the desktop and 15 ms on a runner, and the caps stay. `pr-size`: Linux 11,879,784 bytes (11.33 MB of 16), Windows 13,830,656 bytes (13.19 MB of 18, 2,560 more than `main`).

## The second pull request: the wiring

On the desktop, built as CI builds (`cargo xtask ci`, 2026-11-16, with other sessions' work beside it, so the totals are not comparable with the first pull request's): `cortex-knowledge` 0.1 s of unit tests and 1.0 s of integration tests above the tests' start, `cortex-core` 1.4 s, 6.3 s and 0.9 s of system tests, `cortex-agent` 0.2 s and 3.5 s. The whole run: tier 1 at 19.1 s of 20 (`xtask`'s own tests are 6.9 s of it), tier 2 at 23.8 s of 40, tier 3 at 11.3 s of 60.

**What the observer adds to a `shell` call,** on the call's own path (`perf_observing_a_command`, a release build on the desktop, medians of 20,000): a line that is not a build, test or check 0.11 µs; a line the observer does not take 0.04 µs; a command the store knows and confirmed today 0.34 µs (0.59 µs with the store at its caps); a command the store does not know 0.30 µs (0.90 µs at the caps). Storing what was learned is beside the turn.

**The breaks:** `mutations/step-17.breaks`, 108 breaks, 49 of them this pull request's, every one caught by the tests its list names (`cargo xtask mutate step-17 --jobs 2`, on the desktop, on the pull request's last commit). **Stress:** `cortex-knowledge`, `cortex-core`, `cortex-agent`, `cortex-config` and `cortex-policy`, 1,031 tests, 50 times each under seeds 0 to 49, every test passing every time. Two earlier runs each failed once: `trust::sys_trust_for_the_run_reaches_all_four`, which is [a known item](../../outstanding/a-base-capture-beside-the-turns-git.md) and not this step's; and this step's own test of an expired entry, which read a notice it had not waited for (the last review found it in the same hour, and it waits now).

**Windows.** `try` run 43862916628 ran the five crates' tests on a hosted Windows runner before the pull request was opened: nothing failed, the two tests whose programs are `.cmd` files included.

## The second pull request before its tests were trimmed

<!-- written by cargo xtask ci-times, section "wiring-ab": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 43863357445), the median of 5 runs each, on one runner at 6.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 8.8 s (2,254 tests) | 8.6 s (2,241) | +0.3 s | 20 s |
| 2, integration | 21.2 s (1,334 tests) | 20.6 s (1,312) | +0.6 s | 40 s |
| 3, system | 58.2 s (165 tests) | 46.5 s (152) | +11.7 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "wiring-ab" -->

## The second pull request as it was opened

<!-- written by cargo xtask ci-times, section "wiring-trimmed": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 43869925378), the median of 5 runs each, on one runner at 5.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 8.9 s (2,254 tests) | 9.0 s (2,241) | +0.0 s | 20 s |
| 2, integration | 19.7 s (1,335 tests) | 19.0 s (1,312) | +0.7 s | 40 s |
| 3, system | 47.5 s (164 tests) | 42.0 s (152) | +5.5 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "wiring-trimmed" -->

## The second pull request, on the hosted runners

<!-- written by cargo xtask ci-times, section "wiring-pr": it replaces this up to the end mark -->
Pull request 71, on `main` at `3c5ee17`. "Main" is main's own run at `3c5ee17` (run 43871129198).
The PR's run is 43871451116, at `c64ddb3`.

| Job | Main, `3c5ee17` | The PR at `c64ddb3` |
|---|---|---|
| `lint-linux` | 65 s | 67 s |
| `test-linux` | 123 s | 121 s |
| `lint-windows` | 90 s | 66 s |
| `test-windows` | 242 s | 245 s |
| `pr-size-${{ matrix.name }}` (blocks nothing) | 0 s | — |
| `pr-size-linux` (blocks nothing) | — | 209 s |
| `pr-size-windows` (blocks nothing) | — | 350 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `c64ddb3` (5 ms) | Windows, main (9 ms) | Windows, `c64ddb3` (9 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 20.5 s | 15.4 s | 19.9 s | 18.1 s |
| Tier 2, integration (40 s) | 46.0 s | 27.5 s | 37.5 s | 38.3 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 44.0 s | 23.3 s | 82.0 s | 83.0 s |

<!-- end of what cargo xtask ci-times wrote, section "wiring-pr" -->

`pr-size` on the second pull request: Linux 12,309,928 bytes (11.74 MB of 16), 417,856 more than `main`; Windows 14,391,296 bytes (13.72 MB of 18), 542,720 more. It is the first of the three that links `cortex-knowledge` into the binary.

Two things these tables show that are not this step's: `main`'s own Linux run at `3c5ee17` was on a slow runner (tier 2 at 46 s of 40), and Windows' tier 3 is at 82 s of 90 on `main` itself on a 9 ms runner. The second pull request adds 5.5 s to it in the A/B above.

## Where the second pull request's bytes went

`try`'s `size --breakdown` on `main` after the second pull request (`fa8b464`, run 43874231293) and on the commit before it (`3c5ee17`, run 43874265576), the Linux binary (`x86_64-unknown-linux-musl`, the `dist` profile), by the crate that owns each symbol:

| Crate | Before | After | Change |
|---|---:|---:|---:|
| The whole binary | 11,892,072 | 12,309,928 | +417,856 |
| `cortex_knowledge` | 0 | 112,661 | +112,661 |
| `toml` (`cortex-knowledge` turns on its `display` feature, which writes TOML) | 200,118 | 289,128 | +89,010 |
| In no symbol: headers, unwind tables, padding | 3,232,521 | 3,303,994 | +71,473 |
| `cortex_core` (the wiring) | 411,859 | 473,484 | +61,625 |
| `std` (generic code made for the new types) | 2,007,965 | 2,060,641 | +52,676 |
| `tokio` | 355,355 | 371,029 | +15,674 |
| `cortex_agent` | 239,605 | 243,323 | +3,718 |
| `cortex_protocol` | 112,586 | 116,115 | +3,529 |

For step 33: a quarter of it is the crate and a fifth is `toml`'s writer, which a store that wrote its own file would not need, if the size is wanted back.

## The third pull request: the commands

<!-- written by cargo xtask ci-times, section "commands-ab": it replaces this up to the end mark -->
### Windows, both trees on one runner

`try` with `-f against=main` (run 43876287467), the median of 5 runs each, on one runner at 5.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 9.1 s (2,309 tests) | 9.3 s (2,299) | −0.2 s | 20 s |
| 2, integration | 19.7 s (1,337 tests) | 19.5 s (1,335) | +0.2 s | 40 s |
| 3, system | 48.4 s (166 tests) | 49.5 s (164) | −1.1 s | 90 s |

<!-- end of what cargo xtask ci-times wrote, section "commands-ab" -->

The A/B above was taken before the branch was rebased onto the re-decide step's change (`ffe45b6`); the new system test starts the binary twelve times and no shell.

**The breaks:** `mutations/step-17.breaks` has 123, 15 of them the third pull request's. On its last commit, the 58 that the change touches (`cargo xtask mutate step-17 --changed origin/main`: 18 new or changed in the list, 29 by their file, 11 by a test they name) each did what the list expects; the other 65 were run on the second pull request's last commit. **Stress:** `cortex-cli`, `cortex-core` and `cortex-knowledge`, 453 tests, 50 times each under seeds 0 to 49, every test passing every time.

