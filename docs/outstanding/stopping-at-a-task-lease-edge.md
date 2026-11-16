At a task lease's edge the loop should stop and report, then fall back to the standing lease.

- Area: cortex-agent
- Owner: step 37

| Item | Who | When |
|---|---|---|
| **Stopping at the edge of a task lease.** Step 12 built the way for the policy to end a turn (`Decision::Stop`, `Verdict::Stop`), and the loop ends the turn with the outcome `denied`. What is left is for task leases: at the edge of one the holder stops and reports, and the session falls back to its standing lease. | Step 37 | In that step |
