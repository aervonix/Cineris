A known flake that fails in a break's own run of `mutate` makes a break that names no test look caught.

- Area: tooling
- Owner: a later tooling step

| Item | Who | When |
|---|---|---|
| **A known flake in a break's own run.** The tenth tooling step's known-flake list is read by `mutate`'s run of the tests with nothing broken, which runs a listed test again once when it fails, and not by the run of each break. There a listed test that fails by chance is a failing test like any other: a break whose list names its tests (`caught-by`) is not fooled, since the tests it names must fail; a break with no `caught-by` is "caught" by any failing test, so a flake can make it look caught. Recommendation: when every test that failed with a break made is a known flake that its `caught-by` does not name, run them again once, as the baseline does, and judge the break on what is left; or say so in the verdict. ([steps/tooling-10/parts/known-flakes.md](../steps/tooling-10/parts/known-flakes.md)) | A later tooling step | When `mutate`'s judging is next changed |
