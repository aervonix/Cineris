# Answered in the seventh tooling step, on 2026-11-16

This step is not a numbered step of the build plan: it built the seventh round of developer tooling, from what wave 10's five sessions asked for. Its records are here, in a folder named for the job.

## Windows' integration tier: where its time goes

**The profile** (`try` run 43778010011, `main` at `bd2e6a1`, tier 2 alone with `-f testkit=profile`, an 11 ms runner): 35.1 s above the start of the processes, spread thin. The slowest 20 tests were a quarter of it.

| Crate | Tests | Above the start | Median test |
|---|---|---|---|
| `cortex-core` | 115 | 8.2 s | 62 ms |
| `cortex-llm` | 269 | 6.1 s | 4 ms (the replays at every cut: 3.1 s in three tests) |
| `cortex-session` | 57 | 4.7 s | 11 ms (the durability tests: 4.0 s in ten) |
| `cortex-tools` | 185 | 4.3 s | 9 ms |
| `cortex-agent` | 244 | 3.5 s | 9 ms |
| `cortex-index` | 53 | 2.2 s | 18 ms |

- **The slowest tests:** `durability::an_entry_over_the_line_limit_is_refused_and_nothing_is_lost` 1.8 s (a 66 MiB line serialised unoptimised), `openai_chat::decode::at_every_cut::deepseek` 1.5 s, `durability::a_log_cut_at_every_byte_of_a_last_response_opens_cleanly` 1.2 s, the other two servers' replays 0.9 and 0.75 s, `approvals::an_approval_for_a_change_to_a_file_shows_the_change_within_a_bound` 0.6 s (a 600 kB write diffed).
- **`cortex-core`'s time is the session's set-up on Windows,** not the tests' work: from the profiles, opening a session took 19.5 ms on average (122 of them, 2.4 s), a turn 15 ms; the same test opens a session in 1.4 ms at the desk. Timed inside the core on a scratch branch (`try` 43780823181, one test at a time): `session/new` took 20.1 ms on a Windows runner against 3.3 ms on a Linux one, and 9.2 ms of the 20 was 27 constructions of the path resolver, 0.34 ms each (0.64 ms for all 27 on Linux). 20 of them, 6.5 ms, are `engine_parts`'s `resolve_dir`, which builds a resolver for a path that does not exist yet and, when that fails, one for each parent in turn, each walking the path from its root. The fix (the deepest ancestor that exists, found with a plain `metadata`, and one resolver there) is product code in `cortex-core`, where step 23 is working: it waits for step 23's first pull request.
- **Windows Defender is not the cost:** its real-time scanning is off on the hosted runners, with `C:\` and `D:\` excluded (read on a runner, `try` 43779123608).

## The owner's decision: Windows' tiers 1 and 2 three at a time

No test change found takes 9 s off without testing less: the time is a few milliseconds of the operating system's in each of a thousand tests. Four ways were measured, each against `main` on one runner (`try` with `-f against=main -f count=3`):

| Change | Run | Runner | Tier 2, `main` → change | Tier 1 | Wall time of the tiers 1 and 2 run | At 11 ms, tier 2 about |
|---|---|---|---|---|---|---|
| Tiers 1 and 2 three at a time | 43778700279 | 6 ms | 23.4 → 19.0 s (−19%) | 11.3 → 7.9 s | 13.6 → 15.1 s (+1.5 s) | 35 s |
| Tiers 1 and 2 two at a time | 43778703107 | 8 ms | 38.3 → 23.6 s (−38%) | 19.4 → 9.5 s | 20.8 → 28.8 s (+8 s) | 27 s |
| `cortex-llm` at `opt-level = 1` in the `ci` build | 43778840384 | 11 ms | 43.1 → 38.8 s (−10%) | 19.4 → 17.7 s | unchanged; the build +9.3 s warm and +11.5 s cold on Windows, +5.7 and +6.2 s on Linux (43780885900) | 39 s |
| Three at a time and `cortex-llm` optimised | 43780837934 | 10 ms | 39.6 → 31.9 s (−19%) | 18.3 → 14.0 s | 22.0 → 25.3 s (+2.6 s), and the build | 34 s |

`main` itself read its integration tier at 43.1 s on the 11 ms runner of the third row: over its 40 s budget.

The coordinator held two at a time, which reached the 30 s target, because it made the Windows job about 8 s longer to lower a figure that falls as the tests stop slowing each other. **The owner decided (2026-11-16) on three at a time on Windows, Linux unchanged,** with the target "under the 40 s budget at 11 ms, with room" in place of 30 s: tier 2 about 35 s at 11 ms, for about 1.5 s of the job's time. **`cortex-llm` at `opt-level = 1`: no.** Every CI job builds every workspace crate (the cache holds only third-party crates), so every pull request would pay its 6 to 11 s of build for less than 2 s of the test run's wall time (this settles the open item `cortex-llm-optimised-in-the-ci-build`, deleted). The session-open fix is this step's, once step 23's first pull request has merged.
