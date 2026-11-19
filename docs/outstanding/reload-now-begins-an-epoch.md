`/reload --now`, which would begin a new context epoch at once so that the keys waiting for one take effect, is not built.

- Area: cineris-core
- Owner: a later step that adds an epoch on request to the loop

| Item | Who | When |
|---|---|---|
| **`/reload --now`.** [08-configuration.md](../architecture/08-configuration.md) has a reload's `next epoch` keys wait for the session's next epoch, "and `/reload --now` starts a new epoch at once". Step 24 builds the wait: the keys are taken when a compaction begins, or when the session is opened again ([steps/24/answered.md](../steps/24/answered.md), the maintainers' answer of 2026-11-16). The loop has no way to begin an epoch on request that builds the prompt again: a change of model begins one that keeps the prompt by design. **Recommended:** a `config/reload` field `now` that has the loop begin a `resume`-reason epoch at its next safe boundary, after taking what waits; it costs the provider's cache once, which the notice says. Until then the person compacts (`/compact`) or opens the session again. | A step that adds an epoch on request to the loop | Before the first release, if the owner wants it |
