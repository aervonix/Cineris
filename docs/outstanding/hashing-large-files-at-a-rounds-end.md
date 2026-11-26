A changed file is hashed whole at round end and on rewind, uncapped, at 0.4 s a gibibyte.

- Area: cineris-session
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **A cap on hashing whole files on a round's path.** A file a shell command changed is hashed whole when the round ends, to record what the round left, and a rewind hashes each file it would put back to see whether it is as the harness left it: about 0.4 s a gibibyte, and nothing caps it. Step 13a left it as it is. A cap would record such a file by its size and modification time, and say in the rewind's plan that it could not be vouched for. The review of step 25 adds that the rewind's plan and the rewind each hash every undone file while they hold the log's lock, so a large file holds the session's other writers too. | Step 33 (the performance pass) | Before the first release |
