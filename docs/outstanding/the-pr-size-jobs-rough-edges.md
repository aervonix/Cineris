`ci-wait` counts `pr-size` with the blocking jobs.

- Area: tooling
- Owner: a tooling step

| Item | Who | When |
|---|---|---|
| **The pull-request size job's rough edges**, found by the independent review of step 33's size fix ([steps/33/answered.md](../steps/33/answered.md)). None lets a home path into a binary or delays a required check. **(1) The tools that read a whole `ci` run now see `pr-size` too:** the owner's `ci-wait --pr N` waits for it, about four minutes more, and reads a red `pr-size` (a binary over its budget) as a failed run; a filter by job name in `ci-wait` would settle it (`ci-times` marks those rows `(blocks nothing)` since the eighth tooling step). (2), a difference from a record of another commit than the base, said in the summary by `size --base`, and (3), a change to the job not run on its own pull request, were settled by the eighth tooling step. | A tooling step; (1) partly the owner, whose `ci-wait` is outside the repository | Before steps 24 to 30 run in parallel, for (1) |
