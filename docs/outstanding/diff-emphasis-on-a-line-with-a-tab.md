A diff's line with a tab or an escaped character loses the bold of its changed words.

- Area: cortex-agent
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **A diff's line that holds a tab, or a character `sanitize` escapes, is drawn without its bold.** The round's view (`crates/cortex-tui/src/view/round.rs`, `emphasised`, step 22) draws a line's changed words bold only when the text it shows is the text the ranges were found in; expanding tabs or escaping a character moves the bytes, and the view then leaves the bold out rather than put it in the wrong place. So a Go file's or a Makefile's changed words are never bold. Found by the review of step 23's diff rule (the owner's, 2026-11-16: whole changed words are bold). **To do:** carry the ranges through the tab expansion and the escaping, which both change the text a character at a time, and test a tab-indented line. | Step 32 (the hardening pass), or a step that next touches the round's view | Before the first release |
