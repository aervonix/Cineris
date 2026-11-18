A flaky entry names a command, not the test that flipped, so a check's retry runs the whole command and may call a real failure flaky.

- Area: cineris-knowledge
- Owner: step 52, with its per-ecosystem test selection

| Item | Who | When |
|---|---|---|
| **Which test of a flaky command flipped.** A `flaky` entry of the project profile names a command (`cargo test -p net`), not the test inside it that failed and then passed: step 17 learns it from a command's exit status, and left reading test output to step 19. Step 19 retries the whole command once when it fails, as a check, and calls it flaky when the retry passes: so a command with one flaky test and one real failure that happens to pass on the retry is reported flaky, and a command whose flaky test is one of thousands is run whole again. Reading which test failed needs a reader for each test runner's output (cargo's `test … FAILED`, pytest's `FAILED …::…`, jest's `✕`), which is per-ecosystem work: the entry would name the test, and the retry would run that test alone. Recommendation: with step 52's affected-test selection, which needs the same readers. ([steps/19/answered.md](../steps/19/answered.md)) | Step 52 | With its per-ecosystem test selection |
