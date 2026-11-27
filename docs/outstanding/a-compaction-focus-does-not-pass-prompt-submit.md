The focus of `/compact <focus>` reaches the summary request without passing the `prompt.submit` hooks.

- Area: cineris-agent
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **What is open.** A prompt, a steering message, a queued prompt and the next turn's leftover steers each pass `prompt.submit` before the model sees them, but the text after `/compact` is a manual compaction's focus (`Prompt::compact`), which `Session::begin` (`crates/cineris-agent/src/turn.rs`) sends into the summary request with the `prompt.submit` hooks skipped. The `compaction.before` hooks are given the focus (`HookSubject::Compaction`), but they only add instructions and cannot refuse it, so a `prompt.submit` gate that refuses a text does not refuse it as a focus. Found by the review of step 28 (hooks, lower confidence), and left by the maintainers. **What would close it:** run a manual compaction's focus through the `prompt.submit` hooks as a prompt is (a refusal gives it back and runs no compaction), or say in 12-extensions that a gate does not see a focus and why. | Step 32 | Before the first release |
