# Answered in the eighth tooling step, on 2026-11-16

This step is not a numbered step of the build plan: it built the eighth round of developer tooling, from what wave 11's seven sessions asked for. Its records are here, in a folder named for the job.

## The flaky dialog test on `main`

`cortex-cli`'s `tui::sys_a_projects_first_limit_is_listed_and_in_force_once_trusted` failed on the desk's fast builds. The cause was the test's own: after its `y` it waited for the header with `t.shows("Cortex-Harness")`, which the trust question's path matched wherever the temporary folder lies inside a copy of the repository named after it (the desk's `rx` copy of the main checkout). The wait passed at once, the test typed `go` and Enter microseconds after `y`, and the application read all four in one read, which it takes as typing (`App::burst`): the `y` went to the input box and the dialog stayed. A key log in the application showed it in 5 runs of 5 with the wait removed. The dialog's grace and what the product shows did not change.

The fix (pull request 64): the header waits match the title and version; every dialog answer of the system tests goes through `cortex_testkit::pty::Session::answer_dialog`, which waits for the answers row, types the key alone, and waits for the row to go and for what only the answer brings; the two undo tests wait for the idle input box before `/undo`, which they used to send while the turn was still running (16 failed rounds in 30 under `--cores 2 --load 6`). After it, the 13 `tui::` tests passed 50 rounds plain and 50 under load at the desk, and 10 rounds on Windows (`try` 43844567358).

The review of the fix found that `cortex-agent` emits `turn.completed` before it clears `inbox.running`, so a `/undo` sent the moment the idle box shows can still be refused. The coordinator gave it to step 23's fix session.

## What was built, item by item

The work after the dialog fix was shared out to four sub-agents (stalls, `mutate`, measurement, the test kit), each in a worktree of its own with a breaks list of its own, merged with `mutate --merge`; the session kept `apply`, `decision new`, `ci-times`, the rough edges, the records and the reviews. It went in as three pull requests, each on the one before: about 4,000, 2,700 and 4,600 lines.

| Item | Built | Where it is documented |
|---|---|---|
| 1. The flaky dialog test | Above (pull request 64). | testing-and-ci.md, "Testing the TUI" |
| 2. The shell guard | `cargo xtask apply SCRIPT`: `replace --script`'s edits, `@@ write:` and `@@ delete:`, and `@@ check:` lines (`fmt`, `rules`, `links`, `check STEP…`, `build`, `clippy`, `test`), all or none: nothing written when a change cannot be made, every file put back (a made file removed, a deleted one restored with its permissions) when a check fails, unless `--keep`. The refusals wave 11 met, tried again, each with the form that works. | shell-guard.md, "One command for an edit and its check"; AGENTS.md; CONTRIBUTING.md |
| 3. Stalls | `cargo xtask ci` runs a cluster of late timeouts again, once, when every task waited on I/O for 10% or more of the run (`/proc/pressure/io`), and says so; a recurring timeout, any other failure and a missing test still fail. The tier check does not judge the tiers of a run that stalled, at a desk; CI is judged as before. `CORTEX_PRESSURE_FILE` reads a file in place of the kernel's, for the tests. | testing-and-ci.md, "Test tiers" and "Running the tests many times over" |
| 4. Verification time | `mutate --changed REF`: only the breaks the diff can affect (their file, a test they name, the break itself, the build's settings and anything outside the known folders; a name it cannot place, a pattern, or a non-Rust file of the break's crates chooses it), with how many it left. The `in-string:` key (and `--in-string`): a word inside a string literal. | testing-and-ci.md, "Breaking the code on purpose" |
| 5. Measurement | `cargo xtask perf-ab` and `try … command="perf -p …" -f against=REF`: two trees' ignored `perf_` tests on one runner, interleaved, three rounds at least, each side's median and range and the difference (`cortex_testkit::numbers::report`); `try` with `size` and a profile; `size --breakdown`, `--at COMMIT`, `--profile release`, `--base HASH`; ignored `perf_` tests of search, a re-read's diff and JSON decoding on real code. The budgets were proposed, not set (the item `perf-budgets-for-search-diff-and-decoding`, settled on 2026-11-16: the owner set them as proposed, with Windows' search at 500 ms, and the tenth tooling step holds the tests to them). `performance-budgets.md` says the size check measures the musl build on Linux. | testing-and-ci.md, "Two trees' performance tests on one runner", "The binary's size by crate and at a commit"; performance-budgets.md |
| 6. The test kit | `cortex_testkit::exit` (`Exit`, `Verdict`: either system's wording of how a process ended); `cortex_testkit::glyphs` (`GlyphSets`, `RowPattern`: either glyph set, from the product's tables); `Session::close_console` (Windows `ClosePseudoConsole`, Unix SIGHUP and SIGCONT to the group) and two tests of the application's answer; `cortex-tui`'s `assert_app_frame` snapshot helper, with four dialog frames on it; five more compaction tests sized from the measured prompt (`Rig::fill`) where sizes were typed by hand. The "within 5% of a threshold" warning was not built: which number is a test's threshold is the test's meaning, not its result. | testing-and-ci.md, "Testing the TUI"; conpty.md |
| 7. Records | `cargo xtask decision new "TITLE"`. | CONTRIBUTING.md, AGENTS.md |
| 8. CI | `ci-times` marks the jobs that block nothing, and a test fails when `ci.yml` has a job of neither kind; a change to the size job runs it on its own pull request; the size job says when `main`'s record is not of the base. | testing-and-ci.md, "A CI run's test times", "Each pull request's binary size" |
| 9. Rough edges | `try-result --os` judged by its job; `ci-times --ab` writes an OS it left out; `scratch-test` refuses a flag given on both sides of `--`; the corpus pin's two repairs; the test of a second hook hold; the `mutate` jobs race; `os:` breaks' names looked for in the source. | the items below |

## The independent reviews

One review of each pull request (Opus, none having seen the session's reasoning). Fixed, each with a test and a break:

| Pull request | Finding | Done |
|---|---|---|
| Dialogs | The helper's "answers gone" could read a frame split across reads; the undo test's check of the file name passed on the tool line; the docs said hosted runners were exposed. | `answer_dialog` also waits for what only the answer brings; the screen of the question is read; the docs corrected. |
| First | A stall of the system tests' run excused the budgets of tiers 1 and 2, measured by the other run. | Each record names its run's tiers; only a stalled run's tiers go unjudged. |
| First | `apply` took a file that cannot be read as absent (putting it back deleted it); one file spelt two ways lost a change; a deleted file came back without its permissions. | Each mended. |
| First | `decision new` escaped the title and not the status, and could leave a record without its row; the docs said the re-run is rarely live on CI (Ubuntu's kernels give the pressure). | Mended; docs corrected. |
| Second | `mutate --changed` skipped a break named by a pattern ending inside a module's name (`open*`), a change to `.cargo/config.toml` or a file a build script embeds, and a fixture of the break's crate. | Each now chooses the break. |
| Third | The Windows size job ran in PowerShell, where `$BASE` is empty: it would have failed every pull request that changes the binary. | The step runs in bash. |
| Third | `size --at` took `core.autocrlf` from the machine; a dist build gave build scripts the `gh` token; the Unix hang-up could signal a reused group; a Windows console whose closing thread failed was leaked; two rounds of `perf-ab` read `apart` a third of the time. | Each mended; three rounds at least. |

What needed no mending is in [tests-the-eighth-tooling-step-left-thin](../../outstanding/tests-the-eighth-tooling-step-left-thin.md): the re-run's wiring in `ci`, the Windows close tests (which the console host's own end also satisfies), the `os:` names of a test both systems have, `perf-ab`'s missing rounds, `size --at` inside the workspace.

**Known flake, not this step's:** `tui::sys_the_console_closing_*` (and the older SIGHUP test) can end with exit 1 when the signal lands during a frame's write, about 1 run in 600 under `--cores 2 --load 6`: step 23's review finding 5, fixed by its fix session's first pull request.

## Open items this step settled

- **`a-mutate-jobs-test-that-once-finished-every-piece`:** the cause was in `jobs.rs`: a job took its next piece without waiting for the run's thread to write down the answer "time is up", and the test opened its gate from inside the question. Taking a piece now shares a lock with asking the time. 200 stress repeats pass, alone and under `--cores 2 --load 6`; two breaks of the lock are caught by the test.
- **Parts of `tests-the-seventh-tooling-step-left-thin`:** the test of a second hold now sees the second waiting before it ends the first (the registry counts the waiting holds), so a hold that never waits, or one never woken, fails it; the `caught-by` names of an `os:` break are looked for in the words of the source on the other system. The rest of the item stays.
- **Parts of `rough-edges-of-the-seventh-tooling-steps-commands`:** `ci-times --ab --write` writes an OS it left out as its note; `scratch-test` refuses a cargo profile or ignored tests asked for on both sides of `--`; a `corpus/PIN` that is not a pin is an error that says `corpus pin` mends it, and `pin --update` keeps `corpus/Cargo.lock` until the new one is made. The rest of the item stays.
- **Parts of `the-pr-size-jobs-rough-edges`:** `ci-times` marks the jobs that block nothing (a test fails when `ci.yml` has a job of neither kind), and a change to `ci.yml`, `main.yml` or the `changes` action runs the size job on its own pull request. The rest stays.
