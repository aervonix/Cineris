A file a call was allowed to read or write is opened by its path after the decision, and a folder above it can become a link in between. Security.

- Area: cineris-tools
- Owner: step 32 (hardening pass)

| Item | Who | When |
|---|---|---|
| **A file is opened by name after its decision.** The built-in `read` and `write` tools, and an extension's `cineris.fs.read` and `cineris.fs.write`, resolve a path, are decided on where it leads (which may wait on a person), and then open it by name. `open_regular` refuses a link in the last name only, not in the folders above it. **What a swap needs today:** something running beside the pending decision that can place a link in the workspace: a background command the model left running, or a program an extension started that outlived its `exec`. Since step 28, an extension's read resolves its path again after the decision and refuses one that moved, and a write or a walk acts only on the path the manifest judged, which narrows the window to the open itself; the built-in tools have no such check. Found by the early review of step 28's permission path. **Recommendation:** open beneath the workspace's root rather than by name: `openat2` with `RESOLVE_BENEATH` and `RESOLVE_NO_SYMLINKS` on Linux, and on Windows each component opened from its parent's handle with `FILE_FLAG_OPEN_REPARSE_POINT`, a reparse point refused; then compare the opened handle with the decision's path, for the built-in tools and the extensions' alike. | Step 32 (hardening pass) | Before the first release |
