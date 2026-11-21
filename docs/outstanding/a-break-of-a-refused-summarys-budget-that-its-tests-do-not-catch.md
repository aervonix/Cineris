The break that makes a summary request refused as too long teach nothing, `step-15-fixes/refused-summary-teaches-nothing`, is caught by other tests but not by one its list names, so the list no longer says which tests hold the rule.

- Area: cineris-agent
- Owner: the maintainers, a small-fix session

| Item | Who | When |
|---|---|---|
| **What was seen.** Step 28's run of the breaks a change can affect (`mutate --changed`, on the fourth pull request) found `step-15-fixes/refused-summary-teaches-nothing` not caught by its own tests, on `main` itself too (`36d8266e`, the break run alone); the run's whole verdict (`NOT CAUGHT BY ITS OWN TESTS`) is that the test it names `a_background_summary_refused_as_too_long_teaches_the_fast_roles_budget_alone` passes with the break made, while others of `cineris-agent::it summaries` catch it, so the rule is held. The break deletes the line in `Session::counted` (`crates/cineris-agent/src/turn/epochs.rs`) that has a summary's refusal teach its role's budget (`budget_for_mut(…).overflowed(…)`); the background summary's test passes with the line gone, likely because the fast role's budget it holds is taught by another path. **Recommendation:** take that test out of the break's `caught-by` (or have it hold what only this line teaches), keep the tests that catch it, and run the break again. | The maintainers, a small-fix session | With the next small fixes |
