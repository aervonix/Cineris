# 0039: The binary's budgets are 20 MiB on Linux and 25 on Windows

- **Status:** Accepted by the owner on 2026-11-19: option A of the size step, with the owner's order of the reserves. It replaces decision 0036's "`panic = "abort"` is the reserve"; the rest of 0036 stands.
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
