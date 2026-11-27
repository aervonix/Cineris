A write to the terminal that is refused because it would block is retried at the next call, the end or an emergency, but never waited out, so a terminal another process set non-blocking can still be left with the alternate screen or the mouse on.

- Area: cineris-term
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **A refused write is retried, never waited out** (left by the step 25 fixes, [steps/step-25-fixes/parts/terminal.md](../steps/step-25-fixes/parts/terminal.md)). Since those fixes a mode turned off is marked off only once its write lands, so a write the terminal refused is tried again at the program's next call, at the end or in an emergency. But `sys::unix::write_all` retries only `Interrupted`: a `WouldBlock`, on a terminal another process set non-blocking, fails the write at once, and if it fails again at the end the shell is left with the alternate screen or the mouse on. **A write that lands in part** is taken as none (the last read of the step 25 fixes): when `alternate(true)`'s `?1049h` lands and the keyboard push after it is refused, the screen is marked back to the main one, so the end writes no `?1049l` and the shell is left drawn on the alternate screen; the end could write `?1049l` after a failed "on" write, without the pop, which is harmless on the main screen. **Recommendation:** where a write would block, wait until the terminal can take it (a `poll` for writability, bounded), and retry. | Step 32 (hardening) | Before the first release |
