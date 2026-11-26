# Still outstanding

What the owner or a later step still has to settle: one file per item. Together they are the list that "Still outstanding" in [build-plan.md](../build-plan.md) points to.

**This page is written by `cargo xtask outstanding`** from the items' files (since the tenth tooling step): edit an item's file, never this page. `cargo xtask rules` fails when the page is not what the command writes.

- **Leaving something open:** add a file, named for the item in lowercase words joined by hyphens, and run `cargo xtask outstanding`. The file is the item in a sentence on its first line, which is its line in the index below; a blank line; the fields, a list: `- Area:` the crate it is about (`cineris-tui`) or `tooling`, `windows` or `other`, `- Owner:` who settles it, in a few words, and `- Flaky:` for each test the item is about that fails now and then, its binary and name as nextest gives them (`` - Flaky: `cineris-core::it trust::sys_…` ``), which `stress`, `ci` away from CI and `mutate`'s baseline run again once when it fails ("Known flakes" in [testing-and-ci.md](../testing-and-ci.md)); a blank line; and a table of one row, under the header `| Item | Who | When |`: the item in full, who settles it, and when.
- **Settling an item:** delete its file, run `cargo xtask outstanding`, and record the answer in the settling step's `steps/<step>/answered.md`.
- **Changing an item:** edit its file, and run the command if its first line or a field changed.
- **An item several steps share** stays one file, and each step that does its part leaves the file alone and records that part in its own `steps/<step>/answered.md`. Only the step that finishes the item deletes its file, so parallel steps never edit it.
- **After a rebase:** `cargo xtask outstanding`. Git merges this page line by line, keeping both sides' lines (`merge=union` in `.gitattributes`), so a rebase does not stop here; what that merge gets wrong (a line twice, the line of an item that is gone, a line lost, lines out of order) `rules` finds and the command puts right. **Before merging,** rebase onto the `main` of the moment and run the command, as for any merge: a pull request whose index was right on an older `main` may not be on this one, and a merge that GitHub makes without a rebase would only be found by the checks of `main` itself.

One file per item, and not one per step, because a step settles items that other steps left open: two steps that settle different items then touch different files.

## The items, by crate or area

One line an item: what it covers and who settles it. It is the place to look first when a review asks whether a finding is already known.

### cineris-agent

- [a-background-summary-past-its-turn-cannot-be-interrupted](a-background-summary-past-its-turn-cannot-be-interrupted.md): `interrupt` on an idle session does not stop a background summary request, which runs on and goes uncounted. Owner: step 32.
- [a-kept-run-adopted-after-its-call-moved](a-kept-run-adopted-after-its-call-moved.md): A read kept from a response sent again is used for its call even when the call was planned again onto another path: a stale result. Since step 28 no extension tool can reach it (an extension's tool always runs alone and is never started early); an MCP server's tool still could. Owner: step 30, or the next change to the round's cache.
- [a-masked-instruction-notice-is-told-again](a-masked-instruction-notice-is-told-again.md): A masked instruction-file notice hashes differently on reload, so the file is announced as changed after every resume. Owner: a later step on instruction files, or step 32.
- [a-prompt-past-the-trigger-on-a-small-window](a-prompt-past-the-trigger-on-a-small-window.md): On a small-window model the first estimate exceeds `context.compact_at`, so every turn ends `compaction_failed` unsent. Owner: a later step on local models, or step 33.
- [a-reasoning-budget-request-past-the-window-on-older-models](a-reasoning-budget-request-past-the-window-on-older-models.md): A main request with a reasoning budget can pass the window with its `max_tokens` on Anthropic's models before Claude 4.5, failing every turn alike. Owner: the step that adds a budget model to the catalog.
- [a-summary-cut-off-at-the-output-limit](a-summary-cut-off-at-the-output-limit.md): A summary cut off at the output limit is marked only in its text, not in the log's `summary` entry. Owner: a later step on compaction, with the owner for the format.
- [a-summary-test-that-ran-out-of-time-once](a-summary-test-that-ran-out-of-time-once.md): A test of a background summary that failed ran out of time once on the reference machine, in a run where it takes a few milliseconds; nothing in the test or the loop was found that could make it wait. Owner: the maintainers, at the next occurrence.
- [a-tool-that-makes-links-beside-other-calls](a-tool-that-makes-links-beside-other-calls.md): A tool from an MCP server that declares itself read-only or keyed and makes a link runs beside later calls, which were decided before it ended. Closed for extensions by step 28: an extension's tool always runs alone. Owner: step 30.
- [an-instruction-file-left-out-for-the-cap](an-instruction-file-left-out-for-the-cap.md): An instruction file left out of the prompt for `context.instructions_cap` is still listed in the epoch as given. Owner: a later step on instruction files, with the owner for the format.
- [diff-emphasis-on-a-line-with-a-tab](diff-emphasis-on-a-line-with-a-tab.md): A diff's line with a tab or an escaped character loses the bold of its changed words. Owner: step 32.
- [edits-held-behind-calls-not-yet-started](edits-held-behind-calls-not-yet-started.md): Where an answer may be a stop, a later call with side effects waits behind every earlier call not started while a changing call runs, so edits of different files run less in parallel. Owner: the owner, with a measurement; step 32.
- [stopping-at-a-task-lease-edge](stopping-at-a-task-lease-edge.md): At a task lease's edge the loop should stop and report, then fall back to the standing lease. Owner: step 37.
- [the-maps-share-of-the-context-record](the-maps-share-of-the-context-record.md): The repository map is counted among the notices in the context record, so the profiler cannot say what it costs. Owner: step 44, or step 16.
- [the-token-counting-endpoint-near-the-limit](the-token-counting-endpoint-near-the-limit.md): The provider's token-counting endpoint is never asked near the limit; needs a `Protocol` method and loop call. Owner: a later step that touches the budget, or step 33.

### cineris-base

- [a-break-of-the-test-hooks-permits-caught-only-sometimes](a-break-of-the-test-hooks-permits-caught-only-sometimes.md): The break `tooling-7/one-let-go-lets-every-thread-go` is caught by its own test only some of the time: a break run can call it not caught, and say so again, with nothing changed. Owner: the next small-fix session.
- [a-windows-program-lookup-takes-a-file-with-no-extension](a-windows-program-lookup-takes-a-file-with-no-extension.md): On Windows, looking a program up by name takes a file of that name with no extension before a later folder's `.exe`, and then fails to start it. Owner: step 32 (hardening pass).
- [log-crate-bridge](log-crate-bridge.md): The logging subscriber has no bridge for dependencies that log through the `log` crate at `warn` and above. Owner: step 32, or the first step whose dependency needs it.

### cineris-cli

- [a-nested-run-under-a-session-is-capped-at-read-only](a-nested-run-under-a-session-is-capped-at-read-only.md): A headless run of Cineris started by one of a session's commands runs at `read-only`, whatever it asks for, because it cannot know its parent session's level; a nested sub-task that needs to write cannot run until the parent's level reaches it in a form the model cannot raise. Owner: step 50 (sub-agents and delegated sub-tasks, which run a child session under a narrowed lease: [build-plan.md](../build-plan.md), "Step 50").
- [headless-details-left-open](headless-details-left-open.md): Six small gaps in headless mode: dropped early notices, error records, `files_changed` matching, Windows close, untested signals. Owner: step 27 or step 32.
- [rewind-and-fork-have-no-test-of-the-real-binary](rewind-and-fork-have-no-test-of-the-real-binary.md): `/rewind` and fork are tested through the application's scripted link and the core's own tests, not by the real binary under a pseudo-terminal, in process or under `cineris attach`. Owner: step 25.
- [shell-completions](shell-completions.md): Completions should come from an `xtask` command over the `FLAGS` table; the table exists, the command does not. Owner: step 34.
- [the-applications-first-frame-waits-for-the-approving-check](the-applications-first-frame-waits-for-the-approving-check.md): The application asks whether it runs under a session's command, and marks its own commands, before its first frame, which costs that frame about 0.15 ms on Linux. Owner: step 33.
- [the-binary-near-its-size-budget](the-binary-near-its-size-budget.md): After the size step (2026-11-19) the budgets are 25 MiB on Windows and 20 MiB on Linux (decision 0039): the binary has 8.31 MiB left on Windows (16.69 of 25) and 6.15 MiB on Linux (13.85 of 20), for steps 28 to 34's projected 3.8 to 7.3 MB and 2.95 to 5.6 MB. Open until a step-34 build passes on both. Owner: step 34.

### cineris-config

- [a-projects-shortest-cooldown](a-projects-shortest-cooldown.md): A project's `model.speed_cooldown_min = "0s"` can make a fast tier the user chose change speed every half minute, each change a possible rewrite of the prompt cache. Owner: the owner.
- [a-setting-that-restricts-reads](a-setting-that-restricts-reads.md): No setting fills the launcher's read-restricting policy (`Reach::Only`), so every sandboxed session can read everything. Owner: the owner, then step 32 unless sooner.
- [secret-service-backend](secret-service-backend.md): The Linux Secret Service credential backend needs a non-blocking CI job against a real keyring daemon. Owner: a step the owner places.
- [settings-a-later-step-declares](settings-a-later-step-declares.md): Reserved configuration sections, including `[models]` and `[http]`, still need their keys declared and their project rules set. Owner: steps 5, 13, 15, 16 and others, each for its own section.

### cineris-core

- [a-break-of-a-closed-sessions-questions-that-its-test-does-not-catch](a-break-of-a-closed-sessions-questions-that-its-test-does-not-catch.md): The break that leaves a closed session's open questions in place, `step-23-fixes/a-closed-sessions-question-is-not-withdrawn`, is not caught by the test it names, so the rule that a session's end withdraws its open trust and rewind questions is not held by a test. Owner: the maintainers, a small-fix session.
- [a-break-of-the-statistics-pruning-that-its-test-does-not-catch](a-break-of-the-statistics-pruning-that-its-test-does-not-catch.md): The break that makes `cineris stats` prune what it shows, `step-16/showing-the-statistics-prunes`, is not caught by the test it names, so the rule that showing the statistics deletes nothing is not held by a test. Owner: the maintainers, a small-fix session.
- [a-commands-settings-profile-for-its-turn](a-commands-settings-profile-for-its-turn.md): A custom command's front matter may name a settings profile for its turn, but no mechanism applies a profile for one turn, so since step 28 a command that names one is judged and not sent. Owner: a later step, the maintainers.
- [a-days-time-limit](a-days-time-limit.md): `limits.day.time` is accepted and not enforced, and a session open across midnight draws on the day it began in. Owner: step 37 (task leases' budgets), or a step the owner names.
- [a-file-held-open-on-ntfs-and-its-stamp](a-file-held-open-on-ntfs-and-its-stamp.md): NTFS updates a directory entry's stamp lazily while a writer holds a file open, so the index may read it every pass or keep an old outline. Owner: a later index step, or step 33.
- [a-folder-is-deleted-by-its-path-after-its-check](a-folder-is-deleted-by-its-path-after-its-check.md): `cineris ext remove` deletes an extension's folder by its path after checking where the path leads, and a folder above it swapped for a link in between is followed. Security. Owner: step 32 (hardening pass).
- [a-key-a-reload-added-is-passed-again-after-another-reload](a-key-a-reload-added-is-passed-again-after-another-reload.md): A variable that one reload made a provider's key, and a later reload made no longer one, is given to commands and to the harness's git again. Owner: the maintainers.
- [a-sessions-open-walks-the-same-paths-again](a-sessions-open-walks-the-same-paths-again.md): Windows' integration tier is about 7 s from its failure line on an 8 ms runner, and what is left of a Windows session's extra cost (the open's 29 walks, the first private folder's look-ups, the tools' assembly) is for step 33's performance pass. Owner: step 33, its performance pass.
- [a-supervisor-lowering-a-wait](a-supervisor-lowering-a-wait.md): A client that is not a `user` is refused every tighten-only key in `session/new`'s overrides, a shorter value too; taking a lower one needs a comparison with a second load. Owner: step 37.
- [a-test-of-the-map-turned-off-that-can-miss-its-break-under-load](a-test-of-the-map-turned-off-that-can-miss-its-break-under-load.md): The test that a session with the map turned off starts no background pass can miss its break under load: with the break, the pass it should see may not have reached its gate before the session closes. Owner: the maintainers, a small-fix session.
- [an-exec-argument-is-not-checked-against-the-code-pages-best-fit](an-exec-argument-is-not-checked-against-the-code-pages-best-fit.md): An extension's program is refused an argument with one of a list of characters a Windows code page's best fit turns into `"`, `\` or `-`; an argument is not converted as Windows converts it, so a character the list does not name gets through. Owner: step 32 (hardening pass).
- [an-msys-program-reads-a-doubled-backslash-as-one](an-msys-program-reads-a-doubled-backslash-as-one.md): On Windows an MSYS2 or Cygwin program reads two backslashes inside a quoted argument as one, where the C runtime reads two: an extension's argument such as `\\server\share` reaches such a program as `\server\share`. Owner: step 32 (hardening pass).
- [log-level-from-configuration-in-headless-mode](log-level-from-configuration-in-headless-mode.md): A headless run starts the log before configuration is read, so `log.level` is ignored there (the interactive half is done). Owner: step 32.
- [memory-a-pass-leaves-resident](memory-a-pass-leaves-resident.md): The allocator keeps what the index's pass freed: on real code 22.8 MB resident after a cold pass on Linux, from 8.7 MB, and the idle TUI at 30.5 MB in a glibc build with 20,000 files (12.3 MB in the musl build that ships), against 25 MB. Owner: step 33.
- [reload-now-begins-an-epoch](reload-now-begins-an-epoch.md): `/reload --now`, which would begin a new context epoch at once so that the keys waiting for one take effect, is not built. Owner: a later step that adds an epoch on request to the loop.
- [sessions-kept-with-no-client](sessions-kept-with-no-client.md): A session that `serve --socket` keeps after its last client goes is never ended by time, and runs its turn on with nobody watching. Owner: the owner, then a later step of the server.
- [the-core-thread-waits-on-a-revokes-write](the-core-thread-waits-on-a-revokes-write.md): A change of level waits on the core thread for a revoke's or a record's write of the project's approvals, under the lock that keeps them from losing each other. Owner: the next change to the core's policy, or step 32.
- [the-maps-files-of-recent-sessions](the-maps-files-of-recent-sessions.md): The map's ranking by the files of recent sessions has its seam (`RecentFiles`) and no source: step 16's tool records name no files. Owner: the step that has the statistics record files.
- [the-shell-version-after-a-shell-update](the-shell-version-after-a-shell-update.md): The first session after a shell is installed or updated has no shell version in its prompt, so the next resume is a `resume` epoch. Owner: step 33.
- [the-top-of-a-tree-past-the-listing-cap](the-top-of-a-tree-past-the-listing-cap.md): A listing cut at 100,000 entries holds the first directories of a depth-first walk, not the top of the tree. Owner: a later index step, or step 33.
- [what-start-up-opens-traced](what-start-up-opens-traced.md): The blocking check that start-up opens no file outside an allow-list does not exist; the index's part is checked with a held gate. Owner: the next start-up step, or step 33.

### cineris-index

- [a-memory-peak-test-near-its-limit](a-memory-peak-test-near-its-limit.md): `outlines::outlining_a_name_of_a_megabyte_takes_little_memory` went 84 KiB over its 4 MiB peak once on a hosted Linux runner, in a pull request that did not touch `cineris-index`. Owner: `cineris-index`'s next change, or a tooling step.

### cineris-knowledge

- [a-promote-while-a-confined-command-runs](a-promote-while-a-confined-command-runs.md): A promote made from another process while a confined command of the same project runs is taken for a change from outside, and stops that command's turn. Owner: a fix session that changes the continuity check.
- [a-same-size-rewrite-within-a-clock-tick-on-windows](a-same-size-rewrite-within-a-clock-tick-on-windows.md): On Windows a file's stamp is its time and size, so a same-size rewrite within the clock's tick is not seen by the look for a changed profile. Owner: a step that changes `cineris-base`'s `watch`, or step 32.
- [the-item-an-entry-was-learned-from](the-item-an-entry-was-learned-from.md): An entry's source has the session and the date and not the item: the tools' contract gives a tool no call id. Owner: a step that changes the tools' contract, or step 24.
- [which-test-of-a-flaky-command-flipped](which-test-of-a-flaky-command-flipped.md): A flaky entry names a command, not the test that flipped, so a check's retry runs the whole command and may call a real failure flaky. Owner: step 52, with its per-ecosystem test selection.

### cineris-llm

- [a-fetch-reaches-the-machines-own-and-its-networks-public-addresses](a-fetch-reaches-the-machines-own-and-its-networks-public-addresses.md): An extension's fetch does not refuse the machine's own addresses, or its network's, when they are addresses of the internet: a name a wildcard in `net` allows can be pointed at one. Owner: step 32 (hardening pass).
- [a-price-that-depends-on-the-time-of-day](a-price-that-depends-on-the-time-of-day.md): The catalog's single rate card records DeepSeek's off-peak requests at the peak price, an upper bound. Owner: the owner, with step 16.
- [account-in-the-replay-rule](account-in-the-replay-rule.md): Whether a request carries its account, so a provider tying reasoning to an account never gets another's. Owner: the first step that adds such a provider, or step 53.
- [fuzz-targets-for-later-decoders](fuzz-targets-for-later-decoders.md): Each later provider protocol's decoder needs a fuzz target in `fuzz/` and a line in `xtask/src/outside.rs`. Owner: steps 31, 35 and 36.
- [the-message-for-a-host-outside-ascii](the-message-for-a-host-outside-ascii.md): A refused host outside ASCII is told the `xn--` form by an example, not its own; a percent-encoded one gets the URL library's message. Owner: a later step on providers, or step 33.
- [the-prefix-check-and-image-references](the-prefix-check-and-image-references.md): The debug prefix check skips requests holding a stored image reference; a protocol should write a placeholder. Owner: step 33, or the step that next touches images.
- [the-provider-transports-tls-check-on-windows-fetches-what-a-certificate-names](the-provider-transports-tls-check-on-windows-fetches-what-a-certificate-names.md): The provider transport checks a server's certificate with the platform's verifier, which on Windows fetches what the certificate names, its issuer and its revocation status, from any address and on the thread that polls the request. Owner: step 32 (hardening pass).

### cineris-policy

- [a-hook-cannot-run-a-line-read-as-only-reading-where-another-program-may-run](a-hook-cannot-run-a-line-read-as-only-reading-where-another-program-may-run.md): On Windows, and in a PowerShell or `cmd` session, an extension's script hook can never run a line the engine reads as only reading (`git status`, `cat notes.txt`), and no rule of the user's changes that; settled with the engine's judging of a program by the session's shell. Owner: step 32 (hardening pass).
- [an-extensions-program-is-judged-by-the-sessions-shell](an-extensions-program-is-judged-by-the-sessions-shell.md): The permission engine judges an extension's program by a line in the session's shell, whose own commands and aliases are not the program that runs; step 28 asks about what PowerShell and `cmd` call read-only rather than allow it. Owner: step 32 (hardening pass).
- [approvals-signed-with-a-key-that-needs-a-person](approvals-signed-with-a-key-that-needs-a-person.md): Where nothing confines commands, anything that runs with the user's rights can write the stores that approve extensions and trust projects; only approvals signed with a key that needs a person present would keep the model from approving for itself there. Owner: a step before step 34, the maintainers.
- [cd-readings-the-analysis-does-not-share](cd-readings-the-analysis-does-not-share.md): The command analysis reads `cd` differently from bash (mode-000 directories) and probably from Git for Windows. Owner: step 32, with a Windows run.
- [false-asks-from-the-pattern-and-word-checks](false-asks-from-the-pattern-and-word-checks.md): The pattern and word checks ask or refuse needlessly for several link, quoting, set and heredoc forms. Owner: step 32.
- [git-forms-the-command-analysis-misreads](git-forms-the-command-analysis-misreads.md): The command analysis misses `git config` run through an alias, and misreads `git diff` outside a repository. Owner: step 32.
- [link-makers-the-command-analysis-does-not-see](link-makers-the-command-analysis-does-not-see.md): The command analysis misses links made by `env ln`, abbreviated `cp` options, and `cp -a`, `cp -R`, `mv`. Owner: step 32.
- [names-the-path-resolver-refuses](names-the-path-resolver-refuses.md): Windows names the path resolver cannot read (trailing dot, stream, provider path) go unchecked for off-table commands. Owner: step 32.
- [pattern-matching-corners](pattern-matching-corners.md): `NamePattern` lower-casing and user shell options (`dotglob`, `globstar`, `nocaseglob`) make the pattern analysis match differently from the shell. Owner: step 32.
- [rust-toolchain-is-not-a-protected-path](rust-toolchain-is-not-a-protected-path.md): `rust-toolchain.toml` can be written by a confined command and chooses the compiler the user's next `cargo` runs. Owner: the owner, then step 32.
- [what-the-guards-of-the-harnesss-own-approvals-do-not-reach](what-the-guards-of-the-harnesss-own-approvals-do-not-reach.md): What the three guards of the harness's own approving commands leave for step 32: lines the engine's refusal misses and the subcommands' own check catches, forms only the stated residual reaches, the Windows check's blind spots, an unbounded lock, a refusal a planted job can force, and four lines its two readings judge apart. Security. Owner: step 32 (hardening pass).
- [words-of-unmodelled-lines-the-check-misreads](words-of-unmodelled-lines-the-check-misreads.md): The word check misreads split command words, program text, tricky `cd` targets and over-bound lines, at `autonomous`. Owner: step 32.

### cineris-protocol

- [the-files-a-command-changed](the-files-a-command-changed.md): The review panel lists the files the file tools changed; which files a shell command changed is in the checkpoints, not in any event. Owner: none yet.

### cineris-sandbox

- [a-launcher-refusal-read-from-the-commands-output](a-launcher-refusal-read-from-the-commands-output.md): The launcher's refusal is read from the command's own output, so a check can pass for blocked and not be fed back. Owner: step 32.
- [a-protected-files-stamp-decides-only-where-its-content-is-not-compared](a-protected-files-stamp-decides-only-where-its-content-is-not-compared.md): The look at the protected paths compares a file's stamp only where neither its content nor its hash is known on both sides: a change of a protected file's owner alone, and a file replaced by another of the same bytes, are not counted as changes, and whether they should be is not decided. Owner: step 32, beside [the-put-backs-remaining-edges](the-put-backs-remaining-edges.md).
- [a-refused-socket-the-result-does-not-name](a-refused-socket-the-result-does-not-name.md): A Unix socket refused by the filter, network on, gives a bare `EPERM` and no sandbox explanation. Owner: step 32.
- [a-session-of-its-own-in-the-sandbox](a-session-of-its-own-in-the-sandbox.md): `setsid` is faked in the sandbox, breaking terminal-making programs; a subreaper and cgroup kill would allow it, and would let the approving subcommands see a command that detached itself from its harness. Owner: step 32.
- [confined-git-and-lfs-filters](confined-git-and-lfs-filters.md): The confined read-only git cannot run a git-lfs filter that must write, leaving change detection and undo incomplete. Owner: step 32.
- [datagrams-from-a-socket-pair](datagrams-from-a-socket-pair.md): With the network off, a datagram `socketpair` can `sendto` a local datagram socket by path, such as journald's. Owner: step 32.
- [files-git-runs-that-the-protected-list-does-not-name](files-git-runs-that-the-protected-list-does-not-name.md): A program git's configuration names by a bare word, found on the `PATH`, and the protected names inside a `sandbox.write` root outside the workspace, are not watched; what the configuration names by path is, since the third security fix. Owner: step 32.
- [modes-and-times-outside-the-write-roots](modes-and-times-outside-the-write-roots.md): A confined command can change modes and times of user-owned files anywhere, since Landlock ignores `chmod` and `utimensat`. Owner: step 32.
- [path-forms-on-windows-the-helper-does-not-reach](path-forms-on-windows-the-helper-does-not-reach.md): The protected-path watch and the base state's git parsing handle mixed Windows path forms without `same_location`. Owner: step 32, or the step that brings the sandbox to Windows.
- [the-cost-of-the-look-at-protected-paths](the-cost-of-the-look-at-protected-paths.md): Walking the workspace around each confined command for protected names costs about 47 ms in a large tree. Owner: step 33.
- [the-last-writes-of-a-killed-command](the-last-writes-of-a-killed-command.md): The session sweep that ends a command's tree can lose a race with a fast-forking process. Owner: step 32.
- [the-put-backs-remaining-edges](the-put-backs-remaining-edges.md): The put-back of a confined start's protected change, built in the third security fix, has edges its reviews found and left: none lets a confined start keep a change unnamed at the look after it. Owner: step 32.

### cineris-session

- [an-undo-after-an-incomplete-look](an-undo-after-an-incomplete-look.md): An undo after an incomplete change look does not say what it missed; the checkpoint lacks the flag. Owner: a later step, else step 32.
- [hashing-large-files-at-a-rounds-end](hashing-large-files-at-a-rounds-end.md): A changed file is hashed whole at round end and on rewind, uncapped, at 0.4 s a gibibyte. Owner: step 33.
- [private-keys-in-text-artifacts](private-keys-in-text-artifacts.md): Text artifacts are redacted line by line, so a PEM private key is masked on one line only. Owner: step 32.
- [the-first-lease-flush-before-the-first-request](the-first-lease-flush-before-the-first-request.md): A session's first lease record is flushed to disk before its first request, whatever the durability. Owner: the next step that changes leases, or step 32.

### cineris-stats

- [a-clock-that-jumps-forward](a-clock-that-jumps-forward.md): A clock set months ahead prunes the raw monthly files early, and one set 400 days ahead folds the day buckets for good. Owner: step 33.
- [a-reset-during-a-writers-settle](a-reset-during-a-writers-settle.md): A writer that loaded the aggregates just before `cineris stats reset` saves their part back. Owner: the step that next changes the reset, or step 33.
- [tests-the-step-16-fixes-left-thin](tests-the-step-16-fixes-left-thin.md): The limits lock held through an update, the flushed save, two races of moving a file aside, the reader thread's end and the 64-bit index on Windows are seen only by reading. Owner: step 33.
- [the-usage-panels-year-on-windows](the-usage-panels-year-on-windows.md): On a hosted Windows runner, the usage panel's report of a year misses its 16 ms budget even with the aggregates kept between reports. Owner: the owner.
- [waits-on-the-statistics-without-a-limit](waits-on-the-statistics-without-a-limit.md): A day's purse's first read at session open, and the refresh behind `limits.json`'s lock, wait with no time limit of their own. Owner: step 27.

### cineris-term

- [function-keys-through-conpty](function-keys-through-conpty.md): A function key sent to the application through the test kit's pseudo-console on a hosted Windows runner never reached it as that key; whether a real Windows Terminal is affected is not known. Owner: step 25.
- [input-decoder-corners](input-decoder-corners.md): `sys::unix::wait` and `sys::windows::wait` restart their timeout after an interrupted call, stale wake or key release. Owner: step 32.
- [mouse-reports-in-flight-when-the-mouse-goes-off](mouse-reports-in-flight-when-the-mouse-goes-off.md): A mouse report the terminal sent just before the mouse was turned off can arrive afterwards and be read as text. Owner: step 33.
- [terminal-test-gaps](terminal-test-gaps.md): Terminal-layer tests lack styled, emoji and zero-width scripts, resize re-wrap, per-signal handlers, real SIGTSTP and nested-child cleanup. Owner: steps 22 and 23.
- [the-mouse-under-windows-terminal](the-mouse-under-windows-terminal.md): Under Windows Terminal, and wherever the win32 input mode is on, the viewer and the full screen have no mouse: through a pseudo-console the terminal's mouse reports reach the program as typed keys, however they were asked for. Owner: step 33.

### cineris-testkit

- [timeout-report-timing-on-windows](timeout-report-timing-on-windows.md): The `watch` timeout report counts from the first wait off Linux; Windows takes the process tree from PowerShell. Owner: a later tooling step.

### cineris-tools

- [a-file-is-opened-by-name-after-its-decision](a-file-is-opened-by-name-after-its-decision.md): A file a call was allowed to read or write is opened by its path after the decision, and a folder above it can become a link in between. Security. Owner: step 32 (hardening pass).
- [a-git-look-too-slow-on-a-loaded-windows-runner](a-git-look-too-slow-on-a-loaded-windows-runner.md): The shell's look at changes gave up as too slow on a loaded Windows runner, failing one system test once. Owner: the next step that changes the look, or step 32.
- [names-that-escape-to-the-same-text](names-that-escape-to-the-same-text.md): Two names that are one text once escaped are counted twice and shown once by `list`, and share their matches in `search`. Owner: a later `cineris-tools` step, or step 32.

### cineris-tui

- [a-commands-text-in-the-rewind-picker-and-the-export](a-commands-text-in-the-rewind-picker-and-the-export.md): The interface's rewind picker and `/export` show a custom command's prompt as the text it sent, under the user's own heading, not as the command the user typed. Owner: step 32.
- [a-full-screen-frame-copies-the-open-answer](a-full-screen-frame-copies-the-open-answer.md): Each full-screen frame copies the rows an answer still streaming has committed, so a very long open answer costs more per frame. Owner: step 33.
- [a-kept-box-holds-only-the-end-of-its-output](a-kept-box-holds-only-the-end-of-its-output.md): The viewer and the full screen open a finished command's box on the last 64 KiB of its output, not the whole of it. Owner: step 25.
- [a-signal-while-the-external-editor-runs](a-signal-while-the-external-editor-runs.md): A signal sent to the program alone while the external editor runs leaves the editor running, and the watchdog ends the program without its graceful end, leaving the prompt's folder. Owner: step 32.
- [a-view-in-the-live-rows-moves](a-view-in-the-live-rows-moves.md): A full-screen view scrolled up into the live rows moves when a block above it is kept. Owner: step 33.
- [editor-cost-on-very-large-text](editor-cost-on-very-large-text.md): The prompt editor lays out all its text on every key; a shortened `@` query refilters all paths. Owner: step 33.
- [external-editor-test-gaps](external-editor-test-gaps.md): Parts of the external editor (umask, ConPTY modes, failed hand-over, `.cmd` editors, interrupt guard) are untested. Owner: step 32.
- [one-character-wider-than-the-prompts-row](one-character-wider-than-the-prompts-row.md): One character can be drawn wider than a row where every escape of what acts unseen is shown: an approval's detail, the trust question, the transcript. Owner: step 33.
- [the-bincode-advisory-ignored-for-syntect](the-bincode-advisory-ignored-for-syntect.md): `deny.toml` ignores RUSTSEC-2025-0141, `bincode` unmaintained, reached only through `syntect`'s loading of its packed grammars; drop it when `syntect` moves off `bincode` or a flaw is named. Owner: the next step that touches highlighting, otherwise step 25.
- [the-default-editor-on-windows-11](the-default-editor-on-windows-11.md): Windows 11's tabbed Notepad may exit at once, so `Ctrl+G` removes the prompt's folder while it is open; a folder left behind is not tried again. Owner: the owner, by hand; the folder half, step 32.
- [the-search-counts-text-but-marks-drawn-rows](the-search-counts-text-but-marks-drawn-rows.md): The full screen's search counts its matches in each block's text but marks them in the drawn rows, so a match in a box's own words can take the current mark. Owner: step 33.
- [the-search-worker-keeps-a-copy-of-the-transcript](the-search-worker-keeps-a-copy-of-the-transcript.md): The full screen's search keeps a lower-cased copy of the whole kept transcript for as long as the program runs, and builds it on the interface's thread the first time. Owner: step 33.
- [user-theme-files](user-theme-files.md): 09-tui describes themes as TOML tables that assign a colour to each token; no step builds a user's theme file, and step 24's hot reload reloads `tui.theme`, the name of a built-in theme. Owner: none yet.

### The tooling and CI

- [a-commit-settled-to-nothing-by-hand-in-a-rebase](a-commit-settled-to-nothing-by-hand-in-a-rebase.md): A commit a person settles to nothing during `cargo xtask rebase` is dropped by git and not named in its report. Owner: the maintainers.
- [a-known-flake-in-a-breaks-own-run](a-known-flake-in-a-breaks-own-run.md): A known flake that fails in a break's own run of `mutate` makes a break that names no test look caught. Owner: a later tooling step.
- [a-retried-tests-standard-error-in-the-report](a-retried-tests-standard-error-in-the-report.md): `xtask` reads a retried test's standard error from its first attempt, not the last. Owner: a later tooling step.
- [a-workspace-build-has-the-test-hooks](a-workspace-build-has-the-test-hooks.md): A `cargo build` at the workspace's root builds `cineris` with the test hooks on, one of which shortens the dialogs' quiet time when a variable says so. Owner: step 34, its release built through `xtask`.
- [actionlint-in-the-checks](actionlint-in-the-checks.md): The workflows are not linted by actionlint, which the pinned install action cannot supply as a checksummed binary. Owner: the owner.
- [breaks-whose-lists-no-longer-say-how-they-are-caught](breaks-whose-lists-no-longer-say-how-they-are-caught.md): `mutate --check` passes a break that does not compile and a break that changes nothing, so a list can stop saying how its break is caught and no check says so until the breaks are run. Owner: a later tooling step.
- [c-source-paths-in-a-windows-build](c-source-paths-in-a-windows-build.md): A Windows build of the shipped binary on a local machine keeps the build machine's folders in QuickJS's asserts, since MSVC has no flag the shipped build can give it to take them out. Owner: step 34.
- [caught-by-names-in-a-blocking-job](caught-by-names-in-a-blocking-job.md): The nightly checks each `caught-by` against the tests that exist; whether a blocking job should too, at minutes more, is the owner's choice. Owner: the owner.
- [main-windows-tiers-over-budget-on-slow-runners](main-windows-tiers-over-budget-on-slow-runners.md): On a slow Windows runner, `main`'s own integration and system tiers are over their budgets, so a pull request that lands on one shows warnings it did not cause. Owner: the maintainers, a CI-budgets step.
- [perf-in-the-main-workflow](perf-in-the-main-workflow.md): `cargo xtask perf` is not yet run in the `main` workflow on both systems with stored baselines. Owner: a tooling step, or step 33.
- [replace-and-the-comments-beside-a-match](replace-and-the-comments-beside-a-match.md): `replace` stacks a `--to`'s doc comment on the file's, and can leave a read-only file's temporary copy on Windows. Owner: a later tooling step.
- [rough-edges-of-the-seventh-tooling-steps-commands](rough-edges-of-the-seventh-tooling-steps-commands.md): Small faults of the seventh tooling step's commands (ci-times' unchecked `--against`, api-diff's re-export homes, the corpus lock in the dependency graph) that let no comparison pass wrongly. Owner: a later tooling step.
- [tests-the-eighth-tooling-step-left-thin](tests-the-eighth-tooling-step-left-thin.md): What the eighth tooling step's tools do that no test holds (the re-run's wiring, the Windows close tests, `os:` names of shared tests, `perf-ab`'s missing rounds) and small faults. Owner: a later tooling step.
- [tests-the-fifth-tooling-step-left-unwritten](tests-the-fifth-tooling-step-left-unwritten.md): Seven behaviours of the fifth tooling step's tools that work and that no test or break holds yet. Owner: a later tooling step.
- [tests-the-seventh-tooling-step-left-thin](tests-the-seventh-tooling-step-left-thin.md): Behaviours of the seventh tooling step's tools no blocking test holds (`ab`'s glue, `try`'s commit check, `api-diff`'s report and its Windows runs, the corpus on Windows). Owner: a later tooling step.
- [the-binary-size-by-crate-on-windows](the-binary-size-by-crate-on-windows.md): `size --breakdown` reads symbols with `nm`, which an MSVC binary keeps in a PDB, so it does nothing on Windows. Owner: a later tooling step.
- [the-fuzz-package-has-no-blocking-check](the-fuzz-package-has-no-blocking-check.md): Nothing blocking builds the fuzz package or checks its seeds: a change that breaks a target, or a method its seeds miss, shows only in the nightly. Owner: a tooling step.
- [the-pr-size-jobs-rough-edges](the-pr-size-jobs-rough-edges.md): A script that waits for a whole `ci` run counts `pr-size` with the blocking jobs. Owner: a tooling step.
- [timeout-report-for-tests-without-the-test-kit](timeout-report-for-tests-without-the-test-kit.md): A test using none of the test kit's waits gets no timeout report, since nextest lacks pre-timeout scripts. Owner: a later tooling step.
- [typed-values-in-composite-actions](typed-values-in-composite-actions.md): The repo rule against event values in scripts reads `.github/workflows/` only, not composite actions under `.github/actions/`. Owner: a later tooling step.
- [windows-tier-2-at-its-budget](windows-tier-2-at-its-budget.md): Windows' tier 2 is at its 40 s budget on the slower hosted runners: `main` alone reads 97% of it, and step 27's second pull request takes it past. Owner: the owner (the budget), then a tooling step.

### Windows

- [key-latency-over-conpty](key-latency-over-conpty.md): A key through ConPTY on a hosted Windows runner takes 16 ms at the median, against 2 ms. Owner: the owner; before step 23.
- [the-windows-terminal-check-of-step-25](the-windows-terminal-check-of-step-25.md): The checklist for the real interface on Windows Terminal, step 25's `pr3-wt-checklist.md`, has not been run. Owner: the owner (Windows Terminal check).

### Other

- [json-floats-read-within-a-few-units](json-floats-read-within-a-few-units.md): A float a client sends is read to within a few units in its last place, since `serde_json` is built without its exact reading of floats. Owner: the owner.
