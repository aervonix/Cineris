# The shipped binary's size

What the shipped binary (`cortex`, the `dist` profile) is made of, what each way of making it smaller saves and costs, and what step 33's size fix did ([decision 0036](../decisions/0036-the-shipped-binary-is-optimised-for-size.md)). Measured on 2026-11-16: the size study (read-only, at `main` `c225d64`, after step 23), then the fix, before and after, on the desk and on both hosted runners.

## Method

- **Windows and Linux musl sizes** are the `main` workflow's: `cargo xtask size --check --target TARGET` on its hosted runners, `--locked`, the `dist` profile, stripped (`strip = true`). A branch's sizes come from the same workflow started by hand on it (`gh workflow run main --ref BRANCH`), whose `report` job reports nothing off the main branch. They are what the budgets in [performance-budgets.md](../performance-budgets.md) ("Binary size") are checked against.
- **The desk's sizes** are of the gnu target (`x86_64-unknown-linux-gnu`), built in the `dist` profile on the desktop. The study built each variant unstripped and then ran `strip --strip-all`, which leaves 14,776 bytes less than rustc's own strip: compare the study's desk figures only with each other. The fix's desk figures are `cargo xtask size`'s, stripped by rustc, as the runners'.
- **What the binary is made of** was read from an unstripped desk build: the size of each symbol, put to its crate by its mangled path, and the read-only data by what it holds.
- **Speed** was measured in `release`, which the perf tools build (`cargo xtask perf`, the `perf_` tests, `try -f profile=release`), and start-up on the `dist` binary itself (`cargo xtask startup --profile dist`, 30 runs). The study measured at the desk, three rounds a variant against `opt-level = 3`. The fix measured before and after, on the desk with `rx --exclusive` and on one hosted runner of each system through `try`, each side on a runner of its own: Windows runners differ in speed by up to a factor of two, so a hosted before-and-after is a bound, not a precise ratio.

## Where it stood (the study, `c225d64`)

| | Bytes | Against the budget |
|---|---:|---|
| Windows (`x86_64-pc-windows-msvc`) | 19,226,112 (18.34 MiB) | **351,744 over** 18 MiB; `size-windows` failed on run 43811142390 and opened issue #61 |
| Linux musl (`x86_64-unknown-linux-musl`) | 16,258,408 (15.51 MiB) | 518,808 under 16 MiB |
| Linux gnu, at the desk | 16,114,768 | |

## The options, largest Windows saving first

Bytes saved on Windows, Linux musl and the desk's gnu build.

| # | Option | Windows | Linux musl | Desk gnu | Notes |
|---|---|---:|---:|---:|---|
| 1 | `opt-level = "s"` and `panic = "abort"` | 10,045,952 | 5,787,648 | 5,767,528 | Everything below for "s" and for abort. |
| 2 | `opt-level = "z"` | 6,594,048 | 5,115,872 | 5,094,928 | The views' frame +100%, highlighting +80%, the map +55%, the first frame +25%. Not recommended. |
| 3 | `panic = "abort"` | 6,457,856 | 2,166,784 | 2,164,720 | See "The cost of unwinding", below. |
| 4 | **`opt-level = "s"` for everything (chosen)** | **5,271,552** | **4,247,552** | **4,229,856** | The study's speed figures are in "The cost of 's'", below. |
| 5 | Our crates at "s", third-party at 3 | 4,007,424 | 3,170,304 | 3,166,496 | |
| 6 | Third-party at "s", ours at 3 | 1,216,512 | 1,134,592 | 1,121,136 | |
| 7 | `opt-level = 2` | 970,240 | 606,208 | 598,592 | Nothing measurable on the views, the first frame, highlighting or the map. Search, diffing and JSON decoding not measured. |
| 8 | Highlighting off | 647,680 | 626,784 | 625,432 | Decision 0015 wants it. |
| 9 | `reqwest` without `http2` | 553,472 | 465,216 | 461,144 | Decision 0004 wants HTTP/2. |
| 10 | Non-PIE on Linux | | | 441,720 | Gives up address-space randomisation. Not recommended. |
| 11 | A trimmed grammar set | 183,227 | 183,227 | 183,227 | Needs the grammars' sources and a packer. |
| 12 | **`idna_adapter` held at 1.0 (chosen)** | **133,632** | **139,264** | **136,512** | 21 fewer crates (ICU goes). A host with letters outside ASCII is refused; its `xn--` form works. Holding it at 1.1 instead grows the binary by 125,888 over 1.2. |
| 13 | `tracing` at most `info` in release | | | 104,776 | |

**No gain:** TLS 1.2 off (`hyper-rustls` turns it back on); duplicate crates (none in the binary; `syn` 2 and 3 are build-time only); thin LTO (+1,124,992; with 16 codegen units, +4,534,328): fat LTO with one codegen unit is right; the `-Z` flags, which are not stable on 1.99.1.

### The cost of "s" (the study)

Release build, desk, three rounds, against `opt-level = 3`:

| Measure | `opt-level = 3` | `"s"` | Budget |
|---|---|---|---|
| The views' frame, a 30-row live region | 0.095 to 0.10 ms | 0.145 to 0.149 ms (+50%; 99th 0.22 to 0.26 ms) | 2 ms at the 99th |
| Highlighting 200 lines from cold | 42 to 46 ms | 56 to 59 ms (+30%) | off the UI thread |
| The map alone | 5.8 to 6.5 ms | 6.6 to 7.1 ms (+10%) | |
| Start to first frame | 1.64 to 1.89 ms | 1.49 to 1.60 ms (faster) | 20 ms |
| The desk's build of `dist` | 93 s | 67 s | |

### The cost of unwinding (a correction to decision 0012)

[Decision 0012](../decisions/0012-release-profile.md) chose `panic = "unwind"` on a measured cost of 0.17 to 0.27 MB (2.5 to 3.7%), on a spike with the core dependency set. On the real binary it costs **2.2 MB on Linux** (2,166,784 bytes on musl, 2,164,720 on the desk's gnu build) and **6.5 MB on Windows** (6,457,856 bytes, a third of the binary): 8 to 25 times the spike's figure. The owner kept unwinding, and left decision 0012 as it is; this page holds the correction. What abort would cost: every `catch_unwind` in shipped code (the agent's tool boundary in `cortex-agent`'s `guarded.rs`, the interface's two catches, the repository map's three, `cortex-tools`' `blocking.rs` and process capture, `cortex-term` on Unix) would stop catching, and a panic in a Tokio task or in the highlighter's thread would end the process.

## What it is made of (`c225d64`, desk gnu, 16.1 MB)

- **Code, 12.21 MB (76%):** async state machines 1.12 MB; serde's derives 1.07 MB (`Deserialize` 886 KB, `Serialize` 179 KB); `Clone`, `Eq` and `Hash` 403 KB; `Debug` 331 KB; panic paths 141 KB; `tracing` 118 KB.
- **Unwinding tables, 1.49 MB (9%).**
- **Read-only data, 1.60 MB (10%):** syntect's grammars 368 KB; ring's tables about 180 KB; `encoding_rs`'s CJK tables 118 KB; panic-location source paths 92 KB (66 KB of them the registry's and the standard library's); other text 323 KB.
- **Position-independent relocations and pointer tables, 0.79 MB.**
- **By owner:** our crates 6.65 MB; third-party 5.1 MB; the standard library 0.79 MB.
- **The largest crates:** `cortex-tools` 793 KB, `cortex-core` 715 KB, `cortex-tui` 643 KB, `cortex-protocol` 633 KB, `tokio` 629 KB, `cortex-llm` 563 KB, `cortex-session` 538 KB, `cortex-agent` 525 KB, `ring` about 520 KB, `cortex-stats` 487 KB, `cortex-config` 479 KB, `regex-automata` 470 KB, `rustls` 446 KB, `serde_json` 384 KB.

## Growth by commit

Desk gnu, Linux musl and Windows bytes, each against the commit before.

| Commit | Desk gnu | Linux musl | Windows | What |
|---|---:|---:|---:|---|
| `d050503` | 14,111,096 | 14,246,888 | 16,882,688 | The base |
| `bd2e6a1` | +9,968 | 0 | +8,192 | |
| `e1ac07a` | +63,952 | | | Step 23's first part, before the binary was wired: configuration keys (`cortex-config` +30 KB, `toml` +13 KB) |
| `a1f19cf` | +1,439,000 | | | "Start interactive mode in the binary": highlighting about +510 KB (grammars 368,467, syntect 62 KB, fancy-regex 61 KB, flate2, miniz and bincode 20 KB), `cortex-tui` +478 KB, `cortex-term` +63 KB, the binary +56 KB, the protocol +29 KB, std +47 KB, unwinding tables +102 KB, relocations +35 KB |
| `9baacb6` | +34,640 | | +1,732,096 (step 23's first part in all) | |
| `de5a686` | +64 | | | |
| `c225d64` | +456,048 | +462,848 | +603,136 | Step 23's second part: `cortex-tui` +142 KB, core +51 KB, tokio +43 KB, the binary +34 KB, the protocol +33 KB, std +26 KB, policy +22 KB, tools +10 KB, unwinding +49 KB |

Wave 11 in all: +2.00 MB on Linux (+14%), +2.34 MB on Windows.

## The home folder in the binary

A desk-built binary held the source paths of the third-party crates under the build machine's home folder: the study counted 399 registry paths; the fix's check, on a plain `cargo build --profile dist` at the desk at `61bd1a0`, found 512 paths beginning `/home/<user>` and 529 under the build's `CARGO_HOME` (the 17 more being paths packed against the string before them, which only the second search sees). CI's binaries held the runner's home folder, whose user is `runner`. The fix gives rustc a `--remap-path-prefix` for each of the build machine's folders and checks the result ([decision 0036](../decisions/0036-the-shipped-binary-is-optimised-for-size.md)); the same desk build through `cargo xtask size` held none, and was 7,960 bytes smaller (11,756,168 against 11,748,208), the paths being shorter.

## Step 33's size fix: before and after

The fix ([decision 0036](../decisions/0036-the-shipped-binary-is-optimised-for-size.md)): `opt-level = "s"` on `release`, which `dist` inherits; `idna_adapter` held at 1.0; the build machine's folders remapped out of the binary's paths. "Before" is `76e5c77` (`main` at `b08a745` and the decoding measurement); "after" is `a05c675`.

### Sizes

| | Before | After | Saved | Budget | Left |
|---|---:|---:|---:|---:|---:|
| Windows | 19,226,112 (18.34 MiB, 171 crates; run 43817284507, `main` at `b08a745`) | **13,828,096** (13.19 MiB, 150 crates) | 5,398,016 | 18 MiB | 5,046,272 |
| Linux musl | 16,258,408 (15.51 MiB, 174 crates; the same run) | **11,875,688** (11.33 MiB, 153 crates) | 4,382,720 | 16 MiB | 4,901,528 |
| Linux gnu, at the desk | 16,114,768 (the study's, `c225d64`) | **11,748,208** (153 crates) | 4,366,560 | | |

The "after" sizes are the `main` workflow's, started by hand on the fix's branch (run 43819793545). The savings are the study's two options added up (5,405,184 on Windows, 4,386,816 on musl) to within 7 KB, the base having moved from `c225d64` to `b08a745` between the study and the fix. The desk's "before" is the study's, stripped by `strip --strip-all`, so its saving is about 15 KB less exact than the others. Each binary's home-path check passed. The `dist` build itself got faster: the `main` workflow's size jobs took 4 min 17 s on Windows and 2 min 30 s on Linux, against 7 min 23 s and 4 min 9 s before (build and start-up measured together).

### Speed

Each figure is a median, then a 95th or 99th percentile where the budget is one, in milliseconds. The desk's are `rx --exclusive` runs; a runner's are one `try` run on each side, each on its own runner: perf tests 43818073883 against 43819782424, `cargo xtask perf` 43818079914 against 43819790313, start-up 43818076801 against 43819786551. Where `cargo xtask perf` and the `perf_` run measured the same thing, both are given.

| Measure | Budget | Desk, before → after | Linux runner, before → after | Windows runner, before → after |
|---|---|---|---|---|
| Start to first frame, the real binary (`perf_start_to_first_frame_under_both_names`, `cortex`, 30 runs), median / 95th | 20 / 50 / 120 | 1.89 / 2.00 → **1.55 / 1.88** | 3.45 / 3.53 → **2.94 / 3.14** | 9.92 / 21.82 → **0.00 / 23.61** (ConPTY: see below) |
| Start to first frame, the terminal demo (`cargo xtask perf`), median / 95th | as above | 0.48 / 0.55 → **0.47 / 0.54** | 1.21 / 1.31 → **0.97 / 1.10** | 25.87 / 32.12 → **24.11 / 33.56** |
| A key to its frame, idle, the real binary, median / 99th | 2 at the 99th (the harness's share under ConPTY) | 0.09 / 0.19 → **0.08 / 0.24** | 0.09 / 0.14 → **0.10 / 0.15** | 15.56 / 23.75 → **15.66 / 22.98** (ConPTY's ~16 ms) |
| The editor's key, idle, the harness's share, 99th | 2 | 0.06 → **0.05** | 0.08 → **0.08** | 0.15 → **0.12** |
| A key to its frame while streaming, the real binary, median / 99th | 4 at the 99th | 0.14 / 0.25 → **0.11 / 0.15** | 0.16 / 0.20 → **0.16 / 0.39** | 15.65 / 19.32 → **15.64 / 21.78** (ConPTY) |
| The views' frame, a 30-row live region (`perf_frames_of_the_views_…`), median / 99th | 2 at the 99th | 0.106 / 0.210 → **0.163 / 0.348** | 0.180 / 0.212 → **0.265 / 0.306**; `perf`: 0.180 / 0.206 → 0.233 / 0.351 | 0.204 / 0.260 → **0.236 / 0.275**; `perf`: 0.207 / 0.270 → 0.329 / 0.623 |
| The terminal demo's frame, 30-line live region, median / 99th | 2 at the 99th | 0.10 / 0.17 → **0.11 / 0.18** | 0.14 / 0.25 → **0.15 / 0.19** | 0.52 / 0.82 → **0.54 / 0.75** |
| Highlighting 200 lines from cold (off the UI thread) | none | 47.5 → **55.9** (+18%) | 86.5 → **100.4**; `perf`: 87.0 → 83.5 | 102.7 → **96.6**; `perf`: 101.7 → 129.6 |
| The map's warm pass on the pinned real-code corpus, nothing changed (pass and map together) | 150 | 25.9 → **28.4** (+10%) | 62.5 → **63.6** | 91.9 → **73.8** |
| The map alone, from the cache on the disk | none | 9.0 → **8.7** | 13.8 → **15.6** | 18.2 → **15.9** |
| **Decoding** a 5.7 MB stream through the SSE parser and the Anthropic decoder: 20,000 text deltas and a 1 MB tool call (`perf_decoding_a_long_answer_and_a_large_tool_call`, new) | none | 18.14 (315 MB/s) → **20.75** (275 MB/s, +14%) | 29.43 (194 MB/s) → **34.37** (166 MB/s, +17%) | 41.01 (139 MB/s) → **37.17** (154 MB/s) |
| `cortex --version`, the `dist` binary, median / 95th | 5 / 15 / 50 | 0.53 / 0.60 → **0.48 / 0.59** | 1.01 / 1.09 → **1.24 / 1.40** | 6.27 / 6.85 → **4.82 / 5.09** |
| Headless `cortex -p` to its request, the `dist` binary, median / 95th | 15 / 40 / 100 | 12.53 / 17.27 → **12.86 / 13.67** | 7.85 / 8.72 → **10.23 / 12.13** | 18.87 / 19.72 → **14.59 / 16.21** |

What the table shows:

- **Every figure is inside its budget, by a wide margin except one that was already close.** The desk's headless run is 12.86 ms against 15 ms, as it was before the fix (12.53): its time is the session store's (with `--no-session` it is 3.25 ms), not the code's optimisation level.
- **What "s" slows is the CPU-bound work:** the views' frame (+50% at the desk, as the study found, and still a fifth of its budget at the 99th everywhere), highlighting from cold (+15 to 20%), JSON decoding (+14 to 17%; 275 MB/s is still about 3 µs for a typical 1 KB event) and the map (+10%).
- **Start-up is no slower, and the first frame is faster** at the desk and on Linux: a smaller binary is less to map and fault in.
- **Runner against runner,** the Windows figures moved both ways by more than "s" could explain (the map 20% faster, the views 20% to 60% slower in two runs of the same build): Windows runners differ by up to a factor of two, and one run a side is a bound, not a ratio. The desk's is the precise comparison.
- **The real binary's first frame on Windows is not measurable this way:** under ConPTY the shell's start line and the first frame often arrive in one read, so the time between them reads 0.00 ms. The terminal demo's first frame (24 ms against 120) and the start-up rows are Windows' figures of record.
- **A key under ConPTY** takes about 16 ms to come through it ([conpty.md](../conpty.md)); the budget applies to the harness's share, 0.12 ms.

### Which profile the measurements use

The perf tools build in `release` (`cargo xtask perf`, the `perf_` tests, `try -f profile=release`), and start-up in `dist`. Since the fix `release` has `opt-level = "s"` too, so every figure of record runs code optimised as the shipped binary's is; `release` keeps thin LTO over 16 codegen units, which builds in a fraction of `dist`'s time, and nothing builds at `opt-level = 3`.

## The outlook (the study)

At `opt-level = 3` with unwinding, steps 24 to 34 were expected to add about 4.5 to 6 MB on Linux (to 20.5 to 22 MB) and about 1.3 times that on Windows (25 to 27 MB): steps 24 and 25 about 0.45 MB each, 27 0.3 to 0.5, 28 (QuickJS and `cortex-ext`) 1.3 to 1.6, 29 0.2 to 0.3, 30 (`rmcp` and `cortex-mcp`) 1.3 to 2.5, 32 to 34 0.2. At "s", our own code shrinks by about a quarter: step 34 near 14.5 to 16.3 MB on Linux and 17 to 19 MB on Windows. `panic = "abort"` stays in reserve for Windows.
