In the full screen, the transcript cannot be scrolled by its keys while a dialog is up.

- Area: cineris-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **No scrolling by key under a dialog.** While a dialog is up its keys come first, and the full screen's own (`PageUp`, `PageDown`, `Shift+Up`, `Shift+Down`) go to it: before the dialog is ready they count as typing and hold it another second; after, they do nothing. Only the wheel scrolls. `Ctrl+O` is no way round: under an approval or the trust question it shows the request alone, with no kept block, and under a question it does nothing. So without a mouse there is no way back to what led to the dialog. Found by the review of step 25, made precise by its second review. Recommendation: let the full screen's scrolling keys through when the dialog does not use them, as moves that are not typing (the readiness of step 23 unchanged). | Step 33 | Before the first release |
