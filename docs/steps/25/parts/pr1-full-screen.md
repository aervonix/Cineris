# Step 25, its first pull request: the full-screen presenter and the mouse

The first of step 25's three pull requests: the full-screen presenter, the mouse in it and in the viewer, code highlighted again at a new width, and the measurements of both presenters. The design is [answered.md](../answered.md), "The design, before building", with the coordinator's answers; what this part built is described in [09-tui.md](../../../architecture/09-tui.md), "Full screen as the alternative" and "The mouse".

## What was built

| Where | What |
|---|---|
| `cortex-term`: `full.rs` | `Full` and `Screen`: the whole screen in one write, inside a synchronized update, only the rows that changed, each placed by its number; a resize, a redraw and a failed write have the next frame write every row. |
| `cortex-term`: `input.rs` | The mouse decoder: mode 1006's reports, and mode 1000's own form while the mouse is on (`Decoder::set_mouse`); `Event::Mouse` with `Mouse`, `MouseKind`, `Button`, `Wheel`. |
| `cortex-term`: `terminal.rs`, `sys/` | `Terminal::set_mouse` and `mouse`: modes 1000 and 1006 on with the alternate screen and off before it, at the end and on a resumption; the four emergency restores with the alternate screen on turn them off; on Windows the console's mouse input with the alternate screen, and a mouse record written as the SGR report the decoder reads. `draw_screen` is gone. |
| `cortex-testkit`: `vterm.rs` | Modes 1000 and 1006 in the virtual terminal's `Modes`. |
| `cortex-tui`: `transcript/kept.rs`, `transcript.rs` | `Kept`, the finished blocks, and `Transcript::keep_finished`, `take_finished`, `open_rows` (what an answer still streaming has committed) and `rewrap_open`. |
| `cortex-tui`: `markdown/mod.rs` | `Markdown::keeping`, `take_source`, `render_source` and `code_lines`, `restarted`; `Source` and `CodeColours`. |
| `cortex-tui`: `app/scroll.rs` | The window over the kept blocks and the live rows, which follows the end until moved up, made only near what it shows. |
| `cortex-tui`: `app/search.rs` | The search's worker. |
| `cortex-tui`: `app/state/screen.rs`, `recolour.rs`, `viewer.rs` | The full-screen view (its frame, keys, mouse, search and panels' column); code coloured again; the viewer on the kept blocks and the window, its keys in the key map. `App::frame` is split into `prepare` and `bottom`, which both presenters call. |
| `cortex-tui`: `keys/mod.rs` | The `viewer` and `screen` contexts and their actions; `Gesture`, `Binding`, `parse_binding`, `KeyMap::lookup_gesture` and `gestures_of`. `normalize` reads an upper-case letter alone as `Shift` with the lower-case one, as it did with `Ctrl` or `Alt`. |
| `cortex-tui`: `view/round.rs` | `Placement::Screen`: a committed box a click or the search turned over, and the search's border label. |
| `cortex-tui`: `app.rs` | The loop draws the viewer and the full-screen view through `Full` on the alternate screen, asks for the mouse on every terminal it is given, and draws every row when the alternate screen is entered again. `Settings::presenter` and `Settings::mouse`. |
| `cortex-config`, `cortex-cli` | `tui.presenter`, `terminal.mouse`, `[keys] viewer` and `screen`; `--fullscreen`. |

## Departures from the design, and the choices made

1. **A kept command's output is kept to 64 KiB, not 16.** The design had 16 KiB, with the whole read from the core when the box is opened; that read is not built, and 16 KiB without it would show less in the viewer than step 23 did. So the bound is what a running command keeps, and the read and the lower bound are an open item, [a-kept-box-holds-only-the-end-of-its-output](../../../outstanding/a-kept-box-holds-only-the-end-of-its-output.md), with the worst case's arithmetic. Diffs are not bounded yet either.
2. **No scroll region.** The design had `Full` scroll the transcript's rows inside a region while it follows the end, if a measurement showed it paid. It does not: a frame that follows a stream writes 122 bytes at the median and 4 KB when a committed row moves the area up (measured below), within one read of a pseudo-terminal, so `Full` stays a writer of rows.
3. **Search covers what is kept.** An answer still streaming is searched once it is kept.
4. **The panels' column at 150 columns is 45 wide**, by the design's formula (40, and one for each two columns past 140); screen F5 drew 40, which is the column at 140.
5. **`Placement::Screen`** is a new placement of `view::round`, for a box a click or the search turned over in the full screen: open with all its detail as in the viewer, closed with the committed box's `ctrl+o expands`, and `opened by the search` while the search holds it open. Existing placements draw as before.
6. **The viewer's keys moved into the key map** with the same defaults; `G` is bound as `shift+g`, which `normalize` now matches for a terminal that sends the upper-case letter alone.

## The owed items

- `highlighted-code-drawn-again`: **closed** (its file deleted). Shown by `app::tests::screen::code_committed_plain_is_coloured_once_its_colours_come`, `highlight::tests::a_kept_answer_records_its_code_colours_and_the_lines_that_were_late` and `markdown::tests::a_kept_answers_code_keeps_its_colours_at_another_width`.
- `the-bincode-advisory-ignored-for-syntect` ("the next step that touches highlighting, otherwise step 25"): looked at on 2026-11-17; `syntect` 5.3.0 is still its latest release, on `bincode`, so the ignore stays and the item is left open.

## The tests that show each part

| Part | Tests |
|---|---|
| The same rows under both presenters | `app::tests::screen::the_same_rows_are_drawn_under_both_presenters_frame_by_frame` (frame by frame while two turns stream, at 60, 80 and 120 columns, styles included); `transcript::tests::every_frame_of_a_stream_is_drawn_the_same_by_both_presenters`, `kept_blocks_draw_again_the_lines_that_were_committed` |
| Resize | `app::tests::screen::a_resize_lays_the_transcript_out_again`, `an_answer_resized_while_it_streams_is_laid_out_again`; `transcript::tests::an_answer_resized_while_it_streams_is_laid_out_again`; `cortex-term::it full::a_resize_to_any_width_draws_the_screen_whole_and_right`; inline's are step 20's |
| A frame's properties | `cortex-term::it full::every_frame_is_one_write_with_nothing_cleared_and_the_right_screen`, and `full::tests` |
| Following the end | `app::tests::screen::the_window_follows_the_end_until_it_is_moved_up`; `app::scroll::tests` |
| Search | `app::tests::screen::ctrl_f_searches_the_transcript_and_goes_between_matches`, `a_match_inside_a_closed_box_opens_it_while_it_is_current`; `app::search::tests` |
| Panels beside | `app::tests::screen::panels_sit_beside_the_transcript_on_a_wide_window` |
| The mouse | `cortex-term`: `input::tests::mouse_reports_in_the_sgr_form`, `mouse_reports_in_the_older_form_are_taken_whole`, `a_mouse_report_decodes_the_same_at_every_split`, `terminal::tests::the_mouse_is_on_only_with_the_alternate_screen_and_off_on_every_way_out`, `a_resume_turns_the_mouse_on_again_with_the_alternate_screen`, `the_older_mouse_report_is_read_only_while_the_mouse_is_on`; `cortex-tui`: `app::tests::screen::the_wheel_scrolls_and_a_click_opens_or_closes_a_box`, `the_viewer_scrolls_with_the_wheel_and_a_click_opens_a_box`, `the_inline_screen_takes_no_mouse_and_no_full_screen_keys`, `keys::tests::the_viewer_and_the_full_screen_have_their_keys_and_the_mouse`, `a_gesture_is_bound_only_where_the_mouse_is_reported` |
| Ways out | `app::tests::screen::every_way_out_of_the_full_screen_leaves_the_mouse_and_the_alternate_screen_off` (quit, the core gone, the terminal failing, a suspension and the screen taken again), `the_viewer_turns_the_mouse_off_as_it_returns_to_the_main_screen`; the real binary: `cortex-cli::system tui::sys_the_full_screen_runs_a_turn_scrolls_and_leaves_the_terminal_as_it_was` (the shell's verdict compares the terminal's or the console's modes before and after) |
| The inline presenter unchanged | every snapshot and golden screen of `cortex-tui`, unchanged; `COMPARED` is as it was |

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

## The independent review

A reviewer that had not seen the building read the whole diff (`24305e3d..8e6a45e3`), and nothing was built or run for it. What it found, and what was done; each fix has a test that fails without it (checked by undoing the fix) and a break in `mutations/step-25.breaks`, `mutations/step-23-fixes.breaks` for the eighth.

| # | Found | Done |
|---|---|---|
| 1 | High. A search's answer, or code's late colours, that came while nothing else happened drew no frame: the row stayed `searching…` until the next key. | The loop's drain takes both and says there is a change. `app::tests::screen::a_search_answer_that_comes_while_idle_schedules_a_frame`; `code_committed_plain_is_coloured_once_its_colours_come` asserts the drain. |
| 2 | High. Typing or pasting into the full screen's search did not count as typing, so a slow typist's next key answered a dialog that had just come; and the search row was drawn over the dialog's own. | Both count as typing (`not_taken`), and the row is not drawn under a dialog. `app::tests::dialogs::typing_into_the_full_screen_search_does_not_answer_a_dialog`, `a_paste_into_the_full_screen_search_holds_a_dialog_that_comes`. |
| 3 | High. The viewer opened while an answer streamed lacked the rows the answer had committed. | The viewer draws them before the live rows, as the full screen does. `app::tests::screen::the_viewer_opened_mid_stream_shows_what_the_answer_committed`. |
| 4 | High. `[keys] viewer` and `[keys] screen` never reached the key map. | Every table is read (`key_tables`), and a test compares the tables read with the ones the schema declares and looks a binding up in each new one: `cortex-cli` `interactive::tests::every_table_of_keys_is_read`. |
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
| 16 | Low. Smaller points. | The more-below row uses the glyph set's separator (same test). The column is kept with no panel open, so the transcript is not laid out again as panels come and go, and 09-tui says so, with the 97 columns at 140. No scrolling key under a dialog, and a view in the live rows that moves, are [the-full-screen-under-a-dialog](../../../outstanding/the-full-screen-under-a-dialog.md) and [a-view-in-the-live-rows-moves](../../../outstanding/a-view-in-the-live-rows-moves.md). |
| 17 | Low. Docs: `draw_screen` still public, 16 KiB in 09-tui, a doc comment over the wrong function, a stale test comment. | `Terminal::draw_screen` and its test are removed (an API change, listed below); 09-tui says 64 KiB; the comments are right. |
