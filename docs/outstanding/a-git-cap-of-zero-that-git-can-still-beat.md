A test of git's time limit in the look for a command's changes passes only when git has not answered before its limit of zero is asked about; one CI flake.

- Area: cineris-tools
- Owner: the maintainers, to a fix session
- Flaky: `cineris-tools::it changes::sys_git_that_does_not_answer_in_time_is_not_waited_for`

| Item | Who | When |
|---|---|---|
| **A cap of zero that git can still beat.** `cineris-tools`' `changes::sys_git_that_does_not_answer_in_time_is_not_waited_for` gives the change detector a cap of `Duration::ZERO` and expects its answer not to be used (`Method::None`). It failed once in the `test-linux` job of the flaky-test fixes' pull request (run 44454844994, job 135327250391), with `Method::Git`, on a commit that changes nothing on its path. The reading: `process::brief::answer` starts git, hands its output to a reader thread and waits with `recv_timeout(limit)`; with a limit of zero, it takes the answer when the reader has already sent it, which a fast git and a slow waiting thread allow. So the test depends on the order the machine runs two threads in. The product is right either way: an answer already there is not a wait. To settle: either the test holds git back (for example a `git` on `PATH` that waits until the test lets it go), so that "does not answer in time" is made, not hoped for; or a cap of zero starts no git, if a zero cap is meant as "do not ask", and the test says which. Found by the flaky-test fixes ([steps/flaky-test-fixes/answered.md](../steps/flaky-test-fixes/answered.md)). | The maintainers, to a fix session | After the flaky-test fixes |
