# Answered in the ninth tooling step, on 2026-11-16

This step is not a numbered step of the build plan: it built the ninth round of developer tooling, from what wave 12's sessions asked for, after making room in Windows' tiers 2 and 3. Its records are here, in a folder named for the job.

## Room in Windows' tiers 2 and 3

### Where the time went

- **The baseline:** `main` at `2741c8e` on a 9 ms runner, each test's time profile kept (`try` 43881988635, `-f testkit=profile`): tier 2 at 40.8 s of its 40, tier 3 at 91.3 s of its 90. By crate, tier 3 was `cortex-core` 25.3 s, `cortex-tools` 23.9, `cortex-cli` 19.1, `xtask` 12.1 (the owner guard's git), `cortex-policy` 3.9 and `cortex-session` 3.3; tier 2 was `cortex-core` 10.0 s (a session's open took 18 ms on Windows), `cortex-llm` 5.9, `cortex-tools` 4.9, `cortex-session` 4.3 and `cortex-agent` 4.0.
- **Each test's process starts, counted on a runner.** Every place that starts a process now marks it (`cortex_base::hook::started`, nothing in a product build), and `try -f testkit=starts` writes each test's starts down ("What a runner would say, from a desk" in testing-and-ci.md). Tier 3 on `main` (`try` 43883759862) started git 178 times, the binary 129, PowerShell 7 105, `cmd.exe` 56, a test binary in a role 54 and Windows PowerShell 15. Fitted to each system test's time by least squares, with a term for each crate: **a PowerShell 7 start cost 0.44 s, Windows PowerShell 0.31 s, a test binary 0.08 s, git 0.054 s, `cmd.exe` 0.024 s and the binary 0.022 s** (its own start; what it then does is the test's). PowerShell 7 was about 46 s of the tier's 92.
- **The binary asks PowerShell 7 its version on the side** the first time it runs on a machine, and every system test's machine is a first: one start of 0.44 s in nearly every `cortex-cli` test, whether or not it runs a command. Windows PowerShell is asked nothing (its version is not told to the model).

The slowest tests of tier 3 and their starts, on that run:

| Test | Time | Starts |
|---|---|---|
| `cortex-core` `trust::sys_a_project_trusted_in_the_store_is_trusted_while_its_rules_are_the_ones_the_user_saw` | 2.47 s | git 27, pwsh 6 |
| `cortex-policy` `differential::sys_a_program_receives_the_words_the_analysis_gives_from_every_powershell` | 2.52 s | 120 echoing children and 3 PowerShells, which the test starts itself |
| `xtask` `owner_guard::local::tests::sys_what_the_machine_or_another_remote_could_hide_is_still_read` | 1.53 s | about 40 git, started by the test and the guard |
| `cortex-tools` `shell::pwsh::sys_the_exit_status_is_the_commands` | 1.47 s | pwsh 5 |
| `cortex-tools` `shell::powershell::sys_hostile_arguments_arrive_unchanged` | 1.41 s | powershell 1 |
| `cortex-core` `knowledge::sys_a_test_that_fails_then_passes_with_nothing_changed_becomes_a_flaky_entry` | 1.13 s | pwsh 4 |
| `cortex-cli` `signals::sys_a_run_with_no_console_completes` | 1.40 s | pwsh 2, git 1, the binary 1 |
| `cortex-cli` `tui::sys_undo_asks_about_a_file_edited_since_and_puts_it_back` | 1.18 s | the binary 1, pwsh 1, git 1 |

### What changed

- **Windows PowerShell for the tests that are not about which shell.** The core's rig, `cortex-tools`' `default_shell()` and `cortex-cli`'s `Machine::user_config` give Windows PowerShell on Windows. Each shell's own tests are unchanged. The shell the harness picks itself (PowerShell 7 on a runner) stays covered end to end by `cortex-core`'s `shell::sys_undo_puts_back_what_a_command_changed_where_git_still_has_it` and `shell::sys_a_command_is_given_the_environment_the_command_analysis_relies_on`, `cortex-cli`'s `tui::sys_a_turn_is_interrupted_mid_stream_and_mid_tool` and `signals::sys_a_run_with_no_console_completes`, and `cortex-tools`' `process::sys_the_polite_interrupt_is_enough_for_a_command_that_listens`, where how the shell takes a console event matters. The coordinator agreed to this on condition that one end-to-end test each in `cortex-core` and `cortex-cli` keeps the product's pick.
- **The trust tests refuse a `git status` that is asked about**, rather than allowing it: whether it is asked is what they read of it, and a refused command starts no shell. Where it is not asked it still runs. The core's rig has `turn_answering(text, choose)` for it.
- **Windows' system tests in two lanes of one test each:** the PowerShell-heavy crates' (`cortex-core`, `cortex-tools`, `cortex-policy`) one at a time beside one of the others, in place of any two at once. On one runner, five rounds of each against the same tree two at a time, the tier read 57.8 s at the median against 64.4 s (−10%), with the run's wall time 30.4 s against 32.2 s (`try` 43888750300). One at a time in one lane was tried first: 6.5 s off the tier for 22 s more of the run's wall time (`try` 43884135417), and not taken.
- **Not changed, and why:** the owner guard's tests start about 40 git each, but most of those starts are the guard's own work, or a test's history that the guard reads. Sharing a template repository between processes would save about 1.8 s of a 9 ms runner's tier, with a cache shared across test processes; it is not done. `cortex-policy`'s comparison with every PowerShell starts the echoing program once per line, which is what it tests. The shell tool's own tests (`shell::pwsh::…`) start PowerShell once per case, and each case is about what one start does.

### What it came to

**On a 9 ms runner, five rounds of each tree (`try` 43891258419, the branch at `3c1357c` against `main` at `c0d6356`): tier 3 went from 85.2 s to 72.1 s (−13.1 s, −15%), at the target of 72 s.** By crate: `cortex-core` 25.3 → 17.4 s, `cortex-tools` 21.6 → 18.3, `cortex-cli` 17.5 → 14.6; `xtask` 11.7 → 12.5, in the other lane with `cortex-cli`. Tiers 1 and 2 did not move beyond the noise (+0.6 and +1.0 s). The tables are in [ci.md](ci.md). Earlier measurements of steps of the same work: on a 7 ms runner, three rounds (`try` 43886546354), 73.5 s to 59.7 s with two at a time; the system run's wall time went from 44 s to 38 s.

The slowest tests and their starts after it (`try` 43892837490, the same tree as the pull request's with the next pull request's tools, a 9 ms runner): Windows PowerShell 67 starts and PowerShell 7 25, where PowerShell 7 had been 105; the trust test of the store 2.1 s (git 27, Windows PowerShell 4: two refused); the shell tool's PowerShell 7 tests unchanged, which start the shell they test.

### Tier 2: the target not met, and why

Tier 2 is about 38 to 41 s on 9 to 10 ms runners, against the 32 s target. No test change found takes 9 s off it without testing less: the time is a few milliseconds of the operating system's in each of about 1,360 tests (the seventh tooling step found the same). A session's open on Windows, 18 ms against 1.4 ms at a desk, is the largest single share (2.6 s over 143 opens in `cortex-core`), and is product code in `cortex-core`, where other steps are working.

**What would meet it:** tiers 1 and 2 two at a time on Windows, in place of three. On one 6 ms runner it took the integration tier from 28.1 to 22.2 s (−21%) and the unit tier from 13.8 to 9.5 s, for 5.8 s more of the run's wall time (`try` 43884288025, three rounds of each). It reverses the owner's choice of three in the seventh tooling step. **Proposed, pending the owner:** it is on a branch of its own, `worktree-step-02-tooling-9-two`, ready as a one-change pull request ([windows-tiers-one-and-two-two-at-a-time](../../outstanding/windows-tiers-one-and-two-two-at-a-time.md)).

## The tools: what was built, item by item

One line a sentence, in the style this step set for the docs ([CONTRIBUTING.md](../../../CONTRIBUTING.md), "Writing the docs").

- **A hosted-cost estimate at the desk** (item 2):
  Every place in the workspace that starts a process marks it, `cortex_base::hook::started(program)`, which is nothing in a build without `test-hooks`.
  With `CORTEX_TESTKIT_STARTS` naming a directory, each process writes down the starts it made, for the test nextest says it runs, and a test's children write for it too.
  `try -f testkit=starts` keeps them on a runner, and `cargo xtask ci --tiers --like-runner` keeps them at the desk and prints each tier's starts by kind and what the Windows runner would charge for them.
  The Windows estimate of a test that started processes is its desk time less its starts' desk cost, times its crate's factor for other work, plus each start at the runner's price.
  `--base DIR` runs and estimates another tree too, and warns when a Windows tier's estimate grows by more than 3 s.
  The prices were fitted to a 9 ms runner's run of this step's tree (`try` 43892837490): PowerShell 7 0.33 s, Windows PowerShell 0.28 s, a test binary in a role 0.15 s, git 0.037 s, `cmd.exe` 0.026 s, the harness's binary 0.010 s.
  The work factors of the system tier were measured so that the desk's run of the tree is estimated at the runner's figure, crate by crate: the desk's run (`rx --exclusive`) against the same runner run; the desk's disk stalled in its integration tier (every task waiting on it 73% of the time), which is not read.
  On that tree the system tier is estimated at 99.6 s, where the runner read 72.8 s: the tests that run on one system only start different processes on each, and `cortex-core`'s Linux-only tests (the sandbox's, the adversary) start more at the desk than its Windows run costs.
  The tool is for the change a branch makes (`--base`): twelve more shell starts are 3.3 s of Windows' system tier, warned of; step 17's 34 would have been 9.4 s.
  Documented in testing-and-ci.md, "What a runner would say, from a desk".
- **The core's rig** (item 3):
  `front.approval()` fails at once when a turn ends, none is running after it and nothing was asked, naming the turn's outcome, where it waited a minute for nextest.
  `turn_denying` refuses what is asked, beside `turn_allowing`, both on `turn_answering(text, choose)`; `knowledge.rs`' own copy is gone.
  `front.told("…")` waits for a notice and gives it back, named in the timeout report while it waits.
  **Typed outcomes, not a lint:** `front.results()` gives an `Outcome` that fails the test when it is compared with a text no tool outcome has (`"ok"`), so the step 17 mistake fails at once in every core test and needs no rule that reads source text.
  Documented in testing-and-ci.md, "A reviewer's quick start".
- **An A/B that survives a test failing on one side only** (item 4):
  A round in which a side's runs failed only by tests failing is compared without those tests, left out of every round on both sides; a test that failed on one side is listed under `N TEST(S) FAILED ON ONE SIDE ONLY`, and the run is still a failure, naming it.
  `ci-times --ab` and `try-result` read the rounds by the same rule.
  Documented in "Two trees on one runner".
- **`ci-times --pr N`** compares with `main`'s run at the commit the pull request is based on (GitHub's compare of `main` and the run's head), and says when it falls back to `main`'s latest; a matrix job skipped before its matrix was made (`pr-size-${{ matrix.name }}`) has no row.
- **`pr-size`** names the three crates that grew most when the binary grows by more than 100 KB against `main`'s record, on Linux, from a breakdown of the build against the bytes by crate the `main` workflow's Linux size job now records; the Windows job says the Linux job names them.
- **`mutate --changed`** ends with exit status 3 when it cannot read the diff (no `.git`, one git cannot read, one whose work tree is another folder, a reference that is no commit), apart from 1, a break left uncaught.
  The silent failure the coordinator saw in an `rx --side` copy was not reproduced: the copy has no `.git`, which was refused before this step too, but with exit status 1, which a script reads as a break left uncaught.
- **A dispatched run's id:** `cargo xtask try --dispatch BRANCH --command "…"` checks the inputs as a dry run does, dispatches the workflow and prints the run it made; `try-result` reads a run whose jobs have all ended as finished, whatever the run's own status says.
- **Docs** (item 5): "A reviewer's quick start" in testing-and-ci.md; the As-built table of step 17 in 14-adaptation.md as a list with one sentence a line, and the rule in CONTRIBUTING.md, "Writing the docs".
- **`a-time-limit-test-under-load`** (item 6): the runner starts a command before it arms the call's time limit, so under load the test found the family up and the limit not yet armed; the test now waits for the limit as it waits for the family.
  Settled; its file is deleted.

## The test kits: what was built

Built by two sub-agents, each in a worktree with a list of breaks of its own, merged with `mutate --merge tooling-9 tui hooks`; the session wrote the re-decide test's fix.

- **`cortex-tui`'s test kit** (item 3, from the review of step 23):
  `cortex_tui::testkit`, behind the feature `test-kit`, turned on only as a dev-dependency (`cargo xtask rules` refuses any other way, as it does `test-hooks` and `pty`).
  It gives the scripted `Link`, a `SessionState` builder with sane defaults (`session().id(…).main_model(…).build()`), `caps()`, `app()`, `opened()`, `ask()`, `approval()` and `shown_dialog()`.
  The crate's own tests use it, so there is one copy, and a test in `crates/cortex-tui/tests/it/app.rs` uses it as another crate would.
- **Keys at a person's pace** (item 3): `cortex_testkit::cadence`: typing every N ms, a held key's auto-repeat (X11's and Windows' cadences), keys read together and a paste, as data with times for the application's own clock.
  There is no pseudo-terminal form: a program in one reads its own clock, so a pace there would need real waiting.
- **An awaitable library point** (item 3, from the re-decide step): `cortex_base::hook::await_point!(NAME)` in an async fn: with `test-hooks` an await of `point_async(NAME)`, which a `Hold` holds without stopping the thread; without it, nothing, not even an await (a ready future still costs a state of the future, measured).
  The test kit's `Hold::drive_until_reached` polls an operation until it is held.
  The core marks `BEFORE_THE_NEW_LEVEL_IS_IN_FORCE` in `Core::set_level`, and `redecide::a_revoke_made_while_the_level_changes_is_not_lost` holds it, where it counted the turns of the runtime.
- **`cortex-policy`'s test support** (item 3): `cortex_policy::test_support::TestParts`, behind the feature `test-support`, an `Engine`'s parts with sane defaults and setters; `cortex-core`'s policy tests and `cortex-policy`'s own copies use it.
- **The re-decide test under load** (found by this step's loaded stress run; the coordinator gave it to this step): `redecide::sys_a_copy_through_a_link_an_earlier_command_made_is_asked_about_again` failed once in 50 under `--cores 2 --load 6`.
  A round decides a call when it plans it, and under load the copy's planning could start after the link had run, so its first decision already saw the link and there was nothing to decide again: the product did what its documentation says.
  The round now has an awaited point before a call's tool runs (`cortex_agent::BEFORE_A_CALL_RUNS`, nothing without `test-hooks`), and the test holds the link there until the copy has been asked about, and checks that the link was held when it was.
