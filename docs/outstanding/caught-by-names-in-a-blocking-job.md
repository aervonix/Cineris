The nightly checks each `caught-by` against the tests that exist; whether a blocking job should too, at minutes more, is the owner's choice.

- Area: tooling
- Owner: the owner

| Item | Who | When |
|---|---|---|
| **`caught-by` names checked in a blocking job, or only every night.** Since the tenth tooling step the nightly workflow's `break-names` job runs `cargo xtask mutate --check --tests`, which builds the tests of every set of crates the breaks run (42 sets) and checks each `caught-by` against them, so a name that a renamed or removed test left behind is found within a day and opens the nightly's issue ([steps/tooling-10/parts/smaller-items.md](../steps/tooling-10/parts/smaller-items.md)). The blocking `breaks` step of `check` builds no test and checks names only against a kept list, which CI has none of, so such a name can still reach `main` and break the next session's `mutate` of that break until the nightly says so. A blocking test job, which has built most tests already, could run the check after its run, at the cost of the other sets' builds (minutes, most of them a rebuild with other features): against rule 2's five minutes. Recommendation: the nightly is enough, since a stale name fails no product test and is found within a day. | The owner | When a stale name next costs a session time |
