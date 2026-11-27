# Step 28 fixes: what an approval shows, and the settings it says

Findings 11, 12, 15, 16 and 18 of the full review of step 28, the stale module doc of `ext/manage.rs`, and the review's lower-confidence notes of the prompt and pins areas. The trust and approval stores in `cineris-policy`, and the formats of `pins.toml` and `trust.toml`, are not touched.

## Finding 11: an extension's approval did not name the environment its programs get

- **What it was.** `cineris.exec` gives a program a shell command's environment (the providers' keys and `shell.env_remove` taken out): `GH_TOKEN`, `GITHUB_TOKEN`, a cloud's keys, `SSH_AUTH_SOCK`. A hook command's approval says so before the yes; an extension's (`ext::found::in_words_where`, whose words the trust question, `cineris permissions trust`, `cineris ext add` and `cineris ext inspect` show) said nothing of it.
- **The fix.** Wherever `exec` is not empty, confined or not, the user's own or a project's, the words say the hook commands' own sentence after what the programs can do: `cineris_ext::hook::command::ITS_ENVIRONMENT`, re-exported as `cineris_core::ext::ITS_ENVIRONMENT`, one source for both. An extension that runs no program says nothing of it.
- **Tests:**
  - `cineris-core ext::found::tests::an_approval_says_its_programs_run_with_your_environment`: confined and not, the user's and a project's, once, before its files; never for one that runs nothing. On `main` it fails: the sentence is not in the words.
  - Each surface: the trust question (`cineris-core::it extensions_found::a_projects_extension_walks_through_its_trust` and `extensions_approvals::a_projects_extension_is_approved_seeing_its_environment_and_its_prompt`), `cineris permissions trust` (`Core::project_extensions` in the latter, and `cineris-cli::system commands::sys_permissions_trust_approves_a_projects_extensions_only_with_the_flag_where_none_can_ask`), `cineris ext add` (`cineris-cli::bin/cineris ext::tests::ext_add_shows_what_it_registers_and_its_prompt_before_the_question`, its manifest given `exec = ["gh"]`) and `cineris ext inspect` (`extensions_approvals::ext_inspect_shows_an_extensions_environment_and_its_prompt`).
- **Break:** `approval-hides-the-environment`.
- **Docs:** 06, "What it lists", and security.md §3's hook-commands row.

## Finding 12: indented front matter hid a command's `profile`

- **What it was.** `FrontMatter` (`cineris_ext::skills`, shared by skills and custom commands) passed over an indented line before any key, and folded one after a key into its value; `commands::parse` refuses only what `unread` lists, which was lines at the first column. `---\n  description: Deploy\n  profile: careful\n---` (an indented mapping, valid YAML) was taken with no profile and sent at the session's own settings, and `description: Ship it\n\tprofile: careful` became the description "Ship it profile: careful". The review's `review_an_indented_profile_is_not_passed_over` failed on a runner at `5eda6204` (45939147363): both files taken with no profile.
- **The fix.** `unread` now also lists an indented line that belongs to no field (before the first, or after a line that is none), and a line of a plain value after its first that holds a `key: ` of its own (`keys_within`), which YAML refuses. A quoted value or a `|`/`>` block may hold `key: ` as text, a comment is none, and a value may still go on over indented lines. The strict reader (commands) refuses them; nothing else reads `unread`.
- **Skills.** A skill's front matter is read by the same `fields_of`; its reader takes the fields and does not look at `unread`, so a skill reads exactly as before: a description that wraps onto an indented line holding `: ` is one description, and an indented key is passed over as before. Chosen so because a skill's fields only name and describe it, and refusing a skill whose description YAML would refuse would leave out skills other tools read.
- **Tests:** `cineris-ext commands::tests::a_key_written_indented_is_not_passed_over` (the review's three files, the tab case among them, each refused naming its line, and the first-column one read; a wrapped description, a quoted value and a block holding `profile: ` read; `description:` with a nested `profile:`, and a list item, refused) and `cineris-ext skills::tests::what_a_strict_reader_refuses_is_told_and_a_skill_reads_as_before`.
- **Breaks:** `front-matter-passes-over-an-indented-key`, `front-matter-folds-a-key-into-a-value`, `front-matter-refuses-a-quoted-value`.
- **Docs:** 12-extensions' "The front matter is read strictly" (its line 113), the doc of `FrontMatter::unread`, and a line in [a-commands-settings-profile-for-its-turn](../../../outstanding/a-commands-settings-profile-for-its-turn.md).

## Finding 15: skill-folder reads turned off could stay on

- **What it was.** `cineris setup skill-folders` and the first run wrote `permissions.read_skill_folders` and said reads were asked about, without loading the configuration in force: a managed file that sets it and locks it, or the profile the user's file selects, keeps it on.
- **The fix.** `Core::skill_folders_in_force` loads the configuration as `ext disable` loads it after its write (`configuration_here`, reused) and gives the value with where it comes from, in `cineris config show --origin`'s words. `cineris setup skill-folders` loads it after its write, in the working directory's project, and `otherwise_in_force` says when it differs ("But the configuration in force here still reads your own skill folders without asking, and what is read, secrets included, goes to the model's provider: `permissions.read_skill_folders = true` comes from …, whose word stands over your file's."), or that it could not be checked. The first run says the same after its file is written: `Setup::in_force_otherwise`, a method of the first run's trait with an empty default, asked by the worker only once the file is written, its sentences said after "Written to …". Step 25's fixes, which change the first run and `setup.rs`, had not merged when this was written (`origin/main` at `5eda6204`); none of their lines is changed here.
- **Tests:** `cineris-cli::bin/cineris setup::tests::what_the_configuration_in_force_says_otherwise_is_said`, `setup::tests::the_first_run_says_when_a_managed_lock_keeps_skill_folders_read` (a managed file that sets and locks it), `cineris-cli::system commands::sys_setup_skill_folders_off_says_when_the_profile_keeps_them_read` (the profile the user's file selects) and `cineris-tui app::tests::first_run::what_the_configuration_in_force_holds_otherwise_is_said_after_the_write`.
- **Breaks:** `setup-says-nothing-of-the-value-in-force`, `first-run-asks-nothing-of-the-value-in-force`, `first-run-says-nothing-of-the-value-in-force`, `in-force-is-not-loaded`.
- **Docs:** 08, "First run" (`cineris setup`), and 09's skill folders' step.

## Finding 16: a project extension's section 7 text was not shown before approval

- **What it was.** The trust question and `cineris ext inspect` said only "It adds text of its author's to the system prompt"; `cineris ext add` shows the text whole (the early review's M2).
- **The fix.** `ext::found::prompt_shown` gives the text, quoted and escaped through `quoted_shown`, out of the snapshot already read; one source for the trust question's detail (and so `cineris permissions trust`), `ext inspect` (a new field, `ExtensionListed::prompt`) and `ext add`. Each shows it after `PROMPT_SHOWN`, "What it adds to the system prompt, whole, in its author's words:", on lines of its own.
- **The question's size.** The text is never cut, as `ext add` shows it: the manifest refuses a prompt file of more than 4 KiB (`cineris_ext::manifest::MAX_PROMPT_BYTES`), so that is the most it holds. A question too long for the application's dialog says how many lines are not shown and offers no yes there, naming `cineris permissions trust`, as it does of any list that does not fit; a cap would have the person approve text unseen, which is what the finding was. A long prompt is so approved from the command line.
- **Tests:** `cineris-core::it extensions_approvals::a_projects_extension_is_approved_seeing_its_environment_and_its_prompt` (a 4,096-byte prompt with a quote that would close and an escape sequence, shown whole and escaped in the trust question and `permissions trust`), `extensions_approvals::ext_inspect_shows_an_extensions_environment_and_its_prompt` (a project's and the user's), and `cineris-cli::bin/cineris ext::tests::ext_inspect_prints_the_text_it_adds_to_the_system_prompt`. On `main` the first two fail: no prompt is in the detail, and `ExtensionListed` has none.
- **Breaks:** `trust-question-hides-the-prompt`, `inspect-hides-the-prompt`, `inspect-prints-no-prompt`.
- **Docs:** 06, "What it lists"; 12-extensions' `cineris ext inspect`; security.md §3's trust-question row.

## Finding 18: `ext remove`'s place check never refused

- **What it was.** `ext::manage::within` compared the target's folder, resolved, with the user's folder of extensions, resolved; the target is always `root.join(name)` of a name `folder_name_refused` passed, as the folder's listing or its pin holds it, so it compared the folder with itself. Its break was caught only by a unit test that called it with targets production never builds.
- **Chosen: the dead check taken out, the name rules named as the guard.** Deleting relative to a held handle needs a new primitive in `cineris_sandbox::unfollowed`: a recursive delete through descriptors on Linux (`unlinkat`, `AT_REMOVEDIR`) and, on Windows, `NtCreateFile` with `DELETE` relative to the folder's handle and `FileDispositionInfoEx`, with the read-only attribute, placeholders and handles held without sharing deletion to mind, shown only on a runner. That is not small, and the window it closes needs write access to the configuration folder; [a-folder-is-deleted-by-its-path-after-its-check](../../../outstanding/a-folder-is-deleted-by-its-path-after-its-check.md) stays step 32's, its text corrected.
- **Test:** `cineris-core ext::manage::tests::a_path_that_leads_out_of_the_folder_of_extensions_is_not_deleted`, now through the core as `cineris ext remove` calls it: `..`, `../../elsewhere/x`, an absolute path and `inner/x` (and a `\` path on Windows) are refused before anything is looked at, and nothing outside is deleted; the removal of `Tracker` goes by the folder's name `tracker`, and its yes deletes that folder alone.
- **Break:** `step-28/remove-deletes-outside-the-folder`, now of `folder_name_refused`'s call in `Core::extension_removal`.
- **Docs:** 12-extensions' `cineris ext remove`, security.md §3's `ext add` row, the doc of `Core::remove_extension`, and the outstanding item.

## The module doc of `ext/manage.rs`

It said `add`'s evaluation came "after the person's yes"; since the early review's M2 it is before the question. It now says so, and what the question shows.

## The lower-confidence notes

Each went to an item, with its `- Owner:` line:

- `extensions/status` with no `session_id` answering any client, observers included, and reading and hashing the user's folders on every call: [extensions-status-without-a-session](../../../outstanding/extensions-status-without-a-session.md), step 32.
- `/a:b` always read as a command, and `Core::early` keeping an entry when the lane is gone: [rough-edges-of-step-28s-custom-commands](../../../outstanding/rough-edges-of-step-28s-custom-commands.md), a later step.
- A stamp walked through a link (`ext/watched.rs`, `walk`), with the Windows junction to a share: a line in [a-file-is-opened-by-name-after-its-decision](../../../outstanding/a-file-is-opened-by-name-after-its-decision.md), whose earlier text said the stamps look and read nothing.
- A pinned user's extension whose folder is gone freeing its name: [a-gone-users-extension-frees-its-name](../../../outstanding/a-gone-users-extension-frees-its-name.md), step 32.
- `trust.toml` read through a link at its name: [the-trust-store-is-read-through-a-link-at-its-name](../../../outstanding/the-trust-store-is-read-through-a-link-at-its-name.md), step 32; the store is another session's to change now, so nothing of it is changed here.

## Twins of the mechanisms

- **A front-matter reader that passes over indented lines:** `FrontMatter` is the only one; skills and commands share it, and the index's Markdown outline skips front matter without reading it.
- **An approval of a program that does not name its environment:** a hook command's says it, and now an extension's. An extension's single `cineris.exec` call is asked about as a shell command is, and a shell command's question does not say it either: left so, as a command's own question.
- **A setting written and said without loading what is in force:** `ext disable` loads it; `setup skill-folders` and the first run's skill folders now do. The first run's mode is a twin left open: it says "Mode: …" as chosen where a managed file may lock another; reported to the maintainers.
- **A check that compares a path with itself:** none other found among the `same_location` and `location_within` callers.

## Runs

(Filled in below as they finish.)
