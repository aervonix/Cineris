`extensions/status` with no session answers any client, an observer included, with the user's own extensions, and reads and hashes their folders again on every call.

- Area: cineris-core
- Owner: step 32 (hardening pass)

| Item | Who | When |
|---|---|---|
| **What is open.** `extensions/status` without a `session_id` (since step 28's third pull request; [12-extensions.md](../architecture/12-extensions.md), "As built in step 28: managing extensions") says what a session with no project would find of the user's own extensions: each one's folder under `~`, its hash, its standing, its commands and its hooks. It is answered for any client of the server, an observer included, where the same method with a session needs that session open; and each call lists the user's folder of extensions and reads and hashes every folder in it again (`ext::manage::status`), with no limit on how often. No file's contents leave it, and an observer is a client the user started, so this is a matter of what an observer is told and of the cost of a client that asks in a loop. Found by the full review of step 28 (its prompt area, lower confidence), and left for step 32 by the maintainers. **What would close it:** decide whether an observer may ask for the user's extensions at all (refuse it, as an observer's `turn/start` is refused, or answer it only the names); and answer repeated calls from the stamps a session keeps (`ext::watched`), reading a folder again only when its stamp changed. | Step 32 (hardening pass) | Before the first release |
