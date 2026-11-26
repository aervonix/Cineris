In the full screen, re-wrapping at a new width keeps the row's number inside the top block, not the place being read: paragraph 47 read at 99 columns becomes paragraph 35 at 40.

- Area: cineris-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **A resize keeps the number of the row, not what it showed.** `crates/cineris-tui/src/app/scroll.rs` (lines 229-240 at the review's commit, `77ffbbcc`) keeps the same block at the top on a change of width, and the same row number inside it; in a long answer, a row's number at one width is another paragraph at another. Reading paragraph 47 at 99 columns and narrowing the window to 40 shows paragraph 35. Found by the review of step 25 and confirmed by a scratch test it ran against `cineris-tui` (`review_a_resize_keeps_the_place_read_inside_a_long_answer`, kept with the review, not in the repository). **The doc is the specification here, and the code differs from it:** step 25's design ([answered.md](../steps/25/answered.md), point 2, "Re-wrapping on resize") keeps "the same block at the top and the nearest row inside it". It is another way the view moves beside [a-view-in-the-live-rows-moves](a-view-in-the-live-rows-moves.md). **Recommendation:** anchor the top inside its block by a place in the block's source (the source line and the offset the top row begins at), and find the row that holds it at the new width. | Step 33 | Before the first release |
