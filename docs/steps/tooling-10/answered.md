# Answered in the tenth tooling step, on 2026-11-16

This step is not a numbered step of the build plan. It built the tenth round of developer tooling, from what wave 12's later sessions asked for: step 17, the re-decide step, the step 23 fixes, step 19 and the ninth tooling step. Its records are here, in a folder named for the job. It is cut into pull requests, and each part's record is linked below.

## The owner's answers, through the coordinator

- **`ci-wait` stays the owner's script.**
  This step does not change it, and writes no patch for it.
  It sent the coordinator a specification of the two changes asked for: `--pr N` following the pull request's head after a force-push, and a warning for a run queued with no jobs past a limit.
  The coordinator folded them into the owner's next patch, where `--queued-limit` is another name for `--queued-warn` (10 minutes).
  This step's `try --dispatch` and `try-result` use the same rule, words and default.
- **The cut:** item 1 first, as a pull request of its own, since steps 24 and 27 add open items alongside it; then the known-flake list; then the rest.
- **The open items' index is merged line by line** (`merge=union`), with `rules` catching a merge that is wrong and `cargo xtask outstanding` putting it right. Approved, with one condition: a merge that drops a line must be caught, not only one that duplicates a line.
- **The leader-close test is not marked flaky:** step 23's third pull request fixed its cause and settled its item.

## Records that do not conflict (item 1, the first pull request)

One sentence a line, in the style the ninth tooling step set ([CONTRIBUTING.md](../../../CONTRIBUTING.md), "Writing the docs").

- **`cargo xtask outstanding [--check]`** writes `docs/outstanding/README.md` from the items' files:
  Each item's file now begins with its sentence, the text of its line in the index, then its fields as a list: `- Area:` (a crate of `crates/`, or `tooling`, `windows`, `other`) and `- Owner:` (who settles it, in the few words the index gives), then the table as before.
  The index is sorted: the crates by name, then the named areas in that order, and the items of an area by name.
  The whole page is the command's, its preamble included, so it can be written over a page full of conflict markers.
  The 124 items were converted by a script from the index as it was: each sentence, owner and area is what the index said, and the index the command writes has the same lines as before, in sorted order, with the stray blank line under `cortex-session` gone.
  `- Flaky:` is read, and checked, by the same parser (item 2 uses it).
- **`rules` and `links` compare the whole index with what the command writes** (`xtask/src/rules/layout.rs`, `stale_index`), in place of the fifth tooling step's check that each item was listed once.
  The message names the lines the page lacks, the lines it has and should not, and the lines it has twice, or says the order is wrong, and ends with the command to run.
  An item whose file cannot be read is reported at its line, and the index is not judged until it can be: one problem at a time.
  The rules read what git tracks, and the command reads the folder, so a new item git has not been told of shows as a line the page "has and should not", with `git add` named in the message.
- **A rebase never stops on the index:** `.gitattributes` gives it `merge=union`, which keeps both sides' lines where they conflict.
  Whatever such a merge gets wrong (a line twice, the line of an item whose file is gone, a line lost, two lines out of order) the comparison above finds, since it is of the whole page; each of the four is a case of `rules::layout::tests::an_index_that_is_not_what_the_items_give_is_found`.
  **After a rebase, the one command is `cargo xtask outstanding`**, then commit the index it wrote; `regen` runs it first among its generators.
  Tried by hand on a scratch repository with this tree's index and attributes: one branch adds an item under `cortex-tools`, another adds one in the same place and settles a third; the rebase of the second onto the first went through, with both new lines kept and the settled one gone.
- **A step in several pull requests:** each part writes its own record, `docs/steps/<step>/<group>/<part>.md` (`23/after-review/pr2-trust.md`), which begins with a heading of its own and is linked from the step's `answered.md` or `ci.md`.
  `rules` checks the names (hyphenated, one folder deep), the heading, and the link (a link in fenced code or a comment does not count).
  Documented in [CONTRIBUTING.md](../../../CONTRIBUTING.md) ("Writing the docs"), [docs/steps/README.md](../README.md), rule 14 of the build plan and of `AGENTS.md`.
- **The other shared lists, looked at:**
  `mutations/` is one file per step or part already, with no index, so two steps never touch one file: left.
  The build plan's tables are the owner's decisions, written once and amended by hand, and its "Still outstanding" is a link to the index: left, with a sentence there naming the command.
  `performance-budgets.md`'s rows are budgets the owner sets, with measurements beside them; nothing generates a budget, and a row changes rarely: left.
  `docs/decisions/README.md`'s rows are added by `cargo xtask decision new`; two steps adding a decision at once would also take the same number, which no generated index settles, and the status column is prose the records do not hold in one form: left.
  The "As built" records were turned into lists by the ninth tooling step, which puts two sessions' sentences on different lines.
- **Measured** (the laptop, a debug build of `xtask`): `cargo xtask links` 0.18 to 0.29 s over five runs, and `outstanding --check` 0.01 s, which is the whole cost of reading the 124 items and writing the index.
- **Breaks:** `mutations/tooling-10.breaks`, the `outstanding-*` and `part-record-*` breaks. Two breaks of the fifth tooling step's index check (`the-index-check-passes-an-unlisted-item`, `a-link-in-fenced-code-lists-an-item`) were of code that is gone, and are deleted: what they held is held by the new breaks.
