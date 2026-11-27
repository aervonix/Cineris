`trust.toml` is read through a link at its name, where `pins.toml` and the registration cache refuse one.

- Area: cineris-policy
- Owner: step 32 (hardening pass)

| Item | Who | When |
|---|---|---|
| **What is open.** `TrustStore`'s read (`crates/cineris-policy/src/trust.rs`, `read`) looks at the file's size with `metadata` and reads it with `read_to_string`, both following a link at `trust.toml`'s name; the pins' store and the registration cache, in the same state directory, refuse a link there ([06-permissions-and-sandbox.md](../architecture/06-permissions-and-sandbox.md), "As built in step 28: the user's own pins"). Only a writer of the state directory can put a link there, and one can write the file itself, so this is an inconsistency between the stores, not a way past one. Found by the full review of step 28 (its pins area, lower confidence), and left for step 32 by the maintainers; the trust and approval stores are being changed by another session, which may take it with that change. **What would close it:** read `trust.toml` as `pins.toml` is read, opened with no link followed at its name (`cineris_base::nofollow::open_regular`) and its size taken from the handle, with a test of a link at the name and a break. | Step 32 (hardening pass) | Before the first release |
