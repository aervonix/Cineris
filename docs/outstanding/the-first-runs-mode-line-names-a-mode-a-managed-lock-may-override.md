The first run's "Mode: …" line names the mode the person chose, even where a managed file locks another in force.

- Area: cineris-tui
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **The first run says the mode it wrote, not the mode in force.** Since the fixes to step 28 (finding 15 of its review), the first run and `cineris setup skill-folders` load the configuration after writing the skill folders' value and say when the value in force differs (a managed lock, a selected profile), through `Setup::in_force_otherwise`. The line that states the chosen permission mode does not: where a managed file locks `permissions.mode`, the person is told the mode they picked while sessions run at the managed one. The mode in force depends on the project (a project's file and profile may lower it), so the same check needs a project to load against, which the first run has not got. To settle: say the mode in force for the folder the first run started in, or say that a managed file decides it, as the skill folders' line does. | Step 32 (the hardening pass), found by the fixes to step 28 ([steps/step-28-fixes/answered.md](../steps/step-28-fixes/answered.md)) | Before the first release |
