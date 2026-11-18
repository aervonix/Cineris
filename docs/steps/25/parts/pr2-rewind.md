# Step 25, its second pull request: rewind and fork

Part of step 25's second pull request: `/rewind`, its question, fork from it and from `/sessions`, and the transcript drawn again after a rewind or an undo. The design is [answered.md](../answered.md), "The design, before building", point 4, with the coordinator's answer (f) for the protocol's addition; the rest of the pull request is [pr2-session.md](pr2-session.md). What was built is described in [09-tui.md](../../../architecture/09-tui.md), "Rewind and fork", and the protocol's part in [11-rpc-protocol.md](../../../architecture/11-rpc-protocol.md).

## What was built

| Where | What |
|---|---|
| `cineris-protocol` | `session/rewind_plan` (`SessionRewindPlanParams`, `SessionRewindPlanResult`, `RewindFile`, `RewindTarget`, `RewindFileStatus`, `RewindWhyNot`); `session/rewind`'s `plan`, the mark of the plan shown; the error code `plan_changed`. Additions, no new protocol version; the golden transcript has a plan and a rewind by its mark. |
| `cineris-session` | `CannotRestore` and `FileRestore::cannot`: why a file cannot be put back, by name, for a front end that says it in its own words. |
| `cineris-core`: `rewind.rs`, `core.rs` | The plan read, made as the rewind makes it (`plan_rewind`) under the log's lock, changing nothing; refused as the rewind is (`session_not_stored`, `unknown_item`, `turn_running`). The mark: a short hash of the turns undone and one of what happens to each file (its name, what it goes back to, whether it can be put back and why not). A rewind given a mark is refused before anything is touched when the part it depends on moved: both with files put back, the turns' alone without. |
| `cineris-rpc`: `lane.rs` | `session/rewind_plan` among the reads, beside the lane. |
| `cineris-tui`: `app/state/rewind.rs` | The points (`session/read`, page by page, newest first, the first prompt and steering left out, kept only while the picker is open); the plan and the prompt read whole; the question; the rewind by the plan's mark; the prompt back into the editor after the draft; the inline notice; the transcript read again (a scratch transcript and mapper, then `Kept::replace_before`), the views drawn from it started again and late code coloured again; prompts sent meanwhile held for the read; fork from a point and from `/sessions`. |
| `cineris-tui`: `app/dialog.rs`, `view/dialog.rs` | The rewind's question as a dialog of the application's own (`DialogId::Own`, `Asks::Rewind`, `Keyed::Rewound`), with the dialogs' rules of keys; `1` offered by the rule of a lasting answer, or once the viewer has drawn every row of the plan (`Dialog::seen_whole`). |
| `cineris-tui`: `complete/mod.rs`, `input.rs`, `keys/mod.rs`, `view/menu.rs` | A picker that forks (`Menu::forking`, `Outcome::Forked`), its row (`tab - fork from here`), and the action `fork.fork` in a context of its own. |
| `cineris-tui`: `app/state/commands.rs` | `/sessions` notes a fork (`fork of "…"`, or the parent's id cut short) and forks with `Tab`; a switch is `leave_for`, which a fork of another session uses. |
| `cineris-config`, `cineris-cli` | The `[keys.fork]` table, read by the binary; the reference generated again. |

## Departures from the design, and the choices made

- **The plan's mark.** "A plan that moved between the question and the answer is the core's to refuse" needs the core to know which plan was shown: the plan read gives a `mark`, and `session/rewind` takes it back as `plan`, refused with a new code, `plan_changed`. Both are additions under answer (f). Statuses, not contents, are in the mark: a file edited again after it was shown as edited since does not refuse the rewind, since the person saw it was edited and the core asks about it before it is put back.
- **The plan read refuses what the rewind refuses**, `turn_running` included: a plan under a turn would be stale when it is answered. No test runs a plan under a turn (the core's test rig has no turn to hold open cheaply); the check is beside the rewind's own.
- **The key is `fork.fork`, not `picker.fork`.** An action is named by its context, and the design gives the fork a context of its own (as `toggle.fast` is `/effort`'s): `[keys.fork] fork = ["tab"]`.
- **`cineris-session` changed**, which "What changes outside `cineris-tui`" did not foresee: the reasons a file cannot be put back were sentences only; `FileRestore::cannot` names them, so the protocol carries a value and the interface its own words.
- **The notice has two wordings.** Inline it is the design's (`… what is above this line is no longer part of it.`); in the transcript read again for the viewer and the full screen, where what is above is the active branch, it is `Rewound to before turn 12. The conversation goes on from there.`
- **What is read again replaces the whole kept transcript.** After a switch in `/sessions` the viewer showed the earlier sessions' blocks above the new header; after a rewind it shows the session's active branch alone, then what was kept since the rewind.
- **A prompt sent while the transcript is read again waits for the read,** so that its turn is not read once from the log and again from its events; `/new`, a switch, `/undo`, `/rewind` and a fork wait for it too, and say so, and a second `/undo` waits for the first (a switch during an undo stays allowed, as step 23's fixes have it). The read is milliseconds in process.
- **`/undo` is drawn as a rewind is** (its notice, the transcript read again); its prompt is not put back into the editor, which the design asks only of a rewind.
- **The question counts the plan's turns**, not the picker's: `The plan undoes 3 turns.` The mark holds the plan's turns, so a turn run since the points were read is counted where the person reads it (the review, below).
- **`/sessions`' `Tab` on another session** closes the one open and forks the other at its end by the `--fork` path (opened, forked, closed); on the session open it forks it directly.
- **A row holds at most 512 bytes of its prompt**, on one line, before the menu sanitises and cuts it; the whole prompt is read again (`session/read` of its item) when its point is chosen, for the editor.

## The tests that show each part

- **The plan and the mark, through the real core** (`cineris-core::it rewind_plan`): a plan read twice is the same and changes no file and no line of the log; a file edited since shows as edited and changes the mark, and a rewind by the old mark is refused with nothing changed; as the harness left it again, the rewind by the first mark puts back and removes what the plan said. A conversation-only rewind is refused when a turn ran since, and not when only a file changed. A session with no log has no plan.
- **The lane:** `cineris-rpc lane::tests::a_rewinds_plan_is_a_read_and_the_rewind_a_write`.
- **The interface, through the scripted link** (`cineris-tui app::tests::rewind`): the picker's rows newest first with their time, the first prompt and a steering message no point, pages read in turn, nothing read while a turn runs; one test for each answer (`1`, `2`, `Esc`, and `Enter`, which chooses nothing), each checking the operation sent or that none was; a digit in the question's grace answers nothing; `1` withheld for a plan cut to the window until the viewer has drawn every row, then offered; a refusal of a moved plan sent with the mark and said, with nothing given back; after a rewind the prompt after the draft, the inline notice, and the viewer and the full screen drawn from the branch read again; a prompt sent meanwhile waiting for the read; `Tab` forking from before a point and from `/sessions` (the open session, another one), and a failed fork keeping the session; `/sessions`' fork notes.
- **Hostile text** in every new listing (`hostile_text_in_the_rows_the_plan_and_a_forks_note_is_made_safe`): the picker's rows, the plan's file names in the dialog and in the viewer, and the fork's notes, at 40, 60 and 99 columns, through `testing::problems`.
- **The keys:** `cineris-tui::it bindings::every_default_binding_does_its_action` presses `fork.fork` where it forks, where `Enter` still accepts, and in a picker that does not fork; `cineris-cli`'s `every_table_of_keys_is_read` reads `[keys.fork]`.

## Breaks

`mutations/step-25-rewind.breaks`, 34 breaks (29 with the code, five more with the review's fixes), run with `cargo xtask mutate step-25-rewind` on the laptop: **all 34 caught** by the tests their lists name (the baselines: `cineris-rpc` 50 tests, `cineris-core` 432, `cineris-tui` 641, all passing). The first run left one not caught, `the-mark-leaves-out-the-turns`: the break dropped each turn's id from the hash but kept its line break, so the number of turns still moved the mark; it now drops the turns whole, and is caught. The breaks of earlier steps whose code this part moved (`step-13a/observer-rewinds`, `step-21/tab-accepts-in-the-effort-picker`, two of `step-23-fixes`' switch guards and its resolution, `step-24-panels/a-dialog-leaves-the-focus-on-a-panel`, `step-25/a-finished-block-is-not-kept` and `the-viewer-skips-the-compaction`, two of `step-27-prompts`) find it where it is now, and all 10 are still caught by their tests; `cargo xtask mutate --check` passes.

## The independent review

A reviewer that had not seen the building read the diff (`c39fe2dd..294de103`), read only, against point 4 of the design. Nothing High. What it found, and what was done:

| # | Found | Done |
|---|---|---|
| 1 | Medium. The question said the turns the picker counted, which the mark does not hold: a turn another client ran between the points' read and the plan's would be undone unsaid. | It says the plan's count. `the_question_counts_the_turns_of_the_plan_not_of_the_picker`; break `the-question-counts-the-turns-the-picker-read`. |
| 2 | Medium. The point is the item before the prompt, so a rewind can take what the log holds after the turn before: its last checkpoint when its checks changed a file, and a plan edited between the turns. | The design names that item and the protocol has no other; the plan shows those files and its count. Left open with a recommendation: [a-rewind-to-before-a-prompt-takes-what-followed-the-turn-before](../../../outstanding/a-rewind-to-before-a-prompt-takes-what-followed-the-turn-before.md). |
| 3 | Low. An answer of another kind than the one asked for left the points, the rewind or the read again waiting for good. | Ended as an error is. `an_answer_of_another_kind_ends_the_rewind`; break `an-answer-of-another-kind-leaves-the-rewind-waiting`. |
| 4 | Low. Two reads again of one session could mix their pages (two quick `/undo`s). | Each read has a number, and a second `/undo` waits for the first. `new_a_switch_and_another_undo_wait_for_the_read_again`; break `a-second-undo-while-one-is-on-its-way`. |
| 5 | Low. A prompt sent between the point chosen and the plan is not held. | Left: the core refuses a rewind under a turn, and a plan that moved by its mark; a turn's end closes the question. |
| 6 | Low. The question showed the whole prompt on one line, not its first line. | Its first line, with the ellipsis when there is more. |
| 7 | Low. A fork's notice comes above the new session's header. | Left: it says what happened before the fork is drawn, as a switch's header follows what came before it. |
| 8 | Low, older. The viewer's reveal finds the selected stop's block by its round. | Not this part's code; told to the session that leads the step. |
| tests | A partial view in the viewer could offer `1`; a turn ending under the question, the waits of `/new` and a switch, and `Esc` in the grace had no real check. | `one_is_refused_until_every_file_of_the_plan_was_drawn_whole` checks a partial view first; `a_turn_ending_under_the_question_closes_it_and_rewinds_nothing`; `new_a_switch_and_another_undo_wait_for_the_read_again`; the grace test checks the question is still drawn. Breaks `a-turn-ending-under-the-question-leaves-the-rewind-waiting`, `new-leaves-while-a-rewind-is-read-again`. The plan read's refusal under a turn has no test: the core's rig holds no turn open cheaply. |

## Left open

- [rewind-and-fork-have-no-test-of-the-real-binary](../../../outstanding/rewind-and-fork-have-no-test-of-the-real-binary.md): no pseudo-terminal test of `/rewind` in process or under `cineris attach`, with a recommendation.
- [a-rewind-to-before-a-prompt-takes-what-followed-the-turn-before](../../../outstanding/a-rewind-to-before-a-prompt-takes-what-followed-the-turn-before.md): the point the protocol can name, and what it takes with it, with a recommendation.
