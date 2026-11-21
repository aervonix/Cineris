A terminal test of a turn interrupted mid-stream and mid-tool timed out waiting for a screen once, on a busy Windows runner, on `main`'s side of an A/B.

- Area: cineris-cli
- Owner: the maintainers
- Flaky: `cineris-cli::system tui::sys_a_turn_is_interrupted_mid_stream_and_mid_tool`

| Item | Who | When |
|---|---|---|
| **A terminal test's wait for a screen ran out once on a hosted Windows runner.** `cineris-cli`'s `tui::sys_a_turn_is_interrupted_mid_stream_and_mid_tool` failed in round 4 of the Windows A/B of `try` run 44912996180 (the stop-ends-turn step's final run), on `main`'s side, at `36d8266e`: it panicked in the test kit's pseudo-terminal (`crates/cineris-testkit/src/pty.rs`, `Pty::wait_for`, the panic for a wait whose time ran out, which prints the screen). That round's `main` side took 130.7 s where the others took 106 to 113 s. The test passed in the other four rounds on `main`'s side and in all five on the branch's, and the branch did not change the terminal application or its tests. The run's log has the screen the test saw. The question: which wait ran out (the job's log names it), and is its limit too short for a runner this loaded, or is something on the screen late for another reason? [conpty.md](../conpty.md) has what is known of a pseudo-console's delays. | The maintainers | Before the first release |
