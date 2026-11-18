The debug prefix check skips requests holding a stored image reference; a protocol should write a placeholder.

- Area: cineris-llm
- Owner: step 33, or the step that next touches images

| Item | Who | When |
|---|---|---|
| **The debug build's prefix check skips a request that holds a stored image.** The check asks the client for the request's stable prefix (`ModelClient::stable_prefix`), and a request with an image that is still a reference to an artifact cannot be built until the client loads it ([04-providers.md](../architecture/04-providers.md), "Prompt caching", as built in step 15). A request with other settings than the one before it as it was sent (a raised output limit, or the standard speed the client sent in place of a tier during a cooldown, which the stream reports) also starts the comparison again: since the fixes after the review of step 15 both comparisons are made at the speed the client sent, not the speed asked for (`InFlight` in `cineris-agent`'s `context::prefix`). The context record's comparison of what was assembled still covers both. To settle: let a protocol write an image reference as a fixed placeholder in the stable prefix. | Step 33 (performance), or the step that next touches images | When it is next convenient |
