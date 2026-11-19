# The twelfth tooling step, part C: CI and measurement

Items 9 to 12 of the wave 14 retrospectives' asks, and the maintainers' extra ask of a warning for a break list's name. One sentence a line.

## What was built

### Item 9: `try --final`

- `cargo xtask try --final [--against REF] [--count N] [--commit SHA] [--dry-run | --dispatch BRANCH]` is one dispatch, `command=final`, of three jobs: `try-windows` runs the Windows A/B against `main` (`ab --base ../ab-base --label main --top 20 --keep-going`), `try-linux-22.04` the tests on the kernel with Landlock ABI 4 (`ci --tiers --top 20`), and `try-linux` the size check (`size --check --against-main --target x86_64-unknown-linux-musl`).
- `try.yml` chooses that matrix when `command` is `final`, whatever `os` says, and gives each job its part (`ab`, `tests`, `size`) as `--part`; `try` makes the part's command, so the rules stay in `xtask` under unit tests.
- Only the A/B's job checks the base out, only the size job installs the musl target and is given the token that reads `main`'s size record, and the job's limit is 60 minutes.
- A test reads `try.yml` and holds the matrix's names, runners and parts to `try`'s list (`FINAL_PARTS`), and that both runners pass `--part` on.
- `try` refuses a part that is not its runner's system, and a final run with `--os`, `--profile`, `--filter`, `--load`, `--cores`, `--capture no`, `--testkit` or words after `final`.
- The jobs are named as any `try` job is, so `try-result RUN` reads all three as it reads any run.
- `try-result` of a run of several jobs now ends with each job's conclusion and the command it ran, from its log, together.
- A job that ran no tests and passed (the size check) now shows its log's last lines, where it showed nothing.
- **A size of a commit is recorded under that commit** (the maintainers' report from the size work): `try` with `commit` and `size` wrote the dispatched ref's head in the record's `commit`, since `size` writes `GITHUB_SHA`, which on a runner is the ref's head, and a workflow cannot set GitHub's own variables. `try` now runs `size` as a child of its own with `GITHUB_SHA` set to the commit; `size`'s own code is unchanged.
- The YAML pitfall (a plain `run:` value with ` #` cut at the comment) does not arise: every new script line is in a `|` block, and the matrix's JSON is in an expression.

### Item 10: a whole job two ways, `ab --job`

- `cargo xtask ab --job --base DIR [--count N] [--minutes M] [--setup-base FILE] [--setup-head FILE] [--keep-build] [-p CRATE]… [-- NEXTEST ARGS]`, in `xtask/src/job_ab.rs`.
- **Why `ab --job` and not `try --command job-ab`:** it is the same two trees, alternating order, label and base checkout as `ab`, and the same artifact `try-result` reads; it runs locally as well as on a runner, where a `try`-only command could be tried nowhere but on a runner. On a runner it is `-f command="test --job …" -f against=REF`, as `test` with `against` is `ab`.
- Each side's round: its `target/ci` and `target/tmp` removed (always in the first round, so that both sides start cold; in later rounds unless `--keep-build`), its setup script run, its tests built, the job's runs made (tiers 1 and 2, then tier 3), each timed, and each tier's tests' time summed above the start from the report; `the job` is the setup, the build and the runs.
- Each way has its own setup: a script in the head's tree, run in the side's tree by `bash` (Git's own on Windows, found under `ProgramFiles`, since a plain `bash` there may start WSL; `pwsh` for a `.ps1`), which may write `KEY=VALUE` lines to `$CINERIS_AB_ENV` for that side's build and tests.
- **Robust by construction:** a failed clean, setup or build ends that side's round, and a failed run of the tests does not (the next run is made, and the tiers' sums and `the job` count as failed); either way the round is recorded with its times and message, and the other side and the next rounds run; nothing in the loop returns early; the record is written to `target/ab/job-ab.json` after every side, and each side's report to `target/ab/<side>-job-<round>.xml`.
- The rounds are even in number (`--count`, 2 to 20, 2 when not given), so that each side goes first as often as the other.
- `--minutes M` starts no round that the longest round so far would carry past `M`; `try` gives `--minutes 200`, within the run step's own limit of 230 minutes and the job's of 240.
- It prints a line for each side's round, then each phase's median on each side over the rounds it passed in, with the count (`110.0 (3/4)`), and its change paired by round (the median of head minus base over the rounds it passed in on both sides, how many, and in how many the base went first, marked `*` when not half), and fails at the end when a side failed.
- `try-result` prints the same from the kept record, with each failed test from the kept reports. It leaves out the tiers of the head's last round's report, which is where a test run leaves its own and is not the run's tiers.

### Item 11: `mutate`

Built in `xtask/src/mutate/readers.rs` and `xtask/src/mutate/budget.rs`, with changes to `changed.rs`, `mod.rs` and `drive.rs`.

- **Who reads a data file.** A crate reads a file outside Rust when a string literal in its Rust names the file, or a folder that holds it, read three ways: from the source file's folder (as `include_str!` and `include_bytes!` read it), from the crate's folder (a build script's `rerun-if-changed=PATH`, a path joined to `CARGO_MANIFEST_DIR`), and from the top of the repository.
- Not counted, as too common to mean this repository's file: one name from the top (the product crates name `"AGENTS.md"` for a project's own file), a bare file name, and a literal that climbs to its own folder or above. A literal of two names or more that ends the file's path counts from anywhere.
- A reader the diff removed still counts: each touched Rust file is read as it was at the reference too.
- A reader under a crate's `tests/`, `benches/` or `examples/`, or inside a `#[cfg(test)]` module, counts for that crate alone; any other counts for every crate built on it or testing with it, as `test --dependents` counts them.
- Those crates' breaks are chosen, with the reason "a data file a crate they run reads", and the summary names each file's readers and the crates that can see it.
- A crate's own `Cargo.toml` or `build.rs` now chooses the breaks of the crates built on it too, where it chose only its own.
- **Every break is still chosen,** with the file and the reason printed, for a file outside `crates/ xtask/ docs/ mutations/ .github/` that no crate names (Markdown aside), Rust outside the crates, and a workspace that cannot be read.
- **What it cannot see:** a path built at run time from pieces that do not name the file by two names (`root.join("corpus").join("PIN")`, so `corpus/PIN` still chooses every break, and the test kit's loader of fixtures is no reader of them); a path from a variable or a setting; a file read by a program the tests start; and, as before, a Rust helper a test calls and a break that names no test.
- **What it chooses for `catalog/models.toml` alone: 3,173 of 3,220 breaks,** where it chose all 3,220. `cineris-llm`'s build script compiles the catalog in and `cineris-stats` names it; 17 crates are built on those or test with `cineris-testkit`, which is built on `cineris-llm`; nothing in the source says which of their tests read model data. Only the 47 breaks of crates that cannot see the catalog are left. The rule pays where readers are few: a fixture of `cineris-llm` and `docs/testing-and-ci.md` together chose 1,062 of 3,208.
- **`--budget TIME`** (`90s`, `30m`, `1h30m`) runs the breaks in four classes, each in list order: (1) the diff touched the break's file, or the break itself; (2) a test its `caught-by` names was touched; (3) a file of a crate it runs was touched, or a data file such a crate reads; (4) the rest, and every break without `--changed`.
- Each class runs its tests with nothing broken, then its breaks. No class is begun once the budget is spent; within one, no new piece starts, and pieces under way finish and are kept.
- The up-front compile of every break is skipped under a budget, so that the time goes on breaks reached; a break that does not compile is found when it runs.
- It ends with each class's reached and total, names each break left, and gives the command to carry on (and one naming the breaks left, when 40 or fewer are). A run stopped by its budget exits 2. `--list --budget` prints the order. It is refused beside `--minutes N` above 0, `--check` and `--merge`; `--minutes 0` may stand beside it, as `try` adds it.
- **A real budgeted run:** `--changed 6806291f --budget 4m --jobs 2` reached 92 of 1,836 chosen breaks on the reference machine: classes 1 and 2 whole (80 and 12, all caught), class 3 begun and stopped (0 of 1,744), exit 2.
- `try`'s `mutate` now passes `--budget TIME` on. A runner's checkout has no history, so `--changed` stays refused there.
- Four `--changed` breaks of `mutations/tooling-8.breaks` pointed at code this restructured: their `find` and `in` were moved to the code as it is now; the rules they break still hold.

### Item 12, as there was room

- **(a) `cargo xtask clippy -p CRATE… [-- ARGS]`: done.** Why plain `cargo clippy -p cineris-cli` fails on `main`: `cineris-tui` reads its `clock` field only under its `test-kit` feature, which its own dev-dependency on itself turns on in a build of the whole workspace and nothing turns on in a build of the CLI alone, so the field is "never read" and `-D warnings` fails.
- It works the workspace's features out from the manifests (each member's `default`, every feature a member's dependency names on another member, and what those turn on, to a fixed point) and passes them as `--features CRATE/FEATURE`.
- Cargo takes `CRATE/FEATURE` only for a crate named and its direct dependencies ("the package does not contain these features" for the others, found by running it), so those are passed and the others are named in a note.
- On the reference machine `cargo xtask clippy -p cineris-cli` passes where `cargo clippy -p cineris-cli --all-targets --locked -- -D warnings` fails.
- `apply`'s `@@ check: clippy -p CRATE…` uses it when its words are crates alone.
- **The same dead field is in the shipped build:** the final run's size job, a `dist` build of `cineris`, warns `field clock is never read` in `cineris-tui`. It is product code, outside this step: reported to the maintainers, not changed.
- **(b) a retry of the install step on a "bash startup failure": left.** No such failure is recorded in `docs/` or `docs/outstanding/` to build and test it against, and it changes the blocking jobs of `ci.yml`, which would need a measurement of their times before and after; left for a step that has a failing run to test it with.
- **(c) `pr-size` printing the by-crate difference: left.** Another session is changing `size` and `pr-size` now, and the maintainers asked that nothing there change until they are told.

### The maintainers' extra ask: a break list's name

- **Done.** `mutate --check`, a run of the lists, and the `breaks` step of `cargo xtask check` warn of a list in `mutations/` whose path or name `crate::rename::violations` finds an old name in, the name read as text so that the old alias and the labels built on it count too; no copy of the rename's words.
- Tested with names made of pieces (`mutate::tests::a_lists_name_the_rename_would_rewrite_is_warned_of`), so that the test's own file holds no old name.

## What was measured

| What | Run | Result |
|---|---|---|
| `try --final`, first | 44591011876, at `ca4ed85` | `try-linux-22.04` and `try-linux` passed; `try-windows` failed on one test of this part's own, `job_ab::tests::a_request_is_read_and_what_makes_no_sense_is_refused`: `/abs.sh` has no drive, so Windows does not call it absolute. Fixed (any root, drive or `..` is refused), and the combined result named it in one step |
| A whole job two ways, Windows, three rounds | 44591389659, on a scratch branch at `93ab846` | All three rounds ran, 26 minutes in all; the base's setup failed round 2 on purpose and was recorded, and the run went on. The head's setup built without debug info (`CARGO_PROFILE_CI_DEBUG=false`): its build's median was 187.5 s (3 of 3) against the base's 213.6 s (2 of 3), −26.1 s as first printed. **That figure was confounded** (the review's first finding): the base's two surviving builds both ran first, and paying for the cold disk cache is the first side's. Paired by round, as the tool now prints it, the build is −20.0 s over 2 rounds, both with the base first (−20.5 s in round 1, −19.4 s in round 3), and tier 3's run −1.6 s over the same 2; so the debug-info question cannot be told apart from the order in this run. The head's tiers 1 and 2 failed every round on the test above, so its later phases have no median |
| `size` of a commit | 44609673171: `-f os=linux -f command=size -f commit=24e322c96fdd35c9df0aa00bfbae038b13b8efd3`, dispatched on the branch at `a5f334de` | Passed. The size record's `commit` is `24e322c96fdd35c9df0aa00bfbae038b13b8efd3`, the commit asked for, not the branch's head `a5f334de052b59e6d845d83b24ca979ca51f0db2` that `GITHUB_SHA` names. The commit is this part's own fix: the commit's `xtask` runs the command, so an older commit of `main` would run its own `try`, without the fix, and show nothing of it |
| A whole job two ways, Windows, after the review | 44612441508, on a scratch branch at `41977c5`, `--count 2 --keep-build`, setups on both sides that change nothing | 14 minutes. Round 1 built both sides from cold (196.7 s base, 209.1 s head), whatever the cache had restored into the head's tree; round 2 kept both builds (0.8 s each). Each side went first once, so each phase's paired change is over pairs half base-first: the build +6.2 s and tiers 1 and 2's run +2.8 s over 2 pairs, 1 base first, both sides being the same job; the clean, made in round 1 alone, is one pair and marked `*`. The head's tier 3 failed in both rounds on `rules::tests::sys_the_repository_follows_its_own_rules`, which refuses this record until the step's `answered.md` links it, so the head's later phases have no median |
| `try --final`, second | 44593957018, at `395a4cd` | All three jobs passed; the Windows A/B 22 minutes, Linux 22.04 and the size 3 minutes each |

The second final run, as `try-result` printed it:

- **Windows A/B, five rounds on one runner at 8.0 ms a process start:** tier 1 17.9 s against `main`'s 17.9 s (−0.1 s), tier 2 33.4 s against 33.7 s (−0.3 s), tier 3 84.5 s against 84.6 s (−0.1 s). Each side's wall time's median 94.2 s (head) and 94.3 s (base).
- **Linux 22.04:** tier 1 27.3 s above the start, 137% of its budget, a warning: the job builds without the cache on that image, and its unit tier's largest crate was `cineris-tui` at 14.8 s; no test of this part's is among the slowest. Tiers 2 and 3 81% and 56%.
- **The size:** 15,006,120 bytes, +0 against `main`'s run 44592310651.
- The run's three jobs together, as the new summary prints them:

```text
try-linux-22.04  success  cargo xtask ci --tiers --top 20
try-windows      success  cargo xtask ab --base ../ab-base --label main --top 20 --keep-going
try-linux        success  cargo xtask size --check --against-main --target x86_64-unknown-linux-musl
```

- The scratch branch of the job A/B (`tooling-12-c-try`), which held the two trial setups, is deleted.

## The rebase onto main

After the first and second parts were merged, this branch was rebased onto `main` at `d6f07ee9` with the second part's `cargo xtask rebase --no-regen`, its `xtask` built from `main`'s tree, the generators run after it on their own. Its output, but for its first line, which names the folders it ran from and was built in:

```text
Stopped at 04284ea7 Run the checks before a pull request as one try run, and time a whole job two ways:
  united    xtask/src/main.rs: both sides' additions at 1 place(s); every line of both kept, and checked
Rebased tooling-12-c onto origin/main: every conflict settled.
docs/outstanding/README.md is up to date.
The tip before was 55e08b54f07a: `git reset --keep 55e08b54f07a` takes the branch back to it.
```

The one conflict was `xtask/src/main.rs`'s usage, both sides' lines added at one place; the command settled it and checked it. `docs/testing-and-ci.md` and `AGENTS.md` merged without a conflict. `cargo xtask outstanding` wrote the index again after the open item below was added.

## The pre-push review

An independent review of the branch before its pull request found ten things. Findings 1 and 2 are the step's own bar: a measurement that compares unlike things.

| # | Finding | Verdict | Fix, and its test or break |
|---|---|---|---|
| 1 | The job A/B's order favours a side (3 rounds put the base first twice), and each side's median drops its own failed rounds, so its change compared unlike rounds | Right: run 44591389659's −26.1 s compared builds the base always began | The rounds are even (`--count` 2 to 20, 2 by default); each phase's change is paired by round over the rounds both sides passed, with how many and how many the base began, marked `*` when not half. Tests `a_change_is_paired_by_round_and_says_which_side_went_first`, `each_phase_takes_its_median_over_the_rounds_it_passed_in`, `a_request_is_read_and_what_makes_no_sense_is_refused`; breaks `review-job-count-may-be-odd`, `review-job-default-count-odd`, `review-job-change-not-paired`, `review-job-base-first-not-counted`, `review-job-uneven-order-not-marked`. Run 44591389659 read again above |
| 2 | With `--keep-build` on a runner, only the head starts warm: the cache restores `main`'s build into the head's tree | Right | Both sides' builds are removed in the first round whatever `--keep-build` says (`cleans`); the docs' "only the first round builds from cold" corrected. Test `both_sides_start_cold_and_keep_build_keeps_only_later_rounds`; break `review-job-keep-build-starts-warm`; shown on the runner below |
| 3 | A `mutate --budget` past the job's limit, or a job A/B past its own, loses everything: the job is cancelled and the upload skipped | Right | Each run step has its own limit ten minutes under its job's (30, 50, 80, 230), so an overrun fails the step and the upload runs; `try` refuses a `mutate --budget` over 60 minutes; a job A/B's `--minutes 200` is held under its step's 230 at compile time. Tests in `the_workflows_final_matrix_is_the_final_runs_parts` and `mutate_runs_the_breaks_to_the_end_with_the_options_it_may_have`; break `review-try-budget-past-the-step` |
| 4 | `--changed` skips Rust used as data (`cineris-index`'s `fixtures/outlines/sample.rs`, `xtask`'s `fixtures/rules/*.rs`) | Right | A `.rs` file under a crate outside `src/`, `tests/`, `benches/`, `examples/` and not its `build.rs` is read as data (`rust_as_data`). Test `rust_a_crate_holds_as_data_is_read_as_data`; break `review-rust-fixture-not-data` |
| 5 | A literal with a format hole is never a reader (`format!("{}/../../docs/…", env!("CARGO_MANIFEST_DIR"))`) | Right | The text after the last hole is read with the path rule (`format_tail`). Test `a_format_strings_path_is_read_after_its_last_hole`; break `review-format-hole-no-path` |
| 6 | The list-name warning came only from `check`'s `breaks` step, not from `mutate` | Right; the record's claim was wrong | `mutate --check` and a run of the lists print it too; the claim corrected. Test `the_lists_on_disk_are_the_ones_warned_of` (the lists read from disk). The call in `mutate`'s run is not under a test: `mutate`'s run reads the real repository, and no test starts it |
| 7 | After tiers 1 and 2 fail, tier 3 still runs and counts, where the docs, `render` and the record said a failed phase ends the round | Right: the code is the job's way, the text was wrong | The docs, `render`'s text and the record now say a failed clean, setup or build ends the round and a failed run of the tests does not. Tests `a_failed_run_of_the_tests_does_not_end_the_round` and the tier-3 assertion in `each_phase_takes_its_median_over_the_rounds_it_passed_in`; break `review-job-failed-run-ends-the-round` |
| 8 | A final run is green when `size` could not read `main`'s size | Right | `try-result` names a `size --against-main` job whose log says `main not read` as a problem, and the run fails; `size`'s own code is unchanged. It holds for any `--against-main` run, not only a final one: the comparison asked for did not happen either way. Test `a_size_against_main_that_read_no_main_is_a_problem_of_the_run`; break `review-unread-main-passes` |
| 9 | Tests that miss a broken implementation: the job A/B's failed round going on, the budget's wiring in `mutate::run`, the workflow's base checkout, token and limits | Right | The side's round and the rounds are drivers over a trait of phases and a clock, tested with fake phases (`a_side_times_each_phase_and_a_failed_build_ends_its_round`, `a_failed_round_is_recorded_and_the_rounds_go_on_in_turn`, `the_rounds_stop_before_one_that_would_pass_the_minutes`; breaks `review-job-failed-build-goes-on`, `review-job-rounds-stop-at-a-failure`, `review-job-record-not-kept-each-side`); the budget's order and its compile rule are functions of their own (`a_budget_orders_the_breaks_and_counts_only_those_run_here`); the workflow test holds the base's checkout, the token and both limits. What `mutate::run` does with the drivers around them is still not under a test |
| 10 | A dispatch by hand of `command=final` with no `against` fails confusingly; the budget's order counted the other system's breaks (1,862 shown, 1,836 run) | Right | A final run's A/B job given no `against` is refused with how to give it, before anything is built; the order counts only the breaks run here. Tests in `a_final_runs_job_runs_the_part_it_is_given_and_nothing_else` and `a_budget_orders_the_breaks_and_counts_only_those_run_here`; breaks `review-final-job-defaults-against`, `review-budget-counts-the-other-system` |

## The breaks

`mutations/tooling-12-c.breaks`, 47 breaks: the 16 of the pre-push review's fixes (in its section above), 19 of `try --final`, `ab --job`, `try-result`, `clippy -p` and the list-name warning, and 12 of the data-file rule and `--budget`, put in from the `mutate` work's own list with `cargo xtask mutate --merge tooling-12-c mutate`. Run on the reference machine with `cargo xtask mutate tooling-12-c --jobs 2`: before the review, **31 caught, 0 flagged**; after the review's fixes and the rebase onto `main` at `d6f07ee9`, **47 caught, 0 flagged**.

- Each rule above has one break: the parts of a final run (the tests part made an A/B, the A/B's count lost, `main` not the default, `--os` taken, a part run on the wrong system, a final run with no part); a job A/B's options without `--job`, its `--minutes` from `try`, the round that starts past the budget, a failed phase in its median, a failed side that passes, a comment in the setup's file; `try-result`'s log tail and command; the workspace's features (members' defaults, features followed, dev-dependencies, what cargo refuses); the list-name warning; the data-file readers (from the crate's folder, a climb, a bare name, a reader in tests alone), the crates chosen (built on the reader, no reader, a reader the diff removed, a manifest), and the budget (a named test's class, the order, no class begun once spent, what was left).
- Four `--changed` breaks of `mutations/tooling-8.breaks` were moved to the code as it is now and caught when the `mutate` work ran them.
- The break of the workflow's matrix is not in the list: the matrix is held by a test that reads `try.yml`, and breaking that test would break the check, not the code.

## The cost to the blocking jobs

None: `ci.yml` is not changed. The new tests are pure functions in `xtask`'s unit tier, under 0.1 s together.
- `cargo xtask ci` on the reference machine passed every step but the tier budgets after the review: tier 1 at 152 to 157%, `xtask`'s unit tests at 10 s of it. `ab -p xtask` there put this branch at +10.7 s against `main`, each test about 13 ms slower. It is not the code: the two test binaries, copied to one folder and run 280 times each, 28 at a time, took 29 to 38 ms against 29 to 31 ms; from the reference machine's copy of this branch's build directory the same run took about 150 ms against `main`'s 30. One test alone, its page faults and its system calls are the same on both binaries. After the rebase onto `main` at `d6f07ee9`, `cargo xtask ci` there passed every step, tier 1 at 150% (a warning), `xtask`'s unit tests at 16.0 s of it in the same copy.
- The Windows A/B on one runner (run 44593957018) puts this part at −0.1 s, −0.3 s and −0.1 s in tiers 1, 2 and 3 against `main`: no cost.
- Linux 22.04's tier 1 was 66% of its budget in the first final run and 137% in the second, on the same tests: that runner's speed, which the A/B on one runner takes out.

## Left open

- [A commit settled to nothing by hand in `cargo xtask rebase`](../../../outstanding/a-commit-settled-to-nothing-by-hand-in-a-rebase.md) is dropped by git and not named in its report: an open item of its own, for the maintainers.
- `cineris-tui`'s `clock` field, dead in a build without `test-kit` (above), is a product fix for another step.
- `ab --job` cleans `target/ci` and `target/tmp`; a build in another profile or for another target triple would need its folder named too.
