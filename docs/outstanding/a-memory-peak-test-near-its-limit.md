`outlines::outlining_a_name_of_a_megabyte_takes_little_memory` went 84 KiB over its 4 MiB peak once on a hosted Linux runner, in a pull request that did not touch `cortex-index`.

- Area: cortex-index
- Owner: `cortex-index`'s next change, or a tooling step
- Flaky: `cortex-index::it outlines::outlining_a_name_of_a_megabyte_takes_little_memory`

| Item | Who | When |
|---|---|---|
| **A test of an outline's memory went just over its limit once on a hosted runner.** `cortex-index`'s `outlines::outlining_a_name_of_a_megabyte_takes_little_memory` checks that outlining 1 MiB raises the process's peak resident memory (`VmHWM`, reset first through `/proc/self/clear_refs`) by at most 4,096 KiB. On pull request 78's first `test-linux` (run 43904949399, job 133431172865) it measured 4,180 KiB, and passed when the job ran again. The pull request changed nothing in `cortex-index` or what its tests build. At the desk the four `takes_little_memory` tests passed 100 times in 100 on that branch, and step 18 measured at most about 2 MB more on these inputs. So something on the runner sometimes adds about 2 MB to the peak in the test's window: a page the allocator or the test harness touches then, not the outline. Recommendation: have the test say what it measured when it passes (`cortex_testkit::numbers::report`), run it many times on a hosted runner (`try` with `stress -p cortex-index --count 200` and a filter on it), and then either find what the runner adds or measure the outline's own allocations (a counting allocator in the test binary) in place of the process's peak. Do not loosen the limit unless the cause is shown to be the runner's. | The owner of `cortex-index`'s next change, or a tooling step | Before the first release |
