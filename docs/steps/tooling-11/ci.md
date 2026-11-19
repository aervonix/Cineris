# After the eleventh tooling step: the Windows test job's workspace build cached, and xtask rooted where it runs

The Windows test job's times before the change, each change measured, and the runs that tried it. What was decided is in [answered.md](answered.md).

## Before: where the Windows job's time went

`main` at `8823097` and the seventeen `test-windows` jobs before it that built and ran the tests (2026-11-18 and 19), split by the API's step times and the log's own lines. "Start cost" is the tier report's process start, which tells a runner's speed (5 to 6 ms a fast one, 8 to 9 ms a slow one); a run cut off before the report has none. "xtask build" is `cargo run --package xtask` compiling xtask before it runs anything; "Build tests" the `build tests` step of `cargo xtask ci test`, the workspace's crates and test binaries with the third-party ones already built. All times in seconds.

| Run | Event | End | Start cost | Job | Setup, checkout, changes | Cache restore | Toolchain | nextest | xtask build | Build tests | Tiers 1 and 2 | Tier 3 | Report upload | Post (cache save, clean-up) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 44424135080, attempt 1 (#101) | PR | cut off | 8 ms | 305 | 10 | 8 | 9 | 2 | 24 | 141 | 57 | 46 | 1 | 3 |
| 44424135080, attempt 2 (#101) | PR | cut off | 8 ms | 302 | 8 | 6 | 9 | 3 | 24 | 147 | 53 | 46 | 1 | 3 |
| 44424135080, attempt 3 (#101) | PR | cut off | ? | 305 | 9 | 6 | 9 | 3 | 24 | 158 | 52 | cut off | — | 3 |
| 44417535297 | push | passed | 8 ms | 313 | 10 | 7 | 10 | 3 | 25 | 147 | 53 | 49 | 1 | 3 |
| 44415760689 | PR | passed | 6 ms | 246 | 9 | 6 | 9 | 5 | 20 | 112 | 39 | 40 | 0 | 3 |
| 44412943430 | PR | passed | 5 ms | 202 | 9 | 5 | 9 | 3 | 16 | 89 | 31 | 35 | 1 | 2 |
| 44410799170 (cold) | push | passed | 8 ms | 405 | 10 | 3 | 10 | 5 | 55 | 205 | 52 | 51 | 1 | 11 |
| 44409483773 (cold) | push | failed | 8 ms | 394 | 8 | 2 | 9 | 3 | 50 | 200 | 50 | 65 | 1 | 3 |
| 44395064470 (cold) | PR | failed | 6 ms | 291 | 8 | 2 | 9 | 2 | 39 | 152 | 37 | 37 | 1 | 2 |
| 44386912802 | PR | passed | 6 ms | 209 | 10 | 9 | 8 | 4 | 15 | 85 | 34 | 38 | 1 | 2 |
| 44383998597 | PR | passed | 6 ms | 240 | 10 | 11 | 9 | 3 | 20 | 108 | 36 | 36 | 2 | 2 |
| 44380789740 | push | passed | 8 ms | 293 | 11 | 12 | 10 | 3 | 25 | 130 | 47 | 48 | 2 | 3 |
| 44378553938 | PR | passed | 6 ms | 231 | 9 | 9 | 9 | 3 | 18 | 101 | 37 | 39 | 2 | 2 |
| 44378499657 | PR | passed | 5 ms | 209 | 9 | 8 | 9 | 3 | 18 | 93 | 31 | 32 | 2 | 2 |
| 44366352350 | PR | passed | 8 ms | 283 | 9 | 11 | 11 | 3 | 22 | 130 | 46 | 43 | 2 | 3 |
| 44364311801 | PR | passed | 6 ms | 247 | 11 | 9 | 9 | 3 | 21 | 107 | 41 | 39 | 2 | 2 |
| 44352361969 | push | passed | 5 ms | 184 | 10 | 9 | 9 | 2 | 15 | 75 | 27 | 31 | 1 | 2 |

- **On a slow runner (8 ms) with a warm cache the job took 293 to 313 s.** Setup, the cache, the toolchain and nextest took about 35 s; xtask's own build 22 to 25 s; the build of the workspace's crates 130 to 158 s; the tiers 95 to 110 s; the upload 1 to 2 s.
- **Pull request #101, a change to no crate, was cut off three times at 5m02 to 5m05**, every test having passed in the first two (the third was cut 39 s into tier 3). It built every crate, as every pull request did.
- **The cold runs** (after the rename to Cineris emptied the cache, marked) took 6.5 to 6.75 minutes on `main`, which the twelve minutes of #97 allow.
- **The upload is not where the time goes:** 0 to 2 s, the report 125 KB.
- **The tiers relink nothing:** the second and third `cargo nextest run` of the job each find the build fresh in 0.1 to 0.2 s (`Finished … in 0.23s` in the log), so a nextest archive would save nothing.

## The pull request's CI

<!-- written by cargo xtask ci-times, section "pull-request": it replaces this up to the end mark -->
Pull request 103, on `main` at `f6fedc3`. "Main" is main's own run at `f6fedc3` (run 44440506912).
The PR's run is 44444715888, at `a536785`.

| Job | Main, `f6fedc3` | The PR at `a536785` |
|---|---|---|
| `lint-linux` | 61 s | 63 s |
| `test-linux` | 134 s | 114 s |
| `lint-windows` | 100 s | 101 s |
| `test-windows` | 237 s | 274 s |
| `pr-size-linux` (blocks nothing) | — | 213 s |
| `ts-client` (blocks nothing) | 0 s | 64 s |
| `pr-size-windows` (blocks nothing) | — | 398 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `a536785` (3 ms) | Windows, main (6 ms) | Windows, `a536785` (8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 20.9 s | 14.2 s | 14.5 s | 16.1 s |
| Tier 2, integration (40 s) | 46.4 s | 18.9 s | 33.1 s | 36.8 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 37.3 s | 25.5 s | 71.6 s | 86.9 s |

<!-- end of what cargo xtask ci-times wrote, section "pull-request" -->

## The new job on a scratch branch standing in for `main`

The branch's `ci.yml` on a scratch branch (`exp-wci-3`), the main branch's role chosen by a dispatch input (a branch's caches are read only by its own runs; [answered.md](answered.md), "Each case tried on a runner"). The main branch's role built everything and saved; the pull request's role restored that build and dated the checkout. Columns as in "Before", with the two new steps.

| Run | Role and change | End | Start cost | Job | Workspace restore | Dating | xtask build | Build tests | Tiers 1 and 2 | Tier 3 | Notice and save |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 44438435795 | main | passed | 6 ms | 252 | — | — | 20 | 109 | 41 | 38 | 6 |
| 44436126823 | pull request, `cineris-tools` | passed | 8 ms | 197 | 7 | 1 | 1 | 54 | 48 | 48 | 0 |
| 44441475338 | pull request, `cineris-tools` (1 of 3 in a row) | passed | 6 ms | 177 | 9 | 2 | 1 | 46 | 41 | 39 | 0 |
| 44441919926 | pull request, `cineris-tools` (2 of 3) | failed: the known flake `a-verify-test-waits-on-a-timer-nothing-advances` | 8 ms | 203 | 6 | 1 | 1 | 59 | 57 | 47 | 0 |
| 44442390559 | pull request, `cineris-tools` (3 of 3) | passed | 8 ms | 199 | 6 | 1 | 1 | 57 | 51 | 46 | 0 |
| 44439011586 | pull request, `.gitattributes`: nothing restored | passed | 8 ms | 298 | 1 | 0 | 24 | 135 | 50 | 46 | 1 |
| 44440563206 | pull request, a test written after the checkout: dating refused | failed, as it must | 5 ms | 204 | 5 | 2 | 16 | 88 | 32 | 33 | 0 |
| 44440580488 | pull request, the toolchain: nothing restored | passed | 7 ms | 388 | — | 1 | 53 | 197 | 50 | 50 | 0 |

- **An ordinary change on an 8 ms runner: 197 to 203 s**, against 293 to 313 s before. The workspace's build is 54 to 59 s, and xtask's 1 s.
- **Restoring the workspace's build costs 6 to 9 s for 337 to 363 MB, and dating the checkout 1 to 2 s.** Saving it on the main branch costs 6 to 8 s.
- **A fallback costs today's job and the restore:** 298 s on an 8 ms runner for the attributes case, against 293 to 313 s before.

## The nightly on the branch

Run 44444725527, dispatched on the branch, since xtask's root changed and the nightly runs xtask.
- Every job passed but `properties`, which failed on the known flake `token_by_token_commits_what_all_at_once_does_for_random_documents`.
- That job failed on `main`'s own nightlies of 2026-11-17 and 18 as well.

## One runner, both ways: the A/B

How it was run:
- **The harness:** a job on a scratch branch from `main` at `f6fedc3` (`exp-wci-4`, `.github/exp/exp2.py`).
- **Setup:** it builds `main`'s tree once through the job's own command and packs that build as the main branch would save it. It then commits a one-line change to one crate (the pull request).
- **The rounds:** five, in alternating order, of:
  - **today's job command** (`cargo run --package xtask -- ci test`): the workspace's build removed and every file dated now, as a fresh checkout dates them;
  - **the cached job:** the packed build unpacked and the checkout dated by `.github/scripts/date-the-checkout.sh`, then the same command.
- **What it leaves out:** what both sides pay alike, about 34 s on a slow runner (setup, the third-party cache, the toolchain, nextest, the upload, clean-up: "Before" above), and the cached side's download, about 4 s of its 6 to 9 s of restore (the unpacking is in the table).
- **The runs:** 44449826267, 44449829150 and 44449831953. Times in seconds, the median of five rounds, each round's in the run's `exp2.json`.

| The pull request changes | Runner | Side | xtask build | Build tests | Tiers 1 and 2 | Tier 3 | Unpack and date | Command, median | The job, with the shared parts |
|---|---|---|---|---|---|---|---|---|---|
| `cineris-tools` (a crate in the middle) | 8 ms | today | 22.4 | 140.8 | 52.5 | 47.6 | — | **266.5** (258.1 to 270.8) | about 300 |
| | | cached | 0.3 | 57.3 | 53.3 | 47.3 | 3.7 | **162.9** (156.9 to 169.7) | **about 201** |
| `cineris-cli` (a leaf) | 6 ms | today | 18.1 | 109.2 | 37.9 | 36.2 | — | **201.8** (199.5 to 202.5) | about 236 |
| | | cached | 0.3 | 6.9 | 38.9 | 36.4 | 3.1 | **89.0** (84.0 to 90.7) | **about 127** |
| `cineris-testkit` (under nearly every test binary) | 6 ms | today | 16.9 | 103.9 | 36.8 | 35.7 | — | **193.4** (193.0 to 196.1) | about 227 |
| | | cached | 0.2 | 69.1 | 37.3 | 35.8 | 2.8 | **145.9** (144.0 to 153.2) | **about 184** |

- **The aim is met on a slow runner:** with a change to a crate in the middle, the job comes to about 201 s at the median, 3 min 21 s, against about 300 s today. The real runs above agree: 197 to 203 s on 8 ms runners.
- **A leaf change** builds in 7 s. A change to no crate at all (#101's kind) builds in about 0.2 s ([answered.md](answered.md), "What was tried").
- **The tiers do not move:** 52.5 against 53.3 s and 47.6 against 47.3 s on the slow runner. The tests are the same binaries.
- **The known slow case:** a change to `cineris-testkit` still rebuilds 69 s of a 104 s build on a 6 ms runner. Scaled by the slow runner's build (140.8 against 109.2 s on the leaf's runner, about 1.3 times), that is about 90 s of build and about 240 s of job on an 8 ms runner. A change to `cineris-base` rebuilds 72 of 82 s ("What was tried"), about 4.3 minutes there. The coordinator accepted this case.

### The A/B's first tries, and what they showed

- **Runs 44438574322, 44438578004 and 44438583991** used a harness that had left Python's `.pyc` cache beside itself, an untracked file. The dating refused it ("files that are not its commit's") and every crate was rebuilt, as designed. The cached side then took 3 to 6 s longer than today's at the median, which is the cost of a fallback.
- **Runs 44444591999, 44444595499 and 44444598564** built the stand-in for `main`'s cache with nextest directly, not through `cargo xtask ci test`. The two builds' environments differ, so most test binaries were rebuilt against it. The cached side still gained 26 to 48 s, and the runs on the scratch branch, where both builds go through xtask, did not show it.
- **The runs above** build it as the main branch's job does.

## After the merge: `main`'s saves and the first three pull requests

**The main branch's saves.** Each saved the workspace's build after its tests. That build is what the pull requests below restored.

| Run | `main` at | Start cost | Windows job | Build tests | Save |
|---|---|---|---|---|---|
| 44457639460 | `45f0ee0` (#103) | 8 ms | 309 s | 144 s | 8 s, 363 MB |
| 44458332206 | `fe2ee71` (#105) | 5 ms | 196 s | 82 s | 5 s, 363 MB |

**The first three pull-request runs that touched code and started after the first save** (04:25:32Z on 2026-11-19), as they came, none made for the purpose. Times in seconds, as in "Before".

| Run | Pull request | Start cost | Job | Workspace restore | Dating | Files differing from the build | xtask build | Build tests | Rebuilt | Tiers 1 and 2 | Tier 3 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 44458467969 | #104, two flaky tests, at `b16a354` | 8 ms | **222** | 6 | 1 | 15, against `45f0ee0` | 1 | 71 | `cineris-tools`, `cineris-tui` and what depends on them (8 crates) | 51 | 50 |
| 44459695495 | #104 again, at `8026226` | 7 ms | **176** | 8 | 1 | 13, against `fe2ee71` | 1 | 54 | the same 8 crates | 40 | 40 |
| 44460417305 | #106, the public docs, at `9b516a6` | 6 ms | **231** | 10 | 1 | 84, against `fe2ee71` | 11 | 96 | every workspace crate: it changes `cineris-base` | 39 | 39 |

- **All three passed, well inside five minutes:** 222 s on a slow runner (8 ms), against 293 to 313 s before.
- **Each restored the newest build of `main` at the time**, and dated the checkout against it.
- **#106 is the known slow case:** it changes `cineris-base`, under every crate. It still took 231 s on a 6 ms runner, about 4.3 minutes on a slow one by the A/B's ratio.
