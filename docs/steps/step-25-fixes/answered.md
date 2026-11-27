# Answered in the step 25 fixes, on 2026-11-27

Fixes given by the maintainers after an independent review of step 25 (full screen, session UI and onboarding) found defects in the first run, `auth login`, the import of other tools' configurations, file put-back and the full screen. The owner picked ten to fix; each has a test that fails on `main` and a break its test catches ([`mutations/step-25-fixes.breaks`](../../../mutations/step-25-fixes.breaks)). The rest of the review's findings, and what these fixes left, are open items with owners. The work was done in parts, each with a record of its own: [parts/](parts/).

## Each finding's outcome

The review's numbers.

| Finding | Outcome | Part |
|---|---|---|
| 1. `auth login` echoed a key typed before its row | Fixed: the terminal is taken, echo off, before the core starts | [keys](parts/keys.md) |
| 2. A token the first run showed `[redacted]` printed whole by `config show`, `config/get` and `/config` | Fixed, the owner's choice: masked in every view that displays the configuration, imported as it is | [secrets](parts/secrets.md) |
| 3. Credential names the import did not know | Fixed: one list for the import and the views, the review's words added, `pwd` in an address, `user:password@host` with no scheme, an argument that is not text refused | [secrets](parts/secrets.md) |
| 4. A header's variable derived with no check | Fixed: unique among the servers picked across files, never a provider key's or a `CINERIS_` name, one already set flagged with the host it would go to | [secrets](parts/secrets.md) |
| 5. The commands step hid npm's `pre<name>` and `post<name>` | Fixed: shown with the script, in order | [keys](parts/keys.md) |
| 6. `autonomous` offered where the configuration turns the sandbox off | Fixed: offered from the configuration in force | [keys](parts/keys.md) |
| 7. A file put back after it was removed lost its mode | Fixed, with an optional field in the session log | [put-back](parts/put-back.md) |
| 10. The full screen left the keyboard protocol's flags on the alternate screen | Fixed: each screen's flags pushed and popped on that screen, in every way out; the test kit's `VTerm` has a stack per screen | [terminal](parts/terminal.md) |
| 11. A failed write leaving the alternate screen never retried | Fixed: a mode is marked off only once its write lands | [terminal](parts/terminal.md) |
| 12. Every reasoning kept twice, unbounded, and the 200-turn test played none | Fixed in the interface (one copy, kept to 8 KiB); the test plays reasoning; **the inline budget is passed by the rest of the process** (below) | [reasoning](parts/reasoning.md) |
| 17. `auth login` ignored `[terminal]` | Fixed with 1 | [keys](parts/keys.md) |
| 18. The viewer read a compaction's summary from the session open now | Fixed with 12, the same read | [reasoning](parts/reasoning.md) |
| 8, 9, 13 to 16, 19 to 23, the untested rules, the lower-confidence notes | Open items with owners | [items](parts/items.md) |

## Settled by the maintainers

- **Finding 2: two kinds of view.** The views that display the configuration (`config show` with every `--layer` and `--origin`, `config get`, `Loaded::effective`, the `Debug` forms, the core's `config/get`, `/config`) mask after a credential's name as the import does, a value that is not one token included. The view that asks for consent (the project trust question's held-back values) shows a project's values whole, its local file's too, since a repository can ship one; it masks only what the secret filter finds and that is shaped as a secret (a token, a whole private key's block), so that no code is hidden on a screen that asks for a yes. The server's own name is never read as a credential's. Step 30, which declares `[mcp]`, inherits both rules ([13-mcp.md](../../architecture/13-mcp.md), "Configuration").
- **Finding 5:** npm's `pre` and `post` scripts are shown with the script, not left out with it.
- **Finding 6:** `Core::sandbox_status` answers for the configuration in force (its meaning changes; `initialize`'s `sandbox` does not).
- **Finding 7:** a tracked file a command removed comes back owner-only from git's entry mode (`100755` is `0700`, `100644` is `0600`, less the umask; no execute bit where `core.fileMode=false`): narrower than a checkout, never wider than the person had, and still better than the `0644` of before. A restored tracked file loses its group's and others' read, and the person may `chmod` it back.
- **Finding 12:** a kept reasoning past 8 KiB is drawn cut in the full screen and the viewer's transcript, with a dim row; `Enter` in the viewer reads it whole; a search says how many reasonings it looked in only as far as kept.
- **The test hook of finding 1** (`CINERIS_TEST_HOLD_CORE`, read only with the `test-hooks` feature, refused in a shipped binary by `cargo xtask size`) is added to [a-workspace-build-has-the-test-hooks](../../outstanding/a-workspace-build-has-the-test-hooks.md).

## The session log's new field

A checkpoint's kept version gains an optional `"mode"` (octal, `"755"`), written only where the mode is known: a version without one is written byte for byte as before, an older build reads a log with the field (shown on `main`'s code), a value this build cannot read is no mode and never a lost version, and the log's format version is unchanged. No protocol type changed ([07-sessions-and-context.md](../../architecture/07-sessions-and-context.md), the `checkpoint` row and "Snapshots and undo").

## The 200-turn measurement, and the budget it passes

`perf_two_hundred_turns_in_each_presenter` now plays 16 KiB of reasoning a turn, as step 25's design mix has it. On the reference machine, otherwise idle, the shipped build (`--profile dist --target x86_64-unknown-linux-musl`), resident after 200 turns and its peak, in MB:

| Tree | Inline (budget 40) | Full screen (budget 80) |
|---|---|---|
| `main`, its test (no reasoning) | 32.6, peak 42.4 | 32.8; 35.6 after scrolling, resizing and searching; peak 43.0 |
| The test with reasoning, before the fix | 55.5, peak 71.7 | 55.7; 59.4 after; peak 83.0 |
| The fix | 48.4, peak 64.5 | 48.8; 53.5 after; peak 66.9 |
| The fix, no reasoning | 32.3, peak 41.1 | 32.6; 35.0 after; peak 41.8 |

The interface now holds at most 8 KiB of each reasoning (1.6 MB over 200 turns, by the counting-allocator tests). **The inline presenter is over its budget by 8.4 MB** with reasoning: the rest of the 16 MB is outside the interface (the session keeps each reasoning's text twice, in its part and in its raw block, and each request replays every earlier one). No budget was changed; the conflict was raised with a recommendation, and **the owner settled it:** these fixes merge over the budget, and keeping a reasoning's text once in the session, then measuring the request's peak, is a small step of its own soon after them, with its own review and a new version of the session log's format if the stored raw blocks change ([two-hundred-turns-of-reasoning-pass-the-inline-budget](../../outstanding/two-hundred-turns-of-reasoning-pass-the-inline-budget.md)). To repeat it: `cargo xtask perf -p cineris-cli perf_two_hundred_turns_in_each_presenter` on the shipped build, `CINERIS_PERF_REASONING` setting the bytes of each turn's reasoning.

## The independent review of these fixes

A reviewer that had not seen how the fixes were made read the secret paths as soon as they existed, then the whole change, then the changes since. Each finding it confirmed was fixed with a test that fails without the fix:

- **First read (the secret paths), eight findings:** the trust question hid a project's script after a credential's name, and a server's whole command where its name held a credential's word; `args` as one string or a table, a value starting with `-`, and a header given as an argument were shown whole; header variables collided across files; the "already set" row had a run of spaces its test folded away; a derived `Debug` printed a hidden token; a key name list that over-reached. All fixed ([secrets](parts/secrets.md), "The review's points").
- **The whole change, nine findings:** the consent view could still hide code through the secret filter's own formats (a private key's first line with no last, an address's password holding shell characters); the emergency restore popped the main screen's flags where the program never pushed; a refused write turning the alternate screen on left it marked on; "never wider" did not hold for a command's change; put-back corners on Windows and for a file whose content was back but its mode wider; the viewer could read the wrong reasoning run; smaller terminal points; weak put-back tests; the npm row could run past the cap. All fixed, in the parts' records.
- **The changes since:** it agreed with owner-only modes from git for a file made again (the condition the maintainers set), and found two more, fixed with a test and a break each: git's mode was kept for a file a command edited as well as one it removed, so an undo of a command's edit narrowed a tracked `0644` file to `0600` ([put-back](parts/put-back.md), "The review's last read"); and the trust question still masked an address's password holding `%` or `!`, which `cmd` expands into a variable's text ([secrets](parts/secrets.md), "The review's last read"). Its two low terminal points are in [a-refused-terminal-write-is-retried-but-never-waited-out](../../outstanding/a-refused-terminal-write-is-retried-but-never-waited-out.md) and [a-killed-editors-keyboard-flags-on-the-main-screen](../../outstanding/a-killed-editors-keyboard-flags-on-the-main-screen.md). While re-running the breaks, one was found never caught: the test of a script as an address's password used a `/`, which ends a password, so the filter never found it and the test held nothing; its input was corrected and the break is caught.

## What is left open

The items the review's other findings went to are listed in [parts/items.md](parts/items.md). These fixes opened or changed, besides: [two-hundred-turns-of-reasoning-pass-the-inline-budget](../../outstanding/two-hundred-turns-of-reasoning-pass-the-inline-budget.md), [the-editing-tools-write-a-private-file-open-on-windows](../../outstanding/the-editing-tools-write-a-private-file-open-on-windows.md), [a-refused-terminal-write-is-retried-but-never-waited-out](../../outstanding/a-refused-terminal-write-is-retried-but-never-waited-out.md), [a-killed-editors-keyboard-flags-on-the-main-screen](../../outstanding/a-killed-editors-keyboard-flags-on-the-main-screen.md) and [the-search-worker-keeps-a-copy-of-the-transcript](../../outstanding/the-search-worker-keeps-a-copy-of-the-transcript.md) (narrowed).

## Breaks, stress and Windows

- **The breaks:** the parts' lists put together in [`mutations/step-25-fixes.breaks`](../../../mutations/step-25-fixes.breaks), 88 breaks. Run whole on a hosted runner at the pull request's head (`try`, `mutate step-25-fixes`): on Linux 87 did what the list expects and 0 were flagged, and the one Windows break (`a-file-made-private-since-is-opened-up-on-windows`) did on Windows. The parts' records give each break's test. Older lists re-aimed at the code as it is now, their rules unchanged: `step-09`, `step-10`, `step-20`, `step-23-fixes`, `step-25` and one of `step-28`'s.
- **Stress**, on hosted runners: `cineris-tui`, `cineris-term`, `cineris-session`, `cineris-config` and `cineris-knowledge`, 100 times each with seeds 0 to 99, every test passed every time on both systems (1,389 and 1,416 tests, one count for each); `cineris-core`, `cineris-cli` and `cineris-base`, 30 times with seeds 0 to 29, every test passed every time on both systems (1,145 and 1,241 tests).
- **Windows:** the put-back's owner-only files, the keyboard flags through ConPTY and the external editor ran on a hosted Windows runner in `try --final` and the pull request's CI. Both trees on one runner (5.0 ms a process start), the median of two rounds: tier 1 15.9 s against `main`'s 17.6 s, tier 2 34.4 s against 36.6 s, tier 3 61.9 s against 66.0 s. The shipped Linux binary is 17,574,968 bytes, 61,440 more than `main`'s, within its 20 MB budget.
