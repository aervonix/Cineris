# Step 25 fixes: the terminal's keyboard flags and its failed writes

The review of step 25 (findings 10 and 11, and the "Rules no test or break holds" bullet on the emergency restore's mouse-off) found the full screen leaving the enhanced keyboard protocol's flags pushed on the alternate screen, and a failed write that leaves the alternate screen never tried again. This part fixes both, and gives the test kit's virtual terminal a stack of keyboard flags per screen. The breaks are in [`mutations/step-25-fixes-terminal.breaks`](../../../../mutations/step-25-fixes-terminal.breaks).

## Finding 10: the keyboard flags were left on the alternate screen

**What was wrong.** The kitty keyboard protocol gives each screen a stack of flags of its own, and kitty and ghostty keep the alternate screen's from one use of it to the next. The terminal layer pushed its flags (`CSI > 3 u`) on whatever screen was shown when the reply came, and popped them (`CSI < u`) only after leaving the alternate screen. The full screen draws its first frame, on the alternate screen, before it reads the reply, so every full-screen run pushed on the alternate screen and popped the main screen's empty stack: until the terminal was reset, every later program that took the alternate screen (`less`, `man`, an editor, the external editor from `Ctrl+G`) got `Esc` as `CSI 27 u`, `Ctrl` keys as `CSI … u` and key releases it never asked for.

**The cause.** Pushes and pops were written without regard to the screen they landed on, in four places: `Terminal::reply` and `Terminal::resume` pushed on the screen shown; `Terminal::end`, the emergency restore's entries with the alternate screen (`RESTORE[5]`, `[7]`) and `LEFT_ON` (the external editor's clean-up of a killed editor) popped only after `?1049l` / `?1047l`. The test kit's `VTerm` kept one stack for both screens, so no test could see it.

**What changed, and why.**

- `cineris-term` `Terminal` (`crates/cineris-term/src/terminal.rs`): while the protocol is on, its flags are pushed on each screen the program shows and popped before it leaves that screen. `alternate(true)` writes the push after `?1049h`; `alternate(false)` writes the pop before `?1049l`, and, when the reply came while the alternate screen was shown, the push on the main screen after it. A new field, `main_keyboard`, says whether the main screen's stack holds the push. `end` pops the alternate screen's before leaving it and the main screen's after, only if it was pushed. `resume` (after a stop, whose restore popped both) pushes on the screen shown and leaves the main screen's for when it is back. Whatever the order of the reply, the alternate screen and a stop, each screen's stack ends empty.
- `RESTORE[5]` and `[7]` (`crates/cineris-term/src/sys/mod.rs`) write `CSI < u` before `?1049l` as well as after it. A pop of a stack that holds nothing does nothing, so the second pop is harmless where the main screen never had the flags.
- `LEFT_ON` (`crates/cineris-tui/src/external.rs`) writes `CSI < 99 u` before `?1047l` as well as after it, and its comment now says what it pops: up to 99 levels on each screen. It said "every level".
- The twin in step 23's inline viewer (a resumption while the viewer is open pushed on the alternate screen and the stop's restore had popped the main screen's, so back inline `Shift+Enter` sent the prompt) is the same code in `Terminal::resume`, fixed with it.
- `cineris-testkit` `VTerm` (`crates/cineris-testkit/src/vterm.rs`) keeps a stack per screen, the alternate screen's kept between uses as kitty keeps it, and a full reset empties both. `keyboard()` is still the stack of the screen shown, so every test that read it still says what it meant; `main_keyboard()` and `alternate_keyboard()` are new. The two pty tests that read the stack now read both screens' too: `cineris-term`'s `system.rs` `restored` (every way out of the demo) and `cineris-tui`'s `external.rs` (a killed editor's flags on the alternate screen).

**The tests**, each failing on `main`'s code (run with the code of `main` and the new tests) and passing with the fix:

- `cineris-term` `terminal::tests::the_keyboard_flags_are_left_pushed_on_neither_screen`: the reply on the main screen and on the alternate one, the alternate screen left by the program and by the end, and an emergency restore at each point, read through a `VTerm` with a stack per screen. On `main`: "reply on the alternate false, left true", left `[]`, right `[3]` (the alternate screen got no flags when entered after the reply).
- `cineris-term` `terminal::tests::a_resumption_on_the_alternate_screen_pushes_the_flags_on_each_screen_shown`: the inline viewer's resumption, with the real emergency restore written at the stop. On `main`: "reply on the alternate false: on the main screen", left `[]`, right `[3]` (back on the main screen it had no flags: `Shift+Enter` would send).
- `cineris-term` `sys::tests::the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen`: every entry of `RESTORE`, with the flags pushed on both screens or on the alternate one only. On `main`: "the alternate screen's flags [3]; keyboard true, win32 false, alternate true, pushed on the main screen true".
- `cineris-tui` `app::tests::screen::the_full_screen_and_the_viewer_leave_no_keyboard_flags_on_either_screen`: the application's loop (`app::run`) in the full screen, quitting and with the terminal failing under it, and the viewer opened and closed from the inline screen. On `main`: "the full screen quits: the main screen's flags [], the alternate screen's [3]", the review's finding exactly.
- `cineris-tui` `external::tests::what_a_killed_editor_left_on_is_turned_off_on_both_screens`: `LEFT_ON` after what a killed editor leaves. On `main`: "the main screen's flags [], the alternate screen's [1, 5]".
- `cineris-testkit` `vterm::tests::each_screen_has_a_stack_of_keyboard_flags_of_its_own` (new with the stack per screen).
- Strengthened, not new: `cineris-term::it system::sys_*`'s `restored` and `cineris-tui::it external::sys_*` read both screens' stacks.

## Finding 11: a failed write leaving the alternate screen was never tried again

**What was wrong.** `Terminal::alternate(false)` set `self.alternate = false` before its write, and cleared the emergency restore's record whatever the write did; `set_mouse(false)` did the same with the mouse. After one refused write (a `WouldBlock` on a terminal another process made non-blocking: `write_all` retries only `Interrupted`), neither the end nor an emergency restore turned either off, and the shell was left on the alternate screen with the mouse reported.

**What changed.** A mode turned off is marked off, and the emergency restore's record with it, only once the write has succeeded: `alternate(false)`, `set_mouse(false)`, the keyboard protocol's push in `reply` and `resume` (marked once written, so that a reply after a refused push tries it again), and `end`, which clears the emergency restore's record only when its write succeeded, so that a terminal entered again after it (the external editor's) still leaves the alternate screen in an emergency. A mode turned on is still marked on before its write, so that an emergency in between turns it off.

**The test:** `cineris-term` `terminal::tests::a_failed_write_turning_the_alternate_screen_or_the_mouse_off_is_tried_again` (the alternate screen's write refused, then an emergency and the end; the same, then the program's next call; the mouse's write refused, then the end; the end's own write refused). On `main`: "assertion failed: terminal.is_alternate() && terminal.mouse()" (both marked off by the write that failed).

## The emergency restore's mouse-off under the keyboard protocol and win32 input

`RESTORE[5..=7]`'s `?1006l` and `?1000l` had no test and no break. `sys::tests::the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen` reads every entry for the mouse off, and three breaks take it out of entries 5, 6 and 7.

## The same mechanism elsewhere

Every place in `cineris-term`, `cineris-tui` and `cineris-cli` that records a terminal mode as set or unset around its write or call was read:

| Where | What it records | Done |
|---|---|---|
| `Terminal::alternate(false)`, `set_mouse(false)` | the alternate screen, the mouse, the emergency restore's record | Fixed (finding 11). |
| `Terminal::reply`, `resume` (the keyboard protocol's push) | `keyboard`, `main_keyboard` | Fixed: marked once written. The emergency restore's record is still set before the write, which is the safe side: its pop of a stack with nothing of the program's does nothing. |
| `Terminal::end` | the emergency restore's record of the alternate screen, the keyboard protocol and the win32 input mode | Fixed: cleared only when the write succeeded. The end itself is not tried again: it is the last thing the terminal does, and the device has been left. |
| `Terminal::reply` (the win32 input mode), `alternate(true)`, `set_mouse(true)`, `sys::windows::enter` | modes turned on | Left: marked on before the write is the safe side, since an emergency between the two turns them off, and a second "on" of a mode is harmless. |
| `sys::unix::restore_modes`, `sys::windows::leave` (raw mode, the console's modes and code pages) | `ACTIVE`, marked off whether or not `tcsetattr` or `SetConsoleMode` succeeded | Left: leaving hands the terminal on (to the shell, or to the external editor), and a record left on would let a later stop or signal write the restore into a terminal that is no longer the program's, over the editor. |
| `sys::unix::emergency`, `sys::windows::emergency` | `ACTIVE`, swapped off before the restore's write | Left: the restore runs once, in a signal handler or a console event's thread, where nothing can wait for a terminal to take a refused write. |
| `Inline::finish` | `Sink::region(0)` (the rows the emergency restore moves up) before its write | Left: the write that a restore interrupted sends the restore again with the count, which must be the one after the frame; after a failed write nobody knows where the cursor is, so no count is right. The inline presenter's output must not change in this part. |
| `TerminalHandoff::release` and `reclaim` (`cineris-tui`), the full-screen presenter, `secret::read_key`, `cineris-cli` | nothing of their own: they go through `Terminal` | Nothing to do. |

## Changes of format, protocol and public API

No protocol, log, setting or format change. Public API: `cineris_testkit::vterm::VTerm::main_keyboard` and `alternate_keyboard`, added (the test kit's; additive). `VTerm::keyboard` now gives the stack of the screen shown, which is what it gave every test that read it, since none pushed flags on the alternate screen and read them from the main one. Nothing public of `cineris-term` or `cineris-tui` changed.

## Docs changed

- [09-tui.md](../../../architecture/09-tui.md), "Restoring the terminal" and the external editor's "The terminal": the flags per screen, and a mode marked off only once its write has succeeded.
- [testing-and-ci.md](../../../testing-and-ci.md), "The virtual terminal": the stack per screen.

## Breaks

`cargo xtask mutate step-25-fixes-terminal --jobs 2` on the reference machine: **16 breaks, every one caught** (exit 0).

| Break | Caught by |
|---|---|
| `the-alternate-screen-is-left-with-the-keyboard-flags` | `the_keyboard_flags_are_left_pushed_on_neither_screen`, `a_resumption_on_the_alternate_screen_pushes_the_flags_on_each_screen_shown` |
| `the-alternate-screen-gets-no-keyboard-flags` | `the_keyboard_flags_are_left_pushed_on_neither_screen` |
| `the-main-screen-never-gets-the-flags-of-a-reply-on-the-alternate` | both of the first |
| `a-reply-on-the-alternate-screen-is-taken-as-on-the-main` | `the_keyboard_flags_are_left_pushed_on_neither_screen` |
| `the-end-leaves-the-alternate-screen-with-the-keyboard-flags` | `the_keyboard_flags_are_left_pushed_on_neither_screen`, `a_failed_write_…` |
| `a-resumption-on-the-alternate-screen-counts-the-main-screen-pushed` | `a_resumption_on_the_alternate_screen_pushes_the_flags_on_each_screen_shown` |
| `the-restore-leaves-the-keyboard-flags-on-the-alternate-screen` (`RESTORE[5]`) | `sys::tests::the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen` and three `terminal::tests` |
| `the-restore-under-win32-leaves-the-keyboard-flags-on-the-alternate-screen` (`[7]`) | `sys::tests::the_restore_…` |
| `the-restore-under-the-keyboard-protocol-leaves-the-mouse-on` (`[5]`) | `sys::tests::the_restore_…` and two `terminal::tests` |
| `the-restore-under-win32-leaves-the-mouse-on` (`[6]`) | `sys::tests::the_restore_…` |
| `the-restore-under-both-leaves-the-mouse-on` (`[7]`) | `sys::tests::the_restore_…` |
| `taking-the-terminal-back-leaves-an-editors-flags-on-the-alternate-screen` (`LEFT_ON`) | `external::tests::what_a_killed_editor_left_on_is_turned_off_on_both_screens`, `cineris-tui::it external::sys_a_killed_editor_keeps_the_prompt_and_its_modes_are_undone` |
| `the-virtual-terminal-has-one-keyboard-stack` | `vterm::tests::each_screen_has_a_stack_of_keyboard_flags_of_its_own` |
| `leaving-the-alternate-screen-is-marked-before-its-write` | `a_failed_write_turning_the_alternate_screen_or_the_mouse_off_is_tried_again` |
| `the-mouse-is-marked-off-before-its-write` | the same |
| `an-end-that-failed-clears-the-restores-record` | the same |

`mutations/step-20.breaks`' `keyboard-protocol-left-on` now names `occurrence: 2` of the pop in `Terminal::end` (the main screen's), since `end` writes two.

`cargo xtask stress -p cineris-term --count 100` and `-p cineris-tui --count 100`: every test passed every time.

## Windows

`RESTORE` is the same array on both systems, and its test is not system-specific. What must run on a Windows runner: `cineris-term`'s unit tests (`sys::tests::the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen`, the three `terminal::tests` above), `cineris-term::it system::sys_*` (whose `restored` now reads both screens' stacks through ConPTY), and `cineris-tui`'s unit tests named above. `external::sys_a_killed_editor_keeps_the_prompt_and_its_modes_are_undone` is Unix only. Nothing under `sys/windows*` changed; the workspace was linted for Windows with `cargo xwin clippy`.

## Left open

- **A refused write is not waited out.** `sys::unix::write_all` retries only `Interrupted`: a `WouldBlock` fails the write at once, and the fix makes the next call, the end or an emergency try it again, not wait for the terminal to take it. Waiting for the terminal to be writable (`poll` for `POLLOUT`) before trying again would close it. Suggested for step 32 (hardening), with the review's other failed-write findings.
- The places left in "The same mechanism elsewhere", with their reasons.
- Windows: the tests named under "Windows" have run on Linux only.
