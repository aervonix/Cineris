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

**The slowest twenty of tier 3** in the profiled run, with what each is and what was done:

(Filled in below, with the A/B.)

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

## The independent reviews

(Being written.)

## Gaps

(Being written.)
