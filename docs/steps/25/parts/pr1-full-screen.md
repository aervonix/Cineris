# Step 25, its first pull request: the full-screen presenter and the mouse

The first of step 25's three pull requests: the full-screen presenter, the mouse in it and in the viewer, code highlighted again at a new width, and the measurements of both presenters. The design is [answered.md](../answered.md), "The design, before building", with the coordinator's answers; what this part built is described in [09-tui.md](../../../architecture/09-tui.md), "Full screen as the alternative" and "The mouse".

## What was built

| Where | What |
|---|---|
| `cineris-term`: `full.rs` | `Full` and `Screen`: the whole screen in one write, inside a synchronized update, only the rows that changed, each placed by its number; a resize, a redraw and a failed write have the next frame write every row. |
| `cineris-term`: `input.rs` | The mouse decoder: mode 1006's reports, and mode 1000's own form while the mouse is on (`Decoder::set_mouse`); `Event::Mouse` with `Mouse`, `MouseKind`, `Button`, `Wheel`. |
| `cineris-term`: `terminal.rs`, `sys/` | `Terminal::set_mouse` and `mouse`: modes 1000 and 1006 on with the alternate screen and off before it, at the end and on a resumption; the four emergency restores with the alternate screen on turn them off; on Windows the console's mouse input with the alternate screen (QuickEdit off meanwhile), a mouse record written as the SGR report the decoder reads, and no mode asked of the terminal (`Capabilities::vt_mouse`, departure 7). The terminal demo's `mouse` option names each mouse event. `draw_screen` is gone. |
| `cineris-testkit`: `vterm.rs`, `pty.rs` | Modes 1000 and 1006 in the virtual terminal's `Modes`; `Session::send_key` and `key_bytes`, a named key sent as Windows Terminal sends it to a pseudo-console (win32 input records) and as its xterm sequence elsewhere. |
| `cineris-tui`: `transcript/kept.rs`, `transcript.rs` | `Kept`, the finished blocks, and `Transcript::keep_finished`, `take_finished`, `open_rows` (what an answer still streaming has committed) and `rewrap_open`. |
| `cineris-tui`: `markdown/mod.rs` | `Markdown::keeping`, `take_source`, `render_source` and `code_lines`, `restarted`; `Source` and `CodeColours`. |
| `cineris-tui`: `app/scroll.rs` | The window over the kept blocks and the live rows, which follows the end until moved up, made only near what it shows. |
| `cineris-tui`: `app/search.rs` | The search's worker. |
| `cineris-tui`: `app/state/screen.rs`, `recolour.rs`, `viewer.rs` | The full-screen view (its frame, keys, mouse, search and panels' column); code coloured again; the viewer on the kept blocks and the window, its keys in the key map. `App::frame` is split into `prepare` and `bottom`, which both presenters call. |
| `cineris-tui`: `keys/mod.rs` | The `viewer` and `screen` contexts and their actions; `Gesture`, `Binding`, `parse_binding`, `KeyMap::lookup_gesture` and `gestures_of`. `normalize` reads an upper-case letter alone as `Shift` with the lower-case one, as it did with `Ctrl` or `Alt`. |
| `cineris-tui`: `view/round.rs` | `Placement::Screen`: a committed box a click or the search turned over, and the search's border label. |
| `cineris-tui`: `app.rs` | The loop draws the viewer and the full-screen view through `Full` on the alternate screen, asks for the mouse on every terminal it is given, and draws every row when the alternate screen is entered again. `Settings::presenter` and `Settings::mouse`. |
| `cineris-config`, `cineris-cli` | `tui.presenter`, `terminal.mouse`, `[keys] viewer` and `screen`; `--fullscreen`. |

## Departures from the design, and the choices made

1. **A kept command's output is kept to 64 KiB, not 16.** The design had 16 KiB, with the whole read from the core when the box is opened; that read is not built, and 16 KiB without it would show less in the viewer than step 23 did. So the bound is what a running command keeps, and the read and the lower bound are an open item, [a-kept-box-holds-only-the-end-of-its-output](../../../outstanding/a-kept-box-holds-only-the-end-of-its-output.md), with the worst case's arithmetic. Diffs are not bounded yet either.
2. **No scroll region.** The design had `Full` scroll the transcript's rows inside a region while it follows the end, if a measurement showed it paid. It does not: a frame that follows a stream writes 122 bytes at the median and 4 KB when a committed row moves the area up (measured below), within one read of a pseudo-terminal, so `Full` stays a writer of rows.
3. **Search covers what is kept.** An answer still streaming is searched once it is kept.
4. **The panels' column at 150 columns is 45 wide**, by the design's formula (40, and one for each two columns past 140); screen F5 drew 40, which is the column at 140.
5. **`Placement::Screen`** is a new placement of `view::round`, for a box a click or the search turned over in the full screen: open with all its detail as in the viewer, closed with the committed box's `ctrl+o expands`, and `opened by the search` while the search holds it open. Existing placements draw as before.
6. **The viewer's keys moved into the key map** with the same defaults; `G` is bound as `shift+g`, which `normalize` now matches for a terminal that sends the upper-case letter alone.
7. **On Windows the terminal is not asked for the mouse by its sequences.** The design had the console's mouse input with the alternate screen, and left which path a pseudo-console takes to be measured. Measured on a hosted runner (`cineris-term::it system::sys_the_mouse_reaches_the_program_on_the_alternate_screen`, with the terminal demo's new `mouse` option, try runs 44355791101, 44357459502 and 44358002758): with modes 1000 and 1006 written, ConPTY passes them on to the terminal (`?1000h`, `?1006h`, beside its own `?9001h` and `?1049h`) and then gives the terminal's report to the program as typed keys (the demo read `[<64;10;5M` as text, its last key `M`), which under Windows Terminal would type every turn of the wheel into the prompt; with the console's mouse input alone (QuickEdit off), ConPTY asks the terminal for nothing. So `Capabilities::vt_mouse` is off on Windows: the terminal is asked for nothing, keeps its own wheel and selection, and the keys do what the mouse does; the old console host still gives the program its mouse records. The system tests check that ConPTY is never asked for the mouse, and use `PageUp` there for the wheel. [the-mouse-under-windows-terminal](../../../outstanding/the-mouse-under-windows-terminal.md) is open, and the Windows Terminal checklist of the third pull request checks the wheel and the selection by hand. **Superseded by the follow-up** ([pr1b-windows-mouse.md](pr1b-windows-mouse.md)): a reviewer found that the console's mouse input with QuickEdit off makes ConPTY ask the terminal for modes 1003 and 1006 in one sequence, which the probe here, looking for `?1003h` and `?1006h`, could not see; a hosted runner confirmed it.

## The owed items

- `highlighted-code-drawn-again`: **closed** (its file deleted). Shown by `app::tests::screen::code_committed_plain_is_coloured_once_its_colours_come`, `highlight::tests::a_kept_answer_records_its_code_colours_and_the_lines_that_were_late` and `markdown::tests::a_kept_answers_code_keeps_its_colours_at_another_width`.
- `the-bincode-advisory-ignored-for-syntect` ("the next step that touches highlighting, otherwise step 25"): looked at on 2026-11-17; `syntect` 5.3.0 is still its latest release, on `bincode`, so the ignore stays and the item is left open.

## The tests that show each part

| Part | Tests |
|---|---|
| The same rows under both presenters | `app::tests::screen::the_same_rows_are_drawn_under_both_presenters_frame_by_frame` (frame by frame while two turns stream, at 60, 80 and 120 columns, styles included); `transcript::tests::every_frame_of_a_stream_is_drawn_the_same_by_both_presenters`, `kept_blocks_draw_again_the_lines_that_were_committed` |
| Resize | `app::tests::screen::a_resize_lays_the_transcript_out_again`, `an_answer_resized_while_it_streams_is_laid_out_again`; `transcript::tests::an_answer_resized_while_it_streams_is_laid_out_again`; `cineris-term::it full::a_resize_to_any_width_draws_the_screen_whole_and_right`; inline's are step 20's |
| A frame's properties | `cineris-term::it full::every_frame_is_one_write_with_nothing_cleared_and_the_right_screen`, and `full::tests` |
| Following the end | `app::tests::screen::the_window_follows_the_end_until_it_is_moved_up`; `app::scroll::tests` |
| Search | `app::tests::screen::ctrl_f_searches_the_transcript_and_goes_between_matches`, `a_match_inside_a_closed_box_opens_it_while_it_is_current`; `app::search::tests` |
| Panels beside | `app::tests::screen::panels_sit_beside_the_transcript_on_a_wide_window` |
| The mouse | `cineris-term`: `input::tests::mouse_reports_in_the_sgr_form`, `mouse_reports_in_the_older_form_are_taken_whole`, `a_mouse_report_decodes_the_same_at_every_split`, `terminal::tests::the_mouse_is_on_only_with_the_alternate_screen_and_off_on_every_way_out`, `a_resume_turns_the_mouse_on_again_with_the_alternate_screen`, `the_older_mouse_report_is_read_only_while_the_mouse_is_on`; `cineris-tui`: `app::tests::screen::the_wheel_scrolls_and_a_click_opens_or_closes_a_box`, `the_viewer_scrolls_with_the_wheel_and_a_click_opens_a_box`, `the_inline_screen_takes_no_mouse_and_no_full_screen_keys`, `keys::tests::the_viewer_and_the_full_screen_have_their_keys_and_the_mouse`, `a_gesture_is_bound_only_where_the_mouse_is_reported` |
| Ways out | `app::tests::screen::every_way_out_of_the_full_screen_leaves_the_mouse_and_the_alternate_screen_off` (quit, the core gone, the terminal failing, a suspension and the screen taken again), `the_viewer_turns_the_mouse_off_as_it_returns_to_the_main_screen`; the real binary: `cineris-cli::system tui::sys_the_full_screen_runs_a_turn_scrolls_and_leaves_the_terminal_as_it_was` (the shell's verdict compares the terminal's or the console's modes before and after) |
| The inline presenter unchanged | every snapshot and golden screen of `cineris-tui`, unchanged; `COMPARED` is as it was |

## Measurements

**A full-screen frame after 200 turns** (`perf_a_frame_in_the_full_screen_presenter`, release build, the desktop, 1,000 frames, a window of 120 by 40): the application's frame and `Full`'s bytes for it.

| | Median | 99th percentile | Bytes, median | Bytes, most |
|---|---|---|---|---|
| Idle, following the end | 0.058 ms | 0.106 ms | 0 | 4,821 (the first) |
| Idle, scrolled into the middle | 0.058 ms | 0.070 ms | 0 | 3,811 |
| Streaming, a piece before each frame | 0.120 ms | 0.203 ms | 122 | 4,063 |
| Streaming, the answer 2,500 paragraphs long and still open (after the review) | 0.367 ms | 0.508 ms | 92 | 2,842 |

The frame budget (performance-budgets.md) holds with room to spare. A long answer still open costs more per frame, because its committed rows are put together for each frame: [a-full-screen-frame-copies-the-open-answer](../../../outstanding/a-full-screen-frame-copies-the-open-answer.md).

**200 turns in each presenter** (`perf_two_hundred_turns_in_each_presenter`, the musl `dist` build, a workspace of 20,000 files of real code, the desktop; the table is in [performance-budgets.md](../../../performance-budgets.md), "Memory"): inline 30.4 MB resident after the turns, 38.2 MB at the peak, against 40; full screen 30.5 MB, 33.2 MB after scrolling, resizing and searching, 39.0 MB at the peak, against 80. The inline peak is near its budget, but it is the index's first pass, before any turn (34.2 MB), not the session.

**The tiers, both trees on one runner** (`try` with `-f against`, the median of 5 runs each; each tier's time above the start of its processes):

| Tier | Windows, PR (main) | Change | Linux, PR (main) | Change |
|---|---|---|---|---|
| 1, unit | 9.3 s (9.0 s) | +0.4 s | 22.8 s (22.4 s) | +0.4 s |
| 2, integration | 22.4 s (22.5 s) | −0.1 s | 33.9 s (33.5 s) | +0.3 s |
| 3, system | 53.2 s (53.4 s) | −0.3 s | 30.2 s (29.9 s) | +0.3 s |

Windows is run 44360402752 at 58d79545 (5 ms a process start), Linux run 44353520501 at 513fd21 (6 ms; the commits since then add a Linux system test of the mouse and two unit tests, and change no Linux code). Main's Linux tier 1 was over its 20 s warning line already; the fail line is 30 s. The growth is the 53 to 56 new tests (`cineris-tui`'s unit tests +0.48 s on Windows, +0.77 s on Linux; `cineris-term`'s `full` tests +0.33 s in tier 2). One existing test moved on both systems, `view::input::tests::the_cursor_is_after_the_text_and_never_on_the_edge` (+0.033 s and +0.068 s): it makes 660 inputs, each with the default key map, which this pull request's viewer and screen keys made longer to parse. `KeyMap::defaults` now parses the table once and copies it (198 µs a map against 0.3 µs, a debug build at the desk), and the test went from 0.33 s to 0.21 s there.

## The independent review

A reviewer that had not seen the building read the whole diff (`24305e3d..8e6a45e3`), and nothing was built or run for it. What it found, and what was done; each fix has a test that fails without it (checked by undoing the fix) and a break in `mutations/step-25.breaks`, `mutations/step-23-fixes.breaks` for the eighth.

| # | Found | Done |
|---|---|---|
| 1 | High. A search's answer, or code's late colours, that came while nothing else happened drew no frame: the row stayed `searching…` until the next key. | The loop's drain takes both and says there is a change. `app::tests::screen::a_search_answer_that_comes_while_idle_schedules_a_frame`; `code_committed_plain_is_coloured_once_its_colours_come` asserts the drain. |
| 2 | High. Typing or pasting into the full screen's search did not count as typing, so a slow typist's next key answered a dialog that had just come; and the search row was drawn over the dialog's own. | Both count as typing (`not_taken`), and the row is not drawn under a dialog. `app::tests::dialogs::typing_into_the_full_screen_search_does_not_answer_a_dialog`, `a_paste_into_the_full_screen_search_holds_a_dialog_that_comes`. |
| 3 | High. The viewer opened while an answer streamed lacked the rows the answer had committed. | The viewer draws them before the live rows, as the full screen does. `app::tests::screen::the_viewer_opened_mid_stream_shows_what_the_answer_committed`. |
| 4 | High. `[keys] viewer` and `[keys] screen` never reached the key map. | Every table is read (`key_tables`), and a test compares the tables read with the ones the schema declares and looks a binding up in each new one: `cineris-cli` `interactive::tests::every_table_of_keys_is_read`. |
| 5 | Medium. A dialog that filled the window lost its keys' row in the full screen. | The transcript may have no row. `app::tests::dialogs::a_dialog_that_fills_the_full_screen_keeps_its_keys_row`. |
| 6 | Medium. On Windows, a read that brought only ignored records (mouse moves with no button) waited the whole timeout again. | Each system's wait waits to a deadline (`sys::millis_left`); its twin on Unix, a `poll` a signal interrupted, too. `sys::tests::a_wait_to_a_deadline_takes_what_is_left_of_it`. |
| 7 | Medium. The search test could wait 8 s for an answer a frame had taken. | `settle_search` says so when the answer was taken already. |
| 8 | Medium. The break `the-viewer-overflows-after-end`, repointed in this pull request, could no longer fail: two guards keep the window at the end. | The break is gone, with the reason in the list; the test now compares the rows after `End` with those after `PageDown`, `j` and `Down`: `app::tests::screen::end_then_page_down_in_the_viewer_stays_at_the_end`. |
| 9 | Medium. Gaps: the Windows mouse records, the click in the real binary, the suspension. | `sys::windows::tests::mouse_records_become_the_reports_a_terminal_sends` (Windows, with two `os: windows` breaks); the system test clicks a box's row and sees it close; the suspension is a way out of the ways-out test. Which path ConPTY takes for the mouse (records or text) is for the Windows Terminal checklist of the third pull request. |
| 10 | Medium. A frame copied the open answer's rows twice. | Once now (the window shares the live rows); measured above; the rest is [a-full-screen-frame-copies-the-open-answer](../../../outstanding/a-full-screen-frame-copies-the-open-answer.md). |
| 11 | Low. On Windows, the console's mouse input could be set again just after the emergency restore had put the console's mode back. | Set under the writing lock the restore takes. |
| 12 | Low. The search counts in text and marks in drawn rows, chrome included. | [the-search-counts-text-but-marks-drawn-rows](../../../outstanding/the-search-counts-text-but-marks-drawn-rows.md). |
| 13 | Low. A mouse report in flight when the mouse goes off can become text. | [mouse-reports-in-flight-when-the-mouse-goes-off](../../../outstanding/mouse-reports-in-flight-when-the-mouse-goes-off.md). |
| 14 | Low. The mouse counted as typing in the full screen and not in the viewer, and a gesture that closed the viewer skipped what its key does after. | One rule: the mouse answers no dialog, so it is never typing; a gesture that closes the viewer restarts the panels' and the dialog's readiness as the key does. `app::tests::dialogs::the_wheel_does_not_hide_a_dialogs_answers`. |
| 15 | Low. The search worker: its copy of the transcript, the first build on the interface's thread, an unbounded query, a worker that cannot start. | The query holds 256 characters, and a worker that cannot start says `the search could not start` (`the_query_is_bounded_and_the_more_below_row_follows_the_glyphs`); the rest is [the-search-worker-keeps-a-copy-of-the-transcript](../../../outstanding/the-search-worker-keeps-a-copy-of-the-transcript.md). |
| 16 | Low. Smaller points. | The more-below row uses the glyph set's separator (same test). The column is kept with no panel open, so the transcript is not laid out again as panels come and go, and 09-tui says so, with the 97 columns at 140. No scrolling key under a dialog was `the-full-screen-under-a-dialog`, which the coordinator had built in the second pull request and is closed there; a view in the live rows that moves is [a-view-in-the-live-rows-moves](../../../outstanding/a-view-in-the-live-rows-moves.md). |
| 17 | Low. Docs: `draw_screen` still public, 16 KiB in 09-tui, a doc comment over the wrong function, a stale test comment. | `Terminal::draw_screen` and its test are removed (an API change, listed below); 09-tui says 64 KiB; the comments are right. |

### The second review, of the fixes

A second reviewer, not anchored on either, read the fixes' diff (`8e6a45e3..92b95899`), read only. What it found and what was done; each fix with a test that fails without it and a break, under "What the second review of the first pull request's fixes found" in `mutations/step-25.breaks`.

| # | Found | Done |
|---|---|---|
| 1 | The new Windows mouse test read the wheel up at the wrong place: it would have failed on Windows, where alone it is built. | It checks every report's kind, in order. |
| 2 | Under a dialog, a key the dialog passed while the search was open went to the hidden editor, whose `Enter` would send what it held; a paste went into the search's query and its answer moved the window off the end the request had followed. | A passed key goes to the search, never the editor; under a dialog the search does not move the window. `app::tests::dialogs::typing_into_the_full_screen_search_does_not_answer_a_dialog` (now with `Enter`, the query and the editor checked), `app::tests::screen::the_search_does_not_move_the_window_under_a_dialog`. |
| 3 | The suspended way out never suspended: `Ctrl+Z` is the editor's undo, and `global.suspend` has no default key. | The test binds a key to it, and the played terminal marks the stop: on, off with the mouse first, the stop, on again. |
| 4 | The system test's click could read the box's row from a frame before every `PageUp` was taken (ConPTY gives each its own record). | It waits for the top: the header card's model row as the window's fourth. |
| 5 | The deadline's test checks the arithmetic, not the waits; a remainder of exactly `u32::MAX` ms was "no end". | Capped below "no end". Moving `millis_left` out of either loop would bring the restarted timeout back and fail no test: neither wait can be run in a unit test (they read the real terminal), so that is held by review, as it is said here. |
| 6 | The viewer's half of the mouse rule (a gesture that closes the viewer restarts a dialog's grace) had no test. | `app::tests::dialogs::a_gesture_that_closes_the_viewer_restarts_the_dialogs_grace`. |
| 7 | Three open items misstated the code. | Corrected: `the-full-screen-under-a-dialog` (`Ctrl+O` shows the request alone; keys before readiness hold the dialog), `a-view-in-the-live-rows-moves` (the top is clamped, not moved by the kept rows), `a-full-screen-frame-copies-the-open-answer` (which run each figure is from). |

It found the writing lock (no deadlock), the deadline waits (interrupted calls, wakes, zero and no timeout, overflow) and the other fixes as the first table says.

## The pull request's CI

<!-- written by cargo xtask ci-times, section "The pull request's CI": it replaces this up to the end mark -->
Pull request 94, on `main` at `1e733de`. "Main" is main's own run at `1e733de` (run 44352361969).
The PR's run is 44364311801, at `9344498`.

| Job | Main, `1e733de` | The PR at `9344498` |
|---|---|---|
| `lint-linux` | 48 s | 69 s |
| `test-linux` | 120 s | 140 s |
| `lint-windows` | 106 s | 102 s |
| `test-windows` | 184 s | 247 s |
| `pr-size-linux` (blocks nothing) | — | 263 s |
| `ts-client` (blocks nothing) | — | 13 s |
| `pr-size-windows` (blocks nothing) | — | 331 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `9344498` (5 ms) | Windows, main (5 ms) | Windows, `9344498` (6 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 17.0 s | 23.0 s | 8.1 s | 14.4 s |
| Tier 2, integration (40 s) | 36.4 s | 34.0 s | 23.7 s | 39.8 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 30.0 s | 29.8 s | 57.3 s | 72.2 s |

<!-- end of what cargo xtask ci-times wrote, section "The pull request's CI" -->

The PR's run and main's ran on runners of different speeds (a process start of 5 and 6 ms against 4 and 5), and hosted runners differ by up to a factor of two, so the two columns do not measure this pull request: the A/B with both trees on one runner, in "Measurements", does (+0.4 s in tier 1 on each system, tiers 2 and 3 within ±0.3 s). Windows' tier 2 read 39.8 s of its 40 s on the slower runner, against 22.4 s in the A/B.
