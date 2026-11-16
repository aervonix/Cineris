| Item | Who | When |
|---|---|---|
| **`cargo xtask size --breakdown` does nothing on Windows** (the eighth tooling step). It reads the binary's symbols with `nm`, and an MSVC binary keeps them in a PDB, where `nm` cannot read them; it says so before building. The Linux breakdown is the one to read, the same code being about 1.15 to 1.2 times the size on Windows. `llvm-nm` from `rustup component add llvm-tools`, or reading the PDB, would give Windows' own. | A later tooling step | When a Windows-only growth needs explaining |
