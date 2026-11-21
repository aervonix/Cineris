The break that sends a headless run's words for a person to standard output, `step-13b/diagnostics-on-standard-output`, is not caught by the test it names, so the rule that standard output carries only the result is not held by a test.

- Area: cineris-cli
- Owner: the maintainers, a small-fix session

| Item | Who | When |
|---|---|---|
| **What was seen.** Step 28's run of the breaks a change can affect (`mutate --changed`, on the fourth pull request) found `step-13b/diagnostics-on-standard-output` not caught by its own tests, and it is not caught on `main` itself either (`36d8266e`, the break run alone). The break makes `Front::say` in `crates/cineris-cli/src/headless.rs` print with `println!` where it prints with `eprintln!`, and its list names `cineris-cli::system runs::sys_exit_0_in_each_format_with_only_the_result_on_standard_output`, which passes with it: the run that test makes likely says nothing to a person, so nothing reaches standard output to be caught. **Recommendation:** make the test's run say something to a person in each format (a notice, a warning the configuration gives) and hold standard output to the result alone; or name, in the break, a test whose run does; then run the break again. | The maintainers, a small-fix session | With the next small fixes |
