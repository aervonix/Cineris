The viewer's `/` search query has no bound, so a large paste into it is copied and sanitised at every frame.

- Area: cineris-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **The viewer's query is unbounded.** `crates/cineris-tui/src/app/state/viewer.rs` (lines 575 and 828-833 at the review's commit, `77ffbbcc`) takes any length of query, and the frame copies and sanitises it each time it is drawn. The full screen's query was bounded at 256 characters (`QUERY_MOST`, `app/state/screen.rs`) after the review of step 25's first pull request; the viewer's twin was not. Found by the review of step 25, by reading only. **Recommendation:** bound the viewer's query by the same constant, as the full screen's search does. | Step 33 (performance) | Before the first release |
