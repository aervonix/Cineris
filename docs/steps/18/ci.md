# After step 18: the repository map

Step 18 added `cortex-index` (the extractors, the cache, passes and the map), the map in an epoch in `cortex-agent`, the `outline` tool in `cortex-tools` and a session's index in `cortex-core`. Its CI times, and the measurements taken on the hosted runners, are below.

## The headless start-up with the index's pass beside it

The coordinator asked that a headless run's start be measured with the repository index's first pass running beside it, and that the pass start only after the first request if it pushed the run past its budget. `cargo xtask startup` gained `--workspace DIR` for it, which runs the headless rows in a real tree, with a breakdown row that turns the map (and with it the background pass) off. On the desktop, release build, 30 runs each, the tree this repository (1,492 files indexed; the first run indexes cold, the rest warm):

| Headless `cortex -p`, to its request | Median | 95th percentile | Budget |
|---|---:|---:|---:|
| In an empty workspace (the budget's own measurement) | 13.85 ms | 14.64 ms | 15 ms |
| In this repository, the map on | 15.32 ms | 22.24 ms | 15 ms |
| In this repository, `-c index.map=false` | 15.66 ms | 17.88 ms | |
| In this repository, `--no-session` | 4.06 ms | 4.56 ms | |

**The pass costs the run nothing that can be measured at the median:** with the map on and off the medians are within the runs' own noise. The 95th percentile is not: 22.24 ms with the map on against 17.88 ms off, which may be the pass's threads beside the run's start or the desk (the second review asked that this be said); a quiet-desk run with `--exclusive` would tell them apart (an earlier run of the same tree, before the breakdown row, gave 17.65 ms with the map on). So the pass stays where it is, started once the loop runs. What a real tree adds to a run, about 1.5 ms at the median with the map off as on, is older work: the session's base state, which outside a git repository lists and hashes the workspace on a thread of its own (the desktop's copy of the tree has no `.git`), and the larger instruction files of this repository. The start-up budget is measured in an empty workspace, as before.

## The warm map on hosted Windows runners

The ignored measurement test `perf::perf_a_cold_index_a_warm_one_and_a_warm_map`, in a release build, run by `try` (`-f os=windows -f profile=release -f command="test -p cortex-index --run-ignored only --no-capture --test-threads 1"`). A warm pass over a generated tree of 20,000 files in 221 directories with one file changed, then the map; the median of 15.

| Run | Code | Cold pass | Warm pass and map | Slowest | Map alone | Listing | Listing without ignore files | Cache read, written |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 43702243767 | before the walk beside the cache | 538 ms | 149.5 ms | 151.2 ms | 15.1 ms | | | |
| 43702621648 | the same, with the breakdown | 599 ms | 157.5 ms | 228.8 ms | 15.9 ms | 82.6 ms | 31.4 ms | 0.9 ms, 1.3 ms |
| 43703758856 | the cache walked beside the listing | | 182.8 ms | 250.5 ms | 22.7 ms | 99.1 ms | 35.9 ms | 1.9 ms, 1.9 ms |
| 43704772398 | the listing on the pass's threads | 546 ms | **112.6 ms** | 237.5 ms | 20.6 ms | 89.7 ms (61.7 ms on 2 threads) | 32.5 ms | 2.0 ms, 1.9 ms |
| 43704774461 | the same | 491 ms | **84.6 ms** | 154.5 ms | 13.9 ms | 76.6 ms (39.3 ms on 2 threads) | 27.7 ms | 0.9 ms, 1.2 ms |
| 43704989043 | the same | | **91.5 ms** | 148.5 ms | 14.5 ms | 86.2 ms (52.6 ms on 2 threads) | 31.0 ms | 1.0 ms, 1.3 ms |

The listing column is a walk on one thread with the pass's rules, timed by itself; from the fourth run on, beside it, the same walk on the threads a pass uses (half of a hosted runner's four cores). **With the listing on the pass's threads the warm map is 84.6 to 112.6 ms on hosted Windows runners, under its 150 ms budget on all three runs** (the coordinator's choice of the recommendation's first option); the slowest of a run's fifteen is the first after the cold pass, at 148.5 to 237.5 ms. The second option, the first prompt's map from the pass at the session's opening, was not needed, and is not made.

The third runner was slower across the board (its listing, its map and its cache I/O all 1.2 to 1.4 times the second's), as hosted Windows runners differ by up to two. On the desktop the walk beside the cache took the warm pass and map from 65.9 to 57.0 ms. **On Windows the listing is most of the time, and looking for ignore files in each directory is a third of the whole;** the cache's read and write are 2 to 4 ms. Raised in [answered.md](answered.md). The test of a tree of 100,000 files ran out of the tier's 10 s on Windows while making its files, before its pass.

## The tiers against `main`, on one Windows runner

`try` run 43702202285 (`-f os=windows -f command=test -f against=main`), five rounds in turn, before the walk beside the cache:

| Tier | This branch | `main` | Change | Budget |
|---|---:|---:|---:|---:|
| 1, unit | 16.6 s | 15.7 s | +0.8 s | 20 s |
| 2, integration | 36.7 s | 36.5 s | +0.2 s | 40 s |
| 3, system | 89.8 s | 89.7 s | +0.2 s | 90 s |

`cortex-index` adds 1.8 s to tier 1 and 1.2 s to tier 2. The largest single test is `extract::tests::deep_nesting_costs_no_more_a_line_than_a_flat_file`, 1.36 s on Windows: it is kept at a hundred thousand levels because the quadratic cost it guards is caught by its time alone. Tier 3 was at its budget on `main` already.

## The pull request's CI times

All four jobs passed at `8656056`, in the table below. The Linux runner of the pull request's run started a test's process in 6 ms against main's 3 ms, so it was about half as fast, and its tiers are not this step's cost; the A/B on one Windows runner above is (tier 1 +0.8 s, tier 2 +0.2 s, tier 3 +0.2 s). On Windows, where both runs started processes in 10 ms, every tier is within its budget; main's own tier 3 was over it, at 92.6 s.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 53, on `main` at `7798c25`. "Main" is main's own run at `7798c25` (run 43700052663).
The PR's run is 43706681538, at `8656056`.

| Job | Main, `7798c25` | The PR at `8656056` |
|---|---|---|
| `lint-linux` | 54 s | 52 s |
| `test-linux` | 81 s | 104 s |
| `lint-windows` | 91 s | 70 s |
| `test-windows` | 226 s | 204 s |

| Time above process start, against the budget | Linux, main (3 ms) | Linux, `8656056` (6 ms) | Windows, main (10 ms) | Windows, `8656056` (10 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 5.6 s | 10.3 s | 15.8 s | 16.0 s |
| Tier 2, integration (40 s) | 10.3 s | 19.8 s | 36.5 s | 34.8 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 11.2 s | 15.4 s | 92.6 s | 86.6 s |

<!-- end of what cargo xtask ci-times wrote -->
