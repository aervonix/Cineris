# Step 25, a follow-up to the rename: the core's test of a command's environment reads the whole output

After the project was renamed, `cineris-core::it shell::sys_a_command_is_given_the_environment_the_command_analysis_relies_on` failed on every hosted Windows runner, on `main`'s code and on a branch above it, in all five rounds of try run 44391435700. It had passed at `e2f1941d`, an hour before the rename. The coordinator had its cause found in a part of its own.

## What was found

The assertion was that every entry of the `PATH` a command is given is an absolute directory, and it named the entry it judged not to be one. `try-result` shows the first lines of a failure, and cut this one; the job's log (`cargo xtask ci-log 135109873515 --grep "PATH a command was given"`) holds the message whole. The entry was `` `C… [+1,201 characters]` ``, and it was not a part of the message that was cut: it is the whole entry. The text the test took for `PATH` was 1,995 characters, 54 entries, and the 54th was that one.

That is the output policy's mark for a line longer than the line limit (`docs/architecture/05-tools.md`, "One output policy": 2,000 characters, and a longer line is cut and ends with a marker that says how many characters are missing). The command was `cmd /c set`, which prints `Path=…` on one line; on the runner `PATH` is about 3,180 characters (3,175 exactly, and the line with its `Path=` 3,180). Of the line the model is shown 1,979 characters and the marker `… [+1,201 characters]`, 2,000 in all. The test read the environment from the model's copy of the output, split its `Path` line at `;`, and took the piece after the last separator for an entry: the first character of the 54th entry and the marker.

## The cause

**The product is right; the test read the wrong copy.** The shell tool gives a command a `PATH` of absolute directories only (`crates/cineris-tools/src/shell/env.rs`, `absolute_entries`), and the model is shown its output within the limits, with the whole output kept as an artifact, as the design says. The test's check of the last entry looked at a fragment, and whether the fragment looked absolute depended on where in an entry the 2,000th character fell: in the first three characters of one (`C`, `C:`, or nothing) it is relative, further in it is `C:\P…` or the like, which `Path::is_absolute` takes for absolute. So the test passed or failed by the length of the runner's `PATH`, and checked nothing past its first 1,974 characters.

**What the rename changed is only where the cut fell.** Five entries of the runner's `PATH` hold the checkout's path: `target\ci\build\ring-…\out`, `target\ci` and `target\ci\deps`, which cargo and nextest put first for the test binary, and `target\debug` and `target\debug\deps` for `xtask`'s own run. The checkout moved from `D:\a\<old name>\<old name>` to `D:\a\Cineris\Cineris`: the old name is seven characters longer, so each of those entries is 14 characters shorter. On the same `PATH` with the old names, the cut falls 28 characters into the 52nd entry (`C:\Program Files (x86)\WiX T…`), absolute to the test; with the new, one character into the 54th. The A/B's base side, checked out in `D:\a\Cineris\ab-base`, failed the same way, `ab-base` being as long as `Cineris`; with the old names its cut falls four characters into the 53rd entry (`C:\P…`).

**Nothing else was the cause.** No entry of the runner's `PATH` is relative or quoted: the 53 whole entries are all a drive and a root (one, `C:\\ghcup\bin`, with a doubled separator, which is still absolute). The order and the content are what the image and the build put there, with PowerShell 7's own folder first, which it adds itself at its start.

## What was built

| Where | What |
|---|---|
| `cineris-core`: `tests/it/shell.rs` | The test reads the environment from the whole output (`whole_output`): the artifact the shell tool kept when the model's copy was cut, read through `session/artifact` as a front end reads it, with a check that it was not cut there; the model's copy when nothing was cut, which is then the whole. The model's copy is still checked for the listing (`KEPT=yes`) and for no provider key. |
| the same | Absolute directories of the test's own (`<the test's folder>/not-a-directory-NNN`) end the `PATH` it gives: more characters of them than a line may have (`OutputLimits::default().max_line_chars()`, counted in characters), and at least 48 (`OWN_DIRECTORIES`), whatever the length of the test's folder. So the `Path` line is cut in the model's copy on every machine, Linux included, and the test does not depend on the runner's `PATH` or the checkout's path. They count down, and the last is there twice. |
| the same | The `PATH` the command was given must end with every absolute entry of the one the harness was given, the runner's own and the test's, as they were and in order (a shell may put its own before them, as PowerShell 7 puts its folder): an entry lost, moved, sorted or with a repeat taken out fails it, wherever in the `PATH` it is. A failure names every entry that is not absolute with its place, and both `PATH`s. Only after the `PATH` is checked does the test check that the model's copy was cut, so that a break that shortens the `PATH` fails on the `PATH` and not on how it was read. |

Nothing of the product changed, and no doc: the output policy and the environment are as `05-tools.md` describes them.

## The tests and the breaks

| Rule | Test | Break |
|---|---|---|
| `PATH` keeps every absolute directory it was given, however long it is | `cineris-core::it shell::sys_a_command_is_given_the_environment_the_command_analysis_relies_on` | `step-25-pathfix/path-loses-what-follows-its-first-entries` (keeps the first 32 entries) |
| … and its first | the same | `path-loses-its-first-entry` |
| … in the order it was given | the same | `path-put-in-order` |
| … with a directory it names twice | the same | `path-repeats-taken-out` |

The unit tests of `absolute_entries` give it four entries at most, with no repeat, so they cannot see the first break or the last; before this change the system test could see none of the four, since it checked only that what it read was absolute. The first, at the desk with the code broken by hand, fails on the assertion of the whole `PATH`. The eight breaks of step 13a that name this test (`path-keeps-relative-entries` among them) are unchanged and still apply.

## Elsewhere: what else assumes the repository's folder or path

Looked for: the old and the new name as a folder (either in a path), the runner's `D:\a\…`, `CARGO_MANIFEST_DIR` or `CARGO_TARGET_DIR` taken apart by name, and `env!("CARGO_…")` paths compared as text. Nothing found that depends on them:

- Every `CARGO_MANIFEST_DIR` is joined with a relative path (`../..`, `fixtures`, `snapshots`), never split or compared; `xtask`'s `workspace::root()` is its parent. `CARGO_TARGET_TMPDIR` is only a place for scratch folders, now 14 characters shorter on a runner, which can only take paths further from Windows' 260.
- No test, workflow or script names `D:\a\` or the repository's folder.
- The screen tests that wait for the product's name (`cineris-cli`'s system tests) wait for `Cineris <version>`, which no path holds; `cineris-tui`'s look for `Cineris` in screens of made-up sessions, with no real path on them.
- This test was the only one that read a command's environment, or any value, from a long line of the model's copy of an output.

## Runs

- **Before:** try run 44391435700 (Windows, `main` against a branch above it, five rounds each): the test failed in every round on both sides with the entry above; the message whole is in job 135109873515's log.
- **The fix, every test on Windows with `main` beside it on one runner** (try run 44396482438, at 9967057, the same change before one assertion was formatted, 6 ms a process start, five rounds): the test passed in every round on the branch and failed in every round on `main`, as before. The branch's other failure there, `xtask`'s `sys_the_repository_follows_its_own_rules`, was this record missing: the break list names it, and it was committed after that run. The tiers against `main`'s: +0.9 s, +0.8 s and +0.6 s, each within the spread of one side's own rounds (the unit tier ran 13.6 to 17.9 s on the branch and 14.3 to 16.5 s on `main`). The changed test is not in those figures: having failed on `main`, it is left out of both sides' medians, so nothing else changed and the differences are the runner's.
- **Every test on Linux and Windows** (try run 44400311409, at 71094a5): everything passed on both. The test took 0.037 s on Linux and 0.573 s on Windows, where a PowerShell 7 start was measured at about 0.44 s ([testing-and-ci.md](../../../testing-and-ci.md), "Test tiers"). That Windows runner was a slow one (8 ms a process start) and warned of the unit and integration tiers over their budgets (21.0 s and 44.6 s above the start, of 20 and 40), and the system tier at 89.9 s of 90; the A/B above, both trees on one runner, is the comparison, and moved nothing past its own noise.
- **At the desk, every run through `rx`** (a copy of the checkout, so that `xtask` looked at this tree: [xtask-is-rooted-where-it-was-built.md](../../../outstanding/xtask-is-rooted-where-it-was-built.md)): `cineris-core`'s tests and its dependents' (`test --dependents -p cineris-core`): 604 passed. The core's shell tests 200 times over (`stress -p cineris-core --count 200`): every one passed every time. `cargo xtask check`, and clippy for Windows through `cargo xwin`: nothing found.
- **The breaks** (`mutate step-25-pathfix`, at the desk, after the review): all four caught by the test they name, and by no other of `cineris-core`'s. Before the review the list had the first alone, and it was caught by the wrong check: with 32 entries the `Path` line was within a line's limit, no whole output was kept, and the test failed on that ("the output was not cut") before it looked at the `PATH`. Now, with the code broken by hand, the test fails on the assertion of the whole `PATH` ("does not end with the absolute entries of the one the harness was given"). The first run of the first break was not judged, the desk's disk having stalled under twelve of `cineris-tools`' tests at once; the breaks run `cineris-core`'s tests alone, where the test that catches them is.
- **The eight breaks of step 13a that name this test**, run again on the changed test at the desk: seven caught, by it among others. `provider-key-variables-left-in` was not caught, and not because of this change: since the reload (24673066) the providers' key variables reach the shell tool twice, in its configuration's `env_remove` and in `ShellLive`, which the tool adds to it (`ShellTool::removed`), so the break, which empties the first, leaves the second to remove them. That is read from the code, not run on `main`; the variables it would leave are short lines, the same in the model's copy the old test read as in the whole output. That list is step 13a's: what to do with it is the open item below.

## Left open

- [a-provider-key-break-no-longer-bites.md](../../../outstanding/a-provider-key-break-no-longer-bites.md): step 13a's `provider-key-variables-left-in`, and a break at the source of the key variables in its place.
- [an-empty-path-runs-the-workspaces-files.md](../../../outstanding/an-empty-path-runs-the-workspaces-files.md): found by the review of this fix. When no entry of the harness's `PATH` is absolute, a command is given an empty one, and a Unix shell then looks in the workspace.
- [xtask-is-rooted-where-it-was-built.md](../../../outstanding/xtask-is-rooted-where-it-was-built.md): found while making this fix.
