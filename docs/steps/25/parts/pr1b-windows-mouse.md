# Step 25, a follow-up to its first pull request: nothing asks for the mouse through a pseudo-console

The first pull request (94) stopped writing modes 1000 and 1006 on Windows, having measured that a pseudo-console gives the terminal's mouse reports to the program as typed keys, and kept the console's mouse input, with QuickEdit off while it was on, for the old console host. A reviewer who read that change after the merge found that its probe could not see what the console's mouse input itself asks for; the coordinator had this follow-up made at once, on its own, since `main` carried the risk.

## What was measured

The mouse system test (`cortex-term::it system::sys_the_mouse_reaches_the_program_on_the_alternate_screen`) now reads every parameter of every mode set (`modes_set`), where it searched for `?1000h`, `?1002h`, `?1003h` and `?1006h` as text. Run on a hosted Windows runner against `main`'s code (try run 44370064135), it failed: the modes set through the pseudo-console were `[9001, 1004, 25, 2004, 1004, 1003, 1006, 1049, 25]`. ConPTY asked the terminal for any motion in the SGR form (`ESC [ ? 1003 ; 1006 h`) when the program turned the console's mouse input on with QuickEdit off, so under Windows Terminal every move of the pointer over the viewer or the full screen would have been reported, and, by the first pull request's own measurement, typed into the prompt.

## What was built

| Where | What |
|---|---|
| `cortex-term`: `sys/windows.rs`, `sys/unix.rs`, `terminal.rs` | The console's mouse input and the change to QuickEdit are gone: nothing asks for the mouse on Windows. A console record of the mouse is still read as the report it stands for. |
| `cortex-term`: `terminal.rs` | The terminal asks for the mouse by its sequences only where `Capabilities::vt_mouse` holds and the win32 input mode is off (`asks_vt_mouse`), whatever the system: a Linux build under WSL or over ssh in Windows Terminal is likely behind a pseudo-console in the same mode (inferred, not measured). The off sequence is written wherever the terminal might have been asked, so a failed write can never leave the mouse on. When the mode's reply turns it on with the mouse asked for, the mouse goes off first, in the same write. |
| `cortex-testkit`: `pty.rs`, `vterm.rs` | `Session::without_win32_input`, a terminal that does not know the win32 input mode, as xterm; `VTerm::shows_under_the_alternate`. |
| `cortex-term`, `cortex-cli` system tests | The probe reads every parameter (`modes_set`, with a test of its own); on Unix both mouse tests run in a terminal without the win32 input mode and check that the mouse was asked for. The full-screen test's first wait also finds the shell's line under the alternate screen, or takes the switch to it: through ConPTY the line and the switch can come in one read, and on the try run above the wait timed out with the full screen drawn. Taking the switch, the time to the first frame is near nothing and checks little; the start-up budget is `cargo xtask startup`'s, and the inline tests keep the check. `modes_set` and `MOUSE_MODES` moved to the test kit (`cortex_testkit::vterm`), and the full-screen test checks on Windows that no mouse mode is asked for. A new system test, `sys_a_terminal_in_the_win32_input_mode_is_not_left_asked_for_the_mouse`, waits in a terminal that knows the mode for the mode on and the mouse off. |
| Docs | 09-tui ("The mouse", the `vt_mouse` row), `docs/conpty.md` (mouse reports come as typed keys; read every parameter), the open item `the-mouse-under-windows-terminal` (what was measured and what was inferred, the likely mechanism, alternate scroll's arrows, and the recommendation for step 33). |

The old console host loses the mouse with this, which no test had run; the coordinator settled that on the reviewer's recommendation.

## The tests and the breaks

| Rule | Test | Break |
|---|---|---|
| On Windows nothing asks for the mouse, on any path: the alternate screen, `set_mouse` either way, the end | `terminal::tests::on_windows_the_terminal_is_asked_for_no_mouse_reports`; on a runner, the mouse system test | `windows-asks-the-terminal-for-the-mouse` |
| No mouse by its sequences while the win32 input mode is on, a resumption included, and off first when the mode comes | `terminal::tests::the_win32_input_mode_takes_the_mouse_off_and_keeps_it_off`; `cortex-term::it system::sys_a_terminal_in_the_win32_input_mode_is_not_left_asked_for_the_mouse` | `the-win32-input-mode-keeps-the-vt-mouse`, `the-win32-reply-leaves-the-mouse-on` |
| The off sequence wherever the mouse might have been asked for | the same unit test | `the-mouse-off-only-where-asked` |
| The probe reads every parameter of a mode set | `cortex-term::it system::every_parameter_of_a_mode_set_is_read` | `the-mouse-probe-reads-one-parameter` |

`cargo xtask mutate` on the five breaks this follow-up adds and the three of the first pull request whose code it changed: every one caught. The probe's break first came back not caught, because its file is the test kit's and the test that catches it is `cortex-term`'s; it names `packages: cortex-term` now, and is caught.

## The independent review of the fix

The reviewer of the first finding read the fix before it was pushed and found it right, with no path left that asks for the mouse. What it raised, and what was done: three places stated an inference as measured (corrected, in `conpty.md`, 09-tui and `Capabilities::vt_mouse`'s comment); a terminal that knows the win32 input mode itself, with no pseudo-console between, loses the mouse too (named in the open item, with a hand check); the Unix half of the mouse system test could not tell the two terminals apart (it now checks the modes' state, and a second test the converse); a resumption under the mode had no test (added to the unit test); the probe's commit, alone, failed on Windows (it now comes after the fix); the first wait's fallback checks little (said in the test and above); the off sequence was gated on the mode too (now written wherever the terminal might have been asked); the found input mode put back at exit may ask for the mouse for the shell where QuickEdit is off in the console's defaults (older than this, in the open item and the checklist); the record lacked a Windows run, a stress and the breaks' verdicts (below); and `MOUSE_MODES` lacked 9, 1001 and 1016 (added).

## Runs

- Before the fix, `main`'s code with the new probe: try run 44370064135 (Windows), the mouse test failed with the modes listed above, and the full-screen test timed out at its first wait (the race fixed here).
- The fix, every test on Windows with both trees on one runner (try run 44374257418, at c94dff5, 8 ms a process start): nothing failed; the tiers against `main`'s +0.6 s, +0.3 s and +0.0 s.
- A stress of `cortex-term` and `cortex-cli` on Windows (try run 44374288869): 237 tests, 20 times each, every one passed every time.
- At the desk: every test of the workspace passed, and clippy for Linux and, through `cargo xwin`, for Windows found nothing.

## Not done here

What the wheel does in the full screen under Windows Terminal, whose alternate scroll is likely to send it as `Up` and `Down` to the input: turning mode 1007 off needs its state put back, which is not small, so it is in the open item for step 33, with the checklist's line in the third pull request.
