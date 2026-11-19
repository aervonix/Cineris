A session's base capture can come out incomplete when the turn's own git runs beside it; one CI flake.

- Area: cineris-core
- Owner: the next step that changes the base capture, or step 32
- Flaky: `cineris-core::it trust::sys_a_project_trusted_in_the_store_is_trusted_while_its_rules_are_the_ones_the_user_saw`

| Item | Who | When |
|---|---|---|
| **A base capture that comes out incomplete while the turn's own commands run.** `cineris-core`'s `trust::sys_a_project_trusted_in_the_store_is_trusted_while_its_rules_are_the_ones_the_user_saw` failed once on a hosted Linux runner (the fifth tooling step's A/B, `try` run 43671665085, round 1 on the branch's side; `main`'s side passed): the session's base record said `complete: false` where the test expects a complete one. The capture runs git on a thread of its own while the turn runs `echo more >> tracked.txt` and `git status --short`; a `git status` that refreshes the index takes its lock, and a capture whose git cannot list the untracked files records itself incomplete. The test passed 300 of 300 alone under `stress --cores 2 --load 6` on the reference machine: it needs the whole workspace's load beside it. (It is a known flake since the tenth tooling step: stress it with `--strict`, or `stress` runs a failure once more and excuses it.) The step did not change the test, the capture or the shell. The question: is an incomplete record there the right answer (the capture should then retry once, or the record say why), or should the test wait for the capture before the turn's commands run? The record's `note` says which git command failed, and a rerun that fails should print it. | The next step that changes the base capture, or step 32 | Before the first release |
