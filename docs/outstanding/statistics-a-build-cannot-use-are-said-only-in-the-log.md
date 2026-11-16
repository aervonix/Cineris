A newer build's or an unreadable statistics file is said in the log and by `cortex usage` and `cortex stats`, not by the application.

- Area: cortex-agent
- Owner: step 24

| Item | Who | When |
|---|---|---|
| **Statistics this build cannot use are said only in the log, and when asked.** Step 23's design had the interactive application say, as a notice at the start, that the usage statistics' totals were written by a newer build or cannot be read ([steps/23/answered.md](../steps/23/answered.md), "The design, before building", point 5). Built as a look at the totals when the first session opens, it broke step 16's rule that the statistics are read only when they are asked for, which `usage::a_turn_reaches_the_report_from_the_files_and_its_cost_stays_as_it_was_priced` holds; and the writer, which reads them anyway, finds such a file only when it settles, after the first events, perhaps not before the program ends. So step 23 left it: the writer's warning is in the log, `cortex usage` refuses with `stats_newer` and says why, and `cortex stats` says what it found and where a file was set aside. **Recommendation:** step 24's usage panel and `/usage`, which read the statistics when the person asks, say there what `cortex stats` says; a notice at the start, if it is still wanted, takes a report from the writer to the open sessions (a hook in `cortex-stats`' `RecorderOptions` called where the writer now warns), not a read of its own. | Step 24 (the usage panel and `/usage`), moved from step 23 | Before the first release |
