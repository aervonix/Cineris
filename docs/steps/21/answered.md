# Answered in step 21, on 2026-11-14

Step 21 built the editor in `cortex-tui`: the text model, the key map and its actions, history, paste, the completion menus and pickers, the `Ctrl+P` focus ring and the external editor.

| Question | The owner's answer, 2026-11-14 | Recorded in |
|---|---|---|
| **`Ctrl+Z`: undo or suspend?** 09-tui's key table made `Ctrl+Z` undo and `Ctrl+Shift+Z` redo, and its terminal-layer section said `Ctrl+Z` suspends on Linux, which step 20 built (`Terminal::suspend`). One key cannot do both. And plain virtual-terminal input (xterm without its extended key reporting, VTE terminals, tmux) sends `Ctrl+Shift+Z` as the same byte as `Ctrl+Z`, so there redo could not be reached at all. | **`Ctrl+Z` is undo**, with `Ctrl+_` as a second undo key (what `Ctrl+/` and `Ctrl+-` send, as readline has it; the enhanced keyboard protocol reports the key as `Ctrl+/`, so both spellings are bound). **`Alt+Z` is a second redo key**, which every terminal sends as itself. **Suspending is an action, `global.suspend`, with no key by default:** whoever wants `Ctrl+Z` to suspend binds it in `[keys]`. Asked through the coordinator; the owner chose the step's recommendation. | [architecture/09-tui.md](../../architecture/09-tui.md), "Default keys" and "Terminal layer" |
| **The key that expands a paste chip in place.** 09-tui said "a key expands it in place" and named none. | **`Ctrl+X`.** Nothing in 09-tui or the later steps' sections planned `Ctrl+X`. Approved through the coordinator. | [architecture/09-tui.md](../../architecture/09-tui.md), "Default keys" |
