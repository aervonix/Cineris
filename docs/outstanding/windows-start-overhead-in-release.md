The release-build overhead of starting a shell command on Windows is unmeasured, though `try` can measure it.

- Area: windows
- Owner: step 33, or sooner on any Windows machine

| Item | Who | When |
|---|---|---|
| **The start overhead on Windows, in a release build.** Step 11 measured it on a hosted runner in the unoptimised test build, because nobody has a Windows machine to measure it on. The binary's own start-up in a release build is measured (`try` run 43512041506, in [performance-budgets.md](../performance-budgets.md), "Start-up"); the overhead of starting a command ("Starting a shell command") is not. `try` can measure it, unrun: `gh workflow run try --ref <branch> -f os=windows -f profile=release -f command="test -p cineris-tools --run-ignored only --success-output immediate perf_starting_a_command"`, which builds `cineris-tools`' tests in the release profile and prints the measurement. | Step 33 (the performance pass), or sooner on any Windows machine | Before the first release |
