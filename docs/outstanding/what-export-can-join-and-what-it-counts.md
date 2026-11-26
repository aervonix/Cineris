`/export`'s Markdown can join the halves of a key that only the core can mask, and its count of masks counts `[redacted]` typed in the text too.

- Area: cineris-tui
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **Found by the review of step 25, at lower confidence, by reading.** (1) **Two halves joined in the Markdown.** `export::readable` leaves control characters out, and the client's pass masks again by the filter's formats only (a client over the wire never has the harness's keys, which the core masks by value). A key of no known format, which the core holds and masks by value, printed with a NUL inside it, is two strings to the core's pass, neither the key, and is joined whole in the Markdown once the NUL is left out. The input is contrived. (2) **The count counts typed text:** the number said after an export counts every `[redacted]` in the text, one typed by the person or the model included. Cosmetic. (3) `/export ~/x.md` does not expand `~`, as [09-tui.md](../architecture/09-tui.md) says: a path given is taken from the folder the person started in. Cosmetic; a person may expect `~` to be the home folder. **Recommendation:** for (1), have the core's pass also mask by value the text as `readable` will make it (its control characters left out), or have `readable` put a visible mark where it leaves a character out, so that no two halves meet; for (2), count the masks the passes make, not the text's. (3) is a choice for the owner. | Step 32 (hardening) | Before the first release |
