The break that makes `cineris stats` prune what it shows, `step-16/showing-the-statistics-prunes`, is not caught by the test it names, so the rule that showing the statistics deletes nothing is not held by a test.

- Area: cineris-core
- Owner: the maintainers, a small-fix session

| Item | Who | When |
|---|---|---|
| **What was seen.** Step 28's run of the breaks a change can affect (`mutate --changed`, on its fourth pull request) found `step-16/showing-the-statistics-prunes` not caught, and it is not caught on `main` itself either (`36d8266e`, the break run alone). The break makes `Core::stats_summary` (`crates/cineris-core/src/usage.rs`) pass `Some(settings.retention)` where it passes `None`, so that showing the statistics would prune them; `cineris-core::it usage::showing_the_statistics_prunes_nothing` passes with it, likely because nothing in its store is older than the retention it would prune by. **Recommendation:** give the test a store with a month older than the retention, show the statistics, and hold that month to be there still; then run the break again. | The maintainers, a small-fix session | With the next small fixes |
