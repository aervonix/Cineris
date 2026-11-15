# Still outstanding

What the owner or a later step still has to settle: one file per item, each a one-row table of the item, who settles it, and when. Together they are the list that "Still outstanding" in [build-plan.md](../build-plan.md) points to.

- **Leaving something open:** add a file, named for the item in lowercase words joined by hyphens, with the table's header row `| Item | Who | When |`, and a line for it in the index below, under the crate or area it is about.
- **Settling an item:** delete its file and its line in the index, and record the answer in the settling step's `steps/<step>/answered.md`.
- **Changing an item:** edit its file.
- **An item several steps share** stays one file, and each step that does its part leaves the file alone and records that part in its own `steps/<step>/answered.md`. Only the step that finishes the item deletes its file, so parallel steps never edit it.

One file per item, and not one per step, because a step settles items that other steps left open: two steps that settle different items then touch different files.

## The items, by crate or area

One line an item: what it covers and who settles it. It is the place to look first when a review asks whether a finding is already known. `cargo xtask links` (and `rules`) fails when an item's file is not listed here exactly once, so the index cannot go stale; a parallel step that adds an item adds its line, and a rebase that meets another step's line keeps both.

### cortex-agent

- [a-background-summary-past-its-turn-cannot-be-interrupted](a-background-summary-past-its-turn-cannot-be-interrupted.md): `interrupt` on an idle session does not stop a background summary request, which runs on and goes uncounted. Owner: step 23 or step 27.
- [a-decision-made-before-an-earlier-call-ran](a-decision-made-before-an-earlier-call-ran.md): Policy decides a call when planned, so a later call can pass through a link made earlier. Owner: a small step of its own, agreed with the owner.
- [a-masked-instruction-notice-is-told-again](a-masked-instruction-notice-is-told-again.md): A masked instruction-file notice hashes differently on reload, so the file is announced as changed after every resume. Owner: a later step on instruction files, or step 32.
- [a-prompt-past-the-trigger-on-a-small-window](a-prompt-past-the-trigger-on-a-small-window.md): On a small-window model the first estimate exceeds `context.compact_at`, so every turn ends `compaction_failed` unsent. Owner: a later step on local models, or step 33.
- [a-summary-stopped-or-left-is-not-counted](a-summary-stopped-or-left-is-not-counted.md): Usage of a summary request stopped mid-run, or finishing after its session closed, is in no usage record. Owner: step 16.
- [automatic-compaction-off-and-the-overflow-fallback](automatic-compaction-off-and-the-overflow-fallback.md): With `context.auto_compact = false`, the overflow fallback still compacts once instead of ending the turn `compaction_needed`. Owner: the owner, then a later step; before step 23.
- [session-compact-after-a-turns-last-request](session-compact-after-a-turns-last-request.md): `session/compact` during a turn's last request waits for the next turn instead of running a compaction-only turn. Owner: step 23 or step 27.
- [spans-in-the-agent-loop](spans-in-the-agent-loop.md): The `turn`, `request` and `tool` tracing spans are not opened, since `cortex-agent` does not link `tracing`. Owner: step 16, or step 33 at the latest.
- [stopping-at-a-task-lease-edge](stopping-at-a-task-lease-edge.md): At a task lease's edge the loop should stop and report, then fall back to the standing lease. Owner: step 37.
- [the-context-record-of-an-overflowed-request](the-context-record-of-an-overflowed-request.md): A request refused as too long leaves no `StatEvent::Context` record, so the wrong estimate is not stored. Owner: step 16 or step 44.
- [the-token-counting-endpoint-near-the-limit](the-token-counting-endpoint-near-the-limit.md): The provider's token-counting endpoint is never asked near the limit; needs a `Protocol` method and loop call. Owner: a later step that touches the budget, or step 33.

### cortex-base

- [log-crate-bridge](log-crate-bridge.md): The logging subscriber has no bridge for dependencies that log through the `log` crate at `warn` and above. Owner: step 32, or the first step whose dependency needs it.

### cortex-cli

- [headless-details-left-open](headless-details-left-open.md): Six small gaps in headless mode: dropped early notices, error records, `files_changed` matching, Windows close, untested signals. Owner: step 27 or step 32.
- [reading-a-key-at-a-terminal](reading-a-key-at-a-terminal.md): `cortex auth login` refuses a terminal because reading a key without echo needs the terminal layer. Owner: step 23 or step 25.
- [shell-completions](shell-completions.md): Completions should come from an `xtask` command over the `FLAGS` table; the table exists, the command does not. Owner: step 34.
- [trust-of-a-profile-a-project-defines](trust-of-a-profile-a-project-defines.md): `cortex permissions trust` takes no `--profile`, so a project-defined profile's rules read as changed. Owner: the owner, with step 23.

### cortex-config

- [a-setting-that-restricts-reads](a-setting-that-restricts-reads.md): No setting fills the launcher's read-restricting policy (`Reach::Only`), so every sandboxed session can read everything. Owner: the owner, then step 32 unless sooner.
- [approval-time-limit-setting](approval-time-limit-setting.md): A setting for how long an approval may wait, in the reserved `[rpc]` section, feeding `CoreOptions::approval_timeout`. Owner: step 27.
- [secret-service-backend](secret-service-backend.md): The Linux Secret Service credential backend needs a non-blocking CI job against a real keyring daemon. Owner: a step the owner places.
- [settings-a-later-step-declares](settings-a-later-step-declares.md): Reserved configuration sections, including `[models]` and `[http]`, still need their keys declared and their project rules set. Owner: steps 5, 13, 15, 16 and others, each for its own section.

### cortex-core

- [a-closed-view-can-still-answer](a-closed-view-can-still-answer.md): After `session/close` a client stays among those an approval request went to, so its answer is still taken. Owner: the owner; before step 27.
- [an-observer-that-ends-a-session](an-observer-that-ends-a-session.md): An observer's `session/close` or disconnect can end a session that has no client left that may drive it. Owner: the owner; before step 27.
- [config-reload-through-the-core](config-reload-through-the-core.md): `config/reload` answers `not_available`; what a reload applies to a running session, key by key, is not wired. Owner: step 23.
- [continue-chooses-the-newest-session](continue-chooses-the-newest-session.md): `session/list` orders by session id, so `--continue` takes the newest created, not the last used. Owner: step 23.
- [log-level-from-configuration-in-headless-mode](log-level-from-configuration-in-headless-mode.md): A headless run starts the log before configuration is read, so `log.level` is ignored. Owner: step 23 or step 27.
- [secret-filter-on-what-clients-are-sent](secret-filter-on-what-clients-are-sent.md): Events, approval requests and in-progress items sent to clients skip the secret filter applied to disk writes. Owner: the owner; before step 27.
- [shutdown-and-a-session-being-put-together](shutdown-and-a-session-being-put-together.md): `Core::shutdown` returns before a session still being put together has ended itself. Owner: step 27.

### cortex-llm

- [a-balance-in-the-usage-view](a-balance-in-the-usage-view.md): Whether `usage/report` carries a money balance from the OpenRouter, DeepSeek and Venice.ai usage endpoints, including non-dollar units. Owner: step 16.
- [a-price-that-depends-on-the-time-of-day](a-price-that-depends-on-the-time-of-day.md): The catalog's single rate card records DeepSeek's off-peak requests at the peak price, an upper bound. Owner: the owner, with step 16.
- [account-in-the-replay-rule](account-in-the-replay-rule.md): Whether a request carries its account, so a provider tying reasoning to an account never gets another's. Owner: the first step that adds such a provider, or step 53.
- [fuzz-targets-for-later-decoders](fuzz-targets-for-later-decoders.md): Each later provider protocol's decoder needs a fuzz target in `fuzz/` and a line in `xtask/src/outside.rs`. Owner: steps 31, 35 and 36.
- [the-prefix-check-and-image-references](the-prefix-check-and-image-references.md): The debug prefix check skips requests holding a stored image reference; a protocol should write a placeholder. Owner: step 33, or the step that next touches images.

### cortex-policy

- [cd-readings-the-analysis-does-not-share](cd-readings-the-analysis-does-not-share.md): The command analysis reads `cd` differently from bash (mode-000 directories) and probably from Git for Windows. Owner: step 32, with a Windows run.
- [false-asks-from-the-pattern-and-word-checks](false-asks-from-the-pattern-and-word-checks.md): The pattern and word checks ask or refuse needlessly for several link, quoting, set and heredoc forms. Owner: step 32.
- [files-over-the-undo-limit](files-over-the-undo-limit.md): Editing tools refuse files over 16 MiB; asking instead needs an engine question no rule can lift. Owner: step 23.
- [git-forms-the-command-analysis-misreads](git-forms-the-command-analysis-misreads.md): The command analysis misses `git config` run through an alias, and misreads `git diff` outside a repository. Owner: step 32.
- [link-makers-the-command-analysis-does-not-see](link-makers-the-command-analysis-does-not-see.md): The command analysis misses links made by `env ln`, abbreviated `cp` options, and `cp -a`, `cp -R`, `mv`. Owner: step 32.
- [names-the-path-resolver-refuses](names-the-path-resolver-refuses.md): Windows names the path resolver cannot read (trailing dot, stream, provider path) go unchecked for off-table commands. Owner: step 32.
- [pattern-matching-corners](pattern-matching-corners.md): `NamePattern` lower-casing and user shell options (`dotglob`, `globstar`, `nocaseglob`) make the pattern analysis match differently from the shell. Owner: step 32.
- [rust-toolchain-is-not-a-protected-path](rust-toolchain-is-not-a-protected-path.md): `rust-toolchain.toml` can be written by a confined command and chooses the compiler the user's next `cargo` runs. Owner: the owner, then step 32.
- [words-of-unmodelled-lines-the-check-misreads](words-of-unmodelled-lines-the-check-misreads.md): The word check misreads split command words, program text, tricky `cd` targets and over-bound lines, at `autonomous`. Owner: step 32.

### cortex-sandbox

- [a-refused-socket-the-result-does-not-name](a-refused-socket-the-result-does-not-name.md): A Unix socket refused by the filter, network on, gives a bare `EPERM` and no sandbox explanation. Owner: step 32.
- [a-session-of-its-own-in-the-sandbox](a-session-of-its-own-in-the-sandbox.md): `setsid` is faked in the sandbox, breaking terminal-making programs; a subreaper and cgroup kill would allow it. Owner: step 32.
- [confined-git-and-lfs-filters](confined-git-and-lfs-filters.md): The confined read-only git cannot run a git-lfs filter that must write, leaving change detection and undo incomplete. Owner: step 32.
- [datagrams-from-a-socket-pair](datagrams-from-a-socket-pair.md): With the network off, a datagram `socketpair` can `sendto` a local datagram socket by path, such as journald's. Owner: step 32.
- [files-git-runs-that-the-protected-list-does-not-name](files-git-runs-that-the-protected-list-does-not-name.md): Files git's configuration points to (hooks path, fsmonitor, filters, includes) and `sandbox.write` roots are not protected or watched. Owner: step 32.
- [modes-and-times-outside-the-write-roots](modes-and-times-outside-the-write-roots.md): A confined command can change modes and times of user-owned files anywhere, since Landlock ignores `chmod` and `utimensat`. Owner: step 32.
- [path-forms-on-windows-the-helper-does-not-reach](path-forms-on-windows-the-helper-does-not-reach.md): The protected-path watch and the base state's git parsing handle mixed Windows path forms without `same_location`. Owner: step 32, or the step that brings the sandbox to Windows.
- [the-cost-of-the-look-at-protected-paths](the-cost-of-the-look-at-protected-paths.md): Walking the workspace around each confined command for protected names costs about 47 ms in a large tree. Owner: step 33.
- [the-last-writes-of-a-killed-command](the-last-writes-of-a-killed-command.md): The session sweep that ends a command's tree can lose a race with a fast-forking process. Owner: step 32.

### cortex-session

- [an-undo-after-an-incomplete-look](an-undo-after-an-incomplete-look.md): An undo after an incomplete change look does not say what it missed; the checkpoint lacks the flag. Owner: a later step, else step 32.
- [hashing-large-files-at-a-rounds-end](hashing-large-files-at-a-rounds-end.md): A changed file is hashed whole at round end and on rewind, uncapped, at 0.4 s a gibibyte. Owner: step 33.
- [private-keys-in-text-artifacts](private-keys-in-text-artifacts.md): Text artifacts are redacted line by line, so a PEM private key is masked on one line only. Owner: step 32.

### cortex-term

- [a-paste-that-stalls](a-paste-that-stalls.md): A bracketed paste with no end marker for 2 seconds is split, and the remainder arrives as keys. Owner: step 25.
- [backspace-sent-as-ctrl-h](backspace-sent-as-ctrl-h.md): The decoder reads byte 0x08 as `Ctrl+Backspace`, so a terminal whose Backspace sends `^H` deletes a word. Owner: step 25.
- [input-decoder-corners](input-decoder-corners.md): `sys::unix::wait` and `sys::windows::wait` restart their timeout after an interrupted call, stale wake or key release. Owner: step 32.
- [pastes-without-bracketed-paste](pastes-without-bracketed-paste.md): Pastes in terminals without bracketed paste are not detected, so a pasted line break sends the prompt. Owner: step 25.
- [terminal-test-gaps](terminal-test-gaps.md): Terminal-layer tests lack styled, emoji and zero-width scripts, resize re-wrap, per-signal handlers, real SIGTSTP and nested-child cleanup. Owner: steps 22 and 23.

### cortex-tools

- [cmd-searches-the-current-directory](cmd-searches-the-current-directory.md): `cmd.exe` finds a program in the working directory before `PATH`, so a workspace `git.bat` runs as `git`. Owner: the owner.

### cortex-tui

- [ambiguous-width-glyphs](ambiguous-width-glyphs.md): Fourteen East Asian ambiguous-width glyphs, box edges among them, break box layout in terminals that draw them wide. Owner: step 23, with the owner; before step 25.
- [editor-cost-on-very-large-text](editor-cost-on-very-large-text.md): The prompt editor lays out all its text on every key; a shortened `@` query refilters all paths. Owner: step 33.
- [external-editor-test-gaps](external-editor-test-gaps.md): Parts of the external editor (umask, ConPTY modes, failed hand-over, `.cmd` editors, interrupt guard) are untested. Owner: step 32.
- [file-completion-on-a-large-workspace](file-completion-on-a-large-workspace.md): Filtering an `@` menu over about 100,000 paths on the key's path can miss the idle key budget. Owner: step 23.
- [highlighted-code-drawn-again](highlighted-code-drawn-again.md): `Markdown::render` draws code plain when a finished answer is redrawn at another width. Owner: step 25.
- [highlighting-the-line-in-progress](highlighting-the-line-in-progress.md): The streaming code line is drawn plain until it commits; colouring it a frame behind is the alternative. Owner: the owner, then step 23; before step 25.
- [signals-and-a-graceful-shutdown](signals-and-a-graceful-shutdown.md): Terminal signal handlers end the process without flushing the session log, and writes after an emergency restore fail. Owner: step 23.
- [the-default-editor-on-windows-11](the-default-editor-on-windows-11.md): Windows 11's tabbed Notepad may exit at once, so `Ctrl+G` removes the prompt's folder while it is open. Owner: the owner, by hand, after step 23.
- [the-input-box-while-a-turn-runs](the-input-box-while-a-turn-runs.md): `view::input` has one placeholder; the reference screens need two more states, a turn running and a focused panel. Owner: step 23; before step 25.

### cortex-testkit

- [timeout-report-timing-on-windows](timeout-report-timing-on-windows.md): The `watch` timeout report counts from the first wait off Linux; Windows takes the process tree from PowerShell. Owner: a later tooling step.

### The tooling and CI

- [actionlint-in-the-checks](actionlint-in-the-checks.md): The workflows are not linted by actionlint, which the pinned install action cannot supply as a checksummed binary. Owner: the owner.
- [an-ab-run-ends-at-a-failing-test](an-ab-run-ends-at-a-failing-test.md): `cargo xtask ab` stops at the first round with a failing test and loses the rounds already measured. Owner: a later tooling step.
- [cortex-llm-optimised-in-the-ci-build](cortex-llm-optimised-in-the-ci-build.md): Whether to build `cortex-llm` at `opt-level = 1` in the `ci` profile to speed its integration tests. Owner: the owner.
- [perf-in-the-main-workflow](perf-in-the-main-workflow.md): `cargo xtask perf` is not yet run in the `main` workflow on both systems with stored baselines. Owner: a tooling step, or step 33.
- [the-start-cost-of-a-tier-report](the-start-cost-of-a-tier-report.md): The tier report's start-cost subtraction hides a cost shared by all test processes, with no warning. Owner: a later tooling step.
- [timeout-report-for-tests-without-the-test-kit](timeout-report-for-tests-without-the-test-kit.md): A test using none of the test kit's waits gets no timeout report, since nextest lacks pre-timeout scripts. Owner: a later tooling step.
- [typed-values-in-composite-actions](typed-values-in-composite-actions.md): The repo rule against event values in scripts reads `.github/workflows/` only, not composite actions under `.github/actions/`. Owner: a later tooling step.

### Windows

- [key-latency-over-conpty](key-latency-over-conpty.md): A key through ConPTY on a hosted Windows runner takes 16 ms at the median, against 2 ms. Owner: the owner; before step 23.
- [windows-start-overhead-in-release](windows-start-overhead-in-release.md): The release-build overhead of starting a shell command on Windows is unmeasured, though `try` can measure it. Owner: step 33, or sooner on any Windows machine.

### Other

- [quickjs-with-the-windows-toolchain](quickjs-with-the-windows-toolchain.md): Confirm QuickJS builds in the real extension crate with the Windows MSVC toolchain, or switch to Lua. Owner: step 28, in its first hour.
