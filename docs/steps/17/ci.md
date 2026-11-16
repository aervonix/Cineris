# After step 17: the project profile and memory

Step 17 added `cortex-knowledge` (entries, the private store and the repository's shared profile, the text of section 5), two additions to `cortex-policy` (the exact rule of an accepted command, and a running session's learned rules), the epoch's `profile` in the log, and section 5 in the prompt's builder. Its CI times, and the measurements taken on the hosted runners, are below, one part for each of its pull requests.

## The first pull request: the store

On the desktop, built as CI builds (`cargo xtask ci`, 2026-11-16, with a side run going, so the tiers are rough):

| Tier | Tests | Above the tests' start | Budget | Used |
|---|---:|---:|---:|---:|
| 1, unit | 2,131 | 16.1 s | 20 s | 81% |
| 2, integration | 1,312 | 24.7 s | 40 s | 62% |
| 3, system | 206 | 9.5 s | 60 s | 16% |

`cortex-knowledge`'s own 56 tests take 0.06 s of it.
