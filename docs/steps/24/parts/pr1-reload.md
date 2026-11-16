# Step 24, its parts: the reload through the core

The first of step 24's three pull requests: `config/reload` through the core, the reads the panels make, and the protocol they need ([answered.md](../answered.md), "The design, before building", points 1 and 9, with the coordinator's answers).

## What was built

- **`config/reload`** (`cortex-core`'s `reload`): the session's configuration in force and last loaded (`SessionConfig`), the look at the configuration's files and the trust store's (`LiveConfig::has_changed`, a `WatchList` of `trust.toml`), trust judged first (`assemble::judge`), one load with it (`LiveConfig::update`), the session's reload checked whole (`prepare`) and applied (`put`), the keys that wait given to the loop (`set_next_epoch`), and `config.changed` to every client of a session it changed. The core's own configuration (`CoreConfig`) takes its `at once` keys (`log.level`, `stats.retention`, `usage.*`) and keeps the rest. The look runs before each turn's prompt when no turn runs.
- **The configuration in force is a mix,** `cortex-config`'s `Loaded::taking`, by key, from the schema's declarations.
- **The fourth effect, `next session`,** for `sandbox.enabled`, `sandbox.protect`, `shell.*`, `compat.*`, `index.map_wait` and `index.map`; `usage.*` and `stats.retention` at once. The reference is regenerated.
- **The loop's two slots** (`cortex-agent`'s `reconfigure`): `SessionHandle::reconfigure` (the accounts and the turn's budget, the compaction's thresholds, `verify.rounds`), taken as a request is prepared; `SessionHandle::at_next_epoch` (the context's settings and the roles' models, taken by the compaction that begins an epoch, as its new prompt is built). `Account::with_budget`, `Ledger::set_accounts` and `set_turn_budget`; `SessionHandle::set_trust`.
- **What a reload reaches, made live:** the verifier's settings (`VerifyLive`), the sandbox's launcher (`SessionSandbox::set_grant`), the shell's trust and key variables (`cortex_tools::ShellLive`), the base capture's trust (`BaseCapture::start_watching`), the statistics' retention (`Recorder::set_retention`), the profile's and the map's budgets, the log's level (`CoreOptions::log`, given by the interactive binary unless `--log-level` or `CORTEX_LOG` chose it).
- **`session/set` records its choices** (`reload::chosen`): they outrank the files, and a role so set no longer waits for the next epoch.
- **An epoch read without variants** is fixed to those in force as it is read (`Context::fix_variants`).
- **The reads:** `session/artifact` and `workspace/warm` (`cortex-core`'s `reads`); `project/list`, `session/artifact` and `workspace/warm` in `cortex_rpc::lane::reads`, `usage/refresh` and `workspace/warm` always beside the lane.
- **`config/get`'s origins;** `UsageReport.notes` (a file set aside, lines not read); `SessionUsage.context`; `PlanEdit::Restore`, told to the model as a skip is; a promote refused with `turn_running` while a session of the project runs a turn.

## Where it departs from the design

| Design | As built | Why |
|---|---|---|
| `index.map` at the next epoch | `next session`, with the other keys a session is made with once | Whether a session has a map at all is decided as it is put together (the loop's `ContextParts::map`); `index.map_tokens` does wait for the next epoch. |
| The base capture's trust lowered at once | Read where the capture decides what to read (`start_watching`) | The capture runs once, as the session opens; a reload that lowers trust after that point has nothing left to stop. |
| `keep_recent_tokens` taken before the compaction chooses what to keep | Taken with the rest as the new epoch's prompt is built, for the compactions after | A compaction that begins no epoch must leave the epoch as it was (the review, finding 9). |
| `/reload --now` | An item of its own | The coordinator's answer. |

## Tests

| Rule | Test |
|---|---|
| A reload applies what takes effect at once and lists what waits, telling every client once | `cortex-core::it reload::a_reload_applies_what_takes_effect_at_once_and_lists_what_waits` |
| A broken file keeps the configuration and is said once, again when asked | `reload::a_broken_file_keeps_the_configuration_and_is_said_once` |
| A new allow rule of a trusted project is never in force | `reload::an_allow_rule_added_while_trusted_is_not_in_force` |
| Trust taken away outside is seen by the next look, and takes what trust gave | `reload::trust_taken_away_from_outside_is_seen_by_the_next_look` |
| A project's file cannot raise a user-only key or loosen a tighten-only one | `reload::a_reload_keeps_a_projects_file_to_what_it_may_set` |
| A choice made in the session outranks the files | `reload::a_choice_made_in_the_session_outranks_the_files` |
| A profile is for one session, by a person | `reload::a_profile_is_chosen_for_one_session_by_a_person` |
| `config/get` says where each value came from | `reload::config_get_says_where_each_value_came_from` |
| The start of a turn looks at the files | `reload::a_turn_starts_with_the_files_looked_at` |
| What waits for the next epoch comes in with a compaction | `reload::what_waits_for_the_next_epoch_comes_in_with_a_compaction`; `cortex-agent::it reconfigure::what_waits_for_the_next_epoch_is_taken_by_a_compaction_and_not_before` |
| What applies at once reaches the next request, what was spent kept | `cortex-agent::it reconfigure::a_reload_turns_automatic_compaction_off_from_the_next_request`, `a_reload_changes_the_limits_and_keeps_what_was_spent` |
| `[verify]` reloaded reaches the next round | `cortex-core::it verify::a_reload_of_verify_reaches_the_next_round` |
| A new key variable and lowered trust reach the next command | `cortex-tools shell::tests::a_reload_reaches_the_next_command` |
| The mix by key | `cortex-config::it reloading::taking_some_keys_keeps_the_rest_as_they_were` |
| An epoch read without variants keeps those of its resume | `cortex-agent::it context::an_epoch_read_without_variants_keeps_those_in_force_when_it_was_resumed` |
| A skipped item restored is told to the model | `cortex-tools::it plan::the_user_can_restore_a_skipped_item_and_the_model_is_told` |
| A promote waits for the turn to end | `cortex-core::it knowledge::a_promote_waits_for_the_turn_to_end` |
| An artifact is read back whole; reading ahead answers at once and reads only what it may | `cortex-core::it reload::an_artifact_is_read_back_whole_and_one_not_there_is_refused`, `cortex-core reads::tests::only_a_file_inside_the_workspace_the_gate_lets_through_is_read_ahead`, `a_cut_text_leaves_out_the_character_it_split` |
| The panels' reads run beside the lane; a refresh never waits in it | `cortex-rpc lane::tests::the_panels_reads_run_beside_the_lane_and_a_refresh_never_waits_in_it` |

The breaks are `mutations/step-24.breaks`. Seven breaks of earlier steps follow the code this part moved (`set_level` into `put_engine`, the launcher behind a lock, the effects of the keys that changed, the base capture's flag).

## The review

An independent reviewer read the whole diff against the design and the specification. Its findings, and what each changed; each fix has a test and a break in `mutations/step-24.breaks`.

| Finding | Fix | Test |
|---|---|---|
| A reload that rebuilds every role's model (`retry`) put back the file's model over one chosen with `/model`, and listed chosen keys as waiting | A key chosen in the session is taken into the configuration in force, whatever its effect, and never applied over or listed | `reload::a_reload_never_undoes_a_model_or_a_level_chosen_in_the_session` (a level the default does not equal, a chosen model, and a change of `retry`) |
| Trust taken away while a file is broken was lost: the session's files were first stamped by its first reload, which could not load | The files are stamped as the session is put together, by the load it was made from (`assemble::trust` loads through `LiveConfig`, so no load is added); a load that fails still lowers what needs no loading | `reload::trust_taken_away_with_a_broken_file_is_taken_away` |
| Trust taken away was refused whole when another part of the reload could not be built | What can fail for reasons of its own (models, providers, retry, a day's limit) waits and is said; trust's part is applied | `reload::trust_taken_away_holds_when_another_part_of_the_reload_fails` |
| The accounts a reload changed were never kept: the next change of a limit started from the session's first | Kept in `put` | `reload::a_limit_a_reload_added_stays_through_the_next` |
| A file written between the judgement's look-ahead and the load was loaded trusted unchecked | A load made trusted is checked against the pins on what it loaded (`assemble::pins_approved`), and made again untrusted when they differ | `reload::a_rule_written_between_the_judgement_and_the_load_is_not_in_force` (held at the point between the two) |
| Files were read on the core's thread (the grant's roots, the launcher's policy) | Built inside `Core::blocking`; the launcher is given a ready policy (`SessionSandbox::policy_for`, `set_policy`) | By reading; the lint's allow is the module's, as before |
| Two reloads of one session could interleave, an older one committing over a newer | One reload of a session at a time, look to commit (`Live::reloading`); `session/set`'s choices wait for it too | `reload::two_reloads_of_a_session_never_interleave` (the first held, the second does not reach the point) |
| A `/profile` whose apply failed still dropped the session's choices | The profile and the dropped choices are committed only once applied | `reload::a_profile_that_cannot_be_applied_keeps_the_sessions_choices` |
| What waits for the next epoch was taken when a compaction began, and a compaction that began no epoch left it half in force | Taken with the models as the new epoch's prompt is built | `cortex-agent::it reconfigure::a_compaction_with_no_new_epoch_leaves_what_waits_waiting` |
| `taken` recorded the latest configuration, not what the loop took | The change holds the configuration it was built from | By reading |
| The configuration's lock was held across files' reads, stalling `config/get` and the loop's `taken` | The files are a lock of their own (`Watched`), which only a reload takes | By reading |
| What waits for the next epoch was not checked | Built and checked with the rest, before anything is applied | By reading |
| Removing a key left an empty table, an entry of defaults | `merge::remove` removes the tables it leaves empty | `cortex-config::it reloading::taking_a_key_that_went_leaves_no_empty_table` |
| The core's own reload said every changed key as applied | It says only the keys it acts on (`log.level`, `stats.retention`, `usage.*`) | `reload::a_reload_never_undoes_a_model_or_a_level_chosen_in_the_session` |
| Docs: "every open session"; `load` lost its doc; 08's "checked whole" | Said as built | — |

Left, as the reviewer's "should consider": a test that a rule held back is held back by the engine's own decision as well as by `config/get`; and `ChecksProfile`'s lines, which a reload of `verify.commands` does not change (a check is still chosen from the verifier's own settings, which do change).
