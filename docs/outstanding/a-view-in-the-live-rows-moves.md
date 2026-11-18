A full-screen view scrolled up into the live rows moves when a block above it is kept.

- Area: cineris-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **A view in the live rows moves.** A window scrolled up into the live rows (an answer still streaming above a running command) is anchored by the live block's index and a row of it. When the answer finishes and is kept, that index names the kept answer instead (`Scroll::starting` keeps the index): a top inside the answer's rows stays where it was, but one below them, in what is still live, is clamped to the answer's last row, so the window jumps up to the answer's end. Found by the review of step 25, the mechanism read from the code by its second review, not seen in a run. Recommendation: anchor such a window to the block it shows (the answer by its item), not to the live block's rows. | Step 33 | Before the first release |
