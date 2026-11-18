Windows 11's tabbed Notepad may exit at once, so `Ctrl+G` removes the prompt's folder while it is open; a folder left behind is not tried again.

- Area: cineris-tui
- Owner: the owner, by hand; the folder half, step 32

| Item | Who | When |
|---|---|---|
| **A folder whose file the external editor still holds open, on Windows.** `Ctrl+G` runs the editor on a file in a private folder of its own and removes the folder when the editor ends; on Windows a folder whose file a process still holds open (a VS Code process that outlived the one `Ctrl+G` waited for, an editor's helper) cannot be removed, and is left in the user's temporary folder, private to the user. Whether to retry the removal later, or say where it was left, is open. (The other half of this item, whether Windows 11's Notepad waits, is answered: the coordinator ran the Store Notepad, 11.2609, from a test on the desktop's interactive session on 2026-11-15, and it waits until its window closes, with several tabs open too; [steps/tooling-5/answered.md](../steps/tooling-5/answered.md).) **The folder half was not built in step 23,** whose design had it: `Ctrl+G`'s folder is removed once, when the editor ends, and one that cannot be removed is left with nothing said. To build: a folder that cannot be removed tried again when the application ends, and one still there named on standard error after the terminal is left, tested on Linux with a folder whose file cannot be removed (its folder made read-only). | The owner, by hand, after step 23; the folder half, step 32 | Before the first release |
