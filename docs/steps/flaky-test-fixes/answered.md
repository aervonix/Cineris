# Answered in the flaky-test fixes, on 2026-11-19

A small fix outside the numbered steps, given by the coordinator: two tests that failed now and then, each found to the cause and pinned with a test that does not depend on chance. It settles the open item `a-verify-test-waits-on-a-timer-nothing-advances` (its text is below) and a nightly failure that had no item. Breaks: [`mutations/flaky-test-fixes.breaks`](../../../mutations/flaky-test-fixes.breaks).

## A row of markdown that begins inside an entity's shown escape

**What failed.** The nightly `properties` job failed on 2026-11-17 and 2026-11-18 (run 44377961318), in `cineris-tui`'s `markdown::tests::token_by_token_commits_what_all_at_once_does_for_random_documents`, case 204, seed `3436097759410096234`. Streamed and whole differed at one row: `  ;` where the whole paragraph shows `  }`, at the end of the escape `\u{fe0f}` that `&#xFE0F;` is shown as, wrapped across two rows.

**The smallest case.** Shrunk from the seed's document by taking out lines, words and characters while the two still differed: the paragraph `x&#x200B;&#xFE0F; ` at a width of 18. Its rows committed as it streams are `◇ x\u{200b}\u{fe0f` and `  ;`; laid out whole, `◇ x\u{200b}\u{fe0f` and `  }`. No split of the stream is needed: a paragraph given all at once commits its rows early too ([09-tui](../../architecture/09-tui.md), "The commit rule"), so every answer that wrapped inside such an escape showed the wrong character until it was drawn again.

**The cause.** The inline parser marks a piece *linear* when each byte it shows comes from the source byte at the same distance; a place inside such a piece maps back to the source, and the stream drops the text of committed rows at a row that begins inside one ([`Laid::clean_break`](../../../crates/cineris-tui/src/markdown/rows.rs)). A piece whose text the parser made (an entity, an escaped punctuation mark, a soft line break) was called linear when its text was **as long as** its source. An entity for a character that acts unseen is shown as its escape (since the review of step 23), and `&#xFE0F;` and `\u{fe0f}` are eight bytes each, as are `&#x200B;` and every other four-digit one. So a row that began inside the escape was taken for a clean cut at the same distance into the entity, the entity's `;`, and the text after the cut, parsed alone, showed `;`. With `&#x200B;&#x200B;&#x200B;` a cut after `\` showed `#x200B;`.

**The fix.** A piece the parser made is linear only where it **is** its source, a tab or a line feed of the source shown as a space (`shows_its_source` in [inline.rs](../../../crates/cineris-tui/src/markdown/inline.rs)). An entity is then never linear, a row that begins inside one is not a clean cut, and the paragraph's text is kept until a later row gives one, as for a link or a code span. What a paragraph shows whole is unchanged (the snapshot is the same); only rows committed early change, to what the whole shows.

**The same mechanism elsewhere.** Searched in the streaming renderer for every place that maps a shown place back to the source:

- **Pieces the parser makes:** entities (fixed), escaped punctuation (two bytes shown as one, never linear), soft line breaks (a line feed shown as a space, linear and right), code spans (compared by content, already right).
- **Escapes of characters written out** (`\u{200b}` for a raw U+200B): the line is sanitised as it is read, so the paragraph's source holds the escape's own text, and it is linear and right.
- **The other uses of `linear`:** `Laid::source_after`, `first_word_end` and `forced_break` read it, and each is right once it is true only of pieces that are their source.
- **What the property test could not see:** `inline::tests::pieces_are_in_order_and_linear_ones_are_their_source` states the rule, but its generator never made an entity whose escape is as long as it is. Its parts now include `&#x200B;` and `&#xFE0F;`, and it catches the fault.

**Tests**, in `markdown::tests`. Each lays its documents out at every width in a range, all at once and in two pieces split at every place, and holds the lines to the document laid out whole:

- `a_row_that_begins_inside_an_entitys_shown_escape_shows_what_the_whole_does`: the shrunk case, and its rows.
- `an_entity_cut_anywhere_commits_what_the_whole_shows`: an entity cut mid-way by the pieces and by a row's end; three in a row; one shown as its character.
- `a_shown_escape_cut_anywhere_commits_what_the_whole_shows`: the escape of a character written out, cut anywhere (the other side of the search above).
- `a_wrap_on_either_side_of_a_shown_escape_commits_what_the_whole_shows`: a row that ends just before or just after an escape, from an entity or the character itself; it checks that some width puts a row's end at each side.

**Measured.** The seed passes alone (`CINERIS_PROPERTY_SEED=3436097759410096234`). The property and the inline parser's properties pass 2,000 cases at the nightly's scale and beyond (`CINERIS_PROPERTY_SCALE=80`, 25 cases × 80, the nightly's 1,250 among them).

## A test of the checks that waited on a timer nothing advances

**What was open** (the item's text):

> `cineris-core`'s `verify::a_users_check_a_managed_ask_rule_asks_about_does_not_run_and_a_users_own_ask_does` (step 26's) timed out at 10 s once in the `test-windows` job of step 27's third pull request (CI run 44019306491, job 133824701940), on a slow runner (a process start of 7 ms, tier 2 at 142% of its budget). The test kit's report: after 8.0 s the test's thread was in "a wait of 120 s on the manual fake timer, made when the fake clock read 2026-11-13T10:00:00Z", and the test was waiting for its turn to end. The reading: a race in the test's logic, not slowness […] an event that was missed, not one that came late, leaves the turn waiting for good. Which event, and whose wait the 120 s is, is not known yet; nor whether the core's own path for the checks has the same race, which would hold a real turn until the 120 s pass.

**Whose wait the 120 s is.** The check's time limit, in the process runner ([`Runner::run_until`](../../../crates/cineris-tools/src/process/mod.rs)): `verify.round_time`'s 120 s, which the core gives the check's command as its `timeout_ms`. It was the only wait in the report. Had the command exited, the runner would have ended at once, or begun a wait of 0.5 s for what the command left running; while it is stopping one, it waits 2 s more, and at its end 1 s for the output. None of those waits was there. So the run was still waiting for the command to end: the shell had not exited after 8 s. That wait ends only when the waiting thread tells the exit, or the time limit passes, and the test's timer is moved by nobody.

**Missed or late.** Missed would need the run to look, find nothing and go to wait just as the thread told the exit, with the wake lost between the two. Reading the code rules it out. The run stores its waker under the lock the command's threads tell under, before it looks at anything (`Waits::poll`). Each thread wakes the stored waker after telling (`Shared::update`). Once an exit is seen, every phase of the run sets a wait of its own, so nothing is left on the time limit alone. Late fits every fact, and the stress showed it:

- **The check started a shell in a test of tier 2.** In its second half (the user's own `ask`), the check ran: Windows PowerShell started and ran `./missing-check`. The test was not a system test, so it had tier 2's 10 s limit. It ran among tier 2's tests, two at a time on Windows, on a runner whose tier 2 was at 142% of its budget. That breaks the rule the file states at its top ("A test that runs a check starts a shell, and is named `sys_…`").
- **Its time is the shell's.** On `main` its passing runs on 2026-11-18 took 2.4 s and 3.1 s (SLOW in both). Its first half starts nothing and takes no time to speak of.
- **Under load it fails every time.** A `try` stress of `main`'s `verify` tests on Windows (run 44441142003: 200 repeats asked, `--cores 2 --load 6`, `--strict`) timed out in this test in each of the 152 repeats it reached before the job's 40-minute limit, at 10.3 to 10.7 s. The file's other 33 tests passed in every repeat.
- **Its times run up to the limit.** On a scratch branch off `main`, three variants ran ten times each under the same load, their output shown (try run 44446314773):

  | Variant | Passed, in seconds | Timed out |
  |---|---|---|
  | The test as it was (`./missing-check`) | 4.6, 10.0, 10.07, 10.08, 10.19 | 5 times, at 10.3 to 10.6 s |
  | The same, with `PSModulePath` an empty folder | 7.5, 9.0, 9.0, 9.1, 9.1 | 5 times |
  | The same, with a check program that exists and fails | all 10, at 2.4 to 9.1 s | never |

  A missed exit would split the runs into quick passes and timeouts. Instead, passes run right up to the limit and the timeouts sit just past it. Each timeout's report again shows only the 120 s wait.
- **What costs the time.** Windows PowerShell under that load: from 2.4 s for a program it finds, and more for a command it cannot find, which takes its error path. An empty `PSModulePath` does not change that, so the search of the modules is not the cost.
- **At the desk** (the desktop's Windows, no load, eight starts each), the costs are smaller:
  - `exit 1`: 472 ms the first time, then 117 to 158 ms;
  - a found `check.cmd`: 129 to 172 ms;
  - `./missing-check`: 440 ms the first time, then 158 to 170 ms.

**Whether a real turn is affected.** No. The core's path for the checks has no race. A real turn waits on the real timer, and the check's run ends when the shell does, however late. A shell that is slow makes the turn's check slow, never stuck. The 120 s wait can hold a turn only when its command does not end, which is what the limit is for. With the test's manual timer, a late end became a wait that only the 10 s limit stopped.

Left as it is, and bounded: on Windows, a process that a Job Object still lists after it has exited keeps a run that has seen its command's exit waiting for the 0.5 s it gives what a command left running (the review of these fixes). With a real timer that is half a second. In a test on a manual timer it would be a wait nobody ends, and the test kit's report would name it as a wait of 0.5 s.

**The fix** is in the test. It is split in two, by what each starts:

- `verify::a_users_check_a_managed_ask_rule_asks_about_does_not_run` holds the line back by the managed configuration's `ask`, and starts nothing. It stays in tier 2 on both systems, and now also checks that the turn's verification is skipped.
- `verify::sys_a_users_own_ask_rule_does_not_hold_back_their_check`, Unix only, runs the line where only the user's own `ask` rule would ask, as a system test with tier 3's 60 s limit. Its check is a program in the workspace that passes, so the turn ends with the checks passed, and the model is sent no third request its script does not have (the failing `./missing-check` had sent the turn back to the model). It is Unix only, as the file's top says of the tests that start a shell: the rule it tests is the policy's and the verifier's, the same code on both systems. Windows' tier 3 is at 96% of its budget, and Windows' tier 2 loses its slowest core test. **What Windows loses:** a run there of the user's own `ask` letting the line run; the managed half, which holds it back, still runs there.

The three breaks of [`mutations/tooling-10-decisions.breaks`](../../../mutations/tooling-10-decisions.breaks) that named the old test name the half that catches each now; the one caught by the system test runs on Linux (`os: linux`). The item's `- Flaky:` line went with its file.

**A test of the runner, for the core's side.** `cineris-tools`' `process::tests::an_exit_or_an_end_told_after_the_run_went_to_wait_wakes_it` lets the run look and go to wait, with a time limit on a manual timer that never ends. A thread then tells the command's exit, then the end of its output, and in the other order too. The test checks that each telling wakes the run, and that the run sees it. Two breaks show it bites: a thread that tells without waking, and a run that waits with no waker stored. It holds the wake, not the order inside `Waits::poll` (the waker stored before the look, under the threads' lock), which the reading above settles and which no test can reach without a hook between the two; nor `Run::step`, whose phases each set a wait of their own once the exit is seen.

**Is the mechanism elsewhere.** The mechanism is a test not named `sys_` whose turn starts a command. A search of the text cannot tell a call that runs from one a rule refuses, so it was looked for at run time. With a line added for the run alone, the process runner aborted any test not named `sys_` or `perf_` that started a command. Every such test of the workspace was then run at the desk (4,477 tests). One aborted: this one. 23 tests timed out in the run, all in `cineris-base` and `cineris-llm` while the desk's disk stalled. Those crates sit below `cineris-tools` and cannot start a command through it. Windows-only tests did not run there. The line was taken out again.

**Measured.** STRESS-NUMBERS
