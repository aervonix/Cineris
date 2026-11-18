<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/cineris-logo-dark-background.png">
    <source media="(prefers-color-scheme: light)" srcset="assets/cineris-logo-light-background.png">
    <img src="assets/cineris-logo-light-background.png" alt="The Cineris logo: a hexagonal C around a terminal prompt, with two purple sparkles" width="220">
  </picture>
</p>

# Cineris

*Cineris* (said "SIN-er-is") is Latin for "of ash": a harness rebuilt from scratch.

A terminal agent harness that learns your project, your models and your habits, and stays fast while it does. It is an LLM agent loop that reads and edits code, runs commands and talks to several model providers, written from scratch in Rust for Linux and Windows.

> **Status: early development.** The design is written down and the repository skeleton is in place. The harness itself is not built yet, and there is nothing to install. The [build plan](docs/build-plan.md) lists the steps from here to a first usable release.

## What it is meant to be

- **It adapts.** To the project (its build and test commands, its conventions), to the model (formats and prompt style chosen from results measured on your machine), and to you (profiles, hot-reloaded configuration, extensions it can write for itself).
- **It is lean without being bare.** Lean context, compressed tool output, cache-friendly prompts, and tokens and cost always in view. Near-instant start-up, a small memory footprint and smooth rendering.
- **It is ahead.** Parallel attempts judged by your project's tests, replay and evaluation of real sessions, and a Coordinator that runs a team of sessions inside limits you set.

The full picture is in [docs/vision.md](docs/vision.md). Nothing is sent anywhere: there is no telemetry.

## Documentation

The design docs are the specification the code is built from.

| If you want | Read |
|---|---|
| What the product is for | [docs/vision.md](docs/vision.md) |
| The shape of the whole system | [docs/architecture/00-overview.md](docs/architecture/00-overview.md) |
| What is built next, and what has been decided | [docs/build-plan.md](docs/build-plan.md) |
| Why a choice was made | [docs/decisions/](docs/decisions/README.md) |
| Everything else | [docs/README.md](docs/README.md) |

## Building from source

You need the Rust toolchain manager, [rustup](https://rustup.rs). The toolchain version is pinned in `rust-toolchain.toml` and is installed on first use.

```sh
cargo build                # the workspace
cargo xtask alias          # build `cineris` and put its short alias `cin` beside it
target/debug/cineris --version
target/debug/cin --version
```

The command is `cineris`. `cin` is the same executable under a second name.

Developer commands live in the `xtask` crate. `cargo xtask` lists them, and `./x` is a shorthand for it:

```sh
cargo xtask check          # formatting, repo rules, lints, licence checks
cargo xtask test           # all tests (needs cargo-nextest)
cargo xtask ci             # exactly what the blocking CI jobs run
```

[CONTRIBUTING.md](CONTRIBUTING.md) has the rest: the layout, the test tiers, and the rules the checks enforce.

## Security

To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Licence

[MIT](LICENSE).
