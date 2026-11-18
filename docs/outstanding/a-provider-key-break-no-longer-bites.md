The step 13a break `provider-key-variables-left-in` no longer fails any test: the providers' key variables reach the shell tool's list of what to remove by two paths, and the break empties only one.

- Area: cineris-core
- Owner: the coordinator

| Item | Who | When |
|---|---|---|
| **The break no longer bites.** Found in step 25, while fixing the Windows `PATH` test, by reading the code, not by a run on `main`: since commit 24673066 (the reload), the providers' key variables are removed from a shell command's environment both through the configuration's `env_remove`, which `assemble.rs` fills from `provider::key_variables` (the line the break empties), and through `ShellLive`, which `ShellTool::removed` adds back (`assemble.rs` around 2411 and 2415). With the break made the variables are still removed, so the test it names, `shell::sys_a_command_is_given_the_environment_the_command_analysis_relies_on`, still passes; it could not have caught it before step 25's change either, since those variables print as short lines, the same in the model's copy the old test read. **Recommendation:** break `provider::key_variables` itself, the one source of both paths, and check that a test fails when either path is lost on its own; or mark the break `expect: survives` with the second path named. | A later small fix session | Before the first release |
