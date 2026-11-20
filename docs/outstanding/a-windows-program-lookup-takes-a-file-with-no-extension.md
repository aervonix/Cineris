On Windows, looking a program up by name takes a file of that name with no extension before a later folder's `.exe`, and then fails to start it.

- Area: cineris-base
- Owner: step 32 (hardening pass)

| Item | Who | When |
|---|---|---|
| **A file with no extension is taken for a program on Windows.** `cineris_base::program::find` and `find_outside` try `name.exe`, then the bare `name`, in each folder of `PATH` in turn, and count any file as a program: an extensionless `git` or `bash` in an earlier folder (a shell script some tools put there, outside the workspace) is found before a later folder's `git.exe`, and its start then fails. A robustness gap, not a security one: the file is outside the workspace and is not run by anything but Windows' own rules, which refuse it. Step 28's lookup for an extension's programs (`find_runnable_outside`) takes only `.exe`, `.cmd` and `.bat`, one folder at a time, and is not affected. Found while step 28 looked for the same mechanisms in the harness's other starts. **Recommendation:** on Windows, take only a name with an extension Windows runs (`.exe` for the harness's own starts), as `find_runnable_outside` does, and pass over the bare name. | Step 32 (hardening pass) | Before the first release |
