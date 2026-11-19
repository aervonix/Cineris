# The twelfth tooling step, first pull request: the repo rules and the test kit

Built on `origin/main` at 6806291f.

## What was built

### Public text: no word of the maintainers' private setup

- `xtask/src/rules/public_words.rs`, run by `rules` and `links` ([testing-and-ci.md](../../../testing-and-ci.md#public-text)). It reads every tracked Markdown file and every workflow for the phrases of `xtask/public-words.txt`, and fails with the file and line, the word and the public alternative.
- **The list**, from what pull request #106 removed from the docs and did not put back. The data file spells each word; this record, which the rule reads, names them by what they are:
  - the maintainers' script for heavy commands, in every form the docs used: its name in backticks, and its name before `--`, `check`, `test`, `busy`, `throttle` and `ab`;
  - their script that waits for CI runs;
  - the two machines named by their kind of computer: the reference machine as a kind, with the article, and the second machine as a kind;
  - the two phrases for working on one's own machine, with and without "at";
  - the session that coordinates the build steps, in lower case only (the product's Coordinator has a capital);
  - the agent tool's guard on shell commands;
  - the two locks of the maintainers' shared machine;
  - their private folder under `target/`;
  - the phrase for a session that builds a step.
- **The `allow` lines** (a longer phrase that is a product use):
  - the number of the performance budgets' Desktop column;
  - the measurement pages' description of the second machine's class (an 8-thread portable).
- **Left off the list, and why:**
  - "worktree", "WSL", "sub-agent" and the names of other agents: the product uses them;
  - the machines' CPU models: the measurement pages publish them on purpose.
- **Not read:** Rust comments, which hold about 45 of the words (most in `xtask/src/like_runner.rs`). The one printed message that named a private command, the test kit's corpus message, now says the plain command.
- **Step records are public text:** the folders written before this step are named on `history` lines and keep their words; this step's records and every later one are held to the rule.
- **`AGENTS.md` takes no `allow`.**
- **`.worktreeinclude` lists only ignored files:** `xtask/src/rules/worktree_include.rs`. Each entry must be matched by the root's `.gitignore` and be no file git tracks; it reads `.gitignore` itself, so it works in a copy of the tree with no repository.
- **Found on `main`:** one hit, the second machine's class in the measurement pages, which is the product's own use (`allow`).

### A pty test sends keys through `send_key`

- `xtask/src/rules/pty_keys.rs` ([testing-and-ci.md](../../../testing-and-ci.md#keys-through-send_key)). Every literal handed to `.send(…)` in the crates is decoded and fails on an escape sequence that is a key. The mouse's SGR reports, bracketed paste, focus reports and a terminal's answers pass. A test of the decoder itself marks the line `// raw key: WHY`.
- **`send_key` learned `escape`** (with its character in the win32 record, as Windows Terminal sends it) **and the modifiers** `shift+`, `ctrl+` and `alt+`: xterm's modifier parameter, or the record's control keys' state.
- **Found on `main`, and fixed:** five raw keys in `cineris-cli`'s `tests/system/tui.rs`:
  - two lone Escapes in the interruption test;
  - PageUp, PageDown and an Escape in the memory measurement.

  `cineris-term`'s `sys_keys_arrive_with_their_modifiers` sends raw bytes on purpose, a test of the decoder, and its three lines are marked. The mouse sequences in `tui.rs` are not keys.

### Only a `sys_` or `perf_` test starts a process

- `cineris_base::hook::command_from_a_test`, called by `cineris-tools`' runner before it starts a command. With the `test-hooks` feature, which every test build has, under nextest, a test whose name's last segment starts with neither is stopped: the message goes to its standard error, then the process aborts. An error would be a failed command, which a test of a turn can pass with. The message is in [testing-and-ci.md](../../../testing-and-ci.md#test-tiers).
- **On `main` it stops no test:** the whole workspace's 4,931 tests passed with it on Linux. A planted test not named `sys_` was stopped, with the message, and a planted `sys_` test was not.
- **A refusal of every start** (not only the runner's) was measured too: it also stops none on Linux. See [answered.md](../answered.md).

### The timeout report names the test's own children

- `cineris_base::hook::spawned` and `reaped` keep, in each test's process, the children the harness and the test kit start: the sandbox's process tree (the runner's commands), the harness's git, a pseudo-terminal's program and a `Roll`'s children. For each they keep the process id, the command line, when it started and, where the code that started it waits for it, when it exited. Without the feature they are empty functions, and the command line is never made.
- The report (`cineris_testkit::watch`) lists them after the waits. For each child it gives the id, how long ago it started, what became of it and its command line. What became of it is one of:
  - exited after so long, and its wait returned;
  - still running;
  - exited and not waited for (a zombie, read from `/proc` on Linux);
  - gone with no wait seeing it exit (on Windows, asked of the PowerShell that reads the tree).

  A child gone unseen, while a thread still waits for its end, is a missed wake; a child still running is a late exit.
- **Tested** by a unit test of the text and by the hanging-on-purpose system test, which now also starts a child that ends before the report. That test is ignored in the blocking checks; the nightly workflow runs it on both systems.

### The doc note

[testing-and-ci.md](../../../testing-and-ci.md#test-tiers): a system test that starts more than about 20 processes is Unix-only, or batches them (70 gits in one test took 6.2 s on Windows).

## What it costs the blocking jobs

The three rules run inside `rules` and `links`, in the lint job. Locally, `cargo xtask rules` took 0.8 s with them, as before. The lint-linux times on the runner, before and after, are under "CI" below.

## Breaks

[`mutations/tooling-12-a.breaks`](../../../../mutations/tooling-12-a.breaks): one break per rule of the three repo rules, `send_key`'s new names, the process rule and the report's children.

## Open

- Rust comments still hold the private words; a rule for them would need comments told apart from the product's own uses, and is left.
