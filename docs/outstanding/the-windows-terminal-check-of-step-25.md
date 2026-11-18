The checklist for the real interface on Windows Terminal, step 25's `pr3-wt-checklist.md`, has not been run.

- Area: windows
- Owner: the owner (Windows Terminal check)

| Item | Who | When |
|---|---|---|
| **The Windows Terminal check of step 25.** The checklist of [pr3-wt-checklist.md](../steps/25/parts/pr3-wt-checklist.md) runs the `dist` build in a fresh Windows Terminal window on the desktop, against the test kit's scripted provider on the loopback: each presenter's rows, the wheel and selection, the function keys, pastes, `/copy`, search, a dialog read from below, the first run, `cineris auth login`, and the old console host. The coordinator's script does the rows it can, from a one-off scheduled task with `/IT`; the owner does the five that need a person. **Run it in one sitting with `key-latency-over-conpty`** (its row K1, `cargo xtask perf` in a Windows Terminal tab). **The results go to** decision 0001, "Checked on Windows", with the screenshots kept outside the repository; rows A1 to A4 update `the-mouse-under-windows-terminal`, a pass of row A5 deletes `function-keys-through-conpty`, and K1's numbers go into `key-latency-over-conpty`. This file is deleted once the results are recorded. | The owner, with the coordinator's script | After step 25's third pull request merges |
