# Answered in step 22, on 2026-11-14

Step 22 built the transcript's rendering in `cortex-tui`: the theme, the glyph sets and the rounded box (the foundation shared with step 21), streaming markdown with its commit rule, the views of every element, highlighting off the interface's thread, and the golden screens. The owner's answers came through the coordinator.

| Question | The owner's answer, 2026-11-14 | Recorded in |
|---|---|---|
| **A markdown parser of our own in place of `pulldown-cmark`.** The commit rule needs every piece of text's place in the source and the marks still open, which a whole-document parser does not give. | **Approved, on conditions:** it covers exactly the constructs 09-tui draws, and shows the rest as plain text, never dropped; its tests take cases written from CommonMark's examples for those constructs (the specification is not copied into the repository); the split-at-every-point property and a test of random input hold it, with no panic and no quadratic cost on long lines or deep nesting, and a large document measured; it goes through the full review; and it is recorded in 01-workspace, with `pulldown-cmark` taken out of the planned set. | [architecture/09-tui.md](../../architecture/09-tui.md), "The commit rule"; [architecture/01-workspace.md](../../architecture/01-workspace.md); [measurements/dependency-costs.md](../../measurements/dependency-costs.md) |
| **Where the real views and the reference screens differ.** A labelled bottom border was a column short in the reference screens; the live reasoning row showed its first words where 09 says the latest; `fast` was magenta at 16 colours on a dark background, where 09 says accent text is bold in the default colour there; the code line still streaming was coloured. | **All four are right as 09 says.** The rows are rewritten from the real views, and every changed cell is listed below. | [architecture/tui-look/README.md](../../architecture/tui-look/README.md) |
| **Step 21's rows in the golden screens,** once it merged first. | **Take them in:** the input box (screens 0 and 1) from `view::input` and the menus (3a, 3b) from `view::menu`, whose caller lays out a picker's notes. In 3a, `fast` inside a row's note is drawn muted, as the rest of the note, where the reference has the accent's tint: a note has one style, and 09 records it; list the cells. The hint row is to leave room for step 21's armed quit and the send mode. | [architecture/tui-look/README.md](../../architecture/tui-look/README.md); [architecture/09-tui.md](../../architecture/09-tui.md), the hint row's left side and "Structure of `cortex-tui`" |
| **The colours of the default theme that are under the text target** (red, green and yellow on a light background, muted text on a dark one, keywords in bright magenta). | Not yet: the coordinator is showing the owner. | [outstanding/default-colours-under-the-text-target.md](../../outstanding/default-colours-under-the-text-target.md) |

## The reference cells step 22 changed

`cargo xtask tui-screens --write` rewrote these rows from the real views: 630 cells in 28 rows of 24 ANSI files, and the same rows of the text copies of 7 screens (3a's text did not change). A row is counted from 0, and so is a column. The last column shows one cell of the row: the reference's, then the views'.

| Screen | Background | Row | Cells | Columns | Why | One cell |
|---|---|---|---|---|---|---|
| 1 | dark | 34 | 30 | 10–50 | the code line still streaming, plain until it ends | `"r" {fg=a13}` → `"r" {}` |
| 1 | light | 34 | 30 | 10–50 | the code line still streaming, plain until it ends | `"r" {fg=a5}` → `"r" {}` |
| 1b | dark | 5 | 69 | 3–79 | the live reasoning row: its latest words | `"l" {fg=a8 i}` → `"a" {fg=a8 i}` |
| 1b | light | 5 | 69 | 3–79 | the live reasoning row: its latest words | `"l" {fg=a8 i}` → `"a" {fg=a8 i}` |
| 1c | dark | 23 | 20 | 78–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=a8}` |
| 1c | light | 23 | 20 | 78–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=a8}` |
| 1d | dark | 15 | 19 | 80–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=a8}` |
| 1d | light | 15 | 19 | 80–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=a8}` |
| 2a | dark | 2 | 19 | 80–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=a8}` |
| 2a | light | 2 | 19 | 80–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=a8}` |
| 4 | dark | 30 | 19 | 80–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=a8}` |
| 4 | light | 30 | 19 | 80–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=a8}` |
| 6 | dark | 36 | 30 | 10–50 | the code line still streaming, plain until it ends | `"r" {fg=a13}` → `"r" {}` |
| 6 | light | 36 | 30 | 10–50 | the code line still streaming, plain until it ends | `"r" {fg=a5}` → `"r" {}` |
| 7a | dark | 34 | 30 | 10–50 | the code line still streaming, plain until it ends | `"r" {fg=a13}` → `"r" {}` |
| 7a | light | 34 | 30 | 10–50 | the code line still streaming, plain until it ends | `"r" {fg=a5}` → `"r" {}` |
| 7a | dark | 39 | 4 | 93–96 | `fast` at 16 colours on a dark background: bold in the default colour | `"f" {fg=a13}` → `"f" {b}` |
| 7c | dark | 34 | 30 | 10–50 | the code line still streaming, plain until it ends | `"r" {fg=a13}` → `"r" {}` |
| 7c | light | 34 | 30 | 10–50 | the code line still streaming, plain until it ends | `"r" {fg=a5}` → `"r" {}` |
| 7c | dark | 39 | 4 | 93–96 | `fast` at 16 colours on a dark background: bold in the default colour | `"f" {fg=a13}` → `"f" {b}` |
| 8a | dark | 24 | 18 | 81–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=#7A3CFB}` |
| 8a | light | 24 | 18 | 81–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=#7A3CFB}` |
| 8b | dark | 36 | 19 | 79–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=#7A3CFB}` |
| 8b | light | 36 | 19 | 79–98 | a labelled bottom border, drawn as wide as the box | `" " {fg=a8}` → `"─" {fg=#7A3CFB}` |
| 3a | dark | 5 | 4 | 51–54 | `fast` in the `/model` picker's note: muted, as the rest of the note | `"f" {fg=#8D57FC}` → `"f" {fg=a8}` |
| 3a | dark | 8 | 4 | 51–54 | `fast` in the `/model` picker's note: muted, as the rest of the note | `"f" {fg=#8D57FC}` → `"f" {fg=a8}` |
| 3a | light | 5 | 4 | 51–54 | `fast` in the `/model` picker's note: muted, as the rest of the note | `"f" {fg=#7A3CFB}` → `"f" {fg=a8}` |
| 3a | light | 8 | 4 | 51–54 | `fast` in the `/model` picker's note: muted, as the rest of the note | `"f" {fg=#7A3CFB}` → `"f" {fg=a8}` |
