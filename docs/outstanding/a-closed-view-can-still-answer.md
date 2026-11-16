After `session/close` a client stays among those an approval request went to, so its answer is still taken.

- Area: cortex-core
- Owner: the owner; before step 27

| Item | Who | When |
|---|---|---|
| **A client that closed its view of a session can still answer a question it was asked there.** `session/close` takes the client off the session's events and leaves it in the list of clients an open approval request was sent to, so its answer is still taken. The protocol allows it (the client was sent the request, and an answer is checked for that), but a front end that closed a session probably does not expect to settle its questions. The choice: leave it, or take a client that closes its view out of the questions it was sent, as one that disconnects already is (its answer is refused, since it is no longer a client). Noted by the independent review of step 13a, and not chosen for its fixes. **Built in step 27 as recommended, pending the owner's word** ([../steps/27/answered.md](../steps/27/answered.md), "The open items that name step 27"): a client that closes its view of a session is taken out of the session's open questions, as one that disconnects is. Its answer to them is then refused (`role_not_allowed`: it was not put to this client), a question that ends with its askers and has none left ends as no answer (an undo's keeps the files), and a client that opens the session again is put what is still open (`cortex-core`'s `closing` tests). The owner may still choose to leave it as it was. | The owner | Before step 27 is merged |
