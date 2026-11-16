A listing cut at 100,000 entries holds the first directories of a depth-first walk, not the top of the tree.

- Area: cortex-core
- Owner: a later index step, or step 33

| Item | Who | When |
|---|---|---|
| **The top of a tree past the listing cap.** A pass lists at most 100,000 entries ([15-efficiency.md](../architecture/15-efficiency.md), "Repository map"). The walk is depth first, in name order, so a listing cut at that cap holds the first directories in full and nothing of the later ones: a root with a large `third_party/` that is not ignored, and that sorts before `src/`, has a map without `src/` (found by the review of step 18). The cap on files keeps the shallowest of what was listed, which is the top of the tree only when the listing reached it all. The map says it is partial, and why. A walk breadth first (each level's directories listed before any is gone into), or a budget of entries for each level, keeps the top of any tree; `ignore`'s walker has neither, so it is a walk of our own over its ignore matchers. | A later step on the index, or step 33 | When a workspace past 100,000 entries is reported, or before the first release |
