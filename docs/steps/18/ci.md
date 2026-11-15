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

**The pass costs the run nothing that can be measured:** with the map on and off the medians are within the runs' own noise (an earlier run of the same tree, before the breakdown row, gave 17.65 ms with the map on). So the pass stays where it is, started once the loop runs. What a real tree adds to a run, about 1.5 ms at the median with the map off as on, is older work: the session's base state, which outside a git repository lists and hashes the workspace on a thread of its own (the desktop's copy of the tree has no `.git`), and the larger instruction files of this repository. The start-up budget is measured in an empty workspace, as before.

## The pull request's CI times

To be filled from the pull request's run.
