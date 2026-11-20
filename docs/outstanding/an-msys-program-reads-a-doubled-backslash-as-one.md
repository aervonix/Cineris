On Windows an MSYS2 or Cygwin program reads two backslashes inside a quoted argument as one, where the C runtime reads two: an extension's argument such as `\\server\share` reaches such a program as `\server\share`.

- Area: cineris-core
- Owner: step 32 (hardening pass)

| Item | Who | When |
|---|---|---|
| **A doubled backslash reaches an MSYS program as one.** Since step 28 every argument of an extension's `.exe` on Windows is quoted with the C runtime's escaping, so that an MSYS2 or Cygwin program (Git for Windows' `usr\bin`) does not expand `*` or take `'` as a quote. Inside quotes such a program reads `\\` as one backslash, where the C runtime reads two, and no quoting satisfies both: `\\server\share` arrives as `\server\share`, `C:\x\\y` as `C:\x\y`. It cannot split an argument or expand a pattern, but the engine judges a path that the program then reads differently. Measured on a hosted runner in step 28 (try run 44803442670). **Recommendation:** quote differently for a program with `msys-2.0.dll` or `cygwin1.dll` beside it (every backslash doubled, `"` escaped as `\"`), or refuse a doubled backslash in an argument to one. | Step 32 (hardening pass) | Before the first release |
