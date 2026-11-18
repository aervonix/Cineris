`deny.toml` ignores RUSTSEC-2025-0141, `bincode` unmaintained, reached only through `syntect`'s loading of its packed grammars; drop it when `syntect` moves off `bincode` or a flaw is named.

- Area: cineris-tui
- Owner: the next step that touches highlighting, otherwise step 25

| Item | Who | When |
|---|---|---|
| **The advisory ignored for `bincode`, which `syntect` brings.** `deny.toml` ignores RUSTSEC-2025-0141, which says `bincode` is unmaintained: `syntect` 5.3.0 loads the grammar dumps packed into its own build with it, and nothing from outside reaches it ([dependencies.md](../dependencies.md), "Advisories ignored"). Ignored by the owner's decision of 2026-11-15, in the fifth tooling step, after the nightly `dependencies` job failed on it. To settle: drop the ignore when `syntect` moves off `bincode` (a release that loads its dumps another way), or at once if an advisory names a real flaw in `bincode` 1.3.3; then delete this file, its line in the index and the row. | The next step that touches highlighting, otherwise step 25 | When `syntect` releases a version without `bincode`, or an advisory names a flaw |
