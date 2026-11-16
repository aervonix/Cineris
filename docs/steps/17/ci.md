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
