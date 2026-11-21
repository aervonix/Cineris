The break that makes a summary request refused as too long teach nothing, `step-15-fixes/refused-summary-teaches-nothing`, is not caught by the tests it names, so the rule that such a refusal teaches the budget is not held by a test.

- Area: cineris-agent
- Owner: the maintainers, a small-fix session

| Item | Who | When |
|---|---|---|
| **What was seen.** Step 28's run of the breaks a change can affect (`mutate --changed`, on the fourth pull request) found `step-15-fixes/refused-summary-teaches-nothing` not caught by its own tests, and it is not caught on `main` itself either (`36d8266e`, the break run alone). The break deletes the line in `Session::counted` (`crates/cineris-agent/src/turn/epochs.rs`) that has a summary's refusal teach its role's budget (`budget_for_mut(…).overflowed(…)`); its three tests in `cineris-agent::it summaries` pass with the line gone. Either another path teaches the same budget, and the break is one no test can catch, or the tests do not look at what was taught. **Recommendation:** find which: if another path teaches it, say so in the break's rule and expect it to survive, or delete the break; if not, have one of the tests hold the next summary request to the budget the refusal taught, then run the break again. | The maintainers, a small-fix session | With the next small fixes |
