The viewer and the full screen open a finished command's box on the last 64 KiB of its output, not the whole of it.

- Area: cortex-tui
- Owner: step 25

| Item | Who | When |
|---|---|---|
| **A kept box holds only the end of a long output.** Since step 25 the application keeps the transcript's finished blocks for the viewer and the full screen (`transcript::Kept`); a round keeps each command's output to its last 64 KiB, as a running command does, so the viewer shows what it showed before. Step 25's design had a smaller bound (16 KiB) and the whole output read when the box is opened: the call's result item (`session/read` with `item_id`, which holds up to the tool's 32 KiB) and past that its artifact (`session/artifact`), with `reading…` in the box until it comes. That reading is not built, so the bound stays at 64 KiB, and a session of 200 turns that each run three commands printing 64 KiB or more would keep about 38 MB in its rounds, near the inline presenter's 40 MB budget; the realistic 200-turn session keeps far less (step 25's memory measurement). Diffs are not bounded either. Recommendation: build the read on opening, then lower the bound to 16 KiB and bound a diff to its first 2,000 lines. | Step 25, its second pull request, or step 33 | Before the first release |
