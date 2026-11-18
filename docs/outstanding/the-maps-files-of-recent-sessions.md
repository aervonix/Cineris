The map's ranking by the files of recent sessions has its seam (`RecentFiles`) and no source: step 16's tool records name no files.

- Area: cineris-core
- Owner: the step that has the statistics record files

| Item | Who | When |
|---|---|---|
| **The map's second signal: the files of recent sessions.** The map ranks the files this session's tools touched, then the files of recent sessions, then the files changed most recently ([15-efficiency.md](../architecture/15-efficiency.md), "Repository map"). The second comes from the statistics, which step 16 builds beside step 18. Step 18 left the seam: `cineris_index::RecentFiles`, which the core fills with `NoRecentFiles` (`crates/cineris-core/src/assemble.rs`, the repository index); a session's index asks it once for each map. What fills it: the files the statistics recorded tools reading and changing in this project's last few sessions, most recent first, under the workspace with `/`, read off the start-up path (the index asks only when a map is wanted). | The step that has the statistics record files: step 16 merged first, and its tool records name the tool and the project, not the files (`cineris_stats::ToolRecord`), so step 18 left the seam empty | When `cineris-stats` records files per session |
