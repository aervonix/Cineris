In the full screen, a search whose answer comes while a dialog is up never brings its match into view once the dialog goes.

- Area: cineris-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **The reveal is promised and never made.** `crates/cineris-tui/src/app/state/screen.rs` (lines 770-776 at the review's commit, `77ffbbcc`) returns without moving the view when the search's answer comes under a dialog, and nothing calls `reveal_match` again, though its comment (lines 771-773) says the match is revealed once the dialog goes. Found by the review of step 25 and confirmed by a scratch test it ran against `cineris-tui` (`review_a_search_answered_under_a_dialog_reveals_its_match_once_the_dialog_goes`, kept with the review, not in the repository). The comment says what was meant; the code differs from it. **Recommendation:** keep such an answer's match as one to reveal, and reveal it when the dialog goes, unless the person moved the view meanwhile. | Step 33, with the search's other open points | Before the first release |
