# 0015: Syntax highlighting with syntect, loaded lazily, behind a feature

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

Code blocks and diffs read better highlighted. Highlighting is the most expensive optional part of the UI: grammars are large, and loading them takes time. It must never delay start-up or a frame.

## Options considered

| Option | For | Against |
|---|---|---|
| **No highlighting** | Free. | A visibly poorer UI for a tool whose output is mostly code. |
| **syntect** with its pure-Rust regex engine and a trimmed, precompiled grammar set | Broad language coverage from one dependency. Pure Rust. Line-oriented, which suits streaming. | Size, and a load cost for the grammar set. Regex grammars are approximate and can be slow on pathological lines. |
| **Parser-based highlighting** (a grammar per language) | Accurate. Could share parsers with the repository outline. | Each language is a C grammar: build time and roughly a megabyte each. Rejected for the outline for the same reason ([0022](0022-repository-map.md)). |
| **A small hand-written highlighter** for strings, comments, numbers and keywords | Tiny, fast, no dependency. Good enough for diffs. | A table per language to maintain, and worse results on anything unusual. |

## Measurement

Desktop pinned to 4 cores, syntect with default features off, the pure-Rust regex engine and the default packed grammar set ([measurements/dependency-costs.md](../measurements/dependency-costs.md)):

| | Dev build | Release build | Size added | Crates |
|---|---|---|---|---|
| syntect | 5.2 s | 11.4 s | 2.3 MB | 45 |

**Load and first use** (three runs). Loading the packed default grammar and theme sets takes 0.6 to 0.7 ms, because the sets are unpacked lazily. The cost moves to first use: highlighting 200 short Rust lines from cold took 27 ms in total, which includes compiling the grammar's patterns the first time they are needed. The two were not separated. 27 ms is more than one frame, which is why highlighting never runs on the UI thread.

## Decision

- syntect, default features off, pure-Rust regex engine, behind the `highlight` feature of `cortex-tui` (on by default).
- A trimmed grammar and theme set packed at build time by `xtask`, covering the languages the outline extractors cover plus common configuration and markup formats.
- Loaded on first use, on the blocking pool, never on the UI thread. Until it is ready, code is shown plain. Highlighting applies to lines committed after that; lines already in the scrollback are not rewritten.
- A per-line time limit. A line that exceeds it is shown plain.
- Colours come from the theme's small set of syntax roles, not from an imported editor theme, so highlighting follows the terminal's light or dark background and the 16-colour fallback.

## Amended in step 22

Built in `cortex_tui::highlight`, with these differences from the decision above, each for the reason given:

- **A worker thread of its own, not the blocking pool.** `cortex-tui` has no async runtime: the interface runs on its own thread, and a dedicated worker, started when the first code block asks for it, keeps each block's parse state between lines, which a pool's tasks would have to hand back and forth. The interface hands it lines and takes what it has finished; it never waits for it.
- **The per-line limit is a wait on the interface's side.** syntect cannot be stopped in the middle of a line, so the limit is put where it matters: a completed line waits for its colours for at most 50 ms on the caller's clock, and is committed plain after that. A line over 1,000 bytes is never given to the worker, and the regular-expression engine bounds each pattern's backtracking. A worker stuck on a line costs that line and the ones queued behind it their colours, never a frame.
- **Lines wait while the grammars load,** under the same limit. "Until it is ready, code is shown plain" holds for the live region, where a waiting line is drawn plain; a line commits in colour if the grammars are ready within its wait, so a session's first code block is coloured too. Lines already committed are never rewritten.
- **The default packed grammar set, not a trimmed one.** syntect's own packed set (its `default-syntaxes` feature) covers the outline extractors' languages and more, and costs 628 KB of the stripped `dist` binary: 13.2 MB with the feature and 12.6 MB without, against the 16 MB budget. Trimming and packing a set with `xtask` would need the grammar sources in the repository and saves a fraction of that, so it waits for the size budget to ask for it (as "Revisit when" says). The code line still streaming is drawn plain until it ends ([outstanding/highlighting-the-line-in-progress.md](../outstanding/highlighting-the-line-in-progress.md)).
- **The roles** are the theme's five syntax tokens (keyword, type, string, number, comment), picked by the grammar's scope names; on the reference session they colour exactly what the design round's screens did.

## Why

- It is the only option that gives broad coverage without C code.
- Lazy loading and the feature flag keep its cost off start-up and make it removable where size matters more.
- A hand-written highlighter was close, and remains the fallback if the size budget tightens.

## Revisit when

- The binary size budget is missed: drop to the hand-written highlighter or trim the grammar set further.
- The repository outline moves to real parsers, at which point sharing them becomes attractive.
