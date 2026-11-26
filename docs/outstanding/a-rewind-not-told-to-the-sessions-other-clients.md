A rewind or undo made by one client of a session is not told to its other clients, which go on showing, copying and undoing the branch as it was.

- Area: cineris-core
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **The other clients are not told that the branch moved.** `Core::rewind` (`crates/cineris-core/src/core.rs`, lines 2179-2328 at the review's commit, `77ffbbcc`) emits only `Event::PlanUpdated`, and an `Info` notice when a file stayed as it was; no event says that the session's active branch moved, and the interface reads its transcript again only after a rewind or undo of its own. **The scenario:** interface A (in process) and interface B (`cineris attach`) share a session; B rewinds before turn 3; A still shows turn 3, and A's `/copy` copies it; A's `/undo`, meant for turn 3, undoes turn 2, and those of turn 2's files that are as the harness left them go back with no question. Found by the review of step 25, by reading only: no test shows it. Step 25's design covers only the client that rewinds; the core's side is from steps 13a and 23. [rewind-and-fork-have-no-test-of-the-real-binary](rewind-and-fork-have-no-test-of-the-real-binary.md) asks for a test under `cineris attach`, which is where this would show. **Recommendation:** an event when the session's active position moves (in `session.updated`, or a new one), on which a client reads its transcript again. That is a protocol addition ([11-rpc-protocol.md](../architecture/11-rpc-protocol.md), its versioning rules). | Step 32 (hardening) | Before the first release |
