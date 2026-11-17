The full screen's search counts its matches in each block's text but marks them in the drawn rows, so a match in a box's own words can take the current mark.

- Area: cortex-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **The count and the marks can disagree.** The search's worker counts the query in each kept block's text: an answer's markdown source, a round's calls, targets and output (`KeptBlock::text`). The frame finds the current match, and marks every match, in the rows as drawn, which also hold what the interface adds: a box's border label (`ctrl+o expands`), a call's status (`exit 101 · 3.4s`), an answer's markdown drawn without its marks. For a query such as `ctrl` or `exit` the n-th match of the drawn rows can be a word of that chrome, not the n-th the worker counted: the current mark lands on the border, and a closed box that holds the real match is not opened. Found by the review of step 25. Recommendation: count and mark in one place, the drawn rows' own content: draw a block for the search without its chrome (the text of its content spans only), have the worker count in that, and have `marked` skip the spans the interface adds. | Step 33, with the search's other open points | Before the first release |
