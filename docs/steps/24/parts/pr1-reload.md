# Step 24, its parts: the reload through the core

The first of step 24's three pull requests: `config/reload` through the core, the reads the panels make, and the protocol they need ([answered.md](../answered.md), "The design, before building", points 1 and 9, with the coordinator's answers).

## What was built

- **`config/reload`** (`cortex-core`'s `reload`): the session's configuration in force and last loaded (`SessionConfig`), the look at the configuration's files and the trust store's (`LiveConfig::has_changed`, a `WatchList` of `trust.toml`), trust judged first (`assemble::judge`), one load with it (`LiveConfig::update`), the session's reload checked whole (`prepare`) and applied (`put`), the keys that wait given to the loop (`install_epoch`), and `config.changed` to every client of a session it changed. The core's own configuration (`CoreConfig`) takes its `at once` keys (`log.level`, `stats.retention`, `usage.*`) and keeps the rest. The look runs before each turn's prompt when no turn runs.
- **The configuration in force is a mix,** `cortex-config`'s `Loaded::taking`, by key, from the schema's declarations.
- **The fourth effect, `next session`,** for `sandbox.enabled`, `sandbox.protect`, `shell.*`, `compat.*`, `index.map_wait` and `index.map`; `usage.*` and `stats.retention` at once. The reference is regenerated.
- **The loop's two slots** (`cortex-agent`'s `reconfigure`): `SessionHandle::reconfigure` (the accounts and the turn's budget, the compaction's thresholds, `verify.rounds`), taken as a request is prepared; `SessionHandle::at_next_epoch` (the context's settings and the roles' models, taken by the compaction that begins an epoch, as its new prompt is built). `Account::with_budget`, `Ledger::set_accounts` and `set_turn_budget`; `SessionHandle::set_trust`.
- **What a reload reaches, made live:** the verifier's settings (`VerifyLive`), the sandbox's launcher (`SessionSandbox::policy_for`, built off the core's thread, and `set_policy`), the shell's trust and key variables (`cortex_tools::ShellLive`), the base capture's trust (`BaseCapture::start_watching`), the statistics' retention (`Recorder::set_retention`), the profile's and the map's budgets, the log's level (`CoreOptions::log`, given by the interactive binary unless `--log-level` or `CORTEX_LOG` chose it).
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
| A broken file keeps the configuration and is said once, again when asked | `reload::a_broken_file_keeps_the_configuration_and_is_said_once` (the user's file, which the core's own configuration says too), `a_broken_project_file_is_said_once_by_the_looks_and_again_when_asked` (the session's alone, with another file changed beside it) |
| A new allow rule of a trusted project is never in force | `reload::an_allow_rule_added_while_trusted_is_not_in_force` |
| Trust taken away outside is seen by the next look, and takes what trust gave | `reload::trust_taken_away_from_outside_is_seen_by_the_next_look` |
| A project's file cannot raise a user-only key or loosen a tighten-only one | `reload::a_reload_keeps_a_projects_file_to_what_it_may_set` |
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

## The second review

A second reviewer read the changes since the first review (the hold point, the fixes, their tests and breaks). Its findings, and what each changed; each fix has a test and a break.

| Finding | Fix | Test |
|---|---|---|
| **Must fix.** The loop kept a change for the next epoch once a compaction began; one that began no epoch (nothing to compact, interrupted, a failed summary) kept it past a withdrawal, so a project's model could come in after trust was taken away | The loop reads the slot only as an epoch builds its prompt; a compaction that begins none leaves it where the core can still replace or withdraw it | `cortex-agent::it reconfigure::a_change_withdrawn_after_a_compaction_with_no_epoch_is_never_taken` |
| **Must fix.** `session/set` applied its models and level outside the reload's lock: a reload building from the files put its models over the choice, which every later reload then treated as applied | `session/set` holds the session's reload lock across its change and its record | `reload::a_reload_never_comes_between_a_choice_and_its_record` (the choice held at a point between the two, `CHOICE_BEING_KEPT`; the reload beside it) |
| **Must fix.** A yes the store could not keep was taken back by the first look, which asked the store | The session keeps the rules the person saw (`SessionConfig::for_the_run`), and its looks judge against those | `trusting::trust_for_this_session_only_holds_through_a_look` |
| **Must fix.** The core's own reload held the configuration's lock across file reads, and two could interleave | Its files are a lock of their own (`CoreWatched`), and its reloads one at a time (`core_reloading`) | By reading |
| The interleaving test passed without the lock for most seeds | Fixed with the mutate run's own finding: the second reload is polled past its inline work's longest wait | `reload::two_reloads_of_a_session_never_interleave` |
| A choice left its key waiting for the epoch, which then took the file's value into the configuration | A choice takes its key out of what waits, and its roles out of the change the loop holds, which the session keeps (`EpochKept`) so that nothing is built again | `reload::a_choice_takes_its_key_out_of_what_waits_for_the_next_epoch` |
| A model chosen without the files' reasoning level refused every reload that built models again | Such a role runs at the model's own default, as `session/set` makes it (`assemble::models_lenient`) | `reload::a_chosen_model_without_the_files_reasoning_level_does_not_refuse_a_reload` |
| The load's check of the pins said trust the store took away as rules that changed, and kept the repository's trust | The check is the judgement again on what was loaded (`assemble::judge_loaded`), said as it finds | `reload::trust_taken_away_between_the_judgement_and_the_load_is_said_as_untrusted` |
| With trust taken away and a part that fails, what waited for the epoch was listed but not given to the loop | What can be built is given; what can fail waits with the rest | `reload::trust_taken_away_with_a_part_that_fails_still_gives_the_epoch_its_keys` |
| What waits for the next reload was promised a retry that a look with no change of file never made | Said as it is: tried again at the next change of a file, or with `/reload` (a retry at every turn would say the same error at every turn) | — |
| A reload could commit over what the loop took for an epoch meanwhile, or put an older model back | It commits on top of what is in force then, and builds again when the loop took an epoch while it built (`SessionConfig::generation`) | By reading: the loop's taking cannot be held between a reload's build and its put without a point in the loop |
| A `/profile` after a choice no reload had taken could keep the choice and say it was dropped | A choice is taken into the configuration in force when it is made | `reload::a_choice_is_in_the_configuration_config_get_answers_with` |
| An emptied table was dropped where the newer configuration had it, empty | No change: the load drops an empty table as it reads it, so a configuration taken from never has one (a test that wrote one found it gone) | — |
| The design note still said the context settings are taken before the compaction chooses what to keep | Said as built | — |

**Left:** the trust store's file is stamped after the session's start read it, so a change in the moment between is seen at the next change of a file or `/reload`, not by the next turn's look. Stamping it earlier means threading the stamp through the trust question's look; the window is the session's own start.

**Found by the mutate run** after the first review's fixes: three tests no longer bit their breaks because a second guard now held (the load's check of the pins; the core's own report of a broken user file; the session's stamps), and one (`a_choice_made_in_the_session_outranks_the_files`) never had: its file asked for `supervised`, the default. The breaks now name the tests that catch them; a broken project file has a test of its own, with another file changed beside it; the vacuous test is gone, its rule held by `a_reload_never_undoes_a_model_or_a_level_chosen_in_the_session`.

**Found by the stress run** (`stress -p cortex-core -p cortex-agent -p cortex-tools -p cortex-config --count 100`, seed 43 of 100): the look before a turn asked the loop's state, which stays idle until the loop picks up a turn a prompt has started. A second prompt sent at once looked; the look's blocking work let the first turn end; and the second, meant to queue, started a turn of its own (`approvals::a_wait_for_a_question_goes_on_while_a_turn_is_queued`). The look is now made only when no turn holds the session, asked as a prompt decides (`SessionHandle::turn_running`); the promote's check asks the same. Test: `reload::a_prompt_queued_behind_a_turn_not_yet_begun_does_not_look`, deterministic, with its break.

## CI times

<!-- written by cargo xtask ci-times, section "CI times": it replaces this up to the end mark -->
Pull request 87, on `main` at `327cfaa`. "Main" is main's own run at `327cfaa` (run 43947714781).
The PR's run is 43956687369, at `a66fafc`.

| Job | Main, `327cfaa` | The PR at `a66fafc` |
|---|---|---|
| `lint-linux` | 55 s | 73 s |
| `test-linux` | 104 s | 138 s |
| `lint-windows` | 88 s | 89 s |
| `test-windows` | 283 s | 188 s |
| `pr-size-linux` (blocks nothing) | — | 379 s |
| `pr-size-windows` (blocks nothing) | — | 286 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `a66fafc` (6 ms) | Windows, main (8 ms) | Windows, `a66fafc` (5 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 12.6 s | 14.5 s | 14.6 s | 9.0 s |
| Tier 2, integration (40 s) | 34.2 s | 29.6 s | 38.2 s | 24.6 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 21.6 s | 26.0 s | 81.6 s | 56.6 s |

<!-- end of what cargo xtask ci-times wrote, section "CI times" -->

## Windows, both trees on one runner

<!-- written by cargo xtask ci-times: it replaces this up to the end mark -->
`try` with `-f against=main` (run 43954436047), the median of 5 runs each, on one runner at 5.0 ms a process start:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 6.8 s (2,504 tests) | 7.2 s (2,500) | −0.4 s | 20 s |
| 2, integration | 19.6 s (1,469 tests) | 19.1 s (1,431) | +0.5 s | 40 s |
| 3, system | 50.0 s (185 tests) | 49.6 s (185) | +0.4 s | 90 s |

<!-- end of what cargo xtask ci-times wrote -->
