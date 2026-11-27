A call or a start of an extension already running when the count of threads left running reaches four can still leave one more, since a native loop of the script engine cannot be stopped.

- Area: cineris-ext
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **Threads left running past the cap by calls already running.** A native function of QuickJS that loops without looking at the interrupt (`Array.prototype.reverse` over an object whose length is huge) cannot be stopped, and its thread is left running until the process ends. Since the review of step 28's close a thread counts toward the cap of four from when it is told to stop, a start or a call is let in only below the cap, and one of an extension waits for its own extension's thread being stopped and is refused when it is left ([architecture/12-extensions.md](../architecture/12-extensions.md), "Limits"). That leaves the calls and starts already running when the count reaches four, which nothing stops: five sessions of one process (a server's, a Coordinator's employees) whose calls of one extension spin together leave five threads, each at full speed and holding up to its runtime's memory. **Recommendation:** make the loops stoppable (a QuickJS patched to look at the interrupt in its array and typed-array loops, taken through `rquickjs`), or run the runtimes in a process of the harness's own that can be killed; until then the bound is the one the page states. | Step 32 (hardening pass) | Before extensions run in a server of many sessions |
