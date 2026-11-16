The shell's look at changes gave up as too slow on a loaded Windows runner, failing one system test once.

- Area: cortex-tools
- Owner: the next step that changes the look, or step 32
- Flaky: `cortex-tools::it changes::sys_every_git_the_look_starts_goes_through_its_confinement`

| Item | Who | When |
|---|---|---|
| **The shell tool's look at what a command changed, given up as too slow on a loaded Windows runner.** `cortex-tools`' `changes::sys_every_git_the_look_starts_goes_through_its_confinement` failed in the first CI run of pull request 50 (run 43672945463, `test-windows`): the look came back `complete: false, incomplete: Some(GitTooSlow)`, its git past the look's time cap (`DEFAULT_CAP`, 500 ms), on a runner so loaded that two other tests of the crate took over 5 s and tier 2 read 107% of its budget. A rerun of the job passed. The pull request did not change `cortex-tools`. The question: should the test give the look a cap of its own, as a test of what the look runs and not of how fast git is, so that a slow runner cannot fail it; or is 500 ms too short for the product on a loaded Windows machine too? | The next step that changes the shell's look at changes, or step 32 | Before the first release |
