A commit a person settles to nothing during `cargo xtask rebase` is dropped by git and not named in its report.

- Area: tooling
- Owner: the maintainers

| Item | Who | When |
|---|---|---|
| **A commit settled to nothing by hand in `cargo xtask rebase`**, found while the third part of the twelfth tooling step was rebased ([steps/tooling-12/parts/c-ci-and-measurement.md](../steps/tooling-12/parts/c-ci-and-measurement.md)). When the command stops on a conflict it leaves to a person, and the person settles it so that the commit changes nothing, `rebase --continue` hands on to git, which drops the commit without a word, and the report does not name it. The command names a commit it settled to nothing itself (it skips it with `git rebase --skip`), but it cannot tell this case from a pick that git put back because a file it does not track was in the way: git keeps `REBASE_HEAD` in both. A way to tell them apart (the index against `HEAD` before `--continue`, say, or git's own record of the rebase's todo) would let the report name the dropped commit. | The maintainers | Before a wave's branches are rebased by hand again |
