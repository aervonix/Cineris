# Answered in the sixth tooling step, on 2026-11-15

This step is not a numbered step of the build plan: it built the sixth round of developer tooling, from what wave 9's four sessions asked for. Its records are here, in a folder named for the job.

## Windows' system tier: where its time goes

Measured on scratch branches whose only difference from the step branch, or from `main`, is the measurement named; every scratch branch was deleted at the end.

**The profile** (`try` run 43710070420, the system tests alone with `-f testkit=profile`, a phase for each command the shell tool ran): tier 3 was 91.3 s, `cortex-tools` 38.0 s of it, `cortex-core` 14.7, `xtask` 14.0, `cortex-cli` 11.8. A PowerShell command through the tool took 0.5 to 0.65 s for `pwsh` and 0.3 to 0.4 s for Windows PowerShell, against 30 to 50 ms for `cmd.exe`; a 6,000-line output made with `ForEach-Object` 2.5 s for `pwsh`, and the broken store's 20,000 lines 4.3 s.

**The probes** (one throwaway test, `try` runs 43710659246 and 43713246255, each start timed alone and three at once):

| Start | Alone | Three at once |
|---|---|---|
| `pwsh -NoProfile …`, with the harness's first line | 226 to 288 ms | 440 ms |
| Windows PowerShell 5.1 | 125 to 195 ms (1.75 s the first time on a runner) | 288 ms |
| The same, with `POWERSHELL_TELEMETRY_OPTOUT`, `POWERSHELL_UPDATECHECK=Off`, `DOTNET_TieredPGO=0` or `DOTNET_gcConcurrent=0` | no change | no change |
| A host reading 20 commands on its input | 233 ms for all 20 (`pwsh`) | |
| `cmd /c exit 0` | 7 ms | |
| The test binary as a child | 5.5 ms | |
| `git --version` | 26 ms | |

So the cost is PowerShell's own start, paid by every shell call of a system test, and it rises by half when three start together; the runtime's settings do not touch it.

## Each PowerShell test: what it tests, and what changed

| Test (for `pwsh` and `powershell`) | Kind | Starts before | Starts now | What changed |
|---|---|---|---|---|
| `sys_hostile_arguments_arrive_unchanged` | The command line's encoding: every text quoted into one command | 2 | 1 | The texts through PowerShell (`ConvertTo-Json`) and to a program are one command: the same command line carries both, and both are checked. |
| `sys_the_command_reaches_the_shell_as_written` | The command line's encoding and the harness's first and last lines | 5 | 4 | Quotes after backslashes and the trailing back-tick are one command; each other case (an exit of the command's own, a failure before a back-tick, a command that does not parse) keeps its own start, since each is about one process's exit code. |
| `sys_the_exit_status_is_the_commands` | One process's exit code, five ways | 5 | 5 | Nothing: each case is one exit code. |
| `sys_the_environment_the_directory_and_the_input` | What the process is started with | 4 | 2 | The variables, the root's listing and the closed input are one command (each a property of the one start), between marker lines; the directory given with `cwd` is a second call. Also for `cmd.exe`, bash and sh. |
| `sys_output_too_large_to_show` | What the tool makes of a large output | 1 | 1 | PowerShell builds the 6,000 lines and writes them at once, where `ForEach-Object` took about two seconds. |
| `sys_output_is_merged_in_order` | The process's two streams | 1 | 1 | Nothing. |
| `sys_a_background_process_is_stopped_and_said_so` | The process tree | 1 | 1 | Nothing. |
| `sys_the_model_is_told_the_shell` | The version the shell reports | 1 | 1 | Nothing. |
| `process::sys_a_store_that_panics_loses_the_output_and_not_the_command` (the default shell) | What the tool does with a broken store | 1 | 1 | The 20,000 lines are made as above. |

No test was moved out of the blocking checks, no shell's cases run on fewer shells, and no assertion was weakened. A shared host was weighed and not used: every PowerShell start in tier 3 is the shell tool's own (the product starts a process for each call), or a test of what one process receives (`cortex-policy`'s differential tests, which already put all their lines in one script per shell), so a host would test something the product does not do.

The owner guard's tests (`xtask`, 6.7 s of tier 3 on a 6 ms runner) start git about sixty times each; the commit helper now reads the new commit's hash from the repository's files instead of asking git each time, a third of its starts.

## The owner's decision: Windows' system tests two at a time

The test changes alone took Windows' tier 3 from 93.2 to 80.4 s on one 10 to 11 ms runner (`try` 43715195861): 14%, short of the 65 s asked for. Running the system tests two at a time instead of three took 19% more off the same tree on one runner (57.6 to 46.6 s, `try` 43715198613), for 4.1 s more of the system run's wall time (20.1 to 24.2 s; `main`'s last green `test-windows` job took 3 min 42 s, CI run 43707430100). Three at a time was the owner's decision of 2026-11-15, so the coordinator took it to the owner, who **approved two at a time on Windows only** (2026-11-15): a group of its own, `system-windows`, chosen in the `ci` profile by an override for `cfg(windows)`; Linux stays at three. It is recorded as an amendment in [testing-and-ci.md](../../testing-and-ci.md), "Each tier is measured under its own load". What the two together did to `main`'s tree is in [ci.md](ci.md).

## What it came to

| Run | Runner start | Windows tier 3 above the start |
|---|---|---|
| `main`'s own CI run, 43707430100 | 11 ms | 93.4 s |
| The pull request's CI run, 43718965108 | 11 ms | 71.3 s (−24%); `test-windows` 223 s against `main`'s 222 s |
| `try` 43718003767, the branch against `main` on one runner, 5 rounds | 6 ms | 53.5 → 39.2 s (−27%) |

The 65 s asked for at 10 ms is about what the two A/Bs together give (80.4 s × 46.6 / 57.6 ≈ 65 s); the pull request's run, on a runner a millisecond slower, read 71.3 s. A 12 ms runner, which read every tier about twice a 10 ms one, would put it near 130 s, under the 135 s failure line and over the 90 s warning: the margin is there for the runners CI usually draws, and thin for the slowest. The open item is settled (`windows-tier-three-margin-and-the-powershell-tests`, deleted): it asked for one PowerShell start shared by several cases or tests, which is what the batching does where cases share an encoding or a start (a host shared by tests would test what the product does not do: every PowerShell start in tier 3 is the shell tool's own or the product's shell in a turn).

**The runner estimate** (`ci --tiers --like-runner`) was measured again, since its table was of three system tests at a time on Windows and read that tree's tier 3 at 84 s: from the desk's run of the branch at `5e2866b` against the branch's side of `try` 43718003767. The desk's disk stalled 54% of that run, so the file-heavy crates' factors are low; a quieter desk would make them better. Whole tiers now: Linux 1.4, 1.0 and 1.4; Windows 2.6, 1.8 and 6.4.

**The stall check, on a real stall.** `cargo xtask stress -p cortex-tools -p cortex-base -p cortex-testkit -p cortex-stats -p xtask --count 50` on the desk's side slot: 1,230 tests, 50 times each, and in repeat 1 fourteen file-heavy tests of `cortex-base` timed out together. The command said: "STALL: 13 tests timed out within 4 ms of each other: likely the machine, not the tests", with "During this run's 162 s, some task waited on I/O for 64.4 s, and every task at once for 60.8 s". The report shows them started within 33 ms and stopped 0.83 to 0.86 s past their 10 s limit. Every other repeat of every test passed. `cargo xtask stress -p cortex-agent --count 300`: 303 tests, every one every time. On Windows (`try` 43718006646): `cortex-tools`' shell and process tests, 92 of them, 20 times each, every one every time.

## The A/B runs

| Run | What | Runner start | Tier 3 above the start, base → head (median of 5) |
|---|---|---|---|
| `try` 43712116926 | `main` against itself | 8 ms | 67.0 → 66.2 s (rounds 65.4 to 82.8 on one side: the noise) |
| `try` 43715195861 | The branch at `51f9df9` (the test changes) against `main` | 10 to 11 ms | 93.2 → 80.4 s, −12.8 s (−14%); tier 2 41.2 → 38.3 s, tier 1 17.6 → 17.4 s |
| `try` 43715198613 | The system tests two at a time against three, both the branch | 7 to 8 ms | 57.6 → 46.6 s, −11.0 s (−19%); the system run's wall time 20.1 → 24.2 s |

The slowest tests of tier 3 on the head side, on a 6 ms runner (`try` 43712114548): `trust::sys_a_project_trusted_in_the_store_…` (`cortex-core`) 3.1 s, three turns each with two `pwsh` commands; the owner guard's `sys_what_the_machine_or_another_remote_could_hide_is_still_read` 1.8 s, `sys_a_push_is_searched_in_every_place_it_can_carry_a_term` 1.5 s and `sys_a_clean_push_…` 1.3 s, git started dozens of times; `shell::pwsh::sys_the_exit_status_is_the_commands` 1.8 s and `sys_the_command_reaches_the_shell_as_written` 1.5 s, five and four `pwsh` starts; `differential::sys_a_program_receives_the_words_the_analysis_gives_from_every_powershell` 1.4 s, 120 starts of the echoing child from three scripts; `rules::tests::sys_the_repository_follows_its_own_rules` 1.3 s. By crate: `cortex-tools` 16.1 s, `cortex-core` 8.9, `xtask` 8.1, `cortex-cli` 7.0.

## The breaks, still caught

`cargo xtask mutate tooling-6` on the desk's side slot: 106 of 106 caught by the tests they name (the list as it was at 12:22, every item's breaks; the disk stalled 43.6% of that run, and no verdict was lost to it). The breaks of the two reviews' fixes were run alone after them: every one caught by its named tests. For the PowerShell cases, which run only on Windows, two breaks were made by hand on scratch branches and run on a runner: PowerShell's quoting that does not double a typographic quote (`try` 43713566752) failed `sys_hostile_arguments_arrive_unchanged` on `pwsh` and `powershell`; a last line that always exits 0 (`try` 43713569343) failed `sys_the_exit_status_is_the_commands` and `sys_the_command_reaches_the_shell_as_written` on both. On Linux, the touched tests catch breaks of the POSIX quoting, the fixed `TERM` and the working directory (in `mutations/tooling-6.breaks`).

## The toolbox fixtures, recorded again

Step 18 added the `outline` tool, so the tools the binary sends no longer matched `crates/cortex-llm/fixtures/toolbox/`. `cargo xtask record --toolbox --provider P` at the desk, one provider a call, after a dry run of each (`record --provider P --dry-run toolbox`, nothing sent). Every provider accepted the new toolbox:

| Provider | Model | Tokens in, out | Cost |
|---|---|---|---|
| Anthropic | `claude-haiku-5-5` | 4,688, 4 | $0.000471 |
| DeepSeek | `deepseek-flash` | 2,772 (768 from the cache), 1 | $0.000837 |
| OpenRouter | `z-ai/glm-5.3-flash` | 3,397, 3 | $0.000511 (as the provider states) |
| Venice.ai | `deepseek-v4-flash-0731` | 3,550, 2 | $0.000462 (as the provider states) |

$0.002281 in all, against a budget of 5 US cents. The spend ledger before: 11 calls, $0.085024. The decoders' snapshots of the four streams changed by their token counts alone. `tools::sys_the_toolbox_the_binary_sends_is_the_one_its_fixtures_were_recorded_with` no longer warns, and the open item is closed.

## Answered by the coordinator on the way

| Question or addition | Answer |
|---|---|
| Two read-only shell-guard checks (`git status --short; echo $?`, and a `for` loop over `git log`). | Both refused by Claude Code's own worktree check, not a permission rule: commits stay as before (plain separate commands, script files, `replace --script`). |
| Optional additions from the step 16 review's retrospective. | Done: `scratch-test` narrows by a filterset after `--`; `mutate --file PATH`; `mutate` starts with one job when the disk is busy. Filed: a reviewer's kit (written in the seventh tooling step: [a reviewer's kit](../../testing-and-ci.md#a-reviewers-kit)). `scratch-test` already reads a tree without `.git` as `mutate` does; the testing page now says so. |
| `rx`'s own I/O-pressure line. | `rx` covers its runs; the commands' own check (item 2) is for wherever they run. |

## The independent reviews

An early review of item 1's test changes (Opus 5.5, not having seen the session's reasoning) found no assertion dropped or loosened and no shell or system that lost a case; several checks are stricter (exact slices of lines, the exit code of the batched cases, "injected" over the JSON half too). Two residuals: one write of LF lines for PowerShell's large outputs no longer gave the tool CRLF lines or many small reads (fixed: each line is written to the console's own writer, CRLF and one write each); and the JSON half's own success is now the last statement's (left: a quoting that errors while keeping all 27 texts exact cannot be built).

The full review of the whole diff, then a second of the fixes:

| Finding | Done |
|---|---|
| `mutate` took a break that hung several tests for a machine stall: tests started together by nextest end together at the same limit, and the break was never judged. | First, a group's starts had to be spread; the second review showed tests of two limits (60 s and 10 s) stopped in one moment still passed that. Now only tests stopped late, more than 250 ms past their own limit (10, 60, 300 or 1,800 s, held to `.config/nextest.toml` by a test), make a stall. Tests and breaks for both shapes. |
| The `perf -p CRATE` preset ran under tier 2's 10 s limit, which stopped step 18's largest tree. | A nextest `perf` profile, slow at 60 s and stopped at 30 minutes, with its JUnit report uploaded; the test kit's timeout watch knows its limit. Test and break. |
| `ci-log`'s matcher overflowed the stack on a line of 20,000 characters (about 2,500 on Windows' stack). | A repeat of a one-character atom is a loop. A test with a line of 100,000 characters, and a break. |
| Nothing held that the last good copy of `xtask` is never made on a CI runner, and every command at the desk started git twice to describe the tree. | `keeps_a_copy` under a test; the tree described only when a copy is made, which a test holds by panicking otherwise. Breaks. |
| Two `mutate --file` lists of one name shared their breaks' names and results. | Refused. Test and break. |
| The month generator's tests took 16.4 s of Windows' unit tier, and one failed its own 3 s bound there. | Seven small tests, 64 ms at the desk in CI's build; the whole months are ignored `perf_` tests with no time bound. The writer's wait, which small months never reached, is reached in the crate's tests with a small queue, and its break is caught again. |
| The commands for the generator's measurements hid their output; a comment claimed more than its test checks; the docs stated the first stall rule differently from the code. | Corrected. |

## Gaps

- **Item 1's target:** about 65 s at 10 ms by the two A/Bs together, 71.3 s on the pull request's 11 ms runner ("What it came to", above). Without the thread change, about 80 s at 10 ms is the best this step reaches without testing less: nearly every PowerShell start left in tier 3 is the shell tool's own (one process a call, which is what those tests test) or the product's shell in a `cortex-core` or `cortex-cli` turn. Running the process-tree tests on `cmd.exe` instead of the default `pwsh` would save about 5 s more; it changes which shell they test, and was not done or proposed.
- **Glue no test holds:** the calls of the stall check in `stress`'s and `ci`'s runs (the check itself, and `mutate`'s use of it, are tested); `./x`'s fallback to the last good copy (a shell script); the move-aside path of the copy on Windows.
- **`caught-by` names in CI:** checked only where a list of tests is kept; CI keeps none (`caught-by-names-checked-only-at-the-desk`, which the tenth tooling step settled ([tooling-10/parts/smaller-items.md](../tooling-10/parts/smaller-items.md))).
- **`mutate`'s lock under plain `cargo test`** can be held a moment by another test's child (the outstanding item `the-mutate-lock-and-a-forked-child`, since settled by a retry of the lock).
- **The month generator's byte-for-byte determinism across systems** is tested on each system alone; the two have not been compared.
- **The owner guard's test fixture** reads loose refs: a git whose default is reftable or SHA-256 fails it loudly.
