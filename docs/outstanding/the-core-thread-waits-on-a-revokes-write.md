A change of level waits on the core thread for a revoke's or a record's write of the project's approvals, under the lock that keeps them from losing each other.

- Area: cineris-core
- Owner: the next change to the core's policy, or step 32

| Item | Who | When |
|---|---|---|
| **A change of level can wait for a revoke's write of the project's approvals, on the core thread.** Since the re-decide step of step 12, a revoke, an answer being recorded and a change of level take one lock (`EngineInForce` in `cineris-core`'s `policy`), so that none is lost to another. A revoke of a stored approval, and a record of "always allow in this project", hold it while they rewrite the project's approvals file, which takes the file's own lock (shared with every session of the project) and writes it to the disk; both run on the blocking pool. A change of level puts its engine in force from the core's own task, and waits for the lock there. And the engine put in force compiles its rules under the engine's lock, which every decision and every look of the round at what may start reads. Found by the full review of the step. **Why it is small:** each of these is rare, and they meet only when a person revokes, answers and changes the level at once; the wait is one file's write. **What would close it:** put the engine in force from the blocking pool, as the revoke runs; and compile the new engine's rules before taking the engine's lock, taking over only what changed under it. | The next step that changes `cineris-core`'s policy, or step 32 | Before the RPC server serves many sessions (step 27) |
