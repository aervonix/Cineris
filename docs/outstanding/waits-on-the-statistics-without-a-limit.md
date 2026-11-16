A day's purse's first read at session open, and the refresh behind `limits.json`'s lock, wait with no time limit of their own.

- Area: cortex-stats
- Owner: step 27

| Item | Who | When |
|---|---|---|
| **Two waits on the statistics have no time limit of their own.** The review of step 16's fixes found them; neither holds up a turn or a process's end, which are bounded (a request waits for other processes' spending at most a second, and the end for the writer at most two). (1) **The day's purse's first read**, when a core's first day-limited session of the day is put together: the aggregates and the day's monthly files are read on the blocking pool, under the core's day lock, so a disk that does not answer holds that session's opening, and every other day-limited session's in that core, until it does. Putting a session together reads other files the same way (its log, the configuration), so this is of a kind with them. (2) **`limits.json`'s lock**: a refresh (`cortex usage --refresh`) waits for it while another process holds it; that process holds it for one read and one write of a small file, and the system lets it go when the process ends, so only a process stuck on its disk while it holds it keeps the refresh waiting. To settle: a time limit on putting a session together as a whole (step 27's graceful shutdown touches the same paths), and a lock taken with a time limit (`try_lock` in a loop on the core's timer). | Step 27, or the step that bounds putting a session together | Before the TUI (step 23) opens sessions on a network home directory |
