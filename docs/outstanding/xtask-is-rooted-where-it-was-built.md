`cargo xtask` finds the repository from where its binary was built, not from the current directory, so a binary built in one checkout and run from another checks the first one's tree.

- Area: tooling
- Owner: the coordinator

| Item | Who | When |
|---|---|---|
| **A shared target folder runs another checkout's xtask.** Found in step 25: several checkouts shared one `CARGO_TARGET_DIR`; their xtask sources were the same, so cargo took the first checkout's build as fresh and ran it from every other one. The commands that work from the repository's root (`links`, `rules`, `outstanding --check`, `rename --check`, `mutate --check`, and `branch fold`'s idea of the current branch) then looked at that first checkout, and passed or failed on its tree; the ones that take relative paths (`replace`) acted on the right one. Until it is fixed, those commands go through `rx`, which copies the checkout, or are run with a target folder of the checkout's own. **Recommendation:** have xtask find the root from the current directory (the nearest folder above it with the workspace's `Cargo.toml` and `xtask/`), falling back to where it was built, and say which root it took when the two differ. | A later tooling fix | Before the next session that shares a target folder |
