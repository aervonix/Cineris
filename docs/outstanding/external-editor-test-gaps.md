Parts of the external editor (umask, ConPTY modes, failed hand-over, `.cmd` editors, interrupt guard) are untested.

- Area: cortex-tui
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **What step 21's tests of the external editor do not reach.** (1) The check that the prompt's file is private reads its permission bits, so under a umask of 077 a file created without care would pass it too; CI's umask is 022. (2) The modes the terminal has while the editor runs are checked from the virtual terminal on Linux only: under ConPTY the screen is ConPTY's rendering, so on Windows only the console's modes are compared. (3) A failed hand-over (`EditorError::Release`) is reasoned about, not provoked: it needs a terminal that refuses one write. (4) The Windows search for `.cmd` and `.bat` editors is tested as a search; starting a real `code.cmd --wait` is not. (5) Two parts of the interrupt guard are reasoned about, not provoked: on Unix, the taking of a signal still pending for another thread when the guard ends, and the second of grace after it (a loaded stress run found the race they close, and passes 100 times since); on Windows, the second of grace for a `Ctrl+C` event whose handler thread starts late. (6) Guards do not nest: a second one taken while one lives would end the first's protection when it ends; nothing takes two. | Step 32 (hardening) | Before the first release |
