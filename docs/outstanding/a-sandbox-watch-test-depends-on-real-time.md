A test of the sandbox's watch of protected paths fails when its walk takes longer than the watch's 250 ms of real time, which a loaded runner can; one CI flake.

- Area: cineris-sandbox
- Owner: the maintainers, who queue the fix
- Flaky: `cineris-sandbox protect::tests::a_cargo_made_as_a_link_is_found`

| Item | Who | When |
|---|---|---|
| **A test of the sandbox's watch that depends on real time.** `cineris-sandbox`'s `protect::tests::a_cargo_made_as_a_link_is_found` failed once in the `test-linux` job of the eleventh tooling step's pull request (CI run 44454804717, first attempt), on `assert!(before.is_complete())`. It passed when the job was run again, and the pull request changes nothing in the crate. The watch it builds (`Watch::new`, with `Limits::default()`) stops walking after `Limits.time`, 250 ms of real time, and reports itself incomplete. The test took 0.444 s, so the walk of a scratch workspace of a few files ran past the limit while the runner was busy. Rule 12 of the build plan keeps real waiting out of tests, and a limit on real time is the same exposure. **Recommendation:** have the tests that need a complete walk give the watch its limits (`Watch::with_limits`) with a time no runner reaches. Keep one test of the time limit itself on a clock the test controls, if the watch can take one, or with a limit of zero. Look for the other tests of `protect` that rely on `Limits::default()` the same way. | The maintainers, through the coordinator | The next fix session |
