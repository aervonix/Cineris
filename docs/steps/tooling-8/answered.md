# Answered in the eighth tooling step, on 2026-11-16

This step is not a numbered step of the build plan: it built the eighth round of developer tooling, from what wave 11's seven sessions asked for. Its records are here, in a folder named for the job.

## The flaky dialog test on `main`

`cortex-cli`'s `tui::sys_a_projects_first_limit_is_listed_and_in_force_once_trusted` failed on the desk's fast builds. The cause was the test's own: after its `y` it waited for the header with `t.shows("Cortex-Harness")`, which the trust question's path matched wherever the temporary folder lies inside a copy of the repository named after it (the desk's `rx` copy of the main checkout). The wait passed at once, the test typed `go` and Enter microseconds after `y`, and the application read all four in one read, which it takes as typing (`App::burst`): the `y` went to the input box and the dialog stayed. A key log in the application showed it in 5 runs of 5 with the wait removed. The dialog's grace and what the product shows did not change.

The fix (pull request 64): the header waits match the title and version; every dialog answer of the system tests goes through `cortex_testkit::pty::Session::answer_dialog`, which waits for the answers row, types the key alone, and waits for the row to go and for what only the answer brings; the two undo tests wait for the idle input box before `/undo`, which they used to send while the turn was still running (16 failed rounds in 30 under `--cores 2 --load 6`). After it, the 13 `tui::` tests passed 50 rounds plain and 50 under load at the desk, and 10 rounds on Windows (`try` 43844567358).

The review of the fix found that `cortex-agent` emits `turn.completed` before it clears `inbox.running`, so a `/undo` sent the moment the idle box shows can still be refused. The coordinator gave it to step 23's fix session.

## Open items this step settled

- **`a-mutate-jobs-test-that-once-finished-every-piece`:** the cause was in `jobs.rs`: a job took its next piece without waiting for the run's thread to write down the answer "time is up", and the test opened its gate from inside the question. Taking a piece now shares a lock with asking the time. 200 stress repeats pass, alone and under `--cores 2 --load 6`; two breaks of the lock are caught by the test.
- **Parts of `tests-the-seventh-tooling-step-left-thin`:** the test of a second hold now sees the second waiting before it ends the first (the registry counts the waiting holds), so a hold that never waits, or one never woken, fails it; the `caught-by` names of an `os:` break are looked for in the words of the source on the other system. The rest of the item stays.
- **Parts of `rough-edges-of-the-seventh-tooling-steps-commands`:** `ci-times --ab --write` writes an OS it left out as its note; `scratch-test` refuses a cargo profile or ignored tests asked for on both sides of `--`; a `corpus/PIN` that is not a pin is an error that says `corpus pin` mends it, and `pin --update` keeps `corpus/Cargo.lock` until the new one is made. The rest of the item stays.
- **Parts of `the-pr-size-jobs-rough-edges`:** `ci-times` marks the jobs that block nothing (a test fails when `ci.yml` has a job of neither kind), and a change to `ci.yml`, `main.yml` or the `changes` action runs the size job on its own pull request. The rest stays.
