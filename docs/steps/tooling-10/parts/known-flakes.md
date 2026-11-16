# The tenth tooling step, its second pull request: the known flakes

Item 2: a list of known flaky tests that the tools read. One sentence a line.

## As built

- **The list is the open items' own:** an item names each flaky test it is about, `` - Flaky: `<binary> <test>` ``, as nextest's report gives them (`xtask/src/flaky.rs`, read by `outstanding`'s parser).
  Settling the item, or taking the line out, takes the test off the list, so it cannot drift from the items.
  An item that cannot be read names no flake, and a warning says which: a list that is short runs fewer tests again, never more.
  `flaky::tests::every_known_flake_names_a_test_its_crate_has` holds each mark to a function of its crate, so a renamed test is found on the pull request that renames it.
- **Marked:**
  - `cortex-core::it trust::sys_a_project_trusted_in_the_store_is_trusted_while_its_rules_are_the_ones_the_user_saw` ([a-base-capture-beside-the-turns-git](../../../outstanding/a-base-capture-beside-the-turns-git.md));
  - `cortex-tools::it changes::sys_every_git_the_look_starts_goes_through_its_confinement`, GitTooSlow ([a-git-look-too-slow-on-a-loaded-windows-runner](../../../outstanding/a-git-look-too-slow-on-a-loaded-windows-runner.md));
  - `cortex-index::it outlines::outlining_a_name_of_a_megabyte_takes_little_memory` ([a-memory-peak-test-near-its-limit](../../../outstanding/a-memory-peak-test-near-its-limit.md)).
  The leader-close test is not marked: step 23's third pull request fixed its cause and settled its item.
- **`stress`, `ci` away from CI and `mutate`'s baseline** name a listed test that failed, with its item, run it again once by its binary and its name exactly, and go on when that run passes.
  A listed test that fails its second run, a second run whose command failed or whose report lacks it, and every test that is not listed still fail.
  A test that is not listed is never run again.
- **`ci`:** after each of its two test runs, and after the stall's re-run, the listed tests among what still failed are run again in one run.
  The report keeps the second run's cases (`junit::replace_cases`), so the tier check reads their real times, and the job's summary page has a table of each flake, its item and its second run.
- **CI's blocking jobs run nothing again.** Under GitHub Actions (`GITHUB_ACTIONS=true`) `ci` names the listed tests that failed, on the log and the summary page, and fails as it did.
  That is the only change to what CI does: a line of output and a table, and no change to what passes.
- **`stress`** runs a listed test that failed on some repeat once more, under the seed of the first repeat it failed on, with the run's other arguments and inside its `--cores`/`--load` squeeze, and puts the stress report back afterwards.
  It says how many known flakes passed once more and are not counted as failures of the run.
  **`stress --strict`** runs nothing again, for a session that stresses a known flake to show its fix.
- **`mutate`'s run with nothing broken:** when every failing test is a listed one, they are run again once (the runner's new `test_only`, nextest with an exact filter), and the run goes on if they pass; it says `N tests pass, 1 of them a known flake on its second run`.
  Beside any other failure nothing is run again, and the run stops as before.
  A break's own run is not changed: a known flake that fails there is judged as any test is (left open, below).
- **Documented** in testing-and-ci.md ("Known flakes", and the notes in "Running the tests many times over" and "Breaking the code on purpose"), CONTRIBUTING.md's rows for `stress` and `ci`, a row of AGENTS.md's "Working here", and the preamble of `docs/outstanding/README.md`.

## Runs that show it

At the desk, with a throwaway test that fails the first time it runs under a tag and passes after: one listed as a known flake by a throwaway item, one not. Neither was committed.

- **`stress -p cortex-base --count 3`, both tests:** the listed flake was named, run once more under seed 0, and passed; the unlisted one failed the run: `xtask: 1 test(s) failed on some repeat, besides 1 known flake(s) that passed when run once more`, exit 1.
- **`stress`, the listed one alone:** `1 of 1 known flake(s) passed when run once more, and are not counted as failures of this run`, exit 0. With `--strict`: `--strict: not run again, so each counts as a failure of this run`, exit 1.
- **`ci test`, both tests:** the listed flake ran again and passed, `The step fails: 1 other test(s) failed, which are not run again`, exit 1.
- **`ci test`, the listed one alone:** `The 1 known flake(s) passed when run again, and nothing else failed: the step passes`, exit 0.
- **`ci test` with `GITHUB_ACTIONS=true`:** `Not run again: CI's blocking jobs run nothing again. 1 known flake(s) failed.`, and the step failed, exit 1.
- **`mutate` of one break of `cortex-base`:** `Running the 1 known flake(s) again, once.`, `passed when run again`, `732 tests pass, 1 of them a known flake on its second run`, and the break was then judged (caught by 2 tests).

## Tests and breaks

- `flaky::tests`: a listed flake that passes its second run; one that fails again; a failure that is not listed, alone (nothing is run or said) and beside a flake (the flake alone is run, the run still fails); CI (named, nothing run, the report as it was); a second run with no report (the first put back), without the flake, or whose command failed; matching by binary and name exactly; every mark names a test that is there.
- `stress::tests`: which failing tests are run once more and under which seed (a retry counts; a test not listed, or listed and passing, does not); the command of the run once more; what a known flake that passed excuses, and what it does not; `--strict`.
- `mutate::drive::tests`: a pretend runner's baseline with a flake that passes alone, fails again, fails beside another failure, or is not listed.
- **Not reached by a test:** the line of `checks.rs` that passes `!in_ci()` (a break of it fails no test; the CI run above shows it), `stress`'s start of cargo for the run once more, and the real runner's `test_only` (both shown by the runs above).
- Breaks: the `flaky-*` breaks of `mutations/tooling-10.breaks`. `tooling-5/stress-reports-a-pass-for-a-failed-run` was moved to the code as it is now.

## Left open

- A break's own run in `mutate`: a known flake that fails there makes a break look caught when its list names no test. A break that names its tests (`caught-by`) is not fooled. Filed: [a-known-flake-in-a-breaks-own-run](../../../outstanding/a-known-flake-in-a-breaks-own-run.md).
