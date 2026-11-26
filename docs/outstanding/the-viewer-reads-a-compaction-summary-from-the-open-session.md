The viewer reads a compaction notice's summary from whichever session is open now, so after `/new` an earlier session's notice shows the new session's item of the same id.

- Area: cineris-tui
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **A compaction's summary is read from the wrong session.** `crates/cineris-tui/src/app/state/viewer.rs` (lines 616-639 at the review's commit, `77ffbbcc`) reads the summary of the notice under the cursor by its item id from the session open now; `Block::Compaction` holds no session, and item ids repeat across sessions. After `/new`, `Enter` on the earlier session's notice shows the new session's `i_7`. Found by the review of step 25 and confirmed by a scratch test it ran against `cineris-tui` (`review_a_compaction_notice_of_an_earlier_session_reads_its_summary_from_the_open_one`, kept with the review, not in the repository). Nothing leaves the machine: it is the same person's other session. **Recommendation:** keep the session's id in the block, and read the summary from that session. | Step 32 (hardening) | Before the first release |
