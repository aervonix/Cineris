`/copy` cannot tell whether the terminal took OSC 52, and shows no text for copying by hand where it is known not to.

- Area: cineris-tui
- Owner: step 25

| Item | Who | When |
|---|---|---|
| **`/copy` has no fallback where the terminal does not take the clipboard sequence.** `/copy` writes the last answer in OSC 52, which a terminal does not answer, so whether the clipboard has it cannot be told; since the review of step 23 it says it sent the text, and that a terminal that does not take the sequence ignores it. 09-tui's capability table had promised that `/copy` "reports when it cannot tell" and that "the text is shown for manual copying"; neither was built, and the first cannot be, so the table now says what is built ([architecture/09-tui.md](../architecture/09-tui.md), "Terminal layer"). Where a terminal is known not to take the sequence (the Windows console without virtual-terminal processing, `TERM=dumb`, tmux without `set-clipboard`), the text could be shown instead, in the viewer or the transcript, for copying by hand, with a `[terminal]` setting to say the clipboard sequence is not taken. To settle: decide which terminals are known not to take it, add the setting, and show the text there. | Step 25 (full screen and session UI), which owns the viewer the text would be shown in | With step 25, or before the first release |
