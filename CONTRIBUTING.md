# Contributing

How the repository is laid out, how to build and test it, and the rules the checks enforce. The design itself is in [docs/](docs/README.md): the docs are the specification, and code that must differ from a page changes the page in the same pull request.

## Set-up

You need [rustup](https://rustup.rs) and git. The toolchain is pinned in `rust-toolchain.toml` and rustup installs it on first use. Two more tools are used, both installed as prebuilt binaries in CI (their versions are in [docs/dependencies.md](docs/dependencies.md)):

| Tool | Needed for |
|---|---|
| [cargo-nextest](https://nexte.st) | Running the tests |
| [cargo-deny](https://embarkstudios.github.io/cargo-deny/) | The licence, ban and source checks in `cargo xtask check` |

## The commands

Everything goes through the `xtask` crate: `cargo xtask <command>`, or `./x <command>` from a shell. CI runs these same commands.

A command works on the checkout it is run in: the nearest folder at or above the current directory that holds the workspace's `Cargo.toml` and `xtask/` (git's top level, and the same in a copy with no `.git`). `cargo xtask --root DIR <command>` names another. It never works on the checkout the binary happens to have been built in, which differs when several checkouts share one target folder (`CARGO_TARGET_DIR`): cargo then runs the first checkout's build of `xtask` from all of them, and `xtask` says so (`working on …; this xtask was built from the source in …`). The code it runs is still that build's, so a change to `xtask` itself is tried with a target folder of the checkout's own. Outside every checkout it refuses and asks for `--root`.

| Command | What it does |
|---|---|
| `cargo xtask check` | The static checks (tier 0): `fmt`, `rules`, `breaks`, `generated`, `clippy`, `no-scripts`, `deny`. `cargo xtask check clippy` runs one of them. `no-scripts` checks the binary without the script engine (its `scripts` feature off), so that such a build stays possible. Every step runs even if an earlier one fails, and the timings are printed at the end. |
| `cargo xtask test` | All tests. Arguments go to `cargo nextest run`: `cargo xtask test --profile fast` runs tiers 1 and 2 only, `cargo xtask test -p cineris-cli` one crate, and builds only that crate's tests. |
| `cargo xtask ci` | Exactly what the blocking CI jobs run: the static checks, then tiers 1 to 3. Run it before opening a pull request. It builds with cargo's `ci` profile, as CI does, so its first run builds the dependencies once more, unoptimised, into `target/ci`. After a disk stall (every task waiting on I/O for 10% or more of a run) it runs the cluster of timeouts again once and says so, and away from CI the tier check does not judge the stalled run (since the eighth tooling step). Away from CI a known flake that failed is run again once, and in CI it is named (since the tenth tooling step). |
| `cargo xtask ci --tiers` | Builds and runs the tests as CI does, and prints each tier's time against its budget, split by crate, and its slowest tests. `-p <crate>` for some crates, `--top N` for more tests, `--from junit.xml --os windows` to read the report a CI run kept. |
| `cargo xtask ci --tiers --compare NEW [--against OLD]` | Two runs' tiers side by side, each above its own process-start cost: by tier, by crate, and the tests that moved most. `NEW` and `OLD` are JUnit reports, run ids of the `ci` workflow, or `main`, its latest successful run there (`OLD` when not given). Reads runs through `gh`. |
| `cargo xtask ci-times RUN [--against RUN\|main]` | A run's job times and tier times, as the tables a step's `ci.md` has, ready to paste. |
| `cargo xtask try --command '…'` | What the `try` workflow runs on the hosted runners: `test`, `stress`, `startup`, `perf` or `ci`, in the `ci`, `release` or `dist` profile ([docs/testing-and-ci.md](docs/testing-and-ci.md), "A run on the runners without a pull request"). `try --dispatch BRANCH` starts it, with `--against REF` resolved to its commit at the dispatch, and names a run that has sat queued with no jobs past `--queued-warn MIN` (10). |
| `cargo xtask branch commit-records --branch B --message MSG FILE…`, `branch fold COMMIT [--last]`, `branch to-end COMMIT` | A branch's history without an interactive rebase: records (`docs/steps/**/*.md`, `docs/outstanding/*.md`, `mutations/*.breaks`) committed to a branch no working tree is using, the working tree untouched; the staged changes (or the last commit) folded into an earlier commit of the current branch, or a commit moved to its tip, its old tip kept as `refs/xtask-backup/<branch>/<time>` ([docs/testing-and-ci.md](docs/testing-and-ci.md), "A branch's history without an interactive rebase"). |
| `cargo xtask stress -p <crate>` | Runs the crate's tests many times over (`--count N`, 100 when not given), each time under the next seed, and reports each test that failed on some, with the seed that brings the failure back. `--cores K --load N` squeezes the run onto K cores beside N busy loops, which makes a race show far more often. A known flake (an open item's `- Flaky:`) that failed is run once more under its seed, and does not fail the run if that passes; `--strict` runs nothing again ([docs/testing-and-ci.md](docs/testing-and-ci.md), "Known flakes"). |
| `cargo xtask mutate` | Breaks the code on purpose, by the breaks of `mutations/*.breaks`, on copies of the tree, and reports which tests failed. It stops after five minutes and carries on from there the next time. It runs two breaks at a time, or `--jobs N`, each in a copy of its own with a scratch folder of its own for its tests. `--check` only checks that every break still applies, which `check` also does. `--changed REF` runs only the breaks the diff from `REF` can affect and says how many it left; a break's `in-string: TEXT` changes a word inside a string literal. |
| `cargo xtask replace FILE --from CODE --to CODE` | An edit of Rust code that finds its place by tokens, so it holds however `rustfmt` laid the code out; it must be there once (`--in ITEM`, `--occurrence N`). With `--text`, an exact-text edit of any file; with `--script FILE`, many edits across many files, all or none. `./x replace` still works when `xtask` does not build. "Editing from a script", below. |
| `cargo xtask apply SCRIPT [--dry-run] [--keep]` | The changes of a script (`replace --script`'s edits, `@@ write: PATH` for a whole file, `@@ delete: PATH`) and the checks to run on them (`@@ check: fmt`, `rules`, `links`, `check STEP…`, `build -p CRATE`, `clippy -p CRATE`, `test …`), in one command: all or none, and every file put back when a check fails ("Editing from a script", below). |
| `cargo xtask outstanding [--check]` | Writes the index of `docs/outstanding/README.md` from the items' files: each item's first line, its `- Area:` and its `- Owner:` (since the tenth tooling step). Run it after adding, settling or changing an item, and after a rebase. `rules` fails when the index is not what it writes; `--check` only compares. |
| `cargo xtask decision new "TITLE"` | The next numbered record in `docs/decisions/`, with the headings of the format its index describes, and its row added to the index's table; `--status TEXT` (Provisional when not given), `--dry-run`. |
| `cargo xtask scratch-test CRATE FILE` | Runs one throwaway test file as a module of the crate's integration tests, in a copy of the tree under `target/scratch-test/`: nothing to register, nothing left in the working tree; `--cargo-profile release --ignored` makes it a measurement ([docs/testing-and-ci.md](docs/testing-and-ci.md), "A throwaway test"). |
| `cargo xtask api-diff [--against REF] [-p CRATE]… [--features CRATE/FEATURE,…]` | The public items added, removed and changed between a ref and the working tree, with their signatures before and after ([docs/testing-and-ci.md](docs/testing-and-ci.md), "The public API between two trees"). |
| `cargo xtask owner-guard --unpushed` | What a push of `HEAD` would send, checked for the owner's terms as the git hook checks it ([docs/testing-and-ci.md](docs/testing-and-ci.md), "The owner guard"). |
| `cargo xtask decide [--level L] [--rule RULE]… COMMAND…` | What the permission engine decides of a shell command, with its reasons and the analysis it stands on, without writing a test. |
| `cargo xtask rules` | The repo-rule checks alone (see below). |
| `cargo xtask links` | The part of `rules` that is about the docs, by itself: every link inside the repository leads to a file and a heading that are there, and `docs/steps/` and `docs/outstanding/` hold what they should. It asks cargo nothing, so it answers at once. |
| `cargo xtask fmt` | Formats the workspace. |
| `cargo xtask schema` | Regenerates the protocol's JSON Schema and TypeScript declarations in `docs/protocol/` from the types in `cineris-protocol`. Run it after any change to those types and commit the result. `--check` only compares. |
| `cargo xtask gen` | Regenerates every generated file: what `schema` writes, and the configuration schema and reference in `docs/configuration/`, from the settings in `cineris-config`. Run it after adding or changing a configuration key. |
| `cargo xtask regen [--list] [GENERATOR…]` | Rebuilds every generated file that can be rebuilt from the tree, with no network and no key: the open items' index (`outstanding`), what `gen` writes, every snapshot (`CINERIS_UPDATE_SNAPSHOTS`) and the reference screens (`tui-screens --write`). It ends with the files that differ from `HEAD` and with `links`. "After a rebase", below. |
| `cargo xtask perf-ab --base DIR -p CRATE… [NAME]…` | Two trees' ignored `perf_` tests, built once each and run in turn on one machine, `--count` rounds: each measurement's median and range on both sides and the difference, from the tests' `perf: NAME VALUE UNIT` lines (`cineris_testkit::numbers::report`). What `try` runs for `perf -p … -f against=REF`. |
| `cargo xtask size [--check] [--target T] [--against-main] [--breakdown] [--at COMMIT] [--profile release]`, `unused`, `advisories` | What the non-blocking workflows and the pull-request size job run: the `dist` build with its binary size and crate count (with `--against-main`, against the size `main` last recorded), failing when the binary holds a path under a home folder; the unused-dependency check (needs [cargo-machete](https://github.com/bnjbvr/cargo-machete)), and the advisory check. |
| `cargo xtask home-paths FILE…` | Fails when a built file holds a path under a home folder (`/home/…`, `/Users/…`, `C:\Users\…`) or under cargo's home, naming each kind without the user's name. `size` runs the same check on the binary it builds. |
| `cargo xtask fuzz` | Coverage-guided fuzzing of the stream decoders, the outline extractors and the RPC server's decoder, each target for a fixed time (`--seconds N`, 60 when not given). What the nightly workflow's `fuzz` job runs. Needs a nightly toolchain and [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz). The targets are in `fuzz/`. |
| `cargo xtask live [PROVIDER]`, `cargo xtask live --headless [PROVIDER]`, `cargo xtask record [--provider PROVIDER] [--dry-run] [SCENARIO…]`, `cargo xtask record --toolbox`, `cargo xtask live --spent` | By hand only, against a real provider, and never in CI: the live smoke test, and the recorder of provider fixtures. The provider is `anthropic` when none is named, or `openrouter`, `deepseek` or `venice`. Both need `CINERIS_LIVE_<PROVIDER>_KEY_FILE` set to the path of a file that holds that provider's API key. The key is never given on a command line or in the environment, and a recording that still holds it is refused ([docs/testing-and-ci.md](docs/testing-and-ci.md), "Fixtures"). They cost money: a few hundredths of a cent for the smoke test, about a cent and a half to record everything. Each call's spend is added to a ledger outside the repository, which `live --spent` totals; `record --dry-run` prints a scenario's first request and sends nothing. |
| `cargo xtask alias` | Builds `cineris` and puts its alias `cin` beside it in the target directory. |

## Layout

```
crates/cineris-*    the product, one crate per job (docs/architecture/01-workspace.md)
crates/cineris-cli  the `cineris` binary; `cin` is a link to it
catalog/           the model catalog, embedded in the binary by cineris-llm's build script
xtask/             developer commands; not shipped
corpus/            the list, the lock and the pin of the corpus of real code that perf tests measure (`cargo xtask corpus`); no code
fuzz/              fuzz targets for the decoders; a package of its own, outside the workspace
mutations/         the break lists `cargo xtask mutate` runs, one file per step
docs/              the specification
docs/protocol/     generated: the protocol's JSON Schema and TypeScript declarations
docs/configuration/  generated: the configuration file's JSON Schema and reference
assets/            the logo
AGENTS.md          the rules and commands for anyone changing the code, an agent included; CLAUDE.md includes it
.worktreeinclude   names the one private, gitignored notes file (CLAUDE.local.md) that an agent tool copies into a new working copy
.config/nextest.toml, clippy.toml, deny.toml, rustfmt.toml, rust-toolchain.toml
.github/workflows/ci.yml             the blocking checks
.github/workflows/main.yml           after each merge: release build, size, crate count
.github/workflows/nightly.yml        nightly: advisories, unused dependencies, every release target
.github/workflows/try.yml            by hand: one xtask command on Linux, Windows, both or Ubuntu 22.04, for a branch with no pull request; or two refs compared on one runner
```

A crate may depend only on the workspace crates that [docs/architecture/01-workspace.md](docs/architecture/01-workspace.md) lists for it. `cargo xtask rules` checks this from `cargo metadata`. If the design has to change, change that page and the table in `xtask/src/rules/crate_graph.rs` in the same pull request.

Every crate's manifest follows the same shape: `version`, `edition`, `license` and `publish` come from the workspace, `[lints] workspace = true`, and `[lib] doctest = false`.

## Tests

The tiers are defined in [docs/testing-and-ci.md](docs/testing-and-ci.md). Where a test lives, and its name, decide its tier:

| Tier | Where | Notes |
|---|---|---|
| 1, unit | `#[cfg(test)]` modules inside the crate | Pure logic. No processes, no network, no real time. |
| 2, integration | `tests/it/main.rs`, one binary per crate, with modules inside it | Several crates together, in-process. |
| 3, system | `crates/cineris-cli/tests/system/`, and test-kit-driven binaries | The real binary as a process. **Test names start with `sys_`.** |

- **One integration-test binary per crate.** Every extra binary is another link step. Add a module to `tests/it/`, not a file to `tests/`.
- **No doc tests.** An example that must compile is an ordinary test.
- **No real waiting.** Timeouts, retries, backoff and cooldowns are tested with paused or simulated time: the async runtime's test clock, or the fake clock. A test never sleeps. A process test waits on a condition with a deadline, never on a fixed delay.
- **Property tests** are written with the small helper described in [docs/testing-and-ci.md](docs/testing-and-ci.md) ("Property tests"), not with a property-testing crate. Every run is the same, a failure prints the seed of the failing case, `CINERIS_PROPERTY_SEED=<seed>` runs that case alone, and `CINERIS_PROPERTY_SCALE=<n>` multiplies the number of cases.
- **A test of the agent loop sees a seeded order of events.** The rig puts the blocking pool's work on the test's thread, at moments a seed decides: 0 in an ordinary run, the next one for each repeat of `cargo xtask stress`. A test must pass under every seed, so it fixes an order it needs with a gate. A failed test prints its seed, and `CINERIS_TEST_SEED=<seed>` runs it again in the same order.
- **A test's directory** is `cineris_testkit::scratch!("name")`: new each time, removed when the test passes, with helpers for files and for a git repository that reads none of the machine's git configuration.
- **No retries**, with one exception: a test inside a module named `timing_sensitive` gets one retry, and nextest reports it as flaky when the retry was needed.
- **A test that starts a process** is a system test and is named `sys_…`, wherever it lives. That name puts it in nextest's `process` group (two at a time; in the blocking job, after tiers 1 and 2 and three at a time), gives it tier 3's time limit, and keeps it out of the `fast` profile. Since the twelfth tooling step a test that is not named `sys_…` or `perf_…` is stopped at its first process start, with a message that says what to do ([docs/testing-and-ci.md](docs/testing-and-ci.md), "Test tiers").
- **A process test's children are the test binary itself,** run again (`cineris_testkit::children`): a child that stays until it is killed, a grandchild, a child that ignores the polite interrupt. Each reports to the test over a loopback socket and keeps the connection until it dies, so the test sees it start and sees it go. No helper program is built, and no test uses `sleep`.
- **Every test has a time limit.** In tiers 1 and 2 a test is reported as slow at 2 s and stopped, as a failure, at 10 s. In tier 3 the marks are 10 s and 60 s. A test that is slow needs a reason or belongs in a non-blocking tier.
- **Every tier has a budget** for the time its tests take together, counted above the start of each test's process, which the run measures: 20 s on Linux or 25 s on Windows for tier 1, 40 s for tier 2, and 60 s on Linux or 90 s on Windows for tier 3. `cargo xtask ci` prints the totals after the run, raw and above the start. A tier over its budget is a warning on the pull request, and a tier more than one and a half times over fails the job.

## Editing from a script

Edits across many files are best made by a script, and an agent run may have no choice: some agent sandboxes refuse a shell command that mixes git with a heredoc, a loop, `$(…)` or `--exec`, because they cannot tell what it will do. An edit script that matches exact text breaks the day `rustfmt` wraps a line, and one that fails half way leaves the tree half edited.

- For Rust, `cargo xtask replace FILE --from CODE --to CODE` finds the code by its tokens (white space and comments do not count), refuses a match that is not there exactly once, refuses a `--from` that begins or ends with a comment (the file's comment would stay and `--to`'s be put beside it: match such text with `--text`), and prints the lines it changed. `--from-file` and `--to-file` take the code from files, for code of several lines. Run `cargo xtask fmt` after it.
- For Markdown and any other file, `cargo xtask replace FILE --text --from TEXT --to TEXT` matches the exact text, white space and all, and refuses it when it is there no times or more than once, overlapping times included.
- **For many edits across many files, `cargo xtask replace --script FILE`.** The file holds the edits, each `@@ file: PATH`, then `@@ from`, the lines to find, `@@ to`, what to put there, and `@@ end`; `@@ in: ITEM`, `@@ occurrence: N` and `@@ text` may follow the file's line. A `.rs` file is matched by tokens and any other by its text (`@@ text` makes a Rust file's exact: a comment alone has no tokens). Each edit sees what the edits before it left, and must match there exactly once. If any edit cannot be made, the command names it, with its line in the script, and writes no file; otherwise it writes every file beside itself and then moves it into place. `--dry-run` checks the script and writes nothing.
- **An edit and its checks in one command: `cargo xtask apply SCRIPT`.** The script holds `replace --script`'s edits and two more changes, `@@ write: PATH` (the file's lines, then `@@ end`: a whole file, made or replaced, its parent folders made) and `@@ delete: PATH` on a line of its own. They are made in order, each on what the one before left. Then come the checks, lines of their own, run in the order written: `@@ check: fmt` (rustfmt on the `.rs` files the script touched), `rules`, `links`, `check STEP…` (`cargo xtask check` of those steps, such as `check breaks generated`), `build -p CRATE…` (`cargo check --all-targets`), `clippy -p CRATE…` (warnings denied) and `test ARGUMENT…` (`cargo xtask test` with them, such as `test -p cineris-tui -E 'test(/dialog/)'`). A word with a space in it is quoted.
  - **All or none.** Every change is made in memory first; if one cannot be made, nothing is written and no check runs. When a check fails, the checks after it do not run, and every file the script touched is put back as it was: an edited file gets its old text, a file it made is removed, one it deleted comes back. `--keep` leaves the changes in place to be mended; `--dry-run` says whether every change can be made and which checks would run, and does neither.

  ```text
  # One edit, one new file, then their checks.
  @@ file: xtask/src/stall.rs
  @@ from
  pub(crate) const AT_LEAST: usize = 3;
  @@ to
  pub(crate) const AT_LEAST: usize = 4;
  @@ end
  @@ write: target/scratch/notes.md
  | What | Who | When |
  |---|---|---|
  | A note | a contributor | now |
  @@ end
  @@ check: fmt
  @@ check: links
  @@ check: test -p xtask -E 'test(/stall::/)'
  ```
- **If your agent's sandbox refuses compound shell commands,** keep each call plain. Write a script or an edit to a file with the editor's own write tool and run it as a call of its own (`python3 FILE`, `sh FILE`, `cargo xtask apply FILE`) rather than a heredoc piped into an interpreter or written beside a git command. Put a loop, `$?` or `$(git …)` into the script file. For history, use `cargo xtask branch` (no interactive rebase, no `--exec`) or a script file. And never put a call in the same batch as the call it depends on: if the sandbox refuses one call of a batch, the others still run, against a tree the refused call did not change. After a refusal, check that every edit of the call landed before going on.
- In a Python script, check that each text is there once before replacing any (`assert source.count(old) == 1`), and write the file only when all of them are. Never replace an empty string: it matches between every two characters.
- **When `xtask` does not build,** in the middle of a change to `xtask` itself, `cargo xtask replace` cannot run: cargo builds `xtask` first. Every command of `xtask` that ends well keeps the binary that ran it in `target/xtask-last-good/` (`xtask`, `xtask.exe` on Windows), with a stamp of when it was built and from which commit. **`./x replace …` and `./x links` build `xtask` first and, when that fails, run that copy instead**; anywhere else run it by its path: `target/xtask-last-good/xtask replace FILE --from … --to …` (`target\xtask-last-good\xtask.exe` on Windows, where `cargo xtask` has no way to fall back). Each run of the copy says on standard error when it was built, from which commit, how long ago, and which files of `xtask/` are newer than it, so an old `replace` is never trusted without knowing. It offers `replace` and `links` and nothing else: any other command answers for the code as it was. Make `xtask` build again with `cargo build -p xtask`, and the next command that ends well keeps a new copy.

## After a rebase

A generated file that two branches both changed conflicts as text, and a merge of the text is not the file the merged sources produce. Do not settle it by hand: settle the conflicts in the sources, take either side of each generated file (`git checkout --theirs` or `--ours`), finish the rebase, and run `cargo xtask regen`. It writes every generated file the tree can rebuild, in this order: the open items' index (`docs/outstanding/README.md`, `cargo xtask outstanding`), the protocol's schema and TypeScript declarations (`docs/protocol/`), the configuration's schema and reference (`docs/configuration/`), the reference screens and their text copies (`docs/architecture/tui-look/`), and every snapshot and the hand-written provider fixtures' requests (`CINERIS_UPDATE_SNAPSHOTS`, in the tests of `cineris-tui`, `cineris-llm`, `cineris-index` and `cineris-config`). It needs no network and no key, runs every generator even when one fails, then lists the files that differ from `HEAD`, which are read like code (a snapshot that changed is a change in what a user sees), and runs `links`.

What it leaves out, and why, `cargo xtask regen --list` prints: the tables of `docs/steps/<step>/ci.md` (they are a CI run's measurements: `ci-times RUN --write`, which needs `gh`); the toolbox and recorded provider fixtures (a recording calls a paid API: `record`); the golden transcript (not edited to make a test pass); `Cargo.lock` and the break lists. A test holds the list to the code: every `CINERIS_UPDATE_*` variable that a test reads is either run by `regen` or named there as left to another command, so a new writing mode cannot be forgotten.

**The open items' index never stops a rebase** (since the tenth tooling step). Every open item is a file of its own, and the index of `docs/outstanding/README.md` is written from them by `cargo xtask outstanding`. Git merges the index line by line, keeping both sides' lines (`merge=union` in `.gitattributes`), so two branches that each added an item rebase without a conflict there. What such a merge can get wrong (a line twice, the line of an item whose file is gone, a line lost, lines out of order) `cargo xtask rules` and `links` find, since they compare the whole page with what the command writes. **After a rebase, the one command is `cargo xtask outstanding`**, then commit the index it wrote; `regen` runs it too. Before merging, rebase onto the `main` of the moment and run it, as for any merge: an index that was right on an older `main` may not be on this one.

## Renaming the project

The project was renamed to Cineris in [decision 0038](docs/decisions/0038-the-project-is-named-cineris.md). `cargo xtask rename` made the change, and stays for two jobs: keeping the old names out, and bringing across a branch made before the rename. Its table of old and new names, and its short list of kept files and lines with the reason for each (`cargo xtask rename --kept`), are in `xtask/src/rename.rs` and nowhere else.

- **`cargo xtask rename apply`** renames the working tree: every file's text, then the names of files and folders, after checking that no two paths become one. A text that changed is put back in the form its tools give it: a `Cargo.lock` in cargo's order, a Rust file that was formatted formatted again. A second run changes nothing. `--regen` then runs `cargo xtask regen` and `cargo xtask outstanding`, since the screens' layout depends on the names' widths.
- **`cargo xtask rename --check`** fails on an old name left outside the kept lines. The repo rules run it, so the lint job does.
- **`cargo xtask rename branch BASE..TIP`** brings a branch made before the rename across, without checking anything out. It passes BASE's tree through the same function `apply` uses, as a commit R(BASE) on top of BASE, and then each commit of the range, renamed in its tree, its paths, its message and its identities, with its dates kept, in a line on R(BASE). It writes them with `git fast-import` and points a new branch at the last (TIP's name with `-renamed`, or `--to NAME`). R(BASE) is what `apply` makes of BASE, so a rebase from it replays only the branch's own changes, already in the new names:

  ```
  cargo xtask rename branch BASE..my-branch
  git rebase --onto origin/main <R(BASE), as printed> my-branch-renamed
  cargo xtask regen && cargo xtask outstanding && cargo xtask rename --check
  ```

  The rebase stops only where the branch and `main` changed the same lines since BASE: the hand edits that followed the rename, or anything else merged since. A generated file that stops it is settled as after any rebase ("After a rebase", above). Run the command from a checkout of the renamed `main`, so that the tool that runs is the one the tree was renamed with: `git switch --detach origin/main` in the working tree, or a second checkout under `target/`. BASE is where the branch left `main`, a commit from before the rename: `git merge-base origin/main my-branch`. The same range always gives the same commits, so a second run finds the branch already there; one that holds other commits is replaced only with `--force`, and never while a working tree has it checked out, or is rebasing or bisecting it.

Both `apply` and `branch` refuse a symbolic link (neither renames one, and none is tracked) and a machine where `rustfmt` does not run, since their text would then differ from the text that renamed `main`. `--check` finds the old alias in the forms `apply` renames, and also the labels built on it with no separator, such as a test kit role's mark or a test's environment variable (the two letters of the alias run on into a word, in lower case or in capitals), which `apply` leaves: they are test words, renamed by hand, with `cin` in the old alias's place (as `cintui-signals=` and `CINTUI_SIGNALS_REPORT` were), here and wherever each is read.

## Lints

The lint levels are in the root `Cargo.toml` under `[workspace.lints]`, the lists they use are in `clippy.toml`, and CI turns every warning into an error. The project-specific rules:

- **No `unwrap` or `expect` outside tests**, unless an attribute gives the invariant:

  ```rust
  #[expect(clippy::unwrap_used, reason = "the pattern is a literal and is tested")]
  ```

- **No `println!` or `eprintln!`** outside `cineris-cli` and `xtask`, so nothing writes to the user's terminal by accident.
- **No blocking standard-library calls** (`std::fs`, `std::process::Command`, `std::thread::sleep`). The core runs on one async thread, and a blocking call there stalls every session. Code that is meant to block, because it runs on the blocking pool or the whole crate is synchronous, opts out for the module or the crate and says why:

  ```rust
  #![allow(clippy::disallowed_methods, clippy::disallowed_types, reason = "runs on the blocking pool")]
  ```

- **`unsafe` is denied**, and allowed per module only in `cineris-sandbox` (process trees are there for that reason, [decision 0030](docs/decisions/0030-process-tree-control-in-cineris-sandbox.md)) and `cineris-term`, and in one Windows-only module of `cineris-base` (`private::windows`, which sets access lists and reads a file's reparse tag; [decision 0029](docs/decisions/0029-owner-only-files-on-windows.md)) and one of `cineris-rpc` (`socket::windows`, the named pipe and the check of each client's token; [decision 0037](docs/decisions/0037-the-named-pipe-is-served-from-cineris-rpc-in-one-windows-only-module-that-may-use-unsafe.md)). Every block has a `// SAFETY:` comment. A private file is written with `write_atomic` and `owner_only`, or created with `cineris_base::private::create_new`: no other crate needs `unsafe` for it.
- **An `allow` or `expect` always carries a `reason`.**
- **Public items are documented.**
- **No `todo!`, `unimplemented!` or `dbg!`** on the main branch, and no `std::process::exit`: return an exit code, so the terminal is always restored.

Clippy treats `#[test]` functions and `#[cfg(test)]` modules as tests. Helper functions at the top level of an integration-test file are not covered, so `tests/it/main.rs` and `tests/system/main.rs` start with an `#![allow(…, reason = "…")]` for what test helpers need. `crates/cineris-cli/tests/system/main.rs` is the model.

## The repo rules

`cargo xtask rules` is the part of the static checks that no off-the-shelf tool does. It fails when:

- **A file contains an absolute home-directory path.** Write `~` or a path relative to the repository. Every file is scanned, binary files included.
- **A file holds a name from before the rename to Cineris**, in its text or its path, in any case, or the old alias where it is quoted or named as a command, or a label built on it with no separator: `cargo xtask rename --check`, below. A line that must keep one goes in the tool's list of kept lines, with its reason.
- **A crate depends on a workspace crate it must not**, a heavy third-party crate is used outside its owner crate, or a manifest breaks the conventions above.
- **`docs/dependencies.md` and the manifests disagree.**
- **Anything but a dev-dependency turns on a crate's `test-hooks` or `test-support` feature** (or `cineris-term`'s `pty`, or `cineris-tui`'s `test-kit`): another kind of dependency, a feature of the crate itself (`default` above all), or a feature of another crate that passes it on. The feature holds code for tests, and must not be in a build of the product.
- **A link inside the repository leads nowhere:** a relative link in a Markdown file to a file that is not there, a `#fragment` that is no heading of the page it names, or a `docs/….md` path in prose, configuration or a code comment that names no file. A heading's name is its text in lower case, without punctuation, with a `-` for each space.
- **A step's record or an open item is not where the others expect it:** anything in a folder of `docs/steps/` but `answered.md`, `ci.md` and parts' records (`<group>/<part>.md`, each linked, itself or its group's folder, from the step's `answered.md` or `ci.md`), a folder name that is not a step's (`07`, `13a`) or a hyphenated name, a first line that is not the heading its kind has, a file of `docs/outstanding/` that is not an item's sentence, its fields (`- Area:`, `- Owner:`, any `- Flaky:`) and the table's header and one row, or an index in `docs/outstanding/README.md` that is not what `cargo xtask outstanding` writes from the items.
- **`cineris-protocol` depends on anything but serde and serde_json**, other than schemars behind its `schema` feature.
- **A workflow uses an action that is not pinned by commit hash**; a workflow that runs on pull requests uses a secret or a token that can write; or any job other than the `report` job of a non-blocking workflow has a write permission, or that job has one other than `issues: write`.

`cargo xtask check` fails in its `breaks` step when a break in `mutations/` no longer finds the code it changes: the message names the list and the line. Change the break to the code as it is now, or delete it if the rule it breaks is gone ([docs/testing-and-ci.md](docs/testing-and-ci.md), "Breaking the code on purpose").

`cargo xtask check` also fails, in its `generated` step, when the files in `docs/protocol/` are not what the protocol types generate. Run `cargo xtask schema`. The files in `docs/configuration/` are checked the same way by a test in `cineris-config`, on both OSes. Run `cargo xtask gen`.

## Dependencies

No new dependency without a row in [docs/dependencies.md](docs/dependencies.md) saying why it is here, what else was considered and what it costs. That page has the steps. The policy is in [docs/architecture/01-workspace.md](docs/architecture/01-workspace.md): prefer the standard library, default features off, nothing that needs build tools beyond the platform's compiler, permissive licences only.

`Cargo.lock` is committed. CI builds with `--locked`.

## Continuous integration

Four jobs block a pull request: `lint-linux`, `lint-windows`, `test-linux` and `test-windows`. Each must finish in under 5 minutes, and on a pull request a job that takes longer fails by its timeout (a test job on `main` has 12, so that it can rebuild a cold cache and save it). The layout and the measured times are in [docs/testing-and-ci.md](docs/testing-and-ci.md).

- A step is not done until its tests pass on both OSes.
- Nothing that blocks uses a secret or a paid API.
- Only the main branch writes the build caches. A pull request reads them. `cargo xtask rules` refuses a workflow step that could write one from any other run.
- On Windows the cache holds the main branch's build of the workspace's own crates too, and a pull request rebuilds only the crates it changes and what depends on them: the checkout's files are dated against the commit that build was made from, and anything that cannot be worked out is rebuilt whole ([docs/testing-and-ci.md](docs/testing-and-ci.md), "The workspace's own build, cached").
- A pull request that touches only `docs/**` and `*.md` skips the Rust build and tests. Its four checks still report, and the Linux lint job still runs the repo rules, because those rules are mostly about documentation. The generated files in `docs/protocol/` and `docs/configuration/` are the exception: they count as code.
- Two workflows do not block: `main`, after each merge, and `nightly`. When one fails it opens an issue labelled `ci-failure`, or adds a comment to the open one. Close the issue when the cause is fixed.
- A change that makes the blocking checks slower than their budget moves the slow part to a non-blocking tier, or raises it for a decision. The limit is not loosened quietly.

## Writing the docs

- **An "As built" record is a list, one sentence a line** (since the ninth tooling step). A Markdown table's row is one line, so two sessions that add a sentence to the same part of a step's record change the same line, and their pull requests conflict. A list item per part, with each sentence on a line of its own (a sentence break that Markdown joins back into one paragraph), puts two such changes on different lines. The example is "As built in step 17" in [docs/architecture/14-adaptation.md](docs/architecture/14-adaptation.md):

  ```markdown
  - **The shared file**:
    `<workspace>/.cineris/profile.toml`, `version = 1`: `[[entry]]` tables of a kind, a text, and optionally evidence.
    It is read as a regular file in a `.cineris` that is a directory of the workspace itself.
  ```

  An older table is turned into a list when a step next adds to it. A table of short cells that do not grow (budgets, a comparison of figures) stays a table.

- **A step in several pull requests: a record for each part** (since the tenth tooling step). Two parts of one step that both appended to the step's `answered.md` changed the same lines, and the second to merge met a conflict on every rebase. Each part writes its own record instead, `docs/steps/<step>/<group>/<part>.md`: the group names a stage of the step in hyphenated words (`after-review`), and the part names the part (`pr2-trust`). It begins with a heading of its own (`# Step 23, after the review: trust`), and may hold the part's CI times beside its answers. **The step's own session links the group's folder once**, from the step's `answered.md` (or `ci.md`), before the parts start, so that reading the step's records in order reaches every part and no part adds a line to a shared file:

  ```markdown
  ## After the review

  Each pull request of this stage has its record in [after-review/](after-review/).
  ```

  A link to a part's own file counts too. `cargo xtask rules` and `links` accept such a file only in a step's folder, under hyphenated names, with a heading on its first line, and linked, itself or its group's folder, from the step's `answered.md` or `ci.md`. [docs/steps/README.md](docs/steps/README.md) has the layout.

## Ground rules

These hold for every change. They are the "Rules for every step" of the [build plan](docs/build-plan.md).

1. **Original work.** Other agent harnesses may be studied to understand a problem. Nothing is copied from them: no code, structure, names, prompt text, UI or configuration format. No dependency that is a port or extract of their code.
2. **Speed and efficiency come first**, in the product and in the tooling.
3. **No personal details** in commits, docs, code, configuration or fixtures: no names, no email addresses, no absolute home-directory paths.
4. **Linux and Windows together.**
5. **The docs are the specification.** Keep them true.

## Pull requests

- Keep a pull request to one subject, and say in its description what was built, what was measured, and what is left open.
- Run `cargo xtask ci` first.
- For a change to the loop, or to code that starts processes or crosses threads, run `cargo xtask stress` for the crate first.
- Commit messages say what changed and why, in the imperative: "Add the session index", not "Added" or "Adds".
