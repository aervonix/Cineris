# After the size step: three cuts to the shipped binary, and its outlook to step 34

The step changes no test's tiers: its cuts are in shipped code paths that the existing tests already run, and it adds no test. Its runs on the hosted runners measured sizes, one commit or one option at a time.

## The sizes, run by run

`try` runs of `size`, each on one commit; the bytes are the `dist` binary's.

| Run | What it built | Linux musl | Windows |
|---|---|---:|---:|
| `main` workflow 44466604331 | `main` at `6806291f`, the step's base | 15,006,120 | 18,210,816 |
| 44590207285 | The core's handler built once | 14,739,880 | 17,777,664 |
| 44593577843 | The configuration's tables through one helper | 14,686,632 | 17,775,104 |
| 44593583082 | Blocking work as one type | 14,506,408 | 17,478,144 |
| 44594463308 | "z" for ten crates (the droppable commit) | 14,191,016 | 16,974,336 |
| 44594737129 | `panic = "abort"`, on a branch never merged | 12,466,600 | 11,093,504 |
| 44595120646 | QuickJS linked through `rquickjs`, on a branch never merged | 15,330,008 | 18,241,024 |
| 44598003525 | `opt-level = "z"` for the whole profile, on a branch never merged | 13,363,656 | 15,753,728 |

A `try` run started with `-f commit=X` writes its branch's head, not `X`, in the record's `commit` field; the table names what each run built.

## The final head

On the branch as it stands, rebased onto `main` at `bd3adf5d` (the shell environment's second security fix). `main` there is 15,022,504 bytes on Linux musl and 18,235,392 on Windows (its `main` workflow, run 44622100204).

| Run | What it built | Linux musl | Windows |
|---|---|---:|---:|
| `try` 44622406622 | The cuts, the records and the breaks, without the two droppable commits | 14,522,792 | 17,502,720 |
| `main` workflow 44622410782, started by hand on the branch | The final head, with both droppable commits ("z" for ten crates; the budgets) | 14,207,400 | 16,994,816 |

Against `main` at `bd3adf5d`, the three cuts save the same to the byte as they did on step 25's head: 499,712 bytes on Linux and 732,672 on Windows. **The `main` workflow's release start-up step passed on both systems** (`cargo xtask startup` on the `dist` binary, median / 95th percentile; the step records against the budgets and fails on none). Run 44622410782 is the final head; run 44620318897 is the branch before its last rebase, whose shipped code differs only by the security fix:

| | Linux, 44622410782 | Windows, 44622410782 | Linux, 44620318897 | Windows, 44620318897 | Budget (Linux / Windows runner) |
|---|---|---|---|---|---|
| `cineris --version` | 0.37 / 0.50 ms | 6.07 / 6.25 ms | 0.61 / 0.72 ms | 7.69 / 9.43 ms | 15 / 50 ms |
| Headless `cineris -p`, to its request | 9.72 / 9.99 ms | 33.86 / **241.83** ms | 12.36 / 14.83 ms | 43.36 / 52.43 ms | 40 / 100 ms |

Every median is inside its budget. One figure is not: Windows' headless 95th percentile on the final head's run, 241.83 ms against 100, where the run before had 52.43 ms with the same shipped code apart from the security fix. Thirty runs make the 95th percentile the second slowest, so it is one or two slow starts on that runner; it is recorded as it was measured.

## The pull request's first Windows test job

Its first attempt failed tier 3's budget on Windows, 164.0 s against 90 (the table below has that attempt's figures): two tests took 44.4 s (`gates::sys_a_command_that_only_reads_is_told_to_the_loop_as_changing_nothing`) and 22.4 s (`signals::sys_a_killed_harness_leaves_no_command_running`). A `try` A/B of the two on one Windows runner, three rounds each (run 44623339728), put them at 0.489 s and 0.250 s on the branch against 0.486 s and 0.241 s on `main`: the runner, not the change. The job run again passed, in 197 s.

## On the pull request

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
Pull request 113, on `main` at `bd3adf5`. "Main" is main's own run at `bd3adf5` (run 44622100036).
The PR's run is 44622404609, at `670ce19`.

| Job | Main, `bd3adf5` | The PR at `670ce19` |
|---|---|---|
| `lint-linux` | 78 s | 77 s |
| `test-linux` | 149 s | 112 s |
| `lint-windows` | 82 s | 107 s |
| `test-windows` | 245 s | 197 s |
| `pr-size-linux` (blocks nothing) | — | 168 s |
| `ts-client` (blocks nothing) | 0 s | 8 s |
| `pr-size-windows` (blocks nothing) | — | 407 s |

| Time above process start, against the budget | Linux, main (5 ms) | Linux, `670ce19` (4 ms) | Windows, main (6 ms) | Windows, `670ce19` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 28.9 s | 15.8 s | 15.6 s | 19.5 s |
| Tier 2, integration (40 s) | 34.0 s | 22.1 s | 30.5 s | 33.5 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 36.9 s | 33.1 s | 81.8 s | 164.0 s |

<!-- end of what cargo xtask ci-times wrote -->
