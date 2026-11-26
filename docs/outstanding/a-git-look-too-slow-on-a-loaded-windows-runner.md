The shell's look at changes gave up as too slow on a loaded Windows runner, failing one system test once.

- Area: cineris-tools
- Owner: the next step that changes the look, or step 32
- Flaky: `cineris-tools::it changes::sys_every_git_the_look_starts_goes_through_its_confinement`
- Flaky: `cineris-core::it trust::sys_a_project_trusted_in_the_store_is_trusted_while_its_rules_are_the_ones_the_user_saw`

| Item | Who | When |
|---|---|---|
| **The shell tool's look at what a command changed, given up as too slow on a loaded Windows runner.** `cineris-tools`' `changes::sys_every_git_the_look_starts_goes_through_its_confinement` failed in the first CI run of pull request 50 (run 43672945463, `test-windows`): the look came back `complete: false, incomplete: Some(GitTooSlow)`, its git past the look's time cap (`DEFAULT_CAP`, 500 ms), on a runner so loaded that two other tests of the crate took over 5 s and tier 2 read 107% of its budget. A rerun of the job passed. The pull request did not change `cineris-tools`. The question: should the test give the look a cap of its own, as a test of what the look runs and not of how fast git is, so that a slow runner cannot fail it; or is 500 ms too short for the product on a loaded Windows machine too? **Seen on Linux too, in `cineris-core`:** the second small-fix session's stress of `trust::sys_a_project_trusted_in_the_store_is_trusted_while_its_rules_are_the_ones_the_user_saw` (200 times with two cores under load, on the reference machine) failed once in 200, during a stall in which every task waited on the disk 46% of the time: the turn's look at its command gave up with `GitTooSlow`, so the change was in no checkpoint. Its base capture, the flake that session settled, was recorded and complete in every run. | The next step that changes the shell's look at changes, or step 32 | Before the first release |
