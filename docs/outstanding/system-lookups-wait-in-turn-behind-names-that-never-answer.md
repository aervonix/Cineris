A fetch's name lookup waits for one of the process's eight places, so lookups of names that never answer, from one client, delay every other client's.

- Area: cineris-llm
- Owner: step 30

| Item | Who | When |
|---|---|---|
| **Lookups wait in turn behind names that never answer.** Since the review of step 28's close the system's resolver (`cineris_llm::SystemResolver`) holds at most `LOOKUPS_LIMIT` (8) lookups at once in the process, each place given back when the blocking lookup returns, so that fetches given up at once cannot fill the blocking pool. The cost: a lookup of a name that never answers holds its place for the system resolver's own timeout (tens of seconds), and every place is the process's. N runtimes (in one session or many) that each keep one such lookup queued can make another client's lookup wait about N/8 of those timeouts, and a fetch waits within its own time, so it fails as out of time. Step 30's HTTP MCP client will look its servers up through this resolver and share the places with every extension. The provider transports look their hosts up through their HTTP client's own resolver, and do not wait here. **Recommendation:** with step 30, give each client (an extension, an MCP server) a share of the places, or one place in flight per client, so that one client's names cannot hold another's lookups; and say in a fetch's error when it ran out of time waiting for a place. | Step 30 (MCP client) | Before the MCP client fetches through this resolver |
