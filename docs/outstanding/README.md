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

- [a-background-summary-past-its-turn-cannot-be-interrupted](a-background-summary-past-its-turn-cannot-be-interrupted.md): `interrupt` on an idle session does not stop a background summary request, which runs on and goes uncounted. Owner: step 32.
- [a-tool-that-makes-links-beside-other-calls](a-tool-that-makes-links-beside-other-calls.md): An extension's tool that declares itself read-only or keyed and makes a link runs beside later calls, which were decided before it ended. Owner: step 28.
- [a-kept-run-adopted-after-its-call-moved](a-kept-run-adopted-after-its-call-moved.md): A read kept from a response sent again is used for its call even when the call was planned again onto another path: a stale result. Owner: step 28, or the next change to the round's cache.
- [edits-held-behind-calls-not-yet-started](edits-held-behind-calls-not-yet-started.md): Where an answer may be a stop, a later call with side effects waits behind every earlier call not started while a changing call runs, so edits of different files run less in parallel. Owner: the owner, with a measurement; step 32.
- [an-epoch-read-without-its-variants](an-epoch-read-without-its-variants.md): An epoch from a log before step 16 has no variants and follows the live model's; reload must fix them. Owner: step 24.
- [a-masked-instruction-notice-is-told-again](a-masked-instruction-notice-is-told-again.md): A masked instruction-file notice hashes differently on reload, so the file is announced as changed after every resume. Owner: a later step on instruction files, or step 32.
- [a-prompt-past-the-trigger-on-a-small-window](a-prompt-past-the-trigger-on-a-small-window.md): On a small-window model the first estimate exceeds `context.compact_at`, so every turn ends `compaction_failed` unsent. Owner: a later step on local models, or step 33.
- [a-reasoning-budget-request-past-the-window-on-older-models](a-reasoning-budget-request-past-the-window-on-older-models.md): A main request with a reasoning budget can pass the window with its `max_tokens` on Anthropic's models before Claude 4.5, failing every turn alike. Owner: the step that adds a budget model to the catalog.
- [a-summary-cut-off-at-the-output-limit](a-summary-cut-off-at-the-output-limit.md): A summary cut off at the output limit is marked only in its text, not in the log's `summary` entry. Owner: a later step on compaction, with the owner for the format.
- [an-instruction-file-left-out-for-the-cap](an-instruction-file-left-out-for-the-cap.md): An instruction file left out of the prompt for `context.instructions_cap` is still listed in the epoch as given. Owner: a later step on instruction files, with the owner for the format.
- [config-provenance-in-the-application](config-provenance-in-the-application.md): `/config` shows the values in effect, not the layer each came from; `config/get` gives no provenance. Owner: step 24.
- [diff-emphasis-on-a-line-with-a-tab](diff-emphasis-on-a-line-with-a-tab.md): A diff's line with a tab or an escaped character loses the bold of its changed words. Owner: step 32.
- [statistics-a-build-cannot-use-are-said-only-in-the-log](statistics-a-build-cannot-use-are-said-only-in-the-log.md): A newer build's or an unreadable statistics file is said in the log and by `cortex usage` and `cortex stats`, not by the application. Owner: step 24.
- [stopping-at-a-task-lease-edge](stopping-at-a-task-lease-edge.md): At a task lease's edge the loop should stop and report, then fall back to the standing lease. Owner: step 37.
- [the-export-command-has-no-step](the-export-command-has-no-step.md): `/export`, in 09-tui's commands, is built by no step. Owner: the owner.
- [the-maps-share-of-the-context-record](the-maps-share-of-the-context-record.md): The repository map is counted among the notices in the context record, so the profiler cannot say what it costs. Owner: step 44, or step 16.
- [the-token-counting-endpoint-near-the-limit](the-token-counting-endpoint-near-the-limit.md): The provider's token-counting endpoint is never asked near the limit; needs a `Protocol` method and loop call. Owner: a later step that touches the budget, or step 33.

### cortex-base

- [log-crate-bridge](log-crate-bridge.md): The logging subscriber has no bridge for dependencies that log through the `log` crate at `warn` and above. Owner: step 32, or the first step whose dependency needs it.

### cortex-cli

- [headless-details-left-open](headless-details-left-open.md): Six small gaps in headless mode: dropped early notices, error records, `files_changed` matching, Windows close, untested signals. Owner: step 27 or step 32.
- [reading-a-key-at-a-terminal](reading-a-key-at-a-terminal.md): `cortex auth login` refuses a terminal because reading a key without echo needs the terminal layer. Owner: step 25.
- [shell-completions](shell-completions.md): Completions should come from an `xtask` command over the `FLAGS` table; the table exists, the command does not. Owner: step 34.
- [the-binary-near-its-size-budget](the-binary-near-its-size-budget.md): After step 33's size fix the binary has about 5 MB left on each system (Windows 13.19 of 18 MiB, Linux 11.33 of 16), with extensions and MCP to come; each pull request shows its size. Open until a step-34 build passes on both. Owner: step 34.

### cortex-config

- [a-setting-that-restricts-reads](a-setting-that-restricts-reads.md): No setting fills the launcher's read-restricting policy (`Reach::Only`), so every sandboxed session can read everything. Owner: the owner, then step 32 unless sooner.
- [approval-time-limit-setting](approval-time-limit-setting.md): A setting for how long an approval may wait, in the reserved `[rpc]` section, feeding `CoreOptions::approval_timeout`. Owner: step 27.
- [secret-service-backend](secret-service-backend.md): The Linux Secret Service credential backend needs a non-blocking CI job against a real keyring daemon. Owner: a step the owner places.
- [a-projects-shortest-cooldown](a-projects-shortest-cooldown.md): A project's `model.speed_cooldown_min = "0s"` can make a fast tier the user chose change speed every half minute, each change a possible rewrite of the prompt cache. Owner: the owner.
- [settings-a-later-step-declares](settings-a-later-step-declares.md): Reserved configuration sections, including `[models]` and `[http]`, still need their keys declared and their project rules set. Owner: steps 5, 13, 15, 16 and others, each for its own section.

### cortex-core

- [the-core-thread-waits-on-a-revokes-write](the-core-thread-waits-on-a-revokes-write.md): A change of level waits on the core thread for a revoke's or a record's write of the project's approvals, under the lock that keeps them from losing each other. Owner: the next change to the core's policy, or step 32.
- [a-base-capture-beside-the-turns-git](a-base-capture-beside-the-turns-git.md): A session's base capture can come out incomplete when the turn's own git runs beside it; one CI flake. Owner: the next step that changes the base capture, or step 32.
- [a-closed-view-can-still-answer](a-closed-view-can-still-answer.md): After `session/close` a client stays among those an approval request went to, so its answer is still taken. Owner: the owner; before step 27.
- [a-days-time-limit](a-days-time-limit.md): `limits.day.time` is accepted and not enforced, and a session open across midnight draws on the day it began in. Owner: step 37 (task leases' budgets), or a step the owner names.
- [a-supervisor-lowering-a-wait](a-supervisor-lowering-a-wait.md): A client that is not a `user` is refused every tighten-only key in `session/new`'s overrides, a shorter value too; taking a lower one needs a comparison with a second load. Owner: step 37.
- [an-observer-that-ends-a-session](an-observer-that-ends-a-session.md): An observer's `session/close` or disconnect can end a session that has no client left that may drive it. Owner: the owner; before step 27.
- [config-reload-through-the-core](config-reload-through-the-core.md): `config/reload` answers `not_available`; step 23 settled what a reload applies to a running session, key by key, and step 24 wires it. Owner: step 24.
- [log-level-from-configuration-in-headless-mode](log-level-from-configuration-in-headless-mode.md): A headless run starts the log before configuration is read, so `log.level` is ignored there (the interactive half is done). Owner: step 32.
- [secret-filter-on-what-clients-are-sent](secret-filter-on-what-clients-are-sent.md): Events, approval requests and in-progress items sent to clients skip the secret filter applied to disk writes. Owner: the owner; before step 27.
- [shutdown-and-a-session-being-put-together](shutdown-and-a-session-being-put-together.md): `Core::shutdown` returns before a session still being put together has ended itself. Owner: step 27.
- [the-top-of-a-tree-past-the-listing-cap](the-top-of-a-tree-past-the-listing-cap.md): A listing cut at 100,000 entries holds the first directories of a depth-first walk, not the top of the tree. Owner: a later index step, or step 33.
- [a-file-held-open-on-ntfs-and-its-stamp](a-file-held-open-on-ntfs-and-its-stamp.md): NTFS updates a directory entry's stamp lazily while a writer holds a file open, so the index may read it every pass or keep an old outline. Owner: a later index step, or step 33.
- [memory-a-pass-leaves-resident](memory-a-pass-leaves-resident.md): The allocator keeps what the index's pass freed: on real code 22.8 MB resident after a cold pass on Linux, from 8.7 MB, against the idle TUI's 25 MB. Owner: step 33.
- [the-maps-files-of-recent-sessions](the-maps-files-of-recent-sessions.md): The map's ranking by the files of recent sessions has its seam (`RecentFiles`) and no source: step 16's tool records name no files. Owner: the step that has the statistics record files.
- [what-start-up-opens-traced](what-start-up-opens-traced.md): The blocking check that start-up opens no file outside an allow-list does not exist; the index's part is checked with a held gate. Owner: the next start-up step, or step 33.
- [the-shell-version-after-a-shell-update](the-shell-version-after-a-shell-update.md): The first session after a shell is installed or updated has no shell version in its prompt, so the next resume is a `resume` epoch. Owner: step 33.

### cortex-knowledge

- [a-promote-while-a-confined-command-runs](a-promote-while-a-confined-command-runs.md): A promote made while a confined command of the same project runs is taken for a change from outside, and stops that command's turn. Owner: step 24, or a fix session that changes the continuity check.
- [a-same-size-rewrite-within-a-clock-tick-on-windows](a-same-size-rewrite-within-a-clock-tick-on-windows.md): On Windows a file's stamp is its time and size, so a same-size rewrite within the clock's tick is not seen by the look for a changed profile. Owner: a step that changes `cortex-base`'s `watch`, or step 32.
- [the-item-an-entry-was-learned-from](the-item-an-entry-was-learned-from.md): An entry's source has the session and the date and not the item: the tools' contract gives a tool no call id. Owner: a step that changes the tools' contract, or step 24.

### cortex-llm

- [a-price-that-depends-on-the-time-of-day](a-price-that-depends-on-the-time-of-day.md): The catalog's single rate card records DeepSeek's off-peak requests at the peak price, an upper bound. Owner: the owner, with step 16.
- [account-in-the-replay-rule](account-in-the-replay-rule.md): Whether a request carries its account, so a provider tying reasoning to an account never gets another's. Owner: the first step that adds such a provider, or step 53.
- [fuzz-targets-for-later-decoders](fuzz-targets-for-later-decoders.md): Each later provider protocol's decoder needs a fuzz target in `fuzz/` and a line in `xtask/src/outside.rs`. Owner: steps 31, 35 and 36.
- [the-prefix-check-and-image-references](the-prefix-check-and-image-references.md): The debug prefix check skips requests holding a stored image reference; a protocol should write a placeholder. Owner: step 33, or the step that next touches images.
- [the-message-for-a-host-outside-ascii](the-message-for-a-host-outside-ascii.md): A refused host outside ASCII is told the `xn--` form by an example, not its own; a percent-encoded one gets the URL library's message. Owner: a later step on providers, or step 33.

### cortex-policy

- [cd-readings-the-analysis-does-not-share](cd-readings-the-analysis-does-not-share.md): The command analysis reads `cd` differently from bash (mode-000 directories) and probably from Git for Windows. Owner: step 32, with a Windows run.
- [false-asks-from-the-pattern-and-word-checks](false-asks-from-the-pattern-and-word-checks.md): The pattern and word checks ask or refuse needlessly for several link, quoting, set and heredoc forms. Owner: step 32.
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

- [the-first-lease-flush-before-the-first-request](the-first-lease-flush-before-the-first-request.md): A session's first lease record is flushed to disk before its first request, whatever the durability. Owner: the next step that changes leases, or step 32.

- [an-undo-after-an-incomplete-look](an-undo-after-an-incomplete-look.md): An undo after an incomplete change look does not say what it missed; the checkpoint lacks the flag. Owner: a later step, else step 32.
- [hashing-large-files-at-a-rounds-end](hashing-large-files-at-a-rounds-end.md): A changed file is hashed whole at round end and on rewind, uncapped, at 0.4 s a gibibyte. Owner: step 33.
- [private-keys-in-text-artifacts](private-keys-in-text-artifacts.md): Text artifacts are redacted line by line, so a PEM private key is masked on one line only. Owner: step 32.

### cortex-stats

- [a-clock-that-jumps-forward](a-clock-that-jumps-forward.md): A clock set months ahead prunes the raw monthly files early, and one set 400 days ahead folds the day buckets for good. Owner: step 33.
- [a-reset-during-a-writers-settle](a-reset-during-a-writers-settle.md): A writer that loaded the aggregates just before `cortex stats reset` saves their part back. Owner: the step that next changes the reset, or step 33.
- [tests-the-step-16-fixes-left-thin](tests-the-step-16-fixes-left-thin.md): The limits lock held through an update, the flushed save, two races of moving a file aside, the reader thread's end and the 64-bit index on Windows are seen only by reading. Owner: step 33.
- [waits-on-the-statistics-without-a-limit](waits-on-the-statistics-without-a-limit.md): A day's purse's first read at session open, and the refresh behind `limits.json`'s lock, wait with no time limit of their own. Owner: step 27.

### cortex-term

- [a-paste-that-stalls](a-paste-that-stalls.md): A bracketed paste with no end marker for 2 seconds is split, and the remainder arrives as keys. Owner: step 25.
- [backspace-sent-as-ctrl-h](backspace-sent-as-ctrl-h.md): The decoder reads byte 0x08 as `Ctrl+Backspace`, so a terminal whose Backspace sends `^H` deletes a word. Owner: step 25.
- [input-decoder-corners](input-decoder-corners.md): `sys::unix::wait` and `sys::windows::wait` restart their timeout after an interrupted call, stale wake or key release. Owner: step 32.
- [pastes-without-bracketed-paste](pastes-without-bracketed-paste.md): Pastes in terminals without bracketed paste are not detected, so a pasted line break sends the prompt. Owner: step 25.
- [terminal-test-gaps](terminal-test-gaps.md): Terminal-layer tests lack styled, emoji and zero-width scripts, resize re-wrap, per-signal handlers, real SIGTSTP and nested-child cleanup. Owner: steps 22 and 23.

### cortex-tools

- [a-time-limit-test-under-load](a-time-limit-test-under-load.md): `process::sys_a_time_limit_stops_the_whole_tree` failed once in 50 runs of a loaded stress (seed 17): the limit was not yet among the timer's waits when the test looked. Owner: a tooling or fix session.
- [names-that-escape-to-the-same-text](names-that-escape-to-the-same-text.md): Two names that are one text once escaped are counted twice and shown once by `list`, and share their matches in `search`. Owner: a later `cortex-tools` step, or step 32.
- [a-git-look-too-slow-on-a-loaded-windows-runner](a-git-look-too-slow-on-a-loaded-windows-runner.md): The shell's look at changes gave up as too slow on a loaded Windows runner, failing one system test once. Owner: the next step that changes the look, or step 32.
- [cmd-searches-the-current-directory](cmd-searches-the-current-directory.md): `cmd.exe` finds a program in the working directory before `PATH`, so a workspace `git.bat` runs as `git`. Owner: the owner.

### cortex-tui

- [a-signal-while-the-external-editor-runs](a-signal-while-the-external-editor-runs.md): A signal sent to the program alone while the external editor runs leaves the editor running, and the watchdog ends the program without its graceful end, leaving the prompt's folder. Owner: step 32.
- [editor-cost-on-very-large-text](editor-cost-on-very-large-text.md): The prompt editor lays out all its text on every key; a shortened `@` query refilters all paths. Owner: step 33.
- [external-editor-test-gaps](external-editor-test-gaps.md): Parts of the external editor (umask, ConPTY modes, failed hand-over, `.cmd` editors, interrupt guard) are untested. Owner: step 32.
- [highlighted-code-drawn-again](highlighted-code-drawn-again.md): `Markdown::render` draws code plain when a finished answer is redrawn at another width. Owner: step 25.
- [the-bincode-advisory-ignored-for-syntect](the-bincode-advisory-ignored-for-syntect.md): `deny.toml` ignores RUSTSEC-2025-0141, `bincode` unmaintained, reached only through `syntect`'s loading of its packed grammars; drop it when `syntect` moves off `bincode` or a flaw is named. Owner: the next step that touches highlighting, otherwise step 25.
- [the-default-editor-on-windows-11](the-default-editor-on-windows-11.md): Windows 11's tabbed Notepad may exit at once, so `Ctrl+G` removes the prompt's folder while it is open; a folder left behind is not tried again. Owner: the owner, by hand; the folder half, step 32.

### cortex-testkit

- [timeout-report-timing-on-windows](timeout-report-timing-on-windows.md): The `watch` timeout report counts from the first wait off Linux; Windows takes the process tree from PowerShell. Owner: a later tooling step.

### The tooling and CI

- [caught-by-names-checked-only-at-the-desk](caught-by-names-checked-only-at-the-desk.md): `check` in CI keeps no list of tests, so it does not check `caught-by` names. Owner: the owner.
- [a-retried-tests-standard-error-in-the-report](a-retried-tests-standard-error-in-the-report.md): `xtask` reads a retried test's standard error from its first attempt, not the last. Owner: a later tooling step.
- [actionlint-in-the-checks](actionlint-in-the-checks.md): The workflows are not linted by actionlint, which the pinned install action cannot supply as a checksummed binary. Owner: the owner.
- [the-binary-size-by-crate-on-windows](the-binary-size-by-crate-on-windows.md): `size --breakdown` reads symbols with `nm`, which an MSVC binary keeps in a PDB, so it does nothing on Windows. Owner: a later tooling step.
- [perf-budgets-for-search-diff-and-decoding](perf-budgets-for-search-diff-and-decoding.md): The eighth tooling step measured search, a re-read's diff and JSON decoding on real code, and proposed budgets the owner has not set. Owner: the owner.
- [perf-in-the-main-workflow](perf-in-the-main-workflow.md): `cargo xtask perf` is not yet run in the `main` workflow on both systems with stored baselines. Owner: a tooling step, or step 33.
- [rough-edges-of-the-seventh-tooling-steps-commands](rough-edges-of-the-seventh-tooling-steps-commands.md): Small faults of the seventh tooling step's commands (ci-times' unchecked `--against`, api-diff's re-export homes, the corpus lock in the dependency graph) that let no comparison pass wrongly. Owner: a later tooling step.
- [replace-and-the-comments-beside-a-match](replace-and-the-comments-beside-a-match.md): `replace` stacks a `--to`'s doc comment on the file's, and can leave a read-only file's temporary copy on Windows. Owner: a later tooling step.
- [tests-the-fifth-tooling-step-left-unwritten](tests-the-fifth-tooling-step-left-unwritten.md): Seven behaviours of the fifth tooling step's tools that work and that no test or break holds yet. Owner: a later tooling step.
- [tests-the-eighth-tooling-step-left-thin](tests-the-eighth-tooling-step-left-thin.md): What the eighth tooling step's tools do that no test holds (the re-run's wiring, the Windows close tests, `os:` names of shared tests, `perf-ab`'s missing rounds) and small faults. Owner: a later tooling step.
- [tests-the-seventh-tooling-step-left-thin](tests-the-seventh-tooling-step-left-thin.md): Behaviours of the seventh tooling step's tools no blocking test holds (`ab`'s glue, `try`'s commit check, `api-diff`'s report and its Windows runs, the corpus on Windows). Owner: a later tooling step.
- [timeout-report-for-tests-without-the-test-kit](timeout-report-for-tests-without-the-test-kit.md): A test using none of the test kit's waits gets no timeout report, since nextest lacks pre-timeout scripts. Owner: a later tooling step.
- [typed-values-in-composite-actions](typed-values-in-composite-actions.md): The repo rule against event values in scripts reads `.github/workflows/` only, not composite actions under `.github/actions/`. Owner: a later tooling step.
- [the-pr-size-jobs-rough-edges](the-pr-size-jobs-rough-edges.md): `ci-wait` counts `pr-size` with the blocking jobs. Owner: a tooling step.

### Windows

- [key-latency-over-conpty](key-latency-over-conpty.md): A key through ConPTY on a hosted Windows runner takes 16 ms at the median, against 2 ms. Owner: the owner; before step 23.
- [windows-start-overhead-in-release](windows-start-overhead-in-release.md): The release-build overhead of starting a shell command on Windows is unmeasured, though `try` can measure it. Owner: step 33, or sooner on any Windows machine.

### Other

- [quickjs-with-the-windows-toolchain](quickjs-with-the-windows-toolchain.md): Confirm QuickJS builds in the real extension crate with the Windows MSVC toolchain, or switch to Lua. Owner: step 28, in its first hour.
