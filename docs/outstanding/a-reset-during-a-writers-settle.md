A writer that loaded the aggregates just before `cineris stats reset` saves their part back.

- Area: cineris-stats
- Owner: the step that next changes the reset, or step 33

| Item | Who | When |
|---|---|---|
| **A reset during a writer's settle can bring back what it deleted.** A writer that loaded `aggregates.json` just before `cineris stats reset` deleted it and the monthly files goes on to bring those aggregates up to date: it finds the monthly files gone, takes them as pruned by another process, moves their part into `kept`, and saves. The figures the user reset are back, in a new `aggregates.json`. The window is one settle, milliseconds, every five seconds while a session records. Found by the review of step 16's fixes, by reading; nothing is lost, the opposite. To settle: a reset that leaves a mark (a reset generation in a small file the writer reads with the aggregates, or the aggregates file replaced by an empty one of a new generation rather than deleted), which a save of aggregates loaded under an older mark refuses. | The step that next changes `cineris stats reset`, or step 33 | Before `/usage` (step 23) offers a reset |
