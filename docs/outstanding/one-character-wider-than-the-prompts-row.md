One character can be drawn wider than a row where every escape of what acts unseen is shown: an approval's detail, the trust question, the transcript.

- Area: cineris-tui
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **One character can be drawn wider than a row, outside the prompt.** A base letter followed by a long run of tag characters (U+E0020 to U+E007F) is one grapheme cluster, and what acts unseen is drawn as its escape (`cineris_term::sanitize`), about ten columns a character, so the cluster can be wider than its row and is cut with `…`. **Step 25 closed it in the prompt editor's row**, the case this item was opened for: there a run of four or more of one escape is drawn once with its count (`\u{e0041}×12`, `editor::cluster::drawn`), so a row always holds the cluster. **The rest stays open, by the maintainers' answer (d) to step 25's design:** an approval's detail, the trust question, the first run's commands and the transcript keep every escape, never shortened, because there what is hidden must be shown whole, and the inline presenter's output and what `/copy` gives are unchanged; the item's first recommendation (a counted mark in every place `sanitize` escapes) would hide hidden text where it must be shown. Recommendation for the rest: where such a cluster is wider than its row, wrap its escapes over as many rows as they need, as a long word is wrapped, rather than cut it. | Step 33 | Before the first release |
