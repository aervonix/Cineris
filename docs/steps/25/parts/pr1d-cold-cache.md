# Step 25, a follow-up: a test job on `main` can rebuild a cold cache

Found while bringing step 25's branches across the rename to Cineris, made at the coordinator's request off `main`.

## What was found

The first `ci` run on `main` after the rename (run 44388839866, at `1a4d8a9c`) built from cold on Windows. The build took 3m22s (`cineris-core`, `-rpc`, `-tui`, `-cli` and `xtask`, and the third-party crates syntect loads its grammars with). The job was then cut off by its `timeout-minutes: 5`, 10 s into its tests, at 1029 of 4320. The two tests running at that moment were aborted by the cut, not failed.

Only `main` saves the build cache, and a run that is cut off saves nothing (`CACHE_ON_FAILURE: false`). So once a Windows test job on `main` starts from cold, it can never save a cache: every later run starts from cold too, pull requests' included, and is cut off in turn. Lint and the Linux tests passed: the cold Windows lint took 2m26s.

## What was built

| Where | What |
|---|---|
| `.github/workflows/ci.yml` | The test jobs' `timeout-minutes` is 5 on a pull request, rule 2's budget, and 12 on every other event, with the reason beside it. The lint jobs keep 5. |
| `docs/testing-and-ci.md`, `CONTRIBUTING.md` | Say the same. |

This pull request's own Windows test job will be cut off for the same reason: it reads `main`'s cache, which is still cold. That is not a failure of this change. Once it is merged, the `push` run on `main` has 12 minutes, rebuilds and saves the cache, and pull requests are warm again.
