# 0036: The shipped binary is optimised for size, without ICU, and holds no path of the machine it was built on

- **Status:** Accepted by the owner on 2026-11-16 (the size study's plan B, chosen by the owner; the home paths and the pull-request size job, the owner's picks from the same study). It replaces the line "`opt-level = 3` for `dist`. Size optimisation is held in reserve." of [decision 0012](0012-release-profile.md), which the owner left as it is; the rest of 0012 stands, unwinding on panic included. Its reserve (`panic = "abort"`) is replaced by the order of reserves in [decision 0039](0039-the-binary-s-budgets-are-20-mib-on-linux-and-25-on-windows.md) (the size step, 2026-11-19).
- **Date:** 2026-11-16

## Context

After step 23 the Windows `dist` binary was 19,226,112 bytes (18.34 MiB), over its 18 MiB budget ([performance-budgets.md](../performance-budgets.md), "Binary size"), and the `main` workflow's `size-windows` job failed on every push (run 43811142390, issue #61). The Linux musl binary had 518,808 bytes left, about one step's worth. Steps 24 to 34 are expected to add 4.5 to 6 MB more on Linux and about 1.3 times that on Windows: the script engine and the extension crate, the MCP client, and the rest.

A read-only size study measured what the binary is made of and what each option saves, on the `main` workflow's runners and on the reference machine. Its figures, its method and the outlook are in [measurements/binary-size.md](../measurements/binary-size.md). Two more findings came with it:

- **A binary built on a developer's machine named its user.** It held 399 to 512 source paths under the build machine's home folder (the panic locations of every third-party crate, which cargo compiles from `~/.cargo/registry/src/…`). CI's binaries held the runner's home folder, whose user is `runner`. Rule 3 of the build plan forbids it wherever the binary is built.
- **A pull request never saw its own size.** Only the `main` workflow checked it, after the merge, which is how step 23's second pull request took Windows over.

## Options

The study's, largest Windows saving first (Windows, Linux musl). The speed figures are the study's, a release build on the reference machine against `opt-level = 3`.

| Option | Saves | Costs |
|---|---|---|
| `opt-level = "s"` and `panic = "abort"` | 10.0 MB, 5.8 MB | Everything "s" costs below, and a panicking tool, background task or highlighter thread ends the whole process (decision 0012's reason for unwinding). |
| `opt-level = "z"` | 6.6 MB, 5.1 MB | The views' frame +100%, highlighting +80%, the map +55%, the first frame +25%. |
| `panic = "abort"` alone | 6.5 MB, 2.2 MB | As above. Unwinding is a third of the Windows binary: 8 to 25 times what decision 0012 measured on a spike. |
| **`opt-level = "s"`** | **5.3 MB, 4.2 MB** | The views' frame +50% (0.15 ms against a 2 ms budget), highlighting from cold +30% (off the UI thread), the map +10%, the first frame faster. |
| Our crates at "s", third-party at 3 | 4.0 MB, 3.2 MB | Less saved, two settings to keep. |
| `opt-level = 2` | 1.0 MB, 0.6 MB | Too little. |
| Highlighting off; reqwest without HTTP/2 | 0.6 MB and 0.5 MB each | Features decisions 0015 and 0004 want. |
| **`idna_adapter` held at 1.0** | **134 KB, 139 KB**, 21 crates | A URL whose host has letters outside ASCII is refused; its `xn--` form works. |

## Decision

- **`opt-level = "s"` for the shipped binary.** It is set on `release`, which `dist` inherits, so that every measurement made in `release` (`cargo xtask perf`, the `perf_` tests, `try -f profile=release`) runs code optimised as the shipped binary's is. `release` keeps thin LTO over 16 units, which builds in a fraction of `dist`'s time; nothing builds at `opt-level = 3` any more.
- **Unwinding stays on.** `panic = "abort"` is the reserve: if Windows nears its budget again, it is the next 6.5 MB, and it is a decision for the owner, against decision 0012's reasons.
- **`idna_adapter` is held at 1.0** by a requirement of `cineris-llm` (`~1.0.0`, in the root `Cargo.toml`), not by the lockfile alone, which the next `cargo update` would undo. The harness refuses a host outside ASCII itself, with the `xn--` form named: the core when it reads a provider's `base_url` (`invalid_provider`), and the transport for any request. A test of `cineris-llm` fails if the adapter moves.
- **No path of the build machine in the binary.** Cargo's `trim-paths` is not stable on the pinned toolchain (1.99.1), so every build of the shipped binary through `xtask` (`size`, `startup --profile dist`, and step 34's `dist`) gives rustc a `--remap-path-prefix` for the home folder, cargo's home, the sysroot, the workspace and the target directory (`xtask/src/shipped.rs`). `cargo xtask size` then reads the binary and fails on any path under a home folder (`/home/…`, `/Users/…`, `C:\Users\…`) or under the build's cargo home. A plain `cargo build --profile dist` does not remap: the shipped binary is built through `xtask`.
- **Each pull request shows its size.** A job of `ci.yml`, `pr-size`, builds `dist` on both systems after the four required jobs pass, and reports each binary's size, its room and its difference from `main`'s last record; over the budget it fails. It is not a required check ([testing-and-ci.md](../testing-and-ci.md), "Each pull request's binary size").

## Why

- **"s" saves 4 to 5 MB on each system and every speed figure stays far inside its budget,** measured before and after on the reference machine and on both hosted runners ([measurements/binary-size.md](../measurements/binary-size.md)). The harness's hot paths are rendering, highlighting, the map and JSON decoding; each was measured, and the slowest change leaves the figure a small fraction of its budget.
- **Unwinding is what keeps a bug in one tool from ending an hour-long session.** It costs far more than 0012 thought, and that is recorded, but "s" alone brings Windows well under its budget with room for the steps planned.
- **Measuring what ships.** With `release` at 3 and `dist` at "s", every speed figure of record would describe code nobody runs.
- **A host with letters outside ASCII is rare for a model provider,** and its `xn--` form is always available.

## What is given up

- Speed, by the measured amounts: the views' frame, highlighting from cold and JSON decoding are slower at "s"; the figures and their budgets are on the measurement page.
- A provider URL whose host has letters outside ASCII must be written in its `xn--` form.
- A `dist` binary built by hand with cargo is not checked and holds the builder's paths; the shipped one is built through `xtask`.
- Each pull request that can change the binary spends runner minutes on two `dist` builds, measured in [testing-and-ci.md](../testing-and-ci.md).

## Revisit when

- Windows nears its budget again: `panic = "abort"` is the reserve, for the owner to decide.
- A speed figure nears its budget at "s": "s" for our crates and 3 for one hot third-party crate (or the reverse) is the finer setting the study measured.
- Cargo's `trim-paths` is stable on the pinned toolchain: it replaces the remapping.
- A provider or a user needs a host outside ASCII: the adapter's newer versions bring ICU back, at the cost above.
