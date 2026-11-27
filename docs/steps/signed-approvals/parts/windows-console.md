# Signed approvals, part: commands without the harness's console

Approvals are to be signed with a key made from a password the person types into the harness's console ([../answered.md](../answered.md), "The password prompt"). An independent review found that on Windows every command a session started was attached to that console: `cineris-sandbox`'s tree started a command with `CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED` only. So a command, or a process it left running, could open `CONIN$` and read the password as it was typed (`ReadConsoleInputW`, `PeekConsoleInputW`), type keys into a question the harness asked (`WriteConsoleInputW`), and draw a prompt of its own in the person's console. **The owner's decision:** in this step, start the commands without the harness's console.

## The choice: a console of the command's own, with no window

**Every process the harness starts that may run the model's code or a project's now starts with `CREATE_NO_WINDOW`.** A console program started so gets a console of its own, which has no window; what it starts shares that console, not the harness's; its standard streams are still the pipes the runner gave it. The polite interrupt, a Ctrl-Break event, is sent from inside that console by a helper (below). The fallback for a program that needs a person at a console is none: no program tried needs one, and a program that reads its console's input now waits for keys nobody types, until its time limit, where before it read the person's keys from under the harness.

The three ways compared, measured on the reference machine's Windows (build 26300) with a throwaway program run through WSL's interop (which started it elevated, on a console with no window), and read against Microsoft's documentation and the console's public source:

| | `CREATE_NO_WINDOW` (chosen) | `DETACHED_PROCESS` | A pseudo-console (ConPTY) for each command |
|---|---|---|---|
| The console a console program gets | Its own, with no window (`GetConsoleWindow` is null). `GetConsoleProcessList` from inside lists the command alone: today's flags listed the command and the harness. `CONIN$` opens, and is its own console's | None. `GetConsoleProcessList` and opening `CONIN$` fail with `ERROR_INVALID_HANDLE` | Its own, the pseudo-console's |
| A program that calls `GetConsoleMode`, opens `CONOUT$` or `CONIN$` | Works, on its own console (mode `0x1f7` on `CONIN$`, as on the harness's) | Fails: no console | Works |
| What a console program it starts gets | The same console: one console host for the whole tree | **A new console with a window**, for each one: every `git` a `cmd.exe` runs would flash a window on the person's desktop, and each would be a console of its own | The same pseudo-console |
| Standard streams | The runner's pipes, as before (`GetFileType`: pipe for output and error, the null device for input) | The same | The pipes, given beside the pseudo-console's own, which must still be read |
| Ctrl-Break | Not from the harness's console (below): from inside the command's, by a helper, 4.7 ms median | Cannot be sent: there is no console | Written to the pseudo-console's input |
| Cost of a start | A console host (`conhost.exe`) for each command: `cmd /d /c exit 0` from start to end in 12.8 ms median against 6.5 ms today; a small Rust program 9.1 ms against 3.4 ms. About 6 ms more a command | None more (6.3 ms) | A console host for each command too, a thread to read the pseudo-console's output, and the harness's own `CreateProcessW` in place of the standard library's (its command line, environment and handles built here) |

`DETACHED_PROCESS` is ruled out by its grandchildren, and a pseudo-console costs what `CREATE_NO_WINDOW` costs and much more code for nothing this step needs. A pseudo-console stays the way to give a future feature a console program's screen to show, or a person's keys to type into it.

### What the console host changes

**The job holds the console host.** The command starts it as it starts, so it is a member of the command's Job Object (`C:\Windows\System32\conhost.exe` beside the command, where today's flags gave a job of one member). It ends by itself once the last process on its console has, about a millisecond after the command (median 1.05 ms, at most 1.12 ms in 20 rounds) — **but it was still running at the moment the command was seen to exit in 19 of the 20 rounds, and in all 300 rounds of `cmd /d /c echo hi`.** The runner asks the tree at that moment what the command left running, and would have waited half a second (the linger) after every command for the console host. So the tree leaves the system's console host out of that count (`Inner::others_running`, by its program's path in the system's folder, which only an administrator can write); a process the command left on its console still keeps the host, and is counted itself. The tree's kill takes the host with the rest.

**The host is known by both of its names.** At that moment the host is ending too, and Windows then refused the usual form of its program's name (`QueryFullProcessImageNameW` with `PROCESS_NAME_WIN32`: access denied) in 35 of the 300 rounds, while its own form (`PROCESS_NAME_NATIVE`, `\Device\HarddiskVolume5\Windows\System32\conhost.exe`) was given every time. The first version asked for the usual form alone, and the existing `sys_a_command_that_left_nothing_is_never_counted_itself` failed on the reference machine's Windows at its 13th round. The tree now asks for Windows' own form first, compared with the system folder's volume by its device's name (`QueryDosDeviceW`), and the usual form after it; the test then passed ten runs of 40 rounds.

**The job's process limit behaves as before:** a job limited to one process ran `cmd /d /c "echo outer-ran & cmd /d /c echo inner-ran"`'s outer `cmd.exe` and refused the inner one, and a job of two ran both, with and without the flag. The console host does not count against `JOB_OBJECT_LIMIT_ACTIVE_PROCESS`.

### The interrupt, sent from inside the command's console

Windows sends a console event only to the processes of a group that are on the sender's console. **Sent from the harness's console to a command on its own, `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid)` returns success and delivers nothing** (the command was still running 1.5 s later), and the command's id is added to the harness's console's list of processes, where it stayed after the command was killed: the console takes an id that is no group there for a process whose parent is on it, as microsoft/terminal issue 335 describes, and that stale entry can later hide other processes from console events. So the harness never sends it from its own console.

Instead `Tree::interrupt` starts **the harness's own program under its hidden subcommand**, the guard's (`cineris __tree-guard cin-ctrl-break <pid>`), with no console (`DETACHED_PROCESS`) and in a group of its own. It attaches to the command's console (`AttachConsole(pid)`), sends Ctrl-Break to the command's group there, lets go, and ends with 0, or with the system's error code, which the tree turns back into the error. The harness waits for it, at most 5 s.

- **Measured:** 4.7 ms median from the helper's start to its end (6.0 ms at the 90th percentile, 12 rounds), every command ended by the event itself (`STATUS_CONTROL_C_EXIT`, not the kill's code). Through the shells, with the same helper, the whole job was gone after: `cmd.exe` 11 ms, PowerShell 7 41 ms, Windows PowerShell 761 ms; nothing was left.
- **A tree started without a guard has no interrupt** on Windows: `interrupt` fails with `Unsupported`, which the runner passes over to the kill at once, as it passes over the terminate stage. Every harness that runs sessions gives its runner a guard (`headless`, `interactive`, and so `serve`); a test that wants the interrupt gives one too.
- **On Linux nothing changes:** the guard still runs as before, and the hidden subcommand ignores the new word there.
- A command that runs the helper itself gains nothing: the three calls it makes are open to any program of the user's, and the permission engine refuses `cineris __tree-guard` to a command already.

## Every place the harness starts a process

From a search of the workspace for `Command::new`, `CreateProcessW`, `creation_flags` and `spawn`, and section 12 of [security.md](../../../security.md), which the repo rules keep in step with the code.

| Where | What it runs | Outcome |
|---|---|---|
| `cineris-tools`'s `process::Runner` (`run`, `run_until`, `run_apart`), through `cineris_sandbox::tree::Tree::spawn` | The shell tool's commands; the project's checks (`verify`, through the session's shell); hook commands and extensions' programs (`cineris.exec`, the core's `ext::exec` through `run_apart`) | **A console of its own**, set in `tree::windows` |
| `cineris_base::git::start` | The harness's own git: the base state, a file's earlier content, the checkpoints' `ls-tree`, what a command changed (`cineris-session`, `cineris-core`'s checkpoints, `cineris-tools`'s `shell::changes`). It runs the filters and other programs a repository's configuration names, so it runs a project's code | **A console of its own** (`cineris_base::program::without_console`) |
| `cineris-tools`'s `process::brief::run` | The session's shell asked its version (`Shell::probe_version`): the shell that will run the model's commands | **A console of its own**, as its commands have: the same flag costs one console host a session, and a shell a command replaced is not given the harness's console here either |
| `cineris-sandbox`'s `tree::windows` (`Inner::interrupt`), new | The helper above | **No console at all**, by design: it attaches to the command's |
| `cineris-tui`'s `external` | The person's external editor (`$VISUAL`, `$EDITOR`) | **Keeps the harness's console**: the person's own program, started at the person's request, which the person types into |
| `cineris-sandbox`'s `tree::guard` (the guard process) and `launch` (the launcher becoming the command or git) | Unix only | Nothing to change on Windows |
| `cineris-term`'s `sys::windows` | Pseudo-consoles, behind the `pty` feature only the test kit turns on | Test support, unchanged |
| Starts in tests only: `cineris-cli`'s `workspace` and `cineris-sandbox`'s `unfollowed` (`mklink` through `cmd.exe`), `cineris-sandbox`'s `session` (Unix), `cineris-base`'s `hook`, `nofollow` and `git` tests, the test kit | Test set-up | Unchanged |
| MCP servers | Not built: `cineris-mcp` is empty | Step 30 starts them through the runner or with `without_console` |

The core's `repo_map` spawns a thread, not a process. No process the harness starts for itself keeps the console but the editor.

## The tests

New, Windows only, in the tier-3 tests:

| Test | What it holds |
|---|---|
| `cineris-sandbox::it console::sys_a_command_has_a_console_of_its_own_and_not_the_harnesss` | A command a tree starts (this test binary again) finds itself on a console, not the test's: its `GetConsoleProcessList` has its own id and not the test's, and it opens `CONIN$`, which is its own console's. Its report came through the output pipe |
| `cineris-sandbox::it console::sys_without_a_guard_the_interrupt_is_refused_and_the_harnesss_console_is_left_alone` | Without a guard, `interrupt` fails with `Unsupported`, the command is still running, and the test's console has not gained the command's id (the stale entry an event sent from the harness's console leaves) |
| `cineris-sandbox::it console::sys_the_interrupt_is_sent_from_inside_the_commands_own_console` | With the guard, the interrupt ends a command that listens, with `STATUS_CONTROL_C_EXIT` and not the kill's code |
| `cineris-tools::it console::sys_a_shell_command_does_not_share_the_harnesss_console` | Through the session's command path (the shell tool, its runner, its tree), with `cmd.exe`, which starts a program on its own console: the program's console is not the test's |
| `cineris-tools::it console::sys_ordinary_commands_work_without_the_harnesss_console` | One Windows PowerShell command runs `git --version`, `cargo --version`, `cmd /d /c echo`, `Get-Location` and a `.cmd` shim of npm's kind (which runs `node`, or `cmd.exe` where there is no node), and each says what it should |
| `cineris-tools::it console::sys_the_harnesss_git_does_not_share_its_console` | The harness's git runs an alias of the test's (through git's own `sh`), which reports its console: not the test's. The same alias from a git started as before shows the test's id, so the path through `sh` passes a console on and the check is not empty |
| `cineris-tools process::brief::tests::sys_a_brief_program_does_not_share_the_harnesss_console` | The brief program (the shell asked its version) has a console of its own |

Changed:

- `cineris-sandbox::it tree::sys_the_interrupt_stops_what_listens_and_leaves_what_does_not` gives the tree the guard on Windows.
- `cineris-sandbox::it tree::sys_a_command_that_left_nothing_is_never_counted_itself` is unchanged, and is now also the test of the console host's being left out: 40 rounds of `cmd /d /c echo hi`, each asked at once.
- `cineris-cli`'s system tests that start the binary as a tree and send it Ctrl-Break (`signals.rs`'s `Running`, `serve.rs`'s `sys_a_signal_ends_the_server_with_its_code`) give the tree the binary itself under `__tree-guard` on Windows (`tree_guard()`), which sends the event from inside the binary's own console, as the harness does for its commands; on Linux they give none, as before. `sys_ctrl_break_stops_a_running_extension_command` then also runs the product's own interrupt of its command end to end.
- `cineris-tools::it process::sys_the_polite_interrupt_is_enough_for_a_command_that_listens` is unchanged: its runner has the guard, and it is the test that an interrupt still stops a long-running command through the session's path.

New measurements, ignored `perf_` tests in `cineris-sandbox`: `perf_a_tree_starts_and_ends_one_command` (`tree.start_to_end`) and `perf_an_interrupt_stops_a_command_that_listens` (`tree.interrupt_to_gone`).

The breaks are in [mutations/signed-approvals-windows-console.breaks](../../../../mutations/signed-approvals-windows-console.breaks): the flag dropped from the tree, from the harness's git and from the brief program, and the console host counted as left running. All four are `os: windows`.

## What ran, and what did not

On the reference machine, on Linux:

- `cargo xtask clippy -p cineris-sandbox`, `-p cineris-tools` and `-p cineris-base`: clean.
- `cargo xtask test -p cineris-sandbox` (138 passed, 3 skipped), `-p cineris-tools` (460 passed, 4 skipped), `-p cineris-base` (338 passed, 2 skipped).
- `cargo xtask mutate --check`: every break applies; the four new breaks' tests, being of Windows' code, were looked for in the words of the source.
- `cargo xtask check`: formatting, the break lists, the generated files, the workspace's clippy and the licences pass. The repo rules and the links fail on this part's branch alone, for what the step's own records supply: `../answered.md`, which is on the step's branch, and the step's link to this part's group, `[parts/](parts/)`, which the step's `answered.md` gives once.
- **Linux is unchanged:** the tree's Unix half is untouched, `without_console` does nothing off Windows, the hidden subcommand's new word is read on Windows only, and every test that gives a tree a guard for this part does so on Windows only.

For Windows, on the reference machine:

- `cargo xwin clippy --target x86_64-pc-windows-msvc --workspace --all-targets --locked -- -D warnings`: clean.
- **The Windows test binaries of `cineris-sandbox` and `cineris-tools`, built with `cargo xwin` and run on the machine's own Windows through WSL's interop** (not CI): all of `cineris-sandbox`'s `console::` and `tree::` tests, 17, passed, the three new ones among them; `sys_a_command_that_left_nothing_is_never_counted_itself` failed once before the host was known by both names, and passed ten runs of 40 rounds after; `process::brief::tests::sys_a_brief_program_does_not_share_the_harnesss_console` and `console::sys_the_harnesss_git_does_not_share_its_console` passed (the latter's control showed git's `sh` passing a console on).
- The ignored `perf_` tests in that debug build: `tree.start_to_end` 31.5 ms median for `cmd /d /c exit 0` as a tree (job, suspended start, thread list, wait, end), and `tree.interrupt_to_gone` 5.7 ms median with the test binary as the helper.
- **Not run on Windows:** `cineris-tools`'s `console::sys_a_shell_command_does_not_share_the_harnesss_console` and `console::sys_ordinary_commands_work_without_the_harnesss_console`, `process::sys_the_polite_interrupt_is_enough_for_a_command_that_listens`, and `cineris-cli`'s system tests: their test workspace is under `CARGO_TARGET_TMPDIR`, which a binary built on Linux holds as a Linux path. They are for the runners (below).

## The measurements to run on a runner

After the branch is pushed:

```text
gh workflow run try --ref <branch> -f os=windows -f command="test -p cineris-sandbox -p cineris-tools -p cineris-cli"
gh workflow run try --ref <branch> -f os=windows -f command="test -p cineris-tools" -f against=main -f count=5
gh workflow run try --ref <branch> -f os=windows -f command="test -p cineris-sandbox" -f against=main -f count=5
gh workflow run try --ref <branch> -f os=windows -f command=test -f against=main
gh workflow run try --ref <branch> -f os=windows -f command="perf -p cineris-sandbox" -f against=main
gh workflow run try --ref <branch> -f os=windows -f command="mutate signed-approvals-windows-console --jobs 2"
```

Then `cargo xtask try-result <run id>` for each, and `cargo xtask ci-times --ab <run id>` for the A/Bs. The first runs the new tests and the changed ones; the next three put the cost of a console host for each command against `main` on one runner (the tier-3 shell tests start hundreds of commands, so about 6 ms each on the reference machine, and perhaps twice that on a hosted runner, is what to look for); `perf` gives `tree.start_to_end` and `tree.interrupt_to_gone` on both trees; `mutate` runs the four breaks where they can bite.

## What is left: a program of the user's can attach itself

Commands no longer *have* the harness's console; one that sets out to reach it still can. `AttachConsole(pid)` attaches the calling process to the console of the process `pid` names, and a process on a console can read and write its input as the harness can: open `CONIN$`, then `PeekConsoleInputW`, `ReadConsoleInputW`, `WriteConsoleInputW`. What it asks of the caller, measured on the reference machine's Windows:

- **No console of its own at that moment.** A command lets go of its own with `FreeConsole` first.
- **No right on the process it names.** The probe's target set its own access list, and the attaching process turned all its privileges off first, so that the list held:
  - with the list a sibling part gives the harness (the user keeps `PROCESS_QUERY_LIMITED_INFORMATION`, `SYNCHRONIZE` and `TERMINATE`), `OpenProcess` was refused `PROCESS_QUERY_INFORMATION`, `PROCESS_DUP_HANDLE` and `PROCESS_VM_READ`, and `AttachConsole` succeeded;
  - with `SYNCHRONIZE` alone, the same;
  - with an empty list, which refused `OpenProcess` even `SYNCHRONIZE`, `AttachConsole` still succeeded, and the attached process typed a key into the console's input (`WriteConsoleInputW`) and read it back (`PeekConsoleInputW`).

  The console's driver and host broker the attach; the process's own handle is not asked for. **So the harness's access list does not narrow it.**
- **Any process on that console will do.** Naming another process on the same console, whose list was left as it was, attached as well. A person who starts the harness from a shell has that shell on the harness's console.
- **The same session; the same integrity level is to confirm.** The probe's processes ran elevated (high integrity, every privilege of an administrator's token turned on until the attacher turned them off), as WSL's interop starts Windows programs on that machine, so the attacher and its targets were at one level. Whether a program at the normal level can attach to the console of an elevated harness is on the checklist below.

What this leaves: a program of the user's, a command among them, that lets go of its console and attaches to the harness's (`AttachConsole` with the harness's id, or the id of the shell it was started from) reads the person's keys as they are typed, the password among them, and can type into the harness's questions. It is the design's "a keylogger, or a fake prompt" ([../answered.md](../answered.md), "What signing cannot close"): any program of the user's can read the keyboard on Windows anyway, through a low-level keyboard hook. This part takes the console from what the harness starts, so that reaching it is a deliberate act and not something every command has by being started; it does not keep out a program that sets out to come in. **Only a prompt the system draws outside every console closes it on Windows:** Windows Hello (the design's Owner's call 7), or the system's credential prompt on the secure desktop.

On Linux this part changes nothing: a command already starts in a session of its own with no controlling terminal, and a program of the user's can still open the person's terminal device by its path and read from it, which is the same residual.

## To confirm on the reference machine's interactive session

The measurements above were taken by a program started through WSL's interop, on a console with no window. What only a person at the machine can see:

- [ ] In Windows Terminal, with PowerShell 7 as the session's shell, a command that reads `CONIN$` (`[Console]::ReadKey()` in a loop, or a small program calling `ReadConsoleInputW`) sees none of what is typed into the harness while it runs, and is ended by its time limit; the same with `cmd.exe` and with Git Bash as the shell, and in the classic console host in place of Windows Terminal.
- [ ] No console window appears, not even for a moment, for a session's command, a hook command, a check, an extension's program, the harness's git (the base state, a rewind's checkpoints) or the shell's version: from Windows Terminal, from the classic console host, and with Windows Terminal set as the default terminal application (a console with no window must not be handed to it as a new tab).
- [ ] Ctrl-C at the harness while a long command runs (`ping -t localhost`, a `cargo build`) stops the command with the event: its result says it was stopped with the interrupt, not the kill, and nothing of it is left in Task Manager.
- [ ] A command that lets go of its console and attaches to the harness's (`FreeConsole`, then `AttachConsole` with the harness's process id, then `ReadConsoleInputW`) reads the keys typed into the harness: the residual above, confirmed by a person's typing and not by a program's. Then the same with the sibling part's access list on the harness's process, and through the process id of the shell the harness was started from.
- [ ] With the harness started at the normal level (not elevated), a program at the normal level attaches to its console, as the probe's elevated programs did; and with the harness elevated, whether a program at the normal level can attach to its console.
- [ ] Why WSL's interop started the probe's Windows programs elevated on this machine (a distro started from an elevated terminal, or a setting): an elevated Windows program, with `SeDebugPrivilege` turned on, is what any program of the user's in WSL could then start without a prompt, and it can read and write the memory of every process of the session, the harness's included.
- [ ] `cargo xtask perf -p cineris-sandbox` in a release build on an otherwise idle machine, against `main` with `cargo xtask perf-ab`: `tree.start_to_end` and `tree.interrupt_to_gone`, the cost of a command's console host with the harness's own binary as the helper, where the probe's figures were of a small program.
