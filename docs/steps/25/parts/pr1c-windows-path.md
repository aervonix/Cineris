# Step 25, a follow-up to the rename: the core's test of a command's environment reads the whole output

After the project was renamed, `cineris-core::it shell::sys_a_command_is_given_the_environment_the_command_analysis_relies_on` failed on every hosted Windows runner, on `main`'s code and on a branch above it, in all five rounds of try run 44391435700. It had passed at `e2f1941d`, an hour before the rename. The coordinator had the cause found, as a part of its own, while the rename's own record was closed.

## What was found

The assertion was that every entry of the `PATH` a command is given is an absolute directory, and it named the entry it judged not to be one. `try-result` shows the first lines of a failure, and cut this one; the job's log (`cargo xtask ci-log 135109873515 --grep "PATH a command was given"`) holds the message whole. The entry was `` `C… [+1,201 characters]` ``, and it was not a part of the message that was cut: it is the whole entry. The `PATH` it was split from was 1,995 characters, 54 entries, and the 54th was that one.

That is the output policy's mark for a line longer than the line limit (`docs/architecture/05-tools.md`, "One output policy": 2,000 characters, and a longer line is cut and ends with a marker that says how many characters are missing). The command was `cmd /c set`, which prints `Path=…` on one line; on the runner `PATH` is about 3,180 characters, so the line was 3,185, of which the model is shown 1,979 and the marker `… [+1,201 characters]`, 2,000 in all. The test read the environment from the model's copy of the output, split its `Path` line at `;`, and took the piece after the last separator for an entry: the first character of the 54th entry and the marker.

## The cause

**The product is right; the test read the wrong copy.** The shell tool gives a command a `PATH` of absolute directories only (`crates/cineris-tools/src/shell/env.rs`, `absolute_entries`), and the model is shown its output within the limits, with the whole output kept as an artifact, as the design says. The test's check of the last entry looked at a fragment, and whether the fragment looked absolute depended on where in an entry the 2,000th character fell: in the first three characters of one (`C`, `C:`, or nothing) it is relative, further in it is `C:\P…` or the like, which `Path::is_absolute` takes for absolute. So the test passed or failed by the length of the runner's `PATH`, and checked nothing past its first 1,974 characters.

**What the rename changed is only where the cut fell.** Five entries of the runner's `PATH` hold the checkout's path: `target\ci\build\ring-…\out`, `target\ci` and `target\ci\deps`, which cargo and nextest put first for the test binary, and `target\debug` and `target\debug\deps` for `xtask`'s own run. The checkout moved from `D:\a\<old name>\<old name>` to `D:\a\Cineris\Cineris`: the old name is seven characters longer, so each of those entries is 14 characters shorter. On the same `PATH` with the old names, the cut falls 28 characters into the 52nd entry (`C:\Program Files (x86)\WiX T…`), absolute to the test; with the new, one character into the 54th. The A/B's base side, checked out in `D:\a\Cineris\ab-base`, failed the same way, `ab-base` being as long as `Cineris`; with the old names its cut falls four characters into the 53rd entry (`C:\P…`).

**Nothing else was the cause.** No entry of the runner's `PATH` is relative or quoted: the 53 whole entries are all a drive and a root (one, `C:\\ghcup\bin`, with a doubled separator, which is still absolute). The order and the content are what the image and the build put there, with PowerShell 7's own folder first, which it adds itself at its start.

## What was built

| Where | What |
|---|---|
| `cineris-core`: `tests/it/shell.rs` | The test reads the environment from the whole output: the artifact the shell tool kept, read through `session/artifact` as a front end reads it (`whole_output`), with a check that it was not cut there. The model's copy is still checked for the provider key. |
| the same | Absolute directories of the test's own (`<the test's folder>/not-a-directory-NNN`) end the `PATH` it gives, more characters of them than a line may have (`OutputLimits::default().max_line_chars()`), so the `Path` line is cut in the model's copy on every machine, Linux included, and the test does not depend on the runner's `PATH` or the checkout's path. They must arrive whole and in order at the end of the `PATH` the command was given, so nothing of a long `PATH` can be lost unseen. A failure names every entry that is not absolute with its place, and the whole `PATH`. |

Nothing of the product changed, and no doc: the output policy and the environment are as `05-tools.md` describes them.

## The tests and the breaks

| Rule | Test | Break |
|---|---|---|
| `PATH` keeps every absolute directory it was given, in order, however long it is | `cineris-core::it shell::sys_a_command_is_given_the_environment_the_command_analysis_relies_on` | `step-25-pathfix/path-loses-what-follows-its-first-entries` (keeps the first 32 entries) |

The unit tests of `absolute_entries` give it four entries at most, so they cannot see the break; before this change the system test could not either, since it checked only that what it read was absolute. The six breaks of step 13a that name this test (`path-keeps-relative-entries` among them) are unchanged and still apply.

## Elsewhere: what else assumes the repository's folder or path

Looked for: the old and the new name as a folder (either in a path), the runner's `D:\a\…`, `CARGO_MANIFEST_DIR` or `CARGO_TARGET_DIR` taken apart by name, and `env!("CARGO_…")` paths compared as text. Nothing found that depends on them:

- Every `CARGO_MANIFEST_DIR` is joined with a relative path (`../..`, `fixtures`, `snapshots`), never split or compared; `xtask`'s `workspace::root()` is its parent. `CARGO_TARGET_TMPDIR` is only a place for scratch folders, now 14 characters shorter on a runner, which can only take paths further from Windows' 260.
- No test, workflow or script names `D:\a\` or the repository's folder.
- The screen tests that wait for the product's name (`cineris-cli`'s system tests) wait for `Cineris <version>`, which no path holds; `cineris-tui`'s look for `Cineris` in screens of made-up sessions, with no real path on them.
- This test was the only one that read a command's environment, or any value, from a long line of the model's copy of an output.

## Runs

- **Before:** try run 44391435700 (Windows, `main` against a branch above it, five rounds each): the test failed in every round on both sides with the entry above; the message whole is in job 135109873515's log.
- **The fix, every test on Windows with `main` beside it on one runner** (try run 44396482438, at 9967057, the same change before one assertion was formatted, 6 ms a process start, five rounds): the test passed in every round on the branch and failed in every round on `main`, as before. The branch's other failure there, `xtask`'s `sys_the_repository_follows_its_own_rules`, was this record missing: the break list names it, and it was committed after that run. The tiers against `main`'s: +0.9 s, +0.8 s and +0.6 s, each within the spread of one side's own rounds (the unit tier ran 13.6 to 17.9 s on the branch and 14.3 to 16.5 s on `main`). The changed test is not in those figures: having failed on `main`, it is left out of both sides' medians, so nothing else changed and the differences are the runner's.
- **At the desk:** `cineris-core`'s tests and its dependents' (`test --dependents -p cineris-core`): 604 passed. The core's shell tests 200 times over (`stress -p cineris-core --count 200`): every one passed every time. `cargo xtask check`, and clippy for Windows through `cargo xwin`: nothing found.
- **The break** (`mutate step-25-pathfix/path-loses-what-follows-its-first-entries`, at the desk): caught by the test it names, and by no other of `cineris-core`'s. Its first run was not judged, the desk's disk having stalled under twelve of `cineris-tools`' tests at once; it now runs `cineris-core`'s tests alone, where the test that catches it is.
