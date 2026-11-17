A float a client sends is read to within a few units in its last place, since `serde_json` is built without its exact reading of floats.

- Area: other
- Owner: the owner

| Item | Who | When |
|---|---|---|
| **Floats read within a few units.** The workspace builds `serde_json` without its `float_roundtrip` feature, so a float is read to within 2 units in its last place (the fuzzing of step 27's server: on the desktop, 30% of 20 million random floats did not read back exactly, none by more than 2 units). Nothing in the protocol carries a float where that matters today (what a client sends as configuration overrides could), and the server's fuzz target allows for it. **To choose:** turn `float_roundtrip` on (exact, at some cost in reading speed, measured before it is chosen, and a row in `docs/dependencies.md`), or leave it and say in 11-rpc-protocol that floats are read to within a few units. | The owner | Before a float where it matters crosses the protocol |
