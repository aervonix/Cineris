The budget for loading a project's knowledge at its caps is 5 ms by the design note and 5.1 ms on a hosted runner; the row is written as recommended, pending.

- Area: cortex-knowledge
- Owner: the owner

| Item | Who | When |
|---|---|---|
| **The budget for loading a project's knowledge at its caps, on a hosted runner.** Step 17's design note set 5 ms for reading, parsing and validating both files at their caps (700 entries, 247 KB). The desktop measures 2.65 ms; the hosted runners 5.06 ms (Linux) and 4.67 ms with a 95th percentile of 5.9 ms (Windows) ([steps/17/ci.md](../steps/17/ci.md)). [performance-budgets.md](../performance-budgets.md) carries the recommendation as its row, marked pending: 5 ms on the desktop and 15 ms on a hosted runner, the factor the start-up rows use between the two, with the caps kept. The alternative is to lower the cap on a file's size from 256 KiB to about 128 KiB, which holds 5 ms everywhere and halves what a store may hold. The load is on the blocking pool, at a session's first turn and when a file changed; a typical store loads in 0.2 to 0.6 ms. | The owner (the coordinator has put it to them) | When the owner answers: the row loses its mark, or the cap changes by a new decision |
