| Item | Who | When |
|---|---|---|
| **The map's share of the context record.** The per-request context record (`StatEvent::Context`) counts tokens by source: system prompt, tools, summary, user, assistant, tool results, notices. The repository map is a notice (step 18), so its 1,500 tokens are counted among the notices, and the context profiler cannot say what the map costs an epoch, or suggest a smaller budget. A source of its own (`map`) is an addition to `ContextSources`, which step 16 stores. | Step 44, or step 16 if it is still open | When the profiler's categories are settled |
