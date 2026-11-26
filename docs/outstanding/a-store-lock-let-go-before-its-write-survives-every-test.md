Letting the trust store's or the approvals store's lock go between their read and their write survives every test: no test has a second writer come between them.

- Area: cineris-policy
- Owner: the next small-fix session

| Item | Who | When |
|---|---|---|
| **What is open.** `TrustStore::update` (`crates/cineris-policy/src/trust.rs`) and `ApprovalStore::update` (`approvals.rs`) each hold their store's lock from the read to the write, as the pins' store does. Their tests (`every_change_to_the_store_is_made_under_its_lock`) ask whether the lock is held inside the change, before a write that a break could move the lock's end ahead of; so a change that drops the lock after the read and before the write, letting a second writer read the old file and write over the first one's change, would pass every test. The one break of their lock (`step-12-fixes/no-store-lock`) takes it away whole, which those tests catch. Found by step 28's close, whose fix of the pins' store's test showed the same gap there (`pins::a_writer_between_anothers_read_and_write_waits_and_both_keep_their_pins`, through the point `cineris_policy::trust::BEFORE_THE_PINS_ARE_WRITTEN`), and left for the next small-fix session by the maintainers. **What would close it:** the same pattern for both stores: a point between each one's read and its write, a test that holds the first writer there and has a second come between them, and a break each that lets the lock go before the write, in step 12's lists. | The next small-fix session | Before the first release |
