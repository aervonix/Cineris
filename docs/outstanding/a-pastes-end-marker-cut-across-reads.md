A stalled bracketed paste whose end marker is cut across two reads stays open, so what is typed just after it joins the paste for up to 2 s.

- Area: cineris-term
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **The end of a stalled paste is looked for in one read.** `Terminal::pastes` (`crates/cineris-term/src/terminal.rs`, lines 857-860 and 873-893 at the review's commit, `77ffbbcc`) looks for the paste's end marker (`ESC [ 201 ~`) in one read's bytes. When a stalled paste's marker is cut between two reads it is not seen, and the stalled paste's tail stays open for up to 2 s: `ok` and `Enter` typed then come as `Paste("ok\n")`, so nothing is sent. It is safe: nothing is sent or answered. Found by the review of step 25 and confirmed by a scratch test it ran against `cineris-term` (`review25_a_stalled_pastes_end_marker_cut_across_reads_keeps_the_tail`, kept with the review, not in the repository). **Recommendation:** take the paste's end from the decoder, which sees the marker whole however the reads cut it, not from one read's bytes. | Step 33 (terminal) | Before the first release |
