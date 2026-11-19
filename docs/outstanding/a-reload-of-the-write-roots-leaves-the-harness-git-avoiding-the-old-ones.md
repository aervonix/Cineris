A reload that changes `sandbox.write` leaves the harness's git, and its `PATH`, keeping out of the write roots the session started with.

- Area: cineris-core
- Owner: the maintainers

| Item | Who | When |
|---|---|---|
| **The harness's git keeps out of the session's first write roots.** Found by the early review of the harness's own processes' fixes ([../steps/harness-process-fixes/answered.md](../steps/harness-process-fixes/answered.md)). `SessionGit` is given the workspace and the resolved `sandbox.write` roots when the session is put together (`assemble.rs`, `harness_avoid`). It looks git up again at every start, and builds git's `PATH` without the directories inside them. A reload that adds a write root changes what a confined command may write from the next command on, and leaves those lists as they were. A git, or a filter git finds by a bare name, that a confined command planted in the new root could then be the one the harness's git runs. **Why it is low:** while commands are confined the harness's git runs inside the sandbox, reading only, with no network, so what it runs is confined as the command was; and the new root has to be on `PATH` ahead of the real git. **Recommendation:** give `SessionGit` its roots through a value the reload sets, as the key variables are (`child_env::Withheld`), or turn the sandbox's reload of `sandbox.write` into one that also refreshes them. Not changed in the fixes: the shell is found once per session, and its search is not reloaded either. | The maintainers | Before the first release |
