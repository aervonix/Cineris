The launcher's refusal is read from the command's own output, so a check can pass for blocked and not be fed back.

- Area: cortex-sandbox
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **The sandbox's launcher refusal is read from what the command printed.** `cortex_sandbox::explain` takes a line that begins `cortex-sandbox:`, with exit status 125, 126 or 127, for the launcher's own refusal to start a command, and that line comes from the command's output pipe, which the command writes too. So a command can print such a line and exit 127, and be taken for one the sandbox refused. For the model's own command that only changes the line its result ends with. For one of the project's checks (step 19) it makes the check `blocked`: a failure the model is not told of (the user is, with the line). The checks run only in a project the user trusts, so it takes a trusted repository's own test to do it. Recommendation: the launcher gives its refusal on a channel the command cannot write (a file descriptor the launcher closes before it becomes the command, or the exit status of a launcher that exits before `exec`), and `explain` reads that. ([steps/19/answered.md](../steps/19/answered.md), "The second read") | Step 32 (the hardening pass), or step 49 with the Windows backend | Before the first release |
