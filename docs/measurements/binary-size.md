# The shipped binary's size

What the shipped binary (`cineris`, the `dist` profile) is made of, what each way of making it smaller saves and costs, what step 33's size fix did ([decision 0036](../decisions/0036-the-shipped-binary-is-optimised-for-size.md)), and what the size step cut, measured and projected to step 34 (2026-11-19). Measured on 2026-11-16: the size study (read-only, at `main` `c225d64`, after step 23), then the fix, before and after, on the reference machine and on both hosted runners.

## Method

- **Windows and Linux musl sizes** are the `main` workflow's: `cargo xtask size --check --target TARGET` on its hosted runners, `--locked`, the `dist` profile, stripped (`strip = true`). A branch's sizes come from the same workflow started by hand on it (`gh workflow run main --ref BRANCH`), whose `report` job reports nothing off the main branch. They are what the budgets in [performance-budgets.md](../performance-budgets.md) ("Binary size") are checked against.
- **The reference machine's sizes** are of the gnu target (`x86_64-unknown-linux-gnu`), built in the `dist` profile on the reference machine. The study built each variant unstripped and then ran `strip --strip-all`, which leaves 14,776 bytes less than rustc's own strip: compare the study's reference-machine figures only with each other. The fix's reference-machine figures are `cargo xtask size`'s, stripped by rustc, as the runners'.
- **What the binary is made of** was read from an unstripped build on the reference machine: the size of each symbol, put to its crate by its mangled path, and the read-only data by what it holds.
- **Speed** was measured in `release`, which the perf tools build (`cargo xtask perf`, the `perf_` tests, `try -f profile=release`), and start-up on the `dist` binary itself (`cargo xtask startup --profile dist`, 30 runs). The study measured on the reference machine, three rounds a variant against `opt-level = 3`. The fix measured before and after, on the reference machine, otherwise idle, and on one hosted runner of each system through `try`, each side on a runner of its own: Windows runners differ in speed by up to a factor of two, so a hosted before-and-after is a bound, not a precise ratio.

## Where it stood (the study, `c225d64`)

| | Bytes | Against the budget |
|---|---:|---|
| Windows (`x86_64-pc-windows-msvc`) | 19,226,112 (18.34 MiB) | **351,744 over** 18 MiB; `size-windows` failed on run 43811142390 and opened issue #61 |
| Linux musl (`x86_64-unknown-linux-musl`) | 16,258,408 (15.51 MiB) | 518,808 under 16 MiB |
| Linux gnu, on the reference machine | 16,114,768 | |

## The options, largest Windows saving first

Bytes saved on Windows, Linux musl and the reference machine's gnu build.

| # | Option | Windows | Linux musl | Reference gnu | Notes |
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

Release build, reference machine, three rounds, against `opt-level = 3`:

| Measure | `opt-level = 3` | `"s"` | Budget |
|---|---|---|---|
| The views' frame, a 30-row live region | 0.095 to 0.10 ms | 0.145 to 0.149 ms (+50%; 99th 0.22 to 0.26 ms) | 2 ms at the 99th |
| Highlighting 200 lines from cold | 42 to 46 ms | 56 to 59 ms (+30%) | off the UI thread |
| The map alone | 5.8 to 6.5 ms | 6.6 to 7.1 ms (+10%) | |
| Start to first frame | 1.64 to 1.89 ms | 1.49 to 1.60 ms (faster) | 20 ms |
| The reference machine's build of `dist` | 93 s | 67 s | |

### The cost of unwinding (a correction to decision 0012)

[Decision 0012](../decisions/0012-release-profile.md) chose `panic = "unwind"` on a measured cost of 0.17 to 0.27 MB (2.5 to 3.7%), on a spike with the core dependency set. On the real binary it costs **2.2 MB on Linux** (2,166,784 bytes on musl, 2,164,720 on the reference machine's gnu build) and **6.5 MB on Windows** (6,457,856 bytes, a third of the binary): 8 to 25 times the spike's figure. The owner kept unwinding, and left decision 0012 as it is; this page holds the correction. What abort would cost: every `catch_unwind` in shipped code (the agent's tool boundary in `cineris-agent`'s `guarded.rs`, the interface's two catches, the repository map's three, `cineris-tools`' `blocking.rs` and process capture, `cineris-term` on Unix) would stop catching, and a panic in a Tokio task or in the highlighter's thread would end the process.

## What it is made of (`c225d64`, reference gnu, 16.1 MB)

- **Code, 12.21 MB (76%):** async state machines 1.12 MB; serde's derives 1.07 MB (`Deserialize` 886 KB, `Serialize` 179 KB); `Clone`, `Eq` and `Hash` 403 KB; `Debug` 331 KB; panic paths 141 KB; `tracing` 118 KB.
- **Unwinding tables, 1.49 MB (9%).**
- **Read-only data, 1.60 MB (10%):** syntect's grammars 368 KB; ring's tables about 180 KB; `encoding_rs`'s CJK tables 118 KB; panic-location source paths 92 KB (66 KB of them the registry's and the standard library's); other text 323 KB.
- **Position-independent relocations and pointer tables, 0.79 MB.**
- **By owner:** our crates 6.65 MB; third-party 5.1 MB; the standard library 0.79 MB.
- **The largest crates:** `cineris-tools` 793 KB, `cineris-core` 715 KB, `cineris-tui` 643 KB, `cineris-protocol` 633 KB, `tokio` 629 KB, `cineris-llm` 563 KB, `cineris-session` 538 KB, `cineris-agent` 525 KB, `ring` about 520 KB, `cineris-stats` 487 KB, `cineris-config` 479 KB, `regex-automata` 470 KB, `rustls` 446 KB, `serde_json` 384 KB.

## Growth by commit

Reference gnu, Linux musl and Windows bytes, each against the commit before.

| Commit | Reference gnu | Linux musl | Windows | What |
|---|---:|---:|---:|---|
| `d050503` | 14,111,096 | 14,246,888 | 16,882,688 | The base |
| `bd2e6a1` | +9,968 | 0 | +8,192 | |
| `e1ac07a` | +63,952 | | | Step 23's first part, before the binary was wired: configuration keys (`cineris-config` +30 KB, `toml` +13 KB) |
| `a1f19cf` | +1,439,000 | | | "Start interactive mode in the binary": highlighting about +510 KB (grammars 368,467, syntect 62 KB, fancy-regex 61 KB, flate2, miniz and bincode 20 KB), `cineris-tui` +478 KB, `cineris-term` +63 KB, the binary +56 KB, the protocol +29 KB, std +47 KB, unwinding tables +102 KB, relocations +35 KB |
| `9baacb6` | +34,640 | | +1,732,096 (step 23's first part in all) | |
| `de5a686` | +64 | | | |
| `c225d64` | +456,048 | +462,848 | +603,136 | Step 23's second part: `cineris-tui` +142 KB, core +51 KB, tokio +43 KB, the binary +34 KB, the protocol +33 KB, std +26 KB, policy +22 KB, tools +10 KB, unwinding +49 KB |

Wave 11 in all: +2.00 MB on Linux (+14%), +2.34 MB on Windows.

## The home folder in the binary

A binary built on the reference machine held the source paths of the third-party crates under the build machine's home folder: the study counted 399 registry paths; the fix's check, on a plain `cargo build --profile dist` on the reference machine at `61bd1a0`, found 512 paths beginning `/home/<user>` and 529 under the build's `CARGO_HOME` (the 17 more being paths packed against the string before them, which only the second search sees). CI's binaries held the runner's home folder, whose user is `runner`. The fix gives rustc a `--remap-path-prefix` for each of the build machine's folders and checks the result ([decision 0036](../decisions/0036-the-shipped-binary-is-optimised-for-size.md)); the same build through `cargo xtask size` held none, and was 7,960 bytes smaller (11,756,168 against 11,748,208), the paths being shorter.

## Step 33's size fix: before and after

The fix ([decision 0036](../decisions/0036-the-shipped-binary-is-optimised-for-size.md)): `opt-level = "s"` on `release`, which `dist` inherits; `idna_adapter` held at 1.0; the build machine's folders remapped out of the binary's paths. "Before" is `76e5c77` (`main` at `b08a745` and the decoding measurement); "after" is `a05c675`.

### Sizes

| | Before | After | Saved | Budget | Left |
|---|---:|---:|---:|---:|---:|
| Windows | 19,226,112 (18.34 MiB, 171 crates; run 43817284507, `main` at `b08a745`) | **13,828,096** (13.19 MiB, 150 crates) | 5,398,016 | 18 MiB | 5,046,272 |
| Linux musl | 16,258,408 (15.51 MiB, 174 crates; the same run) | **11,875,688** (11.33 MiB, 153 crates) | 4,382,720 | 16 MiB | 4,901,528 |
| Linux gnu, on the reference machine | 16,114,768 (the study's, `c225d64`) | **11,748,208** (153 crates) | 4,366,560 | | |

The "after" sizes are the `main` workflow's, started by hand on the fix's branch (run 43819793545). The savings are the study's two options added up (5,405,184 on Windows, 4,386,816 on musl) to within 7 KB, the base having moved from `c225d64` to `b08a745` between the study and the fix. The reference machine's "before" is the study's, stripped by `strip --strip-all`, so its saving is about 15 KB less exact than the others. Each binary's home-path check passed. The `dist` build itself got faster: the `main` workflow's size jobs took 4 min 17 s on Windows and 2 min 30 s on Linux, against 7 min 23 s and 4 min 9 s before (build and start-up measured together).

### Speed

Each figure is a median, then a 95th or 99th percentile where the budget is one, in milliseconds. The reference machine's are runs on an otherwise idle machine; a runner's are one `try` run on each side, each on its own runner: perf tests 43818073883 against 43819782424, `cargo xtask perf` 43818079914 against 43819790313, start-up 43818076801 against 43819786551. Where `cargo xtask perf` and the `perf_` run measured the same thing, both are given.

| Measure | Budget | Reference machine, before → after | Linux runner, before → after | Windows runner, before → after |
|---|---|---|---|---|
| Start to first frame, the real binary (`perf_start_to_first_frame_under_both_names`, `cineris`, 30 runs), median / 95th | 20 / 50 / 120 | 1.89 / 2.00 → **1.55 / 1.88** | 3.45 / 3.53 → **2.94 / 3.14** | 9.92 / 21.82 → **0.00 / 23.61** (ConPTY: see below) |
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
| `cineris --version`, the `dist` binary, median / 95th | 5 / 15 / 50 | 0.53 / 0.60 → **0.48 / 0.59** | 1.01 / 1.09 → **1.24 / 1.40** | 6.27 / 6.85 → **4.82 / 5.09** |
| Headless `cineris -p` to its request, the `dist` binary, median / 95th | 15 / 40 / 100 | 12.53 / 17.27 → **12.86 / 13.67** | 7.85 / 8.72 → **10.23 / 12.13** | 18.87 / 19.72 → **14.59 / 16.21** |

What the table shows:

- **Every figure is inside its budget, by a wide margin except one that was already close.** The reference machine's headless run is 12.86 ms against 15 ms, as it was before the fix (12.53): its time is the session store's (with `--no-session` it is 3.25 ms), not the code's optimisation level.
- **What "s" slows is the CPU-bound work:** the views' frame (+50% on the reference machine, as the study found, and still a fifth of its budget at the 99th everywhere), highlighting from cold (+15 to 20%), JSON decoding (+14 to 17%; 275 MB/s is still about 3 µs for a typical 1 KB event) and the map (+10%).
- **Start-up is no slower, and the first frame is faster** on the reference machine and on Linux: a smaller binary is less to map and fault in.
- **Runner against runner,** the Windows figures moved both ways by more than "s" could explain (the map 20% faster, the views 20% to 60% slower in two runs of the same build): Windows runners differ by up to a factor of two, and one run a side is a bound, not a ratio. The reference machine's is the precise comparison.
- **The real binary's first frame on Windows is not measurable this way:** under ConPTY the shell's start line and the first frame often arrive in one read, so the time between them reads 0.00 ms. The terminal demo's first frame (24 ms against 120) and the start-up rows are Windows' figures of record.
- **A key under ConPTY** takes about 16 ms to come through it ([conpty.md](../conpty.md)); the budget applies to the harness's share, 0.12 ms.

### Which profile the measurements use

The perf tools build in `release` (`cargo xtask perf`, the `perf_` tests, `try -f profile=release`), and start-up in `dist`. Since the fix `release` has `opt-level = "s"` too, so every figure of record runs code optimised as the shipped binary's is; `release` keeps thin LTO over 16 codegen units, which builds in a fraction of `dist`'s time, and nothing builds at `opt-level = 3`.

## The outlook (the study)

At `opt-level = 3` with unwinding, steps 24 to 34 were expected to add about 4.5 to 6 MB on Linux (to 20.5 to 22 MB) and about 1.3 times that on Windows (25 to 27 MB): steps 24 and 25 about 0.45 MB each, 27 0.3 to 0.5, 28 (QuickJS and `cineris-ext`) 1.3 to 1.6, 29 0.2 to 0.3, 30 (`rmcp` and `cineris-mcp`) 1.3 to 2.5, 32 to 34 0.2. At "s", our own code shrinks by about a quarter: step 34 near 14.5 to 16.3 MB on Linux and 17 to 19 MB on Windows. `panic = "abort"` stays in reserve for Windows.

## The size step (2026-11-19)

A step outside the numbered plan, to make room for step 28 ([steps/size-step/answered.md](../steps/size-step/answered.md)). After step 25 the Windows binary was 96.5% of its budget, and step 28 adds a script engine. The step made the cuts that leave behaviour as it is, measured those that cost speed or change behaviour, projected the size to step 34, and proposed budgets for the owner to decide.

### Method

- **Sizes** are `cargo xtask size` on the hosted runners: the `main` workflow's for `main`, and `try` runs (`-f command=size`) for each commit of the step and for each option, built on a branch of its own that is never merged. The reference machine's musl build, the same code through the same command, differed from the runner's by 8 bytes.
- **What the binary is made of** was read from the symbols of an unstripped build (`size --breakdown`, and `nm` on its binary). Generic code was put to the crate that instantiated it by the symbol's mangled name (Rust's v0 mangling ends with that crate), so that one function built in two crates shows as two.
- **Speed** was measured with `cargo xtask perf-ab` (the `perf_` tests, `release`, five rounds alternating, on the reference machine, otherwise idle), and with the hand-run measurements of the redaction filter and the permission engine, in each tree in turn.

### Where it stood

| | Bytes | Of the budget | Left |
|---|---:|---:|---:|
| Windows | 18,210,816 (17.37 MiB, 150 crates) | 96.5% of 18 MiB | 663,552 |
| Linux musl | 15,006,120 (14.31 MiB, 153 crates) | 89.4% of 16 MiB | 1,771,096 |

`main` at `6806291f`, run 44466604331: step 25's final head.

### What grew since step 33's size fix

From 13,828,096 bytes on Windows and 11,875,688 on Linux (`a05c675`), by the `main` workflow's record of each push. The steps' merges interleave, so a range can hold more than one.

| What merged | Linux musl | Windows | Windows ÷ Linux |
|---|---:|---:|---:|
| Step 17's project knowledge wired into a session, and its fixes | +426,048 | +559,104 | 1.31 |
| Step 23's review fixes and follow-ups | +180,224 | +253,440 | 1.41 |
| Step 19's verification | +94,208 | +138,240 | 1.47 |
| Step 24 (reload, panels, live views) | +594,048 | +797,696 | 1.34 |
| Steps 26, 27 and 31 (the sandbox, the RPC server and `attach`, the OpenAI-compatible protocol) | +1,245,984 | +1,847,808 | 1.48 |
| Step 25 (full screen, rewind, the first run) | +557,152 | +760,320 | 1.36 |
| Small changes between them | +32,768 | +26,112 | |
| **In all** | **+3,130,432** | **+4,382,720** | **1.40** |

Two things follow for the outlook. **A large step costs 0.4 to 0.6 MB on Linux at "s"**, about twice what the study expected of steps 24 and 25 (0.45 MB each at `opt-level = 3`, a quarter less at "s"). **New code costs 1.4 times as much on Windows as on Linux**, measured over 4.4 MB of it, not the 1.15 to 1.2 of the whole binary: much of what our code adds is the unwinding Windows records for every function.

### The cuts made

Each is its own commit, named by its title; the bytes are each commit's difference from the one before, on the runners.

| Cut | Linux musl | Windows | Speed |
|---|---:|---:|---|
| **The core's handler built once.** The future of an `async fn` that only other crates await is built in each crate that awaits it, and `Core::handle`, with all it calls, was built twice: for the binary's own front ends and for `cineris-rpc`'s server. Its future (and `disconnect`'s and `shutdown`'s) is now boxed inside `cineris-core`. | −266,240 | −433,152 | A request's round trip through the server, +1.2 to 1.5%, within noise |
| **The configuration's TOML tables collected through one helper.** A `BTreeMap`'s `FromIterator` sorts with a closure that names the iterator's type, so each kind of iterator collected into a table built the standard library's stable sort again: twelve of them in `cineris-config`. | −53,248 | −2,560 | None measurable: no hot path |
| **Blocking work handed to the pool as one type.** The runtime builds its task code for each type of work given to `spawn_blocking`, and `run_blocking` passed each caller's closure through: 53 copies. The work is now boxed, with its result boxed as `Any`, through one function that is not generic. | −180,224 | −296,960 | The hop to the blocking pool, 21.24 µs against 21.25 |
| **In all** | **−499,712** | **−732,672** | |

After them: **Windows 17,478,144 bytes** (16.67 MiB, 1,396,224 left) and **Linux musl 14,506,408** (13.83 MiB, 2,270,808 left).

**Windows already folds identical functions** (rustc asks MSVC's linker for `/OPT:ICF` in an optimised build), which is why the second cut saved almost nothing there: the twelve sorts were the same code, and Windows had kept one. The other two cuts removed code that differed, and saved 1.6 times as much on Windows as on Linux.

**The speed of the three, together,** against `main` (`perf-ab`, the `perf_` tests of `cineris-rpc`, `cineris-core` and `cineris-tools`): every figure within noise, the request's round trip, search, diffing and the blocking hop among them, except one: choosing a round's checks from a profile, 6.24 µs against 5.77 (+8%), whose ranges were apart. Ten rounds of that test alone gave the same, 6.23 µs against 5.74 (+0.48 µs). What it times is two `stat` calls and a pure choice over the profile, in `cineris-core`'s verifier and `cineris-knowledge`, and none of the cuts changes a function it runs: it is taken for where the build placed the code (`release` links over 16 units with thin LTO, and changing a crate moves its neighbours). It is 0.48 µs of a 50 µs budget.

### Found and not made

Ranked by bytes against risk. Each figure is Linux musl; Windows is about the same or more, except where its folding already removes the duplicate.

| What | Linux musl | Risk, and why it was not made |
|---|---:|---|
| The statistics' monthly line: `#[serde(flatten)]` over an internally tagged enum, so every line is buffered and read twice over (`FlatMapDeserializer`, `ContentDeserializer`) | about 50 KB | Medium: a reader written by hand must read every line on disk, damaged ones included, as the derive does |
| The model catalog read through one deserializer: the built-in catalog is JSON and a user's entries are TOML, so every catalog type is built for both | about 45 KB | A design change: `cineris-llm` would need `toml` at run time (decision 0017 embeds JSON so that it does not), or a user's error messages would change |
| The other 15 kinds of blocking work (the core's own `spawn_blocking` calls and the session log's) | 20 to 30 KB, a guess | Low, but each site handles a failed task its own way |
| The task code of each spawned future (36 kinds, about 650 bytes each beyond the future's own code) | about 25 KB | Low; a box per spawn |
| The standard library's code built again in each crate that uses it (sorts of `String` and `&str`, an `Event`'s drop, an `Option`'s clone): about 100 KB in pieces of 1 to 6 KB | up to 100 KB, in pieces | Low each, but each piece is small; Windows folds most of them already |
| The setup worker's two channels as one | about 2 KB | Low; too small to be worth it |
| **Identical-code folding on Linux** (`rust-lld` with `--icf=all`) | 102,944 against `rust-lld` without folding (94,760 against today's link, whose binary `rust-lld` makes 8,184 bytes larger) | **Not recommended.** `--icf=safe` saves nothing (rustc writes no table of which addresses are taken); `all` would give `cineris-term`'s two empty signal handlers one address, and the interrupt guard tells its handler by its address (a comment there says so now); it needs `rust-lld` in the shipped build and the release workflow; and Windows gains nothing. |

**Where the returns diminish:** after the three cuts the largest neutral item left is about 50 KB, with a risk to how files on disk are read, and the rest are 30 KB or less each. The three made were 266, 180 and 53 KB on Linux.

### What costs speed: "z" for ten crates

`opt-level = "z"` for ten library crates of ours that no hot path runs through (`cineris-agent`, `-base`, `-config`, `-core`, `-knowledge`, `-policy`, `-rpc`, `-sandbox`, `-session`, `-stats`), set on `release` so that the perf tools measure it: **Linux −315,392, Windows −503,808.** It is the step's last commit but one, and can be dropped.

- **The binary's own crate must stay at "s":** `cineris-cli` alone at "z" made the binary **3.5 MB larger** (18,031,472 against 14,506,416 on the reference machine): its level is what the fat-LTO link is made at.
- `cineris-config` alone at "z" saved 8,192 bytes; the cold third-party crates (TOML, TLS, HTTP) added to the ten saved 69,600 more.

Its speed, against the tree without it:

| Measure (release build, medians of each side's rounds) | Without | With "z" | Change |
|---|---:|---:|---:|
| Choosing a round's checks from a typical profile (`perf_` test, `cineris-core`, `cineris-knowledge`) | 6.20 µs | 11.38 µs | **+84%** |
| Listing 1,000 sessions (`cineris-session`) | 0.79 ms | 1.09 ms | **+37%** |
| Opening a session of 10,000 entries (12.1 MB) | 11.1 ms | 15.0 ms | **+36%** |
| The same, reading the active branch's items | 32.3 ms | 35.4 ms | +9% |
| The session log: one entry appended, no flush | 2.38 µs | 2.54 µs | +7% |
| The session log: one entry appended with its flush | 4.39 ms | 4.35 ms | none: the flush is the disk's |
| The project's knowledge: one change, written and flushed (`cineris-knowledge`) | 9.53 ms | 8.81 ms | none: the disk's |
| The project's knowledge: every entry rendered, a typical store | 44.3 µs | 35.9 µs | 19% less time |
| A year's usage report, the 99th percentile, aggregates kept (`cineris-stats`) | 7.71 ms | 8.96 ms | **+16%** |
| A permission decision, `cargo test --workspace` against an allow rule (`cineris-policy`, by hand) | 8.11 µs | 11.28 µs | **+39%** |
| A permission decision, a line of five commands | 14.50 µs | 19.99 µs | **+38%** |
| Making the permission engine (20 rules) | 18.75 µs | 25.22 µs | **+35%** |
| The redaction filter on a 64 MiB build log (`cineris-base`, by hand) | 469 MiB/s | 541 MiB/s | 15% more throughput |
| A request's round trip through the server (`cineris-rpc`) | 153.9 µs | 153.6 µs | within noise |
| The loop's verification stage when no check runs (`cineris-agent`) | 0.47 µs | 0.57 µs | within noise |
| Start-up of the `dist` binary (`cargo xtask startup --profile dist`): `cineris --version`, median / 95th | 0.53 / 0.66 ms | 0.53 / 0.57 ms | none |
| The same: headless `cineris -p` to its request, median / 95th | 13.99 / 14.77 ms | 13.84 / 14.71 ms | none |

Five rounds of each `perf_` test and three of each hand-run measurement, the two trees in turn, on the reference machine, otherwise idle. **The session log's, the knowledge store's and the usage report's rows were measured again**, with both trees' tests writing to one directory (`CINERIS_TEST_TMPDIR`): the first run let each tree's tests write under its own target directory, on different storage, and showed an append with its flush 85% slower, which was the disk and not the code (the review of the step found it). Every figure is still inside its budget on the reference machine, but several of these paths are the ones with little room elsewhere: a year's usage report is already over its 16 ms on a Windows runner ([outstanding/the-usage-panels-year-on-windows.md](../outstanding/the-usage-panels-year-on-windows.md)), and choosing the checks is most of the verification stage's 50 µs on Windows (34.6 µs there before "z").

**The saving comes from the crates that slow down.** "z" for the five of the ten whose paths did not slow (`cineris-agent`, `-base`, `-config`, `-rpc`, `-sandbox`) saves 32,768 bytes on Linux; the other 282,624 are `cineris-core`, `-knowledge`, `-policy`, `-session` and `-stats`.

**Not recommended:** it saves 0.3 MB on Linux and 0.5 MB on Windows at a cost of a third to five-sixths on listing and opening sessions, the permission engine and the verifier's choice of checks, each run on every turn or every start. Whole-profile "z" takes the same costs and the study's on rendering and highlighting with it.

### What changes behaviour: the owner's options

Measured on the runners against the cuts above, on branches that are never merged.

| Option | Linux musl | Windows | What is given up |
|---|---:|---:|---|
| **`panic = "abort"`** | **−2,039,808** | **−6,384,640** | A panic anywhere ends the process: a tool's, a script's or an MCP server's data that trips a bug ends the session ([decision 0012](../decisions/0012-release-profile.md)) |
| **`opt-level = "z"` for the whole profile** | −1,142,752 | −1,724,416 | The study's speed costs against "s": the views' frame, highlighting and the map slower again (above) |
| `reqwest` without HTTP/2 | −465,216 (the study's) | −553,472 (the study's) | HTTP/2 to the providers ([decision 0004](../decisions/0004-http-and-tls.md)) |
| Highlighting off; a trimmed grammar set | −626,784; −183,227 (the study's) | −647,680; −183,227 | Highlighting ([decision 0015](../decisions/0015-syntax-highlighting.md)); the languages left out |
| Debug-level logging compiled out (`release_max_level_info`) | about −105,000 (the study's, gnu) | | The log's `debug` lines in a shipped build |
| Search without the byte-order-mark sniffing (`encoding_rs`'s tables, 142,395 bytes of symbols) | up to −142,000, a guess | | Searching files in UTF-16 |
| A position-dependent Linux binary | about −440,000 (the study's, gnu) | | Address-space randomisation. Not recommended |
| **Step 28 in Lua, the fallback, in place of QuickJS** | about −0.5 MB of step 28, a guess from the recorded 1,165 KB against 427 KB | | JavaScript as the extension language ([decision 0005](../decisions/0005-extension-mechanism.md)) |
| **Step 30 with an MCP client of our own in place of `rmcp`** | about −0.8 to −1.5 MB of step 30, a guess | | The SDK's tracking of both protocol eras ([architecture/13-mcp.md](../architecture/13-mcp.md)) |

### The engine step 28 adds, measured

QuickJS through `rquickjs` 0.14, linked into today's binary with a full context (every built-in object) and one script evaluated, on a branch that is never merged: **Linux +823,600, Windows +762,880.** Smaller than the 1,165 KB the dependency survey measured at `opt-level = 3`, and smaller on Windows than on Linux: it is C, and has none of the unwinding Rust code records. The host functions and the bindings that step 28 writes are not in it.

### The outlook to step 34

Decimal megabytes added by each step, from the build plan's scope of the step and the costs recorded; each range's low end is the smaller scope and its high end the larger. Our own code is priced from the steps above (0.4 to 0.6 MB on Linux for a large step, at 1.4 times that on Windows).

| Step | Its scope ([build-plan.md](../build-plan.md)) | Linux musl | Windows | Basis |
|---|---|---:|---:|---|
| 28, Extensions I | "Step 28": `cineris-ext`, the script runtime and its host functions, hooks, manifests, the management commands. Size L | +1.3 to +1.8 | +1.45 to +2.0 | QuickJS measured (+0.82, +0.76); the crate a guess from steps 24 to 27 |
| 29, Extensions II | "Step 29": `propose_extension`, its checks, the review dialog. Size M to L | +0.25 to +0.5 | +0.35 to +0.7 | A guess from the steps above |
| 30, MCP client | "Step 30": `cineris-mcp` on `rmcp`, both eras, local and remote servers. Size L | +1.4 to +2.6 | +2.0 to +3.6 | `rmcp` recorded at 1.76 MB (child processes) to 3.59 MB (with HTTP) at `opt-level = 3`, less what the binary already links; a guess |
| 32, Hardening pass | "Step 32": tests, and fixes of what they find. "Must not change: features." Size M | 0 to +0.15 | 0 to +0.2 | A guess |
| 33, Performance pass | "Step 33": measurement and fixes. "Must not change: behaviour." Size M | −0.1 to +0.15 | −0.15 to +0.2 | A guess |
| 34, Release engineering | "Step 34": `cineris alias install` and `remove`, the release workflow. Size M to L | +0.05 to +0.2 | +0.07 to +0.3 | A guess |
| Fixes between the steps | | +0.05 to +0.2 | +0.07 to +0.3 | As the fixes since step 23 |
| **28 to 34 in all** | | **+2.95 to +5.6** | **+3.8 to +7.3** | |

**At step 34, with the three cuts and nothing else:** Linux 17.5 to 20.1 MB (16.6 to 19.2 MiB), against 16 MiB; Windows 21.3 to 24.8 MB (20.3 to 23.6 MiB), against 18 MiB. **Both are over their budgets at the low end of the range:** Linux by 0.7 MB, Windows by 2.4 MB. Windows is over from step 28 on, at any point of the range. The size step's record has the proposal of budgets and reserves that follows from it ([steps/size-step/answered.md](../steps/size-step/answered.md)).
