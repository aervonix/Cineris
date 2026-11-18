A bracketed paste with no end marker for 2 seconds is split, and the remainder arrives as keys.

- Area: cineris-term
- Owner: step 25

| Item | Who | When |
|---|---|---|
| **A paste that stalls for more than 2 seconds is split, and the rest arrives as keys.** Since step 21 a bracketed paste whose end marker has not come within 2 seconds of its last input is ended there and given as a paste, so that a lost end marker does not swallow keys, `Ctrl+C` among them ([steps/21/answered.md](../steps/21/answered.md)). If the paste was only slow (a slow connection pausing in the middle of it), what follows of it arrives as keys, and a line break in it is `Enter`, which sends the prompt. The end marker that comes at the last is dropped. **The review of step 23 found it worse,** since the first key of the rest could answer a dialog; its fixes close that half: a paste that reaches the editor while a dialog is up is input, and the dialog waits for a second with none before it takes a key, so the rest, which follows at once as keys, goes to the editor too. Recommendation: for a short time after a paste was given up on, read `Enter` as a new line rather than send, and append the text that follows to the paste until its end marker comes or a further pause passes; beside the rate-detected pastes, which are the same problem from the other side. | Step 25 (terminal quirks, with the rate-detected pastes of [pastes-without-bracketed-paste.md](pastes-without-bracketed-paste.md)) | Before the first release |
