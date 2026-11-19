# The twelfth tooling step, part B: rebasing and git

Branch `tooling-12-b`, rebased by its own `cargo xtask rebase` onto `origin/main` at 6c43dfb1. This part changes `xtask`, its tests and fixtures, and the docs; no product crate.

## What was built

### 1. `cargo xtask rebase`

- `cargo xtask rebase [--onto REF] [--no-fetch] [--no-regen]`, `--continue`, `--abort` (`xtask/src/rebase/`): fetches `REF`'s remote (`origin/main` when not given), runs `git rebase REF`, and at each stop puts every conflicted file in a class and settles it by the class ([testing-and-ci.md](../../../testing-and-ci.md#rebasing-onto-main) has the table): a generated file (a place `regen::WRITERS` names, but the recorded provider fixtures) and the open items' index take the side rebased onto with `git checkout --ours`, and are written again at the end; a `.rs` or `.md` file whose two sides only added lines at one place, or where one side added lines just before or after lines the other changed, gets both, the side rebased onto first; anything else is left, named, with the reason.
- **No side is lost silently.** `classify::keeps_both` checks the text that would be written, whatever made it: each line as many times as the two sides' changes come to (a lost `}` is found though another `}` could stand in for it, which a plain "is every line there, in order" test misses: the first version of this check had that hole, and its own test found it), each side's added lines in their order, and the base's lines both kept in their order. Then the class's check: brackets balanced outside comments, strings and character literals, and rustfmt leaving the text as it is (Rust); every line put side by side a table's row with its header's number of cell dividers (Markdown).
- The merge is the command's own (`merge.rs`, Myers' diff of lines, about 370 lines with its diff3), so that it is a pure function of three texts, and so that a run of added lines is pushed down past the lines it starts with: an appended test is then the whole test after the old one's `}`, never "`}`, the test, its body" before it.
- **Beside a change** (added after the real rebase below): main reworded a table's last row and added rows after it; the branch added a row after it. git stops on an addition next to a changed line, and so did the first version of this command. It is an addition all the same: the change is kept whole and the addition is put after it (or before, as the text has it). Two changes of one line, or a change beside a change and an addition, are still refused.
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

## Open

- **A commit emptied by its settled conflicts** is dropped with `rebase --skip` and named; no test makes one, and no break covers it.
- **A change both sides made alike, in a file with a union,** is refused by the count (it is taken once and counted twice). Rare in a file git stopped on; a person settles it.
- **The end runs `regen`, which builds and runs tests** for the snapshots' generator; `--no-regen` names the command instead. A rebase is a git operation run in the checkout, so where builds run on another machine, run it with `--no-regen` and the generators there.
- **The record's link.** This part's record is linked from the step's `answered.md`, which the step's own session writes; until then `cargo xtask rules` names it as not linked.
