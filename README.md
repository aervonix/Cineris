<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/cineris-logo-dark-background.png">
    <source media="(prefers-color-scheme: light)" srcset="assets/cineris-logo-light-background.png">
    <img src="assets/cineris-logo-light-background.png" alt="The Cineris logo: a hexagonal C around a terminal prompt, with two purple sparkles" width="220">
  </picture>
</p>

# Cineris

*Cineris* (said "SIN-er-is") is Latin for "of ash": a harness built from scratch.

A terminal agent harness that learns your project, your models and your habits, and stays fast while it does. It is an LLM agent loop that reads and edits code, runs commands and talks to several model providers, written from scratch in Rust for Linux and Windows.

## Status

Cineris is under active development, built in the ordered steps of the [build plan](docs/build-plan.md). **There is no release and no installer yet.** The only way to try it is to [build it from source](#building-from-source).

The milestones so far:

- **Reached:** the headless alpha (after step 15) and the adaptive core (after step 19).
- **Being built now:** the interactive alpha (after step 25), the full terminal interface. Its full-screen presenter has landed; the first-run flow and rewind in the interface have not.
- **Next:** the first usable release (after step 34): extensions the harness can write, MCP, a hardening and a performance pass, and release binaries. Some steps on the way are built already: the Linux sandbox, the RPC server and the second provider protocol.

### What works today

- **Providers.** Anthropic, and any server that speaks the OpenAI chat protocol (OpenRouter, DeepSeek, Venice.ai, Ollama, llama.cpp, vLLM, LM Studio).
- **Tools.** Reading, searching, listing and outlining files; writing and editing them with snapshots and undo; a shell tool; a plan the agent keeps; and `remember`.
- **Permissions.** Permission levels from read-only to unrestricted, rules you can add per run, and project trust.
- **Sandbox.** Linux only, with Landlock and seccomp. Windows has process-tree and resource limits but no sandbox yet.
- **Sessions.** Stored, listed, continued, resumed and forked, with context compaction.
- **Adapting.** A project profile and memory you can review (`cineris project`), a repository map, local usage and cost statistics (`cineris usage`), and verification of a turn against the project's own checks.
- **Headless mode.** `cineris -p`, with text, JSON or JSON-lines output and exit codes.
- **RPC server.** `cineris serve` speaks JSON-RPC over standard input or a local socket, and `cineris attach` runs the terminal interface on a running server.
- **Terminal interface.** Inline and full-screen, with panels, the mouse, `/usage`, `/plan`, `/project`, `/verify`, `/undo` and hot-reloaded settings.

Still to come: extensions, MCP, the Windows sandbox, and everything after the first release in the build plan.

### Trying it from source

After [building](#building-from-source), a one-shot run needs a provider key. Store it, and name a model:

```sh
target/debug/cineris auth login anthropic < ~/anthropic-key.txt
target/debug/cineris -p "explain this repository" --model claude-sonnet-5-5
```

The key can also come from `ANTHROPIC_API_KEY`. Where the key and the model go is in [the configuration page](docs/architecture/08-configuration.md) and [its reference](docs/configuration/reference.md); the flags are in [headless mode](docs/architecture/10-headless.md). Without `-p`, `cineris` opens the terminal interface.

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
