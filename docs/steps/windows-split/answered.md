# Answered in the Windows split, on 2026-11-27

A change outside the numbered steps, given by the maintainers: the pull-request CI's `test-windows` job split into a build job that makes a `cargo nextest archive` of the tests, and two jobs that run from that archive, tiers 1 and 2 in one and the system tests in the other, each within five minutes of its own. The proposal and its figures are in [ci-budgets-2/ci.md](../ci-budgets-2/ci.md), "Where a base-touching job's five minutes go, and a proposal". The CI times are in [ci.md](ci.md).

## Why

A pull request that changes `cineris-base` builds every crate on Windows again. On a slow runner `test-windows` then ran at 279 to 293 s of its 300, and was sometimes cut off after its tests had passed: half of it the build (126 to 146 s), then the two runs of the tests (about 110 to 120 s), then the runner's own steps (about 35 s). None of it is waste; it does not fit in one job. Rule 2's five minutes hold per job.

## The jobs

| Job | Needs | Runs |
|---|---|---|
| `build-windows` | | The caches as `test-windows` restored them (rust-cache's third-party crates, under the key the job of id `test` had, and the workspace's build, which only `main` saves); `cargo xtask ci test --archive DIR`, which builds the tests as `ci test` builds them into a nextest archive and keeps `xtask`'s binary beside it; the archive kept as the artifact `windows-tests` for a day; on `main`, the workspace's build saved. |
| `test-windows-tiers` | `build-windows` | `cargo xtask ci test tiers --from-archive DIR`: the archive extracted over the checkout, tiers 1 and 2 in the `tiers-windows` group, their budgets. Its report is `junit-windows-tiers`. |
| `test-windows-system` | `build-windows` | `cargo xtask ci test system --from-archive DIR`: the start probe, the system tests in the `system-windows` and `system-windows-shells` lanes, tier 3's budget. Its report is `junit-windows-system`, with the probe's beside it. |

Each has five minutes on a pull request and twelve on every other event, as the test jobs have. Linux is as it was: its job keeps the id `test` (so `ts-client`'s and `try`'s rust-cache keys, made of the id, are unchanged) and is now Linux's alone, without the Windows-only steps it skipped. `pr-size` and `ts-client` need the six blocking jobs.

## What was decided, and by whom

- **The names** (the maintainers): `build-windows`, `test-windows-tiers`, `test-windows-system`, in the style of `lint-windows`.
- **Tier 3's start, measured on its own runner** (the maintainers, on the condition that the review agreed; it did). Each tier is held to its budget above the start of a test's process, measured on the run's own unit and integration tests; a job of system tests alone has none, and the other job's runner is not its runner (hosted runners differ by a factor of two). The system job runs a probe first: the libraries' unit tests of `cineris-base`, `cineris-term` and `cineris-tools`, over 500, about 4 s at two at a time. Its report is kept apart from the job's. A probe that runs no test, or one of whose tests fails, fails the job. The other options were a start of nothing (tier 3 about 2 s stricter on an 8 ms runner) and a fourth job judging the merged report (later, and still the other runner's start).
- **A failed build fails the test jobs** (the maintainers). A job skipped because a job it needs failed reports success, and as a required check would pass with nothing tested. The test jobs run unless the run was cancelled, and their first step fails when `build-windows` did not succeed. **`build-windows` is a required check too.** The required checks at the merge: `lint-linux`, `lint-windows`, `test-linux`, `build-windows`, `test-windows-tiers`, `test-windows-system`.
- **The workspace's build is saved on `main` when the build and its archive succeed,** not after the tests pass: the build is as whole either way, and the tests now run in other jobs.

## What reads the jobs

- `cargo xtask ci-times`, `ci --tiers --compare` and every reader of a run's report (`xtask/src/runs.rs`) take Windows' report as `junit-windows-tiers` and `junit-windows-system` made one, tiers 1 and 2 first, and still read a run from before the split, which kept `junit-windows`. A run that kept one part only has no whole report, and is said to have none. A run of `main` kept Windows' report when it kept both parts, or the one.
- The jobs' table of `ci-times` lists an OS's jobs in the order they run: the lint, the build, tiers 1 and 2, the system tests. The test that every job of `ci.yml` is known as blocking or not knows `build-windows` and `test-windows`.
- `cargo xtask rules` refuses, in a workflow that runs on pull requests, an `actions/download-artifact` that names another run or repository (`run-id`, `github-token`, `repository`).
- Nothing else read the job by its name: `ci-log` chooses jobs by a part of their names, `try-result` reads `try` runs, and the `main`, `nightly` and `try` workflows do not name it (`try`'s comment does, and says `build-windows` now).

## The review before the first push

An independent review of the whole change, against the step's brief, found nothing that let a job pass with tests failed or not run, nor a cache, secret or artifact a pull request could reach. It agreed with the start probe, with the addition below. What it found, and what was done:

| Found | Done |
|---|---|
| A split run's report, made of its two parts, held tier 3 above the tiers job's start: on runners of different speeds, about 2.5 s on the system tests, in `ci-times` and `ci --tiers --compare`. | The readers write the system part's own start, from its probe's report in the artifact, on its suites, and hold its tests above it (`TestCase::start`); the tables say both starts. Tests in `junit`, `runs`, `tiers` and `records`. |
| A re-run of one failed test job could fail on its report's upload, its name taken by the attempt before. | `overwrite: true` on the JUnit uploads, Linux's too. |
| Nothing held the probe to enough tests: a probe of few would measure a test that does real work. | A probe of fewer than 500 fails the job. Test in `tiers`. |
| `ci --tiers --from` a system tests' report read it above a start of nothing. | A report with its probe's beside it is read as the system part. Test in `tiers`. |
| Nothing checked that the test jobs check out where the build job built. | The extraction refuses a checkout at another place than the archive's cargo metadata names. Test in `checks`. |
| A system artifact with its probe's report and not its own stopped `ci-times`; a composite action could download another run's artifacts unchecked. | The first is read as no whole report; the rule holds the repository's actions too. Tests in `runs` and `rules::workflows`. |

A second round, on the fixes, found four more, none letting a job pass with tests failed or not run:

| Found | Done |
|---|---|
| With `overwrite`, a run's report can change, and the readers kept their first download of it for good: a report replaced by a job run again was read as the attempt before. | A download is kept with each artifact's id, and fetched again when GitHub lists another. Test in `runs`. |
| A start probe's report left by an earlier run in a tree made `ci --tiers` there read a whole report as the system part. | The probe's report goes with the earlier run's reports, and only a report of system tests alone is read as that part. Tests in `checks` and `tiers`. |
| The readers took a probe's start without the job's checks of it. | They use the job's own check, and a refused probe makes no report. Test in `runs`. |
| A failed extraction still let the runs after it start, from the wrong place. | Each later step refuses until the extraction succeeded. Test in `checks`. |
