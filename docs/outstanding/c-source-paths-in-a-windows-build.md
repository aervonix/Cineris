A Windows build of the shipped binary on a local machine keeps the build machine's folders in QuickJS's asserts, since MSVC has no flag the shipped build can give it to take them out.

- Area: tooling
- Owner: step 34

| Item | Who | When |
|---|---|---|
| **The C compiler's paths in a Windows build.** QuickJS's asserts name their source files, which its binding compiles from its build folder (`target/…`). Since step 28 the shipped build gives the C compiler `-ffile-prefix-map` for every folder it remaps for rustc, and the Linux binary holds none (`cargo xtask size`'s home-path check). MSVC has no stable equivalent (`/d1trimfile` is undocumented), so `xtask`'s `c_flags` gives it nothing: a Windows binary built on a local machine holds that machine's target folder, under the user's home. CI's Windows runner builds under `D:\a\…`, which is no home folder, so `pr-size` passes. **What would close it:** build the released Windows binary only on a runner (step 34's release workflow does), and run `cargo xtask home-paths` on it; or, if MSVC gains a supported flag, give it in `c_flags`; or turn QuickJS's asserts off for Windows only (`disable-assertions`), which the maintainers declined for Linux (an assert that ends the process is the safer failure). | Step 34, with the release workflow | Before a Windows binary built off a runner is published |
