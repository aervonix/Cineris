| Item | Who | When |
|---|---|---|
| **`/export` is in no step.** [09-tui.md](../architecture/09-tui.md)'s commands table has `/export`, which "writes the session as Markdown or JSON, redacted", and no step of the build plan builds it. Step 23's application says it is not in this build (`NOT_YET`). **Recommendation:** step 25, whose session interface (rewind, undo and fork) is its neighbour; it needs the session's log read whole, which `session/read` gives, and the same masking the log applies when it is written. | The owner, to place in a step | Before the first release |
