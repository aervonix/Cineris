# Answered in the tenth tooling step, on 2026-11-16

This step is not a numbered step of the build plan. It built the tenth round of developer tooling, from what wave 12's later sessions asked for: step 17, the re-decide step, the step 23 fixes, step 19 and the ninth tooling step. Its records are here, in a folder named for the job. It is cut into pull requests, each with a section below.

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
  The 124 items were converted by a script from the index as it was: each sentence, owner and area is what the index said, and the index the command writes has the same lines as before, in sorted order (the crates by name, where the old index had `cineris-tools` before `cineris-tui` before `cineris-testkit`), with the stray blank line under `cineris-session` gone.
  Two lines gained an owner: `a-managed-ask-and-the-users-own-check` and `verify-run-a-turn-that-only-verifies` had none in the old index, and now end with their table's `Who` ("the owner", and "the owner, then step 24").
  `- Flaky:` is read and checked by the same parser, for the known-flake list of the next pull request.
- **`rules` and `links` compare the whole index with what the command writes** (`xtask/src/rules/layout.rs`, `stale_index`), in place of the fifth tooling step's check that each item was listed once.
  The message names the lines the page lacks, the lines it has and should not, and the lines it has twice, or says the order is wrong, and ends with the command to run.
  An item whose file cannot be read is reported at its line, and the index is not judged until it can be: one problem at a time.
  The rules read what git tracks, and the command reads the folder, so a new item git has not been told of shows as a line the page "has and should not", with `git add` named in the message.
- **A rebase never stops on the index:** `.gitattributes` gives it `merge=union`, which keeps both sides' lines where they conflict.
  Whatever such a merge gets wrong (a line twice, the line of an item whose file is gone, a line lost, two lines out of order) the comparison above finds, since it is of the whole page; each of the four is a case of `rules::layout::tests::an_index_that_is_not_what_the_items_give_is_found`.
  **After a rebase, the one command is `cargo xtask outstanding`**, then commit the index it wrote; `regen` runs it first among its generators.
  Tried by hand on a scratch repository with this tree's index and attributes: one branch adds an item under `cineris-tools`, another adds one in the same place and settles a third; the rebase of the second onto the first went through, with both new lines kept and the settled one gone.
  A merge line by line can also keep a line that should go: where one side settles an item beside the line the other side adds, git keeps both sides' lines, the settled item's among them. `outstanding::tests::sys_a_merge_line_by_line_of_two_sides_is_what_the_command_writes_or_is_found` runs `git merge-file --union` on such a case and shows the rule finding it.
  **Before merging,** a pull request is rebased onto the `main` of the moment and the command run, as for any merge: an index that was right on an older `main` may not be on this one, and a merge GitHub made without a rebase would only be found by `main`'s own checks.
- **A step in several pull requests:** each part writes its own record, `docs/steps/<step>/<group>/<part>.md` (`23/after-review/pr2-trust.md`), which begins with a heading of its own.
  The step's own session links the group's folder once (`[after-review/](after-review/)`) from the step's `answered.md` or `ci.md`, before the parts start, so that no part adds a line to a shared file; a link to the part's own file counts too.
  `rules` checks the names (hyphenated, one folder deep), the heading, and the link, read as `links` reads links (in code or a comment it does not count; `./` and a reference's definition do).
  Documented in [CONTRIBUTING.md](../../../CONTRIBUTING.md) ("Writing the docs"), [docs/steps/README.md](../README.md), rule 14 of the build plan and of `AGENTS.md`.
- **The other shared lists, looked at:**
  `mutations/` is one file per step or part already, with no index, so two steps never touch one file: left.
  The build plan's tables are the owner's decisions, written once and amended by hand, and its "Still outstanding" is a link to the index: left, with a sentence there naming the command.
  `performance-budgets.md`'s rows are budgets the owner sets, with measurements beside them; nothing generates a budget, and a row changes rarely: left.
  `docs/decisions/README.md`'s rows are added by `cargo xtask decision new`; two steps adding a decision at once would also take the same number, which no generated index settles, and the status column is prose the records do not hold in one form: left.
  The "As built" records were turned into lists by the ninth tooling step, which puts two sessions' sentences on different lines.
- **Measured** (the laptop, a debug build of `xtask`): `cargo xtask links` 0.18 to 0.29 s over five runs, and `outstanding --check` 0.01 s, which is the whole cost of reading the 124 items and writing the index.
- **Breaks:** `mutations/tooling-10.breaks`, the `outstanding-*` and `part-record-*` breaks. Two breaks of the fifth tooling step's index check (`the-index-check-passes-an-unlisted-item`, `a-link-in-fenced-code-lists-an-item`) were of code that is gone, and are deleted: what they held is held by the new breaks.

### The review of the first pull request

An independent reviewer read the whole diff against the specification. Each finding, and what was done:

- **A step's parts still appended to one file** (confirmed with a scratch rebase): the rule wanted each part's own link in `answered.md`, and `links` refuses a link to a part not written yet, so each part's pull request added a line to the shared file.
  Fixed: a link to the group's folder covers every part in it, so the step's own session writes it once (`a_link_to_a_parts_group_covers_every_part_in_it`; breaks `part-record-group-link-is-not-enough`, `part-record-linked-by-its-words`).
- **The docs said `stress`, `ci` and `mutate` read `- Flaky:`**, which only the next pull request makes true: the words now say so.
- **`merge=union` turns a conflict GitHub would refuse into a merge it may make:** the docs now say to rebase onto `main` and run the command before merging, which the merge procedure already requires.
- **The rule's own module comment was stale:** rewritten.
- **The command and its writing were untested:** `outstanding::write` is now a function of a tree, tested for a stale index under `--check` (fails, writes nothing), a write, an index up to date, and an item that cannot be read (nothing written); `git merge-file --union` is run on a real case (above). Breaks `outstanding-check-writes`, `outstanding-the-command-reads-every-file`. The writer `regen` calls stays glue no test reaches: a break of it fails safe, since `rules` then fails on the index `regen` did not write.
- **A swap file or an image in the folder failed the command:** it reads `.md` files alone.
- **The link of a part was read as text** (`./`, a reference, a code span): it is read by the link checker's own reader.
- **A line or a heading twice was named only for an item's line:** every non-blank line now (`outstanding-a-heading-twice-is-not-named`).
- **A missing index was found only through the links to it:** the rule says so itself (`outstanding-a-missing-index-passes`).
- **`- Owner: .` gave an empty owner, and a test's name of single colons passed:** both refused (`outstanding-an-owner-of-a-full-stop`, `outstanding-a-test-name-of-single-colons`); the full stop of an item's line has its break (`outstanding-a-line-without-its-full-stop`).
- **The record said the index's lines were unchanged, and that parts were linked below:** corrected (two lines gained an owner; this step's record is one file).
- **Left, as small:** a part's group may be any hyphenated name (`ci/` too), and its heading is not held to name the step.

## The later pull requests

Each later pull request of this step has its record in [parts/](parts/), one file each, in the convention item 1 made.
