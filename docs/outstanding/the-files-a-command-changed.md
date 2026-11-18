The review panel lists the files the file tools changed; which files a shell command changed is in the checkpoints, not in any event.

- Area: cineris-protocol
- Owner: none yet

| Item | Who | When |
|---|---|---|
| **A command's changes are not listed by name.** Step 24's review panel ([steps/24/answered.md](../steps/24/answered.md), point 9, departure 4) lists the files `edit` and `write` changed, with their counts, and says when a command ran that may have changed others. The checkpoints know which files a command changed (the base capture and the turn's end), but no event or result carries the list to a client. **Recommendation:** a field of `turn.completed`, the paths a turn's commands changed (from the checkpoint the turn's end takes), which the review panel lists beside the file tools' with no counts; capped as the panel's rows are. | A step that touches the checkpoints and the protocol | When the review panel is to be complete |
