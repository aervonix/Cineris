`/rewind` and fork are tested through the application's scripted link and the core's own tests, not by the real binary under a pseudo-terminal, in process or under `cineris attach`.

- Area: cineris-cli
- Owner: step 25

| Item | Who | When |
|---|---|---|
| **Rewind and fork have no test of the real binary.** Step 25's second pull request built `/rewind`, its question and fork in the interface ([steps/25/parts/pr2-rewind.md](../steps/25/parts/pr2-rewind.md)), every step an operation through the `Link`, and tested them through the scripted link (`cineris-tui`'s `app::tests::rewind`), the plan and the mark through the real core (`cineris-core`'s `rewind_plan` tests) and the read through the lane (`cineris-rpc`). The build plan's step 27 notes ask a new behaviour of the interface for a pseudo-terminal test under `cineris attach` as well as in process; none was written, because the system tier's cost was not measured for it and tier 2 is at its budget on Windows. Recommendation: one `sys_` test in `cineris-cli`'s system tier that runs two turns of the scripted provider, rewinds to before the second with `2`, and checks the prompt back in the editor and the next request's history, run once in process and once under `attach`, measured with `ci --tiers --like-runner` before it is kept in the blocking tier. | Step 25, its third pull request, or the step after it that touches `attach` | Before the first release |
