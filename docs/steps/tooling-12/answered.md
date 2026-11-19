# Answered in the twelfth tooling step, on 2026-11-19

This step is not a numbered step of the build plan. It builds the tools and repo rules that the wave 14 retrospectives asked for: what made rebases, Windows round trips, flaky tests, break runs and the public docs cost sessions time. It is three pull requests, each with a record of its own in [parts/](parts/): the repo rules and the test kit, rebasing and git, and CI and measurement.

## The maintainers' answers

- **The cut.** Three pull requests, in the order the step asked for: the repo rules and the test kit first, then rebasing and git, then CI and measurement. The second and the third were built beside the first, each from `main`, and each rebases over the one before it when it merges.
- **The words of the private setup** (the first pull request's rule): the list is taken from what pull request #106 took out of the docs. Words the product uses too are not on it ("worktree", "WSL", "sub-agent", the names of other agents), and Rust comments are not read; the one message that printed a private command, the test kit's corpus message, was reworded. The list, its `allow` lines and the step records written before it are in one data file, `xtask/public-words.txt`, so that a change is one line, and a failure names the word, the file and line, and the public alternative.
- **Step records are public text.** Those written from this step on are held to the rule; the folders written before it are named in the data file and keep their words.
- **Where the process rule lives.** The refusal is in `cineris-tools`' runner, before it starts a command, keyed on nextest's name for the test; the list of a test's children for the timeout report is kept in `cineris_base::hook`, beside the existing mark of each start. A refusal of every start, not only the runner's, was measured and not turned on (below).

## A refusal of every process start, measured

With a line added for the run alone, `cineris_base::hook::started`, which every place that starts a process calls, stopped any test not named `sys_…` or `perf_…`. The whole workspace's tests were run on Linux (4,931 tests): none was stopped. A planted test not named `sys_` was stopped by the same line, with its message, so the count is not a check that never fired. Windows-only tests did not run there. So the wider rule would cost nothing on Linux today; it is left off, and the maintainers can turn it on with the security fix that names every place a process may start.
