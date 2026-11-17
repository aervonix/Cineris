The full screen's search keeps a lower-cased copy of the whole kept transcript for as long as the program runs, and builds it on the interface's thread the first time.

- Area: cortex-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **The search's copy of the transcript.** The search's worker (`app::search::TextSearch`) is given each kept block's text once, lower-cased, and keeps it for the life of the process, though most sessions search seldom; and the first `Ctrl+F` builds every block's text on the interface's thread (`App::feed_search`) before the worker has any. Over 200 turns the copy is a few megabytes, within the full screen's 80 MB (step 25's measurement, 33.2 MB after a search), and the first build took no frame past its budget there; but both grow with the session. Found by the review of step 25, which also asked for the query to be bounded (done: 256 characters) and for a worker that cannot start to say so (done: `the search could not start`). Recommendation: build a block's text on the worker from the kept block (shared, not copied), keep no copy between searches, and drop the worker when the search ends. | Step 33 (memory) | Before the first release |
