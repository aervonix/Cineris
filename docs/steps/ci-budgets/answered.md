# Answered in the CI-budgets fix, on 2026-11-19

A fix outside the numbered steps, given by the maintainers: `main`'s own CI no longer fitted its tier budgets. On Windows test-windows took 342 s at `6c43dfb1` (run 44592310472), tier 3 read 100 to 104 s of its 90 on 8 ms runners, and on Linux tier 1 read 28.9 s of its 20 (run 44627701218). This record says where the time went, what was changed, the flaky trust-dialog test's cause and fix, and what is left for the owner. The CI times are in [ci.md](ci.md).

## Where the time went

Measured as one-runner A/Bs, so that a runner's speed moves both sides alike: `main` at `97feb4dc` against `327cfaa4`, a commit of wave 12 after the tenth tooling step's layout (Windows' tiers 1 and 2 two at a time, its system tests in two lanes), three rounds of each tree (`try` 44632492422). Each tier is its tests' time above the start of their processes, the median of the rounds; the reports were read test by test, the crates' names from before the rename to Cineris taken as their new ones.

| Tier | Linux, wave 12 (6 ms) | Linux, `main` | Windows, wave 12 (8 ms) | Windows, `main` | Budget |
|---|---|---|---|---|---|
| 1, unit | 15.4 s (2,521 tests) | 27.7 s (3,199) | 15.7 s (2,500) | 24.7 s (3,178) | 20 s |
| 2, integration | 29.0 s (1,442) | 34.5 s (1,557) | 37.0 s (1,431) | 45.2 s (1,546) | 40 s |
| 3, system | 26.7 s (270) | 37.4 s (314) | 81.2 s (185) | 104.7 s (215) | 60 s, 90 s on Windows |

**Tier 1.** Almost all of the growth is new tests: 680 of them, +11.1 s of Linux's +12.4 and +8.8 s of Windows' +9.0. The tests that were there in wave 12 took what they took then.

- `cineris-tui` +8.6 s on Linux, +6.1 s on Windows (339 tests to 696): the panels' sweep of the live region, two tests, 2.4 s on Linux and 1.6 s on Windows; the first run's and the screen's application tests, at a floor of 16 to 20 ms each on Windows against 5 ms for the others (a set-up worker, the grammars loaded once a process); the markdown properties of the flaky-test fixes.
- `cineris-cli` +1.8 / +1.5 s: the prompt history's tests, which wrote files of 8 to 16 MB, its real bounds, every run (1.5 s on Linux, 1.1 s on Windows).
- `xtask` +1.6 / +1.0 s: the rename tool's test of a whole tree (0.5 s), `replace`'s scripts, many small tests.
- **On Linux the tier is also the runner's load.** Linux ran tiers 1 and 2 on every core of its 4-core runner, and the same tree's unit tier read 16 to 29 s from one `main` run to the next. Its CPU-heavy tests slowed each other most.

**Tier 2.** `cineris-core` +3.5 s on Linux and +6.5 s on Windows: 79 new tests (trust, settings, reload, rewind plans). On Windows each of the core's integration tests costs 28 ms above its process's start before it does anything (a core started, 6 ms, and a session opened, 23 ms, by its time profile: `try` 44633486139), 7 s of the tier in all; on Linux the whole test takes about 10 ms. The open writes no file (counted with a throwaway test); where the 23 ms go on Windows is not yet known. `cineris-rpc` and `cineris-term` add about a second each.

**Tier 3 on Windows,** +23.6 s, +17.5 s of it new tests:

- `cineris-cli` +12.4 s. `first_run::sys_the_first_run_in_an_empty_home_reaches_a_working_session`, new, 4.8 s, as long on Linux (4.4 s): the first run's readiness rule in real time, its grace of 0.3 s and a second after each answer for four steps (its time profile: the provider 0.37 s, the key 2.0 s, the imports 1.0 s, the mode 1.0 s). `serve::sys_serve_stdio_conforms_to_the_core_in_process` 1.8 s to 3.7 s (27 git starts). `tui::sys_typing_through_an_approval_over_attach_does_not_answer_it`, new, 1.25 s, a dialog's quiet second.
- `xtask` +5.6 s, and 14.4 s in all for the git-heavy tests of tools run locally or on Linux CI: the owner guard's six (about 40 git starts each), `rebase`'s, `branch`'s, `mutate --changed`'s, `git`'s, and `rename`'s two. `checkout_dates`' two (3.4 s), which the Windows job itself runs, and `rules`' (2.4 s) are needed there.
- `cineris-core` +3.5 s: the trust tests (the store test 2.7 to 4.2 s, 27 git and 6 PowerShell starts), and one test that pays Windows PowerShell's first use of a module (below).
- `cineris-tools` +1.1 s: each shell's own tests of `cmd /c` (PowerShell 7 and Windows PowerShell), as they must be.

**The leads given, checked.**

- `gates::sys_a_command_that_only_reads_is_told_to_the_loop_as_changing_nothing` took 2.5 to 3.1 s in every `main` run on Windows, and 0.03 s on Linux. It starts Windows PowerShell twice. Its first command was `Get-Location`, and the first `Get-Location` on a machine has Windows PowerShell look through its modules for it: on one runner (`try` 44634570478, five rounds) the first run on the machine took 2.9 s and every later one 0.6 s, of either side. No other test runs a command of that module, so on a fresh CI runner this test paid it every time.
- The two tests at 44 s and 22 s in one of the size step's runs were the runner, as its A/B said.
- The process-start rule moved no test into tier 3: no system test of `main` was in wave 12's reports in another tier under its name without `sys_`, on either system.

**The build.** A pull request that touches `cineris-base` builds every crate of the workspace: on `main` the build of the tests took 150 s (run 44592310472), with `xtask` itself built first in the `dev` profile, 27 s, because it depends on `cineris-testkit` and so on `cineris-base`. With the set-up (about 35 s) and both test runs (55 s each on an 8 ms runner) that is about 337 s. The size step's pull request (#113), which in the end did not touch `cineris-base`, built nine crates in 69 s and took 197 to 249 s.

## What was changed

1. **Linux's tiers 1 and 2 run two at a time** (the owner's decision of 2026-11-19, asked of the maintainers with the measurement). It changes how the tier is measured, not what its tests cost: the growth is the new tests', and they are read with less of each other's load in them. It holds on every Linux machine, a local run's too. On one 4 ms runner, three rounds of each layout (`try` 44634574569): the unit tier 15.9 to 10.6 s (−33%), the integration tier 21.1 to 14.9 s (−29%), the system tier unchanged, and the two test runs' wall time 26.5 to 32.4 s (+6 s) in a Linux job of 130 to 160 s. It is the layout Windows has had since the tenth tooling step: the group `tiers-linux` in `.config/nextest.toml` ([testing-and-ci.md](../../testing-and-ci.md), "Each tier is measured under its own load").
2. **The git-heavy tests of tools run locally or on Linux CI run on Windows in the nightly,** as `combine`'s already did: ignored on Windows by the blocking job, with a reason that says the nightly runs them, and blocking on Linux as before. The nightly's Windows job runs them with `cargo xtask git-tests-on-windows`, which runs each by its exact name and fails unless each one ran and passed; a unit test holds the list (`GIT_TESTS_ON_WINDOWS`) to the tests marked, path for path. 14.4 s of Windows' system tier on an 8 ms runner (`try` 44632492422, the median of three rounds), 12.3 s in `main`'s own CI runs (the median of 44592310472, 44612738735, 44619868787 and 44622100036). What Windows loses on a pull request: a change to one of those tools that breaks on Windows alone is seen the next night, not before the merge. The owner guard is the one that runs on Windows without being asked: it is the pre-push hook, so it runs on every push from a Windows checkout, and a fault of its own on Windows would now be found by the nightly, while Linux CI's owner-guard step still checks every pull request. The others run on Windows where a person runs them there by hand. The Windows job runs `checkout_dates`, whose tests stay. One way a test could leave Windows unseen is left: a reason reworded so that it no longer ends with the nightly's words would not be held to the list.
3. **The read-only gates test runs `echo` on Windows,** which every other test's command already does, and not `Get-Location`: 2 to 3 s of a fresh runner's system tier. It pins what it pinned, on both systems: the loop is told a command the table finds read-only changes nothing, and the tool does not look for changes; a command that writes is looked at. Its break, `step-13a/read-only-allow-told-as-allow`, is caught on Windows (below, "Breaks").
4. **The prompt history's tests run at bounds of their own.** A history opens within a `Limits` value: the program's constants by default (`FileHistory::open`), smaller ones in a test (`open_with`). The tests' bounds keep the program's order and are each still past one prompt's most (a 64 KiB chip or text), so that every rule is reached as it is at the real sizes, with files of 1.6 to 4 MiB where they were 8 to 16 MB; a test holds both sets to that order. Before the change the five tests took 70 to 560 ms each on a Linux runner (`main` in the one-runner A/B, `try` 44632492422, the median of three rounds); after it, the thirteen history tests took 46 ms together in one run on the reference machine (`cargo xtask test -p cineris-cli -E 'test(/history::tests/)'`, at the bounds as the review left them), and the branch's own A/B has them on the runners ([ci.md](ci.md)). A test the review asked for is added: `a_file_of_one_project_is_written_again_within_its_bytes`, the bound on each project in a file written again, which no test reached at any size, the file's own bound coming first with three projects (its break is `ci-budgets/a-project-written-again-past-its-bytes`). `step-23-followups`' breaks of the bounds now name the code as it reads.
5. **The flaky trust-dialog test,** below.

## The flaky test: `tui::sys_a_project_is_trusted_from_its_dialog_and_its_model_is_used`

**What failed.** In the twelfth tooling step's job A/B (`try` 44623384932, round 1, the head's system tests) it waited 30 s for the shell's verdict after `Ctrl+D`. Its screen showed the input box holding `[?0u`.

**The cause.** `[?0u` is the end of the terminal's reply to the enhanced keyboard protocol's query, `ESC [ ? 0 u`, one of the queries the binary sends as it takes the terminal. Through ConPTY input comes about a key at a time ([conpty.md](../../conpty.md)), and on a loaded runner the reply's ESC came more than 50 ms (`ESCAPE_WAIT`) before its `[`. The decoder took the ESC as the Escape key, which the trust dialog in its grace does nothing with, and typed the rest into the input box; `Ctrl+D` quits only from an empty one. The test's "trust question" phase took 1,030 ms, where the whole test takes 0.35 s on Linux: the dialog waited a quiet second after input it did not take, which fits the reply's arriving within its first few hundred milliseconds. A real terminal on Windows sends its replies through ConPTY too, so a person on a busy machine could have seen the same.

**A second test, the same cause.** In this fix's own job A/B (`try` 44634824935, round 1, `main`'s side, a cold runner), `cineris-tui`'s `external::sys_a_failing_editor_keeps_the_prompt_and_gives_the_terminal_back` waited 20 s for its child to wait for a key, and the screen showed the shell's `restored: yes (exit code: 0)`. After the editor the child takes the terminal back, which sends the queries again, and ends at the first key it reads: a reply's ESC split from its rest would be that key, and the child ends with code 0, as the screen says. It is the only reading of that screen we found; it is not proved by a trace. **The decoder's fix covers it:** the new `Terminal` sends the queries again and makes its replies due afresh. `terminal::tests::a_terminal_entered_again_waits_for_its_own_replies` (a terminal answered and left, then entered again, the split reply typing nothing) holds that nothing of the first terminal's carries to the next: today each keeps its own, so the test is a guard against shared state to come more than a test of a new path.

**The fix** (`cineris-term`, `Terminal::escape_deadline`). While the replies are due, a lone ESC waits `REPLY_ESCAPE_WAIT`, 500 ms, for the rest of its sequence, counted from when it came. The replies are due until the device attributes come (every terminal answers that query, and it is sent last), and only for an ESC that comes within `REPLIES_DUE`, 3 s, of the queries, so a terminal that never answers is held to it no longer. After either, a lone ESC waits its 50 ms as before. The first shape, 500 ms counted from the queries, was changed on the early review's finding: the test kit answers the queries only when it reads them, ConPTY passes the answer on a key at a time, and the `?0u` reply's ESC is about 39 bytes into the replies, which at 16 ms a key would put it near 600 ms (an estimate, not a measurement). The maintainers accepted both shapes; this one is the record's.

- **What a person sees:** an Escape pressed before the terminal's last reply (in practice milliseconds after the start; at most its first 3 s) comes up to 0.5 s late, and an Escape followed by a reply within that time is taken with the reply, as an ESC before any sequence is (it is Alt). Neither matters at the start: every dialog there waits its 0.3 s grace and a quiet second before it takes a key.
- **Every place that sends a query:** the queries go out only when a `Terminal` is made (`with_device`); the external editor gives the terminal back by making a new one (`Terminal::enter`), whose replies are due afresh. A resume after `Ctrl+Z` and a resize send none, and nothing in `cineris-term`, `cineris-tui` or `cineris-cli` asks for the cursor's position or anything else.
- **Tests** (`cineris-term`'s `terminal::tests`, the scripted device and its clock): a reply split after its ESC while the replies are due types nothing; the same split after the last reply is the Escape key and typed text, as before; with no last reply an ESC waits 500 ms from itself, one that comes just before 3 s waits it whole, and one after waits 50 ms; the last reply ends the longer wait. The three tests of the 50 ms wait read the last reply first. Their breaks are in `mutations/ci-budgets.breaks`.

## Breaks

`mutations/ci-budgets.breaks`, run with `cargo xtask mutate ci-budgets` on the reference machine: **8 of 8 did what the list expects** (the four of the ESC's longer wait, the history's two, and the two of the git tools' list and its verdict). Every break on `crates/cineris-cli/src/history.rs` in any list was run again after the review's first finding, twelve of them (`step-23/a-history-file-kept-open`, `step-23-fixes`' three, `step-23-followups`' six, and this step's two): **12 of 12 caught**, `step-23-followups/a-project-holds-past-its-bytes` among them, which the bounds as first shrunk would have let survive.

The tests moved off Windows' blocking job are unchanged, and their breaks are caught on Linux as before; on Windows the nightly runs them, and `cargo xtask git-tests-on-windows` was run on Linux, where they are not ignored: 12 of 12 ran and passed. The gates test's break, `step-13a/read-only-allow-told-as-allow`, was run on a Windows runner with the test's new command (below, in [ci.md](ci.md)).

## Reviews

**An early, focused round on the decoder's change** (a reviewer that had not seen the reasoning, as the maintainers asked before anything was built on it). It agreed with the rule, with one reservation, taken: 500 ms counted from the queries could end before a late reply came (the shape above was changed to the ESC's own wait). Its other findings: an Escape held before a reply is taken with it, as Alt is, which the constant's text now says; and one break of the deadline's two ends no test caught, which `the_last_reply_ends_the_longer_wait_of_the_next_escape` and the third part of the no-reply test now do.

**The full round before the first push,** on the whole branch:

| # | Finding | Verdict | What was done |
|---|---|---|---|
| 1 | The history's smaller bounds left `read`'s bound on a project's bytes unreached: the fixture's file, of three projects in turn, held 369 KB of each against a `keep_bytes` of 512 KiB, so `step-23-followups/a-project-holds-past-its-bytes` would have survived | Right; this record's break run had covered only the step's own list | `keep_bytes` 384 KiB and `compact_bytes` 1.5 MiB (each of three projects past its bytes holds 512 KiB), `read_limit` 4 MiB; the order test holds `compact_bytes / 3 > keep_bytes` and a prompt's most as text and chips together. Every break on `history.rs` was run again (below, "Breaks") |
| 2 | The record unfinished: its last sections empty, `ci.md` a heading, a break misnamed | Right | Finished; the break is `step-13a/read-only-allow-told-as-allow` |
| 3 | Two timings without their runs | Right | Each names its runs or its command |
| 4 | The Linux layout holds on every Linux machine, and changes how the tier is measured, not what its tests cost | Right | Said in [testing-and-ci.md](../../testing-and-ci.md) and above; `--like-runner`'s Linux factors are named as wanting measuring again |
| 5 | The record understated what Windows loses: the owner guard is the pre-push hook | Right | Said above |
| 6 | The nightly step would not run after an earlier step failed; the command ran nothing on a system where the tests are not ignored; a reason reworded escapes the list | Right | `if: !cancelled() && runner.os == 'Windows'`; `--include-ignored`, and the command was run on Linux, 12 of 12; the reworded reason is left, and said |

**The second round, on the changes since:**

| # | Finding | Verdict | What was done |
|---|---|---|---|
| 1 | The record's Breaks, its last section and `ci.md` still empty | Right, as the round read it | Filled: the breaks' verdicts above, the CI times in [ci.md](ci.md), what is left below |
| 2 | The debug-info commit left decision 0012 and the workspace page saying `ci` keeps line tables | Right | Both amended in that commit, so that it can still be dropped whole |
| 3 | The re-entered terminal's test is close to the fresh one's: the two terminals share nothing, so it guards against shared state to come; the record said the fix "fixes" what it only covers | Right | Kept as that guard, and said so; "covers" |
| 4 | The history item's sizes and its "before" timing loosely said | Right | The sizes as they are; before and after named |
| 5 | Compaction's bound on each project's bytes was reached by no test at any size (before this branch too) | Right | `a_file_of_one_project_is_written_again_within_its_bytes`, and its break, caught |

The round checked the arithmetic of every history fixture against the bounds as they now are, and found each break on `history.rs` reached.

## Left for the owner
