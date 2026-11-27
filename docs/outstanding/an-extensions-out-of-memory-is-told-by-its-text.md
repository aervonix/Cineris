The script engine tells that a runtime ran out of memory from the text of the error, not from its allocator.

- Area: cineris-ext
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **Out of memory is told by its text.** A failure whose text holds "out of memory" counts as the runtime's memory limit (`engine::thread`, `classify_text`), and tears the runtime down; the engine's own out-of-memory error is the one the allocator's refusal raises. So a script that catches the real one and goes on keeps its runtime, near its limit, where the page says a limit tears it down, and a script that throws an error of that text tears its own runtime down, which harms nothing but itself. No safety impact was found (the review of step 28's close). **Recommendation:** have the runtime's allocator record that it refused an allocation (`rquickjs`'s runtime with an allocator of the harness's own, holding the limit), and tear the runtime down on that record, not on the text. | Step 32 (hardening pass) | Before the first release |
