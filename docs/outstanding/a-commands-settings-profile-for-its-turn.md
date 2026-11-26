A custom command's front matter may name a settings profile for its turn, but no mechanism applies a profile for one turn, so since step 28 a command that names one is judged and not sent.

- Area: cineris-core
- Owner: a later step, the maintainers

| Item | Who | When |
|---|---|---|
| **What is open.** [12-extensions.md](../architecture/12-extensions.md), "Tier 1: custom commands", lets a command's front matter name "a settings profile for the turn". Step 28 reads `profile` and judges it key by key, as a project's file is, against the session's values in force ("As built in step 28: custom commands"), but cannot apply it: the only way a profile takes effect in a session is `/profile`, a session reload that keeps the profile after the turn, drops the user's `session/set` choices, and defers model and prompt keys to the next epoch. Sending the command without its profile could run it wider than its author meant, so a command that names one is listed as not used and refused (`command_refused`), with what the profile would widen or that it is not applied in this build (settled by the maintainers). **What would close it:** a turn-scoped narrowing in the policy and the loop, limited to the keys that take effect at once (the level, the sandbox's network and write roots, verification, the limits), applied for the command's turn and put back when it ends, never widening the session; then a command's profile that passes the judgment is applied for its turn. | A later step, the maintainers | When custom commands' profiles are wanted |
