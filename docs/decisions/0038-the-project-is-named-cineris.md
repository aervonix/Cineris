# 0038: The project is named Cineris

- **Status:** Accepted by the owner on 2026-11-18. Supersedes [decision 0013](0013-naming.md).
- **Date:** 2026-11-18

## Context

[Decision 0013](0013-naming.md) named the project Cortex-Harness, with the command `cortex` and the alias `cx`. Step 2's clash check found one serious clash: Snowflake's terminal coding agent, its Cortex Code CLI, installs a command named `cortex`. That is the same command name, put on the path by its installer, for the same kind of product. The owner kept the names on 2026-11-12, resting on the rule that installation never overwrites another program's command.

The owner has since decided to rename the project instead. A product that answers to the same command as another coding agent would be confused with it however carefully its installer behaves, and there are no outside users yet. Renaming now costs one change to the repository. Later it would cost every user a migration.

This record says what the names are, how the alias ships (carried over from 0013, which this record replaces), the new install rule, and what the new names were checked against. 0013 stays as written: it is the record of what was decided then.

## Options

The owner chose the name. The options recorded here are about how to switch.

| Option | What it costs | Assessment |
|---|---|---|
| **A clean switch:** every name changes in one pull request, nothing reads the old names afterwards | Anyone with old folders or variables moves them by hand. There is no one outside the project to do that. | **Chosen.** |
| A migration: read the old folders, variables and keys for a while, and move them | Code that exists only for the change, and has to be tested and then taken out again | Rejected: there are no outside users to migrate. |
| Rewrite git history so that old commits carry the new names | Every clone and open branch breaks. The record of what was built when is lost. | Rejected. |

## Decision

**The project is Cineris.** It is Latin for "of ash": the genitive of *cinis*, ash, for a harness built from scratch. English readers say it "SIN-er-is". The brand is "Cineris", never "Cineris CLI".

### The names

| Thing | Name |
|---|---|
| Project, in prose | Cineris |
| Crates | `cineris-<part>`, in `crates/cineris-<part>/`, all with `publish = false` |
| Rust identifiers | `cineris_*`, `Cineris*` |
| Command | `cineris` (`cineris attach`, `cineris serve`, …) |
| Alias | `cin`: the same program under a second name |
| Environment variables | `CINERIS_*` |
| Configuration, data and state directories | `cineris` under the platform's standard locations |
| Project directory | `.cineris/` |
| The Windows Credential Manager's entries | `cineris:<provider>` |
| Instruction file | `AGENTS.md` (unchanged) |
| The developer's own files, outside the product's folders | `~/.config/cineris-dev/owner-terms` (with `CINERIS_OWNER_TERMS_FILE` and the git key `cineris.ownerTermsRequired`), the corpus cache `~/.cache/cineris-dev/corpus` (`%LOCALAPPDATA%\cineris-dev\corpus` on Windows), and the live tests' `~/.local/state/cineris-live/` and `~/.config/cineris-live/` |
| The repository | `aervonix/Cineris` |
| The commit identity | `Cineris <dev@cineris.invalid>`. Commits made before the rename keep `Cortex-Harness <dev@cortex-harness.invalid>`, and everything that reads commits accepts both. |
| The domain | `cineris.dev`, which the owner has bought. Nothing links to it, and no crate names it as its `homepage`, until a site is there. |

The `-dev` folders are named so that none of the developer's files sits inside a folder the product owns.

### The clean switch

- Nothing reads an old name after the rename: no old folder, environment variable, git key or credential entry is accepted, and none is moved. Formats keep their versions: a rename alone is not a format change.
- Git history is not rewritten. Commits before the rename keep the old names and the old identity.
- `cargo xtask rename` made the change ([CONTRIBUTING.md](../../CONTRIBUTING.md), "Renaming the project"). Its table is the one place the old and new names are listed side by side. `cargo xtask rename --check`, run by the repo rules in the lint job, fails on an old name outside a short list of kept files and lines, each kept with its reason: the tool itself, 0013, this record, the rename's own step record, the record of the labels renamed after it, 0013's row in the decisions index, and the build plan's row about the clash. `rename branch` brings a branch made before the rename across.

### How the alias ships

Carried over from 0013 unchanged, but for the names. 0013 has the options and the measurements.

- **One executable, two names.** `cin` is a link to `cineris`: no launcher, no second build target, no shim.
- **The program behaves the same under both names.** It reads the name it was started with only to echo it in usage and help text. An unexpected name is treated as `cineris`.
- **Linux archives** contain `cineris` and a relative symbolic link `cin`.
- **Windows archives** contain `cineris.exe` and an install script that creates `cin.exe` as a hard link beside it, or as a copy if the file system cannot link.
- **`cineris alias install`** and **`cineris alias remove`** create or remove the alias beside the running executable, for people who unpack by hand, and after an update replaces the executable.
- **Package managers** that support a second command name for one executable declare both names there.
- **Tests:** the real binary is run under both names on both OSes and must give the same `--version`, the same behaviour, and help text in the name used. The start-up measurement is taken under both names.

### The install rule (the owner's decision, 2026-11-18)

This rule replaces 0013's "never overwrite another program's command".

- **When another program already provides `cineris` or `cin`,** the installer takes the name only after the user answers yes to a yes/no question that names the other program and its path. The default answer is no.
- **With no terminal to ask on** (a script, CI), the installer never takes the name.
- **A yes makes our command the one that runs.** The other program's file is neither deleted nor changed, so the choice can be undone.
- The alias stays optional: `--no-alias`. `cineris` always works when `cin` is taken.
- Step 34 builds this ([build-plan.md](../build-plan.md), step 34). The install script and `cineris alias install` are tested for each case.

## The clash check of the new names

The new names were checked on 2026-11-18, as 0013 checked the old ones. The sources were the distributions' package-contents searches (Debian and Ubuntu stable, unstable and noble; Alpine edge) and the package managers' listings (Arch and the AUR, Homebrew, MacPorts, nixpkgs, Chocolatey, Scoop's main and extras buckets, winget). The npm, PyPI and crates.io registries were checked too, and so was PowerShell's source for its built-in aliases. These are facts for the record. A clash is no longer a reason to stop: the install rule above covers it.

**`cineris`: no clash.** No package or tool installs a command of that name. The only hits were a game's install script named `play-cineris-somnia.sh` in Debian's `play.it` (not on the path) and small personal repositories.

**`cin`: minor.**

| Tool | What installs the command | How likely on a developer's path |
|---|---|---|
| Cinelerra-GG Infinity, a Linux video editor | Its upstream builds name the executable `cin`, and the AUR's `cin-appimage` links `/usr/bin/cin` to the AppImage. RPM Fusion and the AUR's `cinelerra-gg` name it `cinelerra-gg` instead. | Video editors' users. An established program, but most packagers rename it. |
| Kado's `cin`, a tool for encrypted application configuration | `cin` (`cin.exe`), from winget (`Kado.Cin`), a Homebrew tap, a Scoop bucket and release archives | Negligible: created in June 2026, about 800 downloads |
| npm's `cin` (a CI server, last published in 2012) and `@croissanstudiodev/cin` | `cin`, from a global npm install | Negligible: tens of downloads a month |

Not commands, and so not clashes: the `cin` crate (a library), PyPI's `cin` (no command), and C++'s `std::cin`. No built-in Windows command or PowerShell alias is named `cin`. The owner chose `cin` knowing it is close to `std::cin`. Not looked up: Fedora's package contents, Homebrew formulae's files beyond their names, macOS's `/usr/bin` and the shells' built-ins. None is known to have a `cin`.

## Why

- **A coding agent should not answer to another coding agent's command.** With the old name, people would mistake one product for the other whatever the installer did.
- **Now is the cheapest time.** There are no outside users, no published crates and no released binaries.
- **A clean switch keeps the code free of migration paths** that would exist only for the change.
- **The `-dev` folders keep the developer's files out of the product's folders,** so that removing the product's configuration never touches them, and the other way round.

## What is given up

- Anyone who had the old folders, variables or credential entries on a machine (the owner, and the sessions that build it) moves or recreates them by hand.
- The pull requests, records and links before the rename read with the old names. GitHub redirects the old repository's address.

## Revisit when

- A clash with `cin` gets in users' way often enough that the question is no longer worth it: ship without the alias by default.
- Crates are to be published: check the names on crates.io then.
