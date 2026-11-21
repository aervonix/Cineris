A test of the cap on extensions' threads left running can see its script interrupted where it should be left running, when a slow runner has not yet brought the script to the loop that never looks at the interrupt; one CI flake.

- Area: cineris-ext
- Owner: step 28's script hooks (its fifth part)
- Flaky: `cineris-ext::it engine::past_the_cap_of_threads_left_running_no_extension_starts`

| Item | Who | When |
|---|---|---|
| **What was seen.** In `try` run 44917713850 (a hosted Windows runner at 8 ms a process start, both trees run five times each), `engine::past_the_cap_of_threads_left_running_no_extension_starts` failed once, on one side, in the last round: a spinning extension's start gave `Err(Engine("Error: interrupted"))` where the test holds it to `Err(Abandoned)` (`crates/cineris-ext/tests/it/engine.rs`, line 918). **Why:** each spinning script runs `Array.prototype.reverse` over an object of length 2<sup>53</sup> − 1, a native loop that never looks at the engine's interrupt, so its thread is left running and counted; the test moves the fake clock past `EVALUATION_TIME` as soon as the runtime sleeps, and on a slow runner the script had not reached that loop yet, so the interrupt stopped it at an ordinary check instead. The engine is right in both cases; the test assumed an order it does not wait for. **Recommendation:** have the script tell the test it is about to enter the loop (a host call just before it, which the test awaits) and only then move the clock, so that the case it holds is the one it means. | Step 28's script hooks | With that part |
