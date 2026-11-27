# Step 25 fixes: a file put back keeps its mode

One of the step 25 fixes: the review of step 25's finding 7. The design is in [07-sessions-and-context.md](../../../architecture/07-sessions-and-context.md), "Snapshots and undo" and the `checkpoint` row of "The log".

## What was wrong

A file a rewind or an undo made again lost its mode. A tracked `run.sh` (`0755`) removed by a command came back from `session/undo` at `0644`, no longer executable; a `0600` file edited by `write` and then removed came back from a rewind with "Put them back" at `0644`, readable by others. On Windows such a file came back with its folder's access rules.

**The cause.** A checkpoint kept a file's content and no mode: `FileVersion` was a hash, a size and whether the content was kept. The put-back (`cineris-session`'s `snapshot::put_back`) wrote the content with `write_atomic` and its default options, and `write_atomic` gives a file it creates `0o666` less the umask (on Windows its folder's rules). A file put back over one still there kept that file's mode, whatever it had become. The editing tools' seam (`cineris-tools`' `Checkpoints::before`) carried the bytes alone, and a command's change read from git carried only the content `git cat-file` gives. The fault is from steps 8 and 11, older than step 25's rewind path that the review was reading.

## What changed, and why

| Where | What |
|---|---|
| `cineris-base`: `mode.rs` (new) | `FileMode`: the nine permission bits of a file, never a set-user-id, set-group-id or sticky bit, so a mode read from a log a person can edit can never make a program run as someone else. `FileMode::of(&File)` asks an open handle, never a name: on Linux its mode, on Windows `FileMode::PRIVATE` for a file kept from everyone but its user and nothing for any other. `from_git` reads git's entry mode as a checkout makes the file (`100755` as `777`, `100644` as `666`, each less the umask when it is made); `to_octal` and `from_octal` its form in a log. |
| `cineris-base`: `atomic.rs` | `AtomicWriteOptions::mode`, `None` by default, for a write that puts a person's file back. A file it creates is created with the mode less the umask, from the moment its temporary file exists; on Windows a private mode makes it owner-only (as `owner_only` does) and any other leaves it its folder's rules. A file it writes over keeps its own mode less every permission bit the given one lacks. `owner_only` comes first. |
| `cineris-base`: `private/windows.rs` | `access_list_of(&File)`, the access list read through an open handle (`access_list` opens the path and calls it), and `AccessList::is_kept_from_others` (below). |
| `cineris-session`: `entry.rs` | `FileVersion::mode`, an optional field (below); `FileVersion::new` and `with_mode`. |
| `cineris-session`: `store.rs` | `DiskStore::put_file` records the mode of the handle it reads the content through. |
| `cineris-session`: `snapshot.rs` | `put_back` passes the kept version's mode to `write_atomic`. |
| `cineris-tools`: `checkpoint.rs`, `change.rs` | `Checkpoints::before(file, content, mode)`: the editing tools hand over the mode of the handle they read the file through, with the bytes. `MemoryCheckpoints`' `Change` keeps it (`mode`). |
| `cineris-core`: `checkpoints.rs` | `Rounds::before` records the tool's mode. A command's change read from git takes its mode from `git ls-tree -z <base_commit> -- ./<path>…`, one command for up to 256 paths, asked from the workspace so that git names the paths back relative to it, a workspace inside its repository included; a record cut short, or anything but a regular file's entry (a link, a submodule), gives no mode. |

**Why at the version, and from the handle.** The mode belongs to the version a rewind goes back to, so it is recorded where the content is, from the same open: the editing tools' handle, and `put_file`'s for a file the round reads itself. There is no second look at the file by its name, so nothing swapped in between can lend its mode. A command's change is seen only after it was made, so its mode is git's, as its content is.

**Why `write_atomic`, and not the sandbox's put-back.** The sandbox puts a protected path back with its mode already (`cineris-sandbox`'s `protect/restore.rs`), through descriptors opened with no link followed, on Linux only. `cineris-session` cannot depend on `cineris-sandbox` ([01-workspace.md](../../../architecture/01-workspace.md)), and the session's put-back already writes with `write_atomic`, which follows no link (`follow_links: false`) and has the umask, the owner-only path on Windows and the private temporary file the mode needs. So the mode went into `cineris-base`'s `write_atomic`, where any later put-back of a person's file can ask for it, and the two put-backs share `FileMode`'s rules in spirit: never a set-id bit, never wider than the file was.

**A file put back over one that is still there** keeps its own mode, less every permission bit the kept one lacks: never wider than it is now, nor than it was. So a private file a command replaced with one readable by others (a program that writes a new file and renames it over) is private again, and a file the person made private since, which a checkpoint cannot tell from the command's change, is not opened up. A bit the kept mode had and the file has lost since is not given back: a checkpoint does not record a change of mode alone, so the rewind cannot tell the person's `chmod` from the command's. The review's scenario, the removed file, gets its whole mode, less the umask.

**The umask still applies** to a file made again, as to any file a program creates: a file is never made wider than the process would make any file. A script that was `0775` under a umask of `002` comes back `0775`; under `022`, `0755`.

**Set-id bits** are never recorded and never set: `FileMode` holds the nine permission bits, a `"mode": "4755"` in a log reads as `755`, and a mode git gives is `777` or `666`. A file still there keeps its own set-id bits, as `write_atomic` always kept them.

**On Windows**, "private" is decided by the file's own access list, read through the handle the content is read through (`GetSecurityInfo` on it; a handle opened for reading has the right to read permissions): the list takes nothing from its folder (`SE_DACL_PROTECTED`), it has entries, one allows the current user, and no entry allows anyone but the current user, the SYSTEM account (`S-1-5-18`) and the Administrators group (`S-1-5-32-544`), which reach any file regardless; an entry of a kind that is not read counts against it, and a deny entry does not (`AccessList::is_kept_from_others`). An owner-only file is one; so is a file cut off from its folder with `icacls … /inheritance:r` and left to its user. Such a file is recorded as `"mode": "600"`, and made again owner-only: an access list with the current user alone and inheritance off, given to the call that creates the file (`private::create_new`), as every private file of the harness's own is made. Any other file records no mode and is made again with its folder's rules, as a new file is, which is what it had: its rules came from the same folder. A mode git gives (`777`, `666`) is not private, and changes nothing on Windows.

## The format change

**The session log: an optional field on a checkpoint's version**, approved by the owner. A `before` (and in principle an `after`, which this build never writes with one) may have `"mode"`, octal digits in a string: `{"hash": "…", "bytes": 10, "mode": "755"}`. On Linux the nine permission bits; on Windows `"600"` for a private file and no field for any other. It is additive:

- **It is written only where it is known.** A version whose mode is not known (a protected path a command changed, whose look before keeps none; a file's `after`) is written as it was before the field, byte for byte: `cineris-session` `entry::tests::a_versions_mode_is_written_only_when_known_and_never_loses_the_version` checks the line, and `put_back::unix::a_version_from_a_log_written_before_modes_is_made_again_as_before` writes such a checkpoint, reads the session back from disk and puts the file back with the default mode, as before.
- **A value this build cannot read is no mode**, never a reason to lose the version (`mode_text::read`): a number, a word, `null`, an object, an array or too many digits all read as a version without a mode. A `before` that failed to read as a version would have become `"unknown"`, a file never put back, which is why the field has a reader of its own.
- **An older build reads a newer log.** `FileVersion` refuses no field it does not know, and `Before`'s reader reads a version object through it, so a build from before the field reads `{"hash", "bytes", "mode"}` as the version it always was, and puts the file back with the default mode. **Shown on `main`'s code** (`69090915`, before this change) by a throwaway test run with `cargo xtask scratch-test cineris-session`, not kept, since it is a test of the old code: it wrote a checkpoint, edited its line on the disk into the form this build writes (`{"bytes":10,"mode":"755","hash":…}`), resumed the session, planned the undo and put the file back; it passed, the version read whole and the line left as it was. A fork copies a line as it is, so the field goes with it. The log's version is unchanged.
- **Nothing in `cineris-protocol` changed:** a rewind's plan names what a file goes back to, not its mode, and the plan's mark hashes the content alone. The golden transcript and the recorded fixtures are not touched.

07's `checkpoint` row says so.

## The tests

**On `main`'s code** (`69090915`), the three core tests, which use nothing this change added, were run as they are with `cargo xtask scratch-test cineris-core`, and each failed as the review found:

- `a_private_file_put_back_after_it_was_removed_is_private_again`: "the private file was put back with mode 644: other users can read it" (`420` against `384`);
- `sys_an_undo_puts_back_a_removed_script_with_its_execute_bit` and `…_in_a_workspace_below_its_repository`: "…/bin/run.sh was put back with mode 644: it is no longer executable" (`420` against `493`).

The other tests use the new API, and the breaks below show that each fails without its piece of the fix.

| Test | What it shows |
|---|---|
| `cineris-core::it put_back::sys_an_undo_puts_back_a_removed_script_with_its_execute_bit` | The review's first scenario: a tracked `bin/run.sh` and `tools/-dash.sh` (`0755`), removed with their folder by a command, put back by `session/undo` executable (`0777` less the umask). The name with a leading dash is passed to git after `--` and with `./`. |
| `cineris-core::it put_back::sys_an_undo_puts_back_a_removed_script_in_a_workspace_below_its_repository` | The same where the workspace is a folder inside its repository: git names the paths back from the workspace. |
| `cineris-core::it put_back::a_private_file_put_back_after_it_was_removed_is_private_again` | The review's second: `private.txt` (`0600`), named by the model as `./notes/../private.txt` and `notes/../private.txt`, edited by `write`, removed by the person, put back by a rewind answered "Put them back": `0600` again, and the log says `"mode":"600"`. |
| `cineris-session::it put_back::unix::a_removed_file_is_made_again_with_the_mode_it_had` | `0755` (in a folder removed with it), `0600`, `0640`, `0444` and `04755` made again with their modes less the umask; the set-user-id bit is neither logged nor set. |
| `cineris-session::it put_back::unix::a_file_put_back_over_one_still_there_is_never_left_wider` | Over a file still there: one opened up since is private again, one the person closed since stays closed. |
| `cineris-session::it put_back::unix::a_version_from_a_log_written_before_modes_is_made_again_as_before` | An older log: the line as it was, read back after a resume, the file made with the default mode. |
| `cineris-session::it put_back::unix::a_file_reached_through_a_link_is_not_put_back_with_its_mode` | Links inside the root: a folder on the way swapped for a link to another folder of the root, and the file itself a link to a file there; neither is put back, and what they lead to keeps its content and mode. |
| `cineris-session::it put_back::a_private_file_made_again_on_windows_is_owner_only` | **Windows only** (CI's Windows job): an owner-only file, removed and put back, owner-only again; an ordinary one recorded with no mode and made with its folder's rules. |
| `cineris-base::it atomic::unix::a_file_put_back_is_created_with_the_mode_it_had`, `…::a_file_put_back_over_one_still_there_is_never_left_wider` | `write_atomic`'s `mode`, made and written over. |
| `cineris-base::it atomic::windows::a_file_put_back_private_is_owner_only` | **Windows only**: a private mode makes a file owner-only, made or written over; another leaves it its folder's rules. |
| `cineris-base::it private::windows::a_file_is_kept_from_others_only_when_its_own_list_says_so` | **Windows only**: what "private" is, on lists built by hand. |
| `cineris-base mode::tests::*`, `cineris-core checkpoints::tests::git_modes_are_read_for_regular_files_alone`, `cineris-session entry::tests::a_versions_mode_is_written_only_when_known_and_never_loses_the_version` | The mode's own rules, git's answer read, and the log's field. |

**Paths.** The model's path through `.` and `..` (core); a workspace that is not the top of its repository, and a name with a leading dash (core); a folder made again with the file (session, core); a folder on the way and the file itself made links into the root (session). A `~` in a checkpoint's name is not expanded (a relative name is joined to the workspace root, as before), and this part changes nothing of how a checkpoint's name is found.

## The breaks

`mutations/step-25-fixes-put-back.breaks`:

Run with `cargo xtask mutate step-25-fixes-put-back --jobs 2`: 13 breaks, 13 caught, 0 flagged.

| Break | What it puts back of `main` | Caught by |
|---|---|---|
| `a-put-back-made-with-the-default-mode` | The put-back passes no mode (both of the review's scenarios) | the three core tests, and the session's `a_removed_file_is_made_again_with_the_mode_it_had` and `a_file_put_back_over_one_still_there_is_never_left_wider` |
| `a-new-file-ignores-the-mode-it-is-given` | A file made again gets `0o666` | `cineris-base::it atomic::unix::a_file_put_back_is_created_with_the_mode_it_had`, the session's `a_removed_file_…` |
| `a-file-written-over-keeps-a-wider-mode` | A file still there keeps whatever mode it has | base's and the session's `a_file_put_back_over_one_still_there_is_never_left_wider` |
| `a-file-written-over-takes-the-kept-mode-whole` | A file still there is opened up to the kept mode | the same two |
| `a-set-id-bit-read-from-a-log` | A set-id bit is kept and put back | `cineris-base mode::tests::only_the_nine_permission_bits_are_held`, base's `a_file_put_back_is_created_…`, the session's entry test and `a_removed_file_…` |
| `the-store-keeps-no-mode` | `put_file` records no mode | the session's `a_removed_file_…` and `a_file_put_back_over_…` |
| `the-editing-tools-hand-over-no-mode` | The tools hand over no mode (the second scenario) | `cineris-core::it put_back::a_private_file_put_back_after_it_was_removed_is_private_again` |
| `the-core-drops-the-tools-mode` | The core drops it (the second scenario) | the same |
| `a-commands-change-keeps-no-mode-from-git` | A command's change has no mode (the first scenario) | both core `sys_an_undo_puts_back_a_removed_script_…` tests |
| `git-modes-asked-from-the-top-of-the-repository` | Git asked with `--full-name`, which names paths from the repository's top | `…_in_a_workspace_below_its_repository` |
| `a-links-mode-taken-from-git` | A link's or a submodule's entry gives a mode | `cineris-core checkpoints::tests::git_modes_are_read_for_regular_files_alone` |
| `a-mode-that-cannot-be-read-loses-the-version` | An unreadable `mode` makes the version `"unknown"` | `cineris-session entry::tests::a_versions_mode_is_written_only_when_known_and_never_loses_the_version` |
| `no-mode-written-as-null` | `"mode": null` written where none is known | the same, and `put_back::unix::a_version_from_a_log_written_before_modes_is_made_again_as_before` |

`step-10/checkpoint-given-no-bytes` (`mutations/step-10.breaks`) no longer applied, since the tools now hand over the bytes and the mode as a pair. Its `find` was updated to the code as it is now, and it is still caught by 5 tests (`cineris-tools change::tests::a_change_that_goes_through_is_recorded_as_what_the_file_now_is` among them).

## Twins: other files made again without their mode

Every `write_atomic` caller, and every other writer of a file a person had, was read.

- **Fixed:** the rewind's and the undo's put-back (above), the one place that makes a person's file again from a record.
- **The sandbox's put-back** of a protected path keeps the mode already (`protect/restore.rs`). But the checkpoint a confined command's protected change records for undo (`cineris-core`'s `Rounds::record_protected`) has no mode: the sandbox's `protect::Changed` carries the content alone, so an undo that makes such a file again (a hook the command removed and the sandbox could not put back) makes it with the default mode. Left as it was, and said in 07: carrying the mode means a field on `cineris-sandbox`'s `Before::File`, outside this part. *Recommendation:* step 32 adds the look before's mode to `Before::File` and passes it to `Checkpoints::before`.
- **The editing tools' own write** (`write`, `edit`) and the project profile's (`cineris-knowledge`'s `project`) replace a person's file with `write_atomic`'s defaults: on Linux the mode is kept; on Windows a private file gets its folder's rules, which `write_atomic` documents and its tests hold. It is not a put-back, so it is left, as a new open item: [the-editing-tools-write-a-private-file-open-on-windows](../../../outstanding/the-editing-tools-write-a-private-file-open-on-windows.md), with a one-line fix for the tools.
- **`/export`'s writer** (`cineris-cli`'s `export`) and **the first run's file** (`cineris-config`'s `write_user_config`) never write over a file: each creates a new private file and names it with a hard link that refuses a name already taken. Nothing a person had is made again.
- **The harness's own files** (`cineris-config`'s state and credentials, `cineris-policy`'s approvals, trust and pins, `cineris-stats`, `cineris-index`, `cineris-core`'s shell cache and extension state, `cineris-ext`'s cache, `cineris-cli`'s history, `cineris-session`'s index and fork) are written `owner_only`, or replaced with their mode kept: none is a person's file made again.

## Docs changed

- [07-sessions-and-context.md](../../../architecture/07-sessions-and-context.md): the `checkpoint` row (`mode`, and what an older build does with it), and "Snapshots and undo" (a file made again keeps its mode; how it is recorded; a file still there; Windows; a version with none; the `ls-tree`).
- [05-tools.md](../../../architecture/05-tools.md), "Checkpoints": the seam's `before(file, content, mode)`.
- [security.md](../../../security.md), the row on the file system changing between a check and a use: the checkpoint's mode is asked of the open handle, and the rewind's put-back (its look again before the write, the link refused, the mode made again and never wider).
- A new open item, [the-editing-tools-write-a-private-file-open-on-windows](../../../outstanding/the-editing-tools-write-a-private-file-open-on-windows.md). The index is written by the session.

## Public items changed

All additive except one signature:

- `cineris_base::FileMode` and `cineris_base::mode` (new); `AtomicWriteOptions::mode` (new field; every literal in the workspace uses `..Default::default()`); `cineris_base::private::access_list_of` and `AccessList::is_kept_from_others` (Windows, new).
- `cineris_session::entry::FileVersion::mode` (new field), `FileVersion::new`, `FileVersion::with_mode` (new).
- `cineris_tools::Checkpoints::before` takes a third argument, `mode: Option<FileMode>` (**changed**; its three implementors are in the workspace and are changed with it); `cineris_tools::checkpoint::Change::mode` (new field).

## Left open

- The protected path's mode in an undo's checkpoint (above), for step 32.
- [the-editing-tools-write-a-private-file-open-on-windows](../../../outstanding/the-editing-tools-write-a-private-file-open-on-windows.md), for step 32.
- The Windows tests above run only on a Windows runner. `cargo xwin clippy` for Windows passes on them.

## Measured

- **Stress:** with `cargo xtask stress --count 100`, cineris-session's 123 tests and cineris-core's 27 put-back and checkpoint tests all passed every time, seeds 0 to 99.
- **Cost:** `put_file` asks the handle it already holds for its mode (one `fstat` on Linux, and one `GetSecurityInfo` on Windows). A command's change read from git costs one more `git ls-tree` for up to 256 restorable paths. Neither was timed on its own.
