# Answered in step 33, on 2026-11-16

## The size fix, an early part of the performance pass

The Windows `dist` binary was 19,226,112 bytes, over its 18 MiB budget, so the `main` workflow's `size-windows` failed on every push (run 43811142390, issue #61), and Linux had about one step's room left. A read-only size study measured what the binary holds and what each option saves ([measurements/binary-size.md](../../measurements/binary-size.md)). The owner chose from it through the coordinator, and this fix, ahead of the rest of step 33, carried it out ([decision 0036](../../decisions/0036-the-shipped-binary-is-optimised-for-size.md)).

| Question | The owner's answer, 2026-11-16 | Recorded in |
|---|---|---|
| **How to bring the binary back under its budget,** with room for steps 24 to 34. | **The study's plan B:** `opt-level = "s"` for the shipped profile, and `idna_adapter` held at 1.0. Panic unwinding stays on (it keeps a panicking tool or background task from ending the process); highlighting and HTTP/2 stay in. `panic = "abort"` is the reserve if Windows nears its budget again. | [decision 0036](../../decisions/0036-the-shipped-binary-is-optimised-for-size.md) |
| **Decision 0012's figure for unwinding** (0.17 to 0.27 MB on a spike; 2.2 MB on Linux and 6.5 MB on Windows on the real binary). | **0012 is left as it is.** The measurement page holds the correction. | [measurements/binary-size.md](../../measurements/binary-size.md), "The cost of unwinding" |
| **The build machine's home folder in a desk-built binary** (rule 3). | **No binary may carry a home path, wherever it is built.** Remap it out, and check for it. | [decision 0036](../../decisions/0036-the-shipped-binary-is-optimised-for-size.md) |
| **A pull request never sees its own size.** | **A CI job reports each pull request's size** against its budget and against `main`, before it merges; not a required check, and never delaying the required ones. | [testing-and-ci.md](../../testing-and-ci.md), "Each pull request's binary size" |

### What this fix decided within those answers

- **`release` is at "s" too**, and `dist` inherits it. The perf tools (`cargo xtask perf`, the `perf_` tests, `try -f profile=release` and `perf -p`) build in `release`, and start-up is measured on `dist`: with `release` at 3, every speed figure of record would have described code that does not ship. Building the perf tools in `dist` instead would have made every measurement wait for fat LTO over one unit. Nothing builds at `opt-level = 3` now.
- **The pin is a requirement, not only a lockfile entry.** `url` asks for `idna_adapter = "1"`, so `cargo update` would undo a lockfile pin without a word. `cortex-llm` names `idna_adapter = "~1.0.0"` (optional, under its `http` feature), only to hold it, with the reason in the root `Cargo.toml` and in [dependencies.md](../../dependencies.md); a test of its transport fails if the lockfile's adapter is not 1.0.
- **A host with letters outside ASCII is refused by the harness, not only by the URL library,** whose error says nothing useful: the core refuses a provider's `base_url` with one (`invalid_provider`, naming the key and the `xn--` form), and the transport refuses any such request before a client exists. A provider's base URL is the one place such a URL can reach the binary today.
- **The remapping is in `xtask`, not in the manifest.** Cargo's `trim-paths` is not stable on 1.99.1 (it asks for the nightly feature), and a profile's `rustflags` is not stable either. So every build of the shipped binary through `xtask` (`size`, `startup --profile dist`, and step 34's `dist`) sets `CARGO_ENCODED_RUSTFLAGS` to the flags already set and one `--remap-path-prefix` for each of the home folder, cargo's home, the sysroot, the workspace and the target directory, from the most general to the most specific (rustc applies the last that matches). A plain `cargo build --profile dist` is not remapped; the check finds that.
- **The check reads the binary as bytes** for the shape of a home path, with the repo rule's finder (`/home/<name>`, `/Users/<name>`, a drive letter and `\Users\<name>`), and for the build's own `CARGO_HOME` in either separator and any case. In a binary, strings are packed end to end, so there a root counts after a letter too. It prints each kind with the user's name left out. `size` fails on it whether or not `--check` is given. `cargo xtask home-paths FILE…` reads any file.
- **The pull-request job is `pr-size` in `ci.yml`,** after `needs: [lint, test]`. It skips a pull request whose every file is documentation, a list of breaks, `xtask/**` (but for `xtask/src/shipped.rs`) or a workflow.
- **The decision record was written by hand.** The brief asked for it through `cargo xtask decide`, which is the permission engine's probe and does not write records.

### Shown on a desk build

A plain `cargo build --profile dist` at the desk (at `61bd1a0`), then `cargo xtask home-paths target/dist/cortex`: it failed, with `/home/<user>` 512 times (the first followed by `/.cargo/registry/src/index.crates.io-…/h2-0.4`) and the build's `CARGO_HOME` 529 times. The same tree through `cargo xtask size`: `paths none under a home folder or cargo's home`, and 7,960 bytes smaller.

### Breaks

`mutations/step-33-size-fix.breaks`, run with `cargo xtask mutate step-33-size-fix --jobs 2`: MUTATE_VERDICTS

### The independent review

REVIEW_FINDINGS
