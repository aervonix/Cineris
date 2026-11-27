# Step 25 fixes: the terminal's keyboard flags and its failed writes

The review of step 25 found two defects in the terminal layer: findings 10 and 11, and the "Rules no test or break holds" bullet on the emergency restore's mouse-off. The full screen left the enhanced keyboard protocol's flags pushed on the alternate screen, and a failed write that leaves the alternate screen was never tried again. This part fixes both, and gives the test kit's virtual terminal a stack of keyboard flags per screen. An independent review of the fixes then found five more defects, fixed in a second round (below). The breaks are in [`mutations/step-25-fixes-terminal.breaks`](../../../../mutations/step-25-fixes-terminal.breaks).

## Finding 10: the keyboard flags were left on the alternate screen

**What was wrong.** The kitty keyboard protocol gives each screen a stack of flags of its own, and kitty and ghostty keep the alternate screen's from one use of it to the next. The terminal layer pushed its flags (`CSI > 3 u`) on whatever screen was shown when the reply came, and popped them (`CSI < u`) only after leaving the alternate screen. The full screen draws its first frame, on the alternate screen, before it reads the reply. So every full-screen run pushed on the alternate screen and popped the main screen's stack. Until the terminal was reset, every later program that took the alternate screen (`less`, `man`, an editor, the external editor from `Ctrl+G`) got `Esc` as `CSI 27 u`, `Ctrl` keys as `CSI … u`, and key releases it never asked for.

**The cause.** Pushes and pops were written without regard to the screen they landed on, in four places:
- `Terminal::reply` and `Terminal::resume` pushed on the screen shown;
- `Terminal::end`, the emergency restore's entries with the alternate screen, and `LEFT_ON` (the external editor's clean-up of a killed editor) popped only after `?1049l` / `?1047l`.

The test kit's `VTerm` kept one stack for both screens, so no test could see it.

**What changed, and why.**

- **`Terminal`** (`crates/cineris-term/src/terminal.rs`). While the protocol is on, its flags are pushed on each screen the program shows, and popped before it leaves that screen.
  - `alternate(true)` writes the push after `?1049h`.
  - `alternate(false)` writes the pop before `?1049l`. When the reply came while the alternate screen was shown, it also pushes on the main screen after `?1049l`.
  - A new field, `main_keyboard`, says whether the main screen's stack holds the push.
  - `end` pops the alternate screen's flags before leaving it. It pops the main screen's after, and only if they were pushed there.
  - `resume` runs after a stop, whose restore popped the program's flags. It pushes on the screen shown, and leaves the main screen's push for when that screen is back.
- **The emergency restore** (`crates/cineris-term/src/sys/mod.rs`) pops the alternate screen's flags before it leaves it. It pops the main screen's only where the program pushed there, which a new record bit (`MAIN_KEYBOARD`) says; this is the second round's first fix.
- **`LEFT_ON`** (`crates/cineris-tui/src/external.rs`) pops up to 99 levels on the alternate screen, which it takes first (`?1047h`) and leaves after. It leaves the main screen's stack alone (second round). Its comment said it popped "every level".
- **The twin in step 23's inline viewer** is fixed with it, since it is the same code in `Terminal::resume`. A resumption while the viewer was open pushed on the alternate screen, after the stop's restore had popped the main screen's flags, so back inline `Shift+Enter` sent the prompt.
- **`VTerm`** (`crates/cineris-testkit/src/vterm.rs`) keeps a stack per screen. The alternate screen's stack is kept between uses, as kitty keeps it, and a full reset empties both.
  - `keyboard()` is still the stack of the screen shown, so every test that read it still says what it meant.
  - `main_keyboard()` and `alternate_keyboard()` are new.
  - The two pty tests that read the stack now read both screens' too: `cineris-term`'s `system.rs` `restored` (every way out of the demo) and `cineris-tui`'s `external.rs` (a killed editor's flags on the alternate screen).

**The tests.** Each one fails on `main`'s code (run with the code of `main` and the new tests) and passes with the fix. Since the second round, every test that reads the stacks starts its `VTerm` with a shell's own entry on the main screen (`CSI > 1 u`, pushed before the program), and checks that it is still there after every way out.

- **`cineris-term` `terminal::tests::the_keyboard_flags_are_left_pushed_on_neither_screen`.** It covers the reply on the main screen and on the alternate one, the alternate screen left by the program and by the end, and an emergency restore at each point.
  - On `main`: "reply on the alternate false, left true", left `[]`, right `[3]`. The alternate screen got no flags when entered after the reply.
- **`cineris-term` `terminal::tests::a_resumption_on_the_alternate_screen_pushes_the_flags_on_each_screen_shown`.** It plays the inline viewer's resumption, with the real emergency restore written at the stop.
  - On `main`: "reply on the alternate false: on the main screen", left `[]`, right `[3]`. Back on the main screen there were no flags, so `Shift+Enter` would send.
- **`cineris-term` `sys::tests::the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen`.** It covers every state of the record: the flags pushed on both screens, on the alternate one only, or nowhere, and the win32 input mode on or off.
  - On `main`: "the alternate screen's flags [3]; keyboard true, win32 false, alternate true, pushed on the main screen true".
- **`cineris-tui` `app::tests::screen::the_full_screen_and_the_viewer_leave_no_keyboard_flags_on_either_screen`.** It runs the application's loop (`app::run`) in the full screen, quitting and with the terminal failing under it, and the viewer opened and closed from the inline screen.
  - On `main`: "the full screen quits: the main screen's flags [], the alternate screen's [3]", the review's finding exactly.
- **`cineris-tui` `external::tests::what_a_killed_editor_left_on_is_turned_off_on_both_screens`.** It checks `LEFT_ON` after what a killed editor leaves, and after an editor that ended as it should.
  - On `main`: "the main screen's flags [], the alternate screen's [1, 5]".
- **`cineris-testkit` `vterm::tests::each_screen_has_a_stack_of_keyboard_flags_of_its_own`**, new with the stack per screen.
- **Strengthened, not new:** `cineris-term::it system::sys_*`'s `restored` and `cineris-tui::it external::sys_*` read both screens' stacks.

## Finding 11: a failed write leaving the alternate screen was never tried again

**What was wrong.** `Terminal::alternate(false)` set `self.alternate = false` before its write, and cleared the emergency restore's record whatever the write did. `set_mouse(false)` did the same with the mouse. The trigger is one refused write: a `WouldBlock` on a terminal another process made non-blocking, since `write_all` retries only `Interrupted`. After it, neither the end nor an emergency restore turned either mode off, and the shell was left on the alternate screen with the mouse reported.

**What changed.**
- A mode turned off is marked off, and the emergency restore's record with it, only once the write has succeeded. This covers `alternate(false)` and `set_mouse(false)`.
- The keyboard protocol's push in `reply` and `resume` is marked only once written, so a reply after a refused push tries it again.
- A mode turned on is marked on before its write, so that an emergency during the write turns it off. Since the second round it is put back to off when that write fails.
- Putting the terminal back clears the record whatever its own write did. The first round kept it after a failed end; the second round found that wrong.

**The test:** `cineris-term` `terminal::tests::a_failed_write_turning_the_alternate_screen_or_the_mouse_off_is_tried_again`. It covers three cases: the alternate screen's write refused, then an emergency and the end; the same, then the program's next call; and the mouse's write refused, then the end.
- On `main`: "assertion failed: terminal.is_alternate() && terminal.mouse()". Both were marked off by the write that failed.

## The emergency restore's mouse-off under the keyboard protocol and win32 input

The mouse-off of the restores with the keyboard protocol or the win32 input mode on had no test and no break. `sys::tests::the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen` now checks the mouse off in every state with the alternate screen.

Since the second round, the restore is put together from parts, so one piece of code turns the mouse off for every such state. Step 25's break `the-emergency-restore-leaves-the-mouse-on` now takes it out there, and this test catches it too.

## The second round: an independent review of these fixes

1. **The emergency restore popped the main screen's flags where the program never pushed there.**
   - **What happened.** `RESTORE[5]` and `[7]` wrote `CSI < u` after `?1049l` whatever had happened, and the record had no bit for "the main screen holds the push". The full screen pushes on the alternate screen before the main screen has had the flags. A panic, a SIGTERM, a SIGHUP or a SIGTSTP from outside then popped the entry of the shell, or of the program the interface runs under.
   - **The fix.** A record bit, `MAIN_KEYBOARD`, is kept with `main_keyboard`. It is set once a push on the main screen has landed, and cleared when the stop's restore or the end has popped it. The restore is now put together from parts by `restore`, which is still async-signal-safe: one atomic load, then copies into a buffer on the stack. It pops the main screen's flags only when that bit is set. Sixteen hand-written entries would have repeated each other.
   - **Why the bit is set after the push lands.** Set before it, a refused push would have the shell's entry popped. The cost is a signal that lands inside the push's own write, a few bytes long: it leaves the program's flags on the main screen.
2. **A refused write turning the alternate screen on left it marked on.**
   - **What happened.** From the inline screen with the flags pushed, the viewer's `?1049h` and push could be refused. The end then wrote `CSI < u`, `?1049l` and `CSI < u`, all on the main screen, which popped the shell's entry. A retry returned early, at `on == self.alternate`, and never wrote.
   - **The fix.** The screen is still marked on before the write, so that an emergency during the write leaves it. When the write fails, the state is put back: the screen and the decoder's mouse are marked off, and so is the record. This stays on the safe side because a refused write is taken to have landed nothing (point 3), so the main screen is still shown.
3. **Smaller.**
   - **A write of a mode is all or nothing at this layer.** One that failed is taken to have landed none of its bytes, and a part that landed is not tracked. The writes are a few dozen bytes, far below what a terminal's input queue holds, and `write_all` gives up at the first chunk refused. A partial write of `alternate(false)` that is retried therefore sends nothing twice, in all but that edge.
   - **`reply()`'s push had no test that refuses it.** `a_refused_push_of_the_keyboard_flags_is_not_taken_for_one` adds one, with breaks that move `self.keyboard = true`, or the main screen's record, above the write.
   - **A failed end left stale record bits,** which an emergency in the terminal taken back after the external editor would have acted on, popping the shell's entry. The end now clears the record whatever its write did. After the end the terminal is left, so nothing reads the record until a terminal is entered again; the editor's clean-up turns off what may be left on.
   - **`LEFT_ON`'s `CSI < 99 u` landed on the main screen** after an editor that ended cleanly, and took the shell's entry. This is older than these fixes. It now pops on the alternate screen only, taken with `?1047h` first.

**The tests.** Each fails on the first round's code (`d33cd1e2`) and passes with the fix.
- `terminal::tests::a_failed_write_turning_the_alternate_screen_on_leaves_it_off`. On the first round's code: "assertion failed: !terminal.is_alternate() && !terminal.mouse()".
- `terminal::tests::a_refused_push_of_the_keyboard_flags_is_not_taken_for_one`. On the first round's code: "an emergency after the refused push: the main screen's keyboard flags [] (the shell's is [1])" (the restore popped the shell's entry).
- `terminal::tests::a_terminal_after_an_end_that_failed_restores_nothing_of_it`. On the first round's code: the next terminal's restore still held `?1049l` and both pops.
- The shell's entry added to the stack tests above. On the first round's code: `the_keyboard_flags_are_left_pushed_on_neither_screen` gave "reply on the alternate true, left true, an emergency: the main screen's keyboard flags [] (the shell's is [1])", the resumption's and the failed write's tests the same, and `what_a_killed_editor_left_on_is_turned_off_on_both_screens` "the main screen's flags [] (the shell's is [1])". The application's test passed there: its full screen leaves the alternate screen itself before the end, so nothing popped the main screen twice.

## The same mechanism elsewhere

Every place in `cineris-term`, `cineris-tui` and `cineris-cli` that records a terminal mode as set or unset around its write or call was read:

| Where | What it records | Done |
|---|---|---|
| `Terminal::alternate(false)`, `set_mouse(false)` | the alternate screen, the mouse, the emergency restore's record | Fixed (finding 11). |
| `Terminal::alternate(true)` | the alternate screen, the decoder's mouse, the record | Fixed in the second round: marked on before the write, put back when it fails. |
| `Terminal::reply`, `resume` (the keyboard protocol's push) | `keyboard`, `main_keyboard`, the record's `MAIN_KEYBOARD` | Fixed: marked once written. The record's `KEYBOARD` bit, which pops only the alternate screen's stack, is still set before the write. That is the safe side: a pop there of a stack with nothing of the program's takes only what another program left. |
| `Terminal::end` | the record of the alternate screen, the keyboard protocol on both screens and the win32 input mode | Cleared whatever the write did (second round), since nothing reads it until a terminal is entered again. The end itself is not tried again: it is the last thing the terminal does, and the device has been left. |
| `Terminal::reply` (the win32 input mode), `set_mouse(true)`, `sys::windows::enter` | modes turned on | Left: marking on before the write is the safe side, since an emergency between the two turns them off, and these modes are not stacks. A second "on" is harmless, and so is an "off" of a mode that is not on. |
| `sys::unix::restore_modes`, `sys::windows::leave` (raw mode, the console's modes and code pages) | `ACTIVE`, marked off whether or not `tcsetattr` or `SetConsoleMode` succeeded | Left: leaving hands the terminal on, to the shell or to the external editor. A record left on would let a later stop or signal write the restore into a terminal that is no longer the program's, over the editor. |
| `sys::unix::emergency`, `sys::windows::emergency` | `ACTIVE`, swapped off before the restore's write | Left: the restore runs once, in a signal handler or a console event's thread, where nothing can wait for a terminal to take a refused write. |
| `Inline::finish` | `Sink::region(0)` (the rows the emergency restore moves up) before its write | Left. When a restore interrupts the write, the write sends the restore again with the count, which must be the count after the frame. After a failed write nobody knows where the cursor is, so no count is right. The inline presenter's output must not change in this part. |
| `TerminalHandoff::release` and `reclaim` (`cineris-tui`), the full-screen presenter, `secret::read_key`, `cineris-cli` | nothing of their own: they go through `Terminal` | Nothing to do. |

## Changes of format, protocol and public API

- No protocol, log, setting or format change.
- **Public API:** `cineris_testkit::vterm::VTerm::main_keyboard` and `alternate_keyboard` are added (the test kit's; additive).
- `VTerm::keyboard` now gives the stack of the screen shown. That is what it gave every test that read it, since none pushed flags on the alternate screen and read them from the main one.
- Nothing public of `cineris-term` or `cineris-tui` changed. In `sys`, the crate-private `RESTORE` table became `restore`'s parts, and `restore_sequence` is now for tests only.

## Docs changed

- [09-tui.md](../../../architecture/09-tui.md), "Restoring the terminal" and the external editor's "The terminal": the flags per screen; the main screen popped only where the program pushed; a mode marked off only once its write has succeeded, and put back off when turning it on fails; a write of a mode taken as all or nothing.
- [testing-and-ci.md](../../../testing-and-ci.md), "The virtual terminal": the stack per screen.

## Breaks

The list holds 18 breaks.

The first round's run gave 16 breaks, every one caught: `cargo xtask mutate step-25-fixes-terminal --jobs 2` on the reference machine, exit 0.

The second round changed the list:
- **Dropped:** the four that edited `RESTORE`'s literals, whose code is gone, and `an-end-that-failed-clears-the-restores-record`, whose rule was reversed.
- **Changed:** the `LEFT_ON` break, now `taking-the-terminal-back-pops-the-main-screen`.
- **Added:**
  - `the-restore-leaves-the-keyboard-flags-on-the-alternate-screen`, rewritten for the new `restore`;
  - `the-restore-pops-the-main-screen-where-the-program-never-pushed`;
  - `the-restore-leaves-the-main-screens-flags`;
  - `leaving-the-alternate-screen-forgets-the-main-screens-push`;
  - `a-refused-alternate-screen-is-left-marked-on`;
  - `a-refused-push-is-taken-for-one`;
  - `the-main-screens-push-is-recorded-before-it-lands`;
  - `an-end-that-failed-keeps-the-restores-record`.

`cargo xtask mutate --check --tests` passes on the whole list. The second round's list has not been run here: the maintainers run every part's list on the runners after the push.

In other lists:
- `mutations/step-20.breaks`: `keyboard-protocol-left-on` names `occurrence: 2` of the pop in `Terminal::end` (the main screen's), and the restore's automatic-wrap break is an `in-string` of the restore's last part.
- `mutations/step-25.breaks`: `the-emergency-restore-leaves-the-mouse-on` takes out the composed restore's one mouse-off, and names `sys::tests::the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen` too.

`cargo xtask stress -p cineris-term --count 100` and `-p cineris-tui --count 100` (first round): every test passed every time.

## Windows

The restore is the same code on both systems, and its test is not system-specific. Nothing under `sys/windows*` changed, and the workspace was linted for Windows with `cargo xwin clippy`.

What must run on a Windows runner:
- `cineris-term`'s unit tests: `sys::tests::the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen` and the `terminal::tests` named above;
- `cineris-term::it system::sys_*`, whose `restored` reads both screens' stacks through ConPTY;
- `cineris-tui`'s unit tests named above;
- `cineris-tui::it external::sys_*`. `LEFT_ON` now takes the alternate screen (`?1047h`) for a moment as the terminal is taken back, which a pseudo-console renders.

`external::sys_a_killed_editor_keeps_the_prompt_and_its_modes_are_undone` is Unix only.

## Left open

- **A refused write is not waited out.** `sys::unix::write_all` retries only `Interrupted`, so a `WouldBlock` fails the write at once. The fix makes the next call, the end or an emergency try it again; it does not wait for the terminal to take it. Waiting for the terminal to be writable (`poll` for `POLLOUT`) before trying again would close this. Suggested for step 32 (hardening), with the review's other failed-write findings.
- **A killed editor's flags on the main screen are left.** `LEFT_ON` no longer pops the main screen, so as not to take the shell's entry: nothing tells the two apart. An editor that pushed flags on the main screen and was killed leaves them there. The full-screen editors push on the alternate screen.
- **A signal inside the main screen's push.** It lands between the push and its record bit, and leaves the program's flags on the main screen (second round, point 1).
- **The places left in "The same mechanism elsewhere"**, with their reasons.
- **Windows:** the tests named under "Windows" have run on Linux only.
