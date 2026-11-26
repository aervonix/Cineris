A file read on the first run's set-up worker that never ends holds the configuration's write behind it for good.

- Area: cineris-tui
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **One worker, in order, and a plain open.** The first run's set-up worker (`crates/cineris-tui/src/app/setup.rs`, lines 302-334 at the review's commit, `77ffbbcc`) runs every job on one thread in turn, and the import opens a file the person asked to see inside with a plain `File::open` (`crates/cineris-config/src/import.rs`, lines 661-666). A file whose read never ends (a link into a mount that does not answer, or a FIFO put in a file's place after discovery) holds `Job::Write`, queued behind it, for good: the first run can never write the configuration. Discovery lists only regular files (`discover.rs`, lines 195-200), so a FIFO there from the start is not offered. Found by the review of step 25, by reading only. **Recommendation:** open with `open_regular` and read with a bound, as detection does (`cineris-knowledge`'s `detect`), so that such a file is refused and said, and the jobs behind it run. | Step 32 (hardening) | Before the first release |
