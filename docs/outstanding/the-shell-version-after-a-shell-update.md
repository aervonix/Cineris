The first session after a shell is installed or updated has no shell version in its prompt, so the next resume is a `resume` epoch.

- Area: cineris-core
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **The first session after a shell is installed or updated has no shell version.** The version in the system prompt's section 2 and the shell tool's description comes from a cache that is filled after a miss (`cineris-core`'s `shell_cache`), so the first process after a change says `bash` with no version, and the next says the version. The prompt then differs from the stored one, so a resume after it is a `resume` epoch: new tools, a new `base`, and every reasoning block of the history dropped ([07-sessions-and-context.md](../architecture/07-sessions-and-context.md), "Context epochs"). That is by design for a real change of the prompt, and here it follows any shell update. Found by the review of step 15 (its seventh note), not changed in its fixes. To settle: ask the version before the first session's prompt is built (its cost at start-up is step 33's to weigh), or leave the version out of `base` and tell a change of it as a notice, as the instruction files are. | Step 33 (performance) | When the start-up path is measured |
