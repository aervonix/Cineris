# Answered in the twelfth tooling step, on 2026-11-19

This step is not a numbered step of the build plan. It builds the tools and repo rules that the wave 14 retrospectives asked for: what made rebases, Windows round trips, flaky tests, break runs and the public docs cost sessions time. It is three pull requests, each with a record of its own in [parts/](parts/): the repo rules and the test kit, rebasing and git, and CI and measurement.

## The maintainers' answers

- **The cut.** Three pull requests, in the order the step asked for: the repo rules and the test kit first, then rebasing and git, then CI and measurement. The second and the third were built beside the first, each from `main`, and each rebases over the one before it when it merges.
- **The words of the private setup** (the first pull request's rule): the list is taken from what pull request #106 took out of the docs. Words the product uses too are not on it ("worktree", "WSL", "sub-agent", the names of other agents), and Rust comments are not read; the one message that printed a private command, the test kit's corpus message, was reworded. The list, its `allow` lines and the step records written before it are in one data file, `xtask/public-words.txt`, so that a change is one line, and a failure names the word, the file and line, and the public alternative.
- **Step records are public text.** Those written from this step on are held to the rule; the folders written before it are named in the data file and keep their words.
- **Where the process rule lives.** First in `cineris-tools`' runner, before it starts a command. A refusal of every start, in `cineris_base::hook::started`, which every place that starts a process calls, was then measured (below), and on the maintainers' answer it replaced the runner's: it covers the harness's git, the pseudo-terminal and the test kit's children too, at no cost to the product. The list of a test's children for the timeout report is kept in the same module.
- **Two asks of the label rename's retrospective**, which merged while this step was built: a note on adding a repo rule (in the first pull request, [testing-and-ci.md](../../testing-and-ci.md#adding-a-repo-rule)), and a warning from `mutate` when a break list's name is one the rename would rewrite (the third pull request, if it is cheap).

## A refusal of every process start, measured

With the refusal in `cineris_base::hook::started`, the whole workspace's tests were run on Linux (4,932 tests): none was stopped. A planted test not named `sys_` was stopped, with its message, so the count is not a check that never fired; a system test now checks the same end to end. On Windows: see the first pull request's record. A version probe is refused where the harness decides to ask (`hook::starting`), not on the probe's own thread, which a test may end before or after the probe starts: a refusal there would stop the test or not by that race.
