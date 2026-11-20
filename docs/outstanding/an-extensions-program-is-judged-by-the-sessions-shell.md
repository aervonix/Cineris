The permission engine judges an extension's program by a line in the session's shell, whose own commands and aliases are not the program that runs; step 28 asks about what PowerShell and `cmd` call read-only rather than allow it.

- Area: cineris-policy
- Owner: step 32 (hardening pass)

| Item | Who | When |
|---|---|---|
| **An extension's program is judged as a line of the session's shell.** `cineris.exec` runs a program directly, with no shell, but the engine reads its line in the session's shell's syntax, where the first word may be that shell's own command or alias: in PowerShell `write` is `Write-Output` and `cat` is `Get-Content`, read-only, while the program that runs is `write.exe` or an MSYS `cat`; and a quoted `-x` is text to PowerShell's table, an option to the program. Found by the early review of step 28's process starts. **Since step 28** an exec host call whose line was judged in PowerShell or `cmd`, and which the engine allows as read-only, is asked about instead (`HostQuestion::read_only_asks`; in a POSIX shell the judged name is the program, and nothing changes). **What would close it:** an `Exec` effect that says its line names an external program, which the engine judges by the rows of real programs only (`gh`, `git`, `whoami`) and whose quoted words are a program's arguments; then the guard can go. A change to the engine's rules, which step 28 may not make. | Step 32 (hardening pass) | Before the first release |
