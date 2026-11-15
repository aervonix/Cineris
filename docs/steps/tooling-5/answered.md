# Answered in the fifth tooling step, on 2026-11-15

This step is not a numbered step of the build plan: it built the fifth round of developer tooling, from what wave 8's four build sessions asked for, and carried out one decision of the owner's on the default theme's colours. Its records are here, in a folder named for the job.

## The coordinator's answers and additions

| Question or addition | Answer |
|---|---|
| Measuring on the runners before the step branch's first push. | A scratch branch, `worktree-step-02-tooling-5-try`, for measurement only: a `try` input for the test kit's profile (an allow-list, set through `env:`), the profile written as a file a test, and three throwaway probes, each deleted from it after its run. Every number quoted from it names its commit, whose only difference from `main` is the measurement change. Nothing of the probes is in the step branch. |
| How to read item 1's target, given that `main`'s tier 2 is 27 s on a fast Windows runner and 38 s on a slow one. | About a quarter of tier 2 cut, shown as a share of the tier in the A/B, so that a slow runner (38 s) lands near 29 s: the room is for the slow runners CI draws and for steps 16 and 18. |
| Owner-only files on Windows, if they were the cost of a session's open. | A change to `cortex_base::private` is security-sensitive and is asked about first. The probes showed it is not the cost: no product change was made there. |
| The lease's forced flush at a session's birth. | Not changed: it is behaviour around an enforced value. It is in [performance-budgets.md](../../performance-budgets.md) and an open item. |
| Running the system tests after tiers 1 and 2. | An experiment on the scratch branch first, with the process group at 3 and at 4, against the job as it is, on both OSes (`try` run 43665893748, five rounds of each layout). **Adopted, by the owner's decision of 2026-11-15,** at 3 and in this step: `cargo xtask ci test` runs tiers 1 and 2 and then the system tests alone, three at a time, and each tier is measured under its own load. The budgets stay as they are. The open item that held the question is closed; [testing-and-ci.md](../../testing-and-ci.md), "Each tier is measured under its own load", has the experiment and the layout. |
| The colour decision's reach: code's strings and numbers, and a changed line drawn in its colour where there is no tint. | **Status text only** (the owner, 2026-11-15): the status marks, warnings, errors and measures, and a diff's signs, go bold on a light background; highlighted code and whole changed lines keep their weight, and stay under the target there by that choice. The palette's flag `warm_bold_code_and_lines` is off. |
| The rig cuts (shared per-session set-up, a cheaper tear-down). | Skipped, as agreed: the probes put the rig's own share at about 2 ms a test on Windows (`World::new` 0.5 ms, `Core::start` 0.3 ms, dropping the world 1.5 ms), about 0.2 s of tier 2. A session's open, 4.6 ms there even with no store, is the core's own `assemble`, which a rig cannot share. |


## The owner's colour decision

Decision 0033 is amended ("Amendment, 2026-11-15"); no new decision number. On a light background, status text in red, green or yellow is bold (highlighted code and whole changed lines keep their weight, by the owner's answer); on a dark one, and when none is reported, keywords in code are bright blue; muted stays as it is.

| Colour | One Half Light (`#FAFAFA`) | Campbell (`#0C0C0C`) |
|---|---|---|
| Red, ANSI 1 | 3.5 to 1: status text now bold | 3.2 (bright red, 5.1, is used) |
| Green, ANSI 2 | 3.1 to 1: status text now bold; code's strings unchanged | (bright green is used) |
| Yellow, ANSI 3 | 3.1 to 1: status text now bold; code's numbers unchanged | (bright yellow is used) |
| Keywords | magenta, 5.9 to 1, unchanged | bright blue, 5.0 to 1, was bright magenta, 3.2 |
| Muted, bright black | | 4.3 to 1, unchanged, by the owner's choice |

The reference screens were rewritten by `cargo xtask tui-screens --write`, and every difference from `main`'s was of style alone: 320 cells of 18 screens. (The first build, with code and whole lines bold too, changed 728; the owner's answer took 408 of them back.)

| Screen | Cells changed |
|---|---|
| 1-dark, 6-dark, 7a-dark, 7c-dark | 4 each (keywords) |
| 8a-dark, 8b-dark | 18 each (keywords) |
| 1-light | 25 |
| 1c-light | 12 |
| 1d-light | 6 |
| 2a-light | 1 |
| 2b-light | 21 |
| 4-light | 25 |
| 5-light | 67 |
| 6-light | 25 |
| 7a-light, 7c-light | 19 each |
| 8a-light | 21 |
| 8b-light | 27 |

The view snapshots that changed: `blocks`, `header-and-hint`, `round` and `theme`.

## Windows tier 2: where its time goes

Measured on the scratch branch at `6e96c25` (`main` at `9ddbf67` and the profile's `try` input, nothing else).

| Run | What | Tier 2 above the start |
|---|---|---|
| `main`'s own CI run, 43639354819 (start 10 ms) | The whole suite, as the blocking job runs it | 38.3 s of 40 |
| `try` 43663690282, A/A, three rounds (start 8 ms) | The whole suite, `main` against itself on one runner | 27.1 s (27.4, 27.5, 33.5 on one side) |
| `try` 43663367508 (start 9 ms) | Tier 2 alone, profiled | 21.2 s |

The slowest twenty in `main`'s CI run, with their time alone and what the profile says of it:

| CI run | Alone | Test | Where it goes |
|---|---|---|---|
| 1.90 s | 1.39 s | `cortex-session::it` `durability::an_entry_over_the_line_limit_is_refused_and_nothing_is_lost` | Its own work: encoding a line of 66 MiB, `serde_json`'s generic code compiled unoptimised in `cortex-session`. Already the fewest bytes that make such a line. |
| 1.55 s | 0.98 s | `cortex-llm::it` `openai_chat::decode::at_every_cut::deepseek` | Decoding, unoptimised (no profile: no scratch directory) |
| 1.21 s | 0.96 s | `cortex-session::it` `durability::a_log_cut_at_every_byte_of_a_last_response_opens_cleanly` | Its own work: about ten file operations a cut, at 0.1 to 0.5 ms each on Windows |
| 1.04 s | 0.61 s | `cortex-llm::it` `openai_chat::decode::at_every_cut::openrouter` | Decoding |
| 0.95 s | 0.53 s | `cortex-llm::it` `openai_chat::decode::at_every_cut::venice` | Decoding |
| 0.67 s | 0.41 s | `cortex-core::it` `approvals::an_approval_for_a_change_to_a_file_shows_the_change_within_a_bound` | Opening a session 14 ms; four waits for an approval 10, 142, 53 and 84 ms (the core diffing a large file); the turn's end 75 ms; tear-down 4 ms |
| 0.62 s | 0.50 s | `cortex-session::it` `durability::a_log_cut_at_every_byte_of_a_last_prompt_opens_cleanly` | Its own work, as above |
| 0.61 s | 0.43 s | `cortex-tools::it` `search::a_search_over_a_generated_tree_finds_every_match_and_can_be_cancelled` | Making and searching the tree |
| 0.58 s | 0.37 s | `cortex-term::it` `presenter::a_streaming_frame_is_small` | Presenting, unoptimised |
| 0.56 s | 0.23 s | `cortex-tui::it` `editing::no_row_is_wider_than_its_width_whatever_follows_white_space` | Layout, unoptimised |
| 0.53 s | 0.45 s | `cortex-policy::it` `real_paths::a_pattern_is_judged_by_what_the_directory_holds` | Its own work: real paths |
| 0.51 s | 0.19 s | `cortex-tui::it` `screens::the_views_draw_the_reference_screens` | Drawing 34 screens and loading the grammars |
| 0.45 s | 0.24 s | `cortex-policy::it` `bypasses::the_final_review_of_the_fixes_holds` | Analysis, unoptimised |
| 0.42 s | 0.28 s | `cortex-tools::it` `editing::property_what_a_round_kept_puts_every_changed_file_back` | File work, per case |
| 0.41 s | 0.20 s | `cortex-llm::it` `anthropic::decode::a_small_stream_decodes_the_same_at_every_pair_of_nearby_cuts` | Decoding |
| 0.40 s | 0.29 s | `cortex-agent::it` `order::one_seed_is_one_order_of_events` | Its own work: a world and a session each round |
| 0.36 s | 0.31 s | `cortex-session::it` `durability::a_log_cut_at_every_byte_of_a_last_record_opens_cleanly` | Its own work, as above |
| 0.33 s | 0.24 s | `cortex-tools::it` `execute::a_walk_that_is_cancelled_part_way_stops_there` | Walking a tree |
| 0.30 s | 0.22 s | `cortex-session::it` `snapshot::rewinding_to_any_turn_puts_every_file_back_as_it_was_then` | Its own work: files |
| 0.29 s | 0.18 s | `cortex-term::it` `presenter::every_frame_is_one_synchronized_write_that_clears_nothing_and_stays_in_the_live_region` | Presenting |

The twenty are 30% of the tier; the rest is a thousand tests of tens of milliseconds. By crate, alone on Windows: `cortex-core` 4.4 s (80 tests, 51 ms each: a session's open 14 ms, a turn about 12 ms), `cortex-llm` 4.1 s, `cortex-session` 3.8 s, `cortex-tools` 3.0 s, `cortex-agent` 2.0 s. Two probes on the scratch branch (one test file each, deleted after its run; [performance-budgets.md](../../performance-budgets.md), "Open a session, on Windows against Linux") put the core's session open at 4.6 ms on Windows with no session store as with one, and a session's files at 0.1 to 0.4 ms an operation: the cost is the core's assembly, not the owner-only files, and not the rig's (about 2 ms a test). What was tried and dropped: `cortex-llm` at `opt-level = 1` in CI's profile (1.6 s more to rebuild at the desk for 0.3 s less of its tests), and a cheaper text for the line-limit test (slower).

**What does reach the target** is not in the tests: with the system tests run after tiers 1 and 2, three at a time, Windows tier 2 was 17.8 s against 22.5 s on one runner in the experiment, 21% less, with no test changed. The owner adopted it, and the step has it ([testing-and-ci.md](../../testing-and-ci.md), "Each tier is measured under its own load"). What it did on the rebased tree, against `main`, is in [ci.md](ci.md).

## The estimate at a desk

The factors of `cargo xtask ci --tiers --like-runner` were measured from `main` at `9ddbf67`: the desk's run in CI's profile at four test threads (`rx --exclusive`, desk start 2 ms) against the A/A run's medians (start 3 ms on Linux, 8 ms on Windows). Whole tiers, runner over desk: Linux 2.27, 1.14 and 1.76; Windows 4.33, 2.85 and 12.23. A slow runner is 1.77 times the measured one on Linux and 1.41 on Windows (`main`'s CI run). The table by crate is in `xtask/src/like_runner.rs`. A crate-scoped A/B through `try` (`-f command="test -p cortex-llm" -f against=main -f count=5`, run 43667663980) took 2 min 24 s on Windows and 1 min 20 s on Linux.

## Answered by the coordinator on the way

| Question | Answer |
|---|---|
| Does Windows 11's Notepad, the default external editor, wait? | Yes. The coordinator ran the Store Notepad, 11.2609, from a test on the desktop's interactive session on 2026-11-15: it waits until its window closes, with several tabs open too. The other half of the item stays open: [outstanding/the-default-editor-on-windows-11.md](../../outstanding/the-default-editor-on-windows-11.md). |
| What `ci-wait` and `rx` do now. | `ci-wait` passes over dispatched and scheduled runs created more than `--fresh` minutes before it started, and runs already cancelled, naming each (`--any-age` turns that off); `rx` warns when a `ci` or `--tiers` run starts while a side run is going, or overlaps one. Both are the owner's scripts. |

## The independent review

A reviewer on Opus 5.5, which had not seen the session's reasoning, read the whole diff before the first push. It found no test that checks less than it did (each answer the agent rig's tests now give is the one the old default gave), nothing wrong with `try`'s new input (a choice, reaching the job only as constants through `env:`), and little added to the blocking checks. What it found, and what was done:

| Finding | Done |
|---|---|
| `replace` by tokens passed over a comment at the start or end of `--from`, so the file's comment stayed and `--to`'s was put beside it: ten doubled or stale doc comments in this very diff came from it. | Refused, with the reason and to match such text as text; the ten places fixed, and one more the step had added since. Test and break. |
| `replace --script` took `a.rs` and `./a.rs` for two files: one edit lost, the file changed, an error reported. And its rename dropped a file's mode (a hook would stop running) and replaced a link with a file. | Each edit's file is keyed by where it really is; the new file takes the old one's permissions; a link's target is what is replaced. Test and break. |
| A retried test's failure text, as quick-junit writes it, held the test's own output. | Its text is its stack trace's, cut at the first nested element. Tests and a break. |
| A fake timer's named wait was also a span of the time profile, so a time limit pending for a whole test showed as the test waiting. | Named for the timeout report only (`watch::naming`). The profile is read once a process, so no test holds this; it is listed under "Gaps" below. |
| `ci-times --write` wrote a section the records do not have ("Both trees on one runner" for both OSes), labelled the base "Main" whatever it was, and would have replaced hand-written notes in a section. | One section an OS under the records' heading; the base named as it is; what it writes is between two marks, a section without them is refused, and a heading in fenced code is not one. Tests and a break. |
| `stress` showed a timeout's first lines of standard error, where the test kit's report comes last. | It starts at the report when there is one. Test. |
| The core `Front` counted a request another client answered, or an interruption withdrew, as one it must answer. | A resolved request is not open, and `turns()` looks again after settling. Test. |
| `try` with the profile and `against` would mix both sides' profiles. | The profile is not set in an A/B. |
| Smaller: the index rule counted a link in fenced code or a comment; `budget_text` wrote the system tier's budgets by hand; `download_root` lost its doc comment; `PendingInterrupt` could be dropped on another thread; the records listed a snapshot that did not change, and a question twice. | Each fixed; the first with a test. |
| Behaviours no test held: the profile's file name, `--like-runner`'s thread count, `write_all`'s rollback, `ci-times`' arguments, the A/B reports' names, the `Front`'s settle before it fails. | Tests for the file name, the A/B section and the resolved request; the others are under "Gaps" below. |

A second review, of the fixes alone, found the path keying sound (a missing file, a link, Windows' `\\?\` forms) and the workflow's expression safe. What it found:

| Finding | Done |
|---|---|
| Two of the fixes' tests passed with the fix taken out: the index's link in fenced code and a comment (two hidden links made one "listed twice" finding either way), and the section search's fenced heading (it came after the marks). | Each case on its own, asserting the message; the fenced heading put before the real one. A break for each, both caught. |
| The docs and help still described one A/B section, and a `--write` that replaces the whole section; nothing said a token match may not begin or end with a comment. | `testing-and-ci.md`, `CONTRIBUTING.md`, `xtask`'s help and `ci-times`' and `replace`'s usage say it. |
| One more doubled doc comment (`signals.rs`), from a fix made after the first review read the diff. | Fixed; a scan of every changed file finds no other. |
| `ci-times --ab --write` wrote the first OS's section before refusing the second. | All sections or none. |
| The core `Front` could name the request that was resolved, not the one still open, when it failed. | It names the one still open. |

## Gaps

- **Item 1's target is not met on the step branch.** It is met by the split of the test job (above), which is the owner's to decide.
- **Not held by a test:** that a fake timer's wait is not a span of the time profile; that `--like-runner` runs at four test threads; `replace --script`'s rollback when a rename fails part way; `ci-times`' refusal of `--ab` with a run; how the A/B reports are found in a `try` run's artifact (exercised by hand on run 43663690282).
- **The runner estimate on another tree** is to be measured on this branch's own A/B, and recorded in `ci.md`.
- **Left from the reviews,** in three open items ([tests-the-fifth-tooling-step-left-unwritten](../../outstanding/tests-the-fifth-tooling-step-left-unwritten.md), [replace-and-the-comments-beside-a-match](../../outstanding/replace-and-the-comments-beside-a-match.md), [a-retried-tests-standard-error-in-the-report](../../outstanding/a-retried-tests-standard-error-in-the-report.md)), none of which makes a check pass what it should catch: no test holds that `turns()` looks again after settling (the resolution is usually drained before the first look); a `--to` that begins with a comment, on code with one, still stacks the two (adding a comment is a real use); on Windows, a read-only file's `.replace-new` may be left if its rename fails; `stress`'s copy of the timeout report's first words is not tied to `watch`'s by a test; and a retried test's standard error is read from its first attempt.
