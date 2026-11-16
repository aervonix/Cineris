`replace` stacks a `--to`'s doc comment on the file's, and can leave a read-only file's temporary copy on Windows.

- Area: tooling
- Owner: a later tooling step

| Item | Who | When |
|---|---|---|
| **Two corners of `cargo xtask replace` the fifth tooling step left.** A match by tokens may not begin or end with a comment, which closed the doubled doc comments a `--from` with one made; but a `--from` of code whose `--to` begins with a doc comment, on code that already has one, still stacks the two (`/// Old.` then `/// New.`), and adding a comment is a real use. A warning when `--to` begins with a comment and the file has one just before the match would catch it. And on Windows a script's rewrite of a read-only file makes its `.replace-new` read-only too: if the rename then fails, the clean-up may not remove it. | A later tooling step | When `replace` is next changed |
