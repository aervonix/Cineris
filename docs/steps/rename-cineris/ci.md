# After the rename to Cineris: the tool, and the tree in the new names

The rename changes names, not what is built or tested: the same tests run, and the binary holds the same code with longer names. What it adds to a run is the rename's check in the repo rules (about 0.3 s of `cargo xtask rules` at the desk) and the tool's own tests in `xtask` (20 tests, the slowest the whole-tree pass at about 0.35 s on the laptop).
