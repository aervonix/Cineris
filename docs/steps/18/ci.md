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

To be filled from the pull request's run.
