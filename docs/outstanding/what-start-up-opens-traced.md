The blocking check that start-up opens no file outside an allow-list does not exist; the index's part is checked with a held gate.

- Area: cineris-core
- Owner: the next start-up step, or step 33

| Item | Who | When |
|---|---|---|
| **What start-up opens, traced.** [performance-budgets.md](../performance-budgets.md) lists "no unexpected file access before the first frame" as a blocking tier-2 check, and nothing implements it: `cargo xtask startup` times a run and traces nothing. Step 18 checked one part, the repository index, with a gate held closed while a session opens (`cineris_testkit::Untouched`, `cineris-core`'s `index::the_index_is_not_touched_until_the_session_is_open`), and step 16 checked the statistics with a count of the files it opens for reading, per directory (`cineris_stats::reads`, `cineris-core`'s `usage` tests). Moving that count to `Untouched` is not mechanical: the statistics open their files themselves, and `Untouched` counts calls of a hook, so `cineris-stats` would need a hook in its opening; both stay, and the general check would replace them. The general check traces every file the process opens before its first request or its first frame (on Linux `strace -e trace=openat` or `fanotify`, on Windows Process Monitor's or ETW's file events), and compares it with an allow-list of the configuration files, the catalog's overlay, the session's own files and the shell cache. | The next step on start-up, otherwise step 33 | Before the first usable release |
