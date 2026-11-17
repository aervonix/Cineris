Each full-screen frame copies the rows an answer still streaming has committed, so a very long open answer costs more per frame.

- Area: cortex-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **A frame copies the open answer's committed rows.** The full screen and the viewer draw an answer that is still streaming from the rows it has committed (`Transcript::open_rows`) and its live rows, put together for each frame (`App::screen_live`) and handed to the window as one block. The review of step 25 found them copied twice; the second copy is gone (the window shares the block), and one remains. Measured on the desktop (`perf_a_frame_in_the_full_screen_presenter`, a release build, 120 by 40, after 200 turns): an answer streaming at its start takes 0.105 ms a frame at the median and 0.208 ms at the 99th percentile; the same answer 2,500 paragraphs long, about 5,000 rows, still open, 0.367 ms and 0.508 ms. Within the frame budget, but a frame's cost grows with the answer, where the rest of the full screen costs only the rows near the window. Recommendation: keep the open rows in the transcript as one shared block (`Arc<Vec<Line>>`, appended to in place when no frame holds it), give the window the open rows and the live rows as two blocks, and so make a frame's cost the rows it shows. | Step 33 (performance) | Before the first release |
