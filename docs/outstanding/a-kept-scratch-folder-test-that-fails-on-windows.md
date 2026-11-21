The `mutate` test of a failed baseline's kept scratch folder with two jobs fails now and then on a Windows runner; Linux passes it.

- Area: tooling
- Owner: the maintainers
- Flaky: `xtask::bin/xtask mutate::drive::tests::with_two_jobs_a_failed_baselines_folder_is_still_there_when_the_run_stops`

| Item | Who | When |
|---|---|---|
| **The two-job test of a kept scratch folder fails on Windows.** `xtask`'s `mutate::drive::tests::with_two_jobs_a_failed_baselines_folder_is_still_there_when_the_run_stops` failed on both sides of a Windows A/B against `main` (`try` run 44873812174: `main`'s round 2 and the branch's round 1), and passed 200 of 200 under `stress` on Linux. It fails in its second command, the one with nothing failing (`go("none")`), with `could not make the tests' scratch folder …\tests\cineris-mutate-test\job-2: Access is denied. (os error 5)`: the job could not make its folder in the command's folder, which is removed while the command runs whenever it is found empty. Until it is fixed, `stress`, `ci` away from CI and `mutate`'s baseline run it again once when it fails. | The maintainers | Before the next tooling step that changes `mutate` |
