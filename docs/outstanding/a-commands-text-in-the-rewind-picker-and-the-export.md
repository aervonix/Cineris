The interface's rewind picker and `/export` show a custom command's prompt as the text it sent, under the user's own heading, not as the command the user typed.

- Area: cineris-tui
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **What is open.** Since step 28 a prompt that is a command's text is recorded with the command it came from (`UserMessageItem.command`), and the transcript shows it under a line that names the command and whose it is; a prompt given back, by a hook's refusal, an interrupt or a rewind, comes back as `/name args`. Two places still show the expanded text with no word of the command, found by the early review and the re-check of step 28's custom commands: **the rewind picker's rows**, and **`/export`**, which writes it under `## You`. A project's command's text then reads as the user's own there. **What would close it:** both read `UserMessageItem.command`, the picker showing `/name args` with whose command it was, and the export a line naming the command above its text. | Step 32 | Before the first release |
