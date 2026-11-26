# Step 25 fixes: the review's findings left open

The independent review of step 25 (at `77ffbbcc`) ranked 23 findings, a list of rules no test or break holds, notes of lower confidence, the open items it touched, the docs that disagree with the code, and what later steps should know. The step 25 fixes settle findings 1 to 7, 10 to 12 and 17 in their other parts. This part puts the rest where a later step finds them: an open item for each, or lines added to an item that already covers the same thing, each with the review's file and line, its scenario, how it was confirmed and its suggested fix. The review's own tests are scratch tests kept with the review, outside the repository; each item names the one it rests on. Nothing here changes code but two comments.

The owners follow the review's section 5: step 32 (the hardening pass) for hardening and correctness, step 33 (the performance pass) for performance and the terminal, step 30 (the MCP client) for what `[mcp]` settles, and, for the Windows Terminal checklist, which the review gives to step 34, the item the owner runs it under, which keeps its owner. The index of the open items is written by the session from the files below.

## Each finding and where it went

| Finding | Item | Owner |
|---|---|---|
| 8. A rewind `before` a steering message that began its own turn | [a-rewind-before-a-steering-message-that-began-its-turn](../../../outstanding/a-rewind-before-a-steering-message-that-began-its-turn.md) (new) | Step 32 |
| 9. A rewind or undo not told to the session's other clients | [a-rewind-not-told-to-the-sessions-other-clients](../../../outstanding/a-rewind-not-told-to-the-sessions-other-clients.md) (new) | Step 32 |
| 13. A change of width lays out the whole block and the open answer at each frame | [a-change-of-width-lays-out-whole-blocks](../../../outstanding/a-change-of-width-lays-out-whole-blocks.md) (new), with the review's measurements; linked from [a-full-screen-frame-copies-the-open-answer](../../../outstanding/a-full-screen-frame-copies-the-open-answer.md) | Step 33 |
| 14. The key's input drops a key held down | [the-keys-input-drops-a-key-held-down](../../../outstanding/the-keys-input-drops-a-key-held-down.md) (new); the held `Backspace` row added to [the-windows-terminal-check-of-step-25](../../../outstanding/the-windows-terminal-check-of-step-25.md) | Step 33; the row, the owner's Windows Terminal check |
| 15. A stalled file read on the set-up worker holds the configuration's write | [a-stalled-read-holds-the-first-runs-write](../../../outstanding/a-stalled-read-holds-the-first-runs-write.md) (new) | Step 32 |
| 16. A stalled paste's end marker cut across two reads | [a-pastes-end-marker-cut-across-reads](../../../outstanding/a-pastes-end-marker-cut-across-reads.md) (new) | Step 33 |
| 18. The viewer reads a compaction's summary from the session open now | [the-viewer-reads-a-compaction-summary-from-the-open-session](../../../outstanding/the-viewer-reads-a-compaction-summary-from-the-open-session.md) (new) | Step 32 |
| 19. A dialog that waited behind the one read does not follow the end | [a-dialog-that-waited-behind-the-one-read-does-not-follow-the-end](../../../outstanding/a-dialog-that-waited-behind-the-one-read-does-not-follow-the-end.md) (new) | Step 33 |
| 20. A search answered under a dialog never reveals its match | [a-search-answered-under-a-dialog-never-reveals-its-match](../../../outstanding/a-search-answered-under-a-dialog-never-reveals-its-match.md) (new) | Step 33 |
| 21. Re-wrapping keeps the row's number, not the place read | [a-resize-keeps-the-row-number-not-the-place-read](../../../outstanding/a-resize-keeps-the-row-number-not-the-place-read.md) (new); linked from [a-view-in-the-live-rows-moves](../../../outstanding/a-view-in-the-live-rows-moves.md) | Step 33 |
| 22. The viewer's `/` query has no bound | [the-viewers-search-query-has-no-bound](../../../outstanding/the-viewers-search-query-has-no-bound.md) (new) | Step 33 |
| 23. A `${VAR}` reference under a credential's name refuses its server | [a-variable-reference-under-a-credentials-name-refuses-its-server](../../../outstanding/a-variable-reference-under-a-credentials-name-refuses-its-server.md) (new) | Step 30 |
| Rules no test or break holds | [rules-of-step-25-that-no-test-or-break-holds](../../../outstanding/rules-of-step-25-that-no-test-or-break-holds.md) (new): the two click guards, four rules with a test and no break, three with no test. Not in it: the emergency restore's mouse-off entries, which these fixes mend, and the key typed ahead of `auth login`'s row, which these fixes' test of finding 1 holds | Step 32 |

## The notes of lower confidence, and section 5

| Note | Item | Owner |
|---|---|---|
| Imports and the first run: a program-naming argument masked; an edited command's old description; `CINERIS_CONFIG`'s edges; the writer's leftovers. With them, section 5's check of environment and header names before a reference is resolved | [the-first-runs-import-and-writer-corners](../../../outstanding/the-first-runs-import-and-writer-corners.md) (new) | Step 30 for the import's names; step 32 for the rest |
| `/export`: halves of a key joined in the Markdown; and the two cosmetic notes (the count counts typed `[redacted]`, `~` not expanded) | [what-export-can-join-and-what-it-counts](../../../outstanding/what-export-can-join-and-what-it-counts.md) (new) | Step 32 |
| The terminal: `CSI M` read where no report was asked for; the paste-burst thresholds; with section 5's `START` turning off a crashed program's mouse modes and the emergency handler's alarm armed after its write | [terminal-corners-the-review-of-step-25-found](../../../outstanding/terminal-corners-the-review-of-step-25-found.md) (new) | Step 33; the emergency handler, step 32 |
| The terminal: the full screen asks for the mouse before the win32 input mode's reply | Added to [the-mouse-under-windows-terminal](../../../outstanding/the-mouse-under-windows-terminal.md) | Step 33 (unchanged) |
| The full screen: six small corners, and two docs that say the mouse holds or readies a dialog | [full-screen-corners-the-review-of-step-25-found](../../../outstanding/full-screen-corners-the-review-of-step-25-found.md) (new) | Step 33 |
| Rewind, undo and fork: how far a hand-edited log is trusted, one `../x` refusing every undo, `get_blob` unbounded under the lock, a fork under a running call, a prompt's other parts dropped | [rewind-corners-the-review-of-step-25-found](../../../outstanding/rewind-corners-the-review-of-step-25-found.md) (new) | Step 32 |
| Rewind: the plan and the rewind hash every undone file under the log's lock | Added to [hashing-large-files-at-a-rounds-end](../../../outstanding/hashing-large-files-at-a-rounds-end.md) | Step 33 (unchanged) |
| Section 5: the filter masks AWS's key id and not the secret key beside it | [an-aws-secret-key-beside-its-id-is-not-masked](../../../outstanding/an-aws-secret-key-beside-its-id-is-not-masked.md) (new) | Step 32 |
| Section 5: measure a resize storm and a first scroll into a large block | In [a-change-of-width-lays-out-whole-blocks](../../../outstanding/a-change-of-width-lays-out-whole-blocks.md) | Step 33 |

Section 5's other points are the session's own findings (1, 2, 4, 6, 7, 10, 11, 12), or were placed above.

## The known items the review touched

- **`a-terminals-reply-typed-into-the-input-box-on-a-busy-windows-machine`** was settled after the review's commit by the second small fixes ([answered.md](../../small-fixes-2/answered.md), "A terminal's reply typed into the input box"), which deleted its file. Read against the code as it is now: while the replies are due, a lone ESC is held until the device attributes come, so a reply split after its ESC no longer goes into the key at the first run or cancels `auth login`, and both have a test; the fifth query (`?2004$p`) is covered by the same hold, its reply being sent before the device attributes and one of the forms read without its ESC. What is left is the hold's bound, 3 s after the queries: a reply later than that still goes into the hidden key, to be stored with it, and still cancels `auth login`, whose `Esc` has no grace. It is a new item, [a-late-reply-typed-into-the-key](../../../outstanding/a-late-reply-typed-into-the-key.md) (step 32), which links the second small fixes' record and [input-decoder-corners](../../../outstanding/input-decoder-corners.md) for the rest of the decoder's corners.
- **[mouse-reports-in-flight-when-the-mouse-goes-off](../../../outstanding/mouse-reports-in-flight-when-the-mouse-goes-off.md)**, slightly worse: a report split after its ESC is `dialog.deny` while the wheel turns, the safe direction; the `?2004` reply at the start is held since the second small fixes.
- **[the-mouse-under-windows-terminal](../../../outstanding/the-mouse-under-windows-terminal.md)**: the first frame's window, above.
- **[the-search-counts-text-but-marks-drawn-rows](../../../outstanding/the-search-counts-text-but-marks-drawn-rows.md)**: text drawn escaped (a hostile `\u{202e}`) is one more case.
- **[a-full-screen-frame-copies-the-open-answer](../../../outstanding/a-full-screen-frame-copies-the-open-answer.md)** and **[a-view-in-the-live-rows-moves](../../../outstanding/a-view-in-the-live-rows-moves.md)**: each links the new item of the different cost or mechanism (findings 13 and 21).
- **[the-windows-terminal-check-of-step-25](../../../outstanding/the-windows-terminal-check-of-step-25.md)**: the held `Backspace` row (finding 14), and that finding 1 cannot show in Windows' console.
- **`the-search-worker-keeps-a-copy-of-the-transcript`** is changed by another part of these fixes, with finding 12, and is left alone here.
- **Left as they are,** the review finding them as described: `input-decoder-corners`, `a-kept-box-holds-only-the-end-of-its-output` (what the review says it leaves out, the reasoning, is finding 12's bound, which these fixes make), `rewind-and-fork-have-no-test-of-the-real-binary`, `a-commands-text-in-the-rewind-picker-and-the-export`, `an-undo-after-an-incomplete-look`, `ambiguous-width-glyphs` and `the-default-editor-on-windows-11`.

## The docs that disagree with the code

**Corrected, where the code is right:**

- [07-sessions-and-context.md](../../../architecture/07-sessions-and-context.md), "Secrets in sessions": it named `cineris sessions export`, which does not exist, and said it "shows what it masked". It now names `/export`, says what it masks and that it says one count, and links 09's "Exporting a session".
- [08-configuration.md](../../../architecture/08-configuration.md), "Credentials", what is not wiped: it now names the file store's own copies of every key it stores (the file's text, the table read from it, the text written back) each time it reads or rewrites `credentials.toml`.
- `crates/cineris-term/src/caps.rs`, `Capabilities::QUERIES`'s comment: it now names the query of bracketed paste (DECRQM 2004), sent since step 25.
- `crates/cineris-tui/src/app/setup.rs`, `NotWritten::Found`'s comment: a file found at the name is read at once, with what was chosen applying over it for the run, not "the configuration from the next start".

**Left, where the doc is the specification and the code differs; each item says so:** 09's "as the key's input does" of a key held down (the key's item, finding 14); `before`'s docs in `cineris-protocol`'s `methods.rs`, 11-rpc-protocol, 09 "The points" and `rewind.rs`'s module doc (finding 8's item); 09's paragraph on reading above a dialog, where the wheel "holds the dialog as typing does", and step 25's design, point 3, where a click counts for readiness (the full screen's corners, which recommend that the maintainers settle the code's rule and 09 state it); step 25's design, point 2, "the nearest row inside it" (finding 21's item); and the comment at `app/state/screen.rs` that promises a reveal (finding 20's item). Step 25's own record is not edited.

**Not changed here:** the same stale "from the next start" in a test's comment in `crates/cineris-cli/src/setup.rs` (`a_file_found_at_the_name_is_not_a_failure`, line 515; the review had lines 385-387), a file another part of these fixes changes for finding 6; the session's to word with it.

## Checks

`cargo xtask links` and `cargo xtask check` were run on this part's branch. No tests and no breaks: the part is documents and two comments. The index of open items is not written here.
