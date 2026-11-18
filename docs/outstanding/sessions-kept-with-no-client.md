A session that `serve --socket` keeps after its last client goes is never ended by time, and runs its turn on with nobody watching.

- Area: cineris-core
- Owner: the owner, then a later step of the server

| Item | Who | When |
|---|---|---|
| **Sessions kept with no client.** `serve --socket` turns on `CoreOptions::keep_sessions` (step 27), so a session whose last client disconnects goes on: its turn runs to its end, an approval in it waits for a `user` client to open the session, up to `rpc.approval_timeout`, and another client can open it again ([architecture/11-rpc-protocol.md](../architecture/11-rpc-protocol.md), "As built in step 27"). Nothing ends such a session but a client that may drive it closing it, or the server's end. **What one costs:** memory, about 236 KiB of the server's resident memory for an idle session with no turn (40 sessions took a debug build from 27.7 to 36.9 MiB, measured on the desktop in step 27); and while its turn runs, the model's spending, with nobody there to stop it beyond the session's own limits (`limits`, the lease's budget). **To choose:** whether a kept session with no client ends after a time (a setting such as `rpc.kept_session_idle`), whether a turn running with no client is interrupted at once or allowed to finish, and whether `session/list` should say which sessions are running with no client. | The owner (what a server may do with nobody watching), then the step that adds it | Before the first release |
