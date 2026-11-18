A summary cut off at the output limit is marked only in its text, not in the log's `summary` entry.

- Area: cineris-agent
- Owner: a later step on compaction, with the owner for the format

| Item | Who | When |
|---|---|---|
| **A summary cut off at the output limit is marked only in its text.** Since the fixes after the review of step 15, a summary whose response stopped at the output limit is used, so that the session is not stuck at the trigger, and its text ends where it stops with a fixed line saying it was cut (`SUMMARY_CUT` in `cineris-agent`'s `context::compact`); the notice that the compaction finished says so to the user. Nothing in the log's data says it: a later step reading the `summary` entry sees a summary like any other, and nothing tries for a whole one. It is most likely on a model whose reasoning is a token budget that its maximum output can hardly hold with a summary's 16,384 tokens beside it. To settle: a field of the `summary` entry that says the summary was cut (an addition to the log's format, which needs agreeing), and perhaps one more try with more room before a cut summary is used. | A later step that touches compaction, with the owner for the format | When a cut summary is seen in use, or before the log's format is next added to |
