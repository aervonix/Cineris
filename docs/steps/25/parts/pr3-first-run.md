# Step 25, its third pull request: the first run

The third of step 25's pull requests, in the part that builds the first run and `cineris auth login` at a terminal. The design is [answered.md](../answered.md), "The design, before building", point 5, with the coordinator's answers (a), (b), (c), (e), (f) and (g) and the smaller points; the screens are [design/screens.md](../design/screens.md), F1a to F4, revised to those answers and now golden screens. The first two parts are [pr1-full-screen.md](pr1-full-screen.md) and [pr2-session.md](pr2-session.md).

## What was built

| Where | What |
|---|---|
| `cineris-base`: `secret_buf.rs` | `SecretBuf`: a key being typed, in a buffer of fixed capacity (64 KiB) that never moves, written over with zeros when a character is taken away, when it is emptied and when it is dropped (the standard library's `black_box` and a compiler fence: no `unsafe`, which the crate denies, and no new crate). Text with a line break, or past the capacity, is refused whole. `Debug` says how many characters, never which. |
| `cineris-tui`: `secret.rs` | The key's input, not the editor: typing, a paste, `Backspace`, `Ctrl+U`, `Enter`, `Esc`, nothing else. Its row says `N characters, hidden`. `read_key` for `auth login` at a terminal: inline, one sentence, the row and its keys, the terminal put back whatever happens. |
| `cineris-tui`: `app/setup.rs` | `app::Setup`, the trait the program gives the application (`Start::setup`), its types (no secret in any of them: a variable by its name, an MCP server's values by their names, its command already masked) and the worker its calls run on, which wakes the loop. |
| `cineris-tui`: `app/state/first_run.rs`, `view/setup.rs` | The four steps in the input box's place under the set-up card, which is never committed; one dim line committed per step answered; the dialog's rule of keys, a step's keys shown once it takes them; `s` and `x`; the file written, `config/reload`, the session opened; `project/detect` and `project/update`'s `add` through the `Link`; the last line naming what does each step again. |
| `cineris-tui`: `testkit.rs` | `FakeSetup` and `app_first_run`, for the application's tests and the golden screens. |
| `cineris-config`: `import.rs` | Reading one of the user's MCP files of another client (`~/.claude.json`, Cursor's, Claude Desktop's: `mcpServers`; Codex's `config.toml`: `[mcp_servers.*]`): values dropped to names, every argument and the address through the secret filter's formats and the user-part rule, a match masked and the server refused; a control character or one that acts unseen refuses it too. |
| `cineris-config`: `write.rs` | The user's configuration file, new only (`private::create_new`), through the TOML serialiser, read strictly as a user's file and compared with what was meant before it is created and again from the disk after; a comment for each step skipped. `compat.instructions` and `compat.skills` are always written, `false` unless picked. |
| `cineris-knowledge`: `detect.rs` | The commands of `Cargo.toml`, `package.json`'s scripts, a `Makefile`'s `build`/`test`/`check`/`lint`, `go.mod` and `pyproject.toml`: only plain words the observer classes (`normal_line`, `classify`), each named once; what a script runs given beside it, cut at 1,000 characters. |
| `cineris-protocol`, `cineris-core`, `cineris-rpc` | `project/detect` (`{ session_id }` → `{ commands }`, `DetectedCommand`), an addition with no new protocol version (answer (f)): the core reads the five files through the session's resolver and read gate on the blocking pool; the lane's `reads()` has it. The golden transcript gains its exchange; the schema and the TypeScript declarations are regenerated. `Core::credential_store` and `Core::sandbox_status`, for the first run's store sentence and its levels. |
| `cineris-cli`: `setup.rs`, `interactive.rs`, `commands.rs` | The program's `Setup`, given only when the user's file does not exist (`CINERIS_CONFIG`'s, or `config.toml`; anything at the name counts as there). `cineris attach` gives none. `auth login` at a terminal reads the key with `secret::read_key`. |
| Docs | 08's "First run" (the mode before the project, and why; as built) and "Credentials" (`auth login` at a terminal; what is not wiped); 09's "The first run"; 11's `project/detect`; the screens' README; the design's F1a to F4 replaced by the golden screens' text. |

## Departures, and why

- **The mask is `[redacted]`,** not the drafts' `[masked]`: it is what the harness writes everywhere it masks (the log, `config show`, `/export`), from `cineris_base::redact::MASK`.
- **A step's prose wraps at `tui.prose_width`,** as an answer does; the drafts had wrapped by hand at about the same width. Where a long path does not fit, it wraps onto a line of its own (F1a).
- **A step shows its keys only once it takes them,** as a dialog shows its answers (step 23's rule), and the loop is due to draw at the end of its grace (`App::deadline`). The design said only that a step holds the dialog's rule of keys; showing the keys is how a person, and a pseudo-terminal test, knows the step is listening.
- **Typing into the key's input is taken whenever the step is shown;** only `Enter` and `Esc` wait for its keys. Keys read together (a burst) still never answer it.
- **F4's header is the session's own,** which says nothing yet of the project's profile, and the step's keys leave out `x`, which after the session opens would do what `s` does; with it the hint row's model was cut.
- **The instructions' and skills' rows are shown always,** whatever discovery found in this workspace: each is a setting for every project.
- **`auth login` at a terminal reads `[terminal]`'s overrides as their defaults:** the command does not load the user's configuration of its own (its core does, inside). Esc or Ctrl+C there stores nothing and exits 2, saying so.
- **A server's header is taken from a variable named for the server and the header** (`TRACKER_AUTHORIZATION`): the draft's `TRACKER_AUTH` named the header's short form, which no rule gives.
- **The crash report is not tested with a key held:** no code formats the key's text (the buffer's only reads are its count, its trimmed `Secret` for the store and the ASCII check), and the report holds the panic's message through the log's filter and the backtrace (step 23's test). A test would have to panic the interface on purpose with a key in the input; it is left, said here.
- **One edit outside this part's files that is not made:** closing `reading-a-key-at-a-terminal` leaves two links to its file in [answered.md](../answered.md) (point 5's `auth login` bullet and the owed items' table), which only the coordinator edits. `cargo xtask links` reports them until they become plain text (`` `reading-a-key-at-a-terminal` ``, as `the-export-command-has-no-step`'s row is).

## The tests, by part

- **The key:** `cineris-base secret_buf::tests` (refused whole, wiped when taken away, emptied and by the drop's code, `Debug`, ASCII); `cineris-tui secret::tests` (the keys it answers and the ones it does not, refused pastes, `Ctrl+U`, the row's count and none of the key, a hostile provider name); `app::tests::first_run::a_key_typed_is_never_drawn_and_goes_only_to_the_store` (no piece of the key, nor of a refused paste, in any row of any frame, committed or live; the store gets the key whole) and `a_store_that_refuses_keeps_the_step_and_esc_goes_back`.
- **Each step through the scripted `Link`:** `the_first_run_reaches_a_session_made_with_what_was_chosen` (the file written before the session opens, with what was chosen; the reload, the open, the detect and the `add` sent in order; one card in the scrollback); `every_step_can_be_skipped`; `x_skips_the_whole_set_up`; `a_project_with_no_commands_ends_the_first_run`; `autonomous_is_offered_only_where_the_sandbox_enforces` (and never `unrestricted`); `ctrl_c_twice_leaves_with_nothing_written`.
- **The rule of keys:** `a_prompt_typed_at_the_first_frame_chooses_nothing` (a prompt and its `Enter` at the first frame, an `Enter` held from the step before), `a_steps_keys_are_drawn_at_the_end_of_its_grace_and_taken_only_then`.
- **Imports:** `nothing_unpicked_is_imported` (a file read is not a pick; instructions and skills off), `an_mcp_command_is_shown_whole_and_its_credential_masked`; `cineris-config import::tests` (values dropped, a credential masked and refused, a user part masked, a direction mark refused, Codex's TOML); `cineris-config write::tests` (strict read-back, comments for skips, a hostile server name stays one key, a refused server never written, never over a file); `cineris-cli setup::tests` (discovery apart from the project's files, only a server shown and importable written, where the key is kept, when the first run is due).
- **Hostile text:** `every_listing_holds_hostile_text` (each of `TRICKY`, with a direction override, as a provider's name and variable, a file's path and tool, a project's MCP file, a server's name, command, environment and header, and a command found, its name, line, script and file, and typed into `e`'s field: every row of every frame through `testing::problems`); `view::setup::tests`.
- **Detection:** `cineris-knowledge detect::tests`; `cineris-core reads::tests::detection_reads_only_what_the_gate_lets_through_inside_the_workspace`; `cineris-rpc lane::tests::detecting_a_projects_commands_is_a_read`; the protocol's golden transcript.
- **End to end, under a pseudo-terminal** (`cineris-cli` system tier): `sys_the_first_run_in_an_empty_home_reaches_a_working_session` (no configuration file; a key typed into the second row's input; the imports and the mode kept; the file written; the session answers a prompt from the scripted model; no eight characters of the key in any byte the binary wrote, read raw, nor in the screen and scrollback, nor in any file under the test's home and workspace but the store once the program has ended (its log, its history, the session, the configuration file); the key in the test's file store; the terminal put back) and `sys_auth_login_at_a_terminal_reads_the_key_without_echo`. About 4.6 s and 0.1 s at the desk: most of the first's is the steps' graces and repeat windows, which it waits out as a person would.
- **Golden screens:** F1a to F4 (`f1a` … `f4` in `tui-look/`), drawn by the application, 142 rows compared on each background; `COMPARED` went from 19 screens and 508 rows to 25 screens and 650 rows (651 before the rename to Cineris, whose shorter credentials path put F1a's provider paragraph on one row fewer; the screens were written again from the application with the rename).
- `cargo xtask stress -p cineris-tui --count 40` on the first run's and the screens' tests, and again 30 times with `--cores 2 --load 6`: every test passed every time. The first stress found a race of the tests' own (a frame took the worker's answer before the test waited for it), fixed by counting what the worker owes.

## The breaks

`mutations/step-25-firstrun.breaks`, 22 breaks, run with `cargo xtask mutate step-25-firstrun --minutes 0` at the desk: **every one caught.** Two needed their lists put right after the first run: the address's user part was caught by its one test and not by a second the list had named, and a script's name that is not plain words was not caught at all, because the observer's table classes only plain names, so the `normal_line` check behind it is a second guard (kept, and said here). That break was replaced by the one-word rule for a script's name (`check me`), which its test catches.

| Break | Caught by |
|---|---|
| `a-key-emptied-is-not-written-over` | `cineris-base secret_buf::tests::what_is_taken_away_or_emptied_is_zero` |
| `a-key-takes-a-line-break` | `cineris-base secret_buf::tests::a_line_break_or_too_much_is_refused_whole`, and two of `cineris-tui` |
| `a-key-taken-is-left-in-its-buffer` | `cineris-tui secret::tests`, two |
| `the-key-input-types-control-keys` | `cineris-tui secret::tests::typing_backspace_and_ctrl_u_change_what_it_holds_and_nothing_else_does` |
| `a-step-takes-keys-before-it-is-ready` | `a_prompt_typed_at_the_first_frame_chooses_nothing`, `a_steps_keys_are_drawn_at_the_end_of_its_grace_and_taken_only_then` |
| `a-steps-keys-shown-before-it-is-ready` | `a_steps_keys_are_drawn_at_the_end_of_its_grace_and_taken_only_then` |
| `the-session-opens-before-the-file-is-written` | seven of `app::tests::first_run` |
| `the-instructions-start-picked` | `nothing_unpicked_is_imported`, `the_first_run_reaches_a_session_made_with_what_was_chosen` |
| `a-server-holding-a-credential-can-be-picked` | `the_first_run_reaches_a_session_made_with_what_was_chosen` |
| `autonomous-offered-without-a-sandbox` | `autonomous_is_offered_only_where_the_sandbox_enforces` |
| `the-commands-start-picked` | `the_first_run_reaches_a_session_made_with_what_was_chosen`, and the golden screens (F4) |
| `a-workers-answer-wakes-no-frame` | `cineris-cli::system first_run::sys_the_first_run_in_an_empty_home_reaches_a_working_session` (the app's tests take answers themselves) |
| `an-mcp-argument-is-not-masked` | `cineris-config import::tests`, two |
| `an-mcp-address-keeps-its-user-part` | `cineris-config import::tests::a_credential_in_a_command_or_an_address_is_masked_and_refused` |
| `the-writer-writes-over-a-file` | `cineris-config write::tests::the_file_is_made_new_and_never_over_one_that_is_there` |
| `a-refused-server-is-written` | `cineris-config write::tests::a_server_that_cannot_be_imported_is_never_written` |
| `a-script-name-with-a-space-is-offered` | `cineris-knowledge detect::tests::scripts_are_offered_by_their_names_with_what_they_run` |
| `detection-reads-past-the-read-rules` | `cineris-core reads::tests::detection_reads_only_what_the_gate_lets_through_inside_the_workspace` |
| `detection-waits-in-the-lane` | `cineris-rpc lane::tests::detecting_a_projects_commands_is_a_read` |
| `auth-login-refuses-a-terminal` | `cineris-cli::system first_run::sys_auth_login_at_a_terminal_reads_the_key_without_echo` |
| `the-first-run-over-a-file-that-is-there` | `cineris-cli setup::tests::the_first_run_is_due_only_where_no_file_is`, and 17 system tests whose machines have a file |
| `the-program-writes-a-refused-server` | `cineris-cli setup::tests::only_a_server_shown_and_importable_is_written` |

## The full review of the third pull request

An independent reviewer read the first run's code. Each finding was checked against the code before it was fixed; each fix has a test that fails without it (the fix undone by hand and the test run through `rx`, then put back) and a break in `mutations/step-25.breaks`, in its first-run section, that the test catches. Findings 3 to 6, how keys reach the steps, were fixed first, since they change the path that accepts commands.

| # | Finding | What was done | Test | Break |
|---|---|---|---|---|
| 3 | The commands step could come up under a running turn (`cineris "fix the build"`: the prompt goes at the open, `project/detect` answers mid-turn), a dialog or the viewer, and it took keys while it was not drawn: a dialog drawn over it had its keys go to the hidden step, so `space` then `enter` could keep a command nobody saw. | The last step is asked for (`project/detect`) and shown only while the session is idle (`App::first_run_idle`): no turn running and no prompt on its way, no dialog, not under the viewer, the full screen's search or a rewind, no panel's item being edited, and the input box empty with no menu open. A step that is not what the input box's place shows takes no key (`App::first_run_shown`): the keys go where they would without it. Hidden and shown again, a step starts as if new. Until the last step comes the input box is the input box, and a prompt is sent as any prompt is; 09 and 08 say so. | `app::tests::first_run::the_commands_wait_for_the_turn_a_prompt_on_the_command_line_began`, `the_commands_step_takes_no_key_while_a_turn_or_a_dialog_is_shown` (`x` under the turn, `space` and `enter` under its dialog), `typing_just_before_a_step_comes_holds_it_a_quiet_second` (the step waits while the input box holds text) | `the-commands-come-under-a-turn`, `a-hidden-step-takes-keys`, `the-commands-come-over-what-is-typed` |
| 4 | A step's readiness was not seeded as a dialog's is: dialogs take the application's `untaken` and `taken` on their first frame, steps took only the first run's own last answer, so a person typing when a step appeared had their next key after its grace taken. | A step's first frame, and its first after it was hidden, takes the application's `untaken` and `taken`, as a dialog's does (`App::first_run_drawn`). The other way too: input a step does not take, and a key it takes, now count for what comes after it (`untaken`, `taken`, `last_key`), so the session's trust question just after the mode's `enter` waits a key's repeat window, as after a dialog. Rows the worker's answer changes under a step (the providers, what other tools left, a file's servers) start its grace again, keeping its quiet (`Readiness::again`, new). | `typing_just_before_a_step_comes_holds_it_a_quiet_second`, `the_key_that_answered_a_dialog_holds_the_step_after_it`, `the_key_that_answered_a_step_holds_the_dialog_after_it`, `rows_that_come_under_a_ready_step_start_its_grace_again`, `the_providers_and_what_was_found_start_the_grace_again` (a program of the test's own holds its answers at gates, `testkit::app_first_run_on`); `app::ready::tests::again_starts_its_grace_again_and_keeps_its_quiet` | `a-step-is-not-seeded-with-what-was-typed`, `a-step-is-not-seeded-with-the-key-taken`, `a-steps-key-holds-no-dialog`, `the-providers-come-under-a-ready-step`, `what-was-found-comes-under-a-ready-step`, `a-files-servers-come-under-a-ready-step`, `the-grace-again-forgets-the-quiet` |
| 5 | The imports' selection was an index into the rows one can select, and a file's answer inserts its servers above the rows after it: `space` on the first of two files, `Down` to the second, the first's answer, and `space` picked the first file's first server. | The selection is the row it is (`Pick`), and its place among the rows is found again when a key comes. | `a_files_answer_does_not_move_the_selection_onto_its_servers` | `a-files-answer-moves-the-selection` |
| 6 | Steps were not fitted to the window: F2b is 34 rows, and at 24 the inline presenter keeps a tall region's bottom, so the title and the sentence were gone; with about six servers, rows off the screen could still be selected and nothing scrolled with the selection, against "shown in full before it is accepted". | A step fits the window's rows (`view::setup::fitted`, new): the card goes first; then the rows are a window around the selected one, drawn whole, with `… 3 more above` and `… 2 more below` in the place of the rest; the note, the blank above the rows and the sentence give way, in that order, only when the selected row would not fit beside them; the title stays. A selected row taller than the window is cut, the hint row says `a taller window shows this row whole`, and `enter` or `space` on it does nothing (`FirstRun::whole`): a provider, a mode, a server and a command are accepted only once they have been drawn whole. | `in_a_short_window_the_step_keeps_its_title_and_the_selected_row_whole` (24 rows, eight servers, each drawn whole as the selection moves down), `a_row_the_window_cannot_show_whole_is_not_accepted`; `view::setup::tests::a_step_taller_than_its_room_is_a_window_around_its_selected_row` | `a-step-is-not-fitted`, `the-card-stays-in-a-short-window`, `a-row-not-drawn-whole-is-accepted`, `a-selected-row-too-tall-is-said-whole`, `a-mark-takes-no-room` |

**Tests the fixes changed.** The tests that pressed a key the moment a file's servers came now wait out the step's grace first (`the_first_run_reaches_a_session_made_with_what_was_chosen`, `nothing_unpicked_is_imported`, `every_listing_holds_hostile_text`, and the golden screens' driver before F2b's keys); the system test `sys_the_first_run_in_an_empty_home_reaches_a_working_session` waits for discovery's row as well as the imports' keys before its `enter`. The golden screens are unchanged: they are drawn 40 rows high, where every step fits whole.

**Public items added** (the test kit and the view): `testkit::app_first_run_prompted`, `testkit::app_first_run_on`, `view::setup::fitted` and `view::setup::Fitted`.

**The breaks:** `cargo xtask mutate` ran the 16 this review added and the seven earlier first-run breaks of the code it touched (`a-step-takes-keys-before-it-is-ready`, `a-steps-keys-shown-before-it-is-ready`, `the-session-opens-before-the-file-is-written`, `the-instructions-start-picked`, `a-server-holding-a-credential-can-be-picked`, `autonomous-offered-without-a-sandbox`, `the-commands-start-picked`): **all 23 caught,** each by the tests its list names.

**Stress:** `cargo xtask stress -p cineris-tui` on the first run's, the step view's and the readiness's tests (30), 200 times, and 60 times more with `--cores 2 --load 6`: every test passed every time.

### Departures, and what was decided against

- **The last step waits for an empty input box,** beyond the review's "no turn, dialog or viewer": a step that took the input box's place from a prompt half written would take its next keys. A person who never empties it is never shown the step, and the first run's last line waits with it; `/project` adds commands at any time.
- **What a step takes counts for what follows it,** beyond the finding, which named only a step after a dialog: the trust question after the mode's `enter` was the same hazard the other way round.
- **Rows that come start only the grace again (300 ms), not a key's repeat window:** a file's servers come in milliseconds, and a second after every `space` that reads one would be felt. The quiet owed to input the step did not take is kept.
- **Decided against:** a window that keeps its place as the selection moves (it is laid out anew each frame, growing from the selected row above and below in turn, which needs no state and cannot leave the selection off the screen); a guard against the full screen's search row over a step's keys (not reachable: a step takes every key while it is shown, and the last step waits for the search to close).

## Open

- `reading-a-key-at-a-terminal` is **closed** (its file deleted, the index written by `cargo xtask outstanding`), but for the two links in `answered.md` above.
- Nothing else is left open by this part. The Windows checks below are a person's.

## For a person at a real Windows Terminal

1. The first run in an empty profile directory (`CINERIS_CONFIG` naming a file that does not exist): the card, each step, its keys appearing a moment after it does.
2. A key typed and a key pasted (`Ctrl+V` and right-click): the row counts characters, nothing of the key on the screen or in the scrollback, and Windows Credential Manager holds `cineris:<provider>` afterwards.
3. `F2b` with a real `~/.claude.json`: each command whole, a token in an argument masked.
4. The mode step without the sandbox (Windows until its sandbox step): `autonomous` not offered.
5. `cineris auth login anthropic` in Windows Terminal and in the old console host: the row, `Ctrl+U`, `Esc`, and the terminal as it was afterwards.

## The binary's size

Measured early, as the coordinator asked, before more was built, on try run 44418596981 (`size --check --against-main`, at `3b95969a` on `main` `bb8f58d0`):

| System | This pull request | Budget | Used | Against `main` |
|---|---|---|---|---|
| Windows, `dist` | 18,116,608 bytes (17.28 MiB) | 18 MiB | 96.0% | +257 KB (`main` 17,852,928) |
| Linux musl, `dist` | 14,944,680 bytes (14.25 MiB) | 16 MiB | 89% | +188 KB |

Under both budgets. After this pull request, about 757 KB is left on Windows.

**By crate** (`size --breakdown`, the Linux build at the desk, through `rx`, against `main` built the same way): `cineris_tui` +55.6 KB, the standard library +41.9 KB, headers and unwind tables +36.3 KB, the binary's own crate (`cineris`) +28.6 KB, `serde_json` +5.4 KB, `cineris_core` +5.2 KB, `cineris_protocol` +5.1 KB, `tokio` +4.3 KB, `cineris_knowledge` +2.9 KB, `cineris_term` +2.6 KB, `cineris_config` +2.0 KB; the rest under 1 KB each. 191,952 bytes in all.

**By symbol** (`nm` of the two unstripped builds): 242 symbols new or grown (169,594 bytes) and 38 gone or shrunk (−13,928). The largest are the first run's own code:

- `App::first_run_view` +14.1 KB;
- the program's `Setup::write_config` +9.8 KB and `read_servers` +8.8 KB;
- `App::bottom` +9.1 KB;
- `App::first_run_answers` +7.7 KB;
- the setup worker's thread body +7.4 KB;
- `Setup::discover` +5.7 KB;
- `App::first_run_key` +5.5 KB;
- `serde_json`'s reading of a list of another client's servers +5.4 KB;
- `hint_row` +5.4 KB;
- the core's handler +5.2 KB (`project/detect`);
- `secret::read_key` +2.9 KB.

**Cuts seen, not made** (none is free: each changes code beyond a few lines, and together they come to about 15 KB of the 188 KB):

- The setup worker's two channel types (`Job` and `Answer`) each instantiate the standard library's channel (`send`, `recv`, `drop`: about 5.5 KB in all); one channel type, an enum, would share one copy.
- The standard library's stable sort of `(String, toml::Value)` pairs appears twice, about 4 KB each, in two codegen units of the same generic code (a `BTreeMap` built from an iterator, as it reads). Building the configuration's table once, or `codegen-units = 1` for `dist` (a build-time cost, and a change for every crate), would keep one.
- `App::bottom` grew by the first run's branch inside it (+9.1 KB). Moving that branch into a function of its own would not shrink it, only move it.

So the first run's cost is its own code, and no cheap cut would change the picture. The budget question (Windows at 96%) goes to the owner with the Windows test job's time.
