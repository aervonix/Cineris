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

- `xtask/src/rules/pty_keys.rs` ([testing-and-ci.md](../../../testing-and-ci.md#keys-through-send_key)). It reads the crates' test code (files under a `tests` folder, and test modules in files of their own). Each call of a sender (`.send(…)`, or the tests' helper `type_text(…)`) has its literals, the file's constants and arrays of bytes decoded, and fails on an escape sequence that is a key. The mouse's SGR reports, bracketed paste, focus reports and a terminal's answers pass, as does product code, whose output has the same bytes. A test of the decoder itself marks the line `// raw key: WHY`.
- **`send_key` learned `escape`** (with its character in the win32 record, as Windows Terminal sends it) **and the modifiers** `shift+`, `ctrl+` and `alt+`: xterm's modifier parameter, or the record's control keys' state.
- **Found on `main`, and fixed:** five raw keys in `cineris-cli`'s `tests/system/tui.rs`:
  - two lone Escapes in the interruption test;
  - PageUp, PageDown and an Escape in the memory measurement.

  `cineris-term`'s `sys_keys_arrive_with_their_modifiers` sends raw bytes on purpose, a test of the decoder, and its three lines are marked. The mouse sequences in `tui.rs` are not keys.

### Only a `sys_` or `perf_` test starts a process

- `cineris_base::hook::started`, which every place that starts a process calls, stops a test whose name's last segment starts with neither `sys_` nor `perf_`. It works under the `test-hooks` feature, which every test build has, and under nextest. The message goes to the test's standard error, then the process aborts: an error would be a failed command, which a test of a turn can pass with. The message is in [testing-and-ci.md](../../../testing-and-ci.md#test-tiers).
- **First built in `cineris-tools`' runner alone**, before it starts a command. On the maintainers' answer it moved into `started`, once that was measured to stop no test either: it now covers the harness's git, the pseudo-terminal and the test kit's children too.
- **On `main` it stops no test:** the workspace's 4,932 tests passed with it on Linux. A planted test not named `sys_` was stopped, with the message, and a planted `sys_` test was not. `hook::refusal_tests::sys_a_test_not_named_sys_is_stopped_at_its_first_start` checks that end to end.
- **A version probe is not refused** (`hook::probed`): the harness starts it on a thread of its own, which a test may end before or after it, and a refusal there would make the test pass or fail by that race (the review's finding 6).

### The timeout report names the test's own children

- `cineris_base::hook::spawned` and `reaped` keep, in each test's process, the children the harness and the test kit start, for the timeout report. The starts marked are the sandbox's process tree (every command of the runner), the harness's git, a pseudo-terminal's program and a `Roll`'s children.
- For each child they keep the process id, the command line and when it started. The tree's starts are marked `spawned_tracked`, and the tree marks each exit when its wait returns. Without the feature these are empty functions, and the command line is never made.
- The report (`cineris_testkit::watch`) lists the children after the waits: the id, how long ago it started, what became of it, and its command line. What became of it is one of:
  - exited after so long, and its wait returned;
  - still running;
  - exited and not waited for (a zombie, read from `/proc` on Linux);
  - gone (on Windows, asked of the PowerShell that reads the tree).

  For a tracked child that is gone while its wait has not returned, the report says so: beside a thread still waiting for that end, it is a missed wake. A child still running is a late exit.
- **Tested** by:
  - a unit test of the text;
  - the sandbox's `sys_a_command_runs_and_says_how_it_ended`, which checks that the tree keeps its command and marks its exit;
  - the harness's git test, which checks that git is kept with the command line the confinement started;
  - the hanging-on-purpose system test, which now also starts a child that ends before the report. That test is ignored in the blocking checks; the nightly workflow runs it on both systems.

### The doc note

[testing-and-ci.md](../../../testing-and-ci.md#test-tiers): a system test that starts more than about 20 processes is Unix-only, or batches them (70 gits in one test took 6.2 s on Windows).

## The pre-push review

One independent review of the whole diff, then one of the changes since. Each finding below, and what was done:

| # | Finding | Verdict | Fix, and its test or break |
|---|---|---|---|
| 1 | The report called every child it had not seen reaped "gone, with no wait seeing it exit", a missed wake, though only the sandbox's tree marks its waits; on Windows a PowerShell with no answer made every child "gone" | Confirmed | Starts are marked tracked or not (`spawned_tracked`); only a tracked child is said to be gone with its wait not returned. No answer from PowerShell is "not asked". `reaped` marks the tracked record of a child kept twice (the sandbox test caught a roll's record taking the mark). Tests: the text's unit test, the sandbox and git tests; breaks `children-*` |
| 1b | Process ids are reused (soon on Windows), so a gone child can be taken for a running one | Plausible | Said in the report's docs; comparing start times is left |
| 2 | Public words: plurals, a phrase broken across lines or with two spaces, a few more forms, words in capitals of a `cased` entry, and overlapping hits counted twice | Confirmed | A paragraph is read as one line with its white space collapsed; an `s` after a word is taken; the machine word matches in any case, with `allow` for the budgets' column; four forms added; overlapping hits are one. Test `a_plural_a_broken_line_and_two_spaces_are_found`; breaks `public-words-plural-missed`, `-lines-not-joined`, `-overlapping-hits-twice` |
| 3 | pty keys: other spellings of the Escape, byte arrays, constants and the `type_text` helper passed; an answer's string swallowed the rest of the literal; product code that writes output was flagged | Confirmed | All read; a string ends at its BEL or ST; only test code is read. Tests in `pty_keys::tests`; breaks `pty-keys-*` |
| 4 | `.worktreeinclude`: a folder-only pattern took a file of its name; a wildcard entry skipped the tracked check; a tree with no repository listed the private notes as tracked, and read them as public text | Confirmed | Each fixed; with no repository the rule is not run and the ignored files are not read. Tests in `worktree_include::tests`; breaks `worktree-include-*` |
| 5 | The records still described the runner's refusal | Confirmed | Rewritten |
| 6 | A version probe on a thread of its own could stop a test, or not, by a race | Plausible | The probe is not refused, and the docs say why |
| 7 | No test reached the marks at their real call sites; several branches had no break | Confirmed | The sandbox and git tests check them; 19 breaks added |

The review also checked, and found right:
- the key encodings against xterm's modifier parameter and the win32 record format;
- line endings and path separators;
- the PowerShell script's inputs, which are numbers only;
- the cost to the blocking jobs.

## What it costs the blocking jobs

The three rules run inside `rules` and `links`, in the lint job. Locally, `cargo xtask rules` took 0.8 s with them, as before. The lint-linux times on the runner, before and after, are under "CI" below.

## Breaks

[`mutations/tooling-12-a.breaks`](../../../../mutations/tooling-12-a.breaks): one break per rule of the three repo rules, `send_key`'s new names, the process rule and the report's children, and one for each gap the review found. The first run, of 19 breaks, caught 17. Of the two it flagged:
- the `reaped` mark's test now marks the earlier of two children, which the break could not tell apart;
- the mouse's exemption has a second guard (a key never ends in `M`), so its break is `expect: survives`.

The run after the review's fixes had 38 breaks: 34 did what the list expects, and four were flagged.
- Two named a sandbox test without its binary (`cineris-sandbox::it`). Once named rightly, both were caught.
- The whole-word break is now caught by the plural test: the overlap merge hides it from its first test.
- The tree-or-roll break was caught only when the roll kept the child after the tree did, which is a race. A unit test in `cineris-base` now keeps a child both ways round, and catches it every time.

All 38 now do what the list expects.

## Open

- Rust comments still hold the private words; a rule for them would need comments told apart from the product's own uses, and is left.
