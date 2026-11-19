# The twelfth tooling step, part B: rebasing and git

Branch `tooling-12-b`, rebased by its own `cargo xtask rebase` onto `origin/main` at 6c43dfb1. This part changes `xtask`, its tests and fixtures, and the docs; no product crate.

## What was built

### 1. `cargo xtask rebase`

- `cargo xtask rebase [--onto REF] [--no-fetch] [--no-regen]`, `--continue`, `--abort` (`xtask/src/rebase/`): fetches `REF`'s remote (`origin/main` when not given), runs `git rebase REF`, and at each stop puts every conflicted file in a class and settles it by the class ([testing-and-ci.md](../../../testing-and-ci.md#rebasing-onto-main) has the table): a generated file (one `classify::GENERATED` names, held to `regen::WRITERS`; since the review, file by file) and the open items' index take the side rebased onto with `git checkout --ours`, and are written again at the end; a `.rs` or `.md` file whose two sides only added lines at one place, or where one side added lines just after lines the other replaced with as many or more (since the review), gets both, the side rebased onto first; anything else is left, named, with the reason.
- **No side is lost silently.** `classify::keeps_both` checks the text that would be written, whatever made it: each line as many times as the two sides' changes come to (a lost `}` is found though another `}` could stand in for it, which a plain "is every line there, in order" test misses: the first version of this check had that hole, and its own test found it), each side's added lines in their order, and the base's lines both kept in their order. Then the class's check: brackets balanced outside comments, strings and character literals, and rustfmt leaving the text as it is (Rust); every line put side by side a table's row with its header's number of cell dividers (Markdown).
- The merge is the command's own (`merge.rs`, Myers' diff of lines, about 370 lines with its diff3), so that it is a pure function of three texts, and so that a run of added lines is pushed down past the lines it starts with: an appended test is then the whole test after the old one's `}`, never "`}`, the test, its body" before it.
- **Beside a change** (added after the real rebase below): main reworded a table's last row and added rows after it; the branch added a row after it. git stops on an addition next to a changed line, and so did the first version of this command. It is an addition all the same: the change is kept whole and the addition is put after it. Two changes of one line are still refused, and, since the review, so are an addition before a change and an addition beside a deletion.
- State between a stop and `--continue` is a file `xtask-rebase` in the working tree's git directory: the tip before, the target, and the generators to run. The end writes the open items' index (always, as git merges it line by line), runs `regen` for the generators of the files taken (`--no-regen` names the command), says that what it wrote is not committed (only when it wrote something), and prints `git reset --keep <old tip>`.
- One git per step with plain arguments; `GIT_EDITOR=:` for `--continue`. A commit left empty after its conflicts are settled is dropped with `--skip`, and named.

### 2. Questions for git in one command

`cargo xtask git changed [--against REF]`, `git commits PATH… [--since REF] [--max N]`, `git added --grep RE [--against REF] [PATH…]` (`xtask/src/git_read.rs`). Named for the question each answers; under `git` because what they read is git's; they only read. `changed` is the merge base of `REF` and `HEAD` against the working tree, with untracked files, each change not all in `HEAD` marked; `added` reads `git diff -U0` from the merge base, numbering each added line from its hunk's `+start`, plus the lines of new untracked files, matched with `pattern.rs`.

### 3. `ci-times --write` makes a missing page

`records::write_sections` makes a page that is not there, and its folder, with the heading the step records' rule asks for from its place (`new_page_heading`: `# After step 24: its CI times`, `# After tooling-12: its CI times`, `# Step 24, pr2 trust: its CI times` for a part, `# CI times` elsewhere), and gives an empty page that heading. A page with no such section already got it added at the end; a test now holds that too.

### Docs

[testing-and-ci.md](../../../testing-and-ci.md): new sections "Rebasing onto main" and "Questions for git in one command", and a paragraph "A page that is not there yet" in "A CI run's test times". AGENTS.md: two rows in "Working here", and the `ci-times` row says a missing page is made. `USAGE` in `xtask/src/main.rs` has the commands.

## Tests

- `xtask/src/rebase/tests.rs`: 27 tests. The conflict shapes the last wave met are fixtures in `xtask/fixtures/rebase/`, cut from the repository's own text: AGENTS.md's rows for the open items' index (5996f119) and for a known flake (b37efe3e), added by two pull requests of the tenth tooling step, once with LF and once with CRLF; the fields `keep_sessions` and `allow_unrestricted` of `CoreOptions` and their `Debug` lines; two of `regen.rs`'s tests appended to one module. Refusals: a line changed on both sides, prose, a row of another shape, rows with no table, a change both sides made alike beside a union, a union that lost a brace (refused by the count and by the brackets, each alone), a line of theirs lost or doubled, base lines and a side's lines out of order.
- Three `sys_` tests on repositories made with one `git fast-import`: on both systems, one stop settles a table, a Rust module (rustfmt run on the union) and a generated file, and the end names the generator; on Unix only, a line both sides changed is left with git's markers, `--continue` stops again until a person settles it, then ends with the first stop's generator kept; and abort, a second start refused, changes not committed refused.
- `git_read.rs`: 4 tests, one `sys_` on both systems asking the three questions of a branch with a commit, a change not committed and a new file.
- `records.rs`: 2 tests (the heading by place; a missing page and folder made, an empty page headed, a missing section added).

## What was measured

- On the reference machine, the new tests together take under 0.3 s; the slowest is the cross-system rebase test, 0.06 s (about 17 gits and one rustfmt).
- On the runners, `try` run 44593291869 (`test -p xtask`, both systems, at 9975c6e2), nothing failed. This part's 52 tests on Linux took 0.72 s in all (the four `sys_` tests 0.05 to 0.11 s each); on Windows its 50 (two `sys_` tests are Unix only) took 2.5 s, of which the cross-system rebase test is 1.42 s and the questions for git 0.66 s: about 2.1 s of tier 3's 90 s. `xtask`'s own tiers in that run: Linux 15%, 0%, 8%; Windows 14%, 0%, 19%.
- Nothing is added to a blocking job but these tests and the build of about 2,900 lines of `xtask` (with its tests). No dependency was added.
- `cargo xtask ci` on the reference machine at 9975c6e2 passed every step; tier 1 warned at 116% of its budget while two break runs of other branches loaded the machine, which the run itself warned of. The runner's tiers above are the measurement.

## The real rebase

`origin/main` moved to 6c43dfb1 (#109, the old alias's test labels renamed) while this part was built. `cargo xtask rebase` on `tooling-12-b`:

```text
Rebased tooling-12-b onto origin/main with no conflict.
docs/outstanding/README.md is up to date.
What the end wrote is not committed: look at it, and commit it (or fold it in with `cargo xtask branch fold`).
The tip before was 4f561b1a5af4: `git reset --keep 4f561b1a5af4` takes the branch back to it.
```

No conflict: #109 touched none of this part's lines. The run found a fault: the note "What the end wrote is not committed" was printed when the end wrote nothing. It is now printed only when the index was written or a generator ran.

To show a stop, a throwaway branch was made at 8773da46's parent, with this part's commits and one more that adds a row after row `B` of the table in `docs/steps/25/parts/pr3-first-run.md`, where 8773da46 added rows B-M1 to B-L7 and d3b0e511 later reworded row `B` itself. The first version of the command, run on it:

```text
Stopped at c3992548 Add a row to the table that main also adds rows to (a throwaway branch):
  LEFT      docs/steps/25/parts/pr3-first-run.md: line 211: both sides change line(s) 211..211 of the base: not additions alone
The rebase is stopped. Settle each file LEFT above, `git add` it, then run `cargo xtask rebase --continue` (or `--abort`).
The tip before the rebase was c3992548734d.
```

That is the shape a parallel wave meets most (a row reworded, rows added after it on both sides), so the command now keeps an addition beside a change. Run again:

```text
Stopped at c3992548 Add a row to the table that main also adds rows to (a throwaway branch):
  united    docs/steps/25/parts/pr3-first-run.md: both sides' additions at 1 place(s); every line of both kept, and checked
Rebased tooling-12-b-demo onto origin/main: every conflict settled.
docs/outstanding/README.md is up to date.
The tip before was c3992548734d: `git reset --keep c3992548734d` takes the branch back to it.
```

Row `B` as main reworded it, main's rows B-M1 to B-L7, then the branch's row. The throwaway branches were deleted.

## Breaks

`mutations/tooling-12-b.breaks`, 29 breaks: each test of `keeps_both`, `unite`'s three checks and its call of `keeps_both`, the Markdown row, shape and header checks, the cell dividers' escape, the brackets' comments and character literals, the merge's refusal of two changes, the addition beside a change (its side and its kind), the push down, each class, the side taken, the command's `git add` and `--continue`'s state, `git added`'s line numbers and `git changed`'s mark, and `ci-times`' page, empty page, folder and heading.

`cargo xtask mutate tooling-12-b --jobs 2`, on the reference machine: **29 breaks, 29 did what the list expects, 0 flagged.** The baseline was 818 tests passing. Each break was caught by the tests its list names, and some by more: `rebase-rows-with-no-table` by seven (the break refuses every Markdown union), `rebase-changes-on-both-sides-united` and `rebase-a-change-counts-as-an-addition` by four each, among them the `sys_` test of a line both sides changed. The first `keeps_both` was a plain "every line, in order" check, and the test of a lost brace failed on it, as another `}` satisfied it: that is what led to the count.

## The pre-push review

An independent review read the branch before its pull request. Each finding it confirmed is fixed, with a test and a break in `mutations/tooling-12-b.breaks`.

| # | Finding | Verdict | Fix | Test and break |
|---|---|---|---|---|
| 1 | An addition beside a deletion was united: ours deletes `fn old() {}`, theirs puts `#[cfg(windows)]` above it, and the union made `fn b()` Windows-only with every check passing; the same with `#[test]`, a doc comment, or a `let` added after lines the other side deleted | Confirmed, high | `merge::beside` keeps an addition only when it is **after** lines the other side **replaced with as many or more**; an addition before a change, or beside a deletion or a shrinking change, is left | `an_addition_beside_lines_the_other_side_deleted_is_left` (fixture `deletion-beside.rs`, both ways round; `#[test]`, `///`, the `let`, a change to fewer lines); the reworded-row test now refuses a row added before the change. Breaks `rebase-an-addition-beside-a-deletion`, `rebase-an-addition-before-a-change` |
| 2 | `--continue` that fails with no conflict looped for ever (a file git does not track in the way of a commit), and `--skip`'s status was ignored | Confirmed, high | `go_on` skips only a commit this run settled to nothing, checks `--skip`'s status, and stops with git's words when the rebase did not move: neither `HEAD` nor the list of what is left to do changed. Not `rebase-merge/msgnum`, as the review proposed: git counts a pick it could not make, and put back, as a step, so `msgnum` goes up on every failed `--continue` (seen with git 2.53). git's message names the files in the way | `sys_a_file_not_tracked_in_the_way_stops_the_rebase_with_gits_words` (real git: W1 adds `x.txt`, W2 removes it, `x.txt` not tracked in the tree; the file is untouched, `--abort` works). Breaks `rebase-a-rebase-that-does-not-go-on-loops`, `rebase-go-on-skips-always` |
| 3 | A commit left empty was dropped by git itself (git 2.53's `--continue` exits 0), unreported | Confirmed | Before `--continue`, when `git diff --cached --quiet HEAD` says nothing is staged, the commit is named in the report and skipped with `--skip` | `sys_a_commit_left_empty_is_skipped_and_named_and_a_failed_end_keeps_the_report` (a commit whose only change is a generated file both sides changed; the commit after it goes on). Break `rebase-a-commit-left-empty-not-named` |
| 4 | Hand-written READMEs were generated: `docs/protocol/README.md` and `docs/architecture/tui-look/README.md` took main's side; a test listed `tui-look/screens.md`, a file no generator writes | Confirmed | `classify::GENERATED` names the files each generator writes (`docs/protocol/schema/` and `ts/`, the configuration's two files, the screens' `.ans` and `.txt`, the snapshot folders), and a test holds it to `regen::WRITERS` both ways | `each_file_is_put_in_its_class` (both READMEs as Markdown; `2c.txt`, `2c-dark.ans`), `the_generated_files_are_the_places_regen_writes`. Breaks `rebase-a-hand-written-page-is-generated`, `rebase-the-protocols-readme-is-generated` |
| 5 | A failed `regen` or `outstanding` at the end lost the whole report, the tip before among it | Confirmed | The end's error is put in the report (`end_failed`), the report is printed whole, with `git reset --keep`, and the command then fails | The empty-commit `sys_` test, with an end that fails. Break `rebase-a-failed-end-loses-the-report` |
| 6 | `git added` misread headers: a name with a space kept git's trailing tab, a C-quoted name was dropped | Confirmed | `git_read::header_name` takes the tab off and reads git's C quoting (octal bytes included) | `a_diffs_file_names_are_read_as_git_writes_them`, on git 2.53's own output for `a b.md`, `q"x.md` and `été.md`; the `sys_` test now uses `docs/a page.md`. Breaks `git-a-names-tab-kept`, `git-a-quoted-name-dropped` |
| 7 | `checkout --ours` and `add` read the names as patterns | Confirmed, low | `GIT_LITERAL_PATHSPECS=1` on both | The empty-commit `sys_` test's generated file is named `*.json`, beside a file git does not track that the pattern would add. Break `rebase-names-are-patterns` |
| 8 | `added --against` and `--grep` took a value starting with `-` | Confirmed, low | Refused, as the other parsers refuse it (a pattern can start `[-]`) | `each_question_reads_its_arguments_and_refuses_what_it_cannot_use`. Break `git-added-takes-a-flag-as-a-value` |
| 9 | `keeps_both` did not cover the last line's end or CRLF; a page with no final line end and a row appended by the other side ended with none | Confirmed, low | The union's last line ends with a line end when either side's does, and `keeps_both` refuses a union that lost it. CRLF: the union is written in the side rebased onto's line ends, every line alike, and `keeps_both` reads line ends alike (the repository's text files are LF everywhere); a file whose sides differ only in line ends is merged into ours' | `the_last_lines_end_is_kept_when_either_side_has_it`. Breaks `rebase-the-last-line-end-from-ours-alone`, `rebase-a-lost-line-end-unchecked` |
| 10 | A change both sides made alike was always refused | Confirmed, low; kept | Kept as a refusal, and said in the docs and the usage: it happens only in a file git stopped on that also has a union, and the count takes it twice. git merges such a change itself when nothing else in the file conflicts | `a_change_both_sides_made_alike_beside_a_union_is_left` (as before) |
| | Missing breaks: `--ours` for `--theirs`, the "disagree" refusal, the last line's end, `go_on` skipping always | Confirmed | Added | `rebase-take-theirs`, `rebase-git-and-this-merge-disagree-unsaid`, `rebase-the-last-line-end-from-ours-alone`, `rebase-go-on-skips-always` |
| | `ci-times --write` | Found sound | It already prints where it made a page (`made <path> with the heading …`) | |

The real rebase's union (a row after a reworded row) is still kept: the reworded row is a replacement of one line by eight, and the branch's row is after it.

Two more faults were found while the fixes were shown, and are fixed with them: `--abort`'s report began "Rebased HEAD onto origin/main with no conflict." (it now says only that the rebase was aborted; the abort `sys_` test holds the whole text), and prose's refusal named a blank line as two backticks (it now says "a blank line"; the prose test holds it).

**The command run again after the fixes**, with the binary built from this branch, on three throwaway branches of this checkout, each deleted after:

1. **Every fixture shape at once.** A commit put each fixture's base in `scratch-demo/fixtures/`, one branch on it each fixture's side rebased onto, another each one's other side; the second rebased onto the first. The four shapes the last wave met are united, the CRLF table among them; the four refusals are left, each with its reason (this run was before prose's wording was mended, so its blank line shows as two backticks):

```text
Stopped at 130ce411 Demo: the fixtures' side rebased:
  LEFT      scratch-demo/fixtures/deletion-beside.rs: line 3: both sides change line(s) 3..4 of the base: not additions alone
  LEFT      scratch-demo/fixtures/prose.md: line 4: both sides added text that is not a table's row: ``
  LEFT      scratch-demo/fixtures/row-of-another-shape.md: line 8: a row of 2 cell dividers in a table whose header has 3
  LEFT      scratch-demo/fixtures/same-line.rs: line 2: both sides change line(s) 2..2 of the base: not additions alone
  united    scratch-demo/fixtures/struct-fields.rs: both sides' additions at 2 place(s); every line of both kept, and checked
  united    scratch-demo/fixtures/table-rows-crlf.md: both sides' additions at 1 place(s); every line of both kept, and checked
  united    scratch-demo/fixtures/table-rows.md: both sides' additions at 1 place(s); every line of both kept, and checked
  united    scratch-demo/fixtures/tests-appended.rs: both sides' additions at 1 place(s); every line of both kept, and checked
The rebase is stopped. Settle each file LEFT above, `git add` it, then run `cargo xtask rebase --continue` (or `--abort`).
The tip before the rebase was 130ce4114f4e.
xtask: the rebase stopped on files left for a person
```

2. **A deletion beside an addition, in a real file.** The side rebased onto deleted `paths_of` and its doc comment from `xtask/src/git_read.rs`; the branch put `#[inline]` between them. Before the fix this was united, and the attribute would have gone onto the next item:

```text
Stopped at 1aa0dae7 Demo: mark paths_of inline:
  LEFT      xtask/src/git_read.rs: line 183: both sides change line(s) 183..187 of the base: not additions alone
The rebase is stopped. Settle each file LEFT above, `git add` it, then run `cargo xtask rebase --continue` (or `--abort`).
The tip before the rebase was 1aa0dae7cf0b.
xtask: the rebase stopped on files left for a person
```

3. **The table row of the first demonstration**, again (a row after a row that main reworded and added rows after): still united, as before the fixes.

```text
Stopped at a260b8c9 Add a row to the table that main also adds rows to (a throwaway branch):
  united    docs/steps/25/parts/pr3-first-run.md: both sides' additions at 1 place(s); every line of both kept, and checked
Rebased b-demo3 onto origin/main: every conflict settled.
docs/outstanding/README.md is up to date.
The tip before was a260b8c90fb5: `git reset --keep a260b8c90fb5` takes the branch back to it.
```

**Breaks:** `cargo xtask mutate tooling-12-b --jobs 2`, on the reference machine, after the fixes: the first run flagged two of 43. `rebase-a-change-counts-as-an-addition` was not caught by any test: a change that starts at the place's end would end past it, so the guard it broke could never fail; the guard and the break are gone. `rebase-an-addition-before-a-change` was caught, by the reworded-row test and `changes_apart_merge_and_one_sides_change_is_taken_whole`, but not by the deletion test its list named (a deletion is refused before the place is looked at); its list now names those two. Run again: **42 breaks, 42 did what the list expects, 0 flagged** (824 tests pass with nothing broken). `rebase-a-rebase-that-does-not-go-on-loops` takes 62 s: its test runs to nextest's limit, as the loop the review found would.

**`cargo xtask ci`** on the reference machine after the fixes: every step passed. The tiers were not judged, as the run itself said: the disk stalled (tasks waited on I/O for 49% of the test runs) while other sessions' runs shared the machine. This part's record is linked from the step's `answered.md`, which the step's session writes; the run had a stand-in for it in the working tree, never committed, so that `rules` saw the link it will have.

**`try` run 44607742443** (`test -p xtask`, both systems, at d4c5c848, before the last two commits): every test passed but `rules::tests::sys_the_repository_follows_its_own_rules`, on both systems, for the one reason above (the record not yet linked). This part's tests: Linux 58 in 0.90 s; Windows 54 in 2.70 s, the cross-system rebase test 1.44 s and the questions for git 0.71 s. **`try` run 44610050146**, at 6410f868 with every fix: the same, on both systems: 787 and 788 tests of tiers 1 and 2 passed, and of the system tests only the rules test failed, for the record's link alone.

## Open

- **A change both sides made alike, in a file with a union,** is refused by the count (it is taken once and counted twice). Rare in a file git stopped on; a person settles it (review finding 10, kept).
- **A commit a person settles to nothing** before `--continue` is dropped by git's own `--continue`, unnamed: the command names only the commits it settled itself.
- **The end runs `regen`, which builds and runs tests** for the snapshots' generator; `--no-regen` names the command instead. A rebase is a git operation run in the checkout, so where builds run on another machine, run it with `--no-regen` and the generators there.
- **The record's link.** This part's record is linked from the step's `answered.md`, which the step's own session writes; until then `cargo xtask rules` names it as not linked.
