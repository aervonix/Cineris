With `fail_on_denial`, an extension's host call nobody can be asked about ends the script's call, and the turn goes on: `cineris run` can exit 0 after it.

- Area: cineris-agent
- Owner: the owner

| Item | Who | When |
|---|---|---|
| **A stopped host call does not end the turn.** A host call the policy says to stop at (one nobody can be asked about under `fail_on_denial`, or one in a turn a check stopped) ends the script's call at once: nothing more of the script runs, the tool's result is `denied` and names the stop, and the denial is in `permission_denials`. But in the loop only the policy's decision of a round's own call stops a turn, and a tool's result cannot: the model is asked again, and **`cineris run` can exit 0 after a host call was stopped**, its report listing the denial. Found in step 28's early review. **What would close it:** the round honouring a tool result marked as a stop as it honours the policy's (a change to the loop, which step 28 may not make without the owner's answer: a commit of its own, kept last), or a narrower one the maintainers may settle, such as the headless driver judging `permission_denials` when the run ends. This item closes with whichever lands. | The owner | Before extensions are used in a headless run with `fail_on_denial` |
