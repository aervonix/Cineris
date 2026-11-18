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
- **Golden screens:** F1a to F4 (`f1a` … `f4` in `tui-look/`), drawn by the application, 143 rows compared on each background; `COMPARED` went from 19 screens and 508 rows to 25 screens and 651 rows.
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

## Open

- `reading-a-key-at-a-terminal` is **closed** (its file deleted, the index written by `cargo xtask outstanding`), but for the two links in `answered.md` above.
- Nothing else is left open by this part. The Windows checks below are a person's.

## For a person at a real Windows Terminal

1. The first run in an empty profile directory (`CINERIS_CONFIG` naming a file that does not exist): the card, each step, its keys appearing a moment after it does.
2. A key typed and a key pasted (`Ctrl+V` and right-click): the row counts characters, nothing of the key on the screen or in the scrollback, and Windows Credential Manager holds `cineris:<provider>` afterwards.
3. `F2b` with a real `~/.claude.json`: each command whole, a token in an argument masked.
4. The mode step without the sandbox (Windows until its sandbox step): `autonomous` not offered.
5. `cineris auth login anthropic` in Windows Terminal and in the old console host: the row, `Ctrl+U`, `Esc`, and the terminal as it was afterwards.
