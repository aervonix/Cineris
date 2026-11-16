A letter with a long run of tag characters is one cluster whose escape is wider than the prompt's row, cut with `…`.

- Area: cortex-tui
- Owner: step 25

| Item | Who | When |
|---|---|---|
| **One character can be drawn wider than the prompt's row.** The prompt editor lays each grapheme cluster out in the columns of what is drawn for it (step 23's review fixes), and what acts unseen is drawn as its escape. A base letter followed by a long run of tag characters (U+E0020 to U+E007F) is one cluster whose escape is about ten columns a character, so it can be wider than the row: the cluster takes a row of its own, `RoundedBox::row` cuts it with `…`, and the cursor is kept inside the box by `inside()`. Nothing is hidden that matters (the start of the escape shows, and the text is the model's or the person's own), so it is cosmetic. Capping the cluster to the row gives the same picture, since a cluster cannot be split across rows. Recommendation: draw a long run of one kind of unseen character as a short counted mark (`\u{e0041}…×12`) in every place `sanitize` escapes, so a row always holds it; the escaped text a `/copy` gives is unchanged. | Step 25, with its text inputs | When step 25 draws text inputs of its own |
