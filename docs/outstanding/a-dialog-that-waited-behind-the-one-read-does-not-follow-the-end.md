In the full screen, a dialog that waited behind the one being read opens without following the end, and so does one after the wheel turned under a dialog.

- Area: cineris-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **The next dialog does not follow the end.** `crates/cineris-tui/src/app/state.rs` (lines 1761-1767 and 1375-1382 at the review's commit, `77ffbbcc`): when the transcript is being read above a dialog (`Tab`) and a second dialog waited behind it, the second opens with the view where the reading left it; the same happens after the wheel scrolled under a dialog. Found by the review of step 25 and confirmed by a scratch test it ran against `cineris-tui` (`review_a_dialog_that_waited_behind_the_one_read_follows_the_end_when_it_opens`, kept with the review, not in the repository). From step 25's second pull request. **The doc is the specification here, and the code differs from it:** [09-tui.md](../architecture/09-tui.md), the approval dialog, says reading ends with its dialog however that goes, "and the next dialog follows the end as any does". **Recommendation:** follow the end whenever a dialog becomes the one shown, the one that waited included, and when a dialog's reading ends after the wheel moved the view. | Step 33 | Before the first release |
