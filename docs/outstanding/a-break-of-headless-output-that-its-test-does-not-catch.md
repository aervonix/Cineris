The break that sends a headless run's words for a person to standard output, `step-13b/diagnostics-on-standard-output`, is caught by other tests but not by the one its list names, so the list no longer says which test holds the rule.

- Area: cineris-cli
- Owner: the maintainers, a small-fix session

| Item | Who | When |
|---|---|---|
| **What was seen.** Step 28's run of the breaks a change can affect (`mutate --changed`, on the fourth pull request) found `step-13b/diagnostics-on-standard-output` not caught by its own tests, on `main` itself too (`36d8266e`, the break run alone); the run's whole verdict (`NOT CAUGHT BY ITS OWN TESTS`) is that other tests do catch it: `cineris-cli::system commands::sys_a_project_trusted_with_permissions_trust_is_trusted_on_the_next_start`, `runs::sys_exit_1_when_the_harness_itself_fails`, `runs::sys_exit_2_for_mistakes_…` and more, so the rule is held. The break makes `Front::say` in `crates/cineris-cli/src/headless.rs` print with `println!` where it prints with `eprintln!`, and its list names `cineris-cli::system runs::sys_exit_0_in_each_format_with_only_the_result_on_standard_output`, which passes with it: the run that test makes likely says nothing to a person, so nothing reaches standard output to be caught. **Recommendation:** name in the break the tests that catch it (`cargo xtask mutate` prints them), or make the named test's run say something to a person in each format and hold standard output to the result alone; then run the break again. | The maintainers, a small-fix session | With the next small fixes |
