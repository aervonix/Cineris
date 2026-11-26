# 0039: The binary's budgets are 20 MiB on Linux and 25 on Windows

- **Status:** Accepted by the owner on 2026-11-19: option A of the size step, with the owner's order of the reserves. It replaces decision 0036's "`panic = "abort"` is the reserve"; the rest of 0036 stands. Revisited on 2026-11-26, when step 28 merged: its measured size, above its range in the outlook, and the outlook to step 34 again, pending the owner's review; the budgets and the reserves stand unchanged ("Revisited when step 28 merged", below).
- **Date:** 2026-11-19

## Context

After step 25 the shipped Windows binary was 18,210,816 bytes, 96.5% of its 18 MiB budget, and the Linux musl binary 15,006,120, 89.4% of its 16 MiB. The size step cut 732,672 bytes on Windows and 499,712 on Linux without changing behaviour, and projected steps 28 to 34 at 3.8 to 7.3 MB more on Windows and 2.95 to 5.6 MB on Linux: over both budgets at the low end of the range, and over Windows' from step 28 ([measurements/binary-size.md](../measurements/binary-size.md), "The size step"). Of that range, QuickJS (step 28) is measured; step 30's MCP SDK is the widest guess.

[Decision 0036](0036-the-shipped-binary-is-optimised-for-size.md) named `panic = "abort"` as the reserve. Today it saves 2,039,808 bytes on Linux and 6,384,640 on Windows, and it is the option that changes what a bug does to a session ([decision 0012](0012-release-profile.md)).

## Options

The three the size step put to the owner, with what each leaves at step 34 at the low and the high end of the outlook ([steps/size-step/answered.md](../steps/size-step/answered.md)):

| | Budgets (Linux, Windows) | Spent by step 34 | Left at step 34 (Linux; Windows) |
|---|---|---|---|
| A | 20 MiB, 25 MiB | Nothing | 3.5 to 0.9 MB; 4.9 to 1.4 MB, every reserve unspent |
| B | 16 MiB, 18 MiB | Abort at step 28, whole-profile "z" on Linux by step 30 | 2.5 to −0.15 MB; 4.0 to 0.5 MB |
| C | 18 MiB, 22 MiB | Abort if step 30 lands in the upper half of its range | 1.4 to −1.2 MB; 1.8 to −1.7 MB before abort |

## Decision

- **The budgets are 20 MiB on Linux (both targets) and 25 MiB on Windows**, checked as before by `cargo xtask size --check` in the `main` workflow, `nightly` and each pull request's `pr-size` job.
- **The size is looked at again twice before step 34:** when step 28 merges, with the engine and its bindings measured, and when step 30's design chooses its MCP client.
- **The reserves, in order:** at step 30, an MCP client of our own in place of `rmcp`, which costs no speed; `opt-level = "z"` for the whole profile (1.1 MB on Linux, 1.7 MB on Windows), whose cost in speed the size step measured on the session log, the permission engine and the verifier; `panic = "abort"` (2.0 MB, 6.4 MB), last. This replaces decision 0036's "`panic = "abort"` is the reserve".

## Why

- **The footprint the vision asks for is start-up, memory and smooth rendering,** and the binary's size moves them little: when step 33's fix took 4.4 MB off, the first frame on the reference machine went from 1.89 to 1.55 ms, against 20 ms.
- **Unwinding is worth most at steps 28 and 30,** which bring scripts and MCP servers whose data the harness did not write. Spending abort to make room for them would give up the containment they most need.
- **The budgets leave room at the high end of the outlook** and keep every reserve, so that a step that lands high is met by a reserve, not by a budget raised in haste.

## What is given up

- A larger download and install: up to about 7 MB more on Windows and 4 MB more on Linux than the old budgets allowed.
- The old budgets' pressure: a step has more room before it must look for cuts. The pull request's `pr-size` job still shows each step's growth against `main`.

## Revisit when

- Step 28 merges: its measured size against the outlook.
- Step 30's design chooses its MCP client.
- A step lands outside its range in the outlook.

## Revisited when step 28 merged (2026-11-26), pending the owner's review

Written by the maintainers from the measurements, at the revisit this decision names. It changes neither a budget nor a reserve: those are the owner's to change.

**Measured.** The shipped `dist` binary, built on hosted runners by `cargo xtask size`:

| | Linux musl | Windows |
|---|---:|---:|
| Before step 28, at `119f09c2` (`main` just before its first pull request, #117) | 14,522,792 bytes | 17,498,112 bytes |
| When step 28 merged, at `69090915` | 17,513,528 bytes (16.70 MiB) | 21,224,448 bytes (20.24 MiB) |
| Growth in all | +2,990,736 | +3,726,336 |
| **Step 28's own:** the sum of its pull requests' own size checks | **+2,884,240** | **+3,613,696** |
| The other pull requests' between (#123 +4,096 and +4,608; #125 +98,304 and +96,768; #128 nothing and +6,656; #130 +4,096 and +4,608; #119 and #120 nothing) | +106,496 | +112,640 |

Step 28's own, by pull request (Linux, then Windows): #117, the engine and its host functions, +1,261,936 and +1,383,424; #118, `fetch`, +69,664 and +109,056; #121, extensions found and loaded, +352,352 and +480,768; #122, skills, +143,360 and +192,000; #124, protected writes, +20,480 and +28,672; #126, the hooks' seam, +307,296 and +445,952; #127, script hooks, +135,200 and +182,784; #129, custom commands, +225,312 and +286,720; #131, managing extensions and the harness's own approvals, +368,640 and +504,320. The figures add up, to the byte, to the two ends measured. The records are in [steps/28/ci.md](../steps/28/ci.md).

**Against the outlook, step 28 landed above its range on both systems:** +2.88 MB on Linux against +1.3 to +1.8 MB, 1.1 MB over its high end, and +3.61 MB on Windows against +1.45 to +2.0 MB, 1.6 MB over; 1.6 to 2.2 times its range on Linux, and 1.8 to 2.5 times on Windows. **The engine came in as measured beforehand; our own code is what the outlook missed.** A build of the same code with the engine's feature off (`scripts`, a branch never merged) is 1,737,296 bytes smaller on Linux and 2,094,592 on Windows: QuickJS linked alone was measured at +823,600 and +762,880, so the script hosting behind the feature is about 0.9 and 1.3 MB of it, and step 28's code outside the feature (custom commands, skills, command hooks, the management, the permission engine's refusal of the harness's own approving commands and their own check) about 1.15 and 1.5 MB more. The outlook had priced our own code at 0.4 to 0.6 MB on Linux for a step of its size. In the Linux binary's symbols (`cargo xtask size --breakdown`, which leaves C code out) the crates that grew most are `cineris_core` +450 KB, `cineris_ext` +214 KB, `serde_json` +84 KB, `cineris_policy` +83 KB, `cineris` +81 KB, `cineris_agent` +65 KB, `tokio` +57 KB and `rquickjs_core` +47 KB.

**What is left, and the outlook again.** To the budgets: 3,457,992 bytes on Linux and 4,989,952 on Windows. The outlook for steps 29 to 34 ("The outlook to step 34" in [measurements/binary-size.md](../measurements/binary-size.md), less step 28's row) was +1.65 to +3.8 MB on Linux and +2.35 to +5.3 MB on Windows. Taken again with step 28's ratio, each low end 1.6 times and each high end 2 times as large:

| Steps 29 to 34 | Linux musl | Windows | Against what is left |
|---|---:|---:|---|
| The outlook as written | +1.65 to +3.8 MB | +2.35 to +5.3 MB | within at the low end on both; over at the high end by 0.34 MB on Linux and 0.31 MB on Windows |
| Every step's range taken again | +2.7 to +7.6 MB | +3.8 to +10.6 MB | within at the low end on both; over at the high end by 4.1 MB on Linux and 5.6 MB on Windows |
| The guesses taken again, step 30 as written | +1.9 to +5.0 MB | +2.6 to +7.0 MB | within at the low end, by 1.6 MB on Linux and 2.4 MB on Windows; over at the high end by 1.5 MB and 2.0 MB |

Step 30's range is mostly a library's recorded cost (`rmcp`), which the ratio for our own code does not fit; the third row leaves it as written, and is the one the maintainers take as likely. On it, the reserves in their order cover the high end: an MCP client of our own at step 30, then `opt-level = "z"` for the whole profile (1.1 MB on Linux, 1.7 MB on Windows), then `panic = "abort"` (2.0 MB, 6.4 MB).

**What stands.** The budgets, 20 MiB and 25 MiB, and the reserves in their order, unchanged; **step 30's design, choosing its MCP client, is the next point to decide whether a reserve is spent.** Pending the owner's review.
