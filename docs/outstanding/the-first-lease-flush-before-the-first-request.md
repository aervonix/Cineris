A session's first lease record is flushed to disk before its first request, whatever the durability.

- Area: cortex-session
- Owner: the next step that changes leases, or step 32

| Item | Who | When |
|---|---|---|
| **The first lease record's flush, before a session's first request.** When a session is born (its first user message written), its first entry is the lease it was granted, appended with `append_flushed`: "a lease is flushed whatever the durability", so that a crash never brings back a lease that had ended or been narrowed. The first grant is flushed too, before the first request is sent, even with `session.durability = "never"`. It costs 2.8 ms on a hosted Windows runner and 10 ms on the desktop's virtual disk, on every new session's first prompt (the fifth tooling step's probes; [performance-budgets.md](../performance-budgets.md), "Open a session, on Windows against Linux"). The question: does the first grant need to be on the disk before the first request goes, or only before anything acts under it (a tool call), or is it enough that a later narrowing is flushed? A lost first grant is recomputed from the configuration on resume, which is the same lease unless the settings that started the session asked for a narrower one. It is behaviour around an enforced value, so it was measured and not changed. | The next step that changes leases, or step 32 | Before the first release |
