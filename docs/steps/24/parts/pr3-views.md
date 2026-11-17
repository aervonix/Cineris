# Step 24, its third pull request: the commands and the live views

The commands that need no panel, the hot reload of the look and the keys, the files named with `@` read ahead, a check's whole output in the viewer, a skipped plan item restored, and screens 2a and 2b in their real states ([answered.md](../answered.md), "The design, before building", points 4b, 5 and 7). The usage and project panels, and the measurements of the panels, are this pull request's other part, `pr3-usage-project.md` beside this one.

## What was built

- **`/reload`, `/profile [name]`, `/verify`, `/context`,** each moved from `NOT_YET` into `COMMANDS`, and **`/config` with where each value came from** (`config/get`'s `origins`). A reload's notice is said once: from the command's own result, or from `config.changed` when another client's reload or the core's look before a turn found the change.
- **The hot reload of the look and the keys** (`cortex-tui`'s `app/state/restyle.rs`): the look asked for after a key's frame (at most once a second) and on a focus-in; the `tui` and `keys` sections asked for again when a reload applied a key of them; the look built again, and the key map when a binding changed (through step 24's second part's `App::set_keymap`); a theme chosen with `/theme` kept unless the file's own changed; a section that cannot be read said, the old ones kept. The program gives the application the function that reads the sections (`Start::restyle`, `App::set_restyle`, `Restyle`, `Restyled`), built in `cortex-cli` from `cortex-config`'s new `schema::read_section`.
- **`@` files read ahead:** `Outcome::FileChosen` when an `@` completion is accepted, and `workspace/warm` for it, at most 16 a prompt, a folder not; nothing of the answer said.
- **A check's whole output in the viewer:** the review panel's `Enter` on a check sends `session/artifact` (`Act::Artifact`), and the viewer shows the text under its label (`open_text`), sanitised and wrapped, what was cut said.
- **A skipped plan item restored** with `space` (`PlanEdit::Restore`), the hint row offering it.
- **The usage report's aggregates kept** (`cortex-core`'s `usage::AggregatesCache`, the coordinator's call when the usage panel's year missed its budget): a report reads `aggregates.json` again only when it is not the file the kept aggregates were read from: told apart by its path, the file itself (`cortex_base::FileIdentity`: device and inode, or volume and file index on Windows, where a save is a new file put in place), its length and its time, taken before the read. A report that saved keeps nothing, nor one that did not read the aggregates whole from the file; what is kept is dropped after a quiet minute. A year's report goes from 17.4 / 19.4 ms to 7.2 / 8.2 ms (median / p99, release) on the desktop, and from 40.1 / 81.2 ms to 16.1 / 27.9 ms on a hosted Windows runner (try run 43995127341), which is still over the 16 ms budget there: the open item [the-usage-panels-year-on-windows](../../../outstanding/the-usage-panels-year-on-windows.md) has the numbers and the next levers, for the owner. The measurement is `cortex-stats`' `perf_a_years_report_for_the_usage_panel`, and the budget's row says the rest.
- **Screens 2a and 2b** composed in the application's real states, as the owner asked: asked in the middle of a turn (the spinner and the turn's time before the dialog's keys), and 2a's edit with the rule a lasting answer adds ("Allowing it for longer allows: …", two rows more). Written with `tui-screens --write`; the tui-look README says what changed.

**A protocol addition, told to the coordinator first** (found by the review: `config/get` never held `profiles.*`, so `/profile` alone found none against the real core): `ConfigGetResult.profiles` and `ConfigGetResult.profile`, both optional with defaults, from `cortex-config`'s new `Loaded::profiles` (the names every layer but the project's defines) and, once the project is trusted, `project_profiles`. The coordinator's condition: a project not trusted puts no profile in force this way, so its profiles are not offered and `config/reload` refuses one only it defines (`untrusted_profile`).

## Where it departs from the design

| Design | As built | Why |
|---|---|---|
| The hot reload tested by "a `cortex-cli` test [that] drives the application through the in-process link on a real core", for a focus event and for the core's turn-start look | One system test through the real binary, a key's frame finding the change (`sys_a_look_and_a_binding_changed_in_the_file_reach_the_next_key`), then a broken file said by `/reload`; the turn-start path held by the core's test of the look before a turn (`cortex-core::it reload::a_turn_starts_with_the_files_looked_at`) and the application's test of `config.changed` (`a_reload_of_the_look_reads_it_again_and_leaves_the_keys`) | Driving the application on a real core in process would need the system tests' machine again, as a unit test; a focus event through ConPTY is not known to arrive, and a second look in one process would wait a real second. The key's path needs neither |
| The interface builds the `Look` and the key map from `config/get` | The program gives it a function that does (`App::set_restyle`), with `cortex-config`'s `read_section` | The look's and the keys' settings are `cortex-config`'s types, which `cortex-tui` does not depend on; the binary builds them at the start the same way |
| A focus-in asks for a look at once | At most once a second, as a key | One rule for both; a focus-in after a second away is past it |

## Tests

| Rule | Test |
|---|---|
| A reload's notice is said once, from its result | `cortex-tui app::tests::commands::reload_says_what_the_reload_did_once` |
| A change found elsewhere is said | `commands::a_change_found_elsewhere_is_said` |
| `/profile` loads one, and lists them with the one in force | `commands::profile_loads_one_and_lists_them`, `profile_with_none_defined_says_how_to_make_one` |
| A project's profile is offered and loaded only once the project is trusted | `cortex-core::it reload::a_projects_profile_is_offered_and_loaded_only_once_it_is_trusted` |
| `/verify` asks for the checks, and says why none could run | `commands::verify_asks_for_the_checks_and_says_why_none_could_run` |
| `/context` by part, against the limit and the window | `commands::context_shows_the_last_request_by_part` |
| `/config` says where each value came from | `commands::config_says_where_each_value_came_from` |
| A look after a key's frame or a focus-in, at most once a second, never while one is out | `restyle::a_key_or_the_focus_asks_for_a_look_after_its_frame_at_most_once_a_second` |
| A reload of the look reads it again and leaves the keys | `restyle::a_reload_of_the_look_reads_it_again_and_leaves_the_keys` |
| A binding reloaded is the next key's | `restyle::a_binding_reloaded_reaches_the_next_key` |
| A theme chosen for the run stays unless the file's own changed | `restyle::a_theme_chosen_for_the_run_stays_unless_the_files_own_changed` |
| A look that cannot be read is said, the old one kept | `restyle::a_look_that_cannot_be_read_is_said_and_the_old_one_kept` |
| The look and the keys are built from the core's answer as at the start | `cortex-cli interactive::tests::the_look_and_the_keys_are_read_again_from_the_cores_answer` |
| A section is read from dotted keys and JSON values | `cortex-config::it reloading::a_section_is_read_from_dotted_keys_and_json_values` |
| The look and a binding changed in the file reach the next key; a broken file keeps them | `cortex-cli::system tui::sys_a_look_and_a_binding_changed_in_the_file_reach_the_next_key` |
| A file chosen with `@` is read ahead, at most 16 a prompt, a folder not, a refusal not said | `files::a_file_chosen_with_at_is_read_ahead_and_a_refusal_is_not_said` |
| `space` on a skipped item restores it | `panels::space_on_a_skipped_item_restores_it_and_says_so` |
| `Enter` on a check opens its whole output, escaped, the cut said | `panels::enter_on_a_check_opens_its_whole_output_in_the_viewer` |
| A report opens the aggregates' file no more while it is the one read; one put in place by another process, even during a report's read, is read again; what was kept is given back after a quiet minute | `cortex-core::it usage::the_aggregates_are_read_again_only_when_they_are_not_the_files` (the read counter of `cortex-stats`, and a hold at `AGGREGATES_READ`) |
| The aggregates kept are used only for the same file, unchanged, and only those read whole by a report that did not save them are kept; a file saved again at the same length and time is another | `cortex-core usage::tests::the_aggregates_kept_are_used_only_for_the_same_file_unchanged`, `a_file_saved_again_at_the_same_length_is_another` |
| Screens 2a and 2b in their real states | `cortex-tui::it screens::the_views_draw_the_reference_screens` |

The breaks are `mutations/step-24-views.breaks`.

## Open items

- **Closes** `config-provenance-in-the-application` (its file deleted; the core's half was the first part's): `/config` shows each value's layer, file and line.
- **Adds** [user-theme-files](../../../outstanding/user-theme-files.md) and [the-files-a-command-changed](../../../outstanding/the-files-a-command-changed.md).
