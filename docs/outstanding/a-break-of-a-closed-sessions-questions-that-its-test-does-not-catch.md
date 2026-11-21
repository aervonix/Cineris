The break that leaves a closed session's open questions in place, `step-23-fixes/a-closed-sessions-question-is-not-withdrawn`, is not caught by the test it names, so the rule that a session's end withdraws its open trust and rewind questions is not held by a test.

- Area: cineris-core
- Owner: the maintainers, a small-fix session

| Item | Who | When |
|---|---|---|
| **What was seen.** Step 28's run of the breaks a change can affect (`mutate --changed`, on its fourth pull request) found `step-23-fixes/a-closed-sessions-question-is-not-withdrawn` not caught, and it is not caught on `main` itself either (`36d8266e`, the break run alone). The break deletes `self.inner.clients.withdraw(Some(session))` in `Core::end` (`crates/cineris-core/src/core.rs`); `cineris-core::it asking::an_undos_question_ends_when_its_session_is_closed` passes with it, so its question is ended by another path, or the test does not look at what the clients were sent. **Recommendation:** find which: if another path withdraws the questions, say so in the break's rule and expect it to survive, or delete it; if not, have the test hold the client to be told the question is withdrawn when the session closes, then run the break again. | The maintainers, a small-fix session | With the next small fixes |
