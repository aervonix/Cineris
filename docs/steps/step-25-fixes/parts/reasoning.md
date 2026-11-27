# Step 25 fixes: what is kept of a reasoning

The part of the fixes of the independent review of step 25 that settles its finding 12, every reasoning kept twice and without a bound, in both presenters, with the 200-turn memory test playing none; and, as part of the same change, its finding 18, a compaction's summary read from whichever session is open. The design is [answered.md](../../25/answered.md), point 2; what is built is in [09-tui.md](../../../architecture/09-tui.md), "Full screen as the alternative" ("What it keeps", "Search"), "The viewer" and the table of what each event becomes.

## Finding 12: a reasoning kept twice, without a bound

**What was wrong.** A finished reasoning was held twice for the rest of the session, in both presenters: its streamed text and a copy of it as its summary. Neither copy had a bound, the full screen's search added a third, lower-cased, once a search began, and the 200-turn memory measurement played no reasoning, so its 30.4 MB inline against 40 MB said nothing about it.

**The cause.** `finish_reasoning` (`app/items.rs`) copied the trimmed text into `summary` and left `text` as it was; since step 25 every finished block is kept (`transcript::Kept`, both presenters, for the viewer and the full screen), and `Kept::extend` kept the block with both fields. A round's output had a bound (`OUTPUT_KEPT`), a reasoning none.

**What changed, and why.**

- **One copy.** `finish_reasoning` moves the words from `text` into `summary`, trimmed in place and with the spare capacity of a stream that grew by doubling given back; `text` is left empty. `Reasoning`'s fields say so: `text` while it streams, `summary` the words a finished reasoning draws. The comment that called `summary` "the provider's summary" now says what it is: the protocol gives one text, `ReasoningPart::text`, the visible reasoning or the provider's own summary of it, and that is what is drawn.
- **A bound, from what is drawn.** A finished reasoning draws all of its words, dim and italic at the prose width (`view::reasoning`), and the inline presenter commits it so, whole, before it is kept: that output is not changed. What is bounded is the kept block, which only the viewer and the full screen draw again: `Kept::extend` keeps a reasoning's words to their first `kept::REASONING_KEPT` bytes, 8 KiB, cut at the end of a line in the second half of what fits (or a word's, or where the bytes run out), and marks it `cut`. 8 KiB is about a hundred rows at the prose width, two and a half windows of 40 rows: past that a reasoning is not read in the transcript, and the viewer is where a long one is read. A cut block draws what was kept and a dim row, `… cut here: enter on it in the viewer shows it whole` (`view::reasoning_cut`), only where the words are drawn at all (`tui.reasoning = "summary"`, comfortable density). Over 200 turns that is at most 1.6 MB of reasoning kept, where 16 KiB a turn held twice was 6.6 MB.
- **The rest is read when the viewer needs it**, as a compaction's summary is. Each reasoning block keeps where its whole text is (`transcript::Origin`: the session, the assistant item, and which of the item's reasoning runs it is), set by the mapper as the block begins. `Enter` on a cut reasoning in the viewer sends `session/read` with the item's id, says `reading the reasoning…`, and shows the run the block was (`items::reasoning_runs`: the parts with words that follow one another, an answer's text or a call between two ending the first, as their deltas end the block), trimmed as the block was. A reasoning not cut is shown from what was kept, with no read, as before. A response thrown away and tried again (`item.reset`) is not its item's content any more: its reasoning, still in the transcript or kept already (`Kept::forget_origins`), loses its origin, and a cut one then shows what was kept, titled `The reasoning, as far as it was kept`. `session/read` gives what is needed; the protocol is not changed.
- **The search's copy.** The full screen's search worker is given each kept block's text (`KeptBlock::text`), which for a reasoning is what was kept: what the worker holds of a reasoning, lower-cased, is at most 8 KiB. The copy itself is still the open item's ([the-search-worker-keeps-a-copy-of-the-transcript](../../../outstanding/the-search-worker-keeps-a-copy-of-the-transcript.md), updated to say so, not settled).

**The tests.**

- `cineris-tui::it memory::a_finished_reasoning_is_held_once_in_each_presenter` and `memory::a_long_reasoning_is_held_to_its_bound_in_each_presenter` (`tests/it/memory.rs`), the review's counting allocator made a real test: a counting global allocator in the crate's test binary, and the heap the application holds after 8 turns less what it holds after 4, with reasoning and without, a turn, after a change of width that drops every row the full screen had made. On `main` (the same file, with the bound as a literal, run at `origin/main`): `Inline: 6144 bytes of reasoning a turn hold 14343 bytes a turn more (2.33 times the reasoning)`, `Fullscreen: … 14275 bytes a turn more (2.32 times the reasoning)`, failing. With the fix: 1.00 and 0.99 times for 6 KiB, and 8,189 and 8,121 bytes a turn for 16 KiB (the bound is 8,192).
- `cineris-tui transcript::tests::a_kept_reasoning_holds_its_words_once_to_their_bound`: the bound, the cut at a line's end, the row drawn under it, the search's text, and what `Enter` does before and after the origin is forgotten.
- `cineris-tui app::items::tests::each_reasoning_knows_its_run_of_its_item_and_holds_its_words_once`: three reasoning blocks of one item, split by an answer and a call, have runs 0, 1 and 2 of their item, with the words `reasoning_runs` reads from its parts, and empty `text`; a reset response's reasoning has no origin.
- `cineris-tui app::tests::screen::a_reasoning_kept_only_in_part_is_read_whole_from_its_item`: a reasoning of 400 lines is committed whole inline (every line, and no `cut here`), the viewer draws its start and the row and not its last line, and `Enter` reads item `a1` of `s_test` and shows the last line.

**The inline presenter's output is not changed.** Every snapshot and reference-screen test of `cineris-tui` passes unchanged (`cargo xtask test -p cineris-tui`: 798 tests, the reference screens' `screens::` and the snapshots included); the only edit to them is `..Reasoning::default()` in the two places `tests/it/screens/scenario.rs` and the three in `transcript/tests.rs` that build a `Reasoning` with every field named, for the two fields added.

## Finding 18: a summary read from the session open now

**What was wrong.** After `/new` or `/sessions`, `Enter` on an earlier session's compaction notice read its summary by its item id from the session open now, and showed that session's item of the same id: item ids are a session's own counters.

**The cause, and why it is this part's.** `Block::Compaction` held only the item's id, and the viewer's read took the session from `App::session`. The reasoning's read above is the same read, by the same code (`App::read_summary`), so it would have had the same defect; the fix is the same change.

**What changed.** The mapper is given the session each event came from (`Mapper::event` and `in_progress`), and `Block::Compaction` keeps it beside the item, as a reasoning's `Origin` does. The viewer reads from the block's session. A session left is not open on the connection, where `session/read` refuses (`session_not_open`): nothing is asked, and the keys row says `the summary could not be read` (or `the reasoning …`). The answer is taken by its ticket alone, without the check that the session is still the open one, which is what made a summary read from the session it was of unshowable once that was left.

**The test.** `cineris-tui app::tests::screen::a_summary_is_read_from_the_session_its_block_is_of`: after `/new` to `s_other`, `Enter` on the compaction and on a cut reasoning asks nothing, and says each could not be read. The review's own test (`review_a_compaction_notice_of_an_earlier_session_reads_its_summary_from_the_open_one`, run at `origin/main`) failed there with `the summary of session A's compaction was read from session `s_other`, and shown: true`.

## The breaks

In [mutations/step-25-fixes-reasoning.breaks](../../../../mutations/step-25-fixes-reasoning.breaks), run with `cargo xtask mutate step-25-fixes-reasoning --jobs 2`:

| Break | Rule | Caught by |
|---|---|---|
| `reasoning-copied-into-its-summary` | the words moved, not copied | `memory::a_finished_reasoning_is_held_once_in_each_presenter`, `each_reasoning_knows_its_run_of_its_item_and_holds_its_words_once` |
| `reasoning-kept-without-a-bound` | a kept reasoning to its bound | `memory::a_long_reasoning_is_held_to_its_bound_in_each_presenter`, `a_kept_reasoning_holds_its_words_once_to_their_bound`, `a_reasoning_kept_only_in_part_is_read_whole_from_its_item` |
| `reasoning-runs-all-numbered-the-first` | each block its run of its item | `each_reasoning_knows_its_run_of_its_item_and_holds_its_words_once` |
| `reasoning-runs-not-ended-by-a-call` | a call ends a run | the same |
| `a-reset-response-keeps-its-reasoning-findable` | a response tried again leaves its reasoning unreadable | the same |
| `a-cut-reasoning-shows-what-was-kept` | `Enter` on a cut reasoning reads it whole | `a_reasoning_kept_only_in_part_is_read_whole_from_its_item`, `a_kept_reasoning_holds_its_words_once_to_their_bound` |
| `a-summary-read-from-a-session-left` | a read only of the block's session, while it is open | `a_summary_is_read_from_the_session_its_block_is_of` |

`cargo xtask mutate --check --tests` passes with every break applying and every `caught-by` naming a test that exists. **The verdicts are not in yet:** the run of the list waited behind another session's long run of breaks on the reference machine, and is left to the maintainers.

Three breaks of earlier lists no longer found their code, and now name it as it is, their rules unchanged: `step-23-fixes/an-item-in-progress-at-the-read-is-skipped` (the mapper's new argument), `step-25/a-compactions-summary-is-not-read` and `step-25/a-reasoning-shows-its-first-line-only` (the viewer's new `Summary` variants).

## The measurement

`perf_two_hundred_turns_in_each_presenter` (`crates/cineris-cli/tests/system/tui.rs`) plays the design's mix now: each turn's first response streams 16 KiB of reasoning before its calls, about 4,000 tokens, as a model that sends its reasoning whole gives at a high effort (`CINERIS_PERF_REASONING` sets the bytes), and the test checks that the first turn's reasoning was drawn. The full screen's scroll to the top takes a page more a turn for each KiB of reasoning, since four a turn no longer reached it. The test's change is a commit of its own, before the fix, so that it can be measured before the fix and after it.

**Not measured yet.** The measurement is to be taken on an otherwise idle machine, on the shipped build, at three trees: `main` with its test as it is, the test's commit, and the fix's. The reference machine was not idle while this part was built, so the numbers, and the paragraph of [performance-budgets.md](../../../performance-budgets.md) that is to hold them beside step 25's, are left to the maintainers. The command, at each tree:

```text
cargo xtask corpus fetch
cargo test --profile dist --target x86_64-unknown-linux-musl -p cineris-cli --test system perf_two_hundred_turns -- --ignored --nocapture
```

What to expect, from the counting allocator and arithmetic, not a run: 16 KiB of reasoning a turn held as on `main` (2.33 times its size, the copy and the stream's capacity) is about 7.6 MB over 200 turns, on top of step 25's 30.4 MB inline, against 40 MB; with the fix it is the bound, 1.6 MB, in each presenter, and as much again in the full screen once a search has run. The run with the test alone also says what the core holds of the reasoning in the session's context, which is in the same process and not this part's to bound.

Budgets are not changed. A short run of the test's commit (3 turns, 200 files, not a measurement) passed in both presenters.

## Changes of interface

- **Public API of `cineris-tui`** (none outside the workspace uses it): `transcript::Reasoning` has two more fields, `origin` and `cut`; `transcript::Origin` is new; `Block::Compaction` has a `session` field; `kept::Summary`'s `Reasoning` is now `Reasoning { text, cut }`, `Compaction` is now `Compaction { session, item }`, and `ReadReasoning(Origin)` is new; `kept::REASONING_KEPT`, `Kept::forget_origins` and `view::reasoning_cut` are new. Not additive for code that builds a `Reasoning` with every field named, or matches `Summary` or `Block::Compaction` by position; nothing outside the crate does.
- **No protocol, log, format or setting change.** No budget is changed.

## Docs changed

- [09-tui.md](../../../architecture/09-tui.md): "What it keeps" (the bound, the row, the read), "Search" (what a reasoning gives it), the viewer (the read of a cut reasoning, and from which session both reads are made), and the table of what each event becomes (the words moved, not copied; `origin` and `cut`).
- [performance-budgets.md](../../../performance-budgets.md), "Measured in step 25": the command printed 200 lines, not about 40, and no reasoning was played. The measurement with reasoning is not there yet (above).
- [the-search-worker-keeps-a-copy-of-the-transcript](../../../outstanding/the-search-worker-keeps-a-copy-of-the-transcript.md): what it holds of a reasoning is bounded; the rest of the item stands.
- [testing-and-ci.md](../../../testing-and-ci.md), "A reviewer's kit": a counting global allocator, for how much of the heap a structure holds.

## Left open

- **The measurement and the breaks' verdicts** (above): `cargo xtask mutate step-25-fixes-reasoning --jobs 2`, and the 200-turn test at `main`, at the test's commit and at the fix's, on an otherwise idle machine. The tests that start no process need no Windows run of their own beyond CI's; nothing here is Windows-only.

- **The full screen draws a long reasoning cut,** where before it drew it whole, as the inline presenter does: the bound changes what the full screen and the viewer's transcript show of a reasoning past 8 KiB (the row says so, and `Enter` in the viewer shows it whole). Reading the rest when the window reaches such a block, so that the full screen draws every reasoning whole again, is not built; it would change a block's rows while it is in view. A question for the maintainers.
- **A cut reasoning, or a compaction, of a session left** cannot be read: `session/read` serves only a session open on the connection. The viewer says it could not be read and shows nothing, where for a cut reasoning it could show the start it kept, as it does for one whose response was tried again: a small change, left for step 33 with the rest of finding 12's memory work.
- **The counting allocator** counts every test of `cineris-tui`'s integration binary: an atomic add on each allocation, a few nanoseconds, in its `perf_` tests too.
