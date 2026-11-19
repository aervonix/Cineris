# Answered in the size step, on 2026-11-19

A step outside the numbered plan, made to give step 28 room. After step 25 the shipped Windows binary was 18,210,816 bytes, 96.5% of its 18 MiB budget, and step 28 adds a script engine. The step's measurements, its method and the outlook to step 34 are in [measurements/binary-size.md](../../measurements/binary-size.md), "The size step"; this page holds what it put to the owner, and the owner's answers. The open item it corrected is [the-binary-near-its-size-budget](../../outstanding/the-binary-near-its-size-budget.md).

## What the step did

- **Three cuts that leave behaviour as it is,** each its own commit: the core's request handler built once instead of twice (its future boxed inside `cineris-core`), the configuration's TOML tables collected through one helper (twelve copies of the standard library's sort to one), and blocking work handed to the runtime as one type (53 copies of the runtime's task code to one). **Windows 18,210,816 → 17,478,144 bytes (−732,672), Linux musl 15,006,120 → 14,506,408 (−499,712)**, and the same to the byte on top of the shell environment's second fix, which merged meanwhile: Windows 18,235,392 → 17,502,720, Linux 15,022,504 → 14,522,792. Every `perf_` figure they touch is within noise but one: choosing a round's checks, +0.48 µs (+8%) in two runs, in code none of the cuts changes; the measurement page takes it for where the build placed the code, which no A/B of each cut alone has confirmed.
- **It measured, on branches that are never merged,** the options that cost speed or change behaviour, today's cost of `panic = "abort"`, and QuickJS as step 28 will link it.
- **It found where the returns diminish:** the largest neutral cut left is about 50 KB, and most are under 30 KB (the measurement page ranks them).
- **It corrected** the budget rows of [performance-budgets.md](../../performance-budgets.md) and the open item, which still gave the figures of step 33's size fix.

## For the owner: "z" for ten crates off the hot paths

Built as a commit of its own, to be taken or dropped: `opt-level = "z"` for `cineris-agent`, `-base`, `-config`, `-core`, `-knowledge`, `-policy`, `-rpc`, `-sandbox`, `-session` and `-stats`, set on `release` so that the perf tools measure it. **Linux −315,392, Windows −503,808.** Its speed, against the tree without it, on the reference machine, otherwise idle (the table is on the measurement page, "What costs speed"): **a third to five-sixths slower** on the session store (opening a long session +36%, listing sessions +37%), the permission engine (+38 to +39% a decision, +35% to make it) and choosing a round's checks (+84%); a year's usage report +16%; the redaction filter's throughput 15% higher; the server's round trip and the loop's verification stage unchanged. The five crates whose paths did not slow save only 32,768 bytes of the 315,392.

**The step's recommendation: drop it.** The bytes are in the crates that run on every turn, and two of the paths it slows have little room on Windows already (the usage report is over its budget on a runner; choosing the checks is most of the verification stage's 50 µs there). The maintainers dropped it, on these numbers (below).

## For the owner: the budgets

**The outlook** (the measurement page, "The outlook to step 34"): steps 28 to 34 add **2.95 to 5.6 MB on Linux and 3.8 to 7.3 MB on Windows.** That puts step 34 at 17.5 to 20.1 MB on Linux and 21.3 to 24.8 MB on Windows, with the three cuts and nothing else. Against today's budgets, both are over at the low end of the range, and Windows is over from step 28 on. Two figures under it are measured, not guessed: QuickJS adds 823,600 bytes on Linux and 762,880 on Windows, and a large step of our own code has cost 0.4 to 0.6 MB on Linux at "s", 1.4 times that on Windows. The rest of the range is a guess, and step 30's MCP SDK is the widest part of it.

**The reserves, measured today** (against the three cuts):

| Reserve | Linux musl | Windows | Its cost |
|---|---:|---:|---|
| An MCP client of our own at step 30, in place of `rmcp` | about −0.8 to −1.5 MB of step 30, a guess | more | Tracking both protocol eras by hand ([architecture/13-mcp.md](../../architecture/13-mcp.md)); a design decision for step 30, and no cost in speed |
| "z" for the whole profile | −1,142,752 | −1,724,416 | The ten crates' costs above, and the study's on rendering, highlighting and the map ("z" +100%, +80% and +55% against `opt-level = 3`, where "s" was +50%, +30% and +10%) |
| "z" for ten crates | −315,392 | −503,808 | Above; dropped |
| `panic = "abort"` | −2,039,808 | −6,384,640 | A panic anywhere ends the session, which [decision 0012](../../decisions/0012-release-profile.md) chose unwinding to prevent; steps 28 and 30 are where data the harness did not write meets its parsers |

**Three ways to set the budgets,** each with what is left at step 34 at the low and the high end of the outlook:

| | Budgets (Linux, Windows) | Spent by step 34 | Linux at step 34 | Windows at step 34 | Left at step 34 (Linux; Windows) |
|---|---|---|---:|---:|---|
| **A (recommended)** | **20 MiB, 25 MiB** | Nothing | 17.5 to 20.1 MB | 21.3 to 24.8 MB | **3.5 to 0.9 MB; 4.9 to 1.4 MB**, with every reserve unspent |
| B | 16 MiB, 18 MiB (as now) | Abort at step 28 on both systems; whole-profile "z" on Linux by step 30 | 14.3 to 16.9 MB | 14.9 to 18.4 MB | 2.5 to −0.15 MB; 4.0 to 0.5 MB, with only our own MCP client left (the savings taken as adding up, which they do only roughly) |
| C | 18 MiB, 22 MiB | Nothing until step 30; abort if step 30 lands in the upper half of its range | 17.5 to 20.1 MB | 21.3 to 24.8 MB | 1.4 to −1.2 MB; 1.8 to −1.7 MB before abort |

**Why A.** The budget is there for the vision's footprint: near-instant start, small memory, smooth rendering. Size moves start-up, but by little: when step 33's fix took 4.4 MB off the binary, its first frame on the reference machine went from 1.89 to 1.55 ms, against a budget of 20 ms, and a few megabytes more would cost a few tenths of a millisecond. Unwinding is worth most at steps 28 and 30, which bring scripts and MCP servers whose data the harness did not write. The outlook's spread is mostly steps 28 and 30's dependencies, which those steps will measure. So A sets budgets that leave room at the high end, keeps every reserve, and asks for the size to be looked at again at two points: **when step 28 merges**, with the engine and its bindings measured, and **when step 30's design chooses its MCP client**, where an SDK at the top of its range is the moment to weigh a client of our own against it.

**The order of the reserves** (it replaces decision 0036's "`panic = "abort"` is the reserve"): first, at step 30, an MCP client of our own, which costs no speed; then "z" for the whole profile, whose cost in speed is measured above; then `panic = "abort"`, last, because it is the one that changes what a bug does to a session. The step's last commit sets A's budgets in [performance-budgets.md](../../performance-budgets.md) and in `xtask` (which the `pr-size` job and the `main` workflow check against), and records them and this order in [decision 0039](../../decisions/0039-the-binary-s-budgets-are-20-mib-on-linux-and-25-on-windows.md).

## The independent review

Before the first push, a reviewer who had not seen the step's reasoning read the whole diff against the docs, the code and the measurements' raw logs. It found no change of behaviour in the three cuts: the boxed handlers are as lazy as the `async fn`s were, with the same lifetimes, and `handle_op`'s call of `shutdown` keeps its order of effects; `run_blocking` keeps its value, its panic message and its cancellation; `table_of` keeps every entry, in the table's own order, and the last of two equal keys, as before. It checked every size against the runs' records and every sum, difference and conversion. Its findings, and what was done:

| # | Finding | What was done |
|---|---|---|
| 1 | The session log's flush, +85% under "z", was the disk: each tree's tests wrote under their own target directory, on different storage, and opt-level cannot add 3.6 ms to one `fsync` | Measured again with both trees writing to one directory: the flush is unchanged (4.39 against 4.35 ms); listing and opening sessions still slow by 36 to 37%. The measurement page, this page and the "z" commit say so |
| 2 | With the budget commit kept, its rows still said step 34 was over "this budget" | The rows now say over the old budget, and how much the new one leaves; the open item says which budget its figures are under |
| 3 | The measurement page named the cuts by hashes that only scratch branches held | The cuts are named by their commits' titles |
| 4 | The permission engine's changes were not the medians' | Recomputed from the medians: +39%, +38% and +35% |
| 5 | Decision 0036's amendment said 1.17 MB and "the reference machine" | 1.14 MB (and 1.72 on Windows), measured on the runners |
| 6 | The core's comment cited 190 KB, which the page does not hold | It cites the page's 266 KB and 433 KB |
| 7 | The folding figure mixed two linkers | 102,944 against `rust-lld` without folding, and 94,760 against today's link, both given |
| 8 | "Every `perf_` figure within noise" left out the one that was apart | Said here, with what it is taken for |
| 9 | No break list for the step's code; the runtime-shutdown arm of `run_blocking` has no test (as before the step) | `mutations/size-step.breaks`: four breaks of the cuts' code, each caught (below). The shutdown arm has a test now, `work_handed_to_a_runtime_that_has_ended_is_cancelled` (work given to a runtime after its end is dropped as it is handed over, and the caller is told it was cancelled), and a fifth break that reports it as a panic, which the test catches |
| 10 | A comment line in `Cargo.toml` past the width of its neighbours | Re-wrapped |

**The breaks** (`cargo xtask mutate size-step`): all five are caught. The twenty existing breaks whose code moved into the boxed bodies (`handle_op`, `shutdown_now`, `disconnect_now`) were pointed there, and each was run again: all twenty are caught.

## The owner's answers

- **The budgets: option A.** 20 MiB on Linux (both targets) and 25 MiB on Windows, set in `xtask` and in [performance-budgets.md](../../performance-budgets.md), and accepted as [decision 0039](../../decisions/0039-the-binary-s-budgets-are-20-mib-on-linux-and-25-on-windows.md).
- **The order of the reserves, the owner's:** an MCP client of our own at step 30; then `opt-level = "z"` for the whole profile; then `panic = "abort"`, last. The size is looked at again when step 28 merges and when step 30 chooses its MCP client.
- **"z" for ten crates: dropped,** the maintainers' call on the measured costs: 36 to 84% slower on listing and opening sessions, choosing a round's checks and the permission engine is not worth 315 KB on Linux.
- **The Windows start-up figure** (headless, 241.8 ms at the 95th percentile on one run, against 100: [ci.md](ci.md)) is noted for the change that settles how the start-up budget is checked in CI.
