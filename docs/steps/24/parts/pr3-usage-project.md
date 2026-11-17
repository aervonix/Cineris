# Step 24, its third pull request: the project and usage panels

The project and usage panels, `/project` and `/usage`, screen 4's usage line, and the measurements of the panels ([answered.md](../answered.md), "The design, before building", points 2, 3, 4 and 7). The commands that need no panel and the live views are this pull request's other part, [pr3-views.md](pr3-views.md); the panel mechanism is the second pull request's, [pr2-panels.md](pr2-panels.md).

## What was built

- **Three more hooks of the panel mechanism** (`app::panels::Panel`), and none of its rules changed: what a panel asks of the core when a command opens it (`summoned`, which `/plan` has no use for); the focus taken or moved inside it (`focus_taken`, `moved`), which forget a question asked in a row; and whether the focused item's rows were all in the window in the frame on screen (`shown_whole`), which restarts the readiness when that is new. A panel hears of a refusal of what it sent (`failed`), and `when_opened` is given the session's state. A key that changes what the focused item shows starts the readiness again at once (`Panels::act`).
- **The project panel** (`app/panels/project.rs`, `view/project.rs`). The entries in use and waiting for this person's review, `project/list`, asked as a session opens (a failure that says the core keeps no project there, `not_available`, is not said) and again by `/project`, and changed by `project.updated`. It opens by itself, collapsed, when entries wait; `/project` opens it focused; it closes when nothing waits and `/project` did not open it, and with `x`. Each entry is a row, and the focused entry's detail follows it, wrapped and never cut: the command whole, what it is for, its folder and whether it only reads; the text; where it came from; its standing as the engine treats it (an accepted command in a project not trusted as it is still asks, from the session's trust); what accepting it keeps it until; its check, `not run in this build`; the evidence. `y` accepts, `e` edits, `n` turns off, `d` deletes after asking in the row, `p` promotes: one `project/update` each. **`y` is taken only when the focused entry's whole detail was drawn in the frame on screen;** otherwise the hint row says to make the window taller, and `y` says so and sends nothing. A key is offered, and taken, only where the core acts on it.
- **The usage panel** (`app/panels/usage.rs`, `view/usage.rs`). `/usage` opens it focused and asks one `usage/report` and the user's `usage.refresh` (`config/get`), the history `reading…` until the report comes; a provider's limit with less than a fifth left opens it by itself as one line in the warning colour, which closes it again when no limit is near. It shows the session's usage (`usage.updated`), the history for a range (`tab`: today, this week, this month, all time) by a grouping (`g`: provider then model, project, speed, provider within project) with the turns' time and the zone the days are counted in, the report's notes, and each provider's limits. Its parts are kept in the panel, made again only when what they come from changes. `r` asks one refresh at a time; with none in `usage.refresh`, it says so and sends nothing. A report the core refuses (`stats_newer`) is said in the panel too.
- **`/project` and `/usage`** moved from `NOT_YET` into `COMMANDS`.
- **Screen 4's usage line** is drawn by the panel (`screen_usage` in the screens test), and no row of any screen is counted as not built: [tui-look/README.md](../../../architecture/tui-look/README.md) says each cell that changed.
- **Closes** `statistics-a-build-cannot-use-are-said-only-in-the-log` (its file deleted): the panel says what `cortex stats` says, from `UsageReport.notes` (the first part's), statistics turned off (`recording`), and a refusal (`stats_newer`).

## Where it departs from the design

1. **Screen 4's line names the provider and says what the limit counts:** `anthropic fast tier: 18% of the output token limit left, resets 15:00`, where the drawing said `fast tier: 18% of the hourly output limit left`. A limit as the protocol carries it has what it counts and when it resets, not its period; and with more than one provider a line that names none does not say whose.
2. **The open's `project/list`** is asked for every session, new or stored, so that what waits from earlier sessions shows as the session starts. It may be the first read of the project's files (the loop's first turn was), and reads as `cortex project` does: an entry that expired may be marked, waiting for the store's lock, on the blocking pool beside the lane. 14-adaptation and `cortex-core`'s `assemble.rs` say so.
3. **The project panel's least rows** are its first row and the focused entry's own row: its detail may be cut, and then it offers no `y`. The trait's documentation allows this.
4. **No delete is assumed from a result:** an entry goes when `project.updated` says it was removed.
5. **The usage panel's rows hold the history as a table, not the turns' time and tools per row:** the range's time is one line under the table. The report's `tools` are not shown.
6. **The year measured has eleven models, not twelve:** the statistics' generator (`cortex-stats`' `synthetic`) knows eleven of the catalog's, of the five providers. The application's half is measured with twelve.
7. **`r` reads `usage.refresh` with `config/get` as `/usage` opens the panel,** and again after a reload that changed a `usage.*` key; until then, `r` asks the core, which answers with no provider when the list is empty, and that is said.

## The early review of the project panel, and what each finding changed

A focused review of the accept and promote paths (4c31a16e). It found sound: `whole` matches the frame on screen, ids are never reused, `project.updated` between a frame and a key re-points the focus and restarts the readiness, bursts, held keys and repeats never act, no row is cut, every outside field is sanitised, promote sends one update and writes nothing itself, results for a session left are dropped, and `Panels::act`'s new `observe` changes nothing for the plan and review panels.

| Finding | What changed | Test | Break |
|---|---|---|---|
| 1. The delete question outlived a move of the focus: `y` on the next entry was lost, and coming back asked again with the grace long passed | The `moved` hook forgets a question of another entry; in `act`, a question of an entry no longer focused takes no key | `the_delete_question_is_forgotten_when_the_focus_moves`, `a_question_of_an_entry_gone_takes_no_key_of_the_next` | `the-question-outlives-a-move`, `the-question-takes-the-next-entrys-key` |
| 2. A delete never sent could remove the entry, and any key cleared a real one's | `deleting` is gone: an entry goes when `project.updated` says it was removed | `a_delete_held_back_leaves_the_entry`, `d_asks_in_the_row_y_then_deletes_once_ready_and_any_other_key_keeps_it` | (the state is gone) |
| 3. The purpose was only in the row, which is cut | The detail says `for test · in crates/parser · may change files` | `the_focused_commands_detail_is_whole_and_wrapped_never_cut` | `the-purpose-is-not-in-the-detail` |
| 4. The expiry shown was not the one accepting gives | An entry `y` would accept says `accepting keeps it until …` (a learned entry its kind's lifetime from now), and a proposal past its date says it expired | `each_origin_and_standing_is_said_as_cortex_project_says_it` | `accepting-renews-for-the-wrong-time`, `an-expired-proposal-says-nothing` |
| 5. `project/list` on every open | Kept (departure 2); `not_available` is not said for the open's own list; the comment and 14 say what it reads | `no_project_kept_says_nothing_as_the_session_opens` | `no-project-kept-is-said-on-every-open` |
| 6. No grace when the detail first became whole | `shown_whole` says when it is new, and the readiness starts again | `a_detail_first_drawn_whole_has_its_grace` | `a-detail-first-drawn-whole-has-no-grace`, `the-mechanism-ignores-a-new-whole-detail` |
| 7. The hint and the standing said what the core would not do | `p` only on a private entry in use and accepted; no `e` or `d` on the repository's; this client's prompt counts as a turn running; `y` on an accepted entry that expired; an accepted command in an untrusted project still asks | `p_promotes_an_accepted_entry_and_while_a_turn_runs_the_hint_says_so_and_the_refusal_is_said`, `an_entry_of_the_repositorys_file_offers_no_edit_or_delete`, `a_prompt_sent_says_promote_waits_for_the_turn`, `what_a_key_would_do_is_what_the_core_does`, `the_detail_says_what_the_sessions_trust_lets_a_command_do` | `promote-offered-on-an-entry-not-accepted`, `a-shared-entry-is-edited-here`, `a-prompt-is-no-turn-running`, `an-expired-entry-cannot-be-accepted-again`, `an-accepted-command-in-an-untrusted-project-runs`, `the-sessions-trust-is-not-followed` |
| 8. Tests | The detail compared whole with the command; the question with `Up` and `Down`; a dropped delete; a result for a session left; a change between the frame and `y`; the hostile purpose and check command; the focus kept after `n` | `a_result_for_a_session_left_is_dropped`, `an_entry_changed_between_its_frame_and_y_holds_y_back`, and those above | |
| 9. Docs | 09-tui's "The panels as built" has the project and usage panels; `Panel::least_rows` says a kind may count its item's first row alone | | |

## The pull request's full review, and what each finding of this part's changed

| Finding | What changed | Test | Break |
|---|---|---|---|
| 1. The usage panel a near limit opened never closed once the limit's reset had passed: the panel stayed open with no rows, and `Ctrl+P` reached it | The panel remembers whether a limit was near at the last update, opens when one becomes near, and closes on any update with none near; and a new hook, `Panel::lapsed`, has the mechanism close it before the panels are laid out when the time passed it by with no update | `a_limit_past_its_reset_closes_the_panel_it_opened` | `a-reset-passed-keeps-the-panel-open`, `a-lapsed-usage-panel-is-not-lapsed`, `the-mechanism-keeps-a-lapsed-panel` |
| 2. The project panel said `reading…` for ever after a refused list | It has `failed`: the reading ends, and the refusal is said where the entries would be; the open's own `not_available` tells it too, and is still not said | `a_refused_list_ends_the_reading_and_says_why` | `a-refused-project-list-reads-on` |
| 3a. The question of a deletion said `y` whatever the key | It names the key the map binds to `focus.accept` (the mechanism's `Draw` carries the key map) | `the_delete_question_names_the_bound_key` | `the-question-names-y-whatever-the-key` |
| 3b. `tab` and `g` changed the usage panel's focused row, and the readiness started again, so a quick second key was typed into the prompt | Its rows are one item to the mechanism (one key, no content): its keys change its own view, and nothing acts on a row | `a_key_straight_after_the_views_change_is_taken` | `a-view-key-holds-the-next-back` |
| 3c. Under 60 columns a mixed cost could be cut at the row's end, its `est.` lost | In a narrow window costs put their marked parts first (`$1.92 est. + $41.87`), the output column goes before the cost would be cut, and a total that does not fit beside its word follows it, cut at its end | `a_mixed_cost_keeps_its_basis_in_a_narrow_window` | `a-narrow-total-loses-its-mark`, `a-narrow-row-keeps-its-output` |

## Tests

All tier-1 unit tests of `cortex-tui`, with no real waiting:

- `app/tests/project.rs` (21): opening by itself and with `/project`, each key's `project/update`, `y` only on a detail drawn whole, the window too short and then taller, the question and its `y`, promote and `turn_running`, and the review's findings above; the hostile frame.
- `app/tests/usage.rs` (7): `/usage`'s one round trip and the frame after the answer with the rows (the tier-1 test of the measured path, with a small year: a second round trip, or a read of anything else, fails it); the warning line opening and closing it; `tab` and `g`; `r` one at a time and with none turned on; a refused report; the hostile frame.
- `view/project/tests.rs` and `view/usage/tests.rs`: snapshots of each panel collapsed and focused at 60, 80 and 120 columns, in both glyph sets, with colour and without, light and dark (`snapshots/project.txt`, `snapshots/usage.txt`); the detail never cut; each standing and origin; costs with their basis; the columns dropping; limits with their age; the warning line; hostile text in every field from outside.
- `tests/it/screens`: screen 4 compared whole; the skipped-row test takes a row for a later step's itself, since none is left.

## Measurements

Release builds, median / 99th percentile; the desktop's through `rx` (with other sessions' runs on it), a hosted Windows runner's by `try` (`perf -p cortex-tui`, run 43978345142; `perf -p cortex-stats`, run 43978349712). The budgets are in [performance-budgets.md](../../../performance-budgets.md), whose rows have these numbers.

| What | Desktop | Windows runner | Budget |
|---|---|---|---|
| **The usage panel with a year of history, the core's part:** `usage/report` as the core makes it from a generated year's aggregates (365 days, 62,835 requests, five providers, eleven models, twenty projects, two speeds; aggregates of 3.5 MB), every range and grouping the panel asks for, 320 reports (`perf_a_years_report_for_the_usage_panel`, `cortex-stats`) | 18.6 / 20.6 ms (19.4 / 23.8 in a first run beside a loaded stress run) | 33.9 / 39.9 ms | ≤ 16 ms for the whole path |
| — of it, the aggregates read / brought up to date / the report made (desktop) | 11.0 / 3.2 / 4.4 ms | | |
| **The usage panel, the application's part** (`perf_the_usage_panels_first_frame_with_a_year`, `cortex-tui`, twelve models): the key to its frame (the panel open, `reading…`) | 0.08 / 0.14 ms | 0.17 / 0.29 ms | |
| — the answer to the frame with its rows | 0.14 / 0.56 ms | 0.32 / 1.31 ms | |
| **A key's frame with the four panels open**, idle (`perf_a_keys_frame_with_the_four_panels_open`, `cortex-tui`, 2,000 keys) | 0.020 / 0.052 ms | 0.043 / 0.093 ms | ≤ 2 ms |
| — while an answer streams | 0.054 / 0.106 ms | 0.113 / 0.206 ms | ≤ 4 ms |

**The usage panel's year is over its budget, in the core** (rule 11, said and not cut: the year is the one asked for). The panel opens in the key's own frame, its history `reading…`, and the application's part is well under a millisecond; the rows come when the core's report does, one to three frames later. Most of the core's time is reading the aggregates: a heavy year's day buckets are 3.5 MB of JSON, where the budget's row expected kilobytes. **Recommendation:** the core keeps the aggregates it last read, with the file's identity, and reads the file again only when another process saved it (11 of the 18.6 ms on the desktop); and, if that is not enough on Windows, keeps a year's days in a more compact form than JSON. Neither is this part's to change (`cortex-core`'s `usage::report`, `cortex-stats`' store).

The first measurement of the answer's frame found 3.1 ms at the 99th percentile on the laptop: the usage panel made its parts again for each item the mechanism asked for when it followed the focus, so the work grew with the square of the rows. It keeps them now, made again only when what they come from changes (1.7 ms on the laptop, 0.56 ms on the desktop).

## Breaking the code on purpose

`mutations/step-24-panels2.breaks` holds 39 breaks: the project panel's `y` and its question, what the core would do, the detail, the open's list, the usage panel's asks, its limits, costs and columns, and the mechanism's new hooks. Run with `cargo xtask mutate step-24-panels2 --jobs 2` on the desk's side slot: **37 did what their list says, and 2 were flagged.**

- `y-accepts-what-has-nothing-to-accept` was caught by no test: no test pressed `y` on an entry in use and accepted. `y_on_an_entry_in_use_and_accepted_sends_nothing` now does, after the grace its detail drawn whole has.
- `a-projects-label-is-drawn-raw` was caught by the view's hostile test and not by the application's, which never groups by project (the panel asks for provider and model first). Its list names the view's test alone.

Run again, each is caught by the test its list names. The second part's and the coordinator's lists (`step-24-panels.breaks`, `step-24-views.breaks`) still apply to the code as it is (`mutate --check --tests`).

## Stress

- `cargo xtask stress -p cortex-tui --count 50`, on the desk's side slot: 532 tests, 50 times each, seeds 0 to 49: every test passed every time.
- `cargo xtask stress -p cortex-tui --count 50 --cores 2 --load 6`: the same, every test passed every time.

## Checks

- `cargo xtask ci` at the desk on a8dc0198 with this record and the budgets' rows on top: formatting, the rules, the breaks, the generated files, clippy, licences, tiers 1 and 2 (4136 tests) and the system tests (271) pass. The tier budgets were not judged: the desk's disk stalled for 59% of the tests' run. `cortex-tui`'s tier 1 was 5.7 s of it above the start of the tests' processes, against 4.6 s at the second part's last run.
- `cargo xtask api-diff --against worktree-step-24-views --features cortex-tui/test-kit -p cortex-tui`: 59 public items added (`view::project` and `view::usage`), none removed or changed.
- `cargo xtask mutate --check --tests`: every break of every list applies, and every test a `caught-by` names exists.
